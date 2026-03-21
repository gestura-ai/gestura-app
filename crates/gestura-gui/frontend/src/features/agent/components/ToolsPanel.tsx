import { useCallback, useEffect, useState } from "react";
import { setSessionToolEnabled } from "../../../services/tauri/agent";
import {
  connectMcpServer,
  listConnectedMcpServers,
  listMcpClientTools,
  listMcpTools,
} from "../../../services/tauri/mcp";
import type { McpClientTool, McpServer } from "../../../services/tauri/mcp";
import { listBuiltinTools } from "../../../services/tauri/tools";
import type { ToolInfo } from "../../../services/tauri/tools";
import type { ToastKind } from "../hooks/useToast";

interface ToolsPanelProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
  toolSettings: Record<string, unknown>;
  onRefreshToolSettings: () => Promise<void>;
  onShowToast: (msg: string, kind?: ToastKind) => void;
}

export function ToolsPanel({
  isOpen,
  onClose,
  sessionId,
  toolSettings,
  onRefreshToolSettings,
  onShowToast,
}: ToolsPanelProps) {
  const [togglingTool, setTogglingTool] = useState<Set<string>>(new Set());
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [mcpServers, setMcpServers] = useState<McpServer[]>([]);
  const [mcpClientTools, setMcpClientTools] = useState<McpClientTool[]>([]);
  const [connectedServers, setConnectedServers] = useState<string[]>([]);
  const [serverOpen, setServerOpen] = useState<Set<string>>(new Set());
  const [isAutoConnecting, setIsAutoConnecting] = useState(false);
  const [builtinOpen, setBuiltinOpen] = useState(true);
  const [mcpOpen, setMcpOpen] = useState(true);

  const toolEnabledMap = (toolSettings.enabled_tools ?? {}) as Record<string, boolean>;
  const enabledMcpServers = mcpServers.filter((server) => server.enabled);

  useEffect(() => {
    if (isOpen && tools.length === 0) {
      void listBuiltinTools().then(setTools).catch(() => {
        /* ignore */
      });
    }
  }, [isOpen, tools.length]);

  const reloadMcpRuntimeState = useCallback(async () => {
    const [clientTools, connected] = await Promise.all([
      listMcpClientTools().catch(() => [] as McpClientTool[]),
      listConnectedMcpServers().catch(() => [] as string[]),
    ]);
    setMcpClientTools(clientTools);
    setConnectedServers(connected);
  }, []);

  useEffect(() => {
    if (!isOpen) return;

    void (async () => {
      const [servers, clientTools, connected] = await Promise.all([
        listMcpTools().catch(() => [] as McpServer[]),
        listMcpClientTools().catch(() => [] as McpClientTool[]),
        listConnectedMcpServers().catch(() => [] as string[]),
      ]);
      setMcpServers(servers);
      setMcpClientTools(clientTools);
      setConnectedServers(connected);

      const toConnect = servers.filter((server) => server.enabled && !connected.includes(server.name));
      if (toConnect.length === 0) return;

      setIsAutoConnecting(true);
      await Promise.allSettled(toConnect.map((server) => connectMcpServer(server.name)));
      await reloadMcpRuntimeState();
      setIsAutoConnecting(false);
    })();
  }, [isOpen, reloadMcpRuntimeState]);

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
    [onRefreshToolSettings, onShowToast, sessionId, togglingTool],
  );

  return (
    <>
      <div className={`session-panel-overlay${isOpen ? " visible" : ""}`} onClick={onClose} />
      <div className={`session-panel${isOpen ? " open" : ""}`}>
        <div className="session-panel-header">
          <div className="session-panel-title">
            <span className="session-panel-title-icon icon-tools" aria-hidden="true" />
            <div className="session-panel-title-copy">
              <h3>Tools</h3>
              <p className="session-panel-subtitle">Built-in tools and MCP server tools for this session.</p>
            </div>
          </div>
          <button className="session-panel-close" onClick={onClose} title="Close">
            <span className="icon-close" />
          </button>
        </div>

        <div className="session-panel-content">
          <div className="session-field">
            <button
              className="tools-section-toggle"
              onClick={() => setBuiltinOpen((open) => !open)}
              aria-expanded={builtinOpen}
            >
              <span>Built-in Tools</span>
              <span className={`tools-section-chevron${builtinOpen ? " open" : ""}`}>›</span>
            </button>
            {builtinOpen && (
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
                          {tool.summary && <span className="tool-summary">{tool.summary}</span>}
                        </div>
                      </label>
                    );
                  })
                )}
              </div>
            )}
          </div>

          <div className="session-divider" />

          <div className="session-field">
            <button
              className="tools-section-toggle"
              onClick={() => setMcpOpen((open) => !open)}
              aria-expanded={mcpOpen}
            >
              <span>
                MCP Tools
                {enabledMcpServers.length > 0 && (
                  <span className="tool-mcp-badge">{enabledMcpServers.length}</span>
                )}
                {isAutoConnecting && <span className="mcp-connecting-hint"> Connecting…</span>}
              </span>
              <span className={`tools-section-chevron${mcpOpen ? " open" : ""}`}>›</span>
            </button>
            {mcpOpen && (
              <div className="session-tools-list">
                {enabledMcpServers.length === 0 ? (
                  <div className="task-empty">No MCP servers configured. Add servers in the MCP panel.</div>
                ) : (
                  enabledMcpServers.map((server) => {
                    const isConnected = connectedServers.includes(server.name);
                    const serverTools = mcpClientTools.filter((tool) => tool.server === server.name);
                    const isServerOpen = serverOpen.has(server.name);

                    return (
                      <div key={server.name} className="mcp-server-entry">
                        <button
                          className={`mcp-server-toggle${isServerOpen ? " open" : ""}`}
                          onClick={() =>
                            setServerOpen((prev) => {
                              const next = new Set(prev);
                              if (next.has(server.name)) next.delete(server.name);
                              else next.add(server.name);
                              return next;
                            })
                          }
                          aria-expanded={isServerOpen}
                        >
                          <span className="mcp-server-name">{server.name}</span>
                          <span className={`tools-section-chevron${isServerOpen ? " open" : ""}`}>›</span>
                        </button>

                        {isServerOpen && (
                          <div className="mcp-server-tools-list">
                            {!isConnected ? (
                              isAutoConnecting ? (
                                <div className="task-empty" style={{ paddingLeft: "8px" }}>
                                  Connecting…
                                </div>
                              ) : (
                                <div className="task-empty" style={{ paddingLeft: "8px" }}>
                                  Could not connect to this server.
                                </div>
                              )
                            ) : serverTools.length === 0 ? (
                              <div className="task-empty" style={{ paddingLeft: "8px" }}>
                                No tools discovered from this server.
                              </div>
                            ) : (
                              serverTools.map((tool) => {
                                const defaultEnabled = server.session_default_enabled ?? true;
                                const isEnabled = toolEnabledMap[tool.qualified_name] ?? defaultEnabled;
                                return (
                                  <label key={tool.qualified_name} className="tool-checkbox">
                                    <input
                                      type="checkbox"
                                      checked={isEnabled}
                                      disabled={togglingTool.has(tool.qualified_name)}
                                      onChange={() => handleToggleTool(tool.qualified_name, isEnabled)}
                                    />
                                    <div className="tool-details">
                                      <span className="tool-name">{tool.name}</span>
                                      {tool.description && (
                                        <span className="tool-summary">{tool.description}</span>
                                      )}
                                    </div>
                                  </label>
                                );
                              })
                            )}
                          </div>
                        )}
                      </div>
                    );
                  })
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    </>
  );
}