//! Tool registry: wraps existing claws into OpenAI function-calling specs and
//! provides execution with Defender pre-checks.
//!
//! Tools are split into read-only (auto-approved) and write (need human
//! confirmation) per liteclaw's security model.

use liteclaw_core::{Claw, Ctx};
use liteclaw_model::{ToolFunction, ToolSpec};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Whether a tool runs automatically or needs a human to approve it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Approval {
    /// Read-only: runs automatically.
    Auto,
    /// Mutating: the frontend must confirm before it executes.
    Confirm,
}

/// A tool the agent can invoke: its schema, approval level, and executor.
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: serde_json::Value,
    pub approval: Approval,
    /// How to turn JSON arguments into the claw's positional args.
    pub arg_order: &'static [&'static str],
    /// The underlying claw that actually does the work.
    pub claw: Arc<dyn Claw>,
}

impl Tool {
    /// Render this tool as an OpenAI function-calling spec.
    pub fn to_spec(&self) -> ToolSpec {
        ToolSpec {
            spec_type: "function".into(),
            function: ToolFunction {
                name: self.name.into(),
                description: self.description.into(),
                parameters: self.parameters.clone(),
            },
        }
    }

    /// Extract positional args from the model's JSON arguments, in `arg_order`.
    pub fn positionals(&self, args: &serde_json::Value) -> Vec<String> {
        self.arg_order
            .iter()
            .filter_map(|k| args.get(k).and_then(|v| v.as_str()).map(String::from))
            .collect()
    }

    /// Execute the tool and capture its output as a string. Runs the Defender
    /// pre-check via a default hook chain.
    pub async fn execute(&self, args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
        self.execute_with_hooks(args, ctx, &crate::hooks::default_hooks()).await
    }

    /// Execute with a caller-supplied hook chain. PreToolUse hooks run first
    /// (Defender, custom validators); PostToolUse hooks run after (logging, audit).
    pub async fn execute_with_hooks(
        &self,
        args: &serde_json::Value,
        ctx: &Ctx,
        hooks: &crate::hooks::HookChain,
    ) -> ToolOutcome {
        // PreToolUse hooks (Defender is one of them).
        let hc = crate::hooks::HookContext {
            tool_name: self.name,
            args,
            ctx,
        };
        let effective_args_owned;
        let effective_args = match hooks.pre(&hc).await {
            crate::hooks::PreToolVerdict::Block { reason } => {
                return ToolOutcome::blocked(reason);
            }
            crate::hooks::PreToolVerdict::Allow { modified_args } => {
                match modified_args {
                    Some(ma) => {
                        effective_args_owned = ma;
                        &effective_args_owned
                    }
                    None => args,
                }
            }
        };
        // Execute the actual tool logic.
        let mut outcome = match self.name {
            "read" => exec_read(effective_args, ctx),
            "grep" => exec_grep(effective_args, ctx),
            "audit" => exec_audit(effective_args, ctx),
            "glob" => exec_glob(effective_args, ctx),
            "fetch" => exec_fetch(effective_args, ctx).await,
            "edit" => exec_edit(effective_args, ctx).await,
            "write" => exec_write(effective_args, ctx).await,
            "bash" => exec_bash(effective_args, ctx).await,
            "skill_list" => exec_skill_list(),
            "skill_run" => exec_skill_run(effective_args, ctx).await,
            "undo" => crate::backup::undo_last(),
            other => ToolOutcome::failed(format!("tool '{other}' has no captured executor")),
        };
        // PostToolUse hooks (logging, audit).
        let post_hc = crate::hooks::HookContext {
            tool_name: self.name,
            args: effective_args,
            ctx,
        };
        hooks.post(&post_hc, &mut outcome).await;
        outcome
    }
}

/// The result of executing a tool.
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub ok: bool,
    pub summary: String,
}

impl ToolOutcome {
    pub fn ok(s: impl Into<String>) -> Self {
        Self {
            ok: true,
            summary: s.into(),
        }
    }
    pub fn failed(s: impl Into<String>) -> Self {
        Self {
            ok: false,
            summary: s.into(),
        }
    }
    pub fn blocked(s: impl Into<String>) -> Self {
        Self {
            ok: false,
            summary: s.into(),
        }
    }
}

// ---- Captured executors: return tool output as a string (fed to the model) ----
// These bypass the Claw trait (which prints to the real stdout, un-interceptable
// from the agent). Each mirrors its claw's logic but returns text instead.

/// Max bytes returned to the model from a single read (keeps context bounded).
const READ_MAX_BYTES: usize = 64 * 1024;

fn resolve_path(args: &serde_json::Value, key: &str, ctx: &Ctx) -> std::path::PathBuf {
    let p = args.get(key).and_then(|v| v.as_str()).unwrap_or(".");
    ctx.cwd.join(p)
}

/// Glob: list files matching a pattern (e.g. "**/*.rs", "src/*.ts").
fn exec_glob(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return ToolOutcome::failed("missing 'pattern'"),
    };
    let root = resolve_path(args, "path", ctx);
    let walker = ignore::WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(true)
        .build();
    let mut matches = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let p = entry.path();
        let rel = p.strip_prefix(&ctx.cwd).unwrap_or(p);
        let rel_str = rel.to_string_lossy();
        // Simple glob: support ** and * wildcards.
        if glob_match(&pattern, &rel_str) {
            matches.push(rel_str.to_string());
            if matches.len() >= 100 {
                matches.push("…[truncated at 100]".into());
                break;
            }
        }
    }
    if matches.is_empty() {
        ToolOutcome::failed("no files matched")
    } else {
        ToolOutcome::ok(matches.join("\n"))
    }
}

/// Minimal glob matcher: supports * (any chars except /) and ** (any chars).
fn glob_match(pattern: &str, text: &str) -> bool {
    // Convert glob to regex manually (avoid pulling globset dep).
    let mut re = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    re.push_str(".*");
                } else {
                    re.push_str("[^/]*");
                }
            }
            '?' => re.push('.'),
            '.' | '+' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            _ => re.push(c),
        }
    }
    re.push('$');
    regex::RegexBuilder::new(&re)
        .case_insensitive(true)
        .build()
        .map(|r| r.is_match(text))
        .unwrap_or(false)
}

/// Web fetch: download a URL and return text content. Defender validates the URL
/// against SSRF rules (blocks private IPs, metadata endpoints, exfil sinks).
async fn exec_fetch(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let url = match args.get("url").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ToolOutcome::failed("missing 'url'"),
    };
    // Defender URL check: blocks localhost, private IPs, metadata, exfil.
    let report = ctx.guard_url(url);
    if matches!(report.action, liteclaw_core::defender::Action::Block) {
        return ToolOutcome::blocked(format!(
            "Defender blocked URL ({} score {})",
            report.severity.label(),
            report.score
        ));
    }
    // Fetch with a 30s timeout, text only.
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => return ToolOutcome::failed(format!("http client: {e}")),
    };
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return ToolOutcome::failed(format!("fetch failed: {e}")),
    };
    let status = resp.status();
    let text = match resp.text().await {
        Ok(t) => t,
        Err(e) => return ToolOutcome::failed(format!("read body: {e}")),
    };
    // Truncate to keep model context bounded.
    let max = 8 * 1024;
    let summary = if text.len() > max {
        format!("HTTP {} (truncated):\n{}", status, &text[..max])
    } else {
        format!("HTTP {}:\n{}", status, text)
    };
    ToolOutcome {
        ok: status.is_success(),
        summary,
    }
}

fn exec_read(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let path = resolve_path(args, "path", ctx);
    if path.is_dir() {
        let mut names: Vec<String> = match std::fs::read_dir(&path) {
            Ok(rd) => rd
                .flatten()
                .map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        format!("{n}/")
                    } else {
                        n
                    }
                })
                .collect(),
            Err(e) => return ToolOutcome::failed(format!("read dir: {e}")),
        };
        names.sort();
        return ToolOutcome::ok(names.join("\n"));
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return ToolOutcome::failed(format!("read {}: {e}", path.display())),
    };
    let truncated = bytes.len() > READ_MAX_BYTES;
    let slice = if truncated {
        &bytes[..READ_MAX_BYTES]
    } else {
        &bytes
    };
    let mut text = String::from_utf8_lossy(slice).to_string();
    if truncated {
        text.push_str(&format!("\n…[truncated at {READ_MAX_BYTES} bytes]"));
    }
    ToolOutcome::ok(text)
}

fn exec_grep(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return ToolOutcome::failed("missing 'pattern'"),
    };
    let root = resolve_path(args, "path", ctx);
    let re = match regex::RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .build()
    {
        Ok(r) => r,
        Err(e) => return ToolOutcome::failed(format!("invalid pattern: {e}")),
    };
    let walker = ignore::WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(true)
        .build();
    let mut hits = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let p = entry.path();
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                hits.push(format!("{}:{}:{}", p.display(), i + 1, line));
                if hits.len() >= 50 {
                    hits.push("…[too many matches, truncated at 50]".into());
                    return ToolOutcome::ok(hits.join("\n"));
                }
            }
        }
    }
    if hits.is_empty() {
        ToolOutcome::failed("no matches")
    } else {
        ToolOutcome::ok(hits.join("\n"))
    }
}

fn exec_audit(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let root = resolve_path(args, "path", ctx);
    let walker = ignore::WalkBuilder::new(&root)
        .hidden(true)
        .git_ignore(true)
        .build();
    let exts = [
        "md", "sh", "js", "py", "ts", "rs", "json", "yaml", "yml", "toml",
    ];
    let mut lines = Vec::new();
    let mut any_block = false;
    for entry in walker.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let p = entry.path();
        if !p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| exts.contains(&e))
            .unwrap_or(false)
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        let report = liteclaw_core::scan_text(&text);
        if report.is_clean() {
            continue;
        }
        if matches!(report.action, liteclaw_core::defender::Action::Block) {
            any_block = true;
        }
        lines.push(format!(
            "{}: {} (score {})",
            p.display(),
            report.severity.label(),
            report.score
        ));
    }
    if lines.is_empty() {
        ToolOutcome::ok("no issues found")
    } else {
        ToolOutcome {
            ok: !any_block,
            summary: lines.join("\n"),
        }
    }
}

async fn exec_edit(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let path = resolve_path(args, "path", ctx);
    // Sandbox check.
    if !ctx.sandbox.can_write(&path) {
        return ToolOutcome::failed("write denied by sandbox");
    }
    let old = match args.get("old").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ToolOutcome::failed("missing 'old'"),
    };
    let new = match args.get("new").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ToolOutcome::failed("missing 'new'"),
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return ToolOutcome::failed(format!("read: {e}")),
    };
    match content.matches(old).count() {
        0 => ToolOutcome::failed("no match for 'old'"),
        1 => {
            let updated = content.replacen(old, new, 1);
            match std::fs::write(&path, updated) {
                Ok(_) => ToolOutcome::ok(format!("edited {}", path.display())),
                Err(e) => ToolOutcome::failed(format!("write: {e}")),
            }
        }
        n => ToolOutcome::failed(format!("'old' matches {n} times; refusing ambiguous edit")),
    }
}

async fn exec_write(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let path = resolve_path(args, "path", ctx);
    // Sandbox check.
    if !ctx.sandbox.can_write(&path) {
        return ToolOutcome::failed("write denied by sandbox");
    }
    let content = match args.get("content").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ToolOutcome::failed("missing 'content'"),
    };
    // Create parent dirs if needed.
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ToolOutcome::failed(format!("create dir: {e}"));
            }
        }
    }
    match std::fs::write(&path, content) {
        Ok(_) => ToolOutcome::ok(format!(
            "wrote {} bytes to {}",
            content.len(),
            path.display()
        )),
        Err(e) => ToolOutcome::failed(format!("write: {e}")),
    }
}

/// Bash/shell execution. Defender scans the raw command for dangerous patterns
/// (rm -rf /, mkfs, fork bombs, reverse shells, etc.). Runs in the sandbox cwd
/// with a 60s timeout. stdout + stderr (truncated) are returned to the model.
async fn exec_bash(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let command = match args.get("command").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return ToolOutcome::failed("missing 'command'"),
    };

    // Dedicated Defender pass on the raw command (not the JSON wrapper), so the
    // command-injection patterns (rm -rf, mkfs, ...) match against real text.
    let report = liteclaw_core::scan_text(&command);
    if matches!(report.action, liteclaw_core::defender::Action::Block) {
        return ToolOutcome::blocked(format!(
            "Defender blocked command ({} score {}): matches '{}'",
            report.severity.label(),
            report.score,
            report
                .findings
                .first()
                .map(|f| f.matched.as_str())
                .unwrap_or("?"),
        ));
    }

    // Run via `sh -c` so pipes/redirects work, cwd locked to the sandbox dir.
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(&command);
    cmd.current_dir(&ctx.cwd);

    // Spawn with a bounded output capture.
    let child = match cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return ToolOutcome::failed(format!("spawn failed: {e}")),
    };

    // Wait with a 60s timeout to prevent runaway commands.
    let result =
        tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output()).await;
    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let mut combined = stdout;
            if !stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push_str("\n[stderr]\n");
                }
                combined.push_str(&stderr);
            }
            // Truncate to keep the model context bounded.
            let max = 8 * 1024;
            if combined.len() > max {
                combined.truncate(max);
                combined.push_str(&format!("\n…[truncated at {max} bytes]"));
            }
            let status = output.status;
            let ok = status.success();
            let text = if combined.is_empty() {
                format!("(exit {})", status.code().unwrap_or(-1))
            } else {
                format!("(exit {})\n{}", status.code().unwrap_or(-1), combined)
            };
            ToolOutcome { ok, summary: text }
        }
        Ok(Err(e)) => ToolOutcome::failed(format!("command failed: {e}")),
        Err(_) => ToolOutcome::failed("command timed out after 60s"),
    }
}

/// List all discovered skills as a compact text summary for the model.
fn exec_skill_list() -> ToolOutcome {
    let skills = liteclaw_skills::discover();
    if skills.is_empty() {
        return ToolOutcome::ok("no skills found");
    }
    let lines: Vec<String> = skills
        .iter()
        .map(|s| {
            let script = if s.is_scripted() { " [scripted]" } else { "" };
            format!(
                "- {}: {}{}",
                s.id,
                s.description.chars().take(80).collect::<String>(),
                script
            )
        })
        .collect();
    ToolOutcome::ok(format!("{} skill(s):\n{}", skills.len(), lines.join("\n")))
}

/// Run a script-based skill by id. Defender scans any string arguments.
async fn exec_skill_run(args: &serde_json::Value, ctx: &Ctx) -> ToolOutcome {
    let id = match args.get("id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return ToolOutcome::failed("missing 'id'"),
    };
    let skills = liteclaw_skills::discover();
    let Some(skill) = liteclaw_skills::find(&skills, id).cloned() else {
        return ToolOutcome::failed(format!("no skill with id '{id}'"));
    };
    if !skill.is_scripted() {
        // Prompt-only skill: return its body so the model can use the knowledge.
        return ToolOutcome::ok(format!(
            "(prompt-only skill, returning body)\n\n{}",
            skill.body
        ));
    }
    let Some(script) = skill.main_script().map(|p| p.to_path_buf()) else {
        return ToolOutcome::failed("skill has no main script");
    };
    // Defender-check any extra arguments.
    if let Some(extra) = args.get("args").and_then(|v| v.as_str()) {
        let report = ctx.guard_text(extra);
        if matches!(report.action, liteclaw_core::defender::Action::Block) {
            return ToolOutcome::blocked(format!(
                "Defender blocked skill arg ({} score {})",
                report.severity.label(),
                report.score
            ));
        }
    }
    // Execute via shebang fallback (same logic as SkillClaw).
    #[cfg(unix)]
    let is_exec = std::fs::metadata(&script)
        .map(|m| {
            use std::os::unix::fs::PermissionsExt;
            m.permissions().mode() & 0o111 != 0
        })
        .unwrap_or(false);
    #[cfg(not(unix))]
    let is_exec = false;

    let mut cmd = if is_exec {
        tokio::process::Command::new(&script)
    } else {
        let interp = read_shebang_interpreter(&script).unwrap_or_else(|| "bash".into());
        let mut c = tokio::process::Command::new(interp);
        c.arg(&script);
        c
    };
    cmd.current_dir(&ctx.cwd);
    if let Some(extra) = args.get("args").and_then(|v| v.as_str()) {
        for a in extra.split_whitespace() {
            cmd.arg(a);
        }
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output()).await;
    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let mut combined = stdout;
            if !stderr.is_empty() {
                combined.push_str("\n[stderr]\n");
                combined.push_str(&stderr);
            }
            let max = 8 * 1024;
            if combined.len() > max {
                combined.truncate(max);
                combined.push_str(&format!("\n…[truncated at {max} bytes]"));
            }
            ToolOutcome {
                ok: output.status.success(),
                summary: format!(
                    "(exit {})\n{}",
                    output.status.code().unwrap_or(-1),
                    combined
                ),
            }
        }
        Ok(Err(e)) => ToolOutcome::failed(format!("skill run failed: {e}")),
        Err(_) => ToolOutcome::failed("skill run timed out after 60s"),
    }
}

/// Read the interpreter from a script's `#!` shebang line.
fn read_shebang_interpreter(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let first_line = bytes.split(|&b| b == b'\n').next()?;
    let s = std::str::from_utf8(first_line).ok()?.trim();
    let s = s.strip_prefix("#!")?;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.contains("/env") {
        return s.split_whitespace().last().map(String::from);
    }
    s.split_whitespace()
        .next()
        .and_then(|t| t.rsplit('/').next().map(String::from))
}

/// JSON Schema for `{ path: string }`.
fn schema_path() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "File or directory path" }
        },
        "required": ["path"]
    })
}

/// Build the default tool set from the registered claws.
///
/// Read-only claws are Auto; write claws are Confirm.
pub fn default_tools(claws: &[Arc<dyn Claw>]) -> Vec<Tool> {
    let find = |name: &str| -> Arc<dyn Claw> {
        claws
            .iter()
            .find(|c| c.name() == name)
            .cloned()
            .unwrap_or_else(|| panic!("missing claw: {name}"))
    };

    let read = find("read");
    let grep = find("grep");
    let audit = find("audit");
    let edit = find("edit");

    vec![
        Tool {
            name: "read",
            description: "Read a file or list a directory contents.",
            parameters: schema_path(),
            approval: Approval::Auto,
            arg_order: &["path"],
            claw: read,
        },
        Tool {
            name: "grep",
            description: "Search file contents for a pattern (respects .gitignore).",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "path": { "type": "string", "description": "defaults to current dir" }
                },
                "required": ["pattern"]
            }),
            approval: Approval::Auto,
            arg_order: &["pattern", "path"],
            claw: grep,
        },
        Tool {
            name: "audit",
            description:
                "Scan a directory for security risks (prompt injection, SSRF, path traversal).",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "defaults to current dir" }
                },
                "required": []
            }),
            approval: Approval::Auto,
            arg_order: &["path"],
            claw: audit,
        },
        Tool {
            name: "edit",
            description: "Replace a unique string in an existing file.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old": { "type": "string", "description": "text to find (must match exactly once)" },
                    "new": { "type": "string", "description": "replacement text" }
                },
                "required": ["path", "old", "new"]
            }),
            approval: Approval::Confirm,
            arg_order: &["path", "old", "new"],
            claw: edit.clone(),
        },
        Tool {
            name: "write",
            description: "Create or overwrite a file with the given content.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "file path to create or overwrite" },
                    "content": { "type": "string", "description": "full file content to write" }
                },
                "required": ["path", "content"]
            }),
            approval: Approval::Confirm,
            arg_order: &["path", "content"],
            claw: edit.clone(), // placeholder: execute() dispatches to exec_write, not this claw
        },
        Tool {
            name: "bash",
            description:
                "Run a shell command (gcc, make, cargo, etc.). Defender blocks dangerous commands.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "shell command to execute" }
                },
                "required": ["command"]
            }),
            approval: Approval::Confirm,
            arg_order: &["command"],
            claw: edit.clone(), // placeholder: execute() dispatches to exec_bash, not this claw
        },
    ]
}

/// Build the skill tool set: skill_list (auto) + skill_run (auto).
/// These let the agent discover and invoke installed skills.
pub fn skill_tools(claw: Arc<dyn Claw>) -> Vec<Tool> {
    vec![
        Tool {
            name: "skill_list",
            description: "List all available skills with their descriptions.",
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
            approval: Approval::Auto,
            arg_order: &[],
            claw: claw.clone(),
        },
        Tool {
            name: "skill_run",
            description: "Run a skill by id. For script-based skills it executes the script; for prompt-based skills it returns the SKILL.md body as knowledge.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "skill id (from skill_list)" },
                    "args": { "type": "string", "description": "optional arguments to pass to the skill" }
                },
                "required": ["id"]
            }),
            approval: Approval::Auto,
            arg_order: &["id", "args"],
            claw: claw.clone(),
        },
    ]
}

/// Build extra tools: glob (file matching) + fetch (web download).
pub fn extra_tools(claw: Arc<dyn Claw>) -> Vec<Tool> {
    vec![
        Tool {
            name: "glob",
            description: "List files matching a glob pattern (e.g. '**/*.rs', 'src/*.ts').",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "glob pattern" },
                    "path": { "type": "string", "description": "root dir (defaults to cwd)" }
                },
                "required": ["pattern"]
            }),
            approval: Approval::Auto,
            arg_order: &["pattern", "path"],
            claw: claw.clone(),
        },
        Tool {
            name: "fetch",
            description: "Download a web page and return text content. SSRF-protected.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to fetch" }
                },
                "required": ["url"]
            }),
            approval: Approval::Auto,
            arg_order: &["url"],
            claw: claw.clone(),
        },
        Tool {
            name: "undo",
            description: "Undo the most recent write/edit: restore the file from backup.",
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
            approval: Approval::Auto,
            arg_order: &[],
            claw: claw.clone(),
        },
    ]
}

/// Convert a slice of tools into OpenAI specs.
pub fn to_specs(tools: &[Tool]) -> Vec<ToolSpec> {
    tools.iter().map(|t| t.to_spec()).collect()
}

/// Find a tool by name.
pub fn find<'a>(tools: &'a [Tool], name: &str) -> Option<&'a Tool> {
    tools.iter().find(|t| t.name == name)
}
