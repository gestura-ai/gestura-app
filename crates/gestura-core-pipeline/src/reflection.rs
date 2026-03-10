// Component 2: Reflection Data Structures
//! ERL-inspired experiential reflection types and logic.
//!
//! This module implements concepts from Experiential Reinforcement Learning (ERL)
//! adapted for an LLM agent pipeline. Instead of training-time RL, it provides:
//!
//! - **Quality gating**: only trigger reflection on suboptimal responses
//! - **Structured reflection generation**: LLM-powered analysis of what went wrong
//! - **Cross-episode memory**: reflections stored for retrieval in future turns
//! - **Consolidation**: high-confidence reflections promoted to long-term memory

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{AgentResponse, ToolResult};

/// Configuration for the experiential reflection system.
///
/// Maps to ERL paper concepts:
/// - `quality_threshold` → τ (gated reflection trigger)
/// - `max_injected_reflections` → cross-episode memory retrieval limit
/// - `promotion_confidence` → consolidation gate for long-term storage
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
    /// Maximum number of reflection-guided revision attempts per turn.
    ///
    /// These retries are text-only revisions (no additional tool execution)
    /// so the agent can safely improve a weak answer without replaying
    /// side-effectful actions.
    pub max_retry_attempts: usize,
    /// Minimum confidence for a reflection to be promoted to long-term memory.
    pub promotion_confidence: f32,
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,         // Opt-in
            quality_threshold: 0.6, // Trigger reflection below 60% quality
            max_injected_reflections: 3,
            max_retry_attempts: 1,
            promotion_confidence: 0.75,
        }
    }
}

/// A structured reflection generated after a suboptimal agent turn.
///
/// Mirrors ERL's Δ (reflection) that captures what went wrong and how
/// to improve, enabling the agent to avoid repeating the same mistakes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReflection {
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
    pub tags: Vec<String>,
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
        Self {
            attempt_summary: attempt_summary.into(),
            failure_analysis: failure_analysis.into(),
            corrective_strategy: corrective_strategy.into(),
            improvement_score: None,
            tags: Vec::new(),
            session_id: session_id.into(),
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

    /// Format as a prompt section for context injection.
    pub fn to_prompt_section(&self) -> String {
        let improvement = self.improvement_score.map(|score| {
            format!(
                "             - Observed improvement after retry: {:.0}%\n",
                score * 100.0
            )
        });

        format!(
            "**Reflection** ({})\n\
             - Attempted: {}\n\
             - Issue: {}\n\
             - Strategy: {}\n{}",
            self.timestamp.format("%Y-%m-%d %H:%M UTC"),
            self.attempt_summary,
            self.failure_analysis,
            self.corrective_strategy,
            improvement.unwrap_or_default(),
        )
    }
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
/// This corresponds to ERL's reflection generation step: given the initial
/// attempt and environment feedback, produce a structured analysis.
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
         TAGS: [comma-separated relevant tags]\n",
    );

    prompt
}

/// Parse a structured reflection from an LLM response.
///
/// Expects the format produced by `build_reflection_prompt`.
/// Returns `None` if the response cannot be parsed.
pub fn parse_reflection_response(response: &str, session_id: &str) -> Option<AgentReflection> {
    let mut attempt = None;
    let mut issue = None;
    let mut strategy = None;
    let mut tags = Vec::new();

    for line in response.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("ATTEMPT:") {
            attempt = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("ISSUE:") {
            issue = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("STRATEGY:") {
            strategy = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("TAGS:") {
            tags = value
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
        }
    }

    let attempt = attempt?;
    let issue = issue?;
    let strategy = strategy?;

    Some(AgentReflection::new(session_id, attempt, issue, strategy).with_tags(tags))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_reflection_to_prompt_section() {
        let reflection = AgentReflection::new(
            "s1",
            "Read missing file",
            "File did not exist",
            "Check file existence first",
        );
        let section = reflection.to_prompt_section();
        assert!(section.contains("Read missing file"));
        assert!(section.contains("File did not exist"));
        assert!(section.contains("Check file existence first"));
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
}
