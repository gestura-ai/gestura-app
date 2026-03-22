import { useEffect, useState } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import type { UnlistenFn } from '@tauri-apps/api/event';

import { getSessionActivityLog } from '../../../services/tauri/agent';
import type {
  ShellSessionRecord,
  ShellSessionState,
} from '../types';

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

function upsertShellSession(
  shells: ShellSessionRecord[],
  shellSessionId: string,
  updater: (existing: ShellSessionRecord | null) => ShellSessionRecord,
): ShellSessionRecord[] {
  const index = shells.findIndex((shell) => shell.shellSessionId === shellSessionId);
  if (index >= 0) {
    const next = [...shells];
    next[index] = updater(shells[index]);
    return next;
  }

  return [...shells, updater(null)];
}

function appendShellLine(existing: ShellSessionRecord | null, stream: 'Stdout' | 'Stderr', data: string) {
  const lines = [
    ...(existing?.lines ?? []),
    { stream, data },
  ];
  return lines.length > 4000 ? lines.slice(lines.length - 4000) : lines;
}

function normalizeShellSessionState(raw: unknown): ShellSessionState {
  switch (String(raw ?? '').toLowerCase()) {
    case 'starting': return 'Starting';
    case 'idle': return 'Idle';
    case 'busy': return 'Busy';
    case 'interrupting': return 'Interrupting';
    case 'stopping': return 'Stopping';
    case 'stopped': return 'Stopped';
    case 'failed': return 'Failed';
    default: return 'Starting';
  }
}

function normalizeCommandState(raw: unknown): string {
  switch (String(raw ?? '').toLowerCase()) {
    case 'started': return 'Started';
    case 'running': return 'Running';
    case 'paused': return 'Paused';
    case 'resumed': return 'Resumed';
    case 'completed': return 'Completed';
    case 'failed': return 'Failed';
    case 'stopped': return 'Stopped';
    default: return String(raw ?? '');
  }
}

function mapCommandStateToSessionState(
  state: string | null | undefined,
  existing: ShellSessionRecord | null,
): ShellSessionState {
  switch (state) {
    case 'Started':
    case 'Running':
    case 'Paused':
    case 'Resumed':
      return 'Busy';
    case 'Completed':
    case 'Failed':
      return existing?.state === 'Stopping' || existing?.state === 'Stopped'
        ? existing.state
        : 'Idle';
    case 'Stopped':
      return 'Stopped';
    default:
      return existing?.state ?? 'Starting';
  }
}

function applyShellSessionLifecyclePayload(
  current: ShellSessionRecord[],
  payload: Record<string, unknown>,
  activityAt: number,
): ShellSessionRecord[] {
  const shellSessionId = String(payload['shell_session_id'] ?? '');
  if (!shellSessionId) return current;

  return upsertShellSession(current, shellSessionId, (matched) => ({
    kind: 'shell-session',
    id: matched?.id ?? shellSessionId,
    shellSessionId,
    cwd: payload['cwd'] != null ? String(payload['cwd']) : (matched?.cwd ?? null),
    state: payload['state'] != null
      ? normalizeShellSessionState(payload['state'])
      : (matched?.state ?? 'Starting'),
    interactive: payload['interactive'] != null
      ? Boolean(payload['interactive'])
      : (matched?.interactive ?? false),
    userManaged: payload['user_managed'] != null
      ? Boolean(payload['user_managed'])
      : (matched?.userManaged ?? false),
    activeProcessId: payload['active_process_id'] != null
      ? String(payload['active_process_id'])
      : (matched?.activeProcessId ?? null),
    activeCommand: payload['active_command'] != null
      ? String(payload['active_command'])
      : (matched?.activeCommand ?? null),
    lastExitCode: matched?.lastExitCode ?? null,
    durationMs: matched?.durationMs ?? null,
    startedAt: matched?.startedAt ?? (payload['state'] != null && ['Busy', 'Starting'].includes(normalizeShellSessionState(payload['state']))
      ? activityAt
      : null),
    lastActivityAt: activityAt,
    lines: matched?.lines ?? [],
    collapsed: matched?.collapsed ?? false,
    availableForReuse: payload['available_for_reuse'] != null
      ? Boolean(payload['available_for_reuse'])
      : (matched?.availableForReuse ?? false),
  }));
}

function applyShellLifecyclePayload(
  current: ShellSessionRecord[],
  payload: Record<string, unknown>,
  activityAt: number,
): ShellSessionRecord[] {
  const shellSessionId = String(payload['shell_session_id'] ?? '');
  if (!shellSessionId) return current;
  const commandState = normalizeCommandState(payload['state']);

  return upsertShellSession(current, shellSessionId, (matched) => ({
    kind: 'shell-session',
    id: matched?.id ?? shellSessionId,
    shellSessionId,
    cwd: payload['cwd'] != null ? String(payload['cwd']) : (matched?.cwd ?? null),
    state: mapCommandStateToSessionState(commandState, matched),
    interactive: matched?.interactive ?? true,
    userManaged: matched?.userManaged ?? false,
    activeProcessId: commandState === 'Completed' || commandState === 'Failed' || commandState === 'Stopped'
      ? null
      : String(payload['process_id'] ?? matched?.activeProcessId ?? ''),
    activeCommand: commandState === 'Completed' || commandState === 'Failed' || commandState === 'Stopped'
      ? null
      : String(payload['command'] ?? matched?.activeCommand ?? ''),
    lastExitCode: payload['exit_code'] != null ? Number(payload['exit_code']) : (matched?.lastExitCode ?? null),
    durationMs: payload['duration_ms'] != null ? Number(payload['duration_ms']) : (matched?.durationMs ?? null),
    startedAt: commandState === 'Started' || commandState === 'Running' || commandState === 'Resumed'
      ? matched?.startedAt ?? activityAt
      : matched?.startedAt,
    lastActivityAt: activityAt,
    lines: matched?.lines ?? [],
    collapsed: matched?.collapsed ?? false,
    availableForReuse: commandState === 'Completed' || commandState === 'Failed'
      ? true
      : commandState === 'Stopped'
        ? false
        : (matched?.availableForReuse ?? false),
  }));
}

function applyShellOutputPayload(
  current: ShellSessionRecord[],
  payload: Record<string, unknown>,
  activityAt: number,
): ShellSessionRecord[] {
  const shellSessionId = String(payload['shell_session_id'] ?? '');
  if (!shellSessionId) return current;

  return upsertShellSession(current, shellSessionId, (matched) => ({
    kind: 'shell-session',
    id: matched?.id ?? shellSessionId,
    shellSessionId,
    cwd: matched?.cwd ?? null,
    state: matched?.state ?? 'Idle',
    interactive: matched?.interactive ?? true,
    userManaged: matched?.userManaged ?? false,
    activeProcessId: matched?.activeProcessId ?? null,
    activeCommand: matched?.activeCommand ?? null,
    lastExitCode: matched?.lastExitCode ?? null,
    durationMs: matched?.durationMs ?? null,
    startedAt: matched?.startedAt ?? activityAt,
    lastActivityAt: activityAt,
    collapsed: matched?.collapsed ?? false,
    availableForReuse: matched?.availableForReuse ?? false,
    lines: appendShellLine(
      matched,
      (payload['stream'] as 'Stdout' | 'Stderr') ?? 'Stdout',
      String(payload['data'] ?? ''),
    ),
  }));
}

/**
 * Tracks long-lived shell sessions for the active agent session independently
 * from chat message rendering. The Shell Manager renders session-level state,
 * while chat retains command-level shell blocks.
 */
export function useShellSessions(sessionId: string): ShellSessionRecord[] {
  const [shells, setShells] = useState<ShellSessionRecord[]>([]);
  const markActivity = () => Date.now();

  useEffect(() => {
    setShells([]);
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
      const activityLog = await getSessionActivityLog(sessionId).catch(() => []);
      if (!cancelled && activityLog.length > 0) {
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
        setShells(restored);
      }

      await safeListen('agent-stream-shell-session-lifecycle', (event) => {
        const payload = accept<Record<string, unknown>>(event.payload);
        if (!payload) return;
        const activityAt = markActivity();

        setShells((current) => applyShellSessionLifecyclePayload(current, payload, activityAt));
      });

      await safeListen('agent-stream-shell-lifecycle', (event) => {
        const payload = accept<Record<string, unknown>>(event.payload);
        if (!payload) return;
        const activityAt = markActivity();

        setShells((current) => applyShellLifecyclePayload(current, payload, activityAt));
      });

      await safeListen('agent-stream-shell-output', (event) => {
        const payload = accept<Record<string, unknown>>(event.payload);
        if (!payload) return;
        const activityAt = markActivity();

        setShells((current) => applyShellOutputPayload(current, payload, activityAt));
      });
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
  }, [sessionId]);

  return shells;
}

export default useShellSessions;