// SPDX-License-Identifier: EUPL-1.2
//! Chargement des tuiles de graphe et accès zéro copie aux nœuds et arcs.
//!
//! Une requête commence par `precharger` l'emprise dont elle a besoin ; la
//! recherche travaille ensuite sur `&Graphe` sans aucune allocation de tuile.
//! Le cache survit entre requêtes (dans la Lambda : entre invocations).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use raido_format::{
    bornes_tuile, chemin_tuile, cle_de, cle_tuile, local_de, ArcId, ArchivedArc, ArchivedNoeud, ArchivedRestriction,
    ArchivedTuile, CleTuile, Index, NoeudId, Point, VERSION_FORMAT,
};
use rkyv::util::AlignedVec;

pub struct Graphe {
    dossier: PathBuf,
    pub index: Index,
    tuiles: HashMap<CleTuile, AlignedVec>,
    pub octets_charges: u64,
}

impl Graphe {
    pub fn ouvrir(dossier: &Path) -> Result<Graphe> {
        let index: Index = serde_json::from_slice(&fs::read(dossier.join("index.json")).context("index.json")?)?;
        if index.version_format != VERSION_FORMAT {
            return Err(anyhow!("format {} attendu, index en {}", VERSION_FORMAT, index.version_format));
        }
        Ok(Graphe { dossier: dossier.to_path_buf(), index, tuiles: HashMap::new(), octets_charges: 0 })
    }

    pub fn existe(&self, cle: CleTuile) -> bool {
        self.index.tuiles.iter().any(|t| t.cle == cle)
    }

    /// Charge (si besoin) la tuile ; la validation rkyv n'est faite qu'au chargement.
    pub fn charger(&mut self, cle: CleTuile) -> Result<bool> {
        if self.tuiles.contains_key(&cle) {
            return Ok(true);
        }
        if !self.existe(cle) {
            return Ok(false);
        }
        let octets = fs::read(self.dossier.join(chemin_tuile(cle))).with_context(|| format!("tuile {cle:#x}"))?;
        let mut v = AlignedVec::<16>::with_capacity(octets.len());
        v.extend_from_slice(&octets);
        rkyv::access::<ArchivedTuile, rkyv::rancor::Error>(&v).with_context(|| format!("tuile {cle:#x} invalide"))?;
        self.octets_charges += v.len() as u64;
        self.tuiles.insert(cle, v);
        Ok(true)
    }

    /// Charge toutes les tuiles existantes qui touchent l'emprise (sud, ouest, nord, est).
    /// Renvoie le nombre de tuiles nouvellement chargées.
    pub fn precharger(&mut self, bornes: [f64; 4]) -> Result<usize> {
        let cles: Vec<CleTuile> = self
            .index
            .tuiles
            .iter()
            .map(|t| t.cle)
            .filter(|&c| {
                let (s, o, n, e) = bornes_tuile(c);
                n >= bornes[0] && s <= bornes[2] && e >= bornes[1] && o <= bornes[3]
            })
            .collect();
        let mut nouvelles = 0;
        for c in cles {
            if !self.tuiles.contains_key(&c) && self.charger(c)? {
                nouvelles += 1;
            }
        }
        Ok(nouvelles)
    }

    pub fn nb_tuiles_chargees(&self) -> usize {
        self.tuiles.len()
    }

    #[inline]
    pub fn tuile(&self, cle: CleTuile) -> Option<&ArchivedTuile> {
        // La validation a eu lieu dans `charger` : l'accès direct est sûr sur ces octets.
        self.tuiles.get(&cle).map(|v| unsafe { rkyv::access_unchecked::<ArchivedTuile>(v) })
    }

    #[inline]
    pub fn noeud(&self, id: NoeudId) -> Option<&ArchivedNoeud> {
        self.tuile(cle_de(id)).and_then(|t| t.noeuds.get(local_de(id) as usize))
    }

    #[inline]
    pub fn position(&self, id: NoeudId) -> Option<Point> {
        self.noeud(id).map(|n| Point { lat: n.position.lat.to_native(), lon: n.position.lon.to_native() })
    }

    #[inline]
    pub fn arc(&self, id: ArcId) -> Option<&ArchivedArc> {
        self.tuile(cle_de(id)).and_then(|t| t.arcs.get(local_de(id) as usize))
    }

    /// Arcs sortants d'un nœud, avec leur identifiant global.
    pub fn sortants(&self, id: NoeudId) -> impl Iterator<Item = (ArcId, &ArchivedArc)> + '_ {
        let cle = cle_de(id);
        let local = local_de(id) as usize;
        let tranche = self.tuile(cle).and_then(|t| {
            let d = t.noeuds.get(local)?.premier_arc.to_native() as usize;
            let f = t.noeuds.get(local + 1)?.premier_arc.to_native() as usize;
            Some((t, d, f))
        });
        tranche
            .into_iter()
            .flat_map(move |(t, d, f)| (d..f).map(move |i| (raido_format::id_global(cle, i as u32), &t.arcs[i])))
    }

    pub fn degre_sortant(&self, id: NoeudId) -> usize {
        self.sortants(id).count()
    }

    /// Restrictions dont le pivot est ce nœud.
    pub fn restrictions(&self, id: NoeudId) -> &[ArchivedRestriction] {
        let Some(t) = self.tuile(cle_de(id)) else { return &[] };
        let via = local_de(id);
        let r = &t.restrictions[..];
        let debut = r.partition_point(|x| x.via.to_native() < via);
        let fin = r.partition_point(|x| x.via.to_native() <= via);
        &r[debut..fin]
    }

    /// Points intermédiaires d'un arc, dans le sens de parcours.
    pub fn geometrie(&self, id: ArcId) -> Vec<Point> {
        let Some(t) = self.tuile(cle_de(id)) else { return Vec::new() };
        let a = &t.arcs[local_de(id) as usize];
        let d = a.geom_debut.to_native() as usize;
        let n = a.geom_nb.to_native() as usize;
        let pts: Vec<Point> = t.geometrie[d..d + n].iter().map(|p| Point { lat: p.lat.to_native(), lon: p.lon.to_native() }).collect();
        if a.drapeaux & raido_format::drapeau::GEOM_INVERSEE != 0 {
            pts.into_iter().rev().collect()
        } else {
            pts
        }
    }

    /// Nœud le plus proche d'un point parmi les tuiles chargées voisines (rayon ≤ `max_m`).
    pub fn noeud_proche(&self, lat: f64, lon: f64, max_m: f64) -> Option<(NoeudId, f64)> {
        self.noeuds_proches(lat, lon, max_m, 1).into_iter().next()
    }

    /// Les `k` nœuds les plus proches, par distance croissante.
    pub fn noeuds_proches(&self, lat: f64, lon: f64, max_m: f64, k: usize) -> Vec<(NoeudId, f64)> {
        let p = Point::depuis_degres(lat, lon);
        let (cx, cy) = {
            let (x, y) = raido_format::xy_de_cle(cle_tuile(lat, lon));
            (x as i32, y as i32)
        };
        let n_tuiles = 1i32 << raido_format::ZOOM;
        let mut meilleurs: Vec<(NoeudId, f64)> = Vec::with_capacity(k + 1);
        // Pré-filtre rectangulaire en unités de 1e-7 degré : 1 m ≈ 90 unités en latitude.
        let dlat_u = (max_m * 1e7 / 111_000.0) as i64;
        let dlon_u = (dlat_u as f64 / lat.to_radians().cos().max(0.2)) as i64;
        for dx in -1..=1 {
            for dy in -1..=1 {
                let (x, y) = (cx + dx, cy + dy);
                if !(0..n_tuiles).contains(&x) || !(0..n_tuiles).contains(&y) {
                    continue;
                }
                let cle = raido_format::cle_xy(x as u32, y as u32);
                let Some(t) = self.tuile(cle) else { continue };
                // Les tuiles voisines ne servent qu'aux points près d'un bord.
                if (dx != 0 || dy != 0) && !proche_du_bord(cle, lat, lon, max_m) {
                    continue;
                }
                let n = t.noeuds.len().saturating_sub(1);
                for (i, nd) in t.noeuds[..n].iter().enumerate() {
                    let q = Point { lat: nd.position.lat.to_native(), lon: nd.position.lon.to_native() };
                    if (q.lat as i64 - p.lat as i64).abs() > dlat_u || (q.lon as i64 - p.lon as i64).abs() > dlon_u {
                        continue;
                    }
                    let d = raido_format::haversine_m(p, q);
                    if d > max_m || (meilleurs.len() == k && d >= meilleurs[k - 1].1) {
                        continue;
                    }
                    let pos = meilleurs.partition_point(|&(_, m)| m <= d);
                    meilleurs.insert(pos, (raido_format::id_global(cle, i as u32), d));
                    meilleurs.truncate(k);
                }
            }
        }
        meilleurs
    }
}

fn proche_du_bord(cle_voisine: CleTuile, lat: f64, lon: f64, max_m: f64) -> bool {
    let (s, o, n, e) = bornes_tuile(cle_voisine);
    let dlat = max_m / 111_000.0;
    let dlon = max_m / (111_000.0 * lat.to_radians().cos());
    lat + dlat >= s && lat - dlat <= n && lon + dlon >= o && lon - dlon <= e
}
