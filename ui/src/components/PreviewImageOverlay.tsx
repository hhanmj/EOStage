import { useState, type ReactNode } from "react";
import type { LatLngBoundsExpression } from "leaflet";

export type PreviewImageRenderInput = {
  url: string;
  bounds: LatLngBoundsExpression;
  opacity: number;
  className: string;
  clipPath: string | null;
  onLoad: () => void;
  onError: () => void;
};

export type PreviewImageOverlayProps = {
  identity: string;
  url: string;
  bounds: LatLngBoundsExpression;
  footprintGeojson?: unknown;
  renderImage: (input: PreviewImageRenderInput) => ReactNode;
};

function mercatorYRad(latDeg: number): number {
  const clamped = Math.max(-85.05112878, Math.min(85.05112878, latDeg));
  const rad = (clamped * Math.PI) / 180;
  return Math.log(Math.tan(Math.PI / 4 + rad / 2));
}

function extractOuterRingLonLat(geometry: unknown): number[][] | null {
  if (!geometry || typeof geometry !== "object") return null;
  const g = geometry as {
    type?: unknown;
    coordinates?: unknown;
    geometry?: unknown;
    features?: unknown;
  };
  if (g.type === "Feature") return extractOuterRingLonLat(g.geometry);
  if (g.type === "FeatureCollection" && Array.isArray(g.features)) {
    return extractOuterRingLonLat(g.features[0]);
  }
  if (g.type === "Polygon" && Array.isArray(g.coordinates) && Array.isArray(g.coordinates[0])) {
    return g.coordinates[0] as number[][];
  }
  if (g.type === "MultiPolygon" && Array.isArray(g.coordinates)) {
    let best: number[][] | null = null;
    for (const poly of g.coordinates) {
      if (Array.isArray(poly) && Array.isArray(poly[0])) {
        const ring = poly[0] as number[][];
        if (!best || ring.length > best.length) best = ring;
      }
    }
    return best;
  }
  return null;
}

/**
 * Compute an exact Web Mercator CSS `polygon(...)` clip-path relative to `bounds`
 * so raster quicklooks never leak rectangular black collars outside the true swath polygon.
 */
export function computeMercatorPolygonClipPath(
  footprintGeojson: unknown,
  bounds: LatLngBoundsExpression,
): string | null {
  const ring = extractOuterRingLonLat(footprintGeojson);
  if (!ring || ring.length < 3 || !Array.isArray(bounds) || bounds.length < 2) return null;
  const sw = bounds[0] as number[];
  const ne = bounds[1] as number[];
  if (!Array.isArray(sw) || !Array.isArray(ne) || sw.length < 2 || ne.length < 2) return null;
  const south = Number(sw[0]);
  const west = Number(sw[1]);
  const north = Number(ne[0]);
  const east = Number(ne[1]);
  const lonSpan = east - west;
  if (!Number.isFinite(lonSpan) || lonSpan <= 1e-9 || !Number.isFinite(north - south) || north <= south) {
    return null;
  }
  const yNorth = mercatorYRad(north);
  const ySouth = mercatorYRad(south);
  const ySpan = yNorth - ySouth;
  if (!Number.isFinite(ySpan) || ySpan <= 1e-12) return null;

  const points: string[] = [];
  for (const coord of ring) {
    if (!Array.isArray(coord) || coord.length < 2) continue;
    const lon = Number(coord[0]);
    const lat = Number(coord[1]);
    if (!Number.isFinite(lon) || !Number.isFinite(lat)) continue;
    const xPct = Math.max(-5, Math.min(105, ((lon - west) / lonSpan) * 100));
    const yPct = Math.max(-5, Math.min(105, ((yNorth - mercatorYRad(lat)) / ySpan) * 100));
    points.push(`${xPct.toFixed(2)}% ${yPct.toFixed(2)}%`);
  }
  if (points.length < 3) return null;
  return `polygon(${points.join(", ")})`;
}

export function applyPreviewPolygonClip(overlayLayer: unknown, clipPath: string | null): void {
  const target = overlayLayer as { getElement?: () => HTMLElement | null } | null;
  const el = target?.getElement?.();
  if (!el) return;
  el.style.clipPath = clipPath || "";
  (el.style as CSSStyleDeclaration & { webkitClipPath?: string }).webkitClipPath = clipPath || "";
}

/**
 * Keeps the browser's not-yet-decoded image placeholder invisible. Chromium
 * otherwise paints a transient black rectangle at the geographic image bounds
 * before Leaflet's image overlay finishes loading. Renders at full opacity (1.0)
 * once loaded and clips to true footprint polygon when available.
 */
export function PreviewImageOverlay({
  identity,
  url,
  bounds,
  footprintGeojson,
  renderImage,
}: PreviewImageOverlayProps) {
  const [loadedIdentity, setLoadedIdentity] = useState<string | null>(null);
  const ready = loadedIdentity === identity;
  const clipPath = computeMercatorPolygonClipPath(footprintGeojson, bounds);
  return renderImage({
    url,
    bounds,
    opacity: ready ? 1 : 0,
    className: "catalog-preview-image",
    clipPath,
    onLoad: () => setLoadedIdentity(identity),
    onError: () => setLoadedIdentity((current) => current === identity ? null : current),
  });
}
