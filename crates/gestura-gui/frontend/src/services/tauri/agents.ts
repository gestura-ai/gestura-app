import { invokeTauri } from './invoke';

export interface Agent {
  id: string;
  name: string;
  status: string;
  role?: string;
  capabilities?: string[];
  workspace_dir?: string | null;
  execution_mode?: string;
}

export interface ListAgentsResponse {
  agents: Agent[];
  count: number;
}

export const listAgents = async (): Promise<ListAgentsResponse> => {
  return await invokeTauri<ListAgentsResponse>('list_agents');
};
