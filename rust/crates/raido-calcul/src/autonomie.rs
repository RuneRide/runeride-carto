// SPDX-License-Identifier: EUPL-1.2
//! Autonomie carburant (spec §4.6, U6) : où faire le plein sur une balade trop longue pour le réservoir.
//!
//! Le moteur ne connaît pas les stations (elles vivent dans la base du PHP, `osm_stations`) : la requête les apporte,
//! `stations`, et ce module choisit lesquelles insérer. Le calcul est un post-traitement du parcours déjà trouvé
//! (`requete::traiter_complet`) : les stations deviennent des étapes, parmi les points de passage de la boucle d'origine.

use serde::{Deserialize, Serialize};

use crate::geo;

/// Part de l'autonomie jamais entamée : on fait le plein avant d'arriver à 90 % de ce que le réservoir permet.
pub const MARGE: f64 = 0.9;
/// Écart maximal (m) entre une station et le tracé pour qu'elle compte « sur la route ».
pub const ECART_MAX_M: f64 = 3_000.0;
/// Plein de stations au plus insérées dans un parcours.
pub const PLEINS_MAX: usize = 4;
pub const STATIONS_MAX: usize = 500;

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
pub struct Autonomie {
    /// Distance maximale réservoir plein (km).
    pub km: f64,
    /// Niveau au départ, de 0 à 1 (défaut : plein).
    #[serde(default = "plein")]
    pub reservoir: f64,
}

fn plein() -> f64 {
    1.0
}

impl Autonomie {
    pub fn verifier(&self) -> Result<(), String> {
        if !self.km.is_finite() || !(30.0..=1000.0).contains(&self.km) {
            return Err("autonomie.km doit être entre 30 et 1000".into());
        }
        if !self.reservoir.is_finite() || !(0.05..=1.0).contains(&self.reservoir) {
            return Err("autonomie.reservoir doit être entre 0,05 et 1".into());
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Station {
    pub lat: f64,
    pub lon: f64,
    #[serde(default)]
    pub nom: Option<String>,
    /// Prix du litre (€) s'il est connu : à portée égale, la moins chère passe en premier.
    #[serde(default)]
    pub prix: Option<f64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct StationRendue {
    pub lat: f64,
    pub lon: f64,
    pub nom: Option<String>,
    pub prix: Option<f64>,
    /// Distance parcourue depuis le départ à la station (m).
    pub distance_m: u64,
}

/// Distance cumulée (m) à chaque point de la trace.
pub fn cumul(trace: &[(f64, f64)]) -> Vec<f64> {
    let mut c = Vec::with_capacity(trace.len());
    let mut d = 0.0;
    for (i, &p) in trace.iter().enumerate() {
        if i > 0 {
            d += geo::distance_m(trace[i - 1], p);
        }
        c.push(d);
    }
    c
}

/// Position (m depuis le départ) du point de la trace le plus proche de `p`, et sa distance à la trace.
pub fn projeter(trace: &[(f64, f64)], cumul: &[f64], p: (f64, f64)) -> Option<(f64, f64)> {
    trace
        .iter()
        .enumerate()
        .map(|(i, &q)| (geo::distance_m(p, q), cumul[i]))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(d, pos)| (pos, d))
}

/// Stations à insérer, par ordre de passage : `(indice de la station, position en m)`. Pour chaque tronçon sans plein, on prend
/// parmi les stations à moins de `ECART_MAX_M` du tracé et atteignables (`portée × MARGE`) celles de la seconde moitié de la portée,
/// la moins chère d'abord, sinon la plus lointaine atteignable. `Err` : aucune station atteignable, avec le km où l'on est à sec.
pub fn plan(trace: &[(f64, f64)], cumul: &[f64], a: &Autonomie, stations: &[Station]) -> Result<Vec<(usize, f64)>, String> {
    let total = cumul.last().copied().unwrap_or(0.0);
    let proches: Vec<(usize, f64)> = stations
        .iter()
        .enumerate()
        .filter_map(|(i, s)| projeter(trace, cumul, (s.lat, s.lon)).filter(|&(_, d)| d <= ECART_MAX_M).map(|(pos, _)| (i, pos)))
        .collect();
    let mut sortie: Vec<(usize, f64)> = Vec::new();
    let (mut depuis, mut portee) = (0.0, a.km * 1000.0 * a.reservoir);
    while total - depuis > portee * MARGE {
        if sortie.len() >= PLEINS_MAX {
            return Err(format!("plus de {PLEINS_MAX} pleins nécessaires : parcours trop long pour cette autonomie"));
        }
        let limite = depuis + portee * MARGE;
        let atteignables: Vec<(usize, f64)> = proches.iter().copied().filter(|&(_, pos)| pos > depuis + 1_000.0 && pos <= limite).collect();
        let fenetre: Vec<(usize, f64)> = atteignables.iter().copied().filter(|&(_, pos)| pos >= depuis + (limite - depuis) * 0.5).collect();
        let prix = |i: usize| stations[i].prix.unwrap_or(f64::INFINITY);
        let choix = fenetre
            .iter()
            .copied()
            .min_by(|x, y| prix(x.0).total_cmp(&prix(y.0)).then(y.1.total_cmp(&x.1)))
            .or_else(|| atteignables.iter().copied().max_by(|x, y| x.1.total_cmp(&y.1)));
        let Some((i, pos)) = choix else {
            return Err(format!("autonomie insuffisante : aucune station à moins de {:.0} km du tracé avant le km {:.0}", ECART_MAX_M / 1000.0, limite / 1000.0));
        };
        sortie.push((i, pos));
        depuis = pos;
        portee = a.km * 1000.0;
    }
    Ok(sortie)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ligne droite plein nord de 120 km, un point tous les 500 m.
    fn ligne() -> Vec<(f64, f64)> {
        (0..=240).map(|i| geo::destination(45.0, 5.0, 0.0, i as f64 * 500.0)).collect()
    }
    fn station(km: f64, ecart_m: f64, prix: Option<f64>) -> Station {
        let (la, lo) = geo::destination(45.0, 5.0, 0.0, km * 1000.0);
        let (la, lo) = geo::destination(la, lo, 90.0, ecart_m);
        Station { lat: la, lon: lo, nom: None, prix }
    }

    #[test]
    fn validation() {
        assert!(Autonomie { km: 180.0, reservoir: 0.5 }.verifier().is_ok());
        for (km, r) in [(10.0, 1.0), (2000.0, 1.0), (180.0, 0.0), (180.0, 1.5)] {
            assert!(Autonomie { km, reservoir: r }.verifier().is_err(), "{km} {r}");
        }
        let a: Autonomie = serde_json::from_str(r#"{"km":150}"#).unwrap();
        assert_eq!(a.reservoir, 1.0, "défaut : plein");
    }

    #[test]
    fn parcours_dans_la_portee_n_insere_rien() {
        let t = ligne();
        let c = cumul(&t);
        assert!((c.last().unwrap() - 120_000.0).abs() < 100.0);
        assert!(plan(&t, &c, &Autonomie { km: 150.0, reservoir: 1.0 }, &[station(60.0, 100.0, None)]).unwrap().is_empty(), "120 km < 90 % de 150 km");
    }

    #[test]
    fn plein_choisi_dans_la_seconde_moitie_de_la_portee_et_le_moins_cher() {
        let t = ligne();
        let c = cumul(&t);
        // portée 100 km → limite 90 km ; seconde moitié : 45 à 90 km
        let s = [station(20.0, 100.0, Some(1.5)), station(50.0, 100.0, Some(1.9)), station(80.0, 100.0, Some(1.7)), station(85.0, 100.0, None)];
        let p = plan(&t, &c, &Autonomie { km: 100.0, reservoir: 1.0 }, &s).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, 2, "la moins chère de la fenêtre atteignable (80 km à 1,70 €)");
        assert!((p[0].1 - 80_000.0).abs() < 600.0);
    }

    #[test]
    fn sans_prix_la_plus_lointaine_de_la_fenetre() {
        let t = ligne();
        let c = cumul(&t);
        let s = [station(55.0, 100.0, None), station(85.0, 100.0, None)];
        assert_eq!(plan(&t, &c, &Autonomie { km: 100.0, reservoir: 1.0 }, &s).unwrap()[0].0, 1);
    }

    #[test]
    fn reservoir_a_moitie_et_deux_pleins() {
        let t = ligne();
        let c = cumul(&t);
        // départ à moitié plein : portée 50 km (limite 45) ; puis 100 km pleins (limite 90 depuis le plein)
        let s = [station(40.0, 100.0, None), station(120.0, 100.0, None)];
        let p = plan(&t, &c, &Autonomie { km: 100.0, reservoir: 0.5 }, &s).unwrap();
        assert_eq!(p.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0], "un plein à 40 km suffit : 80 km restants < 90");
        let long: Vec<_> = (0..=480).map(|i| geo::destination(45.0, 5.0, 0.0, i as f64 * 500.0)).collect(); // 240 km
        let cl = cumul(&long);
        let s2 = [station(40.0, 100.0, None), station(120.0, 100.0, None), station(200.0, 100.0, None)];
        let p2 = plan(&long, &cl, &Autonomie { km: 100.0, reservoir: 0.5 }, &s2).unwrap();
        assert_eq!(p2.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    #[test]
    fn station_trop_loin_du_trace_ou_hors_de_portee() {
        let t = ligne();
        let c = cumul(&t);
        let a = Autonomie { km: 100.0, reservoir: 1.0 };
        let e = plan(&t, &c, &a, &[station(60.0, 5_000.0, None)]).unwrap_err();
        assert!(e.contains("aucune station"), "{e}");
        assert!(plan(&t, &c, &a, &[station(95.0, 100.0, None)]).is_err(), "au-delà de 90 % de la portée");
        assert!(plan(&t, &c, &a, &[]).is_err());
    }
}
