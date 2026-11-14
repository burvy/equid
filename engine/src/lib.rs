//! Equid engine. Native Rust API (`Core`) used by tests, plus a thin wasm-bindgen wrapper
//! (`wasm::Engine`) the web app calls. Contract: docs/contracts.md.

pub mod features;
pub mod graph;
pub mod metric;
pub mod wasm;

use graph::{Graph, Places};
use metric::Metric;

/// Result of `field`: cost (seconds-equivalent) from the origin to every node and every place,
/// in file order. `f32::INFINITY` = unreachable under this metric.
#[derive(Debug, Clone)]
pub struct FieldResult {
    pub node_cost: Vec<f32>,
    pub place_cost: Vec<f32>,
}

pub struct Core {
    pub graph: Graph,
    pub places: Places,
}

impl Core {
    pub fn load(graph_json: &str, places_json: &str) -> Result<Core, String> {
        let graph: Graph = serde_json::from_str(graph_json).map_err(|e| format!("graph.json: {e}"))?;
        graph.validate()?;
        let places: Places = serde_json::from_str(places_json).map_err(|e| format!("places.json: {e}"))?;
        places.validate(&graph)?;
        Ok(Core { graph, places })
    }

    /// Nearest graph node to a point (linear scan; fine for ~10k nodes, called once per click).
    pub fn nearest_node(&self, lon: f64, lat: f64) -> u32 {
        let k = lat.to_radians().cos(); // shrink longitude so the comparison is roughly metric
        self.graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (i, ((n.lon - lon) * k).powi(2) + (n.lat - lat).powi(2)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i as u32)
            .unwrap_or(0)
    }

    /// Cost field from `origin` under `metric` at local time `t_min`.
    /// STUB (task 001): straight-line distance / mode speed. Task 003 replaces the body with
    /// petgraph Dijkstra over `metric.edge_cost`; the signature and output stay the same.
    pub fn field(&self, origin: u32, metric: &Metric, _t_min: f32) -> Result<FieldResult, String> {
        let o = self.graph.nodes.get(origin as usize).ok_or("origin node out of range")?;
        let speed = metric.mode.speed_mps() as f64;
        let node_cost: Vec<f32> = self
            .graph
            .nodes
            .iter()
            .map(|n| (haversine_m(o.lon, o.lat, n.lon, n.lat) / speed) as f32)
            .collect();
        let place_cost = self.places.places.iter().map(|p| node_cost[p.node as usize]).collect();
        Ok(FieldResult { node_cost, place_cost })
    }
}

pub fn haversine_m(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6_371_000.0 * a.sqrt().asin()
}
