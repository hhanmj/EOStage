import type { Bbox, Json } from "./bridge.ts";

export type OpticalDerivedSensorFamily =
  | "sentinel2_l2a"
  | "sentinel2_l1c"
  | "landsat_c2_l2"
  | "hls_s30"
  | "hls_l30";

export type OpticalBandPresetId =
  | "true_color"
  | "false_color_nir"
  | "swir"
  | "analysis_full"
  | "custom";

export type OpticalDerivedExclusionCode =
  | "USGS_M2M_ARCHIVE_UNSUPPORTED"
  | "CDSE_SAFE_ARCHIVE_UNSUPPORTED"
  | "UNSUPPORTED_OPTICAL_SOURCE";

export type OpticalDerivedSceneClassification =
  | { eligible: true; family: OpticalDerivedSensorFamily }
  | { eligible: false; reasonCode: OpticalDerivedExclusionCode };

export type OpticalBandOption = Readonly<{
  id: string;
  resolutionM: number;
}>;

export type OpticalDerivedSelectionResolution =
  | {
      ok: true;
      family: OpticalDerivedSensorFamily;
      mode: "single_scene_clip" | "batch_single_scene_clip" | "near_coincident_spatial_mosaic";
      sceneIds: string[];
    }
  | {
      ok: false;
      blockCode:
        | "NO_SCENES_SELECTED"
        | "TOO_MANY_SCENES_FOR_MOSAIC"
        | "MIXED_SENSOR_FAMILIES"
        | OpticalDerivedExclusionCode;
    };

export type OpticalAoiSceneLike = {
  scene_id: string;
  id?: string | null;
  file_name?: string | null;
  title?: string | null;
  provider?: string | null;
  dataset?: string | null;
  footprint_geojson?: Json | null;
  footprint_bbox?: Bbox | null;
  native_metadata?: Json | null;
};

export const OPTICAL_CLIP_MAX_SCENES = 64;
const MAX_MOSAIC_SCENES = OPTICAL_CLIP_MAX_SCENES;

const OPTICAL_FAMILY_BANDS: Record<OpticalDerivedSensorFamily, readonly OpticalBandOption[]> = {
  sentinel2_l2a: [
    { id: "B01", resolutionM: 60 },
    { id: "B02", resolutionM: 10 },
    { id: "B03", resolutionM: 10 },
    { id: "B04", resolutionM: 10 },
    { id: "B05", resolutionM: 20 },
    { id: "B06", resolutionM: 20 },
    { id: "B07", resolutionM: 20 },
    { id: "B08", resolutionM: 10 },
    { id: "B8A", resolutionM: 20 },
    { id: "B09", resolutionM: 60 },
    { id: "B11", resolutionM: 20 },
    { id: "B12", resolutionM: 20 },
  ],
  sentinel2_l1c: [
    { id: "B01", resolutionM: 60 },
    { id: "B02", resolutionM: 10 },
    { id: "B03", resolutionM: 10 },
    { id: "B04", resolutionM: 10 },
    { id: "B05", resolutionM: 20 },
    { id: "B06", resolutionM: 20 },
    { id: "B07", resolutionM: 20 },
    { id: "B08", resolutionM: 10 },
    { id: "B8A", resolutionM: 20 },
    { id: "B09", resolutionM: 60 },
    { id: "B10", resolutionM: 60 },
    { id: "B11", resolutionM: 20 },
    { id: "B12", resolutionM: 20 },
  ],
  landsat_c2_l2: [
    { id: "coastal", resolutionM: 30 },
    { id: "blue", resolutionM: 30 },
    { id: "green", resolutionM: 30 },
    { id: "red", resolutionM: 30 },
    { id: "nir08", resolutionM: 30 },
    { id: "swir16", resolutionM: 30 },
    { id: "swir22", resolutionM: 30 },
  ],
  hls_s30: [
    { id: "B01", resolutionM: 30 },
    { id: "B02", resolutionM: 30 },
    { id: "B03", resolutionM: 30 },
    { id: "B04", resolutionM: 30 },
    { id: "B05", resolutionM: 30 },
    { id: "B06", resolutionM: 30 },
    { id: "B07", resolutionM: 30 },
    { id: "B08", resolutionM: 30 },
    { id: "B8A", resolutionM: 30 },
    { id: "B09", resolutionM: 30 },
    { id: "B10", resolutionM: 30 },
    { id: "B11", resolutionM: 30 },
    { id: "B12", resolutionM: 30 },
  ],
  hls_l30: [
    { id: "B01", resolutionM: 30 },
    { id: "B02", resolutionM: 30 },
    { id: "B03", resolutionM: 30 },
    { id: "B04", resolutionM: 30 },
    { id: "B05", resolutionM: 30 },
    { id: "B06", resolutionM: 30 },
    { id: "B07", resolutionM: 30 },
    { id: "B09", resolutionM: 30 },
    { id: "B10", resolutionM: 30 },
    { id: "B11", resolutionM: 30 },
  ],
};

const BAND_PRESET_MAP: Record<
  OpticalDerivedSensorFamily,
  Record<Exclude<OpticalBandPresetId, "custom">, readonly string[]>
> = {
  sentinel2_l2a: {
    true_color: ["B04", "B03", "B02"],
    false_color_nir: ["B08", "B04", "B03"],
    swir: ["B12", "B8A", "B04"],
    analysis_full: ["B02", "B03", "B04", "B08", "B11", "B12"],
  },
  sentinel2_l1c: {
    true_color: ["B04", "B03", "B02"],
    false_color_nir: ["B08", "B04", "B03"],
    swir: ["B12", "B8A", "B04"],
    analysis_full: ["B02", "B03", "B04", "B08", "B11", "B12"],
  },
  landsat_c2_l2: {
    true_color: ["red", "green", "blue"],
    false_color_nir: ["nir08", "red", "green"],
    swir: ["swir22", "nir08", "red"],
    analysis_full: ["blue", "green", "red", "nir08", "swir16", "swir22"],
  },
  hls_l30: {
    true_color: ["B04", "B03", "B02"],
    false_color_nir: ["B05", "B04", "B03"],
    swir: ["B07", "B05", "B04"],
    analysis_full: ["B02", "B03", "B04", "B05", "B06", "B07"],
  },
  hls_s30: {
    true_color: ["B04", "B03", "B02"],
    false_color_nir: ["B8A", "B04", "B03"],
    swir: ["B12", "B8A", "B04"],
    analysis_full: ["B02", "B03", "B04", "B8A", "B11", "B12"],
  },
};

function metadataRecord(value: Json | null | undefined): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
}

function stringField(record: Record<string, unknown>, key: string): string {
  const value = record[key];
  return typeof value === "string" ? value.trim() : "";
}

export function classifyOpticalDerivedScene(
  scene: OpticalAoiSceneLike,
): OpticalDerivedSceneClassification {
  const provider = (scene.provider ?? "").trim();
  const metadata = metadataRecord(scene.native_metadata);
  const nativeCollection = stringField(metadata, "native_collection_id");
  if (provider === "usgs.eros.m2m" || nativeCollection === "landsat_ot_c2_l2") {
    return { eligible: false, reasonCode: "USGS_M2M_ARCHIVE_UNSUPPORTED" };
  }
  const logicalDataset =
    stringField(metadata, "logical_dataset") || (scene.dataset ?? "").trim();
  if (provider === "copernicus.dataspace.odata") {
    const productType = stringField(metadata, "product_type");
    const displayName = stringField(metadata, "display_name");
    const isL1c =
      logicalDataset === "optical.sentinel-2.l1c"
      || /MSIL1C|S2MSI1C/i.test(scene.scene_id)
      || /MSIL1C|S2MSI1C/i.test(displayName)
      || /MSIL1C|S2MSI1C/i.test(productType);
    if (isL1c) {
      return { eligible: true, family: "sentinel2_l1c" };
    }
    if (logicalDataset === "optical.sentinel-2.l2a") {
      return { eligible: true, family: "sentinel2_l2a" };
    }
    return { eligible: false, reasonCode: "CDSE_SAFE_ARCHIVE_UNSUPPORTED" };
  }
  if (logicalDataset === "optical.sentinel-2.l1c") {
    return { eligible: true, family: "sentinel2_l1c" };
  }
  if (logicalDataset === "optical.sentinel-2.l2a") {
    return { eligible: true, family: "sentinel2_l2a" };
  }
  if (logicalDataset === "optical.landsat.c2.l2") {
    if (provider === "microsoft.planetary-computer") {
      return { eligible: true, family: "landsat_c2_l2" };
    }
    return { eligible: false, reasonCode: "UNSUPPORTED_OPTICAL_SOURCE" };
  }
  if (logicalDataset === "optical.hls.s30.v2") {
    return { eligible: true, family: "hls_s30" };
  }
  if (logicalDataset === "optical.hls.l30.v2") {
    return { eligible: true, family: "hls_l30" };
  }

  if (provider === "microsoft.planetary-computer" || provider === "element84.earth-search") {
    const sourceKind = stringField(metadata, "source_kind").toLowerCase();
    if (sourceKind === "landsat" && provider === "microsoft.planetary-computer") {
      return { eligible: true, family: "landsat_c2_l2" };
    }
    if (
      sourceKind === "sentinel2"
      || (provider === "microsoft.planetary-computer" && logicalDataset === "")
    ) {
      return { eligible: true, family: "sentinel2_l2a" };
    }
  }

  return { eligible: false, reasonCode: "UNSUPPORTED_OPTICAL_SOURCE" };
}

export function availableBandsForOpticalFamily(
  family: OpticalDerivedSensorFamily,
): readonly OpticalBandOption[] {
  return OPTICAL_FAMILY_BANDS[family];
}

export function bandPresetOrder(
  family: OpticalDerivedSensorFamily,
  preset: Exclude<OpticalBandPresetId, "custom">,
): string[] {
  return [...BAND_PRESET_MAP[family][preset]];
}

export function resolveOpticalDerivedSelection(
  scenes: readonly OpticalAoiSceneLike[],
): OpticalDerivedSelectionResolution {
  if (scenes.length === 0) {
    return { ok: false, blockCode: "NO_SCENES_SELECTED" };
  }
  if (scenes.length > MAX_MOSAIC_SCENES) {
    return { ok: false, blockCode: "TOO_MANY_SCENES_FOR_MOSAIC" };
  }

  let family: OpticalDerivedSensorFamily | null = null;
  const sceneIds: string[] = [];
  for (const scene of scenes) {
    const sid = (scene.scene_id || scene.id || "").trim();
    const classified = classifyOpticalDerivedScene({ ...scene, scene_id: sid });
    if (!classified.eligible) {
      return { ok: false, blockCode: classified.reasonCode };
    }
    if (family === null) {
      family = classified.family;
    } else if (family !== classified.family) {
      return { ok: false, blockCode: "MIXED_SENSOR_FAMILIES" };
    }
    sceneIds.push(sid);
  }

  return {
    ok: true,
    family: family!,
    mode:
      scenes.length === 1
        ? "single_scene_clip"
        : "batch_single_scene_clip",
    sceneIds,
  };
}

function isFiniteLonLatPair(value: unknown): value is [number, number] {
  return (
    Array.isArray(value)
    && value.length >= 2
    && typeof value[0] === "number"
    && Number.isFinite(value[0])
    && typeof value[1] === "number"
    && Number.isFinite(value[1])
    && value[0] >= -180
    && value[0] <= 180
    && value[1] >= -90
    && value[1] <= 90
  );
}

function normalizeClosedRing(raw: unknown): [number, number][] | null {
  if (!Array.isArray(raw)) return null;
  const coords: [number, number][] = [];
  for (const point of raw) {
    if (!isFiniteLonLatPair(point)) return null;
    coords.push([point[0], point[1]]);
  }
  if (coords.length < 3) return null;
  const first = coords[0];
  const last = coords[coords.length - 1];
  if (first[0] !== last[0] || first[1] !== last[1]) {
    coords.push([first[0], first[1]]);
  }
  return coords.length >= 4 ? coords : null;
}

function ringsFromGeoJson(geometry: unknown): [number, number][][] {
  if (!geometry || typeof geometry !== "object" || Array.isArray(geometry)) {
    return [];
  }
  const obj = geometry as Record<string, unknown>;
  if (obj.type === "Feature") {
    return ringsFromGeoJson(obj.geometry);
  }
  if (obj.type === "Polygon" && Array.isArray(obj.coordinates)) {
    const outer = normalizeClosedRing(obj.coordinates[0]);
    return outer ? [outer] : [];
  }
  if (obj.type === "MultiPolygon" && Array.isArray(obj.coordinates)) {
    const rings: [number, number][][] = [];
    for (const poly of obj.coordinates) {
      if (Array.isArray(poly)) {
        const outer = normalizeClosedRing(poly[0]);
        if (outer) rings.push(outer);
      }
    }
    return rings;
  }
  return [];
}

function bboxFromRings(
  rings: readonly (readonly [number, number][])[],
): [number, number, number, number] {
  let west = Number.POSITIVE_INFINITY;
  let south = Number.POSITIVE_INFINITY;
  let east = Number.NEGATIVE_INFINITY;
  let north = Number.NEGATIVE_INFINITY;
  for (const ring of rings) {
    for (const [lon, lat] of ring) {
      if (lon < west) west = lon;
      if (lon > east) east = lon;
      if (lat < south) south = lat;
      if (lat > north) north = lat;
    }
  }
  return [west, south, east, north];
}

export function extractWgs84RingsAndBbox(
  geometry: Json | Record<string, unknown> | null | undefined,
  fallbackBbox?: Bbox | readonly [number, number, number, number] | null,
): {
  aoiBboxWgs84: [number, number, number, number];
  aoiRingsWgs84: number[][][];
} | null {
  const rings = ringsFromGeoJson(geometry);
  if (rings.length > 0) {
    const aoiBboxWgs84 = bboxFromRings(rings);
    if (aoiBboxWgs84[0] < aoiBboxWgs84[2] && aoiBboxWgs84[1] < aoiBboxWgs84[3]) {
      return { aoiBboxWgs84, aoiRingsWgs84: rings };
    }
  }

  let tuple: [number, number, number, number] | null = null;
  if (Array.isArray(fallbackBbox) && fallbackBbox.length === 4) {
    const nums = fallbackBbox;
    if (
      nums.every((n) => typeof n === "number" && Number.isFinite(n))
      && nums[0] < nums[2]
      && nums[1] < nums[3]
    ) {
      tuple = [nums[0], nums[1], nums[2], nums[3]];
    }
  } else if (fallbackBbox && typeof fallbackBbox === "object" && !Array.isArray(fallbackBbox)) {
    const box = fallbackBbox as Bbox;
    if (
      Number.isFinite(box.west)
      && Number.isFinite(box.south)
      && Number.isFinite(box.east)
      && Number.isFinite(box.north)
      && box.west < box.east
      && box.south < box.north
    ) {
      tuple = [box.west, box.south, box.east, box.north];
    }
  }
  if (!tuple) return null;

  const [w, s, e, n] = tuple;
  return {
    aoiBboxWgs84: tuple,
    aoiRingsWgs84: [
      [
        [w, s],
        [e, s],
        [e, n],
        [w, n],
        [w, s],
      ],
    ],
  };
}
