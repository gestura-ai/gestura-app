import { invokeTauri } from './invoke';

export interface DelegatedTask {
  id: string;
  agent_id: string;
  description: string;
  priority: number;
  context: Record<string, unknown>;
}

export const listActiveTasks = async (): Promise<DelegatedTask[]> => {
  return await invokeTauri<DelegatedTask[]>('list_active_tasks');
};

export const delegateTask = async (task: DelegatedTask): Promise<void> => {
  await invokeTauri('delegate_task', { task });
};

export const cancelTask = async (taskId: string): Promise<void> => {
  await invokeTauri('cancel_task', { task_id: taskId });
};

export const spawnSubagent = async (agentId: string, name: string): Promise<void> => {
  await invokeTauri('spawn_subagent', { agent_id: agentId, name });
};
