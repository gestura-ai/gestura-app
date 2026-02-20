import { useCallback, useEffect, useState } from "react";
import {
  getSessionWorkspaceById,
  pickWorkspaceDirectory,
  setSessionPermissionLevel,
  setSessionToolEnabled,
} from "../../../services/tauri/agent";
import { listBuiltinTools } from "../../../services/tauri/tools";
import type { ToolInfo } from "../../../services/tauri/tools";
import type { ToastKind } from "../hooks/useToast";

interface SessionSettingsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
  toolSettings: Record<string, unknown>;
  onRefreshToolSettings: () => Promise<void>;
  onShowToast: (msg: string, kind?: ToastKind) => void;
}

const PERMISSION_LEVELS = [
  { value: "sandbox", label: "Sandbox (Read-only)" },
  { value: "restricted", label: "Restricted (Ask before write)" },
  { value: "full", label: "Full Access (Careful!)" },
];

export function SessionSettingsPanel({
  isOpen,
  onClose,
  sessionId,
  toolSettings,
  onRefreshToolSettings,
  onShowToast,
}: SessionSettingsPanelProps) {
  const [workspace, setWorkspace] = useState<string>("");
  const [permissionLevel, setPermissionLevel] = useState("restricted");
  const [togglingTool, setTogglingTool] = useState<Set<string>>(new Set());
  const [tools, setTools] = useState<ToolInfo[]>([]);

  // Enabled tools map from backend: Record<string, boolean>
  const toolEnabledMap = (toolSettings.enabled_tools ?? {}) as Record<string, boolean>;

  const loadWorkspace = useCallback(async () => {
    try {
      const dir = await getSessionWorkspaceById(sessionId);
      setWorkspace(dir ?? "");
    } catch { /* ignore */ }
  }, [sessionId]);

  useEffect(() => {
    if (isOpen) void loadWorkspace();
  }, [isOpen, loadWorkspace]);

  // Load permission level from settings
  useEffect(() => {
    const level = toolSettings.permission_level as string | undefined;
    if (level) setPermissionLevel(level);
  }, [toolSettings]);

  // Load builtin tools list when panel opens
  useEffect(() => {
    if (isOpen && tools.length === 0) {
      void listBuiltinTools().then(setTools).catch(() => { /* ignore */ });
    }
  }, [isOpen, tools.length]);

  const handleChangeWorkspace = useCallback(async () => {
    try {
      const dir = await pickWorkspaceDirectory(sessionId);
      if (dir) {
        setWorkspace(dir);
        onShowToast("Workspace updated", "success");
      }
    } catch (e) {
      onShowToast(`Failed to change workspace: ${e}`, "error");
    }
  }, [sessionId, onShowToast]);

  const handlePermissionChange = useCallback(
    async (level: string) => {
      setPermissionLevel(level);
      try {
        await setSessionPermissionLevel(sessionId, level);
        onShowToast("Permission level updated", "success");
      } catch (e) {
        onShowToast(`Failed to update permissions: ${e}`, "error");
      }
    },
    [sessionId, onShowToast],
  );

  const handleToggleTool = useCallback(
    async (toolName: string, currentlyEnabled: boolean) => {
      if (togglingTool.has(toolName)) return;
      setTogglingTool((prev) => new Set([...prev, toolName]));
      try {
        await setSessionToolEnabled(sessionId, toolName, !currentlyEnabled);
        await onRefreshToolSettings();
      } catch (e) {
        onShowToast(`Failed to toggle tool: ${e}`, "error");
      } finally {
        setTogglingTool((prev) => {
          const next = new Set(prev);
          next.delete(toolName);
          return next;
        });
      }
    },
    [sessionId, togglingTool, onRefreshToolSettings, onShowToast],
  );

  return (
    <>
      <div
        className={`session-panel-overlay${isOpen ? " visible" : ""}`}
        onClick={onClose}
      />
      <div className={`session-panel${isOpen ? " open" : ""}`}>
        <div className="session-panel-header">
          <h3>Settings</h3>
          <button className="session-panel-close" onClick={onClose} title="Close">
            <span className="icon-close" />
          </button>
        </div>

        <div className="session-panel-content">
          {/* Workspace */}
          <div className="session-field">
            <label>Workspace Directory</label>
            <div className="session-field-row">
              <input
                type="text"
                value={workspace || "No workspace set"}
                readOnly
                className="session-info"
                title={workspace}
              />
              <button className="btn-secondary" onClick={handleChangeWorkspace}>
                Change
              </button>
            </div>
          </div>

          <div className="session-divider" />

          {/* Permission Level */}
          <div className="session-field">
            <label>Permission Level</label>
            <select
              className="provider-select"
              value={permissionLevel}
              onChange={(e) => handlePermissionChange(e.target.value)}
            >
              {PERMISSION_LEVELS.map((lvl) => (
                <option key={lvl.value} value={lvl.value}>
                  {lvl.label}
                </option>
              ))}
            </select>
          </div>

          <div className="session-divider" />

          {/* Tool Toggles */}
          <div className="session-field">
            <div className="tools-section-header">
              <span>Tools</span>
            </div>
            <div className="session-tools-list">
              {tools.length === 0 ? (
                <div className="task-empty">Loading tools…</div>
              ) : (
                tools.map((tool) => {
                  const isEnabled = toolEnabledMap[tool.name] !== false;
                  return (
                    <label key={tool.name} className="tool-checkbox">
                      <input
                        type="checkbox"
                        checked={isEnabled}
                        disabled={togglingTool.has(tool.name)}
                        onChange={() => handleToggleTool(tool.name, isEnabled)}
                      />
                      <div className="tool-details">
                        <span className="tool-name">{tool.name}</span>
                        {tool.summary && (
                          <span className="tool-summary">{tool.summary}</span>
                        )}
                      </div>
                    </label>
                  );
                })
              )}
            </div>
          </div>
        </div>
      </div>
    </>
  );
}

