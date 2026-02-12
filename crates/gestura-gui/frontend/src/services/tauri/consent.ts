import { invokeTauri } from './invoke';

export const registerConsent = async (params: {
  user_id: string;
  category: string;
  purpose: string;
}): Promise<void> => {
  await invokeTauri('register_consent', params);
};
