//! liteclaw CLI entry point.
//!
//! Dispatches `lc <claw>` subcommands to registered claws. Global flags
//! (`--json`, `--no-defender`, `--allow-write`) configure the shared [`Ctx`].

mod audit;
mod registry;
mod skills;

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
    /// List discovered skills (global ~/.agents/skills + project .liteclaw/skills).
    Skills,
    /// Show a skill's full SKILL.md by id.
    Skill {
        id: String,
    },
    /// Run a script-based skill by id (passes remaining args to the script).
    ///
    /// Use `lc skill-run <id> -- <args>` (or just `lc skill-run <id> <args>`) to
    /// forward arguments. Known clap flags like --version are intentionally NOT
    /// captured here so they reach the skill script.
    #[command(disable_help_flag = true, disable_version_flag = true)]
    SkillRun {
        id: String,
        #[arg(num_args = 0.., trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// List all available claws.
    Claws,
    /// Start the web UI (chat + agent loop).
    Serve {
        /// Bind address: 127.0.0.1 (local only) or 0.0.0.0 (container/public).
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(default_value = "8080")]
        port: u16,
    },
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
        Command::Skills => {
            return match skills::list(&ctx).await {
                Ok(code) => Ok(code.into()),
                Err(e) => {
                    eprintln!("lc skills: {e:#}");
                    Ok(ExitCode::Failure.into())
                }
            };
        }
        Command::Skill { id } => {
            return match skills::show(id, &ctx).await {
                Ok(code) => Ok(code.into()),
                Err(e) => {
                    eprintln!("lc skill: {e:#}");
                    Ok(ExitCode::Failure.into())
                }
            };
        }
        Command::SkillRun { id, args } => {
            return match skills::run(id, args.clone(), &ctx).await {
                Ok(code) => Ok(code.into()),
                Err(e) => {
                    eprintln!("lc skill run: {e:#}");
                    Ok(ExitCode::Failure.into())
                }
            };
        }
        Command::Claws => {
            for c in registry::all_claws() {
                println!("{:<10} {}", c.name(), c.desc());
            }
            return Ok(StdExitCode::SUCCESS);
        }
        Command::Serve { host, port } => {
            // For the web agent, default to allowing writes under cwd so the
            // edit tool is usable (unless the user passed stricter flags).
            let serve_ctx = if cli.allow_write.is_empty() {
                let cwd2 = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                Ctx::new(cwd2, Sandbox::readonly().allow_write(std::path::PathBuf::from(".")), false, false)
            } else {
                ctx.clone()
            };
            return match liteclaw_web::serve(host, *port, registry::all_claws(), serve_ctx).await {
                Ok(_) => Ok(StdExitCode::SUCCESS),
                Err(e) => {
                    eprintln!("lc serve: {e:#}");
                    Ok(ExitCode::Failure.into())
                }
            };
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
