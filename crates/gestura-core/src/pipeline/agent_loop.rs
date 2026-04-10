#![allow(clippy::question_mark)]
#![allow(clippy::too_many_arguments)]
// Root orchestration for the async agent loop lives here.
// Shared iteration/finalization helpers, tracked-task bookkeeping, narration /
// status emission, and continuation/closeout prompt logic are split into
// sidecar modules to keep streaming and buffered paths reusable.
mod continuation;
mod narration;
mod shared;
mod tracked_tasks;

use super::{request_telemetry::AgentLoopContinuation, *};
use crate::tasks::{
    TaskExecutionEvidence, TaskExecutionEvidenceKind, TaskExecutionKind, TaskVerificationProfile,
};
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use shared::PreparedLoopIteration;
use std::collections::{HashMap, HashSet};
use tracing::Instrument as _;

const MAX_PARALLEL_READ_ONLY_TOOL_CALLS: usize = 4;
const PUBLIC_NARRATION_LLM_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(600);
const STREAM_TASK_JOIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const STREAM_STATUS_FORWARD_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);
const MIN_PUBLIC_NARRATION_TITLE_WORDS: usize = 2;
const MAX_PUBLIC_NARRATION_TITLE_WORDS: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IncompleteRunReason {
    MissingTerminalSummary,
    IterationBudgetExhausted { max_iterations: usize },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct OpenDescendantSummary {
    not_started: usize,
    blocked: usize,
    in_progress: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ToolSuspensionState {
    task: bool,
    file: bool,
    code: bool,
}

#[derive(Clone, Copy, Debug)]
struct OpenSubtaskContinuationInput<'a> {
    saw_any_tool_calls: bool,
    open_descendant_summary: OpenDescendantSummary,
    task_tool_suspended: bool,
    iteration_content: &'a str,
    iteration: usize,
    max_iterations: Option<usize>,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
struct HistoryValidatedTaskCompletion {
    #[serde(default)]
    completed_task_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct FileEditMutationResult {
    #[serde(default)]
    changed: bool,
}

#[derive(Debug, Deserialize)]
struct PublicNarrationPayloadCandidate {
    title: Option<String>,
    message: Option<String>,
    summary: Option<String>,
    reason: Option<String>,
    next_step: Option<String>,
    #[serde(default)]
    evidence: Vec<String>,
}

#[derive(Debug, Default)]
struct PublicNarrationDraft {
    title: Option<String>,
    message: Option<String>,
    summary: Option<String>,
    reason: Option<String>,
    next_step: Option<String>,
    evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PublicNarrationContextFrame {
    stage: crate::streaming::NarrationStage,
    change_kind: PublicNarrationChangeKind,
    summary_hint: Option<String>,
    reason_hint: Option<String>,
    next_step_hint: Option<String>,
    evidence: Vec<String>,
    tracked_work_incomplete: bool,
    completion_ready: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ObservedRuntimeEvidence {
    saw_successful_tool_work: bool,
    saw_diagnostic_progress: bool,
    saw_contradiction: bool,
    saw_blocker: bool,
    saw_mutation: bool,
    successful_source_mutation: bool,
    mutation_requirement_satisfied: bool,
    saw_generic_verification_progress: bool,
    build_attempted: bool,
    build_completed: bool,
    test_attempted: bool,
    test_completed: bool,
    latest_contradiction_summary: Option<String>,
    latest_blocker_summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolIterationStagnationFingerprint {
    outcome_fingerprints: Vec<String>,
    evidence: ObservedRuntimeEvidence,
    missing_requirements: Vec<String>,
    current_task: Option<String>,
    ready_tasks: Vec<String>,
    parallel_ready_tasks: Vec<String>,
    blocked_tasks: Vec<String>,
    completion_ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrackedTaskRuntimeState {
    snapshot: crate::streaming::TaskRuntimeSnapshot,
    open_descendant_summary: OpenDescendantSummary,
    completion_ready: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PublicNarrationState {
    last_message: Option<String>,
    last_message_fingerprint: Option<String>,
    last_state_fingerprint: Option<String>,
    last_runtime_snapshot: Option<crate::streaming::TaskRuntimeSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicNarrationTrigger {
    BatchStart,
    ResultsReview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicNarrationChangeKind {
    Discovery,
    Confirmation,
    Contradiction,
    Decision,
    Blocker,
    Completion,
    Continuation,
}

impl PublicNarrationChangeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Discovery => "discovery",
            Self::Confirmation => "confirmation",
            Self::Contradiction => "contradiction",
            Self::Decision => "decision",
            Self::Blocker => "blocker",
            Self::Completion => "completion",
            Self::Continuation => "continuation",
        }
    }
}

impl OpenDescendantSummary {
    fn from_tasks(tasks: &[crate::Task]) -> Self {
        let mut summary = Self::default();
        for task in tasks {
            match task.status {
                crate::TaskStatus::NotStarted => summary.not_started += 1,
                crate::TaskStatus::Blocked => summary.blocked += 1,
                crate::TaskStatus::InProgress => summary.in_progress += 1,
                crate::TaskStatus::Completed | crate::TaskStatus::Cancelled => {}
            }
        }
        summary
    }

    fn total(self) -> usize {
        self.not_started + self.blocked + self.in_progress
    }

    fn has_open(self) -> bool {
        self.total() > 0
    }

    #[allow(dead_code)]
    fn only_not_started(self) -> bool {
        self.not_started > 0 && self.blocked == 0 && self.in_progress == 0
    }
}

impl AgentPipeline {
    fn stable_stagnation_checksum(text: &str) -> String {
        const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;

        let mut hash = FNV_OFFSET_BASIS;
        for byte in text.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }

        format!("{hash:016x}")
    }

    fn successful_mutation_stagnation_signature(tool_call: &ToolCallRecord) -> Option<String> {
        let arguments = serde_json::from_str::<serde_json::Value>(&tool_call.arguments).ok();

        if Self::is_successful_mutating_file_tool_call(tool_call) {
            let operation = Self::file_operation_for_suspension(tool_call)
                .unwrap_or_else(|| "mutation".to_string());
            let path = arguments
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(|value| value.as_str())
                .unwrap_or("<unknown>");
            return Some(format!(
                "file-mutation:{operation}:{path}:{}",
                Self::stable_stagnation_checksum(&tool_call.arguments)
            ));
        }

        if Self::is_successful_mutating_code_tool_call(tool_call) {
            let operation = arguments
                .as_ref()
                .and_then(|value| value.get("operation"))
                .and_then(|value| value.as_str())
                .unwrap_or(tool_call.name.as_str());
            return Some(format!(
                "code-mutation:{operation}:{}",
                Self::stable_stagnation_checksum(&tool_call.arguments)
            ));
        }

        None
    }

    fn observed_runtime_evidence(tool_calls: &[ToolCallRecord]) -> ObservedRuntimeEvidence {
        let saw_successful_tool_work = Self::has_any_successful_non_task_tool_call(tool_calls);
        let saw_diagnostic_progress = tool_calls
            .iter()
            .any(Self::tool_call_counts_as_diagnostic_progress);
        let latest_non_task_tool_call = tool_calls
            .iter()
            .rev()
            .find(|tool_call| !Self::is_task_tool_name(&tool_call.name));
        let latest_contradiction_summary =
            latest_non_task_tool_call.and_then(Self::tool_call_contradiction_summary);
        let latest_blocker_summary =
            latest_non_task_tool_call.and_then(Self::tool_call_blocker_summary);
        let successful_source_mutation = tool_calls.iter().any(|tool_call| {
            Self::is_successful_mutating_file_tool_call(tool_call)
                || Self::is_successful_mutating_code_tool_call(tool_call)
        });
        let attempted_source_mutation = tool_calls.iter().any(|tool_call| {
            Self::is_file_mutation_attempt(tool_call) || Self::is_code_mutation_attempt(tool_call)
        });
        let successful_shell_mutation = tool_calls
            .iter()
            .any(Self::is_successful_mutating_shell_tool_call);
        let saw_mutation = successful_source_mutation || successful_shell_mutation;
        let mutation_requirement_satisfied = if attempted_source_mutation {
            successful_source_mutation
        } else {
            successful_source_mutation || successful_shell_mutation
        };
        let saw_generic_verification_progress = tool_calls
            .iter()
            .any(Self::is_successful_generic_verification_tool_call);
        let (build_attempted, build_completed, test_attempted, test_completed) =
            Self::build_and_test_completion_status(tool_calls);
        ObservedRuntimeEvidence {
            saw_successful_tool_work,
            saw_diagnostic_progress,
            saw_contradiction: latest_contradiction_summary.is_some(),
            saw_blocker: latest_blocker_summary.is_some(),
            saw_mutation,
            successful_source_mutation,
            mutation_requirement_satisfied,
            saw_generic_verification_progress,
            build_attempted,
            build_completed,
            test_attempted,
            test_completed,
            latest_contradiction_summary,
            latest_blocker_summary,
        }
    }

    fn keyword_match_score(text: &str, keywords: &[&str]) -> usize {
        keywords
            .iter()
            .filter(|keyword| text.contains(**keyword))
            .count()
    }

    fn task_mentions_build_verification(task: &crate::Task) -> bool {
        Self::task_text_contains_any(
            task,
            &[
                "build",
                "bundle",
                "package",
                "cargo check",
                "cargo build",
                "npm run build",
                "pnpm build",
                "yarn build",
                "typecheck",
                "type check",
                "run checks",
                "verification commands",
            ],
        )
    }

    fn task_mentions_test_verification(task: &crate::Task) -> bool {
        Self::task_text_contains_any(
            task,
            &[
                "test",
                "tests",
                "cargo test",
                "npm test",
                "pnpm test",
                "yarn test",
                "pytest",
                "vitest",
                "jest",
                "lint",
                "smoke",
            ],
        )
    }

    fn task_requires_external_verification(task: &crate::Task) -> bool {
        Self::task_text_contains_any(
            task,
            &[
                "cross-check",
                "cross check",
                "verify facts",
                "fact check",
                "fact-check",
                "outside evidence",
                "external evidence",
                "recent sources",
                "supporting sources",
                "source verification",
            ],
        )
    }

    fn task_execution_profile(
        task: &crate::Task,
        requires_build_and_test: bool,
    ) -> TaskVerificationProfile {
        let planning_keywords = [
            "plan",
            "review",
            "approach",
            "inspect",
            "analy",
            "investigat",
            "research",
        ];
        let implementation_keywords = [
            "implement",
            "change",
            "edit",
            "scaffold",
            "create",
            "build ui",
            "update",
            "fix",
        ];
        let verification_keywords = [
            "build",
            "test",
            "verify",
            "validation",
            "check",
            "compile",
            "lint",
            "smoke",
        ];

        let name = task.name.to_ascii_lowercase();
        let description = task.description.to_ascii_lowercase();
        let planning_score = Self::keyword_match_score(&name, &planning_keywords) * 3
            + Self::keyword_match_score(&description, &planning_keywords);
        let implementation_score = Self::keyword_match_score(&name, &implementation_keywords) * 3
            + Self::keyword_match_score(&description, &implementation_keywords);
        let verification_score = Self::keyword_match_score(&name, &verification_keywords) * 3
            + Self::keyword_match_score(&description, &verification_keywords);

        let mut profile = TaskVerificationProfile::default();
        if planning_score > 0
            && planning_score >= implementation_score
            && planning_score >= verification_score
        {
            profile.execution_kind = TaskExecutionKind::Planning;
            profile.parallel_safe = true;
            return profile;
        }
        if implementation_score > 0 && implementation_score >= verification_score {
            profile.execution_kind = TaskExecutionKind::Implementation;
            profile.requires_mutation = true;
            return profile;
        }
        if verification_score > 0 {
            profile.execution_kind = TaskExecutionKind::Verification;
            let mentions_build = Self::task_mentions_build_verification(task);
            let mentions_test = Self::task_mentions_test_verification(task);

            if mentions_build || requires_build_and_test {
                profile.requires_build = true;
            }
            if mentions_test || requires_build_and_test {
                profile.requires_test = true;
            }
            if !profile.requires_build && !profile.requires_test {
                profile.requires_build = requires_build_and_test;
                profile.requires_test = requires_build_and_test;
            }
            profile.requires_external_evidence = Self::task_requires_external_verification(task);
            return profile;
        }

        profile.execution_kind = TaskExecutionKind::General;
        profile.parallel_safe = true;
        profile
    }

    fn task_priority_bucket(task: &crate::Task, profile: &TaskVerificationProfile) -> u8 {
        let status_rank = match task.status {
            crate::TaskStatus::InProgress => 0,
            crate::TaskStatus::NotStarted => 1,
            crate::TaskStatus::Blocked => 2,
            crate::TaskStatus::Completed | crate::TaskStatus::Cancelled => 3,
        };
        let kind_rank = match profile.execution_kind {
            TaskExecutionKind::Planning => 0,
            TaskExecutionKind::Implementation => 1,
            TaskExecutionKind::Verification => 2,
            TaskExecutionKind::General => 3,
        };
        status_rank * 10 + kind_rank
    }

    fn task_runtime_view(task: &crate::Task) -> crate::streaming::TaskRuntimeTaskView {
        crate::streaming::TaskRuntimeTaskView {
            id: task.id.clone(),
            name: task.name.clone(),
            status: task.status.to_string(),
        }
    }

    fn runtime_missing_requirements(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        evidence: ObservedRuntimeEvidence,
    ) -> Vec<String> {
        let mut missing = Vec::new();
        if requires_mutating_file_tool_success && !evidence.mutation_requirement_satisfied {
            missing.push("source mutation not yet verified".to_string());
        }
        if requires_build_and_test && !evidence.build_completed {
            missing.push("build/check command not yet observed".to_string());
        }
        if requires_build_and_test && !evidence.test_completed {
            missing.push("test command not yet observed".to_string());
        }
        if let Some(summary) = evidence.latest_blocker_summary {
            missing.push(format!("unresolved blocker: {summary}"));
        } else if let Some(summary) = evidence.latest_contradiction_summary {
            missing.push(format!("unresolved contradiction: {summary}"));
        }
        missing
    }

    fn runtime_snapshot_status_message(
        current_task: Option<&crate::Task>,
        ready_tasks: &[crate::Task],
        parallel_ready_tasks: &[crate::Task],
        missing_requirements: &[String],
    ) -> String {
        let mut parts = Vec::new();
        if let Some(task) = current_task {
            parts.push(format!("Current task: {} [{}]", task.name, task.status));
        } else if !ready_tasks.is_empty() {
            parts.push("I have ready next steps, but I haven't focused one yet".to_string());
        } else {
            parts.push("I don't have a clear next step yet".to_string());
        }
        if !parallel_ready_tasks.is_empty() {
            parts.push(format!(
                "can also run in parallel: {}",
                parallel_ready_tasks
                    .iter()
                    .map(|task| task.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !missing_requirements.is_empty() {
            parts.push(format!(
                "still need to verify: {}",
                missing_requirements.join(", ")
            ));
        }
        parts.join(" · ")
    }

    fn normalize_stagnation_text(text: &str) -> String {
        let mut normalized = String::with_capacity(text.len().min(160));
        let mut last_was_whitespace = false;

        for ch in text.chars().flat_map(char::to_lowercase) {
            if ch.is_whitespace() {
                if !normalized.is_empty() && !last_was_whitespace {
                    normalized.push(' ');
                }
                last_was_whitespace = true;
                continue;
            }

            normalized.push(ch);
            last_was_whitespace = false;

            if normalized.len() >= 160 {
                break;
            }
        }

        normalized.trim().to_string()
    }

    fn tool_result_fingerprint(tool_call: &ToolCallRecord) -> String {
        if let Some(mutation_signature) = Self::successful_mutation_stagnation_signature(tool_call)
        {
            return format!("{}:success:{mutation_signature}", tool_call.name);
        }

        if let Some(summary) = Self::tool_call_blocker_summary(tool_call) {
            let normalized = Self::normalize_stagnation_text(&summary);
            return if normalized.is_empty() {
                format!("{}:blocked", tool_call.name)
            } else {
                format!("{}:blocked:{normalized}", tool_call.name)
            };
        }

        if let Some(summary) = Self::tool_call_contradiction_summary(tool_call) {
            let normalized = Self::normalize_stagnation_text(&summary);
            return if normalized.is_empty() {
                format!("{}:contradiction", tool_call.name)
            } else {
                format!("{}:contradiction:{normalized}", tool_call.name)
            };
        }

        let (kind, text) = match &tool_call.result {
            ToolResult::Success(output) => ("success", output.as_str()),
            ToolResult::Error(output) => ("error", output.as_str()),
            ToolResult::Skipped(output) => ("skipped", output.as_str()),
        };

        let normalized = Self::normalize_stagnation_text(text);
        if normalized.is_empty() {
            format!("{}:{}", tool_call.name, kind)
        } else {
            format!("{}:{}:{}", tool_call.name, kind, normalized)
        }
    }

    fn task_runtime_view_fingerprint(task: &crate::streaming::TaskRuntimeTaskView) -> String {
        format!("{}:{}", task.id, task.status)
    }

    fn task_runtime_views_fingerprint(
        tasks: &[crate::streaming::TaskRuntimeTaskView],
    ) -> Vec<String> {
        tasks
            .iter()
            .map(Self::task_runtime_view_fingerprint)
            .collect()
    }

    fn narration_requirements_fingerprint(missing_requirements: &[String]) -> String {
        if missing_requirements.is_empty() {
            return "clear".to_string();
        }

        format!(
            "missing:{}:{}",
            missing_requirements.len(),
            Self::stable_stagnation_checksum(&missing_requirements.join("|"))
        )
    }

    fn runtime_snapshot_narration_fingerprint(
        snapshot: &crate::streaming::TaskRuntimeSnapshot,
    ) -> String {
        let requirements = Self::narration_requirements_fingerprint(&snapshot.missing_requirements);

        if let Some(current_task) = snapshot.current_task.as_ref() {
            return format!(
                "runtime:task:{}:{}",
                Self::task_runtime_view_fingerprint(current_task),
                requirements
            );
        }

        if !snapshot.ready_tasks.is_empty() || !snapshot.parallel_ready_tasks.is_empty() {
            return format!("runtime:ready:{requirements}");
        }

        if !snapshot.blocked_tasks.is_empty() {
            return format!("runtime:blocked:{requirements}");
        }

        if !snapshot.open_tasks.is_empty() {
            return format!("runtime:open:{requirements}");
        }

        "runtime:complete".to_string()
    }

    fn no_tool_open_subtask_fingerprint(
        runtime_state: Option<&TrackedTaskRuntimeState>,
        open_descendant_summary: OpenDescendantSummary,
    ) -> Option<String> {
        if !open_descendant_summary.has_open() {
            return None;
        }

        let runtime_fingerprint = runtime_state
            .map(|state| Self::runtime_snapshot_narration_fingerprint(&state.snapshot))
            .unwrap_or_else(|| "runtime:unknown".to_string());

        Some(format!(
            "no-tool-open-subtasks:{runtime_fingerprint}:{}:{}:{}",
            open_descendant_summary.not_started,
            open_descendant_summary.in_progress,
            open_descendant_summary.blocked,
        ))
    }

    fn tool_iteration_stagnation_fingerprint(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        iteration_tool_calls: &[ToolCallRecord],
        runtime_state: Option<&TrackedTaskRuntimeState>,
    ) -> ToolIterationStagnationFingerprint {
        let evidence = Self::observed_runtime_evidence(iteration_tool_calls);
        let mut fingerprint_evidence = evidence.clone();
        if fingerprint_evidence.latest_blocker_summary.is_some()
            || fingerprint_evidence.latest_contradiction_summary.is_some()
        {
            fingerprint_evidence.build_attempted = false;
            fingerprint_evidence.test_attempted = false;
        }
        let missing_requirements = runtime_state
            .map(|state| state.snapshot.missing_requirements.clone())
            .unwrap_or_else(|| {
                Self::runtime_missing_requirements(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    evidence.clone(),
                )
            });

        let (current_task, ready_tasks, parallel_ready_tasks, blocked_tasks, completion_ready) =
            runtime_state
                .map(|state| {
                    (
                        state
                            .snapshot
                            .current_task
                            .as_ref()
                            .map(Self::task_runtime_view_fingerprint),
                        Self::task_runtime_views_fingerprint(&state.snapshot.ready_tasks),
                        Self::task_runtime_views_fingerprint(&state.snapshot.parallel_ready_tasks),
                        Self::task_runtime_views_fingerprint(&state.snapshot.blocked_tasks),
                        state.completion_ready,
                    )
                })
                .unwrap_or_else(|| (None, Vec::new(), Vec::new(), Vec::new(), false));

        ToolIterationStagnationFingerprint {
            outcome_fingerprints: iteration_tool_calls
                .iter()
                .map(Self::tool_result_fingerprint)
                .collect(),
            evidence: fingerprint_evidence,
            missing_requirements,
            current_task,
            ready_tasks,
            parallel_ready_tasks,
            blocked_tasks,
            completion_ready,
        }
    }

    fn update_stagnation_streak<T: Clone + PartialEq>(
        current: T,
        previous: &mut Option<T>,
        streak: &mut usize,
    ) {
        if previous.as_ref() == Some(&current) {
            *streak += 1;
        } else {
            *previous = Some(current);
            *streak = 1;
        }
    }

    fn summarize_stagnation_fingerprint(
        fingerprint: &ToolIterationStagnationFingerprint,
    ) -> String {
        let repeated_outcomes = if fingerprint.outcome_fingerprints.is_empty() {
            "no tool output was captured".to_string()
        } else {
            fingerprint
                .outcome_fingerprints
                .iter()
                .take(2)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        };

        let mut parts = vec![format!("repeated outcomes: {}", repeated_outcomes)];

        if let Some(current_task) = fingerprint.current_task.as_ref() {
            parts.push(format!("current task unchanged: {}", current_task));
        }

        if !fingerprint.missing_requirements.is_empty() {
            parts.push(format!(
                "missing requirements unchanged: {}",
                fingerprint.missing_requirements.join(", ")
            ));
        }

        if let Some(summary) = fingerprint.evidence.latest_blocker_summary.as_ref() {
            parts.push(format!("repeated blocker: {summary}"));
        } else if let Some(summary) = fingerprint.evidence.latest_contradiction_summary.as_ref() {
            parts.push(format!("repeated contradiction: {summary}"));
        }

        parts.join(" | ")
    }

    fn format_runtime_snapshot_for_prompt(
        snapshot: &crate::streaming::TaskRuntimeSnapshot,
    ) -> String {
        let mut lines = Vec::new();
        lines.push(format!("Runtime task state: {}", snapshot.status_message));
        if let Some(current_task) = &snapshot.current_task {
            lines.push(format!(
                "Current runtime-selected task: {} [{}]",
                current_task.name, current_task.status
            ));
        }
        if !snapshot.ready_tasks.is_empty() {
            lines.push("Ready tasks:".to_string());
            for task in snapshot.ready_tasks.iter().take(5) {
                lines.push(format!("- {} [{}]", task.name, task.status));
            }
        }
        if !snapshot.parallel_ready_tasks.is_empty() {
            lines.push("Parallel-safe ready tasks (batch only these together):".to_string());
            for task in snapshot.parallel_ready_tasks.iter().take(5) {
                lines.push(format!("- {} [{}]", task.name, task.status));
            }
        }
        if !snapshot.blocked_tasks.is_empty() {
            lines.push("Blocked tasks:".to_string());
            for task in snapshot.blocked_tasks.iter().take(5) {
                lines.push(format!("- {} [{}]", task.name, task.status));
            }
        }
        if !snapshot.missing_requirements.is_empty() {
            lines.push(format!(
                "Missing runtime completion requirements: {}",
                snapshot.missing_requirements.join(", ")
            ));
        }
        lines.join("\n")
    }

    pub(super) fn prompt_requires_build_and_test(prompt: &str) -> bool {
        let normalized = prompt.to_ascii_lowercase();
        ["build and test", "test and build", "build & test"]
            .iter()
            .any(|needle| normalized.contains(needle))
    }

    pub(super) fn request_requires_mutating_file_tool_success(request: &str) -> bool {
        let normalized = request.trim().to_ascii_lowercase();
        if normalized.is_empty() || normalized.starts_with('/') {
            return false;
        }

        let always_mutating_verbs = [
            "rewrite", "edit", "update", "modify", "replace", "delete", "rename", "move",
            "refactor", "fix", "scaffold",
        ];
        if always_mutating_verbs
            .iter()
            .any(|needle| normalized.contains(needle))
        {
            return true;
        }

        let ambiguous_mutation_verbs = ["write", "create", "add", "remove", "change", "implement"];
        if !ambiguous_mutation_verbs
            .iter()
            .any(|needle| normalized.contains(needle))
        {
            return false;
        }

        [
            "file",
            "files",
            "code",
            "codebase",
            "source file",
            "source files",
            "source code",
            "project",
            "repo",
            "repository",
            "workspace",
            "feature",
            "bug",
            "ui",
            "frontend",
            "backend",
            "endpoint",
            "page",
            "screen",
            "function",
            "class",
            "module",
            "component",
            "crate",
            "test",
            "tests",
            "readme",
            "cargo.toml",
            "package.json",
            "markdown file",
            "md file",
            "save it",
            "save the",
            "save to",
            "write to",
            ".rs",
            ".ts",
            ".tsx",
            ".js",
            ".jsx",
            ".py",
            ".md",
            ".toml",
            ".json",
            ".yaml",
            ".yml",
            "src/",
            "crates/",
            "frontend/",
            "docs/",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }

    fn extract_shell_command(tool_call: &ToolCallRecord) -> Option<String> {
        if tool_call.name != "shell" || !matches!(tool_call.result, ToolResult::Success(_)) {
            return None;
        }

        Self::extract_shell_command_from_record_arguments(&tool_call.arguments)
    }

    fn extract_shell_command_from_record_arguments(arguments: &str) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|value| {
                value
                    .get("command")
                    .and_then(|command| command.as_str())
                    .map(str::to_string)
            })
            .or_else(|| (!arguments.trim().is_empty()).then(|| arguments.to_string()))
    }

    fn normalize_shell_command(command: &str) -> String {
        command
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    }

    fn is_non_mutating_shell_probe_command(command: &str) -> bool {
        let normalized = Self::normalize_shell_command(command);
        let tokens = normalized.split_whitespace().collect::<Vec<_>>();

        tokens.iter().any(|token| {
            matches!(
                *token,
                "--help"
                    | "-h"
                    | "help"
                    | "--version"
                    | "-v"
                    | "version"
                    | "--dry-run"
                    | "--dryrun"
                    | "dry-run"
                    | "--no-run"
            )
        })
    }

    fn is_scaffold_or_init_shell_command_text(command: &str) -> bool {
        if Self::is_non_mutating_shell_probe_command(command) {
            return false;
        }

        let normalized = Self::normalize_shell_command(command);
        [
            "npx create-",
            "npm create",
            "pnpm create",
            "yarn create",
            "bun create",
            "cargo new",
            "cargo init",
            "cargo generate",
            "dotnet new",
            "rails new",
            "django-admin startproject",
            "django-admin startapp",
            "poetry new",
            "uv init",
            "composer create-project",
            "ng new",
            "nuxi init",
            "flutter create",
            "gradle init",
            "./gradlew init",
            "mvn archetype:generate",
            "mix phx.new",
            "degit ",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
    }

    fn is_build_or_check_command(command: &str) -> bool {
        let normalized = Self::normalize_shell_command(command);
        [
            "cargo check",
            "cargo build",
            "cargo tauri build",
            "npm run build",
            "npm run check",
            "npm run compile",
            "npm run verify",
            "pnpm run build",
            "pnpm run check",
            "pnpm run compile",
            "pnpm build",
            "pnpm check",
            "pnpm compile",
            "yarn build",
            "yarn run build",
            "yarn check",
            "yarn run check",
            "yarn compile",
            "yarn run compile",
            "bun build",
            "bun run build",
            "bun run check",
            "bun run compile",
            "tauri build",
            "vite build",
            "next build",
            "nuxt build",
            "astro build",
            "ng build",
            "nx build",
            "turbo build",
            "python -m build",
            "python -m compileall",
            "uv build",
            "poetry build",
            "go build",
            "go vet",
            "dotnet build",
            "dotnet publish",
            "mvn compile",
            "mvn package",
            "mvn verify",
            "mvn install",
            "gradle build",
            "gradle assemble",
            "gradle check",
            "./gradlew build",
            "./gradlew assemble",
            "./gradlew check",
            "bazel build",
            "bazelisk build",
            "make build",
            "make check",
            "make compile",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
    }

    fn is_test_command(command: &str) -> bool {
        let normalized = Self::normalize_shell_command(command);
        [
            "cargo test",
            "npm test",
            "npm run test",
            "pnpm test",
            "pnpm run test",
            "yarn test",
            "yarn run test",
            "bun test",
            "bun run test",
            "pytest",
            "python -m pytest",
            "python -m unittest",
            "tox",
            "nox",
            "go test",
            "dotnet test",
            "mvn test",
            "gradle test",
            "./gradlew test",
            "phpunit",
            "deno test",
            "vitest",
            "jest",
            "playwright test",
            "cypress run",
            "nx test",
            "turbo test",
            "rspec",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
    }

    fn is_frontend_source_path(path: &str) -> bool {
        let normalized = path.trim().replace('\\', "/").to_ascii_lowercase();
        let is_frontend_extension = [
            ".js", ".jsx", ".ts", ".tsx", ".css", ".scss", ".sass", ".less", ".html", ".htm",
            ".vue", ".svelte",
        ]
        .iter()
        .any(|extension| normalized.ends_with(extension));

        is_frontend_extension
            && (normalized.contains("/frontend/")
                || normalized.contains("/src/")
                || normalized.starts_with("src/"))
    }

    fn source_paths_from_tool_call(tool_call: &ToolCallRecord) -> Vec<String> {
        match tool_call.name.as_str() {
            "file" | "write_file" | "edit_file" if Self::is_file_mutation_attempt(tool_call) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                else {
                    return Vec::new();
                };

                value
                    .get("path")
                    .and_then(|path| path.as_str())
                    .map(|path| vec![path.to_string()])
                    .unwrap_or_default()
            }
            name if crate::tools::registry::is_code_tool_name(name)
                && Self::is_code_mutation_attempt(tool_call) =>
            {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                else {
                    return Vec::new();
                };

                let mut paths = value
                    .get("path")
                    .and_then(|path| path.as_str())
                    .map(|path| vec![path.to_string()])
                    .unwrap_or_default();

                if let Some(edits) = value.get("edits").and_then(|edits| edits.as_array()) {
                    for edit in edits {
                        if let Some(path) = edit.get("path").and_then(|path| path.as_str()) {
                            paths.push(path.to_string());
                        }
                    }
                }

                paths
            }
            _ => Vec::new(),
        }
    }

    fn frontend_verification_required(tool_calls: &[ToolCallRecord]) -> bool {
        tool_calls.iter().any(|tool_call| {
            Self::source_paths_from_tool_call(tool_call)
                .iter()
                .any(|path| Self::is_frontend_source_path(path))
        })
    }

    fn is_frontend_capable_build_command(command: &str) -> bool {
        let normalized = Self::normalize_shell_command(command);
        Self::is_build_or_check_command(&normalized)
            && [
                "npm ",
                "pnpm ",
                "yarn ",
                "bun ",
                "vite ",
                "next ",
                "nuxt ",
                "astro ",
                "webpack ",
                "parcel ",
                "rollup ",
                "nx ",
                "turbo ",
                "tauri build",
                "cargo tauri build",
            ]
            .iter()
            .any(|marker| normalized.starts_with(marker) || normalized.contains(marker))
    }

    fn is_frontend_capable_test_command(command: &str) -> bool {
        let normalized = Self::normalize_shell_command(command);
        Self::is_test_command(&normalized)
            && [
                "npm test",
                "npm run test",
                "pnpm test",
                "pnpm run test",
                "yarn test",
                "yarn run test",
                "bun test",
                "bun run test",
                "vitest",
                "jest",
                "playwright test",
                "cypress run",
                "web-test-runner",
            ]
            .iter()
            .any(|marker| Self::shell_command_invokes_marker(&normalized, marker))
    }

    fn is_integrated_frontend_build_command(command: &str) -> bool {
        let normalized = Self::normalize_shell_command(command);
        [
            "cargo tauri build",
            "tauri build",
            "npm run tauri build",
            "pnpm tauri build",
            "pnpm run tauri build",
            "yarn tauri build",
            "yarn run tauri build",
            "bun tauri build",
            "bun run tauri build",
        ]
        .iter()
        .any(|marker| Self::shell_command_invokes_marker(&normalized, marker))
    }

    fn shell_command_invokes_marker(normalized_command: &str, marker: &str) -> bool {
        normalized_command == marker
            || normalized_command.starts_with(&format!("{marker} "))
            || normalized_command.contains(&format!(" && {marker}"))
            || normalized_command.contains(&format!(" ; {marker}"))
            || normalized_command.contains(&format!(" | {marker}"))
    }

    fn shell_command_masks_failure(command: &str) -> bool {
        let normalized = Self::normalize_shell_command(command);
        normalized.contains("|| true")
            || normalized.contains("|| :")
            || normalized.contains("|| exit 0")
    }

    fn concise_outcome_excerpt(text: &str) -> String {
        let condensed = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or(text)
            .trim();
        let mut excerpt = condensed.chars().take(120).collect::<String>();
        if condensed.chars().count() > 120 {
            excerpt.push('…');
        }
        excerpt
    }

    fn output_signals_blocker(output: &str) -> bool {
        let lower = output.to_ascii_lowercase();
        [
            "permission denied",
            "access denied",
            "rate limit",
            "timed out",
            "timeout",
            "not configured",
            "command not found",
            "no such file or directory",
            "unable to resolve host",
            "could not resolve host",
            "connection refused",
            "network is unreachable",
            "authentication required",
            "missing script",
            "not a tauri project",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    }

    fn is_http_probe_command(command: &str) -> bool {
        let normalized = Self::normalize_shell_command(command);
        ["curl", "wget", "http", "httpie"]
            .iter()
            .any(|marker| Self::shell_command_invokes_marker(&normalized, marker))
    }

    fn output_contains_http_failure_status(output: &str) -> bool {
        let lower = output.to_ascii_lowercase();
        [
            " 400 ",
            " 401 ",
            " 402 ",
            " 403 ",
            " 404 ",
            " 409 ",
            " 410 ",
            " 422 ",
            " 429 ",
            " 500 ",
            " 502 ",
            " 503 ",
            " 504 ",
            "404 not found",
            "500 internal server error",
            "502 bad gateway",
            "503 service unavailable",
            "504 gateway timeout",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    }

    fn output_contains_failure_markers(output: &str) -> bool {
        let lower = output.to_ascii_lowercase();
        [
            " failed",
            "failure",
            "failures:",
            "error:",
            " errors",
            "not ok",
            "panic",
            "missing script",
            "not found",
            "cannot navigate to invalid url",
            "expected ",
            "mismatch",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
    }

    fn shell_success_output_negative_summary(
        command: &str,
        output: &str,
    ) -> Option<(bool, String)> {
        let excerpt = Self::concise_outcome_excerpt(output);
        if Self::is_http_probe_command(command) && Self::output_contains_http_failure_status(output)
        {
            return Some((
                false,
                format!("probe observed an HTTP failure response: {excerpt}"),
            ));
        }

        let verification_like = Self::is_build_or_check_command(command)
            || Self::is_test_command(command)
            || Self::is_http_probe_command(command);
        if !verification_like {
            return None;
        }

        if Self::shell_command_masks_failure(command) {
            return Some((
                false,
                "composite verification command masked failures, so it does not count as trusted completion evidence"
                    .to_string(),
            ));
        }

        if Self::output_signals_blocker(output) {
            return Some((
                true,
                format!(
                    "successful command surfaced a blocker instead of completion evidence: {excerpt}"
                ),
            ));
        }

        if Self::output_contains_failure_markers(output) {
            return Some((
                false,
                format!("successful shell output still reported a failing outcome: {excerpt}"),
            ));
        }

        None
    }

    fn tool_call_blocker_summary(tool_call: &ToolCallRecord) -> Option<String> {
        match &tool_call.result {
            ToolResult::Skipped(output) => Some(format!(
                "{} was blocked: {}",
                tool_call.name,
                Self::concise_outcome_excerpt(output)
            )),
            ToolResult::Error(output) if Self::output_signals_blocker(output) => Some(format!(
                "{} was blocked: {}",
                tool_call.name,
                Self::concise_outcome_excerpt(output)
            )),
            ToolResult::Success(output) if tool_call.name == "shell" => {
                let command =
                    Self::extract_shell_command_from_record_arguments(&tool_call.arguments)?;
                Self::shell_success_output_negative_summary(&command, output)
                    .and_then(|(is_blocker, summary)| is_blocker.then_some(summary))
            }
            _ => None,
        }
    }

    fn tool_call_contradiction_summary(tool_call: &ToolCallRecord) -> Option<String> {
        match &tool_call.result {
            ToolResult::Error(output) if !Self::output_signals_blocker(output) => Some(format!(
                "{} contradicted the current path: {}",
                tool_call.name,
                Self::concise_outcome_excerpt(output)
            )),
            ToolResult::Success(output) if tool_call.name == "shell" => {
                let command =
                    Self::extract_shell_command_from_record_arguments(&tool_call.arguments)?;
                Self::shell_success_output_negative_summary(&command, output)
                    .and_then(|(is_blocker, summary)| (!is_blocker).then_some(summary))
            }
            _ => None,
        }
    }

    fn tool_call_effective_success(tool_call: &ToolCallRecord) -> bool {
        match &tool_call.result {
            ToolResult::Success(output) if tool_call.name == "shell" => {
                Self::extract_shell_command_from_record_arguments(&tool_call.arguments)
                    .and_then(|command| {
                        Self::shell_success_output_negative_summary(&command, output)
                    })
                    .is_none()
            }
            ToolResult::Success(_) => true,
            ToolResult::Error(_) | ToolResult::Skipped(_) => false,
        }
    }

    fn tool_call_counts_as_diagnostic_progress(tool_call: &ToolCallRecord) -> bool {
        if Self::tool_call_effective_success(tool_call) {
            return !Self::is_task_tool_name(&tool_call.name);
        }

        Self::tool_call_contradiction_summary(tool_call).is_some()
            || Self::tool_call_blocker_summary(tool_call).is_some()
    }

    fn required_build_verification_label(tool_calls: &[ToolCallRecord]) -> &'static str {
        let _ = tool_calls;
        "a successful build/check command appropriate for the changed part of the project"
    }

    fn build_and_test_completion_status(tool_calls: &[ToolCallRecord]) -> (bool, bool, bool, bool) {
        let frontend_verification_required = Self::frontend_verification_required(tool_calls);
        let mut build_attempted = false;
        let mut build_completed = false;
        let mut frontend_build_attempted = false;
        let mut frontend_build_completed = false;
        let mut integrated_frontend_build_completed = false;
        let mut test_attempted = false;
        let mut test_completed = false;
        let mut frontend_test_attempted = false;
        let mut frontend_test_completed = false;
        let mut general_test_attempted = false;
        let mut general_test_completed = false;

        for tool_call in tool_calls.iter() {
            if tool_call.name != "shell" {
                continue;
            }
            let Some(command) =
                Self::extract_shell_command_from_record_arguments(&tool_call.arguments)
            else {
                continue;
            };
            if Self::is_non_mutating_shell_probe_command(&command) {
                continue;
            }

            let success = Self::tool_call_effective_success(tool_call);

            if Self::is_build_or_check_command(&command) {
                build_attempted = true;
                if success && !frontend_verification_required {
                    build_completed = true;
                }

                if Self::is_frontend_capable_build_command(&command) {
                    frontend_build_attempted = true;
                    if success {
                        frontend_build_completed = true;
                    }
                    if success && frontend_verification_required {
                        build_completed = true;
                    }
                    if success && Self::is_integrated_frontend_build_command(&command) {
                        integrated_frontend_build_completed = true;
                    }
                }
            }

            if Self::is_test_command(&command) {
                test_attempted = true;
                general_test_attempted = true;
                if success {
                    general_test_completed = true;
                }
                if success && !frontend_verification_required {
                    test_completed = true;
                }

                if Self::is_frontend_capable_test_command(&command) {
                    frontend_test_attempted = true;
                    if success {
                        frontend_test_completed = true;
                    }
                    if success && frontend_verification_required {
                        test_completed = true;
                    }
                }
            }
        }

        if frontend_verification_required {
            build_attempted = frontend_build_attempted;
            build_completed = frontend_build_completed;
            test_attempted = frontend_test_attempted
                || (integrated_frontend_build_completed && general_test_attempted);
            test_completed = frontend_test_completed
                || (integrated_frontend_build_completed && general_test_completed);
        }

        (
            build_attempted,
            build_completed,
            test_attempted,
            test_completed,
        )
    }

    fn has_any_successful_non_task_tool_call(tool_calls: &[ToolCallRecord]) -> bool {
        tool_calls.iter().any(|tool_call| {
            !Self::is_task_tool_name(&tool_call.name)
                && Self::tool_call_effective_success(tool_call)
        })
    }

    fn is_successful_generic_verification_tool_call(tool_call: &ToolCallRecord) -> bool {
        if Self::is_task_tool_name(&tool_call.name) || !Self::tool_call_effective_success(tool_call)
        {
            return false;
        }

        if matches!(
            tool_call.name.as_str(),
            "read_file" | "web" | "web_search" | "code"
        ) {
            return true;
        }

        tool_call.name == "file"
            && matches!(
                Self::file_operation_for_suspension(tool_call).as_deref(),
                Some("read" | "search" | "list" | "tree")
            )
    }

    fn is_successful_external_verification_tool_call(tool_call: &ToolCallRecord) -> bool {
        matches!(tool_call.result, ToolResult::Success(_))
            && matches!(tool_call.name.as_str(), "web" | "web_search")
    }

    fn latest_generic_verification_tool_name(tool_calls: &[ToolCallRecord]) -> Option<String> {
        tool_calls
            .iter()
            .rev()
            .find(|tool_call| Self::is_successful_generic_verification_tool_call(tool_call))
            .map(|tool_call| tool_call.name.clone())
    }

    fn generic_verification_satisfies_task(
        task: &crate::Task,
        tool_calls: &[ToolCallRecord],
    ) -> bool {
        if !tool_calls
            .iter()
            .any(Self::is_successful_generic_verification_tool_call)
        {
            return false;
        }

        let profile = Self::task_execution_profile(task, false);
        if !profile.requires_external_evidence {
            return true;
        }

        let latest_mutation_index = tool_calls
            .iter()
            .enumerate()
            .filter_map(|(index, tool_call)| {
                (Self::is_successful_mutating_file_tool_call(tool_call)
                    || Self::is_successful_mutating_code_tool_call(tool_call)
                    || Self::is_successful_mutating_shell_tool_call(tool_call))
                .then_some(index)
            })
            .next_back();

        tool_calls.iter().enumerate().any(|(index, tool_call)| {
            Self::is_successful_external_verification_tool_call(tool_call)
                && latest_mutation_index.is_none_or(|mutation_index| index > mutation_index)
        })
    }

    fn is_successful_mutating_code_tool_call(tool_call: &ToolCallRecord) -> bool {
        if !crate::tools::registry::is_code_tool_name(&tool_call.name)
            || !matches!(tool_call.result, ToolResult::Success(_))
        {
            return false;
        }

        serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
            .ok()
            .and_then(|value| {
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
            })
            .unwrap_or(false)
    }

    fn is_successful_mutating_shell_tool_call(tool_call: &ToolCallRecord) -> bool {
        if tool_call.name != "shell" || !Self::tool_call_effective_success(tool_call) {
            return false;
        }

        let Some(command) = Self::extract_shell_command(tool_call) else {
            return false;
        };
        if Self::is_non_mutating_shell_probe_command(&command) {
            return false;
        }
        let normalized = Self::normalize_shell_command(&command);

        Self::is_scaffold_or_init_shell_command_text(&command)
            || [
                "cargo add ",
                "npm install",
                "npm create",
                "pnpm add",
                "pnpm create",
                "yarn add",
                "mkdir ",
                "touch ",
                "cp ",
                "mv ",
            ]
            .iter()
            .any(|marker| normalized.contains(marker))
    }

    #[allow(dead_code)]
    fn task_matches_phase(task: &crate::Task, keywords: &[&str]) -> bool {
        Self::task_text_contains_any(task, keywords)
    }

    #[allow(dead_code)]
    fn first_open_phase_task<'a>(
        tasks: &'a [crate::Task],
        keywords: &[&str],
    ) -> Option<&'a crate::Task> {
        tasks
            .iter()
            .find(|task| !task.is_terminal() && Self::task_matches_phase(task, keywords))
    }

    fn verification_command_signature(tool_call: &ToolCallRecord) -> Option<String> {
        if tool_call.name != "shell" || !matches!(tool_call.result, ToolResult::Success(_)) {
            return None;
        }

        let command = Self::extract_shell_command(tool_call)?;
        let normalized = Self::normalize_shell_command(&command);

        if Self::is_build_or_check_command(&normalized) || Self::is_test_command(&normalized) {
            Some(normalized)
        } else {
            None
        }
    }

    fn trailing_repeated_successful_verification_command(
        tool_calls: &[ToolCallRecord],
        threshold: usize,
    ) -> Option<String> {
        let mut expected: Option<String> = None;
        let mut consecutive_matches = 0usize;

        for tool_call in tool_calls.iter().rev() {
            if let Some(signature) = Self::verification_command_signature(tool_call) {
                match expected.as_deref() {
                    None => {
                        expected = Some(signature);
                        consecutive_matches = 1;
                    }
                    Some(expected_signature) if expected_signature == signature => {
                        consecutive_matches += 1;
                    }
                    Some(_) => break,
                }

                if consecutive_matches >= threshold {
                    return expected;
                }

                continue;
            }

            if Self::is_task_tool_name(&tool_call.name)
                || matches!(tool_call.result, ToolResult::Skipped(_))
            {
                continue;
            }

            break;
        }

        None
    }

    fn is_missing_requested_build_and_test(
        requires_build_and_test: bool,
        tool_calls: &[ToolCallRecord],
    ) -> bool {
        if !requires_build_and_test {
            return false;
        }

        let (_, build_completed, _, test_completed) =
            Self::build_and_test_completion_status(tool_calls);
        !(build_completed && test_completed)
    }

    fn should_force_initial_execution_without_tools(
        saw_any_tool_calls: bool,
        tools_available: bool,
        requires_build_and_test: bool,
        tracked_task: bool,
        iteration_content: &str,
        iteration: usize,
        max_iterations: Option<usize>,
    ) -> bool {
        !saw_any_tool_calls
            && tools_available
            && (requires_build_and_test || tracked_task)
            && !Self::text_signals_user_blocker_or_question(iteration_content)
            && Self::has_iteration_headroom(iteration, max_iterations)
    }

    fn text_defers_remaining_work(text: &str) -> bool {
        let normalized = text.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        [
            "remaining:",
            "remaining work",
            "still in progress",
            "next unresolved step",
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

    fn text_signals_failed_or_incomplete_work(text: &str) -> bool {
        let normalized = text.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        [
            "unable to",
            "failed to",
            "could not",
            "did not",
            "no changes were made",
            "task is incomplete",
            "work is incomplete",
            "incomplete",
            "not completed",
            "not complete",
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

    #[allow(dead_code)]
    fn text_signals_completed_work(text: &str) -> bool {
        let normalized = text.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        [
            "completed",
            "complete",
            "finished",
            "done",
            "implemented",
            "updated",
            "rewrote",
            "modified",
            "verified",
            "tests passed",
            "build succeeded",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }

    fn text_signals_broad_plan_completion(text: &str) -> bool {
        let normalized = text.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        [
            "all planned deliverables",
            "all planned subtasks",
            "all planned tasks",
            "all planned work",
            "all requested steps",
            "all requested tasks",
            "everything requested is complete",
            "everything requested is now complete",
            "all deliverables are now finished",
            "all deliverables are finished",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
    }

    fn should_finalize_completed_tool_iteration(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        iteration_content: &str,
        all_tool_calls: &[ToolCallRecord],
        iteration_tool_calls: &[ToolCallRecord],
        open_descendant_summary: OpenDescendantSummary,
        _task_tool_suspended: bool,
    ) -> bool {
        if iteration_tool_calls.is_empty()
            || !Self::has_meaningful_final_text(iteration_content)
            || Self::text_signals_user_blocker_or_question(iteration_content)
            || Self::text_signals_failed_or_incomplete_work(iteration_content)
            || Self::text_defers_remaining_work(iteration_content)
            || Self::is_missing_requested_build_and_test(requires_build_and_test, all_tool_calls)
            || !Self::tool_results_support_successful_completion(
                requires_mutating_file_tool_success,
                all_tool_calls,
            )
            || open_descendant_summary.has_open()
        {
            return false;
        }

        iteration_tool_calls
            .iter()
            .all(|tool_call| matches!(tool_call.result, ToolResult::Success(_)))
    }

    fn should_force_tool_free_final_summary_after_completion_ready_tool_iteration(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        iteration_content: &str,
        all_tool_calls: &[ToolCallRecord],
        iteration_tool_calls: &[ToolCallRecord],
        runtime_state: Option<&TrackedTaskRuntimeState>,
        open_descendant_summary: OpenDescendantSummary,
    ) -> bool {
        let Some(runtime_state) = runtime_state else {
            return false;
        };

        runtime_state.completion_ready
            && !open_descendant_summary.has_open()
            && !iteration_tool_calls.is_empty()
            && !Self::has_meaningful_final_text(iteration_content)
            && !Self::text_signals_user_blocker_or_question(iteration_content)
            && !Self::text_signals_failed_or_incomplete_work(iteration_content)
            && !Self::text_defers_remaining_work(iteration_content)
            && !Self::is_missing_requested_build_and_test(requires_build_and_test, all_tool_calls)
            && Self::tool_results_support_successful_completion(
                requires_mutating_file_tool_success,
                all_tool_calls,
            )
    }

    fn is_any_loop_breaker_skip(tool_call: &ToolCallRecord) -> bool {
        matches!(
            &tool_call.result,
            ToolResult::Skipped(message) if message.contains("Loop breaker:")
        )
    }

    fn file_operation_for_suspension(tool_call: &ToolCallRecord) -> Option<String> {
        match tool_call.name.as_str() {
            "read_file" => return Some("read".to_string()),
            "write_file" => return Some("write".to_string()),
            "edit_file" => return Some("edit".to_string()),
            "file" => {}
            _ => return None,
        }

        serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
            .ok()
            .and_then(|value| {
                value
                    .get("operation")
                    .and_then(|operation| operation.as_str())
                    .map(|operation| operation.trim().to_ascii_lowercase())
            })
    }

    fn is_successful_file_mutation(tool_call: &ToolCallRecord) -> bool {
        Self::is_successful_mutating_file_tool_call(tool_call)
    }

    fn is_file_mutation_attempt(tool_call: &ToolCallRecord) -> bool {
        matches!(
            Self::file_operation_for_suspension(tool_call).as_deref(),
            Some("write" | "edit")
        )
    }

    fn is_malformed_file_mutation_attempt(tool_call: &ToolCallRecord) -> bool {
        let Some(operation) = Self::file_operation_for_suspension(tool_call) else {
            return false;
        };

        match (&*operation, &tool_call.result) {
            (
                "write",
                ToolResult::Error(message) | ToolResult::Skipped(message),
            ) => {
                message.contains("Missing required field 'content' for file write operation")
                    || message.contains(
                        "Loop breaker: skipped a repeated malformed `file.write` call without `content`",
                    )
            }
            (
                "edit",
                ToolResult::Error(message) | ToolResult::Skipped(message),
            ) => {
                message.contains("Missing required field 'old' for file edit operation")
                    || message.contains("Missing required field 'new' for file edit operation")
                    || message.contains(
                        "Loop breaker: skipped a repeated malformed `file.edit` call without valid `old`/`new` replacement text",
                    )
            }
            _ => false,
        }
    }

    fn extract_file_read_path_for_loop_detection(tool_call: &ToolCallRecord) -> Option<String> {
        match tool_call.name.as_str() {
            "read_file" => {}
            "file" => {}
            _ => return None,
        }

        let args = serde_json::from_str::<serde_json::Value>(&tool_call.arguments).ok()?;
        let operation = args
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| {
                if args.get("content").is_some() {
                    "write"
                } else {
                    "read"
                }
            })
            .trim()
            .trim_matches(|ch| matches!(ch, '"' | '\''))
            .to_ascii_lowercase();

        if operation != "read" {
            return None;
        }

        Some(
            args.get("path")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(".")
                .trim_matches(|ch| matches!(ch, '"' | '\''))
                .to_string(),
        )
    }

    fn extract_code_batch_read_signature_for_loop_detection(
        tool_call: &ToolCallRecord,
    ) -> Option<String> {
        if tool_call.name != "code" {
            return None;
        }

        let args = serde_json::from_str::<serde_json::Value>(&tool_call.arguments).ok()?;
        let operation = args
            .get("operation")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .trim()
            .trim_matches(|ch| matches!(ch, '"' | '\''))
            .to_ascii_lowercase();

        if operation != "batch_read" {
            return None;
        }

        let mut paths = args
            .get("paths")?
            .as_array()?
            .iter()
            .filter_map(|value| value.as_str())
            .map(|path| {
                path.trim()
                    .trim_matches(|ch| matches!(ch, '"' | '\''))
                    .to_string()
            })
            .filter(|path| !path.is_empty())
            .collect::<Vec<_>>();

        if paths.is_empty() {
            return None;
        }

        paths.sort();
        Some(format!("code.batch_read:{}", paths.join("|")))
    }

    fn low_value_inspection_signature(tool_call: &ToolCallRecord) -> Option<String> {
        if !matches!(tool_call.result, ToolResult::Success(_)) {
            return None;
        }

        if tool_call.name == "read_file" {
            return Self::extract_file_read_path_for_loop_detection(tool_call)
                .map(|path| format!("file.read:{path}"));
        }

        if tool_call.name == "file" {
            return Self::extract_file_read_path_for_loop_detection(tool_call)
                .map(|path| format!("file.read:{path}"));
        }

        if tool_call.name == "code" {
            return Self::extract_code_batch_read_signature_for_loop_detection(tool_call);
        }

        if tool_call.name == "shell" {
            let command = Self::extract_shell_command(tool_call)?;
            let normalized = command
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            if ["cat ", "head ", "tail ", "sed ", "grep ", "wc "]
                .iter()
                .any(|prefix| normalized.starts_with(prefix))
            {
                return Some(format!("shell.inspect:{normalized}"));
            }
        }

        None
    }

    fn has_repeated_identical_low_value_inspection_calls(
        all_tool_calls: &[ToolCallRecord],
        iteration_tool_calls: &[ToolCallRecord],
        threshold: usize,
    ) -> bool {
        if iteration_tool_calls.len() != 1 {
            return false;
        }

        let Some(signature) = Self::low_value_inspection_signature(&iteration_tool_calls[0]) else {
            return false;
        };

        let mut consecutive_matches = 0usize;
        for tool_call in all_tool_calls.iter().rev() {
            if Self::low_value_inspection_signature(tool_call).as_deref()
                == Some(signature.as_str())
            {
                consecutive_matches += 1;
                if consecutive_matches >= threshold {
                    return true;
                }
                continue;
            }

            break;
        }

        false
    }

    fn has_stalled_low_value_inspection_streak(
        all_tool_calls: &[ToolCallRecord],
        iteration_tool_calls: &[ToolCallRecord],
        consecutive_nonterminal_tool_iterations: usize,
    ) -> bool {
        Self::has_repeated_identical_low_value_inspection_calls(
            all_tool_calls,
            iteration_tool_calls,
            3,
        ) || (consecutive_nonterminal_tool_iterations >= 5
            && iteration_tool_calls.len() == 1
            && Self::low_value_inspection_signature(&iteration_tool_calls[0]).is_some())
    }

    fn has_recent_successful_verification_command(
        tool_calls: &[ToolCallRecord],
        lookback: usize,
    ) -> bool {
        tool_calls.iter().rev().take(lookback).any(|tool_call| {
            matches!(tool_call.result, ToolResult::Success(_))
                && Self::verification_command_signature(tool_call).is_some()
        })
    }

    fn iteration_contains_only_successful_low_value_inspection(
        iteration_tool_calls: &[ToolCallRecord],
    ) -> bool {
        !iteration_tool_calls.is_empty()
            && iteration_tool_calls
                .iter()
                .all(|tool_call| Self::low_value_inspection_signature(tool_call).is_some())
    }

    fn should_force_tool_free_final_summary_after_stalled_tool_loop(
        requires_build_and_test: bool,
        iteration_content: &str,
        all_tool_calls: &[ToolCallRecord],
        iteration_tool_calls: &[ToolCallRecord],
        open_descendant_summary: OpenDescendantSummary,
        suspension_state: ToolSuspensionState,
        consecutive_nonterminal_tool_iterations: usize,
    ) -> bool {
        let saw_post_verification_read_only_follow_up = requires_build_and_test
            && !Self::is_missing_requested_build_and_test(requires_build_and_test, all_tool_calls)
            && !open_descendant_summary.has_open()
            && Self::has_recent_successful_verification_command(all_tool_calls, 4)
            && Self::iteration_contains_only_successful_low_value_inspection(iteration_tool_calls);

        if iteration_tool_calls.is_empty()
            || (consecutive_nonterminal_tool_iterations < 3
                && !saw_post_verification_read_only_follow_up)
            || Self::has_meaningful_final_text(iteration_content)
            || Self::text_signals_user_blocker_or_question(iteration_content)
            || Self::text_defers_remaining_work(iteration_content)
            || Self::is_missing_requested_build_and_test(requires_build_and_test, all_tool_calls)
            || open_descendant_summary.has_open()
        {
            return false;
        }

        let saw_loop_breaker_skip = all_tool_calls.iter().any(Self::is_any_loop_breaker_skip)
            || iteration_tool_calls
                .iter()
                .any(Self::is_any_loop_breaker_skip);
        let saw_stalled_low_value_inspection = Self::has_stalled_low_value_inspection_streak(
            all_tool_calls,
            iteration_tool_calls,
            consecutive_nonterminal_tool_iterations,
        );
        let saw_generic_nonterminal_tool_streak = consecutive_nonterminal_tool_iterations >= 5;

        if !(suspension_state.task
            || suspension_state.file
            || suspension_state.code
            || saw_loop_breaker_skip
            || saw_stalled_low_value_inspection
            || saw_generic_nonterminal_tool_streak
            || saw_post_verification_read_only_follow_up)
        {
            return false;
        }

        iteration_tool_calls
            .iter()
            .all(|tool_call| !matches!(tool_call.result, ToolResult::Error(_)))
    }

    fn should_force_required_verification_after_stalled_tool_loop(
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        iteration_content: &str,
        all_tool_calls: &[ToolCallRecord],
        iteration_tool_calls: &[ToolCallRecord],
        open_descendant_summary: OpenDescendantSummary,
        consecutive_nonterminal_tool_iterations: usize,
    ) -> bool {
        let saw_repeated_verification_command =
            Self::trailing_repeated_successful_verification_command(all_tool_calls, 2).is_some();
        let saw_stalled_low_value_inspection = Self::has_stalled_low_value_inspection_streak(
            all_tool_calls,
            iteration_tool_calls,
            consecutive_nonterminal_tool_iterations,
        );
        let saw_generic_nonterminal_tool_streak = consecutive_nonterminal_tool_iterations >= 5;

        requires_build_and_test
            && !iteration_tool_calls.is_empty()
            && consecutive_nonterminal_tool_iterations >= 3
            && !Self::has_meaningful_final_text(iteration_content)
            && !Self::text_signals_user_blocker_or_question(iteration_content)
            && !Self::text_defers_remaining_work(iteration_content)
            && Self::is_missing_requested_build_and_test(requires_build_and_test, all_tool_calls)
            && (!requires_mutating_file_tool_success
                || all_tool_calls
                    .iter()
                    .any(Self::is_successful_mutating_file_tool_call)
                || all_tool_calls
                    .iter()
                    .any(Self::is_successful_mutating_code_tool_call))
            && !open_descendant_summary.has_open()
            && (saw_repeated_verification_command
                || saw_stalled_low_value_inspection
                || saw_generic_nonterminal_tool_streak)
            && iteration_tool_calls
                .iter()
                .all(|tool_call| !matches!(tool_call.result, ToolResult::Error(_)))
    }

    fn should_force_mutating_execution_after_stalled_inspection(
        requires_mutating_file_tool_success: bool,
        iteration_content: &str,
        all_tool_calls: &[ToolCallRecord],
        iteration_tool_calls: &[ToolCallRecord],
        consecutive_nonterminal_tool_iterations: usize,
    ) -> bool {
        requires_mutating_file_tool_success
            && !iteration_tool_calls.is_empty()
            && consecutive_nonterminal_tool_iterations >= 3
            && !Self::has_meaningful_final_text(iteration_content)
            && !Self::text_signals_user_blocker_or_question(iteration_content)
            && !Self::text_defers_remaining_work(iteration_content)
            && !all_tool_calls
                .iter()
                .any(Self::is_successful_mutating_file_tool_call)
            && Self::iteration_contains_only_successful_low_value_inspection(iteration_tool_calls)
            && Self::has_stalled_low_value_inspection_streak(
                all_tool_calls,
                iteration_tool_calls,
                consecutive_nonterminal_tool_iterations,
            )
            && iteration_tool_calls
                .iter()
                .all(|tool_call| !matches!(tool_call.result, ToolResult::Error(_)))
    }

    fn should_force_open_subtask_continuation(input: OpenSubtaskContinuationInput<'_>) -> bool {
        input.saw_any_tool_calls
            && input.open_descendant_summary.has_open()
            && !input.task_tool_suspended
            && !Self::text_signals_user_blocker_or_question(input.iteration_content)
            && Self::has_iteration_headroom(input.iteration, input.max_iterations)
    }

    fn should_force_deferred_tracked_work_continuation(
        saw_any_tool_calls: bool,
        open_descendant_summary: OpenDescendantSummary,
        task_tool_suspended: bool,
        iteration_content: &str,
        iteration: usize,
        max_iterations: Option<usize>,
    ) -> bool {
        saw_any_tool_calls
            && open_descendant_summary.has_open()
            && !task_tool_suspended
            && Self::text_defers_remaining_work(iteration_content)
            && Self::has_iteration_headroom(iteration, max_iterations)
    }

    fn should_force_meaningful_incomplete_tracked_work_continuation(
        saw_any_tool_calls: bool,
        runtime_state: Option<&TrackedTaskRuntimeState>,
        task_tool_suspended: bool,
        iteration_content: &str,
        iteration: usize,
        max_iterations: Option<usize>,
    ) -> bool {
        saw_any_tool_calls
            && !task_tool_suspended
            && runtime_state.is_some_and(|state| {
                !state.completion_ready
                    && Self::runtime_snapshot_has_incomplete_tracked_work(&state.snapshot)
            })
            && Self::has_meaningful_final_text(iteration_content)
            && (Self::text_signals_failed_or_incomplete_work(iteration_content)
                || Self::text_defers_remaining_work(iteration_content))
            && !Self::text_signals_user_blocker_or_question(iteration_content)
            && Self::has_iteration_headroom(iteration, max_iterations)
    }

    #[allow(clippy::too_many_arguments)]
    fn should_escalate_no_tool_open_subtask_stall(
        saw_any_tool_calls: bool,
        terminal_text_is_meaningful: bool,
        iteration_content: &str,
        open_descendant_summary: OpenDescendantSummary,
        task_tool_suspended: bool,
        forced_final_summary_requested: bool,
        stagnant_no_tool_open_subtask_streak: usize,
        iteration: usize,
        max_iterations: Option<usize>,
    ) -> bool {
        let stall_threshold = if Self::text_signals_completed_work(iteration_content)
            || Self::text_defers_remaining_work(iteration_content)
            || Self::text_signals_failed_or_incomplete_work(iteration_content)
        {
            2
        } else {
            4
        };

        saw_any_tool_calls
            && terminal_text_is_meaningful
            && open_descendant_summary.has_open()
            && !task_tool_suspended
            && !forced_final_summary_requested
            && stagnant_no_tool_open_subtask_streak >= stall_threshold
            && Self::has_iteration_headroom(iteration, max_iterations)
    }

    #[allow(clippy::too_many_arguments)]
    fn with_stagnation_recovery_instruction(
        current_prompt: &str,
        stagnant_iteration_streak: usize,
        stagnation_summary: &str,
        missing_requirements: &[String],
    ) -> String {
        let mut prompt = current_prompt.to_string();
        prompt.push_str(&format!(
            "\nUser: The last {} tool iterations produced materially similar outcomes without changing the runtime progress state. The run appears stalled. On the next step, choose one materially different action that is likely to change the workspace, verification status, or blocker state. Do not keep re-running the same action shape or re-reviewing the same unchanged result. If no materially different action is available, stop and give a concise blocker or final status update.\nObserved stall summary: {}\n",
            stagnant_iteration_streak,
            stagnation_summary
        ));
        prompt.push_str(
            "If the repeated outcome is a contradiction or blocker, do not spend another turn merely confirming it. Either take a materially different step that could resolve it, or stop and explain the unresolved blocker/diagnosis clearly.\n",
        );
        if !missing_requirements.is_empty() {
            prompt.push_str(&format!(
                "Unchanged runtime requirements: {}\n",
                missing_requirements.join(", ")
            ));
        }
        prompt
    }

    fn exhausted_iteration_budget(
        iterations_used: usize,
        max_iterations: Option<usize>,
    ) -> Option<usize> {
        max_iterations.filter(|limit| iterations_used >= *limit)
    }

    fn describe_tool_call_for_summary(&self, tool_call: &ToolCallRecord) -> String {
        let focus_suffix = Self::public_tool_focus_phrase(
            tool_call.name.as_str(),
            Some(tool_call.arguments.as_str()),
        )
        .unwrap_or_default();
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
                format!(
                    "Last tool `{}` succeeded ({action}{focus_suffix}).",
                    tool_call.name
                )
            }
            ToolResult::Error(_) => format!(
                "Last tool `{}` failed while trying to {}{}.",
                tool_call.name, action, focus_suffix
            ),
            ToolResult::Skipped(_) => format!(
                "Last tool `{}` was skipped while trying to {}{}.",
                tool_call.name, action, focus_suffix
            ),
        }
    }

    pub(super) fn can_parallelize_read_only_tool_call(name: &str, arguments: &str) -> bool {
        if crate::tools::policy::is_write_operation(name, arguments) {
            return false;
        }

        let operation = serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|value| {
                value
                    .get("operation")
                    .and_then(|operation| operation.as_str())
                    .map(|operation| operation.trim().to_ascii_lowercase())
            });

        match (name, operation.as_deref()) {
            ("web", _) | ("web_search", _) | ("read_file", _) => true,
            ("file", Some("read" | "list" | "tree" | "search")) => true,
            (
                "code",
                Some(
                    "stats" | "map" | "symbols" | "references" | "definition" | "deps" | "glob"
                    | "grep" | "batch_read" | "outline",
                ),
            ) => true,
            _ if name.starts_with("code_") => true,
            _ => false,
        }
    }

    pub(super) async fn execute_parallel_read_only_tool_batch(
        &self,
        batch: Vec<gestura_core_llm::ToolCallInfo>,
        workspace: Option<&SessionWorkspace>,
    ) -> Vec<ToolCallRecord> {
        let mut results = stream::iter(batch.into_iter().enumerate().map(
            |(index, tool_call)| async move {
                let result = self
                    .execute_tool(&tool_call.name, &tool_call.arguments, workspace, None, None)
                    .await;
                (
                    index,
                    ToolCallRecord {
                        id: tool_call.id,
                        name: tool_call.name,
                        arguments: tool_call.arguments,
                        result,
                        duration_ms: 0,
                    },
                )
            },
        ))
        .buffer_unordered(MAX_PARALLEL_READ_ONLY_TOOL_CALLS)
        .collect::<Vec<_>>()
        .await;

        results.sort_by_key(|(index, _)| *index);
        results.into_iter().map(|(_, record)| record).collect()
    }

    /// Execute the agentic loop with streaming
    ///
    /// If `workspace` is provided, all tool operations (shell, file, git) will be
    /// sandboxed to that directory. Paths outside the workspace will be rejected.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_agentic_loop_streaming(
        &self,
        initial_prompt: String,
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
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
        telemetry: &AgentRequestTelemetry,
    ) -> Result<AgentResponse, AppError> {
        Self::mark_tracked_task_in_progress_async(session_id.as_deref(), task_id.as_deref()).await;

        let mut response = Self::initial_loop_response(context);

        let mut current_prompt = initial_prompt;

        // Build provider-specific tool schemas once for this request.
        //
        // IMPORTANT: MCP tool schemas are only included when the pipeline has decided
        // they are relevant/allowed for this request. This prevents unrelated MCP
        // servers from delaying or destabilizing requests that only need built-in tools.
        let tool_schemas = self
            .build_request_tool_schemas(&tools, include_mcp_tool_schemas)
            .await;
        tracing::debug!(
            builtin_tool_count = tools.len(),
            has_schemas = tool_schemas.is_some(),
            "[AgentLoop] Tool schemas initialized"
        );

        let mut saw_any_tool_calls = false;
        let mut forced_execution_after_empty_response = false;
        let mut forced_execution_after_stalled_inspection = false;
        let mut forced_final_summary_requested = false;
        let mut force_required_verification_retry = false;
        let mut force_tool_free_final_summary = false;
        let mut consecutive_nonterminal_tool_iterations = 0usize;
        let mut stagnant_tool_iteration_streak = 0usize;
        let mut last_tool_iteration_fingerprint: Option<ToolIterationStagnationFingerprint> = None;
        let mut stagnant_no_tool_open_subtask_streak = 0usize;
        let mut last_no_tool_open_subtask_fingerprint: Option<String> = None;
        let mut delivered_terminal_summary = false;
        let mut last_runtime_task_snapshot: Option<crate::streaming::TaskRuntimeSnapshot> = None;
        let mut last_public_narration = PublicNarrationState::default();

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
                Self::cancel_tracked_task_async(
                    session_id.as_deref(),
                    task_id.as_deref(),
                    "cancel token raised before iteration",
                )
                .await;
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

            let runtime_state = Self::resolve_runtime_state(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                session_id.as_deref(),
                task_id.as_deref(),
                &response.tool_calls,
            )
            .await;
            if let Some(state) = runtime_state.tracked.as_ref() {
                Self::emit_task_runtime_snapshot_if_changed(
                    &tx,
                    &state.snapshot,
                    &mut last_runtime_task_snapshot,
                );
            }

            // Start streaming for this iteration
            let (inner_tx, mut inner_rx) =
                mpsc::channel::<StreamChunk>(super::STREAM_CHUNK_BUFFER_CAPACITY);
            let inner_cancel = cancel_token.clone();
            let streaming_cfg = crate::streaming::streaming_config_from(&self.config);
            let enable_fallback = self.pipeline_config.enable_fallback;
            let required_verification_retry_pending = force_required_verification_retry;
            force_required_verification_retry = false;
            let PreparedLoopIteration {
                required_verification_retry_pending,
                task_tool_suspended,
                file_tool_suspended,
                code_tool_suspended,
                prompt,
                active_tool_schemas,
            } = self
                .prepare_loop_iteration(
                    iteration,
                    max_iterations,
                    &current_prompt,
                    &tool_schemas,
                    &response.tool_calls,
                    required_verification_retry_pending,
                    force_tool_free_final_summary,
                    telemetry,
                )
                .await;

            // Spawn streaming task (with or without fallback)
            let iteration_stream_span = tracing::info_span!(
                "agent.pipeline.iteration",
                iteration = iteration,
                task_tool_suspended = task_tool_suspended,
                file_tool_suspended = file_tool_suspended,
                code_tool_suspended = code_tool_suspended
            );
            let stream_handle = tokio::spawn(
                async move {
                    if enable_fallback {
                        start_streaming_with_fallback(
                            &streaming_cfg,
                            &prompt,
                            active_tool_schemas,
                            inner_tx,
                            inner_cancel,
                        )
                        .await
                    } else {
                        start_streaming(
                            &streaming_cfg,
                            &prompt,
                            active_tool_schemas,
                            inner_tx,
                            inner_cancel,
                        )
                        .await
                    }
                }
                .instrument(iteration_stream_span),
            );

            tracing::debug!(
                iteration = iteration,
                "[AgentLoop] Streaming task spawned; consuming inner chunks"
            );

            // Collect chunks and forward to caller
            let mut iteration_content = String::new();
            let mut buffered_iteration_text = String::new();
            let mut pending_tool_call: Option<PendingToolCall> = None;
            let mut tool_calls_in_iteration: Vec<ToolCallRecord> = Vec::new();
            let mut saw_tool_call_chunk_in_iteration = false;

            while let Some(chunk) = inner_rx.recv().await {
                match &chunk {
                    StreamChunk::Status { .. } => {
                        // Forward status updates to frontend
                        Self::forward_status_chunk_best_effort(&tx, chunk).await;
                    }
                    StreamChunk::Text(text) => {
                        iteration_content.push_str(text);
                        if saw_tool_call_chunk_in_iteration {
                            response.content.push_str(text);
                            let _ = tx.send(chunk).await;
                        } else {
                            buffered_iteration_text.push_str(text);
                        }
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
                    StreamChunk::Narration { narration, stage } => {
                        Self::emit_narration_if_changed(
                            &tx,
                            *stage,
                            narration.clone(),
                            format!(
                                "stream:{}:{}",
                                stage.as_str(),
                                Self::public_narration_payload_fingerprint(narration)
                            ),
                            &mut last_public_narration,
                        );
                    }
                    StreamChunk::ToolCallStart { id, name } => {
                        Self::flush_buffered_iteration_text(
                            &tx,
                            &mut response,
                            &mut buffered_iteration_text,
                        )
                        .await;
                        saw_tool_call_chunk_in_iteration = true;
                        tracing::debug!(tool = %name, id = %id, "[AgentLoop] ToolCallStart received");
                        // Defensive: if the provider starts a new tool call without ending the
                        // previous one, finalize the previous call so we don't drop it.
                        if let Some(pending) = pending_tool_call.take() {
                            tracing::debug!(
                                tool = %pending.name,
                                "[AgentLoop] Defensive finalize of previous pending tool call"
                            );
                            let previous_tool_call_count = tool_calls_in_iteration.len();
                            self.finalize_pending_tool_call(
                                pending,
                                FinalizePendingToolCallCtx {
                                    workspace,
                                    session_id: session_id.clone(),
                                    permission_level,
                                    required_verification_retry_pending,
                                    cancel_token: &cancel_token,
                                    tool_calls_in_iteration: &mut tool_calls_in_iteration,
                                    response: &mut response,
                                    tx: &tx,
                                },
                            )
                            .await;
                            telemetry
                                .record_tool_calls(
                                    iteration,
                                    &tool_calls_in_iteration[previous_tool_call_count..],
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
                            self.maybe_emit_llm_public_narration(
                                &tx,
                                PublicNarrationTrigger::BatchStart,
                                Some(pending.name.as_str()),
                                Some(pending.arguments.as_str()),
                                last_runtime_task_snapshot.as_ref(),
                                &[],
                                &mut last_public_narration,
                            )
                            .await;

                            let tool_name_log = pending.name.clone();
                            let args_len_log = pending.arguments.len();
                            tracing::debug!(
                                tool = %tool_name_log,
                                args_len = args_len_log,
                                "[AgentLoop] ToolCallEnd: calling finalize_pending_tool_call"
                            );
                            let previous_tool_call_count = tool_calls_in_iteration.len();
                            self.finalize_pending_tool_call(
                                pending,
                                FinalizePendingToolCallCtx {
                                    workspace,
                                    session_id: session_id.clone(),
                                    permission_level,
                                    required_verification_retry_pending,
                                    cancel_token: &cancel_token,
                                    tool_calls_in_iteration: &mut tool_calls_in_iteration,
                                    response: &mut response,
                                    tx: &tx,
                                },
                            )
                            .await;
                            telemetry
                                .record_tool_calls(
                                    iteration,
                                    &tool_calls_in_iteration[previous_tool_call_count..],
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
                        super::send_token_usage_chunk_best_effort(&tx, chunk).await;
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
                    StreamChunk::TaskRuntimeSnapshot { .. } => {
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
                    StreamChunk::ShellSessionLifecycle { .. } => {
                        // Forward interactive shell session lifecycle events to frontend
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
                            let previous_tool_call_count = tool_calls_in_iteration.len();
                            self.finalize_pending_tool_call(
                                pending,
                                FinalizePendingToolCallCtx {
                                    workspace,
                                    session_id: session_id.clone(),
                                    permission_level,
                                    required_verification_retry_pending,
                                    cancel_token: &cancel_token,
                                    tool_calls_in_iteration: &mut tool_calls_in_iteration,
                                    response: &mut response,
                                    tx: &tx,
                                },
                            )
                            .await;
                            telemetry
                                .record_tool_calls(
                                    iteration,
                                    &tool_calls_in_iteration[previous_tool_call_count..],
                                )
                                .await;
                        }

                        if let Some(u) = usage {
                            response.usage = Some(u.clone());
                        }
                    }
                    StreamChunk::Error(e) => {
                        Self::cancel_tracked_task_async(
                            session_id.as_deref(),
                            task_id.as_deref(),
                            "provider emitted error chunk",
                        )
                        .await;
                        tracing::error!(error = %e, iteration = iteration, "[AgentLoop] Error chunk received from inner stream");
                        let _ = tx.send(StreamChunk::Error(e.clone())).await;
                        return Err(AppError::Llm(e.clone()));
                    }
                    StreamChunk::Cancelled => {
                        Self::cancel_tracked_task_async(
                            session_id.as_deref(),
                            task_id.as_deref(),
                            "provider emitted cancelled chunk",
                        )
                        .await;
                        tracing::debug!(
                            iteration = iteration,
                            "[AgentLoop] Cancelled chunk — aborting loop"
                        );
                        let _ = tx.send(chunk).await;
                        telemetry.mark_outcome(RequestOutcome::Cancelled);
                        return Ok(response);
                    }
                    StreamChunk::Paused => {
                        tracing::debug!(
                            iteration = iteration,
                            "[AgentLoop] Paused chunk — suspending loop"
                        );
                        let _ = tx.send(chunk).await;
                        telemetry.mark_outcome(RequestOutcome::Paused);
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
                let previous_tool_call_count = tool_calls_in_iteration.len();
                self.finalize_pending_tool_call(
                    pending,
                    FinalizePendingToolCallCtx {
                        workspace,
                        session_id: session_id.clone(),
                        permission_level,
                        required_verification_retry_pending,
                        cancel_token: &cancel_token,
                        tool_calls_in_iteration: &mut tool_calls_in_iteration,
                        response: &mut response,
                        tx: &tx,
                    },
                )
                .await;
                telemetry
                    .record_tool_calls(
                        iteration,
                        &tool_calls_in_iteration[previous_tool_call_count..],
                    )
                    .await;
            }

            // Wait for stream task
            Self::join_stream_task_after_channel_close(iteration, stream_handle).await;

            tracing::debug!(
                iteration = iteration,
                tool_calls_count = tool_calls_in_iteration.len(),
                "[AgentLoop] Stream task joined"
            );

            let task_tool_suspended = Self::should_suspend_task_tool(&response.tool_calls);

            // If no tool calls, we're done unless we still owe the user a closing summary.
            if tool_calls_in_iteration.is_empty() {
                consecutive_nonterminal_tool_iterations = 0;
                stagnant_tool_iteration_streak = 0;
                last_tool_iteration_fingerprint = None;
                let terminal_text_is_meaningful =
                    Self::has_meaningful_final_text(&iteration_content);
                if Self::should_force_initial_execution_without_tools(
                    saw_any_tool_calls,
                    !tools.is_empty(),
                    requires_build_and_test,
                    task_id.is_some(),
                    &iteration_content,
                    iteration,
                    max_iterations,
                ) {
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::InitialExecutionRequired,
                        )
                        .await;
                    self.maybe_emit_no_tool_continuation_narration(
                        &tx,
                        last_runtime_task_snapshot.as_ref(),
                        &mut last_public_narration,
                        &buffered_iteration_text,
                    )
                    .await;
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    iteration += 1;
                    continue;
                }
                if saw_any_tool_calls
                    && requires_build_and_test
                    && Self::is_missing_requested_build_and_test(
                        requires_build_and_test,
                        &response.tool_calls,
                    )
                    && !Self::text_signals_user_blocker_or_question(&iteration_content)
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::RequiredVerificationPending,
                        )
                        .await;
                    self.maybe_emit_no_tool_continuation_narration(
                        &tx,
                        last_runtime_task_snapshot.as_ref(),
                        &mut last_public_narration,
                        &buffered_iteration_text,
                    )
                    .await;
                    current_prompt = self.build_required_verification_prompt(
                        &current_prompt,
                        &response.content,
                        &response.tool_calls,
                    );
                    force_required_verification_retry = true;
                    iteration += 1;
                    continue;
                }
                let runtime_state = Self::resolve_runtime_state(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    session_id.as_deref(),
                    task_id.as_deref(),
                    &response.tool_calls,
                )
                .await;
                if let Some(state) = runtime_state.tracked.as_ref() {
                    Self::emit_task_runtime_snapshot_if_changed(
                        &tx,
                        &state.snapshot,
                        &mut last_runtime_task_snapshot,
                    );
                }
                let open_descendant_summary = runtime_state.open_descendant_summary;
                if let Some(fingerprint) = Self::no_tool_open_subtask_fingerprint(
                    runtime_state.tracked.as_ref(),
                    open_descendant_summary,
                ) {
                    Self::update_stagnation_streak(
                        fingerprint,
                        &mut last_no_tool_open_subtask_fingerprint,
                        &mut stagnant_no_tool_open_subtask_streak,
                    );
                } else {
                    stagnant_no_tool_open_subtask_streak = 0;
                    last_no_tool_open_subtask_fingerprint = None;
                }
                if Self::should_escalate_no_tool_open_subtask_stall(
                    saw_any_tool_calls,
                    terminal_text_is_meaningful,
                    &iteration_content,
                    open_descendant_summary,
                    task_tool_suspended,
                    forced_final_summary_requested,
                    stagnant_no_tool_open_subtask_streak,
                    iteration,
                    max_iterations,
                ) {
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::ForcedFinalSummary,
                        )
                        .await;
                    tracing::warn!(
                        iteration = iteration,
                        stagnant_no_tool_open_subtask_streak = stagnant_no_tool_open_subtask_streak,
                        "[AgentLoop] Repeated no-tool responses left the same tracked subtasks open — escalating to forced in-progress/final status prompt"
                    );
                    current_prompt = self.build_forced_final_summary_prompt(
                        &current_prompt,
                        &response.content,
                        requires_build_and_test,
                        requires_mutating_file_tool_success,
                        &response.tool_calls,
                        runtime_state
                            .tracked
                            .as_ref()
                            .map(|state| state.snapshot.missing_requirements.as_slice())
                            .unwrap_or(&[]),
                        open_descendant_summary,
                    );
                    forced_final_summary_requested = true;
                    iteration += 1;
                    continue;
                }
                if !forced_final_summary_requested
                    && Self::should_force_open_subtask_continuation(OpenSubtaskContinuationInput {
                        saw_any_tool_calls,
                        open_descendant_summary,
                        task_tool_suspended,
                        iteration_content: &iteration_content,
                        iteration,
                        max_iterations,
                    })
                {
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::OpenSubtasks,
                        )
                        .await;
                    tracing::warn!(
                        iteration = iteration,
                        "[AgentLoop] Tracked task still has open subtasks after a no-tool response — forcing execution continuation"
                    );
                    Self::restore_execution_mode_after_forced_summary(
                        &mut force_tool_free_final_summary,
                        &mut forced_execution_after_empty_response,
                        &mut forced_final_summary_requested,
                    );
                    self.maybe_emit_no_tool_continuation_narration(
                        &tx,
                        runtime_state.tracked.as_ref().map(|state| &state.snapshot),
                        &mut last_public_narration,
                        &buffered_iteration_text,
                    )
                    .await;
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    iteration += 1;
                    continue;
                }

                if !forced_final_summary_requested
                    && Self::should_force_deferred_tracked_work_continuation(
                        saw_any_tool_calls,
                        open_descendant_summary,
                        task_tool_suspended,
                        &iteration_content,
                        iteration,
                        max_iterations,
                    )
                {
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::DeferredTrackedWork,
                        )
                        .await;
                    tracing::warn!(
                        iteration = iteration,
                        "[AgentLoop] Terminal status update deferred remaining tracked task work — forcing execution continuation"
                    );
                    Self::restore_execution_mode_after_forced_summary(
                        &mut force_tool_free_final_summary,
                        &mut forced_execution_after_empty_response,
                        &mut forced_final_summary_requested,
                    );
                    self.maybe_emit_no_tool_continuation_narration(
                        &tx,
                        runtime_state.tracked.as_ref().map(|state| &state.snapshot),
                        &mut last_public_narration,
                        &buffered_iteration_text,
                    )
                    .await;
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    iteration += 1;
                    continue;
                }

                if !forced_final_summary_requested
                    && Self::should_force_meaningful_incomplete_tracked_work_continuation(
                        saw_any_tool_calls,
                        runtime_state.tracked.as_ref(),
                        task_tool_suspended,
                        &iteration_content,
                        iteration,
                        max_iterations,
                    )
                {
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::DeferredTrackedWork,
                        )
                        .await;
                    tracing::warn!(
                        iteration = iteration,
                        "[AgentLoop] Meaningful no-tool summary still reports incomplete tracked work — forcing execution continuation"
                    );
                    Self::restore_execution_mode_after_forced_summary(
                        &mut force_tool_free_final_summary,
                        &mut forced_execution_after_empty_response,
                        &mut forced_final_summary_requested,
                    );
                    self.maybe_emit_no_tool_continuation_narration(
                        &tx,
                        runtime_state.tracked.as_ref().map(|state| &state.snapshot),
                        &mut last_public_narration,
                        &buffered_iteration_text,
                    )
                    .await;
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    iteration += 1;
                    continue;
                }

                if !forced_final_summary_requested
                    && Self::should_force_meaningful_incomplete_tracked_work_continuation(
                        saw_any_tool_calls,
                        runtime_state.tracked.as_ref(),
                        task_tool_suspended,
                        &iteration_content,
                        iteration,
                        max_iterations,
                    )
                {
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::DeferredTrackedWork,
                        )
                        .await;
                    Self::restore_execution_mode_after_forced_summary(
                        &mut force_tool_free_final_summary,
                        &mut forced_execution_after_empty_response,
                        &mut forced_final_summary_requested,
                    );
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    iteration += 1;
                    continue;
                }

                if saw_any_tool_calls
                    && !terminal_text_is_meaningful
                    && !forced_execution_after_empty_response
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::EmptyTerminalRetry,
                        )
                        .await;
                    tracing::warn!(
                        iteration = iteration,
                        "[AgentLoop] Empty/non-substantive terminal iteration after tool use — forcing execution continuation before summary"
                    );
                    Self::restore_execution_mode_after_forced_summary(
                        &mut force_tool_free_final_summary,
                        &mut forced_execution_after_empty_response,
                        &mut forced_final_summary_requested,
                    );
                    self.maybe_emit_no_tool_continuation_narration(
                        &tx,
                        runtime_state.tracked.as_ref().map(|state| &state.snapshot),
                        &mut last_public_narration,
                        &buffered_iteration_text,
                    )
                    .await;
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    forced_execution_after_empty_response = true;
                    iteration += 1;
                    continue;
                }

                if saw_any_tool_calls
                    && !terminal_text_is_meaningful
                    && !forced_final_summary_requested
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    let completion_ready = runtime_state
                        .tracked
                        .as_ref()
                        .is_some_and(|state| state.completion_ready);
                    telemetry
                        .record_iteration_completed(
                            iteration,
                            0,
                            iteration_content.chars().count(),
                            false,
                        )
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::ForcedFinalSummary,
                        )
                        .await;
                    tracing::warn!(
                        iteration = iteration,
                        "[AgentLoop] Empty/non-substantive terminal iteration after tool use — forcing one final summary attempt"
                    );
                    self.maybe_emit_no_tool_continuation_narration(
                        &tx,
                        runtime_state.tracked.as_ref().map(|state| &state.snapshot),
                        &mut last_public_narration,
                        &buffered_iteration_text,
                    )
                    .await;
                    current_prompt = if completion_ready {
                        self.build_tool_free_final_summary_prompt(
                            &current_prompt,
                            &response.content,
                            requires_build_and_test,
                            requires_mutating_file_tool_success,
                            &response.tool_calls,
                            runtime_state
                                .tracked
                                .as_ref()
                                .map(|state| state.snapshot.missing_requirements.as_slice())
                                .unwrap_or(&[]),
                            open_descendant_summary,
                        )
                    } else {
                        self.build_forced_final_summary_prompt(
                            &current_prompt,
                            &response.content,
                            requires_build_and_test,
                            requires_mutating_file_tool_success,
                            &response.tool_calls,
                            runtime_state
                                .tracked
                                .as_ref()
                                .map(|state| state.snapshot.missing_requirements.as_slice())
                                .unwrap_or(&[]),
                            open_descendant_summary,
                        )
                    };
                    if completion_ready {
                        force_tool_free_final_summary = true;
                        forced_execution_after_empty_response = true;
                    }
                    forced_final_summary_requested = true;
                    iteration += 1;
                    continue;
                }

                tracing::debug!(
                    iteration = iteration,
                    "[AgentLoop] No tool calls in iteration — breaking loop"
                );
                Self::flush_buffered_iteration_text(
                    &tx,
                    &mut response,
                    &mut buffered_iteration_text,
                )
                .await;
                delivered_terminal_summary = terminal_text_is_meaningful;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        0,
                        iteration_content.chars().count(),
                        delivered_terminal_summary,
                    )
                    .await;
                break;
            }

            saw_any_tool_calls = true;
            forced_execution_after_empty_response = false;
            forced_final_summary_requested = false;
            consecutive_nonterminal_tool_iterations += 1;

            let mut combined_tool_calls = response.tool_calls.clone();
            combined_tool_calls.extend(tool_calls_in_iteration.clone());
            let runtime_state = Self::resolve_runtime_state(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                session_id.as_deref(),
                task_id.as_deref(),
                &combined_tool_calls,
            )
            .await;
            if let Some(state) = runtime_state.tracked.as_ref() {
                Self::emit_task_runtime_snapshot_if_changed(
                    &tx,
                    &state.snapshot,
                    &mut last_runtime_task_snapshot,
                );
            }
            let open_descendant_summary = runtime_state.open_descendant_summary;
            let has_open_descendants = open_descendant_summary.has_open();
            let stagnation_fingerprint = Self::tool_iteration_stagnation_fingerprint(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                &tool_calls_in_iteration,
                runtime_state.tracked.as_ref(),
            );
            Self::update_stagnation_streak(
                stagnation_fingerprint.clone(),
                &mut last_tool_iteration_fingerprint,
                &mut stagnant_tool_iteration_streak,
            );
            let stagnation_summary =
                Self::summarize_stagnation_fingerprint(&stagnation_fingerprint);

            if Self::should_finalize_completed_tool_iteration(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                &iteration_content,
                &combined_tool_calls,
                &tool_calls_in_iteration,
                open_descendant_summary,
                task_tool_suspended,
            ) {
                tracing::info!(
                    iteration = iteration,
                    tool_calls_count = tool_calls_in_iteration.len(),
                    "[AgentLoop] Meaningful completion text accompanied successful tool calls — accepting iteration as terminal"
                );
                delivered_terminal_summary = true;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        tool_calls_in_iteration.len(),
                        iteration_content.chars().count(),
                        true,
                    )
                    .await;
                break;
            }

            let file_tool_suspended = Self::should_suspend_file_tool(&combined_tool_calls);
            let code_tool_suspended = Self::should_suspend_code_tool(&combined_tool_calls);

            self.maybe_emit_llm_public_narration(
                &tx,
                PublicNarrationTrigger::ResultsReview,
                None,
                None,
                runtime_state.tracked.as_ref().map(|state| &state.snapshot),
                &tool_calls_in_iteration,
                &mut last_public_narration,
            )
            .await;

            let should_force_mutating_execution = !forced_execution_after_stalled_inspection
                && Self::should_force_mutating_execution_after_stalled_inspection(
                    requires_mutating_file_tool_success,
                    &iteration_content,
                    &combined_tool_calls,
                    &tool_calls_in_iteration,
                    consecutive_nonterminal_tool_iterations,
                );

            if should_force_mutating_execution
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    tool_calls_count = tool_calls_in_iteration.len(),
                    consecutive_nonterminal_tool_iterations =
                        consecutive_nonterminal_tool_iterations,
                    low_value_inspection_signature = tool_calls_in_iteration
                        .first()
                        .and_then(Self::low_value_inspection_signature),
                    "[AgentLoop] Request still needs a successful file mutation after a stalled read-only inspection loop — forcing concrete execution retry"
                );
                telemetry
                    .record_iteration_completed(
                        iteration,
                        tool_calls_in_iteration.len(),
                        iteration_content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::InitialExecutionRequired,
                    )
                    .await;
                current_prompt = self
                    .build_stalled_mutation_execution_prompt_async(
                        &current_prompt,
                        &response.content,
                        session_id.as_deref(),
                        task_id.as_deref(),
                    )
                    .await;
                forced_execution_after_stalled_inspection = true;
                iteration += 1;
                continue;
            }

            if stagnant_tool_iteration_streak >= 2
                && requires_build_and_test
                && Self::is_missing_requested_build_and_test(
                    requires_build_and_test,
                    &combined_tool_calls,
                )
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    stagnant_tool_iteration_streak = stagnant_tool_iteration_streak,
                    stagnation_summary = %stagnation_summary,
                    "[AgentLoop] Generic stagnation detector observed repeated no-progress tool outcomes while required verification is still missing — forcing verification retry"
                );
                telemetry
                    .record_iteration_completed(
                        iteration,
                        tool_calls_in_iteration.len(),
                        iteration_content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::RequiredVerificationPending,
                    )
                    .await;
                current_prompt = self.build_required_verification_prompt(
                    &current_prompt,
                    &response.content,
                    &combined_tool_calls,
                );
                current_prompt = Self::with_stagnation_recovery_instruction(
                    &current_prompt,
                    stagnant_tool_iteration_streak,
                    &stagnation_summary,
                    &stagnation_fingerprint.missing_requirements,
                );
                force_required_verification_retry = true;
                iteration += 1;
                continue;
            }

            let should_force_required_verification =
                Self::should_force_required_verification_after_stalled_tool_loop(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &iteration_content,
                    &combined_tool_calls,
                    &tool_calls_in_iteration,
                    open_descendant_summary,
                    consecutive_nonterminal_tool_iterations,
                );

            if should_force_required_verification
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    tool_calls_count = tool_calls_in_iteration.len(),
                    consecutive_nonterminal_tool_iterations =
                        consecutive_nonterminal_tool_iterations,
                    repeated_verification_command = ?Self::trailing_repeated_successful_verification_command(
                        &combined_tool_calls,
                        2,
                    ),
                    low_value_inspection_signature = tool_calls_in_iteration
                        .first()
                        .and_then(Self::low_value_inspection_signature),
                    "[AgentLoop] Missing required build/test after a stalled inspection or repeated verification loop — forcing remaining-verification retry"
                );
                telemetry
                    .record_iteration_completed(
                        iteration,
                        tool_calls_in_iteration.len(),
                        iteration_content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::RequiredVerificationPending,
                    )
                    .await;
                current_prompt = self.build_required_verification_prompt(
                    &current_prompt,
                    &response.content,
                    &combined_tool_calls,
                );
                force_required_verification_retry = true;
                iteration += 1;
                continue;
            }

            let should_force_tool_free_final_summary =
                Self::should_force_tool_free_final_summary_after_stalled_tool_loop(
                    requires_build_and_test,
                    &iteration_content,
                    &combined_tool_calls,
                    &tool_calls_in_iteration,
                    open_descendant_summary,
                    ToolSuspensionState {
                        task: task_tool_suspended,
                        file: file_tool_suspended,
                        code: code_tool_suspended,
                    },
                    consecutive_nonterminal_tool_iterations,
                );

            let should_force_completion_ready_final_summary =
                Self::should_force_tool_free_final_summary_after_completion_ready_tool_iteration(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &iteration_content,
                    &combined_tool_calls,
                    &tool_calls_in_iteration,
                    runtime_state.tracked.as_ref(),
                    open_descendant_summary,
                );

            if should_force_completion_ready_final_summary
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::info!(
                    iteration = iteration,
                    tool_calls_count = tool_calls_in_iteration.len(),
                    "[AgentLoop] Tool iteration completed the tracked runtime without a usable terminal summary — forcing tool-free closeout"
                );
                telemetry
                    .record_iteration_completed(
                        iteration,
                        tool_calls_in_iteration.len(),
                        iteration_content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::ForcedFinalSummary,
                    )
                    .await;
                current_prompt = self.build_tool_free_final_summary_prompt(
                    &current_prompt,
                    &response.content,
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &response.tool_calls,
                    runtime_state
                        .tracked
                        .as_ref()
                        .map(|state| state.snapshot.missing_requirements.as_slice())
                        .unwrap_or(&[]),
                    open_descendant_summary,
                );
                force_tool_free_final_summary = true;
                forced_execution_after_empty_response = true;
                forced_final_summary_requested = true;
                iteration += 1;
                continue;
            }

            if should_force_tool_free_final_summary
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    tool_calls_count = tool_calls_in_iteration.len(),
                    consecutive_nonterminal_tool_iterations =
                        consecutive_nonterminal_tool_iterations,
                    "[AgentLoop] Stalled tool-only loop — forcing tool-free final summary attempt"
                );
                telemetry
                    .record_iteration_completed(
                        iteration,
                        tool_calls_in_iteration.len(),
                        iteration_content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::ForcedFinalSummary,
                    )
                    .await;
                current_prompt = self.build_tool_free_final_summary_prompt(
                    &current_prompt,
                    &response.content,
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &response.tool_calls,
                    runtime_state
                        .tracked
                        .as_ref()
                        .map(|state| state.snapshot.missing_requirements.as_slice())
                        .unwrap_or(&[]),
                    open_descendant_summary,
                );
                force_tool_free_final_summary = true;
                forced_execution_after_empty_response = true;
                forced_final_summary_requested = true;
                iteration += 1;
                continue;
            }

            if stagnant_tool_iteration_streak >= 3
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    stagnant_tool_iteration_streak = stagnant_tool_iteration_streak,
                    stagnation_summary = %stagnation_summary,
                    "[AgentLoop] Generic stagnation detector observed repeated no-progress tool outcomes — forcing tool-free final summary attempt"
                );
                telemetry
                    .record_iteration_completed(
                        iteration,
                        tool_calls_in_iteration.len(),
                        iteration_content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::ForcedFinalSummary,
                    )
                    .await;
                current_prompt = self.build_tool_free_final_summary_prompt(
                    &current_prompt,
                    &response.content,
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &combined_tool_calls,
                    runtime_state
                        .tracked
                        .as_ref()
                        .map(|state| state.snapshot.missing_requirements.as_slice())
                        .unwrap_or(&[]),
                    open_descendant_summary,
                );
                current_prompt = Self::with_stagnation_recovery_instruction(
                    &current_prompt,
                    stagnant_tool_iteration_streak,
                    &stagnation_summary,
                    &stagnation_fingerprint.missing_requirements,
                );
                force_tool_free_final_summary = true;
                forced_execution_after_empty_response = true;
                forced_final_summary_requested = true;
                iteration += 1;
                continue;
            }

            if consecutive_nonterminal_tool_iterations >= 5 {
                tracing::warn!(
                    iteration = iteration,
                    tool_calls_count = tool_calls_in_iteration.len(),
                    consecutive_nonterminal_tool_iterations =
                        consecutive_nonterminal_tool_iterations,
                    has_open_descendants = has_open_descendants,
                    task_tool_suspended = task_tool_suspended,
                    file_tool_suspended = file_tool_suspended,
                    code_tool_suspended = code_tool_suspended,
                    requires_build_and_test = requires_build_and_test,
                    current_iteration_tool = tool_calls_in_iteration
                        .first()
                        .map(|tool_call| tool_call.name.as_str())
                        .unwrap_or("multiple"),
                    should_force_tool_free_final_summary = should_force_tool_free_final_summary,
                    "[AgentLoop] Continuing with tool_results after a long silent tool streak"
                );
            }

            telemetry
                .record_iteration_completed(
                    iteration,
                    tool_calls_in_iteration.len(),
                    iteration_content.chars().count(),
                    false,
                )
                .await;
            telemetry
                .record_iteration_continuation(iteration, AgentLoopContinuation::ToolResults)
                .await;

            // Build continuation prompt with tool results
            current_prompt = self.build_tool_continuation_prompt(
                &current_prompt,
                &iteration_content,
                &tool_calls_in_iteration,
            );
            if stagnant_tool_iteration_streak >= 2 {
                current_prompt = Self::with_stagnation_recovery_instruction(
                    &current_prompt,
                    stagnant_tool_iteration_streak,
                    &stagnation_summary,
                    &stagnation_fingerprint.missing_requirements,
                );
            }
            iteration += 1;
        }

        self.finalize_agent_loop_response(
            &mut response,
            saw_any_tool_calls,
            delivered_terminal_summary,
            max_iterations,
            requires_build_and_test,
            requires_mutating_file_tool_success,
            session_id.as_deref(),
            task_id.as_deref(),
            telemetry,
            Some(&tx),
        )
        .await;

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
        requires_build_and_test: bool,
        requires_mutating_file_tool_success: bool,
        tools: Vec<&'static ToolDefinition>,
        include_mcp_tool_schemas: bool,
        context: crate::context::ResolvedContext,
        workspace: Option<&SessionWorkspace>,
        session_id: Option<String>,
        task_id: Option<String>,
        max_iterations: Option<usize>,
        telemetry: &AgentRequestTelemetry,
    ) -> Result<AgentResponse, AppError> {
        Self::mark_tracked_task_in_progress_async(session_id.as_deref(), task_id.as_deref()).await;

        let mut response = Self::initial_loop_response(context);

        // Build provider-specific tool schemas so the model knows about available tools.
        // MCP schemas are only included when relevant/allowed.
        let tool_schemas = self
            .build_request_tool_schemas(&tools, include_mcp_tool_schemas)
            .await;
        let mut current_prompt = initial_prompt;
        let mut saw_any_tool_calls = false;
        let mut forced_execution_after_empty_response = false;
        let mut forced_execution_after_stalled_inspection = false;
        let mut forced_final_summary_requested = false;
        let mut force_required_verification_retry = false;
        let mut force_tool_free_final_summary = false;
        let mut consecutive_nonterminal_tool_iterations = 0usize;
        let mut stagnant_tool_iteration_streak = 0usize;
        let mut last_tool_iteration_fingerprint: Option<ToolIterationStagnationFingerprint> = None;
        let mut stagnant_no_tool_open_subtask_streak = 0usize;
        let mut last_no_tool_open_subtask_fingerprint: Option<String> = None;
        let mut delivered_terminal_summary = false;
        let mut _last_runtime_task_snapshot: Option<crate::streaming::TaskRuntimeSnapshot> = None;

        let mut iteration = 0usize;
        loop {
            if let Some(limit) = max_iterations
                && iteration >= limit
            {
                break;
            }

            response.iterations = iteration + 1;

            // Call LLM with fallback support, passing tool schemas.
            let required_verification_retry_pending = force_required_verification_retry;
            force_required_verification_retry = false;
            let PreparedLoopIteration {
                required_verification_retry_pending,
                task_tool_suspended,
                file_tool_suspended: _,
                code_tool_suspended: _,
                prompt,
                active_tool_schemas,
            } = self
                .prepare_loop_iteration(
                    iteration,
                    max_iterations,
                    &current_prompt,
                    &tool_schemas,
                    &response.tool_calls,
                    required_verification_retry_pending,
                    force_tool_free_final_summary,
                    telemetry,
                )
                .await;
            let llm_response = match self
                .call_llm_with_fallback(&prompt, active_tool_schemas.as_ref())
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    Self::cancel_tracked_task_async(
                        session_id.as_deref(),
                        task_id.as_deref(),
                        "blocking LLM call failed",
                    )
                    .await;
                    return Err(error);
                }
            };
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
                consecutive_nonterminal_tool_iterations = 0;
                stagnant_tool_iteration_streak = 0;
                last_tool_iteration_fingerprint = None;
                let terminal_text_is_meaningful = Self::has_meaningful_final_text(&content);
                if Self::should_force_initial_execution_without_tools(
                    saw_any_tool_calls,
                    !tools.is_empty(),
                    requires_build_and_test,
                    task_id.is_some(),
                    &content,
                    iteration,
                    max_iterations,
                ) {
                    telemetry
                        .record_iteration_completed(iteration, 0, content.chars().count(), false)
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::InitialExecutionRequired,
                        )
                        .await;
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    iteration += 1;
                    continue;
                }
                if saw_any_tool_calls
                    && requires_build_and_test
                    && Self::is_missing_requested_build_and_test(
                        requires_build_and_test,
                        &response.tool_calls,
                    )
                    && !Self::text_signals_user_blocker_or_question(&content)
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    telemetry
                        .record_iteration_completed(iteration, 0, content.chars().count(), false)
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::RequiredVerificationPending,
                        )
                        .await;
                    current_prompt = self.build_required_verification_prompt(
                        &current_prompt,
                        &response.content,
                        &response.tool_calls,
                    );
                    force_required_verification_retry = true;
                    iteration += 1;
                    continue;
                }
                let runtime_state = Self::resolve_runtime_state(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    session_id.as_deref(),
                    task_id.as_deref(),
                    &response.tool_calls,
                )
                .await;
                if let Some(state) = runtime_state.tracked.as_ref() {
                    _last_runtime_task_snapshot = Some(state.snapshot.clone());
                }
                let open_descendant_summary = runtime_state.open_descendant_summary;
                if let Some(fingerprint) = Self::no_tool_open_subtask_fingerprint(
                    runtime_state.tracked.as_ref(),
                    open_descendant_summary,
                ) {
                    Self::update_stagnation_streak(
                        fingerprint,
                        &mut last_no_tool_open_subtask_fingerprint,
                        &mut stagnant_no_tool_open_subtask_streak,
                    );
                } else {
                    stagnant_no_tool_open_subtask_streak = 0;
                    last_no_tool_open_subtask_fingerprint = None;
                }
                if Self::should_escalate_no_tool_open_subtask_stall(
                    saw_any_tool_calls,
                    terminal_text_is_meaningful,
                    &content,
                    open_descendant_summary,
                    task_tool_suspended,
                    forced_final_summary_requested,
                    stagnant_no_tool_open_subtask_streak,
                    iteration,
                    max_iterations,
                ) {
                    telemetry
                        .record_iteration_completed(iteration, 0, content.chars().count(), false)
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::ForcedFinalSummary,
                        )
                        .await;
                    tracing::warn!(
                        iteration = iteration,
                        stagnant_no_tool_open_subtask_streak = stagnant_no_tool_open_subtask_streak,
                        "Blocking loop: repeated no-tool responses left the same tracked subtasks open — escalating to forced in-progress/final status prompt"
                    );
                    current_prompt = self.build_forced_final_summary_prompt(
                        &current_prompt,
                        &response.content,
                        requires_build_and_test,
                        requires_mutating_file_tool_success,
                        &response.tool_calls,
                        runtime_state
                            .tracked
                            .as_ref()
                            .map(|state| state.snapshot.missing_requirements.as_slice())
                            .unwrap_or(&[]),
                        open_descendant_summary,
                    );
                    forced_final_summary_requested = true;
                    iteration += 1;
                    continue;
                }
                if !forced_final_summary_requested
                    && Self::should_force_open_subtask_continuation(OpenSubtaskContinuationInput {
                        saw_any_tool_calls,
                        open_descendant_summary,
                        task_tool_suspended,
                        iteration_content: &content,
                        iteration,
                        max_iterations,
                    })
                {
                    telemetry
                        .record_iteration_completed(iteration, 0, content.chars().count(), false)
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::OpenSubtasks,
                        )
                        .await;
                    Self::restore_execution_mode_after_forced_summary(
                        &mut force_tool_free_final_summary,
                        &mut forced_execution_after_empty_response,
                        &mut forced_final_summary_requested,
                    );
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    iteration += 1;
                    continue;
                }

                if !forced_final_summary_requested
                    && Self::should_force_deferred_tracked_work_continuation(
                        saw_any_tool_calls,
                        open_descendant_summary,
                        task_tool_suspended,
                        &content,
                        iteration,
                        max_iterations,
                    )
                {
                    telemetry
                        .record_iteration_completed(iteration, 0, content.chars().count(), false)
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::DeferredTrackedWork,
                        )
                        .await;
                    Self::restore_execution_mode_after_forced_summary(
                        &mut force_tool_free_final_summary,
                        &mut forced_execution_after_empty_response,
                        &mut forced_final_summary_requested,
                    );
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    iteration += 1;
                    continue;
                }

                if saw_any_tool_calls
                    && !terminal_text_is_meaningful
                    && !forced_execution_after_empty_response
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    telemetry
                        .record_iteration_completed(iteration, 0, content.chars().count(), false)
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::EmptyTerminalRetry,
                        )
                        .await;
                    Self::restore_execution_mode_after_forced_summary(
                        &mut force_tool_free_final_summary,
                        &mut forced_execution_after_empty_response,
                        &mut forced_final_summary_requested,
                    );
                    current_prompt = self
                        .build_forced_execution_prompt_async(
                            &current_prompt,
                            &response.content,
                            session_id.as_deref(),
                            task_id.as_deref(),
                        )
                        .await;
                    forced_execution_after_empty_response = true;
                    iteration += 1;
                    continue;
                }

                if saw_any_tool_calls
                    && !terminal_text_is_meaningful
                    && !forced_final_summary_requested
                    && Self::has_iteration_headroom(iteration, max_iterations)
                {
                    telemetry
                        .record_iteration_completed(iteration, 0, content.chars().count(), false)
                        .await;
                    telemetry
                        .record_iteration_continuation(
                            iteration,
                            AgentLoopContinuation::ForcedFinalSummary,
                        )
                        .await;
                    current_prompt = self.build_forced_final_summary_prompt(
                        &current_prompt,
                        &response.content,
                        requires_build_and_test,
                        requires_mutating_file_tool_success,
                        &response.tool_calls,
                        runtime_state
                            .tracked
                            .as_ref()
                            .map(|state| state.snapshot.missing_requirements.as_slice())
                            .unwrap_or(&[]),
                        open_descendant_summary,
                    );
                    forced_final_summary_requested = true;
                    iteration += 1;
                    continue;
                }

                response.content = content;
                response.thinking = thinking;
                delivered_terminal_summary = terminal_text_is_meaningful;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        0,
                        response.content.chars().count(),
                        delivered_terminal_summary,
                    )
                    .await;
                break;
            }

            saw_any_tool_calls = true;
            forced_execution_after_empty_response = false;
            forced_final_summary_requested = false;
            consecutive_nonterminal_tool_iterations += 1;
            stagnant_no_tool_open_subtask_streak = 0;
            last_no_tool_open_subtask_fingerprint = None;

            // Execute each structured tool call and collect records.
            let mut iteration_tool_calls: Vec<ToolCallRecord> = Vec::new();
            let mut pending_parallel_batch = Vec::new();
            let mut pending_parallel_signatures = HashSet::new();
            for tc in &llm_response.tool_calls {
                tracing::info!(
                    tool = %tc.name,
                    id = %tc.id,
                    "Blocking loop: executing tool call"
                );

                let parallel_signature = format!("{}\u{1f}{}", tc.name, tc.arguments);
                if Self::can_parallelize_read_only_tool_call(&tc.name, &tc.arguments)
                    && pending_parallel_signatures.contains(&parallel_signature)
                {
                    iteration_tool_calls.extend(
                        self.execute_parallel_read_only_tool_batch(
                            std::mem::take(&mut pending_parallel_batch),
                            workspace,
                        )
                        .await,
                    );
                    pending_parallel_signatures.clear();
                }

                let result = if required_verification_retry_pending && tc.name != "shell" {
                    if !pending_parallel_batch.is_empty() {
                        iteration_tool_calls.extend(
                            self.execute_parallel_read_only_tool_batch(
                                std::mem::take(&mut pending_parallel_batch),
                                workspace,
                            )
                            .await,
                        );
                        pending_parallel_signatures.clear();
                    }
                    tracing::warn!(
                        tool = %tc.name,
                        id = %tc.id,
                        "Blocking loop: required verification retry skipped non-shell tool call"
                    );
                    ToolResult::Skipped(format!(
                        "Required verification retry: skipped `{}`. During the forced build/test retry, only the `shell` tool may be used. Run a concrete non-interactive build/check/test command next instead of more inspection or bookkeeping.",
                        tc.name
                    ))
                } else if let Some(message) = Self::repeated_malformed_tool_call_skip_message(
                    &tc.name,
                    &tc.arguments,
                    response
                        .tool_calls
                        .iter()
                        .chain(iteration_tool_calls.iter()),
                ) {
                    if !pending_parallel_batch.is_empty() {
                        iteration_tool_calls.extend(
                            self.execute_parallel_read_only_tool_batch(
                                std::mem::take(&mut pending_parallel_batch),
                                workspace,
                            )
                            .await,
                        );
                        pending_parallel_signatures.clear();
                    }
                    tracing::warn!(
                        tool = %tc.name,
                        id = %tc.id,
                        "Blocking loop: loop breaker skipped repeated malformed tool call"
                    );
                    ToolResult::Skipped(message)
                } else if Self::can_parallelize_read_only_tool_call(&tc.name, &tc.arguments) {
                    pending_parallel_signatures.insert(parallel_signature);
                    pending_parallel_batch.push(tc.clone());
                    continue;
                } else {
                    if !pending_parallel_batch.is_empty() {
                        iteration_tool_calls.extend(
                            self.execute_parallel_read_only_tool_batch(
                                std::mem::take(&mut pending_parallel_batch),
                                workspace,
                            )
                            .await,
                        );
                        pending_parallel_signatures.clear();
                    }
                    self.execute_tool(
                        &tc.name,
                        &tc.arguments,
                        workspace,
                        session_id.as_deref(),
                        None,
                    )
                    .await
                };
                let duration_ms = 0u64; // No per-call timing in blocking path.
                iteration_tool_calls.push(ToolCallRecord {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                    result,
                    duration_ms,
                });
            }
            if !pending_parallel_batch.is_empty() {
                iteration_tool_calls.extend(
                    self.execute_parallel_read_only_tool_batch(pending_parallel_batch, workspace)
                        .await,
                );
            }
            telemetry
                .record_tool_calls(iteration, &iteration_tool_calls)
                .await;

            let mut combined_tool_calls = response.tool_calls.clone();
            combined_tool_calls.extend(iteration_tool_calls.clone());
            let runtime_state = Self::resolve_runtime_state(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                session_id.as_deref(),
                task_id.as_deref(),
                &combined_tool_calls,
            )
            .await;
            if let Some(state) = runtime_state.tracked.as_ref() {
                _last_runtime_task_snapshot = Some(state.snapshot.clone());
            }
            let open_descendant_summary = runtime_state.open_descendant_summary;
            let task_tool_suspended = Self::should_suspend_task_tool(&combined_tool_calls);
            let file_tool_suspended = Self::should_suspend_file_tool(&combined_tool_calls);
            let code_tool_suspended = Self::should_suspend_code_tool(&combined_tool_calls);
            let stagnation_fingerprint = Self::tool_iteration_stagnation_fingerprint(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                &iteration_tool_calls,
                runtime_state.tracked.as_ref(),
            );
            Self::update_stagnation_streak(
                stagnation_fingerprint.clone(),
                &mut last_tool_iteration_fingerprint,
                &mut stagnant_tool_iteration_streak,
            );
            let stagnation_summary =
                Self::summarize_stagnation_fingerprint(&stagnation_fingerprint);

            let should_force_mutating_execution = !forced_execution_after_stalled_inspection
                && Self::should_force_mutating_execution_after_stalled_inspection(
                    requires_mutating_file_tool_success,
                    &content,
                    &combined_tool_calls,
                    &iteration_tool_calls,
                    consecutive_nonterminal_tool_iterations,
                );

            if should_force_mutating_execution
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    tool_calls_count = iteration_tool_calls.len(),
                    consecutive_nonterminal_tool_iterations =
                        consecutive_nonterminal_tool_iterations,
                    low_value_inspection_signature = iteration_tool_calls
                        .first()
                        .and_then(Self::low_value_inspection_signature),
                    "Blocking loop: request still needs a successful file mutation after a stalled read-only inspection loop — forcing concrete execution retry"
                );
                response.tool_calls = combined_tool_calls;
                response.content = content;
                response.thinking = thinking;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        iteration_tool_calls.len(),
                        response.content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::InitialExecutionRequired,
                    )
                    .await;
                current_prompt = self
                    .build_stalled_mutation_execution_prompt_async(
                        &current_prompt,
                        &response.content,
                        session_id.as_deref(),
                        task_id.as_deref(),
                    )
                    .await;
                forced_execution_after_stalled_inspection = true;
                iteration += 1;
                continue;
            }

            if stagnant_tool_iteration_streak >= 2
                && requires_build_and_test
                && Self::is_missing_requested_build_and_test(
                    requires_build_and_test,
                    &combined_tool_calls,
                )
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    stagnant_tool_iteration_streak = stagnant_tool_iteration_streak,
                    stagnation_summary = %stagnation_summary,
                    "Blocking loop: generic stagnation detector observed repeated no-progress tool outcomes while required verification is still missing — forcing verification retry"
                );
                response.tool_calls = combined_tool_calls;
                response.content = content;
                response.thinking = thinking;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        iteration_tool_calls.len(),
                        response.content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::RequiredVerificationPending,
                    )
                    .await;
                current_prompt = self.build_required_verification_prompt(
                    &current_prompt,
                    &response.content,
                    &response.tool_calls,
                );
                current_prompt = Self::with_stagnation_recovery_instruction(
                    &current_prompt,
                    stagnant_tool_iteration_streak,
                    &stagnation_summary,
                    &stagnation_fingerprint.missing_requirements,
                );
                force_required_verification_retry = true;
                iteration += 1;
                continue;
            }

            if Self::should_finalize_completed_tool_iteration(
                requires_build_and_test,
                requires_mutating_file_tool_success,
                &content,
                &combined_tool_calls,
                &iteration_tool_calls,
                open_descendant_summary,
                task_tool_suspended,
            ) {
                tracing::info!(
                    iteration = iteration,
                    tool_calls_count = iteration_tool_calls.len(),
                    "Blocking loop: meaningful completion text accompanied successful tool calls — accepting iteration as terminal"
                );
                response.tool_calls = combined_tool_calls;
                response.content = content;
                response.thinking = thinking;
                delivered_terminal_summary = true;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        iteration_tool_calls.len(),
                        response.content.chars().count(),
                        true,
                    )
                    .await;
                break;
            }

            let should_force_required_verification =
                Self::should_force_required_verification_after_stalled_tool_loop(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &content,
                    &combined_tool_calls,
                    &iteration_tool_calls,
                    open_descendant_summary,
                    consecutive_nonterminal_tool_iterations,
                );

            let should_force_completion_ready_final_summary =
                Self::should_force_tool_free_final_summary_after_completion_ready_tool_iteration(
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &content,
                    &combined_tool_calls,
                    &iteration_tool_calls,
                    runtime_state.tracked.as_ref(),
                    open_descendant_summary,
                );

            if should_force_completion_ready_final_summary
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::info!(
                    iteration = iteration,
                    tool_calls_count = iteration_tool_calls.len(),
                    "Blocking loop: tool iteration completed the tracked runtime without a usable terminal summary — forcing tool-free closeout"
                );
                telemetry
                    .record_iteration_completed(
                        iteration,
                        iteration_tool_calls.len(),
                        content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::ForcedFinalSummary,
                    )
                    .await;
                current_prompt = self.build_tool_free_final_summary_prompt(
                    &current_prompt,
                    &response.content,
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &response.tool_calls,
                    runtime_state
                        .tracked
                        .as_ref()
                        .map(|state| state.snapshot.missing_requirements.as_slice())
                        .unwrap_or(&[]),
                    open_descendant_summary,
                );
                force_tool_free_final_summary = true;
                forced_execution_after_empty_response = true;
                forced_final_summary_requested = true;
                iteration += 1;
                continue;
            }

            if should_force_required_verification
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    tool_calls_count = iteration_tool_calls.len(),
                    consecutive_nonterminal_tool_iterations =
                        consecutive_nonterminal_tool_iterations,
                    repeated_verification_command = ?Self::trailing_repeated_successful_verification_command(
                        &combined_tool_calls,
                        2,
                    ),
                    low_value_inspection_signature = iteration_tool_calls
                        .first()
                        .and_then(Self::low_value_inspection_signature),
                    "Blocking loop: missing required build/test after a stalled inspection or repeated verification loop — forcing remaining-verification retry"
                );
                response.tool_calls = combined_tool_calls;
                response.content = content;
                response.thinking = thinking;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        iteration_tool_calls.len(),
                        response.content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::RequiredVerificationPending,
                    )
                    .await;
                current_prompt = self.build_required_verification_prompt(
                    &current_prompt,
                    &response.content,
                    &response.tool_calls,
                );
                force_required_verification_retry = true;
                iteration += 1;
                continue;
            }

            let should_force_tool_free_final_summary =
                Self::should_force_tool_free_final_summary_after_stalled_tool_loop(
                    requires_build_and_test,
                    &content,
                    &combined_tool_calls,
                    &iteration_tool_calls,
                    open_descendant_summary,
                    ToolSuspensionState {
                        task: task_tool_suspended,
                        file: file_tool_suspended,
                        code: code_tool_suspended,
                    },
                    consecutive_nonterminal_tool_iterations,
                );

            if should_force_tool_free_final_summary
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    tool_calls_count = iteration_tool_calls.len(),
                    consecutive_nonterminal_tool_iterations =
                        consecutive_nonterminal_tool_iterations,
                    "Blocking loop: stalled tool-only loop — forcing tool-free final summary attempt"
                );
                response.tool_calls = combined_tool_calls;
                response.content = content;
                response.thinking = thinking;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        iteration_tool_calls.len(),
                        response.content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::ForcedFinalSummary,
                    )
                    .await;
                current_prompt = self.build_tool_free_final_summary_prompt(
                    &current_prompt,
                    &response.content,
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &response.tool_calls,
                    runtime_state
                        .tracked
                        .as_ref()
                        .map(|state| state.snapshot.missing_requirements.as_slice())
                        .unwrap_or(&[]),
                    open_descendant_summary,
                );
                force_tool_free_final_summary = true;
                forced_execution_after_empty_response = true;
                forced_final_summary_requested = true;
                iteration += 1;
                continue;
            }

            if stagnant_tool_iteration_streak >= 3
                && Self::has_iteration_headroom(iteration, max_iterations)
            {
                tracing::warn!(
                    iteration = iteration,
                    stagnant_tool_iteration_streak = stagnant_tool_iteration_streak,
                    stagnation_summary = %stagnation_summary,
                    "Blocking loop: generic stagnation detector observed repeated no-progress tool outcomes — forcing tool-free final summary attempt"
                );
                response.tool_calls = combined_tool_calls;
                response.content = content;
                response.thinking = thinking;
                telemetry
                    .record_iteration_completed(
                        iteration,
                        iteration_tool_calls.len(),
                        response.content.chars().count(),
                        false,
                    )
                    .await;
                telemetry
                    .record_iteration_continuation(
                        iteration,
                        AgentLoopContinuation::ForcedFinalSummary,
                    )
                    .await;
                current_prompt = self.build_tool_free_final_summary_prompt(
                    &current_prompt,
                    &response.content,
                    requires_build_and_test,
                    requires_mutating_file_tool_success,
                    &response.tool_calls,
                    runtime_state
                        .tracked
                        .as_ref()
                        .map(|state| state.snapshot.missing_requirements.as_slice())
                        .unwrap_or(&[]),
                    open_descendant_summary,
                );
                current_prompt = Self::with_stagnation_recovery_instruction(
                    &current_prompt,
                    stagnant_tool_iteration_streak,
                    &stagnation_summary,
                    &stagnation_fingerprint.missing_requirements,
                );
                force_tool_free_final_summary = true;
                forced_execution_after_empty_response = true;
                forced_final_summary_requested = true;
                iteration += 1;
                continue;
            }

            telemetry
                .record_iteration_completed(
                    iteration,
                    iteration_tool_calls.len(),
                    content.chars().count(),
                    false,
                )
                .await;
            telemetry
                .record_iteration_continuation(iteration, AgentLoopContinuation::ToolResults)
                .await;

            // Build continuation prompt with tool results for the next iteration.
            current_prompt = self.build_tool_continuation_prompt(
                &current_prompt,
                &content,
                &iteration_tool_calls,
            );
            if stagnant_tool_iteration_streak >= 2 {
                current_prompt = Self::with_stagnation_recovery_instruction(
                    &current_prompt,
                    stagnant_tool_iteration_streak,
                    &stagnation_summary,
                    &stagnation_fingerprint.missing_requirements,
                );
            }
            response.tool_calls.extend(iteration_tool_calls);
            response.content = content;
            response.thinking = thinking;
            iteration += 1;
        }

        self.finalize_agent_loop_response(
            &mut response,
            saw_any_tool_calls,
            delivered_terminal_summary,
            max_iterations,
            requires_build_and_test,
            requires_mutating_file_tool_success,
            session_id.as_deref(),
            task_id.as_deref(),
            telemetry,
            None,
        )
        .await;

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
        let primary_model = match self.config.llm.primary.as_str() {
            "openai" => self
                .config
                .llm
                .openai
                .as_ref()
                .map(|config| config.model.as_str()),
            "anthropic" => self
                .config
                .llm
                .anthropic
                .as_ref()
                .map(|config| config.model.as_str()),
            "grok" => self
                .config
                .llm
                .grok
                .as_ref()
                .map(|config| config.model.as_str()),
            "gemini" => self
                .config
                .llm
                .gemini
                .as_ref()
                .map(|config| config.model.as_str()),
            "ollama" => self
                .config
                .llm
                .ollama
                .as_ref()
                .map(|config| config.model.as_str()),
            _ => None,
        };
        let tools_for_primary = tool_schemas
            .map(|s| tools_slice_for_provider(&self.config.llm.primary, primary_model, s));

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

            let fallback_model = match fallback_provider_name.as_str() {
                "openai" => self
                    .config
                    .llm
                    .openai
                    .as_ref()
                    .map(|config| config.model.as_str()),
                "anthropic" => self
                    .config
                    .llm
                    .anthropic
                    .as_ref()
                    .map(|config| config.model.as_str()),
                "grok" => self
                    .config
                    .llm
                    .grok
                    .as_ref()
                    .map(|config| config.model.as_str()),
                "gemini" => self
                    .config
                    .llm
                    .gemini
                    .as_ref()
                    .map(|config| config.model.as_str()),
                "ollama" => self
                    .config
                    .llm
                    .ollama
                    .as_ref()
                    .map(|config| config.model.as_str()),
                _ => None,
            };
            let tools_for_fallback = tool_schemas
                .map(|s| tools_slice_for_provider(fallback_provider_name, fallback_model, s));

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
mod tests;
