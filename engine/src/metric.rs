//! A metric = mode + non-negative weights over features + forbidden features.
//! Presets (data/presets.json) are named metrics. JSON form (also what `field` takes):
//! `{"mode":"foot","weights":{"time":1,"climb":10},"forbid":["steps"]}`

use crate::features::{edge_features, feature_index, Mode, FEATURES};
use crate::graph::{Edge, Meta};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricSpec {
    #[serde(default)]
    pub mode: Mode,
    /// feature id -> weight a_i >= 0; missing features weigh 0.
    #[serde(default)]
    pub weights: BTreeMap<String, f32>,
    /// Feature ids that make an edge impassable when their value is > 0 (e.g. "steps" for wheelchair).
    #[serde(default)]
    pub forbid: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(flatten)]
    pub metric: MetricSpec,
}

/// Validated, index-based form of a MetricSpec, ready for the inner loop.
#[derive(Debug, Clone)]
pub struct Metric {
    pub mode: Mode,
    pub weights: Vec<f32>,
    pub forbid: Vec<bool>,
}

impl Metric {
    /// Rejects unknown feature ids and negative/NaN weights: a_i >= 0 is what keeps cost >= 0,
    /// which is the condition Dijkstra needs (and what the Lean proof states).
    pub fn compile(spec: &MetricSpec) -> Result<Metric, String> {
        let mut weights = vec![0.0; FEATURES.len()];
        for (id, &a) in &spec.weights {
            let i = feature_index(id).ok_or_else(|| format!("unknown feature '{id}'"))?;
            if !(a >= 0.0 && a.is_finite()) {
                return Err(format!("weight for '{id}' must be finite and >= 0, got {a}"));
            }
            weights[i] = a;
        }
        let mut forbid = vec![false; FEATURES.len()];
        for id in &spec.forbid {
            forbid[feature_index(id).ok_or_else(|| format!("unknown feature '{id}'"))?] = true;
        }
        Ok(Metric { mode: spec.mode, weights, forbid })
    }

    pub fn from_json(json: &str) -> Result<Metric, String> {
        let spec: MetricSpec = serde_json::from_str(json).map_err(|e| format!("bad metric JSON: {e}"))?;
        Metric::compile(&spec)
    }

    /// Cost of traversing `edge` forwards/backwards at time `t_min`, or None if impassable.
    pub fn edge_cost(&self, edge: &Edge, forward: bool, meta: &Meta, t_min: f32) -> Option<f32> {
        let allowed = match self.mode {
            Mode::Foot => edge.foot,
            Mode::Bike => edge.bike && !(forward && edge.oneway_bike == -1) && !(!forward && edge.oneway_bike == 1),
        };
        if !allowed {
            return None;
        }
        let f = edge_features(edge, forward, self.mode, meta, t_min);
        if f.iter().zip(&self.forbid).any(|(&v, &no)| no && v > 0.0) {
            return None;
        }
        Some(f.iter().zip(&self.weights).map(|(v, a)| v * a).sum())
    }
}

/// The presets shipped with the app (single source: data/presets.json).
pub const PRESETS_JSON: &str = include_str!("../../data/presets.json");

pub fn presets() -> Vec<Preset> {
    serde_json::from_str(PRESETS_JSON).expect("data/presets.json is valid")
}
