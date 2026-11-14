//! Task 037: `explain` exposes the raw (unweighted) climb along the route, so 'catch the sunset' can use
//! Naismith walking time (length / speed + 6 s per metre climbed) under presets that don't weight climb.

use engine::metric::Metric;
use engine::Core;

fn m(json: &str) -> Metric {
    Metric::from_json(json).unwrap()
}

/// When climb is weighted, `climb_m` is exactly the climb term's raw amount on the same route.
#[test]
fn climb_m_matches_the_climb_term_when_weighted() {
    let c = Core::load(include_str!("../../data/fixtures/graph.json"), include_str!("../../data/fixtures/places.json")).unwrap();
    let met = m(r#"{"mode":"foot","weights":{"time":1,"climb":10}}"#);
    for target in 1..c.graph.nodes.len() as u32 {
        let Ok(ex) = c.explain(0, &met, 600.0, target) else { continue };
        let term = ex.terms.iter().find(|t| t.0 == "climb").map_or(0.0, |t| t.1);
        assert!((ex.climb_m - term).abs() <= 1e-3 * term.max(1.0), "{target}: climb_m {} vs term {term}", ex.climb_m);
    }
}

/// On real Princeton terrain with a time-only metric (climb weight 0, so no climb term), routes to the
/// sunset spots still report their climb, and Naismith makes the walk longer than flat time.
#[test]
fn hilly_route_reports_unweighted_climb() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/princeton");
    let Ok(g) = std::fs::read_to_string(format!("{dir}/graph.json")) else { return };
    let c = Core::load(&g, &std::fs::read_to_string(format!("{dir}/places.json")).unwrap()).unwrap();
    let time_only = m(r#"{"mode":"foot","weights":{"time":1}}"#);
    let tiny_climb = m(r#"{"mode":"foot","weights":{"time":1,"climb":0.000001}}"#);
    let origin = c.nearest_node(-74.6593, 40.3487); // Nassau Hall
    let speed = 1.3; // m/s, only for the printout
    let mut hilly = 0;
    for p in c.places.places.iter().filter(|p| p.category == "sunset") {
        let ex = c.explain(origin, &time_only, 1140.0, p.node).unwrap();
        assert!(ex.terms.iter().all(|t| t.0 != "climb"), "climb weight 0 -> no climb term");
        let flat = ex.length_m / speed;
        let naismith = flat + 6.0 * ex.climb_m;
        println!("{:>14}: {:5.0} m, climb {:4.1} m, flat {:4.1} min, Naismith {:4.1} min", p.id, ex.length_m, ex.climb_m, flat / 60.0, naismith / 60.0);
        // a negligible climb weight keeps (almost always) the same route and makes explain report the term
        let ex2 = c.explain(origin, &tiny_climb, 1140.0, p.node).unwrap();
        if ex2.path == ex.path {
            let term = ex2.terms.iter().find(|t| t.0 == "climb").map_or(0.0, |t| t.1);
            assert!((ex.climb_m - term).abs() <= 1e-3 * term.max(1.0), "{}: {} vs {term}", p.id, ex.climb_m);
        }
        if ex.climb_m > 1.0 {
            hilly += 1;
        }
    }
    assert!(hilly > 0, "expected some climb to the Princeton sunset spots");
}
