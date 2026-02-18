import React, { useState, useEffect, useRef } from 'react';
import {
  checkSystemPermissions,
  openSystemPreferences,
  PermissionStatus,
  requestPermission as requestPermissionIpc,
} from '../services/tauri/permissions';
import { registerConsent } from '../services/tauri/consent';
import { pairRing as pairRingIpc, scanForRings as scanForRingsIpc } from '../services/tauri/ring';
import { testVoice } from '../services/tauri/voice';
import { ONBOARDING_COMPLETED_KEY } from '../shared/constants/storageKeys';
import { setLocalStorageFlag } from '../shared/storage/localStorageFlag';
import { Button } from '../shared/components/Button';
import { FormGroup } from '../shared/components/FormGroup';

function usePrefersReducedMotion(): boolean {
  const [prefersReducedMotion, setPrefersReducedMotion] = useState(false);

  useEffect(() => {
    if (!window.matchMedia) return;

    const media = window.matchMedia('(prefers-reduced-motion: reduce)');
    const update = () => setPrefersReducedMotion(media.matches);
    update();

    type MediaQueryListLegacy = MediaQueryList & {
      addListener?: (listener: () => void) => void;
      removeListener?: (listener: () => void) => void;
    };

    // Safari 13 compatibility: MediaQueryList used addListener/removeListener.
    if (typeof media.addEventListener === 'function') {
      media.addEventListener('change', update);
      return () => media.removeEventListener('change', update);
    }

    const legacy = media as unknown as MediaQueryListLegacy;
    if (typeof legacy.addListener === 'function') {
      legacy.addListener(update);
      return () => legacy.removeListener?.(update);
    }
  }, []);

  return prefersReducedMotion;
}

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
  onComplete: (data: unknown) => void;
}

const WelcomeStep: React.FC<OnboardingStepProps> = ({ onNext }) => (
  <div className="onboarding-step">
    <p>
      Gestura is a privacy-first AI assistant that <strong>listens, speaks, and acts</strong>, helping with research,
      daily life, and coding. Let's configure the essentials so you can start using your agent right away.
    </p>
    <div className="features-list">
      <div className="feature">
        <h3>Voice Processing</h3>
        <p>Convert speech to text with local or cloud-based models</p>
      </div>
      <div className="feature">
        <h3>Haptic Harmony Ring</h3>
        <p>Connect your ring for gesture control and haptic feedback</p>
      </div>
      <div className="feature">
        <h3>AI Agents</h3>
        <p>Spawn and manage AI agents for various tasks</p>
      </div>
      <div className="feature">
        <h3>Privacy First</h3>
        <p>Your data stays local with optional cloud features</p>
      </div>
    </div>
    <Button tone="primary" onClick={onNext}>
      Get Started
    </Button>
  </div>
);

const PermissionsStep: React.FC<OnboardingStepProps> = ({ onNext, onPrevious }) => {
  const [permissions, setPermissions] = useState<PermissionStatus[]>([]);
  const [loading, setLoading] = useState(true);

  const checkPermissions = async () => {
    console.log('[Onboarding] Checking system permissions...');
    setLoading(true);

    try {
      const result = await checkSystemPermissions();
      const nextPermissions = (result?.permissions ?? []).map((p) => ({
        ...p,
        status: (p.status ?? 'unknown') as PermissionStatus['status'],
      }));
      setPermissions(nextPermissions);
      console.log('[Onboarding] Updated permission statuses:', nextPermissions);
    } catch (error) {
      console.error('Failed to check system permissions:', error);
      setPermissions([]);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    checkPermissions();
  }, []);

  const requestPermission = async (permissionId: string) => {
    try {
      console.log('[Onboarding] Requesting permission:', permissionId);
      await requestPermissionIpc(permissionId);
      // Wait a moment for the system dialog to appear and potentially be granted
      setTimeout(checkPermissions, 1500);
    } catch (error) {
      console.error(`Failed to request ${permissionId} permission:`, error);

      // Fallback: open the relevant System Settings pane directly.
      // (This is especially useful for Accessibility, which is manual on macOS.)
      try {
        await openSystemPreferences(permissionId);
      } catch (openError) {
        console.error(`Failed to open System Settings for ${permissionId}:`, openError);
      }
    }
  };

  const getStatusIcon = (status: PermissionStatus['status']) => {
    switch (status) {
      case 'granted':
        return '✅';
      case 'denied':
        return '❌';
      case 'not_determined':
        return '⏳';
      case 'pending':
        return '🔄';
      default:
        return '❓';
    }
  };

  const getStatusText = (status: PermissionStatus['status']) => {
    switch (status) {
      case 'granted':
        return 'Granted';
      case 'denied':
        return 'Denied - Click to open Settings';
      case 'not_determined':
        return 'Not yet requested';
      case 'pending':
        return 'Checking...';
      default:
        return 'Unknown';
    }
  };

  const allRequiredGranted = permissions
    .filter((p) => p.required)
    .every((p) => p.status === 'granted');

  const canContinue = !loading && permissions.length > 0 && allRequiredGranted;

  return (
    <div className="onboarding-step">
      <h2>System Permissions</h2>
      <p>Gestura needs the following permissions to work properly:</p>

      <div className="permissions-list">
        {permissions.map((perm) => (
          <div key={perm.id} className={`permission-item ${perm.status}`}>
            <div className="permission-info">
              <div className="permission-header">
                <span className="permission-icon">{getStatusIcon(perm.status)}</span>
                <strong>{perm.name}</strong>
                {perm.required && <span className="required-badge">Required</span>}
              </div>
              <p className="permission-description">{perm.description}</p>
              {perm.instructions && <p className="help-text">{perm.instructions}</p>}
              <p className="permission-status">{getStatusText(perm.status)}</p>
            </div>
            {perm.status !== 'granted' && perm.status !== 'pending' && (
              <Button tone="secondary" size="small" onClick={() => requestPermission(perm.id)}>
                {perm.status === 'denied' ? 'Open Settings' : 'Grant Access'}
              </Button>
            )}
          </div>
        ))}
      </div>

      <div className="permissions-actions">
        <Button tone="secondary" onClick={checkPermissions} disabled={loading}>
          {loading ? 'Checking...' : 'Refresh Status'}
        </Button>
      </div>

      <div className="button-group">
        <Button tone="secondary" onClick={onPrevious}>
          Previous
        </Button>
        <Button tone="primary" onClick={onNext} disabled={!canContinue}>
          {canContinue ? 'Continue' : 'Grant Required Permissions'}
        </Button>
      </div>

      {!canContinue && !loading && (
        <p className="permissions-note">
          <small>
            You can skip this step, but some features may not work correctly without the required
            permissions.
          </small>
          <button className="btn-link" onClick={onNext}>
            Skip for now
          </button>
        </p>
      )}
    </div>
  );
};

const VoiceSetupStep: React.FC<OnboardingStepProps> = ({ onPrevious, onComplete }) => {
  const [provider, setProvider] = useState('local');
  const [modelPath, setModelPath] = useState('');
  const [testing, setTesting] = useState(false);

  const testVoiceEngine = async () => {
    setTesting(true);
    try {
      await testVoice();
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

      <FormGroup label="Voice Provider">
        <select value={provider} onChange={(e) => setProvider(e.target.value)}>
          <option value="local">Local Processing (Recommended)</option>
          <option value="openai">OpenAI Whisper API</option>
          <option value="mock">Mock (for testing)</option>
        </select>
      </FormGroup>

      {provider === 'local' && (
        <FormGroup label="Model Path (Optional)" hint="Leave empty to use default model">
          <input
            type="text"
            value={modelPath}
            onChange={(e) => setModelPath(e.target.value)}
            placeholder="/path/to/whisper/model.bin"
          />
        </FormGroup>
      )}

      <div className="button-group">
        <Button tone="secondary" onClick={onPrevious}>
          Previous
        </Button>
        <Button tone="primary" onClick={testVoiceEngine} disabled={testing}>
          {testing ? 'Testing...' : 'Test & Continue'}
        </Button>
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
      const foundRings = await scanForRingsIpc();
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
      await pairRingIpc(selectedRing);
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
        <Button tone="primary" onClick={scanForRings} disabled={scanning}>
          {scanning ? 'Scanning...' : 'Scan for Rings'}
        </Button>

        {rings.length > 0 && (
          <FormGroup label="Available Rings">
            <select
              value={selectedRing}
              onChange={(e) => setSelectedRing(e.target.value)}
            >
              {rings.map(ring => (
                <option key={ring} value={ring}>{ring}</option>
              ))}
            </select>
          </FormGroup>
        )}
      </div>

      <div className="button-group">
        <Button tone="secondary" onClick={onPrevious}>
          Previous
        </Button>
        {rings.length > 0 ? (
          <Button tone="primary" onClick={pairRing}>
            Pair Ring & Continue
          </Button>
        ) : (
          <Button tone="secondary" onClick={onNext}>
            Skip Ring Setup
          </Button>
        )}
      </div>
    </div>
  );
};

const PrivacyConsentStep: React.FC<OnboardingStepProps> = ({ onPrevious, onComplete }) => {
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
          await registerConsent({
            user_id: userId,
            category,
            purpose: `${category} data processing for Gestura functionality`,
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
            <div>
              <strong>Voice Data Processing</strong>
              <p>Allow processing of voice recordings for speech-to-text conversion. Data is processed locally by default.</p>
            </div>
          </label>
        </div>

        <div className="consent-item">
          <label>
            <input
              type="checkbox"
              checked={consents.device}
              onChange={(e) => handleConsentChange('device', e.target.checked)}
            />
            <div>
              <strong>Device Data Collection</strong>
              <p>Allow collection of device information (battery level, connection status) for optimal performance.</p>
            </div>
          </label>
        </div>

        <div className="consent-item">
          <label>
            <input
              type="checkbox"
              checked={consents.usage}
              onChange={(e) => handleConsentChange('usage', e.target.checked)}
            />
            <div>
              <strong>Usage Analytics</strong>
              <p>Allow collection of anonymous usage statistics to improve the application. No personal data is included.</p>
            </div>
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
        <Button tone="secondary" onClick={onPrevious}>
          Previous
        </Button>
        <Button tone="primary" onClick={registerConsents}>
          Save Preferences & Continue
        </Button>
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

      <Button tone="primary" size="large" onClick={finishOnboarding}>
        Start Using Gestura
      </Button>
    </div>
  );
};

const OnboardingWizard: React.FC<{ onComplete: () => void }> = ({ onComplete }) => {
  const [currentStep, setCurrentStep] = useState(0);
  const [, setStepData] = useState<Record<string, unknown>>({});
  const [contentVisible, setContentVisible] = useState(true);
  const [isTransitioning, setIsTransitioning] = useState(false);
  const prefersReducedMotion = usePrefersReducedMotion();

  const transitionTimeoutRef = useRef<number | null>(null);
  const unlockTimeoutRef = useRef<number | null>(null);

  const motionMs = prefersReducedMotion ? 0 : 120;

  const steps: OnboardingStep[] = [
    {
      id: 'welcome',
      title: 'Configure',
      description: 'Set up your agent',
      component: WelcomeStep,
      isComplete: false,
    },
    {
      id: 'permissions',
      title: 'Permissions',
      description: 'Grant system permissions',
      component: PermissionsStep,
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
    if (isTransitioning) return;
    if (currentStep >= steps.length - 1) return;

    const nextStep = currentStep + 1;
    if (motionMs === 0) {
      setCurrentStep(nextStep);
      return;
    }

    setIsTransitioning(true);
    setContentVisible(false);

    if (transitionTimeoutRef.current != null) window.clearTimeout(transitionTimeoutRef.current);
    if (unlockTimeoutRef.current != null) window.clearTimeout(unlockTimeoutRef.current);

    transitionTimeoutRef.current = window.setTimeout(() => {
      setCurrentStep(nextStep);
      // Fade back in on next frame so the hidden styles apply before transitioning.
      window.requestAnimationFrame(() => setContentVisible(true));
      unlockTimeoutRef.current = window.setTimeout(() => setIsTransitioning(false), motionMs);
    }, motionMs);
  };

  const handlePrevious = () => {
    if (isTransitioning) return;
    if (currentStep <= 0) return;

    const prevStep = currentStep - 1;
    if (motionMs === 0) {
      setCurrentStep(prevStep);
      return;
    }

    setIsTransitioning(true);
    setContentVisible(false);

    if (transitionTimeoutRef.current != null) window.clearTimeout(transitionTimeoutRef.current);
    if (unlockTimeoutRef.current != null) window.clearTimeout(unlockTimeoutRef.current);

    transitionTimeoutRef.current = window.setTimeout(() => {
      setCurrentStep(prevStep);
      window.requestAnimationFrame(() => setContentVisible(true));
      unlockTimeoutRef.current = window.setTimeout(() => setIsTransitioning(false), motionMs);
    }, motionMs);
  };

  const handleStepComplete = (data: unknown) => {
    setStepData((prev) => ({ ...prev, [steps[currentStep].id]: data }));

    if (currentStep === steps.length - 1) {
      // Final step completed
      setLocalStorageFlag(ONBOARDING_COMPLETED_KEY, true);
      onComplete();
    } else {
      handleNext();
    }
  };

  useEffect(() => {
    return () => {
      if (transitionTimeoutRef.current != null) window.clearTimeout(transitionTimeoutRef.current);
      if (unlockTimeoutRef.current != null) window.clearTimeout(unlockTimeoutRef.current);
    };
  }, []);

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
        <div
          className={`onboarding-content-inner ${contentVisible ? 'is-visible' : 'is-hidden'}`}
          data-transitioning={isTransitioning ? 'true' : 'false'}
        >
          <CurrentStepComponent
            onNext={handleNext}
            onPrevious={handlePrevious}
            onComplete={handleStepComplete}
          />
        </div>
      </div>
    </div>
  );
};

export default OnboardingWizard;
