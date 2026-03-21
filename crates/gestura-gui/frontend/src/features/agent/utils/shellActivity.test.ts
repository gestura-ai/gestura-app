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
    ).toEqual({ label: 'Active now', tone: 'active' });
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
    ).toEqual({ label: 'Possibly stalled · 1m quiet', tone: 'stalled' });
  });
});