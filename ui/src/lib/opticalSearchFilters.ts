import registryJson from "../../../contracts/v3/resource-registry.v1.json" with { type: "json" };

import { t } from "../i18n/index.ts";
import type { CatalogSearchIntent } from "./desktopShellBridge.ts";
import {
  SEARCH_FILTER_SCHEMA_CATALOG,
  buildSchemaCatalogFilters,
  initialSearchFilterDraft,
  searchFilterSchema,
  type SearchFilterDraft,
  type SearchFilterDraftValue,
  type SearchFilterSchema,
} from "./searchFilterSchema.ts";

/** The schema's workbench dataset_key, e.g. "sentinel2_l2a" or "hls_l30". */
export type OpticalDataset = string;

export type OpticalSearchCapability = {
  key: OpticalDataset;
  /** Reviewed search filter schema; the Registry names it via search_schema_id. */
  schemaId: string;
  resourceId: string;
  /** The Registry catalogue_logical_dataset of frozen candidates. */
  logicalDataset: string;
  datasetLabel: string;
  description: string;
  searchActionLabel: string;
  resultHint: string;
  /** True when the Registry declares no delivery capability for the product. */
  searchOnly: boolean;
};

type RegistryCapability = { kind: string; lifecycle: string };
type RegistryResource = { common: { resource_id: string; capabilities?: RegistryCapability[] } };

/** Capability kinds that can hand a frozen candidate to a delivery task. */
const DELIVERY_CAPABILITY_KINDS = new Set(["full_archive", "asset_set", "download", "remote_subset"]);

function registryDeclaresDelivery(resourceId: string): boolean {
  const resource = (registryJson as unknown as { resources: RegistryResource[] }).resources
    .find((item) => item.common.resource_id === resourceId);
  return (resource?.common.capabilities ?? []).some((capability) => (
    // Same rule as the application's task gate: any delivery capability that
    // is not reserved or retired counts (implemented, admission-controlled, retained).
    DELIVERY_CAPABILITY_KINDS.has(capability.kind)
    && capability.lifecycle !== "reserved"
    && capability.lifecycle !== "retired"
  ));
}

/**
 * Issue #3 R2 B5: the optical Workbench lists every reviewed schema placed on
 * the optical panel, in schema order. Adding a product is a contract edit:
 * the list, its filters, labels, defaults and search action all come from
 * contracts/v3/search-filter-schemas.v1.json.
 */
export const OPTICAL_SEARCH_CAPABILITIES: readonly OpticalSearchCapability[] = SEARCH_FILTER_SCHEMA_CATALOG.schemas
  .filter((schema) => schema.workbench?.panel === "optical")
  .slice()
  .sort((left, right) => (left.workbench?.order ?? 0) - (right.workbench?.order ?? 0))
  .map((schema) => {
    const workbench = schema.workbench!;
    return {
      key: workbench.dataset_key,
      schemaId: schema.schema_id,
      resourceId: schema.resource_id,
      logicalDataset: workbench.logical_dataset,
      datasetLabel: schema.dataset_label,
      description: schema.description,
      searchActionLabel: workbench.search_action_label,
      resultHint: workbench.result_hint,
      searchOnly: !registryDeclaresDelivery(schema.resource_id),
    };
  });

/**
 * A frozen candidate of a search-only optical product can never start a
 * download task; the reason is shown instead of a download action.
 */
export function searchOnlyOpticalReason(nativeMetadata: unknown): string | null {
  const logicalDataset = nativeMetadata && typeof nativeMetadata === "object"
    ? (nativeMetadata as Record<string, unknown>).logical_dataset
    : undefined;
  const capability = opticalCapabilityForLogicalDataset(logicalDataset);
  return capability?.searchOnly
    ? t("shared.opticalFilters.searchOnlyDisabledReason", { dataset: capability.datasetLabel })
    : null;
}

export const DEFAULT_OPTICAL_DATASET: OpticalDataset = OPTICAL_SEARCH_CAPABILITIES[0].key;

export function opticalSearchCapability(dataset: OpticalDataset): OpticalSearchCapability {
  const capability = OPTICAL_SEARCH_CAPABILITIES.find((item) => item.key === dataset);
  if (!capability) throw new Error(`Unknown optical dataset: ${dataset}`);
  return capability;
}

export function opticalCapabilityForLogicalDataset(logicalDataset: unknown): OpticalSearchCapability | null {
  return OPTICAL_SEARCH_CAPABILITIES.find((item) => item.logicalDataset === logicalDataset) ?? null;
}

export function opticalSearchSchema(dataset: OpticalDataset): SearchFilterSchema {
  return searchFilterSchema(opticalSearchCapability(dataset).schemaId);
}

/** One schema-driven filter draft per optical product; switching products keeps each draft. */
export type OpticalFilterDrafts = Record<OpticalDataset, Record<string, SearchFilterDraftValue>>;

export function initialOpticalFilterDrafts(): OpticalFilterDrafts {
  return Object.fromEntries(
    OPTICAL_SEARCH_CAPABILITIES.map((item) => [item.key, initialSearchFilterDraft(searchFilterSchema(item.schemaId))]),
  ) as OpticalFilterDrafts;
}

/** The tightest bound the hand-written optical builders enforced (Landsat). */
export const MAX_OPTICAL_INTENT_RESULT_LIMIT = 100;

export type OpticalIntentBuildResult =
  | { ok: true; intent: CatalogSearchIntent }
  | { ok: false; error: string };

function plainObject(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/**
 * Issue #3 R2 B4: build a route-neutral catalogue intent for any optical
 * product. The product, schema revision and predicates come from the reviewed
 * schema; the application chooses the route from the Registry.
 */
export function buildOpticalCatalogIntent(
  dataset: OpticalDataset,
  input: { aoiGeojson: Record<string, unknown> | null; draft: SearchFilterDraft; resultLimit: number },
): OpticalIntentBuildResult {
  const capability = opticalSearchCapability(dataset);
  const schema = searchFilterSchema(capability.schemaId);
  const aoi = plainObject(input.aoiGeojson);
  if (!aoi) {
    return { ok: false, error: t("shared.opticalFilters.missingTrueAoi", { dataset: capability.datasetLabel }) };
  }
  const built = buildSchemaCatalogFilters(schema, input.draft);
  if (!built.ok) return built;
  if (!Number.isSafeInteger(input.resultLimit) || input.resultLimit < 1 || input.resultLimit > MAX_OPTICAL_INTENT_RESULT_LIMIT) {
    return { ok: false, error: t("shared.opticalFilters.invalidResultLimit") };
  }
  return {
    ok: true,
    intent: {
      resource_id: schema.resource_id,
      search_schema_id: schema.schema_id,
      schema_revision: schema.schema_revision,
      aoi_geojson: aoi as CatalogSearchIntent["aoi_geojson"],
      result_limit: input.resultLimit,
      filters: built.filters,
    },
  };
}
