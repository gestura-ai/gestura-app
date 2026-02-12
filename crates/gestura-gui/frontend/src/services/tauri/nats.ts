import { invokeTauri } from './invoke';

export const getNatsStatus = async (): Promise<boolean> => {
  return await invokeTauri<boolean>('get_nats_status');
};
