import { renderHook, act } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';
import { useTabState } from './useTabState';

// Clear sessionStorage before each test so states don't bleed across tests.
beforeEach(() => {
  sessionStorage.clear();
});

function makeFile(relPath: string) {
  return {
    relPath,
    label: relPath.split('/').pop() ?? relPath,
    content: `// ${relPath}`,
    language: 'typescript',
    kind: 'text' as const,
    viewMode: 'edit' as const,
  };
}

describe('useTabState', () => {
  it('starts with no tabs', () => {
    const { result } = renderHook(() => useTabState());
    expect(result.current.tabs).toHaveLength(0);
    expect(result.current.activeTab).toBeNull();
    expect(result.current.activeTabId).toBeNull();
  });

  it('opens a new tab', () => {
    const { result } = renderHook(() => useTabState());
    act(() => {
      result.current.openTab(makeFile('src/main.ts'));
    });
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.tabs[0].relPath).toBe('src/main.ts');
    expect(result.current.tabs[0].isDirty).toBe(false);
    expect(result.current.activeTabId).toBe(result.current.tabs[0].id);
  });

  it('does not duplicate a tab when opening the same path twice', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('src/lib.ts')); });
    act(() => { result.current.openTab(makeFile('src/lib.ts')); });
    expect(result.current.tabs).toHaveLength(1);
  });

  it('allows the same file to be opened in both edit and preview modes', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('README.md')); });
    act(() => {
      result.current.openTab({
        ...makeFile('README.md'),
        viewMode: 'preview',
        language: 'markdown',
        content: '# README',
      });
    });

    expect(result.current.tabs).toHaveLength(2);
    expect(result.current.tabs.map((tab) => tab.viewMode)).toEqual(['edit', 'preview']);
  });

  it('closes a clean tab and picks a neighbour as active', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('a.ts')); });
    act(() => { result.current.openTab(makeFile('b.ts')); });
    const idA = result.current.tabs[0].id;
    const idB = result.current.tabs[1].id;

    act(() => { result.current.closeTab(idA); });
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.tabs[0].id).toBe(idB);
  });

  it('prevents closing a dirty tab without force', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('dirty.ts')); });
    const id = result.current.tabs[0].id;
    act(() => { result.current.updateTabContent(id, 'changed!'); });
    expect(result.current.tabs[0].isDirty).toBe(true);

    // Close without force — tab should remain open.
    act(() => { result.current.closeTab(id); });
    expect(result.current.tabs).toHaveLength(1);
    expect(result.current.tabs[0].id).toBe(id);
  });

  it('force-closes a dirty tab', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('dirty.ts')); });
    const id = result.current.tabs[0].id;
    act(() => { result.current.updateTabContent(id, 'changed!'); });

    let closed: boolean | undefined;
    act(() => { closed = result.current.closeTab(id, { force: true }); });
    expect(closed).toBe(true);
    expect(result.current.tabs).toHaveLength(0);
  });

  it('marks a tab dirty on content update and clean on markTabClean', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('app.ts')); });
    const id = result.current.tabs[0].id;
    act(() => { result.current.updateTabContent(id, 'new content'); });
    expect(result.current.tabs[0].isDirty).toBe(true);

    act(() => { result.current.markTabClean(id); });
    expect(result.current.tabs[0].isDirty).toBe(false);
  });

  it('toggles diff view on a tab', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('diff.ts')); });
    const id = result.current.tabs[0].id;
    expect(result.current.tabs[0].isDiffView).toBe(false);

    act(() => { result.current.toggleDiffView(id); });
    expect(result.current.tabs[0].isDiffView).toBe(true);

    act(() => { result.current.toggleDiffView(id); });
    expect(result.current.tabs[0].isDiffView).toBe(false);
  });

  it('reorders tabs via drag-and-drop', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('a.ts')); });
    act(() => { result.current.openTab(makeFile('b.ts')); });
    act(() => { result.current.openTab(makeFile('c.ts')); });
    const originalOrder = result.current.tabs.map((t) => t.relPath);

    act(() => { result.current.reorderTabs(0, 2); });
    const newOrder = result.current.tabs.map((t) => t.relPath);
    expect(newOrder[2]).toBe(originalOrder[0]);
  });

  it('updates scroll offset', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('scroll.ts')); });
    const id = result.current.tabs[0].id;
    act(() => { result.current.updateScrollOffset(id, 320); });
    expect(result.current.tabs[0].scrollOffset).toBe(320);
  });

  it('activates an existing tab', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab(makeFile('x.ts')); });
    act(() => { result.current.openTab(makeFile('y.ts')); });
    const idX = result.current.tabs[0].id;

    act(() => { result.current.activateTab(idX); });
    expect(result.current.activeTabId).toBe(idX);
  });

  it('renames all open views for the same file', () => {
    const { result } = renderHook(() => useTabState());
    act(() => { result.current.openTab({ ...makeFile('docs/readme.md'), language: 'markdown' }); });
    act(() => {
      result.current.openTab({
        ...makeFile('docs/readme.md'),
        content: '# docs',
        language: 'markdown',
        viewMode: 'preview',
      });
    });

    const editTabId = result.current.tabs.find((tab) => tab.viewMode === 'edit')?.id;
    expect(editTabId).toBeTruthy();

    act(() => {
      result.current.renameTab(editTabId!, 'guide.md', 'docs/guide.md');
    });

    expect(result.current.tabs.every((tab) => tab.relPath === 'docs/guide.md')).toBe(true);
    expect(result.current.tabs.every((tab) => tab.label === 'guide.md')).toBe(true);
  });
});

