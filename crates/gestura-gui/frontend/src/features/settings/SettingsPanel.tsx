import React from 'react';
import {
  AgentTelemetrySettings,
  AppConfig,
  CompactionStrategy,
  LlmSettings,
  PipelineSettings,
  ReflectionSettings,
  UiSettings,
} from '../../types/config';
import { FormGroup } from '../../shared/components/FormGroup';
import { PanelSection } from '../../shared/components/PanelSection';

interface SettingsPanelProps {
  config: AppConfig;
  onConfigUpdate: (config: AppConfig) => Promise<void>;
}

const SettingsPanel: React.FC<SettingsPanelProps> = ({ config, onConfigUpdate }) => {
  const parseIntegerOr = (value: string, fallback: number) => {
    const parsed = Number.parseInt(value, 10);
    return Number.isNaN(parsed) ? fallback : parsed;
  };

  const updateConfig = (updates: Partial<AppConfig>) => {
    onConfigUpdate({ ...config, ...updates });
  };

  const updateUiSettings = (updates: Partial<UiSettings>) => {
    updateConfig({
      ui: { ...config.ui, ...updates },
    });
  };

  const updateLlmSettings = (updates: Partial<LlmSettings>) => {
    updateConfig({
      llm: { ...config.llm, ...updates },
    });
  };

  const updatePipelineSettings = (updates: Partial<PipelineSettings>) => {
    updateConfig({
      pipeline: { ...config.pipeline, ...updates },
    });
  };

  const updateReflectionSettings = (updates: Partial<ReflectionSettings>) => {
    updatePipelineSettings({
      reflection: { ...config.pipeline.reflection, ...updates },
    });
  };

  // Nested pipeline updates must preserve sibling settings so toggling telemetry
  // does not clobber reflection/iteration/compaction configuration.
  const updateAgentTelemetrySettings = (updates: Partial<AgentTelemetrySettings>) => {
    updatePipelineSettings({
      agent_telemetry: { ...config.pipeline.agent_telemetry, ...updates },
    });
  };

  return (
    <div>
      <h2>Settings</h2>

      <PanelSection heading="Appearance">
        <FormGroup label="Theme Mode">
          <select value={config.ui.theme_mode} onChange={(e) => updateUiSettings({ theme_mode: e.target.value })}>
            <option value="system">Follow System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </FormGroup>

        <FormGroup label="Accent Color">
          <select value={config.ui.accent || 'blue'} onChange={(e) => updateUiSettings({ accent: e.target.value })}>
            <option value="blue">Blue</option>
            <option value="emerald">Emerald</option>
            <option value="amber">Amber</option>
            <option value="purple">Purple</option>
            <option value="rose">Rose</option>
          </select>
        </FormGroup>
      </PanelSection>

      <PanelSection heading="System">
        <FormGroup label="Global Hotkey">
          <input
            type="text"
            value={config.hotkey_listen}
            onChange={(e) => updateConfig({ hotkey_listen: e.target.value })}
            placeholder="Ctrl+Space"
          />
        </FormGroup>

        <FormGroup label="Agent Grace Period (seconds)">
          <input
            type="number"
            value={config.grace_period_secs}
            onChange={(e) => updateConfig({ grace_period_secs: parseInt(e.target.value) || 30 })}
            min="1"
            max="300"
          />
        </FormGroup>
      </PanelSection>

      <PanelSection heading="AI/LLM">
        <FormGroup label="Primary Provider">
          <select value={config.llm.primary} onChange={(e) => updateLlmSettings({ primary: e.target.value })}>
            <option value="openai">OpenAI</option>
            <option value="anthropic">Anthropic (Claude)</option>
            <option value="gemini">Gemini (Google)</option>
            <option value="grok">Grok (xAI)</option>
            <option value="ollama">Ollama (Local)</option>
          </select>
        </FormGroup>
      </PanelSection>

      <PanelSection heading="Voice Engine">
        <FormGroup label="Provider">
          <select
            value={config.voice.provider}
            onChange={(e) =>
              updateConfig({
                voice: { ...config.voice, provider: e.target.value },
              })
            }
          >
            <option value="local">Local Whisper</option>
            <option value="openai">OpenAI Whisper</option>
            <option value="none">Disabled</option>
          </select>
        </FormGroup>
      </PanelSection>

      <PanelSection heading="Context Management">
        <FormGroup
          label="Max History Messages"
          hint="Maximum conversation history messages to include in prompt (1-100)"
        >
          <input
            type="number"
            value={config.pipeline.max_history_messages}
            onChange={(e) => updatePipelineSettings({ max_history_messages: parseInt(e.target.value) || 10 })}
            min="1"
            max="100"
          />
        </FormGroup>

        <FormGroup
          label={
            <>
              <input
                type="checkbox"
                aria-label="Enable iteration budgets"
                checked={config.pipeline.iteration_budget_enabled}
                onChange={(e) => updatePipelineSettings({ iteration_budget_enabled: e.target.checked })}
              />{' '}
              Enable iteration budgets
            </>
          }
          hint="When disabled, agent loops are unbounded and stop only when they naturally finish or are cancelled."
        >
          {null}
        </FormGroup>

        <FormGroup
          label="Max Iterations (General Requests)"
          hint="Applied to non-task-bound requests when iteration budgets are enabled."
        >
          <input
            type="number"
            value={config.pipeline.max_iterations}
            onChange={(e) => updatePipelineSettings({ max_iterations: parseIntegerOr(e.target.value, 10) })}
            min="1"
            max="500"
            disabled={!config.pipeline.iteration_budget_enabled}
          />
        </FormGroup>

        <FormGroup
          label="Max Iterations (Tracked Task Requests)"
          hint="Applied to implementation / tracked-task requests when iteration budgets are enabled."
        >
          <input
            type="number"
            value={config.pipeline.tracked_task_max_iterations}
            onChange={(e) => updatePipelineSettings({ tracked_task_max_iterations: parseIntegerOr(e.target.value, 30) })}
            min="1"
            max="1000"
            disabled={!config.pipeline.iteration_budget_enabled}
          />
        </FormGroup>

        <FormGroup
          label="Auto-Compact Threshold (%)"
          hint="Trigger auto-compaction when context reaches this percentage of limit (0-100%)"
        >
          <input
            type="number"
            value={config.pipeline.auto_compact_threshold_percent}
            onChange={(e) => updatePipelineSettings({ auto_compact_threshold_percent: parseInt(e.target.value) || 80 })}
            min="0"
            max="100"
          />
        </FormGroup>

        <FormGroup
          label="Compaction Strategy"
          hint="How to handle context overflow when auto-compaction is triggered"
        >
          <select
            value={config.pipeline.compaction_strategy}
            onChange={(e) => updatePipelineSettings({ compaction_strategy: e.target.value as CompactionStrategy })}
          >
            <option value="Summarize">Summarize - Condense older messages</option>
            <option value="Truncate">Truncate - Remove oldest messages</option>
            <option value="Clear">Clear - Drop all history</option>
            <option value="Prompt">Prompt - Ask user what to do</option>
            <option value="MemoryBank">Memory Bank - Save to persistent files</option>
          </select>
        </FormGroup>

        <FormGroup
          label="Max Context Tokens"
          hint="Maximum context window tokens (0 = use provider defaults)"
        >
          <input
            type="number"
            value={config.pipeline.max_context_tokens}
            onChange={(e) => updatePipelineSettings({ max_context_tokens: parseInt(e.target.value) || 0 })}
            min="0"
            max="200000"
          />
        </FormGroup>

        <FormGroup
          label={
            <>
              <input
                type="checkbox"
                checked={config.pipeline.log_token_usage}
                onChange={(e) => updatePipelineSettings({ log_token_usage: e.target.checked })}
              />{' '}
              Enable token usage logging
            </>
          }
          hint="Log token usage for debugging and monitoring"
        >
          {null}
        </FormGroup>

        <FormGroup
          label={
            <>
              <input
                type="checkbox"
                aria-label="Enable agent loop telemetry"
                checked={config.pipeline.agent_telemetry.enabled}
                onChange={(e) => updateAgentTelemetrySettings({ enabled: e.target.checked })}
              />{' '}
              Enable agent loop telemetry
            </>
          }
          hint="Capture request telemetry across routing, context resolution, compaction, agent iterations, tool calls, and reflection."
        >
          {null}
        </FormGroup>

        <FormGroup
          label={
            <>
              <input
                type="checkbox"
                aria-label="Enable experiential reflection"
                checked={config.pipeline.reflection.enabled}
                onChange={(e) => updateReflectionSettings({ enabled: e.target.checked })}
              />{' '}
              Enable experiential reflection
            </>
          }
          hint="Generate structured reflections after low-quality turns and reuse them in future context."
        >
          {null}
        </FormGroup>

        <FormGroup
          label="Reflection Quality Threshold (%)"
          hint="Trigger reflection when the response quality score falls below this percentage. Lower values make reflection rarer."
        >
          <input
            type="number"
            aria-label="Reflection Quality Threshold (%)"
            value={config.pipeline.reflection.quality_threshold_percent}
            onChange={(e) =>
              updateReflectionSettings({
                quality_threshold_percent: parseIntegerOr(e.target.value, 60),
              })
            }
            min="0"
            max="100"
          />
        </FormGroup>

        <FormGroup
          label="Past Reflections to Inject"
          hint="Maximum number of relevant past reflections to inject back into prompt context for future turns."
        >
          <input
            type="number"
            aria-label="Past Reflections to Inject"
            value={config.pipeline.reflection.max_injected}
            onChange={(e) =>
              updateReflectionSettings({
                max_injected: parseIntegerOr(e.target.value, 3),
              })
            }
            min="0"
            max="20"
          />
        </FormGroup>

        <FormGroup
          label="Reflection Retry Attempts"
          hint="Number of same-turn text-only revisions after reflection. Current runtime behavior applies at most one retry and never replays tools."
        >
          <input
            type="number"
            aria-label="Reflection Retry Attempts"
            value={config.pipeline.reflection.max_retry_attempts}
            onChange={(e) =>
              updateReflectionSettings({
                max_retry_attempts: parseIntegerOr(e.target.value, 0),
              })
            }
            min="0"
            max="1"
          />
        </FormGroup>

        <FormGroup
          label="Reflection Promotion Confidence (%)"
          hint="Minimum confidence needed before a reflection is promoted into long-term shared memory."
        >
          <input
            type="number"
            aria-label="Reflection Promotion Confidence (%)"
            value={config.pipeline.reflection.promotion_confidence_percent}
            onChange={(e) =>
              updateReflectionSettings({
                promotion_confidence_percent: parseIntegerOr(e.target.value, 75),
              })
            }
            min="0"
            max="100"
          />
        </FormGroup>
      </PanelSection>
    </div>
  );
};

export default SettingsPanel;
