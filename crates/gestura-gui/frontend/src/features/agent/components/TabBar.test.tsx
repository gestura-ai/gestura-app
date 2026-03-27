import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { TabBar } from './TabBar';

function makeTab(overrides: Partial<Parameters<typeof TabBar>[0]['tabs'][number]> = {}) {
  return {
    id: 'tab-1',
    relPath: 'README.md',
    label: 'README.md',
    content: '# Hello',
    isDirty: false,
    scrollOffset: 0,
    viewMode: 'edit' as const,
    isDiffView: false,
    language: 'markdown',
    kind: 'text' as const,
    ...overrides,
  };
}

describe('TabBar', () => {
  afterEach(() => {
    cleanup();
  });

  it('offers rendered view for editable markdown tabs', () => {
    const onOpenRenderedView = vi.fn();

    render(
      <TabBar
        tabs={[makeTab()]}
        activeTabId="tab-1"
        onActivate={vi.fn()}
        onClose={vi.fn(() => true)}
        onReorder={vi.fn()}
        onRenameTab={vi.fn().mockResolvedValue(undefined)}
        onOpenRenderedView={onOpenRenderedView}
      />,
    );

    fireEvent.contextMenu(screen.getByRole('tab'));
    fireEvent.click(screen.getByRole('button', { name: /Rendered View/i }));

    expect(onOpenRenderedView).toHaveBeenCalledWith('tab-1');
  });

  it('does not offer rendered view for preview tabs', () => {
    const { container } = render(
      <TabBar
        tabs={[makeTab({ id: 'tab-preview', viewMode: 'preview' })]}
        activeTabId="tab-preview"
        onActivate={vi.fn()}
        onClose={vi.fn(() => true)}
        onReorder={vi.fn()}
        onRenameTab={vi.fn().mockResolvedValue(undefined)}
        onOpenRenderedView={vi.fn()}
      />,
    );

    const tablist = container.querySelector('[role="tablist"]');
    expect(tablist).not.toBeNull();

    fireEvent.contextMenu(within(tablist as HTMLElement).getByRole('tab'));

    expect(screen.queryByRole('button', { name: /Rendered View/i })).not.toBeInTheDocument();
  });

  it('saves a dirty tab before closing when Save is chosen', async () => {
    const onClose = vi.fn(() => true);
    const onSaveTab = vi.fn().mockResolvedValue(true);

    render(
      <TabBar
        tabs={[makeTab({ isDirty: true })]}
        activeTabId="tab-1"
        onActivate={vi.fn()}
        onClose={onClose}
        onReorder={vi.fn()}
        onSaveTab={onSaveTab}
        onRenameTab={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Close README.md' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => {
      expect(onSaveTab).toHaveBeenCalledWith('tab-1');
      expect(onClose).toHaveBeenCalledWith('tab-1', { force: true });
    });

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('discards a dirty tab without saving when Discard is chosen', async () => {
    const onClose = vi.fn(() => true);
    const onSaveTab = vi.fn().mockResolvedValue(true);

    render(
      <TabBar
        tabs={[makeTab({ isDirty: true })]}
        activeTabId="tab-1"
        onActivate={vi.fn()}
        onClose={onClose}
        onReorder={vi.fn()}
        onSaveTab={onSaveTab}
        onRenameTab={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Close README.md' }));
    fireEvent.click(screen.getByRole('button', { name: 'Discard' }));

    await waitFor(() => {
      expect(onClose).toHaveBeenCalledWith('tab-1', { force: true });
    });
    expect(onSaveTab).not.toHaveBeenCalled();
  });

  it('dismisses the dirty-tab dialog on Escape', () => {
    const onClose = vi.fn(() => true);

    render(
      <TabBar
        tabs={[makeTab({ isDirty: true })]}
        activeTabId="tab-1"
        onActivate={vi.fn()}
        onClose={onClose}
        onReorder={vi.fn()}
        onSaveTab={vi.fn().mockResolvedValue(true)}
        onRenameTab={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Close README.md' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    fireEvent.keyDown(document, { key: 'Escape' });

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });
});