import { invokeTauri } from './invoke';

export type AgentRole =
  | 'supervisor'
  | 'researcher'
  | 'implementer'
  | 'reviewer'
  | 'tester'
  | 'security_reviewer'
  | 'remote_worker'
  | string;

export type AgentExecutionMode = 'shared_workspace' | 'isolated_workspace' | 'git_worktree' | 'remote';
export type ApprovalState = 'not_required' | 'pending' | 'approved' | 'rejected' | 'needs_revision';
export type ApprovalScope = 'pre_execution' | 'review' | 'test_validation';
export type ApprovalActorKind = 'user' | 'supervisor' | 'reviewer' | 'tester' | 'system';
export type ApprovalDecisionKind = 'approved' | 'rejected' | 'needs_revision';
export type SupervisorTaskState =
  | 'queued'
  | 'blocked'
  | 'pending_approval'
  | 'running'
  | 'review_pending'
  | 'test_pending'
  | 'completed'
  | 'failed'
  | 'cancelled';
export type SupervisorRunStatus = 'draft' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled';
export type EnvironmentState =
  | 'requested'
  | 'provisioning'
  | 'ready'
  | 'in_use'
  | 'cleanup_queued'
  | 'cleaning'
  | 'archived'
  | 'removed'
  | 'recovering'
  | 'failed';
export type EnvironmentHealth = 'clean' | 'dirty' | 'missing' | 'drifted' | 'orphaned' | 'unknown';
export type RecoveryStatus = 'not_required' | 'pending' | 'reconciled' | 'needs_operator_action' | 'failed';
export type RecoveryAction =
  | 'noop'
  | 'recreate_missing_environment'
  | 'release_stale_lease'
  | 'archive_dirty_environment'
  | 'queue_cleanup'
  | 'mark_task_blocked';
export type TeamMessageKind =
  | 'status_update'
  | 'clarification'
  | 'blocker'
  | 'handoff'
  | 'review_feedback'
  | 'approval_decision'
  | 'review_request'
  | 'approval_request'
  | 'test_validation_request';
export type CollaborationRequestKind =
  | 'blocker_escalation'
  | 'handoff'
  | 'clarification'
  | 'review_request'
  | 'approval_request'
  | 'test_validation_request';
export type CollaborationActionStatus = 'open' | 'acknowledged' | 'resolved' | 'needs_revision' | 'cancelled';
export type CollaborationThreadStatus = 'active' | 'action_required' | 'needs_revision' | 'resolved';
export type CollaborationEscalationLevel = 'info' | 'warning' | 'critical';

export interface DelegationBrief {
  objective: string;
  acceptance_criteria: string[];
  constraints: string[];
  deliverables: string[];
  context_summary?: string | null;
}

export interface RemoteAgentTarget {
  url: string;
  name?: string | null;
  auth_token?: string | null;
  capabilities: string[];
}

export interface DelegatedTask {
  id: string;
  agent_id: string;
  prompt: string;
  required_tools: string[];
  priority: number;
  context?: Record<string, unknown> | null;
  session_id?: string | null;
  directive_id?: string | null;
  tracking_task_id?: string | null;
  run_id?: string | null;
  parent_task_id?: string | null;
  depends_on: string[];
  role?: AgentRole | null;
  delegation_brief?: DelegationBrief | null;
  planning_only: boolean;
  approval_required: boolean;
  reviewer_required: boolean;
  test_required: boolean;
  workspace_dir?: string | null;
  execution_mode: AgentExecutionMode;
  environment_id?: string | null;
  remote_target?: RemoteAgentTarget | null;
  memory_tags: string[];
  name?: string | null;
}

export interface ExecutionEnvironment {
  id: string;
  execution_mode: AgentExecutionMode;
  root_dir: string;
  write_access: boolean;
  branch_name?: string | null;
  worktree_path?: string | null;
  remote_url?: string | null;
  state: EnvironmentState;
  health: EnvironmentHealth;
  cleanup_policy: string;
  recovery_status: RecoveryStatus;
  recovery_action?: RecoveryAction | null;
  failure?: { kind: string; message: string; occurred_at: string; command?: string | null; stderr?: string | null } | null;
  cleanup_result?: { disposition: string; completed_at: string; retained_path?: string | null; summary: string } | null;
}

export interface RemoteExecutionProgress {
  stage?: string | null;
  message?: string | null;
  percent?: number | null;
  updated_at: string;
}

export interface RemoteExecutionArtifact {
  name: string;
  part_count: number;
  metadata: Record<string, unknown>;
}

export interface RemoteExecutionCompatibility {
  supported_features: string[];
  warnings: string[];
  protocol_version?: string | null;
}

export interface RemoteTaskLease {
  lease_id: string;
  holder_agent_id?: string | null;
  acquired_at: string;
  last_heartbeat_at: string;
  expires_at: string;
  heartbeat_interval_secs: number;
}

export interface RemoteTaskProvenance {
  caller_agent_id?: string | null;
  caller_name?: string | null;
  caller_version?: string | null;
  caller_capabilities: string[];
}

export interface RemoteExecutionRecord {
  target: RemoteAgentTarget;
  remote_task_id: string;
  status: string;
  status_reason?: string | null;
  lease?: RemoteTaskLease | null;
  progress?: RemoteExecutionProgress | null;
  artifacts: RemoteExecutionArtifact[];
  provenance?: RemoteTaskProvenance | null;
  compatibility: RemoteExecutionCompatibility;
  last_synced_at: string;
}

export interface EnvironmentRecord {
  id: string;
  spec: {
    id: string;
    execution_mode: AgentExecutionMode;
    workspace_root: string;
    prepared_path: string;
    session_id?: string | null;
    run_id: string;
    task_id: string;
    agent_id: string;
    cleanup_policy: string;
    write_access: boolean;
    git_worktree?: {
      repo_root: string;
      base_branch: string;
      worktree_branch: string;
      worktree_path: string;
      create_branch_if_missing: boolean;
    } | null;
    remote_url?: string | null;
  };
  state: EnvironmentState;
  health: EnvironmentHealth;
  prepared_path: string;
  lease?: { task_id: string; agent_id: string; lease_kind: string; acquired_at: string; released_at?: string | null } | null;
  cleanup_result?: { disposition: string; completed_at: string; retained_path?: string | null; summary: string } | null;
  recovery_status: RecoveryStatus;
  recovery_action?: RecoveryAction | null;
  failure?: { kind: string; message: string; occurred_at: string; command?: string | null; stderr?: string | null } | null;
  created_at: string;
  updated_at: string;
  last_verified_at?: string | null;
}

export interface TaskApprovalRecord {
  state: ApprovalState;
  scope?: ApprovalScope | null;
  active_request?: ApprovalRequest | null;
  policy: ApprovalPolicy;
  requests: ApprovalRequest[];
  decisions: ApprovalDecision[];
  requested_at?: string | null;
  decided_at?: string | null;
  decided_by?: string | null;
  note?: string | null;
}

export interface ApprovalActor {
  kind: ApprovalActorKind;
  id: string;
  display_name?: string | null;
}

export interface ApprovalRequirement {
  scope?: ApprovalScope | null;
  required: boolean;
  allowed_deciders: ApprovalActorKind[];
}

export interface ApprovalPolicy {
  pre_execution: ApprovalRequirement;
  review: ApprovalRequirement;
  test_validation: ApprovalRequirement;
}

export interface ApprovalRequest {
  id: string;
  scope: ApprovalScope;
  requested_by: ApprovalActor;
  requested_at: string;
  note?: string | null;
}

export interface ApprovalDecision {
  id: string;
  request_id: string;
  scope: ApprovalScope;
  actor: ApprovalActor;
  decision: ApprovalDecisionKind;
  decided_at: string;
  note?: string | null;
}

export interface TeamMessage {
  id: string;
  run_id: string;
  task_id?: string | null;
  kind: TeamMessageKind;
  sender_agent_id?: string | null;
  recipient_agent_id?: string | null;
  content: string;
  thread_id?: string | null;
  reply_to_message_id?: string | null;
  action_request?: TeamActionRequest | null;
  escalation?: TeamEscalation | null;
  result_reference?: TeamResultReference | null;
  artifact_references?: TeamArtifactReference[];
  unread_by_agent_ids?: string[];
  archived_at?: string | null;
  archived_by_agent_id?: string | null;
  archive_note?: string | null;
  created_at: string;
}

export interface TeamActionRequestDraft {
  kind: CollaborationRequestKind;
  requested_for_agent_ids?: string[];
  requested_for_roles?: AgentRole[];
  requested_for_actor_kinds?: ApprovalActorKind[];
  approval_scope?: ApprovalScope | null;
  note?: string | null;
}

export interface TeamActionRequest {
  id: string;
  kind: CollaborationRequestKind;
  status: CollaborationActionStatus;
  requested_at: string;
  requested_by_agent_id?: string | null;
  requested_for_agent_ids: string[];
  requested_for_roles: AgentRole[];
  requested_for_actor_kinds: ApprovalActorKind[];
  approval_scope?: ApprovalScope | null;
  note?: string | null;
  resolved_at?: string | null;
  resolved_by_agent_id?: string | null;
  resolution_note?: string | null;
}

export interface TeamEscalation {
  level: CollaborationEscalationLevel;
  escalated_at: string;
  escalated_by_agent_id?: string | null;
  target_role?: AgentRole | null;
  note?: string | null;
}

export interface TeamEscalationDraft {
  level: CollaborationEscalationLevel;
  escalated_by_agent_id?: string | null;
  target_role?: AgentRole | null;
  note?: string | null;
}

export interface TeamMessageDraft {
  task_id?: string | null;
  kind: TeamMessageKind;
  sender_agent_id?: string | null;
  recipient_agent_id?: string | null;
  content: string;
  thread_id?: string | null;
  reply_to_message_id?: string | null;
  action_request?: TeamActionRequestDraft | null;
  escalation?: TeamEscalationDraft | null;
  unread_by_agent_ids?: string[];
}

export interface TeamArtifactReference {
  task_id?: string | null;
  name: string;
  kind: string;
  uri?: string | null;
  summary?: string | null;
}

export interface TeamResultReference {
  task_id: string;
  success: boolean;
  summary?: string | null;
  artifact_names: string[];
  duration_ms: number;
}

export interface TeamThread {
  id: string;
  run_id: string;
  task_id?: string | null;
  kind: TeamMessageKind;
  status: CollaborationThreadStatus;
  created_at: string;
  updated_at: string;
  archived: boolean;
  archived_at?: string | null;
  unread_count: number;
  message_count: number;
  actionable_message_count: number;
  requires_attention: boolean;
  participant_agent_ids: string[];
  latest_action_request?: TeamActionRequest | null;
  latest_result_reference?: TeamResultReference | null;
  artifact_references: TeamArtifactReference[];
  messages: TeamMessage[];
}

export interface SupervisorTaskRecord {
  task: DelegatedTask;
  state: SupervisorTaskState;
  approval: TaskApprovalRecord;
  environment_id: string;
  environment: ExecutionEnvironment;
  claimed_by?: string | null;
  attempts: number;
  blocked_reasons: string[];
  result?: {
    output: string;
    success: boolean;
    duration_ms: number;
    summary?: string | null;
    terminal_state_hint?: 'completed' | 'failed' | 'cancelled' | 'blocked' | null;
    artifacts?: Array<{ name: string; kind: string; uri?: string | null; summary?: string | null }>;
  } | null;
  remote_execution?: RemoteExecutionRecord | null;
  messages: TeamMessage[];
  created_at: string;
  updated_at: string;
  started_at?: string | null;
  completed_at?: string | null;
}

export interface SupervisorRunTaskSummary {
  total: number;
  queued: number;
  blocked: number;
  pending_approval: number;
  running: number;
  review_pending: number;
  test_pending: number;
  completed: number;
  failed: number;
  cancelled: number;
}

export interface SupervisorInheritancePolicy {
  approval_required: boolean;
  reviewer_required: boolean;
  test_required: boolean;
  execution_mode?: AgentExecutionMode | null;
  workspace_dir?: string | null;
  memory_tags: string[];
  constraint_notes: string[];
}

export interface SupervisorParentRunRef {
  parent_run_id: string;
  parent_task_id?: string | null;
  delegated_by_agent_id?: string | null;
  objective: string;
  created_at: string;
}

export interface ChildSupervisorRunSummary {
  run_id: string;
  name?: string | null;
  objective: string;
  lead_agent_id?: string | null;
  status: SupervisorRunStatus;
  task_summary: SupervisorRunTaskSummary;
  requires_attention: boolean;
  blocked_reasons: string[];
  created_at: string;
  updated_at: string;
  completed_at?: string | null;
}

export interface SupervisorHierarchySummary {
  depth: number;
  max_depth: number;
  child_run_count: number;
  descendant_task_count: number;
  action_required_child_count: number;
  rollup_status: SupervisorRunStatus;
  requires_attention: boolean;
  blocked_reasons: string[];
}

export interface SupervisorRun {
  id: string;
  name?: string | null;
  session_id?: string | null;
  workspace_dir?: string | null;
  lead_agent_id?: string | null;
  parent_run?: SupervisorParentRunRef | null;
  child_runs: ChildSupervisorRunSummary[];
  hierarchy_depth: number;
  max_hierarchy_depth: number;
  inherited_policy?: SupervisorInheritancePolicy | null;
  status: SupervisorRunStatus;
  task_summary: SupervisorRunTaskSummary;
  hierarchy_summary?: SupervisorHierarchySummary | null;
  tasks: SupervisorTaskRecord[];
  messages: TeamMessage[];
  created_at: string;
  updated_at: string;
  completed_at?: string | null;
}

export interface ChildSupervisorRunRequest {
  parent_run_id: string;
  run_id?: string | null;
  lead_agent_id: string;
  objective: string;
  name?: string | null;
  parent_task_id?: string | null;
  session_id?: string | null;
  workspace_dir?: string | null;
  approval_required: boolean;
  reviewer_required: boolean;
  test_required: boolean;
  execution_mode: AgentExecutionMode;
  memory_tags: string[];
  constraint_notes: string[];
}

export const listActiveTasks = async (): Promise<DelegatedTask[]> => {
  return await invokeTauri<DelegatedTask[]>('list_active_tasks');
};

export const listSupervisorRuns = async (): Promise<SupervisorRun[]> => {
  return await invokeTauri<SupervisorRun[]>('list_supervisor_runs');
};

export const listRootSupervisorRuns = async (): Promise<SupervisorRun[]> => {
  return await invokeTauri<SupervisorRun[]>('list_root_supervisor_runs');
};

export const listChildSupervisorRuns = async (parentRunId: string): Promise<SupervisorRun[]> => {
  return await invokeTauri<SupervisorRun[]>('list_child_supervisor_runs', { parentRunId });
};

export const getSupervisorRunAncestry = async (runId: string): Promise<SupervisorRun[]> => {
  return await invokeTauri<SupervisorRun[]>('get_supervisor_run_ancestry', { runId });
};

export const getSupervisorRunDescendants = async (runId: string): Promise<SupervisorRun[]> => {
  return await invokeTauri<SupervisorRun[]>('get_supervisor_run_descendants', { runId });
};

export const listSupervisorLeafTasks = async (runId: string): Promise<SupervisorTaskRecord[]> => {
  return await invokeTauri<SupervisorTaskRecord[]>('list_supervisor_leaf_tasks', { runId });
};

export const createChildSupervisorRun = async (request: ChildSupervisorRunRequest): Promise<SupervisorRun> => {
  return await invokeTauri<SupervisorRun>('create_child_supervisor_run', { request });
};

export const listWorkflowEnvironments = async (runId?: string): Promise<EnvironmentRecord[]> => {
  return await invokeTauri<EnvironmentRecord[]>('list_workflow_environments', { run_id: runId ?? null });
};

export const getWorkflowEnvironment = async (environmentId: string): Promise<EnvironmentRecord | null> => {
  return await invokeTauri<EnvironmentRecord | null>('get_workflow_environment', { environment_id: environmentId });
};

export const delegateTask = async (task: DelegatedTask): Promise<string> => {
  return await invokeTauri<string>('delegate_task', { task });
};

export const cancelTask = async (taskId: string): Promise<void> => {
  await invokeTauri('cancel_task', { task_id: taskId });
};

export const approveWorkflowTask = async (taskId: string, actor: ApprovalActor, note?: string): Promise<void> => {
  await invokeTauri('approve_workflow_task', { task_id: taskId, actor, note });
};

export const rejectWorkflowTask = async (taskId: string, actor: ApprovalActor, note?: string): Promise<void> => {
  await invokeTauri('reject_workflow_task', { task_id: taskId, actor, note });
};

export const retryWorkflowTask = async (taskId: string): Promise<void> => {
  await invokeTauri('retry_workflow_task', { task_id: taskId });
};

export const retryWorkflowEnvironment = async (environmentId: string): Promise<EnvironmentRecord> => {
  return await invokeTauri<EnvironmentRecord>('retry_workflow_environment', { environment_id: environmentId });
};

export const cleanupWorkflowEnvironment = async (
  environmentId: string,
  archiveIfDirty = true
): Promise<EnvironmentRecord> => {
  return await invokeTauri<EnvironmentRecord>('cleanup_workflow_environment', {
    environment_id: environmentId,
    archive_if_dirty: archiveIfDirty,
  });
};

export const reconcileWorkflowState = async (): Promise<void> => {
  await invokeTauri('reconcile_workflow_state');
};

export const claimWorkflowTask = async (taskId: string, agentId: string): Promise<void> => {
  await invokeTauri('claim_workflow_task', { task_id: taskId, agent_id: agentId });
};

export const sendWorkflowMessage = async (
  runId: string,
  content: string,
  kind: TeamMessageKind = 'status_update',
  taskId?: string,
  senderAgentId?: string,
  recipientAgentId?: string
): Promise<TeamMessage> => {
  return await invokeTauri<TeamMessage>('send_workflow_message', {
    run_id: runId,
    task_id: taskId,
    kind,
    sender_agent_id: senderAgentId,
    recipient_agent_id: recipientAgentId,
    content,
  });
};

export const sendWorkflowCollaborationMessage = async (runId: string, draft: TeamMessageDraft): Promise<TeamMessage> => {
  return await invokeTauri<TeamMessage>('send_workflow_collaboration_message', { run_id: runId, draft });
};

export const listWorkflowThreads = async (runId: string, includeArchived = false): Promise<TeamThread[]> => {
  return await invokeTauri<TeamThread[]>('list_workflow_threads', { run_id: runId, include_archived: includeArchived });
};

export const updateWorkflowThreadAction = async (
  runId: string,
  threadId: string,
  status: CollaborationActionStatus,
  actorId?: string,
  note?: string
): Promise<TeamThread> => {
  return await invokeTauri<TeamThread>('update_workflow_thread_action', {
    run_id: runId,
    thread_id: threadId,
    status,
    actor_id: actorId,
    note,
  });
};

export const archiveWorkflowThread = async (
  runId: string,
  threadId: string,
  actorId?: string,
  note?: string
): Promise<TeamThread> => {
  return await invokeTauri<TeamThread>('archive_workflow_thread', {
    run_id: runId,
    thread_id: threadId,
    actor_id: actorId,
    note,
  });
};

export const spawnSubagent = async (
  agentId: string,
  name: string,
  options: Partial<Pick<DelegatedTask, 'role' | 'execution_mode' | 'workspace_dir'>> & { capabilities?: string[] } = {}
): Promise<void> => {
  await invokeTauri('spawn_subagent', {
    agent_id: agentId,
    name,
    role: options.role,
    execution_mode: options.execution_mode,
    capabilities: options.capabilities,
    workspace_dir: options.workspace_dir,
  });
};
