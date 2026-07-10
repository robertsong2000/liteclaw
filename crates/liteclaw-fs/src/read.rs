//! `lc read` — read a file (or list a directory) with line numbers and
//! safety guards. Serves as the reference implementation for a claw.

use async_trait::async_trait;
use liteclaw_core::{Claw, ClawArgs, Ctx, ExitCode};
use std::path::Path;
use tokio::fs;
use tokio::io::{stdout, AsyncWriteExt};

/// Default cap on a single file's output to keep memory bounded.
const MAX_BYTES: usize = 512 * 1024; // 512 KiB
/// Lines longer than this are truncated with a marker.
const MAX_LINE_LEN: usize = 2000;

/// The `read` claw — read files or list directories.
pub struct ReadClaw;

#[async_trait]
impl Claw for ReadClaw {
    fn name(&self) -> &'static str {
        "read"
    }

    fn desc(&self) -> &'static str {
        "Read a file (or list a directory) with line numbers and safety guards"
    }

    async fn run(&self, args: &ClawArgs, ctx: &Ctx) -> anyhow::Result<ExitCode> {
        let Some(target) = args.positionals.first() else {
            eprintln!("usage: lc read <path>");
            return Ok(ExitCode::Failure);
        };
        let path = ctx.cwd.join(target);

        if path.is_dir() {
            return list_dir(&path, ctx).await;
        }
        if !path.is_file() {
            eprintln!("lc read: not a file: {}", path.display());
            return Ok(ExitCode::Failure);
        }

        read_file(&path, ctx).await
    }
}

async fn read_file(path: &Path, ctx: &Ctx) -> anyhow::Result<ExitCode> {
    let bytes = fs::read(path).await?;
    let truncated = bytes.len() > MAX_BYTES;
    let slice = if truncated {
        &bytes[..MAX_BYTES]
    } else {
        &bytes
    };
    let content = String::from_utf8_lossy(slice).to_string();

    // Defender pre-check on the content. We warn but still print (read is
    // non-mutating); mutating claws hard-block instead.
    let report = ctx.guard_text(&content);

    if ctx.json {
        let mut out = serde_json::json!({
            "path": path.display().to_string(),
            "lines": collect_lines(&content),
            "truncated": truncated,
        });
        if !report.is_clean() {
            out["security"] = serde_json::to_value(&report)?;
        }
        println!("{out}");
    } else {
        if !report.is_clean() {
            eprintln!(
                "⚠️  Defender: {} (score {}, {}) — printing anyway (read-only)",
                report.severity.label(),
                report.score,
                report.action.label()
            );
        }
        let mut out = stdout();
        for (i, line) in content.lines().enumerate() {
            let displayed = truncate_line(line);
            let _ = out
                .write_all(format!("{:>6}\t{}\n", i + 1, displayed).as_bytes())
                .await;
        }
        if truncated {
            eprintln!(
                "… [truncated at {} bytes; file is {} bytes]",
                MAX_BYTES,
                bytes.len()
            );
        }
    }
    Ok(ExitCode::Success)
}

async fn list_dir(path: &Path, ctx: &Ctx) -> anyhow::Result<ExitCode> {
    let mut entries = fs::read_dir(path).await?;
    let mut names: Vec<String> = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().to_string();
        let prefix = if entry.file_type().await?.is_dir() {
            "/"
        } else {
            ""
        };
        names.push(format!("{name}{prefix}"));
    }
    names.sort();

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({ "path": path.display().to_string(), "entries": names })
        );
    } else {
        for n in &names {
            println!("{n}");
        }
    }
    Ok(ExitCode::Success)
}

fn collect_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|l| truncate_line(l).into_owned())
        .collect()
}

fn truncate_line(line: &str) -> std::borrow::Cow<'_, str> {
    if line.chars().count() <= MAX_LINE_LEN {
        std::borrow::Cow::Borrowed(line)
    } else {
        let truncated: String = line.chars().take(MAX_LINE_LEN).collect();
        std::borrow::Cow::Owned(format!("{truncated} …"))
    }
}
