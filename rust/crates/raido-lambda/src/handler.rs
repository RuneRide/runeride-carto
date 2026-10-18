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
use raido_calcul::requete::{self, Requete};
use serde_json::{json, Value};

use crate::s3::SourceS3;

pub struct Contexte {
    pub graphe: Mutex<Graphe>,
    pub source_nom: String,
    pub version_binaire: &'static str,
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
    /// `RAIDO_THREADS`, `AWS_REGION`.
    pub fn depuis_env() -> Result<Contexte> {
        let var = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
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
        Ok(Contexte { graphe: Mutex::new(graphe), source_nom, version_binaire: env!("CARGO_PKG_VERSION") })
    }
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

/// Décodeur base64 standard minimal (le corps JSON est petit ; pas de dépendance).
fn base64_decoder(s: &str) -> Option<Vec<u8>> {
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
    fn base64() {
        assert_eq!(base64_decoder("eyJhIjoxfQ==").unwrap(), b"{\"a\":1}");
        assert!(base64_decoder("@@").is_none());
    }

    #[test]
    fn route_v2_et_v1() {
        let v2 = json!({"rawPath": "/v1/balade", "requestContext": {"http": {"method": "post"}}});
        assert_eq!(route(&v2), ("/v1/balade".into(), "POST".into()));
        let v1 = json!({"path": "/v1/sante", "httpMethod": "GET"});
        assert_eq!(route(&v1), ("/v1/sante".into(), "GET".into()));
    }
}
