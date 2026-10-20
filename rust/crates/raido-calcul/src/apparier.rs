// SPDX-License-Identifier: EUPL-1.2
//! Appariement d'un tracé GPS au réseau (map matching), spec `runeride/docs/itineraire.md` §4.4 et §7.2.
//!
//! Pour chaque point échantillonné du tracé, on retient les tronçons du graphe passant à moins de `RAYON_M`, puis un
//! Viterbi choisit la suite de tronçons la plus plausible : le coût d'un candidat croît avec le carré de sa distance au
//! point ; passer d'un tronçon à lui-même ne coûte rien, à un tronçon voisin (nœud commun) très peu, à un tronçon
//! éloigné beaucoup. Le résultat est la liste des identifiants stables `(way_id, segment)` que le moteur lit dans
//! `troncons_connus`, et la part des points qui ont trouvé un tronçon (couverture).
//!
//! Limites : un tronçon plus long qu'environ 1,5 km entre deux intersections n'est vu que si un de ses nœuds est parmi
//! les 40 plus proches dans ce rayon ; les sens uniques ne comptent pas (le tronçon est le même dans les deux sens) ;
//! aucune vitesse ni horodatage n'est utilisé.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};
use raido_format::{ArcId, NoeudId};
use serde::Serialize;

use crate::charge::Graphe;
use crate::geo;

/// Distance maximale d'un point à un tronçon pour en être candidat (bruit GPS de quelques dizaines de mètres).
pub const RAYON_M: f64 = 50.0;
/// Deux points retenus sont au moins à cette distance l'un de l'autre.
pub const PAS_ECHANTILLON_M: f64 = 15.0;
/// Nombre maximal de points échantillonnés (un trajet de 600 km en compte environ 40 000).
pub const MAX_POINTS: usize = 60_000;
const RAYON_NOEUDS_M: f64 = 1_500.0;
const NB_NOEUDS: usize = 40;
const SIGMA_M: f64 = 15.0;

pub type Cle = (i64, u16);

#[derive(Serialize, Clone, Debug)]
pub struct Appariement {
    /// Tronçons parcourus, triés, sans doublon.
    pub troncons: Vec<Cle>,
    /// Part (0 à 1) des points échantillonnés auxquels un tronçon a été trouvé.
    pub couverture: f64,
    pub points: usize,
    pub points_apparies: usize,
}

#[derive(Clone)]
struct Cand {
    cle: Cle,
    src: NoeudId,
    dst: NoeudId,
    dist: f64,
}

/// Arc d'un nœud avec sa polyligne complète (extrémités comprises), mise en cache le temps d'un appariement.
struct ArcGeom {
    cle: Cle,
    dst: NoeudId,
    pts: Vec<(f64, f64)>,
}

/// Distance (m) d'un point à une polyligne, par projection locale (équirectangulaire autour du point).
fn distance_polyligne(p: (f64, f64), pts: &[(f64, f64)]) -> f64 {
    let kx = 111_320.0 * p.0.to_radians().cos();
    let ky = 110_540.0;
    let xy = |q: &(f64, f64)| ((q.1 - p.1) * kx, (q.0 - p.0) * ky);
    let mut meilleur = f64::INFINITY;
    for w in pts.windows(2) {
        let (ax, ay) = xy(&w[0]);
        let (bx, by) = xy(&w[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let l2 = dx * dx + dy * dy;
        let t = if l2 == 0.0 { 0.0 } else { ((-ax) * dx + (-ay) * dy) / l2 };
        let t = t.clamp(0.0, 1.0);
        let d = ((ax + t * dx).powi(2) + (ay + t * dy).powi(2)).sqrt();
        meilleur = meilleur.min(d);
    }
    if pts.len() == 1 {
        let (x, y) = xy(&pts[0]);
        meilleur = (x * x + y * y).sqrt();
    }
    meilleur
}

fn echantillonner(trace: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut sortie: Vec<(f64, f64)> = Vec::new();
    for &p in trace {
        match sortie.last() {
            Some(&q) if haversine(p, q) < PAS_ECHANTILLON_M => {}
            _ => sortie.push(p),
        }
    }
    sortie
}

fn haversine(a: (f64, f64), b: (f64, f64)) -> f64 {
    raido_format::haversine_m(raido_format::Point::depuis_degres(a.0, a.1), raido_format::Point::depuis_degres(b.0, b.1))
}

/// Apparie `trace` (liste de `(lat, lon)` en degrés) au graphe. Les tuiles nécessaires sont chargées ici.
pub fn apparier(g: &mut Graphe, trace: &[(f64, f64)]) -> Result<Appariement> {
    if trace.iter().any(|p| !p.0.is_finite() || !p.1.is_finite() || p.0.abs() > 90.0 || p.1.abs() > 180.0) {
        bail!("trace : coordonnée invalide");
    }
    let pts = echantillonner(trace);
    if pts.len() < 2 {
        bail!("trace : au moins deux points distants de {PAS_ECHANTILLON_M:.0} m sont nécessaires");
    }
    if pts.len() > MAX_POINTS {
        bail!("trace trop longue ({} points échantillonnés, {MAX_POINTS} au plus)", pts.len());
    }
    let (mut s, mut o, mut n, mut e) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in &pts {
        s = s.min(p.0);
        n = n.max(p.0);
        o = o.min(p.1);
        e = e.max(p.1);
    }
    if haversine((s, o), (n, e)) > 600_000.0 {
        bail!("trace de plus de 600 km d'emprise : non prise en charge");
    }
    let dlat = (RAYON_NOEUDS_M + 500.0) / 111_000.0;
    let dlon = dlat / ((s + n) / 2.0).to_radians().cos().max(0.2);
    g.precharger([s - dlat, o - dlon, n + dlat, e + dlon])?;

    let mut cache: HashMap<NoeudId, Vec<ArcGeom>> = HashMap::new();
    let mut couches: Vec<Vec<Cand>> = Vec::new();
    for &p in &pts {
        let mut par_cle: HashMap<Cle, Cand> = HashMap::new();
        for (nd, _) in g.noeuds_proches(p.0, p.1, RAYON_NOEUDS_M, NB_NOEUDS) {
            let arcs = cache.entry(nd).or_insert_with(|| arcs_du_noeud(g, nd));
            for a in arcs.iter() {
                let d = distance_polyligne(p, &a.pts);
                if d > RAYON_M {
                    continue;
                }
                let c = Cand { cle: a.cle, src: nd, dst: a.dst, dist: d };
                par_cle.entry(a.cle).and_modify(|x| if d < x.dist { *x = c.clone() }).or_insert(c);
            }
        }
        if !par_cle.is_empty() {
            let mut v: Vec<Cand> = par_cle.into_values().collect();
            v.sort_by(|a, b| a.dist.partial_cmp(&b.dist).unwrap().then(a.cle.cmp(&b.cle)));
            v.truncate(12);
            couches.push(v);
        }
    }
    let total = pts.len();
    let apparies = couches.len();
    if apparies == 0 {
        return Ok(Appariement { troncons: Vec::new(), couverture: 0.0, points: total, points_apparies: 0 });
    }

    // Viterbi : coût = Σ émission (distance²) + transitions (tronçon identique 0, voisin 0,5, éloigné 3).
    let emission = |c: &Cand| 0.5 * (c.dist / SIGMA_M).powi(2);
    let transition = |a: &Cand, b: &Cand| {
        if a.cle == b.cle {
            0.0
        } else if a.src == b.src || a.src == b.dst || a.dst == b.src || a.dst == b.dst {
            0.5
        } else {
            3.0
        }
    };
    let mut cout: Vec<Vec<f64>> = vec![couches[0].iter().map(emission).collect()];
    let mut retour: Vec<Vec<usize>> = vec![vec![0; couches[0].len()]];
    for i in 1..couches.len() {
        let mut ci = Vec::with_capacity(couches[i].len());
        let mut ri = Vec::with_capacity(couches[i].len());
        for b in &couches[i] {
            let (mut mk, mut mc) = (0, f64::INFINITY);
            for (k, a) in couches[i - 1].iter().enumerate() {
                let c = cout[i - 1][k] + transition(a, b);
                if c < mc {
                    mc = c;
                    mk = k;
                }
            }
            ci.push(mc + emission(b));
            ri.push(mk);
        }
        cout.push(ci);
        retour.push(ri);
    }
    let mut j = cout.last().unwrap().iter().enumerate().min_by(|a, b| a.1.partial_cmp(b.1).unwrap()).map(|(j, _)| j).unwrap();
    let mut chemin: Vec<Cle> = Vec::with_capacity(couches.len());
    for i in (0..couches.len()).rev() {
        chemin.push(couches[i][j].cle);
        j = retour[i][j];
    }
    chemin.reverse();

    // Un tronçon effleuré par un seul point (carrefour traversé en biais) n'est pas retenu, sauf trace très courte.
    let mut appui: HashMap<Cle, usize> = HashMap::new();
    for c in &chemin {
        *appui.entry(*c).or_default() += 1;
    }
    let garder_tout = chemin.len() < 4;
    let mut vus = HashSet::new();
    let mut troncons: Vec<Cle> = chemin.into_iter().filter(|c| (garder_tout || appui[c] >= 2) && vus.insert(*c)).collect();
    troncons.sort_unstable();
    Ok(Appariement { troncons, couverture: apparies as f64 / total as f64, points: total, points_apparies: apparies })
}

fn arcs_du_noeud(g: &Graphe, nd: NoeudId) -> Vec<ArcGeom> {
    let Some(depart) = g.position(nd) else { return Vec::new() };
    let ids: Vec<(ArcId, (i64, u16), NoeudId)> = g.sortants(nd).map(|(id, a)| (id, (a.way_id.to_native(), a.segment.to_native()), a.cible.to_native())).collect();
    ids.into_iter()
        .filter_map(|(id, cle, dst)| {
            let fin = g.position(dst)?;
            let mut pts = vec![geo::deg(depart)];
            pts.extend(g.geometrie(id).into_iter().map(geo::deg));
            pts.push(geo::deg(fin));
            Some(ArcGeom { cle, dst, pts })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_a_une_polyligne() {
        let ligne = [(45.0, 5.0), (45.0, 5.01)]; // ≈ 787 m vers l'est
        let d = distance_polyligne((45.0 + 20.0 / 110_540.0, 5.005), &ligne);
        assert!((d - 20.0).abs() < 0.5, "à 20 m au nord du milieu : {d}");
        let d = distance_polyligne((45.0, 5.0 - 30.0 / (111_320.0 * 45f64.to_radians().cos())), &ligne);
        assert!((d - 30.0).abs() < 0.5, "au-delà de l'extrémité : distance au point de bout : {d}");
        assert_eq!(distance_polyligne((45.0, 5.0), &[(45.0, 5.0)]), 0.0);
    }

    #[test]
    fn echantillonnage_espace_les_points() {
        let trace: Vec<(f64, f64)> = (0..100).map(|i| (45.0 + i as f64 * 1e-5, 5.0)).collect(); // 1,1 m entre points
        let e = echantillonner(&trace);
        assert!(e.windows(2).all(|w| haversine(w[0], w[1]) >= PAS_ECHANTILLON_M));
        assert!(e.len() < 100 && e.len() >= 5, "{}", e.len());
    }
}
