//! OSM tags -> edge features. Pure functions, one rule per line, so each choice is easy to explain.
//! Tag meanings: https://wiki.openstreetmap.org/wiki/Key:highway (and Key:surface, Key:lit, ...).

use engine::graph::{Cycleway, Surface};
use serde_json::{Map, Value};

pub type Tags = Map<String, Value>;

pub fn get<'a>(t: &'a Tags, k: &str) -> Option<&'a str> {
    t.get(k).and_then(|v| v.as_str())
}

fn is_yes(v: Option<&str>) -> bool {
    matches!(v, Some("yes" | "designated" | "permissive" | "official" | "destination"))
}
fn is_no(v: Option<&str>) -> bool {
    matches!(v, Some("no" | "private" | "use_sidepath"))
}

/// highway values we turn into graph edges (walkable or rideable). Everything else is ignored.
pub fn is_path_highway(hw: &str) -> bool {
    matches!(
        hw,
        "footway" | "path" | "pedestrian" | "steps" | "cycleway" | "corridor" | "bridleway" | "track"
            | "living_street" | "residential" | "service" | "unclassified" | "road"
            | "tertiary" | "tertiary_link" | "secondary" | "secondary_link"
            | "primary" | "primary_link" | "trunk" | "trunk_link"
    )
}

/// Ways without motor traffic.
pub fn is_car_free(hw: &str) -> bool {
    matches!(hw, "footway" | "path" | "pedestrian" | "steps" | "cycleway" | "corridor" | "bridleway" | "track")
}

/// Traffic class of a road carrying cars: 1 local, 2 collector, 3 arterial; 0 = no cars.
pub fn road_class(hw: &str) -> u8 {
    match hw {
        "living_street" | "residential" | "service" | "unclassified" | "road" => 1,
        "tertiary" | "tertiary_link" | "secondary" | "secondary_link" => 2,
        "primary" | "primary_link" | "trunk" | "trunk_link" | "motorway" | "motorway_link" => 3,
        _ => 0,
    }
}

/// (foot allowed, bike allowed). Defaults by highway type, then access / foot / bicycle tags override.
pub fn access(t: &Tags) -> (bool, bool) {
    let hw = get(t, "highway").unwrap_or("");
    let sidewalk = matches!(get(t, "sidewalk"), Some("both" | "left" | "right" | "yes"));
    // US trunk roads (Route 1) are walkable only where a sidewalk is mapped.
    let mut foot = !matches!(hw, "trunk" | "trunk_link") || sidewalk;
    let mut bike = !matches!(hw, "footway" | "pedestrian" | "steps" | "corridor" | "bridleway");
    if is_no(get(t, "access")) {
        foot = false;
        bike = false;
    }
    let (f, b) = (get(t, "foot"), get(t, "bicycle"));
    if is_yes(f) {
        foot = true;
    } else if is_no(f) {
        foot = false;
    }
    if is_yes(b) {
        bike = true;
    } else if is_no(b) {
        bike = false;
    }
    if hw == "steps" {
        bike = false; // even if bicycle=yes (that means "you may carry it")
    }
    (foot, bike)
}

/// 0 both ways, 1 only along the way's direction, -1 only against it.
pub fn oneway_bike(t: &Tags) -> i8 {
    if get(t, "oneway:bicycle") == Some("no") {
        return 0;
    }
    let contraflow = ["cycleway", "cycleway:left", "cycleway:right", "cycleway:both"]
        .iter()
        .any(|k| get(t, k).is_some_and(|v| v.starts_with("opposite")));
    if contraflow {
        return 0;
    }
    match get(t, "oneway") {
        Some("yes" | "true" | "1") => 1,
        Some("-1" | "reverse") => -1,
        _ if get(t, "junction") == Some("roundabout") => 1,
        _ => 0,
    }
}

/// Surface class by how bumpy it is for wheels (strollers, wheelchairs, bikes).
pub fn surface_class(raw: &str) -> Option<Surface> {
    let s = raw.split(':').next().unwrap_or(raw); // "concrete:plates" -> "concrete"
    Some(match s {
        "asphalt" | "concrete" | "paved" | "paving_stones" | "bricks" | "brick" | "metal" | "wood"
        | "rubber" | "tartan" | "chipseal" => Surface::Paved,
        "compacted" | "fine_gravel" => Surface::Compacted,
        "gravel" | "dirt" | "ground" | "earth" | "unpaved" | "pebblestone" | "woodchips"
        | "grass_paver" => Surface::Unpaved,
        "sett" | "cobblestone" | "unhewn_cobblestone" | "grass" | "sand" | "mud" | "rock" | "stone" => {
            Surface::Rough
        }
        _ => return None,
    })
}

/// Surface tag if present; untagged car roads are assumed paved (true for every street in the area).
pub fn surface(t: &Tags) -> (Option<Surface>, Option<String>) {
    let raw = get(t, "surface").map(str::to_string);
    let class = match &raw {
        Some(r) => surface_class(r),
        None if road_class(get(t, "highway").unwrap_or("")) > 0 => Some(Surface::Paved),
        None => None,
    };
    (class, raw)
}

pub fn lit(t: &Tags) -> Option<bool> {
    match get(t, "lit")? {
        "no" | "disused" => Some(false),
        "" => None,
        _ => Some(true), // yes, 24/7, automatic, sunset-sunrise, limited, interval
    }
}

/// Roof over the way: covered walkway, arcade, tunnel, building passage, indoor corridor.
pub fn covered(t: &Tags) -> bool {
    matches!(get(t, "covered"), Some("yes" | "arcade" | "colonnade" | "roof"))
        || matches!(get(t, "tunnel"), Some("yes" | "building_passage"))
        || get(t, "indoor") == Some("yes")
        || get(t, "highway") == Some("corridor")
}

/// Bridges and tunnels don't follow the terrain: their elevation is interpolated between the ends.
pub fn off_terrain(t: &Tags) -> bool {
    get(t, "bridge").is_some_and(|v| v != "no") || get(t, "tunnel").is_some_and(|v| v != "no")
}

pub fn cycleway(t: &Tags, bike: bool) -> Option<Cycleway> {
    if !bike {
        return None;
    }
    let hw = get(t, "highway").unwrap_or("");
    if is_car_free(hw) {
        return Some(Cycleway::Track); // separated from cars
    }
    let vals: Vec<&str> = ["cycleway", "cycleway:both", "cycleway:right", "cycleway:left"]
        .iter()
        .filter_map(|k| get(t, k))
        .collect();
    let has = |names: &[&str]| vals.iter().any(|v| names.contains(v));
    Some(if has(&["track"]) {
        Cycleway::Track
    } else if has(&["lane", "buffered_lane", "opposite_lane"]) {
        Cycleway::Lane
    } else if has(&["shared_lane", "share_busway", "shoulder"]) {
        Cycleway::Shared
    } else {
        Cycleway::None
    })
}

/// "25 mph" -> 25; bare numbers in the US are mph by convention.
fn maxspeed_mph(t: &Tags) -> Option<f32> {
    let v = get(t, "maxspeed")?;
    let n: f32 = v.split_whitespace().next()?.parse().ok()?;
    Some(if v.contains("km") { n / 1.609 } else { n })
}

/// Simplified bike Level of Traffic Stress (Mekuria, Furth & Nixon 2012): 1 = a child could ride it,
/// 4 = only confident cyclists. From road class, bike infrastructure, speed limit and lane count.
pub fn lts(t: &Tags, bike: bool) -> Option<u8> {
    if !bike {
        return None;
    }
    let hw = get(t, "highway").unwrap_or("");
    let cw = cycleway(t, bike);
    if is_car_free(hw) || cw == Some(Cycleway::Track) {
        return Some(1);
    }
    let speed = maxspeed_mph(t).unwrap_or(25.0);
    let lanes: u32 = get(t, "lanes").and_then(|v| v.parse().ok()).unwrap_or(2);
    let lane = cw == Some(Cycleway::Lane);
    Some(match road_class(hw) {
        1 => {
            if speed <= 25.0 && hw != "unclassified" { 1 } else if speed <= 30.0 { 2 } else { 3 }
        }
        2 if hw.starts_with("tertiary") => {
            if lane || (speed <= 25.0 && lanes <= 2) { 2 } else { 3 }
        }
        2 => {
            if lane { if speed <= 35.0 { 2 } else { 3 } } else if lanes > 2 || speed > 35.0 { 4 } else { 3 }
        }
        _ => {
            if lane { 3 } else { 4 }
        }
    })
}

/// Place category from amenity / tourism / leisure, or None if we don't use it.
pub fn place_category(t: &Tags) -> Option<String> {
    let amen = ["bench", "toilets", "drinking_water", "cafe", "restaurant", "fast_food", "ice_cream",
        "library", "pharmacy", "bicycle_parking", "shelter"];
    let tour = ["viewpoint", "museum", "artwork", "attraction", "picnic_site"];
    let leis = ["park", "garden", "playground", "nature_reserve"];
    for (key, list) in [("amenity", &amen[..]), ("tourism", &tour[..]), ("leisure", &leis[..])] {
        if let Some(v) = get(t, key) {
            if list.contains(&v) {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Place tags worth passing to the web/LLM (opening hours, accessibility, bench backrest, ...).
pub const PLACE_TAGS: &[&str] = &[
    "opening_hours", "wheelchair", "cuisine", "backrest", "fee", "access", "website", "covered",
    "bottle", "unisex", "changing_table", "capacity", "brand", "description", "operator", "outdoor_seating",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tags(v: Value) -> Tags {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn access_defaults_and_overrides() {
        assert_eq!(access(&tags(json!({"highway":"footway"}))), (true, false));
        assert_eq!(access(&tags(json!({"highway":"footway","bicycle":"yes"}))), (true, true));
        assert_eq!(access(&tags(json!({"highway":"steps","bicycle":"yes"}))), (true, false));
        assert_eq!(access(&tags(json!({"highway":"service","access":"private"}))), (false, false));
        assert_eq!(access(&tags(json!({"highway":"trunk"}))), (false, true));
        assert_eq!(access(&tags(json!({"highway":"trunk","sidewalk":"right"}))), (true, true));
    }

    #[test]
    fn oneway_rules() {
        assert_eq!(oneway_bike(&tags(json!({"oneway":"yes"}))), 1);
        assert_eq!(oneway_bike(&tags(json!({"oneway":"yes","oneway:bicycle":"no"}))), 0);
        assert_eq!(oneway_bike(&tags(json!({"oneway":"-1"}))), -1);
        assert_eq!(oneway_bike(&tags(json!({"junction":"roundabout"}))), 1);
    }

    #[test]
    fn surface_and_lts() {
        assert_eq!(surface(&tags(json!({"highway":"residential"}))).0, Some(Surface::Paved));
        assert_eq!(surface(&tags(json!({"highway":"path"}))).0, None);
        assert_eq!(surface(&tags(json!({"highway":"path","surface":"sett"}))).0, Some(Surface::Rough));
        assert_eq!(lts(&tags(json!({"highway":"residential"})), true), Some(1));
        assert_eq!(lts(&tags(json!({"highway":"primary"})), true), Some(4));
        assert_eq!(lts(&tags(json!({"highway":"primary","cycleway:right":"lane"})), true), Some(3));
        assert_eq!(lts(&tags(json!({"highway":"footway"})), false), None);
    }
}
