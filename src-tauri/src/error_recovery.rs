//! Error recovery mechanisms for Gestura.app
//! Provides automatic recovery from various failure scenarios

use crate::AppError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

/// Recovery strategy for different types of errors
#[derive(Debug, Clone)]
pub enum RecoveryStrategy {
    /// Retry the operation with exponential backoff
    Retry { max_attempts: u32, base_delay: Duration },
    /// Restart the component
    Restart,
    /// Fallback to alternative implementation
    Fallback,
    /// Ignore the error and continue
    Ignore,
    /// Escalate to user intervention
    Escalate,
}

/// Recovery action result
#[derive(Debug)]
pub enum RecoveryResult {
    Success,
    Failed(AppError),
    RequiresUserIntervention(String),
}

/// Error recovery manager
pub struct ErrorRecoveryManager {
    strategies: Arc<RwLock<HashMap<String, RecoveryStrategy>>>,
    recovery_history: Arc<Mutex<Vec<RecoveryAttempt>>>,
    max_history: usize,
}

/// Record of a recovery attempt
#[derive(Debug, Clone)]
pub struct RecoveryAttempt {
    pub error_type: String,
    pub strategy: RecoveryStrategy,
    pub timestamp: Instant,
    pub success: bool,
    pub details: String,
}

impl ErrorRecoveryManager {
    /// Create a new error recovery manager
    pub fn new(max_history: usize) -> Self {
        let mut strategies = HashMap::new();
        
        // Default recovery strategies
        strategies.insert("nats_connection".to_string(), RecoveryStrategy::Retry { 
            max_attempts: 3, 
            base_delay: Duration::from_secs(1) 
        });
        strategies.insert("ble_connection".to_string(), RecoveryStrategy::Retry { 
            max_attempts: 5, 
            base_delay: Duration::from_millis(500) 
        });
        strategies.insert("agent_crash".to_string(), RecoveryStrategy::Restart);
        strategies.insert("voice_engine_failure".to_string(), RecoveryStrategy::Fallback);
        strategies.insert("config_corruption".to_string(), RecoveryStrategy::Escalate);
        
        Self {
            strategies: Arc::new(RwLock::new(strategies)),
            recovery_history: Arc::new(Mutex::new(Vec::new())),
            max_history,
        }
    }

    /// Register a recovery strategy for an error type
    pub async fn register_strategy(&self, error_type: String, strategy: RecoveryStrategy) {
        let mut strategies = self.strategies.write().await;
        strategies.insert(error_type, strategy);
    }

    /// Attempt to recover from an error
    pub async fn recover(&self, error_type: &str, error: &AppError) -> RecoveryResult {
        let strategy = {
            let strategies = self.strategies.read().await;
            strategies.get(error_type).cloned().unwrap_or(RecoveryStrategy::Escalate)
        };

        let start_time = Instant::now();
        let result = self.execute_recovery(&strategy, error_type, error).await;
        
        // Record the attempt
        let attempt = RecoveryAttempt {
            error_type: error_type.to_string(),
            strategy: strategy.clone(),
            timestamp: start_time,
            success: matches!(result, RecoveryResult::Success),
            details: format!("{:?}", error),
        };

        let mut history = self.recovery_history.lock().await;
        history.push(attempt);
        
        // Trim history if needed
        if history.len() > self.max_history {
            history.remove(0);
        }

        tracing::info!("Recovery attempt for {}: {:?}", error_type, result);
        result
    }

    /// Execute a specific recovery strategy
    async fn execute_recovery(&self, strategy: &RecoveryStrategy, error_type: &str, error: &AppError) -> RecoveryResult {
        match strategy {
            RecoveryStrategy::Retry { max_attempts, base_delay } => {
                self.retry_recovery(*max_attempts, *base_delay, error_type).await
            }
            RecoveryStrategy::Restart => {
                self.restart_recovery(error_type).await
            }
            RecoveryStrategy::Fallback => {
                self.fallback_recovery(error_type).await
            }
            RecoveryStrategy::Ignore => {
                tracing::warn!("Ignoring error for {}: {:?}", error_type, error);
                RecoveryResult::Success
            }
            RecoveryStrategy::Escalate => {
                RecoveryResult::RequiresUserIntervention(
                    format!("Manual intervention required for {}: {:?}", error_type, error)
                )
            }
        }
    }

    /// Implement retry recovery with exponential backoff
    async fn retry_recovery(&self, max_attempts: u32, base_delay: Duration, error_type: &str) -> RecoveryResult {
        for attempt in 1..=max_attempts {
            let delay = base_delay * 2_u32.pow(attempt - 1);
            tracing::info!("Retry attempt {} for {} (delay: {:?})", attempt, error_type, delay);
            
            tokio::time::sleep(delay).await;
            
            // Attempt recovery based on error type
            let success = match error_type {
                "nats_connection" => self.recover_nats_connection().await,
                "ble_connection" => self.recover_ble_connection().await,
                "voice_engine_failure" => self.recover_voice_engine().await,
                _ => false,
            };
            
            if success {
                return RecoveryResult::Success;
            }
        }
        
        RecoveryResult::Failed(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("Recovery failed after {} attempts", max_attempts)
        )))
    }

    /// Implement restart recovery
    async fn restart_recovery(&self, error_type: &str) -> RecoveryResult {
        tracing::info!("Attempting restart recovery for {}", error_type);
        
        match error_type {
            "agent_crash" => {
                // In a real implementation, this would restart the crashed agent
                tracing::info!("Restarting crashed agent");
                RecoveryResult::Success
            }
            _ => RecoveryResult::Failed(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("Restart not supported for {}", error_type)
            )))
        }
    }

    /// Implement fallback recovery
    async fn fallback_recovery(&self, error_type: &str) -> RecoveryResult {
        tracing::info!("Attempting fallback recovery for {}", error_type);
        
        match error_type {
            "voice_engine_failure" => {
                // Fallback to mock voice engine
                tracing::info!("Falling back to mock voice engine");
                RecoveryResult::Success
            }
            "nats_connection" => {
                // Fallback to memory bus
                tracing::info!("Falling back to memory bus");
                RecoveryResult::Success
            }
            _ => RecoveryResult::Failed(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("Fallback not available for {}", error_type)
            )))
        }
    }

    /// Attempt to recover NATS connection
    async fn recover_nats_connection(&self) -> bool {
        #[cfg(feature = "nats")]
        {
            match crate::nats_mq::connect_nats("nats://127.0.0.1:4222").await {
                Ok(_) => {
                    tracing::info!("NATS connection recovered");
                    true
                }
                Err(_) => false,
            }
        }
        #[cfg(not(feature = "nats"))]
        {
            // Mock recovery for testing
            true
        }
    }

    /// Attempt to recover BLE connection
    async fn recover_ble_connection(&self) -> bool {
        // In a real implementation, this would attempt to reconnect to BLE devices
        tracing::info!("Attempting BLE connection recovery");
        true // Mock success
    }

    /// Attempt to recover voice engine
    async fn recover_voice_engine(&self) -> bool {
        // In a real implementation, this would restart the voice engine
        tracing::info!("Attempting voice engine recovery");
        true // Mock success
    }

    /// Get recovery statistics
    pub async fn get_recovery_stats(&self) -> RecoveryStats {
        let history = self.recovery_history.lock().await;
        let total_attempts = history.len();
        let successful_attempts = history.iter().filter(|a| a.success).count();
        
        let mut error_type_counts = HashMap::new();
        for attempt in history.iter() {
            *error_type_counts.entry(attempt.error_type.clone()).or_insert(0) += 1;
        }

        RecoveryStats {
            total_attempts,
            successful_attempts,
            success_rate: if total_attempts > 0 { 
                successful_attempts as f64 / total_attempts as f64 
            } else { 
                0.0 
            },
            error_type_counts,
        }
    }

    /// Clear recovery history
    pub async fn clear_history(&self) {
        let mut history = self.recovery_history.lock().await;
        history.clear();
        tracing::info!("Recovery history cleared");
    }
}

/// Recovery statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecoveryStats {
    pub total_attempts: usize,
    pub successful_attempts: usize,
    pub success_rate: f64,
    pub error_type_counts: HashMap<String, usize>,
}

/// Global error recovery instance
static RECOVERY_MANAGER: tokio::sync::OnceCell<ErrorRecoveryManager> = tokio::sync::OnceCell::const_new();

/// Get the global recovery manager
pub async fn get_recovery_manager() -> &'static ErrorRecoveryManager {
    RECOVERY_MANAGER.get_or_init(|| async {
        ErrorRecoveryManager::new(1000)
    }).await
}

/// Convenience function to attempt error recovery
pub async fn attempt_recovery(error_type: &str, error: &AppError) -> RecoveryResult {
    let manager = get_recovery_manager().await;
    manager.recover(error_type, error).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_recovery_manager() {
        let manager = ErrorRecoveryManager::new(10);
        
        // Test retry strategy
        let error = AppError::Io(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "test"));
        let result = manager.recover("nats_connection", &error).await;
        assert!(matches!(result, RecoveryResult::Success));
        
        // Check stats
        let stats = manager.get_recovery_stats().await;
        assert_eq!(stats.total_attempts, 1);
    }

    #[tokio::test]
    async fn test_recovery_strategies() {
        let manager = ErrorRecoveryManager::new(10);
        
        // Test different strategies
        let error = AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, "test"));
        
        let result = manager.recover("agent_crash", &error).await;
        assert!(matches!(result, RecoveryResult::Success));
        
        let result = manager.recover("voice_engine_failure", &error).await;
        assert!(matches!(result, RecoveryResult::Success));
        
        let result = manager.recover("config_corruption", &error).await;
        assert!(matches!(result, RecoveryResult::RequiresUserIntervention(_)));
    }
}
