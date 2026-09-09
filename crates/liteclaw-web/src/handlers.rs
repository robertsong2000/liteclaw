//! HTTP handlers: the chat SSE endpoint and the index page.

use crate::AppState;
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use liteclaw_agent::{default_tools, extra_tools, into_stream, skill_tools, Tool};
use liteclaw_core::Ctx;
use liteclaw_model::{Message, ModelConfig, Role};

/// Request body for POST /api/chat.
#[derive(serde::Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub model: ModelConfig,
    /// When true, all tools (including write/edit/bash) auto-execute without
    /// waiting for human confirmation.
    #[serde(default)]
    pub auto_mode: bool,
    /// When true, the server retrieves manual passages before the model turn
    /// (AUTO-RAG injection, tool cards stay hidden). Defaults to false: the
    /// model drives retrieval itself via skill_list/skill_run, which surfaces
    /// the tool-call cards in the UI.
    #[serde(default)]
    pub auto_rag: bool,
}

/// AUTO-RAG (vehicle-assistant deployment): run the manual-rag skill
/// server-side for every turn and compact the LLM payload, so grounding never
/// depends on the model choosing to retrieve. Disable with LITECLAW_AUTO_RAG=0.
const AUTO_RAG_SKILL: &str = "manual-rag";
/// Newest text-only messages kept verbatim in the compacted payload.
const AUTO_RAG_KEEP: usize = 8;
/// Cap on older assistant answers kept in the compacted payload.
const ASSISTANT_TEXT_CAP: usize = 400;

/// Extract plain text from a message content (string or multimodal parts).
fn message_text(m: &Message) -> String {
    match &m.content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .find_map(|p| p.get("text").and_then(|t| t.as_str()).map(String::from))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Compact the LLM payload: system prompt + last `AUTO_RAG_KEEP` text-only
/// entries. Tool messages and intermediate tool-call rounds are dropped and
/// long assistant answers truncated — the passages the model needs are
/// injected fresh for the current turn, so stale ones in history only dilute
/// attention (measured drift trigger: >10K chars of accumulated context).
fn compact_history(messages: Vec<Message>) -> Vec<Message> {
    let mut sys = None;
    let mut rest: Vec<Message> = Vec::new();
    for m in messages {
        match m.role {
            Role::System if sys.is_none() => sys = Some(m),
            Role::Tool => continue,
            Role::Assistant if m.tool_calls.is_some() => continue,
            Role::Assistant => {
                let mut m = m;
                let text = message_text(&m);
                if text.chars().count() > ASSISTANT_TEXT_CAP {
                    let head: String = text.chars().take(ASSISTANT_TEXT_CAP).collect();
                    m.content = Some(serde_json::Value::String(format!("{head}…")));
                }
                rest.push(m);
            }
            _ => rest.push(m),
        }
    }
    let start = rest.len().saturating_sub(AUTO_RAG_KEEP);
    let mut out = Vec::with_capacity(rest.len() - start + 1);
    if let Some(sys) = sys {
        out.push(sys);
    }
    out.extend(rest.split_off(start));
    out
}

/// Run the RAG skill for the newest user question and insert the retrieved
/// passages directly before that question, so the model reads fresh manual
/// content every turn without needing to initiate retrieval itself.
async fn inject_rag(messages: &mut Vec<Message>, skill_tool: &Tool, ctx: &Ctx) {
    let Some(question) = messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| message_text(m))
    else {
        return;
    };
    if question.trim().is_empty() {
        return;
    }
    let args = serde_json::json!({ "id": AUTO_RAG_SKILL, "args": question });
    let outcome = skill_tool.execute(&args, ctx).await;
    if !outcome.ok || outcome.summary.trim().is_empty() {
        return;
    }
    let grounding = format!(
        "【本轮手册检索结果——已由系统代为查询,无需再调 skill_run】\n{}\n\
         以上原文若已覆盖问题,直接作答即可;若未覆盖,可用不同关键词再检索一次;\
         检索不到的内容明确说手册中没有,不要凭记忆作答。",
        outcome.summary
    );
    let grounding_msg = Message {
        role: Role::User,
        content: Some(serde_json::Value::String(grounding)),
        tool_calls: None,
        tool_call_id: None,
    };
    // Insert right before the trailing user question.
    let user = messages.pop();
    messages.push(grounding_msg);
    if let Some(u) = user {
        messages.push(u);
    }
}

/// Server-side model endpoint registry: for models listed in config.json's
/// `model_endpoints` map (gateway-hosted models), override the frontend-
/// supplied base_url/api_key. Keys stay server-side and never reach the
/// browser; models not listed keep whatever the frontend sent (local Ollama).
fn resolve_model_endpoint(cfg: &mut ModelConfig) {
    let Ok(text) = std::fs::read_to_string(config_path()) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return;
    };
    let Some(ep) = v.get("model_endpoints").and_then(|m| m.get(&cfg.model)) else {
        return;
    };
    if let Some(u) = ep.get("base_url").and_then(|x| x.as_str()) {
        cfg.base_url = u.to_string();
    }
    if let Some(k) = ep.get("api_key").and_then(|x| x.as_str()) {
        cfg.api_key = k.to_string();
    }
}

/// POST /api/chat — start an agent turn and stream events back as SSE.
pub async fn chat(State(state): State<AppState>, Json(req): Json<ChatRequest>) -> Response {
    // Build the model client from the frontend-supplied config. Gateway
    // models resolve their base_url/api_key server-side from config.json's
    // model_endpoints map — credentials never live in the browser.
    let mut model_cfg = req.model;
    resolve_model_endpoint(&mut model_cfg);
    let model = match liteclaw_model::OpenAiClient::new(model_cfg) {
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
    let mut messages = inject_agents_md(req.messages, &ctx.cwd);

    // AUTO-RAG (opt-in, default off): retrieve fresh manual passages
    // server-side and compact the payload. When off, the model drives
    // retrieval itself via skill_list/skill_run — visible as tool cards in
    // the UI. LITECLAW_AUTO_RAG=0 still force-disables it for everyone.
    let env_on = std::env::var("LITECLAW_AUTO_RAG").map(|v| v != "0").unwrap_or(true);
    if req.auto_rag && env_on {
        if let Some(t) = tools.iter().find(|t| t.name == "skill_run") {
            inject_rag(&mut messages, t, &ctx).await;
            messages = compact_history(messages);
        }
    }

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

/// Serve a page from the mounted `web/` directory (frontend is not embedded
/// in the binary; backend and frontend are fully separated).
fn page(disk_path: &str, content_type: &str) -> Response {
    match std::fs::read_to_string(disk_path) {
        Ok(body) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, content_type)],
            body,
        )
            .into_response(),
        Err(_) => (
            StatusCode::NOT_FOUND,
            format!("frontend file missing: {disk_path} (serve the repo's web/ directory)"),
        )
            .into_response(),
    }
}

/// GET / — serve the single-page UI.
pub async fn index() -> Response {
    page("web/index.html", "text/html; charset=utf-8")
}

/// GET /help — RAG question map: what users can ask and how.
pub async fn help() -> Response {
    page("web/help.html", "text/html; charset=utf-8")
}

/// GET /style.css — UI styles.
pub async fn style_css() -> Response {
    page("web/style.css", "text/css; charset=utf-8")
}

/// GET /app.js — UI logic.
pub async fn app_js() -> Response {
    page("web/app.js", "application/javascript; charset=utf-8")
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
