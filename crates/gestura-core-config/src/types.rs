//! Configuration types and pure `AppConfig` methods.
//!
//! All struct/enum definitions live here. Security-dependent methods
//! (`load`, `save`, keychain operations) remain in `gestura-core` as
//! bridge code.

use std::{collections::HashMap, fs, path::Path, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::config_env::{get_env, get_env_bool, get_env_u32};
use crate::hooks_types::HooksSettings;
// Re-export foundation error types for consumers that access them via config.
#[allow(unused_imports)]
pub use gestura_core_foundation::error::{AppError, Result};
use gestura_core_llm::default_models::{
    DEFAULT_ANTHROPIC_MODEL, DEFAULT_GEMINI_MODEL, DEFAULT_GROK_MODEL, DEFAULT_OLLAMA_BASE_URL,
    DEFAULT_OLLAMA_MODEL, DEFAULT_OPENAI_MODEL, DEFAULT_OPENAI_STT_MODEL,
};
use gestura_core_pipeline::types::CompactionStrategy;

// Re-export domain config types for backwards compatibility.
pub use gestura_core_mcp::config::{
    McpJsonFile, McpScope, McpServerEntry, McpTool, McpTransportType, import_claude_desktop_servers,
};
pub use gestura_core_tools::config::{WebSearchConfig, WebSearchProvider};

// ---------------------------------------------------------------------------
// Permission types
// ---------------------------------------------------------------------------

/// Global permission level for new sessions.
///
/// This determines the default permission level that new chat sessions inherit.
/// Users can override this per-session in the session settings panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GlobalPermissionLevel {
    /// Read-only access - no file writes, no shell commands
    #[serde(alias = "sandbox")]
    Sandbox,
    /// Ask before write operations (default)
    #[default]
    #[serde(alias = "restricted")]
    Restricted,
    /// Full access - no confirmation required
    #[serde(alias = "full")]
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
        // Screen capture tools disabled by default (privacy-sensitive)
        default_enabled_tools.insert("screenshot".to_string(), false);
        default_enabled_tools.insert("screen_record".to_string(), false);
        default_enabled_tools.insert("screen".to_string(), false);
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

// ---------------------------------------------------------------------------
// Pipeline settings
// ---------------------------------------------------------------------------

/// Pipeline and context management settings.
///
/// These settings control how the agent pipeline manages conversation context,
/// token limits, and auto-compaction behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PipelineSettings {
    /// Maximum number of history messages to include in prompt.
    pub max_history_messages: usize,
    /// Auto-compaction threshold as percentage (0-100).
    pub auto_compact_threshold_percent: u8,
    /// Strategy to use when auto-compaction is triggered.
    pub compaction_strategy: CompactionStrategy,
    /// Maximum context window tokens (model-dependent). 0 = use provider defaults.
    pub max_context_tokens: usize,
    /// Enable token usage logging for debugging.
    pub log_token_usage: bool,
    /// Project guardrails settings.
    #[serde(default)]
    pub project_guardrails: ProjectGuardrailsSettings,
}

/// Settings for project-level guardrails discovery and prompt injection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProjectGuardrailsSettings {
    /// Enable project guardrails discovery and injection.
    pub enabled: bool,
    /// Maximum number of characters to include from the guardrails file.
    pub max_chars: usize,
}

impl Default for ProjectGuardrailsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_chars: 12_000,
        }
    }
}

impl Default for PipelineSettings {
    fn default() -> Self {
        Self {
            max_history_messages: 10,
            auto_compact_threshold_percent: 80,
            compaction_strategy: CompactionStrategy::default(),
            max_context_tokens: 0,
            log_token_usage: true,
            project_guardrails: ProjectGuardrailsSettings::default(),
        }
    }
}

impl PipelineSettings {
    /// Get auto-compaction threshold as a float (0.0-1.0).
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
            auto_enhance: false,
            style: "concise".to_string(),
            max_length_multiplier_x10: 30,
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

// ---------------------------------------------------------------------------
// AppConfig — main configuration struct
// ---------------------------------------------------------------------------

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
    /// MCP server configuration (full spec, Claude Code compatible).
    #[serde(default, alias = "mcp_tools")]
    pub mcp_servers: Vec<McpServerEntry>,
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
    /// Hooks configuration.
    #[serde(default)]
    pub hooks: HooksSettings,
}

// ---------------------------------------------------------------------------
// Supporting sub-types
// ---------------------------------------------------------------------------

/// Notification settings for response completion and MCP feedback
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NotificationSettings {
    pub sound_enabled: bool,
    pub haptic_enabled: bool,
    pub sound_volume: u8,
    pub haptic_intensity: u8,
    pub notification_sound: String,
    pub command_confirm_sound: String,
    pub mcp_feedback_enabled: bool,
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
    pub theme_mode: String,
    pub accent: Option<String>,
}

/// Developer and simulator settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeveloperSettings {
    pub developer_mode: bool,
    pub enable_simulators: bool,
    pub auto_discover_simulators: bool,
    pub verbose_ble_logging: bool,
    pub simulator: SimulatorSettings,
}

/// Simulator-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulatorSettings {
    pub device_name_pattern: String,
    pub auto_connect: bool,
    pub health_check_interval: u32,
    pub enable_metrics: bool,
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

// ---------------------------------------------------------------------------
// LLM provider config types
// ---------------------------------------------------------------------------

/// LLM settings grouping provider-specific configs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmSettings {
    /// Primary provider id: "openai" | "anthropic" | "gemini" | "grok" | "ollama"
    pub primary: String,
    /// Fallback provider id (optional): used when primary fails
    #[serde(default)]
    pub fallback: Option<String>,
    pub openai: Option<OpenAiConfig>,
    pub anthropic: Option<AnthropicConfig>,
    pub gemini: Option<GeminiConfig>,
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
    DEFAULT_OPENAI_MODEL.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AnthropicConfig {
    #[serde(default)]
    pub api_key: String,
    pub base_url: Option<String>,
    #[serde(default = "default_anthropic_model")]
    pub model: String,
    /// Optional: enable Anthropic "extended thinking" streaming.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
}

fn default_anthropic_model() -> String {
    DEFAULT_ANTHROPIC_MODEL.to_string()
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
    DEFAULT_GROK_MODEL.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GeminiConfig {
    #[serde(default)]
    pub api_key: String,
    pub base_url: Option<String>,
    #[serde(default = "default_gemini_model")]
    pub model: String,
}

fn default_gemini_model() -> String {
    DEFAULT_GEMINI_MODEL.to_string()
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

// ---------------------------------------------------------------------------
// AppConfig Default
// ---------------------------------------------------------------------------

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey_listen: "Ctrl+Space".to_string(),
            grace_period_secs: 30,
            llm: LlmSettings {
                primary: "anthropic".into(),
                fallback: Some("ollama".into()),
                openai: None,
                anthropic: None,
                gemini: None,
                grok: None,
                ollama: Some(OllamaConfig {
                    base_url: DEFAULT_OLLAMA_BASE_URL.into(),
                    model: DEFAULT_OLLAMA_MODEL.into(),
                }),
            },
            voice: VoiceSettings {
                provider: "local".into(),
                input_path: None,
                local_model_path: None,
                openai_api_key: None,
                openai_base_url: None,
                openai_model: Some(DEFAULT_OPENAI_STT_MODEL.into()),
                audio_device: None,
            },
            mcp_servers: vec![],
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
            hooks: HooksSettings::default(),
        }
    }
}
// ---------------------------------------------------------------------------
// Pure AppConfig methods (no security deps)
// ---------------------------------------------------------------------------

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
    pub fn legacy_json_path() -> PathBuf {
        Self::data_dir().join("config.json")
    }

    /// Returns the legacy config backup path: `~/.gestura/config.json.backup`
    #[allow(dead_code)]
    pub fn legacy_json_backup_path() -> PathBuf {
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

    /// Load configuration from disk at an explicit path (async).
    ///
    /// If the file does not exist or cannot be read/parsed, this returns
    /// [`AppConfig::default`].
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

    /// Get a config value by dot-notation key (e.g., "llm.primary")
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            // Core settings
            "hotkey_listen" => Some(self.hotkey_listen.clone()),
            "grace_period_secs" => Some(self.grace_period_secs.to_string()),
            "nats_url" => Some(self.nats_url.clone()),

            // LLM settings
            "llm.primary" => Some(self.llm.primary.clone()),
            "llm.fallback" => self.llm.fallback.clone(),
            "llm.openai.model" => self
                .llm
                .openai
                .as_ref()
                .map(|c| c.model.clone())
                .filter(|s| !s.is_empty()),
            "llm.openai.base_url" => self.llm.openai.as_ref().and_then(|c| c.base_url.clone()),
            "llm.anthropic.model" => self
                .llm
                .anthropic
                .as_ref()
                .map(|c| c.model.clone())
                .filter(|s| !s.is_empty()),
            "llm.anthropic.base_url" => {
                self.llm.anthropic.as_ref().and_then(|c| c.base_url.clone())
            }
            "llm.grok.model" => self
                .llm
                .grok
                .as_ref()
                .map(|c| c.model.clone())
                .filter(|s| !s.is_empty()),
            "llm.grok.base_url" => self.llm.grok.as_ref().and_then(|c| c.base_url.clone()),
            "llm.gemini.model" => self
                .llm
                .gemini
                .as_ref()
                .map(|c| c.model.clone())
                .filter(|s| !s.is_empty()),
            "llm.gemini.base_url" => self.llm.gemini.as_ref().and_then(|c| c.base_url.clone()),
            "llm.ollama.model" => self
                .llm
                .ollama
                .as_ref()
                .map(|c| c.model.clone())
                .filter(|s| !s.is_empty()),
            "llm.ollama.base_url" => self.llm.ollama.as_ref().map(|c| c.base_url.clone()),

            // Voice settings
            "voice.provider" => Some(self.voice.provider.clone()),
            "voice.local_model_path" => self
                .voice
                .local_model_path
                .clone()
                .or_else(|| Some("(not set)".to_string())),
            "voice.input_path" => self.voice.input_path.clone(),
            "voice.audio_device" => self.voice.audio_device.clone(),

            // UI settings
            "ui.theme_mode" => Some(self.ui.theme_mode.clone()),
            "ui.accent" => self.ui.accent.clone(),

            // Pipeline settings
            "pipeline.max_history_messages" => Some(self.pipeline.max_history_messages.to_string()),
            "pipeline.auto_compact_threshold_percent" => {
                Some(self.pipeline.auto_compact_threshold_percent.to_string())
            }
            "pipeline.compaction_strategy" => {
                Some(format!("{:?}", self.pipeline.compaction_strategy))
            }
            "pipeline.max_context_tokens" => Some(self.pipeline.max_context_tokens.to_string()),
            "pipeline.log_token_usage" => Some(self.pipeline.log_token_usage.to_string()),

            // Developer settings
            "developer.enable_simulators" => Some(self.developer.enable_simulators.to_string()),
            "developer.verbose_ble_logging" => Some(self.developer.verbose_ble_logging.to_string()),

            _ => None,
        }
    }

    /// List all available config keys
    pub fn list_keys() -> Vec<&'static str> {
        let mut keys = vec![
            "hotkey_listen",
            "grace_period_secs",
            "nats_url",
            "llm.primary",
            "llm.fallback",
            "llm.openai.model",
            "llm.openai.base_url",
            "llm.anthropic.model",
            "llm.anthropic.base_url",
            "llm.grok.model",
            "llm.grok.base_url",
            "llm.gemini.model",
            "llm.gemini.base_url",
            "llm.ollama.model",
            "llm.ollama.base_url",
            "voice.provider",
            "voice.local_model_path",
            "voice.input_path",
            "voice.audio_device",
            "ui.theme_mode",
            "ui.accent",
            "pipeline.max_history_messages",
            "pipeline.auto_compact_threshold_percent",
            "pipeline.compaction_strategy",
            "pipeline.max_context_tokens",
            "pipeline.log_token_usage",
            "developer.enable_simulators",
            "developer.verbose_ble_logging",
        ];
        keys.sort();
        keys
    }

    /// Apply environment variable overrides to the configuration
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

        // Gemini
        if get_env("GEMINI_API_KEY").is_some()
            || get_env("GEMINI_MODEL").is_some()
            || get_env("GEMINI_BASE_URL").is_some()
        {
            let gemini = self.llm.gemini.get_or_insert_with(Default::default);
            if let Some(v) = get_env("GEMINI_API_KEY") {
                gemini.api_key = v;
            }
            if let Some(v) = get_env("GEMINI_MODEL") {
                gemini.model = v;
            }
            if let Some(v) = get_env("GEMINI_BASE_URL") {
                gemini.base_url = Some(v);
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
}

// ---------------------------------------------------------------------------
// WhisperModelInfo
// ---------------------------------------------------------------------------

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
