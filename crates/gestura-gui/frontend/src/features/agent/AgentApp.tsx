import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';

import ThemeController from '../../app/ThemeController';
import { useViewMode } from './hooks/useViewMode';
import { useKeyboardShortcuts } from '../../shared/hooks/useKeyboardShortcuts';
import { getConfig } from '../../services/tauri/config';
import type { UiSettings } from '../../types/config';
import './AgentApp.css';
import { ChatPanel } from './components/ChatPanel';
import { ExplorerPanel } from './components/ExplorerPanel';
import { EditorArea } from './components/EditorArea';

// ─── Props ────────────────────────────────────────────────────────────────────
export interface AgentAppProps {
  sessionId: string;
}

// ─── Editor window sizes ──────────────────────────────────────────────────────
const EDITOR_SIZE = new LogicalSize(1200, 800);
const CHAT_SIZE = new LogicalSize(800, 600);

// ─── Root shell ───────────────────────────────────────────────────────────────
const AgentApp: React.FC<AgentAppProps> = ({ sessionId }) => {
  const { viewMode, toggleViewMode } = useViewMode();
  const [uiSettings, setUiSettings] = useState<UiSettings>({ theme_mode: 'system', accent: 'blue' });
  const [explorerOpen, setExplorerOpen] = useState(true);
  const [chatOpen, setChatOpen] = useState(true);

  // Load theme configuration on mount.
  useEffect(() => {
    getConfig()
      .then((cfg) => setUiSettings(cfg.ui))
      .catch((err) => console.warn('[AgentApp] config load failed:', err));
  }, []);

  // Resize window to match the active view mode.
  useEffect(() => {
    const win = getCurrentWindow();
    win.setSize(viewMode === 'editor' ? EDITOR_SIZE : CHAT_SIZE).catch((err) =>
      console.warn('[AgentApp] window resize failed:', err)
    );
  }, [viewMode]);

  // Derive isDark from theme settings + system preference.
  const isDark = useMemo(() => {
    if (uiSettings.theme_mode === 'dark') return true;
    if (uiSettings.theme_mode === 'light') return false;
    return window.matchMedia('(prefers-color-scheme: dark)').matches;
  }, [uiSettings.theme_mode]);

  // Queued rel path to open after EditorArea mounts (race fix).
  const pendingOpenRef = React.useRef<string | null>(null);

  // ExplorerPanel calls this when a file is double-clicked.
  // EditorArea exposes handleOpenFile via window.__gesturaOpenFile.
  const handleOpenFile = useCallback((relPath: string) => {
    const fn = (window as unknown as Record<string, unknown>).__gesturaOpenFile;
    if (typeof fn === 'function') {
      (fn as (p: string) => void)(relPath);
      // Switch to editor mode if not already there
      if (viewMode !== 'editor') toggleViewMode();
    } else {
      // EditorArea hasn't mounted yet — queue the path and switch mode.
      pendingOpenRef.current = relPath;
      if (viewMode !== 'editor') toggleViewMode();
    }
  }, [viewMode, toggleViewMode]);

  // Once editor mode is active, drain any queued file open.
  useEffect(() => {
    if (viewMode !== 'editor' || !pendingOpenRef.current) return;
    const relPath = pendingOpenRef.current;
    pendingOpenRef.current = null;
    // EditorArea registers the handler asynchronously on its first render;
    // schedule the call in a microtask to ensure it has run.
    setTimeout(() => {
      const fn = (window as unknown as Record<string, unknown>).__gesturaOpenFile;
      if (typeof fn === 'function') (fn as (p: string) => void)(relPath);
    }, 0);
  }, [viewMode]);

  // ── Keyboard shortcuts ──────────────────────────────────────────────────────
  const handleKeyDown = useCallback(
    (event: KeyboardEvent) => {
      const meta = event.metaKey || event.ctrlKey;
      if (!meta) return;

      // Cmd/Ctrl+E — toggle editor view
      if (event.key === 'e' || event.key === 'E') {
        event.preventDefault();
        toggleViewMode();
        return;
      }

      // Cmd/Ctrl+B — toggle explorer panel (only in editor mode)
      if ((event.key === 'b' || event.key === 'B') && viewMode === 'editor') {
        event.preventDefault();
        setExplorerOpen((prev) => !prev);
        return;
      }

      // Cmd/Ctrl+S — forward to EditorArea via custom event; EditorArea uses
      // its own activeTabId so we don't need to duplicate tab state here.
      if ((event.key === 's' || event.key === 'S') && viewMode === 'editor') {
        event.preventDefault();
        window.dispatchEvent(new CustomEvent('gestura:editor:save'));
      }
    },
    [toggleViewMode, viewMode]
  );

  useKeyboardShortcuts(handleKeyDown);

  // ── Layout ──────────────────────────────────────────────────────────────────
  const isEditor = viewMode === 'editor';

  return (
    <>
      <ThemeController uiSettings={uiSettings} onUpdate={setUiSettings} />
      <div
        className={[
          'agent-app',
          isEditor ? 'agent-app--editor' : 'agent-app--message-only',
          isEditor && explorerOpen ? 'agent-app--explorer-open' : '',
          isEditor && chatOpen ? 'agent-app--chat-open' : '',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        {isEditor && (
          <ExplorerPanel sessionId={sessionId} onOpenFile={handleOpenFile} />
        )}
        {isEditor && (
          <div className="panel-toggle-container">
            <div
              className="panel-toggle-btn panel-toggle-left"
              onClick={() => setExplorerOpen(!explorerOpen)}
              title={explorerOpen ? "Collapse Explorer" : "Expand Explorer"}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d={explorerOpen ? "M15 18l-6-6 6-6" : "M9 18l6-6-6-6"} />
              </svg>
            </div>
          </div>
        )}
        {isEditor && (
          <EditorArea sessionId={sessionId} isDark={isDark} />
        )}
        {isEditor && (
          <div className="panel-toggle-container">
            <div
              className="panel-toggle-btn panel-toggle-right"
              onClick={() => setChatOpen(!chatOpen)}
              title={chatOpen ? "Collapse Chat" : "Expand Chat"}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d={chatOpen ? "M9 18l6-6-6-6" : "M15 18l-6-6 6-6"} />
              </svg>
            </div>
          </div>
        )}
        <ChatPanel sessionId={sessionId} onToggleEditor={toggleViewMode} viewMode={viewMode} />
      </div>
    </>
  );
};

export default AgentApp;

