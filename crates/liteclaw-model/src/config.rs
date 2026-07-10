//! Model connection configuration. One config selects the backend — works for
//! any OpenAI-compatible endpoint (cloud API or local Ollama `/v1`).

use serde::{Deserialize, Serialize};

/// Where and how to reach the model. Sent from the frontend on each chat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Base URL of an OpenAI-compatible API, e.g.
    /// `http://localhost:11434/v1` (Ollama) or `https://api.openai.com/v1`.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// Bearer API key. May be empty for local Ollama.
    #[serde(default)]
    pub api_key: String,
    /// Model id, e.g. `qwen2.5:7b` (Ollama) or `gpt-4o-mini`.
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_model() -> String {
    "qwen2.5:7b".to_string()
}

fn default_base_url() -> String {
    "http://localhost:11434/v1".to_string()
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            api_key: String::new(),
            model: "qwen2.5:7b".to_string(),
        }
    }
}

impl ModelConfig {
    /// Full chat-completions URL.
    pub fn chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/chat/completions")
    }
}
