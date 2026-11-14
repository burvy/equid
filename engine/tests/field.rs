//! Task 003: `field` returns true shortest-path costs (petgraph Dijkstra over Metric::edge_cost).

use engine::metric::{presets, Metric};
use engine::Core;
use std::time::Instant;

const G: &str = include_str!("../../data/fixtures/graph.json");
const P: &str = include_str!("../../data/fixtures/places.json");
const WALK_S_PER_EDGE: f32 = 100.1 / 1.34; // every fixture edge is 100.1 m

fn fixture() -> Core {
    Core::load(G, P).unwrap()
}
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3 * b.abs().max(1.0)
}
fn preset(id: &str) -> Metric {
    Metric::compile(&presets().into_iter().find(|p| p.id == id).unwrap().metric).unwrap()
}

#[test]
fn time_only_walk_matches_hand_computed_path() {
    // 4 x 3 grid, nodes 0..3 top row, 8..11 bottom row. 0 -> 11 is 3 + 2 = 5 edges whichever way you go.
    let c = fixture();
    let m = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let r = c.field(0, &m, 720.0).unwrap();
    assert_eq!(r.node_cost[0], 0.0);
    assert!(near(r.node_cost[1], WALK_S_PER_EDGE));
    assert!(near(r.node_cost[4], WALK_S_PER_EDGE));
    assert!(near(r.node_cost[6], 3.0 * WALK_S_PER_EDGE));
    assert!(near(r.node_cost[11], 5.0 * WALK_S_PER_EDGE));
}

#[test]
fn bike_is_faster_and_respects_oneway() {
    let c = fixture();
    let m = Metric::from_json(r#"{"mode":"bike","weights":{"time":1}}"#).unwrap();
    let r = c.field(0, &m, 720.0).unwrap();
    assert!(near(r.node_cost[1], 100.1 / 4.44));
    // edge 2 (2-3) is oneway_bike = 1 (2 -> 3 only): riding 3 -> 2 must not use it
    let from3 = c.field(3, &m, 720.0).unwrap();
    assert!(from3.node_cost[2] > 100.1 / 4.44 * 1.5, "went the wrong way down a oneway");
    assert!(from3.node_cost[2].is_finite());
}

#[test]
fn wheelchair_routes_around_the_stairs() {
    // Stairs are edge 14 (5 - 9). Walking 5 -> 9 takes it; with steps forbidden it goes 5-4-8-9 (3 edges).
    let c = fixture();
    let walk = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let chair = Metric::from_json(r#"{"weights":{"time":1},"forbid":["steps"]}"#).unwrap();
    assert!(near(c.field(5, &walk, 720.0).unwrap().node_cost[9], WALK_S_PER_EDGE));
    assert!(near(c.field(5, &chair, 720.0).unwrap().node_cost[9], 3.0 * WALK_S_PER_EDGE));
    // and the shipped preset agrees that it is a detour
    let wc = c.field(5, &preset("wheelchair"), 720.0).unwrap().node_cost[9];
    assert!(wc > 3.0 * WALK_S_PER_EDGE);
}

#[test]
fn night_changes_cost_of_the_unlit_footway() {
    // Edge 3 (4 - 5) is an unlit footway. By day 4 -> 5 is one edge; at night with an unlit penalty it detours.
    let c = fixture();
    let m = preset("night");
    let noon = c.field(4, &m, 12.0 * 60.0).unwrap().node_cost[5];
    let night = c.field(4, &m, 22.0 * 60.0).unwrap().node_cost[5];
    assert!(night > noon * 1.5, "noon {noon} night {night}");
}

#[test]
fn unreachable_is_infinity() {
    // cut node 11 off (edges 8 and 16 touch it) by making them non-walkable
    let mut v: serde_json::Value = serde_json::from_str(G).unwrap();
    for i in [8, 16] {
        v["edges"][i]["foot"] = false.into();
    }
    let c = Core::load(&v.to_string(), P).unwrap();
    let m = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let r = c.field(0, &m, 720.0).unwrap();
    assert!(r.node_cost[11].is_infinite());
    assert!(r.node_cost[10].is_finite());
    // places snapped to node 11 inherit the Infinity
    for (p, pc) in c.places.places.iter().zip(&r.place_cost) {
        assert_eq!(pc.is_infinite(), p.node == 11);
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
}

/// Checks `d` is a true shortest-path array: d[origin]=0, no arc can be relaxed (triangle inequality),
/// and every finite node is reached by at least one tight arc (so nothing is cheaper than it should be).
fn assert_shortest(c: &Core, d: &[f32], origin: usize, costs: &[f32]) {
    assert_eq!(d[origin], 0.0);
    let mut tight = vec![false; d.len()];
    tight[origin] = true;
    for (i, e) in c.graph.edges.iter().enumerate() {
        for (k, (u, v)) in [(e.from as usize, e.to as usize), (e.to as usize, e.from as usize)].into_iter().enumerate() {
            let w = costs[2 * i + k];
            if w.is_finite() && d[u].is_finite() {
                assert!(d[v] <= d[u] + w + 1e-3 * (d[u] + w).max(1.0), "arc {u}->{v} relaxes: {} > {} + {w}", d[v], d[u]);
                if near(d[v], d[u] + w) {
                    tight[v] = true;
                }
            }
        }
    }
    for (n, dn) in d.iter().enumerate() {
        assert!(*dn >= 0.0);
        assert_eq!(tight[n], dn.is_finite(), "node {n}");
    }
}

#[test]
fn random_weights_give_non_negative_triangle_consistent_costs() {
    let c = fixture();
    let ids: Vec<&str> = engine::features::FEATURES.iter().map(|f| f.id).collect();
    let mut rng = Lcg(7);
    for _ in 0..300 {
        let mut w = serde_json::Map::new();
        for id in &ids {
            if rng.next() < 0.6 {
                w.insert((*id).into(), (rng.next() * 20.0).into());
            }
        }
        let mode = if rng.next() < 0.5 { "foot" } else { "bike" };
        let forbid: Vec<&str> = if rng.next() < 0.3 { vec!["steps"] } else { vec![] };
        let m = Metric::from_json(&serde_json::json!({"mode": mode, "weights": w, "forbid": forbid}).to_string()).unwrap();
        let t = rng.next() * 1440.0;
        let origin = (rng.next() * 12.0) as usize % 12;
        let r = c.field(origin as u32, &m, t).unwrap();
        assert_shortest(&c, &r.node_cost, origin, &c.arc_costs(&m, t));
    }
}

/// ~20k-edge synthetic grid; prints the time per `field`
/// (run `cargo test -p engine --release --test field -- --nocapture timing`).
#[test]
fn timing_on_20k_edges() {
    let (w, h) = (100usize, 100usize);
    let mut nodes = vec![];
    for y in 0..h {
        for x in 0..w {
            nodes.push(serde_json::json!({"lon": -74.66 + x as f64 * 0.0012, "lat": 40.34 + y as f64 * 0.0009, "elev_m": ((x * 7 + y * 3) % 20) as f64}));
        }
    }
    let mut edges = vec![];
    let surf = ["paved", "unpaved", "rough"];
    let mut add = |a: usize, b: usize, k: usize| {
        edges.push(serde_json::json!({
            "from": a, "to": b, "length_m": 100.0, "geom": [[0.0,0.0,0.0],[0.0,0.0,0.0]],
            "climb_m": (k % 5) as f64, "descent_m": (k % 3) as f64, "max_grade_pct": (k % 12) as f64,
            "highway": "residential", "foot": true, "bike": k % 4 != 0, "steps": k % 50 == 0,
            "surface": surf[k % 3], "lit": k % 5 != 0, "busy": k % 4, "lts": 1 + k % 4,
            "near_bench_m": (k * 13 % 500) as f64,
        }));
    };
    let mut k = 0;
    for y in 0..h {
        for x in 0..w {
            if x + 1 < w {
                add(y * w + x, y * w + x + 1, k);
                k += 1;
            }
            if y + 1 < h {
                add(y * w + x, (y + 1) * w + x, k);
                k += 1;
            }
        }
    }
    let g = serde_json::json!({"meta": {"version":1,"region":"synthetic","bbox":[0,0,1,1],"tz":"UTC","date":"2026-11-14",
        "sunrise_min":395,"sunset_min":1004,"slot_min":30,"slots":48}, "nodes": nodes, "edges": edges});
    let c = Core::load(&g.to_string(), r#"{"version":1,"places":[]}"#).unwrap();
    let m = preset("hiking");
    let _ = c.field(0, &m, 720.0).unwrap(); // warm up
    let n = 20;
    let t0 = Instant::now();
    for i in 0..n {
        let r = c.field((i * 487) as u32 % 10000, &m, 22.0 * 60.0).unwrap();
        assert!(r.node_cost.iter().filter(|v| v.is_finite()).count() > 9000);
    }
    let per = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!(
        "field on {} edges: {per:.2} ms per call (native, {} build)",
        c.graph.edges.len(),
        if cfg!(debug_assertions) { "debug" } else { "release" }
    );
}

#[test]
fn explain_terms_add_up_to_the_field_cost() {
    let c = fixture();
    for id in ["walk", "wheelchair", "night", "hiking"] {
        let m = preset(id);
        let f = c.field(0, &m, 22.0 * 60.0).unwrap();
        for target in 0..12u32 {
            let ex = c.explain(0, &m, 22.0 * 60.0, target).unwrap();
            let sum: f32 = ex.terms.iter().map(|t| t.3).sum();
            assert!(near(sum, f.node_cost[target as usize]), "{id} -> {target}: {sum} vs {}", f.node_cost[target as usize]);
            assert_eq!(ex.path[0], 0);
            assert_eq!(*ex.path.last().unwrap(), target);
        }
    }
}
