import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { useChatSession } from './useChatSession';
import type { StreamEventDispatch } from './useStreamEvents';

let streamDispatch: StreamEventDispatch | null = null;

const getSessionHistoryMock = vi.fn();
const hasSessionPausedExecutionMock = vi.fn();
const getTaskHierarchyMock = vi.fn();
const listKnowledgeItemsMock = vi.fn();
const getEnabledKnowledgeMock = vi.fn();
const getSessionToolSettingsMock = vi.fn();
const listBuiltinToolsMock = vi.fn();
const listDiscoveredMcpToolsMock = vi.fn();
const pauseStreamingMock = vi.fn();
const resumeStreamingMock = vi.fn();

vi.mock('./useStreamEvents', async () => {
  const actual = await vi.importActual<typeof import('./useStreamEvents')>('./useStreamEvents');
  return {
    ...actual,
    useStreamEvents: (_sessionId: string, dispatch: StreamEventDispatch) => {
      streamDispatch = dispatch;
    },
  };
});

vi.mock('../../../services/tauri/agent', () => ({
  sendMessageStreaming: vi.fn(),
  cancelStreaming: vi.fn(),
  pauseStreaming: (...args: unknown[]) => pauseStreamingMock(...args),
  resumeStreaming: (...args: unknown[]) => resumeStreamingMock(...args),
  getSessionHistory: (...args: unknown[]) => getSessionHistoryMock(...args),
  hasSessionPausedExecution: (...args: unknown[]) => hasSessionPausedExecutionMock(...args),
  getTaskHierarchy: (...args: unknown[]) => getTaskHierarchyMock(...args),
  listKnowledgeItems: (...args: unknown[]) => listKnowledgeItemsMock(...args),
  getEnabledKnowledge: (...args: unknown[]) => getEnabledKnowledgeMock(...args),
  getSessionToolSettings: (...args: unknown[]) => getSessionToolSettingsMock(...args),
  listBuiltinTools: (...args: unknown[]) => listBuiltinToolsMock(...args),
  listDiscoveredMcpTools: (...args: unknown[]) => listDiscoveredMcpToolsMock(...args),
  resolveToolConfirmationDecision: vi.fn(),
  enhancePrompt: vi.fn(async (text: string) => text),
  startVoiceListening: vi.fn(),
  stopVoiceListening: vi.fn(),
}));

function blocksToText(blocks: Array<{ kind: string; content?: string }>) {
  return blocks.map((block) => `${block.kind}:${block.content ?? ''}`).join('|');
}

function Harness() {
  const state = useChatSession('session-123');
  return (
    <div>
      <div data-testid="status">{state.status.text}</div>
      <div data-testid="can-resume">{String(state.canResume)}</div>
      <div data-testid="is-processing">{String(state.isProcessing)}</div>
      <div data-testid="is-stopping">{String(state.isStopping)}</div>
      <div data-testid="messages">
        {JSON.stringify(state.messages.map((message) => ({
          role: message.role,
          rawMarkdown: message.rawMarkdown,
          blocks: blocksToText(message.blocks),
        })))}
      </div>
      <div data-testid="streaming">{state.streamingMessage ? blocksToText(state.streamingMessage.blocks) : ''}</div>
      <button type="button" onClick={() => { void state.resumeStream(); }}>
        Resume
      </button>
      <button type="button" onClick={() => { void state.cancelStream(); }}>
        Stop
      </button>
    </div>
  );
}

describe('useChatSession', () => {
  afterEach(() => {
    cleanup();
  });

  beforeEach(() => {
    streamDispatch = null;
    getSessionHistoryMock.mockResolvedValue([]);
    hasSessionPausedExecutionMock.mockResolvedValue(false);
    getTaskHierarchyMock.mockResolvedValue([]);
    listKnowledgeItemsMock.mockResolvedValue([]);
    getEnabledKnowledgeMock.mockResolvedValue([]);
    getSessionToolSettingsMock.mockResolvedValue({ mode: 'allowlist', allowlist: [], requireConfirmation: [] });
    listBuiltinToolsMock.mockResolvedValue([]);
    listDiscoveredMcpToolsMock.mockResolvedValue([]);
    pauseStreamingMock.mockReset();
    resumeStreamingMock.mockReset();
  });

  it('keeps partial assistant content visible when a stream is paused', async () => {
    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'chunk', chunk: 'Partial answer' });
      streamDispatch?.({ type: 'paused' });
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Interrupted — resume available');
    expect(screen.getByTestId('can-resume')).toHaveTextContent('true');
    expect(screen.getByTestId('is-processing')).toHaveTextContent('false');
    expect(screen.getByTestId('streaming')).toHaveTextContent('Partial answer');
  });

  it('marks stop as in progress on the first click and ignores repeated clicks', async () => {
    pauseStreamingMock.mockResolvedValue(undefined);

    render(<Harness />);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Stopping…');
    expect(screen.getByTestId('is-stopping')).toHaveTextContent('true');
    expect(pauseStreamingMock).toHaveBeenCalledTimes(1);

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    });

    expect(pauseStreamingMock).toHaveBeenCalledTimes(1);
  });

  it('resumes into the same assistant message instead of starting a new bubble', async () => {
    resumeStreamingMock.mockImplementation(async () => {
      streamDispatch?.({ type: 'chunk', chunk: ' world' });
      streamDispatch?.({ type: 'done' });
    });

    render(<Harness />);

    await waitFor(() => {
      expect(streamDispatch).not.toBeNull();
    });

    await act(async () => {
      streamDispatch?.({ type: 'chunk', chunk: 'Hello' });
      streamDispatch?.({ type: 'paused' });
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
    });

    await waitFor(() => {
      expect(resumeStreamingMock).toHaveBeenCalledWith('session-123');
    });

    const messages = screen.getByTestId('messages').textContent ?? '';
    expect(messages).toContain('Hello world');
    expect(messages.match(/rawMarkdown/g)).toHaveLength(1);
    expect(screen.getByTestId('can-resume')).toHaveTextContent('false');
  });

  it('rehydrates interrupted thinking-only messages from session history', async () => {
    getSessionHistoryMock.mockResolvedValue([
      { role: 'assistant', content: '', thinking: 'Working through the answer…', timestamp: '2026-03-15T12:00:00.000Z' },
    ]);
    hasSessionPausedExecutionMock.mockResolvedValue(true);

    render(<Harness />);

    await waitFor(() => {
      expect(screen.getByTestId('can-resume')).toHaveTextContent('true');
    });

    expect(screen.getByTestId('status')).toHaveTextContent('Interrupted — resume available');
    expect(screen.getByTestId('messages')).toHaveTextContent('thinking:Working through the answer…');
  });
});