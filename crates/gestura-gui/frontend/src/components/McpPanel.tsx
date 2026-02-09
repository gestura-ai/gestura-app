import React, { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';

type TransportType = 'stdio' | 'http' | 'sse';
type Scope = 'user' | 'project' | 'local';

interface McpServer {
  name: string;
  type: TransportType;
  enabled: boolean;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  headers?: Record<string, string>;
  scope: Scope;
  timeout_secs: number;
  auto_reconnect: boolean;
}

interface ServerStatus {
  name: string;
  uri: string;
  state: string;
  tool_count: number;
  last_error: string | null;
}

interface McpClientTool {
  server: string;
  name: string;
  qualified_name: string;
  description: string | null;
  input_schema: unknown;
}

const DEFAULT_SERVER: McpServer = {
  name: '',
  type: 'stdio',
  enabled: true,
  command: '',
  args: [],
  env: {},
  url: '',
  headers: {},
  scope: 'user',
  timeout_secs: 30,
  auto_reconnect: true,
};

const McpPanel: React.FC = () => {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [statuses, setStatuses] = useState<ServerStatus[]>([]);
  const [connectedServers, setConnectedServers] = useState<string[]>([]);
  const [clientTools, setClientTools] = useState<McpClientTool[]>([]);
  const [connecting, setConnecting] = useState<Record<string, boolean>>({});
  const [editing, setEditing] = useState<McpServer | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [loading, setLoading] = useState(true);
  const [envText, setEnvText] = useState('');
  const [headerText, setHeaderText] = useState('');
  const [toolCallServer, setToolCallServer] = useState('');
  const [toolCallName, setToolCallName] = useState('');
  const [toolCallArgs, setToolCallArgs] = useState('{}');
  const [toolCallResult, setToolCallResult] = useState<string | null>(null);

  const loadServers = useCallback(async () => {
    try {
      const [srvList, statusList, connected, tools] = await Promise.all([
        invoke<McpServer[]>('list_mcp_tools'),
        invoke<ServerStatus[]>('get_mcp_server_status').catch(() => []),
        invoke<string[]>('list_connected_mcp_servers').catch(() => []),
        invoke<McpClientTool[]>('list_mcp_client_tools').catch(() => []),
      ]);
      setServers(srvList);
      setStatuses(statusList);
      setConnectedServers(connected);
      setClientTools(tools);
    } catch (e) {
      console.error('Failed to load MCP servers:', e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { loadServers(); }, [loadServers]);

  const openNewForm = () => {
    setEditing({ ...DEFAULT_SERVER });
    setEnvText('');
    setHeaderText('');
    setIsNew(true);
  };

  const openEditForm = (srv: McpServer) => {
    setEditing({ ...srv });
    setEnvText(Object.entries(srv.env || {}).map(([k, v]) => `${k}=${v}`).join('\n'));
    setHeaderText(Object.entries(srv.headers || {}).map(([k, v]) => `${k}: ${v}`).join('\n'));
    setIsNew(false);
  };

  const saveServer = async () => {
    if (!editing || !editing.name.trim()) return;
    const entry: McpServer = {
      ...editing,
      env: Object.fromEntries(
        envText.split('\n').filter(l => l.includes('=')).map(l => {
          const [k, ...rest] = l.split('=');
          return [k.trim(), rest.join('=').trim()];
        })
      ),
      headers: Object.fromEntries(
        headerText.split('\n').filter(l => l.includes(':')).map(l => {
          const [k, ...rest] = l.split(':');
          return [k.trim(), rest.join(':').trim()];
        })
      ),
    };
    try {
      await invoke('add_mcp_tool', { tool: entry });
      setEditing(null);
      await loadServers();
    } catch (e) {
      console.error('Failed to save MCP server:', e);
    }
  };

  const removeServer = async (name: string) => {
    try {
      await invoke('remove_mcp_tool', { name });
      if (editing?.name === name) setEditing(null);
      await loadServers();
    } catch (e) {
      console.error('Failed to remove MCP server:', e);
    }
  };

  const toggleEnabled = async (srv: McpServer) => {
    try {
      await invoke('add_mcp_tool', { tool: { ...srv, enabled: !srv.enabled } });
      await loadServers();
    } catch (e) {
      console.error('Failed to toggle server:', e);
    }
  };

  const connectServer = async (name: string) => {
    setConnecting(prev => ({ ...prev, [name]: true }));
    try {
      await invoke<string[]>('connect_mcp_server', { name });
      await loadServers();
    } catch (e) {
      console.error(`Failed to connect to ${name}:`, e);
    } finally {
      setConnecting(prev => ({ ...prev, [name]: false }));
    }
  };

  const disconnectServer = async (name: string) => {
    try {
      await invoke('disconnect_mcp_server', { name });
      await loadServers();
    } catch (e) {
      console.error(`Failed to disconnect from ${name}:`, e);
    }
  };

  const callTool = async () => {
    if (!toolCallServer || !toolCallName) return;
    setToolCallResult(null);
    try {
      const args = JSON.parse(toolCallArgs);
      const result = await invoke<unknown>('call_mcp_tool', {
        server: toolCallServer,
        tool: toolCallName,
        arguments: args,
      });
      setToolCallResult(JSON.stringify(result, null, 2));
    } catch (e) {
      setToolCallResult(`Error: ${e}`);
    }
  };

  const isConnected = (name: string) => connectedServers.includes(name);
  const getStatusFor = (name: string) => statuses.find(s => s.name === name);
  const toolsForServer = (name: string) => clientTools.filter(t => t.server === name);

  if (loading) return <div className="mcp-panel"><h2>MCP Servers</h2><p>Loading…</p></div>;

  return (
    <div className="mcp-panel">
      <div className="mcp-header">
        <h2>MCP Servers</h2>
        <button className="btn" onClick={openNewForm}>+ Add Server</button>
      </div>

      <div className="mcp-content">
        {/* Server list */}
        <div className="mcp-server-list">
          {servers.length === 0 && (
            <p className="mcp-empty">No MCP servers configured. Click "Add Server" to get started.</p>
          )}
          {servers.map(srv => {
            const st = getStatusFor(srv.name);
            const connected = isConnected(srv.name);
            const srvTools = toolsForServer(srv.name);
            const isConnecting = connecting[srv.name] || false;
            return (
              <div key={srv.name} className={`mcp-server-card ${srv.enabled ? '' : 'disabled'}`}>
                <div className="mcp-server-card-header">
                  <span className="mcp-server-name">{srv.name}</span>
                  <span className={`mcp-badge mcp-badge-${srv.type}`}>{srv.type}</span>
                  <span className={`mcp-badge mcp-badge-${srv.scope}`}>{srv.scope}</span>
                  <span className={`mcp-status mcp-status-${connected ? 'connected' : 'disconnected'}`}>
                    {isConnecting ? '⏳ Connecting…' : connected ? '● Connected' : '○ Disconnected'}
                  </span>
                  {st && !connected && <span className={`mcp-status mcp-status-${st.state.toLowerCase()}`}>{st.state}</span>}
                </div>
                <div className="mcp-server-card-uri">
                  {srv.type === 'stdio'
                    ? `${srv.command || ''} ${(srv.args || []).join(' ')}`.trim() || '(no command)'
                    : srv.url || '(no url)'}
                </div>
                {st?.last_error && (
                  <div className="mcp-server-card-error">⚠ {st.last_error}</div>
                )}
                {connected && srvTools.length > 0 && (
                  <div className="mcp-server-tools">
                    <strong>Discovered Tools ({srvTools.length}):</strong>
                    <ul>
                      {srvTools.map(t => (
                        <li key={t.qualified_name}>
                          <code>{t.name}</code>
                          {t.description && <span className="tool-desc"> — {t.description}</span>}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
                <div className="mcp-server-card-actions">
                  {srv.enabled && !connected && (
                    <button className="btn btn-sm btn-primary" disabled={isConnecting} onClick={() => connectServer(srv.name)}>
                      {isConnecting ? 'Connecting…' : 'Connect'}
                    </button>
                  )}
                  {connected && (
                    <button className="btn btn-sm btn-warning" onClick={() => disconnectServer(srv.name)}>
                      Disconnect
                    </button>
                  )}
                  <button className="btn btn-sm" onClick={() => openEditForm(srv)}>Edit</button>
                  <button className="btn btn-sm btn-secondary" onClick={() => toggleEnabled(srv)}>
                    {srv.enabled ? 'Disable' : 'Enable'}
                  </button>
                  <button className="btn btn-sm btn-danger" onClick={() => removeServer(srv.name)}>Remove</button>
                </div>
              </div>
            );
          })}
        </div>

        {/* Edit / New form */}
        {editing && (
          <div className="mcp-form">
            <h3>{isNew ? 'Add MCP Server' : `Edit: ${editing.name}`}</h3>

            <div className="form-group">
              <label>Name</label>
              <input
                type="text"
                value={editing.name}
                disabled={!isNew}
                onChange={e => setEditing({ ...editing, name: e.target.value })}
                placeholder="e.g. github, postgres"
              />
            </div>

            <div className="form-group">
              <label>Transport</label>
              <select
                value={editing.type}
                onChange={e => setEditing({ ...editing, type: e.target.value as TransportType })}
              >
                <option value="stdio">stdio (local process)</option>
                <option value="http">http (Streamable HTTP)</option>
                <option value="sse">sse (Server-Sent Events)</option>
              </select>
            </div>

            {editing.type === 'stdio' && (
              <>
                <div className="form-group">
                  <label>Command</label>
                  <input
                    type="text"
                    value={editing.command || ''}
                    onChange={e => setEditing({ ...editing, command: e.target.value })}
                    placeholder="e.g. npx, uvx, docker"
                  />
                </div>
                <div className="form-group">
                  <label>Arguments (one per line)</label>
                  <textarea
                    value={(editing.args || []).join('\n')}
                    onChange={e => setEditing({ ...editing, args: e.target.value.split('\n').filter(Boolean) })}
                    rows={3}
                    placeholder="-y&#10;@modelcontextprotocol/server-github"
                  />
                </div>
                <div className="form-group">
                  <label>Environment variables (KEY=VALUE per line)</label>
                  <textarea
                    value={envText}
                    onChange={e => setEnvText(e.target.value)}
                    rows={3}
                    placeholder="GITHUB_TOKEN=ghp_xxx"
                  />
                </div>
              </>
            )}

            {(editing.type === 'http' || editing.type === 'sse') && (
              <>
                <div className="form-group">
                  <label>URL</label>
                  <input
                    type="text"
                    value={editing.url || ''}
                    onChange={e => setEditing({ ...editing, url: e.target.value })}
                    placeholder="https://api.example.com/mcp/"
                  />
                </div>
                <div className="form-group">
                  <label>Headers (Key: Value per line)</label>
                  <textarea
                    value={headerText}
                    onChange={e => setHeaderText(e.target.value)}
                    rows={3}
                    placeholder="Authorization: Bearer sk-xxx"
                  />
                </div>
              </>
            )}

            <div className="form-group">
              <label>Scope</label>
              <select
                value={editing.scope}
                onChange={e => setEditing({ ...editing, scope: e.target.value as Scope })}
              >
                <option value="user">user (global)</option>
                <option value="project">project (.mcp.json)</option>
                <option value="local">local (override)</option>
              </select>
            </div>

            <div className="form-row">
              <div className="form-group">
                <label>Timeout (seconds)</label>
                <input
                  type="number"
                  value={editing.timeout_secs}
                  onChange={e => setEditing({ ...editing, timeout_secs: parseInt(e.target.value) || 30 })}
                  min={1}
                  max={300}
                />
              </div>
              <div className="form-group">
                <label>
                  <input
                    type="checkbox"
                    checked={editing.auto_reconnect}
                    onChange={e => setEditing({ ...editing, auto_reconnect: e.target.checked })}
                  />
                  {' '}Auto-reconnect
                </label>
              </div>
            </div>

            <div className="mcp-form-actions">
              <button className="btn" onClick={saveServer}>
                {isNew ? 'Add Server' : 'Save Changes'}
              </button>
              <button className="btn btn-secondary" onClick={() => setEditing(null)}>Cancel</button>
            </div>
          </div>
        )}

        {/* Tool Invocation Panel — only shown when at least one server is connected */}
        {connectedServers.length > 0 && (
          <div className="mcp-tool-invoke">
            <h3>Invoke MCP Tool</h3>
            <div className="form-group">
              <label>Server</label>
              <select value={toolCallServer} onChange={e => { setToolCallServer(e.target.value); setToolCallName(''); }}>
                <option value="">Select a server…</option>
                {connectedServers.map(s => <option key={s} value={s}>{s}</option>)}
              </select>
            </div>
            {toolCallServer && (
              <div className="form-group">
                <label>Tool</label>
                <select value={toolCallName} onChange={e => setToolCallName(e.target.value)}>
                  <option value="">Select a tool…</option>
                  {toolsForServer(toolCallServer).map(t => (
                    <option key={t.qualified_name} value={t.name}>{t.name}</option>
                  ))}
                </select>
              </div>
            )}
            <div className="form-group">
              <label>Arguments (JSON)</label>
              <textarea
                value={toolCallArgs}
                onChange={e => setToolCallArgs(e.target.value)}
                rows={4}
                placeholder='{"key": "value"}'
              />
            </div>
            <button className="btn" onClick={callTool} disabled={!toolCallServer || !toolCallName}>
              Call Tool
            </button>
            {toolCallResult !== null && (
              <pre className="mcp-tool-result">{toolCallResult}</pre>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

export default McpPanel;
