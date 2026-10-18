// SPDX-License-Identifier: EUPL-1.2
//! Contrat JSON de `POST /v1/balade` et `POST /v1/trajet` (spec §7.2), partagé par la ligne
//! de commande et la Lambda. J3 : départ, retour ou arrivée, distance ou durée, préférences,
//! évitements, tronçons connus, variante. Étapes, autonomie et météo viennent aux jalons suivants.

use std::collections::HashSet;
use std::time::Instant;

use anyhow::{bail, Result};
use raido_format::{haversine_m, Point};
use serde::{Deserialize, Serialize};

use crate::astar;
use crate::boucle::{self, ParametresBoucle};
use crate::charge::Graphe;
use crate::cout::Preferences;
use crate::{geo, polyligne};

pub const ATTRIBUTION: &str = "© OpenStreetMap contributors (ODbL) · © IGN BD ALTI · Météo-France (Licence Ouverte) · Copernicus GLO-30";

#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default)]
pub struct Coord {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct PreferencesJson {
    #[serde(default)]
    pub sinuosite: u8,
    #[serde(default)]
    pub scenique: u8,
    #[serde(default)]
    pub motard: u8,
    #[serde(default)]
    pub decouverte: u8,
    /// Pénalité des traversées d'agglomération (cran 0..4, défaut 3).
    #[serde(default = "trois")]
    pub agglo: u8,
}

fn trois() -> u8 {
    3
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct Requete {
    pub depart: Coord,
    /// Boucle : retour au départ.
    #[serde(default)]
    pub retour: bool,
    pub arrivee: Option<Coord>,
    pub distance_km: Option<f64>,
    pub duree_min: Option<f64>,
    /// Tolérance relative sur la distance ou la durée (défaut 0,15).
    pub tolerance: Option<f64>,
    #[serde(default)]
    pub preferences: PreferencesJson,
    /// Parmi : `autoroutes`, `peages`, `non_revetu`, `tunnels`.
    #[serde(default)]
    pub evitements: Vec<String>,
    /// Tronçons déjà parcourus, `[way_id, segment]`, pour la découverte.
    #[serde(default)]
    pub troncons_connus: Vec<(i64, u16)>,
    /// Indice de la variante voulue (0 = meilleure).
    #[serde(default)]
    pub variante: usize,
    /// Pondération de l'heuristique (1 = exact).
    pub epsilon: Option<f64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Scores {
    pub sinuosite: f64,
    pub scenique: f64,
    pub motard: f64,
    pub agglo: f64,
    pub nouveaute: f64,
    pub recouvrement: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Reponse {
    /// Polyligne encodée (précision 1e-5).
    pub trace: String,
    pub distance_m: u64,
    pub duree_s: u64,
    pub denivele_m: Option<u32>,
    pub scores: Scores,
    pub variantes: usize,
    pub variante: usize,
    pub avertissements: Vec<String>,
    pub graphe: GrapheInfo,
    pub mesures: Mesures,
    pub attribution: &'static str,
}

#[derive(Serialize, Clone, Debug)]
pub struct GrapheInfo {
    pub version: String,
    pub source: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Mesures {
    pub chargement_ms: u64,
    pub calcul_ms: u64,
    pub tuiles_chargees: usize,
    pub octets_charges: u64,
    pub etats_explores: usize,
}

/// Vitesse de croisière (km/h) utilisée pour convertir une durée demandée en distance
/// cible avant calcul ; la durée rendue est celle du graphe (vitesses par classe + pauses).
const VITESSE_CIBLE_KMH: f64 = 55.0;
/// Pause ajoutée à la durée rendue : 10 min par heure roulée (spec O3).
const PAUSE_PAR_HEURE: f64 = 10.0 / 60.0;

impl Requete {
    pub fn preferences(&self) -> Preferences {
        let ev = |n: &str| self.evitements.iter().any(|e| e == n);
        Preferences {
            sinuosite: self.preferences.sinuosite,
            scenique: self.preferences.scenique,
            motard: self.preferences.motard,
            decouverte: self.preferences.decouverte,
            agglo: self.preferences.agglo,
            eviter_autoroutes: ev("autoroutes"),
            eviter_peages: ev("peages"),
            eviter_non_revetu: ev("non_revetu"),
            eviter_tunnels: ev("tunnels"),
            troncons_connus: self.troncons_connus.iter().copied().collect(),
        }
    }

    /// Distance cible en mètres d'une boucle (distance explicite, sinon durée × vitesse de croisière).
    pub fn distance_cible_m(&self) -> Result<f64> {
        if let Some(d) = self.distance_km {
            if !(2.0..=600.0).contains(&d) {
                bail!("distance_km doit être entre 2 et 600");
            }
            return Ok(d * 1000.0);
        }
        if let Some(m) = self.duree_min {
            if !(10.0..=720.0).contains(&m) {
                bail!("duree_min doit être entre 10 et 720");
            }
            // la durée demandée inclut les pauses : on les retire avant de convertir en distance
            return Ok(m / 60.0 / (1.0 + PAUSE_PAR_HEURE) * VITESSE_CIBLE_KMH * 1000.0);
        }
        bail!("distance_km ou duree_min est obligatoire pour une boucle")
    }

    fn verifier(&self) -> Result<()> {
        let ok = |c: &Coord| c.lat.is_finite() && c.lon.is_finite() && c.lat.abs() <= 90.0 && c.lon.abs() <= 180.0;
        if !ok(&self.depart) {
            bail!("depart invalide");
        }
        if let Some(a) = &self.arrivee {
            if !ok(a) {
                bail!("arrivee invalide");
            }
        }
        if !self.retour && self.arrivee.is_none() {
            bail!("retour (boucle) ou arrivee est obligatoire");
        }
        if self.variante > 2 {
            bail!("variante doit être 0, 1 ou 2");
        }
        if self.troncons_connus.len() > 200_000 {
            bail!("troncons_connus trop long");
        }
        Ok(())
    }
}

/// Erreur imputable à la requête (400) plutôt qu'au service (500).
#[derive(Debug)]
pub struct ErreurRequete(pub String);

impl std::fmt::Display for ErreurRequete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ErreurRequete {}

fn duree_avec_pauses(temps_ds: u64) -> u64 {
    (temps_ds as f64 / 10.0 * (1.0 + PAUSE_PAR_HEURE)).round() as u64
}

/// Traite une requête de boucle (`retour`) ou de trajet (`arrivee`) ; le graphe doit déjà
/// contenir les tuiles nécessaires ou pouvoir les charger (`precharger` est appelé ici).
pub fn traiter(g: &mut Graphe, r: &Requete) -> Result<Reponse> {
    r.verifier().map_err(|e| ErreurRequete(e.to_string()))?;
    let p = r.preferences();
    let epsilon = r.epsilon.unwrap_or(1.0).clamp(1.0, 3.0);
    let d = (r.depart.lat, r.depart.lon);
    let mut avertissements = Vec::new();
    let t0 = Instant::now();
    let depart_dans_index = {
        let b = g.index.bornes;
        d.0 >= b[0] && d.0 <= b[2] && d.1 >= b[1] && d.1 <= b[3]
    };
    if !depart_dans_index {
        return Err(ErreurRequete("départ hors de la zone couverte par le graphe".into()).into());
    }

    if let Some(a) = r.arrivee.filter(|_| !r.retour) {
        let a = (a.lat, a.lon);
        let vol = haversine_m(Point::depuis_degres(d.0, d.1), Point::depuis_degres(a.0, a.1));
        if vol > 400_000.0 {
            return Err(ErreurRequete("trajet de plus de 400 km à vol d'oiseau : non pris en charge".into()).into());
        }
        g.precharger(geo::emprise_entre(d, a, vol * 0.3 + 10_000.0))?;
        let chargement_ms = t0.elapsed().as_millis() as u64;
        let nd = boucle::noeud_accroche(g, &p, d.0, d.1, 2_000.0).ok_or_else(|| ErreurRequete("aucune route près du départ".into()))?;
        let na = boucle::noeud_accroche(g, &p, a.0, a.1, 2_000.0).ok_or_else(|| ErreurRequete("aucune route près de l'arrivée".into()))?;
        let t1 = Instant::now();
        let ch = astar::chemin(g, &p, nd, na, &HashSet::new(), epsilon).ok_or_else(|| ErreurRequete("aucun chemin entre ces deux points avec ces contraintes".into()))?;
        let trace = astar::trace(g, &ch.arcs, nd);
        let scores = scores_de(g, &ch.arcs, &p);
        if scores.revetement_inconnu > 0.10 {
            avertissements.push(format!("revêtement inconnu sur {:.0} % du tracé", scores.revetement_inconnu * 100.0));
        }
        return Ok(Reponse {
            trace: polyligne::encoder(&trace),
            distance_m: ch.longueur_m,
            duree_s: duree_avec_pauses(ch.temps_ds),
            denivele_m: None,
            scores: scores.scores,
            variantes: 1,
            variante: 0,
            avertissements,
            graphe: info(g),
            mesures: Mesures { chargement_ms, calcul_ms: t1.elapsed().as_millis() as u64, tuiles_chargees: g.nb_tuiles_chargees(), octets_charges: g.octets_charges, etats_explores: ch.noeuds_explores },
            attribution: ATTRIBUTION,
        });
    }

    // Boucle.
    let dist = r.distance_cible_m().map_err(|e| ErreurRequete(e.to_string()))?;
    g.precharger(boucle::emprise(d.0, d.1, dist))?;
    let chargement_ms = t0.elapsed().as_millis() as u64;
    let nd = boucle::noeud_accroche(g, &p, d.0, d.1, 2_000.0).ok_or_else(|| ErreurRequete("aucune route près du départ".into()))?;
    let mut pb = ParametresBoucle::pour(dist);
    pb.epsilon = epsilon;
    if let Some(t) = r.tolerance {
        pb.tolerance = t.clamp(0.02, 0.5);
    }
    let t1 = Instant::now();
    let cands = boucle::balade(g, &p, nd, &pb)?;
    if cands.is_empty() {
        return Err(ErreurRequete("aucune boucle trouvée depuis ce point avec ces contraintes".into()).into());
    }
    let i = r.variante.min(cands.len() - 1);
    let c = &cands[i];
    let ecart = (c.longueur_m as f64 - dist) / dist;
    if ecart.abs() > pb.tolerance {
        avertissements.push(format!("distance obtenue à {:+.0} % de la cible", ecart * 100.0));
    }
    let trace = astar::trace(g, &c.arcs, nd);
    let scores = scores_de(g, &c.arcs, &p);
    if scores.revetement_inconnu > 0.10 {
        avertissements.push(format!("revêtement inconnu sur {:.0} % du tracé", scores.revetement_inconnu * 100.0));
    }
    Ok(Reponse {
        trace: polyligne::encoder(&trace),
        distance_m: c.longueur_m,
        duree_s: duree_avec_pauses(c.temps_ds),
        denivele_m: None,
        scores: Scores { recouvrement: c.recouvrement, ..scores.scores },
        variantes: cands.len().min(3),
        variante: i,
        avertissements,
        graphe: info(g),
        mesures: Mesures { chargement_ms, calcul_ms: t1.elapsed().as_millis() as u64, tuiles_chargees: g.nb_tuiles_chargees(), octets_charges: g.octets_charges, etats_explores: c.noeuds_explores },
        attribution: ATTRIBUTION,
    })
}

fn info(g: &Graphe) -> GrapheInfo {
    GrapheInfo { version: g.index.construit_le.clone(), source: g.index.source.clone() }
}

struct ScoresCalcules {
    scores: Scores,
    revetement_inconnu: f64,
}

fn scores_de(g: &Graphe, arcs: &[raido_format::ArcId], p: &Preferences) -> ScoresCalcules {
    use raido_format::{drapeau, score};
    let (mut l_tot, mut s, mut sc, mut mo, mut ag, mut nouveau, mut inconnu) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for &id in arcs {
        let Some(a) = g.arc(id) else { continue };
        let l = a.longueur_m.to_native() as f64;
        l_tot += l;
        s += l * a.scores[score::SINUOSITE] as f64 / 255.0;
        sc += l * a.scores[score::SCENIQUE] as f64 / 255.0;
        mo += l * a.scores[score::MOTARD] as f64 / 255.0;
        ag += l * a.scores[score::AGGLO] as f64 / 255.0;
        if !p.troncons_connus.contains(&(a.way_id.to_native(), a.segment.to_native())) {
            nouveau += l;
        }
        if a.drapeaux & drapeau::REVETEMENT_INCONNU != 0 {
            inconnu += l;
        }
    }
    let l = l_tot.max(1.0);
    ScoresCalcules {
        scores: Scores { sinuosite: s / l, scenique: sc / l, motard: mo / l, agglo: ag / l, nouveaute: if p.troncons_connus.is_empty() { 1.0 } else { nouveau / l }, recouvrement: 0.0 },
        revetement_inconnu: inconnu / l,
    }
}

/// Erreur 400 ou 500 ? Les erreurs de requête sont typées `ErreurRequete`.
pub fn est_erreur_requete(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ErreurRequete>().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requete_minimale() {
        let r: Requete = serde_json::from_str(r#"{"depart":{"lat":45.19,"lon":5.72},"retour":true,"duree_min":180}"#).unwrap();
        assert!(r.verifier().is_ok());
        let d = r.distance_cible_m().unwrap();
        // 3 h dont 10 min/h de pause → 2 h 34 de roulage à 55 km/h ≈ 141 km
        assert!((d - 141_428.0).abs() < 100.0, "{d}");
    }

    #[test]
    fn requete_invalide() {
        let r: Requete = serde_json::from_str(r#"{"depart":{"lat":45.19,"lon":5.72}}"#).unwrap();
        assert!(r.verifier().is_err());
        let r: Requete = serde_json::from_str(r#"{"depart":{"lat":95,"lon":5.72},"retour":true}"#).unwrap();
        assert!(r.verifier().is_err());
    }

    #[test]
    fn evitements() {
        let r: Requete = serde_json::from_str(r#"{"depart":{"lat":45,"lon":5},"retour":true,"evitements":["autoroutes","peages"]}"#).unwrap();
        let p = r.preferences();
        assert!(p.eviter_autoroutes && p.eviter_peages && !p.eviter_tunnels);
    }

    #[test]
    fn pauses() {
        assert_eq!(duree_avec_pauses(36_000), 4_200); // 1 h roulée → 1 h 10
    }
}
