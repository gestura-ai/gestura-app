import { invokeTauri } from './invoke';

export type TransportType = 'stdio' | 'http' | 'sse';
export type Scope = 'user' | 'project' | 'local';

export interface McpServer {
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
  /** Whether tools from this server are enabled by default for new sessions. */
  session_default_enabled?: boolean;
}

export interface ServerStatus {
  name: string;
  uri: string;
  state: string;
  tool_count: number;
  last_error: string | null;
}

export interface McpClientTool {
  server: string;
  name: string;
  qualified_name: string;
  description: string | null;
  input_schema: unknown;
}

export const listMcpTools = async (): Promise<McpServer[]> => {
  return await invokeTauri<McpServer[]>('list_mcp_tools');
};

export const getMcpServerStatus = async (): Promise<ServerStatus[]> => {
  return await invokeTauri<ServerStatus[]>('get_mcp_server_status');
};

export const listConnectedMcpServers = async (): Promise<string[]> => {
  return await invokeTauri<string[]>('list_connected_mcp_servers');
};

export const listMcpClientTools = async (): Promise<McpClientTool[]> => {
  return await invokeTauri<McpClientTool[]>('list_mcp_client_tools');
};

export const addMcpTool = async (tool: McpServer): Promise<void> => {
  await invokeTauri('add_mcp_tool', { tool });
};

export const removeMcpTool = async (name: string): Promise<void> => {
  await invokeTauri('remove_mcp_tool', { name });
};

export const connectMcpServer = async (name: string): Promise<string[]> => {
  return await invokeTauri<string[]>('connect_mcp_server', { name });
};

export const disconnectMcpServer = async (name: string): Promise<void> => {
  await invokeTauri('disconnect_mcp_server', { name });
};

export const callMcpTool = async (params: {
  server: string;
  tool: string;
  arguments: unknown;
}): Promise<unknown> => {
  return await invokeTauri<unknown>('call_mcp_tool', params);
};

// ── Provisioning ───────────────────────────────────────────────────────────────

/**
 * High-level outcome of a server provisioning attempt.
 *
 * - `ready`           – runtime present, package installed/pre-fetched.
 * - `runtime_missing` – required runtime (npx / uv) not found on PATH.
 * - `fetch_failed`    – runtime present but package download/install failed.
 * - `skipped`         – no local installation needed (HTTP/SSE server, or no command).
 */
export type ProvisionStatus = 'ready' | 'runtime_missing' | 'fetch_failed' | 'skipped';

/** Result returned by the backend after a provisioning attempt. */
export interface ProvisionResult {
  /** Server name, echoed back for UI correlation. */
  name: string;
  /** High-level outcome. */
  status: ProvisionStatus;
  /** Human-readable explanation suitable for display in the config panel. */
  message: string;
}

/**
 * Verify runtime availability and pre-install/fetch the package for a
 * newly-added stdio MCP server.  HTTP/SSE servers are skipped automatically.
 *
 * This command never rejects; all outcomes are expressed through `ProvisionStatus`.
 */
export const provisionMcpServer = async (server: McpServer): Promise<ProvisionResult> => {
  return await invokeTauri<ProvisionResult>('provision_mcp_server', { server });
};
