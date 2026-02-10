//! Permission primitives shared across Gestura core.

use serde::{Deserialize, Serialize};

/// Permission level for tool execution in a session.
///
/// This determines whether tools require confirmation before execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionLevel {
    /// Read-only access - write operations are blocked.
    Sandbox,
    /// Ask before write operations (default).
    #[default]
    Restricted,
    /// Full access - no confirmation required.
    Full,
}

impl PermissionLevel {
    /// Parse permission level from a string (case-insensitive).
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "sandbox" => Self::Sandbox,
            "restricted" => Self::Restricted,
            "full" => Self::Full,
            _ => Self::default(),
        }
    }

    /// Check if a tool operation is allowed without confirmation.
    pub fn allows_without_confirmation(&self, is_write_operation: bool) -> bool {
        match self {
            Self::Sandbox => !is_write_operation,
            Self::Restricted => !is_write_operation,
            Self::Full => true,
        }
    }

    /// Check if a tool operation is blocked entirely.
    pub fn blocks(&self, is_write_operation: bool) -> bool {
        match self {
            Self::Sandbox => is_write_operation,
            Self::Restricted => false,
            Self::Full => false,
        }
    }

    /// Check if a tool operation requires confirmation.
    pub fn requires_confirmation(&self, is_write_operation: bool) -> bool {
        match self {
            Self::Sandbox => false, // blocked, not confirmable
            Self::Restricted => is_write_operation,
            Self::Full => false,
        }
    }
}
