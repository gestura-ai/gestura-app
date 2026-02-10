import { useEffect, useRef } from 'react';

/**
 * Runs `callback` on a fixed interval.
 *
 * - Uses a ref to avoid stale closures.
 * - Pass `null` for `delayMs` to disable.
 */
export function useInterval(callback: () => void, delayMs: number | null) {
  const callbackRef = useRef(callback);

  useEffect(() => {
    callbackRef.current = callback;
  }, [callback]);

  useEffect(() => {
    if (delayMs === null) return;
    if (typeof window === 'undefined') return;

    const id = window.setInterval(() => {
      callbackRef.current();
    }, delayMs);

    return () => {
      window.clearInterval(id);
    };
  }, [delayMs]);
}
