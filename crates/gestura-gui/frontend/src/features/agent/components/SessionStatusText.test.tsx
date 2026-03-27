import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { SessionStatusText } from './SessionStatusText';

describe('SessionStatusText', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders cli-style animated text for standard session statuses', () => {
    render(<SessionStatusText status={{ text: 'Thinking…', kind: 'busy' }} />);

    const thinkingWord = screen.getByText('Thinking');
    expect(thinkingWord.closest('.session-status-text')).toHaveClass('session-status-text--normal');
    expect(thinkingWord.closest('.session-status-text')).toHaveClass('session-status-text--animated');

    act(() => {
      vi.advanceTimersByTime(2600);
    });

    expect(screen.getByText('Pondering')).toBeInTheDocument();
  });

  it('uses the dedicated ready-state animated styling for ready statuses', () => {
    render(<SessionStatusText status={{ text: 'Ready', kind: 'ready' }} />);

    const readyText = screen.getByText('Ready');
    expect(readyText.closest('.session-status-text')).toHaveClass('session-status-text--ready');
    expect(readyText.closest('.session-status-text')).toHaveClass('session-status-text--animated');
    expect(readyText.closest('.session-status-text')?.querySelector('.session-status-text__spinner')).not.toBeInTheDocument();
  });

  it('renders warning styling for interrupted or cautionary statuses', () => {
    render(<SessionStatusText status={{ text: 'Interrupted — resume available', kind: 'ready' }} />);

    const warningText = screen.getByText('Interrupted — resume available');
    expect(warningText.closest('.session-status-text')).toHaveClass('session-status-text--warning');
    expect(warningText.closest('.session-status-text')).not.toHaveClass('session-status-text--animated');
  });

  it('renders alert styling for error statuses', () => {
    render(<SessionStatusText status={{ text: 'Error: invoke failed', kind: 'error' }} />);

    const alertText = screen.getByText('Error: invoke failed');
    expect(alertText.closest('.session-status-text')).toHaveClass('session-status-text--alert');
    expect(alertText.closest('.session-status-text')).not.toHaveClass('session-status-text--animated');
  });
});