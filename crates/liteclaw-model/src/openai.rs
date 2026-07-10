//! Streaming OpenAI-compatible client.
//!
//! Sends a chat-completions request with `stream: true` and yields
//! [`StreamEvent`]s as the model produces them. Works against any
//! OpenAI-compatible endpoint (cloud or local Ollama `/v1`).
//!
//! The two non-trivial bits:
//! - SSE line parsing (`data: {...}\n\n`, terminated by `data: [DONE]`).
//! - Tool-call deltas arrive fragmented across chunks, indexed by
//!   `tool_calls[i].index`; we accumulate them into complete calls.

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
    Done { tool_calls: Vec<ToolCall> },
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
    ) -> Result<impl Stream<Item = Result<StreamEvent>> + Send> {
        let mut body = serde_json::json!({
            "model": self.cfg.model,
            "messages": messages,
            "stream": true,
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }

        let mut req = self.http.post(self.cfg.chat_url()).json(&body);
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
        let byte_stream = resp.bytes_stream();
        let event_stream = SseDecoder::new(byte_stream);
        Ok(event_stream)
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

impl<S> SseDecoder<S>
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send,
{
    fn new(inner: S) -> Self {
        Self {
            inner,
            buf: String::new(),
            tool_calls: Vec::new(),
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
        if this.finished {
            return std::task::Poll::Ready(None);
        }
        loop {
            // First, try to pull a complete SSE frame from the buffer.
            if let Some(idx) = this.buf.find("\n\n") {
                let frame = this.buf.drain(..idx).collect::<String>();
                // consume the delimiter
                this.buf.drain(..2);
                match handle_frame(&frame, &mut this.tool_calls) {
                    FrameOutcome::Delta(d) => {
                        return std::task::Poll::Ready(Some(Ok(StreamEvent::Delta(d))));
                    }
                    FrameOutcome::Done => {
                        this.finished = true;
                        let calls = std::mem::take(&mut this.tool_calls)
                            .into_iter()
                            .map(|a| ToolCall {
                                id: a.id,
                                call_type: "function".into(),
                                function: crate::message::FunctionCall {
                                    name: a.name,
                                    arguments: a.arguments,
                                },
                            })
                            .collect();
                        return std::task::Poll::Ready(Some(Ok(StreamEvent::Done {
                            tool_calls: calls,
                        })));
                    }
                    FrameOutcome::Ignore => continue,
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
                    // Upstream ended. Always emit a terminal Done so the agent
                    // loop's while-let exits cleanly (it cannot distinguish a
                    // clean close from a missing [DONE] otherwise). Any buffered
                    // frame is dropped — partial trailing data is not useful.
                    this.finished = true;
                    let calls = std::mem::take(&mut this.tool_calls)
                        .into_iter()
                        .map(|a| ToolCall {
                            id: a.id,
                            call_type: "function".into(),
                            function: crate::message::FunctionCall {
                                name: a.name,
                                arguments: a.arguments,
                            },
                        })
                        .collect();
                    return std::task::Poll::Ready(Some(Ok(StreamEvent::Done {
                        tool_calls: calls,
                    })));
                }
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

enum FrameOutcome {
    Delta(String),
    Done,
    Ignore,
}

fn handle_frame(frame: &str, tool_calls: &mut Vec<ToolCallAccum>) -> FrameOutcome {
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
        return FrameOutcome::Done;
    }

    #[derive(Deserialize)]
    struct Chunk {
        choices: Vec<Choice>,
    }
    #[derive(Deserialize)]
    struct Choice {
        delta: Delta,
    }
    #[derive(Deserialize, Default)]
    struct Delta {
        #[serde(default)]
        content: Option<String>,
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

    if let Some(text) = choice.delta.content {
        if !text.is_empty() {
            return FrameOutcome::Delta(text);
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
                Ok(StreamEvent::Done { tool_calls }) => Some(tool_calls.clone()),
                _ => None,
            })
            .expect("a Done event");
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].function.name, "read");
        assert_eq!(done[0].function.arguments, r#"{"path":"Cargo.toml"}"#);
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
}
