import { t } from "../i18n/index.ts";
import { MESSAGES } from "../i18n/locales.ts";
import type { CatalogSearchQuery, SarProduct } from "./generated/v3-contracts.ts";
import { buildSchemaCatalogFilters, searchFilterSchema, type SearchFilterDraft } from "./searchFilterSchema.ts";

export const LANDSAT_C2_L2_LOGICAL_DATASET = "optical.landsat.c2.l2";
export const LANDSAT_C2_L2_PROVIDER_ID = "usgs.eros.m2m";
export const LANDSAT_C2_L2_COLLECTION_ID = "landsat_ot_c2_l2";
export const MPC_LANDSAT_PROVIDER_ID = "microsoft.planetary-computer";
export const MPC_LANDSAT_COLLECTION_ID = "landsat-c2-l2";
export const MPC_LANDSAT_TILE_ORIGIN = "https://planetarycomputer.microsoft.com";
export const EARTH_SEARCH_LANDSAT_PROVIDER_ID = "element84.earth-search";
export const EARTH_SEARCH_LANDSAT_COLLECTION_ID = "landsat-c2-l2";
export const LANDSAT_SEARCH_ONLY_REASON = MESSAGES["zh-CN"].shared.landsatCatalog.searchOnlyReason;

const LANDSAT_SOURCES = [
  { providerId: LANDSAT_C2_L2_PROVIDER_ID, collectionId: LANDSAT_C2_L2_COLLECTION_ID,
    profileId: "usgs.eros.m2m.landsat.c2.l2.scene.v1", identityRuleId: "usgs.eros.m2m.landsat.c2.l2.identity.v1" },
  { providerId: LANDSAT_C2_L2_PROVIDER_ID, collectionId: "landsat_tm_c2_l2",
    profileId: "usgs.eros.m2m.landsat.c2.l2.scene.v1", identityRuleId: "usgs.eros.m2m.landsat.c2.l2.identity.v1" },
  { providerId: LANDSAT_C2_L2_PROVIDER_ID, collectionId: "landsat_etm_c2_l2",
    profileId: "usgs.eros.m2m.landsat.c2.l2.scene.v1", identityRuleId: "usgs.eros.m2m.landsat.c2.l2.identity.v1" },
  { providerId: MPC_LANDSAT_PROVIDER_ID, collectionId: MPC_LANDSAT_COLLECTION_ID,
    profileId: "planetary-computer.landsat-c2-l2.scene.v1", identityRuleId: "planetary-computer.landsat-c2-l2.identity.v1" },
  { providerId: EARTH_SEARCH_LANDSAT_PROVIDER_ID, collectionId: EARTH_SEARCH_LANDSAT_COLLECTION_ID,
    profileId: "element84.earth-search.landsat-c2-l2.scene.v1", identityRuleId: "element84.earth-search.landsat-c2-l2.identity.v1" },
] as const;

/** Exact pairs only: legacy snapshots are read, never relabelled as Microsoft. */
export function landsatSourceIdentity(provider: unknown, collection: unknown) {
  return LANDSAT_SOURCES.find((source) => source.providerId === provider && source.collectionId === collection) ?? null;
}

/** A display/restore hint, not source or download admission. */
export function isLandsatProviderId(provider: unknown): boolean {
  return LANDSAT_SOURCES.some((source) => source.providerId === provider);
}

/** Reviewed filter schema (Issue #3 R2); the Registry names it via search_schema_id. */
export const LANDSAT_C2_L2_SEARCH_SCHEMA_ID = "search.optical.landsat.c2-l2.v1";
const MPC_LANDSAT_STAC_ITEM_ID = /^L[CTE]0[45789]_L2(?:SP|SR)_\d{6}_\d{8}_02_T[12]$/;

export type LandsatMapTilePreview = {
  productKey: string;
  identity: string;
  origin: typeof MPC_LANDSAT_TILE_ORIGIN;
  urlTemplate: string;
  attribution: string;
  statusLabel: string;
};

export type LandsatSearchInput = {
  aoiGeojson: Record<string, unknown> | null;
  startUtc: string;
  endUtc: string;
  cloudCoverLte: string;
  spacecraft: string[];
  processingLevels: string[];
  resultLimit: number;
};

export type LandsatSearchBuildResult =
  | { ok: true; query: CatalogSearchQuery }
  | { ok: false; error: string };

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** Builds the Landsat C2 L2 catalogue query from explicit field values. */
export function buildLandsatC2L2CatalogQuery(
  input: LandsatSearchInput,
): LandsatSearchBuildResult {
  return buildLandsatC2L2CatalogQueryFromDraft({
    aoiGeojson: input.aoiGeojson,
    resultLimit: input.resultLimit,
    draft: {
      platform: [...input.spacecraft],
      product_type: [...input.processingLevels],
      acquisition_time: { start: input.startUtc, end: input.endUtc },
      cloud_cover: input.cloudCoverLte,
    },
  });
}

/** Builds the Workbench Landsat C2 L2 catalogue query from its schema-driven filter draft. */
export function buildLandsatC2L2CatalogQueryFromDraft(input: {
  aoiGeojson: Record<string, unknown> | null;
  draft: SearchFilterDraft;
  resultLimit: number;
}): LandsatSearchBuildResult {
  const aoi = input.aoiGeojson && record(input.aoiGeojson);
  if (!aoi) {
    return { ok: false, error: t("shared.landsatCatalog.missingTrueAoi") };
  }
  // Date range, cloud cover and the closed platform / processing-level sets
  // come from the reviewed schema; both selections must be non-empty.
  const built = buildSchemaCatalogFilters(searchFilterSchema(LANDSAT_C2_L2_SEARCH_SCHEMA_ID), input.draft);
  if (!built.ok) return built;
  const resultLimit = Number.isSafeInteger(input.resultLimit) && input.resultLimit > 0 && input.resultLimit <= 100
    ? input.resultLimit
    : null;
  if (resultLimit === null) return { ok: false, error: t("shared.landsatCatalog.invalidResultLimit") };
  const filters = built.filters;
  return {
    ok: true,
    query: {
      logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
      provider_id: MPC_LANDSAT_PROVIDER_ID,
      native_collection_id: MPC_LANDSAT_COLLECTION_ID,
      aoi_geojson: aoi,
      cursor: null,
      result_limit: resultLimit,
      filters,
    },
  };
}

export function isLandsatC2L2CatalogProduct(product: SarProduct): boolean {
  const metadata = record(product.native_metadata);
  return landsatSourceIdentity(product.provider, metadata?.native_collection_id) !== null
    && metadata?.logical_dataset === LANDSAT_C2_L2_LOGICAL_DATASET
    && metadata.source_kind === "landsat"
    && metadata.product_kind === "optical_scene";
}

export function landsatProperties(product: SarProduct): Record<string, unknown> {
  return record(record(product.native_metadata)?.properties) ?? {};
}

export function landsatCloudCover(product: SarProduct): number | null {
  const value = landsatProperties(product).cloud_cover;
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 100
    ? value
    : null;
}

export function landsatBrowseAvailable(product: SarProduct): boolean {
  return landsatDownloadDisabledReason(product) === null
    && landsatProperties(product).browse_available === true;
}

/** Exact frozen Microsoft scene admission for provider-owned asset metadata. */
export function landsatAssetDetailsAvailable(product: SarProduct): boolean {
  if (!isLandsatC2L2CatalogProduct(product) || product.provider !== MPC_LANDSAT_PROVIDER_ID) {
    return false;
  }
  const metadata = record(product.native_metadata);
  if (metadata?.native_collection_id !== MPC_LANDSAT_COLLECTION_ID) return false;
  const itemId = landsatProperties(product).entity_id;
  return typeof itemId === "string"
    && MPC_LANDSAT_STAC_ITEM_ID.test(itemId)
    && product.product_id === itemId
    && product.provider_product_id === itemId;
}

/**
 * Disposable map projection for already-frozen Microsoft Landsat results.
 * The renderer never accepts a provider URL: the origin, collection, assets,
 * and render recipe are fixed here, while only the validated native STAC item
 * identity comes from the product snapshot.
 */
export function landsatMapTilePreview(product: SarProduct): LandsatMapTilePreview | null {
  if (
    !landsatAssetDetailsAvailable(product)
    || !landsatBrowseAvailable(product)
  ) {
    return null;
  }
  const itemId = landsatProperties(product).entity_id;
  if (typeof itemId !== "string") return null;
  const productKey = product.provider_product_key?.trim() || product.product_key.trim();
  if (!productKey) return null;

  const params = new URLSearchParams();
  params.set("collection", MPC_LANDSAT_COLLECTION_ID);
  params.set("item", itemId);
  params.append("assets", "red");
  params.append("assets", "green");
  params.append("assets", "blue");
  params.set("color_formula", "gamma RGB 2.7, saturation 1.5, sigmoidal RGB 15 0.55");
  params.set("nodata", "0");
  params.set("format", "png");

  return {
    productKey,
    identity: `${productKey}:mpc-landsat-true-color-v1`,
    origin: MPC_LANDSAT_TILE_ORIGIN,
    urlTemplate: `${MPC_LANDSAT_TILE_ORIGIN}/api/data/v1/item/tiles/WebMercatorQuad/{z}/{x}/{y}@1x?${params.toString()}`,
    attribution: "Microsoft Planetary Computer · USGS Landsat",
    statusLabel: t("shared.landsatCatalog.trueColorTilesLabel"),
  };
}

/** Presentation guard only. The application service independently gates delivery before discovery. */
export function landsatDownloadDisabledReason(product: SarProduct): string | null {
  return product.provider === EARTH_SEARCH_LANDSAT_PROVIDER_ID
    ? t("shared.landsatCatalog.searchOnlyReason")
    : null;
}
