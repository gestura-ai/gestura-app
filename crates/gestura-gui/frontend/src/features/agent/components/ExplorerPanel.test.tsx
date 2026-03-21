import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ExplorerPanel } from './ExplorerPanel';

const explorerGetRootMock = vi.fn();
const explorerListDirMock = vi.fn();
const explorerGitStatusMock = vi.fn();
const explorerOpenRootInFileManagerMock = vi.fn();
const pickWorkspaceDirectoryMock = vi.fn();

vi.mock('../../../services/tauri/explorer', () => ({
  explorerGetRoot: (sessionId: string) => explorerGetRootMock(sessionId),
  explorerListDir: (sessionId: string, dirRel: string) => explorerListDirMock(sessionId, dirRel),
  explorerGitStatus: (sessionId: string) => explorerGitStatusMock(sessionId),
  explorerOpenRootInFileManager: (sessionId: string) => explorerOpenRootInFileManagerMock(sessionId),
}));

vi.mock('../../../services/tauri/agent', () => ({
  pickWorkspaceDirectory: (sessionId: string) => pickWorkspaceDirectoryMock(sessionId),
}));

describe('ExplorerPanel header menu', () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    explorerGetRootMock.mockReset();
    explorerListDirMock.mockReset();
    explorerGitStatusMock.mockReset();
    explorerOpenRootInFileManagerMock.mockReset();
    pickWorkspaceDirectoryMock.mockReset();

    explorerGetRootMock.mockResolvedValue({ root: '/workspace', is_git_repo: true });
    explorerListDirMock.mockResolvedValue({ root: '/workspace', dir_rel: '', entries: [], truncated: false });
    explorerGitStatusMock.mockResolvedValue({ root: '/workspace', is_git_repo: true, paths: {} });
    explorerOpenRootInFileManagerMock.mockResolvedValue(undefined);
    pickWorkspaceDirectoryMock.mockResolvedValue('/workspace/updated');
  });

  it('opens the project root in the system file manager from the header menu', async () => {
    render(<ExplorerPanel sessionId="session-1" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('Project Files')).toHaveAttribute('title', '/workspace');
    });

    fireEvent.click(screen.getByRole('button', { name: 'Project file actions' }));
    fireEvent.click(screen.getByRole('menuitem', { name: /Open in /i }));

    await waitFor(() => {
      expect(explorerOpenRootInFileManagerMock).toHaveBeenCalledWith('session-1');
    });
  });

  it('changes the project root from the header menu and notifies the parent', async () => {
    const onWorkspaceChanged = vi.fn();
    const onShowToast = vi.fn();

    render(
      <ExplorerPanel
        sessionId="session-2"
        onOpenFile={vi.fn()}
        onWorkspaceChanged={onWorkspaceChanged}
        onShowToast={onShowToast}
      />
    );

    await waitFor(() => {
      expect(screen.getByText('Project Files')).toHaveAttribute('title', '/workspace');
    });

    fireEvent.click(screen.getByRole('button', { name: 'Project file actions' }));
    fireEvent.click(screen.getByRole('menuitem', { name: /Change project root/i }));

    await waitFor(() => {
      expect(pickWorkspaceDirectoryMock).toHaveBeenCalledWith('session-2');
      expect(onWorkspaceChanged).toHaveBeenCalledWith('/workspace/updated');
    });
    expect(onShowToast).toHaveBeenCalledWith('Workspace updated', 'success');
  });
});