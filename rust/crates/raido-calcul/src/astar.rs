// SPDX-License-Identifier: EUPL-1.2
//! A* par arcs : l'état de recherche est l'arc par lequel on arrive à un nœud,
//! ce qui permet d'appliquer les restrictions de tourner et d'interdire les
//! demi-tours.  Heuristique : distance à vol d'oiseau / vitesse maximale, en
//! décisecondes, admissible car le coût d'un arc est toujours ≥ son temps.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use raido_format::{haversine_m, ArcId, NoeudId, Point, ID_NUL};

use crate::charge::Graphe;
use crate::cout::{cout, Preferences, V_MAX_MS};

#[derive(Clone, Debug)]
pub struct Chemin {
    pub arcs: Vec<ArcId>,
    pub cout: f64,
    pub longueur_m: u64,
    pub temps_ds: u64,
    pub noeuds_explores: usize,
}

#[derive(Copy, Clone, PartialEq)]
struct Entree {
    f: f64,
    g: f64,
    arc: ArcId,
}

impl Eq for Entree {}
impl Ord for Entree {
    fn cmp(&self, o: &Self) -> Ordering {
        // Tas max → on inverse pour obtenir un tas min sur f.
        o.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for Entree {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Un virage de `de` vers `vers` au nœud `n` est-il autorisé ?
#[inline]
fn virage_autorise(g: &Graphe, n: NoeudId, de: ArcId, vers: ArcId, inverse_de: ArcId) -> bool {
    // Demi-tour interdit sauf en impasse.
    if vers == inverse_de && g.degre_sortant(n) > 1 {
        return false;
    }
    let mut seulement: Option<ArcId> = None;
    for r in g.restrictions(n) {
        if r.de.to_native() != de {
            continue;
        }
        if r.seulement {
            seulement = Some(r.vers.to_native());
        } else if r.vers.to_native() == vers {
            return false;
        }
    }
    seulement.is_none_or(|s| s == vers)
}

/// Plus court chemin (au sens du coût) du nœud `depart` au nœud `arrivee`.
/// `penalises` : arcs dont le coût est quadruplé (recouvrement d'une boucle).
/// `epsilon` ≥ 1 pondère l'heuristique : 1 = optimal, au-delà = A* pondéré
/// (chemin au plus `epsilon` fois plus coûteux que l'optimum, recherche bien plus courte).
pub fn chemin(g: &Graphe, p: &Preferences, depart: NoeudId, arrivee: NoeudId, penalises: &HashSet<ArcId>, epsilon: f64) -> Option<Chemin> {
    let cible = g.position(arrivee)?;
    let h = |pt: Point| haversine_m(pt, cible) / V_MAX_MS * 10.0 * epsilon;
    let mut file = BinaryHeap::new();
    let mut meilleur_g: HashMap<ArcId, f64> = HashMap::new();
    let mut pred: HashMap<ArcId, ArcId> = HashMap::new();
    let mut explores = 0usize;

    if depart == arrivee {
        return Some(Chemin { arcs: Vec::new(), cout: 0.0, longueur_m: 0, temps_ds: 0, noeuds_explores: 0 });
    }
    for (id, a) in g.sortants(depart) {
        if let Some(c) = cout(p, id, a, penalises) {
            // Un arc dont la cible est dans une tuile non chargée est infranchissable.
            let Some(pos) = g.position(a.cible.to_native()) else { continue };
            meilleur_g.insert(id, c);
            pred.insert(id, ID_NUL);
            file.push(Entree { f: c + h(pos), g: c, arc: id });
        }
    }
    while let Some(Entree { g: gc, arc, .. }) = file.pop() {
        if meilleur_g.get(&arc).is_none_or(|&b| gc > b) {
            continue;
        }
        explores += 1;
        let a = g.arc(arc)?;
        let n = a.cible.to_native();
        if n == arrivee {
            return Some(reconstruire(g, &pred, arc, gc, explores));
        }
        let inverse = a.inverse.to_native();
        for (id, b) in g.sortants(n) {
            if !virage_autorise(g, n, arc, id, inverse) {
                continue;
            }
            let Some(c) = cout(p, id, b, penalises) else { continue };
            let ng = gc + c;
            if meilleur_g.get(&id).is_none_or(|&b| ng < b) {
                let Some(pos) = g.position(b.cible.to_native()) else { continue };
                meilleur_g.insert(id, ng);
                pred.insert(id, arc);
                file.push(Entree { f: ng + h(pos), g: ng, arc: id });
            }
        }
    }
    None
}

fn reconstruire(g: &Graphe, pred: &HashMap<ArcId, ArcId>, dernier: ArcId, cout: f64, explores: usize) -> Chemin {
    let mut arcs = vec![dernier];
    let mut a = dernier;
    while let Some(&p) = pred.get(&a) {
        if p == ID_NUL {
            break;
        }
        arcs.push(p);
        a = p;
    }
    arcs.reverse();
    let (mut longueur, mut temps) = (0u64, 0u64);
    for &id in &arcs {
        if let Some(x) = g.arc(id) {
            longueur += x.longueur_m.to_native() as u64;
            temps += x.temps_ds.to_native() as u64;
        }
    }
    Chemin { arcs, cout, longueur_m: longueur, temps_ds: temps, noeuds_explores: explores }
}

/// Trace complète d'un chemin (nœuds et points intermédiaires), en degrés.
pub fn trace(g: &Graphe, arcs: &[ArcId], depart: NoeudId) -> Vec<(f64, f64)> {
    let mut pts = Vec::new();
    if let Some(p) = g.position(depart) {
        pts.push((p.lat_deg(), p.lon_deg()));
    }
    for &id in arcs {
        for q in g.geometrie(id) {
            pts.push((q.lat_deg(), q.lon_deg()));
        }
        if let Some(a) = g.arc(id) {
            if let Some(p) = g.position(a.cible.to_native()) {
                pts.push((p.lat_deg(), p.lon_deg()));
            }
        }
    }
    pts
}
