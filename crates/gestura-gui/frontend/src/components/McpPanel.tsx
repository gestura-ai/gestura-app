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
  const [editing, setEditing] = useState<McpServer | null>(null);
  const [isNew, setIsNew] = useState(false);
  const [loading, setLoading] = useState(true);
  const [envText, setEnvText] = useState('');
  const [headerText, setHeaderText] = useState('');

  const loadServers = useCallback(async () => {
    try {
      const [srvList, statusList] = await Promise.all([
        invoke<McpServer[]>('list_mcp_tools'),
        invoke<ServerStatus[]>('get_mcp_server_status').catch(() => []),
      ]);
      setServers(srvList);
      setStatuses(statusList);
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

  const getStatusFor = (name: string) => statuses.find(s => s.name === name);

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
            return (
              <div key={srv.name} className={`mcp-server-card ${srv.enabled ? '' : 'disabled'}`}>
                <div className="mcp-server-card-header">
                  <span className="mcp-server-name">{srv.name}</span>
                  <span className={`mcp-badge mcp-badge-${srv.type}`}>{srv.type}</span>
                  <span className={`mcp-badge mcp-badge-${srv.scope}`}>{srv.scope}</span>
                  {st && <span className={`mcp-status mcp-status-${st.state.toLowerCase()}`}>{st.state}</span>}
                </div>
                <div className="mcp-server-card-uri">
                  {srv.type === 'stdio'
                    ? `${srv.command || ''} ${(srv.args || []).join(' ')}`.trim() || '(no command)'
                    : srv.url || '(no url)'}
                </div>
                {st?.last_error && (
                  <div className="mcp-server-card-error">⚠ {st.last_error}</div>
                )}
                <div className="mcp-server-card-actions">
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
      </div>
    </div>
  );
};

export default McpPanel;
