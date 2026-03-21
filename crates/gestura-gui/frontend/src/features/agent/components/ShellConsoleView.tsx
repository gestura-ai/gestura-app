import React, { useEffect, useRef, useState } from 'react';

import type { ShellBlock, ShellSessionRecord } from '../types';
import { ansiToHtml } from '../utils/ansi';
import { describeShellActivity } from '../utils/shellActivity';

type RenderableShell = ShellBlock | ShellSessionRecord;

export interface ShellConsoleViewProps {
  block: RenderableShell;
  variant?: 'inline' | 'panel';
  allowCollapse?: boolean;
  readOnly?: boolean;
  onRevealSession?: (shellSessionId: string | null) => void;
}

const TerminalIcon = () => (
  <svg viewBox="0 0 16 16" aria-hidden="true">
    <path d="M2.75 4.25h10.5a1 1 0 0 1 1 1v5.5a1 1 0 0 1-1 1H2.75a1 1 0 0 1-1-1v-5.5a1 1 0 0 1 1-1Z" fill="none" stroke="currentColor" strokeWidth="1.2" />
    <path d="m5 6.4 1.8 1.6L5 9.6" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.2" />
    <path d="M8.7 9.8h2.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.2" />
  </svg>
);

function isShellSessionRecord(block: RenderableShell): block is ShellSessionRecord {
  return block.kind === 'shell-session';
}

function isTerminalState(block: RenderableShell): boolean {
  if (isShellSessionRecord(block)) {
    return block.state === 'Stopped' || block.state === 'Failed';
  }
  return block.state === 'Completed' || block.state === 'Failed' || block.state === 'Stopped';
}

function statusClassName(block: RenderableShell): string {
  if (isShellSessionRecord(block)) {
    if (block.state === 'Interrupting') return 'paused';
    if (block.state === 'Idle') return 'success';
    if (!isTerminalState(block)) return 'running';
    return block.state === 'Stopped' ? 'error' : 'error';
  }
  if (block.state === 'Paused') return 'paused';
  if (!isTerminalState(block)) return 'running';
  return block.exitCode === 0 ? 'success' : 'error';
}

function statusLabel(block: RenderableShell): string {
  if (isShellSessionRecord(block)) {
    switch (block.state) {
      case 'Idle': return 'Idle';
      case 'Busy': return 'Running…';
      case 'Interrupting': return 'Interrupting…';
      case 'Stopping': return 'Stopping…';
      case 'Stopped': return 'Stopped';
      case 'Failed': return 'Failed';
      case 'Starting':
      default:
        return 'Starting…';
    }
  }

  if (block.state === 'Paused') return 'Paused';
  if (block.state === 'Completed') return block.exitCode === 0 ? 'Completed' : `Exit ${block.exitCode}`;
  if (block.state === 'Failed') return 'Failed';
  if (block.state === 'Stopped') return 'Stopped';
  return 'Running…';
}

function commandLabel(block: RenderableShell): string {
  if (isShellSessionRecord(block)) {
    return block.activeCommand || 'Interactive shell session';
  }
  return block.command || 'shell';
}

function workingDirectory(block: RenderableShell): string | null {
  return block.cwd ?? null;
}

export const ShellConsoleView: React.FC<ShellConsoleViewProps> = ({
  block,
  variant = 'inline',
  allowCollapse = true,
  readOnly = false,
  onRevealSession,
}) => {
  const [collapsed, setCollapsed] = useState(() => allowCollapse ? block.collapsed : false);
  const [activityNow, setActivityNow] = useState(() => Date.now());
  const outputRef = useRef<HTMLDivElement>(null);
  const isTerminal = isTerminalState(block);
  const isCollapsed = allowCollapse ? collapsed : false;
  const copyValue = commandLabel(block);
  const commandCwd = workingDirectory(block);
  const activity = describeShellActivity(block, activityNow);

  useEffect(() => {
    if (isCollapsed || !outputRef.current) return;
    outputRef.current.scrollTop = outputRef.current.scrollHeight;
  }, [block.lines.length, isCollapsed]);

  useEffect(() => {
    if (isTerminal) return undefined;
    const intervalId = window.setInterval(() => setActivityNow(Date.now()), 5_000);
    return () => window.clearInterval(intervalId);
  }, [isTerminal]);

  return (
    <div
      className={`shell-console shell-console--${variant}${isCollapsed ? ' collapsed' : ''}`}
      data-process-id={isShellSessionRecord(block) ? block.shellSessionId : block.processId}
    >
      <div className="shell-console-header">
        {allowCollapse ? (
          <button
            type="button"
            className="shell-console-toggle"
            aria-expanded={!isCollapsed}
            onClick={() => setCollapsed((current) => !current)}
          >
            <div className="shell-console-heading">
              <span className="shell-console-heading-icon"><TerminalIcon /></span>
              <strong className="shell-cmd" title={copyValue}>{copyValue}</strong>
            </div>
            <span className={`shell-console-status shell-console-status--${statusClassName(block)}`}>{statusLabel(block)}</span>
            <span className="shell-console-chevron">{isCollapsed ? '▸' : '▾'}</span>
          </button>
        ) : (
          <div className="shell-console-toggle shell-console-toggle--static">
            <div className="shell-console-heading">
              <span className="shell-console-heading-icon"><TerminalIcon /></span>
              <strong className="shell-cmd" title={copyValue}>{copyValue}</strong>
            </div>
            <span className={`shell-console-status shell-console-status--${statusClassName(block)}`}>{statusLabel(block)}</span>
          </div>
        )}
        <div className="shell-controls">
          {!readOnly && variant === 'inline' && block.shellSessionId && onRevealSession && (
            <button
              type="button"
              aria-label="Open in Shell Session Manager"
              title="Open in Shell Session Manager"
              onClick={() => onRevealSession(block.shellSessionId ?? null)}
            >
              ↗
            </button>
          )}
        </div>
      </div>

      {!isCollapsed && (
        <div className="shell-console-body">
          <div className="shell-console-meta">
            {commandCwd && <span className="shell-console-detail" title={commandCwd}>{commandCwd}</span>}
            <span className={`shell-console-activity shell-console-activity--${activity.tone}`}>{activity.label}</span>
            {block.durationMs != null && (
              <span className="shell-console-duration">{(block.durationMs / 1000).toFixed(1)}s</span>
            )}
          </div>
          <div className={`shell-console-output shell-console-output--${variant}`} ref={outputRef}>
            {block.lines.length > 0 ? (
              block.lines.map((line, index) => (
                <div
                  key={index}
                  className={`shell-line${line.stream === 'Stderr' ? ' shell-stderr' : ''}`}
                  dangerouslySetInnerHTML={{ __html: ansiToHtml(line.data) }}
                />
              ))
            ) : (
              <div className="shell-line shell-line--empty">
                {isShellSessionRecord(block) ? 'Interactive shell is idle and waiting for activity…' : 'Waiting for output…'}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default ShellConsoleView;