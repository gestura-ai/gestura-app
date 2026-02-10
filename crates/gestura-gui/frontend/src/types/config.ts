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
  fallback?: string | null;
  openai?: OpenAiConfig;
  anthropic?: AnthropicConfig;
  grok?: GrokConfig;
  ollama?: OllamaConfig;
}

export interface OpenAiConfig {
  api_key: string;
  base_url?: string;
  model: string;
}

export interface AnthropicConfig {
  api_key: string;
  base_url?: string;
  model: string;
  thinking_budget_tokens?: number | null;
}

export interface GrokConfig {
  api_key: string;
  base_url?: string;
  model: string;
}

export interface OllamaConfig {
  base_url: string;
  model: string;
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

/** Transport type for MCP server connections. */
export type McpTransportType = 'stdio' | 'http' | 'sse';

/** Configuration scope for MCP servers. */
export type McpScope = 'user' | 'project' | 'local';

/**
 * Full MCP server configuration entry (Claude Code compatible).
 *
 * Supports all three transport types with their respective config fields.
 */
export interface McpServerEntry {
  /** Unique server name / identifier. */
  name: string;
  /** Transport type. Serialized as `type` in JSON. */
  type: McpTransportType;
  /** Whether this server is enabled. */
  enabled: boolean;
  // -- stdio-specific --
  /** Command to execute (stdio transport). */
  command?: string;
  /** Command arguments (stdio transport). */
  args?: string[];
  /** Environment variables (stdio transport). */
  env?: Record<string, string>;
  // -- http/sse-specific --
  /** Server URL (http/sse transport). */
  url?: string;
  /** Custom HTTP headers (http/sse transport). */
  headers?: Record<string, string>;
  // -- common --
  /** Configuration scope. */
  scope: McpScope;
  /** Connection timeout in seconds. */
  timeout_secs: number;
  /** Auto-reconnect on failure. */
  auto_reconnect: boolean;
}

/** @deprecated Use McpServerEntry instead. Kept for backward compat. */
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
  mcp_servers: McpServerEntry[];
  mdh_pointers: Record<string, string>;
  nats_url: string;
  developer: DeveloperSettings;
  pipeline: PipelineSettings;
}
