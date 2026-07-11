//! MCP client: connect to external MCP servers (stdio) so the agent loop can
//! use their tools alongside built-in ones.
//!
//! Config: ~/.liteclaw/mcp.json
//!   { "servers": { "name": { "command": "...", "args": [...] } } }
//!
//! At startup, each configured server is spawned, we do the JSON-RPC handshake
//! (initialize + tools/list), and its tools are merged into the agent's tool set.
//! When the model calls one, we proxy the call to the server via tools/call.

use crate::ToolOutcome;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Config file: ~/.liteclaw/mcp.json
#[derive(Debug, Deserialize, Default)]
pub struct McpConfig {
    pub servers: HashMap<String, McpServer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServer {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// A discovered tool from an MCP server.
#[derive(Debug, Clone, Serialize)]
pub struct McpToolInfo {
    pub server: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A live connection to one MCP server.
pub struct McpConnection {
    pub server_name: String,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpConnection {
    /// Spawn the server process and do the initialize handshake.
    pub fn spawn(name: &str, cfg: &McpServer) -> anyhow::Result<Self> {
        let mut child = Command::new(&cfg.command)
            .args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
        let mut conn = Self {
            server_name: name.into(),
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        };
        // Initialize handshake.
        let _init = conn.request("initialize", serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "liteclaw", "version": "0.1.0" },
        }))?;
        // Send initialized notification (no id).
        conn.notify("notifications/initialized", serde_json::json!({}))?;
        Ok(conn)
    }

    /// List tools from this server.
    pub fn list_tools(&mut self) -> anyhow::Result<Vec<McpToolInfo>> {
        let resp = self.request("tools/list", serde_json::json!({}))?;
        let tools = resp
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .map(|t| McpToolInfo {
                server: self.server_name.clone(),
                name: t.get("name").and_then(|v| v.as_str()).unwrap_or("").into(),
                description: t.get("description").and_then(|v| v.as_str()).unwrap_or("").into(),
                input_schema: t.get("inputSchema").cloned().unwrap_or(serde_json::json!({})),
            })
            .collect())
    }

    /// Call a tool on this server.
    pub fn call_tool(&mut self, name: &str, args: &serde_json::Value) -> anyhow::Result<ToolOutcome> {
        let resp = self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": args }),
        )?;
        // MCP returns { content: [{ type: "text", text: "..." }], isError: bool }
        let is_error = resp.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
        let text = resp
            .get("content")
            .and_then(|c| c.as_array())
            .and_then(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                    .next()
            })
            .unwrap_or("(no output)");
        Ok(ToolOutcome {
            ok: !is_error,
            summary: text.into(),
        })
    }

    /// Send a JSON-RPC request and read the response.
    fn request(&mut self, method: &str, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        });
        self.send(&req)?;
        // Read lines until we find our response.
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line)?;
            if n == 0 {
                return Err(anyhow::anyhow!("MCP server closed connection"));
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let msg: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if msg.get("id").and_then(|v| v.as_u64()) == Some(id) {
                return Ok(msg.get("result").cloned().unwrap_or(serde_json::Value::Null));
            }
            // Not our response (notification from server); ignore.
        }
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) -> anyhow::Result<()> {
        self.send(&serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&mut self, msg: &serde_json::Value) -> anyhow::Result<()> {
        let line = serde_json::to_string(msg)?;
        writeln!(self.stdin, "{line}")?;
        self.stdin.flush()?;
        Ok(())
    }
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        // Clean up the child process.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Read MCP config from ~/.liteclaw/mcp.json.
pub fn read_config() -> McpConfig {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let path = std::path::PathBuf::from(home).join(".liteclaw/mcp.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Discover all tools from all configured MCP servers. Returns connections
/// (keep alive) + tool infos.
pub fn discover_all() -> (Vec<McpConnection>, Vec<McpToolInfo>) {
    let config = read_config();
    let mut conns = Vec::new();
    let mut tools = Vec::new();
    for (name, server_cfg) in &config.servers {
        match McpConnection::spawn(name, server_cfg) {
            Ok(mut conn) => {
                match conn.list_tools() {
                    Ok(mut t) => {
                        eprintln!("[mcp] {}: {} tools", name, t.len());
                        tools.append(&mut t);
                        conns.push(conn);
                    }
                    Err(e) => {
                        eprintln!("[mcp] {}: tools/list failed: {e}", name);
                    }
                }
            }
            Err(e) => {
                eprintln!("[mcp] {}: spawn failed: {e}", name);
            }
        }
    }
    (conns, tools)
}
