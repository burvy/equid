//! Task 028 examples on data/princeton (fields from task 027): for each factor preset, a start/end pair where
//! the preset route differs from the plain walk and avoids (or finds) the factor. Run with --nocapture.

use engine::features::feature_index;
use engine::metric::{presets, Metric};
use engine::Core;

fn core() -> Option<Core> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/princeton");
    let g = std::fs::read_to_string(format!("{dir}/graph.json")).ok()?;
    Core::load(&g, &std::fs::read_to_string(format!("{dir}/places.json")).ok()?).ok()
}

/// (preset id, measured feature, higher-is-worse).
const CASES: &[(&str, &str)] = &[("peaceful", "dull"), ("flood", "flood"), ("crossings", "crossing"), ("sidewalk-only", "no_sidewalk")];

#[test]
#[ignore = "slow (~1 min): cargo test -p engine --test factor_examples -- --ignored --nocapture"]
fn factor_presets_change_princeton_routes() {
    let Some(core) = core() else { return };
    let ps = presets();
    let n = core.graph.nodes.len() as u32;
    let step = (n / 8).max(1) as usize;
    for (pid, feat) in CASES {
        // the plain-walk comparison route, with a negligible weight so explain reports the measured feature
        let walk = Metric::from_json(&format!(r#"{{"weights":{{"time":1,"{feat}":0.000001}}}}"#)).unwrap();
        // the preset route measured on the same feature: add the same negligible weight when the preset lacks it
        // 'sidewalk-only' is not a preset: time + 2 s per metre of road without a sidewalk
        let mut spec = match ps.iter().find(|p| p.id == *pid) {
            Some(p) => p.metric.clone(),
            None => serde_json::from_str(&format!(r#"{{"weights":{{"time":1,"{feat}":2}}}}"#)).unwrap(),
        };
        spec.weights.entry((*feat).into()).or_insert(0.000001);
        let measured = Metric::compile(&spec).unwrap();
        let _ = feature_index(feat).unwrap();
        let raw = |e: &engine::Explain| e.terms.iter().find(|t| t.0 == *feat).map_or(0.0, |t| t.1);
        let mut best: Option<(f32, u32, u32, f32, f32, f32, f32)> = None;
        // starts/ends: an even sample, plus endpoints of edges that actually carry the feature (sparse data)
        let fi = feature_index(feat).unwrap();
        let mut pts: Vec<u32> = (0..n).step_by(step).collect();
        let hot: Vec<u32> = core
            .graph
            .edges
            .iter()
            .filter(|e| engine::features::edge_features(e, true, engine::features::Mode::Foot, &core.graph.meta, 780.0)[fi] > 0.0)
            .flat_map(|e| [e.from, e.to])
            .collect();
        pts.extend(hot.iter().step_by((hot.len() / 12).max(1)));
        for &a in &pts {
            for &b in &pts {
                if a == b {
                    continue;
                }
                let (Ok(w), Ok(r)) = (core.explain(a, &walk, 780.0, b), core.explain(a, &measured, 780.0, b)) else { continue };
                let gain = raw(&w) - raw(&r);
                if w.length_m > 400.0 && w.length_m < 3000.0 && gain > best.map_or(0.0, |x| x.0) {
                    best = Some((gain, a, b, w.length_m, r.length_m, raw(&w), raw(&r)));
                }
            }
        }
        match best {
            Some((g, a, b, wl, rl, wr, rr)) => println!(
                "EXAMPLE {pid}/{feat} {a}->{b}: walk {wl:.0} m ({feat} {wr:.1}); preset {rl:.0} m ({feat} {rr:.1}); {g:.1} less"
            ),
            None => println!("EXAMPLE {pid}/{feat}: no pair among {} starts changes (data too sparse)", n as usize / step),
        }
    }
}
