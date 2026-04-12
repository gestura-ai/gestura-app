import type { ShellSessionRecord, ShellSessionState } from '../types';
import { buildShellCommandLine } from './shellTranscript';

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
  const lines = [...(existing?.lines ?? []), { stream, data }];
  return lines.length > 4000 ? lines.slice(lines.length - 4000) : lines;
}

function mergeCommandLine(existing: ShellSessionRecord | null, commandLine: ReturnType<typeof buildShellCommandLine>) {
  if (!commandLine) return existing?.lines ?? [];
  const lines = existing?.lines ?? [];
  return lines.some((line) => line.stream === commandLine.stream && line.data === commandLine.data)
    ? lines
    : [commandLine, ...lines];
}

export function normalizeShellSessionState(raw: unknown): ShellSessionState {
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

export function applyShellSessionLifecyclePayload(
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

export function applyShellLifecyclePayload(
  current: ShellSessionRecord[],
  payload: Record<string, unknown>,
  activityAt: number,
): ShellSessionRecord[] {
  const shellSessionId = String(payload['shell_session_id'] ?? '');
  if (!shellSessionId) return current;
  const commandState = normalizeCommandState(payload['state']);

  return upsertShellSession(current, shellSessionId, (matched) => {
    const command = payload['command'] != null ? String(payload['command']) : (matched?.activeCommand ?? '');
    const commandLine = commandState === 'Started' ? buildShellCommandLine(command) : null;

    return {
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
        : command,
      lastExitCode: payload['exit_code'] != null ? Number(payload['exit_code']) : (matched?.lastExitCode ?? null),
      durationMs: payload['duration_ms'] != null ? Number(payload['duration_ms']) : (matched?.durationMs ?? null),
      startedAt: commandState === 'Started' || commandState === 'Running' || commandState === 'Resumed'
        ? matched?.startedAt ?? activityAt
        : matched?.startedAt,
      lastActivityAt: activityAt,
      lines: mergeCommandLine(matched, commandLine),
      collapsed: matched?.collapsed ?? false,
      availableForReuse: commandState === 'Completed' || commandState === 'Failed'
        ? true
        : commandState === 'Stopped'
          ? false
          : (matched?.availableForReuse ?? false),
    };
  });
}

export function applyShellOutputPayload(
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