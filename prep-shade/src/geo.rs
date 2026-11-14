//! Coordinates and rasters.
//!
//! Everything is computed in one flat local frame: metres east/north of the region's bbox centre
//! (equirectangular; over 8 km the error is centimetres, far below the 1 m cells).
//! The LiDAR tiles come in Web Mercator (EPSG:3857), converted exactly to lon/lat first.

use std::io::{Read, Write};
use std::path::Path;

const R: f64 = 6_378_137.0; // WGS84 / Web Mercator sphere radius

/// Web Mercator metres -> (lon, lat) degrees.
pub fn merc_to_lonlat(x: f64, y: f64) -> (f64, f64) {
    ((x / R).to_degrees(), (2.0 * (y / R).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees())
}
/// (lon, lat) degrees -> Web Mercator metres.
pub fn lonlat_to_merc(lon: f64, lat: f64) -> (f64, f64) {
    (R * lon.to_radians(), R * (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln())
}

#[derive(Clone, Copy, Debug)]
pub struct Proj {
    pub lon0: f64,
    pub lat0: f64,
    kx: f64,
    ky: f64,
}
impl Proj {
    pub fn new(lon0: f64, lat0: f64) -> Proj {
        let ky = R * std::f64::consts::PI / 180.0;
        Proj { lon0, lat0, kx: ky * lat0.to_radians().cos(), ky }
    }
    pub fn xy(&self, lon: f64, lat: f64) -> (f64, f64) {
        ((lon - self.lon0) * self.kx, (lat - self.lat0) * self.ky)
    }
    pub fn lonlat(&self, x: f64, y: f64) -> (f64, f64) {
        (self.lon0 + x / self.kx, self.lat0 + y / self.ky)
    }
}

/// A north-up raster of f32 in the local frame. Cell (i, j) covers
/// x in [x0 + i*cell, x0 + (i+1)*cell), y in [y0 + j*cell, ...). NaN = no data.
#[derive(Clone)]
pub struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub cell: f64,
    pub nx: usize,
    pub ny: usize,
    pub v: Vec<f32>,
}

impl Grid {
    pub fn new(x0: f64, y0: f64, cell: f64, nx: usize, ny: usize, fill: f32) -> Grid {
        Grid { x0, y0, cell, nx, ny, v: vec![fill; nx * ny] }
    }
    pub fn same_shape(&self, fill: f32) -> Grid {
        Grid::new(self.x0, self.y0, self.cell, self.nx, self.ny, fill)
    }
    #[inline]
    pub fn idx(&self, x: f64, y: f64) -> Option<usize> {
        let i = ((x - self.x0) / self.cell).floor();
        let j = ((y - self.y0) / self.cell).floor();
        if i < 0.0 || j < 0.0 || i >= self.nx as f64 || j >= self.ny as f64 {
            return None;
        }
        Some(j as usize * self.nx + i as usize)
    }
    /// Nearest-cell value; None outside the grid or on NaN.
    #[inline]
    pub fn at(&self, x: f64, y: f64) -> Option<f32> {
        self.idx(x, y).map(|k| self.v[k]).filter(|v| !v.is_nan())
    }

    /// Fills NaN holes (e.g. no ground points under a building) by repeatedly averaging the known
    /// 8-neighbours, growing inward from the hole edges. Good enough for a ground surface.
    pub fn fill_holes(&mut self) {
        let (nx, ny) = (self.nx, self.ny);
        let mut holes: Vec<usize> = (0..self.v.len()).filter(|&k| self.v[k].is_nan()).collect();
        while !holes.is_empty() {
            let mut fills = Vec::new();
            for &k in &holes {
                let (i, j) = ((k % nx) as i64, (k / nx) as i64);
                let (mut s, mut n) = (0.0f32, 0u32);
                for dj in -1i64..=1 {
                    for di in -1i64..=1 {
                        let (a, b) = (i + di, j + dj);
                        if a < 0 || b < 0 || a >= nx as i64 || b >= ny as i64 {
                            continue;
                        }
                        let w = self.v[b as usize * nx + a as usize];
                        if !w.is_nan() {
                            s += w;
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    fills.push((k, s / n as f32));
                }
            }
            if fills.is_empty() {
                break; // grid has no data at all
            }
            for &(k, v) in &fills {
                self.v[k] = v;
            }
            holes.retain(|&k| self.v[k].is_nan());
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut f = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?);
        let mut hdr = format!("{} {} {} {} {}\n", self.x0, self.y0, self.cell, self.nx, self.ny).into_bytes();
        hdr.resize(64, b' '); // fixed-size text header, then raw little-endian f32, row by row from the south
        f.write_all(&hdr).map_err(|e| e.to_string())?;
        for v in &self.v {
            f.write_all(&v.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Grid, String> {
        let mut buf = Vec::new();
        std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut buf)).map_err(|e| format!("{}: {e}", path.display()))?;
        if buf.len() < 64 {
            return Err(format!("{}: truncated", path.display()));
        }
        let hdr = std::str::from_utf8(&buf[..64]).map_err(|e| e.to_string())?;
        let p: Vec<&str> = hdr.split_whitespace().collect();
        let num = |i: usize| p.get(i).and_then(|s| s.parse::<f64>().ok()).ok_or(format!("{}: bad header", path.display()));
        let (x0, y0, cell, nx, ny) = (num(0)?, num(1)?, num(2)?, num(3)? as usize, num(4)? as usize);
        if buf.len() != 64 + nx * ny * 4 {
            return Err(format!("{}: size mismatch", path.display()));
        }
        let v = buf[64..].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Ok(Grid { x0, y0, cell, nx, ny, v })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mercator_roundtrip() {
        let (x, y) = lonlat_to_merc(-74.66, 40.35);
        let (lon, lat) = merc_to_lonlat(x, y);
        assert!((lon + 74.66).abs() < 1e-9 && (lat - 40.35).abs() < 1e-9);
    }
    #[test]
    fn proj_roundtrip_and_scale() {
        let p = Proj::new(-74.66, 40.35);
        let (x, y) = p.xy(-74.65, 40.36);
        let (lon, lat) = p.lonlat(x, y);
        assert!((lon + 74.65).abs() < 1e-12 && (lat - 40.36).abs() < 1e-12);
        // 0.01 deg lat ~ 1113 m; 0.01 deg lon at 40.35 N ~ 849 m
        assert!((y - 1113.2).abs() < 1.0 && (x - 848.6).abs() < 1.0, "{x} {y}");
    }
    #[test]
    fn fill_holes_fills_everything() {
        let mut g = Grid::new(0.0, 0.0, 1.0, 5, 5, f32::NAN);
        g.v[0] = 10.0;
        g.fill_holes();
        assert!(g.v.iter().all(|v| (*v - 10.0).abs() < 1e-6));
    }
    #[test]
    fn save_load_roundtrip() {
        let mut g = Grid::new(-12.5, 3.0, 1.0, 3, 2, 0.0);
        g.v = vec![1.0, 2.0, f32::NAN, 4.0, 5.0, 6.0];
        let p = std::env::temp_dir().join("prep_shade_grid_test.bin");
        g.save(&p).unwrap();
        let h = Grid::load(&p).unwrap();
        assert_eq!((h.x0, h.y0, h.nx, h.ny), (-12.5, 3.0, 3, 2));
        assert!(h.v[2].is_nan() && h.v[5] == 6.0);
    }
}
