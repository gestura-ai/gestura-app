//! Session-specific checkpoint (rewind) helpers.
//!
//! This layer builds on the generic [`crate::checkpoints`] storage primitives to
//! snapshot and restore a chat session together with related state (tasks + a
//! **redacted** subset of configuration).

use serde::{Deserialize, Serialize};

use crate::chat_sessions::{ChatSession, ChatSessionStore};
use crate::config::{AppConfig, GlobalPermissionSettings, PipelineSettings};
use crate::hooks::HooksSettings;
use crate::tasks::{TaskList, TaskManager};

use super::CheckpointManager;
use super::types::{CheckpointError, CheckpointId, CheckpointMetadata, CheckpointSnapshot};

/// Schema version used for [`SessionCheckpointPayload`].
pub const SESSION_CHECKPOINT_SCHEMA_V1: u32 = 1;

/// A **redacted** subset of configuration captured in a session checkpoint.
///
/// This intentionally excludes secrets such as API keys.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCheckpointConfig {
    /// Pipeline behavior relevant to prompt/history management.
    pub pipeline: PipelineSettings,

    /// Default permission behavior for new sessions.
    pub permissions: GlobalPermissionSettings,

    /// Global primary LLM provider id (e.g., "openai", "anthropic", "ollama").
    pub llm_primary_provider: String,

    /// Optional global fallback LLM provider id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_fallback_provider: Option<String>,

    /// Optional global model selector for OpenAI (no secrets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_model: Option<String>,

    /// Optional global model selector for Anthropic (no secrets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_model: Option<String>,

    /// Optional global model selector for Grok (no secrets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grok_model: Option<String>,

    /// Optional global model selector for Ollama (no secrets).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama_model: Option<String>,

    /// Hooks configuration (no secrets).
    ///
    /// This is safe to capture because hook execution is still governed by
    /// explicit allow-listing of programs and hooks are disabled by default.
    #[serde(default)]
    pub hooks: HooksSettings,
}

impl SessionCheckpointConfig {
    /// Build a checkpoint-safe config snapshot from the full application config.
    ///
    /// This method **must not** include secrets (API keys).
    pub fn from_app_config(config: &AppConfig) -> Self {
        Self {
            pipeline: config.pipeline.clone(),
            permissions: config.permissions.clone(),
            llm_primary_provider: config.llm.primary.clone(),
            llm_fallback_provider: config.llm.fallback.clone(),
            openai_model: config.llm.openai.as_ref().map(|c| c.model.clone()),
            anthropic_model: config.llm.anthropic.as_ref().map(|c| c.model.clone()),
            grok_model: config.llm.grok.as_ref().map(|c| c.model.clone()),
            ollama_model: config.llm.ollama.as_ref().map(|c| c.model.clone()),
            hooks: config.hooks.clone(),
        }
    }
}

/// Typed payload stored inside a session checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpointPayload {
    /// Payload schema version.
    pub schema_version: u32,

    /// The chat session state (messages, tool calls, session overrides).
    pub session: ChatSession,

    /// The task list for this session, including the persisted `current_task_id` pointer.
    pub tasks: TaskList,

    /// Redacted configuration snapshot.
    pub config: SessionCheckpointConfig,
}

impl SessionCheckpointPayload {
    /// Validate basic invariants for this payload.
    pub fn validate(&self) -> Result<(), CheckpointError> {
        if self.schema_version != SESSION_CHECKPOINT_SCHEMA_V1 {
            return Err(CheckpointError::UnsupportedSchema {
                expected: SESSION_CHECKPOINT_SCHEMA_V1,
                found: self.schema_version,
            });
        }

        if self.session.id != self.tasks.session_id {
            return Err(CheckpointError::InvalidInput(format!(
                "checkpoint payload session/task mismatch: session.id='{}' tasks.session_id='{}'",
                self.session.id, self.tasks.session_id
            )));
        }

        Ok(())
    }
}

impl CheckpointManager {
    /// Create a checkpoint for a specific session.
    ///
    /// This snapshots:
    /// - chat session state (history + session overrides)
    /// - task list state (including `current_task_id`)
    /// - a redacted subset of global configuration
    pub fn create_session_checkpoint(
        &self,
        session_id: &str,
        session_store: &dyn ChatSessionStore,
        task_manager: &TaskManager,
        config: &AppConfig,
        label: Option<String>,
    ) -> Result<CheckpointMetadata, CheckpointError> {
        let session = session_store
            .load(session_id)
            .map_err(|e| CheckpointError::ChatSession(e.to_string()))?;

        let tasks = task_manager
            .load_task_list(session_id)
            .map_err(|e| CheckpointError::Tasks(e.to_string()))?;

        let payload = SessionCheckpointPayload {
            schema_version: SESSION_CHECKPOINT_SCHEMA_V1,
            session,
            tasks,
            config: SessionCheckpointConfig::from_app_config(config),
        };
        payload.validate()?;

        let snapshot = CheckpointSnapshot {
            session_id: Some(session_id.to_string()),
            payload: serde_json::to_value(payload)?,
        };

        self.create_checkpoint(snapshot, label)
    }

    /// Restore a session checkpoint payload without applying it.
    pub fn restore_session_checkpoint(
        &self,
        id: &CheckpointId,
    ) -> Result<SessionCheckpointPayload, CheckpointError> {
        let snapshot = self.restore_checkpoint(id)?;
        let payload: SessionCheckpointPayload = serde_json::from_value(snapshot.payload)?;
        payload.validate()?;
        Ok(payload)
    }

    /// Apply a session checkpoint by writing the restored state back to persistence.
    ///
    /// Returns the restored payload (useful for callers that want to update UI state).
    pub fn apply_session_checkpoint(
        &self,
        id: &CheckpointId,
        session_store: &dyn ChatSessionStore,
        task_manager: &TaskManager,
    ) -> Result<SessionCheckpointPayload, CheckpointError> {
        let payload = self.restore_session_checkpoint(id)?;

        session_store
            .save(&payload.session)
            .map_err(|e| CheckpointError::ChatSession(e.to_string()))?;
        task_manager
            .replace_task_list(payload.tasks.clone())
            .map_err(|e| CheckpointError::Tasks(e.to_string()))?;

        Ok(payload)
    }

    /// List checkpoints belonging to a particular session.
    pub fn list_session_checkpoints(
        &self,
        session_id: &str,
    ) -> Result<Vec<CheckpointMetadata>, CheckpointError> {
        let all = self.list_checkpoints()?;
        Ok(all
            .into_iter()
            .filter(|m| m.session_id.as_deref() == Some(session_id))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::chat_sessions::{FileChatSessionStore, MessageSource};
    use crate::checkpoints::{CheckpointRetentionPolicy, FileCheckpointStore};
    use tempfile::tempdir;

    #[test]
    fn create_and_apply_session_checkpoint_restores_session_and_tasks() {
        let temp = tempdir().unwrap();

        let sessions_dir = temp.path().join("sessions");
        let session_store = FileChatSessionStore::new(sessions_dir);

        let checkpoint_store = FileCheckpointStore::new(temp.path().join("checkpoints"));
        let manager =
            CheckpointManager::new(checkpoint_store, CheckpointRetentionPolicy::default());

        let task_manager = TaskManager::new(temp.path());
        let config = AppConfig::default();

        // Create a session.
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let mut session =
            ChatSession::new_with_workspace(workspace_dir, Some("m".to_string())).unwrap();
        session.add_user_message("hello", MessageSource::Text);
        session_store.save(&session).unwrap();

        // Create tasks + set current pointer.
        let t = task_manager
            .create_task(&session.id, "Task", "Desc", None)
            .unwrap();
        task_manager
            .set_current_task_id(&session.id, Some(t.id.clone()))
            .unwrap();

        // Snapshot.
        let meta = manager
            .create_session_checkpoint(
                &session.id,
                &session_store,
                &task_manager,
                &config,
                Some("before-change".to_string()),
            )
            .unwrap();

        // Mutate persisted state.
        let mut mutated = session_store.load(&session.id).unwrap();
        mutated.add_user_message("later", MessageSource::Text);
        session_store.save(&mutated).unwrap();

        let t2 = task_manager
            .create_task(&session.id, "Task2", "Desc2", None)
            .unwrap();
        task_manager
            .set_current_task_id(&session.id, Some(t2.id.clone()))
            .unwrap();

        // Apply checkpoint.
        let applied = manager
            .apply_session_checkpoint(&meta.id, &session_store, &task_manager)
            .unwrap();

        // Verify session rewound.
        let rewound = session_store.load(&session.id).unwrap();
        assert_eq!(rewound.message_count(), 1);
        assert_eq!(rewound.state.messages[0].content, "hello");

        // Verify tasks rewound.
        let loaded_tasks = task_manager.load_task_list(&session.id).unwrap();
        assert_eq!(loaded_tasks.tasks.len(), 1);
        assert_eq!(loaded_tasks.current_task_id(), Some(t.id.as_str()));

        // Verify config included and schema is valid.
        assert_eq!(applied.schema_version, SESSION_CHECKPOINT_SCHEMA_V1);
        assert_eq!(applied.config.pipeline, config.pipeline);
        assert_eq!(applied.config.permissions, config.permissions);
    }

    #[test]
    fn retention_deletes_oldest_files() {
        let temp = tempdir().unwrap();

        let session_store = FileChatSessionStore::new(temp.path().join("sessions"));
        let checkpoint_dir = temp.path().join("checkpoints");
        let checkpoint_store = FileCheckpointStore::new(checkpoint_dir.clone());
        let manager = CheckpointManager::new(
            checkpoint_store,
            CheckpointRetentionPolicy { max_checkpoints: 2 },
        );

        let task_manager = TaskManager::new(temp.path());
        let config = AppConfig::default();

        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let session = ChatSession::new_with_workspace(workspace_dir, None).unwrap();
        session_store.save(&session).unwrap();

        manager
            .create_session_checkpoint(&session.id, &session_store, &task_manager, &config, None)
            .unwrap();
        manager
            .create_session_checkpoint(&session.id, &session_store, &task_manager, &config, None)
            .unwrap();
        manager
            .create_session_checkpoint(&session.id, &session_store, &task_manager, &config, None)
            .unwrap();

        let file_count = std::fs::read_dir(&checkpoint_dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .ok()
                    .and_then(|e| {
                        e.path()
                            .extension()
                            .map(|ext| ext == std::ffi::OsStr::new("json"))
                    })
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(file_count, 2);
    }
}
