//! Task 028 factors: icy, flood, crossing, no_sidewalk, kerb, dull. Each is >= 0, zero when its data is missing,
//! and has a preset that changes the route.

use engine::features::{edge_features, feature_index, Mode, N_FEATURES};
use engine::graph::Edge;
use engine::metric::{presets, Metric};
use engine::Core;

fn fixture() -> Core {
    Core::load(include_str!("../../data/fixtures/graph.json"), include_str!("../../data/fixtures/places.json")).expect("fixture loads")
}

fn base_edge(c: &Core) -> Edge {
    let mut e = c.graph.edges[0].clone();
    e.max_grade_pct = 0.0;
    e.flood = None;
    e.crossings = 0;
    e.crossings_unsignalled = 0;
    e.sidewalk = None;
    e.kerb_raised = 0;
    e.green = None;
    e.water_view = None;
    e.shade = None;
    e.covered = false;
    e.tree = None;
    e.highway = "residential".into();
    e
}

fn f(e: &Edge, c: &Core, id: &str, t: f32) -> f32 {
    edge_features(e, true, Mode::Foot, &c.graph.meta, t)[feature_index(id).unwrap()]
}

#[test]
fn new_features_are_zero_without_their_data() {
    let c = fixture();
    let e = base_edge(&c);
    for id in ["icy", "flood", "crossing", "no_sidewalk", "kerb", "dull"] {
        assert_eq!(f(&e, &c, id, 720.0), 0.0, "{id}");
    }
}

#[test]
fn icy_needs_slope_and_shade_and_never_goes_negative() {
    let c = fixture();
    let mut e = base_edge(&c);
    e.length_m = 100.0;
    e.max_grade_pct = 7.0;
    e.tree = Some(1.0); // fully shaded, no slot data
    assert!((f(&e, &c, "icy", 720.0) - 100.0).abs() < 1e-3);
    e.tree = Some(0.0); // sunny hill: melts by day ...
    assert_eq!(f(&e, &c, "icy", 720.0), 0.0);
    assert!((f(&e, &c, "icy", 23.0 * 60.0) - 100.0).abs() < 1e-3); // ... but not at night
    e.tree = Some(1.0);
    e.max_grade_pct = 2.0; // shady flat: no ice
    assert_eq!(f(&e, &c, "icy", 720.0), 0.0);
    e.max_grade_pct = 5.0; // halfway up the ramp
    assert!((f(&e, &c, "icy", 720.0) - 50.0).abs() < 1e-3);
}

#[test]
fn crossing_kerb_sidewalk_dull_flood_values() {
    let c = fixture();
    let mut e = base_edge(&c);
    e.length_m = 80.0;
    e.crossings = 3;
    e.crossings_unsignalled = 1;
    assert!((f(&e, &c, "crossing", 720.0) - 1.6).abs() < 1e-5); // 1 + 2 x .3
    e.kerb_raised = 2;
    assert_eq!(f(&e, &c, "kerb", 720.0), 2.0);
    e.sidewalk = Some(true);
    assert_eq!(f(&e, &c, "no_sidewalk", 720.0), 0.0);
    e.sidewalk = Some(false);
    assert_eq!(f(&e, &c, "no_sidewalk", 720.0), 80.0);
    e.green = Some(0.25);
    e.water_view = Some(0.75);
    assert!((f(&e, &c, "dull", 720.0) - 20.0).abs() < 1e-4);
    e.flood = Some(0.5);
    assert_eq!(f(&e, &c, "flood", 720.0), 40.0);
}

#[test]
fn bad_factor_data_is_rejected() {
    let c = fixture();
    let mut g = c.graph.clone();
    g.edges[0].flood = Some(1.5);
    assert!(g.validate().is_err());
    let mut g = c.graph.clone();
    g.edges[0].crossings = 1;
    g.edges[0].crossings_unsignalled = 2;
    assert!(g.validate().is_err());
}

#[test]
fn every_preset_compiles_and_factor_presets_exist() {
    let ps = presets();
    for id in ["icy", "peaceful", "flood", "crossings", "wheelchair", "stroller"] {
        assert!(ps.iter().any(|p| p.id == id), "preset {id}");
    }
    for p in &ps {
        Metric::compile(&p.metric).unwrap_or_else(|e| panic!("{}: {e}", p.id));
    }
    assert_eq!(N_FEATURES, 19);
}

/// Princeton example: the icy preset on a winter morning (shade known from LiDAR) changes the route.
#[test]
fn icy_preset_changes_a_princeton_route() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/princeton");
    let (Ok(g), Ok(p)) = (std::fs::read_to_string(format!("{dir}/graph.json")), std::fs::read_to_string(format!("{dir}/places.json"))) else {
        return;
    };
    let core = Core::load(&g, &p).unwrap();
    let ps = presets();
    let get = |id: &str| Metric::compile(&ps.iter().find(|p| p.id == id).unwrap().metric).unwrap();
    let icy = get("icy");
    // the plain walk route, with a negligible icy weight so `explain` reports the icy metres along it
    let walk = Metric::from_json(r#"{"weights":{"time":1,"icy":0.000001}}"#).unwrap();
    let n = core.graph.nodes.len() as u32;
    let mut best: Option<(f32, u32, u32, f32, f32, f32)> = None; // gain in icy metres
    let i_icy = feature_index("icy").unwrap();
    let _ = i_icy;
    let icy_m = |e: &engine::Explain| e.terms.iter().find(|t| t.0 == "icy").map_or(0.0, |t| t.1);
    for a in (0..n).step_by((n as usize / 10).max(1)) {
        for b in (0..n).step_by((n as usize / 10).max(1)) {
            if a == b {
                continue;
            }
            let (Ok(w), Ok(r)) = (core.explain(a, &walk, 600.0, b), core.explain(a, &icy, 600.0, b)) else { continue };
            let gain = icy_m(&w) - icy_m(&r);
            if w.length_m < 3000.0 && gain > best.map_or(0.0, |x| x.0) {
                best = Some((gain, a, b, w.length_m, r.length_m, icy_m(&w)));
            }
        }
    }
    let (gain, a, b, wl, rl, wi) = best.expect("some pair has icy ground to avoid");
    println!("icy example {a}->{b}: walk {wl:.0} m with {wi:.0} icy m; icy preset {rl:.0} m, {gain:.0} icy m fewer");
    assert!(gain > 0.0);
}
