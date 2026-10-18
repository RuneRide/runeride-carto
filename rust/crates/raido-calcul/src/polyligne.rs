// SPDX-License-Identifier: EUPL-1.2
//! Encodage de polyligne (format « Encoded Polyline Algorithm », précision 1e-5), lu par
//! Leaflet, MapLibre, Google Maps et la plupart des outils GPX.

pub fn encoder(points: &[(f64, f64)]) -> String {
    let mut s = String::with_capacity(points.len() * 6);
    let (mut lat_prec, mut lon_prec) = (0i64, 0i64);
    for &(lat, lon) in points {
        let (la, lo) = ((lat * 1e5).round() as i64, (lon * 1e5).round() as i64);
        encoder_valeur(la - lat_prec, &mut s);
        encoder_valeur(lo - lon_prec, &mut s);
        lat_prec = la;
        lon_prec = lo;
    }
    s
}

fn encoder_valeur(v: i64, s: &mut String) {
    let mut n = if v < 0 { !(v << 1) } else { v << 1 } as u64;
    while n >= 0x20 {
        s.push(((0x20 | (n & 0x1f)) + 63) as u8 as char);
        n >>= 5;
    }
    s.push((n + 63) as u8 as char);
}

pub fn decoder(s: &str) -> Vec<(f64, f64)> {
    let b = s.as_bytes();
    let (mut i, mut lat, mut lon) = (0usize, 0i64, 0i64);
    let mut pts = Vec::new();
    let lire = |i: &mut usize| -> Option<i64> {
        let (mut r, mut d) = (0u64, 0u32);
        loop {
            let c = (*b.get(*i)? as u64).checked_sub(63)?;
            *i += 1;
            r |= (c & 0x1f) << d;
            d += 5;
            if c < 0x20 {
                break;
            }
        }
        Some(if r & 1 == 1 { !(r >> 1) as i64 } else { (r >> 1) as i64 })
    };
    while let (Some(dla), Some(dlo)) = (lire(&mut i), lire(&mut i)) {
        lat += dla;
        lon += dlo;
        pts.push((lat as f64 / 1e5, lon as f64 / 1e5));
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exemple_de_reference() {
        // Exemple de la documentation Google.
        let pts = [(38.5, -120.2), (40.7, -120.95), (43.252, -126.453)];
        assert_eq!(encoder(&pts), "_p~iF~ps|U_ulLnnqC_mqNvxq`@");
        let d = decoder("_p~iF~ps|U_ulLnnqC_mqNvxq`@");
        assert_eq!(d.len(), 3);
        assert!((d[2].0 - 43.252).abs() < 1e-9 && (d[2].1 + 126.453).abs() < 1e-9);
    }
}
