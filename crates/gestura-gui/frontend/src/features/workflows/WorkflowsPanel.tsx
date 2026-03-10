import React, { useMemo, useState } from 'react';
import { Agent, listAgents } from '../../services/tauri/agents';
import {
  ApprovalActor,
  ApprovalActorKind,
  ApprovalScope,
  approveWorkflowTask,
  cancelTask,
  claimWorkflowTask,
  DelegatedTask,
  delegateTask,
  EnvironmentRecord,
  cleanupWorkflowEnvironment,
  listActiveTasks,
  listSupervisorRuns,
  listWorkflowEnvironments,
  reconcileWorkflowState,
  rejectWorkflowTask,
  retryWorkflowEnvironment,
  retryWorkflowTask,
  sendWorkflowMessage,
  spawnSubagent,
  SupervisorRun,
  SupervisorTaskRecord,
} from '../../services/tauri/workflows';
import { Button } from '../../shared/components/Button';
import { FormGroup } from '../../shared/components/FormGroup';
import { useAsyncState } from '../../shared/hooks/useAsyncState';
import { useInterval } from '../../shared/hooks/useInterval';

const gatedStates = new Set(['pending_approval', 'review_pending', 'test_pending']);
const retryableStates = new Set(['failed', 'blocked', 'cancelled']);

const summarizeTask = (task: SupervisorTaskRecord | DelegatedTask): string => {
  const value = 'task' in task ? task.task.name ?? task.task.prompt : task.name ?? task.prompt;
  return value.length > 120 ? `${value.slice(0, 117)}...` : value;
};

const approvalScopeForState = (state: SupervisorTaskRecord['state']): ApprovalScope | null => {
  switch (state) {
    case 'pending_approval':
      return 'pre_execution';
    case 'review_pending':
      return 'review';
    case 'test_pending':
      return 'test_validation';
    default:
      return null;
  }
};

const defaultApprovalActorKind = (state: SupervisorTaskRecord['state']): ApprovalActorKind => {
  switch (state) {
    case 'pending_approval':
      return 'supervisor';
    case 'review_pending':
      return 'reviewer';
    case 'test_pending':
      return 'tester';
    default:
      return 'user';
  }
};

const approvalActorOptionsForRecord = (record: SupervisorTaskRecord): ApprovalActorKind[] => {
  const scope = approvalScopeForState(record.state);
  if (!scope) return [];
  const requirement =
    scope === 'pre_execution'
      ? record.approval.policy.pre_execution
      : scope === 'review'
        ? record.approval.policy.review
        : record.approval.policy.test_validation;
  return requirement.allowed_deciders.length > 0 ? requirement.allowed_deciders : [defaultApprovalActorKind(record.state)];
};

const formatApprovalActorLabel = (kind: ApprovalActorKind): string =>
  kind
    .split('_')
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join(' ');

const buildApprovalActor = (record: SupervisorTaskRecord, kind: ApprovalActorKind): ApprovalActor => ({
  kind,
  id: `workflow-panel:${record.task.id}:${kind}`,
  display_name: `Workflow Panel (${formatApprovalActorLabel(kind)})`,
});

const WorkflowsPanel: React.FC = () => {
  const [activeTab, setActiveTab] = useState<'runs' | 'environments'>('runs');
  const [showNewTask, setShowNewTask] = useState(false);
  const [messageDrafts, setMessageDrafts] = useState<Record<string, string>>({});
  const [claimSelections, setClaimSelections] = useState<Record<string, string>>({});
  const [approvalActorSelections, setApprovalActorSelections] = useState<Record<string, ApprovalActorKind>>({});
  const [newTask, setNewTask] = useState({
    description: '',
    agentId: '',
    priority: 5,
    role: 'implementer',
    approvalRequired: false,
    reviewerRequired: false,
    testRequired: false,
    planningOnly: false,
    executionMode: 'shared_workspace',
  });

  const workflowsState = useAsyncState(
    async () => {
      const [tasks, runs, environments, agentRes] = await Promise.all([
        listActiveTasks(),
        listSupervisorRuns(),
        listWorkflowEnvironments(),
        listAgents(),
      ]);
      return {
        activeTasks: tasks,
        runs,
        environments,
        agents: (Array.isArray(agentRes?.agents) ? agentRes.agents : []) as Agent[],
      };
    },
    { errorMessage: 'Failed to load workflow data:' }
  );

  const activeTasks = workflowsState.data?.activeTasks ?? [];
  const runs = workflowsState.data?.runs ?? [];
  const environments = workflowsState.data?.environments ?? [];
  const agents = workflowsState.data?.agents ?? [];

  const sortedRuns = useMemo(
    () => [...runs].sort((left, right) => right.updated_at.localeCompare(left.updated_at)),
    [runs]
  );
  const sortedEnvironments = useMemo(
    () => [...environments].sort((left, right) => right.updated_at.localeCompare(left.updated_at)),
    [environments]
  );

  useInterval(() => {
    void workflowsState.reload({ showLoading: false });
  }, 4000);

  const refresh = () => workflowsState.reload({ showLoading: false });

  const handleCreateTask = async () => {
    if (!newTask.description.trim()) return;

    const task: DelegatedTask = {
      id: `task-${Date.now()}`,
      agent_id: newTask.agentId || `agent-${Date.now()}`,
      prompt: newTask.description.trim(),
      required_tools: [],
      priority: newTask.priority,
      context: {},
      depends_on: [],
      role: newTask.role,
      delegation_brief: {
        objective: newTask.description.trim(),
        acceptance_criteria: [],
        constraints: [],
        deliverables: ['Status summary'],
        context_summary: null,
      },
      planning_only: newTask.planningOnly,
      approval_required: newTask.approvalRequired,
      reviewer_required: newTask.reviewerRequired,
      test_required: newTask.testRequired,
      execution_mode: newTask.executionMode as DelegatedTask['execution_mode'],
      memory_tags: ['workflow-panel'],
      name: newTask.description.trim(),
      run_id: `run-${Date.now()}`,
    };

    try {
      await delegateTask(task);
      setNewTask({
        description: '',
        agentId: '',
        priority: 5,
        role: 'implementer',
        approvalRequired: false,
        reviewerRequired: false,
        testRequired: false,
        planningOnly: false,
        executionMode: 'shared_workspace',
      });
      setShowNewTask(false);
      await refresh();
    } catch (error) {
      console.error('Failed to create workflow task:', error);
    }
  };

  const handleSpawnAgent = async () => {
    const id = `agent-${Date.now()}`;
    const name = `Agent ${agents.length + 1}`;
    try {
      await spawnSubagent(id, name, { role: 'implementer' });
      await refresh();
    } catch (error) {
      console.error('Failed to spawn agent:', error);
    }
  };

  const handleTaskAction = async (action: () => Promise<void>) => {
    try {
      await action();
      await refresh();
    } catch (error) {
      console.error('Workflow action failed:', error);
    }
  };

  const handleSendMessage = async (runId: string) => {
    const content = messageDrafts[runId]?.trim();
    if (!content) return;
    try {
      await sendWorkflowMessage(runId, content);
      setMessageDrafts((current) => ({ ...current, [runId]: '' }));
      await refresh();
    } catch (error) {
      console.error('Failed to send workflow message:', error);
    }
  };

  const handleEnvironmentAction = async (action: () => Promise<void>) => {
    try {
      await action();
      await refresh();
    } catch (error) {
      console.error('Environment action failed:', error);
    }
  };

  if (workflowsState.loading) {
    return (
      <div className="workflows-panel">
        <h2>Workflows</h2>
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <div className="workflows-panel">
      <div className="workflows-header">
        <div>
          <h2>Supervisor Workflows</h2>
          <p>Coordinate specialist agents, approvals, run state, and handoff messaging.</p>
        </div>
        <div className="workflows-actions">
          <Button tone="secondary" onClick={() => handleEnvironmentAction(() => reconcileWorkflowState())}>Reconcile</Button>
          <Button tone="secondary" onClick={handleSpawnAgent}>+ Agent</Button>
          <Button onClick={() => setShowNewTask(true)}>+ Workflow Task</Button>
        </div>
      </div>

      <div className="workflow-tabs" style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
        <Button tone={activeTab === 'runs' ? 'primary' : 'secondary'} onClick={() => setActiveTab('runs')}>
          Runs ({sortedRuns.length})
        </Button>
        <Button tone={activeTab === 'environments' ? 'primary' : 'secondary'} onClick={() => setActiveTab('environments')}>
          Environments ({sortedEnvironments.length})
        </Button>
      </div>

      <div className="workflows-content">
        <div className="workflows-section">
          <h3>Agents ({agents.length})</h3>
          {agents.length === 0 ? (
            <p className="empty-state">No agents running. Spawn one to start delegating.</p>
          ) : (
            <div className="agent-list">
              {agents.map((agent) => (
                <div key={agent.id} className="agent-card">
                  <div className="agent-name">🤖 {agent.name}</div>
                  <div className="agent-status">{agent.status}</div>
                  {'role' in agent && agent.role ? <div className="agent-role">Role: {String(agent.role)}</div> : null}
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="workflows-section">
          <h3>Live Active Tasks ({activeTasks.length})</h3>
          {activeTasks.length === 0 ? (
            <p className="empty-state">No tasks are actively executing right now.</p>
          ) : (
            <div className="task-list">
              {activeTasks.map((task) => (
                <div key={task.id} className="task-card">
                  <div className="task-header">
                    <span className="task-id">{task.id}</span>
                    <span className="task-priority">P{task.priority}</span>
                  </div>
                  <div className="task-description">{summarizeTask(task)}</div>
                  <div className="task-footer">
                    <span className="task-agent">Agent: {task.agent_id}</span>
                    <Button tone="danger" size="small" onClick={() => handleTaskAction(() => cancelTask(task.id))}>
                      Cancel
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {activeTab === 'runs' ? (
          <div className="workflows-section">
            <h3>Supervisor Runs ({sortedRuns.length})</h3>
            {sortedRuns.length === 0 ? (
              <p className="empty-state">No supervisor runs yet. Create a workflow task to start one.</p>
            ) : (
              <div className="task-list">
                {sortedRuns.map((run: SupervisorRun) => (
                  <div key={run.id} className="task-card">
                    <div className="task-header">
                      <span className="task-id">{run.id}</span>
                      <span className="task-priority">{run.status}</span>
                    </div>
                    <div className="task-footer">
                      <span>{run.tasks.length} tasks</span>
                      <span>Updated {new Date(run.updated_at).toLocaleString()}</span>
                    </div>

                    <div className="task-list">
                      {run.tasks.map((record) => {
                        const claimAgent = claimSelections[record.task.id] ?? record.task.agent_id;
                        const approvalScope = approvalScopeForState(record.state);
                        const approvalActorOptions = approvalActorOptionsForRecord(record);
                        const approvalActorKind =
                          approvalActorSelections[record.task.id] ?? approvalActorOptions[0] ?? defaultApprovalActorKind(record.state);
                        const latestDecision = record.approval.decisions[record.approval.decisions.length - 1];
                        return (
                          <div key={record.task.id} className="task-card">
                            <div className="task-header">
                              <span className="task-id">{record.task.id}</span>
                              <span className="task-priority">{record.state}</span>
                            </div>
                            <div className="task-description">{summarizeTask(record)}</div>
                            <div className="task-footer">
                              <span>Role: {record.task.role ?? 'implementer'}</span>
                              <span>Approval: {record.approval.state}</span>
                            </div>
                            {approvalScope ? (
                              <div className="task-description">
                                Gate: {approvalScope} · Requested by{' '}
                                {record.approval.active_request?.requested_by.display_name ?? record.approval.active_request?.requested_by.id ?? 'system'}
                              </div>
                            ) : null}
                            {approvalScope && approvalActorOptions.length > 0 ? (
                              <div className="task-description">
                                Allowed approvers: {approvalActorOptions.map(formatApprovalActorLabel).join(', ')}
                              </div>
                            ) : null}
                            {latestDecision ? (
                              <div className="task-description">
                                Last decision: {latestDecision.decision} by {latestDecision.actor.display_name ?? latestDecision.actor.id}
                              </div>
                            ) : null}
                            {record.blocked_reasons.length > 0 ? (
                              <div className="task-description">Blocked by: {record.blocked_reasons.join('; ')}</div>
                            ) : null}
                            <div className="task-footer">
                              <span>
                                Env: {record.environment.execution_mode} · {record.environment.state}/{record.environment.health}
                              </span>
                              <span>Owner: {record.claimed_by ?? record.task.agent_id}</span>
                            </div>
                            <div className="modal-actions">
                              {gatedStates.has(record.state) ? (
                                <>
                                  <select
                                    value={approvalActorKind}
                                    onChange={(event) =>
                                      setApprovalActorSelections((current) => ({
                                        ...current,
                                        [record.task.id]: event.target.value as ApprovalActorKind,
                                      }))
                                    }
                                  >
                                    {approvalActorOptions.map((kind) => (
                                      <option key={kind} value={kind}>
                                        {formatApprovalActorLabel(kind)}
                                      </option>
                                    ))}
                                  </select>
                                  <Button
                                    size="small"
                                    onClick={() =>
                                      handleTaskAction(() =>
                                        approveWorkflowTask(
                                          record.task.id,
                                          buildApprovalActor(record, approvalActorKind),
                                          approvalScope ? `Approved ${approvalScope} gate.` : undefined
                                        )
                                      )
                                    }
                                  >
                                    Approve
                                  </Button>
                                  <Button
                                    tone="secondary"
                                    size="small"
                                    onClick={() =>
                                      handleTaskAction(() =>
                                        rejectWorkflowTask(
                                          record.task.id,
                                          buildApprovalActor(record, approvalActorKind),
                                          approvalScope ? `Revision requested for ${approvalScope} gate.` : 'Needs revision'
                                        )
                                      )
                                    }
                                  >
                                    Request revision
                                  </Button>
                                </>
                              ) : null}
                              {retryableStates.has(record.state) ? (
                                <Button size="small" onClick={() => handleTaskAction(() => retryWorkflowTask(record.task.id))}>
                                  Retry
                                </Button>
                              ) : null}
                              <Button tone="secondary" size="small" onClick={() => setActiveTab('environments')}>
                                Open env
                              </Button>
                              <select
                                value={claimAgent}
                                onChange={(event) =>
                                  setClaimSelections((current) => ({ ...current, [record.task.id]: event.target.value }))
                                }
                              >
                                {agents.map((agent) => (
                                  <option key={agent.id} value={agent.id}>
                                    {agent.name}
                                  </option>
                                ))}
                              </select>
                              <Button
                                tone="secondary"
                                size="small"
                                disabled={!claimAgent}
                                onClick={() => handleTaskAction(() => claimWorkflowTask(record.task.id, claimAgent))}
                              >
                                Claim
                              </Button>
                              {record.state === 'running' ? (
                                <Button tone="danger" size="small" onClick={() => handleTaskAction(() => cancelTask(record.task.id))}>
                                  Cancel
                                </Button>
                              ) : null}
                            </div>
                          </div>
                        );
                      })}
                    </div>

                    <FormGroup label="Team message">
                      <textarea
                        value={messageDrafts[run.id] ?? ''}
                        onChange={(event) => setMessageDrafts((current) => ({ ...current, [run.id]: event.target.value }))}
                        placeholder="Share a blocker, handoff, or review note..."
                        rows={2}
                      />
                    </FormGroup>
                    <div className="modal-actions">
                      <Button tone="secondary" onClick={() => handleSendMessage(run.id)}>
                        Send message
                      </Button>
                    </div>
                    {run.messages.length > 0 ? (
                      <div className="task-description">Latest: {run.messages[run.messages.length - 1]?.content}</div>
                    ) : null}
                  </div>
                ))}
              </div>
            )}
          </div>
        ) : (
          <div className="workflows-section">
            <h3>Execution Environments ({sortedEnvironments.length})</h3>
            {sortedEnvironments.length === 0 ? (
              <p className="empty-state">No persisted environments yet.</p>
            ) : (
              <div className="task-list">
                {sortedEnvironments.map((environment: EnvironmentRecord) => (
                  <div key={environment.id} className="task-card">
                    <div className="task-header">
                      <span className="task-id">{environment.id}</span>
                      <span className="task-priority">{environment.state}</span>
                    </div>
                    <div className="task-footer">
                      <span>{environment.spec.execution_mode}</span>
                      <span>{environment.health}</span>
                    </div>
                    <div className="task-description">Path: {environment.prepared_path}</div>
                    <div className="task-footer">
                      <span>Run: {environment.spec.run_id}</span>
                      <span>Task: {environment.spec.task_id}</span>
                    </div>
                    {environment.recovery_action ? (
                      <div className="task-description">
                        Recovery: {environment.recovery_status} · {environment.recovery_action}
                      </div>
                    ) : null}
                    {environment.failure ? (
                      <div className="task-description">Failure: {environment.failure.message}</div>
                    ) : null}
                    {environment.cleanup_result ? (
                      <div className="task-description">Cleanup: {environment.cleanup_result.summary}</div>
                    ) : null}
                    <div className="modal-actions">
                      <Button
                        size="small"
                        onClick={() => handleEnvironmentAction(() => retryWorkflowEnvironment(environment.id).then(() => undefined))}
                      >
                        Retry prep
                      </Button>
                      <Button
                        tone="secondary"
                        size="small"
                        onClick={() => handleEnvironmentAction(() => cleanupWorkflowEnvironment(environment.id, true).then(() => undefined))}
                      >
                        Cleanup
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {showNewTask ? (
        <div className="modal-overlay" onClick={() => setShowNewTask(false)}>
          <div className="modal-content" onClick={(event) => event.stopPropagation()}>
            <h3>Create Workflow Task</h3>
            <FormGroup label="Task objective">
              <textarea
                value={newTask.description}
                onChange={(event) => setNewTask((current) => ({ ...current, description: event.target.value }))}
                placeholder="Describe the task..."
                rows={3}
              />
            </FormGroup>

            <FormGroup label="Assigned agent">
              <select value={newTask.agentId} onChange={(event) => setNewTask((current) => ({ ...current, agentId: event.target.value }))}>
                <option value="">Create or use default agent id</option>
                {agents.map((agent) => (
                  <option key={agent.id} value={agent.id}>{agent.name}</option>
                ))}
              </select>
            </FormGroup>

            <FormGroup label="Role">
              <select value={newTask.role} onChange={(event) => setNewTask((current) => ({ ...current, role: event.target.value }))}>
                <option value="implementer">Implementer</option>
                <option value="researcher">Researcher</option>
                <option value="reviewer">Reviewer</option>
                <option value="tester">Tester</option>
                <option value="security_reviewer">Security reviewer</option>
              </select>
            </FormGroup>

            <FormGroup label="Execution mode">
              <select
                value={newTask.executionMode}
                onChange={(event) => setNewTask((current) => ({ ...current, executionMode: event.target.value }))}
              >
                <option value="shared_workspace">Shared workspace</option>
                <option value="isolated_workspace">Isolated workspace</option>
                <option value="git_worktree">Git worktree scaffold</option>
                <option value="remote">Remote</option>
              </select>
            </FormGroup>

            <FormGroup label="Priority (1-10)">
              <input
                type="range"
                min="1"
                max="10"
                value={newTask.priority}
                onChange={(event) => setNewTask((current) => ({ ...current, priority: parseInt(event.target.value, 10) }))}
              />
              <span>{newTask.priority}</span>
            </FormGroup>

            <label><input type="checkbox" checked={newTask.approvalRequired} onChange={(event) => setNewTask((current) => ({ ...current, approvalRequired: event.target.checked }))} /> Require approval before execution</label>
            <label><input type="checkbox" checked={newTask.reviewerRequired} onChange={(event) => setNewTask((current) => ({ ...current, reviewerRequired: event.target.checked }))} /> Require review gate</label>
            <label><input type="checkbox" checked={newTask.testRequired} onChange={(event) => setNewTask((current) => ({ ...current, testRequired: event.target.checked }))} /> Require test gate</label>
            <label><input type="checkbox" checked={newTask.planningOnly} onChange={(event) => setNewTask((current) => ({ ...current, planningOnly: event.target.checked }))} /> Planning only</label>

            <div className="modal-actions">
              <Button tone="secondary" onClick={() => setShowNewTask(false)}>Cancel</Button>
              <Button onClick={handleCreateTask}>Create task</Button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
};

export default WorkflowsPanel;