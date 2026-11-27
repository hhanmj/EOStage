export type PixelPoint = { x: number; y: number };
export type PixelSize = { width: number; height: number };

function finiteOrZero(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), Math.max(minimum, maximum));
}

/** Places a pointer tooltip near its anchor while keeping its box in the map viewport. */
export function constrainMapTooltipPosition(
  anchor: PixelPoint,
  viewport: PixelSize,
  tooltip: PixelSize,
  inset = 8,
): PixelPoint {
  const safeInset = Math.max(0, finiteOrZero(inset));
  const viewportWidth = Math.max(0, finiteOrZero(viewport.width));
  const viewportHeight = Math.max(0, finiteOrZero(viewport.height));
  const tooltipWidth = Math.max(0, finiteOrZero(tooltip.width));
  const tooltipHeight = Math.max(0, finiteOrZero(tooltip.height));
  const gap = 12;
  const preferredX = finiteOrZero(anchor.x) + gap;
  const preferredY = finiteOrZero(anchor.y) - tooltipHeight - gap;
  return {
    x: clamp(preferredX, safeInset, viewportWidth - tooltipWidth - safeInset),
    y: clamp(preferredY, safeInset, viewportHeight - tooltipHeight - safeInset),
  };
}
