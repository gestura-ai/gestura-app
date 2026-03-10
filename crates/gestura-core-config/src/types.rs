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
    DEFAULT_OLLAMA_MODEL, DEFAULT_OPENAI_MODEL,
};
use gestura_core_pipeline::types::CompactionStrategy;

// Re-export domain config types for backwards compatibility.
pub use gestura_core_mcp::config::{
    McpJsonFile, McpScope, McpServerEntry, McpTool, McpTransportType,
    import_claude_desktop_servers, infer_transport_from_endpoint,
};
pub use gestura_core_tools::config::{WebSearchConfig, WebSearchProvider};

// ---------------------------------------------------------------------------
// Serde helper predicates — used by `skip_serializing_if`
// ---------------------------------------------------------------------------

/// Returns `true` when `val` equals its `Default` value.
///
/// Used with `#[serde(skip_serializing_if = "is_default")]` so that struct
/// fields whose value is the default are omitted from the serialized YAML,
/// keeping config files minimal and human-readable.
fn is_default<T: Default + PartialEq>(val: &T) -> bool {
    val == &T::default()
}

fn is_default_hotkey_listen(v: &String) -> bool {
    v == "Ctrl+Space"
}

fn is_default_grace_period_secs(v: &u32) -> bool {
    *v == 30
}

fn is_default_nats_url(v: &String) -> bool {
    v == "nats://127.0.0.1:4223"
}

fn is_default_ui_theme_mode(v: &String) -> bool {
    v == "system"
}

// Default-value provider fns required by `#[serde(default = "...")]`
fn default_hotkey_listen() -> String {
    "Ctrl+Space".to_string()
}

fn default_grace_period_secs() -> u32 {
    30
}

fn default_nats_url() -> String {
    "nats://127.0.0.1:4223".to_string()
}

fn default_ui_theme_mode() -> String {
    "system".to_string()
}

// ---------------------------------------------------------------------------
// Permission types
// ---------------------------------------------------------------------------

/// Global permission level for new sessions.
///
/// This determines the default permission level that new agent sessions inherit.
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
    /// ERL-inspired experiential reflection settings.
    #[serde(default)]
    pub reflection: ReflectionSettings,
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
            reflection: ReflectionSettings::default(),
        }
    }
}

impl PipelineSettings {
    /// Get auto-compaction threshold as a float (0.0-1.0).
    pub fn auto_compact_threshold(&self) -> f64 {
        (self.auto_compact_threshold_percent as f64) / 100.0
    }
}

/// Settings for ERL-inspired experiential reflection.
///
/// When enabled, the agent generates structured reflections on suboptimal
/// turns and stores them for retrieval in future context injection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ReflectionSettings {
    /// Enable the experiential reflection phase.
    pub enabled: bool,
    /// Quality threshold percentage (0–100). Reflection triggers when the
    /// response quality score falls below `threshold / 100`.
    pub quality_threshold_percent: u8,
    /// Maximum number of past reflections to inject into prompt context.
    pub max_injected: usize,
    /// Maximum number of text-only reflection-guided revision attempts.
    pub max_retry_attempts: usize,
    /// Minimum confidence percentage (0–100) for promoting a reflection
    /// to long-term memory.
    pub promotion_confidence_percent: u8,
}

impl Default for ReflectionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            quality_threshold_percent: 60,
            max_injected: 3,
            max_retry_attempts: 1,
            promotion_confidence_percent: 75,
        }
    }
}

impl ReflectionSettings {
    /// Get quality threshold as a float (0.0–1.0).
    pub fn quality_threshold(&self) -> f32 {
        self.quality_threshold_percent as f32 / 100.0
    }

    /// Get promotion confidence as a float (0.0–1.0).
    pub fn promotion_confidence(&self) -> f32 {
        self.promotion_confidence_percent as f32 / 100.0
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
///
/// Only fields that differ from their defaults are written to disk.  All
/// other fields fall back to `Default::default()` at load time, keeping the
/// YAML file minimal and human-readable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    /// Global hotkey to toggle the app or trigger recording.
    #[serde(
        default = "default_hotkey_listen",
        skip_serializing_if = "is_default_hotkey_listen"
    )]
    pub hotkey_listen: String,
    /// Grace period in seconds for agent shutdown.
    #[serde(
        default = "default_grace_period_secs",
        skip_serializing_if = "is_default_grace_period_secs"
    )]
    pub grace_period_secs: u32,
    /// LLM configuration and provider selection.
    #[serde(default)]
    pub llm: LlmSettings,
    /// Voice/STT configuration.
    #[serde(default)]
    pub voice: VoiceSettings,
    /// MCP server configuration (full spec, Claude Code compatible).
    #[serde(default, alias = "mcp_tools", skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServerEntry>,
    /// MDH pointer mappings
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub mdh_pointers: HashMap<String, String>,
    /// UI preferences (theme, accent)
    #[serde(default, skip_serializing_if = "is_default")]
    pub ui: UiSettings,
    /// NATS URL for embedded MQ connectivity.
    #[serde(
        default = "default_nats_url",
        skip_serializing_if = "is_default_nats_url"
    )]
    pub nats_url: String,
    /// Developer and simulator settings
    #[serde(default, skip_serializing_if = "is_default")]
    pub developer: DeveloperSettings,
    /// Notification settings for response completion and feedback
    #[serde(default, skip_serializing_if = "is_default")]
    pub notifications: NotificationSettings,
    /// Web search configuration
    #[serde(default, skip_serializing_if = "is_default")]
    pub web_search: WebSearchConfig,
    /// Global permission settings for tool execution
    #[serde(default, skip_serializing_if = "is_default")]
    pub permissions: GlobalPermissionSettings,
    /// Pipeline and context management settings
    #[serde(default, skip_serializing_if = "is_default")]
    pub pipeline: PipelineSettings,
    /// Prompt enhancement settings
    #[serde(default, skip_serializing_if = "is_default")]
    pub prompt_enhancement: PromptEnhancementSettings,
    /// Hooks configuration.
    #[serde(default, skip_serializing_if = "is_default")]
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
    #[serde(
        default = "default_ui_theme_mode",
        skip_serializing_if = "is_default_ui_theme_mode"
    )]
    pub theme_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme_mode: default_ui_theme_mode(),
            accent: None,
        }
    }
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
#[serde(default)]
pub struct LlmSettings {
    /// Primary provider id: "openai" | "anthropic" | "gemini" | "grok" | "ollama"
    pub primary: String,
    /// Fallback provider id (optional): used when primary fails
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai: Option<OpenAiConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<AnthropicConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini: Option<GeminiConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grok: Option<GrokConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama: Option<OllamaConfig>,
}

impl Default for LlmSettings {
    fn default() -> Self {
        Self {
            primary: "anthropic".to_string(),
            fallback: None,
            openai: None,
            anthropic: None,
            gemini: None,
            grok: None,
            ollama: None,
        }
    }
}

impl LlmSettings {
    /// Ensure the provider-specific config object for `provider` exists and has a
    /// non-empty model field. If the config object is missing it is created with
    /// `Default::default()` (which now includes the correct default model). If the
    /// config already exists but has an empty model, the model is back-filled.
    pub fn ensure_provider_config(&mut self, provider: &str) {
        match provider {
            "openai" => {
                let c = self.openai.get_or_insert_with(OpenAiConfig::default);
                if c.model.is_empty() {
                    c.model = DEFAULT_OPENAI_MODEL.to_string();
                }
            }
            "anthropic" => {
                let c = self.anthropic.get_or_insert_with(AnthropicConfig::default);
                if c.model.is_empty() {
                    c.model = DEFAULT_ANTHROPIC_MODEL.to_string();
                }
            }
            "grok" => {
                let c = self.grok.get_or_insert_with(GrokConfig::default);
                if c.model.is_empty() {
                    c.model = DEFAULT_GROK_MODEL.to_string();
                }
            }
            "gemini" => {
                let c = self.gemini.get_or_insert_with(GeminiConfig::default);
                if c.model.is_empty() {
                    c.model = DEFAULT_GEMINI_MODEL.to_string();
                }
            }
            "ollama" => {
                let c = self.ollama.get_or_insert_with(|| OllamaConfig {
                    base_url: DEFAULT_OLLAMA_BASE_URL.to_string(),
                    model: DEFAULT_OLLAMA_MODEL.to_string(),
                });
                if c.model.is_empty() {
                    c.model = DEFAULT_OLLAMA_MODEL.to_string();
                }
            }
            _ => {} // unknown provider – nothing to do
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenAiConfig {
    /// Stored in the system keychain; never written to the config file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(
        default = "default_openai_model",
        skip_serializing_if = "is_default_openai_model"
    )]
    pub model: String,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: None,
            model: DEFAULT_OPENAI_MODEL.to_string(),
        }
    }
}

fn default_openai_model() -> String {
    DEFAULT_OPENAI_MODEL.to_string()
}

fn is_default_openai_model(v: &String) -> bool {
    v == DEFAULT_OPENAI_MODEL
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnthropicConfig {
    /// Stored in the system keychain; never written to the config file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(
        default = "default_anthropic_model",
        skip_serializing_if = "is_default_anthropic_model"
    )]
    pub model: String,
    /// Optional: enable Anthropic "extended thinking" streaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget_tokens: Option<u32>,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: None,
            model: DEFAULT_ANTHROPIC_MODEL.to_string(),
            thinking_budget_tokens: None,
        }
    }
}

fn default_anthropic_model() -> String {
    DEFAULT_ANTHROPIC_MODEL.to_string()
}

fn is_default_anthropic_model(v: &String) -> bool {
    v == DEFAULT_ANTHROPIC_MODEL
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrokConfig {
    /// Stored in the system keychain; never written to the config file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(
        default = "default_grok_model",
        skip_serializing_if = "is_default_grok_model"
    )]
    pub model: String,
}

impl Default for GrokConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: None,
            model: DEFAULT_GROK_MODEL.to_string(),
        }
    }
}

fn default_grok_model() -> String {
    DEFAULT_GROK_MODEL.to_string()
}

fn is_default_grok_model(v: &String) -> bool {
    v == DEFAULT_GROK_MODEL
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeminiConfig {
    /// Stored in the system keychain; never written to the config file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(
        default = "default_gemini_model",
        skip_serializing_if = "is_default_gemini_model"
    )]
    pub model: String,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: None,
            model: DEFAULT_GEMINI_MODEL.to_string(),
        }
    }
}

fn default_gemini_model() -> String {
    DEFAULT_GEMINI_MODEL.to_string()
}

fn is_default_gemini_model(v: &String) -> bool {
    v == DEFAULT_GEMINI_MODEL
}

fn default_ollama_base_url() -> String {
    DEFAULT_OLLAMA_BASE_URL.to_string()
}

fn is_default_ollama_base_url(v: &String) -> bool {
    v == DEFAULT_OLLAMA_BASE_URL
}

fn default_ollama_model() -> String {
    DEFAULT_OLLAMA_MODEL.to_string()
}

fn is_default_ollama_model(v: &String) -> bool {
    v == DEFAULT_OLLAMA_MODEL
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OllamaConfig {
    #[serde(
        default = "default_ollama_base_url",
        skip_serializing_if = "is_default_ollama_base_url"
    )]
    pub base_url: String,
    #[serde(
        default = "default_ollama_model",
        skip_serializing_if = "is_default_ollama_model"
    )]
    pub model: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: default_ollama_base_url(),
            model: default_ollama_model(),
        }
    }
}

/// Voice settings; default uses local Whisper
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceSettings {
    /// Preferred provider: "local" | "openai" | "none"
    pub provider: String,
    /// Optional input wav file path used for testing transcription
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_path: Option<String>,
    /// Local whisper.cpp model path (.bin)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_model_path: Option<String>,
    /// OpenAI Whisper API settings (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_model: Option<String>,
    /// Selected audio input device name (None = use system default)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_device: Option<String>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            provider: "local".to_string(),
            input_path: None,
            local_model_path: None,
            openai_api_key: None,
            openai_base_url: None,
            openai_model: None,
            audio_device: None,
        }
    }
}

// ---------------------------------------------------------------------------
// AppConfig Default
// ---------------------------------------------------------------------------

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey_listen: default_hotkey_listen(),
            grace_period_secs: default_grace_period_secs(),
            llm: LlmSettings::default(),
            voice: VoiceSettings::default(),
            mcp_servers: vec![],
            mdh_pointers: Default::default(),
            ui: UiSettings::default(),
            nats_url: default_nats_url(),
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
            "pipeline.reflection.enabled",
            "pipeline.reflection.quality_threshold_percent",
            "pipeline.reflection.max_injected",
            "pipeline.reflection.max_retry_attempts",
            "pipeline.reflection.promotion_confidence_percent",
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

    // -----------------------------------------------------------------------
    // MCP server helpers
    // -----------------------------------------------------------------------

    /// Find an MCP server entry by name (immutable).
    pub fn find_mcp_server(&self, name: &str) -> Option<&McpServerEntry> {
        self.mcp_servers.iter().find(|s| s.name == name)
    }

    /// Find an MCP server entry by name (mutable).
    pub fn find_mcp_server_mut(&mut self, name: &str) -> Option<&mut McpServerEntry> {
        self.mcp_servers.iter_mut().find(|s| s.name == name)
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
