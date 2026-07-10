//! liteclaw-model: streaming OpenAI-compatible model client.
//!
//! Connects to any OpenAI-compatible chat-completions endpoint (cloud API or
//! local Ollama `/v1`). Streams text deltas and accumulates tool calls across
//! fragmented chunks.

pub mod config;
pub mod message;
pub mod openai;

pub use config::ModelConfig;
pub use message::{FunctionCall, Message, Role, ToolCall, ToolFunction, ToolSpec};
pub use openai::{OpenAiClient, StreamEvent};
