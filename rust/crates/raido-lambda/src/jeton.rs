// SPDX-License-Identifier: EUPL-1.2
//! Vérification du jeton court (spec §7.1) : JWT RS256 émis par `app.runeride.fr` (classe PHP
//! `RuneRide\Auth\Jwt`), clés publiques lues dans son JWKS (`/.well-known/jwks.json`).
//!
//! Revendications attendues : `iss` (émetteur), `aud` = `raido`, `sub` (identifiant opaque, jamais
//! journalisé), `exp`, `iat`, plus `palier` (`gratuit` | `abonne`), `type` (`petite_boucle` | `grand`)
//! et `limite_km` (facultatif, borne de distance d'une petite boucle). Le JWKS est chargé au premier
//! besoin et rafraîchi au plus une fois par `RAFRAICHISSEMENT_MIN` quand un `kid` est inconnu
//! (rotation de clé côté PHP).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use serde::Deserialize;
use sha2::Sha256;

const RAFRAICHISSEMENT_MIN: Duration = Duration::from_secs(300);
/// Tolérance d'horloge sur `exp` et `iat`.
const TOLERANCE_S: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErreurJeton {
    Absent,
    Malforme(&'static str),
    CleInconnue,
    SignatureInvalide,
    Expire,
    EmetteurOuAudience,
}

impl std::fmt::Display for ErreurJeton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErreurJeton::Absent => f.write_str("jeton absent"),
            ErreurJeton::Malforme(m) => write!(f, "jeton mal formé : {m}"),
            ErreurJeton::CleInconnue => f.write_str("clé de signature inconnue"),
            ErreurJeton::SignatureInvalide => f.write_str("signature invalide"),
            ErreurJeton::Expire => f.write_str("jeton expiré ou pas encore valable"),
            ErreurJeton::EmetteurOuAudience => f.write_str("émetteur ou audience inattendu"),
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Revendications {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub exp: u64,
    #[serde(default)]
    pub iat: u64,
    #[serde(default = "gratuit")]
    pub palier: String,
    #[serde(default = "grand", rename = "type")]
    pub type_: String,
    pub limite_km: Option<f64>,
}

fn gratuit() -> String {
    "gratuit".into()
}
fn grand() -> String {
    "grand".into()
}

impl Revendications {
    pub fn est_petite_boucle(&self) -> bool {
        self.type_ == "petite_boucle"
    }
    /// Distance maximale autorisée (km) pour une petite boucle (défaut 25, spec D4).
    pub fn limite_km(&self) -> f64 {
        self.limite_km.unwrap_or(25.0)
    }
}

/// D'où vient le JWKS.
pub enum SourceJwks {
    Url(String),
    Fichier(std::path::PathBuf),
    /// Tests : JWKS déjà en mémoire.
    Texte(String),
}

pub struct Verificateur {
    source: SourceJwks,
    emetteur: String,
    audience: String,
    cles: Mutex<HashMap<String, VerifyingKey<Sha256>>>,
    dernier_chargement: Mutex<Option<Instant>>,
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<CleJwk>,
}

#[derive(Deserialize)]
struct CleJwk {
    kty: String,
    kid: String,
    n: String,
    e: String,
    #[serde(default)]
    alg: Option<String>,
}

/// Base64 (standard ou URL, avec ou sans `=`).
pub fn base64_decoder(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a') as u32 + 26,
            b'0'..=b'9' => (c - b'0') as u32 + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in s.as_bytes() {
        if c == b'=' || c == b'\n' || c == b'\r' {
            continue;
        }
        acc = acc << 6 | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

impl Verificateur {
    pub fn nouveau(source: SourceJwks, emetteur: &str, audience: &str) -> Verificateur {
        Verificateur { source, emetteur: emetteur.into(), audience: audience.into(), cles: Mutex::new(HashMap::new()), dernier_chargement: Mutex::new(None) }
    }

    fn lire_jwks(&self) -> Result<String> {
        match &self.source {
            SourceJwks::Texte(t) => Ok(t.clone()),
            SourceJwks::Fichier(p) => std::fs::read_to_string(p).with_context(|| format!("JWKS {}", p.display())),
            SourceJwks::Url(u) => {
                let agent = rw_aws::http::agent(rw_aws::http::Delais { connexion: Duration::from_secs(5), reponse: Duration::from_secs(10), global: Duration::from_secs(15) }, "runeride-raido/0.1 (jwks)");
                let mut rep = agent.get(u).call().with_context(|| format!("JWKS {u}"))?;
                if rep.status().as_u16() != 200 {
                    return Err(anyhow!("JWKS {u} : HTTP {}", rep.status()));
                }
                Ok(rep.body_mut().read_to_string().context("JWKS : corps")?)
            }
        }
    }

    /// (Re)charge les clés ; renvoie le nombre de clés RSA retenues.
    pub fn charger(&self) -> Result<usize> {
        let texte = self.lire_jwks()?;
        let jwks: Jwks = serde_json::from_str(&texte).context("JWKS : JSON")?;
        let mut nouvelles = HashMap::new();
        for k in jwks.keys {
            if k.kty != "RSA" || k.alg.as_deref().is_some_and(|a| a != "RS256") {
                continue;
            }
            let (Some(n), Some(e)) = (base64_decoder(&k.n), base64_decoder(&k.e)) else { continue };
            let Ok(cle) = RsaPublicKey::new(BigUint::from_bytes_be(&n), BigUint::from_bytes_be(&e)) else { continue };
            nouvelles.insert(k.kid, VerifyingKey::<Sha256>::new(cle));
        }
        let n = nouvelles.len();
        *self.cles.lock().unwrap_or_else(|e| e.into_inner()) = nouvelles;
        *self.dernier_chargement.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        Ok(n)
    }

    fn cle(&self, kid: &str) -> Option<VerifyingKey<Sha256>> {
        self.cles.lock().unwrap_or_else(|e| e.into_inner()).get(kid).cloned()
    }

    fn rafraichir_si_possible(&self) -> bool {
        let dernier = *self.dernier_chargement.lock().unwrap_or_else(|e| e.into_inner());
        if dernier.is_none_or(|t| t.elapsed() >= RAFRAICHISSEMENT_MIN) {
            return self.charger().is_ok();
        }
        false
    }

    /// Vérifie un jeton « Bearer » et renvoie ses revendications.
    pub fn verifier(&self, jeton: &str, maintenant: u64) -> Result<Revendications, ErreurJeton> {
        let jeton = jeton.trim();
        if jeton.is_empty() {
            return Err(ErreurJeton::Absent);
        }
        let mut parties = jeton.split('.');
        let (Some(h), Some(c), Some(s), None) = (parties.next(), parties.next(), parties.next(), parties.next()) else {
            return Err(ErreurJeton::Malforme("trois parties attendues"));
        };
        let entete: serde_json::Value = serde_json::from_slice(&base64_decoder(h).ok_or(ErreurJeton::Malforme("en-tête"))?).map_err(|_| ErreurJeton::Malforme("en-tête JSON"))?;
        if entete.get("alg").and_then(|a| a.as_str()) != Some("RS256") {
            return Err(ErreurJeton::Malforme("alg ≠ RS256"));
        }
        let kid = entete.get("kid").and_then(|k| k.as_str()).ok_or(ErreurJeton::Malforme("kid absent"))?;
        let cle = match self.cle(kid) {
            Some(c) => c,
            None => {
                // clé inconnue : une rotation a pu avoir lieu, on recharge (au plus toutes les 5 minutes)
                self.rafraichir_si_possible();
                self.cle(kid).ok_or(ErreurJeton::CleInconnue)?
            }
        };
        let signature = Signature::try_from(base64_decoder(s).ok_or(ErreurJeton::Malforme("signature"))?.as_slice()).map_err(|_| ErreurJeton::Malforme("signature"))?;
        let signe = &jeton[..h.len() + 1 + c.len()];
        cle.verify(signe.as_bytes(), &signature).map_err(|_| ErreurJeton::SignatureInvalide)?;
        let rev: Revendications = serde_json::from_slice(&base64_decoder(c).ok_or(ErreurJeton::Malforme("revendications"))?).map_err(|_| ErreurJeton::Malforme("revendications JSON"))?;
        if rev.iss != self.emetteur || rev.aud != self.audience {
            return Err(ErreurJeton::EmetteurOuAudience);
        }
        if rev.exp + TOLERANCE_S <= maintenant || rev.iat > maintenant + TOLERANCE_S {
            return Err(ErreurJeton::Expire);
        }
        Ok(rev)
    }
}

/// Secondes depuis l'époque Unix.
pub fn maintenant() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1v15::SigningKey;
    use rsa::signature::{SignatureEncoding, Signer};
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;

    fn b64url(d: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut s = String::new();
        for morceau in d.chunks(3) {
            let n = (morceau[0] as u32) << 16 | (*morceau.get(1).unwrap_or(&0) as u32) << 8 | *morceau.get(2).unwrap_or(&0) as u32;
            s.push(T[(n >> 18) as usize & 63] as char);
            s.push(T[(n >> 12) as usize & 63] as char);
            if morceau.len() > 1 {
                s.push(T[(n >> 6) as usize & 63] as char);
            }
            if morceau.len() > 2 {
                s.push(T[n as usize & 63] as char);
            }
        }
        s
    }

    struct Banc {
        signe: SigningKey<Sha256>,
        kid: String,
        jwks: String,
    }

    fn banc() -> Banc {
        let mut rng = rand::thread_rng();
        let prive = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public = prive.to_public_key();
        let kid = "cle-de-test".to_string();
        let jwks = serde_json::json!({"keys": [{"kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid,
            "n": b64url(&public.n().to_bytes_be()), "e": b64url(&public.e().to_bytes_be())}]})
        .to_string();
        Banc { signe: SigningKey::<Sha256>::new(prive), kid, jwks }
    }

    fn jeton(b: &Banc, claims: serde_json::Value, kid: &str) -> String {
        let h = b64url(serde_json::json!({"alg": "RS256", "typ": "JWT", "kid": kid}).to_string().as_bytes());
        let c = b64url(claims.to_string().as_bytes());
        let signe = format!("{h}.{c}");
        let sig = b.signe.sign(signe.as_bytes()).to_bytes();
        format!("{signe}.{}", b64url(&sig))
    }

    #[test]
    fn jeton_valide_et_revendications() {
        let b = banc();
        let v = Verificateur::nouveau(SourceJwks::Texte(b.jwks.clone()), "https://app.runeride.fr", "raido");
        assert_eq!(v.charger().unwrap(), 1);
        let t = jeton(&b, serde_json::json!({"iss": "https://app.runeride.fr", "aud": "raido", "sub": "42", "iat": 1000, "exp": 1900, "palier": "gratuit", "type": "petite_boucle"}), &b.kid);
        let r = v.verifier(&t, 1500).unwrap();
        assert_eq!(r.sub, "42");
        assert!(r.est_petite_boucle());
        assert_eq!(r.limite_km(), 25.0);
        assert_eq!(v.verifier(&t, 2000), Err(ErreurJeton::Expire));
    }

    #[test]
    fn jeton_refuse() {
        let b = banc();
        let v = Verificateur::nouveau(SourceJwks::Texte(b.jwks.clone()), "https://app.runeride.fr", "raido");
        v.charger().unwrap();
        let bon = serde_json::json!({"iss": "https://app.runeride.fr", "aud": "raido", "sub": "1", "iat": 1000, "exp": 1900});
        // mauvaise audience (jeton de session « runeride » de l'appli)
        let t = jeton(&b, serde_json::json!({"iss": "https://app.runeride.fr", "aud": "runeride", "sub": "1", "exp": 1900}), &b.kid);
        assert_eq!(v.verifier(&t, 1500), Err(ErreurJeton::EmetteurOuAudience));
        // kid inconnu
        let t = jeton(&b, bon.clone(), "autre");
        assert_eq!(v.verifier(&t, 1500), Err(ErreurJeton::CleInconnue));
        // signature altérée
        let mut t = jeton(&b, bon.clone(), &b.kid);
        let dernier = t.pop().unwrap();
        t.push(if dernier == 'A' { 'B' } else { 'A' });
        assert_eq!(v.verifier(&t, 1500), Err(ErreurJeton::SignatureInvalide));
        // revendications altérées (signature d'un autre contenu)
        let t = jeton(&b, bon, &b.kid);
        let mut p: Vec<&str> = t.split('.').collect();
        let faux = b64url(serde_json::json!({"iss": "https://app.runeride.fr", "aud": "raido", "sub": "1", "exp": 1900, "palier": "abonne"}).to_string().as_bytes());
        p[1] = &faux;
        assert_eq!(v.verifier(&p.join("."), 1500), Err(ErreurJeton::SignatureInvalide));
        assert_eq!(v.verifier("", 1500), Err(ErreurJeton::Absent));
        assert_eq!(v.verifier("a.b", 1500), Err(ErreurJeton::Malforme("trois parties attendues")));
    }

    #[test]
    fn base64_url_et_standard() {
        assert_eq!(base64_decoder("eyJhIjoxfQ==").unwrap(), b"{\"a\":1}");
        assert_eq!(base64_decoder("eyJhIjoxfQ").unwrap(), b"{\"a\":1}");
        assert!(base64_decoder("@@").is_none());
    }
}
