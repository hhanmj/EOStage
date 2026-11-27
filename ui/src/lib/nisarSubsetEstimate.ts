export const NISAR_METADATA_GRID_FLOOR_BYTES = 32 * 1024 * 1024;
export const DEFAULT_NISAR_FULL_SCENE_BYTES = 15_000_000_000;
export const MIN_NISAR_AOI_AREA_RATIO = 0.03;
export const MAX_NISAR_AOI_AREA_RATIO = 1.0;

export interface GeographicBbox {
  west: number;
  south: number;
  east: number;
  north: number;
}

export interface NisarSceneSizeLike {
  file_size_remote?: number | null;
  footprint_bbox?: GeographicBbox | null;
  footprint_geojson?: unknown;
  polarization?: string | null;
}

export interface NisarSubsetEstimate {
  sceneCount: number;
  rawBytes: number;
  estimatedSubsetBytes: number;
  savedBytes: number;
  savingsPercent: number;
  areaRatio: number;
  polarizationScale: number;
  isFallbackEstimate: boolean;
}

export function clampNumber(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, value));
}

const KM_PER_DEG_LAT = 111.32;

export function bboxAreaKm2(bbox: GeographicBbox | null | undefined): number {
  if (!bbox) return 0;
  const latSpan = Math.abs(bbox.north - bbox.south);
  const lonSpan = Math.abs(bbox.east - bbox.west);
  if (!Number.isFinite(latSpan) || !Number.isFinite(lonSpan) || latSpan <= 0 || lonSpan <= 0) {
    return 0;
  }
  const midLatRad = (((bbox.south + bbox.north) * 0.5) * Math.PI) / 180;
  const kmPerDegLon = Math.max(1e-3, KM_PER_DEG_LAT * Math.cos(midLatRad));
  return latSpan * KM_PER_DEG_LAT * lonSpan * kmPerDegLon;
}

export function bboxIntersectionKm2(
  aoi: GeographicBbox | null | undefined,
  scene: GeographicBbox | null | undefined,
): number {
  if (!aoi || !scene) return 0;
  const west = Math.max(aoi.west, scene.west);
  const east = Math.min(aoi.east, scene.east);
  const south = Math.max(aoi.south, scene.south);
  const north = Math.min(aoi.north, scene.north);
  if (east <= west || north <= south) return 0;
  return bboxAreaKm2({ west, south, east, north });
}

function ringAreaKm2(ring: readonly number[][]): number {
  if (ring.length < 3) return 0;
  let sumLat = 0;
  let count = 0;
  for (const pt of ring) {
    if (pt.length >= 2 && Number.isFinite(pt[1])) {
      sumLat += pt[1];
      count += 1;
    }
  }
  if (count === 0) return 0;
  const midLatRad = ((sumLat / count) * Math.PI) / 180;
  const kmPerDegLon = Math.max(1e-3, KM_PER_DEG_LAT * Math.cos(midLatRad));
  let twiceArea = 0;
  for (let i = 0; i < ring.length; i += 1) {
    const p1 = ring[i];
    const p2 = ring[(i + 1) % ring.length];
    const x1 = p1[0] * kmPerDegLon;
    const y1 = p1[1] * KM_PER_DEG_LAT;
    const x2 = p2[0] * kmPerDegLon;
    const y2 = p2[1] * KM_PER_DEG_LAT;
    twiceArea += x1 * y2 - x2 * y1;
  }
  return Math.abs(twiceArea) * 0.5;
}

export function geojsonAreaKm2(geojson: unknown): number {
  if (!geojson || typeof geojson !== "object") return 0;
  const rec = geojson as { type?: string; coordinates?: unknown };
  if (rec.type === "Polygon" && Array.isArray(rec.coordinates) && rec.coordinates.length > 0) {
    const outer = Array.isArray(rec.coordinates[0]) ? (rec.coordinates[0] as number[][]) : [];
    let area = ringAreaKm2(outer);
    for (let i = 1; i < rec.coordinates.length; i += 1) {
      if (Array.isArray(rec.coordinates[i])) {
        area = Math.max(0, area - ringAreaKm2(rec.coordinates[i] as number[][]));
      }
    }
    return area;
  }
  if (rec.type === "MultiPolygon" && Array.isArray(rec.coordinates)) {
    let total = 0;
    for (const poly of rec.coordinates) {
      if (Array.isArray(poly) && poly.length > 0 && Array.isArray(poly[0])) {
        let polyArea = ringAreaKm2(poly[0] as number[][]);
        for (let i = 1; i < poly.length; i += 1) {
          if (Array.isArray(poly[i])) {
            polyArea = Math.max(0, polyArea - ringAreaKm2(poly[i] as number[][]));
          }
        }
        total += polyArea;
      }
    }
    return total;
  }
  return 0;
}

export function bboxFromGeoJson(geojson: unknown): GeographicBbox | null {
  if (!geojson || typeof geojson !== "object") return null;
  const rec = geojson as { type?: string; coordinates?: unknown };
  const rings: number[][][] = [];
  if (rec.type === "Polygon" && Array.isArray(rec.coordinates)) {
    for (const ring of rec.coordinates) {
      if (Array.isArray(ring)) rings.push(ring as number[][]);
    }
  } else if (rec.type === "MultiPolygon" && Array.isArray(rec.coordinates)) {
    for (const poly of rec.coordinates) {
      if (Array.isArray(poly)) {
        for (const ring of poly) {
          if (Array.isArray(ring)) rings.push(ring as number[][]);
        }
      }
    }
  }
  let west = Infinity;
  let south = Infinity;
  let east = -Infinity;
  let north = -Infinity;
  for (const ring of rings) {
    for (const pt of ring) {
      if (pt.length >= 2 && Number.isFinite(pt[0]) && Number.isFinite(pt[1])) {
        west = Math.min(west, pt[0]);
        east = Math.max(east, pt[0]);
        south = Math.min(south, pt[1]);
        north = Math.max(north, pt[1]);
      }
    }
  }
  if (!Number.isFinite(west) || !Number.isFinite(south) || east <= west || north <= south) {
    return null;
  }
  return { west, south, east, north };
}

export function computeNisarPolarizationScale(
  selectedPolarizations: readonly string[],
  availablePolarizations?: readonly string[] | null,
): number {
  const uniqueSelected = new Set(
    selectedPolarizations.map((p) => p.trim().toUpperCase()).filter(Boolean),
  );
  const selectedCount = Math.max(1, uniqueSelected.size);
  const uniqueAvailable = availablePolarizations
    ? new Set(availablePolarizations.map((p) => p.trim().toUpperCase()).filter(Boolean))
    : null;
  const totalCount =
    uniqueAvailable && uniqueAvailable.size > 0
      ? Math.max(uniqueAvailable.size, selectedCount)
      : selectedCount > 2
      ? 4
      : 2;
  return clampNumber(selectedCount / totalCount, 0.25, 1.0);
}

function sceneAvailablePols(scene: NisarSceneSizeLike): string[] | null {
  if (typeof scene.polarization !== "string" || !scene.polarization.trim()) return null;
  const tokens = scene.polarization
    .split(/[\s+/,:;]+/)
    .map((t) => t.trim().toUpperCase())
    .filter((t) => ["HH", "HV", "VV", "VH", "LH", "LV", "RH", "RV"].includes(t));
  return tokens.length > 0 ? tokens : null;
}

export function estimateNisarSceneSubsetBytes(input: {
  rawSceneBytes: number;
  aoiIntersectionKm2: number;
  sceneFootprintKm2: number;
  selectedPolarizations: readonly string[];
  availablePolarizations?: readonly string[] | null;
}): { estimatedBytes: number; areaRatio: number; polarizationScale: number } {
  const rawBytes = Math.max(1, Math.round(input.rawSceneBytes));
  const areaRatio =
    input.sceneFootprintKm2 > 0
      ? clampNumber(
          input.aoiIntersectionKm2 / input.sceneFootprintKm2,
          MIN_NISAR_AOI_AREA_RATIO,
          MAX_NISAR_AOI_AREA_RATIO,
        )
      : MIN_NISAR_AOI_AREA_RATIO;
  const polarizationScale = computeNisarPolarizationScale(
    input.selectedPolarizations,
    input.availablePolarizations,
  );
  const floor = Math.min(rawBytes, NISAR_METADATA_GRID_FLOOR_BYTES);
  const dataPayload = Math.max(0, rawBytes - floor);
  const estimatedBytes = Math.min(
    rawBytes,
    Math.round(floor + dataPayload * areaRatio * polarizationScale),
  );
  return { estimatedBytes, areaRatio, polarizationScale };
}

export function estimateNisarSubsetBatch(input: {
  scenes?: readonly NisarSceneSizeLike[];
  sceneCount?: number;
  aoiBbox?: GeographicBbox | null;
  aoiGeometry?: unknown;
  aoiIntersectionKm2?: number | null;
  sceneFootprintKm2?: number | null;
  polarizations: readonly string[];
  availablePolarizations?: readonly string[] | null;
  exactPreflightBytes?: number | null;
  isFallbackEstimate?: boolean;
}): NisarSubsetEstimate {
  const scenes = input.scenes ?? [];
  const count = Math.max(1, scenes.length || input.sceneCount || 1);
  const effectiveAoiBbox = input.aoiBbox ?? bboxFromGeoJson(input.aoiGeometry);
  const aoiPolyKm2 = geojsonAreaKm2(input.aoiGeometry);

  let rawBytes = 0;
  let estimatedSubsetBytes = 0;
  let areaRatioSum = 0;
  let polScaleSum = 0;

  if (scenes.length > 0) {
    for (const scene of scenes) {
      const sceneRaw =
        typeof scene.file_size_remote === "number" &&
        Number.isFinite(scene.file_size_remote) &&
        scene.file_size_remote > 0
          ? scene.file_size_remote
          : DEFAULT_NISAR_FULL_SCENE_BYTES;
      const sceneFootprintBbox =
        scene.footprint_bbox ?? bboxFromGeoJson(scene.footprint_geojson);
      const scenePolyKm2 = geojsonAreaKm2(scene.footprint_geojson);
      const sceneKm2 =
        typeof input.sceneFootprintKm2 === "number" && input.sceneFootprintKm2 > 0
          ? input.sceneFootprintKm2
          : scenePolyKm2 > 0
          ? scenePolyKm2
          : bboxAreaKm2(sceneFootprintBbox);

      let interKm2 =
        typeof input.aoiIntersectionKm2 === "number" && input.aoiIntersectionKm2 >= 0
          ? input.aoiIntersectionKm2
          : bboxIntersectionKm2(effectiveAoiBbox, sceneFootprintBbox);
      if (aoiPolyKm2 > 0 && interKm2 > 0) {
        interKm2 = Math.min(interKm2, aoiPolyKm2);
      }

      const fallbackRatio = sceneKm2 <= 0 ? (effectiveAoiBbox ? 0.18 : 0.25) : null;
      const resolved =
        fallbackRatio !== null
          ? estimateNisarSceneSubsetBytes({
              rawSceneBytes: sceneRaw,
              aoiIntersectionKm2: fallbackRatio,
              sceneFootprintKm2: 1,
              selectedPolarizations: input.polarizations,
              availablePolarizations:
                input.availablePolarizations ?? sceneAvailablePols(scene),
            })
          : estimateNisarSceneSubsetBytes({
              rawSceneBytes: sceneRaw,
              aoiIntersectionKm2: interKm2,
              sceneFootprintKm2: sceneKm2,
              selectedPolarizations: input.polarizations,
              availablePolarizations:
                input.availablePolarizations ?? sceneAvailablePols(scene),
            });

      rawBytes += sceneRaw;
      estimatedSubsetBytes += resolved.estimatedBytes;
      areaRatioSum += resolved.areaRatio;
      polScaleSum += resolved.polarizationScale;
    }
    areaRatioSum /= scenes.length;
    polScaleSum /= scenes.length;
  } else {
    const ratio =
      typeof input.aoiIntersectionKm2 === "number" &&
      typeof input.sceneFootprintKm2 === "number" &&
      input.sceneFootprintKm2 > 0
        ? clampNumber(
            input.aoiIntersectionKm2 / input.sceneFootprintKm2,
            MIN_NISAR_AOI_AREA_RATIO,
            MAX_NISAR_AOI_AREA_RATIO,
          )
        : effectiveAoiBbox
        ? 0.18
        : 0.25;
    const polScale = computeNisarPolarizationScale(
      input.polarizations,
      input.availablePolarizations,
    );
    const single = estimateNisarSceneSubsetBytes({
      rawSceneBytes: DEFAULT_NISAR_FULL_SCENE_BYTES,
      aoiIntersectionKm2: ratio,
      sceneFootprintKm2: 1,
      selectedPolarizations: input.polarizations,
      availablePolarizations: input.availablePolarizations,
    });
    rawBytes = count * DEFAULT_NISAR_FULL_SCENE_BYTES;
    estimatedSubsetBytes = count * single.estimatedBytes;
    areaRatioSum = single.areaRatio;
    polScaleSum = polScale;
  }

  let isFallbackEstimate = input.isFallbackEstimate ?? true;
  if (
    typeof input.exactPreflightBytes === "number" &&
    Number.isFinite(input.exactPreflightBytes) &&
    input.exactPreflightBytes > 0
  ) {
    estimatedSubsetBytes = Math.min(rawBytes, Math.round(input.exactPreflightBytes));
    isFallbackEstimate = false;
  }

  const savedBytes = Math.max(0, rawBytes - estimatedSubsetBytes);
  const savingsPercent = rawBytes > 0 ? Math.round((savedBytes / rawBytes) * 100) : 0;

  return {
    sceneCount: count,
    rawBytes,
    estimatedSubsetBytes,
    savedBytes,
    savingsPercent,
    areaRatio: areaRatioSum,
    polarizationScale: polScaleSum,
    isFallbackEstimate,
  };
}

export function formatNisarSubsetEstimateSummary(estimate: {
  rawBytes: number;
  estimatedSubsetBytes: number;
}): string {
  const subGb = (estimate.estimatedSubsetBytes / 1_000_000_000).toFixed(1);
  const rawGb = (estimate.rawBytes / 1_000_000_000).toFixed(1);
  return `预计裁剪后约 ${subGb} GB（完整包 ${rawGb} GB，按研究区面积占比估算）`;
}
