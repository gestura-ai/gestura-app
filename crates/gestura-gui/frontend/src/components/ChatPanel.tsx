import React, { useState, useRef, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

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
      unlistenChunk = await listen<string>('chat-stream-chunk', (event) => {
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            updated[lastIdx] = {
              ...updated[lastIdx],
              content: updated[lastIdx].content + event.payload,
            };
          } else {
            updated.push({
              id: Date.now().toString(),
              role: 'assistant',
              content: event.payload,
              isStreaming: true,
              timestamp: new Date(),
            });
          }
          return updated;
        });
      });

      unlistenThinking = await listen<string>('chat-stream-thinking', (event) => {
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            updated[lastIdx] = {
              ...updated[lastIdx],
              thinking: (updated[lastIdx].thinking || '') + event.payload,
            };
          } else {
            updated.push({
              id: Date.now().toString(),
              role: 'assistant',
              content: '',
              thinking: event.payload,
              isStreaming: true,
              timestamp: new Date(),
            });
          }
          return updated;
        });
      });

      // Token usage is emitted separately from `chat-stream-done`
      unlistenTokenUsage = await listen<TokenUsage>('chat-token-usage', (event) => {
        setSessionTokens(prev => ({
          input_tokens: prev.input_tokens + (event.payload?.input_tokens || 0),
          output_tokens: prev.output_tokens + (event.payload?.output_tokens || 0),
          estimated_cost_usd: (prev.estimated_cost_usd || 0) + (event.payload?.estimated_cost_usd || 0),
        }));
        onTokenUsage?.(event.payload);
      });

      unlistenDone = await listen<null>('chat-stream-done', () => {
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

      unlistenToolStart = await listen<{ id: string; name: string }>('chat-stream-tool-start', (event) => {
        currentToolCallRef.current = { id: event.payload.id, name: event.payload.name, args: '' };
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          const addToAssistant = (idx: number) => {
            const toolCalls = updated[idx].toolCalls || [];
            updated[idx] = {
              ...updated[idx],
              toolCalls: [...toolCalls, event.payload.name],
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
              toolCalls: [event.payload.name],
            });
          }
          return updated;
        });
      });

      unlistenToolArgs = await listen<string>('chat-stream-tool-args', (event) => {
        // Args can be large; keep for debugging/future UI but don't render in badges.
        if (currentToolCallRef.current) {
          currentToolCallRef.current.args += event.payload;
        }
      });

      unlistenToolEnd = await listen<null>('chat-stream-tool-end', () => {
        currentToolCallRef.current = null;
      });

      unlistenError = await listen<string>('chat-stream-error', (event) => {
        setMessages((prev) => {
          const updated = [...prev];
          const lastIdx = updated.length - 1;
          if (lastIdx >= 0 && updated[lastIdx].role === 'assistant') {
            updated[lastIdx] = {
              ...updated[lastIdx],
              content: `${updated[lastIdx].content}\n\nError: ${event.payload}`.trim(),
              isStreaming: false,
            };
          } else {
            updated.push({
              id: Date.now().toString(),
              role: 'assistant',
              content: `Error: ${event.payload}`,
              isStreaming: false,
              timestamp: new Date(),
            });
          }
          return updated;
        });
        setIsLoading(false);
      });

      unlistenCancelled = await listen<null>('chat-stream-cancelled', () => {
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
      unlistenVoiceMessage = await listen<{ message: string; session_id: string; type?: string }>('chat-message', (event) => {
        const { message, type, session_id } = event.payload;
        if (!message) return;

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
          invoke('process_chat_message_streaming', { message, session_id, source: 'voice' }).catch((error) => {
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
      unlistenListeningState = await listen<{ is_listening: boolean }>('listening-state-changed', (event) => {
        setIsListening(event.payload.is_listening);
      });

      // Listen for voice session start
      unlistenVoiceSession = await listen<{ session_id: string }>('voice-session-started', () => {
        // Voice session started - UI can show indicator
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
      await invoke('process_chat_message_streaming', { message: messageContent });
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
      await invoke('cancel_chat_streaming');
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

  const renderFormattedContent = (content: string) => {
    // Simple markdown-like rendering for code blocks
    const parts = content.split(/(```[\s\S]*?```)/g);
    return parts.map((part, idx) => {
      if (part.startsWith('```')) {
        const lines = part.slice(3, -3).split('\n');
        const lang = lines[0] || 'code';
        const code = lines.slice(1).join('\n');
        return (
          <pre key={idx} className="code-block">
            <div className="code-header">{lang}</div>
            <code>{code}</code>
          </pre>
        );
      }
      return <span key={idx}>{part}</span>;
    });
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
              {sessionTokens.estimated_cost_usd !== undefined && sessionTokens.estimated_cost_usd > 0 && (
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

