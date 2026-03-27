import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
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
  overrides: Partial<ShellSessionRecord> = {},
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
    ...overrides,
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

    expect(screen.getByText('Shell Manager')).toBeInTheDocument();
    expect(screen.getByText('Sessions')).toBeInTheDocument();
    expect(screen.getByText('dev server ready')).toBeInTheDocument();
    expect(screen.getByText('TTY 01')).toBeInTheDocument();
    expect(screen.getByText(/cwd \/workspace\/app/i)).toBeInTheDocument();
    expect(screen.getByText(/proc process-shell-001/i)).toBeInTheDocument();
    expect(screen.queryByText('New Terminal')).not.toBeInTheDocument();

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

  it('keeps the empty sidebar state minimal and uses the header action for new terminals', () => {
    const { container } = render(
      <div>
        <div />
        <ShellManagerPanel
          sessionId="session-123"
          shells={[]}
          activeShellId={null}
          visible
          mode="expanded"
          height={320}
          resizeBoundaryRef={{ current: document.createElement('div') }}
          defaultWorkingDirectory="/workspace"
          onSetMode={vi.fn()}
          onSetHeight={vi.fn()}
          onActivateShell={vi.fn()}
          onReorderShellTabs={vi.fn()}
          onCloseShellTab={vi.fn()}
          onShowToast={vi.fn()}
        />
      </div>,
    );

    const panel = container.querySelector('[aria-label="Terminal workspace"]');
    expect(panel).not.toBeNull();
    const scope = within(panel as HTMLElement);

    expect(scope.getByText('No terminals yet')).toBeInTheDocument();
    expect(scope.queryByText('New Terminal')).not.toBeInTheDocument();
    expect(scope.getByRole('button', { name: 'Start new terminal session' })).toBeInTheDocument();
  });

  it('surfaces a diagnosis when the active shell looks stalled on a prompt', async () => {
    const nowSpy = vi.spyOn(Date, 'now').mockReturnValue(120_000);

    render(
      <div>
        <div />
        <ShellManagerPanel
          sessionId="session-123"
          shells={[
            makeShell(
              'shell-prompt',
              'pnpm add vite',
              '/workspace/app',
              'Need to install the following packages:\nProceed? (y/n)',
              false,
              { lastActivityAt: 10_000 },
            ),
          ]}
          activeShellId="shell-prompt"
          visible
          mode="expanded"
          height={320}
          resizeBoundaryRef={{ current: document.createElement('div') }}
          defaultWorkingDirectory="/workspace"
          onSetMode={vi.fn()}
          onSetHeight={vi.fn()}
          onActivateShell={vi.fn()}
          onReorderShellTabs={vi.fn()}
          onCloseShellTab={vi.fn()}
          onShowToast={vi.fn()}
        />
      </div>,
    );

    const alert = screen.getByText('Likely waiting for input').closest('.shell-dock__terminal-alert');
    expect(alert).not.toBeNull();
    expect(within(alert as HTMLElement).getByText(/Respond in the terminal or close the session and retry/i)).toBeInTheDocument();
    expect(within(alert as HTMLElement).getByText(/Proceed\? \(y\/n\)/i)).toBeInTheDocument();

    nowSpy.mockRestore();
  });
});