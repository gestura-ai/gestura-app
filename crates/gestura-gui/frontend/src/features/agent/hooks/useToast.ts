import { useCallback, useRef, useState } from "react";

export type ToastKind = "success" | "error" | "warning" | "info";

export interface Toast {
  id: number;
  message: string;
  kind: ToastKind;
  /** Whether the "visible" CSS class should be applied. */
  visible: boolean;
}

export interface ToastState {
  toasts: Toast[];
  /** Show a toast notification for `durationMs` (default 3000). */
  showToast: (message: string, kind?: ToastKind, durationMs?: number) => void;
  /** Dismiss a specific toast immediately. */
  dismissToast: (id: number) => void;
}

let _nextId = 1;

/**
 * Manages a stack of toast notifications.
 * Each toast auto-dismisses after `durationMs` with a hide animation.
 */
export function useToast(): ToastState {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const timers = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  const dismissToast = useCallback((id: number) => {
    // First mark as "hiding" for the CSS fade-out, then remove
    setToasts((prev) =>
      prev.map((t) => (t.id === id ? { ...t, visible: false } : t)),
    );
    const removeTimer = setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 200); // matches CSS transition duration
    timers.current.set(id, removeTimer);
  }, []);

  const showToast = useCallback(
    (message: string, kind: ToastKind = "info", durationMs = 3000) => {
      const id = _nextId++;
      // Add invisible first so CSS transition triggers
      setToasts((prev) => [...prev, { id, message, kind, visible: false }]);
      // Trigger visible on next frame
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          setToasts((prev) =>
            prev.map((t) => (t.id === id ? { ...t, visible: true } : t)),
          );
        });
      });
      // Auto-dismiss
      const timer = setTimeout(() => dismissToast(id), durationMs);
      timers.current.set(id, timer);
    },
    [dismissToast],
  );

  return { toasts, showToast, dismissToast };
}

