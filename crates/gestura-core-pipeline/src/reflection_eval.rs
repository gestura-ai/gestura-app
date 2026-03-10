//! Fixture-based reflection evaluation harness.
//!
//! This module lets us score the current reflection heuristics against canned
//! scenarios without requiring a live model/provider call.

use gestura_core_foundation::context::ResolvedContext;
use serde::{Deserialize, Serialize};

use crate::reflection::{quality_signals_for_response, score_reflection_improvement};
use crate::types::{AgentResponse, ToolCallRecord, ToolResult};

/// Tool outcome used in a fixture-based reflection evaluation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReflectionEvalToolOutcome {
    Success,
    Error,
    Skipped,
}

/// Compact fixture representing one tool result for an evaluation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionEvalToolResult {
    pub name: String,
    pub outcome: ReflectionEvalToolOutcome,
    pub detail: String,
}

/// Compact fixture representing an initial or revised answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionEvalTurn {
    pub content: String,
    pub tool_results: Vec<ReflectionEvalToolResult>,
    pub truncated: bool,
    pub iterations: usize,
}

impl ReflectionEvalTurn {
    fn to_agent_response(&self) -> AgentResponse {
        AgentResponse {
            content: self.content.clone(),
            thinking: None,
            tool_calls: self
                .tool_results
                .iter()
                .enumerate()
                .map(|(index, tool)| ToolCallRecord {
                    id: format!("eval-tool-{index}"),
                    name: tool.name.clone(),
                    arguments: "{}".to_string(),
                    result: match tool.outcome {
                        ReflectionEvalToolOutcome::Success => {
                            ToolResult::Success(tool.detail.clone())
                        }
                        ReflectionEvalToolOutcome::Error => ToolResult::Error(tool.detail.clone()),
                        ReflectionEvalToolOutcome::Skipped => {
                            ToolResult::Skipped(tool.detail.clone())
                        }
                    },
                    duration_ms: 10,
                })
                .collect(),
            usage: None,
            context_used: ResolvedContext::default(),
            truncated: self.truncated,
            iterations: self.iterations,
        }
    }
}

/// A canned scenario for evaluating reflection-guided retry quality.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionEvalCase {
    pub name: String,
    pub request_summary: String,
    pub initial: ReflectionEvalTurn,
    pub retry: ReflectionEvalTurn,
    pub max_iterations: usize,
    pub expected_min_improvement: f32,
}

/// Report for one reflection evaluation case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionEvalReport {
    pub name: String,
    pub request_summary: String,
    pub initial_quality_score: f32,
    pub retry_quality_score: f32,
    pub improvement_score: f32,
    pub passed: bool,
}

/// Summary report for a batch of reflection evaluation cases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionEvalSummary {
    pub reports: Vec<ReflectionEvalReport>,
    pub total_cases: usize,
    pub passed_cases: usize,
    pub average_improvement_score: f32,
}

/// Evaluate one canned reflection case.
pub fn evaluate_reflection_case(case: &ReflectionEvalCase) -> ReflectionEvalReport {
    let initial_quality_score =
        quality_signals_for_response(&case.initial.to_agent_response(), case.max_iterations)
            .score();
    let retry_quality_score =
        quality_signals_for_response(&case.retry.to_agent_response(), case.max_iterations).score();
    let improvement_score =
        score_reflection_improvement(initial_quality_score, retry_quality_score);

    ReflectionEvalReport {
        name: case.name.clone(),
        request_summary: case.request_summary.clone(),
        initial_quality_score,
        retry_quality_score,
        improvement_score,
        passed: improvement_score >= case.expected_min_improvement,
    }
}

/// Evaluate a batch of reflection cases.
pub fn evaluate_reflection_cases(cases: &[ReflectionEvalCase]) -> ReflectionEvalSummary {
    let reports: Vec<_> = cases.iter().map(evaluate_reflection_case).collect();
    let total_cases = reports.len();
    let passed_cases = reports.iter().filter(|report| report.passed).count();
    let average_improvement_score = if total_cases == 0 {
        0.0
    } else {
        reports
            .iter()
            .map(|report| report.improvement_score)
            .sum::<f32>()
            / total_cases as f32
    };

    ReflectionEvalSummary {
        reports,
        total_cases,
        passed_cases,
        average_improvement_score,
    }
}

/// Built-in scenarios for regression-checking reflection effectiveness.
pub fn builtin_reflection_eval_cases() -> Vec<ReflectionEvalCase> {
    vec![
        ReflectionEvalCase {
            name: "tool_failure_becomes_actionable".to_string(),
            request_summary: "User asked for a file that was looked up with the wrong path".to_string(),
            initial: ReflectionEvalTurn {
                content: "I'm sorry, I can't access that file right now.".to_string(),
                tool_results: vec![ReflectionEvalToolResult {
                    name: "file".to_string(),
                    outcome: ReflectionEvalToolOutcome::Error,
                    detail: "config/app.toml does not exist".to_string(),
                }],
                truncated: false,
                iterations: 2,
            },
            retry: ReflectionEvalTurn {
                content: "The file lookup failed because `config/app.toml` does not exist. Please confirm the correct path and I can continue from there.".to_string(),
                tool_results: vec![ReflectionEvalToolResult {
                    name: "file".to_string(),
                    outcome: ReflectionEvalToolOutcome::Error,
                    detail: "config/app.toml does not exist".to_string(),
                }],
                truncated: false,
                iterations: 1,
            },
            max_iterations: 4,
            expected_min_improvement: 0.20,
        },
        ReflectionEvalCase {
            name: "truncated_answer_is_completed".to_string(),
            request_summary: "Agent response was cut off before giving the final answer".to_string(),
            initial: ReflectionEvalTurn {
                content: "Here are the first two steps, but the rest was truncated".to_string(),
                tool_results: Vec::new(),
                truncated: true,
                iterations: 3,
            },
            retry: ReflectionEvalTurn {
                content: "Here are all four steps, followed by the exact next action and the validation command to run.".to_string(),
                tool_results: Vec::new(),
                truncated: false,
                iterations: 1,
            },
            max_iterations: 4,
            expected_min_improvement: 0.15,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{
        builtin_reflection_eval_cases, evaluate_reflection_case, evaluate_reflection_cases,
    };

    #[test]
    fn builtin_reflection_eval_cases_pass() {
        let summary = evaluate_reflection_cases(&builtin_reflection_eval_cases());
        assert_eq!(summary.total_cases, 2);
        assert_eq!(summary.passed_cases, 2);
        assert!(summary.average_improvement_score > 0.15);
    }

    #[test]
    fn regression_case_fails_when_retry_does_not_improve() {
        let mut case = builtin_reflection_eval_cases().remove(0);
        case.retry.content = case.initial.content.clone();
        let report = evaluate_reflection_case(&case);
        assert!(!report.passed);
        assert_eq!(report.improvement_score, 0.0);
    }
}
