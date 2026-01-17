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
    FileRead(String),    // Path pattern
    FileWrite(String),   // Path pattern
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
        self.audit_permission_check(entity_id, permission, result.0, &result.1)
            .await;

        result.0
    }

    /// Internal permission check with reason
    async fn check_permission_internal(
        &self,
        entity_id: &str,
        permission: &Permission,
    ) -> (bool, String) {
        let contexts = self.contexts.read().await;

        if let Some(context) = contexts.get(entity_id) {
            // Check if permission is explicitly denied
            if context.denied_permissions.contains(permission) {
                return (false, "Permission explicitly denied".to_string());
            }

            // Check if context has expired
            if let Some(expires_at) = context.expires_at
                && chrono::Utc::now() > expires_at
            {
                return (false, "Permission context expired".to_string());
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
    fn check_pattern_permission(
        &self,
        requested: &Permission,
        granted: &HashSet<Permission>,
    ) -> bool {
        match requested {
            Permission::FileRead(path) => granted.iter().any(|p| match p {
                Permission::FileRead(pattern) => self.matches_pattern(path, pattern),
                _ => false,
            }),
            Permission::FileWrite(path) => granted.iter().any(|p| match p {
                Permission::FileWrite(pattern) => self.matches_pattern(path, pattern),
                _ => false,
            }),
            Permission::NetworkConnect(host) => granted.iter().any(|p| match p {
                Permission::NetworkConnect(pattern) => self.matches_pattern(host, pattern),
                _ => false,
            }),
            _ => false,
        }
    }

    /// Simple pattern matching (supports * wildcard)
    fn matches_pattern(&self, value: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }

        if let Some(prefix) = pattern.strip_suffix("/*") {
            return value.starts_with(prefix);
        }

        if pattern.starts_with("*.") {
            let suffix = &pattern[1..]; // Remove the *
            return value.ends_with(suffix);
        }

        if let Some(suffix) = pattern.strip_prefix("*/") {
            return value.ends_with(suffix);
        }

        value == pattern
    }

    /// Grant a permission to an entity
    pub async fn grant_permission(
        &self,
        entity_id: &str,
        permission: Permission,
    ) -> Result<(), AppError> {
        let mut contexts = self.contexts.write().await;

        if let Some(context) = contexts.get_mut(entity_id) {
            context.granted_permissions.insert(permission.clone());
            context.denied_permissions.remove(&permission);

            tracing::info!(
                "Granted permission {:?} to entity {}",
                permission,
                entity_id
            );
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Entity {} not found", entity_id),
            )))
        }
    }

    /// Revoke a permission from an entity
    pub async fn revoke_permission(
        &self,
        entity_id: &str,
        permission: Permission,
    ) -> Result<(), AppError> {
        let mut contexts = self.contexts.write().await;

        if let Some(context) = contexts.get_mut(entity_id) {
            context.granted_permissions.remove(&permission);
            context.denied_permissions.insert(permission.clone());

            tracing::info!(
                "Revoked permission {:?} from entity {}",
                permission,
                entity_id
            );
            Ok(())
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Entity {} not found", entity_id),
            )))
        }
    }

    /// Get all permissions for an entity
    pub async fn get_permissions(&self, entity_id: &str) -> Option<HashSet<Permission>> {
        let contexts = self.contexts.read().await;
        contexts
            .get(entity_id)
            .map(|c| c.granted_permissions.clone())
    }

    /// Audit permission check
    async fn audit_permission_check(
        &self,
        entity_id: &str,
        permission: &Permission,
        granted: bool,
        reason: &str,
    ) {
        let contexts = self.contexts.read().await;
        let entity_type = contexts
            .get(entity_id)
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
static PERMISSION_MANAGER: tokio::sync::OnceCell<PermissionManager> =
    tokio::sync::OnceCell::const_new();

/// Get the global permission manager
pub async fn get_permission_manager() -> &'static PermissionManager {
    PERMISSION_MANAGER
        .get_or_init(|| async { PermissionManager::new(10000) })
        .await
}

/// Convenience function to check permission
pub async fn check_permission(entity_id: &str, permission: &Permission) -> bool {
    let manager = get_permission_manager().await;
    manager.check_permission(entity_id, permission).await
}

// ============================================================================
// macOS System Permission Checking
// ============================================================================

/// System permission status for macOS
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemPermissionStatus {
    /// Permission has been granted
    Granted,
    /// Permission has been denied
    Denied,
    /// Permission has not been determined yet
    NotDetermined,
    /// Permission is restricted (e.g., by parental controls)
    Restricted,
    /// Permission status is unknown
    Unknown,
}

impl std::fmt::Display for SystemPermissionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemPermissionStatus::Granted => write!(f, "granted"),
            SystemPermissionStatus::Denied => write!(f, "denied"),
            SystemPermissionStatus::NotDetermined => write!(f, "not_determined"),
            SystemPermissionStatus::Restricted => write!(f, "restricted"),
            SystemPermissionStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// Check microphone permission on macOS using AVCaptureDevice
#[cfg(target_os = "macos")]
pub fn check_microphone_permission() -> SystemPermissionStatus {
    use std::process::Command;

    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework "AVFoundation"
            set authStatus to current application's AVCaptureDevice's authorizationStatusForMediaType:(current application's AVMediaTypeAudio)
            if authStatus = 0 then
                return "not_determined"
            else if authStatus = 1 then
                return "restricted"
            else if authStatus = 2 then
                return "denied"
            else if authStatus = 3 then
                return "granted"
            else
                return "unknown"
            end if
            "#,
        ])
        .output();

    parse_permission_output(output)
}

/// Check accessibility permission on macOS using AXIsProcessTrusted
#[cfg(target_os = "macos")]
pub fn check_accessibility_permission() -> SystemPermissionStatus {
    use std::process::Command;

    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework "ApplicationServices"
            if current application's AXIsProcessTrusted() then
                return "granted"
            else
                return "denied"
            end if
            "#,
        ])
        .output();

    parse_permission_output(output)
}

/// Check bluetooth permission on macOS using CBManager
#[cfg(target_os = "macos")]
pub fn check_bluetooth_permission() -> SystemPermissionStatus {
    use std::process::Command;

    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework "CoreBluetooth"
            set authStatus to current application's CBManager's authorization()
            if authStatus = 0 then
                return "not_determined"
            else if authStatus = 1 then
                return "restricted"
            else if authStatus = 2 then
                return "denied"
            else if authStatus = 3 then
                return "granted"
            else
                return "unknown"
            end if
            "#,
        ])
        .output();

    parse_permission_output(output)
}

// macOS Screen Recording permission FFI (CoreGraphics)
#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// Check Screen Recording permission on macOS.
///
/// Note: CoreGraphics does not expose a rich status (Denied vs NotDetermined)
/// via this API; we treat `false` as denied so the UI can prompt the user to
/// open System Settings.
#[cfg(target_os = "macos")]
pub fn check_screen_recording_permission() -> SystemPermissionStatus {
    // SAFETY: FFI call.
    let granted = unsafe { CGPreflightScreenCaptureAccess() };
    if granted {
        SystemPermissionStatus::Granted
    } else {
        SystemPermissionStatus::Denied
    }
}

/// Parse osascript output to permission status
#[cfg(target_os = "macos")]
fn parse_permission_output(
    output: Result<std::process::Output, std::io::Error>,
) -> SystemPermissionStatus {
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

            if out.status.success() {
                tracing::debug!(
                    "System permission check script succeeded: status={:?}, stdout='{}', stderr='{}'",
                    out.status,
                    stdout,
                    stderr
                );
            } else {
                tracing::warn!(
                    "System permission check script exited with non-zero status: status={:?}, stdout='{}', stderr='{}'",
                    out.status,
                    stdout,
                    stderr
                );
            }

            match stdout.as_str() {
                "granted" => SystemPermissionStatus::Granted,
                "denied" => SystemPermissionStatus::Denied,
                "not_determined" => SystemPermissionStatus::NotDetermined,
                "restricted" => SystemPermissionStatus::Restricted,
                other => {
                    if other.is_empty() {
                        tracing::warn!(
                            "System permission check returned empty status string; defaulting to Unknown",
                        );
                    } else {
                        tracing::warn!(
                            "System permission check returned unrecognised status '{}'; defaulting to Unknown",
                            other
                        );
                    }
                    SystemPermissionStatus::Unknown
                }
            }
        }
        Err(e) => {
            tracing::warn!("Failed to execute system permission check script: {}", e);
            SystemPermissionStatus::Unknown
        }
    }
}

/// Request microphone permission on macOS
/// This triggers the system permission dialog
#[cfg(target_os = "macos")]
pub fn request_microphone_permission() -> bool {
    use std::process::Command;

    tracing::info!("Requesting microphone permission via AVFoundation...");

    // First try to trigger the permission dialog using osascript
    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework "AVFoundation"
            set requestResult to current application's AVCaptureDevice's requestAccessForMediaType:(current application's AVMediaTypeAudio) completionHandler:(missing value)
            return "requested"
            "#,
        ])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

            if out.status.success() {
                tracing::info!(
                    "Microphone permission request script completed successfully: stdout='{}', stderr='{}'",
                    stdout,
                    stderr
                );
                true
            } else {
                tracing::warn!(
                    "Microphone permission request script exited with non-zero status {:?}: stdout='{}', stderr='{}'",
                    out.status,
                    stdout,
                    stderr
                );
                false
            }
        }
        Err(e) => {
            tracing::warn!(
                "Failed to execute microphone permission request script: {}",
                e
            );
            false
        }
    }
}

/// Request bluetooth permission on macOS
/// Bluetooth permission is typically triggered by scanning for devices
#[cfg(target_os = "macos")]
pub fn request_bluetooth_permission() -> bool {
    use std::process::Command;

    tracing::info!("Requesting Bluetooth permission via CoreBluetooth...");

    // Try to trigger Bluetooth permission by initializing CBCentralManager
    // This should prompt the user if permission is not_determined
    let output = Command::new("osascript")
        .args([
            "-e",
            r#"
            use framework "CoreBluetooth"
            -- Creating a CBCentralManager triggers the permission dialog
            set centralManager to current application's CBCentralManager's alloc()'s init()
            delay 0.5
            return "requested"
            "#,
        ])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

            if out.status.success() {
                tracing::info!(
                    "Bluetooth permission request script completed successfully: stdout='{}', stderr='{}'",
                    stdout,
                    stderr
                );
                true
            } else {
                tracing::warn!(
                    "Bluetooth permission request script exited with non-zero status {:?}: stdout='{}', stderr='{}'",
                    out.status,
                    stdout,
                    stderr
                );
                false
            }
        }
        Err(e) => {
            tracing::warn!(
                "Failed to execute Bluetooth permission request script: {}",
                e
            );
            false
        }
    }
}

/// Request Screen Recording permission on macOS.
///
/// This may display a system prompt (first request) and/or require the user to
/// enable the permission in System Settings.
#[cfg(target_os = "macos")]
pub fn request_screen_recording_permission() -> bool {
    // SAFETY: FFI call.
    unsafe { CGRequestScreenCaptureAccess() }
}

/// Check if running macOS 13 (Ventura) or later which uses "System Settings" instead of "System Preferences"
#[cfg(target_os = "macos")]
fn is_macos_ventura_or_later() -> bool {
    use std::process::Command;

    let output = Command::new("sw_vers").arg("-productVersion").output().ok();

    if let Some(out) = output {
        let version_str = String::from_utf8_lossy(&out.stdout);
        if let Some(major) = version_str.trim().split('.').next()
            && let Ok(major_version) = major.parse::<u32>()
        {
            return major_version >= 13;
        }
    }
    // Default to newer format if we can't determine version
    true
}

/// Open System Preferences/Settings to the appropriate pane
/// Uses different URL schemes depending on macOS version:
/// - macOS 13+ (Ventura/Sonoma/Sequoia): com.apple.settings.PrivacySecurity.extension?Privacy_*
/// - macOS 12 and earlier: com.apple.preference.security?Privacy_*
#[cfg(target_os = "macos")]
pub fn open_system_preferences(pane: &str) -> bool {
    use std::process::Command;

    let use_new_urls = is_macos_ventura_or_later();

    let pane_url = match (pane, use_new_urls) {
        ("microphone", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Microphone"
        }
        ("microphone", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        // Back-compat aliases used by some UIs.
        ("privacy_microphone", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Microphone"
        }
        ("privacy_microphone", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        ("accessibility", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility"
        }
        ("accessibility", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        ("privacy_accessibility", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility"
        }
        ("privacy_accessibility", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        ("bluetooth", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Bluetooth"
        }
        ("bluetooth", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth"
        }
        ("privacy_bluetooth", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Bluetooth"
        }
        ("privacy_bluetooth", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth"
        }
        ("screen_recording", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture"
        }
        ("screen_recording", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        ("privacy_screenrecording", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture"
        }
        ("privacy_screenrecording", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        ("privacy_screencapture", true) => {
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture"
        }
        ("privacy_screencapture", false) => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        _ => return false,
    };

    tracing::info!(
        "Opening System {} for {}: {}",
        if use_new_urls {
            "Settings"
        } else {
            "Preferences"
        },
        pane,
        pane_url
    );

    let result = Command::new("open").arg(pane_url).spawn();
    match result {
        Ok(_) => {
            tracing::info!("✅ Successfully opened System Settings for {}", pane);
            true
        }
        Err(e) => {
            tracing::error!("❌ Failed to open System Settings for {}: {}", pane, e);
            false
        }
    }
}

// Non-macOS fallbacks
#[cfg(not(target_os = "macos"))]
pub fn check_microphone_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

#[cfg(not(target_os = "macos"))]
pub fn check_accessibility_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

#[cfg(not(target_os = "macos"))]
pub fn check_bluetooth_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

#[cfg(not(target_os = "macos"))]
pub fn check_screen_recording_permission() -> SystemPermissionStatus {
    SystemPermissionStatus::Granted
}

#[cfg(not(target_os = "macos"))]
pub fn request_microphone_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn request_bluetooth_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn request_screen_recording_permission() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn open_system_preferences(_pane: &str) -> bool {
    false
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
        assert!(
            manager
                .check_permission("test-user", &Permission::VoiceRecord)
                .await
        );
        assert!(
            !manager
                .check_permission("test-user", &Permission::SystemShutdown)
                .await
        );

        // Test explicit grant
        manager
            .grant_permission("test-user", Permission::SystemShutdown)
            .await
            .unwrap();
        assert!(
            manager
                .check_permission("test-user", &Permission::SystemShutdown)
                .await
        );

        // Test revoke
        manager
            .revoke_permission("test-user", Permission::SystemShutdown)
            .await
            .unwrap();
        assert!(
            !manager
                .check_permission("test-user", &Permission::SystemShutdown)
                .await
        );
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
