import type { BuildSearchRequestParams } from "./searchRequest";
import { t } from "../../i18n/index.ts";
import type { SourceDescriptor } from "./types";
import type { Bbox } from "../../lib/bridge";
import { explainActionError, explainErrorCode, lookupErrorText } from "./errorExplanations.ts";

export type ManualConditionsGroup = "source" | "aoi" | "time" | "orbit";

export interface ManualConditionsState {
  resourceId?: string;
  activeResourceId?: string;
  aoiGeometry?: Record<string, unknown> | null;
  aoiName?: string;
  startDate?: string | null;
  endDate?: string | null;
  platforms?: string[];
  productType?: string | null;
  polarizations?: string[];
  beamModes?: string[];
  lookDirection?: string | null;
  burstStrategy?: "all" | "primary" | null;
  orbitDirection?: string | null;
  relativeOrbit?: number | string | null;
  path?: number | string | null;
  frame?: number | string | null;
  relativeOrbitProfile?: { min?: number | null; max?: number | null } | null;
  pathProfile?: { min?: number | null; max?: number | null } | null;
  frameProfile?: {
    valueType: string;
    exactWidth?: number | null;
    min?: number | null;
    max?: number | null;
  } | null;
}

export interface ConditionValidationError {
  field: string;
  message: string;
}

/**
 * 转换条件状态为 BuildSearchRequestParams，严格不产生超出契约的字段
 */
export function conditionsToSearchParams(state: ManualConditionsState): BuildSearchRequestParams {
  const resourceId = state.resourceId ?? state.activeResourceId ?? "";
  const aoiGeometry = state.aoiGeometry ?? null;
  const aoiName = state.aoiName ?? "";
  const startDate = state.startDate ?? null;
  const endDate = state.endDate ?? null;
  const productType = state.productType ?? null;

  let orbitDirection: string | null = null;
  if (state.orbitDirection && state.orbitDirection !== "all") {
    orbitDirection = state.orbitDirection;
  }

  let relativeOrbit: number | null = null;
  if (state.relativeOrbit !== undefined && state.relativeOrbit !== null) {
    if (typeof state.relativeOrbit === "number") {
      if (Number.isInteger(state.relativeOrbit)) {
        relativeOrbit = state.relativeOrbit;
      }
    } else {
      const trimmed = String(state.relativeOrbit).trim();
      if (trimmed !== "" && /^-?\d+$/.test(trimmed)) {
        relativeOrbit = parseInt(trimmed, 10);
      }
    }
  }

  let frame: number | null = null;
  if (state.frame !== undefined && state.frame !== null) {
    if (typeof state.frame === "number") {
      if (Number.isInteger(state.frame)) {
        frame = state.frame;
      }
    } else {
      const trimmed = String(state.frame).trim();
      if (trimmed !== "" && /^-?\d+$/.test(trimmed)) {
        frame = parseInt(trimmed, 10);
      }
    }
  }

  return {
    resourceId,
    aoiGeometry,
    aoiName,
    startDate,
    endDate,
    platforms: state.platforms,
    productType,
    polarizations: state.polarizations,
    ...(state.beamModes !== undefined ? { beamModes: state.beamModes } : {}),
    ...(state.lookDirection !== undefined
      ? {
          lookDirection:
            state.lookDirection && state.lookDirection !== "all"
              ? state.lookDirection
              : null,
        }
      : {}),
    orbitDirection,
    relativeOrbit,
    path: state.path,
    frame,
    ...(state.burstStrategy !== undefined ? { burstStrategy: state.burstStrategy } : {}),
  };
}

export interface RestoredBatchConditions {
  polarizations: string[];
  beamModes: string[];
  lookDirection: string | null;
  burstStrategy: "all" | "primary" | null;
}

function normalizeStringList(raw: unknown): string[] {
  if (Array.isArray(raw)) {
    return raw
      .filter((v): v is string => typeof v === "string")
      .map((v) => v.trim())
      .filter((v) => v.length > 0);
  }
  if (typeof raw === "string") {
    return raw
      .split(",")
      .map((v) => v.trim())
      .filter((v) => v.length > 0);
  }
  return [];
}

/**
 * Issue #8 尾项：从冷启动已保存批次查询（或其 filters 子结构）中提取
 * 极化（polarizations）、波束模式（beamModes）、视向（lookDirection）与 Burst 策略（burstStrategy）
 */
export function restoreConditionsFromBatchQuery(
  query: Record<string, unknown> | null | undefined
): RestoredBatchConditions {
  if (!query || typeof query !== "object") {
    return {
      polarizations: [],
      beamModes: [],
      lookDirection: null,
      burstStrategy: null,
    };
  }
  const filters =
    query.filters && typeof query.filters === "object"
      ? (query.filters as Record<string, unknown>)
      : null;

  const rawPolarizations =
    query.polarizations ??
    query.polarization ??
    query.selected_polarizations ??
    filters?.polarizations ??
    filters?.polarization ??
    filters?.selected_polarizations;
  const polarizations = normalizeStringList(rawPolarizations);

  const rawBeamModes =
    query.beam_modes ??
    query.beamModes ??
    query.beam_mode ??
    query.beamMode ??
    filters?.beam_modes ??
    filters?.beamModes ??
    filters?.beam_mode ??
    filters?.beamMode;
  const beamModes = normalizeStringList(rawBeamModes);

  const rawLookDirection =
    query.look_direction ??
    query.lookDirection ??
    filters?.look_direction ??
    filters?.lookDirection;
  const lookDirection =
    typeof rawLookDirection === "string" && rawLookDirection.trim().length > 0
      ? rawLookDirection.trim().toUpperCase()
      : null;

  const burstObj =
    (query.burst && typeof query.burst === "object"
      ? (query.burst as Record<string, unknown>)
      : null) ??
    (filters?.burst && typeof filters.burst === "object"
      ? (filters.burst as Record<string, unknown>)
      : null);
  const rawBurstStrategy =
    query.burst_asset_policy ??
    query.burstAssetPolicy ??
    query.burst_strategy ??
    query.burstStrategy ??
    query.asset_policy ??
    filters?.burst_asset_policy ??
    filters?.burstAssetPolicy ??
    filters?.burst_strategy ??
    filters?.burstStrategy ??
    filters?.asset_policy ??
    burstObj?.asset_policy ??
    burstObj?.burst_asset_policy;
  let burstStrategy: "all" | "primary" | null = null;
  if (typeof rawBurstStrategy === "string") {
    const normalized = rawBurstStrategy.trim().toLowerCase();
    if (normalized === "all" || normalized === "all_files" || normalized === "full") {
      burstStrategy = "all";
    } else if (
      normalized === "primary" ||
      normalized === "primary_only" ||
      normalized === "tiff_only" ||
      normalized === "data_only"
    ) {
      burstStrategy = "primary";
    }
  }

  return {
    polarizations,
    beamModes,
    lookDirection,
    burstStrategy,
  };
}

/**
 * 校验输入有效性，返回以字段为键的中文错误列表
 */
export function validateConditions(state: ManualConditionsState): ConditionValidationError[] {
  const errors: ConditionValidationError[] = [];

  if (state.relativeOrbit !== undefined && state.relativeOrbit !== null) {
    const min = state.relativeOrbitProfile?.min ?? null;
    const max = state.relativeOrbitProfile?.max ?? null;
    if (typeof state.relativeOrbit === "number") {
      if (!Number.isInteger(state.relativeOrbit)) {
        errors.push({ field: "relativeOrbit", message: t("mapWorkbench.helpers.relativeOrbitInteger") });
      } else if ((min !== null && state.relativeOrbit < min) || (max !== null && state.relativeOrbit > max)) {
        if (min !== null && max !== null) {
          errors.push({
            field: "relativeOrbit",
            message: t("mapWorkbench.helpers.relativeOrbitOutOfRange", { min, max }),
          });
        }
      }
    } else {
      const trimmed = String(state.relativeOrbit).trim();
      if (trimmed !== "") {
        if (!/^-?\d+$/.test(trimmed)) {
          errors.push({ field: "relativeOrbit", message: t("mapWorkbench.helpers.relativeOrbitInteger") });
        } else {
          const num = parseInt(trimmed, 10);
          if ((min !== null && num < min) || (max !== null && num > max)) {
            if (min !== null && max !== null) {
              errors.push({
                field: "relativeOrbit",
                message: t("mapWorkbench.helpers.relativeOrbitOutOfRange", { min, max }),
              });
            }
          }
        }
      }
    }
  }

  if (state.path !== undefined && state.path !== null) {
    const min = state.pathProfile?.min ?? null;
    const max = state.pathProfile?.max ?? null;
    if (typeof state.path === "number") {
      if (!Number.isInteger(state.path)) {
        errors.push({ field: "path", message: t("mapWorkbench.helpers.pathInteger") });
      } else if ((min !== null && state.path < min) || (max !== null && state.path > max)) {
        if (min !== null && max !== null) {
          errors.push({
            field: "path",
            message: t("mapWorkbench.helpers.pathOutOfRange", { min, max }),
          });
        }
      }
    } else {
      const trimmed = String(state.path).trim();
      if (trimmed !== "") {
        if (!/^-?\d+$/.test(trimmed)) {
          errors.push({ field: "path", message: t("mapWorkbench.helpers.pathInteger") });
        } else {
          const num = parseInt(trimmed, 10);
          if ((min !== null && num < min) || (max !== null && num > max)) {
            if (min !== null && max !== null) {
              errors.push({
                field: "path",
                message: t("mapWorkbench.helpers.pathOutOfRange", { min, max }),
              });
            }
          }
        }
      }
    }
  }

  if (state.frame !== undefined && state.frame !== null) {
    const trimmed = String(state.frame).trim();
    if (trimmed !== "") {
      if (state.frameProfile?.exactWidth) {
        const width = state.frameProfile.exactWidth;
        if (!/^\d+$/.test(trimmed) || trimmed.length !== width) {
          errors.push({ field: "frame", message: t("mapWorkbench.helpers.frameDigits", { width }) });
        }
      } else {
        if (!/^-?\d+$/.test(trimmed)) {
          errors.push({ field: "frame", message: t("mapWorkbench.helpers.frameInteger") });
        } else {
          const num = parseInt(trimmed, 10);
          const min = state.frameProfile?.min ?? null;
          const max = state.frameProfile?.max ?? null;
          if ((min !== null && num < min) || (max !== null && num > max)) {
            if (min !== null && max !== null) {
              errors.push({
                field: "frame",
                message: t("mapWorkbench.helpers.frameOutOfRange", { min, max }),
              });
            }
          }
        }
      }
    }
  }

  return errors;
}

export type QuickRangeKind = "3m" | "6m" | "1y" | "3y" | "5y";

/**
 * 计算近3月/近半年/近1年/近3年/近5年的起止日期（本地日历，月末截断）
 */
export function quickRange(
  kind: QuickRangeKind,
  now: Date
): { startDate: string; endDate: string } {
  const endY = now.getFullYear();
  const endM = String(now.getMonth() + 1).padStart(2, "0");
  const endD = String(now.getDate()).padStart(2, "0");
  const endDate = `${endY}-${endM}-${endD}`;

  const monthsToSubtract =
    kind === "3m"
      ? 3
      : kind === "6m"
        ? 6
        : kind === "1y"
          ? 12
          : kind === "3y"
            ? 36
            : 60;
  const totalMonths = now.getFullYear() * 12 + now.getMonth() - monthsToSubtract;
  const startYear = Math.floor(totalMonths / 12);
  const startMonth = totalMonths % 12;

  const daysInMonth = new Date(startYear, startMonth + 1, 0).getDate();
  const startDay = Math.min(now.getDate(), daysInMonth);

  const startY = String(startYear);
  const startMStr = String(startMonth + 1).padStart(2, "0");
  const startDStr = String(startDay).padStart(2, "0");
  const startDate = `${startY}-${startMStr}-${startDStr}`;

  return { startDate, endDate };
}

/**
 * 分组默认全展开状态
 */
export function initialGroupState(): Record<ManualConditionsGroup, boolean> {
  return {
    source: true,
    aoi: true,
    time: true,
    orbit: true,
  };
}

/**
 * 切换指定分组折叠状态（仅影响该分组）
 */
export function toggleGroup(
  state: Record<ManualConditionsGroup, boolean>,
  id: ManualConditionsGroup
): Record<ManualConditionsGroup, boolean> {
  return {
    ...state,
    [id]: !state[id],
  };
}

/**
 * 来源短标签：去掉空格后的中文说明（「Sentinel-1 雷达卫星」→「Sentinel-1」）；
 * 紧贴在拉丁词后的汉字属于名称本身（「NISAR L波段 (ASF)」保持完整）；若为空则回退至全名
 */
export function shortSourceLabel(displayNameZh: string | null | undefined): string {
  if (!displayNameZh) return "";
  const match = displayNameZh.match(/\s[\p{Script=Han}]/u);
  if (!match || match.index === undefined) {
    return displayNameZh.trim();
  }
  const before = displayNameZh.slice(0, match.index).trim();
  return before || displayNameZh.trim();
}

/**
 * 来源摘要
 */
export function sourceSummary(descriptor: SourceDescriptor | null | undefined): string {
  if (!descriptor) return "";
  const raw = descriptor.display_name_zh || (descriptor as { display_name?: string }).display_name || "";
  return shortSourceLabel(raw);
}

function formatKm(km: number): string {
  const rounded = Math.round(km);
  if (rounded < 1) {
    return "<1";
  }
  return String(rounded);
}

/**
 * AOI 摘要：计算宽高（纬度 cos(lat) 换算），不包含猜测形状
 */
export function aoiSummary(
  bbox: Bbox | { west: number; south: number; east: number; north: number } | null | undefined,
  label?: string | null
): { name: string; sizeText: string } {
  if (!bbox) {
    return { name: t("mapWorkbench.helpers.notSet"), sizeText: "" };
  }

  const { west, south, east, north } = bbox;
  const midLat = (south + north) / 2;
  const midLatRad = (midLat * Math.PI) / 180;

  const deltaLat = Math.abs(north - south);
  const heightKm = deltaLat * 111.32;

  let deltaLng = Math.abs(east - west);
  if (deltaLng > 180) {
    deltaLng = 360 - deltaLng;
  }
  const widthKm = deltaLng * 111.32 * Math.cos(midLatRad);

  const sizeText = t("mapWorkbench.helpers.approximateSize", { width: formatKm(widthKm), height: formatKm(heightKm) });
  const name = (label && label.trim()) || t("mapWorkbench.helpers.notSet");

  return { name, sizeText };
}

/**
 * 时间跨度摘要：inclusive 天数
 */
export function timeSummary(start?: string | null, end?: string | null): string {
  if (!start || !end) return "";
  const startMatch = start.trim().match(/^(\d{4})-(\d{2})-(\d{2})$/);
  const endMatch = end.trim().match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!startMatch || !endMatch) return "";

  const sY = Number(startMatch[1]);
  const sM = Number(startMatch[2]) - 1;
  const sD = Number(startMatch[3]);

  const eY = Number(endMatch[1]);
  const eM = Number(endMatch[2]) - 1;
  const eD = Number(endMatch[3]);

  const sUtc = Date.UTC(sY, sM, sD);
  const eUtc = Date.UTC(eY, eM, eD);

  if (eUtc < sUtc) return "";

  const diffDays = Math.round((eUtc - sUtc) / (24 * 60 * 60 * 1000)) + 1;
  return t("mapWorkbench.helpers.days", { days: diffDays });
}

/**
 * 仅筛选 family === 'sar' 的来源选项
 */
export function sarSourceOptions(
  descriptors: SourceDescriptor[]
): Array<{ value: string; label: string }> {
  if (!descriptors || !Array.isArray(descriptors)) return [];
  return descriptors
    .filter((d) => d.family === "sar")
    .map((d) => ({
      value: d.resource_id,
      label: d.display_name_zh,
    }));
}

/**
 * 上次检索摘要
 */
export function lastSearchSummary(
  searchedAt?: string | null,
  itemCount?: number | null,
  now: Date = new Date()
): string {
  if (!searchedAt || typeof searchedAt !== "string" || !searchedAt.trim()) {
    return "";
  }
  const d = new Date(searchedAt);
  if (isNaN(d.getTime())) {
    return "";
  }

  const hours = String(d.getHours()).padStart(2, "0");
  const minutes = String(d.getMinutes()).padStart(2, "0");
  const timeStr = `${hours}:${minutes}`;

  const isToday =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();

  const timePart = isToday ? timeStr : t("mapWorkbench.helpers.pastDate", { month: d.getMonth() + 1, day: d.getDate(), time: timeStr });

  if (typeof itemCount === "number" && Number.isFinite(itemCount)) {
    return t("mapWorkbench.helpers.lastSearchWithCount", { time: timePart, items: itemCount });
  }
  return t("mapWorkbench.helpers.lastSearch", { time: timePart });
}

function formatToolResultError(
  result: Record<string, unknown> | null | undefined,
  fallback: string,
): string {
  const errObj =
    result?.error && typeof result.error === "object"
      ? (result.error as Record<string, unknown>)
      : null;
  const rawCode =
    typeof errObj?.code === "string"
      ? errObj.code
      : typeof result?.code === "string"
        ? result.code
        : undefined;
  const rawContext = typeof result?.context === "string" ? result.context.trim() : "";
  if (rawCode === "V3_STORE_READINESS_UNSUPPORTED" && rawContext === "formal_dem_download") {
    return t("errors.bridgeText.demNativeGisUnavailable");
  }
  if (rawCode === "V3_STORE_READINESS_UNSUPPORTED" && rawContext === "formal_dem_credential") {
    return t("errors.bridgeText.demCredentialUnavailable");
  }
  if (rawCode) {
    const codeText = explainErrorCode(rawCode) || lookupErrorText("bridge", rawCode);
    if (codeText) return codeText;
  }
  const explained = explainActionError(result?.error ?? result, "");
  if (explained) return explained;
  const rawReason =
    typeof result?.error === "string"
      ? result.error.trim()
      : typeof errObj?.message === "string"
        ? errObj.message.trim()
        : "";
  if (rawReason && !rawReason.includes("\n")) {
    return rawReason.startsWith(fallback) ? rawReason : `${fallback}：${rawReason}`;
  }
  return fallback;
}

export type ToolResultKind = "dem" | "orbit" | "convert";

/**
 * 将工具执行结果格式化为单行文本（DEM、精密轨道、高程转换）
 */
export function toolResultLine(
  kind: ToolResultKind,
  result: Record<string, unknown> | null | undefined
): string {
  if (!result || result.ok === false) {
    const fallback =
      kind === "dem"
        ? t("mapWorkbench.helpers.demPlanningFailed")
        : kind === "orbit"
        ? t("mapWorkbench.helpers.orbitMatchFailed")
        : t("mapWorkbench.helpers.conversionFailed");
    return formatToolResultError(result, fallback);
  }

  if (kind === "dem") {
    const rawTile =
      result.tile_count ??
      (result.plan as Record<string, unknown> | undefined)?.tile_count;
    if (typeof rawTile === "number" && Number.isFinite(rawTile)) {
      return t("mapWorkbench.helpers.demTilesPlannedCount", { tiles: rawTile });
    }
    return t("mapWorkbench.helpers.demTilesPlanned");
  }

  if (kind === "orbit") {
    const count =
      result.matched_count ??
      result.orbit_files ??
      (result.report as Record<string, unknown> | undefined)?.matched_count;
    if (typeof count === "number" && Number.isFinite(count)) {
      return t("mapWorkbench.helpers.orbitFilesMatchedCount", { files: count });
    }
    return t("mapWorkbench.helpers.orbitMatchComplete");
  }

  if (kind === "convert") {
    const outputPath = (
      result.output_path ||
      result.sarscape_ready_dem_path ||
      (result.plan as Record<string, unknown> | undefined)?.sarscape_ready_dem_path ||
      (result.plan as Record<string, unknown> | undefined)?.output_path ||
      result.results_path
    ) as string | undefined;

    if (outputPath && typeof outputPath === "string" && outputPath.trim().length > 0) {
      return t("mapWorkbench.helpers.conversionCompletePath", { path: outputPath });
    }
    return t("mapWorkbench.helpers.conversionComplete");
  }

  return "";
}

/**
 * 格式化精密轨道下载结果行
 */
export function orbitDownloadResultLine(
  result: Record<string, unknown> | null | undefined,
  sceneCount: number
): string {
  if (result && result.ok === true) {
    return t("mapWorkbench.helpers.orbitDownloadsSubmitted", { scenes: sceneCount });
  }
  return formatToolResultError(result, t("mapWorkbench.helpers.orbitDownloadFailed"));
}

/**
 * 格式化 DEM 下载启动结果行
 */
export function demStartResultLine(
  result: Record<string, unknown> | null | undefined,
  planningLine: string = t("mapWorkbench.helpers.demTilesPlanned")
): string {
  if (result && result.ok === true) {
    return planningLine;
  }
  return formatToolResultError(result, t("mapWorkbench.helpers.demDownloadNotStarted"));
}

export const STALE_SEARCH_BATCH_AGE_MS = 24 * 60 * 60 * 1000;

export function staleSearchBatchAgeDays(
  searchedAt?: string | null,
  now: Date = new Date()
): number | null {
  if (!searchedAt || typeof searchedAt !== "string" || !searchedAt.trim()) {
    return null;
  }
  const d = new Date(searchedAt);
  if (isNaN(d.getTime())) {
    return null;
  }
  const ageMs = now.getTime() - d.getTime();
  if (ageMs < STALE_SEARCH_BATCH_AGE_MS) {
    return null;
  }
  return Math.max(1, Math.floor(ageMs / STALE_SEARCH_BATCH_AGE_MS));
}

export function staleSearchBatchAgeLabel(
  searchedAt?: string | null,
  now: Date = new Date()
): string | null {
  const days = staleSearchBatchAgeDays(searchedAt, now);
  if (days === null) return null;
  return t("mapWorkbench.helpers.staleDaysAgo", { days });
}

/**
 * 依据实际渲染的控件动态计算条件栏分组 4 标题（W5-3a-F2b）
 * - 仅 Polarization 时显示 "Polarization"
 * - 仅轨道/分幅时显示 "轨道 / Frame"
 * - 两者兼有时显示 "轨道 / Frame / Polarization"
 * - 兜底默认显示 "轨道 / Frame"
 */
export function groupOrbitTitle(
  profile?: {
    flightDirection?: boolean;
    relativeOrbit?: unknown;
    path?: unknown;
    frame?: unknown;
    polarization?: unknown;
  } | null
): string {
  if (!profile) {
    return t("mapWorkbench.helpers.orbitGroup");
  }

  const hasOrbitOrFrame = Boolean(
    profile.flightDirection ||
    profile.relativeOrbit ||
    profile.path ||
    profile.frame
  );
  const hasPolarization = Boolean(profile.polarization);

  if (hasOrbitOrFrame && hasPolarization) {
    return t("mapWorkbench.helpers.orbitPolarizationGroup");
  }
  if (hasPolarization) {
    return t("mapWorkbench.helpers.polarizationGroup");
  }
  return t("mapWorkbench.helpers.orbitGroup");
}
