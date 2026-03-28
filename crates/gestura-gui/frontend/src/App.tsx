import { useCallback, useEffect, useState } from 'react';
import './App.css';
import ThemeController from './app/ThemeController';
import ToolsPanel from './features/tools/ToolsPanel';
import WorkflowsPanel from './features/workflows/WorkflowsPanel';
import VoicePanel from './features/voice/VoicePanel';
import SettingsPanel from './features/settings/SettingsPanel';
import RingPanel from './features/ring/RingPanel';
import StatusBar from './app/StatusBar';
import OnboardingWizard from './app/OnboardingWizard';
import HelpSystem from './shared/components/HelpSystem';
import SimulatorPanel from './features/simulator/SimulatorPanel';
import McpPanel from './features/mcp/McpPanel';
import MemoryConsolePanel from './features/memory/components/MemoryConsolePanel';
import { AppConfig, UiSettings } from './types/config';
import { getConfig, saveConfig, setUiPrefs } from './services/tauri/config';
import { isFirstRun } from './services/tauri/appLifecycle';
import { useKeyboardShortcuts } from './shared/hooks/useKeyboardShortcuts';



function App() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [activePanel, setActivePanel] = useState('voice');
  const [showOnboarding, setShowOnboarding] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  const [loading, setLoading] = useState(true);

  const loadAppState = useCallback(async () => {
    try {
      const [firstRun, cfg] = await Promise.all([isFirstRun(), getConfig()]);
      setShowOnboarding(firstRun);
      setConfig(cfg);
    } catch (error) {
      console.error('Failed to load app state:', error);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadAppState();
  }, [loadAppState]);

  useKeyboardShortcuts((event) => {
    if (event.key === 'F1') {
      event.preventDefault();
      setShowHelp((prev) => !prev);
      return;
    }

    if (event.key === 'Escape') {
      // Preserve legacy semantics: only close if already open.
      setShowHelp((prev) => (prev ? false : prev));
    }
  });

  const handleSaveConfig = async (newConfig: AppConfig) => {
    try {
      await saveConfig(newConfig);
      setConfig(newConfig);
    } catch (error) {
      console.error('Failed to save config:', error);
    }
  };

  const handleUpdateUiSettings = async (ui: UiSettings) => {
    try {
      await setUiPrefs(ui);
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
          <h1 className="text-gradient">Gestura</h1>
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
          <h1 className="text-gradient">Gestura</h1>
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
        onUpdate={handleUpdateUiSettings}
      />

      <div className="header">
        <div className="header-left">
          <h1 className="text-gradient">Gestura</h1>
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
              className={`btn ${activePanel === 'mcp' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('mcp')}
            >
              🔌 MCP
            </button>
            <button
              className={`btn ${activePanel === 'memory' ? '' : 'btn-secondary'}`}
              onClick={() => setActivePanel('memory')}
            >
              🧠 Memory
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
        </div>

        <div className="content">
          {activePanel === 'tools' && (
            <ToolsPanel />
          )}
          {activePanel === 'workflows' && (
            <WorkflowsPanel />
          )}
          {activePanel === 'voice' && (
            <VoicePanel config={config} onConfigUpdate={handleSaveConfig} />
          )}
          {activePanel === 'ring' && (
            <RingPanel />
          )}
          {activePanel === 'mcp' && (
            <McpPanel />
          )}
          {activePanel === 'memory' && (
            <MemoryConsolePanel allowSessionSelection title="Memory Console" />
          )}
          {activePanel === 'simulator' && (
            <SimulatorPanel onClose={() => setActivePanel('ring')} />
          )}
          {activePanel === 'settings' && (
            <SettingsPanel config={config} onConfigUpdate={handleSaveConfig} />
          )}
        </div>
      </div>

      {/* Onboarding Wizard */}
      {showOnboarding && (
        <div className="modal-overlay">
          <OnboardingWizard
            onComplete={async () => {
              setShowOnboarding(false);
              await loadAppState();
            }}
          />
        </div>
      )}

      {/* Help System */}
      <HelpSystem isOpen={showHelp} onClose={() => setShowHelp(false)} />
    </div>
  );
}

export default App;
