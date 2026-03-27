import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  getApiKey,
  getAvailableLlmProviders,
  getConfig,
  getSessionLlmConfig,
  listAnthropicModels,
  listGeminiModels,
  listGrokModels,
  listOllamaModels,
  listOpenAiModels,
  setSessionLlmModel,
  setSessionLlmProvider,
} from '../../../services/tauri/agent';
import type { ToastKind } from '../hooks/useToast';
import {
  dispatchSessionLlmConfigChanged,
  readSessionLlmConfigChangedDetail,
  SESSION_LLM_CONFIG_CHANGED_EVENT,
} from './sessionLlmConfigEvents';

interface ModelOption {
  id: string;
  label: string;
}

interface LlmProviderSelectorProps {
  sessionId: string;
  onShowToast: (msg: string, kind?: ToastKind) => void;
  onHeaderDropdownOpenChange?: (open: boolean) => void;
  variant?: 'header' | 'panel';
  enabled?: boolean;
}

type HeaderDropdownKind = 'provider' | 'model' | null;

const LLM_PROVIDERS: Array<{ id: string; name: string }> = [
  { id: 'openai', name: 'OpenAI' },
  { id: 'anthropic', name: 'Anthropic' },
  { id: 'gemini', name: 'Gemini (Google)' },
  { id: 'grok', name: 'Grok (xAI)' },
  { id: 'ollama', name: 'Ollama (Local)' },
];

function toModelOption(model: unknown): ModelOption | null {
  if (typeof model === 'string' && model.length > 0) {
    return { id: model, label: model };
  }

  if (model && typeof model === 'object') {
    const entry = model as Record<string, unknown>;
    const id =
      (typeof entry.id === 'string' && entry.id.length > 0 ? entry.id : null)
      ?? (typeof entry.name === 'string' && entry.name.length > 0 ? entry.name : null)
      ?? '';

    if (!id) return null;

    const label =
      (typeof entry.name === 'string' && entry.name.length > 0 ? entry.name : null)
      ?? (typeof entry.label === 'string' && entry.label.length > 0 ? entry.label : null)
      ?? id;

    return { id, label };
  }

  return null;
}

interface HeaderDropdownOption {
  id: string;
  label: string;
  selected?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

interface HeaderAttachedDropdownProps {
  ariaLabel: string;
  buttonClassName: string;
  menuClassName: string;
  label: string;
  open: boolean;
  disabled?: boolean;
  options: HeaderDropdownOption[];
  onToggle: () => void;
}

function HeaderAttachedDropdown({
  ariaLabel,
  buttonClassName,
  menuClassName,
  label,
  open,
  disabled = false,
  options,
  onToggle,
}: HeaderAttachedDropdownProps) {
  return (
    <div className={menuClassName}>
      <button
        type="button"
        aria-label={ariaLabel}
        aria-haspopup="menu"
        aria-expanded={open}
        className={buttonClassName}
        disabled={disabled}
        onClick={onToggle}
      >
        <span className="agent-session-llm-selector__trigger-arrow" aria-hidden="true">
          <svg viewBox="0 0 12 12" focusable="false">
            <path d="M2.5 4.5 6 8l3.5-3.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="square" />
          </svg>
        </span>
        <span className="agent-session-llm-selector__trigger-label">{label}</span>
      </button>

      {open && !disabled && (
        <div
          className="agent-session-llm-selector__menu"
          role="menu"
          aria-label={`${ariaLabel} menu`}
        >
          {options.map((option) => (
            <button
              key={option.id}
              type="button"
              role="menuitemradio"
              aria-checked={option.selected}
              className={[
                'agent-session-llm-selector__menu-item',
                option.selected ? 'is-selected' : '',
              ].filter(Boolean).join(' ')}
              disabled={option.disabled}
              onClick={option.onSelect}
            >
              <span className="agent-session-llm-selector__menu-item-label">{option.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function LlmProviderSelector({
  sessionId,
  onShowToast,
  onHeaderDropdownOpenChange,
  variant = 'panel',
  enabled = true,
}: LlmProviderSelectorProps) {
  const [filteredProviders, setFilteredProviders] = useState<Array<{ id: string; name: string }>>([]);
  const [llmProvider, setLlmProvider] = useState('');
  const [llmModels, setLlmModels] = useState<ModelOption[]>([]);
  const [llmModel, setLlmModel] = useState('');
  const [loading, setLoading] = useState(false);
  const [llmModelsLoading, setLlmModelsLoading] = useState(false);
  const [openDropdown, setOpenDropdown] = useState<HeaderDropdownKind>(null);
  const headerSelectorRef = useRef<HTMLDivElement | null>(null);

  const loadModelsForProvider = useCallback(
    async (provider: string, currentModel?: string) => {
      setLlmModelsLoading(true);
      setLlmModels([]);

      try {
        let rawModels: unknown[] = [];
        const cfg = await getConfig().catch(() => null);
        const globalLlmCfg = (cfg as Record<string, unknown> | null)?.llm as Record<string, unknown> | null;

        switch (provider) {
          case 'anthropic':
          case 'openai':
          case 'gemini':
          case 'grok': {
            const apiKey = await getApiKey(provider).catch(() => null) ?? '';
            if (!apiKey) {
              setLlmModel('');
              onShowToast(`Set ${provider} API key to load models`, 'info');
              return { options: [], selectedId: '' };
            }

            if (provider === 'anthropic') rawModels = await listAnthropicModels(apiKey);
            else if (provider === 'openai') rawModels = await listOpenAiModels(apiKey);
            else if (provider === 'gemini') rawModels = await listGeminiModels(apiKey);
            else rawModels = await listGrokModels(apiKey);
            break;
          }
          case 'ollama': {
            const ollamaSection = globalLlmCfg?.ollama as Record<string, unknown> | null;
            const endpoint =
              (ollamaSection?.base_url as string | undefined)
              || (ollamaSection?.endpoint as string | undefined)
              || 'http://localhost:11434';
            rawModels = await listOllamaModels(endpoint);
            break;
          }
          default:
            rawModels = [];
        }

        const options = rawModels.map(toModelOption).filter((model): model is ModelOption => model !== null);
        if (currentModel && !options.some((option) => option.id === currentModel)) {
          options.unshift({ id: currentModel, label: `${currentModel} (saved)` });
        }

        const selectedId = currentModel || options[0]?.id || '';
        setLlmModels(options);
        setLlmModel(selectedId);

        return { options, selectedId };
      } catch (error) {
        console.error('[LlmProviderSelector] Failed to load models for', provider, error);
        setLlmModels([]);
        setLlmModel('');
        return { options: [], selectedId: '' };
      } finally {
        setLlmModelsLoading(false);
      }
    },
    [onShowToast],
  );

  const loadProviderSettings = useCallback(async () => {
    if (!enabled) return;

    setLoading(true);

    try {
      const [availableProviders, sessionLlm, globalConfig] = await Promise.all([
        getAvailableLlmProviders(),
        sessionId ? getSessionLlmConfig(sessionId).catch(() => null) : Promise.resolve(null),
        getConfig().catch(() => null),
      ]);

      const available = LLM_PROVIDERS.filter((provider) => availableProviders[provider.id] === true);
      setFilteredProviders(available);

      if (available.length === 0) {
        setLlmProvider('');
        setLlmModels([]);
        setLlmModel('');
        return;
      }

      const llm = sessionLlm as Record<string, unknown> | null;
      const global = globalConfig as Record<string, unknown> | null;
      const globalLlm = global?.llm as Record<string, unknown> | null;

      let currentProvider =
        (llm?.provider as string)
        || (globalLlm?.primary as string)
        || 'openai';

      if (!available.some((provider) => provider.id === currentProvider)) {
        currentProvider = available[0].id;
      }

      setLlmProvider(currentProvider);

      const providerCfg = globalLlm?.[currentProvider] as Record<string, unknown> | null;
      const globalModelForProvider = providerCfg?.model as string | undefined;
      const currentModel = (llm?.model as string) || globalModelForProvider;
      await loadModelsForProvider(currentProvider, currentModel);
    } catch (error) {
      console.error('[LlmProviderSelector] Failed to load provider settings:', error);
      onShowToast(`Failed to load provider settings: ${error}`, 'error');
    } finally {
      setLoading(false);
    }
  }, [enabled, loadModelsForProvider, onShowToast, sessionId]);

  useEffect(() => {
    if (enabled) {
      void loadProviderSettings();
    }
  }, [enabled, loadProviderSettings]);

  useEffect(() => {
    if (variant !== 'header' || !openDropdown) return undefined;

    const handlePointerDown = (event: MouseEvent) => {
      if (!headerSelectorRef.current?.contains(event.target as Node)) {
        setOpenDropdown(null);
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpenDropdown(null);
      }
    };

    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [openDropdown, variant]);

  useEffect(() => {
    if (variant !== 'header') return undefined;

    onHeaderDropdownOpenChange?.(openDropdown !== null);
    return () => {
      onHeaderDropdownOpenChange?.(false);
    };
  }, [onHeaderDropdownOpenChange, openDropdown, variant]);

  useEffect(() => {
    if (typeof window === 'undefined' || !sessionId) return undefined;

    const handleConfigChanged = (event: Event) => {
      const detail = readSessionLlmConfigChangedDetail(event);
      if (detail?.sessionId !== sessionId || !enabled) return;
      void loadProviderSettings();
    };

    window.addEventListener(SESSION_LLM_CONFIG_CHANGED_EVENT, handleConfigChanged);
    return () => window.removeEventListener(SESSION_LLM_CONFIG_CHANGED_EVENT, handleConfigChanged);
  }, [enabled, loadProviderSettings, sessionId]);

  const handleLlmProviderChange = useCallback(
    async (provider: string) => {
      setLlmProvider(provider);
      setLlmModel('');

      try {
        if (sessionId) {
          await setSessionLlmProvider(sessionId, provider);
        }

        const globalConfig = await getConfig().catch(() => null);
        const globalLlm =
          (globalConfig as Record<string, unknown> | null)?.llm as Record<string, unknown> | null;
        const globalModel =
          (globalLlm?.[provider] as Record<string, unknown> | null)?.model as string | undefined;
        const { selectedId } = await loadModelsForProvider(provider, globalModel);

        if (sessionId && selectedId) {
          await setSessionLlmModel(sessionId, selectedId);
        }

        setOpenDropdown(null);
        dispatchSessionLlmConfigChanged(sessionId);
      } catch (error) {
        console.error('[LlmProviderSelector] Failed to set LLM provider:', error);
        onShowToast(`Failed to update provider: ${error}`, 'error');
      }
    },
    [loadModelsForProvider, onShowToast, sessionId],
  );

  const handleLlmModelChange = useCallback(
    async (model: string) => {
      if (!model) return;

      setLlmModel(model);

      try {
        if (sessionId) {
          await setSessionLlmModel(sessionId, model);
        }
        setOpenDropdown(null);
        dispatchSessionLlmConfigChanged(sessionId);
        onShowToast('Model updated', 'success');
      } catch (error) {
        console.error('[LlmProviderSelector] Failed to set LLM model:', error);
        onShowToast(`Failed to update model: ${error}`, 'error');
      }
    },
    [onShowToast, sessionId],
  );

  const currentProviderLabel = useMemo(
    () => filteredProviders.find((provider) => provider.id === llmProvider)?.name ?? 'Provider',
    [filteredProviders, llmProvider],
  );

  const currentModelLabel = useMemo(
    () => llmModels.find((model) => model.id === llmModel)?.label ?? llmModel ?? 'Model',
    [llmModel, llmModels],
  );

  const providerOptions = useMemo<HeaderDropdownOption[]>(
    () => filteredProviders.map((provider) => ({
      id: provider.id,
      label: provider.name,
      selected: provider.id === llmProvider,
      onSelect: () => void handleLlmProviderChange(provider.id),
    })),
    [filteredProviders, handleLlmProviderChange, llmProvider],
  );

  const modelOptions = useMemo<HeaderDropdownOption[]>(
    () => {
      if (llmModelsLoading) {
        return [{ id: 'loading', label: 'Loading models…', disabled: true, onSelect: () => undefined }];
      }

      if (llmModels.length === 0) {
        return [{ id: 'empty', label: 'No models available', disabled: true, onSelect: () => undefined }];
      }

      return llmModels.map((model) => ({
        id: model.id,
        label: model.label,
        selected: model.id === llmModel,
        onSelect: () => void handleLlmModelChange(model.id),
      }));
    },
    [handleLlmModelChange, llmModel, llmModels, llmModelsLoading],
  );

  if (variant === 'header') {
    return (
      <div
        ref={headerSelectorRef}
        className="agent-session-llm-selector agent-session-llm-selector--header"
        data-testid="agent-session-llm-selector"
      >
        {loading ? (
          <span className="agent-session-llm-selector__status" aria-live="polite">Loading models…</span>
        ) : filteredProviders.length === 0 ? (
          <span className="agent-session-llm-selector__status">No providers configured</span>
        ) : (
          <>
            <HeaderAttachedDropdown
              ariaLabel="LLM provider"
              buttonClassName="agent-session-llm-selector__trigger agent-session-llm-selector__trigger--provider"
              menuClassName="agent-session-llm-selector__dropdown agent-session-llm-selector__dropdown--provider"
              label={currentProviderLabel}
              open={openDropdown === 'provider'}
              options={providerOptions}
              onToggle={() => setOpenDropdown((current) => (current === 'provider' ? null : 'provider'))}
            />
            <HeaderAttachedDropdown
              ariaLabel="LLM model"
              buttonClassName="agent-session-llm-selector__trigger agent-session-llm-selector__trigger--model"
              menuClassName="agent-session-llm-selector__dropdown agent-session-llm-selector__dropdown--model"
              label={llmModelsLoading ? 'Loading…' : currentModelLabel}
              open={openDropdown === 'model'}
              disabled={llmModelsLoading || llmModels.length === 0}
              options={modelOptions}
              onToggle={() => setOpenDropdown((current) => (current === 'model' ? null : 'model'))}
            />
          </>
        )}
      </div>
    );
  }

  return (
    <div className="providers-section">
      <div className="providers-section-header">
        <span className="icon-cpu-chip" />
        <span>LLM Provider</span>
      </div>
      <p className="providers-section-info">
        Select the AI model provider and model for this session.
      </p>
      {loading ? (
        <div className="providers-section-info">Loading provider settings…</div>
      ) : filteredProviders.length === 0 ? (
        <p className="providers-section-info" style={{ color: 'var(--color-danger, #f87171)' }}>
          No providers configured. Add API keys in Settings.
        </p>
      ) : (
        <>
          <div className="session-field">
            <label>Provider</label>
            <select
              className="provider-select"
              value={llmProvider}
              onChange={(event) => void handleLlmProviderChange(event.target.value)}
            >
              {filteredProviders.map((provider) => (
                <option key={provider.id} value={provider.id}>{provider.name}</option>
              ))}
            </select>
          </div>
          <div className="session-field">
            <label>Model</label>
            <select
              className="provider-select"
              value={llmModel}
              disabled={llmModelsLoading || llmModels.length === 0}
              onChange={(event) => void handleLlmModelChange(event.target.value)}
            >
              {llmModelsLoading ? (
                <option value="">Loading models…</option>
              ) : llmModels.length === 0 ? (
                <option value="">No models available</option>
              ) : (
                llmModels.map((model) => (
                  <option key={model.id} value={model.id}>{model.label}</option>
                ))
              )}
            </select>
          </div>
        </>
      )}
    </div>
  );
}
