import { cleanup, createEvent, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ExplorerPanel } from './ExplorerPanel';

const editorCreateFileMock = vi.fn();
const editorDeleteFileMock = vi.fn();
const editorRenameFileMock = vi.fn();
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

vi.mock('../../../services/tauri/editor', () => ({
  editorCreateFile: (sessionId: string, relPath: string, isDir: boolean) => editorCreateFileMock(sessionId, relPath, isDir),
  editorDeleteFile: (sessionId: string, relPath: string) => editorDeleteFileMock(sessionId, relPath),
  editorRenameFile: (sessionId: string, oldRelPath: string, newRelPath: string) => editorRenameFileMock(sessionId, oldRelPath, newRelPath),
}));

function explorerRowFor(name: string): HTMLElement {
  const row = screen.getByText(name).closest('.explorer-row');
  expect(row).not.toBeNull();
  return row as HTMLElement;
}

function firstElementByClass(className: string): HTMLElement {
  const element = document.querySelector(`.${className}`);
  expect(element).not.toBeNull();
  return element as HTMLElement;
}

describe('ExplorerPanel header menu', () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    editorCreateFileMock.mockReset();
    editorDeleteFileMock.mockReset();
    editorRenameFileMock.mockReset();
    explorerGetRootMock.mockReset();
    explorerListDirMock.mockReset();
    explorerGitStatusMock.mockReset();
    explorerOpenEntryInFileManagerMock.mockReset();
    explorerOpenRootInFileManagerMock.mockReset();
    pickWorkspaceDirectoryMock.mockReset();

    editorCreateFileMock.mockResolvedValue(undefined);
    editorDeleteFileMock.mockResolvedValue(undefined);
    editorRenameFileMock.mockResolvedValue(undefined);
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
    expect(onShowToast).not.toHaveBeenCalled();
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

  it('opens a folder in the tree when clicked', async () => {
    explorerListDirMock.mockImplementation(async (_sessionId: string, dirRel: string) => ({
      root: '/workspace',
      dir_rel: dirRel,
      entries: dirRel === ''
        ? [{ name: 'docs', rel_path: 'docs', kind: 'dir', is_symlink: false }]
        : [{ name: 'guide.md', rel_path: 'docs/guide.md', kind: 'file', is_symlink: false }],
      truncated: false,
    }));

    render(<ExplorerPanel sessionId="session-open-folder" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('docs')).toBeInTheDocument();
    });

    fireEvent.click(explorerRowFor('docs'));

    await waitFor(() => {
      expect(screen.getByText('guide.md')).toBeInTheDocument();
    });
  });

  it('prevents native text selection while starting an explorer drag', async () => {
    explorerListDirMock.mockResolvedValue({
      root: '/workspace',
      dir_rel: '',
      entries: [{ name: 'notes.md', rel_path: 'notes.md', kind: 'file', is_symlink: false }],
      truncated: false,
    });

    render(<ExplorerPanel sessionId="session-drag-select" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('notes.md')).toBeInTheDocument();
    });

    const event = createEvent.mouseDown(explorerRowFor('notes.md'), {
      button: 0,
      clientX: 12,
      clientY: 18,
    });

    fireEvent(explorerRowFor('notes.md'), event);

    expect(event.defaultPrevented).toBe(true);
    expect(document.body).toHaveClass('explorer-dragging-global');

    fireEvent.mouseUp(document, { clientX: 12, clientY: 18 });

    await waitFor(() => {
      expect(document.body).not.toHaveClass('explorer-dragging-global');
    });
  });

  it('clears file selection when clicking outside the explorer', async () => {
    explorerListDirMock.mockResolvedValue({
      root: '/workspace',
      dir_rel: '',
      entries: [{ name: 'notes.md', rel_path: 'notes.md', kind: 'file', is_symlink: false }],
      truncated: false,
    });

    render(
      <>
        <ExplorerPanel sessionId="session-selection-clear" onOpenFile={vi.fn()} />
        <button type="button">Elsewhere</button>
      </>
    );

    await waitFor(() => {
      expect(screen.getByText('notes.md')).toBeInTheDocument();
    });

    fireEvent.click(explorerRowFor('notes.md'));
    expect(explorerRowFor('notes.md')).toHaveClass('explorer-row--selected');

    fireEvent.mouseDown(screen.getByRole('button', { name: 'Elsewhere' }));

    await waitFor(() => {
      expect(explorerRowFor('notes.md')).not.toHaveClass('explorer-row--selected');
    });
  });

  it('deletes multiple selected files with the delete key', async () => {
    explorerListDirMock.mockResolvedValue({
      root: '/workspace',
      dir_rel: '',
      entries: [
        { name: 'alpha.ts', rel_path: 'alpha.ts', kind: 'file', is_symlink: false },
        { name: 'beta.ts', rel_path: 'beta.ts', kind: 'file', is_symlink: false },
      ],
      truncated: false,
    });

    render(<ExplorerPanel sessionId="session-6" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('alpha.ts')).toBeInTheDocument();
      expect(screen.getByText('beta.ts')).toBeInTheDocument();
    });

    fireEvent.click(explorerRowFor('alpha.ts'));
    fireEvent.click(explorerRowFor('beta.ts'), { ctrlKey: true });
    fireEvent.keyDown(document, { key: 'Delete' });

    expect(await screen.findByText('Delete 2 items?')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

    await waitFor(() => {
      expect(editorDeleteFileMock).toHaveBeenCalledTimes(2);
    });
    expect(editorDeleteFileMock.mock.calls).toEqual(expect.arrayContaining([
      ['session-6', 'alpha.ts'],
      ['session-6', 'beta.ts'],
    ]));
  });

  it('moves a file into a directory via drag and drop', async () => {
    explorerListDirMock.mockImplementation(async (_sessionId: string, dirRel: string) => ({
      root: '/workspace',
      dir_rel: dirRel,
      entries: dirRel === ''
        ? [
          { name: 'docs', rel_path: 'docs', kind: 'dir', is_symlink: false },
          { name: 'notes.md', rel_path: 'notes.md', kind: 'file', is_symlink: false },
        ]
        : [],
      truncated: false,
    }));

    render(<ExplorerPanel sessionId="session-7" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('docs')).toBeInTheDocument();
      expect(screen.getByText('notes.md')).toBeInTheDocument();
    });

    fireEvent.mouseDown(explorerRowFor('notes.md'), { button: 0, clientX: 10, clientY: 10 });
    fireEvent.mouseMove(document, { clientX: 24, clientY: 24 });
    fireEvent.mouseEnter(explorerRowFor('docs'));

    expect(explorerRowFor('docs')).toHaveClass('explorer-row--drop-target');

    fireEvent.mouseUp(document, { clientX: 24, clientY: 24 });

    await waitFor(() => {
      expect(editorRenameFileMock).toHaveBeenCalledWith('session-7', 'notes.md', 'docs/notes.md');
    });
  });

  it('moves a file into an open folder from the child list area', async () => {
    explorerListDirMock.mockImplementation(async (_sessionId: string, dirRel: string) => ({
      root: '/workspace',
      dir_rel: dirRel,
      entries: dirRel === ''
        ? [
          { name: 'docs', rel_path: 'docs', kind: 'dir', is_symlink: false },
          { name: 'notes.md', rel_path: 'notes.md', kind: 'file', is_symlink: false },
        ]
        : [{ name: 'guide.md', rel_path: 'docs/guide.md', kind: 'file', is_symlink: false }],
      truncated: false,
    }));

    render(<ExplorerPanel sessionId="session-8" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('docs')).toBeInTheDocument();
      expect(screen.getByText('notes.md')).toBeInTheDocument();
    });

    fireEvent.click(explorerRowFor('docs'));

    await waitFor(() => {
      expect(screen.getByText('guide.md')).toBeInTheDocument();
    });

    fireEvent.mouseDown(explorerRowFor('notes.md'), { button: 0, clientX: 20, clientY: 20 });
    fireEvent.mouseMove(document, { clientX: 34, clientY: 34 });
    fireEvent.mouseEnter(firstElementByClass('explorer-children'));
    fireEvent.mouseMove(firstElementByClass('explorer-children'));
    fireEvent.mouseUp(document, { clientX: 34, clientY: 34 });

    await waitFor(() => {
      expect(editorRenameFileMock).toHaveBeenCalledWith('session-8', 'notes.md', 'docs/notes.md');
    });
  });

  it('moves a file into the containing folder when dropped onto a file row inside that folder', async () => {
    explorerListDirMock.mockImplementation(async (_sessionId: string, dirRel: string) => ({
      root: '/workspace',
      dir_rel: dirRel,
      entries: dirRel === ''
        ? [
          { name: 'docs', rel_path: 'docs', kind: 'dir', is_symlink: false },
          { name: 'notes.md', rel_path: 'notes.md', kind: 'file', is_symlink: false },
        ]
        : [{ name: 'guide.md', rel_path: 'docs/guide.md', kind: 'file', is_symlink: false }],
      truncated: false,
    }));

    render(<ExplorerPanel sessionId="session-8b" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('docs')).toBeInTheDocument();
      expect(screen.getByText('notes.md')).toBeInTheDocument();
    });

    fireEvent.click(explorerRowFor('docs'));

    await waitFor(() => {
      expect(screen.getByText('guide.md')).toBeInTheDocument();
    });

    fireEvent.mouseDown(explorerRowFor('notes.md'), { button: 0, clientX: 20, clientY: 20 });
    fireEvent.mouseMove(document, { clientX: 34, clientY: 34 });
    fireEvent.mouseEnter(explorerRowFor('guide.md'));
    fireEvent.mouseUp(document, { clientX: 34, clientY: 34 });

    await waitFor(() => {
      expect(editorRenameFileMock).toHaveBeenCalledWith('session-8b', 'notes.md', 'docs/notes.md');
    });
  });

  it('moves a file to the project root from the root drop zone', async () => {
    explorerListDirMock.mockImplementation(async (_sessionId: string, dirRel: string) => ({
      root: '/workspace',
      dir_rel: dirRel,
      entries: dirRel === ''
        ? [{ name: 'docs', rel_path: 'docs', kind: 'dir', is_symlink: false }]
        : [{ name: 'guide.md', rel_path: 'docs/guide.md', kind: 'file', is_symlink: false }],
      truncated: false,
    }));

    render(<ExplorerPanel sessionId="session-9" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('docs')).toBeInTheDocument();
    });

    fireEvent.click(explorerRowFor('docs'));

    await waitFor(() => {
      expect(screen.getByText('guide.md')).toBeInTheDocument();
    });

    fireEvent.mouseDown(explorerRowFor('guide.md'), { button: 0, clientX: 20, clientY: 20 });
    fireEvent.mouseMove(document, { clientX: 36, clientY: 36 });
    fireEvent.mouseEnter(firstElementByClass('explorer-root-dropzone'));
    fireEvent.mouseUp(document, { clientX: 36, clientY: 36 });

    await waitFor(() => {
      expect(editorRenameFileMock).toHaveBeenCalledWith('session-9', 'docs/guide.md', 'guide.md');
    });
  });

  it('moves a file to the project root when dropped on a root-level file row', async () => {
    explorerListDirMock.mockImplementation(async (_sessionId: string, dirRel: string) => ({
      root: '/workspace',
      dir_rel: dirRel,
      entries: dirRel === ''
        ? [
          { name: 'docs', rel_path: 'docs', kind: 'dir', is_symlink: false },
          { name: 'README.md', rel_path: 'README.md', kind: 'file', is_symlink: false },
        ]
        : [{ name: 'guide.md', rel_path: 'docs/guide.md', kind: 'file', is_symlink: false }],
      truncated: false,
    }));

    render(<ExplorerPanel sessionId="session-10" onOpenFile={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText('docs')).toBeInTheDocument();
      expect(screen.getByText('README.md')).toBeInTheDocument();
    });

    fireEvent.click(explorerRowFor('docs'));

    await waitFor(() => {
      expect(screen.getByText('guide.md')).toBeInTheDocument();
    });

    fireEvent.mouseDown(explorerRowFor('guide.md'), { button: 0, clientX: 20, clientY: 20 });
    fireEvent.mouseMove(document, { clientX: 36, clientY: 36 });
    fireEvent.mouseEnter(explorerRowFor('README.md'));
    fireEvent.mouseUp(document, { clientX: 36, clientY: 36 });

    await waitFor(() => {
      expect(editorRenameFileMock).toHaveBeenCalledWith('session-10', 'docs/guide.md', 'guide.md');
    });
  });
});