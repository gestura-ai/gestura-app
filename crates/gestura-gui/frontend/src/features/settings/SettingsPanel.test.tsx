import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import SettingsPanel from './SettingsPanel';
import type { AppConfig } from '../../types/config';

function makeConfig(): AppConfig {
  return {
    hotkey_listen: 'Ctrl+Shift+G',
    grace_period_secs: 30,
    voice: { provider: 'local' },
    llm: { primary: 'openai' },
    ui: { theme_mode: 'dark', accent: 'blue' },
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
      compaction_strategy: 'Summarize',
      max_context_tokens: 0,
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
  };
}

describe('SettingsPanel reflection controls', () => {
  it('updates nested agent telemetry settings without dropping sibling pipeline config', () => {
    const onConfigUpdate = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <SettingsPanel config={makeConfig()} onConfigUpdate={onConfigUpdate} />
    );

    const telemetryCheckbox = container.querySelector<HTMLInputElement>(
      'input[aria-label="Enable agent loop telemetry"]'
    );

    expect(telemetryCheckbox).not.toBeNull();
    fireEvent.click(telemetryCheckbox!);

    expect(onConfigUpdate).toHaveBeenCalledWith(
      expect.objectContaining({
        pipeline: expect.objectContaining({
          max_history_messages: 10,
          agent_telemetry: expect.objectContaining({ enabled: true }),
        }),
      })
    );
  });

  it('updates nested OTLP trace export settings without dropping sibling telemetry config', () => {
    const onConfigUpdate = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <SettingsPanel config={makeConfig()} onConfigUpdate={onConfigUpdate} />
    );

    const endpointInput = container.querySelector<HTMLInputElement>(
      'input[aria-label="OTLP Trace Endpoint"]'
    );

    expect(endpointInput).not.toBeNull();
    fireEvent.change(endpointInput!, {
      target: { value: 'http://localhost:4318/v1/traces' },
    });

    expect(onConfigUpdate).toHaveBeenCalledWith(
      expect.objectContaining({
        pipeline: expect.objectContaining({
          agent_telemetry: expect.objectContaining({
            enabled: false,
            trace_export: expect.objectContaining({
              protocol: 'grpc',
              endpoint: 'http://localhost:4318/v1/traces',
            }),
          }),
        }),
      })
    );
  });

  it('switches OTLP transport and updates the default endpoint accordingly', () => {
    const onConfigUpdate = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <SettingsPanel config={makeConfig()} onConfigUpdate={onConfigUpdate} />
    );

    const transportSelect = container.querySelector<HTMLSelectElement>(
      'select[aria-label="OTLP Trace Transport"]'
    );

    expect(transportSelect).not.toBeNull();
    fireEvent.change(transportSelect!, {
      target: { value: 'http' },
    });

    expect(onConfigUpdate).toHaveBeenCalledWith(
      expect.objectContaining({
        pipeline: expect.objectContaining({
          agent_telemetry: expect.objectContaining({
            trace_export: expect.objectContaining({
              protocol: 'http',
              endpoint: 'http://127.0.0.1:4318/v1/traces',
            }),
          }),
        }),
      })
    );
  });

  it('updates nested reflection settings without dropping sibling pipeline config', () => {
    const onConfigUpdate = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <SettingsPanel config={makeConfig()} onConfigUpdate={onConfigUpdate} />
    );

    const reflectionCheckbox = container.querySelector<HTMLInputElement>(
      'input[aria-label="Enable experiential reflection"]'
    );

    expect(reflectionCheckbox).not.toBeNull();
    fireEvent.click(reflectionCheckbox!);

    expect(onConfigUpdate).toHaveBeenCalledWith(
      expect.objectContaining({
        pipeline: expect.objectContaining({
          max_history_messages: 10,
          reflection: expect.objectContaining({ enabled: true }),
        }),
      })
    );
  });

  it('updates retry attempts and promotion confidence fields', () => {
    const onConfigUpdate = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <SettingsPanel config={makeConfig()} onConfigUpdate={onConfigUpdate} />
    );

    const retryAttemptsInput = container.querySelector<HTMLInputElement>(
      'input[aria-label="Reflection Retry Attempts"]'
    );
    const promotionConfidenceInput = container.querySelector<HTMLInputElement>(
      'input[aria-label="Reflection Promotion Confidence (%)"]'
    );

    expect(retryAttemptsInput).not.toBeNull();
    expect(promotionConfidenceInput).not.toBeNull();

    fireEvent.change(retryAttemptsInput!, {
      target: { value: '0' },
    });
    fireEvent.change(promotionConfidenceInput!, {
      target: { value: '88' },
    });

    expect(onConfigUpdate).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        pipeline: expect.objectContaining({
          reflection: expect.objectContaining({ max_retry_attempts: 0 }),
        }),
      })
    );
    expect(onConfigUpdate).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({
        pipeline: expect.objectContaining({
          reflection: expect.objectContaining({ promotion_confidence_percent: 88 }),
        }),
      })
    );
  });

  it('updates iteration budget settings without dropping sibling pipeline config', () => {
    const onConfigUpdate = vi.fn().mockResolvedValue(undefined);
    const { container } = render(
      <SettingsPanel config={makeConfig()} onConfigUpdate={onConfigUpdate} />
    );

    const budgetCheckbox = container.querySelector<HTMLInputElement>(
      'input[aria-label="Enable iteration budgets"]'
    );

    expect(budgetCheckbox).not.toBeNull();

    fireEvent.click(budgetCheckbox!);

    expect(onConfigUpdate).toHaveBeenCalledWith(
      expect.objectContaining({
        pipeline: expect.objectContaining({
          iteration_budget_enabled: true,
          max_history_messages: 10,
        }),
      })
    );
  });
});