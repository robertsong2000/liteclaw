//! The agent loop: reason → tool → observe.
//!
//! Given a model client, a conversation, and a tool set, this drives a
//! multi-turn loop:
//!   1. stream the model's response (text deltas forwarded as events);
//!   2. if the model emitted tool calls, Defender-check + execute each, feed
//!      results back into the conversation, and loop;
//!   3. if the model produced only text (no tool calls), the turn is done.
//!
//! Tool execution for `Confirm` tools is delegated to a caller-supplied
//! async confirm callback, so the agent crate stays decoupled from the web
//! layer (which implements the human-in-the-loop UI).

pub mod backup;
pub mod events;
pub mod hooks;
pub mod mcp_client;
pub mod tools;

pub use events::AgentEvent;
pub use hooks::{default_hooks, DefenderHook, Hook, HookChain, HookContext, LogHook, PreToolVerdict};
pub use tools::{default_tools, extra_tools, find, skill_tools, to_specs, Approval, Tool, ToolOutcome};

use anyhow::Result;
use futures::{Stream, StreamExt};
use liteclaw_core::Ctx;
use liteclaw_model::{Message, OpenAiClient, StreamEvent};
use std::future::Future;
use std::sync::Arc;
use tokio::sync::mpsc;

/// A callback that asks for human approval before a mutating tool runs.
/// Returns `true` to allow, `false` to skip. The `confirm_id` is a unique id
/// the frontend references when posting its decision to /api/confirm.
pub type ConfirmFn = Arc<
    dyn Fn(
            String,
            serde_json::Value,
            String,
        ) -> std::pin::Pin<Box<dyn Future<Output = bool> + Send>>
        + Send
        + Sync,
>;

/// Drive an agent turn, streaming [`AgentEvent`]s to the caller.
///
/// `max_iters` bounds the number of tool-use rounds to avoid runaway loops.
pub async fn run_loop(
    tx: mpsc::Sender<AgentEvent>,
    model: OpenAiClient,
    messages: Vec<Message>,
    tools: Vec<Tool>,
    ctx: Arc<Ctx>,
    confirm: Option<ConfirmFn>,
    max_iters: usize,
) -> Result<()> {
    let mut messages = messages;
    let specs = to_specs(&tools);
    let confirm = confirm;
    let mut confirm_counter = Counter::default();
    let mut total_output_chars: usize = 0;
    // Record the time of the FIRST delta — excludes queue/network latency
    // before the model starts producing, so TPS reflects generation speed.
    let mut gen_start: Option<std::time::Instant> = None;
    let mut gen_end: Option<std::time::Instant> = None;
    // Gateway reasoning models (deepseek-flash behind new-api) occasionally
    // stream a whole turn as reasoning_content and stop: no tool calls, no
    // visible answer — the UI would show nothing but a collapsed think
    // block. Retry such turns a bounded number of times.
    let mut empty_answer_retries = 0usize;

    for _iter in 0..max_iters {
        // 1. Stream the model response, accumulating text + tool calls.
        let mut stream = model.chat_stream(&messages, &specs).await?;
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        let mut usage: Option<liteclaw_model::openai::Usage> = None;

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::Delta(chunk) => {
                    if gen_start.is_none() {
                        gen_start = Some(std::time::Instant::now());
                    }
                    gen_end = Some(std::time::Instant::now());
                    text.push_str(&chunk);
                    // Count ALL output chars including <think> blocks — they
                    // are real generated tokens even if filtered for display.
                    total_output_chars += chunk.chars().count();
                    let _ = tx.send(AgentEvent::text_delta(chunk)).await;
                }
                StreamEvent::Done {
                    tool_calls: calls,
                    usage: u,
                } => {
                    tool_calls = calls;
                    if u.is_some() {
                        usage = u;
                    }
                }
            }
        }

        // 2. Record the assistant turn. The visible-answer check must run
        // before `text` is moved into the recorded message.
        let answer_is_empty = visible_answer(&text).is_empty();
        messages.push(Message {
            role: liteclaw_model::Role::Assistant,
            content: if text.is_empty() {
                None
            } else {
                Some(serde_json::Value::String(text))
            },
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls.clone())
            },
            tool_call_id: None,
        });

        // 3. No tool calls → the model answered in plain text; done.
        if tool_calls.is_empty() {
            // Reasoning-only turn: everything streamed inside <think> blocks.
            // The deltas were already forwarded (the UI shows a collapsed
            // think block), but there is no answer to read — drop the turn
            // and retry with a corrective nudge instead of ending here.
            if answer_is_empty && empty_answer_retries < MAX_EMPTY_ANSWER_RETRIES {
                empty_answer_retries += 1;
                messages.pop(); // the reasoning-only assistant turn
                messages.push(Message {
                    role: liteclaw_model::Role::User,
                    content: Some(serde_json::Value::String(
                        "（系统提示：你上一轮只输出了思考过程，没有正式回答。请直接给出正式回答，不要再重复思考。）"
                            .into(),
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                });
                continue;
            }
            // Compute generation-only elapsed time (first delta → last delta),
            // excluding pre-generation queue/network latency.
            let gen_ms = match (gen_start, gen_end) {
                (Some(s), Some(e)) => e.duration_since(s).as_millis(),
                _ => 0,
            };
            // Prefer the provider-reported real token count (Ollama returns
            // one when stream_options.include_usage is honored); fall back to
            // the chars/1.5 heuristic for endpoints that don't send usage.
            // The <think> content is included since it's real generated output.
            let tokens = match usage.map(|u| u.completion_tokens as usize) {
                Some(n) if n > 0 => n,
                _ => ((total_output_chars as f64) / 1.5).round() as usize,
            };
            let tokens = tokens.max(1);
            let tps = if gen_ms > 0 {
                Some((tokens as f64) * 1000.0 / (gen_ms as f64))
            } else {
                None
            };
            let _ = tx
                .send(AgentEvent::Done {
                    tps,
                    tokens: Some(tokens),
                    elapsed_ms: Some(gen_ms),
                })
                .await;
            return Ok(());
        }

        // 4. Execute each tool call, feeding results back.
        for call in tool_calls {
            let args: serde_json::Value =
                serde_json::from_str(&call.function.arguments).unwrap_or(serde_json::Value::Null);
            let Some(tool) = find(&tools, &call.function.name) else {
                let msg = format!("unknown tool: {}", call.function.name);
                let _ = tx.send(AgentEvent::error(&msg)).await;
                messages.push(Message::tool_result(&call.id, msg));
                continue;
            };

            // In auto mode (no confirm callback), treat all tools as auto-run.
            let needs_confirm = tool.approval == Approval::Confirm && confirm.is_some();
            // Generate a confirm id for tools that need human approval.
            let confirm_id = if needs_confirm {
                Some(format!("c{}", confirm_counter.next_val()))
            } else {
                None
            };
            let _ = tx
                .send(AgentEvent::ToolStart {
                    tool: tool.name.into(),
                    arguments: args.clone(),
                    needs_confirmation: needs_confirm,
                    confirm_id: confirm_id.clone(),
                })
                .await;

            let outcome = if needs_confirm {
                // confirm.is_some() is guaranteed by the needs_confirm calc above.
                let cf = confirm.as_ref().unwrap();
                let id = confirm_id.clone().unwrap_or_default();
                let allowed = (cf)(tool.name.into(), args.clone(), id).await;
                if allowed {
                    tool.execute(&args, &ctx).await
                } else {
                    ToolOutcome::failed("denied by user")
                }
            } else {
                tool.execute(&args, &ctx).await
            };

            let _ = tx
                .send(AgentEvent::ToolResult {
                    tool: tool.name.into(),
                    ok: outcome.ok,
                    summary: outcome.summary.clone(),
                })
                .await;

            messages.push(Message::tool_result(&call.id, outcome.summary));
        }
        // Loop again: let the model see the tool results and continue.
    }

    // Hit the iteration cap.
    let _ = tx
        .send(AgentEvent::error(format!(
            "reached max iterations ({max_iters})"
        )))
        .await;
    Ok(())
}

/// How many times `run_loop` retries a turn that produced no visible answer
/// (reasoning-only output). One retry is enough in practice: the failure is
/// sampler-dependent, not deterministic.
const MAX_EMPTY_ANSWER_RETRIES: usize = 1;

/// The part of a streamed response the user actually reads: everything
/// outside `<think>…</think>` blocks, trimmed. An unclosed trailing block
/// swallows the rest (the decoder closes one at stream end, but don't rely
/// on it when deciding whether an answer exists).
fn visible_answer(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest.find("</think>") {
            Some(end) => rest = &rest[end + "</think>".len()..],
            None => return out.trim().to_string(),
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Convenience: run the loop and collect all events into a channel-backed
/// stream. Used by the web handler to pump SSE.
pub fn into_stream(
    model: OpenAiClient,
    messages: Vec<Message>,
    tools: Vec<Tool>,
    ctx: Arc<Ctx>,
    confirm: Option<ConfirmFn>,
    max_iters: usize,
) -> (
    mpsc::Receiver<AgentEvent>,
    tokio::task::JoinHandle<Result<()>>,
) {
    let (tx, rx) = mpsc::channel(64);
    let handle = tokio::spawn(async move {
        if let Err(e) = run_loop(tx, model, messages, tools, ctx, confirm, max_iters).await {
            tracing_log_error(&e);
        }
        Ok::<(), anyhow::Error>(())
    });
    (rx, handle)
}

fn tracing_log_error(e: &anyhow::Error) {
    eprintln!("[agent] loop error: {e:#}");
}

/// Simple incrementing counter for confirm ids (single agent task, no need
/// for atomics).
#[derive(Default)]
struct Counter {
    n: usize,
}
impl Counter {
    fn next_val(&mut self) -> usize {
        self.n += 1;
        self.n
    }
}

/// Convert a channel receiver into a stream that yields `None` when closed.
pub fn rx_to_stream(rx: mpsc::Receiver<AgentEvent>) -> impl Stream<Item = AgentEvent> {
    tokio_stream::wrappers::ReceiverStream::new(rx)
}

#[cfg(test)]
mod tests {
    use super::visible_answer;

    #[test]
    fn plain_text_is_visible() {
        assert_eq!(visible_answer("后雾灯：旋环 4 转 AUTO。"), "后雾灯：旋环 4 转 AUTO。");
    }

    #[test]
    fn think_only_turn_has_no_visible_answer() {
        assert!(visible_answer("<think>reasoning draft…</think>").is_empty());
        assert!(visible_answer("<think>unclosed reasoning…").is_empty());
        assert!(visible_answer("").is_empty());
        assert!(visible_answer("  \n").is_empty());
    }

    #[test]
    fn text_after_think_blocks_survives() {
        let t = "<think>phase 1</think><think>phase 2</think>按除雾键（按钮 13）。";
        assert_eq!(visible_answer(t), "按除雾键（按钮 13）。");
        // Interleaved: reasoning wraps around a visible fragment.
        let t = "先看这里<think>mid-turn reasoning</think>然后是结论。";
        assert_eq!(visible_answer(t), "先看这里然后是结论。");
    }
}
