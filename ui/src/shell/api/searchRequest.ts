import { t } from "../../i18n/index.ts";
import type { UnifiedSearchRequest, TimeRange } from "../../lib/generated/v3-contracts.ts";
import { sarFilterProfile, buildSarFilters } from "./sarFilterProfile.ts";
import { opticalFilters, utcDayRange } from "./sourceConditions.ts";
import { SEARCH_FILTER_SCHEMA_CATALOG, type SearchFilterDraft } from "../../lib/searchFilterSchema.ts";

export interface BuildSearchRequestParams {
  resourceId: string;
  aoiGeometry?: Record<string, unknown> | null;
  aoiName: string;
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
  opticalDraft?: SearchFilterDraft;
}

export interface StoragePolicySettingsInput {
  mode?: string;
  max_concurrent_items?: number;
  organize_by_product?: boolean;
  default_output_root?: string;
  ok?: boolean;
  [key: string]: unknown;
}

export interface SarDownloadPolicySavePayload {
  mode: string;
  max_concurrent_items: number;
  organize_by_product: boolean;
  default_output_root: string;
}

/**
 * 纯函数请求构造器（P0-1）
 * 严格对齐后端 SarSearchQuery 反序列化约束与权威 GeoJSON 要求
 */
export function buildUnifiedSearchRequest(params: BuildSearchRequestParams): UnifiedSearchRequest {
  if (!params.aoiGeometry || Object.keys(params.aoiGeometry).length === 0) {
    throw new Error(t("shared.searchRequest.missingAoiError"));
  }

  const effectiveAoiName = (params.aoiName && params.aoiName.trim()) || t("shared.searchRequest.defaultAoiName");
  const requestId = `req-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

  // 1. SAR 数据源路径
  const profile = sarFilterProfile(params.resourceId);
  if (profile) {
    let timeRange: TimeRange | null = null;
    if (profile.kind === "typed") {
      const dateRes = utcDayRange(params.startDate, params.endDate);
      if (!dateRes.ok) {
        throw new Error(dateRes.error);
      }
      timeRange = dateRes.range;
    } else {
      if (params.startDate || params.endDate) {
        timeRange = {
          start: params.startDate ?? "",
          end: params.endDate ?? "",
        };
      }
    }

    const filters = buildSarFilters(profile, {
      platforms: params.platforms,
      productType: params.productType,
      polarizations: params.polarizations,
      beamModes: params.beamModes,
      lookDirection: params.lookDirection,
      orbitDirection: params.orbitDirection,
      relativeOrbit: params.relativeOrbit,
      path: params.path,
      frame: params.frame,
    });

    return {
      resource_id: params.resourceId,
      aoi: params.aoiGeometry,
      aoi_name: effectiveAoiName,
      time_range: timeRange,
      filters,
      request_id: requestId,
    };
  }

  // 2. 光学数据源路径 (crates/insar-application-service/src/unified_search.rs:280-296)
  const opticalSchema = SEARCH_FILTER_SCHEMA_CATALOG.schemas.find(
    (s) => s.resource_id === params.resourceId
  );
  if (opticalSchema) {
    const optRes = opticalFilters(
      opticalSchema.schema_id,
      params.opticalDraft ?? {},
      params.startDate,
      params.endDate
    );
    if (!optRes.ok) {
      throw new Error(optRes.error);
    }

    return {
      resource_id: params.resourceId,
      aoi: params.aoiGeometry,
      aoi_name: effectiveAoiName,
      time_range: optRes.timeRange,
      filters: optRes.filters,
      request_id: requestId,
    };
  }

  // 3. 辅助/专用工作台数据源路径 (DEM, Event, Geodesy, Atmospheric, Ionospheric, Orbit, Auxiliary)
  const auxiliaryPrefixes = ["dem.", "event.", "geodesy.", "atmospheric.", "ionospheric.", "orbit.", "auxiliary."];
  if (auxiliaryPrefixes.some((prefix) => params.resourceId.startsWith(prefix))) {
    let timeRange: TimeRange | null = null;
    if (params.startDate || params.endDate) {
      const dateRes = utcDayRange(params.startDate, params.endDate);
      if (!dateRes.ok) {
        throw new Error(dateRes.error);
      }
      timeRange = dateRes.range;
    }

    return {
      resource_id: params.resourceId,
      aoi: params.aoiGeometry,
      aoi_name: effectiveAoiName,
      time_range: timeRange,
      filters: {},
      request_id: requestId,
    };
  }

  throw new Error(t("shared.searchRequest.unsupportedResourceError", { resourceId: params.resourceId }));
}

/**
 * 构造存储配置持久化有效载荷（P0-4）
 * 严格剔除 ok 等包装字段，确保仅包含后端 SarDownloadPolicySettings 允许的四字段
 */
export function buildStorageSavePayload(
  policy: StoragePolicySettingsInput | null | undefined,
  newPath: string
): SarDownloadPolicySavePayload {
  const trimmedPath = newPath.trim();
  return {
    mode: (policy?.mode as string) || "manual",
    max_concurrent_items: typeof policy?.max_concurrent_items === "number" ? policy.max_concurrent_items : 2,
    organize_by_product: Boolean(policy?.organize_by_product),
    default_output_root: trimmedPath,
  };
}

/**
 * 格式化存储保存错误（P0-4）
 * 替换 formatAoiSaveError，给出真实中文一句话说明
 */
export function formatStorageSaveError(err: any, fallback = t("shared.searchRequest.saveStorageErrorFallback")): string {
  if (!err) return fallback;
  if (typeof err === "string") return err;
  const code = err.code ? `[${err.code}] ` : "";
  const msg = err.message_zh || err.message || err.error;
  if (msg) return `${code}${msg}`;
  return fallback;
}
