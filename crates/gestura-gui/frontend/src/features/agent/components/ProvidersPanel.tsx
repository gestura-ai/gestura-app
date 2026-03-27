import { useCallback, useEffect, useState } from 'react';
import {
  clearSessionLlmConfig,
  clearSessionVoiceConfig,
  getConfig,
  getSessionVoiceConfig,
  getWhisperModels,
  isWhisperModelDownloaded,
  listOpenAiSttModels,
  setSessionVoiceModel,
  setSessionVoiceProvider,
} from '../../../services/tauri/agent';
import type { ToastKind } from '../hooks/useToast';
import { LlmProviderSelector } from './LlmProviderSelector';
import { dispatchSessionLlmConfigChanged } from './sessionLlmConfigEvents';

interface ProvidersPanelProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
  onShowToast: (msg: string, kind?: ToastKind) => void;
}

interface ModelOption {
  id: string;
  label: string;
}

/** Strip whisper size suffix from display name, e.g. " (142 MB)" */
function formatWhisperName(model: unknown): string {
  const obj = model as Record<string, unknown>;
  const raw = String(obj?.name ?? obj?.filename ?? '').trim();
  if (!raw) return '';
  return raw.replace(/\s*\(\s*[0-9]+(?:\.[0-9]+)?\s*(?:kb|mb|gb)\s*\)\s*$/i, '').trim();
}

/** Returns the last path segment (filename). */
function pathBasename(p: string): string {
  return p.split('/').pop() ?? p;
}

export function ProvidersPanel({
  isOpen,
  onClose,
  sessionId,
  onShowToast,
}: ProvidersPanelProps) {
  // ── STT state ───────────────────────────────────────────────────────────
  const [sttProvider, setSttProvider] = useState('openai');
  const [sttModels, setSttModels] = useState<ModelOption[]>([]);
  const [sttModel, setSttModel] = useState('');
  const [sttModelsLoading, setSttModelsLoading] = useState(false);

  // ── General ─────────────────────────────────────────────────────────────
  const [loading, setLoading] = useState(false);

  // ── Load STT models for the selected provider ────────────────────────────
  const loadSttModels = useCallback(
    async (provider: string, currentModel?: string) => {
      setSttModelsLoading(true);
      setSttModels([]);
      try {
        if (provider === 'local') {
          const allModels = await getWhisperModels();
          const checkedArr = await Promise.all(
            (Array.isArray(allModels) ? allModels : []).map(async (m) => {
              try {
                const obj = m as Record<string, unknown>;
                const filename = String(obj?.filename ?? obj?.name ?? '');
                // Rust returns { exists: boolean, ... } — check .exists, not the object itself.
                const result = await isWhisperModelDownloaded(filename);
                return result.exists ? m : null;
              } catch {
                return null;
              }
            })
          );
          const downloaded = checkedArr.filter((m): m is NonNullable<unknown> => m !== null);
          const options: ModelOption[] = downloaded.map((m) => {
            const obj = m as Record<string, unknown>;
            const filename = String(obj?.filename ?? obj?.name ?? '');
            return { id: filename, label: formatWhisperName(m) || filename };
          });
          setSttModels(options);
          const normalised = currentModel ? pathBasename(currentModel) : '';
          const selectedId =
            normalised && options.some((o) => o.id === normalised)
              ? normalised
              : options[0]?.id ?? '';
          setSttModel(selectedId);
        } else if (provider === 'openai') {
          const rawModels = await listOpenAiSttModels();
          const options = (Array.isArray(rawModels) ? rawModels : [])
            .map((model) => {
              if (typeof model === 'string' && model.length > 0) return { id: model, label: model };
              if (model && typeof model === 'object') {
                const entry = model as Record<string, unknown>;
                const id =
                  (typeof entry.id === 'string' && entry.id.length > 0 ? entry.id : null)
                  ?? (typeof entry.name === 'string' && entry.name.length > 0 ? entry.name : null)
                  ?? '';
                if (!id) return null;
                return {
                  id,
                  label:
                    (typeof entry.name === 'string' && entry.name.length > 0 ? entry.name : null)
                    ?? (typeof entry.label === 'string' && entry.label.length > 0 ? entry.label : null)
                    ?? id,
                };
              }
              return null;
            })
            .filter((m): m is ModelOption => m !== null);
          setSttModels(options);
          const selectedId =
            currentModel && options.some((o) => o.id === currentModel)
              ? currentModel
              : options[0]?.id ?? '';
          setSttModel(selectedId);
        } else {
          setSttModels([]);
          setSttModel('');
        }
      } catch (e) {
        console.error('[ProvidersPanel] Failed to load STT models for', provider, e);
        setSttModels([]);
      } finally {
        setSttModelsLoading(false);
      }
    },
    [],
  );

  // ── Initial voice/STT load when panel opens ──────────────────────────────
  const loadVoiceSettings = useCallback(async () => {
    setLoading(true);
    try {
      const [globalConfig, sessionVoice] = await Promise.all([
        getConfig().catch(() => null),
        getSessionVoiceConfig(sessionId).catch(() => null),
      ]);

      // STT
      const voice = sessionVoice as Record<string, unknown> | null;
      const globalVoice =
        (globalConfig as Record<string, unknown> | null)?.voice as Record<string, unknown> | null;
      const currentSttProvider =
        (voice?.provider as string) || (globalVoice?.provider as string) || 'openai';
      setSttProvider(currentSttProvider);

      let currentSttModel: string | undefined;
      if (currentSttProvider === 'local') {
        currentSttModel =
          (voice?.model as string) || (globalVoice?.local_model_path as string) || undefined;
      } else {
        currentSttModel =
          (voice?.model as string) || (globalVoice?.openai_model as string) || undefined;
      }
      void loadSttModels(currentSttProvider, currentSttModel);
    } catch (e) {
      console.error('[ProvidersPanel] Failed to load provider settings:', e);
      onShowToast(`Failed to load provider settings: ${e}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [loadSttModels, onShowToast, sessionId]);

  useEffect(() => {
    if (isOpen) void loadVoiceSettings();
  }, [isOpen, loadVoiceSettings]);

  // ── STT provider change ──────────────────────────────────────────────────
  const handleSttProviderChange = useCallback(
    async (provider: string) => {
      setSttProvider(provider);
      setSttModel('');
      try {
        await setSessionVoiceProvider(sessionId, provider);
        await loadSttModels(provider);
        onShowToast('STT provider updated for this session', 'success');
      } catch (e) {
        console.error('[ProvidersPanel] Failed to set STT provider:', e);
        onShowToast(`Failed to update STT provider: ${e}`, 'error');
      }
    },
    [loadSttModels, onShowToast, sessionId],
  );

  // ── STT model change ─────────────────────────────────────────────────────
  const handleSttModelChange = useCallback(
    async (model: string) => {
      if (!model) return;
      setSttModel(model);
      try {
        await setSessionVoiceModel(sessionId, model);
        onShowToast('STT model updated for this session', 'success');
      } catch (e) {
        console.error('[ProvidersPanel] Failed to set STT model:', e);
        onShowToast(`Failed to update STT model: ${e}`, 'error');
      }
    },
    [onShowToast, sessionId],
  );

  // ── Reset to global defaults ─────────────────────────────────────────────
  const handleReset = useCallback(async () => {
    try {
      await clearSessionVoiceConfig(sessionId);
      await clearSessionLlmConfig(sessionId);
      await loadVoiceSettings();
      dispatchSessionLlmConfigChanged(sessionId);
      onShowToast('Reset to global defaults', 'success');
    } catch (e) {
      onShowToast(`Failed to reset: ${e}`, 'error');
    }
  }, [loadVoiceSettings, onShowToast, sessionId]);

  return (
    <>
      <div
        className={`session-panel-overlay${isOpen ? ' visible' : ''}`}
        onClick={onClose}
      />
      <div className={`session-panel${isOpen ? ' open' : ''}`}>
        <div className="session-panel-header">
          <h3>Providers</h3>
          <button className="session-panel-close" onClick={onClose} title="Close">
            <span className="icon-close" />
          </button>
        </div>

        <div className="session-panel-content">
          {loading ? (
            <div className="providers-section-info">Loading provider settings…</div>
          ) : (
            <>
              <LlmProviderSelector
                enabled={isOpen}
                sessionId={sessionId}
                onShowToast={onShowToast}
                variant="panel"
              />

              <div className="session-divider" />

              {/* STT Provider Section */}
              <div className="providers-section">
                <div className="providers-section-header">
                  <span className="icon-microphone" />
                  <span>Speech-to-Text Provider</span>
                </div>
                <p className="providers-section-info">
                  Select the speech recognition provider for voice input in this session.
                </p>
                <div className="session-field">
                  <label>STT Provider</label>
                  <select
                    className="provider-select"
                    value={sttProvider}
                    onChange={(e) => void handleSttProviderChange(e.target.value)}
                  >
                    <option value="openai">OpenAI Whisper (Cloud)</option>
                    <option value="local">Local Whisper</option>
                  </select>
                </div>
                <div className="session-field">
                  <label>STT Model</label>
                  <select
                    className="provider-select"
                    value={sttModel}
                    disabled={sttModelsLoading || sttModels.length === 0}
                    onChange={(e) => void handleSttModelChange(e.target.value)}
                  >
                    {sttModelsLoading ? (
                      <option value="">Loading models…</option>
                    ) : sttModels.length === 0 ? (
                      <option value="">
                        {sttProvider === 'local'
                          ? "No local models downloaded"
                          : "No models available"}
                      </option>
                    ) : (
                      sttModels.map((m) => (
                        <option key={m.id} value={m.id}>{m.label}</option>
                      ))
                    )}
                  </select>
                </div>
              </div>

              <div className="session-divider" />

              {/* Reset */}
              <div className="providers-section">
                <button
                  className="btn-secondary"
                  style={{ width: '100%' }}
                  onClick={() => void handleReset()}
                >
                  Reset to Global Defaults
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </>
  );
}

