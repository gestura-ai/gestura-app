import React, { useState, useRef, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { convertFileSrc } from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';

/**
 * Extract the current chat `session_id` from the window querystring.
 *
 * The primary Gestura chat surface (`frontend/public/chat.html`) uses `?session_id=...`.
 * Keeping this in sync allows the (legacy/secondary) React chat panel to route
 * streaming events and cancellations to the correct session when opened as a
 * session-scoped window.
 */
function getSessionIdFromUrl(): string | null {
  try {
    return new URLSearchParams(window.location.search).get('session_id');
  } catch {
    return null;
  }
}

/**
 * Best-effort check for a non-null plain object.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Unwrap payloads emitted by the Rust backend.
 *
 * The backend may:
 * - Insert `session_id` into an object payload (no wrapping), OR
 * - Wrap scalar payloads into `{ session_id, value }`.
 */
function unpackSessionTaggedPayload(raw: unknown): { incomingSessionId: string | null; value: unknown } {
  if (!isRecord(raw)) {
    return { incomingSessionId: null, value: raw };
  }

  const sessionId = typeof raw.session_id === 'string'
    ? raw.session_id
    : typeof raw.sessionId === 'string'
      ? raw.sessionId
      : null;

  if (Object.prototype.hasOwnProperty.call(raw, 'value')) {
    return { incomingSessionId: sessionId, value: (raw as { value: unknown }).value };
  }

  return { incomingSessionId: sessionId, value: raw };
}

/**
 * Decide whether a session-tagged event should be accepted.
 *
 * If `activeSessionId` is set, we require the event to carry a matching session id.
 */
function shouldAcceptSessionEvent(activeSessionId: string | null, incomingSessionId: string | null): boolean {
  if (!activeSessionId) return true;
  return !!incomingSessionId && incomingSessionId === activeSessionId;
}

/**
 * Parse the subset of token usage payloads that this panel understands.
 */
function parseTokenUsage(payload: unknown): TokenUsage | null {
  if (!isRecord(payload)) return null;
  const input = payload.input_tokens;
  const output = payload.output_tokens;
  if (typeof input !== 'number' || typeof output !== 'number') return null;

  const estimatedCost = payload.estimated_cost_usd;
  return {
    input_tokens: input,
    output_tokens: output,
    estimated_cost_usd: typeof estimatedCost === 'number' ? estimatedCost : undefined,
  };
}

interface Message {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  thinking?: string;
  isStreaming?: boolean;
  timestamp: Date;
  toolCalls?: string[];
}

interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  estimated_cost_usd?: number;
}

interface ModelInfo {
  id: string;
  name: string;
  provider: string;
}

interface ChatPanelProps {
  onTokenUsage?: (usage: TokenUsage) => void;
}

const ChatPanel: React.FC<ChatPanelProps> = ({ onTokenUsage }) => {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [showThinking, setShowThinking] = useState(true);
  const [isListening, setIsListening] = useState(false);
  const [selectedModel, setSelectedModel] = useState('');
  const [workspaceDir, setWorkspaceDir] = useState<string | null>(null);
  // Message queue for sending multiple messages during streaming
  const [messageQueue, setMessageQueue] = useState<string[]>([]);
  const [availableModels] = useState<ModelInfo[]>([
    { id: 'anthropic', name: 'Claude (Anthropic)', provider: 'anthropic' },
    { id: 'openai', name: 'GPT-4 (OpenAI)', provider: 'openai' },
    { id: 'ollama', name: 'Ollama (Local)', provider: 'ollama' },
    { id: 'grok', name: 'Grok', provider: 'grok' },
  ]);
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const [sessionTokens, setSessionTokens] = useState<TokenUsage>({ input_tokens: 0, output_tokens: 0 });
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const currentToolCallRef = useRef<{ id: string; name: string; args: string } | null>(null);
  // If the window was opened as `...?session_id=...`, treat it as session-scoped and never
  // allow cross-session event rendering.
  const urlSessionIdRef = useRef<string | null>(getSessionIdFromUrl());
  const sessionIdRef = useRef<string | null>(urlSessionIdRef.current);

  // Fetch workspace directory on mount
  useEffect(() => {
    const fetchWorkspace = async () => {
      try {
        const workspace = await invoke<string | null>('get_session_workspace');
        setWorkspaceDir(workspace);
      } catch (err) {
        console.error('Failed to get workspace:', err);
      }
    };
    fetchWorkspace();
  }, []);

  // Pick a new workspace directory
  const pickWorkspace = useCallback(async () => {
    try {
      const result = await invoke<string | null>('pick_workspace_directory');
      if (result) {
        setWorkspaceDir(result);
      }
    } catch (err) {
      console.error('Failed to pick workspace:', err);
    }
  }, []);

  // Auto-scroll to bottom
  const scrollToBottom = () => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  };

  useEffect(() => {
    scrollToBottom();
  }, [messages]);

  // Copy message to clipboard
  const copyToClipboard = useCallback(async (content: string, messageId: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedMessageId(messageId);
      setTimeout(() => setCopiedMessageId(null), 2000);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  }, []);

  // Toggle voice listening
  const toggleVoiceListening = useCallback(async () => {
    try {
      if (isListening) {
        await invoke('stop_voice_listening');
        setIsListening(false);
      } else {
        await invoke('start_voice_listening');
        setIsListening(true);
      }
    } catch (error) {
      console.error('Voice toggle error:', error);
      setIsListening(false);
    }
  }, [isListening]);

  // Handle model change
  const handleModelChange = useCallback(async (modelId: string) => {
    setSelectedModel(modelId);
    try {
      await invoke('set_llm_provider', { provider: modelId });
    } catch (error) {
      console.error('Failed to set model:', error);
    }
  }, []);

  // Save message to knowledge base
  const saveToKnowledge = useCallback(async (content: string) => {
    try {
      await invoke('add_knowledge_entry', {
        content,
        category: 'chat_response',
        tags: ['saved', 'chat'],
      });
      // Show brief success feedback
      alert('Saved to knowledge base!');
    } catch (error) {
      console.error('Failed to save to knowledge:', error);
      alert('Failed to save to knowledge base');
    }
  }, []);

  // Regenerate last response
  const regenerateResponse = useCallback(async () => {
    if (messages.length < 2) return;

    // Find the last user message (iterate backwards for compatibility)
    let lastUserMsgIndex = -1;
    for (let i = messages.length - 1; i >= 0; i--) {
      if (messages[i].role === 'user') {
        lastUserMsgIndex = i;
        break;
      }
    }
    if (lastUserMsgIndex === -1) return;

    const lastUserMsg = messages[lastUserMsgIndex];

    // Remove messages after the last user message
    setMessages(prev => prev.slice(0, lastUserMsgIndex + 1));

    // Re-send the message
    setInput(lastUserMsg.content);
    // Trigger send after state update
    setTimeout(() => {
      const sendBtn = document.querySelector('.chat-input-actions .btn:not(.btn-icon)') as HTMLButtonElement;
      if (sendBtn) sendBtn.click();
    }, 100);
  }, [messages]);

  // Load current model on mount
  useEffect(() => {
    const loadCurrentModel = async () => {
      try {
        const config = await invoke<{ llm: { primary: string } }>('get_config');
        if (config?.llm?.primary) {
          setSelectedModel(config.llm.primary);
        }
      } catch (error) {
        console.error('Failed to load config:', error);
      }
    };
    loadCurrentModel();
  }, []);

  // Set up event listeners for streaming and voice
  useEffect(() => {
    let unlistenChunk: UnlistenFn;
    let unlistenThinking: UnlistenFn;
    let unlistenDone: UnlistenFn;
    let unlistenToolStart: UnlistenFn;
    let unlistenToolArgs: UnlistenFn;
    let unlistenToolEnd: UnlistenFn;
    let unlistenTokenUsage: UnlistenFn;
    let unlistenError: UnlistenFn;
    let unlistenCancelled: UnlistenFn;
    let unlistenVoiceMessage: UnlistenFn;
    let unlistenListeningState: UnlistenFn;
    let unlistenVoiceSession: UnlistenFn;

    const setupListeners = async () => {
      /**
       * No-op unlisten function.
       *
       * Used when we intentionally refuse to attach an event listener (fail-closed)
       * to avoid cross-window leakage in multi-window Tauri setups.
       */
      const noopUnlisten: UnlistenFn = () => { };

      /**
       * Listen for events scoped to this webview window.
       *
       * In Tauri v2, the global `@tauri-apps/api/event.listen` behaves like `listen_any`
       * unless an explicit target is set. That can cause cross-window event leakage when
       * multiple chat windows exist.
       *
       * We therefore prefer `getCurrentWebviewWindow().listen(...)`.
       */
      const listenScoped = async <T,>(eventName: string, handler: (event: { payload: T }) => void) => {
        try {
          const webview = getCurrentWebviewWindow();
          if (webview && typeof (webview as unknown as { listen?: unknown }).listen === 'function') {
            return await (webview as unknown as { listen: (name: string, cb: (event: { payload: T }) => void) => Promise<UnlistenFn> }).listen(eventName, handler);
          }
        } catch {
          // Fall back below.
        }

        // Fallback: if this window is session-scoped, derive the expected label and
        // attach an explicit target to avoid cross-window delivery.
        const urlSessionId = urlSessionIdRef.current;
        const expectedLabel = urlSessionId ? `chat-${urlSessionId}` : null;
        if (expectedLabel) {
          return await listen<T>(eventName, handler, {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            target: { kind: 'WebviewWindow', label: expectedLabel } as any,
          });
        }

        // Fail-closed: do not attach an unscoped listener.
        // In Tauri v2, an unscoped listener can behave like listen_any and cause
        // cross-window event leakage.
        console.error('[ChatPanel] Refusing to attach unscoped event listener (no webview.listen and no window label target)', {
          eventName,
        });
        return noopUnlisten;
      };

      const getActiveSessionId = () => urlSessionIdRef.current ?? sessionIdRef.current;
      const maybeAdoptSessionId = (incoming: string | null) => {
        // Only adopt in non-session-scoped windows.
        if (urlSessionIdRef.current) return;
        if (!sessionIdRef.current && incoming) {
          sessionIdRef.current = incoming;
        }
      };

      unlistenChunk = await listenScoped<unknown>('chat-stream-chunk', (event) => {
        const { incomingSessionId, value } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        if (typeof value !== 'string') return;
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            updated[lastIdx] = {
              ...updated[lastIdx],
              content: updated[lastIdx].content + value,
            };
          } else {
            updated.push({
              id: Date.now().toString(),
              role: 'assistant',
              content: value,
              isStreaming: true,
              timestamp: new Date(),
            });
          }
          return updated;
        });
      });

      unlistenThinking = await listenScoped<unknown>('chat-stream-thinking', (event) => {
        const { incomingSessionId, value } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        if (typeof value !== 'string') return;
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            updated[lastIdx] = {
              ...updated[lastIdx],
              thinking: (updated[lastIdx].thinking || '') + value,
            };
          } else {
            updated.push({
              id: Date.now().toString(),
              role: 'assistant',
              content: '',
              thinking: value,
              isStreaming: true,
              timestamp: new Date(),
            });
          }
          return updated;
        });
      });

      // Token usage is emitted separately from `chat-stream-done`
      unlistenTokenUsage = await listenScoped<unknown>('chat-token-usage', (event) => {
        const { incomingSessionId, value } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        const usage = parseTokenUsage(value);
        if (!usage) return;
        setSessionTokens(prev => ({
          input_tokens: prev.input_tokens + usage.input_tokens,
          output_tokens: prev.output_tokens + usage.output_tokens,
          estimated_cost_usd: (prev.estimated_cost_usd || 0) + (usage.estimated_cost_usd || 0),
        }));
        onTokenUsage?.(usage);
      });

      unlistenDone = await listenScoped<unknown>('chat-stream-done', (event) => {
        const { incomingSessionId } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            updated[lastIdx] = { ...updated[lastIdx], isStreaming: false };
          }
          return updated;
        });
        setIsLoading(false);
      });

      unlistenToolStart = await listenScoped<unknown>('chat-stream-tool-start', (event) => {
        const { incomingSessionId, value } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        if (!isRecord(value)) return;
        const id = typeof value.id === 'string' ? value.id : '';
        const name = typeof value.name === 'string' ? value.name : 'tool';
        currentToolCallRef.current = { id, name, args: '' };
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          const addToAssistant = (idx: number) => {
            const toolCalls = updated[idx].toolCalls || [];
            updated[idx] = {
              ...updated[idx],
              toolCalls: [...toolCalls, name],
            };
          };

          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            addToAssistant(lastIdx);
          } else {
            updated.push({
              id: Date.now().toString(),
              role: 'assistant',
              content: '',
              isStreaming: true,
              timestamp: new Date(),
              toolCalls: [name],
            });
          }
          return updated;
        });
      });

      unlistenToolArgs = await listenScoped<unknown>('chat-stream-tool-args', (event) => {
        const { incomingSessionId, value } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        if (typeof value !== 'string') return;
        // Args can be large; keep for debugging/future UI but don't render in badges.
        if (currentToolCallRef.current) {
          currentToolCallRef.current.args += value;
        }
      });

      unlistenToolEnd = await listenScoped<unknown>('chat-stream-tool-end', (event) => {
        const { incomingSessionId } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        currentToolCallRef.current = null;
      });

      unlistenError = await listenScoped<unknown>('chat-stream-error', (event) => {
        const { incomingSessionId, value } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        const message = typeof value === 'string' ? value : JSON.stringify(value);
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            updated[lastIdx] = {
              ...updated[lastIdx],
              content: `${updated[lastIdx].content}\n\nError: ${message}`.trim(),
              isStreaming: false,
            };
          } else {
            updated.push({
              id: Date.now().toString(),
              role: 'assistant',
              content: `Error: ${message}`,
              isStreaming: false,
              timestamp: new Date(),
            });
          }
          return updated;
        });
        setIsLoading(false);
      });

      unlistenCancelled = await listenScoped<unknown>('chat-stream-cancelled', (event) => {
        const { incomingSessionId } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            updated[lastIdx] = { ...updated[lastIdx], isStreaming: false };
          }
          return updated;
        });
        setIsLoading(false);
      });

      // Listen for voice transcription messages (both user and assistant)
      unlistenVoiceMessage = await listenScoped<unknown>('chat-message', (event) => {
        const { incomingSessionId, value } = unpackSessionTaggedPayload(event.payload);
        if (!shouldAcceptSessionEvent(getActiveSessionId(), incomingSessionId)) return;
        maybeAdoptSessionId(incomingSessionId);
        if (!isRecord(value)) return;
        const message = typeof value.message === 'string' ? value.message : '';
        const type = typeof value.type === 'string' ? value.type : undefined;
        const session_id = typeof value.session_id === 'string' ? value.session_id : incomingSessionId;
        if (!message) return;

        // Track the most recent session_id so typed messages and cancellations can
        // be scoped correctly even if this panel didn't start the session.
        if (session_id && !urlSessionIdRef.current) {
          sessionIdRef.current = session_id;
        }

        if (type === 'assistant') {
          // AI response from voice processing - add directly without calling LLM again
          const assistantMessage: Message = {
            id: Date.now().toString(),
            role: 'assistant',
            content: message,
            isStreaming: false,
            timestamp: new Date(),
          };
          setMessages((prev) => [...prev, assistantMessage]);
          setIsLoading(false);
        } else {
          // User message from voice transcription (type === 'user' or undefined)
          const userMessage: Message = {
            id: Date.now().toString(),
            role: 'user',
            content: `🎤 ${message}`,
            timestamp: new Date(),
          };

          // Add user message and empty assistant message for streaming
          setMessages((prev) => [
            ...prev,
            userMessage,
            {
              id: (Date.now() + 1).toString(),
              role: 'assistant',
              content: '',
              isStreaming: true,
              timestamp: new Date(),
            },
          ]);
          setIsLoading(true);

          // Process the voice message through LLM
          invoke('process_chat_message_streaming', { message, sessionId: session_id, source: 'voice' }).catch((error) => {
            console.error('Voice chat error:', error);
            setMessages((prev) => {
              const updated = [...prev];
              const lastIdx = updated.length - 1;
              if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
                updated[lastIdx] = {
                  ...updated[lastIdx],
                  content: `Error: ${error}`,
                  isStreaming: false,
                };
              }
              return updated;
            });
            setIsLoading(false);
          });
        }
      });

      // Listen for listening state changes
      unlistenListeningState = await listenScoped<{ is_listening: boolean }>('listening-state-changed', (event) => {
        setIsListening(event.payload.is_listening);
      });

      // Listen for voice session start
      unlistenVoiceSession = await listenScoped<{ session_id: string }>('voice-session-started', (event) => {
        // Voice session started - UI can show indicator
        if (event?.payload?.session_id && !urlSessionIdRef.current) {
          sessionIdRef.current = event.payload.session_id;
        }
        setIsListening(true);
      });
    };

    setupListeners();

    return () => {
      unlistenChunk?.();
      unlistenThinking?.();
      unlistenDone?.();
      unlistenToolStart?.();
      unlistenToolArgs?.();
      unlistenToolEnd?.();
      unlistenTokenUsage?.();
      unlistenError?.();
      unlistenCancelled?.();
      unlistenVoiceMessage?.();
      unlistenListeningState?.();
      unlistenVoiceSession?.();
    };
  }, [onTokenUsage]);

  // Process a message (either from direct send or from queue)
  const processMessage = useCallback(async (messageContent: string) => {
    const userMessage: Message = {
      id: Date.now().toString(),
      role: 'user',
      content: messageContent,
      timestamp: new Date(),
    };

    // Add user message and empty assistant message for streaming
    setMessages((prev) => [
      ...prev,
      userMessage,
      {
        id: (Date.now() + 1).toString(),
        role: 'assistant',
        content: '',
        isStreaming: true,
        timestamp: new Date(),
      },
    ]);
    setIsLoading(true);

    try {
      const session_id = sessionIdRef.current;
      await invoke(
        'process_chat_message_streaming',
        session_id ? { message: messageContent, sessionId: session_id } : { message: messageContent }
      );
    } catch (error) {
      console.error('Chat error:', error);
      setMessages((prev) => {
        const updated = [...prev];
        const lastIdx = updated.length - 1;
        if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
          updated[lastIdx] = {
            ...updated[lastIdx],
            content: `Error: ${error}`,
            isStreaming: false,
          };
        }
        return updated;
      });
      setIsLoading(false);
    }
  }, []);

  const sendMessage = async () => {
    if (!input.trim()) return;

    const trimmedInput = input.trim();
    setInput('');

    // If already loading, queue the message for later
    if (isLoading) {
      setMessageQueue((prev) => [...prev, trimmedInput]);
      // Show queued message indicator in messages
      setMessages((prev) => [
        ...prev,
        {
          id: Date.now().toString(),
          role: 'user',
          content: `⏳ ${trimmedInput}`,
          timestamp: new Date(),
        },
      ]);
      return;
    }

    await processMessage(trimmedInput);
  };

  // Process queued messages when streaming completes
  useEffect(() => {
    if (!isLoading && messageQueue.length > 0) {
      const nextMessage = messageQueue[0];
      setMessageQueue((prev) => prev.slice(1));
      // Update the queued message to remove the ⏳ indicator
      setMessages((prev) => {
        const updated = [...prev];
        const queuedIdx = updated.findIndex(
          (msg) => msg.role === 'user' && msg.content === `⏳ ${nextMessage}`
        );
        if (queuedIdx !== -1) {
          updated[queuedIdx] = {
            ...updated[queuedIdx],
            content: nextMessage,
          };
        }
        return updated;
      });
      processMessage(nextMessage);
    }
  }, [isLoading, messageQueue, processMessage]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  const cancelStream = async () => {
    try {
      const session_id = sessionIdRef.current;
      await invoke('cancel_chat_streaming', session_id ? { sessionId: session_id } : {});
    } catch (error) {
      console.error('Failed to cancel stream:', error);
    }
  };

  const clearMessages = () => {
    setMessages([]);
  };

  const renderMessage = (msg: Message) => {
    const isUser = msg.role === 'user';
    const isSystem = msg.role === 'system';
    const isAssistant = msg.role === 'assistant';
    const isCopied = copiedMessageId === msg.id;

    return (
      <div key={msg.id} className={`chat-message ${msg.role}`}>
        <div className="message-header">
          <span className="message-role">
            {isUser ? '▶ You' : isSystem ? '⚙ System' : '◆ AI'}
          </span>
          <span className="message-time">
            {msg.timestamp.toLocaleTimeString()}
          </span>
          {/* Message action buttons */}
          {isAssistant && !msg.isStreaming && msg.content && (
            <div className="message-actions">
              <button
                className="btn-icon"
                onClick={() => copyToClipboard(msg.content, msg.id)}
                title={isCopied ? 'Copied!' : 'Copy to clipboard'}
              >
                {isCopied ? '✓' : '📋'}
              </button>
              <button
                className="btn-icon"
                onClick={() => saveToKnowledge(msg.content)}
                title="Save to knowledge base"
              >
                📚
              </button>
              <button
                className="btn-icon"
                onClick={regenerateResponse}
                title="Regenerate response"
              >
                🔄
              </button>
            </div>
          )}
        </div>

        {/* Tool calls indicator */}
        {msg.toolCalls && msg.toolCalls.length > 0 && (
          <div className="message-tools">
            <span className="tools-label">🔧 Tools used:</span>
            {msg.toolCalls.map((tool, idx) => (
              <span key={idx} className="tool-badge">{tool}</span>
            ))}
          </div>
        )}

        {/* Thinking section */}
        {showThinking && msg.thinking && (
          <div className="message-thinking">
            <div className="thinking-header">💭 Thinking...</div>
            <div className="thinking-content">{msg.thinking}</div>
          </div>
        )}

        {/* Main content */}
        <div className={`message-content ${msg.isStreaming ? 'streaming' : ''}`}>
          {renderFormattedContent(msg.content)}
          {msg.isStreaming && <span className="cursor">▌</span>}
        </div>
      </div>
    );
  };

  /**
   * Try to parse content as JSON and check if it's a screenshot result.
   */
  const tryParseScreenshotResult = (content: string): { path: string; width?: number; height?: number; file_size_bytes?: number } | null => {
    try {
      const json = JSON.parse(content);
      if (json && typeof json.path === 'string' && json.path.match(/\.(png|jpg|jpeg|gif|bmp|webp)$/i)) {
        return json;
      }
    } catch {
      // Not JSON or not a screenshot result
    }
    return null;
  };

  /**
   * Format file size in human-readable format.
   */
  const formatBytes = (bytes: number): string => {
    if (bytes === 0) return '0 Bytes';
    const k = 1024;
    const sizes = ['Bytes', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return Math.round((bytes / Math.pow(k, i)) * 100) / 100 + ' ' + sizes[i];
  };

  /**
   * Render markdown content with proper formatting.
   *
   * Uses ReactMarkdown for full markdown support including:
   * - Headers, bold, italic, strikethrough
   * - Lists (ordered and unordered)
   * - Tables
   * - Links
   * - Code blocks with syntax highlighting
   * - Inline code
   * - Screenshot images (auto-detected from JSON results)
   *
   * The raw markdown is preserved for copy operations.
   */
  const renderFormattedContent = (content: string) => {
    // Check if content is a screenshot result
    const screenshotResult = tryParseScreenshotResult(content);
    if (screenshotResult) {
      const imageSrc = convertFileSrc(screenshotResult.path);
      return (
        <div className="screenshot-result">
          <img src={imageSrc} alt="Screenshot" className="screenshot-image" />
          <div className="screenshot-info">
            {screenshotResult.width && screenshotResult.height && (
              <span>{screenshotResult.width}×{screenshotResult.height}</span>
            )}
            {screenshotResult.file_size_bytes && (
              <span> • {formatBytes(screenshotResult.file_size_bytes)}</span>
            )}
            <span> • {screenshotResult.path}</span>
          </div>
        </div>
      );
    }

    return (
      <ReactMarkdown
        components={{
          // Custom code block rendering with language header
          code({ className, children, ...props }) {
            const match = /language-(\w+)/.exec(className || '');
            const isInline = !match && !className;
            if (isInline) {
              return (
                <code className="inline-code" {...props}>
                  {children}
                </code>
              );
            }
            const lang = match ? match[1] : 'code';
            return (
              <pre className="code-block">
                <div className="code-header">{lang}</div>
                <code className={className} {...props}>
                  {children}
                </code>
              </pre>
            );
          },
          // Style tables properly
          table({ children }) {
            return <table className="markdown-table">{children}</table>;
          },
          // Style links to open in new tab
          a({ href, children }) {
            return (
              <a href={href} target="_blank" rel="noopener noreferrer">
                {children}
              </a>
            );
          },
        }}
      >
        {content}
      </ReactMarkdown>
    );
  };

  return (
    <div className="chat-panel">
      <div className="chat-header">
        <div className="chat-header-left">
          <h2>Chat</h2>
          {/* Model selector */}
          <select
            className="model-selector"
            value={selectedModel}
            onChange={(e) => handleModelChange(e.target.value)}
            title="Select AI model"
          >
            {availableModels.map((model) => (
              <option key={model.id} value={model.id}>
                {model.name}
              </option>
            ))}
          </select>
        </div>
        <div className="chat-actions">
          {/* Workspace directory display */}
          <button
            className="workspace-display"
            onClick={pickWorkspace}
            title={workspaceDir ? `Workspace: ${workspaceDir}\nClick to change` : 'Click to set workspace directory'}
          >
            📁 {workspaceDir ? workspaceDir.split('/').pop() || workspaceDir : 'No workspace'}
          </button>
          {/* Token usage display */}
          {(sessionTokens.input_tokens > 0 || sessionTokens.output_tokens > 0) && (
            <span className="token-display" title="Session token usage">
              📊 {sessionTokens.input_tokens + sessionTokens.output_tokens} tokens
              {/* Hide cost for local providers (Ollama) and when cost is zero/unavailable */}
              {sessionTokens.estimated_cost_usd !== undefined &&
                sessionTokens.estimated_cost_usd > 0 &&
                !['ollama', 'local'].includes(selectedModel) && (
                  <span className="cost-display"> (${sessionTokens.estimated_cost_usd.toFixed(4)})</span>
                )}
            </span>
          )}
          <button
            className="btn btn-small btn-secondary"
            onClick={() => setShowThinking(!showThinking)}
            title={showThinking ? 'Hide thinking' : 'Show thinking'}
          >
            💭
          </button>
          <button
            className="btn btn-small btn-secondary"
            onClick={clearMessages}
            title="Clear chat"
          >
            🗑️
          </button>
        </div>
      </div>

      <div className="chat-messages">
        {messages.length === 0 ? (
          <div className="chat-empty">
            <p>Start a conversation by typing a message below or use voice input.</p>
            <p className="hint">Try asking about tools with <code>/tools</code> or capabilities with <code>/capabilities</code></p>
          </div>
        ) : (
          messages.map(renderMessage)
        )}
        <div ref={messagesEndRef} />
      </div>

      <div className="chat-input-container">
        {/* Voice listening indicator - inside container for proper positioning */}
        {isListening && (
          <div className="voice-indicator">
            <span className="pulse-dot"></span>
            <span>Listening... Click 🔴 to stop</span>
          </div>
        )}
        {/* Message queue indicator */}
        {messageQueue.length > 0 && (
          <div className="queue-indicator">
            <span>📨 {messageQueue.length} message{messageQueue.length > 1 ? 's' : ''} queued</span>
          </div>
        )}
        <div className="chat-input-row">
          <textarea
            ref={inputRef}
            className="chat-input"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={
              isListening
                ? '🎤 Listening...'
                : isLoading
                  ? 'Type to queue another message...'
                  : 'Type a message... (Shift+Enter for new line)'
            }
            disabled={isListening}
            rows={3}
          />
        </div>
        <div className="chat-input-actions">
          <button
            className={`btn btn-icon ${isListening ? 'btn-listening' : 'btn-secondary'}`}
            onClick={toggleVoiceListening}
            title={isListening ? 'Stop listening' : 'Start voice input'}
            disabled={isLoading}
          >
            {isListening ? '🔴' : '🎤'}
          </button>
          {isLoading ? (
            <>
              <button className="btn btn-secondary" onClick={sendMessage} disabled={!input.trim()}>
                Queue
              </button>
              <button className="btn btn-danger" onClick={cancelStream}>
                Cancel
              </button>
            </>
          ) : (
            <button className="btn" onClick={sendMessage} disabled={!input.trim() || isListening}>
              Send
            </button>
          )}
        </div>
      </div>
    </div>
  );
};

export default ChatPanel;

