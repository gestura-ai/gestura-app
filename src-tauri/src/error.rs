//! Application error types for Gestura
//! Centralized error enum to simplify error propagation and future extensibility.

use thiserror::Error;

/// Top-level application error.
#[derive(Debug, Error)]
pub enum AppError {
    /// Generic I/O failure
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// NATS client error
    #[error("NATS error: {0}")]
    Nats(String),

    /// Placeholder for BLE-related failures (Stage 2)
    #[error("BLE error: {0}")]
    Ble(String),

    /// Placeholder for LLM-related failures (Stage 4)
    #[error("LLM error: {0}")]
    Llm(String),

    /// HTTP client error
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// Voice/STT error
    #[error("Voice error: {0}")]
    Voice(String),
}

