import { type Page } from '@playwright/test';

/**
 * Install a deterministic mock for the Tauri IPC bridge so the GUI can boot in
 * Playwright's regular browser context.
 */
export async function installTauriIpcMock(page: Page): Promise<void> {
  await page.addInitScript(() => {
    // The Tauri JS APIs call into `window.__TAURI_INTERNALS__.invoke(...)`.
    // In Playwright (browser) runs, that bridge does not exist, so we mock it.
    // Ref: @tauri-apps/api/core invoke() implementation.
    // NOTE: This function is executed in the *browser context*.
    // Keep it plain JavaScript (no TypeScript annotations / assertions).
    // eslint-disable-next-line no-undef
    globalThis.isTauri = true;

    const STORAGE_KEY = '__gestura_e2e_config__';
    const API_KEYS_KEY = '__gestura_e2e_api_keys__';

    const defaultConfig = {
      hotkey_listen: 'Ctrl+Space',
      grace_period_secs: 30,
      voice: {
        provider: 'local',
        input_path: '',
        local_model_path: '',
        openai_api_key: '',
        openai_base_url: '',
        openai_model: 'whisper-1',
      },
      llm: {
        primary: 'openai',
        fallback: null,
        openai: { api_key: '', base_url: '', model: 'gpt-4o-mini' },
      },
      ui: { theme_mode: 'system', accent: 'blue' },
      mcp_servers: [],
      mdh_pointers: {},
      nats_url: 'nats://127.0.0.1:4222',
      developer: {
        developer_mode: false,
        enable_simulators: true,
        auto_discover_simulators: false,
        verbose_ble_logging: false,
        simulator: {
          device_name_pattern: 'Gestura Simulator',
          auto_connect: true,
          health_check_interval: 5,
          enable_metrics: false,
          discovery_port_range: [40000, 40100],
        },
      },
      pipeline: {
        max_history_messages: 10,
        auto_compact_threshold_percent: 80,
        compaction_strategy: 'Summarize',
        max_context_tokens: 0,
        log_token_usage: false,
        reflection: {
          enabled: false,
          quality_threshold_percent: 60,
          max_injected: 3,
          max_retry_attempts: 1,
          promotion_confidence_percent: 75,
        },
      },
    };

    const loadConfig = () => {
      try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return { ...defaultConfig };
        const parsed = JSON.parse(raw);
        return {
          ...defaultConfig,
          ...parsed,
          voice: { ...defaultConfig.voice, ...(parsed?.voice || {}) },
          llm: { ...defaultConfig.llm, ...(parsed?.llm || {}) },
          ui: { ...defaultConfig.ui, ...(parsed?.ui || {}) },
          developer: {
            ...defaultConfig.developer,
            ...(parsed?.developer || {}),
            simulator: {
              ...defaultConfig.developer.simulator,
              ...(parsed?.developer?.simulator || {}),
            },
          },
          pipeline: {
            ...defaultConfig.pipeline,
            ...(parsed?.pipeline || {}),
            reflection: {
              ...defaultConfig.pipeline.reflection,
              ...(parsed?.pipeline?.reflection || {}),
            },
          },
        };
      } catch {
        return { ...defaultConfig };
      }
    };

    const saveConfig = (cfg) => {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(cfg));
    };

    const loadApiKeys = () => {
      try {
        const raw = localStorage.getItem(API_KEYS_KEY);
        if (!raw) return {};
        const parsed = JSON.parse(raw);
        return parsed && typeof parsed === 'object' ? parsed : {};
      } catch {
        return {};
      }
    };

    const saveApiKeys = (keys) => {
      localStorage.setItem(API_KEYS_KEY, JSON.stringify(keys || {}));
    };

    const delay = (ms) => new Promise((r) => setTimeout(r, ms));

    const w = window;
    w.__TAURI_INTERNALS__ ??= {};
    const internals = w.__TAURI_INTERNALS__;

    // ---------------------------------------------------------------------
    // Provide a minimal `window.__TAURI__` surface for legacy/static pages
    // (e.g. `public/agent.html`) that call `window.__TAURI__.core.invoke(...)`.
    //
    // The React app typically uses `@tauri-apps/api`, which goes through
    // `window.__TAURI_INTERNALS__`, so we support both.
    // ---------------------------------------------------------------------

    // Minimal callback registry (needed for Channel/event plumbing safety).
    let nextCallbackId = 1;
    const callbacks = new Map();
    internals.transformCallback = (cb, once = false) => {
      const id = nextCallbackId++;
      callbacks.set(id, (data) => {
        if (once) callbacks.delete(id);
        if (typeof cb === 'function') cb(data);
      });
      return id;
    };
    internals.unregisterCallback = (id) => callbacks.delete(id);
    internals.runCallback = (id, data) => callbacks.get(id)?.(data);
    internals.callbacks = callbacks;
    internals.convertFileSrc = (filePath, protocol = 'asset') => {
      return `${protocol}://localhost/${encodeURIComponent(filePath)}`;
    };

    // Mirror internals under `window.__TAURI__` for pages that use the global.
    // Keep this very small: only what our static windows reference.
    w.__TAURI__ ??= {};
    w.__TAURI__.core ??= {};
    w.__TAURI__.core.invoke = (cmd, args) => internals.invoke(cmd, args);
    w.__TAURI__.core.convertFileSrc = (filePath) => internals.convertFileSrc(filePath);
    w.__TAURI__.event ??= {};
    w.__TAURI__.event.listen ??= async () => {
      // Return an unlisten handle.
      return () => { };
    };
    w.__TAURI__.webviewWindow ??= {};
    w.__TAURI__.webviewWindow.getCurrentWebviewWindow ??= () => {
      return {
        label: 'e2e-webview',
        listen: async () => {
          return () => { };
        },
      };
    };
    w.__TAURI__.opener ??= {};
    w.__TAURI__.opener.openUrl ??= async () => null;

    internals.invoke = async (cmd, args) => {
      switch (cmd) {
        case 'set_window_size': {
          const width = Number(args?.width ?? 0);
          const height = Number(args?.height ?? 0);

          // Record calls for optional assertions/debugging.
          const w = window as unknown as {
            __gestura_e2e_window_size_calls__?: Array<{ width: number; height: number }>;
          };
          if (!Array.isArray(w.__gestura_e2e_window_size_calls__)) {
            w.__gestura_e2e_window_size_calls__ = [];
          }
          w.__gestura_e2e_window_size_calls__.push({ width, height });

          return null;
        }

        case 'get_config':
          return loadConfig();

        case 'get_effective_llm_config': {
          // agent.html expects a tuple: [provider, model]
          const cfg = loadConfig();
          const provider = cfg?.llm?.primary || 'openai';
          const model = cfg?.llm?.[provider]?.model || '';
          return [provider, model];
        }

        case 'get_api_key':
          // For e2e, behave like an empty keychain.
          return '';

        case 'save_config': {
          const cfg = args?.cfg;
          saveConfig(cfg);
          return null;
        }

        case 'set_ui_prefs': {
          const cfg = loadConfig();
          const ui = args?.ui;
          const next = { ...cfg, ui: { ...cfg.ui, ...(ui || {}) } };
          saveConfig(next);
          return null;
        }

        case 'update_llm_provider': {
          const cfg = loadConfig();
          const provider = args?.provider ?? 'ollama';
          const next = { ...cfg, llm: { ...(cfg.llm || {}), primary: provider } };
          saveConfig(next);
          return null;
        }

        case 'update_ollama_config': {
          const cfg = loadConfig();
          const baseUrl = args?.baseUrl ?? args?.base_url ?? 'http://localhost:11434';
          const model = args?.model ?? 'llama3.2';
          const next = {
            ...cfg,
            llm: {
              ...(cfg.llm || {}),
              ollama: { base_url: baseUrl, model },
            },
          };
          saveConfig(next);
          return null;
        }

        case 'has_api_key': {
          const provider = String(args?.provider ?? '').toLowerCase();
          const keys = loadApiKeys();
          return Boolean(keys[provider]);
        }

        case 'store_api_key': {
          const provider = String(args?.provider ?? '').toLowerCase();
          const apiKey = String(args?.api_key ?? '');
          const keys = loadApiKeys();
          if (provider && apiKey) {
            keys[provider] = apiKey;
            saveApiKeys(keys);
          }
          return null;
        }

        case 'list_audio_devices':
          return [
            { name: 'E2E Microphone', is_default: true },
            { name: 'E2E Microphone (Alt)', is_default: false },
          ];

        case 'update_audio_device': {
          const cfg = loadConfig();
          const deviceName = args?.device_name ?? '';
          const next = { ...cfg, voice: { ...(cfg.voice || {}), audio_device: deviceName } };
          saveConfig(next);
          return null;
        }

        case 'update_voice_provider': {
          const cfg = loadConfig();
          const provider = args?.provider ?? 'local';
          const next = { ...cfg, voice: { ...(cfg.voice || {}), provider } };
          saveConfig(next);
          return null;
        }

        case 'update_whisper_model': {
          const cfg = loadConfig();
          const modelFilename = args?.model_filename ?? 'ggml-base.en.bin';
          // The real backend stores a full path; for tests a filename is sufficient.
          const next = {
            ...cfg,
            voice: { ...(cfg.voice || {}), local_model_path: modelFilename },
          };
          saveConfig(next);
          return null;
        }

        case 'list_agents':
          return {
            agents: [{ id: 'agent-e2e-001', name: 'E2E Agent', status: 'active' }],
            count: 1,
          };

        case 'get_session_history':
          return [];

        case 'get_session_tool_settings':
          return {
            permission_level: 'ReadOnly',
            enabled_tools: [],
          };

        case 'init_mcp_servers':
          return null;

        case 'list_discovered_mcp_tools':
          return [];

        case 'explorer_get_root':
          return {
            root: '/mock/project',
            is_git_repo: true,
          };

        case 'explorer_list_dir': {
          const dirRel = String(args?.dir_rel ?? '');

          if (dirRel === '') {
            return {
              truncated: false,
              entries: [
                { name: 'src', rel_path: 'src', kind: 'dir' },
                { name: 'README.md', rel_path: 'README.md', kind: 'file' },
                { name: 'Cargo.toml', rel_path: 'Cargo.toml', kind: 'file' },
              ],
            };
          }

          if (dirRel === 'src') {
            return {
              truncated: false,
              entries: [
                { name: 'main.rs', rel_path: 'src/main.rs', kind: 'file' },
                { name: 'lib.rs', rel_path: 'src/lib.rs', kind: 'file' },
              ],
            };
          }

          return { truncated: false, entries: [] };
        }

        case 'explorer_git_status':
          return {
            paths: {
              'src/main.rs': { staged: null, unstaged: 'modified', untracked: false },
              'README.md': { staged: null, unstaged: null, untracked: true },
            },
          };

        case 'list_active_tasks':
          return [];

        case 'get_nats_status':
          return true;

        case 'scan_for_rings':
          // Ensure UI shows a deterministic scanning state.
          await delay(250);
          return ['RING-001'];

        case 'scan_for_simulators':
          await delay(150);
          return ['mock-simulator-001'];

        case 'get_simulator_logs':
          return ['[e2e] simulator log line 1', '[e2e] simulator log line 2'];

        case 'is_developer_mode_enabled':
          return false;

        case 'get_ring_status': {
          const deviceId = args?.device_id ?? 'RING-001';
          return {
            device_id: deviceId,
            is_connected: true,
            battery_level: 80,
            firmware_version: '0.0.0-e2e',
            last_seen: new Date().toISOString(),
          };
        }

        case 'pair_ring':
        case 'send_haptic_feedback':
        case 'start_gesture_monitoring':
        case 'stop_gesture_monitoring':
        case 'reset_simulator':
        case 'send_test_haptic':
        case 'toggle_developer_mode':
        case 'add_mcp_tool':
        case 'remove_mcp_tool':
        case 'disconnect_mcp_server':
        case 'cancel_task':
        case 'spawn_subagent':
        case 'open_system_preferences':
        case 'request_permission':
        case 'register_consent':
        case 'complete_onboarding':
        case 'close_onboarding_window':
          return null;

        case 'connect_mcp_server':
          return [];

        case 'delegate_task':
          return 'task-e2e-001';

        case 'list_builtin_tools':
        case 'list_mcp_tools':
          return [];

        case 'get_mcp_server_status':
        case 'list_connected_mcp_servers':
        case 'list_mcp_client_tools':
          return [];

        case 'call_mcp_tool':
          return { ok: true, echo: args ?? null };

        case 'run_simulator_test':
          return { connectivity: true, latency_ms: 15.5, haptic_tests: [] };

        case 'check_system_permissions':
          return { microphone: true, accessibility: true, bluetooth: true };

        case 'test_voice':
          await delay(300);
          return 'ok';

        case 'run_voice_once':
          await delay(300);
          return 'hello world';

        default:
          // For newly-added commands, fail closed so tests catch missing mocks.
          throw new Error(`[E2E] Unmocked tauri command: ${cmd}`);
      }
    };
  });
}

/** Mark onboarding as completed so the app shell remains interactable. */
export async function setOnboardingComplete(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.setItem('gestura_onboarding_completed', 'true');
  });
}

/** Ensure onboarding is *not* completed (used by onboarding-gate tests). */
export async function clearOnboardingComplete(page: Page): Promise<void> {
  await page.addInitScript(() => {
    localStorage.removeItem('gestura_onboarding_completed');
  });
}

