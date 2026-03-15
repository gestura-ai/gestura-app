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

describe('useStreamEvents', () => {
  beforeEach(() => {
    listeners.clear();
    listenMock.mockClear();
  });

  it('refreshes tasks when a task tool result succeeds', async () => {
    const dispatch = vi.fn<(action: StreamEventAction) => void>();
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
    const dispatch = vi.fn<(action: StreamEventAction) => void>();
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
    });
  });
});