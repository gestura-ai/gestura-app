import { useCallback, useState } from "react";

/** All named side panels in the chat window. */
export type PanelName =
  | "menu"
  | "tasks"
  | "knowledge"
  | "providers"
  | "settings"
  | "none";

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
}

/**
 * Manages which side panel is currently visible.
 * Only one panel (or the menu) can be open at a time.
 */
export function usePanelState(): PanelState {
  const [activePanel, setActivePanel] = useState<PanelName>("none");

  const isOpen = useCallback(
    (panel: PanelName) => activePanel === panel,
    [activePanel],
  );

  const openPanel = useCallback((panel: PanelName) => {
    setActivePanel(panel);
  }, []);

  const closePanel = useCallback(() => {
    setActivePanel("none");
  }, []);

  const togglePanel = useCallback(
    (panel: PanelName) => {
      setActivePanel((current) => (current === panel ? "none" : panel));
    },
    [],
  );

  return { activePanel, isOpen, openPanel, closePanel, togglePanel };
}

