import { act, render, screen } from '@testing-library/react';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ShellSessionRecord } from './types';

let shouldCrashChatPanel = false;
let mockViewMode: 'message-only' | 'editor' = 'message-only';
let latestWorkspaceChanged: ((workspace: string) => void) | undefined;
const useShellSessionsMock = vi.fn((sessionId: string): ShellSessionRecord[] => {
  void sessionId;
  return [];
});
const getSessionWorkspaceByIdMock = vi.fn((sessionId: string): Promise<string> => {
  void sessionId;
  return Promise.resolve('/workspace');
});

const mockWindow = {
  setMinSize: vi.fn().mockResolvedValue(undefined),
  setSize: vi.fn().mockResolvedValue(undefined),
  show: vi.fn().mockResolvedValue(undefined),
  setFocus: vi.fn().mockResolvedValue(undefined),
};

const getConfigMock = vi.fn();

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => mockWindow,
}));

vi.mock('../../app/ThemeController', () => ({
  default: () => null,
}));

vi.mock('./hooks/useViewMode', () => ({
  useViewMode: () => ({
    viewMode: mockViewMode,
    toggleViewMode: vi.fn(),
  }),
}));

vi.mock('../../shared/hooks/useKeyboardShortcuts', () => ({
  useKeyboardShortcuts: () => undefined,
}));

vi.mock('./hooks/usePanelResize', () => ({
  usePanelResize: () => ({
    width: 320,
    handleMouseDown: vi.fn(),
  }),
}));

vi.mock('./hooks/useShellSessions', () => ({
  useShellSessions: (sessionId: string) => useShellSessionsMock(sessionId),
}));

vi.mock('../../services/tauri/config', () => ({
  getConfig: (...args: unknown[]) => getConfigMock(...args),
}));

vi.mock('../../services/tauri/agent', () => ({
  getSessionWorkspaceById: (sessionId: string) => getSessionWorkspaceByIdMock(sessionId),
}));

vi.mock('./components/ChatPanel', () => ({
  ChatPanel: ({
    sessionId,
    quickAccessHost,
    onWorkspaceChanged,
  }: {
    sessionId: string;
    quickAccessHost?: HTMLElement | null;
    onWorkspaceChanged?: (workspace: string) => void;
  }) => {
    if (shouldCrashChatPanel) {
      throw new Error('chat panel boot failure');
    }
    latestWorkspaceChanged = onWorkspaceChanged;
    return (
      <div
        data-testid="chat-panel"
        data-quick-access-host-attached={String(Boolean(quickAccessHost))}
      >
        chat:{sessionId}
        <button type="button" onClick={() => onWorkspaceChanged?.('/workspace/updated')}>
          change workspace
        </button>
      </div>
    );
  },
}));

vi.mock('./components/ExplorerPanel', () => ({
  ExplorerPanel: ({
    sessionId,
    workspaceRoot,
  }: {
    sessionId: string;
    workspaceRoot?: string | null;
  }) => {
    return (
      <div data-testid="explorer-panel" data-workspace-root={workspaceRoot ?? ''}>
        explorer:{sessionId}
      </div>
    );
  },
}));

vi.mock('./components/ShellManagerPanel', () => ({
  ShellManagerPanel: ({
    visible,
    shells,
    defaultWorkingDirectory,
  }: {
    visible: boolean;
    shells: Array<{ shellSessionId: string }>;
    defaultWorkingDirectory?: string | null;
  }) => (
    <div
      data-testid="shell-manager-panel"
      data-visible={String(visible)}
      data-shell-count={shells.length}
      data-working-directory={defaultWorkingDirectory ?? ''}
    />
  ),
}));

vi.mock('./components/AgentSessionHeader', () => ({
  AgentSessionHeader: ({ sessionId }: { sessionId: string }) => (
    <div data-testid="agent-session-header">header:{sessionId}</div>
  ),
}));

import AgentApp from './AgentApp';

describe('AgentApp', () => {
  beforeEach(() => {
    shouldCrashChatPanel = false;
    mockViewMode = 'message-only';
    latestWorkspaceChanged = undefined;
    getConfigMock.mockReset();
    useShellSessionsMock.mockReset();
    useShellSessionsMock.mockReturnValue([]);
    getSessionWorkspaceByIdMock.mockReset();
    getSessionWorkspaceByIdMock.mockResolvedValue('/workspace');
    mockWindow.setMinSize.mockClear();
    mockWindow.setSize.mockClear();
    mockWindow.show.mockClear();
    mockWindow.setFocus.mockClear();
    window.sessionStorage.clear();
    window.localStorage.clear();
    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: vi.fn().mockImplementation(() => ({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    });
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
  });

  it('reveals the agent UI on the next startup tick even when config loading hangs', async () => {
    getConfigMock.mockReturnValue(new Promise(() => undefined));

    const { container } = render(<AgentApp sessionId="session-timeout" />);

    expect(screen.getByTestId('chat-panel')).toHaveTextContent('chat:session-timeout');
    expect(container.querySelector('.agent-app')?.className).not.toContain('app-ready');

    await act(async () => {
      vi.advanceTimersByTime(1);
      await Promise.resolve();
    });

    expect(container.querySelector('.agent-app')?.className).toContain('app-ready');
    expect(mockWindow.setMinSize).toHaveBeenCalledWith(new LogicalSize(500, 320));
    expect(mockWindow.setSize).toHaveBeenCalled();
    expect(mockWindow.setSize).toHaveBeenCalledWith(new LogicalSize(550, 600));
    expect(mockWindow.show).toHaveBeenCalled();
    expect(mockWindow.setFocus).toHaveBeenCalled();
  });

  it('tracks active shell sessions without auto-opening the shell manager', async () => {
    getConfigMock.mockResolvedValue({
      ui: { theme_mode: 'system', accent: 'blue' },
    });
    useShellSessionsMock.mockReturnValue([
      {
        kind: 'shell-session',
        id: 'shell-001',
        shellSessionId: 'shell-001',
        cwd: '/workspace',
        state: 'Busy',
        interactive: true,
        userManaged: false,
        activeProcessId: 'proc-1',
        activeCommand: 'cargo test',
        lastExitCode: null,
        durationMs: null,
        lines: [],
        collapsed: false,
        availableForReuse: false,
      },
    ]);

    render(<AgentApp sessionId="session-shells" />);

    const managers = screen.getAllByTestId('shell-manager-panel');
    const manager = managers[managers.length - 1];
    expect(manager).toBeDefined();
    expect(manager).toHaveAttribute('data-visible', 'false');
    expect(manager).toHaveAttribute('data-shell-count', '1');
  });

  it('renders a top header and bottom dock around the main content', async () => {
    getConfigMock.mockResolvedValue({
      ui: { theme_mode: 'system', accent: 'blue' },
    });

    const { container } = render(<AgentApp sessionId="session-layout" />);

    const header = container.querySelector('[data-testid="agent-session-header"]');
    const chatPanel = container.querySelector('[data-testid="chat-panel"]');
    const main = container.querySelector('.agent-app__main');
    const workspace = container.querySelector('[data-testid="agent-workspace"]');
    const shellManager = container.querySelector('[data-testid="shell-manager-panel"]');
    const quickAccessHost = container.querySelector('[data-testid="agent-quick-access-host"]');

    expect(header).not.toBeNull();
    expect(chatPanel).not.toBeNull();
    expect(main).not.toBeNull();
    expect(workspace).not.toBeNull();
    expect(shellManager).not.toBeNull();
    expect(quickAccessHost).not.toBeNull();
    expect(chatPanel).toHaveAttribute('data-quick-access-host-attached', 'true');
    expect(workspace as HTMLElement).toContainElement(header as HTMLElement);
    expect(workspace as HTMLElement).toContainElement(chatPanel as HTMLElement);

    const relativePosition = shellManager!.compareDocumentPosition(quickAccessHost!);
    expect(relativePosition & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('keeps the header aligned with the workspace side in editor mode', async () => {
    mockViewMode = 'editor';
    getConfigMock.mockResolvedValue({
      ui: { theme_mode: 'system', accent: 'blue' },
    });

    const { container } = render(<AgentApp sessionId="session-editor-layout" />);

    const main = container.querySelector('.agent-app__main');
    const explorer = container.querySelector('[data-testid="explorer-panel"]');
    const workspace = container.querySelector('[data-testid="agent-workspace"]');
    const header = container.querySelector('[data-testid="agent-session-header"]');

    expect(main).not.toBeNull();
    expect(explorer).not.toBeNull();
    expect(workspace).not.toBeNull();
    expect(header).not.toBeNull();
    expect(main as HTMLElement).toContainElement(explorer as HTMLElement);
    expect(main as HTMLElement).toContainElement(workspace as HTMLElement);
    expect(workspace as HTMLElement).toContainElement(header as HTMLElement);

    const relativePosition = explorer!.compareDocumentPosition(workspace!);
    expect(relativePosition & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('remounts the explorer and updates the shell workspace when the project directory changes', async () => {
    mockViewMode = 'editor';
    getConfigMock.mockResolvedValue({
      ui: { theme_mode: 'system', accent: 'blue' },
    });
    getSessionWorkspaceByIdMock.mockImplementation(() => new Promise<string>(() => { }));

    const { container } = render(<AgentApp sessionId="session-workspace" />);

    const getShellManager = () => container.querySelector('[data-testid="shell-manager-panel"]');
    const getExplorer = () => container.querySelector('[data-testid="explorer-panel"]');

    expect(getExplorer()).toHaveAttribute('data-workspace-root', '');
    expect(getShellManager()).toHaveAttribute('data-working-directory', '');
    expect(latestWorkspaceChanged).toBeDefined();

    act(() => {
      latestWorkspaceChanged?.('/workspace/updated');
    });

    expect(getExplorer()).toHaveAttribute('data-workspace-root', '/workspace/updated');
    expect(getShellManager()).toHaveAttribute('data-working-directory', '/workspace/updated');
  });

  it('renders a visible fallback instead of a blank window when the agent UI crashes', async () => {
    shouldCrashChatPanel = true;
    getConfigMock.mockResolvedValue({
      ui: { theme_mode: 'system', accent: 'blue' },
    });

    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);

    render(<AgentApp sessionId="session-crash" />);

    expect(screen.getByText('Agent session failed to load')).toBeInTheDocument();
    expect(screen.getByText(/Session: session-crash/)).toBeInTheDocument();

    consoleError.mockRestore();
  });
});