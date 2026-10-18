// SPDX-License-Identifier: EUPL-1.2
//! Fonction de coût : `temps × (1 + Σ poids × (1 − score)) + pénalités` (spec §4.3).
//! Le coût est toujours ≥ temps, ce qui garde l'heuristique d'A* admissible.

use std::collections::HashSet;

use raido_format::{drapeau, score, ArcId, ArchivedArc, Classe};

/// Préférences et contraintes d'une requête.
#[derive(Clone, Debug, Default)]
pub struct Preferences {
    /// Crans 0..4 (spec §6.2).
    pub sinuosite: u8,
    pub scenique: u8,
    pub motard: u8,
    pub decouverte: u8,
    /// Poids de la pénalité d'agglomération (cran 0..4).
    pub agglo: u8,
    pub eviter_autoroutes: bool,
    pub eviter_peages: bool,
    pub eviter_non_revetu: bool,
    pub eviter_tunnels: bool,
    /// Tronçons déjà parcourus (`way_id`, `segment`) pour la découverte.
    pub troncons_connus: HashSet<(i64, u16)>,
}

impl Preferences {
    /// Réglage « balade » typique : sinueux et scénique au cran 3, agglomérations évitées.
    pub fn balade() -> Preferences {
        Preferences { sinuosite: 3, scenique: 3, motard: 2, agglo: 3, eviter_autoroutes: true, eviter_peages: true, eviter_non_revetu: true, ..Default::default() }
    }
}

/// Poids d'un cran : 0, 0,5, 1, 1,5, 2.
#[inline]
fn poids(cran: u8) -> f64 {
    cran.min(4) as f64 * 0.5
}

/// Vitesse maximale du graphe (m/s), pour l'heuristique : 110 km/h.
pub const V_MAX_MS: f64 = 110.0 / 3.6;

/// Coût d'un arc en décisecondes pondérées ; `None` si l'arc est exclu.
/// `penalises` : arcs à éviter (recouvrement d'une boucle), coût ×4.
#[inline]
pub fn cout(p: &Preferences, id: ArcId, a: &ArchivedArc, penalises: &HashSet<ArcId>) -> Option<f64> {
    let classe = Classe::depuis_u8(a.classe);
    let d = a.drapeaux;
    if p.eviter_autoroutes && classe == Classe::Autoroute {
        return None;
    }
    if p.eviter_peages && d & drapeau::PEAGE != 0 {
        return None;
    }
    if p.eviter_non_revetu && d & drapeau::NON_REVETU != 0 {
        return None;
    }
    if p.eviter_tunnels && d & drapeau::TUNNEL != 0 {
        return None;
    }
    let s = |i: usize| a.scores[i] as f64 / 255.0;
    let mut facteur = 1.0
        + poids(p.sinuosite) * (1.0 - s(score::SINUOSITE))
        + poids(p.scenique) * (1.0 - s(score::SCENIQUE))
        + poids(p.motard) * (1.0 - s(score::MOTARD))
        + poids(p.agglo) * s(score::AGGLO);
    // Les bretelles et voies résidentielles ne sont jamais « sinueuses » au sens balade.
    if matches!(classe, Classe::Bretelle | Classe::Residentielle | Classe::Zone) {
        facteur += poids(p.sinuosite) + poids(p.scenique);
    }
    if p.decouverte > 0 && p.troncons_connus.contains(&(a.way_id.to_native(), a.segment.to_native())) {
        facteur += poids(p.decouverte) * 1.5;
    }
    if penalises.contains(&id) {
        facteur *= 4.0;
    }
    Some(a.temps_ds.to_native() as f64 * facteur)
}
