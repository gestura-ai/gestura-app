//! Progress events emitted during a suite run.
//!
//! The runner fires a typed [`ProgressEvent`] on every significant state
//! transition. Consumers (terminal renderer, test helpers, CI loggers) register
//! a [`ProgressCallback`] and receive events synchronously on the runner thread.
//!
//! Using `Arc<dyn Fn(ProgressEvent) + Send + Sync>` keeps the callback cheap to
//! clone and share between the orchestrator and individual profile runners.

use std::sync::Arc;

use crate::report::EvalReport;

/// A state transition emitted by the runner or orchestrator.
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// An agent profile run is starting.
    ProfileStarted {
        agent_id: String,
        /// Total number of variations that will be run (across all selected scenarios).
        total_variations: usize,
    },

    /// A single variation has finished evaluation.
    VariationDone {
        agent_id: String,
        scenario_id: String,
        variation_id: String,
        passed: bool,
        /// Fraction of checks that passed (0.0 – 1.0).
        score: f32,
        /// Wall-clock time for the subprocess call.
        duration_ms: u64,
    },

    /// A variation hit a rate-limit error and is pausing before a retry.
    RateLimitRetry {
        agent_id: String,
        scenario_id: String,
        variation_id: String,
        /// Which attempt just failed (1 = first attempt, 2 = first retry, …).
        attempt: u32,
        /// Total attempts allowed (1 + retries).
        max_attempts: u32,
        /// How long the runner will sleep before the next attempt.
        wait_secs: u64,
    },

    /// An agent profile has finished all variations.
    ProfileFinished {
        agent_id: String,
        report: EvalReport,
    },

    /// A profile was skipped (e.g. missing manual-auth token).
    ProfileSkipped {
        agent_id: String,
        reason: String,
    },

    /// The entire suite has finished all profiles.
    SuiteFinished {
        elapsed_secs: f64,
    },
}

/// A shared, cheaply-cloneable progress callback.
///
/// `Arc` wrapping means all callers share the same underlying function — no
/// allocation per clone, no lifetime restrictions.
pub type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync + 'static>;
