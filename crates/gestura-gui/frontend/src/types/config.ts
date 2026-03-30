// Shared TypeScript types for Gestura configuration
// This should match the Rust AppConfig struct in src-tauri/src/config.rs

export interface UiSettings {
  theme_mode: string;
  accent?: string;
}

export interface PrivacySettings {
  data_collection: boolean;
  crash_reports: boolean;
  voice_data_local: boolean;
  require_auth: boolean;
  auth_timeout: number;
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
  /** Whether tools from this server are enabled by default for new sessions. */
  session_default_enabled?: boolean;
}

/** @deprecated Use McpServerEntry instead. Kept for backward compat. */
export interface McpTool {
  name: string;
  endpoint: string;
}

export interface ReflectionSettings {
  enabled: boolean;
  quality_threshold_percent: number;
  max_injected: number;
  max_retry_attempts: number;
  promotion_confidence_percent: number;
}

export type AgentTelemetryTraceExportProtocol = 'http' | 'grpc';

export interface AgentTelemetryTraceExportSettings {
  enabled: boolean;
  protocol: AgentTelemetryTraceExportProtocol;
  endpoint: string;
}

/**
 * Opt-in request tracing for the core agent loop.
 *
 * This mirrors `pipeline.agent_telemetry` in Rust and lets the UI expose a
 * durable toggle without needing to know the underlying metric names.
 */
export interface AgentTelemetrySettings {
  enabled: boolean;
  trace_export: AgentTelemetryTraceExportSettings;
}

export type CompactionStrategy = 'Summarize' | 'Truncate' | 'Clear' | 'Prompt' | 'MemoryBank';

export interface PipelineSettings {
  max_history_messages: number;
  iteration_budget_enabled: boolean;
  max_iterations: number;
  tracked_task_max_iterations: number;
  auto_compact_threshold_percent: number;
  compaction_strategy: CompactionStrategy;
  max_context_tokens: number;
  log_token_usage: boolean;
  /** Request-level telemetry emitted across analysis, context, loop, and reflection phases. */
  agent_telemetry: AgentTelemetrySettings;
  reflection: ReflectionSettings;
}

export interface AppConfig {
  hotkey_listen: string;
  hotkey_new_session: string;
  grace_period_secs: number;
  voice: VoiceSettings;
  llm: LlmSettings;
  ui: UiSettings;
  privacy: PrivacySettings;
  mcp_servers: McpServerEntry[];
  mdh_pointers: Record<string, string>;
  nats_url: string;
  developer: DeveloperSettings;
  pipeline: PipelineSettings;
}
