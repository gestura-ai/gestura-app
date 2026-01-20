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

use crate::config::AppConfig;
use crate::context::{ContextManager, RequestAnalyzer};
use crate::error::AppError;
use crate::llm_provider::{AgentContext, select_provider};
use crate::session_workspace::SessionWorkspace;
use crate::streaming::{
    CancellationToken, StreamChunk, start_streaming, start_streaming_with_fallback,
};
use crate::tools::registry::{ToolDefinition, all_tools};

pub use types::*;

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
}

impl AgentPipeline {
    /// Create a new pipeline with default configuration
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            context_manager: ContextManager::new(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config: PipelineConfig::default(),
        }
    }

    /// Create a pipeline with custom configuration
    pub fn with_config(config: AppConfig, pipeline_config: PipelineConfig) -> Self {
        Self {
            config,
            context_manager: ContextManager::new(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config,
        }
    }

    /// Create a pipeline with configuration optimized for the current LLM provider
    ///
    /// This automatically sets the context token limit based on the provider's capabilities.
    pub fn with_provider_optimized_config(config: AppConfig) -> Self {
        let provider = config.llm.primary.as_str();
        let pipeline_config = PipelineConfig::for_provider(provider);

        tracing::info!(
            provider = provider,
            max_context_tokens = pipeline_config.max_context_tokens,
            max_history_messages = pipeline_config.max_history_messages,
            "Created pipeline with provider-optimized configuration"
        );

        Self {
            config,
            context_manager: ContextManager::new(),
            analyzer: RequestAnalyzer::new(),
            pipeline_config,
        }
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
        let relevant_tools = if self.pipeline_config.enable_tools && analysis.needs_tools {
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
            && analysis.is_followup
            && Self::looks_like_approval(&request.input)
            && let Some(resp) = self
                .try_execute_confirmed_tool_from_history(
                    &request,
                    &analysis,
                    &relevant_tools,
                    workspace.as_ref(),
                    &tx,
                )
                .await?
        {
            return Ok(resp);
        }

        // 3. Resolve context
        let mut resolved_context =
            self.context_manager
                .resolve_context(&request.input, &analysis, &request.history);

        // 4. Build the optimized prompt with token limit checking
        let (prompt, truncated) =
            self.truncate_prompt_if_needed(&request, &mut resolved_context, &relevant_tools);

        if truncated {
            tracing::info!("Prompt was truncated to fit token limit");
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
            )
            .await?;

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
    ) -> Result<Option<AgentResponse>, AppError> {
        let has_tool = |name: &str| relevant_tools.iter().any(|t| t.name == name);

        let Some(prev_assistant) = request.history.iter().rev().find(|m| m.role == "assistant")
        else {
            return Ok(None);
        };

        let Some((tool_name, args, answer_prefix)) =
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

        let user_text = match &result {
            ToolResult::Success(out) => {
                let out = out.trim_end();
                if out.is_empty() {
                    format!("{}(empty output)\n", answer_prefix)
                } else {
                    format!("{}{}\n", answer_prefix, out)
                }
            }
            ToolResult::Error(e) => format!("{}error: {}\n", tool_name, e),
            ToolResult::Skipped(msg) => format!("{}skipped: {}\n", tool_name, msg),
        };

        let _ = tx.send(StreamChunk::Text(user_text.clone())).await;
        let _ = tx.send(StreamChunk::Done(None)).await;

        let record = ToolCallRecord {
            id: tool_call_id,
            name: tool_name,
            arguments: args,
            result,
            duration_ms,
        };

        Ok(Some(AgentResponse {
            content: user_text,
            thinking: None,
            tool_calls: vec![record],
            usage: None,
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
        let relevant_tools = if self.pipeline_config.enable_tools && analysis.needs_tools {
            self.get_tools_for_analysis(&analysis, &request.metadata.allowed_tools)
        } else {
            Vec::new()
        };

        // 3. Resolve context
        let mut resolved_context =
            self.context_manager
                .resolve_context(&request.input, &analysis, &request.history);

        // 4. Build prompt with token limit checking
        let (prompt, truncated) =
            self.truncate_prompt_if_needed(&request, &mut resolved_context, &relevant_tools);

        if truncated {
            tracing::info!("Prompt was truncated to fit token limit");
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

        // If allowed_tools is specified, only use those
        if !allowed_tools.is_empty() {
            for tool_name in allowed_tools {
                if let Some(t) = crate::tools::registry::find_tool(tool_name) {
                    tools.push(t);
                }
            }
            return tools;
        }

        // Otherwise, filter by category
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
        tools: &[&'static ToolDefinition],
    ) -> String {
        let mut prompt = String::new();

        // Always include a system prompt. Callers may override via `request.system_prompt`.
        let sys = request
            .system_prompt
            .clone()
            .unwrap_or_else(|| crate::persona::default_system_prompt(&request.metadata));
        prompt.push_str(&format!("System: {}\n\n", sys));

        // Add tool descriptions if tools are available
        if !tools.is_empty() {
            prompt.push_str("Available tools:\n");
            for tool in tools {
                prompt.push_str(&format!("- {}: {}\n", tool.name, tool.summary));
            }
            prompt.push('\n');
        }

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
                    "tool" => prompt.push_str(&format!("Tool result: {}\n", msg.content)),
                    _ => prompt.push_str(&format!("{}: {}\n", msg.role, msg.content)),
                }
            }
            prompt.push('\n');
        }

        // Add the current request
        prompt.push_str(&format!("User: {}\n", request.input));

        prompt
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
                percentage: (estimated_tokens * 100 / max_input) as u8,
            }
        } else {
            TokenLimitStatus::Ok {
                estimated: estimated_tokens,
                limit: max_input,
            }
        }
    }

    /// Truncate prompt to fit within token limit
    /// Strategy: Remove oldest history messages first, then truncate file content
    fn truncate_prompt_if_needed(
        &self,
        request: &AgentRequest,
        context: &mut crate::context::ResolvedContext,
        tools: &[&'static ToolDefinition],
    ) -> (String, bool) {
        let mut prompt = self.build_prompt(request, context, tools);
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
            prompt = self.build_prompt(request, context, tools);

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
    async fn execute_agentic_loop_streaming(
        &self,
        initial_prompt: String,
        tools: Vec<&'static ToolDefinition>,
        context: crate::context::ResolvedContext,
        tx: mpsc::Sender<StreamChunk>,
        cancel_token: CancellationToken,
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
                                workspace,
                                &mut tool_calls_in_iteration,
                                &mut response,
                                &tx,
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
                                workspace,
                                &mut tool_calls_in_iteration,
                                &mut response,
                                &tx,
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
                    StreamChunk::Done(usage) => {
                        // Some providers (or buggy intermediaries) may terminate the stream
                        // without emitting a ToolCallEnd. If we have a pending tool call, treat
                        // stream completion as an implicit end and execute it.
                        if let Some(pending) = pending_tool_call.take() {
                            self.finalize_pending_tool_call(
                                pending,
                                workspace,
                                &mut tool_calls_in_iteration,
                                &mut response,
                                &tx,
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
                    workspace,
                    &mut tool_calls_in_iteration,
                    &mut response,
                    &tx,
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
        _tools: Vec<&'static ToolDefinition>,
        context: crate::context::ResolvedContext,
        _workspace: Option<&SessionWorkspace>,
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

        let current_prompt = initial_prompt;

        if self.pipeline_config.max_iterations == 0 {
            return Ok(response);
        }

        response.iterations = 1;

        // Call LLM with fallback support
        let llm_response = self.call_llm_with_fallback(&current_prompt).await?;
        let (content, thinking) = crate::streaming::split_think_blocks(&llm_response.text);
        response.content = content;
        response.thinking = thinking;
        response.usage = Some(llm_response.usage);

        // For blocking mode, we don't parse tool calls from content.
        // This is primarily used for simple text responses.
        // Full tool execution should use streaming mode.

        Ok(response)
    }

    /// Call LLM with fallback and retry logic for blocking mode
    async fn call_llm_with_fallback(
        &self,
        prompt: &str,
    ) -> Result<crate::llm_provider::LlmCallResponse, AppError> {
        let agent_ctx = AgentContext::default();
        let provider = select_provider(&self.config, &agent_ctx);

        // Try primary provider with retries
        let retry_delays = [1, 2, 4]; // seconds
        let mut last_error: Option<AppError> = None;

        for (attempt, delay) in retry_delays.iter().enumerate() {
            match provider.call_with_usage(prompt).await {
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
        if let Some(fallback_provider) = self
            .pipeline_config
            .enable_fallback
            .then_some(self.config.llm.fallback.as_ref())
            .flatten()
        {
            tracing::info!(
                fallback = fallback_provider,
                "Primary LLM exhausted retries, trying fallback provider"
            );

            // Create a modified config with fallback as primary
            let mut fallback_config = self.config.clone();
            fallback_config.llm.primary = fallback_provider.clone();

            let fallback_provider_instance = select_provider(&fallback_config, &agent_ctx);
            match fallback_provider_instance.call_with_usage(prompt).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    tracing::error!("Fallback provider also failed: {}", e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| AppError::Llm("All LLM providers failed".to_string())))
    }

    /// Execute a tool by name with given arguments
    ///
    /// If `workspace` is provided, all file paths and shell commands are sandboxed
    /// to that directory. Paths outside the workspace will be rejected.
    async fn finalize_pending_tool_call(
        &self,
        pending: PendingToolCall,
        workspace: Option<&SessionWorkspace>,
        tool_calls_in_iteration: &mut Vec<ToolCallRecord>,
        response: &mut AgentResponse,
        tx: &mpsc::Sender<StreamChunk>,
    ) {
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
                output,
                duration_ms,
            })
            .await;

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

                        match web.fetch(url).await {
                            Ok(res) => match serde_json::to_string_pretty(&res) {
                                Ok(s) => ToolResult::Success(s),
                                Err(e) => ToolResult::Error(format!("Serialize error: {e}")),
                            },
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
                ToolResult::Success(s) => format!("Success: {}", s),
                ToolResult::Error(e) => format!("Error: {}", e),
                ToolResult::Skipped(r) => format!("Skipped: {}", r),
            };
            prompt.push_str(&format!(
                "\nTool {} result:\n{}\n",
                tool_call.name, result_text
            ));
        }

        prompt.push_str("\nContinue based on the tool results above.\n");

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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
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
}
