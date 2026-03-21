import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';

import { usePanelState } from './usePanelState';

beforeEach(() => {
  window.sessionStorage.clear();
  window.localStorage.clear();
});

describe('usePanelState', () => {
  it('persists shell manager visibility, mode, and height across storage scopes', () => {
    const { result, unmount } = renderHook(() => usePanelState());

    act(() => {
      result.current.toggleShellManager();
      result.current.setShellManagerMode('expanded');
      result.current.setShellManagerHeight(320);
    });

    expect(result.current.shellManager.visible).toBe(true);
    expect(result.current.shellManager.mode).toBe('expanded');
    expect(result.current.shellManager.height).toBe(320);

    unmount();

    const { result: restored } = renderHook(() => usePanelState());
    expect(restored.current.shellManager.visible).toBe(true);
    expect(restored.current.shellManager.mode).toBe('expanded');
    expect(restored.current.shellManager.height).toBe(320);
  });

  it('syncs, reorders, and closes shell tabs without reopening closed tabs', () => {
    const { result } = renderHook(() => usePanelState());

    act(() => {
      result.current.syncShellTabs(['shell-a', 'shell-b', 'shell-c'], 'shell-b');
    });

    expect(result.current.shellManager.tabOrder).toEqual(['shell-a', 'shell-b', 'shell-c']);
    expect(result.current.shellManager.activeShellId).toBe('shell-b');

    act(() => {
      result.current.reorderShellTabs('shell-c', 'shell-a');
    });

    expect(result.current.shellManager.tabOrder).toEqual(['shell-c', 'shell-a', 'shell-b']);

    act(() => {
      result.current.closeShellTab('shell-a');
      result.current.syncShellTabs(['shell-a', 'shell-b', 'shell-c']);
    });

    expect(result.current.shellManager.visible).toBe(false);
    expect(result.current.shellManager.tabOrder).toEqual(['shell-c', 'shell-b']);
    expect(result.current.shellManager.closedShellIds).toContain('shell-a');
  });
});