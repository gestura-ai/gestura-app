import { useState } from 'react';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { AgentMessage, TaskHierarchy } from '../types';
import { parseMarkdown } from '../utils/markdown';
import { taskLinkHref } from '../utils/taskLinks';
import { MessageList } from './MessageList';

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
}));

vi.mock('./InteractiveShellTerminal', () => ({
  InteractiveShellTerminal: ({
    shell,
    readOnly,
    className,
  }: {
    shell: { lines: Array<{ data: string }> };
    readOnly?: boolean;
    className?: string;
  }) => (
    <div data-testid="interactive-shell-terminal" data-read-only={readOnly ? 'true' : 'false'} className={className}>
      {shell.lines.map((line) => line.data).join('')}
    </div>
  ),
}));

afterEach(() => {
  cleanup();
});

function renderMessageList(
  messages: AgentMessage[],
  tasks: TaskHierarchy = [],
  onRevealShellSession = vi.fn(),
  userScrolledUp = false,
  streamingMessage: AgentMessage | null = null,
) {
  const renderResult = render(
    <MessageList
      messages={messages}
      streamingMessage={streamingMessage}
      tasks={tasks}
      userScrolledUp={userScrolledUp}
      onScrollChange={vi.fn()}
      onRevealShellSession={onRevealShellSession}
    />,
  );

  return { ...renderResult, onRevealShellSession };
}

describe('MessageList', () => {
  it('renders a neutral streaming placeholder instead of a fake thought block', () => {
    const streamingMessage: AgentMessage = {
      id: 'streaming-placeholder',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: true,
      timestamp: Date.now(),
      blocks: [],
    };

    renderMessageList([], [], vi.fn(), false, streamingMessage);

    expect(screen.getByTestId('message-streaming-placeholder')).toBeInTheDocument();
    expect(screen.queryByText('Thought Process')).not.toBeInTheDocument();
    expect(screen.queryByText('Thinking Process…')).not.toBeInTheDocument();
  });

  it('scrolls to the latest content with a single click on the new-messages badge', () => {
    const message: AgentMessage = {
      id: 'message-scroll',
      role: 'assistant',
      rawMarkdown: 'Latest reply',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{ kind: 'text', id: 'text-scroll', content: 'Latest reply' }],
    };

    function ScrollHarness() {
      const [scrolledUp, setScrolledUp] = useState(true);

      return (
        <MessageList
          messages={[message]}
          streamingMessage={null}
          userScrolledUp={scrolledUp}
          onScrollChange={setScrolledUp}
          onRevealShellSession={vi.fn()}
        />
      );
    }

    const { container } = render(<ScrollHarness />);
    const messagesContainer = container.querySelector('.messages-container') as HTMLDivElement;

    Object.defineProperty(messagesContainer, 'scrollHeight', { configurable: true, value: 480 });
    Object.defineProperty(messagesContainer, 'clientHeight', { configurable: true, value: 120 });
    messagesContainer.scrollTop = 40;

    fireEvent.click(screen.getByRole('button', { name: /new messages/i }));

    expect(messagesContainer.scrollTop).toBe(360);
    expect(screen.queryByRole('button', { name: /new messages/i })).not.toBeInTheDocument();
  });

  it('renders compact tool summaries with structured parameters and responses', () => {
    renderMessageList([{
      id: 'message-1',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'tool',
        id: 'tool-1',
        name: 'file',
        args: JSON.stringify({
          operation: 'write',
          path: 'src/main.ts',
          content: 'console.log("too much raw content")',
        }),
        status: 'success',
        result: 'Wrote src/main.ts',
        durationMs: 18,
        collapsed: true,
      }],
    }]);

    expect(screen.getByText('Updating file')).toBeInTheDocument();
    expect(screen.queryByText('Wrote src/main.ts')).not.toBeInTheDocument();
    expect(screen.queryByText('console.log("too much raw content")')).not.toBeInTheDocument();
    expect(screen.queryByText('src/main.ts')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Updating file/i }));

    expect(screen.getAllByText('src/main.ts').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Wrote src/main.ts').length).toBeGreaterThan(0);
    expect(screen.getByText('35 chars')).toBeInTheDocument();
  });

  it('links task names and replaces raw task ids with titles in assistant text', () => {
    const tasks: TaskHierarchy = [{
      id: 'task-run-tests',
      name: 'Run tests',
      description: 'Verify the build.',
      status: 'NotStarted',
      subtasks: [],
    }];

    renderMessageList([{
      id: 'message-task-links',
      role: 'assistant',
      rawMarkdown: 'Please start with Run tests, then update task-run-tests once validation finishes.',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'text',
        id: 'text-task-links',
        content: 'Please start with Run tests, then update task-run-tests once validation finishes.',
      }],
    }], tasks);

    const links = screen.getAllByRole('link', { name: 'Run tests' });
    expect(links).toHaveLength(2);
    expect(links[0]).toHaveAttribute('href', taskLinkHref('task-run-tests'));
    expect(links[1]).toHaveAttribute('href', taskLinkHref('task-run-tests'));
    expect(screen.queryByText('task-run-tests')).not.toBeInTheDocument();
  });

  it('does not auto-link task references inside user-authored text', () => {
    const tasks: TaskHierarchy = [{
      id: 'task-run-tests',
      name: 'Run tests',
      description: 'Verify the build.',
      status: 'NotStarted',
      subtasks: [],
    }];

    renderMessageList([{
      id: 'message-user-task-text',
      role: 'user',
      rawMarkdown: 'I typed Run tests and task-run-tests in my prompt.',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'text',
        id: 'text-user-task-text',
        content: 'I typed Run tests and task-run-tests in my prompt.',
      }],
    }], tasks);

    expect(screen.queryByRole('link', { name: 'Run tests' })).not.toBeInTheDocument();
    expect(screen.getByText(/I typed Run tests and task-run-tests in my prompt\./i)).toBeInTheDocument();
  });

  it('hides duplicate shell tool cards and links inline shells to the manager', () => {
    const onRevealShellSession = vi.fn();

    renderMessageList([{
      id: 'message-2',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [
        {
          kind: 'tool',
          id: 'tool-shell',
          name: 'shell',
          args: JSON.stringify({ command: 'cargo test' }),
          status: 'success',
          result: 'Finished tests',
          durationMs: 42,
          collapsed: true,
        },
        {
          kind: 'shell',
          id: 'shell-1',
          processId: 'proc-1',
          shellSessionId: 'shell-session-1',
          command: 'cargo test',
          cwd: '/workspace',
          state: 'Running',
          durationMs: 42,
          lastActivityAt: Date.now(),
          lines: [{ stream: 'Stdout', data: 'running tests...\n' }],
          collapsed: true,
        },
      ],
    }], [], onRevealShellSession);

    expect(screen.queryByText('Running shell command')).not.toBeInTheDocument();
    expect(screen.getByText('cargo test')).toBeInTheDocument();
    expect(screen.queryByText('/workspace')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Copy command' })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /cargo test/i }));
    expect(screen.getByText('/workspace')).toBeInTheDocument();
    expect(screen.getByTestId('interactive-shell-terminal')).toHaveTextContent('running tests...');
    expect(screen.getByTestId('interactive-shell-terminal')).toHaveAttribute('data-read-only', 'true');

    fireEvent.click(screen.getByRole('button', { name: 'Open in Shell Session Manager' }));
    expect(onRevealShellSession).toHaveBeenCalledWith('shell-session-1');
  });

  it('also hides shell tool cards when chat is rendering a shell-session block', () => {
    renderMessageList([{
      id: 'message-2-session',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [
        {
          kind: 'tool',
          id: 'tool-shell-session',
          name: 'shell',
          args: JSON.stringify({ command: 'cargo test' }),
          status: 'success',
          result: 'Finished tests',
          durationMs: 42,
          collapsed: true,
        },
        {
          kind: 'shell-session',
          id: 'shell-session-2',
          shellSessionId: 'shell-session-2',
          cwd: '/workspace',
          state: 'Busy',
          interactive: true,
          userManaged: false,
          activeProcessId: 'proc-2',
          activeCommand: 'cargo test',
          lastExitCode: null,
          durationMs: 42,
          lastActivityAt: Date.now(),
          lines: [{ stream: 'Stdout', data: 'running tests...\n' }],
          collapsed: true,
          availableForReuse: false,
        },
      ],
    }]);

    expect(screen.queryByText('Running shell command')).not.toBeInTheDocument();
    expect(screen.getByText('cargo test')).toBeInTheDocument();
  });

  it('shows Complete for a successfully finished inline shell session', () => {
    renderMessageList([{
      id: 'message-complete-shell',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [
        {
          kind: 'shell-session',
          id: 'shell-session-complete',
          shellSessionId: 'shell-session-complete',
          cwd: '/workspace',
          state: 'Idle',
          interactive: true,
          userManaged: false,
          activeProcessId: null,
          activeCommand: null,
          lastExitCode: 0,
          durationMs: 4200,
          lastActivityAt: Date.now(),
          lines: [{ stream: 'Stdout', data: '$ cargo test\nAll tests passed\n' }],
          collapsed: true,
          availableForReuse: true,
        },
      ],
    }]);

    expect(screen.getByText('Complete')).toBeInTheDocument();
    expect(screen.queryByText('Idle')).not.toBeInTheDocument();
  });

  it('automatically collapses an inline shell session when it completes successfully', () => {
    const timestamp = Date.now();
    const baseMessage: AgentMessage = {
      id: 'message-auto-collapse-shell',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp,
      blocks: [
        {
          kind: 'shell-session',
          id: 'shell-session-auto-collapse',
          shellSessionId: 'shell-session-auto-collapse',
          cwd: '/workspace',
          state: 'Busy',
          interactive: true,
          userManaged: false,
          activeProcessId: 'proc-auto-collapse',
          activeCommand: 'cargo test',
          lastExitCode: null,
          durationMs: null,
          lastActivityAt: timestamp,
          lines: [{ stream: 'Stdout', data: '$ cargo test\nrunning...\n' }],
          collapsed: true,
          availableForReuse: false,
        },
      ],
    };

    const { rerender } = renderMessageList([baseMessage]);

    fireEvent.click(screen.getByRole('button', { name: /cargo test/i }));
    expect(screen.getByText('/workspace')).toBeInTheDocument();

    rerender(
      <MessageList
        messages={[{
          ...baseMessage,
          blocks: [
            {
              kind: 'shell-session',
              id: 'shell-session-auto-collapse',
              shellSessionId: 'shell-session-auto-collapse',
              cwd: '/workspace',
              state: 'Idle',
              interactive: true,
              userManaged: false,
              activeProcessId: null,
              activeCommand: null,
              lastExitCode: 0,
              durationMs: 1200,
              lastActivityAt: timestamp + 1000,
              lines: [{ stream: 'Stdout', data: '$ cargo test\nAll tests passed\n' }],
              collapsed: false,
              availableForReuse: true,
            },
          ],
        }]}
        streamingMessage={null}
        tasks={[]}
        userScrolledUp={false}
        onScrollChange={vi.fn()}
        onRevealShellSession={vi.fn()}
      />,
    );

    expect(screen.queryByText('/workspace')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Interactive shell session/i })).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByText('Complete')).toBeInTheDocument();
  });

  it('automatically collapses an inline shell session when a reusable command ends with a non-zero exit', () => {
    const timestamp = Date.now();
    const baseMessage: AgentMessage = {
      id: 'message-auto-collapse-shell-failure',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp,
      blocks: [
        {
          kind: 'shell-session',
          id: 'shell-session-auto-collapse-failure',
          shellSessionId: 'shell-session-auto-collapse-failure',
          cwd: '/workspace',
          state: 'Busy',
          interactive: true,
          userManaged: false,
          activeProcessId: 'proc-auto-collapse-failure',
          activeCommand: 'cargo test',
          lastExitCode: null,
          durationMs: null,
          lastActivityAt: timestamp,
          lines: [{ stream: 'Stdout', data: '$ cargo test\nrunning...\n' }],
          collapsed: true,
          availableForReuse: false,
        },
      ],
    };

    const { rerender } = renderMessageList([baseMessage]);

    fireEvent.click(screen.getByRole('button', { name: /cargo test/i }));
    expect(screen.getByText('/workspace')).toBeInTheDocument();

    rerender(
      <MessageList
        messages={[{
          ...baseMessage,
          blocks: [
            {
              kind: 'shell-session',
              id: 'shell-session-auto-collapse-failure',
              shellSessionId: 'shell-session-auto-collapse-failure',
              cwd: '/workspace',
              state: 'Idle',
              interactive: true,
              userManaged: false,
              activeProcessId: null,
              activeCommand: null,
              lastExitCode: 1,
              durationMs: 1200,
              lastActivityAt: timestamp + 1000,
              lines: [{ stream: 'Stderr', data: '$ cargo test\nerror: tests failed\n' }],
              collapsed: false,
              availableForReuse: true,
            },
          ],
        }]}
        streamingMessage={null}
        tasks={[]}
        userScrolledUp={false}
        onScrollChange={vi.fn()}
        onRevealShellSession={vi.fn()}
      />,
    );

    expect(screen.queryByText('/workspace')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Interactive shell session/i })).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByText('Exit 1')).toBeInTheDocument();
  });

  it('automatically collapses an inline shell session when the session stops', () => {
    const timestamp = Date.now();
    const baseMessage: AgentMessage = {
      id: 'message-auto-collapse-shell-stopped',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp,
      blocks: [
        {
          kind: 'shell-session',
          id: 'shell-session-auto-collapse-stopped',
          shellSessionId: 'shell-session-auto-collapse-stopped',
          cwd: '/workspace',
          state: 'Busy',
          interactive: true,
          userManaged: false,
          activeProcessId: 'proc-auto-collapse-stopped',
          activeCommand: 'npm run dev',
          lastExitCode: null,
          durationMs: null,
          lastActivityAt: timestamp,
          lines: [{ stream: 'Stdout', data: '$ npm run dev\nstarting...\n' }],
          collapsed: true,
          availableForReuse: false,
        },
      ],
    };

    const { rerender } = renderMessageList([baseMessage]);

    fireEvent.click(screen.getByRole('button', { name: /npm run dev/i }));
    expect(screen.getByText('/workspace')).toBeInTheDocument();

    rerender(
      <MessageList
        messages={[{
          ...baseMessage,
          blocks: [
            {
              kind: 'shell-session',
              id: 'shell-session-auto-collapse-stopped',
              shellSessionId: 'shell-session-auto-collapse-stopped',
              cwd: '/workspace',
              state: 'Stopped',
              interactive: true,
              userManaged: false,
              activeProcessId: null,
              activeCommand: null,
              lastExitCode: 130,
              durationMs: 1200,
              lastActivityAt: timestamp + 1000,
              lines: [{ stream: 'Stdout', data: '$ npm run dev\nterminated\n' }],
              collapsed: false,
              availableForReuse: false,
            },
          ],
        }]}
        streamingMessage={null}
        tasks={[]}
        userScrolledUp={false}
        onScrollChange={vi.fn()}
        onRevealShellSession={vi.fn()}
      />,
    );

    expect(screen.queryByText('/workspace')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Interactive shell session/i })).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByText('Stopped')).toBeInTheDocument();
  });

  it('re-expands an auto-collapsed inline shell session when the reusable session starts another command', () => {
    const timestamp = Date.now();
    const baseMessage: AgentMessage = {
      id: 'message-shell-reexpand',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp,
      blocks: [
        {
          kind: 'shell-session',
          id: 'shell-session-reexpand',
          shellSessionId: 'shell-session-reexpand',
          cwd: '/workspace',
          state: 'Busy',
          interactive: true,
          userManaged: false,
          activeProcessId: 'proc-reexpand-1',
          activeCommand: 'cargo test',
          lastExitCode: null,
          durationMs: null,
          lastActivityAt: timestamp,
          lines: [{ stream: 'Stdout', data: '$ cargo test\nrunning...\n' }],
          collapsed: true,
          availableForReuse: false,
        },
      ],
    };

    const { rerender } = renderMessageList([baseMessage]);

    fireEvent.click(screen.getByRole('button', { name: /cargo test/i }));
    expect(screen.getByText('/workspace')).toBeInTheDocument();

    rerender(
      <MessageList
        messages={[{
          ...baseMessage,
          blocks: [
            {
              kind: 'shell-session',
              id: 'shell-session-reexpand',
              shellSessionId: 'shell-session-reexpand',
              cwd: '/workspace',
              state: 'Idle',
              interactive: true,
              userManaged: false,
              activeProcessId: null,
              activeCommand: null,
              lastExitCode: 0,
              durationMs: 1200,
              lastActivityAt: timestamp + 1000,
              lines: [{ stream: 'Stdout', data: '$ cargo test\nAll tests passed\n' }],
              collapsed: false,
              availableForReuse: true,
            },
          ],
        }]}
        streamingMessage={null}
        tasks={[]}
        userScrolledUp={false}
        onScrollChange={vi.fn()}
        onRevealShellSession={vi.fn()}
      />,
    );

    expect(screen.queryByText('/workspace')).not.toBeInTheDocument();
    expect(screen.getByText('Complete')).toBeInTheDocument();

    rerender(
      <MessageList
        messages={[{
          ...baseMessage,
          blocks: [
            {
              kind: 'shell-session',
              id: 'shell-session-reexpand',
              shellSessionId: 'shell-session-reexpand',
              cwd: '/workspace',
              state: 'Busy',
              interactive: true,
              userManaged: false,
              activeProcessId: 'proc-reexpand-2',
              activeCommand: 'cargo fmt --check',
              lastExitCode: 0,
              durationMs: null,
              lastActivityAt: timestamp + 2000,
              lines: [{ stream: 'Stdout', data: '$ cargo fmt --check\nchecking...\n' }],
              collapsed: false,
              availableForReuse: false,
            },
          ],
        }]}
        streamingMessage={null}
        tasks={[]}
        userScrolledUp={false}
        onScrollChange={vi.fn()}
        onRevealShellSession={vi.fn()}
      />,
    );

    expect(screen.getByRole('button', { name: /cargo fmt --check/i })).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('/workspace')).toBeInTheDocument();
    expect(screen.getByText('Running…')).toBeInTheDocument();
  });

  it('explains when a stalled inline shell looks like it is waiting for input', () => {
    const nowSpy = vi.spyOn(Date, 'now').mockReturnValue(120_000);

    renderMessageList([{
      id: 'message-2b',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: 120_000,
      blocks: [{
        kind: 'shell',
        id: 'shell-prompt',
        processId: 'proc-prompt',
        shellSessionId: 'shell-session-prompt',
        command: 'pnpm add vite',
        cwd: '/workspace',
        state: 'Running',
        lastActivityAt: 10_000,
        lines: [{ stream: 'Stdout', data: 'Need to install the following packages:\nProceed? (y/n)' }],
        collapsed: true,
      }],
    }]);

    fireEvent.click(screen.getByRole('button', { name: /pnpm add vite/i }));

    const note = screen.getByText('Likely waiting for input').closest('.shell-console-note');
    expect(note).not.toBeNull();
    expect(within(note as HTMLElement).getByText(/Open the Shell Session Manager to respond/i)).toBeInTheDocument();
    expect(within(note as HTMLElement).getByText(/Proceed\? \(y\/n\)/i)).toBeInTheDocument();

    nowSpy.mockRestore();
  });

  it('renders short narration inline without a collapse control', () => {
    renderMessageList([{
      id: 'message-3',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'narration',
        id: 'narration-1',
        title: 'Checking current files',
        stage: 'execution',
        message: 'I’m checking the current files before I make the next change.',
        evidence: [],
      }],
    }]);

    expect(screen.getByText('I’m checking the current files before I make the next change.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Checking current files/i })).not.toBeInTheDocument();
    expect(screen.queryByText(/Working:/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/Checking:/i)).not.toBeInTheDocument();
  });

  it('collapses long narration behind a titled row that expands on demand', () => {
    const longNarrationSegment = [
      'I’m comparing the current implementation with the latest results so I can confirm whether the next step should be a code change, a verification pass, or a task-state update before I move forward with the request.',
      'I also want to check whether the task state still matches what the tools have actually proven so far, because that affects whether I should keep inspecting, make an edit, or switch into validation mode next.',
      'After that, I need to trace the last verification branch, reconcile the new proof with the current task state, and explain why the safest next move is either another edit or a final validation pass.',
      'I’m also reviewing the earlier setup decisions, the current branch of work, the latest validation clues, and the remaining checks so the next narration update is grounded in what I actually confirmed instead of a vague summary that hides important detail from the user.',
      'If the latest proof supports the current direction I’ll keep moving through the planned implementation path, but if it exposes a mismatch I’ll pivot into another inspection step, explain why that branch changed, and make sure the user can see exactly what is driving the decision.',
    ].join(' ');
    const longNarration = `${longNarrationSegment} ${longNarrationSegment}`;

    renderMessageList([{
      id: 'message-4',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [
        {
          kind: 'narration',
          id: 'narration-setup',
          title: 'Checking setup state',
          stage: 'context',
          message: 'I’m checking the setup state before I compare the implementation against the latest results.',
          evidence: [],
        },
        {
          kind: 'text',
          id: 'text-before',
          content: 'Done with the earlier setup.',
        },
        {
          kind: 'narration',
          id: 'narration-2',
          title: 'Reviewing implementation state',
          stage: 'verification',
          message: longNarration,
          evidence: [],
        },
      ],
    }]);

    const toggle = screen.getByRole('button', { name: /Reviewing implementation state/i });
    expect(screen.queryByText(longNarration)).not.toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-expanded', 'false');

    fireEvent.click(toggle);

    expect(screen.getByText(longNarration)).toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(toggle.firstElementChild).toHaveClass('agent-narration-title');
    expect(toggle.lastElementChild).toHaveClass('agent-narration-chevron');
  });

  it('keeps the first narration in a message open as natural prose even when long', () => {
    const longNarration = 'I’m breaking this request into subtasks so I can start with the highest-leverage implementation step, keep the verification work attached to the actual code change, and make the next decision explicit before I move into execution. After that, I’ll take the first concrete subtask and narrate why it comes before the remaining queued work.';

    renderMessageList([{
      id: 'message-4b',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'narration',
        id: 'narration-2b',
        title: 'Planning next step',
        stage: 'planning',
        message: longNarration,
        evidence: [],
      }],
    }]);

    expect(screen.getByText(longNarration)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Planning next step/i })).not.toBeInTheDocument();
  });

  it('keeps non-planning narration inline until it exceeds 40 words', () => {
    const mediumNarration = 'I’m reviewing the latest terminal output, checking the changed files, and confirming the next safe step before I make the update in the current session so the implementation stays grounded in what the tools already proved.';

    renderMessageList([{
      id: 'message-5',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'narration',
        id: 'narration-setup-2',
        title: 'Checking setup state',
        stage: 'context',
        message: 'I’m checking the setup state before I narrate the next progress update.',
        evidence: [],
      }, {
        kind: 'narration',
        id: 'narration-3',
        title: 'Reviewing current progress',
        stage: 'progress',
        message: mediumNarration,
        evidence: [],
      }],
    }]);

    expect(screen.queryByRole('button', { name: /Reviewing current progress/i })).not.toBeInTheDocument();
    expect(screen.getByText(mediumNarration)).toBeInTheDocument();
  });

  it('renders structured narration as natural prose instead of labeled sections', () => {
    renderMessageList([{
      id: 'message-6',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'narration',
        id: 'narration-4',
        title: 'Verification is active',
        stage: 'verification',
        message: 'I verified the changed files and I’m moving into the targeted test pass now.',
        summary: 'The latest results cleared the file edit step and moved the tracked work into verification.',
        reason: 'That matters because the current task still needs direct proof before it can close cleanly.',
        nextStep: 'I’ll run the targeted test command and use that result to decide whether this task is actually done.',
        evidence: ['Current step: "Run targeted verification".', 'Still need to verify: targeted test evidence.'],
      }],
    }]);

    expect(screen.queryByRole('button', { name: /Verification is active/i })).not.toBeInTheDocument();
    expect(screen.getByText('I verified the changed files and I’m moving into the targeted test pass now.')).toBeInTheDocument();
    expect(screen.queryByText('Now')).not.toBeInTheDocument();
    expect(screen.queryByText('Why')).not.toBeInTheDocument();
    expect(screen.queryByText('Next')).not.toBeInTheDocument();
    expect(screen.queryByText('Evidence')).not.toBeInTheDocument();
    expect(screen.queryByText(/Current step/i)).not.toBeInTheDocument();
  });

  it('renders narration markdown instead of raw markdown text', () => {
    renderMessageList([{
      id: 'message-7',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'narration',
        id: 'narration-5',
        title: 'Reviewing results',
        stage: 'verification',
        message: 'I checked **build output** and confirmed `cargo test` is next.\n\n- verify failures\n- rerun tests',
        evidence: [],
      }],
    }]);

    const narration = screen.getByText('build output').closest('.agent-narration-text');

    expect(narration).not.toBeNull();
    expect(within(narration as HTMLElement).getByText('build output').tagName).toBe('STRONG');
    expect(within(narration as HTMLElement).getByText('cargo test').tagName).toBe('CODE');
    expect(within(narration as HTMLElement).getByText('verify failures').tagName).toBe('LI');
    expect(within(narration as HTMLElement).getByText('rerun tests').tagName).toBe('LI');
    expect(screen.queryByText('**build output**')).not.toBeInTheDocument();
  });

  it('renders nested narration bullets with indentation preserved as nested lists', () => {
    renderMessageList([{
      id: 'message-8',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'narration',
        id: 'narration-6',
        title: 'Reviewing results',
        stage: 'verification',
        message: [
          'I checked the latest validation output.',
          '',
          '- verify failures',
          '  - inspect cargo test log',
          '    - capture failing crate',
          '- rerun tests',
        ].join('\n'),
        evidence: [],
      }],
    }]);

    const narration = screen.getByText('capture failing crate').closest('.agent-narration-text');
    expect(narration).not.toBeNull();

    const topLevelItem = within(narration as HTMLElement).getByText('verify failures').closest('li');
    const nestedItem = within(topLevelItem as HTMLElement).getByText('inspect cargo test log').closest('li');
    const deeplyNestedItem = within(nestedItem as HTMLElement).getByText('capture failing crate').closest('li');

    expect(topLevelItem).not.toBeNull();
    expect(nestedItem).not.toBeNull();
    expect(deeplyNestedItem).not.toBeNull();
    expect(nestedItem?.parentElement?.tagName).toBe('UL');
    expect(nestedItem?.parentElement?.parentElement?.tagName).toBe('LI');
    expect(deeplyNestedItem?.parentElement?.tagName).toBe('UL');
  });

  it('preserves paragraph line breaks and list continuation lines in markdown output', () => {
    const html = parseMarkdown([
      'First line of the walkthrough',
      'Second line that should stay on its own line',
      '',
      '- parent bullet',
      '  continuation detail',
      '  - nested bullet',
    ].join('\n'));

    expect(html).toContain('<p>First line of the walkthrough<br />Second line that should stay on its own line</p>');
    expect(html).toContain('parent bullet<br />continuation detail<ul><li>nested bullet</li></ul>');
  });

  it('preserves markdown in structured task-management narrations assembled from fields', () => {
    renderMessageList([{
      id: 'message-9',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'narration',
        id: 'narration-7',
        title: 'Reviewing task updates',
        stage: 'progress',
        message: '',
        summary: 'I updated **task bookkeeping** after checking the latest results.',
        reason: 'That matters because the current task still needs `cargo test` confirmation.',
        nextStep: 'Next I will:\n- rerun the focused test\n- update the task status if it passes',
        evidence: ['Current step: `Run targeted verification`.', 'Still need to verify: nested task cleanup.'],
      }],
    }]);

    const narration = screen.getByText('task bookkeeping').closest('.agent-narration-text');

    expect(narration).not.toBeNull();
    expect(within(narration as HTMLElement).getByText('task bookkeeping').tagName).toBe('STRONG');
    expect(within(narration as HTMLElement).getAllByText('cargo test')[0].tagName).toBe('CODE');
    expect(within(narration as HTMLElement).getAllByText('Run targeted verification')[0].tagName).toBe('CODE');
    expect(within(narration as HTMLElement).getByText('rerun the focused test').tagName).toBe('LI');
    expect(within(narration as HTMLElement).getByText('update the task status if it passes').tagName).toBe('LI');
    expect(within(narration as HTMLElement).getByText(/Current step:/i).closest('li')).not.toBeNull();
    expect(screen.queryByText('Evidence')).not.toBeInTheDocument();
    expect(screen.queryByText('Why')).not.toBeInTheDocument();
    expect(screen.queryByText('Next')).not.toBeInTheDocument();
  });
});