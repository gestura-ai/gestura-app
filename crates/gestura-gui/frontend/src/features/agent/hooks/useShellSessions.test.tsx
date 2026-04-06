import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const listeners = new Map<string, Array<(event: { payload: unknown }) => void>>();
const listenMock = vi.fn(async (eventName: string, handler: (event: { payload: unknown }) => void) => {
  listeners.set(eventName, [...(listeners.get(eventName) ?? []), handler]);
  return () => undefined;
});

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({ listen: listenMock }),
}));

const getSessionActivityLogMock = vi.fn();

vi.mock('../../../services/tauri/agent', () => ({
  getSessionActivityLog: (...args: unknown[]) => getSessionActivityLogMock(...args),
}));

import { useShellSessions } from './useShellSessions';

function emit(eventName: string, payload: unknown) {
  for (const handler of listeners.get(eventName) ?? []) {
    handler({ payload });
  }
}

describe('useShellSessions', () => {
  beforeEach(() => {
    listeners.clear();
    listenMock.mockClear();
    vi.restoreAllMocks();
    getSessionActivityLogMock.mockReset();
    getSessionActivityLogMock.mockResolvedValue([]);
  });

  it('tracks shell activity timestamps from lifecycle and output events', async () => {
    let now = 1_000;
    vi.spyOn(Date, 'now').mockImplementation(() => now);

    const { result } = renderHook(() => useShellSessions('session-1'));
    await waitFor(() => expect(listenMock).toHaveBeenCalled());

    act(() => {
      emit('agent-stream-shell-lifecycle', {
        session_id: 'session-1',
        value: {
          shell_session_id: 'shell-1',
          state: 'Started',
          command: 'cargo test',
        },
      });
    });

    expect(result.current[0]?.startedAt).toBe(1_000);
    expect(result.current[0]?.lastActivityAt).toBe(1_000);
    expect(result.current[0]?.lines[0]?.data).toContain('cargo test');

    now = 6_000;
    act(() => {
      emit('agent-stream-shell-output', {
        session_id: 'session-1',
        value: {
          shell_session_id: 'shell-1',
          stream: 'Stdout',
          data: 'running...',
        },
      });
    });

    expect(result.current[0]?.lastActivityAt).toBe(6_000);
    expect(result.current[0]?.lines[result.current[0].lines.length - 1]?.data).toBe('running...');
  });

  it('defers history hydration until explicitly enabled', async () => {
    const { rerender } = renderHook(
      ({ restoreHistory }) => useShellSessions('session-1', { restoreHistory }),
      { initialProps: { restoreHistory: false } },
    );

    await waitFor(() => expect(listenMock).toHaveBeenCalled());
    expect(getSessionActivityLogMock).not.toHaveBeenCalled();

    rerender({ restoreHistory: true });

    await waitFor(() => expect(getSessionActivityLogMock).toHaveBeenCalledWith('session-1'));
  });

  it('registers shell output listeners even if the first shell listener is still pending', async () => {
    const resolveFirstListenerRef: { current: null | (() => void) } = { current: null };
    listenMock.mockImplementationOnce(() => new Promise((resolve) => {
      resolveFirstListenerRef.current = () => resolve(() => undefined);
    }));

    renderHook(() => useShellSessions('session-1'));

    await waitFor(() => {
      expect((listeners.get('agent-stream-shell-output') ?? []).length).toBeGreaterThan(0);
    });

    if (resolveFirstListenerRef.current) {
      resolveFirstListenerRef.current();
    }
    await Promise.resolve();
  });
});
