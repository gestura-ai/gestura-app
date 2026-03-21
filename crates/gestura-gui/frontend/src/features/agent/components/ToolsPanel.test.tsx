import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ToolsPanel } from './ToolsPanel';

const listBuiltinToolsMock = vi.fn();
const setSessionToolEnabledMock = vi.fn();
const connectMcpServerMock = vi.fn();
const listConnectedMcpServersMock = vi.fn();
const listMcpClientToolsMock = vi.fn();
const listMcpToolsMock = vi.fn();

vi.mock('../../../services/tauri/agent', () => ({
  setSessionToolEnabled: (...args: unknown[]) => setSessionToolEnabledMock(...args),
}));

vi.mock('../../../services/tauri/tools', () => ({
  listBuiltinTools: () => listBuiltinToolsMock(),
}));

vi.mock('../../../services/tauri/mcp', () => ({
  connectMcpServer: (name: string) => connectMcpServerMock(name),
  listConnectedMcpServers: () => listConnectedMcpServersMock(),
  listMcpClientTools: () => listMcpClientToolsMock(),
  listMcpTools: () => listMcpToolsMock(),
}));

afterEach(() => {
  cleanup();
});

describe('ToolsPanel', () => {
  beforeEach(() => {
    listBuiltinToolsMock.mockReset();
    setSessionToolEnabledMock.mockReset();
    connectMcpServerMock.mockReset();
    listConnectedMcpServersMock.mockReset();
    listMcpClientToolsMock.mockReset();
    listMcpToolsMock.mockReset();

    listBuiltinToolsMock.mockResolvedValue([{ name: 'shell', summary: 'Run shell commands' }]);
    setSessionToolEnabledMock.mockResolvedValue(undefined);
    connectMcpServerMock.mockResolvedValue(undefined);
    listConnectedMcpServersMock.mockResolvedValue([]);
    listMcpClientToolsMock.mockResolvedValue([]);
    listMcpToolsMock.mockResolvedValue([]);
  });

  it('renders the Tools title with the tools icon and builtin tool toggles', async () => {
    render(
      <ToolsPanel
        isOpen
        onClose={vi.fn()}
        sessionId="session-123"
        toolSettings={{ enabled_tools: {} }}
        onRefreshToolSettings={vi.fn().mockResolvedValue(undefined)}
        onShowToast={vi.fn()}
      />,
    );

    expect(screen.getByRole('heading', { name: 'Tools' })).toBeInTheDocument();
    expect(document.querySelector('.session-panel-title-icon.icon-tools')).not.toBeNull();

    await waitFor(() => {
      expect(screen.getByText('shell')).toBeInTheDocument();
    });
    expect(screen.getByText('Run shell commands')).toBeInTheDocument();
  });

  it('toggles a builtin tool for the current session', async () => {
    const onRefreshToolSettings = vi.fn().mockResolvedValue(undefined);

    render(
      <ToolsPanel
        isOpen
        onClose={vi.fn()}
        sessionId="session-123"
        toolSettings={{ enabled_tools: { shell: true } }}
        onRefreshToolSettings={onRefreshToolSettings}
        onShowToast={vi.fn()}
      />,
    );

    const checkbox = await screen.findByRole('checkbox');
    fireEvent.click(checkbox);

    await waitFor(() => {
      expect(setSessionToolEnabledMock).toHaveBeenCalledWith('session-123', 'shell', false);
      expect(onRefreshToolSettings).toHaveBeenCalledTimes(1);
    });
  });
});