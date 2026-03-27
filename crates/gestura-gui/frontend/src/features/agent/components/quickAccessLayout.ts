export interface QuickAccessLayoutInput {
  containerWidth: number;
  brandWidth: number;
  primaryActionWidth: number;
  rightWidth: number;
  extraActionCount: number;
  extraActionWidth: number;
  leftGap: number;
  actionGap: number;
  barGap: number;
  safetyBuffer: number;
}

/** Calculates how many extra quick-access actions can remain visible without overlap. */
export function calculateVisibleQuickAccessExtras({
  containerWidth,
  brandWidth,
  primaryActionWidth,
  rightWidth,
  extraActionCount,
  extraActionWidth,
  leftGap,
  actionGap,
  barGap,
  safetyBuffer,
}: QuickAccessLayoutInput): number {
  if (extraActionCount <= 0 || containerWidth <= 0) {
    return 0;
  }

  let leftWidth = brandWidth + primaryActionWidth;
  if (brandWidth > 0 && primaryActionWidth > 0) {
    leftWidth += leftGap;
  }

  let requiredWidth = leftWidth + rightWidth + safetyBuffer;
  if (leftWidth > 0 && rightWidth > 0) {
    requiredWidth += barGap;
  }

  let visibleCount = 0;
  for (let index = 0; index < extraActionCount; index += 1) {
    const hasLeadingAction = primaryActionWidth > 0 || visibleCount > 0;
    const nextWidth = extraActionWidth + (hasLeadingAction ? actionGap : 0);
    if (requiredWidth + nextWidth > containerWidth) {
      break;
    }
    requiredWidth += nextWidth;
    visibleCount += 1;
  }

  return visibleCount;
}