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
  cancelStreaming,
  getSessionHistory,
  getTaskHierarchy,
  listKnowledgeItems,
  getEnabledKnowledge,
  getSessionToolSettings,
  listBuiltinTools,
  listDiscoveredMcpTools,
  resolveToolConfirmationDecision,
  enhancePrompt,
  startVoiceListening,
  stopVoiceListening,
} from '../../../services/tauri/agent';
import type {
  AgentMessage,
  MsgBlock,
  ThinkingBlock,
  TextBlock,
  ToolBlock,
  ShellBlock,
  IterationMarkerBlock,
  ToolConfirmation,
  ToolConfirmationDecision,
  TaskHierarchy,
  KnowledgeItem,
  StatusState,
} from '../types';

// ─── Queued item ──────────────────────────────────────────────────────────────

interface QueuedMessage {
  kind: 'text' | 'voice';
  text: string;
  taskId: string | null;
}

// ─── Return type ─────────────────────────────────────────────────────────────

export interface ChatSessionState {
  messages: AgentMessage[];
  streamingMessage: AgentMessage | null;
  isProcessing: boolean;
  isListening: boolean;
  status: StatusState;
  pendingConfirmation: ToolConfirmation | null;
  tasks: TaskHierarchy;
  knowledgeItems: KnowledgeItem[];
  toolSettings: Record<string, unknown>;
  userScrolledUp: boolean;
  setUserScrolledUp: (v: boolean) => void;
  sendMessage: (text: string, taskId?: string | null) => Promise<void>;
  cancelStream: () => Promise<void>;
  resolveConfirmation: (decision: ToolConfirmationDecision) => Promise<void>;
  toggleVoice: () => Promise<void>;
  enhanceText: (text: string) => Promise<string>;
  refreshTasks: () => Promise<void>;
  refreshKnowledge: () => Promise<void>;
  refreshToolSettings: () => Promise<void>;
}

// ─── Helper — build a contextual iteration marker label ───────────────────────

interface LastToolContext {
  name: string;
  success: boolean;
  output: string | null;
  args: string;
}

function buildIterationLabel(ctx: LastToolContext | null): { label: string; detail?: string } {
  if (!ctx) return { label: '◆ Reviewing results…' };

  const { name, success, args } = ctx;
  const errorNote = success ? '' : ' (with errors)';

  // Try to parse accumulated args JSON for richer context
  let parsedArgs: Record<string, unknown> = {};
  try {
    const cleaned = args.trim();
    if (cleaned) parsedArgs = JSON.parse(cleaned) as Record<string, unknown>;
  } catch { /* partial / streaming JSON — ignore */ }

  switch (name) {
    case 'web': {
      const url = String(parsedArgs['url'] ?? parsedArgs['uri'] ?? '');
      let host = '';
      try { host = url ? ` from ${new URL(url).hostname}` : ''; } catch { /* ignore */ }
      return {
        label: `◆ Reviewing web content${host}${errorNote}`,
        detail: `Scanning the fetched page for relevant content, facts, and actionable information…`,
      };
    }
    case 'web_search': {
      const query = String(parsedArgs['query'] ?? parsedArgs['q'] ?? '');
      const queryStr = query ? ` for "${query.slice(0, 60)}"` : '';
      return {
        label: `◆ Reviewing search results${queryStr}${errorNote}`,
        detail: `Evaluating results to identify the most relevant matches and extract key information…`,
      };
    }
    case 'file': {
      const path = String(parsedArgs['path'] ?? '');
      const fileName = path ? (path.split('/').pop() ?? path) : '';
      const fileStr = fileName ? ` — ${fileName}` : '';
      const op = String(parsedArgs['action'] ?? parsedArgs['operation'] ?? '');
      const isWrite = op === 'write' || op === 'create' || op === 'append';
      return {
        label: `◆ Reviewing file result${fileStr}${errorNote}`,
        detail: isWrite
          ? `Confirming the file was written correctly and checking for any issues…`
          : `Reading through file contents to extract the needed information…`,
      };
    }
    case 'shell': {
      const cmd = String(parsedArgs['command'] ?? parsedArgs['cmd'] ?? '');
      const shortCmd = cmd ? `\`${cmd.split(' ')[0]}\`` : 'command';
      return {
        label: `◆ Reviewing ${shortCmd} output${errorNote}`,
        detail: success
          ? `Processing command output to extract results and plan next steps…`
          : `Analyzing the error output to determine what went wrong and how to proceed…`,
      };
    }
    case 'git': {
      const op = String(parsedArgs['action'] ?? parsedArgs['operation'] ?? parsedArgs['command'] ?? '');
      return {
        label: `◆ Reviewing git ${op || 'result'}${errorNote}`,
        detail: `Checking the git output to understand repository state and determine next actions…`,
      };
    }
    case 'code': {
      const op = String(parsedArgs['action'] ?? parsedArgs['operation'] ?? '');
      return {
        label: `◆ Reviewing code analysis${op ? ` (${op})` : ''}${errorNote}`,
        detail: `Analyzing code structure, symbols, and relationships to inform the next step…`,
      };
    }
    case 'mcp': {
      const toolName = String(parsedArgs['tool'] ?? parsedArgs['tool_name'] ?? '');
      return {
        label: `◆ Reviewing MCP result${toolName ? ` from ${toolName}` : ''}${errorNote}`,
        detail: `Processing the tool response to extract relevant data and decide on next actions…`,
      };
    }
    case 'screenshot':
    case 'screen_record': {
      return {
        label: `◆ Reviewing screen capture${errorNote}`,
        detail: `Analyzing the captured screenshot for context and relevant visual information…`,
      };
    }
    case 'task': {
      return {
        label: `◆ Reviewing task update${errorNote}`,
        detail: `Confirming the task state is correct and planning the next action…`,
      };
    }
    default: {
      return {
        label: `◆ Reviewing ${name} result${errorNote}`,
        detail: `Processing the result to extract useful information and determine next steps…`,
      };
    }
  }
}

// ─── Helper — produce a fresh streaming AgentMessage ─────────────────────────

function makeStreamingMessage(): AgentMessage {
  return { id: nanoid(), role: 'assistant', rawMarkdown: '', blocks: [], isStreaming: true, timestamp: Date.now() };
}

// ─── Hook ─────────────────────────────────────────────────────────────────────

export function useChatSession(sessionId: string): ChatSessionState {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [streamingMessage, setStreamingMessage] = useState<AgentMessage | null>(null);
  const [isProcessing, setIsProcessing] = useState(false);
  const [isListening, setIsListening] = useState(false);
  const [status, setStatus] = useState<StatusState>({ text: 'Ready', kind: 'ready' });
  const [pendingConfirmation, setPendingConfirmation] = useState<ToolConfirmation | null>(null);
  const [tasks, setTasks] = useState<TaskHierarchy>([]);
  const [knowledgeItems, setKnowledgeItems] = useState<KnowledgeItem[]>([]);
  const [toolSettings, setToolSettings] = useState<Record<string, unknown>>({});
  const [userScrolledUp, setUserScrolledUp] = useState(false);

  // Streaming cursor refs (avoid stale closures in event listeners)
  const currentThinkingIdRef = useRef<string | null>(null);
  const currentTextBlockIdRef = useRef<string | null>(null);
  const currentToolBlockIdRef = useRef<string | null>(null);
  const streamingMsgIdRef = useRef<string | null>(null);
  const messageQueueRef = useRef<QueuedMessage[]>([]);
  const isProcessingRef = useRef(false);
  const confirmationQueueRef = useRef<ToolConfirmation[]>([]);
  const pendingConfirmationRef = useRef<ToolConfirmation | null>(null);
  const triggerSendRef = useRef<((text: string, taskId: string | null) => Promise<void>) | null>(null);
  /** Tracks the most recently started/completed tool for contextual iteration markers. */
  const lastToolContextRef = useRef<LastToolContext | null>(null);

  // ── Finalize streaming ──────────────────────────────────────────────────────
  const finalizeStream = useCallback((msg?: AgentMessage | null) => {
    const final = msg ?? null;
    if (final) {
      setMessages((prev) => [...prev, { ...final, isStreaming: false }]);
    }
    setStreamingMessage(null);
    setIsProcessing(false);
    isProcessingRef.current = false;
    streamingMsgIdRef.current = null;
    currentThinkingIdRef.current = null;
    currentTextBlockIdRef.current = null;
    currentToolBlockIdRef.current = null;
    setStatus({ text: 'Ready', kind: 'ready' });

    // Advance queue
    const next = messageQueueRef.current.shift();
    if (next && triggerSendRef.current) {
      void triggerSendRef.current(next.text, next.taskId);
    }
  }, []);

  // ── Ensure streaming message exists ────────────────────────────────────────
  const ensureStreamingMsg = useCallback((): string => {
    if (!streamingMsgIdRef.current) {
      const newMsg = makeStreamingMessage();
      streamingMsgIdRef.current = newMsg.id;
      setStreamingMessage(newMsg);
      return newMsg.id;
    }
    return streamingMsgIdRef.current;
  }, []);

  // ── Block updater helpers ───────────────────────────────────────────────────
  const updateStreamingBlocks = useCallback((updater: (blocks: MsgBlock[]) => MsgBlock[]) => {
    setStreamingMessage((prev) => {
      if (!prev) return prev;
      return { ...prev, blocks: updater(prev.blocks) };
    });
  }, []);

  // ── Stream event dispatcher ─────────────────────────────────────────────────
  const handleStreamEvent = useCallback((action: StreamEventAction) => {
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
        const block: ToolBlock = { kind: 'tool', id, name: action.toolName, args: '', status: 'blocked', collapsed: false };
        ensureStreamingMsg();
        updateStreamingBlocks((blocks) => [...blocks, block]);
        currentTextBlockIdRef.current = null;
        break;
      }

      case 'agent-iteration': {
        if (action.iteration > 0) {
          ensureStreamingMsg();
          const markerId = nanoid();
          const { label, detail } = buildIterationLabel(lastToolContextRef.current);
          const marker: IterationMarkerBlock = { kind: 'iteration-marker', id: markerId, label, detail };
          updateStreamingBlocks((blocks) => [
            ...blocks.map((b) =>
              b.kind === 'tool' && (b.status === 'success' || b.status === 'error') ? { ...b, collapsed: true } : b
            ),
            marker,
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
            blocks.map((b) => b.id === tid && b.kind === 'thinking' ? { ...b, done: true } : b)
          );
          currentThinkingIdRef.current = null;
        }
        const id = nanoid();
        currentToolBlockIdRef.current = id;
        currentTextBlockIdRef.current = null;
        // Initialise context for this tool — args and result filled as they stream in
        lastToolContextRef.current = { name: action.toolName, success: false, output: null, args: '' };
        const block: ToolBlock = { kind: 'tool', id, name: action.toolName, args: '', status: 'running', collapsed: false };
        updateStreamingBlocks((blocks) => [...blocks, block]);
        break;
      }

      case 'tool-args': {
        const tid = currentToolBlockIdRef.current;
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
        const tid = currentToolBlockIdRef.current;
        if (tid) {
          updateStreamingBlocks((blocks) =>
            blocks.map((b) => b.id === tid && b.kind === 'tool' ? { ...b, status: 'executing' } : b)
          );
        }
        break;
      }

      case 'tool-result': {
        const tid = currentToolBlockIdRef.current;
        if (tid) {
          updateStreamingBlocks((blocks) =>
            blocks.map((b) =>
              b.id === tid && b.kind === 'tool'
                ? { ...b, status: action.success ? 'success' : 'error', result: action.output, durationMs: action.durationMs }
                : b
            )
          );
          currentToolBlockIdRef.current = null;
        }
        // Finalise context so the upcoming agent-iteration can produce a rich label
        if (lastToolContextRef.current) {
          lastToolContextRef.current = { ...lastToolContextRef.current, success: action.success, output: action.output };
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

      case 'shell-lifecycle': {
        ensureStreamingMsg();
        const pid = action.processId;
        const p = action.payload;
        updateStreamingBlocks((blocks) => {
          const idx = blocks.findIndex((b) => b.kind === 'shell' && b.processId === pid);
          if (idx >= 0) {
            const existing = blocks[idx] as ShellBlock;
            const updated: ShellBlock = {
              ...existing,
              state: String(p['state'] ?? existing.state) as ShellBlock['state'],
              exitCode: p['exit_code'] != null ? Number(p['exit_code']) : existing.exitCode,
              durationMs: p['duration_ms'] != null ? Number(p['duration_ms']) : existing.durationMs,
            };
            return blocks.map((b, i) => i === idx ? updated : b);
          } else {
            const newBlock: ShellBlock = {
              kind: 'shell', id: nanoid(), processId: pid,
              command: String(p['command'] ?? ''), cwd: p['cwd'] ? String(p['cwd']) : null,
              state: 'Started', lines: [], collapsed: false,
            };
            currentTextBlockIdRef.current = null;
            return [...blocks, newBlock];
          }
        });
        break;
      }

      case 'shell-output': {
        const pid = action.processId;
        updateStreamingBlocks((blocks) =>
          blocks.map((b) =>
            b.kind === 'shell' && b.processId === pid
              ? { ...b, lines: [...b.lines, { stream: action.stream, data: action.data }] }
              : b
          )
        );
        break;
      }

      case 'status':
        setStatus({ text: action.text, kind: action.kind as StatusState['kind'] });
        break;

      case 'retry':
        setStatus({ text: `Retrying (attempt ${action.attempt})…`, kind: 'busy' });
        break;

      case 'context-compacted': {
        const id = nanoid();
        const notice: TextBlock = { kind: 'text', id, content: `*Context compacted: ${action.summary}*` };
        setMessages((prev) => [...prev, { id: nanoid(), role: 'assistant', rawMarkdown: '', blocks: [notice], isStreaming: false, timestamp: Date.now() }]);
        break;
      }

      case 'done':
        setStreamingMessage((prev) => { finalizeStream(prev); return null; });
        break;

      case 'cancelled':
        setStreamingMessage((prev) => { finalizeStream(prev); return null; });
        setStatus({ text: 'Cancelled', kind: 'ready' });
        break;

      case 'error': {
        setStreamingMessage((prev) => { finalizeStream(prev); return null; });
        setStatus({ text: `Error: ${action.message}`, kind: 'error' });
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
        const id = nanoid();
        const block: TextBlock = { kind: 'text', id, content: action.content };
        setMessages((prev) => [...prev, { id: nanoid(), role: 'assistant', rawMarkdown: action.content, blocks: [block], isStreaming: false, timestamp: Date.now() }]);
        break;
      }

      case 'listening-state':
        setIsListening(action.listening);
        break;

      case 'task-changed':
        getTaskHierarchy(sessionId).then(setTasks).catch(() => { });
        break;

      default:
        break;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ensureStreamingMsg, updateStreamingBlocks, finalizeStream]);

  useStreamEvents(sessionId, handleStreamEvent);

  // ── Load session history on mount ───────────────────────────────────────────
  useEffect(() => {
    if (!sessionId) return;
    getSessionHistory(sessionId)
      .then((history) => {
        const msgs: AgentMessage[] = history.map((h) => {
          const id = nanoid();
          const block: TextBlock = { kind: 'text', id, content: h.content };
          return { id, role: h.role === 'user' ? 'user' : 'assistant', rawMarkdown: h.content, blocks: [block], isStreaming: false, timestamp: Date.now() };
        });
        setMessages(msgs);
      })
      .catch((e) => console.warn('[useChatSession] history load failed:', e));
  }, [sessionId]);

  // ── Load tasks, knowledge, tool settings on mount ───────────────────────────
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
      const [settings, builtins, mcp] = await Promise.all([
        getSessionToolSettings(sessionId),
        listBuiltinTools(),
        listDiscoveredMcpTools(sessionId),
      ]);
      setToolSettings({ ...settings, builtins, mcp });
    } catch { /* ignore */ }
  }, [sessionId]);

  useEffect(() => {
    // eslint-disable-next-line
    void refreshTasks();
    void refreshKnowledge();
    void refreshToolSettings();
  }, [refreshTasks, refreshKnowledge, refreshToolSettings]);

  // ── Send message ────────────────────────────────────────────────────────────
  const triggerSend = useCallback(async (text: string, taskId: string | null) => {
    isProcessingRef.current = true;
    setIsProcessing(true);
    setStatus({ text: 'Thinking…', kind: 'busy' });
    const userMsg: AgentMessage = {
      id: nanoid(), role: 'user', rawMarkdown: text,
      blocks: [{ kind: 'text', id: nanoid(), content: text }],
      isStreaming: false, timestamp: Date.now(),
    };
    setMessages((prev) => [...prev, userMsg]);
    try {
      await sendMessageStreaming({ session_id: sessionId, message: text, task_id: taskId ?? null });
    } catch (err) {
      setStatus({ text: `Error: ${String(err)}`, kind: 'error' });
      setIsProcessing(false);
      isProcessingRef.current = false;
    }
  }, [sessionId]);

  useEffect(() => {
    triggerSendRef.current = triggerSend;
  }, [triggerSend]);

  const sendMessage = useCallback(async (text: string, taskId?: string | null) => {
    if (!text.trim()) return;
    if (isProcessingRef.current) {
      messageQueueRef.current.push({ kind: 'text', text, taskId: taskId ?? null });
      return;
    }
    await triggerSend(text, taskId ?? null);
  }, [triggerSend]);

  // ── Cancel streaming ────────────────────────────────────────────────────────
  const cancelStream = useCallback(async () => {
    try {
      await cancelStreaming(sessionId);
    } catch (e) {
      // Cancel command failed (e.g. no active stream or session not yet registered).
      // Force-finalize so the UI doesn't remain stuck at isProcessing=true.
      console.warn('[cancelStream] cancel command failed, forcing finalize:', e);
      finalizeStream(null);
    }
  }, [sessionId, finalizeStream]);

  // ── Tool confirmation ────────────────────────────────────────────────────────
  const resolveConfirmation = useCallback(async (decision: ToolConfirmationDecision) => {
    const conf = pendingConfirmationRef.current;
    if (!conf) return;
    try { await resolveToolConfirmationDecision(conf.confirmation_id, decision); } catch { /* ignore */ }
    pendingConfirmationRef.current = null;
    setPendingConfirmation(null);
    const next = confirmationQueueRef.current.shift();
    if (next) { pendingConfirmationRef.current = next; setPendingConfirmation(next); }
  }, []);

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
    messages, streamingMessage, isProcessing, isListening, status,
    pendingConfirmation, tasks, knowledgeItems, toolSettings,
    userScrolledUp, setUserScrolledUp,
    sendMessage, cancelStream, resolveConfirmation,
    toggleVoice, enhanceText, refreshTasks, refreshKnowledge, refreshToolSettings,
  };
}

