//! Task 023 Done-when: on data/houston, one Vexcel-detected label (parking lot, 15 points from live detect)
//! visibly changes a route when its weight is on vs off.

use engine::metric::Metric;
use engine::Core;

#[test]
fn near_parking_lot_changes_a_houston_route() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/houston");
    let Ok(g) = std::fs::read_to_string(format!("{dir}/graph.json")) else { return };
    let mut core = Core::load(&g, &std::fs::read_to_string(format!("{dir}/places.json")).unwrap()).unwrap();
    let objs = std::fs::read_to_string(format!("{dir}/objects.json")).unwrap();
    core.load_objects(&objs).unwrap();
    let lab = core.labels().into_iter().find(|l| l.label == "parking lot").expect("parking lot in objects.json");
    println!("parking lot: {} vexcel objects, {} edges within 100 m", lab.objects, lab.edges_near);

    // 'off' has a negligible weight only so that explain reports the metric on the same scale
    let off = Metric::from_json(r#"{"weights":{"time":1,"near:parking lot":0.000001}}"#).unwrap();
    let on = Metric::from_json(r#"{"weights":{"time":1,"near:parking lot":3}}"#).unwrap();
    let n = core.graph.nodes.len() as u32;
    let step = (n / 12).max(1);
    let near_raw = |e: &engine::Explain| e.terms.iter().find(|t| t.0 == "near:parking lot").map_or(0.0, |t| t.1);
    let mut best: Option<(f32, u32, u32, engine::Explain, engine::Explain)> = None;
    for a in (0..n).step_by(step as usize) {
        for b in (0..n).step_by(step as usize) {
            if a == b {
                continue;
            }
            let (Ok(w), Ok(r)) = (core.explain(a, &off, 600.0, b), core.explain(a, &on, 600.0, b)) else { continue };
            // proximity metres gained: near_raw is length x (distance / 500), so LOWER = closer to parking lots
            let gain = near_raw(&w) - near_raw(&r);
            if w.length_m < 2500.0 && gain > best.as_ref().map_or(0.0, |x| x.0) {
                best = Some((gain, a, b, w, r));
            }
        }
    }
    let (gain, a, b, w, r) = best.expect("some pair changes route");
    let shared = w.path.iter().filter(|x| r.path.contains(x)).count();
    println!(
        "houston {a}->{b}: weight off {:.0} m, {} nodes; weight on {:.0} m, {} nodes ({} shared); distance-to-parking cost {:.0} -> {:.0}",
        w.length_m, w.path.len(), r.length_m, r.path.len(), shared, near_raw(&w), near_raw(&r)
    );
    assert_ne!(w.path, r.path);
    assert!(gain > 0.0);
}
