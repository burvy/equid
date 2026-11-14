//! Offline cache of the label menu: tile an area, run Vexcel detect for a list of labels, write `objects.json`.
//!   cargo run -p server --bin build_objects -- <graph.json> <out objects.json> "playground,crosswalk,..." [tile_m=200] [max_tiles=40]
//! Area = the graph's bbox. Reads the key like the server (VEXCEL_API_KEY / env file); never prints it.
//! Re-running merges into an existing out file, skipping tiles already done (progress file `<out>.done`).

use engine::objects::{Object, Objects};
use server::vexcel::Detector;
use std::collections::HashSet;

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: build_objects <graph.json> <out objects.json> <label,label,...> [tile_m] [max_tiles]");
        std::process::exit(2);
    }
    let graph: engine::graph::Graph = serde_json::from_str(&std::fs::read_to_string(&a[1]).expect("graph")).expect("graph json");
    let labels: Vec<String> = a[3].split(',').map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect();
    let tile: f64 = a.get(4).and_then(|v| v.parse().ok()).unwrap_or(200.0);
    let max_tiles: usize = a.get(5).and_then(|v| v.parse().ok()).unwrap_or(40);
    let det = Detector::from_env().expect("no Vexcel key (VEXCEL_API_KEY or env file)");

    let mut out: Objects = std::fs::read_to_string(&a[2]).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Objects { version: 1, objects: vec![] });
    let done_path = format!("{}.done", a[2]);
    let mut done: HashSet<String> = std::fs::read_to_string(&done_path).unwrap_or_default().lines().map(String::from).collect();

    let [w, s, e, n] = graph.meta.bbox;
    let lat_mid = (s + n) / 2.0;
    let (dlat, dlon) = (tile / 110_540.0, tile / (111_320.0 * lat_mid.to_radians().cos()));
    let (cols, rows) = ((((e - w) / dlon).ceil() as usize).max(1), (((n - s) / dlat).ceil() as usize).max(1));
    println!("area {cols} x {rows} tiles of {tile} m, {} labels: {}", labels.len(), labels.join(", "));
    let mut calls = 0;
    'tiles: for r in 0..rows {
        for c in 0..cols {
            let id = format!("{c},{r},{}", labels.join("|"));
            if done.contains(&id) {
                continue;
            }
            if calls >= max_tiles {
                println!("stopped at max_tiles={max_tiles}; rerun to continue");
                break 'tiles;
            }
            let bb = [w + c as f64 * dlon, s + r as f64 * dlat, (w + (c + 1) as f64 * dlon).min(e + dlon), (s + (r + 1) as f64 * dlat).min(n + dlat)];
            match det.detect_many(&labels, bb).await {
                Ok(res) => {
                    let mut k = 0;
                    for (label, ds) in res {
                        for d in ds {
                            out.objects.push(Object { label: label.clone(), lon: d.lon, lat: d.lat, src: Some("vexcel".into()), conf: Some(d.conf) });
                            k += 1;
                        }
                    }
                    println!("tile {c},{r}: {k} objects");
                    done.insert(id.clone());
                    std::fs::write(&a[2], serde_json::to_string_pretty(&out).unwrap()).unwrap();
                    std::fs::write(&done_path, done.iter().cloned().collect::<Vec<_>>().join("\n")).unwrap();
                }
                Err(e) => println!("tile {c},{r}: failed: {e}"),
            }
            calls += 1;
        }
    }
    println!("total {} objects in {}", out.objects.len(), a[2]);
}
