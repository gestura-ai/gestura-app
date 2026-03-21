import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useMemo, useRef, useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import type { ShellSessionRecord } from '../types';
import { ShellManagerPanel } from './ShellManagerPanel';

const startShellSessionStreamingMock = vi.fn().mockResolvedValue(undefined);
const shellSessionStopMock = vi.fn().mockResolvedValue(undefined);
const shellSessionAttachMock = vi.fn().mockResolvedValue(undefined);

vi.mock('../../../services/tauri/agent', () => ({
  startShellSessionStreaming: (...args: unknown[]) => startShellSessionStreamingMock(...args),
  shellSessionStop: (...args: unknown[]) => shellSessionStopMock(...args),
  shellSessionAttach: (...args: unknown[]) => shellSessionAttachMock(...args),
}));

vi.mock('./InteractiveShellTerminal', () => ({
  InteractiveShellTerminal: ({ shell }: { shell: ShellSessionRecord }) => (
    <div data-testid="interactive-shell-terminal">{shell.lines.map((line) => line.data).join('')}</div>
  ),
}));

function makeShell(
  shellSessionId: string,
  command: string,
  cwd: string,
  output: string,
  userManaged = true,
): ShellSessionRecord {
  return {
    kind: 'shell-session',
    id: shellSessionId,
    shellSessionId,
    cwd,
    state: 'Busy',
    interactive: true,
    userManaged,
    activeProcessId: `process-${shellSessionId}`,
    activeCommand: command,
    lastExitCode: null,
    durationMs: null,
    collapsed: false,
    availableForReuse: false,
    lines: [{ stream: 'Stdout', data: output }],
  };
}

function ShellManagerHarness() {
  const [shells, setShells] = useState<ShellSessionRecord[]>([
    makeShell('shell-001', 'npm run dev', '/workspace/app', 'dev server ready'),
    makeShell('shell-002', 'cargo test', '/workspace/core', 'test suite running', false),
  ]);
  const [activeShellId, setActiveShellId] = useState<string | null>('shell-001');
  const boundaryRef = useRef<HTMLDivElement>(null);

  const orderedShells = useMemo(() => shells, [shells]);

  return (
    <div>
      <div ref={boundaryRef} />
      <ShellManagerPanel
        sessionId="session-123"
        shells={orderedShells}
        activeShellId={activeShellId}
        visible
        mode="expanded"
        height={320}
        resizeBoundaryRef={boundaryRef}
        defaultWorkingDirectory="/workspace"
        onSetMode={vi.fn()}
        onSetHeight={vi.fn()}
        onActivateShell={setActiveShellId}
        onReorderShellTabs={vi.fn()}
        onCloseShellTab={(shellId) => setShells((current) => current.filter((shell) => shell.shellSessionId !== shellId))}
        onShowToast={vi.fn()}
      />
    </div>
  );
}

describe('ShellManagerPanel', () => {
  it('starts and renders agent sessions like regular interactive terminals', async () => {
    render(<ShellManagerHarness />);

    expect(screen.getByText('Shell Session Manager')).toBeInTheDocument();
    expect(screen.getByText('Sessions')).toBeInTheDocument();
    expect(screen.getByText('dev server ready')).toBeInTheDocument();
    expect(screen.getByText('TTY 01')).toBeInTheDocument();
    expect(screen.getByText('/workspace/app')).toBeInTheDocument();
    expect(screen.getByText('proc')).toBeInTheDocument();
    expect(screen.getByText('proc process-shell-001')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Start new terminal session' }));

    await waitFor(() => {
      expect(startShellSessionStreamingMock).toHaveBeenCalledWith('session-123');
    });

    fireEvent.click(screen.getByText('cargo'));
    await waitFor(() => {
      expect(shellSessionAttachMock).toHaveBeenCalledWith('session-123', 'shell-002');
    });
    expect(screen.queryByRole('button', { name: /continue session/i })).not.toBeInTheDocument();
    expect(screen.getAllByText('cargo test').length).toBeGreaterThan(0);
    expect(screen.queryByText('/workspace/core')).not.toBeInTheDocument();
    expect(screen.getByTestId('interactive-shell-terminal')).toHaveTextContent('test suite running');
    expect(screen.getByText('test suite running')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close active cargo' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Close active cargo' }));
    await waitFor(() => {
      expect(shellSessionStopMock).toHaveBeenCalledWith('shell-002');
    });
    expect(screen.queryByText('cargo test')).not.toBeInTheDocument();
  });
});