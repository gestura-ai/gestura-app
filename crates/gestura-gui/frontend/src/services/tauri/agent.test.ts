import { describe, expect, it, vi } from 'vitest';

vi.mock('./invoke', () => ({
  invokeTauri: vi.fn(),
}));

import { invokeTauri } from './invoke';
import { enhancePrompt } from './agent';

describe('agent IPC wrappers', () => {
  it('enhancePrompt sends prompt text using the Rust command argument names', async () => {
    const mock = vi.mocked(invokeTauri);
    mock.mockResolvedValueOnce('enhanced text');

    await expect(enhancePrompt('session-123', 'draft prompt')).resolves.toBe('enhanced text');

    expect(mock).toHaveBeenCalledWith('enhance_prompt', {
      session_id: 'session-123',
      prompt: 'draft prompt',
    });
  });
});