import { describe, expect, it } from 'vitest';

import { calculateVisibleQuickAccessExtras } from './quickAccessLayout';

const baseLayout = {
  brandWidth: 180,
  primaryActionWidth: 30,
  rightWidth: 280,
  extraActionCount: 3,
  extraActionWidth: 30,
  leftGap: 12,
  actionGap: 4,
  barGap: 12,
  safetyBuffer: 12,
};

describe('calculateVisibleQuickAccessExtras', () => {
  it('keeps all extra actions visible when there is enough room', () => {
    expect(calculateVisibleQuickAccessExtras({ ...baseLayout, containerWidth: 700 })).toBe(3);
  });

  it('collapses extra actions one at a time as space gets tighter', () => {
    expect(calculateVisibleQuickAccessExtras({ ...baseLayout, containerWidth: 626 })).toBe(2);
    expect(calculateVisibleQuickAccessExtras({ ...baseLayout, containerWidth: 592 })).toBe(1);
    expect(calculateVisibleQuickAccessExtras({ ...baseLayout, containerWidth: 558 })).toBe(0);
  });

  it('never returns more actions than exist', () => {
    expect(calculateVisibleQuickAccessExtras({ ...baseLayout, containerWidth: 1200 })).toBe(3);
  });
});