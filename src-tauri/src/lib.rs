//! Library crate for Gestura backend (Tauri v2)
//! Re-exports core modules for use across the application.

// Core modules
pub mod config;
pub mod error;

// UI and interface
pub mod commands;
pub mod hotkeys;
pub mod tray;

// Hardware and device features
pub mod ble;
pub mod haptics;
pub mod simulator;

// Voice and AI features
pub mod audio_capture;
pub mod speech;
pub mod voice;

// Integration modules
pub mod mcp;
pub mod mcp_server;
pub mod mq;
pub mod nats_mq;

// System utilities
pub mod agents;
pub mod dispatcher;
pub mod security;

// Automated testing
pub mod automated_testing;

// Window and session management
pub mod custom_gestures;
pub mod developer_sdk;
pub mod device_simulator;
pub mod error_recovery;
pub mod federated_learning;
pub mod gdpr;
pub mod gesture_pattern_learning;
pub mod mdh_translator;
pub mod mdh_uri_resolver;
pub mod memory_bus;
pub mod noise_cancellation;
pub mod permissions;
pub mod personalized_recommendations;
pub mod plugin_system;
pub mod predictive_text;
pub mod process_spawner;
pub mod query_optimizer;
pub mod sandbox;
pub mod scripting_engine;
pub mod secure_ipc;
pub mod session_manager;
pub mod speaker_identification;
pub mod telemetry;
pub mod third_party_integrations;
pub mod usage_analytics;
pub mod voice_activity_detection;
pub mod voice_model_tuning;
pub mod window_manager;

pub mod kv;
pub mod voice_select;

pub mod api;
pub mod llm_provider;

// Unified NATS connection type alias that compiles with or without feature
#[cfg(feature = "nats")]
pub type NatsConn = async_nats::Client;
#[cfg(not(feature = "nats"))]
pub type NatsConn = ();

pub use config::AppConfig;
pub use error::AppError;

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
