import '@xterm/xterm/css/xterm.css';

import React, { useEffect, useRef } from 'react';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';

import {
  shellSessionInput,
  shellSessionResize,
} from '../../../services/tauri/agent';
import type { ShellSessionRecord } from '../types';

interface InteractiveShellTerminalProps {
  shell: ShellSessionRecord;
  readOnly?: boolean;
}

function readCssColor(name: string, fallback: string): string {
  if (typeof window === 'undefined') return fallback;
  const value = window.getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

function buildTerminalTheme() {
  const accent = readCssColor('--accent-primary', '#3b82f6');
  const background = readCssColor('--bg-base', '#111827');
  const elevated = readCssColor('--bg-glass-strong', background);
  const foreground = readCssColor('--text-primary', '#e5e7eb');
  const secondary = readCssColor('--text-secondary', '#94a3b8');

  return {
    background,
    foreground,
    cursor: accent,
    cursorAccent: background,
    selectionBackground: 'rgba(59, 130, 246, 0.18)',
    black: background,
    red: '#ef4444',
    green: '#22c55e',
    yellow: '#f59e0b',
    blue: accent,
    magenta: '#a855f7',
    cyan: '#06b6d4',
    white: foreground,
    brightBlack: secondary,
    brightRed: '#f87171',
    brightGreen: '#4ade80',
    brightYellow: '#fbbf24',
    brightBlue: '#60a5fa',
    brightMagenta: '#c084fc',
    brightCyan: '#22d3ee',
    brightWhite: elevated,
  };
}

export const InteractiveShellTerminal: React.FC<InteractiveShellTerminalProps> = ({ shell, readOnly = false }) => {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const lastRenderedLineRef = useRef(0);
  const pendingWriteRef = useRef(Promise.resolve());

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return undefined;

    const terminal = new Terminal({
      allowTransparency: true,
      convertEol: false,
      cursorBlink: !readOnly,
      cursorStyle: 'block',
      disableStdin: readOnly,
      fontFamily: 'JetBrains Mono, SFMono-Regular, ui-monospace, monospace',
      fontSize: 13,
      lineHeight: 1.3,
      scrollback: 6000,
      theme: buildTerminalTheme(),
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(host);
    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;
    terminal.write(shell.lines.map((line) => line.data).join(''));
    lastRenderedLineRef.current = shell.lines.length;

    const scheduleResize = () => {
      window.requestAnimationFrame(() => {
        fitAddon.fit();
        void shellSessionResize(shell.shellSessionId, terminal.cols, terminal.rows).catch(console.error);
      });
    };

    const resizeObserver = new ResizeObserver(scheduleResize);
    resizeObserver.observe(host);
    scheduleResize();
    terminal.focus();

    const applyTheme = () => {
      terminal.options.theme = buildTerminalTheme();
    };

    const themeObserver = new MutationObserver(applyTheme);
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class', 'data-theme', 'style'],
    });

    const dataDisposable = terminal.onData((data) => {
      if (readOnly) return;

      pendingWriteRef.current = pendingWriteRef.current
        .catch(() => undefined)
        .then(() => shellSessionInput(shell.shellSessionId, data));
    });

    const focusTimer = window.setTimeout(() => terminal.focus(), 30);

    return () => {
      window.clearTimeout(focusTimer);
      dataDisposable.dispose();
      resizeObserver.disconnect();
      themeObserver.disconnect();
      terminal.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      lastRenderedLineRef.current = 0;
    };
  }, [readOnly, shell.shellSessionId]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;

    const nextLines = shell.lines.slice(lastRenderedLineRef.current);
    if (nextLines.length === 0) return;

    for (const line of nextLines) {
      terminal.write(line.data);
    }
    lastRenderedLineRef.current = shell.lines.length;
  }, [shell.lines]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal) return;
    terminal.focus();
  }, [shell.shellSessionId]);

  return <div className={`interactive-shell-terminal${readOnly ? ' interactive-shell-terminal--read-only' : ''}`} ref={hostRef} />;
};

export default InteractiveShellTerminal;