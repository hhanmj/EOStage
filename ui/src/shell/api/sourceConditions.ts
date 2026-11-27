/**
/**
 * 数据源选项与条件纯逻辑模块（W5-3a-F2）
 * 包含 SAR/光学数据源分组、条件栏类型判定、光学筛选构建与下载能力裁定。
 */

import {
  SEARCH_FILTER_SCHEMA_CATALOG,
  buildSchemaCatalogFilters,
  type SearchFilterDraft,
  type SearchFilterSchema,
} from "../../lib/searchFilterSchema.ts";
import { buildOptionalOpticalDateRange } from "../../lib/opticalCatalogSearch.ts";
import { OPTICAL_SEARCH_CAPABILITIES } from "../../lib/opticalSearchFilters.ts";
import type { SourceDescriptor } from "./types.ts";
import { getCurrentLocale, t } from "../../i18n/index.ts";
import { CATALOG_ITEM_TRANSLATIONS } from "../../lib/dataSourceCatalogTranslations.ts";

function localizedOptionLabel(d: SourceDescriptor): string {
  const locale = getCurrentLocale();
  if (locale !== "zh-CN") {
    const translated = CATALOG_ITEM_TRANSLATIONS[d.resource_id]?.[locale]?.name;
    if (translated) return translated;
  }
  return d.display_name_zh;
}

export interface SourceOptionItem {
  value: string;
  label: string;
  disabled: boolean;
  title?: string;
}

export interface SourceGroupOption {
  group: string;
  options: SourceOptionItem[];
}

export type UtcDayRangeResult =
  | { ok: true; range: { start: string; end: string } | null }
  | { ok: false; error: string };

/**
 * 纯函数：将 YYYY-MM-DD 日期区间转换为含当天的 UTC RFC 3339 范围
 * 1. 两者皆空返回 { ok: true, range: null }
 * 2. 仅填写单边返回 { ok: false, error: "开始和结束日期需同时填写，或都留空" }
 * 3. 开始晚于结束返回 { ok: false, error: "结束日期不能早于开始日期。" }
 * 4. 正常返回闭区间 { start: `${start}T00:00:00Z`, end: `${end}T23:59:59Z` }
 */
export function utcDayRange(
  startDate?: string | null,
  endDate?: string | null
): UtcDayRangeResult {
  const s = (startDate ?? "").trim();
  const e = (endDate ?? "").trim();

  if (!s && !e) {
    return { ok: true, range: null };
  }

  if (!s || !e) {
    return { ok: false, error: t("mapWorkbench.results.pairedDatesRequired") };
  }

  if (s > e) {
    return { ok: false, error: t("mapWorkbench.results.endBeforeStart") };
  }

  return {
    ok: true,
    range: {
      start: `${s}T00:00:00Z`,
      end: `${e}T23:59:59Z`,
    },
  };
}

function findOpticalSchema(schemaId?: string | null): SearchFilterSchema | undefined {
  if (!schemaId) return undefined;
  return SEARCH_FILTER_SCHEMA_CATALOG.schemas.find((item) => item.schema_id === schemaId);
}

/**
 * 分组来源下拉列表：
 * SAR 组在前，光学组在后；
 * 仅包含在 SEARCH_FILTER_SCHEMA_CATALOG 经审查存在的光学源；
 * unavailable 状态的源置灰 disabled 并提供原因 title；
 * 绝不捏造条目。
 */
/**
 * ASF 已从检索接口下线、不在界面上列出的数据源（Owner 2026-10-31）。
 * 注册表与后端代码暂时保留，ASF 恢复时去掉这里即可。
 */
export const HIDDEN_SOURCE_IDS: ReadonlySet<string> = new Set([
  "science.ers-1-2.sar.v1",
  "science.radarsat-1.sar.v1",
  "science.jers-1.sar.v1",
]);

export function sourceOptions(
  descriptors: SourceDescriptor[]
): SourceGroupOption[] {
  if (!descriptors || !Array.isArray(descriptors) || descriptors.length === 0) {
    return [];
  }

  const sarOptions: SourceOptionItem[] = [];
  const opticalOptions: SourceOptionItem[] = [];

  for (const d of descriptors) {
    if (HIDDEN_SOURCE_IDS.has(d.resource_id)) continue;
    if (d.family === "sar") {
      const isUnavailable = d.availability === "unavailable";
      sarOptions.push({
        value: d.resource_id,
        label: localizedOptionLabel(d),
        disabled: isUnavailable,
        title: isUnavailable ? d.availability_reason_zh || t("mapWorkbench.results.unavailable") : undefined,
      });
    } else if (d.family === "optical" && d.filter_schema_id) {
      const schema = findOpticalSchema(d.filter_schema_id);
      if (schema) {
        const isUnavailable = d.availability === "unavailable";
        opticalOptions.push({
          value: d.resource_id,
          label: localizedOptionLabel(d),
          disabled: isUnavailable,
          title: isUnavailable ? d.availability_reason_zh || t("mapWorkbench.results.unavailable") : undefined,
        });
      }
    }
  }

  const groups: SourceGroupOption[] = [];
  if (sarOptions.length > 0) {
    groups.push({ group: "SAR", options: sarOptions });
  }
  if (opticalOptions.length > 0) {
    groups.push({ group: t("mapWorkbench.results.opticalSource"), options: opticalOptions });
  }

  return groups;
}

/**
 * 判定当前数据源对应的条件栏渲染形态
 */
export function conditionsKind(
  resourceId: string,
  descriptors: SourceDescriptor[]
): "sar" | "optical" | null {
  const d = descriptors.find((item) => item.resource_id === resourceId);
  if (!d) return null;
  if (d.family === "sar") return "sar";
  if (d.family === "optical" && d.filter_schema_id) {
    const schema = findOpticalSchema(d.filter_schema_id);
    if (schema) return "optical";
  }
  return null;
}

export type OpticalFiltersResult =
  | {
      ok: true;
      filters: Record<string, unknown>;
      timeRange: { start: string; end: string } | null;
    }
  | { ok: false; error: string };

/**
 * 光学筛选参数校验与构建纯函数
 * crates/insar-application-service/src/unified_search.rs:280-296
 * crates/insar-core/src/search_filter_schema.rs:300-315
 * 1. filters 严格包含 schema 声明的 fixed / multi_enum / number 字段，绝不输出 datetime 或 dataset；
 * 2. timeRange 通过 buildOptionalOpticalDateRange 规范化为以 'Z' 结尾的 UTC 时间戳对象；两者皆空时为 null。
 */
export function opticalFilters(
  schemaId: string,
  draft: SearchFilterDraft,
  startDate?: string | null,
  endDate?: string | null
): OpticalFiltersResult {
  const schema = findOpticalSchema(schemaId);
  if (!schema) {
    return { ok: false, error: t("mapWorkbench.results.unknownSchema", { schemaId }) };
  }

  // 1. 校验与构建属性 filters（facets、fixed、numbers）
  // 对声明了 empty_selection === "restore_all" 的多选筛选项，当用户未选或全不选（留空）时自动回填默认全选集合
  const normalizedDraft: Record<string, SearchFilterDraft[string]> = { ...draft };
  for (const filter of schema.filters) {
    if (filter.control === "multi_enum" && filter.empty_selection === "restore_all") {
      const current = normalizedDraft[filter.filter_id];
      if (!Array.isArray(current) || current.length === 0) {
        normalizedDraft[filter.filter_id] =
          filter.default.length > 0
            ? [...filter.default]
            : filter.options.map((opt) => opt.value);
      }
    }
  }

  const built = buildSchemaCatalogFilters(schema, normalizedDraft);
  if (!built.ok) {
    return built;
  }

  const filters = { ...built.filters };

  // 确保 filters 不发射 schema 的 date_range 键（时间由请求顶层 time_range 提供，后端会自动注入 datetime）
  for (const filter of schema.filters) {
    if (filter.control === "date_range" && filter.query_key) {
      delete filters[filter.query_key];
    }
  }
  delete filters.datetime;
  delete filters.dataset;

  // 2. 校验与构建时间范围
  const rawStart = (startDate || "").trim();
  const rawEnd = (endDate || "").trim();

  const dateRes = utcDayRange(rawStart, rawEnd);
  if (!dateRes.ok) {
    return { ok: false, error: dateRes.error };
  }

  return {
    ok: true,
    filters,
    timeRange: dateRes.range,
  };
}

/**
 * 判定数据源是否为仅检索无交付能力（searchOnly）
 */
export function isSourceSearchOnly(resourceId: string): boolean {
  if (resourceId === "science.dlr.enmap.l2a.v1") {
    return true;
  }
  const cap = OPTICAL_SEARCH_CAPABILITIES.find((c) => c.resourceId === resourceId);
  return cap ? cap.searchOnly : false;
}

/**
 * 数据源下载能力裁定
 */
export function canDownloadForSource(resourceId: string): {
  allowed: boolean;
  reason?: string;
} {
  if (isSourceSearchOnly(resourceId)) {
    return { allowed: false, reason: t("mapWorkbench.results.searchOnly") };
  }
  return { allowed: true };
}
