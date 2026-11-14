//! Fetch step: download the USGS 3DEP LiDAR point cloud for our area from the public
//! Entwine Point Tile (EPT) copy on AWS (no account, no key).
//!
//! EPT = an octree of LAZ files. Node "d-x-y-z" is a cube of side root/2^d; every depth adds
//! points in between the shallower ones, so "all nodes down to depth D" is a uniform thinning
//! of the cloud. Depth 10 here is ~157 m (Mercator) cubes, a few points per m^2: enough for 1 m cells.
//! Which nodes exist (and their point counts) is listed in JSON hierarchy pages.

use crate::geo::lonlat_to_merc;
use rayon::prelude::*;
use serde_json::Value;
use std::path::Path;

pub const DATASET: &str = "USGS_LPC_DE_DelawareValley_HD_2015_LAS_2017";
fn base() -> String {
    format!("https://s3-us-west-2.amazonaws.com/usgs-lidar-public/{DATASET}")
}

fn get_json(url: &str) -> Result<Value, String> {
    let r = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let s = r.into_string().map_err(|e| format!("{url}: {e}"))?;
    serde_json::from_str(&s).map_err(|e| format!("{url}: {e}"))
}

/// Downloads every EPT node touching `outer` (lon/lat bbox) down to `outer_depth`, plus nodes
/// touching `core` down to `core_depth`, into `dir/<key>.laz`. Skips files already there.
/// Writes `dir/tiles.txt` (the tile list) last, so the raster step knows the set is complete.
pub fn fetch(dir: &Path, core: [f64; 4], core_depth: u32, outer: [f64; 4], outer_depth: u32) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let ept = get_json(&format!("{}/ept.json", base()))?;
    let b: Vec<f64> = ept["bounds"].as_array().ok_or("ept.json: no bounds")?.iter().filter_map(|v| v.as_f64()).collect();
    let (rx, ry, rsize) = (b[0], b[1], b[3] - b[0]);
    let merc = |bb: [f64; 4]| {
        let (a, c) = lonlat_to_merc(bb[0], bb[1]);
        let (d, e) = lonlat_to_merc(bb[2], bb[3]);
        [a, c, d, e]
    };
    let (core_m, outer_m) = (merc(core), merc(outer));
    let hits = |key: &str, bb: &[f64; 4]| {
        let k: Vec<f64> = key.split('-').map(|s| s.parse().unwrap_or(0.0)).collect();
        let sz = rsize / 2f64.powi(k[0] as i32);
        let (x0, y0) = (rx + k[1] * sz, ry + k[2] * sz);
        x0 < bb[2] && x0 + sz > bb[0] && y0 < bb[3] && y0 + sz > bb[1]
    };
    let depth = |key: &str| key.split('-').next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(99);
    let wanted = |key: &str| (depth(key) <= outer_depth && hits(key, &outer_m)) || (depth(key) <= core_depth && hits(key, &core_m));

    // Walk the hierarchy pages, only descending into subtrees that touch the outer box.
    let mut nodes: Vec<(String, u64)> = Vec::new();
    let mut pages = vec!["0-0-0-0".to_string()];
    while let Some(p) = pages.pop() {
        let j = get_json(&format!("{}/ept-hierarchy/{p}.json", base()))?;
        for (k, v) in j.as_object().ok_or("hierarchy page not an object")? {
            if !hits(k, &outer_m) || depth(k) > core_depth.max(outer_depth) {
                continue;
            }
            match v.as_i64() {
                Some(-1) if k != &p => pages.push(k.clone()),
                Some(n) if n > 0 && wanted(k) => nodes.push((k.clone(), n as u64)),
                _ => {}
            }
        }
    }
    nodes.sort();
    let total: u64 = nodes.iter().map(|n| n.1).sum();
    let todo: Vec<&(String, u64)> = nodes.iter().filter(|(k, _)| !dir.join(format!("{k}.laz")).exists()).collect();
    eprintln!("lidar: {} tiles, {:.1} M points; {} to download", nodes.len(), total as f64 / 1e6, todo.len());

    // Downloads are I/O bound: more threads than cores.
    let pool = rayon::ThreadPoolBuilder::new().num_threads(16).build().map_err(|e| e.to_string())?;
    let done = std::sync::atomic::AtomicUsize::new(0);
    let errs: Vec<String> = pool.install(|| {
        todo.par_iter()
            .filter_map(|(k, _)| {
                let url = format!("{}/ept-data/{k}.laz", base());
                let out = dir.join(format!("{k}.laz"));
                let tmp = dir.join(format!("{k}.laz.part"));
                let mut last = String::new();
                for attempt in 0..4u64 {
                    if attempt > 0 {
                        std::thread::sleep(std::time::Duration::from_secs(2 * attempt));
                    }
                    let res = (|| -> Result<(), String> {
                        let r = ureq::get(&url).call().map_err(|e| e.to_string())?;
                        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
                        std::io::copy(&mut r.into_reader(), &mut f).map_err(|e| e.to_string())?;
                        drop(f);
                        std::fs::rename(&tmp, &out).map_err(|e| e.to_string())
                    })();
                    match res {
                        Ok(()) => {
                            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            if n % 200 == 0 {
                                eprintln!("  {n}/{}", todo.len());
                            }
                            return None;
                        }
                        Err(e) => last = e,
                    }
                }
                Some(format!("{k}: {last}"))
            })
            .collect()
    });
    if !errs.is_empty() {
        return Err(format!("{} tiles failed, re-run to retry: {}", errs.len(), errs[0]));
    }
    let list: String = nodes.iter().map(|(k, n)| format!("{k} {n}\n")).collect();
    std::fs::write(dir.join("tiles.txt"), list).map_err(|e| e.to_string())?;
    Ok(())
}
