//! Pipeline reflection integration — ERL-inspired experiential learning.
//!
//! This module adds the reflection phase to `AgentPipeline`, implementing:
//!
//! 1. **Context injection** — load past reflections into prompt context
//! 2. **Quality gating** — evaluate response quality after the agentic loop
//! 3. **Reflection generation** — one extra LLM call to analyze suboptimal turns
//! 4. **Memory storage** — save reflections for future retrieval + promotion

use crate::agent_sessions::{AgentSessionStore, FileAgentSessionStore};
use crate::memory_bank::{MemoryBankEntry, MemoryScope, MemoryType};
use crate::session_workspace::SessionWorkspace;
use crate::streaming::{CancellationToken, StreamChunk};

use gestura_core_pipeline::reflection::{
    AgentReflection, QualitySignals, build_reflection_prompt, parse_reflection_response,
    quality_signals_for_response, score_reflection_improvement,
};
use gestura_core_pipeline::types::{AgentResponse, RequestMetadata, ToolCallRecord, ToolResult};

use std::path::Path;
use tokio::sync::mpsc;

use super::AgentPipeline;

pub(super) const REFLECTION_RETRY_SEPARATOR: &str = "\n\nRevised answer after reflection:\n";
const MIN_REFLECTION_RETRY_DELTA: f32 = 0.05;
const RETRY_STREAM_CHUNK_CHARS: usize = 320;

#[derive(Debug, Clone)]
pub(super) struct GeneratedReflection {
    pub reflection: AgentReflection,
    pub initial_quality_score: f32,
}

#[derive(Debug, Clone)]
pub(super) struct ReflectionRetryOutcome {
    pub improved: bool,
    pub revised_content: String,
    pub retry_quality_score: f32,
    pub improvement_score: f32,
    pub usage: crate::llm_provider::TokenUsage,
}

impl AgentPipeline {
    /// Query the memory bank for past reflections relevant to the current request.
    ///
    /// Returns formatted prompt sections ready for injection into `resolved_context.memory_sections`.
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
            .with_limit(max)
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
                tracing::info!(
                    count = results.len(),
                    "Injecting past reflections into prompt context"
                );

                let sections = results
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
                    tracing::warn!("Failed to parse reflection response from LLM");
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
                    improved,
                    revised_content,
                    retry_quality_score,
                    improvement_score,
                    usage: llm_response.usage,
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

    /// Store a reflection and emit the final reflection completion event.
    pub(super) async fn finalize_reflection(
        &self,
        reflection: &AgentReflection,
        metadata: &RequestMetadata,
        workspace: Option<&SessionWorkspace>,
        tx: Option<&mpsc::Sender<StreamChunk>>,
        retry: Option<&ReflectionRetryOutcome>,
    ) {
        let (stored, promoted) = self
            .store_reflection(reflection, metadata, workspace.map(|w| w.root()))
            .await;

        let retry_note = retry.map_or_else(String::new, |retry| {
            if retry.improved {
                format!(
                    " (retry quality {:.0}%, improvement {:.0}%)",
                    retry.retry_quality_score * 100.0,
                    retry.improvement_score * 100.0,
                )
            } else {
                " (retry did not materially improve the answer)".to_string()
            }
        });
        let summary = format!("Learned: {}{}", reflection.corrective_strategy, retry_note);

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
    async fn store_reflection(
        &self,
        reflection: &AgentReflection,
        metadata: &RequestMetadata,
        workspace_root: Option<&Path>,
    ) -> (bool, bool) {
        let mut stored = false;
        let mut promoted = false;

        // 1. Store in session working memory (short-term) as a decision
        if let Some(session_id) = metadata.session_id.as_deref() {
            let store = FileAgentSessionStore::new_default();
            match store.load(session_id) {
                Ok(mut session) => {
                    let summary = format!("Reflection: {}", &reflection.corrective_strategy);
                    let improvement_note = reflection
                        .improvement_score
                        .map(|score| format!("\nObserved improvement: {:.0}%", score * 100.0));
                    let rationale = Some(
                        format!(
                            "Issue: {}\nStrategy: {}",
                            reflection.failure_analysis, reflection.corrective_strategy
                        ) + &improvement_note.unwrap_or_default(),
                    );
                    let mut tags = reflection.tags.clone();
                    tags.push("reflection".to_string());

                    session
                        .state
                        .working_memory
                        .remember_decision(summary, rationale, tags);

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
            let confidence = reflection
                .improvement_score
                .map(|score| (0.55 + (score * 0.40)).clamp(0.55, 0.95))
                .unwrap_or(0.65);

            if confidence >= self.pipeline_config.reflection.promotion_confidence {
                let content = format!(
                    "## Reflection\n\n\
                     **Attempted:** {}\n\n\
                     **Issue:** {}\n\n\
                     **Corrective Strategy:** {}\n\n\
                     **Observed Improvement:** {}\n",
                    reflection.attempt_summary,
                    reflection.failure_analysis,
                    reflection.corrective_strategy,
                    reflection
                        .improvement_score
                        .map(|score| format!("{:.0}%", score * 100.0))
                        .unwrap_or_else(|| "not measured".to_string()),
                );

                let entry = MemoryBankEntry::new(
                    reflection.session_id.clone(),
                    format!("Reflection: {}", reflection.corrective_strategy),
                    content,
                )
                .with_memory_type(MemoryType::Reflection)
                .with_scope(MemoryScope::Session)
                .with_category("reflection")
                .with_provenance(
                    metadata.task_id.clone(),
                    metadata.directive_id.clone(),
                    metadata.agent_id.clone(),
                )
                .with_tags(reflection.tags.clone())
                .with_promotion(
                    reflection.session_id.clone(),
                    "ERL-inspired experiential reflection promoted from session",
                )
                .with_confidence(confidence);

                match crate::memory_bank::save_to_memory_bank(workspace_dir, &entry).await {
                    Ok(path) => {
                        promoted = true;
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

        (stored, promoted)
    }
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

#[cfg(test)]
mod tests {
    use super::{
        build_reflection_retry_prompt, format_tool_results_for_retry, truncate_retry_text,
    };
    use gestura_core_foundation::context::ResolvedContext;
    use gestura_core_pipeline::reflection::AgentReflection;
    use gestura_core_pipeline::types::{AgentResponse, ToolCallRecord, ToolResult};

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
    fn format_tool_results_handles_empty_calls() {
        let summary = format_tool_results_for_retry(&[]);
        assert!(summary.contains("No tools were used"));
    }

    #[test]
    fn truncate_retry_text_adds_ellipsis() {
        let shortened = truncate_retry_text("abcdefghijklmnopqrstuvwxyz", 8);
        assert_eq!(shortened, "abcdefgh…");
    }
}
