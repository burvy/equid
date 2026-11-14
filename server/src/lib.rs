//! Equid trip-agent server. `POST /agent {text, origin?, t?}` -> `{metric, stops, pitch, llm, tools_called}`.

pub mod agent;
pub mod llm;
pub mod mock;

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use engine::Core;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct AppState {
    pub core: Core,
    pub llm: llm::Llm,
}

async fn agent_handler(State(st): State<Arc<AppState>>, Json(req): Json<agent::AgentRequest>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    match agent::run(&st.core, &st.llm, req).await {
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
