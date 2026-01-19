import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './App.css';
import ThemeController from './components/ThemeController';
import ChatPanel from './components/ChatPanel';
import ToolsPanel from './components/ToolsPanel';
import WorkflowsPanel from './components/WorkflowsPanel';
import VoicePanel from './components/VoicePanel';
import SettingsPanel from './components/SettingsPanel';
import RingPanel from './components/RingPanel';
import StatusBar from './components/StatusBar';
import OnboardingWizard from './components/OnboardingWizard';
import HelpSystem from './components/HelpSystem';
import SimulatorPanel from './components/SimulatorPanel';
import { AppConfig, UiSettings } from './types/config';



function App() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [activePanel, setActivePanel] = useState('chat'); // Default to chat
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  const [loading, setLoading] = useState(true);
  const [tokenUsage, setTokenUsage] = useState({ total: 0, session: 0 });

  useEffect(() => {
    // Check onboarding BEFORE loading config to preserve first-run state
    const onboardingCompleted = localStorage.getItem('gestura_onboarding_completed');
    console.log('🚀 Gestura App starting...');
    console.log('📋 Onboarding completed flag:', onboardingCompleted);

    if (!onboardingCompleted) {
      console.log('✅ Showing onboarding wizard (first run or incomplete)');
      setShowOnboarding(true);
    } else {
      console.log('⏭️ Skipping onboarding (already completed)');
    }

    // Load config after checking onboarding
    loadConfig();

    // Add keyboard shortcuts
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'F1') {
        event.preventDefault();
        setShowHelp(!showHelp);
      } else if (event.key === 'Escape' && showHelp) {
        setShowHelp(false);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [showHelp]);

  const loadConfig = async () => {
    try {
      const cfg = await invoke<AppConfig>('get_config');
      setConfig(cfg);
    } catch (error) {
      console.error('Failed to load config:', error);
    } finally {
      setLoading(false);
    }
  };

  const saveConfig = async (newConfig: AppConfig) => {
    try {
      await invoke('save_config', { cfg: newConfig });
      setConfig(newConfig);
    } catch (error) {
      console.error('Failed to save config:', error);
    }
  };

  const updateUiSettings = async (ui: UiSettings) => {
    try {
      await invoke('set_ui_prefs', { ui });
      if (config) {
        setConfig({ ...config, ui });
      }
    } catch (error) {
      console.error('Failed to update UI settings:', error);
    }
  };

  if (loading) {
    return (
      <div className="app">
        <div className="header">
          <h1>Gestura</h1>
        </div>
        <div className="main">
          <div className="content">
            <p>Loading...</p>
          </div>
        </div>
      </div>
    );
  }

  if (!config) {
    return (
      <div className="app">
        <div className="header">
          <h1>Gestura</h1>
        </div>
        <div className="main">
          <div className="content">
            <p>Failed to load configuration</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="app">
      <ThemeController
        uiSettings={config.ui}
        onUpdate={updateUiSettings}
      />

      <div className="header">
        <div className="header-left">
          <h1>Gestura</h1>
        </div>
        <div className="header-right">
          <button
            className="btn btn-secondary btn-small"
            onClick={() => setShowHelp(true)}
            title="Help (F1)"
          >
            ?
          </button>
          <button
            className="btn btn-secondary btn-small"
            onClick={() => setShowOnboarding(true)}
            title="Show Onboarding"
          >
            🚀
          </button>
          <StatusBar />
        </div>
      </div>

      <div className="main">
        <div className="sidebar">
          <nav>
            <button
              className={`btn ${activePanel === 'chat' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('chat')}
            >
              💬 Chat
            </button>
            <button
              className={`btn ${activePanel === 'voice' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('voice')}
            >
              🎤 Voice
            </button>
            <button
              className={`btn ${activePanel === 'ring' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('ring')}
            >
              💍 Ring
            </button>
            <button
              className={`btn ${activePanel === 'tools' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('tools')}
            >
              🔧 Tools
            </button>
            <button
              className={`btn ${activePanel === 'workflows' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('workflows')}
            >
              📋 Workflows
            </button>
            <button
              className={`btn ${activePanel === 'simulator' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('simulator')}
            >
              🧪 Simulator
            </button>
            <button
              className={`btn ${activePanel === 'settings' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('settings')}
            >
              ⚙️ Settings
            </button>
          </nav>
          {tokenUsage.session > 0 && (
            <div className="token-usage">
              <small>Tokens: {tokenUsage.session.toLocaleString()}</small>
            </div>
          )}
        </div>

        <div className="content">
          {activePanel === 'chat' && (
            <ChatPanel
              onTokenUsage={(usage) => setTokenUsage(prev => ({
                total: prev.total + usage.input_tokens + usage.output_tokens,
                session: prev.session + usage.input_tokens + usage.output_tokens
              }))}
            />
          )}
          {activePanel === 'tools' && (
            <ToolsPanel />
          )}
          {activePanel === 'workflows' && (
            <WorkflowsPanel />
          )}
          {activePanel === 'voice' && (
            <VoicePanel config={config} onConfigUpdate={saveConfig} />
          )}
          {activePanel === 'ring' && (
            <RingPanel />
          )}
          {activePanel === 'simulator' && (
            <SimulatorPanel onClose={() => setActivePanel('ring')} />
          )}
          {activePanel === 'settings' && (
            <SettingsPanel config={config} onConfigUpdate={saveConfig} />
          )}
        </div>
      </div>

      {/* Onboarding Wizard */}
      {showOnboarding && (
        <div className="modal-overlay">
          <OnboardingWizard onComplete={() => setShowOnboarding(false)} />
        </div>
      )}

      {/* Help System */}
      <HelpSystem isOpen={showHelp} onClose={() => setShowHelp(false)} />
    </div>
  );
}

export default App;
