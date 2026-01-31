//! High-level checkpoint APIs.

use chrono::Utc;

use super::{
    store::FileCheckpointStore,
    types::{
        Checkpoint, CheckpointError, CheckpointId, CheckpointMetadata, CheckpointRetentionPolicy,
        CheckpointSnapshot,
    },
};

/// Checkpoint manager responsible for creating, listing, and restoring checkpoints.
#[derive(Debug, Clone)]
pub struct CheckpointManager {
    store: FileCheckpointStore,
    policy: CheckpointRetentionPolicy,
}

impl CheckpointManager {
    /// Create a checkpoint manager.
    pub fn new(store: FileCheckpointStore, policy: CheckpointRetentionPolicy) -> Self {
        Self { store, policy }
    }

    /// Create a manager using default store directory and default policy.
    pub fn new_default() -> Self {
        Self::new(
            FileCheckpointStore::new_default(),
            CheckpointRetentionPolicy::default(),
        )
    }

    /// Create and persist a checkpoint.
    pub fn create_checkpoint(
        &self,
        snapshot: CheckpointSnapshot,
        label: Option<String>,
    ) -> Result<CheckpointMetadata, CheckpointError> {
        let id = CheckpointId::new();
        let metadata = CheckpointMetadata {
            id,
            session_id: snapshot.session_id.clone(),
            created_at: Utc::now(),
            label,
        };
        let checkpoint = Checkpoint {
            metadata: metadata.clone(),
            snapshot,
        };
        self.store.save(&checkpoint)?;
        self.enforce_retention()?;
        Ok(metadata)
    }

    /// Restore a checkpoint snapshot.
    pub fn restore_checkpoint(
        &self,
        id: &CheckpointId,
    ) -> Result<CheckpointSnapshot, CheckpointError> {
        Ok(self.store.load(id)?.snapshot)
    }

    /// List all checkpoints (metadata only), sorted by creation time ascending.
    pub fn list_checkpoints(&self) -> Result<Vec<CheckpointMetadata>, CheckpointError> {
        let mut items = self.store.list_metadata()?;
        items.sort_by_key(|m| m.created_at);
        Ok(items)
    }

    /// Enforce the configured retention policy by deleting the oldest checkpoints first.
    fn enforce_retention(&self) -> Result<(), CheckpointError> {
        if self.policy.max_checkpoints == 0 {
            return Ok(());
        }

        let mut items = self.list_checkpoints()?;
        while items.len() > self.policy.max_checkpoints {
            if let Some(oldest) = items.first() {
                self.store.delete(&oldest.id)?;
            }
            items.remove(0);
        }
        Ok(())
    }
}
