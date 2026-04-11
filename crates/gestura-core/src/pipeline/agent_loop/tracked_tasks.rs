use super::*;

impl AgentPipeline {
    pub(super) fn apply_tracked_phase_status(
        session_id: &str,
        task_id: &str,
        target_status: crate::TaskStatus,
    ) -> bool {
        let manager = crate::get_global_task_manager();
        match manager.get_task(session_id, task_id) {
            Ok(Some(task))
                if task.status != target_status
                    && task.is_terminal()
                    && !matches!(
                        target_status,
                        crate::TaskStatus::Completed | crate::TaskStatus::Cancelled
                    ) =>
            {
                true
            }
            Ok(Some(task)) if task.status != target_status => manager
                .update_task_status(session_id, task_id, target_status)
                .is_ok(),
            Ok(Some(_)) => true,
            _ => false,
        }
    }

    pub(super) fn reconcile_tracked_execution_progress_from_tool_activity(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        session_id: Option<&str>,
        task_id: Option<&str>,
        tool_calls: &[ToolCallRecord],
    ) -> Option<TrackedTaskRuntimeState> {
        let Some((session_id, root_task_id)) = Self::tracked_task_context(session_id, task_id)
        else {
            return None;
        };

        let manager = crate::get_global_task_manager();
        let evidence = Self::observed_runtime_evidence(tool_calls);
        let load_descendants = || {
            manager.load_task_list(session_id).ok().map(|task_list| {
                let descendants = task_list
                    .descendants(root_task_id)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>();
                (task_list, descendants)
            })
        };

        let Some((_task_list, descendants)) = load_descendants() else {
            return None;
        };

        let open_descendants = descendants
            .iter()
            .filter(|task| !task.is_terminal())
            .cloned()
            .collect::<Vec<_>>();
        let mut open_leaf_tasks = open_descendants
            .iter()
            .filter(|task| {
                !open_descendants
                    .iter()
                    .any(|candidate| candidate.parent_id.as_deref() == Some(task.id.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();

        open_leaf_tasks.sort_by(|left, right| {
            let left_profile = Self::task_execution_profile(left, requires_build_and_test);
            let right_profile = Self::task_execution_profile(right, requires_build_and_test);
            Self::task_priority_bucket(left, &left_profile)
                .cmp(&Self::task_priority_bucket(right, &right_profile))
                .then_with(|| left.sort_order.cmp(&right.sort_order))
                .then_with(|| left.created_at.cmp(&right.created_at))
        });

        let verification_leaf_exists = open_leaf_tasks.iter().any(|task| {
            Self::task_execution_profile(task, requires_build_and_test).execution_kind
                == TaskExecutionKind::Verification
        });

        let current_open_leaf_id = manager
            .get_current_task_id(session_id)
            .ok()
            .flatten()
            .filter(|current_task_id| {
                open_leaf_tasks
                    .iter()
                    .any(|task| task.id == *current_task_id)
            });
        let current_open_leaf_id_for_status = current_open_leaf_id.clone();

        let mut target_ids = Vec::new();
        if let Some(current_task_id) = current_open_leaf_id {
            target_ids.push(current_task_id);
        }

        if target_ids.is_empty()
            && let Some(first_open_leaf) = open_leaf_tasks.first()
        {
            target_ids.push(first_open_leaf.id.clone());
        }

        let current_target = target_ids.first().and_then(|target_id| {
            open_leaf_tasks
                .iter()
                .find(|task| task.id == *target_id)
                .cloned()
        });
        let current_target_execution_kind = current_target
            .as_ref()
            .map(|task| Self::task_execution_profile(task, requires_build_and_test).execution_kind);

        if evidence.successful_source_mutation
            && matches!(
                current_target_execution_kind,
                Some(TaskExecutionKind::Planning | TaskExecutionKind::General)
            )
            && let Some(implementation_task) = open_leaf_tasks.iter().find(|task| {
                !target_ids.iter().any(|target_id| target_id == &task.id)
                    && Self::task_execution_profile(task, requires_build_and_test).execution_kind
                        == TaskExecutionKind::Implementation
            })
        {
            target_ids.push(implementation_task.id.clone());
        }

        if (evidence.build_attempted || evidence.test_attempted)
            && let Some(verification_task) = open_leaf_tasks.iter().find(|task| {
                !target_ids.iter().any(|target_id| target_id == &task.id)
                    && Self::task_execution_profile(task, requires_build_and_test).execution_kind
                        == TaskExecutionKind::Verification
            })
        {
            target_ids.push(verification_task.id.clone());
        }

        for target_id in target_ids {
            let Some(task) = manager.get_task(session_id, &target_id).ok().flatten() else {
                continue;
            };
            if Self::looks_like_placeholder_task_name(&task.name)
                || Self::looks_like_placeholder_task_name(&task.description)
            {
                continue;
            }
            let profile = Self::task_execution_profile(&task, requires_build_and_test);
            let is_current_target = current_open_leaf_id_for_status
                .as_ref()
                .is_some_and(|current_task_id| current_task_id == &task.id);
            let runtime_note = match profile.execution_kind {
                _ if evidence.saw_blocker => {
                    Some("runtime observed a blocker that still needs resolution".to_string())
                }
                _ if evidence.saw_contradiction => {
                    Some("runtime observed a contradiction that still needs resolution".to_string())
                }
                TaskExecutionKind::Planning if evidence.saw_successful_tool_work => {
                    Some("runtime observed planning or inspection progress".to_string())
                }
                TaskExecutionKind::Implementation if evidence.saw_mutation => {
                    Some("runtime observed concrete implementation work".to_string())
                }
                TaskExecutionKind::Verification
                    if evidence.build_attempted || evidence.test_attempted =>
                {
                    Some("runtime observed verification progress".to_string())
                }
                TaskExecutionKind::General if evidence.saw_successful_tool_work => {
                    Some("runtime observed progress for the focused task".to_string())
                }
                _ => None,
            };

            let updated_state = manager
                .update_execution_state(session_id, &task.id, |state| {
                    state.merge_profile(profile.clone());
                    if let Some(note) = runtime_note.clone() {
                        state.last_runtime_note = Some(note);
                    }

                    if evidence.saw_diagnostic_progress {
                        state.record_evidence(TaskExecutionEvidence::new(
                            TaskExecutionEvidenceKind::Diagnostic,
                            "Runtime observed diagnostic progress",
                            None,
                            None,
                        ));
                    }
                    if let Some(summary) = evidence.latest_contradiction_summary.as_ref() {
                        state.record_evidence(TaskExecutionEvidence::new(
                            TaskExecutionEvidenceKind::Contradiction,
                            summary,
                            None,
                            None,
                        ));
                    }
                    if let Some(summary) = evidence.latest_blocker_summary.as_ref() {
                        state.record_evidence(TaskExecutionEvidence::new(
                            TaskExecutionEvidenceKind::Blocker,
                            summary,
                            None,
                            None,
                        ));
                    }

                    match profile.execution_kind {
                        TaskExecutionKind::Planning => {
                            if evidence.saw_successful_tool_work {
                                state.record_evidence(TaskExecutionEvidence::new(
                                    TaskExecutionEvidenceKind::ToolActivity,
                                    "Runtime observed planning or inspection progress",
                                    None,
                                    None,
                                ));
                            }
                        }
                        TaskExecutionKind::Implementation => {
                            if evidence.saw_mutation {
                                state.record_evidence(TaskExecutionEvidence::new(
                                    TaskExecutionEvidenceKind::Mutation,
                                    "Runtime observed successful source mutation",
                                    None,
                                    None,
                                ));
                            }
                        }
                        TaskExecutionKind::Verification => {
                            if evidence.saw_mutation {
                                state.record_evidence(TaskExecutionEvidence::new(
                                    TaskExecutionEvidenceKind::Mutation,
                                    "Runtime observed prerequisite source mutation before verification",
                                    None,
                                    None,
                                ));
                            }
                            if evidence.build_attempted {
                                state.record_evidence(TaskExecutionEvidence::new(
                                    TaskExecutionEvidenceKind::Build,
                                    if evidence.build_completed {
                                        "Runtime observed successful build/check command"
                                    } else {
                                        "Runtime observed attempted build/check command that did not succeed"
                                    },
                                    Some("shell".to_string()),
                                    None,
                                ).with_success(evidence.build_completed));
                            }
                            if evidence.test_attempted {
                                state.record_evidence(TaskExecutionEvidence::new(
                                    TaskExecutionEvidenceKind::Test,
                                    if evidence.test_completed {
                                        "Runtime observed successful test command"
                                    } else {
                                        "Runtime observed attempted test command that did not succeed"
                                    },
                                    Some("shell".to_string()),
                                    None,
                                ).with_success(evidence.test_completed));
                            }
                            if !profile.requires_build
                                && !profile.requires_test
                                && evidence.saw_generic_verification_progress
                            {
                                state.record_evidence(TaskExecutionEvidence::new(
                                    TaskExecutionEvidenceKind::ToolActivity,
                                    "Runtime observed generic verification progress",
                                    Self::latest_generic_verification_tool_name(tool_calls),
                                    None,
                                ));
                            }
                            if profile.requires_launch_evidence
                                && let Some(command) =
                                    Self::latest_successful_launch_verification_command(tool_calls)
                            {
                                state.record_evidence(TaskExecutionEvidence::new(
                                    TaskExecutionEvidenceKind::ToolActivity,
                                    "Runtime observed direct launch verification",
                                    Some("shell".to_string()),
                                    Some(command),
                                ));
                            }
                        }
                        TaskExecutionKind::General => {
                            if evidence.saw_successful_tool_work {
                                state.record_evidence(TaskExecutionEvidence::new(
                                    TaskExecutionEvidenceKind::ToolActivity,
                                    "Runtime observed successful tool activity",
                                    None,
                                    None,
                                ));
                            }
                        }
                    }
                })
                .ok();

            let Some(updated_state) = updated_state else {
                continue;
            };

            let stronger_phase_handoff_observed = evidence.saw_mutation
                || updated_state.saw_mutation
                || updated_state.build_succeeded
                || updated_state.test_succeeded
                || evidence.build_completed
                || evidence.test_completed;
            let blocker_observed = evidence.saw_blocker;
            let contradiction_observed = evidence.saw_contradiction;
            let unresolved_negative_evidence = blocker_observed || contradiction_observed;

            let target_status = match profile.execution_kind {
                TaskExecutionKind::Planning if updated_state.saw_tool_activity => {
                    if blocker_observed {
                        Some(crate::TaskStatus::Blocked)
                    } else if stronger_phase_handoff_observed
                        && updated_state.satisfies_profile()
                        && !unresolved_negative_evidence
                    {
                        Some(crate::TaskStatus::Completed)
                    } else {
                        Some(crate::TaskStatus::InProgress)
                    }
                }
                TaskExecutionKind::Implementation if updated_state.saw_mutation => {
                    if blocker_observed {
                        Some(crate::TaskStatus::Blocked)
                    } else if (is_current_target && updated_state.satisfies_profile())
                        || evidence.build_completed
                        || evidence.test_completed
                        || !verification_leaf_exists
                    {
                        Some(crate::TaskStatus::Completed)
                    } else {
                        Some(crate::TaskStatus::InProgress)
                    }
                }
                TaskExecutionKind::Verification
                    if updated_state.build_succeeded || updated_state.test_succeeded =>
                {
                    if blocker_observed {
                        Some(crate::TaskStatus::Blocked)
                    } else if updated_state.satisfies_profile() && !unresolved_negative_evidence {
                        Some(crate::TaskStatus::Completed)
                    } else {
                        Some(crate::TaskStatus::InProgress)
                    }
                }
                TaskExecutionKind::Verification if updated_state.saw_tool_activity => {
                    if blocker_observed {
                        Some(crate::TaskStatus::Blocked)
                    } else if updated_state.satisfies_profile() && !unresolved_negative_evidence {
                        Some(crate::TaskStatus::Completed)
                    } else {
                        Some(crate::TaskStatus::InProgress)
                    }
                }
                TaskExecutionKind::General if updated_state.saw_tool_activity => {
                    if blocker_observed {
                        Some(crate::TaskStatus::Blocked)
                    } else if stronger_phase_handoff_observed
                        && updated_state.satisfies_profile()
                        && !unresolved_negative_evidence
                    {
                        Some(crate::TaskStatus::Completed)
                    } else {
                        Some(crate::TaskStatus::InProgress)
                    }
                }
                _ => None,
            };

            if let Some(target_status) = target_status {
                let _ = Self::apply_tracked_phase_status(session_id, &task.id, target_status);
            }
        }

        loop {
            let Some((_, descendants)) = load_descendants() else {
                break;
            };
            let open_descendants = descendants
                .iter()
                .filter(|task| !task.is_terminal())
                .cloned()
                .collect::<Vec<_>>();
            let mut progressed = false;

            for task in open_descendants.iter().rev() {
                let has_open_child = open_descendants
                    .iter()
                    .any(|candidate| candidate.parent_id.as_deref() == Some(task.id.as_str()));
                if has_open_child {
                    continue;
                }

                let has_descendants = descendants
                    .iter()
                    .any(|candidate| candidate.parent_id.as_deref() == Some(task.id.as_str()));
                if !has_descendants {
                    continue;
                }

                if manager
                    .update_task_status(session_id, &task.id, crate::TaskStatus::Completed)
                    .is_ok()
                {
                    progressed = true;
                }
            }

            if !progressed {
                break;
            }
        }

        let (task_list, descendants) = load_descendants()?;
        let open_descendants = descendants
            .iter()
            .filter(|task| !task.is_terminal())
            .cloned()
            .collect::<Vec<_>>();
        let open_descendant_summary = OpenDescendantSummary::from_tasks(&open_descendants);
        let mut open_leaf_tasks = open_descendants
            .iter()
            .filter(|task| {
                !open_descendants
                    .iter()
                    .any(|candidate| candidate.parent_id.as_deref() == Some(task.id.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        open_leaf_tasks.sort_by(|left, right| {
            let left_profile = Self::task_execution_profile(left, requires_build_and_test);
            let right_profile = Self::task_execution_profile(right, requires_build_and_test);
            Self::task_priority_bucket(left, &left_profile)
                .cmp(&Self::task_priority_bucket(right, &right_profile))
                .then_with(|| left.sort_order.cmp(&right.sort_order))
                .then_with(|| left.created_at.cmp(&right.created_at))
        });

        let mut ready_tasks = Vec::new();
        let mut blocked_tasks = Vec::new();
        for task in &open_leaf_tasks {
            match task_list.is_task_blocked(&task.id) {
                Ok(true) => blocked_tasks.push(task.clone()),
                Ok(false) => ready_tasks.push(task.clone()),
                Err(_) => blocked_tasks.push(task.clone()),
            }
        }

        let root_task = manager.get_task(session_id, root_task_id).ok().flatten();
        let root_already_completed = root_task
            .as_ref()
            .is_some_and(|task| task.status == crate::TaskStatus::Completed);

        let mut missing_requirements = Self::runtime_missing_requirements(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            evidence,
        );
        if root_already_completed && !open_descendant_summary.has_open() {
            missing_requirements.clear();
        }

        let completion_candidate =
            !open_descendant_summary.has_open() && missing_requirements.is_empty();
        let root_completion_error = if completion_candidate {
            match root_task {
                Some(task) if task.status == crate::TaskStatus::Completed => None,
                Some(_) => manager
                    .update_task_status(session_id, root_task_id, crate::TaskStatus::Completed)
                    .err()
                    .map(|error| format!("root task completion is still blocked: {error}")),
                None => Some("root task is no longer present in the task list".to_string()),
            }
        } else {
            None
        };
        if let Some(error) = root_completion_error.as_ref() {
            missing_requirements.push(error.clone());
        }
        let completion_ready = completion_candidate && root_completion_error.is_none();

        let mut current_task =
            current_open_leaf_id_for_status
                .as_ref()
                .and_then(|current_task_id| {
                    ready_tasks
                        .iter()
                        .find(|task| task.id == *current_task_id)
                        .cloned()
                });
        if current_task.is_none() {
            current_task = ready_tasks.first().cloned();
        }
        if current_task.is_none()
            && !completion_ready
            && open_descendant_summary.total() == 0
            && !missing_requirements.is_empty()
        {
            current_task = manager
                .get_task(session_id, root_task_id)
                .ok()
                .flatten()
                .filter(|task| !task.is_terminal());
        }

        let parallel_ready_tasks = if current_task.as_ref().is_some_and(|task| {
            Self::task_execution_profile(task, requires_build_and_test).parallel_safe
        }) {
            ready_tasks
                .iter()
                .filter(|task| {
                    Self::task_execution_profile(task, requires_build_and_test).parallel_safe
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        if completion_ready {
            let _ = manager.set_current_task_id(session_id, None);
        } else {
            if !root_already_completed {
                let _ = Self::apply_tracked_phase_status(
                    session_id,
                    root_task_id,
                    crate::TaskStatus::InProgress,
                );
            }
            let _ = manager.set_current_task_id(
                session_id,
                current_task.as_ref().map(|task| task.id.clone()),
            );
        }

        let snapshot = crate::streaming::TaskRuntimeSnapshot {
            root_task_id: root_task_id.to_string(),
            current_task: current_task.as_ref().map(Self::task_runtime_view),
            ready_tasks: ready_tasks.iter().map(Self::task_runtime_view).collect(),
            parallel_ready_tasks: parallel_ready_tasks
                .iter()
                .map(Self::task_runtime_view)
                .collect(),
            blocked_tasks: blocked_tasks.iter().map(Self::task_runtime_view).collect(),
            open_tasks: open_descendants
                .iter()
                .map(Self::task_runtime_view)
                .collect(),
            completed_tasks: descendants
                .iter()
                .filter(|task| task.status == crate::TaskStatus::Completed)
                .map(Self::task_runtime_view)
                .collect(),
            missing_requirements: missing_requirements.clone(),
            status_message: Self::runtime_snapshot_status_message(
                current_task.as_ref(),
                &ready_tasks,
                &parallel_ready_tasks,
                &missing_requirements,
            ),
        };

        Some(TrackedTaskRuntimeState {
            snapshot,
            open_descendant_summary,
            completion_ready,
        })
    }

    pub(super) fn tracked_task_context<'a>(
        session_id: Option<&'a str>,
        task_id: Option<&'a str>,
    ) -> Option<(&'a str, &'a str)> {
        let session_id = session_id?.trim();
        let task_id = task_id?.trim();
        if session_id.is_empty() || task_id.is_empty() {
            return None;
        }
        Some((session_id, task_id))
    }

    pub(super) fn record_tracked_task_memory_event(
        session_id: Option<&str>,
        task_id: Option<&str>,
        phase: crate::tasks::TaskMemoryPhase,
        summary: impl Into<String>,
        scope: Option<String>,
        memory_type: Option<String>,
        memory_file_path: Option<String>,
    ) {
        let Some((session_id, task_id)) = Self::tracked_task_context(session_id, task_id) else {
            return;
        };

        let summary = summary.into();
        let scope_for_compare = scope.clone();
        let memory_type_for_compare = memory_type.clone();
        let memory_file_path_for_compare = memory_file_path.clone();
        let manager = crate::get_global_task_manager();
        let should_record = manager
            .get_memory_lifecycle(session_id, task_id)
            .ok()
            .flatten()
            .and_then(|lifecycle| lifecycle.events.last().cloned())
            .map(|last_event| {
                !(last_event.phase == phase
                    && last_event.summary == summary
                    && last_event.scope == scope_for_compare
                    && last_event.memory_type == memory_type_for_compare
                    && last_event.memory_file_path == memory_file_path_for_compare)
            })
            .unwrap_or(true);

        if !should_record {
            return;
        }

        if let Err(error) = manager.record_memory_event(
            session_id,
            task_id,
            crate::tasks::TaskMemoryEvent::new(
                phase,
                summary,
                scope,
                memory_type,
                memory_file_path,
            ),
        ) {
            tracing::warn!(
                session_id = %session_id,
                task_id = %task_id,
                error = %error,
                "Failed to record tracked task memory event"
            );
        }
    }

    pub(super) fn tracked_task_incomplete_memory_summary(
        state: &TrackedTaskRuntimeState,
    ) -> String {
        let mut details = Vec::new();
        if !state.snapshot.missing_requirements.is_empty() {
            details.push(format!(
                "missing runtime requirements: {}",
                state.snapshot.missing_requirements.join(", ")
            ));
        }
        if state.open_descendant_summary.has_open() {
            details.push(format!(
                "open subtasks remain (not started: {}, in progress: {}, blocked: {})",
                state.open_descendant_summary.not_started,
                state.open_descendant_summary.in_progress,
                state.open_descendant_summary.blocked,
            ));
        }

        if details.is_empty() {
            "Tracked work remains incomplete after the closing summary attempt.".to_string()
        } else {
            format!(
                "Tracked work remains incomplete after the closing summary attempt: {}.",
                details.join("; ")
            )
        }
    }

    pub(super) fn tracked_task_incomplete_terminal_correction(
        final_response: &str,
        state: &TrackedTaskRuntimeState,
    ) -> Option<String> {
        if state.completion_ready
            || !Self::has_meaningful_final_text(final_response)
            || Self::text_defers_remaining_work(final_response)
            || Self::text_signals_failed_or_incomplete_work(final_response)
            || (!state.open_descendant_summary.has_open()
                && state.snapshot.missing_requirements.is_empty())
        {
            return None;
        }

        let mut correction = String::from("I’m not calling this work complete yet.");

        if let Some(current_task) = state.snapshot.current_task.as_ref() {
            correction.push(' ');
            correction.push_str(&format!(
                "The active tracked step is {} [{}].",
                current_task.name, current_task.status
            ));
        }

        if !state.snapshot.missing_requirements.is_empty() {
            correction.push(' ');
            correction.push_str(&format!(
                "I still need direct proof for: {}.",
                state.snapshot.missing_requirements.join(", ")
            ));
        }

        if state.open_descendant_summary.has_open() {
            correction.push(' ');
            correction.push_str(&format!(
                "There is still queued tracked work (not started: {}, in progress: {}, blocked: {}).",
                state.open_descendant_summary.not_started,
                state.open_descendant_summary.in_progress,
                state.open_descendant_summary.blocked,
            ));
        }

        if let Some(summary) = Self::summarize_runtime_task_views(&state.snapshot.ready_tasks, 2) {
            correction.push(' ');
            correction.push_str(&format!("The next ready step is {}.", summary));
        } else if let Some(summary) =
            Self::summarize_runtime_task_views(&state.snapshot.parallel_ready_tasks, 2)
        {
            correction.push(' ');
            correction.push_str(&format!("Parallel-ready work still exists: {}.", summary));
        } else if let Some(summary) =
            Self::summarize_runtime_task_views(&state.snapshot.blocked_tasks, 2)
        {
            correction.push(' ');
            correction.push_str(&format!("Blocked work still needs attention: {}.", summary));
        }

        Some(correction)
    }

    pub(super) fn record_tracked_task_incomplete_memory_event(
        session_id: Option<&str>,
        task_id: Option<&str>,
        state: &TrackedTaskRuntimeState,
    ) {
        Self::record_tracked_task_memory_event(
            session_id,
            task_id,
            crate::tasks::TaskMemoryPhase::Blocked,
            Self::tracked_task_incomplete_memory_summary(state),
            Some("session".to_string()),
            Some("blocker".to_string()),
            None,
        );
    }

    #[allow(dead_code)]
    pub(super) fn highest_priority_open_descendant(
        session_id: &str,
        task_id: &str,
    ) -> Option<crate::Task> {
        crate::get_global_task_manager()
            .list_descendants(session_id, task_id)
            .ok()?
            .into_iter()
            .find(|descendant| !descendant.is_terminal())
    }

    #[allow(dead_code)]
    pub(super) fn sync_current_task_focus_to_highest_priority_open_descendant(
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) {
        let Some((session_id, task_id)) = Self::tracked_task_context(session_id, task_id) else {
            return;
        };

        let Some(next_task) = Self::highest_priority_open_descendant(session_id, task_id) else {
            return;
        };

        if let Err(error) = crate::get_global_task_manager()
            .set_current_task_id(session_id, Some(next_task.id.clone()))
        {
            tracing::warn!(
                session_id = %session_id,
                task_id = %task_id,
                next_task_id = %next_task.id,
                error = %error,
                "Failed to focus highest-priority open descendant before forced execution"
            );
        }
    }

    pub(super) async fn run_blocking_task_bookkeeping<T, F>(label: &'static str, op: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        match tokio::task::spawn_blocking(op).await {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(task = label, error = %error, "Task bookkeeping join failed");
                None
            }
        }
    }

    pub(super) async fn tracked_task_closeout_note_async(
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> Option<String> {
        let session_id = session_id.map(str::to_string);
        let task_id = task_id.map(str::to_string);
        Self::run_blocking_task_bookkeeping("tracked_task_closeout_note", move || {
            Self::tracked_task_closeout_note(session_id.as_deref(), task_id.as_deref())
        })
        .await
        .flatten()
    }

    pub(super) async fn tracked_task_incomplete_terminal_correction_async(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        session_id: Option<&str>,
        task_id: Option<&str>,
        final_response: &str,
        tool_calls: &[ToolCallRecord],
    ) -> Option<String> {
        let state = Self::reconcile_tracked_execution_progress_from_tool_activity_async(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            session_id,
            task_id,
            tool_calls,
        )
        .await?;

        Self::tracked_task_incomplete_terminal_correction(final_response, &state)
    }

    pub(super) async fn mark_tracked_task_in_progress_async(
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) {
        let session_id = session_id.map(str::to_string);
        let task_id = task_id.map(str::to_string);
        let _ = Self::run_blocking_task_bookkeeping("mark_tracked_task_in_progress", move || {
            Self::mark_tracked_task_in_progress(session_id.as_deref(), task_id.as_deref())
        })
        .await;
    }

    pub(super) async fn reconcile_tracked_execution_progress_from_tool_activity_async(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        session_id: Option<&str>,
        task_id: Option<&str>,
        tool_calls: &[ToolCallRecord],
    ) -> Option<TrackedTaskRuntimeState> {
        let session_id = session_id.map(str::to_string);
        let task_id = task_id.map(str::to_string);
        let tool_calls = tool_calls.to_vec();
        Self::run_blocking_task_bookkeeping(
            "reconcile_tracked_execution_progress_from_tool_activity",
            move || {
                Self::reconcile_tracked_execution_progress_from_tool_activity(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    session_id.as_deref(),
                    task_id.as_deref(),
                    &tool_calls,
                )
            },
        )
        .await
        .flatten()
    }

    pub(super) async fn cancel_tracked_task_async(
        session_id: Option<&str>,
        task_id: Option<&str>,
        reason: &str,
    ) {
        let session_id = session_id.map(str::to_string);
        let task_id = task_id.map(str::to_string);
        let reason = reason.to_string();
        let _ = Self::run_blocking_task_bookkeeping("cancel_tracked_task", move || {
            Self::cancel_tracked_task(session_id.as_deref(), task_id.as_deref(), &reason)
        })
        .await;
    }

    pub(super) async fn tracked_open_descendant_summary_async(
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> OpenDescendantSummary {
        let session_id = session_id.map(str::to_string);
        let task_id = task_id.map(str::to_string);
        Self::run_blocking_task_bookkeeping("tracked_open_descendant_summary", move || {
            Self::tracked_open_descendant_summary(session_id.as_deref(), task_id.as_deref())
        })
        .await
        .unwrap_or_default()
    }

    pub(super) fn tracked_task_closeout_note(
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> Option<String> {
        let (Some(session_id), Some(task_id)) = (session_id, task_id) else {
            return None;
        };

        let manager = crate::get_global_task_manager();
        let root_task = manager.get_task(session_id, task_id).ok().flatten()?;
        let open_descendants = manager
            .list_descendants(session_id, task_id)
            .ok()?
            .into_iter()
            .filter(|task| !task.is_terminal())
            .collect::<Vec<_>>();

        if root_task.status == crate::TaskStatus::Completed && open_descendants.is_empty() {
            return Some(
                "From the tracked task state, everything is closed out now: every subtask is terminal and the overall task is complete."
                    .to_string(),
            );
        }

        open_descendants.first().map(|next_task| {
            format!(
                "From the tracked task state, the overall request is still {}. The highest-priority incomplete subtask is {} [{}].",
                root_task.status,
                next_task.name,
                next_task.status,
            )
        })
        .or_else(|| {
            Some(format!(
                "From the tracked task state, the overall request is still {}.",
                root_task.status
            ))
        })
    }

    pub(super) fn mark_tracked_task_in_progress(session_id: Option<&str>, task_id: Option<&str>) {
        let Some((session_id, task_id)) = Self::tracked_task_context(session_id, task_id) else {
            return;
        };

        let manager = crate::get_global_task_manager();
        match manager.get_task(session_id, task_id) {
            Ok(Some(task)) => {
                if !task.is_terminal()
                    && task.status != crate::TaskStatus::InProgress
                    && let Err(error) = manager.update_task_status(
                        session_id,
                        task_id,
                        crate::TaskStatus::InProgress,
                    )
                {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        error = %error,
                        "Failed to mark tracked task in progress"
                    );
                }

                let preserve_current_descendant = manager
                    .get_current_task_id(session_id)
                    .ok()
                    .flatten()
                    .filter(|current_task_id| current_task_id != task_id)
                    .and_then(|current_task_id| {
                        manager
                            .list_descendants(session_id, task_id)
                            .ok()
                            .and_then(|descendants| {
                                descendants.into_iter().find(|descendant| {
                                    descendant.id == current_task_id && !descendant.is_terminal()
                                })
                            })
                    })
                    .is_some();

                if !preserve_current_descendant
                    && let Err(error) =
                        manager.set_current_task_id(session_id, Some(task_id.to_string()))
                {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        error = %error,
                        "Failed to set current tracked task"
                    );
                }
            }
            Ok(None) => {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    "Tracked task was not found when attempting to mark it in progress"
                );
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    error = %error,
                    "Failed to load tracked task before marking it in progress"
                );
            }
        }
    }

    #[allow(dead_code)]
    pub(super) fn should_cleanup_stale_open_descendants_after_success(
        final_response: &str,
        tool_calls: &[ToolCallRecord],
        open_descendant_summary: OpenDescendantSummary,
    ) -> bool {
        Self::has_meaningful_final_text(final_response)
            && Self::text_signals_completed_work(final_response)
            && !Self::text_signals_user_blocker_or_question(final_response)
            && !Self::text_defers_remaining_work(final_response)
            && (Self::should_suspend_task_tool(tool_calls)
                || open_descendant_summary.only_not_started()
                || open_descendant_summary.in_progress > 0)
    }

    #[allow(dead_code)]
    pub(super) fn response_matching_tokens(raw: &str) -> Vec<String> {
        raw.chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .filter(|token| {
                token.len() >= 3
                    && !matches!(
                        *token,
                        "the"
                            | "and"
                            | "for"
                            | "with"
                            | "that"
                            | "this"
                            | "from"
                            | "into"
                            | "then"
                            | "task"
                            | "tasks"
                            | "step"
                            | "steps"
                            | "requested"
                            | "complete"
                            | "completed"
                            | "verified"
                            | "final"
                            | "result"
                    )
            })
            .map(str::to_string)
            .collect()
    }

    #[allow(dead_code)]
    pub(super) fn final_response_mentions_task(task: &crate::Task, final_response: &str) -> bool {
        let response_tokens = Self::response_matching_tokens(final_response)
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        if response_tokens.is_empty() {
            return false;
        }

        let mut task_tokens = Self::response_matching_tokens(&task.name);
        task_tokens.extend(Self::response_matching_tokens(&task.description));
        task_tokens.sort();
        task_tokens.dedup();

        if task_tokens.is_empty() {
            return false;
        }

        let matched = task_tokens
            .iter()
            .filter(|token| response_tokens.contains(*token))
            .count();

        matched >= 2 || (matched == 1 && task_tokens.len() == 1)
    }

    pub(super) fn task_text_contains_any(task: &crate::Task, keywords: &[&str]) -> bool {
        let name = task.name.to_ascii_lowercase();
        let description = task.description.to_ascii_lowercase();

        keywords
            .iter()
            .any(|keyword| name.contains(keyword) || description.contains(keyword))
    }

    #[allow(dead_code)]
    pub(super) fn target_status_for_open_descendant_after_success(
        session_id: &str,
        task: &crate::Task,
        final_response: &str,
        tool_calls: &[ToolCallRecord],
    ) -> Option<crate::TaskStatus> {
        let manager = crate::get_global_task_manager();
        let is_placeholder = Self::looks_like_placeholder_task_name(&task.name)
            || Self::looks_like_placeholder_task_name(&task.description);
        if is_placeholder {
            return Some(crate::TaskStatus::Cancelled);
        }

        if Self::task_requires_user_facing_closeout(task)
            && !Self::final_response_mentions_task(task, final_response)
        {
            return None;
        }

        let inferred_profile = Self::task_execution_profile(task, false);

        if let Ok(Some(mut execution_state)) = manager.get_execution_state(session_id, &task.id) {
            let mut required_profile = execution_state.verification_profile.clone();
            required_profile.requires_mutation |= inferred_profile.requires_mutation;
            required_profile.requires_build |= inferred_profile.requires_build;
            required_profile.requires_test |= inferred_profile.requires_test;
            required_profile.requires_external_evidence |=
                inferred_profile.requires_external_evidence;
            required_profile.requires_launch_evidence |= inferred_profile.requires_launch_evidence;
            execution_state.merge_profile(required_profile);
            match task.status {
                crate::TaskStatus::InProgress | crate::TaskStatus::NotStarted
                    if execution_state.satisfies_profile() =>
                {
                    return Some(crate::TaskStatus::Completed);
                }
                crate::TaskStatus::Blocked
                | crate::TaskStatus::Completed
                | crate::TaskStatus::Cancelled => return None,
                crate::TaskStatus::InProgress | crate::TaskStatus::NotStarted => {}
            }
        }

        match task.status {
            crate::TaskStatus::InProgress => match inferred_profile.execution_kind {
                TaskExecutionKind::Planning | TaskExecutionKind::General => {
                    Some(crate::TaskStatus::Completed)
                }
                TaskExecutionKind::Implementation | TaskExecutionKind::Verification => None,
            },
            crate::TaskStatus::NotStarted => {
                let (_, build_completed, _, test_completed) =
                    Self::build_and_test_completion_status(tool_calls);
                let matches_build_task = Self::task_mentions_build_verification(task);
                let matches_test_task = Self::task_mentions_test_verification(task);

                if matches_build_task || matches_test_task {
                    let build_ok = !matches_build_task || build_completed;
                    let test_ok = !matches_test_task || test_completed;
                    (build_ok && test_ok).then_some(crate::TaskStatus::Completed)
                } else if inferred_profile.execution_kind == TaskExecutionKind::Verification
                    && Self::generic_verification_satisfies_task(task, tool_calls)
                {
                    Some(crate::TaskStatus::Completed)
                } else {
                    let _ = final_response;
                    let _ = inferred_profile;
                    None
                }
            }
            crate::TaskStatus::Blocked
            | crate::TaskStatus::Completed
            | crate::TaskStatus::Cancelled => None,
        }
    }

    #[allow(dead_code)]
    pub(super) fn reconcile_open_descendants_after_success(
        session_id: &str,
        task_id: &str,
        final_response: &str,
        tool_calls: &[ToolCallRecord],
    ) {
        let manager = crate::get_global_task_manager();

        loop {
            let open_descendants = match manager.list_descendants(session_id, task_id) {
                Ok(tasks) => tasks
                    .into_iter()
                    .filter(|task| !task.is_terminal())
                    .collect::<Vec<_>>(),
                Err(error) => {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        error = %error,
                        "Failed to inspect tracked task descendants during success reconciliation"
                    );
                    return;
                }
            };

            if open_descendants.is_empty() {
                return;
            }

            let actions = open_descendants
                .iter()
                .filter(|descendant| {
                    !open_descendants.iter().any(|candidate| {
                        candidate.parent_id.as_deref() == Some(descendant.id.as_str())
                    })
                })
                .filter_map(|descendant| {
                    Self::target_status_for_open_descendant_after_success(
                        session_id,
                        descendant,
                        final_response,
                        tool_calls,
                    )
                    .map(|status| (descendant.id.clone(), status))
                })
                .collect::<Vec<_>>();

            if actions.is_empty() {
                return;
            }

            let mut made_progress = false;
            for (descendant_id, status) in actions {
                match manager.update_task_status(session_id, &descendant_id, status) {
                    Ok(_) => {
                        made_progress = true;
                    }
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session_id,
                            task_id = %task_id,
                            descendant_id = %descendant_id,
                            target_status = ?status,
                            error = %error,
                            "Failed to reconcile tracked subtask after successful agent run"
                        );
                    }
                }
            }

            if !made_progress {
                return;
            }
        }
    }

    pub(super) fn cancel_open_descendants(
        session_id: &str,
        task_id: &str,
        reason: &str,
    ) -> Vec<String> {
        let manager = crate::get_global_task_manager();
        let mut cancelled_descendants = Vec::new();

        loop {
            let open_descendants = match manager.list_descendants(session_id, task_id) {
                Ok(tasks) => tasks
                    .into_iter()
                    .filter(|task| !task.is_terminal())
                    .collect::<Vec<_>>(),
                Err(error) => {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        error = %error,
                        cancellation_reason = reason,
                        "Failed to inspect tracked task descendants during terminal reconciliation"
                    );
                    return cancelled_descendants;
                }
            };

            if open_descendants.is_empty() {
                return cancelled_descendants;
            }

            let leaf_descendants = open_descendants
                .iter()
                .filter(|descendant| {
                    !open_descendants.iter().any(|candidate| {
                        candidate.parent_id.as_deref() == Some(descendant.id.as_str())
                    })
                })
                .map(|descendant| (descendant.id.clone(), descendant.name.clone()))
                .collect::<Vec<_>>();

            if leaf_descendants.is_empty() {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    cancellation_reason = reason,
                    open_descendants = open_descendants.len(),
                    "Tracked task descendants could not be reduced to leaves during terminal reconciliation"
                );
                return cancelled_descendants;
            }

            let mut made_progress = false;
            for (descendant_id, descendant_name) in leaf_descendants {
                match manager.update_task_status(
                    session_id,
                    &descendant_id,
                    crate::TaskStatus::Cancelled,
                ) {
                    Ok(_) => {
                        made_progress = true;
                        cancelled_descendants.push(descendant_name);
                    }
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session_id,
                            task_id = %task_id,
                            descendant_id = %descendant_id,
                            error = %error,
                            cancellation_reason = reason,
                            "Failed to cancel tracked descendant during terminal reconciliation"
                        );
                    }
                }
            }

            if !made_progress {
                return cancelled_descendants;
            }
        }
    }

    #[allow(dead_code)]
    pub(super) fn final_response_signals_successful_completion(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        final_response: &str,
        tool_calls: &[ToolCallRecord],
    ) -> bool {
        Self::has_meaningful_final_text(final_response)
            && !Self::text_signals_user_blocker_or_question(final_response)
            && !Self::text_signals_failed_or_incomplete_work(final_response)
            && !Self::text_defers_remaining_work(final_response)
            && !Self::is_missing_requested_build_and_test(requires_build_and_test, tool_calls)
            && Self::tool_results_support_successful_completion(
                requires_mutating_file_tool_success,
                tool_calls,
            )
    }

    pub(super) fn tool_results_support_successful_completion(
        requires_mutating_file_tool_success: bool,
        tool_calls: &[ToolCallRecord],
    ) -> bool {
        let last_non_task_supports_completion = tool_calls
            .iter()
            .rev()
            .find(|tool_call| !Self::is_task_tool_name(&tool_call.name))
            .map(|tool_call| {
                Self::tool_call_effective_success(tool_call)
                    && Self::tool_call_contradiction_summary(tool_call).is_none()
                    && Self::tool_call_blocker_summary(tool_call).is_none()
            })
            .unwrap_or(!requires_mutating_file_tool_success);

        if !last_non_task_supports_completion {
            return false;
        }

        if !requires_mutating_file_tool_success {
            return true;
        }

        if Self::has_unresolved_source_mutation_failure(tool_calls) {
            return false;
        }

        let has_successful_source_mutation = tool_calls.iter().any(|tool_call| {
            Self::is_successful_mutating_file_tool_call(tool_call)
                || Self::is_successful_mutating_code_tool_call(tool_call)
        });
        let attempted_source_mutation = tool_calls.iter().any(|tool_call| {
            Self::is_file_mutation_attempt(tool_call) || Self::is_code_mutation_attempt(tool_call)
        });

        if attempted_source_mutation {
            return has_successful_source_mutation;
        }

        has_successful_source_mutation
            || tool_calls
                .iter()
                .any(Self::is_successful_mutating_shell_tool_call)
    }

    pub(super) fn has_unresolved_source_mutation_failure(tool_calls: &[ToolCallRecord]) -> bool {
        let last_successful_source_mutation_index = tool_calls
            .iter()
            .enumerate()
            .filter_map(|(index, tool_call)| {
                (Self::is_successful_mutating_file_tool_call(tool_call)
                    || Self::is_successful_mutating_code_tool_call(tool_call))
                .then_some(index)
            })
            .next_back();

        tool_calls.iter().enumerate().any(|(index, tool_call)| {
            last_successful_source_mutation_index.is_some_and(|success_index| {
                index > success_index
                    && matches!(
                        tool_call.result,
                        ToolResult::Error(_) | ToolResult::Skipped(_)
                    )
                    && (Self::is_file_mutation_attempt(tool_call)
                        || Self::is_code_mutation_attempt(tool_call))
            })
        })
    }

    pub(super) fn is_successful_mutating_file_tool_call(tool_call: &ToolCallRecord) -> bool {
        if !matches!(tool_call.name.as_str(), "file" | "write_file" | "edit_file")
            || !matches!(tool_call.result, ToolResult::Success(_))
        {
            return false;
        }

        let Some(operation) = Self::file_operation_for_suspension(tool_call) else {
            return false;
        };

        let ToolResult::Success(output) = &tool_call.result else {
            return false;
        };

        match operation.as_str() {
            "write" => {
                let normalized = output.to_ascii_lowercase();
                !normalized.contains("made no changes") && !normalized.contains("unchanged")
            }
            "edit" => serde_json::from_str::<FileEditMutationResult>(output)
                .map(|result| result.changed)
                .unwrap_or_else(|_| !output.to_ascii_lowercase().contains("unchanged")),
            _ => false,
        }
    }

    pub(super) fn is_code_mutation_attempt(tool_call: &ToolCallRecord) -> bool {
        if !crate::tools::registry::is_code_tool_name(&tool_call.name) {
            return false;
        }

        serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
            .ok()
            .map(|value| {
                value
                    .get("operation")
                    .and_then(|operation| operation.as_str())
                    .map(|operation| matches!(operation, "edit" | "batch_edit" | "apply_fix"))
                    .or_else(|| {
                        value
                            .get("edits")
                            .and_then(|edits| edits.as_array())
                            .map(|edits| !edits.is_empty())
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub(super) fn keep_tracked_task_open(session_id: &str, task_id: &str) {
        let manager = crate::get_global_task_manager();
        let task_is_terminal = manager
            .get_task(session_id, task_id)
            .ok()
            .flatten()
            .is_some_and(|task| task.is_terminal());
        if task_is_terminal {
            return;
        }

        let _ =
            Self::apply_tracked_phase_status(session_id, task_id, crate::TaskStatus::InProgress);
        let _ = manager.set_current_task_id(session_id, Some(task_id.to_string()));
    }

    #[allow(dead_code)]
    pub(super) fn reconcile_tracked_task_after_success(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        session_id: Option<&str>,
        task_id: Option<&str>,
        final_response: &str,
        tool_calls: &[ToolCallRecord],
    ) {
        let Some((session_id, task_id)) = Self::tracked_task_context(session_id, task_id) else {
            return;
        };

        let runtime_state = Self::reconcile_tracked_execution_progress_from_tool_activity(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            Some(session_id),
            Some(task_id),
            tool_calls,
        );
        let final_response_signals_success = Self::final_response_signals_successful_completion(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            final_response,
            tool_calls,
        );

        if !Self::tool_calls_support_requested_success_closeout(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            tool_calls,
        ) {
            if let Some(state) = runtime_state.as_ref() {
                Self::record_tracked_task_incomplete_memory_event(
                    Some(session_id),
                    Some(task_id),
                    state,
                );
            }
            Self::keep_tracked_task_open(session_id, task_id);
            tracing::info!(
                session_id = %session_id,
                task_id = %task_id,
                "Skipping tracked task success reconciliation because runtime evidence does not yet indicate successful completion"
            );
            return;
        }

        if !final_response_signals_success {
            Self::keep_tracked_task_open(session_id, task_id);
            tracing::info!(
                session_id = %session_id,
                task_id = %task_id,
                "Skipping tracked task success reconciliation because the final response does not claim successful completion"
            );
            return;
        }

        if let Some(state) = runtime_state
            && !state.completion_ready
        {
            if !Self::runtime_state_allows_success_closeout(&state) {
                Self::record_tracked_task_incomplete_memory_event(
                    Some(session_id),
                    Some(task_id),
                    &state,
                );
                Self::keep_tracked_task_open(session_id, task_id);
                tracing::info!(
                    session_id = %session_id,
                    task_id = %task_id,
                    missing_requirements = ?state.snapshot.missing_requirements,
                    "Skipping tracked task descendant closeout because runtime requirements remain unmet"
                );
                return;
            }

            if state.open_descendant_summary.has_open() {
                Self::reconcile_open_descendants_after_success(
                    session_id,
                    task_id,
                    final_response,
                    tool_calls,
                );
                if let Some(updated_state) =
                    Self::reconcile_tracked_execution_progress_from_tool_activity(
                        requires_build_and_test,
                        requires_mutating_file_tool_success,
                        Some(session_id),
                        Some(task_id),
                        tool_calls,
                    )
                {
                    if updated_state.completion_ready {
                        return;
                    }

                    if updated_state.open_descendant_summary.has_open()
                        && Self::text_signals_broad_plan_completion(final_response)
                    {
                        let terminalized =
                            Self::terminalize_remaining_open_descendants_after_success_closeout(
                                session_id, task_id, true,
                            );
                        if !terminalized.is_empty()
                            && let Some(closeout_state) =
                                Self::reconcile_tracked_execution_progress_from_tool_activity(
                                    requires_build_and_test,
                                    requires_mutating_file_tool_success,
                                    Some(session_id),
                                    Some(task_id),
                                    tool_calls,
                                )
                        {
                            if closeout_state.completion_ready {
                                return;
                            }

                            Self::record_tracked_task_incomplete_memory_event(
                                Some(session_id),
                                Some(task_id),
                                &closeout_state,
                            );
                            Self::keep_tracked_task_open(session_id, task_id);
                            tracing::warn!(
                                session_id = %session_id,
                                task_id = %task_id,
                                open_descendants = closeout_state.open_descendant_summary.total(),
                                missing_requirements = ?closeout_state.snapshot.missing_requirements,
                                "Tracked task remains open after broad success closeout terminalization"
                            );
                            return;
                        }
                    }

                    Self::record_tracked_task_incomplete_memory_event(
                        Some(session_id),
                        Some(task_id),
                        &updated_state,
                    );
                    Self::keep_tracked_task_open(session_id, task_id);
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        open_descendants = updated_state.open_descendant_summary.total(),
                        missing_requirements = ?updated_state.snapshot.missing_requirements,
                        "Tracked task remains open after success closeout reconciliation"
                    );
                    return;
                }
            }

            Self::record_tracked_task_incomplete_memory_event(
                Some(session_id),
                Some(task_id),
                &state,
            );
            Self::keep_tracked_task_open(session_id, task_id);

            tracing::warn!(
                session_id = %session_id,
                task_id = %task_id,
                open_descendants = state.open_descendant_summary.total(),
                missing_requirements = ?state.snapshot.missing_requirements,
                "Tracked task remains open after runtime reconciliation"
            );
        }
    }

    pub(super) fn cancel_tracked_task(
        session_id: Option<&str>,
        task_id: Option<&str>,
        reason: &str,
    ) {
        let Some((session_id, task_id)) = Self::tracked_task_context(session_id, task_id) else {
            return;
        };

        let manager = crate::get_global_task_manager();
        let cancelled_descendants = Self::cancel_open_descendants(session_id, task_id, reason);
        if !cancelled_descendants.is_empty() {
            tracing::info!(
                session_id = %session_id,
                task_id = %task_id,
                cancelled_descendants = ?cancelled_descendants,
                cancellation_reason = reason,
                "Cancelled tracked descendants after interrupted agent run"
            );
        }
        match manager.get_task(session_id, task_id) {
            Ok(Some(task)) => {
                if task.status != crate::TaskStatus::Cancelled
                    && let Err(error) = manager.update_task_status(
                        session_id,
                        task_id,
                        crate::TaskStatus::Cancelled,
                    )
                {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task_id,
                        error = %error,
                        cancellation_reason = reason,
                        "Failed to cancel tracked task after interrupted agent run"
                    );
                    return;
                }

                if let Ok(Some(current_task_id)) = manager.get_current_task_id(session_id)
                    && current_task_id == task_id
                {
                    let _ = manager.set_current_task_id(session_id, None);
                }
            }
            Ok(None) => {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    cancellation_reason = reason,
                    "Tracked task was not found when attempting to cancel it"
                );
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    error = %error,
                    cancellation_reason = reason,
                    "Failed to load tracked task before cancellation"
                );
            }
        }
    }

    pub(super) fn tracked_open_descendant_summary(
        session_id: Option<&str>,
        task_id: Option<&str>,
    ) -> OpenDescendantSummary {
        let (Some(session_id), Some(task_id)) = (session_id, task_id) else {
            return OpenDescendantSummary::default();
        };

        let manager = crate::get_global_task_manager();
        let Ok(descendants) = manager.list_descendants(session_id, task_id) else {
            return OpenDescendantSummary::default();
        };

        let open_descendants = descendants
            .into_iter()
            .filter(|task| !task.is_terminal())
            .collect::<Vec<_>>();
        OpenDescendantSummary::from_tasks(&open_descendants)
    }

    #[allow(dead_code)]
    pub(super) fn tracked_open_descendant_summary_after_success_reconciliation(
        &self,
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        session_id: Option<&str>,
        task_id: Option<&str>,
        final_response: &str,
        tool_calls: &[ToolCallRecord],
    ) -> OpenDescendantSummary {
        let _ = final_response;
        let Some((session_id, task_id)) = Self::tracked_task_context(session_id, task_id) else {
            return OpenDescendantSummary::default();
        };
        let manager = crate::get_global_task_manager();
        let previous_current_task_id = manager.get_current_task_id(session_id).ok().flatten();

        let runtime_state = Self::reconcile_tracked_execution_progress_from_tool_activity(
            requires_build_and_test,
            requires_mutating_file_tool_success,
            Some(session_id),
            Some(task_id),
            tool_calls,
        );
        let mut summary = runtime_state
            .as_ref()
            .map(|state| state.open_descendant_summary)
            .unwrap_or_default();

        if summary.has_open()
            && Self::tool_calls_support_requested_success_closeout(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                tool_calls,
            )
            && runtime_state
                .as_ref()
                .is_none_or(Self::runtime_state_allows_success_closeout)
            && Self::final_response_signals_successful_completion(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                final_response,
                tool_calls,
            )
        {
            Self::reconcile_open_descendants_after_success(
                session_id,
                task_id,
                final_response,
                tool_calls,
            );
            summary = Self::tracked_open_descendant_summary(Some(session_id), Some(task_id));
            if summary.has_open() {
                if let Some(previous_current_task_id) = previous_current_task_id {
                    let _ = manager.set_current_task_id(session_id, Some(previous_current_task_id));
                }
            } else {
                let _ = manager.set_current_task_id(session_id, None);
            }
        }

        summary
    }

    pub(super) fn tool_calls_support_requested_success_closeout(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        tool_calls: &[ToolCallRecord],
    ) -> bool {
        !Self::is_missing_requested_build_and_test(requires_build_and_test, tool_calls)
            && Self::tool_results_support_successful_completion(
                requires_mutating_file_tool_success,
                tool_calls,
            )
    }

    #[allow(dead_code)]
    pub(super) fn llm_provider_is_configured(&self, provider: &str) -> bool {
        match provider {
            "openai" => self
                .config
                .llm
                .openai
                .as_ref()
                .is_some_and(|config| !config.api_key.trim().is_empty()),
            "anthropic" => self
                .config
                .llm
                .anthropic
                .as_ref()
                .is_some_and(|config| !config.api_key.trim().is_empty()),
            "gemini" => self
                .config
                .llm
                .gemini
                .as_ref()
                .is_some_and(|config| !config.api_key.trim().is_empty()),
            "grok" => self
                .config
                .llm
                .grok
                .as_ref()
                .is_some_and(|config| !config.api_key.trim().is_empty()),
            "ollama" => self.config.llm.ollama.is_some(),
            _ => false,
        }
    }

    #[allow(dead_code)]
    pub(super) fn closeout_history_validation_available(&self) -> bool {
        self.llm_provider_is_configured(&self.config.llm.primary)
            || self
                .pipeline_config
                .enable_fallback
                .then_some(self.config.llm.fallback.as_deref())
                .flatten()
                .is_some_and(|provider| self.llm_provider_is_configured(provider))
    }

    #[allow(dead_code)]
    pub(super) fn load_open_descendants(
        session_id: &str,
        task_id: &str,
    ) -> Option<Vec<crate::Task>> {
        crate::get_global_task_manager()
            .list_descendants(session_id, task_id)
            .map(|tasks| {
                tasks
                    .into_iter()
                    .filter(|task| !task.is_terminal())
                    .collect::<Vec<_>>()
            })
            .map_err(|error| {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %task_id,
                    error = %error,
                    "Failed to load open descendants for closeout history validation"
                );
                error
            })
            .ok()
    }

    #[allow(dead_code)]
    pub(super) fn format_tool_result_for_history_validation(
        &self,
        result: &ToolResult,
    ) -> (&'static str, String) {
        match result {
            ToolResult::Success(output) => ("success", self.truncate_tool_result(output)),
            ToolResult::Error(output) => ("error", self.truncate_tool_result(output)),
            ToolResult::Skipped(output) => ("skipped", self.truncate_tool_result(output)),
        }
    }

    #[allow(dead_code)]
    pub(super) fn build_closeout_history_validation_prompt(
        &self,
        final_response: &str,
        tool_calls: &[ToolCallRecord],
        open_descendants: &[crate::Task],
    ) -> String {
        let mut prompt = String::from(
            "You are validating tracked subtasks after an agent run completed. Determine which candidate task IDs have clear evidence of completion from this run.\n\n",
        );
        prompt.push_str(
            "Return STRICT JSON only in this exact shape: {\"completed_task_ids\":[\"task-id\"]}.\n",
        );
        prompt.push_str(
            "Rules:\n- Include ONLY candidate task IDs listed below.\n- Include a task ID only when the final response or tool history shows the task was actually finished.\n- If the evidence is ambiguous, leave the task ID out.\n- Do NOT infer completion from plans, placeholders, or future work.\n- Never include the root task.\n\n",
        );
        prompt.push_str("Open descendant candidates:\n");
        for task in open_descendants {
            prompt.push_str(&format!(
                "- id={} | status={:?} | name={} | description={}\n",
                task.id, task.status, task.name, task.description
            ));
        }

        prompt.push_str("\nFinal assistant response:\n");
        prompt.push_str(&self.truncate_tool_result(final_response));
        prompt.push_str("\n\nTool history from this run (most recent last):\n");

        let history_window = 20usize;
        let start_index = tool_calls.len().saturating_sub(history_window);
        if start_index > 0 {
            prompt.push_str(&format!(
                "[Only the last {} tool calls are shown due to prompt budget.]\n",
                history_window
            ));
        }

        for (index, tool_call) in tool_calls.iter().enumerate().skip(start_index) {
            let args = self.truncate_tool_result(&tool_call.arguments);
            let (result_kind, result_output) =
                self.format_tool_result_for_history_validation(&tool_call.result);
            prompt.push_str(&format!(
                "{}. tool={} id={} result={}\nargs={}\noutput={}\n\n",
                index + 1,
                tool_call.name,
                tool_call.id,
                result_kind,
                args,
                result_output
            ));
        }

        prompt
    }

    #[allow(dead_code)]
    pub(super) fn parse_closeout_history_validation_response(
        response: &str,
    ) -> Option<HistoryValidatedTaskCompletion> {
        let trimmed = response.trim();
        serde_json::from_str::<HistoryValidatedTaskCompletion>(trimmed)
            .ok()
            .or_else(|| {
                let start = trimmed.find('{')?;
                let end = trimmed.rfind('}')?;
                serde_json::from_str::<HistoryValidatedTaskCompletion>(&trimmed[start..=end]).ok()
            })
    }

    #[allow(dead_code)]
    pub(super) fn open_descendant_depth(
        task: &crate::Task,
        root_task_id: &str,
        descendant_map: &HashMap<&str, &crate::Task>,
    ) -> usize {
        let mut depth = 0usize;
        let mut current_parent = task.parent_id.as_deref();

        while let Some(parent_id) = current_parent {
            depth += 1;
            if parent_id == root_task_id {
                break;
            }
            current_parent = descendant_map
                .get(parent_id)
                .and_then(|parent| parent.parent_id.as_deref());
        }

        depth
    }

    #[allow(dead_code)]
    pub(super) fn apply_history_validated_descendant_completions(
        session_id: &str,
        root_task_id: &str,
        open_descendants: &[crate::Task],
        completed_task_ids: &[String],
    ) -> Vec<String> {
        let completed_id_set = completed_task_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if completed_id_set.is_empty() {
            return Vec::new();
        }

        let descendant_map = open_descendants
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect::<HashMap<_, _>>();
        let mut tasks_to_complete = open_descendants
            .iter()
            .filter(|task| completed_id_set.contains(task.id.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        tasks_to_complete.sort_by(|left, right| {
            let left_depth = Self::open_descendant_depth(left, root_task_id, &descendant_map);
            let right_depth = Self::open_descendant_depth(right, root_task_id, &descendant_map);
            right_depth
                .cmp(&left_depth)
                .then_with(|| left.name.cmp(&right.name))
        });

        let manager = crate::get_global_task_manager();
        let mut applied_task_ids = Vec::new();

        for task in tasks_to_complete {
            if !Self::history_validated_completion_satisfies_direct_proof(session_id, &task) {
                tracing::debug!(
                    session_id = %session_id,
                    task_id = %task.id,
                    task_name = %task.name,
                    "Skipping history-validated completion because direct proof requirements are not satisfied"
                );
                continue;
            }

            match manager.update_task_status(session_id, &task.id, crate::TaskStatus::Completed) {
                Ok(_) => applied_task_ids.push(task.id),
                Err(error) => {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %task.id,
                        task_name = %task.name,
                        error = %error,
                        "Failed to apply history-validated descendant completion"
                    );
                }
            }
        }

        applied_task_ids
    }

    pub(super) fn history_validated_completion_satisfies_direct_proof(
        session_id: &str,
        task: &crate::Task,
    ) -> bool {
        if Self::task_requires_user_facing_closeout(task) {
            return false;
        }

        let inferred_profile = Self::task_execution_profile(task, false);
        let requires_direct_proof = matches!(
            inferred_profile.execution_kind,
            TaskExecutionKind::Implementation | TaskExecutionKind::Verification
        ) || inferred_profile.requires_external_evidence
            || inferred_profile.requires_launch_evidence
            || inferred_profile.requires_build
            || inferred_profile.requires_test;

        if !requires_direct_proof {
            return true;
        }

        let manager = crate::get_global_task_manager();
        manager
            .get_execution_state(session_id, &task.id)
            .ok()
            .flatten()
            .map(|execution_state| {
                let mut execution_state = execution_state;
                execution_state.merge_profile(inferred_profile);
                execution_state.satisfies_profile()
            })
            .unwrap_or(false)
    }

    #[allow(dead_code)]
    pub(super) fn terminalize_remaining_open_descendants_after_success_closeout(
        session_id: &str,
        root_task_id: &str,
        broad_plan_completion_claimed: bool,
    ) -> Vec<(String, crate::TaskStatus)> {
        let manager = crate::get_global_task_manager();
        let mut applied = Vec::new();

        loop {
            let open_descendants = match manager.list_descendants(session_id, root_task_id) {
                Ok(tasks) => tasks
                    .into_iter()
                    .filter(|task| !task.is_terminal())
                    .collect::<Vec<_>>(),
                Err(error) => {
                    tracing::warn!(
                        session_id = %session_id,
                        task_id = %root_task_id,
                        error = %error,
                        "Failed to inspect remaining open descendants during success closeout"
                    );
                    return applied;
                }
            };

            if open_descendants.is_empty() {
                return applied;
            }

            let leaf_actions = open_descendants
                .iter()
                .filter(|descendant| {
                    !open_descendants.iter().any(|candidate| {
                        candidate.parent_id.as_deref() == Some(descendant.id.as_str())
                    })
                })
                .filter_map(|descendant| {
                    let is_placeholder = Self::looks_like_placeholder_task_name(&descendant.name)
                        || Self::looks_like_placeholder_task_name(&descendant.description);
                    let profile = Self::task_execution_profile(descendant, false);
                    let requires_direct_proof = matches!(
                        profile.execution_kind,
                        TaskExecutionKind::Implementation | TaskExecutionKind::Verification
                    ) || profile.requires_external_evidence
                        || profile.requires_launch_evidence
                        || profile.requires_build
                        || profile.requires_test;
                    if Self::task_requires_user_facing_closeout(descendant) {
                        return None;
                    }
                    let target_status = match descendant.status {
                        crate::TaskStatus::InProgress => {
                            if requires_direct_proof && broad_plan_completion_claimed {
                                return None;
                            }
                            crate::TaskStatus::Completed
                        }
                        crate::TaskStatus::NotStarted
                            if broad_plan_completion_claimed
                                && !is_placeholder
                                && !requires_direct_proof =>
                        {
                            crate::TaskStatus::Completed
                        }
                        crate::TaskStatus::NotStarted | crate::TaskStatus::Blocked
                            if requires_direct_proof =>
                        {
                            return None;
                        }
                        crate::TaskStatus::NotStarted | crate::TaskStatus::Blocked => {
                            crate::TaskStatus::Cancelled
                        }
                        crate::TaskStatus::Completed | crate::TaskStatus::Cancelled => {
                            unreachable!("terminal tasks are filtered out above")
                        }
                    };
                    Some((descendant.id.clone(), target_status))
                })
                .collect::<Vec<_>>();

            if leaf_actions.is_empty() {
                tracing::warn!(
                    session_id = %session_id,
                    task_id = %root_task_id,
                    open_descendants = open_descendants.len(),
                    "Remaining open descendants could not be reduced to leaves during success closeout"
                );
                return applied;
            }

            let mut made_progress = false;
            for (task_id, status) in leaf_actions {
                match manager.update_task_status(session_id, &task_id, status) {
                    Ok(_) => {
                        made_progress = true;
                        applied.push((task_id, status));
                    }
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session_id,
                            task_id = %task_id,
                            target_status = ?status,
                            error = %error,
                            "Failed to terminalize remaining open descendant during success closeout"
                        );
                    }
                }
            }

            if !made_progress {
                return applied;
            }
        }
    }

    pub(super) async fn reconcile_tracked_task_after_success_with_history_validation(
        &self,
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        session_id: Option<&str>,
        task_id: Option<&str>,
        final_response: &str,
        tool_calls: &[ToolCallRecord],
    ) {
        let _ = self;
        let session_id = session_id.map(str::to_string);
        let task_id = task_id.map(str::to_string);
        let final_response = final_response.to_string();
        let tool_calls = tool_calls.to_vec();

        let _ = Self::run_blocking_task_bookkeeping(
            "reconcile_tracked_task_after_success_with_history_validation",
            move || {
                Self::reconcile_tracked_task_after_success(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    session_id.as_deref(),
                    task_id.as_deref(),
                    &final_response,
                    &tool_calls,
                )
            },
        )
        .await;
    }
}
