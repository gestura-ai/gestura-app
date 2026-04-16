//! Multi-agent suite orchestrator.
//!
//! [`MultiRunOrchestrator`] drives sequential profile runs, collects [`EvalReport`]s,
//! and optionally persists each report to disk as it finishes.  Progress events flow
//! through the shared [`ProgressCallback`] so the caller (terminal renderer, CI logger)
//! sees live output without polling.
//!
//! Execution model: profiles run **sequentially** to avoid interleaved terminal output
//! and stay within per-provider rate limits.  Variations within a profile run
//! sequentially inside [`CliEvalRunner`].  A `--parallel-profiles N` flag can be
//! layered on top later without touching this interface.

use std::{
    path::PathBuf,
    time::Instant,
};

use tracing::warn;

use crate::{
    CliEvalRunner, CliRunnerOptions,
    config::{EvalConfig, ConfigError, BUILTIN_AGENT_IDS},
    progress::{ProgressCallback, ProgressEvent},
    report::EvalReport,
    scenario::EvalScenarioSuite,
};

// ─── Profile selector ─────────────────────────────────────────────────────────

/// Describes which agent profiles a suite run should include.
///
/// Resolution priority (highest first):
/// 1. Explicit `agent_ids` list — use exactly these profile IDs.
/// 2. `families` list — include all profiles whose ID starts with a family name.
/// 3. Empty selector — include every built-in profile.
#[derive(Debug, Clone, Default)]
pub struct ProfileSelector {
    /// Explicit agent profile IDs, e.g. `["gestura-full", "codex-sandboxed"]`.
    pub agent_ids: Vec<String>,
    /// Agent family names, e.g. `["gestura", "claude-code"]`.
    pub families: Vec<String>,
}

impl ProfileSelector {
    /// Resolve the selector to a list of loaded [`EvalConfig`]s.
    pub fn resolve(&self) -> Result<Vec<EvalConfig>, ConfigError> {
        let ids: Vec<String> = if !self.agent_ids.is_empty() {
            self.agent_ids.clone()
        } else if !self.families.is_empty() {
            BUILTIN_AGENT_IDS
                .iter()
                .filter(|id| self.families.iter().any(|f| id.starts_with(f.as_str())))
                .map(|s| s.to_string())
                .collect()
        } else {
            // Default: all 15 built-in profiles.
            BUILTIN_AGENT_IDS.iter().map(|s| s.to_string()).collect()
        };

        ids.iter().map(|id| EvalConfig::load_builtin(id)).collect()
    }
}

// ─── Suite run plan ───────────────────────────────────────────────────────────

/// Everything the orchestrator needs to execute one comparison run.
pub struct SuiteRunPlan {
    /// Resolved agent profiles to run in order.
    pub profiles: Vec<EvalConfig>,
    /// The scenario suite (shared across all profiles).
    pub suite: EvalScenarioSuite,
    /// If set, each per-profile JSON report is saved here as it finishes.
    pub output_dir: Option<PathBuf>,
    /// When true, no subprocesses are launched — check logic only.
    pub dry_run: bool,
    /// CLI-level binary override forwarded to every runner.
    pub bin_override: Option<PathBuf>,
    /// Restrict to these scenario IDs (empty = all).
    pub scenario_ids: Vec<String>,
}

// ─── Orchestrator ─────────────────────────────────────────────────────────────

/// Runs a collection of agent profiles sequentially against a shared scenario suite.
pub struct MultiRunOrchestrator {
    plan: SuiteRunPlan,
    progress: Option<ProgressCallback>,
}

impl MultiRunOrchestrator {
    pub fn new(plan: SuiteRunPlan) -> Self {
        Self { plan, progress: None }
    }

    /// Attach a progress callback.  All runner events for every profile funnel
    /// through this single callback.
    pub fn with_progress(mut self, cb: ProgressCallback) -> Self {
        self.progress = Some(cb);
        self
    }

    /// Execute every profile in order and return the collected reports.
    ///
    /// Profiles that require manual authentication but lack the token are skipped
    /// (a [`ProgressEvent::ProfileSkipped`] fires instead of crashing the run).
    ///
    /// If `plan.output_dir` is set, each completed report is persisted to
    /// `<output_dir>/<agent-id>-<run-id>.json` immediately after the profile
    /// finishes — so a partial suite is never lost on interruption.
    pub fn run(self) -> Vec<EvalReport> {
        let wall = Instant::now();
        let mut reports = Vec::new();

        for config in &self.plan.profiles {
            // Guard: skip profiles that need a session token if it isn't present.
            if config.agent.requires_manual_auth
                && !self.plan.dry_run
                && let Some(ref env_var) = config.agent.auth_env_var
            {
                let present = std::env::var(env_var)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                if !present {
                    let reason = format!("requires manual auth ({env_var} not set)");
                    warn!(agent_id = %config.agent.id, %reason, "skipping profile");
                    if let Some(ref cb) = self.progress {
                        cb(ProgressEvent::ProfileSkipped {
                            agent_id: config.agent.id.clone(),
                            reason,
                        });
                    }
                    continue;
                }
            }

            let opts = CliRunnerOptions {
                eval_config: config.clone(),
                scenario_ids: self.plan.scenario_ids.clone(),
                dry_run: self.plan.dry_run,
                bin_override: self.plan.bin_override.clone(),
                progress: self.progress.clone(),
                ..CliRunnerOptions::new()
            };

            let runner = CliEvalRunner::new(opts);
            let report = runner.run_suite(&self.plan.suite);

            // Persist immediately so partial suite results survive interruption.
            if let Some(ref dir) = self.plan.output_dir {
                let filename = format!("{}-{}.json", config.agent.id, report.run_id);
                let path = dir.join(&filename);
                match serde_json::to_string_pretty(&report) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(&path, json) {
                            warn!(path = %path.display(), error = %e, "failed to save per-agent report");
                        }
                    }
                    Err(e) => warn!(error = %e, "failed to serialise per-agent report"),
                }
            }

            reports.push(report);
        }

        let elapsed = wall.elapsed().as_secs_f64();
        if let Some(ref cb) = self.progress {
            cb(ProgressEvent::SuiteFinished { elapsed_secs: elapsed });
        }

        reports
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Extract the family prefix from an agent ID.
///
/// `"gestura-full"` → `"gestura"`,  `"claude-code-sandboxed"` → `"claude-code"`
pub fn agent_family(id: &str) -> &str {
    const MODES: &[&str] = &["-full", "-sandboxed", "-iterative"];
    for mode in MODES {
        if let Some(family) = id.strip_suffix(mode) {
            return family;
        }
    }
    id
}
