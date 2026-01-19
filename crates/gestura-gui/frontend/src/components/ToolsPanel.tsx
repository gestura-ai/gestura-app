import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface ToolInfo {
  name: string;
  summary: string;
  inputs: string[];
  side_effects: string[];
  examples: string[];
}

interface McpTool {
  name: string;
  endpoint: string;
  description?: string;
}

const ToolsPanel: React.FC = () => {
  const [builtinTools, setBuiltinTools] = useState<ToolInfo[]>([]);
  const [mcpTools, setMcpTools] = useState<McpTool[]>([]);
  const [selectedTool, setSelectedTool] = useState<ToolInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState('');

  useEffect(() => {
    loadTools();
  }, []);

  const loadTools = async () => {
    try {
      const [builtin, mcp] = await Promise.all([
        invoke<ToolInfo[]>('list_builtin_tools'),
        invoke<McpTool[]>('list_mcp_tools'),
      ]);
      setBuiltinTools(builtin);
      setMcpTools(mcp);
    } catch (error) {
      console.error('Failed to load tools:', error);
    } finally {
      setLoading(false);
    }
  };

  const filteredBuiltinTools = builtinTools.filter(
    (t) =>
      t.name.toLowerCase().includes(filter.toLowerCase()) ||
      t.summary.toLowerCase().includes(filter.toLowerCase())
  );

  const filteredMcpTools = mcpTools.filter(
    (t) =>
      t.name.toLowerCase().includes(filter.toLowerCase()) ||
      (t.description?.toLowerCase().includes(filter.toLowerCase()) ?? false)
  );

  if (loading) {
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
              <h3>MCP Tools ({filteredMcpTools.length})</h3>
              {filteredMcpTools.map((tool) => (
                <div key={tool.name} className="tool-item mcp">
                  <div className="tool-name">🔌 {tool.name}</div>
                  <div className="tool-summary">{tool.description || tool.endpoint}</div>
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

