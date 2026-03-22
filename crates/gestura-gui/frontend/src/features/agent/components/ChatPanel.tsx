/**
 * ChatPanel — agent chat panel (header + messages + overlays).
 * Orchestrates useChatSession and renders overlays using app-scoped panel/toast state.
 *
 * Header matches agent.html: Gestura brand logo, "Gestura Agent", status badge,
 * settings gear (opens MenuPanel). The header and quick launch bar are portaled
 * into app-level docks so they can span the full window, while the text input
 * and message list stay inside the chat panel.
 */
import React, { useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import '../ChatPanel.css';
import { useChatSession } from '../hooks/useChatSession';
import type { PanelName, PanelState } from '../hooks/usePanelState';
import type { ToastState } from '../hooks/useToast';
import { MessageList } from './MessageList';
import { MessageInput, QuickAccessBar } from './MessageInput';
import { ToolConfirmationDialog } from './ToolConfirmationDialog';
import { MenuPanel } from './MenuPanel';
import { TaskPanel } from './TaskPanel';
import { KnowledgePanel } from './KnowledgePanel';
import { MemoryConsolePanel } from '../../memory/components/MemoryConsolePanel';
import { ProvidersPanel } from './ProvidersPanel';
import { SessionSettingsPanel } from './SessionSettingsPanel';
import { ToolsPanel } from './ToolsPanel';
import { checkCliInstalled, openShellForSession } from '../../../services/tauri/agent';

export interface ChatPanelProps {
  sessionId: string;
  /** Called when the user clicks the explorer/message toggle in the quick access bar. */
  onToggleEditor?: () => void;
  /** Called when the session workspace changes from settings. */
  onWorkspaceChanged?: (workspace: string) => void;
  /** Current view mode — drives the quick access bar icon swap. */
  viewMode?: 'message-only' | 'editor';
  /** Optional inline style — used by parent for dynamic width (resizable panel). */
  style?: React.CSSProperties;
  /** Optional app-level host for the shared top header dock. */
  headerHost?: HTMLElement | null;
  /** Optional app-level host for the shared bottom quick-launch dock. */
  quickAccessHost?: HTMLElement | null;
  /** App-scoped overlay panel state. */
  panelState: PanelState;
  /** App-scoped toast state. */
  toastState: ToastState;
}

export const ChatPanel: React.FC<ChatPanelProps> = ({
  sessionId,
  onToggleEditor,
  onWorkspaceChanged,
  viewMode,
  style,
  headerHost,
  quickAccessHost,
  panelState,
  toastState,
}) => {
  const [cliInstalled, setCliInstalled] = useState(false);
  const {
    messages, streamingMessage, isProcessing, isStopping, isListening, status,
    pendingConfirmation, tasks, knowledgeItems, toolSettings, memoryRevision,
    userScrolledUp, setUserScrolledUp,
    sendMessage, cancelStream, resumeStream, canResume, isResuming, resolveConfirmation,
    toggleVoice, enhanceText,
    refreshTasks, refreshKnowledge, refreshToolSettings,
  } = useChatSession(sessionId);

  const {
    isOpen,
    openPanel,
    closePanel,
    togglePanel,
    shellManager,
    toggleShellManager,
    openShellManager,
    setActiveShell,
  } = panelState;
  const { showToast } = toastState;

  // External link handler — open https:// links in system browser
  useEffect(() => {
    let cancelled = false;

    checkCliInstalled()
      .then((installed) => {
        if (!cancelled) {
          setCliInstalled(installed);
        }
      })
      .catch((error) => {
        if (!cancelled) {
          console.warn('[ChatPanel] failed to determine CLI availability:', error);
          setCliInstalled(true);
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      const target = e.target as Element | null;
      const anchor = target?.closest('a[href]') as HTMLAnchorElement | null;
      if (!anchor) return;
      const href = anchor.getAttribute('href') ?? '';
      if (/^https?:\/\//i.test(href)) {
        e.preventDefault();
        // Use Tauri opener plugin if available, otherwise log
        const w = window as unknown as {
          __TAURI__?: { opener?: { openUrl?: (url: string) => Promise<void> } };
        };
        w.__TAURI__?.opener?.openUrl?.(href).catch((err: unknown) =>
          console.error('[opener] Failed to open URL:', href, err)
        );
      }
    };
    document.addEventListener('click', handleClick);
    return () => document.removeEventListener('click', handleClick);
  }, []);

  const handleScrollChange = useCallback((scrolled: boolean) => {
    setUserScrolledUp(scrolled);
  }, [setUserScrolledUp]);

  const handleSend = useCallback((text: string) => {
    void sendMessage(text);
  }, [sendMessage]);

  const handleCancel = useCallback(() => { void cancelStream(); }, [cancelStream]);
  const handleEnhance = useCallback((text: string) => enhanceText(text), [enhanceText]);

  const handleMenuNavigate = useCallback((panel: PanelName) => {
    openPanel(panel);
  }, [openPanel]);

  const handleOpenTerminal = useCallback(async () => {
    try {
      await openShellForSession(sessionId);
      showToast('Opened terminal for this session', 'success');
    } catch (e) {
      showToast(`Failed to open terminal: ${e}`, 'error');
    }
  }, [sessionId, showToast]);

  const handleRevealShellSession = useCallback((shellSessionId: string | null) => {
    openShellManager();
    if (shellSessionId) setActiveShell(shellSessionId);
  }, [openShellManager, setActiveShell]);

  const badgeClass =
    status.kind === 'busy' ? ' busy' :
      status.kind === 'reflection' ? ' busy' :
        status.kind === 'listening' ? ' active' :
          status.kind === 'error' ? ' error' : '';

  const quickAccessDock = (
    <QuickAccessBar
      viewMode={viewMode}
      onToggleEditor={onToggleEditor}
      onOpenTasks={() => togglePanel('tasks')}
      onToggleShellManager={toggleShellManager}
      shellManagerOpen={shellManager.visible}
      showOpenTerminal={cliInstalled}
      onOpenTerminal={handleOpenTerminal}
      dockMode="window"
    />
  );

  const header = (
    <div className="header">
      <div className="header-title">
        <img className="header-logo" src="/assets/gestura-app.svg" alt="" aria-hidden="true" />
        Gestura Agent
      </div>
      <div className="header-controls">
        <div className={`status-badge${badgeClass}`}>{status.text}</div>
        <button type="button" className="btn-settings" title="Menu"
          onClick={() => togglePanel('menu')}>
          <span className="icon-settings"></span>
        </button>
      </div>
    </div>
  );

  return (
    <div className="agent-panel agent-panel--chat" style={style}>
      {headerHost ? createPortal(header, headerHost) : header}

      <div className="chat-workspace">
        <MessageList messages={messages} streamingMessage={streamingMessage}
          onScrollChange={handleScrollChange}
          onRevealShellSession={handleRevealShellSession}
          canResume={canResume} isResuming={isResuming}
          onResume={() => { void resumeStream(); }} />
      </div>

      {/* Scroll-to-bottom indicator */}
      {userScrolledUp && (
        <div className="scroll-indicator" aria-live="polite">
          <button type="button" className="scroll-to-bottom-btn visible"
            onClick={() => setUserScrolledUp(false)}>
            ↓ New messages
          </button>
        </div>
      )}

      {/* Tool confirmation overlay */}
      {pendingConfirmation && (
        <ToolConfirmationDialog confirmation={pendingConfirmation} onDecide={resolveConfirmation} />
      )}

      <MessageInput
        isProcessing={isProcessing} isStopping={isStopping}
        isListening={isListening} status={status}
        onSend={handleSend} onCancel={handleCancel} onVoiceToggle={toggleVoice}
        onEnhance={handleEnhance} viewMode={viewMode}
      />

      {/* App-level bottom quick launch dock */}
      {quickAccessHost ? createPortal(quickAccessDock, quickAccessHost) : null}

      {/* ── Side Panels ── */}
      <MenuPanel isOpen={isOpen('menu')} onClose={closePanel} onNavigate={handleMenuNavigate} />

      <TaskPanel isOpen={isOpen('tasks')} onClose={closePanel} sessionId={sessionId}
        tasks={tasks} onRefreshTasks={refreshTasks} onSendMessage={sendMessage}
        onShowToast={showToast} />

      {isOpen('memory') && (
        <div className="session-panel-overlay visible" onClick={closePanel}>
          <div className="session-panel open" onClick={(event) => event.stopPropagation()}>
            <div className="task-panel-header">
              <div>
                <h3>Memory</h3>
                <p className="task-panel-subtitle">Session working memory + durable memory bank</p>
              </div>
              <button className="session-panel-close" onClick={closePanel} title="Close">
                <span className="icon-close" />
              </button>
            </div>
            <MemoryConsolePanel
              sessionId={sessionId}
              tasks={tasks}
              refreshSignal={memoryRevision}
              title="Session Memory"
            />
          </div>
        </div>
      )}

      <KnowledgePanel isOpen={isOpen('knowledge')} onClose={closePanel}
        sessionId={sessionId}
        knowledgeItems={knowledgeItems} onRefreshKnowledge={refreshKnowledge}
        onShowToast={showToast} />

      <ProvidersPanel isOpen={isOpen('providers')} onClose={closePanel}
        sessionId={sessionId} onShowToast={showToast} />

      <ToolsPanel isOpen={isOpen('tools')} onClose={closePanel}
        sessionId={sessionId} toolSettings={toolSettings}
        onRefreshToolSettings={refreshToolSettings}
        onShowToast={showToast} />

      <SessionSettingsPanel isOpen={isOpen('settings')} onClose={closePanel}
        sessionId={sessionId} toolSettings={toolSettings}
        onWorkspaceChanged={onWorkspaceChanged}
        onShowToast={showToast} />
    </div>
  );
};

export default ChatPanel;

