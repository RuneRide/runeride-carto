// SPDX-License-Identifier: EUPL-1.2
//! Petites fonctions géographiques : point à cap et distance, emprises.

use raido_format::Point;

/// Point situé à `distance_m` de `(lat, lon)` dans la direction `cap_deg` (0 = nord).
pub fn destination(lat: f64, lon: f64, cap_deg: f64, distance_m: f64) -> (f64, f64) {
    const R: f64 = 6_371_008.8;
    let (la1, lo1, cap) = (lat.to_radians(), lon.to_radians(), cap_deg.to_radians());
    let d = distance_m / R;
    let la2 = (la1.sin() * d.cos() + la1.cos() * d.sin() * cap.cos()).asin();
    let lo2 = lo1 + (cap.sin() * d.sin() * la1.cos()).atan2(d.cos() - la1.sin() * la2.sin());
    (la2.to_degrees(), lo2.to_degrees())
}

/// Emprise (sud, ouest, nord, est) autour d'un point, avec une marge en mètres.
pub fn emprise_autour(lat: f64, lon: f64, marge_m: f64) -> [f64; 4] {
    let dlat = marge_m / 111_000.0;
    let dlon = marge_m / (111_000.0 * lat.to_radians().cos().max(0.2));
    [lat - dlat, lon - dlon, lat + dlat, lon + dlon]
}

/// Emprise englobant deux points, élargie d'une marge.
pub fn emprise_entre(a: (f64, f64), b: (f64, f64), marge_m: f64) -> [f64; 4] {
    let lat = (a.0 + b.0) / 2.0;
    let dlat = marge_m / 111_000.0;
    let dlon = marge_m / (111_000.0 * lat.to_radians().cos().max(0.2));
    [a.0.min(b.0) - dlat, a.1.min(b.1) - dlon, a.0.max(b.0) + dlat, a.1.max(b.1) + dlon]
}

pub fn deg(p: Point) -> (f64, f64) {
    (p.lat_deg(), p.lon_deg())
}

/// Générateur pseudo-aléatoire minimal (xorshift64*) pour les bancs d'essai reproductibles.
pub struct Alea(u64);

impl Alea {
    pub fn nouveau(graine: u64) -> Alea {
        Alea(graine.max(1))
    }
    pub fn suivant(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Réel dans [0, 1).
    pub fn reel(&mut self) -> f64 {
        (self.suivant() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_nord_10km() {
        let (la, lo) = destination(45.0, 5.0, 0.0, 10_000.0);
        assert!((la - 45.0899).abs() < 1e-3, "{la}");
        assert!((lo - 5.0).abs() < 1e-9);
    }
}
