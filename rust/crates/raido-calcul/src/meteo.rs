// SPDX-License-Identifier: EUPL-1.2
//! Contrainte météo (spec §4.5, jalon J5) : seuils, échantillonnage de la trace, zones à éviter.
//!
//! La météo elle-même vient d'un `FournisseurMeteo`. Celui de RuneRide Météo (`POST /v1/route`, jalon J6
//! de ce projet-là) n'existe pas encore ; `FournisseurSimule` (zones circulaires fixes) permet de tester la
//! boucle de recalcul et de la mesurer sans réseau.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::geo;

/// Rayon (m) de la zone d'évitement posée autour d'un point en infraction.
pub const RAYON_ZONE_M: f64 = 3_000.0;
/// Nombre maximal de calculs (le premier, puis deux recalculs avec zones) : spec §4.5.
pub const CALCULS_MAX: usize = 3;
/// Nombre maximal de points envoyés à la météo (spec météo §7 : 500 points en moins de 2 s).
pub const POINTS_MAX: usize = 500;

/// Seuils de la requête (`meteo`) ; une valeur absente prend le défaut tranché en O2.
#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
pub struct Seuils {
    #[serde(default = "pluie_defaut")]
    pub pluie_mmh: f64,
    #[serde(default = "rafales_defaut")]
    pub rafales_kmh: f64,
    #[serde(default = "froid_defaut")]
    pub froid_c: f64,
}

fn pluie_defaut() -> f64 {
    1.0
}
fn rafales_defaut() -> f64 {
    60.0
}
fn froid_defaut() -> f64 {
    5.0
}

impl Default for Seuils {
    fn default() -> Seuils {
        Seuils { pluie_mmh: pluie_defaut(), rafales_kmh: rafales_defaut(), froid_c: froid_defaut() }
    }
}

impl Seuils {
    pub fn verifier(&self) -> Result<()> {
        if !self.pluie_mmh.is_finite() || !(0.0..=50.0).contains(&self.pluie_mmh) {
            bail!("meteo.pluie_mmh doit être entre 0 et 50");
        }
        if !self.rafales_kmh.is_finite() || !(10.0..=200.0).contains(&self.rafales_kmh) {
            bail!("meteo.rafales_kmh doit être entre 10 et 200");
        }
        if !self.froid_c.is_finite() || !(-30.0..=30.0).contains(&self.froid_c) {
            bail!("meteo.froid_c doit être entre -30 et 30");
        }
        Ok(())
    }
}

/// Point de la trace envoyé à la météo.
#[derive(Clone, Copy, Debug)]
pub struct Echantillon {
    pub lat: f64,
    pub lon: f64,
    /// Distance depuis le départ (m), pour estimer l'heure de passage.
    pub distance_m: f64,
}

/// Météo prévue en un point, à l'heure de passage.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
pub struct PointMeteo {
    pub lat: f64,
    pub lon: f64,
    pub distance_m: f64,
    pub pluie_mmh: f64,
    pub rafales_kmh: f64,
    /// Température ressentie.
    pub froid_c: f64,
}

pub trait FournisseurMeteo {
    /// Météo à l'heure de passage en chaque échantillon. `depart_a` : ISO 8601 ; `vitesse_kmh` : vitesse moyenne
    /// pour estimer l'heure de passage.
    fn par_point(&self, points: &[Echantillon], depart_a: Option<&str>, vitesse_kmh: f64) -> Result<Vec<PointMeteo>>;
}

impl Seuils {
    /// Ce point dépasse-t-il un seuil ?
    pub fn depasse(&self, p: &PointMeteo) -> bool {
        p.pluie_mmh > self.pluie_mmh || p.rafales_kmh > self.rafales_kmh || p.froid_c < self.froid_c
    }
}

/// Échantillons régulièrement espacés d'une trace (au plus `POINTS_MAX`, un par kilomètre au moins), extrémités comprises.
pub fn echantillonner(trace: &[(f64, f64)]) -> Vec<Echantillon> {
    if trace.is_empty() {
        return Vec::new();
    }
    let mut cumul = Vec::with_capacity(trace.len());
    let mut d = 0.0;
    for (i, &p) in trace.iter().enumerate() {
        if i > 0 {
            d += geo::distance_m(trace[i - 1], p);
        }
        cumul.push(d);
    }
    let pas = (d / POINTS_MAX as f64).max(1_000.0);
    let mut sortie = vec![Echantillon { lat: trace[0].0, lon: trace[0].1, distance_m: 0.0 }];
    let mut prochain = pas;
    for (i, &p) in trace.iter().enumerate().skip(1) {
        if cumul[i] >= prochain {
            sortie.push(Echantillon { lat: p.0, lon: p.1, distance_m: cumul[i] });
            prochain = cumul[i] + pas;
        }
    }
    if let Some(&dernier) = trace.last() {
        if sortie.last().is_some_and(|s| (s.lat, s.lon) != dernier) {
            sortie.push(Echantillon { lat: dernier.0, lon: dernier.1, distance_m: d });
        }
    }
    sortie
}

/// Points qui dépassent un seuil.
pub fn infractions<'a>(pts: &'a [PointMeteo], s: &Seuils) -> Vec<&'a PointMeteo> {
    pts.iter().filter(|p| s.depasse(p)).collect()
}

/// Zones d'évitement (centre, rayon) autour des points en infraction ; un point à moins d'un demi-rayon
/// d'une zone déjà posée n'en crée pas de nouvelle.
pub fn zones_depuis(inf: &[&PointMeteo]) -> Vec<(f64, f64, f64)> {
    let mut zones: Vec<(f64, f64, f64)> = Vec::new();
    for p in inf {
        if zones.iter().all(|z| geo::distance_m((z.0, z.1), (p.lat, p.lon)) > RAYON_ZONE_M / 2.0) {
            zones.push((p.lat, p.lon, RAYON_ZONE_M));
        }
    }
    zones
}

/// Zone de météo fixe pour `FournisseurSimule`.
#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
pub struct ZoneSimulee {
    pub lat: f64,
    pub lon: f64,
    pub rayon_m: f64,
    #[serde(default)]
    pub pluie_mmh: f64,
    #[serde(default)]
    pub rafales_kmh: f64,
    #[serde(default = "froid_ressenti_calme")]
    pub froid_c: f64,
}

fn froid_ressenti_calme() -> f64 {
    15.0
}

/// Météo simulée : temps calme (0 mm/h, 20 km/h, 15 °C) hors des zones, valeurs de la zone dedans.
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct FournisseurSimule {
    pub zones: Vec<ZoneSimulee>,
}

impl FournisseurMeteo for FournisseurSimule {
    fn par_point(&self, points: &[Echantillon], _depart_a: Option<&str>, _vitesse_kmh: f64) -> Result<Vec<PointMeteo>> {
        Ok(points
            .iter()
            .map(|e| {
                let mut m = PointMeteo { lat: e.lat, lon: e.lon, distance_m: e.distance_m, pluie_mmh: 0.0, rafales_kmh: 20.0, froid_c: 15.0 };
                for z in &self.zones {
                    if geo::distance_m((z.lat, z.lon), (e.lat, e.lon)) <= z.rayon_m {
                        m.pluie_mmh = m.pluie_mmh.max(z.pluie_mmh);
                        m.rafales_kmh = m.rafales_kmh.max(z.rafales_kmh);
                        m.froid_c = m.froid_c.min(z.froid_c);
                    }
                }
                m
            })
            .collect())
    }
}

/// Fournisseur de la Lambda tant que `POST /v1/route` de RuneRide Météo n'existe pas (R5) : toujours en erreur,
/// ce qui fait rendre la balade sans contrainte météo, avec l'avertissement « météo indisponible ».
pub struct FournisseurAbsent;

impl FournisseurMeteo for FournisseurAbsent {
    fn par_point(&self, _: &[Echantillon], _: Option<&str>, _: f64) -> Result<Vec<PointMeteo>> {
        bail!("aucune source météo branchée")
    }
}

/// Date de départ ISO 8601 plausible : `AAAA-MM-JJTHH:MM` au moins (le fuseau éventuel est laissé à la météo).
pub fn depart_a_valide(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 16
        && b.len() <= 40
        && b[4] == b'-'
        && b[7] == b'-'
        && (b[10] == b'T' || b[10] == b' ')
        && b[13] == b':'
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15].iter().all(|&i| b[i].is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ligne() -> Vec<(f64, f64)> {
        // 20 km plein nord, un point tous les 500 m
        (0..=40).map(|i| geo::destination(45.0, 5.0, 0.0, i as f64 * 500.0)).collect()
    }

    #[test]
    fn defauts_et_bornes() {
        let s: Seuils = serde_json::from_str("{}").unwrap();
        assert_eq!((s.pluie_mmh, s.rafales_kmh, s.froid_c), (1.0, 60.0, 5.0), "défauts de la spec O2");
        assert!(s.verifier().is_ok());
        for mauvais in [r#"{"pluie_mmh":-1}"#, r#"{"pluie_mmh":51}"#, r#"{"rafales_kmh":5}"#, r#"{"froid_c":40}"#] {
            assert!(serde_json::from_str::<Seuils>(mauvais).unwrap().verifier().is_err(), "{mauvais}");
        }
    }

    #[test]
    fn echantillonnage_garde_les_extremites_et_un_pas_d_un_km() {
        let e = echantillonner(&ligne());
        assert!((e[0].lat - 45.0).abs() < 1e-9 && (e[0].lon - 5.0).abs() < 1e-9);
        let d = e.last().unwrap();
        assert!((d.distance_m - 20_000.0).abs() < 20.0, "{}", d.distance_m);
        assert!(e.len() >= 10 && e.len() <= 22, "{}", e.len());
        assert!(e.windows(2).all(|w| w[1].distance_m > w[0].distance_m));
        assert!(echantillonner(&[]).is_empty());
    }

    #[test]
    fn plafond_de_points() {
        let longue: Vec<_> = (0..=3000).map(|i| geo::destination(45.0, 5.0, 0.0, i as f64 * 500.0)).collect();
        assert!(echantillonner(&longue).len() <= POINTS_MAX + 2);
    }

    #[test]
    fn zone_de_pluie_donne_des_infractions_puis_des_zones() {
        let l = ligne();
        let f = FournisseurSimule { zones: vec![ZoneSimulee { lat: l[20].0, lon: l[20].1, rayon_m: 2_500.0, pluie_mmh: 4.0, rafales_kmh: 0.0, froid_c: 15.0 }] };
        let pts = f.par_point(&echantillonner(&l), None, 50.0).unwrap();
        let s = Seuils::default();
        let inf = infractions(&pts, &s);
        assert!(!inf.is_empty() && inf.len() < pts.len());
        let z = zones_depuis(&inf);
        assert!(!z.is_empty() && z.len() <= inf.len());
        assert!(z.iter().all(|z| z.2 == RAYON_ZONE_M));
        // sans dépassement : rien
        let calme = f.par_point(&echantillonner(&l[..10]), None, 50.0).unwrap();
        assert!(infractions(&calme, &s).is_empty());
    }

    #[test]
    fn seuils_rafales_et_froid() {
        let s = Seuils::default();
        let base = PointMeteo { lat: 0.0, lon: 0.0, distance_m: 0.0, pluie_mmh: 1.0, rafales_kmh: 60.0, froid_c: 5.0 };
        assert!(!s.depasse(&base), "égal au seuil : pas d'infraction");
        assert!(s.depasse(&PointMeteo { pluie_mmh: 1.1, ..base }));
        assert!(s.depasse(&PointMeteo { rafales_kmh: 61.0, ..base }));
        assert!(s.depasse(&PointMeteo { froid_c: 4.9, ..base }));
    }

    #[test]
    fn date_de_depart() {
        assert!(depart_a_valide("2026-10-26T14:00"));
        assert!(depart_a_valide("2026-10-26T14:00:00+02:00"));
        assert!(!depart_a_valide("demain 14h"));
        assert!(!depart_a_valide("2026-10-26"));
    }
}
