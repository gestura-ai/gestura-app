import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

let shouldCrashChatPanel = false;

const mockWindow = {
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
    viewMode: 'message-only',
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

vi.mock('../../services/tauri/config', () => ({
  getConfig: (...args: unknown[]) => getConfigMock(...args),
}));

vi.mock('./components/ChatPanel', () => ({
  ChatPanel: ({ sessionId }: { sessionId: string }) => {
    if (shouldCrashChatPanel) {
      throw new Error('chat panel boot failure');
    }
    return <div data-testid="chat-panel">chat:{sessionId}</div>;
  },
}));

import AgentApp from './AgentApp';

describe('AgentApp', () => {
  beforeEach(() => {
    shouldCrashChatPanel = false;
    getConfigMock.mockReset();
    mockWindow.setSize.mockClear();
    mockWindow.show.mockClear();
    mockWindow.setFocus.mockClear();
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

  it('reveals the agent UI after a bounded startup timeout when config loading hangs', async () => {
    getConfigMock.mockReturnValue(new Promise(() => undefined));

    const { container } = render(<AgentApp sessionId="session-timeout" />);

    expect(screen.getByTestId('chat-panel')).toHaveTextContent('chat:session-timeout');
    expect(container.querySelector('.agent-app')?.className).not.toContain('app-ready');

    await act(async () => {
      vi.advanceTimersByTime(1500);
      await Promise.resolve();
    });

    expect(container.querySelector('.agent-app')?.className).toContain('app-ready');
    expect(mockWindow.setSize).toHaveBeenCalled();
    expect(mockWindow.show).toHaveBeenCalled();
    expect(mockWindow.setFocus).toHaveBeenCalled();
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