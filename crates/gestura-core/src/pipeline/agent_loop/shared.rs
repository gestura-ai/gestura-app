use super::*;
use tokio::sync::mpsc;

/// Shared per-iteration preparation used by both streaming and buffered agent-loop turns.
pub(super) struct PreparedLoopIteration {
    pub(super) required_verification_retry_pending: bool,
    pub(super) task_tool_suspended: bool,
    pub(super) file_tool_suspended: bool,
    pub(super) code_tool_suspended: bool,
    pub(super) prompt: String,
    pub(super) active_tool_schemas: Option<crate::tools::schemas::ProviderToolSchemas>,
}

/// Resolved tracked-runtime state plus a guaranteed open-descendant summary.
pub(super) struct ResolvedRuntimeState {
    pub(super) tracked: Option<TrackedTaskRuntimeState>,
    pub(super) open_descendant_summary: OpenDescendantSummary,
}

impl AgentPipeline {
    pub(super) fn initial_loop_response(context: crate::context::ResolvedContext) -> AgentResponse {
        AgentResponse {
            content: String::new(),
            thinking: None,
            tool_calls: Vec::new(),
            usage: None,
            context_used: context,
            truncated: false,
            iterations: 0,
        }
    }

    pub(super) async fn build_request_tool_schemas(
        &self,
        tools: &[&'static ToolDefinition],
        include_mcp_tool_schemas: bool,
    ) -> Option<crate::tools::schemas::ProviderToolSchemas> {
        if tools.is_empty() {
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
            let mut schemas = crate::tools::schemas::build_provider_tool_schemas(tools);
            if include_mcp_tool_schemas {
                let mcp_tools = crate::mcp::get_mcp_client_registry().all_tools().await;
                if !mcp_tools.is_empty() {
                    schemas.merge(crate::tools::schemas::build_mcp_tool_schemas(&mcp_tools));
                }
            }
            Some(schemas)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn prepare_loop_iteration(
        &self,
        iteration: usize,
        max_iterations: Option<usize>,
        current_prompt: &str,
        tool_schemas: &Option<crate::tools::schemas::ProviderToolSchemas>,
        response_tool_calls: &[ToolCallRecord],
        required_verification_retry_pending: bool,
        force_tool_free_final_summary: bool,
        telemetry: &AgentRequestTelemetry,
    ) -> PreparedLoopIteration {
        let task_tool_suspended = Self::should_suspend_task_tool(response_tool_calls);
        let file_tool_suspended = Self::should_suspend_file_tool(response_tool_calls);
        let code_tool_suspended = Self::should_suspend_code_tool(response_tool_calls);

        telemetry
            .record_iteration_start(iteration, max_iterations, task_tool_suspended)
            .await;

        if task_tool_suspended {
            tracing::warn!(
                iteration = iteration,
                "[AgentLoop] Temporarily disabling task tool schema after repeated malformed task bookkeeping calls"
            );
        }
        if file_tool_suspended {
            tracing::warn!(
                iteration = iteration,
                "[AgentLoop] Temporarily disabling file tool schema after repeated malformed file mutation calls"
            );
        }
        if code_tool_suspended {
            tracing::warn!(
                iteration = iteration,
                "[AgentLoop] Temporarily disabling code tool schema after repeated malformed code.batch_edit calls"
            );
        }

        let mut active_tool_schemas = tool_schemas.clone();
        if task_tool_suspended {
            active_tool_schemas = active_tool_schemas
                .as_ref()
                .map(|schemas| Self::without_tool_schema(schemas, "task"));
        }
        if file_tool_suspended {
            active_tool_schemas = active_tool_schemas
                .as_ref()
                .map(|schemas| Self::without_tool_schemas(schemas, &["write_file", "edit_file"]));
        }
        if code_tool_suspended {
            active_tool_schemas = active_tool_schemas.as_ref().map(|schemas| {
                Self::without_tool_schemas(schemas, crate::tools::registry::code_tool_names())
            });
        }
        if required_verification_retry_pending {
            active_tool_schemas = active_tool_schemas
                .as_ref()
                .map(Self::required_verification_retry_schemas);
        }
        if force_tool_free_final_summary {
            active_tool_schemas = None;
        }

        let mut prompt = current_prompt.to_string();
        if task_tool_suspended {
            prompt = Self::with_task_tool_disabled_instruction(&prompt);
        }
        if file_tool_suspended {
            prompt = Self::with_file_tool_disabled_instruction(&prompt);
        }
        if code_tool_suspended {
            prompt = Self::with_code_tool_disabled_instruction(&prompt);
        }
        if required_verification_retry_pending {
            prompt = Self::with_required_verification_retry_instruction(&prompt);
        }

        PreparedLoopIteration {
            required_verification_retry_pending,
            task_tool_suspended,
            file_tool_suspended,
            code_tool_suspended,
            prompt,
            active_tool_schemas,
        }
    }

    pub(super) async fn resolve_runtime_state(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        session_id: Option<&str>,
        task_id: Option<&str>,
        tool_calls: &[ToolCallRecord],
    ) -> ResolvedRuntimeState {
        let tracked = Self::reconcile_tracked_execution_progress_from_tool_activity_async(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            session_id,
            task_id,
            tool_calls,
        )
        .await;

        let open_descendant_summary = if let Some(state) = tracked.as_ref() {
            state.open_descendant_summary
        } else {
            Self::tracked_open_descendant_summary_async(session_id, task_id).await
        };

        ResolvedRuntimeState {
            tracked,
            open_descendant_summary,
        }
    }

    async fn append_response_text(
        response: &mut AgentResponse,
        tx: Option<&mpsc::Sender<StreamChunk>>,
        text: String,
    ) {
        let emitted = if response.content.trim().is_empty() {
            text
        } else {
            format!("\n\n{text}")
        };
        response.content.push_str(&emitted);
        if let Some(tx) = tx {
            let _ = tx.send(StreamChunk::Text(emitted)).await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn finalize_agent_loop_response(
        &self,
        response: &mut AgentResponse,
        saw_any_tool_calls: bool,
        delivered_terminal_summary: bool,
        max_iterations: Option<usize>,
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        session_id: Option<&str>,
        task_id: Option<&str>,
        telemetry: &AgentRequestTelemetry,
        tx: Option<&mpsc::Sender<StreamChunk>>,
    ) {
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
                telemetry
                    .record_synthetic_summary(
                        match reason {
                            IncompleteRunReason::MissingTerminalSummary => {
                                "missing_terminal_summary"
                            }
                            IncompleteRunReason::IterationBudgetExhausted { .. } => {
                                "iteration_budget_exhausted"
                            }
                        },
                        response.tool_calls.len(),
                    )
                    .await;
                Self::append_response_text(response, tx, summary).await;
            }
        }

        let raw_terminal_response = response.content.clone();

        self.reconcile_tracked_task_after_success_with_history_validation(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            session_id,
            task_id,
            &raw_terminal_response,
            &response.tool_calls,
        )
        .await;

        if let Some(correction) = Self::tracked_task_incomplete_terminal_correction_async(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            session_id,
            task_id,
            &response.content,
            &response.tool_calls,
        )
        .await
            && !response.content.contains(&correction)
        {
            Self::append_response_text(response, tx, correction).await;
        }

        if let Some(closeout_note) =
            Self::tracked_task_closeout_note_async(session_id, task_id).await
            && !response.content.contains(&closeout_note)
        {
            Self::append_response_text(response, tx, closeout_note).await;
        }
    }
}
