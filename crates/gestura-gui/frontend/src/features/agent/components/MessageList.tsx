/**
 * MessageList — renders all agent/user messages including streaming blocks.
 */
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';

import { parseMarkdown } from '../utils/markdown';
import { enhanceTaskReferenceHtml } from '../utils/taskLinks';
import { buildToolPresentation } from '../utils/toolActivity';
import type {
  AgentMessage,
  MsgBlock,
  ThinkingBlock,
  TextBlock,
  ToolBlock,
  IterationMarkerBlock,
  NarrationBlock,
  TaskHierarchy,
} from '../types';
import { ShellConsoleView } from './ShellConsoleView';

// ─── Block renderers ──────────────────────────────────────────────────────────

type MessageRenderOptions = {
  linkifyTaskReferences?: boolean;
};

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

function renderMessageHtml(content: string, tasks: TaskHierarchy, options: MessageRenderOptions = {}): string {
  const html = parseMarkdown(content);
  if (options.linkifyTaskReferences === false) return html;
  return enhanceTaskReferenceHtml(html, tasks);
}

const TextBlockView: React.FC<{
  block: TextBlock;
  tasks: TaskHierarchy;
  role: AgentMessage['role'];
}> = ({ block, tasks, role }) => (
  <div
    className="text-content markdown-body"
    dangerouslySetInnerHTML={{
      __html: renderMessageHtml(block.content, tasks, { linkifyTaskReferences: role !== 'user' }),
    }}
  />
);

const StreamingPlaceholderView: React.FC = () => (
  <div
    className="message-streaming-placeholder"
    data-testid="message-streaming-placeholder"
    aria-label="Agent is responding"
  >
    <span className="message-streaming-placeholder-dot" />
    <span className="message-streaming-placeholder-dot" />
    <span className="message-streaming-placeholder-dot" />
  </div>
);

function toolStatusLabel(block: ToolBlock): string {
  if (block.status === 'running') return 'Running…';
  if (block.status === 'executing') return 'Executing…';
  if (block.status === 'success') return `Success${block.durationMs != null ? ` • ${block.durationMs}ms` : ''}`;
  if (block.status === 'error') return `Error${block.durationMs != null ? ` • ${block.durationMs}ms` : ''}`;
  return 'Blocked';
}

function toolIconKey(name: string): string {
  switch (name.toLowerCase()) {
    case 'file': return 'file';
    case 'git': return 'git';
    case 'code': return 'code';
    case 'web': return 'web';
    case 'web_search': return 'search';
    case 'task':
    case 'tasks': return 'task';
    case 'mcp': return 'mcp';
    case 'screenshot': return 'screenshot';
    case 'screen_record': return 'record';
    case 'shell': return 'shell';
    default: return 'default';
  }
}

const ToolIcon: React.FC<{ name: string }> = ({ name }) => {
  switch (toolIconKey(name)) {
    case 'file':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M4.25 2.75h4.7l2.8 2.8v7a.7.7 0 0 1-.7.7h-6.8a.7.7 0 0 1-.7-.7v-9.1a.7.7 0 0 1 .7-.7Z" fill="none" stroke="currentColor" strokeWidth="1.2" />
          <path d="M8.95 2.9v2.55h2.55" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
        </svg>
      );
    case 'git':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M5.2 3.1a1.5 1.5 0 1 0 0 3a1.5 1.5 0 0 0 0-3Zm5.6 6.8a1.5 1.5 0 1 0 0 3a1.5 1.5 0 0 0 0-3ZM5.2 6.1v4.1a1.5 1.5 0 1 0 1.2 0V8.6h3.2a1.5 1.5 0 1 0 0-1.2H6.4V6.1" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.2" />
        </svg>
      );
    case 'code':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="m5.4 4.4-3 3.6 3 3.6M10.6 4.4l3 3.6-3 3.6M8.9 3 7.1 13" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.2" />
        </svg>
      );
    case 'web':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <circle cx="8" cy="8" r="5.4" fill="none" stroke="currentColor" strokeWidth="1.2" />
          <path d="M2.9 8h10.2M8 2.6c1.4 1.4 2.2 3.3 2.2 5.4 0 2.1-.8 4-2.2 5.4M8 2.6C6.6 4 5.8 5.9 5.8 8c0 2.1.8 4 2.2 5.4" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      );
    case 'search':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <circle cx="7" cy="7" r="3.8" fill="none" stroke="currentColor" strokeWidth="1.2" />
          <path d="m10 10 3 3" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.2" />
        </svg>
      );
    case 'task':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M6.2 4.2h6.1M6.2 8h6.1M6.2 11.8h6.1M3.2 4.2h.01M3.2 8h.01M3.2 11.8h.01" fill="none" stroke="currentColor" strokeLinecap="round" strokeWidth="1.2" />
        </svg>
      );
    case 'mcp':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="2.8" y="3" width="4.2" height="4.2" rx="0.8" fill="none" stroke="currentColor" strokeWidth="1.2" />
          <rect x="9" y="3" width="4.2" height="4.2" rx="0.8" fill="none" stroke="currentColor" strokeWidth="1.2" />
          <rect x="5.9" y="8.8" width="4.2" height="4.2" rx="0.8" fill="none" stroke="currentColor" strokeWidth="1.2" />
          <path d="M7 5.1h2M8 7.2v1.6" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
        </svg>
      );
    case 'screenshot':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <rect x="2.7" y="4" width="10.6" height="8" rx="1.4" fill="none" stroke="currentColor" strokeWidth="1.2" />
          <circle cx="8" cy="8" r="2.1" fill="none" stroke="currentColor" strokeWidth="1.2" />
          <path d="M5.4 4 6.2 2.9h3.6L10.6 4" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case 'record':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <circle cx="8" cy="8" r="2.4" fill="currentColor" />
          <rect x="3.2" y="3.2" width="9.6" height="9.6" rx="2.1" fill="none" stroke="currentColor" strokeWidth="1.2" />
        </svg>
      );
    case 'shell':
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="m4.5 5.1 2 1.9-2 1.9M7.8 9h3.5" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.2" />
        </svg>
      );
    default:
      return (
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M8 2.8 9.3 4l1.8-.2.5 1.7 1.6.9-.8 1.6.8 1.6-1.6.9-.5 1.7-1.8-.2L8 13.2l-1.3-1.2-1.8.2-.5-1.7-1.6-.9.8-1.6-.8-1.6 1.6-.9.5-1.7 1.8.2L8 2.8Zm0 3a2.2 2.2 0 1 0 0 4.4 2.2 2.2 0 0 0 0-4.4Z" fill="none" stroke="currentColor" strokeWidth="1.1" strokeLinejoin="round" />
        </svg>
      );
  }
};

const ToolBlockView: React.FC<{ block: ToolBlock }> = ({ block }) => {
  const [collapsed, setCollapsed] = useState(block.collapsed);
  const presentation = buildToolPresentation(block);

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
          <span className={`tool-call-icon tool-call-icon--${toolIconKey(block.name)}`} aria-hidden="true"><ToolIcon name={block.name} /></span>
          <span className="tool-call-copy">
            <strong className="tool-call-name">{presentation.title}</strong>
          </span>
          <span className={`tool-call-status tool-call-status--${block.status}`}>{toolStatusLabel(block)}</span>
          <span className="tool-call-chevron">{collapsed ? '▸' : '▾'}</span>
        </button>
        {!collapsed && (
          <div className="tool-call-details">
            {(presentation.eyebrow || presentation.detail) && (
              <div className="tool-call-meta">
                <span className="tool-call-label">{presentation.eyebrow}</span>
                {presentation.detail && (
                  <span className="tool-call-detail" title={presentation.detail}>{presentation.detail}</span>
                )}
              </div>
            )}
            {presentation.parameterItems.length > 0 && (
              <section className="tool-call-section" aria-label="Parameters">
                <div className="tool-call-grid">
                  {presentation.parameterItems.map((item) => (
                    <div key={`${block.id}-arg-${item.label}`} className="tool-call-kv">
                      <span>{item.label}</span>
                      <strong title={item.value}>{item.value}</strong>
                    </div>
                  ))}
                </div>
              </section>
            )}
            {block.result != null && !screenshotSrc && (
              <section className="tool-call-section" aria-label="Response">
                <div className={`tool-result ${block.status}`}>
                  <p>{presentation.responseSummary}</p>
                  {presentation.responseItems.length > 0 && (
                    <div className="tool-call-grid">
                      {presentation.responseItems.map((item) => (
                        <div key={`${block.id}-result-${item.label}`} className="tool-call-kv">
                          <span>{item.label}</span>
                          <strong title={item.value}>{item.value}</strong>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </section>
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

const NARRATION_COLLAPSE_THRESHOLD_WORDS = 60;

function narrationWordCount(message: string): number {
  const trimmed = message.trim();
  return trimmed ? trimmed.split(/\s+/).length : 0;
}

function narrationFallbackTitle(stage: NarrationBlock['stage']): string {
  switch (stage) {
    case 'context': return 'Gathering context';
    case 'planning': return 'Planning next step';
    case 'execution': return 'Working on request';
    case 'verification': return 'Checking results';
    case 'blocked': return 'Waiting on blocker';
    case 'progress':
    default:
      return 'Tracking progress';
  }
}

function narrationDisplayMessage(block: NarrationBlock): string {
  const message = block.message.trim();
  if (message) return message;

  const sections = [block.summary, block.reason, block.nextStep]
    .map((part) => part?.trim() ?? '')
    .filter((part) => part.length > 0);

  const evidenceLines = block.evidence
    .map((item) => item.trim())
    .filter((item) => item.length > 0)
    .map((item) => (/^([-+*]|\d+\.)\s+/.test(item) ? item : `- ${item}`));

  if (evidenceLines.length > 0) {
    sections.push(evidenceLines.join('\n'));
  }

  return sections.join('\n\n');
}

const NarrationMarkdown: React.FC<{
  content: string;
  indented?: boolean;
  tasks: TaskHierarchy;
}> = ({ content, indented = false, tasks }) => (
  <div
    className={`agent-narration-text markdown-body${indented ? ' agent-narration-text--indented' : ''}`}
    dangerouslySetInnerHTML={{ __html: renderMessageHtml(content, tasks) }}
  />
);

const NarrationBlockView: React.FC<{
  block: NarrationBlock;
  isFirstNarrationInMessage: boolean;
  hasPreviousNarration: boolean;
  hasNextNarration: boolean;
  tasks: TaskHierarchy;
}> = ({
  block,
  isFirstNarrationInMessage,
  hasPreviousNarration,
  hasNextNarration,
  tasks,
}) => {
    const displayMessage = narrationDisplayMessage(block);
    const wordCount = narrationWordCount(displayMessage);
    const usesTitle = wordCount > NARRATION_COLLAPSE_THRESHOLD_WORDS;
    const shouldStayInline = isFirstNarrationInMessage || block.stage === 'planning';
    const startsExpanded = shouldStayInline;
    const [expanded, setExpanded] = useState(startsExpanded);
    const title = (block.title?.trim() || narrationFallbackTitle(block.stage)).trim();
    const narrationClassName = [
      'agent-narration',
      hasPreviousNarration ? 'agent-narration--after-narration' : '',
      hasNextNarration ? 'agent-narration--before-narration' : '',
      usesTitle && !shouldStayInline ? 'agent-narration--collapsible' : '',
      expanded && usesTitle && !shouldStayInline ? 'expanded' : '',
    ]
      .filter(Boolean)
      .join(' ');

    if (!usesTitle || shouldStayInline) {
      return (
        <div className={narrationClassName}>
          <NarrationMarkdown content={displayMessage} tasks={tasks} />
        </div>
      );
    }

    return (
      <div className={narrationClassName}>
        <button
          type="button"
          className="agent-narration-toggle"
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          <strong className="agent-narration-title">{title}</strong>
          <span className="agent-narration-chevron" aria-hidden="true">{expanded ? '▾' : '▸'}</span>
        </button>
        {expanded && <NarrationMarkdown content={displayMessage} indented tasks={tasks} />}
      </div>
    );
  };

// ─── Single message ───────────────────────────────────────────────────────────

const MessageView: React.FC<{
  message: AgentMessage;
  tasks: TaskHierarchy;
  onRevealShellSession?: (shellSessionId: string | null) => void;
}> = ({ message, tasks, onRevealShellSession }) => {
  const copyText = useCallback(() => {
    navigator.clipboard.writeText(message.rawMarkdown).catch(console.error);
  }, [message.rawMarkdown]);
  const hasShellBlock = message.blocks.some((block) => block.kind === 'shell' || block.kind === 'shell-session');
  const firstNarrationIndex = message.blocks.findIndex((block) => block.kind === 'narration');

  return (
    <div
      className={`message ${message.role}${message.isStreaming ? ' streaming' : ''}${message.isStreaming && message.blocks.length === 0 ? ' empty-streaming' : ''}`}
      data-raw-markdown={message.rawMarkdown}
    >
      <div className="message-content">
        {message.isStreaming && message.blocks.length === 0 && <StreamingPlaceholderView />}
        {message.blocks.map((block: MsgBlock, index) => {
          const previousBlock = message.blocks[index - 1];
          const nextBlock = message.blocks[index + 1];
          switch (block.kind) {
            case 'thinking': return <ThinkingBlockView key={block.id} block={block} />;
            case 'text': return <TextBlockView key={block.id} block={block} tasks={tasks} role={message.role} />;
            case 'narration':
              return (
                <NarrationBlockView
                  key={block.id}
                  block={block}
                  isFirstNarrationInMessage={message.blocks[firstNarrationIndex]?.id === block.id}
                  hasPreviousNarration={previousBlock?.kind === 'narration'}
                  hasNextNarration={nextBlock?.kind === 'narration'}
                  tasks={tasks}
                />
              );
            case 'tool':
              if (block.name === 'shell' && hasShellBlock) return null;
              return <ToolBlockView key={block.id} block={block} />;
            case 'shell':
            case 'shell-session':
              return (
                <ShellConsoleView
                  key={block.id}
                  block={block}
                  onRevealSession={onRevealShellSession}
                />
              );
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
  tasks?: TaskHierarchy;
  userScrolledUp: boolean;
  onScrollChange: (scrolledUp: boolean) => void;
  onRevealShellSession?: (shellSessionId: string | null) => void;
  canResume?: boolean;
  isResuming?: boolean;
  onResume?: () => void;
}

const SCROLL_THRESHOLD = 60;

function bottomScrollTop(element: HTMLDivElement): number {
  return Math.max(0, element.scrollHeight - element.clientHeight);
}

export const MessageList: React.FC<MessageListProps> = ({
  messages,
  streamingMessage,
  tasks = [],
  userScrolledUp,
  onScrollChange,
  onRevealShellSession,
  canResume = false,
  isResuming = false,
  onResume,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);

  // Auto-scroll unless user has scrolled up
  useEffect(() => {
    const element = containerRef.current;
    if (!element || userScrolledUp) return;
    element.scrollTop = bottomScrollTop(element);
  });

  const handleScroll = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    const dist = el.scrollHeight - el.scrollTop - el.clientHeight;
    const scrolled = dist > SCROLL_THRESHOLD;
    onScrollChange(scrolled);
  }, [onScrollChange]);

  const handleWheel = useCallback((event: React.WheelEvent<HTMLDivElement>) => {
    if (event.deltaY < 0) onScrollChange(true);
  }, [onScrollChange]);

  const scrollToBottom = useCallback(() => {
    onScrollChange(false);
    if (containerRef.current) containerRef.current.scrollTop = bottomScrollTop(containerRef.current);
  }, [onScrollChange]);

  const allMessages = streamingMessage ? [...messages, streamingMessage] : messages;

  return (
    <div className="messages-viewport">
      <div className="messages-container" ref={containerRef} onScroll={handleScroll} onWheel={handleWheel}>
        {allMessages.map((msg) => (
          <MessageView
            key={msg.id}
            message={msg}
            tasks={tasks}
            onRevealShellSession={onRevealShellSession}
          />
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
      </div>
      <button
        type="button"
        className={`scroll-to-bottom-btn${userScrolledUp ? ' visible' : ''}`}
        onClick={scrollToBottom}
        aria-hidden={!userScrolledUp}
        tabIndex={userScrolledUp ? 0 : -1}
      >
        ↓ New messages
      </button>
    </div>
  );
};

export default MessageList;

