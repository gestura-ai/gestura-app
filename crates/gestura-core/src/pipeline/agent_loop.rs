use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IncompleteRunReason {
    MissingTerminalSummary,
    IterationBudgetExhausted { max_iterations: usize },
}

impl AgentPipeline {
    fn has_meaningful_final_text(text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return false;
        }

        let alnum_count = trimmed.chars().filter(|c| c.is_alphanumeric()).count();
        let word_count = trimmed.split_whitespace().count();

        alnum_count >= 24 || word_count >= 5
    }

    fn text_defers_remaining_work(text: &str) -> bool {
        let normalized = text.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        [
            "remaining:",
            "remaining work",
            "next turn",
            "will resume",
            "resume with",
            "not executed yet",
            "no code edits",
            "not complete",
            "still need to",
            "left to do",
            "to be done",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }

    fn text_signals_user_blocker_or_question(text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return false;
        }

        let normalized = trimmed.to_ascii_lowercase();
        if trimmed.ends_with('?') {
            return true;
        }

        [
            "need your input",
            "need your confirmation",
            "please confirm",
            "can you confirm",
            "what would you like",
            "which would you like",
            "please provide",
            "i need",
            "i'm blocked",
            "i am blocked",
            "cannot proceed",
            "can't proceed",
            "permission required",
            "approval required",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }

    fn active_task_has_open_descendants(
        &self,
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> bool {
        let (Some(session_id), Some(task_id)) = (session_id, task_id) else {
            return false;
        };

        let manager = crate::get_global_task_manager();
        let Ok(descendants) = manager.list_descendants(session_id, task_id) else {
            return false;
        };

        descendants.into_iter().any(|task| {
            !matches!(
                task.status,
                crate::TaskStatus::Completed | crate::TaskStatus::Cancelled
            )
        })
    }

    fn build_forced_execution_prompt(
        &self,
        current_prompt: &str,
        response_so_far: &str,
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> String {
        let mut prompt = current_prompt.to_string();

        if !response_so_far.trim().is_empty() {
            prompt.push_str(&format!(
                "\nAssistant progress so far:\n{}\n",
                self.truncate_tool_result(response_so_far)
            ));
        }

        if let (Some(session_id), Some(task_id)) = (session_id, task_id) {
            let manager = crate::get_global_task_manager();
            if let Ok(Some(task)) = manager.get_task(session_id, task_id)
                && let Ok(descendants) = manager.list_descendants(session_id, task_id)
            {
                let pending_subtasks = descendants
                    .into_iter()
                    .filter(|subtask| {
                        !matches!(
                            subtask.status,
                            crate::TaskStatus::Completed | crate::TaskStatus::Cancelled
                        )
                    })
                    .take(6)
                    .map(|subtask| format!("- {} [{}]", subtask.name, subtask.status))
                    .collect::<Vec<_>>();

                prompt.push_str(&format!(
                    "\nTracked task still in progress: {}\n",
                    task.name
                ));
                if !pending_subtasks.is_empty() {
                    prompt.push_str("Pending subtasks:\n");
                    for subtask in pending_subtasks {
                        prompt.push_str(&subtask);
                        prompt.push('\n');
                    }
                }
            }
        }

        prompt.push_str(
            "\nUser: The work is not finished yet. Continue the same run now by executing the next highest-priority incomplete subtask with tools. Update the tracked task statuses as you start or finish each subtask, and do not mark the root task complete until every planned subtask is completed or explicitly cancelled. Do not stop with another status update, plan recap, or promise to resume later unless you are genuinely blocked and explain the blocker clearly.\n",
        );

        prompt
    }

    fn build_forced_final_summary_prompt(
        &self,
        current_prompt: &str,
        response_so_far: &str,
    ) -> String {
        let mut prompt = current_prompt.to_string();

        if !response_so_far.trim().is_empty() {
            prompt.push_str(&format!(
                "\nAssistant progress so far:\n{}\n",
                self.truncate_tool_result(response_so_far)
            ));
        }

        prompt.push_str(
            "\nUser: Before you end this turn, provide a concise final status update for the user. Summarize what you accomplished, what remains (if anything), and any build/test/verification results you observed. Do not stop without a direct closing summary. Only call another tool if it is absolutely required to finish the request.\n",
        );

        prompt
    }

    fn build_synthetic_final_summary(
        &self,
        tool_calls: &[ToolCallRecord],
        reason: IncompleteRunReason,
    ) -> Option<String> {
        if tool_calls.is_empty() {
            return None;
        }

        let success_count = tool_calls
            .iter()
            .filter(|call| matches!(call.result, ToolResult::Success(_)))
            .count();
        let error_count = tool_calls
            .iter()
            .filter(|call| matches!(call.result, ToolResult::Error(_)))
            .count();
        let skipped_count = tool_calls
            .iter()
            .filter(|call| matches!(call.result, ToolResult::Skipped(_)))
            .count();

        let mut summary = match reason {
            IncompleteRunReason::MissingTerminalSummary => format!(
                "Status update: The agent completed {} tool call(s) ({} succeeded, {} failed, {} skipped), but the runtime ended the run without a terminal user-facing summary.",
                tool_calls.len(),
                success_count,
                error_count,
                skipped_count
            ),
            IncompleteRunReason::IterationBudgetExhausted { max_iterations } => format!(
                "Status update: The agent completed {} tool call(s) ({} succeeded, {} failed, {} skipped), but the runtime hit the iteration budget limit ({}) before the request finished.",
                tool_calls.len(),
                success_count,
                error_count,
                skipped_count,
                max_iterations
            ),
        };

        if let Some(last_call) = tool_calls.last() {
            let last_result = self.describe_tool_call_for_summary(last_call);
            summary.push(' ');
            summary.push_str(&last_result);
        }

        summary.push_str(" Review the tool activity above for the detailed outputs.");

        Some(summary)
    }

    fn has_iteration_headroom(iteration: usize, max_iterations: Option<usize>) -> bool {
        max_iterations.is_none_or(|limit| iteration + 1 < limit)
    }

    fn exhausted_iteration_budget(
        iterations_used: usize,
        max_iterations: Option<usize>,
    ) -> Option<usize> {
        max_iterations.filter(|limit| iterations_used >= *limit)
    }

    fn describe_tool_call_for_summary(&self, tool_call: &ToolCallRecord) -> String {
        let operation = serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
            .ok()
            .and_then(|value| {
                value
                    .get("operation")
                    .and_then(|operation| operation.as_str())
                    .map(str::to_string)
            });

        let action = match (tool_call.name.as_str(), operation.as_deref()) {
            ("file", Some("read")) => "read a file",
            ("file", Some("write")) => "write a file",
            ("file", Some("edit")) => "edit a file",
            ("file", Some("list")) => "list directory contents",
            ("file", Some("tree")) => "inspect the directory tree",
            ("file", Some("search")) => "search files",
            ("shell", Some("run")) | ("shell", _) => "run a shell command",
            ("git", Some("status")) => "check git status",
            ("git", Some("diff")) => "inspect a git diff",
            ("git", _) => "run a git operation",
            ("task", Some("create")) => "create a task",
            ("task", Some("update_status")) => "update task status",
            ("task", _) => "update task tracking",
            ("code", _) => "run a code analysis action",
            ("web", _) | ("web_search", _) => "look up web content",
            (_, Some(operation)) => {
                return format!(
                    "Last tool `{}` finished operation `{}`.",
                    tool_call.name, operation
                );
            }
            _ => return format!("Last tool `{}` finished.", tool_call.name),
        };

        match &tool_call.result {
            ToolResult::Success(_) => {
                format!("Last tool `{}` succeeded ({action}).", tool_call.name)
            }
            ToolResult::Error(_) => format!(
                "Last tool `{}` failed while trying to {}.",
                tool_call.name, action
            ),
            ToolResult::Skipped(_) => format!(
                "Last tool `{}` was skipped while trying to {}.",
                tool_call.name, action
            ),
        }
    }

    /// Execute the agentic loop with streaming
    ///
    /// If `workspace` is provided, all tool operations (shell, file, git) will be
    /// sandboxed to that directory. Paths outside the workspace will be rejected.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_agentic_loop_streaming(
        &self,
        initial_prompt: String,
        tools: Vec<&'static ToolDefinition>,
        include_mcp_tool_schemas: bool,
        context: crate::context::ResolvedContext,
        tx: mpsc::Sender<StreamChunk>,
        cancel_token: CancellationToken,
        workspace: Option<&SessionWorkspace>,
        session_id: Option<String>,
        task_id: Option<String>,
        max_iterations: Option<usize>,
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
        //
        // IMPORTANT: MCP tool schemas are only included when the pipeline has decided
        // they are relevant/allowed for this request. This prevents unrelated MCP
        // servers from delaying or destabilizing requests that only need built-in tools.
        let tool_schemas = if tools.is_empty() {
            if include_mcp_tool_schemas {
                let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
                if mcp_tools.is_empty() {
                    None
                } else {
                    Some(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools))
                }
            } else {
                None
            }
        } else {
            let mut schemas = crate::tools::schemas::build_provider_tool_schemas(&tools);
            if include_mcp_tool_schemas {
                let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
                if !mcp_tools.is_empty() {
                    schemas.merge(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools));
                }
            }
            Some(schemas)
        };

        tracing::debug!(
            builtin_tool_count = tools.len(),
            has_schemas = tool_schemas.is_some(),
            "[AgentLoop] Tool schemas initialized"
        );

        let mut saw_any_tool_calls = false;
        let mut forced_final_summary_requested = false;
        let mut delivered_terminal_summary = false;

        // Agentic loop - continue until no more tool calls, cancellation, or
        // an optional iteration budget limit.
        let mut iteration = 0usize;
        loop {
            if let Some(limit) = max_iterations
                && iteration >= limit
            {
                break;
            }

            if cancel_token.is_cancelled() {
                let _ = tx.send(cancel_token.interruption_chunk()).await;
                return Ok(response);
            }

            response.iterations = iteration + 1;

            tracing::debug!(
                iteration = iteration,
                permission_level = ?permission_level,
                max_iterations = max_iterations,
                "[AgentLoop] Starting iteration"
            );

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
            let streaming_cfg = crate::streaming::streaming_config_from(&self.config);
            let prompt = current_prompt.clone();
            let enable_fallback = self.pipeline_config.enable_fallback;
            let tool_schemas_for_iteration = tool_schemas.clone();

            // Spawn streaming task (with or without fallback)
            let stream_handle = tokio::spawn(async move {
                if enable_fallback {
                    start_streaming_with_fallback(
                        &streaming_cfg,
                        &prompt,
                        tool_schemas_for_iteration,
                        inner_tx,
                        inner_cancel,
                    )
                    .await
                } else {
                    start_streaming(
                        &streaming_cfg,
                        &prompt,
                        tool_schemas_for_iteration,
                        inner_tx,
                        inner_cancel,
                    )
                    .await
                }
            });

            tracing::debug!(
                iteration = iteration,
                "[AgentLoop] Streaming task spawned; consuming inner chunks"
            );

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
                        tracing::debug!(tool = %name, id = %id, "[AgentLoop] ToolCallStart received");
                        // Defensive: if the provider starts a new tool call without ending the
                        // previous one, finalize the previous call so we don't drop it.
                        if let Some(pending) = pending_tool_call.take() {
                            tracing::debug!(
                                tool = %pending.name,
                                "[AgentLoop] Defensive finalize of previous pending tool call"
                            );
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
                        // Forward ToolCallEnd to the frontend FIRST so the UI can transition
                        // the tool card from "running" → "executing" before we actually run
                        // the tool. This preserves the correct visual ordering:
                        //   ToolCallStart → ToolCallArgs → ToolCallEnd → ToolCallResult
                        let _ = tx.send(chunk).await;
                        if let Some(pending) = pending_tool_call.take() {
                            let tool_name_log = pending.name.clone();
                            let args_len_log = pending.arguments.len();
                            tracing::debug!(
                                tool = %tool_name_log,
                                args_len = args_len_log,
                                "[AgentLoop] ToolCallEnd: calling finalize_pending_tool_call"
                            );
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
                            tracing::debug!(
                                tool = %tool_name_log,
                                "[AgentLoop] finalize_pending_tool_call returned"
                            );
                        } else {
                            tracing::warn!(
                                "[AgentLoop] ToolCallEnd received but no pending tool call — may indicate a provider bug"
                            );
                        }
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
                    StreamChunk::ReflectionStarted { .. } => {
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ReflectionComplete { .. } => {
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ShellOutput { .. } => {
                        // Forward real-time shell output to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ShellLifecycle { .. } => {
                        // Forward shell lifecycle events to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::Done(usage) => {
                        tracing::debug!(
                            iteration = iteration,
                            tool_calls_so_far = tool_calls_in_iteration.len(),
                            "[AgentLoop] Done chunk received from inner stream"
                        );
                        // Some providers (or buggy intermediaries) may terminate the stream
                        // without emitting a ToolCallEnd. If we have a pending tool call, treat
                        // stream completion as an implicit end and execute it.
                        if let Some(pending) = pending_tool_call.take() {
                            tracing::debug!(
                                tool = %pending.name,
                                "[AgentLoop] Done received with pending tool call — implicit ToolCallEnd"
                            );
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
                    }
                    StreamChunk::Error(e) => {
                        tracing::error!(error = %e, iteration = iteration, "[AgentLoop] Error chunk received from inner stream");
                        let _ = tx.send(StreamChunk::Error(e.clone())).await;
                        return Err(AppError::Llm(e.clone()));
                    }
                    StreamChunk::Cancelled => {
                        tracing::debug!(
                            iteration = iteration,
                            "[AgentLoop] Cancelled chunk — aborting loop"
                        );
                        let _ = tx.send(chunk).await;
                        return Ok(response);
                    }
                    StreamChunk::Paused => {
                        tracing::debug!(
                            iteration = iteration,
                            "[AgentLoop] Paused chunk — suspending loop"
                        );
                        let _ = tx.send(chunk).await;
                        return Ok(response);
                    }
                }
            }

            tracing::debug!(
                iteration = iteration,
                tool_calls_count = tool_calls_in_iteration.len(),
                "[AgentLoop] Inner stream channel closed (while-recv loop exited)"
            );

            // If the inner stream ended unexpectedly (no Done/Error/Cancelled), but we have a
            // pending tool call, execute it so the agent loop can continue.
            if let Some(pending) = pending_tool_call.take() {
                tracing::warn!(
                    tool = %pending.name,
                    "[AgentLoop] Channel closed with pending tool call — unexpected; executing anyway"
                );
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

            tracing::debug!(
                iteration = iteration,
                tool_calls_count = tool_calls_in_iteration.len(),
                "[AgentLoop] Stream task joined"
            );

            // If no tool calls, we're done unless we still owe the user a closing summary.
            if tool_calls_in_iteration.is_empty() {
                let terminal_text_is_meaningful =
                    Self::has_meaningful_final_text(&iteration_content);
                if saw_any_tool_calls
                    && self
                        .active_task_has_open_descendants(session_id.as_deref(), task_id.as_deref())
                    && !Self::text_signals_user_blocker_or_question(&iteration_content)
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    tracing::warn!(
                        iteration = iteration,
                        "[AgentLoop] Tracked task still has open subtasks after a no-tool response — forcing execution continuation"
                    );
                    current_prompt = self.build_forced_execution_prompt(
                        &current_prompt,
                        &response.content,
                        session_id.as_deref(),
                        task_id.as_deref(),
                    );
                    iteration += 1;
                    continue;
                }

                if saw_any_tool_calls
                    && self
                        .active_task_has_open_descendants(session_id.as_deref(), task_id.as_deref())
                    && Self::text_defers_remaining_work(&iteration_content)
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    tracing::warn!(
                        iteration = iteration,
                        "[AgentLoop] Terminal status update deferred remaining tracked task work — forcing execution continuation"
                    );
                    current_prompt = self.build_forced_execution_prompt(
                        &current_prompt,
                        &response.content,
                        session_id.as_deref(),
                        task_id.as_deref(),
                    );
                    iteration += 1;
                    continue;
                }

                if saw_any_tool_calls
                    && !terminal_text_is_meaningful
                    && !forced_final_summary_requested
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    tracing::warn!(
                        iteration = iteration,
                        "[AgentLoop] Empty/non-substantive terminal iteration after tool use — forcing one final summary attempt"
                    );
                    current_prompt =
                        self.build_forced_final_summary_prompt(&current_prompt, &response.content);
                    forced_final_summary_requested = true;
                    iteration += 1;
                    continue;
                }

                tracing::debug!(
                    iteration = iteration,
                    "[AgentLoop] No tool calls in iteration — breaking loop"
                );
                delivered_terminal_summary = terminal_text_is_meaningful;
                break;
            }

            saw_any_tool_calls = true;
            forced_final_summary_requested = false;

            // Build continuation prompt with tool results
            current_prompt = self.build_tool_continuation_prompt(
                &current_prompt,
                &iteration_content,
                &tool_calls_in_iteration,
            );
            iteration += 1;
        }

        if saw_any_tool_calls && !delivered_terminal_summary {
            let reason = if let Some(limit) =
                Self::exhausted_iteration_budget(response.iterations, max_iterations)
            {
                IncompleteRunReason::IterationBudgetExhausted {
                    max_iterations: limit,
                }
            } else {
                IncompleteRunReason::MissingTerminalSummary
            };

            if let Some(summary) = self.build_synthetic_final_summary(&response.tool_calls, reason)
            {
                let emitted = if response.content.trim().is_empty() {
                    summary.clone()
                } else {
                    format!("\n\n{}", summary)
                };
                response.content.push_str(&emitted);
                let _ = tx.send(StreamChunk::Text(emitted)).await;
            }
        }

        Ok(response)
    }

    /// Execute the agentic loop without streaming (blocking)
    ///
    /// If `workspace` is provided, all tool operations (shell, file, git) will be
    /// sandboxed to that directory. Paths outside the workspace will be rejected.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_agentic_loop_blocking(
        &self,
        initial_prompt: String,
        tools: Vec<&'static ToolDefinition>,
        include_mcp_tool_schemas: bool,
        context: crate::context::ResolvedContext,
        workspace: Option<&SessionWorkspace>,
        session_id: Option<String>,
        task_id: Option<String>,
        max_iterations: Option<usize>,
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

        // Build provider-specific tool schemas so the model knows about available tools.
        // MCP schemas are only included when relevant/allowed.
        let tool_schemas = if tools.is_empty() {
            if include_mcp_tool_schemas {
                let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
                if mcp_tools.is_empty() {
                    None
                } else {
                    Some(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools))
                }
            } else {
                None
            }
        } else {
            let mut schemas = crate::tools::schemas::build_provider_tool_schemas(&tools);
            if include_mcp_tool_schemas {
                let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
                if !mcp_tools.is_empty() {
                    schemas.merge(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools));
                }
            }
            Some(schemas)
        };

        let mut current_prompt = initial_prompt;
        let mut saw_any_tool_calls = false;
        let mut forced_final_summary_requested = false;
        let mut delivered_terminal_summary = false;

        let mut iteration = 0usize;
        loop {
            if let Some(limit) = max_iterations
                && iteration >= limit
            {
                break;
            }

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
                let terminal_text_is_meaningful = Self::has_meaningful_final_text(&content);
                if saw_any_tool_calls
                    && self
                        .active_task_has_open_descendants(session_id.as_deref(), task_id.as_deref())
                    && !Self::text_signals_user_blocker_or_question(&content)
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    current_prompt = self.build_forced_execution_prompt(
                        &current_prompt,
                        &response.content,
                        session_id.as_deref(),
                        task_id.as_deref(),
                    );
                    iteration += 1;
                    continue;
                }

                if saw_any_tool_calls
                    && self
                        .active_task_has_open_descendants(session_id.as_deref(), task_id.as_deref())
                    && Self::text_defers_remaining_work(&content)
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    current_prompt = self.build_forced_execution_prompt(
                        &current_prompt,
                        &response.content,
                        session_id.as_deref(),
                        task_id.as_deref(),
                    );
                    iteration += 1;
                    continue;
                }

                if saw_any_tool_calls
                    && !terminal_text_is_meaningful
                    && !forced_final_summary_requested
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    current_prompt =
                        self.build_forced_final_summary_prompt(&current_prompt, &response.content);
                    forced_final_summary_requested = true;
                    iteration += 1;
                    continue;
                }

                response.content = content;
                response.thinking = thinking;
                delivered_terminal_summary = terminal_text_is_meaningful;
                break;
            }

            saw_any_tool_calls = true;
            forced_final_summary_requested = false;

            // Execute each structured tool call and collect records.
            let mut iteration_tool_calls: Vec<ToolCallRecord> = Vec::new();
            for tc in &llm_response.tool_calls {
                tracing::info!(
                    tool = %tc.name,
                    id = %tc.id,
                    "Blocking loop: executing tool call"
                );
                let result = self
                    .execute_tool(&tc.name, &tc.arguments, workspace, None)
                    .await;
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
            iteration += 1;
        }

        if saw_any_tool_calls && !delivered_terminal_summary {
            let reason = if let Some(limit) =
                Self::exhausted_iteration_budget(response.iterations, max_iterations)
            {
                IncompleteRunReason::IterationBudgetExhausted {
                    max_iterations: limit,
                }
            } else {
                IncompleteRunReason::MissingTerminalSummary
            };

            if let Some(summary) = self.build_synthetic_final_summary(&response.tool_calls, reason)
            {
                if response.content.trim().is_empty() {
                    response.content = summary;
                } else {
                    response.content.push_str("\n\n");
                    response.content.push_str(&summary);
                }
            }
        }

        Ok(response)
    }

    /// Call LLM with fallback and retry logic for blocking mode.
    ///
    /// When `tool_schemas` is provided, the appropriate provider-specific schema
    /// slice is selected and forwarded to [`LlmProvider::call_with_tools`].
    pub(super) async fn call_llm_with_fallback(
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meaningful_final_text_requires_real_summary_content() {
        assert!(!AgentPipeline::has_meaningful_final_text(""));
        assert!(!AgentPipeline::has_meaningful_final_text("done"));
        assert!(AgentPipeline::has_meaningful_final_text(
            "Built the app, ran the tests, and verified the hello world window renders correctly."
        ));
    }

    #[test]
    fn synthetic_final_summary_reports_tool_activity_transparently() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let summary = pipeline
            .build_synthetic_final_summary(
                &[
                    ToolCallRecord {
                        id: "1".to_string(),
                        name: "file".to_string(),
                        arguments: "{}".to_string(),
                        result: ToolResult::Success("created src/main.rs".to_string()),
                        duration_ms: 12,
                    },
                    ToolCallRecord {
                        id: "2".to_string(),
                        name: "shell".to_string(),
                        arguments: "{}".to_string(),
                        result: ToolResult::Error("cargo build failed".to_string()),
                        duration_ms: 40,
                    },
                ],
                IncompleteRunReason::MissingTerminalSummary,
            )
            .expect("summary should be generated");

        assert!(summary.contains("2 tool call(s)"));
        assert!(summary.contains("1 succeeded, 1 failed, 0 skipped"));
        assert!(summary.contains("runtime ended the run without a terminal user-facing summary"));
        assert!(summary.contains("Last tool `shell` failed while trying to run a shell command."));
        assert!(summary.contains("Review the tool activity above for the detailed outputs."));
        assert!(!summary.contains("cargo build failed"));
    }

    #[test]
    fn synthetic_final_summary_can_report_iteration_budget_exhaustion() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let summary = pipeline
            .build_synthetic_final_summary(
                &[ToolCallRecord {
                    id: "1".to_string(),
                    name: "shell".to_string(),
                    arguments: "{}".to_string(),
                    result: ToolResult::Success("cargo build".to_string()),
                    duration_ms: 12,
                }],
                IncompleteRunReason::IterationBudgetExhausted { max_iterations: 30 },
            )
            .expect("summary should be generated");

        assert!(summary.contains("iteration budget limit (30)"));
    }

    #[test]
    fn detects_deferred_remaining_work_in_status_updates() {
        assert!(AgentPipeline::text_defers_remaining_work(
            "Remaining: initialize the project and build it. Next turn will resume with the highest-priority incomplete subtask."
        ));
        assert!(AgentPipeline::text_defers_remaining_work(
            "No code edits, builds, or tests executed yet."
        ));
        assert!(!AgentPipeline::text_defers_remaining_work(
            "Implemented the UI, ran the tests, and everything passed successfully."
        ));
    }

    #[test]
    fn detects_when_text_is_a_real_user_blocker_or_question() {
        assert!(AgentPipeline::text_signals_user_blocker_or_question(
            "I need your confirmation before I overwrite the existing project."
        ));
        assert!(AgentPipeline::text_signals_user_blocker_or_question(
            "Which directory would you like me to use?"
        ));
        assert!(!AgentPipeline::text_signals_user_blocker_or_question(
            "Reviewing ls output and preparing the next implementation step."
        ));
    }

    #[test]
    fn forced_execution_prompt_requires_status_updates_before_completion() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let prompt = pipeline.build_forced_execution_prompt(
            "Inspect the project and continue execution.",
            "Created the scaffold, but have not built the app yet.",
            None,
            None,
        );

        assert!(
            prompt.contains("Update the tracked task statuses as you start or finish each subtask")
        );
        assert!(prompt.contains("do not mark the root task complete until every planned subtask is completed or explicitly cancelled"));
    }

    #[test]
    fn active_task_open_descendants_detects_nested_open_tasks() {
        let manager = crate::get_global_task_manager();
        let session_id = format!("agent-loop-descendants-{}", uuid::Uuid::new_v4());

        let mut root = crate::Task::new(&session_id, "Root", "Root", None);
        let mut child = crate::Task::new(&session_id, "Child", "Child", Some(root.id.clone()));
        let grandchild = crate::Task::new(
            &session_id,
            "Grandchild",
            "Grandchild",
            Some(child.id.clone()),
        );
        child.set_status(crate::TaskStatus::Completed);
        root.set_status(crate::TaskStatus::InProgress);

        let mut task_list = crate::TaskList::new(&session_id);
        task_list.add_task(root.clone());
        task_list.add_task(child);
        task_list.add_task(grandchild);
        manager
            .replace_task_list(task_list)
            .expect("replace task list");

        let pipeline = AgentPipeline::new(AppConfig::default());
        assert!(pipeline.active_task_has_open_descendants(Some(&session_id), Some(&root.id)));
    }
}
