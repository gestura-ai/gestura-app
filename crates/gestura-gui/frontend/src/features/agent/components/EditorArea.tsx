/**
 * EditorArea — orchestrates the full multi-tab editor experience.
 *
 * Responsibilities:
 * - Renders TabBar above the active editor / diff pane
 * - Calls editorReadFile when a new tab is opened (from ExplorerPanel)
 * - Calls editorWriteFile when Cmd+S is pressed or the tab save event fires
 * - Calls editorGitDiff when a tab switches to diff view
 * - Marks tabs clean after a successful save
 * - Handles Cmd+Shift+D to toggle diff view on the active tab
 * - Exposes `onOpenFile` callback for ExplorerPanel to call
 * - Warns on window close when dirty tabs exist (EPIC 8)
 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTabState } from '../hooks/useTabState';
import { editorReadFile, editorWriteFile, editorGitDiff, editorRenameFile } from '../../../services/tauri/editor';
import { isMarkdownPath, languageFromPath } from '../utils/language';
import type { EditorLanguage } from '../utils/language';
import {
  WORKSPACE_CHANGED_EVENT,
  WORKSPACE_ENTRY_RENAMED_EVENT,
  dispatchWorkspaceEntryRenamed,
} from '../utils/workspaceEvents';
import { TabBar } from './TabBar';
import { EditorPane } from './EditorPane';
import { DiffPane } from './DiffPane';
import type { EditorOpenOptions, EditorTab } from '../types';
import './EditorArea.css';

export interface EditorAreaProps {
  sessionId: string;
  isDark: boolean;
  /** Called by EditorArea when the tab state changes (e.g. for window-level shortcuts) */
  onTabStateChange?: () => void;
}

export const EditorArea: React.FC<EditorAreaProps> = ({ sessionId, isDark }) => {
  const {
    tabs,
    activeTab,
    activeTabId,
    openTab,
    closeTab,
    activateTab,
    updateTabContent,
    refreshTabContent,
    markTabClean,
    toggleDiffView,
    reorderTabs,
    updateScrollOffset,
    renameTab,
    remapTabsForPath,
  } = useTabState();

  // ── Open a file by rel path ─────────────────────────────────────────────────
  const handleOpenFile = useCallback(async (relPath: string, options?: EditorOpenOptions) => {
    const viewMode = options?.viewMode ?? 'edit';
    if (viewMode === 'preview' && !isMarkdownPath(relPath)) return;
    // If already open, just activate
    const existing = tabs.find((t) => t.relPath === relPath && t.viewMode === viewMode);
    if (existing) {
      activateTab(existing.id);
      return;
    }
    try {
      const res = await editorReadFile(sessionId, relPath);
      const lang = languageFromPath(relPath);
      openTab({
        relPath,
        label: relPath.split('/').pop() ?? relPath,
        content: res.kind === 'image' ? (res.data_url ?? '') : res.content,
        language: lang,
        kind: res.kind,
        viewMode,
      });
    } catch (err) {
      console.warn('[EditorArea] failed to read file:', relPath, err);
    }
  }, [tabs, sessionId, openTab, activateTab]);

  const handleOpenRenderedView = useCallback((tabId: string) => {
    const sourceTab = tabs.find((tab) => tab.id === tabId);
    if (!sourceTab || sourceTab.viewMode !== 'edit' || sourceTab.kind !== 'text' || !isMarkdownPath(sourceTab.relPath)) {
      return;
    }

    const existingPreview = tabs.find((tab) => tab.relPath === sourceTab.relPath && tab.viewMode === 'preview');
    if (existingPreview) {
      if (existingPreview.content !== sourceTab.content) {
        refreshTabContent(existingPreview.id, sourceTab.content);
      }
      activateTab(existingPreview.id);
      return;
    }

    openTab({
      relPath: sourceTab.relPath,
      label: sourceTab.label,
      content: sourceTab.content,
      language: sourceTab.language,
      kind: sourceTab.kind,
      viewMode: 'preview',
    });
  }, [activateTab, openTab, refreshTabContent, tabs]);

  // ── Rename a tab (renames on disk + updates tab state) ─────────────────────
  const handleRenameTab = useCallback(async (tabId: string, newLabel: string) => {
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab) return;
    const dir = tab.relPath.split('/').slice(0, -1).join('/');
    const newRelPath = dir ? `${dir}/${newLabel}` : newLabel;
    try {
      await editorRenameFile(sessionId, tab.relPath, newRelPath);
      renameTab(tabId, newLabel, newRelPath);
      dispatchWorkspaceEntryRenamed({ oldRelPath: tab.relPath, newRelPath });
    } catch (err) {
      console.warn('[EditorArea] rename failed:', tab.relPath, '->', newRelPath, err);
    }
  }, [tabs, sessionId, renameTab]);

  // ── Save a tab ──────────────────────────────────────────────────────────────
  const handleSave = useCallback(async (tabId: string): Promise<boolean> => {
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab || tab.kind !== 'text' || tab.viewMode !== 'edit') return false;
    try {
      await editorWriteFile(sessionId, tab.relPath, tab.content);
      markTabClean(tabId);
      tabs
        .filter((openTabItem) => openTabItem.relPath === tab.relPath && openTabItem.viewMode === 'preview')
        .forEach((previewTab) => refreshTabContent(previewTab.id, tab.content));
      return true;
    } catch (err) {
      console.warn('[EditorArea] save failed:', tab.relPath, err);
      return false;
    }
  }, [tabs, sessionId, markTabClean, refreshTabContent]);

  // ── Toggle diff view for a tab ──────────────────────────────────────────────
  const handleToggleDiff = useCallback(async (tabId: string) => {
    const tab = tabs.find((candidate) => candidate.id === tabId);
    if (!tab || tab.kind !== 'text' || tab.viewMode !== 'edit') return;
    toggleDiffView(tabId);
  }, [tabs, toggleDiffView]);

  // ── Listen for the global save event (fired by AgentApp on Cmd+S) ───────────
  // AgentApp may or may not include a tabId; fall back to activeTabId.
  const activeTabIdRef = useRef(activeTabId);
  useEffect(() => { activeTabIdRef.current = activeTabId; }, [activeTabId]);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ tabId?: string } | null>).detail;
      const tabId = detail?.tabId ?? activeTabIdRef.current;
      if (tabId) void handleSave(tabId);
    };
    window.addEventListener('gestura:editor:save', handler);
    return () => window.removeEventListener('gestura:editor:save', handler);
  }, [handleSave]);

  // ── Keyboard shortcut: Cmd+Shift+D — toggle diff view ───────────────────────
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && (e.key === 'd' || e.key === 'D')) {
        if (activeTabId) {
          e.preventDefault();
          void handleToggleDiff(activeTabId);
        }
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [activeTabId, handleToggleDiff]);

  // ── beforeunload: warn when dirty tabs exist (EPIC 8) ───────────────────────
  useEffect(() => {
    const handler = (e: BeforeUnloadEvent) => {
      if (tabs.some((t) => t.isDirty)) {
        e.preventDefault();
        // Modern browsers ignore custom messages but require returnValue to trigger dialog.
        e.returnValue = '';
      }
    };
    window.addEventListener('beforeunload', handler);
    return () => window.removeEventListener('beforeunload', handler);
  }, [tabs]);

  // ── Expose openFile to window so ExplorerPanel can call it cross-component ──
  useEffect(() => {
    (window as unknown as Record<string, unknown>).__gesturaOpenFile = handleOpenFile;
    return () => {
      delete (window as unknown as Record<string, unknown>).__gesturaOpenFile;
    };
  }, [handleOpenFile]);

  // ── Reload non-dirty open tabs when the agent mutates the workspace ──────────
  // Preserves unsaved user edits (isDirty tabs are skipped).
  // Uses a stable ref so the effect handler always sees the current tabs without
  // needing to re-register the listener every time tabs changes.
  const tabsRef = useRef(tabs);
  useEffect(() => { tabsRef.current = tabs; }, [tabs]);

  useEffect(() => {
    const handler = async () => {
      for (const tab of tabsRef.current) {
        if (
          tab.viewMode === 'preview'
          && tabsRef.current.some((openTabItem) => (
            openTabItem.relPath === tab.relPath
            && openTabItem.viewMode === 'edit'
            && openTabItem.isDirty
          ))
        ) {
          continue;
        }
        if (tab.isDirty || tab.kind !== 'text') continue;
        try {
          const res = await editorReadFile(sessionId, tab.relPath);
          if (res.kind === 'text' && res.content !== tab.content) {
            // Use refreshTabContent (not updateTabContent) so the tab is NOT
            // marked dirty — external agent edits should not trigger the
            // "unsaved changes" warning or indicator.
            refreshTabContent(tab.id, res.content);
          }
        } catch {
          // File may have been deleted by the agent — leave the tab open with stale content.
        }
      }
    };
    window.addEventListener(WORKSPACE_CHANGED_EVENT, handler);
    return () => window.removeEventListener(WORKSPACE_CHANGED_EVENT, handler);
  }, [sessionId, refreshTabContent]);

  useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ oldRelPath: string; newRelPath: string }>).detail;
      if (!detail || detail.oldRelPath === detail.newRelPath) return;
      remapTabsForPath(detail.oldRelPath, detail.newRelPath);
    };
    window.addEventListener(WORKSPACE_ENTRY_RENAMED_EVENT, handler);
    return () => window.removeEventListener(WORKSPACE_ENTRY_RENAMED_EVENT, handler);
  }, [remapTabsForPath]);



  return (
    <div className="agent-panel agent-panel--editor">
      <div className="editor-area">
        <TabBar
          tabs={tabs}
          activeTabId={activeTabId}
          onActivate={activateTab}
          onClose={closeTab}
          onReorder={reorderTabs}
          onSaveTab={handleSave}
          onRenameTab={handleRenameTab}
          onOpenRenderedView={handleOpenRenderedView}
        />
        <div className="editor-area__pane">
          {activeTab ? (
            activeTab.isDiffView ? (
              <DiffPaneWrapper
                sessionId={sessionId}
                tab={activeTab}
                isDark={isDark}
              />
            ) : (
              <EditorPane
                tab={activeTab}
                isDark={isDark}
                onContentChange={updateTabContent}
                onSave={handleSave}
                onScrollChange={updateScrollOffset}
              />
            )
          ) : (
            <div className="editor-area__empty">
              <span>Open a file from the Explorer</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default EditorArea;

// ─── DiffPaneWrapper ──────────────────────────────────────────────────────────
// Fetches the git diff for a tab then renders DiffPane once the data is ready.

interface DiffPaneWrapperProps {
  sessionId: string;
  tab: EditorTab;
  isDark: boolean;
}

const DiffPaneWrapper: React.FC<DiffPaneWrapperProps> = ({ sessionId, tab, isDark }) => {
  const [original, setOriginal] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    // eslint-disable-next-line
    setLoading(true);
    editorGitDiff(sessionId, tab.relPath)
      .then((res) => setOriginal(res.original))
      .catch(() => setOriginal(''))
      .finally(() => setLoading(false));
  }, [sessionId, tab.relPath]);

  if (loading) {
    return (
      <div className="editor-area__diff-loading">
        <span>Loading diff…</span>
      </div>
    );
  }

  return (
    <DiffPane
      original={original ?? ''}
      modified={tab.content}
      language={tab.language as EditorLanguage}
      isDark={isDark}
    />
  );
};

