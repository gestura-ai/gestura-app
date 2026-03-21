import { useCallback, useEffect, useState } from "react";
import {
  getSessionWorkspaceById,
  pickWorkspaceDirectory,
  setSessionPermissionLevel,
} from "../../../services/tauri/agent";
import { getConfig, saveConfig } from "../../../services/tauri/config";
import type { AppConfig } from "../../../types/config";
import type { ToastKind } from "../hooks/useToast";

interface SessionSettingsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
  toolSettings: Record<string, unknown>;
  onWorkspaceChanged?: (workspace: string) => void;
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
  onWorkspaceChanged,
  onShowToast,
}: SessionSettingsPanelProps) {
  const [workspace, setWorkspace] = useState<string>("");
  const [permissionLevel, setPermissionLevel] = useState("restricted");
  const [appConfig, setAppConfig] = useState<AppConfig | null>(null);
  const [isSavingPipelineSettings, setIsSavingPipelineSettings] = useState(false);

  const loadWorkspace = useCallback(async () => {
    try {
      const dir = await getSessionWorkspaceById(sessionId);
      setWorkspace(dir ?? "");
    } catch { /* ignore */ }
  }, [sessionId]);

  useEffect(() => {
    if (isOpen) void loadWorkspace();
  }, [isOpen, loadWorkspace]);

  useEffect(() => {
    if (!isOpen) return;
    void getConfig().then(setAppConfig).catch(() => { /* ignore */ });
  }, [isOpen]);

  // Load permission level from settings
  useEffect(() => {
    const level = toolSettings.permission_level as string | undefined;
    if (level) setPermissionLevel(level);
  }, [toolSettings]);

  const handleChangeWorkspace = useCallback(async () => {
    try {
      const dir = await pickWorkspaceDirectory(sessionId);
      if (dir) {
        setWorkspace(dir);
        onWorkspaceChanged?.(dir);
        onShowToast("Workspace updated", "success");
      }
    } catch (e) {
      onShowToast(`Failed to change workspace: ${e}`, "error");
    }
  }, [sessionId, onShowToast, onWorkspaceChanged]);

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

  const updateGlobalPipelineSettings = useCallback(
    async (
      updates: Partial<AppConfig["pipeline"]>,
      successMessage: string,
      failureLabel: string,
    ) => {
      if (!appConfig) return;
      const previous = appConfig;
      const next: AppConfig = {
        ...appConfig,
        pipeline: {
          ...appConfig.pipeline,
          ...updates,
        },
      };

      setAppConfig(next);
      setIsSavingPipelineSettings(true);
      try {
        await saveConfig(next);
        onShowToast(successMessage, "success");
      } catch (e) {
        setAppConfig(previous);
        onShowToast(`Failed to update ${failureLabel}: ${e}`, "error");
      } finally {
        setIsSavingPipelineSettings(false);
      }
    },
    [appConfig, onShowToast],
  );

  const handleReflectionToggle = useCallback(
    async (enabled: boolean) => {
      if (!appConfig) return;
      await updateGlobalPipelineSettings(
        {
          reflection: {
            ...appConfig.pipeline.reflection,
            enabled,
          },
        },
        enabled ? "Experiential reflection enabled" : "Experiential reflection disabled",
        "reflection settings",
      );
    },
    [appConfig, updateGlobalPipelineSettings],
  );

  const handleIterationBudgetToggle = useCallback(
    async (enabled: boolean) => {
      await updateGlobalPipelineSettings(
        { iteration_budget_enabled: enabled },
        enabled ? "Iteration budgets enabled" : "Iteration budgets disabled",
        "iteration budget settings",
      );
    },
    [updateGlobalPipelineSettings],
  );

  const handleIterationBudgetValue = useCallback(
    async (field: "max_iterations" | "tracked_task_max_iterations", value: number) => {
      await updateGlobalPipelineSettings(
        { [field]: value },
        "Iteration budget updated",
        "iteration budget settings",
      );
    },
    [updateGlobalPipelineSettings],
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

          <div className="session-field">
            <label>Experiential Reflection (Global)</label>
            <label className="tool-checkbox">
              <input
                type="checkbox"
                checked={appConfig?.pipeline.reflection.enabled ?? false}
                disabled={!appConfig || isSavingPipelineSettings}
                onChange={(e) => void handleReflectionToggle(e.target.checked)}
              />
              <div className="tool-details">
                <span className="tool-name">Enable reflection for low-quality turns</span>
                <span className="tool-summary">
                  Reflection only runs when enabled and a turn scores below the configured quality threshold.
                </span>
              </div>
            </label>
          </div>

          <div className="session-field">
            <label>Iteration Budgets (Global)</label>
            <label className="tool-checkbox">
              <input
                type="checkbox"
                checked={appConfig?.pipeline.iteration_budget_enabled ?? false}
                disabled={!appConfig || isSavingPipelineSettings}
                onChange={(e) => void handleIterationBudgetToggle(e.target.checked)}
              />
              <div className="tool-details">
                <span className="tool-name">Enable explicit agent iteration budgets</span>
                <span className="tool-summary">
                  When disabled, requests run unbounded until they finish naturally or you cancel them.
                </span>
              </div>
            </label>
          </div>

          <div className="session-field">
            <label>General Request Iteration Budget</label>
            <input
              type="number"
              min="1"
              max="500"
              value={appConfig?.pipeline.max_iterations ?? 10}
              disabled={!appConfig || isSavingPipelineSettings || !appConfig.pipeline.iteration_budget_enabled}
              onChange={(e) => void handleIterationBudgetValue("max_iterations", Number.parseInt(e.target.value, 10) || 10)}
            />
          </div>

          <div className="session-field">
            <label>Tracked Task Iteration Budget</label>
            <input
              type="number"
              min="1"
              max="1000"
              value={appConfig?.pipeline.tracked_task_max_iterations ?? 30}
              disabled={!appConfig || isSavingPipelineSettings || !appConfig.pipeline.iteration_budget_enabled}
              onChange={(e) => void handleIterationBudgetValue("tracked_task_max_iterations", Number.parseInt(e.target.value, 10) || 30)}
            />
          </div>

        </div>
      </div>
    </>
  );
}

