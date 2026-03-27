interface SessionLlmConfigChangedDetail {
  sessionId: string;
}

export const SESSION_LLM_CONFIG_CHANGED_EVENT = 'gestura:session-llm-config-changed';

export function dispatchSessionLlmConfigChanged(sessionId: string): void {
  if (typeof window === 'undefined' || !sessionId) return;

  window.dispatchEvent(new CustomEvent<SessionLlmConfigChangedDetail>(
    SESSION_LLM_CONFIG_CHANGED_EVENT,
    { detail: { sessionId } },
  ));
}

export function readSessionLlmConfigChangedDetail(event: Event): SessionLlmConfigChangedDetail | null {
  return (event as CustomEvent<SessionLlmConfigChangedDetail>).detail ?? null;
}
