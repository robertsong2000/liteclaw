//! liteclaw-core: shared primitives for every "claw" (tool).
//!
//! This crate defines the contract every tool implements ([`Claw`]), the shared
//! execution context ([`Ctx`]) that injects security + sandbox + I/O, the
//! [`defender`] security kernel (a Rust port of clawdefender), and the
//! [`sandbox`] write/network gate.

pub mod claw;
pub mod ctx;
pub mod defender;
pub mod sandbox;

pub use claw::{Claw, ClawArgs, ExitCode};
pub use ctx::Ctx;
pub use defender::{scan_text, scan_url, Finding, ScanReport, Severity};
pub use sandbox::Sandbox;
