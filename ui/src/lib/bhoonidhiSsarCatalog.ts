import type { ProviderAsset, SarProduct } from "./generated/v3-contracts.ts";

/** The admitted Bhoonidhi collection is the NISAR S-band SSAR RSLC route. */
export const BHOONIDHI_PROVIDER_ID = "isro.nrsc.bhoonidhi";
export const BHOONIDHI_LOGICAL_DATASET = "sar.nisar.s.ssar.rslc.v1";
export const BHOONIDHI_PROFILE_ID = "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.v1";
export const BHOONIDHI_COLLECTION_ID = "NISAR_SSAR_RSLC";
export const BHOONIDHI_PUBLIC_ASSET_HOST = "bhoonidhi.nrsc.gov.in";
export const BHOONIDHI_SSAR_DATASET_LABEL = "NISAR S-band SSAR RSLC";
export const BHOONIDHI_SSAR_PREVIEW_PROFILE =
  "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.thumbnail.v1";
export const BHOONIDHI_IDENTITY_RULE_ID =
  "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.identity.v1";
export const BHOONIDHI_FILENAME_RULE_ID =
  "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.filename.v1";
export const BHOONIDHI_RESOLVER_POLICY_ID =
  "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.resolver.v1";
export const BHOONIDHI_PUBLIC_ASSET_RESOLVER_ID =
  "isro.nrsc.bhoonidhi.public-asset.v1";
export const BHOONIDHI_DOWNLOAD_URL =
  "https://bhoonidhi-api.nrsc.gov.in/download";
export const BHOONIDHI_CREDENTIAL_PROFILE = "bhoonidhi-default";

type JsonObject = Record<string, unknown>;

function record(value: unknown): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonObject
    : null;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() === value && value.length > 0
    ? value
    : null;
}

function nativeMetadata(product: SarProduct): JsonObject {
  return record(product.native_metadata) ?? {};
}

function nestedProperties(product: SarProduct): JsonObject {
  return record(nativeMetadata(product).properties) ?? {};
}

function metadataValue(product: SarProduct, ...keys: string[]): unknown {
  const native = nativeMetadata(product);
  const properties = nestedProperties(product);
  for (const key of keys) {
    if (Object.hasOwn(native, key) && native[key] !== null && native[key] !== undefined) {
      return native[key];
    }
    if (Object.hasOwn(properties, key) && properties[key] !== null && properties[key] !== undefined) {
      return properties[key];
    }
  }
  return null;
}

/** Return the preserved STAC/provider property map for an admitted S-RSLC item. */
export function bhoonidhiSsarProperties(product: SarProduct): JsonObject | null {
  const properties = nestedProperties(product);
  if (Object.keys(properties).length > 0) return properties;
  const native = nativeMetadata(product);
  return Object.keys(native).length > 0 ? native : null;
}

/**
 * Closed UI identity check for a frozen Bhoonidhi S-RSLC product.
 *
 * The collection/profile identity is required so a provider or dataset label
 * alone cannot opt a product into the S-band route. CP and W remain provider
 * values and are deliberately not interpreted here.
 */
export function isBhoonidhiSsarCatalogProduct(
  product: SarProduct | null | undefined,
): boolean {
  if (!product) return false;
  const platform = text(product.platform) ?? text(metadataValue(product, "platform", "Platform"));
  const instrument = text(product.instrument) ?? text(metadataValue(product, "instrument", "Instrument"));
  const processing = text(product.processing_level)
    ?? text(metadataValue(product, "processing_level"));
  const logicalDataset = text(metadataValue(product, "logical_dataset"));
  const collection = text(metadataValue(
    product,
    "native_collection_id",
    "collection_concept_id",
    "provider_collection",
  ));
  const profile = text(metadataValue(product, "collection_profile_id"));
  const productId = text(product.provider_product_id) ?? text(product.product_id);
  return product.provider === BHOONIDHI_PROVIDER_ID
    && product.dataset === BHOONIDHI_LOGICAL_DATASET
    && platform === "NISAR"
    && instrument === "SSAR"
    && processing === "RSLC"
    && logicalDataset === BHOONIDHI_LOGICAL_DATASET
    && collection === BHOONIDHI_COLLECTION_ID
    && profile === BHOONIDHI_PROFILE_ID
    && productId !== null;
}

function safePublicAssetUrl(value: string | null | undefined): string | null {
  const raw = value?.trim();
  if (!raw) return null;
  try {
    const parsed = new URL(raw);
    return parsed.protocol === "https:"
      && parsed.hostname.toLowerCase() === BHOONIDHI_PUBLIC_ASSET_HOST
      && !parsed.port
      && !parsed.username
      && !parsed.password
      && !parsed.search
      && !parsed.hash
      && parsed.pathname.length > 1
      ? parsed.toString()
      : null;
  } catch {
    return null;
  }
}

/** The only public S-RSLC preview currently admitted by the mapper is thumbnail. */
export function bhoonidhiThumbnailAsset(product: SarProduct): ProviderAsset | null {
  if (!isBhoonidhiSsarCatalogProduct(product)) return null;
  const candidates = (product.assets ?? []).filter((asset) => {
    const url = safePublicAssetUrl(asset.url);
    const mediaType = asset.media_type?.trim().toLowerCase() ?? "";
    return asset.role === "thumbnail"
      && url !== null
      && mediaType.startsWith("image/")
      && asset.access_descriptor?.requires_credentials !== true;
  });
  return candidates.length === 1 ? candidates[0] : null;
}

export function isReviewedBhoonidhiPublicThumbnailUrl(
  value: string | null | undefined,
): boolean {
  return safePublicAssetUrl(value) !== null;
}

export type BhoonidhiSsarSubsetSelection = {
  frequency: "A";
  polarizations: string[];
};

/**
 * Project the reviewed S-band filename polarization token into the HDF5
 * science-channel labels used by the shared NISAR subset V2 contract.
 *
 * This mirrors the backend's frozen SSAR identity rule: provider CP/W remain
 * provider metadata and are never guessed into LSAR channels.
 */
export function bhoonidhiSsarSubsetSelection(
  product: SarProduct | null | undefined,
): BhoonidhiSsarSubsetSelection | null {
  if (!product || !isBhoonidhiSsarCatalogProduct(product)) return null;
  const itemId = product.provider_product_id ?? product.product_id;
  const parts = itemId.split("_");
  if (
    parts.length !== 18
    || parts[0] !== "NISAR"
    || parts[1] !== "S1"
    || parts[3] !== "RSLC"
    || parts[13] !== "P00500"
    || !["F", "P"].includes(parts[15] ?? "")
  ) return null;

  const mapping: Record<string, { provider: string; channels: string[] }> = {
    SHNA: { provider: "SH", channels: ["HH"] },
    SVNA: { provider: "SV", channels: ["VV"] },
    DHNA: { provider: "DH", channels: ["HH", "HV"] },
    DVNA: { provider: "DV", channels: ["VV", "VH"] },
    CLNA: { provider: "CP", channels: ["LH", "LV"] },
    CRNA: { provider: "CP", channels: ["RH", "RV"] },
  };
  const mapped = mapping[parts[9] ?? ""];
  if (!mapped) return null;
  const providerPolarization = bhoonidhiPropertyText(product, "Polarization", "polarization");
  const providerMode = bhoonidhiPropertyText(product, "Mode", "mode");
  if (providerPolarization !== mapped.provider || providerMode !== "W") return null;
  return { frequency: "A", polarizations: mapped.channels.slice() };
}

export function bhoonidhiPropertyText(
  product: SarProduct,
  ...keys: string[]
): string | null {
  return text(metadataValue(product, ...keys));
}
