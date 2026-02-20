/**
 * TabBar — horizontal tab strip for the multi-tab editor.
 *
 * Features:
 * - Renders all open tabs (EditorTab[])
 * - Highlights the active tab
 * - Shows dirty indicator (•) for unsaved tabs
 * - Close button with dirty-state guard (propagates up to EditorArea)
 * - Drag-to-reorder via HTML5 drag-and-drop API
 */
import React, { useCallback, useState } from 'react';
import type { EditorTab } from '../types';
import './TabBar.css';

export interface TabBarProps {
  tabs: EditorTab[];
  activeTabId: string | null;
  onActivate: (tabId: string) => void;
  /** Returns false when the tab has unsaved changes and close was prevented. */
  onClose: (tabId: string, opts?: { force?: boolean }) => boolean;
  onReorder: (fromIndex: number, toIndex: number) => void;
}

export const TabBar: React.FC<TabBarProps> = ({
  tabs, activeTabId, onActivate, onClose, onReorder,
}) => {
  const [dragSrc, setDragSrc] = useState<number | null>(null);
  const [dragOverIdx, setDragOverIdx] = useState<number | null>(null);

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

  const handleDragStart = useCallback((e: React.DragEvent, idx: number) => {
    setDragSrc(idx);
    e.dataTransfer.effectAllowed = 'move';
    // Ghost image: use the tab element itself
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
    <div className="tab-bar" role="tablist" aria-label="Open files">
      {tabs.map((tab, idx) => {
        const isActive = tab.id === activeTabId;
        const isDragTarget = dragOverIdx === idx && dragSrc !== idx;
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
            title={tab.relPath}
            draggable
            onClick={() => onActivate(tab.id)}
            onDragStart={(e) => handleDragStart(e, idx)}
            onDragOver={(e) => handleDragOver(e, idx)}
            onDrop={(e) => handleDrop(e, idx)}
            onDragEnd={handleDragEnd}
            onDragLeave={() => setDragOverIdx(null)}
            tabIndex={isActive ? 0 : -1}
          >
            <span className="tab-label">{tab.label}</span>
            {tab.isDirty && <span className="tab-dirty-dot" aria-label="Unsaved">●</span>}
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
  );
};

export default TabBar;

