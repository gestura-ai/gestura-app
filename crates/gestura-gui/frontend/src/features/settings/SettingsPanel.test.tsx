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
      auto_compact_threshold_percent: 80,
      compaction_strategy: 'Summarize',
      max_context_tokens: 0,
      log_token_usage: false,
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
  it('updates nested reflection settings without dropping sibling pipeline config', () => {
    const onConfigUpdate = vi.fn().mockResolvedValue(undefined);
    const { getByLabelText } = render(
      <SettingsPanel config={makeConfig()} onConfigUpdate={onConfigUpdate} />
    );

    fireEvent.click(getByLabelText(/Enable experiential reflection/i));

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
});