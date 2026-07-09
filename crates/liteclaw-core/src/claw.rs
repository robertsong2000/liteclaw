//! The `Claw` trait — the unified contract for every liteclaw subcommand.
//!
//! Each "claw" is a small, independently usable tool that can also be composed
//! via pipes. The shared [`Ctx`] is injected into every call so that security
//! scanning (Defender), sandboxing, and streaming I/O are built-in rather than
//! reimplemented per tool.

use std::process::ExitCode as StdExitCode;

use crate::ctx::Ctx;
use async_trait::async_trait;

/// Generic argument bag passed to every claw.
///
/// Individual claws pull what they need (file paths, patterns, flags) from here
/// via typed accessors on their own CLI structs; this struct carries the shared
/// pieces that every claw may want: positional args and the raw working path.
#[derive(Debug, Clone, Default)]
pub struct ClawArgs {
    /// Free-form positional arguments after the subcommand.
    pub positionals: Vec<String>,
}

impl ClawArgs {
    /// Create a new arg bag from an iterator of positional strings.
    pub fn new<I: IntoIterator<Item = String>>(positionals: I) -> Self {
        Self {
            positionals: positionals.into_iter().collect(),
        }
    }
}

/// The process exit code a claw reports. Maps to `std::process::ExitCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success,
    Failure,
}

impl From<ExitCode> for StdExitCode {
    fn from(code: ExitCode) -> Self {
        match code {
            ExitCode::Success => StdExitCode::SUCCESS,
            ExitCode::Failure => StdExitCode::FAILURE,
        }
    }
}

/// Every lightweight "claw" (tool / subcommand) implements this trait.
///
/// Tools are registered in the CLI registry and dispatched by name. Because
/// [`Ctx`] is always injected, every claw automatically inherits:
/// - the Defender security pre-check,
/// - the sandbox write/network gate,
/// - streaming stdout handles.
#[async_trait]
pub trait Claw: Send + Sync {
    /// Stable identifier used as the subcommand name (e.g. `read`, `grep`).
    fn name(&self) -> &'static str;

    /// One-line human description, shown in `lc --help`.
    fn desc(&self) -> &'static str;

    /// Execute the claw against the given args and shared context.
    async fn run(&self, args: &ClawArgs, ctx: &Ctx) -> anyhow::Result<ExitCode>;
}
