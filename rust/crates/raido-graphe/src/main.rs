// SPDX-License-Identifier: EUPL-1.2
//! `raido-graphe construire FICHIER.osm.pbf DOSSIER [--bornes sud,ouest,nord,est]`
//! Construit les tuiles de graphe Raido ; tout se passe en local.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Pipeline du graphe routier Raido (données © OpenStreetMap, ODbL)")]
struct Cli {
    #[command(subcommand)]
    commande: Commande,
}

#[derive(Subcommand)]
enum Commande {
    /// Lit un extrait .osm.pbf et écrit les tuiles rkyv + index.json.
    Construire {
        pbf: PathBuf,
        sortie: PathBuf,
        /// Emprise `sud,ouest,nord,est` en degrés (défaut : tout l'extrait).
        #[arg(long)]
        bornes: Option<String>,
        /// Poids du score scénique (JSON, champs facultatifs, voir `PoidsScenique`) : calibration sans recompiler.
        #[arg(long)]
        poids: Option<PathBuf>,
        /// Écrit en GeoJSON les cols, panoramas et sommets nommés trouvés (diagnostic, vérification à la main).
        #[arg(long)]
        dump_poi: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().commande {
        Commande::Construire { pbf, sortie, bornes, poids, dump_poi } => {
            let bornes = match bornes {
                Some(b) => {
                    let v: Vec<f64> = b.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?;
                    if v.len() != 4 {
                        bail!("--bornes attend sud,ouest,nord,est");
                    }
                    Some([v[0], v[1], v[2], v[3]])
                }
                None => None,
            };
            let t0 = Instant::now();
            let ex = raido_graphe::osm::lire(&pbf)?;
            let t_lecture = t0.elapsed();
            eprintln!(
                "lecture : {} voies, {} restrictions, {} localités, {} nœuds en {:.1} s",
                ex.voies.len(),
                ex.restrictions.len(),
                ex.localites.len(),
                ex.coords.len(),
                t_lecture.as_secs_f64()
            );
            let poids: raido_graphe::scores::PoidsScenique = match &poids {
                Some(f) => serde_json::from_str(&std::fs::read_to_string(f)?).map_err(|e| anyhow::anyhow!("poids {} : {e}", f.display()))?,
                None => Default::default(),
            };
            eprintln!("scénique : {} points et échantillons de zones, {} ways de routes touristiques", ex.pois.len(), ex.routes_touristiques.len());
            if let Some(f) = &dump_poi {
                use raido_graphe::osm::Genre;
                let traits: Vec<serde_json::Value> = ex
                    .pois
                    .iter()
                    .filter(|p| p.nom.is_some() && matches!(p.genre, Genre::Col | Genre::Panorama | Genre::Sommet))
                    .map(|p| serde_json::json!({"type": "Feature", "geometry": {"type": "Point", "coordinates": [p.lon, p.lat]}, "properties": {"genre": format!("{:?}", p.genre), "nom": p.nom}}))
                    .collect();
                std::fs::write(f, serde_json::to_string(&serde_json::json!({"type": "FeatureCollection", "features": traits}))?)?;
                eprintln!("{} points nommés écrits dans {}", traits.len(), f.display());
            }
            let t1 = Instant::now();
            let source = pbf.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let st = raido_graphe::construire::construire_avec(&ex, &sortie, bornes, &source, poids)?;
            eprintln!(
                "graphe : {} intersections, {} arcs, {} tuiles, {:.1} Mo, restrictions {}/{} résolues, {} nœuds sans coordonnées, {} composantes ({} nœuds hors principale), en {:.1} s",
                st.intersections,
                st.arcs,
                st.tuiles,
                st.octets as f64 / 1e6,
                st.restrictions_resolues,
                st.restrictions_lues,
                st.noeuds_sans_coordonnees,
                st.composantes,
                st.noeuds_hors_principale,
                t1.elapsed().as_secs_f64()
            );
            eprintln!(
                "scénique : {:.1} % des arcs ont un score positif, moyenne {:.3}",
                100.0 * st.arcs_scenique_positifs as f64 / st.arcs.max(1) as f64,
                st.somme_scenique / st.arcs.max(1) as f64
            );
            Ok(())
        }
    }
}
