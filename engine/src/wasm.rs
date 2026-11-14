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
pub struct ViewBuffers(crate::view::ViewBuffers);

/// Getters copy into typed arrays (read each once). Positions are (dLon, dLat, z m) offsets from
/// (origin_lon, origin_lat): use deck.gl `COORDINATE_SYSTEM.LNGLAT_OFFSETS` + `coordinateOrigin`.
#[wasm_bindgen]
impl ViewBuffers {
    #[wasm_bindgen(getter)]
    pub fn positions(&self) -> Vec<f32> {
        self.0.positions.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn colors(&self) -> Vec<u8> {
        self.0.colors.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn vertex_cost(&self) -> Vec<f32> {
        self.0.vertex_cost.clone()
    }
    /// Path starts for deck's binary PathLayer `startIndices`: one per path, then the total vertex count appended.
    #[wasm_bindgen(getter)]
    pub fn start_indices(&self) -> Vec<u32> {
        let mut v = self.0.start_indices.clone();
        v.push(self.0.vertex_cost.len() as u32);
        v
    }
    #[wasm_bindgen(getter)]
    pub fn path_edge(&self) -> Vec<u32> {
        self.0.path_edge.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn place_positions(&self) -> Vec<f32> {
        self.0.place_positions.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn place_colors(&self) -> Vec<u8> {
        self.0.place_colors.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn origin_lon(&self) -> f64 {
        self.0.origin_lon
    }
    #[wasm_bindgen(getter)]
    pub fn origin_lat(&self) -> f64 {
        self.0.origin_lat
    }
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

    /// Buffers for the three views from a `field` result (docs/contracts.md section 4). `warp` 0..1 blends
    /// real -> warped positions, `lift` = metres of height per second of cost, `metres_per_s` = warp radius
    /// per second of cost, `budget_s` = cost where the colour ramp reaches red, `hide_beyond` (0..1) fades out
    /// beyond-budget streets/places and caps their height/radius at the budget (1 = drop them).
    pub fn view_buffers(&self, field: &Field, origin: u32, warp: f32, lift: f32, metres_per_s: f32, budget_s: f32, hide_beyond: f32) -> Result<ViewBuffers, JsError> {
        let r = crate::FieldResult { node_cost: field.node_cost.clone(), place_cost: field.place_cost.clone() };
        let p = crate::view::ViewParams { warp, lift, metres_per_s, budget_s, hide_beyond };
        self.core.view_buffers(&r, origin, &p).map(ViewBuffers).map_err(|e| JsError::new(&e))
    }

    /// Isochrone of `cost_s` seconds around `origin`: closed path, xyz offsets as in `view_buffers`, under the same
    /// warp/lift, so map-view isochrones morph into the warp circles. Empty if nothing reaches that cost.
    pub fn isochrone(&self, field: &Field, origin: u32, cost_s: f32, warp: f32, lift: f32, metres_per_s: f32) -> Result<Vec<f32>, JsError> {
        let r = crate::FieldResult { node_cost: field.node_cost.clone(), place_cost: Vec::new() };
        let p = crate::view::ViewParams { warp, lift, metres_per_s, budget_s: cost_s, hide_beyond: 0.0 };
        self.core.isochrone(&r, origin, cost_s, &p).map_err(|e| JsError::new(&e))
    }

    /// Closed circle (97 points, xy offsets as in `view_buffers`) of `radius_m` around `origin`: warp-view rings.
    pub fn ring(&self, origin: u32, radius_m: f32) -> Result<Vec<f32>, JsError> {
        self.core.ring(origin, radius_m).map_err(|e| JsError::new(&e))
    }

    /// Load an objects.json (open-vocabulary objects: `{"version":1,"objects":[{"label","lon","lat"}]}`).
    /// Afterwards `near:<label>` can be weighted in a metric. Returns the number of objects added.
    pub fn load_objects(&mut self, objects_json: &str) -> Result<u32, JsError> {
        self.core.load_objects(objects_json).map(|n| n as u32).map_err(|e| JsError::new(&e))
    }

    /// Add objects of one label: `points_json` = `[[lon,lat],...]`. Returns the label's total object count.
    pub fn add_objects(&mut self, label: &str, points_json: &str) -> Result<u32, JsError> {
        let pts: Vec<(f64, f64)> = serde_json::from_str(points_json).map_err(|e| JsError::new(&format!("points: {e}")))?;
        self.core.add_objects(label, &pts).map(|n| n as u32).map_err(|e| JsError::new(&e))
    }

    /// The label menu: `[{"label","objects","edges_near"}]` for every label with object data
    /// (plus the built-in `bench`, `toilets`, `water`, which are always available).
    pub fn labels_json(&self) -> String {
        serde_json::to_string(&self.core.labels()).unwrap()
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
