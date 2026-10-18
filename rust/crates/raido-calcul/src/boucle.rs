// SPDX-License-Identifier: EUPL-1.2
//! Boucles par pétales (spec §5.4) : deux points de passage virtuels posés sur un
//! triangle orienté selon un cap, trois trajets A* avec pénalité de recouvrement,
//! puis classement des candidats.

use std::collections::HashSet;

use anyhow::Result;
use raido_format::{drapeau_noeud, score, ArcId, NoeudId, ID_NUL};
use rayon::prelude::*;

use crate::astar::{self, Chemin};
use crate::charge::Graphe;
use crate::cout::{self, Preferences};
use crate::geo;

/// Nœud d'accroche d'un point : le plus proche de la composante principale qui
/// possède un arc franchissable à double sens (atteignable et quittable) et qui
/// n'est pas enfermé par les exclusions (au moins `MIN_CONNEXE` nœuds joignables).
/// On cherche à `max_m`, puis à `3 × max_m` ; `None` si rien ne convient (haute
/// montagne, lac…) : mieux vaut le dire que d'accrocher un îlot en silence.
pub fn noeud_accroche(g: &Graphe, p: &Preferences, lat: f64, lon: f64, max_m: f64) -> Option<NoeudId> {
    let vide = HashSet::new();
    for rayon in [max_m, max_m * 3.0] {
        let trouve = g.noeuds_proches(lat, lon, rayon, 32).into_iter().find(|(n, _)| {
            g.noeud(*n).is_some_and(|x| x.drapeaux & drapeau_noeud::PRINCIPAL != 0)
                && g.sortants(*n).any(|(id, a)| a.inverse.to_native() != ID_NUL && cout::cout(p, id, a, &vide).is_some())
                && connexe(g, p, *n)
        });
        if let Some((n, _)) = trouve {
            return Some(n);
        }
    }
    None
}

const MIN_CONNEXE: usize = 60;

/// Parcours en largeur borné : le nœud atteint-il au moins `MIN_CONNEXE` nœuds ?
fn connexe(g: &Graphe, p: &Preferences, depart: NoeudId) -> bool {
    let vide = HashSet::new();
    let mut vus: HashSet<NoeudId> = HashSet::from([depart]);
    let mut file = std::collections::VecDeque::from([depart]);
    while let Some(n) = file.pop_front() {
        for (id, a) in g.sortants(n) {
            if cout::cout(p, id, a, &vide).is_none() {
                continue;
            }
            let c = a.cible.to_native();
            if g.noeud(c).is_some() && vus.insert(c) {
                if vus.len() >= MIN_CONNEXE {
                    return true;
                }
                file.push_back(c);
            }
        }
    }
    false
}

#[derive(Clone, Debug)]
pub struct Candidat {
    pub cap_deg: f64,
    /// Facteur appliqué au rayon des pétales pour ce candidat.
    pub facteur: f64,
    pub arcs: Vec<ArcId>,
    pub longueur_m: u64,
    pub temps_ds: u64,
    /// Part de la longueur parcourue deux fois (aller et retour sur le même tronçon).
    pub recouvrement: f64,
    /// Sinuosité moyenne pondérée par la longueur, 0..1.
    pub sinuosite: f64,
    /// Part de la longueur dans une agglomération, 0..1.
    pub agglo: f64,
    /// Note composite : plus petite = meilleure.
    pub note: f64,
    pub noeuds_explores: usize,
}

pub struct ParametresBoucle {
    pub distance_m: f64,
    /// Nombre de caps essayés (défaut 12, soit tous les 30°).
    pub nb_caps: usize,
    /// Facteurs appliqués au rayon des pétales (défaut 0,9 et 1,1).
    pub facteurs: Vec<f64>,
    /// Tolérance relative sur la distance (défaut 0,15).
    pub tolerance: f64,
    /// Pondération de l'heuristique A* (1 = exact).
    pub epsilon: f64,
}

impl ParametresBoucle {
    pub fn pour(distance_m: f64) -> ParametresBoucle {
        ParametresBoucle { distance_m, nb_caps: 12, facteurs: vec![0.9, 1.1], tolerance: 0.15, epsilon: 1.0 }
    }
}

/// Emprise à précharger pour une boucle depuis `(lat, lon)`.
pub fn emprise(lat: f64, lon: f64, distance_m: f64) -> [f64; 4] {
    geo::emprise_autour(lat, lon, distance_m * 0.45 + 10_000.0)
}

/// Calcule les candidats de boucle depuis `depart`, triés par note croissante.
pub fn balade(g: &Graphe, p: &Preferences, depart: NoeudId, pb: &ParametresBoucle) -> Result<Vec<Candidat>> {
    let pos = g.position(depart).ok_or_else(|| anyhow::anyhow!("nœud de départ inconnu"))?;
    let (lat, lon) = (pos.lat_deg(), pos.lon_deg());
    // Pétales : un (cap, facteur) par essai, calculés en parallèle (le graphe est en lecture seule).
    let essais: Vec<(f64, f64)> = (0..pb.nb_caps)
        .flat_map(|i| pb.facteurs.iter().map(move |&f| (360.0 * i as f64 / pb.nb_caps as f64, f)))
        .collect();
    let petale = |cap: f64, f: f64| -> Option<Candidat> {
        // Triangle quasi équilatéral de périmètre ≈ distance : rayon des sommets ≈ L/3,
        // corrigé du détour routier (0,72 : avec 0,8 les boucles dépassaient de 5 à 15 %).
        let r = pb.distance_m / 3.0 * f * 0.72;
        let (la1, lo1) = geo::destination(lat, lon, cap, r);
        let (la2, lo2) = geo::destination(lat, lon, cap + 60.0, r);
        let p1 = noeud_accroche(g, p, la1, lo1, 3_000.0)?;
        let p2 = noeud_accroche(g, p, la2, lo2, 3_000.0)?;
        if p1 == depart || p2 == depart || p1 == p2 {
            return None;
        }
        boucle_par(g, p, depart, p1, p2, cap, f, pb)
    };
    let mut candidats: Vec<Candidat> = essais.par_iter().filter_map(|&(cap, f)| petale(cap, f)).collect();
    candidats.sort_by(|a, b| a.note.partial_cmp(&b.note).unwrap_or(std::cmp::Ordering::Equal));

    // Raffinement : les trois meilleurs trop longs ou trop courts (> 4 %) sont rejoués
    // avec un rayon corrigé du rapport visé / obtenu ; on garde le meilleur des deux.
    let a_raffiner: Vec<(usize, f64, f64)> = candidats
        .iter()
        .take(3)
        .enumerate()
        .filter(|(_, c)| ((c.longueur_m as f64 - pb.distance_m) / pb.distance_m).abs() > 0.04)
        .map(|(i, c)| (i, c.cap_deg, c.facteur * pb.distance_m / c.longueur_m as f64))
        .collect();
    let raffines: Vec<(usize, Candidat)> =
        a_raffiner.par_iter().filter_map(|&(i, cap, f)| petale(cap, f).map(|c| (i, c))).collect();
    for (i, c) in raffines {
        if c.note < candidats[i].note {
            candidats[i] = c;
        }
    }
    candidats.sort_by(|a, b| a.note.partial_cmp(&b.note).unwrap_or(std::cmp::Ordering::Equal));
    Ok(candidats)
}

#[allow(clippy::too_many_arguments)]
fn boucle_par(g: &Graphe, p: &Preferences, d: NoeudId, p1: NoeudId, p2: NoeudId, cap: f64, facteur: f64, pb: &ParametresBoucle) -> Option<Candidat> {
    let mut penalises: HashSet<ArcId> = HashSet::new();
    let mut arcs = Vec::new();
    let mut explores = 0;
    for (de, a) in [(d, p1), (p1, p2), (p2, d)] {
        let Chemin { arcs: tr, noeuds_explores, .. } = astar::chemin(g, p, de, a, &penalises, pb.epsilon)?;
        explores += noeuds_explores;
        for &id in &tr {
            penalises.insert(id);
            if let Some(x) = g.arc(id) {
                penalises.insert(x.inverse.to_native());
            }
        }
        arcs.extend(tr);
    }
    let mut vus: HashSet<(i64, u16)> = HashSet::new();
    let (mut longueur, mut temps, mut double, mut sin, mut agg) = (0u64, 0u64, 0u64, 0.0f64, 0.0f64);
    for &id in &arcs {
        let a = g.arc(id)?;
        let l = a.longueur_m.to_native() as u64;
        longueur += l;
        temps += a.temps_ds.to_native() as u64;
        sin += l as f64 * a.scores[score::SINUOSITE] as f64 / 255.0;
        agg += l as f64 * a.scores[score::AGGLO] as f64 / 255.0;
        if !vus.insert((a.way_id.to_native(), a.segment.to_native())) {
            double += l;
        }
    }
    if longueur == 0 {
        return None;
    }
    let lf = longueur as f64;
    let recouvrement = double as f64 / lf;
    let ecart = ((lf - pb.distance_m) / pb.distance_m).abs();
    let (sinuosite, agglo) = (sin / lf, agg / lf);
    // Note : écart de distance au-delà de la tolérance (fort), recouvrement (fort),
    // agglomération (moyen), sinuosité (bonus).
    let note = (ecart - pb.tolerance).max(0.0) * 4.0 + ecart + recouvrement * 3.0 + agglo * 1.0 - sinuosite * 1.5;
    Some(Candidat { cap_deg: cap, facteur, arcs, longueur_m: longueur, temps_ds: temps, recouvrement, sinuosite, agglo, note, noeuds_explores: explores })
}
