/**
 * TabBar — horizontal tab strip for the multi-tab editor.
 *
 * Features:
 * - Renders all open tabs (EditorTab[])
 * - Highlights the active tab
 * - Shows dirty indicator (•) for unsaved tabs
 * - Close button with dirty-state guard (propagates up to EditorArea)
 * - Drag-to-reorder via HTML5 drag-and-drop API
 * - Right-click context menu with Rename option
 * - Inline rename input (Enter to commit, Escape/blur to cancel)
 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import type { EditorTab } from '../types';
import { isMarkdownPath } from '../utils/language';
import './TabBar.css';

export interface TabBarProps {
  tabs: EditorTab[];
  activeTabId: string | null;
  onActivate: (tabId: string) => void;
  /** Returns false when the tab has unsaved changes and close was prevented. */
  onClose: (tabId: string, opts?: { force?: boolean }) => boolean;
  onReorder: (fromIndex: number, toIndex: number) => void;
  /** Called when user confirms a rename; should persist the new name on disk. */
  onRenameTab?: (tabId: string, newLabel: string) => Promise<void>;
  onOpenRenderedView?: (tabId: string) => void;
}

interface CtxMenu {
  tabId: string;
  x: number;
  y: number;
}

export const TabBar: React.FC<TabBarProps> = ({
  tabs, activeTabId, onActivate, onClose, onReorder, onRenameTab, onOpenRenderedView,
}) => {
  const [dragSrc, setDragSrc] = useState<number | null>(null);
  const [dragOverIdx, setDragOverIdx] = useState<number | null>(null);
  const [ctxMenu, setCtxMenu] = useState<CtxMenu | null>(null);
  const [editingTabId, setEditingTabId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState('');
  const menuRef = useRef<HTMLDivElement>(null);

  // Close context menu on outside click
  useEffect(() => {
    if (!ctxMenu) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setCtxMenu(null);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [ctxMenu]);

  const handleCloseClick = useCallback(
    (e: React.MouseEvent, tab: EditorTab) => {
      e.stopPropagation();
      if (tab.isDirty) {
        const ok = confirm(`"${tab.label}" has unsaved changes. Close anyway?`);
        if (!ok) return;
        onClose(tab.id, { force: true });
      } else {
        onClose(tab.id);
      }
    },
    [onClose],
  );

  const handleContextMenu = useCallback((e: React.MouseEvent, tab: EditorTab) => {
    e.preventDefault();
    e.stopPropagation();
    setCtxMenu({ tabId: tab.id, x: e.clientX, y: e.clientY });
  }, []);

  const startEditFromMenu = useCallback(() => {
    if (!ctxMenu) return;
    const tab = tabs.find((t) => t.id === ctxMenu.tabId);
    if (!tab) return;
    setCtxMenu(null);
    setEditingTabId(tab.id);
    setEditValue(tab.label);
  }, [ctxMenu, tabs]);

  const openRenderedViewFromMenu = useCallback(() => {
    if (!ctxMenu || !onOpenRenderedView) return;
    setCtxMenu(null);
    onOpenRenderedView(ctxMenu.tabId);
  }, [ctxMenu, onOpenRenderedView]);

  const commitRename = useCallback(async () => {
    if (!editingTabId || !editValue.trim()) { setEditingTabId(null); return; }
    const newLabel = editValue.trim();
    setEditingTabId(null);
    if (onRenameTab) await onRenameTab(editingTabId, newLabel);
  }, [editingTabId, editValue, onRenameTab]);

  const cancelRename = useCallback(() => {
    setEditingTabId(null);
    setEditValue('');
  }, []);

  const handleDragStart = useCallback((e: React.DragEvent, idx: number) => {
    setDragSrc(idx);
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setDragImage(e.currentTarget, 0, 0);
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent, idx: number) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    setDragOverIdx(idx);
  }, []);

  const handleDrop = useCallback((_e: React.DragEvent, toIdx: number) => {
    setDragSrc((currentSrc) => {
      if (currentSrc !== null && currentSrc !== toIdx) {
        onReorder(currentSrc, toIdx);
      }
      return null;
    });
    setDragOverIdx(null);
  }, [onReorder]);

  const handleDragEnd = useCallback(() => {
    setDragSrc(null);
    setDragOverIdx(null);
  }, []);

  if (tabs.length === 0) return null;

  return (
    <>
      <div className="tab-bar" role="tablist" aria-label="Open files">
        {tabs.map((tab, idx) => {
          const isActive = tab.id === activeTabId;
          const isDragTarget = dragOverIdx === idx && dragSrc !== idx;
          const isEditing = editingTabId === tab.id;
          const tabTitle = tab.viewMode === 'preview' ? `${tab.relPath} · Rendered preview` : tab.relPath;
          return (
            <div
              key={tab.id}
              role="tab"
              aria-selected={isActive}
              className={[
                'tab',
                isActive ? 'tab--active' : '',
                tab.isDirty ? 'tab--dirty' : '',
                isDragTarget ? 'tab--drag-target' : '',
              ].filter(Boolean).join(' ')}
              title={tabTitle}
              draggable={!isEditing}
              onClick={() => { if (!isEditing) onActivate(tab.id); }}
              onContextMenu={(e) => handleContextMenu(e, tab)}
              onDragStart={(e) => handleDragStart(e, idx)}
              onDragOver={(e) => handleDragOver(e, idx)}
              onDrop={(e) => handleDrop(e, idx)}
              onDragEnd={handleDragEnd}
              onDragLeave={() => setDragOverIdx(null)}
              tabIndex={isActive ? 0 : -1}
            >
              {isEditing ? (
                <input
                  className="tab-rename-input"
                  value={editValue}
                  autoFocus
                  onClick={(e) => e.stopPropagation()}
                  onChange={(e) => setEditValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') { e.stopPropagation(); void commitRename(); }
                    if (e.key === 'Escape') { e.stopPropagation(); cancelRename(); }
                  }}
                  onBlur={cancelRename}
                />
              ) : (
                <span className="tab-label-wrap">
                  <span className="tab-label">{tab.label}</span>
                  {tab.viewMode === 'preview' && <span className="tab-view-badge">Rendered</span>}
                </span>
              )}
              {tab.isDirty && !isEditing && (
                <span className="tab-dirty-dot" aria-label="Unsaved">●</span>
              )}
              <button
                type="button"
                className="tab-close"
                aria-label={`Close ${tab.label}`}
                onClick={(e) => handleCloseClick(e, tab)}
                tabIndex={-1}
              >
                ×
              </button>
            </div>
          );
        })}
      </div>

      {ctxMenu && (
        (() => {
          const ctxTab = tabs.find((tab) => tab.id === ctxMenu.tabId);
          const canOpenRenderedView = Boolean(
            ctxTab
            && ctxTab.viewMode === 'edit'
            && ctxTab.kind === 'text'
            && isMarkdownPath(ctxTab.relPath)
            && onOpenRenderedView,
          );

          return (
            <div
              ref={menuRef}
              className="tab-context-menu"
              style={{ top: ctxMenu.y, left: ctxMenu.x }}
            >
              {canOpenRenderedView && (
                <button onClick={openRenderedViewFromMenu}>👁 Rendered View</button>
              )}
              <button onClick={startEditFromMenu}>✏️ Rename</button>
            </div>
          );
        })()
      )}
    </>
  );
};

export default TabBar;

