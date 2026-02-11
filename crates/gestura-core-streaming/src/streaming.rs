//! Streaming LLM provider support for Gestura
//!
//! This module provides streaming capabilities for LLM responses, enabling
//! real-time token-by-token delivery to the frontend with cancellation support.

use crate::config::StreamingConfig;
use futures_util::StreamExt;
use gestura_core_foundation::AppError;
use gestura_core_llm::TokenUsage;
use gestura_core_tools::schemas::ProviderToolSchemas;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

/// Default timeout for streaming LLM API calls
const STREAMING_TIMEOUT_SECS: u64 = 300;

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

/// A chunk of streaming response
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Content from the model's thinking process
    Thinking(String),
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
    /// Real-time shell output chunk (stdout or stderr).
    ///
    /// Emitted while a shell command is executing so the UI can stream output
    /// into an embedded terminal component. Each chunk is a small fragment of
    /// text (typically one or a few lines).
    ShellOutput {
        /// Unique identifier for the shell process (matches `ShellLifecycle`).
        process_id: String,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptOutcome {
    Success,
    RetryableError,
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
                let _ = tx.send(chunk).await;
            }
            StreamChunk::Status { .. } => {
                // Forward status updates without marking as output
                let _ = tx.send(chunk).await;
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
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a new cancellation token
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cancel the streaming request
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Check if cancellation has been requested
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
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

/// Stream a response from OpenAI-compatible API
fn build_openai_chat_body(
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

pub async fn stream_openai(
    api_key: &str,
    base_url: &str,
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!("{}/v1/chat/completions", base_url);
    let body = build_openai_chat_body(model, prompt, tools);

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
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Llm(format!("OpenAI HTTP {}: {}", status, body)));
    }

    let mut stream = response.bytes_stream();
    let mut parser = ThinkingParser::new();
    let mut line_buffer = String::new();
    // Track whether we have an active (in-flight) tool call so we can emit
    // `ToolCallEnd` before the next `ToolCallStart` when the model makes
    // multiple concurrent tool calls in a single response.
    let mut in_tool_call = false;

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
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
                        // If a tool call was in flight, close it before signalling done.
                        if in_tool_call {
                            let _ = tx.send(StreamChunk::ToolCallEnd).await;
                        }
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
                            for call in tool_calls {
                                // OpenAI-compatible streaming may send `id`, `name`, and a first
                                // `arguments` fragment in the same delta. Subsequent deltas often
                                // omit `id` and stream only `arguments`.
                                let id = call["id"].as_str();
                                let name = call["function"]["name"].as_str();
                                let args = call["function"]["arguments"].as_str();

                                if let (Some(id), Some(name)) = (id, name) {
                                    // Close previous tool call before starting a new one.
                                    if in_tool_call {
                                        let _ = tx.send(StreamChunk::ToolCallEnd).await;
                                    }
                                    let _ = tx
                                        .send(StreamChunk::ToolCallStart {
                                            id: id.to_string(),
                                            name: name.to_string(),
                                        })
                                        .await;
                                    in_tool_call = true;
                                }

                                if let Some(args) = args
                                    && !args.is_empty()
                                {
                                    let _ =
                                        tx.send(StreamChunk::ToolCallArgs(args.to_string())).await;
                                }
                            }
                        }

                        // Handle finish reason — close the final tool call.
                        if let Some(finish_reason) = json["choices"][0]["finish_reason"].as_str()
                            && finish_reason == "tool_calls"
                            && in_tool_call
                        {
                            let _ = tx.send(StreamChunk::ToolCallEnd).await;
                            in_tool_call = false;
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
        return Err(AppError::Llm(format!(
            "Anthropic HTTP {}: {}",
            status, body
        )));
    }

    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();
    let mut parser = ThinkingParser::new();
    let mut in_tool_block = false;

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
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
        return Err(AppError::Llm(format!("Gemini HTTP {}: {}", status, body)));
    }

    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();
    let mut parser = ThinkingParser::new();
    let mut last_usage: Option<TokenUsage> = None;

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
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

/// Stream a response from Ollama local API
pub async fn stream_ollama(
    base_url: &str,
    model: &str,
    prompt: &str,
    tools: Option<&[serde_json::Value]>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!("{}/api/chat", base_url);
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

    let client = create_streaming_client();
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Ollama streaming request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Llm(format!("Ollama HTTP {}: {}", status, body)));
    }

    let mut stream = response.bytes_stream();
    let mut parser = ThinkingParser::new();
    let mut line_buffer = String::new();

    while let Some(chunk_result) = stream.next().await {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
            return Ok(());
        }

        match chunk_result {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in collect_complete_lines(&mut line_buffer, &text) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
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
                            for call in tool_calls {
                                let name = call["function"]["name"].as_str().unwrap_or_default();
                                let args = &call["function"]["arguments"];

                                if !name.is_empty() {
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
                                }
                            }
                        }

                        // Check if done
                        if json["done"].as_bool() == Some(true) {
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
    let _ = tx
        .send(StreamChunk::Status {
            message: message.clone(),
        })
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

/// Returns `true` if an [`AppError`] indicates a provider is not configured.
fn is_unconfigured_provider_error(err: &AppError) -> bool {
    match err {
        AppError::Llm(msg) => is_unconfigured_provider_message(msg),
        _ => false,
    }
}

/// Stream using the deterministic "echo" provider.
///
/// This provider is intended for dev/test and never performs network I/O.
#[cfg(any(test, feature = "dev"))]
async fn stream_echo(
    prompt: &str,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    if cancel_token.is_cancelled() {
        let _ = tx.send(StreamChunk::Cancelled).await;
        return Ok(());
    }

    let _ = tx.send(StreamChunk::Text(prompt.to_string())).await;
    let _ = tx.send(StreamChunk::Done(None)).await;
    Ok(())
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
    match config.primary.as_str() {
        #[cfg(any(test, feature = "dev"))]
        "echo" => stream_echo(prompt, tx, cancel_token).await,
        #[cfg(not(any(test, feature = "dev")))]
        "echo" => stream_unconfigured_error("echo", tx).await,
        "openai" => {
            if let Some(c) = &config.openai {
                stream_openai(
                    &c.api_key,
                    c.base_url.as_deref().unwrap_or("https://api.openai.com"),
                    &c.model,
                    prompt,
                    tool_schemas.as_ref().map(|s| s.openai.as_slice()),
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
                stream_openai(
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

/// Start streaming with fallback to secondary provider on failure
/// Implements exponential backoff retry (1s, 2s, 4s) before falling back
pub async fn start_streaming_with_fallback(
    config: &StreamingConfig,
    prompt: &str,
    tool_schemas: Option<ProviderToolSchemas>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    // Try primary provider with retries
    let retry_delays = [1, 2, 4]; // seconds
    let mut last_error: Option<AppError> = None;
    let mut skipped_retries_due_to_unconfigured = false;

    for (attempt, delay) in retry_delays.iter().enumerate() {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
            return Ok(());
        }

        // Create a new channel for this attempt
        let (attempt_tx, mut attempt_rx) = mpsc::channel::<StreamChunk>(100);
        let attempt_cancel = cancel_token.clone();
        let config_clone = config.clone();
        let prompt_clone = prompt.to_string();
        let tool_schemas_clone = tool_schemas.clone();

        // Spawn the streaming attempt
        let handle = tokio::spawn(async move {
            start_streaming(
                &config_clone,
                &prompt_clone,
                tool_schemas_clone,
                attempt_tx,
                attempt_cancel,
            )
            .await
        });

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

        // Only back off if we will actually perform another attempt.
        if attempt + 1 < retry_delays.len() {
            // Log retry attempt and notify frontend
            let error_msg = last_error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "Unknown error".to_string());

            tracing::warn!(
                attempt = attempt + 1,
                delay = delay,
                error = %error_msg,
                "Primary LLM failed, retrying in {}s",
                delay
            );

            // Emit retry notification to frontend
            let _ = tx
                .send(StreamChunk::RetryAttempt {
                    attempt: attempt as u32 + 1,
                    max_attempts: retry_delays.len() as u32,
                    delay_ms: *delay * 1000,
                    error_message: error_msg,
                })
                .await;

            tokio::time::sleep(tokio::time::Duration::from_secs(*delay)).await;
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
    fn test_cancellation_token() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
        token.cancel();
        assert!(token.is_cancelled());
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

        let body = build_openai_chat_body("gpt-test", "hi", Some(&tools));
        assert!(body.get("tools").is_some());
        assert_eq!(
            body.get("tool_choice").and_then(|v| v.as_str()),
            Some("auto")
        );
    }

    #[test]
    fn openai_body_omits_tools_when_none() {
        let body = build_openai_chat_body("gpt-test", "hi", None);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn openai_body_omits_temperature() {
        let body = build_openai_chat_body("gpt-test", "hi", None);
        assert!(body.get("temperature").is_none());
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
    async fn start_streaming_with_fallback_unconfigured_primary_skips_retries_and_uses_fallback() {
        let cfg = StreamingConfig {
            primary: "openai".to_string(),
            openai: None,
            fallback: Some("echo".to_string()),
            ..Default::default()
        };

        let (tx, mut rx) = mpsc::channel(128);
        let cancel = CancellationToken::new();

        let res = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            start_streaming_with_fallback(&cfg, "hi", None, tx, cancel),
        )
        .await;
        assert!(res.is_ok(), "expected fallback to complete quickly");
        assert!(res.unwrap().is_ok());

        // Primary emits Status (unconfigured) which is forwarded.
        match rx.recv().await {
            Some(StreamChunk::Status { message }) => assert!(message.contains("not configured")),
            other => panic!("Expected Status chunk, got: {other:?}"),
        }

        // Fallback echoes the prompt.
        match rx.recv().await {
            Some(StreamChunk::Text(t)) => assert_eq!(t, "hi"),
            other => panic!("Expected Text chunk, got: {other:?}"),
        }
        match rx.recv().await {
            Some(StreamChunk::Done(_)) => {}
            other => panic!("Expected Done chunk, got: {other:?}"),
        }
    }
}
