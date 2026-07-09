//! Claw registry — collects all built-in claws for dispatch.

use liteclaw_core::Claw;
use liteclaw_fs::{EditClaw, GrepClaw, ReadClaw};
use std::sync::Arc;

/// Build the list of all claws enabled in this build.
///
/// New claws are registered here. Each must implement [`Claw`].
pub fn all_claws() -> Vec<Arc<dyn Claw>> {
    vec![
        Arc::new(ReadClaw),
        Arc::new(GrepClaw),
        Arc::new(EditClaw),
        Arc::new(crate::audit::AuditClaw),
    ]
}
