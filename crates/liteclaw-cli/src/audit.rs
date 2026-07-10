//! `lc audit` — scan a directory tree for security risks.
//!
//! This is the Rust equivalent of `clawdefender --audit`: walk source files,
//! run each through the Defender kernel, and report findings by severity. It is
//! the showcase claw for the "security-first" identity of liteclaw.

use async_trait::async_trait;
use liteclaw_core::{Claw, ClawArgs, Ctx, ExitCode, Severity};
use std::path::Path;

/// File extensions scanned (matches clawdefender's scope).
const SCANNED_EXT: &[&str] = &[
    "md", "sh", "js", "py", "ts", "rs", "json", "yaml", "yml", "toml",
];

/// The `audit` claw — security-scan a directory.
pub struct AuditClaw;

#[async_trait]
impl Claw for AuditClaw {
    fn name(&self) -> &'static str {
        "audit"
    }

    fn desc(&self) -> &'static str {
        "Scan a directory for prompt injection / SSRF / path traversal risks"
    }

    async fn run(&self, args: &ClawArgs, ctx: &Ctx) -> anyhow::Result<ExitCode> {
        let root = args
            .positionals
            .first()
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());

        if !root.is_dir() {
            eprintln!("lc audit: not a directory: {}", root.display());
            return Ok(ExitCode::Failure);
        }

        let walker = ignore::WalkBuilder::new(&root)
            .hidden(true)
            .git_ignore(true)
            .build();

        let mut findings: Vec<serde_json::Value> = Vec::new();
        let mut max_severity = Severity::Clean;
        let mut any_block = false;

        for entry in walker.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            if !is_scannable(path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let report = liteclaw_core::scan_text(&text);
            if report.is_clean() {
                if !ctx.json {
                    println!("✓ {}", path.display());
                }
                continue;
            }
            max_severity = max_severity.max(report.severity);
            if matches!(report.action, liteclaw_core::defender::Action::Block) {
                any_block = true;
            }
            for f in &report.findings {
                let symbol = severity_symbol(f.severity);
                if ctx.json {
                    findings.push(serde_json::json!({
                        "path": path.display().to_string(),
                        "module": f.module.label(),
                        "severity": f.severity.label(),
                        "score": f.severity.score(),
                        "matched": f.matched,
                    }));
                } else {
                    println!(
                        "{} {:?} [{}] {}: score {}",
                        symbol,
                        f.severity,
                        f.module.label(),
                        path.display(),
                        f.severity.score()
                    );
                }
            }
        }

        if ctx.json {
            println!(
                "{}",
                serde_json::json!({
                    "max_severity": max_severity.label(),
                    "blocked": any_block,
                    "findings": findings,
                })
            );
        } else {
            eprintln!(
                "— max severity: {} (blocked: {})",
                max_severity.label(),
                any_block
            );
        }

        Ok(if any_block {
            ExitCode::Failure
        } else {
            ExitCode::Success
        })
    }
}

fn is_scannable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SCANNED_EXT.contains(&e))
        .unwrap_or(false)
}

fn severity_symbol(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "🔴",
        Severity::High => "🟠",
        Severity::Warning => "🟡",
        Severity::Clean => "✅",
    }
}
