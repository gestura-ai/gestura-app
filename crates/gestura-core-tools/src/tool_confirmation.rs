//! Async pause/resume support for Restricted-mode tool confirmations.
//!
//! The agent pipeline can emit a `StreamChunk::ToolConfirmationRequired` event and
//! then **pause** tool execution until the UI responds (approve/deny).
//!
//! This module provides a small, process-wide confirmation registry that:
//! - registers pending confirmations by id
//! - lets the UI resolve them via Tauri commands
//! - allows the pipeline to await the decision

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::time::Instant;

use lazy_static::lazy_static;
use tokio::sync::oneshot;

/// A scoped user decision for a tool confirmation request.
///
/// This models Claude Code-style confirmation scopes:
/// - allow/deny once (applies to the current tool call only)
/// - allow/deny for session (affects future tool calls in the same session)
/// - allow always (persisted permission, affects future sessions)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolConfirmationDecision {
    /// Allow this tool call only.
    AllowOnce,
    /// Allow this tool call and skip confirmation for this tool for the rest of the session.
    AllowSession,
    /// Allow this tool call and persist an allow rule for future sessions.
    AllowAlways,
    /// Deny this tool call only.
    DenyOnce,
    /// Deny this tool call and block this tool for the rest of the session.
    DenySession,
}

impl ToolConfirmationDecision {
    /// Return `true` if this decision permits executing the tool call.
    pub fn is_allowed(self) -> bool {
        matches!(
            self,
            Self::AllowOnce | Self::AllowSession | Self::AllowAlways
        )
    }

    /// Return a stable string representation for UI / CLI interop.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowSession => "allow_session",
            Self::AllowAlways => "allow_always",
            Self::DenyOnce => "deny_once",
            Self::DenySession => "deny_session",
        }
    }

    /// Parse a user-supplied decision string.
    ///
    /// This accepts a small set of aliases to make UI wiring ergonomic.
    pub fn parse(input: &str) -> Result<Self, String> {
        let normalized = input.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "allow" | "allow_once" | "once_allow" => Ok(Self::AllowOnce),
            "allow_session" | "session_allow" => Ok(Self::AllowSession),
            "allow_always" | "always_allow" | "allow_forever" => Ok(Self::AllowAlways),
            "deny" | "deny_once" | "once_deny" => Ok(Self::DenyOnce),
            "deny_session" | "session_deny" | "block_session" => Ok(Self::DenySession),
            other => Err(format!(
                "Unknown tool confirmation decision '{other}'. Expected one of: allow_once, allow_session, allow_always, deny_once, deny_session"
            )),
        }
    }
}

impl From<bool> for ToolConfirmationDecision {
    fn from(value: bool) -> Self {
        if value {
            Self::AllowOnce
        } else {
            Self::DenyOnce
        }
    }
}

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
    sender: oneshot::Sender<ToolConfirmationDecision>,
}

/// A registry for in-flight tool confirmations.
///
/// This is intentionally small and simple: it supports approve/deny once.
/// (Session/always remember decisions can be layered on later.)
#[derive(Debug, Default)]
pub struct ToolConfirmationManager {
    pending: RwLock<HashMap<String, PendingToolConfirmation>>,
    session_confirmed: RwLock<HashMap<String, HashSet<String>>>,
    session_blocked: RwLock<HashMap<String, HashSet<String>>>,
}

impl ToolConfirmationManager {
    /// Create a new empty confirmation manager.
    pub fn new() -> Self {
        Self {
            pending: RwLock::new(HashMap::new()),
            session_confirmed: RwLock::new(HashMap::new()),
            session_blocked: RwLock::new(HashMap::new()),
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
    ) -> oneshot::Receiver<ToolConfirmationDecision> {
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
        self.resolve_decision(
            confirmation_id,
            session_id,
            ToolConfirmationDecision::from(approved),
        )
    }

    /// Resolve a pending confirmation with a scoped decision.
    ///
    /// Returns an error if the confirmation id is unknown, or if a session id is
    /// required and does not match.
    pub fn resolve_decision(
        &self,
        confirmation_id: &str,
        session_id: Option<&str>,
        decision: ToolConfirmationDecision,
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
        let _ = pending.sender.send(decision);
        Ok(())
    }

    /// Apply a scoped decision to session-level tool policy caches.
    ///
    /// This enables "Allow for session" and "Deny for session" semantics to be
    /// respected by the pipeline when processing later tool calls.
    pub fn apply_session_policy_decision(
        &self,
        session_id: &str,
        tool_name: &str,
        decision: ToolConfirmationDecision,
    ) {
        match decision {
            ToolConfirmationDecision::AllowSession | ToolConfirmationDecision::AllowAlways => {
                if let Ok(mut map) = self.session_confirmed.write() {
                    map.entry(session_id.to_string())
                        .or_default()
                        .insert(tool_name.to_string());
                }
                // An allow should override a prior session block for the same tool.
                if let Ok(mut map) = self.session_blocked.write()
                    && let Some(set) = map.get_mut(session_id)
                {
                    set.remove(tool_name);
                }
            }
            ToolConfirmationDecision::DenySession => {
                if let Ok(mut map) = self.session_blocked.write() {
                    map.entry(session_id.to_string())
                        .or_default()
                        .insert(tool_name.to_string());
                }
                // A deny-session should override any previous allow-session.
                if let Ok(mut map) = self.session_confirmed.write()
                    && let Some(set) = map.get_mut(session_id)
                {
                    set.remove(tool_name);
                }
            }
            ToolConfirmationDecision::AllowOnce | ToolConfirmationDecision::DenyOnce => {}
        }
    }

    /// Return `true` if the tool has been allowed for this session.
    pub fn is_tool_allowed_for_session(&self, session_id: &str, tool_name: &str) -> bool {
        self.session_confirmed
            .read()
            .ok()
            .and_then(|m| m.get(session_id).cloned())
            .is_some_and(|set| set.contains(tool_name))
    }

    /// Return `true` if the tool has been blocked for this session.
    pub fn is_tool_blocked_for_session(&self, session_id: &str, tool_name: &str) -> bool {
        self.session_blocked
            .read()
            .ok()
            .and_then(|m| m.get(session_id).cloned())
            .is_some_and(|set| set.contains(tool_name))
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
        assert!(rx.await.unwrap().is_allowed());
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

    #[test]
    fn session_policy_is_recorded() {
        let mgr = ToolConfirmationManager::new();
        mgr.apply_session_policy_decision("s1", "file", ToolConfirmationDecision::AllowSession);
        assert!(mgr.is_tool_allowed_for_session("s1", "file"));
        assert!(!mgr.is_tool_blocked_for_session("s1", "file"));

        mgr.apply_session_policy_decision("s1", "file", ToolConfirmationDecision::DenySession);
        assert!(!mgr.is_tool_allowed_for_session("s1", "file"));
        assert!(mgr.is_tool_blocked_for_session("s1", "file"));
    }
}
