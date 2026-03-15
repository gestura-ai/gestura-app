import React, { useCallback, useEffect, useMemo, useState } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';

import ThemeController from '../../app/ThemeController';
import { useViewMode } from './hooks/useViewMode';
import { useKeyboardShortcuts } from '../../shared/hooks/useKeyboardShortcuts';
import { usePanelResize } from './hooks/usePanelResize';
import { getConfig } from '../../services/tauri/config';
import type { UiSettings } from '../../types/config';
import './AgentApp.css';
import { ChatPanel } from './components/ChatPanel';
import { ExplorerPanel } from './components/ExplorerPanel';
// Lazy-load EditorArea so that @codemirror/* (675 kB) is only fetched when the
// user first opens the editor view — keeping the initial chat bundle small.
const EditorArea = React.lazy(() => import('./components/EditorArea'));

// ─── Props ────────────────────────────────────────────────────────────────────
export interface AgentAppProps {
  sessionId: string;
}

// ─── Editor window sizes ──────────────────────────────────────────────────────
const EDITOR_SIZE = new LogicalSize(1200, 800);
const CHAT_SIZE = new LogicalSize(800, 600);
const AGENT_BOOT_TIMEOUT_MS = 1500;

const prefersDarkMode = (): boolean =>
  typeof window.matchMedia === 'function'
  && window.matchMedia('(prefers-color-scheme: dark)').matches;

interface AgentErrorBoundaryProps {
  sessionId: string;
  children: ReactNode;
}

interface AgentErrorBoundaryState {
  hasError: boolean;
}

class AgentErrorBoundary extends React.Component<AgentErrorBoundaryProps, AgentErrorBoundaryState> {
  public constructor(props: AgentErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false };
  }

  public static getDerivedStateFromError(): AgentErrorBoundaryState {
    return { hasError: true };
  }

  public componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    console.error('[AgentApp] render failed:', error, errorInfo);
  }

  public render(): ReactNode {
    if (this.state.hasError) {
      return (
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: '12px',
            minHeight: '100vh',
            padding: '24px',
            background: 'var(--bg-base, #0b0f14)',
            color: 'var(--text-primary, #f5f7fa)',
            fontFamily: 'Inter, system-ui, sans-serif',
          }}
        >
          <h2 style={{ margin: 0, fontSize: '18px' }}>Agent session failed to load</h2>
          <p style={{ margin: 0, color: 'var(--text-secondary, #a6b0bf)' }}>
            The agent UI hit a startup error. Please close this window and try again.
          </p>
          <p style={{ margin: 0, fontSize: '12px', color: 'var(--text-secondary, #a6b0bf)' }}>
            Session: {this.props.sessionId || 'unknown'}
          </p>
        </div>
      );
    }

    return this.props.children;
  }
}

// ─── Root shell ───────────────────────────────────────────────────────────────
const AgentApp: React.FC<AgentAppProps> = ({ sessionId }) => {
  const { viewMode, toggleViewMode } = useViewMode();
  const [uiSettings, setUiSettings] = useState<UiSettings>({ theme_mode: 'system', accent: 'blue' });
  const [explorerOpen, setExplorerOpen] = useState(true);
  const [chatOpen, setChatOpen] = useState(true);

  // Controls visibility and the CSS fade-in animation. The window is created
  // hidden by Rust (visible:false); we reveal it here once the theme tokens and
  // layout have been committed to the DOM, eliminating all paint flashes.
  const [isReady, setIsReady] = useState(false);

  const explorer = usePanelResize(240, 160, 480, 'left');
  const chat = usePanelResize(340, 260, 520, 'right');

  // Load theme configuration on mount. When the IPC resolves (or fails), mark
  // the app as ready so the window-show effect can fire in the next commit.
  useEffect(() => {
    let cancelled = false;
    let readyMarked = false;

    const markReady = () => {
      if (cancelled || readyMarked) return;
      readyMarked = true;
      setIsReady(true);
    };

    const timer = window.setTimeout(() => {
      console.warn('[AgentApp] config load timed out — revealing window with default UI settings');
      markReady();
    }, AGENT_BOOT_TIMEOUT_MS);

    getConfig()
      .then((cfg) => {
        if (cancelled) return;
        setUiSettings(cfg.ui);
        markReady();
      })
      .catch((err) => {
        if (cancelled) return;
        console.warn('[AgentApp] config load failed — using defaults:', err);
        markReady();
      });

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, []);

  // Once isReady is true, ThemeController will have already committed its
  // data-theme update (child effects run before parent effects in React).
  // Showing the window here means the OS frame appears fully painted.
  useEffect(() => {
    if (!isReady) return;
    const win = getCurrentWindow();
    // Set the correct initial size for the current view mode before revealing.
    Promise.resolve(
      win.setSize(viewMode === 'editor' ? EDITOR_SIZE : CHAT_SIZE)
    )
      .catch((err) => console.warn('[AgentApp] initial resize failed:', err))
      .finally(() => {
        Promise.resolve(win.show()).catch((err) => console.warn('[AgentApp] window show failed:', err));
        Promise.resolve(win.setFocus()).catch(() => { });
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isReady]);

  // Resize window whenever the view mode changes after the initial reveal.
  useEffect(() => {
    if (!isReady) return;
    const win = getCurrentWindow();
    Promise.resolve(win.setSize(viewMode === 'editor' ? EDITOR_SIZE : CHAT_SIZE)).catch((err) =>
      console.warn('[AgentApp] window resize failed:', err)
    );
  }, [viewMode, isReady]);

  // Derive isDark from theme settings + system preference.
  const isDark = useMemo(() => {
    if (uiSettings.theme_mode === 'dark') return true;
    if (uiSettings.theme_mode === 'light') return false;
    return prefersDarkMode();
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

  // ── Intercept GUI Control commands sent by the Agent ────────────────────────
  useEffect(() => {
    const handleGuiControl = (e: Event) => {
      const payload = (e as CustomEvent<{ action: string, target?: string }>).detail;
      if (!payload || !payload.action) return;

      switch (payload.action) {
        case 'toggle_view_mode':
          toggleViewMode();
          break;
        case 'open_explorer':
          if (viewMode !== 'editor') toggleViewMode();
          setExplorerOpen(true);
          break;
        case 'close_explorer':
          if (viewMode !== 'editor') toggleViewMode();
          setExplorerOpen(false);
          break;
        case 'open_chat':
          if (viewMode !== 'editor') toggleViewMode();
          setChatOpen(true);
          break;
        case 'close_chat':
          if (viewMode !== 'editor') toggleViewMode();
          setChatOpen(false);
          break;
        case 'navigate_config':
          // In the future this might open a preferences modal
          break;
      }
    };
    window.addEventListener('gestura:gui_control', handleGuiControl);
    return () => window.removeEventListener('gestura:gui_control', handleGuiControl);
  }, [toggleViewMode, viewMode]);

  // ── Layout ──────────────────────────────────────────────────────────────────
  const isEditor = viewMode === 'editor';

  const explorerStyle: React.CSSProperties = explorerOpen
    ? { width: explorer.width, flexBasis: explorer.width, minWidth: 160 }
    : {};

  // Only apply inline resize style in editor mode — in message-only mode the
  // CSS (.agent-app--message-only .agent-panel--chat) sets width: 100% and
  // flex: 1 1 auto, and inline styles would override that and pin it to the
  // resizer's fixed width (340 px), making it look like a narrow left panel.
  const chatStyle: React.CSSProperties = (isEditor && chatOpen)
    ? { width: chat.width, flexBasis: chat.width, minWidth: 260 }
    : {};

  return (
    <AgentErrorBoundary sessionId={sessionId}>
      <ThemeController uiSettings={uiSettings} onUpdate={setUiSettings} />
      <div
        className={[
          'agent-app',
          isEditor ? 'agent-app--editor' : 'agent-app--message-only',
          isEditor && explorerOpen ? 'agent-app--explorer-open' : '',
          isEditor && chatOpen ? 'agent-app--chat-open' : '',
          // Triggers the CSS fade-in once theme + config are committed to the DOM.
          isReady ? 'app-ready' : '',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        {isEditor && (
          <ExplorerPanel sessionId={sessionId} onOpenFile={handleOpenFile} style={explorerStyle} />
        )}
        {isEditor && (
          <div className="panel-resizer panel-resizer--left">
            <div className="panel-resizer__track" onMouseDown={explorer.handleMouseDown} />
            <div
              className="panel-resizer__thumb panel-resizer__thumb--left"
              onMouseDown={(e) => { e.stopPropagation(); explorer.handleMouseDown(e); }}
              onClick={() => setExplorerOpen((v) => !v)}
              title={explorerOpen ? 'Collapse Explorer (\u2318B)' : 'Expand Explorer (\u2318B)'}
            >
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <path d={explorerOpen ? 'M15 18l-6-6 6-6' : 'M9 18l6-6-6-6'} />
              </svg>
            </div>
          </div>
        )}
        {isEditor && (
          <React.Suspense fallback={null}>
            <EditorArea sessionId={sessionId} isDark={isDark} />
          </React.Suspense>
        )}
        {isEditor && (
          <div className="panel-resizer panel-resizer--right">
            <div className="panel-resizer__track" onMouseDown={chat.handleMouseDown} />
            <div
              className="panel-resizer__thumb panel-resizer__thumb--right"
              onMouseDown={(e) => { e.stopPropagation(); chat.handleMouseDown(e); }}
              onClick={() => setChatOpen((v) => !v)}
              title={chatOpen ? 'Collapse Chat' : 'Expand Chat'}
            >
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <path d={chatOpen ? 'M9 18l6-6-6-6' : 'M15 18l-6-6 6-6'} />
              </svg>
            </div>
          </div>
        )}
        <ChatPanel sessionId={sessionId} onToggleEditor={toggleViewMode} viewMode={viewMode} style={chatStyle} />
      </div>
    </AgentErrorBoundary>
  );
};

export default AgentApp;

