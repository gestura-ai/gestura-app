/**
 * useChatSession — full state management for the agent chat.
 *
 * Manages messages, streaming state, tool confirmations, voice, tasks,
 * knowledge, and queued messages. Delegates event subscription to
 * useStreamEvents and Tauri commands to services/tauri/agent.ts.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { nanoid } from 'nanoid';

import {
  useStreamEvents,
  type StreamEventAction,
} from './useStreamEvents';
import {
  sendMessageStreaming,
  pauseStreaming,
  resumeStreaming,
  getSessionReplaySnapshot,
  getTaskHierarchy,
  listKnowledgeItems,
  getEnabledKnowledge,
  getSessionToolSettings,
  resolveToolConfirmationDecision,
  enhancePrompt,
  startVoiceListening,
  stopVoiceListening,
  type SessionActivityEvent,
} from '../../../services/tauri/agent';
import type {
  AgentMessage,
  MsgBlock,
  ThinkingBlock,
  TextBlock,
  ToolBlock,
  ShellBlock,
  ShellLine,
  NarrationBlock,
  ToolConfirmation,
  ToolConfirmationDecision,
  TaskHierarchy,
  TaskRuntimeSnapshot,
  KnowledgeItem,
  StatusState,
  ShellSessionRecord,
} from '../types';
import { buildShellCommandLine } from '../utils/shellTranscript';
import {
  applyShellLifecyclePayload,
  applyShellOutputPayload,
  applyShellSessionLifecyclePayload,
} from '../utils/shellSessionState';

// ─── Queued item ──────────────────────────────────────────────────────────────

interface QueuedMessage {
  kind: 'text' | 'voice';
  text: string;
  taskId: string | null;
}

function normalizeShellState(raw: unknown): ShellBlock['state'] {
  switch (String(raw ?? '').toLowerCase()) {
    case 'started': return 'Started';
    case 'running': return 'Running';
    case 'paused': return 'Paused';
    case 'resumed': return 'Resumed';
    case 'completed': return 'Completed';
    case 'failed': return 'Failed';
    case 'stopped': return 'Stopped';
    default: return 'Running';
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function unwrapActivityPayload(payload: unknown): unknown {
  if (isRecord(payload) && 'value' in payload && 'session_id' in payload) {
    return payload.value;
  }
  return payload;
}

function normalizeNarrationStage(
  raw: unknown,
): 'context' | 'planning' | 'execution' | 'verification' | 'blocked' | 'progress' {
  switch (raw) {
    case 'context':
    case 'planning':
    case 'execution':
    case 'verification':
    case 'blocked':
      return raw;
    default:
      return 'progress';
  }
}

function toTaskRuntimeTaskView(value: unknown): TaskRuntimeSnapshot['current_task'] {
  if (!isRecord(value)) return null;
  return {
    id: String(value['id'] ?? ''),
    name: String(value['name'] ?? ''),
    status: String(value['status'] ?? ''),
  };
}

function toTaskRuntimeTaskViews(value: unknown): TaskRuntimeSnapshot['ready_tasks'] {
  if (!Array.isArray(value)) return [];
  return value
    .map((entry) => toTaskRuntimeTaskView(entry))
    .filter((entry): entry is NonNullable<TaskRuntimeSnapshot['current_task']> => {
      return Boolean(entry && entry.id && entry.name);
    });
}

function toTaskRuntimeSnapshot(value: unknown): TaskRuntimeSnapshot | null {
  const payload = isRecord(value) && isRecord(value['snapshot'])
    ? value['snapshot']
    : value;
  if (!isRecord(payload)) return null;

  const rootTaskId = typeof payload['root_task_id'] === 'string' ? payload['root_task_id'] : '';
  const statusMessage = typeof payload['status_message'] === 'string' ? payload['status_message'] : '';
  if (!rootTaskId || !statusMessage) return null;

  return {
    root_task_id: rootTaskId,
    current_task: toTaskRuntimeTaskView(payload['current_task']),
    ready_tasks: toTaskRuntimeTaskViews(payload['ready_tasks']),
    parallel_ready_tasks: toTaskRuntimeTaskViews(payload['parallel_ready_tasks']),
    blocked_tasks: toTaskRuntimeTaskViews(payload['blocked_tasks']),
    open_tasks: toTaskRuntimeTaskViews(payload['open_tasks']),
    completed_tasks: toTaskRuntimeTaskViews(payload['completed_tasks']),
    missing_requirements: Array.isArray(payload['missing_requirements'])
      ? payload['missing_requirements']
        .map((entry) => String(entry))
        .filter((entry) => entry.trim().length > 0)
      : [],
    status_message: statusMessage,
  };
}

function toReplayToolConfirmation(payload: Record<string, unknown>): ToolConfirmation | null {
  if (typeof payload.confirmation_id !== 'string' || typeof payload.tool_name !== 'string') {
    return null;
  }

  return {
    confirmation_id: payload.confirmation_id,
    tool_name: payload.tool_name,
    tool_args: payload.tool_args != null ? String(payload.tool_args) : undefined,
    description: payload.description != null ? String(payload.description) : undefined,
    risk_level: payload.risk_level != null ? String(payload.risk_level) : undefined,
    category: payload.category != null ? String(payload.category) : undefined,
    session_id: payload.session_id != null ? String(payload.session_id) : null,
  };
}

function toReplayAction(entry: SessionActivityEvent): StreamEventAction | null {
  const payload = unwrapActivityPayload(entry.payload);
  const payloadRecord = isRecord(payload) ? payload : null;

  switch (entry.event_type) {
    case 'agent-stream-thinking':
      return typeof payload === 'string' ? { type: 'thinking', chunk: payload } : null;
    case 'agent-stream-chunk':
      return typeof payload === 'string' ? { type: 'chunk', chunk: payload } : null;
    case 'agent-stream-tool-blocked':
      return payloadRecord
        ? {
          type: 'tool-blocked',
          toolName: String(payloadRecord.tool_name ?? 'tool'),
          reason: String(payloadRecord.reason ?? 'blocked'),
        }
        : null;
    case 'agent-stream-agent-iteration':
      return payloadRecord
        ? {
          type: 'agent-iteration',
          iteration: Number(payloadRecord.iteration ?? 0),
        }
        : null;
    case 'agent-stream-tool-start':
      return typeof payload === 'string'
        ? {
          type: 'tool-start',
          toolName: payload,
          toolCallId: null,
        }
        : payloadRecord
          ? {
            type: 'tool-start',
            toolName: String(payloadRecord.name ?? 'tool'),
            toolCallId: typeof payloadRecord.id === 'string' ? payloadRecord.id : null,
          }
          : null;
    case 'agent-stream-tool-args':
      return typeof payload === 'string'
        ? { type: 'tool-args', args: payload, toolCallId: null }
        : payloadRecord
          ? {
            type: 'tool-args',
            args: typeof payloadRecord.args === 'string'
              ? payloadRecord.args
              : JSON.stringify(payloadRecord, null, 2),
            toolCallId: typeof payloadRecord.id === 'string' ? payloadRecord.id : null,
          }
          : null;
    case 'agent-stream-tool-end':
      return {
        type: 'tool-end',
        toolCallId: payloadRecord && typeof payloadRecord.id === 'string' ? payloadRecord.id : null,
      };
    case 'agent-stream-tool-result':
      return payloadRecord
        ? {
          type: 'tool-result',
          name: String(payloadRecord.name ?? ''),
          success: Boolean(payloadRecord.success),
          output: payloadRecord.output != null ? String(payloadRecord.output) : null,
          durationMs: payloadRecord.duration_ms != null ? Number(payloadRecord.duration_ms) : null,
          toolCallId: typeof payloadRecord.id === 'string' ? payloadRecord.id : null,
        }
        : null;
    case 'agent-stream-status':
      return payloadRecord
        ? {
          type: 'status',
          text: String(payloadRecord.text ?? ''),
          kind: String(payloadRecord.kind ?? 'ready'),
        }
        : null;
    case 'agent-stream-retry':
      return payloadRecord
        ? {
          type: 'retry',
          attempt: Number(payloadRecord.attempt ?? 1),
          reason: String(payloadRecord.reason ?? ''),
        }
        : null;
    case 'agent-context-compacted':
      return payloadRecord
        ? { type: 'context-compacted', summary: String(payloadRecord.summary ?? '') }
        : null;
    case 'agent-stream-narration':
      return payloadRecord
        ? {
          type: 'narration',
          title: payloadRecord.title != null ? String(payloadRecord.title) : null,
          message: String(payloadRecord.message ?? ''),
          summary: payloadRecord.summary != null ? String(payloadRecord.summary) : null,
          reason: payloadRecord.reason != null ? String(payloadRecord.reason) : null,
          nextStep: payloadRecord.next_step != null ? String(payloadRecord.next_step) : null,
          evidence: Array.isArray(payloadRecord.evidence)
            ? payloadRecord.evidence.map((entry) => String(entry)).filter((entry) => entry.trim().length > 0)
            : [],
          stage: normalizeNarrationStage(payloadRecord.stage),
        }
        : null;
    case 'agent-stream-task-state': {
      const snapshot = toTaskRuntimeSnapshot(payload);
      return snapshot ? { type: 'task-runtime-state', snapshot } : null;
    }
    case 'agent-stream-shell-lifecycle':
      return payloadRecord && typeof payloadRecord.process_id === 'string'
        ? { type: 'shell-lifecycle', processId: payloadRecord.process_id, payload: payloadRecord }
        : null;
    case 'agent-stream-shell-session-lifecycle':
      return payloadRecord && typeof payloadRecord.shell_session_id === 'string'
        ? {
          type: 'shell-session-lifecycle',
          shellSessionId: payloadRecord.shell_session_id,
          payload: payloadRecord,
        }
        : null;
    case 'agent-stream-shell-output':
      return payloadRecord && typeof payloadRecord.process_id === 'string'
        ? {
          type: 'shell-output',
          processId: payloadRecord.process_id,
          shellSessionId: payloadRecord.shell_session_id != null ? String(payloadRecord.shell_session_id) : null,
          stream: (payloadRecord.stream as 'Stdout' | 'Stderr') ?? 'Stdout',
          data: String(payloadRecord.data ?? ''),
        }
        : null;
    case 'agent-stream-tool-confirmation':
      return payloadRecord
        ? (() => {
          const confirmation = toReplayToolConfirmation(payloadRecord);
          return confirmation ? { type: 'tool-confirmation', payload: confirmation } : null;
        })()
        : null;
    case 'agent-stream-agent-message':
      return payloadRecord
        ? {
          type: 'agent-message',
          role: String(payloadRecord.role ?? 'assistant'),
          content: String(payloadRecord.content ?? ''),
        }
        : null;
    case 'agent-stream-done':
      return { type: 'done' };
    case 'agent-stream-paused':
      return { type: 'paused' };
    case 'agent-stream-cancelled':
      return { type: 'cancelled' };
    case 'agent-stream-error':
      return typeof payload === 'string'
        ? { type: 'error', message: payload }
        : payloadRecord
          ? { type: 'error', message: String(payloadRecord.message ?? payloadRecord.error ?? 'Unknown error') }
          : null;
    case 'agent-stream-resumed':
      return { type: 'resumed' };
    default:
      return null;
  }
}

function toReplayUserMessage(entry: SessionActivityEvent): AgentMessage | null {
  if (entry.event_type !== 'session-user-message') {
    return null;
  }

  const payload = unwrapActivityPayload(entry.payload);
  if (!isRecord(payload) || typeof payload.content !== 'string') {
    return null;
  }

  const timestamp = Date.parse(entry.timestamp);
  return {
    id: nanoid(),
    role: 'user',
    rawMarkdown: payload.content,
    blocks: [{ kind: 'text', id: nanoid(), content: payload.content }],
    isStreaming: false,
    timestamp: Number.isFinite(timestamp) ? timestamp : Date.now(),
  };
}

// ─── Return type ─────────────────────────────────────────────────────────────

export interface ChatSessionState {
  messages: AgentMessage[];
  streamingMessage: AgentMessage | null;
  isProcessing: boolean;
  isStopping: boolean;
  canResume: boolean;
  isResuming: boolean;
  isListening: boolean;
  status: StatusState;
  pendingConfirmation: ToolConfirmation | null;
  tasks: TaskHierarchy;
  runtimeTaskSnapshot: TaskRuntimeSnapshot | null;
  knowledgeItems: KnowledgeItem[];
  toolSettings: Record<string, unknown>;
  memoryRevision: number;
  userScrolledUp: boolean;
  setUserScrolledUp: (v: boolean) => void;
  sendMessage: (text: string, taskId?: string | null) => Promise<void>;
  cancelStream: () => Promise<void>;
  resumeStream: () => Promise<void>;
  resolveConfirmation: (decision: ToolConfirmationDecision) => Promise<void>;
  toggleVoice: () => Promise<void>;
  enhanceText: (text: string) => Promise<string>;
  refreshTasks: () => Promise<void>;
  refreshKnowledge: () => Promise<void>;
  refreshToolSettings: () => Promise<void>;
}

export interface ChatSessionOptions {
  shellSessions?: ShellSessionRecord[];
}

// ─── Helper — build a contextual iteration marker label ───────────────────────

interface LastToolContext {
  name: string;
  success: boolean;
  output: string | null;
  args: string;
  completed: boolean;
}

interface ReviewNarrationDraft {
  title: string;
  message: string;
  stage: NarrationBlock['stage'];
}

function collapseIterationWhitespace(text: string): string {
  return text.replace(/\s+/g, ' ').trim();
}

function truncateIterationDetail(text: string, maxLength = 96): string {
  if (text.length <= maxLength) return text;
  return `${text.slice(0, maxLength - 1).trimEnd()}…`;
}

function parseIterationArgs(raw: string): unknown {
  const trimmed = raw.trim();
  if (!trimmed) return null;

  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return trimmed;
  }
}

function readIterationString(source: Record<string, unknown> | null, keys: string[]): string | null {
  if (!source) return null;

  for (const key of keys) {
    const value = source[key];
    if (typeof value === 'string' && value.trim()) {
      return collapseIterationWhitespace(value);
    }
  }

  return null;
}

function firstIterationDetail(...candidates: Array<string | null | undefined>): string | undefined {
  for (const candidate of candidates) {
    const compact = collapseIterationWhitespace(candidate ?? '');
    if (compact) return truncateIterationDetail(compact);
  }

  return undefined;
}

function iterationTitleTokens(text: string | null | undefined, maxWords: number): string | null {
  const compact = collapseIterationWhitespace(text ?? '');
  if (!compact) return null;

  const tokens = compact
    .split(/[^A-Za-z0-9._/-]+/)
    .map((token) => token.trim())
    .filter(Boolean)
    .slice(0, maxWords);

  return tokens.length >= 2 ? tokens.join(' ') : null;
}

function iterationContextualTitle(verb: string, detail: string | null | undefined, fallback: string, maxWords = 3): string {
  const tokens = iterationTitleTokens(detail, maxWords);
  return tokens ? `${verb} ${tokens}` : fallback;
}

function iterationUrlHost(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    return new URL(url).host || null;
  } catch {
    return null;
  }
}

function quotedDetail(detail: string | null | undefined): string {
  const compact = collapseIterationWhitespace(detail ?? '');
  return compact ? `"${truncateIterationDetail(compact, 120)}"` : 'the latest result';
}

function buildIterationReviewNarration(ctx: LastToolContext | null): ReviewNarrationDraft | null {
  const marker = buildIterationLabel(ctx);
  if (!marker || !ctx?.completed) return null;

  const toolName = collapseIterationWhitespace(ctx.name).toLowerCase();
  const parsedArgs = parseIterationArgs(ctx.args);
  const args = isRecord(parsedArgs) ? parsedArgs : null;
  const operation = readIterationString(args, ['operation', 'action', 'mode', 'subcommand'])?.toLowerCase() ?? null;
  const detail = marker.detail ?? null;
  const focus = quotedDetail(detail);

  if (!ctx.success) {
    return {
      title: marker.label,
      stage: 'blocked',
      message: `I hit a problem while working through ${focus}, so I’m checking whether the issue came from the tool input, the environment, or the current branch of the task before I try again. I want the next move to change the situation instead of replaying the same failed step.`,
    };
  }

  switch (toolName) {
    case 'shell':
      return {
        title: marker.label,
        stage: 'verification',
        message: `I have the latest command result from ${focus} in hand, so I’m checking what it actually proved before I decide whether to keep executing, make a code change, or move into verification. That result should tell me whether this branch of the work really advanced or whether it only narrowed the next decision.`,
      };
    case 'file':
      if (operation === 'write' || operation === 'edit' || operation === 'update' || operation === 'delete' || operation === 'remove') {
        return {
          title: marker.label,
          stage: 'execution',
          message: `I’m reviewing the workspace change around ${focus} so I can tell whether it actually moved the implementation forward or only covered part of the request. That should tell me whether the next step stays in code, branches into another file, or shifts into validation.`,
        };
      }
      return {
        title: marker.label,
        stage: 'context',
        message: `I’m going through the file context around ${focus} and pulling out the details that matter for the next decision. I want to use this read to choose a concrete follow-up step instead of treating the inspection itself like progress.`,
      };
    case 'git':
      return {
        title: marker.label,
        stage: 'verification',
        message: `I’m reviewing the repository signal around ${focus} to see how the current workspace state lines up with the work I think I just moved. That should tell me whether the branch is ready for validation or whether there is still another implementation step hiding behind the diff.`,
      };
    case 'code':
      return {
        title: marker.label,
        stage: 'context',
        message: `I’m checking the code context around ${focus} so I can separate the signal that matters from the surrounding noise before I act on it. That should tell me whether I already have enough evidence to change code or whether I need one more targeted inspection first.`,
      };
    case 'web_search':
      return {
        title: marker.label,
        stage: 'context',
        message: `I’m reading through the research returned for ${focus} and filtering down to the findings that actually matter for this request. Once I know which pieces are strong enough to trust, I can fold them back into the plan and decide whether I already have enough signal to move or need one more targeted lookup.`,
      };
    case 'web':
      return {
        title: marker.label,
        stage: 'context',
        message: `I’m reviewing the source material from ${focus} and checking which parts are concrete enough to guide the next action. That should tell me whether this evidence is ready to shape the plan now or whether I need to cross-check it before I commit to the next step.`,
      };
    case 'mcp':
      return {
        title: marker.label,
        stage: 'progress',
        message: `I’m going through the MCP result around ${focus} so I can see what new capability or evidence it actually unlocked for this request. That should tell me whether I can use this result directly or whether it only set up the next concrete action.`,
      };
    case 'gui_control':
      return {
        title: marker.label,
        stage: 'progress',
        message: `I’m checking the UI action around ${focus} to confirm what changed and whether it gave me the state I needed. That should tell me whether the interface is ready for the next step or whether I need to correct the interaction before moving on.`,
      };
    case 'screenshot':
    case 'screen_record':
      return {
        title: marker.label,
        stage: 'context',
        message: `I’m reviewing the captured screen context around ${focus} so I can anchor the next step in what the interface actually shows instead of relying on assumptions. That should tell me whether the screen already confirms the path forward or whether I need one more targeted action to resolve the ambiguity.`,
      };
    default:
      return {
        title: marker.label,
        stage: 'progress',
        message: `I’m reviewing ${focus} to understand what this latest result changed before I commit to the next move. That should tell me whether this work actually unlocked progress or whether I need to take a different kind of step next.`,
      };
  }
}

function buildIterationLabel(ctx: LastToolContext | null): { label: string; detail?: string } | null {
  if (!ctx?.completed) return null;

  const toolName = collapseIterationWhitespace(ctx.name);
  if (!toolName) return null;

  const normalizedToolName = toolName.toLowerCase();
  const parsedArgs = parseIterationArgs(ctx.args);
  const args = isRecord(parsedArgs) ? parsedArgs : null;
  const operation = readIterationString(args, ['operation', 'action', 'mode', 'subcommand'])?.toLowerCase() ?? null;
  const path = readIterationString(args, ['path', 'target', 'file_path']);
  const command = readIterationString(args, ['command']);
  const query = readIterationString(args, ['query', 'search']);
  const url = readIterationString(args, ['url']);
  const guiAction = readIterationString(args, ['action']);
  const mcpTarget = readIterationString(args, ['tool', 'tool_name', 'server', 'server_name']);
  const detail = firstIterationDetail(path, command, query, url, guiAction, mcpTarget, ctx.output);

  if (normalizedToolName === 'task' || normalizedToolName === 'tasks') {
    return null;
  }

  if (!ctx.success) {
    switch (normalizedToolName) {
      case 'shell':
        return {
          label: iterationContextualTitle('Reviewing', command ?? detail, 'Reviewing shell failure', 4),
          detail,
        };
      case 'file':
        return { label: 'Resolving file operation issue', detail };
      case 'git':
        return { label: 'Resolving repository issue', detail };
      case 'code':
        return { label: 'Resolving code analysis issue', detail };
      case 'web_search':
      case 'web':
        return { label: 'Reviewing incomplete research', detail };
      case 'mcp':
        return { label: 'Reviewing MCP tool failure', detail };
      case 'gui_control':
        return { label: 'Reviewing UI action outcome', detail };
      default:
        return { label: `Following up on ${toolName}`, detail };
    }
  }

  switch (normalizedToolName) {
    case 'shell':
      return {
        label: iterationContextualTitle('Checking', command ?? detail, 'Checking command results', 3),
        detail,
      };
    case 'file':
      switch (operation) {
        case 'write':
        case 'edit':
        case 'update':
        case 'delete':
        case 'remove':
          return { label: 'Reviewing workspace changes', detail };
        case 'search':
          return { label: 'Reviewing file search results', detail };
        case 'list':
        case 'tree':
          return { label: 'Reviewing workspace context', detail };
        default:
          return { label: 'Reviewing file context', detail };
      }
    case 'git':
      return {
        label: operation === 'diff' ? 'Reviewing repository changes' : 'Reviewing repository state',
        detail,
      };
    case 'code':
      return { label: 'Reviewing code context', detail };
    case 'web_search':
      return {
        label: iterationContextualTitle('Researching', query ?? detail, 'Reviewing research findings'),
        detail,
      };
    case 'web':
      return {
        label: iterationContextualTitle('Reviewing', iterationUrlHost(url) ?? detail, 'Reviewing fetched page', 4),
        detail,
      };
    case 'mcp':
      return { label: 'Reviewing MCP results', detail };
    case 'gui_control':
      return { label: 'Confirming UI update', detail };
    case 'permissions':
      return { label: 'Reviewing permission status', detail };
    case 'screenshot':
      return { label: 'Reviewing captured screen context', detail };
    case 'screen_record':
      return { label: 'Reviewing recorded screen activity', detail };
    default:
      return { label: `Reviewing ${toolName} results`, detail };
  }
}

function iterationMarkerSignature(
  contextVersion: number,
  marker: { label: string; detail?: string },
): string {
  return `${contextVersion}:${marker.label}:${marker.detail ?? ''}`;
}

// ─── Helper — produce a fresh streaming AgentMessage ─────────────────────────

function makeStreamingMessage(): AgentMessage {
  return { id: nanoid(), role: 'assistant', rawMarkdown: '', blocks: [], isStreaming: true, timestamp: Date.now() };
}

function makeStreamingMessageWithId(id: string): AgentMessage {
  return { id, role: 'assistant', rawMarkdown: '', blocks: [], isStreaming: true, timestamp: Date.now() };
}

function findShellBlockIndex(
  blocks: MsgBlock[],
  processId: string | null | undefined,
  shellSessionId?: string | null,
): number {
  return blocks.findIndex((block) => {
    if (block.kind !== 'shell') return false;
    if (processId && block.processId === processId) return true;
    return Boolean(shellSessionId && block.shellSessionId === shellSessionId);
  });
}

function findShellSessionBlockIndex(blocks: MsgBlock[], shellSessionId: string | null | undefined): number {
  if (!shellSessionId) return -1;
  return blocks.findIndex((block) => block.kind === 'shell-session' && block.shellSessionId === shellSessionId);
}

function updateShellSessionBlock(
  blocks: MsgBlock[],
  shellSessionId: string,
  updater: (current: ShellSessionRecord[]) => ShellSessionRecord[],
): MsgBlock[] {
  const index = findShellSessionBlockIndex(blocks, shellSessionId);
  if (index >= 0) {
    const updated = updater([blocks[index] as ShellSessionRecord])[0];
    if (!updated) return blocks;
    return blocks.map((block, blockIndex) => (blockIndex === index ? updated : block));
  }

  const created = updater([])[0];
  return created ? [...blocks, created] : blocks;
}

function mergeShellCommandLine(lines: ShellLine[], commandLine: ShellLine | null): ShellLine[] {
  if (!commandLine) return lines;
  return lines.some((line) => line.stream === commandLine.stream && line.data === commandLine.data)
    ? lines
    : [commandLine, ...lines];
}

function isStreamingPlaceholderMessage(message: AgentMessage | null | undefined): boolean {
  if (!message || message.rawMarkdown.trim()) return false;
  return message.blocks.length === 0;
}

function isActiveShellSession(shell: ShellSessionRecord): boolean {
  return shell.state === 'Starting' || shell.state === 'Busy' || shell.state === 'Interrupting';
}

function isReusableIdleShellSession(shell: ShellSessionRecord): boolean {
  return shell.state === 'Idle' && shell.availableForReuse && !shell.userManaged;
}

function cloneShellSessionBlock(
  shell: ShellSessionRecord,
  options: { id?: string; collapsed?: boolean } = {},
): ShellSessionRecord {
  return {
    ...shell,
    id: options.id ?? shell.id,
    collapsed: options.collapsed ?? shell.collapsed,
    lines: shell.lines.map((line) => ({ ...line })),
  };
}

function createPendingReusableShellSessionBlock(shell: ShellSessionRecord, activityAt: number): ShellSessionRecord {
  return cloneShellSessionBlock({
    ...shell,
    state: 'Starting',
    activeProcessId: null,
    activeCommand: null,
    lastActivityAt: activityAt,
    availableForReuse: false,
  });
}

function findSingleReusableShellSession(shellSessions: ShellSessionRecord[]): ShellSessionRecord | null {
  const reusable = shellSessions
    .filter(isReusableIdleShellSession)
    .sort((left, right) => {
      const leftActivity = left.lastActivityAt ?? left.startedAt ?? 0;
      const rightActivity = right.lastActivityAt ?? right.startedAt ?? 0;
      return rightActivity - leftActivity;
    });

  return reusable.length === 1 ? reusable[0] : null;
}

function shellLinesEqual(left: ShellSessionRecord['lines'], right: ShellSessionRecord['lines']): boolean {
  return left.length === right.length
    && left.every((line, index) => line.stream === right[index]?.stream && line.data === right[index]?.data);
}

function shellSessionBlocksEqual(left: ShellSessionRecord, right: ShellSessionRecord): boolean {
  return left.shellSessionId === right.shellSessionId
    && left.cwd === right.cwd
    && left.state === right.state
    && left.interactive === right.interactive
    && left.userManaged === right.userManaged
    && (left.activeProcessId ?? null) === (right.activeProcessId ?? null)
    && (left.activeCommand ?? null) === (right.activeCommand ?? null)
    && (left.lastExitCode ?? null) === (right.lastExitCode ?? null)
    && (left.durationMs ?? null) === (right.durationMs ?? null)
    && (left.startedAt ?? null) === (right.startedAt ?? null)
    && (left.lastActivityAt ?? null) === (right.lastActivityAt ?? null)
    && left.collapsed === right.collapsed
    && left.availableForReuse === right.availableForReuse
    && shellLinesEqual(left.lines, right.lines);
}

function shouldPreserveFresherLocalShellSession(
  current: ShellSessionRecord,
  replacement: ShellSessionRecord,
): boolean {
  const currentActivity = current.lastActivityAt ?? current.startedAt ?? 0;
  const replacementActivity = replacement.lastActivityAt ?? replacement.startedAt ?? 0;
  const currentRepresentsInFlightOrLocalClaim = current.state !== 'Idle'
    || Boolean(current.activeProcessId)
    || Boolean(current.activeCommand)
    || !current.availableForReuse;

  return currentRepresentsInFlightOrLocalClaim
    && replacement.state === 'Idle'
    && !replacement.activeProcessId
    && !replacement.activeCommand
    && replacement.availableForReuse
    && replacementActivity <= currentActivity;
}

function isRecoverableShellSession(
  shell: ShellSessionRecord,
  message: AgentMessage,
  hasShellToolBlock: boolean,
): boolean {
  if (shell.userManaged || !isActiveShellSession(shell)) {
    return false;
  }

  if (hasShellToolBlock) {
    return true;
  }

  const activityAt = shell.lastActivityAt ?? shell.startedAt ?? null;
  return activityAt != null && activityAt >= message.timestamp - 15_000;
}

function reconcileStreamingMessageShellSessions(
  message: AgentMessage | null,
  shellSessions: ShellSessionRecord[],
  isProcessing: boolean,
): AgentMessage | null {
  if (!message || !isProcessing || shellSessions.length === 0) {
    return message;
  }

  const hasShellToolBlock = message.blocks.some((block) => block.kind === 'tool' && block.name === 'shell');
  const existingSessionIds = new Set(
    message.blocks
      .filter((block): block is ShellSessionRecord => block.kind === 'shell-session')
      .map((block) => block.shellSessionId),
  );

  const matchingShellSessionIds = new Set<string>();
  const matchingProcessIds = new Set<string>();
  message.blocks.forEach((block) => {
    if (block.kind === 'shell-session') {
      matchingShellSessionIds.add(block.shellSessionId);
      if (block.activeProcessId) matchingProcessIds.add(block.activeProcessId);
      return;
    }

    if (block.kind === 'shell') {
      if (block.shellSessionId) matchingShellSessionIds.add(block.shellSessionId);
      if (block.processId) matchingProcessIds.add(block.processId);
    }
  });

  const candidates = shellSessions.filter((shell) => {
    if (matchingShellSessionIds.has(shell.shellSessionId)) {
      return true;
    }
    if (shell.activeProcessId && matchingProcessIds.has(shell.activeProcessId)) {
      return true;
    }
    return isRecoverableShellSession(shell, message, hasShellToolBlock);
  });

  if (candidates.length === 0) {
    return message;
  }

  const candidatesBySessionId = new Map(candidates.map((shell) => [shell.shellSessionId, shell]));
  const candidatesByProcessId = new Map(
    candidates
      .filter((shell) => shell.activeProcessId)
      .map((shell) => [shell.activeProcessId as string, shell]),
  );

  let changed = false;
  const insertedShellIds = new Set<string>();
  const nextBlocks: MsgBlock[] = [];

  message.blocks.forEach((block) => {
    if (block.kind === 'shell-session') {
      const replacement = candidatesBySessionId.get(block.shellSessionId);
      if (!replacement) {
        nextBlocks.push(block);
        return;
      }

      if (shouldPreserveFresherLocalShellSession(block, replacement)) {
        insertedShellIds.add(block.shellSessionId);
        nextBlocks.push(block);
        return;
      }

      insertedShellIds.add(replacement.shellSessionId);
      const nextBlock = cloneShellSessionBlock(replacement, { id: block.id, collapsed: block.collapsed });
      if (!shellSessionBlocksEqual(block, nextBlock)) {
        changed = true;
        nextBlocks.push(nextBlock);
        return;
      }

      nextBlocks.push(block);
      return;
    }

    if (block.kind === 'shell') {
      const replacement = (block.shellSessionId && candidatesBySessionId.get(block.shellSessionId))
        || candidatesByProcessId.get(block.processId);
      if (!replacement) {
        nextBlocks.push(block);
        return;
      }

      changed = true;
      if (!insertedShellIds.has(replacement.shellSessionId)) {
        insertedShellIds.add(replacement.shellSessionId);
        nextBlocks.push(cloneShellSessionBlock(replacement, { id: block.id, collapsed: block.collapsed }));
      }
      return;
    }

    nextBlocks.push(block);
  });

  candidates.forEach((shell) => {
    if (insertedShellIds.has(shell.shellSessionId) || existingSessionIds.has(shell.shellSessionId)) {
      return;
    }
    insertedShellIds.add(shell.shellSessionId);
    nextBlocks.push(cloneShellSessionBlock(shell));
    changed = true;
  });

  return changed ? { ...message, blocks: nextBlocks } : message;
}

async function waitForNextPaint(): Promise<void> {
  if (typeof window !== 'undefined' && typeof window.requestAnimationFrame === 'function') {
    await new Promise<void>((resolve) => {
      window.requestAnimationFrame(() => resolve());
    });
    return;
  }

  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

// ─── Hook ─────────────────────────────────────────────────────────────────────

export function useChatSession(sessionId: string, options: ChatSessionOptions = {}): ChatSessionState {
  const { shellSessions = [] } = options;
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [streamingMessage, setStreamingMessage] = useState<AgentMessage | null>(null);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [canResume, setCanResume] = useState(false);
  const [isResuming, setIsResuming] = useState(false);
  const [isListening, setIsListening] = useState(false);
  const [status, setStatus] = useState<StatusState>({ text: 'Ready', kind: 'ready' });
  const [pendingConfirmation, setPendingConfirmation] = useState<ToolConfirmation | null>(null);
  const [tasks, setTasks] = useState<TaskHierarchy>([]);
  const [runtimeTaskSnapshot, setRuntimeTaskSnapshot] = useState<TaskRuntimeSnapshot | null>(null);
  const [knowledgeItems, setKnowledgeItems] = useState<KnowledgeItem[]>([]);
  const [toolSettings, setToolSettings] = useState<Record<string, unknown>>({});
  const [memoryRevision, setMemoryRevision] = useState(0);
  const [userScrolledUp, setUserScrolledUp] = useState(false);

  // Streaming cursor refs (avoid stale closures in event listeners)
  const currentThinkingIdRef = useRef<string | null>(null);
  const currentTextBlockIdRef = useRef<string | null>(null);
  const currentToolBlockIdRef = useRef<string | null>(null);
  const currentToolCallIdRef = useRef<string | null>(null);
  const toolBlockIdsByCallIdRef = useRef(new Map<string, string>());
  const streamingMsgIdRef = useRef<string | null>(null);
  const pausedMessageIdRef = useRef<string | null>(null);
  const messageQueueRef = useRef<QueuedMessage[]>([]);
  const messagesRef = useRef<AgentMessage[]>([]);
  const streamingMessageRef = useRef<AgentMessage | null>(null);
  const shellSessionsRef = useRef<ShellSessionRecord[]>(shellSessions);
  const isProcessingRef = useRef(false);
  const statusRef = useRef<StatusState>({ text: 'Ready', kind: 'ready' });
  const retryStatusRestoreRef = useRef<StatusState | null>(null);
  const confirmationQueueRef = useRef<ToolConfirmation[]>([]);
  const pendingConfirmationRef = useRef<ToolConfirmation | null>(null);
  const triggerSendRef = useRef<((text: string, taskId: string | null) => Promise<void>) | null>(null);
  /** Tracks the most recently started/completed tool for contextual iteration markers. */
  const lastToolContextRef = useRef<LastToolContext | null>(null);
  const lastToolContextVersionRef = useRef(0);
  const lastIterationMarkerSignatureRef = useRef<string | null>(null);
  const lastReflectionNoticeRef = useRef<string | null>(null);

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  useEffect(() => {
    streamingMessageRef.current = streamingMessage;
  }, [streamingMessage]);

  useEffect(() => {
    shellSessionsRef.current = shellSessions;
  }, [shellSessions]);

  useEffect(() => {
    statusRef.current = status;
  }, [status]);

  const updateStatus = useCallback((next: StatusState) => {
    retryStatusRestoreRef.current = null;
    statusRef.current = next;
    setStatus(next);
  }, []);

  const showRetryStatus = useCallback((attempt: number) => {
    retryStatusRestoreRef.current ??= statusRef.current;
    const next: StatusState = { text: `Retrying (attempt ${attempt})…`, kind: 'busy' };
    statusRef.current = next;
    setStatus(next);
  }, []);

  const restoreStatusAfterRetry = useCallback(() => {
    const previous = retryStatusRestoreRef.current;
    if (!previous) {
      return;
    }

    retryStatusRestoreRef.current = null;
    statusRef.current = previous;
    setStatus(previous);
  }, []);

  const resetStreamingCursor = useCallback(() => {
    streamingMsgIdRef.current = null;
    currentThinkingIdRef.current = null;
    currentTextBlockIdRef.current = null;
    currentToolBlockIdRef.current = null;
    currentToolCallIdRef.current = null;
    toolBlockIdsByCallIdRef.current.clear();
    lastToolContextRef.current = null;
    lastToolContextVersionRef.current = 0;
    lastIterationMarkerSignatureRef.current = null;
    lastReflectionNoticeRef.current = null;
  }, []);

  const primeStreamingCursorFromMessage = useCallback((message: AgentMessage) => {
    streamingMsgIdRef.current = message.id;
    currentThinkingIdRef.current = null;
    currentTextBlockIdRef.current = null;
    currentToolBlockIdRef.current = null;
    currentToolCallIdRef.current = null;
    toolBlockIdsByCallIdRef.current.clear();

    for (let idx = message.blocks.length - 1; idx >= 0; idx -= 1) {
      const block = message.blocks[idx];
      if (!currentTextBlockIdRef.current && block.kind === 'text') {
        currentTextBlockIdRef.current = block.id;
      }
      if (!currentThinkingIdRef.current && block.kind === 'thinking' && !block.done) {
        currentThinkingIdRef.current = block.id;
      }
      if (
        !currentToolBlockIdRef.current
        && block.kind === 'tool'
        && (block.status === 'running' || block.status === 'executing')
      ) {
        currentToolBlockIdRef.current = block.id;
      }
    }
  }, []);

  const restorePausedMessageToStreaming = useCallback((): AgentMessage => {
    const currentStreamingMessage = streamingMessageRef.current;
    if (
      currentStreamingMessage
      && (!pausedMessageIdRef.current || currentStreamingMessage.id === pausedMessageIdRef.current)
    ) {
      const resumedStreamingMessage = { ...currentStreamingMessage, isStreaming: true };
      pausedMessageIdRef.current = resumedStreamingMessage.id;
      setStreamingMessage(resumedStreamingMessage);
      primeStreamingCursorFromMessage(resumedStreamingMessage);
      return resumedStreamingMessage;
    }

    const currentMessages = messagesRef.current;
    let resumeIndex = pausedMessageIdRef.current
      ? currentMessages.findIndex((message) => message.id === pausedMessageIdRef.current)
      : -1;

    if (resumeIndex < 0) {
      for (let idx = currentMessages.length - 1; idx >= 0; idx -= 1) {
        if (currentMessages[idx].role === 'assistant') {
          resumeIndex = idx;
          break;
        }
      }
    }

    const resumeMessage = resumeIndex >= 0
      ? { ...currentMessages[resumeIndex], isStreaming: true }
      : makeStreamingMessage();

    if (resumeIndex >= 0) {
      setMessages([
        ...currentMessages.slice(0, resumeIndex),
        ...currentMessages.slice(resumeIndex + 1),
      ]);
    }

    pausedMessageIdRef.current = resumeMessage.id;
    setStreamingMessage(resumeMessage);
    primeStreamingCursorFromMessage(resumeMessage);
    return resumeMessage;
  }, [primeStreamingCursorFromMessage]);

  const resolveToolBlockId = useCallback((toolCallId?: string | null): string | null => {
    if (toolCallId) {
      return toolBlockIdsByCallIdRef.current.get(toolCallId)
        ?? (currentToolCallIdRef.current === toolCallId ? currentToolBlockIdRef.current : null);
    }
    return currentToolBlockIdRef.current;
  }, []);

  // ── Finalize streaming ──────────────────────────────────────────────────────
  const finalizeStream = useCallback((
    msg?: AgentMessage | null,
    options?: {
      statusText?: string;
      statusKind?: StatusState['kind'];
      allowQueueAdvance?: boolean;
      canResumeAfter?: boolean;
    },
  ) => {
    const {
      statusText = 'Ready',
      statusKind = 'ready',
      allowQueueAdvance = true,
      canResumeAfter = false,
    } = options ?? {};
    const final = msg ?? null;
    if (final) {
      const completed = { ...final, isStreaming: false };
      pausedMessageIdRef.current = canResumeAfter ? completed.id : null;
      setMessages((prev) => [...prev, completed]);
    } else if (!canResumeAfter) {
      pausedMessageIdRef.current = null;
    }
    setStreamingMessage(null);
    setIsProcessing(false);
    setIsStopping(false);
    setIsResuming(false);
    setCanResume(canResumeAfter);
    isProcessingRef.current = false;
    resetStreamingCursor();
    updateStatus({ text: statusText, kind: statusKind });

    // Advance queue
    if (allowQueueAdvance) {
      const next = messageQueueRef.current.shift();
      if (next && triggerSendRef.current) {
        void triggerSendRef.current(next.text, next.taskId);
      }
    }
  }, [resetStreamingCursor, updateStatus]);

  const bumpMemoryRevision = useCallback(() => {
    setMemoryRevision((prev) => prev + 1);
  }, []);

  const seedStreamingPlaceholder = useCallback((): string => {
    if (streamingMsgIdRef.current) {
      return streamingMsgIdRef.current;
    }

    const msg = makeStreamingMessage();

    streamingMsgIdRef.current = msg.id;
    currentThinkingIdRef.current = null;
    currentTextBlockIdRef.current = null;
    currentToolBlockIdRef.current = null;
    currentToolCallIdRef.current = null;
    toolBlockIdsByCallIdRef.current.clear();
    setStreamingMessage(msg);
    return msg.id;
  }, []);

  // ── Ensure streaming message exists ────────────────────────────────────────
  const ensureStreamingMsg = useCallback((): string => {
    if (streamingMsgIdRef.current) {
      const existingId = streamingMsgIdRef.current;
      setStreamingMessage((prev) => prev ?? makeStreamingMessageWithId(existingId));
      return existingId;
    }

    const newMsg = makeStreamingMessage();
    streamingMsgIdRef.current = newMsg.id;
    setStreamingMessage((prev) => prev ?? newMsg);
    return newMsg.id;
  }, []);

  // ── Block updater helpers ───────────────────────────────────────────────────
  const updateStreamingBlocks = useCallback((updater: (blocks: MsgBlock[]) => MsgBlock[]) => {
    setStreamingMessage((prev) => {
      if (prev) {
        return { ...prev, blocks: updater(prev.blocks) };
      }

      const streamingMsgId = streamingMsgIdRef.current;
      if (!streamingMsgId) return prev;

      const base = makeStreamingMessageWithId(streamingMsgId);
      return { ...base, blocks: updater(base.blocks) };
    });
  }, []);

  const bindReusableShellSessionToStreamingMessage = useCallback(() => {
    const reusableShell = findSingleReusableShellSession(shellSessionsRef.current);
    if (!reusableShell) {
      return;
    }

    const activityAt = Date.now();
    updateStreamingBlocks((blocks) => {
      if (blocks.some((block) => block.kind === 'shell' || block.kind === 'shell-session')) {
        return blocks;
      }

      return [...blocks, createPendingReusableShellSessionBlock(reusableShell, activityAt)];
    });
  }, [updateStreamingBlocks]);

  // ── Stream event dispatcher ─────────────────────────────────────────────────
  const handleStreamEvent = useCallback((action: StreamEventAction) => {
    if (
      retryStatusRestoreRef.current
      && action.type !== 'retry'
      && action.type !== 'status'
      && action.type !== 'done'
      && action.type !== 'paused'
      && action.type !== 'cancelled'
      && action.type !== 'resumed'
      && action.type !== 'error'
    ) {
      restoreStatusAfterRetry();
    }

    switch (action.type) {
      case 'thinking': {
        ensureStreamingMsg();
        const thinkId = currentThinkingIdRef.current;
        if (thinkId) {
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === thinkId && b.kind === 'thinking'
              ? { ...b, content: b.content + action.chunk }
              : b)
          );
        } else {
          const id = nanoid();
          currentThinkingIdRef.current = id;
          const block: ThinkingBlock = { kind: 'thinking', id, content: action.chunk, done: false, collapsed: false };
          updateStreamingBlocks((blocks) => [block, ...blocks]);
        }
        break;
      }

      case 'chunk': {
        ensureStreamingMsg();
        // Finish & collapse any open thinking block
        if (currentThinkingIdRef.current) {
          const tid = currentThinkingIdRef.current;
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'thinking' ? { ...b, done: true, collapsed: true } : b)
          );
          currentThinkingIdRef.current = null;
        }
        setStreamingMessage((prev) => prev ? { ...prev, rawMarkdown: prev.rawMarkdown + action.chunk } : prev);
        const textId = currentTextBlockIdRef.current;
        if (textId) {
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === textId && b.kind === 'text'
              ? { ...b, content: b.content + action.chunk }
              : b)
          );
        } else {
          // Auto-collapse completed tool blocks when new text content starts
          updateStreamingBlocks((blocks) =>
            blocks.map((b) =>
              b.kind === 'tool' && (b.status === 'success' || b.status === 'error')
                ? { ...b, collapsed: true }
                : b
            )
          );
          const id = nanoid();
          currentTextBlockIdRef.current = id;
          const block: TextBlock = { kind: 'text', id, content: action.chunk };
          updateStreamingBlocks((blocks) => [...blocks, block]);
        }
        break;
      }

      case 'narration': {
        ensureStreamingMsg();
        if (currentThinkingIdRef.current) {
          const tid = currentThinkingIdRef.current;
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'thinking'
              ? { ...b, done: true, collapsed: true }
              : b)
          );
          currentThinkingIdRef.current = null;
        }
        currentTextBlockIdRef.current = null;
        const block: NarrationBlock = {
          kind: 'narration',
          id: nanoid(),
          title: action.title ?? null,
          message: action.message,
          summary: action.summary ?? null,
          reason: action.reason ?? null,
          nextStep: action.nextStep ?? null,
          evidence: action.evidence,
          stage: action.stage,
          source: 'llm',
        };
        updateStreamingBlocks((blocks) => {
          const last = blocks[blocks.length - 1];
          if (last?.kind === 'narration' && last.source === 'review-fallback') {
            return [...blocks.slice(0, -1), block];
          }
          if (
            last?.kind === 'narration' &&
            last.message === action.message &&
            last.stage === action.stage &&
            (last.summary ?? null) === (action.summary ?? null) &&
            (last.reason ?? null) === (action.reason ?? null) &&
            (last.nextStep ?? null) === (action.nextStep ?? null) &&
            last.evidence.join('|') === action.evidence.join('|')
          ) {
            return blocks;
          }
          return [...blocks, block];
        });
        break;
      }

      case 'tool-confirmation':
        confirmationQueueRef.current.push(action.payload);
        if (!pendingConfirmationRef.current) {
          const next = confirmationQueueRef.current.shift();
          if (next) {
            pendingConfirmationRef.current = next;
            setPendingConfirmation(next);
          }
        }
        break;

      case 'tool-blocked': {
        const id = nanoid();
        const block: ToolBlock = { kind: 'tool', id, name: action.toolName, args: '', status: 'blocked', collapsed: true };
        ensureStreamingMsg();
        updateStreamingBlocks((blocks) => [...blocks, block]);
        currentTextBlockIdRef.current = null;
        break;
      }

      case 'agent-iteration': {
        if (action.iteration > 0) {
          ensureStreamingMsg();
          const reviewNarration = buildIterationReviewNarration(lastToolContextRef.current);
          if (!reviewNarration) {
            lastIterationMarkerSignatureRef.current = null;
            updateStreamingBlocks((blocks) => [
              ...blocks.map((b) =>
                b.kind === 'tool' && (b.status === 'success' || b.status === 'error') ? { ...b, collapsed: true } : b
              ),
            ]);
            currentTextBlockIdRef.current = null;
            break;
          }
          const signature = iterationMarkerSignature(
            lastToolContextVersionRef.current,
            { label: reviewNarration.title, detail: reviewNarration.message },
          );
          if (lastIterationMarkerSignatureRef.current === signature) break;
          lastIterationMarkerSignatureRef.current = signature;
          const block: NarrationBlock = {
            kind: 'narration',
            id: nanoid(),
            title: reviewNarration.title,
            message: reviewNarration.message,
            evidence: [],
            stage: reviewNarration.stage,
            source: 'review-fallback',
          };
          updateStreamingBlocks((blocks) => [
            ...blocks.map((b) =>
              b.kind === 'tool' && (b.status === 'success' || b.status === 'error') ? { ...b, collapsed: true } : b
            ),
            block,
          ]);
          currentTextBlockIdRef.current = null;
        }
        break;
      }

      case 'tool-start': {
        ensureStreamingMsg();
        if (currentThinkingIdRef.current) {
          const tid = currentThinkingIdRef.current;
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'thinking'
              ? { ...b, done: true, collapsed: b.collapsed || !b.content.trim() }
              : b)
          );
          currentThinkingIdRef.current = null;
        }
        const id = nanoid();
        currentToolBlockIdRef.current = id;
        currentToolCallIdRef.current = action.toolCallId ?? null;
        if (action.toolCallId) {
          toolBlockIdsByCallIdRef.current.set(action.toolCallId, id);
        }
        currentTextBlockIdRef.current = null;
        // Initialise context for this tool — args and result filled as they stream in
        lastToolContextRef.current = {
          name: action.toolName,
          success: false,
          output: null,
          args: '',
          completed: false,
        };
        lastIterationMarkerSignatureRef.current = null;
        const block: ToolBlock = { kind: 'tool', id, name: action.toolName, args: '', status: 'running', collapsed: true };
        updateStreamingBlocks((blocks) => [...blocks, block]);
        if (action.toolName === 'shell') {
          bindReusableShellSessionToStreamingMessage();
        }
        break;
      }

      case 'tool-args': {
        const tid = resolveToolBlockId(action.toolCallId);
        if (tid) {
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'tool' ? { ...b, args: b.args + action.args } : b)
          );
        }
        // Keep the context args in sync so buildIterationLabel can parse them
        if (lastToolContextRef.current) {
          lastToolContextRef.current = { ...lastToolContextRef.current, args: lastToolContextRef.current.args + action.args };
        }
        break;
      }

      case 'tool-end': {
        const tid = resolveToolBlockId(action.toolCallId);
        if (tid) {
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'tool' ? { ...b, status: 'executing' } : b)
          );
        }
        break;
      }

      case 'tool-result': {
        const tid = resolveToolBlockId(action.toolCallId);
        if (tid) {
          updateStreamingBlocks((blocks) =>
            blocks.map((b) =>
              b.id === tid && b.kind === 'tool'
                ? { ...b, status: action.success ? 'success' : 'error', result: action.output, durationMs: action.durationMs }
                : b
            )
          );
          if (action.toolCallId) {
            toolBlockIdsByCallIdRef.current.delete(action.toolCallId);
            if (currentToolCallIdRef.current === action.toolCallId) {
              currentToolCallIdRef.current = null;
              currentToolBlockIdRef.current = null;
            }
          } else {
            currentToolCallIdRef.current = null;
            currentToolBlockIdRef.current = null;
          }
        }
        // Finalise context so the upcoming agent-iteration can produce a rich label
        if (lastToolContextRef.current) {
          lastToolContextRef.current = {
            ...lastToolContextRef.current,
            success: action.success,
            output: action.output,
            completed: true,
          };
          lastToolContextVersionRef.current += 1;
        }
        if (action.name === 'gui_control' && action.success && lastToolContextRef.current) {
          try {
            const parsedArgs = JSON.parse(lastToolContextRef.current.args);
            if (typeof parsedArgs.action === 'string') {
              window.dispatchEvent(new CustomEvent('gestura:gui_control', { detail: { action: parsedArgs.action, target: parsedArgs.target } }));
            }
          } catch { /* ignore parse errors */ }
        }
        break;
      }

      case 'shell-session-lifecycle': {
        ensureStreamingMsg();
        if (currentThinkingIdRef.current) {
          const tid = currentThinkingIdRef.current;
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'thinking'
              ? { ...b, done: true, collapsed: b.collapsed || !b.content.trim() }
              : b)
          );
          currentThinkingIdRef.current = null;
        }
        const activityAt = Date.now();
        updateStreamingBlocks((blocks) => updateShellSessionBlock(
          blocks,
          action.shellSessionId,
          (current) => applyShellSessionLifecyclePayload(current, action.payload, activityAt),
        ));
        currentTextBlockIdRef.current = null;
        break;
      }

      case 'shell-lifecycle': {
        ensureStreamingMsg();
        if (currentThinkingIdRef.current) {
          const tid = currentThinkingIdRef.current;
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'thinking'
              ? { ...b, done: true, collapsed: b.collapsed || !b.content.trim() }
              : b)
          );
          currentThinkingIdRef.current = null;
        }
        const pid = action.processId;
        const p = action.payload;
        const shellSessionId = p['shell_session_id'] != null ? String(p['shell_session_id']) : null;
        const activityAt = Date.now();
        if (shellSessionId) {
          updateStreamingBlocks((blocks) => updateShellSessionBlock(
            blocks,
            shellSessionId,
            (current) => applyShellLifecyclePayload(current, p, activityAt),
          ));
          currentTextBlockIdRef.current = null;
          break;
        }
        updateStreamingBlocks((blocks) => {
          const idx = findShellBlockIndex(blocks, pid, shellSessionId);
          const nextState = normalizeShellState(p['state']);
          const nextCommand = p['command'] != null ? String(p['command']) : '';
          if (idx >= 0) {
            const existing = blocks[idx] as ShellBlock;
            const command = nextCommand || existing.command;
            const commandLine = nextState === 'Started' && command !== existing.command
              ? buildShellCommandLine(command)
              : null;
            const updated: ShellBlock = {
              ...existing,
              processId: pid || existing.processId,
              shellSessionId: shellSessionId ?? (existing.shellSessionId ?? null),
              command,
              cwd: p['cwd'] != null ? String(p['cwd']) : existing.cwd,
              state: nextState,
              exitCode: p['exit_code'] != null ? Number(p['exit_code']) : existing.exitCode,
              durationMs: p['duration_ms'] != null ? Number(p['duration_ms']) : existing.durationMs,
              startedAt: existing.startedAt ?? activityAt,
              lastActivityAt: activityAt,
              lines: mergeShellCommandLine(existing.lines, commandLine),
            };
            return blocks.map((b, i) => i === idx ? updated : b);
          } else {
            const commandLine = buildShellCommandLine(nextCommand);
            const newBlock: ShellBlock = {
              kind: 'shell', id: nanoid(), processId: pid,
              shellSessionId,
              command: nextCommand, cwd: p['cwd'] ? String(p['cwd']) : null,
              state: nextState, lines: commandLine ? [commandLine] : [], collapsed: true,
              startedAt: activityAt,
              lastActivityAt: activityAt,
            };
            currentTextBlockIdRef.current = null;
            return [...blocks, newBlock];
          }
        });
        break;
      }

      case 'shell-output': {
        ensureStreamingMsg();
        if (currentThinkingIdRef.current) {
          const tid = currentThinkingIdRef.current;
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'thinking'
              ? { ...b, done: true, collapsed: b.collapsed || !b.content.trim() }
              : b)
          );
          currentThinkingIdRef.current = null;
        }
        const pid = action.processId || action.shellSessionId || '';
        if (!pid) break;
        const activityAt = Date.now();
        const shellSessionId = action.shellSessionId;
        if (shellSessionId) {
          updateStreamingBlocks((blocks) => updateShellSessionBlock(
            blocks,
            shellSessionId,
            (current) => applyShellOutputPayload(current, {
              shell_session_id: shellSessionId,
              stream: action.stream,
              data: action.data,
            }, activityAt),
          ));
          currentTextBlockIdRef.current = null;
          break;
        }
        updateStreamingBlocks((blocks) => {
          const idx = findShellBlockIndex(blocks, action.processId, action.shellSessionId);
          if (idx >= 0) {
            return blocks.map((block, blockIndex) => {
              if (blockIndex !== idx || block.kind !== 'shell') return block;
              return {
                ...block,
                processId: action.processId || block.processId,
                shellSessionId: action.shellSessionId ?? (block.shellSessionId ?? null),
                state: block.state === 'Completed' || block.state === 'Failed' || block.state === 'Stopped'
                  ? block.state
                  : 'Running',
                startedAt: block.startedAt ?? activityAt,
                lastActivityAt: activityAt,
                lines: [...block.lines, { stream: action.stream, data: action.data }],
              };
            });
          }

          const newBlock: ShellBlock = {
            kind: 'shell',
            id: nanoid(),
            processId: pid,
            shellSessionId: action.shellSessionId ?? null,
            command: '',
            cwd: null,
            state: 'Running',
            lines: [{ stream: action.stream, data: action.data }],
            collapsed: true,
            startedAt: activityAt,
            lastActivityAt: activityAt,
          };
          currentTextBlockIdRef.current = null;
          return [...blocks, newBlock];
        });
        break;
      }

      case 'status':
        updateStatus({ text: action.text, kind: action.kind as StatusState['kind'] });
        if (action.kind === 'reflection' && lastReflectionNoticeRef.current !== action.text) {
          lastReflectionNoticeRef.current = action.text;
          const id = nanoid();
          const notice: TextBlock = { kind: 'text', id, content: `*${action.text}*` };
          setMessages((prev) => [...prev, {
            id: nanoid(),
            role: 'assistant',
            rawMarkdown: action.text,
            blocks: [notice],
            isStreaming: false,
            timestamp: Date.now(),
          }]);
        }
        break;

      case 'retry':
        showRetryStatus(action.attempt);
        break;

      case 'context-compacted': {
        const id = nanoid();
        const notice: TextBlock = { kind: 'text', id, content: `*Context compacted: ${action.summary}*` };
        setMessages((prev) => [...prev, { id: nanoid(), role: 'assistant', rawMarkdown: '', blocks: [notice], isStreaming: false, timestamp: Date.now() }]);
        break;
      }

      case 'done':
        setStreamingMessage((prev) => {
          finalizeStream(prev, { statusText: 'Ready', statusKind: 'ready', canResumeAfter: false });
          return null;
        });
        bumpMemoryRevision();
        break;

      case 'paused':
        setStreamingMessage((prev) => {
          const paused = prev ? { ...prev, isStreaming: false } : prev;
          if (paused) {
            pausedMessageIdRef.current = paused.id;
          }
          setIsProcessing(false);
          setIsStopping(false);
          setIsResuming(false);
          setCanResume(true);
          isProcessingRef.current = false;
          resetStreamingCursor();
          updateStatus({ text: 'Interrupted — resume available', kind: 'ready' });
          return paused;
        });
        break;

      case 'cancelled':
        setStreamingMessage((prev) => {
          finalizeStream(prev, {
            statusText: 'Cancelled',
            statusKind: 'ready',
            allowQueueAdvance: false,
            canResumeAfter: false,
          });
          return null;
        });
        break;

      case 'resumed':
        if (!streamingMsgIdRef.current) {
          restorePausedMessageToStreaming();
        }
        setCanResume(false);
        setIsStopping(false);
        setIsResuming(true);
        updateStatus({ text: 'Resuming…', kind: 'busy' });
        break;

      case 'error': {
        setStreamingMessage((prev) => {
          finalizeStream(prev, {
            statusText: `Error: ${action.message}`,
            statusKind: 'error',
            canResumeAfter: false,
          });
          return null;
        });
        const errId = nanoid();
        const errBlock: TextBlock = { kind: 'text', id: errId, content: `⚠️ **Error:** ${action.message}` };
        setMessages((prev) => [...prev, {
          id: nanoid(),
          role: 'assistant',
          rawMarkdown: `⚠️ **Error:** ${action.message}`,
          blocks: [errBlock],
          isStreaming: false,
          timestamp: Date.now(),
        }]);
        break;
      }

      case 'agent-message': {
        const content = action.content.trim();
        if (!content) break;

        if (action.role === 'user') {
          if (isProcessingRef.current || !triggerSendRef.current) {
            messageQueueRef.current.push({ kind: 'voice', text: content, taskId: null });
            break;
          }

          void triggerSendRef.current(content, null);
          break;
        }

        const id = nanoid();
        const block: TextBlock = { kind: 'text', id, content };
        setMessages((prev) => [...prev, { id: nanoid(), role: 'assistant', rawMarkdown: content, blocks: [block], isStreaming: false, timestamp: Date.now() }]);
        break;
      }

      case 'listening-state':
        setIsListening(action.listening);
        break;

      case 'task-runtime-state':
        setRuntimeTaskSnapshot(action.snapshot);
        break;

      case 'task-changed':
        getTaskHierarchy(sessionId).then(setTasks).catch(() => { });
        bumpMemoryRevision();
        break;

      default:
        break;
    }
  }, [
    ensureStreamingMsg,
    updateStreamingBlocks,
    bindReusableShellSessionToStreamingMessage,
    finalizeStream,
    restorePausedMessageToStreaming,
    resetStreamingCursor,
    resolveToolBlockId,
    sessionId,
    bumpMemoryRevision,
    restoreStatusAfterRetry,
    showRetryStatus,
    updateStatus,
  ]);

  useStreamEvents(sessionId, handleStreamEvent);

  useEffect(() => {
    if (shellSessions.length === 0) {
      return;
    }

    setStreamingMessage((prev) => reconcileStreamingMessageShellSessions(prev, shellSessions, isProcessingRef.current));
  }, [isProcessing, shellSessions, streamingMessage?.id]);

  // ── Load session history on mount ───────────────────────────────────────────
  useEffect(() => {
    if (!sessionId) return;
    getSessionReplaySnapshot(sessionId)
      .then((snapshot) => {
        messagesRef.current = [];
        streamingMessageRef.current = null;
        isProcessingRef.current = false;
        resetStreamingCursor();
        pausedMessageIdRef.current = null;
        pendingConfirmationRef.current = null;
        setMessages([]);
        setStreamingMessage(null);
        setPendingConfirmation(null);
        setIsProcessing(false);
        setIsStopping(false);
        setIsResuming(false);
        setCanResume(false);
        updateStatus({ text: 'Ready', kind: 'ready' });

        if (snapshot.activity_log.length > 0) {
          snapshot.activity_log.forEach((entry) => {
            const userMessage = toReplayUserMessage(entry);
            if (userMessage) {
              messagesRef.current = [...messagesRef.current, userMessage];
              setMessages((prev) => [...prev, userMessage]);
              return;
            }

            const action = toReplayAction(entry);
            if (action) {
              handleStreamEvent(action);
            }
          });

          if (snapshot.has_paused_execution) {
            setCanResume(true);
            updateStatus({ text: 'Interrupted — resume available', kind: 'ready' });
          }
          return;
        }

        const msgs: AgentMessage[] = snapshot.history.map((h) => {
          const id = nanoid();
          const blocks: MsgBlock[] = [];
          if (h.thinking?.trim()) {
            blocks.push({
              kind: 'thinking',
              id: nanoid(),
              content: h.thinking,
              done: true,
              collapsed: true,
            });
          }
          if (h.content) {
            blocks.push({ kind: 'text', id: nanoid(), content: h.content });
          }
          return {
            id,
            role: h.role === 'user' ? 'user' : 'assistant',
            rawMarkdown: h.content,
            blocks,
            isStreaming: false,
            timestamp: h.timestamp ? Date.parse(h.timestamp) || Date.now() : Date.now(),
          };
        });
        messagesRef.current = msgs;
        setMessages(msgs);
        if (snapshot.has_paused_execution) {
          const resumable = [...msgs].reverse().find((message) => message.role === 'assistant') ?? null;
          pausedMessageIdRef.current = resumable?.id ?? null;
          setCanResume(true);
          updateStatus({ text: 'Interrupted — resume available', kind: 'ready' });
        }
      })
      .catch((e) => console.warn('[useChatSession] history load failed:', e));
  }, [handleStreamEvent, resetStreamingCursor, sessionId, updateStatus]);

  // ── Side-panel data refreshers (loaded lazily by the panels that need them) ─
  const refreshTasks = useCallback(async () => {
    try { setTasks(await getTaskHierarchy(sessionId)); } catch { /* ignore */ }
  }, [sessionId]);

  const refreshKnowledge = useCallback(async () => {
    try {
      const [items, enabledIds] = await Promise.all([
        listKnowledgeItems(),
        getEnabledKnowledge(sessionId),
      ]);
      const enabledSet = new Set(enabledIds);
      setKnowledgeItems(items.map((item) => ({ ...item, enabled: enabledSet.has(item.id) })));
    } catch { /* ignore */ }
  }, [sessionId]);

  const refreshToolSettings = useCallback(async () => {
    try {
      setToolSettings(await getSessionToolSettings(sessionId));
    } catch { /* ignore */ }
  }, [sessionId]);

  // ── Send message ────────────────────────────────────────────────────────────
  const triggerSend = useCallback(async (text: string, taskId: string | null) => {
    const pausedStreamingMessage = streamingMessageRef.current;
    if (pausedStreamingMessage && !isProcessingRef.current) {
      setMessages((prev) => [...prev, { ...pausedStreamingMessage, isStreaming: false }]);
      setStreamingMessage(null);
      resetStreamingCursor();
    }
    pausedMessageIdRef.current = null;
    setCanResume(false);
    setIsStopping(false);
    setIsResuming(false);
    isProcessingRef.current = true;
    setIsProcessing(true);
    updateStatus({ text: 'Thinking…', kind: 'busy' });
    const userMsg: AgentMessage = {
      id: nanoid(), role: 'user', rawMarkdown: text,
      blocks: [{ kind: 'text', id: nanoid(), content: text }],
      isStreaming: false, timestamp: Date.now(),
    };
    setMessages((prev) => [...prev, userMsg]);
    seedStreamingPlaceholder();

    await waitForNextPaint();

    void Promise.resolve(
      sendMessageStreaming({ session_id: sessionId, message: text, task_id: taskId ?? null }),
    ).catch((err) => {
      if (isStreamingPlaceholderMessage(streamingMessageRef.current)) {
        setStreamingMessage(null);
        resetStreamingCursor();
      }
      updateStatus({ text: `Error: ${String(err)}`, kind: 'error' });
      setIsProcessing(false);
      isProcessingRef.current = false;
    });
  }, [resetStreamingCursor, seedStreamingPlaceholder, sessionId, updateStatus]);

  useEffect(() => {
    triggerSendRef.current = triggerSend;
  }, [triggerSend]);

  // ── Cancel streaming ────────────────────────────────────────────────────────
  const cancelStream = useCallback(async () => {
    if (isStopping) return;

    setIsStopping(true);
    updateStatus({ text: 'Stopping…', kind: 'busy' });

    try {
      await pauseStreaming(sessionId);
    } catch (e) {
      console.warn('[cancelStream] pause command failed:', e);
      setIsStopping(false);
      updateStatus({ text: `Stop failed: ${String(e)}`, kind: 'error' });
    }
  }, [isStopping, sessionId, updateStatus]);

  const resumeStream = useCallback(async () => {
    if (!sessionId || isProcessingRef.current || !canResume) return;

    const seedMessage = restorePausedMessageToStreaming();
    setCanResume(false);
    setIsResuming(true);
    isProcessingRef.current = true;
    setIsProcessing(true);
    updateStatus({ text: 'Resuming…', kind: 'busy' });

    try {
      await resumeStreaming(sessionId);
    } catch (err) {
      console.warn('[useChatSession] resume failed:', err);
      setStreamingMessage(null);
      if (seedMessage.blocks.length > 0 || seedMessage.rawMarkdown) {
        const completed = { ...seedMessage, isStreaming: false };
        pausedMessageIdRef.current = completed.id;
        setMessages((prev) => [...prev, completed]);
        setCanResume(true);
      }
      setIsResuming(false);
      setIsProcessing(false);
      isProcessingRef.current = false;
      resetStreamingCursor();
      updateStatus({ text: `Resume failed: ${String(err)}`, kind: 'error' });
    }
  }, [sessionId, canResume, restorePausedMessageToStreaming, resetStreamingCursor, updateStatus]);

  const sendMessage = useCallback(async (text: string, taskId?: string | null) => {
    if (!text.trim()) return;
    if (isProcessingRef.current) {
      messageQueueRef.current.push({ kind: 'text', text, taskId: taskId ?? null });
      return;
    }

    await triggerSend(text, taskId ?? null);
  }, [triggerSend]);

  // ── Tool confirmation ────────────────────────────────────────────────────────
  const resolveConfirmation = useCallback(async (decision: ToolConfirmationDecision) => {
    const conf = pendingConfirmationRef.current;
    if (!conf) return;
    try {
      await resolveToolConfirmationDecision(
        conf.confirmation_id,
        decision,
        conf.session_id ?? sessionId,
      );
    } catch (e) {
      console.warn('[useChatSession] failed to resolve tool confirmation:', e);
      return;
    }
    pendingConfirmationRef.current = null;
    setPendingConfirmation(null);
    const next = confirmationQueueRef.current.shift();
    if (next) { pendingConfirmationRef.current = next; setPendingConfirmation(next); }
  }, [sessionId]);

  // ── Voice ────────────────────────────────────────────────────────────────────
  const toggleVoice = useCallback(async () => {
    if (isListening) {
      await stopVoiceListening(sessionId);
    } else {
      await startVoiceListening(sessionId);
    }
  }, [isListening, sessionId]);

  // ── Prompt enhance ───────────────────────────────────────────────────────────
  const enhanceText = useCallback((text: string) => enhancePrompt(sessionId, text), [sessionId]);

  return {
    messages, streamingMessage, isProcessing, isStopping, canResume, isResuming, isListening, status,
    pendingConfirmation, tasks, runtimeTaskSnapshot, knowledgeItems, toolSettings, memoryRevision,
    userScrolledUp, setUserScrolledUp,
    sendMessage, cancelStream, resumeStream, resolveConfirmation,
    toggleVoice, enhanceText, refreshTasks, refreshKnowledge, refreshToolSettings,
  };
}

