//! File-backed checkpoint store.

use std::{fs, path::PathBuf};

use super::types::{Checkpoint, CheckpointError, CheckpointId, CheckpointMetadata};

/// Default directory for persisted checkpoints.
///
/// This is intentionally outside sandbox workspaces.
pub fn default_checkpoints_dir() -> PathBuf {
    crate::config::AppConfig::data_dir().join("checkpoints")
}

/// File-backed store (one JSON file per checkpoint).
#[derive(Debug, Clone)]
pub struct FileCheckpointStore {
    dir: PathBuf,
}

impl FileCheckpointStore {
    /// Create a store rooted at a custom directory.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Create a store using the default directory.
    pub fn new_default() -> Self {
        Self::new(default_checkpoints_dir())
    }

    /// Ensure the backing directory exists.
    fn ensure_dir(&self) -> Result<(), CheckpointError> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    /// Compute the on-disk path for a checkpoint JSON file.
    fn path_for(&self, id: &CheckpointId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Persist a checkpoint to disk.
    pub fn save(&self, checkpoint: &Checkpoint) -> Result<(), CheckpointError> {
        self.ensure_dir()?;
        let path = self.path_for(&checkpoint.metadata.id);
        let bytes = serde_json::to_vec_pretty(checkpoint)?;
        fs::write(path, bytes)?;
        Ok(())
    }

    /// Load a checkpoint from disk.
    pub fn load(&self, id: &CheckpointId) -> Result<Checkpoint, CheckpointError> {
        self.ensure_dir()?;
        let path = self.path_for(id);
        let bytes = fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CheckpointError::NotFound(*id)
            } else {
                CheckpointError::Io(e)
            }
        })?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Delete a checkpoint.
    pub fn delete(&self, id: &CheckpointId) -> Result<(), CheckpointError> {
        self.ensure_dir()?;
        let path = self.path_for(id);
        fs::remove_file(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CheckpointError::NotFound(*id)
            } else {
                CheckpointError::Io(e)
            }
        })?;
        Ok(())
    }

    /// List metadata for all checkpoints in the store.
    pub fn list_metadata(&self) -> Result<Vec<CheckpointMetadata>, CheckpointError> {
        self.ensure_dir()?;
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path)?;
            let ckpt: Checkpoint = serde_json::from_slice(&bytes)?;
            out.push(ckpt.metadata);
        }
        Ok(out)
    }
}
