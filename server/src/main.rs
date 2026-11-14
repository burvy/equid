//! `cargo run -p server`. Env: ANTHROPIC_API_KEY (else a mock model), EQUID_MODEL, EQUID_DATA (dir with
//! graph.json + places.json, default data/fixtures), PORT (default 8787).

use server::{app, llm::Llm, AppState};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let dir = std::env::var("EQUID_DATA").unwrap_or_else(|_| "data/fixtures".into());
    let read = |f: &str| std::fs::read_to_string(format!("{dir}/{f}")).unwrap_or_else(|e| panic!("{dir}/{f}: {e}"));
    let core = engine::Core::load(&read("graph.json"), &read("places.json")).expect("data loads");
    let llm = Llm::from_env();
    let port = std::env::var("PORT").unwrap_or_else(|_| "8787".into());
    println!("equid server: {} nodes, {} places from {dir}; model: {}", core.graph.nodes.len(), core.places.places.len(), llm.name());
    if matches!(llm, Llm::Mock) {
        println!("ANTHROPIC_API_KEY not set: using the offline mock model");
    }
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await.expect("bind");
    axum::serve(listener, app(Arc::new(AppState { core, llm }))).await.unwrap();
}
