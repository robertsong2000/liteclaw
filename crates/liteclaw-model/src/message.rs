//! Chat message model — mirrors the OpenAI chat-completions schema, including
//! tool calling. These types serialize straight into the request body and can
//! be sent back as-is on subsequent turns.

use serde::{Deserialize, Serialize};

/// Conversation role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool call emitted by the assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Stable id so the tool-result message can reference it.
    pub id: String,
    #[serde(default)]
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

/// The function name + JSON-string arguments of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// Arguments as a JSON-encoded string (per OpenAI convention).
    pub arguments: String,
}

/// One message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    /// Text content. Empty for assistant messages that are pure tool calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Present on assistant messages that request tool execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Present on role=tool messages: which tool call this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
    pub fn assistant_text(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: Some(content.into()), tool_calls: None, tool_call_id: None }
    }
    /// A tool-result message answering a prior tool call.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// A tool/function schema advertised to the model (OpenAI function-calling).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    #[serde(rename = "type", default = "default_type")]
    pub spec_type: String,
    pub function: ToolFunction,
}

fn default_type() -> String {
    "function".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    /// JSON Schema for the function parameters.
    pub parameters: serde_json::Value,
}
