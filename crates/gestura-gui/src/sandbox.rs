//! Agent sandboxing and isolation utilities
//! Provides security boundaries for agent processes

use crate::AppError;
use std::collections::HashMap;
use std::path::PathBuf;

/// Sandbox configuration for agent processes
#[derive(Debug, Clone)]
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

/// Sandbox manager for agent processes
pub struct SandboxManager {
    configs: HashMap<String, SandboxConfig>,
}

impl Default for SandboxManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxManager {
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
            allowed_read_paths = ?config.allowed_read_paths,
            allowed_write_paths = ?config.allowed_write_paths,
            allowed_hosts = ?config.allowed_hosts,
            "Registering sandbox config for agent"
        );
        self.configs.insert(agent_id.to_string(), config);
    }

    /// Get sandbox config for an agent
    pub fn get_config(&self, agent_id: &str) -> SandboxConfig {
        self.configs.get(agent_id).cloned().unwrap_or_default()
    }

    /// Apply sandbox restrictions to a command
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
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "Write access denied for agent {} to path: {:?}",
                    agent_id, path
                ),
            )))
        } else {
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
            Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "Read access denied for agent {} to path: {:?}",
                    agent_id, path
                ),
            )))
        }
    }

    /// Validate network access for an agent
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
        Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "Network access denied for agent {} to host: {}",
                agent_id, host
            ),
        )))
    }
}

/// Create default sandbox config for different agent types
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
        allowed_read_paths = ?config.allowed_read_paths,
        allowed_hosts = ?config.allowed_hosts,
        "Created sandbox config"
    );

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_config_creation() {
        let voice_config = create_default_sandbox("voice-agent");
        assert_eq!(voice_config.max_memory_mb, 256);
        assert_eq!(voice_config.max_cpu_time_secs, 60);

        let mcp_config = create_default_sandbox("mcp-agent");
        assert_eq!(mcp_config.max_memory_mb, 512);
        assert!(mcp_config.allowed_hosts.contains(&"localhost".to_string()));
    }

    #[test]
    fn test_file_access_validation() {
        let mut manager = SandboxManager::new();
        let mut config = SandboxConfig::default();
        config.allowed_read_paths.push(PathBuf::from("/tmp"));
        config.allowed_write_paths.push(PathBuf::from("/var/app"));

        manager.register_agent("test-agent", config);

        // Test read access
        let result =
            manager.validate_file_access("test-agent", &PathBuf::from("/tmp/test.txt"), false);
        assert!(result.is_ok());

        // Test write access
        let result =
            manager.validate_file_access("test-agent", &PathBuf::from("/var/app/data.json"), true);
        assert!(result.is_ok());

        // Test denied access
        let result =
            manager.validate_file_access("test-agent", &PathBuf::from("/etc/passwd"), false);
        assert!(result.is_err());
    }
}
