use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Durable outcome labels that corrective learning can attach to turns and tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeSignalKind {
    /// A same-turn retry materially improved the answer.
    RetryImproved,
    /// A same-turn retry did not materially improve the answer.
    RetryDidNotImprove,
    /// Execution finished and is waiting for review approval.
    ExecutionAwaitingReview,
    /// Execution finished and is waiting for explicit test validation.
    ExecutionAwaitingTestValidation,
    /// The task reached a completed state.
    TaskCompleted,
    /// The task failed.
    TaskFailed,
    /// The task became blocked.
    TaskBlocked,
    /// The task was cancelled.
    TaskCancelled,
    /// The task was approved before execution.
    PreExecutionApproved,
    /// The task was rejected before execution.
    PreExecutionRejected,
    /// The task needs revision before execution.
    PreExecutionNeedsRevision,
    /// Review approval was granted.
    ReviewApproved,
    /// Review approval was rejected.
    ReviewRejected,
    /// Review requested revision.
    ReviewNeedsRevision,
    /// Test validation was approved.
    TestValidationApproved,
    /// Test validation was rejected.
    TestValidationRejected,
    /// Test validation requested revision.
    TestValidationNeedsRevision,
}

impl OutcomeSignalKind {
    /// Stable machine-readable identifier suitable for persistence and tags.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RetryImproved => "retry_improved",
            Self::RetryDidNotImprove => "retry_did_not_improve",
            Self::ExecutionAwaitingReview => "execution_awaiting_review",
            Self::ExecutionAwaitingTestValidation => "execution_awaiting_test_validation",
            Self::TaskCompleted => "task_completed",
            Self::TaskFailed => "task_failed",
            Self::TaskBlocked => "task_blocked",
            Self::TaskCancelled => "task_cancelled",
            Self::PreExecutionApproved => "pre_execution_approved",
            Self::PreExecutionRejected => "pre_execution_rejected",
            Self::PreExecutionNeedsRevision => "pre_execution_needs_revision",
            Self::ReviewApproved => "review_approved",
            Self::ReviewRejected => "review_rejected",
            Self::ReviewNeedsRevision => "review_needs_revision",
            Self::TestValidationApproved => "test_validation_approved",
            Self::TestValidationRejected => "test_validation_rejected",
            Self::TestValidationNeedsRevision => "test_validation_needs_revision",
        }
    }

    /// Human-readable label suitable for UI summaries and markdown persistence.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RetryImproved => "Retry improved",
            Self::RetryDidNotImprove => "Retry did not improve",
            Self::ExecutionAwaitingReview => "Execution awaiting review",
            Self::ExecutionAwaitingTestValidation => "Execution awaiting test validation",
            Self::TaskCompleted => "Task completed",
            Self::TaskFailed => "Task failed",
            Self::TaskBlocked => "Task blocked",
            Self::TaskCancelled => "Task cancelled",
            Self::PreExecutionApproved => "Pre-execution approved",
            Self::PreExecutionRejected => "Pre-execution rejected",
            Self::PreExecutionNeedsRevision => "Pre-execution needs revision",
            Self::ReviewApproved => "Review approved",
            Self::ReviewRejected => "Review rejected",
            Self::ReviewNeedsRevision => "Review needs revision",
            Self::TestValidationApproved => "Test validation approved",
            Self::TestValidationRejected => "Test validation rejected",
            Self::TestValidationNeedsRevision => "Test validation needs revision",
        }
    }

    /// Confidence delta used when outcome-linked learning ranks a reflection.
    #[must_use]
    pub const fn confidence_delta(self) -> f32 {
        match self {
            Self::RetryImproved => 0.03,
            Self::RetryDidNotImprove => -0.10,
            Self::ExecutionAwaitingReview | Self::ExecutionAwaitingTestValidation => 0.02,
            Self::TaskCompleted => 0.12,
            Self::TaskFailed | Self::TaskBlocked => -0.12,
            Self::TaskCancelled => -0.08,
            Self::PreExecutionApproved | Self::ReviewApproved | Self::TestValidationApproved => {
                0.08
            }
            Self::PreExecutionNeedsRevision
            | Self::ReviewNeedsRevision
            | Self::TestValidationNeedsRevision => -0.12,
            Self::PreExecutionRejected | Self::ReviewRejected | Self::TestValidationRejected => {
                -0.16
            }
        }
    }
}

impl fmt::Display for OutcomeSignalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Durable outcome observation attached to a reflection, task, or memory record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeSignal {
    /// Outcome label.
    pub kind: OutcomeSignalKind,
    /// When this outcome was observed.
    pub observed_at: DateTime<Utc>,
    /// Optional explanatory detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl OutcomeSignal {
    /// Build a new outcome signal with the current timestamp.
    #[must_use]
    pub fn new(kind: OutcomeSignalKind) -> Self {
        Self {
            kind,
            observed_at: Utc::now(),
            summary: None,
        }
    }

    /// Attach explanatory detail to the signal.
    #[must_use]
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Stable label used for durable metadata and retrieval tags.
    #[must_use]
    pub const fn durable_label(&self) -> &'static str {
        self.kind.as_str()
    }
}
