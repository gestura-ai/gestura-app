import { renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { useLocalStorageFlag } from './useLocalStorageFlag';

describe('useLocalStorageFlag', () => {
  it('reads initial state from localStorage (key presence)', () => {
    const key = 'test-flag';
    window.localStorage.removeItem(key);

    const { result } = renderHook(() => useLocalStorageFlag(key));
    expect(result.current[0]).toBe(false);

    window.localStorage.setItem(key, 'true');
    const { result: result2 } = renderHook(() => useLocalStorageFlag(key));
    expect(result2.current[0]).toBe(true);
  });

  it('setter updates state and localStorage', () => {
    const key = 'test-flag-2';
    window.localStorage.removeItem(key);

    const { result } = renderHook(() => useLocalStorageFlag(key));
    const [, set] = result.current;

    set(true);
    expect(window.localStorage.getItem(key)).toBe('true');

    set(false);
    expect(window.localStorage.getItem(key)).toBeNull();
  });
});
