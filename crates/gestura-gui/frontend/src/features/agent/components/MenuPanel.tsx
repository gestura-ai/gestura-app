import type { PanelName } from "../hooks/usePanelState";

interface MenuPanelProps {
  isOpen: boolean;
  onClose: () => void;
  onNavigate: (panel: PanelName) => void;
}

/**
 * Main navigation menu panel.
 * Slides in from the right when the settings gear is clicked.
 * Each menu item navigates to the corresponding settings panel.
 */
export function MenuPanel({ isOpen, onClose, onNavigate }: MenuPanelProps) {
  function handleItem(panel: PanelName) {
    onClose();
    onNavigate(panel);
  }

  return (
    <>
      {/* Overlay */}
      <div
        className={`session-panel-overlay${isOpen ? " visible" : ""}`}
        onClick={onClose}
      />

      {/* Menu panel */}
      <div className={`menu-panel${isOpen ? " open" : ""}`}>
        <div className="menu-panel-header">
          <h3>Menu</h3>
          <button
            className="session-panel-close"
            onClick={onClose}
            title="Close"
          >
            <span className="icon-close" />
          </button>
        </div>

        <div className="menu-list">
          <div
            className="menu-item"
            onClick={() => handleItem("settings")}
          >
            <span className="icon-settings" />
            <span className="menu-item-label">Settings</span>
          </div>

          <div
            className="menu-item"
            onClick={() => handleItem("providers")}
          >
            <span className="icon-server-01" />
            <span className="menu-item-label">Providers</span>
          </div>

          <div
            className="menu-item"
            onClick={() => handleItem("knowledge")}
          >
            <span className="icon-knowledge" />
            <span className="menu-item-label">Knowledge</span>
          </div>

          <div
            className="menu-item"
            onClick={() => handleItem("memory")}
          >
            <span className="icon-brain" />
            <span className="menu-item-label">Memory</span>
          </div>

          <div
            className="menu-item"
            onClick={() => handleItem("tasks")}
          >
            <span className="icon-checklist" />
            <span className="menu-item-label">Tasks</span>
          </div>
        </div>
      </div>
    </>
  );
}

