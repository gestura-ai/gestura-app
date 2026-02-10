import React from 'react';
import { AppConfig, LlmSettings, PipelineSettings, UiSettings } from '../../types/config';
import { FormGroup } from '../../shared/components/FormGroup';
import { PanelSection } from '../../shared/components/PanelSection';

interface SettingsPanelProps {
  config: AppConfig;
  onConfigUpdate: (config: AppConfig) => Promise<void>;
}

const SettingsPanel: React.FC<SettingsPanelProps> = ({ config, onConfigUpdate }) => {
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
            onChange={(e) => updatePipelineSettings({ compaction_strategy: e.target.value })}
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
      </PanelSection>
    </div>
  );
};

export default SettingsPanel;
