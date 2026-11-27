export type PreviewViewportOffset = {
  x: number;
  y: number;
};

export type PreviewViewportPoint = {
  x: number;
  y: number;
};

export type PreviewViewportSize = {
  width: number;
  height: number;
};

export type PreviewViewportTransform = {
  zoom: number;
  offset: PreviewViewportOffset;
};

export const PREVIEW_MIN_ZOOM = 0.25;
export const PREVIEW_MAX_ZOOM = 4;
export const PREVIEW_WHEEL_ZOOM_FACTOR = 1.1;

export const ZERO_PREVIEW_VIEWPORT_OFFSET: PreviewViewportOffset = {
  x: 0,
  y: 0,
};

function normalizedMinimumZoom(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return PREVIEW_MIN_ZOOM;
  return Math.min(PREVIEW_MAX_ZOOM, value);
}

export function clampPreviewZoom(
  value: number,
  minimumZoom: number = PREVIEW_MIN_ZOOM,
): number {
  if (!Number.isFinite(value)) return 1;
  const minimum = normalizedMinimumZoom(minimumZoom);
  return Math.min(PREVIEW_MAX_ZOOM, Math.max(minimum, value));
}

export function fitPreviewViewport(
  naturalSize: PreviewViewportSize,
  viewportSize: PreviewViewportSize,
): PreviewViewportTransform {
  const naturalWidth = Number.isFinite(naturalSize.width) && naturalSize.width > 0
    ? naturalSize.width
    : 0;
  const naturalHeight = Number.isFinite(naturalSize.height) && naturalSize.height > 0
    ? naturalSize.height
    : 0;
  const viewportWidth = Number.isFinite(viewportSize.width) && viewportSize.width > 0
    ? viewportSize.width
    : 0;
  const viewportHeight = Number.isFinite(viewportSize.height) && viewportSize.height > 0
    ? viewportSize.height
    : 0;

  if (!naturalWidth || !naturalHeight || !viewportWidth || !viewportHeight) {
    return {
      zoom: 1,
      offset: { ...ZERO_PREVIEW_VIEWPORT_OFFSET },
    };
  }

  const zoom = Math.min(1, viewportWidth / naturalWidth, viewportHeight / naturalHeight);
  return {
    zoom,
    offset: {
      x: (viewportWidth - naturalWidth * zoom) / 2,
      y: (viewportHeight - naturalHeight * zoom) / 2,
    },
  };
}

export function panPreview(
  offset: PreviewViewportOffset,
  delta: PreviewViewportPoint,
): PreviewViewportOffset {
  return {
    x: offset.x + (Number.isFinite(delta.x) ? delta.x : 0),
    y: offset.y + (Number.isFinite(delta.y) ? delta.y : 0),
  };
}

export function zoomPreviewAroundPoint(
  currentZoom: number,
  currentOffset: PreviewViewportOffset,
  requestedZoom: number,
  anchor: PreviewViewportPoint,
  minimumZoom: number = PREVIEW_MIN_ZOOM,
): PreviewViewportTransform {
  const safeCurrentZoom = clampPreviewZoom(currentZoom, minimumZoom);
  const zoom = clampPreviewZoom(requestedZoom, minimumZoom);
  const ratio = zoom / safeCurrentZoom;
  const safeAnchor = {
    x: Number.isFinite(anchor.x) ? anchor.x : 0,
    y: Number.isFinite(anchor.y) ? anchor.y : 0,
  };
  const safeOffset = {
    x: Number.isFinite(currentOffset.x) ? currentOffset.x : 0,
    y: Number.isFinite(currentOffset.y) ? currentOffset.y : 0,
  };
  return {
    zoom,
    offset: {
      x: safeAnchor.x - (safeAnchor.x - safeOffset.x) * ratio,
      y: safeAnchor.y - (safeAnchor.y - safeOffset.y) * ratio,
    },
  };
}

export function resetPreviewViewport(): {
  zoom: number;
  offset: PreviewViewportOffset;
} {
  return {
    zoom: 1,
    offset: { ...ZERO_PREVIEW_VIEWPORT_OFFSET },
  };
}
