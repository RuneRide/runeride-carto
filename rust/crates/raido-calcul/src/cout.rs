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
    /// Vitesses moyennes réglées par l'utilisateur (km/h) pour certaines classes ; les autres gardent celles du graphe.
    pub vitesses: Vec<(Classe, f64)>,
}

impl Preferences {
    /// Facteur appliqué au temps de base d'un arc de cette classe : vitesse du graphe ÷ vitesse réglée. Les vitesses
    /// réglées sont bornées à `V_MAX_MS` (110 km/h) à la lecture de la requête, pour que le temps à vol d'oiseau
    /// reste une borne inférieure admissible pour A*. Approximation : le plafond `maxspeed` déjà appliqué dans le
    /// temps de base du graphe n'est pas relevé.
    #[inline]
    pub fn echelle_temps(&self, classe: Classe) -> f64 {
        match self.vitesses.iter().find(|(c, _)| *c == classe) {
            Some((c, v)) => c.vitesse_defaut_kmh() / v,
            None => 1.0,
        }
    }

    /// Temps de base (ds) d'un arc, avec les vitesses réglées.
    #[inline]
    pub fn temps_arc_ds(&self, a: &ArchivedArc) -> f64 {
        a.temps_ds.to_native() as f64 * self.echelle_temps(Classe::depuis_u8(a.classe))
    }

    /// Somme des temps (ds) d'une suite d'arcs, avec les vitesses réglées.
    pub fn temps_chemin_ds<'a>(&self, arcs: impl IntoIterator<Item = &'a ArchivedArc>) -> u64 {
        arcs.into_iter().map(|a| self.temps_arc_ds(a)).sum::<f64>().round() as u64
    }

    /// Réglage « balade » typique : sinueux et scénique au cran 3, agglomérations évitées.
    pub fn balade() -> Preferences {
        Preferences { sinuosite: 3, scenique: 3, motard: 2, agglo: 3, eviter_autoroutes: true, eviter_peages: true, eviter_non_revetu: true, ..Default::default() }
    }
}

/// Poids d'un cran : 0, 0,5, 1, 1,5, 2.
#[inline]
pub fn poids(cran: u8) -> f64 {
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
    Some(a.temps_ds.to_native() as f64 * p.echelle_temps(classe) * facteur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echelle_temps_suit_la_vitesse_reglee() {
        let mut p = Preferences::default();
        assert_eq!(p.echelle_temps(Classe::Secondaire), 1.0, "sans réglage : temps du graphe");
        p.vitesses = vec![(Classe::Secondaire, 37.5)];
        assert_eq!(p.echelle_temps(Classe::Secondaire), 2.0, "75 → 37,5 km/h : deux fois plus long");
        assert_eq!(p.echelle_temps(Classe::Tertiaire), 1.0, "les autres classes ne bougent pas");
        p.vitesses = vec![(Classe::Secondaire, V_MAX_MS * 3.6)];
        assert!(p.echelle_temps(Classe::Secondaire) >= 75.0 / 110.0 - 1e-9, "à 110 km/h au plus : l'heuristique reste admissible");
    }
}
