import { act, render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  useStreamEvents,
  type StreamEventAction,
  type StreamEventDispatch,
} from './useStreamEvents';

type TauriListener = (event: { payload: unknown }) => void;

const listeners = new Map<string, TauriListener>();
const listenMock = vi.fn(async (eventName: string, handler: TauriListener) => {
  listeners.set(eventName, handler);
  return vi.fn();
});

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    listen: listenMock,
  }),
}));

function HookHarness(props: { sessionId: string; dispatch: StreamEventDispatch }) {
  useStreamEvents(props.sessionId, props.dispatch);
  return null;
}

function createDispatchMock() {
  return vi.fn((action: StreamEventAction) => {
    void action;
  });
}

describe('useStreamEvents', () => {
  beforeEach(() => {
    listeners.clear();
    listenMock.mockClear();
    if (!window.requestAnimationFrame) {
      window.requestAnimationFrame = (callback: FrameRequestCallback): number => {
        return window.setTimeout(() => callback(performance.now()), 0);
      };
    }
    if (!window.cancelAnimationFrame) {
      window.cancelAnimationFrame = (handle: number) => {
        window.clearTimeout(handle);
      };
    }
  });

  it('refreshes tasks when a task tool result succeeds', async () => {
    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-tool-result')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-stream-tool-result')?.({
        payload: { session_id: 'session-123', name: 'task', success: true, output: 'ok' },
      });
    });

    expect(dispatch.mock.calls.map(([action]) => action.type)).toEqual([
      'task-changed',
      'tool-result',
    ]);
  });

  it('does not refresh tasks for failed task tool results', async () => {
    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-tool-result')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-stream-tool-result')?.({
        payload: { session_id: 'session-123', name: 'task', success: false, output: 'nope' },
      });
    });

    expect(dispatch).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenCalledWith({
      type: 'tool-result',
      name: 'task',
      success: false,
      output: 'nope',
      durationMs: null,
      toolCallId: null,
    });
  });

  it('dispatches paused and resumed stream lifecycle events', async () => {
    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-paused')).toBe(true);
      expect(listeners.has('agent-stream-resumed')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-stream-paused')?.({ payload: { session_id: 'session-123' } });
      listeners.get('agent-stream-resumed')?.({ payload: { session_id: 'session-123' } });
    });

    expect(dispatch.mock.calls.map(([action]) => action.type)).toContain('paused');
    expect(dispatch.mock.calls.map(([action]) => action.type)).toContain('resumed');
  });

  it('dispatches runtime task snapshots from the stream', async () => {
    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-task-state')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-stream-task-state')?.({
        payload: {
          session_id: 'session-123',
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
        },
      });
    });

    await waitFor(() => {
      expect(dispatch).toHaveBeenCalledWith({
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
  });

  it('coalesces adjacent chunk events before dispatching', async () => {
    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-chunk')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-stream-chunk')?.({ payload: { session_id: 'session-123', value: 'hello ' } });
      listeners.get('agent-stream-chunk')?.({ payload: { session_id: 'session-123', value: 'world' } });
    });

    await waitFor(() => {
      expect(dispatch).toHaveBeenCalledWith({ type: 'chunk', chunk: 'hello world' });
    });
    expect(dispatch).toHaveBeenCalledTimes(1);
  });

  it('flushes shell output without waiting for animation-frame batching', async () => {
    const dispatch = createDispatchMock();
    const requestAnimationFrameMock = vi.fn(() => 123);
    window.requestAnimationFrame = requestAnimationFrameMock;
    window.cancelAnimationFrame = vi.fn();

    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-shell-output')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-stream-shell-output')?.({
        payload: {
          session_id: 'session-123',
          process_id: 'shell-proc-1',
          stream: 'Stdout',
          data: 'progress\r',
        },
      });
      await Promise.resolve();
    });

    expect(dispatch).toHaveBeenCalledWith({
      type: 'shell-output',
      processId: 'shell-proc-1',
      shellSessionId: null,
      stream: 'Stdout',
      data: 'progress\r',
    });
    expect(requestAnimationFrameMock).not.toHaveBeenCalled();
  });

  it('preserves shell session ids on output events for output-first hydration', async () => {
    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-shell-output')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-stream-shell-output')?.({
        payload: {
          session_id: 'session-123',
          process_id: 'shell-proc-1',
          shell_session_id: 'shell-session-1',
          stream: 'Stdout',
          data: 'progress\r',
        },
      });
      await Promise.resolve();
    });

    expect(dispatch).toHaveBeenCalledWith({
      type: 'shell-output',
      processId: 'shell-proc-1',
      shellSessionId: 'shell-session-1',
      stream: 'Stdout',
      data: 'progress\r',
    });
  });

  it('dispatches shell session lifecycle events to chat state immediately', async () => {
    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-shell-session-lifecycle')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-stream-shell-session-lifecycle')?.({
        payload: {
          session_id: 'session-123',
          shell_session_id: 'shell-session-1',
          state: 'Busy',
          active_process_id: 'proc-1',
          active_command: 'cargo test',
        },
      });
    });

    expect(dispatch).toHaveBeenCalledWith({
      type: 'shell-session-lifecycle',
      shellSessionId: 'shell-session-1',
      payload: {
        session_id: 'session-123',
        shell_session_id: 'shell-session-1',
        state: 'Busy',
        active_process_id: 'proc-1',
        active_command: 'cargo test',
      },
    });
  });

  it('registers shell listeners even if earlier listeners are still pending', async () => {
    const resolveProbeRef: { current: null | (() => void) } = { current: null };
    listenMock.mockImplementationOnce(() => new Promise((resolve) => {
      resolveProbeRef.current = () => resolve(vi.fn());
    }));

    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-stream-shell-output')).toBe(true);
    });

    if (resolveProbeRef.current) {
      resolveProbeRef.current();
    }
    await Promise.resolve();
  });

  it('normalizes backend voice agent-message payloads into chat actions', async () => {
    const dispatch = createDispatchMock();
    render(<HookHarness sessionId="session-123" dispatch={dispatch} />);

    await waitFor(() => {
      expect(listeners.has('agent-message')).toBe(true);
    });

    await act(async () => {
      listeners.get('agent-message')?.({
        payload: {
          session_id: 'session-123',
          type: 'user',
          message: 'Transcribed voice prompt',
        },
      });
    });

    expect(dispatch).toHaveBeenCalledWith({
      type: 'agent-message',
      role: 'user',
      content: 'Transcribed voice prompt',
    });
  });
});