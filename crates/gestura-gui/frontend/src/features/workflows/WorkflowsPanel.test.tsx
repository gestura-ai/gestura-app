import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  ChildSupervisorRunSummary,
  EnvironmentRecord,
  SupervisorInheritancePolicy,
  SupervisorParentRunRef,
  SupervisorRun,
} from '../../services/tauri/workflows';

const workflowMocks = vi.hoisted(() => ({
  archiveWorkflowThread: vi.fn().mockResolvedValue(undefined),
  approveWorkflowTask: vi.fn().mockResolvedValue(undefined),
  cancelTask: vi.fn().mockResolvedValue(undefined),
  claimWorkflowTask: vi.fn().mockResolvedValue(undefined),
  cleanupWorkflowEnvironment: vi.fn().mockResolvedValue(undefined),
  createChildSupervisorRun: vi.fn().mockResolvedValue(undefined),
  delegateTask: vi.fn().mockResolvedValue('task-123'),
  listActiveTasks: vi.fn().mockResolvedValue([]),
  listWorkflowEnvironments: vi.fn().mockResolvedValue([]),
  listWorkflowThreads: vi.fn().mockResolvedValue([
    {
      id: 'thread-1',
      run_id: 'run-1',
      task_id: 'task-1',
      kind: 'approval_request',
      status: 'action_required',
      created_at: '2026-03-10T00:00:00Z',
      updated_at: '2026-03-10T00:00:00Z',
      archived: false,
      unread_count: 1,
      message_count: 1,
      actionable_message_count: 1,
      requires_attention: true,
      participant_agent_ids: ['orchestrator'],
      latest_action_request: {
        id: 'action-1',
        kind: 'approval_request',
        status: 'open',
        requested_by_agent_id: 'orchestrator',
        requested_for_roles: ['supervisor'],
        requested_for_agent_ids: [],
        requested_for_actor_kinds: [],
        approval_scope: 'pre_execution',
        note: 'Awaiting approval',
        resolution_note: null,
        resolved_by_agent_id: null,
        created_at: '2026-03-10T00:00:00Z',
        resolved_at: null,
      },
      latest_result_reference: null,
      artifact_references: [],
      messages: [
        {
          id: 'msg-1',
          run_id: 'run-1',
          task_id: 'task-1',
          kind: 'approval_request',
          sender_agent_id: 'orchestrator',
          recipient_agent_id: null,
          thread_id: 'thread-1',
          reply_to_message_id: null,
          content: 'Task submitted. Awaiting approval.',
          action_request: {
            id: 'action-1',
            kind: 'approval_request',
            status: 'open',
            requested_by_agent_id: 'orchestrator',
            requested_for_roles: ['supervisor'],
            requested_for_agent_ids: [],
            requested_for_actor_kinds: [],
            approval_scope: 'pre_execution',
            note: 'Awaiting approval',
            resolution_note: null,
            resolved_by_agent_id: null,
            created_at: '2026-03-10T00:00:00Z',
            resolved_at: null,
          },
          escalation: null,
          result_reference: null,
          artifact_references: [],
          unread_by_agent_ids: ['workflow-panel'],
          created_at: '2026-03-10T00:00:00Z',
        },
      ],
    },
  ]),
  listSupervisorRuns: vi.fn().mockResolvedValue([]),
  reconcileWorkflowState: vi.fn().mockResolvedValue(undefined),
  rejectWorkflowTask: vi.fn().mockResolvedValue(undefined),
  retryWorkflowEnvironment: vi.fn().mockResolvedValue(undefined),
  retryWorkflowTask: vi.fn().mockResolvedValue(undefined),
  sendWorkflowCollaborationMessage: vi.fn().mockResolvedValue(undefined),
  sendWorkflowMessage: vi.fn().mockResolvedValue(undefined),
  spawnSubagent: vi.fn().mockResolvedValue(undefined),
  updateWorkflowThreadAction: vi.fn().mockResolvedValue(undefined),
}));

const asyncStateMock = vi.hoisted(() => ({
  data: {
    activeTasks: [],
    agents: [{ id: 'agent-1', name: 'Reviewer', status: 'running', role: 'reviewer' }],
    environments: [
      {
        id: 'env-1',
        spec: {
          id: 'env-1',
          execution_mode: 'shared_workspace',
          workspace_root: '.',
          prepared_path: '.',
          run_id: 'run-1',
          task_id: 'task-1',
          agent_id: 'agent-1',
          cleanup_policy: 'keep_always',
          write_access: true,
        },
        state: 'ready',
        health: 'clean',
        prepared_path: '.',
        recovery_status: 'not_required',
        created_at: '2026-03-10T00:00:00Z',
        updated_at: '2026-03-10T00:00:00Z',
      },
    ] as EnvironmentRecord[],
    runs: [
      {
        id: 'run-1',
        name: 'Root workflow',
        parent_run: null,
        child_runs: [],
        hierarchy_depth: 0,
        max_hierarchy_depth: 1,
        inherited_policy: null,
        status: 'waiting',
        task_summary: {
          total: 1,
          queued: 0,
          blocked: 0,
          pending_approval: 1,
          running: 0,
          review_pending: 0,
          test_pending: 0,
          completed: 0,
          failed: 0,
          cancelled: 0,
        },
        hierarchy_summary: {
          depth: 0,
          max_depth: 1,
          child_run_count: 0,
          descendant_task_count: 0,
          action_required_child_count: 0,
          rollup_status: 'waiting',
          requires_attention: false,
          blocked_reasons: [],
        },
        tasks: [
          {
            task: {
              id: 'task-1',
              agent_id: 'agent-1',
              prompt: 'Review the patch',
              required_tools: [],
              priority: 3,
              depends_on: [],
              role: 'reviewer',
              planning_only: false,
              approval_required: true,
              reviewer_required: true,
              test_required: false,
              execution_mode: 'shared_workspace',
              memory_tags: [],
              name: 'Review the patch',
            },
            state: 'pending_approval',
            approval: {
              state: 'pending',
              scope: 'pre_execution',
              active_request: {
                id: 'req-1',
                scope: 'pre_execution',
                requested_by: { kind: 'system', id: 'orchestrator', display_name: 'Orchestrator' },
                requested_at: '2026-03-10T00:00:00Z',
                note: 'Task submitted. Awaiting explicit pre-execution approval.',
              },
              policy: {
                pre_execution: { scope: 'pre_execution', required: true, allowed_deciders: ['supervisor', 'user'] },
                review: { scope: 'review', required: true, allowed_deciders: ['reviewer', 'supervisor'] },
                test_validation: { scope: 'test_validation', required: false, allowed_deciders: ['tester', 'supervisor'] },
              },
              requests: [],
              decisions: [],
            },
            environment_id: 'env-1',
            environment: {
              id: 'env-1',
              execution_mode: 'shared_workspace',
              root_dir: '.',
              write_access: true,
              state: 'ready',
              health: 'clean',
              cleanup_policy: 'keep_always',
              recovery_status: 'not_required',
            },
            attempts: 0,
            blocked_reasons: [],
            messages: [],
            created_at: '2026-03-10T00:00:00Z',
            updated_at: '2026-03-10T00:00:00Z',
          },
        ],
        messages: [],
        created_at: '2026-03-10T00:00:00Z',
        updated_at: '2026-03-10T00:00:00Z',
      },
    ] as SupervisorRun[],
    threadsByRun: {
      'run-1': [
        {
          id: 'thread-1',
          run_id: 'run-1',
          task_id: 'task-1',
          kind: 'approval_request',
          status: 'action_required',
          created_at: '2026-03-10T00:00:00Z',
          updated_at: '2026-03-10T00:00:00Z',
          archived: false,
          unread_count: 1,
          message_count: 1,
          actionable_message_count: 1,
          requires_attention: true,
          participant_agent_ids: ['orchestrator'],
          latest_action_request: {
            id: 'action-1',
            kind: 'approval_request',
            status: 'open',
            requested_by_agent_id: 'orchestrator',
            requested_for_roles: ['supervisor'],
            requested_for_agent_ids: [],
            requested_for_actor_kinds: [],
            approval_scope: 'pre_execution',
            note: 'Awaiting approval',
            resolution_note: null,
            resolved_by_agent_id: null,
            created_at: '2026-03-10T00:00:00Z',
            resolved_at: null,
          },
          latest_result_reference: null,
          artifact_references: [],
          messages: [
            {
              id: 'msg-1',
              run_id: 'run-1',
              task_id: 'task-1',
              kind: 'approval_request',
              sender_agent_id: 'orchestrator',
              recipient_agent_id: null,
              thread_id: 'thread-1',
              reply_to_message_id: null,
              content: 'Task submitted. Awaiting approval.',
              action_request: {
                id: 'action-1',
                kind: 'approval_request',
                status: 'open',
                requested_by_agent_id: 'orchestrator',
                requested_for_roles: ['supervisor'],
                requested_for_agent_ids: [],
                requested_for_actor_kinds: [],
                approval_scope: 'pre_execution',
                note: 'Awaiting approval',
                resolution_note: null,
                resolved_by_agent_id: null,
                created_at: '2026-03-10T00:00:00Z',
                resolved_at: null,
              },
              escalation: null,
              result_reference: null,
              artifact_references: [],
              unread_by_agent_ids: ['workflow-panel'],
              created_at: '2026-03-10T00:00:00Z',
            },
          ],
        },
      ],
    },
  },
  loading: false,
  error: undefined,
  reload: vi.fn().mockResolvedValue(undefined),
  setData: vi.fn(),
}));

vi.mock('../../services/tauri/workflows', () => workflowMocks);
vi.mock('../../services/tauri/agents', () => ({
  listAgents: vi.fn().mockResolvedValue({
    agents: [{ id: 'agent-1', name: 'Reviewer', status: 'running', role: 'reviewer' }],
  }),
}));
vi.mock('../../shared/hooks/useAsyncState', () => ({
  useAsyncState: vi.fn(() => asyncStateMock),
}));
vi.mock('../../shared/hooks/useInterval', () => ({
  useInterval: vi.fn(),
}));
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: vi.fn(() => ({
    listen: vi.fn().mockResolvedValue(() => { }),
  })),
}));

describe('WorkflowsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders supervisor runs and allows approving a gated task', async () => {
    const { default: WorkflowsPanel } = await import('./WorkflowsPanel');
    render(<WorkflowsPanel />);

    await waitFor(() => {
      expect(screen.getByText('Supervisor Runs (1)')).toBeInTheDocument();
    });

    expect(screen.getByText('Review the patch')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));

    await waitFor(() => {
      expect(workflowMocks.approveWorkflowTask).toHaveBeenCalledWith(
        'task-1',
        {
          kind: 'supervisor',
          id: 'workflow-panel:task-1:supervisor',
          display_name: 'Workflow Panel (Supervisor)',
        },
        'Approved pre_execution gate.'
      );
    });
  });

  it('renders collaboration threads and allows acknowledging them', async () => {
    const { default: WorkflowsPanel } = await import('./WorkflowsPanel');
    render(<WorkflowsPanel />);

    await waitFor(() => {
      expect(screen.getAllByRole('button', { name: 'Acknowledge' }).length).toBeGreaterThan(0);
    });

    fireEvent.click(screen.getAllByRole('button', { name: 'Acknowledge' })[0]);

    await waitFor(() => {
      expect(workflowMocks.updateWorkflowThreadAction).toHaveBeenCalledWith(
        'run-1',
        'thread-1',
        'acknowledged',
        'workflow-panel',
        'Acknowledged from the workflows panel.'
      );
    });
  });

  it('renders child supervisor runs and allows creating another child run', async () => {
    const rootRun = asyncStateMock.data.runs[0] as SupervisorRun;
    asyncStateMock.data.runs = [
      {
        ...rootRun,
        child_runs: [
          {
            run_id: 'run-child-1',
            name: 'Frontend pod',
            objective: 'Own frontend delivery',
            lead_agent_id: 'supervisor-alpha',
            status: 'running',
            task_summary: {
              total: 1,
              queued: 0,
              blocked: 0,
              pending_approval: 0,
              running: 1,
              review_pending: 0,
              test_pending: 0,
              completed: 0,
              failed: 0,
              cancelled: 0,
            },
            requires_attention: false,
            blocked_reasons: [],
            created_at: '2026-03-10T00:00:00Z',
            updated_at: '2026-03-10T00:00:00Z',
            completed_at: null,
          },
        ] as ChildSupervisorRunSummary[],
        hierarchy_summary: {
          ...(rootRun.hierarchy_summary ?? {
            depth: 0,
            max_depth: 1,
            child_run_count: 0,
            descendant_task_count: 0,
            action_required_child_count: 0,
            rollup_status: 'waiting',
            requires_attention: false,
            blocked_reasons: [],
          }),
          child_run_count: 1,
          descendant_task_count: 1,
        },
      },
      {
        id: 'run-child-1',
        name: 'Frontend pod',
        parent_run: {
          parent_run_id: 'run-1',
          parent_task_id: null,
          delegated_by_agent_id: 'supervisor-root',
          objective: 'Own frontend delivery',
          created_at: '2026-03-10T00:00:00Z',
        } as SupervisorParentRunRef,
        child_runs: [],
        hierarchy_depth: 1,
        max_hierarchy_depth: 1,
        inherited_policy: {
          approval_required: true,
          reviewer_required: false,
          test_required: false,
          execution_mode: 'shared_workspace',
          workspace_dir: '.',
          memory_tags: ['frontend'],
          constraint_notes: [],
        } as SupervisorInheritancePolicy,
        status: 'running',
        task_summary: {
          total: 1,
          queued: 0,
          blocked: 0,
          pending_approval: 0,
          running: 1,
          review_pending: 0,
          test_pending: 0,
          completed: 0,
          failed: 0,
          cancelled: 0,
        },
        hierarchy_summary: {
          depth: 1,
          max_depth: 1,
          child_run_count: 0,
          descendant_task_count: 0,
          action_required_child_count: 0,
          rollup_status: 'running',
          requires_attention: false,
          blocked_reasons: [],
        },
        tasks: [],
        messages: [],
        created_at: '2026-03-10T00:00:00Z',
        updated_at: '2026-03-10T00:00:00Z',
        completed_at: null,
      },
    ];

    const { default: WorkflowsPanel } = await import('./WorkflowsPanel');
    render(<WorkflowsPanel />);

    await waitFor(() => {
      expect(screen.getByText('Frontend pod')).toBeInTheDocument();
      expect(screen.getByText(/Child of run-1/)).toBeInTheDocument();
    });

    fireEvent.click(screen.getAllByRole('button', { name: 'Create child supervisor' })[0]);
    fireEvent.change(screen.getByPlaceholderText('Delegate a bounded sub-supervisor mission...'), {
      target: { value: 'Coordinate QA and release readiness' },
    });
    fireEvent.change(screen.getByPlaceholderText('supervisor-subteam-a'), {
      target: { value: 'supervisor-beta' },
    });
    fireEvent.change(screen.getByPlaceholderText('Frontend delivery pod'), {
      target: { value: 'QA pod' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create child run' }));

    await waitFor(() => {
      expect(workflowMocks.createChildSupervisorRun).toHaveBeenCalledWith(
        expect.objectContaining({
          parent_run_id: 'run-1',
          lead_agent_id: 'supervisor-beta',
          objective: 'Coordinate QA and release readiness',
          name: 'QA pod',
        })
      );
    });

    asyncStateMock.data.runs = [rootRun];
  });

  it('renders environment recovery details and allows reconcile retry and cleanup actions', async () => {
    workflowMocks.reconcileWorkflowState.mockResolvedValue(undefined);
    workflowMocks.retryWorkflowEnvironment.mockResolvedValue(undefined);
    workflowMocks.cleanupWorkflowEnvironment.mockResolvedValue(undefined);

    const originalEnvironment = asyncStateMock.data.environments[0];
    asyncStateMock.data.environments = [
      {
        ...originalEnvironment,
        spec: {
          ...originalEnvironment.spec,
          execution_mode: 'git_worktree',
          prepared_path: '/tmp/gestura/.gestura/worktrees/session/run-1/agent-1/task-1',
          cleanup_policy: 'remove_when_clean_otherwise_archive',
          git_worktree: {
            repo_root: '/tmp/gestura',
            base_branch: 'main',
            worktree_branch: 'gestura/session/run-1/agent-1/task-1',
            worktree_path: '/tmp/gestura/.gestura/worktrees/session/run-1/agent-1/task-1',
            create_branch_if_missing: true,
          },
        },
        state: 'archived',
        health: 'dirty',
        prepared_path: '/tmp/gestura/.gestura/worktrees/session/run-1/agent-1/task-1',
        recovery_status: 'needs_operator_action',
        recovery_action: 'mark_task_blocked',
        cleanup_result: {
          disposition: 'archived',
          completed_at: '2026-03-10T01:00:00Z',
          retained_path: '/tmp/gestura/.gestura/worktrees/session/run-1/agent-1/task-1',
          summary: 'Archived dirty worktree for operator inspection.',
        },
      },
    ];

    const { default: WorkflowsPanel } = await import('./WorkflowsPanel');
    const view = render(<WorkflowsPanel />);
    const current = within(view.container);

    fireEvent.click(current.getByRole('button', { name: 'Environments (1)' }));

    await waitFor(() => {
      expect(current.getByText('Execution Environments (1)')).toBeInTheDocument();
    });

    expect(current.getByText('git_worktree')).toBeInTheDocument();
    expect(current.getByText('dirty')).toBeInTheDocument();
    expect(current.getByText(/Recovery: needs_operator_action · mark_task_blocked/)).toBeInTheDocument();
    expect(current.getByText(/Cleanup: Archived dirty worktree for operator inspection\./)).toBeInTheDocument();
    expect(current.getByText(/Path: \/tmp\/gestura\/\.gestura\/worktrees\/session\/run-1\/agent-1\/task-1/)).toBeInTheDocument();

    fireEvent.click(current.getByRole('button', { name: 'Reconcile' }));
    fireEvent.click(current.getByRole('button', { name: 'Retry prep' }));
    fireEvent.click(current.getByRole('button', { name: 'Cleanup' }));

    await waitFor(() => {
      expect(workflowMocks.reconcileWorkflowState).toHaveBeenCalledTimes(1);
      expect(workflowMocks.retryWorkflowEnvironment).toHaveBeenCalledWith('env-1');
      expect(workflowMocks.cleanupWorkflowEnvironment).toHaveBeenCalledWith('env-1', true);
    });

    asyncStateMock.data.environments = [originalEnvironment];
  });
});