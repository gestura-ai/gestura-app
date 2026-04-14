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
//!
//! # Multi-agent comparison suite
//! ./target/debug/gestura-eval suite --families gestura,claude-code
//!
//! # Generate report from saved JSON files
//! ./target/debug/gestura-eval report --from ./eval-results/2026-04-14
//! ```

pub mod cli_runner;
pub mod comparison;
pub mod config;
pub mod evaluator;
pub mod html_report;
pub mod orchestrator;
pub mod progress;
pub mod report;
pub mod scenario;

pub use cli_runner::{CliEvalRunner, CliRunnerOptions};
pub use comparison::{
    AgentLatency, AgentRank, CategoryMatrix, CheckHeatmap, ComparisonEngine, ComparisonReport,
    FamilyDegradation, VariationMatrix,
};
pub use config::{
    AgentMeta, AgentMode, EvalConfig, ExecutionConfig, ModelConfig, PermissionConfig,
    ScenarioOverride, SubprocessDef, Thresholds, VariationOverride, BUILTIN_AGENT_IDS,
};
pub use evaluator::{CheckResult, EvaluationResult, RuleEvaluator};
pub use orchestrator::{MultiRunOrchestrator, ProfileSelector, SuiteRunPlan, agent_family};
pub use progress::{ProgressCallback, ProgressEvent};
pub use report::{EvalReport, EvalSummary, ScenarioResult, VariationResult};
pub use scenario::{EvalScenario, EvalScenarioSuite, EvalVariation, HistoryMessage, Rubric};
