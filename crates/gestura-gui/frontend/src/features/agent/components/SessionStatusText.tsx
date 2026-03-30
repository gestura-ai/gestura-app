import React, { useEffect, useMemo, useState } from 'react';

import type { StatusState } from '../types';

type SessionStatusTone = 'normal' | 'warning' | 'alert';

const SPINNER_TICK_MS = 50;
const TICKS_PER_WORD = 50;
const ROLL_TICKS = 12;
const BRAILLE_FRAMES = ['⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'] as const;
const THINKING_WORDS = ['Thinking', 'Pondering', 'Contemplating', 'Reflecting', 'Reasoning', 'Analyzing', 'Processing', 'Considering'] as const;
const READY_WORDS = ['Ready', 'Standing by', 'Waiting', 'Available'] as const;
const LISTENING_WORDS = ['Listening', 'Capturing', 'Transcribing', 'Parsing'] as const;
const RESUMING_WORDS = ['Resuming', 'Restoring', 'Rehydrating', 'Continuing'] as const;

function isExplicitLoadingStatus(text: string): boolean {
  return /\bloading\b|\bwarming\s+up\b|\bstarting\b/i.test(text);
}

function toneForStatus(status: StatusState): SessionStatusTone {
  const text = status.text.trim();

  if (status.kind === 'error' || /(error|failed|failure|stop failed|resume failed)/i.test(text)) {
    return 'alert';
  }

  if (/(interrupted|retrying|cancelled|canceled|stopping)/i.test(text)) {
    return 'warning';
  }

  return 'normal';
}

function animatedWordsForStatus(status: StatusState, tone: SessionStatusTone): readonly string[] | null {
  const text = status.text.trim();

  if (tone !== 'normal') {
    return null;
  }

  if (isExplicitLoadingStatus(text)) {
    return null;
  }

  if (status.kind === 'listening') {
    return LISTENING_WORDS;
  }

  if (/^ready$/i.test(text)) {
    return READY_WORDS;
  }

  if (/resum/i.test(text)) {
    return RESUMING_WORDS;
  }

  if (status.kind === 'busy' || status.kind === 'reflection' || /thinking|reflect|reason|analyz|process/i.test(text)) {
    return THINKING_WORDS;
  }

  return null;
}

function rollingWord(words: readonly string[], tick: number): string {
  if (words.length === 0) {
    return '';
  }

  if (words.length === 1) {
    return words[0];
  }

  const wordCycle = Math.floor(tick / TICKS_PER_WORD);
  const tickInWord = tick % TICKS_PER_WORD;
  const currentWord = words[wordCycle % words.length];
  const nextWord = words[(wordCycle + 1) % words.length];

  if (tickInWord < TICKS_PER_WORD - ROLL_TICKS) {
    return currentWord;
  }

  const rollProgress = tickInWord - (TICKS_PER_WORD - ROLL_TICKS);
  const currentChars = Array.from(currentWord);
  const nextChars = Array.from(nextWord);
  const maxLen = Math.max(currentChars.length, nextChars.length);
  const charsRolled = maxLen === 0
    ? 0
    : Math.floor(((rollProgress + 1) * maxLen) / ROLL_TICKS);

  let result = '';
  for (let index = 0; index < maxLen; index += 1) {
    result += index < charsRolled
      ? nextChars[index] ?? ' '
      : currentChars[index] ?? ' ';
  }

  return result.trimEnd();
}

export interface SessionStatusTextProps {
  status: StatusState;
}

export const SessionStatusText: React.FC<SessionStatusTextProps> = ({ status }) => {
  const statusText = status.text.trim();
  const tone = toneForStatus(status);
  const loading = isExplicitLoadingStatus(statusText);
  const words = useMemo(() => animatedWordsForStatus(status, tone), [status, tone]);
  const animated = Boolean(words && words.length > 1);
  const ready = tone === 'normal' && /^ready$/i.test(statusText);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (!animated) {
      return;
    }

    const interval = window.setInterval(() => {
      setTick((current) => current + 1);
    }, SPINNER_TICK_MS);

    return () => {
      window.clearInterval(interval);
    };
  }, [animated, status.kind, status.text]);

  const displayWord = words ? rollingWord(words, tick) : status.text;
  const spinnerFrame = (animated || loading) && !ready ? BRAILLE_FRAMES[tick % BRAILLE_FRAMES.length] : null;

  return (
    <div
      className={[
        'session-status-text',
        `session-status-text--${tone}`,
        animated ? 'session-status-text--animated' : '',
        ready ? 'session-status-text--ready' : '',
      ].filter(Boolean).join(' ')}
      title={status.text}
      aria-live="polite"
    >
      {spinnerFrame && (
        <span className="session-status-text__spinner" aria-hidden="true">{spinnerFrame}</span>
      )}
      <span className="session-status-text__word">{displayWord}</span>
    </div>
  );
};

export default SessionStatusText;