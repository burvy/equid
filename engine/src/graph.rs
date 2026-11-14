//! Data contract types: `graph.json` and `places.json` (see docs/contracts.md).
//! These structs ARE the contract: if a file parses into them, it is valid.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub version: u32,
    pub region: String,
    /// [west, south, east, north] in degrees
    pub bbox: [f64; 4],
    pub tz: String,
    /// Demo date (YYYY-MM-DD) the sun times and shade slots were computed for.
    pub date: String,
    /// Local minutes since midnight.
    pub sunrise_min: f32,
    pub sunset_min: f32,
    /// Length of one time slot in minutes (30) and number of slots (48) for time-dependent arrays.
    pub slot_min: u32,
    pub slots: u32,
    #[serde(default)]
    pub generated: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub lon: f64,
    pub lat: f64,
    pub elev_m: f32,
    #[serde(default)]
    pub osm: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    Paved,
    Compacted,
    Unpaved,
    Rough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cycleway {
    Track,
    Lane,
    Shared,
    None,
}

/// One street segment between two graph nodes, stored ONCE (undirected).
/// Direction-dependent fields (climb/descent, oneway_bike) are given for from -> to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: u32,
    pub to: u32,
    #[serde(default)]
    pub osm: Option<i64>,
    pub length_m: f32,
    /// Shape polyline from -> to, including both endpoints: [lon, lat, elev_m].
    pub geom: Vec<[f64; 3]>,
    pub climb_m: f32,
    pub descent_m: f32,
    pub max_grade_pct: f32,
    pub highway: String,
    pub foot: bool,
    pub bike: bool,
    /// 0 = both ways, 1 = bike only from -> to, -1 = bike only to -> from.
    #[serde(default)]
    pub oneway_bike: i8,
    #[serde(default)]
    pub steps: bool,
    #[serde(default)]
    pub surface: Option<Surface>,
    #[serde(default)]
    pub surface_raw: Option<String>,
    #[serde(default)]
    pub lit: Option<bool>,
    /// Where `lit` comes from: "tag" (OSM lit), or an inference rule: "lamp", "road", "campus", "trail". null when lit is null.
    #[serde(default)]
    pub lit_src: Option<String>,
    #[serde(default)]
    pub covered: bool,
    /// Adjacent traffic: 0 path away from roads, 1 residential/service, 2 tertiary/secondary, 3 primary/trunk.
    #[serde(default)]
    pub busy: u8,
    #[serde(default)]
    pub cycleway: Option<Cycleway>,
    /// Bike level of traffic stress 1..4, null when bike = false or unknown.
    #[serde(default)]
    pub lts: Option<u8>,
    #[serde(default)]
    pub wheelchair: Option<String>,
    /// Distance from the edge to the nearest bench / toilets / drinking water, capped at 500 m.
    #[serde(default = "cap")]
    pub near_bench_m: f32,
    #[serde(default = "cap")]
    pub near_toilets_m: f32,
    #[serde(default = "cap")]
    pub near_water_m: f32,
    /// Static tree-cover fraction 0..1 over the edge (later; null = unknown).
    #[serde(default)]
    pub tree: Option<f32>,
    /// Shade fraction 0..1 per time slot (meta.slots values), null until the LiDAR task lands.
    #[serde(default)]
    pub shade: Option<Vec<f32>>,
    /// Share 0..1 of the edge inside a FEMA 1% / 0.2% flood zone (null = unknown, counts 0).
    #[serde(default)]
    pub flood: Option<f32>,
    /// Marked crossings on the edge, and the subset with no signal or stop.
    #[serde(default)]
    pub crossings: u8,
    #[serde(default)]
    pub crossings_unsignalled: u8,
    /// Sidewalk on a road: true = yes, false = tagged none, null = unknown or not a road.
    #[serde(default)]
    pub sidewalk: Option<bool>,
    /// Stop signs, give-way signs and traffic signals on this road edge (each node charged once); a stop for bikes.
    #[serde(default)]
    pub stops: u8,
    /// Crossings with a raised kerb (a barrier for wheelchairs).
    #[serde(default)]
    pub kerb_raised: u8,
    /// Share 0..1 within ~30 m of green space / open water (null = unknown, counts as scenic).
    #[serde(default)]
    pub green: Option<f32>,
    #[serde(default)]
    pub water_view: Option<f32>,
    /// Derived at load (never in graph.json): street crossings at the edge's junctions, see `Graph::derive_junctions`.
    #[serde(skip)]
    pub junction_crossings: f32,
}

pub const NEAR_CAP_M: f32 = 500.0;
fn cap() -> f32 {
    NEAR_CAP_M
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    pub meta: Meta,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Place {
    /// OSM type letter + id, e.g. "n123", "w456".
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub lon: f64,
    pub lat: f64,
    pub category: String,
    #[serde(default)]
    pub tags: serde_json::Map<String, serde_json::Value>,
    /// Nearest graph node index, and the distance to it.
    pub node: u32,
    pub snap_m: f32,
    #[serde(default)]
    pub rating: Option<f32>,
    /// Number of reviews behind `rating`.
    #[serde(default)]
    pub n: Option<u32>,
    #[serde(default)]
    pub rating_src: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Places {
    pub version: u32,
    pub places: Vec<Place>,
}

/// Car roads a pedestrian can walk along or must cross; weight = how much of a crossing passing one costs.
pub fn road_class_weight(highway: &str) -> Option<f32> {
    match highway {
        "primary" | "trunk" | "primary_link" | "trunk_link" => Some(1.0),
        "secondary" | "tertiary" | "secondary_link" | "tertiary_link" => Some(0.7),
        "residential" | "service" | "unclassified" | "living_street" => Some(0.3),
        _ => None,
    }
}

impl Graph {
    /// Fill `junction_crossings` from topology: walking a car road, each junction at either end charges
    /// 0.5 x (sum of the class weights of the OTHER car roads there, minus the largest, which is taken as the road
    /// continuing straight). So a walk along a primary past a residential T pays 0.3 per junction (0.15 per end of each
    /// of the two edges), a bend or dead end pays 0. Approximation: turning into a side street is charged as if passing it.
    /// Footways and mapped crossing ways get 0 here; their crossings are in `crossings`.
    pub fn derive_junctions(&mut self) {
        let mut inc: Vec<Vec<usize>> = vec![vec![]; self.nodes.len()];
        for (i, e) in self.edges.iter().enumerate() {
            if e.foot && road_class_weight(&e.highway).is_some() {
                inc[e.from as usize].push(i);
                inc[e.to as usize].push(i);
            }
        }
        let charge = |at: usize, me: usize, edges: &[Edge]| -> f32 {
            let (mut sum, mut max) = (0.0f32, 0.0f32);
            for &j in inc[at].iter().filter(|&&j| j != me) {
                let w = road_class_weight(&edges[j].highway).unwrap_or(0.0);
                sum += w;
                max = max.max(w);
            }
            0.5 * (sum - max)
        };
        let vals: Vec<f32> = (0..self.edges.len())
            .map(|i| {
                let e = &self.edges[i];
                if e.foot && road_class_weight(&e.highway).is_some() {
                    charge(e.from as usize, i, &self.edges) + charge(e.to as usize, i, &self.edges)
                } else {
                    0.0
                }
            })
            .collect();
        for (e, v) in self.edges.iter_mut().zip(vals) {
            e.junction_crossings = v;
        }
    }

    /// Checks the invariants the engine relies on (indices in range, non-negative lengths, ...).
    pub fn validate(&self) -> Result<(), String> {
        let n = self.nodes.len() as u32;
        let slots = self.meta.slots as usize;
        for (i, e) in self.edges.iter().enumerate() {
            if e.from >= n || e.to >= n {
                return Err(format!("edge {i}: node index out of range"));
            }
            if !(e.length_m >= 0.0 && e.climb_m >= 0.0 && e.descent_m >= 0.0 && e.max_grade_pct >= 0.0) {
                return Err(format!("edge {i}: negative or NaN length/climb/descent/grade"));
            }
            if e.geom.len() < 2 {
                return Err(format!("edge {i}: geom needs at least both endpoints"));
            }
            for (name, v) in [("flood", e.flood), ("green", e.green), ("water_view", e.water_view)] {
                if v.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
                    return Err(format!("edge {i}: {name} must be in [0,1]"));
                }
            }
            if e.crossings_unsignalled > e.crossings {
                return Err(format!("edge {i}: crossings_unsignalled exceeds crossings"));
            }
            if let Some(s) = &e.shade {
                if s.len() != slots || s.iter().any(|v| !(0.0..=1.0).contains(v)) {
                    return Err(format!("edge {i}: shade must be {slots} values in [0,1]"));
                }
            }
        }
        Ok(())
    }
}

impl Places {
    pub fn validate(&self, graph: &Graph) -> Result<(), String> {
        let n = graph.nodes.len() as u32;
        for p in &self.places {
            if p.node >= n {
                return Err(format!("place {}: node index out of range", p.id));
            }
        }
        Ok(())
    }
}
