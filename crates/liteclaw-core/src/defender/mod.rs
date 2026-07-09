//! The Defender security kernel — a Rust port of clawdefender.
//!
//! Provides [`scan_text`] and [`scan_url`] pre-checks that every mutating claw
//! runs before acting on untrusted input. Rule tables and the scan engine live
//! in sibling modules.

pub mod engine;
pub mod rules;

pub use engine::{scan_text, scan_url, Action};
pub use rules::{Module, Severity};

/// A single detection hit.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Finding {
    pub module: Module,
    pub severity: Severity,
    /// The regex pattern that fired.
    pub pattern: String,
    /// The substring of the input that matched.
    pub matched: String,
}

/// Aggregated result of a scan.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanReport {
    /// Every individual finding, in rule order.
    pub findings: Vec<Finding>,
    /// Aggregate severity (max-score bucket).
    pub severity: Severity,
    /// Numeric score (max across findings).
    pub score: u32,
    /// Recommended action.
    pub action: Action,
}

impl ScanReport {
    /// A clean report with no findings.
    pub fn clean() -> Self {
        Self {
            findings: Vec::new(),
            severity: Severity::Clean,
            score: 0,
            action: Action::Allow,
        }
    }

    /// True when the input passed cleanly.
    pub fn is_clean(&self) -> bool {
        self.action == Action::Allow
    }

    /// Compact JSON (no findings array) — matches clawdefender's `--json` shape.
    pub fn to_compact_json(&self) -> serde_json::Value {
        serde_json::json!({
            "clean": self.action == Action::Allow,
            "severity": self.severity.label(),
            "score": self.score,
            "action": self.action.label(),
        })
    }
}
