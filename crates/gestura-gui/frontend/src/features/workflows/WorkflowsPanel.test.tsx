import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const workflowMocks = vi.hoisted(() => ({
  approveWorkflowTask: vi.fn().mockResolvedValue(undefined),
  cancelTask: vi.fn().mockResolvedValue(undefined),
  claimWorkflowTask: vi.fn().mockResolvedValue(undefined),
  cleanupWorkflowEnvironment: vi.fn().mockResolvedValue(undefined),
  delegateTask: vi.fn().mockResolvedValue('task-123'),
  listActiveTasks: vi.fn().mockResolvedValue([]),
  listWorkflowEnvironments: vi.fn().mockResolvedValue([]),
  listSupervisorRuns: vi.fn().mockResolvedValue([]),
  reconcileWorkflowState: vi.fn().mockResolvedValue(undefined),
  rejectWorkflowTask: vi.fn().mockResolvedValue(undefined),
  retryWorkflowEnvironment: vi.fn().mockResolvedValue(undefined),
  retryWorkflowTask: vi.fn().mockResolvedValue(undefined),
  sendWorkflowMessage: vi.fn().mockResolvedValue(undefined),
  spawnSubagent: vi.fn().mockResolvedValue(undefined),
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
    ],
    runs: [
      {
        id: 'run-1',
        status: 'waiting',
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
    ],
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
});