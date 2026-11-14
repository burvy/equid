//! Edge features f_i(edge, direction, t) >= 0 and the metric cost = Σ a_i · f_i with a_i >= 0.
//! Every "finder" (walk, stroller, shade, night, bike, ...) is just a weight vector over these.
//! Units: `time` is seconds; every other feature is metres of travel (or metres climbed) that
//! "count" for that feature, so a weight a_i reads as "seconds of penalty per metre".

use crate::graph::{Edge, Meta, Surface, NEAR_CAP_M};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Foot,
    Bike,
}

impl Mode {
    /// Base speed in m/s: walking 4.8 km/h, cycling 16 km/h.
    pub fn speed_mps(self) -> f32 {
        match self {
            Mode::Foot => 1.34,
            Mode::Bike => 4.44,
        }
    }
}

pub struct FeatureInfo {
    pub id: &'static str,
    pub unit: &'static str,
    pub desc: &'static str,
}

/// Order matters: it is the index into the feature vector. Append only.
pub const FEATURES: &[FeatureInfo] = &[
    FeatureInfo { id: "time", unit: "s", desc: "travel time at the mode's base speed" },
    FeatureInfo { id: "climb", unit: "m", desc: "metres climbed in the travel direction" },
    FeatureInfo { id: "steep", unit: "m", desc: "length x max(0, grade% - 5) / 5: 0 at <=5%, full length at 10%" },
    FeatureInfo { id: "steps", unit: "m", desc: "length of stairs" },
    FeatureInfo { id: "rough", unit: "m", desc: "length x roughness (paved 0, compacted .33, unpaved .67, rough 1, unknown .2)" },
    FeatureInfo { id: "busy", unit: "m", desc: "length x adjacent traffic (busy / 3)" },
    FeatureInfo { id: "unlit", unit: "m", desc: "at night only: length x (unlit 1, unknown .5, lit 0)" },
    FeatureInfo { id: "sun", unit: "m", desc: "by day only: length x (1 - shade(t)); shade from slots, else covered/tree cover" },
    FeatureInfo { id: "exposed", unit: "m", desc: "length not under cover (rain)" },
    FeatureInfo { id: "stress", unit: "m", desc: "length x (bike LTS - 1) / 3, unknown LTS counts .5" },
    FeatureInfo { id: "bench_gap", unit: "m", desc: "length x (distance to nearest bench / 500)" },
    FeatureInfo { id: "toilet_gap", unit: "m", desc: "length x (distance to nearest toilets / 500)" },
    FeatureInfo { id: "water_gap", unit: "m", desc: "length x (distance to nearest drinking water / 500)" },
    FeatureInfo { id: "icy", unit: "m", desc: "length x ice share x shade: ice share 0 at <=3% grade, 1 at >=7%; shade = shade(t) by day, 1 at night (shaded hills stay icy)" },
    FeatureInfo { id: "flood", unit: "m", desc: "length x share of the edge in a FEMA flood zone (unknown 0)" },
    FeatureInfo { id: "crossing", unit: "n", desc: "weighted crossings: each unsignalled crossing 1, each signalled crossing .3" },
    FeatureInfo { id: "no_sidewalk", unit: "m", desc: "length of road tagged without a sidewalk (unknown 0)" },
    FeatureInfo { id: "kerb", unit: "n", desc: "number of crossings with a raised kerb (wheelchair barrier)" },
    FeatureInfo { id: "dull", unit: "m", desc: "length x (1 - max(green share, water-view share)): not near greenery or water (unknown 0)" },
    FeatureInfo { id: "stop", unit: "n", desc: "number of stop signs, give-ways and signals on the edge (a stop for bikes; weight = seconds per stop)" },
];

pub const N_FEATURES: usize = 20;
const _: () = assert!(FEATURES.len() == N_FEATURES);

pub fn feature_index(id: &str) -> Option<usize> {
    FEATURES.iter().position(|f| f.id == id)
}

/// Time of day in local minutes since midnight. Wraps, so 1500 = 01:00.
pub fn is_night(meta: &Meta, t_min: f32) -> bool {
    let t = t_min.rem_euclid(1440.0);
    t < meta.sunrise_min || t >= meta.sunset_min
}

/// Feature vector of `edge` traversed forwards (from -> to) or backwards, at time `t_min`.
/// Every entry is >= 0 for any valid edge (lengths, climbs and fractions are all >= 0).
pub fn edge_features(edge: &Edge, forward: bool, mode: Mode, meta: &Meta, t_min: f32) -> [f32; N_FEATURES] {
    let len = edge.length_m;
    let climb = if forward { edge.climb_m } else { edge.descent_m };
    let night = is_night(meta, t_min);

    let roughness = match edge.surface {
        Some(Surface::Paved) => 0.0,
        Some(Surface::Compacted) => 0.33,
        Some(Surface::Unpaved) => 0.67,
        Some(Surface::Rough) => 1.0,
        None => 0.2,
    };
    let unlit = match edge.lit {
        Some(true) => 0.0,
        Some(false) => 1.0,
        None => 0.5,
    };
    let shade = match &edge.shade {
        Some(slots) if !slots.is_empty() => {
            let i = (t_min.rem_euclid(1440.0) / meta.slot_min as f32) as usize % slots.len();
            slots[i]
        }
        _ if edge.covered => 1.0,
        _ => edge.tree.unwrap_or(0.0),
    }
    .clamp(0.0, 1.0);
    // Shaded hills stay icy: by day the sun melts the sunny part; at night nothing does.
    let ice_shade = if night { 1.0 } else { shade };
    let ice = ((edge.max_grade_pct - 3.0) / 4.0).clamp(0.0, 1.0);
    let signalled = edge.crossings.saturating_sub(edge.crossings_unsignalled.min(edge.crossings));
    let no_sidewalk = if edge.sidewalk == Some(false) { len } else { 0.0 };
    let dull = match (edge.green, edge.water_view) {
        (None, None) => 0.0,
        (g, w) => 1.0 - g.unwrap_or(0.0).max(w.unwrap_or(0.0)).clamp(0.0, 1.0),
    };
    let lts = match edge.lts {
        Some(l) => (l.clamp(1, 4) as f32 - 1.0) / 3.0,
        None => 0.5,
    };
    let gap = |d: f32| d.clamp(0.0, NEAR_CAP_M) / NEAR_CAP_M;

    [
        len / mode.speed_mps(),
        climb,
        len * ((edge.max_grade_pct - 5.0) / 5.0).max(0.0),
        if edge.steps { len } else { 0.0 },
        len * roughness,
        len * (edge.busy.min(3) as f32 / 3.0),
        if night { len * unlit } else { 0.0 },
        if night { 0.0 } else { len * (1.0 - shade) },
        if edge.covered { 0.0 } else { len },
        len * lts,
        len * gap(edge.near_bench_m),
        len * gap(edge.near_toilets_m),
        len * gap(edge.near_water_m),
        len * ice * ice_shade,
        len * edge.flood.unwrap_or(0.0).clamp(0.0, 1.0),
        edge.crossings_unsignalled.min(edge.crossings) as f32 + 0.3 * signalled as f32,
        no_sidewalk,
        edge.kerb_raised as f32,
        len * dull,
        edge.stops as f32,
    ]
}
