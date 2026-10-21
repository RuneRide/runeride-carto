// SPDX-License-Identifier: EUPL-1.2
//! Découpage des voies aux intersections, création des arcs orientés, résolution
//! des restrictions de tourner et écriture des tuiles rkyv + `index.json`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use raido_format::{
    chemin_tuile, cle_tuile, drapeau, drapeau_noeud, geohash_u64, id_global, Arc, CleTuile, Index, Noeud, NoeudId, Point,
    Restriction, Tuile, TuileIndex, ID_NUL, VERSION_FORMAT,
};

use crate::osm::Extrait;
use crate::scores::{self, Agglomerations, PoidsScenique, Scenique};

/// Arc avant attribution de son indice définitif dans sa tuile.
struct ArcBrut {
    source: NoeudId,
    arc: Arc,
    /// Indice provisoire de l'arc inverse dans `bruts`, ou `usize::MAX`.
    inverse_brut: usize,
    geom: Vec<Point>,
}

#[derive(Default)]
pub struct Statistiques {
    pub voies: usize,
    pub intersections: usize,
    pub arcs: usize,
    pub restrictions_lues: usize,
    pub restrictions_resolues: usize,
    pub tuiles: usize,
    pub octets: u64,
    pub noeuds_sans_coordonnees: usize,
    pub composantes: usize,
    pub noeuds_hors_principale: usize,
    /// Arcs dont le score scénique est positif, et somme des scores (pour la moyenne).
    pub arcs_scenique_positifs: usize,
    pub somme_scenique: f64,
}

/// Construit le graphe et l'écrit dans `sortie/` ; `bornes` (sud, ouest, nord, est) limite l'emprise.
pub fn construire(ex: &Extrait, sortie: &Path, bornes: Option<[f64; 4]>, source: &str) -> Result<Statistiques> {
    construire_avec(ex, sortie, bornes, source, PoidsScenique::default())
}

/// Comme `construire`, avec les poids du score scénique (calibration, jalon J2).
pub fn construire_avec(ex: &Extrait, sortie: &Path, bornes: Option<[f64; 4]>, source: &str, poids: PoidsScenique) -> Result<Statistiques> {
    let mut st = Statistiques { voies: ex.voies.len(), restrictions_lues: ex.restrictions.len(), ..Default::default() };
    let dans_bornes = |lat: f64, lon: f64| bornes.is_none_or(|b| lat >= b[0] && lat <= b[2] && lon >= b[1] && lon <= b[3]);

    // 1. Intersections : nœud partagé par ≥ 2 voies, extrémité de voie, ou pivot de restriction.
    let mut usage: HashMap<i64, u8> = HashMap::with_capacity(ex.coords.len());
    for v in &ex.voies {
        for (i, n) in v.noeuds.iter().enumerate() {
            let e = usage.entry(*n).or_insert(0);
            *e = e.saturating_add(if i == 0 || i == v.noeuds.len() - 1 { 2 } else { 1 });
        }
    }
    for r in &ex.restrictions {
        if let Some(e) = usage.get_mut(&r.via_noeud) {
            *e = 2;
        }
    }

    // 2. Attribution des identifiants de nœuds par tuile.
    let mut noeud_id: HashMap<i64, NoeudId> = HashMap::new();
    let mut noeuds_par_tuile: HashMap<CleTuile, Vec<Point>> = HashMap::new();
    for (osm, &u) in &usage {
        if u < 2 {
            continue;
        }
        let Some(&(lat, lon)) = ex.coords.get(osm) else {
            st.noeuds_sans_coordonnees += 1;
            continue;
        };
        if !dans_bornes(lat, lon) {
            continue;
        }
        let cle = cle_tuile(lat, lon);
        let v = noeuds_par_tuile.entry(cle).or_default();
        noeud_id.insert(*osm, id_global(cle, v.len() as u32));
        v.push(Point::depuis_degres(lat, lon));
    }
    st.intersections = noeud_id.len();

    // 3. Arcs : chaque voie est découpée aux intersections.
    let agglos = Agglomerations::nouvelle(&ex.localites);
    let scenique = Scenique::nouveau(&ex.pois, &ex.routes_touristiques, poids);
    let mut bruts: Vec<ArcBrut> = Vec::new();
    // Pour les restrictions : (nœud pivot OSM, way) → arcs entrants / sortants provisoires.
    let pivots: std::collections::HashSet<i64> = ex.restrictions.iter().map(|r| r.via_noeud).collect();
    let mut entrants: HashMap<(i64, i64), usize> = HashMap::new();
    let mut sortants: HashMap<(i64, i64), usize> = HashMap::new();

    for v in &ex.voies {
        let mut debut = 0usize;
        let mut segment: u16 = 0;
        for i in 1..v.noeuds.len() {
            if usage.get(&v.noeuds[i]).copied().unwrap_or(0) < 2 && i != v.noeuds.len() - 1 {
                continue;
            }
            let (Some(&a), Some(&b)) = (noeud_id.get(&v.noeuds[debut]), noeud_id.get(&v.noeuds[i])) else {
                debut = i;
                segment += 1;
                continue;
            };
            let pts: Vec<Point> = v.noeuds[debut..=i]
                .iter()
                .filter_map(|n| ex.coords.get(n).map(|&(la, lo)| Point::depuis_degres(la, lo)))
                .collect();
            if pts.len() < 2 || (a == b && pts.len() < 3) {
                debut = i;
                segment += 1;
                continue;
            }
            let (longueur, virages) = scores::longueur_et_virages(&pts);
            let sin = scores::sinuosite(longueur, virages);
            let milieu = pts[pts.len() / 2];
            let agglo = agglos.score(milieu);
            let scen = scenique.score(&pts, v.id);
            if scen > 0.0 {
                st.arcs_scenique_positifs += 1;
            }
            st.somme_scenique += scen;
            let rond_point = v.drapeaux & drapeau::ROND_POINT != 0;
            let temps = scores::temps_ds(longueur, v.classe, v.vitesse_kmh, sin, rond_point);
            let geom: Vec<Point> = pts[1..pts.len() - 1].to_vec();
            let modele = Arc {
                cible: ID_NUL,
                inverse: ID_NUL,
                longueur_m: longueur.round().max(1.0) as u32,
                temps_ds: temps,
                classe: v.classe as u8,
                drapeaux: v.drapeaux,
                scores: [scores::en_u8(sin), scores::en_u8(scen), 0, scores::en_u8(agglo)],
                way_id: v.id,
                segment,
                geohash: geohash_u64(milieu.lat_deg(), milieu.lon_deg()),
                geom_debut: 0,
                geom_nb: geom.len().min(u16::MAX as usize) as u16,
            };
            let (osm_a, osm_b) = (v.noeuds[debut], v.noeuds[i]);
            let mut avant = None;
            let mut arriere = None;
            if v.sens >= 0 {
                avant = Some(bruts.len());
                bruts.push(ArcBrut { source: a, arc: Arc { cible: b, ..modele.clone() }, inverse_brut: usize::MAX, geom: geom.clone() });
            }
            if v.sens <= 0 {
                arriere = Some(bruts.len());
                let mut arc = Arc { cible: a, ..modele.clone() };
                // Double sens : l'arc retour partage la géométrie de l'aller (lue à l'envers).
                // Sens unique inverse (`oneway=-1`) : pas d'aller, on stocke la géométrie retournée.
                let geom_retour = if avant.is_some() {
                    arc.drapeaux |= drapeau::GEOM_INVERSEE;
                    Vec::new()
                } else {
                    geom.iter().rev().copied().collect()
                };
                bruts.push(ArcBrut { source: b, arc, inverse_brut: usize::MAX, geom: geom_retour });
            }
            if let (Some(x), Some(y)) = (avant, arriere) {
                bruts[x].inverse_brut = y;
                bruts[y].inverse_brut = x;
            }
            if pivots.contains(&osm_a) || pivots.contains(&osm_b) {
                if let Some(x) = avant {
                    sortants.insert((osm_a, v.id), x);
                    entrants.insert((osm_b, v.id), x);
                }
                if let Some(y) = arriere {
                    sortants.insert((osm_b, v.id), y);
                    entrants.insert((osm_a, v.id), y);
                }
            }
            debut = i;
            segment += 1;
        }
    }
    st.arcs = bruts.len();

    // 4. Rangement par tuile puis par nœud source : indices définitifs.
    let mut ordre: Vec<usize> = (0..bruts.len()).collect();
    ordre.sort_unstable_by_key(|&i| bruts[i].source);
    let mut id_final: Vec<u64> = vec![ID_NUL; bruts.len()];
    let mut par_tuile: HashMap<CleTuile, Vec<usize>> = HashMap::new();
    for &i in &ordre {
        let cle = raido_format::cle_de(bruts[i].source);
        let v = par_tuile.entry(cle).or_default();
        id_final[i] = id_global(cle, v.len() as u32);
        v.push(i);
    }

    // 4 bis. Composantes connexes (au sens faible) par union-find : la plus grande
    // est la composante principale, les autres sont des îlots.
    let mut dense: HashMap<NoeudId, u32> = HashMap::with_capacity(noeud_id.len());
    for (&cle, pts) in &noeuds_par_tuile {
        for i in 0..pts.len() {
            let id = id_global(cle, i as u32);
            let k = dense.len() as u32;
            dense.insert(id, k);
        }
    }
    let mut parent: Vec<u32> = (0..dense.len() as u32).collect();
    fn racine(parent: &mut [u32], mut x: u32) -> u32 {
        while parent[x as usize] != x {
            let p = parent[x as usize];
            parent[x as usize] = parent[p as usize];
            x = p;
        }
        x
    }
    for b in &bruts {
        let (x, y) = (dense[&b.source], dense[&b.arc.cible]);
        let (rx, ry) = (racine(&mut parent, x), racine(&mut parent, y));
        if rx != ry {
            parent[rx as usize] = ry;
        }
    }
    let mut taille: HashMap<u32, u32> = HashMap::new();
    for i in 0..parent.len() as u32 {
        *taille.entry(racine(&mut parent, i)).or_default() += 1;
    }
    let principale = taille.iter().max_by_key(|(_, &n)| n).map(|(&r, _)| r).unwrap_or(0);
    st.noeuds_hors_principale = parent.len() - taille.get(&principale).copied().unwrap_or(0) as usize;
    st.composantes = taille.len();
    let est_principal = |id: NoeudId, parent: &mut [u32]| racine(parent, dense[&id]) == principale;

    // 5. Restrictions résolues en identifiants d'arcs définitifs.
    let mut restrictions_par_tuile: HashMap<CleTuile, Vec<Restriction>> = HashMap::new();
    for r in &ex.restrictions {
        let (Some(&de), Some(&vers), Some(&via)) =
            (entrants.get(&(r.via_noeud, r.de_voie)), sortants.get(&(r.via_noeud, r.vers_voie)), noeud_id.get(&r.via_noeud))
        else {
            continue;
        };
        restrictions_par_tuile.entry(raido_format::cle_de(via)).or_default().push(Restriction {
            via: raido_format::local_de(via),
            de: id_final[de],
            vers: id_final[vers],
            seulement: r.seulement,
        });
        st.restrictions_resolues += 1;
    }

    // 6. Écriture des tuiles.
    fs::create_dir_all(sortie)?;
    let mut index_tuiles = Vec::new();
    let mut bornes_vues = [90.0f64, 180.0, -90.0, -180.0];
    let mut cles: Vec<CleTuile> = noeuds_par_tuile.keys().copied().collect();
    cles.sort_unstable();
    for cle in cles {
        let positions = &noeuds_par_tuile[&cle];
        let indices = par_tuile.remove(&cle).unwrap_or_default();
        let mut noeuds: Vec<Noeud> = Vec::with_capacity(positions.len() + 1);
        let mut arcs: Vec<Arc> = Vec::with_capacity(indices.len());
        let mut geometrie: Vec<Point> = Vec::new();
        let mut k = 0usize;
        for (local, p) in positions.iter().enumerate() {
            bornes_vues[0] = bornes_vues[0].min(p.lat_deg());
            bornes_vues[1] = bornes_vues[1].min(p.lon_deg());
            bornes_vues[2] = bornes_vues[2].max(p.lat_deg());
            bornes_vues[3] = bornes_vues[3].max(p.lon_deg());
            let drapeaux = if est_principal(id_global(cle, local as u32), &mut parent) { drapeau_noeud::PRINCIPAL } else { 0 };
            noeuds.push(Noeud { position: *p, premier_arc: arcs.len() as u32, drapeaux });
            while k < indices.len() && raido_format::local_de(bruts[indices[k]].source) == local as u32 {
                let b = &bruts[indices[k]];
                let mut arc = b.arc.clone();
                if b.inverse_brut != usize::MAX {
                    arc.inverse = id_final[b.inverse_brut];
                }
                if arc.drapeaux & drapeau::GEOM_INVERSEE != 0 {
                    // La géométrie est celle de l'arc aller : même tuile si la source
                    // de l'aller y est, sinon on la recopie à l'envers ici.
                    let aller = &bruts[b.inverse_brut];
                    if raido_format::cle_de(aller.source) == cle {
                        arc.geom_debut = u32::MAX; // résolu après la boucle
                    } else {
                        arc.drapeaux &= !drapeau::GEOM_INVERSEE;
                        arc.geom_debut = geometrie.len() as u32;
                        geometrie.extend(aller.geom.iter().rev());
                    }
                } else {
                    arc.geom_debut = geometrie.len() as u32;
                    geometrie.extend_from_slice(&b.geom);
                }
                arcs.push(arc);
                k += 1;
            }
        }
        noeuds.push(Noeud { position: Point { lat: 0, lon: 0 }, premier_arc: arcs.len() as u32, drapeaux: 0 });
        // Résolution différée : l'arc retour pointe sur la géométrie de l'aller (même tuile).
        for (j, &i) in indices.iter().enumerate() {
            if arcs[j].geom_debut == u32::MAX {
                let aller_final = id_final[bruts[i].inverse_brut];
                arcs[j].geom_debut = arcs[raido_format::local_de(aller_final) as usize].geom_debut;
            }
        }
        let mut restrictions = restrictions_par_tuile.remove(&cle).unwrap_or_default();
        restrictions.sort_by_key(|r| r.via);
        let tuile = Tuile { cle, noeuds, arcs, geometrie, restrictions };
        let octets = rkyv::to_bytes::<rkyv::rancor::Error>(&tuile).context("sérialisation rkyv")?;
        let chemin = sortie.join(chemin_tuile(cle));
        fs::create_dir_all(chemin.parent().unwrap())?;
        fs::write(&chemin, &octets)?;
        st.octets += octets.len() as u64;
        index_tuiles.push(TuileIndex {
            cle,
            fichier: chemin_tuile(cle),
            octets: octets.len() as u64,
            nb_noeuds: positions.len() as u32,
            nb_arcs: tuile.arcs.len() as u32,
        });
    }
    st.tuiles = index_tuiles.len();

    let index = Index {
        version_format: VERSION_FORMAT,
        rkyv: "0.8".to_string(),
        source: source.to_string(),
        construit_le: horodatage(),
        bornes: bornes_vues,
        tuiles: index_tuiles,
        nb_noeuds: st.intersections as u64,
        nb_arcs: st.arcs as u64,
    };
    fs::write(sortie.join("index.json"), serde_json::to_string_pretty(&index)?)?;
    Ok(st)
}

fn horodatage() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Date civile UTC sans dépendance : algorithme des jours juliens.
    let jours = (s / 86_400) as i64;
    let (h, m) = ((s % 86_400) / 3_600, (s % 3_600) / 60);
    let z = jours + 719_468;
    let ere = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + ere * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}Z")
}
