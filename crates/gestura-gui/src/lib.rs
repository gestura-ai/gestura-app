//! Library crate for Gestura backend (Tauri v2)
//!
//! This crate provides the Tauri-specific shell around gestura-core.
//! Core business logic is in gestura-core; this crate adds:
//! - Tauri commands and API bindings
//! - System tray and hotkey handling
//! - Window management
//! - Hardware integrations (BLE, haptics)

// ============================================================================
// Re-export core modules from gestura-core
// ============================================================================

// Core types and configuration (re-exported from gestura-core)
pub use gestura_core::AppConfig;
pub use gestura_core::AppError;
pub use gestura_core::Result;
pub use gestura_core::config;
pub use gestura_core::error;

// Core modules re-exported from gestura-core
pub use gestura_core::llm_provider;
pub use gestura_core::mcp;
pub use gestura_core::session_manager;

// System tools from gestura-core
pub use gestura_core::tools;

// ============================================================================
// Tauri-specific modules that wrap gestura-core functionality
// These modules add Tauri-specific features (AppHandle, events, etc.)
// ============================================================================

// Audio and speech (Tauri-specific wrappers with AppHandle support)
pub mod audio_capture;
pub mod speech;

// GDPR and telemetry (Tauri-specific with additional methods)
pub mod gdpr;
pub mod telemetry;

// ============================================================================
// Tauri-specific modules (not in gestura-core)
// ============================================================================

// UI and interface
pub mod commands;
pub mod hotkeys;
pub mod tray;

// Hardware and device features
pub mod ble;
pub mod haptics;
pub mod simulator;

// Voice extensions (Tauri-specific wrappers)
pub mod voice;

// Integration modules (Tauri-specific)
pub mod mcp_server;
pub mod mq;
pub mod nats_mq;

// System utilities
pub mod agents;
pub mod dispatcher;
pub mod orchestrator;
pub mod security;

// Agent-to-agent protocol (A2A)
pub mod a2a;

// Automated testing
pub mod automated_testing;

// Window and session management
pub mod custom_gestures;
pub mod developer_sdk;
pub mod device_simulator;
pub mod error_recovery;
pub mod federated_learning;
pub mod gesture_pattern_learning;
pub mod mdh_translator;
pub mod mdh_uri_resolver;
pub mod memory_bus;
pub mod noise_cancellation;
pub mod notifications;
pub mod permissions;
pub mod personalized_recommendations;
pub mod plugin_system;
pub mod predictive_text;
pub mod process_spawner;
pub mod query_optimizer;
pub mod sandbox;
pub mod scripting_engine;
pub mod secure_ipc;
pub mod shell_session;
pub mod speaker_identification;
pub mod third_party_integrations;
pub mod usage_analytics;
pub mod voice_activity_detection;
pub mod voice_model_tuning;
pub mod window_manager;

pub mod kv;
pub mod voice_select;

/// Chat event utilities (window-scoped emission + optional diagnostics trace).
pub mod chat_events;

/// Small shared utilities.
pub(crate) mod text_utils;

/// Frontend receipt tracing utilities (diagnostics-only).
pub mod chat_receipts;

/// Multi-window chat isolation probe utilities (diagnostics-only).
pub mod chat_probe;

/// Task integration for bidirectional sync between AgentOrchestrator and TaskManager.
pub mod task_integration;

pub mod api;

// ============================================================================
// NATS connection type alias
// ============================================================================

#[cfg(feature = "nats")]
pub type NatsConn = async_nats::Client;
#[cfg(not(feature = "nats"))]
pub type NatsConn = ();

/// Global application state managed by Tauri
#[derive(Clone)]
pub struct AppState {
    /// Async NATS connection (if available)
    pub nats: Option<crate::NatsConn>,
    /// Agent manager for lifecycle
    pub agents: agents::AgentManager,
    /// Application configuration
    pub config: AppConfig,
    /// Ring manager for BLE operations (optional)
    pub ring_manager: Option<std::sync::Arc<dyn ble::RingManager>>,
}
