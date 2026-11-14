//! Equid engine. Native Rust API (`Core`) used by tests, plus a thin wasm-bindgen wrapper
//! (`wasm::Engine`) the web app calls. Contract: docs/contracts.md.

pub mod features;
pub mod graph;
pub mod metric;
pub mod wasm;

use graph::{Graph, Places};
use metric::Metric;
use petgraph::algo::dijkstra;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::{EdgeFiltered, EdgeRef};

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
    /// Directed petgraph view, built once at load. Undirected edge i becomes arc 2i (from->to,
    /// weight (i, true)) and arc 2i+1 (to->from, weight (i, false)); costs are filled per query.
    arcs: DiGraph<(), (u32, bool), u32>,
}

impl Core {
    pub fn load(graph_json: &str, places_json: &str) -> Result<Core, String> {
        let graph: Graph = serde_json::from_str(graph_json).map_err(|e| format!("graph.json: {e}"))?;
        graph.validate()?;
        let places: Places = serde_json::from_str(places_json).map_err(|e| format!("places.json: {e}"))?;
        places.validate(&graph)?;
        let mut arcs = DiGraph::with_capacity(graph.nodes.len(), graph.edges.len() * 2);
        for _ in &graph.nodes {
            arcs.add_node(());
        }
        for (i, e) in graph.edges.iter().enumerate() {
            arcs.add_edge(NodeIndex::new(e.from as usize), NodeIndex::new(e.to as usize), (i as u32, true));
            arcs.add_edge(NodeIndex::new(e.to as usize), NodeIndex::new(e.from as usize), (i as u32, false));
        }
        Ok(Core { graph, places, arcs })
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

    /// Cost of every arc (index 2i = edge i forward, 2i+1 = backward) under `metric` at `t_min`.
    /// `INFINITY` = impassable. All finite costs are >= 0 (weights >= 0, features >= 0).
    pub fn arc_costs(&self, metric: &Metric, t_min: f32) -> Vec<f32> {
        let meta = &self.graph.meta;
        let mut out = Vec::with_capacity(self.graph.edges.len() * 2);
        for e in &self.graph.edges {
            for forward in [true, false] {
                out.push(metric.edge_cost(e, forward, meta, t_min).unwrap_or(f32::INFINITY));
            }
        }
        out
    }

    /// Cost field from `origin` under `metric` at local time `t_min` (the departure time; costs
    /// are evaluated at that time for the whole trip). Dijkstra from petgraph over the arc costs;
    /// impassable arcs are filtered out, so unreachable nodes/places get `f32::INFINITY`.
    pub fn field(&self, origin: u32, metric: &Metric, t_min: f32) -> Result<FieldResult, String> {
        if origin as usize >= self.graph.nodes.len() {
            return Err("origin node out of range".into());
        }
        let costs = self.arc_costs(metric, t_min);
        let passable = EdgeFiltered::from_fn(&self.arcs, |e| costs[e.id().index()].is_finite());
        let dist = dijkstra(&passable, NodeIndex::new(origin as usize), None, |e| costs[e.id().index()]);
        let mut node_cost = vec![f32::INFINITY; self.graph.nodes.len()];
        for (n, d) in dist {
            node_cost[n.index()] = d;
        }
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
