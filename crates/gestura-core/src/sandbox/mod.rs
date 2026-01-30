//! Agent sandboxing and isolation utilities
//!
//! Provides security boundaries for agent processes, including:
//! - Resource limits (memory, CPU time)
//! - File system access control
//! - Network access control
//!
//! # Example
//!
//! ```rust,ignore
//! use gestura_core::sandbox::{SandboxConfig, SandboxManager, create_default_sandbox};
//!
//! let mut manager = SandboxManager::new();
//! let config = create_default_sandbox("mcp-agent");
//! manager.register_agent("my-agent", config);
//!
//! // Validate file access
//! manager.validate_file_access("my-agent", &path, false)?;
//! ```

use crate::error::AppError;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sandbox configuration for agent processes
///
/// Defines resource limits and access controls for an agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxConfig {
    /// Maximum memory usage in MB
    pub max_memory_mb: u64,
    /// Maximum CPU time in seconds
    pub max_cpu_time_secs: u64,
    /// Allowed file system paths (read-only)
    pub allowed_read_paths: Vec<PathBuf>,
    /// Allowed file system paths (read-write)
    pub allowed_write_paths: Vec<PathBuf>,
    /// Allowed network hosts
    pub allowed_hosts: Vec<String>,
    /// Environment variables to pass through
    pub env_vars: HashMap<String, String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: 512,
            max_cpu_time_secs: 300,
            allowed_read_paths: vec![],
            allowed_write_paths: vec![],
            allowed_hosts: vec![],
            env_vars: HashMap::new(),
        }
    }
}

impl SandboxConfig {
    /// Create a new sandbox config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Set memory limit in MB
    pub fn with_memory_limit(mut self, mb: u64) -> Self {
        self.max_memory_mb = mb;
        self
    }

    /// Set CPU time limit in seconds
    pub fn with_cpu_limit(mut self, secs: u64) -> Self {
        self.max_cpu_time_secs = secs;
        self
    }

    /// Add a read-only path
    pub fn with_read_path(mut self, path: PathBuf) -> Self {
        self.allowed_read_paths.push(path);
        self
    }

    /// Add a read-write path
    pub fn with_write_path(mut self, path: PathBuf) -> Self {
        self.allowed_write_paths.push(path);
        self
    }

    /// Add an allowed network host
    pub fn with_allowed_host(mut self, host: String) -> Self {
        self.allowed_hosts.push(host);
        self
    }

    /// Set an environment variable
    pub fn with_env(mut self, key: String, value: String) -> Self {
        self.env_vars.insert(key, value);
        self
    }
}

/// Sandbox manager for agent processes
///
/// Manages sandbox configurations for multiple agents and provides
/// validation methods for file and network access.
pub struct SandboxManager {
    configs: HashMap<String, SandboxConfig>,
}

impl Default for SandboxManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxManager {
    /// Create a new sandbox manager
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
        }
    }

    /// Register sandbox config for an agent
    pub fn register_agent(&mut self, agent_id: &str, config: SandboxConfig) {
        tracing::debug!(
            agent_id = %agent_id,
            max_memory_mb = config.max_memory_mb,
            max_cpu_time_secs = config.max_cpu_time_secs,
            "Registering sandbox config for agent"
        );
        self.configs.insert(agent_id.to_string(), config);
    }

    /// Unregister an agent's sandbox config
    pub fn unregister_agent(&mut self, agent_id: &str) {
        self.configs.remove(agent_id);
    }

    /// Get sandbox config for an agent
    pub fn get_config(&self, agent_id: &str) -> SandboxConfig {
        self.configs.get(agent_id).cloned().unwrap_or_default()
    }

    /// Check if an agent has a registered config
    pub fn has_agent(&self, agent_id: &str) -> bool {
        self.configs.contains_key(agent_id)
    }

    /// List all registered agent IDs
    pub fn list_agents(&self) -> Vec<String> {
        self.configs.keys().cloned().collect()
    }

    /// Apply sandbox restrictions to a command
    ///
    /// Sets resource limits and environment variables on the command.
    pub fn apply_sandbox(
        &self,
        agent_id: &str,
        mut cmd: tokio::process::Command,
    ) -> tokio::process::Command {
        let config = self.get_config(agent_id);

        // Set resource limits (platform-specific)
        #[cfg(unix)]
        {
            // On Unix systems, we can use ulimit-style restrictions
            // This is a simplified approach - production would use cgroups or similar
            cmd.env(
                "RLIMIT_AS",
                (config.max_memory_mb * 1024 * 1024).to_string(),
            );
            cmd.env("RLIMIT_CPU", config.max_cpu_time_secs.to_string());
        }

        // Set allowed environment variables
        cmd.env_clear();
        for (key, value) in &config.env_vars {
            cmd.env(key, value);
        }

        // Add basic security environment
        cmd.env("GESTURA_AGENT_ID", agent_id);
        cmd.env("GESTURA_SANDBOX", "1");

        tracing::info!("Applied sandbox config for agent: {}", agent_id);
        cmd
    }

    /// Validate file access for an agent
    ///
    /// Returns Ok if the agent is allowed to access the path, Err otherwise.
    pub fn validate_file_access(
        &self,
        agent_id: &str,
        path: &PathBuf,
        write_access: bool,
    ) -> Result<(), AppError> {
        let config = self.get_config(agent_id);
        let access_type = if write_access { "write" } else { "read" };

        tracing::debug!(
            agent_id = %agent_id,
            path = ?path,
            access_type = %access_type,
            "Validating file access for agent"
        );

        if write_access {
            for allowed_path in &config.allowed_write_paths {
                if path.starts_with(allowed_path) {
                    tracing::debug!(
                        agent_id = %agent_id,
                        path = ?path,
                        matched_pattern = ?allowed_path,
                        "File write access granted"
                    );
                    return Ok(());
                }
            }
            tracing::warn!(
                agent_id = %agent_id,
                path = ?path,
                allowed_write_paths = ?config.allowed_write_paths,
                "File write access denied"
            );
            Err(AppError::PermissionDenied(format!(
                "Write access denied for agent {} to path: {:?}",
                agent_id, path
            )))
        } else {
            // Check read paths first
            for allowed_path in &config.allowed_read_paths {
                if path.starts_with(allowed_path) {
                    tracing::debug!(
                        agent_id = %agent_id,
                        path = ?path,
                        matched_pattern = ?allowed_path,
                        "File read access granted (read path)"
                    );
                    return Ok(());
                }
            }
            // Write paths also grant read access
            for allowed_path in &config.allowed_write_paths {
                if path.starts_with(allowed_path) {
                    tracing::debug!(
                        agent_id = %agent_id,
                        path = ?path,
                        matched_pattern = ?allowed_path,
                        "File read access granted (write path)"
                    );
                    return Ok(());
                }
            }
            tracing::warn!(
                agent_id = %agent_id,
                path = ?path,
                allowed_read_paths = ?config.allowed_read_paths,
                allowed_write_paths = ?config.allowed_write_paths,
                "File read access denied"
            );
            Err(AppError::PermissionDenied(format!(
                "Read access denied for agent {} to path: {:?}",
                agent_id, path
            )))
        }
    }

    /// Validate network access for an agent
    ///
    /// Returns Ok if the agent is allowed to access the host, Err otherwise.
    pub fn validate_network_access(&self, agent_id: &str, host: &str) -> Result<(), AppError> {
        let config = self.get_config(agent_id);

        tracing::debug!(
            agent_id = %agent_id,
            host = %host,
            "Validating network access for agent"
        );

        if config.allowed_hosts.is_empty() {
            // If no hosts specified, allow all (permissive default)
            tracing::debug!(
                agent_id = %agent_id,
                host = %host,
                "Network access granted (permissive default - no host restrictions)"
            );
            return Ok(());
        }

        for allowed_host in &config.allowed_hosts {
            if host == allowed_host || host.ends_with(&format!(".{}", allowed_host)) {
                tracing::debug!(
                    agent_id = %agent_id,
                    host = %host,
                    matched_pattern = %allowed_host,
                    "Network access granted"
                );
                return Ok(());
            }
        }

        tracing::warn!(
            agent_id = %agent_id,
            host = %host,
            allowed_hosts = ?config.allowed_hosts,
            "Network access denied"
        );
        Err(AppError::PermissionDenied(format!(
            "Network access denied for agent {} to host: {}",
            agent_id, host
        )))
    }
}

/// Create default sandbox config for different agent types
///
/// Returns a pre-configured sandbox based on the agent type.
pub fn create_default_sandbox(agent_type: &str) -> SandboxConfig {
    tracing::debug!(agent_type = %agent_type, "Creating default sandbox config");

    let mut config = SandboxConfig::default();

    match agent_type {
        "voice-agent" => {
            config.max_memory_mb = 256;
            config.max_cpu_time_secs = 60;
            // Allow access to temp directory for audio files
            if let Some(temp_dir) = std::env::temp_dir().to_str() {
                config.allowed_read_paths.push(PathBuf::from(temp_dir));
            }
        }
        "mcp-agent" => {
            config.max_memory_mb = 512;
            config.max_cpu_time_secs = 300;
            config.allowed_hosts = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        }
        "default-agent" => {
            config.max_memory_mb = 128;
            config.max_cpu_time_secs = 120;
        }
        _ => {
            tracing::debug!(
                agent_type = %agent_type,
                "Unknown agent type, using restrictive defaults"
            );
            // Use restrictive defaults for unknown agent types
            config.max_memory_mb = 64;
            config.max_cpu_time_secs = 30;
        }
    }

    tracing::debug!(
        agent_type = %agent_type,
        max_memory_mb = config.max_memory_mb,
        max_cpu_time_secs = config.max_cpu_time_secs,
        "Created sandbox config"
    );

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_config_defaults() {
        let config = SandboxConfig::default();
        assert_eq!(config.max_memory_mb, 512);
        assert_eq!(config.max_cpu_time_secs, 300);
        assert!(config.allowed_read_paths.is_empty());
        assert!(config.allowed_write_paths.is_empty());
        assert!(config.allowed_hosts.is_empty());
    }

    #[test]
    fn test_sandbox_config_builder() {
        let config = SandboxConfig::new()
            .with_memory_limit(256)
            .with_cpu_limit(60)
            .with_read_path(PathBuf::from("/tmp"))
            .with_write_path(PathBuf::from("/var/app"))
            .with_allowed_host("localhost".to_string())
            .with_env("KEY".to_string(), "VALUE".to_string());

        assert_eq!(config.max_memory_mb, 256);
        assert_eq!(config.max_cpu_time_secs, 60);
        assert_eq!(config.allowed_read_paths.len(), 1);
        assert_eq!(config.allowed_write_paths.len(), 1);
        assert_eq!(config.allowed_hosts.len(), 1);
        assert_eq!(config.env_vars.get("KEY"), Some(&"VALUE".to_string()));
    }

    #[test]
    fn test_sandbox_config_creation() {
        let voice_config = create_default_sandbox("voice-agent");
        assert_eq!(voice_config.max_memory_mb, 256);
        assert_eq!(voice_config.max_cpu_time_secs, 60);

        let mcp_config = create_default_sandbox("mcp-agent");
        assert_eq!(mcp_config.max_memory_mb, 512);
        assert!(mcp_config.allowed_hosts.contains(&"localhost".to_string()));

        let default_config = create_default_sandbox("default-agent");
        assert_eq!(default_config.max_memory_mb, 128);

        let unknown_config = create_default_sandbox("unknown");
        assert_eq!(unknown_config.max_memory_mb, 64);
    }

    #[test]
    fn test_sandbox_manager_registration() {
        let mut manager = SandboxManager::new();
        assert!(!manager.has_agent("test"));

        let config = SandboxConfig::default();
        manager.register_agent("test", config);
        assert!(manager.has_agent("test"));

        let agents = manager.list_agents();
        assert!(agents.contains(&"test".to_string()));

        manager.unregister_agent("test");
        assert!(!manager.has_agent("test"));
    }

    #[test]
    fn test_file_access_validation() {
        let mut manager = SandboxManager::new();
        let config = SandboxConfig::new()
            .with_read_path(PathBuf::from("/tmp"))
            .with_write_path(PathBuf::from("/var/app"));
        manager.register_agent("test-agent", config);

        // Test read access to allowed read path
        let result =
            manager.validate_file_access("test-agent", &PathBuf::from("/tmp/test.txt"), false);
        assert!(result.is_ok());

        // Test write access to allowed write path
        let result =
            manager.validate_file_access("test-agent", &PathBuf::from("/var/app/data.json"), true);
        assert!(result.is_ok());

        // Test read access via write path
        let result =
            manager.validate_file_access("test-agent", &PathBuf::from("/var/app/data.json"), false);
        assert!(result.is_ok());

        // Test denied read access
        let result =
            manager.validate_file_access("test-agent", &PathBuf::from("/etc/passwd"), false);
        assert!(result.is_err());

        // Test denied write access
        let result =
            manager.validate_file_access("test-agent", &PathBuf::from("/tmp/test.txt"), true);
        assert!(result.is_err());
    }

    #[test]
    fn test_network_access_validation() {
        let mut manager = SandboxManager::new();
        let config = SandboxConfig::new()
            .with_allowed_host("localhost".to_string())
            .with_allowed_host("api.example.com".to_string());
        manager.register_agent("test-agent", config);

        // Test allowed exact match
        assert!(
            manager
                .validate_network_access("test-agent", "localhost")
                .is_ok()
        );
        assert!(
            manager
                .validate_network_access("test-agent", "api.example.com")
                .is_ok()
        );

        // Test allowed subdomain
        assert!(
            manager
                .validate_network_access("test-agent", "sub.api.example.com")
                .is_ok()
        );

        // Test denied host
        assert!(
            manager
                .validate_network_access("test-agent", "evil.com")
                .is_err()
        );
    }

    #[test]
    fn test_network_access_permissive_default() {
        let mut manager = SandboxManager::new();
        let config = SandboxConfig::default(); // No hosts specified
        manager.register_agent("permissive", config);

        // Should allow all hosts when no restrictions specified
        assert!(
            manager
                .validate_network_access("permissive", "any.host.com")
                .is_ok()
        );
    }
}
