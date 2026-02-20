import type { Toast } from "../hooks/useToast";

interface ToastContainerProps {
  toasts: Toast[];
  onDismiss: (id: number) => void;
}

/**
 * Renders the floating toast stack in the top-right corner.
 * Matches the .toast-container / .toast structure from agent.html.
 */
export function ToastContainer({ toasts, onDismiss }: ToastContainerProps) {
  if (toasts.length === 0) return null;

  return (
    <div className="toast-container">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`toast toast-${t.kind}${t.visible ? " visible" : " hiding"}`}
          onClick={() => onDismiss(t.id)}
          title="Click to dismiss"
        >
          {t.message}
        </div>
      ))}
    </div>
  );
}

