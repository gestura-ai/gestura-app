import { invokeTauri } from './invoke';

export const testVoice = async (): Promise<string> => {
  return await invokeTauri<string>('test_voice');
};

export const runVoiceOnce = async (): Promise<string> => {
  return await invokeTauri<string>('run_voice_once');
};
