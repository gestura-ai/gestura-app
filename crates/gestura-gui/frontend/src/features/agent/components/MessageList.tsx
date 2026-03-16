/**
 * MessageList — renders all agent/user messages including streaming blocks.
 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';

import { parseMarkdown } from '../utils/markdown';
import { ansiToHtml } from '../utils/ansi';
import {
  shellProcessStop,
  shellProcessPause,
  shellProcessResume,
  openShellForSession,
} from '../../../services/tauri/agent';
import type {
  AgentMessage,
  MsgBlock,
  ThinkingBlock,
  TextBlock,
  ToolBlock,
  ShellBlock,
  IterationMarkerBlock,
} from '../types';

// ─── Block renderers ──────────────────────────────────────────────────────────

const ThinkingBlockView: React.FC<{ block: ThinkingBlock }> = ({ block }) => {
  const [collapsed, setCollapsed] = useState(block.collapsed);

  return (
    <div className={`thinking-block${collapsed ? ' collapsed' : ''}`}>
      <button
        type="button"
        className={`thinking-header${!block.done ? ' animating' : ''}`}
        onClick={() => setCollapsed((c) => !c)}
      >
        {block.done ? 'Thought Process' : 'Thinking Process…'}
      </button>
      {!collapsed && (
        <div className="thinking-content">{block.content}</div>
      )}
    </div>
  );
};

const TextBlockView: React.FC<{ block: TextBlock }> = ({ block }) => (
  <div
    className="text-content markdown-body"
    dangerouslySetInnerHTML={{ __html: parseMarkdown(block.content) }}
  />
);

const ToolBlockView: React.FC<{ block: ToolBlock }> = ({ block }) => {
  const [collapsed, setCollapsed] = useState(block.collapsed);

  // Sync with external collapse signals (e.g. parent collapses on new text content).
  // eslint-disable-next-line
  useEffect(() => { setCollapsed(block.collapsed); }, [block.collapsed]);
  let screenshotSrc: string | null = null;
  try {
    const parsed = JSON.parse(block.result ?? '{}') as { path?: string; inline_base64?: string; inline_mime_type?: string };
    if (parsed.path && /\.(png|jpg|jpeg|gif|bmp|webp)$/i.test(parsed.path)) {
      screenshotSrc = convertFileSrc(parsed.path);
    } else if (parsed.inline_base64 && parsed.inline_mime_type) {
      screenshotSrc = `data:${parsed.inline_mime_type};base64,${parsed.inline_base64}`;
    }
  } catch { /* not a screenshot result */ }

  return (
    <div className={`tool-call ${block.status}${collapsed ? ' collapsed' : ''}`}>
      <div className="tool-call-body">
        <button
          type="button"
          className="tool-call-header"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((c) => !c)}
        >
          <span className="tool-call-label">Tool</span>
          <strong className="tool-call-name">{block.name}</strong>
          <span className="tool-call-status">
            {block.status === 'running' ? 'Running…' :
              block.status === 'executing' ? 'Executing…' :
                block.status === 'success' ? `Success${block.durationMs != null ? ` • ${block.durationMs}ms` : ''}` :
                  block.status === 'error' ? `Error${block.durationMs != null ? ` • ${block.durationMs}ms` : ''}` :
                    block.status === 'blocked' ? 'Blocked' : ''}
          </span>
          <span className="tool-call-chevron">{collapsed ? '▸' : '▾'}</span>
        </button>
        {!collapsed && (
          <div className="tool-call-details">
            {block.args && <pre className="tool-args">{block.args}</pre>}
            {block.result != null && !screenshotSrc && (
              <div className={`tool-result ${block.status}`}>{block.result}</div>
            )}
            {screenshotSrc && (
              <div className="tool-result screenshot-result">
                <img src={screenshotSrc} alt="Screenshot" className="screenshot-image" />
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
};

const ShellBlockView: React.FC<{ block: ShellBlock; sessionId: string }> = ({ block, sessionId }) => {
  const [collapsed, setCollapsed] = useState(block.collapsed);
  const outputRef = useRef<HTMLDivElement>(null);
  const isTerminal = block.state === 'Completed' || block.state === 'Failed' || block.state === 'Stopped';

  useEffect(() => {
    if (outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight;
    }
  }, [block.lines.length]);

  return (
    <div className={`shell-console${collapsed ? ' collapsed' : ''}`} data-process-id={block.processId}>
      <div className="shell-console-header">
        <span className="shell-icon">⬡</span>
        <span className="shell-cmd" title={block.command}>{block.command || 'shell'}</span>
        {block.cwd && <span className="shell-cwd" title={block.cwd}>{block.cwd}</span>}
        <div className="shell-controls">
          {!isTerminal && (
            <>
              <button title={block.state === 'Paused' ? 'Resume' : 'Pause'} onClick={() => {
                if (block.state === 'Paused') shellProcessResume(block.processId).catch(console.error);
                else shellProcessPause(block.processId).catch(console.error);
              }}>{block.state === 'Paused' ? '▶' : '⏸'}</button>
              <button title="Stop" onClick={() => shellProcessStop(block.processId).catch(console.error)}>⏹</button>
            </>
          )}
          <button title="Copy command" onClick={() => navigator.clipboard.writeText(block.command).catch(console.error)}>⧉</button>
          <button title="Open in terminal" onClick={() => {
            openShellForSession(sessionId).catch(console.error);
          }}>↗</button>
          <button title={collapsed ? 'Expand' : 'Collapse'} onClick={() => setCollapsed((c) => !c)}>
            {collapsed ? '▸' : '▾'}
          </button>
        </div>
      </div>
      {!collapsed && (
        <div className="shell-console-output" ref={outputRef}>
          {block.lines.map((line, idx) => (
            <div
              key={idx}
              className={`shell-line${line.stream === 'Stderr' ? ' shell-stderr' : ''}`}
              dangerouslySetInnerHTML={{ __html: ansiToHtml(line.data) }}
            />
          ))}
        </div>
      )}
      <div className="shell-console-footer">
        <span className={`shell-status-dot ${block.state === 'Paused' ? 'paused' : isTerminal ? (block.exitCode === 0 ? 'success' : 'error') : 'running'}`} />
        <span className="shell-status-label">
          {block.state === 'Paused' ? 'Paused' :
            block.state === 'Completed' ? (block.exitCode === 0 ? 'Completed' : `Exit ${block.exitCode}`) :
              block.state === 'Failed' ? 'Failed' :
                block.state === 'Stopped' ? 'Stopped' : 'Running…'}
        </span>
        {block.durationMs != null && (
          <span className="shell-duration">{(block.durationMs / 1000).toFixed(1)}s</span>
        )}
      </div>
    </div>
  );
};

const IterationMarkerView: React.FC<{ block: IterationMarkerBlock }> = ({ block }) => (
  <div className="agent-iteration-marker">
    <div className="iteration-header">
      <span className="iteration-label">{block.label}</span>
    </div>
    {block.detail && (
      <div className="iteration-detail">{block.detail}</div>
    )}
  </div>
);

// ─── Single message ───────────────────────────────────────────────────────────

const MessageView: React.FC<{ message: AgentMessage; sessionId: string }> = ({ message, sessionId }) => {
  const copyText = useCallback(() => {
    navigator.clipboard.writeText(message.rawMarkdown).catch(console.error);
  }, [message.rawMarkdown]);

  return (
    <div
      className={`message ${message.role}${message.isStreaming ? ' streaming' : ''}`}
      data-raw-markdown={message.rawMarkdown}
    >
      <div className="message-content">
        {message.blocks.map((block: MsgBlock) => {
          switch (block.kind) {
            case 'thinking': return <ThinkingBlockView key={block.id} block={block} />;
            case 'text': return <TextBlockView key={block.id} block={block} />;
            case 'tool': return <ToolBlockView key={block.id} block={block} />;
            case 'shell': return <ShellBlockView key={block.id} block={block} sessionId={sessionId} />;
            case 'iteration-marker': return <IterationMarkerView key={block.id} block={block} />;
            default: return null;
          }
        })}
      </div>
      {message.role === 'assistant' && !message.isStreaming && message.rawMarkdown && (
        <button type="button" className="copy-btn" title="Copy" onClick={copyText}>⧉</button>
      )}
    </div>
  );
};

// ─── MessageList ──────────────────────────────────────────────────────────────

export interface MessageListProps {
  messages: AgentMessage[];
  streamingMessage: AgentMessage | null;
  sessionId: string;
  onScrollChange: (scrolledUp: boolean) => void;
  canResume?: boolean;
  isResuming?: boolean;
  onResume?: () => void;
}

const SCROLL_THRESHOLD = 60;

export const MessageList: React.FC<MessageListProps> = ({
  messages, streamingMessage, sessionId, onScrollChange, canResume = false, isResuming = false, onResume,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const [userScrolledUp, setUserScrolledUp] = useState(false);

  // Auto-scroll unless user has scrolled up
  useEffect(() => {
    if (!containerRef.current || userScrolledUp) return;
    containerRef.current.scrollTop = containerRef.current.scrollHeight;
  });

  const handleScroll = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
    const scrolled = dist > SCROLL_THRESHOLD;
    setUserScrolledUp(scrolled);
    onScrollChange(scrolled);
  }, [onScrollChange]);

  const scrollToBottom = useCallback(() => {
    setUserScrolledUp(false);
    onScrollChange(false);
    if (containerRef.current) containerRef.current.scrollTop = containerRef.current.scrollHeight;
  }, [onScrollChange]);

  const allMessages = streamingMessage ? [...messages, streamingMessage] : messages;

  return (
    <div className="messages-container" ref={containerRef} onScroll={handleScroll}>
      {allMessages.map((msg) => (
        <MessageView key={msg.id} message={msg} sessionId={sessionId} />
      ))}
      {canResume && (
        <div className={`paused-marker${isResuming ? ' resumed' : ''}`}>
          <span className="pause-icon" aria-hidden="true">⏸</span>
          <span>{isResuming ? 'Resuming interrupted response…' : 'Response interrupted. Resume from where it stopped.'}</span>
          {!isResuming && onResume && (
            <button type="button" className="resume-btn" onClick={onResume}>
              Resume
            </button>
          )}
        </div>
      )}
      {userScrolledUp && (
        <button type="button" className="scroll-to-bottom-btn visible" onClick={scrollToBottom}>
          ↓
        </button>
      )}
    </div>
  );
};

export default MessageList;

