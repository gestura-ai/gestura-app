import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AppConfig, VoiceSettings } from '../types/config';

interface VoicePanelProps {
  config: AppConfig;
  onConfigUpdate: (config: AppConfig) => Promise<void>;
}

const VoicePanel: React.FC<VoicePanelProps> = ({ config, onConfigUpdate }) => {
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<string>('');

  const handleVoiceTest = async () => {
    setTesting(true);
    setTestResult('');
    
    try {
      const result = await invoke<string>('test_voice');
      setTestResult(result);
    } catch (error) {
      setTestResult(`Error: ${error}`);
    } finally {
      setTesting(false);
    }
  };

  const handleVoiceRun = async () => {
    setTesting(true);
    setTestResult('');
    
    try {
      const result = await invoke<string>('run_voice_once');
      setTestResult(`Transcription: ${result}`);
    } catch (error) {
      setTestResult(`Error: ${error}`);
    } finally {
      setTesting(false);
    }
  };

  const updateVoiceConfig = (updates: Partial<VoiceSettings>) => {
    const newConfig = {
      ...config,
      voice: { ...config.voice, ...updates }
    };
    onConfigUpdate(newConfig);
  };

  return (
    <div>
      <h2>Voice Processing</h2>
      
      <div className="panel">
        <h3>Configuration</h3>
        
        <div className="form-group">
          <label>Provider</label>
          <select 
            value={config.voice.provider} 
            onChange={(e) => updateVoiceConfig({ provider: e.target.value })}
          >
            <option value="local">Local (Whisper)</option>
            <option value="openai">OpenAI Whisper</option>
            <option value="none">Disabled</option>
          </select>
        </div>

        {config.voice.provider === 'local' && (
          <div className="form-group">
            <label>Model Path</label>
            <input
              type="text"
              value={config.voice.local_model_path || ''}
              onChange={(e) => updateVoiceConfig({ local_model_path: e.target.value })}
              placeholder="/path/to/whisper/model.bin"
            />
          </div>
        )}

        {config.voice.provider === 'openai' && (
          <div className="form-group">
            <label>API Key</label>
            <input
              type="password"
              value={config.voice.openai_api_key || ''}
              onChange={(e) => updateVoiceConfig({ openai_api_key: e.target.value })}
              placeholder="sk-..."
            />
          </div>
        )}

        <div className="form-group">
          <label>Test Input Path (WAV file)</label>
          <input
            type="text"
            value={config.voice.input_path || ''}
            onChange={(e) => updateVoiceConfig({ input_path: e.target.value })}
            placeholder="/path/to/test.wav"
          />
        </div>
      </div>

      <div className="panel">
        <h3>Testing</h3>
        
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1rem' }}>
          <button 
            className="btn" 
            onClick={handleVoiceTest}
            disabled={testing}
          >
            {testing ? 'Testing...' : 'Test Engine'}
          </button>
          
          <button 
            className="btn" 
            onClick={handleVoiceRun}
            disabled={testing || !config.voice.input_path}
          >
            {testing ? 'Processing...' : 'Run Transcription'}
          </button>
        </div>

        {testResult && (
          <div className="panel" style={{ background: 'var(--bg)', fontFamily: 'monospace', fontSize: '0.875rem' }}>
            <pre>{testResult}</pre>
          </div>
        )}
      </div>
    </div>
  );
};

export default VoicePanel;
