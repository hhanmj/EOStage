import { useEffect, useMemo, useReducer, useRef, useState } from "react";
import { ChevronDown, ChevronRight, FolderOpen, Loader2 } from "lucide-react";

import { useI18n, type MessageKey, type MessageValues } from "@/i18n";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { OpticalPreparationStatus } from "@/components/OpticalPreparationStatus";
import { formatByteSize } from "@/lib/formatByteSize";
import { preflightNisarSubsetBatchV2, type Bbox, type SceneRow } from "@/lib/bridge";
import {
  landsatContentSelectionLabel,
  landsatHasSurfaceTemperature,
  landsatSelectionSizeBytes,
  type LandsatContentSelection,
} from "@/lib/landsatAssetSelection";
import { landsatAssetDisplayName, landsatAssetRoleLabel } from "@/lib/landsatAssetDetails";
import type { AccountId } from "@/shell/api/types";
import type { DesktopLandsatAssetDetail } from "@/lib/desktopShellBridge";
import {
  SENTINEL2_L1C_BANDS,
  SENTINEL2_REFLECTANCE_BANDS,
  hlsBandOptionsForProduct,
  sentinel2BandPresetLabel,
  type HlsBandPreset,
  type Sentinel2BandPreset,
} from "@/lib/sentinel2BandSelection";
import { setPreciseOrbitPreference } from "@/lib/preciseOrbitPreferences";
import {
  setSavedCompanionGacosPreference,
  setSavedCompanionOrbitPreference,
} from "@/lib/companionDownloadLogic";
import {
  beginDownloadDestinationConfirmation,
  downloadDestinationConfirmationReducer,
  inferUnifiedDatasetFamily,
  isSentinel1CompanionEligible,
  type ConfirmedDownloadTaskDraft,
  type DownloadAcquisitionDraft,
} from "@/lib/downloadDestinationConfirmation";
import {
  estimateNisarSubsetBatch,
  formatNisarSubsetEstimateSummary,
} from "@/lib/nisarSubsetEstimate";
import { CATALOG_METADATA_REGISTRY } from "@/lib/dataSourceCatalogLogic";
import {
  formatHumanSceneLabel,
  isTemporarySystemDownloadPath,
} from "@/lib/storagePathValidation";

function acquisitionSummary(
  draft: DownloadAcquisitionDraft,
  t: (key: MessageKey, values?: MessageValues) => string,
  activeResourceId?: string | null,
): { label: string; details: string[] } {
  if (draft.acquisition.kind === "nisar_subset") {
    const parameters = draft.acquisition.frozenParameters;
    const frequency = typeof parameters.frequency === "string" ? parameters.frequency : "—";
    const polarizations = Array.isArray(parameters.polarizations)
      ? parameters.polarizations.filter((value): value is string => typeof value === "string")
      : [];
    return {
      label: t("panels.downloadDestination.nisarSubset"),
      details: [
        t("panels.downloadDestination.frequency", { value: frequency }),
        t("panels.downloadDestination.polarizations", { value: polarizations.join(" + ") || "—" }),
        t("panels.downloadDestination.clientDerived"),
      ],
    };
  }
  if (draft.acquisition.kind === "sentinel1") {
    const parameters = draft.acquisition.frozenParameters;
    const details: string[] = [];
    if (parameters.download_orbit === true) details.push(t("panels.downloadDestination.companionOrbit"));
    if (parameters.request_gacos === true) details.push(t("panels.downloadDestination.companionGacos"));
    return { label: "Sentinel-1 SLC", details };
  }
  if (draft.acquisition.kind === "sentinel2") {
    const parameters = draft.acquisition.frozenParameters;
    const delivery = parameters.delivery_form === "analysis_bands" ? "analysis_bands" : "safe_archive";
    const isL1c = parameters.product_level === "l1c";
    const selectedBandKeys = Array.isArray(parameters.selected_band_keys)
      ? parameters.selected_band_keys.filter((value): value is string => typeof value === "string")
      : [];
    return {
      label: isL1c ? "Sentinel-2 L1C" : "Sentinel-2 L2A",
      details: [
        delivery === "safe_archive"
          ? t("panels.downloadDestination.deliverySafe")
          : t("panels.downloadDestination.deliveryAnalysisBands", { bands: selectedBandKeys.length }),
      ],
    };
  }
  if (draft.acquisition.kind === "hls") {
    const parameters = draft.acquisition.frozenParameters;
    const product = parameters.hls_product === "L30" ? "L30" : "S30";
    const selectedBandKeys = Array.isArray(parameters.selected_band_keys)
      ? parameters.selected_band_keys.filter((value): value is string => typeof value === "string")
      : [];
    return {
      label: product === "L30" ? "HLS L30 (Landsat 30 m)" : "HLS S30 (Sentinel-2 30 m)",
      details: [
        t("panels.downloadDestination.deliveryAnalysisBands", { bands: selectedBandKeys.length }),
      ],
    };
  }
  if (draft.acquisition.kind === "landsat") {
    const parameters = draft.acquisition.frozenParameters;
    const delivery = parameters.delivery_form;
    const selection = (["surface_reflectance", "surface_temperature", "custom"].includes(String(parameters.content_selection))
      ? parameters.content_selection
      : "all") as LandsatContentSelection;
    const selectedAssetKeys = Array.isArray(parameters.selected_asset_keys)
      ? parameters.selected_asset_keys.filter((value): value is string => typeof value === "string")
      : [];
    return {
      label: "Landsat Collection 2 Level-2",
      details: [
        selectedAssetKeys.length > 0
          ? t("panels.downloadDestination.landsatContentFiles", { selection: landsatContentSelectionLabel(selection), files: selectedAssetKeys.length })
          : t("panels.downloadDestination.landsatContent", { selection: landsatContentSelectionLabel(selection) }),
        delivery === "official_archive"
          ? t("panels.downloadDestination.deliveryOfficialArchive")
          : t("panels.downloadDestination.deliveryExpandedFiles"),
      ],
    };
  }
  if (draft.acquisition.kind === "burst") {
    const parameters = draft.acquisition.frozenParameters;
    const details: string[] = [];
    if (parameters.download_orbit === true) details.push(t("panels.downloadDestination.companionOrbit"));
    if (parameters.request_gacos === true) details.push(t("panels.downloadDestination.companionGacos"));
    return { label: "Sentinel-1 Burst", details };
  }
  if (draft.acquisition.kind === "generic_sar") {
    const parameters = draft.acquisition.frozenParameters;
    if (parameters.dedicated_geodata === true && typeof parameters.dataset_label === "string") {
      const details: string[] = [];
      if (parameters.dedicated_action === "xyz_tiles") {
        if (typeof parameters.tile_source_label === "string" && parameters.tile_source_label.trim()) {
          details.push(`图源：${parameters.tile_source_label}`);
        }
        if (typeof parameters.zoom_label === "string" && parameters.zoom_label.trim()) {
          details.push(`瓦片层级：${parameters.zoom_label}`);
        }
        if (typeof parameters.export_format === "string" && parameters.export_format.trim()) {
          details.push(`导出格式：${parameters.export_format}`);
        }
        return { label: parameters.dataset_label, details };
      }
      if (parameters.dedicated_action === "dem_download") {
        if (typeof parameters.tile_count === "number" && parameters.tile_count > 0) {
          details.push(`预计覆盖 ${parameters.tile_count} 块瓦片`);
        }
        if (typeof parameters.product_level === "string" && parameters.product_level.trim()) {
          details.push(`高程基准：${parameters.product_level}`);
        }
        return { label: parameters.dataset_label, details };
      }
      if (parameters.dedicated_action === "orbit_download") {
        if (typeof parameters.orbit_scene_count === "number" && parameters.orbit_scene_count > 0) {
          details.push(`待匹配景数：${parameters.orbit_scene_count} 景`);
        }
        return { label: parameters.dataset_label, details };
      }
      if (typeof parameters.product_level === "string" && parameters.product_level.trim()) {
        details.push(`产品级别: ${parameters.product_level}`);
      }
      if (typeof parameters.estimated_total_bytes === "number" && parameters.estimated_total_bytes > 0) {
        details.push(`预估完整体积约 ${formatByteSize(parameters.estimated_total_bytes)}`);
      }
      return { label: parameters.dataset_label, details };
    }
    const details: string[] = [];
    const catalogLabel = activeResourceId ? CATALOG_METADATA_REGISTRY[activeResourceId]?.name : undefined;
    if (catalogLabel) details.push(catalogLabel);
    if (parameters.download_orbit === true) details.push(t("panels.downloadDestination.companionOrbit"));
    return { label: t("panels.downloadDestination.dataProduct"), details };
  }
  return { label: draft.acquisition.kind, details: [] };
}

export interface DownloadDestinationConfirmationProps {
  draft: DownloadAcquisitionDraft;
  chooseDirectory: () => Promise<string | null>;
  onCancel: () => void;
  onCancelPreparation?: () => Promise<void>;
  preparationRequestId?: string;
  onConfirm: (draft: ConfirmedDownloadTaskDraft) => Promise<void>;
  selectedScenes?: SceneRow[];
  aoiBbox?: Bbox | null;
  aoiGeometry?: Record<string, unknown> | null;
  activeResourceId?: string | null;
  onConfirmNisarSubset?: (opts: {
    frequency: "A" | "B";
    polarizations: string[];
    outputDir: string;
  }) => Promise<void>;
  onConfirmOpticalDelivery?: (opts: {
    deliveryMode: "cog_subset" | "archive";
    bands?: string[];
    outputDir: string;
  }) => Promise<void>;
  onConfirmSarCompanions?: (opts: {
    includeOrbit: boolean;
    includeGacos: boolean;
    includeDem: boolean;
    outputDir: string;
  }) => Promise<void>;
  duplicateMatches?: { scene_id: string; path: string; size_bytes?: number }[];
  duplicatePolicy?: "skip" | "overwrite";
  onDuplicatePolicyChange?: (p: "skip" | "overwrite") => void;
  onOutputRootChange?: (dir: string) => void;
  onOpenAccountSettings?: (accountId?: AccountId) => void;
}

const NISAR_POL_OPTIONS = ["HH", "HV", "VV", "VH"] as const;

export function DownloadDestinationConfirmation({
  draft,
  chooseDirectory,
  onCancel,
  onCancelPreparation,
  preparationRequestId,
  onConfirm,
  selectedScenes,
  aoiBbox,
  aoiGeometry,
  activeResourceId,
  onConfirmNisarSubset,
  onConfirmOpticalDelivery,
  onConfirmSarCompanions,
  duplicateMatches,
  duplicatePolicy,
  onDuplicatePolicyChange,
  onOutputRootChange,
  onOpenAccountSettings,
}: DownloadDestinationConfirmationProps) {
  const { t } = useI18n();
  const [state, dispatch] = useReducer(
    downloadDestinationConfirmationReducer,
    draft,
    beginDownloadDestinationConfirmation,
  );
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const [cancelPending, setCancelPending] = useState(false);
  const [cancelError, setCancelError] = useState<string | null>(null);
  const [manifestOpen, setManifestOpen] = useState(false);
  const showSubmittingSpinner = useDelayedFlag(state.status === "submitting");

  // Unified delivery sheet controls (Item 3)
  const datasetFamily = useMemo(
    () => inferUnifiedDatasetFamily(draft, activeResourceId),
    [draft, activeResourceId],
  );
  const showSentinel1Companions = useMemo(
    () => isSentinel1CompanionEligible(draft, activeResourceId),
    [draft, activeResourceId],
  );
  const initialFreq = draft.acquisition.frozenParameters?.frequency === "B" ? "B" : "A";
  const initialPols = Array.isArray(draft.acquisition.frozenParameters?.polarizations)
    ? draft.acquisition.frozenParameters.polarizations.filter((p): p is string => typeof p === "string")
    : ["HH", "HV"];
  const [nisarFrequency, setNisarFrequency] = useState<"A" | "B">(initialFreq);
  const [nisarPolarizations, setNisarPolarizations] = useState<string[]>(
    initialPols.length > 0 ? initialPols : ["HH"],
  );
  const [unifiedDeliveryChoice, setUnifiedDeliveryChoice] = useState<"aoi_subset" | "full_archive">(
    draft.acquisition.kind === "nisar_subset" || Boolean(aoiBbox || aoiGeometry)
      ? "aoi_subset"
      : "full_archive",
  );
  const [sarIncludeDem, setSarIncludeDem] = useState(
    draft.acquisition.frozenParameters?.request_dem === true,
  );
  const [liveNisarPreflightStatus, setLiveNisarPreflightStatus] = useState<{
    checked: boolean;
    eligibleCount: number | null;
    availablePolarizations: string[];
  }>({ checked: false, eligibleCount: null, availablePolarizations: [] });

  useEffect(() => {
    if (datasetFamily !== "nisar" || unifiedDeliveryChoice !== "aoi_subset" || !aoiGeometry) {
      return;
    }
    let cancelled = false;
    const itemIds = (selectedScenes?.map((s) => s.scene_id) ?? draft.selectedProductIds).filter(Boolean);
    if (itemIds.length === 0 || !draft.sourceBatchId) return;
    void preflightNisarSubsetBatchV2({
      source_batch_id: draft.sourceBatchId,
      source_item_ids: [...itemIds],
      expected_workbench_revision: draft.sourceBatchRevision,
      subset_draft_geometry: aoiGeometry,
    })
      .then((res) => {
        if (cancelled || !res.ok) return;
        const matchingSel = res.preflight.common_selections.find((s) => s.frequency === nisarFrequency);
        setLiveNisarPreflightStatus({
          checked: true,
          eligibleCount: res.preflight.eligible_items.length,
          availablePolarizations: matchingSel?.polarizations ?? [],
        });
      })
      .catch(() => {
        // Fallback geometric calculation stays active when non-Tauri or batch preflight unavailable
      });
    return () => {
      cancelled = true;
    };
  }, [datasetFamily, unifiedDeliveryChoice, aoiGeometry, selectedScenes, draft.selectedProductIds, draft.sourceBatchId, draft.sourceBatchRevision, nisarFrequency]);

  const [localDuplicatePolicy, setLocalDuplicatePolicy] = useState<"skip" | "overwrite">(duplicatePolicy ?? "skip");
  const resolvedDuplicatePolicy = duplicatePolicy ?? localDuplicatePolicy;
  const handleDuplicatePolicySelect = (policy: "skip" | "overwrite") => {
    setLocalDuplicatePolicy(policy);
    onDuplicatePolicyChange?.(policy);
  };

  const nisarSavings = useMemo(
    () =>
      estimateNisarSubsetBatch({
        scenes: selectedScenes,
        sceneCount: draft.selectedProductCount,
        aoiBbox,
        aoiGeometry,
        polarizations: nisarPolarizations,
        availablePolarizations: liveNisarPreflightStatus.availablePolarizations,
        isFallbackEstimate: !liveNisarPreflightStatus.checked,
      }),
    [
      selectedScenes,
      draft.selectedProductCount,
      aoiBbox,
      aoiGeometry,
      nisarPolarizations,
      liveNisarPreflightStatus.availablePolarizations,
      liveNisarPreflightStatus.checked,
    ],
  );
  const nisarEstimateSummary = useMemo(
    () => formatNisarSubsetEstimateSummary(nisarSavings),
    [nisarSavings],
  );

  if (state.status === "idle") return null;
  const busy = state.status === "choosing_directory" || state.status === "submitting";
  const summary = acquisitionSummary(state.draft, t, activeResourceId);
  const landsatParameters = state.draft.acquisition.kind === "landsat"
    ? state.draft.acquisition.frozenParameters
    : null;
  const landsatDelivery = landsatParameters?.delivery_form;
  const landsatAssets = Array.isArray(landsatParameters?.available_assets)
    ? landsatParameters.available_assets.filter((asset): asset is DesktopLandsatAssetDetail => {
        if (!asset || typeof asset !== "object" || Array.isArray(asset)) return false;
        const record = asset as unknown as Record<string, unknown>;
        return typeof record.asset_key === "string"
          && typeof record.file_name === "string"
          && ["primary", "auxiliary", "metadata"].includes(String(record.role))
          && typeof record.size_bytes === "number"
          && Number.isSafeInteger(record.size_bytes)
          && record.size_bytes > 0;
      })
    : [];
  const landsatContentSelection = (["surface_reflectance", "surface_temperature", "custom"].includes(String(landsatParameters?.content_selection))
    ? landsatParameters?.content_selection
    : "all") as LandsatContentSelection;
  const landsatSelectedAssetKeys = Array.isArray(landsatParameters?.selected_asset_keys)
    ? landsatParameters.selected_asset_keys.filter((value): value is string => typeof value === "string")
    : [];
  const landsatSelectedAssetSet = new Set(landsatSelectedAssetKeys);
  const landsatSelectedSize = landsatSelectionSizeBytes(landsatAssets, landsatSelectedAssetKeys);
  const landsatProductFolder = typeof landsatParameters?.product_folder_name === "string"
    ? landsatParameters.product_folder_name.trim()
    : "";
  const landsatPartialSelection = landsatParameters !== null && landsatContentSelection !== "all";
  const landsatSelectionUnavailable = landsatParameters !== null
    && landsatParameters.delivery_form === "complete_asset_set"
    && landsatAssets.length > 0
    && landsatSelectedAssetKeys.length === 0;
  const landsatDeliveryUnavailable = landsatParameters !== null && (
    (landsatDelivery === "official_archive"
      ? landsatParameters.official_archive_available !== true
      : landsatParameters.complete_asset_set_available !== true)
    || landsatSelectionUnavailable
  );

  const sentinel2Parameters = state.draft.acquisition.kind === "sentinel2"
    ? state.draft.acquisition.frozenParameters
    : null;
  const sentinel2IsL1c = sentinel2Parameters?.product_level === "l1c";
  const sentinel2BandCatalog = sentinel2IsL1c ? SENTINEL2_L1C_BANDS : SENTINEL2_REFLECTANCE_BANDS;
  const sentinel2Delivery = sentinel2Parameters?.delivery_form === "analysis_bands"
    ? "analysis_bands"
    : "safe_archive";
  const sentinel2BandPreset = (["vegetation", "all_10m", "all_reflectance", "custom"].includes(
    String(sentinel2Parameters?.band_preset),
  ) ? sentinel2Parameters?.band_preset : "true_color") as Sentinel2BandPreset;
  const sentinel2SelectedBandKeys = Array.isArray(sentinel2Parameters?.selected_band_keys)
    ? sentinel2Parameters.selected_band_keys.filter(
        (value): value is string => typeof value === "string",
      )
    : [];
  const sentinel2SelectedBandSet = new Set(sentinel2SelectedBandKeys);
  const sentinel2DeliveryUnavailable = sentinel2Parameters !== null && (
    (sentinel2Delivery === "safe_archive"
      ? sentinel2Parameters.safe_archive_available !== true
      : sentinel2Parameters.analysis_bands_available !== true)
    || (sentinel2Delivery === "analysis_bands" && sentinel2SelectedBandKeys.length === 0)
  );

  const hlsParameters = state.draft.acquisition.kind === "hls"
    ? state.draft.acquisition.frozenParameters
    : null;
  const hlsProductVal: "L30" | "S30" = hlsParameters?.hls_product === "L30" ? "L30" : "S30";
  const hlsBandCatalog = hlsBandOptionsForProduct(hlsProductVal);
  const hlsBandPreset = (["false_color", "swir", "all_bands", "custom"].includes(
    String(hlsParameters?.band_preset),
  ) ? hlsParameters?.band_preset : "true_color") as HlsBandPreset;
  const hlsSelectedBandKeys = Array.isArray(hlsParameters?.selected_band_keys)
    ? hlsParameters.selected_band_keys.filter(
        (value): value is string => typeof value === "string",
      )
    : [];
  const hlsSelectedBandSet = new Set(hlsSelectedBandKeys);
  const hlsDeliveryUnavailable = hlsParameters !== null && hlsSelectedBandKeys.length === 0;

  const useUnifiedNisarSheet = state.draft.acquisition.kind === "nisar_subset" && Boolean(onConfirmNisarSubset);
  // Item 6: 光学非裁剪下载不再重复内嵌「按研究区裁剪」切换器；统一保留波段/交付形式挑选。
  void onConfirmOpticalDelivery;
  const useUnifiedOpticalSheet = false;

  async function handleChooseDirectory() {
    if (busy) return;
    dispatch({ type: "begin_directory_selection" });
    try {
      const outputRoot = await chooseDirectory();
      if (outputRoot && isTemporarySystemDownloadPath(outputRoot)) {
        dispatch({ type: "directory_selection_cancelled" });
        dispatch({ type: "output_root_changed", outputRoot });
        return;
      }
      dispatch(outputRoot
        ? { type: "directory_selected", outputRoot }
        : { type: "directory_selection_cancelled" });
      if (outputRoot) {
        onOutputRootChange?.(outputRoot);
      }
    } catch (cause) {
      dispatch({
        type: "directory_selected",
        outputRoot: "",
      });
      void cause;
    }
  }

  async function handleConfirm() {
    if (state.status !== "reviewing") return;
    setCancelError(null); setCancelPending(false);
    const submitting = downloadDestinationConfirmationReducer(state, { type: "confirm" });
    dispatch({ type: "confirm" });
    if (submitting.status !== "submitting") return;
    try {
      if (useUnifiedNisarSheet && unifiedDeliveryChoice === "aoi_subset" && onConfirmNisarSubset) {
        await onConfirmNisarSubset({
          frequency: nisarFrequency,
          polarizations: nisarPolarizations,
          outputDir: submitting.confirmedDraft.outputRoot,
        });
        if (onConfirmSarCompanions && showSentinel1Companions) {
          const includeOrbit = state.draft.acquisition.frozenParameters.download_orbit === true;
          const includeGacos = state.draft.acquisition.frozenParameters.request_gacos === true;
          if (includeOrbit || includeGacos || sarIncludeDem) {
            await onConfirmSarCompanions({
              includeOrbit,
              includeGacos,
              includeDem: sarIncludeDem,
              outputDir: submitting.confirmedDraft.outputRoot,
            });
          }
        }
      } else {
        await onConfirm(submitting.confirmedDraft);
        if (onConfirmSarCompanions && showSentinel1Companions) {
          const includeOrbit = state.draft.acquisition.frozenParameters.download_orbit === true;
          const includeGacos = state.draft.acquisition.frozenParameters.request_gacos === true;
          if (includeOrbit || includeGacos || sarIncludeDem) {
            await onConfirmSarCompanions({
              includeOrbit,
              includeGacos,
              includeDem: sarIncludeDem,
              outputDir: submitting.confirmedDraft.outputRoot,
            });
          }
        }
      }
      dispatch({ type: "submission_succeeded" });
    } catch (cause) {
      dispatch({
        type: "submission_failed",
        message: cause instanceof Error ? cause.message : String(cause),
      });
    } finally {
      setCancelPending(false);
    }
  }

  async function cancelPreparation() {
    if (state.status !== "submitting" || !onCancelPreparation || cancelPending) return;
    setCancelPending(true); setCancelError(null);
    try { await onCancelPreparation(); }
    catch { setCancelError(t("panels.downloadDestination.cancelUnconfirmed")); setCancelPending(false); }
    // Only the original submission response decides whether creation or cancellation won.
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape" && !busy) {
      event.preventDefault();
      onCancel();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ) ?? [],
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <ModalOverlay closeOnBackdropClick={false} onClose={onCancel} className="z-[1700]">
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="download-destination-title"
        className="flex max-h-[88vh] w-full max-w-[680px] flex-col overflow-hidden rounded-sp-modal border border-sp-line bg-sp-surface text-sp-text shadow-modal"
        onKeyDown={handleKeyDown}
      >
        <div className="shrink-0 border-b border-sp-line px-5 py-4">
          <h2 id="download-destination-title" className="text-base font-semibold text-sp-text">
            {t("panels.downloadDestination.title")}
          </h2>
          <p className="mt-1 text-xs text-sp-text-secondary">
            {t("panels.downloadDestination.description")}
          </p>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          <div className="grid gap-1.5 rounded-sp-md border border-sp-line bg-sp-surface-muted/60 px-3.5 py-2.5 text-xs sm:grid-cols-2">
          <div><span className="text-muted-foreground">{t("panels.downloadDestination.product")}</span>{summary.label}</div>
          <div><span className="text-muted-foreground">{t("panels.downloadDestination.content")}</span>{t("panels.downloadDestination.productCount", { products: draft.selectedProductCount })}</div>
          {summary.details.map((detail) => <div key={detail}>{detail}</div>)}
          <div>
            <span className="text-muted-foreground">{t("panels.downloadDestination.layout")}</span>
            <span
              title={
                draft.layout === "direct"
                  ? "文件直接保存在所选目录下，不额外新建分类子文件夹"
                  : draft.layout === "sar_dataset_processing"
                    ? "在所选目录下按传感器/产品类型自动分文件夹归档"
                    : undefined
              }
            >
              {draft.layout === "direct"
                ? t("panels.downloadDestination.layoutDirectLabel")
                : draft.layout === "sar_dataset_processing"
                  ? t("panels.downloadDestination.layoutStructuredLabel")
                  : draft.layout}
            </span>
          </div>
          <div><span className="text-muted-foreground">{t("panels.downloadDestination.concurrency")}</span>{draft.concurrency}</div>
          {draft.selectionScope === "all_matching" && draft.allMatchPreview && (
            <div data-testid="all-match-confirmation-summary" className="space-y-1 border-t border-border/60 pt-2 text-xs sm:col-span-2">
              <div className="font-medium text-foreground">
                {t("panels.downloadDestination.allMatchScopeNotice", {
                  matched: draft.allMatchPreview.matched_count,
                  admissible: draft.allMatchPreview.admissible_count,
                })}
              </div>
              <div className="text-muted-foreground">
                {t("panels.downloadDestination.allMatchAccountingDetail", {
                  admissible: draft.allMatchPreview.admissible_count,
                  skipped: draft.allMatchPreview.skipped_active_count,
                  rejected: draft.allMatchPreview.rejected_count,
                })}
              </div>
              <div className="text-muted-foreground">
                <span>{t("panels.downloadDestination.allMatchEstimatedSize")}</span>
                <span className="font-mono text-foreground">
                  {draft.allMatchPreview.known_bytes > 0 && draft.allMatchPreview.unknown_size_count === 0
                    ? formatByteSize(draft.allMatchPreview.known_bytes)
                    : draft.allMatchPreview.known_bytes > 0
                    ? t("panels.downloadDestination.allMatchKnownSizeWithUnknown", {
                        knownSize: formatByteSize(draft.allMatchPreview.known_bytes),
                        unknownCount: draft.allMatchPreview.unknown_size_count,
                      })
                    : t("panels.downloadDestination.allMatchSizeUnknown", {
                        unknownCount: draft.allMatchPreview.unknown_size_count || draft.allMatchPreview.admissible_count,
                      })}
                </span>
              </div>
            </div>
          )}
        </div>

        <div className="mt-4 space-y-1.5">
          <label htmlFor="download-destination-output" className="text-sm font-medium">{t("panels.downloadDestination.destination")}</label>
          <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-2">
            <Input
              id="download-destination-output"
              value={state.outputRoot}
              onChange={(event) => {
                const next = event.target.value;
                dispatch({ type: "output_root_changed", outputRoot: next });
                onOutputRootChange?.(next);
              }}
              placeholder="输入或粘贴本地目录路径，或点击右侧选择目录"
              disabled={busy}
              title={state.outputRoot}
              className="font-mono text-xs"
            />
            <Button type="button" variant="outline" disabled={busy} onClick={() => void handleChooseDirectory()}>
              {state.status === "choosing_directory"
                ? <Loader2 className="h-4 w-4 animate-spin" />
                : <FolderOpen className="h-4 w-4" />}
              {t("panels.downloadDestination.chooseDirectory")}
            </Button>
          </div>
        </div>

        {landsatProductFolder && (
          <p className="mt-1 text-xs text-muted-foreground break-all">
            {t("panels.downloadDestination.productFolder", { path: `${state.outputRoot.replace(/[\\/]+$/, "")}/${landsatProductFolder}` })}
          </p>
        )}

        {selectedScenes && selectedScenes.length > 0 && (
          <div className="mt-4 rounded-sp-md border border-sp-line bg-sp-surface-muted/40">
            <button
              type="button"
              data-testid="toggle-scene-manifest"
              onClick={() => setManifestOpen((prev) => !prev)}
              className="flex w-full items-center justify-between px-3 py-2 text-xs font-medium text-sp-text hover:bg-sp-hover cursor-pointer"
            >
              <span className="inline-flex items-center gap-1.5">
                {manifestOpen ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
                已选景清单（{selectedScenes.length} 景）
              </span>
              <span className="text-sp-text-muted">{manifestOpen ? "收起" : "展开明细"}</span>
            </button>
            {manifestOpen && (
              <div className="max-h-40 divide-y divide-sp-line overflow-y-auto border-t border-sp-line px-3 py-1.5 text-xs">
                {selectedScenes.map((scene) => {
                  const displayName = formatHumanSceneLabel(scene);
                  return (
                    <div key={scene.scene_id} className="flex items-center justify-between gap-2 py-1">
                      <span className="truncate font-mono text-sp-text" title={displayName}>
                        {displayName}
                      </span>
                      <span className="shrink-0 text-sp-text-muted tabular-nums">
                        {(scene.acquisition_datetime || "").slice(0, 10)}
                        {typeof scene.file_size_remote === "number" && scene.file_size_remote > 0
                          ? ` · ${formatByteSize(scene.file_size_remote)}`
                          : ""}
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {duplicateMatches && duplicateMatches.length > 0 && (
          <div
            data-testid="download-duplicate-policy"
            className="mt-4 space-y-2 rounded-sp-md border border-sp-warning-border/60 bg-sp-warning-bg/20 p-3 text-xs"
          >
            <div className="font-semibold text-sp-text">
              {t("panels.downloadDestination.existingDuplicateTitle", {
                count: duplicateMatches.length,
              })}
            </div>
            <div className="max-h-24 space-y-1 overflow-y-auto text-sp-text-muted">
              {duplicateMatches.map((dup) => (
                <div key={`${dup.scene_id}:${dup.path}`} className="flex items-center justify-between gap-2 font-mono">
                  <span className="truncate" title={dup.path}>
                    {dup.scene_id} · {dup.path}
                  </span>
                  {typeof dup.size_bytes === "number" && dup.size_bytes > 0 && (
                    <span className="shrink-0 tabular-nums">{formatByteSize(dup.size_bytes)}</span>
                  )}
                </div>
              ))}
            </div>
            <div className="flex flex-wrap items-center gap-4 pt-1">
              <label className="inline-flex items-center gap-1.5 cursor-pointer text-sp-text">
                <input
                  type="radio"
                  name="download-duplicate-policy"
                  value="skip"
                  checked={resolvedDuplicatePolicy === "skip"}
                  disabled={busy}
                  onChange={() => handleDuplicatePolicySelect("skip")}
                />
                <span>{t("panels.downloadDestination.existingDuplicateSkip")}</span>
              </label>
              <label className="inline-flex items-center gap-1.5 cursor-pointer text-sp-text">
                <input
                  type="radio"
                  name="download-duplicate-policy"
                  value="overwrite"
                  checked={resolvedDuplicatePolicy === "overwrite"}
                  disabled={busy}
                  onChange={() => handleDuplicatePolicySelect("overwrite")}
                />
                <span>{t("panels.downloadDestination.existingDuplicateOverwrite")}</span>
              </label>
            </div>
          </div>
        )}

        {useUnifiedNisarSheet && (
          <div className="mt-4 space-y-3 border-t border-sp-line pt-3.5">
            <fieldset className="space-y-2 border-0 p-0 text-sm">
              <legend className="mb-1.5 text-xs font-semibold text-sp-text">交付方式</legend>
              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="radio"
                  name="unified-delivery-mode"
                  value="aoi_subset"
                  checked={unifiedDeliveryChoice === "aoi_subset"}
                  disabled={busy}
                  onChange={() => setUnifiedDeliveryChoice("aoi_subset")}
                />
                <span>
                  <span className="block font-medium">按当前研究区裁剪下载 (AOI Subset)</span>
                  <span className="block text-xs text-muted-foreground">
                    {datasetFamily === "nisar"
                      ? "通过 HDF5 HTTP Range 仅拉取研究区范围内的 NISAR 频段与极化切片，显著节省传输带宽。"
                      : "按研究区边界裁剪并下载所选 COG 波段，免去下载整景冗余数据。"}
                  </span>
                </span>
              </label>
              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="radio"
                  name="unified-delivery-mode"
                  value="full_archive"
                  checked={unifiedDeliveryChoice === "full_archive"}
                  disabled={busy}
                  onChange={() => setUnifiedDeliveryChoice("full_archive")}
                />
                <span>
                  <span className="block font-medium">整景原包下载</span>
                  <span className="block text-xs text-muted-foreground">
                    {datasetFamily === "nisar"
                      ? "下载完整原始 NISAR HDF5 归档包。"
                      : "下载官方完整归档包（Sentinel-2 .SAFE / Landsat .tar）。"}
                  </span>
                </span>
              </label>
            </fieldset>

            {useUnifiedNisarSheet && unifiedDeliveryChoice === "aoi_subset" && (
              <div className="space-y-3 rounded-sp-md border border-sp-line bg-sp-surface-muted/50 p-3 text-xs">
                <div className="flex flex-wrap items-center gap-4">
                  <span className="font-medium text-sp-text">频段 (Frequency)：</span>
                  {(["A", "B"] as const).map((freq) => (
                    <label key={freq} className="inline-flex items-center gap-1.5 cursor-pointer">
                      <input
                        type="radio"
                        name="nisar-frequency"
                        value={freq}
                        checked={nisarFrequency === freq}
                        disabled={busy}
                        onChange={() => setNisarFrequency(freq)}
                      />
                      <span className="font-mono font-medium">Frequency {freq}</span>
                    </label>
                  ))}
                </div>
                <div className="flex flex-wrap items-center gap-3">
                  <span className="font-medium text-sp-text">极化通道：</span>
                  {NISAR_POL_OPTIONS.map((pol) => {
                    const checked = nisarPolarizations.includes(pol);
                    return (
                      <label key={pol} className="inline-flex items-center gap-1.5 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={checked}
                          disabled={busy}
                          onChange={() => {
                            setNisarPolarizations((prev) =>
                              checked ? prev.filter((p) => p !== pol) : [...prev, pol],
                            );
                          }}
                        />
                        <span className="font-mono">{pol}</span>
                      </label>
                    );
                  })}
                </div>
                <div
                  data-testid="nisar-range-v2-preflight"
                  className="space-y-2 rounded-sp-sm border border-sp-line bg-sp-surface p-2"
                >
                  <div className="grid grid-cols-3 gap-2">
                    <div>
                      <span className="block text-sp-text-muted">原始整景估算</span>
                      <span className="font-mono font-medium text-sp-text">
                        {formatByteSize(nisarSavings.rawBytes)}
                      </span>
                    </div>
                    <div>
                      <span className="block text-sp-text-muted">AOI 裁剪后预计</span>
                      <span className="font-mono font-semibold text-sp-text">
                        {formatByteSize(nisarSavings.estimatedSubsetBytes)}
                      </span>
                    </div>
                    <div>
                      <span className="block text-sp-text-muted">裁剪预计节省</span>
                      <span className="font-mono font-semibold text-sp-accent">
                        {nisarSavings.savingsPercent}% ({formatByteSize(nisarSavings.savedBytes)})
                      </span>
                    </div>
                  </div>
                  <div
                    data-testid="nisar-subset-estimate-summary"
                    className="border-t border-sp-line/60 pt-1.5 text-[12px] text-sp-text-secondary"
                  >
                    {nisarEstimateSummary}
                  </div>
                </div>
              </div>
            )}
          </div>
        )}

        {showSentinel1Companions && state.draft.acquisition.frozenParameters.allow_companion_orbit === true && (
          <div className="mt-4 space-y-1.5 text-sm">
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                checked={state.draft.acquisition.frozenParameters.download_orbit === true}
                disabled={busy}
                onChange={(event) => {
                  const enabled = event.target.checked;
                  dispatch({ type: "companion_orbit_changed", enabled });
                  const sourceKey = typeof state.draft.acquisition.frozenParameters.source_key === "string"
                    ? state.draft.acquisition.frozenParameters.source_key
                    : "";
                  if (sourceKey) {
                    setPreciseOrbitPreference(sourceKey, enabled);
                    setSavedCompanionOrbitPreference(sourceKey, enabled);
                  }
                }}
              />
              {t("panels.downloadDestination.companionOrbitOption")}
            </label>
            {state.draft.acquisition.frozenParameters.download_orbit === true && (
              <span className="inline-block rounded-full bg-sp-surface-muted text-sp-accent px-2 py-0.5 text-xs font-medium">
                {t("panels.downloadDestination.defaultPreciseOrbitNotice")}
              </span>
            )}
            <p className="text-xs text-muted-foreground break-all">
              {t("panels.downloadDestination.sarscapeDirectory", { path: state.outputRoot.replace(/[\/]+$/, "") || t("panels.downloadDestination.selectedSlcDirectory") })}
            </p>
            <p className="text-xs text-muted-foreground">{t("panels.downloadDestination.orbitProgressNote")}</p>
          </div>
        )}

        {showSentinel1Companions && state.draft.acquisition.frozenParameters.allow_companion_gacos === true && (
          <div className="mt-3 space-y-1.5 text-sm border-t border-border/50 pt-3">
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                data-testid="confirm-checkbox-gacos"
                checked={state.draft.acquisition.frozenParameters.request_gacos === true}
                disabled={busy}
                onChange={(event) => {
                  const enabled = event.target.checked;
                  dispatch({ type: "companion_gacos_changed", enabled });
                  const sourceKey = typeof state.draft.acquisition.frozenParameters.source_key === "string"
                    ? state.draft.acquisition.frozenParameters.source_key
                    : "";
                  if (sourceKey) {
                    setSavedCompanionGacosPreference(sourceKey, enabled);
                  }
                }}
              />
              <span>{t("panels.downloadDestination.companionGacos")}（大气延迟 ZTD）</span>
            </label>
            <p className="text-xs text-muted-foreground">
              按所选影像日期与空间范围自动向 www.gacos.net 提交申请，结果生成后会发往已配置的通知邮箱，并在下载中心跟进导入。
            </p>
          </div>
        )}

        {onConfirmSarCompanions && showSentinel1Companions && (
          <div
            data-testid="confirm-sar-companion-section"
            className="mt-3 space-y-1.5 text-sm border-t border-border/50 pt-3"
          >
            <div className="text-xs font-semibold text-sp-text">Sentinel-1 伴随辅助数据</div>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="checkbox"
                data-testid="confirm-checkbox-dem"
                checked={sarIncludeDem}
                disabled={busy}
                onChange={(event) => setSarIncludeDem(event.target.checked)}
              />
              <span>30m 参考 DEM</span>
            </label>
          </div>
        )}

        {state.draft.acquisition.kind === "sentinel2" && sentinel2Parameters && (
          <>
            <div className="mt-4 border-t border-sp-line pt-3.5">
              <fieldset className="space-y-2 border-0 p-0 text-sm">
                <legend className="mb-1.5 text-xs font-semibold text-sp-text">{t("panels.downloadDestination.deliveryForm")}</legend>
                <label className="flex items-start gap-2">
                  <input
                    type="radio"
                    name="sentinel2-delivery-form"
                    value="safe_archive"
                    checked={sentinel2Delivery === "safe_archive"}
                    disabled={busy || sentinel2Parameters.safe_archive_available !== true}
                    onChange={() => dispatch({ type: "sentinel2_delivery_changed", deliveryForm: "safe_archive" })}
                  />
                  <span>
                    <span className="block font-medium">{t("panels.downloadDestination.safeProduct")}</span>
                    <span className="block text-xs text-muted-foreground">
                      {t("panels.downloadDestination.safeProductDescription")}
                    </span>
                  </span>
                </label>
                <label className="flex items-start gap-2">
                  <input
                    type="radio"
                    name="sentinel2-delivery-form"
                    value="analysis_bands"
                    checked={sentinel2Delivery === "analysis_bands"}
                    disabled={busy || sentinel2Parameters.analysis_bands_available !== true}
                    onChange={() => dispatch({ type: "sentinel2_delivery_changed", deliveryForm: "analysis_bands" })}
                  />
                  <span>
                    <span className="block font-medium">{t("panels.downloadDestination.analysisBands")}</span>
                    <span className="block text-xs text-muted-foreground">
                      {sentinel2IsL1c
                        ? "按波段保存独立 .jp2 文件集（支持仅选所需波段）；不是完整 .SAFE 包。"
                        : t("panels.downloadDestination.analysisBandsDescription")}
                    </span>
                  </span>
                </label>
                {sentinel2Parameters.safe_archive_available !== true && (
                  <p className="text-xs text-amber-700 dark:text-amber-300">
                    {t("panels.downloadDestination.safeUnavailable")}
                    {onOpenAccountSettings && (
                      <button
                        type="button"
                        data-testid="confirm-open-cdse-account-btn"
                        disabled={busy}
                        onClick={() => onOpenAccountSettings("cdse")}
                        className="ml-2 inline-flex items-center font-medium text-sp-accent underline hover:opacity-80 disabled:opacity-50 cursor-pointer"
                      >
                        前往配置 CDSE 账号
                      </button>
                    )}
                  </p>
                )}
              </fieldset>
            </div>

            {sentinel2Delivery === "analysis_bands" && (
              <div className="mt-4 border-t border-sp-line pt-3.5">
                <fieldset className="space-y-2 border-0 p-0 text-sm">
                  <legend className="mb-1.5 text-xs font-semibold text-sp-text">{t("panels.downloadDestination.analysisBandContent")}</legend>
                  {([
                    ["true_color", t("panels.downloadDestination.presetTrueColor"), "B02 + B03 + B04"],
                    ["vegetation", t("panels.downloadDestination.presetVegetation"), sentinel2IsL1c ? "B08 + B04 + B03 + B02" : "B04 + B08"],
                    ["all_10m", t("panels.downloadDestination.presetAll10m"), "B02 + B03 + B04 + B08"],
                    [
                      "all_reflectance",
                      t("panels.downloadDestination.presetAllReflectance"),
                      sentinel2IsL1c
                        ? "13 个原生分辨率波段 (.jp2，含 B10 卷云)"
                        : t("panels.downloadDestination.presetAllReflectanceDescription"),
                    ],
                    ["custom", t("panels.downloadDestination.presetCustom"), t("panels.downloadDestination.presetCustomDescription")],
                  ] as const).map(([preset, label, description]) => (
                    <label key={preset} className="flex items-start gap-2">
                      <input
                        type="radio"
                        name="sentinel2-band-preset"
                        value={preset}
                        checked={sentinel2BandPreset === preset}
                        disabled={busy}
                        onChange={() => dispatch({ type: "sentinel2_band_preset_changed", preset })}
                      />
                      <span>
                        <span className="block font-medium">{label}</span>
                        <span className="block text-xs text-muted-foreground">{description}</span>
                      </span>
                    </label>
                  ))}
                  <div className="rounded-md bg-sp-surface-muted px-2.5 py-1.5 text-xs text-sp-text-secondary">
                    {t("panels.downloadDestination.selectedBands", { selected: sentinel2SelectedBandKeys.length, total: sentinel2BandCatalog.length })}
                    {" · "}{sentinel2BandPresetLabel(sentinel2BandPreset)}
                  </div>
                  {sentinel2BandPreset === "custom" && (
                    <div className="max-h-64 space-y-0.5 overflow-y-auto rounded-sp-md border border-sp-line bg-sp-card p-1.5">
                      {sentinel2BandCatalog.map((band) => (
                        <label key={band.key} className="flex items-start gap-2 rounded-sp-sm px-2 py-1.5 hover:bg-sp-hover">
                          <input
                            type="checkbox"
                            checked={sentinel2SelectedBandSet.has(band.key)}
                            disabled={busy}
                            onChange={() => dispatch({ type: "sentinel2_band_toggled", bandKey: band.key })}
                          />
                          <span className="min-w-0 flex-1">
                            <span className="flex flex-wrap items-baseline justify-between gap-x-2">
                              <span className="font-medium">{band.label}</span>
                              <span className="font-mono text-xs text-muted-foreground">{band.resolutionM} m</span>
                            </span>
                            <span className="block text-xs text-muted-foreground">{band.description}</span>
                          </span>
                        </label>
                      ))}
                    </div>
                  )}
                  <p className="text-xs text-muted-foreground">
                    {sentinel2IsL1c
                      ? "分析波段将按原生 JP2 保存至独立产品目录。"
                      : t("panels.downloadDestination.analysisBandsNote")}
                  </p>
                </fieldset>
              </div>
            )}
          </>
        )}

        {state.draft.acquisition.kind === "hls" && hlsParameters && (
          <div className="mt-4 border-t border-sp-line pt-3.5">
            <fieldset className="space-y-2 border-0 p-0 text-sm">
              <legend className="mb-1.5 text-xs font-semibold text-sp-text">
                {t("panels.downloadDestination.analysisBandContent")}
              </legend>
              {([
                ["true_color", t("panels.downloadDestination.presetTrueColor"), "B04 + B03 + B02 (30 m COG)"],
                ["false_color", "假彩色 (NIR)", hlsProductVal === "L30" ? "B05 + B04 + B03" : "B8A + B04 + B03"],
                ["swir", "短波红外 (SWIR)", hlsProductVal === "L30" ? "B07 + B06 + B04" : "B12 + B11 + B04"],
                ["all_bands", "全部官方波段与质量层", `${hlsBandCatalog.length} 个原生 30 m GeoTIFF（含 Fmask）`],
                ["custom", t("panels.downloadDestination.presetCustom"), t("panels.downloadDestination.presetCustomDescription")],
              ] as const).map(([preset, label, description]) => (
                <label key={preset} className="flex items-start gap-2">
                  <input
                    type="radio"
                    name="hls-band-preset"
                    value={preset}
                    checked={hlsBandPreset === preset}
                    disabled={busy}
                    onChange={() => dispatch({ type: "hls_band_preset_changed", preset })}
                  />
                  <span>
                    <span className="block font-medium">{label}</span>
                    <span className="block text-xs text-muted-foreground">{description}</span>
                  </span>
                </label>
              ))}
              <div className="rounded-md bg-sp-surface-muted px-2.5 py-1.5 text-xs text-sp-text-secondary">
                {t("panels.downloadDestination.selectedBands", {
                  selected: hlsSelectedBandKeys.length,
                  total: hlsBandCatalog.length,
                })}
              </div>
              {hlsBandPreset === "custom" && (
                <div className="max-h-64 space-y-0.5 overflow-y-auto rounded-sp-md border border-sp-line bg-sp-card p-1.5">
                  {hlsBandCatalog.map((band) => (
                    <label key={band.key} className="flex items-start gap-2 rounded-sp-sm px-2 py-1.5 hover:bg-sp-hover">
                      <input
                        type="checkbox"
                        checked={hlsSelectedBandSet.has(band.key)}
                        disabled={busy}
                        onChange={() => dispatch({ type: "hls_band_toggled", bandKey: band.key })}
                      />
                      <span className="min-w-0 flex-1">
                        <span className="flex flex-wrap items-baseline justify-between gap-x-2">
                          <span className="font-medium">{band.label}</span>
                          <span className="font-mono text-xs text-muted-foreground">{band.resolutionM} m</span>
                        </span>
                        <span className="block text-xs text-muted-foreground">{band.description}</span>
                      </span>
                    </label>
                  ))}
                </div>
              )}
            </fieldset>
          </div>
        )}

        {state.draft.acquisition.kind === "landsat" && landsatParameters && (
          <>
            <div className="mt-4 border-t border-sp-line pt-3.5">
              <fieldset className="space-y-2 border-0 p-0 text-sm">
                <legend className="mb-1.5 text-xs font-semibold text-sp-text">{t("panels.downloadDestination.downloadContent")}</legend>
                {([
                  ["all", t("panels.downloadDestination.completeProduct"), landsatAssets.length > 0 ? t("panels.downloadDestination.fileCount", { files: landsatAssets.length }) : t("panels.downloadDestination.completeProduct")],
                  ["surface_reflectance", t("panels.downloadDestination.surfaceReflectance"), landsatAssets.length > 0 ? t("panels.downloadDestination.surfaceReflectanceDescription") : t("panels.downloadDestination.fileListRequired")],
                  ["surface_temperature", t("panels.downloadDestination.surfaceTemperature"), t("panels.downloadDestination.surfaceTemperatureDescription")],
                  ["custom", t("panels.downloadDestination.customFiles"), t("panels.downloadDestination.customFilesDescription")],
                ] as const).filter(([selection]) => landsatParameters.batch_complete_files !== true || selection === "all")
                  .map(([selection, label, description]) => {
                  const disabled = busy
                    || (selection !== "all" && landsatAssets.length === 0)
                    || (selection === "surface_temperature" && !landsatHasSurfaceTemperature(landsatAssets));
                  return (
                    <label key={selection} className="flex items-start gap-2">
                      <input
                        type="radio"
                        name="landsat-content-selection"
                        value={selection}
                        checked={landsatContentSelection === selection}
                        disabled={disabled}
                        onChange={() => dispatch({ type: "landsat_content_changed", selection })}
                      />
                      <span>
                        <span className="block font-medium">{label}</span>
                        <span className="block text-xs text-muted-foreground">{description}</span>
                      </span>
                    </label>
                  );
                })}
                {landsatAssets.length > 0 && (
                  <div className="rounded-md bg-sp-surface-muted px-2.5 py-1.5 text-xs text-sp-text-secondary">
                    {t("panels.downloadDestination.selectedFiles", { selected: landsatSelectedAssetKeys.length, total: landsatAssets.length })}
                    {landsatSelectedAssetKeys.length > 0 ? ` · ${formatByteSize(landsatSelectedSize)}` : ""}
                  </div>
                )}
                {landsatContentSelection === "custom" && landsatAssets.length > 0 && (
                  <div className="max-h-64 space-y-0.5 overflow-y-auto rounded-sp-md border border-sp-line bg-sp-card p-1.5">
                    {landsatAssets.map((asset) => (
                      <label key={asset.asset_key} className="flex items-start gap-2 rounded-sp-sm px-2 py-1.5 hover:bg-sp-hover">
                        <input
                          type="checkbox"
                          checked={landsatSelectedAssetSet.has(asset.asset_key)}
                          disabled={busy}
                          onChange={() => dispatch({ type: "landsat_asset_toggled", assetKey: asset.asset_key })}
                        />
                        <span className="min-w-0 flex-1">
                          <span className="flex flex-wrap items-baseline justify-between gap-x-2">
                            <span className="font-medium">{landsatAssetDisplayName(asset.asset_key)}</span>
                            <span className="font-mono text-xs text-muted-foreground">{formatByteSize(asset.size_bytes)}</span>
                          </span>
                          <span className="block truncate font-mono text-xs text-muted-foreground" title={asset.file_name}>
                            {asset.file_name}
                          </span>
                          <span className="block text-xs text-muted-foreground">{landsatAssetRoleLabel(asset.role)}</span>
                        </span>
                      </label>
                    ))}
                  </div>
                )}
                {landsatAssets.length === 0 && landsatParameters.batch_complete_files !== true && (
                  <p className="text-xs text-muted-foreground">{t("panels.downloadDestination.noFileList")}</p>
                )}
              </fieldset>
            </div>

            {landsatPartialSelection ? (
              <div className="mt-4 rounded-sp-md border border-sp-line bg-sp-surface-muted/60 px-3 py-2 text-xs text-sp-text-secondary">
                {t("panels.downloadDestination.expandedFilesNote")}
              </div>
            ) : (
              <div className="mt-4 border-t border-sp-line pt-3.5">
                <fieldset className="space-y-2 border-0 p-0 text-sm">
                  <legend className="mb-1.5 text-xs font-semibold text-sp-text">{t("panels.downloadDestination.deliveryForm")}</legend>
                  <label className="flex items-start gap-2">
                    <input
                      type="radio"
                      name="landsat-delivery-form"
                      value="official_archive"
                      checked={landsatDelivery === "official_archive"}
                      disabled={busy || landsatParameters.official_archive_available !== true}
                      onChange={() => dispatch({ type: "landsat_delivery_changed", deliveryForm: "official_archive" })}
                    />
                    <span>
                      <span className="block font-medium">{t("panels.downloadDestination.officialArchive")}</span>
                      <span className="block text-xs text-muted-foreground">{t("panels.downloadDestination.officialArchiveDescription")}</span>
                    </span>
                  </label>
                  <label className="flex items-start gap-2">
                    <input
                      type="radio"
                      name="landsat-delivery-form"
                      value="complete_asset_set"
                      checked={landsatDelivery === "complete_asset_set"}
                      disabled={busy || landsatParameters.complete_asset_set_available !== true}
                      onChange={() => dispatch({ type: "landsat_delivery_changed", deliveryForm: "complete_asset_set" })}
                    />
                    <span>
                      <span className="block font-medium">{t("panels.downloadDestination.expandedFileSet")}</span>
                      <span className="block text-xs text-muted-foreground">{t("panels.downloadDestination.expandedFileSetDescription")}</span>
                    </span>
                  </label>
                  {landsatParameters.official_archive_available !== true && (
                    <p className="text-xs text-amber-700 dark:text-amber-300">
                      {typeof landsatParameters.official_archive_unavailable_reason === "string"
                        ? landsatParameters.official_archive_unavailable_reason
                        : t("panels.downloadDestination.officialArchiveUnavailable")}
                      {onOpenAccountSettings && (
                        <button
                          type="button"
                          data-testid="confirm-open-usgs-account-btn"
                          disabled={busy}
                          onClick={() => onOpenAccountSettings("usgs_m2m")}
                          className="ml-2 inline-flex items-center font-medium text-sp-accent underline hover:opacity-80 disabled:opacity-50 cursor-pointer"
                        >
                          前往配置 USGS M2M 账号
                        </button>
                      )}
                    </p>
                  )}
                </fieldset>
              </div>
            )}
          </>
        )}

        {state.status === "submitting" && preparationRequestId && (
          <OpticalPreparationStatus requestId={preparationRequestId} total={state.draft.selectedProductCount} />
        )}
        {state.error && (
          <div role="alert" className="mt-3 max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-sp-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {state.error}
          </div>
        )}
        </div>

        <div className="flex shrink-0 items-center justify-end gap-2 border-t border-sp-line bg-sp-surface-muted/40 px-5 py-3">
          {cancelError && <span role="alert" className="text-xs text-destructive">{cancelError}</span>}
          {state.status === "submitting" && onCancelPreparation && (
            <Button type="button" variant="outline" disabled={cancelPending} onClick={() => void cancelPreparation()}>
              {cancelPending ? t("panels.downloadDestination.cancelling") : t("panels.downloadDestination.cancelPreparation")}
            </Button>
          )}
          <Button type="button" variant="outline" disabled={busy} onClick={onCancel}>{t("panels.downloadDestination.cancel")}</Button>
          <Button
            type="button"
            aria-busy={state.status === "submitting"}
            disabled={
              busy
              || !state.outputRoot.trim()
              || landsatDeliveryUnavailable
              || sentinel2DeliveryUnavailable
              || hlsDeliveryUnavailable
              || (useUnifiedNisarSheet && unifiedDeliveryChoice === "aoi_subset" && nisarPolarizations.length === 0)
            }
            onClick={() => void handleConfirm()}
            className="min-w-[120px] justify-center"
          >
            {showSubmittingSpinner && <Loader2 className="h-4 w-4 shrink-0 animate-spin" />}
            <span>{t("panels.downloadDestination.createTask")}</span>
          </Button>
        </div>
      </div>
    </ModalOverlay>
  );
}
