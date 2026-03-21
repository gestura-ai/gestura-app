import { describe, expect, it } from 'vitest';

import { buildToolPresentation } from './toolActivity';

describe('buildToolPresentation', () => {
  it('turns file operations into reflective summaries with structured parameters', () => {
    const presentation = buildToolPresentation({
      kind: 'tool',
      id: 'tool-1',
      name: 'file',
      args: JSON.stringify({
        operation: 'write',
        path: 'src/features/agent/MessageList.tsx',
        content: 'x'.repeat(80),
      }),
      status: 'success',
      result: 'Wrote src/features/agent/MessageList.tsx',
      durationMs: 21,
      collapsed: true,
    });

    expect(presentation.title).toBe('Updating file');
    expect(presentation.detail).toBe('src/features/agent/MessageList.tsx');
    expect(presentation.parameterItems).toEqual(expect.arrayContaining([
      { label: 'path', value: 'src/features/agent/MessageList.tsx' },
      { label: 'content', value: '80 chars' },
    ]));
    expect(presentation.responseSummary).toBe('Wrote src/features/agent/MessageList.tsx');
  });

  it('summarizes structured responses instead of surfacing raw payloads', () => {
    const presentation = buildToolPresentation({
      kind: 'tool',
      id: 'tool-2',
      name: 'web_search',
      args: JSON.stringify({ query: 'gestura shell manager design' }),
      status: 'success',
      result: JSON.stringify({ summary: 'Found 3 relevant results', results: [{ title: 'A' }, { title: 'B' }, { title: 'C' }] }),
      durationMs: 55,
      collapsed: true,
    });

    expect(presentation.title).toBe('Searching the web');
    expect(presentation.detail).toBe('gestura shell manager design');
    expect(presentation.responseSummary).toBe('Found 3 relevant results');
    expect(presentation.responseItems).toEqual(expect.arrayContaining([
      { label: 'summary', value: 'Found 3 relevant results' },
      { label: 'results', value: '3 items' },
    ]));
  });
});