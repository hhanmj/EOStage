import catalogJson from "../../../contracts/v3/search-filter-schemas.v1.json" with { type: "json" };

import { t } from "../i18n/index.ts";
import { buildOptionalOpticalDateRange } from "./opticalCatalogSearch.ts";

/**
 * Issue #3 R2: reviewed, EOStage-owned search filter schemas.
 *
 * The contract lives in contracts/v3/search-filter-schemas.v1.json and is
 * validated by Rust (insar-core search_filter_schema, plus a provider test that
 * every emitted key is accepted by every Registry route of the resource).
 * A schema only describes discovery predicates and how they map onto the
 * route-neutral CatalogSearchQuery.filters vocabulary. It never selects a
 * provider, grants delivery or carries native field names.
 */

export type SearchFilterGroup = "facet" | "time" | "quality";

export type SearchFilterOption = {
  value: string;
  label: string;
  description: string;
  registry_platform?: string;
};

type FilterBase = {
  filter_id: string;
  label: string;
  group: SearchFilterGroup;
};

export type MultiEnumSearchFilter = FilterBase & {
  control: "multi_enum";
  query_key: string;
  options: SearchFilterOption[];
  default: string[];
  /** omit: empty means no constraint. restore_all: empty is never sent; the UI restores all options. */
  empty_selection: "omit" | "restore_all";
  invalid_message: string;
};

export type FixedSearchFilter = FilterBase & {
  control: "fixed";
  /** When present the single option is sent as a constant predicate. */
  query_key?: string;
  options: SearchFilterOption[];
};

export type DateRangeSearchFilter = FilterBase & {
  control: "date_range";
  query_key: string;
  help?: string;
};

export type NumberSearchFilter = FilterBase & {
  control: "number";
  query_key: string;
  operator: "lte" | "gte";
  minimum: number;
  maximum: number;
  unit?: string;
  default: number | null;
  placeholder?: string;
  invalid_message: string;
};

export type SearchFilter =
  | MultiEnumSearchFilter
  | FixedSearchFilter
  | DateRangeSearchFilter
  | NumberSearchFilter;

/** Presentation placement only; it grants no route, credential or delivery. */
export type SearchFilterWorkbench = {
  panel: "optical";
  dataset_key: string;
  order: number;
  /** Equals the Registry resource's catalogue_logical_dataset (Rust-validated). */
  logical_dataset: string;
  search_action_label: string;
  result_hint: string;
};

export type SearchFilterSchema = {
  schema_id: string;
  schema_revision: number;
  resource_id: string;
  dataset_label: string;
  description: string;
  workbench?: SearchFilterWorkbench;
  filters: SearchFilter[];
};

export type SearchFilterSchemaCatalog = {
  schema_version: 1;
  catalog_id: string;
  review_version: string;
  schemas: SearchFilterSchema[];
};

export const SEARCH_FILTER_SCHEMA_CATALOG = catalogJson as unknown as SearchFilterSchemaCatalog;

export function searchFilterSchema(schemaId: string): SearchFilterSchema {
  const schema = SEARCH_FILTER_SCHEMA_CATALOG.schemas.find((item) => item.schema_id === schemaId);
  if (!schema) throw new Error(`Unknown search filter schema: ${schemaId}`);
  return schema;
}

export type DateRangeDraft = { start: string; end: string };
/** multi_enum: selected values; date_range: raw start/end; number: raw text input. */
export type SearchFilterDraftValue = readonly string[] | DateRangeDraft | string;
export type SearchFilterDraft = Readonly<Record<string, SearchFilterDraftValue | undefined>>;

export function initialSearchFilterDraft(schema: SearchFilterSchema): Record<string, SearchFilterDraftValue> {
  const draft: Record<string, SearchFilterDraftValue> = {};
  for (const filter of schema.filters) {
    if (filter.control === "multi_enum") draft[filter.filter_id] = [...filter.default];
    else if (filter.control === "date_range") draft[filter.filter_id] = { start: "", end: "" };
    else if (filter.control === "number") draft[filter.filter_id] = filter.default === null ? "" : String(filter.default);
  }
  return draft;
}

export type FacetSearchFilter = MultiEnumSearchFilter | FixedSearchFilter;
export type ConditionSearchFilter = DateRangeSearchFilter | NumberSearchFilter;

/** Product facets in schema order (multi-selects and fixed facts). */
export function facetFilters(schema: SearchFilterSchema): FacetSearchFilter[] {
  return schema.filters.filter((filter): filter is FacetSearchFilter => {
    if (filter.group !== "facet") return false;
    if (filter.control !== "multi_enum" && filter.control !== "fixed") {
      throw new Error(`${schema.schema_id}: facet ${filter.filter_id} must be a multi-select or fixed fact`);
    }
    return true;
  });
}

/** Date ranges and numeric conditions in schema order. */
export function conditionFilters(schema: SearchFilterSchema): ConditionSearchFilter[] {
  return schema.filters.filter((filter): filter is ConditionSearchFilter =>
    filter.group !== "facet" && (filter.control === "date_range" || filter.control === "number"));
}

/** Apply the schema's empty-selection rule to a menu change. */
export function updateMultiEnumSelection(filter: MultiEnumSearchFilter, next: readonly string[]): string[] {
  if (next.length === 0 && filter.empty_selection === "restore_all") {
    return filter.options.map((option) => option.value);
  }
  return [...next];
}

/** The shared multi-select shows "全部" (an empty value) when every option is selected. */
export function multiEnumMenuValues(filter: MultiEnumSearchFilter, selected: readonly string[]): string[] {
  return selected.length === filter.options.length ? [] : [...selected];
}

export function draftSelection(draft: SearchFilterDraft, filterId: string): string[] {
  const value = draft[filterId];
  return Array.isArray(value) ? [...value as readonly string[]] : [];
}

export function draftDateRange(draft: SearchFilterDraft, filterId: string): DateRangeDraft {
  const value = draft[filterId];
  return value && typeof value === "object" && !Array.isArray(value)
    ? { start: (value as DateRangeDraft).start ?? "", end: (value as DateRangeDraft).end ?? "" }
    : { start: "", end: "" };
}

export function draftText(draft: SearchFilterDraft, filterId: string): string {
  const value = draft[filterId];
  return typeof value === "string" ? value : "";
}

export type SchemaCatalogFiltersResult =
  | { ok: true; filters: Record<string, unknown> }
  | { ok: false; error: string };

const NON_NEGATIVE_DECIMAL = /^(?:0|[1-9]\d*)(?:\.\d+)?$/;

function parseNumber(filter: NumberSearchFilter, raw: SearchFilterDraftValue | undefined): number | null | "invalid" {
  if (raw === undefined) return null;
  if (typeof raw !== "string") return "invalid";
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (!NON_NEGATIVE_DECIMAL.test(trimmed)) return "invalid";
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) && parsed >= filter.minimum && parsed <= filter.maximum ? parsed : "invalid";
}

function selectedValues(raw: SearchFilterDraftValue | undefined): string[] | null {
  if (raw === undefined) return [];
  if (!Array.isArray(raw)) return null;
  return raw as string[];
}

function dateDraft(raw: SearchFilterDraftValue | undefined): DateRangeDraft | null {
  if (raw === undefined) return { start: "", end: "" };
  if (typeof raw !== "object" || Array.isArray(raw)) return null;
  const value = raw as DateRangeDraft;
  return typeof value.start === "string" && typeof value.end === "string" ? value : null;
}

/**
 * Validate a filter draft against its schema and emit CatalogSearchQuery.filters.
 *
 * Validation order (first error wins): date ranges, numbers, multi-selects —
 * the order the hand-written optical builders used. Emission order: constant
 * fixed predicates, multi-selects (schema order), date ranges, numbers — the
 * key order the hand-written builders produced, so serialized queries match.
 */
export function buildSchemaCatalogFilters(
  schema: SearchFilterSchema,
  draft: SearchFilterDraft,
): SchemaCatalogFiltersResult {
  const dates = new Map<string, { start: string; end: string } | null>();
  for (const filter of schema.filters) {
    if (filter.control !== "date_range") continue;
    const value = dateDraft(draft[filter.filter_id]);
    if (!value) return { ok: false, error: t("shared.searchFilterSchema.invalidFilterValue", { label: filter.label }) };
    const range = buildOptionalOpticalDateRange(value.start, value.end);
    if (!range.ok) return range;
    dates.set(filter.query_key, range.datetime);
  }

  const numbers = new Map<string, number | null>();
  for (const filter of schema.filters) {
    if (filter.control !== "number") continue;
    const value = parseNumber(filter, draft[filter.filter_id]);
    if (value === "invalid") return { ok: false, error: filter.invalid_message };
    numbers.set(filter.query_key, value);
  }

  const selections = new Map<string, string[] | null>();
  for (const filter of schema.filters) {
    if (filter.control !== "multi_enum") continue;
    const raw = selectedValues(draft[filter.filter_id]);
    if (!raw) return { ok: false, error: filter.invalid_message };
    const allowed = new Set(filter.options.map((option) => option.value));
    const unique = [...new Set(raw)];
    if (unique.some((value) => !allowed.has(value))) return { ok: false, error: filter.invalid_message };
    if (unique.length === 0) {
      if (filter.empty_selection === "restore_all") return { ok: false, error: filter.invalid_message };
      selections.set(filter.query_key, null);
      continue;
    }
    selections.set(filter.query_key, unique.sort());
  }

  const filters: Record<string, unknown> = {};
  for (const filter of schema.filters) {
    if (filter.control === "fixed" && filter.query_key) filters[filter.query_key] = filter.options[0].value;
  }
  for (const [key, value] of selections) if (value) filters[key] = value;
  for (const [key, value] of dates) if (value) filters[key] = value;
  for (const [key, value] of numbers) if (value !== null) filters[key] = value;
  return { ok: true, filters };
}

/**
 * Localizes schema filter labels defined in contracts/v3/search-filter-schemas.v1.json
 * while keeping canonical Chinese text unchanged under zh-CN.
 */
export function localizeSearchFilterLabel(filter: Pick<SearchFilter, "filter_id" | "label">): string {
  if (filter.label === "载荷族") {
    return t("mapWorkbench.schemaFilters.instrumentFamily");
  }
  switch (filter.filter_id) {
    case "platform":
      return t("mapWorkbench.schemaFilters.platform");
    case "cloud_cover":
    case "cloud_cover_lte":
      return t("mapWorkbench.schemaFilters.maxCloud");
    case "product_type":
      return t("mapWorkbench.schemaFilters.productType");
    case "product_level":
    case "processing_level":
      return t("mapWorkbench.schemaFilters.processingLevel");
    case "instrument":
    case "instruments":
      return t("mapWorkbench.schemaFilters.instrument");
    case "instrument_family":
      return t("mapWorkbench.schemaFilters.instrumentFamily");
    case "acquisition_time":
    case "date_range":
      return t("mapWorkbench.schemaFilters.dateRange");
    default:
      return filter.label;
  }
}

export function localizeSearchFilterPlaceholder(filter: Pick<NumberSearchFilter, "filter_id" | "placeholder">): string {
  if (
    filter.filter_id === "cloud_cover" ||
    filter.filter_id === "cloud_cover_lte" ||
    filter.placeholder === "留空不限制"
  ) {
    return t("mapWorkbench.conditions.cloudInputPlaceholder");
  }
  return filter.placeholder || t("mapWorkbench.conditions.noRestriction");
}

export function localizeSearchFilterOptionLabel(option: Pick<SearchFilterOption, "value" | "label">): string {
  if (option.label === "L2 协调地表反射率") {
    return t("mapWorkbench.schemaFilters.hlsHarmonizedL2");
  }
  return option.label;
}
