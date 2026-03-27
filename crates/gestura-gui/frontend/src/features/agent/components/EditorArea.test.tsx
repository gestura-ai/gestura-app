import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { EditorArea } from './EditorArea';
import { WORKSPACE_CHANGED_EVENT, WORKSPACE_ENTRY_RENAMED_EVENT } from '../utils/workspaceEvents';

const editorReadFileMock = vi.fn();
const editorRenameFileMock = vi.fn();
const editorWriteFileMock = vi.fn();
const editorGitDiffMock = vi.fn();

vi.mock('../../../services/tauri/editor', () => ({
  editorReadFile: (sessionId: string, relPath: string) => editorReadFileMock(sessionId, relPath),
  editorRenameFile: (sessionId: string, oldRelPath: string, newRelPath: string) => (
    editorRenameFileMock(sessionId, oldRelPath, newRelPath)
  ),
  editorWriteFile: (sessionId: string, relPath: string, content: string) => (
    editorWriteFileMock(sessionId, relPath, content)
  ),
  editorGitDiff: (sessionId: string, relPath: string) => editorGitDiffMock(sessionId, relPath),
}));

vi.mock('./TabBar', () => ({
  TabBar: ({
    tabs,
    onClose,
    onSaveTab,
    onRenameTab,
  }: {
    tabs: Array<{ id: string; label: string }>;
    onClose: (tabId: string, opts?: { force?: boolean }) => boolean;
    onSaveTab?: (tabId: string) => Promise<boolean>;
    onRenameTab?: (tabId: string, newLabel: string) => Promise<void>;
  }) => (
    <div>
      {tabs.map((tab) => <div key={tab.id}>{tab.label}</div>)}
      {tabs[0] && <button type="button" onClick={() => { void onRenameTab?.(tabs[0].id, 'guide.md'); }}>rename first tab</button>}
      {tabs[0] && <button type="button" onClick={() => {
        if (!onSaveTab) return;
        void onSaveTab(tabs[0].id).then((didSave) => {
          if (didSave !== false) onClose(tabs[0].id, { force: true });
        });
      }}>save close first tab</button>}
      {tabs[0] && <button type="button" onClick={() => { onClose(tabs[0].id, { force: true }); }}>discard close first tab</button>}
    </div>
  ),
}));

vi.mock('./EditorPane', () => ({
  EditorPane: ({
    tab,
    onContentChange,
  }: {
    tab: { id: string; relPath: string; content: string };
    onContentChange: (tabId: string, content: string) => void;
  }) => (
    <div>
      <div data-testid="active-rel-path">{tab.relPath}</div>
      <button type="button" onClick={() => onContentChange(tab.id, `${tab.content} changed`)}>dirty active tab</button>
    </div>
  ),
}));

vi.mock('./DiffPane', () => ({
  DiffPane: () => <div>diff</div>,
}));

describe('EditorArea rename sync', () => {
  beforeEach(() => {
    sessionStorage.clear();
    editorReadFileMock.mockReset();
    editorRenameFileMock.mockReset();
    editorWriteFileMock.mockReset();
    editorGitDiffMock.mockReset();
    editorReadFileMock.mockResolvedValue({ kind: 'text', content: '# hello' });
    editorRenameFileMock.mockResolvedValue(undefined);
    editorWriteFileMock.mockResolvedValue(undefined);
    editorGitDiffMock.mockResolvedValue({ original: '' });
  });

  afterEach(() => {
    cleanup();
    delete (window as unknown as { __gesturaOpenFile?: unknown }).__gesturaOpenFile;
  });

  it('updates open tab labels and paths when the explorer renames a file', async () => {
    render(<EditorArea sessionId="session-sync" isDark={false} />);

    await waitFor(() => {
      expect(typeof (window as unknown as { __gesturaOpenFile?: unknown }).__gesturaOpenFile).toBe('function');
    });

    await act(async () => {
      await ((window as unknown as { __gesturaOpenFile?: (relPath: string) => Promise<void> }).__gesturaOpenFile?.('docs/readme.md'));
    });

    await waitFor(() => {
      expect(screen.getByText('readme.md')).toBeInTheDocument();
      expect(screen.getByTestId('active-rel-path')).toHaveTextContent('docs/readme.md');
    });

    act(() => {
      window.dispatchEvent(new CustomEvent(WORKSPACE_ENTRY_RENAMED_EVENT, {
        detail: { oldRelPath: 'docs/readme.md', newRelPath: 'docs/guide.md' },
      }));
    });

    await waitFor(() => {
      expect(screen.getByText('guide.md')).toBeInTheDocument();
      expect(screen.getByTestId('active-rel-path')).toHaveTextContent('docs/guide.md');
    });
  });

  it('broadcasts a workspace refresh when a tab rename succeeds', async () => {
    const workspaceChanged = vi.fn();
    window.addEventListener(WORKSPACE_CHANGED_EVENT, workspaceChanged);

    render(<EditorArea sessionId="session-sync-2" isDark={false} />);

    await act(async () => {
      await ((window as unknown as { __gesturaOpenFile?: (relPath: string) => Promise<void> }).__gesturaOpenFile?.('docs/readme.md'));
    });

    fireEvent.click(await screen.findByRole('button', { name: 'rename first tab' }));

    await waitFor(() => {
      expect(editorRenameFileMock).toHaveBeenCalledWith('session-sync-2', 'docs/readme.md', 'docs/guide.md');
      expect(screen.getByText('guide.md')).toBeInTheDocument();
      expect(workspaceChanged).toHaveBeenCalled();
    });

    window.removeEventListener(WORKSPACE_CHANGED_EVENT, workspaceChanged);
  });

  it('saves a dirty tab before closing it', async () => {
    render(<EditorArea sessionId="session-close-save" isDark={false} />);

    await act(async () => {
      await ((window as unknown as { __gesturaOpenFile?: (relPath: string) => Promise<void> }).__gesturaOpenFile?.('docs/readme.md'));
    });

    fireEvent.click(await screen.findByRole('button', { name: 'dirty active tab' }));
    fireEvent.click(screen.getByRole('button', { name: 'save close first tab' }));

    await waitFor(() => {
      expect(editorWriteFileMock).toHaveBeenCalledWith('session-close-save', 'docs/readme.md', '# hello changed');
      expect(screen.queryByTestId('active-rel-path')).not.toBeInTheDocument();
      expect(screen.queryByText('readme.md')).not.toBeInTheDocument();
    });
  });

  it('discards a dirty tab without saving and closes it', async () => {
    render(<EditorArea sessionId="session-close-discard" isDark={false} />);

    await act(async () => {
      await ((window as unknown as { __gesturaOpenFile?: (relPath: string) => Promise<void> }).__gesturaOpenFile?.('docs/readme.md'));
    });

    fireEvent.click(await screen.findByRole('button', { name: 'dirty active tab' }));
    fireEvent.click(screen.getByRole('button', { name: 'discard close first tab' }));

    await waitFor(() => {
      expect(editorWriteFileMock).not.toHaveBeenCalled();
      expect(screen.queryByTestId('active-rel-path')).not.toBeInTheDocument();
      expect(screen.queryByText('readme.md')).not.toBeInTheDocument();
    });
  });
});