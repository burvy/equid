//! Equid trip-agent server. `POST /agent {text, origin?, t?}` -> `{metric, stops, pitch, llm, tools_called}`.

pub mod agent;
pub mod hours;
pub mod llm;
pub mod mock;
pub mod vexcel;

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use engine::Core;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct AppState {
    /// Mutable: live detects add objects. Requests are serialised (fine for a demo server).
    pub core: tokio::sync::Mutex<Core>,
    pub llm: llm::Llm,
    pub detector: Option<vexcel::Detector>,
}

async fn agent_handler(State(st): State<Arc<AppState>>, Json(req): Json<agent::AgentRequest>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match agent::run(&mut *st.core.lock().await, &st.llm, st.detector.as_ref(), req).await {
        Ok(r) => Ok(Json(serde_json::to_value(r).unwrap())),
        Err(e) => Err((StatusCode::BAD_GATEWAY, Json(json!({"error": e})))),
    }
}

pub fn app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/agent", post(agent_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}
