import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { useViewMode } from './useViewMode';

describe('useViewMode', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    sessionStorage.clear();
  });

  it('falls back to message-only when sessionStorage read throws', () => {
    const getItemSpy = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('sessionStorage unavailable');
    });

    const { result } = renderHook(() => useViewMode());

    expect(result.current.viewMode).toBe('message-only');
    expect(getItemSpy).toHaveBeenCalled();
  });

  it('updates view mode even when sessionStorage write throws', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('sessionStorage unavailable');
    });

    const { result } = renderHook(() => useViewMode());

    act(() => {
      result.current.toggleViewMode();
    });

    expect(result.current.viewMode).toBe('editor');
  });
});