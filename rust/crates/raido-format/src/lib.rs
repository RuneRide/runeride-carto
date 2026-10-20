// SPDX-License-Identifier: EUPL-1.2
//! Format des tuiles de graphe Raido (spec `runeride/docs/itineraire.md` §5.3).
//!
//! Une tuile z8 Web Mercator contient ses nœuds (intersections), ses arcs sortants
//! (CSR : `Noeud::premier_arc` indexe `arcs`), la géométrie intermédiaire des arcs
//! et les restrictions de tourner dont le nœud pivot est dans la tuile.  Les
//! identifiants globaux (`NoeudId`, `ArcId`) portent la clé de tuile dans leurs
//! 32 bits hauts, ce qui permet au moteur de charger les tuiles à la demande.
//! Sérialisation `rkyv` (accès zéro copie), version épinglée dans `index.json`.

use rkyv::{Archive, Deserialize, Serialize};

/// Niveau de zoom du découpage en tuiles (z9 : ≈ 75 × 50 km en France ; z8 donnait 41 Mo pour Lyon).
pub const ZOOM: u32 = 9;
/// Version du format : à incrémenter à chaque changement de structure.
pub const VERSION_FORMAT: u32 = 3;

/// Clé de tuile : `x << 16 | y`.
pub type CleTuile = u32;
/// Identifiant global de nœud : `cle_tuile << 32 | indice_local`.
pub type NoeudId = u64;
/// Identifiant global d'arc : `cle_tuile << 32 | indice_local`.
pub type ArcId = u64;
pub const ID_NUL: u64 = u64::MAX;

#[inline]
pub fn id_global(cle: CleTuile, local: u32) -> u64 {
    (cle as u64) << 32 | local as u64
}
#[inline]
pub fn cle_de(id: u64) -> CleTuile {
    (id >> 32) as u32
}
#[inline]
pub fn local_de(id: u64) -> u32 {
    (id & 0xffff_ffff) as u32
}

#[inline]
pub fn cle_xy(x: u32, y: u32) -> CleTuile {
    x << 16 | y
}
#[inline]
pub fn xy_de_cle(cle: CleTuile) -> (u32, u32) {
    (cle >> 16, cle & 0xffff)
}

/// Clé de la tuile contenant un point (Web Mercator, zoom `ZOOM`).
pub fn cle_tuile(lat: f64, lon: f64) -> CleTuile {
    let n = (1u32 << ZOOM) as f64;
    let x = ((lon + 180.0) / 360.0 * n).floor().clamp(0.0, n - 1.0) as u32;
    let lat_r = lat.clamp(-85.05, 85.05).to_radians();
    let y = ((1.0 - (lat_r.tan() + 1.0 / lat_r.cos()).ln() / std::f64::consts::PI) / 2.0 * n)
        .floor()
        .clamp(0.0, n - 1.0) as u32;
    cle_xy(x, y)
}

/// Bornes (sud, ouest, nord, est) d'une tuile, en degrés.
pub fn bornes_tuile(cle: CleTuile) -> (f64, f64, f64, f64) {
    let n = (1u32 << ZOOM) as f64;
    let (xi, yi) = xy_de_cle(cle);
    let (x, y) = (xi as f64, yi as f64);
    let lon = |x: f64| x / n * 360.0 - 180.0;
    let lat = |y: f64| {
        let t = std::f64::consts::PI * (1.0 - 2.0 * y / n);
        t.sinh().atan().to_degrees()
    };
    (lat(y + 1.0), lon(x), lat(y), lon(x + 1.0))
}

/// Classe de route (ordre décroissant d'importance).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classe {
    Autoroute = 0,
    Nationale = 1,
    Primaire = 2,
    Secondaire = 3,
    Tertiaire = 4,
    NonClassee = 5,
    Residentielle = 6,
    Bretelle = 7,
    Zone = 8,
}

impl Classe {
    /// Vitesse de base à moto en balade, par classe de route (km/h). Valeurs provisoires (J1 donnait 40 à 50 km/h de
    /// moyenne en montagne, jugé pessimiste) ; la calibration sur les trajets GeoRide (spec O3) les remplacera.
    pub fn vitesse_defaut_kmh(self) -> f64 {
        match self {
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

    /// Classe réglable par l'utilisateur (nom de l'API : `secondaire`, `non_classee`…). Bretelles et zones ne le sont pas.
    pub fn depuis_nom(nom: &str) -> Option<Classe> {
        Some(match nom {
            "autoroute" => Classe::Autoroute,
            "nationale" => Classe::Nationale,
            "primaire" => Classe::Primaire,
            "secondaire" => Classe::Secondaire,
            "tertiaire" => Classe::Tertiaire,
            "non_classee" => Classe::NonClassee,
            "residentielle" => Classe::Residentielle,
            _ => return None,
        })
    }

    pub fn depuis_u8(v: u8) -> Classe {
        match v {
            0 => Classe::Autoroute,
            1 => Classe::Nationale,
            2 => Classe::Primaire,
            3 => Classe::Secondaire,
            4 => Classe::Tertiaire,
            5 => Classe::NonClassee,
            6 => Classe::Residentielle,
            7 => Classe::Bretelle,
            _ => Classe::Zone,
        }
    }
}

/// Drapeaux d'un arc (champ `Arc::drapeaux`).
pub mod drapeau {
    pub const PEAGE: u8 = 1 << 0;
    pub const TUNNEL: u8 = 1 << 1;
    pub const NON_REVETU: u8 = 1 << 2;
    pub const REVETEMENT_INCONNU: u8 = 1 << 3;
    /// La géométrie intermédiaire est à lire à l'envers (arc retour d'une voie à double sens).
    pub const GEOM_INVERSEE: u8 = 1 << 4;
    pub const ROND_POINT: u8 = 1 << 5;
}

/// Drapeaux d'un nœud (champ `Noeud::drapeaux`).
pub mod drapeau_noeud {
    /// Le nœud appartient à la composante connexe principale du graphe ; les
    /// autres (hameaux reliés par des pistes exclues, îlots) ne servent pas d'accroche.
    pub const PRINCIPAL: u8 = 1 << 0;
}

/// Indices des scores dans `Arc::scores` (0 = nul, 255 = maximal).
pub mod score {
    pub const SINUOSITE: usize = 0;
    pub const SCENIQUE: usize = 1;
    pub const MOTARD: usize = 2;
    pub const AGGLO: usize = 3;
}

#[derive(Archive, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Latitude en 1e-7 degré.
    pub lat: i32,
    /// Longitude en 1e-7 degré.
    pub lon: i32,
}

impl Point {
    pub fn depuis_degres(lat: f64, lon: f64) -> Point {
        Point { lat: (lat * 1e7).round() as i32, lon: (lon * 1e7).round() as i32 }
    }
    pub fn lat_deg(&self) -> f64 {
        self.lat as f64 * 1e-7
    }
    pub fn lon_deg(&self) -> f64 {
        self.lon as f64 * 1e-7
    }
}

#[derive(Archive, Serialize, Deserialize, Clone, Debug)]
pub struct Noeud {
    pub position: Point,
    /// Indice du premier arc sortant dans `Tuile::arcs` ; les arcs du nœud `i`
    /// vont de `noeuds[i].premier_arc` à `noeuds[i+1].premier_arc` (sentinelle finale).
    pub premier_arc: u32,
    pub drapeaux: u8,
}

#[derive(Archive, Serialize, Deserialize, Clone, Debug)]
pub struct Arc {
    pub cible: NoeudId,
    /// Arc parcourant le même tronçon en sens inverse, ou `ID_NUL` (sens unique).
    pub inverse: ArcId,
    pub longueur_m: u32,
    /// Temps de parcours de base en décisecondes.
    pub temps_ds: u32,
    pub classe: u8,
    pub drapeaux: u8,
    /// Sinuosité, scénique, motard, agglomération (§4.3).
    pub scores: [u8; 4],
    /// Identifiant OSM du way et indice du segment entre intersections (identifiant stable §5.3).
    pub way_id: i64,
    pub segment: u16,
    /// Géohash (précision 9) du milieu du tronçon, encodé en base 32 sur 45 bits.
    pub geohash: u64,
    /// Tranche de `Tuile::geometrie` : points intermédiaires (sans les extrémités).
    pub geom_debut: u32,
    pub geom_nb: u16,
}

/// Restriction de tourner dont le pivot est le nœud `via` de cette tuile.
#[derive(Archive, Serialize, Deserialize, Clone, Debug)]
pub struct Restriction {
    pub via: u32,
    pub de: ArcId,
    pub vers: ArcId,
    /// `true` : `only_*` (seule sortie autorisée), `false` : `no_*` (sortie interdite).
    pub seulement: bool,
}

#[derive(Archive, Serialize, Deserialize, Clone, Debug)]
pub struct Tuile {
    pub cle: CleTuile,
    /// Nœuds, plus une sentinelle finale dont seul `premier_arc` compte.
    pub noeuds: Vec<Noeud>,
    pub arcs: Vec<Arc>,
    pub geometrie: Vec<Point>,
    /// Triées par `via`.
    pub restrictions: Vec<Restriction>,
}

/// Index du graphe (`index.json`), lisible sans rkyv.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Index {
    pub version_format: u32,
    pub rkyv: String,
    pub source: String,
    pub construit_le: String,
    pub bornes: [f64; 4],
    pub tuiles: Vec<TuileIndex>,
    pub nb_noeuds: u64,
    pub nb_arcs: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct TuileIndex {
    pub cle: CleTuile,
    pub fichier: String,
    pub octets: u64,
    pub nb_noeuds: u32,
    pub nb_arcs: u32,
}

/// Chemin relatif du fichier d'une tuile.
pub fn chemin_tuile(cle: CleTuile) -> String {
    let (x, y) = xy_de_cle(cle);
    format!("tuiles/{ZOOM}/{x}/{y}.rkyv")
}

/// Encode un géohash de précision 9 (45 bits) en entier.
pub fn geohash_u64(lat: f64, lon: f64) -> u64 {
    let (mut lat_min, mut lat_max, mut lon_min, mut lon_max) = (-90.0, 90.0, -180.0, 180.0);
    let mut bits = 0u64;
    for i in 0..45 {
        if i % 2 == 0 {
            let mid = (lon_min + lon_max) / 2.0;
            if lon >= mid {
                bits = bits << 1 | 1;
                lon_min = mid;
            } else {
                bits <<= 1;
                lon_max = mid;
            }
        } else {
            let mid = (lat_min + lat_max) / 2.0;
            if lat >= mid {
                bits = bits << 1 | 1;
                lat_min = mid;
            } else {
                bits <<= 1;
                lat_max = mid;
            }
        }
    }
    bits
}

/// Distance orthodromique en mètres.
pub fn haversine_m(a: Point, b: Point) -> f64 {
    const R: f64 = 6_371_008.8;
    let (la1, la2) = (a.lat_deg().to_radians(), b.lat_deg().to_radians());
    let dla = la2 - la1;
    let dlo = (b.lon_deg() - a.lon_deg()).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuile_et_bornes_coherentes() {
        let (lat, lon) = (45.76, 4.84); // Lyon
        let cle = cle_tuile(lat, lon);
        let (s, o, n, e) = bornes_tuile(cle);
        assert!(s <= lat && lat < n, "{s} {lat} {n}");
        assert!(o <= lon && lon < e);
        assert_eq!(cle_tuile(s + 1e-6, o + 1e-6), cle);
    }

    #[test]
    fn identifiants_globaux() {
        let id = id_global(0x1234, 42);
        assert_eq!(cle_de(id), 0x1234);
        assert_eq!(local_de(id), 42);
    }

    #[test]
    fn haversine_lyon_grenoble() {
        let d = haversine_m(Point::depuis_degres(45.764, 4.8357), Point::depuis_degres(45.1885, 5.7245));
        assert!((d - 93_500.0).abs() < 1_500.0, "{d}");
    }

    #[test]
    fn geohash_stable() {
        assert_eq!(geohash_u64(45.0, 5.0), geohash_u64(45.0, 5.0));
        assert_ne!(geohash_u64(45.0, 5.0), geohash_u64(45.001, 5.0));
    }
}
