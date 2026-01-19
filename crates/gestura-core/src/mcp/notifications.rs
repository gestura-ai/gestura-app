//! MCP Notifications - Progress, logging, and cancellation
//! Provides notification handling for long-running operations.

use super::types::{
    CancelledNotification, LogLevel, LoggingMessage, ProgressNotification, ProgressToken,
};
use std::collections::HashMap;
use std::sync::RwLock;
use tokio::sync::broadcast;

/// Notification sender for MCP notifications
pub type NotificationSender = broadcast::Sender<McpNotification>;
/// Notification receiver for MCP notifications
pub type NotificationReceiver = broadcast::Receiver<McpNotification>;

/// MCP notification types
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpNotification {
    /// Progress update
    Progress(ProgressNotification),
    /// Log message
    Log(LoggingMessage),
    /// Request cancelled
    Cancelled(CancelledNotification),
    /// Tools list changed
    ToolsListChanged,
    /// Resources list changed
    ResourcesListChanged,
    /// Prompts list changed
    PromptsListChanged,
}

/// Progress tracker for long-running operations
#[derive(Debug)]
pub struct ProgressTracker {
    active_operations: RwLock<HashMap<String, OperationProgress>>,
    sender: NotificationSender,
}

/// Progress state for an operation
#[derive(Debug, Clone)]
pub struct OperationProgress {
    pub token: ProgressToken,
    pub current: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
    pub cancelled: bool,
}

impl ProgressTracker {
    /// Create a new progress tracker
    pub fn new(sender: NotificationSender) -> Self {
        Self {
            active_operations: RwLock::new(HashMap::new()),
            sender,
        }
    }

    /// Start tracking a new operation
    pub fn start_operation(&self, token: impl Into<ProgressToken>, total: Option<f64>) -> String {
        let token = token.into();
        let id = match &token {
            ProgressToken::String(s) => s.clone(),
            ProgressToken::Integer(i) => i.to_string(),
        };

        let progress = OperationProgress {
            token: token.clone(),
            current: 0.0,
            total,
            message: None,
            cancelled: false,
        };

        if let Ok(mut ops) = self.active_operations.write() {
            ops.insert(id.clone(), progress);
        }

        id
    }

    /// Update progress for an operation
    pub fn update_progress(&self, id: &str, current: f64, message: Option<String>) {
        let notification = {
            let mut ops = match self.active_operations.write() {
                Ok(ops) => ops,
                Err(_) => return,
            };

            if let Some(op) = ops.get_mut(id) {
                op.current = current;
                op.message = message.clone();

                Some(ProgressNotification {
                    progress_token: op.token.clone(),
                    progress: current,
                    total: op.total,
                    message,
                })
            } else {
                None
            }
        };

        if let Some(notif) = notification {
            let _ = self.sender.send(McpNotification::Progress(notif));
        }
    }

    /// Complete an operation
    pub fn complete_operation(&self, id: &str) {
        if let Ok(mut ops) = self.active_operations.write()
            && let Some(op) = ops.remove(id)
        {
            let _ = self
                .sender
                .send(McpNotification::Progress(ProgressNotification {
                    progress_token: op.token,
                    progress: op.total.unwrap_or(100.0),
                    total: op.total,
                    message: Some("Complete".to_string()),
                }));
        }
    }

    /// Cancel an operation
    pub fn cancel_operation(&self, id: &str, reason: Option<String>) -> bool {
        if let Ok(mut ops) = self.active_operations.write()
            && let Some(op) = ops.get_mut(id)
        {
            op.cancelled = true;
            let _ = self
                .sender
                .send(McpNotification::Cancelled(CancelledNotification {
                    request_id: serde_json::json!(id),
                    reason,
                }));
            return true;
        }

        false
    }

    /// Check if an operation is cancelled
    pub fn is_cancelled(&self, id: &str) -> bool {
        if let Ok(ops) = self.active_operations.read()
            && let Some(op) = ops.get(id)
        {
            return op.cancelled;
        }

        false
    }
}

/// Logger for MCP logging notifications
#[derive(Debug, Clone)]
pub struct McpLogger {
    sender: NotificationSender,
    logger_name: Option<String>,
}

impl McpLogger {
    /// Create a new MCP logger
    pub fn new(sender: NotificationSender, logger_name: Option<String>) -> Self {
        Self {
            sender,
            logger_name,
        }
    }

    /// Log a message at the specified level
    pub fn log(&self, level: LogLevel, data: impl Into<serde_json::Value>) {
        let _ = self.sender.send(McpNotification::Log(LoggingMessage {
            level,
            logger: self.logger_name.clone(),
            data: data.into(),
        }));
    }

    /// Log a debug message
    pub fn debug(&self, message: impl Into<String>) {
        self.log(LogLevel::Debug, serde_json::json!(message.into()));
    }

    /// Log an info message
    pub fn info(&self, message: impl Into<String>) {
        self.log(LogLevel::Info, serde_json::json!(message.into()));
    }

    /// Log a warning message
    pub fn warning(&self, message: impl Into<String>) {
        self.log(LogLevel::Warning, serde_json::json!(message.into()));
    }

    /// Log an error message
    pub fn error(&self, message: impl Into<String>) {
        self.log(LogLevel::Error, serde_json::json!(message.into()));
    }
}

/// Create a notification channel
pub fn create_notification_channel() -> (NotificationSender, NotificationReceiver) {
    broadcast::channel(100)
}
