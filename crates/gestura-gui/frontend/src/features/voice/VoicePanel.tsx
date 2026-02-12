import React, { useState } from 'react';
import { AppConfig, VoiceSettings } from '../../types/config';
import { runVoiceOnce, testVoice } from '../../services/tauri/voice';
import { Button } from '../../shared/components/Button';
import { FormGroup } from '../../shared/components/FormGroup';
import { PanelSection } from '../../shared/components/PanelSection';

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
      const result = await testVoice();
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
      const result = await runVoiceOnce();
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
      voice: { ...config.voice, ...updates },
    };
    onConfigUpdate(newConfig);
  };

  return (
    <div>
      <h2>Voice Processing</h2>

      <PanelSection heading="Configuration">
        <FormGroup label="Provider">
          <select value={config.voice.provider} onChange={(e) => updateVoiceConfig({ provider: e.target.value })}>
            <option value="local">Local (Whisper)</option>
            <option value="openai">OpenAI Whisper</option>
            <option value="none">Disabled</option>
          </select>
        </FormGroup>

        {config.voice.provider === 'local' && (
          <FormGroup label="Model Path">
            <input
              type="text"
              value={config.voice.local_model_path || ''}
              onChange={(e) => updateVoiceConfig({ local_model_path: e.target.value })}
              placeholder="/path/to/whisper/model.bin"
            />
          </FormGroup>
        )}

        {config.voice.provider === 'openai' && (
          <FormGroup label="API Key">
            <input
              type="password"
              value={config.voice.openai_api_key || ''}
              onChange={(e) => updateVoiceConfig({ openai_api_key: e.target.value })}
              placeholder="sk-..."
            />
          </FormGroup>
        )}

        <FormGroup label="Test Input Path (WAV file)">
          <input
            type="text"
            value={config.voice.input_path || ''}
            onChange={(e) => updateVoiceConfig({ input_path: e.target.value })}
            placeholder="/path/to/test.wav"
          />
        </FormGroup>
      </PanelSection>

      <PanelSection heading="Testing">
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1rem' }}>
          <Button onClick={handleVoiceTest} disabled={testing}>
            {testing ? 'Testing...' : 'Test Engine'}
          </Button>

          <Button onClick={handleVoiceRun} disabled={testing || !config.voice.input_path}>
            {testing ? 'Processing...' : 'Run Transcription'}
          </Button>
        </div>

        {testResult && (
          <PanelSection style={{ background: 'var(--bg)', fontFamily: 'monospace', fontSize: '0.875rem' }}>
            <pre>{testResult}</pre>
          </PanelSection>
        )}
      </PanelSection>
    </div>
  );
};

export default VoicePanel;
