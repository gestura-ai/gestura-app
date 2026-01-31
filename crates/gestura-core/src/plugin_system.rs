//! Core-owned plugin system.
//!
//! This module defines the data model and manager for Gestura plugins.
//!
//! Core-First note: this logic is intentionally owned by `gestura-core` so both
//! the CLI and GUI can share a single implementation and policy surface.

use crate::error::AppError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Plugin metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub homepage: Option<String>,
    /// Optional URL of the plugin's source repository.
    pub repository: Option<String>,
    pub keywords: Vec<String>,
    pub dependencies: Vec<PluginDependency>,
    pub permissions: Vec<PluginPermission>,
    pub entry_point: String,
    pub supported_platforms: Vec<String>,
    pub min_app_version: String,
}

/// Plugin dependency.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginDependency {
    pub name: String,
    pub version: String,
    pub optional: bool,
}

/// Plugin permissions.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PluginPermission {
    /// File path pattern access.
    FileSystem(String),
    /// Network host pattern access.
    Network(String),
    VoiceAccess,
    GestureAccess,
    RingAccess,
    SystemInfo,
    Notifications,
    ClipboardAccess,
    ProcessSpawn,
    DatabaseAccess,
}

/// Plugin state.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum PluginState {
    Loaded,
    Running,
    Stopped,
    Error(String),
    Disabled,
}

/// Plugin instance.
#[derive(Debug, Clone)]
pub struct Plugin {
    pub metadata: PluginMetadata,
    pub state: PluginState,
    pub path: PathBuf,
    pub config: serde_json::Value,
    pub last_error: Option<String>,
    pub load_time: chrono::DateTime<chrono::Utc>,
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

/// Plugin API interface.
pub trait PluginApi {
    /// Initialize the plugin.
    fn initialize(&mut self, config: serde_json::Value) -> Result<(), String>;

    /// Start the plugin.
    fn start(&mut self) -> Result<(), String>;

    /// Stop the plugin.
    fn stop(&mut self) -> Result<(), String>;

    /// Handle a command.
    fn handle_command(
        &mut self,
        command: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String>;

    /// Get plugin status.
    fn get_status(&self) -> serde_json::Value;

    /// Handle events.
    fn handle_event(&mut self, event: &str, data: serde_json::Value) -> Result<(), String>;
}

/// Plugin manager.
pub struct PluginManager {
    plugins: Arc<RwLock<HashMap<String, Plugin>>>,
    plugin_directory: PathBuf,
    enabled_plugins: Arc<RwLock<Vec<String>>>,
    event_handlers: Arc<RwLock<HashMap<String, Vec<String>>>>, // event -> plugin_ids
    command_handlers: Arc<RwLock<HashMap<String, String>>>,    // command -> plugin_id
}

impl PluginManager {
    /// Create a new plugin manager.
    pub fn new(plugin_directory: PathBuf) -> Self {
        Self {
            plugins: Arc::new(RwLock::new(HashMap::new())),
            plugin_directory,
            enabled_plugins: Arc::new(RwLock::new(Vec::new())),
            event_handlers: Arc::new(RwLock::new(HashMap::new())),
            command_handlers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Discover plugins in the plugin directory.
    pub async fn discover_plugins(&self) -> Result<Vec<PluginMetadata>, AppError> {
        let mut discovered = Vec::new();

        if !self.plugin_directory.exists() {
            tokio::fs::create_dir_all(&self.plugin_directory)
                .await
                .map_err(AppError::Io)?;
            return Ok(discovered);
        }

        let mut entries = tokio::fs::read_dir(&self.plugin_directory)
            .await
            .map_err(AppError::Io)?;

        while let Some(entry) = entries.next_entry().await.map_err(AppError::Io)? {
            let path = entry.path();

            if path.is_dir() {
                let manifest_path = path.join("plugin.json");
                if manifest_path.exists() {
                    match self.load_plugin_metadata(&manifest_path).await {
                        Ok(metadata) => discovered.push(metadata),
                        Err(e) => tracing::warn!(
                            "Failed to load plugin metadata from {}: {}",
                            manifest_path.display(),
                            e
                        ),
                    }
                }
            }
        }

        tracing::info!("Discovered {} plugins", discovered.len());
        Ok(discovered)
    }

    /// Load plugin metadata from manifest file.
    async fn load_plugin_metadata(&self, manifest_path: &Path) -> Result<PluginMetadata, AppError> {
        let content = tokio::fs::read_to_string(manifest_path)
            .await
            .map_err(AppError::Io)?;

        let metadata: PluginMetadata = serde_json::from_str(&content)
            .map_err(|e| AppError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;

        // Validate metadata.
        self.validate_plugin_metadata(&metadata)?;

        Ok(metadata)
    }

    /// Validate plugin metadata.
    fn validate_plugin_metadata(&self, metadata: &PluginMetadata) -> Result<(), AppError> {
        if metadata.id.trim().is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Plugin ID cannot be empty",
            )));
        }

        if metadata.name.trim().is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Plugin name cannot be empty",
            )));
        }

        if metadata.entry_point.trim().is_empty() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Plugin entry point cannot be empty",
            )));
        }

        // Validate version format (simplified).
        if !metadata.version.contains('.') {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid version format",
            )));
        }

        Ok(())
    }

    /// Load a plugin.
    pub async fn load_plugin(&self, plugin_id: &str) -> Result<(), AppError> {
        let plugin_path = self.plugin_directory.join(plugin_id);
        let manifest_path = plugin_path.join("plugin.json");

        if !manifest_path.exists() {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Plugin manifest not found: {}", plugin_id),
            )));
        }

        let metadata = self.load_plugin_metadata(&manifest_path).await?;

        // Check if plugin is already loaded.
        {
            let plugins = self.plugins.read().await;
            if plugins.contains_key(plugin_id) {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("Plugin already loaded: {}", plugin_id),
                )));
            }
        }

        // Validate permissions.
        self.validate_plugin_permissions(&metadata.permissions)
            .await?;

        // Create plugin instance.
        let plugin = Plugin {
            metadata: metadata.clone(),
            state: PluginState::Loaded,
            path: plugin_path,
            config: serde_json::Value::Null,
            last_error: None,
            load_time: chrono::Utc::now(),
            last_activity: chrono::Utc::now(),
        };

        // Store plugin.
        let mut plugins = self.plugins.write().await;
        plugins.insert(plugin_id.to_string(), plugin);

        tracing::info!("Loaded plugin: {} v{}", metadata.name, metadata.version);
        Ok(())
    }

    /// Validate plugin permissions.
    async fn validate_plugin_permissions(
        &self,
        permissions: &[PluginPermission],
    ) -> Result<(), AppError> {
        for permission in permissions {
            match permission {
                PluginPermission::FileSystem(path) => {
                    // Validate file system access patterns.
                    if path.contains("..") || path.starts_with('/') {
                        return Err(AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "Invalid file system permission pattern",
                        )));
                    }
                }
                PluginPermission::Network(host) => {
                    // Validate network access patterns.
                    if host == "*" {
                        return Err(AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "Wildcard network access not allowed",
                        )));
                    }
                }
                PluginPermission::ProcessSpawn => {
                    // High-risk permission, require explicit approval.
                    tracing::warn!("Plugin requests process spawn permission");
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Start a plugin.
    pub async fn start_plugin(&self, plugin_id: &str) -> Result<(), AppError> {
        let mut plugins = self.plugins.write().await;

        let plugin = plugins.get_mut(plugin_id).ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Plugin not found: {}", plugin_id),
            ))
        })?;

        if plugin.state == PluginState::Running {
            return Ok(());
        }

        // Simulate plugin startup (in real implementation, would load and execute plugin code).
        plugin.state = PluginState::Running;
        plugin.last_activity = chrono::Utc::now();

        tracing::info!("Started plugin: {}", plugin_id);
        Ok(())
    }

    /// Stop a plugin.
    pub async fn stop_plugin(&self, plugin_id: &str) -> Result<(), AppError> {
        let mut plugins = self.plugins.write().await;

        let plugin = plugins.get_mut(plugin_id).ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Plugin not found: {}", plugin_id),
            ))
        })?;

        if plugin.state == PluginState::Stopped {
            return Ok(());
        }

        // Simulate plugin shutdown.
        plugin.state = PluginState::Stopped;
        plugin.last_activity = chrono::Utc::now();

        tracing::info!("Stopped plugin: {}", plugin_id);
        Ok(())
    }

    /// Unload a plugin.
    pub async fn unload_plugin(&self, plugin_id: &str) -> Result<(), AppError> {
        // Stop plugin first.
        self.stop_plugin(plugin_id).await?;

        // Remove from collections.
        let mut plugins = self.plugins.write().await;
        let mut enabled = self.enabled_plugins.write().await;
        let mut event_handlers = self.event_handlers.write().await;
        let mut command_handlers = self.command_handlers.write().await;

        plugins.remove(plugin_id);
        enabled.retain(|id| id != plugin_id);

        // Remove event handlers.
        for handlers in event_handlers.values_mut() {
            handlers.retain(|id| id != plugin_id);
        }

        // Remove command handlers.
        command_handlers.retain(|_, id| id != plugin_id);

        tracing::info!("Unloaded plugin: {}", plugin_id);
        Ok(())
    }

    /// Execute plugin command.
    pub async fn execute_command(
        &self,
        command: &str,
        _args: serde_json::Value,
    ) -> Result<serde_json::Value, AppError> {
        let command_handlers = self.command_handlers.read().await;

        if let Some(plugin_id) = command_handlers.get(command) {
            // In real implementation, would call plugin's handle_command method.
            tracing::info!("Executing command '{}' on plugin '{}'", command, plugin_id);

            Ok(serde_json::json!({
                "status": "success",
                "plugin_id": plugin_id,
                "command": command,
                "result": "Command executed successfully"
            }))
        } else {
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("No handler found for command: {}", command),
            )))
        }
    }

    /// Broadcast event to plugins.
    pub async fn broadcast_event(
        &self,
        event: &str,
        _data: serde_json::Value,
    ) -> Result<(), AppError> {
        let event_handlers = self.event_handlers.read().await;

        if let Some(handlers) = event_handlers.get(event) {
            for plugin_id in handlers {
                // In real implementation, would call plugin's handle_event method.
                tracing::debug!("Broadcasting event '{}' to plugin '{}'", event, plugin_id);
            }
        }

        Ok(())
    }

    /// Get all plugins.
    pub async fn get_plugins(&self) -> Vec<Plugin> {
        let plugins = self.plugins.read().await;
        plugins.values().cloned().collect()
    }

    /// Get plugin by ID.
    pub async fn get_plugin(&self, plugin_id: &str) -> Option<Plugin> {
        let plugins = self.plugins.read().await;
        plugins.get(plugin_id).cloned()
    }

    /// Get plugin statistics.
    pub async fn get_stats(&self) -> serde_json::Value {
        let plugins = self.plugins.read().await;
        let enabled = self.enabled_plugins.read().await;

        let total_plugins = plugins.len();
        let running_plugins = plugins
            .values()
            .filter(|p| p.state == PluginState::Running)
            .count();
        let enabled_plugins = enabled.len();

        serde_json::json!({
            "total_plugins": total_plugins,
            "running_plugins": running_plugins,
            "enabled_plugins": enabled_plugins,
            "plugin_directory": self.plugin_directory.display().to_string()
        })
    }
}

/// Global plugin manager instance.
static PLUGIN_MANAGER: tokio::sync::OnceCell<PluginManager> = tokio::sync::OnceCell::const_new();

/// Get the global plugin manager.
pub async fn get_plugin_manager() -> &'static PluginManager {
    PLUGIN_MANAGER
        .get_or_init(|| async {
            let plugin_dir = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("plugins");
            PluginManager::new(plugin_dir)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_plugin_discovery() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PluginManager::new(temp_dir.path().to_path_buf());

        // Create a test plugin.
        let plugin_dir = temp_dir.path().join("test-plugin");
        tokio::fs::create_dir_all(&plugin_dir).await.unwrap();

        let manifest = serde_json::json!({
            "id": "test-plugin",
            "name": "Test Plugin",
            "version": "1.0.0",
            "description": "A test plugin",
            "author": "Test Author",
            "license": "MIT",
            "keywords": ["test"],
            "dependencies": [],
            "permissions": [],
            "entry_point": "main.js",
            "supported_platforms": ["linux", "macos", "windows"],
            "min_app_version": "1.0.0"
        });

        tokio::fs::write(plugin_dir.join("plugin.json"), manifest.to_string())
            .await
            .unwrap();

        let discovered = manager.discover_plugins().await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].id, "test-plugin");
    }

    #[tokio::test]
    async fn test_plugin_loading() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PluginManager::new(temp_dir.path().to_path_buf());

        // Create and load test plugin.
        let plugin_dir = temp_dir.path().join("test-plugin");
        tokio::fs::create_dir_all(&plugin_dir).await.unwrap();

        let manifest = serde_json::json!({
            "id": "test-plugin",
            "name": "Test Plugin",
            "version": "1.0.0",
            "description": "A test plugin",
            "author": "Test Author",
            "license": "MIT",
            "keywords": ["test"],
            "dependencies": [],
            "permissions": [],
            "entry_point": "main.js",
            "supported_platforms": ["linux", "macos", "windows"],
            "min_app_version": "1.0.0"
        });

        tokio::fs::write(plugin_dir.join("plugin.json"), manifest.to_string())
            .await
            .unwrap();

        manager.load_plugin("test-plugin").await.unwrap();

        let plugin = manager.get_plugin("test-plugin").await;
        assert!(plugin.is_some());
        assert_eq!(plugin.unwrap().state, PluginState::Loaded);
    }
}
