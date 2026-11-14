//! Demo step: does the shade data change routes? Runs the engine's own "shade" preset
//! (time + 1 s per sunny metre) between a few Princeton landmarks at 13:00 and at 16:00 and prints
//! how the routes differ. Also appended to data/princeton/shade_stats.txt.

use engine::metric::Metric;
use engine::Core;
use std::fmt::Write as _;
use std::collections::HashMap;
use std::path::Path;

const SPOTS: [(&str, f64, f64); 6] = [
    ("Palmer Square", -74.6613, 40.3496),
    ("Princeton station (Dinky)", -74.6599, 40.3427),
    ("Princeton Battlefield", -74.6765, 40.3306),
    ("Community Park North", -74.6713, 40.3640),
    ("Lake Carnegie boathouse", -74.6440, 40.3410),
    ("Institute for Advanced Study", -74.6672, 40.3318),
];

fn preset(root: &Path, id: &str) -> Result<Metric, String> {
    let text = std::fs::read_to_string(root.join("data/presets.json")).map_err(|e| e.to_string())?;
    let all: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let p = all.as_array().and_then(|a| a.iter().find(|p| p["id"] == id)).ok_or(format!("preset {id} not found"))?;
    Metric::from_json(&p.to_string())
}

/// The pair whose 13:00 and 16:00 shade routes differ most: (label, walk path, shade@13 path, shade@16 path).
pub type Best = (String, Vec<u32>, Vec<u32>, Vec<u32>);

pub fn run(root: &Path, dir: &Path) -> Result<(String, Option<Best>), String> {
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).map_err(|e| e.to_string());
    let core = Core::load(&read("graph.json")?, &read("places.json")?)?;
    let shade = preset(root, "shade")?;
    let walk = preset(root, "walk")?;
    let mut out = String::new();
    let _ = writeln!(out, "route demo: preset 'shade' (time + 1 s per sunny metre) at 13:00 vs 16:00, walk route for reference");
    let mut by_ends: HashMap<(u32, u32), usize> = HashMap::new();
    for (i, e) in core.graph.edges.iter().enumerate() {
        let k = (e.from.min(e.to), e.from.max(e.to));
        let keep = by_ends.get(&k).is_some_and(|&j| core.graph.edges[j].length_m <= e.length_m);
        if !keep {
            by_ends.insert(k, i);
        }
    }
    let mut best: Option<(f64, Best)> = None;
    for a in 0..SPOTS.len() {
        for b in a + 1..SPOTS.len() {
            let (na, nb) = (core.nearest_node(SPOTS[a].1, SPOTS[a].2), core.nearest_node(SPOTS[b].1, SPOTS[b].2));
            let w = core.explain(na, &walk, 13.0 * 60.0, nb)?;
            let r13 = core.explain(na, &shade, 13.0 * 60.0, nb)?;
            let r16 = core.explain(na, &shade, 16.0 * 60.0, nb)?;
            // sunny metres along a route at time t: sum of length x (1 - shade[slot]), the engine's `sun` feature
            let sunny = |e: &engine::Explain, t: f32| sunny_m(&core, &by_ends, &e.path, t);
            let s13: std::collections::HashSet<u32> = r13.path.iter().copied().collect();
            let shared = r16.path.iter().filter(|n| s13.contains(n)).count() as f64 / r16.path.len().max(r13.path.len()) as f64;
            // sun along the WALK route at each time, to show what the shade route saves
            let w13 = sunny(&w, 780.0);
            let w16 = sunny(&w, 960.0);
            let line = format!(
                "  {} -> {}:\n    walk     {:5.0} m, sunny {:4.0} m at 13:00 / {:4.0} m at 16:00\n    shade@13 {:5.0} m, sunny {:4.0} m\n    shade@16 {:5.0} m, sunny {:4.0} m\n    the 13:00 and 16:00 shade routes share {:.0}% of their nodes\n",
                SPOTS[a].0, SPOTS[b].0, w.length_m, w13, w16, r13.length_m, sunny(&r13, 780.0), r16.length_m, sunny(&r16, 960.0), shared * 100.0
            );
            out.push_str(&line);
            if best.as_ref().is_none_or(|(s, _)| shared < *s) {
                best = Some((shared, (format!("{} -> {}", SPOTS[a].0, SPOTS[b].0), w.path.clone(), r13.path.clone(), r16.path.clone())));
            }
        }
    }
    if let Some((s, (name, ..))) = &best {
        let _ = writeln!(out, "  most different: {name} ({:.0}% shared)", s * 100.0);
    }
    Ok((out, best.map(|b| b.1)))
}

/// Sunny metres along a node path at local time t (the shortest edge between consecutive nodes).
fn sunny_m(core: &Core, by_ends: &HashMap<(u32, u32), usize>, path: &[u32], t: f32) -> f32 {
    let m = &core.graph.meta;
    let slot = ((t / m.slot_min as f32) as usize).min(m.slots as usize - 1);
    path.windows(2)
        .filter_map(|w| by_ends.get(&(w[0].min(w[1]), w[0].max(w[1]))))
        .map(|&i| {
            let e = &core.graph.edges[i];
            e.length_m * (1.0 - e.shade.as_ref().map(|s| s[slot]).unwrap_or(0.0))
        })
        .sum()
}
