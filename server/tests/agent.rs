//! The three scripted prompts through the whole agent loop (mock model; swap in a key for the real one).

use engine::metric::Metric;
use server::agent::{numbers_in, run, ungrounded_numbers, AgentRequest};
use server::llm::Llm;

fn core() -> engine::Core {
    engine::Core::load(include_str!("../../data/fixtures/graph.json"), include_str!("../../data/fixtures/places.json")).unwrap()
}

async fn ask(text: &str, t: f32) -> server::agent::AgentResponse {
    let c = core();
    let req = AgentRequest { text: text.into(), origin: Some(server::agent::Origin { lon: None, lat: None, node: Some(0) }), t: Some(t) };
    run(&c, &Llm::Mock, req).await.expect("agent finishes")
}

fn check(r: &server::agent::AgentResponse) {
    // the engine accepts the metric
    let spec = serde_json::to_string(&r.metric).unwrap();
    Metric::from_json(&spec).expect("valid metric");
    assert!(r.metric.weights.values().all(|w| *w >= 0.0));
    assert!(!r.stops.is_empty());
    assert!(r.stops.iter().all(|s| s.minutes.is_some()));
    assert!(r.pitch.len() > 30);
    assert_eq!(r.tools_called.last().map(String::as_str), Some("finish"));
}

#[tokio::test]
async fn stroller_plus_coffee() {
    let r = ask("I'm pushing a stroller and want a coffee", 10.0 * 60.0).await;
    check(&r);
    assert!(r.metric.forbid.contains(&"steps".to_string()));
    assert_eq!(r.stops[0].category, "cafe");
}

#[tokio::test]
async fn hot_afternoon() {
    let r = ask("It's a hot afternoon, I need shade and water", 15.0 * 60.0).await;
    check(&r);
    assert!(r.metric.weights.contains_key("sun"));
}

#[tokio::test]
async fn night_walk_home() {
    let r = ask("night walk home, keep me on lit streets", 22.0 * 60.0).await;
    check(&r);
    assert!(r.metric.weights.contains_key("unlit"));
}

#[tokio::test]
async fn pitch_numbers_are_grounded_and_checker_rejects_invented_ones() {
    let r = ask("stroller and coffee", 600.0).await;
    // numbers in the pitch all come from the tool trace, so the finished answer passed the check
    assert!(!numbers_in(&r.pitch).is_empty());
    assert_eq!(ungrounded_numbers("It is 7.3 minutes away", "minutes 7.3 route 120"), Vec::<f64>::new());
    assert_eq!(ungrounded_numbers("It is 9 minutes away, 120 m", "minutes 7.3 route 120"), vec![9.0]);
}

#[tokio::test]
async fn bad_origin_is_an_error() {
    let c = core();
    let req = AgentRequest { text: "x".into(), origin: Some(server::agent::Origin { lon: Some(1.0), lat: None, node: None }), t: None };
    assert!(run(&c, &Llm::Mock, req).await.is_err());
}
