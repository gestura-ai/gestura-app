//! Gestura Core Library
//!
//! This crate contains the shared business logic for the Gestura voice-first AI assistant.
//! It is used by both the GUI (Tauri) and CLI applications.
//!
//! # Features
//!
//! - `voice-local` - Enable local Whisper speech-to-text (default)
//! - `voice-openai` - Enable OpenAI Whisper API
//! - `nats` - Enable NATS messaging integration
//! - `ble` - Enable Bluetooth LE for haptic devices
//! - `security` - Enable encryption and keychain features
//!
//! # Modules
//!
//! Core functionality is organized into these modules:
//! - `config` - Application configuration management
//! - `error` - Error types and handling
//! - `llm` - LLM provider integrations (OpenAI, Anthropic, Grok, Ollama)
//! - `speech` - Speech-to-text processing
//! - `audio` - Audio capture and processing
//! - `mcp` - Model Context Protocol integration
//! - `session` - Agent session management

/// Crate version from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Crate name
pub const NAME: &str = env!("CARGO_PKG_NAME");

// ============================================================================
// Core Modules (file-backed — business logic + domain-crate integration)
// ============================================================================

pub mod agent_sessions;
pub mod agents {
    pub use gestura_core_agents::*;
}
pub mod checkpoints;
pub mod compaction;
pub mod config;
pub mod context {
    pub use gestura_core_context::*;
}
pub(crate) mod guardrails;
pub mod llm_overrides;
pub mod llm_provider;
pub mod llm_validation;
/// OpenAI(-compatible) API compatibility helpers (e.g., parameter support quirks).
pub mod openai_compat;
pub mod orchestrator;
pub mod pipeline;
pub mod prompt_enhancement;
pub mod speech;
pub mod streaming;
pub mod token_tracker {
    pub use gestura_core_llm::token_tracker::*;
}
pub mod tools;

// ============================================================================
// Inline Modules (domain-crate types surfaced through gestura-core)
// ============================================================================

// -- gestura-core-foundation --
pub mod error {
    pub use gestura_core_foundation::error::*;
}
pub mod events {
    pub use gestura_core_foundation::events::*;
}
pub mod execution_mode {
    pub use gestura_core_foundation::execution_mode::*;
}
pub mod interaction {
    pub use gestura_core_foundation::interaction::*;
}
pub mod model_display {
    pub use gestura_core_foundation::model_display::*;
}
pub mod platform {
    pub use gestura_core_foundation::platform::*;
}
pub mod stream_error {
    pub use gestura_core_foundation::stream_error::*;
}
pub mod stream_health {
    pub use gestura_core_foundation::stream_health::*;
}
pub mod stream_reconnect {
    pub use gestura_core_foundation::stream_reconnect::*;
}
pub mod telemetry {
    pub use gestura_core_foundation::telemetry::*;
}

// -- gestura-core-llm --
pub mod default_models {
    pub use gestura_core_llm::default_models::*;
}
pub mod model_listing {
    pub use gestura_core_llm::model_listing::*;
}

// -- gestura-core-mcp --
pub mod mcp {
    pub use gestura_core_mcp::*;
}

// -- gestura-core-sessions --
pub mod session_manager {
    pub use gestura_core_sessions::session_manager::*;
}
pub mod session_workspace {
    pub use gestura_core_sessions::session_workspace::*;
}

// -- gestura-core-streaming --
pub mod stream_cancellation {
    pub use gestura_core_streaming::cancellation::*;
}

// -- gestura-core-tasks --
pub mod tasks {
    pub use gestura_core_tasks::tasks::*;
}
pub mod workflows {
    pub use gestura_core_tasks::workflows::*;
}

// -- gestura-core-security --
pub mod security {
    pub use gestura_core_security::*;
}
pub mod gdpr {
    pub use gestura_core_security::gdpr::*;
}
pub mod sandbox {
    pub use gestura_core_security::sandbox::*;
}

// -- gestura-core-audio --
pub mod audio {
    pub use gestura_core_audio::noise_cancellation::*;
}
pub mod audio_capture {
    pub use gestura_core_audio::audio_capture::*;
}
pub mod stt_provider {
    pub use gestura_core_audio::stt_provider::*;
}

// -- gestura-core-tools --
pub mod tool_inspection {
    pub use gestura_core_tools::tool_inspection::*;
}

// -- gestura-core-config --
pub mod config_env {
    pub use gestura_core_config::config_env::*;
}

// -- gestura-core-foundation + gestura-core-security --
pub mod secrets {
    pub use gestura_core_foundation::secrets::*;
    pub use gestura_core_security::secrets::SecureStorageSecretProvider;
}

// -- gestura-core-memory-bank --
pub mod memory_bank {
    pub use gestura_core_memory_bank::*;
}

// -- gestura-core-a2a --
pub mod a2a {
    pub use gestura_core_a2a::*;
}

// -- gestura-core-explorer --
pub mod explorer {
    pub use gestura_core_explorer::*;
}

// -- gestura-core-knowledge --
pub mod knowledge {
    pub use gestura_core_knowledge::*;
}

// -- gestura-core-nats --
pub mod nats_mq {
    pub use gestura_core_nats::*;
}

// -- gestura-core-ipc --
pub mod hotkey_ipc {
    pub use gestura_core_ipc::*;
}

// -- gestura-core-analytics --
pub mod analytics {
    pub use gestura_core_analytics::analytics::*;
}
pub mod recommendations {
    pub use gestura_core_analytics::recommendations::*;
}

// -- gestura-core-hooks --
pub mod hooks {
    pub use gestura_core_hooks::*;
}

// -- gestura-core-scripting --
pub mod scripting {
    pub use gestura_core_scripting::*;
}

// -- gestura-core-plugins --
pub mod plugin_system {
    pub use gestura_core_plugins::*;
}

// -- gestura-core-retry --
pub mod retry {
    pub use gestura_core_retry::*;
}

// -- tool_confirmation (merged into gestura-core-tools) --
pub mod tool_confirmation {
    pub use gestura_core_tools::tool_confirmation::*;
}

// Re-export common types for convenience
pub use a2a::{
    A2AClient, A2AError, A2AMessage, A2ARequest, A2AResponse, A2AServer, A2ATask, AgentCard,
    AgentCardRegistry, AgentProfile, Artifact, AuthenticationInfo, MessagePart, OAuth2Config,
    ProfileStore, Skill, create_gestura_agent_card, is_token_well_formed,
};
pub use agent_sessions::{
    AgentSession, AgentSessionResult, AgentSessionStore, ConversationMessage,
    FileAgentSessionStore, MessageSource, SessionFilter, SessionInfo, SessionLlmConfig,
    SessionPermissionLevel, SessionState, SessionToolCall, SessionToolSettings, SessionVoiceConfig,
    default_agent_sessions_dir,
};
pub use analytics::{
    AnalyticsConfig, AnalyticsInsights, ErrorAnalysis, EventType, PerformanceMetrics, PrivacyMode,
    TimePeriod, UsageAnalytics, UsageEvent, UsagePatterns,
};
pub use audio::{
    NoiseCancellationConfig, NoiseCancellationProcessor, NoiseReductionStats,
    create_music_noise_canceller, create_speech_noise_canceller,
};
pub use audio_capture::{
    AudioCaptureConfig, AudioDeviceInfo, is_microphone_available, list_audio_input_devices,
    record_audio, request_stop_recording, reset_stop_flag,
};
pub use checkpoints::{
    Checkpoint, CheckpointError, CheckpointId, CheckpointManager, CheckpointMetadata,
    CheckpointRetentionPolicy, CheckpointSnapshot, FileCheckpointStore, default_checkpoints_dir,
};
pub use compaction::{
    CompactionConfig, CompactionEvent, CompactionEventType, CompactionResult, CompactionStrategy,
    ContextCompactor,
};
pub use config::{
    AppConfig, AppConfigSecurityExt, McpJsonFile, McpScope, McpServerEntry, McpTransportType,
    import_claude_desktop_servers,
};
pub use context::{
    CacheStats, ContextCache, ContextCategory, ContextManager, ContextManagerStats, EntityType,
    ExtractedEntity, FileContext, RequestAnalysis, RequestAnalyzer, ResolvedContext, ToolContext,
};
pub use default_models::{
    DEFAULT_ANTHROPIC_MODEL, DEFAULT_GEMINI_BASE_URL, DEFAULT_GEMINI_MODEL, DEFAULT_GROK_MODEL,
    DEFAULT_OLLAMA_BASE_URL, DEFAULT_OLLAMA_MODEL, DEFAULT_OPENAI_MODEL, DEFAULT_OPENAI_STT_MODEL,
};
pub use error::{AppError, Result};
pub use events::{
    AgentEvent, EventBufferConfig, EventEmitter, EventReceiver, EventSender, ProgressStage,
    ProgressTracker, create_event_channel,
};
pub use execution_mode::{
    ExecutionMode, ModeConfig, ModeManager, ToolCategory, ToolExecutionCheck, ToolPermission,
};
pub use explorer::{
    ExplorerEntry, ExplorerEntryKind, ExplorerError, ExplorerGitChangeKind, ExplorerGitPathStatus,
    ExplorerGitStatusResponse, ExplorerListDirResponse, ExplorerRootResponse, canonical_root,
    ensure_safe_rel_path, list_dir, normalize_git_change_path, resolve_under_root,
};
pub use gdpr::{
    ConsentRecord, ConsentStatus, DataAuditEntry, DataCategory, DataOperation, GdprManager,
    get_gdpr_manager,
};
pub use hooks::{HookContext, HookEngine, HookEvent, HookExecutionRecord, HooksSettings};
pub use interaction::{
    ButtonPressType, GestureType, HapticFeedback, HapticPattern, InteractionContext,
    InteractionEvent, InteractionType, SlideDirection, ToolHint,
};
pub use knowledge::{
    KnowledgeError, KnowledgeItem, KnowledgeMatch, KnowledgeQuery, KnowledgeReference,
    KnowledgeSettingsManager, KnowledgeStore, LoadCondition, SessionKnowledgeSettings,
    register_builtin_knowledge,
};
pub use llm_provider::{
    AgentContext, LlmCallResponse, LlmProvider, TokenUsage, UnconfiguredProvider, select_provider,
};
pub use mcp::{
    CachedTool, JsonRpcError, JsonRpcRequest, JsonRpcResponse, LocalMcp, McpCacheStats, McpClient,
    McpClientRegistry, McpDiscoveryManager, McpIntegrator, McpRequestContext, McpResourceHandler,
    McpServer, McpServerConfig, McpServerInfo, McpToolHandler, MdhResource, PopularMcpServer,
    RegistryBrowseEntry, RegistryBrowsePage, ServerState, TokenInfo, browse_mcp_registry, get_mcp,
    get_mcp_client_registry, list_popular_mcp_servers, mdh_translate, normalize_mcp_server_name,
};
pub use memory_bank::{
    MemoryBankEntry, MemoryBankError, clear_memory_bank, ensure_memory_bank_dir,
    get_memory_bank_dir, list_memory_bank, load_from_memory_bank, save_to_memory_bank,
    search_memory_bank,
};
pub use model_display::{
    format_anthropic_model_name, format_gemini_model_name, format_grok_model_name,
    format_model_name, format_openai_model_name, is_local_provider,
};
pub use model_listing::{ModelInfo, check_ollama_connectivity, list_models_for_provider};
pub use pipeline::{
    AgentPipeline, AgentRequest, AgentResponse, Message, PausedExecutionState, PermissionLevel,
    PipelineConfig, PipelineConfigExt, RequestMetadata, RequestSource, ToolCallRecord, ToolResult,
};
pub use platform::detect_system_dark_mode;
pub use recommendations::{
    PersonalizedRecommendationEngine, Recommendation, RecommendationConfig, RecommendationFeedback,
    RecommendationType, SessionPatterns, UserBehaviorPattern,
};
pub use retry::{ErrorClass, RetryCallback, RetryEvent, RetryManager, RetryPolicy};
pub use session_manager::{AuthToken, SessionManager, TokenType, UserSession, get_session_manager};
pub use session_workspace::{
    SessionWorkspace, WorkspaceError, WorkspaceResult, cleanup_old_sessions, get_sessions_base_dir,
};
pub use speech::{
    LlmResponse, SpeechConfig, SpeechProcessor, SpeechProcessorCoreExt, TranscriptionResult,
    get_speech_processor, is_speech_recording, update_speech_config,
};
pub use stream_error::{StreamError, StreamErrorCategory, StreamResult};
pub use stream_health::{
    StreamHealthConfig, StreamHealthEvent, StreamHealthHandle, StreamHealthMonitor,
    StreamHealthStatus,
};
pub use stream_reconnect::{
    ReconnectConfig, ReconnectEvent, ReconnectManager, ReconnectState, StreamState,
};
pub use streaming::{
    CancellationToken, ShellOutputStream, ShellProcessState, StreamChunk, start_streaming,
};
pub use tasks::{
    Task, TaskError, TaskList, TaskManager, TaskSource, TaskStatus, get_global_task_manager,
};
pub use telemetry::{
    Metric, MetricType, SystemHealth, TelemetryManager, Timer, get_telemetry_manager,
};
pub use token_tracker::{
    BudgetStatus, TokenTracker, UsageRecord, UsageStats, format_token_count, get_token_tracker,
};
pub use tool_inspection::{
    ConfirmationRequest, ConfirmationResponse, InspectionResult, ToolInspectionManager,
    ToolMetadata,
};
pub use tools::{
    ToolDefinition, all_tools, find_tool, looks_like_capabilities_question,
    looks_like_tools_question, render_capabilities, render_tool_detail, render_tools_overview,
};
pub use workflows::{Workflow, WorkflowError, WorkflowInfo, WorkflowManager};

// NATS MQ module exports (messaging/JetStream)
#[cfg(feature = "nats")]
pub use nats_mq::NatsHealthMonitor;
pub use nats_mq::{
    Connection as NatsConnection, DispatchEvent, connect_nats, connect_with_retry, init_jetstream,
    publish_json, spawn_nats_server, subjects, subscribe, subscribe_wildcard,
};

// Sandbox module exports (agent process isolation)
pub use sandbox::{SandboxConfig, SandboxManager, create_default_sandbox};

// Scripting module exports (Lua/Python/JS automation)
pub use scripting::{
    Script, ScriptContext, ScriptExecutionResult, ScriptLanguage, ScriptPermission, ScriptTrigger,
    ScriptingEngine, get_scripting_engine,
};

// Security module exports (SecureStorage, encryption, etc.)
#[cfg(feature = "security")]
pub use security::{Encryptor, KeychainStorage, SecureConfigManager};
pub use security::{
    McpToken, MockSecureStorage, SecureStorage, SecureStorageError, create_secure_storage,
};

// Security/policy helpers and permission primitives.
pub use tools::PermissionManager;
pub use tools::permissions::{Permission, PermissionAuditEntry, PermissionCheck, PermissionScope};
pub use tools::policy::{
    ToolCallDecision, ToolConfirmationInfo, ToolPolicyEvaluation, evaluate_tool_call,
    is_action_allowed, is_shell_command_write_operation, is_write_operation, requires_confirmation,
};

// Agents module exports (agent lifecycle, task delegation)
pub use agents::{
    AgentCommand, AgentEnvelope, AgentInfo, AgentManager, AgentSpawner, AgentStatus, DelegatedTask,
    OrchestratorToolCall, TaskResult,
};

// Orchestrator exports (core-owned orchestration implementation)
pub use orchestrator::{AgentOrchestrator, OrchestratorAgentManager, OrchestratorObserver};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::const_is_empty)]
    fn test_version() {
        // VERSION comes from CARGO_PKG_VERSION which is always non-empty for valid packages
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn test_name() {
        assert_eq!(NAME, "gestura-core");
    }
}
