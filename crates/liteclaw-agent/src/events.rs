//! Events emitted by the agent loop. These are serialized to SSE and streamed
//! to the frontend, which renders them as chat bubbles / tool cards.

use serde::{Deserialize, Serialize};

/// A single event during an agent turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A chunk of assistant text (streamed token-by-token).
    TextDelta { text: String },
    /// The model is invoking a tool.
    ToolStart {
        tool: String,
        arguments: serde_json::Value,
        /// Whether this tool requires human confirmation before executing.
        needs_confirmation: bool,
        /// When needs_confirmation is true, an id the frontend must POST back to
        /// /api/confirm to allow or deny this specific call.
        #[serde(skip_serializing_if = "Option::is_none")]
        confirm_id: Option<String>,
    },
    /// A tool finished and produced a result.
    ToolResult {
        tool: String,
        ok: bool,
        /// Short human-readable summary of the output.
        summary: String,
    },
    /// The agent turn is complete.
    Done,
    /// An error terminated the turn.
    Error { message: String },
}

impl AgentEvent {
    pub fn text_delta(s: impl Into<String>) -> Self {
        AgentEvent::TextDelta { text: s.into() }
    }
    pub fn error(s: impl Into<String>) -> Self {
        AgentEvent::Error { message: s.into() }
    }
}
