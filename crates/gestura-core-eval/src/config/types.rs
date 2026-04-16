//! Agent profile type definitions.
//!
//! Every field that can differ between agent profiles lives here.
//! The canonical defaults are in `agents/baseline.toml` — not in Rust code —
//! so the baseline is human-readable and version-controlled alongside the profiles.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// ─── Agent identity ───────────────────────────────────────────────────────────

/// Identity and execution contract for one agent profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMeta {
    /// Stable machine ID (e.g. `"gestura-full"`).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// One-line description shown in `--list-agents`.
    pub description: String,
    /// Fundamental execution contract — drives subprocess strategy and expected behaviours.
    pub mode: AgentMode,
    /// When `true` the agent cannot authenticate via a static API key; it requires
    /// an OAuth session token that must be obtained interactively (e.g. `auggie login`).
    ///
    /// Profiles with this flag set will **not** run in automated (non-dry-run) contexts
    /// unless the authentication environment variable they need is already present.
    /// The evaluator fails fast with a clear error rather than timing out mid-run.
    ///
    /// Corresponds to `requires_manual_auth = true` in the profile TOML.
    #[serde(default)]
    pub requires_manual_auth: bool,
    /// Environment variable that must be set for this profile to run without `--dry-run`.
    ///
    /// Used together with `requires_manual_auth`. When set, the evaluator checks for
    /// this variable at startup and exits early if it is absent or empty.
    ///
    /// Example: `"AUGMENT_SESSION_AUTH"`.
    #[serde(default)]
    pub auth_env_var: Option<String>,
}

impl Default for AgentMeta {
    fn default() -> Self {
        Self {
            id: "baseline".into(),
            name: "Baseline".into(),
            description: "Default evaluation profile".into(),
            mode: AgentMode::Autonomous,
            requires_manual_auth: false,
            auth_env_var: None,
        }
    }
}

/// How the agent executes and what the eval runner should expect from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// Single-shot, no confirmation required. Full e2e task completion.
    Autonomous,
    /// No tool calls, no network, no writes. Safe for untrusted or sensitive inputs.
    Sandboxed,
    /// Pauses before side-effectful actions and requests explicit human approval.
    Iterative,
}

// ─── Model ────────────────────────────────────────────────────────────────────

/// LLM model configuration for this agent profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Provider name: `anthropic` | `openai` | `grok` | `gemini` | `ollama`.
    pub provider: String,
    /// Model name, e.g. `claude-sonnet-4-5`, `gpt-4o`, `gemini-2.0-flash`.
    pub name: String,
    /// Sampling temperature (0.0 – 2.0).
    pub temperature: f32,
    /// Hard token limit on generated responses.
    pub max_tokens: u32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".into(),
            name: "claude-sonnet-4-5".into(),
            temperature: 0.7,
            max_tokens: 8192,
        }
    }
}

// ─── Permissions ──────────────────────────────────────────────────────────────

/// Tool and system access permissions for this agent profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionConfig {
    /// Broad permission tier applied by the pipeline.
    pub level: PermissionLevel,
    /// Master switch: allow any tool invocations.
    pub tools_enabled: bool,
    /// Allow `shell` tool (arbitrary command execution).
    pub shell_enabled: bool,
    /// Allow outbound network access from tools.
    pub network_enabled: bool,
    /// Allow filesystem writes.
    pub write_enabled: bool,
    /// Explicit tool allowlist — if non-empty, only these tool names may run.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Explicit tool denylist — always blocked regardless of level.
    #[serde(default)]
    pub denied_tools: Vec<String>,
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            level: PermissionLevel::Restricted,
            tools_enabled: false,
            shell_enabled: false,
            network_enabled: false,
            write_enabled: false,
            allowed_tools: vec![],
            denied_tools: vec![],
        }
    }
}

/// Broad permission tier, maps directly to Gestura's pipeline permission model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    /// All operations permitted.
    Full,
    /// Local writes allowed; no shell or arbitrary network.
    Restricted,
    /// Read-only, no network, no writes, no shell.
    Sandbox,
    /// Granted per-tool via explicit allowlist.
    PerTool,
}

// ─── Execution ────────────────────────────────────────────────────────────────

/// Agentic loop and approval-gate settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Maximum agentic loop iterations before forcing a response.
    pub max_iterations: usize,
    /// Wall-clock timeout per variation, in seconds.
    pub timeout_secs: u64,
    /// Iterative mode: the agent is expected to pause before dangerous actions.
    pub require_confirmation: bool,
    /// String sent as the approval response in iterative mode (e.g. `"yes"`).
    pub confirmation_response: String,
    /// How many times to retry a failed subprocess call before recording it as
    /// failed.  Retries are only attempted on rate-limit (429) errors; hard
    /// failures (binary not found, non-429 exit codes) fail immediately.
    pub retries: u32,
    /// Minimum pause inserted **after** each variation call completes, in
    /// milliseconds.  A value of 0 (the default) means no deliberate throttle.
    ///
    /// Use this as a conservative backstop when a provider's per-minute token
    /// budget is tight.  The `--variation-delay` CLI flag overrides this at
    /// runtime without changing the profile TOML.
    #[serde(default)]
    pub delay_between_variations_ms: u64,
    /// Initial wait before the first rate-limit retry, in seconds.  Doubles on
    /// each subsequent retry (exponential backoff).  Default: 15 s.
    ///
    /// With `retries = 1` and the default backoff: one 15 s pause, then fail.
    /// With `retries = 3`: 15 s → 30 s → 60 s, then fail.
    #[serde(default = "default_rate_limit_backoff_secs")]
    pub rate_limit_backoff_secs: u64,
    /// How many times to run each variation.  Scores are averaged; pass/fail
    /// uses majority vote (> half of trials must pass).
    ///
    /// `1` (the default) is identical to previous single-trial behaviour.
    /// Use `3`–`5` for statistically reliable benchmarks.  Note that each
    /// additional trial multiplies API cost and runtime proportionally.
    #[serde(default = "default_trials")]
    pub trials: u32,
}

fn default_rate_limit_backoff_secs() -> u64 { 15 }
fn default_trials() -> u32 { 1 }

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_iterations: 1,
            timeout_secs: 60,
            require_confirmation: false,
            confirmation_response: "yes".into(),
            retries: 0,
            delay_between_variations_ms: 0,
            rate_limit_backoff_secs: 15,
            trials: 1,
        }
    }
}

// ─── Subprocess ───────────────────────────────────────────────────────────────

/// How to invoke the agent binary for each variation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubprocessDef {
    /// Explicit path to the binary. `None` = auto-detect sibling or PATH.
    pub bin: Option<String>,
    /// Arguments inserted before the prompt (e.g. `["exec"]` or `["-p"]`).
    #[serde(default)]
    pub args_prefix: Vec<String>,
    /// Environment variables forwarded to every subprocess invocation.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Strip this literal prefix from stdout before evaluation.
    /// Some CLIs prepend role labels (e.g. `"Assistant: "`) or spinner lines
    /// to their output. Setting this ensures the evaluator sees clean text.
    #[serde(default)]
    pub response_strip_prefix: Option<String>,
}

impl Default for SubprocessDef {
    fn default() -> Self {
        Self {
            bin: None,
            args_prefix: vec!["exec".into()],
            env: HashMap::from([("GESTURA_DISABLE_KEYCHAIN".into(), "1".into())]),
            response_strip_prefix: None,
        }
    }
}

// ─── Thresholds ───────────────────────────────────────────────────────────────

/// Pass/fail thresholds for this agent profile.
///
/// Different modes have different expectations: a sandboxed agent will naturally
/// produce shorter, more restricted answers, so its thresholds are relaxed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thresholds {
    /// Minimum rule-check score for one variation to be considered passing (0.0 – 1.0).
    pub min_variation_score: f32,
    /// Minimum fraction of variations that must pass for a scenario to pass (0.0 – 1.0).
    pub min_scenario_pass_rate: f32,
    /// Minimum mean score across all variations for the run to exit 0 (0.0 – 1.0).
    pub min_overall_score: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self { min_variation_score: 0.8, min_scenario_pass_rate: 1.0, min_overall_score: 0.8 }
    }
}

// ─── Scenario / variation overrides ──────────────────────────────────────────

/// Agent-specific adjustments to one scenario's rubric.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScenarioOverride {
    /// Skip this scenario entirely for this agent profile.
    #[serde(default)]
    pub disabled: bool,
    /// Keyed by variation ID (e.g. `"v1"`).
    #[serde(default)]
    pub variation_overrides: HashMap<String, VariationOverride>,
}

/// Rubric adjustments for a single variation within a scenario.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VariationOverride {
    /// Override the minimum word count expected in the response.
    #[serde(default)]
    pub min_words: Option<usize>,
    /// Override the maximum word count expected in the response.
    #[serde(default)]
    pub max_words: Option<usize>,
    /// Extra check names appended to the variation's check list.
    #[serde(default)]
    pub additional_checks: Vec<String>,
    /// Check names removed from the variation's check list.
    #[serde(default)]
    pub disabled_checks: Vec<String>,
}

