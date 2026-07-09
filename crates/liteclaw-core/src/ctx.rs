//! Shared execution context injected into every claw.
//!
//! `Ctx` carries the Defender kernel, sandbox, and the JSON-output flag so that
//! every tool automatically inherits security pre-checks and consistent output
//! formatting without reimplementing them.

use crate::defender::ScanReport;
use crate::sandbox::Sandbox;

/// The runtime context shared by all claws in a single invocation.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// Sandbox governing what filesystem/network access is permitted.
    pub sandbox: Sandbox,
    /// When true, claws emit JSON to stdout instead of human-readable text.
    pub json: bool,
    /// When true, the Defender pre-check is skipped entirely. Off by default;
    /// intended for trusted inputs / testing only.
    pub no_defender: bool,
    /// Working directory claws operate in. Defaults to process cwd.
    pub cwd: std::path::PathBuf,
}

impl Ctx {
    /// Build a context from the usual global CLI flags.
    pub fn new(cwd: std::path::PathBuf, sandbox: Sandbox, json: bool, no_defender: bool) -> Self {
        Self {
            sandbox,
            json,
            no_defender,
            cwd,
        }
    }

    /// Run the Defender text pre-check unless disabled. Returns the report.
    pub fn guard_text(&self, input: &str) -> ScanReport {
        if self.no_defender {
            return ScanReport::clean();
        }
        crate::defender::scan_text(input)
    }

    /// Run the Defender URL pre-check unless disabled. Returns the report.
    pub fn guard_url(&self, url: &str) -> ScanReport {
        if self.no_defender {
            return ScanReport::clean();
        }
        crate::defender::scan_url(url)
    }
}

impl Default for Ctx {
    fn default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        Self::new(cwd, Sandbox::readonly(), false, false)
    }
}
