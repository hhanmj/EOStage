import { t } from "../i18n/index.ts";
import type { Bbox, SceneRow } from "./bridge.ts";
import {
  isTemporarySystemDownloadPath,
  TEMPORARY_DOWNLOAD_PATH_ERROR_ZH,
} from "./storagePathValidation.ts";
import type { DownloadLayout } from "./generated/v3-contracts";
import type { DesktopLandsatAssetDetail } from "./desktopShellBridge.ts";
import {
  landsatAssetKeysForPreset,
  landsatHasSurfaceTemperature,
  type LandsatContentSelection,
} from "./landsatAssetSelection.ts";
import {
  hlsBandKeysForPreset,
  normalizeHlsBandKeys,
  normalizeSentinel2BandKeys,
  normalizeSentinel2L1cBandKeys,
  sentinel2BandKeysForPreset,
  sentinel2L1cBandKeysForPreset,
  validSentinel2BandKeys,
  validSentinel2L1cBandKeys,
  type HlsBandPreset,
  type Sentinel2BandPreset,
  type Sentinel2DeliveryForm,
} from "./sentinel2BandSelection.ts";

export type DownloadSelectionScope = "checked" | "exact_item" | "all_matching";
export type DownloadAllMatchPreview = Readonly<{
  batch_id: string;
  revision: number;
  batch_total_count: number;
  matched_count: number;
  excluded_count: number;
  skipped_active_count: number;
  rejected_count: number;
  admissible_count: number;
  known_bytes: number;
  unknown_size_count: number;
  selection_digest: string;
}>;
export type LandsatDeliveryForm = "official_archive" | "complete_asset_set";
export type DownloadAcquisitionKind =
  | "sentinel1"
  | "historical_asf"
  | "burst"
  | "generic_sar"
  | "alos2"
  | "nisar_complete"
  | "nisar_subset"
  | "sentinel2"
  | "hls"
  | "landsat"
  | "orbit";

export type DownloadAcquisition = Readonly<{
  kind: DownloadAcquisitionKind;
  frozenParameters: Readonly<Record<string, unknown>>;
}>;

export type DownloadAcquisitionDraft = Readonly<{
  sourceBatchId: string;
  sourceBatchRevision: number;
  selectionScope: DownloadSelectionScope;
  selectedProductIds: readonly string[];
  selectedProductCount: number;
  excludedProductIds?: readonly string[];
  allMatchPreview?: DownloadAllMatchPreview | null;
  acquisition: DownloadAcquisition;
  destinationPrefill: string;
  layout: DownloadLayout;
  taskPolicy: string;
  concurrency: number;
}>;

export type ConfirmedDownloadTaskDraft = Readonly<{
  sourceBatchId: string;
  sourceBatchRevision: number;
  selectionScope: DownloadSelectionScope;
  selectedProductIds: readonly string[];
  selectedProductCount: number;
  excludedProductIds?: readonly string[];
  allMatchPreview?: DownloadAllMatchPreview | null;
  acquisition: DownloadAcquisition;
  outputRoot: string;
  layout: DownloadLayout;
  taskPolicy: string;
  concurrency: number;
  includePreciseOrbit?: boolean;
  includeGacos?: boolean;
}>;

export type DownloadAcquisitionDraftInput = {
  sourceBatchId: string;
  sourceBatchRevision: number;
  selectionScope: DownloadSelectionScope;
  selectedProductIds: readonly string[];
  selectedProductCount?: number;
  excludedProductIds?: readonly string[];
  allMatchPreview?: DownloadAllMatchPreview | null;
  acquisition: {
    kind: DownloadAcquisitionKind;
    frozenParameters: Readonly<Record<string, unknown>>;
  };
  destinationPrefill: string;
  layout: DownloadLayout;
  taskPolicy: string;
  concurrency: number;
};

type ActiveConfirmationState = Readonly<{
  draft: DownloadAcquisitionDraft;
  outputRoot: string;
  error: string | null;
  confirmedDraft: ConfirmedDownloadTaskDraft | null;
}>;

export type DownloadDestinationConfirmationState =
  | Readonly<{ status: "idle" }>
  | (ActiveConfirmationState & Readonly<{ status: "reviewing" | "choosing_directory" }>)
  | (ActiveConfirmationState & Readonly<{
      status: "submitting";
      confirmedDraft: ConfirmedDownloadTaskDraft;
    }>);

export type DownloadDestinationConfirmationEvent =
  | Readonly<{ type: "cancel" }>
  | Readonly<{ type: "begin_directory_selection" }>
  | Readonly<{ type: "directory_selection_cancelled" }>
  | Readonly<{ type: "directory_selected"; outputRoot: string }>
  | Readonly<{ type: "output_root_changed"; outputRoot: string }>
  | Readonly<{ type: "companion_orbit_changed"; enabled: boolean }>
  | Readonly<{ type: "companion_gacos_changed"; enabled: boolean }>
  | Readonly<{ type: "landsat_delivery_changed"; deliveryForm: LandsatDeliveryForm }>
  | Readonly<{ type: "landsat_content_changed"; selection: LandsatContentSelection }>
  | Readonly<{ type: "landsat_asset_toggled"; assetKey: string }>
  | Readonly<{ type: "sentinel2_delivery_changed"; deliveryForm: Sentinel2DeliveryForm }>
  | Readonly<{ type: "sentinel2_band_preset_changed"; preset: Sentinel2BandPreset }>
  | Readonly<{ type: "sentinel2_band_toggled"; bandKey: string }>
  | Readonly<{ type: "hls_band_preset_changed"; preset: HlsBandPreset }>
  | Readonly<{ type: "hls_band_toggled"; bandKey: string }>
  | Readonly<{ type: "confirm" }>
  | Readonly<{ type: "submission_failed"; message: string }>
  | Readonly<{ type: "submission_succeeded" }>;

export const initialDownloadDestinationConfirmationState: DownloadDestinationConfirmationState =
  Object.freeze({ status: "idle" as const });

function cloneAndFreeze<T>(value: T): T {
  if (Array.isArray(value)) {
    return Object.freeze(value.map((item) => cloneAndFreeze(item))) as T;
  }
  if (value !== null && typeof value === "object") {
    const clone = Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, item]) => [
        key,
        cloneAndFreeze(item),
      ]),
    );
    return Object.freeze(clone) as T;
  }
  return value;
}

function requiredText(value: string, label: string): string {
  const normalized = value.trim();
  if (!normalized) throw new Error(`${label} is required`);
  return normalized;
}

export function createDownloadAcquisitionDraft(
  input: DownloadAcquisitionDraftInput,
): DownloadAcquisitionDraft {
  const sourceBatchId = requiredText(input.sourceBatchId, "source batch ID");
  if (!Number.isSafeInteger(input.sourceBatchRevision) || input.sourceBatchRevision < 0) {
    throw new Error("source batch revision must be a non-negative safe integer");
  }
  const selectedProductIds = Array.from(new Set(
    input.selectedProductIds.map((item) => item.trim()).filter(Boolean),
  ));
  const selectedProductCount = input.selectionScope === "all_matching"
    ? (input.allMatchPreview?.admissible_count ?? input.selectedProductCount ?? 0)
    : (input.selectedProductCount ?? selectedProductIds.length);
  if (!Number.isSafeInteger(selectedProductCount) || selectedProductCount < 1) {
    throw new Error("at least one selected product is required");
  }
  if (selectedProductIds.length === 0 && input.selectionScope === "exact_item") {
    throw new Error("an exact-item selection requires its selected product ID");
  }
  if (selectedProductIds.length > selectedProductCount) {
    throw new Error("selected product IDs cannot exceed the authoritative selection count");
  }
  if (!Number.isSafeInteger(input.concurrency) || input.concurrency < 1) {
    throw new Error("task concurrency must be a positive safe integer");
  }
  if (input.acquisition.kind === "nisar_subset") {
    const parameters = input.acquisition.frozenParameters;
    if (!parameters.subset_aoi) throw new Error("NISAR subset AOI must be frozen before destination confirmation");
    for (const field of ["frequency", "transfer_mode"] as const) {
      if (typeof parameters[field] !== "string" || !parameters[field].trim()) {
        throw new Error(`NISAR ${field} must be frozen before destination confirmation`);
      }
    }
    const polarizations = parameters.polarizations;
    if (!Array.isArray(polarizations) || polarizations.length === 0
      || !polarizations.every((value) => typeof value === "string" && value.trim())) {
      throw new Error("NISAR polarizations must be frozen before destination confirmation");
    }
  }
  if (input.acquisition.kind === "landsat") {
    const parameters = input.acquisition.frozenParameters;
    if (!['official_archive', 'complete_asset_set'].includes(String(parameters.delivery_form))) {
      throw new Error("Landsat delivery form must be frozen before destination confirmation");
    }
    if (
      typeof parameters.official_archive_available !== "boolean"
      || typeof parameters.complete_asset_set_available !== "boolean"
    ) {
      throw new Error("Landsat delivery availability must be frozen before destination confirmation");
    }
  }
  if (input.acquisition.kind === "sentinel2") {
    const parameters = input.acquisition.frozenParameters;
    const isL1c = parameters.product_level === "l1c";
    if (!["safe_archive", "analysis_bands"].includes(String(parameters.delivery_form))) {
      throw new Error("Sentinel-2 delivery form must be frozen before destination confirmation");
    }
    if (
      typeof parameters.safe_archive_available !== "boolean"
      || typeof parameters.analysis_bands_available !== "boolean"
    ) {
      throw new Error("Sentinel-2 delivery availability must be frozen before destination confirmation");
    }
    if (
      !["true_color", "vegetation", "all_10m", "all_reflectance", "custom"].includes(
        String(parameters.band_preset),
      )
    ) {
      throw new Error("Sentinel-2 band preset must be frozen before destination confirmation");
    }
    const rawBandKeys = Array.isArray(parameters.selected_band_keys)
      ? parameters.selected_band_keys.filter((value): value is string => typeof value === "string")
      : [];
    const selectedBandKeys = isL1c
      ? normalizeSentinel2L1cBandKeys(rawBandKeys)
      : normalizeSentinel2BandKeys(rawBandKeys);
    const validBands = isL1c
      ? validSentinel2L1cBandKeys(selectedBandKeys)
      : validSentinel2BandKeys(selectedBandKeys);
    if (parameters.delivery_form === "analysis_bands" && !validBands) {
      throw new Error("Sentinel-2 analysis-band delivery requires at least one reviewed band");
    }
  }
  if (input.acquisition.kind === "hls") {
    const parameters = input.acquisition.frozenParameters;
    const product = parameters.hls_product;
    if (product !== "L30" && product !== "S30") {
      throw new Error("HLS product level (L30/S30) must be frozen before destination confirmation");
    }
    if (
      !["true_color", "false_color", "swir", "all_bands", "custom"].includes(
        String(parameters.band_preset),
      )
    ) {
      throw new Error("HLS band preset must be frozen before destination confirmation");
    }
    const rawBandKeys = Array.isArray(parameters.selected_band_keys)
      ? parameters.selected_band_keys.filter((value): value is string => typeof value === "string")
      : [];
    if (normalizeHlsBandKeys(product, rawBandKeys).length === 0) {
      throw new Error("HLS delivery requires at least one selected band");
    }
  }

  return cloneAndFreeze({
    sourceBatchId,
    sourceBatchRevision: input.sourceBatchRevision,
    selectionScope: input.selectionScope,
    selectedProductIds,
    selectedProductCount,
    ...(input.excludedProductIds ? { excludedProductIds: Array.from(new Set(input.excludedProductIds.map((item) => item.trim()).filter(Boolean))) } : {}),
    ...(input.allMatchPreview ? { allMatchPreview: input.allMatchPreview } : {}),
    acquisition: {
      kind: input.acquisition.kind,
      frozenParameters: input.acquisition.frozenParameters,
    },
    destinationPrefill: input.destinationPrefill.trim(),
    layout: input.layout,
    taskPolicy: requiredText(input.taskPolicy, "task policy"),
    concurrency: input.concurrency,
  });
}

export function beginDownloadDestinationConfirmation(
  draft: DownloadAcquisitionDraft,
): DownloadDestinationConfirmationState {
  return Object.freeze({
    status: "reviewing" as const,
    draft,
    outputRoot: draft.destinationPrefill,
    error: null,
    confirmedDraft: null,
  });
}

function confirmedDraft(
  draft: DownloadAcquisitionDraft,
  outputRoot: string,
): ConfirmedDownloadTaskDraft {
  const downloadOrbit = typeof draft.acquisition.frozenParameters?.download_orbit === "boolean"
    ? draft.acquisition.frozenParameters.download_orbit
    : (typeof draft.acquisition.frozenParameters?.include_precise_orbit === "boolean"
      ? draft.acquisition.frozenParameters.include_precise_orbit
      : undefined);
  const requestGacos = typeof draft.acquisition.frozenParameters?.request_gacos === "boolean"
    ? draft.acquisition.frozenParameters.request_gacos
    : undefined;
  return cloneAndFreeze({
    sourceBatchId: draft.sourceBatchId,
    sourceBatchRevision: draft.sourceBatchRevision,
    selectionScope: draft.selectionScope,
    selectedProductIds: draft.selectedProductIds,
    selectedProductCount: draft.selectedProductCount,
    ...(draft.excludedProductIds ? { excludedProductIds: draft.excludedProductIds } : {}),
    ...(draft.allMatchPreview ? { allMatchPreview: draft.allMatchPreview } : {}),
    acquisition: draft.acquisition,
    outputRoot,
    layout: draft.layout,
    taskPolicy: draft.taskPolicy,
    concurrency: draft.concurrency,
    ...(downloadOrbit !== undefined ? { includePreciseOrbit: downloadOrbit } : {}),
    ...(requestGacos !== undefined ? { includeGacos: requestGacos } : {}),
  });
}

function landsatAvailableAssets(parameters: Readonly<Record<string, unknown>>): DesktopLandsatAssetDetail[] {
  const value = parameters.available_assets;
  if (!Array.isArray(value)) return [];
  return value.filter((asset): asset is DesktopLandsatAssetDetail => {
    if (!asset || typeof asset !== "object" || Array.isArray(asset)) return false;
    const record = asset as Record<string, unknown>;
    return typeof record.asset_key === "string"
      && typeof record.file_name === "string"
      && ["primary", "auxiliary", "metadata"].includes(String(record.role))
      && typeof record.size_bytes === "number"
      && Number.isSafeInteger(record.size_bytes)
      && record.size_bytes > 0;
  });
}

function landsatSelectedAssetKeys(parameters: Readonly<Record<string, unknown>>): string[] {
  return Array.isArray(parameters.selected_asset_keys)
    ? parameters.selected_asset_keys.filter((value): value is string => typeof value === "string" && Boolean(value.trim()))
    : [];
}

function landsatContentSelection(parameters: Readonly<Record<string, unknown>>): LandsatContentSelection {
  const selection = parameters.content_selection;
  return selection === "surface_reflectance"
    || selection === "surface_temperature"
    || selection === "custom"
    ? selection
    : "all";
}

function sentinel2DeliveryForm(parameters: Readonly<Record<string, unknown>>): Sentinel2DeliveryForm {
  return parameters.delivery_form === "analysis_bands" ? "analysis_bands" : "safe_archive";
}

function sentinel2BandPreset(parameters: Readonly<Record<string, unknown>>): Sentinel2BandPreset {
  const preset = parameters.band_preset;
  return preset === "vegetation"
    || preset === "all_10m"
    || preset === "all_reflectance"
    || preset === "custom"
    ? preset
    : "true_color";
}

function sentinel2IsL1c(parameters: Readonly<Record<string, unknown>>): boolean {
  return parameters.product_level === "l1c";
}

function sentinel2PresetKeysForParams(
  parameters: Readonly<Record<string, unknown>>,
  preset: Sentinel2BandPreset,
): string[] {
  return sentinel2IsL1c(parameters)
    ? sentinel2L1cBandKeysForPreset(preset)
    : sentinel2BandKeysForPreset(preset);
}

function sentinel2SelectedBandKeys(parameters: Readonly<Record<string, unknown>>): string[] {
  const raw = Array.isArray(parameters.selected_band_keys)
    ? parameters.selected_band_keys.filter((value): value is string => typeof value === "string")
    : [];
  return sentinel2IsL1c(parameters)
    ? normalizeSentinel2L1cBandKeys(raw)
    : normalizeSentinel2BandKeys(raw);
}

function sentinel2ValidBandsForParams(
  parameters: Readonly<Record<string, unknown>>,
  keys: readonly string[],
): boolean {
  return sentinel2IsL1c(parameters)
    ? validSentinel2L1cBandKeys(keys)
    : validSentinel2BandKeys(keys);
}

function hlsProduct(parameters: Readonly<Record<string, unknown>>): "L30" | "S30" {
  return parameters.hls_product === "L30" ? "L30" : "S30";
}

function hlsBandPreset(parameters: Readonly<Record<string, unknown>>): HlsBandPreset {
  const preset = parameters.band_preset;
  return preset === "false_color"
    || preset === "swir"
    || preset === "all_bands"
    || preset === "custom"
    ? preset
    : "true_color";
}

function hlsSelectedBandKeys(parameters: Readonly<Record<string, unknown>>): string[] {
  const raw = Array.isArray(parameters.selected_band_keys)
    ? parameters.selected_band_keys.filter((value): value is string => typeof value === "string")
    : [];
  return normalizeHlsBandKeys(hlsProduct(parameters), raw);
}

export function downloadDestinationConfirmationReducer(
  state: DownloadDestinationConfirmationState,
  event: DownloadDestinationConfirmationEvent,
): DownloadDestinationConfirmationState {
  if (event.type === "cancel" || event.type === "submission_succeeded") {
    return initialDownloadDestinationConfirmationState;
  }
  if (state.status === "idle") return state;

  if (event.type === "begin_directory_selection" && state.status !== "submitting") {
    return Object.freeze({ ...state, status: "choosing_directory" as const, error: null });
  }
  if (event.type === "directory_selection_cancelled" && state.status === "choosing_directory") {
    return Object.freeze({ ...state, status: "reviewing" as const });
  }
  if (event.type === "directory_selected" && state.status !== "submitting") {
    const outputRoot = event.outputRoot.trim();
    return Object.freeze({
      ...state,
      status: "reviewing" as const,
      outputRoot: outputRoot || state.outputRoot,
      error: outputRoot ? null : t("shared.downloadConfirm.selectOutputRoot"),
      confirmedDraft: null,
    });
  }
  if (event.type === "output_root_changed" && state.status !== "submitting") {
    const nextRoot = event.outputRoot;
    return Object.freeze({
      ...state,
      status: "reviewing" as const,
      outputRoot: nextRoot,
      error:
        !nextRoot.trim()
          ? state.error
          : isTemporarySystemDownloadPath(nextRoot)
            ? TEMPORARY_DOWNLOAD_PATH_ERROR_ZH
            : null,
      confirmedDraft: null,
    });
  }
  if (event.type === "companion_orbit_changed" && state.status === "reviewing"
    && state.draft.acquisition.frozenParameters.allow_companion_orbit === true) {
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: { ...state.draft.acquisition.frozenParameters, download_orbit: event.enabled },
        },
      },
      confirmedDraft: null,
    });
  }
  if (event.type === "companion_gacos_changed" && state.status === "reviewing"
    && state.draft.acquisition.frozenParameters.allow_companion_gacos === true) {
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: { ...state.draft.acquisition.frozenParameters, request_gacos: event.enabled },
        },
      },
      confirmedDraft: null,
    });
  }
  if (event.type === "sentinel2_delivery_changed" && state.status === "reviewing"
    && state.draft.acquisition.kind === "sentinel2") {
    const parameters = state.draft.acquisition.frozenParameters;
    const available = event.deliveryForm === "safe_archive"
      ? parameters.safe_archive_available === true
      : parameters.analysis_bands_available === true;
    if (!available) return state;
    let selectedBandKeys = sentinel2SelectedBandKeys(parameters);
    if (event.deliveryForm === "analysis_bands" && selectedBandKeys.length === 0) {
      selectedBandKeys = sentinel2PresetKeysForParams(parameters, sentinel2BandPreset(parameters));
    }
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: {
            ...parameters,
            delivery_form: event.deliveryForm,
            selected_band_keys: selectedBandKeys,
          },
        },
      },
      confirmedDraft: null,
      error: null,
    });
  }
  if (event.type === "sentinel2_band_preset_changed" && state.status === "reviewing"
    && state.draft.acquisition.kind === "sentinel2") {
    const parameters = state.draft.acquisition.frozenParameters;
    if (sentinel2DeliveryForm(parameters) !== "analysis_bands") return state;
    const current = sentinel2SelectedBandKeys(parameters);
    const selectedBandKeys = event.preset === "custom"
      ? (current.length > 0 ? current : sentinel2PresetKeysForParams(parameters, "true_color"))
      : sentinel2PresetKeysForParams(parameters, event.preset);
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: {
            ...parameters,
            band_preset: event.preset,
            selected_band_keys: selectedBandKeys,
          },
        },
      },
      confirmedDraft: null,
      error: null,
    });
  }
  if (event.type === "sentinel2_band_toggled" && state.status === "reviewing"
    && state.draft.acquisition.kind === "sentinel2") {
    const parameters = state.draft.acquisition.frozenParameters;
    if (sentinel2DeliveryForm(parameters) !== "analysis_bands"
      || sentinel2BandPreset(parameters) !== "custom") return state;
    const current = new Set(sentinel2SelectedBandKeys(parameters));
    if (current.has(event.bandKey)) current.delete(event.bandKey);
    else current.add(event.bandKey);
    const selectedBandKeys = sentinel2IsL1c(parameters)
      ? normalizeSentinel2L1cBandKeys([...current])
      : normalizeSentinel2BandKeys([...current]);
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: { ...parameters, selected_band_keys: selectedBandKeys },
        },
      },
      confirmedDraft: null,
      error: null,
    });
  }
  if (event.type === "hls_band_preset_changed" && state.status === "reviewing"
    && state.draft.acquisition.kind === "hls") {
    const parameters = state.draft.acquisition.frozenParameters;
    const product = hlsProduct(parameters);
    const current = hlsSelectedBandKeys(parameters);
    const selectedBandKeys = event.preset === "custom"
      ? (current.length > 0 ? current : hlsBandKeysForPreset(product, "true_color"))
      : hlsBandKeysForPreset(product, event.preset);
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: {
            ...parameters,
            band_preset: event.preset,
            selected_band_keys: selectedBandKeys,
          },
        },
      },
      confirmedDraft: null,
      error: null,
    });
  }
  if (event.type === "hls_band_toggled" && state.status === "reviewing"
    && state.draft.acquisition.kind === "hls") {
    const parameters = state.draft.acquisition.frozenParameters;
    if (hlsBandPreset(parameters) !== "custom") return state;
    const product = hlsProduct(parameters);
    const current = new Set(hlsSelectedBandKeys(parameters));
    if (current.has(event.bandKey)) current.delete(event.bandKey);
    else current.add(event.bandKey);
    const selectedBandKeys = normalizeHlsBandKeys(product, [...current]);
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: { ...parameters, selected_band_keys: selectedBandKeys },
        },
      },
      confirmedDraft: null,
      error: null,
    });
  }
  if (event.type === "landsat_delivery_changed" && state.status === "reviewing"
    && state.draft.acquisition.kind === "landsat") {
    const parameters = state.draft.acquisition.frozenParameters;
    if (event.deliveryForm === "official_archive" && landsatContentSelection(parameters) !== "all") {
      return state;
    }
    const available = event.deliveryForm === "official_archive"
      ? parameters.official_archive_available === true
      : parameters.complete_asset_set_available === true;
    if (!available) return state;
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: { ...parameters, delivery_form: event.deliveryForm },
        },
      },
      confirmedDraft: null,
      error: null,
    });
  }
  if (event.type === "landsat_content_changed" && state.status === "reviewing"
    && state.draft.acquisition.kind === "landsat") {
    const parameters = state.draft.acquisition.frozenParameters;
    const assets = landsatAvailableAssets(parameters);
    if (event.selection !== "all" && assets.length === 0) return state;
    if (event.selection === "surface_temperature" && !landsatHasSurfaceTemperature(assets)) return state;
    if (event.selection !== "all" && parameters.complete_asset_set_available !== true) return state;
    const currentKeys = landsatSelectedAssetKeys(parameters);
    const selectedAssetKeys = event.selection === "custom"
      ? (currentKeys.length > 0 ? currentKeys : landsatAssetKeysForPreset(assets, "all"))
      : landsatAssetKeysForPreset(assets, event.selection);
    const deliveryForm = event.selection === "all"
      ? parameters.delivery_form
      : "complete_asset_set";
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: {
            ...parameters,
            content_selection: event.selection,
            selected_asset_keys: selectedAssetKeys,
            delivery_form: deliveryForm,
          },
        },
      },
      confirmedDraft: null,
      error: null,
    });
  }
  if (event.type === "landsat_asset_toggled" && state.status === "reviewing"
    && state.draft.acquisition.kind === "landsat") {
    const parameters = state.draft.acquisition.frozenParameters;
    if (landsatContentSelection(parameters) !== "custom") return state;
    const assets = landsatAvailableAssets(parameters);
    if (!assets.some((asset) => asset.asset_key === event.assetKey)) return state;
    const current = new Set(landsatSelectedAssetKeys(parameters));
    if (current.has(event.assetKey)) current.delete(event.assetKey);
    else current.add(event.assetKey);
    const selectedAssetKeys = assets
      .map((asset) => asset.asset_key)
      .filter((key) => current.has(key));
    return cloneAndFreeze({
      ...state,
      draft: {
        ...state.draft,
        acquisition: {
          ...state.draft.acquisition,
          frozenParameters: { ...parameters, selected_asset_keys: selectedAssetKeys },
        },
      },
      confirmedDraft: null,
      error: null,
    });
  }
  if (event.type === "confirm" && state.status === "reviewing") {
    if (state.draft.acquisition.kind === "sentinel2") {
      const parameters = state.draft.acquisition.frozenParameters;
      const delivery = sentinel2DeliveryForm(parameters);
      const available = delivery === "safe_archive"
        ? parameters.safe_archive_available === true
        : parameters.analysis_bands_available === true;
      if (!available) {
        return Object.freeze({ ...state, error: t("shared.downloadConfirm.s2DeliveryUnavailable") });
      }
      if (delivery === "analysis_bands" && !sentinel2ValidBandsForParams(parameters, sentinel2SelectedBandKeys(parameters))) {
        return Object.freeze({ ...state, error: t("shared.downloadConfirm.selectAtLeastOneS2Band") });
      }
    }
    if (state.draft.acquisition.kind === "hls") {
      const parameters = state.draft.acquisition.frozenParameters;
      if (hlsSelectedBandKeys(parameters).length === 0) {
        return Object.freeze({ ...state, error: t("shared.downloadConfirm.selectAtLeastOneS2Band") });
      }
    }
    if (state.draft.acquisition.kind === "landsat") {
      const parameters = state.draft.acquisition.frozenParameters;
      const selection = landsatContentSelection(parameters);
      const selectedAssetKeys = landsatSelectedAssetKeys(parameters);
      const assets = landsatAvailableAssets(parameters);
      if (selection !== "all" && parameters.delivery_form !== "complete_asset_set") {
        return Object.freeze({ ...state, error: t("shared.downloadConfirm.landsatExpandedOnly") });
      }
      if (parameters.delivery_form === "complete_asset_set" && assets.length > 0 && selectedAssetKeys.length === 0) {
        return Object.freeze({ ...state, error: t("shared.downloadConfirm.selectAtLeastOneLandsatFile") });
      }
    }
    const outputRoot = state.outputRoot.trim();
    if (!outputRoot) {
      return Object.freeze({ ...state, error: t("shared.downloadConfirm.selectOutputRoot") });
    }
    if (isTemporarySystemDownloadPath(outputRoot)) {
      return Object.freeze({ ...state, error: TEMPORARY_DOWNLOAD_PATH_ERROR_ZH });
    }
    return Object.freeze({
      ...state,
      status: "submitting" as const,
      error: null,
      confirmedDraft: confirmedDraft(state.draft, outputRoot),
    });
  }
  if (event.type === "submission_failed" && state.status === "submitting") {
    return Object.freeze({
      status: "reviewing" as const,
      draft: state.draft,
      outputRoot: state.outputRoot,
      error: event.message.trim() || t("shared.downloadConfirm.createTaskFailed"),
      confirmedDraft: null,
    });
  }
  return state;
}

export function newTaskDestinationDecision(input: {
  intent: "append" | "create_successor" | "create_new";
  taskStatus: string | null;
}): Readonly<{ kind: "reuse_existing_task" | "confirm_new_task" }> {
  if (input.intent === "append" && ["queued", "running", "paused"].includes(input.taskStatus ?? "")) {
    return Object.freeze({ kind: "reuse_existing_task" as const });
  }
  return Object.freeze({ kind: "confirm_new_task" as const });
}

export type UnifiedDatasetFamily = "nisar" | "sentinel2" | "landsat" | "sar" | "other";

export function inferUnifiedDatasetFamily(
  draft: DownloadAcquisitionDraft,
  activeResourceId?: string | null,
): UnifiedDatasetFamily {
  const rid = (activeResourceId ?? "").toLowerCase();
  const frozenRid = typeof draft.acquisition.frozenParameters?.resource_id === "string"
    ? draft.acquisition.frozenParameters.resource_id.toLowerCase()
    : "";
  const effectiveRid = rid || frozenRid;
  if (draft.acquisition.kind === "nisar_subset" || draft.acquisition.kind === "nisar_complete" || effectiveRid.includes("nisar")) {
    return "nisar";
  }
  if (draft.acquisition.kind === "sentinel2" || effectiveRid.includes("sentinel-2")) {
    return "sentinel2";
  }
  if (draft.acquisition.kind === "landsat" || effectiveRid.includes("landsat")) {
    return "landsat";
  }
  if (draft.acquisition.frozenParameters?.dedicated_geodata === true) {
    return "other";
  }
  if (["sentinel1", "burst", "generic_sar", "historical_asf", "alos2"].includes(draft.acquisition.kind)) {
    if (
      draft.acquisition.kind === "generic_sar"
      && effectiveRid
      && (
        effectiveRid.startsWith("dem.")
        || effectiveRid.startsWith("auxiliary.")
        || effectiveRid.startsWith("atmospheric.")
        || effectiveRid.startsWith("orbit.")
        || effectiveRid.startsWith("open.")
        || effectiveRid.startsWith("event.")
        || effectiveRid.startsWith("geodesy.")
        || effectiveRid.includes("black-marble")
      )
    ) {
      return "other";
    }
    return "sar";
  }
  return "other";
}

/**
 * Only Sentinel-1 imagery acquisitions (SLC / Burst / GRD) expose companion
 * auxiliary download options (POEORB orbit, GACOS ZTD, 30m reference DEM).
 * Open geodata (e.g. Black Marble), optical sensors, DEMs, standalone orbits,
 * and derived SAR/InSAR products must not show Sentinel-1 companion options.
 */
export function isSentinel1CompanionEligible(
  draft: DownloadAcquisitionDraft,
  activeResourceId?: string | null,
): boolean {
  const params = draft.acquisition.frozenParameters;
  if (params?.dedicated_geodata === true) {
    return false;
  }
  if (draft.acquisition.kind === "sentinel1" || draft.acquisition.kind === "burst") {
    return true;
  }
  if (draft.acquisition.kind === "generic_sar") {
    const rid = (activeResourceId ?? "").toLowerCase().trim();
    const frozenRid = typeof params?.resource_id === "string"
      ? params.resource_id.toLowerCase().trim()
      : "";
    const effectiveRid = rid || frozenRid;
    if (!effectiveRid || effectiveRid.startsWith("orbit.")) {
      return false;
    }
    return effectiveRid.includes("sentinel-1") || effectiveRid.includes("sentinel1");
  }
  return false;
}

export interface NisarRangePreflightEstimate {
  sceneCount: number;
  rawBytes: number;
  estimatedSubsetBytes: number;
  savedBytes: number;
  savingsPercent: number;
  isFallbackEstimate: boolean;
}

const DEFAULT_NISAR_SCENE_BYTES = 15 * 1024 * 1024 * 1024; // ~15 GiB typical RSLC HDF5

function bboxAreaRatio(aoi: Bbox | null | undefined, footprint: Bbox | null | undefined): number {
  if (!aoi || !footprint) return 0.18;
  const fW = Math.max(1e-6, Math.abs(footprint.east - footprint.west));
  const fH = Math.max(1e-6, Math.abs(footprint.north - footprint.south));
  const iW = Math.max(0, Math.min(aoi.east, footprint.east) - Math.max(aoi.west, footprint.west));
  const iH = Math.max(0, Math.min(aoi.north, footprint.north) - Math.max(aoi.south, footprint.south));
  if (iW <= 0 || iH <= 0) return 0.12;
  return Math.min(1, Math.max(0.03, (iW * iH) / (fW * fH)));
}

export function estimateNisarRangePreflight(input: {
  scenes?: readonly SceneRow[];
  sceneCount?: number;
  aoiBbox?: Bbox | null;
  polarizations: readonly string[];
  isFallbackEstimate?: boolean;
}): NisarRangePreflightEstimate {
  const scenes = input.scenes ?? [];
  const count = Math.max(1, scenes.length || input.sceneCount || 1);
  let rawBytes = 0;
  let weightedAreaRatio = 0;

  if (scenes.length > 0) {
    for (const s of scenes) {
      const size = typeof s.file_size_remote === "number" && Number.isFinite(s.file_size_remote) && s.file_size_remote > 0
        ? s.file_size_remote
        : DEFAULT_NISAR_SCENE_BYTES;
      rawBytes += size;
      weightedAreaRatio += bboxAreaRatio(input.aoiBbox, s.footprint_bbox);
    }
    weightedAreaRatio /= scenes.length;
  } else {
    rawBytes = count * DEFAULT_NISAR_SCENE_BYTES;
    weightedAreaRatio = input.aoiBbox ? 0.18 : 0.25;
  }

  const polCount = Math.max(1, Math.min(4, input.polarizations.length || 1));
  const polScale = Math.min(1, polCount / 2);
  const subsetFraction = Math.min(0.95, Math.max(0.03, weightedAreaRatio * polScale));
  const estimatedSubsetBytes = Math.round(rawBytes * subsetFraction);
  const savedBytes = Math.max(0, rawBytes - estimatedSubsetBytes);
  const savingsPercent = rawBytes > 0 ? Math.round((savedBytes / rawBytes) * 100) : 0;

  return {
    sceneCount: count,
    rawBytes,
    estimatedSubsetBytes,
    savedBytes,
    savingsPercent,
    isFallbackEstimate: input.isFallbackEstimate ?? true,
  };
}
