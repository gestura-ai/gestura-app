//! Checkpoints (snapshots) for agent sessions and related state.
//!
//! A checkpoint captures enough state to “rewind” a session in a safe, deterministic way.
//!
//! This module is **core-first**: adapters (CLI/GUI) should call into
//! [`CheckpointManager`] rather than reimplementing storage or retention policies.

pub mod manager;
pub mod session;
pub mod store;
pub mod types;

pub use manager::CheckpointManager;
pub use store::{FileCheckpointStore, default_checkpoints_dir};
pub use types::{
    Checkpoint, CheckpointError, CheckpointId, CheckpointMetadata, CheckpointRetentionPolicy,
    CheckpointSnapshot,
};

pub use session::{
    SESSION_CHECKPOINT_SCHEMA_V1, SessionCheckpointConfig, SessionCheckpointPayload,
};
