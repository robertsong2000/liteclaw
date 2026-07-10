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

pub mod events;
pub mod tools;

pub use events::AgentEvent;
pub use tools::{default_tools, find, skill_tools, to_specs, Approval, Tool, ToolOutcome};

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
    dyn Fn(String, serde_json::Value, String) -> std::pin::Pin<Box<dyn Future<Output = bool> + Send>>
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

    for _iter in 0..max_iters {
        // 1. Stream the model response, accumulating text + tool calls.
        let mut stream = model.chat_stream(&messages, &specs).await?;
        let mut text = String::new();
        let mut tool_calls = Vec::new();

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
                StreamEvent::Done { tool_calls: calls } => {
                    tool_calls = calls;
                }
            }
        }

        // 2. Record the assistant turn.
        messages.push(Message {
            role: liteclaw_model::Role::Assistant,
            content: if text.is_empty() { None } else { Some(text) },
            tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls.clone()) },
            tool_call_id: None,
        });

        // 3. No tool calls → the model answered in plain text; done.
        if tool_calls.is_empty() {
            // Compute generation-only elapsed time (first delta → last delta),
            // excluding pre-generation queue/network latency.
            let gen_ms = match (gen_start, gen_end) {
                (Some(s), Some(e)) => e.duration_since(s).as_millis(),
                _ => 0,
            };
            // Token estimate: CJK-heavy text ≈ 1.5 chars/token (not 3, which
            // is English-only). The <think> content is included since it's
            // real generated output.
            let tokens = ((total_output_chars as f64) / 1.5).round() as usize;
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
            let args: serde_json::Value = serde_json::from_str(&call.function.arguments)
                .unwrap_or(serde_json::Value::Null);
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

/// Convenience: run the loop and collect all events into a channel-backed
/// stream. Used by the web handler to pump SSE.
pub fn into_stream(
    model: OpenAiClient,
    messages: Vec<Message>,
    tools: Vec<Tool>,
    ctx: Arc<Ctx>,
    confirm: Option<ConfirmFn>,
    max_iters: usize,
) -> (mpsc::Receiver<AgentEvent>, tokio::task::JoinHandle<Result<()>>) {
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
