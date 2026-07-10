//! liteclaw-web: axum HTTP server serving the chat UI and the SSE agent
//! endpoint. The whole UI is a single HTML file embedded into the binary.

pub mod handlers;

use anyhow::Result;
use liteclaw_core::{Claw, Ctx};
use std::sync::Arc;

/// Shared server state: the registered claws + the execution context.
#[derive(Clone)]
pub struct AppState {
    pub claws: Vec<Arc<dyn Claw>>,
    pub ctx: Arc<Ctx>,
}

/// Run the web server on the given port. Blocks until shutdown.
pub async fn serve(port: u16, claws: Vec<Arc<dyn Claw>>, ctx: Ctx) -> Result<()> {
    let state = AppState {
        claws,
        ctx: Arc::new(ctx),
    };

    let app = axum::Router::new()
        .route("/", axum::routing::get(handlers::index))
        .route("/api/chat", axum::routing::post(handlers::chat))
        .route("/api/config", axum::routing::get(handlers::get_config))
        .route("/api/config", axum::routing::post(handlers::post_config))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    eprintln!("liteclaw web UI → http://localhost:{port}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind {addr} failed: {e}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("server error: {e}"))?;
    Ok(())
}
