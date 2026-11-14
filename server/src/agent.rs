//! The trip agent: plain Claude tool use, no framework. The model can only (1) pick a weight vector
//! (`make_metric`), (2) look at what the engine computed (`field`, `explain`, `list_*`), and
//! (3) `finish` with stop ids + a pitch. The engine does all the arithmetic, so whatever the model
//! writes, costs come from `Core::field` over non-negative weights (the Lean-proved setting).
//! The pitch is checked: every number in it must appear in a tool result or in the user's request.

use crate::hours;
use crate::llm::Llm;
use crate::vexcel::Detector;
use engine::features::FEATURES;
use engine::metric::{Metric, MetricSpec};
use engine::objects::Object;
use engine::Core;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_TURNS: usize = 14;

#[derive(Debug, Deserialize)]
pub struct AgentRequest {
    pub text: String,
    /// `{lon, lat}` (snapped to the nearest node) or `{node}`.
    #[serde(default)]
    pub origin: Option<Origin>,
    /// Local minutes since midnight (default 12:00).
    #[serde(default)]
    pub t: Option<f32>,
}

#[derive(Debug, Deserialize)]
pub struct Origin {
    pub lon: Option<f64>,
    pub lat: Option<f64>,
    pub node: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct Stop {
    pub id: String,
    pub name: Option<String>,
    pub category: String,
    pub minutes: Option<f64>,
    pub why: String,
}

#[derive(Debug, Serialize)]
pub struct AgentResponse {
    pub metric: MetricSpec,
    pub stops: Vec<Stop>,
    pub pitch: String,
    pub llm: String,
    pub tools_called: Vec<String>,
    /// Objects found by live detects during this request (so the web can `engine.add_objects` and run the metric itself).
    pub detected: Vec<Object>,
    /// Listings the pitch recommends (task 034), with the Matterport tour link; empty for ordinary trips.
    pub listings: Vec<ListingPick>,
}

#[derive(Debug, Serialize)]
pub struct ListingPick {
    pub id: String,
    pub name: String,
    pub address: Option<String>,
    /// Score under the current metric (0-100, see engine::listings).
    pub score: Option<f64>,
    pub tour_url: Option<String>,
    pub why: String,
}

const SYSTEM: &str = "You are Equid's trip agent. Equid measures distance as personal cost: every street edge costs \
sum(weight_i x feature_i), and you choose the weights. Given the user's trip description:\n\
1. Call list_features, then make_metric with weights (seconds of penalty per metre of each feature; 'time' is 1) and \
'forbid' for features that make an edge impassable (e.g. steps for a stroller or wheelchair). Weights must be >= 0.\n\
2. Call list_places / field to see what is reachable and what it costs, and pick the stops that make the trip best \
(e.g. a bench or fountain along the way, plus the destination the user wants). Use explain on your top pick to learn why it costs what it does.\n\
3. Finish by calling finish with stops (place ids from the tools, each with a short 'why') and a pitch: 2-4 sentences, \
like a friendly realtor, about the top pick for THIS user.\n\
Open vocabulary: besides the built-in features, any object can matter ('near:<label>' weights: cost grows with distance to the nearest such object, so a positive weight pulls the route toward them). Call list_labels to see which objects the data has (e.g. playground, crosswalk, trash can); use make_metric weights like {\"near:playground\": 2}. If the trip needs an object that is not in the menu, call detect(label) to find it live in a small area around the user, then use it. Only use labels from list_labels or ones you detected successfully.
\nFactor cheat-sheet (features from list_features): icy (shady slopes stay icy: winter/ice/frost; try 3-5), flood (FEMA flood zones, 4-8), crossing (street crossings, unsignalled count most; seconds per crossing, 15-30; older walkers, kids), no_sidewalk (roads without sidewalks; strollers, wheelchairs, kids), kerb (raised kerbs at crossings; wheelchair: weight 30+ or forbid), dull (not near greenery or water: use for peaceful or scenic walks, 0.4-1, with busy). Data can be missing in an area: such a feature then costs nothing, so say so if the explain shows no effect.
Rules: use only facts the tools returned. Every number in your pitch (minutes, metres, counts) must appear in a tool \
result; do not compute new numbers or invent facts (no opening hours or reviews unless a tool returned them). \
Costs are in minutes of personal cost, not clock time. Never output weights or costs as the final answer; call finish.\n\
Opening hours: the context gives the day and time of the trip. Places with readable hours carry hours_today, open_at_departure \
and closes_at; trust those (e.g. 'open until 21:00 tonight') rather than re-reading opening_hours, and prefer places that are open.\n\
Homes: if the user wants somewhere to live (apartment, home, rent, buy), the trip is their daily life seen from each front door. Call make_metric for how THEY move (as above), then rank_listings: it scores every available listing by how cheaply (under their metric) its everyday destinations are reached, and returns rent/price, beds, amenities, the nearest places and a Matterport tour_url. Pitch the best one or two with finish(listings=[{id, why}]) quoting only numbers rank_listings returned, and put the tour_url of your top pick in the pitch as a link so they can walk through it. Optional categories narrow what the score counts (e.g. playground, park for a family; cafe, restaurant for nightlife).
Writing: the pitch and each why are plain sentences for a person. Name places by their name (or category and street if unnamed); \
never write place ids (like n4802855943 or w109782066), JSON, braces or quotes around the text.";

fn tools() -> Vec<Value> {
    vec![
        json!({"name": "list_features", "description": "The features a metric can weight: id, unit, meaning.",
               "input_schema": {"type": "object", "properties": {}}}),
        json!({"name": "list_labels", "description": "The object labels the data has (open vocabulary, e.g. playground, crosswalk): label, number of objects, number of edges within 100 m. bench, toilets and water are always available. Weight one as 'near:<label>' in make_metric.",
               "input_schema": {"type": "object", "properties": {}}}),
        json!({"name": "detect", "description": "Find a NEW kind of object live from aerial imagery in a small square around the user's origin (any plain-language label, e.g. 'dog park'). Adds it to the label menu so make_metric can weight 'near:<label>'. Slow (seconds): only for labels missing from list_labels.",
               "input_schema": {"type": "object", "properties": {"label": {"type": "string"}, "radius_m": {"type": "integer", "minimum": 50, "maximum": 300}}, "required": ["label"]}}),
        json!({"name": "list_places", "description": "Places in the area (id, name, category, rating, opening hours), optionally filtered by category and/or name (case-insensitive substring, e.g. 'conte' for a destination the user names). At most 60 per call. No costs: use field for costs.",
               "input_schema": {"type": "object", "properties": {"categories": {"type": "array", "items": {"type": "string"}}, "name": {"type": "string"}}}}),
        json!({"name": "make_metric", "description": "Set the cost metric for this trip. weights: feature id -> non-negative number. forbid: feature ids that make an edge impassable. mode: foot or bike. Replaces any earlier metric.",
               "input_schema": {"type": "object", "properties": {
                   "mode": {"type": "string", "enum": ["foot", "bike"]},
                   "weights": {"type": "object", "additionalProperties": {"type": "number", "minimum": 0}},
                   "forbid": {"type": "array", "items": {"type": "string"}}},
                 "required": ["weights"]}}),
        json!({"name": "field", "description": "Run the engine from the user's origin under the current metric. Returns the cheapest places (minutes of personal cost), optionally filtered by categories.",
               "input_schema": {"type": "object", "properties": {
                   "categories": {"type": "array", "items": {"type": "string"}},
                   "limit": {"type": "integer", "minimum": 1, "maximum": 20}}}}),
        json!({"name": "explain", "description": "Why a place costs what it costs: route length and each feature's share of the cost on the cheapest route.",
               "input_schema": {"type": "object", "properties": {"place_id": {"type": "string"}}, "required": ["place_id"]}}),
        json!({"name": "rank_listings", "description": "Rank the available apartments/homes by how well they fit the current metric (call make_metric first): for each listing, the cost from its front door to the cheapest place of each category within a budget. Score 0-100 (100 = every category at the door). Returns the top ones with rent/price, beds, amenities, nearest places in minutes and a Matterport tour_url. Says so if the data has no listings.",
               "input_schema": {"type": "object", "properties": {
                   "limit": {"type": "integer", "minimum": 1, "maximum": 10},
                   "categories": {"type": "array", "items": {"type": "string"}},
                   "budget_min": {"type": "integer", "minimum": 5, "maximum": 60},
                   "include_unavailable": {"type": "boolean"}}}}),
        json!({"name": "finish", "description": "Deliver the answer. stops: chosen place ids with a one-line why. listings: for a housing request, the recommended listing ids from rank_listings with a one-line why. pitch: 2-4 sentences, grounded in tool results.",
               "input_schema": {"type": "object", "properties": {
                   "stops": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string"}, "why": {"type": "string"}}, "required": ["id", "why"]}},
                   "listings": {"type": "array", "items": {"type": "object", "properties": {"id": {"type": "string"}, "why": {"type": "string"}}, "required": ["id", "why"]}},
                   "pitch": {"type": "string"}},
                 "required": ["pitch"]}}),
    ]
}

/// All decimal numbers in `s` as f64 (e.g. "12.5 min, 3 stops" -> [12.5, 3.0]).
pub fn numbers_in(s: &str) -> Vec<f64> {
    let b = s.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let st = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if let Ok(v) = s[st..i].parse() {
                out.push(v);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Numbers in `pitch` that appear nowhere in `grounding`.
pub fn ungrounded_numbers(pitch: &str, grounding: &str) -> Vec<f64> {
    let known = numbers_in(grounding);
    numbers_in(pitch).into_iter().filter(|n| !known.iter().any(|k| (k - n).abs() < 1e-9)).collect()
}

fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')).map(|w| w.trim_matches('-')).filter(|w| !w.is_empty())
}

/// `n4802855943`, `w109782066`, `r2918026`: OSM node/way/relation ids as the data uses them.
pub fn looks_like_osm_id(w: &str) -> bool {
    let b = w.as_bytes();
    b.len() >= 5 && matches!(b[0], b'n' | b'w' | b'r') && b[1..].iter().all(u8::is_ascii_digit)
}

/// Model text with tool-call debris removed: a leading `pitch:` key, quotes wrapping the whole text,
/// unbalanced quotes and stray `{ } [ ] \` at either end. Balanced quotes inside are kept.
pub fn clean_text(s: &str) -> String {
    let mut t = s.trim().to_string();
    for key in ["{\"pitch\":", "\"pitch\":", "pitch:"] {
        if let Some(rest) = t.strip_prefix(key) {
            t = rest.trim().to_string();
        }
    }
    let count = |t: &str, c: char| t.matches(c).count();
    loop {
        let before = t.clone();
        if count(&t, '"') == 2 && t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
            t = t[1..t.len() - 1].to_string();
        }
        if let Some(c) = t.chars().last() {
            let stray = match c {
                '"' => count(&t, '"') % 2 == 1,
                '}' => count(&t, '{') < count(&t, '}'),
                ']' => count(&t, '[') < count(&t, ']'),
                '\\' => true,
                _ => false,
            };
            if stray {
                t.pop();
            }
        }
        if let Some(c) = t.chars().next() {
            let stray = match c {
                '"' => count(&t, '"') % 2 == 1,
                '{' => count(&t, '{') > count(&t, '}'),
                '[' => count(&t, '[') > count(&t, ']'),
                _ => false,
            };
            if stray {
                t.remove(0);
            }
        }
        t = t.trim().to_string();
        if t == before {
            return t;
        }
    }
}

fn minutes(s: f32) -> f64 {
    (s as f64 / 60.0 * 10.0).round() / 10.0
}

struct Session<'a> {
    core: &'a mut Core,
    detector: Option<&'a Detector>,
    detected: Vec<Object>,
    origin: u32,
    t: f32,
    /// Weekday of the trip (meta.date), Monday = 0; None if the date can't be read.
    day: Option<usize>,
    metric: Option<(MetricSpec, Metric)>,
    /// Budget (minutes) of the last rank_listings call, reused to score the picks in `finish`.
    budget: f32,
    lens: Vec<String>,
    /// Everything the model may legitimately quote: the request and every tool result.
    grounding: String,
}

impl Session<'_> {
    fn place_json(&self, i: usize) -> Value {
        let p = &self.core.places.places[i];
        let oh = p.tags.get("opening_hours");
        let mut v = json!({"id": p.id, "name": p.name, "category": p.category, "rating": p.rating, "opening_hours": oh});
        if let (Some(oh), Some(day)) = (oh.and_then(Value::as_str), self.day) {
            if let Some(h) = hours::today(oh, day, self.t) {
                for (k, x) in h.as_object().into_iter().flatten() {
                    v[k] = x.clone();
                }
            }
        }
        v
    }

    fn wants(args: &Value, p: &engine::graph::Place) -> bool {
        match args["categories"].as_array() {
            Some(c) if !c.is_empty() => c.iter().any(|x| x.as_str() == Some(p.category.as_str())),
            _ => true,
        }
    }

    /// Run one tool. Ok = result for the model; Err = tool error the model can react to.
    fn call(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "list_features" => Ok(json!(FEATURES
                .iter()
                .map(|f| json!({"id": f.id, "unit": f.unit, "desc": f.desc}))
                .collect::<Vec<_>>())),
            "list_labels" => Ok(json!({
                "labels": self.core.labels(),
                "always_available": ["bench", "toilets", "water"],
                "can_detect_new": self.detector.is_some()})),
            "list_places" => {
                let q = args["name"].as_str().map(|q| q.trim().to_lowercase()).filter(|q| !q.is_empty());
                let hit = |p: &engine::graph::Place| q.as_ref().is_none_or(|q| p.name.as_deref().is_some_and(|n| n.to_lowercase().contains(q.as_str())));
                let all: Vec<usize> = (0..self.core.places.places.len())
                    .filter(|&i| Self::wants(args, &self.core.places.places[i]) && hit(&self.core.places.places[i]))
                    .collect();
                let v: Vec<Value> = all.iter().take(60).map(|&i| self.place_json(i)).collect();
                let mut out = json!({"places": v});
                if all.len() > 60 {
                    out["note"] = json!(format!("showing 60 of {} matches: narrow with categories or name", all.len()));
                }
                Ok(out)
            }
            "make_metric" => {
                let spec: MetricSpec = serde_json::from_value(args.clone()).map_err(|e| format!("bad metric: {e}"))?;
                let m = Metric::compile(&spec)?;
                self.metric = Some((spec.clone(), m));
                Ok(json!({"ok": true, "metric": spec}))
            }
            "field" => {
                let (_, m) = self.metric.as_ref().ok_or("call make_metric first")?;
                let f = self.core.field(self.origin, m, self.t)?;
                let limit = args["limit"].as_u64().unwrap_or(8).clamp(1, 20) as usize;
                let mut idx: Vec<usize> = (0..f.place_cost.len())
                    .filter(|&i| f.place_cost[i].is_finite() && Self::wants(args, &self.core.places.places[i]))
                    .collect();
                idx.sort_by(|&a, &b| f.place_cost[a].total_cmp(&f.place_cost[b]));
                let unreachable = f.place_cost.iter().filter(|c| c.is_infinite()).count();
                let places: Vec<Value> = idx
                    .iter()
                    .take(limit)
                    .map(|&i| {
                        let mut v = self.place_json(i);
                        v["minutes"] = json!(minutes(f.place_cost[i]));
                        v
                    })
                    .collect();
                Ok(json!({"cheapest_places": places, "unreachable_places": unreachable}))
            }
            "rank_listings" => {
                let (_, m) = self.metric.as_ref().ok_or("call make_metric first")?;
                if self.core.listings.is_empty() {
                    return Ok(json!({"listings": [], "note": "this area has no listings data; say so and recommend places instead"}));
                }
                let lens: Vec<String> = args["categories"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                let budget = args["budget_min"].as_f64().unwrap_or(20.0).clamp(5.0, 60.0) as f32;
                let limit = args["limit"].as_u64().unwrap_or(5).clamp(1, 10) as usize;
                let only = !args["include_unavailable"].as_bool().unwrap_or(false);
                let ranked = self.core.rank_listings(m, self.t, budget, &lens, only)?;
                let top: Vec<Value> = ranked
                    .iter()
                    .take(limit)
                    .map(|r| {
                        let l = &self.core.listings[r.index];
                        let near: Vec<Value> = r.nearest.iter().filter(|n| n.minutes.is_finite()).take(4)
                            .map(|n| json!({"category": n.category, "place": n.place, "minutes": minutes(n.minutes * 60.0)})).collect();
                        json!({"listing_id": l.id, "name": l.name, "address": l.address, "kind": l.kind,
                            "rent_min": l.rent_min, "rent_max": l.rent_max, "price": l.price, "beds_min": l.beds_min, "beds_max": l.beds_max,
                            "baths_min": l.baths_min, "sqft_min": l.sqft_min, "amenities": l.amenities, "tour_url": l.tour_url(),
                            "score": (r.score as f64 * 10.0).round() / 10.0, "places_within_budget": r.reachable_places, "nearest": near})
                    })
                    .collect();
                self.budget = budget;
                self.lens = lens;
                Ok(json!({"budget_minutes": budget as f64, "ranked_of": ranked.len(), "listings": top,
                    "how": "score = mean over the categories of (1 - minutes/budget), cheapest place of each category from the front door under the current metric, x100"}))
            }
            "explain" => {
                let id = args["place_id"].as_str().ok_or("place_id required")?;
                let (_, m) = self.metric.as_ref().ok_or("call make_metric first")?;
                let i = self.core.places.places.iter().position(|p| p.id == id).ok_or("unknown place id")?;
                let ex = self.core.explain(self.origin, m, self.t, self.core.places.places[i].node)?;
                let terms: Vec<Value> = ex
                    .terms
                    .iter()
                    .map(|t| json!({"feature": t.0, "minutes": minutes(t.3), "share_pct": (t.3 / ex.total.max(1e-6) * 100.0).round() as i64}))
                    .collect();
                let mut v = self.place_json(i);
                v["route_metres"] = json!(ex.length_m.round() as i64);
                v["minutes"] = json!(minutes(ex.total));
                v["breakdown"] = json!(terms);
                Ok(v)
            }
            _ => Err(format!("unknown tool '{name}'")),
        }
    }

    /// Live detect of a new label around the origin (async: calls the detector).
    async fn detect(&mut self, args: &Value) -> Result<Value, String> {
        let det = self.detector.ok_or("live detection is not available (no Vexcel key)")?;
        let label = args["label"].as_str().ok_or("label required")?.trim().to_lowercase();
        if label.is_empty() || label.len() > 60 {
            return Err("bad label".into());
        }
        let radius = args["radius_m"].as_f64().unwrap_or(150.0).clamp(50.0, 300.0);
        let o = &self.core.graph.nodes[self.origin as usize];
        let (dlat, dlon) = (radius / 110_540.0, radius / (111_320.0 * o.lat.to_radians().cos()));
        let found = match det.detect(&label, [o.lon - dlon, o.lat - dlat, o.lon + dlon, o.lat + dlat]).await {
            Ok(f) => f,
            // 401/403 = no imagery access for this area (geofenced key): retrying any label here cannot work.
            Err(e) if e.contains(" 403") || e.contains(" 401") => {
                return Ok(json!({"label": label, "found": 0, "retryable": false,
                    "note": "live detection is not available for this area (imagery access denied). Not retryable here: do not call detect again for any label; use only labels from list_labels or built-in features."}));
            }
            Err(e) => return Err(e),
        };
        if found.is_empty() {
            return Ok(json!({"label": label, "found": 0, "note": "none detected in this area; do not use this label"}));
        }
        let pts: Vec<(f64, f64)> = found.iter().map(|d| (d.lon, d.lat)).collect();
        let total = self.core.add_objects(&label, &pts)?;
        for d in &found {
            self.detected.push(Object { label: label.clone(), lon: d.lon, lat: d.lat, src: Some(det.name().into()), conf: Some(d.conf) });
        }
        Ok(json!({"label": label, "found": found.len(), "total_objects": total, "area_m": (radius * 2.0) as i64}))
    }

    /// Place ids (known ones, or anything shaped like an OSM id) that appear as words in `s`.
    fn ids_in(&self, s: &str) -> Vec<String> {
        words(s).filter(|w| looks_like_osm_id(w) || self.core.places.places.iter().any(|p| p.id == *w)).map(String::from).collect()
    }

    /// `why` lines are shown as-is, so fix rather than reject: "(n123)" goes, a bare id becomes the place's name.
    fn drop_ids(&self, s: &str) -> String {
        let mut out = s.to_string();
        for id in self.ids_in(s) {
            let place = self.core.places.places.iter().find(|p| p.id == id);
            let name = place.map(|p| p.name.clone().unwrap_or_else(|| p.category.replace('_', " "))).unwrap_or_else(|| "this place".into());
            out = out.replace(&format!(" ({id})"), "").replace(&format!("({id})"), "").replace(&id, &name);
        }
        out
    }

    /// Validate a `finish` call; Ok = the final answer.
    fn finish(&self, args: &Value, llm: &str, called: Vec<String>) -> Result<AgentResponse, String> {
        let (spec, m) = self.metric.as_ref().ok_or("call make_metric before finish")?;
        let pitch = clean_text(args["pitch"].as_str().ok_or("pitch required")?);
        let raw = args["stops"].as_array().cloned().unwrap_or_default();
        let picks_raw = args["listings"].as_array().cloned().unwrap_or_default();
        if (raw.is_empty() && picks_raw.is_empty()) || pitch.is_empty() {
            return Err("need at least one stop (or listing) and a pitch".into());
        }
        let f = self.core.field(self.origin, m, self.t)?;
        let mut stops = vec![];
        for s in &raw {
            let id = s["id"].as_str().ok_or("stop id required")?;
            let i = self.core.places.places.iter().position(|p| p.id == id).ok_or(format!("unknown place id '{id}'"))?;
            let p = &self.core.places.places[i];
            stops.push(Stop {
                id: p.id.clone(),
                name: p.name.clone(),
                category: p.category.clone(),
                minutes: f.place_cost[i].is_finite().then(|| minutes(f.place_cost[i])),
                why: self.drop_ids(&clean_text(s["why"].as_str().unwrap_or(""))),
            });
        }
        let ids = self.ids_in(&pitch);
        if !ids.is_empty() {
            return Err(format!("your pitch mentions place ids {ids:?}. A person reads it: name the places instead (or their category and street), and call finish again."));
        }
        let bad = ungrounded_numbers(&pitch, &self.grounding);
        if !bad.is_empty() {
            return Err(format!("these numbers in your pitch are not in any tool result: {bad:?}. Rewrite the pitch using only numbers the tools returned."));
        }
        let mut listings = vec![];
        for s in &picks_raw {
            let id = s["id"].as_str().ok_or("listing id required")?;
            let l = self.core.listings.iter().find(|l| l.id == id).ok_or(format!("unknown listing id '{id}': use listing_id from rank_listings"))?;
            let score = self.core.rank_listings(m, self.t, self.budget, &self.lens, false).ok().and_then(|r| r.iter().find(|x| self.core.listings[x.index].id == id).map(|x| (x.score as f64 * 10.0).round() / 10.0));
            listings.push(ListingPick { id: l.id.clone(), name: l.name.clone(), address: l.address.clone(), score, tour_url: l.tour_url(), why: clean_text(s["why"].as_str().unwrap_or("")) });
        }
        if let Some(top) = listings.first().and_then(|p| p.tour_url.as_ref()) {
            if !pitch.contains(top.as_str()) {
                return Err(format!("put the Matterport link of your top listing in the pitch so the user can tour it: {top}"));
            }
        }
        Ok(AgentResponse { metric: spec.clone(), stops, pitch, llm: llm.to_string(), tools_called: called, detected: self.detected.clone(), listings })
    }
}

pub async fn run(core: &mut Core, llm: &Llm, detector: Option<&Detector>, req: AgentRequest) -> Result<AgentResponse, String> {
    let origin = match &req.origin {
        Some(Origin { node: Some(n), .. }) if (*n as usize) < core.graph.nodes.len() => *n,
        Some(Origin { lon: Some(lon), lat: Some(lat), .. }) => core.nearest_node(*lon, *lat),
        None => core.nearest_node((core.graph.meta.bbox[0] + core.graph.meta.bbox[2]) / 2.0, (core.graph.meta.bbox[1] + core.graph.meta.bbox[3]) / 2.0),
        _ => return Err("origin must be {lon, lat} or a valid {node}".into()),
    };
    let t = req.t.unwrap_or(720.0).rem_euclid(1440.0);
    let day = hours::weekday(&core.graph.meta.date);
    let when = day.map(|d| format!("{} {}, ", hours::DAY_NAMES[d], core.graph.meta.date)).unwrap_or_default();
    let ctx = format!("{}\n\n(Context: {when}leaving at {:02}:{:02} local time; sunrise {:02}:{:02}, sunset {:02}:{:02}.)", req.text.trim(), t as u32 / 60, t as u32 % 60,
        core.graph.meta.sunrise_min as u32 / 60, core.graph.meta.sunrise_min as u32 % 60, core.graph.meta.sunset_min as u32 / 60, core.graph.meta.sunset_min as u32 % 60);
    let mut s = Session { core, detector, detected: vec![], origin, t, day, metric: None, budget: 20.0, lens: vec![], grounding: ctx.clone() };
    let tools = tools();
    let mut messages = vec![json!({"role": "user", "content": ctx})];
    let mut called: Vec<String> = vec![];
    for _ in 0..MAX_TURNS {
        let resp = llm.complete(SYSTEM, &messages, &tools).await?;
        let content = resp["content"].as_array().cloned().unwrap_or_default();
        let uses: Vec<&Value> = content.iter().filter(|b| b["type"] == "tool_use").collect();
        if uses.is_empty() {
            return Err("model ended without calling finish".into());
        }
        messages.push(json!({"role": "assistant", "content": content}));
        let mut results = vec![];
        for u in uses {
            let (id, name, args) = (u["id"].clone(), u["name"].as_str().unwrap_or(""), &u["input"]);
            called.push(name.to_string());
            if name == "finish" {
                match s.finish(args, &llm.name(), called.clone()) {
                    Ok(r) => return Ok(r),
                    Err(e) => results.push(json!({"type": "tool_result", "tool_use_id": id, "content": e, "is_error": true})),
                }
                continue;
            }
            let outcome = if name == "detect" { s.detect(args).await } else { s.call(name, args) };
            match outcome {
                Ok(v) => {
                    let text = v.to_string();
                    s.grounding.push('\n');
                    s.grounding.push_str(&text);
                    results.push(json!({"type": "tool_result", "tool_use_id": id, "content": text}));
                }
                Err(e) => results.push(json!({"type": "tool_result", "tool_use_id": id, "content": e, "is_error": true})),
            }
        }
        messages.push(json!({"role": "user", "content": results}));
    }
    Err("agent did not finish within the turn limit".into())
}
