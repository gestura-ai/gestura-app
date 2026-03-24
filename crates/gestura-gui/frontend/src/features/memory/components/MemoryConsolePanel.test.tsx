import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const agentMocks = vi.hoisted(() => ({
  clearMemoryConsoleEntries: vi.fn(),
  deleteMemoryEntry: vi.fn(),
  getMemoryConsoleOverview: vi.fn(),
  getMemoryConsoleSessions: vi.fn(),
  getMemoryEntryDetail: vi.fn(),
  getMemoryPromotionCandidates: vi.fn(),
  getMemoryTaskLifecycle: vi.fn(),
  getMemoryWorkingSnapshot: vi.fn(),
  promoteMemoryCandidateEntry: vi.fn(),
  refreshMemoryConsoleGovernance: vi.fn(),
  searchMemoryConsoleEntries: vi.fn(),
  setMemoryEntryArchived: vi.fn(),
  updateMemoryEntryDetail: vi.fn(),
}));

vi.mock('../../../services/tauri/agent', () => agentMocks);

import { MemoryConsolePanel } from './MemoryConsolePanel';

describe('MemoryConsolePanel', () => {
  beforeEach(() => {
    const sharedEntry = {
      entry_id: 'entry-1',
      summary: 'Supervisor revised scope',
      memory_kind: 'long_term',
      memory_type: 'reflection',
      scope: 'directive',
      category: 'shared_cognition',
      confidence: 0.82,
      tags: ['shared-cognition', 'steering'],
      session_id: 'session-1',
      task_id: 'task-9',
      directive_id: 'directive-4',
      agent_id: 'supervisor',
      governance_state: 'active',
      governance_issue_count: 0,
      archived: false,
      timestamp: '2026-03-12T00:00:00Z',
    };

    agentMocks.getMemoryConsoleSessions.mockResolvedValue([
      { session_id: 'session-1', title: 'Session 1', workspace_dir: '/tmp/workspace' },
    ]);
    agentMocks.getMemoryConsoleOverview.mockResolvedValue({
      workspace_dir: '/tmp/workspace',
      session: { session_id: 'session-1', title: 'Session 1', workspace_dir: '/tmp/workspace' },
      durable_total: 2,
      open_blocker_count: 1,
      promotion_candidate_count: 0,
      working_resource_count: 1,
      working_decision_count: 1,
      governance_review_count: 0,
      governance_issue_count: 0,
      working_summary: 'Supervisor is coordinating agent findings.',
      recent_entries: [],
      counts_by_kind: [],
      counts_by_type: [],
      counts_by_scope: [],
      counts_by_category: [{ key: 'shared_cognition', count: 2 }],
      counts_by_governance: [],
    });
    agentMocks.searchMemoryConsoleEntries.mockResolvedValue({
      query: { category: 'shared_cognition', text: null, limit: 24, include_archived: true },
      working_memory: [],
      durable_memory: [sharedEntry],
    });
    agentMocks.getMemoryEntryDetail.mockResolvedValue({
      summary: sharedEntry,
      content: 'Tighten the execution scope to the API shim only.',
      governance_note: null,
      governance_suggestions: [],
      strategy_key: null,
      outcome_labels: [],
    });
    agentMocks.getMemoryTaskLifecycle.mockResolvedValue({
      task_id: 'task-9',
      lifecycle: {
        events: [
          {
            phase: 'promoted',
            summary: 'Promoted execution result to durable memory',
            scope: 'workspace',
            memory_type: 'reflection',
            memory_file_path: '/tmp/workspace/memory/entry-1.md',
            recorded_at: '2026-03-12T00:05:00Z',
          },
        ],
        last_memory_file_path: '/tmp/workspace/memory/entry-1.md',
      },
    });
    agentMocks.getMemoryWorkingSnapshot.mockResolvedValue({
      summary: 'Session memory is currently sparse but healthy.',
      resources: [],
      decisions: [],
      blockers: [],
      timeline: [],
      next_actions: [],
      open_questions: [],
    });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it('surfaces shared cognition overview counts and quick-filter search results', async () => {
    render(<MemoryConsolePanel sessionId="session-1" workspaceDir="/tmp/workspace" allowSessionSelection={false} />);

    expect(
      await screen.findByText(/Short-term session working memory, long-term memory bank entries/i),
    ).toBeInTheDocument();

    const sharedSummary = await screen.findByText(/Shared cognition entries/i);
    expect(sharedSummary.parentElement).toHaveTextContent('Shared cognition entries');
    expect(sharedSummary.parentElement).toHaveTextContent('2');

    fireEvent.click(await screen.findByRole('button', { name: 'Shared cognition' }));

    await waitFor(() => {
      expect(agentMocks.searchMemoryConsoleEntries).toHaveBeenCalledWith(
        expect.objectContaining({ category: 'shared_cognition', text: null, limit: 24, include_archived: true }),
        'session-1',
        '/tmp/workspace',
      );
    });

    fireEvent.click((await screen.findByText('Supervisor revised scope')).closest('button') as HTMLButtonElement);
    const categoryMeta = await screen.findByText(/Category \/ scope \/ type:/);
    expect(categoryMeta.parentElement).toHaveTextContent('shared_cognition / directive / reflection');
    const ownershipMeta = screen.getByText(/Task \/ directive \/ agent:/);
    expect(ownershipMeta.parentElement).toHaveTextContent('task-9 / directive-4 / supervisor');
    const confidenceMeta = screen.getByText(/Confidence \/ tags:/);
    expect(confidenceMeta.parentElement).toHaveTextContent('82% / shared-cognition, steering');
  });

  it('auto-loads current session task memory history and refreshes on signal changes', async () => {
    const tasks = [
      {
        id: 'task-9',
        name: 'Finalize execution chain',
        status: 'Completed' as const,
        subtasks: [
          {
            id: 'task-10',
            name: 'Archive results',
            status: 'Completed' as const,
          },
        ],
      },
    ];

    const { rerender } = render(
      <MemoryConsolePanel
        sessionId="session-1"
        workspaceDir="/tmp/workspace"
        tasks={tasks}
        refreshSignal={0}
        allowSessionSelection={false}
      />,
    );

    fireEvent.change(screen.getByRole('combobox', { name: 'Memory view' }), {
      target: { value: 'task' },
    });

    await waitFor(() => {
      expect(agentMocks.getMemoryTaskLifecycle).toHaveBeenCalledWith('session-1', 'task-9');
    });

    expect(await screen.findByText('Finalize execution chain')).toBeInTheDocument();
    expect(screen.getByText(/Promoted execution result to durable memory/)).toBeInTheDocument();
    expect(screen.getByText(/Latest durable memory:/).parentElement).toHaveTextContent('/tmp/workspace/memory/entry-1.md');

    rerender(
      <MemoryConsolePanel
        sessionId="session-1"
        workspaceDir="/tmp/workspace"
        tasks={tasks}
        refreshSignal={1}
        allowSessionSelection={false}
      />,
    );

    await waitFor(() => {
      expect(agentMocks.getMemoryConsoleOverview).toHaveBeenCalledTimes(2);
    });
  });

  it('renders session working memory safely when Tauri omits empty arrays', async () => {
    agentMocks.getMemoryWorkingSnapshot.mockResolvedValue({
      summary: 'Fresh session with no captured memory yet.',
    });

    render(<MemoryConsolePanel sessionId="session-1" workspaceDir="/tmp/workspace" allowSessionSelection={false} />);

    fireEvent.change(screen.getByRole('combobox', { name: 'Memory view' }), {
      target: { value: 'working' },
    });

    expect(await screen.findByText('Fresh session with no captured memory yet.')).toBeInTheDocument();
    expect(screen.getByText('Resources').parentElement).toHaveTextContent('0');
    expect(screen.getByText('Decisions').parentElement).toHaveTextContent('0');
    expect(screen.getByText('Blockers').parentElement).toHaveTextContent('0');
    expect(screen.getByText(/Next actions/i).parentElement).toHaveTextContent('None');
    expect(screen.getByText(/Open questions/i).parentElement).toHaveTextContent('None');
  });
});

