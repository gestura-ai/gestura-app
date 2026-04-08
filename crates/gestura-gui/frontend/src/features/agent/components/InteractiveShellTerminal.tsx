import '@xterm/xterm/css/xterm.css';

import React, { useEffect, useRef } from 'react';
import { FitAddon } from '@xterm/addon-fit';
import { Terminal } from '@xterm/xterm';

import {
  shellSessionInput,
  shellSessionResize,
} from '../../../services/tauri/agent';
import type { ShellSessionRecord } from '../types';
import { buildTerminalTheme } from './terminalTheme';

interface InteractiveShellTerminalProps {
  shell: ShellSessionRecord;
  readOnly?: boolean;
  className?: string;
}

export const InteractiveShellTerminal: React.FC<InteractiveShellTerminalProps> = ({
  shell,
  readOnly = false,
  className,
}) => {
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
        if (!readOnly) {
          void shellSessionResize(shell.shellSessionId, terminal.cols, terminal.rows).catch(console.error);
        }
      });
    };

    const resizeObserver = new ResizeObserver(scheduleResize);
    resizeObserver.observe(host);
    scheduleResize();
    if (!readOnly) {
      terminal.focus();
    }

    const applyTheme = () => {
      terminal.options.theme = buildTerminalTheme();
    };

    const themeObserver = new MutationObserver(applyTheme);
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class', 'data-theme', 'style'],
    });

    const dataDisposable = readOnly
      ? null
      : terminal.onData((data) => {
        pendingWriteRef.current = pendingWriteRef.current
          .catch(() => undefined)
          .then(() => shellSessionInput(shell.shellSessionId, data));
      });

    const focusTimer = readOnly ? null : window.setTimeout(() => terminal.focus(), 30);

    return () => {
      if (focusTimer != null) window.clearTimeout(focusTimer);
      dataDisposable?.dispose();
      resizeObserver.disconnect();
      themeObserver.disconnect();
      terminal.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      lastRenderedLineRef.current = 0;
    };
    // `shell.lines` are applied incrementally by the follow-up effect below; including
    // them here would recreate the terminal on every output chunk.
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    if (readOnly) return;
    const terminal = terminalRef.current;
    if (!terminal) return;
    terminal.focus();
  }, [readOnly, shell.shellSessionId]);

  const classes = [
    'interactive-shell-terminal',
    readOnly ? 'interactive-shell-terminal--read-only' : '',
    className ?? '',
  ].filter(Boolean).join(' ');

  return <div className={classes} ref={hostRef} />;
};

export default InteractiveShellTerminal;