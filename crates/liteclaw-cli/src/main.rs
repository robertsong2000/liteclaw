//! liteclaw CLI entry point.
//!
//! Dispatches `lc <claw>` subcommands to registered claws. Global flags
//! (`--json`, `--no-defender`, `--allow-write`) configure the shared [`Ctx`].

mod audit;
mod registry;

use clap::Parser;
use liteclaw_core::{ClawArgs, Ctx, ExitCode, Sandbox};
use std::path::PathBuf;
use std::process::ExitCode as StdExitCode;

/// liteclaw — a lightweight claw for everyday tasks.
///
/// Each subcommand is a small tool ("claw") that can be used standalone or
/// composed via pipes. Security scanning is built into every claw.
#[derive(Parser, Debug)]
#[command(name = "lc", version, propagate_version = true)]
struct Cli {
    /// Emit JSON to stdout instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,

    /// Disable the Defender security pre-check (use with trusted input only).
    #[arg(long, global = true)]
    no_defender: bool,

    /// Grant write access to a directory (repeatable). Default: read-only.
    #[arg(long = "allow-write", value_name = "DIR", global = true)]
    allow_write: Vec<PathBuf>,

    /// The claw to run.
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Read a file (or list a directory) with line numbers and safety guards.
    Read { path: String },
    /// Search file contents (respects .gitignore).
    Grep {
        pattern: String,
        #[arg(default_value = ".")]
        path: String,
    },
    /// Replace a unique string in a file (exact match).
    Edit {
        path: String,
        old: String,
        new: String,
    },
    /// Scan a directory for prompt injection / SSRF / path traversal risks.
    Audit {
        #[arg(default_value = ".")]
        path: String,
    },
    /// List all available claws.
    Claws,
}

#[tokio::main]
async fn main() -> std::io::Result<StdExitCode> {
    let cli = Cli::parse();

    let mut sandbox = Sandbox::readonly();
    for dir in &cli.allow_write {
        sandbox = sandbox.allow_write(dir.clone());
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let ctx = Ctx::new(cwd, sandbox, cli.json, cli.no_defender);

    // Dispatch: map the parsed subcommand to (claw name, positional args).
    let (claw_name, positionals): (&str, Vec<String>) = match &cli.command {
        Command::Read { path } => ("read", vec![path.clone()]),
        Command::Grep { pattern, path } => ("grep", vec![pattern.clone(), path.clone()]),
        Command::Edit { path, old, new } => ("edit", vec![path.clone(), old.clone(), new.clone()]),
        Command::Audit { path } => ("audit", vec![path.clone()]),
        Command::Claws => {
            for c in registry::all_claws() {
                println!("{:<8} {}", c.name(), c.desc());
            }
            return Ok(StdExitCode::SUCCESS);
        }
    };

    let claws = registry::all_claws();
    let Some(claw) = claws.iter().find(|c| c.name() == claw_name) else {
        eprintln!("lc: unknown claw '{claw_name}'");
        return Ok(StdExitCode::FAILURE);
    };

    let args = ClawArgs::new(positionals);
    match claw.run(&args, &ctx).await {
        Ok(code) => Ok(code.into()),
        Err(e) => {
            eprintln!("lc {claw_name}: {e:#}");
            Ok(ExitCode::Failure.into())
        }
    }
}
