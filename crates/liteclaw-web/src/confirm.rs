//! Confirmation registry: bridges the agent loop's need for human approval
//! with the frontend's POST /api/confirm response.
//!
//! When the agent wants to run a Confirm tool (write/edit/bash), it creates a
//! pending entry here and awaits. The frontend shows allow/deny buttons; when
//! the user clicks, it POSTs the decision, which resolves the pending entry.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// Shared store of pending confirmations, keyed by confirm id.
#[derive(Clone, Default)]
pub struct ConfirmRegistry {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
}

impl ConfirmRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a pending confirmation. Returns the receiver the agent loop
    /// awaits; the sender is stored until `resolve` is called with the id.
    pub fn register(&self, id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.to_string(), tx);
        rx
    }

    /// Resolve a pending confirmation. Returns true if the id existed.
    pub fn resolve(&self, id: &str, allowed: bool) -> bool {
        if let Some(tx) = self.pending.lock().unwrap().remove(id) {
            let _ = tx.send(allowed);
            true
        } else {
            false
        }
    }
}
