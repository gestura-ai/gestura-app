//! File-backed agent session store.

use chrono::{Datelike, Local, NaiveDate};
use std::fs;
use std::path::{Path, PathBuf};

use gestura_core_foundation::AppError;

use super::types::AgentSession;

/// Result type for agent session store operations.
pub type AgentSessionResult<T> = Result<T, AppError>;

/// Filter for listing sessions.
#[derive(Debug, Clone, Default)]
pub enum SessionFilter {
    /// Return all sessions.
    #[default]
    All,
    /// Sessions created today (local time).
    Today,
    /// Sessions created within the current week (local time).
    ThisWeek,
    /// Sessions created within the current month (local time).
    ThisMonth,
    /// Sessions created within an optional inclusive date range.
    DateRange {
        /// Inclusive start date.
        from: Option<NaiveDate>,
        /// Inclusive end date.
        to: Option<NaiveDate>,
    },
}

/// Minimal session info used for list UIs.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Session id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Creation time.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Last activity time.
    pub last_active: chrono::DateTime<chrono::Utc>,
    /// Message count.
    pub message_count: usize,
    /// Optional model.
    pub model: Option<String>,
}

/// A storage abstraction for agent sessions.
pub trait AgentSessionStore {
    /// Save a session.
    fn save(&self, session: &AgentSession) -> AgentSessionResult<()>;

    /// Load a session by id.
    fn load(&self, id: &str) -> AgentSessionResult<AgentSession>;

    /// Delete a session by id.
    fn delete(&self, id: &str) -> AgentSessionResult<bool>;

    /// List sessions matching a filter.
    fn list(&self, filter: SessionFilter) -> AgentSessionResult<Vec<SessionInfo>>;

    /// Load the most recently active session.
    fn load_last(&self) -> AgentSessionResult<Option<AgentSession>>;

    /// Find a session id by prefix (used for CLI convenience).
    fn find_by_prefix(&self, prefix: &str) -> AgentSessionResult<Option<String>>;
}

/// Returns the Gestura data directory (`~/.gestura/`).
///
/// This mirrors `AppConfig::data_dir()` while keeping the sessions crate
/// independent from the config module.
fn gestura_data_dir() -> PathBuf {
    super::gestura_home_dir().join(".gestura")
}

/// Default directory for persisted agent sessions.
///
/// This is intentionally **separate** from `session_workspace::get_sessions_base_dir()`
/// to keep persistence outside sandbox workspaces.
pub fn default_agent_sessions_dir() -> PathBuf {
    gestura_data_dir().join("agent_sessions")
}

/// File-backed session store (one JSON file per session).
#[derive(Debug, Clone)]
pub struct FileAgentSessionStore {
    dir: PathBuf,
}

impl FileAgentSessionStore {
    /// Create a store rooted at a custom directory.
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Create a store using the default directory.
    pub fn new_default() -> Self {
        Self::new(default_agent_sessions_dir())
    }

    fn ensure_dir(&self) -> AgentSessionResult<()> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    fn validate_session_id(&self, id: &str) -> AgentSessionResult<()> {
        if id.is_empty() {
            return Err(AppError::InvalidInput("session id is empty".to_string()));
        }
        if id.len() > 128 {
            return Err(AppError::InvalidInput("session id too long".to_string()));
        }
        if id.contains(['/', '\\']) || id.contains("..") {
            return Err(AppError::InvalidInput("invalid session id".to_string()));
        }
        Ok(())
    }

    fn path_for(&self, id: &str) -> AgentSessionResult<PathBuf> {
        self.validate_session_id(id)?;
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn matches_filter(&self, session: &AgentSession, filter: &SessionFilter) -> bool {
        match filter {
            SessionFilter::All => true,
            SessionFilter::Today => {
                let created = session.created_at.with_timezone(&Local).date_naive();
                created == Local::now().date_naive()
            }
            SessionFilter::ThisWeek => {
                let created = session.created_at.with_timezone(&Local).date_naive();
                let now = Local::now().date_naive();
                let created_week = created.iso_week();
                let now_week = now.iso_week();
                created_week.week() == now_week.week() && created_week.year() == now_week.year()
            }
            SessionFilter::ThisMonth => {
                let created = session.created_at.with_timezone(&Local);
                let now = Local::now();
                created.year() == now.year() && created.month() == now.month()
            }
            SessionFilter::DateRange { from, to } => {
                let created = session.created_at.with_timezone(&Local).date_naive();
                if let Some(from) = from
                    && created < *from
                {
                    return false;
                }
                if let Some(to) = to
                    && created > *to
                {
                    return false;
                }
                true
            }
        }
    }

    fn load_from_path(&self, path: &Path) -> AgentSessionResult<AgentSession> {
        let json = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }
}

impl Default for FileAgentSessionStore {
    fn default() -> Self {
        Self::new_default()
    }
}

impl AgentSessionStore for FileAgentSessionStore {
    fn save(&self, session: &AgentSession) -> AgentSessionResult<()> {
        self.ensure_dir()?;
        let path = self.path_for(&session.id)?;
        let json = serde_json::to_string_pretty(session)?;
        fs::write(path, json)?;
        Ok(())
    }

    fn load(&self, id: &str) -> AgentSessionResult<AgentSession> {
        let path = self.path_for(id)?;
        if !path.exists() {
            return Err(AppError::NotFound(format!("session '{id}' not found")));
        }
        self.load_from_path(&path)
    }

    fn delete(&self, id: &str) -> AgentSessionResult<bool> {
        let path = self.path_for(id)?;
        if path.exists() {
            fs::remove_file(path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn list(&self, filter: SessionFilter) -> AgentSessionResult<Vec<SessionInfo>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == std::ffi::OsStr::new("json"))
            {
                let session = match self.load_from_path(&path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if self.matches_filter(&session, &filter) {
                    let message_count = session.message_count();
                    sessions.push(SessionInfo {
                        id: session.id,
                        title: session.title,
                        created_at: session.created_at,
                        last_active: session.last_active,
                        message_count,
                        model: session.model,
                    });
                }
            }
        }

        sessions.sort_by_key(|s| std::cmp::Reverse(s.last_active));
        Ok(sessions)
    }

    fn load_last(&self) -> AgentSessionResult<Option<AgentSession>> {
        let infos = self.list(SessionFilter::All)?;
        if let Some(info) = infos.first() {
            return Ok(Some(self.load(&info.id)?));
        }
        Ok(None)
    }

    fn find_by_prefix(&self, prefix: &str) -> AgentSessionResult<Option<String>> {
        let infos = self.list(SessionFilter::All)?;
        for info in infos {
            if info.id.starts_with(prefix) {
                return Ok(Some(info.id));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_sessions::MessageSource;
    use tempfile::tempdir;

    #[test]
    fn roundtrip_save_load() {
        let temp = tempdir().unwrap();
        let store = FileAgentSessionStore::new(temp.path().to_path_buf());

        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).unwrap();

        let mut session =
            AgentSession::new_with_workspace(workspace_dir, Some("test-model".to_string()))
                .unwrap();
        session.add_user_message("hello", MessageSource::Text);
        store.save(&session).unwrap();

        let loaded = store.load(&session.id).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.model, session.model);
        assert_eq!(loaded.message_count(), 1);
    }

    #[test]
    fn list_and_find_by_prefix() {
        let temp = tempdir().unwrap();
        let store = FileAgentSessionStore::new(temp.path().to_path_buf());

        let workspace_dir = temp.path().join("workspace2");
        std::fs::create_dir_all(&workspace_dir).unwrap();
        let session = AgentSession::new_with_workspace(workspace_dir, None).unwrap();
        store.save(&session).unwrap();

        let infos = store.list(SessionFilter::All).unwrap();
        assert_eq!(infos.len(), 1);
        let prefix = &session.id[..8];
        let found = store.find_by_prefix(prefix).unwrap();
        assert_eq!(found, Some(session.id));
    }
}
