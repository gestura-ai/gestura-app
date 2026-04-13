//! Streaming LLM provider support for Gestura
//!
//! This module provides streaming capabilities for LLM responses, enabling
//! real-time token-by-token delivery to the frontend with cancellation support.

use crate::config::StreamingConfig;
use futures_util::StreamExt;
use gestura_core_foundation::AppError;
use gestura_core_llm::TokenUsage;
use gestura_core_llm::openai::{
    OpenAiApi, is_openai_model_incompatible_with_agent_session, openai_agent_session_model_message,
    openai_api_for_model,
};
use gestura_core_retry::RetryPolicy;
use gestura_core_tools::schemas::ProviderToolSchemas;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::Instrument as _;

/// Default timeout for streaming LLM API calls
const STREAMING_TIMEOUT_SECS: u64 = 300;
const STREAM_CHUNK_BUFFER_CAPACITY: usize = 256;
const STATUS_CHUNK_SEND_TIMEOUT: Duration = Duration::from_millis(100);
const TOKEN_USAGE_CHUNK_SEND_TIMEOUT: Duration = Duration::from_millis(100);

async fn send_status_chunk_best_effort(tx: &mpsc::Sender<StreamChunk>, chunk: StreamChunk) {
    debug_assert!(matches!(chunk, StreamChunk::Status { .. }));

    match tokio::time::timeout(STATUS_CHUNK_SEND_TIMEOUT, tx.send(chunk)).await {
        Ok(Ok(())) | Ok(Err(_)) => {}
        Err(_) => {
            tracing::debug!(
                timeout_ms = STATUS_CHUNK_SEND_TIMEOUT.as_millis(),
                "Dropping transient status chunk because the stream receiver is not draining fast enough"
            );
        }
    }
}

async fn send_token_usage_chunk_best_effort(tx: &mpsc::Sender<StreamChunk>, chunk: StreamChunk) {
    debug_assert!(matches!(chunk, StreamChunk::TokenUsageUpdate { .. }));

    match tokio::time::timeout(TOKEN_USAGE_CHUNK_SEND_TIMEOUT, tx.send(chunk)).await {
        Ok(Ok(())) | Ok(Err(_)) => {}
        Err(_) => {
            tracing::debug!(
                timeout_ms = TOKEN_USAGE_CHUNK_SEND_TIMEOUT.as_millis(),
                "Dropping transient token-usage chunk because the stream receiver is not draining fast enough"
            );
        }
    }
}

/// Pricing per 1M tokens (input/output) for various providers
/// Prices are in USD and updated as of January 2026
pub mod pricing {
    /// OpenAI GPT-4 Turbo pricing (per 1M tokens)
    pub const OPENAI_GPT4_TURBO_INPUT: f64 = 10.0;
    pub const OPENAI_GPT4_TURBO_OUTPUT: f64 = 30.0;

    /// OpenAI GPT-3.5 Turbo pricing (per 1M tokens)
    pub const OPENAI_GPT35_TURBO_INPUT: f64 = 0.50;
    pub const OPENAI_GPT35_TURBO_OUTPUT: f64 = 1.50;

    /// Anthropic Claude 3.5 Sonnet pricing (per 1M tokens)
    pub const ANTHROPIC_CLAUDE_35_SONNET_INPUT: f64 = 3.0;
    pub const ANTHROPIC_CLAUDE_35_SONNET_OUTPUT: f64 = 15.0;

    /// Anthropic Claude 3 Opus pricing (per 1M tokens)
    pub const ANTHROPIC_CLAUDE_3_OPUS_INPUT: f64 = 15.0;
    pub const ANTHROPIC_CLAUDE_3_OPUS_OUTPUT: f64 = 75.0;

    /// Anthropic Claude 3 Haiku pricing (per 1M tokens)
    pub const ANTHROPIC_CLAUDE_3_HAIKU_INPUT: f64 = 0.25;
    pub const ANTHROPIC_CLAUDE_3_HAIKU_OUTPUT: f64 = 1.25;

    /// Google Gemini 2.0 Flash pricing (per 1M tokens)
    pub const GEMINI_20_FLASH_INPUT: f64 = 0.10;
    pub const GEMINI_20_FLASH_OUTPUT: f64 = 0.40;

    /// Google Gemini 2.0 Flash-Lite pricing (per 1M tokens)
    pub const GEMINI_20_FLASH_LITE_INPUT: f64 = 0.075;
    pub const GEMINI_20_FLASH_LITE_OUTPUT: f64 = 0.30;

    /// Google Gemini 1.5 Pro pricing (per 1M tokens)
    pub const GEMINI_15_PRO_INPUT: f64 = 1.25;
    pub const GEMINI_15_PRO_OUTPUT: f64 = 5.00;

    /// Google Gemini 1.5 Flash pricing (per 1M tokens)
    pub const GEMINI_15_FLASH_INPUT: f64 = 0.075;
    pub const GEMINI_15_FLASH_OUTPUT: f64 = 0.30;

    /// xAI Grok pricing (per 1M tokens) - estimated
    pub const XAI_GROK_INPUT: f64 = 5.0;
    pub const XAI_GROK_OUTPUT: f64 = 15.0;

    /// Ollama (local) - free
    pub const OLLAMA_INPUT: f64 = 0.0;
    pub const OLLAMA_OUTPUT: f64 = 0.0;

    /// Default fallback pricing (per 1M tokens)
    pub const DEFAULT_INPUT: f64 = 1.0;
    pub const DEFAULT_OUTPUT: f64 = 3.0;
}

/// Token usage status indicator for visual feedback
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenUsageStatus {
    /// Green: Healthy usage (<70% of limit)
    Green,
    /// Yellow: Approaching limit (70-90% of limit)
    Yellow,
    /// Red: Near or exceeding limit (>90% of limit)
    Red,
}

/// Which output stream a shell chunk originated from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellOutputStream {
    /// Standard output
    Stdout,
    /// Standard error
    Stderr,
}

/// Lifecycle state of a shell process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellProcessState {
    /// Process has been spawned and is running
    Started,
    /// Process completed normally
    Completed,
    /// Process failed (non-zero exit or spawn error)
    Failed,
    /// Process was stopped by user request (SIGTERM/SIGKILL)
    Stopped,
    /// Process was paused by user request (SIGSTOP)
    Paused,
    /// Process was resumed after pause (SIGCONT)
    Resumed,
}

/// Lifecycle state of a long-lived interactive shell session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellSessionState {
    /// PTY shell process is starting.
    Starting,
    /// PTY shell is alive and available for reuse.
    Idle,
    /// PTY shell currently has an active command lease.
    Busy,
    /// PTY shell is attempting to interrupt the active foreground job.
    Interrupting,
    /// PTY shell is shutting down.
    Stopping,
    /// PTY shell was stopped intentionally.
    Stopped,
    /// PTY shell terminated unexpectedly or became unusable.
    Failed,
}

/// Compact task view for runtime-authored task-state updates.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskRuntimeTaskView {
    /// Stable task identifier.
    pub id: String,
    /// Human-readable task name.
    pub name: String,
    /// Runtime task status string.
    pub status: String,
}

/// Runtime-authored task scheduler snapshot streamed to UI surfaces.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskRuntimeSnapshot {
    /// Root task driving the current run.
    pub root_task_id: String,
    /// Current runtime-selected task, if any.
    pub current_task: Option<TaskRuntimeTaskView>,
    /// Ready tasks the runtime deems actionable now.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ready_tasks: Vec<TaskRuntimeTaskView>,
    /// Tasks the runtime considers safe to batch in parallel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parallel_ready_tasks: Vec<TaskRuntimeTaskView>,
    /// Tasks currently blocked by dependencies or parent ordering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_tasks: Vec<TaskRuntimeTaskView>,
    /// Open tasks that are not yet terminal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_tasks: Vec<TaskRuntimeTaskView>,
    /// Recently completed tasks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_tasks: Vec<TaskRuntimeTaskView>,
    /// Runtime-detected missing completion requirements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_requirements: Vec<String>,
    /// Human-readable scheduler summary.
    pub status_message: String,
}

/// Public-facing narration stage for brief between-tool updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NarrationStage {
    /// Gathering local or external context before the next step.
    Context,
    /// Planning or scoping the next action.
    Planning,
    /// Executing the primary requested work.
    Execution,
    /// Verifying or validating the result.
    Verification,
    /// Waiting on a blocker or missing requirement.
    Blocked,
    /// General progress update.
    Progress,
}

impl NarrationStage {
    /// Return the stable snake_case label used by the UI.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Planning => "planning",
            Self::Execution => "execution",
            Self::Verification => "verification",
            Self::Blocked => "blocked",
            Self::Progress => "progress",
        }
    }
}

/// Structured public narration content rendered between major loop events.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublicNarration {
    /// Short collapsed heading for the narration block.
    pub title: String,
    /// Natural prose fallback used by plain-text surfaces.
    pub message: String,
    /// Concise statement of what changed or what the agent is doing now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Why the current step matters or why it was chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What the agent expects to do immediately after this point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    /// Short evidence bullets grounding the narration in observed runtime facts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

/// A chunk of streaming response
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Content from the model's thinking process
    Thinking(String),
    /// Public-facing narration explaining the current direction.
    Narration {
        /// Structured public narration content for user-facing progress updates.
        narration: PublicNarration,
        stage: NarrationStage,
    },
    /// A text chunk from the LLM
    Text(String),
    /// Start of a tool call
    ToolCallStart { id: String, name: String },
    /// Arguments delta for the current tool call
    ToolCallArgs(String),
    /// End of the current tool call (LLM finished specifying the call)
    ToolCallEnd,
    /// Result of tool execution with status and output
    ToolCallResult {
        /// Tool name
        name: String,
        /// Whether the tool succeeded
        success: bool,
        /// Output or error message
        output: String,
        /// Execution duration in milliseconds
        duration_ms: u64,
    },
    /// Retry attempt notification for user feedback
    RetryAttempt {
        /// Current attempt number (1-indexed)
        attempt: u32,
        /// Maximum attempts configured
        max_attempts: u32,
        /// Delay before next retry in milliseconds
        delay_ms: u64,
        /// Error that triggered the retry
        error_message: String,
    },
    /// Context compaction notification for user feedback
    ContextCompacted {
        /// Number of messages before compaction
        messages_before: usize,
        /// Number of messages after compaction
        messages_after: usize,
        /// Tokens saved by compaction
        tokens_saved: usize,
        /// Summary of what was compacted
        summary: String,
    },
    /// Token usage notification for user feedback
    /// Provides real-time visibility into context window utilization
    TokenUsageUpdate {
        /// Estimated tokens in current request
        estimated: usize,
        /// Maximum input tokens allowed
        limit: usize,
        /// Utilization percentage (0-100)
        percentage: u8,
        /// Status indicator: Green (<70%), Yellow (70-90%), Red (>90%)
        status: TokenUsageStatus,
        /// Estimated cost in USD for this request (input only)
        estimated_cost: f64,
    },

    /// A user-facing status message intended for UIs.
    ///
    /// This is for short, transient notifications that should not count as
    /// streamed "output" (i.e., it must not prevent retry when a provider
    /// attempt fails before any actual response content is forwarded).
    Status {
        /// Human-readable status message.
        message: String,
    },
    /// A request from the agent to change configuration.
    ///
    /// This is surfaced to UIs (GUI/TUI) so they can prompt for confirmation or
    /// apply changes immediately in permissive mode.
    ConfigRequest {
        /// Operation type (e.g. "set")
        operation: String,
        /// Config key (e.g. "provider", "model", "permission_level")
        key: String,
        /// Requested value. When `None`, this represents a "query"/"show" style request.
        value: Option<String>,
        /// Whether the UI must request explicit confirmation before applying.
        requires_confirmation: bool,
    },
    /// Tool execution requires user confirmation before proceeding.
    ///
    /// This is emitted when a tool call is detected but the session's permission
    /// level requires user approval before execution. The UI should display a
    /// confirmation dialog and respond via the confirmation channel.
    ToolConfirmationRequired {
        /// Unique ID for this confirmation request
        confirmation_id: String,
        /// Tool name that requires confirmation
        tool_name: String,
        /// Tool arguments (JSON string)
        tool_args: String,
        /// Human-readable description of what the tool will do
        description: String,
        /// Risk level (0-10, higher = more dangerous)
        risk_level: u8,
        /// Category of the tool (e.g., "write", "shell", "network")
        category: String,
    },
    /// Tool execution was blocked by permission settings.
    ///
    /// This is emitted when a tool call is detected but the session's permission
    /// level blocks the operation entirely (e.g., Sandbox mode blocking writes).
    ToolBlocked {
        /// Tool name that was blocked
        tool_name: String,
        /// Reason for blocking
        reason: String,
    },
    /// Memory bank entry saved notification for user feedback
    /// Emitted when context is saved to persistent memory bank file
    MemoryBankSaved {
        /// Path to the saved memory bank file
        file_path: String,
        /// Session ID associated with this entry
        session_id: String,
        /// Brief summary of what was saved
        summary: String,
        /// Number of messages saved
        messages_saved: usize,
    },
    /// Agentic loop iteration boundary marker.
    ///
    /// Emitted at the start of each agentic loop iteration. When `iteration > 0`,
    /// it signals that the text following this marker is the LLM's **intermediate
    /// reasoning** about previous tool results (not the final response). UIs should
    /// render this text differently (e.g., with a `◆` prefix or distinct styling)
    /// and clearly delineate iteration boundaries.
    AgentLoopIteration {
        /// Zero-based iteration index (0 = first LLM call, 1+ = continuation after tools)
        iteration: u32,
    },
    /// Runtime-authored task-state snapshot.
    TaskRuntimeSnapshot {
        /// Authoritative runtime snapshot for the tracked task tree.
        snapshot: TaskRuntimeSnapshot,
    },
    /// Real-time shell output chunk (stdout or stderr).
    ///
    /// Emitted while a shell command is executing so the UI can stream output
    /// into an embedded terminal component. Each chunk is a small fragment of
    /// text (typically one or a few lines).
    ShellOutput {
        /// Unique identifier for the shell process (matches `ShellLifecycle`).
        process_id: String,
        /// Long-lived shell session that produced this output, if any.
        shell_session_id: Option<String>,
        /// Whether this chunk comes from stdout or stderr.
        stream: ShellOutputStream,
        /// The raw text data (may contain ANSI escape sequences).
        data: String,
    },
    /// Shell process lifecycle event.
    ///
    /// Emitted when a shell process transitions between states (started,
    /// completed, failed, stopped, paused, resumed). The UI uses this to
    /// update the console header, show exit codes, and enable/disable
    /// control buttons.
    ShellLifecycle {
        /// Unique identifier for the shell process (matches `ShellOutput`).
        process_id: String,
        /// Long-lived shell session that owns this command run, if any.
        shell_session_id: Option<String>,
        /// New state of the process.
        state: ShellProcessState,
        /// Exit code (only meaningful when `state` is `Completed` or `Failed`).
        exit_code: Option<i32>,
        /// Wall-clock duration in milliseconds (set on terminal states).
        duration_ms: Option<u64>,
        /// The command string that was executed.
        command: String,
        /// Working directory for the command.
        cwd: Option<String>,
    },
    /// Interactive shell session lifecycle event.
    ShellSessionLifecycle {
        /// Long-lived shell session identifier.
        shell_session_id: String,
        /// New state of the shell session.
        state: ShellSessionState,
        /// Current working directory tracked for the session.
        cwd: Option<String>,
        /// Active command process id, when the session is busy.
        active_process_id: Option<String>,
        /// Active command string, when the session is busy.
        active_command: Option<String>,
        /// Whether the session is currently eligible for reuse.
        available_for_reuse: bool,
        /// Whether the session is a user-facing interactive shell.
        interactive: bool,
        /// Whether the session is currently reserved for direct user management.
        user_managed: bool,
    },
    /// Stream completed successfully with optional token usage
    Done(Option<TokenUsage>),
    /// Stream was cancelled
    Cancelled,
    /// Stream was paused (cancelled with the intent to resume later).
    ///
    /// The caller is responsible for capturing the `PausedExecutionState` from the
    /// accumulated streaming context. This variant is a signal to the UI to render
    /// a resumable pause marker rather than a hard cancellation.
    Paused,
    /// An error occurred
    Error(String),
    /// Context overflow error - requires compaction before retry.
    ///
    /// This is emitted when the LLM request exceeds the model's context window.
    /// Unlike generic errors, this signals to the pipeline that it should:
    /// 1. Compact the context (summarize history, remove old messages)
    /// 2. Retry the request with the reduced context
    /// 3. Optionally learn the model's actual limit for future requests
    ContextOverflow {
        /// The original error message from the provider
        error_message: String,
    },
    /// Experiential reflection phase has started (ERL-inspired).
    ///
    /// UIs can use this to surface that the pipeline is performing a
    /// post-answer self-review step rather than continuing normal tool use.
    ReflectionStarted {
        /// Human-readable reason for triggering reflection.
        reason: String,
    },
    /// Experiential reflection phase completed (ERL-inspired).
    ///
    /// This reports both the learned summary and whether the result stayed only
    /// in short-term/session storage or was also promoted into long-term memory.
    ReflectionComplete {
        /// Brief summary of what was learned.
        summary: String,
        /// Whether the reflection was stored in session working memory.
        stored: bool,
        /// Whether the reflection was promoted to long-term memory bank.
        promoted: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptOutcome {
    Success,
    RetryableError,
    /// Context length exceeded - needs compaction, not blind retry
    ContextOverflowError,
    FatalError,
    Cancelled,
    Paused,
    UnexpectedEnd,
}

#[derive(Debug, Clone)]
struct AttemptForwardResult {
    outcome: AttemptOutcome,
    /// Whether we forwarded any non-terminal output chunk (Text/Thinking/tool call) to the caller.
    forwarded_output: bool,
    /// Error message when outcome is RetryableError/FatalError.
    error: Option<String>,
}

/// Forward chunks from a single provider attempt to the caller.
///
/// Design goal: preserve a *true streaming* UX.
///
/// Retry policy: we only consider retrying if the attempt fails **before any output is forwarded**.
async fn forward_attempt_stream(
    attempt_rx: &mut mpsc::Receiver<StreamChunk>,
    tx: &mpsc::Sender<StreamChunk>,
) -> AttemptForwardResult {
    let mut forwarded_output = false;

    while let Some(chunk) = attempt_rx.recv().await {
        match &chunk {
            StreamChunk::Text(_)
            | StreamChunk::Thinking(_)
            | StreamChunk::ToolCallStart { .. }
            | StreamChunk::ToolCallArgs(_)
            | StreamChunk::ToolCallEnd
            | StreamChunk::ToolCallResult { .. } => {
                forwarded_output = true;
                let _ = tx.send(chunk).await;
            }
            StreamChunk::RetryAttempt { .. } => {
                // Forward retry notifications without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::ContextCompacted { .. } => {
                // Forward compaction notifications without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::TokenUsageUpdate { .. } => {
                // Forward token usage updates without marking as output
                send_token_usage_chunk_best_effort(tx, chunk).await;
            }
            StreamChunk::Status { .. } => {
                // Forward status updates without marking as output
                send_status_chunk_best_effort(tx, chunk).await;
            }
            StreamChunk::ConfigRequest { .. } => {
                // Forward config requests without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::ToolConfirmationRequired { .. } => {
                // Forward tool confirmation requests without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::ToolBlocked { .. } => {
                // Forward tool blocked notifications without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::MemoryBankSaved { .. } => {
                // Forward memory bank notifications without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::AgentLoopIteration { .. } => {
                // Forward agent loop iteration markers without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::Narration { .. } => {
                // Forward narration updates without marking as model output.
                let _ = tx.try_send(chunk);
            }
            StreamChunk::TaskRuntimeSnapshot { .. } => {
                // Forward runtime task-state updates without marking as output
                let _ = tx.try_send(chunk);
            }
            StreamChunk::ReflectionStarted { .. } | StreamChunk::ReflectionComplete { .. } => {
                // Forward reflection events without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::ShellOutput { .. } => {
                // Forward shell output chunks without marking as output –
                // they are part of tool execution, already tracked via
                // ToolCallResult.
                let _ = tx.send(chunk).await;
            }
            StreamChunk::ShellLifecycle { .. } => {
                // Forward shell lifecycle events without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::ShellSessionLifecycle { .. } => {
                // Forward shell session lifecycle events without marking as output
                let _ = tx.send(chunk).await;
            }
            StreamChunk::Done(_) => {
                let _ = tx.send(chunk).await;
                return AttemptForwardResult {
                    outcome: AttemptOutcome::Success,
                    forwarded_output,
                    error: None,
                };
            }
            StreamChunk::Cancelled => {
                let _ = tx.send(StreamChunk::Cancelled).await;
                return AttemptForwardResult {
                    outcome: AttemptOutcome::Cancelled,
                    forwarded_output,
                    error: None,
                };
            }
            StreamChunk::Paused => {
                let _ = tx.send(StreamChunk::Paused).await;
                return AttemptForwardResult {
                    outcome: AttemptOutcome::Paused,
                    forwarded_output,
                    error: None,
                };
            }
            StreamChunk::Error(e) => {
                // Context overflow errors need special handling - they cannot be fixed
                // by blind retries. The caller should compact context and retry.
                if is_context_overflow_message(e) {
                    return AttemptForwardResult {
                        outcome: AttemptOutcome::ContextOverflowError,
                        forwarded_output,
                        error: Some(e.clone()),
                    };
                }

                // If we already streamed anything to the caller, we cannot safely retry
                // without causing duplicated / confusing output.
                if forwarded_output {
                    let _ = tx.send(StreamChunk::Error(e.clone())).await;
                    return AttemptForwardResult {
                        outcome: AttemptOutcome::FatalError,
                        forwarded_output,
                        error: Some(e.clone()),
                    };
                }

                return AttemptForwardResult {
                    outcome: AttemptOutcome::RetryableError,
                    forwarded_output,
                    error: Some(e.clone()),
                };
            }
            StreamChunk::ContextOverflow { error_message } => {
                // Context overflow received as a chunk - forward and signal recovery needed
                return AttemptForwardResult {
                    outcome: AttemptOutcome::ContextOverflowError,
                    forwarded_output,
                    error: Some(error_message.clone()),
                };
            }
        }
    }

    AttemptForwardResult {
        outcome: AttemptOutcome::UnexpectedEnd,
        forwarded_output,
        error: None,
    }
}

/// Cancellation token for streaming requests
#[derive(Clone, Debug)]
pub struct CancellationToken {
    disposition: Arc<AtomicU8>,
}

/// Requested interruption disposition for a streaming request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CancellationDisposition {
    Running = 0,
    Cancelled = 1,
    Paused = 2,
}

impl CancellationToken {
    /// Create a new cancellation token
    pub fn new() -> Self {
        Self {
            disposition: Arc::new(AtomicU8::new(CancellationDisposition::Running as u8)),
        }
    }

    /// Cancel the streaming request
    pub fn cancel(&self) {
        self.disposition
            .store(CancellationDisposition::Cancelled as u8, Ordering::SeqCst);
    }

    /// Pause the streaming request with the intent to resume later.
    pub fn pause(&self) {
        let _ = self.disposition.compare_exchange(
            CancellationDisposition::Running as u8,
            CancellationDisposition::Paused as u8,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }

    /// Check if cancellation has been requested
    pub fn is_cancelled(&self) -> bool {
        !matches!(self.disposition(), CancellationDisposition::Running)
    }

    /// Returns `true` when the request should be treated as resumably paused.
    pub fn is_pause_requested(&self) -> bool {
        matches!(self.disposition(), CancellationDisposition::Paused)
    }

    /// Returns the requested interruption disposition.
    pub fn disposition(&self) -> CancellationDisposition {
        match self.disposition.load(Ordering::SeqCst) {
            value if value == CancellationDisposition::Paused as u8 => {
                CancellationDisposition::Paused
            }
            value if value == CancellationDisposition::Cancelled as u8 => {
                CancellationDisposition::Cancelled
            }
            _ => CancellationDisposition::Running,
        }
    }

    /// Terminal streaming chunk matching the currently requested interruption.
    pub fn interruption_chunk(&self) -> StreamChunk {
        match self.disposition() {
            CancellationDisposition::Paused => StreamChunk::Paused,
            CancellationDisposition::Cancelled | CancellationDisposition::Running => {
                StreamChunk::Cancelled
            }
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a reqwest client for streaming requests
fn create_streaming_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(STREAMING_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Helper to parse <think> tags from chunks.
/// Handles tags that may be split across multiple chunks by buffering partial matches.
struct ThinkingParser {
    in_think_block: bool,
    /// Buffer for potential partial tag at end of chunk
    buffer: String,
}

impl ThinkingParser {
    fn new() -> Self {
        Self {
            in_think_block: false,
            buffer: String::new(),
        }
    }

    fn process(&mut self, chunk: &str) -> Vec<StreamChunk> {
        let mut chunks = Vec::new();

        // Prepend any buffered content from previous chunk
        let input = if self.buffer.is_empty() {
            chunk.to_string()
        } else {
            std::mem::take(&mut self.buffer) + chunk
        };

        let mut remaining = input.as_str();

        while !remaining.is_empty() {
            if self.in_think_block {
                if let Some(end_idx) = remaining.find("</think>") {
                    let thinking_content = &remaining[..end_idx];
                    if !thinking_content.is_empty() {
                        chunks.push(StreamChunk::Thinking(thinking_content.to_string()));
                    }
                    self.in_think_block = false;
                    remaining = &remaining[end_idx + 8..];
                } else {
                    // Check for partial </think> at end
                    let partial = Self::find_partial_end_tag(remaining);
                    if partial > 0 {
                        let safe_len = remaining.len() - partial;
                        if safe_len > 0 {
                            chunks.push(StreamChunk::Thinking(remaining[..safe_len].to_string()));
                        }
                        self.buffer = remaining[safe_len..].to_string();
                    } else {
                        chunks.push(StreamChunk::Thinking(remaining.to_string()));
                    }
                    break;
                }
            } else if let Some(start_idx) = remaining.find("<think>") {
                let text_content = &remaining[..start_idx];
                if !text_content.is_empty() {
                    chunks.push(StreamChunk::Text(text_content.to_string()));
                }
                self.in_think_block = true;
                remaining = &remaining[start_idx + 7..];
            } else {
                // Check for partial <think> at end
                let partial = Self::find_partial_start_tag(remaining);
                if partial > 0 {
                    let safe_len = remaining.len() - partial;
                    if safe_len > 0 {
                        chunks.push(StreamChunk::Text(remaining[..safe_len].to_string()));
                    }
                    self.buffer = remaining[safe_len..].to_string();
                } else {
                    chunks.push(StreamChunk::Text(remaining.to_string()));
                }
                break;
            }
        }
        chunks
    }

    /// Find length of partial "<think>" at end of string
    fn find_partial_start_tag(s: &str) -> usize {
        const TAG: &str = "<think>";
        for len in (1..TAG.len()).rev() {
            if s.ends_with(&TAG[..len]) {
                return len;
            }
        }
        0
    }

    /// Find length of partial "</think>" at end of string
    fn find_partial_end_tag(s: &str) -> usize {
        const TAG: &str = "</think>";
        for len in (1..TAG.len()).rev() {
            if s.ends_with(&TAG[..len]) {
                return len;
            }
        }
        0
    }
}

/// Split a complete assistant message into (user-facing text, optional thinking) based on
/// `<think>...</think>` blocks.
///
/// This is used by non-streaming callers to keep behavior consistent with streaming.
pub fn split_think_blocks(input: &str) -> (String, Option<String>) {
    let mut parser = ThinkingParser::new();
    let mut content = String::new();
    let mut thinking = String::new();

    for chunk in parser.process(input) {
        match chunk {
            StreamChunk::Text(t) => content.push_str(&t),
            StreamChunk::Thinking(t) => thinking.push_str(&t),
            _ => {}
        }
    }

    let thinking = if thinking.trim().is_empty() {
        None
    } else {
        Some(thinking)
    };

    (content, thinking)
}

fn collect_complete_lines(buffer: &mut String, incoming: &str) -> Vec<String> {
    buffer.push_str(incoming);
    let mut out = Vec::new();
    let mut start = 0usize;

    {
        let bytes = buffer.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'\n' {
                let line = buffer[start..i].trim_end_matches('\r');
                out.push(line.to_string());
                start = i + 1;
            }
        }
    }

    if start > 0 {
        buffer.drain(..start);
    }

    out
}

/// Build the JSON request body for an OpenAI Chat Completions streaming call.
fn build_openai_chat_request_body(
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": true
    });

    // Enable structured tool calling when schemas are provided.
    if let Some(tools) = tools
        && !tools.is_empty()
    {
        body["tools"] = serde_json::Value::Array(tools.to_vec());
        body["tool_choice"] = serde_json::json!("auto");
    }

    body
}

/// Build the JSON request body for an OpenAI Responses streaming call.
fn build_openai_responses_request_body(
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "input": [{"role": "user", "content": prompt}],
        "stream": true
    });

    if let Some(tools) = tools
        && !tools.is_empty()
    {
        body["tools"] = serde_json::Value::Array(tools.to_vec());
        body["tool_choice"] = serde_json::json!("auto");
    }

    body
}

fn openai_endpoint_path(api: OpenAiApi) -> &'static str {
    match api {
        OpenAiApi::ChatCompletions => "/v1/chat/completions",
        OpenAiApi::Responses => "/v1/responses",
    }
}

fn format_openai_http_error(
    status: reqwest::StatusCode,
    provider_name: &str,
    model: &str,
    api: OpenAiApi,
    body: &str,
    retry_after: Option<Duration>,
) -> String {
    if status == reqwest::StatusCode::NOT_FOUND && body.contains("This is not a chat model") {
        let mut message = format!(
            "{provider_name} model '{}' appears to require /v1/responses, but Gestura selected {}. Raw provider error: {}",
            model.trim(),
            openai_endpoint_path(api),
            body
        );
        if let Some(retry_after) = retry_after {
            message.push_str(&format_retry_after_suffix(retry_after));
        }
        return message;
    }

    let mut message = format!(
        "{provider_name} {} HTTP {}: {}",
        openai_endpoint_path(api),
        status,
        body
    );
    if let Some(retry_after) = retry_after {
        message.push_str(&format_retry_after_suffix(retry_after));
    }
    message
}

fn format_retry_after_suffix(retry_after: Duration) -> String {
    format!(
        " Provider suggested retrying after {} seconds.",
        retry_after.as_secs().max(1)
    )
}

fn parse_retry_after_value(value: &str) -> Option<Duration> {
    let seconds = value.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds.max(1)))
}

fn response_retry_after_hint(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()
        .and_then(parse_retry_after_value)
}

fn retry_after_hint_from_error_message(message: &str) -> Option<Duration> {
    let marker = "provider suggested retrying after ";
    let lower = message.to_ascii_lowercase();
    let start = lower.find(marker)? + marker.len();
    let remainder = &lower[start..];
    let seconds = remainder
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds.max(1)))
}

fn error_is_rate_limited_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("http 429")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("quota")
}

fn select_streaming_retry_delay(
    policy: &RetryPolicy,
    retry_attempt: u32,
    error_message: &str,
) -> Duration {
    let base_delay = policy.delay_for_attempt(retry_attempt);

    if let Some(retry_after) = retry_after_hint_from_error_message(error_message) {
        return retry_after.max(base_delay);
    }

    if error_is_rate_limited_message(error_message) {
        return base_delay.max(Duration::from_secs(5));
    }

    base_delay
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PendingOpenAiToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PendingOpenAiResponsesToolCall {
    id: String,
    name: String,
    arguments: String,
    finished: bool,
}

fn merge_openai_tool_call_delta(
    pending: &mut BTreeMap<usize, PendingOpenAiToolCall>,
    call: &serde_json::Value,
    fallback_index: usize,
) {
    let index = call
        .get("index")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or(fallback_index);

    let entry = pending.entry(index).or_default();

    if let Some(id) = call["id"].as_str()
        && !id.is_empty()
    {
        entry.id = id.to_string();
    }

    if let Some(name) = call["function"]["name"].as_str()
        && !name.is_empty()
    {
        entry.name = name.to_string();
    }

    if let Some(arguments) = call["function"]["arguments"].as_str()
        && !arguments.is_empty()
    {
        entry.arguments.push_str(arguments);
    }
}

fn take_openai_tool_calls(
    pending: &mut BTreeMap<usize, PendingOpenAiToolCall>,
) -> Vec<(usize, PendingOpenAiToolCall)> {
    std::mem::take(pending)
        .into_iter()
        .filter(|(_, call)| !call.name.is_empty())
        .collect()
}

async fn emit_openai_tool_calls(
    tx: &mpsc::Sender<StreamChunk>,
    pending: &mut BTreeMap<usize, PendingOpenAiToolCall>,
) {
    for (index, call) in take_openai_tool_calls(pending) {
        let id = if call.id.is_empty() {
            format!("openai-tool-{index}")
        } else {
            call.id
        };

        let _ = tx
            .send(StreamChunk::ToolCallStart {
                id,
                name: call.name,
            })
            .await;

        if !call.arguments.is_empty() {
            let _ = tx.send(StreamChunk::ToolCallArgs(call.arguments)).await;
        }

        let _ = tx.send(StreamChunk::ToolCallEnd).await;
    }
}

fn merge_openai_responses_tool_item(
    pending: &mut BTreeMap<usize, PendingOpenAiResponsesToolCall>,
    tool_indices: &mut HashMap<String, usize>,
    event: &serde_json::Value,
    fallback_index: usize,
) {
    let index = resolve_openai_responses_tool_index(tool_indices, event, fallback_index);

    let item = event.get("item").unwrap_or(event);
    let entry = pending.entry(index).or_default();

    if let Some(id) = item["call_id"].as_str().or_else(|| item["id"].as_str())
        && !id.is_empty()
    {
        entry.id = id.to_string();
    }

    if let Some(name) = item["name"].as_str()
        && !name.is_empty()
    {
        entry.name = name.to_string();
    }

    if let Some(arguments) = item["arguments"].as_str()
        && !arguments.is_empty()
    {
        entry.arguments = arguments.to_string();
    }

    if event["type"].as_str() == Some("response.output_item.done")
        || item["status"].as_str() == Some("completed")
    {
        entry.finished = true;
    }
}

fn merge_openai_responses_tool_argument_delta(
    pending: &mut BTreeMap<usize, PendingOpenAiResponsesToolCall>,
    tool_indices: &mut HashMap<String, usize>,
    event: &serde_json::Value,
    fallback_index: usize,
) {
    let index = resolve_openai_responses_tool_index(tool_indices, event, fallback_index);

    let entry = pending.entry(index).or_default();

    if let Some(id) = event["call_id"].as_str()
        && !id.is_empty()
    {
        entry.id = id.to_string();
    } else if entry.id.is_empty()
        && let Some(id) = event["item_id"].as_str()
        && !id.is_empty()
    {
        entry.id = id.to_string();
    }

    if let Some(delta) = event["delta"].as_str()
        && !delta.is_empty()
    {
        entry.arguments.push_str(delta);
    }
}

fn complete_openai_responses_tool_arguments(
    pending: &mut BTreeMap<usize, PendingOpenAiResponsesToolCall>,
    tool_indices: &mut HashMap<String, usize>,
    event: &serde_json::Value,
    fallback_index: usize,
) {
    let index = resolve_openai_responses_tool_index(tool_indices, event, fallback_index);

    let entry = pending.entry(index).or_default();

    if let Some(id) = event["call_id"].as_str()
        && !id.is_empty()
    {
        entry.id = id.to_string();
    } else if entry.id.is_empty()
        && let Some(id) = event["item_id"].as_str()
        && !id.is_empty()
    {
        entry.id = id.to_string();
    }

    if let Some(arguments) = event["arguments"].as_str()
        && !arguments.is_empty()
    {
        entry.arguments = arguments.to_string();
    }

    entry.finished = true;
}

async fn emit_ready_openai_responses_tool_calls(
    tx: &mpsc::Sender<StreamChunk>,
    pending: &mut BTreeMap<usize, PendingOpenAiResponsesToolCall>,
    emitted_ids: &mut HashSet<String>,
    flush_all: bool,
) {
    let mut ready = Vec::new();

    for (&index, call) in pending.iter() {
        if call.name.is_empty() {
            if flush_all {
                continue;
            }
            break;
        }

        if flush_all || call.finished {
            ready.push(index);
            continue;
        }

        break;
    }

    for index in ready {
        if let Some(call) = pending.remove(&index) {
            let id = if call.id.is_empty() {
                format!("openai-response-tool-{index}")
            } else {
                call.id
            };

            if !emitted_ids.insert(id.clone()) {
                tracing::debug!(
                    tool_call_id = %id,
                    pending_index = index,
                    "Skipping duplicate OpenAI Responses tool-call emission"
                );
                continue;
            }

            let _ = tx
                .send(StreamChunk::ToolCallStart {
                    id,
                    name: call.name,
                })
                .await;

            if !call.arguments.is_empty() {
                let _ = tx.send(StreamChunk::ToolCallArgs(call.arguments)).await;
            }

            let _ = tx.send(StreamChunk::ToolCallEnd).await;
        }
    }
}

fn openai_responses_output_index(event: &serde_json::Value) -> Option<usize> {
    event
        .get("output_index")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
}

fn openai_responses_tool_aliases(event: &serde_json::Value) -> Vec<String> {
    let item = event.get("item").unwrap_or(event);
    let mut aliases = Vec::with_capacity(4);

    for candidate in [
        item["call_id"].as_str(),
        event["call_id"].as_str(),
        item["id"].as_str(),
        event["item_id"].as_str(),
    ] {
        if let Some(alias) = candidate.filter(|alias| !alias.is_empty())
            && !aliases.iter().any(|existing| existing == alias)
        {
            aliases.push(alias.to_string());
        }
    }

    aliases
}

fn resolve_openai_responses_tool_index(
    tool_indices: &mut HashMap<String, usize>,
    event: &serde_json::Value,
    fallback_index: usize,
) -> usize {
    let aliases = openai_responses_tool_aliases(event);

    if let Some(existing_index) = aliases
        .iter()
        .find_map(|alias| tool_indices.get(alias).copied())
    {
        for alias in aliases {
            tool_indices.insert(alias, existing_index);
        }
        return existing_index;
    }

    let index = openai_responses_output_index(event).unwrap_or(fallback_index);
    for alias in aliases {
        tool_indices.insert(alias, index);
    }
    index
}

async fn stream_openai_chat_compatible(
    api_key: &str,
    base_url: &str,
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        openai_endpoint_path(OpenAiApi::ChatCompletions)
    );
    let body = build_openai_chat_request_body(model, prompt, tools);

    let client = create_streaming_client();
    let response = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("OpenAI streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let retry_after = response_retry_after_hint(response.headers());
        let body = response.text().await.unwrap_or_default();

        // ALWAYS log this so we know this code path is being hit
        tracing::error!(
            status = %status,
            body_len = body.len(),
            "[CONTEXT_OVERFLOW_CHECK] HTTP error received in stream_openai_chat_compatible"
        );

        let error_msg = format_openai_http_error(
            status,
            "OpenAI",
            model,
            OpenAiApi::ChatCompletions,
            &body,
            retry_after,
        );

        // Check if this is a context overflow error - needs special handling
        let is_overflow =
            is_context_overflow_message(&error_msg) || is_context_overflow_message(&body);
        tracing::error!(
            is_overflow = is_overflow,
            body_preview = %body.chars().take(300).collect::<String>(),
            "[CONTEXT_OVERFLOW_CHECK] Checking for context overflow"
        );

        if is_overflow {
            tracing::error!("[CONTEXT_OVERFLOW_CHECK] Returning AppError::ContextOverflow");
            return Err(AppError::ContextOverflow(error_msg));
        }

        return Err(AppError::Llm(error_msg));
    }

    let mut stream = response.bytes_stream();
    let mut parser = ThinkingParser::new();
    let mut line_buffer = String::new();
    // OpenAI-compatible providers may stream multiple tool calls concurrently,
    // identifying each call by `index` and interleaving argument fragments
    // across SSE events. Buffer them until the provider signals the end of the
    // tool-call block, then emit complete Start/Args/End sequences in index
    // order so downstream consumers never merge fragments from different calls.
    let mut pending_tool_calls = BTreeMap::<usize, PendingOpenAiToolCall>::new();

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(cancel_token.interruption_chunk()).await;
            return Ok(());
        }

        match chunk_result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in collect_complete_lines(&mut line_buffer, &text) {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    if data == "[DONE]" {
                        emit_openai_tool_calls(&tx, &mut pending_tool_calls).await;
                        let _ = tx.send(StreamChunk::Done(None)).await;
                        return Ok(());
                    }
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        // Handle content
                        if let Some(content) = json["choices"][0]["delta"]["content"].as_str()
                            && !content.is_empty()
                        {
                            let chunks = parser.process(content);
                            for chunk in chunks {
                                let _ = tx.send(chunk).await;
                            }
                        }

                        // Handle tool calls
                        if let Some(tool_calls) =
                            json["choices"][0]["delta"]["tool_calls"].as_array()
                        {
                            for (fallback_index, call) in tool_calls.iter().enumerate() {
                                merge_openai_tool_call_delta(
                                    &mut pending_tool_calls,
                                    call,
                                    fallback_index,
                                );
                            }
                        }

                        // Handle finish reason — emit each complete tool call in order.
                        if let Some(finish_reason) = json["choices"][0]["finish_reason"].as_str()
                            && finish_reason == "tool_calls"
                        {
                            emit_openai_tool_calls(&tx, &mut pending_tool_calls).await;
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx
                    .send(StreamChunk::Error(format!("Stream error: {}", e)))
                    .await;
                return Err(AppError::Llm(format!("Stream error: {}", e)));
            }
        }
    }

    let _ = tx.send(StreamChunk::Done(None)).await;
    Ok(())
}

async fn stream_openai_responses(
    api_key: &str,
    base_url: &str,
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        openai_endpoint_path(OpenAiApi::Responses)
    );
    let body = build_openai_responses_request_body(model, prompt, tools);

    let client = create_streaming_client();
    let response = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("OpenAI streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let retry_after = response_retry_after_hint(response.headers());
        let body = response.text().await.unwrap_or_default();
        let error_msg = format_openai_http_error(
            status,
            "OpenAI",
            model,
            OpenAiApi::Responses,
            &body,
            retry_after,
        );

        // Check if this is a context overflow error
        if is_context_overflow_message(&error_msg) || is_context_overflow_message(&body) {
            return Err(AppError::ContextOverflow(error_msg));
        }

        return Err(AppError::Llm(error_msg));
    }

    let mut stream = response.bytes_stream();
    let mut parser = ThinkingParser::new();
    let mut line_buffer = String::new();
    let mut pending_tool_calls = BTreeMap::<usize, PendingOpenAiResponsesToolCall>::new();
    let mut tool_call_indices = HashMap::<String, usize>::new();
    let mut emitted_tool_call_ids = HashSet::<String>::new();
    let mut fallback_index = 0usize;

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(cancel_token.interruption_chunk()).await;
            return Ok(());
        }

        match chunk_result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in collect_complete_lines(&mut line_buffer, &text) {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    if data == "[DONE]" {
                        emit_ready_openai_responses_tool_calls(
                            &tx,
                            &mut pending_tool_calls,
                            &mut emitted_tool_call_ids,
                            true,
                        )
                        .await;
                        let _ = tx.send(StreamChunk::Done(None)).await;
                        return Ok(());
                    }

                    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
                        continue;
                    };

                    match json["type"].as_str().unwrap_or_default() {
                        "response.output_text.delta" => {
                            if let Some(delta) = json["delta"].as_str()
                                && !delta.is_empty()
                            {
                                for chunk in parser.process(delta) {
                                    let _ = tx.send(chunk).await;
                                }
                            }
                        }
                        "response.output_item.added" | "response.output_item.done" => {
                            if json["item"]["type"].as_str() == Some("function_call") {
                                merge_openai_responses_tool_item(
                                    &mut pending_tool_calls,
                                    &mut tool_call_indices,
                                    &json,
                                    fallback_index,
                                );
                                emit_ready_openai_responses_tool_calls(
                                    &tx,
                                    &mut pending_tool_calls,
                                    &mut emitted_tool_call_ids,
                                    false,
                                )
                                .await;
                            }
                        }
                        "response.function_call_arguments.delta" => {
                            merge_openai_responses_tool_argument_delta(
                                &mut pending_tool_calls,
                                &mut tool_call_indices,
                                &json,
                                fallback_index,
                            );
                        }
                        "response.function_call_arguments.done" => {
                            complete_openai_responses_tool_arguments(
                                &mut pending_tool_calls,
                                &mut tool_call_indices,
                                &json,
                                fallback_index,
                            );
                            emit_ready_openai_responses_tool_calls(
                                &tx,
                                &mut pending_tool_calls,
                                &mut emitted_tool_call_ids,
                                false,
                            )
                            .await;
                        }
                        "response.completed" => {
                            emit_ready_openai_responses_tool_calls(
                                &tx,
                                &mut pending_tool_calls,
                                &mut emitted_tool_call_ids,
                                true,
                            )
                            .await;
                            let _ = tx.send(StreamChunk::Done(None)).await;
                            return Ok(());
                        }
                        "response.failed" => {
                            let message = json["response"]["error"]["message"]
                                .as_str()
                                .unwrap_or("OpenAI Responses stream failed")
                                .to_string();
                            let _ = tx.send(StreamChunk::Error(message.clone())).await;
                            return Err(AppError::Llm(message));
                        }
                        _ => {}
                    }

                    fallback_index = fallback_index.saturating_add(1);
                }
            }
            Err(e) => {
                let _ = tx
                    .send(StreamChunk::Error(format!("Stream error: {}", e)))
                    .await;
                return Err(AppError::Llm(format!("Stream error: {}", e)));
            }
        }
    }

    emit_ready_openai_responses_tool_calls(
        &tx,
        &mut pending_tool_calls,
        &mut emitted_tool_call_ids,
        true,
    )
    .await;
    let _ = tx.send(StreamChunk::Done(None)).await;
    Ok(())
}

pub async fn stream_openai(
    api_key: &str,
    base_url: &str,
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    if is_openai_model_incompatible_with_agent_session(model) {
        return Err(AppError::Llm(openai_agent_session_model_message(model)));
    }

    match openai_api_for_model(model) {
        OpenAiApi::ChatCompletions => {
            stream_openai_chat_compatible(api_key, base_url, model, prompt, tools, tx, cancel_token)
                .await
        }
        OpenAiApi::Responses => {
            stream_openai_responses(api_key, base_url, model, prompt, tools, tx, cancel_token).await
        }
    }
}

/// Stream a response from Anthropic Claude API
///
/// This is an implementation detail used by `start_streaming(..)`.
/// To keep the API maintainable, we pass arguments via a struct.
#[derive(Debug)]
pub struct AnthropicStreamRequest<'a> {
    pub api_key: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
    pub thinking_budget_tokens: Option<u32>,
    pub prompt: &'a str,
    pub tools: Option<&'a [serde_json::Value]>,
    pub tx: mpsc::Sender<StreamChunk>,
    pub cancel_token: CancellationToken,
}

fn build_anthropic_messages_body(
    model: &str,
    prompt: &str,
    thinking_budget_tokens: Option<u32>,
    tools: Option<&[serde_json::Value]>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "messages": [{"role": "user", "content": [{"type": "text", "text": prompt}]}],
        "stream": true
    });

    // Enable structured tool calling when schemas are provided.
    if let Some(tools) = tools
        && !tools.is_empty()
    {
        body["tools"] = serde_json::Value::Array(tools.to_vec());
    }

    // Optional provider-native thinking stream (emitted as StreamChunk::Thinking).
    if let Some(budget_tokens) = thinking_budget_tokens {
        body["thinking"] = serde_json::json!({ "type": "enabled", "budget_tokens": budget_tokens });
    }

    body
}

pub async fn stream_anthropic(req: AnthropicStreamRequest<'_>) -> Result<(), AppError> {
    let AnthropicStreamRequest {
        api_key,
        base_url,
        model,
        thinking_budget_tokens,
        prompt,
        tools,
        tx,
        cancel_token,
    } = req;

    let url = format!("{}/v1/messages", base_url);
    let body = build_anthropic_messages_body(model, prompt, thinking_budget_tokens, tools);

    let client = create_streaming_client();
    let response = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Anthropic streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let error_msg = format!("Anthropic HTTP {}: {}", status, body);

        // Check if this is a context overflow error
        if is_context_overflow_message(&error_msg) || is_context_overflow_message(&body) {
            return Err(AppError::ContextOverflow(error_msg));
        }

        return Err(AppError::Llm(error_msg));
    }

    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();
    let mut parser = ThinkingParser::new();
    let mut in_tool_block = false;

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(cancel_token.interruption_chunk()).await;
            return Ok(());
        }

        match chunk_result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in collect_complete_lines(&mut line_buffer, &text) {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
                        continue;
                    };
                    match json["type"].as_str() {
                        Some("message_stop") => {
                            let _ = tx.send(StreamChunk::Done(None)).await;
                            return Ok(());
                        }
                        Some("content_block_delta") => {
                            if let Some(delta) = json["delta"].as_object() {
                                // Anthropic can stream both normal text and (optionally) extended thinking.
                                if let Some(content) = delta.get("text").and_then(|v| v.as_str())
                                    && !content.is_empty()
                                {
                                    for chunk in parser.process(content) {
                                        let _ = tx.send(chunk).await;
                                    }
                                }

                                if let Some(thinking) =
                                    delta.get("thinking").and_then(|v| v.as_str())
                                    && !thinking.is_empty()
                                {
                                    let _ =
                                        tx.send(StreamChunk::Thinking(thinking.to_string())).await;
                                }

                                if in_tool_block
                                    && let Some(partial_json) =
                                        delta.get("partial_json").and_then(|v| v.as_str())
                                    && !partial_json.is_empty()
                                {
                                    let _ = tx
                                        .send(StreamChunk::ToolCallArgs(partial_json.to_string()))
                                        .await;
                                }
                            }
                        }
                        Some("content_block_start") => {
                            // Anthropic SSE format: content_block IS the tool_use object:
                            //   {"type":"content_block_start","content_block":{"type":"tool_use","id":"toolu_xxx","name":"shell","input":{}}}
                            let block = &json["content_block"];
                            if block["type"].as_str() == Some("tool_use")
                                && let Some(name) = block["name"].as_str()
                            {
                                let id = block["id"].as_str().unwrap_or_default().to_string();
                                let _ = tx
                                    .send(StreamChunk::ToolCallStart {
                                        id,
                                        name: name.to_string(),
                                    })
                                    .await;
                                in_tool_block = true;
                            }
                        }
                        Some("content_block_stop") => {
                            if in_tool_block {
                                let _ = tx.send(StreamChunk::ToolCallEnd).await;
                                in_tool_block = false;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                let _ = tx
                    .send(StreamChunk::Error(format!("Stream error: {}", e)))
                    .await;
                return Err(AppError::Llm(format!("Stream error: {}", e)));
            }
        }
    }

    let _ = tx.send(StreamChunk::Done(None)).await;
    Ok(())
}

/// Build the JSON request body for Gemini streaming.
fn build_gemini_body(prompt: &str, tools: Option<&[serde_json::Value]>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "contents": [{"role": "user", "parts": [{"text": prompt}]}]
    });

    if let Some(tools) = tools
        && !tools.is_empty()
    {
        body["tools"] = serde_json::json!([{"functionDeclarations": tools}]);
        body["toolConfig"] = serde_json::json!({"functionCallingConfig": {"mode": "AUTO"}});
    }

    body
}

/// Stream a response from Google Gemini API (Generative Language API).
///
/// Gemini uses SSE with `alt=sse` query parameter and authenticates via API key
/// in the query string (not Bearer token).
pub async fn stream_gemini(
    api_key: &str,
    base_url: &str,
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!(
        "{}/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
        base_url, model, api_key
    );
    let body = build_gemini_body(prompt, tools);

    let client = create_streaming_client();
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Gemini streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let error_msg = format!("Gemini HTTP {}: {}", status, body);

        // Check if this is a context overflow error
        if is_context_overflow_message(&error_msg) || is_context_overflow_message(&body) {
            return Err(AppError::ContextOverflow(error_msg));
        }

        return Err(AppError::Llm(error_msg));
    }

    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();
    let mut parser = ThinkingParser::new();
    let mut last_usage: Option<TokenUsage> = None;

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(cancel_token.interruption_chunk()).await;
            return Ok(());
        }

        match chunk_result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in collect_complete_lines(&mut line_buffer, &text) {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
                        continue;
                    };

                    // Track usage metadata from each chunk (last one wins).
                    if let Some(usage) = json.get("usageMetadata") {
                        let input_tokens = usage["promptTokenCount"].as_u64().unwrap_or(0) as u32;
                        let output_tokens =
                            usage["candidatesTokenCount"].as_u64().unwrap_or(0) as u32;
                        last_usage = Some(
                            TokenUsage::new(input_tokens, output_tokens).with_provider("gemini"),
                        );
                    }

                    // Process candidate parts.
                    if let Some(parts) = json
                        .pointer("/candidates/0/content/parts")
                        .and_then(|v| v.as_array())
                    {
                        for (idx, part) in parts.iter().enumerate() {
                            // Text part
                            if let Some(content) = part["text"].as_str()
                                && !content.is_empty()
                            {
                                for chunk in parser.process(content) {
                                    let _ = tx.send(chunk).await;
                                }
                            }

                            // Function call part — Gemini sends complete function calls
                            // (not streamed arguments), so emit start + args + end.
                            if let Some(fc) = part.get("functionCall")
                                && let Some(name) = fc["name"].as_str()
                            {
                                let id = format!("gemini-call-{}", idx);
                                let _ = tx
                                    .send(StreamChunk::ToolCallStart {
                                        id,
                                        name: name.to_string(),
                                    })
                                    .await;

                                let args = fc
                                    .get("args")
                                    .map(|a| a.to_string())
                                    .unwrap_or_else(|| "{}".to_string());
                                let _ = tx.send(StreamChunk::ToolCallArgs(args)).await;
                                let _ = tx.send(StreamChunk::ToolCallEnd).await;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx
                    .send(StreamChunk::Error(format!("Stream error: {}", e)))
                    .await;
                return Err(AppError::Llm(format!("Stream error: {}", e)));
            }
        }
    }

    let _ = tx.send(StreamChunk::Done(last_usage)).await;
    Ok(())
}

/// Keepalive interval for Ollama streaming.
///
/// Ollama may take a long time to load a model into memory (especially large
/// models). Additionally, when a model decides to make a tool call, the entire
/// tool call JSON only appears in the final `done:true` NDJSON line — no
/// individual tokens are streamed during that deliberation phase. Both of these
/// situations produce silence on the wire that can trigger the caller's idle
/// timer. We send periodic `Status` keepalive chunks throughout the **entire**
/// stream lifetime (not only during model loading) to prevent premature
/// timeouts.
const OLLAMA_KEEPALIVE_INTERVAL_SECS: u64 = 30;

/// Stream a response from Ollama local API
pub async fn stream_ollama(
    base_url: &str,
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": true
    });

    // Ollama uses OpenAI-compatible tool schema format.
    if let Some(tools) = tools
        && !tools.is_empty()
    {
        body["tools"] = serde_json::Value::Array(tools.to_vec());
    }

    tracing::debug!(
        model = model,
        url = %url,
        tools_count = tools.map(|t| t.len()).unwrap_or(0),
        has_tools = tools.map(|t| !t.is_empty()).unwrap_or(false),
        "[Ollama] Starting stream request"
    );

    let client = create_streaming_client();

    // Pre-connection keepalive: Ollama must load the model into VRAM before it
    // can start streaming. For large models (e.g. 20B+) this can take 100–200s,
    // which exceeds the GUI's 90s idle timer. We spawn a background task that
    // sends a Status chunk every 30s while reqwest's .send().await is blocking,
    // so the idle timer keeps getting reset during the model-loading phase.
    let pre_conn_tx = tx.clone();
    let pre_conn_model = model.to_string();
    let pre_conn_handle = tokio::spawn(
        {
            let interval = Duration::from_secs(OLLAMA_KEEPALIVE_INTERVAL_SECS);
            async move {
                loop {
                    tokio::time::sleep(interval).await;
                    tracing::debug!(
                        model = %pre_conn_model,
                        "[Ollama] Pre-connection keepalive: model still loading"
                    );
                    send_status_chunk_best_effort(
                        &pre_conn_tx,
                        StreamChunk::Status {
                            message: format!("Loading model '{pre_conn_model}'…"),
                        },
                    )
                    .await;
                }
            }
        }
        .instrument(tracing::Span::current()),
    );

    let send_result = client.post(&url).json(&body).send().await;

    // Abort the pre-connection keepalive now that we have a response (or error).
    // abort() is instant; the subsequent await just confirms the task has stopped.
    pre_conn_handle.abort();
    let _ = pre_conn_handle.await;

    let response = send_result
        .map_err(|e| AppError::Llm(format!("Ollama streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let error_msg = format!("Ollama HTTP {}: {}", status, body);

        // Check if this is a context overflow error
        if is_context_overflow_message(&error_msg) || is_context_overflow_message(&body) {
            return Err(AppError::ContextOverflow(error_msg));
        }

        return Err(AppError::Llm(error_msg));
    }

    // Immediately notify the caller that we have a connection. This resets
    // the caller's idle timer, which is critical because Ollama may spend a
    // long time loading the model into memory before sending any tokens.
    send_status_chunk_best_effort(
        &tx,
        StreamChunk::Status {
            message: format!("Connected to Ollama — loading model '{}'…", model),
        },
    )
    .await;
    tracing::debug!(
        model = model,
        "[Ollama] HTTP connection established; 'Connected' status sent"
    );

    let mut stream = response.bytes_stream();
    let mut parser = ThinkingParser::new();
    let mut line_buffer = String::new();

    let keepalive_interval = Duration::from_secs(OLLAMA_KEEPALIVE_INTERVAL_SECS);
    let keepalive_sleep = tokio::time::sleep(keepalive_interval);
    tokio::pin!(keepalive_sleep);

    loop {
        tokio::select! {
            maybe_chunk = stream.next() => {
                let Some(chunk_result) = maybe_chunk else {
                    // Stream ended
                    break;
                };

                if cancel_token.is_cancelled() {
                    let _ = tx.send(cancel_token.interruption_chunk()).await;
                    return Ok(());
                }

                match chunk_result {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        for line in collect_complete_lines(&mut line_buffer, &text) {
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                                tracing::trace!(
                                    done = json["done"].as_bool().unwrap_or(false),
                                    has_content = json["message"]["content"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
                                    has_tool_calls = json["message"]["tool_calls"].is_array(),
                                    "[Ollama] NDJSON line parsed"
                                );

                                // Extract content from message (may arrive before done)
                                if let Some(content) = json["message"]["content"].as_str()
                                    && !content.is_empty()
                                {
                                    let chunks = parser.process(content);
                                    for chunk in chunks {
                                        let _ = tx.send(chunk).await;
                                    }
                                }

                                // Handle tool calls (Ollama returns them in the message)
                                if let Some(tool_calls) = json["message"]["tool_calls"].as_array() {
                                    tracing::debug!(
                                        model = model,
                                        count = tool_calls.len(),
                                        "[Ollama] Tool calls found in NDJSON line"
                                    );
                                    for call in tool_calls {
                                        let name = call["function"]["name"].as_str().unwrap_or_default();
                                        let args = &call["function"]["arguments"];

                                        if !name.is_empty() {
                                            tracing::debug!(
                                                tool = name,
                                                "[Ollama] Emitting ToolCallStart/Args/End"
                                            );
                                            let id = format!("ollama-tool-{}", uuid::Uuid::new_v4());
                                            let _ = tx
                                                .send(StreamChunk::ToolCallStart {
                                                    id,
                                                    name: name.to_string(),
                                                })
                                                .await;

                                            let args_str = if args.is_object() || args.is_array() {
                                                serde_json::to_string(args).unwrap_or_default()
                                            } else {
                                                args.as_str().unwrap_or("{}").to_string()
                                            };

                                            let _ = tx.send(StreamChunk::ToolCallArgs(args_str)).await;
                                            let _ = tx.send(StreamChunk::ToolCallEnd).await;
                                            tracing::debug!(tool = name, "[Ollama] ToolCallEnd emitted");
                                        }
                                    }
                                }

                                // Check if done
                                if json["done"].as_bool() == Some(true) {
                                    tracing::debug!(model = model, "[Ollama] done=true — sending Done chunk");
                                    let _ = tx.send(StreamChunk::Done(None)).await;
                                    return Ok(());
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(StreamChunk::Error(format!("Stream error: {}", e)))
                            .await;
                        return Err(AppError::Llm(format!("Stream error: {}", e)));
                    }
                }
            }

            // Keepalive: send periodic status messages throughout the entire
            // stream. This prevents the caller's idle timer (typically 90 s)
            // from firing during two distinct silent phases:
            //   1. Cold starts — Ollama loads large models into memory before
            //      sending any tokens.
            //   2. Tool-call generation — Ollama only sends a tool call in the
            //      final `done:true` NDJSON line. The model deliberates in
            //      silence before that line arrives, which can easily exceed
            //      90 s on a local large model.
            () = &mut keepalive_sleep => {
                if cancel_token.is_cancelled() {
                    let _ = tx.send(cancel_token.interruption_chunk()).await;
                    return Ok(());
                }
                tracing::debug!(model = model, "[Ollama] Keepalive firing — sending Status chunk");
                send_status_chunk_best_effort(
                    &tx,
                    StreamChunk::Status {
                        message: format!("Working… (model '{}')", model),
                    },
                )
                .await;
                tracing::debug!(
                    model = model,
                    "[Ollama] Keepalive Status sent"
                );
                // Reset the keepalive timer for the next interval.
                keepalive_sleep
                    .as_mut()
                    .reset(tokio::time::Instant::now() + keepalive_interval);
            }
        }
    }

    let _ = tx.send(StreamChunk::Done(None)).await;
    Ok(())
}

/// Emit an error when the LLM provider is not configured.
///
/// Sends a `Status` message (which does not count as "output" for retry gating)
/// followed by an `Error` chunk, then returns an error to the caller.
async fn stream_unconfigured_error(
    provider_name: &str,
    tx: mpsc::Sender<StreamChunk>,
) -> Result<(), AppError> {
    let message = format!(
        "LLM provider '{}' is not configured. Please configure it in Settings or run 'gestura config edit'.",
        provider_name
    );
    // Status chunk does not count as output for retry purposes
    send_status_chunk_best_effort(
        &tx,
        StreamChunk::Status {
            message: message.clone(),
        },
    )
    .await;
    let _ = tx.send(StreamChunk::Error(message.clone())).await;
    Err(AppError::Llm(message))
}

/// Returns `true` if a message indicates the provider is not configured.
///
/// We use this to skip pointless retry delays when failure is caused solely by
/// missing local configuration (e.g., absent API key).
fn is_unconfigured_provider_message(message: &str) -> bool {
    message.contains("is not configured") || message.contains("not configured")
}

/// Returns `true` if an error message indicates a context length overflow.
///
/// These errors cannot be fixed by blind retries - they require context compaction
/// or switching to a model with a larger context window.
fn is_context_overflow_message(message: &str) -> bool {
    let msg_lower = message.to_lowercase();
    // OpenAI format: "contextlengthexceeded" (no underscore in JSON code field)
    // or "context_length_exceeded" (with underscore in some error messages)
    let is_overflow = msg_lower.contains("contextlengthexceeded")
        || msg_lower.contains("context_length_exceeded")
        || msg_lower.contains("context length")
        || msg_lower.contains("maximum context")
        || (msg_lower.contains("tokens") && msg_lower.contains("exceeds"))
        || (msg_lower.contains("token") && msg_lower.contains("limit"));

    if is_overflow {
        tracing::warn!(
            message_preview = %message.chars().take(200).collect::<String>(),
            "Detected context overflow error"
        );
    }

    is_overflow
}

/// Returns `true` if an [`AppError`] indicates a provider is not configured.
fn is_unconfigured_provider_error(err: &AppError) -> bool {
    match err {
        AppError::Llm(msg) => is_unconfigured_provider_message(msg),
        _ => false,
    }
}

/// Start a streaming LLM request based on config.
///
/// Returns an error if the selected provider is not configured.
pub async fn start_streaming(
    config: &StreamingConfig,
    prompt: &str,
    tool_schemas: Option<ProviderToolSchemas>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    async {
        match config.primary.as_str() {
            "openai" => {
                if let Some(c) = &config.openai {
                    let openai_tools =
                        tool_schemas
                            .as_ref()
                            .map(|schemas| match openai_api_for_model(&c.model) {
                                OpenAiApi::ChatCompletions => schemas.openai.as_slice(),
                                OpenAiApi::Responses => schemas.openai_responses.as_slice(),
                            });
                    stream_openai(
                        &c.api_key,
                        c.base_url.as_deref().unwrap_or("https://api.openai.com"),
                        &c.model,
                        prompt,
                        openai_tools,
                        tx,
                        cancel_token,
                    )
                    .await
                } else {
                    stream_unconfigured_error("openai", tx).await
                }
            }
            "anthropic" => {
                if let Some(c) = &config.anthropic {
                    stream_anthropic(AnthropicStreamRequest {
                        api_key: &c.api_key,
                        base_url: c.base_url.as_deref().unwrap_or("https://api.anthropic.com"),
                        model: &c.model,
                        thinking_budget_tokens: c.thinking_budget_tokens,
                        prompt,
                        tools: tool_schemas.as_ref().map(|s| s.anthropic.as_slice()),
                        tx,
                        cancel_token,
                    })
                    .await
                } else {
                    stream_unconfigured_error("anthropic", tx).await
                }
            }
            "grok" => {
                // Grok uses OpenAI-compatible API
                if let Some(c) = &config.grok {
                    stream_openai_chat_compatible(
                        &c.api_key,
                        c.base_url.as_deref().unwrap_or("https://api.x.ai"),
                        &c.model,
                        prompt,
                        tool_schemas.as_ref().map(|s| s.openai.as_slice()),
                        tx,
                        cancel_token,
                    )
                    .await
                } else {
                    stream_unconfigured_error("grok", tx).await
                }
            }
            "gemini" => {
                if let Some(c) = &config.gemini {
                    stream_gemini(
                        &c.api_key,
                        c.base_url
                            .as_deref()
                            .unwrap_or("https://generativelanguage.googleapis.com"),
                        &c.model,
                        prompt,
                        tool_schemas.as_ref().map(|s| s.gemini.as_slice()),
                        tx,
                        cancel_token,
                    )
                    .await
                } else {
                    stream_unconfigured_error("gemini", tx).await
                }
            }
            "ollama" => {
                if let Some(c) = &config.ollama {
                    stream_ollama(
                        &c.base_url,
                        &c.model,
                        prompt,
                        tool_schemas.as_ref().map(|s| s.openai.as_slice()),
                        tx,
                        cancel_token,
                    )
                    .await
                } else {
                    stream_unconfigured_error("ollama", tx).await
                }
            }
            other => stream_unconfigured_error(other, tx).await,
        }
    }
    .instrument(tracing::info_span!(
        "agent.streaming.request",
        provider = %config.primary,
        has_tool_schemas = tool_schemas.is_some()
    ))
    .await
}

/// Start streaming with fallback to secondary provider on failure
/// Implements jittered exponential backoff with rate-limit-aware delay selection before falling back.
pub async fn start_streaming_with_fallback(
    config: &StreamingConfig,
    prompt: &str,
    tool_schemas: Option<ProviderToolSchemas>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    // Try primary provider with retries
    let retry_policy = RetryPolicy::for_streaming();
    let total_attempts = retry_policy.max_attempts.max(1) as usize;
    let mut last_error: Option<AppError> = None;
    let mut skipped_retries_due_to_unconfigured = false;

    for attempt in 0..total_attempts {
        if cancel_token.is_cancelled() {
            let _ = tx.send(cancel_token.interruption_chunk()).await;
            return Ok(());
        }

        // Create a new channel for this attempt
        let (attempt_tx, mut attempt_rx) =
            mpsc::channel::<StreamChunk>(STREAM_CHUNK_BUFFER_CAPACITY);
        let attempt_cancel = cancel_token.clone();
        let config_clone = config.clone();
        let prompt_clone = prompt.to_string();
        let tool_schemas_clone = tool_schemas.clone();

        // Spawn the streaming attempt
        let attempt_span = tracing::info_span!(
            "agent.streaming.fallback_attempt",
            attempt = attempt + 1,
            total_attempts = total_attempts
        );
        let handle = tokio::spawn(
            async move {
                start_streaming(
                    &config_clone,
                    &prompt_clone,
                    tool_schemas_clone,
                    attempt_tx,
                    attempt_cancel,
                )
                .await
            }
            .instrument(attempt_span),
        );

        // Forward chunks to the caller in real-time.
        // If the attempt fails before producing any output, we can retry.
        let forward = forward_attempt_stream(&mut attempt_rx, &tx).await;

        // Wait for the task to complete (capture errors that might occur before any chunk arrives).
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                last_error = Some(e);
            }
            Err(e) => {
                last_error = Some(AppError::Llm(format!("Task failed: {}", e)));
            }
        }

        match forward.outcome {
            AttemptOutcome::Success => return Ok(()),
            AttemptOutcome::Cancelled | AttemptOutcome::Paused => return Ok(()),
            AttemptOutcome::FatalError => {
                let err = AppError::Llm(
                    forward
                        .error
                        .clone()
                        .unwrap_or_else(|| "Streaming failed".to_string()),
                );
                return Err(err);
            }
            AttemptOutcome::ContextOverflowError => {
                // Context overflow cannot be fixed by retry - caller must compact context.
                // Return immediately with a specific error so the pipeline can handle it.
                let error_msg = forward
                    .error
                    .clone()
                    .unwrap_or_else(|| "Context length exceeded".to_string());

                tracing::warn!(
                    error = %error_msg,
                    "Context overflow detected - returning to pipeline for compaction"
                );

                // Emit a special chunk so the pipeline knows to compact
                let _ = tx
                    .send(StreamChunk::ContextOverflow {
                        error_message: error_msg.clone(),
                    })
                    .await;

                return Err(AppError::ContextOverflow(error_msg));
            }
            AttemptOutcome::RetryableError => {
                if let Some(ref e) = forward.error {
                    last_error = Some(AppError::Llm(e.clone()));
                }
            }
            AttemptOutcome::UnexpectedEnd => {
                if forward.forwarded_output {
                    // We streamed partial output but never got a terminal event; treat as fatal.
                    let err = AppError::Llm(
                        "Streaming ended unexpectedly (no terminal event received)".to_string(),
                    );
                    let _ = tx.send(StreamChunk::Error(err.to_string())).await;
                    return Err(err);
                }
                // Otherwise, allow retry (error may be captured from handle.await above).
            }
        }

        // If the provider is simply not configured, retries won't help.
        // Skip backoff and jump directly to fallback (if configured).
        let unconfigured = forward
            .error
            .as_deref()
            .map(is_unconfigured_provider_message)
            .unwrap_or(false)
            || last_error
                .as_ref()
                .map(is_unconfigured_provider_error)
                .unwrap_or(false);

        if unconfigured {
            skipped_retries_due_to_unconfigured = true;
            break;
        }

        // Context overflow errors require compaction, not blind retries.
        // Return immediately so the pipeline can compact and retry.
        let is_context_overflow = forward
            .error
            .as_deref()
            .map(is_context_overflow_message)
            .unwrap_or(false)
            || matches!(&last_error, Some(AppError::ContextOverflow(_)));

        if is_context_overflow {
            let error_msg = forward
                .error
                .clone()
                .or_else(|| last_error.as_ref().map(|e| e.to_string()))
                .unwrap_or_else(|| "Context length exceeded".to_string());

            tracing::warn!(
                error = %error_msg,
                "Context overflow detected - skipping retries, returning for compaction"
            );

            // Emit context overflow chunk so UI knows what's happening
            let _ = tx
                .send(StreamChunk::ContextOverflow {
                    error_message: error_msg.clone(),
                })
                .await;

            return Err(AppError::ContextOverflow(error_msg));
        }

        // Only back off if we will actually perform another attempt.
        if attempt + 1 < total_attempts {
            // Log retry attempt and notify frontend
            let error_msg = last_error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string());
            let retry_delay =
                select_streaming_retry_delay(&retry_policy, attempt as u32 + 1, &error_msg);

            tracing::warn!(
                attempt = attempt + 1,
                delay_ms = retry_delay.as_millis(),
                error = %error_msg,
                "Primary LLM failed, retrying after backoff"
            );

            // Emit retry notification to frontend
            let _ = tx
                .send(StreamChunk::RetryAttempt {
                    attempt: attempt as u32 + 1,
                    max_attempts: total_attempts as u32,
                    delay_ms: retry_delay.as_millis() as u64,
                    error_message: error_msg,
                })
                .await;

            tokio::time::sleep(retry_delay).await;
        }
    }

    // Primary failed after retries, try fallback if configured
    if let Some(ref fallback_provider) = config.fallback {
        if skipped_retries_due_to_unconfigured {
            tracing::info!(
                fallback = fallback_provider,
                "Primary LLM is not configured, trying fallback provider"
            );
        } else {
            tracing::info!(
                fallback = fallback_provider,
                "Primary LLM exhausted retries, trying fallback provider"
            );
        }

        // Create a modified config with fallback as primary
        let mut fallback_config = config.clone();
        fallback_config.primary = fallback_provider.clone();

        // Try fallback provider (no retries for fallback)
        let result = start_streaming(
            &fallback_config,
            prompt,
            tool_schemas,
            tx.clone(),
            cancel_token,
        )
        .await;

        if result.is_ok() {
            return Ok(());
        }

        tracing::error!("Fallback provider also failed");
    }

    // All attempts failed
    if let Some(error) = last_error {
        let _ = tx.send(StreamChunk::Error(error.to_string())).await;
        Err(error)
    } else {
        let err = AppError::Llm("All LLM providers failed".to_string());
        let _ = tx.send(StreamChunk::Error(err.to_string())).await;
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_http_error_includes_retry_after_hint_when_present() {
        let message = format_openai_http_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "OpenAI",
            "gpt-5.4",
            OpenAiApi::Responses,
            "rate limit reached",
            Some(Duration::from_secs(12)),
        );

        assert!(message.contains("HTTP 429"));
        assert!(message.contains("retrying after 12 seconds"));
    }

    #[test]
    fn retry_delay_prefers_provider_retry_after_hint() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_delay_ms: 1_000,
            max_delay_ms: 8_000,
            backoff_multiplier: 2.0,
            jitter_factor: 0.0,
        };

        let delay = select_streaming_retry_delay(
            &policy,
            1,
            "OpenAI /v1/responses HTTP 429: rate limit reached. Provider suggested retrying after 12 seconds.",
        );

        assert_eq!(delay, Duration::from_secs(12));
    }

    #[test]
    fn retry_delay_uses_rate_limit_floor_without_retry_after_hint() {
        let policy = RetryPolicy {
            max_attempts: 3,
            initial_delay_ms: 1_000,
            max_delay_ms: 8_000,
            backoff_multiplier: 2.0,
            jitter_factor: 0.0,
        };

        let delay = select_streaming_retry_delay(
            &policy,
            1,
            "OpenAI /v1/responses HTTP 429: Too many requests",
        );

        assert_eq!(delay, Duration::from_secs(5));
    }

    #[test]
    fn test_cancellation_token() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
        assert!(!token.is_pause_requested());
        assert!(matches!(token.interruption_chunk(), StreamChunk::Cancelled));
    }

    #[test]
    fn test_cancellation_token_pause_intent() {
        let token = CancellationToken::new();
        token.pause();

        assert!(token.is_cancelled());
        assert!(token.is_pause_requested());
        assert!(matches!(token.interruption_chunk(), StreamChunk::Paused));

        token.cancel();
        assert!(token.is_cancelled());
        assert!(!token.is_pause_requested());
        assert!(matches!(token.interruption_chunk(), StreamChunk::Cancelled));
    }

    #[test]
    fn split_think_blocks_extracts_thinking() {
        let input = "<think>plan</think>answer";
        let (content, thinking) = split_think_blocks(input);
        assert_eq!(content, "answer");
        assert_eq!(thinking.as_deref(), Some("plan"));
    }

    #[test]
    fn thinking_parser_handles_complete_tags() {
        let mut parser = ThinkingParser::new();
        let chunks = parser.process("<think>thinking content</think>response text");

        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::Thinking(t) if t == "thinking content"));
        assert!(matches!(&chunks[1], StreamChunk::Text(t) if t == "response text"));
    }

    #[test]
    fn thinking_parser_handles_split_start_tag() {
        let mut parser = ThinkingParser::new();

        // First chunk ends with partial "<think>"
        let chunks1 = parser.process("Hello <thi");
        assert_eq!(chunks1.len(), 1);
        assert!(matches!(&chunks1[0], StreamChunk::Text(t) if t == "Hello "));

        // Second chunk completes the tag
        let chunks2 = parser.process("nk>thinking</think>done");
        assert_eq!(chunks2.len(), 2);
        assert!(matches!(&chunks2[0], StreamChunk::Thinking(t) if t == "thinking"));
        assert!(matches!(&chunks2[1], StreamChunk::Text(t) if t == "done"));
    }

    #[test]
    fn thinking_parser_handles_split_end_tag() {
        let mut parser = ThinkingParser::new();

        // First chunk has start tag and partial end tag
        let chunks1 = parser.process("<think>thinking content</th");
        assert_eq!(chunks1.len(), 1);
        assert!(matches!(&chunks1[0], StreamChunk::Thinking(t) if t == "thinking content"));

        // Second chunk completes the end tag
        let chunks2 = parser.process("ink>response");
        assert_eq!(chunks2.len(), 1);
        assert!(matches!(&chunks2[0], StreamChunk::Text(t) if t == "response"));
    }

    #[test]
    fn thinking_parser_handles_text_before_think() {
        let mut parser = ThinkingParser::new();
        let chunks = parser.process("prefix<think>thought</think>suffix");

        assert_eq!(chunks.len(), 3);
        assert!(matches!(&chunks[0], StreamChunk::Text(t) if t == "prefix"));
        assert!(matches!(&chunks[1], StreamChunk::Thinking(t) if t == "thought"));
        assert!(matches!(&chunks[2], StreamChunk::Text(t) if t == "suffix"));
    }

    #[test]
    fn thinking_parser_handles_no_think_tags() {
        let mut parser = ThinkingParser::new();
        let chunks = parser.process("just regular text");

        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Text(t) if t == "just regular text"));
    }

    #[test]
    fn openai_body_includes_tools_and_tool_choice_when_provided() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Run a command",
                "parameters": {"type": "object", "properties": {}}
            }
        })];

        let body = build_openai_chat_request_body("gpt-test", "hi", Some(&tools));
        assert!(body.get("tools").is_some());
        assert_eq!(
            body.get("tool_choice").and_then(|v| v.as_str()),
            Some("auto")
        );
    }

    #[test]
    fn openai_body_omits_tools_when_none() {
        let body = build_openai_chat_request_body("gpt-test", "hi", None);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn openai_body_omits_temperature() {
        let body = build_openai_chat_request_body("gpt-test", "hi", None);
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn openai_responses_body_uses_responses_shape() {
        let tools = vec![serde_json::json!({
            "type": "function",
            "name": "shell",
            "description": "Run a command",
            "parameters": {"type": "object", "properties": {}}
        })];

        let body = build_openai_responses_request_body("gpt-5.4", "hi", Some(&tools));
        assert_eq!(body["model"], "gpt-5.4");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"], "hi");
        assert!(body.get("tools").is_some());
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn openai_http_error_mentions_selected_endpoint() {
        let message = format_openai_http_error(
            reqwest::StatusCode::NOT_FOUND,
            "OpenAI",
            "gpt-5.3-codex",
            OpenAiApi::ChatCompletions,
            "This is not a chat model",
            None,
        );
        assert!(message.contains("/v1/responses"));
        assert!(message.contains("/v1/chat/completions"));
    }

    #[test]
    fn openai_tool_call_deltas_are_assembled_by_index() {
        let mut pending = BTreeMap::new();

        merge_openai_tool_call_delta(
            &mut pending,
            &serde_json::json!({
                "index": 0,
                "id": "call_0",
                "function": {"name": "task", "arguments": "{\"operation\":\"update_status\",\"task_id\":\"abc"}
            }),
            0,
        );
        merge_openai_tool_call_delta(
            &mut pending,
            &serde_json::json!({
                "index": 1,
                "id": "call_1",
                "function": {"name": "shell", "arguments": "{\"command\":\"cargo check"}
            }),
            1,
        );
        merge_openai_tool_call_delta(
            &mut pending,
            &serde_json::json!({
                "index": 0,
                "function": {"arguments": "\",\"status\":\"completed\"}"}
            }),
            0,
        );
        merge_openai_tool_call_delta(
            &mut pending,
            &serde_json::json!({
                "index": 1,
                "function": {"arguments": "\",\"timeout_secs\":300}"}
            }),
            1,
        );

        let calls = take_openai_tool_calls(&mut pending);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, 0);
        assert_eq!(calls[0].1.id, "call_0");
        assert_eq!(calls[0].1.name, "task");
        assert_eq!(
            calls[0].1.arguments,
            "{\"operation\":\"update_status\",\"task_id\":\"abc\",\"status\":\"completed\"}"
        );
        assert_eq!(calls[1].0, 1);
        assert_eq!(calls[1].1.id, "call_1");
        assert_eq!(calls[1].1.name, "shell");
        assert_eq!(
            calls[1].1.arguments,
            "{\"command\":\"cargo check\",\"timeout_secs\":300}"
        );
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn emit_openai_tool_calls_streams_complete_calls_in_index_order() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut pending = BTreeMap::new();
        pending.insert(
            1,
            PendingOpenAiToolCall {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                arguments: "{\"command\":\"pwd\"}".to_string(),
            },
        );
        pending.insert(
            0,
            PendingOpenAiToolCall {
                id: "call_0".to_string(),
                name: "file".to_string(),
                arguments: "{\"operation\":\"list\"}".to_string(),
            },
        );

        emit_openai_tool_calls(&tx, &mut pending).await;

        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallStart { id, name }) if id == "call_0" && name == "file"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallArgs(args)) if args == "{\"operation\":\"list\"}"
        ));
        assert!(matches!(rx.recv().await, Some(StreamChunk::ToolCallEnd)));
        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallStart { id, name }) if id == "call_1" && name == "shell"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallArgs(args)) if args == "{\"command\":\"pwd\"}"
        ));
        assert!(matches!(rx.recv().await, Some(StreamChunk::ToolCallEnd)));
        assert!(pending.is_empty());
    }

    #[test]
    fn openai_responses_tool_calls_are_buffered_by_output_index() {
        let mut pending = BTreeMap::new();
        let mut tool_indices = HashMap::new();

        merge_openai_responses_tool_item(
            &mut pending,
            &mut tool_indices,
            &serde_json::json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "id": "fc_0",
                    "call_id": "call_0",
                    "name": "file"
                }
            }),
            0,
        );
        merge_openai_responses_tool_argument_delta(
            &mut pending,
            &mut tool_indices,
            &serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "output_index": 0,
                "item_id": "fc_0",
                "delta": "{\"operation\":\"list\"}"
            }),
            0,
        );
        complete_openai_responses_tool_arguments(
            &mut pending,
            &mut tool_indices,
            &serde_json::json!({
                "type": "response.function_call_arguments.done",
                "output_index": 0,
                "item_id": "fc_0",
                "arguments": "{\"operation\":\"list\"}"
            }),
            0,
        );

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[&0].id, "call_0");
        assert_eq!(pending[&0].name, "file");
        assert_eq!(pending[&0].arguments, "{\"operation\":\"list\"}");
        assert!(pending[&0].finished);
    }

    #[test]
    fn openai_responses_tool_calls_reuse_stable_aliases_when_output_index_is_missing() {
        let mut pending = BTreeMap::new();
        let mut tool_indices = HashMap::new();

        merge_openai_responses_tool_item(
            &mut pending,
            &mut tool_indices,
            &serde_json::json!({
                "type": "response.output_item.added",
                "item": {
                    "type": "function_call",
                    "id": "fc_0",
                    "call_id": "call_0",
                    "name": "file"
                }
            }),
            3,
        );
        merge_openai_responses_tool_argument_delta(
            &mut pending,
            &mut tool_indices,
            &serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc_0",
                "delta": "{\"operation\":\"list\"}"
            }),
            8,
        );
        complete_openai_responses_tool_arguments(
            &mut pending,
            &mut tool_indices,
            &serde_json::json!({
                "type": "response.function_call_arguments.done",
                "call_id": "call_0",
                "arguments": "{\"operation\":\"list\"}"
            }),
            13,
        );

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[&3].id, "call_0");
        assert_eq!(pending[&3].arguments, "{\"operation\":\"list\"}");
        assert!(pending[&3].finished);
    }

    #[tokio::test]
    async fn emit_openai_responses_tool_calls_waits_for_lowest_ready_index() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut pending = BTreeMap::new();
        let mut emitted_ids = HashSet::new();
        pending.insert(
            0,
            PendingOpenAiResponsesToolCall {
                id: "call_0".to_string(),
                name: "file".to_string(),
                arguments: "{\"operation\":\"list\"}".to_string(),
                finished: true,
            },
        );
        pending.insert(
            1,
            PendingOpenAiResponsesToolCall {
                id: "call_1".to_string(),
                name: "shell".to_string(),
                arguments: "{\"command\":\"pwd\"}".to_string(),
                finished: true,
            },
        );

        emit_ready_openai_responses_tool_calls(&tx, &mut pending, &mut emitted_ids, false).await;

        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallStart { id, name }) if id == "call_0" && name == "file"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallArgs(args)) if args == "{\"operation\":\"list\"}"
        ));
        assert!(matches!(rx.recv().await, Some(StreamChunk::ToolCallEnd)));
        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallStart { id, name }) if id == "call_1" && name == "shell"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallArgs(args)) if args == "{\"command\":\"pwd\"}"
        ));
        assert!(matches!(rx.recv().await, Some(StreamChunk::ToolCallEnd)));
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn emit_openai_responses_tool_calls_skips_duplicate_call_ids() {
        let (tx, mut rx) = mpsc::channel(10);
        let mut pending = BTreeMap::new();
        let mut emitted_ids = HashSet::new();
        pending.insert(
            0,
            PendingOpenAiResponsesToolCall {
                id: "call_dup".to_string(),
                name: "file".to_string(),
                arguments: "{\"operation\":\"list\"}".to_string(),
                finished: true,
            },
        );
        pending.insert(
            1,
            PendingOpenAiResponsesToolCall {
                id: "call_dup".to_string(),
                name: "file".to_string(),
                arguments: "{\"operation\":\"list\"}".to_string(),
                finished: true,
            },
        );

        emit_ready_openai_responses_tool_calls(&tx, &mut pending, &mut emitted_ids, false).await;

        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallStart { id, name }) if id == "call_dup" && name == "file"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamChunk::ToolCallArgs(args)) if args == "{\"operation\":\"list\"}"
        ));
        assert!(matches!(rx.recv().await, Some(StreamChunk::ToolCallEnd)));
        assert!(rx.try_recv().is_err());
        assert!(pending.is_empty());
    }

    #[test]
    fn anthropic_body_includes_tools_when_provided() {
        let tools = vec![serde_json::json!({
            "name": "shell",
            "description": "Run a command",
            "input_schema": {"type": "object", "properties": {}}
        })];

        let body = build_anthropic_messages_body("claude-test", "hi", None, Some(&tools));
        assert!(body.get("tools").is_some());
    }

    #[tokio::test]
    async fn test_stream_chunk_types() {
        let (tx, mut rx) = mpsc::channel(10);

        tx.send(StreamChunk::Text("Hello".to_string()))
            .await
            .unwrap();
        tx.send(StreamChunk::Done(None)).await.unwrap();

        if let Some(StreamChunk::Text(text)) = rx.recv().await {
            assert_eq!(text, "Hello");
        } else {
            panic!("Expected Text chunk");
        }

        if let Some(StreamChunk::Done(_)) = rx.recv().await {
            // OK
        } else {
            panic!("Expected Done chunk");
        }
    }

    #[tokio::test]
    async fn start_streaming_unconfigured_provider_returns_error() {
        let cfg = StreamingConfig {
            primary: "openai".to_string(),
            openai: None,
            ..Default::default()
        };

        let (tx, mut rx) = mpsc::channel(128);
        let cancel = CancellationToken::new();

        tokio::spawn(async move {
            let prompt = "hello world";
            let _ = start_streaming(&cfg, prompt, None, tx, cancel).await;
        });

        // First chunk should be a Status message (does not count as output for retry).
        match rx.recv().await {
            Some(StreamChunk::Status { message }) => {
                assert!(message.contains("not configured"));
            }
            other => panic!("Expected Status chunk, got: {other:?}"),
        }

        // Next chunk should be an Error.
        match rx.recv().await {
            Some(StreamChunk::Error(msg)) => {
                assert!(msg.contains("not configured"));
            }
            other => panic!("Expected Error chunk, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn forward_attempt_stream_forwards_immediately() {
        let (outer_tx, mut outer_rx) = mpsc::channel::<StreamChunk>(10);
        let (attempt_tx, mut attempt_rx) = mpsc::channel::<StreamChunk>(10);

        let forward_handle =
            tokio::spawn(async move { forward_attempt_stream(&mut attempt_rx, &outer_tx).await });

        attempt_tx
            .send(StreamChunk::Text("A".to_string()))
            .await
            .unwrap();

        // If forwarding is real-time, we should observe the text chunk before we send Done.
        match outer_rx.recv().await {
            Some(StreamChunk::Text(t)) => assert_eq!(t, "A"),
            other => panic!("Expected Text chunk, got: {other:?}"),
        }

        attempt_tx.send(StreamChunk::Done(None)).await.unwrap();
        match outer_rx.recv().await {
            Some(StreamChunk::Done(_)) => {}
            other => panic!("Expected Done chunk, got: {other:?}"),
        }

        let result = forward_handle.await.unwrap();
        assert_eq!(result.outcome, AttemptOutcome::Success);
    }

    #[tokio::test]
    async fn forward_attempt_stream_retryable_error_before_output_is_not_forwarded() {
        let (outer_tx, mut outer_rx) = mpsc::channel::<StreamChunk>(10);
        let (attempt_tx, mut attempt_rx) = mpsc::channel::<StreamChunk>(10);

        let forward_handle =
            tokio::spawn(async move { forward_attempt_stream(&mut attempt_rx, &outer_tx).await });

        attempt_tx
            .send(StreamChunk::Error("nope".to_string()))
            .await
            .unwrap();

        // Should not forward any chunk if no output has been streamed (enables clean retries).
        // The receiver may either time out (no activity) or complete with `None` if the sender is dropped.
        let recv =
            tokio::time::timeout(std::time::Duration::from_millis(50), outer_rx.recv()).await;
        match recv {
            Err(_) => {}   // no activity
            Ok(None) => {} // sender dropped without sending anything
            Ok(Some(other)) => panic!("did not expect any forwarded chunk, got: {other:?}"),
        }

        let result = forward_handle.await.unwrap();
        assert_eq!(result.outcome, AttemptOutcome::RetryableError);
    }

    #[tokio::test]
    async fn forward_attempt_stream_fatal_error_after_output_is_forwarded() {
        let (outer_tx, mut outer_rx) = mpsc::channel::<StreamChunk>(10);
        let (attempt_tx, mut attempt_rx) = mpsc::channel::<StreamChunk>(10);

        let forward_handle =
            tokio::spawn(async move { forward_attempt_stream(&mut attempt_rx, &outer_tx).await });

        attempt_tx
            .send(StreamChunk::Text("hello".to_string()))
            .await
            .unwrap();
        match outer_rx.recv().await {
            Some(StreamChunk::Text(t)) => assert_eq!(t, "hello"),
            other => panic!("Expected Text chunk, got: {other:?}"),
        }

        attempt_tx
            .send(StreamChunk::Error("boom".to_string()))
            .await
            .unwrap();
        match outer_rx.recv().await {
            Some(StreamChunk::Error(e)) => assert_eq!(e, "boom"),
            other => panic!("Expected Error chunk, got: {other:?}"),
        }

        let result = forward_handle.await.unwrap();
        assert_eq!(result.outcome, AttemptOutcome::FatalError);
    }

    #[tokio::test]
    async fn forward_attempt_stream_drops_status_under_backpressure_without_blocking_retry() {
        let (outer_tx, mut outer_rx) = mpsc::channel::<StreamChunk>(1);
        outer_tx
            .send(StreamChunk::Status {
                message: "occupied".to_string(),
            })
            .await
            .unwrap();

        let (attempt_tx, mut attempt_rx) = mpsc::channel::<StreamChunk>(10);
        let forward_handle =
            tokio::spawn(async move { forward_attempt_stream(&mut attempt_rx, &outer_tx).await });

        attempt_tx
            .send(StreamChunk::Status {
                message: "keepalive".to_string(),
            })
            .await
            .unwrap();
        attempt_tx
            .send(StreamChunk::Error("retry me".to_string()))
            .await
            .unwrap();
        drop(attempt_tx);

        let result = tokio::time::timeout(Duration::from_millis(300), forward_handle)
            .await
            .expect("status backpressure should not stall forwarder")
            .expect("forwarder join should succeed");

        assert_eq!(result.outcome, AttemptOutcome::RetryableError);
        assert!(!result.forwarded_output);
        assert_eq!(result.error.as_deref(), Some("retry me"));

        match outer_rx.recv().await {
            Some(StreamChunk::Status { message }) => assert_eq!(message, "occupied"),
            other => panic!("expected only the pre-filled status chunk, got: {other:?}"),
        }

        let recv = tokio::time::timeout(Duration::from_millis(50), outer_rx.recv()).await;
        match recv {
            Err(_) => {}
            Ok(None) => {}
            Ok(Some(other)) => {
                panic!("did not expect forwarded status/error chunk, got: {other:?}")
            }
        }
    }

    #[tokio::test]
    async fn forward_attempt_stream_drops_token_usage_under_backpressure_without_blocking_retry() {
        let (outer_tx, mut outer_rx) = mpsc::channel::<StreamChunk>(1);
        outer_tx
            .send(StreamChunk::TokenUsageUpdate {
                estimated: 42,
                limit: 100,
                percentage: 42,
                status: TokenUsageStatus::Green,
                estimated_cost: 0.0001,
            })
            .await
            .unwrap();

        let (attempt_tx, mut attempt_rx) = mpsc::channel::<StreamChunk>(10);
        let forward_handle =
            tokio::spawn(async move { forward_attempt_stream(&mut attempt_rx, &outer_tx).await });

        attempt_tx
            .send(StreamChunk::TokenUsageUpdate {
                estimated: 50,
                limit: 100,
                percentage: 50,
                status: TokenUsageStatus::Green,
                estimated_cost: 0.0002,
            })
            .await
            .unwrap();
        attempt_tx
            .send(StreamChunk::Error("retry me".to_string()))
            .await
            .unwrap();
        drop(attempt_tx);

        let result = tokio::time::timeout(Duration::from_millis(300), forward_handle)
            .await
            .expect("token-usage backpressure should not stall forwarder")
            .expect("forwarder join should succeed");

        assert_eq!(result.outcome, AttemptOutcome::RetryableError);
        assert!(!result.forwarded_output);
        assert_eq!(result.error.as_deref(), Some("retry me"));

        match outer_rx.recv().await {
            Some(StreamChunk::TokenUsageUpdate { estimated, .. }) => assert_eq!(estimated, 42),
            other => panic!("expected only the pre-filled token-usage chunk, got: {other:?}"),
        }

        let recv = tokio::time::timeout(Duration::from_millis(50), outer_rx.recv()).await;
        match recv {
            Err(_) => {}
            Ok(None) => {}
            Ok(Some(other)) => {
                panic!("did not expect forwarded token-usage/error chunk, got: {other:?}")
            }
        }
    }
}
