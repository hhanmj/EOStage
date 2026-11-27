/**
 * companionDownloadLogic.ts
 * 配套下载纯逻辑模块
 */

import type { SceneRow, Bbox } from "./bridge.ts";
import { getStorage } from "./webStorage.ts";

export type AuxiliaryExtentMode = "study_area" | "scene_coverage" | "redraw_on_map";

export interface CompanionState {
  preciseOrbit: boolean;
  dem: boolean;
  gacos: boolean;
  extentMode: AuxiliaryExtentMode;
}

export interface OrbitMatchSummary {
  totalScenes: number;
  matchedCount: number;
  pendingCount: number;
  isSentinel1: boolean;
}

export interface DemEstimateSummary {
  estimatedTiles: number;
  resolution: string;
}

export interface GacosEstimateSummary {
  uniqueDateCount: number;
  requestCount: number;
  dateBatches: number;
  spatialBlocks: number;
}

export const COMPANION_STORAGE_KEY_PREFIX = "sp_companion_pref_";

const DERIVED_SAR_INTERFEROGRAM_RESOURCE_IDS = new Set([
  "science.aria.s1-gunw.v1",
  "science.opera.disp-s1.v1",
  "science.comet.licsar.v1",
]);

/**
 * 判断是否为 Sentinel-1 原始雷达数据源（排除已预配准/干涉成品 ARIA、OPERA、COMET-LiCSAR）
 */
export function isSentinel1Resource(resourceId?: string | null): boolean {
  if (!resourceId) return false;
  const s = resourceId.toLowerCase().trim();
  if (s === "science.aws.sentinel-1-grd.v1") return false;
  if (DERIVED_SAR_INTERFEROGRAM_RESOURCE_IDS.has(s)) return false;
  return s.includes("sentinel-1") || s.includes("sentinel1") || s.includes("s1");
}

/**
 * 判断数据源是否属于支持伴随下载的雷达家族（Sentinel-1、NISAR L/S、ALOS-1/2、OPERA/ARIA/LiCSAR 等）
 */
export function isSarCompanionResource(
  resourceId?: string | null,
  family?: string | null
): boolean {
  const s = (resourceId || "").toLowerCase().trim();
  if (!s) return family === "sar";
  if (
    s.startsWith("dem.") ||
    s.startsWith("atmospheric.") ||
    s.startsWith("auxiliary.") ||
    s.startsWith("orbit.") ||
    s.startsWith("open.") ||
    s.startsWith("science.gee.")
  ) {
    return false;
  }
  if (family === "optical" || family === "dem" || family === "open") {
    return false;
  }
  if (family === "sar") return true;
  return (
    s.includes("sentinel-1") ||
    s.includes("sentinel1") ||
    s.includes("nisar") ||
    s.includes("alos") ||
    s.includes("opera.disp-s1") ||
    s.includes("aria.s1-gunw") ||
    s.includes("comet.licsar") ||
    s.includes("radarsat") ||
    s.includes("seasat") ||
    s.includes("ers") ||
    s.includes("envisat")
  );
}

/**
 * 获取本地偏好记忆中的 30m 参考 DEM 伴随开关（全部雷达适用，默认 false）
 */
export function getSavedCompanionDemPreference(resourceId: string): boolean {
  if (!isSarCompanionResource(resourceId)) return false;
  try {
    const storage = getStorage();
    if (!storage) return false;
    const key = `${COMPANION_STORAGE_KEY_PREFIX}${resourceId}_dem`;
    const val = storage.getItem(key);
    if (val !== null) return val === "true";
  } catch {
    // Fail-closed safely
  }
  return false;
}

/**
 * 保存本地偏好记忆中的 30m 参考 DEM 伴随开关
 */
export function setSavedCompanionDemPreference(resourceId: string, enabled: boolean): void {
  try {
    const storage = getStorage();
    if (!storage) return;
    const key = `${COMPANION_STORAGE_KEY_PREFIX}${resourceId}_dem`;
    storage.setItem(key, String(enabled));
  } catch {
    // Fail-closed safely
  }
}

/**
 * 判断是否默认勾选精密轨道（SLC/Burst 默认 true，GRD 默认 false）
 */
export function getDefaultPreciseOrbit(resourceId?: string | null, productType?: string | null): boolean {
  if (!isSentinel1Resource(resourceId)) return false;
  const prod = (productType || "").toUpperCase();
  if (prod.includes("GRD")) return false;
  return true;
}

/**
 * 获取本地偏好记忆中的精密轨道设置
 */
export function getSavedCompanionOrbitPreference(resourceId: string, productType?: string): boolean {
  if (!isSentinel1Resource(resourceId)) return false;
  try {
    const storage = getStorage();
    if (!storage) return getDefaultPreciseOrbit(resourceId, productType);
    const key = `${COMPANION_STORAGE_KEY_PREFIX}${resourceId}_orbit`;
    const val = storage.getItem(key);
    if (val !== null) return val === "true";
  } catch {
    // Fail-closed to default
  }
  return getDefaultPreciseOrbit(resourceId, productType);
}

/**
 * 保存本地偏好记忆中的精密轨道设置
 */
export function setSavedCompanionOrbitPreference(resourceId: string, enabled: boolean): void {
  try {
    const storage = getStorage();
    if (!storage) return;
    const key = `${COMPANION_STORAGE_KEY_PREFIX}${resourceId}_orbit`;
    storage.setItem(key, String(enabled));
  } catch {
    // Fail-closed safely
  }
}

/**
 * 获取本地偏好记忆中的 GACOS 大气延迟设置（雷达光源适用，默认 false）
 */
export function getSavedCompanionGacosPreference(resourceId: string): boolean {
  if (!isSarCompanionResource(resourceId)) return false;
  try {
    const storage = getStorage();
    if (!storage) return false;
    const key = `${COMPANION_STORAGE_KEY_PREFIX}${resourceId}_gacos`;
    const val = storage.getItem(key);
    if (val !== null) return val === "true";
  } catch {
    // Fail-closed to default
  }
  return false;
}

/**
 * 保存本地偏好记忆中的 GACOS 大气延迟设置
 */
export function setSavedCompanionGacosPreference(resourceId: string, enabled: boolean): void {
  try {
    const storage = getStorage();
    if (!storage) return;
    const key = `${COMPANION_STORAGE_KEY_PREFIX}${resourceId}_gacos`;
    storage.setItem(key, String(enabled));
  } catch {
    // Fail-closed safely
  }
}

/**
 * 获取本地偏好记忆中的辅助数据范围模式（默认 study_area）
 */
export function getSavedCompanionExtentMode(resourceId: string): AuxiliaryExtentMode {
  try {
    const storage = getStorage();
    if (!storage) return "study_area";
    const key = `${COMPANION_STORAGE_KEY_PREFIX}${resourceId}_extent_mode`;
    const val = storage.getItem(key);
    if (val === "study_area" || val === "scene_coverage" || val === "redraw_on_map") {
      return val;
    }
  } catch {
    // Fail-closed safely
  }
  return "study_area";
}

/**
 * 保存本地偏好记忆中的辅助数据范围模式
 */
export function setSavedCompanionExtentMode(resourceId: string, mode: AuxiliaryExtentMode): void {
  try {
    const storage = getStorage();
    if (!storage) return;
    const key = `${COMPANION_STORAGE_KEY_PREFIX}${resourceId}_extent_mode`;
    storage.setItem(key, mode);
  } catch {
    // Fail-closed safely
  }
}

/**
 * 计算精密轨道匹配情况
 * 规则：采集时间距今 >= 20 天发布 POEORB；< 20 天为尚未发布（待补）
 */
export function calculateOrbitMatch(
  scenes: SceneRow[],
  resourceId?: string | null,
  referenceNow: number = Date.now()
): OrbitMatchSummary {
  const isS1 = isSentinel1Resource(resourceId);
  if (!isS1 || scenes.length === 0) {
    return {
      totalScenes: scenes.length,
      matchedCount: 0,
      pendingCount: 0,
      isSentinel1: isS1,
    };
  }

  const twentyDaysMs = 20 * 24 * 60 * 60 * 1000;
  let matched = 0;
  let pending = 0;

  for (const scene of scenes) {
    const dateStr = (scene.acquisition_datetime || (scene as any).acquisition_date || (scene as any).start_time) || (scene as any).start_time || "";
    if (!dateStr) {
      matched += 1;
      continue;
    }
    const t = new Date(dateStr).getTime();
    if (isNaN(t) || referenceNow - t >= twentyDaysMs) {
      matched += 1;
    } else {
      pending += 1;
    }
  }

  return {
    totalScenes: scenes.length,
    matchedCount: matched,
    pendingCount: pending,
    isSentinel1: isS1,
  };
}

/**
 * 将可能的数组形式 [west, south, east, north] 或 Bbox 对象归一化为标准 Bbox
 */
export function normalizeBbox(b: Bbox | number[] | null | undefined): Bbox | null {
  if (!b) return null;
  if (Array.isArray(b) && b.length >= 4 && typeof b[0] === "number") {
    return {
      west: Number(b[0]),
      south: Number(b[1]),
      east: Number(b[2]),
      north: Number(b[3]),
      crs: "EPSG:4326",
    };
  }
  if (typeof (b as Bbox).west === "number") {
    return b as Bbox;
  }
  return null;
}

/**
 * 计算两个或多个 Bbox 的并集外包框
 */
export function unionBboxes(bboxes: (Bbox | number[] | null | undefined)[]): Bbox | null {
  const valid = bboxes.map(normalizeBbox).filter((b): b is Bbox => Boolean(b));
  if (valid.length === 0) return null;

  let west = valid[0].west;
  let south = valid[0].south;
  let east = valid[0].east;
  let north = valid[0].north;

  for (let i = 1; i < valid.length; i++) {
    west = Math.min(west, valid[i].west);
    south = Math.min(south, valid[i].south);
    east = Math.max(east, valid[i].east);
    north = Math.max(north, valid[i].north);
  }

  return { west, south, east, north, crs: valid[0].crs || "EPSG:4326" };
}

/**
 * 从 GeoJSON 对象（Polygon / MultiPolygon / Feature / FeatureCollection）提取外包框
 */
export function geojsonToBbox(geometry: Record<string, unknown> | null | undefined): Bbox | null {
  if (!geometry || typeof geometry !== "object") return null;

  let minLon = Infinity;
  let minLat = Infinity;
  let maxLon = -Infinity;
  let maxLat = -Infinity;
  let count = 0;

  const visitCoords = (coords: unknown): void => {
    if (!Array.isArray(coords)) return;
    if (
      coords.length >= 2 &&
      typeof coords[0] === "number" &&
      typeof coords[1] === "number" &&
      Number.isFinite(coords[0]) &&
      Number.isFinite(coords[1])
    ) {
      const lon = coords[0];
      const lat = coords[1];
      if (lon < minLon) minLon = lon;
      if (lon > maxLon) maxLon = lon;
      if (lat < minLat) minLat = lat;
      if (lat > maxLat) maxLat = lat;
      count += 1;
      return;
    }
    for (const item of coords) {
      visitCoords(item);
    }
  };

  const visitGeojson = (node: unknown): void => {
    if (!node || typeof node !== "object") return;
    const rec = node as Record<string, unknown>;
    if (Array.isArray(rec.coordinates)) {
      visitCoords(rec.coordinates);
    }
    if (rec.geometry && typeof rec.geometry === "object") {
      visitGeojson(rec.geometry);
    }
    if (Array.isArray(rec.features)) {
      for (const f of rec.features) {
        visitGeojson(f);
      }
    }
    if (Array.isArray(rec.geometries)) {
      for (const g of rec.geometries) {
        visitGeojson(g);
      }
    }
  };

  visitGeojson(geometry);
  if (count === 0 || !(minLon < maxLon) || !(minLat < maxLat)) {
    return null;
  }
  return {
    west: minLon,
    south: minLat,
    east: maxLon,
    north: maxLat,
    crs: "EPSG:4326",
  };
}

/**
 * 从单个 SceneRow 提取外包框（依次检查 footprint_bbox、sar_product.footprint_bbox、footprint_geojson）
 */
export function extractSceneBbox(scene: SceneRow): Bbox | null {
  const direct = normalizeBbox(scene.footprint_bbox as Bbox | number[] | null | undefined);
  if (direct) return direct;
  const sarBbox = normalizeBbox(
    (scene as unknown as { sar_product?: { footprint_bbox?: Bbox | number[] | null } }).sar_product
      ?.footprint_bbox
  );
  if (sarBbox) return sarBbox;
  return geojsonToBbox(scene.footprint_geojson as Record<string, unknown> | null | undefined);
}

/**
 * 从 SceneRow 列表中提取外包框
 */
export function scenesCoverageBbox(scenes: SceneRow[]): Bbox | null {
  const bboxes = scenes
    .map((s) => extractSceneBbox(s))
    .filter((b): b is Bbox => Boolean(b));
  return unionBboxes(bboxes);
}

/**
 * 将 Bbox 限制在单次请求最大跨度内（默认 9.8°，留出 0.05°×2 缓冲后不超 GACOS 10° 上限）
 */
export function clampBboxToMaxSpan(bbox: Bbox, maxSpanDegrees = 9.8): Bbox {
  let { west, south, east, north } = bbox;
  const width = east - west;
  const height = north - south;
  if (width > maxSpanDegrees) {
    const centerLon = (west + east) / 2;
    west = centerLon - maxSpanDegrees / 2;
    east = centerLon + maxSpanDegrees / 2;
  }
  if (height > maxSpanDegrees) {
    const centerLat = (south + north) / 2;
    south = centerLat - maxSpanDegrees / 2;
    north = centerLat + maxSpanDegrees / 2;
  }
  return { west, south, east, north, crs: bbox.crs || "EPSG:4326" };
}

/**
 * 根据所选模式计算有效的辅助数据范围 Bbox
 */
export function resolveAuxiliaryExtent(
  mode: AuxiliaryExtentMode,
  studyArea: Bbox | number[] | null,
  scenes: SceneRow[],
  customRedrawBbox?: Bbox | number[] | null
): Bbox | null {
  const normalizedStudyArea = normalizeBbox(studyArea);
  const normalizedCustom = normalizeBbox(customRedrawBbox);
  if (mode === "redraw_on_map" && normalizedCustom) {
    return normalizedCustom;
  }
  if (mode === "scene_coverage") {
    const scBbox = scenesCoverageBbox(scenes);
    return scBbox || normalizedStudyArea;
  }
  // study_area 模式：若有 studyArea 则带轻微缓冲（或直接使用）
  return normalizedStudyArea || scenesCoverageBbox(scenes);
}

/**
 * 将 Bbox 转换为 GeoJSON Polygon 几何对象
 */
export function bboxToPolygonGeojson(bbox: Bbox): Record<string, unknown> {
  const { west, south, east, north } = bbox;
  return {
    type: "Polygon",
    coordinates: [
      [
        [west, south],
        [east, south],
        [east, north],
        [west, north],
        [west, south],
      ],
    ],
  };
}

/**
 * 根据辅助数据范围模式解析用于 GACOS / DEM 的 GeoJSON 几何体
 */
export function resolveAuxiliaryExtentGeojson(
  mode: AuxiliaryExtentMode,
  studyAreaGeometry: Record<string, unknown> | null | undefined,
  studyAreaBbox: Bbox | number[] | null | undefined,
  scenes: SceneRow[]
): Record<string, unknown> | null {
  const MAX_SAFE_GACOS_SPAN = 9.8;
  if (mode === "study_area" && studyAreaGeometry && Object.keys(studyAreaGeometry).length > 0) {
    const geomBbox = geojsonToBbox(studyAreaGeometry);
    if (
      geomBbox &&
      geomBbox.east - geomBbox.west <= MAX_SAFE_GACOS_SPAN &&
      geomBbox.north - geomBbox.south <= MAX_SAFE_GACOS_SPAN
    ) {
      return studyAreaGeometry;
    }
    // 若行政区几何包含远端飞地或跨度超过 10° 上限，优先退回已选影像覆盖范围或裁剪后的外包框
    const scBbox = scenesCoverageBbox(scenes);
    if (
      scBbox &&
      scBbox.east - scBbox.west <= MAX_SAFE_GACOS_SPAN &&
      scBbox.north - scBbox.south <= MAX_SAFE_GACOS_SPAN
    ) {
      return bboxToPolygonGeojson(scBbox);
    }
    if (geomBbox) {
      return bboxToPolygonGeojson(clampBboxToMaxSpan(geomBbox, MAX_SAFE_GACOS_SPAN));
    }
  }
  const extent = resolveAuxiliaryExtent(mode, studyAreaBbox ?? null, scenes);
  if (extent) {
    return bboxToPolygonGeojson(clampBboxToMaxSpan(extent, MAX_SAFE_GACOS_SPAN));
  }
  if (studyAreaGeometry && Object.keys(studyAreaGeometry).length > 0) {
    const geomBbox = geojsonToBbox(studyAreaGeometry);
    return geomBbox
      ? bboxToPolygonGeojson(clampBboxToMaxSpan(geomBbox, MAX_SAFE_GACOS_SPAN))
      : studyAreaGeometry;
  }
  return null;
}

/**
 * 估算 Copernicus 30m DEM 瓦片数量（1°×1° 瓦片网格）
 */
export function estimateDemTiles(extent: Bbox | null): DemEstimateSummary {
  if (!extent) {
    return { estimatedTiles: 4, resolution: "Copernicus 30 m" };
  }

  const w = Math.floor(extent.west);
  const e = Math.ceil(extent.east);
  const s = Math.floor(extent.south);
  const n = Math.ceil(extent.north);

  const lonSpan = Math.max(1, Math.abs(e - w));
  const latSpan = Math.max(1, Math.abs(n - s));

  const tiles = lonSpan * latSpan;
  return {
    estimatedTiles: Math.max(1, tiles),
    resolution: "Copernicus 30 m",
  };
}

/**
 * 计算 GACOS 申请批次（严格依据 §7 新问题 lines 234）
 * 规则：
 * 1. 每次申请不超过 20 个唯一日期：dateBatches = Math.ceil(dates.length / 20)
 * 2. 单块范围不超过 10°×10°：spatialBlocks = Math.ceil((east - west) / 10) * Math.ceil((north - south) / 10)
 * 3. 总申请次数 = dateBatches * spatialBlocks
 */
export function estimateGacosRequests(
  scenes: SceneRow[],
  extent: Bbox | null
): GacosEstimateSummary {
  // 1. 统计唯一采集日期
  const dateSet = new Set<string>();
  for (const scene of scenes) {
    const raw = (scene.acquisition_datetime || (scene as any).acquisition_date || (scene as any).start_time) || (scene as any).start_time;
    if (raw) {
      // 提取 YYYY-MM-DD
      const datePart = raw.slice(0, 10);
      if (datePart) dateSet.add(datePart);
    }
  }

  const uniqueDateCount = Math.max(1, dateSet.size);
  const dateBatches = Math.max(1, Math.ceil(uniqueDateCount / 20));

  // 2. 空间切块（每块最大 10°×10°）
  let spatialBlocks = 1;
  if (extent) {
    const lonSpan = Math.max(0.1, Math.abs(extent.east - extent.west));
    const latSpan = Math.max(0.1, Math.abs(extent.north - extent.south));
    const lonBlocks = Math.max(1, Math.ceil(lonSpan / 10));
    const latBlocks = Math.max(1, Math.ceil(latSpan / 10));
    spatialBlocks = lonBlocks * latBlocks;
  }

  const requestCount = dateBatches * spatialBlocks;

  return {
    uniqueDateCount,
    requestCount,
    dateBatches,
    spatialBlocks,
  };
}
