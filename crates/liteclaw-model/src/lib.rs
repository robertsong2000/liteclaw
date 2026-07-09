//! liteclaw-model: model client skeleton.
//!
//! The trait-only surface lets the CLI registry and a future agent loop be
//! wired against a stable abstraction now. The concrete OpenAI-compatible
//! streaming client (reqwest + rustls, no openssl) and `lc chat` land in a
//! later milestone.

use async_trait::async_trait;

/// A single message in a chat conversation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// Conversation role.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// A model backend. Implementations: OpenAI-compatible API, local Ollama.
///
/// TODO(next milestone): implement with `reqwest` (rustls) + SSE streaming.
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// Model identifier (e.g. "gpt-4o-mini", "qwen2.5:7b").
    fn model(&self) -> &str;

    /// Send a conversation and return the assistant reply.
    ///
    /// Streaming variants (`stream`) will be added when the concrete client
    /// lands; for now a single completion shape keeps the trait stable.
    async fn complete(&self, messages: &[Message]) -> anyhow::Result<String>;
}

/// A stub client used until the real backend is implemented. Always errors so
/// callers fail loudly rather than silently no-op'ing.
pub struct UnimplementedClient;

#[async_trait]
impl ModelClient for UnimplementedClient {
    fn model(&self) -> &str {
        "unimplemented"
    }

    async fn complete(&self, _messages: &[Message]) -> anyhow::Result<String> {
        anyhow::bail!(
            "model client not implemented yet — lc chat lands in the next milestone"
        )
    }
}
