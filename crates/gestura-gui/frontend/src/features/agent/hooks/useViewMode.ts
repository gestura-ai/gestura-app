import { useState, useCallback } from 'react';
import type { ViewMode } from '../types';

const STORAGE_KEY = 'gestura:agent:viewMode';

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
  const [viewMode, setViewModeState] = useState<ViewMode>(() => {
    const stored = sessionStorage.getItem(STORAGE_KEY);
    return stored === 'editor' ? 'editor' : 'message-only';
  });

  const setViewMode = useCallback((mode: ViewMode) => {
    sessionStorage.setItem(STORAGE_KEY, mode);
    setViewModeState(mode);
  }, []);

  const toggleViewMode = useCallback(() => {
    setViewModeState((prev) => {
      const next: ViewMode = prev === 'message-only' ? 'editor' : 'message-only';
      sessionStorage.setItem(STORAGE_KEY, next);
      return next;
    });
  }, []);

  return { viewMode, setViewMode, toggleViewMode };
}

