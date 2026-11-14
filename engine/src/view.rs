//! View buffers: the per-vertex math for the map / warp / 3D views, so the page can hand GPU-ready
//! arrays straight to deck.gl (binary PathLayer: positions + startIndices + colours).
//!
//! Every vertex has a real position and a warped one:
//!   warped = origin + (direction to the vertex) x (cost to it x metres_per_s)      (bearing kept, radius = cost)
//! `warp` in [0,1] blends real -> warped (so switching views animates), `lift` is metres of height per
//! second of cost (0 = flat). Positions are lon/lat DEGREE OFFSETS from `origin_lon/lat` (the bbox centre)
//! with z in metres, for deck's LNGLAT_OFFSETS: f32 absolute lon/lat would jitter by ~1 m.

use crate::graph::Graph;

const M_PER_DEG_LAT: f64 = 110_540.0;
const M_PER_DEG_LON_EQ: f64 = 111_320.0;

#[derive(Debug, Clone, Copy)]
pub struct ViewParams {
    /// 0 = real positions, 1 = warped.
    pub warp: f32,
    /// Metres of height per second of cost.
    pub lift: f32,
    /// Warp radius scale: cost (s) x this = radius (m).
    pub metres_per_s: f32,
    /// Cost (s) at which the colour ramp reaches red; beyond it, grey.
    pub budget_s: f32,
    /// 0..1: how far beyond-budget streets/places are put away. They fade out (alpha x (1 - hide)) and
    /// their position cost is pulled down to the budget, so in 3D they don't rise as grey towers and in warp
    /// they don't sit far out over the picture. At 1, edges wholly beyond the budget are dropped (no picking).
    pub hide_beyond: f32,
}

/// Vertex data of every edge's geometry, flattened once at load.
pub struct Shapes {
    /// Start offset of edge i's vertices in the arrays below (length edges + 1).
    start: Vec<u32>,
    lon: Vec<f64>,
    lat: Vec<f64>,
    /// Fraction (0..1) of the way along the edge, by length.
    frac: Vec<f32>,
    /// Bbox centre: the origin for position offsets.
    pub origin_lon: f64,
    pub origin_lat: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ViewBuffers {
    /// xyz per vertex (dLon deg, dLat deg, z m), paths concatenated.
    pub positions: Vec<f32>,
    /// rgba per vertex.
    pub colors: Vec<u8>,
    /// Cost (s) per vertex (after linear interpolation along the edge).
    pub vertex_cost: Vec<f32>,
    /// Index of each path's first vertex (one per path); the wasm getter appends the total for deck's `startIndices`.
    pub start_indices: Vec<u32>,
    /// Graph edge index of each path (for picking). Edges with both ends unreachable are omitted.
    pub path_edge: Vec<u32>,
    /// xyz per place, places.json order. Unreachable places stay at their real position, z 0, grey.
    pub place_positions: Vec<f32>,
    pub place_colors: Vec<u8>,
    pub origin_lon: f64,
    pub origin_lat: f64,
}

impl Shapes {
    pub fn new(g: &Graph) -> Shapes {
        let [w, s, e, n] = g.meta.bbox;
        let (mut start, mut lon, mut lat, mut frac) = (vec![0u32], vec![], vec![], vec![]);
        for edge in &g.edges {
            let pts = &edge.geom;
            let mut cum = vec![0.0f64];
            for i in 1..pts.len() {
                let k = pts[i - 1][1].to_radians().cos();
                let d = (((pts[i][0] - pts[i - 1][0]) * k * M_PER_DEG_LON_EQ).powi(2)
                    + ((pts[i][1] - pts[i - 1][1]) * M_PER_DEG_LAT).powi(2))
                .sqrt();
                cum.push(cum[i - 1] + d);
            }
            let total = *cum.last().unwrap();
            for (i, p) in pts.iter().enumerate() {
                lon.push(p[0]);
                lat.push(p[1]);
                frac.push(if total > 0.0 { (cum[i] / total) as f32 } else { i as f32 / (pts.len() - 1).max(1) as f32 });
            }
            start.push(lon.len() as u32);
        }
        Shapes { start, lon, lat, frac, origin_lon: (w + e) / 2.0, origin_lat: (s + n) / 2.0 }
    }
}

/// Cost along an edge, linearly interpolated between its end nodes (close to exact, much simpler).
pub fn vertex_cost(cf: f32, ct: f32, s: f32) -> f32 {
    if !cf.is_finite() {
        ct
    } else if !ct.is_finite() {
        cf
    } else {
        cf + (ct - cf) * s
    }
}

/// Cost -> rgba: green (near) -> yellow -> red (at the budget), grey beyond it or unreachable.
pub fn cost_color(c: f32, budget_s: f32) -> [u8; 4] {
    if !c.is_finite() {
        return [150, 150, 150, 60];
    }
    let x = c / budget_s;
    if x > 1.0 || x.is_nan() {
        return [120, 120, 130, 70];
    }
    const STOPS: [[f32; 3]; 3] = [[26.0, 152.0, 80.0], [254.0, 224.0, 90.0], [215.0, 48.0, 39.0]];
    let t = x * 2.0;
    let i = (t.floor() as usize).min(1);
    let f = t - i as f32;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    let m = |k: usize| (a[k] + (b[k] - a[k]) * f) as u8;
    [m(0), m(1), m(2), 230]
}

/// (position cost, rgba) for a point with cost `c`, applying `hide_beyond`.
fn shown(c: f32, p: &ViewParams) -> (f32, [u8; 4]) {
    let mut col = cost_color(c, p.budget_s);
    let h = p.hide_beyond.clamp(0.0, 1.0);
    if h > 0.0 && c > p.budget_s {
        col[3] = (col[3] as f32 * (1.0 - h)) as u8;
        let cap = if c.is_finite() { c + (p.budget_s - c) * h } else { p.budget_s };
        return (cap, col);
    }
    (c, col)
}

/// Position of a point with cost `c` under the view parameters, as (dLon, dLat, z) offsets.
fn blend(sh: &Shapes, lon: f64, lat: f64, c: f32, o: (f64, f64, f64), p: &ViewParams) -> [f32; 3] {
    let (olon, olat, k) = o;
    let (mut x, mut y) = (lon, lat);
    if p.warp > 0.0 {
        let dx = (lon - olon) * k * M_PER_DEG_LON_EQ;
        let dy = (lat - olat) * M_PER_DEG_LAT;
        let d = dx.hypot(dy);
        let r = c as f64 * p.metres_per_s as f64;
        let (wx, wy) = if d > 0.01 { (dx / d * r, dy / d * r) } else { (0.0, 0.0) };
        let (wlon, wlat) = (olon + wx / (k * M_PER_DEG_LON_EQ), olat + wy / M_PER_DEG_LAT);
        x += (wlon - lon) * p.warp as f64;
        y += (wlat - lat) * p.warp as f64;
    }
    [(x - sh.origin_lon) as f32, (y - sh.origin_lat) as f32, c * p.lift]
}

pub fn build(
    g: &Graph,
    sh: &Shapes,
    node_cost: &[f32],
    place_cost: &[f32],
    places: &[(f64, f64)],
    origin: u32,
    p: &ViewParams,
) -> ViewBuffers {
    let on = &g.nodes[origin as usize];
    let o = (on.lon, on.lat, on.lat.to_radians().cos());
    let mut b = ViewBuffers { origin_lon: sh.origin_lon, origin_lat: sh.origin_lat, ..Default::default() };
    for (i, e) in g.edges.iter().enumerate() {
        let (cf, ct) = (node_cost[e.from as usize], node_cost[e.to as usize]);
        if !cf.is_finite() && !ct.is_finite() {
            continue;
        }
        if p.hide_beyond >= 1.0 && !(cf <= p.budget_s) && !(ct <= p.budget_s) {
            continue;
        }
        b.start_indices.push(b.vertex_cost.len() as u32);
        b.path_edge.push(i as u32);
        for v in sh.start[i] as usize..sh.start[i + 1] as usize {
            let c = vertex_cost(cf, ct, sh.frac[v]);
            let (pc, col) = shown(c, p);
            b.positions.extend(blend(sh, sh.lon[v], sh.lat[v], pc, o, p));
            b.colors.extend(col);
            b.vertex_cost.push(c);
        }
    }
    for (&(lon, lat), &c) in places.iter().zip(place_cost) {
        let (pc, col) = shown(c, p);
        if c.is_finite() {
            b.place_positions.extend(blend(sh, lon, lat, pc, o, p));
        } else {
            b.place_positions.extend([(lon - sh.origin_lon) as f32, (lat - sh.origin_lat) as f32, 0.0]);
        }
        b.place_colors.extend(col);
    }
    b
}

/// Circle of `radius_m` around node `origin` as (dLon, dLat) offsets from the bbox centre, closed (steps + 1 points).
pub fn ring(g: &Graph, sh: &Shapes, origin: u32, radius_m: f32, steps: usize) -> Vec<f32> {
    let on = &g.nodes[origin as usize];
    let k = on.lat.to_radians().cos();
    let mut out = Vec::with_capacity((steps + 1) * 2);
    for i in 0..=steps {
        let a = i as f64 / steps as f64 * std::f64::consts::TAU;
        let lon = on.lon + a.sin() * radius_m as f64 / (k * M_PER_DEG_LON_EQ);
        let lat = on.lat + a.cos() * radius_m as f64 / M_PER_DEG_LAT;
        out.extend([(lon - sh.origin_lon) as f32, (lat - sh.origin_lat) as f32]);
    }
    out
}

/// One point where the cost field crosses a level: on edge `edge` at `frac` (0..1 by length) of the way from `from` to `to`.
#[derive(Debug, Clone, Copy)]
pub struct ContourPt {
    pub lon: f64,
    pub lat: f64,
    pub edge: u32,
    pub frac: f32,
}

/// Bearing bins for the isochrone outline (2 degrees each).
pub const ISO_BINS: usize = 180;

/// Isochrone at cost `c` (s): along every edge whose end costs straddle `c`, the point where the (linearly
/// interpolated, as in `vertex_cost`) cost equals `c`. Then, per bearing bin from `origin`, only the farthest
/// such point, in bearing order: the outer reach of `c`, a simple closed outline (holes are ignored).
pub fn contour(g: &Graph, sh: &Shapes, node_cost: &[f32], origin: u32, c: f32) -> Vec<ContourPt> {
    let on = &g.nodes[origin as usize];
    let k = on.lat.to_radians().cos();
    let mut best: Vec<Option<(f64, ContourPt)>> = vec![None; ISO_BINS];
    for (i, e) in g.edges.iter().enumerate() {
        let (cf, ct) = (node_cost[e.from as usize], node_cost[e.to as usize]);
        if !cf.is_finite() || !ct.is_finite() || (cf - c) * (ct - c) > 0.0 || cf == ct {
            continue;
        }
        let s = (c - cf) / (ct - cf); // where vertex_cost(cf, ct, s) == c
        // the geometry segment containing fraction s
        let (a, b) = (sh.start[i] as usize, sh.start[i + 1] as usize);
        let mut v = a;
        while v + 2 < b && sh.frac[v + 1] < s {
            v += 1;
        }
        let (f0, f1) = (sh.frac[v], sh.frac[v + 1]);
        let u = if f1 > f0 { ((s - f0) / (f1 - f0)) as f64 } else { 0.0 };
        let lon = sh.lon[v] + (sh.lon[v + 1] - sh.lon[v]) * u;
        let lat = sh.lat[v] + (sh.lat[v + 1] - sh.lat[v]) * u;
        let (dx, dy) = ((lon - on.lon) * k * M_PER_DEG_LON_EQ, (lat - on.lat) * M_PER_DEG_LAT);
        let d = dx.hypot(dy);
        let bin = (((dx.atan2(dy) + std::f64::consts::TAU) % std::f64::consts::TAU) / std::f64::consts::TAU * ISO_BINS as f64) as usize % ISO_BINS;
        if best[bin].is_none_or(|(bd, _)| d > bd) {
            best[bin] = Some((d, ContourPt { lon, lat, edge: i as u32, frac: s }));
        }
    }
    best.into_iter().flatten().map(|(_, p)| p).collect()
}

/// The isochrone of cost `c` as a closed path of xyz offsets (as in `build`), under the view parameters.
/// Every point has cost `c`, so it goes through the same blend as the streets: at warp 1 it lies on the circle
/// of radius c x metres_per_s (bearing kept), and in between it morphs. Bearing gaps wider than one bin are
/// filled by interpolating distance linearly in polar coordinates (so the warp circle stays round). Empty if
/// nothing crosses `c`.
pub fn isochrone(g: &Graph, sh: &Shapes, node_cost: &[f32], origin: u32, c: f32, p: &ViewParams) -> Vec<f32> {
    use std::f64::consts::TAU;
    let pts = contour(g, sh, node_cost, origin, c);
    let on = &g.nodes[origin as usize];
    let k = on.lat.to_radians().cos();
    let o = (on.lon, on.lat, k);
    // (bearing, metres) of each point
    let polar: Vec<(f64, f64)> = pts
        .iter()
        .map(|q| {
            let (dx, dy) = ((q.lon - on.lon) * k * M_PER_DEG_LON_EQ, (q.lat - on.lat) * M_PER_DEG_LAT);
            (dx.atan2(dy).rem_euclid(TAU), dx.hypot(dy))
        })
        .collect();
    let step = TAU / ISO_BINS as f64;
    let mut out = Vec::with_capacity((ISO_BINS + 1) * 3);
    let n = pts.len();
    for i in 0..n {
        out.extend(blend(sh, pts[i].lon, pts[i].lat, c, o, p));
        let ((b0, d0), (b1, d1)) = (polar[i], polar[(i + 1) % n]);
        let gap = (b1 - b0).rem_euclid(TAU);
        let gap = if n == 1 { TAU } else { gap };
        let fill = (gap / step).floor() as usize;
        for j in 1..fill {
            let t = j as f64 / fill as f64;
            let (b, d) = (b0 + gap * t, d0 + (d1 - d0) * t);
            let (lon, lat) = (on.lon + b.sin() * d / (k * M_PER_DEG_LON_EQ), on.lat + b.cos() * d / M_PER_DEG_LAT);
            out.extend(blend(sh, lon, lat, c, o, p));
        }
    }
    if n > 0 {
        out.extend(blend(sh, pts[0].lon, pts[0].lat, c, o, p));
    }
    out
}
