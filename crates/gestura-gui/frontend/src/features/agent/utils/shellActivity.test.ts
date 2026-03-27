import { describe, expect, it } from 'vitest';

import { describeShellActivity } from './shellActivity';

describe('describeShellActivity', () => {
  it('marks running shells with recent activity as active', () => {
    expect(
      describeShellActivity({
        kind: 'shell',
        id: 'shell-1',
        processId: 'proc-1',
        command: 'cargo test',
        cwd: '/workspace',
        state: 'Running',
        lastActivityAt: 9_000,
        lines: [],
        collapsed: false,
      }, 10_000),
    ).toEqual({ label: 'Active now', tone: 'active', diagnosis: null });
  });

  it('marks long-quiet running shells as possibly stalled', () => {
    expect(
      describeShellActivity({
        kind: 'shell-session',
        id: 'shell-2',
        shellSessionId: 'shell-2',
        cwd: '/workspace',
        state: 'Busy',
        interactive: true,
        userManaged: false,
        activeProcessId: 'proc-2',
        activeCommand: 'npm run build',
        lastExitCode: null,
        durationMs: null,
        startedAt: 1_000,
        lastActivityAt: 20_000,
        lines: [],
        collapsed: false,
        availableForReuse: false,
      }, 90_000),
    ).toEqual({
      label: 'Possibly stalled · 1m quiet',
      tone: 'stalled',
      diagnosis: {
        kind: 'generic',
        label: 'No prompt or error detected yet',
        detail: 'The shell has been quiet without a clear interactive prompt or error. Consider checking the live terminal, interrupting it, or retrying with a simpler command.',
        excerpt: null,
      },
    });
  });

  it('recognizes stalled shells that are waiting for user input', () => {
    expect(
      describeShellActivity({
        kind: 'shell-session',
        id: 'shell-3',
        shellSessionId: 'shell-3',
        cwd: '/workspace',
        state: 'Busy',
        interactive: true,
        userManaged: false,
        activeProcessId: 'proc-3',
        activeCommand: 'pnpm add vite',
        lastExitCode: null,
        durationMs: null,
        startedAt: 1_000,
        lastActivityAt: 15_000,
        lines: [{ stream: 'Stdout', data: 'Need to install the following packages:\nProceed? (y/n)' }],
        collapsed: false,
        availableForReuse: false,
      }, 90_000),
    ).toMatchObject({
      tone: 'stalled',
      diagnosis: {
        kind: 'waiting-input',
        label: 'Likely waiting for input',
      },
    });
  });

  it('recognizes stalled shells that likely errored before going quiet', () => {
    expect(
      describeShellActivity({
        kind: 'shell',
        id: 'shell-4',
        processId: 'proc-4',
        shellSessionId: 'shell-session-4',
        command: 'cargo tset',
        cwd: '/workspace',
        state: 'Running',
        lastActivityAt: 10_000,
        lines: [{ stream: 'Stderr', data: 'error: no such command: `tset`' }],
        collapsed: false,
      }, 90_000),
    ).toMatchObject({
      tone: 'stalled',
      diagnosis: {
        kind: 'error-output',
        label: 'Recent output suggests an error',
        excerpt: 'error: no such command: `tset`',
      },
    });
  });
});