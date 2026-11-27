import { t } from "../i18n/index.ts";
import { MESSAGES } from "../i18n/locales.ts";
import { formatByteSize } from "./formatByteSize.ts";
import { opticalCapabilityForLogicalDataset, type OpticalSearchCapability } from "./opticalSearchFilters.ts";
import type {
  PreviewDescriptor,
  PreviewErrorKind,
  ProviderAsset,
  ResolvedPreviewAsset,
  SarProduct,
} from "./generated/v3-contracts";
import {
  earthSearchSentinel2ThumbnailAsset,
  isEarthSearchSentinel2Product,
  isMicrosoftSentinel2SearchOnlyProduct,
  isSentinel2L1cCatalogProduct,
  isSentinel2L2aCatalogProduct,
  sentinel2CloudCover,
  sentinel2Online,
  sentinel2Tile,
} from "./sentinel2Catalog.ts";
import {
  isLandsatC2L2CatalogProduct,
  landsatBrowseAvailable,
  landsatCloudCover,
  landsatProperties,
  landsatDownloadDisabledReason,
} from "./landsatCatalog.ts";
import {
  ALOS2_DATASET_LABEL,
  isAlos2CatalogProduct,
} from "./asfAlos2Catalog.ts";
import {
  NISAR_DATASET_LABEL,
  isNisarCatalogProduct,
  isReviewedNisarPublicBrowseUrl,
  nisarMainBrowseAsset,
  nisarProperties,
} from "./asfNisarCatalog.ts";
import {
  BHOONIDHI_SSAR_DATASET_LABEL,
  BHOONIDHI_SSAR_PREVIEW_PROFILE,
  bhoonidhiSsarProperties,
  bhoonidhiThumbnailAsset,
  isBhoonidhiSsarCatalogProduct,
  isReviewedBhoonidhiPublicThumbnailUrl,
} from "./bhoonidhiSsarCatalog.ts";

export type PreviewRuntimeStatus =
  | "not_requested"
  | "loading"
  | "available"
  | "unavailable"
  | "authentication_required"
  | "failed"
  | "cancelled"
  | "stale";

export type PreviewRuntimeState = {
  status: PreviewRuntimeStatus;
  product_key: string | null;
  descriptor: PreviewDescriptor | null;
  url: string | null;
  message: string;
  error_kind?: PreviewErrorKind | "preview_request_failed";
};

export const EMPTY_PREVIEW_STATE: PreviewRuntimeState = {
  status: "not_requested",
  product_key: null,
  descriptor: null,
  url: null,
  get message() { return t("shared.catalogPreview.emptyMessage"); },
};

export function resolveCatalogPreviewBatchId(input: {
  scope: "current" | "task";
  currentBatchId?: string | null;
  taskSourceBatchId?: string | null;
  taskBatchId?: string | null;
  taskSnapshotBatchId?: string | null;
}): string {
  const clean = (value: string | null | undefined) => value?.trim() || "";
  if (input.scope === "current") return clean(input.currentBatchId);
  return clean(input.taskSourceBatchId)
    || clean(input.taskBatchId)
    || clean(input.taskSnapshotBatchId);
}

export type CatalogFieldCell = {
  label: string;
  value: string;
};

/**
 * Disposable UI projection. Never persist this type in SearchBatch, task,
 * SQLite, Provider, or project payloads.
 */
/**
 * ASF missions that can never start a download task in this product: the
 * three legacy missions were removed from ASF's searchable interfaces
 * (Owner evidence 2026-10-31) and are retired in the Registry. The label
 * matches the source presentation ("…（ASF 已下线）").
 */
export const RESTRICTED_SAR_DATASETS: ReadonlySet<string> = new Set([
  "ERS",
  "JERS-1",
  "RADARSAT-1",
]);

/** Typed reason shown for rows of the ASF-removed legacy missions. */
export const ASF_REMOVED_REASON_ZH = MESSAGES["zh-CN"].shared.catalogPreview.asfRemoved;

export function isRestrictedSarDataset(dataset: unknown): boolean {
  return typeof dataset === "string" && RESTRICTED_SAR_DATASETS.has(dataset.trim().toUpperCase());
}

export function restrictedSarReason(dataset: unknown): string {
  if (isRestrictedSarDataset(dataset) && String(dataset).trim().toUpperCase() === "JERS-1") {
    return t("shared.catalogPreview.jersRestricted");
  }
  return t("shared.catalogPreview.asfRemoved");
}

export type CatalogResultRow = {
  provider_product_key: string;
  display_name: string;
  collection: string;
  /** Raw dataset selector (e.g. `JERS-1`) used for the restricted reason lookup. */
  dataset: string;
  provider: string;
  platform: string;
  processing_level: string;
  acquisition_time: string;
  footprint_available: boolean;
  preview_status: PreviewRuntimeStatus;
  file_size_label: string;
  file_size_title?: string;
  asset_summary: string;
  kind: "sar" | "optical";
  domain_field_cells: CatalogFieldCell[];
  selected: boolean;
  active: boolean;
  /** True for restricted missions that are searchable but never downloadable (JERS-1). */
  download_restricted: boolean;
};

export type SarPreviewSource =
  | {
      kind: "candidate";
      product_key: string;
      descriptor: PreviewDescriptor;
      asset: ProviderAsset;
      url: string;
    }
  | {
      kind: "unavailable";
      reason: string;
    };

/**
 * A CDSE quicklook is discovered from a frozen product UUID by the desktop
 * application. It has no browser-loadable OData asset href.
 */
export type CdseQuicklookPreviewSource = {
  kind: "cdse_quicklook";
  product_key: string;
  descriptor: PreviewDescriptor;
};

export type LandsatBrowsePreviewSource = {
  kind: "landsat_browse";
  product_key: string;
  descriptor: PreviewDescriptor;
};

export type CatalogPreviewSource =
  | SarPreviewSource
  | CdseQuicklookPreviewSource
  | LandsatBrowsePreviewSource;

type PreviewCandidateSource = Extract<SarPreviewSource, { kind: "candidate" }>
  | CdseQuicklookPreviewSource
  | LandsatBrowsePreviewSource;

type ResolvedPreviewSource = {
  kind: "resolved_preview";
  product_key: string;
};

type PreviewStateSource = CatalogPreviewSource | ResolvedPreviewSource;

type PreviewLoader = (url: string, signal: AbortSignal) => Promise<void>;
type PreviewPublisher = (state: PreviewRuntimeState) => void;

const PREVIEW_ERROR_MESSAGES: Record<PreviewErrorKind, string> = {
  get provider_no_preview() { return t("shared.catalogPreview.errorMessages.provider_no_preview"); },
  get preview_http_403() { return t("shared.catalogPreview.errorMessages.preview_http_403"); },
  get preview_http_404() { return t("shared.catalogPreview.errorMessages.preview_http_404"); },
  get preview_timeout() { return t("shared.catalogPreview.errorMessages.preview_timeout"); },
  get preview_invalid_media_type() { return t("shared.catalogPreview.errorMessages.preview_invalid_media_type"); },
  get preview_decode_failed() { return t("shared.catalogPreview.errorMessages.preview_decode_failed"); },
};

export function previewErrorMessage(code: string | null | undefined): string {
  return code && code in PREVIEW_ERROR_MESSAGES
    ? PREVIEW_ERROR_MESSAGES[code as PreviewErrorKind]
    : t("shared.catalogPreview.errorMessages.preview_request_failed");
}

/** Typed desktop failure retained through the disposable viewer lifecycle. */
export class PreviewResolutionError extends Error {
  readonly kind: PreviewErrorKind | "preview_request_failed";
  constructor(code?: string | null) {
    const kind = code && Object.hasOwn(PREVIEW_ERROR_MESSAGES, code)
      ? code as PreviewErrorKind : "preview_request_failed";
    super(previewErrorMessage(kind));
    this.name = "PreviewResolutionError";
    this.kind = kind;
  }
}

const IMAGE_ROLES: ProviderAsset["role"][] = ["thumbnail", "preview", "browse"];
const REVIEWED_ASF_PREVIEW_HOST_SUFFIX = ".asf.alaska.edu";
export const ASF_SENTINEL1_GRD_GEOGRAPHIC_PREVIEW_PROFILE =
  "asf.sentinel-1.grd-browse-geographic.v1";

function productIdentity(product: SarProduct): string {
  return product.provider_product_key?.trim() || product.product_key.trim();
}

function displayValue(value: unknown): string {
  if (value === null || value === undefined || value === "") return "—";
  return String(value);
}

function fileSizeLabel(value: number | null | undefined): string {
  return value != null && value > 0 ? formatByteSize(value, "") : "";
}

function downloadableAssetCount(product: SarProduct, assets: readonly ProviderAsset[]): number {
  const mapped = assets.filter((asset) => (
    (product.dataset.toUpperCase() === "SEASAT"
      ? asset.role === "primary"
      : !IMAGE_ROLES.includes(asset.role))
    && Boolean(asset.url?.trim() || asset.access_descriptor)
  )).length;
  return mapped || (product.download_url?.trim() ? 1 : 0);
}

function compactPlatform(value: string | null | undefined): string {
  const normalized = String(value || "").trim();
  const sentinel = /^Sentinel-1([A-D])$/i.exec(normalized);
  if (sentinel) return `S1${sentinel[1].toUpperCase()}`;
  const landsat = /^landsat[_ -]?([89])$/i.exec(normalized);
  if (landsat) return `Landsat ${landsat[1]}`;
  return normalized || "—";
}

function polarizationDisplay(product: SarProduct): string {
  const raw = String(product.polarization || "").trim().toUpperCase();
  const channels: Record<string, string> = {
    DV: "VV+VH (DV)",
    DH: "HH+HV (DH)",
    SV: "VV (SV)",
    SH: "HH (SH)",
  };
  if (channels[raw]) return channels[raw];
  if (product.dataset.toUpperCase() === "SENTINEL-1") {
    if (raw === "VV+VH" || raw === "VV_VH") return "VV+VH (DV)";
    if (raw === "HH+HV" || raw === "HH_HV") return "HH+HV (DH)";
  }
  return raw.replaceAll("_", "+") || "—";
}

function geographicPreviewProfile(product: SarProduct, role: ProviderAsset["role"]): string {
  const reviewedSentinelGrd =
    role === "browse"
    && product.provider.toUpperCase() === "ASF"
    && product.dataset.toUpperCase() === "SENTINEL-1"
    && product.product_family.toUpperCase() === "GRD"
    && Boolean(product.footprint_bbox || product.footprint_geojson);
  return reviewedSentinelGrd
    ? ASF_SENTINEL1_GRD_GEOGRAPHIC_PREVIEW_PROFILE
    : role === "thumbnail"
      ? "sar.provider-thumbnail.v1"
      : "sar.provider-browse.v1";
}

export function isReviewedPublicPreviewUrl(value: string | null | undefined): boolean {
  if (isReviewedBhoonidhiPublicThumbnailUrl(value)) return true;
  if (isReviewedNisarPublicBrowseUrl(value)) return true;
  const url = safeHttpsUrl(value);
  if (!url) return false;
  const host = new URL(url).hostname.toLowerCase();
  return host === "asf.alaska.edu" || host.endsWith(REVIEWED_ASF_PREVIEW_HOST_SUFFIX);
}

/** Preview data returned by the Tauri resolver is already origin-controlled. */
export function isDesktopControlledPreviewDataUrl(value: string | null | undefined): boolean {
  if (!value) return false;
  const match = /^data:(image\/(?:jpeg|png|webp));base64,/.exec(value);
  return Boolean(match && isSafeCdseQuicklookDataUrl(value, match[1]));
}

export function isPreviewActionAllowed(value: string | null | undefined): boolean {
  return isReviewedPublicPreviewUrl(value) || isDesktopControlledPreviewDataUrl(value);
}

export type PublicPreviewSaveRuntime = {
  fetch: (
    url: string,
    init: { credentials: "omit"; cache: "no-store"; signal: AbortSignal | undefined },
  ) => Promise<Response>;
  createObjectUrl: (blob: Blob) => string;
  revokeObjectUrl: (url: string) => void;
  clickDownload: (url: string, fileName: string) => void;
};

function browserPreviewSaveRuntime(): PublicPreviewSaveRuntime {
  return {
    fetch: (url, init) => window.fetch(url, init),
    createObjectUrl: (blob) => URL.createObjectURL(blob),
    revokeObjectUrl: (url) => URL.revokeObjectURL(url),
    clickDownload: (url, fileName) => {
      const anchor = document.createElement("a");
      anchor.href = url;
      anchor.download = fileName;
      anchor.rel = "noopener noreferrer";
      anchor.style.display = "none";
      document.body.appendChild(anchor);
      anchor.click();
      anchor.remove();
    },
  };
}

function safePreviewFileName(value: string): string {
  const sanitized = value.trim().replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_");
  return sanitized || "product-preview.jpg";
}

export async function saveReviewedPublicPreview(
  url: string,
  suggestedFileName: string,
  runtime: PublicPreviewSaveRuntime = browserPreviewSaveRuntime(),
  signal?: AbortSignal,
): Promise<void> {
  if (!isReviewedPublicPreviewUrl(url)) {
    throw new Error("Preview URL is not an approved public Preview host");
  }
  const response = await runtime.fetch(url, {
    credentials: "omit",
    cache: "no-store",
    signal,
  });
  if (!response.ok) {
    throw new Error(`Preview request failed with HTTP ${response.status}`);
  }
  const blob = await response.blob();
  if (!blob.type.toLowerCase().startsWith("image/")) {
    throw new Error("Preview response is not an image");
  }
  const objectUrl = runtime.createObjectUrl(blob);
  try {
    runtime.clickDownload(objectUrl, safePreviewFileName(suggestedFileName));
  } finally {
    runtime.revokeObjectUrl(objectUrl);
  }
}

function assetIdentity(product: SarProduct, asset: ProviderAsset, index: number): string {
  return (
    asset.stable_asset_key?.trim()
    || asset.asset_key?.trim()
    || `${productIdentity(product)}::PREVIEW::${asset.role}::${index}`
  );
}

function inferredImageMediaType(asset: ProviderAsset): string | null {
  const declared = asset.media_type?.trim().toLowerCase() || "";
  if (declared.startsWith("image/")) return declared;
  const cleanUrl = asset.url?.split(/[?#]/, 1)[0].toLowerCase() || "";
  if (cleanUrl.endsWith(".jpg") || cleanUrl.endsWith(".jpeg")) return "image/jpeg";
  if (cleanUrl.endsWith(".png")) return "image/png";
  if (cleanUrl.endsWith(".webp")) return "image/webp";
  return null;
}

function safeHttpsUrl(value: string | null | undefined): string | null {
  if (!value?.trim()) return null;
  try {
    const parsed = new URL(value);
    return parsed.protocol === "https:" ? parsed.toString() : null;
  } catch {
    return null;
  }
}

export function deriveSarPreview(product: SarProduct): SarPreviewSource {
  const assets = product.assets ?? [];
  const imageRoles = product.dataset.toUpperCase() === "SEASAT"
    ? (["browse", "thumbnail"] as const)
    : IMAGE_ROLES;
  for (const role of imageRoles) {
    for (let index = 0; index < assets.length; index += 1) {
      const asset = assets[index];
      if (asset.role !== role) continue;
      const url = safeHttpsUrl(asset.url);
      const mediaType = inferredImageMediaType(asset);
      if (!url || !mediaType) continue;
      const credentialRequirement = asset.access_descriptor?.requires_credentials
        ? "provider_credentials"
        : "none";
      const previewKind = role === "thumbnail" ? "thumbnail" : "quicklook";
      return {
        kind: "candidate",
        product_key: productIdentity(product),
        asset,
        url,
        descriptor: {
          preview_kind: previewKind,
          source_asset_keys: [assetIdentity(product, asset, index)],
          render_profile_id:
            geographicPreviewProfile(product, role),
          media_type: mediaType,
          credential_requirement: credentialRequirement,
          cache_policy: credentialRequirement === "none" ? "session" : "no_store",
          provenance:
            role === "thumbnail"
              ? "Provider thumbnail asset; display only"
              : "Provider browse asset; display only",
        },
      };
    }
  }
  return {
    kind: "unavailable",
    reason: t("shared.catalogPreview.noThumbnailOrBrowse"),
  };
}

/**
 * CDSE quicklooks are optional runtime discovery results, not frozen browser
 * URLs. The named desktop port materializes a bounded image only for the
 * current frozen product selection.
 */
export function deriveCatalogPreview(product: SarProduct): CatalogPreviewSource {
  if (isLandsatC2L2CatalogProduct(product)) {
    if (!landsatBrowseAvailable(product)) {
      return {
        kind: "unavailable",
        reason: landsatDownloadDisabledReason(product) ?? t("shared.catalogPreview.noLandsatBrowse"),
      };
    }
    const key = productIdentity(product);
    return {
      kind: "landsat_browse",
      product_key: key,
      descriptor: {
        preview_kind: "quicklook",
        source_asset_keys: [key],
        render_profile_id: "usgs.landsat.c2.l2.exact-scene-browse.v1",
        media_type: "image/*",
        credential_requirement: "provider_credentials",
        cache_policy: "no_store",
        provenance: t("shared.catalogPreview.landsatBrowseProvenance"),
      },
    };
  }
  if (isBhoonidhiSsarCatalogProduct(product)) {
    const thumbnail = bhoonidhiThumbnailAsset(product);
    const url = thumbnail ? safeHttpsUrl(thumbnail.url) : null;
    const mediaType = thumbnail ? inferredImageMediaType(thumbnail) : null;
    if (!thumbnail || !url || !mediaType) {
      return {
        kind: "unavailable",
        reason: t("shared.catalogPreview.noBhoonidhiThumbnail", { dataset: BHOONIDHI_SSAR_DATASET_LABEL }),
      };
    }
    const thumbnailIndex = product.assets?.indexOf(thumbnail) ?? 0;
    return {
      kind: "candidate",
      product_key: productIdentity(product),
      asset: thumbnail,
      url,
      descriptor: {
        preview_kind: "thumbnail",
        source_asset_keys: [assetIdentity(product, thumbnail, Math.max(0, thumbnailIndex))],
        render_profile_id: BHOONIDHI_SSAR_PREVIEW_PROFILE,
        media_type: mediaType,
        credential_requirement: "none",
        cache_policy: "session",
        provenance: t("shared.catalogPreview.bhoonidhiThumbnailProvenance"),
      },
    };
  }
  if (isNisarCatalogProduct(product)) {
    const preview = nisarMainBrowseAsset(product);
    const url = preview?.url ?? null;
    const mediaType = preview ? inferredImageMediaType(preview) : null;
    if (!preview || !url || !mediaType) {
      return {
        kind: "unavailable",
        reason: t("shared.catalogPreview.noNisarPreview"),
      };
    }
    return {
      kind: "candidate",
      product_key: productIdentity(product),
      asset: preview,
      url,
      descriptor: {
        preview_kind: "quicklook",
        source_asset_keys: [assetIdentity(product, preview, 0)],
        render_profile_id: "asf.nisar.complete-rslc.main-latlon.v1",
        media_type: mediaType,
        credential_requirement: "none",
        cache_policy: "session",
        provenance: t("shared.catalogPreview.nisarPreviewProvenance"),
      },
    };
  }
  if (isAlos2CatalogProduct(product)) {
    return {
      kind: "unavailable",
      reason: t("shared.catalogPreview.noAlos2Browse"),
    };
  }
  if (isSentinel2L2aCatalogProduct(product) || isSentinel2L1cCatalogProduct(product)) {
    if (isMicrosoftSentinel2SearchOnlyProduct(product)) {
      return {
        kind: "unavailable",
        reason: t("shared.catalogPreview.microsoftS2SearchOnly"),
      };
    }
    if (isEarthSearchSentinel2Product(product)) {
      const thumbnail = earthSearchSentinel2ThumbnailAsset(product);
      const url = thumbnail?.url?.trim() ?? null;
      if (!thumbnail || !url) {
        return {
          kind: "unavailable",
          reason: t("shared.catalogPreview.invalidS2ArchiveEvidence"),
        };
      }
      const thumbIdx = (product.assets ?? []).indexOf(thumbnail);
      return {
        kind: "candidate",
        product_key: productIdentity(product),
        asset: thumbnail,
        url,
        descriptor: {
          preview_kind: "quicklook",
          source_asset_keys: [assetIdentity(product, thumbnail, Math.max(0, thumbIdx))],
          render_profile_id: "earth-search.sentinel-2-l2a.thumbnail.v1",
          media_type: "image/jpeg",
          credential_requirement: "none",
          cache_policy: "session",
          provenance: "Element 84 Earth Search · AWS Sentinel-2 Public Quicklook",
        },
      };
    }
    const archive = (product.assets ?? []).find((asset) => asset.role === "primary");
    const archiveKey = archive?.stable_asset_key?.trim() ?? "";
    if (!archiveKey) {
      return {
        kind: "unavailable",
        reason: t("shared.catalogPreview.invalidS2ArchiveEvidence"),
      };
    }
    return {
      kind: "cdse_quicklook",
      product_key: productIdentity(product),
      descriptor: {
        preview_kind: "thumbnail",
        source_asset_keys: [archiveKey],
        render_profile_id: "cdse.sentinel-2.static-quicklook.v1",
        // The actual reviewed image type is supplied by the desktop response.
        media_type: "image/*",
        credential_requirement: "none",
        cache_policy: "no_store",
        provenance: t("shared.catalogPreview.cdseQuicklookProvenance"),
      },
    };
  }
  return deriveSarPreview(product);
}

function landsatCatalogRow(
  product: SarProduct,
  key: string,
  selectedKeys: ReadonlySet<string>,
  activeKey: string | null,
  previewState: PreviewRuntimeState | undefined,
): CatalogResultRow {
  const properties = landsatProperties(product);
  const cloud = landsatCloudCover(product);
  const wrsPath = typeof properties.wrs_path === "string" ? properties.wrs_path : null;
  const wrsRow = typeof properties.wrs_row === "string" ? properties.wrs_row : null;
  const category = typeof properties.collection_category === "string"
    ? properties.collection_category
    : null;
  return {
    provider_product_key: key,
    display_name: product.scene_id?.trim() || product.product_id,
    collection: product.dataset,
    dataset: product.dataset,
    provider: product.provider,
    platform: compactPlatform(product.platform),
    processing_level: displayValue(product.processing_level).toUpperCase(),
    acquisition_time: displayValue(product.acquisition_start),
    footprint_available: Boolean(product.footprint_geojson),
    preview_status: previewState?.status ?? "not_requested",
    file_size_label: fileSizeLabel(product.file_size_remote) || "—",
    asset_summary: "",
    kind: "optical",
    domain_field_cells: [
      { label: "Cloud", value: cloud === null ? "—" : `${cloud}%` },
      { label: "WRS", value: wrsPath && wrsRow ? `${wrsPath}/${wrsRow}` : "—" },
      { label: "Category", value: category ?? "—" },
    ],
    selected: selectedKeys.has(key),
    active: activeKey === key,
    download_restricted: false,
  };
}

function nisarCatalogRow(
  product: SarProduct,
  key: string,
  selectedKeys: ReadonlySet<string>,
  activeKey: string | null,
  previewState: PreviewRuntimeState | undefined,
): CatalogResultRow {
  const native = nisarProperties(product) ?? {};
  const primary = (product.assets ?? []).find((asset) => asset.role === "primary");
  const reportedSize = Number(product.file_size_remote ?? 0) > 0
    ? product.file_size_remote
    : primary?.size_bytes;
  return {
    provider_product_key: key,
    display_name: product.file_name?.trim() || product.scene_id?.trim() || product.product_id,
    collection: NISAR_DATASET_LABEL,
    dataset: product.dataset,
    provider: product.provider,
    platform: "NISAR",
    processing_level: "RSLC · Provisional",
    acquisition_time: displayValue(product.acquisition_start),
    footprint_available: Boolean(product.footprint_geojson || product.footprint_bbox),
    preview_status: previewState?.status ?? "not_requested",
    file_size_label: fileSizeLabel(reportedSize),
    asset_summary: t("shared.catalogPreview.fullHdf5"),
    kind: "sar",
    domain_field_cells: [
      { label: "Direction", value: displayValue(product.flight_direction).toUpperCase() },
      { label: "Coverage", value: displayValue(native.frame_coverage) },
      { label: "Freq A", value: displayValue(native.frequency_a_polarization) },
      { label: "Freq B (L)", value: displayValue(native.frequency_b_polarization) },
      { label: "Range BW", value: displayValue(native.range_bandwidth) },
      {
        label: "Joint L/S",
        value: native.joint_observation === true ? "Yes · metadata only" : "No · metadata only",
      },
    ],
    selected: selectedKeys.has(key),
    active: activeKey === key,
    download_restricted: false,
  };
}

function bhoonidhiCatalogRow(
  product: SarProduct,
  key: string,
  selectedKeys: ReadonlySet<string>,
  activeKey: string | null,
  previewState: PreviewRuntimeState | undefined,
): CatalogResultRow {
  const properties = bhoonidhiSsarProperties(product) ?? {};
  const reportedSize = Number(product.file_size_remote ?? 0) > 0
    ? product.file_size_remote
    : (product.assets ?? []).find((asset) => asset.role === "primary")?.size_bytes;
  const property = (...keys: string[]) => {
    for (const propertyKey of keys) {
      const value = properties[propertyKey];
      if (value !== null && value !== undefined && value !== "") return displayValue(value);
    }
    return "—";
  };
  return {
    provider_product_key: key,
    display_name: product.file_name?.trim() || product.scene_id?.trim() || product.product_id,
    collection: BHOONIDHI_SSAR_DATASET_LABEL,
    dataset: product.dataset,
    provider: product.provider,
    platform: "NISAR",
    processing_level: displayValue(product.processing_level || product.product_family).toUpperCase(),
    acquisition_time: displayValue(product.acquisition_start),
    footprint_available: Boolean(product.footprint_geojson || product.footprint_bbox),
    preview_status: previewState?.status ?? "not_requested",
    file_size_label: fileSizeLabel(reportedSize),
    asset_summary: t("shared.catalogPreview.fullHdf5"),
    kind: "sar",
    domain_field_cells: [
      { label: "Direction", value: displayValue(product.flight_direction || property("flight_direction", "Node", "node")).toUpperCase() },
      { label: "Coverage", value: property("Coverage", "coverage") },
      { label: "Polarization", value: property("Polarization", "polarization") },
      { label: "Mode", value: property("Mode", "mode") },
      { label: "Acquisition", value: property("AcquistionMode", "AcquisitionMode", "acquisition_mode") },
      { label: "Online", value: property("Online", "online") },
    ],
    selected: selectedKeys.has(key),
    active: activeKey === key,
    download_restricted: false,
  };
}

function alos2CatalogRow(
  product: SarProduct,
  key: string,
  selectedKeys: ReadonlySet<string>,
  activeKey: string | null,
  previewState: PreviewRuntimeState | undefined,
): CatalogResultRow {
  const primary = (product.assets ?? []).find((asset) => asset.role === "primary");
  const reportedSize = Number(product.file_size_remote ?? 0) > 0
    ? product.file_size_remote
    : primary?.size_bytes;
  return {
    provider_product_key: key,
    display_name: product.file_name?.trim() || product.scene_id?.trim() || product.product_id,
    collection: ALOS2_DATASET_LABEL,
    dataset: product.dataset,
    provider: product.provider,
    platform: "ALOS-2",
    processing_level: "Level 1.1",
    acquisition_time: displayValue(product.acquisition_start),
    footprint_available: Boolean(product.footprint_geojson || product.footprint_bbox),
    preview_status: previewState?.status ?? "not_requested",
    file_size_label: fileSizeLabel(reportedSize),
    asset_summary: t("shared.catalogPreview.fullZip"),
    kind: "sar",
    domain_field_cells: [
      { label: "Direction", value: displayValue(product.flight_direction).toUpperCase() },
      { label: "Beam", value: displayValue(product.beam_mode) },
      { label: "Polarization", value: polarizationDisplay(product) },
    ],
    selected: selectedKeys.has(key),
    active: activeKey === key,
    download_restricted: false,
  };
}

function opticalCatalogRow(
  product: SarProduct,
  key: string,
  selectedKeys: ReadonlySet<string>,
  activeKey: string | null,
  previewState: PreviewRuntimeState | undefined,
): CatalogResultRow {
  const assets = product.assets ?? [];
  const primarySize = assets.find((asset) => asset.role === "primary")?.size_bytes;
  const reportedSize = Number(product.file_size_remote ?? 0) > 0
    ? product.file_size_remote
    : primarySize;
  const cloud = sentinel2CloudCover(product);
  const online = sentinel2Online(product);
  const cloudCogOnDemand =
    isMicrosoftSentinel2SearchOnlyProduct(product) || isEarthSearchSentinel2Product(product);
  return {
    provider_product_key: key,
    display_name: product.file_name?.trim() || product.scene_id?.trim() || product.product_id,
    collection: product.dataset,
    dataset: product.dataset,
    provider: product.provider,
    platform: compactPlatform(product.platform),
    processing_level: "L2A",
    acquisition_time: displayValue(product.acquisition_start),
    footprint_available: Boolean(product.footprint_geojson),
    preview_status: previewState?.status ?? "not_requested",
    file_size_label: fileSizeLabel(reportedSize),
    asset_summary: cloudCogOnDemand
      ? t("shared.catalogPreview.resolveOnDemand")
      : online === true
        ? t("shared.catalogPreview.fullArchive")
        : online === false
          ? t("shared.catalogPreview.archiveOffline")
          : t("shared.catalogPreview.archiveStatusUnknown"),
    kind: "optical",
    domain_field_cells: [
      { label: "Cloud", value: cloud === null ? "—" : `${cloud}%` },
      { label: "Tile", value: sentinel2Tile(product) ?? "—" },
    ],
    selected: selectedKeys.has(key),
    active: activeKey === key,
    download_restricted: false,
  };
}

/**
 * Issue #3 R2 B5 (review M3): a schema-described optical product without a
 * dedicated row (e.g. HLS) shows its schema labels and cloud cover, and says
 * when it is search-only, instead of falling into the SAR row.
 */
function describedOpticalCatalogRow(
  product: SarProduct,
  key: string,
  selectedKeys: ReadonlySet<string>,
  activeKey: string | null,
  previewState: PreviewRuntimeState | undefined,
  capability: OpticalSearchCapability,
): CatalogResultRow {
  const metadata = product.native_metadata && typeof product.native_metadata === "object"
    ? product.native_metadata as Record<string, unknown>
    : {};
  const properties = metadata.properties && typeof metadata.properties === "object"
    ? metadata.properties as Record<string, unknown>
    : {};
  const cloud = typeof properties.cloud_cover === "number" && Number.isFinite(properties.cloud_cover)
    ? properties.cloud_cover
    : null;
  return {
    provider_product_key: key,
    display_name: product.scene_id?.trim() || product.product_id,
    collection: product.dataset,
    dataset: product.dataset,
    provider: product.provider,
    platform: compactPlatform(product.platform),
    processing_level: displayValue(product.processing_level || product.product_family),
    acquisition_time: displayValue(product.acquisition_start),
    footprint_available: Boolean(product.footprint_geojson || product.footprint_bbox),
    preview_status: previewState?.status ?? "not_requested",
    file_size_label: fileSizeLabel(product.file_size_remote),
    asset_summary: capability.searchOnly ? t("shared.catalogPreview.searchOnly") : "",
    kind: "optical",
    domain_field_cells: [
      { label: "Cloud", value: cloud === null ? "—" : `${cloud}%` },
      { label: "Instrument", value: displayValue(product.instrument) },
    ],
    selected: selectedKeys.has(key),
    active: activeKey === key,
    download_restricted: false,
  };
}

export function projectCatalogRows(
  products: readonly SarProduct[],
  selectedKeys: ReadonlySet<string>,
  activeKey: string | null,
  previewStates: ReadonlyMap<string, PreviewRuntimeState> = new Map(),
): CatalogResultRow[] {
  return products.map((product) => {
    const key = productIdentity(product);
    const assets = product.assets ?? [];
    const downloadableAssets = downloadableAssetCount(product, assets);
    const primarySize = assets.find((asset) => asset.role === "primary")?.size_bytes;
    const reportedSize = Number(product.file_size_remote ?? 0) > 0
      ? product.file_size_remote
      : primarySize;
    const previewState = previewStates.get(key);
    if (isLandsatC2L2CatalogProduct(product)) {
      return landsatCatalogRow(product, key, selectedKeys, activeKey, previewState);
    }
    if (isNisarCatalogProduct(product)) {
      return nisarCatalogRow(product, key, selectedKeys, activeKey, previewState);
    }
    if (isBhoonidhiSsarCatalogProduct(product)) {
      return bhoonidhiCatalogRow(product, key, selectedKeys, activeKey, previewState);
    }
    if (isAlos2CatalogProduct(product)) {
      return alos2CatalogRow(product, key, selectedKeys, activeKey, previewState);
    }
    if (isSentinel2L2aCatalogProduct(product) || isSentinel2L1cCatalogProduct(product)) {
      return opticalCatalogRow(product, key, selectedKeys, activeKey, previewState);
    }
    const describedOptical = opticalCapabilityForLogicalDataset(
      product.native_metadata && typeof product.native_metadata === "object"
        ? (product.native_metadata as Record<string, unknown>).logical_dataset
        : undefined,
    );
    // Only products the schema-described projection produced (its dataset
    // label) use this row; Sentinel-2/Landsat keep their dedicated rows and an
    // unreviewed source of theirs keeps the generic row as before.
    if (describedOptical && product.dataset === describedOptical.datasetLabel) {
      return describedOpticalCatalogRow(product, key, selectedKeys, activeKey, previewState, describedOptical);
    }
    return {
      provider_product_key: key,
      display_name: product.file_name?.trim() || product.scene_id?.trim() || product.product_id,
      collection: product.dataset,
      dataset: product.dataset,
      provider: product.provider,
      platform: compactPlatform(product.platform),
      processing_level: displayValue(product.processing_level || product.product_family),
      acquisition_time: displayValue(product.acquisition_start),
      footprint_available: Boolean(product.footprint_geojson || product.footprint_bbox),
      preview_status: previewState?.status ?? "not_requested",
      file_size_label: fileSizeLabel(reportedSize),
      asset_summary: product.dataset.toUpperCase() === "SEASAT"
        ? t("shared.catalogPreview.hdf5Primary")
        : downloadableAssets > 1
          ? t("shared.catalogPreview.downloadableFilesCount", { count: downloadableAssets })
          : "",
      kind: "sar",
      domain_field_cells: [
        { label: "Direction", value: displayValue(product.flight_direction).toUpperCase() },
        { label: "Polarization", value: polarizationDisplay(product) },
        { label: "Path", value: displayValue(product.path ?? product.relative_orbit) },
        { label: "Frame", value: displayValue(product.frame ?? product.asf_frame) },
      ],
      selected: selectedKeys.has(key),
      active: activeKey === key,
      download_restricted: isRestrictedSarDataset(product.dataset),
    };
  });
}

/** Compatibility export for older SAR-only call sites and focused tests. */
export const projectSarCatalogRows = projectCatalogRows;

function stateFor(
  status: PreviewRuntimeStatus,
  source: PreviewStateSource,
  message: string,
  url?: string | null,
): PreviewRuntimeState {
  if (source.kind === "unavailable") {
    return {
      status,
      product_key: null,
      descriptor: null,
      url: null,
      message,
    };
  }
  return {
    status,
    product_key: source.product_key,
    descriptor: source.kind === "resolved_preview" ? null : source.descriptor,
    url: url ?? (source.kind === "candidate" ? source.url : null),
    message,
  };
}

export class PreviewRequestCoordinator {
  private readonly publish: PreviewPublisher;
  private generation = 0;
  private active:
    | {
        generation: number;
        controller: AbortController;
        source: PreviewCandidateSource | ResolvedPreviewSource;
        cancelledByUser: boolean;
        cancelRemote?: () => void;
      }
    | null = null;

  constructor(publish: PreviewPublisher) {
    this.publish = publish;
  }

  async request(source: SarPreviewSource, loader: PreviewLoader): Promise<PreviewRuntimeState> {
    this.abortAsStale();
    const generation = ++this.generation;

    if (source.kind === "unavailable") {
      const unavailable = stateFor("unavailable", source, source.reason);
      this.publish(unavailable);
      return unavailable;
    }
    if (source.descriptor.credential_requirement === "provider_credentials") {
      const authentication = stateFor(
        "authentication_required",
        source,
        t("shared.catalogPreview.authRequired"),
      );
      this.publish(authentication);
      return authentication;
    }

    const active = {
      generation,
      controller: new AbortController(),
      source,
      cancelledByUser: false,
    };
    this.active = active;
    this.publish(stateFor("loading", source, t("shared.catalogPreview.loadingPreview")));
    try {
      await loader(source.url, active.controller.signal);
      if (generation !== this.generation || this.active !== active) {
        return stateFor("stale", source, t("shared.catalogPreview.staleIgnored"));
      }
      if (active.cancelledByUser) {
        this.active = null;
        return stateFor("cancelled", source, t("shared.catalogPreview.cancelled"));
      }
      const available = stateFor("available", source, source.descriptor.provenance);
      this.active = null;
      this.publish(available);
      return available;
    } catch (error) {
      if (generation !== this.generation || this.active !== active) {
        return stateFor("stale", source, t("shared.catalogPreview.staleIgnored"));
      }
      if (active.cancelledByUser || active.controller.signal.aborted) {
        const cancelled = stateFor("cancelled", source, t("shared.catalogPreview.cancelled"));
        this.active = null;
        if (active.cancelledByUser) this.publish(cancelled);
        return cancelled;
      }
      const message = error instanceof Error && error.message.trim()
        ? t("shared.catalogPreview.loadFailedWithDetail", { detail: error.message })
        : t("shared.catalogPreview.errorMessages.preview_request_failed");
      const failed = stateFor("failed", source, message);
      this.active = null;
      this.publish(failed);
      return failed;
    }
  }

  /**
   * Materialize a CDSE quicklook through the named desktop command. The
   * resulting data URL is session-only and never causes a WebView network
   * request to the frozen OData asset href.
   */
  async requestDesktopPreview(
    source: CdseQuicklookPreviewSource | LandsatBrowsePreviewSource,
    loader: (signal: AbortSignal) => Promise<{ dataUrl: string; mediaType: string }>,
    cancelRemote: () => void,
  ): Promise<PreviewRuntimeState> {
    this.abortAsStale();
    const generation = ++this.generation;
    const active = {
      generation,
      controller: new AbortController(),
      source,
      cancelledByUser: false,
      cancelRemote,
    };
    this.active = active;
    this.publish(stateFor(
      "loading",
      source,
      source.kind === "landsat_browse"
        ? t("shared.catalogPreview.loadingLandsatBrowse")
        : t("shared.catalogPreview.loadingS2Quicklook"),
    ));
    try {
      const quicklook = await loader(active.controller.signal);
      if (generation !== this.generation || this.active !== active) {
        return stateFor("stale", source, t("shared.catalogPreview.staleIgnored"));
      }
      if (active.cancelledByUser) {
        this.active = null;
        return stateFor("cancelled", source, t("shared.catalogPreview.cancelled"));
      }
      if (!isSafeCdseQuicklookDataUrl(quicklook.dataUrl, quicklook.mediaType)) {
        throw new Error(t("shared.catalogPreview.invalidDesktopPreviewData"));
      }
      const resolvedSource = {
        ...source,
        descriptor: { ...source.descriptor, media_type: quicklook.mediaType },
      };
      const available = stateFor(
        "available",
        resolvedSource,
        resolvedSource.descriptor.provenance,
        quicklook.dataUrl,
      );
      this.active = null;
      this.publish(available);
      return available;
    } catch (error) {
      if (generation !== this.generation || this.active !== active) {
        return stateFor("stale", source, t("shared.catalogPreview.staleIgnored"));
      }
      if (active.cancelledByUser || active.controller.signal.aborted) {
        const cancelled = stateFor("cancelled", source, t("shared.catalogPreview.cancelled"));
        this.active = null;
        if (active.cancelledByUser) this.publish(cancelled);
        return cancelled;
      }
      const message = error instanceof Error && error.message.trim()
        ? t("shared.catalogPreview.loadFailedWithDetail", { detail: error.message })
        : t("shared.catalogPreview.errorMessages.preview_request_failed");
      const failed = stateFor("failed", source, message);
      this.active = null;
      this.publish(failed);
      return failed;
    }
  }

  async requestResolvedPreview(
    productKey: string,
    loader: (signal: AbortSignal) => Promise<ResolvedPreviewAsset>,
    cancelRemote: () => void,
  ): Promise<PreviewRuntimeState> {
    this.abortAsStale();
    const generation = ++this.generation;
    const source: ResolvedPreviewSource = {
      kind: "resolved_preview",
      product_key: productKey,
    };
    const active = {
      generation,
      controller: new AbortController(),
      source,
      cancelledByUser: false,
      cancelRemote,
    };
    this.active = active;
    this.publish(stateFor("loading", source, t("shared.catalogPreview.loadingControlledPreview")));
    try {
      const preview = await loader(active.controller.signal);
      if (generation !== this.generation || this.active !== active) {
        return stateFor("stale", source, t("shared.catalogPreview.staleIgnored"));
      }
      if (active.cancelledByUser) {
        this.active = null;
        return stateFor("cancelled", source, t("shared.catalogPreview.cancelled"));
      }
      if (
        preview.product_key !== productKey ||
        !preview.descriptor.source_asset_keys.includes(preview.source_asset_key) ||
        preview.descriptor.media_type !== preview.media_type ||
        !isSafeCdseQuicklookDataUrl(preview.data_url, preview.media_type)
      ) {
        throw new PreviewResolutionError("preview_decode_failed");
      }
      const available: PreviewRuntimeState = {
        status: "available",
        product_key: preview.product_key,
        descriptor: preview.descriptor,
        url: preview.data_url,
        message: preview.descriptor.provenance,
      };
      this.active = null;
      this.publish(available);
      return available;
    } catch (error) {
      if (generation !== this.generation || this.active !== active) {
        return stateFor("stale", source, t("shared.catalogPreview.staleIgnored"));
      }
      if (active.cancelledByUser || active.controller.signal.aborted) {
        const cancelled = stateFor("cancelled", source, t("shared.catalogPreview.cancelled"));
        this.active = null;
        if (active.cancelledByUser) this.publish(cancelled);
        return cancelled;
      }
      const message = error instanceof Error && error.message.trim()
        ? error.message
        : PREVIEW_ERROR_MESSAGES.preview_decode_failed;
      const failed = stateFor(error instanceof PreviewResolutionError && error.kind === "provider_no_preview"
        ? "unavailable" : "failed", source, message);
      failed.error_kind = error instanceof PreviewResolutionError ? error.kind : "preview_request_failed";
      this.active = null;
      this.publish(failed);
      return failed;
    }
  }

  async requestCdseQuicklook(
    source: CdseQuicklookPreviewSource,
    loader: (signal: AbortSignal) => Promise<{ dataUrl: string; mediaType: string }>,
    cancelRemote: () => void,
  ): Promise<PreviewRuntimeState> {
    return this.requestDesktopPreview(source, loader, cancelRemote);
  }

  cancel(): PreviewRuntimeState | null {
    const active = this.active;
    if (!active) return null;
    if (active.cancelledByUser) return null;
    active.cancelledByUser = true;
    active.controller.abort();
    const cancelRemote = active.cancelRemote;
    active.cancelRemote = undefined;
    try {
      cancelRemote?.();
    } catch {
      // A remote cancellation failure must not resurrect a locally cancelled preview.
    }
    const cancelled = stateFor("cancelled", active.source, t("shared.catalogPreview.cancelled"));
    this.publish(cancelled);
    return cancelled;
  }

  dispose() {
    this.abortAsStale();
    this.generation += 1;
  }

  private abortAsStale() {
    if (!this.active) return;
    this.active.controller.abort();
    const cancelRemote = this.active.cancelRemote;
    this.active.cancelRemote = undefined;
    try {
      cancelRemote?.();
    } catch {
      // Stale work stays stale even when the best-effort remote cancel fails.
    }
    this.active = null;
  }
}

function isSafeCdseQuicklookDataUrl(value: string, mediaType: string): boolean {
  if (!matchesImageMediaType(mediaType)) return false;
  const prefix = `data:${mediaType};base64,`;
  const encoded = value.slice(prefix.length);
  return value.startsWith(prefix)
    && encoded.length > 0
    && encoded.length <= Math.ceil((16 * 1024 * 1024) / 3) * 4
    && encoded.length % 4 === 0
    && /^[A-Za-z0-9+/]*={0,2}$/.test(encoded);
}

function matchesImageMediaType(value: string): value is "image/jpeg" | "image/png" | "image/webp" {
  return value === "image/jpeg" || value === "image/png" || value === "image/webp";
}

export function loadPreviewImage(url: string, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    const cleanup = () => {
      signal.removeEventListener("abort", abort);
      image.onload = null;
      image.onerror = null;
    };
    const abort = () => {
      cleanup();
      image.src = "";
      reject(new DOMException("Preview request aborted", "AbortError"));
    };
    if (signal.aborted) {
      abort();
      return;
    }
    signal.addEventListener("abort", abort, { once: true });
    image.onload = () => {
      cleanup();
      resolve();
    };
    image.onerror = () => {
      cleanup();
      reject(new Error(t("shared.catalogPreview.decodeOrRemoteRejected")));
    };
    image.decoding = "async";
    image.src = url;
  });
}
