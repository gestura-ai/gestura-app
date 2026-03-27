/**
 * ExplorerPanel — left-side file-tree panel for the three-panel editor layout.
 *
 * Features:
 * - Lazy-loads directory contents via Tauri explorer_list_dir
 * - Git status decorations on file entries
 * - Tree selection with multi-select delete support
 * - Keyboard delete/backspace shortcut for selected entries
 * - Drag-and-drop move support for files and directories
 * - Right-click context menu: New File, New Folder, Rename, Delete
 */
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  explorerGetRoot,
  explorerListDir,
  explorerGitStatus,
  explorerOpenEntryInFileManager,
  explorerOpenRootInFileManager,
} from '../../../services/tauri/explorer';
import { pickWorkspaceDirectory } from '../../../services/tauri/agent';
import {
  editorCreateFile,
  editorDeleteFile,
  editorRenameFile,
} from '../../../services/tauri/editor';
import type { EditorOpenOptions, ExplorerEntry, ExplorerGitChangeKind, ExplorerGitPathStatus } from '../types';
import { isMarkdownPath } from '../utils/language';
import { dispatchWorkspaceEntryRenamed } from '../utils/workspaceEvents';
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

interface ExplorerDragPreview {
  name: string;
  x: number;
  y: number;
}

interface ExplorerDragIntent {
  entry: ExplorerEntry;
  startX: number;
  startY: number;
}

interface ExplorerDropTarget {
  relPath: string;
  token: string;
}

const hasMultiSelectModifier = (event: Pick<MouseEvent, 'metaKey' | 'ctrlKey'>): boolean => event.metaKey || event.ctrlKey;
const DRAG_START_DISTANCE_PX = 6;
const ROOT_DROP_TARGET = '__explorer_root__';
const EXPLORER_GLOBAL_DRAG_CLASS = 'explorer-dragging-global';

const joinRelPath = (dirRel: string, name: string): string => (dirRel ? `${dirRel}/${name}` : name);

const parentDirOf = (relPath: string): string => relPath.split('/').slice(0, -1).join('/');

const isSameOrDescendantPath = (parentRelPath: string, candidateRelPath: string): boolean => (
  candidateRelPath === parentRelPath || candidateRelPath.startsWith(`${parentRelPath}/`)
);

function dedupeEntries(entries: ExplorerEntry[]): ExplorerEntry[] {
  const unique = new Map<string, ExplorerEntry>();
  for (const entry of entries) unique.set(entry.rel_path, entry);
  return [...unique.values()];
}

function topLevelDeleteEntries(entries: ExplorerEntry[]): ExplorerEntry[] {
  const sorted = dedupeEntries(entries).sort((a, b) => a.rel_path.length - b.rel_path.length);
  return sorted.filter((entry) => !sorted.some((candidate) => (
    candidate.rel_path !== entry.rel_path
    && isDir(candidate)
    && isSameOrDescendantPath(candidate.rel_path, entry.rel_path)
  )));
}

function canMoveEntryToDirectoryPath(source: ExplorerEntry, targetDirRelPath: string): boolean {
  if (source.rel_path === targetDirRelPath) return false;
  if (isDir(source) && isSameOrDescendantPath(source.rel_path, targetDirRelPath)) return false;
  return joinRelPath(targetDirRelPath, source.name) !== source.rel_path;
}

function canDropEntryIntoDirectory(source: ExplorerEntry, target: ExplorerEntry): boolean {
  if (!isDir(target)) return false;
  return canMoveEntryToDirectoryPath(source, target.rel_path);
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName);
}

function makeDirectoryDropTarget(relPath: string, token?: string): ExplorerDropTarget {
  return {
    relPath,
    token: token ?? (relPath || ROOT_DROP_TARGET),
  };
}

export interface ExplorerPanelProps {
  sessionId: string;
  workspaceRoot?: string | null;
  onOpenFile: (relPath: string, options?: EditorOpenOptions) => void;
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
  onOpenFile,
  onWorkspaceChanged,
  onShowToast,
  style,
}) => {
  const panelRef = useRef<HTMLDivElement>(null);
  const [root, setRoot] = useState<string>('');
  const [isGitRepo, setIsGitRepo] = useState(false);
  const [rootEntries, setRootEntries] = useState<ExplorerEntry[]>([]);
  const [gitStatus, setGitStatus] = useState<Record<string, ExplorerGitPathStatus>>({});
  const [selectedEntries, setSelectedEntries] = useState<ExplorerEntry[]>([]);
  const [ctxMenu, setCtxMenu] = useState<ContextMenu | null>(null);
  const [headerMenuOpen, setHeaderMenuOpen] = useState(false);
  const [renaming, setRenaming] = useState<ExplorerEntry | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [newName, setNewName] = useState('');
  const [creating, setCreating] = useState<{ parentDir: string; isDir: boolean } | null>(null);
  /** Entries pending delete confirmation — null means no dialog is open. */
  const [deleteConfirm, setDeleteConfirm] = useState<ExplorerEntry[] | null>(null);
  const [draggingEntry, setDraggingEntry] = useState<ExplorerEntry | null>(null);
  const [dropTargetPath, setDropTargetPath] = useState<string | null>(null);
  const [dragPreview, setDragPreview] = useState<ExplorerDragPreview | null>(null);
  const draggingEntryRef = useRef<ExplorerEntry | null>(null);
  const dragIntentRef = useRef<ExplorerDragIntent | null>(null);
  const dropTargetRef = useRef<ExplorerDropTarget | null>(null);
  const suppressClickRef = useRef(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const headerMenuRef = useRef<HTMLDivElement>(null);

  // Incremented whenever the agent mutates the workspace (file/shell/git tool completes).
  const [refreshKey, setRefreshKey] = useState(0);

  // Aggregated git status for directories — derived from flat file-path gitStatus map.
  const gitDirAgg = useMemo(() => computeGitDirAgg(gitStatus), [gitStatus]);
  const selectedPaths = useMemo(() => new Set(selectedEntries.map((entry) => entry.rel_path)), [selectedEntries]);
  const deleteTargets = useMemo(() => topLevelDeleteEntries(deleteConfirm ?? []), [deleteConfirm]);
  const openInLabel = useMemo(() => fileManagerLabel(), []);

  const triggerRefresh = useCallback(() => {
    setRefreshKey((k) => k + 1);
  }, []);

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

  useEffect(() => {
    if (selectedEntries.length === 0) return;

    const clearSelectionIfOutside = (target: EventTarget | null) => {
      if (draggingEntryRef.current || !panelRef.current) return;
      if (target instanceof Node && panelRef.current.contains(target)) return;
      setSelectedEntries([]);
    };

    const handlePointerDown = (e: MouseEvent) => {
      clearSelectionIfOutside(e.target);
    };

    const handleFocusIn = (e: FocusEvent) => {
      clearSelectionIfOutside(e.target);
    };

    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('focusin', handleFocusIn);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('focusin', handleFocusIn);
    };
  }, [selectedEntries.length]);

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

  useEffect(() => {
    if (selectedEntries.length === 0) return;
    const handler = (e: KeyboardEvent) => {
      if (deleteConfirm || headerMenuOpen || renaming || creating) return;
      if (isEditableTarget(e.target)) return;
      if (e.key !== 'Delete' && e.key !== 'Backspace') return;
      const active = document.activeElement;
      if (!panelRef.current || !active || !panelRef.current.contains(active)) return;
      e.preventDefault();
      setCtxMenu(null);
      setDeleteConfirm(selectedEntries);
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [creating, deleteConfirm, headerMenuOpen, renaming, selectedEntries]);

  const handleCtxMenu = useCallback((e: React.MouseEvent, entry?: ExplorerEntry) => {
    e.preventDefault();
    const parentDir = entry && isDir(entry) ? entry.rel_path : (entry ? parentDirOf(entry.rel_path) : '');
    if (!entry) setSelectedEntries([]);
    setCtxMenu({ x: e.clientX, y: e.clientY, entry, parentDir });
  }, []);

  const handleSelectEntry = useCallback((entry: ExplorerEntry, event: Pick<MouseEvent, 'metaKey' | 'ctrlKey'>) => {
    setCtxMenu(null);
    const additive = hasMultiSelectModifier(event);
    setSelectedEntries((current) => {
      const alreadySelected = current.some((item) => item.rel_path === entry.rel_path);
      if (additive) {
        return alreadySelected
          ? current.filter((item) => item.rel_path !== entry.rel_path)
          : [...current, entry];
      }
      return alreadySelected && current.length === 1 ? [entry] : [entry];
    });
  }, []);

  const handleEntryContextMenu = useCallback((e: React.MouseEvent, entry: ExplorerEntry) => {
    setSelectedEntries((current) => (
      current.some((item) => item.rel_path === entry.rel_path) ? current : [entry]
    ));
    handleCtxMenu(e, entry);
  }, [handleCtxMenu]);

  const selectedEntriesForAction = useCallback((entry?: ExplorerEntry): ExplorerEntry[] => {
    if (!entry) return selectedEntries;
    return selectedPaths.has(entry.rel_path) ? selectedEntries : [entry];
  }, [selectedEntries, selectedPaths]);

  const handleOpenRenderedView = useCallback((relPath: string) => {
    setCtxMenu(null);
    onOpenFile(relPath, { viewMode: 'preview' });
  }, [onOpenFile]);

  const handleCreate = useCallback(async (isDir: boolean) => {
    if (!newName.trim() || !creating) return;
    const rel = joinRelPath(creating.parentDir, newName.trim());
    try {
      await editorCreateFile(sessionId, rel, isDir);
      setCreating(null);
      setNewName('');
      triggerRefresh();
    } catch (error) {
      onShowToast?.(`Failed to create ${isDir ? 'folder' : 'file'}: ${error}`, 'error');
    }
  }, [creating, newName, onShowToast, sessionId, triggerRefresh]);

  const handleRename = useCallback(async () => {
    if (!renaming || !renameValue.trim()) return;
    const dir = parentDirOf(renaming.rel_path);
    const newPath = joinRelPath(dir, renameValue.trim());
    try {
      await editorRenameFile(sessionId, renaming.rel_path, newPath);
      dispatchWorkspaceEntryRenamed({ oldRelPath: renaming.rel_path, newRelPath: newPath });
      setRenaming(null);
      setRenameValue('');
      setSelectedEntries([]);
    } catch (error) {
      onShowToast?.(`Failed to rename ${renaming.name}: ${error}`, 'error');
    }
  }, [onShowToast, renaming, renameValue, sessionId]);

  const handleStartRename = useCallback((entry: ExplorerEntry) => {
    setSelectedEntries([entry]);
    setRenaming(entry);
    setRenameValue(entry.name);
  }, []);

  /** Executes the actual delete — only called after the confirm dialog is accepted. */
  const handleDelete = useCallback(async (entries: ExplorerEntry[]) => {
    const targets = topLevelDeleteEntries(entries);
    if (targets.length === 0) return;
    try {
      for (const entry of targets) {
        await editorDeleteFile(sessionId, entry.rel_path);
      }
      setSelectedEntries([]);
      triggerRefresh();
    } catch (error) {
      onShowToast?.(`Failed to delete selected item${targets.length > 1 ? 's' : ''}: ${error}`, 'error');
    }
  }, [onShowToast, sessionId, triggerRefresh]);

  const clearDraggingState = useCallback(() => {
    dragIntentRef.current = null;
    draggingEntryRef.current = null;
    dropTargetRef.current = null;
    if (typeof document !== 'undefined') {
      document.body.classList.remove(EXPLORER_GLOBAL_DRAG_CLASS);
    }
    setDraggingEntry(null);
    setDropTargetPath(null);
    setDragPreview(null);
  }, []);

  const moveEntryIntoDirectory = useCallback(async (source: ExplorerEntry, target: ExplorerDropTarget) => {
    if (!canMoveEntryToDirectoryPath(source, target.relPath)) return;
    const newRelPath = joinRelPath(target.relPath, source.name);
    try {
      await editorRenameFile(sessionId, source.rel_path, newRelPath);
      dispatchWorkspaceEntryRenamed({ oldRelPath: source.rel_path, newRelPath });
      setSelectedEntries([]);
    } catch (error) {
      onShowToast?.(`Failed to move ${source.name}: ${error}`, 'error');
    }
  }, [onShowToast, sessionId]);

  const handleBeginDragIntent = useCallback((entry: ExplorerEntry, event: React.MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    dragIntentRef.current = {
      entry,
      startX: event.clientX,
      startY: event.clientY,
    };
    if (typeof document !== 'undefined') {
      document.body.classList.add(EXPLORER_GLOBAL_DRAG_CLASS);
    }
    suppressClickRef.current = false;
  }, []);

  const handleHoverEntry = useCallback((entry: ExplorerEntry): boolean => {
    const source = draggingEntryRef.current;
    if (!source) return false;

    const isDirectoryEntry = isDir(entry);
    const targetRelPath = isDirectoryEntry ? entry.rel_path : parentDirOf(entry.rel_path);
    if (!canMoveEntryToDirectoryPath(source, targetRelPath)) return false;

    const target = makeDirectoryDropTarget(
      targetRelPath,
      isDirectoryEntry ? entry.rel_path : `row-parent:${entry.rel_path}`,
    );
    dropTargetRef.current = target;
    setDropTargetPath(target.token);
    return true;
  }, []);

  const handleHoverRoot = useCallback((): boolean => {
    const source = draggingEntryRef.current;
    if (!source || !canMoveEntryToDirectoryPath(source, '')) return false;
    const target = makeDirectoryDropTarget('');
    dropTargetRef.current = target;
    setDropTargetPath(target.token);
    return true;
  }, []);

  const handleHoverOpenDirectory = useCallback((entry: ExplorerEntry): boolean => {
    const source = draggingEntryRef.current;
    if (!source || !isDir(entry) || !canDropEntryIntoDirectory(source, entry)) return false;
    const target = makeDirectoryDropTarget(entry.rel_path, `container:${entry.rel_path}`);
    dropTargetRef.current = target;
    setDropTargetPath(target.token);
    return true;
  }, []);

  const handleLeaveTarget = useCallback((token: string) => {
    if (dropTargetRef.current?.token !== token) return;
    dropTargetRef.current = null;
    setDropTargetPath(null);
  }, []);

  const consumeSuppressedClick = useCallback((): boolean => {
    if (!suppressClickRef.current) return false;
    suppressClickRef.current = false;
    return true;
  }, []);

  useEffect(() => {
    const handleMouseMove = (event: MouseEvent) => {
      const intent = dragIntentRef.current;
      if (!intent) return;

      const source = draggingEntryRef.current;
      if (!source) {
        const deltaX = event.clientX - intent.startX;
        const deltaY = event.clientY - intent.startY;
        if (Math.hypot(deltaX, deltaY) < DRAG_START_DISTANCE_PX) return;
        draggingEntryRef.current = intent.entry;
        setDraggingEntry(intent.entry);
        setSelectedEntries([intent.entry]);
        setCtxMenu(null);
        suppressClickRef.current = true;
      }

      const activeSource = draggingEntryRef.current;
      if (!activeSource) return;
      if (typeof window.getSelection === 'function') {
        window.getSelection()?.removeAllRanges();
      }
      setDragPreview({
        name: activeSource.name,
        x: event.clientX,
        y: event.clientY,
      });
    };

    const handleMouseUp = () => {
      const source = draggingEntryRef.current;
      const target = dropTargetRef.current;
      const shouldSuppressClick = suppressClickRef.current;

      clearDraggingState();

      if (source && target && canMoveEntryToDirectoryPath(source, target.relPath)) {
        void moveEntryIntoDirectory(source, target);
      }

      if (shouldSuppressClick) {
        window.setTimeout(() => {
          suppressClickRef.current = false;
        }, 0);
      }
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
    window.addEventListener('blur', handleMouseUp);
    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      window.removeEventListener('blur', handleMouseUp);
    };
  }, [clearDraggingState, moveEntryIntoDirectory]);

  const handleOpenInFileManager = useCallback(async () => {
    if (!root) return;
    setHeaderMenuOpen(false);

    try {
      await explorerOpenRootInFileManager(sessionId);
    } catch (error) {
      onShowToast?.(`Failed to open project root: ${error}`, 'error');
    }
  }, [onShowToast, root, sessionId]);

  const handleShowEntryInFileManager = useCallback(async (entry: ExplorerEntry) => {
    setCtxMenu(null);

    try {
      await explorerOpenEntryInFileManager(sessionId, entry.rel_path);
    } catch (error) {
      onShowToast?.(`Failed to show ${entry.name} in ${openInLabel}: ${error}`, 'error');
    }
  }, [onShowToast, openInLabel, sessionId]);

  const handleChangeProjectRoot = useCallback(async () => {
    setHeaderMenuOpen(false);
    try {
      const dir = await pickWorkspaceDirectory(sessionId);
      if (!dir) return;
      onWorkspaceChanged?.(dir);
    } catch (error) {
      onShowToast?.(`Failed to change project root: ${error}`, 'error');
    }
  }, [onShowToast, onWorkspaceChanged, sessionId]);

  return (
    <div className="agent-panel agent-panel--explorer" style={style}>
      <div
        ref={panelRef}
        className="explorer-panel"
        onContextMenu={(e) => handleCtxMenu(e)}
      >
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

        <div
          className={[
            'explorer-tree',
            dropTargetPath === ROOT_DROP_TARGET ? 'explorer-tree--root-drop-target' : '',
          ].filter(Boolean).join(' ')}
          role="tree"
          onMouseEnter={() => {
            void handleHoverRoot();
          }}
          onMouseMove={(e) => {
            if (e.target === e.currentTarget) {
              void handleHoverRoot();
            }
          }}
          onMouseLeave={() => {
            handleLeaveTarget(ROOT_DROP_TARGET);
          }}
          onClick={(e) => {
            if (e.target === e.currentTarget) {
              setSelectedEntries([]);
              setCtxMenu(null);
            }
          }}
        >
          {draggingEntry && canMoveEntryToDirectoryPath(draggingEntry, '') && (
            <div
              className={[
                'explorer-root-dropzone',
                dropTargetPath === ROOT_DROP_TARGET ? 'explorer-root-dropzone--active' : '',
              ].filter(Boolean).join(' ')}
              onMouseEnter={() => {
                void handleHoverRoot();
              }}
              onMouseMove={() => {
                void handleHoverRoot();
              }}
              onMouseLeave={() => {
                handleLeaveTarget(ROOT_DROP_TARGET);
              }}
            >
            </div>
          )}
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
              selectedPaths={selectedPaths}
              draggingEntry={draggingEntry}
              draggingPath={draggingEntry?.rel_path ?? null}
              dropTargetPath={dropTargetPath}
              onSelect={handleSelectEntry}
              onContextMenu={handleEntryContextMenu}
              onBeginDragIntent={handleBeginDragIntent}
              onHoverEntry={handleHoverEntry}
              onHoverOpenDirectory={handleHoverOpenDirectory}
              onLeaveTarget={handleLeaveTarget}
              consumeSuppressedClick={consumeSuppressedClick}
              renamingPath={renaming?.rel_path ?? null}
              renameValue={renameValue}
              onRenameChange={setRenameValue}
              onRenameCommit={handleRename}
              onRenameCancel={() => { setRenaming(null); setRenameValue(''); }}
              onStartRename={handleStartRename}
            />
          ))}
          {draggingEntry && canMoveEntryToDirectoryPath(draggingEntry, '') && (
            <div
              className={[
                'explorer-root-dropzone',
                'explorer-root-dropzone--spacer',
                dropTargetPath === ROOT_DROP_TARGET ? 'explorer-root-dropzone--active' : '',
              ].filter(Boolean).join(' ')}
              onMouseEnter={() => {
                void handleHoverRoot();
              }}
              onMouseMove={() => {
                void handleHoverRoot();
              }}
              onMouseLeave={() => {
                handleLeaveTarget(ROOT_DROP_TARGET);
              }}
            >
            </div>
          )}
        </div>

        {dragPreview && draggingEntry && (
          <div
            className="explorer-drag-preview"
            style={{ left: dragPreview.x + 14, top: dragPreview.y + 14 }}
            aria-hidden="true"
          >
            <span className={isDir(draggingEntry) ? 'icon-folder' : fileIconClass(draggingEntry.name)} />
            <span>{dragPreview.name}</span>
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
                {deleteTargets.length === 1
                  ? `Delete “${deleteTargets[0].name}”?`
                  : `Delete ${deleteTargets.length} items?`}
              </p>
              <p className="explorer-delete-body">
                {deleteTargets.length === 1
                  ? (isDir(deleteTargets[0])
                    ? 'This will permanently delete the folder and all its contents.'
                    : 'This will permanently delete the file.')
                  : (deleteTargets.some(isDir)
                    ? 'This will permanently delete the selected files and folders, including all folder contents.'
                    : 'This will permanently delete the selected files.')}
              </p>
              <div className="explorer-delete-actions">
                <button type="button" onClick={() => setDeleteConfirm(null)}>Cancel</button>
                <button
                  type="button"
                  className="ctx-delete"
                  onClick={() => { const entries = deleteTargets; setDeleteConfirm(null); void handleDelete(entries); }}
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
              New File
            </button>
            <button onClick={() => { setCtxMenu(null); setCreating({ parentDir: ctxMenu.parentDir, isDir: true }); setNewName(''); }}>
              New Folder
            </button>
            {ctxMenu.entry && <>
              <div className="context-menu-sep" />
              <button onClick={() => { void handleShowEntryInFileManager(ctxMenu.entry!); }}>
                Show in {openInLabel}
              </button>
              {ctxMenu.entry.kind === 'file' && isMarkdownPath(ctxMenu.entry.rel_path) && (
                <button onClick={() => { void handleOpenRenderedView(ctxMenu.entry!.rel_path); }}>
                  Rendered View
                </button>
              )}
              <button onClick={() => {
                if (!ctxMenu.entry) return;
                setCtxMenu(null); setSelectedEntries([ctxMenu.entry]); setRenaming(ctxMenu.entry); setRenameValue(ctxMenu.entry.name);
              }}>
                Rename
              </button>
              <button
                className="ctx-delete"
                onClick={() => {
                  const entry = ctxMenu.entry;
                  setCtxMenu(null);
                  if (entry) setDeleteConfirm(selectedEntriesForAction(entry));
                }}
              >
                Delete
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
  onOpenFile: (relPath: string, options?: EditorOpenOptions) => void;
  selectedPaths: Set<string>;
  draggingEntry: ExplorerEntry | null;
  draggingPath: string | null;
  dropTargetPath: string | null;
  onSelect: (entry: ExplorerEntry, event: Pick<MouseEvent, 'metaKey' | 'ctrlKey'>) => void;
  onContextMenu: (e: React.MouseEvent, entry: ExplorerEntry) => void;
  onBeginDragIntent: (entry: ExplorerEntry, event: React.MouseEvent<HTMLDivElement>) => void;
  onHoverEntry: (entry: ExplorerEntry) => boolean;
  onHoverOpenDirectory: (entry: ExplorerEntry) => boolean;
  onLeaveTarget: (token: string) => void;
  consumeSuppressedClick: () => boolean;
  /** Rel-path of the entry currently being renamed (null = none). */
  renamingPath: string | null;
  renameValue: string;
  onRenameChange: (v: string) => void;
  onRenameCommit: () => Promise<void>;
  onRenameCancel: () => void;
  onStartRename: (entry: ExplorerEntry) => void;
}

const LONG_PRESS_MS = 600;
const DRAG_HOVER_OPEN_MS = 650;

const TreeNode: React.FC<TreeNodeProps> = ({
  entry, sessionId, depth, gitStatus, gitDirAgg, refreshKey, onOpenFile,
  selectedPaths, draggingEntry, draggingPath, dropTargetPath, onSelect, onContextMenu,
  onBeginDragIntent, onHoverEntry, onHoverOpenDirectory, onLeaveTarget, consumeSuppressedClick,
  renamingPath, renameValue, onRenameChange, onRenameCommit, onRenameCancel, onStartRename,
}) => {
  const [open, setOpen] = useState(false);
  const [children, setChildren] = useState<ExplorerEntry[]>([]);
  const [loading, setLoading] = useState(false);

  // Long-press state
  const longPressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const isLongPress = useRef(false);
  const dragHoverOpenTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearDragHoverOpenTimer = useCallback(() => {
    if (dragHoverOpenTimer.current !== null) {
      clearTimeout(dragHoverOpenTimer.current);
      dragHoverOpenTimer.current = null;
    }
  }, []);

  const ensureOpen = useCallback(async () => {
    if (!isDir(entry) || open) return;
    if (children.length === 0) {
      setLoading(true);
      try {
        const res = await explorerListDir(sessionId, entry.rel_path);
        setChildren(res.entries.sort(dirFirstSort));
      } finally { setLoading(false); }
    }
    setOpen(true);
  }, [children, entry, open, sessionId]);

  const activateEntry = useCallback(async () => {
    // If a long-press just fired, skip the normal activation.
    if (isLongPress.current) { isLongPress.current = false; return; }
    if (!isDir(entry)) { onOpenFile(entry.rel_path); return; }
    if (!open) {
      await ensureOpen();
      return;
    }
    setOpen(false);
  }, [ensureOpen, entry, onOpenFile, open]);

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

  useEffect(() => () => {
    clearDragHoverOpenTimer();
  }, [clearDragHoverOpenTimer]);

  const status: ExplorerGitPathStatus | null | undefined = isDir(entry)
    ? gitDirAgg.get(entry.rel_path)
    : (gitStatus[entry.rel_path] ?? entry.git_status ?? null);
  const badges = buildGitBadges(status);
  const nameCls = nameClassForStatus(status);

  const iconClass = isDir(entry)
    ? (open ? 'icon-folder-open' : 'icon-folder')
    : fileIconClass(entry.name);

  const isRenaming = renamingPath === entry.rel_path;
  const isSelected = selectedPaths.has(entry.rel_path);
  const isDragging = draggingPath === entry.rel_path;
  const rowDropToken = isDir(entry) ? entry.rel_path : `row-parent:${entry.rel_path}`;
  const containerDropToken = `container:${entry.rel_path}`;
  const isRowDropTarget = dropTargetPath === rowDropToken;
  const isContainerDropTarget = dropTargetPath === containerDropToken;

  return (
    <div className="explorer-node">
      <div
        className={[
          'explorer-row',
          isSelected ? 'explorer-row--selected' : '',
          isDragging ? 'explorer-row--dragging' : '',
          isRowDropTarget ? 'explorer-row--drop-target' : '',
        ].filter(Boolean).join(' ')}
        style={{ '--explorer-depth': depth } as React.CSSProperties}
        onClick={isRenaming ? undefined : (e) => {
          if (consumeSuppressedClick()) return;
          e.currentTarget.focus();
          onSelect(entry, e.nativeEvent);
          if (!hasMultiSelectModifier(e.nativeEvent)) {
            void activateEntry();
          }
        }}
        onMouseDown={isRenaming ? undefined : (e) => {
          handleMouseDown();
          onBeginDragIntent(entry, e);
        }}
        onMouseUp={cancelLongPress}
        onMouseLeave={() => {
          cancelLongPress();
          clearDragHoverOpenTimer();
          onLeaveTarget(rowDropToken);
        }}
        onMouseEnter={() => {
          const canDrop = onHoverEntry(entry);
          if (
            canDrop
            && draggingEntry
            && dragHoverOpenTimer.current === null
            && isDir(entry)
            && !open
          ) {
            dragHoverOpenTimer.current = setTimeout(() => {
              dragHoverOpenTimer.current = null;
              void ensureOpen();
            }, DRAG_HOVER_OPEN_MS);
          }
        }}
        onMouseMove={() => {
          const canDrop = onHoverEntry(entry);
          if (!canDrop) {
            clearDragHoverOpenTimer();
          }
        }}
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
        aria-selected={isSelected}
        tabIndex={0}
        title={entry.rel_path}
        onKeyDown={(e) => { if (e.key === 'Enter' && !isRenaming) void activateEntry(); }}
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
      {open && (
        <div
          className={[
            'explorer-children',
            isContainerDropTarget ? 'explorer-children--drop-target' : '',
          ].filter(Boolean).join(' ')}
          style={{ '--explorer-depth': depth } as React.CSSProperties}
          onMouseEnter={(e) => {
            if (e.target === e.currentTarget) {
              onHoverOpenDirectory(entry);
            }
          }}
          onMouseMove={(e) => {
            if (e.target === e.currentTarget) {
              onHoverOpenDirectory(entry);
            }
          }}
          onMouseLeave={(e) => {
            if (e.target === e.currentTarget) {
              onLeaveTarget(containerDropToken);
            }
          }}
        >
          {draggingEntry && canDropEntryIntoDirectory(draggingEntry, entry) && (
            <div
              className={[
                'explorer-child-dropzone',
                isContainerDropTarget ? 'explorer-child-dropzone--active' : '',
              ].filter(Boolean).join(' ')}
              style={{ '--explorer-depth': depth } as React.CSSProperties}
              onMouseEnter={() => {
                onHoverOpenDirectory(entry);
              }}
              onMouseMove={() => {
                onHoverOpenDirectory(entry);
              }}
              onMouseLeave={() => {
                onLeaveTarget(containerDropToken);
              }}
            >
            </div>
          )}
          {children.map((child) => (
            <TreeNode
              key={child.rel_path}
              entry={child}
              sessionId={sessionId}
              depth={depth + 1}
              gitStatus={gitStatus}
              gitDirAgg={gitDirAgg}
              refreshKey={refreshKey}
              onOpenFile={onOpenFile}
              selectedPaths={selectedPaths}
              draggingEntry={draggingEntry}
              draggingPath={draggingPath}
              dropTargetPath={dropTargetPath}
              onSelect={onSelect}
              onContextMenu={onContextMenu}
              onBeginDragIntent={onBeginDragIntent}
              onHoverEntry={onHoverEntry}
              onHoverOpenDirectory={onHoverOpenDirectory}
              onLeaveTarget={onLeaveTarget}
              consumeSuppressedClick={consumeSuppressedClick}
              renamingPath={renamingPath}
              renameValue={renameValue}
              onRenameChange={onRenameChange}
              onRenameCommit={onRenameCommit}
              onRenameCancel={onRenameCancel}
              onStartRename={onStartRename}
            />
          ))}
          {draggingEntry && canDropEntryIntoDirectory(draggingEntry, entry) && (
            <div
              className={[
                'explorer-child-dropzone',
                'explorer-child-dropzone--tail',
                isContainerDropTarget ? 'explorer-child-dropzone--active' : '',
              ].filter(Boolean).join(' ')}
              style={{ '--explorer-depth': depth } as React.CSSProperties}
              onMouseEnter={() => {
                onHoverOpenDirectory(entry);
              }}
              onMouseMove={() => {
                onHoverOpenDirectory(entry);
              }}
              onMouseLeave={() => {
                onLeaveTarget(containerDropToken);
              }}
            >
            </div>
          )}
        </div>
      )}
    </div>
  );
};

