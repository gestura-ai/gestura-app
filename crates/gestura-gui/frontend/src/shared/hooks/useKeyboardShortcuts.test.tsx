import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { useKeyboardShortcuts } from './useKeyboardShortcuts';

describe('useKeyboardShortcuts', () => {
  it('attaches keydown listener and calls the latest handler', () => {
    const h1 = vi.fn();
    const { rerender, unmount } = renderHook(({ handler }) => useKeyboardShortcuts(handler), {
      initialProps: { handler: h1 },
    });

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(h1).toHaveBeenCalledTimes(1);

    const h2 = vi.fn();
    rerender({ handler: h2 });
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));

    expect(h1).toHaveBeenCalledTimes(1);
    expect(h2).toHaveBeenCalledTimes(1);

    unmount();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k' }));
    expect(h2).toHaveBeenCalledTimes(1);
  });

  it('respects enabled=false', () => {
    const handler = vi.fn();
    renderHook(() => useKeyboardShortcuts(handler, { enabled: false }));
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'x' }));
    expect(handler).not.toHaveBeenCalled();
  });
});
