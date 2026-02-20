import { useCallback, useState } from "react";
import {
  setKnowledgeEnabled,
} from "../../../services/tauri/agent";
import type { KnowledgeItem } from "../types";
import type { ToastKind } from "../hooks/useToast";

interface KnowledgePanelProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
  knowledgeItems: KnowledgeItem[];
  onRefreshKnowledge: () => Promise<void>;
  onShowToast: (msg: string, kind?: ToastKind) => void;
}

/**
 * Side panel for browsing and toggling knowledge base items.
 * Matches .session-panel structure from agent.html.
 */
export function KnowledgePanel({
  isOpen,
  onClose,
  sessionId,
  knowledgeItems,
  onRefreshKnowledge,
  onShowToast,
}: KnowledgePanelProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [toggling, setToggling] = useState<Set<string>>(new Set());

  const enabledCount = knowledgeItems.filter((k) => k.enabled).length;

  const filtered = knowledgeItems.filter(
    (k) =>
      !searchQuery ||
      (k.name ?? "").toLowerCase().includes(searchQuery.toLowerCase()) ||
      (k.description ?? "").toLowerCase().includes(searchQuery.toLowerCase()),
  );

  const handleToggle = useCallback(
    async (item: KnowledgeItem) => {
      if (toggling.has(item.id)) return;
      setToggling((prev) => new Set([...prev, item.id]));
      try {
        await setKnowledgeEnabled(sessionId, item.id, !item.enabled);
        await onRefreshKnowledge();
      } catch (e) {
        onShowToast(`Failed to update knowledge: ${e}`, "error");
      } finally {
        setToggling((prev) => {
          const next = new Set(prev);
          next.delete(item.id);
          return next;
        });
      }
    },
    [sessionId, toggling, onRefreshKnowledge, onShowToast],
  );

  return (
    <>
      <div
        className={`session-panel-overlay${isOpen ? " visible" : ""}`}
        onClick={onClose}
      />
      <div className={`session-panel${isOpen ? " open" : ""}`}>
        <div className="session-panel-header">
          <h3>Knowledge</h3>
          <button className="session-panel-close" onClick={onClose} title="Close">
            <span className="icon-close" />
          </button>
        </div>

        <div className="session-panel-content">
          <div className="knowledge-info">
            <p className="knowledge-enabled-count">
              {enabledCount} of {knowledgeItems.length} item
              {knowledgeItems.length !== 1 ? "s" : ""} enabled
            </p>
          </div>

          <div className="knowledge-toolbar">
            <input
              className="knowledge-search"
              type="text"
              placeholder="Search knowledge..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>

          <div className="knowledge-list">
            {filtered.length === 0 ? (
              <div className="task-empty">
                {searchQuery
                  ? "No knowledge items match your search."
                  : "No knowledge items available."}
              </div>
            ) : (
              filtered.map((item) => (
                <div
                  key={item.id}
                  className={`knowledge-item${item.enabled ? " enabled" : ""}`}
                >
                  <label className="knowledge-checkbox">
                    <input
                      type="checkbox"
                      checked={!!item.enabled}
                      disabled={toggling.has(item.id)}
                      onChange={() => handleToggle(item)}
                    />
                  </label>
                  <div className="knowledge-content">
                    <div className="knowledge-header">
                      <span className="knowledge-name">{item.name}</span>
                      {item.category && (
                        <span className="knowledge-category">{item.category}</span>
                      )}
                    </div>
                    {item.description && (
                      <p className="knowledge-description">{item.description}</p>
                    )}
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </>
  );
}

