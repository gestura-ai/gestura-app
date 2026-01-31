//! Checkpoint domain types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Stable identifier for a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CheckpointId(Uuid);

impl CheckpointId {
    /// Create a new random checkpoint identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Return the underlying UUID value.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for CheckpointId {
    /// Generate a default checkpoint id.
    ///
    /// This uses a random UUID (same behavior as [`CheckpointId::new`]).
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CheckpointId {
    /// Format this checkpoint id as a UUID string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Metadata about a checkpoint, excluding the stored snapshot payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointMetadata {
    /// Unique checkpoint id.
    pub id: CheckpointId,

    /// Optional session identifier this checkpoint belongs to.
    ///
    /// This is duplicated from [`CheckpointSnapshot::session_id`] so callers can
    /// filter checkpoints by session without loading the full snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Optional user-facing label.
    pub label: Option<String>,
}

/// Snapshot payload captured by a checkpoint.
///
/// For now this is stored as generic JSON so we can evolve the schema without
/// breaking persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    /// Optional session identifier this checkpoint belongs to.
    pub session_id: Option<String>,
    /// Opaque snapshot payload.
    pub payload: Value,
}

/// A full checkpoint (metadata + snapshot payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Checkpoint metadata.
    pub metadata: CheckpointMetadata,
    /// Snapshot captured at `metadata.created_at`.
    pub snapshot: CheckpointSnapshot,
}

/// Retention policy governing how many checkpoints to keep.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRetentionPolicy {
    /// Maximum number of checkpoints to retain (oldest are deleted first).
    pub max_checkpoints: usize,
}

impl Default for CheckpointRetentionPolicy {
    /// Default retention policy.
    fn default() -> Self {
        Self {
            max_checkpoints: 50,
        }
    }
}

/// Errors produced by checkpoint creation, storage, and retrieval.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    /// I/O error while reading or writing checkpoints.
    #[error("checkpoint I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error.
    #[error("checkpoint JSON error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Invalid user input.
    #[error("invalid checkpoint input: {0}")]
    InvalidInput(String),

    /// Requested checkpoint does not exist.
    #[error("checkpoint not found: {0}")]
    NotFound(CheckpointId),

    /// Chat session persistence error while creating or applying a checkpoint.
    #[error("checkpoint chat session error: {0}")]
    ChatSession(String),

    /// Task persistence error while creating or applying a checkpoint.
    #[error("checkpoint task error: {0}")]
    Tasks(String),

    /// Checkpoint payload schema is not supported by this build.
    #[error("unsupported checkpoint schema version: expected {expected}, found {found}")]
    UnsupportedSchema { expected: u32, found: u32 },
}
