import React, { useState } from 'react';
import {
  addMcpTool,
  callMcpTool,
  connectMcpServer,
  disconnectMcpServer,
  getMcpServerStatus,
  listConnectedMcpServers,
  listMcpClientTools,
  listMcpTools,
  McpClientTool,
  McpServer,
  removeMcpTool,
  Scope,
  ServerStatus,
  TransportType,
} from '../../services/tauri/mcp';
import { useAsyncState } from '../../shared/hooks/useAsyncState';
import { Button } from '../../shared/components/Button';
import { FormGroup } from '../../shared/components/FormGroup';

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
  const [connecting, setConnecting] = useState<Record<string, boolean>>({});
  const [editing, setEditing] = useState<McpServer | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [envText, setEnvText] = useState('');
  const [headerText, setHeaderText] = useState('');
  const [toolCallServer, setToolCallServer] = useState('');
  const [toolCallName, setToolCallName] = useState('');
  const [toolCallArgs, setToolCallArgs] = useState('{}');
  const [toolCallResult, setToolCallResult] = useState<string | null>(null);

  const mcpState = useAsyncState(
    async () => {
      const [srvList, statusList, connected, tools] = await Promise.all([
        listMcpTools(),
        getMcpServerStatus().catch(() => []),
        listConnectedMcpServers().catch(() => []),
        listMcpClientTools().catch(() => []),
      ]);
      return { servers: srvList, statuses: statusList, connectedServers: connected, clientTools: tools };
    },
    { errorMessage: 'Failed to load MCP servers:' }
  );

  const servers: McpServer[] = mcpState.data?.servers ?? [];
  const statuses: ServerStatus[] = mcpState.data?.statuses ?? [];
  const connectedServers: string[] = mcpState.data?.connectedServers ?? [];
  const clientTools: McpClientTool[] = mcpState.data?.clientTools ?? [];

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
      await addMcpTool(entry);
      setEditing(null);
      await mcpState.reload({ showLoading: false });
    } catch (e) {
      console.error('Failed to save MCP server:', e);
    }
  };

  const removeServer = async (name: string) => {
    try {
      await removeMcpTool(name);
      if (editing?.name === name) setEditing(null);
      await mcpState.reload({ showLoading: false });
    } catch (e) {
      console.error('Failed to remove MCP server:', e);
    }
  };

  const toggleEnabled = async (srv: McpServer) => {
    try {
      await addMcpTool({ ...srv, enabled: !srv.enabled });
      await mcpState.reload({ showLoading: false });
    } catch (e) {
      console.error('Failed to toggle server:', e);
    }
  };

  const connectServer = async (name: string) => {
    setConnecting(prev => ({ ...prev, [name]: true }));
    try {
      await connectMcpServer(name);
      await mcpState.reload({ showLoading: false });
    } catch (e) {
      console.error(`Failed to connect to ${name}:`, e);
    } finally {
      setConnecting(prev => ({ ...prev, [name]: false }));
    }
  };

  const disconnectServer = async (name: string) => {
    try {
      await disconnectMcpServer(name);
      await mcpState.reload({ showLoading: false });
    } catch (e) {
      console.error(`Failed to disconnect from ${name}:`, e);
    }
  };

  const callTool = async () => {
    if (!toolCallServer || !toolCallName) return;
    setToolCallResult(null);
    try {
      const args = JSON.parse(toolCallArgs);
      const result = await callMcpTool({ server: toolCallServer, tool: toolCallName, arguments: args });
      setToolCallResult(JSON.stringify(result, null, 2));
    } catch (e) {
      setToolCallResult(`Error: ${e}`);
    }
  };

  const isConnected = (name: string) => connectedServers.includes(name);
  const getStatusFor = (name: string) => statuses.find(s => s.name === name);
  const toolsForServer = (name: string) => clientTools.filter(t => t.server === name);

  if (mcpState.loading) return <div className="mcp-panel"><h2>MCP Servers</h2><p>Loading…</p></div>;

  return (
    <div className="mcp-panel">
      <div className="mcp-header">
        <h2>MCP Servers</h2>
        <Button onClick={openNewForm}>+ Add Server</Button>
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
                    <Button tone="primary" size="small" disabled={isConnecting} onClick={() => connectServer(srv.name)}>
                      {isConnecting ? 'Connecting…' : 'Connect'}
                    </Button>
                  )}
                  {connected && (
                    <Button size="small" onClick={() => disconnectServer(srv.name)}>
                      Disconnect
                    </Button>
                  )}
                  <Button size="small" onClick={() => openEditForm(srv)}>Edit</Button>
                  <Button tone="secondary" size="small" onClick={() => toggleEnabled(srv)}>
                    {srv.enabled ? 'Disable' : 'Enable'}
                  </Button>
                  <Button tone="danger" size="small" onClick={() => removeServer(srv.name)}>Remove</Button>
                </div>
              </div>
            );
          })}
        </div>

        {/* Edit / New form */}
        {editing && (
          <div className="mcp-form">
            <h3>{isNew ? 'Add MCP Server' : `Edit: ${editing.name}`}</h3>

            <FormGroup label="Name">
              <input
                type="text"
                value={editing.name}
                disabled={!isNew}
                onChange={e => setEditing({ ...editing, name: e.target.value })}
                placeholder="e.g. github, postgres"
              />
            </FormGroup>

            <FormGroup label="Transport">
              <select
                value={editing.type}
                onChange={e => setEditing({ ...editing, type: e.target.value as TransportType })}
              >
                <option value="stdio">stdio (local process)</option>
                <option value="http">http (Streamable HTTP)</option>
                <option value="sse">sse (Server-Sent Events)</option>
              </select>
            </FormGroup>

            {editing.type === 'stdio' && (
              <>
                <FormGroup label="Command">
                  <input
                    type="text"
                    value={editing.command || ''}
                    onChange={e => setEditing({ ...editing, command: e.target.value })}
                    placeholder="e.g. npx, uvx, docker"
                  />
                </FormGroup>

                <FormGroup label="Arguments (one per line)">
                  <textarea
                    value={(editing.args || []).join('\n')}
                    onChange={e => setEditing({ ...editing, args: e.target.value.split('\n').filter(Boolean) })}
                    rows={3}
                    placeholder="-y&#10;@modelcontextprotocol/server-github"
                  />
                </FormGroup>

                <FormGroup label="Environment variables (KEY=VALUE per line)">
                  <textarea
                    value={envText}
                    onChange={e => setEnvText(e.target.value)}
                    rows={3}
                    placeholder="GITHUB_TOKEN=ghp_xxx"
                  />
                </FormGroup>
              </>
            )}

            {(editing.type === 'http' || editing.type === 'sse') && (
              <>
                <FormGroup label="URL">
                  <input
                    type="text"
                    value={editing.url || ''}
                    onChange={e => setEditing({ ...editing, url: e.target.value })}
                    placeholder="https://api.example.com/mcp/"
                  />
                </FormGroup>

                <FormGroup label="Headers (Key: Value per line)">
                  <textarea
                    value={headerText}
                    onChange={e => setHeaderText(e.target.value)}
                    rows={3}
                    placeholder="Authorization: Bearer sk-xxx"
                  />
                </FormGroup>
              </>
            )}

            <FormGroup label="Scope">
              <select
                value={editing.scope}
                onChange={e => setEditing({ ...editing, scope: e.target.value as Scope })}
              >
                <option value="user">user (global)</option>
                <option value="project">project (.mcp.json)</option>
                <option value="local">local (override)</option>
              </select>
            </FormGroup>

            <div className="form-row">
              <FormGroup label="Timeout (seconds)">
                <input
                  type="number"
                  value={editing.timeout_secs}
                  onChange={e => setEditing({ ...editing, timeout_secs: parseInt(e.target.value) || 30 })}
                  min={1}
                  max={300}
                />
              </FormGroup>
              <FormGroup
                label={
                  <>
                    <input
                      type="checkbox"
                      checked={editing.auto_reconnect}
                      onChange={e => setEditing({ ...editing, auto_reconnect: e.target.checked })}
                    />{' '}
                    Auto-reconnect
                  </>
                }
              >
                {null}
              </FormGroup>
            </div>

            <div className="mcp-form-actions">
              <Button onClick={saveServer}>
                {isNew ? 'Add Server' : 'Save Changes'}
              </Button>
              <Button tone="secondary" onClick={() => setEditing(null)}>Cancel</Button>
            </div>
          </div>
        )}

        {/* Tool Invocation Panel — only shown when at least one server is connected */}
        {connectedServers.length > 0 && (
          <div className="mcp-tool-invoke">
            <h3>Invoke MCP Tool</h3>
            <FormGroup label="Server">
              <select value={toolCallServer} onChange={e => { setToolCallServer(e.target.value); setToolCallName(''); }}>
                <option value="">Select a server…</option>
                {connectedServers.map(s => <option key={s} value={s}>{s}</option>)}
              </select>
            </FormGroup>
            {toolCallServer && (
              <FormGroup label="Tool">
                <select value={toolCallName} onChange={e => setToolCallName(e.target.value)}>
                  <option value="">Select a tool…</option>
                  {toolsForServer(toolCallServer).map(t => (
                    <option key={t.qualified_name} value={t.name}>{t.name}</option>
                  ))}
                </select>
              </FormGroup>
            )}
            <FormGroup label="Arguments (JSON)">
              <textarea
                value={toolCallArgs}
                onChange={e => setToolCallArgs(e.target.value)}
                rows={4}
                placeholder='{"key": "value"}'
              />
            </FormGroup>
            <Button onClick={callTool} disabled={!toolCallServer || !toolCallName}>
              Call Tool
            </Button>
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
