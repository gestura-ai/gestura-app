import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AppConfig } from './types/config';

const getConfigMock = vi.fn<[], Promise<AppConfig>>();
const isFirstRunMock = vi.fn<[], Promise<boolean>>();

vi.mock('./app/ThemeController', () => ({
  default: () => null,
}));

vi.mock('./app/StatusBar', () => ({
  default: () => <div data-testid="status-bar" />,
}));

vi.mock('./app/OnboardingWizard', () => ({
  default: () => <div data-testid="onboarding" />,
}));

vi.mock('./shared/components/HelpSystem', () => ({
  default: () => null,
}));

vi.mock('./shared/hooks/useKeyboardShortcuts', () => ({
  useKeyboardShortcuts: () => undefined,
}));

vi.mock('./services/tauri/appLifecycle', () => ({
  isFirstRun: () => isFirstRunMock(),
  completeOnboarding: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('./services/tauri/config', () => ({
  getConfig: () => getConfigMock(),
  saveConfig: vi.fn().mockResolvedValue(undefined),
  setUiPrefs: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('./features/voice/VoicePanel', () => ({
  default: () => <h2>Voice Processing</h2>,
}));

vi.mock('./features/ring/RingPanel', () => ({
  default: () => <h2>Haptic Harmony Ring</h2>,
}));

vi.mock('./features/tools/ToolsPanel', () => ({
  default: () => <h2>Tools</h2>,
}));

vi.mock('./features/workflows/WorkflowsPanel', () => ({
  default: () => <h2>Workflows</h2>,
}));

vi.mock('./features/mcp/McpPanel', () => ({
  default: () => <h2>MCP</h2>,
}));

vi.mock('./features/memory/components/MemoryConsolePanel', () => ({
  default: () => <h2>Memory Console</h2>,
}));

vi.mock('./features/simulator/SimulatorPanel', () => ({
  default: () => <h2>Simulator</h2>,
}));

vi.mock('./features/settings/SettingsPanel', () => ({
  default: () => <h2>Settings</h2>,
}));

import App from './App';

function makeConfig(): AppConfig {
  return {
    hotkey_listen: 'Ctrl+Shift+G',
    grace_period_secs: 30,
    voice: { provider: 'local' },
    llm: { primary: 'openai' },
    ui: { theme_mode: 'dark', accent: 'blue' },
    privacy: {
      data_collection: false,
      crash_reports: true,
      voice_data_local: true,
      require_auth: false,
      auth_timeout: 15,
    },
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
      auto_compact_threshold_percent: 80,
      compaction_strategy: 'MemoryBank',
      max_context_tokens: 0,
      log_token_usage: true,
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
        quality_threshold_percent: 70,
        max_injected: 3,
        max_retry_attempts: 1,
        promotion_confidence_percent: 80,
      },
    },
  };
}

describe('App', () => {
  it('boots to the generic voice panel and does not expose hello-world UI', async () => {
    isFirstRunMock.mockResolvedValue(false);
    getConfigMock.mockResolvedValue(makeConfig());

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Voice Processing' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /voice/i })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /hello/i })).not.toBeInTheDocument();
  });

  it('shows onboarding when the backend reports first-run state', async () => {
    isFirstRunMock.mockResolvedValue(true);
    getConfigMock.mockResolvedValue(makeConfig());

    render(<App />);

    expect(await screen.findByTestId('onboarding')).toBeInTheDocument();
  });
});