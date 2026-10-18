// SPDX-License-Identifier: EUPL-1.2
//! Scores par tronçon (spec §4.3) et temps de parcours de base.
//! J1 : sinuosité et agglomération ; scénique et motard restent à 0 (jalons J2 et J7).

use raido_format::{haversine_m, Classe, Point};

use crate::osm::Localite;

/// Vitesse de base à moto en balade, par classe de route (km/h).
/// Valeurs provisoires (J1 donnait 40 à 50 km/h de moyenne en montagne, jugé
/// pessimiste) ; la calibration sur les trajets GeoRide de l'utilisateur (spec O3)
/// les remplacera.
pub fn vitesse_classe(c: Classe) -> f64 {
    match c {
        Classe::Autoroute => 115.0,
        Classe::Nationale => 90.0,
        Classe::Primaire => 80.0,
        Classe::Secondaire => 75.0,
        Classe::Tertiaire => 65.0,
        Classe::NonClassee => 55.0,
        Classe::Residentielle => 40.0,
        Classe::Bretelle => 60.0,
        Classe::Zone => 20.0,
    }
}

/// Longueur (m) et somme des changements de cap (rad) d'une polyligne.
pub fn longueur_et_virages(pts: &[Point]) -> (f64, f64) {
    let mut longueur = 0.0;
    let mut virages = 0.0;
    let mut cap_prec: Option<f64> = None;
    for w in pts.windows(2) {
        let d = haversine_m(w[0], w[1]);
        if d < 0.5 {
            continue;
        }
        longueur += d;
        let cap = cap_rad(w[0], w[1]);
        if let Some(p) = cap_prec {
            let mut delta = (cap - p).abs();
            if delta > std::f64::consts::PI {
                delta = 2.0 * std::f64::consts::PI - delta;
            }
            virages += delta;
        }
        cap_prec = Some(cap);
    }
    (longueur, virages)
}

fn cap_rad(a: Point, b: Point) -> f64 {
    let (la1, la2) = (a.lat_deg().to_radians(), b.lat_deg().to_radians());
    let dlo = (b.lon_deg() - a.lon_deg()).to_radians();
    (dlo.sin() * la2.cos()).atan2(la1.cos() * la2.sin() - la1.sin() * la2.cos() * dlo.cos())
}

/// Sinuosité 0..1 : radians de virage par kilomètre, saturée à 4 rad/km
/// (≈ deux épingles par kilomètre).  Les tronçons très courts héritent d'un
/// plafond pour ne pas sur-noter un simple raccord d'intersection.
pub fn sinuosite(longueur_m: f64, virages_rad: f64) -> f64 {
    if longueur_m < 20.0 {
        return 0.0;
    }
    let par_km = virages_rad / (longueur_m / 1000.0);
    let s = (par_km / 4.0).min(1.0);
    if longueur_m < 150.0 {
        s.min(0.5)
    } else {
        s
    }
}

/// Temps de parcours en décisecondes : vitesse de classe, bornée par `maxspeed`,
/// ralentie par la sinuosité (jusqu'à −25 %) et par les ronds-points.
pub fn temps_ds(longueur_m: f64, classe: Classe, maxspeed: Option<u16>, sinuosite: f64, rond_point: bool) -> u32 {
    let mut v = vitesse_classe(classe);
    if let Some(m) = maxspeed {
        v = v.min(m as f64);
    }
    v *= 1.0 - 0.25 * sinuosite;
    if rond_point {
        v = v.min(30.0);
    }
    let v_ms = v.max(5.0) / 3.6;
    ((longueur_m / v_ms) * 10.0).round().max(1.0) as u32
}

/// Index spatial grossier des localités (grille de 0,05°) pour le score d'agglomération.
pub struct Agglomerations {
    cellules: std::collections::HashMap<(i32, i32), Vec<Localite>>,
}

impl Agglomerations {
    pub const SEUIL_POPULATION: u32 = 5_000;
    const PAS: f64 = 0.05;

    pub fn nouvelle(localites: &[Localite]) -> Agglomerations {
        let mut cellules: std::collections::HashMap<(i32, i32), Vec<Localite>> = Default::default();
        for l in localites.iter().filter(|l| l.population >= Self::SEUIL_POPULATION) {
            cellules.entry(Self::cellule(l.lat, l.lon)).or_default().push(l.clone());
        }
        Agglomerations { cellules }
    }

    fn cellule(lat: f64, lon: f64) -> (i32, i32) {
        ((lat / Self::PAS).floor() as i32, (lon / Self::PAS).floor() as i32)
    }

    /// Rayon (m) de l'agglomération selon sa population : 5 000 hab. ≈ 1 km, 100 000 ≈ 4,3 km.
    fn rayon_m(population: u32) -> f64 {
        13.5 * (population as f64).sqrt()
    }

    /// Score 0..1 : 1 dans l'agglomération, décroissant jusqu'à 0 à 2 km de sa limite.
    pub fn score(&self, p: Point) -> f64 {
        let (ci, cj) = Self::cellule(p.lat_deg(), p.lon_deg());
        // Les plus grandes villes (rayon ≈ 10 km) débordent de 4 cellules.
        let mut meilleur = 0.0f64;
        for i in ci - 4..=ci + 4 {
            for j in cj - 4..=cj + 4 {
                if let Some(ls) = self.cellules.get(&(i, j)) {
                    for l in ls {
                        let d = haversine_m(p, Point::depuis_degres(l.lat, l.lon));
                        let r = Self::rayon_m(l.population);
                        let s = if d <= r { 1.0 } else { (1.0 - (d - r) / 2_000.0).max(0.0) };
                        meilleur = meilleur.max(s);
                    }
                }
            }
        }
        meilleur
    }
}

pub fn en_u8(score: f64) -> u8 {
    (score.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ligne_droite_sans_virage() {
        let pts = [Point::depuis_degres(45.0, 5.0), Point::depuis_degres(45.0, 5.01), Point::depuis_degres(45.0, 5.02)];
        let (l, v) = longueur_et_virages(&pts);
        assert!((l - 1_572.0).abs() < 20.0, "{l}");
        assert!(v < 1e-6);
        assert_eq!(sinuosite(l, v), 0.0);
    }

    #[test]
    fn epingles_saturent() {
        // Quatre demi-tours sur 1 km : 4π rad/km > 4 → saturé.
        assert_eq!(sinuosite(1_000.0, 4.0 * std::f64::consts::PI), 1.0);
        assert!((sinuosite(1_000.0, 2.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn temps_coherent() {
        // 10 km de tertiaire droite à 65 km/h = 553,8 s = 5538 ds.
        assert_eq!(temps_ds(10_000.0, Classe::Tertiaire, None, 0.0, false), 5_538);
        assert!(temps_ds(10_000.0, Classe::Tertiaire, Some(50), 0.0, false) > 5_538);
        assert!(temps_ds(10_000.0, Classe::Tertiaire, None, 1.0, false) > 5_538);
    }

    #[test]
    fn agglomeration() {
        let a = Agglomerations::nouvelle(&[Localite { lat: 45.76, lon: 4.84, population: 500_000 }]);
        assert_eq!(a.score(Point::depuis_degres(45.76, 4.84)), 1.0);
        assert_eq!(a.score(Point::depuis_degres(46.5, 4.84)), 0.0);
    }
}
