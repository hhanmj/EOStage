export const MAP_TILE_Z_INDEX = {
  base: 0,
  imageAnnotation: 10,
  vectorAnnotation: 20,
  preview: 30,
} as const;

export const MAP_PREVIEW_PANE_Z_INDEX = 620;

export type AnnotationOverlayPreference = {
  image: boolean;
  vector: boolean;
};

export function effectiveAnnotationOverlays(
  preference: AnnotationOverlayPreference,
  baseLayerContainsLabels: boolean,
): AnnotationOverlayPreference {
  if (!baseLayerContainsLabels) return { ...preference };
  return { image: false, vector: false };
}

export type MapSummaryBbox = {
  west: number;
  east: number;
  south: number;
  north: number;
  crs?: string;
};

function isUsableSummaryBbox(value: MapSummaryBbox | null | undefined): value is MapSummaryBbox {
  return Boolean(
    value &&
      Number.isFinite(value.west) &&
      Number.isFinite(value.east) &&
      Number.isFinite(value.south) &&
      Number.isFinite(value.north) &&
      value.west < value.east &&
      value.south < value.north,
  );
}

export function mapSummaryBbox<T extends MapSummaryBbox>(
  mapFocus: T,
  aoiBbox: T | null | undefined,
): T {
  return isUsableSummaryBbox(aoiBbox) ? aoiBbox : mapFocus;
}
