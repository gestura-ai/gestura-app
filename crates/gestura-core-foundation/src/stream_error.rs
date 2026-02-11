//! Streaming Error Types
//!
//! Provides standardized error types for streaming APIs with proper
//! categorization, logging, and frontend propagation.

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// Streaming error category for classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamErrorCategory {
    /// Network-related errors (connection, timeout)
    Network,
    /// Authentication/authorization errors
    Auth,
    /// Rate limiting errors
    RateLimit,
    /// Provider-specific errors
    Provider,
    /// Invalid request/response format
    Format,
    /// Resource exhaustion (tokens, quota)
    Resource,
    /// Internal errors
    Internal,
    /// Cancellation (not really an error)
    Cancelled,
}

impl StreamErrorCategory {
    /// Check if this error category is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            StreamErrorCategory::Network
                | StreamErrorCategory::RateLimit
                | StreamErrorCategory::Provider
        )
    }

    /// Get suggested retry delay in milliseconds
    pub fn suggested_retry_delay_ms(&self) -> Option<u64> {
        match self {
            StreamErrorCategory::Network => Some(1000),
            StreamErrorCategory::RateLimit => Some(5000),
            StreamErrorCategory::Provider => Some(2000),
            _ => None,
        }
    }
}

/// Streaming error with rich context
#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub struct StreamError {
    /// Error category
    pub category: StreamErrorCategory,
    /// Error code (provider-specific or internal)
    pub code: String,
    /// Human-readable message
    pub message: String,
    /// Provider name if applicable
    pub provider: Option<String>,
    /// HTTP status code if applicable
    pub http_status: Option<u16>,
    /// Whether this error is retryable
    pub retryable: bool,
    /// Suggested retry delay in milliseconds
    pub retry_after_ms: Option<u64>,
    /// Additional context for debugging
    pub context: Option<String>,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl StreamError {
    /// Create a new stream error
    pub fn new(
        category: StreamErrorCategory,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let retryable = category.is_retryable();
        let retry_after_ms = category.suggested_retry_delay_ms();
        Self {
            category,
            code: code.into(),
            message: message.into(),
            provider: None,
            http_status: None,
            retryable,
            retry_after_ms,
            context: None,
        }
    }

    /// Create a network error
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(StreamErrorCategory::Network, "NETWORK_ERROR", message)
    }

    /// Create a timeout error
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(StreamErrorCategory::Network, "TIMEOUT", message)
    }

    /// Create an auth error
    pub fn auth(message: impl Into<String>) -> Self {
        Self::new(StreamErrorCategory::Auth, "AUTH_ERROR", message)
    }

    /// Create a rate limit error
    pub fn rate_limit(message: impl Into<String>, retry_after_ms: Option<u64>) -> Self {
        let mut err = Self::new(StreamErrorCategory::RateLimit, "RATE_LIMITED", message);
        err.retry_after_ms = retry_after_ms;
        err
    }

    /// Create a provider error
    pub fn provider(provider: impl Into<String>, message: impl Into<String>) -> Self {
        let mut err = Self::new(StreamErrorCategory::Provider, "PROVIDER_ERROR", message);
        err.provider = Some(provider.into());
        err
    }

    /// Create a format error
    pub fn format(message: impl Into<String>) -> Self {
        Self::new(StreamErrorCategory::Format, "FORMAT_ERROR", message)
    }

    /// Create a resource exhaustion error
    pub fn resource(message: impl Into<String>) -> Self {
        Self::new(StreamErrorCategory::Resource, "RESOURCE_EXHAUSTED", message)
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StreamErrorCategory::Internal, "INTERNAL_ERROR", message)
    }

    /// Create a cancellation error
    pub fn cancelled() -> Self {
        let mut err = Self::new(
            StreamErrorCategory::Cancelled,
            "CANCELLED",
            "Stream was cancelled",
        );
        err.retryable = false;
        err
    }

    /// Set provider
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Set HTTP status
    pub fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    /// Set context
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Set retry delay
    pub fn with_retry_after(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self.retryable = true;
        self
    }

    /// Mark as non-retryable
    pub fn non_retryable(mut self) -> Self {
        self.retryable = false;
        self.retry_after_ms = None;
        self
    }

    /// Parse error from HTTP response
    pub fn from_http_response(provider: &str, status: u16, body: &str) -> Self {
        let category = match status {
            401 | 403 => StreamErrorCategory::Auth,
            429 => StreamErrorCategory::RateLimit,
            400 | 422 => StreamErrorCategory::Format,
            500..=599 => StreamErrorCategory::Provider,
            _ => StreamErrorCategory::Internal,
        };

        let code = format!("HTTP_{}", status);
        let message = if body.is_empty() {
            format!("HTTP {} error from {}", status, provider)
        } else {
            // Try to extract error message from JSON
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
                json.get("error")
                    .and_then(|e| e.get("message").or(Some(e)))
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| body.chars().take(200).collect())
            } else {
                body.chars().take(200).collect()
            }
        };

        Self::new(category, code, message)
            .with_provider(provider)
            .with_http_status(status)
    }

    /// Log the error with appropriate level
    pub fn log(&self) {
        match self.category {
            StreamErrorCategory::Cancelled => {
                tracing::debug!(
                    category = ?self.category,
                    code = %self.code,
                    "Stream cancelled"
                );
            }
            StreamErrorCategory::RateLimit => {
                tracing::warn!(
                    category = ?self.category,
                    code = %self.code,
                    provider = ?self.provider,
                    retry_after_ms = ?self.retry_after_ms,
                    "Rate limited: {}", self.message
                );
            }
            StreamErrorCategory::Auth => {
                tracing::error!(
                    category = ?self.category,
                    code = %self.code,
                    provider = ?self.provider,
                    "Authentication error: {}", self.message
                );
            }
            _ => {
                tracing::error!(
                    category = ?self.category,
                    code = %self.code,
                    provider = ?self.provider,
                    http_status = ?self.http_status,
                    retryable = self.retryable,
                    "Stream error: {}", self.message
                );
            }
        }
    }
}

/// Result type for streaming operations
pub type StreamResult<T> = Result<T, StreamError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_error_category_retryable() {
        assert!(StreamErrorCategory::Network.is_retryable());
        assert!(StreamErrorCategory::RateLimit.is_retryable());
        assert!(StreamErrorCategory::Provider.is_retryable());
        assert!(!StreamErrorCategory::Auth.is_retryable());
        assert!(!StreamErrorCategory::Format.is_retryable());
    }

    #[test]
    fn test_stream_error_new() {
        let err = StreamError::new(StreamErrorCategory::Network, "TEST", "Test error");
        assert_eq!(err.category, StreamErrorCategory::Network);
        assert_eq!(err.code, "TEST");
        assert_eq!(err.message, "Test error");
        assert!(err.retryable);
    }

    #[test]
    fn test_stream_error_network() {
        let err = StreamError::network("Connection failed");
        assert_eq!(err.category, StreamErrorCategory::Network);
        assert_eq!(err.code, "NETWORK_ERROR");
        assert!(err.retryable);
    }

    #[test]
    fn test_stream_error_timeout() {
        let err = StreamError::timeout("Request timed out");
        assert_eq!(err.category, StreamErrorCategory::Network);
        assert_eq!(err.code, "TIMEOUT");
    }

    #[test]
    fn test_stream_error_auth() {
        let err = StreamError::auth("Invalid API key");
        assert_eq!(err.category, StreamErrorCategory::Auth);
        assert!(!err.retryable);
    }

    #[test]
    fn test_stream_error_rate_limit() {
        let err = StreamError::rate_limit("Too many requests", Some(5000));
        assert_eq!(err.category, StreamErrorCategory::RateLimit);
        assert_eq!(err.retry_after_ms, Some(5000));
        assert!(err.retryable);
    }

    #[test]
    fn test_stream_error_provider() {
        let err = StreamError::provider("openai", "Model not found");
        assert_eq!(err.category, StreamErrorCategory::Provider);
        assert_eq!(err.provider, Some("openai".to_string()));
    }

    #[test]
    fn test_stream_error_cancelled() {
        let err = StreamError::cancelled();
        assert_eq!(err.category, StreamErrorCategory::Cancelled);
        assert!(!err.retryable);
    }

    #[test]
    fn test_stream_error_with_context() {
        let err = StreamError::network("Failed")
            .with_provider("anthropic")
            .with_http_status(500)
            .with_context("During streaming response");

        assert_eq!(err.provider, Some("anthropic".to_string()));
        assert_eq!(err.http_status, Some(500));
        assert_eq!(err.context, Some("During streaming response".to_string()));
    }

    #[test]
    fn test_stream_error_from_http_response() {
        let err = StreamError::from_http_response(
            "openai",
            401,
            r#"{"error":{"message":"Invalid API key"}}"#,
        );
        assert_eq!(err.category, StreamErrorCategory::Auth);
        assert_eq!(err.http_status, Some(401));
        assert!(err.message.contains("Invalid API key"));
    }

    #[test]
    fn test_stream_error_from_http_response_rate_limit() {
        let err = StreamError::from_http_response("anthropic", 429, "Rate limit exceeded");
        assert_eq!(err.category, StreamErrorCategory::RateLimit);
        assert!(err.retryable);
    }

    #[test]
    fn test_stream_error_display() {
        let err = StreamError::network("Connection reset");
        let display = format!("{}", err);
        assert!(display.contains("NETWORK_ERROR"));
        assert!(display.contains("Connection reset"));
    }
}
