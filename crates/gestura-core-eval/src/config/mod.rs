//! Agent profile configuration — loading, merging, and the [`EvalConfig`] facade.
//!
//! # Profile resolution order
//!
//! 1. **`agents/baseline.toml`** — always loaded first; defines universal defaults.
//! 2. **Named agent overlay** — built-in or on-disk TOML merged on top via deep merge.
//!    Any field absent in the overlay inherits the baseline value.
//!
//! # Built-in profiles
//!
//! | Agent ID                  | Mode        | Description                                           |
//! |---------------------------|-------------|-------------------------------------------------------|
//! | `gestura-full`            | autonomous  | Gestura CLI · full tools + shell, no confirmation     |
//! | `gestura-sandboxed`       | sandboxed   | Gestura CLI · no tools, no network, no writes         |
//! | `gestura-iterative`       | iterative   | Gestura CLI · confirmation gate on side-effectful ops |
//! | `claude-code-full`        | autonomous  | Claude Code CLI · `--dangerously-skip-permissions`    |
//! | `claude-code-sandboxed`   | sandboxed   | Claude Code CLI · `--allowedTools ""` (no tools)      |
//! | `claude-code-iterative`   | iterative   | Claude Code CLI · default confirmation gates          |
//! | `augment-full`            | autonomous  | Augment Code agent · unrestricted e2e execution       |
//! | `augment-sandboxed`       | sandboxed   | Augment Code agent · read-only, no tool execution     |
//! | `augment-iterative`       | iterative   | Augment Code agent · confirmation-gated workflow      |
//! | `codex-full`              | autonomous  | OpenAI Codex CLI · `--approval-mode full-auto`        |
//! | `codex-sandboxed`         | sandboxed   | OpenAI Codex CLI · `--sandbox` (no tool execution)   |
//! | `codex-iterative`         | iterative   | OpenAI Codex CLI · `--approval-mode suggest`          |
//! | `opencode-full`           | autonomous  | OpenCode · unrestricted, `--yes` auto-approval        |
//! | `opencode-sandboxed`      | sandboxed   | OpenCode · `--no-tools`, read-only reasoning          |
//! | `opencode-iterative`      | iterative   | OpenCode · `--interactive` confirmation gates         |

mod types;
pub use types::*;

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt, fs, path::Path};

// ─── Embedded TOML profiles ───────────────────────────────────────────────────

const BUILTIN_BASELINE: &str              = include_str!("../../agents/baseline.toml");
const BUILTIN_GESTURA_FULL: &str          = include_str!("../../agents/gestura-full.toml");
const BUILTIN_GESTURA_SB: &str            = include_str!("../../agents/gestura-sandboxed.toml");
const BUILTIN_GESTURA_ITER: &str          = include_str!("../../agents/gestura-iterative.toml");
const BUILTIN_CLAUDE_CODE_FULL: &str      = include_str!("../../agents/claude-code-full.toml");
const BUILTIN_CLAUDE_CODE_SB: &str        = include_str!("../../agents/claude-code-sandboxed.toml");
const BUILTIN_CLAUDE_CODE_ITER: &str      = include_str!("../../agents/claude-code-iterative.toml");
const BUILTIN_AUGMENT_FULL: &str          = include_str!("../../agents/augment-full.toml");
const BUILTIN_AUGMENT_SB: &str            = include_str!("../../agents/augment-sandboxed.toml");
const BUILTIN_AUGMENT_ITER: &str          = include_str!("../../agents/augment-iterative.toml");
const BUILTIN_CODEX_FULL: &str            = include_str!("../../agents/codex-full.toml");
const BUILTIN_CODEX_SB: &str              = include_str!("../../agents/codex-sandboxed.toml");
const BUILTIN_CODEX_ITER: &str            = include_str!("../../agents/codex-iterative.toml");
const BUILTIN_OPENCODE_FULL: &str         = include_str!("../../agents/opencode-full.toml");
const BUILTIN_OPENCODE_SB: &str           = include_str!("../../agents/opencode-sandboxed.toml");
const BUILTIN_OPENCODE_ITER: &str         = include_str!("../../agents/opencode-iterative.toml");

/// All agent IDs recognised by [`EvalConfig::load_builtin`].
pub const BUILTIN_AGENT_IDS: &[&str] = &[
    "gestura-full",
    "gestura-sandboxed",
    "gestura-iterative",
    "claude-code-full",
    "claude-code-sandboxed",
    "claude-code-iterative",
    "augment-full",
    "augment-sandboxed",
    "augment-iterative",
    "codex-full",
    "codex-sandboxed",
    "codex-iterative",
    "opencode-full",
    "opencode-sandboxed",
    "opencode-iterative",
];

fn builtin_agent_toml(id: &str) -> Result<&'static str, ConfigError> {
    match id {
        "gestura-full"           => Ok(BUILTIN_GESTURA_FULL),
        "gestura-sandboxed"      => Ok(BUILTIN_GESTURA_SB),
        "gestura-iterative"      => Ok(BUILTIN_GESTURA_ITER),
        "claude-code-full"       => Ok(BUILTIN_CLAUDE_CODE_FULL),
        "claude-code-sandboxed"  => Ok(BUILTIN_CLAUDE_CODE_SB),
        "claude-code-iterative"  => Ok(BUILTIN_CLAUDE_CODE_ITER),
        "augment-full"           => Ok(BUILTIN_AUGMENT_FULL),
        "augment-sandboxed"      => Ok(BUILTIN_AUGMENT_SB),
        "augment-iterative"      => Ok(BUILTIN_AUGMENT_ITER),
        "codex-full"             => Ok(BUILTIN_CODEX_FULL),
        "codex-sandboxed"        => Ok(BUILTIN_CODEX_SB),
        "codex-iterative"        => Ok(BUILTIN_CODEX_ITER),
        "opencode-full"          => Ok(BUILTIN_OPENCODE_FULL),
        "opencode-sandboxed"     => Ok(BUILTIN_OPENCODE_SB),
        "opencode-iterative"     => Ok(BUILTIN_OPENCODE_ITER),
        other => Err(ConfigError::UnknownAgent(other.to_string())),
    }
}

// ─── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ConfigError {
    UnknownAgent(String),
    ParseError(toml::de::Error),
    IoError(std::io::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownAgent(id) => write!(
                f,
                "unknown agent profile '{}'. Built-in profiles: {}",
                id,
                BUILTIN_AGENT_IDS.join(", ")
            ),
            Self::ParseError(e) => write!(f, "TOML parse error: {e}"),
            Self::IoError(e)    => write!(f, "IO error: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}
impl From<toml::de::Error>  for ConfigError { fn from(e: toml::de::Error)  -> Self { Self::ParseError(e) } }
impl From<std::io::Error>   for ConfigError { fn from(e: std::io::Error)   -> Self { Self::IoError(e) } }

// ─── Top-level config ─────────────────────────────────────────────────────────

/// Full eval configuration for one agent profile.
///
/// Always loaded by merging [`BUILTIN_BASELINE`] with an agent-specific overlay.
/// Any field absent in the overlay inherits the baseline value.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvalConfig {
    #[serde(default)] pub agent:       AgentMeta,
    #[serde(default)] pub model:       ModelConfig,
    #[serde(default)] pub permissions: PermissionConfig,
    #[serde(default)] pub execution:   ExecutionConfig,
    #[serde(default)] pub subprocess:  SubprocessDef,
    #[serde(default)] pub thresholds:  Thresholds,
    /// Per-scenario overrides keyed by scenario ID (e.g. `"s3_planning"`).
    #[serde(default)] pub scenarios:   HashMap<String, ScenarioOverride>,
}

impl EvalConfig {
    /// Load the baseline profile only (no agent overlay).
    ///
    /// # Panics
    /// Panics if `agents/baseline.toml` is malformed — this is a build-time bug.
    pub fn baseline() -> Self {
        toml::from_str(BUILTIN_BASELINE)
            .expect("gestura-core-eval: agents/baseline.toml is malformed — build-time bug")
    }

    /// Load a built-in named agent profile (baseline merged with agent overlay).
    ///
    /// # Errors
    /// Returns [`ConfigError::UnknownAgent`] if `agent_id` is not in [`BUILTIN_AGENT_IDS`].
    pub fn load_builtin(agent_id: &str) -> Result<Self, ConfigError> {
        Self::merge_toml(BUILTIN_BASELINE, builtin_agent_toml(agent_id)?)
    }

    /// Load from a custom TOML file on disk, merged on top of baseline.
    ///
    /// Useful for team-specific or CI-specific agent profiles not shipped in the binary.
    pub fn load_from_path(path: &Path) -> Result<Self, ConfigError> {
        let overlay = fs::read_to_string(path)?;
        Self::merge_toml(BUILTIN_BASELINE, &overlay)
    }

    /// The resolved binary path for subprocess invocation.
    ///
    /// Precedence: `cli_override` → `subprocess.bin` in config → auto-detect sibling/PATH.
    pub fn resolve_bin(&self, cli_override: Option<&std::path::PathBuf>) -> std::path::PathBuf {
        if let Some(p) = cli_override {
            return p.clone();
        }
        if let Some(ref b) = self.subprocess.bin {
            return std::path::PathBuf::from(b);
        }
        // Auto-detect: sibling binary next to this process, or fall back to PATH.
        if let Ok(exe) = std::env::current_exe() {
            let sibling = exe.with_file_name("gestura");
            if sibling.exists() {
                return sibling;
            }
        }
        std::path::PathBuf::from("gestura")
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    fn merge_toml(base: &str, overlay: &str) -> Result<Self, ConfigError> {
        let mut base_val: toml::Value  = toml::from_str(base)?;
        let overlay_val: toml::Value   = toml::from_str(overlay)?;
        deep_merge(&mut base_val, overlay_val);
        Ok(toml::Value::try_into(base_val)?)
    }
}

// ─── Deep TOML merge ──────────────────────────────────────────────────────────

/// Recursively merge `overlay` into `base`.
///
/// - **Tables** are merged key-by-key (deep).
/// - **Scalars and arrays** are replaced wholesale by the overlay value.
///
/// This means array fields (e.g. `args_prefix`, `allowed_tools`, `additional_checks`)
/// are *replaced*, not appended, when an agent profile specifies them. This is intentional:
/// profiles declare their complete, authoritative list.
fn deep_merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => deep_merge(existing, v),
                    None           => { b.insert(k, v); }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

