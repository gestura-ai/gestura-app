import { invokeTauri } from './invoke';

export interface Agent {
  id: string;
  name: string;
  status: string;
}

export interface ListAgentsResponse {
  agents: Agent[];
  count: number;
}

export const listAgents = async (): Promise<ListAgentsResponse> => {
  return await invokeTauri<ListAgentsResponse>('list_agents');
};
