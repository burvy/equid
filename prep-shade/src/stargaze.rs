//! Stargazing spots: places on the walk network where you see a lot of sky and it is dark.
//!
//! Open sky: from head height, look along 36 directions (every 10 deg) and find the skyline angle h
//! (roofs, tree crowns, hills; crowns count as solid, since even bare branches clutter a star field).
//! The usable sky is everything above 10 deg (lower down, haze and town glow hide stars anyway).
//! In a direction where h > 10 deg the part from 10 deg to h is lost. Sky share = the visible part of
//! the sky above 10 deg, weighted by solid angle (1 - sin(alt)), averaged over the 36 directions.
//!
//! Darkness: distance to the nearest street that has lights (graph `lit` = true: OSM tag or the prep
//! inference rules). A street for cars whose lighting is unknown (most residential streets) probably has
//! a few lamps, plus porch lights: it counts as a lit street at twice its real distance. Light outside
//! the region is unknown, so the distance is capped by the distance to the edge of the region box.

use crate::geo::Proj;
use crate::shade::{Scene, EYE_M};
use engine::graph::Graph;
use rayon::prelude::*;

const DIRECTIONS: usize = 36;
/// Stars below this altitude don't count (haze, glow from town).
pub const MIN_ALT_DEG: f64 = 10.0;
/// Look this far for the skyline. Past 1 km, blocking 10 deg would take a 175 m tall obstacle.
const MAX_D: f64 = 1000.0;
/// Full darkness score at this distance from the nearest lit street (a streetlight's glare fades in ~100-200 m).
pub const DARK_FULL_M: f64 = 250.0;

pub struct Sky {
    /// Share of the sky above MIN_ALT_DEG you can see, 0..1.
    pub open: f64,
    /// Metres to the nearest lit street (capped by the distance to the region edge).
    pub dark_m: f64,
    /// false = no ground model here (open = 0).
    pub ok: bool,
}

impl Sky {
    /// Ranking score: open sky times darkness (both 0..1). Simple on purpose: a spot needs both.
    pub fn score(&self) -> f64 {
        self.open * (self.dark_m / DARK_FULL_M).min(1.0)
    }
}

/// Sky share for one point (local metres, eye height above the ground model).
pub fn open_sky(scene: &Scene, x: f64, y: f64) -> Option<f64> {
    let z0 = scene.ground(x, y)? + EYE_M;
    let s_min = MIN_ALT_DEG.to_radians().sin();
    let mut acc = 0.0;
    for k in 0..DIRECTIONS {
        let az = k as f64 * 360.0 / DIRECTIONS as f64;
        let (t, _) = scene.horizon_tan(x, y, z0, az, MAX_D, f64::INFINITY);
        let s_h = t.atan().sin().max(s_min); // sin of the skyline angle, at least the 10 deg floor
        acc += (1.0 - s_h) / (1.0 - s_min);
    }
    Some(acc / DIRECTIONS as f64)
}

/// Unknown lighting on these counts as "maybe lit" (see the module doc).
pub fn maybe_lit(highway: &str) -> bool {
    matches!(highway, "residential" | "unclassified" | "living_street" | "service" | "road")
}

/// Sky + darkness for every graph node. `lit_pts` / `maybe_pts` are points along lit and maybe-lit
/// streets (local metres).
pub fn evaluate(scene: &Scene, graph: &Graph, pr: &Proj, lit_pts: &[(f64, f64)], maybe_pts: &[(f64, f64)], bbox_xy: [f64; 4]) -> Vec<Sky> {
    let idx = PointIndex::new(lit_pts, 50.0);
    let maybe = PointIndex::new(maybe_pts, 50.0);
    graph
        .nodes
        .par_iter()
        .map(|n| {
            let (x, y) = pr.xy(n.lon, n.lat);
            let edge = (x - bbox_xy[0]).min(bbox_xy[2] - x).min(y - bbox_xy[1]).min(bbox_xy[3] - y).max(0.0);
            let far = DARK_FULL_M.max(edge);
            let dark_m = idx.nearest(x, y, far).min(2.0 * maybe.nearest(x, y, far / 2.0)).min(edge);
            match open_sky(scene, x, y) {
                Some(open) => Sky { open, dark_m, ok: true },
                None => Sky { open: 0.0, dark_m, ok: false },
            }
        })
        .collect()
}

/// Bucket grid for "nearest lit point" lookups.
struct PointIndex {
    cell: f64,
    map: std::collections::HashMap<(i64, i64), Vec<(f64, f64)>>,
}
impl PointIndex {
    fn new(pts: &[(f64, f64)], cell: f64) -> Self {
        let mut map: std::collections::HashMap<(i64, i64), Vec<(f64, f64)>> = Default::default();
        for &(x, y) in pts {
            map.entry(((x / cell).floor() as i64, (y / cell).floor() as i64)).or_default().push((x, y));
        }
        PointIndex { cell, map }
    }
    /// Distance to the nearest point, or `max` if none is within `max`.
    fn nearest(&self, x: f64, y: f64, max: f64) -> f64 {
        let r = (max / self.cell).ceil() as i64 + 1;
        let (ci, cj) = ((x / self.cell).floor() as i64, (y / self.cell).floor() as i64);
        let mut best = max * max;
        for dj in -r..=r {
            for di in -r..=r {
                if let Some(v) = self.map.get(&(ci + di, cj + dj)) {
                    for &(px, py) in v {
                        best = best.min((px - x).powi(2) + (py - y).powi(2));
                    }
                }
            }
        }
        best.sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::Grid;

    fn flat(z: f32) -> Grid {
        Grid::new(-1000.0, -1000.0, 1.0, 2000, 2000, z)
    }

    #[test]
    fn flat_ground_sees_the_whole_sky() {
        let scene = Scene::new(flat(10.0), flat(10.0), flat(0.0), 0.1);
        let s = open_sky(&scene, 0.0, 0.0).unwrap();
        assert!((s - 1.0).abs() < 1e-9, "{s}");
    }

    #[test]
    fn a_wall_on_one_side_hides_part_of_the_sky() {
        // 30 m wall 10 m east of the point: blocks ~half the directions up to steep angles
        let mut dsm = flat(0.0);
        for j in 0..dsm.ny {
            for i in 1010..1030 {
                dsm.v[j * dsm.nx + i] = 30.0;
            }
        }
        let scene = Scene::new(dsm, flat(0.0), flat(0.0), 0.1);
        let s = open_sky(&scene, 0.0, 0.0).unwrap();
        assert!(s > 0.4 && s < 0.75, "{s}");
    }

    #[test]
    fn nearest_lit_point() {
        let idx = PointIndex::new(&[(100.0, 0.0), (0.0, 300.0)], 50.0);
        assert!((idx.nearest(0.0, 0.0, 500.0) - 100.0).abs() < 1e-9);
        assert_eq!(idx.nearest(1000.0, 1000.0, 200.0), 200.0);
    }
}
