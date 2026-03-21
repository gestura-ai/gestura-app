import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  shellSessionAttach,
  shellSessionStop,
  startShellSessionStreaming,
} from '../../../services/tauri/agent';
import type { ShellSessionRecord } from '../types';
import type { ShellPanelMode } from '../hooks/usePanelState';
import { describeShellActivity } from '../utils/shellActivity';
import { InteractiveShellTerminal } from './InteractiveShellTerminal';

interface ShellManagerPanelProps {
  sessionId: string;
  shells: ShellSessionRecord[];
  activeShellId: string | null;
  visible: boolean;
  mode: ShellPanelMode;
  height: number;
  resizeBoundaryRef: React.RefObject<HTMLDivElement>;
  defaultWorkingDirectory?: string | null;
  onSetMode: (mode: ShellPanelMode) => void;
  onSetHeight: (height: number) => void;
  onActivateShell: (shellId: string) => void;
  onReorderShellTabs: (sourceId: string, targetId: string) => void;
  onCloseShellTab: (shellId: string) => void;
  onShowToast: (message: string, kind?: 'success' | 'error') => void;
}

function sessionLabel(shell: ShellSessionRecord, index: number): string {
  const commandName = shell.activeCommand?.trim().split(/\s+/)[0];
  return commandName ? commandName : `Terminal ${index + 1}`;
}

function sessionStateLabel(shell: ShellSessionRecord): string {
  switch (shell.state) {
    case 'Busy': return 'Running';
    case 'Interrupting': return 'Interrupting';
    case 'Stopping': return 'Stopping';
    case 'Stopped': return 'Stopped';
    case 'Failed': return 'Failed';
    case 'Starting': return 'Starting';
    case 'Idle':
    default:
      return 'Ready';
  }
}

function sessionOwnerHeaderLabel(shell: ShellSessionRecord): string {
  return shell.userManaged ? 'Attached to you' : 'Agent-owned';
}

function sessionListOwnerLabel(shell: ShellSessionRecord): string {
  return shell.userManaged ? 'You' : 'Agent';
}

function sessionCommandLabel(shell: ShellSessionRecord): string {
  return shell.activeCommand?.trim() || (shell.userManaged ? 'interactive shell ready' : 'agent shell ready');
}

function sessionWorkingDirectory(shell: ShellSessionRecord, defaultWorkingDirectory?: string | null): string {
  return shell.cwd || defaultWorkingDirectory || 'Project workspace';
}

function sessionSlotLabel(index: number): string {
  return `TTY ${String(index + 1).padStart(2, '0')}`;
}

function sessionProcessLabel(shell: ShellSessionRecord): string | null {
  return shell.activeProcessId ? `proc ${shell.activeProcessId}` : null;
}

const PlusIcon = () => (
  <svg viewBox="0 0 16 16" aria-hidden="true">
    <path d="M8 3.25v9.5M3.25 8h9.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.4" />
  </svg>
);

const TerminalIcon = () => (
  <svg viewBox="0 0 16 16" aria-hidden="true">
    <path d="M2.75 4.25h10.5a1 1 0 0 1 1 1v5.5a1 1 0 0 1-1 1H2.75a1 1 0 0 1-1-1v-5.5a1 1 0 0 1 1-1Z" fill="none" stroke="currentColor" strokeWidth="1.2" />
    <path d="m5 6.4 1.8 1.6L5 9.6" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.2" />
    <path d="M8.7 9.8h2.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.2" />
  </svg>
);

export const ShellManagerPanel: React.FC<ShellManagerPanelProps> = ({
  sessionId,
  shells,
  activeShellId,
  visible,
  mode,
  height,
  resizeBoundaryRef,
  defaultWorkingDirectory,
  onSetMode,
  onSetHeight,
  onActivateShell,
  onReorderShellTabs,
  onCloseShellTab,
  onShowToast,
}) => {
  const [draggedShellId, setDraggedShellId] = useState<string | null>(null);
  const [isStartingShell, setIsStartingShell] = useState(false);
  const [activityNow, setActivityNow] = useState(() => Date.now());
  const autoAttachedShellIdRef = useRef<string | null>(null);

  const activeShell = useMemo(
    () => shells.find((shell) => shell.shellSessionId === activeShellId) ?? shells[0] ?? null,
    [activeShellId, shells],
  );
  const activeShellIndex = useMemo(
    () => (activeShell ? shells.findIndex((shell) => shell.shellSessionId === activeShell.shellSessionId) : -1),
    [activeShell, shells],
  );
  const runningShellCount = useMemo(
    () => shells.filter((shell) => shell.state === 'Busy' || shell.state === 'Starting').length,
    [shells],
  );
  const agentShellCount = useMemo(
    () => shells.filter((shell) => !shell.userManaged).length,
    [shells],
  );
  const activeShellTitle = activeShell
    ? sessionLabel(activeShell, activeShellIndex >= 0 ? activeShellIndex : 0)
    : 'No terminal selected';
  const activeShellCommand = activeShell
    ? sessionCommandLabel(activeShell)
    : 'Select a terminal session';
  const activeShellPrimaryHeading = activeShell
    ? (activeShell.userManaged ? activeShellTitle : activeShellCommand)
    : 'No terminal selected';
  const activeShellSecondaryHeading = activeShell && activeShell.userManaged
    ? activeShellCommand
    : null;
  const activeShellDirectory = activeShell
    ? sessionWorkingDirectory(activeShell, defaultWorkingDirectory)
    : defaultWorkingDirectory || 'Project workspace';
  const activeShellProcess = activeShell ? sessionProcessLabel(activeShell) : null;
  const activeShellActivity = activeShell ? describeShellActivity(activeShell, activityNow) : null;

  useEffect(() => {
    if (!shells.some((shell) => shell.state === 'Busy' || shell.state === 'Starting' || shell.state === 'Interrupting')) {
      return undefined;
    }

    const intervalId = window.setInterval(() => setActivityNow(Date.now()), 5_000);
    return () => window.clearInterval(intervalId);
  }, [shells]);

  useEffect(() => {
    if (!visible || !activeShell || activeShell.userManaged) return;
    if (autoAttachedShellIdRef.current === activeShell.shellSessionId) return;

    autoAttachedShellIdRef.current = activeShell.shellSessionId;
    Promise.resolve(shellSessionAttach(sessionId, activeShell.shellSessionId)).catch((error) => {
      autoAttachedShellIdRef.current = null;
      console.error(error);
      onShowToast('Failed to connect input for the selected agent shell session', 'error');
    });
  }, [activeShell, onShowToast, sessionId, visible]);

  const handleResizeStart = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    event.preventDefault();
    if (mode === 'collapsed') onSetMode('expanded');

    const boundary = resizeBoundaryRef.current;
    const boundaryRect = boundary?.getBoundingClientRect();
    const bottom = boundaryRect?.bottom ?? window.innerHeight;
    const maxHeight = Math.max(220, Math.round((boundaryRect?.height ?? window.innerHeight) * 0.75));

    const onMove = (moveEvent: MouseEvent) => {
      const nextHeight = Math.min(maxHeight, Math.max(180, Math.round(bottom - moveEvent.clientY)));
      onSetHeight(nextHeight);
    };

    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };

    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }, [mode, onSetHeight, onSetMode, resizeBoundaryRef]);

  const handleStartShell = useCallback(async () => {
    try {
      setIsStartingShell(true);
      await startShellSessionStreaming(sessionId);
      onShowToast('Started terminal session in the project workspace', 'success');
    } catch (error) {
      console.error(error);
      onShowToast(`Failed to start terminal session: ${error}`, 'error');
    } finally {
      setIsStartingShell(false);
    }
  }, [onShowToast, sessionId]);

  const handleDeleteShell = useCallback(async (shell: ShellSessionRecord) => {
    try {
      await shellSessionStop(shell.shellSessionId);
      onCloseShellTab(shell.shellSessionId);
      onShowToast('Closed terminal session', 'success');
    } catch (error) {
      console.error(error);
      onShowToast('Failed to close terminal session', 'error');
    }
  }, [onCloseShellTab, onShowToast]);

  const handleActivateShell = useCallback((shell: ShellSessionRecord) => {
    onActivateShell(shell.shellSessionId);
  }, [onActivateShell]);

  if (!visible) return null;

  return (
    <section
      className={`shell-dock shell-dock--${mode}`}
      aria-label="Terminal workspace"
      style={{ height: mode === 'collapsed' ? 58 : height }}
    >
      <div className="shell-dock__resize-handle" onMouseDown={handleResizeStart}>
        <span className="shell-dock__resize-grip" />
      </div>

      <header className="shell-dock__bar">
        <div className="shell-dock__bar-main">
          <div className="shell-dock__title-row">
            <span className="shell-dock__title-icon"><TerminalIcon /></span>
            <div className="shell-dock__title-stack">
              <strong>Shell Session Manager</strong>
              <span title={activeShellCommand}>
                {activeShell ? `${activeShellTitle} • ${activeShellCommand}` : activeShellCommand}
              </span>
            </div>
          </div>

          <div className="shell-dock__bar-meta" aria-label="Shell manager summary">
            <span className="shell-dock__bar-stat">
              <span className="shell-dock__bar-stat-label">Sessions</span>
              <strong>{shells.length}</strong>
            </span>
            {runningShellCount > 0 && (
              <span className="shell-dock__bar-stat">
                <span className="shell-dock__bar-stat-label">Running</span>
                <strong>{runningShellCount}</strong>
              </span>
            )}
            {agentShellCount > 0 && (
              <span className="shell-dock__bar-stat">
                <span className="shell-dock__bar-stat-label">Agent-owned</span>
                <strong>{agentShellCount}</strong>
              </span>
            )}
          </div>
        </div>

        <div className="shell-dock__actions">
          <button
            type="button"
            className="shell-dock__toolbar-button primary"
            onClick={() => void handleStartShell()}
            disabled={isStartingShell}
            aria-label={isStartingShell ? 'Starting terminal session' : 'Start new terminal session'}
            title={isStartingShell ? 'Starting terminal session' : 'New Terminal'}
          >
            <PlusIcon />
            <span>{isStartingShell ? 'Starting…' : 'New Terminal'}</span>
          </button>
        </div>
      </header>

      {mode !== 'collapsed' && (
        <div className="shell-dock__body">
          <aside className="shell-dock__sidebar" aria-label="Terminal sessions list">
            {shells.length > 0 ? shells.map((shell, index) => {
              const isActive = shell.shellSessionId === activeShell?.shellSessionId;
              return (
                <div
                  key={shell.shellSessionId}
                  role="button"
                  tabIndex={0}
                  className={[
                    'shell-dock__session',
                    `shell-dock__session--${shell.state.toLowerCase()}`,
                    isActive ? 'active' : '',
                  ].filter(Boolean).join(' ')}
                  onClick={() => void handleActivateShell(shell)}
                  onKeyDown={(event) => {
                    if (event.key !== 'Enter' && event.key !== ' ') return;
                    event.preventDefault();
                    void handleActivateShell(shell);
                  }}
                  draggable
                  onDragStart={() => setDraggedShellId(shell.shellSessionId)}
                  onDragEnd={() => setDraggedShellId(null)}
                  onDragOver={(event) => event.preventDefault()}
                  onDrop={() => {
                    if (!draggedShellId) return;
                    onReorderShellTabs(draggedShellId, shell.shellSessionId);
                    setDraggedShellId(null);
                  }}
                >
                  <div className="shell-dock__session-main">
                    <span className="shell-dock__session-slot">{sessionSlotLabel(index)}</span>
                    <strong className="shell-dock__session-name" title={sessionLabel(shell, index)}>{sessionLabel(shell, index)}</strong>
                    <span className="shell-dock__session-divider" aria-hidden="true">•</span>
                    <span className={`shell-dock__session-list-state shell-dock__session-list-state--${shell.state.toLowerCase()}`}>
                      {sessionStateLabel(shell)}
                    </span>
                    <span className="shell-dock__session-divider" aria-hidden="true">•</span>
                    <span className={`shell-dock__session-list-owner shell-dock__session-list-owner--${shell.userManaged ? 'user' : 'agent'}`}>
                      {sessionListOwnerLabel(shell)}
                    </span>
                  </div>
                  <button
                    type="button"
                    className="btn-icon shell-dock__icon-button shell-dock__session-close"
                    aria-label={`Close ${sessionLabel(shell, index)}`}
                    title={`Close ${sessionLabel(shell, index)}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      void handleDeleteShell(shell);
                    }}
                  >
                    <span className="icon-close" aria-hidden="true" />
                  </button>
                </div>
              );
            }) : (
              <div className="shell-dock__empty-state shell-dock__empty-state--sidebar">
                <strong>No terminals yet</strong>
                <span>Start a new terminal or wait for the agent to run shell work.</span>
                <button type="button" className="primary" onClick={() => void handleStartShell()} disabled={isStartingShell}>
                  {isStartingShell ? 'Starting…' : 'New Terminal'}
                </button>
              </div>
            )}
          </aside>

          <main className="shell-dock__terminal-pane">
            {activeShell ? (
              <>
                <div className="shell-dock__terminal-header">
                  <div className="shell-dock__terminal-context">
                    <div className="shell-dock__terminal-title-row">
                      <strong>{activeShellPrimaryHeading}</strong>
                      <span className={`shell-dock__terminal-badge shell-dock__terminal-badge--${activeShell.userManaged ? 'user' : 'agent'}`}>
                        {sessionOwnerHeaderLabel(activeShell)}
                      </span>
                      <span className={`shell-dock__terminal-badge shell-dock__terminal-badge--state shell-dock__terminal-badge--state-${activeShell.state.toLowerCase()}`}>
                        {sessionStateLabel(activeShell)}
                      </span>
                      {activeShellActivity && (
                        <span className={`shell-dock__terminal-badge shell-dock__terminal-badge--activity shell-activity--${activeShellActivity.tone}`}>
                          {activeShellActivity.label}
                        </span>
                      )}
                    </div>
                    {activeShellSecondaryHeading && (
                      <span title={activeShellSecondaryHeading}>{activeShellSecondaryHeading}</span>
                    )}
                    {(activeShell.userManaged || activeShellProcess) && (
                      <div className="shell-dock__terminal-detail-row">
                        {activeShell.userManaged && (
                          <span className="shell-dock__terminal-detail">
                            <span className="shell-dock__terminal-detail-label">cwd</span>
                            <span className="shell-dock__terminal-detail-value" title={activeShellDirectory}>{activeShellDirectory}</span>
                          </span>
                        )}
                        {activeShellProcess && (
                          <span className="shell-dock__terminal-detail">
                            <span className="shell-dock__terminal-detail-label">proc</span>
                            <span className="shell-dock__terminal-detail-value" title={activeShellProcess}>{activeShellProcess}</span>
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                  <div className="shell-dock__terminal-meta">
                    <button
                      type="button"
                      className="btn-icon shell-dock__icon-button shell-dock__terminal-close"
                      aria-label={`Close active ${activeShellTitle}`}
                      title={`Close active ${activeShellTitle}`}
                      onClick={() => void handleDeleteShell(activeShell)}
                    >
                      <span className="icon-close" aria-hidden="true" />
                    </button>
                  </div>
                </div>
                <InteractiveShellTerminal shell={activeShell} />
              </>
            ) : (
              <div className="shell-dock__empty-state shell-dock__empty-state--terminal">
                <strong>Terminal manager ready</strong>
                <span>
                  Agent-run shells and your own terminals will appear here. Select one to inspect it or take it over.
                </span>
              </div>
            )}
          </main>
        </div>
      )}
    </section>
  );
};

export default ShellManagerPanel;