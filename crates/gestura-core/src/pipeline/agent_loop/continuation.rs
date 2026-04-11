use super::*;
use tokio::sync::mpsc;

impl AgentPipeline {
    #[cfg(test)]
    pub(super) fn build_forced_execution_prompt(
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

        if let (Some(session_id), Some(task_id)) = (session_id, task_id)
            && let Some(runtime_state) =
                Self::reconcile_tracked_execution_progress_from_tool_activity(
                    false,
                    false,
                    Some(session_id),
                    Some(task_id),
                    &[],
                )
        {
            prompt.push('\n');
            prompt.push_str(&Self::format_runtime_snapshot_for_prompt(
                &runtime_state.snapshot,
            ));
            prompt.push('\n');
        }

        prompt.push_str(
            "\nUser: The work is not finished yet. Continue the same run now by executing the runtime-selected current task, or the next ready task if the current one is blocked. Only batch tasks together when the runtime explicitly marks them as parallel-safe. Keep task status aligned with actual execution evidence, not with plans or promises. If you create new work, create a concrete subtask with a specific `name`. Do not mark the root task complete until every planned subtask is completed or explicitly cancelled for a real reason and required verification has actually run. Prioritize implementation, build, and test execution over planning chatter. Do not stop with another task update, plan recap, or promise to resume later unless you are genuinely blocked and explain the blocker clearly.\n",
        );

        prompt
    }

    pub(super) async fn build_forced_execution_prompt_async(
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

        if let Some(runtime_state) =
            Self::reconcile_tracked_execution_progress_from_tool_activity_async(
                false,
                false,
                session_id,
                task_id,
                &[],
            )
            .await
        {
            prompt.push('\n');
            prompt.push_str(&Self::format_runtime_snapshot_for_prompt(
                &runtime_state.snapshot,
            ));
            prompt.push('\n');
        }

        prompt.push_str(
            "\nUser: The work is not finished yet. Continue the same run now by executing the runtime-selected current task, or the next ready task if the current one is blocked. Only batch tasks together when the runtime explicitly marks them as parallel-safe. Keep task status aligned with actual execution evidence, not with plans or promises. If you create new work, create a concrete subtask with a specific `name`. Do not mark the root task complete until every planned subtask is completed or explicitly cancelled for a real reason and required verification has actually run. Prioritize implementation, build, and test execution over planning chatter. Do not stop with another task update, plan recap, or promise to resume later unless you are genuinely blocked and explain the blocker clearly.\n",
        );

        prompt
    }

    pub(super) fn restore_execution_mode_after_forced_summary(
        force_tool_free_final_summary: &mut bool,
        forced_execution_after_empty_response: &mut bool,
        forced_final_summary_requested: &mut bool,
    ) {
        *force_tool_free_final_summary = false;
        *forced_execution_after_empty_response = false;
        *forced_final_summary_requested = false;
    }

    pub(super) fn build_required_verification_prompt(
        &self,
        current_prompt: &str,
        response_so_far: &str,
        tool_calls: &[ToolCallRecord],
    ) -> String {
        let mut prompt = current_prompt.to_string();

        if !response_so_far.trim().is_empty() {
            prompt.push_str(&format!(
                "\nAssistant progress so far:\n{}\n",
                self.truncate_tool_result(response_so_far)
            ));
        }

        let (_, build_completed, _, test_completed) =
            Self::build_and_test_completion_status(tool_calls);
        let build_label = Self::required_build_verification_label(tool_calls);
        let missing = match (build_completed, test_completed) {
            (false, false) => {
                format!("both {build_label} and a successful test command")
            }
            (false, true) => build_label.to_string(),
            (true, false) => "a successful test command".to_string(),
            (true, true) => "no additional verification".to_string(),
        };

        prompt.push_str(&format!(
            "\nUser: You must not finish yet because I explicitly asked you to build and test this project, and this run is still missing {missing}. Continue working now: install dependencies if needed, run the remaining non-interactive verification commands, and only stop after reporting actual build/test results observed in this run. Do not claim readiness without executing the missing verification.\n"
        ));

        if let Some(command) =
            Self::trailing_repeated_successful_verification_command(tool_calls, 2)
        {
            let next_step = match (build_completed, test_completed) {
                (true, false) => {
                    "Run a real test command next and do not rerun the same successful build/check command unchanged."
                }
                (false, true) => {
                    "Run a successful build/check command next using the project’s actual build path, and do not rerun the same successful test command unchanged."
                }
                (false, false) => {
                    "Do not keep rerunning the same successful verification command unchanged; execute one missing verification step now, then the other."
                }
                (true, true) => {
                    "Do not keep rerunning the same successful verification command unchanged."
                }
            };
            prompt.push_str(&format!(
                "Important: this run is looping on the already-successful verification command `{command}`. {next_step}\n"
            ));
        }

        prompt
    }
    #[cfg(test)]
    pub(super) fn build_stalled_mutation_execution_prompt(
        &self,
        current_prompt: &str,
        response_so_far: &str,
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> String {
        let mut prompt = self.build_forced_execution_prompt(
            current_prompt,
            response_so_far,
            session_id,
            task_id,
        );
        prompt.push_str(
            "Important: this run is stuck in read-only inspection and still has not completed a successful file mutation required by the request. Stop rereading scaffold or source files you already inspected. Use the information you already have to make one concrete `edit_file` or `write_file` change next, then continue with any remaining build/test verification. Only do another read if a specific write fails and you need the minimum extra context to unblock that exact change.\n",
        );
        prompt
    }

    pub(super) async fn build_stalled_mutation_execution_prompt_async(
        &self,
        current_prompt: &str,
        response_so_far: &str,
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> String {
        let mut prompt = self
            .build_forced_execution_prompt_async(
                current_prompt,
                response_so_far,
                session_id,
                task_id,
            )
            .await;
        prompt.push_str(
            "Important: this run is stuck in read-only inspection and still has not completed a successful file mutation required by the request. Stop rereading scaffold or source files you already inspected. Use the information you already have to make one concrete `edit_file` or `write_file` change next, then continue with any remaining build/test verification. Only do another read if a specific write fails and you need the minimum extra context to unblock that exact change.\n",
        );
        prompt
    }

    pub(super) async fn join_stream_task_after_channel_close(
        iteration: usize,
        mut stream_handle: tokio::task::JoinHandle<Result<(), AppError>>,
    ) {
        match tokio::time::timeout(STREAM_TASK_JOIN_TIMEOUT, &mut stream_handle).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => {
                tracing::warn!(
                    iteration = iteration,
                    error = %error,
                    "Streaming task finished with an error after the inner channel closed"
                );
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    iteration = iteration,
                    error = %error,
                    "Streaming task join failed after the inner channel closed"
                );
            }
            Err(_) => {
                tracing::warn!(
                    iteration = iteration,
                    timeout_ms = STREAM_TASK_JOIN_TIMEOUT.as_millis(),
                    "Streaming task did not join promptly after the inner channel closed; aborting it to avoid stalling the agent loop"
                );
                stream_handle.abort();
                if let Err(error) = stream_handle.await
                    && !error.is_cancelled()
                {
                    tracing::warn!(
                        iteration = iteration,
                        error = %error,
                        "Streaming task abort join returned an unexpected error"
                    );
                }
            }
        }
    }

    pub(super) async fn forward_status_chunk_best_effort(
        tx: &mpsc::Sender<StreamChunk>,
        chunk: StreamChunk,
    ) {
        debug_assert!(matches!(chunk, StreamChunk::Status { .. }));

        match tokio::time::timeout(STREAM_STATUS_FORWARD_TIMEOUT, tx.send(chunk)).await {
            Ok(Ok(())) | Ok(Err(_)) => {}
            Err(_) => {
                tracing::debug!(
                    timeout_ms = STREAM_STATUS_FORWARD_TIMEOUT.as_millis(),
                    "Dropping transient provider status chunk because the frontend stream receiver is not draining fast enough"
                );
            }
        }
    }

    pub(super) async fn flush_buffered_iteration_text(
        tx: &mpsc::Sender<StreamChunk>,
        response: &mut AgentResponse,
        buffered_text: &mut String,
    ) {
        if buffered_text.is_empty() {
            return;
        }

        let emitted = std::mem::take(buffered_text);
        response.content.push_str(&emitted);
        let _ = tx.send(StreamChunk::Text(emitted)).await;
    }

    pub(super) fn build_no_tool_progress_narration(
        &self,
        snapshot: Option<&crate::streaming::TaskRuntimeSnapshot>,
        suppressed_iteration_text: &str,
    ) -> Option<(
        crate::streaming::NarrationStage,
        crate::streaming::PublicNarration,
        String,
    )> {
        let normalized = suppressed_iteration_text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let message = Self::sanitize_public_narration_text(&normalized)?;
        let context_frame = self.build_results_review_narration_context_frame(
            snapshot
                .map(|state| {
                    Self::narration_stage_for_task_name(
                        state.current_task.as_ref().map(|task| task.name.as_str()),
                        &state.missing_requirements,
                    )
                })
                .unwrap_or(crate::streaming::NarrationStage::Progress),
            snapshot,
            None,
            &[],
        );
        let stage = context_frame.stage;
        let narration = Self::finalize_public_narration(
            stage,
            None,
            PublicNarrationDraft {
                message: Some(message.clone()),
                ..PublicNarrationDraft::default()
            },
            &context_frame,
        )?;
        let fingerprint = format!(
            "no-tool-progress:{}:{}",
            snapshot
                .map(Self::runtime_snapshot_narration_fingerprint)
                .unwrap_or_else(|| "runtime:none".to_string()),
            Self::stable_stagnation_checksum(&message)
        );

        Some((stage, narration, fingerprint))
    }

    pub(super) async fn maybe_emit_no_tool_continuation_narration(
        &self,
        tx: &mpsc::Sender<StreamChunk>,
        snapshot: Option<&crate::streaming::TaskRuntimeSnapshot>,
        narration_state: &mut PublicNarrationState,
        suppressed_iteration_text: &str,
    ) {
        if suppressed_iteration_text.trim().is_empty() {
            return;
        }

        if let Some((stage, narration, fingerprint)) =
            self.build_no_tool_progress_narration(snapshot, suppressed_iteration_text)
        {
            Self::emit_narration_if_changed(tx, stage, narration, fingerprint, narration_state);
            return;
        }

        self.maybe_emit_llm_public_narration(
            tx,
            PublicNarrationTrigger::ResultsReview,
            None,
            None,
            snapshot,
            &[],
            narration_state,
        )
        .await;
    }

    pub(super) fn observed_verification_status_message(
        requires_build_and_test: bool,
        tool_calls: &[ToolCallRecord],
    ) -> String {
        let (_, build_completed, _, test_completed) =
            Self::build_and_test_completion_status(tool_calls);
        let build_label = Self::required_build_verification_label(tool_calls);

        if requires_build_and_test {
            match (build_completed, test_completed) {
                (true, true) => {
                    format!("This run observed both {build_label} and a successful test command.")
                }
                (true, false) => format!(
                    "This run observed {build_label} but did not observe a successful test command."
                ),
                (false, true) => format!(
                    "This run observed a successful test command but did not observe {build_label}."
                ),
                (false, false) => {
                    format!("This run did not observe {build_label} or a successful test command.")
                }
            }
        } else {
            match (build_completed, test_completed) {
                (true, true) => format!(
                    "If you mention verification, limit it to the fact that this run observed both {build_label} and a successful test command."
                ),
                (true, false) => format!(
                    "If you mention verification, limit it to the fact that this run observed {build_label} but not a successful test command."
                ),
                (false, true) => format!(
                    "If you mention verification, limit it to the fact that this run observed a successful test command but not {build_label}."
                ),
                (false, false) => format!(
                    "If you mention verification, say that this run did not observe {build_label} or a successful test command."
                ),
            }
        }
    }

    pub(super) fn open_descendant_summary_message(
        open_descendant_summary: OpenDescendantSummary,
    ) -> Option<String> {
        open_descendant_summary.has_open().then(|| {
            format!(
                "Tracked task bookkeeping still shows open subtasks (not started: {}, in progress: {}, blocked: {}). Do not claim the overall task tree is complete; explicitly mention the remaining tracked work.",
                open_descendant_summary.not_started,
                open_descendant_summary.in_progress,
                open_descendant_summary.blocked,
            )
        })
    }

    pub(super) fn should_request_incomplete_progress_narration(
        open_descendant_summary: OpenDescendantSummary,
        missing_requirements: &[String],
    ) -> bool {
        open_descendant_summary.has_open() || !missing_requirements.is_empty()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_forced_final_summary_prompt(
        &self,
        current_prompt: &str,
        response_so_far: &str,
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        tool_calls: &[ToolCallRecord],
        runtime_missing_requirements: &[String],
        open_descendant_summary: OpenDescendantSummary,
    ) -> String {
        let mut prompt = current_prompt.to_string();
        let mut missing_requirements = Self::runtime_missing_requirements(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            Self::observed_runtime_evidence(tool_calls),
        );
        for requirement in runtime_missing_requirements {
            if !missing_requirements.contains(requirement) {
                missing_requirements.push(requirement.clone());
            }
        }

        if !response_so_far.trim().is_empty() {
            prompt.push_str(&format!(
                "\nAssistant progress so far:\n{}\n",
                self.truncate_tool_result(response_so_far)
            ));
        }

        if Self::should_request_incomplete_progress_narration(
            open_descendant_summary,
            &missing_requirements,
        ) {
            prompt.push_str(
                "\nUser: Before you end this turn, provide a detailed in-progress status narration for the user instead of a success summary. Describe exactly what you accomplished in this run, what work or open checks still remain, and any build/test/verification results you observed. Make it explicit that the overall request is still in progress. Lead with the most concrete outcome or blocker from this run, explain what it means for the user, and then name the next unresolved step. Do not write a field-shaped recap or generic filler. Do not use closing-success wording such as 'completed', 'done', 'finished successfully', or 'ready'. Only call another tool if it is absolutely required to finish the request.\n",
            );
        } else {
            prompt.push_str(
                "\nUser: Before you end this turn, provide a detailed final closeout for the user, not a terse wrap-up. Summarize exactly what you accomplished in this run, the concrete artifacts or files produced or changed, any build/test/research/verification results you observed, and what those results mean for the user. Lead with the most concrete outcome, then cover verification evidence, then name any remaining uncertainty or next step. Do not stop without a direct closing summary. Do not write a field-shaped recap or generic tool log. Only call another tool if it is absolutely required to finish the request.\n",
            );
        }

        prompt.push_str(&format!(
            "Ground your summary strictly in the recorded results from this run. {} ",
            Self::observed_verification_status_message(requires_build_and_test, tool_calls)
        ));

        if requires_build_and_test
            && Self::is_missing_requested_build_and_test(requires_build_and_test, tool_calls)
        {
            prompt.push_str(
                "Do not claim the project is fully verified, ready, or complete because the requested build/test verification is still incomplete in this run. ",
            );
        }

        if !missing_requirements.is_empty() {
            prompt.push_str(&format!(
                "Runtime task bookkeeping still shows missing completion requirements ({}). Do not claim the request is complete until those gaps are explicitly acknowledged or the recorded results actually satisfy them. ",
                missing_requirements.join(", ")
            ));
        }

        if let Some(open_descendant_message) =
            Self::open_descendant_summary_message(open_descendant_summary)
        {
            prompt.push_str(&open_descendant_message);
            prompt.push(' ');
        }

        prompt.push_str(
            "Do not claim any edits, builds, tests, or readiness that are not directly supported by the recorded tool results in this run.\n",
        );

        prompt
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_tool_free_final_summary_prompt(
        &self,
        current_prompt: &str,
        response_so_far: &str,
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        tool_calls: &[ToolCallRecord],
        runtime_missing_requirements: &[String],
        open_descendant_summary: OpenDescendantSummary,
    ) -> String {
        let mut prompt = self.build_forced_final_summary_prompt(
            current_prompt,
            response_so_far,
            requires_build_and_test,
            requires_mutating_file_tool_success,
            tool_calls,
            runtime_missing_requirements,
            open_descendant_summary,
        );
        if Self::should_request_incomplete_progress_narration(
            open_descendant_summary,
            runtime_missing_requirements,
        ) {
            prompt.push_str(
                "\nUser: Tool use is disabled for this summary retry because the run is stuck in a tool loop. Do not call any more tools. Based only on the tool results already observed in this run, provide the best direct in-progress status narration you can for the user now. Make clear that the overall task is not complete yet.\n",
            );
        } else {
            prompt.push_str(
                "\nUser: Tool use is disabled for this final-summary retry because the run is stuck in a tool loop. Do not call any more tools. Based only on the tool results already observed in this run, provide the best direct detailed closing summary you can for the user now.\n",
            );
        }
        prompt
    }

    pub(super) fn build_synthetic_final_summary(
        &self,
        tool_calls: &[ToolCallRecord],
        reason: IncompleteRunReason,
    ) -> Option<String> {
        let tool_calls = Self::synthetic_summary_tool_calls(tool_calls);
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
            IncompleteRunReason::MissingTerminalSummary => {
                "I reached the end of this run without producing a proper wrap-up for the user."
                    .to_string()
            }
            IncompleteRunReason::IterationBudgetExhausted { max_iterations } => format!(
                "I hit the iteration budget limit ({max_iterations}) before I could finish the request cleanly."
            ),
        };

        summary.push(' ');
        summary.push_str(&Self::synthetic_final_status_line(&tool_calls));

        summary.push(' ');
        summary.push_str(&format!(
            "The observed run covered {} tool call(s) ({} succeeded, {} failed, {} skipped).",
            tool_calls.len(),
            success_count,
            error_count,
            skipped_count
        ));

        if let Some(verification_summary) = Self::synthetic_verification_status_line(&tool_calls) {
            summary.push(' ');
            summary.push_str(&verification_summary);
        }

        if let Some(last_call) = tool_calls.last().copied() {
            let last_result = self.describe_tool_call_for_summary(last_call);
            summary.push(' ');
            summary.push_str(&last_result);
        }

        summary.push_str(" Review the recorded tool activity above for the detailed outputs.");

        Some(summary)
    }

    fn synthetic_summary_tool_calls(tool_calls: &[ToolCallRecord]) -> Vec<&ToolCallRecord> {
        let mut seen = std::collections::HashSet::new();
        let mut unique = Vec::new();

        for tool_call in tool_calls {
            if seen.insert(Self::synthetic_summary_tool_call_key(tool_call)) {
                unique.push(tool_call);
            }
        }

        unique
    }

    fn synthetic_summary_tool_call_key(tool_call: &ToolCallRecord) -> String {
        if !tool_call.id.trim().is_empty() {
            return format!("id:{}", tool_call.id);
        }

        let result_key = match &tool_call.result {
            ToolResult::Success(output) => format!("success:{output}"),
            ToolResult::Error(output) => format!("error:{output}"),
            ToolResult::Skipped(output) => format!("skipped:{output}"),
        };

        format!(
            "{}|{}|{}|{}|{}",
            tool_call.name, tool_call.arguments, result_key, tool_call.duration_ms, tool_call.id
        )
    }

    fn synthetic_final_status_line(tool_calls: &[&ToolCallRecord]) -> String {
        if let Some((latest_index, latest_command, latest_success)) =
            Self::synthetic_latest_verification_event(tool_calls)
        {
            return if latest_success {
                if Self::synthetic_previous_failed_verification(tool_calls, latest_index).is_some()
                {
                    "Final status from the observed run: the latest verification finished successfully after earlier failed attempts.".to_string()
                } else {
                    format!(
                        "Final status from the observed run: the latest observed verification command {latest_command} succeeded, but this fallback is standing in for the missing user-facing wrap-up."
                    )
                }
            } else if Self::synthetic_previous_passing_verification(tool_calls, latest_index)
                .is_some()
            {
                "Final status from the observed run: the latest verification ended with a failure or unresolved state after an earlier passing check.".to_string()
            } else {
                "Final status from the observed run: the latest verification ended without a clean success signal.".to_string()
            };
        }

        let relevant_calls = tool_calls
            .iter()
            .copied()
            .filter(|tool_call| !Self::is_task_tool_name(&tool_call.name))
            .collect::<Vec<_>>();
        let relevant_calls = if relevant_calls.is_empty() {
            tool_calls.to_vec()
        } else {
            relevant_calls
        };

        let effective_successes = relevant_calls
            .iter()
            .filter(|tool_call| Self::tool_call_effective_success(tool_call))
            .count();
        let unresolved = relevant_calls.len().saturating_sub(effective_successes);

        match (effective_successes > 0, unresolved > 0) {
            (true, true) => "Final status from the observed run: mixed results — some steps succeeded, but at least one later step failed or stayed unresolved.".to_string(),
            (true, false) => "Final status from the observed run: the recorded steps finished without a visible failure, but this fallback is standing in for the missing user-facing wrap-up.".to_string(),
            (false, true) => "Final status from the observed run: the recorded path ended without a clean completion signal.".to_string(),
            (false, false) => "Final status from the observed run is unavailable.".to_string(),
        }
    }

    fn synthetic_verification_status_line(tool_calls: &[&ToolCallRecord]) -> Option<String> {
        let (latest_index, latest_command, latest_success) =
            Self::synthetic_latest_verification_event(tool_calls)?;

        if latest_success {
            if let Some(previous_failure) =
                Self::synthetic_previous_failed_verification(tool_calls, latest_index)
            {
                Some(format!(
                    "Verification evidence: the latest observed verification command {latest_command} succeeded after earlier failing attempts such as {previous_failure}."
                ))
            } else {
                Some(format!(
                    "Verification evidence: the latest observed verification command {latest_command} succeeded."
                ))
            }
        } else if let Some(previous_success) =
            Self::synthetic_previous_passing_verification(tool_calls, latest_index)
        {
            Some(format!(
                "Verification evidence: the latest observed verification command {latest_command} failed or stayed unresolved after an earlier passing command {previous_success}."
            ))
        } else {
            Some(format!(
                "Verification evidence: the latest observed verification command {latest_command} failed or stayed unresolved."
            ))
        }
    }

    fn synthetic_latest_verification_event(
        tool_calls: &[&ToolCallRecord],
    ) -> Option<(usize, String, bool)> {
        tool_calls
            .iter()
            .enumerate()
            .filter_map(|(index, tool_call)| {
                Self::synthetic_verification_event(tool_call)
                    .map(|(command_label, success)| (index, command_label, success))
            })
            .next_back()
    }

    fn synthetic_previous_failed_verification(
        tool_calls: &[&ToolCallRecord],
        latest_index: usize,
    ) -> Option<String> {
        tool_calls
            .iter()
            .take(latest_index)
            .filter_map(|tool_call| Self::synthetic_verification_event(tool_call))
            .filter_map(|(command_label, success)| (!success).then_some(command_label))
            .next_back()
    }

    fn synthetic_previous_passing_verification(
        tool_calls: &[&ToolCallRecord],
        latest_index: usize,
    ) -> Option<String> {
        tool_calls
            .iter()
            .take(latest_index)
            .filter_map(|tool_call| Self::synthetic_verification_event(tool_call))
            .filter_map(|(command_label, success)| success.then_some(command_label))
            .next_back()
    }

    fn synthetic_verification_event(tool_call: &ToolCallRecord) -> Option<(String, bool)> {
        if tool_call.name != "shell" {
            return None;
        }

        let command = Self::extract_shell_command_from_record_arguments(&tool_call.arguments)?;
        if !Self::is_build_or_check_command(&command)
            && !Self::is_test_command(&command)
            && !Self::is_http_probe_command(&command)
            && !Self::is_launch_verification_command(&command)
        {
            return None;
        }

        Some((
            Self::synthetic_summary_command_label(&command),
            Self::tool_call_effective_success(tool_call),
        ))
    }

    fn synthetic_summary_command_label(command: &str) -> String {
        let normalized = command.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut excerpt = normalized.chars().take(80).collect::<String>();
        if normalized.chars().count() > 80 {
            excerpt.push('…');
        }
        format!("`{excerpt}`")
    }

    pub(super) fn has_iteration_headroom(iteration: usize, max_iterations: Option<usize>) -> bool {
        max_iterations.is_none_or(|limit| iteration + 1 < limit)
    }

    pub(super) fn without_tool_schema(
        schemas: &crate::tools::schemas::ProviderToolSchemas,
        tool_name: &str,
    ) -> crate::tools::schemas::ProviderToolSchemas {
        fn openai_name(value: &serde_json::Value) -> Option<&str> {
            value
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(|name| name.as_str())
        }

        fn named_entry(value: &serde_json::Value) -> Option<&str> {
            value.get("name").and_then(|name| name.as_str())
        }

        let mut filtered = schemas.clone();

        let should_remove = |name: Option<&str>| -> bool {
            let Some(name) = name else {
                return false;
            };
            if name == tool_name {
                return true;
            }
            if tool_name == "task" && Self::is_task_tool_name(name) {
                return true;
            }
            if tool_name == "file" && Self::is_file_tool_name(name) {
                return true;
            }
            if tool_name == "code" && Self::is_code_tool_name(name) {
                return true;
            }
            false
        };

        filtered
            .openai
            .retain(|entry| !should_remove(openai_name(entry)));
        filtered
            .openai_responses
            .retain(|entry| !should_remove(named_entry(entry)));
        filtered
            .anthropic
            .retain(|entry| !should_remove(named_entry(entry)));
        filtered
            .gemini
            .retain(|entry| !should_remove(named_entry(entry)));
        filtered
    }

    pub(super) fn without_tool_schemas(
        schemas: &crate::tools::schemas::ProviderToolSchemas,
        tool_names: &[&str],
    ) -> crate::tools::schemas::ProviderToolSchemas {
        let mut filtered = schemas.clone();
        for tool_name in tool_names {
            filtered = Self::without_tool_schema(&filtered, tool_name);
        }
        filtered
    }

    pub(super) fn required_verification_retry_schemas(
        schemas: &crate::tools::schemas::ProviderToolSchemas,
    ) -> crate::tools::schemas::ProviderToolSchemas {
        let mut disabled_tools = vec!["task", "file", "read_file", "write_file", "edit_file"];
        disabled_tools.extend(crate::tools::registry::code_tool_names().iter().copied());
        Self::without_tool_schemas(schemas, &disabled_tools)
    }

    pub(super) fn should_suspend_task_tool(tool_calls: &[ToolCallRecord]) -> bool {
        const TASK_BOOKKEEPING_SUSPENSION_THRESHOLD: usize = 2;

        let mut malformed_attempts = 0usize;
        for tool_call in tool_calls.iter().rev() {
            if Self::is_task_tool_name(&tool_call.name)
                && matches!(tool_call.result, ToolResult::Success(_))
            {
                break;
            }

            if Self::is_task_tool_name(&tool_call.name)
                && matches!(
                    &tool_call.result,
                    ToolResult::Skipped(message) if message.contains("Loop breaker:")
                )
            {
                return true;
            }

            if Self::has_missing_task_update_status_issue(tool_call)
                || Self::has_missing_task_update_fields_issue(tool_call)
                || Self::has_missing_task_create_name_issue(tool_call)
            {
                malformed_attempts += 1;
                if malformed_attempts >= TASK_BOOKKEEPING_SUSPENSION_THRESHOLD {
                    return true;
                }
            }
        }

        false
    }

    pub(super) fn should_suspend_file_tool(tool_calls: &[ToolCallRecord]) -> bool {
        const FILE_MUTATION_SUSPENSION_THRESHOLD: usize = 4;

        let mut malformed_attempts = 0usize;
        for tool_call in tool_calls.iter().rev() {
            if Self::is_successful_file_mutation(tool_call) {
                break;
            }

            if Self::is_malformed_file_mutation_attempt(tool_call) {
                malformed_attempts += 1;
                if malformed_attempts >= FILE_MUTATION_SUSPENSION_THRESHOLD {
                    return true;
                }
            }
        }

        false
    }

    pub(super) fn should_suspend_code_tool(_tool_calls: &[ToolCallRecord]) -> bool {
        false
    }

    pub(super) fn with_task_tool_disabled_instruction(current_prompt: &str) -> String {
        let mut prompt = current_prompt.to_string();
        prompt.push_str(
            "\nUser: Repeated malformed task bookkeeping calls mean the `task` tool is disabled for the rest of this run. Do not call `task` again. Continue the real implementation, build, or test work with the other available tools instead. If stale tracked subtasks remain open at the end of an otherwise successful run, the runtime will reconcile that bookkeeping automatically.\n",
        );
        prompt
    }

    pub(super) fn with_file_tool_disabled_instruction(current_prompt: &str) -> String {
        let mut prompt = current_prompt.to_string();
        prompt.push_str(
            "\nUser: Repeated malformed file-mutation calls mean `write_file` and `edit_file` are disabled for the rest of this run. Do not call `write_file` or `edit_file` again in this run. The generic `file` tool is only for read/list/tree/search inspection. Continue with other available tools such as `shell` or `code`, or provide a concise user-facing summary if you cannot safely proceed further.\n",
        );
        prompt
    }

    pub(super) fn with_code_tool_disabled_instruction(current_prompt: &str) -> String {
        let mut prompt = current_prompt.to_string();
        prompt.push_str(
            "\nUser: Repeated malformed `code.batch_edit` calls mean the code-tool family is disabled for the rest of this run. Do not call `code` or any `code_*` tool again in this run. Continue with other available tools such as `file` or `shell`, or provide a concise user-facing summary if you cannot safely proceed further.\n",
        );
        prompt
    }

    pub(super) fn with_required_verification_retry_instruction(current_prompt: &str) -> String {
        let mut prompt = current_prompt.to_string();
        prompt.push_str(
            "\nUser: The previous loop stalled before completing the required build/test work. For the next step, do not call `task`, `file`, or `code`; use a concrete non-interactive `shell` command to complete the missing build/test verification now.\n",
        );
        prompt
    }
}
