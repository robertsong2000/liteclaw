//! `lc edit` — exact string replacement in a file.
//!
//! Mirrors the "must match exactly one occurrence" contract common to
//! agentic file tools: if `old` matches zero or multiple times, the edit is
//! rejected rather than applied ambiguously. Writes are gated by the sandbox
//! (`--allow-write`) and the Defender pre-checks both `old` and `new`.

use async_trait::async_trait;
use liteclaw_core::{Claw, ClawArgs, Ctx, ExitCode};
use tokio::fs;

/// The `edit` claw — exact, unique-match string replacement.
pub struct EditClaw;

#[async_trait]
impl Claw for EditClaw {
    fn name(&self) -> &'static str {
        "edit"
    }

    fn desc(&self) -> &'static str {
        "Replace a unique string in a file (exact match)"
    }

    async fn run(&self, args: &ClawArgs, ctx: &Ctx) -> anyhow::Result<ExitCode> {
        if args.positionals.len() < 3 {
            eprintln!("usage: lc edit <path> <old> <new>");
            return Ok(ExitCode::Failure);
        }
        let path = ctx.cwd.join(&args.positionals[0]);
        let old = &args.positionals[1];
        let new = &args.positionals[2];

        // Sandbox: must be explicitly allowed for writes.
        if !ctx.sandbox.can_write(&path) {
            eprintln!(
                "lc edit: write denied (not in --allow-write): {}",
                path.display()
            );
            return Ok(ExitCode::Failure);
        }

        // Defender: block if old or new text is dangerous.
        for payload in [old.as_str(), new.as_str()] {
            let report = ctx.guard_text(payload);
            if matches!(report.action, liteclaw_core::defender::Action::Block) {
                eprintln!(
                    "⛔ Defender: edit blocked — {} (score {})",
                    report.severity.label(),
                    report.score
                );
                return Ok(ExitCode::Failure);
            }
        }

        let content = match fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("lc edit: cannot read {}: {e}", path.display());
                return Ok(ExitCode::Failure);
            }
        };

        let count = content.matches(old).count();
        match count {
            0 => {
                eprintln!("lc edit: no match for `old` in {}", path.display());
                Ok(ExitCode::Failure)
            }
            1 => {
                let updated = content.replacen(old, new, 1);
                fs::write(&path, updated).await?;
                if ctx.json {
                    println!(
                        "{}",
                        serde_json::json!({ "path": path.display().to_string(), "status": "ok" })
                    );
                } else {
                    println!("✓ edited {}", path.display());
                }
                Ok(ExitCode::Success)
            }
            n => {
                eprintln!(
                    "lc edit: `old` matches {n} times in {}; refusing ambiguous edit",
                    path.display()
                );
                Ok(ExitCode::Failure)
            }
        }
    }
}
