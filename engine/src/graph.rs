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

impl Graph {
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
