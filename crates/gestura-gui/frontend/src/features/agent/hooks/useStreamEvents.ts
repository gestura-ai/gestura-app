/**
 * useStreamEvents — subscribes to all agent Tauri streaming events.
 *
 * Accepts a dispatch callback so callers (useChatSession) can maintain
 * full control over the message state model. Session isolation is enforced
 * here: events that carry a different session_id are silently dropped.
 */
import { useEffect } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import type { UnlistenFn } from '@tauri-apps/api/event';
import type {
  ToolConfirmation,
  StreamHealthPayload,
} from '../types';

// ─── Payload helpers ──────────────────────────────────────────────────────────

interface UnpackedPayload<T = unknown> {
  incomingSessionId: string | null;
  value: T;
}

function unpackPayload<T = unknown>(raw: unknown): UnpackedPayload<T> {
  if (!raw || typeof raw !== 'object') {
    return { incomingSessionId: null, value: raw as T };
  }
  const obj = raw as Record<string, unknown>;
  return {
    incomingSessionId: (obj['session_id'] as string | null | undefined) ?? null,
    value: (obj['value'] !== undefined ? obj['value'] : raw) as T,
  };
}

// ─── Event type discriminants ─────────────────────────────────────────────────

export type StreamEventAction =
  | { type: 'thinking'; chunk: string }
  | { type: 'chunk'; chunk: string }
  | { type: 'tool-confirmation'; payload: ToolConfirmation }
  | { type: 'tool-blocked'; toolName: string; reason: string }
  | { type: 'agent-iteration'; iteration: number }
  | { type: 'tool-start'; toolName: string }
  | { type: 'tool-args'; args: string }
  | { type: 'tool-end' }
  | { type: 'tool-result'; name: string; success: boolean; output: string | null; durationMs: number | null }
  | { type: 'shell-lifecycle'; processId: string; payload: Record<string, unknown> }
  | { type: 'shell-output'; processId: string; stream: 'Stdout' | 'Stderr'; data: string }
  | { type: 'retry'; attempt: number; reason: string }
  | { type: 'status'; text: string; kind: string }
  | { type: 'context-compacted'; summary: string }
  | { type: 'done' }
  | { type: 'cancelled' }
  | { type: 'error'; message: string }
  | { type: 'health'; payload: StreamHealthPayload }
  | { type: 'agent-message'; role: string; content: string }
  | { type: 'listening-state'; listening: boolean }
  | { type: 'task-changed' };

export type StreamEventDispatch = (action: StreamEventAction) => void;

// ─── Hook ─────────────────────────────────────────────────────────────────────

/**
 * Subscribe to all agent streaming events for the given session.
 * @param sessionId - Session to filter events for.
 * @param dispatch  - Called for every accepted event.
 */
export function useStreamEvents(sessionId: string, dispatch: StreamEventDispatch): void {
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    const unlisten: UnlistenFn[] = [];

    function accept<T = unknown>(eventName: string, raw: unknown): { ok: true; value: T } | { ok: false } {
      const { incomingSessionId, value } = unpackPayload<T>(raw);
      if (sessionId && (!incomingSessionId || incomingSessionId !== sessionId)) {
        return { ok: false };
      }
      void eventName;
      return { ok: true, value };
    }

    async function setup() {
      unlisten.push(await win.listen('agent-probe', () => { /* diagnostics only */ }));

      unlisten.push(await win.listen('agent-stream-thinking', (e) => {
        const r = accept<string>('agent-stream-thinking', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'thinking', chunk: typeof r.value === 'string' ? r.value : JSON.stringify(r.value) });
      }));

      unlisten.push(await win.listen('agent-stream-chunk', (e) => {
        const r = accept<string>('agent-stream-chunk', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'chunk', chunk: typeof r.value === 'string' ? r.value : JSON.stringify(r.value) });
      }));

      unlisten.push(await win.listen('agent-stream-tool-confirmation', (e) => {
        const r = accept<ToolConfirmation>('agent-stream-tool-confirmation', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'tool-confirmation', payload: r.value });
      }));

      unlisten.push(await win.listen('agent-stream-tool-blocked', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-tool-blocked', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({ type: 'tool-blocked', toolName: String(p['tool_name'] ?? 'tool'), reason: String(p['reason'] ?? 'blocked') });
      }));

      unlisten.push(await win.listen('agent-stream-agent-iteration', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-agent-iteration', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'agent-iteration', iteration: Number((r.value as Record<string, unknown>)['iteration'] ?? 0) });
      }));

      unlisten.push(await win.listen('agent-stream-tool-start', (e) => {
        const r = accept<unknown>('agent-stream-tool-start', e.payload);
        if (!r.ok) return;
        const toolName = typeof r.value === 'string' ? r.value : String((r.value as Record<string, unknown>)?.['name'] ?? 'tool');
        dispatch({ type: 'tool-start', toolName });
      }));

      unlisten.push(await win.listen('agent-stream-tool-args', (e) => {
        const r = accept<unknown>('agent-stream-tool-args', e.payload);
        if (!r.ok) return;
        const args = typeof r.value === 'string' ? r.value : JSON.stringify(r.value, null, 2);
        dispatch({ type: 'tool-args', args });
      }));

      unlisten.push(await win.listen('agent-stream-tool-end', (e) => {
        const r = accept('agent-stream-tool-end', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'tool-end' });
      }));

      unlisten.push(await win.listen('agent-stream-tool-result', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-tool-result', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'tool-result',
          name: String(p['name'] ?? ''),
          success: Boolean(p['success']),
          output: p['output'] != null ? String(p['output']) : null,
          durationMs: p['duration_ms'] != null ? Number(p['duration_ms']) : null,
        });
      }));

      unlisten.push(await win.listen('agent-stream-shell-lifecycle', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-shell-lifecycle', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({ type: 'shell-lifecycle', processId: String(p['process_id'] ?? ''), payload: p });
      }));

      unlisten.push(await win.listen('agent-stream-shell-output', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-shell-output', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'shell-output',
          processId: String(p['process_id'] ?? ''),
          stream: (p['stream'] as 'Stdout' | 'Stderr') ?? 'Stdout',
          data: String(p['data'] ?? ''),
        });
      }));

      unlisten.push(await win.listen('agent-stream-retry', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-retry', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({ type: 'retry', attempt: Number(p['attempt'] ?? 1), reason: String(p['reason'] ?? '') });
      }));

      unlisten.push(await win.listen('agent-stream-status', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-status', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({ type: 'status', text: String(p['text'] ?? ''), kind: String(p['kind'] ?? 'ready') });
      }));

      unlisten.push(await win.listen('agent-context-compacted', (e) => {
        const r = accept<Record<string, unknown>>('agent-context-compacted', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({ type: 'context-compacted', summary: String(p['summary'] ?? '') });
      }));

      unlisten.push(await win.listen('agent-stream-done', (e) => {
        const r = accept('agent-stream-done', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'done' });
      }));

      unlisten.push(await win.listen('agent-stream-cancelled', (e) => {
        const r = accept('agent-stream-cancelled', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'cancelled' });
      }));

      unlisten.push(await win.listen('agent-stream-error', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-error', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({ type: 'error', message: String(p['message'] ?? p['error'] ?? 'Unknown error') });
      }));

      for (const evtName of ['stream-health-status', 'stream-health-warning', 'stream-reconnect-attempt', 'stream-reconnect-success', 'stream-reconnect-failed'] as const) {
        unlisten.push(await win.listen(evtName, (e) => {
          const r = accept<StreamHealthPayload>(evtName, e.payload);
          if (!r.ok) return;
          dispatch({ type: 'health', payload: r.value });
        }));
      }

      unlisten.push(await win.listen('agent-message', (e) => {
        const r = accept<Record<string, unknown>>('agent-message', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({ type: 'agent-message', role: String(p['role'] ?? 'assistant'), content: String(p['content'] ?? '') });
      }));

      unlisten.push(await win.listen('listening-state-changed', (e) => {
        const r = accept<boolean>('listening-state-changed', e.payload);
        if (!r.ok) return;
        const val = typeof r.value === 'boolean' ? r.value : Boolean((r.value as Record<string, unknown>)?.['listening']);
        dispatch({ type: 'listening-state', listening: val });
      }));

      for (const evtName of ['task-created', 'task-updated', 'task-deleted'] as const) {
        unlisten.push(await win.listen(evtName, () => {
          dispatch({ type: 'task-changed' });
        }));
      }
    }

    setup().catch((err) => console.error('[useStreamEvents] setup error:', err));

    return () => {
      unlisten.forEach((fn) => fn());
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);
}

