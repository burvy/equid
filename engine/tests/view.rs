//! Task 016: view buffers (map / warp / 3D) built in Rust.

use engine::metric::Metric;
use engine::view::ViewParams;
use engine::Core;
use std::time::Instant;

const G: &str = include_str!("../../data/fixtures/graph.json");
const P: &str = include_str!("../../data/fixtures/places.json");
const M_LAT: f64 = 110_540.0;
const M_LON: f64 = 111_320.0;

fn params(warp: f32) -> ViewParams {
    ViewParams { warp, lift: 2.0, metres_per_s: 1.5, budget_s: 600.0 }
}

/// Real (lon, lat) of every vertex of the paths in `b`, in buffer order.
fn real_vertices(c: &Core, path_edge: &[u32]) -> Vec<(f64, f64)> {
    path_edge.iter().flat_map(|&e| c.graph.edges[e as usize].geom.iter().map(|p| (p[0], p[1]))).collect()
}

#[test]
fn warp_keeps_bearing_and_radius_is_cost() {
    let c = Core::load(G, P).unwrap();
    let m = Metric::from_json(r#"{"weights":{"time":1,"climb":10}}"#).unwrap();
    let origin = 5u32;
    let f = c.field(origin, &m, 720.0).unwrap();
    let b = c.view_buffers(&f, origin, &params(1.0)).unwrap();
    let on = &c.graph.nodes[origin as usize];
    let k = on.lat.to_radians().cos();
    let real = real_vertices(&c, &b.path_edge);
    assert_eq!(real.len(), b.vertex_cost.len());
    let mut checked = 0;
    for (i, &(lon, lat)) in real.iter().enumerate() {
        let (dx, dy) = ((lon - on.lon) * k * M_LON, (lat - on.lat) * M_LAT);
        if dx.hypot(dy) < 1.0 {
            continue;
        }
        let (wlon, wlat) = (b.origin_lon + b.positions[3 * i] as f64, b.origin_lat + b.positions[3 * i + 1] as f64);
        let (wx, wy) = ((wlon - on.lon) * k * M_LON, (wlat - on.lat) * M_LAT);
        // bearing kept: warped vector is parallel to the real one, same direction
        let cross = dx * wy - dy * wx;
        let dot = dx * wx + dy * wy;
        assert!(cross.abs() < 1e-3 * dx.hypot(dy) * wx.hypot(wy).max(1.0), "vertex {i} bearing changed");
        assert!(dot >= 0.0, "vertex {i} flipped");
        // radius = cost x scale
        let want = b.vertex_cost[i] as f64 * 1.5;
        assert!((wx.hypot(wy) - want).abs() < 0.05, "vertex {i}: r {} want {want}", wx.hypot(wy));
        checked += 1;
    }
    assert!(checked > 20);
}

#[test]
fn warp_radius_is_monotone_in_cost() {
    let c = Core::load(G, P).unwrap();
    let m = Metric::from_json(r#"{"weights":{"time":1,"steep":5}}"#).unwrap();
    let f = c.field(0, &m, 720.0).unwrap();
    let b = c.view_buffers(&f, 0, &params(1.0)).unwrap();
    let on = &c.graph.nodes[0];
    let k = on.lat.to_radians().cos();
    let mut rc: Vec<(f32, f64)> = (0..b.vertex_cost.len())
        .map(|i| {
            let (wx, wy) = (
                (b.origin_lon + b.positions[3 * i] as f64 - on.lon) * k * M_LON,
                (b.origin_lat + b.positions[3 * i + 1] as f64 - on.lat) * M_LAT,
            );
            (b.vertex_cost[i], wx.hypot(wy))
        })
        .collect();
    rc.sort_by(|a, b| a.0.total_cmp(&b.0));
    for w in rc.windows(2) {
        assert!(w[1].1 + 0.05 >= w[0].1, "radius not monotone in cost: {w:?}");
    }
}

#[test]
fn map_view_is_real_positions_and_3d_lifts_by_cost() {
    let c = Core::load(G, P).unwrap();
    let m = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let f = c.field(0, &m, 720.0).unwrap();
    let b = c.view_buffers(&f, 0, &params(0.0)).unwrap();
    let real = real_vertices(&c, &b.path_edge);
    for (i, &(lon, lat)) in real.iter().enumerate() {
        assert!((b.origin_lon + b.positions[3 * i] as f64 - lon).abs() < 1e-6);
        assert!((b.origin_lat + b.positions[3 * i + 1] as f64 - lat).abs() < 1e-6);
        assert!((b.positions[3 * i + 2] - 2.0 * b.vertex_cost[i]).abs() < 1e-3);
    }
}

#[test]
fn layout_matches_deck_binary_paths() {
    let c = Core::load(G, P).unwrap();
    let m = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let f = c.field(0, &m, 720.0).unwrap();
    let b = c.view_buffers(&f, 0, &params(0.5)).unwrap();
    let n = b.vertex_cost.len();
    assert_eq!(b.positions.len(), 3 * n);
    assert_eq!(b.colors.len(), 4 * n);
    assert_eq!(b.start_indices.len(), b.path_edge.len());
    assert_eq!(b.start_indices[0], 0);
    assert!(b.start_indices.windows(2).all(|w| w[0] < w[1]) && (*b.start_indices.last().unwrap() as usize) < n);
    assert_eq!(b.path_edge.len(), c.graph.edges.len()); // everything reachable in the fixture
    assert_eq!(b.place_positions.len(), 3 * c.places.places.len());
    assert_eq!(b.place_colors.len(), 4 * c.places.places.len());
    assert!(c.view_buffers(&f, 99, &params(0.0)).is_err());
    let ring = c.ring(0, 100.0).unwrap();
    assert_eq!(ring.len(), 97 * 2);
    assert_eq!(ring[..2], ring[96 * 2..]);
}

#[test]
fn unreachable_edges_are_omitted_and_places_greyed() {
    let mut v: serde_json::Value = serde_json::from_str(G).unwrap();
    for i in [8, 16] {
        v["edges"][i]["foot"] = false.into();
    }
    let c = Core::load(&v.to_string(), P).unwrap();
    let m = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let f = c.field(0, &m, 720.0).unwrap();
    let b = c.view_buffers(&f, 0, &params(1.0)).unwrap();
    // edges 8 and 16 touch node 11 but their other end (10, 7) is reachable, so they stay
    // (the far end takes its neighbour's cost); an edge with BOTH ends unreachable is dropped
    assert_eq!(b.path_edge.len(), c.graph.edges.len());
    for (i, p) in c.places.places.iter().enumerate() {
        if p.node == 11 {
            assert_eq!(b.place_colors[4 * i + 3], 60);
            assert_eq!(b.place_positions[3 * i + 2], 0.0);
        }
    }
}

/// ~20k-edge grid with 4 vertices per edge; prints ms per call
/// (run `cargo test -p engine --release --test view -- --nocapture timing`).
#[test]
fn timing_on_20k_edges() {
    let (w, h) = (100usize, 100usize);
    let mut nodes = vec![];
    for y in 0..h {
        for x in 0..w {
            nodes.push(serde_json::json!({"lon": -74.66 + x as f64 * 0.0003, "lat": 40.34 + y as f64 * 0.0003, "elev_m": 10.0}));
        }
    }
    let ll = |i: usize| (-74.66 + (i % w) as f64 * 0.0003, 40.34 + (i / w) as f64 * 0.0003);
    let mut edges = vec![];
    let mut add = |a: usize, b: usize| {
        let ((x0, y0), (x1, y1)) = (ll(a), ll(b));
        let g: Vec<_> = (0..4).map(|j| { let t = j as f64 / 3.0; [x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, 10.0] }).collect();
        edges.push(serde_json::json!({"from": a, "to": b, "length_m": 33.0, "geom": g, "climb_m": 0.0, "descent_m": 0.0,
            "max_grade_pct": 0.0, "highway": "residential", "foot": true, "bike": true}));
    };
    for y in 0..h {
        for x in 0..w {
            if x + 1 < w { add(y * w + x, y * w + x + 1); }
            if y + 1 < h { add(y * w + x, (y + 1) * w + x); }
        }
    }
    let g = serde_json::json!({"meta": {"version":1,"region":"s","bbox":[-74.66,40.34,-74.63,40.37],"tz":"UTC","date":"2026-11-14",
        "sunrise_min":395,"sunset_min":1004,"slot_min":30,"slots":48}, "nodes": nodes, "edges": edges});
    let c = Core::load(&g.to_string(), r#"{"version":1,"places":[]}"#).unwrap();
    let m = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let f = c.field(5050, &m, 720.0).unwrap();
    let _ = c.view_buffers(&f, 5050, &params(1.0)).unwrap();
    let n = 20;
    let t0 = Instant::now();
    for _ in 0..n {
        let b = c.view_buffers(&f, 5050, &params(0.7)).unwrap();
        assert!(b.vertex_cost.len() > 70_000);
    }
    println!(
        "view_buffers on {} edges ({} vertices): {:.2} ms per call ({})",
        c.graph.edges.len(),
        c.graph.edges.len() * 4,
        t0.elapsed().as_secs_f64() * 1000.0 / n as f64,
        if cfg!(debug_assertions) { "debug" } else { "release" }
    );
}
