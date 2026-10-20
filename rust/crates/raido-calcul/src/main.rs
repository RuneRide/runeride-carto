// SPDX-License-Identifier: EUPL-1.2
//! Ligne de commande du moteur Raido : `trajet`, `balade`, `banc`.
//! Sert au PoC J1 ; la Lambda (J3) réutilisera la bibliothèque.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{anyhow, bail, Result};
use clap::{Args, Parser, Subcommand};
use raido_calcul::boucle::{self, ParametresBoucle};
use raido_calcul::charge::Graphe;
use raido_calcul::cout::Preferences;
use raido_calcul::{apparier, astar, geo};

#[derive(Parser)]
#[command(about = "Moteur de balade Raido (PoC J1)")]
struct Cli {
    /// Dossier du graphe (contient index.json).
    #[arg(long, short = 'g')]
    graphe: PathBuf,
    #[command(subcommand)]
    commande: Commande,
}

#[derive(Args, Clone)]
struct Prefs {
    #[arg(long, default_value_t = 3)]
    sinuosite: u8,
    #[arg(long, default_value_t = 3)]
    scenique: u8,
    #[arg(long, default_value_t = 2)]
    motard: u8,
    #[arg(long, default_value_t = 3)]
    agglo: u8,
    #[arg(long, default_value_t = 0)]
    decouverte: u8,
    /// Autorise les autoroutes et péages (évités par défaut).
    #[arg(long)]
    autoroutes: bool,
    /// Autorise les routes non revêtues.
    #[arg(long)]
    non_revetu: bool,
    /// Itinéraire le plus rapide : tous les curseurs à zéro.
    #[arg(long)]
    rapide: bool,
    /// Pondération de l'heuristique A* (1 = exact, 1,5 = plus rapide, un peu moins optimal).
    #[arg(long, default_value_t = 1.0)]
    epsilon: f64,
    /// Écrit la trace en GeoJSON dans ce fichier.
    #[arg(long)]
    geojson: Option<PathBuf>,
}

impl Prefs {
    fn preferences(&self) -> Preferences {
        if self.rapide {
            return Preferences { eviter_autoroutes: !self.autoroutes, eviter_peages: !self.autoroutes, eviter_non_revetu: !self.non_revetu, ..Default::default() };
        }
        Preferences {
            sinuosite: self.sinuosite,
            scenique: self.scenique,
            motard: self.motard,
            agglo: self.agglo,
            decouverte: self.decouverte,
            eviter_autoroutes: !self.autoroutes,
            eviter_peages: !self.autoroutes,
            eviter_non_revetu: !self.non_revetu,
            eviter_tunnels: false,
            troncons_connus: HashSet::new(),
            vitesses: Vec::new(),
        }
    }
}

#[derive(Subcommand)]
enum Commande {
    /// Trajet d'un point à un autre (`lat,lon`).
    Trajet {
        de: String,
        a: String,
        #[command(flatten)]
        prefs: Prefs,
    },
    /// Boucle depuis un point (`lat,lon`) d'une distance donnée.
    Balade {
        depart: String,
        #[arg(long, default_value_t = 120.0)]
        distance_km: f64,
        #[arg(long, default_value_t = 3)]
        variantes: usize,
        #[command(flatten)]
        prefs: Prefs,
    },
    /// Banc d'essai : N boucles depuis des départs tirés au sort dans l'emprise du graphe.
    /// Banc de l'appariement : boucles calculées sur le graphe, bruitées comme un GPS, puis rattachées au réseau ;
    /// compare les tronçons retrouvés à ceux réellement parcourus (rappel pondéré par la longueur, précision).
    Apparier {
        #[arg(long, default_value_t = 20)]
        n: usize,
        #[arg(long, default_value_t = 60.0)]
        distance_km: f64,
        #[arg(long, default_value_t = 1)]
        graine: u64,
        /// Écart-type du bruit gaussien ajouté à chaque point, en mètres.
        #[arg(long, default_value_t = 8.0)]
        bruit_m: f64,
    },
    Banc {
        #[arg(long, default_value_t = 20)]
        n: usize,
        #[arg(long, default_value_t = 120.0)]
        distance_km: f64,
        #[arg(long, default_value_t = 1)]
        graine: u64,
        /// Vide le cache de tuiles entre deux départs (mesure « à froid »).
        #[arg(long)]
        froid: bool,
        #[arg(long, default_value_t = 1.0)]
        epsilon: f64,
        /// Nombre de fils de calcul (défaut : tous les cœurs ; 1 pour simuler une Lambda de 2 Go).
        #[arg(long)]
        fils: Option<usize>,
    },
}

fn point(s: &str) -> Result<(f64, f64)> {
    let v: Vec<f64> = s.split(',').map(|x| x.trim().parse()).collect::<Result<_, _>>()?;
    if v.len() != 2 {
        bail!("attendu lat,lon : {s}");
    }
    Ok((v[0], v[1]))
}

fn geojson(chemin: &PathBuf, traces: &[(String, Vec<(f64, f64)>)]) -> Result<()> {
    let features: Vec<serde_json::Value> = traces
        .iter()
        .map(|(nom, pts)| {
            serde_json::json!({
                "type": "Feature",
                "properties": {"nom": nom},
                "geometry": {"type": "LineString", "coordinates": pts.iter().map(|(la, lo)| [lo, la]).collect::<Vec<_>>()}
            })
        })
        .collect();
    std::fs::write(chemin, serde_json::to_string(&serde_json::json!({"type": "FeatureCollection", "features": features}))?)?;
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut g = Graphe::ouvrir(&cli.graphe)?;
    match cli.commande {
        Commande::Trajet { de, a, prefs } => {
            let (d, ar) = (point(&de)?, point(&a)?);
            let t0 = Instant::now();
            let marge = raido_format::haversine_m(raido_format::Point::depuis_degres(d.0, d.1), raido_format::Point::depuis_degres(ar.0, ar.1)) * 0.3 + 10_000.0;
            let n = g.precharger(geo::emprise_entre(d, ar, marge))?;
            let t_charge = t0.elapsed();
            let p = prefs.preferences();
            let nd = boucle::noeud_accroche(&g, &p, d.0, d.1, 2_000.0).ok_or_else(|| anyhow!("aucune route près du départ"))?;
            let na = boucle::noeud_accroche(&g, &p, ar.0, ar.1, 2_000.0).ok_or_else(|| anyhow!("aucune route près de l'arrivée"))?;
            let t1 = Instant::now();
            let ch = astar::chemin(&g, &p, nd, na, &HashSet::new(), prefs.epsilon).ok_or_else(|| anyhow!("aucun chemin"))?;
            println!(
                "{:.1} km, {} min, coût {:.0}, {} arcs, {} états explorés ; tuiles {} ({} chargées, {:.1} Mo) en {:.2} s, calcul {:.3} s",
                ch.longueur_m as f64 / 1000.0,
                ch.temps_ds / 600,
                ch.cout,
                ch.arcs.len(),
                ch.noeuds_explores,
                g.nb_tuiles_chargees(),
                n,
                g.octets_charges as f64 / 1e6,
                t_charge.as_secs_f64(),
                t1.elapsed().as_secs_f64()
            );
            if let Some(f) = &prefs.geojson {
                geojson(f, &[("trajet".into(), astar::trace(&g, &ch.arcs, nd))])?;
            }
        }
        Commande::Balade { depart, distance_km, variantes, prefs } => {
            let d = point(&depart)?;
            let dist = distance_km * 1000.0;
            let t0 = Instant::now();
            let n = g.precharger(boucle::emprise(d.0, d.1, dist))?;
            let t_charge = t0.elapsed();
            let p = prefs.preferences();
            let nd = boucle::noeud_accroche(&g, &p, d.0, d.1, 2_000.0).ok_or_else(|| anyhow!("aucune route près du départ"))?;
            let t1 = Instant::now();
            let mut pb = ParametresBoucle::pour(dist);
            pb.epsilon = prefs.epsilon;
            let cands = boucle::balade(&g, &p, nd, &pb)?;
            let dt = t1.elapsed();
            println!(
                "{} candidats en {:.2} s ; tuiles {} ({} chargées, {:.1} Mo en {:.2} s)",
                cands.len(),
                dt.as_secs_f64(),
                g.nb_tuiles_chargees(),
                n,
                g.octets_charges as f64 / 1e6,
                t_charge.as_secs_f64()
            );
            let mut traces = Vec::new();
            for (i, c) in cands.iter().take(variantes).enumerate() {
                println!(
                    "  #{} cap {:>3}° : {:.1} km, {} min, recouvrement {:.0} %, sinuosité {:.2}, agglo {:.2}, note {:.2}, {} états",
                    i + 1,
                    c.cap_deg,
                    c.longueur_m as f64 / 1000.0,
                    c.temps_ds / 600,
                    c.recouvrement * 100.0,
                    c.sinuosite,
                    c.agglo,
                    c.note,
                    c.noeuds_explores
                );
                traces.push((format!("variante {}", i + 1), astar::trace(&g, &c.arcs, nd)));
            }
            if let Some(f) = &prefs.geojson {
                geojson(f, &traces)?;
            }
        }
        Commande::Apparier { n, distance_km, graine, bruit_m } => {
            let b = g.index.bornes;
            let mut alea = geo::Alea::nouveau(graine);
            let p = Preferences::balade();
            let dist = distance_km * 1000.0;
            let (mut rappels, mut precisions, mut couvertures, mut temps) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut essais = 0;
            while rappels.len() < n && essais < n * 6 {
                essais += 1;
                let lat = b[0] + (b[2] - b[0]) * (0.15 + 0.7 * alea.reel());
                let lon = b[1] + (b[3] - b[1]) * (0.15 + 0.7 * alea.reel());
                g.precharger(boucle::emprise(lat, lon, dist))?;
                let Some(nd) = boucle::noeud_accroche(&g, &p, lat, lon, 3_000.0) else { continue };
                let mut pb = ParametresBoucle::pour(dist);
                pb.epsilon = 1.0;
                let Some(c) = boucle::balade(&g, &p, nd, &pb)?.into_iter().next() else { continue };
                // Vérité : tronçons (way, segment) des arcs de la boucle, avec leur longueur.
                let mut verite: std::collections::HashMap<(i64, u16), u64> = std::collections::HashMap::new();
                for &id in &c.arcs {
                    if let Some(a) = g.arc(id) {
                        *verite.entry((a.way_id.to_native(), a.segment.to_native())).or_default() += a.longueur_m.to_native() as u64;
                    }
                }
                // Trace bruitée (Box-Muller), comme un GPS : même chemin, positions décalées.
                let bruit = |alea: &mut geo::Alea| {
                    let (u1, u2) = (alea.reel().max(1e-12), alea.reel());
                    let r = (-2.0 * u1.ln()).sqrt() * bruit_m;
                    (r * (2.0 * std::f64::consts::PI * u2).cos(), r * (2.0 * std::f64::consts::PI * u2).sin())
                };
                let trace: Vec<(f64, f64)> = astar::trace(&g, &c.arcs, nd)
                    .into_iter()
                    .map(|(la, lo)| {
                        let (dx, dy) = bruit(&mut alea);
                        (la + dy / 110_540.0, lo + dx / (111_320.0 * la.to_radians().cos()))
                    })
                    .collect();
                let t0 = Instant::now();
                let r = apparier::apparier(&mut g, &trace)?;
                temps.push(t0.elapsed().as_secs_f64());
                let trouves: std::collections::HashSet<(i64, u16)> = r.troncons.iter().copied().collect();
                let total_m: u64 = verite.values().sum();
                let juste_m: u64 = verite.iter().filter(|(k, _)| trouves.contains(k)).map(|(_, l)| l).sum();
                let bons = trouves.iter().filter(|k| verite.contains_key(k)).count();
                rappels.push(juste_m as f64 / total_m.max(1) as f64);
                precisions.push(bons as f64 / trouves.len().max(1) as f64);
                couvertures.push(r.couverture);
                println!(
                    "{lat:.4},{lon:.4} : {:.1} km, {} points, rappel {:.1} %, précision {:.1} %, couverture {:.0} %, {:.2} s",
                    c.longueur_m as f64 / 1000.0,
                    r.points,
                    rappels.last().unwrap() * 100.0,
                    precisions.last().unwrap() * 100.0,
                    r.couverture * 100.0,
                    temps.last().unwrap()
                );
            }
            if rappels.is_empty() {
                bail!("aucune boucle calculée");
            }
            let moy = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            let min = |v: &[f64]| v.iter().cloned().fold(f64::INFINITY, f64::min);
            println!(
                "\n{} boucles de {:.0} km, bruit {:.0} m : rappel moyen {:.1} % (min {:.1} %), précision moyenne {:.1} % (min {:.1} %), couverture moyenne {:.1} %, {:.2} s par trace",
                rappels.len(), distance_km, bruit_m, moy(&rappels) * 100.0, min(&rappels) * 100.0, moy(&precisions) * 100.0, min(&precisions) * 100.0, moy(&couvertures) * 100.0, moy(&temps)
            );
        }
        Commande::Banc { n, distance_km, graine, froid, epsilon, fils } => {
            if let Some(f) = fils {
                rayon::ThreadPoolBuilder::new().num_threads(f).build_global()?;
            }
            let b = g.index.bornes;
            let mut pb = ParametresBoucle::pour(distance_km * 1000.0);
            pb.epsilon = epsilon;
            let mut alea = geo::Alea::nouveau(graine);
            let mut temps = Vec::new();
            let mut echecs = 0;
            let mut octets_froid = Vec::new();
            let mut ecarts = Vec::new();
            let mut vitesses = Vec::new();
            let p = Preferences::balade();
            let dist = distance_km * 1000.0;
            let mut essais = 0;
            while temps.len() < n && essais < n * 4 {
                essais += 1;
                // On reste à 15 % des bords pour que la boucle tienne dans l'extrait.
                let lat = b[0] + (b[2] - b[0]) * (0.15 + 0.7 * alea.reel());
                let lon = b[1] + (b[3] - b[1]) * (0.15 + 0.7 * alea.reel());
                if froid {
                    g = Graphe::ouvrir(&cli.graphe)?;
                }
                let t0 = Instant::now();
                g.precharger(boucle::emprise(lat, lon, dist))?;
                let Some(nd) = boucle::noeud_accroche(&g, &p, lat, lon, 3_000.0) else { continue };
                let cands = boucle::balade(&g, &p, nd, &pb)?;
                let dt = t0.elapsed().as_secs_f64();
                if froid {
                    octets_froid.push(g.octets_charges);
                }
                match cands.first() {
                    Some(c) => {
                        temps.push(dt);
                        ecarts.push((c.longueur_m as f64 - dist) / dist);
                        vitesses.push(c.longueur_m as f64 / 1000.0 / (c.temps_ds as f64 / 36_000.0));
                        println!(
                            "{lat:.4},{lon:.4} : {:.2} s, {} candidats, meilleur {:.1} km recouvrement {:.0} % sinuosité {:.2}",
                            dt,
                            cands.len(),
                            c.longueur_m as f64 / 1000.0,
                            c.recouvrement * 100.0,
                            c.sinuosite
                        );
                    }
                    None => {
                        echecs += 1;
                        println!("{lat:.4},{lon:.4} : {:.2} s, aucune boucle", dt);
                    }
                }
            }
            temps.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if temps.is_empty() {
                bail!("aucune boucle calculée");
            }
            let pct = |q: f64| temps[((temps.len() as f64 - 1.0) * q).round() as usize];
            let n_ok = ecarts.len() as f64;
            println!(
                "\n{} boucles de {:.0} km, {} échecs : médiane {:.2} s, p95 {:.2} s, max {:.2} s ; écart de distance moyen {:+.1} % (|écart| {:.1} %), vitesse moyenne {:.0} km/h{}",
                temps.len(),
                distance_km,
                echecs,
                pct(0.5),
                pct(0.95),
                temps[temps.len() - 1],
                ecarts.iter().sum::<f64>() / n_ok * 100.0,
                ecarts.iter().map(|e| e.abs()).sum::<f64>() / n_ok * 100.0,
                vitesses.iter().sum::<f64>() / n_ok,
                if froid {
                    format!(
                        " ; octets chargés à froid : médiane {:.1} Mo",
                        octets_froid.iter().sum::<u64>() as f64 / octets_froid.len().max(1) as f64 / 1e6
                    )
                } else {
                    String::new()
                }
            );
        }
    }
    Ok(())
}
