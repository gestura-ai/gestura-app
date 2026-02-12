import type { AppConfig, UiSettings } from '../../types/config';

import { invokeTauri } from './invoke';

/**
 * Fetch the current application configuration from the Rust backend.
 */
export const getConfig = async (): Promise<AppConfig> => {
  return await invokeTauri<AppConfig>('get_config');
};

/**
 * Persist a full configuration update.
 *
 * IPC contract: `save_config` expects payload `{ cfg }`.
 */
export const saveConfig = async (cfg: AppConfig): Promise<void> => {
  await invokeTauri<void>('save_config', { cfg });
};

/**
 * Persist UI-only preference updates.
 *
 * IPC contract: `set_ui_prefs` expects payload `{ ui }`.
 */
export const setUiPrefs = async (ui: UiSettings): Promise<void> => {
  await invokeTauri<void>('set_ui_prefs', { ui });
};
