/**
 * ExplorerPanel — left-side file-tree panel for the three-panel editor layout.
 *
 * Features:
 * - Lazy-loads directory contents via Tauri explorer_list_dir
 * - Git status decorations on file entries
 * - Double-click to open a file in the editor
 * - Right-click context menu: New File, New Folder, Rename, Delete
 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import {
  explorerGetRoot,
  explorerListDir,
  explorerGitStatus,
} from '../../../services/tauri/explorer';
import {
  editorCreateFile,
  editorDeleteFile,
  editorRenameFile,
} from '../../../services/tauri/editor';
import type { ExplorerEntry, GitChangeKind } from '../types';
import './ExplorerPanel.css';

// ─── helpers ─────────────────────────────────────────────────────────────────

function gitBadge(s?: GitChangeKind | null): string {
  if (!s || s === 'Unmodified') return '';
  const map: Partial<Record<GitChangeKind, string>> = {
    Added: 'A', Modified: 'M', Deleted: 'D', Renamed: 'R',
    Copied: 'C', Untracked: 'U', Conflicted: '!',
  };
  return map[s] ?? '';
}

function gitClass(s?: GitChangeKind | null): string {
  if (!s || s === 'Unmodified') return '';
  if (s === 'Added' || s === 'Untracked') return 'git-added';
  if (s === 'Modified' || s === 'Renamed' || s === 'Copied') return 'git-modified';
  if (s === 'Deleted') return 'git-deleted';
  if (s === 'Conflicted') return 'git-conflict';
  return '';
}

function fileIconClass(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase() ?? '';
  // Data / config
  if (ext === 'json' || ext === 'jsonl') return 'icon-file-json-02';
  if (ext === 'yaml' || ext === 'yml') return 'icon-file-yaml-02';
  if (ext === 'toml' || ext === 'ini' || ext === 'cfg' || ext === 'conf' || ext === 'properties') return 'icon-file-config-02';
  // Markup / style
  if (ext === 'html' || ext === 'htm' || ext === 'xml' || ext === 'svg') return 'icon-file-code-02';
  if (ext === 'css' || ext === 'scss' || ext === 'sass' || ext === 'less') return 'icon-file-code-02';
  // Code
  if (['rs', 'py', 'js', 'jsx', 'ts', 'tsx', 'go', 'java', 'kt', 'c', 'h', 'cpp', 'hpp', 'cs', 'swift'].includes(ext)) return 'icon-file-code-02';
  // Docs
  if (ext === 'txt' || ext === 'md' || ext === 'rst' || ext === 'log') return 'icon-file-text-02';
  // Shell
  if (ext === 'sh' || ext === 'bash' || ext === 'zsh' || ext === 'fish') return 'icon-terminal';
  return 'icon-file-02';
}

// ─── types ────────────────────────────────────────────────────────────────────

interface ContextMenu {
  x: number; y: number;
  entry?: ExplorerEntry;
  parentDir: string;
}

export interface ExplorerPanelProps {
  sessionId: string;
  onOpenFile: (relPath: string) => void;
}

// ─── ExplorerPanel ───────────────────────────────────────────────────────────

export const ExplorerPanel: React.FC<ExplorerPanelProps> = ({ sessionId, onOpenFile }) => {
  const [root, setRoot] = useState<string>('');
  const [isGitRepo, setIsGitRepo] = useState(false);
  const [rootEntries, setRootEntries] = useState<ExplorerEntry[]>([]);
  const [gitStatus, setGitStatus] = useState<Record<string, GitChangeKind>>({});
  const [ctxMenu, setCtxMenu] = useState<ContextMenu | null>(null);
  const [renaming, setRenaming] = useState<ExplorerEntry | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [newName, setNewName] = useState('');
  const [creating, setCreating] = useState<{ parentDir: string; isDir: boolean } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // Incremented whenever the agent mutates the workspace (file/shell/git tool completes).
  const [refreshKey, setRefreshKey] = useState(0);

  // Mount-only: resolve project root path and git status once.
  useEffect(() => {
    explorerGetRoot(sessionId).then((res) => {
      setRoot(res.root);
      setIsGitRepo(res.is_git_repo);
    }).catch(() => { });
  }, [sessionId]);

  // Refresh the root file listing whenever sessionId or refreshKey changes.
  useEffect(() => {
    explorerListDir(sessionId, '').then((res) => {
      setRootEntries(res.entries.sort((a, b) =>
        (b.is_dir ? 1 : 0) - (a.is_dir ? 1 : 0) || a.name.localeCompare(b.name)));
    }).catch(() => { });
  }, [sessionId, refreshKey]);

  // Refresh git decorations whenever sessionId, git-repo flag, or refreshKey changes.
  useEffect(() => {
    if (!isGitRepo) return;
    explorerGitStatus(sessionId).then((res) => setGitStatus(res.paths)).catch(() => { });
  }, [isGitRepo, sessionId, refreshKey]);

  // Listen for agent workspace mutations and trigger a refresh.
  useEffect(() => {
    const handler = () => setRefreshKey((k) => k + 1);
    window.addEventListener('gestura:workspace:changed', handler);
    return () => window.removeEventListener('gestura:workspace:changed', handler);
  }, []);

  // Close menu on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ctxMenu && menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setCtxMenu(null);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [ctxMenu]);

  const handleCtxMenu = useCallback((e: React.MouseEvent, entry?: ExplorerEntry) => {
    e.preventDefault();
    const parentDir = entry?.is_dir ? entry.rel_path : (entry?.rel_path.split('/').slice(0, -1).join('/') ?? '');
    setCtxMenu({ x: e.clientX, y: e.clientY, entry, parentDir });
  }, []);

  const handleCreate = useCallback(async (isDir: boolean) => {
    if (!newName.trim() || !creating) return;
    const rel = creating.parentDir ? `${creating.parentDir}/${newName.trim()}` : newName.trim();
    await editorCreateFile(sessionId, rel, isDir).catch(() => { });
    setCreating(null); setNewName('');
    const res = await explorerListDir(sessionId, '').catch(() => null);
    if (res) setRootEntries(res.entries.sort((a, b) =>
      (b.is_dir ? 1 : 0) - (a.is_dir ? 1 : 0) || a.name.localeCompare(b.name)));
  }, [creating, newName, sessionId]);

  const handleRename = useCallback(async () => {
    if (!renaming || !renameValue.trim()) return;
    const dir = renaming.rel_path.split('/').slice(0, -1).join('/');
    const newPath = dir ? `${dir}/${renameValue.trim()}` : renameValue.trim();
    await editorRenameFile(sessionId, renaming.rel_path, newPath).catch(() => { });
    setRenaming(null); setRenameValue('');
    const res = await explorerListDir(sessionId, '').catch(() => null);
    if (res) setRootEntries(res.entries.sort((a, b) =>
      (b.is_dir ? 1 : 0) - (a.is_dir ? 1 : 0) || a.name.localeCompare(b.name)));
  }, [renaming, renameValue, sessionId]);

  const handleDelete = useCallback(async (entry: ExplorerEntry) => {
    if (!confirm(`Delete "${entry.name}"?`)) return;
    await editorDeleteFile(sessionId, entry.rel_path).catch(() => { });
    const res = await explorerListDir(sessionId, '').catch(() => null);
    if (res) setRootEntries(res.entries.sort((a, b) =>
      (b.is_dir ? 1 : 0) - (a.is_dir ? 1 : 0) || a.name.localeCompare(b.name)));
  }, [sessionId]);

  const rootName = root.split('/').pop() ?? root;

  return (
    <div className="agent-panel agent-panel--explorer">
      <div className="explorer-panel" onContextMenu={(e) => handleCtxMenu(e)}>
        <div className="explorer-header">
          <div className="explorer-title-container">
            <span className="icon-folder" aria-hidden="true" />
            <span className="explorer-title" title={root}>{rootName || 'Explorer'}</span>
          </div>
          {isGitRepo && <span className="explorer-git-badge" title="Git repository">⎇</span>}
        </div>

        <div className="explorer-tree" role="tree">
          {rootEntries.map((entry) => (
            <TreeNode
              key={entry.rel_path}
              entry={entry}
              sessionId={sessionId}
              depth={0}
              gitStatus={gitStatus}
              refreshKey={refreshKey}
              onOpenFile={onOpenFile}
              onContextMenu={handleCtxMenu}
            />
          ))}
        </div>

        {/* Rename inline input */}
        {renaming && (
          <div className="explorer-rename-overlay">
            <input
              className="explorer-rename-input"
              value={renameValue}
              autoFocus
              onChange={(e) => setRenameValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void handleRename();
                if (e.key === 'Escape') { setRenaming(null); setRenameValue(''); }
              }}
              onBlur={() => { setRenaming(null); setRenameValue(''); }}
            />
          </div>
        )}

        {/* Create inline input */}
        {creating && (
          <div className="explorer-rename-overlay">
            <input
              className="explorer-rename-input"
              placeholder={creating.isDir ? 'Folder name…' : 'File name…'}
              value={newName}
              autoFocus
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void handleCreate(creating.isDir);
                if (e.key === 'Escape') { setCreating(null); setNewName(''); }
              }}
              onBlur={() => { setCreating(null); setNewName(''); }}
            />
          </div>
        )}

        {/* Context menu */}
        {ctxMenu && (
          <div
            ref={menuRef}
            className="context-menu"
            style={{ top: ctxMenu.y, left: ctxMenu.x }}
          >
            <button onClick={() => { setCtxMenu(null); setCreating({ parentDir: ctxMenu.parentDir, isDir: false }); setNewName(''); }}>
              📄 New File
            </button>
            <button onClick={() => { setCtxMenu(null); setCreating({ parentDir: ctxMenu.parentDir, isDir: true }); setNewName(''); }}>
              📁 New Folder
            </button>
            {ctxMenu.entry && <>
              <div className="context-menu-sep" />
              <button onClick={() => {
                if (!ctxMenu.entry) return;
                setCtxMenu(null); setRenaming(ctxMenu.entry); setRenameValue(ctxMenu.entry.name);
              }}>
                ✏️ Rename
              </button>
              <button className="ctx-delete" onClick={() => { const e = ctxMenu.entry; setCtxMenu(null); if (e) void handleDelete(e); }}>
                🗑 Delete
              </button>
            </>}
          </div>
        )}
      </div>
    </div>
  );
};

// ─── TreeNode ─────────────────────────────────────────────────────────────────

interface TreeNodeProps {
  entry: ExplorerEntry;
  sessionId: string;
  depth: number;
  gitStatus: Record<string, GitChangeKind>;
  /** Incremented by ExplorerPanel when the workspace changes; causes open dirs to re-fetch. */
  refreshKey: number;
  onOpenFile: (relPath: string) => void;
  onContextMenu: (e: React.MouseEvent, entry: ExplorerEntry) => void;
}

const TreeNode: React.FC<TreeNodeProps> = ({
  entry, sessionId, depth, gitStatus, refreshKey, onOpenFile, onContextMenu,
}) => {
  const [open, setOpen] = useState(false);
  const [children, setChildren] = useState<ExplorerEntry[]>([]);
  const [loading, setLoading] = useState(false);

  const toggle = useCallback(async () => {
    if (!entry.is_dir) { onOpenFile(entry.rel_path); return; }
    if (!open && children.length === 0) {
      setLoading(true);
      try {
        const res = await explorerListDir(sessionId, entry.rel_path);
        setChildren(res.entries.sort((a, b) =>
          (b.is_dir ? 1 : 0) - (a.is_dir ? 1 : 0) || a.name.localeCompare(b.name)));
      } finally { setLoading(false); }
    }
    setOpen((v) => !v);
  }, [entry, open, children, sessionId, onOpenFile]);

  // Re-fetch children when a workspace change is signalled and this node is open.
  useEffect(() => {
    if (!open || !entry.is_dir || refreshKey === 0) return;
    explorerListDir(sessionId, entry.rel_path)
      .then((res) => setChildren(res.entries.sort((a, b) =>
        (b.is_dir ? 1 : 0) - (a.is_dir ? 1 : 0) || a.name.localeCompare(b.name))))
      .catch(() => { });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey]);

  const status = gitStatus[entry.rel_path] ?? entry.git_status;
  const badge = gitBadge(status);
  const cls = gitClass(status);

  const iconClass = entry.is_dir
    ? (open ? 'icon-folder-open' : 'icon-folder')
    : fileIconClass(entry.name);

  return (
    <div className="explorer-node">
      <div
        className="explorer-row"
        style={{ '--explorer-depth': depth } as React.CSSProperties}
        onDoubleClick={() => { if (!entry.is_dir) onOpenFile(entry.rel_path); }}
        onClick={toggle}
        onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, entry); }}
        role="treeitem"
        aria-expanded={entry.is_dir ? open : undefined}
        tabIndex={0}
        title={entry.rel_path}
        onKeyDown={(e) => { if (e.key === 'Enter') void toggle(); }}
      >
        <div className="explorer-indent" />
        <div className="explorer-icon">
          {entry.is_dir && loading
            ? <span className="icon-loader-01" />
            : <span className={iconClass} />
          }
        </div>
        <div className={`explorer-name ${cls}`}>{entry.name}</div>
        <div className="explorer-meta">
          <div className="explorer-git-badges">
            {badge && <span className={`git-badge ${cls}`} title={String(status)}>{badge}</span>}
          </div>
        </div>
      </div>
      {open && children.map((child) => (
        <TreeNode
          key={child.rel_path}
          entry={child}
          sessionId={sessionId}
          depth={depth + 1}
          gitStatus={gitStatus}
          refreshKey={refreshKey}
          onOpenFile={onOpenFile}
          onContextMenu={onContextMenu}
        />
      ))}
    </div>
  );
};

