use super::*;

impl AgentPipeline {
    /// Execute the agentic loop with streaming
    ///
    /// If `workspace` is provided, all tool operations (shell, file, git) will be
    /// sandboxed to that directory. Paths outside the workspace will be rejected.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_agentic_loop_streaming(
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
            // Even with no built-in tools, MCP tools may be available.
            let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
            if mcp_tools.is_empty() {
                None
            } else {
                Some(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools))
            }
        } else {
            let mut schemas = crate::tools::schemas::build_provider_tool_schemas(&tools);
            // Merge in any MCP tool schemas.
            let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
            if !mcp_tools.is_empty() {
                schemas.merge(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools));
            }
            Some(schemas)
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
                    StreamChunk::ShellOutput { .. } => {
                        // Forward real-time shell output to frontend
                        let _ = tx.send(chunk).await;
                    }
                    StreamChunk::ShellLifecycle { .. } => {
                        // Forward shell lifecycle events to frontend
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
                    StreamChunk::Paused => {
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
    pub(super) async fn execute_agentic_loop_blocking(
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
            let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
            if mcp_tools.is_empty() {
                None
            } else {
                Some(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools))
            }
        } else {
            let mut schemas = crate::tools::schemas::build_provider_tool_schemas(&tools);
            let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
            if !mcp_tools.is_empty() {
                schemas.merge(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools));
            }
            Some(schemas)
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
}
