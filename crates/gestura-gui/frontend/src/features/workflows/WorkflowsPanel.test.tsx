import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const workflowMocks = vi.hoisted(() => ({
  approveWorkflowTask: vi.fn().mockResolvedValue(undefined),
  cancelTask: vi.fn().mockResolvedValue(undefined),
  claimWorkflowTask: vi.fn().mockResolvedValue(undefined),
  delegateTask: vi.fn().mockResolvedValue('task-123'),
  listActiveTasks: vi.fn().mockResolvedValue([]),
  listSupervisorRuns: vi.fn().mockResolvedValue([]),
  rejectWorkflowTask: vi.fn().mockResolvedValue(undefined),
  retryWorkflowTask: vi.fn().mockResolvedValue(undefined),
  sendWorkflowMessage: vi.fn().mockResolvedValue(undefined),
  spawnSubagent: vi.fn().mockResolvedValue(undefined),
}));

const asyncStateMock = vi.hoisted(() => ({
  data: {
    activeTasks: [],
    agents: [{ id: 'agent-1', name: 'Reviewer', status: 'running', role: 'reviewer' }],
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
            approval: { state: 'pending' },
            environment: { id: 'env-1', execution_mode: 'shared_workspace', root_dir: '.', write_access: true },
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
      expect(workflowMocks.approveWorkflowTask).toHaveBeenCalledWith('task-1');
    });
  });
});