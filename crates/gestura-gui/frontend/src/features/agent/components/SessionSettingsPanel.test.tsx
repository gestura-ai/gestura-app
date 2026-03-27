import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { SessionSettingsPanel } from './SessionSettingsPanel';

const getSessionWorkspaceByIdMock = vi.fn();
const pickWorkspaceDirectoryMock = vi.fn();
const getSessionReflectionSettingsMock = vi.fn();
const setSessionReflectionEnabledMock = vi.fn();
const clearSessionReflectionSettingsMock = vi.fn();
const getConfigMock = vi.fn();
const saveConfigMock = vi.fn();
const listBuiltinToolsMock = vi.fn();
const connectMcpServerMock = vi.fn();
const listConnectedMcpServersMock = vi.fn();
const listMcpClientToolsMock = vi.fn();
const listMcpToolsMock = vi.fn();

vi.mock('../../../services/tauri/agent', () => ({
  clearSessionReflectionSettings: (sessionId: string) => clearSessionReflectionSettingsMock(sessionId),
  getSessionReflectionSettings: (sessionId: string) => getSessionReflectionSettingsMock(sessionId),
  getSessionWorkspaceById: (sessionId: string) => getSessionWorkspaceByIdMock(sessionId),
  pickWorkspaceDirectory: (sessionId: string) => pickWorkspaceDirectoryMock(sessionId),
  setSessionPermissionLevel: vi.fn().mockResolvedValue(undefined),
  setSessionReflectionEnabled: (sessionId: string, enabled: boolean) =>
    setSessionReflectionEnabledMock(sessionId, enabled),
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
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    getSessionWorkspaceByIdMock.mockReset();
    pickWorkspaceDirectoryMock.mockReset();
    getSessionReflectionSettingsMock.mockReset();
    setSessionReflectionEnabledMock.mockReset();
    clearSessionReflectionSettingsMock.mockReset();
    getConfigMock.mockReset();
    saveConfigMock.mockReset();
    listBuiltinToolsMock.mockReset();
    connectMcpServerMock.mockReset();
    listConnectedMcpServersMock.mockReset();
    listMcpClientToolsMock.mockReset();
    listMcpToolsMock.mockReset();
    getSessionWorkspaceByIdMock.mockResolvedValue('/workspace');
    pickWorkspaceDirectoryMock.mockResolvedValue('/workspace/updated');
    getSessionReflectionSettingsMock.mockResolvedValue(null);
    setSessionReflectionEnabledMock.mockResolvedValue(undefined);
    clearSessionReflectionSettingsMock.mockResolvedValue(undefined);
    getConfigMock.mockResolvedValue({
      pipeline: {
        reflection: { enabled: true },
        iteration_budget_enabled: false,
        max_iterations: 10,
        tracked_task_max_iterations: 30,
      },
    });
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

  it('allows a session to override reflection independently of the global default', async () => {
    const onShowToast = vi.fn();

    render(
      <SessionSettingsPanel
        isOpen
        onClose={vi.fn()}
        sessionId="session-123"
        toolSettings={{}}
        onShowToast={onShowToast}
      />
    );

    const select = await screen.findByDisplayValue('Use global default (currently enabled)');
    fireEvent.change(select, { target: { value: 'disabled' } });

    await waitFor(() => {
      expect(setSessionReflectionEnabledMock).toHaveBeenCalledWith('session-123', false);
    });
    expect(onShowToast).toHaveBeenCalledWith('Reflection disabled for this session', 'success');
  });

  it('allows clearing a session reflection override back to the global default', async () => {
    const onShowToast = vi.fn();
    getSessionReflectionSettingsMock.mockResolvedValueOnce({ enabled: false });

    render(
      <SessionSettingsPanel
        isOpen
        onClose={vi.fn()}
        sessionId="session-123"
        toolSettings={{}}
        onShowToast={onShowToast}
      />
    );

    const select = await screen.findByDisplayValue('Disabled for this session');
    fireEvent.change(select, { target: { value: 'global' } });

    await waitFor(() => {
      expect(clearSessionReflectionSettingsMock).toHaveBeenCalledWith('session-123');
    });
    expect(onShowToast).toHaveBeenCalledWith(
      'Session reflection now follows the global default',
      'success',
    );
  });
});