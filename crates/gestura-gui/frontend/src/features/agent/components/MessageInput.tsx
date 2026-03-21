/**
 * MessageInput — the chat input bar matching agent.html design.
 *
 * Uses a glassmorphism pill (input-container) with icon-microphone, icon-sparkles,
 * and icon-send buttons.
 */
import React, { useCallback, useRef, useState, KeyboardEvent } from 'react';
import type { StatusState } from '../types';

export interface MessageInputProps {
  isProcessing: boolean;
  isStopping?: boolean;
  isListening: boolean;
  status: StatusState;
  onSend: (text: string) => void;
  onCancel: () => void;
  onVoiceToggle: () => void;
  onEnhance: (text: string) => Promise<string>;
  /** Current view mode — drives the explorer/message icon swap. */
  viewMode?: 'message-only' | 'editor';
}

export interface QuickAccessBarProps {
  /** Called when the explorer/message toggle icon is clicked. */
  onToggleEditor?: () => void;
  /** Called when the Tasks quick-access button is clicked. */
  onOpenTasks?: () => void;
  /** Called when the Shell Manager quick-access button is clicked. */
  onToggleShellManager?: () => void;
  /** Whether the Shell Manager is currently visible. */
  shellManagerOpen?: boolean;
  /** Called when the Terminal quick-access button is clicked. */
  onOpenTerminal?: () => void;
  /** Current view mode — drives the explorer/message icon swap. */
  viewMode?: 'message-only' | 'editor';
  /** Layout mode for rendering within the chat panel vs. the shared window-bottom dock. */
  dockMode?: 'panel' | 'window';
}

export const MessageInput: React.FC<MessageInputProps> = ({
  isProcessing,
  isStopping = false,
  isListening,
  // status is passed through for future use (session panels, inline status)
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  status: _status,
  onSend,
  onCancel,
  onVoiceToggle,
  onEnhance,
  viewMode,
}) => {
  const [text, setText] = useState('');
  const [enhancing, setEnhancing] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Auto-resize textarea
  const autoResize = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, []);

  const handleChange = useCallback((e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setText(e.target.value);
    autoResize();
  }, [autoResize]);

  const handleKeyDown = useCallback((e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, isProcessing]);

  const handleSend = useCallback(() => {
    const trimmed = text.trim();
    if (!trimmed) return;
    onSend(trimmed);
    setText('');
    if (textareaRef.current) {
      textareaRef.current.style.height = 'auto';
    }
  }, [text, onSend]);

  const handleEnhance = useCallback(async () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    setEnhancing(true);
    try {
      const enhanced = await onEnhance(trimmed);
      setText(enhanced);
      setTimeout(autoResize, 0);
    } catch (err) {
      console.warn('[MessageInput] enhance failed:', err);
    } finally {
      setEnhancing(false);
    }
  }, [text, onEnhance, autoResize]);

  // In editor mode, show a message icon to return to message-only;
  // in message-only mode, show a folder icon to open editor.
  const isEditor = viewMode === 'editor';

  return (
    <div className={`input-area${isEditor ? ' input-area--compact' : ''}`}>
      <div className="input-container">
        {isProcessing ? (
          <button
            type="button"
            className="btn-icon btn-cancel"
            title={isStopping ? 'Stopping…' : 'Stop response'}
            onClick={onCancel}
            disabled={isStopping}
          >
            <span className="icon-close"></span>
          </button>
        ) : (
          <button
            type="button"
            className={`btn-icon btn-voice${isListening ? ' active' : ''}`}
            title={isListening ? 'Stop listening' : 'Voice Input'}
            onClick={onVoiceToggle}
          >
            <span className="icon-microphone"></span>
          </button>
        )}
        <textarea
          ref={textareaRef}
          className="message-input"
          placeholder="Type a message..."
          rows={1}
          value={text}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          disabled={isListening}
          aria-label="Message input"
          autoComplete="off"
        />
        {!isProcessing && (
          <>
            <button
              type="button"
              className={`btn-icon${enhancing ? ' enhancing' : ''}`}
              title="Enhance prompt (Cmd/Ctrl+K)"
              disabled={!text.trim() || enhancing}
              onClick={handleEnhance}
            >
              <span className="icon-sparkles"></span>
            </button>
            <button
              type="button"
              className="btn-icon"
              title="Send message"
              disabled={!text.trim()}
              onClick={handleSend}
            >
              <span className="icon-send"></span>
            </button>
          </>
        )}
      </div>
    </div>
  );
};

export const QuickAccessBar: React.FC<QuickAccessBarProps> = ({
  onToggleEditor,
  onOpenTasks,
  onToggleShellManager,
  shellManagerOpen = false,
  onOpenTerminal,
  viewMode,
  dockMode = 'panel',
}) => {
  const isEditor = viewMode === 'editor';
  const dockClassName = [
    'quick-access-dock',
    dockMode === 'window' ? 'quick-access-dock--window' : '',
    isEditor ? 'quick-access-dock--editor' : '',
  ].filter(Boolean).join(' ');

  return (
    <div className={dockClassName}>
      <div className="quick-access-bar">
        {onToggleEditor && (
          <button
            type="button"
            className="btn-icon"
            title={isEditor ? 'Messages (Cmd/Ctrl+E)' : 'Explorer (Cmd/Ctrl+E)'}
            onClick={onToggleEditor}
          >
            <span className={isEditor ? 'icon-message' : 'icon-folder'}></span>
          </button>
        )}
        <button type="button" className="btn-icon" title="Tasks (Cmd/Ctrl+T)"
          onClick={onOpenTasks}>
          <span className="icon-checklist"></span>
        </button>
        <button
          type="button"
          className={`btn-icon${shellManagerOpen ? ' active' : ''}`}
          title="Shell Manager (Cmd/Ctrl+`)"
          onClick={onToggleShellManager}
        >
          <span className="icon-terminal" aria-hidden="true"></span>
        </button>
        <button type="button" className="btn-icon" title="Open Session in Shell"
          onClick={onOpenTerminal}>
          <span className="icon-terminal-square" aria-hidden="true"></span>
        </button>
      </div>
    </div>
  );
};

export default MessageInput;

