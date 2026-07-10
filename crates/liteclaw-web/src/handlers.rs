//! HTTP handlers: the chat SSE endpoint and the index page.

use crate::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use liteclaw_agent::{default_tools, into_stream};
use liteclaw_model::{Message, ModelConfig};

/// Request body for POST /api/chat.
#[derive(serde::Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub model: ModelConfig,
    /// When true, all tools (including write/edit/bash) auto-execute without
    /// waiting for human confirmation.
    #[serde(default)]
    pub auto_mode: bool,
}

/// POST /api/chat — start an agent turn and stream events back as SSE.
pub async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Response {
    // Build the model client from the frontend-supplied config.
    let model = match liteclaw_model::OpenAiClient::new(req.model) {
        Ok(m) => m,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("bad model config: {e}"))
                .into_response();
        }
    };

    // Build the tool set from the registered claws (read-only tools auto-run).
    let tools = default_tools(&state.claws);

    // Wire the confirm callback. In auto_mode all tools run without asking;
    // otherwise write/edit/bash pause for human approval via POST /api/confirm.
    let ctx = state.ctx.clone();
    let confirm = if req.auto_mode {
        None // no callback → Confirm tools execute immediately (see agent loop)
    } else {
        Some(crate::make_confirm(state.confirms.clone()))
    };
    let (rx, _handle) = into_stream(model, req.messages, tools, ctx, confirm, 8);

    // Serialize each AgentEvent as an SSE frame.
    let sse = tokio_stream::wrappers::ReceiverStream::new(rx).map(|event| {
        let json = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
        // SSE frame: "data: <json>\n\n"
        Ok::<_, std::convert::Infallible>(format!("data: {json}\n\n"))
    });

    let body = Body::from_stream(sse);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/event-stream"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}

/// GET / — serve the embedded single-page UI.
pub async fn index() -> Response {
    let html = include_str!("static/index.html");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

/// Path to the persisted config file: `~/.liteclaw/config.json`.
fn config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home).join(".liteclaw/config.json")
}

/// GET /api/config — read the saved model config (or defaults if absent).
pub async fn get_config() -> Response {
    let path = config_path();
    let cfg = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<ModelConfig>(&text).unwrap_or_default(),
        Err(_) => ModelConfig::default(),
    };
    Json(cfg).into_response()
}

/// POST /api/config — persist the model config to `~/.liteclaw/config.json`.
pub async fn post_config(Json(cfg): Json<ModelConfig>) -> Response {
    let path = config_path();
    if let Err(e) = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new("."))) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir failed: {e}"))
            .into_response();
    }
    match serde_json::to_string_pretty(&cfg) {
        Ok(text) => match std::fs::write(&path, text) {
            Ok(_) => (StatusCode::OK, "saved").into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("write failed: {e}"))
                .into_response(),
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize failed: {e}"))
            .into_response(),
    }
}

/// Request body for POST /api/confirm.
#[derive(serde::Deserialize)]
pub struct ConfirmRequest {
    pub confirm_id: String,
    pub allowed: bool,
}

/// POST /api/confirm — resolve a pending tool confirmation from the frontend.
pub async fn confirm(
    State(state): State<AppState>,
    Json(req): Json<ConfirmRequest>,
) -> Response {
    if state.confirms.resolve(&req.confirm_id, req.allowed) {
        (StatusCode::OK, "resolved").into_response()
    } else {
        (StatusCode::NOT_FOUND, "no such pending confirmation").into_response()
    }
}
