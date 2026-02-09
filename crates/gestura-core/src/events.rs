//! Typed event system for real-time UI updates
//!
//! This module provides a structured event system for communicating state changes
//! from the agent pipeline to the frontend. Based on Block Goose architecture patterns.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Event types for the agent pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum AgentEvent {
    /// Pipeline has started processing a request
    PipelineStarted {
        request_id: String,
        timestamp_ms: u64,
    },
    /// Progress update for long-running operations
    Progress {
        request_id: String,
        stage: ProgressStage,
        percent: Option<u8>,
        message: String,
    },
    /// Token streaming event
    TokenStream {
        request_id: String,
        content: String,
        is_thinking: bool,
    },
    /// Tool execution started
    ToolStarted {
        request_id: String,
        tool_id: String,
        tool_name: String,
    },
    /// Tool execution progress
    ToolProgress {
        request_id: String,
        tool_id: String,
        percent: Option<u8>,
        message: String,
    },
    /// Tool execution completed
    ToolCompleted {
        request_id: String,
        tool_id: String,
        success: bool,
        duration_ms: u64,
        output_preview: String,
    },
    /// Context was compacted
    ContextCompacted {
        request_id: String,
        messages_before: usize,
        messages_after: usize,
        tokens_saved: usize,
    },
    /// Retry attempt
    RetryAttempt {
        request_id: String,
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        reason: String,
    },
    /// Pipeline completed successfully
    PipelineCompleted {
        request_id: String,
        duration_ms: u64,
        tokens_used: Option<u64>,
    },
    /// Pipeline failed with error
    PipelineFailed {
        request_id: String,
        error: String,
        recoverable: bool,
    },
    /// Pipeline was cancelled
    PipelineCancelled { request_id: String },
    /// Pipeline was paused (cancelled with resume intent).
    PipelinePaused { request_id: String },
}

/// Stages of pipeline progress
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressStage {
    /// Analyzing the request
    Analyzing,
    /// Resolving context
    ResolvingContext,
    /// Waiting for LLM response
    WaitingForLlm,
    /// Executing tools
    ExecutingTools,
    /// Generating response
    GeneratingResponse,
    /// Finalizing
    Finalizing,
}

impl ProgressStage {
    /// Get a human-readable description of the stage
    pub fn description(&self) -> &'static str {
        match self {
            Self::Analyzing => "Analyzing request...",
            Self::ResolvingContext => "Resolving context...",
            Self::WaitingForLlm => "Waiting for AI response...",
            Self::ExecutingTools => "Executing tools...",
            Self::GeneratingResponse => "Generating response...",
            Self::Finalizing => "Finalizing...",
        }
    }
}

/// Configuration for event buffering
#[derive(Debug, Clone)]
pub struct EventBufferConfig {
    /// Minimum interval between events of the same type
    pub min_interval: Duration,
    /// Maximum events to buffer before forcing a flush
    pub max_buffer_size: usize,
    /// Whether to coalesce similar events
    pub coalesce_similar: bool,
}

impl Default for EventBufferConfig {
    fn default() -> Self {
        Self {
            min_interval: Duration::from_millis(50),
            max_buffer_size: 100,
            coalesce_similar: true,
        }
    }
}

/// Event emitter with optional buffering for rate limiting
pub struct EventEmitter {
    tx: mpsc::Sender<AgentEvent>,
    config: EventBufferConfig,
    last_progress_emit: Option<Instant>,
    last_token_emit: Option<Instant>,
    token_buffer: String,
}

impl EventEmitter {
    /// Create a new event emitter with default configuration
    pub fn new(tx: mpsc::Sender<AgentEvent>) -> Self {
        Self::with_config(tx, EventBufferConfig::default())
    }

    /// Create a new event emitter with custom configuration
    pub fn with_config(tx: mpsc::Sender<AgentEvent>, config: EventBufferConfig) -> Self {
        Self {
            tx,
            config,
            last_progress_emit: None,
            last_token_emit: None,
            token_buffer: String::new(),
        }
    }

    /// Emit an event immediately (bypasses buffering)
    pub async fn emit(&self, event: AgentEvent) -> Result<(), mpsc::error::SendError<AgentEvent>> {
        self.tx.send(event).await
    }

    /// Emit a progress event with rate limiting
    pub async fn emit_progress(
        &mut self,
        request_id: String,
        stage: ProgressStage,
        percent: Option<u8>,
        message: String,
    ) {
        let now = Instant::now();
        let should_emit = self
            .last_progress_emit
            .map(|last| now.duration_since(last) >= self.config.min_interval)
            .unwrap_or(true);

        if should_emit {
            self.last_progress_emit = Some(now);
            let _ = self
                .tx
                .send(AgentEvent::Progress {
                    request_id,
                    stage,
                    percent,
                    message,
                })
                .await;
        }
    }

    /// Buffer token content and emit when threshold is reached
    pub async fn buffer_token(&mut self, request_id: &str, content: &str, is_thinking: bool) {
        self.token_buffer.push_str(content);

        let now = Instant::now();
        let should_flush = self
            .last_token_emit
            .map(|last| now.duration_since(last) >= self.config.min_interval)
            .unwrap_or(true)
            || self.token_buffer.len() >= self.config.max_buffer_size;

        if should_flush && !self.token_buffer.is_empty() {
            self.last_token_emit = Some(now);
            let content = std::mem::take(&mut self.token_buffer);
            let _ = self
                .tx
                .send(AgentEvent::TokenStream {
                    request_id: request_id.to_string(),
                    content,
                    is_thinking,
                })
                .await;
        }
    }

    /// Flush any buffered tokens
    pub async fn flush_tokens(&mut self, request_id: &str, is_thinking: bool) {
        if !self.token_buffer.is_empty() {
            let content = std::mem::take(&mut self.token_buffer);
            let _ = self
                .tx
                .send(AgentEvent::TokenStream {
                    request_id: request_id.to_string(),
                    content,
                    is_thinking,
                })
                .await;
        }
    }

    /// Emit pipeline started event
    pub async fn pipeline_started(&self, request_id: &str) {
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self
            .tx
            .send(AgentEvent::PipelineStarted {
                request_id: request_id.to_string(),
                timestamp_ms,
            })
            .await;
    }

    /// Emit pipeline completed event
    pub async fn pipeline_completed(
        &self,
        request_id: &str,
        duration_ms: u64,
        tokens_used: Option<u64>,
    ) {
        let _ = self
            .tx
            .send(AgentEvent::PipelineCompleted {
                request_id: request_id.to_string(),
                duration_ms,
                tokens_used,
            })
            .await;
    }

    /// Emit pipeline failed event
    pub async fn pipeline_failed(&self, request_id: &str, error: &str, recoverable: bool) {
        let _ = self
            .tx
            .send(AgentEvent::PipelineFailed {
                request_id: request_id.to_string(),
                error: error.to_string(),
                recoverable,
            })
            .await;
    }

    /// Emit tool started event
    pub async fn tool_started(&self, request_id: &str, tool_id: &str, tool_name: &str) {
        let _ = self
            .tx
            .send(AgentEvent::ToolStarted {
                request_id: request_id.to_string(),
                tool_id: tool_id.to_string(),
                tool_name: tool_name.to_string(),
            })
            .await;
    }

    /// Emit tool completed event
    pub async fn tool_completed(
        &self,
        request_id: &str,
        tool_id: &str,
        success: bool,
        duration_ms: u64,
        output_preview: &str,
    ) {
        let _ = self
            .tx
            .send(AgentEvent::ToolCompleted {
                request_id: request_id.to_string(),
                tool_id: tool_id.to_string(),
                success,
                duration_ms,
                output_preview: output_preview.to_string(),
            })
            .await;
    }
}

/// Progress tracker for long-running operations
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    request_id: String,
    current_stage: ProgressStage,
    stage_start: Instant,
    total_start: Instant,
}

impl ProgressTracker {
    /// Create a new progress tracker
    pub fn new(request_id: String) -> Self {
        let now = Instant::now();
        Self {
            request_id,
            current_stage: ProgressStage::Analyzing,
            stage_start: now,
            total_start: now,
        }
    }

    /// Get the current request ID
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Get the current stage
    pub fn current_stage(&self) -> ProgressStage {
        self.current_stage
    }

    /// Advance to the next stage
    pub fn advance_to(&mut self, stage: ProgressStage) {
        self.current_stage = stage;
        self.stage_start = Instant::now();
    }

    /// Get duration of current stage
    pub fn stage_duration(&self) -> Duration {
        self.stage_start.elapsed()
    }

    /// Get total duration since start
    pub fn total_duration(&self) -> Duration {
        self.total_start.elapsed()
    }

    /// Get total duration in milliseconds
    pub fn total_duration_ms(&self) -> u64 {
        self.total_start.elapsed().as_millis() as u64
    }
}

/// Shared event channel for broadcasting events to multiple listeners
pub type EventReceiver = mpsc::Receiver<AgentEvent>;
pub type EventSender = mpsc::Sender<AgentEvent>;

/// Create a new event channel pair
pub fn create_event_channel(buffer_size: usize) -> (EventSender, EventReceiver) {
    mpsc::channel(buffer_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_stage_description() {
        assert_eq!(
            ProgressStage::Analyzing.description(),
            "Analyzing request..."
        );
        assert_eq!(
            ProgressStage::ExecutingTools.description(),
            "Executing tools..."
        );
    }

    #[test]
    fn test_progress_tracker() {
        let mut tracker = ProgressTracker::new("test-123".to_string());
        assert_eq!(tracker.current_stage(), ProgressStage::Analyzing);
        assert_eq!(tracker.request_id(), "test-123");

        tracker.advance_to(ProgressStage::WaitingForLlm);
        assert_eq!(tracker.current_stage(), ProgressStage::WaitingForLlm);
    }

    #[test]
    fn test_event_buffer_config_default() {
        let config = EventBufferConfig::default();
        assert_eq!(config.min_interval, Duration::from_millis(50));
        assert_eq!(config.max_buffer_size, 100);
        assert!(config.coalesce_similar);
    }

    #[tokio::test]
    async fn test_event_emitter_emit() {
        let (tx, mut rx) = create_event_channel(10);
        let emitter = EventEmitter::new(tx);

        emitter.pipeline_started("req-1").await;

        let event = rx.recv().await.unwrap();
        match event {
            AgentEvent::PipelineStarted { request_id, .. } => {
                assert_eq!(request_id, "req-1");
            }
            _ => panic!("Expected PipelineStarted event"),
        }
    }
}
