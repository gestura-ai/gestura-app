import type { ShellBlock, ShellSessionRecord } from '../types';

type ActivityTone = 'active' | 'recent' | 'quiet' | 'stalled' | 'idle';

export interface ShellActivitySummary {
  label: string;
  tone: ActivityTone;
}

type RenderableShell = ShellBlock | ShellSessionRecord;

const ACTIVE_THRESHOLD_MS = 5_000;
const RECENT_THRESHOLD_MS = 20_000;
const STALLED_THRESHOLD_MS = 60_000;

function formatElapsed(ms: number): string {
  const seconds = Math.max(1, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  return `${minutes}m`;
}

function isTerminalState(shell: RenderableShell): boolean {
  if (shell.kind === 'shell-session') {
    return shell.state === 'Idle' || shell.state === 'Stopped' || shell.state === 'Failed';
  }

  return shell.state === 'Completed' || shell.state === 'Failed' || shell.state === 'Stopped';
}

export function describeShellActivity(shell: RenderableShell, now = Date.now()): ShellActivitySummary {
  const referenceTime = shell.lastActivityAt ?? shell.startedAt ?? null;

  if (isTerminalState(shell)) {
    if (!referenceTime) return { label: 'No recent activity', tone: 'idle' };
    return { label: `Last active ${formatElapsed(now - referenceTime)} ago`, tone: 'idle' };
  }

  if (!referenceTime) {
    return { label: 'Awaiting output', tone: 'quiet' };
  }

  const idleMs = Math.max(0, now - referenceTime);
  if (idleMs <= ACTIVE_THRESHOLD_MS) return { label: 'Active now', tone: 'active' };
  if (idleMs <= RECENT_THRESHOLD_MS) return { label: `Active ${formatElapsed(idleMs)} ago`, tone: 'recent' };
  if (idleMs <= STALLED_THRESHOLD_MS) return { label: `Quiet for ${formatElapsed(idleMs)}`, tone: 'quiet' };
  return { label: `Possibly stalled · ${formatElapsed(idleMs)} quiet`, tone: 'stalled' };
}