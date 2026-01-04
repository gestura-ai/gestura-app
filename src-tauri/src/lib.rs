//! Library crate for Gestura backend (Tauri v2)
//! Re-exports core modules for use across the application.

// Core modules
pub mod config;
pub mod error;

// UI and interface
pub mod tray;
pub mod hotkeys;
pub mod commands;

// Hardware and device features
pub mod ble;
pub mod haptics;
pub mod simulator;

// Voice and AI features
pub mod voice;
pub mod speech;

// Integration modules
pub mod mcp;
pub mod mcp_server;
pub mod mq;
pub mod nats_mq;

// System utilities
pub mod agents;
pub mod security;
pub mod dispatcher;

// Automated testing
pub mod automated_testing;

// Window and session management
pub mod window_manager;
pub mod mdh_translator;
pub mod sandbox;
pub mod memory_bus;
pub mod secure_ipc;
pub mod device_simulator;
pub mod error_recovery;
pub mod permissions;
pub mod gdpr;
pub mod telemetry;
pub mod mdh_uri_resolver;
pub mod session_manager;
pub mod voice_activity_detection;
pub mod speaker_identification;
pub mod noise_cancellation;
pub mod query_optimizer;
pub mod voice_model_tuning;
pub mod gesture_pattern_learning;
pub mod usage_analytics;
pub mod predictive_text;
pub mod personalized_recommendations;
pub mod federated_learning;
pub mod plugin_system;
pub mod custom_gestures;
pub mod scripting_engine;
pub mod third_party_integrations;
pub mod developer_sdk;
pub mod process_spawner;

pub mod kv;
pub mod voice_select;

pub mod llm_provider;
pub mod api;

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

