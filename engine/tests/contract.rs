//! The fixture files parse into the contract types, and the metric invariants hold on them.

use engine::features::{edge_features, Mode, FEATURES, N_FEATURES};
use engine::metric::{presets, Metric, MetricSpec};
use engine::Core;

fn fixture() -> Core {
    let g = include_str!("../../data/fixtures/graph.json");
    let p = include_str!("../../data/fixtures/places.json");
    Core::load(g, p).expect("fixture loads")
}

#[test]
fn fixture_parses_and_validates() {
    let c = fixture();
    assert_eq!(c.graph.nodes.len(), 12);
    assert_eq!(c.graph.edges.len(), 17);
    assert_eq!(c.places.places.len(), 6);
}

#[test]
fn features_are_non_negative_both_directions_all_day() {
    let c = fixture();
    for e in &c.graph.edges {
        for forward in [true, false] {
            for mode in [Mode::Foot, Mode::Bike] {
                for t in (0..1440).step_by(30) {
                    let f = edge_features(e, forward, mode, &c.graph.meta, t as f32);
                    assert!(f.iter().all(|v| *v >= 0.0 && v.is_finite()), "{f:?}");
                }
            }
        }
    }
}

#[test]
fn all_presets_compile_and_give_non_negative_costs() {
    let c = fixture();
    let ps = presets();
    assert!(ps.len() >= 5);
    for p in &ps {
        let m = Metric::compile(&p.metric).unwrap_or_else(|e| panic!("preset {}: {e}", p.id));
        for e in &c.graph.edges {
            if let Some(cost) = m.edge_cost(e, true, &c.graph.meta, 12.0 * 60.0) {
                assert!(cost >= 0.0, "preset {} cost {cost}", p.id);
            }
        }
    }
}

#[test]
fn wheelchair_cannot_use_steps() {
    let c = fixture();
    let wc = presets().into_iter().find(|p| p.id == "wheelchair").unwrap();
    let m = Metric::compile(&wc.metric).unwrap();
    let steps = c.graph.edges.iter().find(|e| e.steps).unwrap();
    assert_eq!(m.edge_cost(steps, true, &c.graph.meta, 720.0), None);
}

#[test]
fn negative_or_unknown_weights_rejected() {
    assert!(Metric::from_json(r#"{"weights":{"time":-1}}"#).is_err());
    assert!(Metric::from_json(r#"{"weights":{"teleport":1}}"#).is_err());
    assert!(Metric::from_json(r#"{"mode":"bike","weights":{"time":1},"forbid":["steps"]}"#).is_ok());
    let _ = MetricSpec::default();
    assert_eq!(FEATURES.len(), N_FEATURES);
}

#[test]
fn unlit_only_counts_at_night_and_sun_only_by_day() {
    let c = fixture();
    let meta = &c.graph.meta;
    let dark = c.graph.edges.iter().find(|e| e.lit == Some(false)).unwrap();
    let i_unlit = engine::features::feature_index("unlit").unwrap();
    let i_sun = engine::features::feature_index("sun").unwrap();
    let noon = edge_features(dark, true, Mode::Foot, meta, 720.0);
    let night = edge_features(dark, true, Mode::Foot, meta, 22.0 * 60.0);
    assert_eq!(noon[i_unlit], 0.0);
    assert!(night[i_unlit] > 0.0);
    assert!(noon[i_sun] > 0.0);
    assert_eq!(night[i_sun], 0.0);
}

#[test]
fn field_shapes() {
    let c = fixture();
    let m = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let o = c.nearest_node(-74.662, 40.348);
    assert_eq!(o, 0);
    let r = c.field(o, &m, 600.0).unwrap();
    assert_eq!(r.node_cost.len(), 12);
    assert_eq!(r.place_cost.len(), 6);
    assert_eq!(r.node_cost[0], 0.0);
    assert!(r.node_cost.iter().all(|v| *v >= 0.0));
    assert!(c.field(99, &m, 600.0).is_err());
}
