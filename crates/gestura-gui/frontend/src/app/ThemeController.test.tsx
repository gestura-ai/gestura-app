import { render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import ThemeController from './ThemeController';

describe('ThemeController', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.documentElement.removeAttribute('data-theme');
    document.documentElement.style.removeProperty('--accent');
    document.documentElement.style.removeProperty('--accent-contrast');
    document.documentElement.style.removeProperty('--accent-rgb');
  });

  it('falls back to legacy MediaQueryList listeners without crashing', () => {
    const addListener = vi.fn();
    const removeListener = vi.fn();

    Object.defineProperty(window, 'matchMedia', {
      writable: true,
      value: vi.fn().mockReturnValue({
        matches: true,
        addListener,
        removeListener,
      }),
    });

    const { unmount } = render(
      <ThemeController
        uiSettings={{ theme_mode: 'system', accent: 'blue' }}
        onUpdate={() => undefined}
      />
    );

    expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    expect(addListener).toHaveBeenCalledTimes(1);

    unmount();

    expect(removeListener).toHaveBeenCalledTimes(1);
  });
});