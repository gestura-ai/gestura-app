//! Async pause/resume support for Restricted-mode tool confirmations.
//!
//! The agent pipeline can emit a `StreamChunk::ToolConfirmationRequired` event and
//! then **pause** tool execution until the UI responds (approve/deny).
//!
//! This module provides a small, process-wide confirmation registry that:
//! - registers pending confirmations by id
//! - lets the UI resolve them via Tauri commands
//! - allows the pipeline to await the decision

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

use lazy_static::lazy_static;
use tokio::sync::oneshot;

/// A pending tool confirmation awaiting a user decision.
#[derive(Debug)]
pub struct PendingToolConfirmation {
    /// Optional session id used to prevent cross-session confirmation.
    pub session_id: Option<String>,
    /// Tool name for debugging/observability.
    pub tool_name: String,
    /// Tool args (JSON string) for debugging/observability.
    pub tool_args: String,
    /// When this confirmation was registered.
    pub created_at: Instant,
    /// Decision channel.
    sender: oneshot::Sender<bool>,
}

/// A registry for in-flight tool confirmations.
///
/// This is intentionally small and simple: it supports approve/deny once.
/// (Session/always remember decisions can be layered on later.)
#[derive(Debug, Default)]
pub struct ToolConfirmationManager {
    pending: RwLock<HashMap<String, PendingToolConfirmation>>,
}

impl ToolConfirmationManager {
    /// Create a new empty confirmation manager.
    pub fn new() -> Self {
        Self {
            pending: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new confirmation request and return a receiver for the decision.
    ///
    /// If a confirmation with the same id already exists, it is replaced.
    pub fn register(
        &self,
        confirmation_id: String,
        session_id: Option<String>,
        tool_name: String,
        tool_args: String,
    ) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        let pending = PendingToolConfirmation {
            session_id,
            tool_name,
            tool_args,
            created_at: Instant::now(),
            sender: tx,
        };

        if let Ok(mut map) = self.pending.write() {
            map.insert(confirmation_id, pending);
        }
        rx
    }

    /// Resolve a pending confirmation.
    ///
    /// Returns an error if the confirmation id is unknown, or if a session id is
    /// required and does not match.
    pub fn resolve(
        &self,
        confirmation_id: &str,
        session_id: Option<&str>,
        approved: bool,
    ) -> Result<(), String> {
        let pending = {
            let mut map = self
                .pending
                .write()
                .map_err(|_| "tool confirmation manager poisoned".to_string())?;
            map.remove(confirmation_id)
        }
        .ok_or_else(|| format!("Unknown confirmation id: {confirmation_id}"))?;

        if let Some(expected) = pending.session_id.as_deref() {
            let got = session_id.ok_or_else(|| {
                format!("Missing session id while resolving confirmation {confirmation_id}")
            })?;
            if expected != got {
                return Err(format!(
                    "Session mismatch for confirmation {confirmation_id}: expected {expected}, got {got}"
                ));
            }
        }

        // If the receiver side already went away (timeout/cancel), treat as success.
        let _ = pending.sender.send(approved);
        Ok(())
    }

    /// Remove a pending confirmation without resolving it.
    ///
    /// This is useful on timeout/cancel to avoid leaking entries.
    pub fn abandon(&self, confirmation_id: &str) {
        if let Ok(mut map) = self.pending.write() {
            map.remove(confirmation_id);
        }
    }

    /// Return the number of currently pending confirmations.
    pub fn pending_count(&self) -> usize {
        self.pending.read().map(|m| m.len()).unwrap_or_default()
    }
}

lazy_static! {
    /// Global confirmation manager used by the agent pipeline and UI commands.
    pub static ref TOOL_CONFIRMATIONS: ToolConfirmationManager = ToolConfirmationManager::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_and_resolve_allows() {
        let mgr = ToolConfirmationManager::new();
        let id = "c1".to_string();
        let rx = mgr.register(
            id.clone(),
            Some("s1".to_string()),
            "shell".to_string(),
            "{}".to_string(),
        );
        mgr.resolve(&id, Some("s1"), true).unwrap();
        assert!(rx.await.unwrap());
        assert_eq!(mgr.pending_count(), 0);
    }

    #[tokio::test]
    async fn resolve_rejects_session_mismatch() {
        let mgr = ToolConfirmationManager::new();
        let id = "c2".to_string();
        let _rx = mgr.register(
            id.clone(),
            Some("s1".to_string()),
            "file".to_string(),
            "{}".to_string(),
        );
        let err = mgr.resolve(&id, Some("s2"), true).unwrap_err();
        assert!(err.contains("Session mismatch"));
    }

    #[tokio::test]
    async fn abandon_removes_pending() {
        let mgr = ToolConfirmationManager::new();
        let id = "c3".to_string();
        let _rx = mgr.register(id.clone(), None, "file".to_string(), "{}".to_string());
        assert_eq!(mgr.pending_count(), 1);
        mgr.abandon(&id);
        assert_eq!(mgr.pending_count(), 0);
    }
}
