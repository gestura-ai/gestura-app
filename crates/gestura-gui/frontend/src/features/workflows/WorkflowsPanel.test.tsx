import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type {
  ActiveTaskSnapshot,
  ChildSupervisorRunSummary,
  EnvironmentRecord,
  SupervisorInheritancePolicy,
  SupervisorParentRunRef,
  SupervisorRun,
} from '../../services/tauri/workflows';

const workflowMocks = vi.hoisted(() => ({
  acknowledgeBlockedWorkflowTask: vi.fn().mockResolvedValue(undefined),
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
  pauseWorkflowTask: vi.fn().mockResolvedValue(undefined),
  restartWorkflowTaskFromScratch: vi.fn().mockResolvedValue(undefined),
  resumeWorkflowTask: vi.fn().mockResolvedValue(undefined),
  retryWorkflowEnvironment: vi.fn().mockResolvedValue(undefined),
  retryWorkflowTask: vi.fn().mockResolvedValue(undefined),
  sendWorkflowCollaborationMessage: vi.fn().mockResolvedValue(undefined),
  sendWorkflowMessage: vi.fn().mockResolvedValue(undefined),
  spawnSubagent: vi.fn().mockResolvedValue(undefined),
  updateWorkflowThreadAction: vi.fn().mockResolvedValue(undefined),
}));

const asyncStateMock = vi.hoisted(() => ({
  data: {
    activeTasks: [] as ActiveTaskSnapshot[],
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
        shared_cognition: [
          {
            id: 'note-1',
            run_id: 'run-1',
            task_id: 'task-1',
            directive_id: 'directive-1',
            kind: 'steering',
            message_kind: 'status_update',
            summary: 'Use ripgrep first',
            detail: 'Use ripgrep first and keep the worktree clean before editing.',
            sender_agent_id: 'supervisor',
            recipient_agent_id: 'agent-1',
            tags: ['shared-cognition', 'workflow-run:run-1'],
            confidence: 0.92,
            source_message_id: 'msg-shared-1',
            created_at: '2026-03-10T00:05:00Z',
          },
        ],
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
    cleanup();
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

  it('renders shared cognition notes for workflow runs', async () => {
    const { default: WorkflowsPanel } = await import('./WorkflowsPanel');
    render(<WorkflowsPanel />);

    await waitFor(() => {
      expect(screen.getByText(/Shared cognition: 1/)).toBeInTheDocument();
    });

    expect(screen.getAllByText(/Use ripgrep first/i).length).toBeGreaterThan(0);
    expect(screen.getByText(/keep the worktree clean before editing/i)).toBeInTheDocument();
  });

  it('renders checkpoint controls for blocked tasks and invokes resume/restart/acknowledge actions', async () => {
    const originalRun = asyncStateMock.data.runs[0] as SupervisorRun;
    asyncStateMock.data.runs = [
      {
        ...originalRun,
        task_summary: {
          ...originalRun.task_summary,
          blocked: 1,
          pending_approval: 0,
        },
        tasks: [
          {
            ...originalRun.tasks[0],
            state: 'blocked',
            approval: {
              ...originalRun.tasks[0].approval,
              state: 'approved',
              active_request: null,
            },
            blocked_reasons: ["execution interrupted during restart; task can resume from checkpoint 'after tool file result'"],
            checkpoint: {
              stage: 'blocked',
              replay_safety: 'checkpoint_resumable',
              resume_disposition: 'resume_from_checkpoint',
              safe_boundary_label: 'after tool file result',
              available_actions: [
                'resume_from_checkpoint',
                'restart_from_scratch',
                'acknowledge_blocked',
              ],
              note: 'resume available after restart',
              completed_tool_call_count: 1,
              has_resume_state: true,
              result_published: false,
              updated_at: '2026-03-10T00:00:00Z',
            },
          },
        ],
      },
    ];

    const { default: WorkflowsPanel } = await import('./WorkflowsPanel');
    render(<WorkflowsPanel />);

    await waitFor(() => {
      expect(screen.getByText(/Checkpoint: Blocked/i)).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Resume' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Restart from scratch' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Acknowledge blocked' })).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    fireEvent.click(screen.getByRole('button', { name: 'Restart from scratch' }));
    fireEvent.click(screen.getByRole('button', { name: 'Acknowledge blocked' }));

    await waitFor(() => {
      expect(workflowMocks.resumeWorkflowTask).toHaveBeenCalledWith('task-1');
      expect(workflowMocks.restartWorkflowTaskFromScratch).toHaveBeenCalledWith('task-1');
      expect(workflowMocks.acknowledgeBlockedWorkflowTask).toHaveBeenCalledWith(
        'task-1',
        'Acknowledged in workflow panel.'
      );
    });

    asyncStateMock.data.runs = [originalRun];
  });

  it('renders pause and cancel controls for active local tasks', async () => {
    const activeTask: ActiveTaskSnapshot = {
      task: {
        ...(asyncStateMock.data.runs[0] as SupervisorRun).tasks[0].task,
        id: 'task-active-local',
        prompt: 'Continue implementing the local workflow controls',
        execution_mode: 'shared_workspace',
      },
      state: 'running',
      blocked_reasons: [],
      checkpoint: {
        stage: 'running',
        replay_safety: 'checkpoint_resumable',
        resume_disposition: 'resume_from_checkpoint',
        safe_boundary_label: 'after file read',
        completed_tool_call_count: 1,
        has_resume_state: true,
        result_published: false,
        available_actions: ['resume_from_checkpoint'],
        note: 'Checkpoint captured',
        updated_at: '2026-03-10T00:00:00Z',
      },
      local_execution: {
        status: 'running',
        status_reason: null,
        last_synced_at: '2026-03-10T00:00:00Z',
        progress: {
          phase: 'waiting',
          waiting_reason: 'shell_process',
          stage: 'shell_running',
          message: 'Streaming shell output',
          percent: 45,
          iteration: 2,
          current_tool_name: 'shell',
          last_completed_tool_name: 'file',
          last_completed_tool_duration_ms: 12,
          completed_tool_call_count: 1,
          has_partial_content: true,
          partial_content_chars: 48,
          has_partial_thinking: false,
          partial_thinking_chars: 0,
          token_usage: null,
          environment: null,
          updated_at: '2026-03-10T00:00:00Z',
        },
      },
      remote_execution: null,
    };
    asyncStateMock.data.activeTasks = [activeTask];

    const { default: WorkflowsPanel } = await import('./WorkflowsPanel');
    render(<WorkflowsPanel />);

    await waitFor(() => {
      expect(screen.getByText('Live Active Tasks (1)')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Pause' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
      expect(screen.getByText(/Current tool: shell/)).toBeInTheDocument();
      expect(screen.getByText(/Checkpoint: Running/)).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole('button', { name: 'Pause' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => {
      expect(workflowMocks.pauseWorkflowTask).toHaveBeenCalledWith('task-active-local');
      expect(workflowMocks.cancelTask).toHaveBeenCalledWith('task-active-local');
    });

    asyncStateMock.data.activeTasks = [];
  });

  it('renders remote progress for active remote tasks without pause controls', async () => {
    const originalRuns = asyncStateMock.data.runs;
    const remoteTask: ActiveTaskSnapshot = {
      task: {
        ...(originalRuns[0] as SupervisorRun).tasks[0].task,
        id: 'task-active-remote',
        prompt: 'Track the remote workflow execution',
        execution_mode: 'remote',
      },
      state: 'running',
      blocked_reasons: [],
      checkpoint: null,
      local_execution: null,
      remote_execution: {
        target: {
          url: 'http://localhost:32145/a2a',
          name: 'remote-peer',
          auth_token: null,
          capabilities: ['shell'],
        },
        remote_task_id: 'remote-task-1',
        status: 'running',
        status_reason: 'Awaiting remote shell completion',
        lease: null,
        progress: {
          stage: 'shell_running',
          message: 'Remote shell still streaming',
          percent: 60,
          updated_at: '2026-03-10T00:00:00Z',
        },
        artifacts: [],
        provenance: null,
        compatibility: {
          supported_features: ['artifacts'],
          warnings: [],
          protocol_version: '2025-11-25',
        },
        last_synced_at: '2026-03-10T00:00:00Z',
      },
    };
    try {
      asyncStateMock.data.runs = [];
      asyncStateMock.data.activeTasks = [remoteTask];

      const { default: WorkflowsPanel } = await import('./WorkflowsPanel');
      render(<WorkflowsPanel />);

      await waitFor(() => {
        expect(screen.getByText('Live Active Tasks (1)')).toBeInTheDocument();
      });

      const activeTasksSection = screen.getByText('Live Active Tasks (1)').closest('.workflows-section');
      expect(activeTasksSection).not.toBeNull();

      expect(within(activeTasksSection as HTMLElement).getByText(/Remote: running/)).toBeInTheDocument();
      expect(
        within(activeTasksSection as HTMLElement).getByText(/60% · shell_running · Remote shell still streaming/)
      ).toBeInTheDocument();
      expect(within(activeTasksSection as HTMLElement).queryByRole('button', { name: 'Pause' })).not.toBeInTheDocument();
      expect(within(activeTasksSection as HTMLElement).getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
    } finally {
      asyncStateMock.data.activeTasks = [];
      asyncStateMock.data.runs = originalRuns;
    }
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
        shared_cognition: [],
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