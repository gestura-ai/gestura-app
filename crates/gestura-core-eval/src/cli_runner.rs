//! Subprocess-based CLI runner.
//!
//! Drives an agent binary as a black-box subprocess — one call per variation — then
//! feeds the captured stdout to [`RuleEvaluator`].
//!
//! Everything the runner needs is read from [`EvalConfig`]: which binary to invoke,
//! what `args_prefix` to prepend, what environment variables to forward, what
//! pass/fail thresholds to apply, and any per-scenario rubric overrides.

use std::{path::PathBuf, process::Command, time::Instant};

use tracing::{debug, info, warn};

use crate::{
    config::EvalConfig,
    evaluator::RuleEvaluator,
    report::{EvalReport, ScenarioResult, VariationResult},
    scenario::{EvalScenario, EvalScenarioSuite, EvalVariation},
};

/// Runtime options for one eval run — agent profile + ephemeral CLI flags.
#[derive(Debug, Clone)]
pub struct CliRunnerOptions {
    /// Loaded agent profile (model, permissions, subprocess settings, thresholds).
    pub eval_config: EvalConfig,
    /// IDs of specific scenarios to run (empty = all).
    pub scenario_ids: Vec<String>,
    /// When true, no subprocess is launched — rule checks run on a stub response.
    pub dry_run: bool,
    /// CLI-level binary override (takes precedence over `eval_config.subprocess.bin`).
    pub bin_override: Option<PathBuf>,
}

impl CliRunnerOptions {
    /// Build options from the baseline Gestura profile with auto-detected binary.
    pub fn new() -> Self {
        Self {
            eval_config: EvalConfig::baseline(),
            scenario_ids: Vec::new(),
            dry_run: false,
            bin_override: None,
        }
    }

    /// Build options from a named built-in agent profile.
    pub fn for_agent(agent_id: &str) -> Result<Self, crate::config::ConfigError> {
        Ok(Self { eval_config: EvalConfig::load_builtin(agent_id)?, ..Self::new() })
    }
}

impl Default for CliRunnerOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Runner that drives an agent CLI as a subprocess.
pub struct CliEvalRunner {
    options: CliRunnerOptions,
}

impl CliEvalRunner {
    pub fn new(options: CliRunnerOptions) -> Self {
        Self { options }
    }

    /// The resolved binary path for this run.
    fn bin(&self) -> PathBuf {
        self.options
            .eval_config
            .resolve_bin(self.options.bin_override.as_ref())
    }

    /// Run the full (or filtered) suite and return a finalised [`EvalReport`].
    pub fn run_suite(&self, suite: &EvalScenarioSuite) -> EvalReport {
        let cfg = &self.options.eval_config;
        let bin = self.bin();

        let mut report = EvalReport::new(
            &cfg.agent.id,
            &cfg.agent.name,
            format!("{:?}", cfg.agent.mode).to_lowercase(),
            &cfg.model.provider,
            &cfg.model.name,
            self.options.dry_run,
        );

        let scenarios = suite.filter_by_ids(&self.options.scenario_ids);
        info!(
            total = scenarios.len(),
            dry_run = self.options.dry_run,
            agent_id = %cfg.agent.id,
            bin = %bin.display(),
            "gestura-eval run starting"
        );

        for scenario in scenarios {
            report.scenarios.push(self.run_scenario(scenario));
        }

        report.finalize();
        info!(
            score = report.summary.overall_score,
            passed = report.summary.passed_variations,
            total = report.summary.total_variations,
            "gestura-eval run complete"
        );
        report
    }

    fn run_scenario(&self, scenario: &EvalScenario) -> ScenarioResult {
        info!(id = %scenario.id, "scenario");
        let mut var_results = Vec::with_capacity(scenario.variations.len());
        for variation in &scenario.variations {
            var_results.push(self.run_variation(scenario, variation));
        }
        let total = var_results.len() as f32;
        let passed_count = var_results.iter().filter(|v| v.passed).count() as f32;
        let score = if total > 0.0 { passed_count / total } else { 1.0 };
        ScenarioResult {
            scenario_id: scenario.id.clone(),
            scenario_name: scenario.name.clone(),
            category: scenario.category.clone(),
            passed: var_results.iter().all(|v| v.passed),
            variations: var_results,
            score,
        }
    }

    fn run_variation(
        &self,
        scenario: &EvalScenario,
        variation: &EvalVariation,
    ) -> VariationResult {
        debug!(scenario = %scenario.id, variation = %variation.id, "variation");
        let prompt_preview = truncate(&variation.prompt, 80);
        let start = Instant::now();

        let (response_text, pipeline_error) = if self.options.dry_run {
            (
                format!(
                    "[DRY-RUN stub — {} / {}] Placeholder response for check-logic validation.",
                    scenario.id, variation.id
                ),
                None,
            )
        } else {
            self.invoke_agent(scenario, variation)
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let eval = RuleEvaluator::evaluate(variation, &response_text);

        VariationResult {
            variation_id: variation.id.clone(),
            prompt_preview,
            response: response_text,
            duration_ms,
            pipeline_error,
            checks: eval.checks,
            score: eval.score,
            passed: eval.passed,
        }
    }

    /// Build the prompt and invoke the agent binary as a subprocess.
    ///
    /// Command structure: `<bin> [args_prefix...] <prompt>`
    ///
    /// Environment variables from `subprocess.env` are forwarded. If the scenario
    /// rubric disables tools, `GESTURA_TOOLS_ENABLED=false` is also set (Gestura-
    /// specific; other agents ignore it safely).
    fn invoke_agent(
        &self,
        scenario: &EvalScenario,
        variation: &EvalVariation,
    ) -> (String, Option<String>) {
        let cfg = &self.options.eval_config;
        let prompt = build_prompt(variation);
        let bin = self.bin();

        let mut cmd = Command::new(&bin);

        // Prepend agent-specific args (e.g. `["--dangerously-skip-permissions", "-p"]`)
        // then append the prompt as the final argument.
        cmd.args(&cfg.subprocess.args_prefix);
        cmd.arg(&prompt);

        // Forward config-defined env vars.
        for (k, v) in &cfg.subprocess.env {
            cmd.env(k, v);
        }

        // Gestura-specific: disable tool execution when rubric says no tools.
        if !scenario.rubric.tools_enabled {
            cmd.env("GESTURA_TOOLS_ENABLED", "false");
        }

        match cmd.output() {
            Ok(out) => {
                let raw_stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();

                // Strip configured response prefix (e.g. "Assistant: " labels).
                let stdout = if let Some(ref prefix) = cfg.subprocess.response_strip_prefix {
                    raw_stdout
                        .strip_prefix(prefix.as_str())
                        .unwrap_or(&raw_stdout)
                        .trim()
                        .to_string()
                } else {
                    raw_stdout
                };

                if !out.status.success() {
                    let err = if !stderr.is_empty() {
                        stderr
                    } else {
                        format!("exit {}", out.status)
                    };
                    warn!(
                        scenario = %scenario.id,
                        variation = %variation.id,
                        error = %err,
                        "agent subprocess failed"
                    );
                    (stdout, Some(err))
                } else {
                    debug!(
                        scenario = %scenario.id,
                        variation = %variation.id,
                        words = stdout.split_whitespace().count(),
                        "response captured"
                    );
                    (stdout, None)
                }
            }
            Err(e) => {
                warn!(bin = %bin.display(), error = %e, "failed to launch agent subprocess");
                (String::new(), Some(e.to_string()))
            }
        }
    }
}

/// Prepend conversation history (if any) to the final prompt so the LLM has
/// the full conversational context even via a single CLI invocation.
fn build_prompt(variation: &EvalVariation) -> String {
    if variation.history.is_empty() {
        return variation.prompt.clone();
    }

    let mut buf = String::new();
    buf.push_str("Continue this conversation and respond to the final user message.\n\n");
    for msg in &variation.history {
        let role = match msg.role.as_str() {
            "user"      => "User",
            "assistant" => "Assistant",
            other       => other,
        };
        buf.push_str(&format!("{role}: {}\n\n", msg.content));
    }
    buf.push_str(&format!("User: {}", variation.prompt));
    buf
}

fn truncate(s: &str, max_chars: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max_chars {
        s
    } else {
        let mut t: String = s.chars().take(max_chars - 1).collect();
        t.push('…');
        t
    }
}

