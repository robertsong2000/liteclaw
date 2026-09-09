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
    /// Extra JSON fields merged into the chat-completions request body —
    /// gateway knobs local Ollama ignores (e.g. `{"enable_thinking": false}`
    /// for Qwen flash models behind new-api). Absent = provider defaults.
    #[serde(default)]
    pub extra_body: Option<serde_json::Map<String, serde_json::Value>>,
    /// Disable thinking for models that support it. Sent as
    /// `reasoning_effort: "none"` in the chat-completions body — Ollama maps
    /// that to `think: false` for every thinking-capable local model, and
    /// OpenAI-compatible gateways understand the same field. The frontend
    /// only sends it for Ollama models; gateway models use `extra_body`
    /// (`enable_thinking`) instead.
    #[serde(default)]
    pub no_think: bool,
    /// Talk to Ollama's native `/api/chat` protocol instead of the
    /// OpenAI-compatible `/v1` shim. The native protocol carries per-request
    /// knobs the shim cannot express (`options.num_ctx`, `think`), so local
    /// Ollama models set it; gateway models stay on `/v1`.
    #[serde(default)]
    pub native: bool,
    /// Requested context window for native-Ollama requests
    /// (`options.num_ctx`), overriding whatever the model tag baked in —
    /// lets stock official models run at full context without custom
    /// Modelfile tags. None = leave the model default.
    #[serde(default)]
    pub num_ctx: Option<u64>,
}

fn default_model() -> String {
    "qwen2.5:7b".to_string()
}

fn default_base_url() -> String {
    "http://172.21.0.1:11434/v1".to_string()
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            base_url: default_base_url(),
            api_key: String::new(),
            model: "qwen2.5:7b".to_string(),
            extra_body: None,
            no_think: false,
            native: false,
            num_ctx: None,
        }
    }
}

impl ModelConfig {
    /// Full chat-completions URL.
    pub fn chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/chat/completions")
    }

    /// Native Ollama chat endpoint, derived from the OpenAI-style `/v1` base
    /// URL (`http://host:11434/v1` → `http://host:11434/api/chat`).
    pub fn ollama_chat_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        format!("{base}/api/chat")
    }
}
