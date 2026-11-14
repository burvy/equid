//! The one place the model provider is touched. Everything else sees `Llm::complete`:
//! (system, messages, tools) -> a Messages-API-shaped response `{content: [blocks], stop_reason}`.
//! Swapping provider = changing this file. The API key lives only in `Llm::Anthropic.key`
//! (never printed: no Debug derive, errors never include request headers).

use serde_json::{json, Value};

pub enum Llm {
    Anthropic { key: String, model: String, client: reqwest::Client },
    /// Deterministic offline stand-in that drives the same tool loop (tests, demo without a key).
    Mock,
}

pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";

impl Llm {
    /// `ANTHROPIC_API_KEY` set -> real Claude (model from `EQUID_MODEL`, default `DEFAULT_MODEL`); else the mock.
    pub fn from_env() -> Llm {
        match std::env::var("ANTHROPIC_API_KEY") {
            Ok(key) if !key.trim().is_empty() => Llm::Anthropic {
                key: key.trim().to_string(),
                model: std::env::var("EQUID_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
                client: reqwest::Client::new(),
            },
            _ => Llm::Mock,
        }
    }

    pub fn name(&self) -> String {
        match self {
            Llm::Anthropic { model, .. } => format!("anthropic:{model}"),
            Llm::Mock => "mock".to_string(),
        }
    }

    pub async fn complete(&self, system: &str, messages: &[Value], tools: &[Value]) -> Result<Value, String> {
        match self {
            Llm::Mock => Ok(crate::mock::respond(messages)),
            Llm::Anthropic { key, model, client } => {
                let body = json!({
                    "model": model,
                    "max_tokens": 2048,
                    "system": system,
                    "tools": tools,
                    "messages": messages,
                });
                let resp = client
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| format!("anthropic request failed: {}", e.without_url()))?;
                let status = resp.status();
                let v: Value = resp.json().await.map_err(|e| format!("anthropic response not JSON: {}", e.without_url()))?;
                if !status.is_success() {
                    let msg = v["error"]["message"].as_str().unwrap_or("unknown error");
                    return Err(format!("anthropic {status}: {msg}"));
                }
                Ok(v)
            }
        }
    }
}
