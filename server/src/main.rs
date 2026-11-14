//! `cargo run -p server`. Env: ANTHROPIC_API_KEY (else a mock model), EQUID_MODEL, EQUID_DATA (dir with
//! graph.json + places.json, default data/fixtures), PORT (default 8787).

use server::{app, llm::Llm, vexcel::Detector, AppState};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let dir = std::env::var("EQUID_DATA").unwrap_or_else(|_| "data/fixtures".into());
    let read = |f: &str| std::fs::read_to_string(format!("{dir}/{f}")).unwrap_or_else(|e| panic!("{dir}/{f}: {e}"));
    let mut core = engine::Core::load(&read("graph.json"), &read("places.json")).expect("data loads");
    if let Ok(objs) = std::fs::read_to_string(format!("{dir}/objects.json")) {
        let n = core.load_objects(&objs).expect("objects.json loads");
        println!("loaded {n} objects, labels: {}", core.labels().iter().map(|l| l.label.as_str()).collect::<Vec<_>>().join(", "));
    }
    let detector = Detector::from_env();
    println!("live object detection: {}", if detector.is_some() { "on (Vexcel key found)" } else { "off (no VEXCEL_API_KEY)" });
    let llm = Llm::from_env();
    let port = std::env::var("PORT").unwrap_or_else(|_| "8787".into());
    println!("equid server: {} nodes, {} places from {dir}; model: {}", core.graph.nodes.len(), core.places.places.len(), llm.name());
    if matches!(llm, Llm::Mock) {
        println!("ANTHROPIC_API_KEY not set: using the offline mock model");
    }
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.expect("bind");
    axum::serve(listener, app(Arc::new(AppState { core: tokio::sync::Mutex::new(core), llm, detector }))).await.unwrap();
}
