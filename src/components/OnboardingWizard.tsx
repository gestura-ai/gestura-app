import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface OnboardingStep {
  id: string;
  title: string;
  description: string;
  component: React.ComponentType<OnboardingStepProps>;
  isComplete: boolean;
}

interface OnboardingStepProps {
  onNext: () => void;
  onPrevious: () => void;
  onComplete: (data: any) => void;
}

const WelcomeStep: React.FC<OnboardingStepProps> = ({ onNext }) => (
  <div className="onboarding-step">
    <h2>Welcome to Gestura</h2>
    <p>
      Gestura is your intelligent voice and gesture assistant. Let's get you set up 
      with everything you need to start using voice commands and haptic feedback.
    </p>
    <div className="features-list">
      <div className="feature">
        <h3>🎤 Voice Processing</h3>
        <p>Convert speech to text with local or cloud-based models</p>
      </div>
      <div className="feature">
        <h3>💍 Haptic Harmony Ring</h3>
        <p>Connect your ring for gesture control and haptic feedback</p>
      </div>
      <div className="feature">
        <h3>🤖 AI Agents</h3>
        <p>Spawn and manage AI agents for various tasks</p>
      </div>
      <div className="feature">
        <h3>🔒 Privacy First</h3>
        <p>Your data stays local with optional cloud features</p>
      </div>
    </div>
    <button className="btn btn-primary" onClick={onNext}>
      Get Started
    </button>
  </div>
);

const VoiceSetupStep: React.FC<OnboardingStepProps> = ({ onNext, onPrevious, onComplete }) => {
  const [provider, setProvider] = useState('local');
  const [modelPath, setModelPath] = useState('');
  const [testing, setTesting] = useState(false);

  const testVoiceEngine = async () => {
    setTesting(true);
    try {
      await invoke('test_voice');
      onComplete({ provider, modelPath });
    } catch (error) {
      console.error('Voice test failed:', error);
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="onboarding-step">
      <h2>Voice Processing Setup</h2>
      <p>Choose how you want to process voice commands:</p>
      
      <div className="form-group">
        <label>Voice Provider</label>
        <select value={provider} onChange={(e) => setProvider(e.target.value)}>
          <option value="local">Local Processing (Recommended)</option>
          <option value="openai">OpenAI Whisper API</option>
          <option value="mock">Mock (for testing)</option>
        </select>
      </div>

      {provider === 'local' && (
        <div className="form-group">
          <label>Model Path (Optional)</label>
          <input
            type="text"
            value={modelPath}
            onChange={(e) => setModelPath(e.target.value)}
            placeholder="/path/to/whisper/model.bin"
          />
          <small>Leave empty to use default model</small>
        </div>
      )}

      <div className="button-group">
        <button className="btn btn-secondary" onClick={onPrevious}>
          Previous
        </button>
        <button 
          className="btn btn-primary" 
          onClick={testVoiceEngine}
          disabled={testing}
        >
          {testing ? 'Testing...' : 'Test & Continue'}
        </button>
      </div>
    </div>
  );
};

const RingSetupStep: React.FC<OnboardingStepProps> = ({ onNext, onPrevious, onComplete }) => {
  const [scanning, setScanning] = useState(false);
  const [rings, setRings] = useState<string[]>([]);
  const [selectedRing, setSelectedRing] = useState('');

  const scanForRings = async () => {
    setScanning(true);
    try {
      const foundRings = await invoke<string[]>('scan_for_rings');
      setRings(foundRings);
      if (foundRings.length > 0) {
        setSelectedRing(foundRings[0]);
      }
    } catch (error) {
      console.error('Ring scan failed:', error);
    } finally {
      setScanning(false);
    }
  };

  const pairRing = async () => {
    if (!selectedRing) return;
    
    try {
      await invoke('pair_ring', { deviceId: selectedRing });
      onComplete({ ringId: selectedRing });
    } catch (error) {
      console.error('Ring pairing failed:', error);
    }
  };

  return (
    <div className="onboarding-step">
      <h2>Haptic Harmony Ring Setup</h2>
      <p>Connect your Haptic Harmony ring for gesture control and feedback:</p>
      
      <div className="ring-setup">
        <button 
          className="btn btn-primary" 
          onClick={scanForRings}
          disabled={scanning}
        >
          {scanning ? 'Scanning...' : 'Scan for Rings'}
        </button>

        {rings.length > 0 && (
          <div className="form-group">
            <label>Available Rings</label>
            <select 
              value={selectedRing} 
              onChange={(e) => setSelectedRing(e.target.value)}
            >
              {rings.map(ring => (
                <option key={ring} value={ring}>{ring}</option>
              ))}
            </select>
          </div>
        )}
      </div>

      <div className="button-group">
        <button className="btn btn-secondary" onClick={onPrevious}>
          Previous
        </button>
        {rings.length > 0 ? (
          <button className="btn btn-primary" onClick={pairRing}>
            Pair Ring & Continue
          </button>
        ) : (
          <button className="btn btn-secondary" onClick={onNext}>
            Skip Ring Setup
          </button>
        )}
      </div>
    </div>
  );
};

const PrivacyConsentStep: React.FC<OnboardingStepProps> = ({ onNext, onPrevious, onComplete }) => {
  const [consents, setConsents] = useState({
    voice: false,
    device: false,
    usage: false,
  });

  const handleConsentChange = (category: string, value: boolean) => {
    setConsents(prev => ({ ...prev, [category]: value }));
  };

  const registerConsents = async () => {
    try {
      const userId = 'default-user'; // In a real app, this would be the actual user ID
      
      for (const [category, granted] of Object.entries(consents)) {
        if (granted) {
          await invoke('register_consent', {
            userId,
            category,
            purpose: `${category} data processing for Gestura functionality`
          });
        }
      }
      
      onComplete({ consents });
    } catch (error) {
      console.error('Consent registration failed:', error);
    }
  };

  return (
    <div className="onboarding-step">
      <h2>Privacy & Data Consent</h2>
      <p>Please review and consent to data processing:</p>
      
      <div className="consent-options">
        <div className="consent-item">
          <label>
            <input
              type="checkbox"
              checked={consents.voice}
              onChange={(e) => handleConsentChange('voice', e.target.checked)}
            />
            <strong>Voice Data Processing</strong>
            <p>Allow processing of voice recordings for speech-to-text conversion. Data is processed locally by default.</p>
          </label>
        </div>

        <div className="consent-item">
          <label>
            <input
              type="checkbox"
              checked={consents.device}
              onChange={(e) => handleConsentChange('device', e.target.checked)}
            />
            <strong>Device Data Collection</strong>
            <p>Allow collection of device information (battery level, connection status) for optimal performance.</p>
          </label>
        </div>

        <div className="consent-item">
          <label>
            <input
              type="checkbox"
              checked={consents.usage}
              onChange={(e) => handleConsentChange('usage', e.target.checked)}
            />
            <strong>Usage Analytics</strong>
            <p>Allow collection of anonymous usage statistics to improve the application. No personal data is included.</p>
          </label>
        </div>
      </div>

      <div className="privacy-note">
        <p><strong>Your Privacy Rights:</strong></p>
        <ul>
          <li>You can withdraw consent at any time in Settings</li>
          <li>You can export your data at any time</li>
          <li>You can request data deletion</li>
          <li>All processing complies with GDPR</li>
        </ul>
      </div>

      <div className="button-group">
        <button className="btn btn-secondary" onClick={onPrevious}>
          Previous
        </button>
        <button className="btn btn-primary" onClick={registerConsents}>
          Save Preferences & Continue
        </button>
      </div>
    </div>
  );
};

const CompletionStep: React.FC<OnboardingStepProps> = ({ onComplete }) => {
  const finishOnboarding = () => {
    onComplete({ completed: true });
  };

  return (
    <div className="onboarding-step">
      <h2>🎉 Setup Complete!</h2>
      <p>Gestura is now ready to use. Here are some things you can try:</p>
      
      <div className="next-steps">
        <div className="step">
          <h3>🎤 Test Voice Commands</h3>
          <p>Go to the Voice panel and try recording a voice command</p>
        </div>
        <div className="step">
          <h3>💍 Explore Ring Features</h3>
          <p>Visit the Ring panel to test haptic feedback and gestures</p>
        </div>
        <div className="step">
          <h3>⚙️ Customize Settings</h3>
          <p>Check out the Settings panel to personalize your experience</p>
        </div>
        <div className="step">
          <h3>📚 Learn More</h3>
          <p>Press F1 or click the help icon for detailed documentation</p>
        </div>
      </div>

      <button className="btn btn-primary btn-large" onClick={finishOnboarding}>
        Start Using Gestura
      </button>
    </div>
  );
};

const OnboardingWizard: React.FC<{ onComplete: () => void }> = ({ onComplete }) => {
  const [currentStep, setCurrentStep] = useState(0);
  const [stepData, setStepData] = useState<any>({});

  const steps: OnboardingStep[] = [
    {
      id: 'welcome',
      title: 'Welcome',
      description: 'Introduction to Gestura',
      component: WelcomeStep,
      isComplete: false,
    },
    {
      id: 'voice',
      title: 'Voice Setup',
      description: 'Configure voice processing',
      component: VoiceSetupStep,
      isComplete: false,
    },
    {
      id: 'ring',
      title: 'Ring Setup',
      description: 'Connect your Haptic Harmony ring',
      component: RingSetupStep,
      isComplete: false,
    },
    {
      id: 'privacy',
      title: 'Privacy',
      description: 'Data consent and privacy settings',
      component: PrivacyConsentStep,
      isComplete: false,
    },
    {
      id: 'complete',
      title: 'Complete',
      description: 'Setup finished',
      component: CompletionStep,
      isComplete: false,
    },
  ];

  const handleNext = () => {
    if (currentStep < steps.length - 1) {
      setCurrentStep(currentStep + 1);
    }
  };

  const handlePrevious = () => {
    if (currentStep > 0) {
      setCurrentStep(currentStep - 1);
    }
  };

  const handleStepComplete = (data: any) => {
    setStepData(prev => ({ ...prev, [steps[currentStep].id]: data }));
    
    if (currentStep === steps.length - 1) {
      // Final step completed
      localStorage.setItem('gestura_onboarding_completed', 'true');
      onComplete();
    } else {
      handleNext();
    }
  };

  const CurrentStepComponent = steps[currentStep].component;

  return (
    <div className="onboarding-wizard">
      <div className="onboarding-header">
        <div className="progress-bar">
          <div 
            className="progress-fill" 
            style={{ width: `${((currentStep + 1) / steps.length) * 100}%` }}
          />
        </div>
        <div className="step-info">
          <span className="step-number">Step {currentStep + 1} of {steps.length}</span>
          <span className="step-title">{steps[currentStep].title}</span>
        </div>
      </div>

      <div className="onboarding-content">
        <CurrentStepComponent
          onNext={handleNext}
          onPrevious={handlePrevious}
          onComplete={handleStepComplete}
        />
      </div>
    </div>
  );
};

export default OnboardingWizard;
