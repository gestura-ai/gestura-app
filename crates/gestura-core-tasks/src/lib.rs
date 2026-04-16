//! Task management and reusable workflow primitives for Gestura.
//!
//! `gestura-core-tasks` owns the persistent task-list model and reusable
//! workflow definitions used by agent sessions, orchestration layers, and user
//! interfaces.
//!
//! ## Responsibilities
//!
//! - session-scoped task CRUD and persistence via `TaskManager`
//! - hierarchical task lists, task state transitions, and metadata tracking
//! - task-memory lifecycle events used to mirror memory promotions and blockers
//! - reusable markdown workflow definitions discovered by `WorkflowManager`
//!
//! ## Architecture role
//!
//! This crate is the source of truth for task and workflow domain behavior.
//! Higher-level orchestration—such as deciding when a supervisor creates or
//! blocks tasks—remains in `gestura-core`, but the underlying task graph and
//! workflow loading logic live here.
//!
//! ## Storage model
//!
//! Task state is persisted under the workspace `.gestura/` area so it can be
//! resumed across sessions. Workflow definitions are loaded from workspace-local
//! or user-level workflow directories, allowing reusable templates without
//! hard-coding them into the pipeline.
//!
//! ## Stable import paths
//!
//! Most code should import through the facade:
//!
//! - `gestura_core::tasks::*`
//! - `gestura_core::workflows::*`

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub mod tasks;
pub mod workflows;

#[cfg(feature = "advanced-primitives")]
pub mod semantic_client;
#[cfg(feature = "advanced-primitives")]
pub mod verification;

pub use tasks::get_global_task_manager;

#[cfg(feature = "advanced-primitives")]
pub use semantic_client::{
    SemanticClient, SemanticClientConfig, SemanticClientError, SemanticQueryHit,
    SemanticQueryRequest, SemanticQueryResult,
};
#[cfg(feature = "advanced-primitives")]
pub use verification::{
    PromptVerificationTargets, VerificationAttempt, VerificationCheck, VerificationLoop,
    VerificationLoopConfig, VerificationReport,
};

/// Compile-time flag exported to downstream crates so the middleware branch can
/// constant-fold away when advanced primitives are disabled.
pub const ADVANCED_PRIMITIVES_ENABLED: bool = cfg!(feature = "advanced-primitives");

/// Request envelope for the optional advanced-planning middleware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedPlanRequest {
    /// Original user intent after upstream intent parsing.
    pub user_intent: String,
    /// Base system prompt that should be preserved and augmented.
    pub base_system_prompt: String,
    /// Session identifier if the request is already session-scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Active task identifier if the request is already attached to a task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Human-readable request source for telemetry and semantic payloads.
    pub source: String,
    /// Whether the upstream runtime classified this intent as complex/multi-step.
    #[serde(default)]
    pub complex_intent: bool,
    /// Whether the request explicitly asks for verification such as build/test.
    #[serde(default)]
    pub requires_verification: bool,
    /// Opaque request-scoped hints passed through from the agent pipeline.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata_hints: HashMap<String, String>,
}

/// BOS1921 waveform bridge payload derived from Gestura's existing haptic model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bos1921Waveform {
    /// Downstream transport selector.
    pub driver: String,
    /// Semantic preset name that a BOS1921-capable bridge can map to a waveform.
    pub preset: String,
    /// Suggested amplitude percentage.
    pub amplitude_percent: u8,
    /// Suggested waveform duration.
    pub duration_ms: u32,
    /// Suggested repeat count.
    pub repeat_count: u8,
    /// Delay between repeats.
    pub repeat_delay_ms: u32,
}

/// Result of the optional advanced-planning middleware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedPlanOutcome {
    /// Whether the middleware actively augmented the request.
    pub applied: bool,
    /// Final system prompt that should continue through the normal pipeline.
    pub system_prompt: String,
    /// Additional structured hints that downstream observers or hooks may inspect.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata_hints: HashMap<String, String>,
    /// Optional haptic bridge payload for BOS1921-capable transports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bos1921_waveform: Option<Bos1921Waveform>,
}

impl AdvancedPlanOutcome {
    /// Return a no-op outcome that preserves the existing pipeline behavior.
    pub fn passthrough(system_prompt: String) -> Self {
        Self {
            applied: false,
            system_prompt,
            metadata_hints: HashMap::new(),
            bos1921_waveform: None,
        }
    }
}

/// Optional advanced task primitives for complex intent-first planning.
pub struct AdvancedPrimitives;

#[cfg(feature = "advanced-primitives")]
impl AdvancedPrimitives {
    /// Build an enhanced multi-step planning prompt while preserving the normal
    /// pipeline and observer flow.
    pub async fn run_enhanced_plan(request: AdvancedPlanRequest) -> AdvancedPlanOutcome {
        use gestura_core_foundation::interaction::HapticFeedback;

        if !request.complex_intent {
            return AdvancedPlanOutcome::passthrough(request.base_system_prompt);
        }

        let semantic_config = semantic_client_config_from_hints(&request.metadata_hints);
        let verification_config = verification_config_from_hints(&request.metadata_hints);
        let semantic_query = request
            .metadata_hints
            .get("advanced_primitives.semantic.query")
            .cloned()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| request.user_intent.clone());

        let semantic_result = if semantic_config.enabled {
            match semantic_client::SemanticClient::new(semantic_config.clone()) {
                Ok(client) => match client
                    .query(&semantic_client::SemanticQueryRequest {
                        query: semantic_query,
                        domain: semantic_config.domain.clone(),
                        session_id: request.session_id.clone(),
                        task_id: request.task_id.clone(),
                        source: request.source.clone(),
                        hints: request.metadata_hints.clone(),
                    })
                    .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        tracing::debug!(
                            ?error,
                            "advanced semantic query skipped after client error"
                        );
                        None
                    }
                },
                Err(error) => {
                    tracing::debug!(
                        ?error,
                        "advanced semantic client disabled due to invalid config"
                    );
                    None
                }
            }
        } else {
            None
        };

        let verification_targets = build_verification_targets(&request, semantic_result.as_ref());
        let semantic_snapshot = semantic_result.clone();
        let verification_loop = verification::VerificationLoop::new(verification_config.clone());
        let verification_report = verification_loop
            .run(
                |attempt, repair_notes| {
                    let request = request.clone();
                    let semantic_snapshot = semantic_snapshot.clone();
                    let repair_notes = repair_notes.cloned();
                    async move {
                        compose_enhanced_system_prompt(
                            &request,
                            semantic_snapshot.as_ref(),
                            repair_notes.as_ref(),
                            attempt,
                        )
                    }
                },
                |_, candidate| {
                    let candidate = candidate.to_string();
                    let verification_targets = verification_targets.clone();
                    async move { verification::verify_prompt(&candidate, &verification_targets) }
                },
            )
            .await;

        let final_prompt = verification_report
            .attempts
            .last()
            .map(|attempt| attempt.candidate.clone())
            .unwrap_or_else(|| {
                compose_enhanced_system_prompt(&request, semantic_result.as_ref(), None, 0)
            });

        let haptic_feedback = if verification_report.passed {
            Some(HapticFeedback::success())
        } else if semantic_result.is_some() {
            Some(HapticFeedback::notification())
        } else {
            None
        };
        let bos1921_waveform = haptic_feedback
            .as_ref()
            .map(|feedback| bos1921_from_feedback(feedback, semantic_result.is_some()));

        let mut metadata_hints = HashMap::from([
            (
                "advanced_primitives.enabled".to_string(),
                "true".to_string(),
            ),
            (
                "advanced_primitives.applied".to_string(),
                "true".to_string(),
            ),
            (
                "advanced_primitives.mode".to_string(),
                "complex_intent".to_string(),
            ),
            (
                "advanced_primitives.verification".to_string(),
                serde_json::to_string(&verification_report).unwrap_or_else(|_| "{}".to_string()),
            ),
        ]);

        if let Some(semantic_result) = &semantic_result {
            metadata_hints.insert(
                "advanced_primitives.semantic".to_string(),
                serde_json::to_string(semantic_result).unwrap_or_else(|_| "{}".to_string()),
            );
        }
        if let Some(waveform) = &bos1921_waveform {
            metadata_hints.insert(
                "advanced_primitives.bos1921".to_string(),
                serde_json::to_string(waveform).unwrap_or_else(|_| "{}".to_string()),
            );
        }

        AdvancedPlanOutcome {
            applied: true,
            system_prompt: final_prompt,
            metadata_hints,
            bos1921_waveform,
        }
    }
}

#[cfg(not(feature = "advanced-primitives"))]
impl AdvancedPrimitives {
    /// Preserve the original workflow unchanged when advanced primitives are
    /// disabled at compile time.
    pub async fn run_enhanced_plan(request: AdvancedPlanRequest) -> AdvancedPlanOutcome {
        AdvancedPlanOutcome::passthrough(request.base_system_prompt)
    }
}

#[cfg(feature = "advanced-primitives")]
fn compose_enhanced_system_prompt(
    request: &AdvancedPlanRequest,
    semantic_result: Option<&semantic_client::SemanticQueryResult>,
    repair_notes: Option<&verification::VerificationCheck>,
    attempt: u8,
) -> String {
    let mut prompt = request.base_system_prompt.clone();
    prompt.push_str("\n\nAdvanced intent middleware:\n");
    prompt.push_str("- The next user request is a complex multi-step intent. Preserve the original goal and keep the execution path modality-neutral across voice, chat, gesture, and future inputs.\n");
    prompt.push_str("- Prefer existing MCP/A2A, NATS, hooks, memory-bank, and dual-orchestrator capabilities when they are the best fit instead of inventing a parallel workflow.\n");
    prompt.push_str("Intent anchor:\n");
    prompt.push_str(&format!(
        "- Source: {}\n- Session: {}\n- Task: {}\n- User intent: {}\n",
        request.source,
        request.session_id.as_deref().unwrap_or("n/a"),
        request.task_id.as_deref().unwrap_or("n/a"),
        request.user_intent.trim()
    ));
    prompt.push_str("Reasoning approach (apply before acting):\n");
    prompt.push_str("1. Restate the exact request in one sentence to confirm understanding.\n");
    prompt.push_str("2. Identify active constraints: conciseness requirements, hedging obligations, formatting rules, permission level.\n");
    prompt.push_str("3. Decompose into ordered steps; flag any ambiguity or missing information before proceeding.\n");
    prompt.push_str("4. After drafting output, self-critique: does it fully satisfy the request? Does it violate any constraint? Revise if needed.\n");
    prompt.push_str("5. Emit only the final output — no narration of the reasoning process unless the user asked for it.\n");
    prompt.push_str("Ordered execution phases:\n");
    prompt.push_str(
        "1. Inspect the current state, constraints, permissions, and dependencies before acting.\n",
    );
    prompt.push_str("2. Prepare only the prerequisites that are truly needed, then execute the requested work in order.\n");
    prompt.push_str("3. Keep subtasks ordered and deduplicated; do not promote later phases until prerequisite work is actually complete.\n");
    prompt.push_str("4. Capture concrete evidence before claiming completion.\n");
    prompt.push_str("Verification gate:\n");
    if request.requires_verification {
        prompt.push_str("- Before you claim success, explicitly run or request the right verification steps, build/test checks, or equivalent domain validation and summarize the observed evidence.\n");
    } else {
        prompt.push_str("- Before you claim success, explicitly validate the final outcome and summarize the evidence that makes the result trustworthy.\n");
    }
    prompt.push_str("Completion guardrails:\n");
    prompt.push_str("- Do not skip prerequisite setup.\n");
    prompt.push_str("- Do not mark implementation complete before verification evidence exists.\n");
    prompt.push_str("- If a step fails, repair the plan and continue from the failed step instead of replaying completed work.\n");

    if let Some(semantic_result) = semantic_result {
        prompt.push_str("Live semantic context:\n");
        if let Some(domain) = semantic_result.domain.as_deref() {
            prompt.push_str(&format!("- Domain: {}\n", domain));
        }
        prompt.push_str(&format!("- Summary: {}\n", semantic_result.summary.trim()));
        for hit in semantic_result.hits.iter().take(2) {
            prompt.push_str(&format!(
                "- Source: {} — {}\n",
                hit.title,
                hit.snippet.trim()
            ));
        }
    }

    if let Some(repair_notes) = repair_notes
        && !repair_notes.missing_requirements.is_empty()
    {
        prompt.push_str("Verification repair notes:\n");
        for note in &repair_notes.missing_requirements {
            prompt.push_str(&format!("- {}\n", note));
        }
    }

    if attempt > 0 {
        prompt.push_str(&format!(
            "Retry note:\n- This is automatic planning repair attempt {} after a verification miss.\n",
            attempt
        ));
    }

    prompt
}

#[cfg(feature = "advanced-primitives")]
fn build_verification_targets(
    request: &AdvancedPlanRequest,
    semantic_result: Option<&semantic_client::SemanticQueryResult>,
) -> verification::PromptVerificationTargets {
    let mut required_headings = vec![
        "Intent anchor:".to_string(),
        "Reasoning approach (apply before acting):".to_string(),
        "Ordered execution phases:".to_string(),
        "Verification gate:".to_string(),
        "Completion guardrails:".to_string(),
    ];
    if semantic_result.is_some() {
        required_headings.push("Live semantic context:".to_string());
    }

    let mut required_phrases = vec![
        "keep subtasks ordered and deduplicated".to_string(),
        "do not mark implementation complete before verification evidence exists".to_string(),
    ];
    if request.requires_verification {
        required_phrases.push("build/test checks".to_string());
    }

    verification::PromptVerificationTargets {
        required_headings,
        required_phrases,
        require_ordered_headings: true,
        require_verification_gate: true,
    }
}

#[cfg(feature = "advanced-primitives")]
fn semantic_client_config_from_hints(
    hints: &HashMap<String, String>,
) -> semantic_client::SemanticClientConfig {
    let endpoint = hint_value(hints, "advanced_primitives.semantic.endpoint");
    let api_key = hint_value(hints, "advanced_primitives.semantic.api_key");
    let domain = hint_value(hints, "advanced_primitives.semantic.domain");
    let enabled = hint_bool(
        hints,
        "advanced_primitives.semantic.enabled",
        endpoint.is_some(),
    );

    semantic_client::SemanticClientConfig {
        enabled,
        endpoint,
        api_key,
        domain,
        max_results: hint_usize(hints, "advanced_primitives.semantic.max_results", 3),
        timeout_ms: hint_u64(hints, "advanced_primitives.semantic.timeout_ms", 1_500),
    }
}

#[cfg(feature = "advanced-primitives")]
fn verification_config_from_hints(
    hints: &HashMap<String, String>,
) -> verification::VerificationLoopConfig {
    verification::VerificationLoopConfig {
        enabled: hint_bool(hints, "advanced_primitives.verification.enabled", true),
        max_automatic_retries: hint_u8(hints, "advanced_primitives.verification.max_retries", 2)
            .min(2),
    }
}

#[cfg(feature = "advanced-primitives")]
fn bos1921_from_feedback(
    feedback: &gestura_core_foundation::interaction::HapticFeedback,
    semantic_hit: bool,
) -> Bos1921Waveform {
    Bos1921Waveform {
        driver: "bos1921".to_string(),
        preset: if semantic_hit {
            "semantic-success".to_string()
        } else {
            "verification-success".to_string()
        },
        amplitude_percent: (feedback.intensity * 100.0).round().clamp(0.0, 100.0) as u8,
        duration_ms: feedback.duration_ms,
        repeat_count: feedback.repeat_count,
        repeat_delay_ms: feedback.repeat_delay_ms,
    }
}

#[cfg(feature = "advanced-primitives")]
fn hint_value(hints: &HashMap<String, String>, key: &str) -> Option<String> {
    hints
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(feature = "advanced-primitives")]
fn hint_bool(hints: &HashMap<String, String>, key: &str, default: bool) -> bool {
    match hints
        .get(key)
        .map(|value| value.trim().to_ascii_lowercase())
    {
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on") => true,
        Some(value) if matches!(value.as_str(), "0" | "false" | "no" | "off") => false,
        _ => default,
    }
}

#[cfg(feature = "advanced-primitives")]
fn hint_u8(hints: &HashMap<String, String>, key: &str, default: u8) -> u8 {
    hints
        .get(key)
        .and_then(|value| value.trim().parse::<u8>().ok())
        .unwrap_or(default)
}

#[cfg(feature = "advanced-primitives")]
fn hint_u64(hints: &HashMap<String, String>, key: &str, default: u64) -> u64 {
    hints
        .get(key)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

#[cfg(feature = "advanced-primitives")]
fn hint_usize(hints: &HashMap<String, String>, key: &str, default: usize) -> usize {
    hints
        .get(key)
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

#[cfg(all(test, feature = "advanced-primitives"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn advanced_primitives_emit_waveform_and_verification_hints() {
        let mut hints = HashMap::new();
        hints.insert(
            "advanced_primitives.semantic.enabled".to_string(),
            "false".to_string(),
        );

        let outcome = AdvancedPrimitives::run_enhanced_plan(AdvancedPlanRequest {
            user_intent: "Plan and implement the change, then verify it".to_string(),
            base_system_prompt: "System: base".to_string(),
            session_id: Some("session-1".to_string()),
            task_id: Some("task-1".to_string()),
            source: "GuiText".to_string(),
            complex_intent: true,
            requires_verification: true,
            metadata_hints: hints,
        })
        .await;

        assert!(outcome.applied);
        assert!(outcome.system_prompt.contains("Verification gate:"));
        assert!(
            outcome
                .metadata_hints
                .contains_key("advanced_primitives.verification")
        );
        assert_eq!(
            outcome
                .bos1921_waveform
                .as_ref()
                .map(|waveform| waveform.driver.as_str()),
            Some("bos1921")
        );
    }
}
