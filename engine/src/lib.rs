//! Equid engine. Native Rust API (`Core`) used by tests, plus a thin wasm-bindgen wrapper
//! (`wasm::Engine`) the web app calls. Contract: docs/contracts.md.

pub mod features;
pub mod graph;
pub mod metric;
pub mod view;
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

/// Why a place costs what it costs: the cheapest route's totals per feature (task 017 `explain`).
#[derive(Debug, Clone)]
pub struct Explain {
    /// Graph nodes from origin to target.
    pub path: Vec<u32>,
    pub length_m: f32,
    /// Total cost along the path (= field value at the target).
    pub total: f32,
    /// (feature id, raw amount in the feature's unit, weight, weight x raw = seconds-equivalent), only weight > 0 or raw > 0.
    pub terms: Vec<(&'static str, f32, f32, f32)>,
}

pub struct Core {
    pub graph: Graph,
    pub places: Places,
    /// Directed petgraph view, built once at load. Undirected edge i becomes arc 2i (from->to,
    /// weight (i, true)) and arc 2i+1 (to->from, weight (i, false)); costs are filled per query.
    arcs: DiGraph<(), (u32, bool), u32>,
    shapes: view::Shapes,
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
        let shapes = view::Shapes::new(&graph);
        Ok(Core { graph, places, arcs, shapes })
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

    /// GPU-ready buffers for the map / warp / 3D views from a field (see `view`).
    pub fn view_buffers(&self, field: &FieldResult, origin: u32, p: &view::ViewParams) -> Result<view::ViewBuffers, String> {
        if origin as usize >= self.graph.nodes.len() {
            return Err("origin node out of range".into());
        }
        if field.node_cost.len() != self.graph.nodes.len() || field.place_cost.len() != self.places.places.len() {
            return Err("field does not match this graph".into());
        }
        let pts: Vec<(f64, f64)> = self.places.places.iter().map(|q| (q.lon, q.lat)).collect();
        Ok(view::build(&self.graph, &self.shapes, &field.node_cost, &field.place_cost, &pts, origin, p))
    }

    /// Warp-view ring of `radius_m` around `origin`, as (dLon, dLat) offsets (97 points, closed).
    pub fn ring(&self, origin: u32, radius_m: f32) -> Result<Vec<f32>, String> {
        if origin as usize >= self.graph.nodes.len() {
            return Err("origin node out of range".into());
        }
        Ok(view::ring(&self.graph, &self.shapes, origin, radius_m, 96))
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

impl Core {
    /// Cheapest route origin -> `target` under `metric` at `t_min`, with its cost split by feature.
    /// Backtracks tight arcs of the Dijkstra field; `Err` if the target is unreachable.
    pub fn explain(&self, origin: u32, metric: &Metric, t_min: f32, target: u32) -> Result<Explain, String> {
        let d = self.field(origin, metric, t_min)?.node_cost;
        let n = self.graph.nodes.len();
        if target as usize >= n {
            return Err("target node out of range".into());
        }
        if !d[target as usize].is_finite() {
            return Err("unreachable under this metric".into());
        }
        let costs = self.arc_costs(metric, t_min);
        let meta = &self.graph.meta;
        let mut path = vec![target];
        let mut seen = vec![false; n];
        seen[target as usize] = true;
        let mut raw = vec![0.0f32; features::FEATURES.len()];
        let mut length_m = 0.0;
        let mut v = target as usize;
        while v != origin as usize {
            let best = self
                .arcs
                .edges_directed(NodeIndex::new(v), petgraph::Direction::Incoming)
                .filter(|a| costs[a.id().index()].is_finite() && !seen[a.source().index()])
                .map(|a| (a, (d[a.source().index()] + costs[a.id().index()] - d[v]).abs()))
                .min_by(|x, y| x.1.total_cmp(&y.1))
                .ok_or("no predecessor found")?
                .0;
            let (ei, fwd) = *best.weight();
            let e = &self.graph.edges[ei as usize];
            let f = features::edge_features(e, fwd, metric.mode, meta, t_min);
            for (r, x) in raw.iter_mut().zip(f.iter()) {
                *r += x;
            }
            length_m += e.length_m;
            v = best.source().index();
            seen[v] = true;
            path.push(v as u32);
        }
        path.reverse();
        let terms = features::FEATURES
            .iter()
            .enumerate()
            .filter(|(i, _)| raw[*i] > 0.0 && metric.weights[*i] > 0.0)
            .map(|(i, fi)| (fi.id, raw[i], metric.weights[i], raw[i] * metric.weights[i]))
            .collect();
        Ok(Explain { path, length_m, total: d[target as usize], terms })
    }
}

pub fn haversine_m(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6_371_000.0 * a.sqrt().asin()
}
