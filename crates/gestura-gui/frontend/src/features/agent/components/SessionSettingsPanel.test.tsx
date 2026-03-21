import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { SessionSettingsPanel } from './SessionSettingsPanel';

const getSessionWorkspaceByIdMock = vi.fn();
const pickWorkspaceDirectoryMock = vi.fn();
const getConfigMock = vi.fn();
const saveConfigMock = vi.fn();
const listBuiltinToolsMock = vi.fn();
const connectMcpServerMock = vi.fn();
const listConnectedMcpServersMock = vi.fn();
const listMcpClientToolsMock = vi.fn();
const listMcpToolsMock = vi.fn();

vi.mock('../../../services/tauri/agent', () => ({
  getSessionWorkspaceById: (sessionId: string) => getSessionWorkspaceByIdMock(sessionId),
  pickWorkspaceDirectory: (sessionId: string) => pickWorkspaceDirectoryMock(sessionId),
  setSessionPermissionLevel: vi.fn().mockResolvedValue(undefined),
  setSessionToolEnabled: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../../services/tauri/config', () => ({
  getConfig: () => getConfigMock(),
  saveConfig: (...args: unknown[]) => saveConfigMock(...args),
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

describe('SessionSettingsPanel', () => {
  beforeEach(() => {
    getSessionWorkspaceByIdMock.mockReset();
    pickWorkspaceDirectoryMock.mockReset();
    getConfigMock.mockReset();
    saveConfigMock.mockReset();
    listBuiltinToolsMock.mockReset();
    connectMcpServerMock.mockReset();
    listConnectedMcpServersMock.mockReset();
    listMcpClientToolsMock.mockReset();
    listMcpToolsMock.mockReset();
    getSessionWorkspaceByIdMock.mockResolvedValue('/workspace');
    pickWorkspaceDirectoryMock.mockResolvedValue('/workspace/updated');
    getConfigMock.mockResolvedValue({ pipeline: { reflection: { enabled: false } } });
    saveConfigMock.mockResolvedValue(undefined);
    listBuiltinToolsMock.mockResolvedValue([]);
    connectMcpServerMock.mockResolvedValue(undefined);
    listConnectedMcpServersMock.mockResolvedValue([]);
    listMcpClientToolsMock.mockResolvedValue([]);
    listMcpToolsMock.mockResolvedValue([]);
  });

  it('notifies the parent when the workspace directory changes', async () => {
    const onWorkspaceChanged = vi.fn();
    const onShowToast = vi.fn();

    render(
      <SessionSettingsPanel
        isOpen
        onClose={vi.fn()}
        sessionId="session-123"
        toolSettings={{}}
        onRefreshToolSettings={vi.fn().mockResolvedValue(undefined)}
        onWorkspaceChanged={onWorkspaceChanged}
        onShowToast={onShowToast}
      />
    );

    fireEvent.click(screen.getByRole('button', { name: 'Change' }));

    await waitFor(() => {
      expect(onWorkspaceChanged).toHaveBeenCalledWith('/workspace/updated');
    });
    expect(onShowToast).toHaveBeenCalledWith('Workspace updated', 'success');
  });
});