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
            voice: VoiceSettings {
                provider: "local".into(),
                input_path: None,
                local_model_path: None,
                openai_api_key: None,
                openai_base_url: None,
                openai_model: None,
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
        }
    }
}

impl AppConfig {
    /// Returns the default directory for storing Gestura data (models, cache, config, etc.)
    /// On all platforms: ~/.gestura/
    pub fn data_dir() -> PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".gestura")
    }

    /// Returns the default config path.
    /// On all platforms: ~/.gestura/config.json
    /// Note: This does NOT create the directory - use data_dir() for that.
    pub fn default_path() -> PathBuf {
        Self::data_dir().join("config.json")
    }

    /// Check if a configuration file exists (for first-run detection).
    /// Returns true if the config file exists at the default path.
    /// Note: This does NOT create the config directory to preserve first-run detection.
    pub fn exists() -> bool {
        Self::default_path().exists()
    }

    /// Check if this is the first run of the application.
    /// Returns true if no config file exists yet.
    /// Note: This checks without creating the directory to preserve first-run state.
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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self).expect("serialize config");
        fs::write(path, data)
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
            // === English-optimized models ===
            WhisperModelInfo {
                name: "Base (English)".to_string(),
                filename: "ggml-base.en.bin".to_string(),
                size_mb: 142,
                description: "Fast, good accuracy for English. Recommended for most users.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin".to_string(),
                language: "English".to_string(),
                recommended: true,
            },
            WhisperModelInfo {
                name: "Tiny (English)".to_string(),
                filename: "ggml-tiny.en.bin".to_string(),
                size_mb: 75,
                description: "Fastest model, lower accuracy. Good for quick voice commands.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin".to_string(),
                language: "English".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Small (English)".to_string(),
                filename: "ggml-small.en.bin".to_string(),
                size_mb: 466,
                description: "Better accuracy, moderate speed. Good for clearer audio.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin".to_string(),
                language: "English".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Medium (English)".to_string(),
                filename: "ggml-medium.en.bin".to_string(),
                size_mb: 1500,
                description: "High accuracy, slower. Best for complex speech and accents.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin".to_string(),
                language: "English".to_string(),
                recommended: false,
            },
            // === Multilingual models ===
            WhisperModelInfo {
                name: "Tiny (Multilingual)".to_string(),
                filename: "ggml-tiny.bin".to_string(),
                size_mb: 75,
                description: "Fastest multilingual. Supports 99+ languages with basic accuracy.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin".to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Base (Multilingual)".to_string(),
                filename: "ggml-base.bin".to_string(),
                size_mb: 142,
                description: "Fast multilingual model. Good balance of speed and accuracy.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin".to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Small (Multilingual)".to_string(),
                filename: "ggml-small.bin".to_string(),
                size_mb: 466,
                description: "Better accuracy for non-English languages. Supports 99+ languages.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin".to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Medium (Multilingual)".to_string(),
                filename: "ggml-medium.bin".to_string(),
                size_mb: 1500,
                description: "High accuracy for all languages. Recommended for non-English use.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin".to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Large v3".to_string(),
                filename: "ggml-large-v3.bin".to_string(),
                size_mb: 3100,
                description: "Best accuracy, slowest. Latest model with superior multilingual support.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin".to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Large v3 Turbo".to_string(),
                filename: "ggml-large-v3-turbo.bin".to_string(),
                size_mb: 1600,
                description: "Fast large model. Near-best accuracy with better speed than Large v3.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin".to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
            // === Quantized models (smaller file sizes) ===
            WhisperModelInfo {
                name: "Small (Quantized Q5)".to_string(),
                filename: "ggml-small-q5_1.bin".to_string(),
                size_mb: 190,
                description: "Quantized small model. 60% smaller with minimal accuracy loss.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin".to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Medium (Quantized Q5)".to_string(),
                filename: "ggml-medium-q5_0.bin".to_string(),
                size_mb: 540,
                description: "Quantized medium model. 65% smaller, good for limited storage.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium-q5_0.bin".to_string(),
                language: "Multilingual (99+ languages)".to_string(),
                recommended: false,
            },
            WhisperModelInfo {
                name: "Large v3 Turbo (Quantized Q5)".to_string(),
                filename: "ggml-large-v3-turbo-q5_0.bin".to_string(),
                size_mb: 580,
                description: "Quantized turbo model. Best quality-to-size ratio for large models.".to_string(),
                url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin".to_string(),
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
        assert_eq!(c.llm.primary, "echo");
    }
}
