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

    expect(presentation.title).toBe('Researching gestura shell manager');
    expect(presentation.detail).toBe('gestura shell manager design');
    expect(presentation.responseSummary).toBe('Found 3 relevant results');
    expect(presentation.responseItems).toEqual(expect.arrayContaining([
      { label: 'summary', value: 'Found 3 relevant results' },
      { label: 'results', value: '3 items' },
    ]));
  });

  it('renders task status validation errors as actionable summaries instead of raw char counts', () => {
    const presentation = buildToolPresentation({
      kind: 'tool',
      id: 'tool-3',
      name: 'task',
      args: JSON.stringify({
        operation: 'update_status',
        task_id: '67f4aef5-1c1d-43c5-837e-e46d0cd98557',
      }),
      status: 'error',
      result: `Missing required field 'status' for update_status operation. \`update_status\` requires both \`task_id\` and \`status\`. Provided fields: operation, task_id. Retry with {"operation":"update_status","task_id":"67f4aef5-1c1d-43c5-837e-e46d0cd98557","status":"inprogress"} using one of: \`notstarted\`, \`blocked\`, \`inprogress\`, \`completed\`, or \`cancelled\`. Do not omit \`status\` to ask the runtime to infer or preserve the current state; if no status changed, skip the task update and continue the real work.`,
      durationMs: 7,
      collapsed: true,
    });

    expect(presentation.title).toBe('Updating task status');
    expect(presentation.detail).toBeNull();
    expect(presentation.responseSummary).toBe('Task status update needs an explicit status value.');
    expect(presentation.responseItems).toEqual(expect.arrayContaining([
      { label: 'task id', value: '67f4aef5-1c1d-43c5-837e-e46d0cd98557' },
      { label: 'provided fields', value: 'operation, task_id' },
      { label: 'retry example', value: '{"operation":"update_status","task_id":"67f4aef5-1c1d-43c5-837e-e46d0cd98557","status":"inprogress"}' },
    ]));
  });

  it('uses operation-aware task titles for successful status changes', () => {
    const presentation = buildToolPresentation({
      kind: 'tool',
      id: 'tool-4',
      name: 'task',
      args: JSON.stringify({
        operation: 'update_status',
        task_id: '67f4aef5-1c1d-43c5-837e-e46d0cd98557',
        status: 'completed',
      }),
      status: 'success',
      result: 'Updated task 67f4aef5-1c1d-43c5-837e-e46d0cd98557 status to Completed',
      durationMs: 12,
      collapsed: true,
    });

    expect(presentation.title).toBe('Marking task complete');
    expect(presentation.responseSummary).toBe('Updated task status to Completed.');
    expect(presentation.responseItems).toEqual(expect.arrayContaining([
      { label: 'task id', value: '67f4aef5-1c1d-43c5-837e-e46d0cd98557' },
      { label: 'status', value: 'Completed' },
    ]));
  });
});