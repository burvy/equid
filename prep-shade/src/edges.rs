//! Shade step: rasters + sun -> graph.json `shade` / `tree`, places.json sunset spots.
//!
//! Per edge, points every ~5 m along its shape, at head height. For every 30-minute slot of the demo day,
//! the sun's position at 3 moments in the slot (+5, +15, +25 min). shade[slot] = 1 - the average share of
//! direct sun reaching those (point, moment) pairs (0 = full sun, 1 = full shade; night = 1).
//! tree = share of the edge's surroundings (+-2 m) under a tree crown.

use crate::geo::{Grid, Proj};
use crate::shade::{kappa_for_day, Scene, EYE_M};
use crate::sun::Day;
use crate::stargaze;
use crate::sunset::{self, Candidate};
use crate::{load_rasters, proj, Paths};
use engine::graph::{Graph, Place, Places};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::fmt::Write as _;

const SAMPLE_M: f64 = 5.0;
const SUB_MIN: [f64; 3] = [5.0, 15.0, 25.0];
/// An edge is "on a bridge" if this share of its points have bridge-deck returns; then people stand
/// on the deck, not on the ground below it.
const BRIDGE_SHARE: f64 = 0.6;
/// Sunset spots: how many, how far apart, and how close to the real sunset the sun must stay visible.
const SUNSET_SPOTS: usize = 8;
const SUNSET_SPACING_M: f64 = 400.0;
const SUNSET_SLACK_MIN: u32 = 10;
const OPEN_SKYLINE_DEG: f64 = 3.0;
/// Stargazing spots: how many and how far apart.
const STAR_SPOTS: usize = 8;
const STAR_SPACING_M: f64 = 400.0;
/// A stargazing spot must see at least this share of the sky above 10 deg (darkness alone in a wood is no use).
const STAR_MIN_OPEN: f64 = 0.7;

pub fn run(p: &Paths) -> Result<(), String> {
    let dir = match std::env::args().nth(2) {
        Some(d) => std::path::PathBuf::from(d),
        None => p.root.join("data/princeton"),
    };
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).map_err(|e| format!("{}: {e}", dir.join(n).display()));
    let mut graph: Graph = serde_json::from_str(&read("graph.json")?).map_err(|e| format!("graph.json: {e}"))?;
    let mut places: Places = serde_json::from_str(&read("places.json")?).map_err(|e| format!("places.json: {e}"))?;
    let (dsm, dtm, deck, tree) = load_rasters(p)?;
    let day = Day::parse(&graph.meta.date)?;
    let kappa = kappa_for_day(day.doy);
    let pr = proj();
    let scene = Scene::new(dsm, dtm, tree, kappa);
    let slots = graph.meta.slots as usize;
    let slot_min = graph.meta.slot_min as f64;
    let t0 = std::time::Instant::now();

    // Sun direction for every (slot, moment). The sun is ~150 million km away: one direction for the whole town.
    let suns: Vec<Vec<(f64, f64)>> =
        (0..slots).map(|s| SUB_MIN.iter().map(|m| day.sun(s as f64 * slot_min + m, pr.lat0, pr.lon0)).collect()).collect();

    // ---- shade + tree per edge
    let results: Vec<Option<(Vec<f32>, f32)>> = graph
        .edges
        .par_iter()
        .map(|e| {
            let pts = samples(&pr, &e.geom);
            let on_bridge = pts.iter().filter(|(x, y)| deck.at(*x, *y).is_some()).count() as f64 >= BRIDGE_SHARE * pts.len() as f64;
            let mut eyes = Vec::with_capacity(pts.len());
            for &(x, y) in &pts {
                let g = scene.ground(x, y)?;
                let stand = if on_bridge { deck.at(x, y).map(|d| d as f64).unwrap_or(g).max(g) } else { g };
                eyes.push((x, y, stand + EYE_M));
            }
            let shade: Vec<f32> = if e.covered {
                vec![1.0; slots] // under a roof all day
            } else {
                suns.iter()
                    .map(|moments| {
                        let mut light = 0.0;
                        for &(alt, az) in moments {
                            for &(x, y, z) in &eyes {
                                light += scene.sunlight(x, y, z, alt, az);
                            }
                        }
                        r2(1.0 - light / (moments.len() * eyes.len()) as f64)
                    })
                    .collect()
            };
            Some((shade, r2(canopy_share(&scene.tree, &pts))))
        })
        .collect();
    let mut missing = 0;
    for (e, r) in graph.edges.iter_mut().zip(results) {
        match r {
            Some((s, t)) => {
                e.shade = Some(s);
                e.tree = Some(t);
            }
            None => {
                e.shade = None;
                e.tree = None;
                missing += 1;
            }
        }
    }
    let t_shade = t0.elapsed().as_secs_f64();

    // ---- sunset: every graph node is a candidate spot
    let cands: Vec<Candidate> = graph.nodes.iter().map(|n| {
        let (x, y) = pr.xy(n.lon, n.lat);
        Candidate { x, y }
    }).collect();
    let verdicts = sunset::evaluate(&scene, &pr, &day, graph.meta.sunset_min as f64, &cands);
    let sunset_min = graph.meta.sunset_min.floor() as u32;
    // open view toward the sunset (no roof or crown more than OPEN_SKYLINE_DEG up in that direction), LiDAR-backed
    let mut order: Vec<usize> = (0..cands.len())
        .filter(|&i| verdicts[i].trusted && verdicts[i].visible_until.is_some() && verdicts[i].skyline_deg <= OPEN_SKYLINE_DEG)
        .collect();
    // latest sun first; ties: the lower (more open) skyline, then higher ground
    order.sort_by(|&a, &b| {
        let (va, vb) = (&verdicts[a], &verdicts[b]);
        vb.visible_until.cmp(&va.visible_until)
            .then(va.skyline_deg.partial_cmp(&vb.skyline_deg).unwrap_or(std::cmp::Ordering::Equal))
            .then(graph.nodes[b].elev_m.partial_cmp(&graph.nodes[a].elev_m).unwrap_or(std::cmp::Ordering::Equal))
    });
    let watch_sunset = order.iter().filter(|&&i| verdicts[i].visible_until.unwrap() + SUNSET_SLACK_MIN >= sunset_min).count();
    let mut picked: Vec<usize> = Vec::new();
    for &i in &order {
        if picked.len() == SUNSET_SPOTS {
            break;
        }
        if picked.iter().all(|&j| dist(&cands[i], &cands[j]) >= SUNSET_SPACING_M) {
            picked.push(i);
        }
    }
    let names = Names::load(&p.root.join("data/raw/overpass.json"), &places, &pr);
    places.places.retain(|pl| pl.category != "sunset");
    let mut spot_lines = String::new();
    for (rank, &i) in picked.iter().enumerate() {
        let v = &verdicts[i];
        let n = &graph.nodes[i];
        let until = v.visible_until.unwrap();
        let near = names.near(cands[i].x, cands[i].y);
        let name = match &near {
            Some(s) => format!("Sunset spot: {s}"),
            None => format!("Sunset spot #{}", rank + 1),
        };
        let mut tags = serde_json::Map::new();
        tags.insert("sun_visible_until".into(), json!(hhmm(until)));
        tags.insert("sun_visible_until_min".into(), json!(until));
        tags.insert("sunset".into(), json!(hhmm(sunset_min)));
        tags.insert("skyline_deg".into(), json!((v.skyline_deg * 10.0).round() / 10.0));
        tags.insert("date".into(), json!(graph.meta.date));
        tags.insert("source".into(), json!("USGS 3DEP LiDAR 2015 + NOAA sun position (prep-shade)"));
        let _ = writeln!(spot_lines, "  {name}: sun until {} (sunset {}), skyline {:.1} deg, ({:.5}, {:.5})", hhmm(until), hhmm(sunset_min), v.skyline_deg, n.lon, n.lat);
        places.places.push(Place {
            id: format!("sunset-{}", rank + 1),
            name: Some(name),
            lon: n.lon,
            lat: n.lat,
            category: "sunset".into(),
            tags,
            node: i as u32,
            snap_m: 0.0,
            rating: None,
            n: None,
            rating_src: None,
        });
    }
    // Existing viewpoints and parks: tell them too when the sun leaves them (at their snapped node).
    for pl in places.places.iter_mut().filter(|pl| matches!(pl.category.as_str(), "viewpoint" | "park" | "garden" | "nature_reserve")) {
        let v = &verdicts[pl.node as usize];
        match (v.trusted, v.visible_until) {
            (true, Some(u)) => {
                pl.tags.insert("sun_visible_until".into(), json!(hhmm(u)));
            }
            _ => {
                pl.tags.remove("sun_visible_until");
            }
        }
    }

    // ---- stargazing: open sky (36 skyline rays per node) + distance to the nearest lit street
    let t_star = std::time::Instant::now();
    let lit_pts: Vec<(f64, f64)> = graph.edges.iter().filter(|e| e.lit == Some(true)).flat_map(|e| samples(&pr, &e.geom)).collect();
    let (bx0, by0) = pr.xy(crate::BBOX[0], crate::BBOX[1]);
    let (bx1, by1) = pr.xy(crate::BBOX[2], crate::BBOX[3]);
    let maybe_pts: Vec<(f64, f64)> =
        graph.edges.iter().filter(|e| e.lit.is_none() && stargaze::maybe_lit(&e.highway)).flat_map(|e| samples(&pr, &e.geom)).collect();
    let skies = stargaze::evaluate(&scene, &graph, &pr, &lit_pts, &maybe_pts, [bx0, by0, bx1, by1]);
    let mut order: Vec<usize> = (0..skies.len()).filter(|&i| skies[i].ok && skies[i].open >= STAR_MIN_OPEN && skies[i].score() > 0.0).collect();
    // best score first; ties: more sky, then higher ground
    order.sort_by(|&a, &b| {
        skies[b].score().partial_cmp(&skies[a].score()).unwrap_or(std::cmp::Ordering::Equal)
            .then(skies[b].open.partial_cmp(&skies[a].open).unwrap_or(std::cmp::Ordering::Equal))
            .then(graph.nodes[b].elev_m.partial_cmp(&graph.nodes[a].elev_m).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut picked: Vec<usize> = Vec::new();
    for &i in &order {
        if picked.len() == STAR_SPOTS {
            break;
        }
        if picked.iter().all(|&j| dist(&cands[i], &cands[j]) >= STAR_SPACING_M) {
            picked.push(i);
        }
    }
    places.places.retain(|pl| pl.category != "stargazing");
    let mut star_lines = String::new();
    for (rank, &i) in picked.iter().enumerate() {
        let k = &skies[i];
        let n = &graph.nodes[i];
        let name = match names.near(cands[i].x, cands[i].y) {
            Some(s) => format!("Stargazing spot: {s}"),
            None => format!("Stargazing spot #{}", rank + 1),
        };
        let mut tags = serde_json::Map::new();
        tags.insert("sky_open".into(), json!(r2(k.open)));
        tags.insert("dark_m".into(), json!(k.dark_m.round()));
        tags.insert("score".into(), json!(r2(k.score())));
        tags.insert("source".into(), json!("USGS 3DEP LiDAR 2015 skyline + OSM street lighting (prep-shade)"));
        let _ = writeln!(star_lines, "  {name}: sky {:.0}%, {:.0} m from a lit street, score {:.2}, ({:.5}, {:.5})", k.open * 100.0, k.dark_m, k.score(), n.lon, n.lat);
        places.places.push(Place {
            id: format!("stargazing-{}", rank + 1),
            name: Some(name),
            lon: n.lon,
            lat: n.lat,
            category: "stargazing".into(),
            tags,
            node: i as u32,
            snap_m: 0.0,
            rating: None,
            n: None,
            rating_src: None,
        });
    }
    let t_star = t_star.elapsed().as_secs_f64();

    // ---- check with the engine's own loader, then write
    let graph_json = serde_json::to_string(&graph).map_err(|e| e.to_string())?;
    let places_json = serde_json::to_string_pretty(&places).map_err(|e| e.to_string())?;
    engine::Core::load(&graph_json, &places_json).map_err(|e| format!("engine rejects the output: {e}"))?;
    std::fs::write(dir.join("graph.json"), &graph_json).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("places.json"), &places_json).map_err(|e| e.to_string())?;

    // ---- stats
    let mut s = String::new();
    let _ = writeln!(s, "prep-shade stats for {} on {} (UTC{:+})", graph.meta.region, graph.meta.date, day.utc_offset_h);
    let _ = writeln!(s, "edges {} ({} without LiDAR), shade+tree in {:.1} s", graph.edges.len(), missing, t_shade);
    let total_len: f64 = graph.edges.iter().filter(|e| e.shade.is_some()).map(|e| e.length_m as f64).sum();
    let _ = writeln!(s, "share of network length in shade (length-weighted mean of shade[slot]):");
    for hh in [8u32, 10, 12, 13, 14, 15, 16] {
        let slot = (hh * 60) as usize / graph.meta.slot_min as usize;
        let (alt, az) = day.sun(slot as f64 * slot_min + 15.0, pr.lat0, pr.lon0);
        let shaded: f64 = graph.edges.iter().filter_map(|e| e.shade.as_ref().map(|sh| sh[slot] as f64 * e.length_m as f64)).sum();
        let mostly = graph.edges.iter().filter(|e| e.shade.as_ref().is_some_and(|sh| sh[slot] >= 0.5)).count();
        let _ = writeln!(s, "  {:02}:00-{:02}:30  sun alt {:5.1} az {:5.1}  shaded {:5.1}% of length, {:5.1}% of edges mostly shaded",
            hh, hh, alt, az, shaded * 100.0 / total_len, mostly as f64 * 100.0 / graph.edges.len() as f64);
    }
    let tree_mean: f64 = graph.edges.iter().filter_map(|e| e.tree.map(|t| t as f64 * e.length_m as f64)).sum::<f64>() / total_len;
    let _ = writeln!(s, "tree crown over/next to the path (+-2 m): {:.1}% of length", tree_mean * 100.0);
    let bridges = graph.edges.iter().filter(|e| {
        let pts = samples(&pr, &e.geom);
        pts.iter().filter(|(x, y)| deck.at(*x, *y).is_some()).count() as f64 >= BRIDGE_SHARE * pts.len() as f64
    }).count();
    let _ = writeln!(s, "edges on bridge decks: {bridges}");
    let _ = writeln!(s, "sunset {}: {} of {} nodes have an open view west (skyline <= {OPEN_SKYLINE_DEG} deg) and keep the sun until within {} min of sunset; spots (>= {} m apart):",
        hhmm(sunset_min), watch_sunset, cands.len(), SUNSET_SLACK_MIN, SUNSET_SPACING_M);
    s.push_str(&spot_lines);
    let _ = writeln!(s, "sunset + skylines in {:.1} s", t0.elapsed().as_secs_f64() - t_shade);
    let _ = writeln!(s, "stargazing: sky share above {} deg (36 skyline rays) x darkness (full at {} m from a lit street; {} lit + {} maybe-lit sample points); spots (>= {} m apart), {:.1} s:",
        stargaze::MIN_ALT_DEG, stargaze::DARK_FULL_M, lit_pts.len(), maybe_pts.len(), STAR_SPACING_M, t_star);
    s.push_str(&star_lines);
    // Sanity check: named streets/paths (>= 300 m), mean stargazing score of their end nodes.
    let mut by_name: std::collections::BTreeMap<&str, (f64, f64, f64, f64)> = Default::default(); // (length, score*len, open*len, dark*len)
    for e in &graph.edges {
        let Some(name) = e.osm.and_then(|id| names.way_name.get(&id)) else { continue };
        let (a, b) = (&skies[e.from as usize], &skies[e.to as usize]);
        if !(a.ok && b.ok) {
            continue;
        }
        let l = e.length_m as f64;
        let r = by_name.entry(name.as_str()).or_default();
        r.0 += l;
        r.1 += (a.score() + b.score()) / 2.0 * l;
        r.2 += (a.open + b.open) / 2.0 * l;
        r.3 += (a.dark_m + b.dark_m) / 2.0 * l;
    }
    let mut rows: Vec<(&str, f64, f64, f64, f64)> = by_name.into_iter().filter(|(_, r)| r.0 >= 300.0).map(|(n, r)| (n, r.0, r.1 / r.0, r.2 / r.0, r.3 / r.0)).collect();
    rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    let fmt = |r: &(&str, f64, f64, f64, f64)| format!("    {:<34} {:5.0} m  score {:.2}  sky {:3.0}%  {:4.0} m from light", r.0, r.1, r.2, r.3 * 100.0, r.4);
    let _ = writeln!(s, "  named streets/paths >= 300 m by stargazing score; best:");
    for r in rows.iter().take(6) {
        let _ = writeln!(s, "{}", fmt(r));
    }
    let _ = writeln!(s, "  worst:");
    for r in rows.iter().rev().take(4) {
        let _ = writeln!(s, "{}", fmt(r));
    }
    for want in ["Nassau Street", "Witherspoon Street", "Washington Road", "Cross Country Course", "Delaware & Raritan Canal Towpath", "Johnson Trolley Line"] {
        if let Some(r) = rows.iter().find(|r| r.0 == want) {
            let _ = writeln!(s, "  check: {}", fmt(r).trim_start());
        }
    }
    // Sanity check against what you see on the street: named streets (>= 300 m in the network), by shade at 13:00.
    let slot13 = (13 * 60) / graph.meta.slot_min as usize;
    let mut by_name: std::collections::BTreeMap<&str, (f64, f64, f64)> = Default::default(); // (length, shade13*len, tree*len)
    for e in &graph.edges {
        let (Some(sh), Some(t), Some(name)) = (e.shade.as_ref(), e.tree, e.osm.and_then(|id| names.way_name.get(&id))) else { continue };
        let a = by_name.entry(name.as_str()).or_default();
        let l = e.length_m as f64;
        a.0 += l;
        a.1 += sh[slot13] as f64 * l;
        a.2 += t as f64 * l;
    }
    let mut rows: Vec<(&str, f64, f64, f64)> = by_name.into_iter().filter(|(_, a)| a.0 >= 300.0).map(|(n, a)| (n, a.0, a.1 / a.0, a.2 / a.0)).collect();
    rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    let _ = writeln!(s, "named streets >= 300 m, shade at 13:00 (and tree cover); {} streets:", rows.len());
    let fmt = |r: &(&str, f64, f64, f64)| format!("    {:<28} {:5.0} m  shade {:4.0}%  trees {:4.0}%", r.0, r.1, r.2 * 100.0, r.3 * 100.0);
    let _ = writeln!(s, "  shadiest:");
    for r in rows.iter().take(10) {
        let _ = writeln!(s, "{}", fmt(r));
    }
    let _ = writeln!(s, "  sunniest:");
    for r in rows.iter().rev().take(10) {
        let _ = writeln!(s, "{}", fmt(r));
    }
    for want in ["Nassau Street", "Witherspoon Street", "Mercer Street", "Library Place", "Hodge Road", "Prospect Avenue", "University Place", "Washington Road"] {
        if let Some(r) = rows.iter().find(|r| r.0 == want) {
            let _ = writeln!(s, "  check: {}", fmt(r).trim_start());
        }
    }
    let (demo_text, best) = crate::demo::run(&p.root, &dir)?;
    s.push_str(&demo_text);
    if let Some((label, walk, r13, r16)) = best {
        // pictures for the README / pitch: white = plain walk route, red = shade route at that time
        let img = p.root.join("docs/img");
        std::fs::create_dir_all(&img).map_err(|e| e.to_string())?;
        let slot = |h: usize| h * 60 / graph.meta.slot_min as usize;
        for (h, route) in [(13usize, r13), (16, r16)] {
            let out = img.join(format!("shade_route_{h}00.png"));
            crate::image::routes_png(&scene.dsm, &scene.dtm, &scene.tree, &graph, &pr, slot(h), &[(walk.clone(), [255, 255, 255]), (route, [230, 30, 60])], &out, 2.5)?;
        }
        let _ = writeln!(s, "  pictures: docs/img/shade_route_1300.png, shade_route_1600.png ({label}; streets yellow = sun, blue = shade; white = walk route, red = shade route)");
    }
    eprint!("{s}");
    std::fs::write(dir.join("shade_stats.txt"), s).map_err(|e| e.to_string())?;
    Ok(())
}

fn r2(x: f64) -> f32 {
    ((x * 100.0).round() / 100.0) as f32
}
fn hhmm(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}
fn dist(a: &Candidate, b: &Candidate) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Points every ~SAMPLE_M metres along a [lon, lat, elev] polyline (centres of equal pieces), local metres.
pub fn samples(pr: &Proj, geom: &[[f64; 3]]) -> Vec<(f64, f64)> {
    let pts: Vec<(f64, f64)> = geom.iter().map(|g| pr.xy(g[0], g[1])).collect();
    let seg: Vec<f64> = pts.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).collect();
    let total: f64 = seg.iter().sum();
    let n = ((total / SAMPLE_M).round() as usize).max(1);
    let mut out = Vec::with_capacity(n);
    let (mut k, mut acc) = (0usize, 0.0);
    for s in 0..n {
        let want = (s as f64 + 0.5) * total / n as f64;
        while k + 1 < seg.len() && acc + seg[k] < want {
            acc += seg[k];
            k += 1;
        }
        let f = if seg.get(k).copied().unwrap_or(0.0) > 0.0 { ((want - acc) / seg[k]).clamp(0.0, 1.0) } else { 0.0 };
        let (a, b) = (pts[k], pts[(k + 1).min(pts.len() - 1)]);
        out.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
    }
    out
}

/// Share of cells within +-2 m of the points that are tree crown.
fn canopy_share(tree: &Grid, pts: &[(f64, f64)]) -> f64 {
    let (mut n, mut c) = (0usize, 0usize);
    for &(x, y) in pts {
        for dy in -2..=2 {
            for dx in -2..=2 {
                if let Some(v) = tree.at(x + dx as f64, y + dy as f64) {
                    n += 1;
                    if v > 0.5 {
                        c += 1;
                    }
                }
            }
        }
    }
    if n == 0 {
        0.0
    } else {
        c as f64 / n as f64
    }
}

/// Human names for sunset spots: the nearest named park/garden/viewpoint (from places.json),
/// else the nearest named street or path (from the raw OSM download).
struct Names {
    parks: Vec<(f64, f64, String)>,
    streets: Vec<(f64, f64, String)>,
    /// OSM way id -> name (graph edges carry their way id)
    way_name: std::collections::HashMap<i64, String>,
}
impl Names {
    fn load(overpass: &std::path::Path, places: &Places, pr: &Proj) -> Names {
        let parks = places.places.iter()
            .filter(|p| matches!(p.category.as_str(), "park" | "garden" | "nature_reserve" | "viewpoint" | "attraction"))
            .filter_map(|p| p.name.clone().map(|n| { let (x, y) = pr.xy(p.lon, p.lat); (x, y, n) }))
            .collect();
        let mut streets = Vec::new();
        let mut way_name = std::collections::HashMap::new();
        if let Some(osm) = std::fs::read_to_string(overpass).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            for e in osm["elements"].as_array().into_iter().flatten() {
                let (Some(name), Some(geom)) = (e["tags"]["name"].as_str(), e["geometry"].as_array()) else { continue };
                if let Some(id) = e["id"].as_i64() {
                    way_name.insert(id, name.to_string());
                }
                if e["tags"]["highway"].is_null() {
                    continue;
                }
                for g in geom {
                    if let (Some(lon), Some(lat)) = (g["lon"].as_f64(), g["lat"].as_f64()) {
                        let (x, y) = pr.xy(lon, lat);
                        streets.push((x, y, name.to_string()));
                    }
                }
            }
        }
        Names { parks, streets, way_name }
    }
    fn near(&self, x: f64, y: f64) -> Option<String> {
        let best = |v: &Vec<(f64, f64, String)>, max: f64| {
            v.iter().map(|(a, b, n)| (((a - x).powi(2) + (b - y).powi(2)).sqrt(), n))
                .filter(|(d, _)| *d <= max)
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap())
                .map(|(_, n)| n.clone())
        };
        best(&self.parks, 200.0).or_else(|| best(&self.streets, 150.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_are_evenly_spaced_centres() {
        let pr = Proj::new(0.0, 0.0);
        let (lon1, _) = pr.lonlat(20.0, 0.0);
        let s = samples(&pr, &[[0.0, 0.0, 0.0], [lon1, 0.0, 0.0]]);
        assert_eq!(s.len(), 4);
        for (k, (x, y)) in s.iter().enumerate() {
            assert!((x - (2.5 + 5.0 * k as f64)).abs() < 1e-6 && y.abs() < 1e-9, "{k}: {x}");
        }
        // a very short edge still gets one point (its middle)
        let (lon2, _) = pr.lonlat(1.0, 0.0);
        let s = samples(&pr, &[[0.0, 0.0, 0.0], [lon2, 0.0, 0.0]]);
        assert_eq!(s.len(), 1);
        assert!((s[0].0 - 0.5).abs() < 1e-6);
    }
}
