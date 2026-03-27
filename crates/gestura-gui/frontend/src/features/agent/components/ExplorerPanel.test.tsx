import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ExplorerPanel } from './ExplorerPanel';

const explorerGetRootMock = vi.fn();
const explorerListDirMock = vi.fn();
const explorerGitStatusMock = vi.fn();
const explorerOpenEntryInFileManagerMock = vi.fn();
const explorerOpenRootInFileManagerMock = vi.fn();
const pickWorkspaceDirectoryMock = vi.fn();

vi.mock('../../../services/tauri/explorer', () => ({
  explorerGetRoot: (sessionId: string) => explorerGetRootMock(sessionId),
  explorerListDir: (sessionId: string, dirRel: string) => explorerListDirMock(sessionId, dirRel),
  explorerGitStatus: (sessionId: string) => explorerGitStatusMock(sessionId),
  explorerOpenEntryInFileManager: (sessionId: string, relPath: string) => explorerOpenEntryInFileManagerMock(sessionId, relPath),
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
    explorerOpenEntryInFileManagerMock.mockReset();
    explorerOpenRootInFileManagerMock.mockReset();
    pickWorkspaceDirectoryMock.mockReset();

    explorerGetRootMock.mockResolvedValue({ root: '/workspace', is_git_repo: true });
    explorerListDirMock.mockResolvedValue({ root: '/workspace', dir_rel: '', entries: [], truncated: false });
    explorerGitStatusMock.mockResolvedValue({ root: '/workspace', is_git_repo: true, paths: {} });
    explorerOpenEntryInFileManagerMock.mockResolvedValue(undefined);
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

  it('opens markdown files in rendered view from the file tree context menu', async () => {
    const onOpenFile = vi.fn();
    explorerListDirMock.mockResolvedValue({
      root: '/workspace',
      dir_rel: '',
      entries: [{ name: 'README.md', rel_path: 'README.md', kind: 'file', is_symlink: false }],
      truncated: false,
    });

    const { container } = render(<ExplorerPanel sessionId="session-3" onOpenFile={onOpenFile} />);

    await waitFor(() => {
      expect(screen.getByText('README.md')).toBeInTheDocument();
    });

    const row = container.querySelector('.explorer-row');
    expect(row).not.toBeNull();

    fireEvent.contextMenu(row!);
    fireEvent.click(screen.getByRole('button', { name: /Rendered View/i }));

    expect(onOpenFile).toHaveBeenCalledWith('README.md', { viewMode: 'preview' });
  });

  it('shows a file in Finder from the file tree context menu', async () => {
    explorerListDirMock.mockResolvedValue({
      root: '/workspace',
      dir_rel: '',
      entries: [{ name: 'README.md', rel_path: 'README.md', kind: 'file', is_symlink: false }],
      truncated: false,
    });

    const { container } = render(<ExplorerPanel sessionId="session-4" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('README.md')).toBeInTheDocument();
    });

    const row = container.querySelector('.explorer-row');
    expect(row).not.toBeNull();

    fireEvent.contextMenu(row!);
    fireEvent.click(screen.getByRole('button', { name: /Show in /i }));

    await waitFor(() => {
      expect(explorerOpenEntryInFileManagerMock).toHaveBeenCalledWith('session-4', 'README.md');
    });
  });

  it('shows a directory in Finder from the file tree context menu', async () => {
    explorerListDirMock.mockResolvedValue({
      root: '/workspace',
      dir_rel: '',
      entries: [{ name: 'docs', rel_path: 'docs', kind: 'dir', is_symlink: false }],
      truncated: false,
    });

    const { container } = render(<ExplorerPanel sessionId="session-5" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('docs')).toBeInTheDocument();
    });

    const row = container.querySelector('.explorer-row');
    expect(row).not.toBeNull();

    fireEvent.contextMenu(row!);
    fireEvent.click(screen.getByRole('button', { name: /Show in /i }));

    await waitFor(() => {
      expect(explorerOpenEntryInFileManagerMock).toHaveBeenCalledWith('session-5', 'docs');
    });
  });
});