import { t } from "../i18n/index.ts";
import type { CatalogSearchQuery, ProviderAsset, SarProduct } from "./generated/v3-contracts.ts";
import { buildSchemaCatalogFilters, searchFilterSchema, type SearchFilterDraft } from "./searchFilterSchema.ts";

export const SENTINEL2_L2A_LOGICAL_DATASET = "optical.sentinel-2.l2a";
export const SENTINEL2_L1C_LOGICAL_DATASET = "optical.sentinel-2.l1c";
export const SENTINEL2_L2A_PROVIDER_ID = "copernicus.dataspace.odata";
export const SENTINEL2_L2A_COLLECTION_ID = "SENTINEL-2";
export const SENTINEL2_L2A_PRODUCT_TYPE = "S2MSI2A";
export const MPC_SENTINEL2_L2A_PROVIDER_ID = "microsoft.planetary-computer";
export const MPC_SENTINEL2_L2A_COLLECTION_ID = "sentinel-2-l2a";
export const MPC_SENTINEL2_L2A_PROFILE_ID = "planetary-computer.sentinel-2-l2a.scene.v1";
export const MPC_SENTINEL2_L2A_IDENTITY_RULE_ID = "planetary-computer.sentinel-2-l2a.identity.v1";
export const MPC_SENTINEL2_L2A_FILENAME_RULE_ID = "planetary-computer.sentinel-2-l2a.product-uri.v1";
export const MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID = "planetary-computer.sentinel-2-l2a.search-only.v1";
export const MPC_SENTINEL2_L2A_TILE_ORIGIN = "https://planetarycomputer.microsoft.com";
export const EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID = "element84.earth-search";
export const EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID = "sentinel-2-l2a";
export const EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID = "element84.earth-search.sentinel-2-l2a.scene.v1";
export const EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID = "element84.earth-search.sentinel-2-l2a.identity.v1";
export const EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID = "element84.earth-search.sentinel-2-l2a.product-uri.v1";
export const EARTH_SEARCH_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID = "element84.earth-search.sentinel-2-l2a.search-only.v1";
export const EARTH_SEARCH_SENTINEL2_COGS_ORIGIN = "https://sentinel-cogs.s3.us-west-2.amazonaws.com";

export type Sentinel2MapTilePreview = {
  productKey: string;
  identity: string;
  origin: typeof MPC_SENTINEL2_L2A_TILE_ORIGIN;
  urlTemplate: string;
  attribution: string;
  statusLabel: string;
};

export type Sentinel2SearchInput = {
  aoiGeojson: Record<string, unknown> | null;
  startUtc: string;
  endUtc: string;
  cloudCoverLte: string;
  resultLimit: number;
  spacecraft?: readonly string[];
};

export type Sentinel2SearchBuildResult =
  | { ok: true; query: CatalogSearchQuery }
  | { ok: false; error: string };

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** Reviewed filter schema (Issue #3 R2); the Registry names it via search_schema_id. */
export const SENTINEL2_L2A_SEARCH_SCHEMA_ID = "search.optical.sentinel-2.l2a.v1";

export type OpticalDraftSearchInput = {
  aoiGeojson: Record<string, unknown> | null;
  /** A filter draft keyed by the reviewed schema's filter ids. */
  draft: SearchFilterDraft;
  resultLimit: number;
};

/** Builds the Sentinel-2 L2A catalogue query from explicit field values. */
export function buildSentinel2L2aCatalogQuery(
  input: Sentinel2SearchInput,
): Sentinel2SearchBuildResult {
  return buildSentinel2L2aCatalogQueryFromDraft({
    aoiGeojson: input.aoiGeojson,
    resultLimit: input.resultLimit,
    draft: {
      platform: [...(input.spacecraft ?? [])],
      acquisition_time: { start: input.startUtc, end: input.endUtc },
      cloud_cover: input.cloudCoverLte,
    },
  });
}

/** Builds the only user-editable Sentinel-2 L2A catalogue query in the Workbench. */
export function buildSentinel2L2aCatalogQueryFromDraft(
  input: OpticalDraftSearchInput,
): Sentinel2SearchBuildResult {
  const aoi = input.aoiGeojson && record(input.aoiGeojson);
  if (!aoi) {
    return { ok: false, error: t("shared.sentinel2Catalog.missingTrueAoi") };
  }
  // Date range, cloud cover, platform values and the constant product type
  // come from the reviewed schema. An empty platform selection means all
  // platforms and is omitted, as old frozen queries were.
  const built = buildSchemaCatalogFilters(searchFilterSchema(SENTINEL2_L2A_SEARCH_SCHEMA_ID), input.draft);
  if (!built.ok) return built;
  const resultLimit = Number.isSafeInteger(input.resultLimit) && input.resultLimit > 0 && input.resultLimit <= 500
    ? input.resultLimit
    : null;
  if (resultLimit === null) {
    return { ok: false, error: t("shared.sentinel2Catalog.invalidResultLimit") };
  }
  const filters = built.filters;
  return {
    ok: true,
    query: {
      logical_dataset: SENTINEL2_L2A_LOGICAL_DATASET,
      provider_id: SENTINEL2_L2A_PROVIDER_ID,
      native_collection_id: SENTINEL2_L2A_COLLECTION_ID,
      aoi_geojson: aoi,
      cursor: null,
      result_limit: resultLimit,
      filters,
    },
  };
}

export function isMicrosoftSentinel2SearchOnlyProduct(product: SarProduct): boolean {
  const metadata = record(product.native_metadata);
  return product.provider === MPC_SENTINEL2_L2A_PROVIDER_ID
    && metadata?.logical_dataset === SENTINEL2_L2A_LOGICAL_DATASET
    && metadata.native_collection_id === MPC_SENTINEL2_L2A_COLLECTION_ID
    && metadata.collection_profile_id === MPC_SENTINEL2_L2A_PROFILE_ID
    && metadata.identity_rule_id === MPC_SENTINEL2_L2A_IDENTITY_RULE_ID
    && metadata.filename_rule_id === MPC_SENTINEL2_L2A_FILENAME_RULE_ID
    && metadata.resolver_policy_id === MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID
    && metadata.source_kind === "sentinel2"
    && metadata.product_kind === "optical_scene";
}

export function isEarthSearchSentinel2Product(product: SarProduct): boolean {
  const metadata = record(product.native_metadata);
  return product.provider === EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
    && metadata?.logical_dataset === SENTINEL2_L2A_LOGICAL_DATASET
    && metadata.native_collection_id === EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID
    && metadata.collection_profile_id === EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID
    && metadata.identity_rule_id === EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID
    && metadata.filename_rule_id === EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID
    && metadata.resolver_policy_id === EARTH_SEARCH_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID
    && metadata.source_kind === "sentinel2"
    && metadata.product_kind === "optical_scene";
}

export function isValidEarthSearchSentinel2ThumbnailUrl(itemId: string, rawUrl: string): boolean {
  const itemParts = itemId.trim().split("_");
  if (
    itemParts.length !== 5
    || !["S2A", "S2B", "S2C"].includes(itemParts[0])
    || !/^\d{2}[C-HJ-NP-X][A-Z]{2}$/.test(itemParts[1])
    || !/^\d{8}$/.test(itemParts[2])
    || !/^\d+$/.test(itemParts[3])
    || itemParts[4] !== "L2A"
  ) {
    return false;
  }
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return false;
  }
  const pathLower = parsed.pathname.toLowerCase();
  if (
    parsed.origin !== EARTH_SEARCH_SENTINEL2_COGS_ORIGIN
    || parsed.username !== ""
    || parsed.password !== ""
    || parsed.search !== ""
    || parsed.hash !== ""
    || pathLower.includes("%2f")
    || pathLower.includes("%5c")
    || pathLower.includes("%2e")
  ) {
    return false;
  }
  const segments = parsed.pathname.split("/").filter(Boolean);
  if (
    segments.length !== 8
    || segments[0] !== "sentinel-s2-l2a-cogs"
    || segments[6] !== itemId
    || !segments[7].endsWith(".jpg")
    || segments[7].length <= 4
  ) {
    return false;
  }
  return true;
}

export function earthSearchSentinel2ThumbnailAsset(product: SarProduct): ProviderAsset | null {
  if (!isEarthSearchSentinel2Product(product)) return null;
  const itemId = product.provider_product_id?.trim() || product.product_id?.trim() || "";
  if (!itemId) return null;
  for (const asset of product.assets ?? []) {
    const url = asset.url?.trim() ?? "";
    if (asset.role === "thumbnail" && isValidEarthSearchSentinel2ThumbnailUrl(itemId, url)) {
      return asset;
    }
  }
  return null;
}

/**
 * Map-only true-color projection for a frozen Microsoft Sentinel-2 L2A scene.
 * Provider asset URLs are never accepted here: the fixed Data API origin and
 * visual rendering recipe are generated from the already-reviewed native item
 * identity. This does not grant download or task execution authority.
 */
export function sentinel2MapTilePreview(product: SarProduct): Sentinel2MapTilePreview | null {
  if (!isMicrosoftSentinel2SearchOnlyProduct(product)) return null;
  const itemId = product.product_id?.trim() ?? "";
  return sentinel2ResolvedMapTilePreview(product, itemId);
}

export function sentinel2ResolvedMapTilePreview(
  product: SarProduct,
  itemId: string,
): Sentinel2MapTilePreview | null {
  if (!isSentinel2L2aCatalogProduct(product)) return null;
  const providerProductId = product.provider_product_id?.trim() ?? "";
  const safeName = product.scene_id?.trim() ?? "";
  const itemMatch = /^(S2[ABC])_MSIL2A_(\d{8}T\d{6})_(R\d{3})_(T\d{2}[A-Z]{3})_(\d{8}T\d{6})$/.exec(itemId);
  const safeMatch = /^(S2[ABC])_MSIL2A_(\d{8}T\d{6})_N\d{4}_(R\d{3})_(T\d{2}[A-Z]{3})_(\d{8}T\d{6})\.SAFE$/.exec(safeName);
  if (
    !itemMatch
    || !safeMatch
    || itemMatch[1] !== safeMatch[1]
    || itemMatch[2] !== safeMatch[2]
    || itemMatch[3] !== safeMatch[3]
    || itemMatch[4] !== safeMatch[4]
    || itemMatch[5] !== safeMatch[5]
  ) return null;
  const productKey = product.product_key.trim();
  if (!productKey) return null;
  if (isMicrosoftSentinel2SearchOnlyProduct(product)) {
    const expectedKey = `PROVIDER::${MPC_SENTINEL2_L2A_PROVIDER_ID}::COLLECTION::${MPC_SENTINEL2_L2A_COLLECTION_ID}::ITEM::${itemId}`;
    const providerKey = product.provider_product_key?.trim() || productKey;
    if (providerProductId !== itemId || providerKey !== expectedKey || productKey !== expectedKey) return null;
  }

  const params = new URLSearchParams();
  params.set("collection", MPC_SENTINEL2_L2A_COLLECTION_ID);
  params.set("item", itemId);
  params.set("assets", "visual");
  params.set("asset_bidx", "visual|1,2,3");
  params.set("nodata", "0");
  params.set("format", "png");
  return {
    productKey,
    identity: `${productKey}:mpc-sentinel2-true-color-v1`,
    origin: MPC_SENTINEL2_L2A_TILE_ORIGIN,
    urlTemplate: `${MPC_SENTINEL2_L2A_TILE_ORIGIN}/api/data/v1/item/tiles/WebMercatorQuad/{z}/{x}/{y}@1x?${params.toString()}`,
    attribution: "Microsoft Planetary Computer · Copernicus Sentinel-2",
    statusLabel: t("shared.sentinel2Catalog.trueColorTilesLabel"),
  };
}

export function isSentinel2L2aCatalogProduct(product: SarProduct): boolean {
  const metadata = record(product.native_metadata);
  const cdse = product.provider === SENTINEL2_L2A_PROVIDER_ID
    && metadata?.logical_dataset === SENTINEL2_L2A_LOGICAL_DATASET
    && metadata.native_collection_id === SENTINEL2_L2A_COLLECTION_ID
    && metadata.source_kind === "sentinel2"
    && metadata.product_kind === "optical_scene";
  return cdse || isMicrosoftSentinel2SearchOnlyProduct(product) || isEarthSearchSentinel2Product(product);
}

export function isSentinel2L1cCatalogProduct(product: SarProduct): boolean {
  const metadata = record(product.native_metadata);
  return product.provider === SENTINEL2_L2A_PROVIDER_ID
    && metadata?.logical_dataset === SENTINEL2_L1C_LOGICAL_DATASET
    && metadata.native_collection_id === SENTINEL2_L2A_COLLECTION_ID
    && metadata.source_kind === "sentinel2"
    && metadata.product_kind === "optical_scene";
}

export function sentinel2DownloadDisabledReason(_product: SarProduct): string | null {
  // Microsoft remains a search-only source. A user download is re-resolved
  // against the authoritative CDSE SAFE catalogue before any task is created,
  // so the search-only source itself is no longer a UI-level download blocker.
  return null;
}

function productProperties(product: SarProduct): Record<string, unknown> {
  const metadata = record(product.native_metadata);
  return record(metadata?.properties) ?? {};
}

export function sentinel2CloudCover(product: SarProduct): number | null {
  const value = productProperties(product).cloud_cover;
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 100
    ? value
    : null;
}

export function sentinel2Online(product: SarProduct): boolean | null {
  const value = productProperties(product).online;
  return typeof value === "boolean" ? value : null;
}

/** The MGRS tile is displayed only when the reviewed Sentinel-2 name carries it. */
export function sentinel2Tile(product: SarProduct): string | null {
  const name = product.file_name?.trim().replace(/\.zip$/i, "") || product.scene_id?.trim() || "";
  const match = /^S2[ABC]_MSIL(?:1C|2A)_\d{8}T\d{6}_[^_]+_[^_]+_T(\d{2}[A-Z]{3})_\d{8}T\d{6}\.SAFE$/.exec(name);
  return match ? `T${match[1]}` : null;
}
