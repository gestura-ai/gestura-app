//! Configuration management for Gestura
//! This module defines the AppConfig struct and load/save helpers.

use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// Application configuration persisted to a JSON file.
/// Extend with additional settings in later stages (BLE, LLMs, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    /// Global hotkey to toggle the app or trigger recording.
    pub hotkey_listen: String,
    /// Grace period in seconds for agent shutdown.
    pub grace_period_secs: u32,
    /// LLM configuration and provider selection.
    pub llm: LlmSettings,
    /// Voice/STT configuration.
    pub voice: VoiceSettings,
    /// MCP tools configuration (names/endpoints) and MDH pointer map.
    pub mcp_tools: Vec<McpTool>,
    pub mdh_pointers: std::collections::HashMap<String, String>,
    /// UI preferences (theme, accent)
    pub ui: UiSettings,
    /// NATS URL for embedded MQ connectivity.
    pub nats_url: String,
    /// Developer and simulator settings
    pub developer: DeveloperSettings,
}

/// UI preferences including theme mode and accent color.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiSettings {
    /// Theme mode: "system" | "light" | "dark"
    pub theme_mode: String,
    /// Optional accent color token (e.g., "blue", "amber", or hex)
    pub accent: Option<String>,
}

/// Developer and simulator settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeveloperSettings {
    /// Enable developer mode features
    pub developer_mode: bool,
    /// Enable simulator support
    pub enable_simulators: bool,
    /// Auto-discover simulators on localhost
    pub auto_discover_simulators: bool,
    /// Show detailed BLE connection logs
    pub verbose_ble_logging: bool,
    /// Simulator-specific configuration
    pub simulator: SimulatorSettings,
}

/// Simulator-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulatorSettings {
    /// Default simulator device name pattern
    pub device_name_pattern: String,
    /// Auto-connect to simulators when found
    pub auto_connect: bool,
    /// Simulator health check interval in seconds
    pub health_check_interval: u32,
    /// Enable simulator performance metrics
    pub enable_metrics: bool,
    /// Localhost discovery port range
    pub discovery_port_range: (u16, u16),
}

/// LLM settings grouping provider-specific configs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmSettings {
    /// Primary provider id: "openai" | "anthropic" | "grok" | "ollama" | "echo"
    pub primary: String,
    pub openai: Option<OpenAiConfig>,
    pub anthropic: Option<AnthropicConfig>,
    pub grok: Option<GrokConfig>,
    pub ollama: Option<OllamaConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAiConfig { pub api_key: String, pub base_url: Option<String>, pub model: String }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnthropicConfig { pub api_key: String, pub base_url: Option<String>, pub model: String }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrokConfig { pub api_key: String, pub base_url: Option<String>, pub model: String }
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OllamaConfig { pub base_url: String, pub model: String }

/// Voice settings; default uses OpenAI Whisper API if api_key present
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceSettings {
    /// Preferred provider: "local" | "openai" | "none". Local is preferred if a model is available.
    pub provider: String,
    /// Optional input wav file path used for testing transcription
    pub input_path: Option<String>,
    /// Local whisper.cpp model path (.bin); if present, local engine will be used first.
    pub local_model_path: Option<String>,
    /// OpenAI Whisper API settings (optional)
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: Option<String>,
}

/// MCP tool entry (basic)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpTool { pub name: String, pub endpoint: String }

impl Default for DeveloperSettings {
    fn default() -> Self {
        Self {
            developer_mode: false,
            enable_simulators: true,
            auto_discover_simulators: true,
            verbose_ble_logging: false,
            simulator: SimulatorSettings::default(),
        }
    }
}

impl Default for SimulatorSettings {
    fn default() -> Self {
        Self {
            device_name_pattern: "Haptic Harmony Ring Simulator".to_string(),
            auto_connect: true,
            health_check_interval: 30,
            enable_metrics: true,
            discovery_port_range: (8080, 8090),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey_listen: "Ctrl+Space".to_string(),
            grace_period_secs: 30,
            llm: LlmSettings {
                primary: "echo".into(),
                openai: None,
                anthropic: None,
                grok: None,
                ollama: None,
            },
            voice: VoiceSettings { provider: "local".into(), input_path: None, local_model_path: None, openai_api_key: None, openai_base_url: None, openai_model: None },
            mcp_tools: vec![],
            mdh_pointers: Default::default(),
            ui: UiSettings { theme_mode: "system".into(), accent: None },
            nats_url: "nats://127.0.0.1:4223".to_string(),
            developer: DeveloperSettings::default(),
        }
    }
}

impl AppConfig {
    /// Returns the default config path in the user's config directory.
    /// On macOS: ~/Library/Application Support/Gestura/config.json
    /// On Linux: ~/.config/Gestura/config.json
    /// On Windows: %APPDATA%/Gestura/config.json
    pub fn default_path() -> PathBuf {
        let mut dir = dirs::config_dir().unwrap_or_default();
        dir.push("Gestura");
        fs::create_dir_all(&dir).ok();
        dir.push("config.json");
        dir
    }

    /// Load configuration from disk, falling back to defaults if missing or malformed.
    pub fn load() -> Self {
        let path = Self::default_path();
        match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save configuration to disk, returning an error if writing fails.
    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        let data = serde_json::to_string_pretty(self).expect("serialize config");
        fs::write(path, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let c = AppConfig::default();
        assert_eq!(c.hotkey_listen, "Ctrl+Space");
        assert_eq!(c.grace_period_secs, 30);
        assert_eq!(c.llm.primary, "echo");
    }
}

