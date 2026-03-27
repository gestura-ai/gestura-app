import { describe, expect, it, vi } from 'vitest';

import { invokeTauri, TauriInvokeError } from './invoke';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

import { invoke as tauriInvoke } from '@tauri-apps/api/core';

describe('invokeTauri', () => {
  it('calls tauri invoke and returns the result', async () => {
    const invokeMock = vi.mocked(tauriInvoke);
    invokeMock.mockResolvedValueOnce({ ok: true });

    await expect(invokeTauri<{ ok: boolean }>('some_command', { a: 1 })).resolves.toEqual({ ok: true });
    expect(invokeMock).toHaveBeenCalledWith('some_command', { a: 1 });
  });

  it('throws TauriInvokeError and omits args from logs', async () => {
    const invokeMock = vi.mocked(tauriInvoke);
    const consoleSpy = vi.spyOn(console, 'error').mockImplementation(() => { });

    invokeMock.mockRejectedValueOnce(new Error('boom'));
    const secretArgs = { token: 'SUPER_SECRET' };

    const result = invokeTauri('bad_command', secretArgs);
    await expect(result).rejects.toBeInstanceOf(TauriInvokeError);
    await expect(result).rejects.toMatchObject({ command: 'bad_command', message: 'boom' });

    expect(consoleSpy).toHaveBeenCalled();
    const logged = consoleSpy.mock.calls.map((c) => c.join(' ')).join('\n');
    expect(logged).toContain('bad_command');
    expect(logged).toContain('boom');
    expect(logged).not.toContain('SUPER_SECRET');

    consoleSpy.mockRestore();
  });

  it('normalizes non-Error rejections into a message', async () => {
    const invokeMock = vi.mocked(tauriInvoke);
    vi.spyOn(console, 'error').mockImplementation(() => { });

    invokeMock.mockRejectedValueOnce({ code: 123, reason: 'nope' });
    const promise = invokeTauri('obj_error');
    await expect(promise).rejects.toBeInstanceOf(TauriInvokeError);

    try {
      await promise;
    } catch (err) {
      const e = err as TauriInvokeError;
      expect(e.command).toBe('obj_error');
      expect(e.message).toBe('nope');
    }
  });

  it('falls back to a non-empty message when tauri rejects with a blank Error', async () => {
    const invokeMock = vi.mocked(tauriInvoke);
    vi.spyOn(console, 'error').mockImplementation(() => { });

    invokeMock.mockRejectedValueOnce(new Error(''));

    await expect(invokeTauri('blank_error')).rejects.toMatchObject({
      command: 'blank_error',
      message: 'Error',
    });
  });
});
