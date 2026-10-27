// SPDX-License-Identifier: EUPL-1.2
//! Relief : altitude (Copernicus GLO-30 converti par `rust/tools/convertir-mnt.py`), dénivelé d'un tronçon et
//! terme « route de montagne » du score scénique.
//!
//! Chaque tuile est une grille 3600 × 3600 d'entiers 16 bits (mètres), nommée `N45E005.i16`. Une tuile absente
//! donne `None` : le tronçon n'a alors ni dénivelé ni bonus de relief (jamais une altitude inventée).

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use raido_format::{haversine_m, Point};

const N: usize = 3600;
const INCONNU: i16 = i16::MIN;
/// Variation minimale (m) comptée comme montée ou descente : en dessous, c'est du bruit du modèle numérique.
pub const SEUIL_BRUIT_M: f64 = 3.0;
/// Pas (m) d'échantillonnage de l'altitude le long d'un tronçon (le pixel fait 22 à 30 m).
const PAS_M: f64 = 30.0;
/// Rayon (m) de l'anneau de comparaison pour la « dominance » d'une route sur son voisinage.
const RAYON_ANNEAU_M: f64 = 1_500.0;

pub struct Mnt {
    tuiles: HashMap<(i32, i32), Vec<i16>>,
}

fn nom_tuile(la: i32, lo: i32) -> String {
    format!("{}{:02}{}{:03}.i16", if la >= 0 { 'N' } else { 'S' }, la.abs(), if lo >= 0 { 'E' } else { 'W' }, lo.abs())
}

impl Mnt {
    /// Charge les tuiles du dossier qui recouvrent `bornes` (sud, ouest, nord, est), marge d'un degré comprise.
    pub fn charger(dossier: &Path, bornes: [f64; 4]) -> Result<Mnt> {
        let mut tuiles = HashMap::new();
        for la in (bornes[0].floor() as i32 - 1)..=(bornes[2].floor() as i32 + 1) {
            for lo in (bornes[1].floor() as i32 - 1)..=(bornes[3].floor() as i32 + 1) {
                let f = dossier.join(nom_tuile(la, lo));
                if !f.exists() {
                    continue;
                }
                let o = fs::read(&f).with_context(|| f.display().to_string())?;
                if o.len() != N * N * 2 {
                    bail!("{} : {} octets, {} attendus", f.display(), o.len(), N * N * 2);
                }
                tuiles.insert((la, lo), o.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect());
            }
        }
        if tuiles.is_empty() {
            bail!("aucune tuile de relief dans {} pour l'emprise demandée", dossier.display());
        }
        Ok(Mnt { tuiles })
    }

    pub fn nb_tuiles(&self) -> usize {
        self.tuiles.len()
    }

    fn pixel(&self, la: i32, lo: i32, r: usize, c: usize) -> Option<f64> {
        let v = *self.tuiles.get(&(la, lo))?.get(r * N + c)?;
        (v != INCONNU).then_some(v as f64)
    }

    /// Altitude (m) par interpolation bilinéaire entre les centres de pixels ; `None` hors des tuiles chargées ou sur un trou.
    pub fn altitude(&self, lat: f64, lon: f64) -> Option<f64> {
        // Coordonnées en pixels depuis le coin nord-ouest de la tuile, centres de pixels à +0,5.
        let (la0, lo0) = (lat.floor() as i32, lon.floor() as i32);
        let y = (la0 as f64 + 1.0 - lat) * N as f64 - 0.5;
        let x = (lon - lo0 as f64) * N as f64 - 0.5;
        let (y0, x0) = (y.floor(), x.floor());
        let (fy, fx) = (y - y0, x - x0);
        let mut acc = 0.0;
        for (dy, wy) in [(0i64, 1.0 - fy), (1, fy)] {
            for (dx, wx) in [(0i64, 1.0 - fx), (1, fx)] {
                let (mut r, mut c) = (y0 as i64 + dy, x0 as i64 + dx);
                let (mut la, mut lo) = (la0, lo0);
                // Au-delà du bord d'une tuile, on lit la voisine.
                if r < 0 {
                    r += N as i64;
                    la += 1;
                } else if r >= N as i64 {
                    r -= N as i64;
                    la -= 1;
                }
                if c < 0 {
                    c += N as i64;
                    lo -= 1;
                } else if c >= N as i64 {
                    c -= N as i64;
                    lo += 1;
                }
                acc += wy * wx * self.pixel(la, lo, r as usize, c as usize)?;
            }
        }
        Some(acc)
    }

    /// Altitudes le long d'une polyligne, un échantillon tous les `PAS_M` environ (extrémités comprises).
    fn profil(&self, pts: &[Point]) -> Option<Vec<f64>> {
        let mut alt = vec![self.altitude(pts.first()?.lat_deg(), pts.first()?.lon_deg())?];
        for w in pts.windows(2) {
            let l = haversine_m(w[0], w[1]);
            let n = (l / PAS_M).ceil().max(1.0) as usize;
            for k in 1..=n {
                let t = k as f64 / n as f64;
                let la = w[0].lat_deg() + (w[1].lat_deg() - w[0].lat_deg()) * t;
                let lo = w[0].lon_deg() + (w[1].lon_deg() - w[0].lon_deg()) * t;
                alt.push(self.altitude(la, lo)?);
            }
        }
        Some(alt)
    }

    /// Montée et descente cumulées (m) d'une polyligne parcourue dans le sens des points ; `None` sans relief.
    /// Une variation n'est comptée qu'une fois qu'elle dépasse `SEUIL_BRUIT_M` depuis le dernier point retenu.
    pub fn denivele(&self, pts: &[Point]) -> Option<(u32, u32)> {
        Some(cumuler(&self.profil(pts)?))
    }

    /// Terme 0..1 « route de montagne » : moyenne, sur des échantillons de `pas_m`, de la moitié de l'altitude relative
    /// à 600 m (0 à 600 m, 1 à 2 000 m) et de la moitié de la dominance (hauteur au-dessus de la moyenne de huit points
    /// à 1,5 km : 0 à niveau, 1 à +250 m — versant ou crête avec dégagement). **Valeurs provisoires**, à calibrer avec le reste
    /// du score scénique.
    pub fn terme_relief(&self, pts: &[Point], pas_m: f64) -> Option<f64> {
        let mut ech = vec![*pts.first()?];
        let mut depuis = 0.0;
        for w in pts.windows(2) {
            depuis += haversine_m(w[0], w[1]);
            if depuis >= pas_m {
                ech.push(w[1]);
                depuis = 0.0;
            }
        }
        if let Some(d) = pts.last() {
            if ech.last().map(|p| (p.lat, p.lon)) != Some((d.lat, d.lon)) {
                ech.push(*d);
            }
        }
        let mut somme = 0.0;
        for p in &ech {
            let (la, lo) = (p.lat_deg(), p.lon_deg());
            let a = self.altitude(la, lo)?;
            let (dla, dlo) = (RAYON_ANNEAU_M / 110_540.0, RAYON_ANNEAU_M / (111_320.0 * la.to_radians().cos()));
            let mut anneau = 0.0;
            for k in 0..8 {
                let cap = (k as f64 * 45.0).to_radians();
                anneau += self.altitude(la + dla * cap.cos(), lo + dlo * cap.sin())?;
            }
            let altitude = ((a - 600.0) / 1400.0).clamp(0.0, 1.0);
            let dominance = ((a - anneau / 8.0) / 250.0).clamp(0.0, 1.0);
            somme += 0.5 * altitude + 0.5 * dominance;
        }
        Some(somme / ech.len() as f64)
    }
}

/// Montée et descente cumulées d'un profil, avec seuil de bruit.
pub fn cumuler(alt: &[f64]) -> (u32, u32) {
    let (mut haut, mut bas) = (0.0, 0.0);
    let Some(&premier) = alt.first() else { return (0, 0) };
    let mut ref_ = premier;
    for &a in &alt[1..] {
        let d = a - ref_;
        if d >= SEUIL_BRUIT_M {
            haut += d;
            ref_ = a;
        } else if d <= -SEUIL_BRUIT_M {
            bas -= d;
            ref_ = a;
        }
    }
    (haut.round() as u32, bas.round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumul_ignore_le_bruit_et_compte_les_vraies_variations() {
        assert_eq!(cumuler(&[]), (0, 0));
        assert_eq!(cumuler(&[100.0, 101.0, 100.0, 101.5, 99.0]), (0, 0), "oscillations sous le seuil");
        assert_eq!(cumuler(&[100.0, 110.0, 120.0]), (20, 0));
        assert_eq!(cumuler(&[100.0, 130.0, 110.0, 150.0]), (70, 20), "montée, descente, montée");
        let (h, b) = cumuler(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!((h, b), (6, 0), "une pente régulière de 1 m par pas finit par franchir le seuil, sans perte");
    }

    fn mnt_plat_incline() -> Mnt {
        // Tuile N45E005 : altitude = 500 + 100 × (ligne / 3600) — un gradient nord-sud.
        let mut v = vec![0i16; N * N];
        for r in 0..N {
            for c in 0..N {
                v[r * N + c] = (500.0 + 100.0 * r as f64 / N as f64).round() as i16;
            }
        }
        Mnt { tuiles: HashMap::from([((45, 5), v)]) }
    }

    #[test]
    fn altitude_bilineaire_et_hors_tuile() {
        let m = mnt_plat_incline();
        let a = m.altitude(45.5, 5.5).unwrap();
        assert!((a - 550.0).abs() < 1.0, "{a}");
        assert!(m.altitude(45.5, 6.5).is_none(), "tuile voisine absente");
        assert!(m.altitude(10.0, 10.0).is_none());
    }

    #[test]
    fn denivele_d_une_montee_vers_le_nord() {
        let m = mnt_plat_incline();
        // la ligne 0 est au nord et l'altitude croît avec la ligne : vers le nord, on descend
        let sud_nord = [Point::depuis_degres(45.2, 5.5), Point::depuis_degres(45.8, 5.5)];
        let (h, b) = m.denivele(&sud_nord).unwrap();
        // 0,6° = 2160 lignes → 60 m de moins vers le nord
        assert_eq!(h, 0);
        assert!((b as i32 - 60).abs() <= 3, "{b}");
        let (h2, b2) = m.denivele(&[sud_nord[1], sud_nord[0]]).unwrap();
        assert!((h2 as i32 - 60).abs() <= 3, "{h2}");
        assert_eq!(b2, 0);
    }

    #[test]
    fn terme_relief_borne_et_nul_en_plaine_basse() {
        let m = mnt_plat_incline();
        let t = m.terme_relief(&[Point::depuis_degres(45.4, 5.4), Point::depuis_degres(45.45, 5.45)], 150.0).unwrap();
        // altitude ≈ 540 m (< 600) et pas de dominance sur un plan incliné de 3 m/km : proche de zéro
        assert!((0.0..0.05).contains(&t), "{t}");
    }
}
