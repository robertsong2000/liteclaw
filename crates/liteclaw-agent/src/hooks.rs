//! Hooks lifecycle: pluggable interceptors around tool execution.
//!
//! Hooks run at two points:
//! - `PreToolUse`: BEFORE a tool runs. Can inspect/modify args, or VETO (block)
//!   the call. The Defender security check is now implemented as a PreToolUse hook.
//! - `PostToolUse`: AFTER a tool runs. Can audit, log, or modify the result.
//!
//! This makes the security layer pluggable instead of hardcoded into execute().

use crate::ToolOutcome;
use liteclaw_core::Ctx;

/// Context passed to hooks, carrying everything they need to make a decision.
pub struct HookContext<'a> {
    pub tool_name: &'a str,
    pub args: &'a serde_json::Value,
    pub ctx: &'a Ctx,
}

/// What a PreToolUse hook returns.
pub enum PreToolVerdict {
    /// Allow the tool to proceed (optionally with modified args).
    Allow { modified_args: Option<serde_json::Value> },
    /// Block the tool; the outcome summary explains why.
    Block { reason: String },
}

/// The hook trait. Both Pre and Post are optional (default to no-op).
#[async_trait::async_trait]
pub trait Hook: Send + Sync {
    fn name(&self) -> &'static str;

    /// Called before a tool executes. Default: allow.
    async fn pre_tool_use(&self, _hc: &HookContext<'_>) -> PreToolVerdict {
        PreToolVerdict::Allow { modified_args: None }
    }

    /// Called after a tool executes. Default: passthrough.
    async fn post_tool_use(&self, _hc: &HookContext<'_>, _outcome: &mut ToolOutcome) {}
}

/// A chain of hooks executed in order. First Block verdict wins.
pub struct HookChain {
    hooks: Vec<Box<dyn Hook>>,
}

impl HookChain {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn push(mut self, hook: Box<dyn Hook>) -> Self {
        self.hooks.push(hook);
        self
    }

    /// Run all PreToolUse hooks. Returns Allow (with possibly modified args) or
    /// the first Block verdict.
    pub async fn pre(&self, hc: &HookContext<'_>) -> PreToolVerdict {
        let mut current_args = None;
        for hook in &self.hooks {
            // If a prior hook modified args, reflect that for the next hook.
            let effective_hc = match &current_args {
                Some(_) => HookContext {
                    tool_name: hc.tool_name,
                    args: current_args.as_ref().unwrap(),
                    ctx: hc.ctx,
                },
                None => HookContext {
                    tool_name: hc.tool_name,
                    args: hc.args,
                    ctx: hc.ctx,
                },
            };
            match hook.pre_tool_use(&effective_hc).await {
                PreToolVerdict::Allow { modified_args } => {
                    if let Some(ma) = modified_args {
                        current_args = Some(ma);
                    }
                }
                PreToolVerdict::Block { reason } => {
                    return PreToolVerdict::Block { reason };
                }
            }
        }
        PreToolVerdict::Allow { modified_args: current_args }
    }

    /// Run all PostToolUse hooks in order, letting each mutate the outcome.
    pub async fn post(&self, hc: &HookContext<'_>, outcome: &mut ToolOutcome) {
        for hook in &self.hooks {
            hook.post_tool_use(hc, outcome).await;
        }
    }
}

impl Default for HookChain {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Built-in hooks ──────────────────────────────────────────────────

/// Defender security hook: runs the Defender pre-check as a PreToolUse hook.
/// Blocks if the input matches injection / SSRF / traversal patterns.
pub struct DefenderHook;

#[async_trait::async_trait]
impl Hook for DefenderHook {
    fn name(&self) -> &'static str {
        "defender"
    }

    async fn pre_tool_use(&self, hc: &HookContext<'_>) -> PreToolVerdict {
        let raw = hc.args.to_string();
        let report = hc.ctx.guard_text(&raw);
        if matches!(report.action, liteclaw_core::defender::Action::Block) {
            return PreToolVerdict::Block {
                reason: format!(
                    "Defender blocked ({} score {})",
                    report.severity.label(),
                    report.score
                ),
            };
        }
        // Also check URL for fetch tool specifically.
        if hc.tool_name == "fetch" {
            if let Some(url) = hc.args.get("url").and_then(|v| v.as_str()) {
                let url_report = hc.ctx.guard_url(url);
                if matches!(url_report.action, liteclaw_core::defender::Action::Block) {
                    return PreToolVerdict::Block {
                        reason: format!(
                            "Defender URL blocked ({} score {})",
                            url_report.severity.label(),
                            url_report.score
                        ),
                    };
                }
            }
        }
        PreToolVerdict::Allow { modified_args: None }
    }
}

/// Logging hook: prints tool calls to stderr for debugging (PostToolUse).
pub struct LogHook;

#[async_trait::async_trait]
impl Hook for LogHook {
    fn name(&self) -> &'static str {
        "log"
    }

    async fn post_tool_use(&self, hc: &HookContext<'_>, _outcome: &mut ToolOutcome) {
        eprintln!("[hook:log] {} called", hc.tool_name);
    }
}

/// Build the default hook chain: Defender (pre) + Backup (pre) + Log (post).
pub fn default_hooks() -> HookChain {
    HookChain::new()
        .push(Box::new(DefenderHook))
        .push(Box::new(crate::backup::BackupHook))
        .push(Box::new(LogHook))
}
