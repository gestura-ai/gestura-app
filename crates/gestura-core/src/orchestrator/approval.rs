use super::{AgentRole, DelegatedTask};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Approval state tracked by the supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    /// No explicit approval step is required.
    #[default]
    NotRequired,
    /// Waiting for an explicit decision.
    Pending,
    /// Approved to proceed or complete.
    Approved,
    /// Rejected and should not proceed.
    Rejected,
    /// Revision requested before retrying.
    NeedsRevision,
}

/// Gate scope for an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    /// Approval before execution begins.
    PreExecution,
    /// Review approval after execution.
    Review,
    /// Test validation approval after review/execution.
    TestValidation,
}

/// Actor category authorized to make approval decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalActorKind {
    /// End-user/operator acting directly.
    User,
    /// Supervisor or orchestration lead.
    Supervisor,
    /// Reviewer acting on a review gate.
    Reviewer,
    /// Tester acting on a validation gate.
    Tester,
    /// System-generated provenance.
    System,
}

/// Actor provenance for an approval request or decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalActor {
    /// Actor kind.
    pub kind: ApprovalActorKind,
    /// Stable actor identifier or origin label.
    pub id: String,
    /// Optional display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl ApprovalActor {
    /// Build a new actor record.
    pub fn new(kind: ApprovalActorKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
            display_name: None,
        }
    }

    /// Build the standard orchestrator/system actor.
    pub fn system(id: impl Into<String>) -> Self {
        Self::new(ApprovalActorKind::System, id)
    }
}

/// Policy requirement for a single approval scope.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApprovalRequirement {
    /// Scope this requirement applies to.
    pub scope: Option<ApprovalScope>,
    /// Whether the approval gate is required.
    #[serde(default)]
    pub required: bool,
    /// Actor kinds allowed to decide this gate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_deciders: Vec<ApprovalActorKind>,
}

impl ApprovalRequirement {
    fn new(scope: ApprovalScope, required: bool, allowed_deciders: Vec<ApprovalActorKind>) -> Self {
        Self {
            scope: Some(scope),
            required,
            allowed_deciders,
        }
    }

    /// Returns true when the actor kind is allowed for this scope.
    pub fn allows(&self, actor_kind: ApprovalActorKind) -> bool {
        self.allowed_deciders.contains(&actor_kind)
    }
}

/// End-to-end approval policy for a delegated task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApprovalPolicy {
    /// Pre-execution approval requirement.
    #[serde(default)]
    pub pre_execution: ApprovalRequirement,
    /// Review requirement after execution.
    #[serde(default)]
    pub review: ApprovalRequirement,
    /// Test validation requirement after execution/review.
    #[serde(default)]
    pub test_validation: ApprovalRequirement,
}

impl ApprovalPolicy {
    /// Build the default policy for a delegated task.
    pub fn for_task(task: &DelegatedTask) -> Self {
        Self {
            pre_execution: ApprovalRequirement::new(
                ApprovalScope::PreExecution,
                task.approval_required,
                vec![ApprovalActorKind::Supervisor, ApprovalActorKind::User],
            ),
            review: ApprovalRequirement::new(
                ApprovalScope::Review,
                task.reviewer_required,
                vec![ApprovalActorKind::Reviewer, ApprovalActorKind::Supervisor],
            ),
            test_validation: ApprovalRequirement::new(
                ApprovalScope::TestValidation,
                task.test_required,
                vec![ApprovalActorKind::Tester, ApprovalActorKind::Supervisor],
            ),
        }
    }

    /// Return the configured requirement for a scope.
    pub fn requirement(&self, scope: ApprovalScope) -> &ApprovalRequirement {
        match scope {
            ApprovalScope::PreExecution => &self.pre_execution,
            ApprovalScope::Review => &self.review,
            ApprovalScope::TestValidation => &self.test_validation,
        }
    }
}

/// Snapshot of a single approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Request identifier.
    pub id: String,
    /// Gate scope being requested.
    pub scope: ApprovalScope,
    /// Actor that requested the gate.
    pub requested_by: ApprovalActor,
    /// Request timestamp.
    pub requested_at: DateTime<Utc>,
    /// Optional request note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Decision outcome for an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecisionKind {
    /// Gate approved.
    Approved,
    /// Gate explicitly rejected.
    Rejected,
    /// Gate requires revision before retrying.
    NeedsRevision,
}

/// Audit entry for a single approval decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecision {
    /// Decision identifier.
    pub id: String,
    /// Request identifier this decision applies to.
    pub request_id: String,
    /// Gate scope being decided.
    pub scope: ApprovalScope,
    /// Decision actor.
    pub actor: ApprovalActor,
    /// Decision outcome.
    pub decision: ApprovalDecisionKind,
    /// Decision timestamp.
    pub decided_at: DateTime<Utc>,
    /// Optional note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Approval details tracked per task.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskApprovalRecord {
    /// Current approval state.
    pub state: ApprovalState,
    /// Active scope being awaited, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ApprovalScope>,
    /// Active request awaiting a decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_request: Option<ApprovalRequest>,
    /// Approval policy derived for the task.
    #[serde(default)]
    pub policy: ApprovalPolicy,
    /// Historical requests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requests: Vec<ApprovalRequest>,
    /// Historical decisions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<ApprovalDecision>,
    /// When approval was requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<DateTime<Utc>>,
    /// When a decision was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<DateTime<Utc>>,
    /// Actor that made the latest decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    /// Optional explanatory note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TaskApprovalRecord {
    /// Build a record when no explicit gate is currently pending.
    pub fn not_required(task: &DelegatedTask) -> Self {
        Self {
            state: ApprovalState::NotRequired,
            scope: None,
            active_request: None,
            policy: ApprovalPolicy::for_task(task),
            requests: Vec::new(),
            decisions: Vec::new(),
            requested_at: None,
            decided_at: None,
            decided_by: None,
            note: None,
        }
    }

    /// Build a record with an active request.
    pub fn pending(
        task: &DelegatedTask,
        scope: ApprovalScope,
        requested_by: ApprovalActor,
        note: Option<String>,
    ) -> Self {
        let mut record = Self::not_required(task);
        record.request(scope, requested_by, note);
        record
    }

    /// Reset the active gate state while preserving historical audit entries.
    pub fn reset_for_task(&mut self, task: &DelegatedTask) {
        self.state = ApprovalState::NotRequired;
        self.scope = None;
        self.active_request = None;
        self.policy = ApprovalPolicy::for_task(task);
        self.requested_at = None;
        self.decided_at = None;
        self.decided_by = None;
        self.note = None;
    }

    /// Queue a new approval request while preserving prior audit entries.
    pub fn request(
        &mut self,
        scope: ApprovalScope,
        requested_by: ApprovalActor,
        note: Option<String>,
    ) {
        let now = Utc::now();
        let request = ApprovalRequest {
            id: Uuid::new_v4().to_string(),
            scope,
            requested_by,
            requested_at: now,
            note: note.clone(),
        };
        self.state = ApprovalState::Pending;
        self.scope = Some(scope);
        self.active_request = Some(request.clone());
        self.requests.push(request);
        self.requested_at = Some(now);
        self.decided_at = None;
        self.decided_by = None;
        self.note = note;
    }

    /// Return the most recent decision, if any.
    pub fn latest_decision(&self) -> Option<&ApprovalDecision> {
        self.decisions.last()
    }

    /// Resolve the list of allowed actors for the current scope.
    pub fn allowed_actor_kinds(&self, scope: ApprovalScope) -> &[ApprovalActorKind] {
        &self.policy.requirement(scope).allowed_deciders
    }

    /// Ensure the actor is authorized for the given approval scope.
    pub fn authorize(&self, scope: ApprovalScope, actor: &ApprovalActor) -> Result<(), String> {
        let requirement = self.policy.requirement(scope);
        if !requirement.required {
            return Err(format!(
                "Approval scope '{scope:?}' is not required for this task"
            ));
        }
        if !requirement.allows(actor.kind) {
            return Err(format!(
                "Actor '{}' with role '{:?}' is not authorized for {:?} approval",
                actor.id, actor.kind, scope
            ));
        }
        Ok(())
    }

    /// Record an explicit decision for the active request.
    pub fn record_decision(
        &mut self,
        scope: ApprovalScope,
        decision: ApprovalDecisionKind,
        actor: ApprovalActor,
        note: Option<String>,
    ) -> Result<ApprovalDecision, String> {
        self.authorize(scope, &actor)?;
        let active_request = self
            .active_request
            .as_ref()
            .ok_or_else(|| format!("No active approval request exists for {:?}", scope))?;
        if active_request.scope != scope {
            return Err(format!(
                "Active approval request scope mismatch: expected {:?}, found {:?}",
                scope, active_request.scope
            ));
        }

        let decided_at = Utc::now();
        let entry = ApprovalDecision {
            id: Uuid::new_v4().to_string(),
            request_id: active_request.id.clone(),
            scope,
            actor: actor.clone(),
            decision,
            decided_at,
            note: note.clone(),
        };

        self.state = match decision {
            ApprovalDecisionKind::Approved => ApprovalState::Approved,
            ApprovalDecisionKind::Rejected => ApprovalState::Rejected,
            ApprovalDecisionKind::NeedsRevision => ApprovalState::NeedsRevision,
        };
        self.scope = None;
        self.active_request = None;
        self.decisions.push(entry.clone());
        self.decided_at = Some(decided_at);
        self.decided_by = Some(actor.id);
        self.note = note;

        Ok(entry)
    }
}

/// Build the default approval actor role for a task gate scope.
pub fn default_actor_kind_for_scope(scope: ApprovalScope) -> ApprovalActorKind {
    match scope {
        ApprovalScope::PreExecution => ApprovalActorKind::Supervisor,
        ApprovalScope::Review => ApprovalActorKind::Reviewer,
        ApprovalScope::TestValidation => ApprovalActorKind::Tester,
    }
}

/// Infer the default actor kind from an agent role when possible.
pub fn actor_kind_for_agent_role(role: Option<&AgentRole>) -> ApprovalActorKind {
    match role {
        Some(AgentRole::Reviewer | AgentRole::SecurityReviewer) => ApprovalActorKind::Reviewer,
        Some(AgentRole::Tester) => ApprovalActorKind::Tester,
        Some(AgentRole::Supervisor) => ApprovalActorKind::Supervisor,
        _ => ApprovalActorKind::User,
    }
}
