//! Offline stand-in for the model: reads the conversation so far and emits the next tool call, like a
//! well-behaved Claude would (list_features -> make_metric -> list_places -> field -> explain -> finish).
//! It exercises the whole tool loop and the grounding check without a key; it is NOT the AI.

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

pub fn respond(messages: &[Value]) -> Value {
    let text = user_text(messages);
    let step = messages.iter().filter(|m| m["role"] == "assistant").count();
    let has = |w: &[&str]| w.iter().any(|k| text.contains(k));

    let (weights, forbid, cats) = if has(&["stroller", "pram", "wheelchair"]) {
        (json!({"time": 1, "climb": 10, "steep": 3, "rough": 3}), json!(["steps"]), if has(&["coffee", "cafe", "café"]) { vec!["cafe"] } else { vec![] })
    } else if has(&["night", "dark", "walk home"]) {
        (json!({"time": 1, "unlit": 3, "busy": 0.5}), json!([]), vec!["cafe", "library", "pharmacy"])
    } else if has(&["hot", "sun", "shade"]) {
        (json!({"time": 1, "sun": 1.5, "water_gap": 0.5, "climb": 3}), json!([]), vec!["drinking_water", "park", "cafe"])
    } else {
        (json!({"time": 1}), json!([]), vec![])
    };

    match step {
        0 => tool(step, "list_features", json!({})),
        1 => tool(step, "make_metric", json!({"mode": "foot", "weights": weights, "forbid": forbid})),
        2 => tool(step, "list_places", json!({"categories": cats})),
        3 => tool(step, "field", json!({"categories": cats, "limit": 3})),
        4 => {
            let id = last_result(messages, "field").and_then(|f| f["cheapest_places"][0]["id"].as_str().map(String::from)).unwrap_or_default();
            tool(step, "explain", json!({"place_id": id}))
        }
        _ => {
            let field = last_result(messages, "field").unwrap_or(json!({}));
            let ex = last_result(messages, "explain").unwrap_or(json!({}));
            let places = field["cheapest_places"].as_array().cloned().unwrap_or_default();
            let top = &ex;
            let name = top["name"].as_str().or(top["category"].as_str()).unwrap_or("this spot");
            let mut pitch = format!(
                "{name} is your best bet: {} minutes of personal cost over {} metres of route.",
                top["minutes"], top["route_metres"]
            );
            if let Some(b) = top["breakdown"].as_array().and_then(|b| b.iter().max_by(|x, y| x["share_pct"].as_i64().cmp(&y["share_pct"].as_i64()))) {
                pitch += &format!(" Most of that is {} ({}%), which your weights account for.", b["feature"].as_str().unwrap_or("time"), b["share_pct"]);
            }
            let stops: Vec<Value> = places.iter().take(2).map(|p| json!({"id": p["id"], "why": format!("{} minutes away under your metric", p["minutes"])})).collect();
            tool(step, "finish", json!({"stops": stops, "pitch": pitch}))
        }
    }
}
