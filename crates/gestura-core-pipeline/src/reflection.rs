//! ERL-inspired experiential reflection types and pure helpers.
//!
//! This module adapts the high-level loop from Experiential Reinforcement
//! Learning (ERL) into Gestura's pipeline model:
//!
//! 1. **Experience** — the agent makes an initial attempt and observes tool
//!    outcomes plus any obvious failure signals.
//! 2. **Reflection** — when the response quality score falls below a configured
//!    threshold, the runtime asks the model for a structured explanation of what
//!    went wrong and how to improve.
//! 3. **Consolidation** — the resulting reflection can be reused within the same
//!    turn, stored in session working memory, and promoted into long-term memory
//!    for later prompt injection.
//!
//! This crate owns the *portable* pieces of that design:
//!
//! - reflection configuration and data structures
//! - quality-signal extraction and heuristic scoring
//! - prompt construction for reflection generation
//! - parsing of the structured reflection response format
//!
//! The concrete runtime integration lives in
//! `gestura-core/src/pipeline/reflection.rs`, which wires these helpers into the
//! agent loop, streaming events, session storage, and memory-bank promotion.

use chrono::{DateTime, Utc};
use gestura_core_foundation::OutcomeSignal;
use serde::{Deserialize, Serialize};

use crate::types::{AgentResponse, ToolResult};

/// Configuration for the experiential reflection system.
///
/// The settings map the ERL-inspired design onto Gestura's runtime behavior:
///
/// - `enabled` keeps the feature opt-in because it adds an extra LLM call on
///   weak turns and can therefore increase latency/cost.
/// - `quality_threshold` maps to ERL's τ-style gate for deciding when a turn is
///   poor enough to merit reflection.
/// - `max_injected_reflections` limits how much cross-episode corrective memory
///   can be injected back into future prompts.
/// - `max_retry_attempts` bounds same-turn corrective retries. A retry may be a
///   text-only revision or one safe read-only re-execution driven by the
///   reflection strategy, but the runtime still caps it to a single retry.
/// - `promotion_confidence` gates whether a reflection is strong enough to move
///   from short-term/session memory into long-term memory-bank storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionConfig {
    /// Enable the reflection phase in the agent loop.
    pub enabled: bool,
    /// Quality threshold (0.0–1.0). Reflection only triggers when
    /// the response quality score falls below this value.
    /// Maps to ERL's τ parameter (gated reflection).
    pub quality_threshold: f32,
    /// Maximum number of past reflections to inject into prompt context.
    pub max_injected_reflections: usize,
    /// Maximum number of reflection-guided corrective retries per turn.
    ///
    /// The runtime currently applies at most one bounded retry. That retry may
    /// be a text-only revision or one safe re-execution with read-only tool
    /// policy.
    pub max_retry_attempts: usize,
    /// Minimum confidence for a reflection to be promoted to long-term memory.
    pub promotion_confidence: f32,
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,          // On by default
            quality_threshold: 0.6, // Trigger reflection below 60% quality
            max_injected_reflections: 3,
            max_retry_attempts: 1,
            promotion_confidence: 0.75,
        }
    }
}

/// A structured reflection generated after a suboptimal agent turn.
///
/// This is Gestura's durable representation of ERL's corrective reflection: a
/// concise summary of the attempted action, the failure mode, and the strategy
/// the agent should apply next time.
///
/// The runtime can:
///
/// - use it immediately for a same-turn retry,
/// - store it in session working memory as short-term corrective context, and
/// - promote it into `MemoryType::Reflection` for retrieval in future turns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReflection {
    /// Stable identifier so downstream outcomes can update the same reflection.
    pub reflection_id: String,
    /// What the agent attempted.
    pub attempt_summary: String,
    /// What went wrong or was suboptimal.
    pub failure_analysis: String,
    /// Concrete corrective strategy for future attempts.
    pub corrective_strategy: String,
    /// Quality improvement score (0.0–1.0) — did the reflection help?
    /// Set after a subsequent attempt to measure improvement.
    pub improvement_score: Option<f32>,
    /// Tags for retrieval (tool names, error categories, task types).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Durable outcome signals linked back from retries, gates, and task outcomes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcome_signals: Vec<OutcomeSignal>,
    /// Session context.
    pub session_id: String,
    /// Task ID if available.
    pub task_id: Option<String>,
    /// When this reflection was created.
    pub timestamp: DateTime<Utc>,
}

impl AgentReflection {
    /// Create a new reflection.
    pub fn new(
        session_id: impl Into<String>,
        attempt_summary: impl Into<String>,
        failure_analysis: impl Into<String>,
        corrective_strategy: impl Into<String>,
    ) -> Self {
        let session_id = session_id.into();
        Self {
            reflection_id: build_reflection_id(&session_id),
            attempt_summary: attempt_summary.into(),
            failure_analysis: failure_analysis.into(),
            corrective_strategy: corrective_strategy.into(),
            improvement_score: None,
            tags: Vec::new(),
            outcome_signals: Vec::new(),
            session_id,
            task_id: None,
            timestamp: Utc::now(),
        }
    }

    /// Attach tags for retrieval.
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Attach a task ID.
    pub fn with_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    /// Attach durable outcome signals for corrective-learning provenance.
    #[must_use]
    pub fn with_outcome_signals(mut self, outcome_signals: Vec<OutcomeSignal>) -> Self {
        self.outcome_signals = outcome_signals;
        self
    }

    /// Record a single durable outcome signal.
    pub fn push_outcome_signal(&mut self, signal: OutcomeSignal) {
        self.outcome_signals = merge_outcome_signals(&self.outcome_signals, &[signal]);
    }

    /// Score how strongly this reflection should be promoted into durable memory.
    #[must_use]
    pub fn promotion_confidence(&self) -> f32 {
        reflection_promotion_confidence(self.improvement_score, &self.outcome_signals)
    }

    /// Format as a prompt section for context injection.
    pub fn to_prompt_section(&self) -> String {
        let improvement = self.improvement_score.map(|score| {
            format!(
                "             - Observed improvement after retry: {:.0}%\n",
                score * 100.0
            )
        });
        let outcomes = if self.outcome_signals.is_empty() {
            String::new()
        } else {
            format!(
                "             - Outcomes: {}\n",
                self.outcome_signals
                    .iter()
                    .map(|signal| signal.kind.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        format!(
            "**Reflection** ({})\n\
             - Attempted: {}\n\
             - Issue: {}\n\
             - Strategy: {}\n{}{}",
            self.timestamp.format("%Y-%m-%d %H:%M UTC"),
            self.attempt_summary,
            self.failure_analysis,
            self.corrective_strategy,
            improvement.unwrap_or_default(),
            outcomes,
        )
    }
}

fn build_reflection_id(session_id: &str) -> String {
    let nanos = Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or_else(|| Utc::now().timestamp_micros() * 1_000);
    let session_fragment = session_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    if session_fragment.is_empty() {
        format!("reflection-{nanos}")
    } else {
        format!("reflection-{session_fragment}-{nanos}")
    }
}

/// Score how strongly a reflection should be promoted into durable memory.
#[must_use]
pub fn reflection_promotion_confidence(
    improvement_score: Option<f32>,
    outcome_signals: &[OutcomeSignal],
) -> f32 {
    let base = improvement_score
        .map(|score| (0.55 + (score * 0.35)).clamp(0.55, 0.90))
        .unwrap_or(0.62);
    let delta: f32 = outcome_signals
        .iter()
        .map(|signal| signal.kind.confidence_delta())
        .sum();
    (base + delta).clamp(0.30, 0.97)
}

/// Merge outcome signals, replacing older entries of the same kind with the newest.
#[must_use]
pub fn merge_outcome_signals(
    existing: &[OutcomeSignal],
    incoming: &[OutcomeSignal],
) -> Vec<OutcomeSignal> {
    let mut merged = existing.to_vec();
    for signal in incoming {
        if let Some(slot) = merged
            .iter_mut()
            .find(|current| current.kind == signal.kind)
        {
            *slot = signal.clone();
        } else {
            merged.push(signal.clone());
        }
    }
    merged.sort_by_key(|signal| signal.observed_at);
    merged
}

/// Score how much a reflection-guided retry improved quality.
///
/// Returns a normalized 0.0–1.0 value where:
/// - `0.0` means no measurable improvement
/// - `1.0` means the retry closed the entire remaining quality gap
pub fn score_reflection_improvement(initial_quality: f32, retry_quality: f32) -> f32 {
    if retry_quality <= initial_quality {
        return 0.0;
    }

    let available_headroom = (1.0 - initial_quality).max(f32::EPSILON);
    ((retry_quality - initial_quality) / available_headroom).clamp(0.0, 1.0)
}

/// Build heuristic quality signals from an agent response.
///
/// These signals stand in for the explicit reward signal used in ERL. Gestura
/// does not have a verifiable task reward for most agent turns, so the runtime
/// falls back to observable quality proxies such as tool errors, iteration
/// pressure, truncation, and explicit failure language.
pub fn quality_signals_for_response(
    response: &AgentResponse,
    max_iterations: usize,
) -> QualitySignals {
    let total_tool_calls = response.tool_calls.len();
    let error_count = response
        .tool_calls
        .iter()
        .filter(|tc| matches!(&tc.result, ToolResult::Error(_)))
        .count();

    let tool_error_rate = if total_tool_calls > 0 {
        error_count as f32 / total_tool_calls as f32
    } else {
        0.0
    };

    QualitySignals {
        tool_error_rate,
        iterations_used: response.iterations,
        max_iterations,
        was_truncated: response.truncated,
        has_failure_patterns: detect_failure_patterns(&response.content),
        is_empty_response: response.content.trim().is_empty(),
    }
}

/// Heuristic quality signals extracted from an agent response.
///
/// These signals drive the quality gate (ERL's τ threshold) that
/// determines whether a reflection should be triggered.
#[derive(Debug, Clone)]
pub struct QualitySignals {
    /// Fraction of tool calls that resulted in errors (0.0–1.0).
    pub tool_error_rate: f32,
    /// Number of agentic loop iterations used.
    pub iterations_used: usize,
    /// Maximum iterations configured.
    pub max_iterations: usize,
    /// Whether the response was truncated due to token limits.
    pub was_truncated: bool,
    /// Whether the response contains apology/failure patterns.
    pub has_failure_patterns: bool,
    /// Whether the response is empty or near-empty.
    pub is_empty_response: bool,
}

impl QualitySignals {
    /// Compute an aggregate quality score (0.0–1.0, higher is better).
    ///
    /// This is the heuristic that replaces ERL's verifiable reward signal.
    /// In our setting we don't have a ground-truth reward, so we use
    /// observable proxy signals from the agent's execution.
    pub fn score(&self) -> f32 {
        if self.is_empty_response {
            return 0.0;
        }

        let mut score: f32 = 1.0;

        // Tool errors are a strong negative signal
        score -= self.tool_error_rate * 0.4;

        // Using many iterations suggests the agent is struggling
        if self.max_iterations > 0 {
            let iteration_ratio = self.iterations_used as f32 / self.max_iterations as f32;
            if iteration_ratio > 0.7 {
                score -= (iteration_ratio - 0.7) * 0.5;
            }
        }

        // Truncation indicates context overflow issues
        if self.was_truncated {
            score -= 0.15;
        }

        // Explicit failure/apology patterns
        if self.has_failure_patterns {
            score -= 0.25;
        }

        score.clamp(0.0, 1.0)
    }
}

/// Check if response text contains common failure/apology patterns.
pub fn detect_failure_patterns(text: &str) -> bool {
    let lower = text.to_lowercase();
    let patterns = [
        "i'm sorry, i can't",
        "i cannot",
        "i'm unable to",
        "unfortunately, i",
        "i don't have the ability",
        "i apologize, but i",
        "i'm not able to",
        "error occurred",
        "failed to execute",
    ];
    patterns.iter().any(|p| lower.contains(p))
}

/// Build the reflection prompt that asks the LLM to analyze a suboptimal turn.
///
/// This is the pure prompt-construction step for reflection generation:
///
/// - the original user request becomes the task context,
/// - the agent response becomes the failed/suboptimal attempt,
/// - tool errors and quality signals become the environment feedback,
/// - and the output contract forces the model into the structured
///   `ATTEMPT`/`ISSUE`/`STRATEGY`/`TAGS` format expected by the parser.
pub fn build_reflection_prompt(
    user_request: &str,
    agent_response: &str,
    quality_signals: &QualitySignals,
    tool_errors: &[String],
) -> String {
    let mut prompt = String::from(
        "System: You are a self-reflective AI assistant analyzing a previous interaction \
         that was suboptimal. Generate a structured reflection to improve future responses.\n\n",
    );

    prompt.push_str(&format!("User request: {}\n\n", user_request));
    prompt.push_str(&format!(
        "Agent response (quality score: {:.2}):\n{}\n\n",
        quality_signals.score(),
        agent_response
    ));

    if !tool_errors.is_empty() {
        prompt.push_str("Tool errors encountered:\n");
        for error in tool_errors {
            prompt.push_str(&format!("- {}\n", error));
        }
        prompt.push('\n');
    }

    prompt.push_str(
        "Provide a brief, structured reflection in the following format:\n\
         ATTEMPT: [1-2 sentence summary of what was attempted]\n\
         ISSUE: [1-2 sentence analysis of what went wrong]\n\
         STRATEGY: [1-2 sentence corrective strategy for future attempts]\n\
         TAGS: [comma-separated relevant tags]\n\
         Important:\n\
         - Output plain text only.\n\
         - Do not wrap the reflection in Markdown code fences.\n\
         - Do not add any preamble, explanation, or extra sections before or after the four fields.\n",
    );

    prompt
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReflectionField {
    Attempt,
    Issue,
    Strategy,
    Tags,
}

fn strip_tag_blocks(input: &str, open: &str, close: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0usize;

    while let Some(start_rel) = input[cursor..].find(open) {
        let start = cursor + start_rel;
        output.push_str(&input[cursor..start]);
        let content_start = start + open.len();
        let Some(end_rel) = input[content_start..].find(close) else {
            return output.trim().to_string();
        };
        cursor = content_start + end_rel + close.len();
    }

    output.push_str(&input[cursor..]);
    output.trim().to_string()
}

fn sanitize_reflection_response(response: &str) -> String {
    strip_tag_blocks(response, "<think>", "</think>")
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn compact_reflection_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_reflection_field_value(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(',')
        .trim_matches(|c: char| matches!(c, '*' | '_' | '`' | '"' | '\''))
        .trim()
        .to_string()
}

fn push_reflection_segment(target: &mut String, segment: &str) {
    let segment = compact_reflection_value(&clean_reflection_field_value(segment));
    if segment.is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push(' ');
    }
    target.push_str(&segment);
}

fn normalize_reflection_label(label: &str) -> Option<ReflectionField> {
    let normalized = label
        .trim()
        .trim_matches(|c: char| matches!(c, '*' | '_' | '`' | '[' | ']' | '(' | ')' | '#'))
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if matches!(ch, ' ' | '-' | '_') {
                Some('_')
            } else {
                None
            }
        })
        .collect::<String>();

    let normalized = normalized.trim_matches('_');

    match normalized {
        "attempt" | "attempt_summary" | "summary" | "what_was_attempted" => {
            Some(ReflectionField::Attempt)
        }
        "issue" | "failure" | "failure_analysis" | "problem" | "analysis" | "what_went_wrong" => {
            Some(ReflectionField::Issue)
        }
        "strategy"
        | "corrective_strategy"
        | "correction"
        | "fix"
        | "improvement_strategy"
        | "next_time" => Some(ReflectionField::Strategy),
        "tags" | "labels" => Some(ReflectionField::Tags),
        _ => None,
    }
}

fn strip_reflection_line_prefix(line: &str) -> &str {
    let mut trimmed = line.trim_start();

    loop {
        if let Some(rest) = trimmed.strip_prefix('>') {
            trimmed = rest.trim_start();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ") {
            trimmed = rest.trim_start();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("* ") {
            trimmed = rest.trim_start();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("• ") {
            trimmed = rest.trim_start();
            continue;
        }

        let digit_count = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
        if digit_count > 0 {
            let suffix = &trimmed[digit_count..];
            if let Some(rest) = suffix.strip_prefix(". ") {
                trimmed = rest.trim_start();
                continue;
            }
            if let Some(rest) = suffix.strip_prefix(") ") {
                trimmed = rest.trim_start();
                continue;
            }
        }

        break;
    }

    trimmed
}

fn parse_reflection_field_line(line: &str) -> Option<(ReflectionField, String)> {
    let candidate = strip_reflection_line_prefix(line);
    let colon_idx = candidate.find(':')?;
    let label = candidate[..colon_idx].trim();
    let value = candidate[colon_idx + 1..].trim();
    let field = normalize_reflection_label(label)?;
    Some((field, value.to_string()))
}

fn parse_tag_values(value: &str) -> Vec<String> {
    let trimmed = strip_reflection_line_prefix(value).trim();
    let trimmed = clean_reflection_field_value(trimmed);
    let trimmed = trimmed.trim_matches(|c: char| matches!(c, '[' | ']' | '{' | '}'));
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split(',')
        .map(clean_reflection_field_value)
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn extract_jsonish_string_value(source: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = source.find(&needle)? + needle.len();
    let remainder = source[start..].trim_start();
    let remainder = remainder.strip_prefix(':')?.trim_start();
    let remainder = remainder.strip_prefix('"')?;

    let mut value = String::new();
    let mut escaped = false;
    for ch in remainder.chars() {
        if escaped {
            value.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(value.trim().to_string()),
            other => value.push(other),
        }
    }

    None
}

fn extract_jsonish_tags(source: &str) -> Option<Vec<String>> {
    if let Some(tags_str) = extract_jsonish_string_value(source, "tags") {
        return Some(parse_tag_values(&tags_str));
    }

    let needle = "\"tags\"";
    let start = source.find(needle)? + needle.len();
    let remainder = source[start..].trim_start();
    let remainder = remainder.strip_prefix(':')?.trim_start();
    let remainder = remainder.strip_prefix('[')?;
    let end = remainder.find(']')?;
    let body = &remainder[..end];

    Some(
        body.split(',')
            .map(|item| item.trim().trim_matches(|c: char| matches!(c, '"' | '\'')))
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn parse_jsonish_reflection_response(response: &str, session_id: &str) -> Option<AgentReflection> {
    let attempt = extract_jsonish_string_value(response, "attempt_summary")
        .or_else(|| extract_jsonish_string_value(response, "attempt"))?;
    let issue = extract_jsonish_string_value(response, "failure_analysis")
        .or_else(|| extract_jsonish_string_value(response, "issue"))?;
    let strategy = extract_jsonish_string_value(response, "corrective_strategy")
        .or_else(|| extract_jsonish_string_value(response, "strategy"))?;
    let tags = extract_jsonish_tags(response).unwrap_or_default();

    Some(
        AgentReflection::new(session_id, attempt, issue, strategy)
            .with_tags(tags.into_iter().filter(|tag| !tag.is_empty()).collect()),
    )
}

/// Parse a structured reflection from an LLM response.
///
/// Expects the format produced by `build_reflection_prompt` and intentionally
/// fails closed if any of the core fields are missing. The runtime treats an
/// unparsable reflection as non-durable so it does not enter session or
/// long-term memory in a malformed shape.
pub fn parse_reflection_response(response: &str, session_id: &str) -> Option<AgentReflection> {
    let response = sanitize_reflection_response(response);
    let mut attempt = String::new();
    let mut issue = String::new();
    let mut strategy = String::new();
    let mut tags = Vec::new();
    let mut current_field = None;

    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some((field, value)) = parse_reflection_field_line(trimmed) {
            current_field = Some(field);
            match field {
                ReflectionField::Attempt => push_reflection_segment(&mut attempt, &value),
                ReflectionField::Issue => push_reflection_segment(&mut issue, &value),
                ReflectionField::Strategy => push_reflection_segment(&mut strategy, &value),
                ReflectionField::Tags => tags.extend(parse_tag_values(&value)),
            }
            continue;
        }

        match current_field {
            Some(ReflectionField::Attempt) => push_reflection_segment(&mut attempt, trimmed),
            Some(ReflectionField::Issue) => push_reflection_segment(&mut issue, trimmed),
            Some(ReflectionField::Strategy) => push_reflection_segment(&mut strategy, trimmed),
            Some(ReflectionField::Tags) => tags.extend(parse_tag_values(trimmed)),
            None => {}
        }
    }

    let mut deduped_tags = Vec::new();
    for tag in tags {
        if !tag.is_empty() && !deduped_tags.contains(&tag) {
            deduped_tags.push(tag);
        }
    }

    if !attempt.is_empty() && !issue.is_empty() && !strategy.is_empty() {
        return Some(
            AgentReflection::new(session_id, attempt, issue, strategy).with_tags(deduped_tags),
        );
    }

    parse_jsonish_reflection_response(&response, session_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gestura_core_foundation::OutcomeSignalKind;

    #[test]
    fn test_quality_scoring_high_quality_response() {
        let signals = QualitySignals {
            tool_error_rate: 0.0,
            iterations_used: 1,
            max_iterations: 10,
            was_truncated: false,
            has_failure_patterns: false,
            is_empty_response: false,
        };
        let score = signals.score();
        assert!(score > 0.9, "Good response should score > 0.9, got {score}");
    }

    #[test]
    fn test_quality_scoring_tool_errors() {
        let signals = QualitySignals {
            tool_error_rate: 0.5, // Half the tools errored
            iterations_used: 3,
            max_iterations: 10,
            was_truncated: false,
            has_failure_patterns: false,
            is_empty_response: false,
        };
        let score = signals.score();
        assert!(
            score < 0.85,
            "50% tool errors should lower score, got {score}"
        );
    }

    #[test]
    fn test_quality_scoring_many_iterations() {
        let signals = QualitySignals {
            tool_error_rate: 0.0,
            iterations_used: 9,
            max_iterations: 10,
            was_truncated: false,
            has_failure_patterns: false,
            is_empty_response: false,
        };
        let score = signals.score();
        assert!(
            score < 0.95,
            "Using 90% iterations should lower score, got {score}"
        );
    }

    #[test]
    fn test_quality_scoring_empty_response() {
        let signals = QualitySignals {
            tool_error_rate: 0.0,
            iterations_used: 1,
            max_iterations: 10,
            was_truncated: false,
            has_failure_patterns: false,
            is_empty_response: true,
        };
        assert_eq!(signals.score(), 0.0);
    }

    #[test]
    fn test_quality_scoring_combined_issues() {
        let signals = QualitySignals {
            tool_error_rate: 0.3,
            iterations_used: 8,
            max_iterations: 10,
            was_truncated: true,
            has_failure_patterns: true,
            is_empty_response: false,
        };
        let score = signals.score();
        assert!(
            score < 0.5,
            "Multiple issues should produce low score, got {score}"
        );
    }

    #[test]
    fn test_detect_failure_patterns() {
        assert!(detect_failure_patterns("I'm sorry, I can't do that"));
        assert!(detect_failure_patterns(
            "Unfortunately, I cannot access that file"
        ));
        assert!(!detect_failure_patterns(
            "Here is the file content you requested"
        ));
    }

    #[test]
    fn test_reflection_prompt_construction() {
        let signals = QualitySignals {
            tool_error_rate: 0.5,
            iterations_used: 3,
            max_iterations: 10,
            was_truncated: false,
            has_failure_patterns: false,
            is_empty_response: false,
        };
        let prompt = build_reflection_prompt(
            "Read the file",
            "Error: file not found",
            &signals,
            &["FileNotFound: /tmp/missing.txt".to_string()],
        );
        assert!(prompt.contains("Read the file"));
        assert!(prompt.contains("Error: file not found"));
        assert!(prompt.contains("FileNotFound"));
        assert!(prompt.contains("ATTEMPT:"));
        assert!(prompt.contains("ISSUE:"));
        assert!(prompt.contains("STRATEGY:"));
    }

    #[test]
    fn test_reflection_response_parsing() {
        let response = "\
            ATTEMPT: Tried to read the file at /tmp/missing.txt\n\
            ISSUE: The file path was incorrect; the file does not exist\n\
            STRATEGY: Verify file existence before attempting to read; suggest alternatives\n\
            TAGS: file, read, path-error\n";

        let reflection = parse_reflection_response(response, "session-123").unwrap();
        assert_eq!(
            reflection.attempt_summary,
            "Tried to read the file at /tmp/missing.txt"
        );
        assert!(reflection.failure_analysis.contains("incorrect"));
        assert!(reflection.corrective_strategy.contains("Verify"));
        assert_eq!(reflection.tags, vec!["file", "read", "path-error"]);
        assert_eq!(reflection.session_id, "session-123");
    }

    #[test]
    fn test_reflection_response_parsing_incomplete() {
        let response = "ATTEMPT: Something\nISSUE: Something else\n";
        let reflection = parse_reflection_response(response, "s1");
        assert!(reflection.is_none(), "Missing STRATEGY should return None");
    }

    #[test]
    fn test_reflection_response_parsing_markdown_and_multiline() {
        let response = "<think>diagnosing tool output</think>\n\
            - **Attempt:** Tried to inspect the missing config file.\n\
              I answered before verifying the real path.\n\
            - **Issue:** The response relied on an assumed file location\n\
              instead of repository evidence.\n\
            - **Strategy:** Search for the config file first, then answer\n\
              only from the verified path and contents.\n\
            - **Tags:** file, verification\n";

        let reflection = parse_reflection_response(response, "session-md").unwrap();
        assert!(
            reflection
                .attempt_summary
                .contains("inspect the missing config file")
        );
        assert!(
            reflection
                .attempt_summary
                .contains("verifying the real path")
        );
        assert!(reflection.failure_analysis.contains("repository evidence"));
        assert!(
            reflection
                .corrective_strategy
                .contains("verified path and contents")
        );
        assert_eq!(reflection.tags, vec!["file", "verification"]);
    }

    #[test]
    fn test_reflection_response_parsing_aliases_and_tag_list() {
        let response = "attempt_summary: Investigated a build failure without reading the actual error output.\n\
            failure_analysis: The explanation guessed at causes instead of grounding them in the logs.\n\
            corrective_strategy: Read the concrete stderr output first, then explain only the confirmed failure mode.\n\
            tags:\n\
            - shell\n\
            - validation\n";

        let reflection = parse_reflection_response(response, "session-alias").unwrap();
        assert!(reflection.attempt_summary.contains("build failure"));
        assert!(
            reflection
                .failure_analysis
                .contains("grounding them in the logs")
        );
        assert!(
            reflection
                .corrective_strategy
                .contains("concrete stderr output")
        );
        assert_eq!(reflection.tags, vec!["shell", "validation"]);
    }

    #[test]
    fn test_reflection_response_parsing_jsonish_payload() {
        let response = "```json\n{\n  \"attempt_summary\": \"Tried to edit the wrong file\",\n  \"failure_analysis\": \"The response assumed the target path without confirming it\",\n  \"corrective_strategy\": \"Locate the file first, then apply the edit to the verified path\",\n  \"tags\": [\"file\", \"path\"]\n}\n```";

        let reflection = parse_reflection_response(response, "session-json").unwrap();
        assert_eq!(reflection.attempt_summary, "Tried to edit the wrong file");
        assert!(
            reflection
                .failure_analysis
                .contains("assumed the target path")
        );
        assert!(
            reflection
                .corrective_strategy
                .contains("Locate the file first")
        );
        assert_eq!(reflection.tags, vec!["file", "path"]);
    }

    #[test]
    fn test_reflection_to_prompt_section() {
        let reflection = AgentReflection::new(
            "s1",
            "Read missing file",
            "File did not exist",
            "Check file existence first",
        )
        .with_outcome_signals(vec![
            OutcomeSignal::new(OutcomeSignalKind::RetryImproved)
                .with_summary("The revised answer used the correct path."),
        ]);
        let section = reflection.to_prompt_section();
        assert!(section.contains("Read missing file"));
        assert!(section.contains("File did not exist"));
        assert!(section.contains("Check file existence first"));
        assert!(section.contains("Retry improved"));
    }

    #[test]
    fn test_reflection_improvement_score_increases_with_retry_quality() {
        let score = score_reflection_improvement(0.40, 0.76);
        assert!(
            score > 0.5,
            "Expected strong improvement signal, got {score}"
        );
    }

    #[test]
    fn test_reflection_improvement_score_zero_when_retry_is_not_better() {
        assert_eq!(score_reflection_improvement(0.65, 0.65), 0.0);
        assert_eq!(score_reflection_improvement(0.65, 0.52), 0.0);
    }

    #[test]
    fn test_promotion_confidence_uses_outcome_signals() {
        let baseline = AgentReflection::new(
            "s1",
            "Attempted a retry",
            "The first answer was weak",
            "Revise with the missing evidence",
        );
        let stronger = baseline.clone().with_outcome_signals(vec![
            OutcomeSignal::new(OutcomeSignalKind::RetryImproved),
            OutcomeSignal::new(OutcomeSignalKind::ReviewApproved),
        ]);
        let weaker = baseline.with_outcome_signals(vec![
            OutcomeSignal::new(OutcomeSignalKind::RetryDidNotImprove),
            OutcomeSignal::new(OutcomeSignalKind::ReviewNeedsRevision),
        ]);

        assert!(stronger.promotion_confidence() > 0.70);
        assert!(weaker.promotion_confidence() < 0.50);
    }

    #[test]
    fn test_merge_outcome_signals_replaces_existing_kind() {
        let first = OutcomeSignal::new(OutcomeSignalKind::ReviewApproved)
            .with_summary("Initial approval note");
        let replacement = OutcomeSignal::new(OutcomeSignalKind::ReviewApproved)
            .with_summary("Final approval note");

        let merged = merge_outcome_signals(&[first], std::slice::from_ref(&replacement));

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].summary.as_deref(), replacement.summary.as_deref());
    }
}
