/**
 * SnapshotItem → 真实展示行投影
 * 审计 P0-02 防线：
 * 只展示 metadata / payload 里权威存在的真实字段；
 * 缺失字段诚实回退为 null / "未知" / undefined，严禁伪造 relative_orbit、frame、file_size、orbit_direction 等！
 * 长文件名按视觉基线 §4b-6 规则中间截断，保留前缀与扩展名后缀，悬停显示全名。
 */

import type { SnapshotItem, RowTaskState, RowAction } from "./types.ts";
import { t } from "../../i18n/index.ts";
import type { SceneRow } from "../../lib/bridge.ts";

/**
 * 长文件名中间截断（视觉基线 §4b-6）
 * 保留前部特征和末尾唯一标识及扩展名
 */
export function truncateMiddle(name: string, headChars = 20, tailChars = 14): string {
  if (!name || name.length <= headChars + tailChars + 3) return name;
  return `${name.slice(0, headChars)}...${name.slice(-tailChars)}`;
}

export function formatBytes(bytes?: number | null): string {
  if (bytes === undefined || bytes === null || Number.isNaN(bytes) || bytes < 0) return t("mapWorkbench.row.unknown");
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const val = (bytes / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 1);
  return `${val} ${units[i]}`;
}

/** 科学属性里的数值：有限数字，或可完整解析为整数的数字字符串；其他一律视为缺失。 */
function propertyNumber(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value === "string" && /^\d+$/.test(value.trim())) return Number.parseInt(value.trim(), 10);
  return null;
}

function propertyText(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function opticalPlatformLabel(platform: string): string {
  const normalized = platform.trim().toLowerCase().replaceAll("-", "_");
  const sentinel = /^(?:sentinel_?2|s2)([abc])$/.exec(normalized);
  if (sentinel) return `Sentinel-2${sentinel[1].toUpperCase()}`;
  const landsat = /^(?:landsat_?|lc0?|le0?|lt0?|lo0?)(\d)$/.exec(normalized);
  if (landsat) return `Landsat ${landsat[1]}`;
  return platform.trim();
}

export function opticalTileLabel(scene: SceneRow): string | null {
  const meta = (scene.native_metadata || {}) as Record<string, unknown>;
  const rawProps = meta.properties;
  const props = rawProps && typeof rawProps === "object" && !Array.isArray(rawProps)
    ? rawProps as Record<string, unknown>
    : {};
  const wrsPath = propertyText(props.wrs_path) ?? propertyText(props["landsat:wrs_path"]);
  const wrsRow = propertyText(props.wrs_row) ?? propertyText(props["landsat:wrs_row"]);
  if (wrsPath && wrsRow) return `WRS ${wrsPath}/${wrsRow}`;

  const displayName = scene.file_name || scene.scene_id;
  const tile =
    propertyText(props.mgrs_tile) ??
    propertyText(props["mgrs:tile"]) ??
    propertyText(props.tile_id) ??
    /_(T\d{2}[A-Z]{3})_/.exec(displayName)?.[1] ??
    /^HLS\.[LS]30\.(T\d{2}[A-Z]{3})\./.exec(displayName)?.[1] ??
    null;
  return tile ? `Tile ${tile}` : null;
}

function snapshotSize(item: SnapshotItem, assets: unknown[]): number | null {
  const access = (item as SnapshotItem & { asset_access?: unknown }).asset_access;
  if (access && typeof access === "object" && !Array.isArray(access)) {
    const expected = (access as Record<string, unknown>).expected_size_bytes;
    if (propertyNumber(expected) !== null) return propertyNumber(expected);
  }
  for (const candidate of assets) {
    if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) continue;
    const size = propertyNumber((candidate as Record<string, unknown>).size_bytes);
    if (size !== null) return size;
  }
  return null;
}

function isOpaqueProviderIdentifier(value: string): boolean {
  const s = value.trim();
  if (!s) return true;
  if (/^[a-z0-9_-]+(\.[a-z0-9_-]+){1,}$/i.test(s)) return true;
  return false;
}

function derivePlatformFromHint(hint: string): string | null {
  const u = hint.trim().toUpperCase();
  if (!u) return null;
  if (u.includes("NISAR") || u.includes("C2853145197-ASF") || u.includes("C3622236985-ASF")) {
    return "NISAR";
  }
  if (/^S1[ABCD]_/.test(u) || u.includes("SENTINEL-1") || u.includes("SENTINEL_1")) {
    return "Sentinel-1";
  }
  const s2 = /^(?:SENTINEL[_-]?2|S2)([ABC])\b/.exec(u);
  if (s2) return `Sentinel-2${s2[1]}`;
  if (u.includes("SENTINEL-2")) return "Sentinel-2";
  if (/^ALOS2/.test(u) || u.includes("ALOS-2") || u.includes("PALSAR-2") || u.includes("C3315903479-ASF")) {
    return "ALOS-2";
  }
  if (/^ALPSRP/.test(u) || u.includes("ALOS.PALSAR")) return "ALOS PALSAR";
  if (u.includes("OPERA_L") || u.includes("C2834279090-ASF")) return "OPERA";
  if (u.includes("S1-GUNW") || u.includes("C1595771383-ASF")) return "Sentinel-1";
  const ls = /^L[CETM]0?(\d)/.exec(u);
  if (ls) return `Landsat ${ls[1]}`;
  return null;
}

export function projectSnapshotItemToSceneRow(item: SnapshotItem): SceneRow {
  const meta = (item.metadata || {}) as Record<string, unknown>;
  const payload = (item.payload || {}) as Record<string, unknown>;
  // 科学属性集（NISAR 目录映射器把 flight_direction/track/relative_orbit/frame 放在这里，不在顶层）
  const props = (
    meta.properties !== null && typeof meta.properties === "object" && !Array.isArray(meta.properties)
      ? meta.properties
      : {}
  ) as Record<string, unknown>;

  // 1. 轨道方向：优先读取真实 flight_direction，兼容读取权威存在的 orbit_direction / Node，不存在绝不编造
  let orbitDirection = "";
  const rawFlightDir = typeof meta.flight_direction === "string" ? meta.flight_direction : (typeof payload.flight_direction === "string" ? payload.flight_direction : "");
  const rawOrbitDir =
    rawFlightDir ||
    (typeof meta.orbit_direction === "string" ? meta.orbit_direction : (typeof payload.orbit_direction === "string" ? payload.orbit_direction : "")) ||
    (typeof props.flight_direction === "string" ? props.flight_direction : "") ||
    (typeof props.Node === "string" ? props.Node : (typeof props.node === "string" ? props.node : ""));
  if (rawOrbitDir.toLowerCase().includes("asc")) {
    orbitDirection = "ascending";
  } else if (rawOrbitDir.toLowerCase().includes("desc")) {
    orbitDirection = "descending";
  }

  // 2. 相对轨道与分幅号：仅在存在真实数值时提取，严禁编造 128 或 498！
  const rawRelOrbit = meta.relative_orbit ?? payload.relative_orbit;
  const relativeOrbit =
    typeof rawRelOrbit === "number" && !Number.isNaN(rawRelOrbit)
      ? rawRelOrbit
      : (propertyNumber(props.relative_orbit) ?? propertyNumber(props.track) ?? propertyNumber(props.Track));

  const rawPath = meta.path ?? payload.path;
  const path =
    typeof rawPath === "number" && !Number.isNaN(rawPath)
      ? rawPath
      : (propertyNumber(props.Track) ?? propertyNumber(props.track));

  const rawFrame = meta.frame ?? payload.frame;
  const frame =
    typeof rawFrame === "number" && !Number.isNaN(rawFrame)
      ? rawFrame
      : (propertyNumber(props.frame) ?? propertyNumber(props.Frame));

  const rawAbsOrbit = meta.absolute_orbit ?? payload.absolute_orbit;
  const absoluteOrbit =
    typeof rawAbsOrbit === "number" && !Number.isNaN(rawAbsOrbit)
      ? rawAbsOrbit
      : (propertyNumber(props.absolute_orbit) ?? propertyNumber(props.Imaging_Orbit) ?? propertyNumber(props.imaging_orbit));

  // 3. 极化模式与波束模式
  const polarization =
    (typeof meta.polarization === "string" ? meta.polarization : "") ||
    (typeof payload.polarization === "string" ? payload.polarization : "") ||
    propertyText(props.polarization) ||
    propertyText(props.Polarization) ||
    "";
  const beamMode =
    (typeof meta.beam_mode === "string" ? meta.beam_mode : "") ||
    (typeof payload.beam_mode === "string" ? payload.beam_mode : "") ||
    propertyText(props.beam_mode) ||
    propertyText(props.mode) ||
    propertyText(props.Mode) ||
    "";
  const productType = typeof meta.product_type === "string" ? meta.product_type : (typeof payload.product_type === "string" ? payload.product_type : "");

  // 4. 文件远程大小
  const rawSize = meta.size_bytes ?? payload.size_bytes ?? meta.file_size_remote ?? payload.file_size_remote;
  const remoteSize = propertyNumber(rawSize);

  const rawAssets = Array.isArray(meta.assets)
    ? meta.assets
    : (Array.isArray(payload.assets) ? payload.assets : []);
  const isOptical = meta.product_kind === "optical_scene";
  const namePlatform = /^(S2[ABC])_/.exec(item.display_name)?.[1] ?? null;
  const candidateRawPlatform = propertyText(meta.platform) ?? propertyText(payload.platform) ??
    propertyText(props.platform) ?? propertyText(props.spacecraft) ?? (isOptical ? namePlatform : null);
  const rawPlatform = candidateRawPlatform && !isOpaqueProviderIdentifier(candidateRawPlatform)
    ? candidateRawPlatform
    : null;
  const inferredHintPlatform =
    derivePlatformFromHint(item.display_name) ??
    derivePlatformFromHint(item.item_id) ??
    derivePlatformFromHint(propertyText(meta.resource_id) ?? propertyText(payload.resource_id) ?? "") ??
    derivePlatformFromHint(propertyText(meta.dataset) ?? propertyText(props.dataset) ?? "") ??
    derivePlatformFromHint(propertyText(props.collection_short_name) ?? propertyText(props.short_name) ?? "");
  const fallbackProviderLabel =
    item.provider_id === "asf.nasa.earthdata" || item.provider_id === "asf.catalog"
      ? "ASF SAR"
      : isOpaqueProviderIdentifier(item.provider_id)
        ? t("mapWorkbench.row.unknown")
        : item.provider_id;
  const platform = isOptical && rawPlatform
    ? opticalPlatformLabel(rawPlatform)
    : rawPlatform ?? inferredHintPlatform ?? fallbackProviderLabel;
  const rawProductType = propertyText(props.product_type);
  const processingLevel = propertyText(meta.processing_level) ?? propertyText(payload.processing_level) ??
    (isOptical ? propertyText(props.processing_level) : null);

  const nativeMetadata: Record<string, unknown> = { ...meta };
  for (const key of [
    "release_tier",
    "tier",
    "collection_concept_id",
    "collection_short_name",
    "short_name",
  ] as const) {
    if (nativeMetadata[key] === undefined && payload[key] !== undefined) {
      nativeMetadata[key] = payload[key];
    }
  }

  const rawMetaBbox = meta.footprint_bbox as Record<string, unknown> | null | undefined;
  const footprintBbox =
    rawMetaBbox &&
    typeof rawMetaBbox === "object" &&
    typeof rawMetaBbox.west === "number" &&
    typeof rawMetaBbox.south === "number" &&
    typeof rawMetaBbox.east === "number" &&
    typeof rawMetaBbox.north === "number" &&
    Number.isFinite(rawMetaBbox.west) &&
    Number.isFinite(rawMetaBbox.south) &&
    Number.isFinite(rawMetaBbox.east) &&
    Number.isFinite(rawMetaBbox.north)
      ? {
          west: rawMetaBbox.west,
          south: rawMetaBbox.south,
          east: rawMetaBbox.east,
          north: rawMetaBbox.north,
          crs: typeof rawMetaBbox.crs === "string" ? rawMetaBbox.crs : "EPSG:4326",
        }
      : undefined;

  return {
    scene_id: item.item_id,
    product_key: item.item_id,
    product_id: item.item_id,
    provider: item.provider_id,
    file_name: item.display_name,
    acquisition_datetime: item.acquisition_datetime ?? "",
    ...(footprintBbox ? { footprint_bbox: footprintBbox } : {}),
    footprint_geojson: item.footprint_geojson as any,
    platform,
    processing_level: processingLevel,
    product_type: isOptical ? rawProductType ?? productType : productType,
    beam_mode: beamMode,
    polarization,
    orbit_direction: orbitDirection,
    relative_orbit: relativeOrbit,
    frame,
    absolute_orbit: absoluteOrbit,
    path,
    file_size_remote: remoteSize ?? snapshotSize(item, rawAssets),
    has_url: false,
    assets: rawAssets as any,
    native_metadata: nativeMetadata as any,
  };
}

/**
 * 结果卡行内权威操作映射（项 2）
 * 严格按照 RowTaskState 切换：
 * - running -> ["pause"]
 * - paused -> ["resume"]
 * - failed -> [] (结果卡不设重检入口，重试/重新检索后重试留在抽屉任务卡)
 * - cancelled / completed / interrupted -> []
 * - queued -> 原样返回后端 actions
 * - rowState 缺失或 task_status 为 null/undefined -> ["download"] (现状保留)
 */
export function deriveVisibleRowActions(rowState?: RowTaskState | null): RowAction[] {
  if (!rowState || !rowState.task_status) {
    return ["download"];
  }

  switch (rowState.task_status) {
    case "running":
      return ["pause"];
    case "paused":
      return ["resume"];
    case "failed":
      return [];
    case "cancelled":
    case "completed":
    case "interrupted":
      return [];
    case "queued":
      return rowState.actions ? [...rowState.actions] : [];
    default:
      return ["download"];
  }
}

/**
 * 格式化轨道飞行方向完整英文显示（Owner 2026-10-25 裁定）
 */
export function formatOrbitDirection(direction?: string | null): string {
  if (!direction) return "Unknown";
  const lower = direction.trim().toLowerCase();
  if (lower.includes("asc")) return "Ascending";
  if (lower.includes("desc")) return "Descending";
  return direction.trim() || "Unknown";
}

/**
 * 景在列表中的名称：文件名；没有文件名时为「未命名」，不退回 provider ID。
 */
export function sceneListLabel(scene: { file_name?: string | null }): string {
  const name = typeof scene.file_name === "string" ? scene.file_name.trim() : "";
  return name || t("mapWorkbench.row.unnamed");
}

/**
 * 格式化轨道飞行方向紧凑徽章显示（结果卡片右上状态）
 */
export function formatOrbitBadge(direction?: string | null): string {
  if (!direction) return "Unknown";
  const lower = direction.trim().toLowerCase();
  if (lower.includes("asc")) return "ASC";
  if (lower.includes("desc")) return "DESC";
  return direction.trim() || "Unknown";
}

/**
 * 格式化轨道与分幅元数据信息行
 * 示例："Ascending · Relative orbit 11 · Frame 65"
 * 或："Descending · Path 42 · Frame 100"
 */
export function formatOrbitFrameLine(
  orbitDirection?: string | null,
  relativeOrbit?: number | null,
  path?: number | null,
  frame?: number | null
): string {
  const dirStr = formatOrbitDirection(orbitDirection);

  let orbitStr = "Path Unknown";
  if (relativeOrbit !== undefined && relativeOrbit !== null && !Number.isNaN(relativeOrbit)) {
    orbitStr = `Path ${relativeOrbit}`;
  } else if (path !== undefined && path !== null && !Number.isNaN(path)) {
    orbitStr = `Path ${path}`;
  }

  let frameStr = "Frame Unknown";
  if (frame !== undefined && frame !== null && !Number.isNaN(frame)) {
    frameStr = `Frame ${frame}`;
  }

  return `${dirStr} · ${orbitStr} · ${frameStr}`;
}

export interface ResultRowBadge {
  key: string;
  label?: string;
  value: string;
}

/**
 * 提取光学真实云量数字（W5-3a-F2）
 * 权威依据：
 * - MPC Sentinel-2: crates/insar-application-service/src/lib.rs:16073-16075, 16171
 * - CDSE Sentinel-2: crates/insar-application-service/src/lib.rs:12340-12346, 16395
 * - HLS (L30/S30): crates/insar-application-service/src/hls_catalogue.rs:62-67, 134
 * - Landsat C2 L2: crates/insar-application-service/src/lib.rs:13587-13591
 * 真实键名恒为 "cloud_cover"，存放于 metadata.properties.cloud_cover（兼容顶层 metadata.cloud_cover）
 */
export function extractCloudCover(scene: SceneRow): number | null {
  const meta = (scene.native_metadata || {}) as Record<string, unknown>;
  const props = (meta.properties || {}) as Record<string, unknown>;
  const val = props.cloud_cover ?? meta.cloud_cover;
  if (
    typeof val === "number" &&
    !Number.isNaN(val) &&
    Number.isFinite(val) &&
    val >= 0 &&
    val <= 100
  ) {
    return val;
  }
  return null;
}

/**
 * 结果行徽章构建器（W5-3b 规范对齐样稿，W5-3a-F2 分族支持）
 * - SAR 族：关键字段（日期、方向、Relative orbit、Frame）始终展示，缺失回退为「未知」（方向回退为 "Unknown"）；
 * - 光学族：显示日期、平台、云量（仅限真实数值）、级别、大小，绝不展示 Relative orbit 未知 / Frame 未知 / Unknown。
 */
export function buildResultRowBadges(
  scene: SceneRow,
  family?: string | null
): ResultRowBadge[] {
  const badges: ResultRowBadge[] = [];

  // 1. 日期 (YYYY-MM-DD，缺失回退「未知」，无 label)
  let dateVal = t("mapWorkbench.row.unknown");
  if (scene.acquisition_datetime && scene.acquisition_datetime.trim()) {
    const d = scene.acquisition_datetime.trim().split("T")[0];
    if (d) dateVal = d;
  }
  badges.push({ key: "date", label: "", value: dateVal });

  // 光学数据源族分支
  if (family === "optical") {
    const meta = (scene.native_metadata || {}) as Record<string, unknown>;
    const props = (meta.properties || {}) as Record<string, unknown>;

    // 2. 平台 (从 metadata 提取)
    const plat =
      (typeof scene.platform === "string" && scene.platform.trim()) ||
      (typeof props.platform === "string" && props.platform.trim()) ||
      (typeof meta.platform === "string" && meta.platform.trim()) ||
      "";
    if (plat) {
      badges.push({ key: "platform", label: "", value: opticalPlatformLabel(plat) });
    }

    // 3. 云量 ("云量 N%"，仅当携带真实数字时显示)
    const cloudVal = extractCloudCover(scene);
    if (cloudVal !== null) {
      // 最多一位小数：99.998516 → 100，32.448849 → 32.4
      const cloudText = String(Math.round(cloudVal * 10) / 10);
      badges.push({ key: "cloud", label: "", value: t("mapWorkbench.row.cloudCover", { value: cloudText }) });
    }

    // 4. 级别 / 产品类型
    const procLevel = scene.processing_level ? scene.processing_level.trim() : "";
    const prodType = scene.product_type ? scene.product_type.trim() : "";
    const levelVal = procLevel || prodType;
    if (levelVal) {
      badges.push({ key: "level", label: "", value: levelVal });
    }

    // 5. 大小
    if (
      typeof scene.file_size_remote === "number" &&
      !Number.isNaN(scene.file_size_remote) &&
      Number.isFinite(scene.file_size_remote) &&
      scene.file_size_remote >= 0
    ) {
      badges.push({ key: "size", label: "", value: formatBytes(scene.file_size_remote) });
    }

    // 绝不展示 Relative orbit 未知 / Frame 未知 / Unknown
    return badges;
  }

  // 2. 方向 (Ascending / Descending / Unknown，无 label)
  const dirVal = formatOrbitDirection(scene.orbit_direction);
  badges.push({ key: "direction", label: "", value: dirVal });

  // 3. 轨道 (Relative orbit N / Path N / Relative orbit 未知，无 label)
  let orbitVal = t("mapWorkbench.row.relativeOrbitUnknown");
  if (scene.relative_orbit !== undefined && scene.relative_orbit !== null && !Number.isNaN(scene.relative_orbit)) {
    orbitVal = `Path ${scene.relative_orbit}`;
  } else if (scene.path !== undefined && scene.path !== null && !Number.isNaN(scene.path)) {
    orbitVal = `Path ${scene.path}`;
  }
  badges.push({ key: "orbit", label: "", value: orbitVal });

  // 4. 分幅 (Frame N / Frame 未知，无 label)
  let frameVal = t("mapWorkbench.row.frameUnknown");
  if (scene.frame !== undefined && scene.frame !== null && !Number.isNaN(scene.frame)) {
    frameVal = `Frame ${scene.frame}`;
  }
  badges.push({ key: "frame", label: "", value: frameVal });

  // 5. 极化 (次要字段：无 label 直接展示值如 VV+VH，缺失整枚省略)
  if (scene.polarization && scene.polarization.trim()) {
    badges.push({ key: "polarization", label: "", value: scene.polarization.trim() });
  }

  // 6. 级别 (次要字段：processing_level 优先，product_type 次之，无 label，缺失整枚省略)
  const procLevel = scene.processing_level ? scene.processing_level.trim() : "";
  const prodType = scene.product_type ? scene.product_type.trim() : "";
  const levelVal = procLevel || prodType;
  if (levelVal) {
    badges.push({ key: "level", label: "", value: levelVal });
  }

  // 7. 大小 (次要字段：经 formatBytes，无 label，缺失或无效整枚省略)
  if (
    typeof scene.file_size_remote === "number" &&
    !Number.isNaN(scene.file_size_remote) &&
    Number.isFinite(scene.file_size_remote) &&
    scene.file_size_remote >= 0
  ) {
    badges.push({ key: "size", label: "", value: formatBytes(scene.file_size_remote) });
  }

  return badges;
}
