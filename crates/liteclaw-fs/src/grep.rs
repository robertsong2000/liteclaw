//! `lc grep` — search file contents with ripgrep-grade ignore handling.
//!
//! Uses the `ignore` crate (the same library ripgrep is built on) so that
//! `.gitignore` / `.liteclawignore` rules are respected automatically. Pattern
//! matching uses the `regex` crate (case-insensitive).

use async_trait::async_trait;
use liteclaw_core::{Claw, ClawArgs, Ctx, ExitCode};
use regex::RegexBuilder;

/// The `grep` claw — content search across a directory tree.
pub struct GrepClaw;

#[async_trait]
impl Claw for GrepClaw {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn desc(&self) -> &'static str {
        "Search file contents (respects .gitignore)"
    }

    async fn run(&self, args: &ClawArgs, ctx: &Ctx) -> anyhow::Result<ExitCode> {
        if args.positionals.is_empty() {
            eprintln!("usage: lc grep <pattern> [path]");
            return Ok(ExitCode::Failure);
        }
        let pattern = &args.positionals[0];
        let root = args
            .positionals
            .get(1)
            .map(|p| ctx.cwd.join(p))
            .unwrap_or_else(|| ctx.cwd.clone());

        // Defender-check the pattern itself (e.g. flag traversal payloads).
        let report = ctx.guard_text(pattern);
        if !report.is_clean() {
            eprintln!(
                "⚠️  Defender: pattern flagged as {} (score {})",
                report.severity.label(),
                report.score
            );
        }

        let re = match RegexBuilder::new(pattern).case_insensitive(true).build() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("lc grep: invalid pattern: {e}");
                return Ok(ExitCode::Failure);
            }
        };

        let walker = ignore::WalkBuilder::new(&root)
            .hidden(false)
            .git_ignore(true)
            .add_custom_ignore_filename(".liteclawignore")
            .build();

        let mut found_any = false;
        let mut matches: Vec<serde_json::Value> = Vec::new();

        for entry in walker.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            for (lineno, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    found_any = true;
                    if ctx.json {
                        matches.push(serde_json::json!({
                            "path": path.display().to_string(),
                            "line": lineno + 1,
                            "text": line,
                        }));
                    } else {
                        println!("{}:{}:{}", path.display(), lineno + 1, line);
                    }
                }
            }
        }

        if ctx.json {
            println!("{}", serde_json::json!({ "matches": matches }));
        }
        Ok(if found_any {
            ExitCode::Success
        } else {
            ExitCode::Failure
        })
    }
}
