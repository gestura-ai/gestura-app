import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { MenuPanel } from './MenuPanel';

describe('MenuPanel', () => {
  it('uses the brain icon for memory and routes clicks to the memory panel', () => {
    const onClose = vi.fn();
    const onNavigate = vi.fn();

    render(<MenuPanel isOpen onClose={onClose} onNavigate={onNavigate} />);

    const memoryLabel = screen.getByText('Memory');
    const memoryItem = memoryLabel.closest('.menu-item');

    expect(memoryItem).not.toBeNull();
    expect(memoryItem?.querySelector('.icon-brain')).not.toBeNull();

    fireEvent.click(memoryItem!);

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onNavigate).toHaveBeenCalledWith('memory');
  });
});