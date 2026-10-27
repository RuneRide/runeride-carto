// SPDX-License-Identifier: EUPL-1.2
//! Contrat JSON de `POST /v1/balade` et `POST /v1/trajet` (spec §7.2), partagé par la ligne
//! de commande et la Lambda. J3 : départ, retour ou arrivée, distance ou durée, préférences,
//! évitements, tronçons connus, variante. Étapes, autonomie et météo viennent aux jalons suivants.

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use anyhow::{bail, Result};
use raido_format::{haversine_m, Classe, Point};
use serde::{Deserialize, Serialize};

use crate::astar;
use crate::boucle::{self, ParametresBoucle};
use crate::charge::Graphe;
use crate::cout::{Preferences, V_MAX_MS};
use crate::meteo::{self, FournisseurMeteo, PointMeteo, Seuils};
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
    /// Vitesses moyennes réglées par l'utilisateur (km/h, 20 à 130, ramenées à 110 pour garder l'heuristique admissible),
    /// par classe : `autoroute`, `nationale`, `primaire`, `secondaire`, `tertiaire`, `non_classee`, `residentielle`.
    #[serde(default)]
    pub vitesses: BTreeMap<String, f64>,
    /// Pause par heure de route, en minutes (0 à 30, défaut 10 : spec O3).
    pub pause_min_par_h: Option<f64>,
    /// Seuils de la contrainte météo (spec §4.5, O2 : pluie 1 mm/h, rafales 60 km/h, froid ressenti 5 °C). Sans ce champ,
    /// la météo n'est pas consultée.
    pub meteo: Option<Seuils>,
    /// Heure de départ, ISO 8601 (`2026-10-26T14:00`), pour la météo à l'heure de passage.
    pub depart_a: Option<String>,
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
    /// Col ou panorama sur lequel la boucle a été bâtie (curseur scénique ≥ 2), le cas échéant.
    pub passe_par: Option<String>,
    /// Météo à l'heure de passage, point par point (présente quand la contrainte météo a été évaluée).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub meteo: Vec<PointMeteo>,
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
            vitesses: self.vitesses.iter().filter_map(|(n, v)| Classe::depuis_nom(n).map(|c| (c, v.min(V_MAX_MS * 3.6)))).collect(),
            arcs_meteo: HashSet::new(),
        }
    }

    /// Pause ajoutée par heure roulée, en fraction de l'heure.
    pub fn pause_par_heure(&self) -> f64 {
        self.pause_min_par_h.unwrap_or(10.0).clamp(0.0, 30.0) / 60.0
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
            return Ok(m / 60.0 / (1.0 + self.pause_par_heure()) * VITESSE_CIBLE_KMH * 1000.0);
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
        for (nom, v) in &self.vitesses {
            if Classe::depuis_nom(nom).is_none() {
                bail!("vitesses : classe inconnue « {nom} »");
            }
            if !v.is_finite() || !(20.0..=130.0).contains(v) {
                bail!("vitesses : {nom} doit être entre 20 et 130 km/h");
            }
        }
        if let Some(p) = self.pause_min_par_h {
            if !p.is_finite() || !(0.0..=30.0).contains(&p) {
                bail!("pause_min_par_h doit être entre 0 et 30");
            }
        }
        if let Some(s) = &self.meteo {
            s.verifier()?;
        }
        if let Some(d) = &self.depart_a {
            if !meteo::depart_a_valide(d) {
                bail!("depart_a doit être une date ISO 8601 (AAAA-MM-JJTHH:MM)");
            }
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

fn duree_avec_pauses(temps_ds: u64, pause_par_heure: f64) -> u64 {
    (temps_ds as f64 / 10.0 * (1.0 + pause_par_heure)).round() as u64
}

/// Temps de base (ds) d'un chemin avec les vitesses réglées dans la requête.
fn temps_pref(g: &Graphe, p: &Preferences, arcs: &[raido_format::ArcId]) -> u64 {
    p.temps_chemin_ds(arcs.iter().filter_map(|&id| g.arc(id)))
}

/// Traite une requête de boucle (`retour`) ou de trajet (`arrivee`) ; le graphe doit déjà
/// contenir les tuiles nécessaires ou pouvoir les charger (`precharger` est appelé ici).
pub fn traiter(g: &mut Graphe, r: &Requete) -> Result<Reponse> {
    traiter_zones(g, r, &[])
}

/// Un calcul avec des zones d'évitement météo `(lat, lon, rayon_m)` : les arcs qui en partent sont pénalisés.
fn traiter_zones(g: &mut Graphe, r: &Requete, zones: &[(f64, f64, f64)]) -> Result<Reponse> {
    r.verifier().map_err(|e| ErreurRequete(e.to_string()))?;
    let mut p = r.preferences();
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
        p.arcs_meteo = arcs_des_zones(g, zones);
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
            duree_s: duree_avec_pauses(temps_pref(g, &p, &ch.arcs), r.pause_par_heure()),
            denivele_m: denivele_de(g, &ch.arcs),
            scores: scores.scores,
            variantes: 1,
            variante: 0,
            passe_par: None,
            meteo: Vec::new(),
            avertissements,
            graphe: info(g),
            mesures: Mesures { chargement_ms, calcul_ms: t1.elapsed().as_millis() as u64, tuiles_chargees: g.nb_tuiles_chargees(), octets_charges: g.octets_charges, etats_explores: ch.noeuds_explores },
            attribution: ATTRIBUTION,
        });
    }

    // Boucle.
    let dist = r.distance_cible_m().map_err(|e| ErreurRequete(e.to_string()))?;
    g.precharger(boucle::emprise(d.0, d.1, dist))?;
    p.arcs_meteo = arcs_des_zones(g, zones);
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
        duree_s: duree_avec_pauses(temps_pref(g, &p, &c.arcs), r.pause_par_heure()),
        denivele_m: denivele_de(g, &c.arcs),
        scores: Scores { recouvrement: c.recouvrement, ..scores.scores },
        variantes: cands.len().min(3),
        variante: i,
        passe_par: c.passe_par.clone(),
        meteo: Vec::new(),
        avertissements,
        graphe: info(g),
        mesures: Mesures { chargement_ms, calcul_ms: t1.elapsed().as_millis() as u64, tuiles_chargees: g.nb_tuiles_chargees(), octets_charges: g.octets_charges, etats_explores: c.noeuds_explores },
        attribution: ATTRIBUTION,
    })
}

/// Montée cumulée (m) d'un chemin, ou `None` si le graphe a été construit sans relief.
fn denivele_de(g: &Graphe, arcs: &[raido_format::ArcId]) -> Option<u32> {
    g.index.relief.then(|| arcs.iter().filter_map(|&id| g.arc(id)).map(|a| a.montee_m.to_native() as u32).sum())
}

fn arcs_des_zones(g: &Graphe, zones: &[(f64, f64, f64)]) -> HashSet<raido_format::ArcId> {
    zones.iter().flat_map(|&(la, lo, rayon)| g.arcs_dans_rayon(la, lo, rayon)).collect()
}

/// Même chose que `traiter`, avec la contrainte météo (spec §4.5) quand la requête porte `meteo` : on calcule,
/// on interroge la météo le long de la trace, on pose des zones d'évitement autour des points en infraction et
/// on recalcule, trois calculs au plus. On rend le candidat qui a le moins de points en infraction (le premier à
/// égalité) ; s'il en reste, l'avertissement « météo non évitable ». Sans `meteo`, ou si la météo est
/// indisponible, le résultat est celui de `traiter` (avec un avertissement dans le second cas).
pub fn traiter_meteo(g: &mut Graphe, r: &Requete, f: &dyn FournisseurMeteo) -> Result<Reponse> {
    let Some(seuils) = r.meteo else { return traiter(g, r) };
    let mut zones: Vec<(f64, f64, f64)> = Vec::new();
    let mut meilleur: Option<(usize, Reponse)> = None;
    for _ in 0..meteo::CALCULS_MAX {
        let mut rep = traiter_zones(g, r, &zones)?;
        let echantillons = meteo::echantillonner(&polyligne::decoder(&rep.trace));
        let vitesse = rep.distance_m as f64 / rep.duree_s.max(1) as f64 * 3.6;
        let pts = match f.par_point(&echantillons, r.depart_a.as_deref(), vitesse) {
            Ok(p) => p,
            Err(_) => {
                // Pas de météo : on garde le meilleur calcul déjà obtenu, sinon celui-ci, et on le dit.
                let mut sortie = meilleur.map(|(_, m)| m).unwrap_or(rep);
                sortie.avertissements.push("météo indisponible : contrainte météo ignorée".into());
                return Ok(sortie);
            }
        };
        let inf = meteo::infractions(&pts, &seuils);
        let n = inf.len();
        let nouvelles = meteo::zones_depuis(&inf);
        rep.meteo = pts;
        if meilleur.as_ref().is_none_or(|(m, _)| n < *m) {
            meilleur = Some((n, rep));
        }
        if n == 0 {
            break;
        }
        zones.extend(nouvelles);
    }
    let (n, mut rep) = meilleur.expect("au moins un calcul");
    if n > 0 {
        rep.avertissements.push(format!("météo non évitable : {n} point(s) de la trace dépassent les seuils"));
    }
    Ok(rep)
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
    fn requete(json: &str) -> Requete {
        serde_json::from_str(json).expect("requête JSON")
    }

    #[test]
    fn vitesses_et_pause_sont_lues_et_validees() {
        let r = requete(r#"{"depart":{"lat":45.0,"lon":5.0},"retour":true,"distance_km":30,"vitesses":{"secondaire":60,"tertiaire":125},"pause_min_par_h":0}"#);
        assert!(r.verifier().is_ok());
        let p = r.preferences();
        assert_eq!(p.vitesses.len(), 2);
        assert!(p.vitesses.iter().all(|(_, v)| *v <= 110.0 + 1e-9), "bornées à 110 km/h");
        assert_eq!(r.pause_par_heure(), 0.0);
        assert_eq!(requete(r#"{"depart":{"lat":45.0,"lon":5.0},"retour":true}"#).pause_par_heure(), 10.0 / 60.0, "défaut : 10 min par heure");
        for mauvais in [r#""vitesses":{"piste":50}"#, r#""vitesses":{"secondaire":10}"#, r#""vitesses":{"secondaire":131}"#, r#""pause_min_par_h":31"#, r#""pause_min_par_h":-1"#] {
            let r = requete(&format!(r#"{{"depart":{{"lat":45.0,"lon":5.0}},"retour":true,"distance_km":30,{mauvais}}}"#));
            assert!(r.verifier().is_err(), "{mauvais}");
        }
    }

    use super::*;

    #[test]
    fn meteo_et_depart_a_sont_lus_et_valides() {
        let r = requete(r#"{"depart":{"lat":45.0,"lon":5.0},"retour":true,"distance_km":30,"meteo":{"pluie_mmh":2},"depart_a":"2026-10-26T14:00"}"#);
        assert!(r.verifier().is_ok());
        let s = r.meteo.unwrap();
        assert_eq!((s.pluie_mmh, s.rafales_kmh, s.froid_c), (2.0, 60.0, 5.0), "seuils absents : défauts O2");
        for mauvais in [r#""meteo":{"rafales_kmh":5}"#, r#""depart_a":"demain""#] {
            let r = requete(&format!(r#"{{"depart":{{"lat":45.0,"lon":5.0}},"retour":true,"distance_km":30,{mauvais}}}"#));
            assert!(r.verifier().is_err(), "{mauvais}");
        }
        assert!(requete(r#"{"depart":{"lat":45.0,"lon":5.0},"retour":true}"#).meteo.is_none(), "sans meteo : pas de contrainte");
    }

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
        assert_eq!(duree_avec_pauses(36_000, 10.0 / 60.0), 4_200); // 1 h roulée → 1 h 10
        assert_eq!(duree_avec_pauses(36_000, 0.0), 3_600, "sans pause");
    }
}
