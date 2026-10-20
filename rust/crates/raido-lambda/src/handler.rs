// SPDX-License-Identifier: EUPL-1.2
//! Du JSON de la Function URL (format « API Gateway v2 ») à la réponse HTTP.
//!
//! Routes : `GET /v1/sante`, `POST /v1/balade`, `POST /v1/trajet`. Tout le reste : 404.
//! Le graphe est partagé entre invocations d'un même environnement (cache de tuiles en
//! mémoire et sur `/tmp`) ; Lambda n'exécute qu'une invocation à la fois par environnement,
//! le verrou ne sert qu'à satisfaire le compilateur.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use raido_calcul::charge::{Graphe, SourceCache, SourceDisque, SourceTuiles};
use raido_calcul::apparier;
use raido_calcul::requete::{self, Requete};
use serde_json::{json, Value};

use crate::jeton::{self, base64_decoder, ErreurJeton, SourceJwks, Verificateur};
use crate::s3::SourceS3;

pub struct Contexte {
    pub graphe: Mutex<Graphe>,
    pub source_nom: String,
    pub version_binaire: &'static str,
    /// `None` : vérification désactivée (`RAIDO_JETON=desactive`, tests locaux seulement).
    pub verificateur: Option<Verificateur>,
    /// Clé de service de `POST /v1/apparier` (`RAIDO_CLE_SERVICE`) ; absente : la route n'existe pas.
    pub cle_service: Option<String>,
}

/// Journal JSON sur la sortie d'erreur (CloudWatch Logs la capture comme la sortie standard), une ligne
/// par événement, sans donnée personnelle ; la sortie standard reste libre pour le mode local.
pub fn journal(niveau: &str, mut v: Value) {
    if let Value::Object(m) = &mut v {
        m.insert("niveau".into(), json!(niveau));
    }
    eprintln!("{v}");
}

impl Contexte {
    /// Variables : `RAIDO_SOURCE` (`s3`, défaut, ou `local:/chemin`), `RAIDO_BUCKET`, `RAIDO_PREFIXE`
    /// (`graphe/courant`), `RAIDO_CACHE` (`/tmp/raido`, vide pour désactiver), `RAIDO_S3_ENDPOINT` (tests),
    /// `RAIDO_THREADS`, `AWS_REGION` ; jeton : `RAIDO_JETON` (`obligatoire`, défaut, ou `desactive`),
    /// `RAIDO_JWKS_URL` (défaut `https://app.runeride.fr/.well-known/jwks.json`) ou `RAIDO_JWKS_FICHIER`,
    /// `RAIDO_EMETTEUR` (`https://app.runeride.fr`), `RAIDO_AUDIENCE` (`raido`) ; `RAIDO_CLE_SERVICE` (clé de `/v1/apparier`).
    pub fn depuis_env() -> Result<Contexte> {
        let var = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
        let verificateur = if var("RAIDO_JETON").as_deref() == Some("desactive") {
            journal("WARN", json!({"evenement": "jeton_desactive", "message": "aucune vérification du jeton : réservé aux essais locaux"}));
            None
        } else {
            let source = match var("RAIDO_JWKS_FICHIER") {
                Some(f) => SourceJwks::Fichier(PathBuf::from(f)),
                None => SourceJwks::Url(var("RAIDO_JWKS_URL").unwrap_or_else(|| "https://app.runeride.fr/.well-known/jwks.json".into())),
            };
            let v = Verificateur::nouveau(source, &var("RAIDO_EMETTEUR").unwrap_or_else(|| "https://app.runeride.fr".into()), &var("RAIDO_AUDIENCE").unwrap_or_else(|| "raido".into()));
            // Chargement au démarrage : un JWKS injoignable est signalé mais n'empêche pas de démarrer
            // (nouvel essai à la première requête, puis toutes les 5 minutes).
            match v.charger() {
                Ok(n) => journal("INFO", json!({"evenement": "jwks", "cles": n})),
                Err(e) => journal("WARN", json!({"evenement": "jwks", "erreur": format!("{e:#}")})),
            }
            Some(v)
        };
        if let Some(t) = var("RAIDO_THREADS").and_then(|t| t.parse::<usize>().ok()) {
            let _ = rayon::ThreadPoolBuilder::new().num_threads(t).build_global();
        }
        let source_cfg = var("RAIDO_SOURCE").unwrap_or_else(|| "s3".into());
        let prefixe = var("RAIDO_PREFIXE").unwrap_or_else(|| "graphe/courant".into());
        let interne: Box<dyn SourceTuiles> = if let Some(chemin) = source_cfg.strip_prefix("local:") {
            Box::new(SourceDisque(PathBuf::from(chemin)))
        } else {
            let bucket = var("RAIDO_BUCKET").ok_or_else(|| anyhow!("RAIDO_BUCKET absente"))?;
            let region = var("AWS_REGION").or_else(|| var("AWS_DEFAULT_REGION")).ok_or_else(|| anyhow!("AWS_REGION absente"))?;
            Box::new(SourceS3::nouvelle(&bucket, &region, &prefixe, var("RAIDO_S3_ENDPOINT"))?)
        };
        let cache = std::env::var("RAIDO_CACHE").unwrap_or_else(|_| "/tmp/raido".into());
        let source: Box<dyn SourceTuiles> = if cache.is_empty() {
            interne
        } else {
            // Un sous-dossier par préfixe : changer de version de graphe n'hérite pas d'un vieux cache.
            Box::new(SourceCache::nouvelle(interne, PathBuf::from(cache).join(prefixe.replace('/', "_"))))
        };
        let source_nom = source.nom();
        let graphe = Graphe::depuis_source(source).context("ouverture du graphe")?;
        journal("INFO", json!({"evenement": "demarrage", "source": source_nom, "graphe": graphe.index.construit_le, "tuiles": graphe.index.tuiles.len()}));
        Ok(Contexte { graphe: Mutex::new(graphe), source_nom, version_binaire: env!("CARGO_PKG_VERSION"), verificateur, cle_service: var("RAIDO_CLE_SERVICE") })
    }
}

/// Jeton « Bearer » de l'en-tête `authorization` (noms d'en-tête en minuscules dans l'événement v2).
fn jeton_de(evt: &Value) -> Option<&str> {
    let entetes = evt.get("headers")?.as_object()?;
    let a = entetes.iter().find(|(k, _)| k.eq_ignore_ascii_case("authorization"))?.1.as_str()?;
    let (schema, valeur) = a.split_once(' ')?;
    schema.eq_ignore_ascii_case("bearer").then(|| valeur.trim())
}

/// Corps de `POST /v1/apparier` : `trace` = suite de `[lat, lon]` en degrés.
#[derive(serde::Deserialize)]
struct DemandeApparier {
    trace: Vec<[f64; 2]>,
}

/// Comparaison de clés sans fuite par le temps de calcul.
fn egal_temps_constant(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |r, (x, y)| r | (x ^ y)) == 0
}

fn reponse(statut: u16, corps: Value) -> Value {
    json!({
        "statusCode": statut,
        "headers": {"content-type": "application/json; charset=utf-8", "cache-control": "no-store"},
        "body": corps.to_string(),
        "isBase64Encoded": false
    })
}

fn erreur(statut: u16, message: &str, code: &str) -> Value {
    reponse(statut, json!({"erreur": message, "code": code}))
}

/// Chemin et méthode d'un événement Function URL / API Gateway v2 (et, par tolérance, v1).
fn route(evt: &Value) -> (String, String) {
    let chemin = evt
        .get("rawPath")
        .or_else(|| evt.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("/")
        .to_string();
    let methode = evt
        .pointer("/requestContext/http/method")
        .or_else(|| evt.get("httpMethod"))
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_uppercase();
    (chemin, methode)
}

fn corps(evt: &Value) -> Result<Vec<u8>> {
    let b = evt.get("body").and_then(Value::as_str).unwrap_or("");
    if evt.get("isBase64Encoded").and_then(Value::as_bool).unwrap_or(false) {
        base64_decoder(b).ok_or_else(|| anyhow!("corps base64 illisible"))
    } else {
        Ok(b.as_bytes().to_vec())
    }
}

/// Traite un événement HTTP et renvoie la réponse au format Function URL.
pub fn traiter_http(ctx: &Contexte, evt: &Value) -> Value {
    let (chemin, methode) = route(evt);
    let debut = std::time::Instant::now();
    let rep = match (methode.as_str(), chemin.trim_end_matches('/')) {
        ("GET", "/v1/sante") => {
            let g = ctx.graphe.lock().unwrap_or_else(|e| e.into_inner());
            reponse(200, json!({
                "etat": "ok",
                "version": ctx.version_binaire,
                "graphe": {"version": g.index.construit_le, "source_osm": g.index.source, "format": g.index.version_format, "tuiles": g.index.tuiles.len(), "bornes": g.index.bornes},
                "tuiles_en_memoire": g.nb_tuiles_chargees(),
                "source": ctx.source_nom,
                "attribution": requete::ATTRIBUTION
            }))
        }
        ("POST", "/v1/balade") | ("POST", "/v1/trajet") => {
            let boucle = chemin.contains("balade");
            // Jeton court : obligatoire sauf en mode désactivé. Ses revendications bornent la requête (spec D4, §7.2).
            let revendications = match &ctx.verificateur {
                None => None,
                Some(v) => match v.verifier(jeton_de(evt).unwrap_or(""), jeton::maintenant()) {
                    Ok(r) => Some(r),
                    Err(ErreurJeton::Absent) => return erreur(401, "jeton requis (Authorization: Bearer …)", "jeton_requis"),
                    Err(e) => return erreur(401, &e.to_string(), "jeton_invalide"),
                },
            };
            match corps(evt).and_then(|c| serde_json::from_slice::<Requete>(&c).context("JSON de la requête")) {
                Err(e) => erreur(400, &format!("requête illisible : {e}"), "requete_invalide"),
                Ok(mut r) => {
                    // /v1/balade impose la boucle ; /v1/trajet impose une arrivée.
                    if boucle {
                        r.retour = true;
                    } else if r.arrivee.is_none() {
                        return erreur(400, "arrivee est obligatoire pour /v1/trajet", "requete_invalide");
                    } else {
                        r.retour = false;
                    }
                    if let Some(rev) = &revendications {
                        if rev.est_petite_boucle() {
                            let trop_long = !boucle || r.distance_cible_m().map_or(true, |d| d > rev.limite_km() * 1000.0);
                            if trop_long {
                                return erreur(403, &format!("ce jeton n'autorise qu'une boucle de {:.0} km au plus", rev.limite_km()), "quota_requis");
                            }
                        }
                        if rev.palier != "abonne" && r.preferences.decouverte > 0 {
                            return erreur(403, "la découverte est réservée aux abonnés", "abonnement_requis");
                        }
                    }
                    let mut g = ctx.graphe.lock().unwrap_or_else(|e| e.into_inner());
                    match requete::traiter(&mut g, &r) {
                        Ok(rep) => reponse(200, serde_json::to_value(&rep).unwrap_or(Value::Null)),
                        Err(e) if requete::est_erreur_requete(&e) => erreur(422, &e.to_string(), "calcul_impossible"),
                        Err(e) => {
                            journal("ERROR", json!({"evenement": "calcul", "chemin": chemin, "erreur": format!("{e:#}")}));
                            erreur(500, "erreur interne du calcul", "erreur_interne")
                        }
                    }
                }
            }
        }
        // Privée : appelée par le PHP de app.runeride.fr (jamais par le navigateur), avec la clé de service.
        ("POST", "/v1/apparier") => match ctx.cle_service.as_deref() {
            None => erreur(404, "route inconnue", "route_inconnue"),
            Some(cle) => {
                let fournie = evt.get("headers").and_then(Value::as_object).and_then(|h| h.iter().find(|(k, _)| k.eq_ignore_ascii_case("x-cle-service"))).and_then(|(_, v)| v.as_str()).unwrap_or("");
                if !egal_temps_constant(fournie.as_bytes(), cle.as_bytes()) {
                    return erreur(401, "clé de service absente ou invalide", "cle_invalide");
                }
                match corps(evt).and_then(|c| serde_json::from_slice::<DemandeApparier>(&c).context("JSON de la requête")) {
                    Err(e) => erreur(400, &format!("requête illisible : {e}"), "requete_invalide"),
                    Ok(d) => {
                        let trace: Vec<(f64, f64)> = d.trace.iter().map(|p| (p[0], p[1])).collect();
                        let mut g = ctx.graphe.lock().unwrap_or_else(|e| e.into_inner());
                        match apparier::apparier(&mut g, &trace) {
                            Ok(r) => reponse(200, json!({"troncons": r.troncons, "couverture": r.couverture, "points": r.points, "points_apparies": r.points_apparies, "graphe": {"version": g.index.construit_le}})),
                            Err(e) => erreur(422, &e.to_string(), "apparier_impossible"),
                        }
                    }
                }
            }
        },
        ("OPTIONS", _) => json!({"statusCode": 204, "headers": {}, "body": ""}),
        _ => erreur(404, "route inconnue", "route_inconnue"),
    };
    journal("INFO", json!({"evenement": "requete", "methode": methode, "chemin": chemin, "statut": rep.get("statusCode"), "ms": debut.elapsed().as_millis() as u64}));
    rep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cles_comparees_a_temps_constant() {
        assert!(egal_temps_constant(b"secret", b"secret"));
        assert!(!egal_temps_constant(b"secret", b"secreT"));
        assert!(!egal_temps_constant(b"secret", b"secre"));
        assert!(!egal_temps_constant(b"", b"x"));
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decoder("eyJhIjoxfQ==").unwrap(), b"{\"a\":1}");
        assert!(base64_decoder("@@").is_none());
    }

    #[test]
    fn jeton_bearer() {
        let e = json!({"headers": {"Authorization": "Bearer abc.def.ghi"}});
        assert_eq!(jeton_de(&e), Some("abc.def.ghi"));
        let e = json!({"headers": {"authorization": "Basic x"}});
        assert_eq!(jeton_de(&e), None);
        assert_eq!(jeton_de(&json!({})), None);
    }

    #[test]
    fn route_v2_et_v1() {
        let v2 = json!({"rawPath": "/v1/balade", "requestContext": {"http": {"method": "post"}}});
        assert_eq!(route(&v2), ("/v1/balade".into(), "POST".into()));
        let v1 = json!({"path": "/v1/sante", "httpMethod": "GET"});
        assert_eq!(route(&v1), ("/v1/sante".into(), "GET".into()));
    }
}
