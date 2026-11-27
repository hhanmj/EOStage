import type { Bbox, SceneRow } from "../../lib/bridge.ts";
import { sceneProductKey } from "../../lib/sarProduct.ts";
import { RESULT_PAGE_SIZE } from "./batchDeliveryLogic.ts";

/**
 * 获取场景在地图内部使用的身份标识（与 WorkbenchMap 保持完全一致）。
 */
export function mapIdentityOf(row: SceneRow): string {
  return sceneProductKey(row);
}

/**
 * 构建 scene_id 与地图 identity 的双向索引映射。
 */
export function buildIdentityIndex(rows: SceneRow[]): {
  toIdentity: Map<string, string>;
  toSceneId: Map<string, string>;
} {
  const toIdentity = new Map<string, string>();
  const toSceneId = new Map<string, string>();
  for (const r of rows) {
    if (!r.scene_id) continue;
    const identity = mapIdentityOf(r);
    toIdentity.set(r.scene_id, identity);
    toSceneId.set(identity, r.scene_id);
  }
  return { toIdentity, toSceneId };
}

export type StackableRowInput = {
  scene_id?: string | null;
  product_id?: string | null;
  file_name?: string | null;
  product_name?: string | null;
  orbit_direction?: string | null;
  relative_orbit?: number | null;
  path?: number | null;
  track?: number | null;
  frame?: number | null;
  burst_id?: string | null;
  cloud_cover?: number | null;
  footprint_bbox?: Bbox | null;
  footprint_geojson?: unknown;
  native_metadata?: unknown;
};

function recordObject(val: unknown): Record<string, unknown> | null {
  return val && typeof val === "object" && !Array.isArray(val)
    ? (val as Record<string, unknown>)
    : null;
}

function extractMgrsTile(row: StackableRowInput, meta: Record<string, unknown> | null): string | null {
  const props = recordObject(meta?.properties);
  const direct = [props?.mgrs_tile, props?.tile_id, props?.grid_code, meta?.mgrs_tile, meta?.tile_id];
  for (const val of direct) {
    if (typeof val === "string") {
      const m = /\b(T?\d{2}[A-Z]{3})\b/i.exec(val.trim());
      if (m) {
        const raw = m[1].toUpperCase();
        return raw.startsWith("T") ? raw : `T${raw}`;
      }
    }
  }
  const names = [row.file_name, row.scene_id, row.product_id, row.product_name];
  for (const name of names) {
    if (typeof name !== "string" || !name.trim()) continue;
    const underscored = /(?:^|_)(T\d{2}[A-Z]{3})(?:_|\.|$)/i.exec(name.trim());
    if (underscored) return underscored[1].toUpperCase();
    const standalone = /\b(T\d{2}[A-Z]{3})\b/i.exec(name.trim());
    if (standalone) return standalone[1].toUpperCase();
  }
  return null;
}

function extractWrsPathRow(
  row: StackableRowInput,
  meta: Record<string, unknown> | null,
): { path: string; row: string } | null {
  const props = recordObject(meta?.properties);
  const pRaw = props?.wrs_path ?? meta?.wrs_path;
  const rRaw = props?.wrs_row ?? meta?.wrs_row;
  const parsePart = (v: unknown): string | null => {
    if (typeof v === "number" && Number.isFinite(v) && v >= 0) {
      return String(Math.trunc(v)).padStart(3, "0");
    }
    if (typeof v === "string" && /^\d{1,3}$/.test(v.trim())) {
      return v.trim().padStart(3, "0");
    }
    return null;
  };
  const pStr = parsePart(pRaw);
  const rStr = parsePart(rRaw);
  if (pStr && rStr) return { path: pStr, row: rStr };

  const names = [row.scene_id, row.file_name, row.product_id, row.product_name];
  for (const name of names) {
    if (typeof name !== "string" || !name.trim()) continue;
    const m = /\bL[CEOT]\d{2}_L\d[A-Z0-9]+_(\d{3})(\d{3})_/i.exec(name.trim());
    if (m) return { path: m[1], row: m[2] };
  }
  return null;
}

function extractSpatialBboxFallback(row: StackableRowInput): Bbox | null {
  const b = row.footprint_bbox as unknown;
  if (
    Array.isArray(b) &&
    b.length >= 4 &&
    b.slice(0, 4).every((n) => typeof n === "number" && Number.isFinite(n)) &&
    b[2] !== b[0] &&
    b[3] !== b[1]
  ) {
    return {
      west: Number(b[0]),
      south: Number(b[1]),
      east: Number(b[2]),
      north: Number(b[3]),
      crs: "EPSG:4326",
    };
  }
  const obj = row.footprint_bbox;
  if (
    obj &&
    Number.isFinite(obj.west) &&
    Number.isFinite(obj.south) &&
    Number.isFinite(obj.east) &&
    Number.isFinite(obj.north) &&
    obj.east !== obj.west &&
    obj.north !== obj.south
  ) {
    return obj;
  }
  const gj = recordObject(row.footprint_geojson);
  if (!gj || !Array.isArray(gj.coordinates)) return null;
  let minLon = Infinity;
  let minLat = Infinity;
  let maxLon = -Infinity;
  let maxLat = -Infinity;
  const walk = (node: unknown) => {
    if (!Array.isArray(node)) return;
    if (node.length >= 2 && typeof node[0] === "number" && typeof node[1] === "number") {
      if (Number.isFinite(node[0]) && Number.isFinite(node[1])) {
        minLon = Math.min(minLon, node[0]);
        maxLon = Math.max(maxLon, node[0]);
        minLat = Math.min(minLat, node[1]);
        maxLat = Math.max(maxLat, node[1]);
      }
      return;
    }
    for (const child of node) walk(child);
  };
  walk(gj.coordinates);
  if (!Number.isFinite(minLon) || !Number.isFinite(minLat) || minLon === maxLon || minLat === maxLat) {
    return null;
  }
  return { west: minLon, south: minLat, east: maxLon, north: maxLat, crs: "EPSG:4326" };
}

/**
 * 计算场景的分堆键（支持 SAR 同轨同帧、光学 MGRS 图幅 / WRS-2 条带号，以及空间覆盖兜底）。
 * 对含轨道号的 SAR 数据保持严格约束：缺少定位键 (Frame/Burst/LatSlot) 时返回 null；
 * 无轨道号的光学及其他多光谱/产品按 MGRS -> WRS-2 -> 0.2° 空间覆盖外包框归组。
 */
export function stackKey(row?: StackableRowInput | null): string | null {
  if (!row) return null;
  const meta = recordObject(row.native_metadata);
  const metaTrack = meta && typeof meta.track === "number" ? meta.track : null;
  const metaBurstId =
    meta && typeof meta.burst_id === "string"
      ? meta.burst_id
      : meta && typeof meta.burstId === "string"
        ? meta.burstId
        : null;
  const orbit = [row.relative_orbit, row.path, row.track, metaTrack].find(
    (v): v is number => typeof v === "number" && Number.isFinite(v),
  );
  if (orbit !== undefined) {
    let locKey: string | null = null;
    if (typeof row.frame === "number" && Number.isFinite(row.frame)) {
      locKey = String(row.frame);
    } else if (typeof row.burst_id === "string" && row.burst_id.trim()) {
      locKey = row.burst_id.trim();
    } else if (typeof metaBurstId === "string" && metaBurstId.trim()) {
      locKey = metaBurstId.trim();
    } else if (
      row.footprint_bbox &&
      Number.isFinite(row.footprint_bbox.south) &&
      Number.isFinite(row.footprint_bbox.north)
    ) {
      const centerLat = (row.footprint_bbox.south + row.footprint_bbox.north) / 2;
      locKey = `L${Math.round(centerLat * 10)}`;
    }

    if (locKey === null) return null;

    const dirRaw = typeof row.orbit_direction === "string" ? row.orbit_direction.trim().toUpperCase() : "";
    const dir = dirRaw.startsWith("A") ? "ASC" : dirRaw.startsWith("D") ? "DESC" : "";
    return dir ? `${dir}:${orbit}:${locKey}` : `${orbit}:${locKey}`;
  }

  const mgrs = extractMgrsTile(row, meta);
  if (mgrs) return `MGRS:${mgrs}`;

  const wrs = extractWrsPathRow(row, meta);
  if (wrs) return `WRS:${wrs.path}:${wrs.row}`;

  const bbox = extractSpatialBboxFallback(row);
  if (bbox) {
    const q = (v: number) => (Math.round(v * 5) / 5).toFixed(1);
    return `BBOX:${q(bbox.west)}:${q(bbox.south)}:${q(bbox.east)}:${q(bbox.north)}`;
  }

  return null;
}

export function describeSceneStack(row: StackableRowInput): {
  groupLabel: string;
  shortTag: string;
} {
  const meta = recordObject(row.native_metadata);
  const metaTrack = meta && typeof meta.track === "number" ? meta.track : null;
  const metaBurstId =
    meta && typeof meta.burst_id === "string"
      ? meta.burst_id
      : meta && typeof meta.burstId === "string"
        ? meta.burstId
        : null;
  const orbit = [row.relative_orbit, row.path, row.track, metaTrack].find(
    (v): v is number => typeof v === "number" && Number.isFinite(v),
  );

  if (orbit !== undefined) {
    const dirRaw = typeof row.orbit_direction === "string" ? row.orbit_direction.trim().toUpperCase() : "";
    const dirLabel = dirRaw.startsWith("A")
      ? "Ascending (ASC)"
      : dirRaw.startsWith("D")
        ? "Descending (DESC)"
        : "Orbit";
    const parts = [dirLabel, `Path/Orbit ${orbit}`];
    let shortTag = `P${orbit}`;
    if (typeof row.frame === "number" && Number.isFinite(row.frame)) {
      parts.push(`Frame ${row.frame}`);
      shortTag = `P${orbit}/F${row.frame}`;
    } else if (typeof row.burst_id === "string" && row.burst_id.trim()) {
      parts.push(`Burst ${row.burst_id.trim()}`);
      shortTag = `P${orbit}/${row.burst_id.trim()}`;
    } else if (metaBurstId?.trim()) {
      parts.push(`Burst ${metaBurstId.trim()}`);
      shortTag = `P${orbit}/${metaBurstId.trim()}`;
    } else {
      const k = stackKey(row);
      if (k && k.includes(":L")) {
        parts.push(`Slot ${k.split(":").pop()}`);
      }
    }
    return { groupLabel: parts.join(" · "), shortTag };
  }

  const mgrs = extractMgrsTile(row, meta);
  if (mgrs) {
    return {
      groupLabel: `MGRS 图幅 ${mgrs}`,
      shortTag: mgrs,
    };
  }

  const wrs = extractWrsPathRow(row, meta);
  if (wrs) {
    return {
      groupLabel: `WRS-2 条带 ${wrs.path} / ${wrs.row}`,
      shortTag: `${wrs.path}/${wrs.row}`,
    };
  }

  const bbox = extractSpatialBboxFallback(row);
  if (bbox) {
    const cLon = ((bbox.west + bbox.east) / 2).toFixed(2);
    const cLat = ((bbox.south + bbox.north) / 2).toFixed(2);
    return {
      groupLabel: `同幅覆盖区 (${cLon}°E, ${cLat}°N)`,
      shortTag: "同幅",
    };
  }

  return { groupLabel: "其他影像", shortTag: "" };
}

/**
 * 从场景列表中构建分堆映射。
 * 仅包含成员数量 >= 2 的堆。
 */
export function buildStacks(rows: SceneRow[]): Map<string, string[]> {
  const candidateMap = new Map<string, string[]>();
  for (const r of rows) {
    const k = stackKey(r);
    if (!k) continue;
    const list = candidateMap.get(k);
    if (list) {
      list.push(r.scene_id);
    } else {
      candidateMap.set(k, [r.scene_id]);
    }
  }
  const result = new Map<string, string[]>();
  for (const [k, list] of candidateMap.entries()) {
    if (list.length >= 2) {
      result.set(k, list);
    }
  }
  return result;
}

/**
 * 查询指定场景所在的分堆场景 ID 列表。
 * 若所属分堆成员 >= 2，返回分堆所有场景 ID；否则返回仅包含该 sceneId 自身的数组。
 */
export function stackFor(rows: SceneRow[], sceneId: string): string[] {
  const target = rows.find((r) => r.scene_id === sceneId);
  if (!target) return [sceneId];
  const k = stackKey(target);
  if (!k) return [sceneId];
  const matching = rows.filter((r) => stackKey(r) === k).map((r) => r.scene_id);
  return matching.length >= 2 ? matching : [sceneId];
}

/**
 * 查询指定场景在排序列表中的页码（1-based）。
 * 若不存在返回 null。
 */
export function pageForScene(
  sortedIds: string[],
  sceneId: string,
  pageSize = RESULT_PAGE_SIZE
): number | null {
  const index = sortedIds.indexOf(sceneId);
  if (index === -1) return null;
  return Math.floor(index / pageSize) + 1;
}
