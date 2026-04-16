//! Evaluation report types and formatting.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::evaluator::CheckResult;
use crate::judge::JudgeScore;

/// Full evaluation report for a completed run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalReport {
    /// Unique ID for this run.
    pub run_id: String,
    /// ISO-8601 timestamp of when the run started.
    pub timestamp: DateTime<Utc>,
    /// Agent profile ID (e.g. `"gestura-full"`, `"claude-code-sandboxed"`).
    pub agent_id: String,
    /// Human-readable agent profile name.
    pub agent_name: String,
    /// Agentic mode for this run (e.g. `"autonomous"`, `"sandboxed"`, `"iterative"`).
    pub agent_mode: String,
    /// LLM provider name from config (e.g. `"anthropic"`).
    pub provider: String,
    /// Model name from config (e.g. `"claude-sonnet-4-6"`).
    pub model: String,
    /// Whether this was a dry-run (no actual LLM calls).
    pub dry_run: bool,
    /// Per-scenario results.
    pub scenarios: Vec<ScenarioResult>,
    /// Aggregate summary across all scenarios.
    pub summary: EvalSummary,
}

/// Result for a single scenario (all its variations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub scenario_id: String,
    pub scenario_name: String,
    pub category: String,
    pub variations: Vec<VariationResult>,
    /// Fraction of variations that passed (0.0 – 1.0).
    pub score: f32,
    pub passed: bool,
}

/// Result for a single prompt variation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariationResult {
    pub variation_id: String,
    pub prompt_preview: String,
    /// The full LLM response from the representative trial (empty in dry-run).
    pub response: String,
    /// Total wall-clock duration across all trials, in milliseconds.
    pub duration_ms: u64,
    /// Pipeline error from the representative trial, if any.
    pub pipeline_error: Option<String>,
    /// Rule-based check results from the representative trial.
    pub checks: Vec<CheckResult>,
    /// Average score across all trials (identical to `score` when `trials = 1`).
    pub score: f32,
    /// Majority-vote pass: true when more than half of trials passed.
    pub passed: bool,
    /// Per-trial scores in run order.  Length 1 for single-trial runs.
    /// Populated even when `trials = 1` so downstream consumers don't need
    /// to special-case single vs. multi-trial reports.
    #[serde(default)]
    pub trial_scores: Vec<f32>,
    /// Per-trial full response text in run order.
    #[serde(default)]
    pub trial_responses: Vec<String>,
    /// Optional LLM-as-judge quality score.  `None` when the judge is not
    /// configured (no `ANTHROPIC_API_KEY`) or when the response was empty.
    /// Does **not** affect the rule-based `score` or `passed` fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_score: Option<JudgeScore>,
}

/// Aggregate summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSummary {
    pub total_scenarios: usize,
    pub passed_scenarios: usize,
    pub failed_scenarios: usize,
    pub total_variations: usize,
    pub passed_variations: usize,
    pub failed_variations: usize,
    /// Mean score across all variations (0.0 – 1.0).
    pub overall_score: f32,
}

impl EvalReport {
    /// Build a new empty report shell (scenarios are added by the runner).
    pub fn new(
        agent_id: impl Into<String>,
        agent_name: impl Into<String>,
        agent_mode: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        dry_run: bool,
    ) -> Self {
        Self {
            run_id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            agent_id: agent_id.into(),
            agent_name: agent_name.into(),
            agent_mode: agent_mode.into(),
            provider: provider.into(),
            model: model.into(),
            dry_run,
            scenarios: Vec::new(),
            summary: EvalSummary {
                total_scenarios: 0,
                passed_scenarios: 0,
                failed_scenarios: 0,
                total_variations: 0,
                passed_variations: 0,
                failed_variations: 0,
                overall_score: 0.0,
            },
        }
    }

    /// Recompute the summary from `self.scenarios`. Call after all scenarios are added.
    pub fn finalize(&mut self) {
        let total_s = self.scenarios.len();
        let passed_s = self.scenarios.iter().filter(|s| s.passed).count();
        let all_vars: Vec<&VariationResult> =
            self.scenarios.iter().flat_map(|s| s.variations.iter()).collect();
        let total_v = all_vars.len();
        let passed_v = all_vars.iter().filter(|v| v.passed).count();
        let score_sum: f32 = all_vars.iter().map(|v| v.score).sum();
        let overall = if total_v > 0 { score_sum / total_v as f32 } else { 1.0 };

        self.summary = EvalSummary {
            total_scenarios: total_s,
            passed_scenarios: passed_s,
            failed_scenarios: total_s - passed_s,
            total_variations: total_v,
            passed_variations: passed_v,
            failed_variations: total_v - passed_v,
            overall_score: (overall * 1000.0).round() / 1000.0,
        };
    }

    /// Print a human-readable summary to stdout.
    ///
    /// Three display modes, controlled by the two flags:
    ///
    /// | `verbose` | `show_breaking` | Behaviour |
    /// |-----------|-----------------|-----------|
    /// | false     | false           | Pass/fail + failing check details only — no response text |
    /// | false     | true            | Same, plus the agent response (≤ 400 chars) for failed variations |
    /// | true      | any             | All check results (✓ and ✗) + full response (≤ 800 chars) for every variation |
    pub fn print_text(&self, verbose: bool, show_breaking: bool) {
        let s = &self.summary;
        println!();
        println!("╔══════════════════════════════════════════════════════════╗");
        println!("║          GESTURA EVAL REPORT                             ║");
        println!("╚══════════════════════════════════════════════════════════╝");
        println!("  Run ID  : {}", self.run_id);
        println!("  Time    : {}", self.timestamp.format("%Y-%m-%d %H:%M:%S UTC"));
        println!("  Agent   : {} [{}]", self.agent_name, self.agent_id);
        println!("  Mode    : {}", self.agent_mode);
        println!("  Provider: {} / {}", self.provider, self.model);
        if self.dry_run {
            println!("  ⚠ DRY-RUN (no actual subprocess calls)");
        }
        println!();
        println!(
            "  Scenarios : {}/{} passed",
            s.passed_scenarios, s.total_scenarios
        );
        println!(
            "  Variations: {}/{} passed",
            s.passed_variations, s.total_variations
        );
        println!("  Score     : {:.1}%", s.overall_score * 100.0);
        println!();

        for scenario in &self.scenarios {
            let icon = if scenario.passed { "✅" } else { "❌" };
            println!(
                "  {} [{}] {} ({:.0}%)",
                icon,
                scenario.scenario_id,
                scenario.scenario_name,
                scenario.score * 100.0
            );

            for v in &scenario.variations {
                let vicon = if v.passed { "  ✓" } else { "  ✗" };
                let trial_note = if v.trial_scores.len() > 1 {
                    let min = v.trial_scores.iter().cloned().fold(f32::INFINITY, f32::min);
                    let max = v.trial_scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                    let passed_n = v.trial_scores.iter().filter(|&&s| s >= 0.5).count();
                    format!(
                        " ({}/{} trials, range {:.0}–{:.0}%)",
                        passed_n, v.trial_scores.len(), min * 100.0, max * 100.0
                    )
                } else {
                    String::new()
                };
                println!("      {} {} — {:.0}%{}", vicon, v.variation_id, v.score * 100.0, trial_note);
                println!("        Prompt : {}", v.prompt_preview);

                // ── Pipeline / subprocess error ─────────────────────────────
                if let Some(ref err) = v.pipeline_error {
                    println!("        ⚠ Pipeline error: {err}");
                }

                // ── Check results ───────────────────────────────────────────
                // Always show genuinely failing checks.
                // Show passing checks and skipped checks only in verbose mode.
                // Skipped checks (empty response) use ⊘ to distinguish them from
                // real failures — the response_not_empty failure already explains why.
                for check in &v.checks {
                    let show = if check.skipped {
                        verbose // only clutter the output in verbose mode
                    } else {
                        verbose || !check.passed
                    };
                    if show {
                        let cicon = if check.skipped {
                            "⊘"
                        } else if check.passed {
                            "✓"
                        } else {
                            "✗"
                        };
                        println!(
                            "        {} {:<35} {}",
                            cicon,
                            check.name,
                            check.details
                        );
                    }
                }

                // ── Agent response ──────────────────────────────────────────
                // verbose        → show for every variation (800 char limit)
                // show_breaking  → show only for failed variations (400 char limit)
                // default        → never shown
                let show_response = verbose || (show_breaking && !v.passed);
                if show_response {
                    let response_limit = if verbose { 800 } else { 400 };
                    let display_response = if v.response.trim().is_empty() {
                        "<empty response>".to_string()
                    } else {
                        truncate_response(&v.response, response_limit)
                    };
                    println!(
                        "        ┌─ Agent response ({} words) ─────────────────────",
                        v.response.split_whitespace().count()
                    );
                    for line in display_response.lines() {
                        println!("        │ {line}");
                    }
                    println!("        └─────────────────────────────────────────────────");
                }
            }
            println!();
        }
    }

    /// Serialize the report to pretty-printed JSON on stdout.
    ///
    /// The JSON payload includes every `VariationResult.response` field in full,
    /// so machine consumers always have complete diagnostic context without
    /// any truncation.
    pub fn print_json(&self) {
        println!(
            "{}",
            serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
        );
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Truncate `text` to at most `max_chars` characters, appending `…` if cut.
/// Preserves line breaks so multi-line responses stay readable.
fn truncate_response(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_chars - 1).collect();
        format!("{truncated}…\n<response truncated at {max_chars} chars; use --verbose or --json for full output>")
    }
}

