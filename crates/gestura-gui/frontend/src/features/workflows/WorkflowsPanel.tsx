import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { Agent, listAgents } from '../../services/tauri/agents';
import {
  ActiveTaskSnapshot,
  acknowledgeBlockedWorkflowTask,
  ApprovalActor,
  ApprovalActorKind,
  ApprovalScope,
  DelegatedCheckpointAction,
  archiveWorkflowThread,
  approveWorkflowTask,
  cancelTask,
  createChildSupervisorRun,
  claimWorkflowTask,
  CollaborationActionStatus,
  CollaborationEscalationLevel,
  DelegatedTask,
  delegateTask,
  EnvironmentRecord,
  cleanupWorkflowEnvironment,
  listActiveTasks,
  listSupervisorRuns,
  listWorkflowEnvironments,
  listWorkflowThreads,
  LocalExecutionRecord,
  OrchestratorToolCall,
  pauseWorkflowTask,
  reconcileWorkflowState,
  rejectWorkflowTask,
  restartWorkflowTaskFromScratch,
  resumeWorkflowTask,
  retryWorkflowEnvironment,
  retryWorkflowTask,
  sendWorkflowCollaborationMessage,
  sendWorkflowMessage,
  spawnSubagent,
  SupervisorRun,
  SupervisorTaskRecord,
  TeamMessageKind,
  TeamThread,
  updateWorkflowThreadAction,
} from '../../services/tauri/workflows';
import { Button } from '../../shared/components/Button';
import { FormGroup } from '../../shared/components/FormGroup';
import { useAsyncState } from '../../shared/hooks/useAsyncState';
import { useInterval } from '../../shared/hooks/useInterval';

const gatedStates = new Set(['pending_approval', 'review_pending', 'test_pending']);
const retryableStates = new Set(['failed', 'blocked', 'cancelled']);

const formatCheckpointLabel = (value: string): string =>
  value
    .split('_')
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join(' ');

const checkpointHasAction = (
  record: SupervisorTaskRecord,
  action: DelegatedCheckpointAction,
): boolean => record.checkpoint?.available_actions.includes(action) ?? false;

const summarizeTask = (task: SupervisorTaskRecord | DelegatedTask | ActiveTaskSnapshot): string => {
  const value = 'task' in task ? task.task.name ?? task.task.prompt : task.name ?? task.prompt;
  return value.length > 120 ? `${value.slice(0, 117)}...` : value;
};

const summarizeLocalExecution = (localExecution?: LocalExecutionRecord | null): string[] => {
  const progress = localExecution?.progress;
  if (!progress) {
    return [];
  }

  const lines = [
    [
      `Local: ${progress.phase}`,
      progress.stage ? formatCheckpointLabel(progress.stage) : null,
      progress.percent != null ? `${progress.percent}%` : null,
      progress.iteration > 0 ? `Iteration ${progress.iteration}` : null,
    ]
      .filter(Boolean)
      .join(' · '),
  ];

  if (progress.waiting_reason || progress.message) {
    lines.push(
      [
        progress.waiting_reason ? `Waiting: ${formatCheckpointLabel(progress.waiting_reason)}` : null,
        progress.message ?? null,
      ]
        .filter(Boolean)
        .join(' · ')
    );
  }

  const toolSummary = [
    progress.current_tool_name ? `Current tool: ${progress.current_tool_name}` : null,
    progress.last_completed_tool_name
      ? `Last tool: ${progress.last_completed_tool_name}${progress.last_completed_tool_duration_ms != null ? ` (${progress.last_completed_tool_duration_ms}ms)` : ''}`
      : null,
    progress.completed_tool_call_count > 0 ? `Completed calls: ${progress.completed_tool_call_count}` : null,
  ]
    .filter(Boolean)
    .join(' · ');
  if (toolSummary) {
    lines.push(toolSummary);
  }

  if (progress.has_partial_content || progress.has_partial_thinking) {
    lines.push(
      [
        progress.has_partial_content ? `Partial response: ${progress.partial_content_chars} chars` : null,
        progress.has_partial_thinking ? `Partial thinking: ${progress.partial_thinking_chars} chars` : null,
      ]
        .filter(Boolean)
        .join(' · ')
    );
  }

  return lines;
};

const summarizeRemoteExecution = (
  remoteExecution?: SupervisorTaskRecord['remote_execution'] | ActiveTaskSnapshot['remote_execution'],
): string[] => {
  if (!remoteExecution) {
    return [];
  }

  const lines = [[`Remote: ${remoteExecution.status}`, remoteExecution.status_reason ?? null].filter(Boolean).join(' · ')];
  if (remoteExecution.progress) {
    lines.push(
      [
        remoteExecution.progress.percent != null ? `${remoteExecution.progress.percent}%` : null,
        remoteExecution.progress.stage ? remoteExecution.progress.stage : null,
        remoteExecution.progress.message ?? null,
      ]
        .filter(Boolean)
        .join(' · ')
    );
  }

  return lines.filter((line) => line.length > 0);
};

const summarizeToolTrace = (toolCalls?: OrchestratorToolCall[] | null): string | null => {
  if (!toolCalls?.length) {
    return null;
  }

  return toolCalls
    .slice(0, 3)
    .map((toolCall) => `${toolCall.tool_name} ${toolCall.success ? '✓' : '✗'}${toolCall.duration_ms > 0 ? ` (${toolCall.duration_ms}ms)` : ''}`)
    .join(' • ');
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

const collectRunMessages = (run: SupervisorRun) =>
  [...run.messages, ...run.tasks.flatMap((task) => task.messages ?? [])].sort((left, right) =>
    left.created_at.localeCompare(right.created_at)
  );

const summarizeCollaboration = (run: SupervisorRun, threads: TeamThread[] = []) => {
  const messages = collectRunMessages(run);
  const threadIds = new Set(messages.map((message) => message.thread_id ?? message.id));
  const actionRequiredCount = messages.filter((message) => {
    const status = message.action_request?.status;
    return status === 'open' || status === 'acknowledged';
  }).length;
  const latestThreadMessage = threads[0]?.messages[threads[0].messages.length - 1] ?? null;
  return {
    latestMessage: latestThreadMessage ?? messages[messages.length - 1] ?? null,
    threadCount: threads.length > 0 ? threads.length : threadIds.size,
    actionRequiredCount: threads.length > 0 ? threads.filter((thread) => thread.requires_attention).length : actionRequiredCount,
  };
};

const collaborationRequestKindForMessageKind = (kind: TeamMessageKind) => {
  switch (kind) {
    case 'clarification':
      return 'clarification' as const;
    case 'blocker':
      return 'blocker_escalation' as const;
    case 'handoff':
      return 'handoff' as const;
    case 'review_request':
      return 'review_request' as const;
    case 'approval_request':
      return 'approval_request' as const;
    case 'test_validation_request':
      return 'test_validation_request' as const;
    default:
      return null;
  }
};

const defaultRequestedRolesForKind = (kind: TeamMessageKind) => {
  switch (kind) {
    case 'review_request':
      return ['reviewer'];
    case 'approval_request':
      return ['supervisor'];
    case 'test_validation_request':
      return ['tester'];
    default:
      return [];
  }
};

const defaultThreadActionNote = (status: CollaborationActionStatus) => {
  switch (status) {
    case 'acknowledged':
      return 'Acknowledged from the workflows panel.';
    case 'resolved':
      return 'Resolved from the workflows panel.';
    case 'needs_revision':
      return 'Revision requested from the workflows panel.';
    case 'cancelled':
      return 'Cancelled from the workflows panel.';
    default:
      return 'Updated from the workflows panel.';
  }
};

const buildDefaultChildRunDraft = (run: SupervisorRun) => ({
  name: '',
  objective: '',
  leadAgentId: '',
  approvalRequired: run.inherited_policy?.approval_required ?? false,
  reviewerRequired: run.inherited_policy?.reviewer_required ?? false,
  testRequired: run.inherited_policy?.test_required ?? false,
  executionMode: (run.inherited_policy?.execution_mode ?? 'shared_workspace') as DelegatedTask['execution_mode'],
  memoryTags: (run.inherited_policy?.memory_tags ?? []).join(', '),
  constraintNotes: (run.inherited_policy?.constraint_notes ?? []).join('\n'),
});

const splitCommaSeparated = (value: string) =>
  value
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean);

const WorkflowsPanel: React.FC = () => {
  const [activeTab, setActiveTab] = useState<'runs' | 'environments'>('runs');
  const [showNewTask, setShowNewTask] = useState(false);
  const [messageDrafts, setMessageDrafts] = useState<Record<string, string>>({});
  const [messageKinds, setMessageKinds] = useState<Record<string, TeamMessageKind>>({});
  const [messageTaskSelections, setMessageTaskSelections] = useState<Record<string, string>>({});
  const [replyTargets, setReplyTargets] = useState<Record<string, string>>({});
  const [escalationSelections, setEscalationSelections] = useState<Record<string, CollaborationEscalationLevel>>({});
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
  const [childRunDrafts, setChildRunDrafts] = useState<Record<string, ReturnType<typeof buildDefaultChildRunDraft>>>({});
  const [expandedChildForms, setExpandedChildForms] = useState<Record<string, boolean>>({});

  const workflowsState = useAsyncState(
    async () => {
      const [tasks, runs, environments, agentRes] = await Promise.all([
        listActiveTasks(),
        listSupervisorRuns(),
        listWorkflowEnvironments(),
        listAgents(),
      ]);
      const threadEntries = await Promise.all(
        runs.map(async (run) => [run.id, await listWorkflowThreads(run.id)] as const)
      );
      return {
        activeTasks: tasks,
        runs,
        environments,
        agents: (Array.isArray(agentRes?.agents) ? agentRes.agents : []) as Agent[],
        threadsByRun: Object.fromEntries(threadEntries) as Record<string, TeamThread[]>,
      };
    },
    { errorMessage: 'Failed to load workflow data:' }
  );

  const activeTasks = workflowsState.data?.activeTasks ?? [];
  const runs = workflowsState.data?.runs ?? [];
  const environments = workflowsState.data?.environments ?? [];
  const agents = workflowsState.data?.agents ?? [];
  const threadsByRun = workflowsState.data?.threadsByRun ?? {};
  const refreshTimerRef = useRef<number | null>(null);
  const refreshQueuedRef = useRef(false);
  const refreshInFlightRef = useRef(false);

  const sortedRuns = useMemo(
    () => [...runs].sort((left, right) => right.updated_at.localeCompare(left.updated_at)),
    [runs]
  );
  const childRunsByParent = useMemo(() => {
    return sortedRuns.reduce<Record<string, SupervisorRun[]>>((acc, run) => {
      const parentRunId = run.parent_run?.parent_run_id;
      if (parentRunId) {
        acc[parentRunId] = [...(acc[parentRunId] ?? []), run];
      }
      return acc;
    }, {});
  }, [sortedRuns]);
  const hierarchicalRuns = useMemo(() => {
    const roots = sortedRuns.filter((run) => !run.parent_run);
    const children = sortedRuns.filter((run) => Boolean(run.parent_run));
    const ordered = roots.flatMap((root) => [root, ...(childRunsByParent[root.id] ?? [])]);
    const seen = new Set(ordered.map((run) => run.id));
    for (const orphanChild of children) {
      if (!seen.has(orphanChild.id)) {
        ordered.push(orphanChild);
      }
    }
    return ordered;
  }, [childRunsByParent, sortedRuns]);
  const sortedEnvironments = useMemo(
    () => [...environments].sort((left, right) => right.updated_at.localeCompare(left.updated_at)),
    [environments]
  );

  const runRefresh = useCallback(async () => {
    if (refreshInFlightRef.current) {
      refreshQueuedRef.current = true;
      return;
    }

    refreshInFlightRef.current = true;
    try {
      await workflowsState.reload({ showLoading: false });
    } finally {
      refreshInFlightRef.current = false;
      if (refreshQueuedRef.current) {
        refreshQueuedRef.current = false;
        window.setTimeout(() => {
          void runRefresh();
        }, 100);
      }
    }
  }, [workflowsState.reload]);

  const scheduleRefresh = useCallback((delayMs = 150) => {
    if (refreshTimerRef.current != null) return;
    refreshTimerRef.current = window.setTimeout(() => {
      refreshTimerRef.current = null;
      void runRefresh();
    }, delayMs);
  }, [runRefresh]);

  useInterval(() => {
    scheduleRefresh(0);
  }, 4000);

  useEffect(() => {
    let unlisten: Array<() => void> = [];

    const bindWorkflowEvents = async () => {
      try {
        const appWindow = getCurrentWebviewWindow();
        unlisten = await Promise.all(
          [
            'orchestrator-run-updated',
            'orchestrator-team-message',
            'orchestrator-team-thread-updated',
            'orchestrator-environment-updated',
          ].map((eventName) => appWindow.listen(eventName, () => scheduleRefresh()))
        );
      } catch (error) {
        console.debug('Workflow event listener unavailable:', error);
      }
    };

    void bindWorkflowEvents();
    return () => {
      if (refreshTimerRef.current != null) {
        window.clearTimeout(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
      unlisten.forEach((dispose) => dispose());
    };
  }, [scheduleRefresh]);

  const refresh = () => runRefresh();

  const childRunDraftFor = (run: SupervisorRun) => childRunDrafts[run.id] ?? buildDefaultChildRunDraft(run);

  const updateChildRunDraft = (
    run: SupervisorRun,
    patch: Partial<ReturnType<typeof buildDefaultChildRunDraft>>
  ) => {
    setChildRunDrafts((current) => ({
      ...current,
      [run.id]: {
        ...childRunDraftFor(run),
        ...patch,
      },
    }));
  };

  const handleCreateChildRun = async (run: SupervisorRun) => {
    const draft = childRunDraftFor(run);
    if (!draft.objective.trim() || !draft.leadAgentId.trim()) return;
    try {
      await createChildSupervisorRun({
        parent_run_id: run.id,
        name: draft.name.trim() || undefined,
        objective: draft.objective.trim(),
        lead_agent_id: draft.leadAgentId.trim(),
        session_id: run.session_id ?? undefined,
        workspace_dir: run.workspace_dir ?? undefined,
        approval_required: draft.approvalRequired,
        reviewer_required: draft.reviewerRequired,
        test_required: draft.testRequired,
        execution_mode: draft.executionMode,
        memory_tags: splitCommaSeparated(draft.memoryTags),
        constraint_notes: draft.constraintNotes
          .split('\n')
          .map((entry) => entry.trim())
          .filter(Boolean),
      });
      setChildRunDrafts((current) => ({ ...current, [run.id]: buildDefaultChildRunDraft(run) }));
      setExpandedChildForms((current) => ({ ...current, [run.id]: false }));
      await refresh();
    } catch (error) {
      console.error('Failed to create child supervisor run:', error);
    }
  };

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
      const kind = messageKinds[runId] ?? 'status_update';
      const replyTargetId = replyTargets[runId];
      const replyThread = threadsByRun[runId]?.find((thread) => thread.id === replyTargetId);
      const requestKind = collaborationRequestKindForMessageKind(kind);
      if (kind === 'status_update' && !replyThread) {
        await sendWorkflowMessage(runId, content, kind, messageTaskSelections[runId] || undefined);
      } else {
        await sendWorkflowCollaborationMessage(runId, {
          task_id: replyThread?.task_id ?? messageTaskSelections[runId] ?? undefined,
          kind,
          content,
          thread_id: replyThread?.id,
          reply_to_message_id: replyThread?.messages[replyThread.messages.length - 1]?.id,
          action_request: requestKind
            ? {
              kind: requestKind,
              requested_for_roles: defaultRequestedRolesForKind(kind),
              note: content,
            }
            : undefined,
          escalation:
            kind === 'blocker'
              ? {
                level: escalationSelections[runId] ?? 'warning',
                escalated_by_agent_id: 'workflow-panel',
                note: 'Escalated from the workflows panel',
              }
              : undefined,
        });
      }
      setMessageDrafts((current) => ({ ...current, [runId]: '' }));
      setReplyTargets((current) => ({ ...current, [runId]: '' }));
      await refresh();
    } catch (error) {
      console.error('Failed to send workflow message:', error);
    }
  };

  const handleThreadAction = async (
    runId: string,
    threadId: string,
    status: CollaborationActionStatus
  ) => {
    try {
      await updateWorkflowThreadAction(runId, threadId, status, 'workflow-panel', defaultThreadActionNote(status));
      await refresh();
    } catch (error) {
      console.error('Failed to update workflow thread:', error);
    }
  };

  const handleArchiveThread = async (runId: string, threadId: string) => {
    try {
      await archiveWorkflowThread(runId, threadId, 'workflow-panel', 'Archived from the workflows panel.');
      setReplyTargets((current) => ({ ...current, [runId]: current[runId] === threadId ? '' : current[runId] }));
      await refresh();
    } catch (error) {
      console.error('Failed to archive workflow thread:', error);
    }
  };

  const handleEscalateThread = async (run: SupervisorRun, thread: TeamThread) => {
    try {
      const latestMessage = thread.messages[thread.messages.length - 1];
      await sendWorkflowCollaborationMessage(run.id, {
        task_id: thread.task_id ?? undefined,
        kind: 'blocker',
        content: `Escalated thread ${thread.id} for operator attention.`,
        thread_id: thread.id,
        reply_to_message_id: latestMessage?.id,
        escalation: {
          level: 'warning',
          escalated_by_agent_id: 'workflow-panel',
          note: 'Escalated from the workflows panel',
        },
        action_request: {
          kind: 'blocker_escalation',
          requested_for_roles: ['supervisor'],
          note: 'Operator escalation requested from the workflows panel.',
        },
      });
      await refresh();
    } catch (error) {
      console.error('Failed to escalate workflow thread:', error);
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
          Runs ({hierarchicalRuns.length})
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
                <div key={task.task.id} className="task-card">
                  <div className="task-header">
                    <span className="task-id">{task.task.id}</span>
                    <span className="task-priority">{task.state}</span>
                  </div>
                  <div className="task-description">{summarizeTask(task)}</div>
                  <div className="task-description">
                    Agent: {task.task.agent_id} · Mode: {task.task.execution_mode} · Priority: P{task.task.priority}
                  </div>
                  {summarizeLocalExecution(task.local_execution).map((line) => (
                    <div key={`${task.task.id}-${line}`} className="task-description">
                      {line}
                    </div>
                  ))}
                  {summarizeRemoteExecution(task.remote_execution).map((line) => (
                    <div key={`${task.task.id}-remote-${line}`} className="task-description">
                      {line}
                    </div>
                  ))}
                  {task.blocked_reasons.length > 0 ? (
                    <div className="task-description">Blocked by: {task.blocked_reasons.join('; ')}</div>
                  ) : null}
                  {task.checkpoint ? (
                    <div className="task-description">
                      Checkpoint: {formatCheckpointLabel(task.checkpoint.stage)} · {task.checkpoint.safe_boundary_label} · Resume:{' '}
                      {task.checkpoint.has_resume_state ? 'available' : 'not captured'}
                    </div>
                  ) : null}
                  <div className="task-footer">
                    <span className="task-agent">Run: {task.task.run_id ?? 'standalone'}</span>
                    {task.task.execution_mode !== 'remote' ? (
                      <Button size="small" onClick={() => handleTaskAction(() => pauseWorkflowTask(task.task.id))}>
                        Pause
                      </Button>
                    ) : null}
                    <Button tone="danger" size="small" onClick={() => handleTaskAction(() => cancelTask(task.task.id))}>
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
            <h3>Supervisor Runs ({hierarchicalRuns.length})</h3>
            {hierarchicalRuns.length === 0 ? (
              <p className="empty-state">No supervisor runs yet. Create a workflow task to start one.</p>
            ) : (
              <div className="task-list">
                {hierarchicalRuns.map((run: SupervisorRun) => {
                  const collaborationThreads = threadsByRun[run.id] ?? [];
                  const collaboration = summarizeCollaboration(run, collaborationThreads);
                  const selectedMessageKind = messageKinds[run.id] ?? 'status_update';
                  const selectedReplyThreadId = replyTargets[run.id] ?? '';
                  const isChildRun = Boolean(run.parent_run);
                  const childDraft = childRunDraftFor(run);
                  return (
                    <div
                      key={run.id}
                      className="task-card"
                      style={isChildRun ? { marginLeft: 24, borderLeft: '3px solid rgba(91, 141, 239, 0.65)' } : undefined}
                    >
                      <div className="task-header">
                        <span className="task-id">{run.name ?? run.id}</span>
                        <span className="task-priority">{run.hierarchy_summary?.rollup_status ?? run.status}</span>
                      </div>
                      <div className="task-description">
                        Run: {run.id}
                        {run.parent_run ? ` • Child of ${run.parent_run.parent_run_id}` : ' • Root workflow'}
                        {run.parent_run?.objective ? ` • Objective: ${run.parent_run.objective}` : ''}
                      </div>
                      <div className="task-footer">
                        <span>
                          {run.task_summary.total || run.tasks.length} tasks
                          {run.child_runs.length > 0 ? ` • ${run.child_runs.length} child runs` : ''}
                        </span>
                        <span>Updated {new Date(run.updated_at).toLocaleString()}</span>
                      </div>
                      {run.hierarchy_summary ? (
                        <div className="task-description">
                          Depth {run.hierarchy_summary.depth}/{run.hierarchy_summary.max_depth + 1}
                          {run.hierarchy_summary.descendant_task_count > 0
                            ? ` • Descendant tasks ${run.hierarchy_summary.descendant_task_count}`
                            : ''}
                          {run.hierarchy_summary.action_required_child_count > 0
                            ? ` • Child attention ${run.hierarchy_summary.action_required_child_count}`
                            : ''}
                        </div>
                      ) : null}
                      {run.child_runs.length > 0 ? (
                        <div className="task-description">
                          Child runs:{' '}
                          {run.child_runs
                            .map((child) => `${child.name ?? child.run_id} (${child.status}, ${child.task_summary.total} tasks)`)
                            .join(' • ')}
                        </div>
                      ) : null}
                      {!isChildRun && run.hierarchy_depth < run.max_hierarchy_depth ? (
                        <>
                          <div className="modal-actions" style={{ marginTop: 8 }}>
                            <Button
                              tone="secondary"
                              size="small"
                              onClick={() =>
                                setExpandedChildForms((current) => ({ ...current, [run.id]: !current[run.id] }))
                              }
                            >
                              {expandedChildForms[run.id] ? 'Hide child supervisor form' : 'Create child supervisor'}
                            </Button>
                          </div>
                          {expandedChildForms[run.id] ? (
                            <div className="task-card" style={{ marginTop: 12 }}>
                              <FormGroup label="Child supervisor objective">
                                <textarea
                                  value={childDraft.objective}
                                  onChange={(event) => updateChildRunDraft(run, { objective: event.target.value })}
                                  placeholder="Delegate a bounded sub-supervisor mission..."
                                  rows={2}
                                />
                              </FormGroup>
                              <FormGroup label="Lead agent id">
                                <input
                                  value={childDraft.leadAgentId}
                                  onChange={(event) => updateChildRunDraft(run, { leadAgentId: event.target.value })}
                                  placeholder="supervisor-subteam-a"
                                />
                              </FormGroup>
                              <FormGroup label="Display name (optional)">
                                <input
                                  value={childDraft.name}
                                  onChange={(event) => updateChildRunDraft(run, { name: event.target.value })}
                                  placeholder="Frontend delivery pod"
                                />
                              </FormGroup>
                              <div className="modal-actions" style={{ marginBottom: 8 }}>
                                <label><input type="checkbox" checked={childDraft.approvalRequired} onChange={(event) => updateChildRunDraft(run, { approvalRequired: event.target.checked })} /> Approval gate</label>
                                <label><input type="checkbox" checked={childDraft.reviewerRequired} onChange={(event) => updateChildRunDraft(run, { reviewerRequired: event.target.checked })} /> Review gate</label>
                                <label><input type="checkbox" checked={childDraft.testRequired} onChange={(event) => updateChildRunDraft(run, { testRequired: event.target.checked })} /> Test gate</label>
                              </div>
                              <div className="modal-actions" style={{ marginBottom: 8 }}>
                                <select
                                  value={childDraft.executionMode}
                                  onChange={(event) =>
                                    updateChildRunDraft(run, { executionMode: event.target.value as DelegatedTask['execution_mode'] })
                                  }
                                >
                                  <option value="shared_workspace">Shared workspace</option>
                                  <option value="isolated_workspace">Isolated workspace</option>
                                  <option value="git_worktree">Git worktree</option>
                                  <option value="remote">Remote</option>
                                </select>
                              </div>
                              <FormGroup label="Memory tags (comma separated)">
                                <input
                                  value={childDraft.memoryTags}
                                  onChange={(event) => updateChildRunDraft(run, { memoryTags: event.target.value })}
                                  placeholder="workflow-panel, child-supervision"
                                />
                              </FormGroup>
                              <FormGroup label="Constraint notes (one per line)">
                                <textarea
                                  value={childDraft.constraintNotes}
                                  onChange={(event) => updateChildRunDraft(run, { constraintNotes: event.target.value })}
                                  placeholder="Stay within frontend scope\nEscalate blocking API changes"
                                  rows={2}
                                />
                              </FormGroup>
                              <div className="modal-actions">
                                <Button size="small" onClick={() => handleCreateChildRun(run)}>
                                  Create child run
                                </Button>
                              </div>
                            </div>
                          ) : null}
                        </>
                      ) : null}

                      <div className="task-list">
                        {run.tasks.map((record) => {
                          const claimAgent = claimSelections[record.task.id] ?? record.task.agent_id;
                          const approvalScope = approvalScopeForState(record.state);
                          const approvalActorOptions = approvalActorOptionsForRecord(record);
                          const approvalActorKind =
                            approvalActorSelections[record.task.id] ?? approvalActorOptions[0] ?? defaultApprovalActorKind(record.state);
                          const latestDecision = record.approval.decisions[record.approval.decisions.length - 1];
                          const checkpoint = record.checkpoint;
                          const canResumeFromCheckpoint = checkpointHasAction(record, 'resume_from_checkpoint');
                          const canRestartFromScratch = checkpointHasAction(record, 'restart_from_scratch');
                          const canAcknowledgeBlocked = checkpointHasAction(record, 'acknowledge_blocked');
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
                              {summarizeLocalExecution(record.local_execution).map((line) => (
                                <div key={`${record.task.id}-${line}`} className="task-description">
                                  {line}
                                </div>
                              ))}
                              {summarizeRemoteExecution(record.remote_execution).map((line) => (
                                <div key={`${record.task.id}-remote-${line}`} className="task-description">
                                  {line}
                                </div>
                              ))}
                              {checkpoint ? (
                                <>
                                  <div className="task-description">
                                    Checkpoint: {formatCheckpointLabel(checkpoint.stage)} · {formatCheckpointLabel(checkpoint.resume_disposition)} · {checkpoint.safe_boundary_label}
                                  </div>
                                  <div className="task-description">
                                    Replay safety: {formatCheckpointLabel(checkpoint.replay_safety)} · Tool calls: {checkpoint.completed_tool_call_count} · Resume state:{' '}
                                    {checkpoint.has_resume_state ? 'available' : 'not captured'}
                                  </div>
                                  {checkpoint.note ? <div className="task-description">Checkpoint note: {checkpoint.note}</div> : null}
                                </>
                              ) : null}
                              {record.remote_execution?.provenance?.caller_name || record.remote_execution?.provenance?.caller_agent_id ? (
                                <div className="task-description">
                                  Provenance: {record.remote_execution?.provenance?.caller_name ?? record.remote_execution?.provenance?.caller_agent_id}
                                </div>
                              ) : null}
                              {record.remote_execution?.artifacts.length ? (
                                <div className="task-description">
                                  Remote artifacts: {record.remote_execution.artifacts.map((artifact) => artifact.name).join(', ')}
                                </div>
                              ) : null}
                              {record.remote_execution?.compatibility.warnings.length ? (
                                <div className="task-description">
                                  Compatibility: {record.remote_execution.compatibility.warnings.join('; ')}
                                </div>
                              ) : null}
                              {summarizeToolTrace(record.result?.tool_calls) ? (
                                <div className="task-description">Tool trace: {summarizeToolTrace(record.result?.tool_calls)}</div>
                              ) : null}
                              {record.result?.artifacts?.length ? (
                                <div className="task-description">
                                  Result artifacts: {record.result.artifacts.map((artifact) => artifact.name).join(', ')}
                                </div>
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
                                {canResumeFromCheckpoint ? (
                                  <Button size="small" onClick={() => handleTaskAction(() => resumeWorkflowTask(record.task.id))}>
                                    Resume
                                  </Button>
                                ) : null}
                                {canRestartFromScratch ? (
                                  <Button
                                    tone="secondary"
                                    size="small"
                                    onClick={() => handleTaskAction(() => restartWorkflowTaskFromScratch(record.task.id))}
                                  >
                                    Restart from scratch
                                  </Button>
                                ) : null}
                                {canAcknowledgeBlocked ? (
                                  <Button
                                    tone="secondary"
                                    size="small"
                                    onClick={() =>
                                      handleTaskAction(() =>
                                        acknowledgeBlockedWorkflowTask(record.task.id, 'Acknowledged in workflow panel.')
                                      )
                                    }
                                  >
                                    Acknowledge blocked
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
                        <div className="modal-actions" style={{ marginBottom: 8 }}>
                          <select
                            value={selectedMessageKind}
                            onChange={(event) =>
                              setMessageKinds((current) => ({
                                ...current,
                                [run.id]: event.target.value as TeamMessageKind,
                              }))
                            }
                          >
                            <option value="status_update">Status update</option>
                            <option value="clarification">Clarification</option>
                            <option value="blocker">Blocker</option>
                            <option value="handoff">Handoff</option>
                            <option value="review_request">Review request</option>
                            <option value="approval_request">Approval request</option>
                            <option value="test_validation_request">Test validation request</option>
                          </select>
                          <select
                            value={messageTaskSelections[run.id] ?? ''}
                            onChange={(event) =>
                              setMessageTaskSelections((current) => ({ ...current, [run.id]: event.target.value }))
                            }
                          >
                            <option value="">Run-level thread</option>
                            {run.tasks.map((record) => (
                              <option key={record.task.id} value={record.task.id}>
                                {record.task.id}
                              </option>
                            ))}
                          </select>
                          {selectedMessageKind === 'blocker' ? (
                            <select
                              value={escalationSelections[run.id] ?? 'warning'}
                              onChange={(event) =>
                                setEscalationSelections((current) => ({
                                  ...current,
                                  [run.id]: event.target.value as CollaborationEscalationLevel,
                                }))
                              }
                            >
                              <option value="info">Info</option>
                              <option value="warning">Warning</option>
                              <option value="critical">Critical</option>
                            </select>
                          ) : null}
                        </div>
                        {selectedReplyThreadId ? (
                          <div className="task-description" style={{ marginBottom: 8 }}>
                            Replying to thread {selectedReplyThreadId}
                            <Button tone="secondary" size="small" onClick={() => setReplyTargets((current) => ({ ...current, [run.id]: '' }))}>
                              Clear
                            </Button>
                          </div>
                        ) : null}
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
                      {(run.shared_cognition ?? []).length > 0 ? (
                        <>
                          <div className="task-description">
                            Shared cognition: {(run.shared_cognition ?? []).length}
                            {(run.shared_cognition ?? [])[(run.shared_cognition ?? []).length - 1]
                              ? ` • Latest ${(run.shared_cognition ?? [])[(run.shared_cognition ?? []).length - 1]?.kind}: ${(run.shared_cognition ?? [])[(run.shared_cognition ?? []).length - 1]?.summary}`
                              : ''}
                          </div>
                          <div className="task-list">
                            {[...(run.shared_cognition ?? [])]
                              .sort((left, right) => left.created_at.localeCompare(right.created_at))
                              .slice(-4)
                              .reverse()
                              .map((note) => (
                                <div key={note.id} className="task-card">
                                  <div className="task-header">
                                    <span className="task-id">{note.kind}</span>
                                    <span className="task-priority">{Math.round(note.confidence * 100)}%</span>
                                  </div>
                                  <div className="task-description">
                                    {note.summary}
                                    {note.task_id ? ` • Task ${note.task_id}` : ''}
                                    {note.directive_id ? ` • Directive ${note.directive_id}` : ''}
                                  </div>
                                  <div className="task-description">{note.detail}</div>
                                  <div className="task-footer">
                                    <span>{note.sender_agent_id ?? 'workflow-panel'}</span>
                                    <span>{new Date(note.created_at).toLocaleString()}</span>
                                  </div>
                                </div>
                              ))}
                          </div>
                        </>
                      ) : null}
                      {collaboration.threadCount > 0 ? (
                        <>
                          <div className="task-description">
                            Threads: {collaboration.threadCount}
                            {collaboration.actionRequiredCount > 0 ? ` • Action required: ${collaboration.actionRequiredCount}` : ''}
                            {collaboration.latestMessage ? ` • Latest: ${collaboration.latestMessage.content}` : ''}
                          </div>
                          <div className="task-list">
                            {collaborationThreads.map((thread) => {
                              const latestMessage = thread.messages[thread.messages.length - 1];
                              const requestStatus = thread.latest_action_request?.status;
                              return (
                                <div key={thread.id} className="task-card">
                                  <div className="task-header">
                                    <span className="task-id">{thread.id}</span>
                                    <span className="task-priority">{thread.status}</span>
                                  </div>
                                  <div className="task-description">
                                    {thread.kind}
                                    {thread.task_id ? ` • Task ${thread.task_id}` : ''}
                                    {thread.requires_attention ? ' • Needs attention' : ''}
                                    {thread.unread_count > 0 ? ` • Unread ${thread.unread_count}` : ''}
                                  </div>
                                  <div className="task-description">{latestMessage?.content ?? 'No messages yet.'}</div>
                                  <div className="task-footer">
                                    <span>{thread.message_count} messages</span>
                                    <span>{new Date(thread.updated_at).toLocaleString()}</span>
                                  </div>
                                  <div className="modal-actions">
                                    <Button
                                      tone="secondary"
                                      size="small"
                                      onClick={() => setReplyTargets((current) => ({ ...current, [run.id]: thread.id }))}
                                    >
                                      Reply
                                    </Button>
                                    {requestStatus === 'open' ? (
                                      <Button size="small" onClick={() => handleThreadAction(run.id, thread.id, 'acknowledged')}>
                                        Acknowledge
                                      </Button>
                                    ) : null}
                                    {thread.requires_attention || requestStatus === 'acknowledged' ? (
                                      <Button size="small" onClick={() => handleThreadAction(run.id, thread.id, 'resolved')}>
                                        Resolve
                                      </Button>
                                    ) : null}
                                    {thread.requires_attention ? (
                                      <Button
                                        tone="secondary"
                                        size="small"
                                        onClick={() => handleThreadAction(run.id, thread.id, 'needs_revision')}
                                      >
                                        Request revision
                                      </Button>
                                    ) : null}
                                    <Button tone="secondary" size="small" onClick={() => handleEscalateThread(run, thread)}>
                                      Escalate
                                    </Button>
                                    {thread.status === 'resolved' ? (
                                      <Button tone="secondary" size="small" onClick={() => handleArchiveThread(run.id, thread.id)}>
                                        Archive
                                      </Button>
                                    ) : null}
                                  </div>
                                </div>
                              );
                            })}
                          </div>
                        </>
                      ) : null}
                    </div>
                  )
                })}
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