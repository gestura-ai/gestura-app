//! Retry management for transient failures
//!
//! Provides configurable retry policies with exponential backoff and jitter,
//! error classification, and user notification support.
//!
//! Based on patterns from Block Goose's RetryManager architecture.

use gestura_core_foundation::error::AppError;
use rand::Rng as _;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Error classification for retry decisions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Transient error that may succeed on retry (rate limits, timeouts, network issues)
    Transient,
    /// Permanent error that will not succeed on retry (auth failure, invalid input)
    Permanent,
    /// Context overflow error that requires compaction before retry
    ContextOverflow,
    /// Unknown error classification - treat as transient with limited retries
    Unknown,
}

impl ErrorClass {
    /// Classify an AppError for retry decisions
    pub fn classify(error: &AppError) -> Self {
        match error {
            // Transient errors - worth retrying
            AppError::Timeout(_) => Self::Transient,
            AppError::Http(e) => {
                if e.is_timeout() || e.is_connect() {
                    Self::Transient
                } else if let Some(status) = e.status() {
                    match status.as_u16() {
                        429 => Self::Transient,       // Rate limit
                        500..=599 => Self::Transient, // Server errors
                        401 | 403 => Self::Permanent, // Auth errors
                        400 | 404 => Self::Permanent, // Client errors
                        _ => Self::Unknown,
                    }
                } else {
                    Self::Unknown
                }
            }
            AppError::Llm(msg) => {
                let msg_lower = msg.to_lowercase();
                // Context overflow errors - need compaction, not blind retry
                if msg_lower.contains("context_length_exceeded")
                    || msg_lower.contains("context length")
                    || msg_lower.contains("maximum context")
                    || msg_lower.contains("token limit")
                    || (msg_lower.contains("tokens") && msg_lower.contains("exceeds"))
                {
                    Self::ContextOverflow
                } else if msg_lower.contains("rate limit")
                    || msg_lower.contains("429")
                    || msg_lower.contains("timeout")
                    || msg_lower.contains("connection")
                    || msg_lower.contains("temporarily")
                {
                    Self::Transient
                } else if msg_lower.contains("401")
                    || msg_lower.contains("403")
                    || msg_lower.contains("unauthorized")
                    || msg_lower.contains("invalid api key")
                    || msg_lower.contains("not configured")
                {
                    Self::Permanent
                } else {
                    Self::Unknown
                }
            }
            // Context overflow - needs compaction, not retry
            AppError::ContextOverflow(_) => Self::ContextOverflow,
            // Permanent errors - don't retry
            AppError::Config(_) => Self::Permanent,
            AppError::PermissionDenied(_) => Self::Permanent,
            AppError::InvalidInput(_) => Self::Permanent,
            AppError::NotFound(_) => Self::Permanent,
            // Unknown - treat conservatively
            _ => Self::Unknown,
        }
    }

    /// Whether this error class should be retried with standard backoff
    pub fn should_retry(&self) -> bool {
        matches!(self, Self::Transient | Self::Unknown)
    }

    /// Whether this error requires context compaction before retry
    pub fn needs_compaction(&self) -> bool {
        matches!(self, Self::ContextOverflow)
    }

    /// Whether this error is recoverable (either by retry or compaction)
    pub fn is_recoverable(&self) -> bool {
        !matches!(self, Self::Permanent)
    }
}

/// Retry policy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (0 = no retries)
    pub max_attempts: u32,
    /// Initial delay before first retry (milliseconds)
    pub initial_delay_ms: u64,
    /// Maximum delay between retries (milliseconds)
    pub max_delay_ms: u64,
    /// Multiplier for exponential backoff (e.g., 2.0 = double each time)
    pub backoff_multiplier: f64,
    /// Jitter factor (0.0-1.0) to add randomness to delays
    pub jitter_factor: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 30000,
            backoff_multiplier: 2.0,
            jitter_factor: 0.25,
        }
    }
}

impl RetryPolicy {
    /// Create a policy for API calls (moderate retries)
    pub fn for_api() -> Self {
        Self::default()
    }

    /// Create a policy for tool execution (fewer retries)
    pub fn for_tools() -> Self {
        Self {
            max_attempts: 2,
            initial_delay_ms: 500,
            max_delay_ms: 5000,
            backoff_multiplier: 2.0,
            jitter_factor: 0.1,
        }
    }

    /// Create a policy for streaming (quick retries)
    pub fn for_streaming() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 8000,
            backoff_multiplier: 2.0,
            jitter_factor: 0.25,
        }
    }

    /// Calculate delay for a given attempt number (0-indexed)
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        let base_delay =
            self.initial_delay_ms as f64 * self.backoff_multiplier.powi((attempt - 1) as i32);
        let capped_delay = base_delay.min(self.max_delay_ms as f64);

        // Add jitter (only if jitter_factor > 0)
        let jitter_range = capped_delay * self.jitter_factor;
        let jitter = if jitter_range > 0.0 {
            rand::thread_rng().gen_range(-jitter_range..jitter_range)
        } else {
            0.0
        };
        let final_delay = (capped_delay + jitter).max(0.0);

        Duration::from_millis(final_delay as u64)
    }
}

/// Retry event for notification callbacks
#[derive(Debug, Clone)]
pub struct RetryEvent {
    /// Current attempt number (1-indexed)
    pub attempt: u32,
    /// Maximum attempts configured
    pub max_attempts: u32,
    /// Delay before next retry
    pub delay: Duration,
    /// Error that triggered the retry
    pub error_message: String,
    /// Error classification
    pub error_class: ErrorClass,
}

/// Callback type for retry notifications
pub type RetryCallback = Box<dyn Fn(RetryEvent) + Send + Sync>;

/// Retry manager for executing operations with automatic retry
pub struct RetryManager {
    policy: RetryPolicy,
    on_retry: Option<RetryCallback>,
}

impl RetryManager {
    /// Create a new retry manager with the given policy
    pub fn new(policy: RetryPolicy) -> Self {
        Self {
            policy,
            on_retry: None,
        }
    }

    /// Create a retry manager with default API policy
    pub fn for_api() -> Self {
        Self::new(RetryPolicy::for_api())
    }

    /// Create a retry manager for streaming operations
    pub fn for_streaming() -> Self {
        Self::new(RetryPolicy::for_streaming())
    }

    /// Create a retry manager for tool execution
    pub fn for_tools() -> Self {
        Self::new(RetryPolicy::for_tools())
    }

    /// Set a callback to be notified on retry attempts
    pub fn with_retry_callback(mut self, callback: RetryCallback) -> Self {
        self.on_retry = Some(callback);
        self
    }

    /// Execute an async operation with retry logic
    ///
    /// Returns the result of the operation, or the last error if all retries fail.
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> Result<T, AppError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, AppError>>,
    {
        let mut last_error: Option<AppError> = None;

        for attempt in 0..=self.policy.max_attempts {
            // Wait before retry (skip for first attempt)
            if attempt > 0 {
                let delay = self.policy.delay_for_attempt(attempt);
                tokio::time::sleep(delay).await;
            }

            match operation().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let error_class = ErrorClass::classify(&e);

                    // Don't retry permanent errors
                    if !error_class.should_retry() {
                        return Err(e);
                    }

                    // Check if we have more attempts
                    if attempt < self.policy.max_attempts {
                        let delay = self.policy.delay_for_attempt(attempt + 1);

                        // Notify callback if set
                        if let Some(ref callback) = self.on_retry {
                            callback(RetryEvent {
                                attempt: attempt + 1,
                                max_attempts: self.policy.max_attempts,
                                delay,
                                error_message: e.to_string(),
                                error_class,
                            });
                        }

                        tracing::warn!(
                            attempt = attempt + 1,
                            max_attempts = self.policy.max_attempts,
                            delay_ms = delay.as_millis(),
                            error = %e,
                            error_class = ?error_class,
                            "Operation failed, will retry"
                        );
                    }

                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| AppError::Internal("Retry exhausted".to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_classification_transient() {
        let timeout_err = AppError::Timeout("connection timed out".to_string());
        assert_eq!(ErrorClass::classify(&timeout_err), ErrorClass::Transient);

        let rate_limit_err = AppError::Llm("rate limit exceeded (429)".to_string());
        assert_eq!(ErrorClass::classify(&rate_limit_err), ErrorClass::Transient);
    }

    #[test]
    fn test_error_classification_permanent() {
        let config_err = AppError::Config("missing API key".to_string());
        assert_eq!(ErrorClass::classify(&config_err), ErrorClass::Permanent);

        let auth_err = AppError::Llm("401 unauthorized".to_string());
        assert_eq!(ErrorClass::classify(&auth_err), ErrorClass::Permanent);
    }

    #[test]
    fn test_error_classification_context_overflow() {
        // From error message
        let overflow_err = AppError::Llm(
            "maximum context length is 16385 tokens".to_string()
        );
        assert_eq!(ErrorClass::classify(&overflow_err), ErrorClass::ContextOverflow);

        // From explicit variant
        let explicit_err = AppError::ContextOverflow("context too large".to_string());
        assert_eq!(ErrorClass::classify(&explicit_err), ErrorClass::ContextOverflow);

        // From different message format
        let token_err = AppError::Llm(
            "Request tokens exceeds limit".to_string()
        );
        assert_eq!(ErrorClass::classify(&token_err), ErrorClass::ContextOverflow);
    }

    #[test]
    fn test_context_overflow_needs_compaction() {
        assert!(ErrorClass::ContextOverflow.needs_compaction());
        assert!(!ErrorClass::Transient.needs_compaction());
        assert!(!ErrorClass::Permanent.needs_compaction());
    }

    #[test]
    fn test_delay_calculation() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 10000,
            backoff_multiplier: 2.0,
            jitter_factor: 0.0, // No jitter for predictable testing
        };

        assert_eq!(policy.delay_for_attempt(0), Duration::ZERO);
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(1000));
        assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(2000));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(4000));
        // Should cap at max_delay_ms
        assert_eq!(policy.delay_for_attempt(5), Duration::from_millis(10000));
    }

    #[test]
    fn test_should_retry() {
        assert!(ErrorClass::Transient.should_retry());
        assert!(ErrorClass::Unknown.should_retry());
        assert!(!ErrorClass::Permanent.should_retry());
    }

    #[tokio::test]
    async fn test_retry_manager_success_first_try() {
        let manager = RetryManager::new(RetryPolicy::default());
        let result: Result<i32, AppError> = manager.execute(|| async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_manager_permanent_error_no_retry() {
        let manager = RetryManager::new(RetryPolicy::default());
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let call_count_clone = call_count.clone();

        let result: Result<i32, AppError> = manager
            .execute(|| {
                let count = call_count_clone.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err(AppError::Config("permanent error".to_string()))
                }
            })
            .await;

        assert!(result.is_err());
        // Should only be called once (no retries for permanent errors)
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
