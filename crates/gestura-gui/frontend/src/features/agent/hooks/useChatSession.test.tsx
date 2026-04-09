import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useChatSession } from './useChatSession';
import type { StreamEventDispatch } from './useStreamEvents';
import type { ShellSessionRecord } from '../types';

let streamDispatch: StreamEventDispatch | null = null;

const getSessionHistoryMock = vi.fn();
const getSessionReplaySnapshotMock = vi.fn();
const hasSessionPausedExecutionMock = vi.fn();
const getTaskHierarchyMock = vi.fn();
const listKnowledgeItemsMock = vi.fn();
const getEnabledKnowledgeMock = vi.fn();
const getSessionToolSettingsMock = vi.fn();
const listBuiltinToolsMock = vi.fn();
const listDiscoveredMcpToolsMock = vi.fn();
const pauseStreamingMock = vi.fn();
const resumeStreamingMock = vi.fn();
const sendMessageStreamingMock = vi.fn();

vi.mock('./useStreamEvents', async () => {
  const actual = await vi.importActual<typeof import('./useStreamEvents')>('./useStreamEvents');
  return {
    ...actual,
    useStreamEvents: (_sessionId: string, dispatch: StreamEventDispatch) => {
      streamDispatch = dispatch;
    },
  };
});

vi.mock('../../../services/tauri/agent', () => ({
  sendMessageStreaming: (...args: unknown[]) => sendMessageStreamingMock(...args),
  cancelStreaming: vi.fn(),
  pauseStreaming: (...args: unknown[]) => pauseStreamingMock(...args),
  resumeStreaming: (...args: unknown[]) => resumeStreamingMock(...args),
  getSessionHistory: (...args: unknown[]) => getSessionHistoryMock(...args),
  getSessionReplaySnapshot: (...args: unknown[]) => getSessionReplaySnapshotMock(...args),
  hasSessionPausedExecution: (...args: unknown[]) => hasSessionPausedExecutionMock(...args),
  getTaskHierarchy: (...args: unknown[]) => getTaskHierarchyMock(...args),
  listKnowledgeItems: (...args: unknown[]) => listKnowledgeItemsMock(...args),
  getEnabledKnowledge: (...args: unknown[]) => getEnabledKnowledgeMock(...args),
  getSessionToolSettings: (...args: unknown[]) => getSessionToolSettingsMock(...args),
  listBuiltinTools: (...args: unknown[]) => listBuiltinToolsMock(...args),
  listDiscoveredMcpTools: (...args: unknown[]) => listDiscoveredMcpToolsMock(...args),
  resolveToolConfirmationDecision: vi.fn(),
  enhancePrompt: vi.fn(async (text: string) => text),
  startVoiceListening: vi.fn(),
  stopVoiceListening: vi.fn(),
}));

function blocksToText(
  blocks: Array<{
    kind: string;
    title?: string | null;
    content?: string;
    label?: string;
    detail?: string;
    name?: string;
    message?: string;
    summary?: string | null;
    reason?: string | null;
    nextStep?: string | null;
    evidence?: string[];
  }>,
) {
  return blocks
    .map((block) => {
      if (block.kind === 'iteration-marker') {
        return `${block.kind}:${block.label ?? ''}:${block.detail ?? ''}`;
      }
      if (block.kind === 'tool') {
        return `${block.kind}:${block.name ?? ''}:${block.content ?? ''}`;
      }
      if (block.kind === 'narration') {
        return [
          block.kind,
          block.title ?? '',
          block.message ?? '',
          block.summary ?? '',
          block.reason ?? '',
          block.nextStep ?? '',
          (block.evidence ?? []).join('&'),
        ].join(':');
      }
      return `${block.kind}:${block.content ?? ''}`;
    })
    .join('|');
}

function Harness({ shellSessions = [] }: { shellSessions?: ShellSessionRecord[] }) {
  const state = useChatSession('session-123', { shellSessions });
  return (
    <div>
      <div data-testid="status">{state.status.text}</div>
      <div data-testid="memory-revision">{state.memoryRevision}</div>
      <div data-testid="can-resume">{String(state.canResume)}</div>
      <div data-testid="is-processing">{String(state.isProcessing)}</div>
      <div data-testid="is-stopping">{String(state.isStopping)}</div>
      <div data-testid="runtime-task-snapshot">{JSON.stringify(state.runtimeTaskSnapshot)}</div>
      <div data-testid="messages">
        {JSON.stringify(state.messages.map((message) => ({
          role: message.role,
          rawMarkdown: message.rawMarkdown,
          blocks: blocksToText(message.blocks),
        })))}
      </div>
      <div data-testid="streaming-present">{String(Boolean(state.streamingMessage))}</div>
      <div data-testid="streaming">{state.streamingMessage ? blocksToText(state.streamingMessage.blocks) : ''}</div>
      <div data-testid="streaming-blocks">{state.streamingMessage ? JSON.stringify(state.streamingMessage.blocks) : '[]'}</div>
      <div data-testid="streaming-narration-titles">
        {state.streamingMessage
          ? JSON.stringify(
            state.streamingMessage.blocks
              .filter((block) => block.kind === 'narration')
              .map((block) => block.title ?? ''),
          )
          : ''}
      </div>
      <button type="button" onClick={() => { void state.resumeStream(); }}>
        Resume
      </button>
      <button type="button" onClick={() => { void state.cancelStream(); }}>
        Stop
      </button>
      <button type="button" onClick={() => { void state.sendMessage('you timed out please pick up where you left off'); }}>
        Send resume-like prompt
      </button>
      <button type="button" onClick={() => { void state.sendMessage('Run the tool workflow'); }}>
        Send
      </button>
    </div>
  );
}

describe('useChatSession', () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    streamDispatch = null;
    getSessionHistoryMock.mockResolvedValue([]);
    getSessionReplaySnapshotMock.mockResolvedValue({ history: [], activity_log: [], has_paused_execution: false });
    hasSessionPausedExecutionMock.mockResolvedValue(false);
    getTaskHierarchyMock.mockResolvedValue([]);
    listKnowledgeItemsMock.mockResolvedValue([]);
    getEnabledKnowledgeMock.mockResolvedValue([]);
    getSessionToolSettingsMock.mockResolvedValue({ mode: 'allowlist', allowlist: [], requireConfirmation: [] });
    listBuiltinToolsMock.mockResolvedValue([]);
    listDiscoveredMcpToolsMock.mockResolvedValue([]);
    sendMessageStreamingMock.mockReset();
    pauseStreamingMock.mockReset();
    resumeStreamingMock.mockReset();
  });

  it('keeps partial assistant content visible when a stream is paused', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'chunk', chunk: 'Partial answer' });
      streamDispatch?.({ type: 'paused' });
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Interrupted — resume available');
    expect(screen.getByTestId('can-resume')).toHaveTextContent('true');
    expect(screen.getByTestId('is-processing')).toHaveTextContent('false');
    expect(screen.getByTestId('streaming')).toHaveTextContent('Partial answer');
  });

  it('stores structured narration fields on the streaming message', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({
        type: 'narration',
        title: 'Verification is active',
        message: 'I’m reviewing the latest result before I close this task out.',
        summary: 'The latest result kept the work in verification.',
        reason: 'That matters because the task still needs one more piece of proof.',
        nextStep: 'I’ll run the targeted validation check next.',
        evidence: ['Current step: "Run targeted verification".'],
        stage: 'verification',
      });
    });

    expect(screen.getByTestId('streaming')).toHaveTextContent(
      'narration:Verification is active:I’m reviewing the latest result before I close this task out.:The latest result kept the work in verification.:That matters because the task still needs one more piece of proof.:I’ll run the targeted validation check next.:Current step: "Run targeted verification".',
    );
  });

  it('stores runtime task snapshots from streaming events', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({
        type: 'task-runtime-state',
        snapshot: {
          root_task_id: 'root-task',
          current_task: { id: 'verify-task', name: 'Verify facts', status: 'not_started' },
          ready_tasks: [{ id: 'verify-task', name: 'Verify facts', status: 'not_started' }],
          parallel_ready_tasks: [],
          blocked_tasks: [],
          open_tasks: [{ id: 'verify-task', name: 'Verify facts', status: 'not_started' }],
          completed_tasks: [],
          missing_requirements: ['verification still required'],
          status_message: 'Verification remains open',
        },
      });
    });

    expect(screen.getByTestId('runtime-task-snapshot')).toHaveTextContent('Verify facts');
    expect(screen.getByTestId('runtime-task-snapshot')).toHaveTextContent('verification still required');
  });

  it('materializes a shell session block immediately from session lifecycle events', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({
        type: 'shell-session-lifecycle',
        shellSessionId: 'shell-session-1',
        payload: {
          shell_session_id: 'shell-session-1',
          state: 'Busy',
          active_process_id: 'proc-1',
          active_command: 'cargo test',
          cwd: '/workspace',
          interactive: true,
          user_managed: false,
        },
      });
    });

    const blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]');
    expect(blocks).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          kind: 'shell-session',
          shellSessionId: 'shell-session-1',
          state: 'Busy',
          activeProcessId: 'proc-1',
          activeCommand: 'cargo test',
          cwd: '/workspace',
        }),
      ]),
    );
  });

  it('materializes shell output immediately when output arrives before lifecycle', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({
        type: 'shell-output',
        processId: 'proc-out-first',
        shellSessionId: 'shell-session-out-first',
        stream: 'Stdout',
        data: 'compiling...\n',
      });
    });

    let blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]') as Array<Record<string, unknown>>;
    let shellBlock = blocks.find((block) => block.kind === 'shell-session');

    expect(shellBlock).toEqual(expect.objectContaining({
      shellSessionId: 'shell-session-out-first',
      state: 'Idle',
    }));
    expect(shellBlock?.lines).toEqual(expect.arrayContaining([
      expect.objectContaining({ data: 'compiling...\n' }),
    ]));

    await act(async () => {
      streamDispatch?.({
        type: 'shell-lifecycle',
        processId: 'proc-out-first',
        payload: {
          process_id: 'proc-out-first',
          shell_session_id: 'shell-session-out-first',
          state: 'Started',
          command: 'cargo check',
          cwd: '/workspace',
        },
      });
    });

    blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]');
    shellBlock = blocks.find((block) => block.kind === 'shell-session');
    const lines = Array.isArray(shellBlock?.lines) ? shellBlock.lines as Array<Record<string, unknown>> : [];

    expect(shellBlock).toEqual(expect.objectContaining({
      shellSessionId: 'shell-session-out-first',
      state: 'Busy',
      activeProcessId: 'proc-out-first',
      activeCommand: 'cargo check',
      cwd: '/workspace',
    }));
    expect(String(lines[0]?.data ?? '')).toContain('cargo check');
    expect(String(lines[1]?.data ?? '')).toContain('compiling...');
    expect(blocks.find((block) => block.kind === 'shell')).toBeUndefined();
  });

  it('keeps a session-backed shell transcript reusable after a failed command', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({
        type: 'shell-lifecycle',
        processId: 'proc-failed-1',
        payload: {
          process_id: 'proc-failed-1',
          shell_session_id: 'shell-session-failed-1',
          state: 'Started',
          command: 'cargo test --workspace',
          cwd: '/workspace',
        },
      });
      streamDispatch?.({
        type: 'shell-output',
        processId: 'proc-failed-1',
        shellSessionId: 'shell-session-failed-1',
        stream: 'Stderr',
        data: 'error: test failed\n',
      });
      streamDispatch?.({
        type: 'shell-lifecycle',
        processId: 'proc-failed-1',
        payload: {
          process_id: 'proc-failed-1',
          shell_session_id: 'shell-session-failed-1',
          state: 'Failed',
          command: 'cargo test --workspace',
          cwd: '/workspace',
          exit_code: 1,
          duration_ms: 55,
        },
      });
    });

    const blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]') as Array<Record<string, unknown>>;
    const shellBlock = blocks.find((block) => block.kind === 'shell-session');
    const lines = Array.isArray(shellBlock?.lines) ? shellBlock.lines as Array<Record<string, unknown>> : [];

    expect(shellBlock).toEqual(expect.objectContaining({
      shellSessionId: 'shell-session-failed-1',
      state: 'Idle',
      activeProcessId: null,
      activeCommand: null,
      lastExitCode: 1,
      availableForReuse: true,
    }));
    expect(lines.some((line) => String(line.data ?? '').includes('cargo test --workspace'))).toBe(true);
    expect(lines.some((line) => String(line.data ?? '').includes('error: test failed'))).toBe(true);
    expect(blocks.find((block) => block.kind === 'shell')).toBeUndefined();
  });

  it('marks stop as in progress on the first click and ignores repeated clicks', async () => {
    pauseStreamingMock.mockResolvedValue(undefined);

    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByTestId('status')).toHaveTextContent('Ready');
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Stopping…');
    expect(screen.getByTestId('is-stopping')).toHaveTextContent('true');
    expect(pauseStreamingMock).toHaveBeenCalledTimes(1);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    });

    expect(pauseStreamingMock).toHaveBeenCalledTimes(1);
  });

  it('routes voice-originated agent messages into the streaming send path', async () => {
    sendMessageStreamingMock.mockResolvedValue(undefined);

    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'agent-message', role: 'user', content: 'Open the project README' });
    });

    await waitFor(() => {
      expect(sendMessageStreamingMock).toHaveBeenCalledWith({
        session_id: 'session-123',
        message: 'Open the project README',
        task_id: null,
      });
    });

    expect(screen.getByTestId('messages')).toHaveTextContent('"role":"user"');
    expect(screen.getByTestId('messages')).toHaveTextContent('Open the project README');
  });

  it('resumes into the same assistant message instead of starting a new bubble', async () => {
    resumeStreamingMock.mockImplementation(async () => {
      streamDispatch?.({ type: 'chunk', chunk: ' world' });
      streamDispatch?.({ type: 'done' });
    });

    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'chunk', chunk: 'Hello' });
      streamDispatch?.({ type: 'paused' });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    });

    await waitFor(() => {
      expect(resumeStreamingMock).toHaveBeenCalledWith('session-123');
    });

    const messages = screen.getByTestId('messages').textContent ?? '';
    expect(messages).toContain('Hello world');
    expect(messages.match(/rawMarkdown/g)).toHaveLength(1);
    expect(screen.getByTestId('can-resume')).toHaveTextContent('false');
  });

  it('sends resume-like follow-up prompts as normal messages even when resume is available', async () => {
    sendMessageStreamingMock.mockResolvedValue(undefined);

    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'chunk', chunk: 'Partial answer' });
      streamDispatch?.({ type: 'paused' });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send resume-like prompt' }));
    });

    await waitFor(() => {
      expect(sendMessageStreamingMock).toHaveBeenCalledWith({
        session_id: 'session-123',
        message: 'you timed out please pick up where you left off',
        task_id: null,
      });
    });

    expect(resumeStreamingMock).not.toHaveBeenCalled();
  });

  it('does not await the full streaming invoke before marking the send as active', async () => {
    const originalRequestAnimationFrame = window.requestAnimationFrame;
    let paintCallback: FrameRequestCallback | null = null;
    window.requestAnimationFrame = vi.fn((cb: FrameRequestCallback) => {
      paintCallback = cb;
      return 1;
    });

    let rejectStreaming: ((reason?: unknown) => void) | null = null;
    sendMessageStreamingMock.mockImplementation(
      () => new Promise((_, reject: (reason?: unknown) => void) => { rejectStreaming = reject; }),
    );

    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByTestId('status')).toHaveTextContent('Ready');
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    expect(screen.getByTestId('is-processing')).toHaveTextContent('true');
    expect(screen.getByTestId('status')).toHaveTextContent('Thinking…');
    expect(screen.getByTestId('messages')).toHaveTextContent('Run the tool workflow');
    expect(screen.getByTestId('streaming-present')).toHaveTextContent('true');
    expect(screen.getByTestId('streaming-blocks')).toHaveTextContent('[]');
    expect(sendMessageStreamingMock).not.toHaveBeenCalled();

    await act(async () => {
      paintCallback?.(performance.now());
      await Promise.resolve();
    });

    expect(sendMessageStreamingMock).toHaveBeenCalledWith({
      session_id: 'session-123',
      message: 'Run the tool workflow',
      task_id: null,
    });

    await act(async () => {
      rejectStreaming?.(new Error('invoke failed'));
    });

    await waitFor(() => {
      expect(screen.getByTestId('is-processing')).toHaveTextContent('false');
      expect(screen.getByTestId('status')).toHaveTextContent('Error: Error: invoke failed');
      expect(screen.getByTestId('streaming-blocks')).toHaveTextContent('[]');
      expect(screen.getByTestId('streaming-present')).toHaveTextContent('false');
    });

    window.requestAnimationFrame = originalRequestAnimationFrame;
  });

  it('shows a streaming placeholder immediately when a message is sent', async () => {
    sendMessageStreamingMock.mockResolvedValue(undefined);

    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByTestId('status')).toHaveTextContent('Ready');
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    expect(screen.getByTestId('is-processing')).toHaveTextContent('true');
    expect(screen.getByTestId('streaming-present')).toHaveTextContent('true');
    expect(screen.getByTestId('streaming-blocks')).toHaveTextContent('[]');
  });

  it('recovers an inline shell-session block from shared shell session state during an active request', async () => {
    sendMessageStreamingMock.mockResolvedValue(undefined);

    const { rerender } = render(<Harness shellSessions={[]} />);

    await waitFor(() => {
      expect(screen.getByTestId('status')).toHaveTextContent('Ready');
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    expect(screen.getByTestId('is-processing')).toHaveTextContent('true');
    expect(screen.getByTestId('streaming-blocks')).toHaveTextContent('[]');

    rerender(
      <Harness shellSessions={[
        {
          kind: 'shell-session',
          id: 'shell-session-recovered',
          shellSessionId: 'shell-session-recovered',
          cwd: '/workspace',
          state: 'Busy',
          interactive: true,
          userManaged: false,
          activeProcessId: 'proc-recovered',
          activeCommand: 'cargo test --workspace',
          lastExitCode: null,
          durationMs: null,
          startedAt: Date.now(),
          lastActivityAt: Date.now(),
          lines: [{ stream: 'Stdout', data: '$ cargo test --workspace\n' }],
          collapsed: false,
          availableForReuse: false,
        },
      ]}
      />,
    );

    await waitFor(() => {
      const blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]') as Array<Record<string, unknown>>;
      const shellBlock = blocks.find((block) => block.kind === 'shell-session');
      expect(shellBlock).toEqual(expect.objectContaining({
        shellSessionId: 'shell-session-recovered',
        state: 'Busy',
        activeProcessId: 'proc-recovered',
        activeCommand: 'cargo test --workspace',
      }));
    });
  });

  it('upgrades a provisional shell block into a shell-session block when shared session state arrives', async () => {
    sendMessageStreamingMock.mockResolvedValue(undefined);
    const { rerender } = render(<Harness shellSessions={[]} />);

    await waitFor(() => {
      expect(screen.getByTestId('status')).toHaveTextContent('Ready');
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({
        type: 'shell-output',
        processId: 'proc-upgrade',
        shellSessionId: null,
        stream: 'Stdout',
        data: 'running...\n',
      });
    });

    let blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]') as Array<Record<string, unknown>>;
    expect(blocks.find((block) => block.kind === 'shell')).toBeTruthy();

    rerender(
      <Harness shellSessions={[
        {
          kind: 'shell-session',
          id: 'shell-session-upgrade',
          shellSessionId: 'shell-session-upgrade',
          cwd: '/workspace',
          state: 'Busy',
          interactive: true,
          userManaged: false,
          activeProcessId: 'proc-upgrade',
          activeCommand: 'cargo check',
          lastExitCode: null,
          durationMs: null,
          startedAt: Date.now(),
          lastActivityAt: Date.now(),
          lines: [{ stream: 'Stdout', data: '$ cargo check\n' }],
          collapsed: true,
          availableForReuse: false,
        },
      ]}
      />,
    );

    await waitFor(() => {
      blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]') as Array<Record<string, unknown>>;
      expect(blocks.find((block) => block.kind === 'shell')).toBeUndefined();
      expect(blocks.find((block) => block.kind === 'shell-session')).toEqual(expect.objectContaining({
        shellSessionId: 'shell-session-upgrade',
        activeProcessId: 'proc-upgrade',
      }));
    });
  });

  it('prebinds a reusable shell session on shell tool start for follow-up requests', async () => {
    sendMessageStreamingMock.mockResolvedValue(undefined);

    render(
      <Harness shellSessions={[
        {
          kind: 'shell-session',
          id: 'shell-session-reuse',
          shellSessionId: 'shell-session-reuse',
          cwd: '/workspace',
          state: 'Idle',
          interactive: true,
          userManaged: false,
          activeProcessId: null,
          activeCommand: null,
          lastExitCode: 0,
          durationMs: 31,
          startedAt: Date.now() - 2_000,
          lastActivityAt: Date.now() - 1_000,
          lines: [{ stream: 'Stdout', data: '$ cargo test --workspace\n' }],
          collapsed: false,
          availableForReuse: true,
        },
      ]}
      />,
    );

    await waitFor(() => {
      expect(screen.getByTestId('status')).toHaveTextContent('Ready');
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'tool-start', toolName: 'shell', toolCallId: 'tool-shell-reuse' });
    });

    await waitFor(() => {
      const blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]') as Array<Record<string, unknown>>;
      expect(blocks.find((block) => block.kind === 'shell-session')).toEqual(expect.objectContaining({
        shellSessionId: 'shell-session-reuse',
        state: 'Starting',
        activeProcessId: null,
      }));
    });
  });

  it('restores the active status after a retry succeeds and streaming resumes', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'status', text: 'Thinking…', kind: 'busy' });
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Thinking…');

    await act(async () => {
      streamDispatch?.({ type: 'retry', attempt: 1, reason: 'temporary provider failure' });
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Retrying (attempt 1)…');

    await act(async () => {
      streamDispatch?.({ type: 'chunk', chunk: 'Recovered output' });
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Thinking…');
    expect(screen.getByTestId('streaming')).toHaveTextContent('Recovered output');
  });

  it('rehydrates interrupted thinking-only messages from session history', async () => {
    getSessionReplaySnapshotMock.mockResolvedValue({
      history: [
        { role: 'assistant', content: '', thinking: 'Working through the answer…', timestamp: '2026-03-15T12:00:00.000Z' },
      ],
      activity_log: [],
      has_paused_execution: true,
    });

    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByTestId('can-resume')).toHaveTextContent('true');
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Interrupted — resume available');
    expect(screen.getByTestId('messages')).toHaveTextContent('thinking:Working through the answer…');
  });

  it('emits one review narration per completed tool result', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'tool-start', toolName: 'shell' });
      streamDispatch?.({ type: 'tool-args', args: '{"command":"cargo check"}' });
      streamDispatch?.({ type: 'tool-result', name: 'shell', success: true, output: 'Finished dev profile', durationMs: 42 });
      streamDispatch?.({ type: 'agent-iteration', iteration: 1 });
      streamDispatch?.({ type: 'agent-iteration', iteration: 2 });
    });

    const streaming = screen.getByTestId('streaming').textContent ?? '';
    expect(streaming).toContain('narration:Checking cargo check:I have the latest command result from "cargo check" in hand');
    expect(streaming.match(/narration:/g)).toHaveLength(1);

    await act(async () => {
      streamDispatch?.({ type: 'tool-start', toolName: 'shell' });
      streamDispatch?.({ type: 'tool-args', args: '{"command":"cargo test"}' });
      streamDispatch?.({ type: 'tool-result', name: 'shell', success: true, output: 'test result: ok', durationMs: 55 });
      streamDispatch?.({ type: 'agent-iteration', iteration: 3 });
    });

    const updatedStreaming = screen.getByTestId('streaming').textContent ?? '';
    expect(updatedStreaming).toContain('narration:Checking cargo test:I have the latest command result from "cargo test" in hand');
    expect(updatedStreaming.match(/narration:/g)).toHaveLength(2);
  });

  it('applies delayed tool results to the correct tool block when a newer tool has already started', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'tool-start', toolName: 'web_search', toolCallId: 'tool-1' });
      streamDispatch?.({ type: 'tool-end', toolCallId: 'tool-1' });
      streamDispatch?.({ type: 'tool-start', toolName: 'file', toolCallId: 'tool-2' });
      streamDispatch?.({ type: 'tool-result', name: 'web_search', success: true, output: 'done', durationMs: 12, toolCallId: 'tool-1' });
    });

    const blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]') as Array<Record<string, unknown>>;
    const firstTool = blocks.find((block) => block['kind'] === 'tool' && block['name'] === 'web_search');
    const secondTool = blocks.find((block) => block['kind'] === 'tool' && block['name'] === 'file');

    expect(firstTool?.['status']).toBe('success');
    expect(firstTool?.['result']).toBe('done');
    expect(secondTool?.['status']).toBe('running');
  });

  it('derives failed shell review titles from command context instead of repeating a generic label', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'tool-start', toolName: 'shell' });
      streamDispatch?.({ type: 'tool-args', args: '{"command":"cargo test --workspace"}' });
      streamDispatch?.({ type: 'tool-result', name: 'shell', success: false, output: 'error: test failed', durationMs: 55 });
      streamDispatch?.({ type: 'agent-iteration', iteration: 1 });
    });

    expect(screen.getByTestId('streaming')).toHaveTextContent(
      'I hit a problem while working through "cargo test --workspace"',
    );
    expect(screen.getByTestId('streaming-narration-titles')).toHaveTextContent('Reviewing cargo test --workspace');
    expect(screen.getByTestId('streaming-narration-titles')).not.toHaveTextContent('Resolving shell issue');
  });

  it('adds the executed shell command to the streamed shell transcript', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({
        type: 'shell-lifecycle',
        processId: 'proc-shell-1',
        payload: {
          process_id: 'proc-shell-1',
          shell_session_id: 'shell-session-1',
          state: 'Started',
          command: 'cargo test --workspace',
          cwd: '/workspace',
        },
      });
      streamDispatch?.({
        type: 'shell-output',
        processId: 'proc-shell-1',
        shellSessionId: 'shell-session-1',
        stream: 'Stdout',
        data: 'running tests...\n',
      });
    });

    const blocks = JSON.parse(screen.getByTestId('streaming-blocks').textContent ?? '[]') as Array<Record<string, unknown>>;
    const shellBlock = blocks.find((block) => block.kind === 'shell-session');
    const lines = Array.isArray(shellBlock?.lines) ? shellBlock.lines as Array<Record<string, unknown>> : [];

    expect(String(lines[0]?.data ?? '')).toContain('cargo test --workspace');
    expect(String(lines[1]?.data ?? '')).toContain('running tests...');
  });

  it('keeps task bookkeeping review narration suppressed so narration stays primary', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'tool-start', toolName: 'task' });
      streamDispatch?.({ type: 'tool-args', args: '{"action":"update_status","task_id":"task-1"}' });
      streamDispatch?.({ type: 'tool-result', name: 'task', success: true, output: '{"ok":true}', durationMs: 18 });
      streamDispatch?.({ type: 'agent-iteration', iteration: 1 });
    });

    expect(screen.getByTestId('streaming')).not.toHaveTextContent('review-fallback');
    expect(screen.getByTestId('streaming')).not.toHaveTextContent('I’m reviewing');
  });

  it('replaces synthetic review narration with the arriving llm narration', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'tool-start', toolName: 'web_search' });
      streamDispatch?.({ type: 'tool-args', args: '{"query":"smart lighting market 2025 consumer drivers"}' });
      streamDispatch?.({ type: 'tool-result', name: 'web_search', success: true, output: 'Found relevant results', durationMs: 21 });
      streamDispatch?.({ type: 'agent-iteration', iteration: 1 });
    });

    expect(screen.getByTestId('streaming')).toHaveTextContent('I’m reading through the research returned for "smart lighting market 2025 consumer drivers"');

    await act(async () => {
      streamDispatch?.({
        type: 'narration',
        title: 'Reviewing research findings',
        message: 'I’ve got a few promising market signals in view now, so I’m sorting out which consumer drivers actually matter for the request before I carry them into the plan.',
        summary: 'The search returned usable market signals.',
        reason: 'That matters because only a subset of the results will actually shape the recommendation.',
        nextStep: 'I’ll fold the strongest signals into the next planning move.',
        evidence: [],
        stage: 'context',
      });
    });

    const streaming = screen.getByTestId('streaming').textContent ?? '';
    expect(streaming).toContain('I’ve got a few promising market signals in view now');
    expect(streaming).not.toContain('I’m reading through the research returned for "smart lighting market 2025 consumer drivers"');
  });

  it('bumps the memory revision when streaming completes and tasks change', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    expect(screen.getByTestId('memory-revision')).toHaveTextContent('0');

    await act(async () => {
      streamDispatch?.({ type: 'done' });
    });

    expect(screen.getByTestId('memory-revision')).toHaveTextContent('1');

    await act(async () => {
      streamDispatch?.({ type: 'task-changed' });
    });

    await waitFor(() => {
      expect(screen.getByTestId('memory-revision')).toHaveTextContent('2');
    });
  });
});