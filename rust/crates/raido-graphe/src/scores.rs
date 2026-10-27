// SPDX-License-Identifier: EUPL-1.2
//! Scores par tronçon (spec §4.3) et temps de parcours de base.
//! J1 : sinuosité et agglomération ; scénique et motard restent à 0 (jalons J2 et J7).

use raido_format::{haversine_m, Classe, Point};

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::osm::{Genre, Localite, Poi};

/// Vitesse de base à moto en balade, par classe de route (km/h) : table de `raido_format::Classe`.
pub fn vitesse_classe(c: Classe) -> f64 {
    c.vitesse_defaut_kmh()
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

/// Poids du score scénique (spec §4.3). **Valeurs provisoires, à calibrer sur des balades connues (jalon J2)** : le fichier
/// JSON passé à `raido-graphe construire --poids` remplace ceux de ce fichier, champ par champ, sans recompiler.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PoidsScenique {
    /// Portée (m) d'un point ou d'un échantillon de contour autour d'un tronçon.
    pub rayon_m: f64,
    /// Pas (m) de l'échantillonnage le long d'un tronçon.
    pub echantillon_m: f64,
    pub col: f64,
    pub panorama: f64,
    pub sommet: f64,
    pub eau: f64,
    pub foret: f64,
    pub parc: f64,
    /// Bonus d'appartenance à une relation de route touristique.
    pub route_touristique: f64,
    /// Pénalité en zone industrielle ou commerciale (retranchée du bonus).
    pub activite: f64,
    /// Poids du terme « route de montagne » (altitude et dominance, `relief::Mnt::terme_relief`) ; sans effet sans `--relief`.
    pub relief: f64,
}

impl Default for PoidsScenique {
    fn default() -> Self {
        PoidsScenique { rayon_m: 300.0, echantillon_m: 150.0, col: 0.9, panorama: 0.7, sommet: 0.4, eau: 0.35, foret: 0.3, parc: 0.3, route_touristique: 0.5, activite: 0.5, relief: 0.35 }
    }
}

impl PoidsScenique {
    fn de(&self, g: Genre) -> f64 {
        match g {
            Genre::Col => self.col,
            Genre::Panorama => self.panorama,
            Genre::Sommet => self.sommet,
            Genre::Eau => self.eau,
            Genre::Foret => self.foret,
            Genre::Parc => self.parc,
            Genre::Activite => self.activite,
        }
    }
}

/// Index spatial des points scéniques (grille de 0,005°, ≈ 550 m) et calcul du score scénique d'un tronçon.
pub struct Scenique {
    poids: PoidsScenique,
    cellules: HashMap<(i32, i32), Vec<(Genre, f64, f64)>>,
    routes: HashSet<i64>,
}

impl Scenique {
    const PAS: f64 = 0.005;

    pub fn nouveau(pois: &[Poi], routes_touristiques: &HashSet<i64>, poids: PoidsScenique) -> Scenique {
        let mut cellules: HashMap<(i32, i32), Vec<(Genre, f64, f64)>> = HashMap::new();
        for p in pois {
            cellules.entry(Self::cellule(p.lat, p.lon)).or_default().push((p.genre, p.lat, p.lon));
        }
        Scenique { poids, cellules, routes: routes_touristiques.clone() }
    }

    fn cellule(lat: f64, lon: f64) -> (i32, i32) {
        ((lat / Self::PAS).floor() as i32, (lon / Self::PAS).floor() as i32)
    }

    /// Genres présents à moins de `rayon_m` du point.
    fn genres_proches(&self, lat: f64, lon: f64) -> HashSet<Genre> {
        let (ci, cj) = Self::cellule(lat, lon);
        let marge = (self.poids.rayon_m / 550.0).ceil() as i32;
        let (ky, kx) = (110_540.0, 111_320.0 * lat.to_radians().cos());
        let mut genres = HashSet::new();
        for i in ci - marge..=ci + marge {
            for j in cj - marge..=cj + marge {
                for &(g, la, lo) in self.cellules.get(&(i, j)).into_iter().flatten() {
                    let (dy, dx) = ((la - lat) * ky, (lo - lon) * kx);
                    if dx * dx + dy * dy <= self.poids.rayon_m * self.poids.rayon_m {
                        genres.insert(g);
                    }
                }
            }
        }
        genres
    }

    /// Score 0..1 d'un tronçon : bonus (1 − Π(1 − poids × part des échantillons proches d'un point de ce genre)), relevé
    /// par l'appartenance à une route touristique, diminué de la pénalité de zone d'activité.
    pub fn score(&self, pts: &[Point], voie_osm: i64) -> f64 {
        if pts.is_empty() {
            return 0.0;
        }
        // Échantillons : extrémités et un point tous les `echantillon_m` environ le long de la polyligne.
        let mut ech: Vec<Point> = vec![pts[0]];
        let mut depuis = 0.0;
        for w in pts.windows(2) {
            depuis += haversine_m(w[0], w[1]);
            if depuis >= self.poids.echantillon_m {
                ech.push(w[1]);
                depuis = 0.0;
            }
        }
        if ech.last().map(|p| (p.lat, p.lon)) != pts.last().map(|p| (p.lat, p.lon)) {
            ech.push(*pts.last().unwrap());
        }
        let mut compte: HashMap<Genre, usize> = HashMap::new();
        for p in &ech {
            for g in self.genres_proches(p.lat_deg(), p.lon_deg()) {
                *compte.entry(g).or_default() += 1;
            }
        }
        let n = ech.len() as f64;
        let mut reste = 1.0;
        for (&g, &c) in &compte {
            if g != Genre::Activite {
                reste *= 1.0 - self.poids.de(g) * (c as f64 / n);
            }
        }
        if self.routes.contains(&voie_osm) {
            reste *= 1.0 - self.poids.route_touristique;
        }
        let penalite = compte.get(&Genre::Activite).map_or(0.0, |&c| self.poids.activite * c as f64 / n);
        (1.0 - reste - penalite).clamp(0.0, 1.0)
    }
}

pub fn en_u8(score: f64) -> u8 {
    (score.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(lat: f64, lon: f64) -> Point {
        Point::depuis_degres(lat, lon)
    }

    fn poi(genre: Genre, lat: f64, lon: f64) -> Poi {
        Poi { genre, lat, lon, nom: None }
    }

    #[test]
    fn score_scenique_proximite_penalite_et_route_touristique() {
        // Tronçon de ≈ 790 m vers l'est ; un col à ≈ 100 m au nord de son milieu, un panorama à 2 km (hors portée).
        let tr = [pt(45.0, 5.0), pt(45.0, 5.005), pt(45.0, 5.01)];
        let col = poi(Genre::Col, 45.0 + 100.0 / 110_540.0, 5.005);
        let loin = poi(Genre::Panorama, 45.0 + 2_000.0 / 110_540.0, 5.005);
        let sans: HashSet<i64> = HashSet::new();
        let s = Scenique::nouveau(&[col.clone(), loin.clone()], &sans, PoidsScenique::default());
        let v = s.score(&tr, 1);
        assert!(v > 0.3 && v < 0.9, "col proche d'une partie du tronçon : {v}");
        let vide = Scenique::nouveau(&[loin], &sans, PoidsScenique::default());
        assert_eq!(vide.score(&tr, 1), 0.0, "rien dans le rayon");
        let routes: HashSet<i64> = HashSet::from([1]);
        let avec_route = Scenique::nouveau(&[col.clone()], &routes, PoidsScenique::default());
        assert!(avec_route.score(&tr, 1) > s.score(&tr, 1), "la route touristique relève le score");
        assert_eq!(avec_route.score(&tr, 2), s.score(&tr, 1), "une autre voie n'en profite pas");
        let zone = poi(Genre::Activite, 45.0, 5.005);
        let penalise = Scenique::nouveau(&[col, zone], &sans, PoidsScenique::default());
        assert!(penalise.score(&tr, 1) < s.score(&tr, 1), "zone d'activité : pénalité");
        assert!(penalise.score(&tr, 1) >= 0.0);
    }

    #[test]
    fn poids_json_partiels_et_stricts() {
        let p: PoidsScenique = serde_json::from_str(r#"{"col": 1.0, "rayon_m": 200}"#).unwrap();
        assert_eq!((p.col, p.rayon_m, p.panorama), (1.0, 200.0, PoidsScenique::default().panorama), "champs absents : défauts");
        assert!(serde_json::from_str::<PoidsScenique>(r#"{"colle": 1.0}"#).is_err(), "faute de frappe refusée");
    }

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
