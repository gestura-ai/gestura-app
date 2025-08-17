//! Permission scoping system for Gestura.app
//! Provides fine-grained access control for different components and operations

use crate::AppError;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Permission scope for different operations
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Permission {
    // Voice permissions
    VoiceRecord,
    VoiceProcess,
    VoiceModelAccess,
    
    // BLE permissions
    BleDiscover,
    BleConnect,
    BleRead,
    BleWrite,
    
    // Haptic permissions
    HapticSend,
    HapticPattern,
    HapticIntensity,
    
    // Agent permissions
    AgentSpawn,
    AgentKill,
    AgentCommunicate,
    
    // File system permissions
    FileRead(String),  // Path pattern
    FileWrite(String), // Path pattern
    FileExecute(String), // Path pattern
    
    // Network permissions
    NetworkConnect(String), // Host pattern
    NetworkListen(u16),     // Port
    
    // System permissions
    SystemInfo,
    SystemConfig,
    SystemShutdown,
    
    // MCP permissions
    McpToolCall,
    McpResourceRead,
    McpResourceWrite,
}

/// Permission context for a specific entity (user, agent, etc.)
#[derive(Debug, Clone)]
pub struct PermissionContext {
    pub entity_id: String,
    pub entity_type: EntityType,
    pub granted_permissions: HashSet<Permission>,
    pub denied_permissions: HashSet<Permission>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Type of entity requesting permissions
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EntityType {
    User,
    Agent,
    McpClient,
    System,
}

/// Permission manager
pub struct PermissionManager {
    contexts: Arc<RwLock<HashMap<String, PermissionContext>>>,
    default_permissions: Arc<RwLock<HashMap<EntityType, HashSet<Permission>>>>,
    audit_log: Arc<RwLock<Vec<PermissionAuditEntry>>>,
    max_audit_entries: usize,
}

/// Audit log entry for permission checks
#[derive(Debug, Clone)]
pub struct PermissionAuditEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub entity_id: String,
    pub entity_type: EntityType,
    pub permission: Permission,
    pub granted: bool,
    pub reason: String,
}

impl PermissionManager {
    /// Create a new permission manager
    pub fn new(max_audit_entries: usize) -> Self {
        let mut default_permissions = HashMap::new();
        
        // Default permissions for different entity types
        let mut user_permissions = HashSet::new();
        user_permissions.insert(Permission::VoiceRecord);
        user_permissions.insert(Permission::VoiceProcess);
        user_permissions.insert(Permission::BleDiscover);
        user_permissions.insert(Permission::BleConnect);
        user_permissions.insert(Permission::HapticSend);
        user_permissions.insert(Permission::SystemInfo);
        user_permissions.insert(Permission::SystemConfig);
        default_permissions.insert(EntityType::User, user_permissions);
        
        let mut agent_permissions = HashSet::new();
        agent_permissions.insert(Permission::VoiceProcess);
        agent_permissions.insert(Permission::BleRead);
        agent_permissions.insert(Permission::AgentCommunicate);
        agent_permissions.insert(Permission::FileRead("/tmp/*".to_string()));
        default_permissions.insert(EntityType::Agent, agent_permissions);
        
        let mut mcp_permissions = HashSet::new();
        mcp_permissions.insert(Permission::McpToolCall);
        mcp_permissions.insert(Permission::McpResourceRead);
        mcp_permissions.insert(Permission::HapticSend);
        default_permissions.insert(EntityType::McpClient, mcp_permissions);
        
        let mut system_permissions = HashSet::new();
        system_permissions.insert(Permission::SystemShutdown);
        system_permissions.insert(Permission::AgentSpawn);
        system_permissions.insert(Permission::AgentKill);
        default_permissions.insert(EntityType::System, system_permissions);

        Self {
            contexts: Arc::new(RwLock::new(HashMap::new())),
            default_permissions: Arc::new(RwLock::new(default_permissions)),
            audit_log: Arc::new(RwLock::new(Vec::new())),
            max_audit_entries,
        }
    }

    /// Register a permission context for an entity
    pub async fn register_context(&self, context: PermissionContext) {
        let mut contexts = self.contexts.write().await;
        contexts.insert(context.entity_id.clone(), context);
    }

    /// Check if an entity has a specific permission
    pub async fn check_permission(&self, entity_id: &str, permission: &Permission) -> bool {
        let result = self.check_permission_internal(entity_id, permission).await;
        
        // Log the permission check
        self.audit_permission_check(entity_id, permission, result.0, &result.1).await;
        
        result.0
    }

    /// Internal permission check with reason
    async fn check_permission_internal(&self, entity_id: &str, permission: &Permission) -> (bool, String) {
        let contexts = self.contexts.read().await;
        
        if let Some(context) = contexts.get(entity_id) {
            // Check if permission is explicitly denied
            if context.denied_permissions.contains(permission) {
                return (false, "Permission explicitly denied".to_string());
            }
            
            // Check if context has expired
            if let Some(expires_at) = context.expires_at {
                if chrono::Utc::now() > expires_at {
                    return (false, "Permission context expired".to_string());
                }
            }
            
            // Check if permission is explicitly granted
            if context.granted_permissions.contains(permission) {
                return (true, "Permission explicitly granted".to_string());
            }
            
            // Check default permissions for entity type
            let default_perms = self.default_permissions.read().await;
            if let Some(defaults) = default_perms.get(&context.entity_type) {
                if defaults.contains(permission) {
                    return (true, "Permission granted by default".to_string());
                }
                
                // Check pattern-based permissions
                if self.check_pattern_permission(permission, defaults) {
                    return (true, "Permission granted by pattern match".to_string());
                }
            }
            
            (false, "Permission not granted".to_string())
        } else {
            (false, "Entity not found".to_string())
        }
    }

    /// Check pattern-based permissions (for file paths, network hosts, etc.)
    fn check_pattern_permission(&self, requested: &Permission, granted: &HashSet<Permission>) -> bool {
        match requested {
            Permission::FileRead(path) => {
                granted.iter().any(|p| match p {
                    Permission::FileRead(pattern) => self.matches_pattern(path, pattern),
                    _ => false,
                })
            }
            Permission::FileWrite(path) => {
                granted.iter().any(|p| match p {
                    Permission::FileWrite(pattern) => self.matches_pattern(path, pattern),
                    _ => false,
                })
            }
            Permission::NetworkConnect(host) => {
                granted.iter().any(|p| match p {
                    Permission::NetworkConnect(pattern) => self.matches_pattern(host, pattern),
                    _ => false,
                })
            }
            _ => false,
        }
    }

    /// Simple pattern matching (supports * wildcard)
    fn matches_pattern(&self, value: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        if pattern.ends_with("/*") {
            let prefix = &pattern[..pattern.len() - 2];
            return value.starts_with(prefix);
        }

        if pattern.starts_with("*.") {
            let suffix = &pattern[1..]; // Remove the *
            return value.ends_with(suffix);
        }

        if pattern.starts_with("*/") {
            let suffix = &pattern[2..];
            return value.ends_with(suffix);
        }

        value == pattern
    }

    /// Grant a permission to an entity
    pub async fn grant_permission(&self, entity_id: &str, permission: Permission) -> Result<(), AppError> {
        let mut contexts = self.contexts.write().await;
        
        if let Some(context) = contexts.get_mut(entity_id) {
            context.granted_permissions.insert(permission.clone());
            context.denied_permissions.remove(&permission);
            
            tracing::info!("Granted permission {:?} to entity {}", permission, entity_id);
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Entity {} not found", entity_id)
            )))
        }
    }

    /// Revoke a permission from an entity
    pub async fn revoke_permission(&self, entity_id: &str, permission: Permission) -> Result<(), AppError> {
        let mut contexts = self.contexts.write().await;
        
        if let Some(context) = contexts.get_mut(entity_id) {
            context.granted_permissions.remove(&permission);
            context.denied_permissions.insert(permission.clone());
            
            tracing::info!("Revoked permission {:?} from entity {}", permission, entity_id);
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Entity {} not found", entity_id)
            )))
        }
    }

    /// Get all permissions for an entity
    pub async fn get_permissions(&self, entity_id: &str) -> Option<HashSet<Permission>> {
        let contexts = self.contexts.read().await;
        contexts.get(entity_id).map(|c| c.granted_permissions.clone())
    }

    /// Audit permission check
    async fn audit_permission_check(&self, entity_id: &str, permission: &Permission, granted: bool, reason: &str) {
        let contexts = self.contexts.read().await;
        let entity_type = contexts.get(entity_id)
            .map(|c| c.entity_type.clone())
            .unwrap_or(EntityType::System);
        drop(contexts);

        let entry = PermissionAuditEntry {
            timestamp: chrono::Utc::now(),
            entity_id: entity_id.to_string(),
            entity_type,
            permission: permission.clone(),
            granted,
            reason: reason.to_string(),
        };

        let mut audit_log = self.audit_log.write().await;
        audit_log.push(entry);
        
        // Trim audit log if needed
        if audit_log.len() > self.max_audit_entries {
            audit_log.remove(0);
        }
    }

    /// Get audit log entries
    pub async fn get_audit_log(&self, limit: Option<usize>) -> Vec<PermissionAuditEntry> {
        let audit_log = self.audit_log.read().await;
        if let Some(limit) = limit {
            audit_log.iter().rev().take(limit).cloned().collect()
        } else {
            audit_log.clone()
        }
    }

    /// Clear audit log
    pub async fn clear_audit_log(&self) {
        let mut audit_log = self.audit_log.write().await;
        audit_log.clear();
        tracing::info!("Permission audit log cleared");
    }
}

/// Global permission manager instance
static PERMISSION_MANAGER: tokio::sync::OnceCell<PermissionManager> = tokio::sync::OnceCell::const_new();

/// Get the global permission manager
pub async fn get_permission_manager() -> &'static PermissionManager {
    PERMISSION_MANAGER.get_or_init(|| async {
        PermissionManager::new(10000)
    }).await
}

/// Convenience function to check permission
pub async fn check_permission(entity_id: &str, permission: &Permission) -> bool {
    let manager = get_permission_manager().await;
    manager.check_permission(entity_id, permission).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_permission_manager() {
        let manager = PermissionManager::new(100);
        
        // Create a test context
        let context = PermissionContext {
            entity_id: "test-user".to_string(),
            entity_type: EntityType::User,
            granted_permissions: HashSet::new(),
            denied_permissions: HashSet::new(),
            expires_at: None,
        };
        
        manager.register_context(context).await;
        
        // Test default permissions
        assert!(manager.check_permission("test-user", &Permission::VoiceRecord).await);
        assert!(!manager.check_permission("test-user", &Permission::SystemShutdown).await);
        
        // Test explicit grant
        manager.grant_permission("test-user", Permission::SystemShutdown).await.unwrap();
        assert!(manager.check_permission("test-user", &Permission::SystemShutdown).await);
        
        // Test revoke
        manager.revoke_permission("test-user", Permission::SystemShutdown).await.unwrap();
        assert!(!manager.check_permission("test-user", &Permission::SystemShutdown).await);
    }

    #[tokio::test]
    async fn test_pattern_permissions() {
        let manager = PermissionManager::new(100);
        
        // Test pattern matching
        assert!(manager.matches_pattern("/tmp/test.txt", "/tmp/*"));
        assert!(manager.matches_pattern("example.com", "*.com"));
        assert!(!manager.matches_pattern("/home/user", "/tmp/*"));
    }
}
