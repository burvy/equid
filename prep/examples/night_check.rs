//! Does the "night" preset actually prefer lit streets on the real data?
//! cargo run --release -p prep --example night_check
//! Compares the walk and night routes at 21:00 for a few named trips plus 300 random trips.

use engine::metric::{presets, Metric};
use engine::Core;

fn main() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/princeton/");
    let g = std::fs::read_to_string(format!("{root}graph.json")).unwrap();
    let p = std::fs::read_to_string(format!("{root}places.json")).unwrap();
    let core = Core::load(&g, &p).unwrap();
    let metric = |id: &str| Metric::compile(&presets().into_iter().find(|x| x.id == id).unwrap().metric).unwrap();
    let (walk, night) = (metric("walk"), metric("night"));
    let t = 21.0 * 60.0;

    // share of a route's length that is lit / known-unlit / unknown
    let shares = |path: &[u32]| {
        let (mut lit, mut dark, mut unk, mut tot) = (0.0, 0.0, 0.0, 0.0);
        for w in path.windows(2) {
            let e = core
                .graph
                .edges
                .iter()
                .filter(|e| (e.from == w[0] && e.to == w[1]) || (e.from == w[1] && e.to == w[0]))
                .min_by(|a, b| a.length_m.total_cmp(&b.length_m))
                .unwrap();
            tot += e.length_m;
            match e.lit {
                Some(true) => lit += e.length_m,
                Some(false) => dark += e.length_m,
                None => unk += e.length_m,
            }
        }
        (tot, lit / tot * 100.0, dark / tot * 100.0, unk / tot * 100.0)
    };
    let compare = |name: &str, a: u32, b: u32, print: bool| -> Option<(f32, f32, f32)> {
        let rw = core.explain(a, &walk, t, b).ok()?;
        let rn = core.explain(a, &night, t, b).ok()?;
        let (lw, litw, darkw, unkw) = shares(&rw.path);
        let (ln, litn, darkn, unkn) = shares(&rn.path);
        if print {
            println!("{name}");
            println!("  walk : {lw:6.0} m, lit {litw:4.0}%, unlit {darkw:4.0}%, unknown {unkw:4.0}%");
            println!("  night: {ln:6.0} m, lit {litn:4.0}%, unlit {darkn:4.0}%, unknown {unkn:4.0}%  (+{:.0}% distance)", (ln / lw - 1.0) * 100.0);
        }
        Some((litn - litw, unkn + darkn - unkw - darkw, ln / lw - 1.0))
    };
    let node = |lon, lat| core.nearest_node(lon, lat);
    let trips = [
        ("Princeton station (Dinky) -> Palmer Square", (-74.6603, 40.3426), (-74.6610, 40.3497)),
        ("Lake Carnegie boathouse -> Nassau Hall", (-74.6513, 40.3402), (-74.6593, 40.3487)),
        ("Institute for Advanced Study area -> Nassau St", (-74.6680, 40.3310 + 0.0025), (-74.6555, 40.3500)),
    ];
    for (name, a, b) in trips {
        compare(name, node(a.0, a.1), node(b.0, b.1), true);
    }
    // 300 deterministic pseudo-random trips (LCG), 0.8-3 km apart
    let n = core.graph.nodes.len() as u64;
    let mut x: u64 = 12345;
    let mut rnd = || {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((x >> 33) % n) as u32
    };
    let (mut cnt, mut changed, mut dlit, mut ddark, mut dlen) = (0, 0, 0.0, 0.0, 0.0);
    while cnt < 300 {
        let (a, b) = (rnd(), rnd());
        let (na, nb) = (&core.graph.nodes[a as usize], &core.graph.nodes[b as usize]);
        let d = engine::haversine_m(na.lon, na.lat, nb.lon, nb.lat);
        if !(800.0..3000.0).contains(&d) {
            continue;
        }
        if let Some((l, k, len)) = compare("", a, b, false) {
            cnt += 1;
            if l.abs() > 0.5 || k.abs() > 0.5 {
                changed += 1;
            }
            dlit += l;
            ddark += k;
            dlen += len;
        }
    }
    println!(
        "300 random trips at 21:00: night route differs on {changed}; on average lit share {:+.1} pts, unlit+unknown share {:+.1} pts, distance {:+.1}%",
        dlit / 300.0, ddark / 300.0, dlen / 300.0 * 100.0
    );
}
