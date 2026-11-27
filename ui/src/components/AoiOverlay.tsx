import { Fragment, type ReactNode } from "react";
import type { PathOptions } from "leaflet";

import type { Bbox, Json } from "@/lib/bridge";

export type AoiOverlayPart = {
  key: string;
  className: "aoi-overlay";
  pathOptions: PathOptions;
  positions?: [number, number][][];
  bounds?: [[number, number], [number, number]];
};

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function ring(value: unknown): [number, number][] | null {
  if (!Array.isArray(value)) return null;
  const positions = value.flatMap((position) => (
    Array.isArray(position)
    && typeof position[0] === "number"
    && typeof position[1] === "number"
    && Number.isFinite(position[0])
    && Number.isFinite(position[1])
      ? [[position[1], position[0]] as [number, number]]
      : []
  ));
  return positions.length >= 3 ? positions : null;
}

function polygons(value: unknown): [number, number][][][] {
  const geometry = record(value);
  if (!geometry) return [];
  if (geometry.type === "Polygon" && Array.isArray(geometry.coordinates)) {
    const result = geometry.coordinates.map(ring).filter((item): item is [number, number][] => item !== null);
    return result.length ? [result] : [];
  }
  if (geometry.type === "MultiPolygon" && Array.isArray(geometry.coordinates)) {
    return geometry.coordinates.flatMap((polygon) => polygons({ type: "Polygon", coordinates: polygon }));
  }
  if (geometry.type === "Feature") return polygons(geometry.geometry);
  if (geometry.type === "FeatureCollection" && Array.isArray(geometry.features)) {
    return geometry.features.flatMap(polygons);
  }
  return [];
}

function validBbox(bbox: Bbox | null): bbox is Bbox {
  return Boolean(bbox && bbox.west < bbox.east && bbox.south < bbox.north);
}

/** Production AOI primitive boundary; deliberately never emits scene-footprint markers. */
export function AoiOverlay({
  geometry,
  renderGeometry,
  bbox,
  variant = "normal",
  suppressInteriorFill = false,
  renderPolygon,
  renderRectangle,
}: {
  geometry: Json | null | undefined;
  /** Optional render-only projection; `geometry` remains the authoritative value. */
  renderGeometry?: Json | null;
  bbox: Bbox | null | undefined;
  variant?: "normal" | "smoke";
  /** When true (raster/tile preview overlay active), drop yellow interior wash so imagery stays 100% unoccluded. */
  suppressInteriorFill?: boolean;
  renderPolygon: (part: AoiOverlayPart) => ReactNode;
  renderRectangle: (part: AoiOverlayPart) => ReactNode;
}) {
  // `null` is an intentional fail-closed projection result. Only an omitted
  // renderGeometry prop asks for the authoritative geometry fallback.
  const geometryPolygons = polygons(renderGeometry === undefined ? geometry : renderGeometry);
  const polygonOptions: PathOptions = {
    color: "#eab308",
    weight: 3,
    fill: !suppressInteriorFill,
    fillColor: "#facc15",
    fillOpacity: suppressInteriorFill ? 0 : 0.22,
    className: "aoi-overlay",
  };
  const rectangleOptions: PathOptions = variant === "smoke"
    ? {
        color: "#eab308",
        weight: 2.5,
        fill: !suppressInteriorFill,
        fillColor: "#facc15",
        fillOpacity: suppressInteriorFill ? 0 : 0.18,
        className: "aoi-overlay",
      }
    : {
        color: "#eab308",
        weight: 2.5,
        fill: !suppressInteriorFill,
        fillColor: "#facc15",
        fillOpacity: suppressInteriorFill ? 0 : 0.2,
        className: "aoi-overlay",
      };
  if (geometryPolygons.length) {
    return (
      <>
        {geometryPolygons.map((positions, index) => (
          <Fragment key={`aoi:${index}`}>
            {renderPolygon({ key: `aoi:${index}`, className: "aoi-overlay", pathOptions: polygonOptions, positions })}
          </Fragment>
        ))}
      </>
    );
  }
  const aoiBbox = bbox ?? null;
  if (!validBbox(aoiBbox)) return null;
  return (
    <>{renderRectangle({
      key: "aoi:bbox",
      className: "aoi-overlay",
      pathOptions: rectangleOptions,
      bounds: [[aoiBbox.south, aoiBbox.west], [aoiBbox.north, aoiBbox.east]],
    })}</>
  );
}
