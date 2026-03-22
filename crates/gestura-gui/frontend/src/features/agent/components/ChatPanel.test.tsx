import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PanelState } from '../hooks/usePanelState';
import type { ToastState } from '../hooks/useToast';
import { ChatPanel } from './ChatPanel';

const useChatSessionMock = vi.fn();
const checkCliInstalledMock = vi.fn();

vi.mock('../hooks/useChatSession', () => ({
  useChatSession: (sessionId: string) => useChatSessionMock(sessionId),
}));

vi.mock('../../../services/tauri/agent', () => ({
  checkCliInstalled: (...args: unknown[]) => checkCliInstalledMock(...args),
  openShellForSession: vi.fn(),
}));

vi.mock('./MessageList', () => ({
  MessageList: () => <div data-testid="message-list" />,
}));

vi.mock('./MessageInput', async () => {
  const actual = await vi.importActual<typeof import('./MessageInput')>('./MessageInput');
  return {
    ...actual,
    MessageInput: () => <div data-testid="message-input" />,
  };
});

vi.mock('./ToolConfirmationDialog', () => ({
  ToolConfirmationDialog: () => <div data-testid="tool-confirmation-dialog" />,
}));

vi.mock('./MenuPanel', () => ({
  MenuPanel: () => null,
}));

vi.mock('./TaskPanel', () => ({
  TaskPanel: () => null,
}));

vi.mock('./KnowledgePanel', () => ({
  KnowledgePanel: () => null,
}));

vi.mock('../../memory/components/MemoryConsolePanel', () => ({
  MemoryConsolePanel: () => null,
}));

vi.mock('./ProvidersPanel', () => ({
  ProvidersPanel: () => null,
}));

vi.mock('./SessionSettingsPanel', () => ({
  SessionSettingsPanel: () => null,
}));

vi.mock('./ToolsPanel', () => ({
  ToolsPanel: () => null,
}));

const basePanelState: PanelState = {
  activePanel: 'none',
  isOpen: () => false,
  openPanel: vi.fn(),
  closePanel: vi.fn(),
  togglePanel: vi.fn(),
  shellManager: {
    visible: false,
    mode: 'expanded',
    height: 220,
    activeShellId: null,
    tabOrder: [],
    closedShellIds: [],
  },
  openShellManager: vi.fn(),
  closeShellManager: vi.fn(),
  toggleShellManager: vi.fn(),
  setShellManagerMode: vi.fn(),
  setShellManagerHeight: vi.fn(),
  setActiveShell: vi.fn(),
  syncShellTabs: vi.fn(),
  reorderShellTabs: vi.fn(),
  closeShellTab: vi.fn(),
};

const toastState: ToastState = {
  toasts: [],
  showToast: vi.fn(),
  dismissToast: vi.fn(),
};

describe('ChatPanel CLI quick access visibility', () => {
  beforeEach(() => {
    useChatSessionMock.mockReturnValue({
      messages: [],
      streamingMessage: null,
      isProcessing: false,
      isStopping: false,
      isListening: false,
      status: { text: 'Ready', kind: 'ready' },
      pendingConfirmation: null,
      tasks: [],
      knowledgeItems: [],
      toolSettings: {},
      memoryRevision: 0,
      userScrolledUp: false,
      setUserScrolledUp: vi.fn(),
      sendMessage: vi.fn(),
      cancelStream: vi.fn(),
      resumeStream: vi.fn(),
      canResume: false,
      isResuming: false,
      resolveConfirmation: vi.fn(),
      toggleVoice: vi.fn(),
      enhanceText: vi.fn(),
      refreshTasks: vi.fn(),
      refreshKnowledge: vi.fn(),
      refreshToolSettings: vi.fn(),
    });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it('hides the open-in-shell quick access button when the CLI is not installed', async () => {
    checkCliInstalledMock.mockResolvedValue(false);
    const quickAccessHost = document.createElement('div');
    document.body.appendChild(quickAccessHost);

    render(
      <ChatPanel
        sessionId="session-no-cli"
        quickAccessHost={quickAccessHost}
        panelState={basePanelState}
        toastState={toastState}
      />
    );

    await waitFor(() => {
      expect(checkCliInstalledMock).toHaveBeenCalledTimes(1);
    });

    expect(screen.queryByTitle('Open Session in Shell')).not.toBeInTheDocument();
    quickAccessHost.remove();
  });

  it('shows the open-in-shell quick access button when the CLI is installed', async () => {
    checkCliInstalledMock.mockResolvedValue(true);
    const quickAccessHost = document.createElement('div');
    document.body.appendChild(quickAccessHost);

    render(
      <ChatPanel
        sessionId="session-with-cli"
        quickAccessHost={quickAccessHost}
        panelState={basePanelState}
        toastState={toastState}
      />
    );

    await waitFor(() => {
      expect(screen.getByTitle('Open Session in Shell')).toBeInTheDocument();
    });

    quickAccessHost.remove();
  });
});