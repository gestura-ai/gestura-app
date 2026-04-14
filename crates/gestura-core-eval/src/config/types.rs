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
}

impl Default for AgentMeta {
    fn default() -> Self {
        Self {
            id: "baseline".into(),
            name: "Baseline".into(),
            description: "Default evaluation profile".into(),
            mode: AgentMode::Autonomous,
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
    /// Retry a failed subprocess call this many times before recording as failed.
    pub retries: u32,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_iterations: 1,
            timeout_secs: 60,
            require_confirmation: false,
            confirmation_response: "yes".into(),
            retries: 0,
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

