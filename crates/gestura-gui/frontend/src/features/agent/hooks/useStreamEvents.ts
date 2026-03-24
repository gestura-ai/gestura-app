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
  TaskRuntimeSnapshot,
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

function readStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((entry) => (typeof entry === 'string' ? entry.trim() : ''))
    .filter((entry) => entry.length > 0);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function readTaskRuntimeTaskView(value: unknown): TaskRuntimeSnapshot['current_task'] {
  if (!isRecord(value)) return null;
  return {
    id: String(value['id'] ?? ''),
    name: String(value['name'] ?? ''),
    status: String(value['status'] ?? ''),
  };
}

function readTaskRuntimeTaskViews(value: unknown): TaskRuntimeSnapshot['ready_tasks'] {
  if (!Array.isArray(value)) return [];
  return value
    .map((entry) => readTaskRuntimeTaskView(entry))
    .filter((entry): entry is NonNullable<TaskRuntimeSnapshot['current_task']> => {
      return Boolean(entry && entry.id && entry.name);
    });
}

function readTaskRuntimeSnapshot(value: unknown): TaskRuntimeSnapshot | null {
  const payload = isRecord(value) && isRecord(value['snapshot'])
    ? value['snapshot']
    : value;
  if (!isRecord(payload)) return null;

  const rootTaskId = typeof payload['root_task_id'] === 'string' ? payload['root_task_id'] : '';
  const statusMessage = typeof payload['status_message'] === 'string' ? payload['status_message'] : '';
  if (!rootTaskId || !statusMessage) return null;

  return {
    root_task_id: rootTaskId,
    current_task: readTaskRuntimeTaskView(payload['current_task']),
    ready_tasks: readTaskRuntimeTaskViews(payload['ready_tasks']),
    parallel_ready_tasks: readTaskRuntimeTaskViews(payload['parallel_ready_tasks']),
    blocked_tasks: readTaskRuntimeTaskViews(payload['blocked_tasks']),
    open_tasks: readTaskRuntimeTaskViews(payload['open_tasks']),
    completed_tasks: readTaskRuntimeTaskViews(payload['completed_tasks']),
    missing_requirements: readStringArray(payload['missing_requirements']),
    status_message: statusMessage,
  };
}

// ─── Event type discriminants ─────────────────────────────────────────────────

export type StreamEventAction =
  | { type: 'thinking'; chunk: string }
  | { type: 'chunk'; chunk: string }
  | {
    type: 'narration';
    title?: string | null;
    message: string;
    summary?: string | null;
    reason?: string | null;
    nextStep?: string | null;
    evidence: string[];
    stage: 'context' | 'planning' | 'execution' | 'verification' | 'blocked' | 'progress';
  }
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
  | { type: 'paused' }
  | { type: 'cancelled' }
  | { type: 'resumed' }
  | { type: 'error'; message: string }
  | { type: 'health'; payload: StreamHealthPayload }
  | { type: 'agent-message'; role: string; content: string }
  | { type: 'listening-state'; listening: boolean }
  | { type: 'task-runtime-state'; snapshot: TaskRuntimeSnapshot }
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
    let cancelled = false;

    // Debounced notification that the agent likely mutated the workspace.
    // ExplorerPanel + EditorArea both listen for this to refresh without polling.
    let workspaceChangedTimer: number | null = null;
    let currentToolName: string | null = null;
    let sawWorkspaceMutation = false;

    function shouldSignalWorkspaceChanged(toolName: string | null | undefined): boolean {
      const t = (toolName ?? '').toLowerCase();
      // Allow-list: only tools that commonly mutate local files.
      return t === 'file' || t === 'shell' || t === 'git';
    }

    function shouldRefreshTasks(toolName: string | null | undefined, success: boolean): boolean {
      if (!success) return false;
      const t = (toolName ?? '').toLowerCase();
      return t === 'task' || t === 'tasks';
    }

    function scheduleWorkspaceChanged(): void {
      if (workspaceChangedTimer != null) {
        window.clearTimeout(workspaceChangedTimer);
      }
      workspaceChangedTimer = window.setTimeout(() => {
        if (cancelled) return;
        window.dispatchEvent(new CustomEvent('gestura:workspace:changed'));
      }, 250);
    }

    function accept<T = unknown>(eventName: string, raw: unknown): { ok: true; value: T } | { ok: false } {
      const { incomingSessionId, value } = unpackPayload<T>(raw);
      if (sessionId && (!incomingSessionId || incomingSessionId !== sessionId)) {
        return { ok: false };
      }
      void eventName;
      return { ok: true, value };
    }

    type ListenHandler = Parameters<typeof win.listen>[1];

    async function safeListen(eventName: string, handler: ListenHandler): Promise<void> {
      if (cancelled) return;
      const fn = await win.listen(eventName, handler);
      if (cancelled) {
        // React.StrictMode (dev) can mount/unmount quickly; if we were cleaned up
        // while awaiting, immediately detach to avoid dangling listeners.
        try {
          fn();
        } catch {
          // best-effort
        }
        return;
      }
      unlisten.push(fn);
    }

    async function setup() {
      await safeListen('agent-probe', () => { /* diagnostics only */ });

      await safeListen('agent-stream-thinking', (e) => {
        const r = accept<string>('agent-stream-thinking', e.payload);
        if (!r.ok) return;
        dispatch({
          type: 'thinking',
          chunk: typeof r.value === 'string' ? r.value : JSON.stringify(r.value),
        });
      });

      await safeListen('agent-stream-chunk', (e) => {
        const r = accept<string>('agent-stream-chunk', e.payload);
        if (!r.ok) return;
        dispatch({
          type: 'chunk',
          chunk: typeof r.value === 'string' ? r.value : JSON.stringify(r.value),
        });
      });

      await safeListen('agent-stream-tool-confirmation', (e) => {
        const r = accept<ToolConfirmation>('agent-stream-tool-confirmation', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'tool-confirmation', payload: r.value });
      });

      await safeListen('agent-stream-tool-blocked', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-tool-blocked', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'tool-blocked',
          toolName: String(p['tool_name'] ?? 'tool'),
          reason: String(p['reason'] ?? 'blocked'),
        });
      });

      await safeListen('agent-stream-agent-iteration', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-agent-iteration', e.payload);
        if (!r.ok) return;
        dispatch({
          type: 'agent-iteration',
          iteration: Number((r.value as Record<string, unknown>)['iteration'] ?? 0),
        });
      });

      await safeListen('agent-stream-tool-start', (e) => {
        const r = accept<unknown>('agent-stream-tool-start', e.payload);
        if (!r.ok) return;
        const toolName =
          typeof r.value === 'string'
            ? r.value
            : String((r.value as Record<string, unknown>)?.['name'] ?? 'tool');
        currentToolName = toolName;
        if (shouldSignalWorkspaceChanged(toolName)) sawWorkspaceMutation = true;
        dispatch({ type: 'tool-start', toolName });
      });

      await safeListen('agent-stream-tool-args', (e) => {
        const r = accept<unknown>('agent-stream-tool-args', e.payload);
        if (!r.ok) return;
        const args = typeof r.value === 'string' ? r.value : JSON.stringify(r.value, null, 2);
        dispatch({ type: 'tool-args', args });
      });

      await safeListen('agent-stream-tool-end', (e) => {
        const r = accept('agent-stream-tool-end', e.payload);
        if (!r.ok) return;

        if (shouldSignalWorkspaceChanged(currentToolName)) {
          sawWorkspaceMutation = true;
          scheduleWorkspaceChanged();
        }
        currentToolName = null;

        dispatch({ type: 'tool-end' });
      });

      await safeListen('agent-stream-tool-result', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-tool-result', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        const name = String(p['name'] ?? '');
        const success = Boolean(p['success']);

        // Refresh the workspace after tools that commonly mutate local files.
        // We do this regardless of `success` since a partially failed tool may still
        // have written files.
        if (shouldSignalWorkspaceChanged(name)) {
          sawWorkspaceMutation = true;
          scheduleWorkspaceChanged();
        }

        // Core-side task tool mutations update the shared TaskManager directly, but
        // they do not emit Tauri `task-*` events. Refresh the task hierarchy when a
        // task tool call succeeds so autogenerated subtasks stay current mid-run.
        if (shouldRefreshTasks(name, success)) {
          dispatch({ type: 'task-changed' });
        }

        dispatch({
          type: 'tool-result',
          name,
          success,
          output: p['output'] != null ? String(p['output']) : null,
          durationMs: p['duration_ms'] != null ? Number(p['duration_ms']) : null,
        });
      });

      await safeListen('agent-stream-shell-lifecycle', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-shell-lifecycle', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'shell-lifecycle',
          processId: String(p['process_id'] ?? ''),
          payload: p,
        });
      });

      await safeListen('agent-stream-shell-output', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-shell-output', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'shell-output',
          processId: String(p['process_id'] ?? ''),
          stream: (p['stream'] as 'Stdout' | 'Stderr') ?? 'Stdout',
          data: String(p['data'] ?? ''),
        });
      });

      await safeListen('agent-stream-retry', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-retry', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'retry',
          attempt: Number(p['attempt'] ?? 1),
          reason: String(p['reason'] ?? ''),
        });
      });

      await safeListen('agent-stream-status', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-status', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'status',
          text: String(p['text'] ?? ''),
          kind: String(p['kind'] ?? 'ready'),
        });
      });

      await safeListen('agent-stream-narration', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-narration', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'narration',
          title: p['title'] != null ? String(p['title']) : null,
          message: String(p['message'] ?? ''),
          summary: p['summary'] != null ? String(p['summary']) : null,
          reason: p['reason'] != null ? String(p['reason']) : null,
          nextStep: p['next_step'] != null ? String(p['next_step']) : null,
          evidence: readStringArray(p['evidence']),
          stage: (p['stage'] as 'context' | 'planning' | 'execution' | 'verification' | 'blocked' | 'progress' | undefined) ?? 'progress',
        });
      });

      await safeListen('agent-stream-task-state', (e) => {
        const r = accept<Record<string, unknown>>('agent-stream-task-state', e.payload);
        if (!r.ok) return;
        const snapshot = readTaskRuntimeSnapshot(r.value);
        if (!snapshot) return;
        dispatch({ type: 'task-runtime-state', snapshot });
      });

      await safeListen('agent-context-compacted', (e) => {
        const r = accept<Record<string, unknown>>('agent-context-compacted', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({ type: 'context-compacted', summary: String(p['summary'] ?? '') });
      });

      await safeListen('agent-stream-done', (e) => {
        const r = accept('agent-stream-done', e.payload);
        if (!r.ok) return;
        // Cheap convergence refresh at the end of a stream, only when we saw a
        // tool that commonly mutates local files.
        if (sawWorkspaceMutation) scheduleWorkspaceChanged();
        dispatch({ type: 'done' });
      });

      await safeListen('agent-stream-paused', (e) => {
        const r = accept('agent-stream-paused', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'paused' });
      });

      await safeListen('agent-stream-cancelled', (e) => {
        const r = accept('agent-stream-cancelled', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'cancelled' });
      });

      await safeListen('agent-stream-resumed', (e) => {
        const r = accept('agent-stream-resumed', e.payload);
        if (!r.ok) return;
        dispatch({ type: 'resumed' });
      });

      await safeListen('agent-stream-error', (e) => {
        const r = accept<unknown>('agent-stream-error', e.payload);
        if (!r.ok) return;
        let message: string;
        if (typeof r.value === 'string') {
          message = r.value;
        } else {
          const p = r.value as Record<string, unknown>;
          message = String(p['message'] ?? p['error'] ?? 'Unknown error');
        }
        dispatch({ type: 'error', message });
      });

      for (const evtName of [
        'stream-health-status',
        'stream-health-warning',
        'stream-reconnect-attempt',
        'stream-reconnect-success',
        'stream-reconnect-failed',
      ] as const) {
        await safeListen(evtName, (e) => {
          const r = accept<StreamHealthPayload>(evtName, e.payload);
          if (!r.ok) return;
          dispatch({ type: 'health', payload: r.value });
        });
      }

      await safeListen('agent-message', (e) => {
        const r = accept<Record<string, unknown>>('agent-message', e.payload);
        if (!r.ok) return;
        const p = r.value as Record<string, unknown>;
        dispatch({
          type: 'agent-message',
          role: String(p['role'] ?? 'assistant'),
          content: String(p['content'] ?? ''),
        });
      });

      await safeListen('listening-state-changed', (e) => {
        const r = accept<boolean>('listening-state-changed', e.payload);
        if (!r.ok) return;
        const val =
          typeof r.value === 'boolean'
            ? r.value
            : Boolean((r.value as Record<string, unknown>)?.['listening']);
        dispatch({ type: 'listening-state', listening: val });
      });

      for (const evtName of ['task-created', 'task-updated', 'task-deleted'] as const) {
        await safeListen(evtName, () => {
          dispatch({ type: 'task-changed' });
        });
      }
    }

    setup().catch((err) => console.error('[useStreamEvents] setup error:', err));

    return () => {
      cancelled = true;
      if (workspaceChangedTimer != null) {
        window.clearTimeout(workspaceChangedTimer);
      }
      unlisten.forEach((fn) => fn());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);
}

