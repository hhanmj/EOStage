/**
 * aoiBookmarksAndMerge.ts
 * 行政区划与空间范围增强模块：
 * 1. 行政区划真实来源提取（依据后端返回的真实 provider，不贴未经核实的审定背书）
 * 2. 空间范围收藏夹（AOI Bookmarks）本地持久化与管理
 * 3. 行政区检索与使用历史（AOI Search History）本地持久化与管理
 * 4. 多区域几何（Polygon / MultiPolygon）与 Bbox 一键合并引擎
 */

import type { AdminBoundary, Bbox } from "./bridge.ts";
import { canonicalAdministrativeAoiGeometry } from "./administrativeAoiGeometry.ts";

export interface AoiBookmarkItem {
  id: string;
  label: string;
  bbox: Bbox;
  geometry: Record<string, unknown>;
  authoritySource: string;
  createdAt: string;
}

export interface AoiHistoryItem {
  id: string;
  label: string;
  query?: string;
  bbox: Bbox;
  geometry: Record<string, unknown>;
  adminLevel?: string;
  authoritySource: string;
  usedAt: string;
}

export interface MergeableRegionItem {
  id: string;
  label: string;
  bbox: Bbox;
  geometry: Record<string, unknown>;
  authoritySource?: string;
}

export interface MergedRegionResult {
  label: string;
  bbox: Bbox;
  tupleBbox: [number, number, number, number];
  geometry: Record<string, unknown>;
  regionCount: number;
}

const BOOKMARKS_STORAGE_KEY = "eostage.aoiBookmarks.v1";
const HISTORY_STORAGE_KEY = "eostage.aoiSearchHistory.v1";
const MAX_BOOKMARKS = 30;
const MAX_HISTORY = 15;

let memoryBookmarks: AoiBookmarkItem[] = [];
let memoryHistory: AoiHistoryItem[] = [];

function getStorage(): Storage | null {
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage;
    }
  } catch {
    // 忽略沙盒或隐私模式异常
  }
  return null;
}

const CHINA_PROVINCE_KEYWORDS = [
  "北京", "天津", "上海", "重庆", "河北", "山西", "辽宁", "吉林", "黑龙江",
  "江苏", "浙江", "安徽", "福建", "江西", "山东", "河南", "湖北", "湖南",
  "广东", "海南", "四川", "贵州", "云南", "陕西", "甘肃", "青海", "台湾",
  "内蒙古", "广西", "西藏", "宁夏", "新疆", "香港", "澳门", "中国",
];

/**
 * 判定行政区候选是否位于中国境内
 */
export function isDomesticChinaAdminBoundary(
  candidate: Partial<AdminBoundary> | { label?: string; display_name?: string; country_code?: string | null; provider?: string; bbox?: Bbox | null }
): boolean {
  const code = (candidate.country_code || "").trim().toUpperCase();
  if (code === "CN" || code === "CHN" || code === "HK" || code === "MO" || code === "TW") {
    return true;
  }
  const provider = (candidate.provider || "").toLowerCase();
  if (
    provider.includes("datav") ||
    provider.includes("tianditu") ||
    provider.includes("天地图") ||
    provider.includes("aliyun")
  ) {
    return true;
  }
  const text = `${candidate.display_name || ""} ${candidate.label || ""}`;
  if (CHINA_PROVINCE_KEYWORDS.some((kw) => text.includes(kw))) {
    return true;
  }
  if (/[\u4e00-\u9fa5]{2,}(省|市|区|县|自治州|盟|旗)/.test(text)) {
    return true;
  }
  const b = candidate.bbox;
  if (b && Number.isFinite(b.west) && Number.isFinite(b.south) && Number.isFinite(b.east) && Number.isFinite(b.north)) {
    if (b.west >= 73 && b.east <= 136 && b.south >= 3 && b.north <= 54 && /[\u4e00-\u9fa5]/.test(text)) {
      return true;
    }
  }
  return false;
}

/**
 * 返回行政区来源标识及说明（严格依据后端返回的真实 provider）
 */
export function resolveAdminBoundaryAuthority(
  candidate: Partial<AdminBoundary>
): {
  sourceKey: string;
  badgeZh: string;
  providerHint: string;
} {
  const provider = (candidate.provider || "").trim();
  if (provider) {
    return {
      sourceKey: provider.toLowerCase(),
      badgeZh: provider,
      providerHint: `行政区划来源：${provider}`,
    };
  }
  return {
    sourceKey: "admin_boundary",
    badgeZh: "行政区划",
    providerHint: "行政区划边界数据源",
  };
}

/**
 * 读取范围收藏夹列表
 */
export function getAoiBookmarks(): AoiBookmarkItem[] {
  const storage = getStorage();
  if (!storage) return [...memoryBookmarks];
  try {
    const raw = storage.getItem(BOOKMARKS_STORAGE_KEY);
    if (!raw) return [...memoryBookmarks];
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      memoryBookmarks = parsed.filter(
        (item): item is AoiBookmarkItem =>
          Boolean(item && typeof item.id === "string" && typeof item.label === "string" && item.bbox && item.geometry)
      );
      return [...memoryBookmarks];
    }
  } catch {
    // ignore
  }
  return [...memoryBookmarks];
}

/**
 * 保存或更新一个空间范围到收藏夹
 */
export function saveAoiBookmark(input: {
  label: string;
  bbox: Bbox;
  geometry: Record<string, unknown>;
  authoritySource?: string;
}): AoiBookmarkItem[] {
  const cleanLabel = input.label.trim() || "未命名范围";
  const canonical = canonicalAdministrativeAoiGeometry(input.geometry) ?? input.geometry;
  const current = getAoiBookmarks();
  const id = `bm-${cleanLabel}-${input.bbox.west.toFixed(3)}-${input.bbox.south.toFixed(3)}`;
  const entry: AoiBookmarkItem = {
    id,
    label: cleanLabel,
    bbox: { ...input.bbox, crs: input.bbox.crs || "EPSG:4326" },
    geometry: canonical,
    authoritySource: input.authoritySource ?? "admin_boundary",
    createdAt: new Date().toISOString(),
  };

  const next = [entry, ...current.filter((item) => item.id !== id && item.label !== cleanLabel)].slice(
    0,
    MAX_BOOKMARKS
  );
  memoryBookmarks = next;
  const storage = getStorage();
  if (storage) {
    try {
      storage.setItem(BOOKMARKS_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // ignore
    }
  }
  return [...next];
}

/**
 * 从收藏夹移除指定范围
 */
export function removeAoiBookmark(id: string): AoiBookmarkItem[] {
  const current = getAoiBookmarks();
  const next = current.filter((item) => item.id !== id);
  memoryBookmarks = next;
  const storage = getStorage();
  if (storage) {
    try {
      storage.setItem(BOOKMARKS_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // ignore
    }
  }
  return [...next];
}

/**
 * 清空范围收藏夹（用于测试重置）
 */
export function clearAoiBookmarks(): void {
  memoryBookmarks = [];
  const storage = getStorage();
  if (storage) {
    try {
      storage.removeItem(BOOKMARKS_STORAGE_KEY);
    } catch {
      // ignore
    }
  }
}

/**
 * 读取行政区检索与使用历史列表（按最近使用时间倒序）
 */
export function getAoiSearchHistory(): AoiHistoryItem[] {
  const storage = getStorage();
  if (!storage) return [...memoryHistory];
  try {
    const raw = storage.getItem(HISTORY_STORAGE_KEY);
    if (!raw) return [...memoryHistory];
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      memoryHistory = parsed.filter(
        (item): item is AoiHistoryItem =>
          Boolean(
            item &&
              typeof item.id === "string" &&
              typeof item.label === "string" &&
              item.bbox &&
              item.geometry
          )
      );
      return [...memoryHistory];
    }
  } catch {
    // ignore
  }
  return [...memoryHistory];
}

/**
 * 记录一次行政区检索/选择到历史列表（自动去重并置顶）
 */
export function saveAoiSearchHistory(input: {
  label: string;
  query?: string;
  bbox: Bbox;
  geometry: Record<string, unknown>;
  adminLevel?: string;
  authoritySource?: string;
}): AoiHistoryItem[] {
  const cleanLabel = input.label.trim() || "未命名范围";
  const canonical = canonicalAdministrativeAoiGeometry(input.geometry) ?? input.geometry;
  const current = getAoiSearchHistory();
  const id = `hist-${cleanLabel}-${input.bbox.west.toFixed(3)}-${input.bbox.south.toFixed(3)}`;
  const entry: AoiHistoryItem = {
    id,
    label: cleanLabel,
    query: input.query?.trim() || undefined,
    bbox: { ...input.bbox, crs: input.bbox.crs || "EPSG:4326" },
    geometry: canonical,
    adminLevel: input.adminLevel?.trim() || undefined,
    authoritySource: input.authoritySource ?? "admin_boundary",
    usedAt: new Date().toISOString(),
  };

  const next = [
    entry,
    ...current.filter((item) => item.id !== id && item.label !== cleanLabel),
  ].slice(0, MAX_HISTORY);
  memoryHistory = next;
  const storage = getStorage();
  if (storage) {
    try {
      storage.setItem(HISTORY_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // ignore
    }
  }
  return [...next];
}

/**
 * 从行政区检索历史中删除单条记录
 */
export function removeAoiSearchHistoryItem(id: string): AoiHistoryItem[] {
  const current = getAoiSearchHistory();
  const next = current.filter((item) => item.id !== id);
  memoryHistory = next;
  const storage = getStorage();
  if (storage) {
    try {
      storage.setItem(HISTORY_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // ignore
    }
  }
  return [...next];
}

/**
 * 清空全部行政区检索历史
 */
export function clearAoiSearchHistory(): AoiHistoryItem[] {
  memoryHistory = [];
  const storage = getStorage();
  if (storage) {
    try {
      storage.removeItem(HISTORY_STORAGE_KEY);
    } catch {
      // ignore
    }
  }
  return [];
}

/**
 * 提取单个 Polygon 或 MultiPolygon 的多边形坐标数组 (Polygon rings 列表)
 */
function extractPolygonCoordinateList(geometry: Record<string, unknown> | null | undefined): unknown[][][] {
  if (!geometry || typeof geometry !== "object") return [];
  const canonical = canonicalAdministrativeAoiGeometry(geometry) ?? geometry;
  const type = canonical.type;
  const coords = canonical.coordinates;
  if (!Array.isArray(coords)) return [];
  if (type === "Polygon") {
    return [coords as unknown[][]];
  }
  if (type === "MultiPolygon") {
    return coords as unknown[][][];
  }
  return [];
}

/**
 * 从 Polygon / MultiPolygon 几何坐标计算 Bbox（当未提供显式 bbox 时兜底）
 */
export function computeGeometryBbox(geometry: Record<string, unknown> | null | undefined): Bbox | null {
  const polygons = extractPolygonCoordinateList(geometry);
  if (polygons.length === 0) return null;

  let west = Infinity;
  let south = Infinity;
  let east = -Infinity;
  let north = -Infinity;

  const visit = (node: unknown): void => {
    if (!Array.isArray(node)) return;
    if (
      node.length >= 2 &&
      typeof node[0] === "number" &&
      typeof node[1] === "number" &&
      Number.isFinite(node[0]) &&
      Number.isFinite(node[1])
    ) {
      const lng = node[0];
      const lat = node[1];
      if (lng < west) west = lng;
      if (lng > east) east = lng;
      if (lat < south) south = lat;
      if (lat > north) north = lat;
      return;
    }
    for (const child of node) {
      visit(child);
    }
  };

  visit(polygons);

  if (!Number.isFinite(west) || !Number.isFinite(south) || !Number.isFinite(east) || !Number.isFinite(north)) {
    return null;
  }
  return { west, south, east, north, crs: "EPSG:4326" };
}

/**
 * 将多个区域（行政区或收藏范围）一键合并为单个 MultiPolygon 几何与联合 Bbox
 */
export function mergeAoiRegions(regions: ReadonlyArray<MergeableRegionItem>): MergedRegionResult | null {
  if (!regions || regions.length === 0) return null;

  const allPolygons: unknown[][][] = [];
  let west = Infinity;
  let south = Infinity;
  let east = -Infinity;
  let north = -Infinity;
  const labels: string[] = [];

  for (const reg of regions) {
    const polys = extractPolygonCoordinateList(reg.geometry);
    for (const p of polys) {
      allPolygons.push(p);
    }
    const b = reg.bbox ?? computeGeometryBbox(reg.geometry);
    if (b) {
      if (b.west < west) west = b.west;
      if (b.south < south) south = b.south;
      if (b.east > east) east = b.east;
      if (b.north > north) north = b.north;
    }
    const shortName = reg.label.split(/[·,，]/)[0]?.trim() || reg.label.trim();
    if (shortName && !labels.includes(shortName)) {
      labels.push(shortName);
    }
  }

  if (allPolygons.length === 0 || !Number.isFinite(west)) {
    return null;
  }

  const geometry: Record<string, unknown> =
    allPolygons.length === 1
      ? { type: "Polygon", coordinates: allPolygons[0] }
      : { type: "MultiPolygon", coordinates: allPolygons };

  const label =
    labels.length <= 3
      ? labels.join(" + ")
      : `${labels.slice(0, 2).join(" + ")} 等 ${regions.length} 个区域合并`;

  const bbox: Bbox = { west, south, east, north, crs: "EPSG:4326" };
  return {
    label,
    bbox,
    tupleBbox: [west, south, east, north],
    geometry,
    regionCount: regions.length,
  };
}
