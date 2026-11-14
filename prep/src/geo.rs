//! Geometry helpers: a local flat projection (fine over a few km), the elevation grid, sun times.

use std::path::Path;

/// Equirectangular projection around a reference point: (lon, lat) -> metres east/north.
/// Error is well under 0.1% across a 5 km town, simpler than a real map projection.
#[derive(Clone, Copy)]
pub struct Proj {
    lon0: f64,
    lat0: f64,
    kx: f64,
}

const M_PER_DEG_LAT: f64 = 111_132.0;

impl Proj {
    pub fn new(lon0: f64, lat0: f64) -> Proj {
        Proj { lon0, lat0, kx: M_PER_DEG_LAT * lat0.to_radians().cos() }
    }
    pub fn xy(&self, lon: f64, lat: f64) -> (f64, f64) {
        ((lon - self.lon0) * self.kx, (lat - self.lat0) * M_PER_DEG_LAT)
    }
}

pub fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Distance from point p to segment ab (all in metres).
pub fn dist_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

/// Regular lat/lon grid of elevations (metres), read from the fetch.sh cache, bilinear lookup.
pub struct ElevGrid {
    lat0: f64,
    lon0: f64,
    dlat: f64,
    dlon: f64,
    rows: usize,
    cols: usize,
    z: Vec<f64>,
}

impl ElevGrid {
    /// `points` = elev_points.txt (lat,lon per line, row-major), `dir` = one JSON response per 100 points.
    pub fn load(points: &Path, dir: &Path) -> Result<ElevGrid, String> {
        let text = std::fs::read_to_string(points).map_err(|e| format!("{}: {e} (run prep/fetch.sh)", points.display()))?;
        let pts: Vec<(f64, f64)> = text
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                let (a, b) = l.split_once(',').expect("lat,lon");
                (a.parse().unwrap(), b.parse().unwrap())
            })
            .collect();
        let cols = pts.iter().take_while(|p| p.0 == pts[0].0).count();
        let rows = pts.len() / cols;
        if rows * cols != pts.len() || rows < 2 || cols < 2 {
            return Err("elev_points.txt is not a full grid".into());
        }
        let mut z = Vec::with_capacity(pts.len());
        for b in 0..pts.len().div_ceil(100) {
            let f = dir.join(format!("{b:04}.json"));
            let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e} (run prep/fetch.sh)", f.display()))?;
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
            for r in v["results"].as_array().ok_or("no results")? {
                z.push(r["elevation"].as_f64().ok_or_else(|| format!("{}: null elevation", f.display()))?);
            }
        }
        if z.len() != pts.len() {
            return Err(format!("elevation count {} != points {}", z.len(), pts.len()));
        }
        Ok(ElevGrid {
            lat0: pts[0].0,
            lon0: pts[0].1,
            dlat: pts[cols].0 - pts[0].0,
            dlon: pts[1].1 - pts[0].1,
            rows,
            cols,
            z,
        })
    }

    /// Bilinear interpolation; points outside the grid are clamped to its edge.
    pub fn at(&self, lon: f64, lat: f64) -> f64 {
        let fr = ((lat - self.lat0) / self.dlat).clamp(0.0, (self.rows - 1) as f64);
        let fc = ((lon - self.lon0) / self.dlon).clamp(0.0, (self.cols - 1) as f64);
        let (r, c) = ((fr as usize).min(self.rows - 2), (fc as usize).min(self.cols - 2));
        let (tr, tc) = (fr - r as f64, fc - c as f64);
        let z = |i: usize, j: usize| self.z[i * self.cols + j];
        (1.0 - tr) * ((1.0 - tc) * z(r, c) + tc * z(r, c + 1)) + tr * ((1.0 - tc) * z(r + 1, c) + tc * z(r + 1, c + 1))
    }

    pub fn range(&self) -> (f64, f64) {
        self.z.iter().fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)))
    }
}

/// Sunrise and sunset in local minutes since midnight (NOAA solar calculator approximation,
/// accurate to ~1 minute). `utc_offset_h` = -5 for EST.
pub fn sun_times(year: i32, month: u32, day: u32, lat: f64, lon: f64, utc_offset_h: f64) -> (f64, f64) {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let cum = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let doy = cum[(month - 1) as usize] + day + u32::from(leap && month > 2);
    let g = 2.0 * std::f64::consts::PI / if leap { 366.0 } else { 365.0 } * (doy as f64 - 1.0);
    let eqtime = 229.18
        * (0.000075 + 0.001868 * g.cos() - 0.032077 * g.sin() - 0.014615 * (2.0 * g).cos() - 0.040849 * (2.0 * g).sin());
    let decl = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin() - 0.006758 * (2.0 * g).cos()
        + 0.000907 * (2.0 * g).sin() - 0.002697 * (3.0 * g).cos() + 0.00148 * (3.0 * g).sin();
    let phi = lat.to_radians();
    // 90.833 deg = sun's centre 50' below the horizon (refraction + solar radius)
    let ha = ((90.833f64.to_radians().cos() / (phi.cos() * decl.cos())) - phi.tan() * decl.tan()).acos().to_degrees();
    let rise = 720.0 - 4.0 * (lon + ha) - eqtime + utc_offset_h * 60.0;
    let set = 720.0 - 4.0 * (lon - ha) - eqtime + utc_offset_h * 60.0;
    (rise, set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn princeton_sun_nov_15() {
        // Princeton NJ mid-November: sunrise ~06:43, sunset ~16:42 EST (almanac values, +-4 min)
        let (r, s) = sun_times(2026, 11, 15, 40.35, -74.66, -5.0);
        assert!((r - (6.0 * 60.0 + 43.0)).abs() < 4.0, "sunrise {r}");
        assert!((s - (16.0 * 60.0 + 42.0)).abs() < 4.0, "sunset {s}");
    }

    #[test]
    fn segment_distance() {
        assert_eq!(dist_to_segment((0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)), 1.0);
        assert_eq!(dist_to_segment((3.0, 0.0), (-1.0, 0.0), (1.0, 0.0)), 2.0);
    }
}
