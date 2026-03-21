import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { AgentMessage } from '../types';
import { MessageList } from './MessageList';

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (path: string) => `asset://${path}`,
}));

function renderMessageList(messages: AgentMessage[], onRevealShellSession = vi.fn()) {
  render(
    <MessageList
      messages={messages}
      streamingMessage={null}
      onScrollChange={vi.fn()}
      onRevealShellSession={onRevealShellSession}
    />,
  );

  return { onRevealShellSession };
}

describe('MessageList', () => {
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
          lines: [],
          collapsed: true,
        },
      ],
    }], onRevealShellSession);

    expect(screen.queryByText('Running shell command')).not.toBeInTheDocument();
    expect(screen.getByText('cargo test')).toBeInTheDocument();
    expect(screen.queryByText('/workspace')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Copy command' })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /cargo test/i }));
    expect(screen.getByText('/workspace')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Open in Shell Session Manager' }));
    expect(onRevealShellSession).toHaveBeenCalledWith('shell-session-1');
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
      }],
    }]);

    expect(screen.getByText('I’m checking the current files before I make the next change.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Checking current files/i })).not.toBeInTheDocument();
    expect(screen.queryByText(/Working:/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/Checking:/i)).not.toBeInTheDocument();
  });

  it('collapses long narration behind a titled row that expands on demand', () => {
    const longNarration = 'I’m comparing the current implementation with the runtime evidence so I can confirm whether the next step should be a code change, a verification pass, or a task-state update before I move forward with the request. I also want to check whether the tracked task state still matches what the tools have actually proven so far, because that affects whether I should keep inspecting, make an edit, or switch into validation mode next.';

    renderMessageList([{
      id: 'message-4',
      role: 'assistant',
      rawMarkdown: '',
      isStreaming: false,
      timestamp: Date.now(),
      blocks: [{
        kind: 'narration',
        id: 'narration-2',
        title: 'Reviewing implementation state',
        stage: 'verification',
        message: longNarration,
      }],
    }]);

    const toggle = screen.getByRole('button', { name: 'Reviewing implementation state' });
    expect(screen.queryByText(longNarration)).not.toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-expanded', 'false');

    fireEvent.click(toggle);

    expect(screen.getByText(longNarration)).toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
  });
});