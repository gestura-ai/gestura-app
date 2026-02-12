import { invokeTauri } from './invoke';

export interface ToolInfo {
  name: string;
  summary: string;
  inputs: string[];
  side_effects: string[];
  examples: string[];
}

export const listBuiltinTools = async (): Promise<ToolInfo[]> => {
  return await invokeTauri<ToolInfo[]>('list_builtin_tools');
};
