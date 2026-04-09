/**
 * ChatPanel — agent chat panel (messages + overlays).
 * Orchestrates useChatSession and renders overlays using app-scoped panel/toast state.
 *
 * The quick launch bar is portaled into the app-level bottom dock so it can span
 * the full window, while the text input and message list stay inside the chat panel.
 */
import React, { useCallback, useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import '../ChatPanel.css';
import { useChatSession } from '../hooks/useChatSession';
import type { PanelName, PanelState } from '../hooks/usePanelState';
import type { ToastState } from '../hooks/useToast';
import type { ShellSessionRecord } from '../types';
import { TASK_LINK_SCHEME } from '../utils/taskLinks';
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
import { checkCliInstalled, exportSessionJson, openShellForSession } from '../../../services/tauri/agent';

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
  /** Optional app-level host for the shared bottom quick-launch dock. */
  quickAccessHost?: HTMLElement | null;
  /** App-scoped overlay panel state. */
  panelState: PanelState;
  /** App-scoped toast state. */
  toastState: ToastState;
  /** Durable shell session state shared with Shell Manager. */
  shellSessions?: ShellSessionRecord[];
}

export const ChatPanel: React.FC<ChatPanelProps> = ({
  sessionId,
  onToggleEditor,
  onWorkspaceChanged,
  viewMode,
  style,
  quickAccessHost,
  panelState,
  toastState,
  shellSessions = [],
}) => {
  const [cliInstalled, setCliInstalled] = useState(false);
  const [highlightedTaskId, setHighlightedTaskId] = useState<string | null>(null);
  const {
    messages, streamingMessage, isProcessing, isStopping, isListening, status,
    pendingConfirmation, tasks, runtimeTaskSnapshot, knowledgeItems, toolSettings, memoryRevision,
    userScrolledUp, setUserScrolledUp,
    sendMessage, cancelStream, resumeStream, canResume, isResuming, resolveConfirmation,
    toggleVoice, enhanceText,
    refreshTasks, refreshKnowledge, refreshToolSettings,
  } = useChatSession(sessionId, { shellSessions });

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
  const tasksOpen = isOpen('tasks');
  const memoryOpen = isOpen('memory');
  const knowledgeOpen = isOpen('knowledge');
  const toolsOpen = isOpen('tools');
  const settingsOpen = isOpen('settings');

  useEffect(() => {
    if (tasksOpen || memoryOpen) {
      void refreshTasks();
    }
  }, [memoryOpen, refreshTasks, tasksOpen]);

  useEffect(() => {
    if (knowledgeOpen) {
      void refreshKnowledge();
    }
  }, [knowledgeOpen, refreshKnowledge]);

  useEffect(() => {
    if (toolsOpen || settingsOpen) {
      void refreshToolSettings();
    }
  }, [refreshToolSettings, settingsOpen, toolsOpen]);

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
      if (href.startsWith(TASK_LINK_SCHEME)) {
        e.preventDefault();
        const taskId = decodeURIComponent(href.slice(TASK_LINK_SCHEME.length));
        setHighlightedTaskId(taskId);
        openPanel('tasks');
        return;
      }
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
  }, [openPanel, refreshTasks]);

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

  const handleExportSession = useCallback(async () => {
    try {
      const exportPath = await exportSessionJson(sessionId);
      if (exportPath) {
        showToast(`Exported session JSON to ${exportPath}`, 'success');
      }
    } catch (e) {
      showToast(`Failed to export session JSON: ${e}`, 'error');
    }
  }, [sessionId, showToast]);

  const handleRevealShellSession = useCallback((shellSessionId: string | null) => {
    openShellManager();
    if (shellSessionId) setActiveShell(shellSessionId);
  }, [openShellManager, setActiveShell]);

  const quickAccessDock = (
    <QuickAccessBar
      viewMode={viewMode}
      onToggleEditor={onToggleEditor}
      onOpenMenu={() => togglePanel('menu')}
      onOpenTasks={() => togglePanel('tasks')}
      onToggleShellManager={toggleShellManager}
      shellManagerOpen={shellManager.visible}
      showOpenTerminal={cliInstalled}
      onOpenTerminal={handleOpenTerminal}
      status={status}
      dockMode="window"
    />
  );

  const overlayRoot = typeof document !== 'undefined' ? document.body : null;

  const sidePanels = (
    <>
      <MenuPanel
        isOpen={isOpen('menu')}
        onClose={closePanel}
        onNavigate={handleMenuNavigate}
        onExportSession={handleExportSession}
      />

      <TaskPanel isOpen={isOpen('tasks')} onClose={closePanel} sessionId={sessionId}
        tasks={tasks} onRefreshTasks={refreshTasks} onSendMessage={sendMessage}
        runtimeTaskSnapshot={runtimeTaskSnapshot}
        highlightedTaskId={tasksOpen ? highlightedTaskId : null}
        onShowToast={showToast} />

      <MemoryConsolePanel
        isOpen={isOpen('memory')}
        onClose={closePanel}
        sessionId={sessionId}
        tasks={tasks}
        refreshSignal={memoryRevision}
        title="Session Memory"
      />

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
    </>
  );

  return (
    <div className="agent-panel agent-panel--chat" style={style}>
      <div className="chat-workspace">
        <MessageList messages={messages} streamingMessage={streamingMessage}
          tasks={tasks}
          userScrolledUp={userScrolledUp}
          onScrollChange={handleScrollChange}
          onRevealShellSession={handleRevealShellSession}
          canResume={canResume} isResuming={isResuming}
          onResume={() => { void resumeStream(); }} />
      </div>

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
      {overlayRoot ? createPortal(sidePanels, overlayRoot) : sidePanels}
    </div>
  );
};

export default ChatPanel;

