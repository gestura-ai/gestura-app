//! # gestura-core-eval
//!
//! Reproducible evaluation harness for Gestura agentic response quality.
//!
//! Contains 8 standardised test scenarios × 3 prompt variations each, covering:
//! simple queries, multi-turn conversation, complex planning, error handling,
//! tool extensibility, privacy-sensitive tasks, context retention, and
//! long-context coherence.
//!
//! The harness is intentionally **separate from the `gestura` product binary**.
//! It ships as the standalone `gestura-eval` binary that drives `gestura` as a
//! black-box subprocess, keeping the thin CLI interface uncontaminated with
//! eval logic and making the tests reproducible across any agentic interface
//! that wraps the same underlying binary.
//!
//! ## Quick start
//!
//! ```bash
//! # Build both binaries
//! cargo build -p gestura-cli -p gestura-core-eval
//!
//! # List scenario IDs
//! ./target/debug/gestura-eval --list
//!
//! # Dry-run: validate check logic without subprocess calls
//! ./target/debug/gestura-eval --dry-run
//!
//! # Full run (requires a configured LLM in gestura)
//! ./target/debug/gestura-eval
//!
//! # Single scenario, JSON report
//! ./target/debug/gestura-eval --scenario s1_simple_query --json
//! ```

pub mod cli_runner;
pub mod config;
pub mod evaluator;
pub mod report;
pub mod scenario;

pub use cli_runner::{CliEvalRunner, CliRunnerOptions};
pub use config::{
    AgentMeta, AgentMode, EvalConfig, ExecutionConfig, ModelConfig, PermissionConfig,
    ScenarioOverride, SubprocessDef, Thresholds, VariationOverride, BUILTIN_AGENT_IDS,
};
pub use evaluator::{CheckResult, EvaluationResult, RuleEvaluator};
pub use report::{EvalReport, EvalSummary, ScenarioResult, VariationResult};
pub use scenario::{EvalScenario, EvalScenarioSuite, EvalVariation, HistoryMessage, Rubric};

