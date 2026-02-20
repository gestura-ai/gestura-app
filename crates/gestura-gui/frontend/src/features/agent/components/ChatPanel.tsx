/**
 * ChatPanel — agent chat panel (header + messages + input + all side panels).
 * Orchestrates useChatSession, usePanelState, useToast, and renders all overlays.
 *
 * Header matches agent.html: inline SVG icon, "Gestura Agent", status badge,
 * settings gear (opens MenuPanel). View-mode toggle lives in MessageInput quick-bar.
 */
import React, { useCallback, useEffect } from 'react';
import '../ChatPanel.css';
import { useChatSession } from '../hooks/useChatSession';
import { usePanelState } from '../hooks/usePanelState';
import { useToast } from '../hooks/useToast';
import { MessageList } from './MessageList';
import { MessageInput } from './MessageInput';
import { ToolConfirmationDialog } from './ToolConfirmationDialog';
import { ToastContainer } from './ToastContainer';
import { MenuPanel } from './MenuPanel';
import { TaskPanel } from './TaskPanel';
import { KnowledgePanel } from './KnowledgePanel';
import { ProvidersPanel } from './ProvidersPanel';
import { SessionSettingsPanel } from './SessionSettingsPanel';
import type { PanelName } from '../hooks/usePanelState';
import { openShellForSession } from '../../../services/tauri/agent';

export interface ChatPanelProps {
  sessionId: string;
  /** Called when the user clicks the explorer/message toggle in the quick access bar. */
  onToggleEditor?: () => void;
  /** Current view mode — drives the quick access bar icon swap. */
  viewMode?: 'message-only' | 'editor';
}

export const ChatPanel: React.FC<ChatPanelProps> = ({ sessionId, onToggleEditor, viewMode }) => {
  const {
    messages, streamingMessage, isProcessing, isListening, status,
    pendingConfirmation, tasks, knowledgeItems, toolSettings,
    userScrolledUp, setUserScrolledUp,
    sendMessage, cancelStream, resolveConfirmation,
    toggleVoice, enhanceText,
    refreshTasks, refreshKnowledge, refreshToolSettings,
  } = useChatSession(sessionId);

  const { isOpen, openPanel, closePanel, togglePanel } = usePanelState();
  const { toasts, showToast, dismissToast } = useToast();

  // Cmd/Ctrl+T — open tasks panel
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key === 't' || e.key === 'T')) {
        e.preventDefault();
        togglePanel('tasks');
      }
    };
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [togglePanel]);

  // External link handler — open https:// links in system browser
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

  const badgeClass =
    status.kind === 'busy' ? ' busy' :
      status.kind === 'listening' ? ' active' :
        status.kind === 'error' ? ' error' : '';

  return (
    <div className="agent-panel agent-panel--chat">
      {/* Header */}
      <div className="header">
        <div className="header-title">
          <span className="icon-message" aria-hidden="true">
            <svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg" aria-hidden="true" focusable="false">
              <path d="M21 15C21 15.5304 20.7893 16.0391 20.4142 16.4142C20.0391 16.7893 19.5304 17 19 17H7L3 21V5C3 4.46957 3.21071 3.96086 3.58579 3.58579C3.96086 3.21071 4.46957 3 5 3H19C19.5304 3 20.0391 3.21071 20.4142 3.58579C20.7893 3.96086 21 4.46957 21 5V15Z" />
            </svg>
          </span>
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

      {/* Messages */}
      <MessageList messages={messages} streamingMessage={streamingMessage}
        sessionId={sessionId} onScrollChange={handleScrollChange} />

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

      {/* Input + Quick Access Bar */}
      <MessageInput
        isProcessing={isProcessing} isListening={isListening} status={status}
        onSend={handleSend} onCancel={handleCancel} onVoiceToggle={toggleVoice}
        onEnhance={handleEnhance} viewMode={viewMode} onToggleEditor={onToggleEditor}
        onOpenTasks={() => togglePanel('tasks')}
        onOpenTerminal={handleOpenTerminal}
        sessionId={sessionId}
      />

      {/* ── Side Panels ── */}
      <MenuPanel isOpen={isOpen('menu')} onClose={closePanel} onNavigate={handleMenuNavigate} />

      <TaskPanel isOpen={isOpen('tasks')} onClose={closePanel} sessionId={sessionId}
        tasks={tasks} onRefreshTasks={refreshTasks} onSendMessage={sendMessage}
        onShowToast={showToast} />

      <KnowledgePanel isOpen={isOpen('knowledge')} onClose={closePanel}
        sessionId={sessionId}
        knowledgeItems={knowledgeItems} onRefreshKnowledge={refreshKnowledge}
        onShowToast={showToast} />

      <ProvidersPanel isOpen={isOpen('providers')} onClose={closePanel}
        sessionId={sessionId} onShowToast={showToast} />

      <SessionSettingsPanel isOpen={isOpen('settings')} onClose={closePanel}
        sessionId={sessionId} toolSettings={toolSettings}
        onRefreshToolSettings={refreshToolSettings} onShowToast={showToast} />

      {/* Toast Notifications */}
      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </div>
  );
};

export default ChatPanel;

