//! Streaming OpenAI-compatible client.
//!
//! Sends a chat-completions request with `stream: true` and yields
//! [`StreamEvent`]s as the model produces them. Works against any
//! OpenAI-compatible endpoint (cloud, gateway, or local Ollama `/v1`) —
//! one protocol for every backend.
//!
//! The non-trivial bits:
//! - SSE line parsing (`data: {...}\n\n`, terminated by `data: [DONE]`).
//! - Tool-call deltas arrive fragmented across chunks, indexed by
//!   `tool_calls[i].index`; we accumulate them into complete calls.
//! - Reasoning models (qwen3/minicpm5 on local Ollama, Qwen flash behind
//!   new-api) stream their thinking in a separate `reasoning_content`
//!   field. We wrap it in inline `<think>...</think>` tags so downstream
//!   consumers see one shape (the chat UI renders those as a collapsible
//!   "思考过程" block). `no_think` suppresses it upstream via
//!   `reasoning_effort: "none"`, which Ollama maps to `think: false`.

use std::collections::VecDeque;
use std::pin::Pin;

use crate::config::ModelConfig;
use crate::message::{Message, ToolCall, ToolSpec};
use anyhow::{anyhow, Result};
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
    pub async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolSpec],
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>> {
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
            .map_err(|e| anyhow!("request failed: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("model API error {status}: {text}"));
        }

        // Convert the response byte stream into a stream of parsed SSE events.
        let event_stream = SseDecoder::new(resp.bytes_stream());
        Ok(Box::pin(event_stream))
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

#[cfg(test)]
mod tests {
    use super::*;

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
