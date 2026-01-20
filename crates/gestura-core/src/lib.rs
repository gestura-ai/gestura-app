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
pub mod audio_capture;
pub mod config;
pub mod context;
pub mod error;
pub mod gdpr;
pub mod interaction;
pub mod knowledge;
pub mod llm_provider;
pub mod mcp;
mod persona;
pub mod pipeline;
pub mod retry;
pub mod session_manager;
pub mod session_workspace;
pub mod speech;
pub mod streaming;
pub mod telemetry;
pub mod token_tracker;
pub mod tools;

// Re-export common types for convenience
pub use audio_capture::{
    AudioCaptureConfig, AudioDeviceInfo, is_microphone_available, list_audio_input_devices,
    record_audio, request_stop_recording, reset_stop_flag,
};
pub use config::AppConfig;
pub use context::{
    CacheStats, ContextCache, ContextCategory, ContextManager, ContextManagerStats, EntityType,
    ExtractedEntity, FileContext, RequestAnalysis, RequestAnalyzer, ResolvedContext, ToolContext,
};
pub use error::{AppError, Result};
pub use gdpr::{
    ConsentRecord, ConsentStatus, DataAuditEntry, DataCategory, DataOperation, GdprManager,
    get_gdpr_manager,
};
pub use interaction::{
    ButtonPressType, GestureType, HapticFeedback, HapticPattern, InteractionContext,
    InteractionEvent, InteractionType, SlideDirection, ToolHint,
};
pub use knowledge::{
    KnowledgeError, KnowledgeItem, KnowledgeMatch, KnowledgeQuery, KnowledgeReference,
    KnowledgeStore, LoadCondition, register_builtin_knowledge,
};
#[cfg(any(feature = "dev", test))]
pub use llm_provider::EchoProvider;
pub use llm_provider::{
    AgentContext, LlmCallResponse, LlmProvider, TokenUsage, UnconfiguredProvider, select_provider,
};
pub use mcp::{LocalMcp, McpIntegrator, MdhResource, TokenInfo, get_mcp, mdh_translate};
pub use pipeline::{
    AgentPipeline, AgentRequest, AgentResponse, Message, PipelineConfig, RequestMetadata,
    RequestSource, ToolCallRecord, ToolResult,
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
pub use streaming::{CancellationToken, StreamChunk, start_streaming};
pub use telemetry::{
    Metric, MetricType, SystemHealth, TelemetryManager, Timer, get_telemetry_manager,
};
pub use token_tracker::{
    BudgetStatus, TokenTracker, UsageRecord, UsageStats, format_token_count, get_token_tracker,
};
pub use tools::{
    ToolDefinition, all_tools, find_tool, looks_like_capabilities_question,
    looks_like_tools_question, render_capabilities, render_tool_detail, render_tools_overview,
};

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
