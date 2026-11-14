//! Open-vocabulary objects (task 015). Any plain-language label ("playground", "crosswalk", "trash can")
//! can be a street feature: given the points where that object was seen (OSM, Vexcel detect, ...),
//! the engine computes per edge the distance to the nearest one (capped at 500 m, like `near_bench_m`)
//! and the metric can weight it as `near:<label>` (cost = weight x length x distance / 500, >= 0).
//! The data format is a plain list of points; the engine derives the per-edge distances at load time,
//! so a live detect just adds more points.

use crate::graph::{Graph, NEAR_CAP_M};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const M_PER_DEG_LAT: f64 = 110_540.0;
const M_PER_DEG_LON_EQ: f64 = 111_320.0;

/// `objects.json`: `{"version":1,"objects":[{"label":"playground","lon":-74.65,"lat":40.34,"src":"osm"}]}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objects {
    pub version: u32,
    pub objects: Vec<Object>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub label: String,
    pub lon: f64,
    pub lat: f64,
    /// Where it came from: "osm", "vexcel", ...
    #[serde(default)]
    pub src: Option<String>,
    /// Detection confidence 0..1 if it came from a model.
    #[serde(default)]
    pub conf: Option<f32>,
}

/// All known points of one label plus the derived per-edge distance (metres, capped at 500).
#[derive(Debug, Clone, Default)]
pub struct Layer {
    pub points: Vec<(f64, f64)>,
    pub near: Vec<f32>,
}

/// One entry of the label menu.
#[derive(Debug, Clone, Serialize)]
pub struct LabelInfo {
    pub label: String,
    /// Number of objects seen.
    pub objects: usize,
    /// Edges within 100 m of one (how much of the street network it matters for).
    pub edges_near: usize,
}

pub struct Metres {
    k: f64,
    olon: f64,
    olat: f64,
}

impl Metres {
    pub fn new(g: &Graph) -> Metres {
        let [w, s, e, n] = g.meta.bbox;
        let (olon, olat) = ((w + e) / 2.0, (s + n) / 2.0);
        Metres { k: olat.to_radians().cos() * M_PER_DEG_LON_EQ, olon, olat }
    }
    fn xy(&self, lon: f64, lat: f64) -> (f64, f64) {
        ((lon - self.olon) * self.k, (lat - self.olat) * M_PER_DEG_LAT)
    }
}

fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// Per-edge distance (m, capped at 500) from the edge geometry to the nearest of `points`.
pub fn near_per_edge(g: &Graph, m: &Metres, points: &[(f64, f64)]) -> Vec<f32> {
    let cap = NEAR_CAP_M as f64;
    let cell = cap;
    let key = |x: f64, y: f64| ((x / cell).floor() as i64, (y / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<(f64, f64)>> = HashMap::new();
    for &(lon, lat) in points {
        let p = m.xy(lon, lat);
        grid.entry(key(p.0, p.1)).or_default().push(p);
    }
    g.edges
        .iter()
        .map(|e| {
            let pts: Vec<(f64, f64)> = e.geom.iter().map(|v| m.xy(v[0], v[1])).collect();
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in &pts {
                x0 = x0.min(p.0);
                y0 = y0.min(p.1);
                x1 = x1.max(p.0);
                y1 = y1.max(p.1);
            }
            let (cx0, cy0) = key(x0 - cap, y0 - cap);
            let (cx1, cy1) = key(x1 + cap, y1 + cap);
            let mut best = cap;
            for cx in cx0..=cx1 {
                for cy in cy0..=cy1 {
                    for &p in grid.get(&(cx, cy)).into_iter().flatten() {
                        for w in pts.windows(2) {
                            best = best.min(seg_dist(p, w[0], w[1]));
                        }
                    }
                }
            }
            best as f32
        })
        .collect()
}
