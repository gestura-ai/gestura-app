import { useState, useCallback } from 'react';
import type { EditorTab } from '../types';

let tabCounter = 0;
function nextTabId(): string {
  return `tab-${Date.now()}-${++tabCounter}`;
}

const TABS_STORAGE_KEY = 'gestura:agent:tabs';
const ACTIVE_STORAGE_KEY = 'gestura:agent:activeTab';

function loadPersistedTabs(): EditorTab[] {
  try {
    const raw = sessionStorage.getItem(TABS_STORAGE_KEY);
    if (raw) return JSON.parse(raw) as EditorTab[];
  } catch {
    // ignore
  }
  return [];
}

function loadPersistedActiveTabId(): string | null {
  try {
    const stored = sessionStorage.getItem(ACTIVE_STORAGE_KEY);
    return stored || null;
  } catch {
    return null;
  }
}

function persistTabs(tabs: EditorTab[], activeId: string | null) {
  try {
    sessionStorage.setItem(TABS_STORAGE_KEY, JSON.stringify(tabs));
    sessionStorage.setItem(ACTIVE_STORAGE_KEY, activeId ?? '');
  } catch {
    // storage may be unavailable; non-fatal
  }
}

/**
 * Manages the multi-tab editor state:
 * - open / close / switch tabs
 * - mark tabs dirty / clean
 * - update content
 * - reorder tabs via drag-and-drop
 * - persist open tabs in sessionStorage
 */
export function useTabState() {
  const [tabs, setTabs] = useState<EditorTab[]>(() => loadPersistedTabs());
  const [activeTabId, setActiveTabId] = useState<string | null>(() => loadPersistedActiveTabId());

  const openTab = useCallback(
    (file: Omit<EditorTab, 'id' | 'isDirty' | 'scrollOffset' | 'isDiffView'>) => {
      setTabs((prev) => {
        // If already open, just activate it
        const existing = prev.find((t) => t.relPath === file.relPath);
        if (existing) {
          setActiveTabId(existing.id);
          persistTabs(prev, existing.id);
          return prev;
        }
        const newTab: EditorTab = {
          ...file,
          id: nextTabId(),
          isDirty: false,
          scrollOffset: 0,
          isDiffView: false,
        };
        const next = [...prev, newTab];
        setActiveTabId(newTab.id);
        persistTabs(next, newTab.id);
        return next;
      });
    },
    []
  );

  const closeTab = useCallback(
    (tabId: string, opts?: { force?: boolean }): boolean => {
      let shouldClose = true;
      setTabs((prev) => {
        const tab = prev.find((t) => t.id === tabId);
        if (!tab) return prev;
        if (tab.isDirty && !opts?.force) {
          shouldClose = false;
          return prev;
        }
        const next = prev.filter((t) => t.id !== tabId);
        // Pick a sensible next active tab
        setActiveTabId((currentActive) => {
          if (currentActive !== tabId) return currentActive;
          const idx = prev.findIndex((t) => t.id === tabId);
          const neighbor = next[idx] ?? next[idx - 1] ?? next[0];
          const nextActive = neighbor?.id ?? null;
          persistTabs(next, nextActive);
          return nextActive;
        });
        if (shouldClose) persistTabs(next, null);
        return next;
      });
      return shouldClose;
    },
    []
  );

  const activateTab = useCallback((tabId: string) => {
    setActiveTabId(tabId);
    setTabs((prev) => {
      persistTabs(prev, tabId);
      return prev;
    });
  }, []);

  const updateTabContent = useCallback((tabId: string, content: string) => {
    setTabs((prev) => {
      const next = prev.map((t) =>
        t.id === tabId ? { ...t, content, isDirty: true } : t
      );
      persistTabs(next, tabId);
      return next;
    });
  }, []);

  /**
   * Update the content of a tab WITHOUT marking it dirty.
   * Used for external refreshes (e.g. agent file edits) where the user
   * has no unsaved changes and the file should just reflect the on-disk state.
   */
  const refreshTabContent = useCallback((tabId: string, content: string) => {
    setTabs((prev) => {
      const next = prev.map((t) =>
        t.id === tabId ? { ...t, content } : t
      );
      persistTabs(next, activeTabId);
      return next;
    });
  }, [activeTabId]);

  const markTabClean = useCallback((tabId: string) => {
    setTabs((prev) => {
      const next = prev.map((t) =>
        t.id === tabId ? { ...t, isDirty: false } : t
      );
      persistTabs(next, activeTabId);
      return next;
    });
  }, [activeTabId]);

  const toggleDiffView = useCallback((tabId: string) => {
    setTabs((prev) =>
      prev.map((t) =>
        t.id === tabId ? { ...t, isDiffView: !t.isDiffView } : t
      )
    );
  }, []);

  const reorderTabs = useCallback((fromIndex: number, toIndex: number) => {
    setTabs((prev) => {
      if (fromIndex === toIndex) return prev;
      const next = [...prev];
      const [moved] = next.splice(fromIndex, 1);
      next.splice(toIndex, 0, moved);
      persistTabs(next, activeTabId);
      return next;
    });
  }, [activeTabId]);

  const updateScrollOffset = useCallback((tabId: string, offset: number) => {
    setTabs((prev) => {
      const next = prev.map((t) =>
        t.id === tabId ? { ...t, scrollOffset: offset } : t
      );
      persistTabs(next, activeTabId);
      return next;
    });
  }, [activeTabId]);

  /**
   * Update the label and relPath of a tab (used after renaming a file on disk).
   */
  const renameTab = useCallback((tabId: string, newLabel: string, newRelPath: string) => {
    setTabs((prev) => {
      const next = prev.map((t) =>
        t.id === tabId ? { ...t, label: newLabel, relPath: newRelPath } : t
      );
      persistTabs(next, activeTabId);
      return next;
    });
  }, [activeTabId]);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;

  return {
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
  };
}

