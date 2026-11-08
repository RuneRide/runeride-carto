// SPDX-License-Identifier: EUPL-1.2
//! Fournisseur météo HTTP : `POST {RAIDO_METEO_URL}/v1/route` de RuneRide Météo (spec météo §7).
//!
//! **Contrat provisoire** : la route n'existe pas encore (jalon J6 de runeride-weather) ; la spec dit « corps : trace (polyligne),
//! heure de départ, vitesse moyenne ; réponse : par point, heure de passage, T, pluie, vent, rafales, froid ressenti ». Les noms de
//! champs ci-dessous sont ceux que Raido a choisis (`pluie_mmh`, `rafales_kmh`, `froid_c`) ; ils tiennent dans cette seule fonction
//! d'adaptation et se corrigent quand la route sera écrite. Le corps porte l'en-tête `x-amz-content-sha256` exigé derrière CloudFront.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use raido_calcul::meteo::{Echantillon, FournisseurMeteo, PointMeteo};
use raido_calcul::polyligne;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub struct FournisseurHttp {
    url: String,
    agent: ureq::Agent,
}

#[derive(Deserialize)]
struct Reponse {
    points: Vec<PointRecu>,
}

#[derive(Deserialize)]
struct PointRecu {
    pluie_mmh: f64,
    rafales_kmh: f64,
    froid_c: f64,
}

impl FournisseurHttp {
    /// `base` : adresse de RuneRide Météo sans barre finale (par exemple `https://weather-data.runeride.fr`).
    pub fn nouveau(base: &str) -> FournisseurHttp {
        let agent = rw_aws::http::agent(
            rw_aws::http::Delais { connexion: Duration::from_secs(2), reponse: Duration::from_secs(5), global: Duration::from_secs(6) },
            "runeride-raido/0.1 (meteo)",
        );
        FournisseurHttp { url: format!("{}/v1/route", base.trim_end_matches('/')), agent }
    }
}

fn hex(o: &[u8]) -> String {
    o.iter().map(|b| format!("{b:02x}")).collect()
}

impl FournisseurMeteo for FournisseurHttp {
    fn par_point(&self, points: &[Echantillon], depart_a: Option<&str>, vitesse_kmh: f64) -> Result<Vec<PointMeteo>> {
        let trace: Vec<(f64, f64)> = points.iter().map(|p| (p.lat, p.lon)).collect();
        let mut corps = serde_json::json!({"trace": polyligne::encoder(&trace), "vitesse_kmh": (vitesse_kmh * 10.0).round() / 10.0});
        if let Some(d) = depart_a {
            corps["depart"] = d.into();
        }
        let octets = serde_json::to_vec(&corps)?;
        let mut rep = self
            .agent
            .post(&self.url)
            .header("content-type", "application/json")
            .header("x-amz-content-sha256", &hex(&Sha256::digest(&octets)))
            .send(&octets[..])
            .with_context(|| format!("météo {}", self.url))?;
        if rep.status().as_u16() != 200 {
            bail!("météo {} : HTTP {}", self.url, rep.status());
        }
        let r: Reponse = serde_json::from_str(&rep.body_mut().read_to_string().context("météo : corps")?).context("météo : JSON")?;
        if r.points.len() != points.len() {
            return Err(anyhow!("météo : {} points rendus pour {} envoyés", r.points.len(), points.len()));
        }
        Ok(points
            .iter()
            .zip(r.points)
            .map(|(e, m)| PointMeteo { lat: e.lat, lon: e.lon, distance_m: e.distance_m, pluie_mmh: m.pluie_mmh, rafales_kmh: m.rafales_kmh, froid_c: m.froid_c })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// Serveur HTTP minimal : lit une requête, la transmet au test, répond `statut` et `corps`.
    fn serveur(statut: u16, corps: String) -> (String, mpsc::Receiver<(String, Vec<u8>)>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let adresse = format!("http://{}", l.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut tampon = Vec::new();
            let mut bloc = [0u8; 4096];
            let (entetes, debut_corps) = loop {
                let n = s.read(&mut bloc).unwrap();
                tampon.extend_from_slice(&bloc[..n]);
                if let Some(i) = tampon.windows(4).position(|w| w == b"\r\n\r\n") {
                    break (String::from_utf8_lossy(&tampon[..i]).to_string(), i + 4);
                }
            };
            let longueur = entetes.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
            while tampon.len() < debut_corps + longueur {
                let n = s.read(&mut bloc).unwrap();
                tampon.extend_from_slice(&bloc[..n]);
            }
            tx.send((entetes, tampon[debut_corps..debut_corps + longueur].to_vec())).unwrap();
            write!(s, "HTTP/1.1 {statut} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{corps}", corps.len()).unwrap();
        });
        (adresse, rx)
    }

    fn echantillons() -> Vec<Echantillon> {
        (0..3).map(|i| Echantillon { lat: 45.0 + i as f64 * 0.01, lon: 5.0, distance_m: i as f64 * 1100.0 }).collect()
    }

    #[test]
    fn envoie_la_trace_et_range_les_points() {
        let (url, rx) = serveur(200, r#"{"points":[{"pluie_mmh":0,"rafales_kmh":20,"froid_c":15},{"pluie_mmh":3.5,"rafales_kmh":40,"froid_c":12},{"pluie_mmh":0,"rafales_kmh":70,"froid_c":4}]}"#.into());
        let pts = FournisseurHttp::nouveau(&url).par_point(&echantillons(), Some("2026-11-08T14:00"), 52.34).unwrap();
        assert_eq!(pts.len(), 3);
        assert_eq!((pts[1].pluie_mmh, pts[2].rafales_kmh, pts[2].froid_c), (3.5, 70.0, 4.0));
        assert_eq!(pts[1].distance_m, 1100.0, "la distance vient de l'échantillon envoyé");
        let (entetes, corps) = rx.recv().unwrap();
        assert!(entetes.starts_with("POST /v1/route "), "{entetes}");
        let sha = hex(&Sha256::digest(&corps));
        assert!(entetes.to_ascii_lowercase().contains(&format!("x-amz-content-sha256: {sha}")), "condensé du corps exigé derrière CloudFront");
        let j: serde_json::Value = serde_json::from_slice(&corps).unwrap();
        assert_eq!(j["depart"], "2026-11-08T14:00");
        assert_eq!(j["vitesse_kmh"], 52.3);
        assert_eq!(polyligne::decoder(j["trace"].as_str().unwrap()).len(), 3);
    }

    #[test]
    fn erreurs_http_et_reponses_incoherentes() {
        for (statut, corps) in [(503, r#"{"erreur":"x"}"#.to_string()), (200, r#"{"points":[]}"#.to_string()), (200, "pas du json".to_string())] {
            let (url, _rx) = serveur(statut, corps);
            assert!(FournisseurHttp::nouveau(&url).par_point(&echantillons(), None, 50.0).is_err(), "{statut}");
        }
        // aucun serveur à cette adresse
        assert!(FournisseurHttp::nouveau("http://127.0.0.1:9").par_point(&echantillons(), None, 50.0).is_err());
    }
}
