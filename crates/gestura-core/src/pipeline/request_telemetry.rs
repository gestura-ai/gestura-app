use super::{AgentRequest, AgentResponse, RequestSource, ToolCallRecord, ToolResult};
use crate::{error::AppError, streaming::StreamChunk};
use gestura_core_foundation::{
    context::{RequestAnalysis, ResolvedContext},
    telemetry::get_telemetry_manager,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

/// Request-scoped telemetry helper for the agent pipeline.
///
/// Keeping the emission logic here lets the pipeline and agent loop report
/// lifecycle events without duplicating metric names, tags, or best-effort
/// guards. Every method becomes a no-op when telemetry is disabled.

/// Final outcome recorded for a single request execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RequestOutcome {
    #[default]
    Running,
    Succeeded,
    Cancelled,
    Paused,
    Failed,
}

/// High-level execution mode used to tag a request trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestRunMode {
    Streaming,
    Blocking,
}

/// Why the agent loop continued after an iteration instead of terminating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentLoopContinuation {
    ToolResults,
    OpenSubtasks,
    DeferredTrackedWork,
    EmptyTerminalRetry,
    ForcedFinalSummary,
}

/// Per-request telemetry state shared across pipeline and loop code paths.
#[derive(Clone, Debug)]
pub(super) struct AgentRequestTelemetry {
    enabled: bool,
    started_at: Instant,
    common_tags: HashMap<String, String>,
    outcome: Arc<Mutex<RequestOutcome>>,
}

impl AgentRequestTelemetry {
    /// Creates a new request trace and emits the initial `request.started`
    /// event with stable per-request tags.
    pub(super) async fn start(request: &AgentRequest, mode: RequestRunMode, enabled: bool) -> Self {
        let mut common_tags = HashMap::from([
            ("request_id".into(), uuid::Uuid::new_v4().to_string()),
            (
                "mode".into(),
                match mode {
                    RequestRunMode::Streaming => "streaming",
                    RequestRunMode::Blocking => "blocking",
                }
                .into(),
            ),
            ("source".into(), source_tag(request.metadata.source).into()),
            ("streaming".into(), request.streaming.to_string()),
            ("history_messages".into(), request.history.len().to_string()),
            (
                "has_resume_state".into(),
                request.resume_from.is_some().to_string(),
            ),
        ]);
        for (key, value) in [
            ("session_id", request.metadata.session_id.as_ref()),
            ("task_id", request.metadata.task_id.as_ref()),
            ("directive_id", request.metadata.directive_id.as_ref()),
            ("agent_id", request.metadata.agent_id.as_ref()),
        ] {
            if let Some(value) = value {
                common_tags.insert(key.into(), value.clone());
            }
        }
        let telemetry = Self {
            enabled,
            started_at: Instant::now(),
            common_tags,
            outcome: Arc::new(Mutex::new(RequestOutcome::Running)),
        };
        telemetry
            .event(
                "agent.pipeline.request.started",
                vec![
                    ("input_chars", request.input.chars().count().to_string()),
                    (
                        "allowed_tools_count",
                        request.metadata.allowed_tools.len().to_string(),
                    ),
                    (
                        "memory_tags_count",
                        request.metadata.memory_tags.len().to_string(),
                    ),
                ],
            )
            .await;
        telemetry
    }

    /// Updates the terminal outcome for early-exit paths such as cancel/pause.
    pub(super) fn mark_outcome(&self, outcome: RequestOutcome) {
        if self.enabled {
            *self.outcome.lock().expect("telemetry lock poisoned") = outcome;
        }
    }

    /// Records the analyzer's normalized understanding of the request.
    pub(super) async fn record_analysis(&self, analysis: &RequestAnalysis) {
        self.event(
            "agent.pipeline.analysis.completed",
            vec![
                ("needs_tools", analysis.needs_tools.to_string()),
                ("is_followup", analysis.is_followup.to_string()),
                ("confidence", format!("{:.2}", analysis.confidence)),
                ("category_count", analysis.categories.len().to_string()),
                ("entity_count", analysis.entities.len().to_string()),
                (
                    "suggested_tool_count",
                    analysis.suggested_tools.len().to_string(),
                ),
            ],
        )
        .await;
    }

    /// Captures the filtered tool set that will shape tool schema injection.
    pub(super) async fn record_tool_selection(
        &self,
        count: usize,
        include_mcp: bool,
        fallback: bool,
    ) {
        self.event(
            "agent.pipeline.tools.selected",
            vec![
                ("selected_tool_count", count.to_string()),
                ("include_mcp_tool_schemas", include_mcp.to_string()),
                ("used_all_tools_fallback", fallback.to_string()),
            ],
        )
        .await;
    }

    /// Records the resolved context payload at a named lifecycle phase.
    ///
    /// We emit this before prompt construction and again after compaction, since
    /// compaction can change history selection while enrichment re-injects memory
    /// and knowledge sections.
    pub(super) async fn record_context_resolved(
        &self,
        phase: &'static str,
        context: &ResolvedContext,
    ) {
        self.event(
            "agent.pipeline.context.resolved",
            vec![
                ("phase", phase.to_string()),
                ("category_count", context.categories.len().to_string()),
                ("tool_count", context.tools.len().to_string()),
                ("file_count", context.files.len().to_string()),
                (
                    "memory_sections_count",
                    context.memory_sections.len().to_string(),
                ),
                ("knowledge_count", context.knowledge.len().to_string()),
                (
                    "has_history_summary",
                    context.history_summary.is_some().to_string(),
                ),
                ("estimated_tokens", context.estimated_tokens.to_string()),
            ],
        )
        .await;
    }

    /// Emits compaction-related telemetry derived from stream chunks so request
    /// traces capture context-shaping side effects.
    pub(super) async fn record_compaction(&self, chunk: &StreamChunk) {
        match chunk {
            StreamChunk::ContextCompacted {
                messages_before,
                messages_after,
                tokens_saved,
                summary,
            } => {
                self.event(
                    "agent.pipeline.context.compacted",
                    vec![
                        ("messages_before", messages_before.to_string()),
                        ("messages_after", messages_after.to_string()),
                        ("tokens_saved", tokens_saved.to_string()),
                        ("summary", summary.clone()),
                    ],
                )
                .await
            }
            StreamChunk::MemoryBankSaved {
                file_path,
                session_id,
                summary,
                messages_saved,
            } => {
                self.event(
                    "agent.pipeline.context.memory_bank_saved",
                    vec![
                        ("file_path", file_path.clone()),
                        ("session_id", session_id.clone()),
                        ("summary", summary.clone()),
                        ("messages_saved", messages_saved.to_string()),
                    ],
                )
                .await
            }
            _ => {}
        }
    }

    /// Records prompt-shaping information immediately before model execution.
    ///
    /// Token counts here are heuristic and intended for debugging rather than
    /// exact provider-side accounting.
    pub(super) async fn record_prompt_prepared(
        &self,
        prompt: &str,
        truncated: bool,
        context: &ResolvedContext,
    ) {
        self.event(
            "agent.pipeline.prompt.prepared",
            vec![
                ("prompt_chars", prompt.chars().count().to_string()),
                (
                    "estimated_prompt_tokens",
                    estimate_tokens(prompt).to_string(),
                ),
                ("truncated", truncated.to_string()),
                (
                    "context_estimated_tokens",
                    context.estimated_tokens.to_string(),
                ),
            ],
        )
        .await;
    }

    /// Marks the beginning of one agent-loop iteration.
    pub(super) async fn record_iteration_start(
        &self,
        iteration: usize,
        max_iterations: Option<usize>,
        task_tool_suspended: bool,
    ) {
        self.event(
            "agent.pipeline.iteration.started",
            vec![
                ("iteration", iteration.to_string()),
                (
                    "max_iterations",
                    max_iterations
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unbounded".into()),
                ),
                ("task_tool_suspended", task_tool_suspended.to_string()),
            ],
        )
        .await;
    }

    /// Records the observable result of one iteration before the loop either
    /// terminates or builds a continuation prompt.
    pub(super) async fn record_iteration_completed(
        &self,
        iteration: usize,
        tool_calls: usize,
        content_chars: usize,
        delivered_terminal_summary: bool,
    ) {
        self.event(
            "agent.pipeline.iteration.completed",
            vec![
                ("iteration", iteration.to_string()),
                ("tool_call_count", tool_calls.to_string()),
                ("content_chars", content_chars.to_string()),
                (
                    "delivered_terminal_summary",
                    delivered_terminal_summary.to_string(),
                ),
            ],
        )
        .await;
    }

    /// Records the control-flow reason that kept the loop running.
    pub(super) async fn record_iteration_continuation(
        &self,
        iteration: usize,
        kind: AgentLoopContinuation,
    ) {
        self.event(
            "agent.pipeline.iteration.continued",
            vec![
                ("iteration", iteration.to_string()),
                ("reason", continuation_tag(kind).into()),
            ],
        )
        .await;
    }

    /// Emits one completion event per finalized tool call in the iteration.
    pub(super) async fn record_tool_calls(&self, iteration: usize, tool_calls: &[ToolCallRecord]) {
        for tool_call in tool_calls {
            self.event(
                "agent.pipeline.tool_call.completed",
                vec![
                    ("iteration", iteration.to_string()),
                    ("tool_name", tool_call.name.clone()),
                    ("outcome", tool_result_tag(&tool_call.result).into()),
                    ("duration_ms", tool_call.duration_ms.to_string()),
                ],
            )
            .await;
        }
    }

    /// Notes that the pipeline synthesized a fallback final summary.
    pub(super) async fn record_synthetic_summary(
        &self,
        reason: &'static str,
        tool_call_count: usize,
    ) {
        self.event(
            "agent.pipeline.summary.synthetic",
            vec![
                ("reason", reason.into()),
                ("tool_call_count", tool_call_count.to_string()),
            ],
        )
        .await;
    }

    /// Records generation of a structured reflection artifact.
    pub(super) async fn record_reflection_generated(
        &self,
        initial_quality_score: f32,
        confidence: f32,
    ) {
        self.event(
            "agent.pipeline.reflection.generated",
            vec![
                (
                    "initial_quality_score",
                    format!("{initial_quality_score:.2}"),
                ),
                ("confidence", format!("{confidence:.2}")),
            ],
        )
        .await;
    }

    /// Records the bounded retry pass triggered by reflection, if any.
    pub(super) async fn record_reflection_retry(
        &self,
        improved: bool,
        improvement_score: f32,
        added_iterations: usize,
    ) {
        self.event(
            "agent.pipeline.reflection.retry_completed",
            vec![
                ("improved", improved.to_string()),
                ("improvement_score", format!("{improvement_score:.2}")),
                ("added_iterations", added_iterations.to_string()),
            ],
        )
        .await;
    }

    /// Emits the terminal request event and duration metric.
    ///
    /// If no earlier path marked a terminal outcome, `finish()` infers success
    /// vs failure from the presence of an error.
    pub(super) async fn finish(&self, response: Option<&AgentResponse>, error: Option<&AppError>) {
        if !self.enabled {
            return;
        }
        let outcome = {
            let mut state = self.outcome.lock().expect("telemetry lock poisoned");
            if *state == RequestOutcome::Running {
                *state = if error.is_some() {
                    RequestOutcome::Failed
                } else {
                    RequestOutcome::Succeeded
                };
            }
            *state
        };
        let mut tags = vec![("outcome", outcome.as_str().into())];
        if let Some(response) = response {
            tags.push(("iterations", response.iterations.to_string()));
            tags.push(("tool_call_count", response.tool_calls.len().to_string()));
            tags.push((
                "response_chars",
                response.content.chars().count().to_string(),
            ));
            tags.push(("truncated", response.truncated.to_string()));
            if let Some(usage) = &response.usage {
                tags.push(("usage_total_tokens", usage.total_tokens.to_string()));
            }
        }
        if let Some(error) = error {
            tags.push(("error_kind", error_tag(error).into()));
        }
        self.event(
            match outcome {
                RequestOutcome::Succeeded => "agent.pipeline.request.completed",
                RequestOutcome::Cancelled => "agent.pipeline.request.cancelled",
                RequestOutcome::Paused => "agent.pipeline.request.paused",
                RequestOutcome::Failed => "agent.pipeline.request.failed",
                RequestOutcome::Running => "agent.pipeline.request.completed",
            },
            tags,
        )
        .await;
        self.histogram(
            "agent.pipeline.request.duration_ms",
            self.started_at.elapsed().as_millis() as f64,
            vec![("outcome", outcome.as_str().into())],
        )
        .await;
    }

    /// Best-effort counter emission with the request's stable tag set.
    async fn event(&self, name: &str, extra_tags: Vec<(&'static str, String)>) {
        if self.enabled {
            get_telemetry_manager()
                .await
                .increment_counter(name, 1.0, self.tags(extra_tags))
                .await;
        }
    }

    /// Best-effort histogram emission with the request's stable tag set.
    async fn histogram(&self, name: &str, value: f64, extra_tags: Vec<(&'static str, String)>) {
        if self.enabled {
            get_telemetry_manager()
                .await
                .record_histogram(name, value, self.tags(extra_tags))
                .await;
        }
    }

    /// Merges stable request tags with per-event dimensions.
    fn tags(&self, extra_tags: Vec<(&'static str, String)>) -> HashMap<String, String> {
        let mut tags = self.common_tags.clone();
        for (key, value) in extra_tags {
            tags.insert(key.into(), value);
        }
        tags
    }
}

impl RequestOutcome {
    fn as_str(self) -> &'static str {
        match self {
            RequestOutcome::Running => "running",
            RequestOutcome::Succeeded => "succeeded",
            RequestOutcome::Cancelled => "cancelled",
            RequestOutcome::Paused => "paused",
            RequestOutcome::Failed => "failed",
        }
    }
}
fn estimate_tokens(text: &str) -> usize {
    ((text.split_whitespace().count() as f64 * 1.3) as usize)
        .max(text.chars().count() / 4)
        .max(1)
}
fn source_tag(source: RequestSource) -> &'static str {
    match source {
        RequestSource::GuiText => "gui_text",
        RequestSource::GuiVoice => "gui_voice",
        RequestSource::CliTui => "cli_tui",
        RequestSource::CliBasic => "cli_basic",
        RequestSource::Orchestrator => "orchestrator",
        RequestSource::Unknown => "unknown",
    }
}
fn continuation_tag(kind: AgentLoopContinuation) -> &'static str {
    match kind {
        AgentLoopContinuation::ToolResults => "tool_results",
        AgentLoopContinuation::OpenSubtasks => "open_subtasks",
        AgentLoopContinuation::DeferredTrackedWork => "deferred_tracked_work",
        AgentLoopContinuation::EmptyTerminalRetry => "empty_terminal_retry",
        AgentLoopContinuation::ForcedFinalSummary => "forced_final_summary",
    }
}
fn tool_result_tag(result: &ToolResult) -> &'static str {
    match result {
        ToolResult::Success(_) => "success",
        ToolResult::Error(_) => "error",
        ToolResult::Skipped(_) => "skipped",
    }
}
fn error_tag(error: &AppError) -> &'static str {
    match error {
        AppError::Io(_) => "io",
        AppError::Json(_) => "json",
        AppError::Toml(_) => "toml",
        AppError::Nats(_) => "nats",
        AppError::Ble(_) => "ble",
        AppError::Llm(_) => "llm",
        AppError::Http(_) => "http",
        AppError::Voice(_) => "voice",
        AppError::Audio(_) => "audio",
        AppError::Config(_) => "config",
        AppError::Session(_) => "session",
        AppError::Mcp(_) => "mcp",
        AppError::PermissionDenied(_) => "permission_denied",
        AppError::NotFound(_) => "not_found",
        AppError::Timeout(_) => "timeout",
        AppError::InvalidInput(_) => "invalid_input",
        AppError::Internal(_) => "internal",
    }
}
