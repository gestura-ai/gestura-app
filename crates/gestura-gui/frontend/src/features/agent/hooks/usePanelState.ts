import { useCallback, useEffect, useState } from 'react';

/** All named side panels in the chat window. */
export type PanelName =
  | 'menu'
  | 'tasks'
  | 'memory'
  | 'knowledge'
  | 'providers'
  | 'settings'
  | 'none';

export type ShellPanelMode = 'expanded' | 'collapsed';

export interface ShellManagerPanelState {
  visible: boolean;
  mode: ShellPanelMode;
  height: number;
  activeShellId: string | null;
  tabOrder: string[];
  closedShellIds: string[];
}

const SHELL_MANAGER_VISIBLE_KEY = 'gestura.agent.shellManager.visible';
const SHELL_MANAGER_MODE_KEY = 'gestura.agent.shellManager.mode';
const SHELL_MANAGER_HEIGHT_KEY = 'gestura.agent.shellManager.height';
const MIN_SHELL_MANAGER_HEIGHT = 180;

function getDefaultShellManagerHeight(): number {
  if (typeof window === 'undefined') return 220;
  return Math.max(MIN_SHELL_MANAGER_HEIGHT, Math.round(window.innerHeight * 0.2));
}

function readSessionBoolean(key: string, fallback: boolean): boolean {
  if (typeof window === 'undefined') return fallback;
  try {
    const value = window.sessionStorage.getItem(key);
    return value == null ? fallback : value === 'true';
  } catch {
    return fallback;
  }
}

function readSessionNumber(key: string, fallback: number): number {
  if (typeof window === 'undefined') return fallback;
  try {
    const value = Number(window.localStorage.getItem(key));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  } catch {
    return fallback;
  }
}

function readSessionMode(key: string, fallback: ShellPanelMode): ShellPanelMode {
  if (typeof window === 'undefined') return fallback;
  try {
    const value = window.sessionStorage.getItem(key);
    return value === 'collapsed' ? 'collapsed' : value === 'expanded' || value === 'maximized' ? 'expanded' : fallback;
  } catch {
    return fallback;
  }
}

export interface PanelState {
  /** Currently visible panel, or "none". */
  activePanel: PanelName;
  /** Returns true when the given panel is open. */
  isOpen: (panel: PanelName) => boolean;
  /** Open a specific panel (closes whatever was open). */
  openPanel: (panel: PanelName) => void;
  /** Close whatever panel is currently open. */
  closePanel: () => void;
  /** Toggle a panel — opens if closed, closes if already open. */
  togglePanel: (panel: PanelName) => void;
  /** Shell Manager dock state. */
  shellManager: ShellManagerPanelState;
  /** Show the Shell Manager dock. */
  openShellManager: () => void;
  /** Hide the Shell Manager dock. */
  closeShellManager: () => void;
  /** Toggle the Shell Manager dock. */
  toggleShellManager: () => void;
  /** Update the Shell Manager display mode. */
  setShellManagerMode: (mode: ShellPanelMode) => void;
  /** Update the Shell Manager height when resized. */
  setShellManagerHeight: (height: number) => void;
  /** Select the active shell tab. */
  setActiveShell: (shellId: string | null) => void;
  /** Sync the available shell tabs with live shell sessions. */
  syncShellTabs: (
    shellIds: string[],
    preferredActiveId?: string | null,
  ) => void;
  /** Reorder shell tabs after drag-and-drop. */
  reorderShellTabs: (sourceId: string, targetId: string) => void;
  /** Close a shell tab within the manager without affecting the underlying process. */
  closeShellTab: (shellId: string) => void;
}

/**
 * Manages which side panel is currently visible.
 * Only one panel (or the menu) can be open at a time.
 */
export function usePanelState(): PanelState {
  const [activePanel, setActivePanel] = useState<PanelName>('none');
  const [shellManager, setShellManager] = useState<ShellManagerPanelState>(() => ({
    visible: readSessionBoolean(SHELL_MANAGER_VISIBLE_KEY, false),
    mode: readSessionMode(SHELL_MANAGER_MODE_KEY, 'expanded'),
    height: readSessionNumber(SHELL_MANAGER_HEIGHT_KEY, getDefaultShellManagerHeight()),
    activeShellId: null,
    tabOrder: [],
    closedShellIds: [],
  }));

  useEffect(() => {
    if (typeof window === 'undefined') return;
    try {
      window.sessionStorage.setItem(SHELL_MANAGER_VISIBLE_KEY, String(shellManager.visible));
    } catch {
      // Ignore storage failures and keep in-memory state authoritative.
    }
  }, [shellManager.visible]);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    try {
      window.sessionStorage.setItem(SHELL_MANAGER_MODE_KEY, shellManager.mode);
    } catch {
      // Ignore storage failures and keep in-memory state authoritative.
    }
  }, [shellManager.mode]);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    try {
      window.localStorage.setItem(SHELL_MANAGER_HEIGHT_KEY, String(shellManager.height));
    } catch {
      // Ignore storage failures and keep in-memory state authoritative.
    }
  }, [shellManager.height]);

  const isOpen = useCallback(
    (panel: PanelName) => activePanel === panel,
    [activePanel],
  );

  const openPanel = useCallback((panel: PanelName) => {
    setActivePanel(panel);
  }, []);

  const closePanel = useCallback(() => {
    setActivePanel('none');
  }, []);

  const togglePanel = useCallback(
    (panel: PanelName) => {
      setActivePanel((current) => (current === panel ? 'none' : panel));
    },
    [],
  );

  const openShellManager = useCallback(() => {
    setShellManager((current) => ({ ...current, visible: true, mode: 'expanded' }));
  }, []);

  const closeShellManager = useCallback(() => {
    setShellManager((current) => ({ ...current, visible: false }));
  }, []);

  const toggleShellManager = useCallback(() => {
    setShellManager((current) => current.visible
      ? { ...current, visible: false }
      : { ...current, visible: true, mode: 'expanded' });
  }, []);

  const setShellManagerMode = useCallback((mode: ShellPanelMode) => {
    setShellManager((current) => ({ ...current, mode }));
  }, []);

  const setShellManagerHeight = useCallback((height: number) => {
    const maxHeight = typeof window === 'undefined'
      ? Math.round(height)
      : Math.max(MIN_SHELL_MANAGER_HEIGHT, Math.round(window.innerHeight * 0.75));
    const nextHeight = Math.max(MIN_SHELL_MANAGER_HEIGHT, Math.min(maxHeight, Math.round(height)));
    setShellManager((current) => ({ ...current, height: nextHeight }));
  }, []);

  const setActiveShell = useCallback((shellId: string | null) => {
    setShellManager((current) => ({ ...current, activeShellId: shellId }));
  }, []);

  const syncShellTabs = useCallback((
    shellIds: string[],
    preferredActiveId?: string | null,
  ) => {
    setShellManager((current) => {
      const incomingIds = Array.from(new Set(shellIds.filter(Boolean)));
      const incomingSet = new Set(incomingIds);
      const closedShellIds = current.closedShellIds.filter(
        (shellId) => incomingSet.has(shellId),
      );
      const visibleIds = incomingIds.filter((shellId) => !closedShellIds.includes(shellId));
      const orderedIds = [
        ...current.tabOrder.filter((shellId) => visibleIds.includes(shellId)),
        ...visibleIds.filter((shellId) => !current.tabOrder.includes(shellId)),
      ];

      let activeShellId = current.activeShellId;
      if (!activeShellId || !orderedIds.includes(activeShellId)) {
        activeShellId = preferredActiveId && orderedIds.includes(preferredActiveId)
          ? preferredActiveId
          : orderedIds[orderedIds.length - 1] ?? null;
      }

      return {
        ...current,
        activeShellId,
        tabOrder: orderedIds,
        closedShellIds,
      };
    });
  }, []);

  const reorderShellTabs = useCallback((sourceId: string, targetId: string) => {
    if (!sourceId || !targetId || sourceId === targetId) return;

    setShellManager((current) => {
      if (!current.tabOrder.includes(sourceId) || !current.tabOrder.includes(targetId)) {
        return current;
      }

      const nextOrder = current.tabOrder.filter((shellId) => shellId !== sourceId);
      const targetIndex = nextOrder.indexOf(targetId);
      nextOrder.splice(targetIndex, 0, sourceId);

      return { ...current, tabOrder: nextOrder };
    });
  }, []);

  const closeShellTab = useCallback((shellId: string) => {
    setShellManager((current) => {
      if (!current.tabOrder.includes(shellId)) return current;

      const nextOrder = current.tabOrder.filter((tabId) => tabId !== shellId);
      const nextClosedShellIds = current.closedShellIds.includes(shellId)
        ? current.closedShellIds
        : [...current.closedShellIds, shellId];
      const currentIndex = current.tabOrder.indexOf(shellId);
      const nextActiveShellId = current.activeShellId === shellId
        ? nextOrder[currentIndex - 1] ?? nextOrder[0] ?? null
        : current.activeShellId;

      return {
        ...current,
        activeShellId: nextActiveShellId,
        tabOrder: nextOrder,
        closedShellIds: nextClosedShellIds,
      };
    });
  }, []);

  return {
    activePanel,
    isOpen,
    openPanel,
    closePanel,
    togglePanel,
    shellManager,
    openShellManager,
    closeShellManager,
    toggleShellManager,
    setShellManagerMode,
    setShellManagerHeight,
    setActiveShell,
    syncShellTabs,
    reorderShellTabs,
    closeShellTab,
  };
}

