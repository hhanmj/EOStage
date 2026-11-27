import type {
  AssetAccessDescriptor,
  BBox,
  ProviderAsset,
  SarProduct,
} from "./generated/v3-contracts.ts";

export const ALOS2_LOGICAL_DATASET = "sar.alos-2.palsar-2.scansar.l1.1.v1";
export const ALOS2_DATASET_LABEL = "ALOS-2 PALSAR-2 ScanSAR L1.1";

const ALOS2_PROVIDER_ID = "asf.nasa.earthdata";
const ALOS2_COLLECTION_ID = "C3315903479-ASF";
const ALOS2_PROFILE_ID = "asf.alos-2.palsar-2.scansar.l1.1.v1";
const ALOS2_IDENTITY_RULE_ID = "asf.alos-2.cmr-granule.v1";
const ALOS2_FILENAME_RULE_ID = "asf.alos-2.scansar-zip.v1";
const ALOS2_RESOLVER_POLICY_ID = "asf.alos-2.earthdata-complete-zip.v1";
const ALOS2_PRODUCT_TYPE = "ALOS2_L1_PSR2";

type JsonObject = Record<string, unknown>;

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

function text(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 && value === value.trim()
    ? value
    : null;
}

function nullableText(value: unknown): string | null | undefined {
  return value === null ? null : text(value) ?? undefined;
}

function safeNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : null;
}

function stringArray(value: unknown): string[] | null {
  return Array.isArray(value) && value.every((entry) => text(entry) !== null)
    ? value.slice() as string[]
    : null;
}

function descriptor(value: unknown): AssetAccessDescriptor | null {
  const source = record(value);
  const stableHref = source && text(source.stable_href);
  const credentialProfile = source && text(source.credential_profile);
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
    || text(source.resolver_id) !== "earthdata_https"
    || source.scheme !== "earthdata_https"
    || !stableHref
    || typeof source.requires_credentials !== "boolean"
    || source.requires_credentials !== true
    || credentialProfile !== "earthdata-default"
    || !resolverParameters
    || Object.keys(resolverParameters).length !== 0
  ) {
    return null;
  }
  return {
    resolver_id: "earthdata_https",
    scheme: "earthdata_https",
    stable_href: stableHref,
    requires_credentials: true,
    credential_profile: credentialProfile,
    resolver_parameters: {},
  };
}

function primaryAsset(value: unknown): ProviderAsset | null {
  const source = record(value);
  const assetKey = source && text(source.asset_key);
  const stableAssetKey = source && text(source.stable_asset_key);
  const access = source && descriptor(source.access_descriptor);
  const roles = source && stringArray(source.native_roles);
  const url = source && nullableText(source.url);
  const fileName = source && nullableText(source.file_name);
  const mediaType = source && nullableText(source.media_type);
  const sizeBytes = source && safeNumber(source.size_bytes);
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
    || !assetKey
    || !stableAssetKey
    || !access
    || source.role !== "primary"
    || roles?.length !== 1
    || roles[0] !== "archive"
    || url !== null
    || fileName !== assetKey
    || mediaType !== "application/zip"
    || sizeBytes === null
    || sizeBytes === 0
  ) {
    return null;
  }
  return {
    asset_key: assetKey,
    stable_asset_key: stableAssetKey,
    access_descriptor: access,
    role: "primary",
    native_roles: ["archive"],
    url: null,
    file_name: fileName,
    media_type: "application/zip",
    size_bytes: sizeBytes,
  };
}

function descriptorsMatch(
  left: AssetAccessDescriptor,
  right: AssetAccessDescriptor,
): boolean {
  return left.resolver_id === right.resolver_id
    && left.scheme === right.scheme
    && left.stable_href === right.stable_href
    && left.requires_credentials === right.requires_credentials
    && left.credential_profile === right.credential_profile
    && Object.keys(left.resolver_parameters).length === 0
    && Object.keys(right.resolver_parameters).length === 0;
}

function assetRoutesMatch(properties: JsonObject, archive: ProviderAsset): boolean {
  const assetKey = archive.asset_key;
  const stableHref = archive.access_descriptor?.stable_href;
  const routesByAsset = record(properties.asset_routes);
  const routes = assetKey && routesByAsset && routesByAsset[assetKey];
  if (
    !assetKey
    || !stableHref
    || !routesByAsset
    || Object.keys(routesByAsset).length !== 1
    || !Array.isArray(routes)
    || routes.length === 0
    || routes.length > 8
  ) {
    return false;
  }

  let httpsCount = 0;
  const seen = new Set<string>();
  for (const value of routes) {
    const route = record(value);
    const scheme = route && text(route.scheme);
    const locator = route && text(route.locator);
    if (
      !route
      || !exactKeys(route, ["scheme", "locator"])
      || !scheme
      || !locator
      || seen.has(`${scheme}\n${locator}`)
    ) {
      return false;
    }
    seen.add(`${scheme}\n${locator}`);
    let parsed: URL;
    try {
      parsed = new URL(locator);
    } catch {
      return false;
    }
    if (
      !parsed.hostname
      || parsed.username
      || parsed.password
      || parsed.search
      || parsed.hash
    ) {
      return false;
    }
    const pathSegments = parsed.pathname.split("/").filter(Boolean);
    if (pathSegments[pathSegments.length - 1] !== archive.file_name) return false;
    if (scheme === "https") {
      if (parsed.protocol !== "https:" || (parsed.port && parsed.port !== "443") || locator !== stableHref) {
        return false;
      }
      httpsCount += 1;
    } else if (scheme === "s3") {
      if (parsed.protocol !== "s3:" || parsed.port) return false;
    } else {
      return false;
    }
  }
  return httpsCount === 1;
}

function frozenExecutionMatches(
  item: JsonObject,
  archive: ProviderAsset,
  productId: string,
  productKey: string,
  checksumAlgorithm: string,
  checksumValue: string,
): boolean {
  const frozen = record(item.asset_access);
  const reference = frozen && record(frozen.reference);
  const frozenDescriptor = frozen && descriptor(frozen.descriptor);
  const execution = record(item.execution_policy);
  const archiveDescriptor = archive.access_descriptor;
  return Boolean(
    frozen
    && exactKeys(frozen, [
      "reference",
      "descriptor",
      "role",
      "file_name",
      "expected_size_bytes",
      "checksum_algorithm",
      "checksum_value",
    ])
    && reference
    && exactKeys(reference, [
      "provider_id",
      "collection_id",
      "provider_product_id",
      "provider_product_key",
      "canonical_product_key",
      "asset_key",
      "stable_asset_key",
    ])
    && frozenDescriptor
    && archiveDescriptor
    && descriptorsMatch(frozenDescriptor, archiveDescriptor)
    && reference.provider_id === ALOS2_PROVIDER_ID
    && reference.collection_id === ALOS2_COLLECTION_ID
    && reference.provider_product_id === productId
    && reference.provider_product_key === productKey
    && reference.canonical_product_key === null
    && reference.asset_key === archive.asset_key
    && reference.stable_asset_key === archive.stable_asset_key
    && frozen.role === "primary"
    && frozen.file_name === archive.file_name
    && safeNumber(frozen.expected_size_bytes) === archive.size_bytes
    && frozen.checksum_algorithm === checksumAlgorithm
    && frozen.checksum_value === checksumValue
    && execution
    && exactKeys(execution, ["authority", "policy_id", "collection_profile_id"])
    && execution?.authority === "resolver_authoritative"
    && execution.policy_id === ALOS2_RESOLVER_POLICY_ID
    && execution.collection_profile_id === ALOS2_PROFILE_ID
  );
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
      const [longitude, latitude] = value as number[];
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

function optionalPropertyNumber(properties: JsonObject, key: string): number | null {
  const value = properties[key];
  return value === undefined || value === null ? null : safeNumber(value);
}

export function alos2Properties(product: SarProduct): JsonObject | null {
  const properties = record(product.native_metadata.properties);
  return properties?.logical_dataset === ALOS2_LOGICAL_DATASET ? properties : null;
}

export function isAlos2CatalogProduct(product: SarProduct): boolean {
  const properties = alos2Properties(product);
  return product.provider === ALOS2_PROVIDER_ID
    && product.dataset === ALOS2_LOGICAL_DATASET
    && product.product_family === ALOS2_PRODUCT_TYPE
    && product.processing_level === "1.1"
    && product.platform === "ALOS-2"
    && product.instrument === "PALSAR-2"
    && properties?.collection_concept_id === ALOS2_COLLECTION_ID
    && properties.collection_version === "1"
    && properties.collection_profile_id === ALOS2_PROFILE_ID
    && properties.identity_rule_id === ALOS2_IDENTITY_RULE_ID
    && properties.filename_rule_id === ALOS2_FILENAME_RULE_ID
    && properties.resolver_policy_id === ALOS2_RESOLVER_POLICY_ID;
}

export function alos2ProductFromFrozenItem(value: unknown): SarProduct | null {
  const item = record(value);
  const metadata = item && record(item.metadata);
  const payload = item && record(item.payload);
  const properties = metadata && record(metadata.properties);
  const assets = metadata && Array.isArray(metadata.assets) ? metadata.assets : null;
  const itemId = item && text(item.item_id);
  const providerId = item && text(item.provider_id);
  const displayName = item && text(item.display_name);
  const productId = metadata && text(metadata.provider_product_id);
  const productKey = metadata && text(metadata.provider_product_key);
  if (
    !item
    || !metadata
    || !payload
    || Object.keys(payload).length !== 0
    || !properties
    || !assets
    || !itemId
    || providerId !== ALOS2_PROVIDER_ID
    || !displayName
    || !productId
    || !/^G[0-9]+-ASF$/.test(productId)
    || productKey !== itemId
    || metadata.dataset !== ALOS2_LOGICAL_DATASET
    || metadata.logical_dataset !== ALOS2_LOGICAL_DATASET
    || metadata.native_collection_id !== ALOS2_COLLECTION_ID
    || metadata.collection_profile_id !== ALOS2_PROFILE_ID
    || metadata.identity_rule_id !== ALOS2_IDENTITY_RULE_ID
    || metadata.filename_rule_id !== ALOS2_FILENAME_RULE_ID
    || metadata.delivery_model !== "archive_first"
    || metadata.resolver_policy_id !== ALOS2_RESOLVER_POLICY_ID
    || metadata.product_family !== ALOS2_PRODUCT_TYPE
    || metadata.processing_level !== "1.1"
    || metadata.source_kind !== "sar"
    || metadata.product_kind !== "sar_product"
    || properties.provider_product_key !== itemId
    || properties.provider_product_id !== productId
    || properties.cmr_granule_concept_id !== productId
    || properties.cmr_native_id !== displayName
    || properties.logical_dataset !== ALOS2_LOGICAL_DATASET
    || properties.collection_concept_id !== ALOS2_COLLECTION_ID
    || properties.collection_short_name !== ALOS2_PRODUCT_TYPE
    || properties.collection_version !== "1"
    || properties.collection_profile_id !== ALOS2_PROFILE_ID
    || properties.identity_rule_id !== ALOS2_IDENTITY_RULE_ID
    || properties.filename_rule_id !== ALOS2_FILENAME_RULE_ID
    || properties.resolver_policy_id !== ALOS2_RESOLVER_POLICY_ID
    || properties.platform !== "ALOS-2"
    || properties.instrument !== "PALSAR-2"
    || properties.processing_level !== "1.1"
    || properties.preview_available !== false
  ) {
    return null;
  }
  const expectedItemId = `PROVIDER::${ALOS2_PROVIDER_ID}::COLLECTION::${ALOS2_COLLECTION_ID}::ITEM::${productId}`;
  if (itemId !== expectedItemId) return null;

  const beamMode = text(properties.beam_mode);
  const polarizations = stringArray(properties.polarizations);
  const expectedPolarizations = beamMode === "WBS"
    ? ["HH"]
    : beamMode === "WBD"
      ? ["HH", "HV"]
      : null;
  if (!expectedPolarizations || JSON.stringify(polarizations) !== JSON.stringify(expectedPolarizations)) {
    return null;
  }

  const parsedAssets = assets.map(primaryAsset);
  if (parsedAssets.length !== 1 || parsedAssets[0] === null) return null;
  const archive = parsedAssets[0];
  const assetKey = archive.asset_key!;
  if (archive.stable_asset_key !== `${itemId}::ASSET::${assetKey}`) return null;
  const integrityByAsset = record(properties.asset_integrity);
  const integrity = integrityByAsset && record(integrityByAsset[assetKey]);
  const checksumAlgorithm = integrity && text(integrity.algorithm);
  const checksumValue = integrity && text(integrity.value);
  const expectedChecksumLength = checksumAlgorithm === "MD5" ? 32 : checksumAlgorithm === "SHA256" ? 64 : 0;
  if (
    !integrityByAsset
    || Object.keys(integrityByAsset).length !== 1
    || !integrity
    || !exactKeys(integrity, ["file_name", "size_bytes", "algorithm", "value"])
    || integrity.file_name !== archive.file_name
    || safeNumber(integrity.size_bytes) !== archive.size_bytes
    || !checksumAlgorithm
    || !checksumValue
    || checksumValue.length !== expectedChecksumLength
    || !/^[0-9a-f]+$/.test(checksumValue)
    || !assetRoutesMatch(properties, archive)
    || !frozenExecutionMatches(
      item,
      archive,
      productId,
      productKey,
      checksumAlgorithm,
      checksumValue,
    )
  ) {
    return null;
  }

  const footprint = record(item.footprint_geojson);
  const footprintBbox = bboxFromGeoJson(footprint);
  if (!footprint || !footprintBbox) return null;
  const acquisitionStart = nullableText(item.acquisition_datetime);
  if (acquisitionStart === undefined) return null;
  const acquisitionStop = nullableText(properties.acquisition_end_utc);
  if (acquisitionStop === undefined) return null;

  return {
    product_key: itemId,
    provider_product_key: productKey,
    provider_product_id: productId,
    canonical_product_key: null,
    identity_rule_id: ALOS2_IDENTITY_RULE_ID,
    product_id: productId,
    scene_id: displayName,
    provider: ALOS2_PROVIDER_ID,
    dataset: ALOS2_LOGICAL_DATASET,
    platform: "ALOS-2",
    instrument: "PALSAR-2",
    processing_level: "1.1",
    product_family: ALOS2_PRODUCT_TYPE,
    beam_mode: beamMode,
    beam_swath: null,
    polarization: polarizations!.join("+"),
    flight_direction: nullableText(properties.flight_direction) ?? null,
    look_direction: nullableText(properties.look_direction) ?? null,
    relative_orbit: null,
    absolute_orbit: optionalPropertyNumber(properties, "orbit"),
    frame: optionalPropertyNumber(properties, "frame"),
    asf_frame: null,
    path: optionalPropertyNumber(properties, "path"),
    acquisition_start: acquisitionStart,
    acquisition_stop: acquisitionStop,
    footprint_geojson: footprint,
    footprint_bbox: footprintBbox,
    download_url: null,
    file_name: archive.file_name,
    file_size_remote: archive.size_bytes,
    checksum: `${checksumAlgorithm!.toLowerCase()}:${checksumValue}`,
    local_path: null,
    native_metadata: { ...metadata },
    burst: null,
    assets: [archive],
  };
}
