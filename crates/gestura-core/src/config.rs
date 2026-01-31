//! Configuration management for Gestura
//!
//! This module defines the AppConfig struct and load/save helpers.
//! Configuration is stored as YAML in `~/.gestura/config.yaml`.
//!
//! ## Backward compatibility
//!
//! Older versions stored configuration as JSON in `~/.gestura/config.json`.
//! On load, if `config.yaml` does not exist but `config.json` does, we
//! automatically migrate the JSON file to YAML.
//!
//! ## Configuration Precedence
//!
//! Configuration values are loaded with the following precedence (highest first):
//! 1. Environment variables (GESTURA_* prefix)
//! 2. Config file (`~/.gestura/config.yaml`)
//! 3. Default values
//!
//! See [`crate::config_env`] for environment variable documentation.

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path, path::PathBuf};

use crate::config_env::{get_env, get_env_bool, get_env_u32};
use crate::error::{AppError, Result};

/// Global permission level for new sessions.
///
/// This determines the default permission level that new chat sessions inherit.
/// Users can override this per-session in the session settings panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GlobalPermissionLevel {
    /// Read-only access - no file writes, no shell commands
    Sandbox,
    /// Ask before write operations (default)
    #[default]
    Restricted,
    /// Full access - no confirmation required
    Full,
}

/// Global permission settings for tool execution.
///
/// These settings define the default permission behavior for new sessions.
/// Individual sessions can override these settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GlobalPermissionSettings {
    /// Default permission level for new sessions
    pub default_level: GlobalPermissionLevel,
    /// Default enabled tools for new sessions (tool name -> enabled)
    pub default_enabled_tools: HashMap<String, bool>,
}

impl Default for GlobalPermissionSettings {
    fn default() -> Self {
        let mut default_enabled_tools = HashMap::new();
        // Default enabled tools - names must match gestura_core::tools::registry
        default_enabled_tools.insert("file".to_string(), true);
        default_enabled_tools.insert("shell".to_string(), true);
        default_enabled_tools.insert("git".to_string(), true);
        default_enabled_tools.insert("code".to_string(), true);
        default_enabled_tools.insert("web".to_string(), true);
        default_enabled_tools.insert("web_search".to_string(), true);
        default_enabled_tools.insert("task".to_string(), true); // Task management for UI task panel
        // Advanced tools disabled by default
        default_enabled_tools.insert("a2a".to_string(), false);
        default_enabled_tools.insert("permissions".to_string(), false);
        default_enabled_tools.insert("mcp".to_string(), false);

        Self {
            default_level: GlobalPermissionLevel::default(),
            default_enabled_tools,
        }
    }
}

/// Pipeline and context management settings.
///
/// These settings control how the agent pipeline manages conversation context,
/// token limits, and auto-compaction behavior. They are persisted in the
/// application configuration and merged with provider-specific defaults at runtime.
///
/// # Examples
///
/// ```
/// use gestura_core::config::PipelineSettings;
/// use gestura_core::pipeline::CompactionStrategy;
///
/// // Create custom settings
/// let mut settings = PipelineSettings::default();
/// settings.max_history_messages = 20;
/// settings.auto_compact_threshold_percent = 75;
/// settings.compaction_strategy = CompactionStrategy::MemoryBank;
///
/// // Convert threshold to float for comparison
/// assert_eq!(settings.auto_compact_threshold(), 0.75);
/// ```
///
/// # Configuration via CLI
///
/// ```bash
/// # Set maximum history messages
/// gestura config set pipeline.max_history_messages 20
///
/// # Set auto-compaction threshold (0-100%)
/// gestura config set pipeline.auto_compact_threshold_percent 75
///
/// # Set compaction strategy
/// gestura config set pipeline.compaction_strategy MemoryBank
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PipelineSettings {
    /// Maximum number of history messages to include in prompt.
    ///
    /// Older messages beyond this limit are dropped to save tokens.
    /// Default: 10 messages
    ///
    /// Range: 1-100 (enforced by CLI validation)
    pub max_history_messages: usize,

    /// Auto-compaction threshold as percentage (0-100).
    ///
    /// When estimated tokens exceed this percentage of the context limit,
    /// automatically trigger context compaction using the configured strategy.
    ///
    /// Stored as integer percentage (0-100) for `Eq` trait compatibility.
    /// Use `auto_compact_threshold()` method to get as float (0.0-1.0).
    ///
    /// Default: 80 (triggers at 80% of token limit)
    ///
    /// Range: 0-100
    pub auto_compact_threshold_percent: u8,

    /// Strategy to use when auto-compaction is triggered.
    ///
    /// Available strategies:
    /// - `Summarize`: Condense older messages into a summary (default)
    /// - `Truncate`: Remove oldest messages
    /// - `Clear`: Drop all history and start fresh
    /// - `Prompt`: Ask user what to do
    /// - `MemoryBank`: Save context to persistent markdown files
    ///
    /// Default: `Summarize`
    pub compaction_strategy: crate::pipeline::CompactionStrategy,

    /// Maximum context window tokens (model-dependent).
    ///
    /// Set to 0 to use provider-specific defaults:
    /// - OpenAI GPT-4: 128,000 tokens
    /// - Anthropic Claude: 200,000 tokens
    /// - Grok: 131,072 tokens
    /// - Ollama: 32,768 tokens (conservative default)
    ///
    /// Default: 0 (use provider defaults)
    pub max_context_tokens: usize,

    /// Enable token usage logging for debugging.
    ///
    /// When enabled, logs token usage estimates and visual indicators
    /// (green/yellow/red) to help monitor context window utilization.
    ///
    /// Default: true
    pub log_token_usage: bool,
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            max_history_messages: 10,
            auto_compact_threshold_percent: 80, // 80% = 0.8
            compaction_strategy: crate::pipeline::CompactionStrategy::default(),
            max_context_tokens: 0, // 0 = use provider defaults
            log_token_usage: true,
        }
    }
}

impl PipelineSettings {
    /// Get auto-compaction threshold as a float (0.0-1.0).
    ///
    /// Converts the integer percentage (0-100) to a float for use in
    /// threshold comparisons.
    ///
    /// # Examples
    ///
    /// ```
    /// use gestura_core::config::PipelineSettings;
    ///
    /// let settings = PipelineSettings::default();
    /// assert_eq!(settings.auto_compact_threshold(), 0.80);
    ///
    /// let mut custom = PipelineSettings::default();
    /// custom.auto_compact_threshold_percent = 75;
    /// assert_eq!(custom.auto_compact_threshold(), 0.75);
    /// ```
    pub fn auto_compact_threshold(&self) -> f64 {
        (self.auto_compact_threshold_percent as f64) / 100.0
    }
}

/// Prompt enhancement settings for LLM-powered prompt improvement
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PromptEnhancementSettings {
    /// Enable auto-enhancement while typing (debounced)
    pub auto_enhance: bool,
    /// Enhancement style: "concise" | "detailed" | "technical"
    pub style: String,
    /// Maximum length multiplier for enhanced prompts (1.0 - 5.0)
    /// Stored as integer (10-50) for Eq compatibility, divide by 10 to get actual value
    pub max_length_multiplier_x10: u8,
}

impl Default for PromptEnhancementSettings {
    fn default() -> Self {
        Self {
            auto_enhance: false, // Opt-in feature
            style: "concise".to_string(),
            max_length_multiplier_x10: 30, // 3.0x default
        }
    }
}

impl PromptEnhancementSettings {
    /// Get max_length_multiplier as f64
    pub fn max_length_multiplier(&self) -> f64 {
        (self.max_length_multiplier_x10 as f64) / 10.0
    }

    /// Set max_length_multiplier from f64 (clamps to 1.0-5.0 range)
    pub fn set_max_length_multiplier(&mut self, value: f64) {
        let clamped = value.clamp(1.0, 5.0);
        self.max_length_multiplier_x10 = (clamped * 10.0).round() as u8;
    }
}

/// Application configuration persisted to a YAML file.
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
    /// Global permission settings for tool execution
    #[serde(default)]
    pub permissions: GlobalPermissionSettings,
    /// Pipeline and context management settings
    #[serde(default)]
    pub pipeline: PipelineSettings,
    /// Prompt enhancement settings
    #[serde(default)]
    pub prompt_enhancement: PromptEnhancementSettings,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OpenAiConfig {
    #[serde(default)]
    pub api_key: String,
    pub base_url: Option<String>,
    #[serde(default = "default_openai_model")]
    pub model: String,
}

fn default_openai_model() -> String {
    "gpt-4o".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AnthropicConfig {
    #[serde(default)]
    pub api_key: String,
    pub base_url: Option<String>,
    #[serde(default = "default_anthropic_model")]
    pub model: String,

    /// Optional: enable Anthropic "extended thinking" streaming.
    ///
    /// When set, we send `thinking: { type: "enabled", budget_tokens: N }` in the request body.
    /// Only certain Claude models support this; if your model does not, Anthropic will reject the request.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
}

fn default_anthropic_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GrokConfig {
    #[serde(default)]
    pub api_key: String,
    pub base_url: Option<String>,
    #[serde(default = "default_grok_model")]
    pub model: String,
}

fn default_grok_model() -> String {
    "grok-3".to_string()
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
            permissions: GlobalPermissionSettings::default(),
            pipeline: PipelineSettings::default(),
            prompt_enhancement: PromptEnhancementSettings::default(),
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

    /// Returns the default config path: `~/.gestura/config.yaml`
    pub fn default_path() -> PathBuf {
        Self::data_dir().join("config.yaml")
    }

    /// Returns the legacy config path used by older versions: `~/.gestura/config.json`
    fn legacy_json_path() -> PathBuf {
        Self::data_dir().join("config.json")
    }

    /// Returns the legacy config backup path: `~/.gestura/config.json.backup`
    fn legacy_json_backup_path() -> PathBuf {
        Self::data_dir().join("config.json.backup")
    }

    /// Check if a configuration file exists
    pub fn exists() -> bool {
        Self::default_path().exists() || Self::legacy_json_path().exists()
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

    /// Load configuration from disk at an explicit path.
    ///
    /// This is primarily useful for tests and tooling that want to avoid
    /// mutating the user's real config file.
    ///
    /// If the file does not exist or cannot be read/parsed, this returns
    /// [`AppConfig::default`].
    pub fn load_from_path(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(s) => {
                let is_json = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"));

                if is_json {
                    serde_json::from_str(&s).unwrap_or_default()
                } else {
                    serde_yaml::from_str(&s).unwrap_or_default()
                }
            }
            Err(_) => Self::default(),
        }
    }

    /// Load configuration from disk, falling back to defaults if missing (sync version).
    ///
    /// If `~/.gestura/config.yaml` is missing but `~/.gestura/config.json` exists,
    /// this will automatically migrate the JSON file to YAML.
    pub fn load() -> Self {
        let yaml_path = Self::default_path();
        if yaml_path.exists() {
            return Self::load_from_path(&yaml_path);
        }

        let json_path = Self::legacy_json_path();
        if json_path.exists()
            && let Ok(s) = fs::read_to_string(&json_path)
            && let Ok(cfg) = serde_json::from_str::<Self>(&s)
        {
            // Best-effort migration: write YAML and optionally back up the JSON.
            let _ = cfg.save_to_path(&yaml_path);
            if !Self::legacy_json_backup_path().exists() {
                let _ = fs::rename(&json_path, Self::legacy_json_backup_path());
            }
            return cfg;
        }

        Self::default()
    }

    /// Load configuration from disk at an explicit path (async).
    ///
    /// This is the async equivalent of [`AppConfig::load_from_path`]. If the file
    /// does not exist or cannot be read/parsed, this returns [`AppConfig::default`].
    pub async fn load_from_path_async(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        match tokio::fs::read_to_string(&path).await {
            Ok(s) => {
                let is_json = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"));

                if is_json {
                    serde_json::from_str(&s).unwrap_or_default()
                } else {
                    serde_yaml::from_str(&s).unwrap_or_default()
                }
            }
            Err(_) => Self::default(),
        }
    }

    /// Load configuration from disk asynchronously, falling back to defaults if missing.
    ///
    /// This is the preferred method for GUI/Tauri commands to avoid blocking the UI thread.
    pub async fn load_async() -> Self {
        let yaml_path = Self::default_path();
        if tokio::fs::try_exists(&yaml_path).await.unwrap_or(false) {
            return Self::load_from_path_async(&yaml_path).await;
        }

        let json_path = Self::legacy_json_path();
        if tokio::fs::try_exists(&json_path).await.unwrap_or(false)
            && let Ok(s) = tokio::fs::read_to_string(&json_path).await
            && let Ok(cfg) = serde_json::from_str::<Self>(&s)
        {
            // Best-effort migration: write YAML and optionally back up the JSON.
            let _ = cfg.save_to_path_async(&yaml_path).await;
            let backup_path = Self::legacy_json_backup_path();
            if !tokio::fs::try_exists(&backup_path).await.unwrap_or(false) {
                let _ = tokio::fs::rename(&json_path, backup_path).await;
            }
            return cfg;
        }

        Self::default()
    }

    /// Save configuration to disk at an explicit path.
    ///
    /// This is primarily useful for tests and tooling that want deterministic,
    /// isolated config files.
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_yaml::to_string(self)
            .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;
        fs::write(path, data)?;
        Ok(())
    }

    /// Save configuration to disk (sync version).
    pub fn save(&self) -> Result<()> {
        self.save_to_path(Self::default_path())
    }

    /// Save configuration to disk at an explicit path (async).
    ///
    /// This is the async equivalent of [`AppConfig::save_to_path`].
    pub async fn save_to_path_async(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let data = serde_yaml::to_string(self)
            .map_err(|e| AppError::Config(format!("Failed to serialize config: {}", e)))?;
        tokio::fs::write(path, data).await?;
        Ok(())
    }

    /// Save configuration to disk asynchronously.
    ///
    /// This is the preferred method for GUI/Tauri commands to avoid blocking the UI thread.
    pub async fn save_async(&self) -> Result<()> {
        self.save_to_path_async(Self::default_path()).await
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

    /// Apply environment variable overrides to the configuration
    ///
    /// This method applies GESTURA_* environment variables on top of the
    /// current configuration. Environment variables take precedence over
    /// config file values.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Set environment variable
    /// std::env::set_var("GESTURA_LLM_PRIMARY", "openai");
    ///
    /// // Load config with env overrides
    /// let config = AppConfig::load().apply_env_overrides();
    /// assert_eq!(config.llm.primary, "openai");
    /// ```
    pub fn apply_env_overrides(mut self) -> Self {
        // Core settings
        if let Some(v) = get_env("HOTKEY_LISTEN") {
            self.hotkey_listen = v;
        }
        if let Some(v) = get_env_u32("GRACE_PERIOD_SECS") {
            self.grace_period_secs = v;
        }
        if let Some(v) = get_env("NATS_URL") {
            self.nats_url = v;
        }

        // LLM settings
        if let Some(v) = get_env("LLM_PRIMARY") {
            self.llm.primary = v;
        }
        if let Some(v) = get_env("LLM_FALLBACK") {
            self.llm.fallback = Some(v);
        }

        // OpenAI
        if get_env("OPENAI_API_KEY").is_some()
            || get_env("OPENAI_MODEL").is_some()
            || get_env("OPENAI_BASE_URL").is_some()
        {
            let openai = self.llm.openai.get_or_insert_with(Default::default);
            if let Some(v) = get_env("OPENAI_API_KEY") {
                openai.api_key = v;
            }
            if let Some(v) = get_env("OPENAI_MODEL") {
                openai.model = v;
            }
            if let Some(v) = get_env("OPENAI_BASE_URL") {
                openai.base_url = Some(v);
            }
        }

        // Anthropic
        if get_env("ANTHROPIC_API_KEY").is_some()
            || get_env("ANTHROPIC_MODEL").is_some()
            || get_env("ANTHROPIC_BASE_URL").is_some()
        {
            let anthropic = self.llm.anthropic.get_or_insert_with(Default::default);
            if let Some(v) = get_env("ANTHROPIC_API_KEY") {
                anthropic.api_key = v;
            }
            if let Some(v) = get_env("ANTHROPIC_MODEL") {
                anthropic.model = v;
            }
            if let Some(v) = get_env("ANTHROPIC_BASE_URL") {
                anthropic.base_url = Some(v);
            }
        }

        // Grok
        if get_env("GROK_API_KEY").is_some()
            || get_env("GROK_MODEL").is_some()
            || get_env("GROK_BASE_URL").is_some()
        {
            let grok = self.llm.grok.get_or_insert_with(Default::default);
            if let Some(v) = get_env("GROK_API_KEY") {
                grok.api_key = v;
            }
            if let Some(v) = get_env("GROK_MODEL") {
                grok.model = v;
            }
            if let Some(v) = get_env("GROK_BASE_URL") {
                grok.base_url = Some(v);
            }
        }

        // Ollama
        if get_env("OLLAMA_BASE_URL").is_some() || get_env("OLLAMA_MODEL").is_some() {
            let ollama = self.llm.ollama.get_or_insert_with(|| OllamaConfig {
                base_url: "http://localhost:11434".to_string(),
                model: "llama3.2".to_string(),
            });
            if let Some(v) = get_env("OLLAMA_BASE_URL") {
                ollama.base_url = v;
            }
            if let Some(v) = get_env("OLLAMA_MODEL") {
                ollama.model = v;
            }
        }

        // Voice settings
        if let Some(v) = get_env("VOICE_PROVIDER") {
            self.voice.provider = v;
        }
        if let Some(v) = get_env("VOICE_LOCAL_MODEL_PATH") {
            self.voice.local_model_path = Some(v);
        }
        if let Some(v) = get_env("VOICE_OPENAI_API_KEY") {
            self.voice.openai_api_key = Some(v);
        }
        if let Some(v) = get_env("VOICE_OPENAI_MODEL") {
            self.voice.openai_model = Some(v);
        }
        if let Some(v) = get_env("VOICE_AUDIO_DEVICE") {
            self.voice.audio_device = Some(v);
        }

        // UI settings
        if let Some(v) = get_env("UI_THEME_MODE") {
            self.ui.theme_mode = v;
        }
        if let Some(v) = get_env("UI_ACCENT") {
            self.ui.accent = Some(v);
        }

        // Developer settings
        if let Some(v) = get_env_bool("DEVELOPER_MODE") {
            self.developer.developer_mode = v;
        }
        if let Some(v) = get_env_bool("ENABLE_SIMULATORS") {
            self.developer.enable_simulators = v;
        }
        if let Some(v) = get_env_bool("VERBOSE_BLE_LOGGING") {
            self.developer.verbose_ble_logging = v;
        }

        // Web search settings
        if let Some(v) = get_env("SERPAPI_KEY") {
            self.web_search.serpapi_key = Some(v);
        }
        if let Some(v) = get_env("BRAVE_SEARCH_KEY") {
            self.web_search.brave_key = Some(v);
        }

        self
    }

    /// Load configuration with environment variable overrides applied
    ///
    /// This is the recommended way to load configuration as it respects
    /// the full precedence hierarchy: env vars > config file > defaults
    pub fn load_with_env() -> Self {
        Self::load().apply_env_overrides()
    }

    /// Load configuration asynchronously with environment variable overrides
    pub async fn load_with_env_async() -> Self {
        Self::load_async().await.apply_env_overrides()
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
    use crate::pipeline::CompactionStrategy;
    use std::sync::{Mutex, OnceLock};

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

    #[test]
    fn test_backward_compatibility_without_pipeline_settings() {
        // Create a default config and serialize it
        let default_config = AppConfig::default();
        let mut json_value: serde_json::Value = serde_json::to_value(&default_config).unwrap();

        // Remove the pipeline field to simulate an old config file
        json_value.as_object_mut().unwrap().remove("pipeline");

        // Deserialize should succeed and use default pipeline settings
        let config: AppConfig = serde_json::from_value(json_value).unwrap();

        // Verify pipeline settings have default values
        assert_eq!(config.pipeline.max_history_messages, 10);
        assert_eq!(config.pipeline.auto_compact_threshold_percent, 80);
        assert_eq!(
            config.pipeline.compaction_strategy,
            CompactionStrategy::Summarize
        );
        assert_eq!(config.pipeline.max_context_tokens, 0);
        assert!(config.pipeline.log_token_usage);
    }

    #[test]
    fn test_backward_compatibility_with_partial_pipeline_settings() {
        // Create a default config and serialize it
        let default_config = AppConfig::default();
        let mut json_value: serde_json::Value = serde_json::to_value(&default_config).unwrap();

        // Modify pipeline to only have max_history_messages
        let pipeline_obj = serde_json::json!({
            "max_history_messages": 20
        });
        json_value
            .as_object_mut()
            .unwrap()
            .insert("pipeline".to_string(), pipeline_obj);

        // Deserialize should succeed and use defaults for missing fields
        let config: AppConfig = serde_json::from_value(json_value).unwrap();

        // Verify explicitly set value
        assert_eq!(config.pipeline.max_history_messages, 20);

        // Verify other fields have default values
        assert_eq!(config.pipeline.auto_compact_threshold_percent, 80);
        assert_eq!(
            config.pipeline.compaction_strategy,
            CompactionStrategy::Summarize
        );
        assert_eq!(config.pipeline.max_context_tokens, 0);
        assert!(config.pipeline.log_token_usage);
    }

    #[test]
    fn test_pipeline_settings_serialization_roundtrip() {
        // Create a config with custom pipeline settings
        let mut config = AppConfig::default();
        config.pipeline.max_history_messages = 15;
        config.pipeline.auto_compact_threshold_percent = 75;
        config.pipeline.compaction_strategy = CompactionStrategy::MemoryBank;
        config.pipeline.max_context_tokens = 50000;
        config.pipeline.log_token_usage = false;

        // Serialize to YAML
        let yaml = serde_yaml::to_string(&config).unwrap();

        // Deserialize back
        let deserialized: AppConfig = serde_yaml::from_str(&yaml).unwrap();

        // Verify all pipeline settings are preserved
        assert_eq!(deserialized.pipeline.max_history_messages, 15);
        assert_eq!(deserialized.pipeline.auto_compact_threshold_percent, 75);
        assert_eq!(
            deserialized.pipeline.compaction_strategy,
            CompactionStrategy::MemoryBank
        );
        assert_eq!(deserialized.pipeline.max_context_tokens, 50000);
        assert!(!deserialized.pipeline.log_token_usage);
    }

    /// Global lock used to serialize environment-variable mutation across tests.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// RAII helper for setting a process-wide environment variable for the duration of a scope.
    ///
    /// ## Safety / Concurrency
    /// Environment variables are process-global state. Tests that use this helper should
    /// serialize access (e.g. by holding `env_lock()`) to avoid concurrent mutation.
    struct ScopedEnvVar {
        key: &'static str,
        old: Option<String>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: String) -> Self {
            let old = std::env::var(key).ok();
            // Rust 2024: mutating process-wide environment variables is `unsafe`.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            match &self.old {
                Some(v) => unsafe {
                    std::env::set_var(self.key, v);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    #[test]
    fn migrates_legacy_json_config_to_yaml_on_load() {
        // This test mutates process-wide env vars; serialize it.
        let _guard = env_lock().lock().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let _home = ScopedEnvVar::set("HOME", home.to_string_lossy().to_string());
        let _userprofile = ScopedEnvVar::set("USERPROFILE", home.to_string_lossy().to_string());
        let _homedrive = ScopedEnvVar::set("HOMEDRIVE", "C:".to_string());
        let _homepath = ScopedEnvVar::set("HOMEPATH", "\\".to_string());

        let gestura_dir = home.join(".gestura");
        fs::create_dir_all(&gestura_dir).unwrap();

        let json_path = gestura_dir.join("config.json");
        let yaml_path = gestura_dir.join("config.yaml");
        let backup_path = gestura_dir.join("config.json.backup");

        // Write legacy JSON config
        let cfg = AppConfig::default();
        let json = serde_json::to_string_pretty(&cfg).unwrap();
        fs::write(&json_path, json).unwrap();
        assert!(!yaml_path.exists());

        // Loading should migrate and return the legacy config contents.
        let loaded = AppConfig::load();
        assert_eq!(loaded, cfg);

        // YAML should exist after migration.
        assert!(yaml_path.exists());

        // Legacy JSON should be backed up (best-effort).
        assert!(!json_path.exists() || backup_path.exists());
    }
}
