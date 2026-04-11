import { useCallback, useEffect, useRef, useState } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import type { UnlistenFn } from '@tauri-apps/api/event';

import { getSessionActivityLog } from '../../../services/tauri/agent';
import type {
  ShellSessionRecord,
} from '../types';
import {
  applyShellLifecyclePayload,
  applyShellOutputPayload,
  applyShellSessionLifecyclePayload,
} from '../utils/shellSessionState';

interface UnpackedPayload<T = unknown> {
  incomingSessionId: string | null;
  value: T;
}

function unpackPayload<T = unknown>(raw: unknown): UnpackedPayload<T> {
  if (!raw || typeof raw !== 'object') {
    return { incomingSessionId: null, value: raw as T };
  }

  const obj = raw as Record<string, unknown>;
  return {
    incomingSessionId: (obj['session_id'] as string | null | undefined) ?? null,
    value: (obj['value'] !== undefined ? obj['value'] : raw) as T,
  };
}

/**
 * Tracks long-lived shell sessions for the active agent session independently
 * from the shell manager UI. Chat can also surface the same session-backed
 * state inline when shell streams are tied to a reusable terminal session.
 */
export function useShellSessions(
  sessionId: string,
  options: { restoreHistory?: boolean } = {},
): ShellSessionRecord[] {
  const [shellState, setShellState] = useState<{ sessionId: string; shells: ShellSessionRecord[] }>({
    sessionId,
    shells: [],
  });
  const historyHydratedSessionRef = useRef<string | null>(null);
  const markActivity = useCallback(() => Date.now(), []);
  const { restoreHistory = true } = options;
  const shells = shellState.sessionId === sessionId ? shellState.shells : [];
  const updateShells = useCallback((updater: (current: ShellSessionRecord[]) => ShellSessionRecord[]) => {
    setShellState((current) => ({
      sessionId,
      shells: updater(current.sessionId === sessionId ? current.shells : []),
    }));
  }, [sessionId]);

  useEffect(() => {
    historyHydratedSessionRef.current = null;
    const win = getCurrentWebviewWindow();
    const unlisten: UnlistenFn[] = [];
    let cancelled = false;

    function accept<T = unknown>(raw: unknown): T | null {
      const { incomingSessionId, value } = unpackPayload<T>(raw);
      if (sessionId && (!incomingSessionId || incomingSessionId !== sessionId)) {
        return null;
      }

      return value;
    }

    async function safeListen(
      eventName: string,
      handler: Parameters<typeof win.listen>[1],
    ): Promise<void> {
      if (cancelled) return;
      const fn = await win.listen(eventName, handler);
      if (cancelled) {
        try {
          fn();
        } catch {
          // best effort
        }
        return;
      }
      unlisten.push(fn);
    }

    async function setup(): Promise<void> {
      await Promise.all([
        safeListen('agent-stream-shell-session-lifecycle', (event) => {
          const payload = accept<Record<string, unknown>>(event.payload);
          if (!payload) return;
          const activityAt = markActivity();

          updateShells((current) => applyShellSessionLifecyclePayload(current, payload, activityAt));
        }),
        safeListen('agent-stream-shell-lifecycle', (event) => {
          const payload = accept<Record<string, unknown>>(event.payload);
          if (!payload) return;
          const activityAt = markActivity();

          updateShells((current) => applyShellLifecyclePayload(current, payload, activityAt));
        }),
        safeListen('agent-stream-shell-output', (event) => {
          const payload = accept<Record<string, unknown>>(event.payload);
          if (!payload) return;
          const activityAt = markActivity();

          updateShells((current) => applyShellOutputPayload(current, payload, activityAt));
        }),
      ]);
    }

    void setup();

    return () => {
      cancelled = true;
      unlisten.forEach((fn) => {
        try {
          fn();
        } catch {
          // best effort
        }
      });
    };
  }, [markActivity, sessionId, updateShells]);

  useEffect(() => {
    if (!restoreHistory || historyHydratedSessionRef.current === sessionId) {
      return;
    }

    let cancelled = false;

    void getSessionActivityLog(sessionId)
      .then((activityLog) => {
        if (cancelled) return;
        historyHydratedSessionRef.current = sessionId;
        if (activityLog.length === 0) return;

        const restored = activityLog.reduce<ShellSessionRecord[]>((current, entry) => {
          const { incomingSessionId, value } = unpackPayload<Record<string, unknown>>(entry.payload);
          if (sessionId && incomingSessionId && incomingSessionId !== sessionId) {
            return current;
          }
          if (!value || typeof value !== 'object') {
            return current;
          }

          const timestamp = Date.parse(entry.timestamp);
          const activityAt = Number.isFinite(timestamp) ? timestamp : markActivity();

          switch (entry.event_type) {
            case 'agent-stream-shell-session-lifecycle':
              return applyShellSessionLifecyclePayload(current, value, activityAt);
            case 'agent-stream-shell-lifecycle':
              return applyShellLifecyclePayload(current, value, activityAt);
            case 'agent-stream-shell-output':
              return applyShellOutputPayload(current, value, activityAt);
            default:
              return current;
          }
        }, []);

        updateShells((current) => (current.length > 0 ? current : restored));
      })
      .catch(() => {
        historyHydratedSessionRef.current = sessionId;
      });

    return () => {
      cancelled = true;
    };
  }, [markActivity, restoreHistory, sessionId, updateShells]);

  return shells;
}

export default useShellSessions;