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
  | 'approval_decision';

export interface DelegationBrief {
  objective: string;
  acceptance_criteria: string[];
  constraints: string[];
  deliverables: string[];
  context_summary?: string | null;
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
  remote_target?: { url: string; name?: string | null; capabilities: string[] } | null;
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
  created_at: string;
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
  result?: { output: string; success: boolean; duration_ms: number; summary?: string | null } | null;
  messages: TeamMessage[];
  created_at: string;
  updated_at: string;
  started_at?: string | null;
  completed_at?: string | null;
}

export interface SupervisorRun {
  id: string;
  session_id?: string | null;
  workspace_dir?: string | null;
  lead_agent_id?: string | null;
  status: SupervisorRunStatus;
  tasks: SupervisorTaskRecord[];
  messages: TeamMessage[];
  created_at: string;
  updated_at: string;
  completed_at?: string | null;
}

export const listActiveTasks = async (): Promise<DelegatedTask[]> => {
  return await invokeTauri<DelegatedTask[]>('list_active_tasks');
};

export const listSupervisorRuns = async (): Promise<SupervisorRun[]> => {
  return await invokeTauri<SupervisorRun[]>('list_supervisor_runs');
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
