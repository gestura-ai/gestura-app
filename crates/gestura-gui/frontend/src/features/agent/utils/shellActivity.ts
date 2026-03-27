import type { ShellBlock, ShellSessionRecord } from '../types';

type ActivityTone = 'active' | 'recent' | 'quiet' | 'stalled' | 'idle';

export type ShellStallDiagnosisKind = 'waiting-input' | 'error-output' | 'generic';

export interface ShellStallDiagnosis {
  kind: ShellStallDiagnosisKind;
  label: string;
  detail: string;
  excerpt?: string | null;
}

export interface ShellActivitySummary {
  label: string;
  tone: ActivityTone;
  diagnosis?: ShellStallDiagnosis | null;
}

type RenderableShell = ShellBlock | ShellSessionRecord;

const ACTIVE_THRESHOLD_MS = 5_000;
const RECENT_THRESHOLD_MS = 20_000;
const STALLED_THRESHOLD_MS = 60_000;
const RECENT_LINE_WINDOW = 16;

const INTERACTIVE_PROMPTS = [
  'ok to proceed?',
  'need to install the following packages',
  'would you like to continue',
  'press enter to continue',
  'press any key to continue',
  'select an option',
  '(y/n)',
  '[y/n]',
  'yes/no',
  'confirm',
  'enter passphrase',
  'enter password',
  'password:',
  'passphrase:',
  'choice:',
  'continue?',
];

const ERROR_PATTERNS = [
  'command not found',
  'no such file or directory',
  'permission denied',
  'not recognized as an internal or external command',
  'is not recognized as an internal or external command',
  'npm err!',
  'traceback (most recent call last)',
  'syntax error',
  'failed with exit code',
  'error:',
  'fatal:',
  'panic:',
  'exception:',
];

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

function stripAnsi(value: string): string {
  return value.replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, '');
}

function normalizeOutput(value: string): string {
  return stripAnsi(value)
    .replace(/[\r\n]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

function collectRecentOutput(shell: RenderableShell): Array<{ stream: 'Stdout' | 'Stderr'; text: string; normalized: string }> {
  return shell.lines
    .slice(-RECENT_LINE_WINDOW)
    .map((line) => {
      const text = normalizeOutput(line.data);
      return {
        stream: line.stream,
        text,
        normalized: text.toLowerCase(),
      };
    })
    .filter((line) => line.text.length > 0);
}

function buildExcerpt(lines: Array<{ text: string }>, fallback: string | null = null): string | null {
  const lastLine = lines.length > 0 ? lines[lines.length - 1] : undefined;
  const excerpt = lastLine?.text ?? fallback;
  return excerpt && excerpt.length > 0 ? excerpt.slice(0, 180) : null;
}

function diagnoseStallFromOutput(shell: RenderableShell): ShellStallDiagnosis {
  const recentOutput = collectRecentOutput(shell);
  const recentStderr = recentOutput.filter((line) => line.stream === 'Stderr');
  const interactiveMatch = recentOutput.find((line) =>
    INTERACTIVE_PROMPTS.some((needle) => line.normalized.includes(needle)),
  );

  if (interactiveMatch) {
    return {
      kind: 'waiting-input',
      label: 'Likely waiting for input',
      detail: shell.kind === 'shell-session'
        ? 'Recent terminal output looks interactive. Respond in the terminal or close the session and retry with a non-interactive command.'
        : 'Recent shell output looks interactive. Open the Shell Session Manager to respond, or cancel and retry with a non-interactive command.',
      excerpt: buildExcerpt([interactiveMatch]),
    };
  }

  const errorMatch = recentStderr.find((line) =>
    ERROR_PATTERNS.some((needle) => line.normalized.includes(needle)),
  ) ?? recentOutput.find((line) =>
    ERROR_PATTERNS.some((needle) => line.normalized.includes(needle)),
  );

  if (errorMatch || recentStderr.length > 0) {
    return {
      kind: 'error-output',
      label: 'Recent output suggests an error',
      detail: 'The command appears to have emitted an error before going quiet. Review the latest output, then cancel and retry with a corrected command if needed.',
      excerpt: buildExcerpt(errorMatch ? [errorMatch] : recentStderr, buildExcerpt(recentOutput)),
    };
  }

  return {
    kind: 'generic',
    label: 'No prompt or error detected yet',
    detail: 'The shell has been quiet without a clear interactive prompt or error. Consider checking the live terminal, interrupting it, or retrying with a simpler command.',
    excerpt: buildExcerpt(recentOutput),
  };
}

export function describeShellActivity(shell: RenderableShell, now = Date.now()): ShellActivitySummary {
  const referenceTime = shell.lastActivityAt ?? shell.startedAt ?? null;

  if (isTerminalState(shell)) {
    if (!referenceTime) return { label: 'No recent activity', tone: 'idle', diagnosis: null };
    return { label: `Last active ${formatElapsed(now - referenceTime)} ago`, tone: 'idle', diagnosis: null };
  }

  if (!referenceTime) {
    return { label: 'Awaiting output', tone: 'quiet', diagnosis: null };
  }

  const idleMs = Math.max(0, now - referenceTime);
  if (idleMs <= ACTIVE_THRESHOLD_MS) return { label: 'Active now', tone: 'active', diagnosis: null };
  if (idleMs <= RECENT_THRESHOLD_MS) return { label: `Active ${formatElapsed(idleMs)} ago`, tone: 'recent', diagnosis: null };
  if (idleMs <= STALLED_THRESHOLD_MS) return { label: `Quiet for ${formatElapsed(idleMs)}`, tone: 'quiet', diagnosis: null };
  return {
    label: `Possibly stalled · ${formatElapsed(idleMs)} quiet`,
    tone: 'stalled',
    diagnosis: diagnoseStallFromOutput(shell),
  };
}