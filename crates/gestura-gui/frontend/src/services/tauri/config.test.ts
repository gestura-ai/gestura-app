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
    iteration_budget_enabled: false,
    max_iterations: 10,
    tracked_task_max_iterations: 30,
    auto_compact_threshold_percent: 75,
    compaction_strategy: 'Summarize',
    max_context_tokens: 4096,
    log_token_usage: false,
    agent_telemetry: {
      enabled: false,
      trace_export: {
        enabled: false,
        protocol: 'grpc',
        endpoint: 'http://127.0.0.1:4317',
      },
    },
    reflection: {
      enabled: false,
      quality_threshold_percent: 60,
      max_injected: 3,
      max_retry_attempts: 1,
      promotion_confidence_percent: 75,
    },
  },
});

describe('config IPC wrappers', () => {
  it('getConfig invokes get_config', async () => {
    const mock = vi.mocked(invokeTauri);
    const cfg = makeConfig();
    mock.mockResolvedValueOnce(cfg);

    await expect(getConfig()).resolves.toMatchObject({
      ...cfg,
      ui: { accent: 'blue', ...cfg.ui },
    });
    // getConfig intentionally passes no args object.
    expect(mock).toHaveBeenCalledWith('get_config');
  });

  it('getConfig fills in omitted default sections from Rust IPC', async () => {
    const mock = vi.mocked(invokeTauri);
    mock.mockResolvedValueOnce({
      hotkey_listen: 'Ctrl+Shift+G',
      grace_period_secs: 1,
      voice: { provider: 'local' },
      llm: { primary: 'openai' },
      mcp_servers: [],
      mdh_pointers: {},
      nats_url: 'nats://127.0.0.1:4222',
    });

    await expect(getConfig()).resolves.toMatchObject({
      ui: { theme_mode: 'system', accent: 'blue' },
      developer: {
        developer_mode: false,
        enable_simulators: false,
      },
      pipeline: {
        agent_telemetry: {
          enabled: false,
          trace_export: {
            enabled: false,
            protocol: 'grpc',
            endpoint: 'http://127.0.0.1:4317',
          },
        },
        reflection: {
          enabled: false,
        },
      },
    });
  });

  it('saveConfig invokes save_config with { cfg }', async () => {
    const mock = vi.mocked(invokeTauri);
    const cfg = makeConfig();
    mock.mockResolvedValueOnce(undefined);

    await saveConfig(cfg);
    expect(mock).toHaveBeenCalledWith('save_config', { cfg });
  });

  it('saveConfig normalizes legacy compaction strategies before IPC', async () => {
    const mock = vi.mocked(invokeTauri);
    const cfg = makeConfig();
    cfg.pipeline.compaction_strategy = 'message_count' as never;
    mock.mockResolvedValueOnce(undefined);

    await saveConfig(cfg);
    expect(mock).toHaveBeenCalledWith('save_config', {
      cfg: {
        ...cfg,
        pipeline: {
          ...cfg.pipeline,
          compaction_strategy: 'Summarize',
        },
      },
    });
  });

  it('getConfig normalizes invalid compaction strategies from Rust IPC payloads', async () => {
    const mock = vi.mocked(invokeTauri);
    const cfg = makeConfig();
    mock.mockResolvedValueOnce({
      ...cfg,
      pipeline: {
        ...cfg.pipeline,
        compaction_strategy: 'message_count',
      },
    });

    await expect(getConfig()).resolves.toMatchObject({
      pipeline: {
        compaction_strategy: 'Summarize',
      },
    });
  });

  it('setUiPrefs invokes set_ui_prefs with { ui }', async () => {
    const mock = vi.mocked(invokeTauri);
    const ui: UiSettings = { theme_mode: 'light', accent: '#ffffff' };
    mock.mockResolvedValueOnce(undefined);

    await setUiPrefs(ui);
    expect(mock).toHaveBeenCalledWith('set_ui_prefs', { ui });
  });
});
