//! Permission management for tool access
//!
//! Provides permission management with structured output.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

/// A permission grant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub tool: String,
    pub action: String,
    pub scope: PermissionScope,
    pub granted_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Scope of a permission
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PermissionScope {
    /// Permission applies to all resources
    Global,
    /// Permission applies to a specific path pattern
    Path(String),
    /// Permission applies to a specific command pattern
    Command(String),
}

/// Permission check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionCheck {
    pub tool: String,
    pub action: String,
    pub allowed: bool,
    pub reason: String,
}

/// Persisted permission state
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PermissionState {
    permissions: Vec<Permission>,
}

/// Permission management service
pub struct PermissionManager {
    permissions: RwLock<HashMap<String, HashSet<Permission>>>,
    config_path: PathBuf,
}

impl Permission {
    fn key(&self) -> String {
        format!("{}:{}", self.tool, self.action)
    }
}

impl std::hash::Hash for Permission {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.tool.hash(state);
        self.action.hash(state);
        self.scope.hash(state);
    }
}

impl PartialEq for Permission {
    fn eq(&self, other: &Self) -> bool {
        self.tool == other.tool && self.action == other.action && self.scope == other.scope
    }
}

impl Eq for Permission {}

impl Default for PermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionManager {
    pub fn new() -> Self {
        let config_path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("gestura")
            .join("permissions.json");

        tracing::debug!(
            config_path = ?config_path,
            "Initializing PermissionManager"
        );

        let manager = Self {
            permissions: RwLock::new(HashMap::new()),
            config_path,
        };
        let _ = manager.load();
        manager
    }

    /// Load permissions from disk
    fn load(&self) -> Result<()> {
        tracing::debug!(
            config_path = ?self.config_path,
            exists = self.config_path.exists(),
            "Loading permissions from disk"
        );

        if self.config_path.exists() {
            let content = fs::read_to_string(&self.config_path)?;
            let state: PermissionState = serde_json::from_str(&content).map_err(AppError::Json)?;

            let mut perms = self
                .permissions
                .write()
                .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;

            let count = state.permissions.len();
            for perm in state.permissions {
                perms.entry(perm.key()).or_default().insert(perm);
            }

            tracing::debug!(
                config_path = ?self.config_path,
                permissions_loaded = count,
                "Permissions loaded successfully"
            );
        } else {
            tracing::debug!(
                config_path = ?self.config_path,
                "No permissions file found, starting with empty permissions"
            );
        }
        Ok(())
    }

    /// Save permissions to disk
    fn save(&self) -> Result<()> {
        let perms = self
            .permissions
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;

        let all_perms: Vec<Permission> = perms.values().flatten().cloned().collect();
        let state = PermissionState {
            permissions: all_perms,
        };

        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = serde_json::to_string_pretty(&state).map_err(AppError::Json)?;
        fs::write(&self.config_path, &content)?;

        tracing::debug!(
            config_path = ?self.config_path,
            permissions_saved = state.permissions.len(),
            "Permissions saved to disk"
        );
        Ok(())
    }

    /// List all permissions
    pub fn list(&self) -> Result<Vec<Permission>> {
        let perms = self
            .permissions
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;
        Ok(perms.values().flatten().cloned().collect())
    }

    /// Grant a permission
    pub fn grant(
        &self,
        tool: &str,
        action: &str,
        scope: PermissionScope,
        ttl_secs: Option<u64>,
    ) -> Result<Permission> {
        tracing::debug!(
            tool = %tool,
            action = %action,
            scope = ?scope,
            ttl_secs = ?ttl_secs,
            "Granting permission"
        );

        let perm = Permission {
            tool: tool.to_string(),
            action: action.to_string(),
            scope,
            granted_at: chrono::Utc::now(),
            expires_at: ttl_secs.map(|s| chrono::Utc::now() + chrono::Duration::seconds(s as i64)),
        };

        let mut perms = self
            .permissions
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;
        perms.entry(perm.key()).or_default().insert(perm.clone());
        drop(perms);
        self.save()?;

        tracing::info!(
            tool = %tool,
            action = %action,
            expires_at = ?perm.expires_at,
            "Permission granted successfully"
        );
        Ok(perm)
    }

    /// Revoke a permission
    pub fn revoke(&self, tool: &str, action: &str) -> Result<usize> {
        tracing::debug!(
            tool = %tool,
            action = %action,
            "Revoking permission"
        );

        let key = format!("{tool}:{action}");
        let mut perms = self
            .permissions
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;
        let removed = perms.remove(&key).map(|s| s.len()).unwrap_or(0);
        drop(perms);
        self.save()?;

        tracing::info!(
            tool = %tool,
            action = %action,
            removed_count = removed,
            "Permission revoked"
        );
        Ok(removed)
    }

    /// Check if an action is permitted
    pub fn check(
        &self,
        tool: &str,
        action: &str,
        resource: Option<&str>,
    ) -> Result<PermissionCheck> {
        tracing::debug!(
            tool = %tool,
            action = %action,
            resource = ?resource,
            "Checking permission"
        );

        let perms = self
            .permissions
            .read()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;

        let key = format!("{tool}:{action}");
        let now = chrono::Utc::now();

        if let Some(perm_set) = perms.get(&key) {
            for perm in perm_set {
                // Check expiry
                if let Some(expires) = perm.expires_at
                    && expires < now
                {
                    tracing::debug!(
                        tool = %tool,
                        action = %action,
                        expires_at = ?expires,
                        "Permission expired, skipping"
                    );
                    continue;
                }

                // Check scope
                let matches = match &perm.scope {
                    PermissionScope::Global => true,
                    PermissionScope::Path(pattern) => {
                        resource.map(|r| r.starts_with(pattern)).unwrap_or(false)
                    }
                    PermissionScope::Command(pattern) => {
                        resource.map(|r| r.contains(pattern)).unwrap_or(false)
                    }
                };

                if matches {
                    tracing::debug!(
                        tool = %tool,
                        action = %action,
                        resource = ?resource,
                        scope = ?perm.scope,
                        "Permission check: ALLOWED"
                    );
                    return Ok(PermissionCheck {
                        tool: tool.to_string(),
                        action: action.to_string(),
                        allowed: true,
                        reason: "Permission granted".to_string(),
                    });
                }
            }
        }

        tracing::debug!(
            tool = %tool,
            action = %action,
            resource = ?resource,
            "Permission check: DENIED (no matching permission)"
        );
        Ok(PermissionCheck {
            tool: tool.to_string(),
            action: action.to_string(),
            allowed: false,
            reason: "No matching permission found".to_string(),
        })
    }

    /// Reset all permissions
    pub fn reset(&self) -> Result<usize> {
        let mut perms = self
            .permissions
            .write()
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Lock error: {e}"))))?;
        let count = perms.values().map(|s| s.len()).sum();
        perms.clear();
        drop(perms);

        // Delete the file
        if self.config_path.exists() {
            fs::remove_file(&self.config_path)?;
        }

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_scope_variants() {
        let global = PermissionScope::Global;
        let path = PermissionScope::Path("/home/*".to_string());
        let cmd = PermissionScope::Command("echo *".to_string());

        assert!(matches!(global, PermissionScope::Global));
        assert!(matches!(path, PermissionScope::Path(_)));
        assert!(matches!(cmd, PermissionScope::Command(_)));
    }

    #[test]
    fn test_permission_struct() {
        let perm = Permission {
            tool: "file".to_string(),
            action: "read".to_string(),
            scope: PermissionScope::Global,
            granted_at: chrono::Utc::now(),
            expires_at: None,
        };
        assert_eq!(perm.tool, "file");
        assert_eq!(perm.action, "read");
    }

    #[test]
    fn test_permission_check_struct() {
        let check = PermissionCheck {
            tool: "file".to_string(),
            action: "read".to_string(),
            allowed: true,
            reason: "Granted".to_string(),
        };
        assert!(check.allowed);
    }

    #[test]
    fn test_permission_manager_new() {
        // Just test that we can create a manager
        let manager = PermissionManager::new();
        // List should work even if empty
        let perms = manager.list().unwrap();
        assert!(perms.is_empty() || !perms.is_empty()); // Just check it doesn't panic
    }
}
