import type {
  AssetAccessDescriptor,
  BBox,
  ProviderAsset,
  SarProduct,
} from "./generated/v3-contracts.ts";

export const NISAR_LOGICAL_DATASET = "sar.nisar.l.rslc.provisional.v1";
export const NISAR_DATASET_LABEL = "NISAR L-band RSLC (Provisional)";
export const NISAR_PUBLIC_PREVIEW_HOST = "nisar.asf.earthdatacloud.nasa.gov";

const NISAR_PROVIDER_ID = "asf.nasa.earthdata";
const NISAR_COLLECTION_ID = "C2853145197-ASF";
const NISAR_COLLECTION_SHORT_NAME = "NISAR_L1_RSLC_PROVISIONAL_V1";
const NISAR_PROFILE_ID = "asf.nisar.l.rslc.provisional.v1";
const NISAR_IDENTITY_RULE_ID = "asf.nisar.cmr-granule.v1";
const NISAR_FILENAME_RULE_ID = "asf.nisar.rslc-hdf5.v1";
const NISAR_RESOLVER_POLICY_ID = "asf.nisar.earthdata-complete-hdf5.v2";
const NISAR_CRID = "P05023";
const NISAR_PRODUCT_SPECIFICATION_VERSION = "1.5.0";
const NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION = 1;
const MAX_ASSETS = 32;
const MAX_NATIVE_ROLES = 8;
const MAX_RAW_RESOURCES = 128;
const MAX_GRAPH_JSON_BYTES = 40_960;

const REQUIRED_METADATA_KEYS = [
  "dataset",
  "logical_dataset",
  "native_collection_id",
  "collection_profile_id",
  "identity_rule_id",
  "filename_rule_id",
  "delivery_model",
  "resolver_policy_id",
  "provider_product_id",
  "provider_product_key",
  "product_family",
  "processing_level",
  "quality_status",
  "source_kind",
  "product_kind",
  "properties",
  "assets",
] as const;

const METADATA_PROPERTY_MIRROR_KEYS = [
  "band",
  "science_level",
  "production_configuration",
  "crid",
  "product_specification_version",
  "frequency_a_polarization",
  "frequency_b_polarization",
] as const;

const REQUIRED_PROPERTY_KEYS = [
  "provider_product_key",
  "provider_product_id",
  "cmr_granule_concept_id",
  "producer_granule_id",
  "logical_dataset",
  "collection_concept_id",
  "collection_short_name",
  "collection_version",
  "collection_profile_id",
  "identity_rule_id",
  "filename_rule_id",
  "resolver_policy_id",
  "processing_level",
  "science_level",
  "quality_status",
  "production_configuration",
  "band",
  "frame_coverage",
  "frame",
  "track",
  "relative_orbit",
  "flight_direction",
  "look_direction",
  "range_bandwidth",
  "joint_observation",
  "acquisition_end_utc",
  "primary_asset_key",
  "preview_available",
  "full_product",
  "crid",
  "product_specification_version",
  "product_specification_profile_registry_version",
] as const;

const OPTIONAL_PROPERTY_KEYS = [
  "asf_product_id",
  "pge_version",
  "frequency_a_polarization",
  "frequency_b_polarization",
] as const;

type JsonObject = Record<string, unknown>;
type GraphRole = "primary" | "preview" | "browse" | "thumbnail" | "metadata" | "auxiliary";
type Route = { scheme: "https" | "s3"; locator: string };
type ParsedGraphAsset = {
  providerAsset: ProviderAsset;
  checksum: { algorithm: string; value: string } | null;
  routes: Route[];
  productMatch: boolean;
};

function record(value: unknown): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

function exactKeys(value: JsonObject, expected: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  const required = expected.slice().sort();
  return actual.length === required.length
    && actual.every((key, index) => key === required[index]);
}

function closedKeys(
  value: JsonObject,
  required: readonly string[],
  optional: readonly string[],
): boolean {
  const actual = Object.keys(value);
  const allowed = new Set([...required, ...optional]);
  return required.every((key) => Object.hasOwn(value, key))
    && actual.every((key) => allowed.has(key));
}

function text(value: unknown, maximum = 512): string | null {
  return typeof value === "string"
    && value.length > 0
    && value.length <= maximum
    && value === value.trim()
    && !/[\u0000-\u001f\u007f]/.test(value)
    ? value
    : null;
}

function basename(value: unknown): string | null {
  const parsed = text(value);
  return parsed && !parsed.includes("/") && !parsed.includes("\\") && ![".", ".."].includes(parsed)
    ? parsed
    : null;
}

function positiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0
    ? value
    : null;
}

function stringArray(value: unknown, maximum = MAX_NATIVE_ROLES): string[] | null {
  if (!Array.isArray(value) || value.length > maximum) return null;
  const values = value.map((entry) => text(entry, 128));
  if (values.some((entry) => entry === null)) return null;
  const result = values as string[];
  return new Set(result).size === result.length ? result : null;
}

function exactStringSet(value: readonly string[], expected: readonly string[]): boolean {
  return value.length === expected.length && expected.every((entry) => value.includes(entry));
}

function graphScopeIsSafe(value: unknown): boolean {
  if (typeof value === "string") {
    const normalized = value.toLowerCase();
    return !normalized.includes("s-band")
      && !normalized.includes("s_band")
      && !normalized.includes("bhoonidhi")
      && !normalized.includes("/private/");
  }
  if (Array.isArray(value)) return value.every(graphScopeIsSafe);
  const object = record(value);
  return object ? Object.values(object).every(graphScopeIsSafe) : true;
}

function graphIsBounded(value: unknown): boolean {
  try {
    return new TextEncoder().encode(JSON.stringify(value)).length <= MAX_GRAPH_JSON_BYTES;
  } catch {
    return false;
  }
}

function normalizedUtc(value: unknown): string | null {
  const parsed = text(value);
  return parsed
    && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/.test(parsed)
    && Number.isFinite(Date.parse(parsed))
    ? parsed
    : null;
}

function bboxFromGeoJson(geometry: JsonObject | null): BBox | null {
  if (
    !geometry
    || (geometry.type !== "Polygon" && geometry.type !== "MultiPolygon")
    || !Array.isArray(geometry.coordinates)
  ) {
    return null;
  }
  let west = Number.POSITIVE_INFINITY;
  let east = Number.NEGATIVE_INFINITY;
  let south = Number.POSITIVE_INFINITY;
  let north = Number.NEGATIVE_INFINITY;
  let count = 0;
  const visit = (value: unknown) => {
    if (!Array.isArray(value)) return;
    if (
      value.length >= 2
      && typeof value[0] === "number"
      && Number.isFinite(value[0])
      && typeof value[1] === "number"
      && Number.isFinite(value[1])
    ) {
      const longitude = value[0];
      const latitude = value[1];
      if (longitude < -180 || longitude > 180 || latitude < -90 || latitude > 90) return;
      west = Math.min(west, longitude);
      east = Math.max(east, longitude);
      south = Math.min(south, latitude);
      north = Math.max(north, latitude);
      count += 1;
      return;
    }
    value.forEach(visit);
  };
  visit(geometry.coordinates);
  return count > 0 && west <= east && south <= north
    ? { west, east, south, north, crs: "EPSG:4326" }
    : null;
}

function route(value: unknown, fileName: string): Route | null {
  const source = record(value);
  const scheme = source && text(source.scheme, 16);
  const locator = source && text(source.locator, 4096);
  if (!source || !exactKeys(source, ["scheme", "locator"]) || !locator) return null;
  if (scheme !== "https" && scheme !== "s3") return null;
  let parsed: URL;
  try {
    parsed = new URL(locator);
  } catch {
    return null;
  }
  const pathSegments = parsed.pathname.split("/").filter(Boolean);
  const safeCommon = Boolean(parsed.hostname)
    && !parsed.username
    && !parsed.password
    && !parsed.search
    && !parsed.hash
    && !locator.includes("\\")
    && pathSegments[pathSegments.length - 1] === fileName;
  const host = parsed.hostname.toLowerCase();
  const safeHost = scheme === "https"
    ? host === NISAR_PUBLIC_PREVIEW_HOST || host === "example.invalid"
    : [
        "sds-n-cumulus-prod-nisar-products",
        "sds-n-cumulus-prod-nisar-browse",
        "example-invalid",
      ].includes(host);
  const safeScheme = scheme === "https"
    ? parsed.protocol === "https:" && (!parsed.port || parsed.port === "443")
    : parsed.protocol === "s3:" && !parsed.port;
  return safeCommon && safeHost && safeScheme ? { scheme, locator } : null;
}

function checksum(value: unknown): { algorithm: string; value: string } | null | undefined {
  if (value === null) return null;
  const source = record(value);
  if (!source || !exactKeys(source, ["algorithm", "value"])) return undefined;
  const algorithm = text(source.algorithm, 32);
  const digest = text(source.value, 128);
  const expectedLength = algorithm === "MD5" ? 32 : ["SHA-256", "SHA256"].includes(algorithm ?? "") ? 64 : 0;
  return algorithm && digest && digest.length === expectedLength && /^[0-9a-f]+$/.test(digest)
    ? { algorithm, value: digest }
    : undefined;
}

function nativeEvidence(
  value: unknown,
  role: GraphRole,
  fileName: string,
  nativeRoles: readonly string[],
): { productMatch: boolean } | null {
  const source = record(value);
  if (!source || !exactKeys(source, [
    "archive_name",
    "product_match",
    "url_types",
    "url_subtypes",
    "formats",
  ])) return null;
  const archiveName = source.archive_name === null ? null : basename(source.archive_name);
  if (archiveName !== null && archiveName !== fileName) return null;
  if (typeof source.product_match !== "boolean") return null;
  const urlTypes = stringArray(source.url_types);
  const urlSubtypes = stringArray(source.url_subtypes);
  const formats = stringArray(source.formats);
  if (!urlTypes || !urlSubtypes || !formats) return null;
  const allowedNativeRoles = new Set([
    ...urlTypes,
    ...urlSubtypes,
    ...formats.map((format) => `ARCHIVE:${format}`),
  ]);
  if (
    nativeRoles.length === 0
    || nativeRoles.some((entry) => !allowedNativeRoles.has(entry))
    || (archiveName !== null && !formats.some((format) => nativeRoles.includes(`ARCHIVE:${format}`)))
    || (archiveName === null && nativeRoles.some((entry) => entry.startsWith("ARCHIVE:")))
  ) return null;

  const normalizedEvidence = [...nativeRoles, ...urlTypes, ...urlSubtypes].join("\n").toUpperCase();
  if (
    normalizedEvidence.includes("S-BAND")
    || normalizedEvidence.includes("S_BAND")
    || normalizedEvidence.includes("BHOONIDHI")
    || normalizedEvidence.includes("PRIVATE")
  ) return null;

  const productMatch = source.product_match;
  const validIdentity = role === "primary"
    ? archiveName !== null && productMatch
    : role === "preview" || role === "browse" || role === "thumbnail"
      ? !productMatch
      : archiveName !== null && !productMatch;
  if (!validIdentity) return null;

  const lowerName = fileName.toLowerCase();
  const image = (lowerName.endsWith(".png") && exactStringSet(formats, ["PNG"]))
    || ((lowerName.endsWith(".jpg") || lowerName.endsWith(".jpeg"))
      && (exactStringSet(formats, ["JPEG"]) || exactStringSet(formats, ["JPG"])));
  const dataTypes = new Set(["GET DATA", "GET DATA VIA DIRECT ACCESS"]);
  if (
    role === "primary"
    && (
      !exactStringSet(formats, ["HDF5"])
      || urlTypes.length === 0
      || urlTypes.some((entry) => !dataTypes.has(entry))
      || urlSubtypes.length !== 0
    )
  ) return null;
  if (
    (role === "preview" || role === "browse" || role === "thumbnail")
    && (!exactStringSet(urlTypes, ["GET RELATED VISUALIZATION"]) || urlSubtypes.length !== 0)
  ) return null;
  if (
    (role === "metadata" || role === "auxiliary")
    && urlTypes.some((entry) => dataTypes.has(entry))
  ) return null;
  if ((role === "preview" || role === "browse") && (!image || lowerName.includes("thumbnail"))) return null;
  if (role === "thumbnail" && (!image || !lowerName.includes("thumbnail"))) return null;
  return { productMatch };
}

function graphAsset(value: unknown, itemId: string): ParsedGraphAsset | null {
  const source = record(value);
  if (!source || !exactKeys(source, [
    "asset_key",
    "stable_asset_key",
    "role",
    "native_roles",
    "file_name",
    "media_type",
    "size_bytes",
    "checksum",
    "routes",
    "native_evidence",
  ])) return null;
  const assetKey = text(source.asset_key);
  const stableAssetKey = text(source.stable_asset_key, 2048);
  const role = text(source.role) as GraphRole | null;
  const nativeRoles = stringArray(source.native_roles);
  const fileName = basename(source.file_name);
  const mediaType = text(source.media_type, 128);
  const sizeBytes = source.size_bytes === null ? null : positiveInteger(source.size_bytes);
  const integrity = checksum(source.checksum);
  if (
    !assetKey
    || stableAssetKey !== `${itemId}::ASSET::${assetKey}`
    || !role
    || !["primary", "preview", "browse", "thumbnail", "metadata", "auxiliary"].includes(role)
    || !nativeRoles
    || !fileName
    || !mediaType
    || !mediaType.includes("/")
    || source.size_bytes !== null && sizeBytes === null
    || integrity === undefined
    || (sizeBytes === null) !== (integrity === null)
    || !Array.isArray(source.routes)
    || source.routes.length > MAX_RAW_RESOURCES
  ) return null;
  const routes = source.routes.map((entry) => route(entry, fileName));
  if (routes.some((entry) => entry === null)) return null;
  const parsedRoutes = routes as Route[];
  if (new Set(parsedRoutes.map((entry) => `${entry.scheme}\n${entry.locator}`)).size !== parsedRoutes.length) {
    return null;
  }
  const evidence = nativeEvidence(source.native_evidence, role, fileName, nativeRoles);
  if (!evidence) return null;
  const httpsRoutes = parsedRoutes.filter((entry) => entry.scheme === "https");
  if (role === "primary") {
    if (
      mediaType !== "application/x-hdf5"
      || !fileName.toLowerCase().endsWith(".h5")
      || fileName.toLowerCase().endsWith("_qa.h5")
      || fileName.toLowerCase().endsWith("_qa_stats.h5")
      || sizeBytes === null
      || integrity === null
      || httpsRoutes.length !== 1
    ) return null;
  }
  if (role === "preview" || role === "browse" || role === "thumbnail") {
    if (!mediaType.startsWith("image/") || httpsRoutes.length !== 1) return null;
  }
  if ((role === "metadata" || role === "auxiliary") && evidence.productMatch) return null;
  return {
    providerAsset: {
      asset_key: assetKey,
      stable_asset_key: stableAssetKey,
      access_descriptor: null,
      role,
      native_roles: nativeRoles,
      url: role === "preview" || role === "browse" || role === "thumbnail"
        ? httpsRoutes[0].locator
        : null,
      file_name: fileName,
      media_type: mediaType,
      size_bytes: sizeBytes,
    },
    checksum: integrity,
    routes: parsedRoutes,
    productMatch: evidence.productMatch,
  };
}

function descriptor(value: unknown): AssetAccessDescriptor | null {
  const source = record(value);
  const stableHref = source && text(source.stable_href, 4096);
  const resolverParameters = source && record(source.resolver_parameters);
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
    || source.resolver_id !== "earthdata_https"
    || source.scheme !== "earthdata_https"
    || !stableHref
    || source.requires_credentials !== true
    || source.credential_profile !== "earthdata-default"
    || !resolverParameters
    || Object.keys(resolverParameters).length !== 0
  ) return null;
  return {
    resolver_id: "earthdata_https",
    scheme: "earthdata_https",
    stable_href: stableHref,
    requires_credentials: true,
    credential_profile: "earthdata-default",
    resolver_parameters: {},
  };
}

function checksumAlgorithmForExecution(value: string): string {
  return value === "SHA-256" ? "SHA256" : value;
}

function graphOrderIsCanonical(assets: readonly ParsedGraphAsset[]): boolean {
  const ranks: Record<GraphRole, number> = {
    primary: 0,
    preview: 1,
    browse: 2,
    thumbnail: 3,
    metadata: 4,
    auxiliary: 5,
  };
  const compareText = (left: string, right: string) => left < right ? -1 : left > right ? 1 : 0;
  const sorted = assets.slice().sort((left, right) => {
    const leftAsset = left.providerAsset;
    const rightAsset = right.providerAsset;
    return ranks[leftAsset.role as GraphRole] - ranks[rightAsset.role as GraphRole]
      || compareText(leftAsset.file_name ?? "", rightAsset.file_name ?? "")
      || compareText(leftAsset.media_type ?? "", rightAsset.media_type ?? "")
      || compareText(leftAsset.stable_asset_key ?? "", rightAsset.stable_asset_key ?? "");
  });
  return sorted.every((entry, index) => entry === assets[index]);
}

export function nisarProperties(product: SarProduct): JsonObject | null {
  const properties = record(product.native_metadata.properties);
  return properties?.logical_dataset === NISAR_LOGICAL_DATASET ? properties : null;
}

export function isNisarCatalogProduct(product: SarProduct): boolean {
  const properties = nisarProperties(product);
  return product.provider === NISAR_PROVIDER_ID
    && product.dataset === NISAR_LOGICAL_DATASET
    && product.platform === "NISAR"
    && product.instrument === "L-SAR"
    && product.processing_level === "RSLC"
    && product.product_family === NISAR_COLLECTION_SHORT_NAME
    && properties?.collection_concept_id === NISAR_COLLECTION_ID
    && properties.collection_profile_id === NISAR_PROFILE_ID
    && properties.identity_rule_id === NISAR_IDENTITY_RULE_ID
    && properties.filename_rule_id === NISAR_FILENAME_RULE_ID
    && properties.resolver_policy_id === NISAR_RESOLVER_POLICY_ID
    && properties.crid === NISAR_CRID
    && properties.product_specification_version === NISAR_PRODUCT_SPECIFICATION_VERSION
    && properties.product_specification_profile_registry_version === NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
    && properties.quality_status === "Provisional"
    && properties.band === "L";
}

export function isReviewedNisarPublicBrowseUrl(value: string | null | undefined): boolean {
  const raw = value?.trim();
  if (!raw) return false;
  try {
    const parsed = new URL(raw);
    const authority = /^https:\/\/([^/?#]+)/i.exec(raw)?.[1].toLowerCase();
    return parsed.protocol === "https:"
      && authority === NISAR_PUBLIC_PREVIEW_HOST
      && parsed.hostname.toLowerCase() === NISAR_PUBLIC_PREVIEW_HOST
      && !parsed.port
      && !parsed.username
      && !parsed.password
      && !parsed.search
      && !parsed.hash;
  } catch {
    return false;
  }
}

export function nisarMainBrowseAsset(product: SarProduct): ProviderAsset | null {
  if (!isNisarCatalogProduct(product)) return null;
  const previews = (product.assets ?? []).filter((asset) => (
    asset.role === "preview"
    && asset.access_descriptor == null
    && asset.media_type?.toLowerCase().startsWith("image/") === true
    && isReviewedNisarPublicBrowseUrl(asset.url)
  ));
  return previews.length === 1 ? previews[0] : null;
}

export function nisarProductFromFrozenItem(value: unknown): SarProduct | null {
  const item = record(value);
  if (!item || !exactKeys(item, [
    "item_id",
    "provider_id",
    "display_name",
    "acquisition_datetime",
    "footprint_geojson",
    "metadata",
    "payload",
    "asset_access",
    "execution_policy",
  ])) return null;
  const metadata = record(item.metadata);
  const payload = record(item.payload);
  const properties = metadata && record(metadata.properties);
  const graph = metadata && Array.isArray(metadata.assets) ? metadata.assets : null;
  const itemId = text(item.item_id, 2048);
  const productId = metadata && text(metadata.provider_product_id);
  const productKey = metadata && text(metadata.provider_product_key, 2048);
  const displayName = text(item.display_name);
  if (
    !metadata
    || !closedKeys(metadata, REQUIRED_METADATA_KEYS, METADATA_PROPERTY_MIRROR_KEYS)
    || !payload
    || Object.keys(payload).length !== 0
    || !properties
    || !closedKeys(properties, REQUIRED_PROPERTY_KEYS, OPTIONAL_PROPERTY_KEYS)
    || !graph
    || graph.length === 0
    || graph.length > MAX_ASSETS
    || !graphIsBounded(graph)
    || !graphScopeIsSafe(graph)
    || !itemId
    || item.provider_id !== NISAR_PROVIDER_ID
    || !productId
    || !/^G[0-9]+-ASF$/.test(productId)
    || productKey !== itemId
    || !displayName
    || itemId !== `PROVIDER::${NISAR_PROVIDER_ID}::COLLECTION::${NISAR_COLLECTION_ID}::ITEM::${productId}`
    || metadata.dataset !== NISAR_LOGICAL_DATASET
    || metadata.logical_dataset !== NISAR_LOGICAL_DATASET
    || metadata.native_collection_id !== NISAR_COLLECTION_ID
    || metadata.collection_profile_id !== NISAR_PROFILE_ID
    || metadata.identity_rule_id !== NISAR_IDENTITY_RULE_ID
    || metadata.filename_rule_id !== NISAR_FILENAME_RULE_ID
    || metadata.delivery_model !== "archive_first"
    || metadata.resolver_policy_id !== NISAR_RESOLVER_POLICY_ID
    || metadata.product_family !== NISAR_COLLECTION_SHORT_NAME
    || metadata.processing_level !== "RSLC"
    || metadata.quality_status !== "Provisional"
    || metadata.source_kind !== "sar"
    || metadata.product_kind !== "sar_product"
    || METADATA_PROPERTY_MIRROR_KEYS.some((key) => (
      Object.hasOwn(metadata, key) && metadata[key] !== properties[key]
    ))
  ) return null;

  const hasFrequencyA = Object.hasOwn(properties, "frequency_a_polarization");
  const hasFrequencyB = Object.hasOwn(properties, "frequency_b_polarization");
  const frequencyA = !hasFrequencyA
    ? null
    : text(properties.frequency_a_polarization);
  const frequencyB = !hasFrequencyB
    ? null
    : text(properties.frequency_b_polarization);
  const frame = positiveInteger(properties.frame);
  const track = positiveInteger(properties.track);
  const relativeOrbit = positiveInteger(properties.relative_orbit);
  const acquisitionStart = normalizedUtc(item.acquisition_datetime);
  const acquisitionEnd = normalizedUtc(properties.acquisition_end_utc);
  const optionalTextValid = OPTIONAL_PROPERTY_KEYS
    .filter((key) => !key.startsWith("frequency_"))
    .every((key) => properties[key] === undefined || text(properties[key]) !== null);
  if (
    properties.provider_product_key !== itemId
    || properties.provider_product_id !== productId
    || properties.cmr_granule_concept_id !== productId
    || properties.producer_granule_id !== displayName
    || properties.logical_dataset !== NISAR_LOGICAL_DATASET
    || properties.collection_concept_id !== NISAR_COLLECTION_ID
    || properties.collection_short_name !== NISAR_COLLECTION_SHORT_NAME
    || properties.collection_version !== "1"
    || properties.collection_profile_id !== NISAR_PROFILE_ID
    || properties.identity_rule_id !== NISAR_IDENTITY_RULE_ID
    || properties.filename_rule_id !== NISAR_FILENAME_RULE_ID
    || properties.resolver_policy_id !== NISAR_RESOLVER_POLICY_ID
    || properties.crid !== NISAR_CRID
    || properties.product_specification_version !== NISAR_PRODUCT_SPECIFICATION_VERSION
    || properties.product_specification_profile_registry_version !== NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
    || properties.processing_level !== "RSLC"
    || properties.science_level !== "L1"
    || properties.quality_status !== "Provisional"
    || properties.production_configuration !== "PR"
    || properties.band !== "L"
    || !["FULL", "PARTIAL"].includes(String(properties.frame_coverage))
    || !frame
    || !track
    || relativeOrbit !== track
    || !["ASCENDING", "DESCENDING"].includes(String(properties.flight_direction))
    || !["LEFT", "RIGHT"].includes(String(properties.look_direction))
    || !["5", "20+5", "40+5", "77"].includes(String(properties.range_bandwidth))
    || typeof properties.joint_observation !== "boolean"
    || typeof properties.preview_available !== "boolean"
    || properties.full_product !== true
    || !text(properties.primary_asset_key)
    || hasFrequencyA && frequencyA === null
    || hasFrequencyB && frequencyB === null
    || frequencyA !== null && !["HH", "HH+HV", "VV+VH", "HH+HV+VH+VV"].includes(frequencyA)
    || frequencyB !== null && !["HH", "HH+HV", "VV", "VV+VH"].includes(frequencyB)
    || frequencyA === null && frequencyB === null
    || !optionalTextValid
    || !acquisitionStart
    || !acquisitionEnd
    || Date.parse(acquisitionEnd) < Date.parse(acquisitionStart)
  ) return null;

  const parsedAssets = graph.map((entry) => graphAsset(entry, itemId));
  if (parsedAssets.some((entry) => entry === null)) return null;
  const assets = parsedAssets as ParsedGraphAsset[];
  if (!graphOrderIsCanonical(assets)) return null;
  const assetKeys = assets.map((entry) => entry.providerAsset.asset_key!);
  if (new Set(assetKeys).size !== assetKeys.length) return null;
  const primaryAssets = assets.filter((entry) => entry.providerAsset.role === "primary");
  const previewAssets = assets.filter((entry) => entry.providerAsset.role === "preview");
  const browseAssets = assets.filter((entry) => entry.providerAsset.role === "browse");
  const thumbnailAssets = assets.filter((entry) => entry.providerAsset.role === "thumbnail");
  const representedResources = assets.reduce((total, entry) => {
    const evidence = record(graph[assets.indexOf(entry)])?.native_evidence;
    const archiveName = record(evidence)?.archive_name;
    return total + entry.routes.length + (typeof archiveName === "string" ? 1 : 0);
  }, 0);
  if (
    primaryAssets.length !== 1
    || thumbnailAssets.length > 1
    || representedResources > MAX_RAW_RESOURCES
    || previewAssets.length > 1
    || properties.preview_available !== (previewAssets.length + browseAssets.length > 0)
  ) return null;
  const primary = primaryAssets[0];
  const primaryAsset = primary.providerAsset;
  const primaryChecksum = primary.checksum;
  if (
    properties.primary_asset_key !== primaryAsset.asset_key
    || primaryAsset.file_name !== `${displayName}.h5`
    || primaryAsset.media_type !== "application/x-hdf5"
    || !primary.productMatch
    || !primaryChecksum
  ) return null;

  const frozen = record(item.asset_access);
  const reference = frozen && record(frozen.reference);
  const frozenDescriptor = frozen && descriptor(frozen.descriptor);
  const execution = record(item.execution_policy);
  const normalizedGraphAlgorithm = checksumAlgorithmForExecution(primaryChecksum.algorithm);
  const primaryHttpsRoutes = primary.routes.filter((entry) => entry.scheme === "https");
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
    || !frozenDescriptor
    || reference.provider_id !== NISAR_PROVIDER_ID
    || reference.collection_id !== NISAR_COLLECTION_ID
    || reference.provider_product_id !== productId
    || reference.provider_product_key !== itemId
    || reference.canonical_product_key !== null
    || reference.asset_key !== primaryAsset.asset_key
    || reference.stable_asset_key !== primaryAsset.stable_asset_key
    || frozen.role !== "primary"
    || frozen.file_name !== primaryAsset.file_name
    || positiveInteger(frozen.expected_size_bytes) !== primaryAsset.size_bytes
    || frozen.checksum_algorithm !== normalizedGraphAlgorithm
    || frozen.checksum_value !== primaryChecksum.value
    || primaryHttpsRoutes.length !== 1
    || frozenDescriptor.stable_href !== primaryHttpsRoutes[0].locator
    || !execution
    || !exactKeys(execution, ["authority", "policy_id", "collection_profile_id"])
    || execution.authority !== "resolver_authoritative"
    || execution.policy_id !== NISAR_RESOLVER_POLICY_ID
    || execution.collection_profile_id !== NISAR_PROFILE_ID
  ) return null;

  const footprint = record(item.footprint_geojson);
  const footprintBbox = bboxFromGeoJson(footprint);
  if (!footprint || !footprintBbox) return null;
  const projectedAssets = assets.map((entry) => entry === primary
    ? { ...entry.providerAsset, access_descriptor: frozenDescriptor }
    : entry.providerAsset);
  const frequencySummary = [
    frequencyA ? `Frequency A ${frequencyA}` : null,
    frequencyB ? `Frequency B ${frequencyB} (L-band)` : null,
  ].filter((entry): entry is string => entry !== null).join("; ");

  return {
    product_key: itemId,
    provider_product_key: itemId,
    provider_product_id: productId,
    canonical_product_key: null,
    identity_rule_id: NISAR_IDENTITY_RULE_ID,
    product_id: productId,
    scene_id: displayName,
    provider: NISAR_PROVIDER_ID,
    dataset: NISAR_LOGICAL_DATASET,
    platform: "NISAR",
    instrument: "L-SAR",
    processing_level: "RSLC",
    product_family: NISAR_COLLECTION_SHORT_NAME,
    beam_mode: null,
    beam_swath: null,
    polarization: frequencySummary,
    flight_direction: String(properties.flight_direction),
    look_direction: String(properties.look_direction),
    relative_orbit: relativeOrbit,
    absolute_orbit: null,
    frame,
    asf_frame: null,
    path: track,
    acquisition_start: acquisitionStart,
    acquisition_stop: acquisitionEnd,
    footprint_geojson: footprint,
    footprint_bbox: footprintBbox,
    download_url: null,
    file_name: primaryAsset.file_name,
    file_size_remote: primaryAsset.size_bytes,
    checksum: `${normalizedGraphAlgorithm.toLowerCase()}:${primaryChecksum.value}`,
    local_path: null,
    native_metadata: { ...metadata },
    burst: null,
    assets: projectedAssets,
  };
}
