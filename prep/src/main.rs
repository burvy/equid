//! prep: raw downloads (from prep/fetch.sh) -> data/<region>/{graph,places,objects}.json + stats.txt.
//! Regions are listed in prep/regions.tsv: `cargo run --release -p prep [-- <region>]` (default princeton).
//! Deterministic: same cache in, same files out. No network access here.
//!
//! Pipeline: OSM ways -> keep walkable/rideable -> clip to bbox -> split at intersections into edges
//! -> keep the largest connected piece -> elevation per vertex -> per-edge features -> places snapped
//! to the nearest node -> validate with the engine's own loader -> write.

mod factors;
mod geo;
mod listings;
mod night;
mod objects;
mod region;
mod tags;

use engine::graph::{Cycleway, Edge, Graph, Meta, Node, Place, Places, Surface, NEAR_CAP_M};
use geo::{dist, dist_to_segment, ElevGrid, Proj};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use tags::{get, Tags};

/// Demo day; sunrise/sunset and (later) shade slots are computed for it.
const DATE: (i32, u32, u32) = (2026, 11, 15);
/// Elevation profile sample spacing along edges, and the window over which grade is measured
/// (shorter windows just amplify DEM noise).
const SAMPLE_M: f64 = 5.0;
const GRADE_WINDOW_M: f64 = 20.0;
/// A footway this close to a road counts as running beside it (sidewalks, crossings).
const BESIDE_ROAD_M: f64 = 15.0;

struct Way {
    id: i64,
    nodes: Vec<i64>,
    pts: Vec<(f64, f64)>, // (lon, lat)
    tags: Tags,
}

fn r6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}
fn r1(x: f64) -> f32 {
    ((x * 10.0).round() / 10.0) as f32
}

fn main() {
    if let Err(e) = run() {
        eprintln!("prep: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let name = std::env::args().nth(1).unwrap_or_else(|| "princeton".into());
    let reg = region::Region::load(&root.join("prep/regions.tsv"), &name)?;
    let raw = root.join(&reg.raw_dir);
    let out = root.join("data").join(&reg.name);
    let bbox = reg.bbox;
    let inside = |p: (f64, f64)| p.0 >= bbox[0] && p.0 <= bbox[2] && p.1 >= bbox[1] && p.1 <= bbox[3];

    let osm_text = std::fs::read_to_string(raw.join("overpass.json")).map_err(|e| format!("overpass.json: {e} (run prep/fetch.sh {name})"))?;
    let osm: Value = serde_json::from_str(&osm_text).map_err(|e| e.to_string())?;
    let osm_time = osm["osm3s"]["timestamp_osm_base"].as_str().unwrap_or("?").to_string();
    let elements = osm["elements"].as_array().ok_or("overpass.json: no elements")?;
    let grid = ElevGrid::load(&raw.join("elev_points.txt"), &raw.join("ned10m"))?;
    let proj = Proj::new((bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0);
    let night_text = std::fs::read_to_string(raw.join("night.json")).map_err(|e| format!("night.json: {e} (run prep/fetch.sh {name})"))?;
    let night = night::Night::load(&serde_json::from_str(&night_text).map_err(|e| e.to_string())?, &proj, reg.campus_prefix.as_deref());
    let read_json = |f: &str| -> Result<Value, String> {
        let t = std::fs::read_to_string(raw.join(f)).map_err(|e| format!("{f}: {e} (run prep/fetch.sh {name})"))?;
        serde_json::from_str(&t).map_err(|e| format!("{f}: {e}"))
    };
    let flood = read_json("flood.json")?;
    let flood = flood["features"].as_array().is_some_and(|f| !f.is_empty()).then_some(&flood);
    let factors = factors::Factors::load(&read_json("nodes.json")?, &read_json("landscape.json")?, flood, &proj);

    // ---- 1. ways
    let mut ways = Vec::new();
    let mut car_segments = Vec::new(); // (class, a, b) in metres, for the "beside a busy road" test
    for e in elements {
        let Some(t) = e["tags"].as_object() else { continue };
        if e["type"] != "way" {
            continue;
        }
        let Some(hw) = get(t, "highway") else { continue };
        let (Some(ns), Some(geom)) = (e["nodes"].as_array(), e["geometry"].as_array()) else { continue };
        let pts: Option<Vec<(f64, f64)>> = geom.iter().map(|g| Some((g["lon"].as_f64()?, g["lat"].as_f64()?))).collect();
        let Some(pts) = pts else { continue };
        let nodes: Vec<i64> = ns.iter().filter_map(Value::as_i64).collect();
        if nodes.len() != pts.len() || pts.len() < 2 {
            continue;
        }
        let class = tags::road_class(hw);
        if class > 0 {
            for w in pts.windows(2) {
                car_segments.push((class, proj.xy(w[0].0, w[0].1), proj.xy(w[1].0, w[1].1)));
            }
        }
        if !tags::is_path_highway(hw) || get(t, "area") == Some("yes") && hw != "pedestrian" {
            continue;
        }
        let (foot, bike) = tags::access(t);
        if foot || bike {
            ways.push(Way { id: e["id"].as_i64().unwrap_or(0), nodes, pts, tags: t.clone() });
        }
    }

    // ---- 2. graph nodes = points used by 2+ ways (intersections) or ends of a clipped run
    let mut uses: HashMap<i64, u32> = HashMap::new();
    for w in &ways {
        for (i, &n) in w.nodes.iter().enumerate() {
            if inside(w.pts[i]) {
                *uses.entry(n).or_default() += 1;
            }
        }
    }
    struct RawEdge {
        way: usize,
        ids: Vec<i64>,
        pts: Vec<(f64, f64)>,
    }
    let mut raw_edges = Vec::new();
    for (wi, w) in ways.iter().enumerate() {
        // runs of consecutive inside points
        let mut i = 0;
        while i < w.pts.len() {
            if !inside(w.pts[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < w.pts.len() && inside(w.pts[i]) {
                i += 1;
            }
            // split run [start, i) at intersections
            let mut cur = (vec![w.nodes[start]], vec![w.pts[start]]);
            for k in start + 1..i {
                cur.0.push(w.nodes[k]);
                cur.1.push(w.pts[k]);
                if k == i - 1 || uses[&w.nodes[k]] >= 2 {
                    let done = std::mem::replace(&mut cur, (vec![w.nodes[k]], vec![w.pts[k]]));
                    raw_edges.push(RawEdge { way: wi, ids: done.0, pts: done.1 });
                }
            }
        }
    }
    raw_edges.retain(|e| e.ids.len() >= 2 && e.ids[0] != *e.ids.last().unwrap()); // drop self-loops

    // ---- 3. largest connected component (union-find over OSM node ids)
    let mut idx: HashMap<i64, usize> = HashMap::new();
    let mut coord: Vec<(i64, (f64, f64))> = Vec::new();
    for e in &raw_edges {
        for (id, p) in [(e.ids[0], e.pts[0]), (*e.ids.last().unwrap(), *e.pts.last().unwrap())] {
            idx.entry(id).or_insert_with(|| {
                coord.push((id, p));
                coord.len() - 1
            });
        }
    }
    let mut parent: Vec<usize> = (0..coord.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for e in &raw_edges {
        let (a, b) = (find(&mut parent, idx[&e.ids[0]]), find(&mut parent, idx[e.ids.last().unwrap()]));
        parent[a] = b;
    }
    let mut size: HashMap<usize, usize> = HashMap::new();
    for i in 0..coord.len() {
        *size.entry(find(&mut parent, i)).or_default() += 1;
    }
    let (&big, _) = size.iter().max_by_key(|(&r, &s)| (s, std::cmp::Reverse(r))).ok_or("empty graph")?;
    let n_components = size.len();
    let nodes_before = coord.len();

    // final node numbering: in order of first appearance, keeping only the big component
    let mut new_idx = vec![u32::MAX; coord.len()];
    let mut nodes = Vec::new();
    for i in 0..coord.len() {
        if find(&mut parent, i) == big {
            new_idx[i] = nodes.len() as u32;
            let (id, (lon, lat)) = coord[i];
            nodes.push(Node { lon: r6(lon), lat: r6(lat), elev_m: r1(grid.at(lon, lat)), osm: Some(id) });
        }
    }

    // ---- 4. places of interest (also feed the near_* features)
    let mut places_raw = Vec::new(); // (id, tags, centroid, vertices)
    for e in elements {
        let Some(t) = e["tags"].as_object() else { continue };
        let Some(cat) = tags::place_category(t) else { continue };
        let kind = e["type"].as_str().unwrap_or("");
        let verts: Vec<(f64, f64)> = match kind {
            "node" => vec![(e["lon"].as_f64().unwrap_or(0.0), e["lat"].as_f64().unwrap_or(0.0))],
            "way" => e["geometry"].as_array().map(|g| pts_of(g)).unwrap_or_default(),
            "relation" => e["members"]
                .as_array()
                .map(|ms| ms.iter().filter_map(|m| m["geometry"].as_array()).flat_map(|g| pts_of(g)).collect())
                .unwrap_or_default(),
            _ => vec![],
        };
        if verts.is_empty() {
            continue;
        }
        let c = (verts.iter().map(|p| p.0).sum::<f64>() / verts.len() as f64, verts.iter().map(|p| p.1).sum::<f64>() / verts.len() as f64);
        if !inside(c) {
            continue;
        }
        let id = format!("{}{}", &kind[..1], e["id"].as_i64().unwrap_or(0));
        places_raw.push((id, t.clone(), cat, c, verts));
    }
    let amenity_xy = |cat: &str| -> Vec<(f64, f64)> {
        places_raw.iter().filter(|p| p.2 == cat).map(|p| proj.xy(p.3 .0, p.3 .1)).collect()
    };
    let (benches, toilets, water) = (amenity_xy("bench"), amenity_xy("toilets"), amenity_xy("drinking_water"));

    // spatial hash of car-road segments (50 m cells) for the "beside a road" test
    let cell = |p: (f64, f64)| ((p.0 / 50.0).floor() as i64, (p.1 / 50.0).floor() as i64);
    let mut road_cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, s) in car_segments.iter().enumerate() {
        let (a, b) = (cell(s.1), cell(s.2));
        for cx in a.0.min(b.0) - 1..=a.0.max(b.0) + 1 {
            for cy in a.1.min(b.1) - 1..=a.1.max(b.1) + 1 {
                road_cells.entry((cx, cy)).or_default().push(i);
            }
        }
    }
    let road_beside = |p: (f64, f64)| -> u8 {
        road_cells.get(&cell(p)).map_or(0, |v| {
            v.iter().map(|&i| &car_segments[i]).filter(|s| dist_to_segment(p, s.1, s.2) <= BESIDE_ROAD_M).map(|s| s.0).max().unwrap_or(0)
        })
    };

    // ---- 5. edges with features
    let mut edges = Vec::new();
    let mut edge_ids: Vec<Vec<i64>> = Vec::new(); // OSM node ids along each edge, for factors::fill
    let mut edge_tags: Vec<&Tags> = Vec::new();
    for e in &raw_edges {
        let (a, b) = (idx[&e.ids[0]], idx[e.ids.last().unwrap()]);
        if new_idx[a] == u32::MAX {
            continue;
        }
        let w = &ways[e.way];
        let t = &w.tags;
        let hw = get(t, "highway").unwrap_or("").to_string();
        let xy: Vec<(f64, f64)> = e.pts.iter().map(|p| proj.xy(p.0, p.1)).collect();
        let length: f64 = xy.windows(2).map(|s| dist(s[0], s[1])).sum();

        // elevation profile, sampled every SAMPLE_M metres; bridges/tunnels interpolate end to end
        let off = tags::off_terrain(t);
        let (z0, z1) = (grid.at(e.pts[0].0, e.pts[0].1), grid.at(e.pts.last().unwrap().0, e.pts.last().unwrap().1));
        let mut along = 0.0;
        let mut prof: Vec<(f64, f64)> = vec![(0.0, z0)];
        let mut vert_z = vec![z0];
        for k in 1..e.pts.len() {
            let seg = dist(xy[k - 1], xy[k]);
            let steps = (seg / SAMPLE_M).ceil().max(1.0) as usize;
            for s in 1..=steps {
                let f = s as f64 / steps as f64;
                let d = along + seg * f;
                let z = if off {
                    z0 + (z1 - z0) * if length > 0.0 { d / length } else { 0.0 }
                } else {
                    grid.at(e.pts[k - 1].0 + (e.pts[k].0 - e.pts[k - 1].0) * f, e.pts[k - 1].1 + (e.pts[k].1 - e.pts[k - 1].1) * f)
                };
                prof.push((d, z));
            }
            along += seg;
            vert_z.push(prof.last().unwrap().1);
        }
        let (mut climb, mut descent) = (0.0, 0.0);
        for s in prof.windows(2) {
            let dz = s[1].1 - s[0].1;
            if dz > 0.0 { climb += dz } else { descent -= dz }
        }
        let max_grade = if length < GRADE_WINDOW_M {
            if length > 1.0 { (z1 - z0).abs() / length * 100.0 } else { 0.0 }
        } else {
            let mut g: f64 = 0.0;
            let mut j = 0;
            for i in 0..prof.len() {
                while j < prof.len() && prof[j].0 - prof[i].0 < GRADE_WINDOW_M {
                    j += 1;
                }
                if j == prof.len() {
                    break;
                }
                g = g.max((prof[j].1 - prof[i].1).abs() / (prof[j].0 - prof[i].0) * 100.0);
            }
            g
        };

        let (foot, bike) = tags::access(t);
        let (surface, surface_raw) = tags::surface(t);
        let own = tags::road_class(&hw);
        let busy = if own > 0 { own } else { road_beside(xy[xy.len() / 2]).max(road_beside(mid(&xy))) };
        let (lit, lit_src) = night.lit(tags::lit(t), &hw, surface, busy, &xy, mid(&xy));
        let near = |pts: &[(f64, f64)]| -> f32 {
            let d = pts
                .iter()
                .map(|&p| xy.windows(2).map(|s| dist_to_segment(p, s[0], s[1])).fold(f64::MAX, f64::min))
                .fold(NEAR_CAP_M as f64, f64::min);
            r1(d)
        };
        edge_ids.push(e.ids.clone());
        edge_tags.push(&w.tags);
        edges.push(Edge {
            from: new_idx[a],
            to: new_idx[b],
            osm: Some(w.id),
            length_m: r1(length),
            geom: e.pts.iter().zip(&vert_z).map(|(p, z)| [r6(p.0), r6(p.1), (z * 10.0).round() / 10.0]).collect(),
            climb_m: r1(climb),
            descent_m: r1(descent),
            max_grade_pct: r1(max_grade),
            highway: hw.clone(),
            foot,
            bike,
            oneway_bike: if bike { tags::oneway_bike(t) } else { 0 },
            steps: hw == "steps",
            surface,
            surface_raw,
            lit,
            lit_src: lit_src.map(str::to_string),
            covered: tags::covered(t),
            busy,
            cycleway: tags::cycleway(t, bike),
            lts: tags::lts(t, bike),
            wheelchair: get(t, "wheelchair").map(str::to_string),
            near_bench_m: near(&benches),
            near_toilets_m: near(&toilets),
            near_water_m: near(&water),
            tree: None,
            shade: None,
            flood: None,
            crossings: 0,
            crossings_unsignalled: 0,
            sidewalk: None,
            kerb_raised: 0,
            stops: 0,
            junction_crossings: 0.0,
            green: None,
            water_view: None,
        });
    }

    let factor_report = factors.fill(&mut edges, &edge_ids, &edge_tags, &proj);

    // ---- 6. snap places to the nearest graph node (areas: nearest to any outline vertex)
    let node_xy: Vec<(f64, f64)> = nodes.iter().map(|n| proj.xy(n.lon, n.lat)).collect();
    let mut places: Vec<Place> = places_raw
        .iter()
        .map(|(id, t, cat, c, verts)| {
            let (node, snap) = verts
                .iter()
                .map(|v| proj.xy(v.0, v.1))
                .flat_map(|p| node_xy.iter().enumerate().map(move |(i, &q)| (i, dist(p, q))))
                .fold((0, f64::MAX), |best, x| if x.1 < best.1 { x } else { best });
            let tags_out = tags::PLACE_TAGS.iter().filter_map(|k| t.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
            Place {
                id: id.clone(),
                name: get(t, "name").map(str::to_string),
                lon: r6(c.0),
                lat: r6(c.1),
                category: cat.clone(),
                tags: tags_out,
                node: node as u32,
                snap_m: r1(snap),
                rating: None,
                n: None,
                rating_src: None,
            }
        })
        .collect();
    places.sort_by(|a, b| (&a.category, &a.id).cmp(&(&b.category, &b.id)));

    // ---- 7. assemble, validate with the engine's loader, write
    let (sunrise, sunset) = geo::sun_times(DATE.0, DATE.1, DATE.2, (bbox[1] + bbox[3]) / 2.0, (bbox[0] + bbox[2]) / 2.0, reg.utc_offset_h);
    let graph = Graph {
        meta: Meta {
            version: 1,
            region: reg.name.clone(),
            bbox,
            tz: reg.tz.clone(),
            date: format!("{:04}-{:02}-{:02}", DATE.0, DATE.1, DATE.2),
            sunrise_min: sunrise.round() as f32,
            sunset_min: sunset.round() as f32,
            slot_min: 30,
            slots: 48,
            generated: Some(format!("prep from OSM snapshot {osm_time}")),
            source: Some("Roads and places: (c) OpenStreetMap contributors, ODbL, via Overpass API. Elevation: USGS 3DEP 1/3 arc-second (ned10m) via OpenTopoData.".into()),
        },
        nodes,
        edges,
    };
    let places = Places { version: 1, places };
    let graph_json = serde_json::to_string(&graph).map_err(|e| e.to_string())?;
    let places_json = serde_json::to_string_pretty(&places).map_err(|e| e.to_string())?;
    let mut core = engine::Core::load(&graph_json, &places_json).map_err(|e| format!("output failed the engine's validation: {e}"))?;

    // ---- 8. point objects (contracts §2b): OSM ones are rebuilt, others (Vexcel, ...) already in the file are kept
    let objects_text = std::fs::read_to_string(raw.join("objects.json")).map_err(|e| format!("objects.json: {e} (run prep/fetch.sh {name})"))?;
    let old = std::fs::read_to_string(out.join("objects.json")).ok();
    let objs = objects::build(&serde_json::from_str(&objects_text).map_err(|e| e.to_string())?, &inside, old.as_deref())?;
    let objects_json = serde_json::to_string_pretty(&objs).map_err(|e| e.to_string())?;
    core.load_objects(&objects_json).map_err(|e| format!("objects failed the engine's validation: {e}"))?;

    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut stats = stats(&graph, &places, n_components, nodes_before, &grid, &night);
    stats += &factors::stats(&graph.edges, &factor_report);
    stats += &objects::stats(&objs, &core.labels());
    for (file, body) in [("graph.json", &graph_json), ("places.json", &places_json), ("objects.json", &objects_json), ("stats.txt", &stats)] {
        std::fs::write(out.join(file), body).map_err(|e| format!("{file}: {e}"))?;
    }
    print!("{stats}");
    println!("wrote data/{name}/graph.json ({:.1} MB), places.json, objects.json, stats.txt", graph_json.len() as f64 / 1e6);

    // ---- 9. CoStar listings (task 033), LOCAL ONLY: gitignored, and kept out of stats.txt (which is committed)
    let kit = std::env::var("COSTAR_DIR").unwrap_or_else(|_| r"Z:\App Data\Downloads\costar".into());
    let kit = Path::new(&kit);
    if kit.join("data").is_dir() {
        let snap = |lon: f64, lat: f64| {
            let p = proj.xy(lon, lat);
            node_xy.iter().enumerate().map(|(i, &q)| (i as u32, dist(p, q))).fold((0, f64::MAX), |b, x| if x.1 < b.1 { x } else { b })
        };
        let l = listings::build(kit, bbox, &snap)?;
        if l.stats.starts_with("listings 0 ") {
            println!("CoStar kit: no listings in this box");
        } else {
            std::fs::write(out.join("listings.json"), &l.json).map_err(|e| format!("listings.json: {e}"))?;
            print!("{}", l.stats);
            println!("wrote data/{name}/listings.json (local only, gitignored)");
        }
    } else {
        println!("CoStar kit not found at {} (set COSTAR_DIR): no listings.json", kit.display());
    }
    Ok(())
}

fn pts_of(g: &[Value]) -> Vec<(f64, f64)> {
    g.iter().filter_map(|p| Some((p["lon"].as_f64()?, p["lat"].as_f64()?))).collect()
}

/// Midpoint along a polyline (by length).
fn mid(xy: &[(f64, f64)]) -> (f64, f64) {
    let total: f64 = xy.windows(2).map(|s| dist(s[0], s[1])).sum();
    let mut acc = 0.0;
    for s in xy.windows(2) {
        let d = dist(s[0], s[1]);
        if acc + d >= total / 2.0 && d > 0.0 {
            let f = (total / 2.0 - acc) / d;
            return (s[0].0 + (s[1].0 - s[0].0) * f, s[0].1 + (s[1].1 - s[0].1) * f);
        }
        acc += d;
    }
    xy[0]
}

/// Data-quality summary: counts and how much of the network carries each tag.
fn stats(g: &Graph, p: &Places, components: usize, nodes_before: usize, grid: &ElevGrid, night: &night::Night) -> String {
    let n = g.edges.len() as f64;
    let km: f64 = g.edges.iter().map(|e| e.length_m as f64).sum::<f64>() / 1000.0;
    let pct = |f: &dyn Fn(&Edge) -> bool| {
        let c = g.edges.iter().filter(|e| f(e)).count() as f64;
        let l: f64 = 0.0 + g.edges.iter().filter(|e| f(e)).map(|e| e.length_m as f64).sum::<f64>() / 1000.0; // 0.0 + : an empty sum is -0.0
        format!("{:5.1}% of edges, {:5.1}% of length", 100.0 * c / n, 100.0 * l / km)
    };
    let mut s = String::new();
    let _ = writeln!(s, "Equid data prep stats ({})", g.meta.generated.as_deref().unwrap_or(""));
    let _ = writeln!(s, "nodes {}  edges {}  network {:.1} km  (kept 1 of {} connected pieces, {} of {} nodes)", g.nodes.len(), g.edges.len(), km, components, g.nodes.len(), nodes_before);
    let (lo, hi) = grid.range();
    let maxg = g.edges.iter().map(|e| e.max_grade_pct).fold(0.0f32, f32::max);
    let _ = writeln!(s, "elevation {lo:.1}..{hi:.1} m; max edge grade {maxg:.1}%; sunrise {} min, sunset {} min", g.meta.sunrise_min, g.meta.sunset_min);
    let _ = writeln!(s, "foot allowed      {}", pct(&|e| e.foot));
    let _ = writeln!(s, "bike allowed      {}", pct(&|e| e.bike));
    let _ = writeln!(s, "lit tagged in OSM {}", pct(&|e| e.lit_src.as_deref() == Some("tag")));
    let _ = writeln!(s, "lit known (after rules) {}  ({} street lamps in OSM; campus = {})", pct(&|e| e.lit.is_some()), night.n_lamps, if night.campus_names.is_empty() { "none".into() } else { night.campus_names.join(" + ") });
    for src in ["tag", "lamp", "road", "campus", "trail"] {
        let _ = writeln!(s, "  lit_src={src:7} {}", pct(&|e| e.lit_src.as_deref() == Some(src)));
    }
    let _ = writeln!(s, "  lit=yes         {}", pct(&|e| e.lit == Some(true)));
    let _ = writeln!(s, "  lit=no          {}", pct(&|e| e.lit == Some(false)));
    let _ = writeln!(s, "surface tagged    {}", pct(&|e| e.surface_raw.is_some()));
    let _ = writeln!(s, "surface known     {}  (untagged car roads assumed paved)", pct(&|e| e.surface.is_some()));
    let _ = writeln!(s, "  not paved       {}", pct(&|e| e.surface.is_some_and(|x| x != Surface::Paved)));
    let _ = writeln!(s, "steps             {}", pct(&|e| e.steps));
    let _ = writeln!(s, "covered           {}", pct(&|e| e.covered));
    let _ = writeln!(s, "wheelchair tagged {}", pct(&|e| e.wheelchair.is_some()));
    let _ = writeln!(s, "grade > 5%        {}", pct(&|e| e.max_grade_pct > 5.0));
    for b in 0..=3u8 {
        let _ = writeln!(s, "busy={b}            {}", pct(&|e| e.busy == b));
    }
    for l in 1..=4u8 {
        let _ = writeln!(s, "bike lts={l}        {}", pct(&|e| e.lts == Some(l)));
    }
    for (name, c) in [("track", Cycleway::Track), ("lane", Cycleway::Lane), ("shared", Cycleway::Shared)] {
        let _ = writeln!(s, "cycleway={name:7}  {}", pct(&|e| e.cycleway == Some(c)));
    }
    let _ = writeln!(s, "bench within 100 m {}", pct(&|e| e.near_bench_m <= 100.0));
    let _ = writeln!(s, "toilets within 250 m {}", pct(&|e| e.near_toilets_m <= 250.0));
    let _ = writeln!(s, "water within 250 m {}", pct(&|e| e.near_water_m <= 250.0));
    let mut cats: BTreeMap<&str, usize> = BTreeMap::new();
    for pl in &p.places {
        *cats.entry(&pl.category).or_default() += 1;
    }
    let named = p.places.iter().filter(|x| x.name.is_some()).count();
    let _ = writeln!(s, "places {} ({} named): {}", p.places.len(), named, cats.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "));
    s
}
