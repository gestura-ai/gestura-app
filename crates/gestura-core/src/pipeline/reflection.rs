//! Pipeline reflection integration — ERL-inspired experiential learning.
//!
//! This module is the runtime half of Gestura's reflection system. It takes the
//! portable reflection types/helpers from `gestura-core-pipeline` and wires them
//! into the agent loop implemented by `AgentPipeline`.
//!
//! ## Runtime flow
//!
//! When reflection is enabled, the pipeline currently does the following:
//!
//! 1. **Context injection** — during context enrichment, retrieve relevant
//!    `MemoryType::Reflection` entries from the memory bank and append them to
//!    `resolved_context.memory_sections`.
//! 2. **Quality gating** — after the main agentic loop returns, score the turn
//!    using heuristic quality signals.
//! 3. **Reflection generation** — if the score falls below the configured
//!    threshold, emit `StreamChunk::ReflectionStarted` and make one lightweight,
//!    no-tools LLM call to produce a structured reflection.
//! 4. **Same-turn retry** — optionally ask the model for a text-only revision of
//!    the already-produced answer using the reflection as guidance. This never
//!    replays tools or side effects.
//! 5. **Consolidation** — store the reflection in session working memory and,
//!    when confidence is high enough, promote it to the long-term memory bank as
//!    `MemoryType::Reflection`.
//! 6. **Completion event** — emit `StreamChunk::ReflectionComplete` so CLI/GUI
//!    surfaces can show what was learned and whether it was persisted.
//!
//! ## Boundaries
//!
//! - Pure prompt/parse/scoring logic lives in `gestura-core-pipeline`.
//! - Config lives in `gestura-core-config` and is mapped into
//!   `PipelineConfig.reflection`.
//! - Long-term persistence uses the memory-bank domain.
//! - Streaming visibility is expressed via `gestura-core-streaming` events.

use crate::agent_sessions::{AgentSessionStore, FileAgentSessionStore};
use crate::memory_bank::{
    MemoryBankEntry, MemoryScope, MemoryType, ReflectionMemoryState, list_memory_bank,
};
use crate::session_workspace::SessionWorkspace;
use crate::streaming::{CancellationToken, StreamChunk};

use gestura_core_foundation::{OutcomeSignal, OutcomeSignalKind};
use gestura_core_pipeline::reflection::{
    AgentReflection, QualitySignals, ReflectionConfig, build_reflection_prompt,
    merge_outcome_signals, parse_reflection_response, quality_signals_for_response,
    reflection_promotion_confidence, score_reflection_improvement,
};
use gestura_core_pipeline::types::{AgentResponse, RequestMetadata, ToolCallRecord, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::json;

use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

use super::{AgentPipeline, AppError, PermissionLevel, ToolDefinition};

pub(super) const REFLECTION_RETRY_SEPARATOR: &str = "\n\nRevised answer after reflection:\n";
const MIN_REFLECTION_RETRY_DELTA: f32 = 0.05;
const RETRY_STREAM_CHUNK_CHARS: usize = 320;
const RETRY_READ_ONLY_PERMISSION_LEVEL: PermissionLevel = PermissionLevel::Sandbox;

#[derive(Debug, Clone)]
pub(super) struct GeneratedReflection {
    pub reflection: AgentReflection,
    pub initial_quality_score: f32,
}

#[derive(Debug, Clone)]
pub(super) enum ReflectionRetryMode {
    TextRevision,
    CorrectiveReexecution,
}

impl std::fmt::Display for ReflectionRetryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TextRevision => write!(f, "text revision"),
            Self::CorrectiveReexecution => write!(f, "corrective re-execution"),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ReflectionRetryOutcome {
    pub mode: ReflectionRetryMode,
    pub improved: bool,
    pub revised_content: String,
    pub retry_quality_score: f32,
    pub improvement_score: f32,
    pub usage: Option<crate::llm_provider::TokenUsage>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub iterations: usize,
}

pub(super) struct ReflectionReexecutionContext<'a> {
    pub base_prompt: &'a str,
    pub tools: Vec<&'static ToolDefinition>,
    pub context: crate::context::ResolvedContext,
    pub workspace: Option<&'a SessionWorkspace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TaskReflectionLink {
    pub reflection_id: String,
    pub session_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub attempt_summary: String,
    pub failure_analysis: String,
    pub corrective_strategy: String,
    pub strategy_key: String,
    pub improvement_score: Option<f32>,
    #[serde(default)]
    pub outcome_signals: Vec<OutcomeSignal>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub confidence: f32,
    pub reflection_state: ReflectionMemoryState,
    #[serde(default)]
    pub success_count: u16,
    #[serde(default)]
    pub failure_count: u16,
    #[serde(default = "default_reflection_promotion_threshold")]
    pub promotion_threshold: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_memory_file_path: Option<String>,
}

fn default_reflection_promotion_threshold() -> f32 {
    ReflectionConfig::default().promotion_confidence
}

impl TaskReflectionLink {
    fn from_reflection(
        reflection: &AgentReflection,
        task_id: &str,
        directive_id: Option<String>,
        agent_id: Option<String>,
        promotion_threshold: f32,
        promoted_memory_file_path: Option<String>,
    ) -> Self {
        let mut tags = reflection.tags.clone();
        tags.push("reflection".to_string());
        tags.extend(outcome_signal_labels(&reflection.outcome_signals));
        let (success_count, failure_count) = reflection_signal_counts(&reflection.outcome_signals);
        let reflection_state = derive_reflection_state(
            reflection.promotion_confidence(),
            success_count,
            failure_count,
        );
        tags.retain(|tag| !is_reflection_state_tag(tag));
        tags.push(reflection_state_tag(reflection_state).to_string());
        tags.sort();
        tags.dedup();

        Self {
            reflection_id: reflection.reflection_id.clone(),
            session_id: reflection.session_id.clone(),
            task_id: task_id.to_string(),
            directive_id,
            agent_id,
            attempt_summary: reflection.attempt_summary.clone(),
            failure_analysis: reflection.failure_analysis.clone(),
            corrective_strategy: reflection.corrective_strategy.clone(),
            strategy_key: normalize_strategy_key(&reflection.corrective_strategy),
            improvement_score: reflection.improvement_score,
            outcome_signals: reflection.outcome_signals.clone(),
            tags,
            confidence: reflection.promotion_confidence(),
            reflection_state,
            success_count,
            failure_count,
            promotion_threshold,
            promoted_memory_file_path,
        }
    }

    fn apply_outcome_signals(&mut self, incoming: &[OutcomeSignal]) {
        self.outcome_signals = merge_outcome_signals(&self.outcome_signals, incoming);
        self.tags
            .extend(outcome_signal_labels(&self.outcome_signals));
        (self.success_count, self.failure_count) = reflection_signal_counts(&self.outcome_signals);
        self.confidence =
            reflection_promotion_confidence(self.improvement_score, &self.outcome_signals);
        self.set_reflection_state(derive_reflection_state(
            self.confidence,
            self.success_count,
            self.failure_count,
        ));
    }

    fn set_reflection_state(&mut self, state: ReflectionMemoryState) {
        self.reflection_state = state;
        self.tags.retain(|tag| !is_reflection_state_tag(tag));
        self.tags.push(reflection_state_tag(state).to_string());
        self.tags.sort();
        self.tags.dedup();
    }

    fn decision_summary(&self) -> String {
        format!("Reflection: {}", self.corrective_strategy)
    }

    fn decision_rationale(&self) -> String {
        let improvement_note = self
            .improvement_score
            .map(|score| format!("\nObserved improvement: {:.0}%", score * 100.0))
            .unwrap_or_default();
        let outcome_note = outcome_signal_summary(&self.outcome_signals)
            .map(|summary| format!("\nOutcome signals: {summary}"))
            .unwrap_or_default();
        format!(
            "Issue: {}\nStrategy: {}\nState: {}\nSuccesses: {}\nFailures: {}{}{}",
            self.failure_analysis,
            self.corrective_strategy,
            self.reflection_state,
            self.success_count,
            self.failure_count,
            improvement_note,
            outcome_note,
        )
    }

    fn memory_content(&self) -> String {
        let outcome_section = format_outcome_signal_section(&self.outcome_signals);
        format!(
            "## Reflection\n\n\
             **Attempted:** {}\n\n\
             **Issue:** {}\n\n\
             **Corrective Strategy:** {}\n\n\
             **Strategy State:** {}\n\n\
             **Successes / Failures:** {} / {}\n\n\
             **Observed Improvement:** {}\n\n\
             {}",
            self.attempt_summary,
            self.failure_analysis,
            self.corrective_strategy,
            self.reflection_state,
            self.success_count,
            self.failure_count,
            self.improvement_score
                .map(|score| format!("{:.0}%", score * 100.0))
                .unwrap_or_else(|| "not measured".to_string()),
            outcome_section,
        )
    }

    fn should_promote(&self) -> bool {
        self.promoted_memory_file_path.is_none()
            && self.confidence >= self.promotion_threshold
            && self.reflection_state == ReflectionMemoryState::Active
    }

    fn to_memory_entry(&self) -> MemoryBankEntry {
        MemoryBankEntry::new(
            self.session_id.clone(),
            self.decision_summary(),
            self.memory_content(),
        )
        .with_memory_type(MemoryType::Reflection)
        .with_scope(MemoryScope::Session)
        .with_category("reflection")
        .with_reflection_id(self.reflection_id.clone())
        .with_reflection_learning(
            self.strategy_key.clone(),
            self.reflection_state,
            self.success_count,
            self.failure_count,
        )
        .with_provenance(
            Some(self.task_id.clone()),
            self.directive_id.clone(),
            self.agent_id.clone(),
        )
        .with_tags(self.tags.clone())
        .with_promotion(
            self.session_id.clone(),
            "ERL-inspired experiential reflection promoted from session",
        )
        .with_outcome_provenance(
            outcome_signal_summary(&self.outcome_signals),
            outcome_signal_labels(&self.outcome_signals),
        )
        .with_confidence(self.confidence)
    }
}

fn normalize_strategy_key(strategy: &str) -> String {
    let normalized = strategy
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();
    let mut collapsed = normalized
        .split_whitespace()
        .take(10)
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        collapsed = "reflection-strategy".to_string();
    }
    collapsed
}

fn reflection_signal_counts(outcome_signals: &[OutcomeSignal]) -> (u16, u16) {
    let success_count = outcome_signals
        .iter()
        .filter(|signal| is_positive_outcome_signal(signal.kind))
        .count() as u16;
    let failure_count = outcome_signals
        .iter()
        .filter(|signal| is_negative_outcome_signal(signal.kind))
        .count() as u16;
    (success_count, failure_count)
}

fn derive_reflection_state(
    confidence: f32,
    success_count: u16,
    failure_count: u16,
) -> ReflectionMemoryState {
    if failure_count >= 3 || (failure_count >= 2 && success_count == 0 && confidence < 0.40) {
        ReflectionMemoryState::Archived
    } else if failure_count >= 2 && success_count == 0 {
        ReflectionMemoryState::Decayed
    } else {
        ReflectionMemoryState::Active
    }
}

fn is_positive_outcome_signal(kind: OutcomeSignalKind) -> bool {
    matches!(
        kind,
        OutcomeSignalKind::RetryImproved
            | OutcomeSignalKind::PreExecutionApproved
            | OutcomeSignalKind::ReviewApproved
            | OutcomeSignalKind::TestValidationApproved
            | OutcomeSignalKind::TaskCompleted
    )
}

fn is_negative_outcome_signal(kind: OutcomeSignalKind) -> bool {
    matches!(
        kind,
        OutcomeSignalKind::RetryDidNotImprove
            | OutcomeSignalKind::PreExecutionRejected
            | OutcomeSignalKind::PreExecutionNeedsRevision
            | OutcomeSignalKind::ReviewRejected
            | OutcomeSignalKind::ReviewNeedsRevision
            | OutcomeSignalKind::TestValidationRejected
            | OutcomeSignalKind::TestValidationNeedsRevision
            | OutcomeSignalKind::TaskFailed
            | OutcomeSignalKind::TaskBlocked
            | OutcomeSignalKind::TaskCancelled
    )
}

fn reflection_state_tag(state: ReflectionMemoryState) -> &'static str {
    match state {
        ReflectionMemoryState::Active => "reflection_state_active",
        ReflectionMemoryState::Decayed => "reflection_state_decayed",
        ReflectionMemoryState::NeedsReview => "reflection_state_needs_review",
        ReflectionMemoryState::Archived => "reflection_state_archived",
    }
}

fn is_reflection_state_tag(tag: &str) -> bool {
    matches!(
        tag,
        "reflection_state_active"
            | "reflection_state_decayed"
            | "reflection_state_needs_review"
            | "reflection_state_archived"
    )
}

impl AgentPipeline {
    /// Query the memory bank for past reflections relevant to the current request.
    ///
    /// Returns formatted prompt sections ready for injection into
    /// `resolved_context.memory_sections`.
    ///
    /// Retrieval is intentionally narrow:
    ///
    /// - only `MemoryType::Reflection` entries are eligible,
    /// - only long-term memory kinds are searched,
    /// - session/workspace/repository scopes are considered,
    /// - and task/agent/tag metadata are reused when available so the injected
    ///   reflections are tied to the current request shape.
    pub(super) async fn load_relevant_reflections(
        &self,
        workspace_dir: &Path,
        metadata: &RequestMetadata,
        query: &str,
    ) -> Option<Vec<String>> {
        let max = self.pipeline_config.reflection.max_injected_reflections;
        if max == 0 {
            return None;
        }

        let mut memory_query = crate::memory_bank::MemoryBankQuery::text(query)
            .with_limit(max.saturating_mul(3).max(max))
            .with_min_confidence(0.5);

        // Only reflection-type entries
        memory_query.memory_types = vec![MemoryType::Reflection];
        memory_query.kinds = vec![crate::memory_bank::MemoryKind::LongTerm];
        memory_query.scopes = vec![
            MemoryScope::Session,
            MemoryScope::Workspace,
            MemoryScope::Repository,
        ];

        if let Some(task_id) = metadata.task_id.as_deref() {
            memory_query = memory_query.with_task(task_id.to_string());
        }
        if let Some(agent_id) = metadata.agent_id.as_deref() {
            memory_query = memory_query.with_agent(agent_id.to_string());
        }
        if !metadata.memory_tags.is_empty() {
            memory_query = memory_query.with_tags(metadata.memory_tags.clone());
        }

        match crate::memory_bank::search_memory_bank_with_query(workspace_dir, &memory_query).await
        {
            Ok(results) if !results.is_empty() => {
                let eligible_results = results
                    .into_iter()
                    .filter(|result| result.entry.is_prompt_eligible_reflection())
                    .take(max)
                    .collect::<Vec<_>>();
                if eligible_results.is_empty() {
                    return None;
                }
                tracing::info!(
                    count = eligible_results.len(),
                    "Injecting past reflections into prompt context"
                );

                let sections = eligible_results
                    .into_iter()
                    .map(|r| {
                        let mut section = String::from("### Past Reflection\n");
                        section.push_str(&r.entry.to_prompt_section(300));
                        section
                    })
                    .collect::<Vec<_>>();

                Some(sections)
            }
            _ => None,
        }
    }

    /// Evaluate the quality of an agent response using heuristic signals.
    ///
    /// This replaces ERL's verifiable reward signal with observable proxy metrics.
    pub(super) fn evaluate_response_quality(
        &self,
        response: &AgentResponse,
        max_iterations: usize,
    ) -> QualitySignals {
        quality_signals_for_response(response, max_iterations)
    }

    /// Generate a structured reflection for a low-quality response.
    ///
    /// Returns the parsed reflection plus the initial quality score when the
    /// response qualifies for reflection.
    ///
    /// This is the only extra-model-call step in the reflection flow, which is
    /// why the feature stays opt-in and behind a quality gate.
    pub(super) async fn maybe_generate_reflection(
        &self,
        request_input: &str,
        response: &AgentResponse,
        metadata: &RequestMetadata,
        tx: Option<&mpsc::Sender<StreamChunk>>,
        _cancel_token: &CancellationToken,
        max_iterations: usize,
    ) -> Option<GeneratedReflection> {
        if !self.pipeline_config.reflection.enabled {
            return None;
        }

        let quality = self.evaluate_response_quality(response, max_iterations);
        let quality_score = quality.score();

        if quality_score >= self.pipeline_config.reflection.quality_threshold {
            tracing::debug!(
                quality_score,
                threshold = self.pipeline_config.reflection.quality_threshold,
                "Response quality above reflection threshold, skipping"
            );
            return None;
        }

        // Gate passed — reflection triggered
        let reason = format!(
            "Quality score {:.2} below threshold {:.2}{}{}{}",
            quality_score,
            self.pipeline_config.reflection.quality_threshold,
            if quality.tool_error_rate > 0.0 {
                format!(" (tool errors: {:.0}%)", quality.tool_error_rate * 100.0)
            } else {
                String::new()
            },
            if quality.was_truncated {
                " (truncated)"
            } else {
                ""
            },
            if quality.has_failure_patterns {
                " (failure patterns detected)"
            } else {
                ""
            },
        );

        tracing::info!(
            quality_score,
            threshold = self.pipeline_config.reflection.quality_threshold,
            reason = %reason,
            "Reflection triggered"
        );

        if let Some(tx) = tx {
            let _ = tx
                .send(StreamChunk::ReflectionStarted {
                    reason: reason.clone(),
                })
                .await;
        }

        // Collect tool errors for the reflection prompt
        let tool_errors: Vec<String> = response
            .tool_calls
            .iter()
            .filter_map(|tc| {
                if let ToolResult::Error(e) = &tc.result {
                    Some(format!("{}: {}", tc.name, e))
                } else {
                    None
                }
            })
            .collect();

        // Build the reflection prompt
        let reflection_prompt =
            build_reflection_prompt(request_input, &response.content, &quality, &tool_errors);

        // Generate reflection via a lightweight LLM call (blocking, no tools)
        match self.generate_reflection_response(&reflection_prompt).await {
            Some(reflection_text) => {
                let session_id = metadata
                    .session_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());

                if let Some(mut reflection) =
                    parse_reflection_response(&reflection_text, &session_id)
                {
                    reflection.task_id = metadata.task_id.clone();
                    return Some(GeneratedReflection {
                        reflection,
                        initial_quality_score: quality_score,
                    });
                } else {
                    tracing::warn!(
                        reflection_excerpt = %truncate_retry_text(&reflection_text, 320),
                        "Failed to parse reflection response from LLM"
                    );
                    if let Some(tx) = tx {
                        let _ = tx
                            .send(StreamChunk::ReflectionComplete {
                                summary: "Reflection generated but could not be parsed".to_string(),
                                stored: false,
                                promoted: false,
                            })
                            .await;
                    }
                }
            }
            None => {
                tracing::warn!("Failed to generate reflection — LLM call returned no content");
                if let Some(tx) = tx {
                    let _ = tx
                        .send(StreamChunk::ReflectionComplete {
                            summary: "Reflection could not be generated".to_string(),
                            stored: false,
                            promoted: false,
                        })
                        .await;
                }
            }
        }

        None
    }

    /// Attempt a safe text-only retry using the generated reflection.
    ///
    /// This does not re-execute tools. Instead it asks the model to revise the
    /// already-produced answer using the reflection and observed tool outcomes.
    ///
    /// The retry is deliberately conservative: it can improve wording and repair
    /// omissions in the final answer, but it cannot introduce new side effects by
    /// rerunning tools.
    pub(super) async fn maybe_run_reflection_retry(
        &self,
        request_input: &str,
        response: &AgentResponse,
        reflection: &AgentReflection,
        initial_quality_score: f32,
        tx: Option<&mpsc::Sender<StreamChunk>>,
    ) -> Option<ReflectionRetryOutcome> {
        if self.pipeline_config.reflection.max_retry_attempts == 0 {
            return None;
        }

        if let Some(tx) = tx {
            let _ = tx
                .send(StreamChunk::Status {
                    message:
                        "Low-confidence answer detected; attempting a reflection-guided revision..."
                            .to_string(),
                })
                .await;
        }

        let retry_prompt = build_reflection_retry_prompt(request_input, response, reflection);
        match self.call_llm_with_fallback(&retry_prompt, None).await {
            Ok(llm_response) => {
                let (revised_content, _thinking) =
                    crate::streaming::split_think_blocks(&llm_response.text);
                let retry_response = AgentResponse {
                    content: revised_content.clone(),
                    thinking: None,
                    tool_calls: Vec::new(),
                    usage: Some(llm_response.usage.clone()),
                    context_used: response.context_used.clone(),
                    truncated: false,
                    iterations: 1,
                };
                let retry_quality_score =
                    self.evaluate_response_quality(&retry_response, 1).score();
                let improvement_score =
                    score_reflection_improvement(initial_quality_score, retry_quality_score);
                let improved = !revised_content.trim().is_empty()
                    && improvement_score >= MIN_REFLECTION_RETRY_DELTA;

                Some(ReflectionRetryOutcome {
                    mode: ReflectionRetryMode::TextRevision,
                    improved,
                    revised_content,
                    retry_quality_score,
                    improvement_score,
                    usage: Some(llm_response.usage),
                    tool_calls: Vec::new(),
                    iterations: 1,
                })
            }
            Err(e) => {
                tracing::warn!(error = %e, "Reflection-guided retry failed");
                if let Some(tx) = tx {
                    let _ = tx
                        .send(StreamChunk::Status {
                            message: format!(
                                "Reflection-guided revision failed; keeping the original answer ({e})"
                            ),
                        })
                        .await;
                }
                None
            }
        }
    }

    /// Determine whether a low-quality turn should use the stronger corrective
    /// re-execution path instead of a text-only revision.
    pub(super) fn should_attempt_reflection_reexecution(
        &self,
        response: &AgentResponse,
        reflection: &AgentReflection,
        tools_available: bool,
    ) -> bool {
        if self.pipeline_config.reflection.max_retry_attempts == 0 || !tools_available {
            return false;
        }

        let strategy_text = format!(
            "{} {}",
            reflection.failure_analysis, reflection.corrective_strategy
        )
        .to_ascii_lowercase();
        let verification_strategy = [
            "inspect", "read", "search", "verify", "validate", "check", "review", "examine",
            "compare", "look up", "grep", "find", "trace", "rerun", "re-run", "test",
        ]
        .iter()
        .any(|keyword| strategy_text.contains(keyword));
        let had_tool_failure = response
            .tool_calls
            .iter()
            .any(|call| matches!(call.result, ToolResult::Error(_) | ToolResult::Skipped(_)));

        verification_strategy || had_tool_failure || response.tool_calls.is_empty()
    }

    /// Run one bounded corrective retry that may use read-only tools.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn maybe_run_reflection_reexecution(
        &self,
        response: &AgentResponse,
        reflection: &AgentReflection,
        initial_quality_score: f32,
        retry_ctx: ReflectionReexecutionContext<'_>,
    ) -> Option<ReflectionRetryOutcome> {
        let retry_prompt =
            build_reflection_reexecution_prompt(retry_ctx.base_prompt, response, reflection);
        match self
            .execute_reflection_retry_blocking_loop(
                retry_prompt,
                retry_ctx.tools,
                retry_ctx.context,
                retry_ctx.workspace,
            )
            .await
        {
            Ok(retry_response) => {
                let retry_quality_score = self
                    .evaluate_response_quality(&retry_response, self.pipeline_config.max_iterations)
                    .score();
                let improvement_score =
                    score_reflection_improvement(initial_quality_score, retry_quality_score);
                let improved = !retry_response.content.trim().is_empty()
                    && improvement_score >= MIN_REFLECTION_RETRY_DELTA;

                Some(ReflectionRetryOutcome {
                    mode: ReflectionRetryMode::CorrectiveReexecution,
                    improved,
                    revised_content: retry_response.content,
                    retry_quality_score,
                    improvement_score,
                    usage: retry_response.usage,
                    tool_calls: retry_response.tool_calls,
                    iterations: retry_response.iterations,
                })
            }
            Err(error) => {
                tracing::warn!(error = %error, "Reflection corrective re-execution failed");
                None
            }
        }
    }

    async fn execute_reflection_retry_blocking_loop(
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

        let tool_schemas = if tools.is_empty() {
            None
        } else {
            Some(crate::tools::schemas::build_provider_tool_schemas(&tools))
        };

        let mut current_prompt = initial_prompt;
        for iteration in 0..self.pipeline_config.max_iterations {
            response.iterations = iteration + 1;
            let llm_response = self
                .call_llm_with_fallback(&current_prompt, tool_schemas.as_ref())
                .await?;
            let (content, thinking) = crate::streaming::split_think_blocks(&llm_response.text);

            if let Some(existing_usage) = response.usage.as_mut() {
                existing_usage.input_tokens += llm_response.usage.input_tokens;
                existing_usage.output_tokens += llm_response.usage.output_tokens;
                existing_usage.total_tokens += llm_response.usage.total_tokens;
                existing_usage.estimated_cost_usd = match (
                    existing_usage.estimated_cost_usd,
                    llm_response.usage.estimated_cost_usd,
                ) {
                    (Some(lhs), Some(rhs)) => Some(lhs + rhs),
                    (Some(lhs), None) => Some(lhs),
                    (None, Some(rhs)) => Some(rhs),
                    (None, None) => None,
                };
                if existing_usage.model.is_none() {
                    existing_usage.model = llm_response.usage.model.clone();
                }
                if existing_usage.provider.is_none() {
                    existing_usage.provider = llm_response.usage.provider.clone();
                }
            } else {
                response.usage = Some(llm_response.usage);
            }

            if llm_response.tool_calls.is_empty() {
                response.content = content;
                response.thinking = thinking;
                break;
            }

            let mut iteration_tool_calls = Vec::new();
            let mut pending_parallel_batch = Vec::new();
            let mut pending_parallel_signatures = std::collections::HashSet::new();
            for tc in &llm_response.tool_calls {
                let decision = crate::tools::policy::evaluate_tool_call(
                    RETRY_READ_ONLY_PERMISSION_LEVEL,
                    &tc.name,
                    &tc.arguments,
                )
                .decision;

                let parallel_signature = format!("{}\u{1f}{}", tc.name, tc.arguments);
                if matches!(decision, crate::tools::policy::ToolCallDecision::Allowed)
                    && Self::can_parallelize_read_only_tool_call(&tc.name, &tc.arguments)
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

                let result = match decision {
                    crate::tools::policy::ToolCallDecision::Allowed
                        if Self::can_parallelize_read_only_tool_call(&tc.name, &tc.arguments) =>
                    {
                        pending_parallel_signatures.insert(parallel_signature);
                        pending_parallel_batch.push(tc.clone());
                        continue;
                    }
                    crate::tools::policy::ToolCallDecision::Allowed => {
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
                        self.execute_tool(&tc.name, &tc.arguments, workspace, None)
                            .await
                    }
                    crate::tools::policy::ToolCallDecision::Blocked { reason } => {
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
                        ToolResult::Skipped(reason)
                    }
                    crate::tools::policy::ToolCallDecision::RequiresConfirmation(info) => {
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
                        ToolResult::Skipped(format!(
                            "Skipped during reflection retry: {}",
                            info.description
                        ))
                    }
                };

                iteration_tool_calls.push(ToolCallRecord {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                    result,
                    duration_ms: 0,
                });
            }
            if !pending_parallel_batch.is_empty() {
                iteration_tool_calls.extend(
                    self.execute_parallel_read_only_tool_batch(pending_parallel_batch, workspace)
                        .await,
                );
            }

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

    /// Store a reflection and emit the final reflection completion event.
    ///
    /// This is the consolidation step of the runtime flow: after any optional
    /// same-turn retry, the pipeline persists the reflection and reports whether
    /// it was only stored for the current session or also promoted to long-term
    /// memory.
    pub(super) async fn finalize_reflection(
        &self,
        reflection: &AgentReflection,
        metadata: &RequestMetadata,
        workspace: Option<&SessionWorkspace>,
        tx: Option<&mpsc::Sender<StreamChunk>>,
        retry: Option<&ReflectionRetryOutcome>,
    ) {
        let mut learned_reflection = reflection.clone();
        if let Some(retry) = retry {
            let signal = if retry.improved {
                OutcomeSignal::new(OutcomeSignalKind::RetryImproved).with_summary(format!(
                    "{} quality reached {:.0}% with {:.0}% normalized improvement ({} iterations, {} tool calls).",
                    retry.mode,
                    retry.retry_quality_score * 100.0,
                    retry.improvement_score * 100.0,
                    retry.iterations,
                    retry.tool_calls.len(),
                ))
            } else {
                OutcomeSignal::new(OutcomeSignalKind::RetryDidNotImprove).with_summary(format!(
                    "{} did not materially improve the answer ({} iterations, {} tool calls).",
                    retry.mode,
                    retry.iterations,
                    retry.tool_calls.len(),
                ))
            };
            learned_reflection.push_outcome_signal(signal);
        }

        let (stored, promoted) = self
            .store_reflection(&learned_reflection, metadata, workspace.map(|w| w.root()))
            .await;

        let retry_note = retry.map_or_else(String::new, |retry| {
            if retry.improved {
                format!(
                    " ({} quality {:.0}%, improvement {:.0}%, {} tool calls)",
                    retry.mode,
                    retry.retry_quality_score * 100.0,
                    retry.improvement_score * 100.0,
                    retry.tool_calls.len(),
                )
            } else {
                format!(" ({} did not materially improve the answer)", retry.mode)
            }
        });
        let summary = format!(
            "Learned: {}{}",
            learned_reflection.corrective_strategy, retry_note
        );

        if let Some(tx) = tx {
            let _ = tx
                .send(StreamChunk::ReflectionComplete {
                    summary,
                    stored,
                    promoted,
                })
                .await;
        }
    }

    /// Emit a revised answer to the streaming client in user-visible chunks.
    pub(super) async fn emit_reflection_retry_text(
        &self,
        tx: &mpsc::Sender<StreamChunk>,
        content: &str,
    ) {
        let mut rest = content;
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(RETRY_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());
            let (chunk, next) = rest.split_at(split_at);
            rest = next;
            if !chunk.is_empty() {
                let _ = tx.send(StreamChunk::Text(chunk.to_string())).await;
            }
        }
    }

    /// Make a lightweight LLM call to generate a reflection (no tools, non-streaming).
    async fn generate_reflection_response(&self, prompt: &str) -> Option<String> {
        use crate::llm_provider::{AgentContext, select_provider};

        let ctx = AgentContext::default();
        let provider = select_provider(&self.config, &ctx);

        match provider.call(prompt).await {
            Ok(content) => {
                let content = content.trim().to_string();
                if content.is_empty() {
                    None
                } else {
                    Some(content)
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Reflection LLM call failed");
                None
            }
        }
    }

    /// Store a reflection in session working memory and optionally promote to long-term.
    ///
    /// Returns `(stored, promoted)` booleans.
    ///
    /// Storage happens in two tiers:
    ///
    /// 1. short-term/session working memory for immediate follow-up turns
    /// 2. long-term memory-bank promotion when the reflection appears strong
    ///    enough to generalize beyond the current episode
    async fn store_reflection(
        &self,
        reflection: &AgentReflection,
        metadata: &RequestMetadata,
        workspace_root: Option<&Path>,
    ) -> (bool, bool) {
        let mut stored = false;
        let mut promoted = false;
        let mut promoted_memory_file_path: Option<String> = None;

        // 1. Store in session working memory (short-term) as a decision
        if let Some(session_id) = metadata.session_id.as_deref() {
            let store = FileAgentSessionStore::new_default();
            match store.load(session_id) {
                Ok(mut session) => {
                    let mut link = metadata.task_id.as_deref().map(|task_id| {
                        TaskReflectionLink::from_reflection(
                            reflection,
                            task_id,
                            metadata.directive_id.clone(),
                            metadata.agent_id.clone(),
                            self.pipeline_config.reflection.promotion_confidence,
                            None,
                        )
                    });
                    let summary = link
                        .as_ref()
                        .map(TaskReflectionLink::decision_summary)
                        .unwrap_or_else(|| {
                            format!("Reflection: {}", &reflection.corrective_strategy)
                        });
                    let rationale = link
                        .as_ref()
                        .map(|entry| Some(entry.decision_rationale()))
                        .unwrap_or_else(|| {
                            let improvement_note = reflection.improvement_score.map(|score| {
                                format!("\nObserved improvement: {:.0}%", score * 100.0)
                            });
                            let outcome_note = outcome_signal_summary(&reflection.outcome_signals)
                                .map(|summary| format!("\nOutcome signals: {summary}"))
                                .unwrap_or_default();
                            Some(
                                format!(
                                    "Issue: {}\nStrategy: {}",
                                    reflection.failure_analysis, reflection.corrective_strategy
                                ) + &improvement_note.unwrap_or_default()
                                    + &outcome_note,
                            )
                        });
                    let tags = link
                        .as_ref()
                        .map(|entry| entry.tags.clone())
                        .unwrap_or_else(|| {
                            let mut tags = reflection.tags.clone();
                            tags.push("reflection".to_string());
                            tags.extend(outcome_signal_labels(&reflection.outcome_signals));
                            tags.sort();
                            tags.dedup();
                            tags
                        });

                    session.state.working_memory.remember_linked_decision(
                        summary,
                        rationale,
                        tags,
                        link.take().map(|entry| entry.reflection_id),
                    );

                    if let Err(e) = store.save(&session) {
                        tracing::warn!(error = %e, "Failed to save reflection to session working memory");
                    } else {
                        stored = true;
                        tracing::debug!(session_id, "Saved reflection to session working memory");
                    }
                }
                Err(e) => {
                    tracing::debug!(session_id, error = %e, "Could not load session for reflection storage");
                }
            }
        }

        // 2. Promote to long-term memory bank if above promotion confidence
        if let Some(workspace_dir) = workspace_root {
            let confidence = reflection.promotion_confidence();

            if confidence >= self.pipeline_config.reflection.promotion_confidence {
                let task_id = metadata.task_id.as_deref().unwrap_or("reflection");
                let link = TaskReflectionLink::from_reflection(
                    reflection,
                    task_id,
                    metadata.directive_id.clone(),
                    metadata.agent_id.clone(),
                    self.pipeline_config.reflection.promotion_confidence,
                    None,
                );
                let mut entry = link.to_memory_entry();
                entry.task_id = metadata.task_id.clone();

                match crate::memory_bank::save_to_memory_bank(workspace_dir, &entry).await {
                    Ok(path) => {
                        promoted = true;
                        promoted_memory_file_path = Some(path.display().to_string());
                        tracing::info!(
                            path = %path.display(),
                            "Promoted reflection to long-term memory bank"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to promote reflection to memory bank");
                    }
                }
            }
        }

        if let (Some(workspace_dir), Some(session_id), Some(task_id)) = (
            metadata.workspace_dir.as_deref(),
            metadata.session_id.as_deref(),
            metadata.task_id.as_deref(),
        ) {
            let mut link = TaskReflectionLink::from_reflection(
                reflection,
                task_id,
                metadata.directive_id.clone(),
                metadata.agent_id.clone(),
                self.pipeline_config.reflection.promotion_confidence,
                promoted_memory_file_path.clone(),
            );
            if let Err(error) =
                reconcile_reflection_strategy_conflicts(workspace_dir, &mut link).await
            {
                tracing::warn!(
                    error = %error,
                    session_id,
                    task_id,
                    "Failed to reconcile reflection strategy conflicts during initial storage"
                );
            }
            let store = FileAgentSessionStore::new_default();
            if let Ok(mut session) = store.load(session_id) {
                session.state.working_memory.remember_linked_decision(
                    link.decision_summary(),
                    Some(link.decision_rationale()),
                    link.tags.clone(),
                    Some(link.reflection_id.clone()),
                );
                if let Err(error) = store.save(&session) {
                    tracing::warn!(
                        error = %error,
                        session_id,
                        task_id,
                        "Failed to persist reconciled reflection decision during initial storage"
                    );
                }
            }
            persist_task_reflection_link(workspace_dir, session_id, task_id, &link);
        }

        (stored, promoted)
    }
}

pub(crate) async fn sync_task_reflection_outcomes(
    workspace_dir: &Path,
    session_id: &str,
    task_id: &str,
    incoming: &[OutcomeSignal],
) -> bool {
    let store = FileAgentSessionStore::new_default();
    sync_task_reflection_outcomes_with_store(workspace_dir, session_id, task_id, incoming, &store)
        .await
}

async fn sync_task_reflection_outcomes_with_store(
    workspace_dir: &Path,
    session_id: &str,
    task_id: &str,
    incoming: &[OutcomeSignal],
    store: &FileAgentSessionStore,
) -> bool {
    let Some(mut link) = load_task_reflection_link(workspace_dir, session_id, task_id) else {
        return false;
    };

    let previous_signals = link.outcome_signals.clone();
    link.apply_outcome_signals(incoming);
    if previous_signals == link.outcome_signals {
        return false;
    }

    if let Ok(mut session) = store.load(session_id) {
        session.state.working_memory.remember_linked_decision(
            link.decision_summary(),
            Some(link.decision_rationale()),
            link.tags.clone(),
            Some(link.reflection_id.clone()),
        );
        if let Err(error) = store.save(&session) {
            tracing::warn!(error = %error, session_id, task_id, "Failed to update reflection working memory");
        }
    }

    if let Some(memory_path) = link.promoted_memory_file_path.clone() {
        let memory_path = PathBuf::from(memory_path);
        match crate::memory_bank::load_from_memory_bank(&memory_path).await {
            Ok(mut entry) => {
                entry.summary = link.decision_summary();
                entry.content = link.memory_content();
                entry.reflection_id = Some(link.reflection_id.clone());
                entry.strategy_key = Some(link.strategy_key.clone());
                entry.reflection_state = Some(link.reflection_state);
                entry.success_count = link.success_count;
                entry.failure_count = link.failure_count;
                entry.tags = link.tags.clone();
                entry.outcome_summary = outcome_signal_summary(&link.outcome_signals);
                entry.outcome_labels = outcome_signal_labels(&link.outcome_signals);
                entry.confidence = link.confidence;
                if let Err(error) = crate::memory_bank::update_memory_bank_entry(
                    workspace_dir,
                    &memory_path,
                    &entry,
                )
                .await
                {
                    tracing::warn!(error = %error, path = %memory_path.display(), "Failed to update reflection memory entry");
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    path = %memory_path.display(),
                    "Failed to load existing reflection memory entry; will retry promotion if eligible"
                );
                link.promoted_memory_file_path = None;
            }
        }
    }

    if link.should_promote() {
        let entry = link.to_memory_entry();
        match crate::memory_bank::save_to_memory_bank(workspace_dir, &entry).await {
            Ok(path) => {
                link.promoted_memory_file_path = Some(path.display().to_string());
                tracing::info!(
                    path = %path.display(),
                    session_id,
                    task_id,
                    confidence = link.confidence,
                    threshold = link.promotion_threshold,
                    "Promoted reflection to long-term memory bank after downstream outcomes"
                );
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    session_id,
                    task_id,
                    confidence = link.confidence,
                    threshold = link.promotion_threshold,
                    "Failed to promote reflection after downstream outcomes"
                );
            }
        }
    }

    if let Err(error) = reconcile_reflection_strategy_conflicts(workspace_dir, &mut link).await {
        tracing::warn!(error = %error, session_id, task_id, "Failed to reconcile reflection strategy conflicts");
    }

    if let Ok(mut session) = store.load(session_id) {
        session.state.working_memory.remember_linked_decision(
            link.decision_summary(),
            Some(link.decision_rationale()),
            link.tags.clone(),
            Some(link.reflection_id.clone()),
        );
        if let Err(error) = store.save(&session) {
            tracing::warn!(error = %error, session_id, task_id, "Failed to persist reconciled reflection working memory");
        }
    }

    persist_task_reflection_link(workspace_dir, session_id, task_id, &link);
    true
}

async fn reconcile_reflection_strategy_conflicts(
    workspace_dir: &Path,
    link: &mut TaskReflectionLink,
) -> Result<(), crate::memory_bank::MemoryBankError> {
    let Some(current_polarity) = reflection_polarity(link.success_count, link.failure_count) else {
        return Ok(());
    };

    let entries = list_memory_bank(workspace_dir).await?;
    let mut conflicting_paths = Vec::new();
    for entry in entries {
        if entry.memory_type != MemoryType::Reflection
            || entry.strategy_key.as_deref() != Some(link.strategy_key.as_str())
            || entry.reflection_id.as_deref() == Some(link.reflection_id.as_str())
        {
            continue;
        }

        let Some(other_polarity) = reflection_polarity(entry.success_count, entry.failure_count)
        else {
            continue;
        };
        if other_polarity != current_polarity
            && let Some(path) = entry.file_path.clone()
        {
            conflicting_paths.push(path);
        }
    }

    if conflicting_paths.is_empty() {
        return Ok(());
    }

    link.set_reflection_state(ReflectionMemoryState::NeedsReview);

    if let Some(memory_path) = link.promoted_memory_file_path.as_deref() {
        let memory_path = PathBuf::from(memory_path);
        if let Ok(mut entry) = crate::memory_bank::load_from_memory_bank(&memory_path).await {
            apply_needs_review_to_entry(&mut entry);
            crate::memory_bank::update_memory_bank_entry(workspace_dir, &memory_path, &entry)
                .await?;
        }
    }

    for path in conflicting_paths {
        let mut entry = crate::memory_bank::load_from_memory_bank(&path).await?;
        apply_needs_review_to_entry(&mut entry);
        crate::memory_bank::update_memory_bank_entry(workspace_dir, &path, &entry).await?;
    }

    Ok(())
}

fn apply_needs_review_to_entry(entry: &mut MemoryBankEntry) {
    entry.reflection_state = Some(ReflectionMemoryState::NeedsReview);
    entry.tags.retain(|tag| !is_reflection_state_tag(tag));
    entry
        .tags
        .push(reflection_state_tag(ReflectionMemoryState::NeedsReview).to_string());
    entry.tags.sort();
    entry.tags.dedup();
}

fn reflection_polarity(success_count: u16, failure_count: u16) -> Option<bool> {
    if success_count > 0 && failure_count == 0 {
        Some(true)
    } else if failure_count > 0 && success_count == 0 {
        Some(false)
    } else {
        None
    }
}

fn load_task_reflection_link(
    workspace_dir: &Path,
    session_id: &str,
    task_id: &str,
) -> Option<TaskReflectionLink> {
    let manager = crate::tasks::TaskManager::new(workspace_dir);
    let metadata = manager
        .get_task(session_id, task_id)
        .ok()
        .flatten()?
        .metadata?;
    serde_json::from_value(metadata.get("reflection_learning")?.clone()).ok()
}

fn persist_task_reflection_link(
    workspace_dir: &Path,
    session_id: &str,
    task_id: &str,
    link: &TaskReflectionLink,
) {
    let Ok(value) = serde_json::to_value(link) else {
        return;
    };
    merge_task_metadata_local(
        &crate::tasks::TaskManager::new(workspace_dir),
        session_id,
        task_id,
        json!({ "reflection_learning": value }),
    );
}

fn merge_task_metadata_local(
    manager: &crate::tasks::TaskManager,
    session_id: &str,
    task_id: &str,
    patch: serde_json::Value,
) {
    let existing = manager
        .get_task(session_id, task_id)
        .ok()
        .flatten()
        .and_then(|task| task.metadata)
        .unwrap_or_else(|| json!({}));

    let Some(mut existing_map) = existing.as_object().cloned() else {
        return;
    };
    let Some(patch_map) = patch.as_object() else {
        return;
    };

    for (key, value) in patch_map {
        existing_map.insert(key.clone(), value.clone());
    }

    let _ =
        manager.update_task_metadata(session_id, task_id, serde_json::Value::Object(existing_map));
}

fn build_reflection_retry_prompt(
    request_input: &str,
    response: &AgentResponse,
    reflection: &AgentReflection,
) -> String {
    let tool_summary = format_tool_results_for_retry(&response.tool_calls);

    format!(
        "You are revising a prior assistant answer after self-reflection.\n\n\
         User request:\n{request_input}\n\n\
         Previous answer:\n{}\n\n\
         Observed tool outcomes:\n{tool_summary}\n\n\
         Reflection guidance:\n\
         - What went wrong: {}\n\
         - Corrective strategy: {}\n\n\
         Produce a revised final answer for the user.\n\
         Requirements:\n\
         - Do not call tools.\n\
         - Do not mention hidden reflection or internal analysis.\n\
         - If something is still blocked, say exactly what remains blocked and the best next step.\n\
         - Prefer a direct, corrected answer over an apology.\n",
        response.content, reflection.failure_analysis, reflection.corrective_strategy,
    )
}

fn build_reflection_reexecution_prompt(
    base_prompt: &str,
    response: &AgentResponse,
    reflection: &AgentReflection,
) -> String {
    let prior_answer = truncate_retry_text(&response.content, 800);
    let tool_summary = format_tool_results_for_retry(&response.tool_calls);

    format!(
        "{base_prompt}\n\n---\n\
         SYSTEM NOTE: The previous attempt was low quality. Start a fresh corrective retry from scratch.\n\
         Previous answer excerpt:\n{prior_answer}\n\n\
         Observed tool outcomes:\n{tool_summary}\n\n\
         Reflection guidance:\n\
         - What went wrong: {}\n\
         - Corrective strategy: {}\n\n\
         Requirements for this corrective retry:\n\
         - Follow the corrective strategy before drafting the final answer.\n\
         - Re-verify claims instead of trusting the previous answer.\n\
         - Use tools only when needed to gather missing evidence.\n\
         - Treat blocked or skipped tools as blockers; do not fabricate results.\n\
         - Produce a corrected final answer for the user without mentioning hidden reflection.\n",
        reflection.failure_analysis, reflection.corrective_strategy,
    )
}

fn format_tool_results_for_retry(tool_calls: &[ToolCallRecord]) -> String {
    if tool_calls.is_empty() {
        return "- No tools were used in the initial answer.".to_string();
    }

    tool_calls
        .iter()
        .map(|call| match &call.result {
            ToolResult::Success(output) => format!(
                "- {} succeeded: {}",
                call.name,
                truncate_retry_text(output, 180)
            ),
            ToolResult::Error(error) => format!("- {} failed: {}", call.name, error),
            ToolResult::Skipped(reason) => format!("- {} was skipped: {}", call.name, reason),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_retry_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", truncated.trim_end())
    } else {
        truncated
    }
}

fn outcome_signal_summary(signals: &[OutcomeSignal]) -> Option<String> {
    if signals.is_empty() {
        return None;
    }

    Some(
        signals
            .iter()
            .map(|signal| {
                signal
                    .summary
                    .clone()
                    .unwrap_or_else(|| signal.kind.label().to_string())
            })
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn outcome_signal_labels(signals: &[OutcomeSignal]) -> Vec<String> {
    signals
        .iter()
        .map(|signal| signal.durable_label().to_string())
        .collect()
}

fn format_outcome_signal_section(signals: &[OutcomeSignal]) -> String {
    if signals.is_empty() {
        return "## Outcome Signals\n\n- No durable outcome signals recorded yet.\n".to_string();
    }

    let lines = signals
        .iter()
        .map(|signal| {
            let detail = signal
                .summary
                .as_deref()
                .map(|summary| format!(": {summary}"))
                .unwrap_or_default();
            format!("- {}{}", signal.kind.label(), detail)
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("## Outcome Signals\n\n{lines}\n")
}

#[cfg(test)]
mod tests {
    use super::{
        TaskReflectionLink, build_reflection_reexecution_prompt, build_reflection_retry_prompt,
        format_outcome_signal_section, format_tool_results_for_retry, load_task_reflection_link,
        outcome_signal_summary, persist_task_reflection_link,
        reconcile_reflection_strategy_conflicts, sync_task_reflection_outcomes_with_store,
        truncate_retry_text,
    };
    use crate::agent_sessions::{AgentSession, AgentSessionStore, FileAgentSessionStore};
    use crate::memory_bank::{
        MemoryBankEntry, MemoryScope, MemoryType, ReflectionMemoryState, load_from_memory_bank,
        save_to_memory_bank,
    };
    use crate::tasks::TaskManager;
    use crate::{AgentPipeline, AppConfig, RequestMetadata};
    use gestura_core_foundation::context::ResolvedContext;
    use gestura_core_foundation::{OutcomeSignal, OutcomeSignalKind};
    use gestura_core_pipeline::reflection::AgentReflection;
    use gestura_core_pipeline::types::{AgentResponse, ToolCallRecord, ToolResult};
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn retry_prompt_includes_strategy_and_tool_failures() {
        let reflection = AgentReflection::new(
            "session-1",
            "Tried to inspect a config file",
            "The file lookup failed because the path was wrong",
            "Verify the path first and then answer with the actual file contents",
        );
        let response = AgentResponse {
            content: "I could not find the file.".to_string(),
            thinking: None,
            tool_calls: vec![ToolCallRecord {
                id: "call-1".to_string(),
                name: "file".to_string(),
                arguments: r#"{"path":"wrong.toml"}"#.to_string(),
                result: ToolResult::Error("wrong.toml does not exist".to_string()),
                duration_ms: 12,
            }],
            usage: None,
            context_used: ResolvedContext::default(),
            truncated: false,
            iterations: 1,
        };

        let prompt = build_reflection_retry_prompt("show me the config", &response, &reflection);
        assert!(prompt.contains("show me the config"));
        assert!(prompt.contains("wrong.toml does not exist"));
        assert!(prompt.contains("Verify the path first"));
        assert!(prompt.contains("Do not call tools"));
    }

    #[test]
    fn reexecution_prompt_requests_fresh_verified_retry() {
        let reflection = AgentReflection::new(
            "session-reexec",
            "Answered from memory",
            "The prior answer was not grounded in repository evidence",
            "Inspect the relevant files first, then answer with verified findings",
        );
        let response = AgentResponse {
            content: "I guessed the answer without checking the repository.".to_string(),
            thinking: None,
            tool_calls: vec![ToolCallRecord {
                id: "call-1".to_string(),
                name: "file".to_string(),
                arguments: r#"{"operation":"read","path":"src/main.rs"}"#.to_string(),
                result: ToolResult::Error("path not found".to_string()),
                duration_ms: 12,
            }],
            usage: None,
            context_used: ResolvedContext::default(),
            truncated: false,
            iterations: 1,
        };

        let prompt = build_reflection_reexecution_prompt("BASE PROMPT", &response, &reflection);
        assert!(prompt.contains("BASE PROMPT"));
        assert!(prompt.contains("Start a fresh corrective retry from scratch"));
        assert!(prompt.contains("Inspect the relevant files first"));
        assert!(prompt.contains("path not found"));
        assert!(prompt.contains("Re-verify claims"));
    }

    #[test]
    fn format_tool_results_handles_empty_calls() {
        let summary = format_tool_results_for_retry(&[]);
        assert!(summary.contains("No tools were used"));
    }

    #[test]
    fn truncate_retry_text_adds_ellipsis() {
        let shortened = truncate_retry_text("abcdefghijklmnopqrstuvwxyz", 8);
        assert_eq!(shortened, "abcdefgh…");
    }

    #[test]
    fn reflection_outcome_helpers_format_durable_metadata() {
        let signals = vec![
            OutcomeSignal::new(OutcomeSignalKind::RetryImproved)
                .with_summary("Retry quality improved from 40% to 76%"),
            OutcomeSignal::new(OutcomeSignalKind::ReviewApproved),
        ];

        let summary = outcome_signal_summary(&signals).unwrap();
        let section = format_outcome_signal_section(&signals);

        assert!(summary.contains("Retry quality improved from 40% to 76%"));
        assert!(summary.contains("Review approved"));
        assert!(section.contains("## Outcome Signals"));
        assert!(section.contains("Retry improved"));
        assert!(section.contains("Review approved"));
    }

    #[test]
    fn reflection_reexecution_is_chosen_for_verification_strategies() {
        let pipeline = AgentPipeline::new(AppConfig::default());
        let response = AgentResponse {
            content: "Here is my unverified guess.".to_string(),
            thinking: None,
            tool_calls: Vec::new(),
            usage: None,
            context_used: ResolvedContext::default(),
            truncated: false,
            iterations: 1,
        };
        let reflection = AgentReflection::new(
            "session-routing",
            "Guessed the answer",
            "The answer was not grounded in source files",
            "Inspect the relevant modules first, then answer with verified findings",
        );

        assert!(pipeline.should_attempt_reflection_reexecution(&response, &reflection, true));
        assert!(!pipeline.should_attempt_reflection_reexecution(&response, &reflection, false));
    }

    #[tokio::test]
    async fn sync_task_reflection_outcomes_updates_linked_records() {
        let workspace = TempDir::new().unwrap();
        let session_store = FileAgentSessionStore::new(workspace.path().join("sessions"));
        let session =
            AgentSession::new_with_workspace(workspace.path().to_path_buf(), None).unwrap();
        let session_id = session.id.clone();
        session_store.save(&session).unwrap();

        let manager = TaskManager::new(workspace.path());
        let task = manager
            .create_task(
                &session_id,
                "Reflection task",
                "Track reflective learning",
                None,
            )
            .unwrap();

        let mut reflection = AgentReflection::new(
            session_id.clone(),
            "Tried to answer before validating repository context",
            "The answer was based on assumptions instead of concrete code inspection",
            "Inspect the relevant modules first, then answer with verified findings",
        );
        reflection.task_id = Some(task.id.clone());
        reflection.improvement_score = Some(0.24);
        reflection.push_outcome_signal(OutcomeSignal::new(OutcomeSignalKind::RetryDidNotImprove));

        let mut link =
            TaskReflectionLink::from_reflection(&reflection, &task.id, None, None, 0.75, None);
        let entry = MemoryBankEntry::new(
            session_id.clone(),
            link.decision_summary(),
            link.memory_content(),
        )
        .with_memory_type(MemoryType::Reflection)
        .with_scope(MemoryScope::Session)
        .with_category("reflection")
        .with_reflection_id(reflection.reflection_id.clone())
        .with_provenance(Some(task.id.clone()), None, None)
        .with_tags(link.tags.clone())
        .with_outcome_provenance(
            outcome_signal_summary(&link.outcome_signals),
            super::outcome_signal_labels(&link.outcome_signals),
        )
        .with_confidence(link.confidence);
        let entry_path = save_to_memory_bank(workspace.path(), &entry).await.unwrap();
        link.promoted_memory_file_path = Some(entry_path.display().to_string());
        persist_task_reflection_link(workspace.path(), &session_id, &task.id, &link);

        let synced = sync_task_reflection_outcomes_with_store(
            workspace.path(),
            &session_id,
            &task.id,
            &[OutcomeSignal::new(OutcomeSignalKind::ReviewApproved)],
            &session_store,
        )
        .await;

        assert!(synced);

        let stored_link =
            load_task_reflection_link(workspace.path(), &session_id, &task.id).unwrap();
        assert!(
            stored_link
                .outcome_signals
                .iter()
                .any(|signal| { signal.kind == OutcomeSignalKind::ReviewApproved })
        );

        let stored_session = session_store.load(&session_id).unwrap();
        let decision = stored_session
            .state
            .working_memory
            .decisions
            .iter()
            .find(|decision| {
                decision.reference_id.as_deref() == Some(reflection.reflection_id.as_str())
            })
            .unwrap();
        assert!(decision.tags.iter().any(|tag| tag == "review_approved"));

        let stored_entry = load_from_memory_bank(&entry_path).await.unwrap();
        assert_eq!(
            stored_entry.reflection_id.as_deref(),
            Some(reflection.reflection_id.as_str())
        );
        assert!(
            stored_entry
                .outcome_labels
                .iter()
                .any(|label| label == "review_approved")
        );
        assert_eq!(
            stored_entry.reflection_state,
            Some(ReflectionMemoryState::Active)
        );
    }

    #[tokio::test]
    async fn reconcile_reflection_strategy_conflicts_marks_entries_for_review() {
        let workspace = TempDir::new().unwrap();

        let conflicting_entry = MemoryBankEntry::new(
            "session-conflict-a".to_string(),
            "Reflection: inspect files first".to_string(),
            "Inspect the relevant files before concluding behavior.".to_string(),
        )
        .with_memory_type(MemoryType::Reflection)
        .with_scope(MemoryScope::Workspace)
        .with_reflection_id("reflection-conflict-a")
        .with_reflection_learning(
            "inspect-the-relevant-files-first",
            ReflectionMemoryState::Decayed,
            0,
            2,
        )
        .with_tags(vec!["reflection_state_decayed".to_string()]);
        let conflicting_path = save_to_memory_bank(workspace.path(), &conflicting_entry)
            .await
            .unwrap();

        let mut reflection = AgentReflection::new(
            "session-conflict-b",
            "Inspected some files first",
            "The previous answer skipped validation of key modules",
            "Inspect the relevant files first",
        );
        reflection.improvement_score = Some(0.88);
        reflection.push_outcome_signal(OutcomeSignal::new(OutcomeSignalKind::ReviewApproved));
        reflection.push_outcome_signal(OutcomeSignal::new(OutcomeSignalKind::TaskCompleted));

        let mut link = TaskReflectionLink::from_reflection(
            &reflection,
            "task-conflict-b",
            None,
            None,
            0.70,
            None,
        );
        let current_path = save_to_memory_bank(workspace.path(), &link.to_memory_entry())
            .await
            .unwrap();
        link.promoted_memory_file_path = Some(current_path.display().to_string());

        reconcile_reflection_strategy_conflicts(workspace.path(), &mut link)
            .await
            .unwrap();

        assert_eq!(link.reflection_state, ReflectionMemoryState::NeedsReview);

        let updated_current = load_from_memory_bank(&current_path).await.unwrap();
        assert_eq!(
            updated_current.reflection_state,
            Some(ReflectionMemoryState::NeedsReview)
        );

        let updated_conflicting = load_from_memory_bank(&conflicting_path).await.unwrap();
        assert_eq!(
            updated_conflicting.reflection_state,
            Some(ReflectionMemoryState::NeedsReview)
        );
        assert!(
            updated_conflicting
                .tags
                .iter()
                .any(|tag| tag == "reflection_state_needs_review")
        );
    }

    #[tokio::test]
    async fn load_relevant_reflections_prefers_prompt_eligible_governed_memories() {
        let workspace = TempDir::new().unwrap();
        let pipeline = AgentPipeline::new(AppConfig::default());

        let active_entry = MemoryBankEntry::new(
            "session-learning-a".to_string(),
            "Reflection: inspect modules first".to_string(),
            "Inspect the relevant modules before answering architecture questions.".to_string(),
        )
        .with_memory_type(MemoryType::Reflection)
        .with_scope(MemoryScope::Workspace)
        .with_reflection_id("reflection-learning-a")
        .with_reflection_learning(
            "inspect-the-relevant-modules-first",
            ReflectionMemoryState::Active,
            2,
            0,
        )
        .with_tags(vec!["reflection".to_string()])
        .with_confidence(0.92);
        save_to_memory_bank(workspace.path(), &active_entry)
            .await
            .unwrap();

        let review_entry = MemoryBankEntry::new(
            "session-learning-b".to_string(),
            "Reflection: inspect modules first".to_string(),
            "Conflicting lesson that should be withheld from prompt injection.".to_string(),
        )
        .with_memory_type(MemoryType::Reflection)
        .with_scope(MemoryScope::Workspace)
        .with_reflection_id("reflection-learning-b")
        .with_reflection_learning(
            "inspect-the-relevant-modules-first",
            ReflectionMemoryState::NeedsReview,
            1,
            1,
        )
        .with_tags(vec!["reflection".to_string()])
        .with_confidence(0.98);
        save_to_memory_bank(workspace.path(), &review_entry)
            .await
            .unwrap();

        let archived_entry = MemoryBankEntry::new(
            "session-learning-c".to_string(),
            "Reflection: inspect modules first".to_string(),
            "Archived lesson that should never be injected.".to_string(),
        )
        .with_memory_type(MemoryType::Reflection)
        .with_scope(MemoryScope::Workspace)
        .with_reflection_id("reflection-learning-c")
        .with_reflection_learning(
            "inspect-the-relevant-modules-first",
            ReflectionMemoryState::Archived,
            0,
            3,
        )
        .with_tags(vec!["reflection".to_string()])
        .with_confidence(0.99);
        save_to_memory_bank(workspace.path(), &archived_entry)
            .await
            .unwrap();

        let metadata = RequestMetadata {
            memory_tags: vec!["reflection".to_string()],
            ..Default::default()
        };
        let injected = pipeline
            .load_relevant_reflections(
                workspace.path(),
                &metadata,
                "inspect the relevant modules first",
            )
            .await
            .unwrap();

        assert!(
            injected
                .iter()
                .any(|section| section.contains("Reflection: inspect modules first"))
        );
        assert!(
            injected
                .iter()
                .any(|section| section.contains("Inspect the relevant modules before answering"))
        );
        assert!(
            !injected
                .iter()
                .any(|section| section.contains("Conflicting lesson"))
        );
        assert!(
            !injected
                .iter()
                .any(|section| section.contains("Archived lesson"))
        );
    }

    #[tokio::test]
    async fn sync_task_reflection_outcomes_promotes_when_confidence_crosses_threshold() {
        let workspace = TempDir::new().unwrap();
        let session_store = FileAgentSessionStore::new(workspace.path().join("sessions"));
        let session =
            AgentSession::new_with_workspace(workspace.path().to_path_buf(), None).unwrap();
        let session_id = session.id.clone();
        session_store.save(&session).unwrap();

        let manager = TaskManager::new(workspace.path());
        let task = manager
            .create_task(
                &session_id,
                "Late promotion task",
                "Promote reflection only after downstream outcomes arrive",
                None,
            )
            .unwrap();

        let mut reflection = AgentReflection::new(
            session_id.clone(),
            "Answered before confirming whether the path existed",
            "The answer relied on assumptions instead of the actual repository state",
            "Inspect the relevant files first and only answer from verified evidence",
        );
        reflection.task_id = Some(task.id.clone());
        reflection.improvement_score = Some(0.40);
        reflection.push_outcome_signal(OutcomeSignal::new(OutcomeSignalKind::RetryDidNotImprove));

        let link = TaskReflectionLink::from_reflection(
            &reflection,
            &task.id,
            Some("directive-late-promotion".to_string()),
            Some("reflection-agent".to_string()),
            0.75,
            None,
        );
        assert!(link.confidence < link.promotion_threshold);
        persist_task_reflection_link(workspace.path(), &session_id, &task.id, &link);

        let synced = sync_task_reflection_outcomes_with_store(
            workspace.path(),
            &session_id,
            &task.id,
            &[
                OutcomeSignal::new(OutcomeSignalKind::ReviewApproved),
                OutcomeSignal::new(OutcomeSignalKind::TaskCompleted),
            ],
            &session_store,
        )
        .await;

        assert!(synced);

        let stored_link =
            load_task_reflection_link(workspace.path(), &session_id, &task.id).unwrap();
        assert!(stored_link.confidence >= stored_link.promotion_threshold);
        let promoted_path = stored_link.promoted_memory_file_path.as_ref().unwrap();
        let stored_entry = load_from_memory_bank(Path::new(promoted_path))
            .await
            .unwrap();
        assert_eq!(stored_entry.task_id.as_deref(), Some(task.id.as_str()));
        assert_eq!(
            stored_entry.reflection_state,
            Some(ReflectionMemoryState::Active)
        );
        assert_eq!(
            stored_entry.directive_id.as_deref(),
            Some("directive-late-promotion")
        );
        assert_eq!(stored_entry.agent_id.as_deref(), Some("reflection-agent"));
        assert!(
            stored_entry
                .outcome_labels
                .iter()
                .any(|label| label == "review_approved")
        );
        assert!(
            stored_entry
                .outcome_labels
                .iter()
                .any(|label| label == "task_completed")
        );
    }
}
