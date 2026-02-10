import { describe, expect, it, vi } from 'vitest';

import type { AppConfig, UiSettings } from '../../types/config';

vi.mock('./invoke', () => ({
  invokeTauri: vi.fn(),
}));

import { invokeTauri } from './invoke';
import { getConfig, saveConfig, setUiPrefs } from './config';

const makeConfig = (): AppConfig => ({
  hotkey_listen: 'Ctrl+Shift+G',
  grace_period_secs: 1,
  voice: { provider: 'local' },
  llm: { primary: 'openai' },
  ui: { theme_mode: 'dark' },
  mcp_servers: [],
  mdh_pointers: {},
  nats_url: 'nats://127.0.0.1:4222',
  developer: {
    developer_mode: false,
    enable_simulators: false,
    auto_discover_simulators: false,
    verbose_ble_logging: false,
    simulator: {
      device_name_pattern: 'Gestura*',
      auto_connect: false,
      health_check_interval: 1,
      enable_metrics: false,
      discovery_port_range: [10000, 10010],
    },
  },
  pipeline: {
    max_history_messages: 10,
    auto_compact_threshold_percent: 75,
    compaction_strategy: 'none',
    max_context_tokens: 4096,
    log_token_usage: false,
  },
});

describe('config IPC wrappers', () => {
  it('getConfig invokes get_config', async () => {
    const mock = vi.mocked(invokeTauri);
    const cfg = makeConfig();
    mock.mockResolvedValueOnce(cfg);

    await expect(getConfig()).resolves.toEqual(cfg);
    // getConfig intentionally passes no args object.
    expect(mock).toHaveBeenCalledWith('get_config');
  });

  it('saveConfig invokes save_config with { cfg }', async () => {
    const mock = vi.mocked(invokeTauri);
    const cfg = makeConfig();
    mock.mockResolvedValueOnce(undefined);

    await saveConfig(cfg);
    expect(mock).toHaveBeenCalledWith('save_config', { cfg });
  });

  it('setUiPrefs invokes set_ui_prefs with { ui }', async () => {
    const mock = vi.mocked(invokeTauri);
    const ui: UiSettings = { theme_mode: 'light', accent: '#ffffff' };
    mock.mockResolvedValueOnce(undefined);

    await setUiPrefs(ui);
    expect(mock).toHaveBeenCalledWith('set_ui_prefs', { ui });
  });
});
