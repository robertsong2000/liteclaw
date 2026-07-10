//! Minimal MCP (Model Context Protocol) server over stdio JSON-RPC.
//!
//! Exposes liteclaw's tools to external MCP clients (Claude Code, Cursor, etc.)
//! without pulling an external SDK — just reads JSON-RPC from stdin, writes to
//! stdout. Implements: initialize, tools/list, tools/call.

use liteclaw_agent::{default_tools, extra_tools, skill_tools, Tool, ToolOutcome};
use liteclaw_core::{Ctx, ExitCode, Sandbox};
use std::io::{self, BufRead, Write};
use std::sync::Arc;

/// Run the MCP server loop: read JSON-RPC requests from stdin, respond on stdout.
pub fn run() -> i32 {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let ctx = Ctx::new(
        cwd,
        Sandbox::readonly().allow_write(std::path::PathBuf::from(".")),
        false,
        false,
    );
    let ctx = Arc::new(ctx);

    // Build the tool set from real registered claws + skill/extra tools.
    let claws = crate::registry::all_claws();
    let mut tools = default_tools(&claws);
    // skill_tools/extra_tools need an Arc<dyn Claw> placeholder (they use
    // captured executors, never the claw field).
    let placeholder: Arc<dyn liteclaw_core::Claw> = Arc::new(DummyClaw);
    tools.extend(skill_tools(placeholder.clone()));
    tools.extend(extra_tools(placeholder));

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = req.get("id").cloned();
        let result = match method {
            "initialize" => serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "liteclaw", "version": "0.1.0" },
            }),
            "tools/list" => serde_json::json!({
                "tools": tools.iter().map(|t| serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.parameters,
                })).collect::<Vec<_>>(),
            }),
            "tools/call" => {
                let name = req
                    .pointer("/params/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args = req
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                call_tool(&tools, name, &args, &ctx)
            }
            _ => serde_json::json!({ "error": { "code": -32601, "message": "method not found" } }),
        };
        // Only respond to requests with an id (notifications have none).
        if let Some(id) = id {
            let resp = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
            writeln!(out, "{resp}").ok();
            out.flush().ok();
        }
    }
    0
}

/// Execute a tool by name and format the result as an MCP tool-call response.
fn call_tool(tools: &[Tool], name: &str, args: &serde_json::Value, ctx: &Arc<Ctx>) -> serde_json::Value {
    let Some(tool) = tools.iter().find(|t| t.name == name) else {
        return serde_json::json!({
            "content": [{ "type": "text", "text": format!("unknown tool: {name}") }],
            "isError": true,
        });
    };
    // Clone the args (owned) and run in a dedicated thread with its own runtime,
    // to avoid "cannot start runtime from within a runtime".
    let args_owned = args.clone();
    let ctx2 = ctx.clone();
    let outcome = std::thread::scope(|s| {
        s.spawn(move || {
            let rt = tokio::runtime::Runtime::new().ok()?;
            Some(rt.block_on(tool.execute(&args_owned, &ctx2)))
        })
        .join()
        .ok()
        .flatten()
    });
    let outcome = outcome.unwrap_or_else(|| ToolOutcome { ok: false, summary: "tool execution failed".into() });
    serde_json::json!({
        "content": [{ "type": "text", "text": outcome.summary }],
        "isError": !outcome.ok,
    })
}

/// A no-op claw used as a placeholder for tool registration (MCP only uses
/// the captured executors, never the claw field).
struct DummyClaw;
#[async_trait::async_trait]
impl liteclaw_core::Claw for DummyClaw {
    fn name(&self) -> &'static str {
        "dummy"
    }
    fn desc(&self) -> &'static str {
        "placeholder"
    }
    async fn run(
        &self,
        _args: &liteclaw_core::ClawArgs,
        _ctx: &Ctx,
    ) -> anyhow::Result<ExitCode> {
        Ok(ExitCode::Success)
    }
}
