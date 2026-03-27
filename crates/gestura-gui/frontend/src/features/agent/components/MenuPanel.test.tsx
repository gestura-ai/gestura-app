import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { MenuPanel } from './MenuPanel';

afterEach(() => {
  cleanup();
});

describe('MenuPanel', () => {
  it('uses the tools icon and routes clicks to the tools panel', () => {
    const onClose = vi.fn();
    const onNavigate = vi.fn();
    const onExportSession = vi.fn();

    render(
      <MenuPanel
        isOpen
        onClose={onClose}
        onNavigate={onNavigate}
        onExportSession={onExportSession}
      />,
    );

    const toolsLabel = screen.getByText('Tools');
    const toolsItem = toolsLabel.closest('.menu-item');

    expect(toolsItem).not.toBeNull();
    expect(toolsItem?.querySelector('.icon-tools')).not.toBeNull();

    fireEvent.click(toolsItem!);

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith('tools');
    expect(onExportSession).not.toHaveBeenCalled();
  });

  it('renders providers above tools in the menu order', () => {
    const onClose = vi.fn();
    const onNavigate = vi.fn();
    const onExportSession = vi.fn();

    const { container } = render(
      <MenuPanel
        isOpen
        onClose={onClose}
        onNavigate={onNavigate}
        onExportSession={onExportSession}
      />,
    );

    const labels = Array.from(container.querySelectorAll('.menu-item-label')).map((node) => node.textContent);

    expect(labels.indexOf('Providers')).toBeGreaterThan(-1);
    expect(labels.indexOf('Tools')).toBeGreaterThan(-1);
    expect(labels.indexOf('Providers')).toBeLessThan(labels.indexOf('Tools'));
  });

  it('uses the brain icon for memory and routes clicks to the memory panel', () => {
    const onClose = vi.fn();
    const onNavigate = vi.fn();
    const onExportSession = vi.fn();

    render(
      <MenuPanel
        isOpen
        onClose={onClose}
        onNavigate={onNavigate}
        onExportSession={onExportSession}
      />,
    );

    const memoryLabel = screen.getByText('Memory');
    const memoryItem = memoryLabel.closest('.menu-item');

    expect(memoryItem).not.toBeNull();
    expect(memoryItem?.querySelector('.icon-brain')).not.toBeNull();

    fireEvent.click(memoryItem!);

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith('memory');
    expect(onExportSession).not.toHaveBeenCalled();
  });

  it('exports the session JSON from the menu', () => {
    const onClose = vi.fn();
    const onNavigate = vi.fn();
    const onExportSession = vi.fn();

    render(
      <MenuPanel
        isOpen
        onClose={onClose}
        onNavigate={onNavigate}
        onExportSession={onExportSession}
      />,
    );

    const exportLabel = screen.getByText('Export Session JSON');
    const exportItem = exportLabel.closest('.menu-item');

    expect(exportItem).not.toBeNull();
    expect(exportItem?.querySelector('.icon-download-01')).not.toBeNull();

    fireEvent.click(exportItem!);

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onExportSession).toHaveBeenCalledTimes(1);
    expect(onNavigate).not.toHaveBeenCalled();
  });
});