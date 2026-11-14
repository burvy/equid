//! Point objects for the open-vocabulary `near:<label>` features (contracts §2b) from OSM (prep/objects.ql).
//! One plain-language label per OSM tag, first match wins. Bench, toilets and drinking water are left out on
//! purpose: the engine already has them as built-in features (`near:bench` = `bench_gap`, ...).
//! Objects from other sources already in the output file (Vexcel detect via build_objects) are kept.

use engine::objects::{LabelInfo, Object, Objects};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// (OSM key, value, label)
pub const LABELS: &[(&str, &str, &str)] = &[
    ("amenity", "waste_basket", "trash can"),
    ("amenity", "bicycle_parking", "bike rack"),
    ("amenity", "shelter", "shelter"),
    ("amenity", "fountain", "fountain"),
    ("amenity", "post_box", "mailbox"),
    ("leisure", "playground", "playground"),
    ("leisure", "dog_park", "dog park"),
    ("leisure", "picnic_table", "picnic table"),
    ("leisure", "swimming_pool", "swimming pool"),
    ("leisure", "fitness_station", "outdoor gym"),
    ("leisure", "pitch", "sports field"),
    ("highway", "crossing", "crosswalk"),
    ("footway", "crossing", "crosswalk"),
    ("highway", "street_lamp", "street lamp"),
    ("highway", "bus_stop", "bus stop"),
    ("highway", "traffic_signals", "traffic light"),
    ("natural", "tree", "tree"),
    ("tourism", "artwork", "public art"),
    ("tourism", "picnic_site", "picnic area"),
];

pub fn label(tags: &Value) -> Option<&'static str> {
    LABELS.iter().find(|(k, v, _)| tags[*k] == *v).map(|l| l.2)
}

fn r6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// OSM objects inside the box, sorted (label, lon, lat) so the file is deterministic, followed by the
/// non-OSM objects of `old` (the current objects.json) in their original order.
pub fn build(osm: &Value, inside: &dyn Fn((f64, f64)) -> bool, old: Option<&str>) -> Result<Objects, String> {
    let mut objects = Vec::new();
    for e in osm["elements"].as_array().ok_or("objects.json (raw): no elements")? {
        let Some(l) = label(&e["tags"]) else { continue };
        // nodes carry lon/lat, ways/relations a center (`out center`)
        let (lon, lat) = match (e["lon"].as_f64(), e["lat"].as_f64()) {
            (Some(lon), Some(lat)) => (lon, lat),
            _ => match (e["center"]["lon"].as_f64(), e["center"]["lat"].as_f64()) {
                (Some(lon), Some(lat)) => (lon, lat),
                _ => continue,
            },
        };
        if inside((lon, lat)) {
            objects.push(Object { label: l.into(), lon: r6(lon), lat: r6(lat), src: Some("osm".into()), conf: None });
        }
    }
    objects.sort_by(|a, b| (&a.label, a.lon, a.lat).partial_cmp(&(&b.label, b.lon, b.lat)).unwrap());
    objects.dedup_by(|a, b| a.label == b.label && a.lon == b.lon && a.lat == b.lat);
    if let Some(text) = old {
        let o: Objects = serde_json::from_str(text).map_err(|e| format!("existing objects.json: {e}"))?;
        // rounded like the OSM points (0.1 m): serde_json's float parsing isn't exact, so unrounded values drift by
        // one ulp on every re-run
        objects.extend(o.objects.into_iter().filter(|x| x.src.as_deref() != Some("osm")).map(|x| Object { lon: r6(x.lon), lat: r6(x.lat), ..x }));
    }
    Ok(Objects { version: 1, objects })
}

pub fn stats(o: &Objects, labels: &[LabelInfo]) -> String {
    let mut by_src: BTreeMap<&str, usize> = BTreeMap::new();
    for x in &o.objects {
        *by_src.entry(x.src.as_deref().unwrap_or("?")).or_default() += 1;
    }
    let mut s = String::new();
    let _ = writeln!(s, "objects {} ({}); per label: objects / edges within 100 m", o.objects.len(), by_src.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "));
    for l in labels {
        let _ = writeln!(s, "  {:14} {:5} / {}", l.label, l.objects, l.edges_near);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels_center_bbox_and_kept_vexcel() {
        let osm = json!({"elements":[
            {"type":"node","lon":1.0,"lat":1.0,"tags":{"highway":"crossing"}},
            {"type":"way","center":{"lon":1.5,"lat":1.5},"tags":{"footway":"crossing","highway":"footway"}},
            {"type":"node","lon":1.2,"lat":1.2,"tags":{"amenity":"bench"}},
            {"type":"node","lon":9.0,"lat":9.0,"tags":{"natural":"tree"}}
        ]});
        let old = r#"{"version":1,"objects":[{"label":"crosswalk","lon":0,"lat":0,"src":"osm"},
            {"label":"swimming pool","lon":1.1,"lat":1.1,"src":"vexcel","conf":0.9}]}"#;
        let inside = |p: (f64, f64)| p.0 < 5.0 && p.1 < 5.0;
        let o = build(&osm, &inside, Some(old)).unwrap();
        let got: Vec<(&str, f64, Option<&str>)> = o.objects.iter().map(|x| (x.label.as_str(), x.lon, x.src.as_deref())).collect();
        assert_eq!(got, vec![("crosswalk", 1.0, Some("osm")), ("crosswalk", 1.5, Some("osm")), ("swimming pool", 1.1, Some("vexcel"))]);
    }
}
