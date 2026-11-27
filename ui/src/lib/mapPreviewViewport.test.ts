import assert from "node:assert/strict";
import test from "node:test";

import {
  clampPreviewZoom,
  fitPreviewViewport,
  panPreview,
  PREVIEW_MAX_ZOOM,
  PREVIEW_MIN_ZOOM,
  PREVIEW_WHEEL_ZOOM_FACTOR,
  resetPreviewViewport,
  zoomPreviewAroundPoint,
} from "./mapPreviewViewport.ts";

test("zooming around the pointer keeps the pointed image coordinate stationary", () => {
  assert.deepEqual(
    zoomPreviewAroundPoint(1, { x: 10, y: 20 }, 2, { x: 100, y: 80 }),
    { zoom: 2, offset: { x: -80, y: -40 } },
  );
});

test("preview zoom is clamped to the shared viewport bounds", () => {
  assert.equal(clampPreviewZoom(0.01), PREVIEW_MIN_ZOOM);
  assert.equal(clampPreviewZoom(99), PREVIEW_MAX_ZOOM);
  assert.equal(clampPreviewZoom(Number.NaN), 1);
});

test("preview pan and reset return immutable viewport values", () => {
  const offset = { x: 4, y: -2 };
  assert.deepEqual(panPreview(offset, { x: 12, y: 8 }), { x: 16, y: 6 });
  assert.deepEqual(offset, { x: 4, y: -2 });
  assert.deepEqual(resetPreviewViewport(), { zoom: 1, offset: { x: 0, y: 0 } });
});

test("fit uses the measured image and viewport sizes and preserves the wheel anchor", () => {
  const fit = fitPreviewViewport(
    { width: 4000, height: 2000 },
    { width: 400, height: 300 },
  );
  assert.deepEqual(fit, { zoom: 0.1, offset: { x: 0, y: 50 } });

  const imagePoint = { x: 1200, y: 600 };
  const anchor = {
    x: fit.offset.x + imagePoint.x * fit.zoom,
    y: fit.offset.y + imagePoint.y * fit.zoom,
  };
  const zoomed = zoomPreviewAroundPoint(
    fit.zoom,
    fit.offset,
    fit.zoom * PREVIEW_WHEEL_ZOOM_FACTOR,
    anchor,
    fit.zoom,
  );
  const zoomedAnchor = {
    x: zoomed.offset.x + imagePoint.x * zoomed.zoom,
    y: zoomed.offset.y + imagePoint.y * zoomed.zoom,
  };
  assert.ok(Math.abs(zoomed.zoom - 0.11) < 1e-12);
  assert.ok(Math.abs(zoomedAnchor.x - anchor.x) < 1e-9);
  assert.ok(Math.abs(zoomedAnchor.y - anchor.y) < 1e-9);
});
