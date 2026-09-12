//! Streaming chat client for both backend protocols.
//!
//! - OpenAI-compatible `/v1/chat/completions` (cloud, gateways) — SSE in,
//!   parsed into [`StreamEvent`]s.
//! - Ollama's native `/api/chat` (local models, [`ModelConfig::native`]) —
//!   NDJSON in, parsed into the same [`StreamEvent`]s. The native protocol
//!   carries per-request knobs the `/v1` shim cannot express:
//!   `options.num_ctx` (context window) and `think`.
//!
//! The non-trivial bits:
//! - SSE line parsing (`data: {...}\n\n`, terminated by `data: [DONE]`).
//! - Tool-call deltas arrive fragmented across chunks, indexed by
//!   `tool_calls[i].index`; we accumulate them into complete calls.
//! - Reasoning models (qwen3/minicpm5 on local Ollama, Qwen flash behind
//!   new-api) stream their thinking in a separate field (`reasoning_content`
//!   on /v1, `message.thinking` native). We wrap it in inline
//!   `<think>...</think>` tags so downstream consumers see one shape (the
//!   chat UI renders those as a collapsible "思考过程" block). `no_think`
//!   suppresses it upstream (`reasoning_effort: "none"` on /v1, `think:
//!   false` native).

use std::collections::VecDeque;
use std::pin::Pin;

use crate::config::ModelConfig;
use crate::message::{Message, Role, ToolCall, ToolSpec};
use anyhow::{anyhow, Context, Result};
use futures::Stream;
use futures::StreamExt;
use serde::Deserialize;

/// What the model emits during a streamed completion.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A chunk of assistant text.
    Delta(String),
    /// The model finished. If it ended with tool calls, they're here.
    /// `usage` carries the provider-reported real token counts when the
    /// endpoint supports `stream_options.include_usage` (Ollama does).
    Done {
        tool_calls: Vec<ToolCall>,
        usage: Option<Usage>,
    },
}

/// Real token usage reported by the provider on the final stream chunk.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
}

/// A streaming chat completion client.
pub struct OpenAiClient {
    cfg: ModelConfig,
    http: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(cfg: ModelConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| anyhow!("failed to build http client: {e}"))?;
        Ok(Self { cfg, http })
    }

    /// Begin a streaming completion. `tools` may be empty to disable tool use.
    ///
    /// The returned stream yields [`StreamEvent`]s. The caller drives it to
    /// completion, accumulating text deltas and reading tool calls from the
    /// terminal `Done` event.
    ///
    /// Dispatches on [`ModelConfig::native`]: Ollama's native `/api/chat`
    /// for local models (per-request `num_ctx`/`think`), the
    /// OpenAI-compatible `/v1` protocol for everything else.
    pub async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolSpec],
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
        if self.cfg.native {
            Ok(Box::pin(self.ollama_chat_stream(messages, tools).await?))
        } else {
            Ok(Box::pin(self.openai_chat_stream(messages, tools).await?))
        }
    }

    async fn openai_chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolSpec],
    ) -> Result<impl Stream<Item = Result<StreamEvent>> + Send> {
        let body = self.build_body(messages, tools);

        let mut req = self.http
            .post(self.cfg.chat_url())
            .header("connection", "close")  // SSE 流式响应复用 keep-alive 连接时,上游收尾不干净会让下一个请求秒回空
            .json(&body);
        if !self.cfg.api_key.is_empty() {
            req = req.bearer_auth(&self.cfg.api_key);
        }

        let resp = req
            .send()
            .await
            .context("request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("model API error {status}: {text}"));
        }

        // Convert the response byte stream into a stream of parsed SSE events.
        let event_stream = SseDecoder::new(resp.bytes_stream());
        Ok(event_stream)
    }

    /// Ollama native streaming chat (`/api/chat`, NDJSON) — the protocol
    /// used for local models because it carries per-request knobs the /v1
    /// shim cannot express: `options.num_ctx` overrides whatever context the
    /// model tag baked in, and `think: false` disables thinking (both
    /// verified: request options beat Modelfile parameters). Messages need
    /// remapping ([`to_native_messages`]); tool specs share the OpenAI JSON
    /// shape; the response decodes via [`OllamaDecoder`].
    async fn ollama_chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolSpec],
    ) -> Result<impl Stream<Item = Result<StreamEvent>> + Send> {
        let body = self.build_native_body(messages, tools);

        let req = self.http
            .post(self.cfg.ollama_chat_url())
            .header("connection", "close")
            .json(&body);
        let resp = req
            .send()
            .await
            .context("request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("model API error {status}: {text}"));
        }
        Ok(OllamaDecoder::new(resp.bytes_stream()))
    }

    /// Assemble the native `/api/chat` request body: remapped messages,
    /// tool specs, `think: false` when thinking is disabled
    /// ([`ModelConfig::no_think`]), and `options.num_ctx` when a context
    /// window is requested ([`ModelConfig::num_ctx`]).
    fn build_native_body(&self, messages: &[Message], tools: &[ToolSpec]) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "messages": to_native_messages(messages),
            "stream": true,
        });
        if self.cfg.no_think {
            body["think"] = serde_json::json!(false);
        }
        if let Some(num_ctx) = self.cfg.num_ctx {
            body["options"] = serde_json::json!({ "num_ctx": num_ctx });
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }
        body
    }

    /// Assemble the chat-completions request body: model + messages + stream
    /// options, tool specs, `reasoning_effort` when thinking is disabled
    /// ([`ModelConfig::no_think`]), then any gateway-specific `extra_body`
    /// fields (e.g. `enable_thinking`) layered on top — an explicit
    /// `extra_body` knob always wins over the `no_think` default.
    fn build_body(&self, messages: &[Message], tools: &[ToolSpec]) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }
        if self.cfg.no_think {
            body["reasoning_effort"] = serde_json::json!("none");
        }
        if let Some(extra) = &self.cfg.extra_body {
            for (k, v) in extra {
                body[k.as_str()] = v.clone();
            }
        }
        body
    }
}

/// Map OpenAI-style messages onto Ollama's native chat format: multimodal
/// parts collapse to text plus base64 `images`, assistant tool-call
/// arguments become a JSON object, tool results keep only their content.
fn to_native_messages(messages: &[Message]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            let mut msg = serde_json::json!({ "role": m.role });
            match m.role {
                Role::User => {
                    msg["content"] = serde_json::Value::String(text_of(&m.content));
                    let images = images_of(&m.content);
                    if !images.is_empty() {
                        msg["images"] = serde_json::Value::Array(
                            images.into_iter().map(serde_json::Value::String).collect(),
                        );
                    }
                }
                Role::Assistant => {
                    if let Some(c) = &m.content {
                        msg["content"] = c.clone();
                    }
                    if let Some(calls) = &m.tool_calls {
                        let native: Vec<serde_json::Value> = calls
                            .iter()
                            .map(|c| {
                                let args: serde_json::Value = serde_json::from_str(
                                    &c.function.arguments,
                                )
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                                serde_json::json!({
                                    "function": {
                                        "name": c.function.name,
                                        "arguments": args,
                                    }
                                })
                            })
                            .collect();
                        msg["tool_calls"] = serde_json::Value::Array(native);
                    }
                }
                _ => {
                    msg["content"] = serde_json::Value::String(text_of(&m.content));
                }
            }
            msg
        })
        .collect()
}

/// Plain text of a content value (string or multimodal parts).
fn text_of(content: &Option<serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Base64 payloads of multimodal image parts (data-URL prefix stripped).
fn images_of(content: &Option<serde_json::Value>) -> Vec<String> {
    match content {
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                p.get("image_url")
                    .and_then(|i| i.get("url"))
                    .and_then(|u| u.as_str())
            })
            .filter_map(|u| u.split("base64,").nth(1))
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

/// Decode a byte stream of SSE-formatted data into [`StreamEvent`]s.
///
/// Buffers bytes, splits on `\n\n` to get SSE frames, parses `data:` lines as
/// OpenAI streaming chunks. Accumulates tool-call deltas by index.
struct SseDecoder<S> {
    inner: S,
    buf: String,
    /// Accumulated tool calls indexed by their delta `index`.
    tool_calls: Vec<ToolCallAccum>,
    /// Provider-reported usage from the final chunk, if it sent one.
    usage: Option<Usage>,
    /// Queued events awaiting delivery. A single SSE frame can carry both
    /// reasoning and content deltas; queueing keeps them in order.
    pending: VecDeque<StreamEvent>,
    /// True while streaming inside a wrapped `<think>` block.
    in_reasoning: bool,
    /// Set once we've emitted a terminal Done (saw [DONE] or upstream closed).
    /// All subsequent polls return None so the consumer's while-let exits even
    /// if the underlying HTTP keep-alive connection stays open.
    finished: bool,
}

#[derive(Default, Clone)]
struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccum {
    /// Convert fragmented-delta accumulators into complete OpenAI tool calls.
    fn finish_all(v: Vec<ToolCallAccum>) -> Vec<ToolCall> {
        v.into_iter()
            .map(|a| ToolCall {
                id: a.id,
                call_type: "function".into(),
                function: crate::message::FunctionCall {
                    name: a.name,
                    arguments: a.arguments,
                },
            })
            .collect()
    }
}

impl<S> SseDecoder<S>
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send,
{
    fn new(inner: S) -> Self {
        Self {
            inner,
            buf: String::new(),
            tool_calls: Vec::new(),
            usage: None,
            pending: VecDeque::new(),
            in_reasoning: false,
            finished: false,
        }
    }
}

impl<S> Stream for SseDecoder<S>
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send,
{
    type Item = Result<StreamEvent>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            // First, deliver queued events — a frame can queue several
            // (think-tag wraps + reasoning + content) and order matters.
            if let Some(ev) = this.pending.pop_front() {
                if matches!(ev, StreamEvent::Done { .. }) {
                    this.finished = true;
                }
                return std::task::Poll::Ready(Some(Ok(ev)));
            }
            if this.finished {
                return std::task::Poll::Ready(None);
            }
            // Then, try to pull a complete SSE frame from the buffer.
            if let Some(idx) = this.buf.find("\n\n") {
                let frame = this.buf.drain(..idx).collect::<String>();
                // consume the delimiter
                this.buf.drain(..2);
                match handle_frame(
                    &frame,
                    &mut this.tool_calls,
                    &mut this.usage,
                    &mut this.pending,
                    &mut this.in_reasoning,
                ) {
                    // Both outcomes queue events (if any); drained next pass.
                    FrameOutcome::Ignore => continue,
                    FrameOutcome::Done => continue,
                }
            }

            // Otherwise, pull more bytes from the upstream.
            match this.inner.poll_next_unpin(cx) {
                std::task::Poll::Ready(Some(Ok(chunk))) => {
                    this.buf.push_str(std::str::from_utf8(&chunk).unwrap_or(""));
                    continue;
                }
                std::task::Poll::Ready(Some(Err(e))) => {
                    return std::task::Poll::Ready(Some(Err(anyhow!("stream error: {e}"))));
                }
                std::task::Poll::Ready(None) => {
                    // Upstream ended. Queue a terminal Done (after closing any
                    // open <think> block) so the agent loop's while-let exits
                    // cleanly (it cannot distinguish a clean close from a
                    // missing [DONE] otherwise). Any buffered frame is
                    // dropped — partial trailing data is not useful.
                    queue_done(
                        ToolCallAccum::finish_all(std::mem::take(&mut this.tool_calls)),
                        &mut this.usage,
                        &mut this.in_reasoning,
                        &mut this.pending,
                    );
                    this.finished = true;
                    continue;
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

/// Close any open `<think>` block and queue the terminal Done event.
fn queue_done(
    calls: Vec<ToolCall>,
    usage_out: &mut Option<Usage>,
    in_reasoning: &mut bool,
    pending: &mut VecDeque<StreamEvent>,
) {
    if *in_reasoning {
        pending.push_back(StreamEvent::Delta("</think>".into()));
        *in_reasoning = false;
    }
    pending.push_back(StreamEvent::Done {
        tool_calls: calls,
        usage: usage_out.take(),
    });
}

enum FrameOutcome {
    /// Terminal Done event was queued.
    Done,
    /// Frame consumed; any deltas it carried were queued.
    Ignore,
}

fn handle_frame(
    frame: &str,
    tool_calls: &mut Vec<ToolCallAccum>,
    usage_out: &mut Option<Usage>,
    pending: &mut VecDeque<StreamEvent>,
    in_reasoning: &mut bool,
) -> FrameOutcome {
    // An SSE frame is one or more `data:` lines.
    let mut data_lines = Vec::new();
    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim().to_string());
        }
    }
    if data_lines.is_empty() {
        return FrameOutcome::Ignore;
    }
    let data = data_lines.join("\n");
    if data == "[DONE]" {
        queue_done(
            ToolCallAccum::finish_all(std::mem::take(tool_calls)),
            usage_out,
            in_reasoning,
            pending,
        );
        return FrameOutcome::Done;
    }

    #[derive(Deserialize)]
    struct Chunk {
        choices: Vec<Choice>,
        /// Final chunk (Ollama with include_usage) carries real token counts
        /// and an empty `choices` array.
        #[serde(default)]
        usage: Option<Usage>,
    }
    #[derive(Deserialize)]
    struct Choice {
        delta: Delta,
    }
    #[derive(Deserialize, Default)]
    struct Delta {
        #[serde(default)]
        content: Option<String>,
        /// Reasoning stream on gateway models (qwen3.8-flash etc.). Some
        /// providers use the shorter `reasoning` instead — accept both.
        #[serde(default)]
        reasoning_content: Option<String>,
        #[serde(default)]
        reasoning: Option<String>,
        #[serde(default)]
        tool_calls: Vec<DeltaToolCall>,
    }
    #[derive(Deserialize)]
    struct DeltaToolCall {
        #[serde(default)]
        index: usize,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        function: Option<DeltaFunction>,
    }
    #[derive(Deserialize, Default)]
    struct DeltaFunction {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        arguments: Option<String>,
    }

    let chunk: Chunk = match serde_json::from_str(&data) {
        Ok(c) => c,
        Err(_) => return FrameOutcome::Ignore, // skip keepalives / partials
    };
    if let Some(u) = chunk.usage {
        *usage_out = Some(u);
    }
    let Some(choice) = chunk.choices.into_iter().next() else {
        return FrameOutcome::Ignore;
    };

    // Accumulate tool-call deltas by index.
    for dtc in choice.delta.tool_calls {
        while tool_calls.len() <= dtc.index {
            tool_calls.push(ToolCallAccum::default());
        }
        let accum = &mut tool_calls[dtc.index];
        if let Some(id) = dtc.id {
            accum.id = id;
        }
        if let Some(f) = dtc.function {
            if let Some(name) = f.name {
                accum.name = name;
            }
            if let Some(args) = f.arguments {
                accum.arguments.push_str(&args);
            }
        }
    }

    // Reasoning deltas wrap in <think> tags so the whole downstream pipeline
    // (agent passthrough → UI think filter) treats them exactly like the
    // inline think blocks local Ollama thinking models emit.
    if let Some(r) = choice.delta.reasoning_content.or(choice.delta.reasoning) {
        if !r.is_empty() {
            if !*in_reasoning {
                pending.push_back(StreamEvent::Delta("<think>".into()));
                *in_reasoning = true;
            }
            pending.push_back(StreamEvent::Delta(r));
        }
    }

    if let Some(text) = choice.delta.content {
        if !text.is_empty() {
            if *in_reasoning {
                pending.push_back(StreamEvent::Delta("</think>".into()));
                *in_reasoning = false;
            }
            pending.push_back(StreamEvent::Delta(text));
        }
    }
    FrameOutcome::Ignore
}

/// Decode Ollama's native NDJSON chat stream into [`StreamEvent`]s.
///
/// Each line is one JSON object:
/// `{"message":{"thinking"/"content"/"tool_calls"},"done":bool}`; the final
/// line carries `done: true` plus `prompt_eval_count`/`eval_count`. Tool
/// calls arrive complete (no index fragmentation). `thinking` deltas are
/// wrapped in `<think>` tags, exactly like gateway `reasoning_content`.
struct OllamaDecoder<S> {
    inner: S,
    buf: String,
    tool_calls: Vec<ToolCall>,
    in_reasoning: bool,
    usage: Option<Usage>,
    pending: VecDeque<StreamEvent>,
    finished: bool,
}

#[derive(Deserialize)]
struct OllamaChunk {
    #[serde(default)]
    message: Option<OllamaMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
}

#[derive(Deserialize)]
struct OllamaMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Deserialize)]
struct OllamaToolCall {
    function: OllamaFunction,
}

#[derive(Deserialize)]
struct OllamaFunction {
    name: String,
    /// Native arguments are a JSON *object* (OpenAI uses a JSON string).
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

impl<S> OllamaDecoder<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            buf: String::new(),
            tool_calls: Vec::new(),
            in_reasoning: false,
            usage: None,
            pending: VecDeque::new(),
            finished: false,
        }
    }
}

impl<S> Stream for OllamaDecoder<S>
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send,
{
    type Item = Result<StreamEvent>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            // Deliver queued events first — order matters (think wrap, text,
            // terminal Done).
            if let Some(ev) = this.pending.pop_front() {
                if matches!(ev, StreamEvent::Done { .. }) {
                    this.finished = true;
                }
                return std::task::Poll::Ready(Some(Ok(ev)));
            }
            if this.finished {
                return std::task::Poll::Ready(None);
            }
            // NDJSON frames are single newline-separated JSON objects.
            if let Some(idx) = this.buf.find('\n') {
                let line = this.buf.drain(..idx).collect::<String>();
                this.buf.drain(..1);
                if handle_ollama_line(
                    &line,
                    &mut this.tool_calls,
                    &mut this.usage,
                    &mut this.pending,
                    &mut this.in_reasoning,
                ) {
                    queue_done(
                        std::mem::take(&mut this.tool_calls),
                        &mut this.usage,
                        &mut this.in_reasoning,
                        &mut this.pending,
                    );
                }
                continue;
            }
            match this.inner.poll_next_unpin(cx) {
                std::task::Poll::Ready(Some(Ok(chunk))) => {
                    this.buf.push_str(std::str::from_utf8(&chunk).unwrap_or(""));
                    continue;
                }
                std::task::Poll::Ready(Some(Err(e))) => {
                    return std::task::Poll::Ready(Some(Err(anyhow!("stream error: {e}"))));
                }
                std::task::Poll::Ready(None) => {
                    // Upstream ended: close any open <think> block and emit
                    // the terminal Done so the agent loop exits cleanly.
                    queue_done(
                        std::mem::take(&mut this.tool_calls),
                        &mut this.usage,
                        &mut this.in_reasoning,
                        &mut this.pending,
                    );
                    this.finished = true;
                    continue;
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

/// Handle one NDJSON line; returns true when it was the terminal `done` line
/// (the caller then queues the Done event).
fn handle_ollama_line(
    line: &str,
    tool_calls: &mut Vec<ToolCall>,
    usage_out: &mut Option<Usage>,
    pending: &mut VecDeque<StreamEvent>,
    in_reasoning: &mut bool,
) -> bool {
    let Ok(chunk) = serde_json::from_str::<OllamaChunk>(line) else {
        return false; // skip keepalives / partial lines
    };
    // The terminal done line can ALSO carry the final message — including
    // the model's tool call. Process message fields before the done check,
    // otherwise the tool call is silently dropped and the agent sees none.
    if let Some(msg) = chunk.message {
        if let Some(t) = msg.thinking {
            if !t.is_empty() {
                if !*in_reasoning {
                    pending.push_back(StreamEvent::Delta("<think>".into()));
                    *in_reasoning = true;
                }
                pending.push_back(StreamEvent::Delta(t));
            }
        }
        if let Some(c) = msg.content {
            if !c.is_empty() {
                if *in_reasoning {
                    pending.push_back(StreamEvent::Delta("</think>".into()));
                    *in_reasoning = false;
                }
                pending.push_back(StreamEvent::Delta(c));
            }
        }
        if let Some(calls) = msg.tool_calls {
            for tc in calls {
                let args = serde_json::to_string(&tc.function.arguments.clone().unwrap_or_default())
                    .unwrap_or_else(|_| "{}".into());
                tool_calls.push(ToolCall {
                    id: format!("call_{}", tool_calls.len()),
                    call_type: "function".into(),
                    function: crate::message::FunctionCall {
                        name: tc.function.name,
                        arguments: args,
                    },
                });
            }
        }
    }
    if chunk.done {
        if chunk.prompt_eval_count.is_some() || chunk.eval_count.is_some() {
            *usage_out = Some(Usage {
                prompt_tokens: chunk.prompt_eval_count.unwrap_or(0),
                completion_tokens: chunk.eval_count.unwrap_or(0),
            });
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_errors_preserve_transport_cause_in_both_protocols() {
        for native in [false, true] {
            let client = OpenAiClient::new(ModelConfig {
                base_url: "unsupported://model".into(), native, ..Default::default()
            }).unwrap();
            let error = match client.chat_stream(&[], &[]).await {
                Ok(_) => panic!("invalid scheme must fail"),
                Err(error) => error,
            };
            assert!(error.chain().any(|cause| cause.downcast_ref::<reqwest::Error>().is_some()),
                "transport error was reduced to a string: {error:#}");
            assert!(error.chain().count() >= 2);
        }
    }

    /// Build a fake upstream that yields the given SSE frames as bytes.
    fn fake_stream(
        frames: Vec<String>,
    ) -> impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send {
        use futures::stream;
        stream::iter(frames.into_iter().map(|f| Ok(bytes::Bytes::from(f))))
    }

    #[tokio::test]
    async fn decodes_text_deltas() {
        let s = SseDecoder::new(fake_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n".to_string(),
            "data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ]));
        let events: Vec<_> = s.collect::<Vec<_>>().await;
        let deltas: String = events
            .iter()
            .filter_map(|e| match e {
                Ok(StreamEvent::Delta(d)) => Some(d.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, "Hello world");
        assert!(events
            .iter()
            .any(|e| matches!(e, Ok(StreamEvent::Done { .. }))));
    }

    #[tokio::test]
    async fn accumulates_fragmented_tool_calls() {
        // Tool-call arguments arrive fragmented across chunks, same index.
        let s = SseDecoder::new(fake_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\n".to_string(),
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":\\\"Cargo.toml\\\"}\"}}]}}]}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ]));
        let events: Vec<_> = s.collect::<Vec<_>>().await;
        let done = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::Done { tool_calls, .. }) => Some(tool_calls.clone()),
                _ => None,
            })
            .expect("a Done event");
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].function.name, "read");
        assert_eq!(done[0].function.arguments, r#"{"path":"Cargo.toml"}"#);
    }

    #[tokio::test]
    async fn captures_usage_from_final_chunk() {
        // Ollama with include_usage sends a final chunk with empty choices +
        // real token counts, then [DONE].
        let s = SseDecoder::new(fake_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n".to_string(),
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":14,\"completion_tokens\":19}}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ]));
        let events: Vec<_> = s.collect::<Vec<_>>().await;
        let done = events
            .iter()
            .find_map(|e| match e {
                Ok(StreamEvent::Done { usage, .. }) => *usage,
                _ => None,
            })
            .expect("a Done event with usage");
        assert_eq!(done.prompt_tokens, 14);
        assert_eq!(done.completion_tokens, 19);
    }

    #[tokio::test]
    async fn ignores_keepalive_comments() {
        // Some servers send `: keepalive` comments — must be ignored.
        let s = SseDecoder::new(fake_stream(vec![
            ": keepalive\n\n".to_string(),
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ]));
        let events: Vec<_> = s.collect::<Vec<_>>().await;
        let deltas: String = events
            .iter()
            .filter_map(|e| match e {
                Ok(StreamEvent::Delta(d)) => Some(d.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, "ok");
    }

    #[tokio::test]
    async fn wraps_reasoning_content_in_think_tags() {
        // Gateway reasoning models (qwen3.8-flash via new-api) stream thinking
        // in a separate field; downstream must see inline <think> blocks.
        let s = SseDecoder::new(fake_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想一下\"}}]}\n\n".to_string(),
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"，答案是\"}}]}\n\n".to_string(),
            "data: {\"choices\":[{\"delta\":{\"content\":\"2\"}}]}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ]));
        let (text, done) = collect_text_and_done(s).await;
        assert_eq!(text, "<think>想一下，答案是</think>2");
        assert!(done);
    }

    #[tokio::test]
    async fn closes_unclosed_think_on_stream_end() {
        // Reasoning still open when upstream closes without [DONE]: the close
        // tag must be synthesized before Done so think blocks stay balanced.
        let s = SseDecoder::new(fake_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"思考中\"}}]}\n\n".to_string(),
        ]));
        let (text, done) = collect_text_and_done(s).await;
        assert_eq!(text, "<think>思考中</think>");
        assert!(done);
    }

    #[tokio::test]
    async fn plain_content_produces_no_think_tags() {
        // Endpoints without reasoning (local Ollama) must be untouched.
        let s = SseDecoder::new(fake_stream(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n".to_string(),
            "data: [DONE]\n\n".to_string(),
        ]));
        let (text, done) = collect_text_and_done(s).await;
        assert_eq!(text, "你好");
        assert!(done);
    }

    #[test]
    fn extra_body_merges_into_request() {
        // Gateway knobs like enable_thinking must land in the body without
        // disturbing the standard fields.
        let cfg = ModelConfig {
            extra_body: Some(
                serde_json::json!({ "enable_thinking": false })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
            ..Default::default()
        };
        let client = OpenAiClient::new(cfg).unwrap();
        let body = client.build_body(&[], &[]);
        assert_eq!(body["enable_thinking"], serde_json::json!(false));
        assert_eq!(body["stream"], serde_json::json!(true));
        assert_eq!(body["stream_options"]["include_usage"], serde_json::json!(true));
    }

    #[test]
    fn no_extra_body_keeps_defaults() {
        let client = OpenAiClient::new(ModelConfig::default()).unwrap();
        let body = client.build_body(&[], &[]);
        assert!(body.get("enable_thinking").is_none());
        assert_eq!(body["stream"], serde_json::json!(true));
    }

    #[test]
    fn no_think_sets_reasoning_effort_none() {
        // no_think must translate into the /v1 knob Ollama understands
        // (mapped to think:false for every thinking-capable model).
        let cfg = ModelConfig {
            no_think: true,
            ..Default::default()
        };
        let client = OpenAiClient::new(cfg).unwrap();
        let body = client.build_body(&[], &[]);
        assert_eq!(body["reasoning_effort"], serde_json::json!("none"));
    }

    #[test]
    fn native_body_carries_think_and_num_ctx() {
        // Native Ollama requests must express the /v1-inexpressible knobs:
        // think:false (disable thinking) and options.num_ctx (context window
        // override, so stock official models run without custom tags).
        let cfg = ModelConfig {
            native: true,
            no_think: true,
            num_ctx: Some(32768),
            ..Default::default()
        };
        let client = OpenAiClient::new(cfg).unwrap();
        let body = client.build_native_body(
            &[Message::system("你是车辆助手"), Message::user("hi")],
            &[],
        );
        assert_eq!(body["think"], serde_json::json!(false));
        assert_eq!(body["options"]["num_ctx"], serde_json::json!(32768));
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn native_body_omits_optional_knobs() {
        // Defaults: no think key (model keeps its own default behavior) and
        // no options key (model tag keeps its own context length).
        let cfg = ModelConfig {
            native: true,
            ..Default::default()
        };
        let client = OpenAiClient::new(cfg).unwrap();
        let body = client.build_native_body(&[Message::user("hi")], &[]);
        assert!(body.get("think").is_none());
        assert!(body.get("options").is_none());
    }

    #[test]
    fn extra_body_overrides_no_think() {
        // An explicit gateway knob must win over the no_think default.
        let cfg = ModelConfig {
            no_think: true,
            extra_body: Some(
                serde_json::json!({ "enable_thinking": true })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
            ..Default::default()
        };
        let client = OpenAiClient::new(cfg).unwrap();
        let body = client.build_body(&[], &[]);
        assert_eq!(body["reasoning_effort"], serde_json::json!("none"));
        assert_eq!(body["enable_thinking"], serde_json::json!(true));
    }

    #[test]
    fn ollama_url_derived_from_v1_base() {
        let cfg = ModelConfig {
            base_url: "http://172.21.0.1:11434/v1".into(),
            ..Default::default()
        };
        assert_eq!(cfg.ollama_chat_url(), "http://172.21.0.1:11434/api/chat");
    }

    #[test]
    fn native_messages_conversion() {
        use crate::message::{FunctionCall, ToolCall as Tc};
        let messages = vec![
            Message::system("你是车辆助手"),
            Message {
                role: Role::User,
                content: Some(serde_json::json!([
                    {"type": "text", "text": "看图"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJD"}},
                ])),
                tool_calls: None,
                tool_call_id: None,
            },
            Message {
                role: Role::Assistant,
                content: None,
                tool_calls: Some(vec![Tc {
                    id: "call_0".into(),
                    call_type: "function".into(),
                    function: FunctionCall {
                        name: "skill_run".into(),
                        arguments: r#"{"id":"manual-rag"}"#.into(),
                    },
                }]),
                tool_call_id: None,
            },
            Message::tool_result("call_0", "[]"),
        ];
        let native = to_native_messages(&messages);
        assert_eq!(native[0]["role"], "system");
        assert_eq!(native[1]["content"], "看图");
        assert_eq!(native[1]["images"][0], "QUJD");
        // Arguments must be a native JSON object, not an encoded string.
        assert_eq!(native[2]["tool_calls"][0]["function"]["arguments"]["id"], "manual-rag");
        assert_eq!(native[3]["role"], "tool");
        assert_eq!(native[3]["content"], "[]");
    }

    #[tokio::test]
    async fn ollama_decoder_thinking_and_usage() {
        // NDJSON lines are newline-terminated, like real Ollama output.
        let s = OllamaDecoder::new(fake_stream(vec![
            r#"{"message":{"thinking":"想一想"},"done":false}"#.to_string() + "\n",
            r#"{"message":{"content":"答案是"},"done":false}"#.to_string() + "\n",
            r#"{"message":{"content":"2"},"done":false}"#.to_string() + "\n",
            r#"{"message":{},"done":true,"prompt_eval_count":11,"eval_count":22}"#.to_string() + "\n",
        ]));
        let (text, done, usage) = collect_ollama(s).await;
        assert_eq!(text, "<think>想一想</think>答案是2");
        assert!(done);
        assert_eq!(usage.map(|u| (u.prompt_tokens, u.completion_tokens)), Some((11, 22)));
    }

    #[tokio::test]
    async fn ollama_decoder_tool_call_on_done_line() {
        // Ollama puts the model's tool call ON the terminal done line —
        // dropping it there silently kills agent tool use (the model looks
        // like it "never calls tools").
        let s = OllamaDecoder::new(fake_stream(vec![
            r#"{"message":{"thinking":"想一想"},"done":false}"#.to_string() + "\n",
            r#"{"message":{"content":"调用工具"},"done":false}"#.to_string() + "\n",
            r#"{"message":{"tool_calls":[{"function":{"name":"skill_list","arguments":{}}}]},"done":true,"prompt_eval_count":5,"eval_count":6}"#.to_string() + "\n",
        ]));
        let mut text = String::new();
        let mut calls = Vec::new();
        let mut done = false;
        let mut stream = s;
        while let Some(ev) = stream.next().await {
            match ev.unwrap() {
                StreamEvent::Delta(d) => text.push_str(&d),
                StreamEvent::Done { tool_calls: c, usage } => {
                    calls = c;
                    done = true;
                    assert_eq!(usage.map(|u| (u.prompt_tokens, u.completion_tokens)), Some((5, 6)));
                }
            }
        }
        assert_eq!(text, "<think>想一想</think>调用工具");
        assert!(done);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "skill_list");
        assert_eq!(calls[0].function.arguments, "{}");
    }

    /// Drain an OllamaDecoder into (text, saw Done, terminal usage).
    async fn collect_ollama(
        s: impl Stream<Item = Result<StreamEvent>> + Unpin,
    ) -> (String, bool, Option<Usage>) {
        let mut text = String::new();
        let mut done = false;
        let mut usage = None;
        let mut stream = s;
        while let Some(ev) = stream.next().await {
            match ev.unwrap() {
                StreamEvent::Delta(d) => text.push_str(&d),
                StreamEvent::Done { usage: u, .. } => {
                    done = true;
                    usage = u;
                }
            }
        }
        (text, done, usage)
    }

    /// Drain a decoder into (concatenated text deltas, saw terminal Done).
    async fn collect_text_and_done(
        s: impl Stream<Item = Result<StreamEvent>> + Unpin,
    ) -> (String, bool) {
        let mut text = String::new();
        let mut done = false;
        let mut stream = s;
        while let Some(ev) = stream.next().await {
            match ev.unwrap() {
                StreamEvent::Delta(d) => text.push_str(&d),
                StreamEvent::Done { .. } => done = true,
            }
        }
        (text, done)
    }
}
