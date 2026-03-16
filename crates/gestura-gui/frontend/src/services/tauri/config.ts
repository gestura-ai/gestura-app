import type { AppConfig, CompactionStrategy, UiSettings } from '../../types/config';

import { invokeTauri } from './invoke';

type PartialConfig = Partial<AppConfig> & Record<string, unknown>;

const normalizeCompactionStrategy = (value: unknown): CompactionStrategy => {
  const raw = typeof value === 'string' ? value.trim() : '';
  const normalized = raw.toLowerCase().replace(/[-_\s]+/g, '');

  switch (normalized) {
    case 'summarize':
      return 'Summarize';
    case 'truncate':
      return 'Truncate';
    case 'clear':
      return 'Clear';
    case 'prompt':
      return 'Prompt';
    case 'memorybank':
      return 'MemoryBank';
    case 'messagecount':
    case 'none':
    default:
      return 'Summarize';
  }
};

const sanitizeConfigForRust = (cfg: AppConfig): AppConfig => ({
  ...cfg,
  pipeline: {
    ...cfg.pipeline,
    compaction_strategy: normalizeCompactionStrategy(cfg.pipeline.compaction_strategy),
  },
});

const defaultConfig = (): AppConfig => ({
  hotkey_listen: 'Ctrl+Space',
  grace_period_secs: 30,
  voice: {
    provider: 'local',
  },
  llm: {
    primary: 'anthropic',
  },
  ui: {
    theme_mode: 'system',
    accent: 'blue',
  },
  mcp_servers: [],
  mdh_pointers: {},
  nats_url: 'nats://127.0.0.1:4223',
  developer: {
    developer_mode: false,
    enable_simulators: false,
    auto_discover_simulators: false,
    verbose_ble_logging: false,
    simulator: {
      device_name_pattern: 'Gestura*',
      auto_connect: false,
      health_check_interval: 30,
      enable_metrics: false,
      discovery_port_range: [10000, 10010],
    },
  },
  pipeline: {
    max_history_messages: 10,
    iteration_budget_enabled: false,
    max_iterations: 10,
    tracked_task_max_iterations: 30,
    auto_compact_threshold_percent: 80,
    compaction_strategy: 'Summarize',
    max_context_tokens: 0,
    log_token_usage: true,
    // Keep request tracing opt-in so existing installs do not suddenly emit a
    // much richer metric stream until the user explicitly enables it.
    agent_telemetry: {
      enabled: false,
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

const normalizeConfig = (raw: PartialConfig): AppConfig => {
  const defaults = defaultConfig();

  return {
    ...defaults,
    ...raw,
    voice: {
      ...defaults.voice,
      ...(raw.voice ?? {}),
    },
    llm: {
      ...defaults.llm,
      ...(raw.llm ?? {}),
    },
    ui: {
      ...defaults.ui,
      ...(raw.ui ?? {}),
    },
    developer: {
      ...defaults.developer,
      ...(raw.developer ?? {}),
      simulator: {
        ...defaults.developer.simulator,
        ...((raw.developer as AppConfig['developer'] | undefined)?.simulator ?? {}),
      },
    },
    pipeline: {
      ...defaults.pipeline,
      ...(raw.pipeline ?? {}),
      compaction_strategy: normalizeCompactionStrategy(
        (raw.pipeline as AppConfig['pipeline'] | undefined)?.compaction_strategy,
      ),
      // Merge nested telemetry defaults explicitly so older configs that predate
      // the setting still deserialize into a fully-shaped object for the UI.
      agent_telemetry: {
        ...defaults.pipeline.agent_telemetry,
        ...((raw.pipeline as AppConfig['pipeline'] | undefined)?.agent_telemetry ?? {}),
      },
      reflection: {
        ...defaults.pipeline.reflection,
        ...((raw.pipeline as AppConfig['pipeline'] | undefined)?.reflection ?? {}),
      },
    },
    mcp_servers: raw.mcp_servers ?? defaults.mcp_servers,
    mdh_pointers: raw.mdh_pointers ?? defaults.mdh_pointers,
  };
};

/**
 * Fetch the current application configuration from the Rust backend.
 */
export const getConfig = async (): Promise<AppConfig> => {
  const raw = await invokeTauri<PartialConfig>('get_config');
  return normalizeConfig(raw);
};

/**
 * Persist a full configuration update.
 *
 * IPC contract: `save_config` expects payload `{ cfg }`.
 */
export const saveConfig = async (cfg: AppConfig): Promise<void> => {
  await invokeTauri<void>('save_config', { cfg: sanitizeConfigForRust(cfg) });
};

/**
 * Persist UI-only preference updates.
 *
 * IPC contract: `set_ui_prefs` expects payload `{ ui }`.
 */
export const setUiPrefs = async (ui: UiSettings): Promise<void> => {
  await invokeTauri<void>('set_ui_prefs', { ui });
};
