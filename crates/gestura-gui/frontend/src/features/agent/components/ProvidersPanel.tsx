import { useCallback, useEffect, useState } from "react";
import {
  clearSessionLlmConfig,
  clearSessionVoiceConfig,
  getApiKey,
  getAvailableLlmProviders,
  getConfig,
  getSessionLlmConfig,
  getSessionVoiceConfig,
  getWhisperModels,
  isWhisperModelDownloaded,
  listAnthropicModels,
  listGeminiModels,
  listGrokModels,
  listOllamaModels,
  listOpenAiModels,
  listOpenAiSttModels,
  setSessionLlmModel,
  setSessionLlmProvider,
  setSessionVoiceModel,
  setSessionVoiceProvider,
} from "../../../services/tauri/agent";
import type { ToastKind } from "../hooks/useToast";

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

const LLM_PROVIDERS: { id: string; name: string }[] = [
  { id: "openai", name: "OpenAI" },
  { id: "anthropic", name: "Anthropic" },
  { id: "gemini", name: "Gemini (Google)" },
  { id: "grok", name: "Grok (xAI)" },
  { id: "ollama", name: "Ollama (Local)" },
];

/** Normalise a raw model entry (string | {id?, name?, label?}) into a ModelOption.
 *  Falls back to `name` as the id — needed for Ollama which returns {name} with no id. */
function toModelOption(m: unknown): ModelOption | null {
  if (typeof m === "string" && m.length > 0) return { id: m, label: m };
  if (m && typeof m === "object") {
    const obj = m as Record<string, unknown>;
    // id → name → label (Ollama returns only `name`, not `id`)
    const id =
      (typeof obj.id === "string" && obj.id.length > 0 ? obj.id : null) ??
      (typeof obj.name === "string" && obj.name.length > 0 ? obj.name : null) ??
      "";
    if (!id) return null;
    const label =
      (typeof obj.name === "string" && obj.name.length > 0 ? obj.name : null) ??
      (typeof obj.label === "string" && obj.label.length > 0 ? obj.label : null) ??
      id;
    return { id, label };
  }
  return null;
}

/** Strip whisper size suffix from display name, e.g. " (142 MB)" */
function formatWhisperName(model: unknown): string {
  const obj = model as Record<string, unknown>;
  const raw = String(obj?.name ?? obj?.filename ?? "").trim();
  if (!raw) return "";
  return raw.replace(/\s*\(\s*[0-9]+(?:\.[0-9]+)?\s*(?:kb|mb|gb)\s*\)\s*$/i, "").trim();
}

/** Returns the last path segment (filename). */
function pathBasename(p: string): string {
  return p.split("/").pop() ?? p;
}

export function ProvidersPanel({
  isOpen,
  onClose,
  sessionId,
  onShowToast,
}: ProvidersPanelProps) {
  // ── LLM state ───────────────────────────────────────────────────────────
  const [filteredProviders, setFilteredProviders] = useState<{ id: string; name: string }[]>([]);
  const [llmProvider, setLlmProvider] = useState("");
  const [llmModels, setLlmModels] = useState<ModelOption[]>([]);
  const [llmModel, setLlmModel] = useState("");
  const [llmModelsLoading, setLlmModelsLoading] = useState(false);

  // ── STT state ───────────────────────────────────────────────────────────
  const [sttProvider, setSttProvider] = useState("openai");
  const [sttModels, setSttModels] = useState<ModelOption[]>([]);
  const [sttModel, setSttModel] = useState("");
  const [sttModelsLoading, setSttModelsLoading] = useState(false);

  // ── General ─────────────────────────────────────────────────────────────
  const [loading, setLoading] = useState(false);

  // ── Load LLM models for the selected provider ────────────────────────────
  const loadModelsForProvider = useCallback(
    async (provider: string, currentModel?: string) => {
      setLlmModelsLoading(true);
      setLlmModels([]);
      try {
        let rawModels: unknown[] = [];
        // Fetch config once for all cases that need it.
        const cfg = await getConfig().catch(() => null);
        const globalLlmCfg = (cfg as Record<string, unknown> | null)?.llm as Record<string, unknown> | null;

        switch (provider) {
          case "anthropic":
          case "openai":
          case "gemini":
          case "grok": {
            // Explicitly resolve API key from keychain (matches agent.html's resolveApiKey).
            // Passing the real key avoids the unreliable sync keychain fallback in Rust.
            const apiKey = await getApiKey(provider).catch(() => null) ?? "";
            if (!apiKey) {
              setLlmModels([]);
              setLlmModel("");
              setLlmModelsLoading(false);
              onShowToast(`Set ${provider} API key to load models`, "info");
              return;
            }
            if (provider === "anthropic") rawModels = await listAnthropicModels(apiKey);
            else if (provider === "openai") rawModels = await listOpenAiModels(apiKey);
            else if (provider === "gemini") rawModels = await listGeminiModels(apiKey);
            else rawModels = await listGrokModels(apiKey);
            break;
          }
          case "ollama": {
            // Resolve Ollama endpoint from config (mirrors agent.html's resolveOllamaEndpoint).
            const ollamaSection = globalLlmCfg?.ollama as Record<string, unknown> | null;
            const endpoint =
              (ollamaSection?.base_url as string | undefined) ||
              (ollamaSection?.endpoint as string | undefined) ||
              "http://localhost:11434";
            rawModels = await listOllamaModels(endpoint);
            break;
          }
        }
        const options = rawModels.map(toModelOption).filter((m): m is ModelOption => m !== null);
        // If the saved model is not in the fetched list, inject it so the user's
        // preference is preserved rather than silently falling back to options[0].
        if (currentModel && !options.some((o) => o.id === currentModel)) {
          options.unshift({ id: currentModel, label: `${currentModel} (saved)` });
        }
        setLlmModels(options);
        const selectedId = currentModel || (options[0]?.id ?? "");
        setLlmModel(selectedId);
      } catch (e) {
        console.error("[ProvidersPanel] Failed to load models for", provider, e);
        setLlmModels([]);
      } finally {
        setLlmModelsLoading(false);
      }
    },
    [onShowToast]
  );

  // ── Load STT models for the selected provider ────────────────────────────
  const loadSttModels = useCallback(
    async (provider: string, currentModel?: string) => {
      setSttModelsLoading(true);
      setSttModels([]);
      try {
        if (provider === "local") {
          const allModels = await getWhisperModels();
          const checkedArr = await Promise.all(
            (Array.isArray(allModels) ? allModels : []).map(async (m) => {
              try {
                const obj = m as Record<string, unknown>;
                const filename = String(obj?.filename ?? obj?.name ?? "");
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
            const filename = String(obj?.filename ?? obj?.name ?? "");
            return { id: filename, label: formatWhisperName(m) || filename };
          });
          setSttModels(options);
          const normalised = currentModel ? pathBasename(currentModel) : "";
          const selectedId =
            normalised && options.some((o) => o.id === normalised)
              ? normalised
              : options[0]?.id ?? "";
          setSttModel(selectedId);
        } else if (provider === "openai") {
          const rawModels = await listOpenAiSttModels();
          const options = (Array.isArray(rawModels) ? rawModels : [])
            .map(toModelOption)
            .filter((m): m is ModelOption => m !== null);
          setSttModels(options);
          const selectedId =
            currentModel && options.some((o) => o.id === currentModel)
              ? currentModel
              : options[0]?.id ?? "";
          setSttModel(selectedId);
        } else {
          setSttModels([]);
          setSttModel("");
        }
      } catch (e) {
        console.error("[ProvidersPanel] Failed to load STT models for", provider, e);
        setSttModels([]);
      } finally {
        setSttModelsLoading(false);
      }
    },
    []
  );

  // ── Initial load when panel opens ────────────────────────────────────────
  const loadProviderSettings = useCallback(async () => {
    if (!sessionId) return;
    setLoading(true);
    try {
      const [availableProviders, sessionLlm, globalConfig, sessionVoice] = await Promise.all([
        getAvailableLlmProviders(),
        getSessionLlmConfig(sessionId).catch(() => null),
        getConfig().catch(() => null),
        getSessionVoiceConfig(sessionId).catch(() => null),
      ]);

      // Only show providers that have API keys configured
      const available = LLM_PROVIDERS.filter((p) => availableProviders[p.id] === true);
      setFilteredProviders(available);

      if (available.length > 0) {
        const llm = sessionLlm as Record<string, unknown> | null;
        const global = globalConfig as Record<string, unknown> | null;
        const globalLlm = global?.llm as Record<string, unknown> | null;

        // Session → global primary → first available
        let currentProvider =
          (llm?.provider as string) || (globalLlm?.primary as string) || "openai";
        if (!available.some((p) => p.id === currentProvider)) {
          currentProvider = available[0].id;
        }
        setLlmProvider(currentProvider);

        const providerCfg = globalLlm?.[currentProvider] as Record<string, unknown> | null;
        const globalModelForProvider = providerCfg?.model as string | undefined;
        const currentModel = (llm?.model as string) || globalModelForProvider;
        void loadModelsForProvider(currentProvider, currentModel);
      }

      // STT
      const voice = sessionVoice as Record<string, unknown> | null;
      const globalVoice =
        (globalConfig as Record<string, unknown> | null)?.voice as Record<string, unknown> | null;
      const currentSttProvider =
        (voice?.provider as string) || (globalVoice?.provider as string) || "openai";
      setSttProvider(currentSttProvider);

      let currentSttModel: string | undefined;
      if (currentSttProvider === "local") {
        currentSttModel =
          (voice?.model as string) || (globalVoice?.local_model_path as string) || undefined;
      } else {
        currentSttModel =
          (voice?.model as string) || (globalVoice?.openai_model as string) || undefined;
      }
      void loadSttModels(currentSttProvider, currentSttModel);
    } catch (e) {
      console.error("[ProvidersPanel] Failed to load provider settings:", e);
      onShowToast(`Failed to load provider settings: ${e}`, "error");
    } finally {
      setLoading(false);
    }
  }, [sessionId, loadModelsForProvider, loadSttModels, onShowToast]);

  useEffect(() => {
    if (isOpen) void loadProviderSettings();
  }, [isOpen, loadProviderSettings]);

  // ── LLM provider change ──────────────────────────────────────────────────
  const handleLlmProviderChange = useCallback(
    async (provider: string) => {
      setLlmProvider(provider);
      setLlmModel("");
      try {
        await setSessionLlmProvider(sessionId, provider);
        const globalConfig = await getConfig().catch(() => null);
        const globalLlm =
          (globalConfig as Record<string, unknown> | null)?.llm as Record<string, unknown> | null;
        const globalModel =
          (globalLlm?.[provider] as Record<string, unknown> | null)?.model as string | undefined;
        await loadModelsForProvider(provider, globalModel);
      } catch (e) {
        console.error("[ProvidersPanel] Failed to set LLM provider:", e);
        onShowToast(`Failed to update provider: ${e}`, "error");
      }
    },
    [sessionId, loadModelsForProvider, onShowToast]
  );

  // ── LLM model change ─────────────────────────────────────────────────────
  const handleLlmModelChange = useCallback(
    async (model: string) => {
      if (!model) return;
      setLlmModel(model);
      try {
        await setSessionLlmModel(sessionId, model);
        onShowToast("Model updated", "success");
      } catch (e) {
        console.error("[ProvidersPanel] Failed to set LLM model:", e);
        onShowToast(`Failed to update model: ${e}`, "error");
      }
    },
    [sessionId, onShowToast]
  );

  // ── STT provider change ──────────────────────────────────────────────────
  const handleSttProviderChange = useCallback(
    async (provider: string) => {
      setSttProvider(provider);
      setSttModel("");
      try {
        await setSessionVoiceProvider(sessionId, provider);
        await loadSttModels(provider);
        onShowToast("STT provider updated for this session", "success");
      } catch (e) {
        console.error("[ProvidersPanel] Failed to set STT provider:", e);
        onShowToast(`Failed to update STT provider: ${e}`, "error");
      }
    },
    [sessionId, loadSttModels, onShowToast]
  );

  // ── STT model change ─────────────────────────────────────────────────────
  const handleSttModelChange = useCallback(
    async (model: string) => {
      if (!model) return;
      setSttModel(model);
      try {
        await setSessionVoiceModel(sessionId, model);
        onShowToast("STT model updated for this session", "success");
      } catch (e) {
        console.error("[ProvidersPanel] Failed to set STT model:", e);
        onShowToast(`Failed to update STT model: ${e}`, "error");
      }
    },
    [sessionId, onShowToast]
  );

  // ── Reset to global defaults ─────────────────────────────────────────────
  const handleReset = useCallback(async () => {
    try {
      await Promise.all([
        clearSessionLlmConfig(sessionId),
        clearSessionVoiceConfig(sessionId),
      ]);
      await loadProviderSettings();
      onShowToast("Reset to global defaults", "success");
    } catch (e) {
      onShowToast(`Failed to reset: ${e}`, "error");
    }
  }, [sessionId, loadProviderSettings, onShowToast]);

  return (
    <>
      <div
        className={`session-panel-overlay${isOpen ? " visible" : ""}`}
        onClick={onClose}
      />
      <div className={`session-panel${isOpen ? " open" : ""}`}>
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
              {/* LLM Provider Section */}
              <div className="providers-section">
                <div className="providers-section-header">
                  <span className="icon-cpu-chip" />
                  <span>LLM Provider</span>
                </div>
                <p className="providers-section-info">
                  Select the AI model provider and model for this session.
                </p>
                {filteredProviders.length === 0 ? (
                  <p className="providers-section-info" style={{ color: "var(--color-danger, #f87171)" }}>
                    No providers configured. Add API keys in Settings.
                  </p>
                ) : (
                  <>
                    <div className="session-field">
                      <label>Provider</label>
                      <select
                        className="provider-select"
                        value={llmProvider}
                        onChange={(e) => void handleLlmProviderChange(e.target.value)}
                      >
                        {filteredProviders.map((p) => (
                          <option key={p.id} value={p.id}>{p.name}</option>
                        ))}
                      </select>
                    </div>
                    <div className="session-field">
                      <label>Model</label>
                      <select
                        className="provider-select"
                        value={llmModel}
                        disabled={llmModelsLoading || llmModels.length === 0}
                        onChange={(e) => void handleLlmModelChange(e.target.value)}
                      >
                        {llmModelsLoading ? (
                          <option value="">Loading models…</option>
                        ) : llmModels.length === 0 ? (
                          <option value="">No models available</option>
                        ) : (
                          llmModels.map((m) => (
                            <option key={m.id} value={m.id}>{m.label}</option>
                          ))
                        )}
                      </select>
                    </div>
                  </>
                )}
              </div>

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
                        {sttProvider === "local"
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
                  style={{ width: "100%" }}
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

