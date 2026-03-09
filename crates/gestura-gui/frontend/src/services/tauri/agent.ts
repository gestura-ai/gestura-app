/**
 * Typed wrappers for all agent-related Tauri commands.
 * Mirrors the invoke calls found in agent.html.
 */
import { invokeTauri } from './invoke';
import type {
  TaskHierarchy,
  Task,
  TaskStatus,
  KnowledgeItem,
  ToolConfirmationDecision,
} from '../../features/agent/types';

// ─── Streaming ────────────────────────────────────────────────────────────────

export interface SendMessageArgs {
  session_id: string;
  message: string;
  task_id?: string | null;
}

export const sendMessageStreaming = (args: SendMessageArgs): Promise<void> =>
  invokeTauri('process_agent_message_streaming', { ...args });

export const cancelStreaming = (sessionId: string): Promise<void> =>
  invokeTauri('cancel_agent_streaming', { session_id: sessionId });

// ─── Session history ──────────────────────────────────────────────────────────

export interface HistoryMessage {
  role: string;
  content: string;
}

export const getSessionHistory = (sessionId: string): Promise<HistoryMessage[]> =>
  invokeTauri('get_session_history', { session_id: sessionId });

// ─── Config ───────────────────────────────────────────────────────────────────

export const getConfig = (): Promise<Record<string, unknown>> =>
  invokeTauri('get_config');

export const saveConfig = (config: Record<string, unknown>): Promise<void> =>
  invokeTauri('save_config', { config });

// ─── LLM providers ───────────────────────────────────────────────────────────

export const getEffectiveLlmConfig = (sessionId: string): Promise<Record<string, unknown>> =>
  invokeTauri('get_effective_llm_config', { session_id: sessionId });

export const getAvailableLlmProviders = (): Promise<Record<string, boolean>> =>
  invokeTauri('get_available_llm_providers');

export const getSessionLlmConfig = (sessionId: string): Promise<Record<string, unknown>> =>
  invokeTauri('get_session_llm_config', { session_id: sessionId });

export const setSessionLlmProvider = (sessionId: string, provider: string): Promise<void> =>
  invokeTauri('set_session_llm_provider', { session_id: sessionId, provider });

export const setSessionLlmModel = (sessionId: string, model: string): Promise<void> =>
  invokeTauri('set_session_llm_model', { session_id: sessionId, model });

export const clearSessionLlmConfig = (sessionId: string): Promise<void> =>
  invokeTauri('clear_session_llm_config', { session_id: sessionId });

// Callers must resolve the API key via getApiKey() first and pass it explicitly.
// Rust falls back to keychain when api_key is empty, but the sync keychain path is
// unreliable inside an async command — explicit key is always preferred (matches agent.html).
export const listAnthropicModels = (apiKey: string): Promise<unknown[]> =>
  invokeTauri('list_anthropic_models', { api_key: apiKey });

export const listOpenAiModels = (apiKey: string): Promise<unknown[]> =>
  invokeTauri('list_openai_models', { api_key: apiKey });

export const listGeminiModels = (apiKey: string): Promise<unknown[]> =>
  invokeTauri('list_gemini_models', { api_key: apiKey });

export const listGrokModels = (apiKey: string): Promise<unknown[]> =>
  invokeTauri('list_grok_models', { api_key: apiKey });

// Rust expects `{ endpoint }` — required, no default.
export const listOllamaModels = (endpoint: string): Promise<unknown[]> =>
  invokeTauri('list_ollama_models', { endpoint });

export const updateLlmProvider = (provider: string, settings: Record<string, unknown>): Promise<void> =>
  invokeTauri('update_llm_provider', { provider, settings });

// ─── API keys ─────────────────────────────────────────────────────────────────

export const getApiKey = (provider: string): Promise<string | null> =>
  invokeTauri('get_api_key', { provider });

// ─── Prompt enhancement ───────────────────────────────────────────────────────

export const enhancePrompt = (sessionId: string, text: string): Promise<string> =>
  invokeTauri('enhance_prompt', { session_id: sessionId, text });

// ─── Voice ───────────────────────────────────────────────────────────────────

export const startVoiceListening = (sessionId: string): Promise<void> =>
  invokeTauri('start_voice_listening', { session_id: sessionId });

export const stopVoiceListening = (sessionId: string): Promise<void> =>
  invokeTauri('stop_voice_listening', { session_id: sessionId });

export const getSessionVoiceConfig = (sessionId: string): Promise<Record<string, unknown>> =>
  invokeTauri('get_session_voice_config', { session_id: sessionId });

export const setSessionVoiceProvider = (sessionId: string, provider: string): Promise<void> =>
  invokeTauri('set_session_voice_provider', { session_id: sessionId, provider });

export const setSessionVoiceModel = (sessionId: string, model: string): Promise<void> =>
  invokeTauri('set_session_voice_model', { session_id: sessionId, model });

export const clearSessionVoiceConfig = (sessionId: string): Promise<void> =>
  invokeTauri('clear_session_voice_config', { session_id: sessionId });

export const getWhisperModels = (): Promise<unknown[]> =>
  invokeTauri('get_whisper_models');

// Rust returns { exists: boolean, path: string, is_valid: boolean, ... }
export const isWhisperModelDownloaded = (
  modelFilename: string
): Promise<{ exists: boolean; path: string; is_valid: boolean }> =>
  invokeTauri('is_whisper_model_downloaded', { model_filename: modelFilename });

// Rust expects api_key (falls back to keychain when empty).
export const listOpenAiSttModels = (): Promise<unknown[]> =>
  invokeTauri('list_openai_stt_models', { api_key: '' });

// ─── Tasks ────────────────────────────────────────────────────────────────────

export const getTaskHierarchy = (sessionId: string): Promise<TaskHierarchy> =>
  invokeTauri('get_task_hierarchy', { session_id: sessionId });

export const createTask = (sessionId: string, name: string, description?: string | null): Promise<Task> =>
  invokeTauri('create_task', { session_id: sessionId, name, description: description ?? null });

export const updateTask = (sessionId: string, taskId: string, name?: string, description?: string | null): Promise<Task> =>
  invokeTauri('update_task', { session_id: sessionId, task_id: taskId, name, description: description ?? null });

export const updateTaskStatus = (sessionId: string, taskId: string, status: TaskStatus): Promise<void> =>
  invokeTauri('update_task_status', { session_id: sessionId, task_id: taskId, status });

export const deleteTask = (sessionId: string, taskId: string): Promise<void> =>
  invokeTauri('delete_task', { session_id: sessionId, task_id: taskId });

export const breakDownRequirements = (sessionId: string, requirements: string): Promise<Task[]> =>
  invokeTauri('break_down_requirements', { session_id: sessionId, requirements });

// ─── Knowledge ────────────────────────────────────────────────────────────────

export const listKnowledgeItems = (): Promise<KnowledgeItem[]> =>
  invokeTauri('list_knowledge_items');

export const getEnabledKnowledge = (sessionId: string): Promise<string[]> =>
  invokeTauri('get_enabled_knowledge', { session_id: sessionId });

export const setKnowledgeEnabled = (sessionId: string, id: string, enabled: boolean): Promise<void> =>
  invokeTauri('set_knowledge_enabled', { session_id: sessionId, knowledge_id: id, enabled });

// ─── Tools / permissions ──────────────────────────────────────────────────────

export const getSessionToolSettings = (sessionId: string): Promise<Record<string, unknown>> =>
  invokeTauri('get_session_tool_settings', { session_id: sessionId });

export const setSessionToolEnabled = (sessionId: string, toolName: string, enabled: boolean): Promise<void> =>
  invokeTauri('set_session_tool_enabled', { session_id: sessionId, tool_name: toolName, enabled });

export const setSessionPermissionLevel = (sessionId: string, level: string): Promise<void> =>
  invokeTauri('set_session_permission_level', { session_id: sessionId, level });

export const listBuiltinTools = (): Promise<unknown[]> =>
  invokeTauri('list_builtin_tools');

export const initMcpServers = (sessionId: string): Promise<void> =>
  invokeTauri('init_mcp_servers', { session_id: sessionId });

export const listDiscoveredMcpTools = (sessionId: string): Promise<unknown[]> =>
  invokeTauri('list_discovered_mcp_tools', { session_id: sessionId });

// ─── Tool confirmation ────────────────────────────────────────────────────────

export const resolveToolConfirmationDecision = (
  confirmationId: string,
  decision: ToolConfirmationDecision,
): Promise<void> =>
  invokeTauri('resolve_tool_confirmation_decision', { confirmation_id: confirmationId, decision });

export const approveToolConfirmation = (confirmationId: string): Promise<void> =>
  invokeTauri('approve_tool_confirmation', { confirmation_id: confirmationId });

export const denyToolConfirmation = (confirmationId: string): Promise<void> =>
  invokeTauri('deny_tool_confirmation', { confirmation_id: confirmationId });

// ─── Workspace / shell ────────────────────────────────────────────────────────

export const pickWorkspaceDirectory = (sessionId: string): Promise<string | null> =>
  invokeTauri('pick_workspace_directory', { session_id: sessionId });

export const getSessionWorkspaceById = (sessionId: string): Promise<string | null> =>
  invokeTauri('get_session_workspace_by_id', { session_id: sessionId });

export const openShellForSession = (sessionId: string): Promise<void> =>
  invokeTauri('open_shell_for_session', { session_id: sessionId });

export const shellProcessStop = (processId: string): Promise<void> =>
  invokeTauri('shell_process_stop', { process_id: processId });

export const shellProcessPause = (processId: string): Promise<void> =>
  invokeTauri('shell_process_pause', { process_id: processId });

export const shellProcessResume = (processId: string): Promise<void> =>
  invokeTauri('shell_process_resume', { process_id: processId });

// ─── Debug / diagnostics ──────────────────────────────────────────────────────

export const recordAgentReceipt = (payload: string): Promise<void> =>
  invokeTauri('record_agent_receipt', { payload });

export const clearAgentEventTrace = (): Promise<void> =>
  invokeTauri('clear_agent_event_trace');

export const getAgentEventTrace = (): Promise<unknown[]> =>
  invokeTauri('get_agent_event_trace');

