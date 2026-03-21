/**
 * ExplorerPanel — left-side file-tree panel for the three-panel editor layout.
 *
 * Features:
 * - Lazy-loads directory contents via Tauri explorer_list_dir
 * - Git status decorations on file entries
 * - Double-click to open a file in the editor
 * - Right-click context menu: New File, New Folder, Rename, Delete
 */
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  explorerGetRoot,
  explorerListDir,
  explorerGitStatus,
  explorerOpenRootInFileManager,
} from '../../../services/tauri/explorer';
import { pickWorkspaceDirectory } from '../../../services/tauri/agent';
import {
  editorCreateFile,
  editorDeleteFile,
  editorRenameFile,
} from '../../../services/tauri/editor';
import type {
  ExplorerEntry,
  ExplorerGitChangeKind,
  ExplorerGitPathStatus,
} from '../types';
import './ExplorerPanel.css';

// ─── helpers ─────────────────────────────────────────────────────────────────

/** Returns true when the entry is a directory (matches Rust ExplorerEntryKind). */
const isDir = (e: ExplorerEntry): boolean => e.kind === 'dir';

/** Directories-first sort comparator. */
const dirFirstSort = (a: ExplorerEntry, b: ExplorerEntry): number =>
  (isDir(b) ? 1 : 0) - (isDir(a) ? 1 : 0) || a.name.localeCompare(b.name);



function kindPriority(kind: ExplorerGitChangeKind | null | undefined): number {
  switch (kind) {
    case 'deleted': return 60;
    case 'modified': return 50;
    case 'added': return 40;
    case 'renamed': return 30;
    case 'copied': return 20;
    case 'unknown': return 10;
    default: return 0;
  }
}

function pickBetterKind(
  a: ExplorerGitChangeKind | null | undefined,
  b: ExplorerGitChangeKind | null | undefined,
): ExplorerGitChangeKind | null {
  if (!a) return b ?? null;
  if (!b) return a ?? null;
  return kindPriority(b) > kindPriority(a) ? b : a;
}

function mergeAgg(
  into: ExplorerGitPathStatus | undefined,
  status: ExplorerGitPathStatus | null | undefined,
): ExplorerGitPathStatus {
  const out: ExplorerGitPathStatus = into ?? { staged: null, unstaged: null, untracked: false };
  if (status?.staged) out.staged = pickBetterKind(out.staged, status.staged);
  if (status?.unstaged) out.unstaged = pickBetterKind(out.unstaged, status.unstaged);
  if (status?.untracked) out.untracked = true;
  return out;
}

function computeGitDirAgg(gitPathsObj: Record<string, ExplorerGitPathStatus>): Map<string, ExplorerGitPathStatus> {
  const agg = new Map<string, ExplorerGitPathStatus>();
  for (const [rel, st] of Object.entries(gitPathsObj ?? {})) {
    // root aggregate
    agg.set('', mergeAgg(agg.get(''), st));
    const parts = rel.split('/').filter(Boolean);
    // all parent dirs
    for (let i = 0; i < Math.max(0, parts.length - 1); i++) {
      const dirRel = parts.slice(0, i + 1).join('/');
      agg.set(dirRel, mergeAgg(agg.get(dirRel), st));
    }
  }
  return agg;
}

function badgeForKind(
  kind: ExplorerGitChangeKind | null | undefined,
  isUntracked: boolean,
): { icon: string; cls: string; title: string } | null {
  if (isUntracked) return { icon: 'icon-git-untracked', cls: 'untracked', title: 'Untracked' };
  switch (String(kind ?? '').toLowerCase()) {
    case 'modified': return { icon: 'icon-git-modified', cls: 'modified', title: 'Modified' };
    case 'added': return { icon: 'icon-git-added', cls: 'added', title: 'Added' };
    case 'deleted': return { icon: 'icon-git-deleted', cls: 'deleted', title: 'Deleted' };
    case 'renamed': return { icon: 'icon-git-renamed', cls: 'renamed', title: 'Renamed' };
    case 'copied': return { icon: 'icon-git-copied', cls: 'copied', title: 'Copied' };
    case 'unknown': return { icon: 'icon-git-unknown', cls: 'unknown', title: 'Changed' };
    default: return null;
  }
}

function buildGitBadges(status: ExplorerGitPathStatus | null | undefined): Array<{
  key: string;
  icon: string;
  cls: string;
  stageCls: string;
  title: string;
}> {
  if (!status) return [];
  const out: Array<{ key: string; icon: string; cls: string; stageCls: string; title: string }> = [];

  const u = status.untracked ? badgeForKind(null, true) : null;
  if (u) out.push({ key: 'untracked', ...u, stageCls: 'stage-untracked', title: u.title });

  const unstaged = badgeForKind(status.unstaged, false);
  if (unstaged) out.push({ key: 'unstaged', ...unstaged, stageCls: 'stage-unstaged', title: `Working tree: ${unstaged.title}` });

  const staged = badgeForKind(status.staged, false);
  if (staged) out.push({ key: 'staged', ...staged, stageCls: 'stage-staged', title: `Staged: ${staged.title}` });

  return out;
}

function nameClassForStatus(status: ExplorerGitPathStatus | null | undefined): string {
  if (!status) return '';
  if (status.untracked) return 'git-untracked';
  const best = pickBetterKind(status.unstaged ?? null, status.staged ?? null);
  switch (best) {
    case 'added': return 'git-added';
    case 'modified': return 'git-modified';
    case 'deleted': return 'git-deleted';
    case 'renamed':
    case 'copied':
    case 'unknown':
      return 'git-modified';
    default:
      return '';
  }
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
  workspaceRoot?: string | null;
  onOpenFile: (relPath: string) => void;
  onWorkspaceChanged?: (workspace: string) => void;
  onShowToast?: (message: string, kind?: 'success' | 'error' | 'warning' | 'info') => void;
  style?: React.CSSProperties;
}

function fileManagerLabel(): string {
  if (typeof navigator === 'undefined') return 'File Manager';
  const platform = `${navigator.platform ?? ''} ${navigator.userAgent ?? ''}`.toLowerCase();
  if (platform.includes('mac')) return 'Finder';
  if (platform.includes('win')) return 'File Explorer';
  return 'File Manager';
}

// ─── ExplorerPanel ───────────────────────────────────────────────────────────

export const ExplorerPanel: React.FC<ExplorerPanelProps> = ({
  sessionId,
  workspaceRoot: _workspaceRoot,
  onOpenFile,
  onWorkspaceChanged,
  onShowToast,
  style,
}) => {
  const [root, setRoot] = useState<string>('');
  const [isGitRepo, setIsGitRepo] = useState(false);
  const [rootEntries, setRootEntries] = useState<ExplorerEntry[]>([]);
  const [gitStatus, setGitStatus] = useState<Record<string, ExplorerGitPathStatus>>({});
  const [ctxMenu, setCtxMenu] = useState<ContextMenu | null>(null);
  const [headerMenuOpen, setHeaderMenuOpen] = useState(false);
  const [renaming, setRenaming] = useState<ExplorerEntry | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [newName, setNewName] = useState('');
  const [creating, setCreating] = useState<{ parentDir: string; isDir: boolean } | null>(null);
  /** Entry pending delete confirmation — null means no dialog is open. */
  const [deleteConfirm, setDeleteConfirm] = useState<ExplorerEntry | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const headerMenuRef = useRef<HTMLDivElement>(null);

  // Incremented whenever the agent mutates the workspace (file/shell/git tool completes).
  const [refreshKey, setRefreshKey] = useState(0);

  // Aggregated git status for directories — derived from flat file-path gitStatus map.
  const gitDirAgg = useMemo(() => computeGitDirAgg(gitStatus), [gitStatus]);

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
      setRootEntries(res.entries.sort(dirFirstSort));
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
      if (headerMenuOpen && headerMenuRef.current && !headerMenuRef.current.contains(e.target as Node)) {
        setHeaderMenuOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [ctxMenu, headerMenuOpen]);

  // Dismiss the delete-confirm dialog on Escape
  useEffect(() => {
    if (!deleteConfirm && !headerMenuOpen) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return;
      if (deleteConfirm) setDeleteConfirm(null);
      if (headerMenuOpen) setHeaderMenuOpen(false);
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [deleteConfirm, headerMenuOpen]);

  const handleCtxMenu = useCallback((e: React.MouseEvent, entry?: ExplorerEntry) => {
    e.preventDefault();
    const parentDir = entry && isDir(entry) ? entry.rel_path : (entry?.rel_path.split('/').slice(0, -1).join('/') ?? '');
    setCtxMenu({ x: e.clientX, y: e.clientY, entry, parentDir });
  }, []);

  const handleCreate = useCallback(async (isDir: boolean) => {
    if (!newName.trim() || !creating) return;
    const rel = creating.parentDir ? `${creating.parentDir}/${newName.trim()}` : newName.trim();
    await editorCreateFile(sessionId, rel, isDir).catch(() => { });
    setCreating(null); setNewName('');
    const res = await explorerListDir(sessionId, '').catch(() => null);
    if (res) setRootEntries(res.entries.sort(dirFirstSort));
  }, [creating, newName, sessionId]);

  const handleRename = useCallback(async () => {
    if (!renaming || !renameValue.trim()) return;
    const dir = renaming.rel_path.split('/').slice(0, -1).join('/');
    const newPath = dir ? `${dir}/${renameValue.trim()}` : renameValue.trim();
    await editorRenameFile(sessionId, renaming.rel_path, newPath).catch(() => { });
    setRenaming(null); setRenameValue('');
    const res = await explorerListDir(sessionId, '').catch(() => null);
    if (res) setRootEntries(res.entries.sort(dirFirstSort));
  }, [renaming, renameValue, sessionId]);

  const handleStartRename = useCallback((entry: ExplorerEntry) => {
    setRenaming(entry);
    setRenameValue(entry.name);
  }, []);

  /** Executes the actual delete — only called after the confirm dialog is accepted. */
  const handleDelete = useCallback(async (entry: ExplorerEntry) => {
    await editorDeleteFile(sessionId, entry.rel_path).catch(() => { });
    const res = await explorerListDir(sessionId, '').catch(() => null);
    if (res) setRootEntries(res.entries.sort(dirFirstSort));
  }, [sessionId]);

  const openInLabel = useMemo(() => fileManagerLabel(), []);

  const handleOpenInFileManager = useCallback(async () => {
    if (!root) return;
    setHeaderMenuOpen(false);

    try {
      await explorerOpenRootInFileManager(sessionId);
      onShowToast?.(`Opened project root in ${openInLabel}`, 'success');
    } catch (error) {
      onShowToast?.(`Failed to open project root: ${error}`, 'error');
    }
  }, [onShowToast, openInLabel, root, sessionId]);

  const handleChangeProjectRoot = useCallback(async () => {
    setHeaderMenuOpen(false);
    try {
      const dir = await pickWorkspaceDirectory(sessionId);
      if (!dir) return;
      onWorkspaceChanged?.(dir);
      onShowToast?.('Workspace updated', 'success');
    } catch (error) {
      onShowToast?.(`Failed to change project root: ${error}`, 'error');
    }
  }, [onShowToast, onWorkspaceChanged, sessionId]);

  return (
    <div className="agent-panel agent-panel--explorer" style={style}>
      <div className="explorer-panel" onContextMenu={(e) => handleCtxMenu(e)}>
        <div className="explorer-header">
          <div className="explorer-title-container">
            <span className="icon-folder" aria-hidden="true" />
            <span className="explorer-title" title={root || 'Project Files'}>Project Files</span>
          </div>
          <div className="explorer-header-actions">
            {isGitRepo && <span className="explorer-git-badge" title="Git repository">⎇</span>}
            <div className="explorer-header-menu-wrap" ref={headerMenuRef}>
              <button
                type="button"
                className="explorer-header-menu-button"
                aria-label="Project file actions"
                aria-haspopup="menu"
                aria-expanded={headerMenuOpen}
                onClick={(event) => {
                  event.stopPropagation();
                  setHeaderMenuOpen((open) => !open);
                }}
              >
                <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
                  <circle cx="8" cy="3" r="1.2" fill="currentColor" />
                  <circle cx="8" cy="8" r="1.2" fill="currentColor" />
                  <circle cx="8" cy="13" r="1.2" fill="currentColor" />
                </svg>
              </button>
              {headerMenuOpen && (
                <div className="explorer-header-menu" role="menu" aria-label="Project file actions menu">
                  <button type="button" role="menuitem" onClick={() => { void handleOpenInFileManager(); }}>
                    Open in {openInLabel}
                  </button>
                  <button type="button" role="menuitem" onClick={() => { void handleChangeProjectRoot(); }}>
                    Change project root…
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>

        <div className="explorer-tree" role="tree">
          {rootEntries.map((entry) => (
            <TreeNode
              key={entry.rel_path}
              entry={entry}
              sessionId={sessionId}
              depth={0}
              gitStatus={gitStatus}
              gitDirAgg={gitDirAgg}
              refreshKey={refreshKey}
              onOpenFile={onOpenFile}
              onContextMenu={handleCtxMenu}
              renamingPath={renaming?.rel_path ?? null}
              renameValue={renameValue}
              onRenameChange={setRenameValue}
              onRenameCommit={handleRename}
              onRenameCancel={() => { setRenaming(null); setRenameValue(''); }}
              onStartRename={handleStartRename}
            />
          ))}
        </div>

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

        {/* Delete confirmation dialog — replaces window.confirm() which is a no-op in Tauri WebView */}
        {deleteConfirm && (
          <div
            className="explorer-delete-overlay"
            role="dialog"
            aria-modal="true"
            aria-labelledby="explorer-delete-title"
            onClick={(e) => { if (e.target === e.currentTarget) setDeleteConfirm(null); }}
          >
            <div className="explorer-delete-dialog">
              <p id="explorer-delete-title" className="explorer-delete-title">
                Delete &ldquo;{deleteConfirm.name}&rdquo;?
              </p>
              <p className="explorer-delete-body">
                {isDir(deleteConfirm)
                  ? 'This will permanently delete the folder and all its contents.'
                  : 'This will permanently delete the file.'}
              </p>
              <div className="explorer-delete-actions">
                <button type="button" onClick={() => setDeleteConfirm(null)}>Cancel</button>
                <button
                  type="button"
                  className="ctx-delete"
                  onClick={() => { const e = deleteConfirm; setDeleteConfirm(null); void handleDelete(e); }}
                >
                  Delete
                </button>
              </div>
            </div>
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
              <button className="ctx-delete" onClick={() => { const e = ctxMenu.entry; setCtxMenu(null); if (e) setDeleteConfirm(e); }}>
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
  gitStatus: Record<string, ExplorerGitPathStatus>;
  /** Aggregated git status for directories, keyed by rel_path. */
  gitDirAgg: Map<string, ExplorerGitPathStatus>;
  /** Incremented by ExplorerPanel when the workspace changes; causes open dirs to re-fetch. */
  refreshKey: number;
  onOpenFile: (relPath: string) => void;
  onContextMenu: (e: React.MouseEvent, entry: ExplorerEntry) => void;
  /** Rel-path of the entry currently being renamed (null = none). */
  renamingPath: string | null;
  renameValue: string;
  onRenameChange: (v: string) => void;
  onRenameCommit: () => Promise<void>;
  onRenameCancel: () => void;
  onStartRename: (entry: ExplorerEntry) => void;
}

const LONG_PRESS_MS = 600;

const TreeNode: React.FC<TreeNodeProps> = ({
  entry, sessionId, depth, gitStatus, gitDirAgg, refreshKey, onOpenFile, onContextMenu,
  renamingPath, renameValue, onRenameChange, onRenameCommit, onRenameCancel, onStartRename,
}) => {
  const [open, setOpen] = useState(false);
  const [children, setChildren] = useState<ExplorerEntry[]>([]);
  const [loading, setLoading] = useState(false);

  // Long-press state
  const longPressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const isLongPress = useRef(false);

  const toggle = useCallback(async () => {
    // If a long-press just fired, skip the normal click toggle.
    if (isLongPress.current) { isLongPress.current = false; return; }
    if (!isDir(entry)) { onOpenFile(entry.rel_path); return; }
    if (!open && children.length === 0) {
      setLoading(true);
      try {
        const res = await explorerListDir(sessionId, entry.rel_path);
        setChildren(res.entries.sort(dirFirstSort));
      } finally { setLoading(false); }
    }
    setOpen((v) => !v);
  }, [entry, open, children, sessionId, onOpenFile]);

  const handleMouseDown = useCallback(() => {
    longPressTimer.current = setTimeout(() => {
      isLongPress.current = true;
      onStartRename(entry);
    }, LONG_PRESS_MS);
  }, [entry, onStartRename]);

  const cancelLongPress = useCallback(() => {
    if (longPressTimer.current !== null) {
      clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
  }, []);

  // Re-fetch children when a workspace change is signalled and this node is open.
  useEffect(() => {
    if (!open || !isDir(entry) || refreshKey === 0) return;
    explorerListDir(sessionId, entry.rel_path)
      .then((res) => setChildren(res.entries.sort(dirFirstSort)))
      .catch(() => { });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey]);

  const status: ExplorerGitPathStatus | null | undefined = isDir(entry)
    ? gitDirAgg.get(entry.rel_path)
    : (gitStatus[entry.rel_path] ?? entry.git_status ?? null);
  const badges = buildGitBadges(status);
  const nameCls = nameClassForStatus(status);

  const iconClass = isDir(entry)
    ? (open ? 'icon-folder-open' : 'icon-folder')
    : fileIconClass(entry.name);

  const isRenaming = renamingPath === entry.rel_path;

  return (
    <div className="explorer-node">
      <div
        className="explorer-row"
        style={{ '--explorer-depth': depth } as React.CSSProperties}
        onDoubleClick={() => { if (!isDir(entry)) onOpenFile(entry.rel_path); }}
        onClick={isRenaming ? undefined : toggle}
        onMouseDown={isRenaming ? undefined : handleMouseDown}
        onMouseUp={cancelLongPress}
        onMouseLeave={cancelLongPress}
        onContextMenu={(e) => {
          // Stop the panel-level context menu handler from overriding the
          // entry-specific right-click menu.
          e.preventDefault();
          e.stopPropagation();
          cancelLongPress();
          onContextMenu(e, entry);
        }}
        role="treeitem"
        aria-expanded={isDir(entry) ? open : undefined}
        tabIndex={0}
        title={entry.rel_path}
        onKeyDown={(e) => { if (e.key === 'Enter' && !isRenaming) void toggle(); }}
      >
        <div className="explorer-indent" />
        <div className="explorer-icon">
          {isDir(entry) && loading
            ? <span className="icon-loader-01" />
            : <span className={iconClass} />
          }
        </div>
        {isRenaming ? (
          <input
            className="explorer-rename-inline"
            value={renameValue}
            autoFocus
            onClick={(e) => e.stopPropagation()}
            onChange={(e) => onRenameChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') { e.stopPropagation(); void onRenameCommit(); }
              if (e.key === 'Escape') { e.stopPropagation(); onRenameCancel(); }
            }}
            onBlur={onRenameCancel}
          />
        ) : (
          <div className={`explorer-name ${nameCls}`}>{entry.name}</div>
        )}
        <div className="explorer-meta">
          <div className="explorer-git-badges">
            {badges.map((b) => (
              <span key={b.key} className={`git-badge ${b.cls} ${b.stageCls}`} title={b.title}>
                <span className={`git-badge-icon ${b.icon}`} />
              </span>
            ))}
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
          gitDirAgg={gitDirAgg}
          refreshKey={refreshKey}
          onOpenFile={onOpenFile}
          onContextMenu={onContextMenu}
          renamingPath={renamingPath}
          renameValue={renameValue}
          onRenameChange={onRenameChange}
          onRenameCommit={onRenameCommit}
          onRenameCancel={onRenameCancel}
          onStartRename={onStartRename}
        />
      ))}
    </div>
  );
};

