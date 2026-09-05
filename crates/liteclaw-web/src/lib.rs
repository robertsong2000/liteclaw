//! liteclaw-web: axum HTTP server serving the chat UI and the SSE agent
//! endpoint. The whole UI is a single HTML file embedded into the binary.

pub mod auth;
pub mod confirm;
pub mod handlers;

use anyhow::Result;
use liteclaw_agent::ConfirmFn;
use liteclaw_core::{Claw, Ctx};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Shared server state: the registered claws + the execution context + the
/// confirmation registry + login sessions.
#[derive(Clone)]
pub struct AppState {
    pub claws: Vec<Arc<dyn Claw>>,
    pub ctx: Arc<Ctx>,
    pub confirms: confirm::ConfirmRegistry,
    pub sessions: auth::Sessions,
}

/// Build a confirm callback that bridges to the frontend via the registry.
pub fn make_confirm(reg: confirm::ConfirmRegistry) -> ConfirmFn {
    Arc::new(
        move |_tool: String,
              _args: serde_json::Value,
              id: String|
              -> Pin<Box<dyn Future<Output = bool> + Send>> {
            let reg = reg.clone();
            Box::pin(async move {
                let rx = reg.register(&id);
                rx.await.unwrap_or_default()
            })
        },
    )
}

/// Run the web server. `host` is the bind address: "127.0.0.1" for local-only
/// (default), "0.0.0.0" for container/public access. Blocks until shutdown.
pub async fn serve(host: &str, port: u16, claws: Vec<Arc<dyn Claw>>, ctx: Ctx) -> Result<()> {
    let state = AppState {
        claws,
        ctx: Arc::new(ctx),
        confirms: confirm::ConfirmRegistry::new(),
        sessions: auth::Sessions::new(),
    };

    // Public routes: the page itself + login. Everything under /api/* (except
    // /api/login) requires a valid session token via the auth middleware.
    let api = axum::Router::new()
        .route("/chat", axum::routing::post(handlers::chat))
        .route("/config", axum::routing::get(handlers::get_config))
        .route("/config", axum::routing::post(handlers::post_config))
        .route("/confirm", axum::routing::post(handlers::confirm))
        .route("/history", axum::routing::get(handlers::list_history))
        .route("/history", axum::routing::post(handlers::save_session))
        .route("/history/:id", axum::routing::get(handlers::get_session))
        .route("/history/:id", axum::routing::delete(handlers::delete_session))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    let app = axum::Router::new()
        .route("/", axum::routing::get(handlers::index))
        .route("/help", axum::routing::get(handlers::help))
        .route("/api/login", axum::routing::post(auth::login))
        .nest("/api", api)
        .with_state(state);

    let ip: std::net::IpAddr = host
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid host '{host}': {e}"))?;
    let addr = std::net::SocketAddr::from((ip, port));
    eprintln!("liteclaw web UI → http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind {addr} failed: {e}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("server error: {e}"))?;
    Ok(())
}
