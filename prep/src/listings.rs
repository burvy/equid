//! Homes and apartments for rent or sale inside the region, from the CoStar data kit (task 033).
//! LOCAL ONLY: the kit's terms are unknown and the repo is public, so data/<region>/listings.json is gitignored and
//! nothing from the kit goes into stats.txt. The kit is read as data only (its tools/*.py are never run).
//!
//! Sources (kit/models/*.md explain every field):
//! - commercial-property.jsonl: apartment communities (propertyType 11): units, style, class/rating, pets, parking
//! - apartment-listing.jsonl: the community's live Apartments.com listing: rents, units available, floor plans
//! - residential-listings.jsonl: MLS listings of homes for sale or rent (one entry per home: its current listing,
//!   else its latest one)
//! - commercial-properties.csv / residential-listings.csv: Matterport 3D model ids
//!
//! The kit has amenity ids but not their names, so `amenities` is built from the structured yes/no fields.

use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader};
use std::path::Path;

pub struct Built {
    pub json: String,
    pub stats: String,
}

/// `snap(lon, lat)` -> (nearest graph node, distance in m).
pub fn build(kit: &Path, bbox: [f64; 4], snap: &dyn Fn(f64, f64) -> (u32, f64)) -> Result<Built, String> {
    let inside = |a: &Value| -> Option<(f64, f64)> {
        let (lon, lat) = (a["longitude"].as_f64()?, a["latitude"].as_f64()?);
        (lon >= bbox[0] && lon <= bbox[2] && lat >= bbox[1] && lat <= bbox[3]).then_some((lon, lat))
    };
    let mp_commercial = matterport_csv(&kit.join("commercial-properties.csv"), 1)?;
    let mp_homes = matterport_csv(&kit.join("residential-listings.csv"), 2)?;

    let mut out: BTreeMap<String, Value> = BTreeMap::new();

    // ---- apartment communities
    let mut apt_listing: HashMap<i64, Value> = HashMap::new();
    each(&kit.join("data/apartment-listing.jsonl"), &mut |r| {
        if inside(&r["address"]).is_some() {
            if let Some(pid) = r["propertyId"].as_i64() {
                apt_listing.insert(pid, r);
            }
        }
    })?;
    let mut props: Vec<Value> = Vec::new();
    each(&kit.join("data/commercial-property.jsonl"), &mut |p| {
        if p["propertyType"].as_i64() == Some(11) && inside(&p["primaryAddress"]).is_some() {
            props.push(p);
        }
    })?;
    for p in props {
        // standing multifamily only (1 Existing, 5 UnderRenovation): offices, shops and proposed buildings are not homes
        if p["propertyType"].as_i64() != Some(11) || !matches!(p["constructionStatus"].as_i64(), Some(1) | Some(5)) {
            continue;
        }
        let Some((lon, lat)) = inside(&p["primaryAddress"]) else { continue };
        let id = p["id"].as_i64().unwrap_or(0);
        let a = &p["propertyApartment"];
        let l = apt_listing.get(&id);
        let lv = |k: &str| l.map(|l| l[k].clone()).unwrap_or(Value::Null);
        let pos = |v: Value| if v.as_f64().is_some_and(|x| x > 0.0) { v } else { Value::Null };
        // floor plans: the listing's (they carry rents) else the property's unit mix
        let plans: Vec<&Value> = match l {
            Some(l) => arr(&l["apartmentModels"]).collect(),
            None => arr(&a["models"]).collect(),
        };
        let beds: Vec<i64> = plans.iter().filter_map(|m| m["numBeds"].as_i64()).collect();
        let baths: Vec<f64> = plans.iter().filter_map(|m| m["numBaths"].as_f64()).collect();
        let sq_min = plans.iter().filter_map(|m| m["minSquareFeet"].as_i64()).filter(|&x| x > 0).min();
        let sq_max = plans.iter().filter_map(|m| m["maxSquareFeet"].as_i64()).max();
        let year = l.and_then(|l| l["yearBuilt"].as_i64()).or_else(|| {
            arr(&p["constructionEvents"]).filter(|e| e["eventType"].as_i64() == Some(2)).filter_map(|e| e["year"].as_i64()).max()
        });
        let addr = &p["primaryAddress"];
        let street = [addr["streetNumber"].as_str(), addr["streetDirection"].as_str(), addr["streetName"].as_str(), addr["streetType"].as_str()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let name = p["buildingName"].as_str().or(l.and_then(|l| l["buildingName"].as_str())).filter(|s| !s.is_empty()).unwrap_or(&street);
        let (node, snap_m) = snap(lon, lat);
        let mut m = Map::new();
        m.insert("id".into(), json!(format!("apt:{id}")));
        m.insert("kind".into(), json!("apartment"));
        m.insert("status".into(), json!(if l.is_some() { "active" } else { "no_listing" }));
        m.insert("name".into(), json!(name));
        m.insert("address".into(), json!(street));
        m.insert("lon".into(), json!(r6(lon)));
        m.insert("lat".into(), json!(r6(lat)));
        m.insert("node".into(), json!(node));
        m.insert("snap_m".into(), json!(r1(snap_m)));
        // rent from the floor plans that state one: the community minRent is 0 when any plan has no rent
        // (models/apartment-listing.md). Per-unit pricing only; student per-bed rents are not comparable.
        let rents: Vec<i64> = plans
            .iter()
            .filter(|m| m["rentPricingType"].as_i64().unwrap_or(1) == 1)
            .flat_map(|m| [m["minRent"].as_i64(), m["maxRent"].as_i64()])
            .flatten()
            .filter(|&x| x > 0)
            .collect();
        m.insert("rent_min".into(), json!(rents.iter().min()));
        m.insert("rent_max".into(), json!(rents.iter().max()));
        m.insert("price".into(), Value::Null);
        m.insert("beds_min".into(), json!(beds.iter().min()));
        m.insert("beds_max".into(), json!(beds.iter().max()));
        m.insert("baths_min".into(), json!(baths.iter().cloned().reduce(f64::min)));
        m.insert("baths_max".into(), json!(baths.iter().cloned().reduce(f64::max)));
        m.insert("sqft_min".into(), json!(sq_min));
        m.insert("sqft_max".into(), json!(sq_max));
        m.insert("year_built".into(), json!(year));
        m.insert("units".into(), pos(a["totalNumUnits"].clone()));
        m.insert("units_available".into(), lv("numUnitsAvailable"));
        m.insert("class".into(), json!(enum_name(&p["buildingClass"], &["", "A", "B", "C", "F"])));
        m.insert("rating".into(), pos(p["buildingRating"].clone()));
        m.insert("manager".into(), lv("propertyManagementCompanyName"));
        m.insert("amenities".into(), json!(apartment_amenities(&p, l)));
        let photo = l.and_then(|l| l["primaryPhoto"]["attachmentUri"].as_str()).or(p["primaryPhoto"]["attachmentUri"].as_str());
        m.insert("photo".into(), json!(photo));
        m.insert("matterport".into(), json!(mp_commercial.get(&id).cloned().unwrap_or_default()));
        m.insert("source".into(), json!(if l.is_some() { "costar commercial-property + apartment-listing" } else { "costar commercial-property" }));
        out.insert(format!("apt:{id}"), Value::Object(m));
    }

    // ---- homes (MLS): group the listings of one home (propertyId), keep its current listing, else its latest
    let mut homes: BTreeMap<i64, Vec<Value>> = BTreeMap::new();
    each(&kit.join("data/residential-listings.jsonl"), &mut |r| {
        if inside(&r["property"]["address"]).is_some() {
            homes.entry(r["propertyId"].as_i64().unwrap_or(0)).or_default().push(r);
        }
    })?;
    for (_, mut ls) in homes {
        // current (Active 1 / Contract 2) first, then latest listDate, then the primary MLS listing, then id
        ls.sort_by_key(|r| {
            let current = matches!(r["listingStatus"].as_i64(), Some(1) | Some(2));
            (!current, std::cmp::Reverse(r["listDate"].as_str().unwrap_or("").to_string()), !r["isPrimaryForThisListingInstance"].as_bool().unwrap_or(false), r["id"].as_i64())
        });
        let r = &ls[0];
        let prop = &r["property"];
        let addr = &prop["address"];
        let (lon, lat) = inside(addr).unwrap();
        let id = r["id"].as_i64().unwrap_or(0);
        let rent = r["transactionType"].as_i64() == Some(2);
        let sold = r["listingStatus"].as_i64() == Some(4);
        let status = match r["listingStatus"].as_i64() {
            Some(1) => "active",
            Some(2) => "contract",
            Some(4) if rent => "rented",
            Some(4) => "sold",
            _ => "inactive",
        };
        let mut street = addr["fullStreetAddress"].as_str().unwrap_or("").to_string();
        if let Some(u) = addr["unitNumber"].as_str().filter(|u| !u.is_empty()) {
            street = format!("{street} #{u}");
        }
        let name = if street.is_empty() { "Address hidden".to_string() } else { street.clone() };
        let mut mp: BTreeSet<String> = BTreeSet::new();
        for l in &ls {
            if let Some(v) = mp_homes.get(&l["id"].as_i64().unwrap_or(0)) {
                mp.extend(v.iter().cloned());
            }
        }
        // first photo: an image media item's uri, else a tour's thumbnail
        let photo = arr(&r["media"])
            .find_map(|m| m["uri"].as_str().filter(|u| u.contains("/image.")))
            .or_else(|| arr(&r["media"]).find_map(|m| m["thumbnailUri"].as_str()));
        let (node, snap_m) = snap(lon, lat);
        let price = r["currentPrice"].clone();
        let mut m = Map::new();
        m.insert("id".into(), json!(format!("home:{id}")));
        m.insert("kind".into(), json!(if rent { "home_rent" } else { "home_sale" }));
        m.insert("status".into(), json!(status));
        m.insert("name".into(), json!(name));
        m.insert("address".into(), json!(street));
        m.insert("lon".into(), json!(r6(lon)));
        m.insert("lat".into(), json!(r6(lat)));
        m.insert("node".into(), json!(node));
        m.insert("snap_m".into(), json!(r1(snap_m)));
        // a rental's rent: rentMin/rentMax, else its asking price (which is the monthly rent)
        let rent_or_price = |k: &str| if !rent { Value::Null } else if r[k].is_number() { r[k].clone() } else { price.clone() };
        m.insert("rent_min".into(), rent_or_price("rentMin"));
        m.insert("rent_max".into(), rent_or_price("rentMax"));
        // Texas does not disclose sale prices: a sold home's price is null (models/residential-listings.md)
        m.insert("price".into(), if rent { Value::Null } else { price });
        m.insert("beds_min".into(), prop["bedsMin"].clone());
        m.insert("beds_max".into(), prop["bedsMax"].clone());
        m.insert("baths_min".into(), prop["bathsMin"].clone());
        m.insert("baths_max".into(), prop["bathsMax"].clone());
        m.insert("sqft_min".into(), prop["sqftMin"].clone());
        m.insert("sqft_max".into(), prop["sqftMax"].clone());
        m.insert("year_built".into(), prop["yearBuilt"].clone());
        m.insert("home_type".into(), json!(home_type(&prop["propertyType"])));
        m.insert("hoa_month".into(), r["resoValues"]["hoa"]["totalFeesPerMonth"].clone());
        m.insert("days_on_market".into(), if sold { Value::Null } else { r["dom"].clone() });
        m.insert("amenities".into(), json!(home_amenities(&r["resoValues"])));
        m.insert("photo".into(), json!(photo));
        m.insert("matterport".into(), json!(mp.into_iter().collect::<Vec<_>>()));
        m.insert("source".into(), json!("costar residential-listings"));
        out.insert(format!("home:{id}"), Value::Object(m));
    }

    let items: Vec<Value> = out.into_values().collect();
    let stats = stats(&items);
    let doc = json!({
        "version": 1,
        "note": "LOCAL ONLY, never commit: CoStar data kit (terms unknown). Built by prep (prep/src/listings.rs).",
        "listings": items,
    });
    Ok(Built { json: serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?, stats })
}

fn apartment_amenities(p: &Value, l: Option<&Value>) -> Vec<String> {
    let a = &p["propertyApartment"];
    let rp = &a["residentialProfile"];
    let mut v: Vec<String> = Vec::new();
    if let Some(s) = enum_name(&a["apartmentStyleType"], &["", "garden", "mid-rise", "high-rise", "low-rise", "single-family", "townhome"]) {
        v.push(s.into());
    }
    match l.and_then(|l| l["lifestyleType"].as_i64()).or(a["apartmentLifestyleType"].as_i64()) {
        Some(2) => v.push("senior".into()),
        Some(3) => v.push("student".into()),
        Some(5) => v.push("corporate".into()),
        _ => {}
    }
    if matches!(a["apartmentRentType"].as_i64(), Some(2) | Some(3)) {
        v.push("affordable units".into());
    }
    if arr(&a["elevators"]).next().is_some() {
        v.push("elevator".into());
    }
    if rp["hasNoPetsAllowed"].as_bool() == Some(true) {
        v.push("no pets".into());
    } else {
        let pets: BTreeSet<i64> = arr(&rp["petPolicies"]).filter_map(|x| x["petType"].as_i64()).collect();
        if pets.contains(&3) {
            v.push("dogs ok".into());
        }
        if pets.contains(&2) {
            v.push("cats ok".into());
        }
    }
    let parking: BTreeSet<i64> = arr(&rp["parkingPolicies"]).filter_map(|x| x["parkingType"].as_i64()).collect();
    if parking.iter().any(|t| matches!(t, 5 | 11 | 12)) {
        v.push("garage parking".into());
    } else if parking.iter().any(|t| matches!(t, 3 | 8)) {
        v.push("covered parking".into());
    } else if parking.contains(&1) {
        v.push("surface parking".into());
    } else if parking.contains(&4) || rp["hasNoParkingAvailable"].as_bool() == Some(true) {
        v.push("street parking only".into());
    }
    if parking.contains(&10) {
        v.push("EV charging".into());
    }
    if rp["hasVisitorParkingAvailable"].as_bool() == Some(true) {
        v.push("visitor parking".into());
    }
    if arr(&a["models"]).any(|m| m["isShortTermLeaseAvailable"].as_bool() == Some(true)) {
        v.push("short-term lease".into());
    }
    v
}

fn home_amenities(r: &Value) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let flag = |k: &str| r[k].as_bool() == Some(true);
    let bits = |k: &str| r[k].as_i64().unwrap_or(0);
    for (k, name) in [
        ("hasGarage", "garage"),
        ("isPetFriendly", "pets ok"),
        ("hasWasherDryer", "washer/dryer"),
        ("hasDishwasher", "dishwasher"),
        ("hasGym", "gym"),
        ("hasElevator", "elevator"),
        ("hasSpa", "spa"),
        ("hasFireplace", "fireplace"),
        ("hasHardwoodFloors", "hardwood floors"),
        ("hasStorage", "storage"),
        ("hasAccessibilityFeatures", "accessible"),
        ("isWaterfront", "waterfront"),
        ("isNearPublicTransportation", "near transit"),
    ] {
        if flag(k) {
            v.push(name.into());
        }
    }
    let pool = bits("poolType");
    if pool & 1 != 0 {
        v.push("private pool".into());
    } else if pool & 6 != 0 {
        v.push("community pool".into());
    }
    let out = bits("outdoorSpaceType");
    for (b, name) in [(1, "balcony"), (2, "deck"), (4, "porch/patio"), (8, "fenced yard"), (32, "terrace")] {
        if out & b != 0 {
            v.push(name.into());
        }
    }
    if bits("securityFeatureType") & 4 != 0 {
        v.push("gated".into());
    }
    if r["structure"]["hasNewConstruction"].as_bool() == Some(true) {
        v.push("new construction".into());
    }
    v
}

fn home_type(t: &Value) -> Option<&'static str> {
    Some(match t.as_i64()? {
        1 => "house",
        2 => "townhouse",
        4 => "condo",
        256 => "co-op",
        512 | 1024 | 2048 | 16 => "multi-family",
        _ => "other",
    })
}

fn stats(items: &[Value]) -> String {
    let mut by: BTreeMap<(String, String), usize> = BTreeMap::new();
    for i in items {
        *by.entry((i["kind"].as_str().unwrap_or("").into(), i["status"].as_str().unwrap_or("").into())).or_default() += 1;
    }
    let mut s = format!("listings {} in box: {}\n", items.len(), by.iter().map(|((k, st), n)| format!("{k}/{st} {n}")).collect::<Vec<_>>().join(", "));
    for f in ["rent_min", "price", "beds_min", "baths_min", "sqft_min", "year_built", "units_available", "rating", "photo"] {
        let n = items.iter().filter(|i| !i[f].is_null()).count();
        s += &format!("  {f:16} {n:3} of {}\n", items.len());
    }
    let am = items.iter().filter(|i| i["amenities"].as_array().is_some_and(|a| !a.is_empty())).count();
    let mp = items.iter().filter(|i| i["matterport"].as_array().is_some_and(|a| !a.is_empty())).count();
    let far = items.iter().filter(|i| i["snap_m"].as_f64().unwrap_or(0.0) > 100.0).count();
    s += &format!("  amenities (>=1)  {am:3} of {}\n  matterport (>=1) {mp:3} of {}\n  snapped > 100 m  {far:3}\n", items.len(), items.len());
    s
}

/// id -> Matterport model ids, from a starter CSV. The id is column 0 and the model id column `col`; both come before
/// the address, so a plain comma split is safe for them.
fn matterport_csv(path: &Path, col: usize) -> Result<HashMap<i64, Vec<String>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut m: HashMap<i64, Vec<String>> = HashMap::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.splitn(col + 2, ',').collect();
        let (Some(id), Some(model)) = (f.first().and_then(|s| s.parse().ok()), f.get(col).filter(|s| !s.is_empty())) else { continue };
        let v = m.entry(id).or_default();
        if !v.iter().any(|x| x == model) {
            v.push(model.to_string());
        }
    }
    for v in m.values_mut() {
        v.sort();
    }
    Ok(m)
}

/// Calls `f` on each record of a JSON Lines file, one at a time (commercial-property.jsonl is ~190 MB).
fn each(path: &Path, f: &mut dyn FnMut(Value)) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if !line.trim().is_empty() {
            f(serde_json::from_str(&line).map_err(|e| format!("{}: {e}", path.display()))?);
        }
    }
    Ok(())
}

fn arr(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

fn enum_name(v: &Value, names: &[&'static str]) -> Option<&'static str> {
    names.get(v.as_i64()? as usize).copied().filter(|s| !s.is_empty())
}

fn r6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}
