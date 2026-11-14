//! Task 015: open-vocabulary objects -> per-edge `near:<label>` features.

use engine::metric::Metric;
use engine::Core;

const G: &str = include_str!("../../data/fixtures/graph.json");
const P: &str = include_str!("../../data/fixtures/places.json");

fn core() -> Core {
    Core::load(G, P).unwrap()
}

#[test]
fn near_distance_is_zero_on_the_edge_and_capped_far_away() {
    let mut c = core();
    // a playground exactly at node 5 (touches edges 3, 4, 10, 14)
    let n5 = (c.graph.nodes[5].lon, c.graph.nodes[5].lat);
    c.add_objects("Playground ", &[n5]).unwrap();
    assert_eq!(c.labels().len(), 1);
    assert_eq!(c.labels()[0].label, "playground");
    let m = Metric::from_json(r#"{"weights":{"time":0,"near:playground":1}}"#).unwrap();
    let costs = c.arc_costs(&m, 720.0);
    let touching = [3usize, 4, 10, 14];
    for i in 0..c.graph.edges.len() {
        let cost = costs[2 * i];
        if touching.contains(&i) {
            assert!(cost < 1e-2, "edge {i} touches the object but costs {cost}");
        } else {
            assert!(cost > 0.0 && cost <= c.graph.edges[i].length_m + 1e-3, "edge {i}: {cost}");
        }
    }
    // edge 8 (10 - 11): its nearest point is node 10, ~141 m from node 5: cost = length x 141 / 500 ~ 28
    assert!((costs[16] - 28.2).abs() < 2.0, "{}", costs[16]);
}

#[test]
fn weighting_a_label_changes_the_route() {
    let mut c = core();
    // objects along the top row (nodes 0..3): weighting the DISTANCE to fountains pulls routes toward them
    let pts: Vec<(f64, f64)> = (0..4).map(|i| (c.graph.nodes[i].lon, c.graph.nodes[i].lat)).collect();
    c.add_objects("fountain", &pts).unwrap();
    let plain = Metric::from_json(r#"{"weights":{"time":1}}"#).unwrap();
    let avoid = Metric::from_json(r#"{"weights":{"time":1,"near:fountain":60}}"#).unwrap();
    let a = c.field(4, &plain, 720.0).unwrap().node_cost[7];
    let b = c.field(4, &avoid, 720.0).unwrap().node_cost[7];
    assert!(b > a, "penalty must raise the cost: {a} vs {b}");
    // and costs stay non-negative / explain adds up, including the dynamic term
    let ex = c.explain(4, &avoid, 720.0, 7).unwrap();
    let sum: f32 = ex.terms.iter().map(|t| t.3).sum();
    assert!((sum - b).abs() < 1e-2 * b.max(1.0), "{sum} vs {b}");
    // a heavy weight on the distance to fountains pulls the route along the fountains (the top row): 4-0-1-2-3-7
    assert_eq!(ex.path, vec![4, 0, 1, 2, 3, 7]);
    // a light one keeps the short route and shows the near term in the breakdown
    let light = Metric::from_json(r#"{"weights":{"time":1,"near:fountain":0.1}}"#).unwrap();
    let ex = c.explain(4, &light, 720.0, 7).unwrap();
    assert!(ex.terms.iter().any(|t| t.0 == "near:fountain"), "{:?}", ex.terms);
}

#[test]
fn unknown_label_is_an_error_not_silently_free() {
    let c = core();
    let m = Metric::from_json(r#"{"weights":{"time":1,"near:trash can":1}}"#).unwrap();
    let e = c.field(0, &m, 720.0).unwrap_err();
    assert!(e.contains("trash can"), "{e}");
}

#[test]
fn builtin_near_labels_alias_the_static_features_and_forbid_is_rejected() {
    let c = core();
    let a = Metric::from_json(r#"{"weights":{"near:bench":2}}"#).unwrap();
    let b = Metric::from_json(r#"{"weights":{"bench_gap":2}}"#).unwrap();
    assert_eq!(c.arc_costs(&a, 0.0), c.arc_costs(&b, 0.0));
    assert!(Metric::from_json(r#"{"weights":{"near:":1}}"#).is_err());
    assert!(Metric::from_json(r#"{"weights":{"near:x":-1}}"#).is_err());
    assert!(Metric::from_json(r#"{"forbid":["near:x"]}"#).is_err());
}

#[test]
fn load_objects_json_groups_by_label() {
    let mut c = core();
    let n = c
        .load_objects(r#"{"version":1,"objects":[{"label":"Crosswalk","lon":-74.662,"lat":40.348,"src":"vexcel","conf":0.8},
            {"label":"crosswalk","lon":-74.660,"lat":40.349},{"label":"bike rack","lon":-74.661,"lat":40.3485}]}"#)
        .unwrap();
    assert_eq!(n, 3);
    let l = c.labels();
    assert_eq!(l.iter().map(|x| (x.label.as_str(), x.objects)).collect::<Vec<_>>(), vec![("bike rack", 1), ("crosswalk", 2)]);
    assert!(c.load_objects("{").is_err());
}
