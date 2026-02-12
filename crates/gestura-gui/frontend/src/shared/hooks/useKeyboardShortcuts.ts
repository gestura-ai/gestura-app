import { useEffect, useRef } from 'react';

export type KeyboardShortcutHandler = (event: KeyboardEvent) => void;

export interface UseKeyboardShortcutsOptions {
  enabled?: boolean;
}

/**
 * Attach a single global `keydown` listener that always calls the latest handler.
 *
 * This avoids re-registering the DOM listener on every render / state change.
 */
export function useKeyboardShortcuts(
  handler: KeyboardShortcutHandler,
  options?: UseKeyboardShortcutsOptions
): void {
  const enabled = options?.enabled ?? true;
  const handlerRef = useRef<KeyboardShortcutHandler>(handler);

  useEffect(() => {
    handlerRef.current = handler;
  }, [handler]);

  useEffect(() => {
    if (!enabled) return;
    if (typeof window === 'undefined') return;

    const onKeyDown = (event: KeyboardEvent) => {
      handlerRef.current(event);
    };

    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [enabled]);
}
