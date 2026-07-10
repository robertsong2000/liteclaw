//! Session management + auth middleware.
//!
//! Simple token-based auth: POST /api/login validates credentials and returns
//! a session token; protected routes check the `Authorization: Bearer <token>`
//! header. No cookies, no axum-extra, no new deps.

use crate::AppState;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

/// Hardcoded credentials (per user request). A future milestone can move these
/// to ~/.liteclaw/config.json.
pub const USERNAME: &str = "renault";
pub const PASSWORD: &str = "renault123";

/// Session expiry: 24 hours.
const SESSION_TTL_SECS: u64 = 86400;

/// Shared session store: token → creation time.
#[derive(Clone, Default)]
pub struct Sessions {
    inner: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new token.
    pub fn insert(&self, token: String) {
        self.inner.lock().unwrap().insert(token, Instant::now());
    }

    /// Validate a token; removes expired ones opportunistically.
    pub fn valid(&self, token: &str) -> bool {
        let mut map = self.inner.lock().unwrap();
        // Prune expired sessions.
        map.retain(|_, t| t.elapsed().as_secs() < SESSION_TTL_SECS);
        map.contains_key(token)
    }
}

/// Generate a pseudo-random token from time + pid + counter (no rand dep).
fn gen_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}{n:x}{}", std::process::id())
}

/// Login request body.
#[derive(serde::Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// POST /api/login — validate credentials, return a session token.
pub async fn login(State(state): State<AppState>, Json(req): Json<LoginRequest>) -> Response {
    if req.username == USERNAME && req.password == PASSWORD {
        let token = gen_token();
        state.sessions.insert(token.clone());
        Json(serde_json::json!({ "token": token })).into_response()
    } else {
        (StatusCode::UNAUTHORIZED, "invalid credentials").into_response()
    }
}

/// Auth middleware: check Authorization header against the session store.
/// Applied to all protected /api/* routes.
pub async fn require_auth(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let auth = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    match auth {
        Some(token) if state.sessions.valid(token) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    }
}
