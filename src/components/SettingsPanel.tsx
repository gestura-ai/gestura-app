import React from 'react';
import { AppConfig, UiSettings } from '../types/config';

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
      ui: { ...config.ui, ...updates }
    });
  };

  const updateLlmSettings = (updates: any) => {
    updateConfig({
      llm: { ...config.llm, ...updates }
    });
  };

  return (
    <div>
      <h2>Settings</h2>
      
      <div className="panel">
        <h3>Appearance</h3>
        
        <div className="form-group">
          <label>Theme Mode</label>
          <select 
            value={config.ui.theme_mode} 
            onChange={(e) => updateUiSettings({ theme_mode: e.target.value })}
          >
            <option value="system">Follow System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>

        <div className="form-group">
          <label>Accent Color</label>
          <select 
            value={config.ui.accent || 'blue'} 
            onChange={(e) => updateUiSettings({ accent: e.target.value })}
          >
            <option value="blue">Blue</option>
            <option value="emerald">Emerald</option>
            <option value="amber">Amber</option>
            <option value="purple">Purple</option>
            <option value="rose">Rose</option>
          </select>
        </div>
      </div>

      <div className="panel">
        <h3>System</h3>
        
        <div className="form-group">
          <label>Global Hotkey</label>
          <input
            type="text"
            value={config.hotkey_listen}
            onChange={(e) => updateConfig({ hotkey_listen: e.target.value })}
            placeholder="Ctrl+Space"
          />
        </div>

        <div className="form-group">
          <label>Agent Grace Period (seconds)</label>
          <input
            type="number"
            value={config.grace_period_secs}
            onChange={(e) => updateConfig({ grace_period_secs: parseInt(e.target.value) || 30 })}
            min="1"
            max="300"
          />
        </div>
      </div>

      <div className="panel">
        <h3>AI/LLM</h3>
        
        <div className="form-group">
          <label>Primary Provider</label>
          <select 
            value={config.llm.primary} 
            onChange={(e) => updateLlmSettings({ primary: e.target.value })}
          >
            <option value="echo">Echo (Test)</option>
            <option value="openai">OpenAI</option>
            <option value="anthropic">Anthropic (Claude)</option>
            <option value="grok">Grok (xAI)</option>
            <option value="ollama">Ollama (Local)</option>
          </select>
        </div>
      </div>

      <div className="panel">
        <h3>Voice Engine</h3>
        
        <div className="form-group">
          <label>Provider</label>
          <select 
            value={config.voice.provider} 
            onChange={(e) => updateConfig({ 
              voice: { ...config.voice, provider: e.target.value }
            })}
          >
            <option value="local">Local Whisper</option>
            <option value="openai">OpenAI Whisper</option>
            <option value="none">Disabled</option>
          </select>
        </div>
      </div>
    </div>
  );
};

export default SettingsPanel;
