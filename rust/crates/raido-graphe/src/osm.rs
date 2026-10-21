// SPDX-License-Identifier: EUPL-1.2
//! Lecture du PBF en deux passes : (1) voies carrossables, restrictions de tourner
//! et localités ; (2) coordonnées des seuls nœuds référencés par ces voies.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use osmpbf::{Element, ElementReader, RelMemberType};
use raido_format::{drapeau, Classe};

#[derive(Clone, Debug)]
pub struct Voie {
    pub id: i64,
    pub noeuds: Vec<i64>,
    pub classe: Classe,
    /// 0 = double sens, 1 = sens des nœuds, -1 = sens inverse.
    pub sens: i8,
    pub vitesse_kmh: Option<u16>,
    pub drapeaux: u8,
}

#[derive(Clone, Debug)]
pub struct RestrictionOsm {
    pub de_voie: i64,
    pub via_noeud: i64,
    pub vers_voie: i64,
    pub seulement: bool,
}

#[derive(Clone, Debug)]
pub struct Localite {
    pub lat: f64,
    pub lon: f64,
    pub population: u32,
}

/// Nature d'un point ou d'une zone qui compte pour le score scénique (spec §4.2, §4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Genre {
    /// `mountain_pass=yes`, ou `natural=saddle` nommé « Col … », « Cormet … », « Pas … »
    Col,
    /// `tourism=viewpoint`
    Panorama,
    /// `natural=peak`
    Sommet,
    /// `natural=water`
    Eau,
    /// `landuse=forest`, `natural=wood`
    Foret,
    /// `leisure=nature_reserve`, `boundary=national_park`, `boundary=protected_area`
    Parc,
    /// `landuse=industrial|commercial|retail` : pénalise
    Activite,
}

/// Point scénique ponctuel (nœud) ou échantillon du contour d'une zone.
#[derive(Clone, Debug)]
pub struct Poi {
    pub genre: Genre,
    pub lat: f64,
    pub lon: f64,
    /// Nom OSM (cols, sommets, panoramas), pour les diagnostics.
    pub nom: Option<String>,
}

/// Zone (way) à échantillonner une fois les coordonnées lues.
#[derive(Clone, Debug)]
pub struct ZoneBrute {
    pub genre: Genre,
    pub noeuds: Vec<i64>,
}

#[derive(Default)]
pub struct Extrait {
    pub voies: Vec<Voie>,
    pub restrictions: Vec<RestrictionOsm>,
    pub localites: Vec<Localite>,
    /// Points scéniques portés par des nœuds.
    pub pois: Vec<Poi>,
    /// Zones (ways) à échantillonner ; résolues par `echantillonner_zones` après la lecture des coordonnées.
    pub zones: Vec<ZoneBrute>,
    /// Ways membres d'une relation de route touristique (`route=motorbike`, ou `route=road` avec `tourism` ou `scenic=yes`).
    pub routes_touristiques: HashSet<i64>,
    /// Coordonnées (lat, lon en degrés) des nœuds référencés.
    pub coords: HashMap<i64, (f64, f64)>,
}

/// Genre d'un nœud scénique d'après ses étiquettes.
fn genre_noeud(tags: &HashMap<&str, &str>) -> Option<Genre> {
    let nom_de_col = tags.get("name").is_some_and(|n| n.starts_with("Col ") || n.starts_with("Cormet ") || n.starts_with("Pas "));
    // Beaucoup de cols sont étiquetés `natural=saddle` (selle) avec un nom en « Col … » plutôt que `mountain_pass=yes`.
    if tags.get("mountain_pass") == Some(&"yes") || (tags.get("natural") == Some(&"saddle") && nom_de_col) {
        Some(Genre::Col)
    } else if tags.get("tourism") == Some(&"viewpoint") {
        Some(Genre::Panorama)
    } else if tags.get("natural") == Some(&"peak") {
        Some(Genre::Sommet)
    } else {
        None
    }
}

/// Genre d'une zone (way) d'après ses étiquettes.
fn genre_zone(tags: &HashMap<&str, &str>) -> Option<Genre> {
    if tags.get("natural") == Some(&"water") {
        Some(Genre::Eau)
    } else if tags.get("landuse") == Some(&"forest") || tags.get("natural") == Some(&"wood") {
        Some(Genre::Foret)
    } else if tags.get("leisure") == Some(&"nature_reserve") || matches!(tags.get("boundary").copied(), Some("national_park") | Some("protected_area")) {
        Some(Genre::Parc)
    } else if matches!(tags.get("landuse").copied(), Some("industrial") | Some("commercial") | Some("retail")) {
        Some(Genre::Activite)
    } else {
        None
    }
}

/// Une relation de route est « touristique » pour les motards si elle est balisée comme telle.
fn route_touristique(tags: &HashMap<&str, &str>) -> bool {
    tags.get("type") == Some(&"route")
        && match tags.get("route").copied() {
            Some("motorbike") => true,
            Some("road") => tags.contains_key("tourism") || tags.get("scenic") == Some(&"yes"),
            _ => false,
        }
}

fn classe(highway: &str) -> Option<Classe> {
    Some(match highway {
        "motorway" => Classe::Autoroute,
        "trunk" => Classe::Nationale,
        "primary" => Classe::Primaire,
        "secondary" => Classe::Secondaire,
        "tertiary" => Classe::Tertiaire,
        "unclassified" => Classe::NonClassee,
        "residential" => Classe::Residentielle,
        "motorway_link" | "trunk_link" | "primary_link" | "secondary_link" | "tertiary_link" => Classe::Bretelle,
        "living_street" => Classe::Zone,
        _ => return None,
    })
}

const NON_REVETU: &[&str] = &[
    "unpaved", "gravel", "dirt", "ground", "grass", "sand", "compacted", "fine_gravel", "pebblestone", "earth",
    "mud", "woodchips", "rock",
];

fn vitesse(maxspeed: &str) -> Option<u16> {
    let s = maxspeed.trim();
    if let Some(mph) = s.strip_suffix("mph") {
        return mph.trim().parse::<f32>().ok().map(|v| (v * 1.609) as u16);
    }
    match s {
        "FR:urban" => Some(50),
        "FR:rural" => Some(80),
        "FR:motorway" => Some(130),
        "walk" => Some(10),
        "none" => Some(130),
        _ => s.parse::<u16>().ok().filter(|v| (5..=150).contains(v)),
    }
}

/// Interprète les étiquettes d'un way ; `None` si la voie n'est pas carrossable à moto.
fn voie_depuis(id: i64, tags: &HashMap<&str, &str>, noeuds: Vec<i64>) -> Option<Voie> {
    let mut classe = classe(tags.get("highway")?)?;
    // Accès : on exclut les voies privées ou interdites aux véhicules à moteur, sauf mention explicite moto.
    let moto = tags.get("motorcycle").copied();
    if matches!(moto, Some("no") | Some("private")) {
        return None;
    }
    if moto.is_none() {
        let mv = tags.get("motor_vehicle").copied();
        if matches!(mv, Some("no") | Some("private") | Some("agricultural") | Some("forestry")) {
            return None;
        }
        if mv.is_none() && matches!(tags.get("access").copied(), Some("no") | Some("private")) {
            return None;
        }
    }
    if tags.get("area") == Some(&"yes") || noeuds.len() < 2 {
        return None;
    }
    let mut drapeaux = 0u8;
    let rond_point = tags.get("junction").is_some_and(|j| *j == "roundabout" || *j == "circular");
    if rond_point {
        drapeaux |= drapeau::ROND_POINT;
    }
    let mut sens: i8 = match tags.get("oneway").copied() {
        Some("yes") | Some("1") | Some("true") => 1,
        Some("-1") | Some("reverse") => -1,
        Some("no") | Some("0") | Some("false") => 0,
        _ => {
            if rond_point || classe == Classe::Autoroute {
                1
            } else {
                0
            }
        }
    };
    // Une bretelle d'autoroute sans oneway explicite est à sens unique en pratique.
    if classe == Classe::Bretelle && tags.get("highway") == Some(&"motorway_link") && tags.get("oneway").is_none() {
        sens = 1;
    }
    if tags.get("toll") == Some(&"yes") {
        drapeaux |= drapeau::PEAGE;
    }
    if tags.get("tunnel").is_some_and(|t| *t != "no") {
        drapeaux |= drapeau::TUNNEL;
    }
    match tags.get("surface") {
        Some(s) if NON_REVETU.contains(s) => drapeaux |= drapeau::NON_REVETU,
        Some(_) => {}
        None => {
            if matches!(classe, Classe::NonClassee | Classe::Residentielle | Classe::Zone) {
                drapeaux |= drapeau::REVETEMENT_INCONNU;
            }
        }
    }
    if tags.get("smoothness").is_some_and(|s| matches!(*s, "bad" | "very_bad" | "horrible" | "very_horrible" | "impassable")) {
        drapeaux |= drapeau::NON_REVETU;
    }
    // Les `trunk` à 2×2 voies sont des quasi-autoroutes ; on garde la classe mais on le note via la vitesse.
    let vitesse_kmh = tags.get("maxspeed").and_then(|v| vitesse(v));
    if classe == Classe::Nationale && tags.get("expressway") == Some(&"yes") {
        classe = Classe::Autoroute;
    }
    Some(Voie { id, noeuds, classe, sens, vitesse_kmh, drapeaux })
}

fn restriction_depuis(tags: &HashMap<&str, &str>, membres: &[(RelMemberType, i64, String)]) -> Option<RestrictionOsm> {
    if tags.get("type") != Some(&"restriction") {
        return None;
    }
    let r = tags.get("restriction").or_else(|| tags.get("restriction:motorcycle")).copied()?;
    if tags.get("except").is_some_and(|e| e.split(';').any(|x| x == "motorcycle")) {
        return None;
    }
    let seulement = if r.starts_with("no_") {
        false
    } else if r.starts_with("only_") {
        true
    } else {
        return None;
    };
    let mut de = None;
    let mut vers = None;
    let mut via = None;
    for (t, id, role) in membres {
        match (role.as_str(), t) {
            ("from", RelMemberType::Way) => de = Some(*id),
            ("to", RelMemberType::Way) => vers = Some(*id),
            ("via", RelMemberType::Node) => via = Some(*id),
            // Pivot = way : restriction complexe, ignorée dans ce PoC.
            ("via", _) => return None,
            _ => {}
        }
    }
    Some(RestrictionOsm { de_voie: de?, via_noeud: via?, vers_voie: vers?, seulement })
}

const LOCALITES: &[&str] = &["city", "town", "village"];

/// Lit le PBF (deux passes parallèles) et renvoie les données utiles au graphe.
pub fn lire(pbf: &Path) -> Result<Extrait> {
    let lecteur = ElementReader::from_path(pbf).with_context(|| format!("ouverture de {}", pbf.display()))?;
    let mut ex = lecteur
        .par_map_reduce(
            |e| {
                let mut acc = Extrait::default();
                match e {
                    Element::Way(w) => {
                        let tags: HashMap<&str, &str> = w.tags().collect();
                        if tags.contains_key("highway") {
                            if let Some(v) = voie_depuis(w.id(), &tags, w.refs().collect()) {
                                acc.voies.push(v);
                            }
                        } else if let Some(genre) = genre_zone(&tags) {
                            acc.zones.push(ZoneBrute { genre, noeuds: w.refs().collect() });
                        }
                    }
                    Element::Relation(r) => {
                        let tags: HashMap<&str, &str> = r.tags().collect();
                        if route_touristique(&tags) {
                            acc.routes_touristiques.extend(r.members().filter(|m| m.member_type == RelMemberType::Way).map(|m| m.member_id));
                        }
                        if tags.get("type") == Some(&"restriction") {
                            let membres: Vec<_> =
                                r.members()
                                    .map(|m| {
                                        let role = m.role().unwrap_or("").to_string();
                                        (m.member_type, m.member_id, role)
                                    })
                                    .collect();
                            if let Some(res) = restriction_depuis(&tags, &membres) {
                                acc.restrictions.push(res);
                            }
                        }
                    }
                    Element::Node(n) => {
                        let tags: HashMap<&str, &str> = n.tags().collect();
                        if let Some(l) = localite(&tags, n.lat(), n.lon()) {
                            acc.localites.push(l);
                        }
                        if let Some(genre) = genre_noeud(&tags) {
                            acc.pois.push(Poi { genre, lat: n.lat(), lon: n.lon(), nom: tags.get("name").map(|x| x.to_string()) });
                        }
                    }
                    Element::DenseNode(n) => {
                        let tags: HashMap<&str, &str> = n.tags().collect();
                        if let Some(l) = localite(&tags, n.lat(), n.lon()) {
                            acc.localites.push(l);
                        }
                        if let Some(genre) = genre_noeud(&tags) {
                            acc.pois.push(Poi { genre, lat: n.lat(), lon: n.lon(), nom: tags.get("name").map(|x| x.to_string()) });
                        }
                    }
                }
                acc
            },
            Extrait::default,
            |mut a, b| {
                a.voies.extend(b.voies);
                a.restrictions.extend(b.restrictions);
                a.localites.extend(b.localites);
                a.pois.extend(b.pois);
                a.zones.extend(b.zones);
                a.routes_touristiques.extend(b.routes_touristiques);
                a
            },
        )
        .context("première passe (voies, restrictions, localités)")?;

    let mut voulus: HashSet<i64> = ex.voies.iter().flat_map(|v| v.noeuds.iter().copied()).collect();
    let nb_noeuds_voies = voulus.len();
    voulus.extend(ex.zones.iter().flat_map(|z| z.noeuds.iter().copied()));
    let _ = nb_noeuds_voies;
    let lecteur = ElementReader::from_path(pbf)?;
    ex.coords = lecteur
        .par_map_reduce(
            |e| {
                let mut m = HashMap::new();
                match e {
                    Element::Node(n) if voulus.contains(&n.id()) => {
                        m.insert(n.id(), (n.lat(), n.lon()));
                    }
                    Element::DenseNode(n) if voulus.contains(&n.id()) => {
                        m.insert(n.id(), (n.lat(), n.lon()));
                    }
                    _ => {}
                }
                m
            },
            HashMap::new,
            |mut a, mut b| {
                // On étend toujours la plus grande table : moins de réallocations.
                if a.len() < b.len() {
                    std::mem::swap(&mut a, &mut b);
                }
                a.extend(b);
                a
            },
        )
        .context("seconde passe (coordonnées)")?;
    ex.echantillonner_zones();
    Ok(ex)
}

/// Pas minimal (m) entre deux échantillons d'un contour de zone : assez fin pour la portée de 300 m du score scénique.
const PAS_ECHANTILLON_ZONE_M: f64 = 120.0;

impl Extrait {
    /// Transforme les zones en points échantillonnés le long de leur contour (un tous les ~120 m), puis libère les
    /// listes de nœuds. Les coordonnées de ces nœuds ne servent plus ensuite : on les retire pour ne pas gonfler le graphe.
    pub fn echantillonner_zones(&mut self) {
        let voies: HashSet<i64> = self.voies.iter().flat_map(|v| v.noeuds.iter().copied()).collect();
        let zones = std::mem::take(&mut self.zones);
        let mut utilises: HashSet<i64> = HashSet::new();
        for z in &zones {
            let mut dernier: Option<(f64, f64)> = None;
            for n in &z.noeuds {
                let Some(&(lat, lon)) = self.coords.get(n) else { continue };
                let loin = dernier.is_none_or(|(la, lo)| {
                    let dy = (lat - la) * 110_540.0;
                    let dx = (lon - lo) * 111_320.0 * lat.to_radians().cos();
                    (dx * dx + dy * dy).sqrt() >= PAS_ECHANTILLON_ZONE_M
                });
                if loin {
                    self.pois.push(Poi { genre: z.genre, lat, lon, nom: None });
                    dernier = Some((lat, lon));
                }
            }
            utilises.extend(z.noeuds.iter().copied());
        }
        self.coords.retain(|n, _| voies.contains(n) || !utilises.contains(n));
    }
}

fn localite(tags: &HashMap<&str, &str>, lat: f64, lon: f64) -> Option<Localite> {
    let place = tags.get("place")?;
    if !LOCALITES.contains(place) {
        return None;
    }
    let population = tags
        .get("population")
        .and_then(|p| p.replace(' ', "").split(';').next().and_then(|x| x.parse::<u32>().ok()))
        .unwrap_or(match *place {
            "city" => 100_000,
            "town" => 10_000,
            _ => 500,
        });
    Some(Localite { lat, lon, population })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags<'a>(l: &[(&'a str, &'a str)]) -> HashMap<&'a str, &'a str> {
        l.iter().copied().collect()
    }

    #[test]
    fn voie_simple_double_sens() {
        let v = voie_depuis(1, &tags(&[("highway", "tertiary"), ("maxspeed", "70")]), vec![1, 2, 3]).unwrap();
        assert_eq!(v.classe, Classe::Tertiaire);
        assert_eq!(v.sens, 0);
        assert_eq!(v.vitesse_kmh, Some(70));
    }

    #[test]
    fn autoroute_sens_unique_implicite_et_peage() {
        let v = voie_depuis(1, &tags(&[("highway", "motorway"), ("toll", "yes")]), vec![1, 2]).unwrap();
        assert_eq!(v.sens, 1);
        assert_ne!(v.drapeaux & drapeau::PEAGE, 0);
    }

    #[test]
    fn exclusions() {
        assert!(voie_depuis(1, &tags(&[("highway", "track")]), vec![1, 2]).is_none());
        assert!(voie_depuis(1, &tags(&[("highway", "residential"), ("access", "private")]), vec![1, 2]).is_none());
        assert!(voie_depuis(1, &tags(&[("highway", "residential"), ("motor_vehicle", "no")]), vec![1, 2]).is_none());
        // Interdit aux voitures mais ouvert aux motos : gardé.
        assert!(voie_depuis(1, &tags(&[("highway", "residential"), ("motor_vehicle", "no"), ("motorcycle", "yes")]), vec![1, 2]).is_some());
    }

    #[test]
    fn cols_et_zones_reconnus() {
        assert_eq!(genre_noeud(&tags(&[("mountain_pass", "yes")])), Some(Genre::Col));
        assert_eq!(genre_noeud(&tags(&[("natural", "saddle"), ("name", "Col du Lautaret")])), Some(Genre::Col));
        assert_eq!(genre_noeud(&tags(&[("natural", "saddle"), ("name", "Selle de Machin")])), None, "une selle sans nom de col n'en est pas un");
        assert_eq!(genre_noeud(&tags(&[("tourism", "viewpoint")])), Some(Genre::Panorama));
        assert_eq!(genre_noeud(&tags(&[("natural", "peak")])), Some(Genre::Sommet));
        assert_eq!(genre_zone(&tags(&[("natural", "water")])), Some(Genre::Eau));
        assert_eq!(genre_zone(&tags(&[("landuse", "forest")])), Some(Genre::Foret));
        assert_eq!(genre_zone(&tags(&[("boundary", "national_park")])), Some(Genre::Parc));
        assert_eq!(genre_zone(&tags(&[("landuse", "retail")])), Some(Genre::Activite));
        assert_eq!(genre_zone(&tags(&[("landuse", "farmland")])), None);
        assert!(route_touristique(&tags(&[("type", "route"), ("route", "motorbike")])));
        assert!(route_touristique(&tags(&[("type", "route"), ("route", "road"), ("tourism", "yes")])));
        assert!(!route_touristique(&tags(&[("type", "route"), ("route", "road"), ("network", "e-road")])), "une route numérotée ordinaire n'est pas touristique");
        assert!(!route_touristique(&tags(&[("type", "route"), ("route", "bicycle"), ("tourism", "yes")])));
    }

    #[test]
    fn vitesse_fr() {
        assert_eq!(vitesse("FR:rural"), Some(80));
        assert_eq!(vitesse("30 mph"), Some(48));
        assert_eq!(vitesse("n/a"), None);
    }
}
