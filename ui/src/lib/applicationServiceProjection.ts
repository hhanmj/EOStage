import { t } from "../i18n/index.ts";
import type {
  DownloadArchiveItem,
  DownloadBatchContext,
  DownloadStatus,
  SceneRow,
} from "./bridge.ts";
import type {
  DesktopLegacyApplicationSnapshot,
  DesktopCoreRestoreSnapshot,
} from "./desktopShellBridge.ts";
import type {
  AssetAccessDescriptor,
  ProviderAsset,
  SarBurstSelection,
  SarProduct,
} from "./generated/v3-contracts.ts";
import {
  ALOS2_LOGICAL_DATASET,
  alos2ProductFromFrozenItem,
} from "./asfAlos2Catalog.ts";
import {
  NISAR_LOGICAL_DATASET,
  nisarProductFromFrozenItem,
} from "./asfNisarCatalog.ts";
import { projectSarProductToScene } from "./sarProduct.ts";
import { sceneFootprintRenderModel } from "./sceneFootprint.ts";
import {
  LANDSAT_C2_L2_LOGICAL_DATASET,
  landsatSourceIdentity,
} from "./landsatCatalog.ts";
import {
  EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
  EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID,
  EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
  EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID,
  EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
  EARTH_SEARCH_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID,
  isValidEarthSearchSentinel2ThumbnailUrl,
  MPC_SENTINEL2_L2A_COLLECTION_ID,
  MPC_SENTINEL2_L2A_FILENAME_RULE_ID,
  MPC_SENTINEL2_L2A_IDENTITY_RULE_ID,
  MPC_SENTINEL2_L2A_PROFILE_ID,
  MPC_SENTINEL2_L2A_PROVIDER_ID,
  MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID,
  SENTINEL2_L1C_LOGICAL_DATASET,
  SENTINEL2_L2A_COLLECTION_ID,
  SENTINEL2_L2A_LOGICAL_DATASET,
  SENTINEL2_L2A_PRODUCT_TYPE,
  SENTINEL2_L2A_PROVIDER_ID,
} from "./sentinel2Catalog.ts";

const SENTINEL2_L1C_PRODUCT_TYPE = "S2MSI1C";
import { asfRestrictedAccessPresentation } from "./asfRestrictedAccess.ts";
import {
  opticalCapabilityForLogicalDataset,
  type OpticalSearchCapability,
} from "./opticalSearchFilters.ts";
import { facetFilters, searchFilterSchema } from "./searchFilterSchema.ts";
import {
  BHOONIDHI_COLLECTION_ID,
  BHOONIDHI_CREDENTIAL_PROFILE,
  BHOONIDHI_DOWNLOAD_URL,
  BHOONIDHI_FILENAME_RULE_ID,
  BHOONIDHI_IDENTITY_RULE_ID,
  BHOONIDHI_LOGICAL_DATASET,
  BHOONIDHI_PROFILE_ID,
  BHOONIDHI_PUBLIC_ASSET_RESOLVER_ID,
  BHOONIDHI_PROVIDER_ID,
  BHOONIDHI_RESOLVER_POLICY_ID,
  isReviewedBhoonidhiPublicThumbnailUrl,
} from "./bhoonidhiSsarCatalog.ts";

type JsonObject = Record<string, unknown>;

type ProjectedTask = {
  task_id: string;
  batch_context: DownloadBatchContext;
  scenes: SceneRow[];
  selected_item_ids: string[];
  checked_item_ids: string[];
  page_index: number;
  page_size: number;
  output_dir: string;
  layout: string;
  state: string;
  revision: number;
  concurrency: number;
  task_paused: boolean;
  active_item_ids: string[];
  paused_item_ids: string[];
  queued_item_ids: string[];
  completed_item_ids: string[];
  failed_item_ids: string[];
  skipped_item_ids: string[];
  cancelled_item_ids: string[];
  operation?: {
    kind: "dem" | "orbit" | "gacos";
    phase: string;
    region_id: string;
    approved_worker_operation: string;
    payload: JsonObject;
  };
};

function gacosTaskKind(operation: ProjectedTask["operation"]): "request" | "local_result" | "result" | null {
  if (operation?.kind !== "gacos") return null;
  switch (operation.approved_worker_operation) {
    case "gacos_manual_request": return "request";
    case "gacos_import_result": return "local_result";
    case "gacos_fetch_result": return "result";
    default: return null;
  }
}

function gacosTaskDisplayName(operation: ProjectedTask["operation"]): string | undefined {
  const kind = gacosTaskKind(operation);
  if (kind === "request") return t("shared.applicationService.gacosRequest");
  if (kind === "local_result") return t("shared.applicationService.gacosLocalImport");
  if (kind === "result") return t("shared.applicationService.gacosResultDownload");
  return undefined;
}

function gacosTaskTypeLabel(operation: ProjectedTask["operation"]): string | undefined {
  const kind = gacosTaskKind(operation);
  if (kind === "request") return "GACOS request";
  if (kind === "local_result") return t("shared.applicationService.gacosLocalImport");
  if (kind === "result") return t("shared.applicationService.gacosResultDownload");
  return undefined;
}

export type ApplicationWorkbenchProjection = {
  batch_context: DownloadBatchContext;
  scenes: SceneRow[];
  checked_item_ids: string[];
};

export type ApplicationSnapshotProjection = {
  workbench: ApplicationWorkbenchProjection | null;
  tasks: ProjectedTask[];
};

function asRecord(value: unknown): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function asFiniteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

function asStringArray(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((item) => typeof item === "string")
    ? value.slice()
    : null;
}

function asArray(value: unknown): unknown[] | null {
  return Array.isArray(value) ? value : null;
}

function asBoolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function recordOrEmpty(value: unknown): JsonObject | null {
  return value === undefined || value === null ? {} : asRecord(value);
}

function stringMetadata(metadata: JsonObject, key: string): string | null {
  return asString(metadata[key]);
}

function numberMetadata(metadata: JsonObject, key: string): number | null {
  return asNumber(metadata[key]);
}

function canonicalUuid(value: string | null): boolean {
  return Boolean(value && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value));
}

function catalogueProperties(
  metadata: JsonObject,
  expectedProductType: string = SENTINEL2_L2A_PRODUCT_TYPE,
): JsonObject | null {
  const properties = asRecord(metadata.properties);
  return properties
    && properties.product_type === expectedProductType
    && asBoolean(properties.online) !== null
    ? properties
    : null;
}

function exactKeys(value: JsonObject, expected: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  const required = expected.slice().sort();
  return actual.length === required.length
    && actual.every((key, index) => key === required[index]);
}

function frozenText(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 && value === value.trim()
    ? value
    : null;
}

function nullableFrozenText(value: unknown): string | null | undefined {
  return value === null ? null : frozenText(value) ?? undefined;
}

function frozenStringArray(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((entry) => frozenText(entry) !== null)
    ? value.slice() as string[]
    : null;
}

/** One Burst TIFF/XML asset exactly as the Rust burst mapper freezes it; anything else fails closed. */
function frozenBurstAsset(value: unknown): ProviderAsset | null {
  const source = asRecord(value);
  if (!source || !exactKeys(source, [
    "asset_key", "stable_asset_key", "access_descriptor", "role", "native_roles",
    "url", "file_name", "media_type", "size_bytes",
  ])) return null;
  const assetKey = frozenText(source.asset_key);
  const stableAssetKey = frozenText(source.stable_asset_key);
  const url = frozenText(source.url);
  const fileName = frozenText(source.file_name);
  const mediaType = frozenText(source.media_type);
  const nativeRoles = frozenStringArray(source.native_roles);
  const sizeBytes = source.size_bytes === null ? null : asNumber(source.size_bytes);
  const role = source.role === "primary" || source.role === "metadata" ? source.role : null;
  if (
    !role
    || !assetKey
    || !stableAssetKey
    || !url
    || !fileName
    || !mediaType
    || !nativeRoles
    || (source.size_bytes !== null && sizeBytes === null)
    || source.access_descriptor !== null
  ) return null;
  return {
    asset_key: assetKey,
    stable_asset_key: stableAssetKey,
    access_descriptor: null,
    role,
    native_roles: nativeRoles,
    url,
    file_name: fileName,
    media_type: mediaType,
    size_bytes: sizeBytes,
  };
}

/**
 * Issue #15: the M1 freeze keeps an SLC-BURST candidate's official TIFF/XML
 * assets and burst identity in metadata; the Workbench needs both to build a
 * Burst task request. A frozen per-asset task item carries no asset list.
 */
function frozenBurstEvidence(
  metadata: JsonObject,
): { assets: ProviderAsset[]; burst: SarBurstSelection | null } | null {
  let assets: ProviderAsset[] = [];
  if (metadata.assets !== undefined) {
    const parsed = asArray(metadata.assets)?.map(frozenBurstAsset);
    if (!parsed || parsed.some((asset) => asset === null)) return null;
    assets = parsed as ProviderAsset[];
  }
  if (metadata.burst === undefined || metadata.burst === null) return { assets, burst: null };
  const burst = asRecord(metadata.burst);
  if (!burst || !Object.keys(burst).every((key) => [
    "burst_id", "full_burst_id", "relative_burst_id", "absolute_burst_id", "subswath", "burst_index",
  ].includes(key))) return null;
  const optionalText = (field: unknown) => field === undefined || field === null ? null : frozenText(field) ?? undefined;
  const optionalInteger = (field: unknown) => field === undefined || field === null ? null : asNumber(field) ?? undefined;
  const selection = {
    burst_id: optionalText(burst.burst_id),
    full_burst_id: optionalText(burst.full_burst_id),
    relative_burst_id: optionalInteger(burst.relative_burst_id),
    absolute_burst_id: optionalInteger(burst.absolute_burst_id),
    subswath: optionalText(burst.subswath),
    burst_index: optionalInteger(burst.burst_index),
  };
  if (Object.values(selection).some((field) => field === undefined)) return null;
  return { assets, burst: selection as SarBurstSelection };
}

function frozenDescriptor(
  value: unknown,
  resolverId: string,
  scheme: AssetAccessDescriptor["scheme"],
  stableHref: string,
  requiresCredentials: boolean,
  credentialProfile: string | null,
): AssetAccessDescriptor | null {
  const source = asRecord(value);
  const resolverParameters = source && asRecord(source.resolver_parameters);
  if (
    !source
    || !exactKeys(source, [
      "resolver_id",
      "scheme",
      "stable_href",
      "requires_credentials",
      "credential_profile",
      "resolver_parameters",
    ])
    || source.resolver_id !== resolverId
    || source.scheme !== scheme
    || source.stable_href !== stableHref
    || source.requires_credentials !== requiresCredentials
    || source.credential_profile !== credentialProfile
    || !resolverParameters
    || Object.keys(resolverParameters).length !== 0
  ) return null;
  return {
    resolver_id: resolverId,
    scheme,
    stable_href: stableHref,
    requires_credentials: requiresCredentials,
    credential_profile: credentialProfile,
    resolver_parameters: {},
  };
}

function safeBhoonidhiPublicUrl(value: unknown): string | null {
  return frozenText(value) && isReviewedBhoonidhiPublicThumbnailUrl(value as string)
    ? value as string
    : null;
}

function bhoonidhiPublicAssetFromFrozenMetadata(
  value: unknown,
  productId: string,
): ProviderAsset | null {
  const source = asRecord(value);
  const assetKey = source && frozenText(source.asset_key);
  const stableAssetKey = source && frozenText(source.stable_asset_key);
  const role = source?.role;
  const url = source && safeBhoonidhiPublicUrl(source.url);
  const access = source && frozenDescriptor(
    source.access_descriptor,
    BHOONIDHI_PUBLIC_ASSET_RESOLVER_ID,
    "public_https",
    url ?? "",
    false,
    null,
  );
  const roles = source && frozenStringArray(source.native_roles);
  const fileName = source && nullableFrozenText(source.file_name);
  const mediaType = source && nullableFrozenText(source.media_type);
  if (
    !source
    || !exactKeys(source, [
      "asset_key",
      "stable_asset_key",
      "access_descriptor",
      "role",
      "native_roles",
      "url",
      "file_name",
      "media_type",
      "size_bytes",
    ])
    || (role !== "metadata" && role !== "thumbnail")
    || assetKey !== role
    || stableAssetKey !== BHOONIDHI_PROVIDER_ID + ":" + BHOONIDHI_COLLECTION_ID + ":" + productId + ":" + assetKey
    || !url
    || !access
    || access.stable_href !== url
    || roles?.length !== 1
    || roles[0] !== assetKey
    || fileName === undefined
    || mediaType === undefined
    || source.size_bytes !== null
  ) return null;
  return {
    asset_key: assetKey,
    stable_asset_key: stableAssetKey,
    access_descriptor: access,
    role: role as "metadata" | "thumbnail",
    native_roles: [assetKey],
    url,
    file_name: fileName,
    media_type: mediaType,
    size_bytes: null,
  };
}

function bhoonidhiPrimaryAssetFromFrozenItem(
  item: JsonObject,
  itemId: string,
  productId: string,
): ProviderAsset | null {
  const frozen = asRecord(item.asset_access);
  const reference = frozen && asRecord(frozen.reference);
  const access = frozen && frozenDescriptor(
    frozen.descriptor,
    BHOONIDHI_RESOLVER_POLICY_ID,
    "oauth_bearer_https",
    BHOONIDHI_DOWNLOAD_URL,
    true,
    BHOONIDHI_CREDENTIAL_PROFILE,
  );
  const fileName = productId + ".h5";
  const stableAssetKey = itemId + "::ASSET::data";
  if (
    !frozen
    || !exactKeys(frozen, [
      "reference",
      "descriptor",
      "role",
      "file_name",
      "expected_size_bytes",
      "checksum_algorithm",
      "checksum_value",
    ])
    || !reference
    || !exactKeys(reference, [
      "provider_id",
      "collection_id",
      "provider_product_id",
      "provider_product_key",
      "canonical_product_key",
      "asset_key",
      "stable_asset_key",
    ])
    || !access
    || reference.provider_id !== BHOONIDHI_PROVIDER_ID
    || reference.collection_id !== BHOONIDHI_COLLECTION_ID
    || reference.provider_product_id !== productId
    || reference.provider_product_key !== itemId
    || reference.canonical_product_key !== null
    || reference.asset_key !== "data"
    || reference.stable_asset_key !== stableAssetKey
    || frozen.role !== "primary"
    || frozen.file_name !== fileName
    || frozen.expected_size_bytes !== null
    || frozen.checksum_algorithm !== null
    || frozen.checksum_value !== null
  ) return null;
  return {
    asset_key: "data",
    stable_asset_key: stableAssetKey,
    access_descriptor: access,
    role: "primary",
    native_roles: ["data"],
    url: null,
    file_name: fileName,
    media_type: "application/x-hdf5",
    size_bytes: null,
  };
}

function bhoonidhiPropertiesMatch(
  properties: JsonObject,
  productId: string,
  displayName: string,
): boolean {
  if (
    frozenText(properties.ID) !== productId
    || frozenText(properties.Online) !== "Y"
    || frozenText(properties.provider_collection) !== BHOONIDHI_COLLECTION_ID
  ) return false;
  for (const [rawKey, normalizedKey] of [
    ["ProductCode", "product_code"],
    ["SoftwareVersion", "software_version"],
    ["Producer", "producer"],
    ["Platform", "platform"],
    ["Instrument", "instrument"],
    ["Mode", "mode"],
    ["Polarization", "polarization"],
  ] as const) {
    const raw = frozenText(properties[rawKey]);
    if (!raw || frozenText(properties[normalizedKey]) !== raw) return false;
  }
  if (frozenText(properties.Platform) !== "NISAR" || frozenText(properties.Instrument) !== "SSAR") {
    return false;
  }
  if (properties.Scene_Identifier !== undefined) {
    if (
      frozenText(properties.Scene_Identifier) !== displayName
      || frozenText(properties.scene_identifier) !== displayName
    ) return false;
  }
  for (const key of ["AcquistionMode", "AcquisitionMode"] as const) {
    if (properties[key] !== undefined && frozenText(properties[key]) !== frozenText(properties.acquisition_mode)) {
      return false;
    }
  }
  return frozenText(properties.Coverage) !== null;
}

function frozenCoordinate(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function bhoonidhiBBoxMatches(value: unknown, expected: JsonObject): boolean {
  const bbox = asRecord(value);
  return Boolean(
    bbox
    && bbox.crs === "EPSG:4326"
    && frozenCoordinate(bbox.west) === expected.west
    && frozenCoordinate(bbox.east) === expected.east
    && frozenCoordinate(bbox.south) === expected.south
    && frozenCoordinate(bbox.north) === expected.north
  );
}

function bhoonidhiProductFromFrozenItem(
  item: JsonObject,
  metadata: JsonObject,
  payload: JsonObject,
): SarProduct | null {
  const itemId = frozenText(item.item_id);
  const providerId = frozenText(item.provider_id);
  const displayName = frozenText(item.display_name);
  const productId = frozenText(metadata.provider_product_id);
  const productKey = frozenText(metadata.provider_product_key);
  const properties = asRecord(metadata.properties);
  const sourceAssets = asArray(metadata.assets);
  const footprint = item.footprint_geojson === null
    ? null
    : sceneFootprintRenderModel(item.footprint_geojson);
  const acquisitionStart = frozenText(item.acquisition_datetime);
  const expectedFileName = productId ? productId + ".h5" : null;
  const expectedItemId = productId
    ? "PROVIDER::" + BHOONIDHI_PROVIDER_ID + "::COLLECTION::" + BHOONIDHI_COLLECTION_ID + "::ITEM::" + productId
    : null;
  if (
    !exactKeys(item, [
      "item_id",
      "provider_id",
      "display_name",
      "acquisition_datetime",
      "footprint_geojson",
      "metadata",
      "payload",
      "asset_access",
      "execution_policy",
    ])
    || !itemId
    || providerId !== BHOONIDHI_PROVIDER_ID
    || !displayName
    || !metadata
    || !payload
    || Object.keys(payload).length !== 0
    || !productId
    || !productKey
    || productKey !== itemId
    || itemId !== expectedItemId
    || metadata.dataset !== BHOONIDHI_LOGICAL_DATASET
    || metadata.logical_dataset !== BHOONIDHI_LOGICAL_DATASET
    || metadata.native_collection_id !== BHOONIDHI_COLLECTION_ID
    || metadata.collection_profile_id !== BHOONIDHI_PROFILE_ID
    || metadata.identity_rule_id !== BHOONIDHI_IDENTITY_RULE_ID
    || metadata.filename_rule_id !== BHOONIDHI_FILENAME_RULE_ID
    || metadata.delivery_model !== "archive_first"
    || metadata.resolver_policy_id !== BHOONIDHI_RESOLVER_POLICY_ID
    || metadata.provider_product_id !== productId
    || metadata.provider_product_key !== itemId
    || metadata.product_family !== BHOONIDHI_COLLECTION_ID
    || metadata.processing_level !== "RSLC"
    || metadata.band !== "S"
    || metadata.source_kind !== "sar"
    || metadata.product_kind !== "sar_slc"
    || metadata.primary_asset_key !== "data"
    || metadata.primary_file_name !== expectedFileName
    || !properties
    || !sourceAssets
    || sourceAssets.length > 2
    || !acquisitionStart
    || !footprint
    || !bhoonidhiPropertiesMatch(properties, productId, displayName)
    || !bhoonidhiBBoxMatches(metadata.footprint_bbox, footprint.bbox)
  ) return null;

  const primary = bhoonidhiPrimaryAssetFromFrozenItem(item, itemId, productId);
  if (!primary || primary.file_name !== metadata.primary_file_name) return null;
  const publicAssets = sourceAssets.map((value) =>
    bhoonidhiPublicAssetFromFrozenMetadata(value, productId));
  if (
    publicAssets.some((asset) => asset === null)
    || new Set(publicAssets.map((asset) => asset!.asset_key)).size !== publicAssets.length
    || new Set(publicAssets.map((asset) => asset!.role)).size !== publicAssets.length
  ) return null;

  const execution = asRecord(item.execution_policy);
  if (
    !execution
    || !exactKeys(execution, ["authority", "policy_id", "collection_profile_id"])
    || execution.authority !== "resolver_authoritative"
    || execution.policy_id !== BHOONIDHI_RESOLVER_POLICY_ID
    || execution.collection_profile_id !== BHOONIDHI_PROFILE_ID
  ) return null;

  return {
    product_key: itemId,
    provider_product_key: itemId,
    provider_product_id: productId,
    canonical_product_key: null,
    identity_rule_id: BHOONIDHI_IDENTITY_RULE_ID,
    product_id: productId,
    scene_id: displayName,
    provider: BHOONIDHI_PROVIDER_ID,
    dataset: BHOONIDHI_LOGICAL_DATASET,
    platform: frozenText(properties.platform),
    instrument: frozenText(properties.instrument),
    processing_level: "RSLC",
    product_family: BHOONIDHI_COLLECTION_ID,
    beam_mode: frozenText(properties.mode) ?? frozenText(properties.Mode),
    beam_swath: null,
    polarization: frozenText(properties.polarization) ?? frozenText(properties.Polarization),
    flight_direction: (() => {
      const value = (
        frozenText(metadata.flight_direction)
        ?? frozenText(properties.flight_direction)
        ?? frozenText(properties.Node)
        ?? frozenText(properties.node)
      )?.toUpperCase();
      if (!value) return null;
      if (value.startsWith("ASC")) return "ASCENDING";
      if (value.startsWith("DESC")) return "DESCENDING";
      return null;
    })(),
    look_direction: frozenText(properties.look_direction) ?? frozenText(properties.Look_dir),
    relative_orbit: asNumber(properties.Track) ?? asNumber(properties.track),
    absolute_orbit: asNumber(properties.Imaging_Orbit) ?? asNumber(properties.imaging_orbit),
    frame: asNumber(properties.Frame) ?? asNumber(properties.frame),
    asf_frame: null,
    path: asNumber(properties.Track) ?? asNumber(properties.track),
    acquisition_start: acquisitionStart,
    acquisition_stop: null,
    footprint_geojson: footprint.geojson as JsonObject,
    footprint_bbox: footprint.bbox,
    download_url: null,
    file_name: expectedFileName,
    file_size_remote: null,
    checksum: null,
    local_path: null,
    native_metadata: { ...metadata },
    burst: null,
    assets: [primary, ...publicAssets as ProviderAsset[]],
  };
}

function sentinel2ProductFromFrozenItem(
  item: JsonObject,
  metadata: JsonObject,
  payload: JsonObject,
): SarProduct | null {
  const itemId = asString(item.item_id);
  const providerId = asString(item.provider_id);
  const displayName = asString(item.display_name);
  const productId = stringMetadata(metadata, "provider_product_id");
  const productKey = stringMetadata(metadata, "provider_product_key");
  const sourceAssets = asArray(metadata.assets);
  const isL2a = metadata.logical_dataset === SENTINEL2_L2A_LOGICAL_DATASET;
  const isL1c = metadata.logical_dataset === SENTINEL2_L1C_LOGICAL_DATASET;
  const cdseSource = providerId === SENTINEL2_L2A_PROVIDER_ID
    && metadata.native_collection_id === SENTINEL2_L2A_COLLECTION_ID;
  const microsoftSource = isL2a
    && providerId === MPC_SENTINEL2_L2A_PROVIDER_ID
    && metadata.native_collection_id === MPC_SENTINEL2_L2A_COLLECTION_ID;
  const earthSearchSource = isL2a
    && providerId === EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID
    && metadata.native_collection_id === EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID;
  const expectedProductType = isL1c ? SENTINEL2_L1C_PRODUCT_TYPE : SENTINEL2_L2A_PRODUCT_TYPE;
  const properties = microsoftSource || earthSearchSource
    ? asRecord(metadata.properties)
    : catalogueProperties(metadata, expectedProductType);
  if (
    !itemId
    || !displayName?.endsWith(".SAFE")
    || !productId
    || productKey !== itemId
    || (!isL2a && !isL1c)
    || metadata.source_kind !== "sentinel2"
    || metadata.product_kind !== "optical_scene"
    || !properties
    || !sourceAssets
    || !payload
    || Object.keys(payload).length !== 0
    || (!cdseSource && !microsoftSource && !earthSearchSource)
  ) {
    return null;
  }

  const platform = /^(S2[ABC])_/.exec(displayName)?.[1] ?? null;
  const footprint = item.footprint_geojson === null ? null : sceneFootprintRenderModel(item.footprint_geojson);
  if (item.footprint_geojson !== null && !footprint) return null;

  if (earthSearchSource) {
    const itemMatch = /^(S2[ABC])_(\d{2}[C-HJ-NP-X][A-Z]{2})_(\d{8})_(\d+)_L2A$/.exec(productId);
    const safeMatch = /^(S2[ABC])_MSIL2A_(\d{8})T\d{6}_N\d{4}_R\d{3}_T(\d{2}[A-Z]{3})_\d{8}T\d{6}\.SAFE$/.exec(displayName);
    const cloudCover = asFiniteNumber(properties.cloud_cover);
    if (
      metadata.collection_profile_id !== EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID
      || metadata.identity_rule_id !== EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID
      || metadata.filename_rule_id !== EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID
      || metadata.resolver_policy_id !== EARTH_SEARCH_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID
      || sourceAssets.length > 1
      || Object.keys(properties).length !== 2
      || properties.product_type !== SENTINEL2_L2A_PRODUCT_TYPE
      || cloudCover === null
      || cloudCover < 0
      || cloudCover > 100
      || !itemMatch
      || !safeMatch
      || itemMatch[1] !== safeMatch[1]
      || itemMatch[2] !== safeMatch[3]
      || itemMatch[3] !== safeMatch[2]
    ) return null;
    const projectedAssets: ProviderAsset[] = [];
    for (const rawAsset of sourceAssets) {
      const asset = asRecord(rawAsset);
      const url = asset && asString(asset.url);
      if (
        !asset
        || asset.role !== "thumbnail"
        || !url
        || !isValidEarthSearchSentinel2ThumbnailUrl(productId, url)
      ) {
        return null;
      }
      projectedAssets.push({
        asset_key: asString(asset.asset_key) ?? "thumbnail",
        stable_asset_key: asString(asset.stable_asset_key) ?? null,
        role: "thumbnail",
        native_roles: asStringArray(asset.native_roles) ?? ["thumbnail"],
        url,
        file_name: asString(asset.file_name) ?? `${productId}_thumbnail.jpg`,
        media_type: asString(asset.media_type) ?? "image/jpeg",
        size_bytes: null,
        access_descriptor: null,
      });
    }
    return {
      product_key: itemId,
      provider_product_key: productKey,
      provider_product_id: productId,
      canonical_product_key: null,
      identity_rule_id: stringMetadata(metadata, "identity_rule_id"),
      product_id: productId,
      scene_id: displayName,
      provider: providerId,
      dataset: "SENTINEL-2 L2A",
      platform,
      instrument: "MSI",
      processing_level: "L2A",
      product_family: SENTINEL2_L2A_PRODUCT_TYPE,
      beam_mode: null,
      beam_swath: null,
      polarization: null,
      flight_direction: null,
      look_direction: null,
      relative_orbit: null,
      absolute_orbit: null,
      frame: null,
      asf_frame: null,
      path: null,
      acquisition_start: asString(item.acquisition_datetime),
      acquisition_stop: null,
      footprint_geojson: footprint?.geojson ?? null,
      footprint_bbox: footprint?.bbox ?? null,
      download_url: null,
      file_name: null,
      file_size_remote: null,
      checksum: null,
      local_path: null,
      native_metadata: { ...metadata },
      burst: null,
      assets: projectedAssets,
    };
  }

  if (microsoftSource) {
    const itemMatch = /^(S2[ABC])_MSIL2A_(\d{8}T\d{6})_(R\d{3})_(T\d{2}[A-Z]{3})_(\d{8}T\d{6})$/.exec(productId);
    const safeMatch = /^(S2[ABC])_MSIL2A_(\d{8}T\d{6})_N\d{4}_(R\d{3})_(T\d{2}[A-Z]{3})_(\d{8}T\d{6})\.SAFE$/.exec(displayName);
    const cloudCover = asFiniteNumber(properties.cloud_cover);
    if (
      metadata.collection_profile_id !== MPC_SENTINEL2_L2A_PROFILE_ID
      || metadata.identity_rule_id !== MPC_SENTINEL2_L2A_IDENTITY_RULE_ID
      || metadata.filename_rule_id !== MPC_SENTINEL2_L2A_FILENAME_RULE_ID
      || metadata.resolver_policy_id !== MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID
      || sourceAssets.length !== 0
      || Object.keys(properties).length !== 2
      || properties.product_type !== SENTINEL2_L2A_PRODUCT_TYPE
      || cloudCover === null
      || cloudCover < 0
      || cloudCover > 100
      || !itemMatch
      || !safeMatch
      || itemMatch[1] !== safeMatch[1]
      || itemMatch[2] !== safeMatch[2]
      || itemMatch[3] !== safeMatch[3]
      || itemMatch[4] !== safeMatch[4]
      || itemMatch[5] !== safeMatch[5]
    ) return null;
    return {
      product_key: itemId,
      provider_product_key: productKey,
      provider_product_id: productId,
      canonical_product_key: null,
      identity_rule_id: stringMetadata(metadata, "identity_rule_id"),
      product_id: productId,
      scene_id: displayName,
      provider: providerId,
      dataset: "SENTINEL-2 L2A",
      platform,
      instrument: "MSI",
      processing_level: "L2A",
      product_family: SENTINEL2_L2A_PRODUCT_TYPE,
      beam_mode: null,
      beam_swath: null,
      polarization: null,
      flight_direction: null,
      look_direction: null,
      relative_orbit: null,
      absolute_orbit: null,
      frame: null,
      asf_frame: null,
      path: null,
      acquisition_start: asString(item.acquisition_datetime),
      acquisition_stop: null,
      footprint_geojson: footprint?.geojson ?? null,
      footprint_bbox: footprint?.bbox ?? null,
      download_url: null,
      file_name: null,
      file_size_remote: null,
      checksum: null,
      local_path: null,
      native_metadata: { ...metadata },
      burst: null,
      assets: [],
    };
  }

  if (!canonicalUuid(productId)) return null;

  const primary = sourceAssets.filter((value) => {
    const asset = asRecord(value);
    const roles = asset && asStringArray(asset.native_roles);
    return asset?.asset_key === "archive"
      && typeof asset.stable_asset_key === "string"
      && asset.role === "primary"
      && roles?.length === 1
      && roles[0] === "archive";
  });
  if (primary.length !== 1) return null;
  const primaryAsset = asRecord(primary[0]);
  const primarySize = primaryAsset && (primaryAsset.size_bytes === undefined ? null : asNumber(primaryAsset.size_bytes));
  if (primaryAsset?.size_bytes !== undefined && primarySize === null) return null;

  const checksums = asArray(properties.checksums);
  const checksum = checksums?.find((value) => {
    const candidate = asRecord(value);
    return ["SHA256", "MD5"].includes(String(candidate?.algorithm ?? ""))
      && typeof candidate?.value === "string";
  });
  const checksumValue = asRecord(checksum);
  const archiveFileName = `${displayName}.zip`;
  return {
    product_key: itemId,
    provider_product_key: productKey,
    provider_product_id: productId,
    canonical_product_key: null,
    identity_rule_id: stringMetadata(metadata, "identity_rule_id"),
    product_id: productId,
    scene_id: displayName,
    provider: providerId,
    dataset: isL1c ? "SENTINEL-2 L1C" : "SENTINEL-2 L2A",
    platform,
    instrument: "MSI",
    processing_level: isL1c ? "L1C" : "L2A",
    product_family: expectedProductType,
    beam_mode: null,
    beam_swath: null,
    polarization: null,
    flight_direction: null,
    look_direction: null,
    relative_orbit: null,
    absolute_orbit: null,
    frame: null,
    asf_frame: null,
    path: null,
    acquisition_start: asString(item.acquisition_datetime),
    acquisition_stop: asString(properties.content_date_end),
    footprint_geojson: footprint?.geojson ?? null,
    footprint_bbox: footprint?.bbox ?? null,
    download_url: null,
    file_name: archiveFileName,
    file_size_remote: primarySize,
    checksum: checksumValue
      ? `${String(checksumValue.algorithm).toLowerCase()}:${String(checksumValue.value)}`
      : null,
    local_path: null,
    native_metadata: { ...metadata },
    burst: null,
    assets: [
      {
        asset_key: "archive",
        stable_asset_key: asString(primaryAsset?.stable_asset_key),
        access_descriptor: null,
        role: "primary",
        native_roles: ["archive"],
        url: null,
        file_name: null,
        media_type: null,
        size_bytes: primarySize,
      },
    ],
  };
}

function landsatProductFromFrozenItem(
  item: JsonObject,
  metadata: JsonObject,
  payload: JsonObject,
): SarProduct | null {
  const itemId = asString(item.item_id);
  const providerId = asString(item.provider_id);
  const displayName = asString(item.display_name);
  const productKey = stringMetadata(metadata, "provider_product_key");
  const providerProductId = stringMetadata(metadata, "provider_product_id");
  const properties = asRecord(metadata.properties);
  const metadataAssets = asArray(metadata.assets);
  if (
    !itemId
    || !providerId
    || !landsatSourceIdentity(providerId, metadata.native_collection_id)
    || !displayName
    || productKey !== itemId
    || metadata.logical_dataset !== LANDSAT_C2_L2_LOGICAL_DATASET
    || metadata.source_kind !== "landsat"
    || metadata.product_kind !== "optical_scene"
    || !properties
    || !metadataAssets
    || metadataAssets.length !== 0
    || Object.keys(payload).length !== 0
  ) return null;

  const entityId = asString(properties.entity_id);
  const landsatProductId = asString(properties.landsat_product_id);
  const datasetAlias = asString(properties.dataset_alias);
  const spacecraft = asString(properties.spacecraft);
  const processingLevel = asString(properties.processing_level);
  const dataType = asString(properties.data_type_l2);
  const category = asString(properties.collection_category);
  const cloudCover = asFiniteNumber(properties.cloud_cover);
  const wrsPath = asString(properties.wrs_path);
  const wrsRow = asString(properties.wrs_row);
  const browseAvailable = asBoolean(properties.browse_available);
  const acquisitionTime = asString(item.acquisition_datetime);
  if (
    !entityId
    || providerProductId !== entityId
    || !landsatProductId
    || landsatProductId !== displayName
    || datasetAlias !== metadata.native_collection_id
    || metadata.collection_profile_id !== landsatSourceIdentity(providerId, metadata.native_collection_id)?.profileId
    || metadata.identity_rule_id !== landsatSourceIdentity(providerId, metadata.native_collection_id)?.identityRuleId
    || !["landsat_8", "landsat_9"].includes(spacecraft ?? "")
    || !["l2sp", "l2sr"].includes(processingLevel ?? "")
    || !dataType
    || !["T1", "T2", "RT"].includes(category ?? "")
    || cloudCover === null
    || cloudCover < 0
    || cloudCover > 100
    || !/^\d{3}$/.test(wrsPath ?? "")
    || !/^\d{3}$/.test(wrsRow ?? "")
    || browseAvailable === null
    || !acquisitionTime
  ) return null;
  const footprint = item.footprint_geojson === null ? null : sceneFootprintRenderModel(item.footprint_geojson);
  if (item.footprint_geojson !== null && !footprint) return null;

  return {
    product_key: itemId,
    provider_product_key: productKey,
    provider_product_id: entityId,
    canonical_product_key: null,
    identity_rule_id: stringMetadata(metadata, "identity_rule_id"),
    product_id: entityId,
    scene_id: landsatProductId,
    provider: providerId,
    dataset: "LANDSAT 8-9 C2 L2",
    platform: spacecraft,
    instrument: dataType.startsWith("OLI_TIRS") ? "OLI/TIRS" : "OLI",
    processing_level: processingLevel,
    product_family: dataType,
    beam_mode: null,
    beam_swath: null,
    polarization: null,
    flight_direction: null,
    look_direction: null,
    relative_orbit: null,
    absolute_orbit: null,
    frame: null,
    asf_frame: null,
    path: null,
    acquisition_start: acquisitionTime,
    acquisition_stop: null,
    footprint_geojson: footprint?.geojson ?? null,
    footprint_bbox: footprint?.bbox ?? null,
    download_url: null,
    file_name: null,
    file_size_remote: null,
    checksum: null,
    local_path: null,
    native_metadata: { ...metadata },
    burst: null,
    assets: [],
  };
}

/**
 * Issue #3 R2 B5: frozen candidates of a schema-described optical product
 * without a dedicated projection (e.g. HLS). Labels come from the reviewed
 * schema; the scientific facts (identity, time, footprint, platform, cloud
 * cover) come from the frozen candidate. No URL or delivery field is read.
 */
function describedOpticalProductFromFrozenItem(
  item: JsonObject,
  metadata: JsonObject,
  payload: JsonObject,
  capability: OpticalSearchCapability,
): SarProduct | null {
  const itemId = asString(item.item_id);
  const providerId = asString(item.provider_id);
  const displayName = asString(item.display_name);
  const productId = stringMetadata(metadata, "provider_product_id");
  const productKey = stringMetadata(metadata, "provider_product_key");
  const properties = asRecord(metadata.properties);
  const assets = asArray(metadata.assets);
  const cloudCover = properties ? asFiniteNumber(properties.cloud_cover) : null;
  if (
    !itemId
    || !providerId
    || !displayName
    || !productId
    || productKey !== itemId
    || metadata.logical_dataset !== capability.logicalDataset
    || metadata.product_kind !== "optical_scene"
    || !properties
    || !assets
    || assets.length !== 0
    || Object.keys(payload).length !== 0
    || (properties.cloud_cover !== undefined && (cloudCover === null || cloudCover < 0 || cloudCover > 100))
  ) {
    return null;
  }
  const footprint = item.footprint_geojson === null ? null : sceneFootprintRenderModel(item.footprint_geojson);
  if (item.footprint_geojson !== null && !footprint) return null;
  const facets = Object.fromEntries(
    facetFilters(searchFilterSchema(capability.schemaId)).map((filter) => [filter.filter_id, filter]),
  );
  const platformValue = typeof properties.platform === "string" ? properties.platform : null;
  const platform = platformValue
    ? facets.platform?.options.find((option) => option.value === platformValue)?.label ?? platformValue
    : null;
  const fixedLabel = (filterId: string) => {
    const filter = facets[filterId];
    return filter?.control === "fixed" ? filter.options[0]?.label ?? null : null;
  };
  return {
    product_key: itemId,
    provider_product_key: productKey,
    provider_product_id: productId,
    canonical_product_key: null,
    identity_rule_id: stringMetadata(metadata, "identity_rule_id"),
    product_id: productId,
    scene_id: displayName,
    provider: providerId,
    dataset: capability.datasetLabel,
    platform,
    instrument: fixedLabel("instrument"),
    processing_level: fixedLabel("product_level"),
    product_family: fixedLabel("product_type") ?? capability.datasetLabel,
    beam_mode: null,
    beam_swath: null,
    polarization: null,
    flight_direction: null,
    look_direction: null,
    relative_orbit: null,
    absolute_orbit: null,
    frame: null,
    asf_frame: null,
    path: null,
    acquisition_start: asString(item.acquisition_datetime),
    acquisition_stop: null,
    footprint_geojson: footprint?.geojson ?? null,
    footprint_bbox: footprint?.bbox ?? null,
    download_url: null,
    file_name: null,
    file_size_remote: null,
    checksum: null,
    local_path: null,
    native_metadata: { ...metadata },
    burst: null,
    assets: [],
  };
}

function productFromFrozenItem(value: unknown): SarProduct | null {
  const item = asRecord(value);
  if (!item) return null;
  const itemId = asString(item.item_id);
  const providerId = asString(item.provider_id);
  const displayName = asString(item.display_name);
  const metadata = recordOrEmpty(item.metadata);
  const payload = recordOrEmpty(item.payload);
  if (!itemId || !providerId || !displayName || !metadata || !payload) return null;
  if (
    metadata.logical_dataset === SENTINEL2_L2A_LOGICAL_DATASET
    || metadata.logical_dataset === SENTINEL2_L1C_LOGICAL_DATASET
  ) {
    return sentinel2ProductFromFrozenItem(item, metadata, payload);
  }
  if (metadata.logical_dataset === LANDSAT_C2_L2_LOGICAL_DATASET) {
    return landsatProductFromFrozenItem(item, metadata, payload);
  }
  if (metadata.logical_dataset === ALOS2_LOGICAL_DATASET) {
    return alos2ProductFromFrozenItem(item);
  }
  if (metadata.logical_dataset === BHOONIDHI_LOGICAL_DATASET) {
    return bhoonidhiProductFromFrozenItem(item, metadata, payload);
  }
  if (metadata.logical_dataset === NISAR_LOGICAL_DATASET) {
    const deliveryKind = metadata.delivery_kind;
    const sourceItemId = metadata.source_item_id;
    const preparedIntent = metadata.nisar_subset_prepared_intent_v2_envelope;
    const subsetPlan = metadata.nisar_subset_plan_v2_envelope;
    if (
      deliveryKind !== undefined && deliveryKind !== "remote_derived_subset_v2"
      || sourceItemId !== undefined && sourceItemId !== itemId
      || preparedIntent !== undefined && !asRecord(preparedIntent)
      || subsetPlan !== undefined && !asRecord(subsetPlan)
    ) return null;
    const catalogueMetadata = { ...metadata };
    delete catalogueMetadata.delivery_kind;
    delete catalogueMetadata.source_item_id;
    delete catalogueMetadata.nisar_subset_prepared_intent_v2_envelope;
    delete catalogueMetadata.nisar_subset_plan_v2_envelope;
    const product = nisarProductFromFrozenItem({ ...item, metadata: catalogueMetadata });
    return product ? { ...product, native_metadata: { ...metadata } } : null;
  }
  const describedOptical = opticalCapabilityForLogicalDataset(metadata.logical_dataset);
  if (describedOptical) {
    return describedOpticalProductFromFrozenItem(item, metadata, payload, describedOptical);
  }
  const productId = stringMetadata(metadata, "product_id") ?? displayName;
  const dataset = stringMetadata(metadata, "dataset");
  const provider = stringMetadata(metadata, "provider") ?? providerId;
  const family = stringMetadata(metadata, "product_family");
  if (!dataset || !family) return null;
  const footprint = item.footprint_geojson === null ? null : sceneFootprintRenderModel(item.footprint_geojson);
  if (item.footprint_geojson !== null && !footprint) return null;
  const burstEvidence = dataset.trim().toUpperCase() === "SLC-BURST"
    ? frozenBurstEvidence(metadata)
    : { assets: [], burst: null };
  if (!burstEvidence) return null;
  return {
    product_key: itemId,
    provider_product_key: stringMetadata(metadata, "provider_product_key"),
    provider_product_id: stringMetadata(metadata, "provider_product_id"),
    canonical_product_key: stringMetadata(metadata, "canonical_product_key"),
    identity_rule_id: stringMetadata(metadata, "identity_rule_id"),
    product_id: productId,
    scene_id: stringMetadata(metadata, "scene_id"),
    provider,
    dataset,
    platform: stringMetadata(metadata, "platform"),
    instrument: stringMetadata(metadata, "instrument"),
    processing_level: stringMetadata(metadata, "processing_level"),
    product_family: family,
    beam_mode: stringMetadata(metadata, "beam_mode"),
    beam_swath: stringMetadata(metadata, "beam_swath"),
    polarization: stringMetadata(metadata, "polarization"),
    flight_direction: stringMetadata(metadata, "flight_direction"),
    look_direction: stringMetadata(metadata, "look_direction"),
    relative_orbit: numberMetadata(metadata, "relative_orbit"),
    absolute_orbit: numberMetadata(metadata, "absolute_orbit"),
    frame: numberMetadata(metadata, "frame"),
    asf_frame: stringMetadata(metadata, "asf_frame"),
    path: numberMetadata(metadata, "path"),
    acquisition_start: asString(item.acquisition_datetime),
    acquisition_stop: stringMetadata(metadata, "acquisition_stop"),
    footprint_geojson: footprint?.geojson ?? null,
    footprint_bbox: footprint?.bbox ?? null,
    download_url: asString(payload.download_url),
    file_name: stringMetadata(metadata, "file_name") ?? asString(payload.file_name),
    file_size_remote: asNumber(payload.file_size_remote),
    checksum: stringMetadata(metadata, "checksum"),
    local_path: null,
    native_metadata: { ...metadata },
    burst: burstEvidence.burst,
    assets: burstEvidence.assets,
  };
}

/** Project one bounded Store page item without requiring a full candidate array. */
export function projectFrozenItemToScene(value: unknown): SceneRow | null {
  const product = productFromFrozenItem(value);
  return product ? projectSarProductToScene(product) : null;
}

function batchContext(batch: JsonObject, workbench: JsonObject): DownloadBatchContext | null {
  const batchId = asString(batch.batch_id);
  const createdAt = asString(batch.created_at);
  const aoiName = asString(batch.aoi_name);
  const aoiGeojson = asRecord(batch.aoi_geojson);
  const query = asRecord(batch.query);
  const pageIndex = asNumber(workbench.page_index);
  const pageSize = asNumber(workbench.page_size);
  const revision = asNumber(workbench.revision);
  if (!batchId || !createdAt || !aoiName || !aoiGeojson || !query || pageIndex === null || pageSize === null || revision === null) {
    return null;
  }
  return {
    batch_id: batchId,
    created_at: createdAt,
    aoi_name: aoiName,
    aoi_geojson: aoiGeojson,
    query,
    page_index: pageIndex,
    page_size: pageSize,
    revision,
  };
}

function scenesFromFrozenCandidates(candidates: unknown): SceneRow[] | null {
  if (!Array.isArray(candidates)) return null;
  const products = candidates.map(productFromFrozenItem);
  return products.every((product): product is SarProduct => product !== null)
    ? products.map(projectSarProductToScene)
    : null;
}

function projectedTask(snapshot: unknown, runtime: unknown): ProjectedTask | null {
  const task = asRecord(snapshot);
  const state = asRecord(runtime);
  if (!task || !state) return null;
  const taskId = asString(task.task_id);
  const runtimeTaskId = asString(state.task_id);
  const batchId = asString(task.batch_id);
  const createdAt = asString(task.created_at);
  const aoiName = asString(task.aoi_name);
  const aoiGeojson = asRecord(task.aoi_geojson);
  const query = asRecord(task.query);
  const outputDir = asString(task.output_root);
  const layout = asString(task.layout);
  const taskState = asString(state.status);
  const revision = asNumber(state.revision);
  const pageIndex = asNumber(state.page_index);
  const pageSize = asNumber(state.page_size);
  const concurrency = asNumber(state.concurrency);
  const taskPaused = typeof state.task_paused === "boolean" ? state.task_paused : null;
  const selected = asStringArray(state.selected_item_ids);
  const checked = asStringArray(state.checked_item_ids);
  const active = asStringArray(state.active_item_ids);
  const paused = asStringArray(state.paused_item_ids);
  const queued = asStringArray(state.queued_item_ids);
  const completed = asStringArray(state.completed_item_ids);
  const failed = asStringArray(state.failed_item_ids);
  const skipped = asStringArray(state.skipped_item_ids);
  const cancelled = asStringArray(state.cancelled_item_ids);
  const operationRecord = asRecord(task.operation);
  const operationKind = operationRecord?.kind;
  const operation = operationRecord && (operationKind === "dem" || operationKind === "orbit" || operationKind === "gacos")
    ? {
        kind: operationKind as "dem" | "orbit" | "gacos",
        phase: asString(operationRecord.phase) ?? "execute",
        region_id: asString(operationRecord.region_id) ?? "",
        approved_worker_operation: asString(operationRecord.approved_worker_operation) ?? "",
        payload: asRecord(operationRecord.payload) ?? {},
      }
    : undefined;
  const scenes = operation ? [] : scenesFromFrozenCandidates(task.candidates);
  if (
    !taskId || taskId !== runtimeTaskId || !batchId || !createdAt || !aoiName || !aoiGeojson || !query ||
    !outputDir || !layout || !taskState || revision === null || pageIndex === null || pageSize === null ||
    concurrency === null || taskPaused === null || !selected || !checked || !active || !paused || !queued ||
    !completed || !failed || !skipped || !cancelled || !scenes
  ) return null;
  return {
    task_id: taskId,
    batch_context: {
      batch_id: batchId,
      source_batch_id: batchId,
      task_batch_id: taskId,
      created_at: createdAt,
      aoi_name: aoiName,
      aoi_geojson: aoiGeojson,
      query,
      page_index: pageIndex,
      page_size: pageSize,
      revision,
    },
    scenes,
    selected_item_ids: selected,
    checked_item_ids: checked,
    page_index: pageIndex,
    page_size: pageSize,
    output_dir: outputDir,
    layout,
    state: taskState,
    revision,
    concurrency,
    task_paused: taskPaused,
    active_item_ids: active,
    paused_item_ids: paused,
    queued_item_ids: queued,
    completed_item_ids: completed,
    failed_item_ids: failed,
    skipped_item_ids: skipped,
    cancelled_item_ids: cancelled,
    ...(operation ? { operation } : {}),
  };
}

/**
 * Legacy/test-only projection for the historical full Application Service
 * snapshot. The packaged Tauri renderer uses CoreRestore/page projections and
 * never calls this compatibility reader.
 */
export function projectApplicationSnapshot(
  snapshot: DesktopLegacyApplicationSnapshot | DesktopCoreRestoreSnapshot,
): ApplicationSnapshotProjection | null {
  if (!snapshot.ok) return null;
  if ("core" in snapshot) return null;
  const restore = asRecord(snapshot.restore);
  if (!restore || !Array.isArray(restore.tasks) || !Array.isArray(restore.runtimes)) return null;

  let workbench: ApplicationWorkbenchProjection | null = null;
  if (restore.workbench_batch !== null || restore.workbench !== null) {
    const batch = asRecord(restore.workbench_batch);
    const state = asRecord(restore.workbench);
    const scenes = batch ? scenesFromFrozenCandidates(batch.candidates) : null;
    const context = batch && state ? batchContext(batch, state) : null;
    const checked = state ? asStringArray(state.checked_item_ids) : null;
    if (!batch || !state || !scenes || !context || !checked) return null;
    workbench = {
      batch_context: { ...context, checked_item_ids: checked },
      scenes,
      checked_item_ids: checked,
    };
  }

  const runtimeByTaskId = new Map<string, unknown>();
  for (const runtime of restore.runtimes) {
    const value = asRecord(runtime);
    const taskId = value ? asString(value.task_id) : null;
    if (!taskId || runtimeByTaskId.has(taskId)) return null;
    runtimeByTaskId.set(taskId, runtime);
  }
  const tasks: ProjectedTask[] = [];
  for (const task of restore.tasks) {
    const value = asRecord(task);
    const taskId = value ? asString(value.task_id) : null;
    if (!taskId) return null;
    const projection = projectedTask(task, runtimeByTaskId.get(taskId));
    if (!projection) return null;
    tasks.push(projection);
  }
  return { workbench, tasks };
}

export function taskWorkbenchFromProjection(task: ProjectedTask) {
  return {
    task_id: task.task_id,
    provider_id: task.operation?.kind === "orbit"
      ? "sentinel1.orbit"
      : task.operation?.kind === "dem"
        ? "compat.dem"
        : gacosTaskKind(task.operation) === "result"
          ? "gacos.manual-result"
          : gacosTaskKind(task.operation) === "local_result"
            ? "gacos.local-result"
            : gacosTaskKind(task.operation) === "request"
              ? "gacos.webform"
              : task.scenes[0]?.provider,
    batch_context: task.batch_context,
    scenes: task.scenes,
    checked_scene_ids: task.checked_item_ids,
    selected_scene_ids: task.selected_item_ids,
    output_dir: task.output_dir,
    download_layout: task.layout,
    use_product_subdirs: task.layout !== "direct",
  };
}

export function taskStatusFromProjection(task: ProjectedTask): DownloadStatus {
  const done = task.completed_item_ids.length + task.skipped_item_ids.length;
  const activeDownloads = task.active_item_ids.map((scene_id) => ({ scene_id, bytes: 0 }));
  const failures = task.failed_item_ids.length + task.cancelled_item_ids.length;
  const isManualGacosRequest = task.operation?.kind === "gacos"
    && task.operation.approved_worker_operation === "gacos_manual_request";
  return {
    ok: true,
    task_id: task.task_id,
    name: gacosTaskDisplayName(task.operation),
    task_type: gacosTaskTypeLabel(task.operation),
    approved_worker_operation: task.operation?.approved_worker_operation ?? null,
    dataset_id: gacosTaskKind(task.operation) ? "gacos.atmosphere" : task.scenes[0]?.dataset ?? null,
    failure_code: null,
    state: task.state,
    revision: task.revision,
    total: task.selected_item_ids.length,
    done,
    concurrency: task.concurrency,
    current_scene: task.active_item_ids[0] ?? "",
    active_downloads: activeDownloads,
    paused: task.state === "paused",
    cancelled: task.state === "cancelled",
    error: null,
    summary_line: `${done}/${task.selected_item_ids.length}`,
    results_path: "",
    output_dir: task.output_dir,
    download_layout: task.layout,
    succeeded: task.completed_item_ids.length,
    skipped: task.skipped_item_ids.length,
    failed: task.failed_item_ids.length,
    interrupted: task.cancelled_item_ids.length,
    has_failures: failures > 0,
    paused_scene_ids: task.paused_item_ids,
    selected_scene_ids: task.selected_item_ids,
    checked_scene_ids: task.checked_item_ids,
    queued_scene_ids: task.queued_item_ids,
    snapshot_scenes: task.scenes,
    aoi_name: task.batch_context.aoi_name,
    batch_context: task.batch_context,
    task_paused: task.task_paused,
    resume_supported: !isManualGacosRequest,
    retry_supported: !isManualGacosRequest
      && (task.failed_item_ids.length > 0 || task.cancelled_item_ids.length > 0),
    log: [],
  };
}

type ProjectedDownloadProgress = {
  item_id: string;
  status: string;
  bytes_completed: number;
  total_bytes: number | null;
  bytes_per_second: number;
  updated_at: string;
};

type ProjectedDownloadResult = {
  item_id: string;
  outcome: string;
  destination: string;
  bytes_written: number;
  failure_code: string | null;
  failure_message: string | null;
  finished_at: string;
};

type ProjectedDownloadSnapshot = {
  task_id: string;
  revision: number;
  progress: ProjectedDownloadProgress[];
  results: ProjectedDownloadResult[];
};

function projectDownloadSnapshot(
  snapshot: unknown,
  task: ProjectedTask,
): ProjectedDownloadSnapshot | null {
  const value = asRecord(snapshot);
  if (!value) return null;
  const taskId = asString(value.task_id);
  const revision = asNumber(value.revision);
  if (!taskId || taskId !== task.task_id || revision === null || !Array.isArray(value.progress) || !Array.isArray(value.results)) {
    return null;
  }

  const knownItemIds = new Set(task.selected_item_ids);
  const progress = value.progress.map((entry) => {
    const record = asRecord(entry);
    if (!record || asString(record.task_id) !== taskId) return null;
    const itemId = asString(record.item_id);
    const status = asString(record.status);
    const bytesCompleted = asNumber(record.bytes_completed);
    const totalBytes = record.total_bytes === undefined
      ? undefined
      : record.total_bytes === null
        ? null
        : asNumber(record.total_bytes);
    const bytesPerSecond = asFiniteNumber(record.bytes_per_second);
    const updatedAt = asString(record.updated_at);
    if (!itemId || !knownItemIds.has(itemId) || !status || bytesCompleted === null ||
      totalBytes === undefined || bytesPerSecond === null || !updatedAt) return null;
    return {
      item_id: itemId,
      status,
      bytes_completed: bytesCompleted,
      total_bytes: totalBytes,
      bytes_per_second: bytesPerSecond,
      updated_at: updatedAt,
    };
  });
  if (progress.some((entry) => entry === null) || new Set(progress.map((entry) => entry!.item_id)).size !== progress.length) {
    return null;
  }

  const results = value.results.map((entry) => {
    const record = asRecord(entry);
    if (!record || asString(record.task_id) !== taskId) return null;
    const itemId = asString(record.item_id);
    const outcome = asString(record.outcome);
    const destination = asString(record.destination);
    const bytesWritten = asNumber(record.bytes_written);
    const finishedAt = asString(record.finished_at);
    const failure = record.failure === null ? null : asRecord(record.failure);
    const failureCode = failure ? asString(failure.code) : null;
    const failureMessage = failure
      ? asString(failure.reason_zh) ?? asString(failure.code)
      : null;
    if (!itemId || !knownItemIds.has(itemId) || !outcome || !destination || bytesWritten === null ||
      !finishedAt || (record.failure !== null && !failure)) return null;
    return {
      item_id: itemId,
      outcome,
      destination,
      bytes_written: bytesWritten,
      failure_code: failureCode,
      failure_message: failureMessage,
      finished_at: finishedAt,
    };
  });
  if (results.some((entry) => entry === null) || new Set(results.map((entry) => entry!.item_id)).size !== results.length) {
    return null;
  }

  return {
    task_id: taskId,
    revision,
    progress: progress as ProjectedDownloadProgress[],
    results: results as ProjectedDownloadResult[],
  };
}

/**
 * Adds the durable download-engine progress/results to a task-runtime
 * projection. Malformed or mismatched engine data is rejected rather than
 * being used to manufacture download progress in the UI.
 */
export function taskStatusFromDownloadSnapshot(
  task: ProjectedTask,
  snapshot: unknown,
): DownloadStatus | null {
  const engine = projectDownloadSnapshot(snapshot, task);
  if (!engine) return null;

  const status = taskStatusFromProjection(task);
  const resultByItemId = new Map(engine.results.map((result) => [result.item_id, result]));
  const progressByItemId = new Map(engine.progress.map((progress) => [progress.item_id, progress]));
  const active = engine.progress.filter((progress) => progress.status === "active");
  const byteTotals = task.selected_item_ids.map((itemId) => {
    const progress = progressByItemId.get(itemId);
    const result = resultByItemId.get(itemId);
    return progress?.total_bytes ?? result?.bytes_written ?? null;
  });
  const totalBytes = byteTotals.every((value) => value !== null)
    ? byteTotals.reduce((sum, value) => sum + (value ?? 0), 0)
    : null;
  const doneBytes = engine.results.reduce((sum, result) => sum + result.bytes_written, 0);
  const currentBytes = active.reduce((sum, progress) => sum + progress.bytes_completed, 0);
  const successfulResult = engine.results.find((result) =>
    ["completed", "skipped"].includes(result.outcome.toLowerCase()),
  );
  const failedResult = engine.results.find((result) => result.failure_code !== null) ?? null;
  const restricted = asfRestrictedAccessPresentation(
    status.dataset_id,
    failedResult?.failure_code,
  );

  return {
    ...status,
    revision: task.revision,
    failure_code: failedResult?.failure_code ?? null,
    error: restricted?.message ?? status.error,
    summary_line: restricted?.message ?? status.summary_line,
    active_downloads: active.map((progress) => ({
      scene_id: progress.item_id,
      bytes: progress.bytes_completed,
      expected_size: progress.total_bytes,
      updated_at: Number.isFinite(Date.parse(progress.updated_at))
        ? Date.parse(progress.updated_at)
        : undefined,
    })),
    total_bytes: totalBytes,
    done_bytes: doneBytes,
    current_bytes: currentBytes,
    current_expected_size: active[0]?.total_bytes ?? null,
    bytes_per_second: active.reduce((sum, progress) => sum + progress.bytes_per_second, 0),
    current_scene: active[0]?.item_id ?? status.current_scene,
    results_path: successfulResult?.destination ?? "",
    log: engine.results.map((result) => ({
      scene_id: result.item_id,
      outcome: result.outcome,
      bytes_written: result.bytes_written,
      message: result.failure_message ?? undefined,
      detail: result.failure_message ?? result.outcome,
      ts: Number.isFinite(Date.parse(result.finished_at)) ? Date.parse(result.finished_at) : undefined,
    })),
  };
}

export function historyItemFromProjection(
  task: ProjectedTask,
  downloadStatus: DownloadStatus | null = null,
): DownloadArchiveItem {
  const status = task.state;
  return {
    id: `asf-task:${task.task_id}`,
    task_id: task.task_id,
    name: task.operation?.kind === "dem"
      ? t("shared.applicationService.demFormalTask")
      : task.operation?.kind === "orbit"
        ? t("shared.applicationService.poeorbFormalTask")
        : gacosTaskDisplayName(task.operation)
          ?? (task.scenes[0]?.dataset
            ? t("shared.applicationService.datasetDownloadTask", { dataset: task.scenes[0].dataset })
            : t("shared.applicationService.sarDownloadTask")),
    status,
    dataset_id: gacosTaskKind(task.operation) ? "gacos.atmosphere" : task.scenes[0]?.dataset ?? null,
    task_type: gacosTaskTypeLabel(task.operation) ?? null,
    approved_worker_operation: task.operation?.approved_worker_operation ?? null,
    failure_code: downloadStatus?.failure_code ?? null,
    detail: downloadStatus?.summary_line || `${task.completed_item_ids.length + task.skipped_item_ids.length}/${task.selected_item_ids.length}`,
    kind: task.operation?.kind ?? "asf",
    output_dir: task.output_dir,
    total: task.selected_item_ids.length,
    concurrency: task.concurrency,
    download_layout: task.layout,
    snapshot: task.scenes,
    scene_ids: task.selected_item_ids,
    aoi_name: task.batch_context.aoi_name,
    batch_context: task.batch_context,
    ts: Date.parse(task.batch_context.created_at) || 0,
    logs: downloadStatus?.log.map((entry) => entry.detail).filter(Boolean) ?? [],
  };
}

export function itemIdsForTask(task: ProjectedTask, ids: string[]): string[] | null {
  const candidates = new Map<string, string>();
  for (const scene of task.scenes) {
    const stable = scene.product_key ?? scene.sar_product?.product_key;
    if (!stable) return null;
    candidates.set(stable, stable);
    candidates.set(scene.scene_id, stable);
    if (scene.product_id) candidates.set(scene.product_id, stable);
  }
  const resolved = ids.map((id) => candidates.get(id) ?? null);
  return resolved.every((id): id is string => id !== null) ? resolved : null;
}
