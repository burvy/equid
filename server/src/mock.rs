//! Offline stand-in for the model: reads the conversation so far and emits the next tool call, like a
//! well-behaved Claude would (list_features -> [list_labels -> detect] -> make_metric -> list_places -> field
//! -> explain -> finish). It exercises the whole tool loop and the grounding check without a key;
//! it is NOT the AI.

use serde_json::{json, Value};

fn user_text(messages: &[Value]) -> String {
    messages[0]["content"].as_str().unwrap_or("").to_lowercase()
}

/// Result JSON of the most recent call to tool `name`.
fn last_result(messages: &[Value], name: &str) -> Option<Value> {
    let mut ids = std::collections::HashMap::new();
    for m in messages {
        if m["role"] == "assistant" {
            for b in m["content"].as_array().into_iter().flatten() {
                if b["type"] == "tool_use" {
                    ids.insert(b["id"].as_str().unwrap_or("").to_string(), b["name"].as_str().unwrap_or("").to_string());
                }
            }
        }
    }
    let mut found = None;
    for m in messages {
        if m["role"] == "user" {
            for b in m["content"].as_array().into_iter().flatten() {
                if b["type"] == "tool_result" && ids.get(b["tool_use_id"].as_str().unwrap_or("")).map(|n| n == name).unwrap_or(false) {
                    found = serde_json::from_str(b["content"].as_str().unwrap_or("")).ok();
                }
            }
        }
    }
    found
}

fn tool(n: usize, name: &str, input: Value) -> Value {
    json!({"stop_reason": "tool_use", "content": [{"type": "tool_use", "id": format!("mock_{n}"), "name": name, "input": input}]})
}

/// Object labels a mock "model" would look for, from keywords in the request (label, categories of places to stop at).
const OBJECTS: &[(&[&str], &str)] = &[
    (&["playground", "kids", "toddler"], "playground"),
    (&["dog"], "dog park"),
    (&["trash", "litter"], "trash can"),
    (&["crosswalk", "cross the road"], "crosswalk"),
];

pub fn respond(messages: &[Value]) -> Value {
    let text = user_text(messages);
    let has = |w: &[&str]| w.iter().any(|k| text.contains(k));
    let wanted_label = OBJECTS.iter().find(|(k, _)| has(k)).map(|(_, l)| *l);

    // plan = the tool sequence for this kind of request
    let mut plan = vec!["list_features"];
    if wanted_label.is_some() {
        plan.push("list_labels");
        // detect only when the label is not in the menu (decided from the list_labels result)
        let in_menu = wanted_label
            .and_then(|l| last_result(messages, "list_labels").map(|r| r["labels"].as_array().map(|a| a.iter().any(|x| x["label"] == l)).unwrap_or(false)))
            .unwrap_or(true);
        if !in_menu {
            plan.push("detect");
        }
    }
    plan.extend(["make_metric", "list_places", "field", "explain", "finish"]);
    let step = messages.iter().filter(|m| m["role"] == "assistant").count();
    let name = plan.get(step).copied().unwrap_or("finish");

    let (mut weights, forbid, cats) = if has(&["stroller", "pram", "wheelchair"]) {
        (json!({"time": 1, "climb": 10, "steep": 3, "rough": 3}), json!(["steps"]), if has(&["coffee", "cafe", "café"]) { vec!["cafe"] } else { vec![] })
    } else if has(&["night", "dark", "walk home"]) {
        (json!({"time": 1, "unlit": 3, "busy": 0.5}), json!([]), vec!["cafe", "library", "pharmacy"])
    } else if has(&["hot", "sun", "shade"]) {
        (json!({"time": 1, "sun": 1.5, "water_gap": 0.5, "climb": 3}), json!([]), vec!["drinking_water", "park", "cafe"])
    } else {
        (json!({"time": 1}), json!([]), vec![])
    };
    if let Some(l) = wanted_label {
        weights[format!("near:{l}")] = json!(2);
    }

    match name {
        "list_features" => tool(step, name, json!({})),
        "list_labels" => tool(step, name, json!({})),
        "detect" => tool(step, name, json!({"label": wanted_label.unwrap_or("playground"), "radius_m": 200})),
        "make_metric" => tool(step, name, json!({"mode": "foot", "weights": weights, "forbid": forbid})),
        "list_places" => tool(step, name, json!({"categories": cats})),
        "field" => tool(step, name, json!({"categories": cats, "limit": 3})),
        "explain" => {
            let id = last_result(messages, "field").and_then(|f| f["cheapest_places"][0]["id"].as_str().map(String::from)).unwrap_or_default();
            tool(step, name, json!({"place_id": id}))
        }
        _ => {
            let field = last_result(messages, "field").unwrap_or(json!({}));
            let ex = last_result(messages, "explain").unwrap_or(json!({}));
            let places = field["cheapest_places"].as_array().cloned().unwrap_or_default();
            let place = ex["name"].as_str().or(ex["category"].as_str()).unwrap_or("this spot");
            let mut pitch = format!("{place} is your best bet: {} minutes of personal cost over {} metres of route.", ex["minutes"], ex["route_metres"]);
            if let Some(b) = ex["breakdown"].as_array().and_then(|b| b.iter().max_by(|x, y| x["share_pct"].as_i64().cmp(&y["share_pct"].as_i64()))) {
                pitch += &format!(" Most of that is {} ({}%), which your weights account for.", b["feature"].as_str().unwrap_or("time"), b["share_pct"]);
            }
            let stops: Vec<Value> = places.iter().take(2).map(|p| json!({"id": p["id"], "why": format!("{} minutes away under your metric", p["minutes"])})).collect();
            tool(step, "finish", json!({"stops": stops, "pitch": pitch}))
        }
    }
}
