import React, { useState } from 'react';
import { listMcpTools, McpServer } from '../../services/tauri/mcp';
import { listBuiltinTools, ToolInfo } from '../../services/tauri/tools';
import { useAsyncState } from '../../shared/hooks/useAsyncState';

const ToolsPanel: React.FC = () => {
  const [selectedTool, setSelectedTool] = useState<ToolInfo | null>(null);
  const [filter, setFilter] = useState('');

  const formatError = (e: unknown): string => {
    return e instanceof Error ? e.message : String(e);
  };

  const toolsState = useAsyncState(
    async () => {
      const [builtinRes, mcpRes] = await Promise.allSettled([listBuiltinTools(), listMcpTools()]);

      const builtin = builtinRes.status === 'fulfilled' ? builtinRes.value : [];
      const mcp = mcpRes.status === 'fulfilled' ? mcpRes.value : [];
      const warnings: string[] = [];

      if (builtinRes.status === 'rejected') warnings.push(`Built-in tools unavailable: ${String(builtinRes.reason)}`);
      if (mcpRes.status === 'rejected') warnings.push(`MCP tools unavailable: ${String(mcpRes.reason)}`);

      // Only hard-fail if *both* sources fail. Otherwise, show what we have.
      if (builtinRes.status === 'rejected' && mcpRes.status === 'rejected') {
        throw new Error(warnings.join(' | '));
      }

      return { builtin, mcp, warnings };
    },
    { errorMessage: 'Failed to load tools:' }
  );

  const builtinTools: ToolInfo[] = toolsState.data?.builtin ?? [];
  const mcpTools: McpServer[] = toolsState.data?.mcp ?? [];
  const warnings: string[] = toolsState.data?.warnings ?? [];

  const filteredBuiltinTools = builtinTools.filter(
    (t) => t.name.toLowerCase().includes(filter.toLowerCase()) || t.summary.toLowerCase().includes(filter.toLowerCase())
  );

  const filteredMcpTools = mcpTools.filter(
    (t) =>
      t.name.toLowerCase().includes(filter.toLowerCase()) ||
      (t.url?.toLowerCase().includes(filter.toLowerCase()) ?? false) ||
      (t.command?.toLowerCase().includes(filter.toLowerCase()) ?? false)
  );

  if (toolsState.loading) {
    return (
      <div className="tools-panel">
        <h2>Tools</h2>
        <p>Loading tools...</p>
      </div>
    );
  }

  return (
    <div className="tools-panel">
      <div className="tools-header">
        <h2>Tools</h2>
        <input
          type="text"
          className="tools-filter"
          placeholder="Filter tools..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>

      {toolsState.error != null && (
        <p className="tools-error">Failed to load tools: {formatError(toolsState.error)}</p>
      )}
      {warnings.length > 0 && (
        <p className="tools-warning">{warnings.join(' • ')}</p>
      )}

      <div className="tools-content">
        <div className="tools-list">
          <h3>Built-in Tools ({filteredBuiltinTools.length})</h3>
          {filteredBuiltinTools.map((tool) => (
            <div
              key={tool.name}
              className={`tool-item ${selectedTool?.name === tool.name ? 'selected' : ''}`}
              onClick={() => setSelectedTool(tool)}
            >
              <div className="tool-name">🔧 {tool.name}</div>
              <div className="tool-summary">{tool.summary}</div>
            </div>
          ))}

          {mcpTools.length > 0 && (
            <>
              <h3>MCP Servers ({filteredMcpTools.length})</h3>
              {filteredMcpTools.map((srv) => (
                <div key={srv.name} className={`tool-item mcp ${srv.enabled ? '' : 'disabled'}`}>
                  <div className="tool-name">
                    {srv.enabled ? '🔌' : '⏸️'} {srv.name}
                    <span className="tool-badge">{srv.type}</span>
                  </div>
                  <div className="tool-summary">
                    {srv.type === 'stdio'
                      ? `${srv.command ?? ''} ${(srv.args ?? []).join(' ')}`.trim()
                      : srv.url ?? '(no URL)'}
                  </div>
                </div>
              ))}
            </>
          )}
        </div>

        {selectedTool && (
          <div className="tool-detail">
            <h3>{selectedTool.name}</h3>
            <p className="tool-description">{selectedTool.summary}</p>

            {selectedTool.inputs.length > 0 && (
              <div className="tool-section">
                <h4>Inputs</h4>
                <ul>
                  {selectedTool.inputs.map((input, idx) => (
                    <li key={idx}>{input}</li>
                  ))}
                </ul>
              </div>
            )}

            {selectedTool.side_effects.length > 0 && (
              <div className="tool-section">
                <h4>Side Effects</h4>
                <ul className="side-effects">
                  {selectedTool.side_effects.map((effect, idx) => (
                    <li key={idx}>⚠️ {effect}</li>
                  ))}
                </ul>
              </div>
            )}

            {selectedTool.examples.length > 0 && (
              <div className="tool-section">
                <h4>Examples</h4>
                {selectedTool.examples.map((example, idx) => (
                  <pre key={idx} className="tool-example">
                    <code>{example}</code>
                  </pre>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

export default ToolsPanel;
