//! Legacy GUI session migration + sanitization.
//!
//! The GUI previously persisted all sessions in a single JSON file at:
//! `~/.gestura/gui_sessions.json`.
//!
//! As part of the Core-First migration, session persistence is unified under
//! [`super::FileChatSessionStore`], which stores one JSON file per session in
//! `~/.gestura/chat_sessions/`.
//!
//! This module keeps the **business logic** for:
//! - locating the legacy file
//! - migrating legacy sessions into the core store
//! - sanitizing persisted session overrides

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{ChatSession, ChatSessionStore, SessionState};

/// Returns the Gestura data directory (`~/.gestura/`).
fn gestura_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".gestura")
}

/// Returns the path for the legacy GUI session history file: `~/.gestura/gui_sessions.json`.
pub fn legacy_gui_sessions_file_path() -> PathBuf {
    gestura_data_dir().join("gui_sessions.json")
}

/// Sanitize potentially invalid persisted (provider, model) overrides.
///
/// This is defensive against historic bug states such as `provider=openai` with a non-OpenAI
/// model value.
///
/// The `is_model_compatible` function is injected by the caller to avoid coupling this
/// crate to a specific validation implementation.
///
/// Returns `true` if any repair was applied.
pub fn sanitize_session_llm_override(
    session_id: &str,
    state: &mut SessionState,
    global_llm_provider: &str,
    is_model_compatible: impl Fn(&str, &str) -> bool,
) -> bool {
    let Some(ref mut llm_cfg) = state.llm_config else {
        return false;
    };

    let mut repaired = false;
    if let Some(ref model) = llm_cfg.model {
        let effective_provider = llm_cfg.provider.as_deref().unwrap_or(global_llm_provider);
        if !is_model_compatible(effective_provider, model) {
            tracing::warn!(
                session_id = %session_id,
                provider = %effective_provider,
                model = %model,
                "Clearing incompatible persisted session LLM model override"
            );
            llm_cfg.model = None;
            repaired = true;
        }
    }

    // If both fields are now empty, clear the override container.
    if llm_cfg.provider.is_none() && llm_cfg.model.is_none() {
        state.llm_config = None;
    }

    repaired
}

/// One-time migration from the legacy `gui_sessions.json` file into the unified core store.
///
/// Uses [`legacy_gui_sessions_file_path`] and delegates to
/// [`migrate_legacy_gui_sessions_to_core_at_path`].
///
/// Returns the migrated core sessions if migration succeeded (even partially).
pub fn migrate_legacy_gui_sessions_to_core<S: ChatSessionStore>(
    store: &S,
    global_llm_provider: &str,
    is_model_compatible: impl Fn(&str, &str) -> bool,
) -> Vec<ChatSession> {
    migrate_legacy_gui_sessions_to_core_at_path(
        store,
        global_llm_provider,
        &legacy_gui_sessions_file_path(),
        &is_model_compatible,
    )
}

/// One-time migration from a legacy GUI sessions file into the unified core store.
///
/// Returns the migrated core sessions if migration succeeded (even partially).
pub fn migrate_legacy_gui_sessions_to_core_at_path<S: ChatSessionStore>(
    store: &S,
    global_llm_provider: &str,
    path: &Path,
    is_model_compatible: &dyn Fn(&str, &str) -> bool,
) -> Vec<ChatSession> {
    if !path.exists() {
        return Vec::new();
    }

    let mut migrated = Vec::new();
    match std::fs::read_to_string(path) {
        Ok(json) => match serde_json::from_str::<LegacyPersistedSessions>(&json) {
            Ok(persisted) => {
                for mut session in persisted.sessions {
                    // Windows don't survive app restart.
                    session.is_open = false;
                    session.window_label = None;
                    session.message_count = session.state.messages.len();

                    // Sanitize LLM overrides before persisting.
                    let _ = sanitize_session_llm_override(
                        &session.id,
                        &mut session.state,
                        global_llm_provider,
                        is_model_compatible,
                    );

                    let model = session
                        .state
                        .llm_config
                        .as_ref()
                        .and_then(|cfg| cfg.model.clone());
                    let core_session = ChatSession {
                        id: session.id,
                        title: session.title,
                        created_at: session.created_at,
                        last_active: session.last_active,
                        model,
                        state: session.state,
                    };

                    match store.save(&core_session) {
                        Ok(()) => migrated.push(core_session),
                        Err(e) => tracing::warn!(
                            session_id = %core_session.id,
                            error = %e,
                            "Failed to migrate legacy GUI session to core store"
                        ),
                    }
                }

                // Best-effort cleanup: remove legacy file after migration.
                if let Err(e) = std::fs::remove_file(path) {
                    tracing::debug!(
                        error = %e,
                        path = %path.display(),
                        "Failed to remove legacy sessions file"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "Failed to parse legacy GUI sessions file"
                );
            }
        },
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "Failed to read legacy GUI sessions file"
            );
        }
    }

    migrated
}

/// Legacy persisted session data container (single JSON file).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct LegacyPersistedSessions {
    sessions: Vec<LegacyGuiChatSession>,
    version: u32,
}

/// Legacy GUI session view-model as persisted historically by the desktop app.
///
/// Note: only used for reading the legacy file; new persistence uses core [`ChatSession`].
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct LegacyGuiChatSession {
    id: String,
    title: String,
    created_at: DateTime<Utc>,
    last_active: DateTime<Utc>,
    is_open: bool,
    window_label: Option<String>,
    message_count: usize,
    #[serde(default)]
    state: SessionState,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_sessions::FileChatSessionStore;

    /// Stub validation function for tests.
    fn always_incompatible(_provider: &str, _model: &str) -> bool {
        false
    }

    /// Stub validation function that always reports compatible.
    fn always_compatible(_provider: &str, _model: &str) -> bool {
        true
    }

    #[test]
    fn sanitize_clears_incompatible_model_override_and_prunes_empty_container() {
        let mut state = SessionState {
            llm_config: Some(crate::chat_sessions::SessionLlmConfig {
                provider: Some("openai".to_string()),
                model: Some("claude-3-5-sonnet-20241022".to_string()),
            }),
            ..Default::default()
        };

        let repaired =
            sanitize_session_llm_override("s1", &mut state, "openai", always_incompatible);
        assert!(repaired);
        assert_eq!(
            state.llm_config.as_ref().unwrap().provider.as_deref(),
            Some("openai")
        );
        assert_eq!(state.llm_config.as_ref().unwrap().model, None);

        // Now clear provider too; container should be pruned.
        state.llm_config = Some(crate::chat_sessions::SessionLlmConfig {
            provider: None,
            model: Some("gpt-4o".to_string()),
        });
        let repaired =
            sanitize_session_llm_override("s2", &mut state, "anthropic", always_incompatible);
        assert!(repaired);
        assert!(state.llm_config.is_none());
    }

    #[test]
    fn migrate_returns_empty_when_legacy_file_missing() {
        let temp = tempfile::tempdir().unwrap();
        let store = FileChatSessionStore::new(temp.path().join("store"));
        let missing = temp.path().join("gui_sessions.json");

        let migrated = migrate_legacy_gui_sessions_to_core_at_path(
            &store,
            "openai",
            &missing,
            &always_compatible,
        );
        assert!(migrated.is_empty());
    }

    #[test]
    fn migrate_saves_sessions_and_removes_legacy_file() {
        let temp = tempfile::tempdir().unwrap();
        let store_dir = temp.path().join("store");
        let store = FileChatSessionStore::new(store_dir);
        let legacy_path = temp.path().join("gui_sessions.json");

        let legacy = LegacyPersistedSessions {
            sessions: vec![LegacyGuiChatSession {
                id: "abc".to_string(),
                title: "Hello".to_string(),
                created_at: Utc::now(),
                last_active: Utc::now(),
                is_open: true,
                window_label: Some("chat-abc".to_string()),
                message_count: 123,
                state: SessionState::default(),
            }],
            version: 1,
        };

        std::fs::write(&legacy_path, serde_json::to_string(&legacy).unwrap()).unwrap();

        let migrated = migrate_legacy_gui_sessions_to_core_at_path(
            &store,
            "openai",
            &legacy_path,
            &always_compatible,
        );
        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0].id, "abc");
        assert!(!legacy_path.exists());

        // Confirm it was persisted into the store.
        let loaded = store.load("abc").unwrap();
        assert_eq!(loaded.title, "Hello");
    }
}
