//! Shadows: how much direct sun reaches a person on the street? Walk along the straight line from
//! them toward the sun over the 1 m surface model one cell at a time ("ray marching"):
//! - a roof, wall or hill above the line blocks the sun completely;
//! - a tree crown above the line only dims it: light that crosses L metres of crown keeps
//!   exp(-kappa * L) of its strength (Beer-Lambert law, the standard canopy light model).
//!   kappa depends on the season: bare November branches let much more through than July leaves.
//!
//! Speed-up (does not change the answer): the surface is cut into 32 x 32 m blocks and each block's
//! highest point is kept. Once the ray is above a block's highest point it cannot hit anything in
//! that block (the ray only climbs), so it jumps straight to the block's edge.

use crate::geo::Grid;

/// Height of the point we test above the ground: roughly a walking person's head/shoulders.
pub const EYE_M: f64 = 1.6;
/// Rays that leave the LiDAR area are treated as unblocked.
const BLOCK: usize = 32;

pub struct Scene {
    pub dsm: Grid,
    pub dtm: Grid,
    /// 1 = tree crown cell (porous), 0 = roof/terrain (solid)
    pub tree: Grid,
    /// Beer-Lambert extinction per metre of crown the sunbeam crosses (season dependent, see `kappa_for_day`)
    pub kappa: f64,
    bmax: Vec<f32>,
    bnx: usize,
    zmax: f32,
}

impl Scene {
    pub fn new(dsm: Grid, dtm: Grid, tree: Grid, kappa: f64) -> Scene {
        let bnx = dsm.nx.div_ceil(BLOCK);
        let bny = dsm.ny.div_ceil(BLOCK);
        let mut bmax = vec![f32::NEG_INFINITY; bnx * bny];
        for j in 0..dsm.ny {
            for i in 0..dsm.nx {
                let b = (j / BLOCK) * bnx + i / BLOCK;
                bmax[b] = bmax[b].max(dsm.v[j * dsm.nx + i]);
            }
        }
        let zmax = bmax.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        Scene { dsm, dtm, tree, kappa, bmax, bnx, zmax }
    }

    /// Ground height at (x, y), local metres.
    pub fn ground(&self, x: f64, y: f64) -> Option<f64> {
        self.dtm.at(x, y).map(|v| v as f64)
    }

    /// Steepest angle (as tan) of the surface seen from (x, y, z0) looking along azimuth `az_deg`
    /// (clockwise from north), out to `max_d` metres. The sun is hidden iff tan(altitude) < this.
    /// Exact when `stop_tan` is +inf. With a finite `stop_tan` (shade test) it returns as soon as the
    /// surface reaches that angle, or once nothing ahead can (then the value is only a lower bound).
    /// Returns (tan of the horizon angle, ran off the edge of the data before max_d).
    pub fn horizon_tan(&self, x: f64, y: f64, z0: f64, az_deg: f64, max_d: f64, stop_tan: f64) -> (f64, bool) {
        let (dx, dy) = (az_deg.to_radians().sin(), az_deg.to_radians().cos());
        let g = &self.dsm;
        let cell = g.cell;
        let mut best = f64::NEG_INFINITY;
        let mut d = 0.5 * cell;
        while d <= max_d {
            let (px, py) = (x + dx * d, y + dy * d);
            let fi = (px - g.x0) / cell;
            let fj = (py - g.y0) / cell;
            if fi < 0.0 || fj < 0.0 || fi >= g.nx as f64 || fj >= g.ny as f64 {
                return (best, true);
            }
            let (i, j) = (fi as usize, fj as usize);
            // Nothing in the whole area can beat `best` (or the stop threshold) from here on: done.
            // (h / d is monotonic in d, so checking the nearest and farthest d covers everything between.)
            let need = if stop_tan.is_finite() { stop_tan } else { best };
            let top = self.zmax as f64 - z0;
            if top >= 0.0 && top / d <= need {
                return (best, false);
            }
            // Nothing in this block can: jump to where the ray leaves the block.
            let bm = self.bmax[(j / BLOCK) * self.bnx + i / BLOCK] as f64 - z0;
            if bm / d <= need {
                let bx0 = g.x0 + ((i / BLOCK) * BLOCK) as f64 * cell;
                let by0 = g.y0 + ((j / BLOCK) * BLOCK) as f64 * cell;
                let side = BLOCK as f64 * cell;
                let tx = if dx > 1e-12 { (bx0 + side - px) / dx } else if dx < -1e-12 { (bx0 - px) / dx } else { f64::INFINITY };
                let ty = if dy > 1e-12 { (by0 + side - py) / dy } else if dy < -1e-12 { (by0 - py) / dy } else { f64::INFINITY };
                let exit = d + tx.min(ty).max(0.0);
                if bm / exit <= need {
                    // land on the same cell-centre lattice a plain march would use
                    d += ((exit - d) / cell).floor().max(0.0) * cell + cell;
                    continue;
                }
            }
            let t = (g.v[j * g.nx + i] as f64 - z0) / d;
            if t > best {
                best = t;
                if best >= stop_tan {
                    return (best, false);
                }
            }
            d += cell;
        }
        (best, false)
    }


    /// Share of direct sunlight reaching (x, y, z0) from the sun at (alt_deg, az_deg), 0..1.
    /// March along the ray toward the sun over the 1 m grid:
    /// - roof, wall or terrain above the ray: blocked, 0;
    /// - tree crown above the ray (and the ray above the crown's base, i.e. not passing under it
    ///   beside the trunk): light is dimmed by Beer-Lambert's law, T *= exp(-kappa * metres inside the crown).
    pub fn sunlight(&self, x: f64, y: f64, z0: f64, alt_deg: f64, az_deg: f64) -> f64 {
        if alt_deg <= 0.0 {
            return 0.0;
        }
        let tan_alt = alt_deg.to_radians().tan();
        let per_cell_in_crown = self.dsm.cell / alt_deg.to_radians().cos(); // ray length per cell crossed
        let dim = (-self.kappa * per_cell_in_crown).exp();
        let (dx, dy) = (az_deg.to_radians().sin(), az_deg.to_radians().cos());
        let g = &self.dsm;
        let cell = g.cell;
        let mut t = 1.0f64;
        let mut d = 0.5 * cell;
        while d <= 5000.0 {
            let zr = z0 + d * tan_alt;
            if zr > self.zmax as f64 {
                return t;
            }
            let (px, py) = (x + dx * d, y + dy * d);
            let fi = (px - g.x0) / cell;
            let fj = (py - g.y0) / cell;
            if fi < 0.0 || fj < 0.0 || fi >= g.nx as f64 || fj >= g.ny as f64 {
                return t;
            }
            let (i, j) = (fi as usize, fj as usize);
            // the ray only climbs: above this block's highest point now = above all of the block
            let bm = self.bmax[(j / BLOCK) * self.bnx + i / BLOCK] as f64;
            if zr > bm {
                let bx0 = g.x0 + ((i / BLOCK) * BLOCK) as f64 * cell;
                let by0 = g.y0 + ((j / BLOCK) * BLOCK) as f64 * cell;
                let side = BLOCK as f64 * cell;
                let tx = if dx > 1e-12 { (bx0 + side - px) / dx } else if dx < -1e-12 { (bx0 - px) / dx } else { f64::INFINITY };
                let ty = if dy > 1e-12 { (by0 + side - py) / dy } else if dy < -1e-12 { (by0 - py) / dy } else { f64::INFINITY };
                d += (tx.min(ty).max(0.0) / cell).floor() * cell + cell;
                continue;
            }
            let k = j * g.nx + i;
            let top = g.v[k] as f64;
            if zr < top {
                if self.tree.v[k] > 0.5 {
                    let ground = self.dtm.v[k] as f64;
                    let base = ground + (0.35 * (top - ground)).max(2.5);
                    if zr >= base {
                        t *= dim;
                        if t < 0.02 {
                            return 0.0;
                        }
                    }
                } else {
                    return 0.0;
                }
            }
            d += cell;
        }
        t
    }
}

/// Crown extinction coefficient kappa (per metre of crown crossed) for a day of the year.
/// Leaf-on (Jun-Sep) 0.30/m: 10 m of crown passes 5% of the sun. Leaf-off (Dec-Mar) 0.05/m: bare
/// branches, 10 m passes 60%. Linear in between (leaf-out Apr-May, leaf-fall Oct-Nov).
/// These are round, defensible numbers, not measured for Princeton; they set how dark a tree's shade is.
pub fn kappa_for_day(doy: u32) -> f64 {
    const ON: f64 = 0.30;
    const OFF: f64 = 0.05;
    let d = doy as f64;
    let leaf = if d < 91.0 || d >= 335.0 {
        0.0 // Dec 1 .. Mar 31
    } else if d < 152.0 {
        (d - 91.0) / 61.0 // Apr 1 .. May 31: leaves come out
    } else if d < 274.0 {
        1.0 // Jun 1 .. Sep 30
    } else {
        1.0 - (d - 274.0) / 61.0 // Oct 1 .. Nov 30: leaves fall
    };
    OFF + (ON - OFF) * leaf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat ground at 0 with a 10 m tall wall along x in [50, 52), all y; optionally the wall is a hedge of tree crown.
    fn wall_scene(tree: bool) -> Scene {
        let mut dsm = Grid::new(0.0, 0.0, 1.0, 200, 100, 0.0);
        let mut t = dsm.same_shape(0.0);
        for j in 0..100 {
            for i in 50..52 {
                dsm.v[j * 200 + i] = 10.0;
                t.v[j * 200 + i] = if tree { 1.0 } else { 0.0 };
            }
        }
        let dtm = dsm.same_shape(0.0);
        Scene::new(dsm, dtm, t, 0.1)
    }

    #[test]
    fn wall_casts_a_shadow_of_the_right_length() {
        let s = wall_scene(false);
        let shaded = |x: f64, alt: f64, az: f64| s.sunlight(x, 50.0, EYE_M, alt, az) == 0.0;
        // Sun due east (az 90) at 45 deg: shadow of a 10 m wall reaches ~10 m (minus eye height) west of it.
        assert!(shaded(45.0, 45.0, 90.0), "5 m west of the wall");
        assert!(!shaded(30.0, 45.0, 90.0), "20 m west of the wall");
        assert_eq!(s.sunlight(30.0, 50.0, EYE_M, 45.0, 90.0), 1.0);
        // Sun due west: the west side is lit.
        assert!(!shaded(45.0, 45.0, 270.0));
        // Low sun from the east (10 deg): shadow ~ (10-1.6)/tan 10 = 47.6 m.
        assert!(shaded(10.0, 10.0, 90.0), "40 m away, low sun");
        assert!(!shaded(1.0, 30.0, 90.0), "49 m away, sun at 30");
        // Night is shade.
        assert!(shaded(150.0, -3.0, 90.0));
    }

    #[test]
    fn tree_crown_dims_by_beer_lambert() {
        let s = wall_scene(true);
        // 5 m west, sun due east at 45 deg: the ray crosses the 2 cells of crown at z ~ 6..8 m
        // (above the crown base 3.5 m): 2 cells x 1/cos45 m each.
        let got = s.sunlight(45.0, 50.0, EYE_M, 45.0, 90.0);
        let want = (-0.1 * 2.0 * std::f64::consts::SQRT_2).exp();
        assert!((got - want).abs() < 1e-9, "{got} vs {want}");
        // Right next to the hedge with a lowish sun the ray passes under the crown base (3.5 m): full sun.
        assert_eq!(s.sunlight(49.0, 50.0, EYE_M, 30.0, 90.0), 1.0);
    }

    #[test]
    fn seasons() {
        assert!((kappa_for_day(196) - 0.30).abs() < 1e-12); // mid July
        assert!((kappa_for_day(15) - 0.05).abs() < 1e-12); // mid January
        let nov15 = kappa_for_day(319);
        assert!(nov15 > 0.05 && nov15 < 0.12, "{nov15}");
    }

    #[test]
    fn block_skip_matches_brute_force() {
        // Random-ish bumpy surface; compare the jumping march against a plain cell-by-cell one.
        let mut dsm = Grid::new(0.0, 0.0, 1.0, 300, 300, 0.0);
        let mut seed = 12345u64;
        for v in dsm.v.iter_mut() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            if (seed >> 33) % 97 == 0 {
                *v = ((seed >> 40) % 25) as f32;
            }
        }
        let dtm = dsm.same_shape(0.0);
        let s = Scene::new(dsm.clone(), dtm.clone(), dtm, 0.1);
        for k in 0..400 {
            let (x, y) = (20.0 + (k * 37 % 260) as f64, 20.0 + (k * 53 % 260) as f64);
            let az = (k * 29 % 360) as f64;
            let (fast, _) = s.horizon_tan(x, y, 1.6, az, 400.0, f64::INFINITY);
            let mut slow = f64::NEG_INFINITY;
            let mut d = 0.5;
            while d <= 400.0 {
                let (px, py) = (x + az.to_radians().sin() * d, y + az.to_radians().cos() * d);
                match dsm.at(px, py) {
                    Some(z) => slow = slow.max((z as f64 - 1.6) / d),
                    None => break,
                }
                d += 1.0;
            }
            assert!((fast - slow).abs() < 1e-9, "k={k}: {fast} vs {slow}");
        }
    }
}
