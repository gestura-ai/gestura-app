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
