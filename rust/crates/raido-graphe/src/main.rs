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
    },
}

fn main() -> Result<()> {
    match Cli::parse().commande {
        Commande::Construire { pbf, sortie, bornes } => {
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
            let t1 = Instant::now();
            let source = pbf.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let st = raido_graphe::construire::construire(&ex, &sortie, bornes, &source)?;
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
            Ok(())
        }
    }
}
