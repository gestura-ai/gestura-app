//! Application error types for Gestura Core
//!
//! Centralized error enum to simplify error propagation and future extensibility.
//! This module provides the foundational error types used across the Gestura ecosystem.

use thiserror::Error;

/// Top-level application error for Gestura Core.
///
/// This error type encompasses all possible failure modes in the core library,
/// allowing for unified error handling across GUI and CLI applications.
#[derive(Debug, Error)]
pub enum AppError {
    /// Generic I/O failure (file operations, etc.)
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// TOML parsing/serialization error
    #[error("TOML error: {0}")]
    Toml(String),

    /// NATS messaging client error
    #[error("NATS error: {0}")]
    Nats(String),

    /// BLE-related failures for haptic device communication
    #[error("BLE error: {0}")]
    Ble(String),

    /// LLM-related failures (API errors, model not found, etc.)
    #[error("LLM error: {0}")]
    Llm(String),

    /// HTTP client error (for API calls)
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// Voice/STT processing error
    #[error("Voice error: {0}")]
    Voice(String),

    /// Audio capture/playback error
    #[error("Audio error: {0}")]
    Audio(String),

    /// Configuration error (invalid settings, missing config, etc.)
    #[error("Config error: {0}")]
    Config(String),

    /// Session management error
    #[error("Session error: {0}")]
    Session(String),

    /// MCP (Model Context Protocol) error
    #[error("MCP error: {0}")]
    Mcp(String),

    /// Permission denied error (for system operations)
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Resource not found error
    #[error("Not found: {0}")]
    NotFound(String),

    /// Timeout error
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Invalid input or argument error
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// Internal error (unexpected state, etc.)
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type alias for operations that may fail with AppError
pub type Result<T> = std::result::Result<T, AppError>;

/// Convert a TOML parsing error to AppError
impl From<toml::de::Error> for AppError {
    fn from(err: toml::de::Error) -> Self {
        AppError::Toml(err.to_string())
    }
}

/// Convert a TOML serialization error to AppError
impl From<toml::ser::Error> for AppError {
    fn from(err: toml::ser::Error) -> Self {
        AppError::Toml(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let app_err: AppError = io_err.into();
        assert!(matches!(app_err, AppError::Io(_)));
        assert!(app_err.to_string().contains("file not found"));
    }

    #[test]
    fn test_llm_error() {
        let err = AppError::Llm("API rate limit exceeded".to_string());
        assert!(err.to_string().contains("API rate limit exceeded"));
    }

    #[test]
    fn test_config_error() {
        let err = AppError::Config("Missing API key".to_string());
        assert!(err.to_string().contains("Missing API key"));
    }

    #[test]
    fn test_result_type_alias() {
        fn fallible_operation() -> Result<i32> {
            Ok(42)
        }
        assert_eq!(fallible_operation().unwrap(), 42);
    }
}
