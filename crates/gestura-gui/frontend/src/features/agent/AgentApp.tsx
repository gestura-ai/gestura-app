import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';

import ThemeController from '../../app/ThemeController';
import { useViewMode } from './hooks/useViewMode';
import { useKeyboardShortcuts } from '../../shared/hooks/useKeyboardShortcuts';
import { usePanelResize } from './hooks/usePanelResize';
import { usePanelState } from './hooks/usePanelState';
import { useToast } from './hooks/useToast';
import { useShellSessions } from './hooks/useShellSessions';
import { getConfig } from '../../services/tauri/config';
import { getSessionWorkspaceById } from '../../services/tauri/agent';
import type { UiSettings } from '../../types/config';
import './AgentApp.css';
import { AgentSessionHeader } from './components/AgentSessionHeader';
import { ChatPanel } from './components/ChatPanel';
import { ExplorerPanel } from './components/ExplorerPanel';
import { ShellManagerPanel } from './components/ShellManagerPanel';
import { ToastContainer } from './components/ToastContainer';
import type { EditorOpenOptions, ShellSessionRecord } from './types';
// Lazy-load EditorArea so that @codemirror/* (675 kB) is only fetched when the
// user first opens the editor view — keeping the initial chat bundle small.
const EditorArea = React.lazy(() => import('./components/EditorArea'));

// ─── Props ────────────────────────────────────────────────────────────────────
export interface AgentAppProps {
  sessionId: string;
}

// ─── Editor window sizes ──────────────────────────────────────────────────────
const EDITOR_SIZE = new LogicalSize(1200, 800);
const CHAT_SIZE = new LogicalSize(550, 600);
const MIN_AGENT_WINDOW_SIZE = new LogicalSize(500, 320);
const AGENT_REVEAL_DELAY_MS = 0;
const AGENT_CONFIG_WARN_TIMEOUT_MS = 1500;

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
  const panelState = usePanelState();
  const toastState = useToast();
  const shellSessions = useShellSessions(sessionId, { restoreHistory: panelState.shellManager.visible });
  const [uiSettings, setUiSettings] = useState<UiSettings>({ theme_mode: 'system', accent: 'blue' });
  const [explorerOpen, setExplorerOpen] = useState(true);
  const [chatOpen, setChatOpen] = useState(true);
  const [headerOptionsOpen, setHeaderOptionsOpen] = useState(false);
  const [sessionWorkspace, setSessionWorkspace] = useState<string | null>(null);
  const [quickAccessHost, setQuickAccessHost] = useState<HTMLDivElement | null>(null);
  const appRef = useRef<HTMLDivElement>(null);

  // Controls visibility and the CSS fade-in animation. The window is created
  // hidden by Rust (visible:false); we reveal it here once the theme tokens and
  // layout have been committed to the DOM, eliminating all paint flashes.
  const [isReady, setIsReady] = useState(false);

  const explorer = usePanelResize(240, 160, 480, 'left');
  const chat = usePanelResize(340, 260, 520, 'right');
  const {
    shellManager,
    syncShellTabs,
    togglePanel,
    toggleShellManager,
    setShellManagerMode,
    setShellManagerHeight,
    setActiveShell,
    reorderShellTabs,
    closeShellTab,
  } = panelState;

  const orderedShellSessions = useMemo(() => {
    const shellMap = new Map(shellSessions.map((shell) => [shell.shellSessionId, shell]));
    const ordered = shellManager.tabOrder
      .map((shellId) => shellMap.get(shellId) ?? null)
      .filter((shell): shell is ShellSessionRecord => shell != null);
    const orderedIds = new Set(ordered.map((shell) => shell.shellSessionId));
    const extras = shellSessions.filter((shell) => (
      !orderedIds.has(shell.shellSessionId) && !shellManager.closedShellIds.includes(shell.shellSessionId)
    ));
    return [...ordered, ...extras];
  }, [shellManager.closedShellIds, shellManager.tabOrder, shellSessions]);

  // Reveal on the first startup tick using default UI settings, then hydrate the
  // real config in the background. This keeps config IPC off the critical path
  // for first visibility while still letting fast config loads apply before the
  // window is shown (microtasks resolve before the timer fires).
  useEffect(() => {
    let cancelled = false;
    const revealTimer = window.setTimeout(() => {
      if (cancelled) return;
      setIsReady(true);
    }, AGENT_REVEAL_DELAY_MS);

    const configWarnTimer = window.setTimeout(() => {
      if (cancelled) return;
      console.warn('[AgentApp] config load is still pending after startup — continuing with default UI settings');
    }, AGENT_CONFIG_WARN_TIMEOUT_MS);

    getConfig()
      .then((cfg) => {
        if (cancelled) return;
        window.clearTimeout(configWarnTimer);
        setUiSettings(cfg.ui);
      })
      .catch((err) => {
        if (cancelled) return;
        window.clearTimeout(configWarnTimer);
        console.warn('[AgentApp] config load failed — using defaults:', err);
      });

    return () => {
      cancelled = true;
      window.clearTimeout(revealTimer);
      window.clearTimeout(configWarnTimer);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;

    getSessionWorkspaceById(sessionId)
      .then((workspace) => {
        if (!cancelled) setSessionWorkspace(workspace);
      })
      .catch((error) => {
        if (!cancelled) {
          console.warn('[AgentApp] failed to resolve session workspace:', error);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [sessionId]);

  useEffect(() => {
    syncShellTabs(
      shellSessions.map((shell) => shell.shellSessionId),
      shellSessions[shellSessions.length - 1]?.shellSessionId ?? null,
    );
  }, [shellSessions, syncShellTabs]);

  // Once isReady is true, ThemeController will have already committed its
  // data-theme update (child effects run before parent effects in React).
  // Showing the window here means the OS frame appears fully painted.
  useEffect(() => {
    if (!isReady) return;
    const win = getCurrentWindow();
    // Set the correct initial size for the current view mode before revealing.
    Promise.resolve(win.setMinSize(MIN_AGENT_WINDOW_SIZE))
      .catch((err) => console.warn('[AgentApp] min window size failed:', err))
      .then(() => win.setSize(viewMode === 'editor' ? EDITOR_SIZE : CHAT_SIZE))
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

  type PendingOpenFile = { relPath: string; options?: EditorOpenOptions };

  // Queued rel path to open after EditorArea mounts (race fix).
  const pendingOpenRef = React.useRef<PendingOpenFile | null>(null);

  // ExplorerPanel calls this when a file is double-clicked.
  // EditorArea exposes handleOpenFile via window.__gesturaOpenFile.
  const handleOpenFile = useCallback((relPath: string, options?: EditorOpenOptions) => {
    const fn = (window as unknown as Record<string, unknown>).__gesturaOpenFile;
    if (typeof fn === 'function') {
      (fn as (path: string, openOptions?: EditorOpenOptions) => void)(relPath, options);
      // Switch to editor mode if not already there
      if (viewMode !== 'editor') toggleViewMode();
    } else {
      // EditorArea hasn't mounted yet — queue the path and switch mode.
      pendingOpenRef.current = { relPath, options };
      if (viewMode !== 'editor') toggleViewMode();
    }
  }, [viewMode, toggleViewMode]);

  const handleWorkspaceChanged = useCallback((workspace: string) => {
    setSessionWorkspace(workspace);
    window.dispatchEvent(new CustomEvent('gestura:workspace:changed'));
  }, []);

  // Once editor mode is active, drain any queued file open.
  useEffect(() => {
    if (viewMode !== 'editor' || !pendingOpenRef.current) return;
    const pending = pendingOpenRef.current;
    pendingOpenRef.current = null;
    // EditorArea registers the handler asynchronously on its first render;
    // schedule the call in a microtask to ensure it has run.
    setTimeout(() => {
      const fn = (window as unknown as Record<string, unknown>).__gesturaOpenFile;
      if (typeof fn === 'function') {
        (fn as (path: string, openOptions?: EditorOpenOptions) => void)(pending.relPath, pending.options);
      }
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

      // Cmd/Ctrl+T — toggle tasks panel
      if (event.key === 't' || event.key === 'T') {
        event.preventDefault();
        togglePanel('tasks');
        return;
      }

      // Cmd/Ctrl+` — toggle shell manager
      if (event.code === 'Backquote') {
        event.preventDefault();
        toggleShellManager();
        return;
      }

      // Cmd/Ctrl+S — forward to EditorArea via custom event; EditorArea uses
      // its own activeTabId so we don't need to duplicate tab state here.
      if ((event.key === 's' || event.key === 'S') && viewMode === 'editor') {
        event.preventDefault();
        window.dispatchEvent(new CustomEvent('gestura:editor:save'));
      }
    },
    [togglePanel, toggleShellManager, toggleViewMode, viewMode]
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
        ref={appRef}
        className={[
          'agent-app',
          isEditor ? 'agent-app--editor' : 'agent-app--message-only',
          isEditor && explorerOpen ? 'agent-app--explorer-open' : '',
          isEditor && chatOpen ? 'agent-app--chat-open' : '',
          headerOptionsOpen ? 'agent-app--header-options-open' : '',
          shellManager.visible ? 'agent-app--shell-open' : '',
          shellManager.visible && shellManager.mode === 'collapsed' ? 'agent-app--shell-collapsed' : '',
          // Triggers the CSS fade-in once theme + config are committed to the DOM.
          isReady ? 'app-ready' : '',
        ]
          .filter(Boolean)
          .join(' ')}
      >
        <div className="agent-app__main">
          {isEditor && (
            <ExplorerPanel
              key={`explorer:${sessionId}:${sessionWorkspace ?? ''}`}
              sessionId={sessionId}
              workspaceRoot={sessionWorkspace}
              onOpenFile={handleOpenFile}
              onWorkspaceChanged={handleWorkspaceChanged}
              onShowToast={toastState.showToast}
              style={explorerStyle}
            />
          )}
          {isEditor && (
            <div className="panel-resizer panel-resizer--left">
              <div className="panel-resizer__track" onMouseDown={headerOptionsOpen ? undefined : explorer.handleMouseDown} />
              <div
                className="panel-resizer__thumb panel-resizer__thumb--left"
                onMouseDown={headerOptionsOpen ? undefined : (e) => { e.stopPropagation(); explorer.handleMouseDown(e); }}
                onClick={headerOptionsOpen ? undefined : () => setExplorerOpen((v) => !v)}
                aria-disabled={headerOptionsOpen}
                title={explorerOpen ? 'Collapse Explorer (\u2318B)' : 'Expand Explorer (\u2318B)'}
              >
                <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                  <path d={explorerOpen ? 'M15 18l-6-6 6-6' : 'M9 18l6-6-6-6'} />
                </svg>
              </div>
            </div>
          )}
          <div className="agent-app__workspace" data-testid="agent-workspace">
            <AgentSessionHeader
              sessionId={sessionId}
              onShowToast={toastState.showToast}
              onOptionsOverlayOpenChange={setHeaderOptionsOpen}
            />

            <div className="agent-app__workspace-main">
              {isEditor && (
                <React.Suspense fallback={null}>
                  <EditorArea sessionId={sessionId} isDark={isDark} />
                </React.Suspense>
              )}
              {isEditor && (
                <div className="panel-resizer panel-resizer--right">
                  <div className="panel-resizer__track" onMouseDown={headerOptionsOpen ? undefined : chat.handleMouseDown} />
                  <div
                    className="panel-resizer__thumb panel-resizer__thumb--right"
                    onMouseDown={headerOptionsOpen ? undefined : (e) => { e.stopPropagation(); chat.handleMouseDown(e); }}
                    onClick={headerOptionsOpen ? undefined : () => setChatOpen((v) => !v)}
                    aria-disabled={headerOptionsOpen}
                    title={chatOpen ? 'Collapse Chat' : 'Expand Chat'}
                  >
                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                      <path d={chatOpen ? 'M9 18l6-6-6-6' : 'M15 18l-6-6 6-6'} />
                    </svg>
                  </div>
                </div>
              )}
              <ChatPanel
                sessionId={sessionId}
                onToggleEditor={toggleViewMode}
                onWorkspaceChanged={handleWorkspaceChanged}
                viewMode={viewMode}
                style={chatStyle}
                quickAccessHost={quickAccessHost}
                panelState={panelState}
                toastState={toastState}
              />
            </div>
          </div>
        </div>

        <ShellManagerPanel
          sessionId={sessionId}
          shells={orderedShellSessions}
          activeShellId={shellManager.activeShellId}
          visible={shellManager.visible}
          mode={shellManager.mode}
          height={shellManager.height}
          resizeBoundaryRef={appRef}
          defaultWorkingDirectory={sessionWorkspace}
          onSetMode={setShellManagerMode}
          onSetHeight={setShellManagerHeight}
          onActivateShell={setActiveShell}
          onReorderShellTabs={reorderShellTabs}
          onCloseShellTab={closeShellTab}
          onShowToast={toastState.showToast}
        />

        <div
          ref={setQuickAccessHost}
          className="agent-app__quick-access-host"
          data-testid="agent-quick-access-host"
        />

        <ToastContainer toasts={toastState.toasts} onDismiss={toastState.dismissToast} />
      </div>
    </AgentErrorBoundary>
  );
};

export default AgentApp;

