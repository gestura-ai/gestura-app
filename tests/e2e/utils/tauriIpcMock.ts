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
      },
    };

    const loadConfig = () => {
      try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return { ...defaultConfig };
        const parsed = JSON.parse(raw);
        return { ...defaultConfig, ...parsed };
      } catch {
        return { ...defaultConfig };
      }
    };

    const saveConfig = (cfg) => {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(cfg));
    };

    const delay = (ms) => new Promise((r) => setTimeout(r, ms));

    const w = window;
    w.__TAURI_INTERNALS__ ??= {};
    const internals = w.__TAURI_INTERNALS__;

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

    internals.invoke = async (cmd, args) => {
      switch (cmd) {
        case 'get_config':
          return loadConfig();

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

        case 'list_agents':
          return {
            agents: [{ id: 'agent-e2e-001', name: 'E2E Agent', status: 'active' }],
            count: 1,
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

