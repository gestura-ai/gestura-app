//! Session-scoped knowledge settings
//!
//! Manages which knowledge items are enabled for each session.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

/// Pseudo-session ID used to store *default* knowledge enablement.
///
/// When a real session has no settings file yet, its enabled knowledge set will
/// fall back to this default settings file.
pub const DEFAULT_KNOWLEDGE_SETTINGS_SESSION_ID: &str = "__default__";

/// Session-scoped knowledge settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionKnowledgeSettings {
    /// Session ID
    pub session_id: String,
    /// Set of enabled knowledge item IDs
    pub enabled_knowledge: HashSet<String>,
}

impl SessionKnowledgeSettings {
    /// Create new settings for a session
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            enabled_knowledge: HashSet::new(),
        }
    }

    /// Enable a knowledge item
    pub fn enable(&mut self, knowledge_id: String) {
        self.enabled_knowledge.insert(knowledge_id);
    }

    /// Disable a knowledge item
    pub fn disable(&mut self, knowledge_id: &str) {
        self.enabled_knowledge.remove(knowledge_id);
    }

    /// Check if a knowledge item is enabled
    pub fn is_enabled(&self, knowledge_id: &str) -> bool {
        self.enabled_knowledge.contains(knowledge_id)
    }
}

/// Manager for session-scoped knowledge settings
pub struct KnowledgeSettingsManager {
    /// Base directory for settings files
    base_dir: PathBuf,
    /// In-memory cache of settings
    cache: RwLock<HashMap<String, SessionKnowledgeSettings>>,
}

impl KnowledgeSettingsManager {
    /// Create a new manager with the given base directory
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Get the settings file path for a session
    fn settings_path(&self, session_id: &str) -> PathBuf {
        self.base_dir
            .join(".gestura")
            .join("knowledge_settings")
            .join(format!("{}.json", session_id))
    }

    /// Load settings for a session
    pub fn load(&self, session_id: &str) -> Result<SessionKnowledgeSettings, std::io::Error> {
        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(settings) = cache.get(session_id) {
                return Ok(settings.clone());
            }
        }

        // Load from file (or synthesize from defaults if missing)
        let path = self.settings_path(session_id);

        let settings = if path.exists() {
            let content = fs::read_to_string(&path)?;
            serde_json::from_str::<SessionKnowledgeSettings>(&content)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
        } else if session_id == DEFAULT_KNOWLEDGE_SETTINGS_SESSION_ID {
            SessionKnowledgeSettings::new(session_id.to_string())
        } else {
            // Fall back to defaults for brand-new sessions.
            let defaults = self.load_defaults_no_cache()?;
            let mut s = SessionKnowledgeSettings::new(session_id.to_string());
            s.enabled_knowledge = defaults.enabled_knowledge;
            s
        };

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(session_id.to_string(), settings.clone());
        }

        Ok(settings)
    }

    fn load_defaults_no_cache(&self) -> Result<SessionKnowledgeSettings, std::io::Error> {
        let path = self.settings_path(DEFAULT_KNOWLEDGE_SETTINGS_SESSION_ID);
        if !path.exists() {
            return Ok(SessionKnowledgeSettings::new(
                DEFAULT_KNOWLEDGE_SETTINGS_SESSION_ID.to_string(),
            ));
        }

        let content = fs::read_to_string(&path)?;
        serde_json::from_str::<SessionKnowledgeSettings>(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Save settings for a session
    pub fn save(&self, settings: &SessionKnowledgeSettings) -> Result<(), std::io::Error> {
        let path = self.settings_path(&settings.session_id);

        // Ensure directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Save to file
        let content = serde_json::to_string_pretty(settings)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(&path, content)?;

        // Update cache
        {
            let mut cache = self.cache.write().unwrap();
            cache.insert(settings.session_id.clone(), settings.clone());
        }

        Ok(())
    }

    /// Set knowledge enabled/disabled for a session
    pub fn set_knowledge_enabled(
        &self,
        session_id: &str,
        knowledge_id: &str,
        enabled: bool,
    ) -> Result<(), std::io::Error> {
        let mut settings = self.load(session_id)?;

        if enabled {
            settings.enable(knowledge_id.to_string());
        } else {
            settings.disable(knowledge_id);
        }

        self.save(&settings)
    }

    /// Get list of enabled knowledge IDs for a session
    pub fn get_enabled_knowledge(&self, session_id: &str) -> Result<Vec<String>, std::io::Error> {
        let settings = self.load(session_id)?;
        let mut out: Vec<String> = settings.enabled_knowledge.into_iter().collect();
        out.sort();
        Ok(out)
    }

    /// Check if a knowledge item is enabled for a session
    pub fn is_enabled(&self, session_id: &str, knowledge_id: &str) -> Result<bool, std::io::Error> {
        let settings = self.load(session_id)?;
        Ok(settings.is_enabled(knowledge_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_missing_session_falls_back_to_defaults() {
        let tmp = tempdir().unwrap();
        let mgr = KnowledgeSettingsManager::new(tmp.path().to_path_buf());

        // Save defaults
        let mut defaults =
            SessionKnowledgeSettings::new(DEFAULT_KNOWLEDGE_SETTINGS_SESSION_ID.to_string());
        defaults.enable("rust-expert".to_string());
        defaults.enable("tauri-expert".to_string());
        mgr.save(&defaults).unwrap();

        // Missing session should inherit defaults
        let s1 = mgr.load("session-1").unwrap();
        assert!(s1.enabled_knowledge.contains("rust-expert"));
        assert!(s1.enabled_knowledge.contains("tauri-expert"));
    }

    #[test]
    fn load_existing_session_does_not_merge_defaults() {
        let tmp = tempdir().unwrap();
        let mgr = KnowledgeSettingsManager::new(tmp.path().to_path_buf());

        let mut defaults =
            SessionKnowledgeSettings::new(DEFAULT_KNOWLEDGE_SETTINGS_SESSION_ID.to_string());
        defaults.enable("rust-expert".to_string());
        mgr.save(&defaults).unwrap();

        // Explicit session settings should be authoritative.
        let mut s1 = SessionKnowledgeSettings::new("session-1".to_string());
        s1.enable("mcp-expert".to_string());
        mgr.save(&s1).unwrap();

        let loaded = mgr.load("session-1").unwrap();
        assert!(loaded.enabled_knowledge.contains("mcp-expert"));
        assert!(!loaded.enabled_knowledge.contains("rust-expert"));
    }
}
