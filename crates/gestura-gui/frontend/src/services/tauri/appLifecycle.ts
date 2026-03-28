import { invokeTauri } from './invoke';

/** Returns whether the backend still considers the app to be in first-run state. */
export const isFirstRun = async (): Promise<boolean> => invokeTauri<boolean>('is_first_run');

/** Marks onboarding as complete using the backend-owned application lifecycle contract. */
export const completeOnboarding = async (): Promise<void> => {
  await invokeTauri<void>('complete_onboarding');
};