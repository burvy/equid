//! Path factors from task 027 (contracts §2, filled after the edges exist):
//!  flood         share of the edge inside a FEMA NFHL 1% or 0.2% annual-chance flood zone (raw flood.json)
//!  crossings     highway=crossing nodes, charged to the way that crosses (footway=crossing ...), once each
//!  crossings_unsignalled  the subset without crossing=traffic_signals / crossing:signals=yes
//!  stops         highway=stop|give_way|traffic_signals nodes on roads, once each (bike stop-and-go)
//!  kerb_raised   kerb=raised on crossing or kerb nodes, once each on a walkable edge (wheelchair barrier)
//!  sidewalk      roads only: sidewalk tagged present (both/left/right/yes/separate) or absent (no/none)
//!  green, water_view  share of the edge within NEAR_M of green space / open water (raw landscape.json)
//!
//! "Once each": a node lies on every edge that meets there, so a count is charged to one edge only (the
//! first eligible one), otherwise a route through the node would pay twice.

use crate::geo::{dist_to_segment, Proj};
use crate::tags::{get, road_class, Tags};
use engine::graph::Edge;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;

/// "Near" green space / water: within this many metres of the area or line.
pub const NEAR_M: f64 = 30.0;
/// Sample spacing along an edge for the shares.
const STEP_M: f64 = 5.0;

type P = (f64, f64);

/// A set of polygons (even-odd over all ring segments, so multipolygon members need no assembly)
/// and lines, with a segment grid for distance queries.
#[derive(Default)]
pub struct Layer {
    polys: Vec<([f64; 4], Vec<(P, P)>)>,
    grid: HashMap<(i64, i64), Vec<(P, P)>>,
    pub n_features: usize,
}

fn cell(p: P) -> (i64, i64) {
    ((p.0 / NEAR_M).floor() as i64, (p.1 / NEAR_M).floor() as i64)
}

impl Layer {
    fn add(&mut self, rings: Vec<Vec<P>>, area: bool) {
        let segs: Vec<(P, P)> = rings.iter().flat_map(|r| r.windows(2).map(|w| (w[0], w[1]))).collect();
        if segs.is_empty() {
            return;
        }
        self.n_features += 1;
        for &(a, b) in &segs {
            let (ca, cb) = (cell(a), cell(b));
            for cx in ca.0.min(cb.0)..=ca.0.max(cb.0) {
                for cy in ca.1.min(cb.1)..=ca.1.max(cb.1) {
                    self.grid.entry((cx, cy)).or_default().push((a, b));
                }
            }
        }
        if area {
            let mut bb = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for &(a, _) in &segs {
                bb = [bb[0].min(a.0), bb[1].min(a.1), bb[2].max(a.0), bb[3].max(a.1)];
            }
            self.polys.push((bb, segs));
        }
    }

    pub fn inside(&self, p: P) -> bool {
        self.polys.iter().any(|(bb, segs)| {
            if p.0 < bb[0] || p.0 > bb[2] || p.1 < bb[1] || p.1 > bb[3] {
                return false;
            }
            let mut inside = false;
            for &(a, b) in segs {
                if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                    inside = !inside;
                }
            }
            inside
        })
    }

    /// Inside an area or within NEAR_M of any outline / line.
    pub fn near(&self, p: P) -> bool {
        let c = cell(p);
        for cx in c.0 - 1..=c.0 + 1 {
            for cy in c.1 - 1..=c.1 + 1 {
                if self.grid.get(&(cx, cy)).is_some_and(|v| v.iter().any(|s| dist_to_segment(p, s.0, s.1) <= NEAR_M)) {
                    return true;
                }
            }
        }
        self.inside(p)
    }
}

/// Node tags that matter here, by OSM node id.
#[derive(Default, Clone, Copy)]
struct NodeInfo {
    crossing: bool,
    signalled: bool,
    stop: bool,
    kerb_raised: bool,
}

pub struct Factors {
    nodes: HashMap<i64, NodeInfo>,
    pub flood: Option<Layer>,
    pub green: Layer,
    pub water: Layer,
}

fn ring(g: &[Value], proj: &Proj) -> Vec<P> {
    g.iter().filter_map(|p| Some(proj.xy(p["lon"].as_f64()?, p["lat"].as_f64()?))).collect()
}

impl Factors {
    /// From the region's nodes.json, landscape.json (Overpass) and flood.json (NFHL GeoJSON, optional).
    pub fn load(nodes: &Value, landscape: &Value, flood: Option<&Value>, proj: &Proj) -> Factors {
        let mut info = HashMap::new();
        for e in nodes["elements"].as_array().into_iter().flatten() {
            let (Some(id), t) = (e["id"].as_i64(), &e["tags"]) else { continue };
            let s = |k: &str| t[k].as_str().unwrap_or("");
            let crossing = s("highway") == "crossing" || (!s("crossing").is_empty() && s("crossing") != "no");
            let n = NodeInfo {
                crossing,
                signalled: s("crossing") == "traffic_signals" || s("crossing:signals") == "yes",
                stop: matches!(s("highway"), "stop" | "give_way" | "traffic_signals"),
                kerb_raised: s("kerb") == "raised",
            };
            if n.crossing || n.stop || n.kerb_raised {
                info.insert(id, n);
            }
        }

        let (mut green, mut water) = (Layer::default(), Layer::default());
        for e in landscape["elements"].as_array().into_iter().flatten() {
            let t = &e["tags"];
            let s = |k: &str| t[k].as_str().unwrap_or("");
            let is_water = s("natural") == "water" || s("waterway") == "riverbank";
            let is_line_water = matches!(s("waterway"), "river" | "canal" | "stream");
            let rings: Vec<Vec<P>> = match e["type"].as_str() {
                Some("way") => e["geometry"].as_array().map(|g| vec![ring(g, proj)]).unwrap_or_default(),
                Some("relation") => e["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| m["geometry"].as_array().map(|g| ring(g, proj)))
                    .collect(),
                _ => vec![],
            };
            if rings.is_empty() {
                continue;
            }
            // an area: a closed way, or a multipolygon relation
            let area = match e["type"].as_str() {
                Some("way") => rings[0].len() > 3 && rings[0].first() == rings[0].last(),
                _ => s("type") == "multipolygon",
            };
            if is_line_water && !is_water {
                water.add(rings, false);
            } else if is_water {
                water.add(rings, area);
            } else {
                green.add(rings, area);
            }
        }

        let flood = flood.map(|f| {
            let mut l = Layer::default();
            for ft in f["features"].as_array().into_iter().flatten() {
                let polys: Vec<&Value> = match ft["geometry"]["type"].as_str() {
                    Some("Polygon") => vec![&ft["geometry"]["coordinates"]],
                    Some("MultiPolygon") => ft["geometry"]["coordinates"].as_array().into_iter().flatten().collect(),
                    _ => vec![],
                };
                for poly in polys {
                    let rings: Vec<Vec<P>> = poly
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|r| {
                            r.as_array().into_iter().flatten().filter_map(|c| Some(proj.xy(c[0].as_f64()?, c[1].as_f64()?))).collect()
                        })
                        .collect();
                    l.add(rings, true);
                }
            }
            l
        });
        Factors { nodes: info, flood, green, water }
    }

    /// Fill the factor fields. `ids[i]` are the OSM node ids along edge i, `tags[i]` its way's tags.
    pub fn fill(&self, edges: &mut [Edge], ids: &[Vec<i64>], tags: &[&Tags], proj: &Proj) -> Report {
        let mut rep = Report::default();
        // shares, sampled every STEP_M metres (midpoints of equal pieces)
        for e in edges.iter_mut() {
            let xy: Vec<P> = e.geom.iter().map(|g| proj.xy(g[0], g[1])).collect();
            let (mut total, mut fl, mut gr, mut wa) = (0.0, 0.0, 0.0, 0.0);
            for s in xy.windows(2) {
                let d = crate::geo::dist(s[0], s[1]);
                let n = (d / STEP_M).ceil().max(1.0) as usize;
                for k in 0..n {
                    let f = (k as f64 + 0.5) / n as f64;
                    let p = (s[0].0 + (s[1].0 - s[0].0) * f, s[0].1 + (s[1].1 - s[0].1) * f);
                    let w = d / n as f64;
                    total += w;
                    if self.flood.as_ref().is_some_and(|l| l.inside(p)) {
                        fl += w;
                    }
                    if self.green.near(p) {
                        gr += w;
                    }
                    if self.water.near(p) {
                        wa += w;
                    }
                }
            }
            let share = |x: f64| if total > 0.0 { ((x / total * 100.0).round() / 100.0) as f32 } else { 0.0 };
            e.flood = self.flood.as_ref().map(|_| share(fl));
            e.green = Some(share(gr));
            e.water_view = Some(share(wa));
        }

        // sidewalk on roads
        for (e, t) in edges.iter_mut().zip(tags) {
            if road_class(&e.highway) > 0 {
                e.sidewalk = sidewalk(t);
            }
        }

        // node counts, each node charged to one edge: the first eligible edge that touches it
        let crossing_way = |t: &Tags| {
            ["footway", "cycleway", "path"].iter().any(|k| get(t, k) == Some("crossing")) || get(t, "crossing").is_some()
        };
        let mut touching: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, ns) in ids.iter().enumerate() {
            for n in ns {
                if self.nodes.contains_key(n) {
                    touching.entry(*n).or_default().push(i);
                }
            }
        }
        let is_road: Vec<bool> = edges.iter().map(|e| road_class(&e.highway) > 0).collect();
        let walk: Vec<bool> = edges.iter().zip(&is_road).map(|(e, r)| !r && e.foot).collect();
        let mut node_ids: Vec<&i64> = touching.keys().collect();
        node_ids.sort();
        for n in node_ids {
            let es = &touching[n];
            let info = self.nodes[n];
            let first = |ok: &dyn Fn(usize) -> bool| es.iter().copied().find(|&i| ok(i));
            let walkable = |i: usize| walk[i];
            if info.crossing {
                rep.crossing_nodes += 1;
                // the crossing way; else a walkable path through the node; else nobody
                match first(&|i| crossing_way(tags[i])).or_else(|| first(&walkable)) {
                    Some(i) => {
                        edges[i].crossings = edges[i].crossings.saturating_add(1);
                        if !info.signalled {
                            edges[i].crossings_unsignalled = edges[i].crossings_unsignalled.saturating_add(1);
                        }
                    }
                    None => rep.crossings_on_roads_only += 1,
                }
            }
            if info.stop {
                if let Some(i) = first(&|i| is_road[i]) {
                    edges[i].stops = edges[i].stops.saturating_add(1);
                    rep.stops += 1;
                }
            }
            if info.kerb_raised {
                if let Some(i) = first(&|i| crossing_way(tags[i])).or_else(|| first(&walkable)).or_else(|| first(&|_| true)) {
                    edges[i].kerb_raised = edges[i].kerb_raised.saturating_add(1);
                    rep.kerb_raised += 1;
                }
            }
        }
        rep.green_features = self.green.n_features;
        rep.water_features = self.water.n_features;
        rep.flood_features = self.flood.as_ref().map(|l| l.n_features);
        rep
    }
}

/// Sidewalk present / absent from `sidewalk`, `sidewalk:both|left|right`. None = not tagged.
pub fn sidewalk(t: &Tags) -> Option<bool> {
    let yes = |v: &str| matches!(v, "both" | "left" | "right" | "yes" | "separate");
    let no = |v: &str| matches!(v, "no" | "none");
    if let Some(v) = get(t, "sidewalk") {
        if yes(v) {
            return Some(true);
        }
        if no(v) {
            return Some(false);
        }
    }
    let sides: Vec<&str> = ["sidewalk:both", "sidewalk:left", "sidewalk:right"].iter().filter_map(|k| get(t, k)).collect();
    if sides.iter().any(|v| yes(v)) {
        Some(true)
    } else if !sides.is_empty() && sides.iter().all(|v| no(v)) {
        Some(false)
    } else {
        None
    }
}

#[derive(Default)]
pub struct Report {
    crossing_nodes: usize,
    crossings_on_roads_only: usize,
    stops: usize,
    kerb_raised: usize,
    green_features: usize,
    water_features: usize,
    flood_features: Option<usize>,
}

pub fn stats(edges: &[Edge], r: &Report) -> String {
    let n = edges.len() as f64;
    let km: f64 = edges.iter().map(|e| e.length_m as f64).sum::<f64>() / 1000.0;
    let pct = |f: &dyn Fn(&Edge) -> bool| {
        let c = edges.iter().filter(|e| f(e)).count() as f64;
        let l: f64 = 0.0 + edges.iter().filter(|e| f(e)).map(|e| e.length_m as f64).sum::<f64>() / 1000.0;
        format!("{:5.1}% of edges, {:5.1}% of length", 100.0 * c / n, 100.0 * l / km)
    };
    let len_share = |f: &dyn Fn(&Edge) -> f32| 100.0 * edges.iter().map(|e| (e.length_m * f(e)) as f64).sum::<f64>() / 1000.0 / km;
    let roads: Vec<&Edge> = edges.iter().filter(|e| road_class(&e.highway) > 0).collect();
    let road_km: f64 = roads.iter().map(|e| e.length_m as f64).sum::<f64>() / 1000.0;
    let road_pct = |v: Option<bool>| 100.0 * roads.iter().filter(|e| e.sidewalk == v).map(|e| e.length_m as f64).sum::<f64>() / 1000.0 / road_km;
    let mut s = String::new();
    match r.flood_features {
        Some(k) => {
            let _ = writeln!(s, "flood zone (FEMA NFHL 1% + 0.2%, {k} polygons): {:.1}% of length; edges touching one {}", len_share(&|e| e.flood.unwrap_or(0.0)), pct(&|e| e.flood.unwrap_or(0.0) > 0.0));
        }
        None => {
            let _ = writeln!(s, "flood zone: no NFHL data (flood = null)");
        }
    }
    let _ = writeln!(s, "green within {NEAR_M} m ({} OSM features): {:.1}% of length", r.green_features, len_share(&|e| e.green.unwrap_or(0.0)));
    let _ = writeln!(s, "water within {NEAR_M} m ({} OSM features): {:.1}% of length", r.water_features, len_share(&|e| e.water_view.unwrap_or(0.0)));
    let tot = |f: &dyn Fn(&Edge) -> u8| edges.iter().map(|e| f(e) as usize).sum::<usize>();
    let _ = writeln!(
        s,
        "crossings {} of {} OSM crossing nodes charged ({} unsignalled; {} sit on roads with no mapped crossing path: not charged)",
        tot(&|e| e.crossings),
        r.crossing_nodes,
        tot(&|e| e.crossings_unsignalled),
        r.crossings_on_roads_only
    );
    let _ = writeln!(s, "stops (stop / give_way / signals on roads) {}; kerb=raised {} (OSM kerb tags are sparse)", r.stops, r.kerb_raised);
    let _ = writeln!(
        s,
        "sidewalk on roads ({road_km:.1} km): yes {:.1}%, no {:.1}%, untagged {:.1}% of road length",
        road_pct(Some(true)),
        road_pct(Some(false)),
        road_pct(None)
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tags(v: Value) -> Tags {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn sidewalk_tags() {
        assert_eq!(sidewalk(&tags(json!({"sidewalk":"both"}))), Some(true));
        assert_eq!(sidewalk(&tags(json!({"sidewalk":"separate"}))), Some(true));
        assert_eq!(sidewalk(&tags(json!({"sidewalk":"no"}))), Some(false));
        assert_eq!(sidewalk(&tags(json!({"sidewalk:left":"no","sidewalk:right":"yes"}))), Some(true));
        assert_eq!(sidewalk(&tags(json!({"sidewalk:both":"no"}))), Some(false));
        assert_eq!(sidewalk(&tags(json!({"highway":"residential"}))), None);
    }

    #[test]
    fn layer_inside_and_near() {
        let mut l = Layer::default();
        let sq = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0), (0.0, 0.0)];
        l.add(vec![sq], true);
        assert!(l.inside((50.0, 50.0)));
        assert!(!l.inside((150.0, 50.0)));
        assert!(l.near((120.0, 50.0)));
        assert!(!l.near((140.0, 50.0)));
        let mut line = Layer::default();
        line.add(vec![vec![(0.0, 0.0), (100.0, 0.0)]], false);
        assert!(!line.inside((50.0, 1.0)) && line.near((50.0, 25.0)) && !line.near((50.0, 35.0)));
    }
}
