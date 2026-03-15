import { useState, useCallback } from 'react';
import type { ViewMode } from '../types';

const STORAGE_KEY = 'gestura:agent:viewMode';

function readViewMode(): ViewMode {
  try {
    const stored = sessionStorage.getItem(STORAGE_KEY);
    return stored === 'editor' ? 'editor' : 'message-only';
  } catch {
    return 'message-only';
  }
}

function persistViewMode(mode: ViewMode): void {
  try {
    sessionStorage.setItem(STORAGE_KEY, mode);
  } catch {
    // Some native webview environments can throw when sessionStorage is
    // unavailable. Treat persistence as best-effort so the agent window can
    // still boot.
  }
}

/**
 * Manages the agent window's two-mode layout state.
 *
 * - `message-only` — original single-panel chat view
 * - `editor`       — three-panel layout (explorer | editor tabs | chat)
 *
 * The chosen mode is persisted in `sessionStorage` so it survives hot reloads
 * but resets when the window is closed.
 */
export function useViewMode() {
  const [viewMode, setViewModeState] = useState<ViewMode>(() => readViewMode());

  const setViewMode = useCallback((mode: ViewMode) => {
    persistViewMode(mode);
    setViewModeState(mode);
  }, []);

  const toggleViewMode = useCallback(() => {
    setViewModeState((prev) => {
      const next: ViewMode = prev === 'message-only' ? 'editor' : 'message-only';
      persistViewMode(next);
      return next;
    });
  }, []);

  return { viewMode, setViewMode, toggleViewMode };
}

