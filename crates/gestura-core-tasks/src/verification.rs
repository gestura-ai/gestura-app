#![cfg(feature = "advanced-primitives")]

//! Verification helpers for advanced planning primitives.

use serde::{Deserialize, Serialize};
use std::future::Future;

/// Configuration for the Ralph-style verification loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationLoopConfig {
    /// Whether the verification loop should run.
    pub enabled: bool,
    /// Maximum number of automatic retries after the initial attempt.
    pub max_automatic_retries: u8,
}

impl Default for VerificationLoopConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_automatic_retries: 2,
        }
    }
}

/// Verification requirements for prompt-like outputs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptVerificationTargets {
    /// Headings that must appear in the generated artifact.
    pub required_headings: Vec<String>,
    /// Phrases that must appear to preserve important runtime guarantees.
    pub required_phrases: Vec<String>,
    /// Require the headings to appear in-order.
    pub require_ordered_headings: bool,
    /// Require an explicit verification-oriented section.
    pub require_verification_gate: bool,
}

/// Result of verifying a single attempt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationCheck {
    /// Whether the candidate passed verification.
    pub accepted: bool,
    /// Missing requirements that should be repaired before retry.
    pub missing_requirements: Vec<String>,
}

/// Record of one verification attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationAttempt {
    /// Attempt number beginning at zero.
    pub attempt: u8,
    /// Generated candidate that was checked.
    pub candidate: String,
    /// Verification result for this candidate.
    pub check: VerificationCheck,
}

/// Aggregate verification report covering retries and final status.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationReport {
    /// Whether the final candidate passed verification.
    pub passed: bool,
    /// How many automatic retries were consumed.
    pub automatic_retries_used: u8,
    /// Full attempt history.
    pub attempts: Vec<VerificationAttempt>,
    /// Missing requirements on the final attempt, if any remain.
    pub final_missing_requirements: Vec<String>,
}

/// Ralph-style verification loop with up to two automatic repairs.
#[derive(Debug, Clone)]
pub struct VerificationLoop {
    config: VerificationLoopConfig,
}

impl VerificationLoop {
    /// Create a new verification loop with the provided configuration.
    pub fn new(config: VerificationLoopConfig) -> Self {
        Self { config }
    }

    /// Run the verification loop over a generated candidate.
    pub async fn run<Produce, ProduceFuture, Verify, VerifyFuture>(
        &self,
        mut produce: Produce,
        mut verify: Verify,
    ) -> VerificationReport
    where
        Produce: FnMut(u8, Option<&VerificationCheck>) -> ProduceFuture,
        ProduceFuture: Future<Output = String>,
        Verify: FnMut(u8, &str) -> VerifyFuture,
        VerifyFuture: Future<Output = VerificationCheck>,
    {
        if !self.config.enabled {
            let candidate = produce(0, None).await;
            return VerificationReport {
                passed: true,
                automatic_retries_used: 0,
                attempts: vec![VerificationAttempt {
                    attempt: 0,
                    candidate,
                    check: VerificationCheck {
                        accepted: true,
                        missing_requirements: Vec::new(),
                    },
                }],
                final_missing_requirements: Vec::new(),
            };
        }

        let mut attempts = Vec::new();
        let mut previous_check: Option<VerificationCheck> = None;

        for attempt in 0..=self.config.max_automatic_retries {
            let candidate = produce(attempt, previous_check.as_ref()).await;
            let check = verify(attempt, &candidate).await;
            let accepted = check.accepted;
            attempts.push(VerificationAttempt {
                attempt,
                candidate,
                check: check.clone(),
            });

            if accepted {
                return VerificationReport {
                    passed: true,
                    automatic_retries_used: attempt,
                    attempts,
                    final_missing_requirements: Vec::new(),
                };
            }

            previous_check = Some(check);
        }

        let final_missing_requirements = previous_check
            .map(|check| check.missing_requirements)
            .unwrap_or_default();
        VerificationReport {
            passed: false,
            automatic_retries_used: self.config.max_automatic_retries,
            attempts,
            final_missing_requirements,
        }
    }
}

/// Verify a planning prompt against required headings and phrases.
pub fn verify_prompt(candidate: &str, targets: &PromptVerificationTargets) -> VerificationCheck {
    let lowered = candidate.to_ascii_lowercase();
    let mut missing_requirements = Vec::new();
    let mut heading_positions = Vec::new();

    for heading in &targets.required_headings {
        let needle = heading.to_ascii_lowercase();
        match lowered.find(&needle) {
            Some(position) => heading_positions.push(position),
            None => missing_requirements.push(format!("missing heading `{heading}`")),
        }
    }

    if targets.require_ordered_headings
        && heading_positions
            .windows(2)
            .any(|window| window[0] > window[1])
    {
        missing_requirements.push("required headings are out of order".to_string());
    }

    for phrase in &targets.required_phrases {
        if !lowered.contains(&phrase.to_ascii_lowercase()) {
            missing_requirements.push(format!("missing phrase `{phrase}`"));
        }
    }

    if targets.require_verification_gate && !lowered.contains("verification gate:") {
        missing_requirements.push("missing explicit verification gate".to_string());
    }

    VerificationCheck {
        accepted: missing_requirements.is_empty(),
        missing_requirements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verification_loop_repairs_until_prompt_passes() {
        let loop_runner = VerificationLoop::new(VerificationLoopConfig::default());
        let report = loop_runner
            .run(
                |attempt, _| async move {
                    if attempt == 0 {
                        "Intent anchor:\nCompletion guardrails:".to_string()
                    } else {
                        "Intent anchor:\nOrdered execution phases:\nVerification gate:\nCompletion guardrails:\nkeep subtasks ordered and deduplicated\ndo not mark implementation complete before verification evidence exists".to_string()
                    }
                },
                |_, candidate| {
                    let candidate = candidate.to_string();
                    async move {
                        verify_prompt(
                            &candidate,
                            &PromptVerificationTargets {
                                required_headings: vec![
                                    "Intent anchor:".to_string(),
                                    "Ordered execution phases:".to_string(),
                                    "Verification gate:".to_string(),
                                    "Completion guardrails:".to_string(),
                                ],
                                required_phrases: vec![
                                    "keep subtasks ordered and deduplicated".to_string(),
                                    "do not mark implementation complete before verification evidence exists"
                                        .to_string(),
                                ],
                                require_ordered_headings: true,
                                require_verification_gate: true,
                            },
                        )
                    }
                },
            )
            .await;

        assert!(report.passed);
        assert_eq!(report.automatic_retries_used, 1);
        assert_eq!(report.attempts.len(), 2);
    }

    #[test]
    fn verify_prompt_reports_missing_sections() {
        let check = verify_prompt(
            "Intent anchor:\nCompletion guardrails:",
            &PromptVerificationTargets {
                required_headings: vec![
                    "Intent anchor:".to_string(),
                    "Ordered execution phases:".to_string(),
                    "Verification gate:".to_string(),
                    "Completion guardrails:".to_string(),
                ],
                required_phrases: vec!["keep subtasks ordered and deduplicated".to_string()],
                require_ordered_headings: true,
                require_verification_gate: true,
            },
        );

        assert!(!check.accepted);
        assert!(
            check
                .missing_requirements
                .iter()
                .any(|entry| entry.contains("Ordered execution phases"))
        );
    }
}
