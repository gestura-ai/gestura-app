//! Chat session model + persistence (facade).
//!
//! All types and logic live in [`gestura_core_sessions::chat_sessions`].
//! This module re-exports everything plus provides config-dependent
//! extensions that require types from `crate::config`.

// Re-export the entire sessions crate chat_sessions module.
pub use gestura_core_sessions::chat_sessions::*;

use crate::config::{AppConfig, GlobalPermissionLevel, GlobalPermissionSettings};
use std::path::Path;

// ---------------------------------------------------------------------------
// Extension trait: config-dependent constructors for SessionToolSettings
// ---------------------------------------------------------------------------

/// Config-dependent constructors for [`SessionToolSettings`].
///
/// These live here (not in the sessions crate) because they depend on
/// [`GlobalPermissionSettings`] and [`AppConfig`] which remain in core's config
/// module.
pub trait SessionToolSettingsConfigExt {
    /// Create session tool settings from the global permission settings.
    fn from_global_permissions(settings: &GlobalPermissionSettings) -> SessionToolSettings;

    /// Convenience helper to derive session tool settings from the full app config.
    fn from_global_config(config: &AppConfig) -> SessionToolSettings;
}

impl SessionToolSettingsConfigExt for SessionToolSettings {
    fn from_global_permissions(settings: &GlobalPermissionSettings) -> SessionToolSettings {
        let permission_level = match settings.default_level {
            GlobalPermissionLevel::Sandbox => SessionPermissionLevel::Sandbox,
            GlobalPermissionLevel::Restricted => SessionPermissionLevel::Restricted,
            GlobalPermissionLevel::Full => SessionPermissionLevel::Full,
        };

        SessionToolSettings {
            permission_level,
            enabled_tools: settings.default_enabled_tools.clone(),
        }
    }

    fn from_global_config(config: &AppConfig) -> SessionToolSettings {
        Self::from_global_permissions(&config.permissions)
    }
}

// ---------------------------------------------------------------------------
// Wrapper: sanitize_session_llm_override (injects concrete validator)
// ---------------------------------------------------------------------------

/// Sanitize potentially invalid persisted (provider, model) overrides.
///
/// This wrapper injects [`crate::llm_validation::is_model_compatible_with_provider`]
/// as the concrete model-compatibility validator.
///
/// Returns `true` if any repair was applied.
pub fn sanitize_session_llm_override(
    session_id: &str,
    state: &mut SessionState,
    global_llm_provider: &str,
) -> bool {
    gestura_core_sessions::chat_sessions::sanitize_session_llm_override(
        session_id,
        state,
        global_llm_provider,
        crate::llm_validation::is_model_compatible_with_provider,
    )
}

// ---------------------------------------------------------------------------
// Wrapper: migrate_legacy_gui_sessions_to_core (injects concrete validator)
// ---------------------------------------------------------------------------

/// One-time migration from the legacy `gui_sessions.json` file into the unified
/// core store.
///
/// This wrapper injects the concrete model-compatibility validator.
pub fn migrate_legacy_gui_sessions_to_core<S: ChatSessionStore>(
    store: &S,
    global_llm_provider: &str,
) -> Vec<ChatSession> {
    gestura_core_sessions::chat_sessions::migrate_legacy_gui_sessions_to_core(
        store,
        global_llm_provider,
        crate::llm_validation::is_model_compatible_with_provider,
    )
}

/// One-time migration from a legacy GUI sessions file at a specific path.
///
/// This wrapper injects the concrete model-compatibility validator.
pub fn migrate_legacy_gui_sessions_to_core_at_path<S: ChatSessionStore>(
    store: &S,
    global_llm_provider: &str,
    path: &Path,
) -> Vec<ChatSession> {
    gestura_core_sessions::chat_sessions::migrate_legacy_gui_sessions_to_core_at_path(
        store,
        global_llm_provider,
        path,
        &crate::llm_validation::is_model_compatible_with_provider,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalPermissionLevel, GlobalPermissionSettings};
    use std::collections::HashMap;

    #[test]
    fn session_tool_settings_from_global_permissions_maps_level_and_tools() {
        let mut tools = HashMap::new();
        tools.insert("file".to_string(), true);
        tools.insert("shell".to_string(), false);

        let settings = GlobalPermissionSettings {
            default_level: GlobalPermissionLevel::Sandbox,
            default_enabled_tools: tools.clone(),
        };

        let session_tools = SessionToolSettings::from_global_permissions(&settings);
        assert_eq!(
            session_tools.permission_level,
            SessionPermissionLevel::Sandbox
        );
        assert_eq!(session_tools.enabled_tools, tools);
    }
}
