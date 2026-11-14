//! Browser API (wasm-bindgen). Thin: parse args, call `Core`, hand back typed arrays.
//! JS usage (see web/src/main.ts):
//!   const eng = new Engine(graphJson, placesJson);
//!   const f = eng.field(eng.nearest_node(lon, lat), JSON.stringify(preset), 14 * 60);
//!   f.node_cost  // Float32Array, one per node; f.place_cost // one per place

use crate::features::FEATURES;
use crate::metric::{Metric, PRESETS_JSON};
use crate::Core;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Engine {
    core: Core,
}

#[wasm_bindgen]
pub struct Field {
    node_cost: Vec<f32>,
    place_cost: Vec<f32>,
}

#[wasm_bindgen]
impl Field {
    /// Cost per node (seconds-equivalent), Infinity = unreachable. Copies into a Float32Array.
    #[wasm_bindgen(getter)]
    pub fn node_cost(&self) -> Vec<f32> {
        self.node_cost.clone()
    }
    /// Cost per place, in places.json order.
    #[wasm_bindgen(getter)]
    pub fn place_cost(&self) -> Vec<f32> {
        self.place_cost.clone()
    }
}

#[wasm_bindgen]
impl Engine {
    #[wasm_bindgen(constructor)]
    pub fn new(graph_json: &str, places_json: &str) -> Result<Engine, JsError> {
        Core::load(graph_json, places_json).map(|core| Engine { core }).map_err(|e| JsError::new(&e))
    }

    pub fn node_count(&self) -> u32 {
        self.core.graph.nodes.len() as u32
    }

    pub fn place_count(&self) -> u32 {
        self.core.places.places.len() as u32
    }

    pub fn nearest_node(&self, lon: f64, lat: f64) -> u32 {
        self.core.nearest_node(lon, lat)
    }

    /// `metric_json`: `{"mode":"foot","weights":{...},"forbid":[...]}` (a preset object works too).
    /// `t_min`: local minutes since midnight.
    pub fn field(&self, origin: u32, metric_json: &str, t_min: f32) -> Result<Field, JsError> {
        let metric = Metric::from_json(metric_json).map_err(|e| JsError::new(&e))?;
        let r = self.core.field(origin, &metric, t_min).map_err(|e| JsError::new(&e))?;
        Ok(Field { node_cost: r.node_cost, place_cost: r.place_cost })
    }

    /// `[{id, unit, desc}, ...]` in feature-vector order, for building sliders.
    pub fn features_json() -> String {
        let v: Vec<_> = FEATURES
            .iter()
            .map(|f| serde_json::json!({"id": f.id, "unit": f.unit, "desc": f.desc}))
            .collect();
        serde_json::to_string(&v).unwrap()
    }

    /// Contents of data/presets.json.
    pub fn presets_json() -> String {
        PRESETS_JSON.to_string()
    }
}
