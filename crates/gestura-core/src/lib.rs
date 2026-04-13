//! Gestura's public core facade.
//!
//! `gestura-core` is the stable Rust API surface for Gestura.app. It owns the
//! cross-domain orchestration that ties the workspace together and re-exports
//! focused `gestura-core-*` domain crates so downstream code can import through
//! a single, consistent path.
//!
//! ## Generated docs quick start
//!
//! If you are exploring the generated docs, start with these high-signal module
//! entry points:
//!
//! - [`pipeline`]: main agent execution flow and request/response model
//! - [`tools`]: built-in tools, permissions, schemas, and confirmation helpers
//! - [`config`]: application configuration plus secret-hydration/runtime bridges
//! - [`llm_provider`]: provider selection and LLM facade access
//! - [`mcp`] and [`a2a`]: protocol surfaces for MCP and agent-to-agent flows
//! - [`agent_sessions`], [`session_manager`], and [`tasks`]: session and task state
//! - [`knowledge`], [`memory_bank`], and [`memory_console`]: expert knowledge and memory
//! - [`security`] and [`streaming`]: security/runtime streaming boundaries
//!
//! A good reading path is:
//!
//! 1. start here in `gestura-core`
//! 2. jump into one of the facade modules above
//! 3. follow the re-export or module docs into the owning `gestura-core-*` crate
//!    when you need domain-level detail
//!
//! ## Core-first architecture
//!
//! The workspace follows a core-first layout:
//!
//! - domain logic lives in dedicated crates such as `gestura-core-tools`,
//!   `gestura-core-mcp`, `gestura-core-config`, and `gestura-core-context`
//! - `gestura-core` re-exports those crates under stable public module names
//! - presentation layers (`gestura-cli`, `gestura-gui`) stay thin and delegate
//!   to this facade instead of owning business logic
//!
//! In practice, this means new work usually follows this path:
//!
//! 1. add or update behavior in the relevant domain crate
//! 2. expose the stable public entry point from `gestura-core`
//! 3. consume the stable API from CLI and GUI code
//!
//! ## What this crate owns directly
//!
//! This facade is intentionally more than a re-export crate. It also owns the
//! integration points that combine multiple domains, including:
//!
//! - the agent pipeline and tool execution loop
//! - provider selection from application configuration
//! - guardrails, checkpoints, compaction, and orchestration helpers
//! - configuration bridges that depend on security or runtime integration
//! - shared surfaces consumed by both CLI and GUI entry points
//!
//! ## High-signal module groups
//!
//! - `pipeline`: agent request execution, tool routing, reflection, and
//!   streaming integration
//! - `tools`: stable access to built-in tools, schemas, permissions, and
//!   streaming shell helpers
//! - `config`: application configuration plus core-owned security bridges
//! - `llm_provider`: provider selection and facade access to LLM types
//! - `mcp`, `a2a`, `knowledge`, `memory_bank`: protocol and knowledge surfaces
//! - `session_manager`, `session_workspace`, `agent_sessions`: session state and
//!   workspace lifecycle
//!
//! ## Cargo features
//!
//! The facade exposes optional capabilities through Cargo features:
//!
//! - `voice-local`: local Whisper speech-to-text support
//! - `nats`: NATS messaging integration
//! - `json-ld`: JSON-LD processing for MDH workflows
//! - `security`: encryption, keychain, and secure-storage integrations
//! - `macos-permissions`, `linux-permissions`, `windows-permissions`:
//!   platform-specific permission helpers
//!
//! ## Documentation strategy
//!
//! The long-term goal is for crate-level and module-level Rustdoc to become the
//! canonical architecture and API reference surfaced by `cargo doc`. External
//! documents should increasingly focus on operational workflows such as install,
//! packaging, release, and troubleshooting.

/// Crate version from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Crate name
pub const NAME: &str = env!("CARGO_PKG_NAME");

// ============================================================================
// Core Modules (file-backed — business logic + domain-crate integration)
// ============================================================================

pub mod agent_sessions;
/// Agent lifecycle management and orchestration primitives re-exported from
/// `gestura-core-agents`.
pub mod agents {
    pub use gestura_core_agents::*;
}
pub mod checkpoints;
pub mod compaction;
pub mod config;
/// Smart request analysis, entity extraction, and context resolution re-exported
/// from `gestura-core-context`.
pub mod context {
    pub use gestura_core_context::*;
}
pub(crate) mod guardrails;
pub mod llm_overrides;
pub mod llm_provider;
pub mod llm_validation;
pub mod memory_console;
/// OpenAI(-compatible) API compatibility helpers (e.g., parameter support quirks).
pub mod openai_compat;
pub mod orchestrator;
pub mod pipeline;
pub mod prompt_enhancement;
pub mod speech;
pub mod streaming;
/// Token accounting helpers re-exported from `gestura-core-llm`.
pub mod token_tracker {
    pub use gestura_core_llm::token_tracker::*;
}
pub mod tools;

// ============================================================================
// Inline Modules (domain-crate types surfaced through gestura-core)
// ============================================================================

// -- gestura-core-foundation --
/// Shared application errors and result aliases re-exported from
/// `gestura-core-foundation`.
pub mod error {
    pub use gestura_core_foundation::error::*;
}
/// Outcome-linked learning signals re-exported from `gestura-core-foundation`.
pub mod outcomes {
    pub use gestura_core_foundation::outcomes::*;
}
/// Cross-cutting event types re-exported from `gestura-core-foundation`.
pub mod events {
    pub use gestura_core_foundation::events::*;
}
/// Execution-mode policy primitives re-exported from
/// `gestura-core-foundation`.
pub mod execution_mode {
    pub use gestura_core_foundation::execution_mode::*;
}
/// Shared interaction model types re-exported from `gestura-core-foundation`.
pub mod interaction {
    pub use gestura_core_foundation::interaction::*;
}
/// Human-friendly model display helpers re-exported from
/// `gestura-core-foundation`.
pub mod model_display {
    pub use gestura_core_foundation::model_display::*;
}
/// Platform-detection helpers re-exported from `gestura-core-foundation`.
pub mod platform {
    pub use gestura_core_foundation::platform::*;
}
/// Streaming error types re-exported from `gestura-core-foundation`.
pub mod stream_error {
    pub use gestura_core_foundation::stream_error::*;
}
/// Streaming health state types re-exported from `gestura-core-foundation`.
pub mod stream_health {
    pub use gestura_core_foundation::stream_health::*;
}
/// Streaming reconnection helpers re-exported from `gestura-core-foundation`.
pub mod stream_reconnect {
    pub use gestura_core_foundation::stream_reconnect::*;
}
/// Telemetry and instrumentation types re-exported from
/// `gestura-core-foundation`.
pub mod telemetry {
    pub use gestura_core_foundation::telemetry::*;
}

// -- gestura-core-llm --
/// Built-in provider model defaults re-exported from `gestura-core-llm`.
pub mod default_models {
    pub use gestura_core_llm::default_models::*;
}
/// Model discovery and listing helpers re-exported from `gestura-core-llm`.
pub mod model_listing {
    pub use gestura_core_llm::model_listing::*;
}

// -- gestura-core-mcp --
/// Model Context Protocol types and services re-exported from
/// `gestura-core-mcp`.
pub mod mcp {
    pub use gestura_core_mcp::*;
}

// -- gestura-core-sessions --
/// Authentication and session-management services re-exported from
/// `gestura-core-sessions`.
pub mod session_manager {
    pub use gestura_core_sessions::session_manager::*;
}
/// Session-scoped workspace management re-exported from
/// `gestura-core-sessions`.
pub mod session_workspace {
    pub use gestura_core_sessions::session_workspace::*;
}

// -- gestura-core-streaming --
/// Streaming cancellation primitives re-exported from
/// `gestura-core-streaming`.
pub mod stream_cancellation {
    pub use gestura_core_streaming::cancellation::*;
}

// -- gestura-core-tasks --
/// Task-management primitives re-exported from `gestura-core-tasks`.
pub mod tasks {
    pub use gestura_core_tasks::tasks::*;
}
/// Workflow helpers re-exported from `gestura-core-tasks`.
pub mod workflows {
    pub use gestura_core_tasks::workflows::*;
}

// -- gestura-core-security --
/// Security domain types and services re-exported from `gestura-core-security`.
pub mod security {
    pub use gestura_core_security::*;
}
/// GDPR-specific helpers re-exported from `gestura-core-security`.
pub mod gdpr {
    pub use gestura_core_security::gdpr::*;
}
/// Sandbox primitives re-exported from `gestura-core-security`.
pub mod sandbox {
    pub use gestura_core_security::sandbox::*;
}

// -- gestura-core-audio --
/// Audio noise-cancellation utilities re-exported from `gestura-core-audio`.
pub mod audio {
    pub use gestura_core_audio::noise_cancellation::*;
}
/// Audio capture helpers re-exported from `gestura-core-audio`.
pub mod audio_capture {
    pub use gestura_core_audio::audio_capture::*;
}
/// Speech-to-text provider interfaces re-exported from `gestura-core-audio`.
pub mod stt_provider {
    pub use gestura_core_audio::stt_provider::*;
}

// -- gestura-core-tools --
/// Tool inspection and review helpers re-exported from `gestura-core-tools`.
pub mod tool_inspection {
    pub use gestura_core_tools::tool_inspection::*;
}

// -- gestura-core-config --
/// Environment-based configuration loading re-exported from
/// `gestura-core-config`.
pub mod config_env {
    pub use gestura_core_config::config_env::*;
}

// -- gestura-core-foundation + gestura-core-security --
/// Secret-provider abstractions surfaced from foundation and security crates.
pub mod secrets {
    pub use gestura_core_foundation::secrets::*;
    pub use gestura_core_security::secrets::SecureStorageSecretProvider;
}

// -- gestura-core-memory-bank --
/// Persistent memory-bank types and services re-exported from
/// `gestura-core-memory-bank`.
pub mod memory_bank {
    pub use gestura_core_memory_bank::*;
}

// -- gestura-core-a2a --
/// Agent-to-Agent protocol types and helpers re-exported from
/// `gestura-core-a2a`.
pub mod a2a {
    pub use gestura_core_a2a::*;
}

// -- gestura-core-explorer --
/// File-system exploration helpers re-exported from `gestura-core-explorer`.
pub mod explorer {
    pub use gestura_core_explorer::*;
}

// -- gestura-core-knowledge --
/// Built-in knowledge and expertise surfaces re-exported from
/// `gestura-core-knowledge`.
pub mod knowledge {
    pub use gestura_core_knowledge::*;
}

// -- gestura-core-nats --
/// NATS messaging primitives re-exported from `gestura-core-nats`.
pub mod nats_mq {
    pub use gestura_core_nats::*;
}

// -- gestura-core-ipc --
/// Hotkey inter-process communication types re-exported from `gestura-core-ipc`.
pub mod hotkey_ipc {
    pub use gestura_core_ipc::*;
}

// -- gestura-core-analytics --
/// Usage analytics types re-exported from `gestura-core-analytics`.
pub mod analytics {
    pub use gestura_core_analytics::analytics::*;
}
/// Recommendation types re-exported from `gestura-core-analytics`.
pub mod recommendations {
    pub use gestura_core_analytics::recommendations::*;
}

// -- gestura-core-hooks --
/// Hook engine types re-exported from `gestura-core-hooks`.
pub mod hooks {
    pub use gestura_core_hooks::*;
}

// -- gestura-core-scripting --
/// Scripting-engine types re-exported from `gestura-core-scripting`.
pub mod scripting {
    pub use gestura_core_scripting::*;
}

// -- gestura-core-plugins --
/// Plugin-system types re-exported from `gestura-core-plugins`.
pub mod plugin_system {
    pub use gestura_core_plugins::*;
}

// -- gestura-core-retry --
/// Retry strategy helpers re-exported from `gestura-core-retry`.
pub mod retry {
    pub use gestura_core_retry::*;
}

// -- gestura-core-intent --
/// Unified intent normalization layer re-exported from `gestura-core-intent`.
pub mod intent {
    pub use gestura_core_intent::*;
}

// -- tool_confirmation (merged into gestura-core-tools) --
/// Tool-confirmation flows re-exported from `gestura-core-tools`.
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
    SessionPermissionLevel, SessionReflectionSettings, SessionState, SessionToolCall,
    SessionToolSettings, SessionVoiceConfig, default_agent_sessions_dir,
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
    MemoryBankEntry, MemoryBankError, MemoryBankQuery, MemoryGovernanceRefreshReport,
    MemoryGovernanceRelationship, MemoryGovernanceState, MemoryGovernanceSuggestion, MemoryKind,
    MemoryScope, MemorySearchResult, MemoryType, ReflectionMemoryState, clear_memory_bank,
    ensure_memory_bank_dir, get_memory_bank_dir, list_memory_bank, load_from_memory_bank,
    refresh_memory_bank_governance, save_to_memory_bank, search_memory_bank,
    search_memory_bank_with_query,
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
    Task, TaskError, TaskList, TaskManager, TaskMemoryEvent, TaskMemoryLifecycle, TaskMemoryPhase,
    TaskSource, TaskStatus, get_global_task_manager,
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
