// Shared TypeScript types for Gestura configuration
// This should match the Rust AppConfig struct in src-tauri/src/config.rs

export interface UiSettings {
  theme_mode: string;
  accent?: string;
}

export interface VoiceSettings {
  provider: string;
  input_path?: string;
  local_model_path?: string;
  openai_api_key?: string;
  openai_base_url?: string;
  openai_model?: string;
}

export interface LlmSettings {
  primary: string;
  openai?: any;
  anthropic?: any;
  grok?: any;
  ollama?: any;
}

export interface SimulatorSettings {
  device_name_pattern: string;
  auto_connect: boolean;
  health_check_interval: number;
  enable_metrics: boolean;
  discovery_port_range: [number, number];
}

export interface DeveloperSettings {
  developer_mode: boolean;
  enable_simulators: boolean;
  auto_discover_simulators: boolean;
  verbose_ble_logging: boolean;
  simulator: SimulatorSettings;
}

export interface McpTool {
  name: string;
  endpoint: string;
}

export interface PipelineSettings {
  max_history_messages: number;
  auto_compact_threshold_percent: number;
  compaction_strategy: string;
  max_context_tokens: number;
  log_token_usage: boolean;
}

export interface AppConfig {
  hotkey_listen: string;
  grace_period_secs: number;
  voice: VoiceSettings;
  llm: LlmSettings;
  ui: UiSettings;
  mcp_tools: McpTool[];
  mdh_pointers: Record<string, string>;
  nats_url: string;
  developer: DeveloperSettings;
  pipeline: PipelineSettings;
}
