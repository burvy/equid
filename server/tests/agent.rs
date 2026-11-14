//! The three scripted prompts through the whole agent loop (mock model; swap in a key for the real one).

use engine::metric::Metric;
use server::agent::{numbers_in, run, ungrounded_numbers, AgentRequest};
use server::llm::Llm;
use server::vexcel::Detector;

fn core() -> engine::Core {
    engine::Core::load(include_str!("../../data/fixtures/graph.json"), include_str!("../../data/fixtures/places.json")).unwrap()
}

fn core_with_objects() -> engine::Core {
    let mut c = core();
    c.load_objects(include_str!("../../data/fixtures/objects.json")).unwrap();
    c
}

async fn ask_with(mut c: engine::Core, det: Option<&Detector>, text: &str, t: f32) -> server::agent::AgentResponse {
    let req = AgentRequest { text: text.into(), origin: Some(server::agent::Origin { lon: None, lat: None, node: Some(0) }), t: Some(t) };
    run(&mut c, &Llm::Mock, det, req).await.expect("agent finishes")
}

async fn ask(text: &str, t: f32) -> server::agent::AgentResponse {
    ask_with(core(), None, text, t).await
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
    let mut c = core();
    let req = AgentRequest { text: "x".into(), origin: Some(server::agent::Origin { lon: Some(1.0), lat: None, node: None }), t: None };
    assert!(run(&mut c, &Llm::Mock, None, req).await.is_err());
}

#[tokio::test]
async fn dog_walker_uses_a_label_from_the_menu() {
    let r = ask_with(core_with_objects(), None, "walking my dog, want to pass a dog park and a bin", 17.0 * 60.0).await;
    check(&r);
    assert!(r.metric.weights.contains_key("near:dog park"));
    assert!(r.tools_called.contains(&"list_labels".to_string()));
    assert!(!r.tools_called.contains(&"detect".to_string()), "in the menu: no live detect needed");
    assert!(r.detected.is_empty());
}

#[tokio::test]
async fn label_missing_from_the_menu_triggers_a_live_detect() {
    let det = Detector::Fixed { n: 3 };
    let r = ask_with(core_with_objects(), Some(&det), "I have a toddler, take me past a playground", 11.0 * 60.0).await;
    check(&r);
    assert!(r.tools_called.contains(&"detect".to_string()));
    assert_eq!(r.detected.len(), 3);
    assert!(r.detected.iter().all(|o| o.label == "playground"));
    assert!(r.metric.weights.contains_key("near:playground"));
}

#[tokio::test]
async fn without_a_detector_the_detect_tool_reports_unavailable_and_agent_does_not_finish_with_it() {
    let mut c = core_with_objects();
    let req = AgentRequest { text: "kids playground please".into(), origin: Some(server::agent::Origin { lon: None, lat: None, node: Some(0) }), t: Some(600.0) };
    // the mock keeps calling make_metric with near:playground, which the engine rejects (no data): run must fail, not fake it
    assert!(run(&mut c, &Llm::Mock, None, req).await.is_err());
}
