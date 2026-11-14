//! Is an edge lit at night? OSM `lit` tags cover ~1% of Princeton, and no street lamps are mapped,
//! so we fill the gap with a few plain rules. The first rule that applies wins; `lit_src` records which.
//!
//!  tag    OSM `lit` on the way (authoritative)
//!  lamp   a mapped street lamp within LAMP_M (none in OSM today; kept so added lamps count)
//!  road   a tertiary-or-bigger road, or a footway/cycleway beside one: US town arterials have streetlights
//!  campus footway / pedestrian / steps on Princeton University or Seminary grounds (lamp-posted walks)
//!  trail  path / track / bridleway away from roads that isn't paved: woods and towpath, unlit
//!  (none) everything else stays unknown (residential streets: some lit, some not)

use crate::geo::{dist_to_segment, Proj};
use engine::graph::Surface;
use serde_json::Value;

pub const LAMP_M: f64 = 25.0;

pub struct Night {
    lamps: Vec<(f64, f64)>,
    /// Ring segments of campus polygons, in metres. Point-in-polygon by even-odd ray casting over
    /// all segments works without assembling the multipolygon rings.
    campus: Vec<((f64, f64), (f64, f64))>,
    pub n_lamps: usize,
    pub campus_names: Vec<String>,
}

impl Night {
    /// From data/raw/night.json (prep/night.ql): street lamps + university polygons.
    pub fn load(v: &Value, proj: &Proj) -> Night {
        let mut lamps = Vec::new();
        let mut campus = Vec::new();
        let mut campus_names = Vec::new();
        for e in v["elements"].as_array().into_iter().flatten() {
            let t = &e["tags"];
            if t["highway"] == "street_lamp" {
                if let (Some(lon), Some(lat)) = (e["lon"].as_f64(), e["lat"].as_f64()) {
                    lamps.push(proj.xy(lon, lat));
                }
                continue;
            }
            // Only Princeton University / Seminary grounds: the Institute for Advanced Study
            // polygon is mostly the Institute Woods, which are not lit.
            let name = t["name"].as_str().unwrap_or("");
            if !(t["amenity"] == "university" && name.starts_with("Princeton")) {
                continue;
            }
            let rings: Vec<&Vec<Value>> = match e["type"].as_str() {
                Some("way") => e["geometry"].as_array().into_iter().collect(),
                Some("relation") => e["members"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|m| m["geometry"].as_array())
                    .collect(),
                _ => vec![],
            };
            for ring in rings {
                let pts: Vec<(f64, f64)> =
                    ring.iter().filter_map(|p| Some(proj.xy(p["lon"].as_f64()?, p["lat"].as_f64()?))).collect();
                campus.extend(pts.windows(2).map(|w| (w[0], w[1])));
            }
            campus_names.push(name.to_string());
        }
        Night { n_lamps: lamps.len(), lamps, campus, campus_names }
    }

    fn on_campus(&self, p: (f64, f64)) -> bool {
        let mut inside = false;
        for &(a, b) in &self.campus {
            if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                inside = !inside;
            }
        }
        inside
    }

    /// (lit, source) for an edge. `xy` = edge polyline in metres, `mid` = its midpoint,
    /// `busy` = the edge's adjacent traffic class (0..3, from main.rs).
    pub fn lit(&self, tag: Option<bool>, hw: &str, surface: Option<Surface>, busy: u8, xy: &[(f64, f64)], mid: (f64, f64)) -> (Option<bool>, Option<&'static str>) {
        if tag.is_some() {
            return (tag, Some("tag"));
        }
        let near_lamp = self
            .lamps
            .iter()
            .any(|&l| xy.windows(2).any(|s| dist_to_segment(l, s[0], s[1]) <= LAMP_M));
        if near_lamp {
            return (Some(true), Some("lamp"));
        }
        if busy >= 2 {
            return (Some(true), Some("road"));
        }
        if matches!(hw, "footway" | "pedestrian" | "steps") && self.on_campus(mid) {
            return (Some(true), Some("campus"));
        }
        if matches!(hw, "path" | "track" | "bridleway") && busy == 0 && surface != Some(Surface::Paved) {
            return (Some(false), Some("trail"));
        }
        (None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn night() -> Night {
        let proj = Proj::new(0.0, 0.0);
        let sq = json!([{"lon":0.0,"lat":0.0},{"lon":0.01,"lat":0.0},{"lon":0.01,"lat":0.01},{"lon":0.0,"lat":0.01},{"lon":0.0,"lat":0.0}]);
        let v = json!({"elements":[
            {"type":"way","tags":{"amenity":"university","name":"Princeton University"},"geometry":sq},
            {"type":"way","tags":{"amenity":"university","name":"Institute for Advanced Study"},"geometry":[]},
            {"type":"node","lon":0.02,"lat":0.0,"tags":{"highway":"street_lamp"}}
        ]});
        Night::load(&v, &proj)
    }

    #[test]
    fn rules_in_order() {
        let n = night();
        assert_eq!(n.campus_names, vec!["Princeton University"]);
        let inside = [(500.0, 500.0), (510.0, 500.0)];
        let outside = [(5000.0, 5000.0), (5010.0, 5000.0)];
        let by_lamp = [(2226.0, 10.0), (2240.0, 10.0)]; // ~10 m from the lamp at lon 0.02
        // tag wins over everything
        assert_eq!(n.lit(Some(false), "footway", None, 3, &inside, inside[0]), (Some(false), Some("tag")));
        assert_eq!(n.lit(None, "residential", None, 1, &by_lamp, by_lamp[0]), (Some(true), Some("lamp")));
        assert_eq!(n.lit(None, "footway", None, 2, &outside, outside[0]), (Some(true), Some("road")));
        assert_eq!(n.lit(None, "footway", None, 0, &inside, inside[0]), (Some(true), Some("campus")));
        assert_eq!(n.lit(None, "footway", None, 0, &outside, outside[0]), (None, None));
        assert_eq!(n.lit(None, "path", Some(Surface::Unpaved), 0, &outside, outside[0]), (Some(false), Some("trail")));
        assert_eq!(n.lit(None, "path", Some(Surface::Paved), 0, &outside, outside[0]), (None, None));
        assert_eq!(n.lit(None, "residential", Some(Surface::Paved), 1, &outside, outside[0]), (None, None));
    }
}
