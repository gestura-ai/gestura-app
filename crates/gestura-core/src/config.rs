//! Configuration management for Gestura
//!
//! This module defines the AppConfig struct and load/save helpers.
//! Configuration is stored as JSON in ~/.gestura/config.json

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};

use crate::error::{AppError, Result};

/// Application configuration persisted to a JSON file.
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
    /// MDH pointer mappings
    pub mdh_pointers: HashMap<String, String>,
    /// UI preferences (theme, accent)
    pub ui: UiSettings,
    /// NATS URL for embedded MQ connectivity.
    pub nats_url: String,
    /// Developer and simulator settings
    pub developer: DeveloperSettings,
    /// Notification settings for response completion and feedback
    #[serde(default)]
    pub notifications: NotificationSettings,
    /// Web search configuration
    #[serde(default)]
    pub web_search: WebSearchConfig,
}

/// Notification settings for response completion and MCP feedback
///
/// NOTE: This struct is `#[serde(default)]` to allow seamless schema evolution
/// (adding new settings without breaking older config files).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NotificationSettings {
    /// Enable sound notifications on response completion
    pub sound_enabled: bool,
    /// Enable haptic notifications on response completion (requires connected ring)
    pub haptic_enabled: bool,
    /// Sound volume (0.0 to 1.0)
    pub sound_volume: u8, // 0-100, stored as u8 for Eq compatibility
    /// Haptic intensity (0.0 to 1.0)
    pub haptic_intensity: u8, // 0-100, stored as u8 for Eq compatibility
    /// Selected notification sound for general notifications.
    ///
    /// Expected values come from the config UI: "default" | "chime" | "ping" |
    /// "pop" | "subtle" | "none".
    pub notification_sound: String,
    /// Selected sound for command confirmations.
    ///
    /// Expected values come from the config UI: "default" | "success" | "click" |
    /// "beep" | "none".
    pub command_confirm_sound: String,
    /// Enable alternate notification pattern for MCP feedback requests
    pub mcp_feedback_enabled: bool,
    /// Auto-start listening after MCP feedback notification
    pub auto_listen_on_feedback: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            haptic_enabled: true,
            sound_volume: 70,
            haptic_intensity: 70,
            notification_sound: "default".to_string(),
            command_confirm_sound: "default".to_string(),
            mcp_feedback_enabled: true,
            auto_listen_on_feedback: true,
        }
    }
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

impl Default for SimulatorSettings {
    fn default() -> Self {
        Self {
            device_name_pattern: "Gestura Simulator*".to_string(),
            auto_connect: true,
            health_check_interval: 30,
            enable_metrics: true,
            discovery_port_range: (9000, 9100),
        }
    }
}

/// LLM settings grouping provider-specific configs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmSettings {
    /// Primary provider id: "openai" | "anthropic" | "grok" | "ollama"
    pub primary: String,
    /// Fallback provider id (optional): used when primary fails
    #[serde(default)]
    pub fallback: Option<String>,
    pub openai: Option<OpenAiConfig>,
    pub anthropic: Option<AnthropicConfig>,
    pub grok: Option<GrokConfig>,
    pub ollama: Option<OllamaConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAiConfig {
    pub api_key: String,
    pub base_url: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: Option<String>,
    pub model: String,

    /// Optional: enable Anthropic "extended thinking" streaming.
    ///
    /// When set, we send `thinking: { type: "enabled", budget_tokens: N }` in the request body.
    /// Only certain Claude models support this; if your model does not, Anthropic will reject the request.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrokConfig {
    pub api_key: String,
    pub base_url: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OllamaConfig {
    pub base_url: String,
    pub model: String,
}

/// Voice settings; default uses OpenAI Whisper API if api_key present
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceSettings {
    /// Preferred provider: "local" | "openai" | "none"
    pub provider: String,
    /// Optional input wav file path used for testing transcription
    pub input_path: Option<String>,
    /// Local whisper.cpp model path (.bin)
    pub local_model_path: Option<String>,
    /// OpenAI Whisper API settings (optional)
    pub openai_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_model: Option<String>,
    /// Selected audio input device name (None = use system default)
    #[serde(default)]
    pub audio_device: Option<String>,
}

/// MCP tool entry (basic)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    pub endpoint: String,
}

// ============================================================================
// Web Search Configuration
// ============================================================================

/// Web search provider selection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchProvider {
    /// Local HTTP-based search (no API key required) - DEFAULT
    /// Uses DuckDuckGo HTML scraping with smart content extraction
    #[default]
    Local,
    /// SerpAPI provider (requires API key)
    SerpApi,
    /// DuckDuckGo Instant Answer API (no API key, limited results)
    DuckDuckGo,
    /// Brave Search API (requires API key)
    Brave,
}

/// Web search configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WebSearchConfig {
    /// Primary search provider
    pub provider: WebSearchProvider,
    /// SerpAPI API key (optional)
    pub serpapi_key: Option<String>,
    /// Brave Search API key (optional)
    pub brave_key: Option<String>,
    /// Maximum number of search results to return
    pub max_results: usize,
    /// Request timeout in seconds
    pub timeout_secs: u64,
    /// User agent string for HTTP requests
    pub user_agent: String,
    /// Enable content extraction from search result pages
    pub extract_content: bool,
    /// Maximum content length per page (in characters)
    pub max_content_length: usize,
    /// Fallback providers if primary fails (in order)
    pub fallback_providers: Vec<WebSearchProvider>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: WebSearchProvider::Local,
            serpapi_key: None,
            brave_key: None,
            max_results: 5,
            timeout_secs: 30,
            user_agent: "Gestura/0.2.0 (+https://gestura.ai)".to_string(),
            extract_content: true,
            max_content_length: 10_000,
            fallback_providers: vec![WebSearchProvider::DuckDuckGo],
        }
    }
}

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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey_listen: "Ctrl+Space".to_string(),
            grace_period_secs: 30,
            llm: LlmSettings {
                primary: "anthropic".into(), // Default to Anthropic; user must configure API key
                fallback: Some("ollama".into()), // Fallback to local Ollama if available
                openai: None,
                anthropic: None,
                grok: None,
                // Provide sensible Ollama defaults so it works when selected
                ollama: Some(OllamaConfig {
                    base_url: "http://localhost:11434".into(),
                    model: "llama3.2".into(),
                }),
            },
            voice: VoiceSettings {
                provider: "local".into(),
                input_path: None,
                local_model_path: None,
                openai_api_key: None,
                openai_base_url: None,
                // Default to GPT-4o Transcribe for best accuracy when using OpenAI
                openai_model: Some("gpt-4o-transcribe".into()),
                audio_device: None,
            },
            mcp_tools: vec![],
            mdh_pointers: Default::default(),
            ui: UiSettings {
                theme_mode: "system".into(),
                accent: None,
            },
            nats_url: "nats://127.0.0.1:4223".to_string(),
            developer: DeveloperSettings::default(),
            notifications: NotificationSettings::default(),
            web_search: WebSearchConfig::default(),
        }
    }
}

impl AppConfig {
    /// Returns the default directory for storing Gestura data
    /// On all platforms: ~/.gestura/
    pub fn data_dir() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gestura")
    }

    /// Returns the default config path: ~/.gestura/config.json
    pub fn default_path() -> PathBuf {
        Self::data_dir().join("config.json")
    }

    /// Check if a configuration file exists
    pub fn exists() -> bool {
        Self::default_path().exists()
    }

    /// Check if this is the first run of the application
    pub fn is_first_run() -> bool {
        !Self::exists()
    }

    /// Returns the default directory for Whisper models
    pub fn whisper_models_dir() -> PathBuf {
        Self::data_dir().join("models").join("whisper")
    }

    /// Returns the default path for the recommended Whisper model
    pub fn default_whisper_model_path() -> PathBuf {
        Self::whisper_models_dir().join("ggml-base.en.bin")
    }

    /// Load configuration from disk, falling back to defaults if missing (sync version)
    pub fn load() -> Self {
        let path = Self::default_path();
        match fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Load configuration from disk asynchronously, falling back to defaults if missing
    ///
    /// This is the preferred method for GUI/Tauri commands to avoid blocking the UI thread.
    pub async fn load_async() -> Self {
        let path = Self::default_path();
        match tokio::fs::read_to_string(&path).await {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save configuration to disk (sync version)
    pub fn save(&self) -> Result<()> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;
        fs::write(path, data)?;
        Ok(())
    }

    /// Save configuration to disk asynchronously
    ///
    /// This is the preferred method for GUI/Tauri commands to avoid blocking the UI thread.
    pub async fn save_async(&self) -> Result<()> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;
        tokio::fs::write(path, data).await?;
        Ok(())
    }

    /// Get a config value by dot-notation key (e.g., "llm.primary")
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "hotkey_listen" => Some(self.hotkey_listen.clone()),
            "grace_period_secs" => Some(self.grace_period_secs.to_string()),
            "llm.primary" => Some(self.llm.primary.clone()),
            "voice.provider" => Some(self.voice.provider.clone()),
            "ui.theme_mode" => Some(self.ui.theme_mode.clone()),
            "nats_url" => Some(self.nats_url.clone()),
            _ => None,
        }
    }
}

/// Information about available Whisper models
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperModelInfo {
    pub name: String,
    pub filename: String,
    pub size_mb: u64,
    pub description: String,
    pub url: String,
    pub language: String,
    pub recommended: bool,
}

impl WhisperModelInfo {
    /// Get list of recommended Whisper models
    pub fn available_models() -> Vec<Self> {
        vec![
            WhisperModelInfo {
                name: "Base (English)".to_string(),
                filename: "ggml-base.en.bin".to_string(),
                size_mb: 142,
                description: "Fast, good accuracy for English. Recommended for most users."
                    .to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"
                    .to_string(),
                language: "English".to_string(),
                recommended: true,
            },
            WhisperModelInfo {
                name: "Tiny (English)".to_string(),
                filename: "ggml-tiny.en.bin".to_string(),
                size_mb: 75,
                description: "Fastest model, lower accuracy. Good for quick voice commands."
                    .to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin"
                    .to_string(),
                language: "English".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Small (English)".to_string(),
                filename: "ggml-small.en.bin".to_string(),
                size_mb: 466,
                description: "Better accuracy, moderate speed.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin"
                    .to_string(),
                language: "English".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Medium (English)".to_string(),
                filename: "ggml-medium.en.bin".to_string(),
                size_mb: 1500,
                description: "High accuracy, slower. Best for complex speech.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin"
                    .to_string(),
                language: "English".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Base (Multilingual)".to_string(),
                filename: "ggml-base.bin".to_string(),
                size_mb: 142,
                description: "Fast multilingual model. Good balance of speed and accuracy."
                    .to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
                    .to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
        ]
    }

    /// Find model info by filename
    pub fn find_by_filename(filename: &str) -> Option<Self> {
        Self::available_models()
            .into_iter()
            .find(|m| m.filename == filename)
    }

    /// Get the default/recommended model filename
    pub fn default_model_filename() -> &'static str {
        "ggml-base.en.bin"
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
        assert_eq!(c.llm.primary, "anthropic");
        assert_eq!(c.llm.fallback, Some("ollama".to_string()));
        // Ollama should have sensible defaults so it works when selected
        assert!(c.llm.ollama.is_some());
        let ollama = c.llm.ollama.unwrap();
        assert_eq!(ollama.base_url, "http://localhost:11434");
        assert_eq!(ollama.model, "llama3.2");
    }

    #[test]
    fn test_config_get() {
        let c = AppConfig::default();
        assert_eq!(c.get("llm.primary"), Some("anthropic".to_string()));
        assert_eq!(c.get("unknown.key"), None);
    }

    #[test]
    fn test_whisper_model_info() {
        let models = WhisperModelInfo::available_models();
        assert!(!models.is_empty());
        let recommended: Vec<_> = models.iter().filter(|m| m.recommended).collect();
        assert_eq!(recommended.len(), 1);
    }
}
