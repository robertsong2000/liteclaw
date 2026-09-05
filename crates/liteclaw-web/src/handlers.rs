//! HTTP handlers: the chat SSE endpoint and the index page.

use crate::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use liteclaw_agent::{default_tools, extra_tools, into_stream, skill_tools};
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
pub async fn chat(State(state): State<AppState>, Json(req): Json<ChatRequest>) -> Response {
    // Build the model client from the frontend-supplied config.
    let model = match liteclaw_model::OpenAiClient::new(req.model) {
        Ok(m) => m,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("bad model config: {e}")).into_response();
        }
    };

    // Build the tool set: core claws + skill tools (list/run).
    let mut tools = default_tools(&state.claws);
    if let Some(first) = state.claws.first() {
        tools.extend(skill_tools(first.clone()));
        tools.extend(extra_tools(first.clone()));
    }

    // Wire the confirm callback. In auto_mode all tools run without asking;
    // otherwise write/edit/bash pause for human approval via POST /api/confirm.
    let ctx = state.ctx.clone();
    let confirm = if req.auto_mode {
        None // no callback → Confirm tools execute immediately (see agent loop)
    } else {
        Some(crate::make_confirm(state.confirms.clone()))
    };

    // Inject AGENTS.md into the system prompt: read from cwd, prepend to the
    // first system message so the model knows project conventions.
    let messages = inject_agents_md(req.messages, &ctx.cwd);

    let (rx, _handle) = into_stream(model, messages, tools, ctx, confirm, 8);

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

/// GET /help — embedded RAG question map: what users can ask and how.
pub async fn help() -> Response {
    let html = include_str!("static/help.html");
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("mkdir failed: {e}"),
        )
            .into_response();
    }
    match serde_json::to_string_pretty(&cfg) {
        Ok(text) => match std::fs::write(&path, text) {
            Ok(_) => (StatusCode::OK, "saved").into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write failed: {e}"),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize failed: {e}"),
        )
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
pub async fn confirm(State(state): State<AppState>, Json(req): Json<ConfirmRequest>) -> Response {
    if state.confirms.resolve(&req.confirm_id, req.allowed) {
        (StatusCode::OK, "resolved").into_response()
    } else {
        (StatusCode::NOT_FOUND, "no such pending confirmation").into_response()
    }
}

// ─── Conversation history persistence ────────────────────────────────

/// One conversation session.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Session {
    pub id: String,
    pub title: String,
    /// Full OpenAI-schema messages, including tool_calls / tool results, so a
    /// session can be restored with zero context loss on switch.
    pub messages: Vec<liteclaw_model::Message>,
    pub updated: u64,
}

/// The on-disk history file: a list of sessions.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct HistoryFile {
    sessions: Vec<Session>,
}

fn history_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home).join(".liteclaw/history.json")
}

fn read_history() -> HistoryFile {
    std::fs::read_to_string(history_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_history(h: &HistoryFile) -> Result<(), std::io::Error> {
    let path = history_path();
    std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")))?;
    let text = serde_json::to_string_pretty(h)?;
    std::fs::write(&path, text)
}

/// GET /api/history — list all saved sessions (without full messages).
pub async fn list_history() -> Response {
    let h = read_history();
    // Return summaries only (id, title, updated, message count) to keep it light.
    let summaries: Vec<serde_json::Value> = h
        .sessions
        .iter()
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "title": s.title,
                "updated": s.updated,
                "message_count": s.messages.len(),
            })
        })
        .collect();
    Json(serde_json::json!({ "sessions": summaries })).into_response()
}

/// GET /api/history/:id — full messages of one session.
pub async fn get_session(axum::extract::Path(id): axum::extract::Path<String>) -> Response {
    let h = read_history();
    match h.sessions.iter().find(|s| s.id == id) {
        Some(s) => Json(s).into_response(),
        None => (StatusCode::NOT_FOUND, "session not found").into_response(),
    }
}

/// POST /api/history — save (create or update) a session.
pub async fn save_session(Json(session): Json<Session>) -> Response {
    let mut h = read_history();
    // Upsert: replace if id exists, else push.
    if let Some(existing) = h.sessions.iter_mut().find(|s| s.id == session.id) {
        // Update in place: preserve the session's position in the list so the
        // sidebar doesn't jump around when a conversation gets new messages.
        *existing = session;
    } else {
        // New session: prepend so it lands on top (once). Subsequent saves hit
        // the update-in-place branch above and don't move it.
        h.sessions.insert(0, session);
    }
    // Keep only the latest 50 sessions.
    if h.sessions.len() > 50 {
        let cutoff = h.sessions.len() - 50;
        h.sessions.drain(..cutoff);
    }
    match write_history(&h) {
        Ok(_) => (StatusCode::OK, "saved").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("write: {e}")).into_response(),
    }
}

/// DELETE /api/history/:id — delete a session.
pub async fn delete_session(axum::extract::Path(id): axum::extract::Path<String>) -> Response {
    let mut h = read_history();
    let before = h.sessions.len();
    h.sessions.retain(|s| s.id != id);
    if h.sessions.len() == before {
        return (StatusCode::NOT_FOUND, "session not found").into_response();
    }
    match write_history(&h) {
        Ok(_) => (StatusCode::OK, "deleted").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("write: {e}")).into_response(),
    }
}

// ─── AGENTS.md injection ─────────────────────────────────────────────

/// Read AGENTS.md from cwd and prepend its content to the first system message.
/// If no system message exists, create one. If AGENTS.md is absent, pass through.
fn inject_agents_md(mut messages: Vec<Message>, cwd: &std::path::Path) -> Vec<Message> {
    let agents_md = cwd.join("AGENTS.md");
    let Some(content) = std::fs::read_to_string(&agents_md).ok() else {
        return messages; // no AGENTS.md, nothing to inject
    };
    let snippet = format!(
        "\n\n--- 项目 AGENTS.md 约定 ---\n{content}\n--- AGENTS.md 结束 ---"
    );

    // Find the first system message and append. If none, prepend a new one.
    if let Some(first) = messages.iter_mut().find(|m| m.role == liteclaw_model::Role::System) {
        match &first.content {
            Some(serde_json::Value::String(s)) => {
                let mut combined = s.clone();
                combined.push_str(&snippet);
                first.content = Some(serde_json::Value::String(combined));
            }
            _ => {
                first.content = Some(serde_json::Value::String(snippet.trim().into()));
            }
        }
    } else {
        messages.insert(0, Message::system(snippet.trim()));
    }
    messages
}
