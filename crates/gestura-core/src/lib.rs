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
//! - `session` - Chat session management

/// Crate version from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Crate name
pub const NAME: &str = env!("CARGO_PKG_NAME");

// ============================================================================
// Core Modules
// ============================================================================

pub mod a2a;
pub mod agents;
pub mod analytics;
pub mod audio;
pub mod audio_capture;
pub mod chat_sessions;
pub mod checkpoints;
pub mod compaction;
pub mod config;
pub mod config_env;
pub mod config_validation;
pub mod config_watcher;
pub mod context;
pub mod error;
pub mod events;
pub mod execution_mode;
pub mod gdpr;
pub mod guardrails;
pub mod hooks;
pub mod interaction;
pub mod knowledge;
pub mod llm_overrides;
pub mod llm_provider;
pub mod llm_validation;
pub mod mcp;
pub mod memory_bank;
pub mod model_display;
pub mod nats_mq;
/// OpenAI(-compatible) API compatibility helpers (e.g., parameter support quirks).
pub mod openai_compat;
pub mod orchestrator;
mod persona;
pub mod pipeline;
pub mod plugin_system;
pub mod prompt_enhancement;
pub mod recommendations;
pub mod retry;
pub mod sandbox;
pub mod scripting;
pub mod secrets;
pub mod security;
pub mod session_manager;
pub mod session_workspace;
pub mod speech;
pub mod stream_cancellation;
pub mod stream_error;
pub mod stream_health;
pub mod stream_reconnect;
pub mod streaming;
pub mod stt_provider;
pub mod tasks;
pub mod telemetry;
pub mod token_tracker;
pub mod tool_confirmation;
pub mod tool_inspection;
pub mod tools;

// Re-export common types for convenience
pub use a2a::{
    A2AClient, A2AError, A2AMessage, A2ARequest, A2AResponse, A2AServer, A2ATask, AgentCard,
    AgentCardRegistry, AgentProfile, Artifact, AuthenticationInfo, MessagePart, OAuth2Config,
    ProfileStore, Skill, create_gestura_agent_card, is_token_well_formed,
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
pub use chat_sessions::{
    ChatSession, ChatSessionResult, ChatSessionStore, ConversationMessage, FileChatSessionStore,
    MessageSource, SessionFilter, SessionInfo, SessionLlmConfig, SessionPermissionLevel,
    SessionState, SessionToolCall, SessionToolSettings, SessionVoiceConfig,
    default_chat_sessions_dir,
};
pub use checkpoints::{
    Checkpoint, CheckpointError, CheckpointId, CheckpointManager, CheckpointMetadata,
    CheckpointRetentionPolicy, CheckpointSnapshot, FileCheckpointStore, default_checkpoints_dir,
};
pub use compaction::{
    CompactionConfig, CompactionEvent, CompactionEventType, CompactionResult, CompactionStrategy,
    ContextCompactor,
};
pub use config::AppConfig;
pub use context::{
    CacheStats, ContextCache, ContextCategory, ContextManager, ContextManagerStats, EntityType,
    ExtractedEntity, FileContext, RequestAnalysis, RequestAnalyzer, ResolvedContext, ToolContext,
};
pub use error::{AppError, Result};
pub use events::{
    AgentEvent, EventBufferConfig, EventEmitter, EventReceiver, EventSender, ProgressStage,
    ProgressTracker, create_event_channel,
};
pub use execution_mode::{
    ExecutionMode, ModeConfig, ModeManager, ToolCategory, ToolExecutionCheck, ToolPermission,
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
    CachedTool, JsonRpcError, JsonRpcRequest, JsonRpcResponse, LocalMcp, McpCacheStats,
    McpDiscoveryManager, McpIntegrator, McpRequestContext, McpResourceHandler, McpServer,
    McpServerConfig, McpServerInfo, McpToolHandler, MdhResource, ServerState, TokenInfo, get_mcp,
    mdh_translate,
};
pub use memory_bank::{
    MemoryBankEntry, MemoryBankError, clear_memory_bank, ensure_memory_bank_dir,
    get_memory_bank_dir, list_memory_bank, load_from_memory_bank, save_to_memory_bank,
    search_memory_bank,
};
pub use model_display::{
    format_anthropic_model_name, format_grok_model_name, format_model_name,
    format_openai_model_name, is_local_provider,
};
pub use pipeline::{
    AgentPipeline, AgentRequest, AgentResponse, Message, PermissionLevel, PipelineConfig,
    RequestMetadata, RequestSource, ToolCallRecord, ToolResult,
};
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
    LlmResponse, SpeechConfig, SpeechProcessor, TranscriptionResult, get_speech_processor,
    is_speech_recording, update_speech_config,
};
pub use stream_error::{StreamError, StreamErrorCategory, StreamResult};
pub use stream_health::{
    StreamHealthConfig, StreamHealthEvent, StreamHealthHandle, StreamHealthMonitor,
    StreamHealthStatus,
};
pub use stream_reconnect::{
    ReconnectConfig, ReconnectEvent, ReconnectManager, ReconnectState, StreamState,
};
pub use streaming::{CancellationToken, StreamChunk, start_streaming};
pub use tasks::{Task, TaskError, TaskList, TaskManager, TaskSource, TaskStatus};
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
