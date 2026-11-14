//! Task 034: listings ranked by a metric. Uses data/houston/listings.json (local-only, gitignored):
//! every test returns early when it is absent, so the suite passes without it.

use engine::metric::{presets, Metric};
use engine::Core;

fn core() -> Option<Core> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../data/houston");
    let g = std::fs::read_to_string(format!("{dir}/graph.json")).ok()?;
    let mut c = Core::load(&g, &std::fs::read_to_string(format!("{dir}/places.json")).ok()?).ok()?;
    c.load_listings(&std::fs::read_to_string(format!("{dir}/listings.json")).ok()?).ok()?;
    Some(c)
}

fn preset(id: &str) -> Metric {
    Metric::compile(&presets().into_iter().find(|p| p.id == id).unwrap().metric).unwrap()
}

#[test]
fn rank_is_sorted_filtered_and_bounded() {
    let Some(c) = core() else { return };
    let m = preset("walk");
    let avail = c.rank_listings(&m, 720.0, 20.0, &[], true).unwrap();
    assert!(!avail.is_empty() && avail.iter().all(|r| c.listings[r.index].available()));
    assert!(avail.windows(2).all(|w| w[0].score >= w[1].score));
    assert!(avail.iter().all(|r| (0.0..=100.0).contains(&r.score)));
    let all = c.rank_listings(&m, 720.0, 20.0, &[], false).unwrap();
    assert_eq!(all.len(), c.listings.len());
    assert!(c.rank_listings(&m, 720.0, 0.0, &[], true).is_err());
}

#[test]
fn different_presets_give_different_top5() {
    let Some(c) = core() else { return };
    let top = |id: &str, t: f32| -> Vec<String> {
        let r = c.rank_listings(&preset(id), t, 20.0, &[], true).unwrap();
        println!("{id}:");
        for x in r.iter().take(5) {
            println!("  {:5.1}  {} ({} places in budget)", x.score, c.listings[x.index].name, x.reachable_places);
        }
        r.iter().take(5).map(|x| c.listings[x.index].id.clone()).collect()
    };
    let (stroller, night) = (top("stroller", 720.0), top("night", 22.0 * 60.0));
    assert_ne!(stroller, night, "stroller and night should not rank the same five");
}
