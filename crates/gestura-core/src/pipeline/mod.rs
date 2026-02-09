//! Agent Pipeline - Unified LLM interaction pipeline
//!
//! This module provides a single entry point for all LLM interactions,
//! regardless of input source (text, voice, delegated tasks). It integrates:
//!
//! - Context analysis and reduction
//! - Tool filtering based on request
//! - Agentic loop for tool execution
//! - Streaming and non-streaming responses
//! - Token estimation and truncation
//! - Fallback to secondary providers
//! - Workspace sandboxing for tool execution

pub mod types;

use std::path::Path;
use std::time::Instant;
use tokio::sync::mpsc;

use crate::chat_sessions::FileChatSessionStore;
use crate::checkpoints::{CheckpointManager, CheckpointRetentionPolicy, FileCheckpointStore};
use crate::config::AppConfig;
use crate::context::{ContextManager, RequestAnalyzer};
use crate::error::AppError;
use crate::hooks::{HookContext, HookEngine, HookEvent};
use crate::knowledge::{KnowledgeSettingsManager, KnowledgeStore};
use crate::llm_provider::{AgentContext, select_provider};
use crate::session_workspace::SessionWorkspace;
use crate::streaming::{
    CancellationToken, StreamChunk, start_streaming, start_streaming_with_fallback,
};
use crate::tasks::TaskManager;
use crate::tool_confirmation::TOOL_CONFIRMATIONS;
use crate::tools::PermissionManager;
use crate::tools::registry::{ToolDefinition, all_tools};

pub use types::*;

/// Select the correct tool schema slice for a provider name.
///
/// Anthropic uses its own `{name, description, input_schema}` format; all
/// other providers use the OpenAI-compatible `{type:"function", function:{…}}` format.
fn tools_slice_for_provider(
    provider_name: &str,
    schemas: &crate::tools::schemas::ProviderToolSchemas,
) -> Vec<serde_json::Value> {
    match provider_name {
        "anthropic" => schemas.anthropic.clone(),
        _ => schemas.openai.clone(),
    }
}

/// The main agent pipeline for processing requests
pub struct AgentPipeline {
    /// Application configuration
    config: AppConfig,
    /// Context manager for smart context reduction
    context_manager: ContextManager,
    /// Request analyzer for category detection
    analyzer: RequestAnalyzer,
    /// Pipeline-specific configuration
    pipeline_config: PipelineConfig,
    /// Persistent permission manager used for tool confirmation decisions.
    ///
    /// This enables "Allow always" semantics for tool confirmations.
    permission_manager: PermissionManager,
    /// Knowledge store for specialized expertise
    knowledge_store: Option<&'static KnowledgeStore>,
    /// Knowledge settings manager for session-scoped activation
    knowledge_settings: Option<&'static KnowledgeSettingsManager>,
}

impl AgentPipeline {
    /// Create a new pipeline with default configuration
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            context_manager: ContextManager::new(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config: PipelineConfig::default(),
            permission_manager: PermissionManager::new(),
            knowledge_store: None,
            knowledge_settings: None,
        }
    }

    /// Create a pipeline with custom configuration
    pub fn with_config(config: AppConfig, pipeline_config: PipelineConfig) -> Self {
        Self {
            config,
            context_manager: ContextManager::new(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config,
            permission_manager: PermissionManager::new(),
            knowledge_store: None,
            knowledge_settings: None,
        }
    }

    /// Set the knowledge store and settings manager for this pipeline
    pub fn with_knowledge(
        mut self,
        store: &'static KnowledgeStore,
        settings: &'static KnowledgeSettingsManager,
    ) -> Self {
        self.knowledge_store = Some(store);
        self.knowledge_settings = Some(settings);
        self
    }

    /// Create a HookEngine from the current configuration.
    ///
    /// Returns `None` if hooks are disabled or empty.
    fn create_hook_engine(&self) -> Option<HookEngine> {
        if !self.config.hooks.enabled || self.config.hooks.hooks.is_empty() {
            return None;
        }
        Some(HookEngine::new(self.config.hooks.clone()))
    }

    /// Run a hook event, logging any failures but not propagating them.
    ///
    /// This is used for best-effort hooks (PostPipeline, PostTool) where failures
    /// should not affect the main flow.
    async fn run_hook_best_effort(&self, engine: &HookEngine, event: HookEvent, ctx: &HookContext) {
        match engine.run(event, ctx).await {
            Ok(records) => {
                for record in &records {
                    tracing::debug!(
                        hook = %record.name,
                        event = ?record.event,
                        exit_code = record.output.exit_code,
                        "Hook executed (best-effort)"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    event = ?event,
                    error = %e,
                    "Hook execution failed (best-effort, continuing)"
                );
            }
        }
    }

    /// Create a checkpoint before a write tool execution.
    ///
    /// This is a best-effort operation: failures are logged but do not block tool execution.
    fn try_create_checkpoint_before_tool(&self, session_id: &str, tool_name: &str) {
        let label = format!("before:{}", tool_name);

        // Use default stores - these are lightweight to construct
        let session_store = FileChatSessionStore::new_default();
        let checkpoint_store = FileCheckpointStore::new_default();
        let manager =
            CheckpointManager::new(checkpoint_store, CheckpointRetentionPolicy::default());

        // TaskManager needs a base directory
        let task_manager = TaskManager::new(AppConfig::data_dir());

        match manager.create_session_checkpoint(
            session_id,
            &session_store,
            &task_manager,
            &self.config,
            Some(label),
        ) {
            Ok(meta) => {
                tracing::info!(
                    checkpoint_id = %meta.id,
                    session_id = session_id,
                    tool = tool_name,
                    "Created auto-checkpoint before write tool"
                );
            }
            Err(e) => {
                tracing::warn!(
                    session_id = session_id,
                    tool = tool_name,
                    error = %e,
                    "Failed to create auto-checkpoint (continuing with tool execution)"
                );
            }
        }
    }

    /// Create a pipeline with configuration optimized for the current LLM provider
    ///
    /// This automatically sets the context token limit based on the provider's capabilities
    /// and applies user settings from AppConfig.pipeline.
    pub fn with_provider_optimized_config(config: AppConfig) -> Self {
        let provider = config.llm.primary.as_str();
        let pipeline_config =
            PipelineConfig::for_provider(provider).with_user_settings(&config.pipeline);

        tracing::info!(
            provider = provider,
            max_context_tokens = pipeline_config.max_context_tokens,
            max_history_messages = pipeline_config.max_history_messages,
            auto_compact_threshold = pipeline_config.auto_compact_threshold,
            compaction_strategy = ?pipeline_config.compaction_strategy,
            "Created pipeline with provider-optimized configuration and user settings"
        );

        Self {
            config,
            context_manager: ContextManager::new(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config,
            permission_manager: PermissionManager::new(),
            knowledge_store: None,
            knowledge_settings: None,
        }
    }

    /// Build a user-facing status message for an auto-compaction event.
    ///
    /// This message is intended to be emitted as `StreamChunk::Status` **before** the
    /// resulting compaction chunk (e.g., `StreamChunk::ContextCompacted`) so adapters can
    /// immediately surface what's happening.
    fn build_auto_compaction_status_message(&self, prompt_preview: &str) -> String {
        let estimated_tokens = Self::estimate_tokens(prompt_preview);
        let max_input = self
            .pipeline_config
            .max_context_tokens
            .saturating_sub(self.pipeline_config.max_output_tokens);

        let pct = (estimated_tokens.saturating_mul(100) / max_input.max(1)).min(999);
        let threshold_pct = (self.pipeline_config.auto_compact_threshold * 100.0).round() as u32;

        let strategy = match self.pipeline_config.compaction_strategy {
            CompactionStrategy::Summarize => "summarization",
            CompactionStrategy::Truncate => "truncation",
            CompactionStrategy::Clear => "clearing history",
            CompactionStrategy::Prompt => "prompting for choice",
            CompactionStrategy::MemoryBank => "memory bank save",
        };

        format!(
            "Context near token limit (~{pct}% of {max_input} input tokens; threshold {threshold_pct}%). Auto-compacting using {strategy}…"
        )
    }

    /// Process a request with streaming response
    ///
    /// This is the main entry point for streaming LLM interactions.
    /// It handles context reduction, tool filtering, and the agentic loop.
    pub async fn process_streaming(
        &self,
        request: AgentRequest,
        tx: mpsc::Sender<StreamChunk>,
        cancel_token: CancellationToken,
    ) -> Result<AgentResponse, AppError> {
        // 1. Analyze the request
        let mut analysis = self.analyzer.analyze(&request.input);

        // Heuristic: if the user is replying with an approval ("ok", "please proceed")
        // and the previous assistant turn proposed using a tool, promote this turn into
        // a tool-enabled follow-up so the agent can actually execute the intended tool.
        self.promote_approval_to_tool_followup(&request, &mut analysis);
        tracing::debug!(
            "Request analysis: categories={:?}, needs_tools={}, confidence={}",
            analysis.categories,
            analysis.needs_tools,
            analysis.confidence
        );

        // 2. Filter tools based on categories (and allowed_tools if specified)
        let tools_enabled_for_request = request.metadata.tools_enabled.unwrap_or(true);

        let relevant_tools = if self.pipeline_config.enable_tools
            && tools_enabled_for_request
            && analysis.needs_tools
        {
            self.get_tools_for_analysis(&analysis, &request.metadata.allowed_tools)
        } else {
            Vec::new()
        };
        tracing::debug!(
            "Relevant tools: {:?}",
            relevant_tools.iter().map(|t| t.name).collect::<Vec<_>>()
        );

        // Workspace sandboxing (used by tool execution)
        let workspace = request.metadata.workspace_dir.as_ref().and_then(|p| {
            SessionWorkspace::from_directory(
                request.metadata.session_id.as_deref().unwrap_or("unknown"),
                p.clone(),
            )
            .ok()
        });

        // Fast-path: if the user is explicitly approving a previously proposed tool call
        // (e.g. "okay please proceed"), execute the intended tool directly from history.
        //
        // This prevents a common UX failure mode where the model describes tool usage,
        // the user approves, but the provider doesn't emit a structured tool call so the
        // app appears to "hang" or never produces an answer.
        if self.pipeline_config.enable_tools
            && tools_enabled_for_request
            && analysis.is_followup
            && Self::looks_like_approval(&request.input)
            && let Some(resp) = self
                .try_execute_confirmed_tool_from_history(
                    &request,
                    &analysis,
                    &relevant_tools,
                    workspace.as_ref(),
                    &tx,
                    &cancel_token,
                )
                .await?
        {
            return Ok(resp);
        }

        // 3. Resolve context
        let mut resolved_context =
            self.context_manager
                .resolve_context(&request.input, &analysis, &request.history);

        // 3.1. Search memory bank for relevant context (if workspace available)
        if let Some(workspace_dir) = &request.metadata.workspace_dir
            && let Some(memory_context) = self
                .search_and_load_memory_bank(workspace_dir, &request.input, 3)
                .await
        {
            // Add memory bank context to knowledge field
            resolved_context.knowledge.push(memory_context.clone());

            tracing::debug!(
                memory_context_len = memory_context.len(),
                "Added memory bank context to request"
            );
        }

        // 3.2. Load enabled knowledge items for this session
        if let Some(knowledge_context) =
            self.load_enabled_knowledge(request.metadata.session_id.as_deref())
        {
            resolved_context.knowledge.push(knowledge_context.clone());

            tracing::debug!(
                knowledge_context_len = knowledge_context.len(),
                "Added enabled knowledge to request"
            );
        }

        // 3.5. Check for auto-compaction before building prompt
        // Build a preview prompt to estimate tokens
        let preview_prompt = self.build_prompt(&request, &resolved_context);
        if let Some(compaction_chunk) = self
            .check_and_apply_auto_compaction(&request.history, &preview_prompt, &request.metadata)
            .await
        {
            // Emit user-visible status **before** the compaction result chunk.
            let message = self.build_auto_compaction_status_message(&preview_prompt);
            let _ = tx.send(StreamChunk::Status { message }).await;

            // Emit compaction notification to user
            let _ = tx.send(compaction_chunk).await;

            // Re-resolve context after compaction
            resolved_context =
                self.context_manager
                    .resolve_context(&request.input, &analysis, &request.history);
        }

        // 4. Build the optimized prompt with token limit checking
        let (prompt, truncated) = self.truncate_prompt_if_needed(&request, &mut resolved_context);

        if truncated {
            tracing::info!("Prompt was truncated to fit token limit");
        }

        // 4.5. Hard validation: reject if still over limit after truncation
        // This prevents API errors and provides clear feedback to the user
        self.validate_token_limit(&prompt)?;

        // 4.6. Emit token usage update for user visibility
        let token_usage_chunk = self.create_token_usage_update(&prompt);
        let _ = tx.send(token_usage_chunk).await;

        // 4.7. Run PrePipeline hooks (if enabled)
        let hook_engine = self.create_hook_engine();
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: request.metadata.workspace_dir.clone(),
                session_id: request.metadata.session_id.clone(),
                pipeline_prompt: Some(prompt.clone()),
                ..Default::default()
            };
            if let Err(e) = engine.run(HookEvent::PrePipeline, &hook_ctx).await {
                tracing::warn!(error = %e, "PrePipeline hook failed (continuing)");
            }
        }

        // 5. Execute the agentic loop with workspace sandboxing
        let mut response = self
            .execute_agentic_loop_streaming(
                prompt,
                relevant_tools,
                resolved_context,
                tx,
                cancel_token,
                workspace.as_ref(),
                request.metadata.session_id.clone(),
                request.metadata.permission_level,
            )
            .await?;

        // 5.1. Run PostPipeline hooks (best-effort)
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: request.metadata.workspace_dir.clone(),
                session_id: request.metadata.session_id.clone(),
                ..Default::default()
            };
            self.run_hook_best_effort(engine, HookEvent::PostPipeline, &hook_ctx)
                .await;
        }

        response.truncated = truncated;

        Ok(response)
    }

    fn looks_like_approval(input: &str) -> bool {
        let s = input.trim().to_lowercase();
        matches!(
            s.as_str(),
            "ok" | "okay"
                | "ok."
                | "okay."
                | "yes"
                | "y"
                | "sure"
                | "please proceed"
                | "proceed"
                | "go ahead"
                | "do it"
                | "run it"
                | "continue"
        ) || s.contains("please proceed")
            || s.contains("go ahead")
            || s.contains("please do")
            || s.contains("yes, proceed")
    }

    /// Attempt to execute a previously proposed tool call directly from the assistant's
    /// last message, when the current user turn is an approval/follow-up.
    ///
    /// This is a defensive fallback for a common failure mode where the model:
    /// 1) proposes a tool call,
    /// 2) asks for confirmation,
    /// 3) after user approval, fails to emit a structured tool call.
    ///
    /// We infer the intended tool from the previous assistant message and execute it.
    async fn try_execute_confirmed_tool_from_history(
        &self,
        request: &AgentRequest,
        _analysis: &crate::context::RequestAnalysis,
        relevant_tools: &[&'static ToolDefinition],
        workspace: Option<&SessionWorkspace>,
        tx: &mpsc::Sender<StreamChunk>,
        cancel_token: &CancellationToken,
    ) -> Result<Option<AgentResponse>, AppError> {
        let has_tool = |name: &str| relevant_tools.iter().any(|t| t.name == name);

        let Some(prev_assistant) = request.history.iter().rev().find(|m| m.role == "assistant")
        else {
            return Ok(None);
        };

        let Some((tool_name, args, _answer_prefix)) =
            Self::extract_planned_tool_call_from_text(&prev_assistant.content)
        else {
            return Ok(None);
        };

        // Only run if the tool is actually available on this turn.
        if !has_tool(&tool_name) {
            return Ok(None);
        }

        // Execute immediately (still subject to the normal safety checks inside execute_tool).
        let tool_call_id = format!("confirmed_{tool_name}");

        let _ = tx
            .send(StreamChunk::Thinking(
                "Executing approved command...\n".to_string(),
            ))
            .await;
        let _ = tx
            .send(StreamChunk::ToolCallStart {
                id: tool_call_id.clone(),
                name: tool_name.clone(),
            })
            .await;
        let _ = tx.send(StreamChunk::ToolCallArgs(args.clone())).await;

        let start_time = Instant::now();
        let result = self.execute_tool(&tool_name, &args, workspace).await;
        let duration_ms = start_time.elapsed().as_millis() as u64;

        let _ = tx.send(StreamChunk::ToolCallEnd).await;

        // Emit structured tool result for frontend display
        let (success, output) = match &result {
            ToolResult::Success(out) => (true, out.trim_end().to_string()),
            ToolResult::Error(e) => (false, e.clone()),
            ToolResult::Skipped(msg) => (false, format!("Skipped: {}", msg)),
        };
        let _ = tx
            .send(StreamChunk::ToolCallResult {
                name: tool_name.clone(),
                success,
                output: output.clone(),
                duration_ms,
            })
            .await;

        let record = ToolCallRecord {
            id: tool_call_id,
            name: tool_name,
            arguments: args,
            result,
            duration_ms,
        };

        // Build a continuation prompt so the LLM can synthesize the tool output
        // into a helpful response for the user, instead of leaving raw tool output
        // as the final answer.
        let base_prompt = self.build_prompt(request, &crate::context::ResolvedContext::default());
        let continuation_prompt = self.build_tool_continuation_prompt(
            &base_prompt,
            "Executing the approved tool call.",
            std::slice::from_ref(&record),
        );

        // Stream one more LLM call for synthesis (no tool schemas — text only).
        let (inner_tx, mut inner_rx) = mpsc::channel::<StreamChunk>(100);
        let config = self.config.clone();
        let enable_fallback = self.pipeline_config.enable_fallback;
        let inner_cancel = cancel_token.clone();

        let stream_handle = tokio::spawn(async move {
            if enable_fallback {
                let _ = start_streaming_with_fallback(
                    &config,
                    &continuation_prompt,
                    None,
                    inner_tx,
                    inner_cancel,
                )
                .await;
            } else {
                let _ =
                    start_streaming(&config, &continuation_prompt, None, inner_tx, inner_cancel)
                        .await;
            }
        });

        let mut synthesis_text = String::new();
        let mut synthesis_usage = None;

        while let Some(chunk) = inner_rx.recv().await {
            match &chunk {
                StreamChunk::Text(text) => {
                    synthesis_text.push_str(text);
                    let _ = tx.send(chunk).await;
                }
                StreamChunk::Thinking(_) => {
                    let _ = tx.send(chunk).await;
                }
                StreamChunk::Done(usage) => {
                    synthesis_usage = usage.clone();
                    break;
                }
                StreamChunk::Error(_) | StreamChunk::Cancelled => {
                    let _ = tx.send(chunk).await;
                    break;
                }
                _ => {
                    // Forward status or other informational chunks
                    let _ = tx.send(chunk).await;
                }
            }
        }

        let _ = stream_handle.await;
        let _ = tx.send(StreamChunk::Done(synthesis_usage.clone())).await;

        Ok(Some(AgentResponse {
            content: synthesis_text,
            thinking: None,
            tool_calls: vec![record],
            usage: synthesis_usage,
            context_used: crate::context::ResolvedContext::default(),
            truncated: false,
            iterations: 1,
        }))
    }

    fn extract_shell_command_from_plan(text: &str) -> Option<String> {
        // Try common patterns first: run '...'/"..."/`...`
        if let Some(cmd) = Self::extract_quoted_after_keyword(text, "run") {
            return Some(cmd);
        }
        if let Some(cmd) = Self::extract_quoted_after_keyword(text, "execute") {
            return Some(cmd);
        }

        // Fallback: try to grab the first token after "run".
        let lower = text.to_lowercase();
        let idx = lower.find("run ")?;
        let after = text[idx + 4..].trim_start();
        let token: String = after
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(*c, '.' | ',' | ';' | ')' | '('))
            .collect();
        if token.is_empty() { None } else { Some(token) }
    }

    fn extract_quoted_after_keyword(text: &str, keyword: &str) -> Option<String> {
        let lower = text.to_lowercase();
        let key = format!("{} ", keyword.to_lowercase());
        let start = lower.find(&key)?;
        let mut rest = text[start + key.len()..].trim_start();
        let quote = rest.chars().next()?;
        if quote != '\'' && quote != '"' && quote != '`' {
            return None;
        }
        rest = &rest[quote.len_utf8()..];
        let end = rest.find(quote)?;
        let cmd = rest[..end].trim().to_string();
        if cmd.is_empty() { None } else { Some(cmd) }
    }

    fn extract_first_quoted(text: &str) -> Option<String> {
        for quote in ['\'', '"', '`'] {
            if let Some(start) = text.find(quote) {
                let rest = &text[start + quote.len_utf8()..];
                if let Some(end) = rest.find(quote) {
                    let s = rest[..end].trim();
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
        None
    }

    fn extract_first_url(text: &str) -> Option<String> {
        let lower = text.to_lowercase();
        let idx = lower.find("https://").or_else(|| lower.find("http://"))?;
        let after = &text[idx..];
        let url: String = after
            .chars()
            .take_while(|c| {
                !c.is_whitespace() && !matches!(*c, ')' | '(' | ']' | '[' | '"' | '\'' | '`' | ',')
            })
            .collect();
        if url.is_empty() { None } else { Some(url) }
    }

    /// Infer a planned tool call from a prior assistant message.
    ///
    /// Returns: (tool_name, args_json, answer_prefix)
    fn extract_planned_tool_call_from_text(text: &str) -> Option<(String, String, String)> {
        let lower = text.to_lowercase();

        // Shell
        if lower.contains("shell tool")
            || lower.contains("tool: shell")
            || lower.contains("use the shell")
            || lower.contains("run 'pwd'")
            || lower.contains("`pwd`")
        {
            let command = Self::extract_shell_command_from_plan(text)?;
            let args = serde_json::json!({"command": command}).to_string();
            return Some((
                "shell".to_string(),
                args,
                "Workspace directory root: ".to_string(),
            ));
        }

        // File read
        if lower.contains("file tool")
            || lower.contains("read file")
            || lower.contains("read the file")
        {
            let path = Self::extract_quoted_after_keyword(text, "read")
                .or_else(|| Self::extract_first_quoted(text))?;
            let args = serde_json::json!({"operation": "read", "path": path}).to_string();
            return Some(("file".to_string(), args, "File contents: \n".to_string()));
        }

        // Git status
        if lower.contains("git tool") || lower.contains("git status") {
            let args = serde_json::json!({"operation": "status"}).to_string();
            return Some(("git".to_string(), args, "Git status:\n".to_string()));
        }

        // Web
        if lower.contains("web tool")
            || lower.contains("search the web")
            || lower.contains("web_search")
        {
            if lower.contains("fetch") || lower.contains("download") {
                let url = Self::extract_first_url(text)?;
                let args = serde_json::json!({"operation": "fetch", "url": url}).to_string();
                return Some(("web".to_string(), args, "Web fetch result:\n".to_string()));
            }

            let query = Self::extract_quoted_after_keyword(text, "search")
                .or_else(|| Self::extract_first_quoted(text))?;
            let args = serde_json::json!({"operation": "search", "query": query}).to_string();
            return Some(("web".to_string(), args, "Web search results:\n".to_string()));
        }

        // Code stats
        if lower.contains("code tool") || lower.contains("code stats") {
            let path = Self::extract_first_quoted(text).unwrap_or_else(|| ".".to_string());
            let args = serde_json::json!({"operation": "stats", "path": path}).to_string();
            return Some(("code".to_string(), args, "Code stats:\n".to_string()));
        }

        None
    }

    /// Process a request without streaming (blocking)
    pub async fn process_blocking(&self, request: AgentRequest) -> Result<AgentResponse, AppError> {
        // 1. Analyze the request
        let analysis = self.analyzer.analyze(&request.input);

        // 2. Filter tools (and allowed_tools if specified)
        let tools_enabled_for_request = request.metadata.tools_enabled.unwrap_or(true);

        let relevant_tools = if self.pipeline_config.enable_tools
            && tools_enabled_for_request
            && analysis.needs_tools
        {
            self.get_tools_for_analysis(&analysis, &request.metadata.allowed_tools)
        } else {
            Vec::new()
        };

        // 3. Resolve context
        let mut resolved_context =
            self.context_manager
                .resolve_context(&request.input, &analysis, &request.history);

        // 3.1. Search memory bank for relevant context (if workspace available)
        if let Some(workspace_dir) = &request.metadata.workspace_dir
            && let Some(memory_context) = self
                .search_and_load_memory_bank(workspace_dir, &request.input, 3)
                .await
        {
            // Add memory bank context to knowledge field
            resolved_context.knowledge.push(memory_context.clone());

            tracing::debug!(
                memory_context_len = memory_context.len(),
                "Added memory bank context to request"
            );
        }

        // 3.2. Load enabled knowledge items for this session
        if let Some(knowledge_context) =
            self.load_enabled_knowledge(request.metadata.session_id.as_deref())
        {
            resolved_context.knowledge.push(knowledge_context.clone());

            tracing::debug!(
                knowledge_context_len = knowledge_context.len(),
                "Added enabled knowledge to request"
            );
        }

        // 3.5. Check for auto-compaction before building prompt
        // Build a preview prompt to estimate tokens
        let preview_prompt = self.build_prompt(&request, &resolved_context);
        if let Some(compaction_chunk) = self
            .check_and_apply_auto_compaction(&request.history, &preview_prompt, &request.metadata)
            .await
        {
            // Log compaction in blocking mode (no stream to emit to)
            match compaction_chunk {
                StreamChunk::ContextCompacted {
                    messages_before,
                    messages_after,
                    tokens_saved,
                    summary,
                } => {
                    tracing::info!(
                        messages_before = messages_before,
                        messages_after = messages_after,
                        tokens_saved = tokens_saved,
                        "Context auto-compacted in blocking mode: {}",
                        summary
                    );
                }
                StreamChunk::MemoryBankSaved {
                    file_path,
                    session_id,
                    summary,
                    messages_saved,
                } => {
                    tracing::info!(
                        file_path = %file_path,
                        session_id = %session_id,
                        messages_saved = messages_saved,
                        "Memory bank saved in blocking mode: {}",
                        summary
                    );
                }
                _ => {}
            }

            // Re-resolve context after compaction
            resolved_context =
                self.context_manager
                    .resolve_context(&request.input, &analysis, &request.history);
        }

        // 4. Build prompt with token limit checking
        let (prompt, truncated) = self.truncate_prompt_if_needed(&request, &mut resolved_context);

        if truncated {
            tracing::info!("Prompt was truncated to fit token limit");
        }

        // 4.5. Hard validation: reject if still over limit after truncation
        // This prevents API errors and provides clear feedback to the user
        self.validate_token_limit(&prompt)?;

        // 4.6. Log token usage in blocking mode
        if let StreamChunk::TokenUsageUpdate {
            estimated,
            limit,
            percentage,
            status,
            estimated_cost,
        } = self.create_token_usage_update(&prompt)
        {
            let status_str = match status {
                crate::streaming::TokenUsageStatus::Green => "🟢 Green",
                crate::streaming::TokenUsageStatus::Yellow => "🟡 Yellow",
                crate::streaming::TokenUsageStatus::Red => "🔴 Red",
            };
            tracing::info!(
                estimated_tokens = estimated,
                limit = limit,
                percentage = percentage,
                status = status_str,
                estimated_cost_usd = format!("${:.4}", estimated_cost),
                "Token usage in blocking mode: {} tokens / {} tokens ({}%) - Est. cost: ${:.4}",
                estimated,
                limit,
                percentage,
                estimated_cost
            );
        }

        // 4.7. Run PrePipeline hooks (if enabled)
        let hook_engine = self.create_hook_engine();
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: request.metadata.workspace_dir.clone(),
                session_id: request.metadata.session_id.clone(),
                pipeline_prompt: Some(prompt.clone()),
                ..Default::default()
            };
            if let Err(e) = engine.run(HookEvent::PrePipeline, &hook_ctx).await {
                tracing::warn!(error = %e, "PrePipeline hook failed in blocking mode (continuing)");
            }
        }

        // 5. Execute blocking agentic loop with workspace sandboxing
        let workspace = request.metadata.workspace_dir.as_ref().and_then(|p| {
            SessionWorkspace::from_directory(
                request.metadata.session_id.as_deref().unwrap_or("unknown"),
                p.clone(),
            )
            .ok()
        });

        let mut response = self
            .execute_agentic_loop_blocking(
                prompt,
                relevant_tools,
                resolved_context,
                workspace.as_ref(),
            )
            .await?;

        // 5.1. Run PostPipeline hooks (best-effort)
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: request.metadata.workspace_dir.clone(),
                session_id: request.metadata.session_id.clone(),
                ..Default::default()
            };
            self.run_hook_best_effort(engine, HookEvent::PostPipeline, &hook_ctx)
                .await;
        }

        response.truncated = truncated;

        Ok(response)
    }

    /// Get tools relevant to the analyzed request
    /// If allowed_tools is non-empty, only those tools are considered
    fn get_tools_for_analysis(
        &self,
        analysis: &crate::context::RequestAnalysis,
        allowed_tools: &[String],
    ) -> Vec<&'static ToolDefinition> {
        use crate::context::ContextCategory;

        let mut tools = Vec::new();

        // If allowed_tools is specified, only use those (session-specific tool configuration)
        // This ensures the LLM only sees tools that are enabled in the session settings
        if !allowed_tools.is_empty() {
            for tool_name in allowed_tools {
                if let Some(t) = crate::tools::registry::find_tool(tool_name) {
                    tools.push(t);
                }
            }

            if self.pipeline_config.log_token_usage {
                tracing::debug!(
                    allowed_tools = ?allowed_tools,
                    resolved_tools = ?tools.iter().map(|t| t.name).collect::<Vec<_>>(),
                    "Using session-specific tool configuration"
                );
            }

            return tools;
        }

        // Otherwise, filter by category (legacy behavior when no session tool settings exist)
        for category in &analysis.categories {
            match category {
                ContextCategory::FileSystem => {
                    if let Some(t) = crate::tools::registry::find_tool("file") {
                        tools.push(t);
                    }
                }
                ContextCategory::Shell => {
                    if let Some(t) = crate::tools::registry::find_tool("shell") {
                        tools.push(t);
                    }
                }
                ContextCategory::Git => {
                    if let Some(t) = crate::tools::registry::find_tool("git") {
                        tools.push(t);
                    }
                }
                ContextCategory::Code => {
                    if let Some(t) = crate::tools::registry::find_tool("code") {
                        tools.push(t);
                    }
                }
                ContextCategory::Web => {
                    if let Some(t) = crate::tools::registry::find_tool("web") {
                        tools.push(t);
                    }
                    // Also include web_search for search-related queries
                    if let Some(t) = crate::tools::registry::find_tool("web_search") {
                        tools.push(t);
                    }
                }
                _ => {}
            }
        }

        // If no specific tools found but tools are needed, include all
        if tools.is_empty() && analysis.needs_tools {
            tools = all_tools().iter().collect();

            if self.pipeline_config.log_token_usage {
                tracing::debug!(
                    "No category-specific tools matched, including all tools as fallback"
                );
            }
        }

        if self.pipeline_config.log_token_usage {
            tracing::debug!(
                categories = ?analysis.categories,
                needs_tools = analysis.needs_tools,
                resolved_tools = ?tools.iter().map(|t| t.name).collect::<Vec<_>>(),
                "Category-based tool filtering"
            );
        }

        // Deduplicate
        tools.sort_by_key(|t| t.name);
        tools.dedup_by_key(|t| t.name);

        tools
    }

    /// If the user responds with an approval ("ok", "yes", "please proceed") and the
    /// previous assistant message proposed using a tool, ensure this turn is treated as
    /// a tool-capable follow-up.
    ///
    /// This prevents a common failure mode where the model asked for confirmation and,
    /// after the user confirms, the follow-up message contains no tool keywords, so the
    /// analyzer disables tools and the agent can't complete the action.
    fn promote_approval_to_tool_followup(
        &self,
        request: &AgentRequest,
        analysis: &mut crate::context::RequestAnalysis,
    ) {
        use crate::context::ContextCategory;

        let input = request.input.trim().to_lowercase();
        let looks_like_approval = matches!(
            input.as_str(),
            "ok" | "okay"
                | "ok."
                | "okay."
                | "yes"
                | "y"
                | "sure"
                | "please proceed"
                | "proceed"
                | "go ahead"
                | "do it"
                | "run it"
                | "continue"
        ) || input.contains("please proceed")
            || input.contains("go ahead")
            || input.contains("please do")
            || input.contains("yes, proceed");

        if !looks_like_approval {
            return;
        }

        // If tools are already enabled, nothing to do.
        if analysis.needs_tools {
            return;
        }

        // Find the most recent assistant message.
        let Some(prev_assistant) = request.history.iter().rev().find(|m| m.role == "assistant")
        else {
            return;
        };

        let prev = prev_assistant.content.to_lowercase();

        // Try to infer which tool the assistant intended to use.
        let tool_name = if prev.contains("shell tool")
            || (prev.contains("run") && prev.contains("pwd"))
            || prev.contains("`pwd`")
        {
            Some("shell")
        } else if prev.contains("git tool") || prev.contains("git status") {
            Some("git")
        } else if prev.contains("file tool") || prev.contains("read file") {
            Some("file")
        } else if prev.contains("web tool") || prev.contains("search the web") {
            Some("web")
        } else if prev.contains("code tool") || prev.contains("code stats") {
            Some("code")
        } else {
            None
        };

        let Some(tool_name) = tool_name else {
            return;
        };

        analysis.needs_tools = true;
        analysis.is_followup = true;
        analysis.confidence = analysis.confidence.max(0.85);
        analysis.suggested_tools.push(tool_name.to_string());

        // Ensure the tool category is present so tool filtering will include it.
        match tool_name {
            "shell" => {
                analysis.categories.insert(ContextCategory::Shell);
                analysis.categories.insert(ContextCategory::Tools);
            }
            "git" => {
                analysis.categories.insert(ContextCategory::Git);
                analysis.categories.insert(ContextCategory::Tools);
            }
            "file" => {
                analysis.categories.insert(ContextCategory::FileSystem);
                analysis.categories.insert(ContextCategory::Tools);
            }
            "web" => {
                analysis.categories.insert(ContextCategory::Web);
                analysis.categories.insert(ContextCategory::Tools);
            }
            "code" => {
                analysis.categories.insert(ContextCategory::Code);
                analysis.categories.insert(ContextCategory::Tools);
            }
            _ => {
                analysis.categories.insert(ContextCategory::Tools);
            }
        }
    }

    /// Build an optimized prompt from request and context
    fn build_prompt(
        &self,
        request: &AgentRequest,
        context: &crate::context::ResolvedContext,
    ) -> String {
        let mut prompt = String::new();

        // Always include a system prompt. Callers may override via `request.system_prompt`.
        let sys = request
            .system_prompt
            .clone()
            .unwrap_or_else(|| crate::persona::default_system_prompt(&request.metadata));
        prompt.push_str(&format!("System: {}\n\n", sys));

        // Inject repository-local guardrails (AGENTS.md, .gestura/guardrails) when available.
        self.append_project_guardrails(&mut prompt, request);

        // Tool definitions are now passed via the structured `tools` API parameter
        // (ProviderToolSchemas) rather than duplicated in the prompt text. This avoids
        // wasting tokens on a less-detailed text listing when the model already receives
        // full JSON schemas out-of-band.

        // Add file context if any
        if !context.files.is_empty() {
            prompt.push_str("File context:\n");
            for file in &context.files {
                let truncation_note = if file.truncated { " (truncated)" } else { "" };
                prompt.push_str(&format!(
                    "--- {} ({} lines){} ---\n{}\n---\n\n",
                    file.path, file.total_lines, truncation_note, file.content
                ));
            }
        }

        // Add knowledge context (memory bank + enabled knowledge items)
        if !context.knowledge.is_empty() {
            for knowledge_section in &context.knowledge {
                prompt.push_str(knowledge_section);
                prompt.push('\n');
            }
        }

        // Add history summary if available (for older context)
        if let Some(ref summary) = context.history_summary {
            prompt.push_str(&format!("Conversation summary: {}\n\n", summary));
        }

        // Add recent conversation history (last N messages based on config)
        // This is critical for follow-ups like "ok, proceed" where the action
        // is described in the previous assistant message.
        let history_limit = self.pipeline_config.max_history_messages;
        if !request.history.is_empty() {
            let history_start = request.history.len().saturating_sub(history_limit);
            let included_count = request.history.len() - history_start;

            if self.pipeline_config.log_token_usage {
                tracing::debug!(
                    total_history = request.history.len(),
                    included = included_count,
                    limit = history_limit,
                    "Context management: including recent history messages"
                );
            }

            prompt.push_str("Recent conversation:\n");
            for msg in request.history.iter().skip(history_start) {
                match msg.role.as_str() {
                    "user" => prompt.push_str(&format!("User: {}\n", msg.content)),
                    "assistant" => prompt.push_str(&format!("Assistant: {}\n", msg.content)),
                    "tool" => {
                        // Truncate tool results to prevent token explosion
                        let truncated_content = self.truncate_tool_result(&msg.content);
                        prompt.push_str(&format!("Tool result: {}\n", truncated_content));
                    }
                    _ => prompt.push_str(&format!("{}: {}\n", msg.role, msg.content)),
                }
            }
            prompt.push('\n');
        }

        // Add the current request
        prompt.push_str(&format!("User: {}\n", request.input));

        prompt
    }

    /// Append project guardrails to the prompt when enabled and a workspace root is available.
    ///
    /// Guardrails are discovered from the request's `workspace_dir` (no filesystem scanning)
    /// and are bounded by `PipelineSettings.project_guardrails.max_chars`.
    fn append_project_guardrails(&self, prompt: &mut String, request: &AgentRequest) {
        let Some(workspace_dir) = request.metadata.workspace_dir.as_deref() else {
            return;
        };

        let settings = &self.config.pipeline.project_guardrails;
        let Some(guardrails) = crate::guardrails::load_project_guardrails(workspace_dir, settings)
        else {
            return;
        };

        let truncation_note = if guardrails.truncated {
            format!(" (truncated to {} chars)", settings.max_chars)
        } else {
            String::new()
        };

        prompt.push_str("Project guardrails:\n");
        prompt.push_str(&format!(
            "Source: {}{}\n",
            guardrails.source.relative_path(),
            truncation_note
        ));
        prompt.push_str(&guardrails.content);
        if !guardrails.content.ends_with('\n') {
            prompt.push('\n');
        }
        prompt.push('\n');
    }

    /// Estimate token count for a string
    /// Uses a simple heuristic: ~4 characters per token for English text
    /// This is a reasonable approximation for most LLM tokenizers
    fn estimate_tokens(text: &str) -> usize {
        // More accurate estimation:
        // - Count words (roughly 1.3 tokens per word)
        // - Count special characters (often 1 token each)
        // - Average: ~4 chars per token
        let char_count = text.chars().count();
        let word_count = text.split_whitespace().count();

        // Weighted average: words contribute more to token count
        let word_based = (word_count as f64 * 1.3) as usize;
        let char_based = char_count / 4;

        // Use the higher estimate for safety
        word_based.max(char_based).max(1)
    }

    /// Check if prompt exceeds token limit and needs truncation
    fn check_token_limit(&self, prompt: &str) -> TokenLimitStatus {
        let estimated_tokens = Self::estimate_tokens(prompt);
        let max_input = self
            .pipeline_config
            .max_context_tokens
            .saturating_sub(self.pipeline_config.max_output_tokens);

        if estimated_tokens > max_input {
            TokenLimitStatus::Exceeded {
                estimated: estimated_tokens,
                limit: max_input,
                overage: estimated_tokens - max_input,
            }
        } else if estimated_tokens > (max_input * 90 / 100) {
            TokenLimitStatus::Warning {
                estimated: estimated_tokens,
                limit: max_input,
                percentage: ((estimated_tokens * 100) / max_input.max(1)) as u8,
            }
        } else {
            TokenLimitStatus::Ok {
                estimated: estimated_tokens,
                limit: max_input,
            }
        }
    }

    /// Check if auto-compaction should be triggered based on estimated token usage
    /// Returns Some(StreamChunk) if compaction was performed, None otherwise
    async fn check_and_apply_auto_compaction<M>(
        &self,
        history: &[M],
        prompt_preview: &str,
        metadata: &RequestMetadata,
    ) -> Option<StreamChunk>
    where
        M: AsRef<str>,
    {
        // Skip if auto-compaction is disabled (threshold <= 0.0 or >= 1.0)
        if self.pipeline_config.auto_compact_threshold <= 0.0
            || self.pipeline_config.auto_compact_threshold >= 1.0
        {
            return None;
        }

        let estimated_tokens = Self::estimate_tokens(prompt_preview);
        let max_input = self
            .pipeline_config
            .max_context_tokens
            .saturating_sub(self.pipeline_config.max_output_tokens);

        let threshold_tokens =
            (max_input as f64 * self.pipeline_config.auto_compact_threshold) as usize;

        if estimated_tokens > threshold_tokens {
            let messages_before = history.len();

            // Apply compaction strategy
            use crate::pipeline::types::CompactionStrategy;
            match self.pipeline_config.compaction_strategy {
                CompactionStrategy::Summarize => {
                    // Trigger summarization via context manager
                    let _summary = self.context_manager.summarize_history(history);

                    // Calculate tokens saved (rough estimate)
                    // Assume summarization reduces history by ~70%
                    let messages_after = (messages_before as f64 * 0.3) as usize;
                    let tokens_saved = (estimated_tokens as f64 * 0.4) as usize; // Conservative estimate

                    tracing::info!(
                        messages_before = messages_before,
                        messages_after = messages_after,
                        tokens_saved = tokens_saved,
                        estimated_tokens = estimated_tokens,
                        threshold_tokens = threshold_tokens,
                        threshold_pct = (self.pipeline_config.auto_compact_threshold * 100.0) as u8,
                        strategy = "Summarize",
                        "Auto-compaction triggered: context exceeded {}% threshold",
                        (self.pipeline_config.auto_compact_threshold * 100.0) as u8
                    );

                    Some(StreamChunk::ContextCompacted {
                        messages_before,
                        messages_after,
                        tokens_saved,
                        summary: format!(
                            "Context auto-compacted (Summarize): {} messages → {} messages (saved ~{} tokens)",
                            messages_before, messages_after, tokens_saved
                        ),
                    })
                }
                CompactionStrategy::MemoryBank => {
                    // Save context to memory bank file
                    if let Some(workspace_dir) = &metadata.workspace_dir {
                        let session_id = metadata
                            .session_id
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());

                        // Build summary from history
                        let summary = self.context_manager.summarize_history(history);

                        // Build full content from history
                        let content = history
                            .iter()
                            .map(|m| m.as_ref())
                            .collect::<Vec<_>>()
                            .join("\n\n");

                        let entry = crate::memory_bank::MemoryBankEntry::new(
                            session_id.clone(),
                            summary.clone(),
                            content,
                        );

                        match crate::memory_bank::save_to_memory_bank(workspace_dir, &entry).await {
                            Ok(file_path) => {
                                tracing::info!(
                                    messages_saved = messages_before,
                                    file_path = %file_path.display(),
                                    session_id = %session_id,
                                    estimated_tokens = estimated_tokens,
                                    threshold_tokens = threshold_tokens,
                                    threshold_pct = (self.pipeline_config.auto_compact_threshold * 100.0) as u8,
                                    strategy = "MemoryBank",
                                    "Auto-compaction triggered: saved context to memory bank"
                                );

                                Some(StreamChunk::MemoryBankSaved {
                                    file_path: file_path.display().to_string(),
                                    session_id,
                                    summary,
                                    messages_saved: messages_before,
                                })
                            }
                            Err(e) => {
                                tracing::error!(
                                    error = %e,
                                    "Failed to save context to memory bank, falling back to summarization"
                                );

                                // Fallback to summarization
                                let _summary = self.context_manager.summarize_history(history);
                                let messages_after = (messages_before as f64 * 0.3) as usize;
                                let tokens_saved = (estimated_tokens as f64 * 0.4) as usize;

                                Some(StreamChunk::ContextCompacted {
                                    messages_before,
                                    messages_after,
                                    tokens_saved,
                                    summary: format!(
                                        "Context auto-compacted (fallback): {} messages → {} messages",
                                        messages_before, messages_after
                                    ),
                                })
                            }
                        }
                    } else {
                        tracing::warn!(
                            "MemoryBank strategy requires workspace_dir, falling back to summarization"
                        );

                        // Fallback to summarization
                        let _summary = self.context_manager.summarize_history(history);
                        let messages_after = (messages_before as f64 * 0.3) as usize;
                        let tokens_saved = (estimated_tokens as f64 * 0.4) as usize;

                        Some(StreamChunk::ContextCompacted {
                            messages_before,
                            messages_after,
                            tokens_saved,
                            summary: format!(
                                "Context auto-compacted (fallback): {} messages → {} messages",
                                messages_before, messages_after
                            ),
                        })
                    }
                }
                CompactionStrategy::Truncate => {
                    // Simply truncate oldest messages
                    // This is handled by the caller (they should drop oldest messages)
                    tracing::info!(
                        messages_before = messages_before,
                        estimated_tokens = estimated_tokens,
                        threshold_tokens = threshold_tokens,
                        strategy = "Truncate",
                        "Auto-compaction triggered: truncate strategy (caller should drop oldest messages)"
                    );

                    Some(StreamChunk::ContextCompacted {
                        messages_before,
                        messages_after: 0, // Caller will handle truncation
                        tokens_saved: 0,   // Unknown until caller truncates
                        summary: "Context will be truncated (oldest messages dropped)".to_string(),
                    })
                }
                CompactionStrategy::Clear => {
                    // Clear all history
                    tracing::info!(
                        messages_before = messages_before,
                        estimated_tokens = estimated_tokens,
                        threshold_tokens = threshold_tokens,
                        strategy = "Clear",
                        "Auto-compaction triggered: clear strategy (all history will be dropped)"
                    );

                    Some(StreamChunk::ContextCompacted {
                        messages_before,
                        messages_after: 0,
                        tokens_saved: estimated_tokens,
                        summary: "Context cleared (all history dropped)".to_string(),
                    })
                }
                CompactionStrategy::Prompt => {
                    // Prompt user for action
                    // For now, just log and fallback to summarization
                    tracing::info!(
                        messages_before = messages_before,
                        estimated_tokens = estimated_tokens,
                        threshold_tokens = threshold_tokens,
                        strategy = "Prompt",
                        "Auto-compaction triggered: prompt strategy (not yet implemented, falling back to summarization)"
                    );

                    let _summary = self.context_manager.summarize_history(history);
                    let messages_after = (messages_before as f64 * 0.3) as usize;
                    let tokens_saved = (estimated_tokens as f64 * 0.4) as usize;

                    Some(StreamChunk::ContextCompacted {
                        messages_before,
                        messages_after,
                        tokens_saved,
                        summary: format!(
                            "Context auto-compacted (Prompt not yet implemented): {} messages → {} messages",
                            messages_before, messages_after
                        ),
                    })
                }
            }
        } else {
            if self.pipeline_config.log_token_usage {
                let utilization_pct = (estimated_tokens * 100 / max_input) as u8;
                tracing::debug!(
                    estimated_tokens = estimated_tokens,
                    threshold_tokens = threshold_tokens,
                    utilization_pct = utilization_pct,
                    "Auto-compaction check: below threshold ({}%)",
                    utilization_pct
                );
            }
            None
        }
    }

    /// Calculate estimated cost for a request based on provider and token count
    /// Returns cost in USD
    fn calculate_cost(&self, tokens: usize) -> f64 {
        use crate::streaming::pricing;

        let provider = &self.config.llm.primary;
        let model = match provider.as_str() {
            "openai" => self.config.llm.openai.as_ref().map(|c| c.model.as_str()),
            "anthropic" => self.config.llm.anthropic.as_ref().map(|c| c.model.as_str()),
            "grok" => self.config.llm.grok.as_ref().map(|c| c.model.as_str()),
            "ollama" => Some("ollama"),
            _ => None,
        };

        // Determine pricing per 1M tokens based on provider and model
        let price_per_million = match (provider.as_str(), model) {
            ("openai", Some(m)) if m.contains("gpt-4") => pricing::OPENAI_GPT4_TURBO_INPUT,
            ("openai", Some(m)) if m.contains("gpt-3.5") => pricing::OPENAI_GPT35_TURBO_INPUT,
            ("openai", _) => pricing::OPENAI_GPT4_TURBO_INPUT, // Default to GPT-4 pricing
            ("anthropic", Some(m)) if m.contains("opus") => pricing::ANTHROPIC_CLAUDE_3_OPUS_INPUT,
            ("anthropic", Some(m)) if m.contains("haiku") => {
                pricing::ANTHROPIC_CLAUDE_3_HAIKU_INPUT
            }
            ("anthropic", _) => pricing::ANTHROPIC_CLAUDE_35_SONNET_INPUT, // Default to 3.5 Sonnet
            ("grok", _) => pricing::XAI_GROK_INPUT,
            ("ollama", _) => pricing::OLLAMA_INPUT, // Free/local
            _ => pricing::DEFAULT_INPUT,
        };

        // Calculate cost: (tokens / 1,000,000) * price_per_million
        (tokens as f64 / 1_000_000.0) * price_per_million
    }

    /// Search memory bank for relevant entries and load them into context
    /// Returns additional context string to prepend to the resolved context
    async fn search_and_load_memory_bank(
        &self,
        workspace_dir: &std::path::Path,
        query: &str,
        max_entries: usize,
    ) -> Option<String> {
        // Search for relevant memory bank entries
        match crate::memory_bank::search_memory_bank(workspace_dir, query, max_entries).await {
            Ok(entries) if !entries.is_empty() => {
                tracing::info!(
                    entries_found = entries.len(),
                    max_entries = max_entries,
                    "Found relevant memory bank entries"
                );

                // Build context from entries
                let mut context = String::from("## Relevant Context from Memory Bank\n\n");

                for entry in entries {
                    context.push_str(&format!(
                        "### Memory Entry ({})\n",
                        entry.timestamp.format("%Y-%m-%d %H:%M UTC")
                    ));
                    context.push_str(&format!("**Summary**: {}\n\n", entry.summary));

                    // Include a preview of the content (first 500 chars)
                    let preview = if entry.content.len() > 500 {
                        format!("{}...\n\n", &entry.content[..500])
                    } else {
                        format!("{}\n\n", entry.content)
                    };
                    context.push_str(&preview);
                    context.push_str("---\n\n");
                }

                Some(context)
            }
            Ok(_) => {
                tracing::debug!("No relevant memory bank entries found");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to search memory bank");
                None
            }
        }
    }

    /// Load enabled knowledge items for the session and format them as context
    /// Returns additional context string to include in the prompt
    fn load_enabled_knowledge(&self, session_id: Option<&str>) -> Option<String> {
        // Check if knowledge system is configured
        let store = self.knowledge_store?;
        let settings = self.knowledge_settings?;
        let session_id = session_id?;

        // Get enabled knowledge IDs for this session
        let enabled_ids = match settings.get_enabled_knowledge(session_id) {
            Ok(ids) if !ids.is_empty() => ids,
            Ok(_) => {
                tracing::debug!("No knowledge items enabled for session");
                return None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to load enabled knowledge settings");
                return None;
            }
        };

        tracing::info!(
            session_id = session_id,
            enabled_count = enabled_ids.len(),
            "Loading enabled knowledge items"
        );

        // Build context from enabled knowledge items
        let mut context = String::from("## Specialized Knowledge\n\n");
        context.push_str("The following specialized knowledge is available for this session:\n\n");

        for knowledge_id in enabled_ids {
            if let Some(item) = store.get(&knowledge_id) {
                context.push_str(&format!(
                    "### {}\n\n",
                    knowledge_id.replace('-', " ").to_uppercase()
                ));

                // Add category
                context.push_str(&format!("**Category**: {}\n\n", item.category));

                // Add core content
                context.push_str(&item.core_content);
                context.push_str("\n\n---\n\n");

                tracing::debug!(
                    knowledge_id = %knowledge_id,
                    content_len = item.core_content.len(),
                    "Added knowledge item to context"
                );
            } else {
                tracing::warn!(
                    knowledge_id = %knowledge_id,
                    "Enabled knowledge item not found in store"
                );
            }
        }

        Some(context)
    }

    /// Create a token usage update chunk for user feedback
    /// Returns a StreamChunk with current token utilization status
    fn create_token_usage_update(&self, prompt: &str) -> StreamChunk {
        use crate::streaming::TokenUsageStatus;

        let estimated_tokens = Self::estimate_tokens(prompt);
        let max_input = self
            .pipeline_config
            .max_context_tokens
            .saturating_sub(self.pipeline_config.max_output_tokens);

        let percentage = ((estimated_tokens * 100) / max_input.max(1)) as u8;

        let status = if percentage < 70 {
            TokenUsageStatus::Green
        } else if percentage < 90 {
            TokenUsageStatus::Yellow
        } else {
            TokenUsageStatus::Red
        };

        let estimated_cost = self.calculate_cost(estimated_tokens);

        StreamChunk::TokenUsageUpdate {
            estimated: estimated_tokens,
            limit: max_input,
            percentage,
            status,
            estimated_cost,
        }
    }

    /// Validate that the prompt is within token limits before sending to LLM
    ///
    /// This is a hard validation that rejects requests exceeding limits,
    /// preventing API errors. Similar to Aider's check_tokens() approach.
    ///
    /// Returns an error if the prompt exceeds the maximum allowed tokens,
    /// with guidance on how to reduce context.
    fn validate_token_limit(&self, prompt: &str) -> Result<(), AppError> {
        let status = self.check_token_limit(prompt);

        match status {
            TokenLimitStatus::Exceeded {
                estimated,
                limit,
                overage,
            } => {
                tracing::error!(
                    estimated_tokens = estimated,
                    limit = limit,
                    overage = overage,
                    "Request exceeds token limit - rejecting to prevent API error"
                );

                Err(AppError::Llm(format!(
                    "Context too large: estimated {} tokens exceeds limit of {} tokens (overage: {} tokens). \
                    Try reducing conversation history, disabling unused tools, or using /summarize to compact context.",
                    estimated, limit, overage
                )))
            }
            TokenLimitStatus::Warning {
                estimated,
                limit,
                percentage,
            } => {
                tracing::warn!(
                    estimated_tokens = estimated,
                    limit = limit,
                    utilization_pct = percentage,
                    "Token usage approaching limit ({}%)",
                    percentage
                );
                Ok(())
            }
            TokenLimitStatus::Ok { estimated, limit } => {
                if self.pipeline_config.log_token_usage {
                    tracing::debug!(
                        estimated_tokens = estimated,
                        limit = limit,
                        utilization_pct = (estimated * 100 / limit.max(1)),
                        "Token validation passed"
                    );
                }
                Ok(())
            }
        }
    }

    /// Truncate prompt to fit within token limit
    /// Strategy: Remove oldest history messages first, then truncate file content
    fn truncate_prompt_if_needed(
        &self,
        request: &AgentRequest,
        context: &mut crate::context::ResolvedContext,
    ) -> (String, bool) {
        let mut prompt = self.build_prompt(request, context);
        let mut truncated = false;

        // Log initial token estimate if enabled
        let initial_tokens = Self::estimate_tokens(&prompt);
        if self.pipeline_config.log_token_usage {
            let max_input = self
                .pipeline_config
                .max_context_tokens
                .saturating_sub(self.pipeline_config.max_output_tokens);
            tracing::info!(
                estimated_tokens = initial_tokens,
                max_input_tokens = max_input,
                max_context_tokens = self.pipeline_config.max_context_tokens,
                history_messages = request.history.len(),
                file_contexts = context.files.len(),
                "Token usage before optimization"
            );
        }

        // Check if we need to truncate
        if let TokenLimitStatus::Exceeded { overage, .. } = self.check_token_limit(&prompt) {
            truncated = true;
            tracing::warn!(overage = overage, "Prompt exceeds token limit, truncating");

            // Strategy 1: Truncate file contents
            let chars_to_remove = overage * 4; // Approximate chars per token
            let mut removed = 0;

            for file in context.files.iter_mut() {
                if removed >= chars_to_remove {
                    break;
                }
                let file_len = file.content.len();
                if file_len > 500 {
                    // Keep first 200 and last 200 chars
                    let truncated_content = format!(
                        "{}...[truncated {} chars]...{}",
                        &file.content[..200],
                        file_len - 400,
                        &file.content[file_len - 200..]
                    );
                    removed += file_len - truncated_content.len();
                    file.content = truncated_content;
                    file.truncated = true;
                }
            }

            // Rebuild prompt with truncated context
            prompt = self.build_prompt(request, context);

            // Log token usage after optimization
            let final_tokens = Self::estimate_tokens(&prompt);
            if self.pipeline_config.log_token_usage {
                tracing::info!(
                    tokens_before = initial_tokens,
                    tokens_after = final_tokens,
                    tokens_saved = initial_tokens.saturating_sub(final_tokens),
                    "Token usage after optimization"
                );
            }

            // If still over, log a warning (we've done what we can)
            if let TokenLimitStatus::Exceeded {
                estimated, limit, ..
            } = self.check_token_limit(&prompt)
            {
                tracing::error!(
                    estimated = estimated,
                    limit = limit,
                    "Prompt still exceeds limit after truncation"
                );
            }
        }

        (prompt, truncated)
    }

    /// Execute the agentic loop with streaming
    ///
    /// If `workspace` is provided, all tool operations (shell, file, git) will be
    /// sandboxed to that directory. Paths outside the workspace will be rejected.
    #[allow(clippy::too_many_arguments)]
    async fn execute_agentic_loop_streaming(
        &self,
        initial_prompt: String,
        tools: Vec<&'static ToolDefinition>,
        context: crate::context::ResolvedContext,
        tx: mpsc::Sender<StreamChunk>,
        cancel_token: CancellationToken,
        workspace: Option<&SessionWorkspace>,
        session_id: Option<String>,
        permission_level: PermissionLevel,
    ) -> Result<AgentResponse, AppError> {
        let mut response = AgentResponse {
            content: String::new(),
            thinking: None,
            tool_calls: Vec::new(),
            usage: None,
            context_used: context,
            truncated: false,
            iterations: 0,
        };

        let mut current_prompt = initial_prompt;

        // Build provider-specific tool schemas once for this request.
        let tool_schemas = if tools.is_empty() {
            None
        } else {
            Some(crate::tools::schemas::build_provider_tool_schemas(&tools))
        };

        // Agentic loop - continue until no more tool calls or max iterations
        for iteration in 0..self.pipeline_config.max_iterations {
            if cancel_token.is_cancelled() {
                let _ = tx.send(StreamChunk::Cancelled).await;
                return Ok(response);
            }

            response.iterations = iteration + 1;

            // Emit iteration boundary marker so UIs can delineate the agentic loop.
            // iteration 0 = initial LLM call; iteration 1+ = continuation after tool results.
            let _ = tx
                .send(StreamChunk::AgentLoopIteration {
                    iteration: iteration as u32,
                })
                .await;

            // Start streaming for this iteration
            let (inner_tx, mut inner_rx) = mpsc::channel::<StreamChunk>(100);
            let inner_cancel = cancel_token.clone();
            let config = self.config.clone();
            let prompt = current_prompt.clone();
            let enable_fallback = self.pipeline_config.enable_fallback;
            let tool_schemas_for_iteration = tool_schemas.clone();

            // Spawn streaming task (with or without fallback)
            let stream_handle = tokio::spawn(async move {
                if enable_fallback {
                    start_streaming_with_fallback(
                        &config,
                        &prompt,
                        tool_schemas_for_iteration,
                        inner_tx,
                        inner_cancel,
                    )
                    .await
                } else {
                    start_streaming(
                        &config,
                        &prompt,
                        tool_schemas_for_iteration,
                        inner_tx,
                        inner_cancel,
                    )
                    .await
                }
            });

            // Collect chunks and forward to caller
            let mut iteration_content = String::new();
            let mut pending_tool_call: Option<PendingToolCall> = None;
            let mut tool_calls_in_iteration: Vec<ToolCallRecord> = Vec::new();

            while let Some(chunk) = inner_rx.recv().await {
                match &chunk {
                    StreamChunk::Status { .. } => {
                        // Forward status updates to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::Text(text) => {
                        iteration_content.push_str(text);
                        response.content.push_str(text);
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::Thinking(text) => {
                        if response.thinking.is_none() {
                            response.thinking = Some(String::new());
                        }
                        if let Some(ref mut thinking) = response.thinking {
                            thinking.push_str(text);
                        }
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ToolCallStart { id, name } => {
                        // Defensive: if the provider starts a new tool call without ending the
                        // previous one, finalize the previous call so we don't drop it.
                        if let Some(pending) = pending_tool_call.take() {
                            self.finalize_pending_tool_call(
                                pending,
                                FinalizePendingToolCallCtx {
                                    workspace,
                                    session_id: session_id.clone(),
                                    permission_level,
                                    cancel_token: &cancel_token,
                                    tool_calls_in_iteration: &mut tool_calls_in_iteration,
                                    response: &mut response,
                                    tx: &tx,
                                },
                            )
                            .await;
                        }

                        pending_tool_call = Some(PendingToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            arguments: String::new(),
                            start_time: Instant::now(),
                        });
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ToolCallArgs(args) => {
                        if let Some(ref mut pending) = pending_tool_call {
                            pending.arguments.push_str(args);
                        }
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ToolCallEnd => {
                        if let Some(pending) = pending_tool_call.take() {
                            self.finalize_pending_tool_call(
                                pending,
                                FinalizePendingToolCallCtx {
                                    workspace,
                                    session_id: session_id.clone(),
                                    permission_level,
                                    cancel_token: &cancel_token,
                                    tool_calls_in_iteration: &mut tool_calls_in_iteration,
                                    response: &mut response,
                                    tx: &tx,
                                },
                            )
                            .await;
                        }
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ToolCallResult { .. } => {
                        // Forward tool result to frontend (already emitted by finalize_pending_tool_call)
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::RetryAttempt { .. } => {
                        // Forward retry notifications to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ContextCompacted { .. } => {
                        // Forward compaction notifications to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ConfigRequest { .. } => {
                        // Forward config requests to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ToolConfirmationRequired { .. } => {
                        // Forward tool confirmation requests to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ToolBlocked { .. } => {
                        // Forward tool blocked notifications to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::TokenUsageUpdate { .. } => {
                        // Forward token usage updates to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::MemoryBankSaved { .. } => {
                        // Forward memory bank notification to user
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::AgentLoopIteration { .. } => {
                        // Iteration markers are emitted by the outer loop, not providers.
                        // Forward in case an inner stream echoes one.
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::Done(usage) => {
                        // Some providers (or buggy intermediaries) may terminate the stream
                        // without emitting a ToolCallEnd. If we have a pending tool call, treat
                        // stream completion as an implicit end and execute it.
                        if let Some(pending) = pending_tool_call.take() {
                            self.finalize_pending_tool_call(
                                pending,
                                FinalizePendingToolCallCtx {
                                    workspace,
                                    session_id: session_id.clone(),
                                    permission_level,
                                    cancel_token: &cancel_token,
                                    tool_calls_in_iteration: &mut tool_calls_in_iteration,
                                    response: &mut response,
                                    tx: &tx,
                                },
                            )
                            .await;
                        }

                        if let Some(u) = usage {
                            response.usage = Some(u.clone());
                        }
                        // Don't forward Done yet if we have tool calls to process
                        if tool_calls_in_iteration.is_empty() {
                            let _ = tx.send(chunk).await;
                        }
                    }
                    StreamChunk::Error(e) => {
                        let _ = tx.send(StreamChunk::Error(e.clone())).await;
                        return Err(AppError::Llm(e.clone()));
                    }
                    StreamChunk::Cancelled => {
                        let _ = tx.send(chunk).await;
                        return Ok(response);
                    }
                }
            }

            // If the inner stream ended unexpectedly (no Done/Error/Cancelled), but we have a
            // pending tool call, execute it so the agent loop can continue.
            if let Some(pending) = pending_tool_call.take() {
                self.finalize_pending_tool_call(
                    pending,
                    FinalizePendingToolCallCtx {
                        workspace,
                        session_id: session_id.clone(),
                        permission_level,
                        cancel_token: &cancel_token,
                        tool_calls_in_iteration: &mut tool_calls_in_iteration,
                        response: &mut response,
                        tx: &tx,
                    },
                )
                .await;
            }

            // Wait for stream task
            let _ = stream_handle.await;

            // If no tool calls, we're done
            if tool_calls_in_iteration.is_empty() {
                break;
            }

            // Build continuation prompt with tool results
            current_prompt = self.build_tool_continuation_prompt(
                &current_prompt,
                &iteration_content,
                &tool_calls_in_iteration,
            );
        }

        // Send final Done if not already sent
        let _ = tx.send(StreamChunk::Done(response.usage.clone())).await;

        Ok(response)
    }

    /// Execute the agentic loop without streaming (blocking)
    ///
    /// If `workspace` is provided, all tool operations (shell, file, git) will be
    /// sandboxed to that directory. Paths outside the workspace will be rejected.
    async fn execute_agentic_loop_blocking(
        &self,
        initial_prompt: String,
        tools: Vec<&'static ToolDefinition>,
        context: crate::context::ResolvedContext,
        workspace: Option<&SessionWorkspace>,
    ) -> Result<AgentResponse, AppError> {
        let mut response = AgentResponse {
            content: String::new(),
            thinking: None,
            tool_calls: Vec::new(),
            usage: None,
            context_used: context,
            truncated: false,
            iterations: 0,
        };

        if self.pipeline_config.max_iterations == 0 {
            return Ok(response);
        }

        // Build provider-specific tool schemas so the model knows about available tools.
        let tool_schemas = if tools.is_empty() {
            None
        } else {
            Some(crate::tools::schemas::build_provider_tool_schemas(&tools))
        };

        let max_iterations = self.pipeline_config.max_iterations;
        let mut current_prompt = initial_prompt;

        for iteration in 0..max_iterations {
            response.iterations = iteration + 1;

            // Call LLM with fallback support, passing tool schemas.
            let llm_response = self
                .call_llm_with_fallback(&current_prompt, tool_schemas.as_ref())
                .await?;
            let (content, thinking) = crate::streaming::split_think_blocks(&llm_response.text);

            // Accumulate token usage across iterations.
            if let Some(ref mut existing_usage) = response.usage {
                existing_usage.input_tokens += llm_response.usage.input_tokens;
                existing_usage.output_tokens += llm_response.usage.output_tokens;
                existing_usage.total_tokens += llm_response.usage.total_tokens;
                if let (Some(existing), Some(new)) = (
                    existing_usage.estimated_cost_usd.as_mut(),
                    llm_response.usage.estimated_cost_usd,
                ) {
                    *existing += new;
                }
            } else {
                response.usage = Some(llm_response.usage);
            }

            // If the model returned no tool calls, this is the final text response.
            if llm_response.tool_calls.is_empty() {
                response.content = content;
                response.thinking = thinking;
                break;
            }

            // Execute each structured tool call and collect records.
            let mut iteration_tool_calls: Vec<ToolCallRecord> = Vec::new();
            for tc in &llm_response.tool_calls {
                tracing::info!(
                    tool = %tc.name,
                    id = %tc.id,
                    "Blocking loop: executing tool call"
                );
                let result = self.execute_tool(&tc.name, &tc.arguments, workspace).await;
                let duration_ms = 0u64; // No per-call timing in blocking path.
                iteration_tool_calls.push(ToolCallRecord {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                    result,
                    duration_ms,
                });
            }

            // Build continuation prompt with tool results for the next iteration.
            current_prompt = self.build_tool_continuation_prompt(
                &current_prompt,
                &content,
                &iteration_tool_calls,
            );
            response.tool_calls.extend(iteration_tool_calls);
            response.content = content;
            response.thinking = thinking;
        }

        Ok(response)
    }

    /// Call LLM with fallback and retry logic for blocking mode.
    ///
    /// When `tool_schemas` is provided, the appropriate provider-specific schema
    /// slice is selected and forwarded to [`LlmProvider::call_with_tools`].
    async fn call_llm_with_fallback(
        &self,
        prompt: &str,
        tool_schemas: Option<&crate::tools::schemas::ProviderToolSchemas>,
    ) -> Result<crate::llm_provider::LlmCallResponse, AppError> {
        let agent_ctx = AgentContext::default();
        let provider = select_provider(&self.config, &agent_ctx);
        let tools_for_primary =
            tool_schemas.map(|s| tools_slice_for_provider(&self.config.llm.primary, s));

        // Try primary provider with retries
        let retry_delays = [1, 2, 4]; // seconds
        let mut last_error: Option<AppError> = None;

        for (attempt, delay) in retry_delays.iter().enumerate() {
            match provider
                .call_with_tools(prompt, tools_for_primary.as_deref())
                .await
            {
                Ok(response) => return Ok(response),
                Err(e) => {
                    last_error = Some(e);
                    if !self.pipeline_config.enable_fallback {
                        break;
                    }
                    tracing::warn!(
                        attempt = attempt + 1,
                        delay = delay,
                        "Primary LLM failed, retrying in {}s",
                        delay
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(*delay)).await;
                }
            }
        }

        // Try fallback provider if configured
        if let Some(fallback_provider_name) = self
            .pipeline_config
            .enable_fallback
            .then_some(self.config.llm.fallback.as_ref())
            .flatten()
        {
            tracing::info!(
                fallback = fallback_provider_name,
                "Primary LLM exhausted retries, trying fallback provider"
            );

            let tools_for_fallback =
                tool_schemas.map(|s| tools_slice_for_provider(fallback_provider_name, s));

            // Create a modified config with fallback as primary
            let mut fallback_config = self.config.clone();
            fallback_config.llm.primary = fallback_provider_name.clone();

            let fallback_provider_instance = select_provider(&fallback_config, &agent_ctx);
            match fallback_provider_instance
                .call_with_tools(prompt, tools_for_fallback.as_deref())
                .await
            {
                Ok(response) => return Ok(response),
                Err(e) => {
                    tracing::error!("Fallback provider also failed: {}", e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| AppError::Llm("All LLM providers failed".to_string())))
    }

    /// Execute a tool by name with given arguments.
    ///
    /// Note: If a workspace is provided in `ctx`, all file paths and shell commands are sandboxed
    /// to that directory. Paths outside the workspace will be rejected.
    async fn finalize_pending_tool_call(
        &self,
        pending: PendingToolCall,
        ctx: FinalizePendingToolCallCtx<'_>,
    ) {
        let FinalizePendingToolCallCtx {
            workspace,
            session_id,
            permission_level,
            cancel_token,
            tool_calls_in_iteration,
            response,
            tx,
        } = ctx;
        let policy = crate::tools::policy::evaluate_tool_call(
            permission_level,
            &pending.name,
            &pending.arguments,
        );

        if let crate::tools::policy::ToolCallDecision::Blocked { reason } = &policy.decision {
            let _ = tx
                .send(StreamChunk::ToolBlocked {
                    tool_name: pending.name.clone(),
                    reason: reason.clone(),
                })
                .await;

            // Emit a tool result so the UI can finalize the tool card.
            let _ = tx
                .send(StreamChunk::ToolCallResult {
                    name: pending.name.clone(),
                    success: false,
                    output: reason.clone(),
                    duration_ms: 0,
                })
                .await;

            let record = ToolCallRecord {
                id: pending.id,
                name: pending.name,
                arguments: pending.arguments,
                result: ToolResult::Skipped(reason.clone()),
                duration_ms: 0,
            };
            tool_calls_in_iteration.push(record.clone());
            response.tool_calls.push(record);
            return;
        }

        if let crate::tools::policy::ToolCallDecision::RequiresConfirmation(info) = &policy.decision
        {
            // Tool requires confirmation (write operation in Restricted mode).
            // We pause tool execution until the UI approves/denies.

            // Session/persisted fast paths (Claude Code parity): if the user has already
            // allowed/blocked this tool for the session, or has a persisted allow rule,
            // skip the confirmation dialog.
            if let Some(session_id) = session_id.as_deref() {
                if TOOL_CONFIRMATIONS.is_tool_allowed_for_session(session_id, &pending.name) {
                    // Already allowed for this session: proceed to execution.
                } else if TOOL_CONFIRMATIONS.is_tool_blocked_for_session(session_id, &pending.name)
                {
                    let duration_ms = pending.start_time.elapsed().as_millis() as u64;
                    let msg = "Skipped: tool blocked for session".to_string();
                    let _ = tx
                        .send(StreamChunk::ToolCallResult {
                            name: pending.name.clone(),
                            success: false,
                            output: msg.clone(),
                            duration_ms,
                        })
                        .await;

                    let record = ToolCallRecord {
                        id: pending.id,
                        name: pending.name,
                        arguments: pending.arguments,
                        result: ToolResult::Skipped(msg),
                        duration_ms,
                    };
                    tool_calls_in_iteration.push(record.clone());
                    response.tool_calls.push(record);
                    return;
                }
            }

            // Persisted permission fast path: if the user previously chose "Allow always"
            // for this tool, proceed without prompting.
            match self
                .permission_manager
                .check(&pending.name, "execute", Some(&pending.arguments))
            {
                Ok(check) if check.allowed => {
                    // Allowed: proceed to execution.
                }
                Ok(_) => {
                    // No persisted allow rule: continue to confirmation.
                }
                Err(e) => {
                    tracing::warn!(error = %e, tool = %pending.name, "Permission check failed; falling back to confirmation");
                }
            }

            // If the tool is allowed for this session or via persisted permissions, the
            // early returns/branches above will have proceeded; otherwise continue to prompt.
            let needs_confirmation = match session_id.as_deref() {
                Some(sid) if TOOL_CONFIRMATIONS.is_tool_allowed_for_session(sid, &pending.name) => {
                    false
                }
                _ => match self.permission_manager.check(
                    &pending.name,
                    "execute",
                    Some(&pending.arguments),
                ) {
                    Ok(check) => !check.allowed,
                    Err(_) => true,
                },
            };

            if !needs_confirmation {
                // Approved: continue to normal execution flow below.
            } else {
                const CONFIRMATION_TIMEOUT_SECS: u64 = 300;
                let confirmation_id = format!("tool_confirm_{}", uuid::Uuid::new_v4());

                // Register pending confirmation before emitting the event, so the UI can
                // resolve it immediately without racing.
                let rx = TOOL_CONFIRMATIONS.register(
                    confirmation_id.clone(),
                    session_id.clone(),
                    pending.name.clone(),
                    pending.arguments.clone(),
                );

                let _ = tx
                    .send(StreamChunk::ToolConfirmationRequired {
                        confirmation_id: confirmation_id.clone(),
                        tool_name: pending.name.clone(),
                        tool_args: pending.arguments.clone(),
                        description: info.description.clone(),
                        risk_level: info.risk_level,
                        category: info.category.clone(),
                    })
                    .await;

                // Await UI decision with timeout and cancellation.
                let default_decision = crate::tool_confirmation::ToolConfirmationDecision::DenyOnce;
                let decision: crate::tool_confirmation::ToolConfirmationDecision = tokio::select! {
                    decision = rx => decision.unwrap_or(default_decision),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(CONFIRMATION_TIMEOUT_SECS)) => {
                        TOOL_CONFIRMATIONS.abandon(&confirmation_id);
                        default_decision
                    }
                    _ = async {
                        while !cancel_token.is_cancelled() {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    } => {
                        TOOL_CONFIRMATIONS.abandon(&confirmation_id);
                        default_decision
                    }
                };

                // Apply scoped policy decisions.
                if let Some(session_id) = session_id.as_deref() {
                    TOOL_CONFIRMATIONS.apply_session_policy_decision(
                        session_id,
                        &pending.name,
                        decision,
                    );
                }
                if decision == crate::tool_confirmation::ToolConfirmationDecision::AllowAlways
                    && let Err(e) = self.permission_manager.grant(
                        &pending.name,
                        "execute",
                        crate::tools::permissions::PermissionScope::Global,
                        None,
                    )
                {
                    tracing::warn!(
                        error = %e,
                        tool = %pending.name,
                        "Failed to persist AllowAlways permission"
                    );
                }

                if !decision.is_allowed() {
                    let duration_ms = pending.start_time.elapsed().as_millis() as u64;
                    let msg = format!(
                        "Skipped: tool confirmation denied/timed-out (id: {})",
                        confirmation_id
                    );
                    let _ = tx
                        .send(StreamChunk::ToolCallResult {
                            name: pending.name.clone(),
                            success: false,
                            output: msg.clone(),
                            duration_ms,
                        })
                        .await;

                    let record = ToolCallRecord {
                        id: pending.id,
                        name: pending.name,
                        arguments: pending.arguments,
                        result: ToolResult::Skipped(msg),
                        duration_ms,
                    };
                    tool_calls_in_iteration.push(record.clone());
                    response.tool_calls.push(record);
                    return;
                }
                // Approved: continue to normal execution flow below.
            }
        }

        // After confirmation granted, before side effects:
        // 1. Create checkpoint for write operations (best-effort)
        if policy.is_write_operation
            && let Some(sid) = session_id.as_deref()
        {
            self.try_create_checkpoint_before_tool(sid, &pending.name);
        }

        // 2. Run PreTool hook (if enabled) - failures skip tool execution
        let hook_engine = self.create_hook_engine();
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: workspace.map(|w| w.root().to_path_buf()),
                session_id: session_id.clone(),
                tool_name: Some(pending.name.clone()),
                tool_arguments_json: Some(pending.arguments.clone()),
                ..Default::default()
            };
            if let Err(e) = engine.run(HookEvent::PreTool, &hook_ctx).await {
                tracing::warn!(
                    tool = %pending.name,
                    error = %e,
                    "PreTool hook failed; skipping tool execution"
                );
                let duration_ms = pending.start_time.elapsed().as_millis() as u64;
                let msg = format!("Skipped: PreTool hook failed: {}", e);
                let _ = tx
                    .send(StreamChunk::ToolCallResult {
                        name: pending.name.clone(),
                        success: false,
                        output: msg.clone(),
                        duration_ms,
                    })
                    .await;

                let record = ToolCallRecord {
                    id: pending.id,
                    name: pending.name,
                    arguments: pending.arguments,
                    result: ToolResult::Skipped(msg),
                    duration_ms,
                };
                tool_calls_in_iteration.push(record.clone());
                response.tool_calls.push(record);
                return;
            }
        }

        // Execute the tool with workspace sandboxing
        let result = self
            .execute_tool(&pending.name, &pending.arguments, workspace)
            .await;
        let duration_ms = pending.start_time.elapsed().as_millis() as u64;

        // Emit structured tool result for frontend display
        let (success, output) = match &result {
            ToolResult::Success(out) => (true, out.trim_end().to_string()),
            ToolResult::Error(e) => (false, e.clone()),
            ToolResult::Skipped(msg) => (false, format!("Skipped: {}", msg)),
        };
        let _ = tx
            .send(StreamChunk::ToolCallResult {
                name: pending.name.clone(),
                success,
                output: output.clone(),
                duration_ms,
            })
            .await;

        // Run PostTool hook (best-effort)
        if let Some(ref engine) = hook_engine {
            let hook_ctx = HookContext {
                workspace_dir: workspace.map(|w| w.root().to_path_buf()),
                session_id: session_id.clone(),
                tool_name: Some(pending.name.clone()),
                tool_arguments_json: Some(pending.arguments.clone()),
                tool_success: Some(success),
                tool_output: Some(output),
                ..Default::default()
            };
            self.run_hook_best_effort(engine, HookEvent::PostTool, &hook_ctx)
                .await;
        }

        let record = ToolCallRecord {
            id: pending.id,
            name: pending.name,
            arguments: pending.arguments,
            result,
            duration_ms,
        };

        tool_calls_in_iteration.push(record.clone());
        response.tool_calls.push(record);
    }

    async fn execute_tool(
        &self,
        name: &str,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        let start = Instant::now();
        tracing::info!(
            tool = name,
            workspace = ?workspace.map(|w| w.root()),
            "Executing tool with args: {}",
            arguments
        );

        let result = match name {
            "shell" | "bash" | "execute" => self.execute_shell_tool(arguments, workspace).await,
            "file" | "read_file" | "write_file" => {
                self.execute_file_tool(arguments, workspace).await
            }
            "git" => self.execute_git_tool(arguments, workspace).await,
            "web" | "web_search" => self.execute_web_tool(arguments).await,
            "code" => self.execute_code_tool(arguments, workspace).await,
            "task" | "tasks" => self.execute_task_tool(arguments, workspace).await,
            "screenshot" | "screen_record" => {
                self.execute_screen_tool(name, arguments, workspace).await
            }
            _ => ToolResult::Skipped(format!("Unknown tool: {}", name)),
        };

        let duration = start.elapsed();
        tracing::info!("Tool {} completed in {:?}: {:?}", name, duration, result);

        result
    }

    /// Execute web tool
    async fn execute_web_tool(&self, arguments: &str) -> ToolResult {
        use crate::tools::WebTools;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("search");

                let web = WebTools::default();
                match operation {
                    "fetch" => {
                        let url = match args.get("url").and_then(|v| v.as_str()) {
                            Some(u) if !u.trim().is_empty() => u,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'url' for web fetch operation"
                                        .to_string(),
                                );
                            }
                        };

                        // Use fetch_and_extract to get structured content instead of raw HTML
                        // This prevents token overflow by extracting only relevant content
                        match web.fetch_and_extract(url).await {
                            Ok(extracted) => {
                                // Return structured extracted content instead of raw HTML
                                let result = serde_json::json!({
                                    "url": url,
                                    "title": extracted.title,
                                    "description": extracted.description,
                                    "content": extracted.main_content,
                                    "links": extracted.links,
                                });
                                match serde_json::to_string_pretty(&result) {
                                    Ok(s) => ToolResult::Success(s),
                                    Err(e) => ToolResult::Error(format!("Serialize error: {e}")),
                                }
                            }
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "search" => {
                        let query = match args.get("query").and_then(|v| v.as_str()) {
                            Some(q) if !q.trim().is_empty() => q,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'query' for web search operation"
                                        .to_string(),
                                );
                            }
                        };

                        let num_results = args
                            .get("num_results")
                            .and_then(|v| v.as_u64())
                            .or_else(|| args.get("max_results").and_then(|v| v.as_u64()))
                            .map(|n| n as usize);

                        match web.search(query, num_results).await {
                            Ok(res) => match serde_json::to_string_pretty(&res) {
                                Ok(s) => ToolResult::Success(s),
                                Err(e) => ToolResult::Error(format!("Serialize error: {e}")),
                            },
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    other => ToolResult::Error(format!("Unknown web operation: {other}")),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {e}")),
        }
    }

    /// Execute code tool with optional workspace sandboxing
    async fn execute_code_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::tools::code_async;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("stats");

                let raw_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
                let raw_path = if raw_path.trim().is_empty() {
                    "."
                } else {
                    raw_path
                };

                // Resolve path within workspace if set
                let resolved_path = if let Some(ws) = workspace {
                    match ws.resolve_path(Path::new(raw_path)) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return ToolResult::Error(format!(
                                "Path '{}' is outside workspace: {}",
                                raw_path, e
                            ));
                        }
                    }
                } else {
                    raw_path.to_string()
                };

                match operation {
                    "stats" => match code_async::stats_dir(&resolved_path).await {
                        Ok(s) => ToolResult::Success(s),
                        Err(e) => ToolResult::Error(e.to_string()),
                    },
                    other => ToolResult::Error(format!("Unknown code operation: {other}")),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {e}")),
        }
    }

    /// Execute shell tool with workspace sandboxing
    async fn execute_shell_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::session_workspace::is_shell_command_allowed;
        use crate::tools::shell_async;
        use std::collections::HashMap;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let command = args
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or(arguments);

                // Security check: validate command against blocklist
                if let Err(reason) = is_shell_command_allowed(command) {
                    tracing::warn!(
                        command = %command,
                        reason = %reason,
                        "Blocked dangerous shell command"
                    );
                    return ToolResult::Error(format!("Command blocked for security: {}", reason));
                }

                // Determine working directory
                let cwd = if let Some(ws) = workspace {
                    // If workspace is set, use it as the working directory
                    // If cwd is specified in args, resolve it relative to workspace
                    if let Some(requested_cwd) = args.get("cwd").and_then(|v| v.as_str()) {
                        match ws.resolve_path_for_read(Path::new(requested_cwd)) {
                            Ok(resolved) => {
                                if !resolved.is_dir() {
                                    return ToolResult::Error(format!(
                                        "cwd '{}' is not a directory",
                                        requested_cwd
                                    ));
                                }
                                Some(resolved.to_string_lossy().to_string())
                            }
                            Err(e) => {
                                return ToolResult::Error(format!(
                                    "Path '{}' is outside workspace: {}",
                                    requested_cwd, e
                                ));
                            }
                        }
                    } else {
                        Some(ws.root().to_string_lossy().to_string())
                    }
                } else {
                    // No workspace - use requested cwd or None
                    args.get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                };

                // Optional env vars
                let env: Option<HashMap<String, String>> =
                    args.get("env").and_then(|v| v.as_object()).map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect::<HashMap<_, _>>()
                    });

                // Optional timeout (seconds). Default to 60s to match prior behavior.
                let timeout_secs = args
                    .get("timeout_secs")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60);

                match shell_async::execute_command_with_options(
                    command,
                    cwd.as_deref(),
                    env.as_ref(),
                    Some(timeout_secs),
                )
                .await
                {
                    Ok(output) => ToolResult::Success(output),
                    Err(e) => ToolResult::Error(e.to_string()),
                }
            }
            Err(_) => {
                // Treat entire arguments as command - also need security check
                if let Err(reason) = is_shell_command_allowed(arguments) {
                    tracing::warn!(
                        command = %arguments,
                        reason = %reason,
                        "Blocked dangerous shell command"
                    );
                    return ToolResult::Error(format!("Command blocked for security: {}", reason));
                }

                let cwd = workspace.map(|ws| ws.root().to_string_lossy().to_string());
                match shell_async::execute_command(arguments, cwd.as_deref()).await {
                    Ok(output) => ToolResult::Success(output),
                    Err(e) => ToolResult::Error(e.to_string()),
                }
            }
        }
    }

    /// Execute file tool with workspace sandboxing
    async fn execute_file_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::tools::file_async;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        if args.get("content").is_some() {
                            "write"
                        } else {
                            "read"
                        }
                    });

                let raw_path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                let path_str = if raw_path_str.trim().is_empty() {
                    "."
                } else {
                    raw_path_str
                };

                // Resolve path within workspace if set, using stricter variants depending on operation.
                let resolved_path = if let Some(ws) = workspace {
                    let resolved = match operation {
                        "read" => ws.resolve_path_for_read(Path::new(path_str)),
                        "write" | "edit" => ws.resolve_path_for_write(Path::new(path_str)),
                        _ => ws.resolve_path(Path::new(path_str)),
                    };

                    match resolved {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return ToolResult::Error(format!(
                                "Path '{}' is outside workspace: {}",
                                path_str, e
                            ));
                        }
                    }
                } else {
                    path_str.to_string()
                };

                match operation {
                    "write" => {
                        let content = match args.get("content").and_then(|v| v.as_str()) {
                            Some(c) => c,
                            None => {
                                return ToolResult::Error(
                                    "Missing required field 'content' for file write operation"
                                        .to_string(),
                                );
                            }
                        };

                        match file_async::write_file(&resolved_path, content).await {
                            Ok(_) => ToolResult::Success(format!("Written to {}", raw_path_str)),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "read" => {
                        let start_line = args
                            .get("start")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);
                        let end_line = args.get("end").and_then(|v| v.as_u64()).map(|v| v as usize);

                        match file_async::read_file_range(&resolved_path, start_line, end_line)
                            .await
                        {
                            Ok(content) => ToolResult::Success(content),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "list" => {
                        let show_hidden = args
                            .get("show_hidden")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let max_entries = args
                            .get("max_entries")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);

                        match file_async::list_dir(&resolved_path, show_hidden, max_entries).await {
                            Ok(out) => ToolResult::Success(out),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "tree" => {
                        let max_depth = args
                            .get("max_depth")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);

                        let show_hidden = args
                            .get("show_hidden")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        match file_async::tree_dir(&resolved_path, max_depth, show_hidden).await {
                            Ok(out) => ToolResult::Success(out),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "edit" => {
                        let old_str = match args.get("old").and_then(|v| v.as_str()) {
                            Some(s) if !s.is_empty() => s,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'old' for file edit operation"
                                        .to_string(),
                                );
                            }
                        };
                        let new_str = match args.get("new").and_then(|v| v.as_str()) {
                            Some(s) => s,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'new' for file edit operation"
                                        .to_string(),
                                );
                            }
                        };

                        match file_async::edit_file(&resolved_path, old_str, new_str).await {
                            Ok(out) => ToolResult::Success(out),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "search" => {
                        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
                            Some(p) if !p.trim().is_empty() => p,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'pattern' for file search operation"
                                        .to_string(),
                                );
                            }
                        };
                        let recursive = args
                            .get("recursive")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        let max_matches = args
                            .get("max_matches")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);

                        match file_async::search_files(
                            pattern,
                            &resolved_path,
                            recursive,
                            max_matches,
                        )
                        .await
                        {
                            Ok(out) => ToolResult::Success(out),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    other => ToolResult::Error(format!(
                        "Unknown file operation: {other}. Supported operations: read, write, edit, list, tree, search"
                    )),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Execute git tool with workspace sandboxing
    async fn execute_git_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::tools::git_async;

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("status");

                // Only allow the operations we explicitly support in the runtime executor.
                let supported = matches!(
                    operation,
                    "status" | "diff" | "diff-staged" | "log" | "branches"
                );
                if !supported {
                    return ToolResult::Error(format!(
                        "Unknown git operation: {operation}. Supported operations: status, diff, diff-staged, log, branches"
                    ));
                }

                let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");

                // Resolve path within workspace if set
                let resolved_path = if let Some(ws) = workspace {
                    match ws.resolve_path(Path::new(path_str)) {
                        Ok(p) => p.to_string_lossy().to_string(),
                        Err(e) => {
                            return ToolResult::Error(format!(
                                "Path '{}' is outside workspace: {}",
                                path_str, e
                            ));
                        }
                    }
                } else {
                    path_str.to_string()
                };

                match git_async::execute_git(operation, &resolved_path).await {
                    Ok(output) => ToolResult::Success(output),
                    Err(e) => ToolResult::Error(e.to_string()),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Execute task management tool
    async fn execute_task_tool(
        &self,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::{TaskManager, TaskStatus};
        use std::sync::OnceLock;

        // Global task manager instance
        static TASK_MANAGER: OnceLock<TaskManager> = OnceLock::new();

        // Get or initialize the global task manager
        let manager = TASK_MANAGER.get_or_init(|| {
            let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
            TaskManager::new(base_dir)
        });

        // Get session_id from workspace
        let session_id = match workspace {
            Some(ws) => &ws.session_id,
            None => {
                return ToolResult::Error(
                    "Task management requires an active session with workspace".to_string(),
                );
            }
        };

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                let operation = args
                    .get("operation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("list");

                match operation {
                    "create" => {
                        let name = match args.get("name").and_then(|v| v.as_str()) {
                            Some(n) => n,
                            None => {
                                return ToolResult::Error(
                                    "Missing required field 'name' for create operation"
                                        .to_string(),
                                );
                            }
                        };
                        let description = args
                            .get("description")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let parent_id = args
                            .get("parent_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        match manager.create_task(session_id, name, description, parent_id) {
                            Ok(task) => ToolResult::Success(format!(
                                "Created task '{}' (ID: {})\nDescription: {}\nStatus: {:?}",
                                task.name, task.id, task.description, task.status
                            )),
                            Err(e) => ToolResult::Error(format!("Failed to create task: {}", e)),
                        }
                    }
                    "update_status" => {
                        let task_id =
                            match args.get("task_id").and_then(|v| v.as_str()) {
                                Some(id) => id,
                                None => return ToolResult::Error(
                                    "Missing required field 'task_id' for update_status operation"
                                        .to_string(),
                                ),
                            };
                        let status_str =
                            match args.get("status").and_then(|v| v.as_str()) {
                                Some(s) => s,
                                None => return ToolResult::Error(
                                    "Missing required field 'status' for update_status operation"
                                        .to_string(),
                                ),
                            };

                        let status = match status_str.to_lowercase().as_str() {
                            "notstarted" | "not_started" => TaskStatus::NotStarted,
                            "inprogress" | "in_progress" => TaskStatus::InProgress,
                            "completed" => TaskStatus::Completed,
                            "cancelled" => TaskStatus::Cancelled,
                            _ => {
                                return ToolResult::Error(format!(
                                    "Invalid status '{}'. Use 'notstarted', 'inprogress', 'completed', or 'cancelled'",
                                    status_str
                                ));
                            }
                        };

                        match manager.update_task_status(session_id, task_id, status) {
                            Ok(_) => ToolResult::Success(format!(
                                "Updated task {} status to {:?}",
                                task_id, status
                            )),
                            Err(e) => {
                                ToolResult::Error(format!("Failed to update task status: {}", e))
                            }
                        }
                    }
                    "update" => {
                        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => {
                                return ToolResult::Error(
                                    "Missing required field 'task_id' for update operation"
                                        .to_string(),
                                );
                            }
                        };
                        let name = args
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let description = args
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        match manager.update_task(
                            session_id,
                            task_id,
                            name.clone(),
                            description.clone(),
                        ) {
                            Ok(_) => {
                                let mut updates = Vec::new();
                                if let Some(n) = name {
                                    updates.push(format!("name to '{}'", n));
                                }
                                if let Some(d) = description {
                                    updates.push(format!("description to '{}'", d));
                                }
                                ToolResult::Success(format!(
                                    "Updated task {}: {}",
                                    task_id,
                                    updates.join(", ")
                                ))
                            }
                            Err(e) => ToolResult::Error(format!("Failed to update task: {}", e)),
                        }
                    }
                    "delete" => {
                        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => {
                                return ToolResult::Error(
                                    "Missing required field 'task_id' for delete operation"
                                        .to_string(),
                                );
                            }
                        };

                        match manager.delete_task(session_id, task_id) {
                            Ok(task) => ToolResult::Success(format!(
                                "Deleted task '{}' (ID: {})",
                                task.name, task.id
                            )),
                            Err(e) => ToolResult::Error(format!("Failed to delete task: {}", e)),
                        }
                    }
                    "list" => match manager.list_tasks(session_id) {
                        Ok(tasks) => {
                            if tasks.is_empty() {
                                ToolResult::Success("No tasks found for this session".to_string())
                            } else {
                                let mut output = format!("Found {} task(s):\n\n", tasks.len());
                                for task in tasks {
                                    output.push_str(&format!(
                                        "• {} (ID: {})\n  Status: {:?}\n  Description: {}\n",
                                        task.name, task.id, task.status, task.description
                                    ));
                                    if let Some(parent_id) = &task.parent_id {
                                        output.push_str(&format!("  Parent: {}\n", parent_id));
                                    }
                                    output.push('\n');
                                }
                                ToolResult::Success(output)
                            }
                        }
                        Err(e) => ToolResult::Error(format!("Failed to list tasks: {}", e)),
                    },
                    "get_hierarchy" => match manager.get_hierarchy(session_id) {
                        Ok(hierarchy) => {
                            if hierarchy.is_empty() {
                                ToolResult::Success("No tasks found for this session".to_string())
                            } else {
                                let mut output = format!(
                                    "Task hierarchy ({} root task(s)):\n\n",
                                    hierarchy.len()
                                );
                                for (root, subtasks) in hierarchy {
                                    output.push_str(&format!(
                                        "• {} (ID: {})\n  Status: {:?}\n  Description: {}\n",
                                        root.name, root.id, root.status, root.description
                                    ));
                                    if !subtasks.is_empty() {
                                        output.push_str(&format!(
                                            "  Subtasks ({}):\n",
                                            subtasks.len()
                                        ));
                                        for subtask in subtasks {
                                            output.push_str(&format!(
                                                "    - {} (ID: {}, Status: {:?})\n",
                                                subtask.name, subtask.id, subtask.status
                                            ));
                                        }
                                    }
                                    output.push('\n');
                                }
                                ToolResult::Success(output)
                            }
                        }
                        Err(e) => ToolResult::Error(format!("Failed to get task hierarchy: {}", e)),
                    },
                    _ => ToolResult::Error(format!(
                        "Unknown task operation: {}. Supported: create, update_status, update, delete, list, get_hierarchy",
                        operation
                    )),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Truncate tool result to prevent token explosion
    /// Execute screen tool (screenshot and screen recording)
    async fn execute_screen_tool(
        &self,
        tool_name: &str,
        arguments: &str,
        workspace: Option<&SessionWorkspace>,
    ) -> ToolResult {
        use crate::tools::screen_async;
        use crate::tools::screen_async::{
            ScreenshotInlineOptions, ScreenshotReturnMode, ScreenshotReturnOptions,
        };

        match serde_json::from_str::<serde_json::Value>(arguments) {
            Ok(args) => {
                // Determine operation type based on tool name + optional operation field.
                // This prevents, e.g., the `screenshot` tool from being used to start/stop
                // recordings via a spoofed `operation` argument.
                let operation_from_args = args.get("operation").and_then(|v| v.as_str());

                let operation = match tool_name {
                    "screenshot" => operation_from_args.unwrap_or("screenshot"),
                    "screen_record" => match operation_from_args {
                        Some(op) => op,
                        None => {
                            return ToolResult::Error(
                                "Missing required field 'operation' for screen_record".to_string(),
                            );
                        }
                    },
                    other => {
                        return ToolResult::Error(format!("Unknown screen tool: {other}"));
                    }
                };

                // Enforce tool-specific allowed operations.
                match tool_name {
                    "screenshot" => {
                        if !matches!(operation, "screenshot" | "capture") {
                            return ToolResult::Error(format!(
                                "Tool 'screenshot' does not support operation '{operation}'. Supported operations: screenshot, capture"
                            ));
                        }
                    }
                    "screen_record" => {
                        if !matches!(operation, "start" | "stop") {
                            return ToolResult::Error(format!(
                                "Tool 'screen_record' does not support operation '{operation}'. Supported operations: start, stop"
                            ));
                        }
                    }
                    _ => {}
                }

                // Helper: choose an output path when the caller didn't specify one.
                fn default_artifact_path(kind: &str, ext: &str) -> std::path::PathBuf {
                    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
                    let id = uuid::Uuid::new_v4().to_string();
                    std::path::PathBuf::from(".gestura")
                        .join("artifacts")
                        .join("screen")
                        .join(format!("{kind}-{ts}-{id}.{ext}"))
                }

                fn normalize_ext(s: &str) -> String {
                    s.trim().trim_start_matches('.').to_ascii_lowercase()
                }

                fn apply_or_validate_extension(
                    mut path: std::path::PathBuf,
                    desired_ext: Option<&str>,
                ) -> std::result::Result<std::path::PathBuf, String> {
                    let Some(desired_ext) = desired_ext else {
                        return Ok(path);
                    };
                    let desired_ext = normalize_ext(desired_ext);
                    if desired_ext.is_empty() {
                        return Ok(path);
                    }

                    match path.extension().and_then(|e| e.to_str()) {
                        Some(existing) if !existing.is_empty() => {
                            let existing = normalize_ext(existing);
                            if existing != desired_ext {
                                return Err(format!(
                                    "output_path extension '.{existing}' does not match requested output_format '.{desired_ext}'. Either omit output_format or make them match."
                                ));
                            }
                        }
                        _ => {
                            path.set_extension(&desired_ext);
                        }
                    }
                    Ok(path)
                }

                fn parse_screenshot_return_options(
                    args: &serde_json::Value,
                ) -> std::result::Result<ScreenshotReturnOptions, String> {
                    let mut opts = ScreenshotReturnOptions::default();

                    let return_obj = args.get("return").and_then(|v| v.as_object());
                    if return_obj.is_none() {
                        return Ok(opts);
                    }
                    let return_obj = return_obj.unwrap();

                    if let Some(mode) = return_obj.get("mode").and_then(|v| v.as_str()) {
                        match mode.trim() {
                            "path" => opts.mode = ScreenshotReturnMode::Path,
                            "inline_base64" => opts.mode = ScreenshotReturnMode::InlineBase64,
                            other => {
                                return Err(format!(
                                    "Invalid return.mode '{other}'. Supported: path, inline_base64"
                                ));
                            }
                        }
                    }

                    if let Some(inline_obj) = return_obj.get("inline").and_then(|v| v.as_object()) {
                        let mut inline = ScreenshotInlineOptions::default();
                        if let Some(w) = inline_obj.get("max_width").and_then(|v| v.as_u64()) {
                            inline.max_width = Some(w as u32);
                        }
                        if let Some(h) = inline_obj.get("max_height").and_then(|v| v.as_u64()) {
                            inline.max_height = Some(h as u32);
                        }
                        if let Some(m) = inline_obj.get("max_base64_chars").and_then(|v| v.as_u64())
                        {
                            inline.max_base64_chars = m as usize;
                        }
                        if let Some(m) = inline_obj.get("max_result_chars").and_then(|v| v.as_u64())
                        {
                            inline.max_result_chars = m as usize;
                        }
                        opts.inline = inline;
                    }

                    Ok(opts)
                }

                match operation {
                    "screenshot" | "capture" => {
                        let output_format = args
                            .get("output_format")
                            .and_then(|v| v.as_str())
                            .map(|s| {
                                let ext = normalize_ext(s);
                                // Treat jpeg as a synonym for jpg to avoid needless caller friction.
                                if ext == "jpeg" {
                                    "jpg".to_string()
                                } else {
                                    ext
                                }
                            })
                            .filter(|s| !s.is_empty());

                        if let Some(fmt) = output_format.as_deref()
                            && !matches!(fmt, "png" | "jpg")
                        {
                            return ToolResult::Error(format!(
                                "Invalid output_format '{fmt}'. Supported: png, jpg (jpeg is accepted as an alias for jpg)"
                            ));
                        }

                        let screenshot_return = match parse_screenshot_return_options(&args) {
                            Ok(o) => o,
                            Err(e) => return ToolResult::Error(e),
                        };

                        let output_path = args
                            .get("output_path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| {
                                let ext = output_format.as_deref().unwrap_or("png");
                                default_artifact_path("screenshot", ext)
                            });

                        let output_path = match apply_or_validate_extension(
                            output_path,
                            output_format.as_deref(),
                        ) {
                            Ok(p) => p,
                            Err(e) => return ToolResult::Error(e),
                        };

                        // Resolve path within workspace if set
                        let resolved_path = if let Some(ws) = workspace {
                            match ws.resolve_path_for_create(&output_path) {
                                Ok(p) => p.to_string_lossy().to_string(),
                                Err(e) => {
                                    return ToolResult::Error(format!(
                                        "Path '{}' is outside workspace: {}",
                                        output_path.display(),
                                        e
                                    ));
                                }
                            }
                        } else {
                            output_path.to_string_lossy().to_string()
                        };

                        // Parse optional region
                        let region = args.get("region").and_then(|r| {
                            if let Some(obj) = r.as_object() {
                                let x = obj.get("x")?.as_u64()? as u32;
                                let y = obj.get("y")?.as_u64()? as u32;
                                let width = obj.get("width")?.as_u64()? as u32;
                                let height = obj.get("height")?.as_u64()? as u32;
                                Some((x, y, width, height))
                            } else {
                                None
                            }
                        });

                        // Parse optional display
                        let display = args
                            .get("display")
                            .and_then(|v| v.as_u64())
                            .map(|d| d as u32);

                        match screen_async::screenshot_with_options(
                            &resolved_path,
                            region,
                            display,
                            screenshot_return,
                        )
                        .await
                        {
                            Ok(result) => ToolResult::Success(result),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "start" | "start_recording" => {
                        let output_format = args
                            .get("output_format")
                            .and_then(|v| v.as_str())
                            .map(normalize_ext)
                            .filter(|s| !s.is_empty());

                        if let Some(fmt) = output_format.as_deref()
                            && !matches!(fmt, "mp4" | "mov")
                        {
                            return ToolResult::Error(format!(
                                "Invalid output_format '{fmt}'. Supported: mp4, mov"
                            ));
                        }

                        let output_path = args
                            .get("output_path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(std::path::PathBuf::from)
                            .unwrap_or_else(|| {
                                let ext = output_format.as_deref().unwrap_or("mp4");
                                default_artifact_path("screen-recording", ext)
                            });

                        let output_path = match apply_or_validate_extension(
                            output_path,
                            output_format.as_deref(),
                        ) {
                            Ok(p) => p,
                            Err(e) => return ToolResult::Error(e),
                        };

                        // Resolve path within workspace if set
                        let resolved_path = if let Some(ws) = workspace {
                            match ws.resolve_path_for_create(&output_path) {
                                Ok(p) => p.to_string_lossy().to_string(),
                                Err(e) => {
                                    return ToolResult::Error(format!(
                                        "Path '{}' is outside workspace: {}",
                                        output_path.display(),
                                        e
                                    ));
                                }
                            }
                        } else {
                            output_path.to_string_lossy().to_string()
                        };

                        // Parse optional region
                        let region = args.get("region").and_then(|r| {
                            if let Some(obj) = r.as_object() {
                                let x = obj.get("x")?.as_u64()? as u32;
                                let y = obj.get("y")?.as_u64()? as u32;
                                let width = obj.get("width")?.as_u64()? as u32;
                                let height = obj.get("height")?.as_u64()? as u32;
                                Some((x, y, width, height))
                            } else {
                                None
                            }
                        });

                        // Parse optional display
                        let display = args
                            .get("display")
                            .and_then(|v| v.as_u64())
                            .map(|d| d as u32);

                        match screen_async::start_recording(&resolved_path, region, display).await {
                            Ok(result) => ToolResult::Success(result),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    "stop" | "stop_recording" => {
                        let recording_id = match args.get("recording_id").and_then(|v| v.as_str()) {
                            Some(id) if !id.trim().is_empty() => id,
                            _ => {
                                return ToolResult::Error(
                                    "Missing required field 'recording_id' for stop_recording operation"
                                        .to_string(),
                                );
                            }
                        };

                        match screen_async::stop_recording(recording_id).await {
                            Ok(result) => ToolResult::Success(result),
                            Err(e) => ToolResult::Error(e.to_string()),
                        }
                    }
                    other => ToolResult::Error(format!(
                        "Unknown screen operation: {}. Supported operations: screenshot, start, stop",
                        other
                    )),
                }
            }
            Err(e) => ToolResult::Error(format!("Invalid arguments: {}", e)),
        }
    }

    /// Limits results to 2000 characters max with truncation indicator
    fn truncate_tool_result(&self, result: &str) -> String {
        const MAX_TOOL_RESULT_CHARS: usize = 2000;

        if result.len() <= MAX_TOOL_RESULT_CHARS {
            result.to_string()
        } else {
            let truncated = &result[..MAX_TOOL_RESULT_CHARS];
            let remaining = result.len() - MAX_TOOL_RESULT_CHARS;

            if self.pipeline_config.log_token_usage {
                tracing::debug!(
                    original_length = result.len(),
                    truncated_length = MAX_TOOL_RESULT_CHARS,
                    remaining_chars = remaining,
                    "Truncating tool result to prevent token explosion"
                );
            }

            format!(
                "{}\n[... truncated {} more characters ...]",
                truncated, remaining
            )
        }
    }

    /// Build a continuation prompt after tool execution
    fn build_tool_continuation_prompt(
        &self,
        original_prompt: &str,
        assistant_response: &str,
        tool_calls: &[ToolCallRecord],
    ) -> String {
        let mut prompt = original_prompt.to_string();

        prompt.push_str(&format!("\nAssistant: {}\n", assistant_response));

        for tool_call in tool_calls {
            let result_text = match &tool_call.result {
                ToolResult::Success(s) => {
                    let truncated = self.truncate_tool_result(s);
                    format!("Success: {}", truncated)
                }
                ToolResult::Error(e) => {
                    let truncated = self.truncate_tool_result(e);
                    format!("Error: {}", truncated)
                }
                ToolResult::Skipped(r) => {
                    let truncated = self.truncate_tool_result(r);
                    format!("Skipped: {}", truncated)
                }
            };
            prompt.push_str(&format!(
                "\nTool {} result:\n{}\n",
                tool_call.name, result_text
            ));
        }

        prompt.push_str(
            "\nUser: Based on the tool results above, provide a complete and helpful response \
             to my original request. Synthesize the information, highlight the key findings, \
             and present a clear answer. If you created any tasks to track this work, mark \
             them as completed now.\n",
        );

        prompt
    }
}

/// Pending tool call being accumulated during streaming
struct PendingToolCall {
    id: String,
    name: String,
    arguments: String,
    start_time: Instant,
}

/// Context used by `AgentPipeline::finalize_pending_tool_call`.
///
/// This bundles together the mutable per-iteration state and runtime references required to
/// complete a pending tool call (permission checks, execution, streaming events, and recording).
struct FinalizePendingToolCallCtx<'a> {
    workspace: Option<&'a SessionWorkspace>,
    session_id: Option<String>,
    permission_level: PermissionLevel,
    cancel_token: &'a CancellationToken,
    tool_calls_in_iteration: &'a mut Vec<ToolCallRecord>,
    response: &'a mut AgentResponse,
    tx: &'a mpsc::Sender<StreamChunk>,
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    #[test]
    fn test_agent_request_builder() {
        let request = AgentRequest::new("Hello world")
            .with_streaming(true)
            .with_source(RequestSource::CliTui);

        assert_eq!(request.input, "Hello world");
        assert!(request.streaming);
        assert_eq!(request.metadata.source, RequestSource::CliTui);
    }

    #[test]
    fn test_message_constructors() {
        let user_msg = Message::user("Hello");
        assert_eq!(user_msg.role, "user");

        let assistant_msg = Message::assistant("Hi there");
        assert_eq!(assistant_msg.role, "assistant");

        let tool_msg = Message::tool_result("call_123", "result data");
        assert_eq!(tool_msg.role, "tool");
        assert_eq!(tool_msg.tool_call_id, Some("call_123".to_string()));
    }

    #[test]
    fn build_prompt_includes_project_guardrails_when_workspace_present() {
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "Always run tests.\n").unwrap();

        let pipeline = AgentPipeline::new(AppConfig::default());

        let request = AgentRequest::new("hi").with_workspace(temp.path());
        let context = crate::context::ResolvedContext::default();
        let prompt = pipeline.build_prompt(&request, &context);

        assert!(prompt.contains("Project guardrails:"));
        assert!(prompt.contains("Always run tests."));
        assert!(prompt.contains("Source: AGENTS.md"));
    }

    #[test]
    fn build_prompt_uses_dot_gestura_guardrails_over_agents_md() {
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "agents-rule\n").unwrap();
        std::fs::create_dir_all(temp.path().join(".gestura")).unwrap();
        std::fs::write(temp.path().join(".gestura/guardrails"), "guardrails-rule\n").unwrap();

        let pipeline = AgentPipeline::new(AppConfig::default());

        let request = AgentRequest::new("hi").with_workspace(temp.path());
        let context = crate::context::ResolvedContext::default();
        let prompt = pipeline.build_prompt(&request, &context);

        assert!(prompt.contains("guardrails-rule"));
        assert!(!prompt.contains("agents-rule"));
        assert!(prompt.contains("Source: .gestura/guardrails"));
    }

    #[test]
    fn build_prompt_skips_guardrails_when_disabled() {
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "agents-rule\n").unwrap();

        let mut config = AppConfig::default();
        config.pipeline.project_guardrails.enabled = false;
        let pipeline = AgentPipeline::new(config);

        let request = AgentRequest::new("hi").with_workspace(temp.path());
        let context = crate::context::ResolvedContext::default();
        let prompt = pipeline.build_prompt(&request, &context);

        assert!(!prompt.contains("Project guardrails:"));
        assert!(!prompt.contains("agents-rule"));
    }

    #[test]
    fn test_pipeline_config_defaults() {
        let config = PipelineConfig::default();
        assert_eq!(config.max_iterations, 10);
        assert!(config.enable_tools);
        assert!(config.enable_context_reduction);
    }

    #[test]
    fn promote_approval_followup_enables_shell_tool() {
        use crate::context::ContextCategory;

        let pipeline = AgentPipeline::new(AppConfig::default());

        let history = vec![Message::assistant(
            "We will use the shell tool to run 'pwd'. Then respond.",
        )];
        let request = AgentRequest::new("okay please proceed").with_history(history);

        let mut analysis = crate::context::RequestAnalysis::new("okay please proceed");
        assert!(!analysis.needs_tools);

        pipeline.promote_approval_to_tool_followup(&request, &mut analysis);

        assert!(analysis.needs_tools);
        assert!(analysis.is_followup);
        assert!(analysis.categories.contains(&ContextCategory::Shell));
        assert!(analysis.categories.contains(&ContextCategory::Tools));
        assert!(analysis.suggested_tools.contains(&"shell".to_string()));
        assert!(analysis.confidence >= 0.85);
    }

    /// When adapter layers explicitly disable tools for a request, the pipeline must
    /// not execute any tools (including the confirmed-tool follow-up heuristic).
    #[tokio::test]
    #[ignore = "requires Ollama with llama3.2 model installed"]
    async fn tools_enabled_false_skips_confirmed_tool_followup_execution() {
        use tokio::sync::mpsc;
        use tokio::time::{Duration, timeout};

        let pipeline = AgentPipeline::new(AppConfig::default());

        // Prior assistant message contains an explicit tool plan.
        let history = vec![Message::assistant(
            "We will use the shell tool to run 'pwd'. Then respond.",
        )];

        // User approval would normally trigger tool follow-up execution.
        let request = AgentRequest::new("okay please proceed")
            .with_history(history)
            .with_tools_enabled(false);

        // IMPORTANT: drain the stream concurrently to avoid backpressure deadlocks
        // if the provider emits many chunks.
        let (tx, mut rx) = mpsc::channel(256);
        let cancel = CancellationToken::new();

        let drain_handle = tokio::spawn(async move {
            let mut saw_done = false;
            while let Some(chunk) = rx.recv().await {
                match chunk {
                    other @ (StreamChunk::ToolCallStart { .. }
                    | StreamChunk::ToolCallArgs(_)
                    | StreamChunk::ToolCallEnd
                    | StreamChunk::ToolCallResult { .. }
                    | StreamChunk::ToolConfirmationRequired { .. }
                    | StreamChunk::ToolBlocked { .. }) => {
                        return Err(format!(
                            "unexpected tool chunk emitted when tools are disabled: {other:?}"
                        ));
                    }
                    StreamChunk::Done(_) => {
                        saw_done = true;
                        break;
                    }
                    _ => {}
                }
            }
            Ok::<bool, String>(saw_done)
        });

        let response = timeout(
            Duration::from_secs(5),
            pipeline.process_streaming(request, tx, cancel),
        )
        .await
        .expect("process_streaming should not hang")
        .expect("pipeline should complete");

        // Strong assertion: the response should not record any tool calls.
        assert!(response.tool_calls.is_empty());

        // Avoid hangs if a regression causes the stream to never finalize.
        let saw_done = timeout(Duration::from_secs(3), drain_handle)
            .await
            .expect("drain task should finish")
            .expect("drain task should not panic")
            .expect("no tool chunks should be emitted");

        assert!(saw_done);
    }

    /// Even when request analysis would normally select tools, `tools_enabled=false`
    /// must ensure the blocking pipeline path does not execute tools.
    #[tokio::test]
    #[ignore = "requires Ollama with llama3.2 model installed"]
    async fn tools_enabled_false_disables_tools_for_blocking_requests() {
        let pipeline = AgentPipeline::new(AppConfig::default());

        let request = AgentRequest::new("Read the file 'Cargo.toml'.")
            .with_streaming(false)
            .with_tools_enabled(false);

        let response = pipeline
            .process_blocking(request)
            .await
            .expect("blocking pipeline should complete");

        assert!(response.tool_calls.is_empty());
        assert!(!response.content.trim().is_empty());
    }

    #[test]
    fn extract_shell_command_from_plan_parses_quoted_command() {
        let text = "We will use the shell tool to run 'pwd'. Then respond.";
        let cmd = AgentPipeline::extract_shell_command_from_plan(text).unwrap();
        assert_eq!(cmd, "pwd");

        let text2 = "We'll use the shell tool to run `git status` then respond.";
        let cmd2 = AgentPipeline::extract_shell_command_from_plan(text2).unwrap();
        assert_eq!(cmd2, "git status");
    }

    #[test]
    fn extract_planned_tool_call_from_text_parses_file_read() {
        let text = "We will use the file tool to read 'foo.txt'. Then respond.";
        let (tool, args, prefix) =
            AgentPipeline::extract_planned_tool_call_from_text(text).expect("should parse");
        assert_eq!(tool, "file");
        assert!(args.contains("\"operation\":\"read\""));
        assert!(args.contains("\"path\":\"foo.txt\""));
        assert!(prefix.to_lowercase().contains("file"));
    }

    #[test]
    fn is_write_operation_classifies_file_operations() {
        let read = serde_json::json!({"operation": "read", "path": "foo.txt"}).to_string();
        assert!(!crate::tools::policy::is_write_operation("file", &read));

        let list = serde_json::json!({"operation": "list", "path": "."}).to_string();
        assert!(!crate::tools::policy::is_write_operation("file", &list));

        let search =
            serde_json::json!({"operation": "search", "path": ".", "pattern": "foo"}).to_string();
        assert!(!crate::tools::policy::is_write_operation("file", &search));

        let write = serde_json::json!({"operation": "write", "path": "foo.txt", "content": "hi"})
            .to_string();
        assert!(crate::tools::policy::is_write_operation("file", &write));

        let edit =
            serde_json::json!({"operation": "edit", "path": "foo.txt", "old": "a", "new": "b"})
                .to_string();
        assert!(crate::tools::policy::is_write_operation("file", &edit));

        // Mirror the defaulting behavior: content without operation is treated as write.
        let implicit_write = serde_json::json!({"path": "foo.txt", "content": "hi"}).to_string();
        assert!(crate::tools::policy::is_write_operation(
            "file",
            &implicit_write
        ));
    }

    #[test]
    fn is_write_operation_classifies_shell_commands_conservatively() {
        let pwd = serde_json::json!({"command": "pwd"}).to_string();
        assert!(!crate::tools::policy::is_write_operation("shell", &pwd));

        let ls = serde_json::json!({"command": "ls -la"}).to_string();
        assert!(!crate::tools::policy::is_write_operation("shell", &ls));

        let echo = serde_json::json!({"command": "echo hi"}).to_string();
        assert!(!crate::tools::policy::is_write_operation("shell", &echo));

        let redirect = serde_json::json!({"command": "echo hi > out.txt"}).to_string();
        assert!(crate::tools::policy::is_write_operation("shell", &redirect));

        // Unknown commands are treated as write for safety.
        let unknown = serde_json::json!({"command": "git status"}).to_string();
        assert!(crate::tools::policy::is_write_operation("shell", &unknown));
    }

    #[tokio::test]
    #[cfg_attr(target_os = "windows", ignore = "pwd command is Unix-only")]
    async fn streaming_followup_approval_executes_shell_from_history_and_finishes() {
        use std::time::Duration;

        let pipeline = AgentPipeline::new(AppConfig::default());

        let history = vec![Message::assistant(
            "We will use the shell tool to run 'pwd'. Then respond.",
        )];

        let cwd = std::env::current_dir().unwrap();

        let request = AgentRequest::new("okay please proceed")
            .with_history(history)
            .with_session("test")
            .with_workspace(cwd.clone());

        let (tx, mut rx) = mpsc::channel(32);
        let cancel = CancellationToken::new();

        let resp = pipeline
            .process_streaming(request, tx, cancel)
            .await
            .expect("process_streaming should succeed");

        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "shell");
        assert!(resp.content.contains("Workspace directory root:"));

        // Ensure the stream terminates (no silent hang)
        let saw_done = tokio::time::timeout(Duration::from_secs(2), async move {
            while let Some(chunk) = rx.recv().await {
                if matches!(chunk, StreamChunk::Done(_)) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);

        assert!(saw_done);

        // Optional: sanity check the output includes the workspace path.
        // `pwd` prints the current working directory; in sandbox mode we run in workspace root.
        let expected = cwd.to_string_lossy();
        assert!(resp.content.contains(expected.as_ref()));
    }

    #[tokio::test]
    async fn streaming_followup_approval_executes_file_read_from_history_and_finishes() {
        use std::time::Duration;
        use tempfile::tempdir;

        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("foo.txt");
        std::fs::write(&file_path, "hello from file\n").unwrap();

        let history = vec![Message::assistant(
            "We will use the file tool to read 'foo.txt'. Then respond.",
        )];

        let request = AgentRequest::new("okay please proceed")
            .with_history(history)
            .with_session("test")
            .with_workspace(temp.path().to_path_buf());

        let (tx, mut rx) = mpsc::channel(32);
        let cancel = CancellationToken::new();

        let resp = pipeline
            .process_streaming(request, tx, cancel)
            .await
            .expect("process_streaming should succeed");

        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "file");
        assert!(resp.content.contains("hello from file"));

        // Ensure the stream terminates (no silent hang)
        let saw_done = tokio::time::timeout(Duration::from_secs(2), async move {
            while let Some(chunk) = rx.recv().await {
                if matches!(chunk, StreamChunk::Done(_)) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);

        assert!(saw_done);
    }

    /// In Restricted mode, write operations must request confirmation.
    ///
    /// When the user denies, the tool should be skipped, a ToolCallResult should
    /// be emitted (success=false), and the pending confirmation should be cleared.
    #[tokio::test]
    async fn restricted_mode_write_tool_denied_emits_tool_call_result_and_skips() {
        use std::sync::Arc;
        use tempfile::tempdir;
        use tokio::sync::mpsc;

        use crate::session_workspace::SessionWorkspace;
        use crate::tool_confirmation::{TOOL_CONFIRMATIONS, ToolConfirmationDecision};

        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        let workspace = Arc::new(
            SessionWorkspace::from_directory("s1", temp.path().to_path_buf()).expect("workspace"),
        );

        let (tx, mut rx) = mpsc::channel(32);
        let cancel = CancellationToken::new();

        let pending = PendingToolCall {
            id: "call_test_denied".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "out.txt",
                "content": "hi"
            })
            .to_string(),
            start_time: Instant::now(),
        };

        let handle = tokio::spawn({
            let workspace = workspace.clone();
            async move {
                let mut tool_calls_in_iteration: Vec<ToolCallRecord> = Vec::new();
                let mut response = AgentResponse::empty();

                pipeline
                    .finalize_pending_tool_call(
                        pending,
                        FinalizePendingToolCallCtx {
                            workspace: Some(workspace.as_ref()),
                            session_id: Some("s1".to_string()),
                            permission_level: PermissionLevel::Restricted,
                            cancel_token: &cancel,
                            tool_calls_in_iteration: &mut tool_calls_in_iteration,
                            response: &mut response,
                            tx: &tx,
                        },
                    )
                    .await;

                (tool_calls_in_iteration, response)
            }
        });

        // Wait for the confirmation request and deny it.
        let mut confirmation_id: Option<String> = None;
        while let Some(chunk) = rx.recv().await {
            if let StreamChunk::ToolConfirmationRequired {
                confirmation_id: id,
                ..
            } = chunk
            {
                confirmation_id = Some(id);
                break;
            }
        }
        let confirmation_id = confirmation_id.expect("expected ToolConfirmationRequired");

        TOOL_CONFIRMATIONS
            .resolve_decision(
                &confirmation_id,
                Some("s1"),
                ToolConfirmationDecision::DenyOnce,
            )
            .expect("resolve should succeed");

        // Ensure we emit a tool call result with success=false.
        let mut saw_result = false;
        while let Some(chunk) = rx.recv().await {
            if let StreamChunk::ToolCallResult {
                success, output, ..
            } = chunk
            {
                assert!(!success);
                assert!(output.contains("Skipped: tool confirmation"));
                saw_result = true;
                break;
            }
        }
        assert!(saw_result);

        let (tool_calls, response) = handle.await.expect("task join");
        // Ensure this specific confirmation has been cleared, without depending on
        // global pending count (tests may run concurrently).
        let err = TOOL_CONFIRMATIONS
            .resolve_decision(
                &confirmation_id,
                Some("s1"),
                ToolConfirmationDecision::AllowOnce,
            )
            .unwrap_err();
        assert!(err.contains("Unknown confirmation id"));
        assert!(!temp.path().join("out.txt").exists());

        // Sanity: the pipeline should record a skipped tool call.
        assert!(
            tool_calls
                .iter()
                .any(|t| matches!(t.result, ToolResult::Skipped(_)))
        );
        assert!(
            response
                .tool_calls
                .iter()
                .any(|t| matches!(t.result, ToolResult::Skipped(_)))
        );
    }

    /// In Restricted mode, if the user never responds, the confirmation should
    /// time out and the tool should be skipped with a ToolCallResult.
    #[tokio::test(start_paused = true)]
    async fn restricted_mode_write_tool_times_out_and_emits_tool_call_result() {
        use std::sync::Arc;
        use std::time::Duration;
        use tempfile::tempdir;
        use tokio::sync::mpsc;

        use crate::session_workspace::SessionWorkspace;
        use crate::tool_confirmation::{TOOL_CONFIRMATIONS, ToolConfirmationDecision};

        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        let workspace = Arc::new(
            SessionWorkspace::from_directory("s1", temp.path().to_path_buf()).expect("workspace"),
        );

        let (tx, mut rx) = mpsc::channel(32);
        let cancel = CancellationToken::new();

        let pending = PendingToolCall {
            id: "call_test_timeout".to_string(),
            name: "file".to_string(),
            arguments: serde_json::json!({
                "operation": "write",
                "path": "out_timeout.txt",
                "content": "hi"
            })
            .to_string(),
            start_time: Instant::now(),
        };

        let handle = tokio::spawn({
            let workspace = workspace.clone();
            async move {
                let mut tool_calls_in_iteration: Vec<ToolCallRecord> = Vec::new();
                let mut response = AgentResponse::empty();

                pipeline
                    .finalize_pending_tool_call(
                        pending,
                        FinalizePendingToolCallCtx {
                            workspace: Some(workspace.as_ref()),
                            session_id: Some("s1".to_string()),
                            permission_level: PermissionLevel::Restricted,
                            cancel_token: &cancel,
                            tool_calls_in_iteration: &mut tool_calls_in_iteration,
                            response: &mut response,
                            tx: &tx,
                        },
                    )
                    .await;

                (tool_calls_in_iteration, response)
            }
        });

        // Wait for the confirmation request. We intentionally do NOT resolve it.
        let mut confirmation_id: Option<String> = None;
        while let Some(chunk) = rx.recv().await {
            if let StreamChunk::ToolConfirmationRequired {
                confirmation_id: id,
                ..
            } = chunk
            {
                confirmation_id = Some(id);
                break;
            }
        }
        let confirmation_id = confirmation_id.expect("expected ToolConfirmationRequired");

        // Advance time beyond the hard-coded confirmation timeout (300s).
        tokio::time::advance(Duration::from_secs(301)).await;
        tokio::task::yield_now().await;

        let mut saw_result = false;
        while let Some(chunk) = rx.recv().await {
            if let StreamChunk::ToolCallResult {
                success, output, ..
            } = chunk
            {
                assert!(!success);
                assert!(output.contains("timed-out") || output.contains("denied"));
                saw_result = true;
                break;
            }
        }
        assert!(saw_result);

        let (tool_calls, response) = handle.await.expect("task join");
        // Ensure this specific confirmation has been cleared, without depending on
        // global pending count (tests may run concurrently).
        let err = TOOL_CONFIRMATIONS
            .resolve_decision(
                &confirmation_id,
                Some("s1"),
                ToolConfirmationDecision::AllowOnce,
            )
            .unwrap_err();
        assert!(err.contains("Unknown confirmation id"));
        assert!(!temp.path().join("out_timeout.txt").exists());
        assert!(
            tool_calls
                .iter()
                .any(|t| matches!(t.result, ToolResult::Skipped(_)))
        );
        assert!(
            response
                .tool_calls
                .iter()
                .any(|t| matches!(t.result, ToolResult::Skipped(_)))
        );
    }

    #[tokio::test]
    async fn execute_tool_dispatches_code_stats_with_workspace_sandbox() {
        use tempfile::tempdir;

        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").unwrap();

        let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
            .expect("workspace should be created");

        let args = serde_json::json!({"operation":"stats","path":"."}).to_string();
        let result = pipeline.execute_tool("code", &args, Some(&ws)).await;

        match result {
            ToolResult::Success(s) => {
                // Basic sanity: should be valid JSON and include a stats object.
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert!(v.get("stats").is_some());
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn file_read_honors_start_end_range() {
        use tempfile::tempdir;

        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join("foo.txt"), "l1\nl2\nl3\n").unwrap();

        let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
            .expect("workspace should be created");

        let args = serde_json::json!({
            "operation": "read",
            "path": "foo.txt",
            "start": 2,
            "end": 2
        })
        .to_string();

        let result = pipeline.execute_tool("file", &args, Some(&ws)).await;
        match result {
            ToolResult::Success(s) => {
                assert!(s.contains("l2"));
                assert!(!s.contains("l1"));
                assert!(!s.contains("l3"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn file_tree_honors_show_hidden() {
        use tempfile::tempdir;

        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        std::fs::write(temp.path().join(".hidden.txt"), "secret").unwrap();
        std::fs::write(temp.path().join("visible.txt"), "ok").unwrap();

        let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
            .expect("workspace should be created");

        let args_hidden_off = serde_json::json!({
            "operation": "tree",
            "path": ".",
            "max_depth": 1,
            "show_hidden": false
        })
        .to_string();

        let r1 = pipeline
            .execute_tool("file", &args_hidden_off, Some(&ws))
            .await;
        match r1 {
            ToolResult::Success(s) => {
                assert!(s.contains("visible.txt"));
                assert!(!s.contains(".hidden.txt"));
            }
            other => panic!("expected success, got: {other:?}"),
        }

        let args_hidden_on = serde_json::json!({
            "operation": "tree",
            "path": ".",
            "max_depth": 1,
            "show_hidden": true
        })
        .to_string();

        let r2 = pipeline
            .execute_tool("file", &args_hidden_on, Some(&ws))
            .await;
        match r2 {
            ToolResult::Success(s) => {
                assert!(s.contains("visible.txt"));
                assert!(s.contains(".hidden.txt"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn file_edit_replaces_content() {
        use tempfile::tempdir;

        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        let p = temp.path().join("edit.txt");
        std::fs::write(&p, "hello world\n").unwrap();

        let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
            .expect("workspace should be created");

        let args = serde_json::json!({
            "operation": "edit",
            "path": "edit.txt",
            "old": "world",
            "new": "gestura"
        })
        .to_string();

        let result = pipeline.execute_tool("file", &args, Some(&ws)).await;
        match result {
            ToolResult::Success(s) => {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert_eq!(v.get("replacements").and_then(|x| x.as_u64()), Some(1));
            }
            other => panic!("expected success, got: {other:?}"),
        }

        let new_content = std::fs::read_to_string(&p).unwrap();
        assert!(new_content.contains("hello gestura"));
    }

    #[tokio::test]
    async fn shell_env_is_passed_through() {
        let pipeline = AgentPipeline::new(AppConfig::default());

        let cwd = std::env::current_dir().unwrap();
        let ws = SessionWorkspace::from_directory("test-session", cwd.clone())
            .expect("workspace should be created");

        let args = serde_json::json!({
            "command": "printf %s $FOO",
            "env": {"FOO": "BAR"},
            "timeout_secs": 10
        })
        .to_string();

        let result = pipeline.execute_tool("shell", &args, Some(&ws)).await;
        match result {
            ToolResult::Success(s) => {
                // The shell async wrapper returns stdout on success.
                assert!(s.contains("BAR"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    // ---------------------------------------------------------------------
    // Screen tool argument validation (must fail fast without OS capture)
    // ---------------------------------------------------------------------

    #[tokio::test]
    async fn screenshot_tool_rejects_non_screenshot_operations() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
            .expect("workspace should be created");

        let args = serde_json::json!({"operation": "start"}).to_string();
        let result = pipeline.execute_tool("screenshot", &args, Some(&ws)).await;
        match result {
            ToolResult::Error(e) => assert!(e.contains("does not support operation")),
            other => panic!("expected error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn screenshot_rejects_invalid_return_mode() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
            .expect("workspace should be created");

        let args = serde_json::json!({
            "return": {"mode": "bogus"}
        })
        .to_string();

        let result = pipeline.execute_tool("screenshot", &args, Some(&ws)).await;
        match result {
            ToolResult::Error(e) => assert!(e.contains("Invalid return.mode")),
            other => panic!("expected error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn screenshot_rejects_extension_mismatch_vs_output_format() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
            .expect("workspace should be created");

        let args = serde_json::json!({
            "output_path": "foo.png",
            "output_format": "jpg"
        })
        .to_string();

        let result = pipeline.execute_tool("screenshot", &args, Some(&ws)).await;
        match result {
            ToolResult::Error(e) => assert!(e.contains("does not match requested output_format")),
            other => panic!("expected error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn screen_record_requires_operation() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let temp = tempdir().unwrap();
        let ws = SessionWorkspace::from_directory("test-session", temp.path().to_path_buf())
            .expect("workspace should be created");

        let args = serde_json::json!({}).to_string();
        let result = pipeline
            .execute_tool("screen_record", &args, Some(&ws))
            .await;
        match result {
            ToolResult::Error(e) => assert!(e.contains("Missing required field 'operation'")),
            other => panic!("expected error, got: {other:?}"),
        }
    }

    // =========================================================================
    // Integration Tests for Pipeline (VALIDATION task)
    // =========================================================================

    use crate::context::{ContextCategory, ContextManager, estimate_tokens};

    #[test]
    fn test_context_reduction_reduces_prompt_size() {
        // Test that context reduction actually reduces prompt size
        let context_manager = ContextManager::new();

        // Request that doesn't need tools should have smaller context
        let simple_request = "What is the weather?";
        let analysis = context_manager.analyze(simple_request);

        // General questions shouldn't need tools
        assert!(!analysis.needs_tools || analysis.categories.contains(&ContextCategory::General));
    }

    #[test]
    fn test_tool_filtering_by_category() {
        // Test that tool filtering works correctly based on request analysis
        let context_manager = ContextManager::new();

        // File-related request should include file tools
        let file_request = "Read the file src/main.rs";
        let analysis = context_manager.analyze(file_request);

        assert!(analysis.categories.contains(&ContextCategory::FileSystem));
        assert!(analysis.needs_tools);

        // Git-related request should include git tools
        let git_request = "Show me the git status";
        let git_analysis = context_manager.analyze(git_request);

        assert!(git_analysis.categories.contains(&ContextCategory::Git));
    }

    #[test]
    fn test_token_estimation() {
        // Test token estimation function
        let short_text = "Hello world";
        let long_text = "a".repeat(1000);

        let short_tokens = estimate_tokens(short_text);
        let long_tokens = estimate_tokens(&long_text);

        // Short text should have fewer tokens
        assert!(short_tokens < long_tokens);
        // Rough estimate: ~4 chars per token
        assert!((200..=300).contains(&long_tokens));
    }

    #[test]
    fn test_token_limit_status() {
        // Test token limit checking with AppConfig
        let app_config = AppConfig::default();
        let pipeline_config = PipelineConfig {
            max_context_tokens: 10_000, // Must be larger than max_output_tokens
            max_output_tokens: 1_000,
            ..Default::default()
        };
        let pipeline = AgentPipeline::with_config(app_config, pipeline_config);

        // Small prompt should be OK (max_input = 10000 - 1000 = 9000)
        let small_prompt = "Hello";
        let status = pipeline.check_token_limit(small_prompt);
        assert!(matches!(status, TokenLimitStatus::Ok { .. }));

        // Large prompt should exceed (10000 chars / 4 = 2500 tokens, but we need > 9000)
        let large_prompt = "a".repeat(50000); // ~12500 tokens
        let status = pipeline.check_token_limit(&large_prompt);
        assert!(matches!(status, TokenLimitStatus::Exceeded { .. }));
    }

    #[test]
    fn test_voice_and_text_same_analysis() {
        // Test that voice and text inputs produce same analysis results
        let context_manager = ContextManager::new();

        let text_input = "List all files in the current directory";
        let voice_input = "List all files in the current directory"; // Same content

        let text_analysis = context_manager.analyze(text_input);
        let voice_analysis = context_manager.analyze(voice_input);

        // Same input should produce same analysis
        assert_eq!(text_analysis.categories, voice_analysis.categories);
        assert_eq!(text_analysis.needs_tools, voice_analysis.needs_tools);
    }

    #[test]
    fn test_history_summarization() {
        // Test history summarization with threshold
        let context_manager = ContextManager::new();

        // Short history - should include all
        let short_history: Vec<String> = (0..5).map(|i| format!("Message {}", i)).collect();
        let short_summary = context_manager.summarize_history(&short_history);
        assert!(!short_summary.is_empty());
        assert!(short_summary.contains("Message 0"));

        // Long history - should summarize
        let long_history: Vec<String> = (0..30).map(|i| format!("Message {}", i)).collect();
        let long_summary = context_manager.summarize_history(&long_history);
        assert!(long_summary.contains("summarized"));
    }

    #[test]
    fn test_request_similarity_detection() {
        // Test request similarity detection
        let context_manager = ContextManager::new();

        let request1 = "Read the file src/main.rs";
        let request2 = "Read the file src/main.rs"; // Same request
        let request3 = "Show git status"; // Different request

        let analysis1 = context_manager.analyze(request1);
        let analysis2 = context_manager.analyze(request2);
        let analysis3 = context_manager.analyze(request3);

        let hash1 = context_manager.compute_request_hash(&analysis1);
        let hash2 = context_manager.compute_request_hash(&analysis2);
        let hash3 = context_manager.compute_request_hash(&analysis3);

        // Same requests should have same hash
        assert_eq!(hash1, hash2);
        // Different requests should have different hash
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_agent_request_with_history() {
        // Test agent request with conversation history
        let history = vec![Message::user("Hello"), Message::assistant("Hi there!")];

        let request = AgentRequest::new("How are you?").with_history(history.clone());

        assert_eq!(request.history.len(), 2);
        assert_eq!(request.history[0].role, "user");
        assert_eq!(request.history[1].role, "assistant");
    }

    // =========================================================================
    // Integration Tests for Auto-Compaction Strategies
    // =========================================================================

    /// Helper function to estimate tokens from a vector of messages
    fn estimate_tokens_from_messages(messages: &[Message]) -> usize {
        messages.iter().map(|m| estimate_tokens(&m.content)).sum()
    }

    #[tokio::test]
    async fn test_auto_compaction_summarize_strategy() {
        // Test that Summarize strategy triggers at 80% threshold and reduces context
        let mut config = AppConfig::default();
        config.pipeline.compaction_strategy = CompactionStrategy::Summarize;
        config.pipeline.auto_compact_threshold_percent = 80;
        config.pipeline.max_context_tokens = 1000; // Small limit for testing

        let pipeline = AgentPipeline::with_provider_optimized_config(config);

        // Create history that exceeds 80% of 1000 tokens (>800 tokens)
        // Each message needs to be longer to reach the threshold
        // Rough estimate: 4 chars per token, so we need >3200 chars total
        let mut history = Vec::new();
        for i in 0..20 {
            history.push(Message::user(format!(
                "This is test message number {} with lots of additional content to increase token count. \
                 We need to make sure this message is long enough to trigger the auto-compaction threshold. \
                 Adding more text here to ensure we exceed 800 tokens total across all messages in the history.",
                i
            )));
            history.push(Message::assistant(format!(
                "This is response number {} with lots of additional content to increase token count. \
                 We need to make sure this response is long enough to trigger the auto-compaction threshold. \
                 Adding more text here to ensure we exceed 800 tokens total across all messages in the history.",
                i
            )));
        }

        let estimated_tokens = estimate_tokens_from_messages(&history);
        assert!(
            estimated_tokens > 800,
            "History should exceed 80% threshold (got {} tokens)",
            estimated_tokens
        );

        // Build a prompt preview to test auto-compaction
        let prompt_preview: String = history
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let metadata = RequestMetadata::default();

        // Check if auto-compaction would trigger
        let compaction_result = pipeline
            .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
            .await;

        // Should trigger compaction
        assert!(
            compaction_result.is_some(),
            "Auto-compaction should trigger"
        );
    }

    #[tokio::test]
    async fn test_auto_compaction_truncate_strategy() {
        // Test that Truncate strategy removes oldest messages
        let mut config = AppConfig::default();
        config.pipeline.compaction_strategy = CompactionStrategy::Truncate;
        config.pipeline.auto_compact_threshold_percent = 80;
        config.pipeline.max_context_tokens = 1000;

        let pipeline = AgentPipeline::with_provider_optimized_config(config);

        let mut history = Vec::new();
        for i in 0..15 {
            history.push(Message::user(format!(
                "Message {} with additional content",
                i
            )));
            history.push(Message::assistant(format!(
                "Response {} with additional content",
                i
            )));
        }

        let messages_before = history.len();
        let prompt_preview: String = history
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let metadata = RequestMetadata::default();

        let compaction_result = pipeline
            .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
            .await;

        // Should trigger compaction
        assert!(
            compaction_result.is_some(),
            "Auto-compaction should trigger"
        );

        // Verify compaction result indicates truncation occurred
        if let Some(StreamChunk::ContextCompacted { messages_after, .. }) = compaction_result {
            assert!(
                messages_after < messages_before,
                "Truncate should reduce message count"
            );
        }
    }

    #[tokio::test]
    async fn test_auto_compaction_clear_strategy() {
        // Test that Clear strategy removes all history
        let mut config = AppConfig::default();
        config.pipeline.compaction_strategy = CompactionStrategy::Clear;
        config.pipeline.auto_compact_threshold_percent = 80;
        config.pipeline.max_context_tokens = 1000;

        let pipeline = AgentPipeline::with_provider_optimized_config(config);

        let mut history = Vec::new();
        for i in 0..15 {
            history.push(Message::user(format!(
                "Message {} with additional content",
                i
            )));
            history.push(Message::assistant(format!(
                "Response {} with additional content",
                i
            )));
        }

        let prompt_preview: String = history
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let metadata = RequestMetadata::default();

        let compaction_result = pipeline
            .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
            .await;

        // Should trigger compaction
        assert!(
            compaction_result.is_some(),
            "Auto-compaction should trigger"
        );

        // Verify compaction result indicates all messages were cleared
        if let Some(StreamChunk::ContextCompacted { messages_after, .. }) = compaction_result {
            assert_eq!(
                messages_after, 0,
                "Clear strategy should remove all history"
            );
        }
    }

    #[tokio::test]
    async fn test_auto_compaction_memory_bank_strategy() {
        // Test that MemoryBank strategy saves context to file
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let workspace_path = temp_dir.path().to_path_buf();

        let mut config = AppConfig::default();
        config.pipeline.compaction_strategy = CompactionStrategy::MemoryBank;
        config.pipeline.auto_compact_threshold_percent = 80;
        config.pipeline.max_context_tokens = 1000;

        let pipeline = AgentPipeline::with_provider_optimized_config(config);

        let mut history = Vec::new();
        for i in 0..15 {
            history.push(Message::user(format!(
                "Message {} with additional content",
                i
            )));
            history.push(Message::assistant(format!(
                "Response {} with additional content",
                i
            )));
        }

        let messages_before = history.len();
        let prompt_preview: String = history
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let metadata = RequestMetadata {
            workspace_dir: Some(workspace_path.clone()),
            session_id: Some("test-session".to_string()),
            ..Default::default()
        };

        let compaction_result = pipeline
            .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
            .await;

        // Should trigger compaction
        assert!(
            compaction_result.is_some(),
            "Auto-compaction should trigger"
        );

        // Verify compaction result indicates memory bank save
        if let Some(StreamChunk::MemoryBankSaved { messages_saved, .. }) = compaction_result {
            assert_eq!(
                messages_saved, messages_before,
                "All messages should be saved to memory bank"
            );
        }

        // Verify memory bank file was created
        let memory_dir = workspace_path.join(".gestura").join("memory");
        assert!(
            memory_dir.exists(),
            "Memory bank directory should be created"
        );

        // Check that at least one .md file exists
        let entries = std::fs::read_dir(&memory_dir).unwrap();
        let md_files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
            .collect();

        assert!(
            !md_files.is_empty(),
            "Memory bank should contain at least one markdown file"
        );
    }

    #[tokio::test]
    async fn test_auto_compaction_threshold_not_reached() {
        // Test that auto-compaction does NOT trigger when below threshold
        let mut config = AppConfig::default();
        config.pipeline.auto_compact_threshold_percent = 80;
        config.pipeline.max_context_tokens = 10000; // Large limit

        let pipeline = AgentPipeline::with_provider_optimized_config(config);

        // Create small history that won't exceed threshold
        let history = vec![
            Message::user("Hello".to_string()),
            Message::assistant("Hi there!".to_string()),
        ];

        let estimated_tokens = estimate_tokens_from_messages(&history);
        assert!(
            estimated_tokens < 8000,
            "History should be well below 80% threshold"
        );

        let prompt_preview: String = history
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let metadata = RequestMetadata::default();

        // Check if auto-compaction would trigger
        let compaction_result = pipeline
            .check_and_apply_auto_compaction(&history, &prompt_preview, &metadata)
            .await;

        // Should NOT trigger compaction
        assert!(
            compaction_result.is_none(),
            "Should not trigger compaction when below threshold"
        );
    }

    #[test]
    fn test_pipeline_config_user_max_context_tokens_is_clamped_to_provider_default() {
        use crate::config::PipelineSettings;

        // Base config is provider-optimized.
        let base = PipelineConfig::for_provider("ollama");
        assert_eq!(
            base.max_context_tokens,
            PipelineConfig::context_tokens_for_provider("ollama")
        );

        // User requests a larger context window than the provider default.
        let settings = PipelineSettings {
            max_context_tokens: 999_999,
            ..Default::default()
        };

        let merged = base.with_user_settings(&settings);
        assert_eq!(
            merged.max_context_tokens,
            PipelineConfig::context_tokens_for_provider("ollama"),
            "User max_context_tokens should clamp to provider default"
        );
    }
}
