//! Streaming LLM provider support for Gestura
//!
//! This module provides streaming capabilities for LLM responses, enabling
//! real-time token-by-token delivery to the frontend with cancellation support.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::llm_provider::TokenUsage;
use crate::tools::schemas::ProviderToolSchemas;
use futures_util::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

/// Character chunk size used for the built-in streaming echo fallback.
///
/// This intentionally emits *multiple* `StreamChunk::Text` events so the UI behaves like
/// a true streaming experience even when no provider is configured.
const ECHO_STREAM_CHUNK_CHARS: usize = 24;

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
    /// Stream completed successfully with optional token usage
    Done(Option<TokenUsage>),
    /// Stream was cancelled
    Cancelled,
    /// An error occurred
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptOutcome {
    Success,
    RetryableError,
    FatalError,
    Cancelled,
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
pub(crate) fn split_think_blocks(input: &str) -> (String, Option<String>) {
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
                                    let _ = tx
                                        .send(StreamChunk::ToolCallStart {
                                            id: id.to_string(),
                                            name: name.to_string(),
                                        })
                                        .await;
                                }

                                if let Some(args) = args
                                    && !args.is_empty()
                                {
                                    let _ =
                                        tx.send(StreamChunk::ToolCallArgs(args.to_string())).await;
                                }
                            }
                        }

                        // Handle finish reason
                        if let Some(finish_reason) = json["choices"][0]["finish_reason"].as_str()
                            && finish_reason == "tool_calls"
                        {
                            let _ = tx.send(StreamChunk::ToolCallEnd).await;
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
                            if let Some(tool_use) = json["content_block"]["tool_use"].as_object()
                                && let Some(name) = tool_use["name"].as_str()
                            {
                                let id = tool_use["id"].as_str().unwrap_or_default().to_string();
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

/// Stream a response from Ollama local API
pub async fn stream_ollama(
    base_url: &str,
    model: &str,
    prompt: &str,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    let url = format!("{}/api/chat", base_url);
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": true
    });

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
                        // Check if done
                        if json["done"].as_bool() == Some(true) {
                            let _ = tx.send(StreamChunk::Done(None)).await;
                            return Ok(());
                        }
                        // Extract content from message
                        if let Some(content) = json["message"]["content"].as_str()
                            && !content.is_empty()
                        {
                            let chunks = parser.process(content);
                            for chunk in chunks {
                                let _ = tx.send(chunk).await;
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

    let _ = tx.send(StreamChunk::Done(None)).await;
    Ok(())
}

async fn stream_echo_fallback(
    provider_name: &str,
    prompt: &str,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    // Put the configuration hint into the *thinking* channel so it doesn't pollute
    // the assistant's visible answer (but can still be shown in debug/advanced UI).
    let note = format!(
        "LLM provider '{provider_name}' is not configured. Configure llm.{provider_name} (api_key/model) to get real streaming + thinking. Falling back to Echo mode."
    );
    let _ = tx.send(StreamChunk::Thinking(note)).await;
    tokio::task::yield_now().await;

    let full = format!("ECHO: {prompt}");
    let mut rest = full.as_str();

    while !rest.is_empty() {
        if cancel_token.is_cancelled() {
            let _ = tx.send(StreamChunk::Cancelled).await;
            return Ok(());
        }

        let split_at = rest
            .char_indices()
            .nth(ECHO_STREAM_CHUNK_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(rest.len());

        let (chunk, next) = rest.split_at(split_at);
        rest = next;

        if !chunk.is_empty() {
            let _ = tx.send(StreamChunk::Text(chunk.to_string())).await;
            tokio::task::yield_now().await;
        }
    }

    let _ = tx.send(StreamChunk::Done(None)).await;
    Ok(())
}

/// Start a streaming LLM request based on config
pub async fn start_streaming(
    config: &AppConfig,
    prompt: &str,
    tool_schemas: Option<ProviderToolSchemas>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    match config.llm.primary.as_str() {
        "openai" => {
            if let Some(c) = &config.llm.openai {
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
                stream_echo_fallback("openai", prompt, tx, cancel_token).await
            }
        }
        "anthropic" => {
            if let Some(c) = &config.llm.anthropic {
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
                stream_echo_fallback("anthropic", prompt, tx, cancel_token).await
            }
        }
        "grok" => {
            // Grok uses OpenAI-compatible API
            if let Some(c) = &config.llm.grok {
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
                stream_echo_fallback("grok", prompt, tx, cancel_token).await
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                stream_ollama(&c.base_url, &c.model, prompt, tx, cancel_token).await
            } else {
                stream_echo_fallback("ollama", prompt, tx, cancel_token).await
            }
        }
        _ => stream_echo_fallback("unknown", prompt, tx, cancel_token).await,
    }
}

/// Start streaming with fallback to secondary provider on failure
/// Implements exponential backoff retry (1s, 2s, 4s) before falling back
pub async fn start_streaming_with_fallback(
    config: &AppConfig,
    prompt: &str,
    tool_schemas: Option<ProviderToolSchemas>,
    tx: mpsc::Sender<StreamChunk>,
    cancel_token: CancellationToken,
) -> Result<(), AppError> {
    // Try primary provider with retries
    let retry_delays = [1, 2, 4]; // seconds
    let mut last_error: Option<AppError> = None;

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
            AttemptOutcome::Cancelled => return Ok(()),
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

    // Primary failed after retries, try fallback if configured
    if let Some(ref fallback_provider) = config.llm.fallback {
        tracing::info!(
            fallback = fallback_provider,
            "Primary LLM exhausted retries, trying fallback provider"
        );

        // Create a modified config with fallback as primary
        let mut fallback_config = config.clone();
        fallback_config.llm.primary = fallback_provider.clone();

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
    async fn start_streaming_unconfigured_provider_still_feels_streaming() {
        let mut cfg = AppConfig::default();
        cfg.llm.primary = "openai".to_string();
        cfg.llm.openai = None;

        let (tx, mut rx) = mpsc::channel(128);
        let cancel = CancellationToken::new();

        tokio::spawn(async move {
            let prompt = "hello world this is a deliberately long prompt to force the echo fallback to emit multiple chunks";
            let _ = start_streaming(&cfg, prompt, None, tx, cancel).await;
        });

        // First chunk should be a helpful note.
        match rx.recv().await {
            Some(StreamChunk::Thinking(t)) => assert!(t.contains("not configured")),
            other => panic!("Expected Thinking chunk, got: {other:?}"),
        }

        // Then we should get multiple Text chunks before Done.
        let mut text_chunks = 0usize;
        while let Some(chunk) = rx.recv().await {
            match chunk {
                StreamChunk::Text(_) => {
                    text_chunks += 1;
                    if text_chunks >= 2 {
                        break;
                    }
                }
                StreamChunk::Done(_) => break,
                _ => {}
            }
        }
        assert!(
            text_chunks >= 2,
            "Expected >=2 text chunks, got {text_chunks}"
        );
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
}
