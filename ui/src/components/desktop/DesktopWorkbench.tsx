import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { cn } from "@/lib/utils";
import { getCurrentLocale, translate as translateMessage, useI18n } from "@/i18n";

function dispatchPendingAccountCardFocus(accountId?: string) {
  if (accountId && typeof window !== "undefined") {
    try {
      window.sessionStorage?.setItem("eostage:pending-focus-account", accountId);
    } catch {
      // ignore storage errors
    }
    window.dispatchEvent(new window.CustomEvent("eostage:focus-account-card", { detail: { accountId } }));
  }
}
import { DesktopTitlebar } from "./DesktopTitlebar";
import { DesktopWelcomeOnboarding, WELCOME_ONBOARDING_STORAGE_KEY } from "./DesktopWelcomeOnboarding";
import {
  DesktopGuidedTour,
  GuidedTourPromptBanner,
  GUIDED_TOUR_STORAGE_KEY,
  computeGuidedTourCatalogSummary,
  type GuidedTourStepConfig,
} from "./DesktopGuidedTour";
import { recordSubsetSizeEstimate } from "@/lib/subsetSizeEstimateRegistry";
import {
  estimateXyzTilesForBbox,
  getXyzSourceMaxZoom,
  normalizeXyzZooms,
} from "@/lib/xyzTileEstimator";
import {
  type WaybackPeriodSelection,
  formatWaybackSubfolderName,
  isValidWaybackVersionId,
  joinOutputSubdir,
  scaleWaybackEstimateForPeriods,
} from "@/lib/waybackClient";
import { DesktopActivityRail, SHELL_FONT_SIZES, MIN_SHELL_FONT_SIZE, MAX_SHELL_FONT_SIZE, DEFAULT_SHELL_FONT_SIZE } from "./DesktopActivityRail";
import {
  SEARCH_FILTER_SCHEMA_CATALOG,
  initialSearchFilterDraft,
  type SearchFilterDraft,
} from "@/lib/searchFilterSchema";
import {
  toolResultLine,
  orbitDownloadResultLine,
  demStartResultLine,
} from "@/shell/api/manualConditionsLogic";
import {
  selectableIds,
  createTaskErrorText,
  createTaskOutcomeNotice,
  togglePageSelection,
  toggleRowInAllMatch,
  togglePageInAllMatch,
  type TaskOutcomeNotice,
} from "@/shell/api/resultListLogic";
import {
  isGacosWebformTask,
} from "@/shell/api/taskLogic";
import {
  parseShellRoute,
  resolveInitialRoute,
  initialHashToWrite,
  routeMeta,
  readLastRoute,
  writeLastRoute,
  shellStorage,
  type ShellRoute,
} from "@/shell/api/shellRoutes";
import { DesktopAgentDrawer, type AgentSpatialArtifact } from "./DesktopAgentDrawer";
import { DesktopAgentWorkbench, stopAllFixtureFactory } from "./DesktopAgentWorkbench";
import { subscribeDesktopHostEvent } from "@/lib/desktopHostEvents";
import { useAgentSessionShell } from "./agentSessionHooks";
import { createLatestResultGate } from "@/shell/api/latestResultGate";
import { AgentHistoryPage } from "./AgentHistoryPage";
import { ManualWorkbench } from "./ManualWorkbench";
import {
  getWorkbenchToolOptions,
  updateWorkbenchToolOptions,
  subscribeWorkbenchToolOptions,
  getActiveDatasetProvider,
  setActiveDatasetProvider,
  isMultiProviderDataset,
  MULTI_PROVIDER_CONFIGS,
  parentDirFromPath,
  resolveEffectiveDemSourceDatum,
  type WorkbenchToolOptionsSnapshot,
} from "./ManualConditionsPanel";
import { DataSourceCatalogModal } from "./DataSourceCatalogModal";
import {
  canonicalSourceDisplayName,
  CATALOG_METADATA_REGISTRY,
  localizedSourceDisplayName,
} from "@/lib/dataSourceCatalogLogic";
import { filterGeePublicCatalog, isStaticGeeAssetId, type GeeCatalogTheme } from "@/lib/geePublicCatalogIndex";
import {
  extractGeodataRasterOverlayFromResponse,
  type GeodataRasterOverlaySpec,
} from "@/components/WorkbenchMap";
import {
  LandsatDownloadConfirmation,
  isMpcLandsatScene,
  type LandsatDownloadRequest,
} from "./LandsatDownloadConfirmation";
import {
  isTemporarySystemDownloadPath,
  nonTemporaryOutputDir,
  TEMPORARY_DOWNLOAD_PATH_ERROR_ZH,
} from "@/lib/storagePathValidation";
import {
  filterBatchCollectionByAoi,
  findAgentSceneBatchOwner,
  getAgentSessionWorkspaceState,
  peekAgentSessionWorkspaceState,
  projectAgentSessionBatchCollection,
  resolveAgentSessionSourceLabel,
  sceneIntersectsAoiBbox,
  setAgentSessionBatchFilter,
  setAgentSessionWorkspaceState,
  upsertAgentSessionBatch,
  type AgentSessionBatchEntry,
  type AgentSessionWorkspaceState,
} from "@/shell/api/agentSessionWorkspace";
import {
  isSentinel1OrbitApplicable,
  getPreciseOrbitPreference,
  setPreciseOrbitPreference,
  getOrbitPreferenceSourceKey,
} from "@/lib/preciseOrbitPreferences";
import {
  type CompanionState,
  getDefaultPreciseOrbit,
  getSavedCompanionOrbitPreference,
  setSavedCompanionOrbitPreference,
  getSavedCompanionDemPreference,
  setSavedCompanionDemPreference,
  getSavedCompanionGacosPreference,
  setSavedCompanionGacosPreference,
  getSavedCompanionExtentMode,
  isSentinel1Resource,
  resolveAuxiliaryExtent,
} from "@/lib/companionDownloadLogic";
import {
  executeCompanionGacosRequest,
  executeStandaloneGacosRequest,
  parseGacosManualDatesList,
  splitGacosDatesIntoBatches,
  type GacosBatchSubmitState,
} from "@/lib/companionGacosFlow";
import { DownloadDestinationConfirmation } from "@/components/DownloadDestinationConfirmation";
import { createDownloadAcquisitionDraft, type DownloadAcquisitionDraft } from "@/lib/downloadDestinationConfirmation";
import { DesktopBhoonidhiAccountPanel, DesktopCdseAccountPanel, DesktopEarthdataAccountPanel, DesktopUsgsAccountPanel } from "./DesktopAccountPanels";
import { DesktopDownloadDrawer, type DesktopPendingDownloadPreparation, type DownloadTaskItem, type TaskOutputRef } from "./DesktopDownloadDrawer";
import { GlobalCommandPalette } from "./GlobalCommandPalette";
import { subscribeSessionsChanged } from "./useSessionOrganizer";
import { defaultUnifiedClient } from "@/shell/api/unifiedClient";
import type { ExistingDownloadCheckItem, ExistingDownloadMatch } from "@/shell/api/types";
import { DownloadCenterPage } from "./DownloadCenterPage";
import { PluginsAndSkillsPage } from "./PluginsAndSkillsPage";
import {
  deriveTaskEvents,
  readNotificationState,
  writeNotificationState,
  mergeNotificationEvents,
  markRead,
  markAllRead,
  clearReadNotifications,
  type NotificationItem,
  type NotificationState,
} from "@/shell/api/notificationBell";
import {
  SettingsModal,
  readLastSettingsCategory,
  rememberSettingsCategory,
  type SettingsCategory,
} from "@/components/SettingsModal";
import { normalizeSettingsCategory } from "@/shell/api/settingsEntries";
import { InplaceLoginCard } from "@/shell/components/InplaceLoginCard";
import { StatusStateCard } from "@/shell/components/StatusStateCard";
import { EmptyState } from "@/components/ui/EmptyState";
import { EmptyStateWireframe } from "@/components/ui/EmptyStateVisuals";
import { StorageSettingsTab } from "@/shell/components/settings/StorageSettingsTab";
import { AdminBoundaryModal } from "./AdminBoundaryModal";
import { EOStageLoginModal } from "./EOStageLoginModal";
import { EOStageBillingModal, type BillingModalTab } from "./EOStageBillingModal";
import {
  eostageAccountLogout,
  eostageAccountStatus,
  eostageAccountSyncModels,
  notifyEOStageAccountChanged,
  subscribeEOStageAccountChanged,
  type EOStageAccountStatus,
} from "@/shell/api/eostageAccount";
import { saveAoiSearchHistory } from "@/lib/aoiBookmarksAndMerge";
import { explainActionError, preferCodeExplanation } from "@/shell/api/errorExplanations";
import {
  resolveAccountActionFromError,
  resolveOpticalTaskCreationError,
} from "@/shell/api/opticalCreateError";
import {
  stopAllAvailability,
  stopAllNotification,
  shouldAddStopAllStreamEvent,
} from "@/shell/api/stopAllLogic";
import { taskListSignature } from "@/shell/api/taskListSignature";
import { subscribeChannelsChanged, notifyChannelsChanged } from "@/shell/api/agentChannelsEvents";
import { getDesktopShellBridge } from "@/lib/desktopShellBridge";
import type {
  AgentSessionSummary,
  AgentStopAllResult,
  AgentActivityStatus,
  AgentWorkspace,
  UnifiedSearchRequest,
} from "@/lib/generated/v3-contracts";
import { formatBridgeErrorValue } from "@/lib/bridgeErrorFormat";
import {
  listSources,
  getSourceViews,
  searchStart,
  getSearchProgress,
  getBatchPage,
  cancelCatalogSearch,
  createTask,
  previewAllMatchTask,
  createAllMatchTask,
  taskAction,
  createStartupEffect,
  createTaskRowSyncer,
  createTaskOverviewSource,
  taskSnapshotChanged,
  createTaskActionRunner,
  resolveTaskActionErrorText,
  resolveAgentRailNavigation,
  pickAgentSearchedView,
  getFixtureWorkingAoi,
  getFixtureStorageRoot,
  type SourceDescriptor,
  type SourceView,
  type SearchProgress,
  type RowTaskState,
  type RowAction,
  type TypedError,
  type AccountId,
  isFixtureBackend,
  getTasks,
  type ApplicationTask,
  type BatchPageResult,
  getModelChannelReadiness,
  type ModelChannelReadiness,
} from "@/shell/api";
import { isSourceSearchOnly } from "@/shell/api/sourceConditions";
import { UnifiedPoller } from "@/shell/api/poller";
import {
  deliveryFromPage,
  deliveryFromProgress,
  loadingBatchOf,
  RESULT_PAGE_SIZE,
  type BatchDelivery,
} from "@/shell/api/batchDeliveryLogic";
import {
  WorkbenchMap,
  type MapLayerKey,
  type WorkbenchDrawMode,
} from "@/components/WorkbenchMap";
import { ORBIT_LEGEND_LABELS_EN } from "@/shell/api/orbitTerms";
import {
  getTransferEnginePrefs,
  loadTransferEnginePrefsFromDesktop,
} from "@/lib/transferEnginePrefs";
import {
  getWorkingAoi,
  setWorkingAoi,
  clearWorkingAoi,
  getDownloadStatus,
  pauseAsfDownload,
  resumeAsfDownload,
  getCredentialStatus,
  pickDirectory,
  planDemDownloadBbox,
  startDemDownload,
  startDemDownloadBbox,
  matchOrbitsDirectory,
  startOrbitDownloadSnapshot,
  runLocalDemConversion,
  getSarDownloadPolicySettings,
  saveSarDownloadPolicySettings,
  getDemTaskSettings,
  saveDemTaskSettings,
  getNetworkSettings,
  saveNetworkSettings,
  getCacheSummary,
  getRuntimeComponents,
  openApplicationTaskOutput,
  queryOpenGeodata,
  isTauriDesktopHost,
  getNisarRemoteSubsetCapability,
  getApplicationEffectiveCapability,
  preflightNisarSubsetBatchV2,
  prepareNisarSubsetBatchV2,
  confirmNisarSubsetBatchTaskV2,
  startNewlyConfirmedTask,
  projectApplicationCoreSnapshot,
  searchAdminBoundaries,
  resolveAdminBoundary,
  pickOpenFile,
  previewAoiFile,
  formatBridgeError,
  resizeNativeWindowFromEdge,
  getCdseSessionStatus,
  HLS_L30_SUPPORTED_BANDS,
  HLS_S30_SUPPORTED_BANDS,
  startHlsBandBatchDownload,
  startSentinel2BandDownload,
  startSentinel2BandBatchDownload,
  startSentinel2L1cBandBatchDownload,
  startSentinel2ArchiveDownload,
  startSentinel2ArchiveBatchDownload,
  startLandsatAssetBatchDownload,
  startLandsatBundleBatchDownload,
  startLandsatAssetDownload,
  startLandsatBundleDownload,
  getDesktopOnboardingState,
  setDesktopOnboardingCompleted,
  type SarDownloadPolicySettings,
  type Bbox,
  type SceneRow,
  type Json,
  type AdminBoundary,
} from "@/lib/bridge";
import { SENTINEL2_L1C_BANDS, SENTINEL2_REFLECTANCE_BANDS } from "@/lib/sentinel2BandSelection";
import { ensureCdseCredentialSession, hasSavedProviderLogin } from "@/lib/providerCredentialSessions";
import { observeOpticalPreparation } from "@/lib/opticalPreparation";

function humanPreparationTitle(resourceId: string): string {
  const trimmed = (resourceId || "").trim();
  if (!trimmed) return "数据下载任务";
  const fromCatalog = CATALOG_METADATA_REGISTRY[trimmed]?.name;
  if (fromCatalog) return fromCatalog;
  return canonicalSourceDisplayName(trimmed);
}

type OpticalConfirmationDelivery = {
  kind: "sentinel2_l2a" | "sentinel2_l1c" | "hls";
  deliveryForm: "safe_archive" | "analysis_bands";
  bandKeys: string[];
};

const OPTICAL_BAND_SELECTABLE_RESOURCES = new Set([
  "science.sentinel-2.l2a.v1",
  "science.sentinel-2.l1c.v1",
  "science.hls.l30.v2",
  "science.hls.s30.v2",
]);

function isBandSelectableOpticalResource(resourceId: string): boolean {
  return OPTICAL_BAND_SELECTABLE_RESOURCES.has(resourceId);
}

async function resolveCdseSafeAvailability(): Promise<boolean> {
  try {
    const [creds, session] = await Promise.all([
      getCredentialStatus(),
      getCdseSessionStatus(),
    ]);
    if (session.ok && session.status.authenticated) return true;
    return creds.ok && hasSavedProviderLogin(creds.cdse);
  } catch {
    return false;
  }
}

function parseS1SceneAcquisitionIso(sceneId: string): string {
  const match = /(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})/i.exec(sceneId);
  if (!match) return "";
  return `${match[1]}-${match[2]}-${match[3]}T${match[4]}:${match[5]}:${match[6]}Z`;
}

function buildLocalSlcCandidateScenes(sceneIds: readonly string[]): SceneRow[] {
  return sceneIds
    .map((raw) => raw.trim())
    .filter((id) => /^S1[ABCD][_-]/i.test(id))
    .map((id) => {
      const platform = /^S1([ABCD])/i.exec(id)?.[1]?.toUpperCase() ?? "A";
      return {
        scene_id: id,
        product_id: id,
        product_key: id,
        platform: `S1${platform}`,
        product_type: "SLC",
        beam_mode: "IW",
        polarization: "",
        acquisition_datetime: parseS1SceneAcquisitionIso(id),
        absolute_orbit: 0,
        relative_orbit: 0,
        path: 0,
        frame: 0,
        orbit_direction: "",
        has_url: false,
      };
    });
}

function isUpstreamDemError(input: unknown): boolean {
  const raw = typeof input === "string"
    ? input
    : JSON.stringify(input ?? "");
  const lower = raw.toLowerCase();
  if (
    lower.includes("invalid_request")
    || lower.includes("storage")
    || lower.includes("output")
    || lower.includes("aoi")
    || raw.includes("目录")
    || raw.includes("路径")
    || raw.includes("范围")
    || raw.includes("参数")
    || raw.includes("为空")
  ) {
    return false;
  }
  return true;
}
import { thumbnailUrlFor } from "@/shell/api/thumbnailLogic";
import { NisarSubsetWorkspace } from "@/components/NisarSubsetWorkspace";
import { NisarSubsetConfirmation } from "@/components/NisarSubsetConfirmation";
import { NisarMultiLevelSubsetModal } from "@/components/desktop/NisarMultiLevelSubsetModal";
import { OpticalAoiClipModal } from "@/components/desktop/OpticalAoiClipModal";
import {
  precheckOpticalAoiCoverage,
  startOpticalDerivedClipOrMosaicTask,
  openPath,
} from "@/lib/bridge";
import {
  classifyOpticalDerivedScene,
  resolveOpticalDerivedSelection,
  type OpticalAoiSceneLike,
} from "@/lib/opticalAoiClipLogic";
import {
  availableNisarSubsetSelections,
  isNisarRslcProduct,
} from "@/lib/nisarSubset";
import {
  createNisarSubsetWorkspaceDraft,
  type NisarSubsetWorkspaceDraft,
  type NisarSubsetWorkspaceRow,
} from "@/lib/nisarSubsetWorkspace";
import { canonicalAdministrativeAoiGeometry } from "@/lib/administrativeAoiGeometry";
import type {
  DesktopEffectiveCapabilityStatus,
  DesktopJsonObject,
  DesktopNisarConfirmSubsetBatchTaskV2Request,
  DesktopNisarPreparedSubsetBatchV2,
  DesktopNisarRemoteSubsetCapability,
} from "@/lib/desktopShellBridge";
import {
  emitTopToast,
  TopCenterToastHost,
} from "@/components/ui/TopCenterToast";
import { ProgressNotificationHost } from "@/components/ui/ProgressNotification";
import { useDelayedFlag } from "@/hooks/useLoadingPrimitives";
import { Loader2, KeyRound,
  X,
  ExternalLink,
  PanelLeftOpen,
  AlertCircle,
  AlertTriangle,
  MessageSquare,
  History,
  Map as MapIcon,
  Download as DownloadIcon,
  Settings as SettingsIcon,
  Blocks,
  Layers,
} from "lucide-react";
import {
  buildUnifiedSearchRequest,
  buildStorageSavePayload,
  formatStorageSaveError,
} from "@/shell/api/searchRequest";
import {
  sarFilterProfile,
  cleanStaleFilterValues,
  isNisarLogicalResourceId,
  isNisarSBandResourceId,
  resolveNisarProductLevel,
} from "@/shell/api/sarFilterProfile";
import { mergeRowStates } from "@/shell/api/rowStateSync";
import { withCreatePending, withoutCreatePending } from "@/shell/api/creatingFeedback";
import {
  clearUnpinnedGeeMapLayers,
  geeCloudAnalyze,
  geeChunkedDownload,
  pushGeeArtifactToMap,
  type GeeArtifactRecord,
} from "@/lib/geeCloudSuiteClient";
import type { GeeSyncedResultsPayload } from "@/components/desktop/GeeCatalogAutocomplete";

export type ScientificMode = "sar" | "dem" | "orbit" | "convert" | "gee";

export interface DesktopWorkbenchProps {
  dark: boolean;
  onToggleDark: () => void;
}

const SIDEBAR_EXPANDED_KEY = "eostage.shell.sidebarExpanded";
const SIDEBAR_WIDTH_KEY = "eostage.shell.sidebarWidth";
const SHELL_FONT_SIZE_KEY = "eostage.shell.fontSize";

const DEM_RESOURCE_ID_TO_CODE: Record<string, string> = {
  "dem.cop30.v1": "COP30",
  "dem.cop90.v1": "COP90",
  "dem.nasadem.v1": "NASADEM",
  "dem.aw3d30.v1": "AW3D30",
  "dem.aw3d30-ellipsoidal.v1": "AW3D30_E",
  "dem.nisar-dem.v1": "NISAR_DEM",
  "dem.fabdem.v1": "FABDEM",
  "dem.gem15-dtm.v1": "GEM15_DTM",
  "dem.gem15-dsm.v1": "GEM15_DSM",
  "dem.srtm-gl1.v1": "SRTM_GL1",
  "dem.srtm-gl1-ellipsoidal.v1": "SRTM_GL1_E",
  "dem.srtm-gl3.v1": "SRTM_GL3",
  "dem.aster-gdem.v3": "ASTER_GDEM_V3",
};

const DEM_CODE_TO_RESOURCE_ID: Record<string, string> = Object.fromEntries(
  Object.entries(DEM_RESOURCE_ID_TO_CODE).map(([resId, code]) => [code, resId]),
);

const OPEN_GEODATA_RESOURCE_TO_DATASET: Record<string, string> = {
  "science.esa.worldcover.v1": "esa_worldcover",
  "science.jrc.gsw.v1": "jrc_gsw",
  "science.alos.palsar-mosaic.v1": "alos_palsar_mosaic",
  "science.modis.ndvi.v1": "modis_ndvi",
  "atmospheric.sentinel-5p.l2.v1": "sentinel_5p",
  "science.opera.disp-s1.v1": "opera_disp_s1",
  "science.aria.s1-gunw.v1": "aria_s1_gunw",
  "science.comet.licsar.v1": "comet_licsar",
  "event.usgs.earthquake.v1": "usgs_earthquake",
  "geodesy.ngl.gnss.v1": "ngl_gnss",
  "orbit.nisar.oe.v1": "nisar_orbit_oe",
  "orbit.copernicus.pod.v1": "copernicus_pod_orbit",
  "atmospheric.era5.reanalysis.v1": "open_meteo_era5",
  "ionospheric.igs.gim.v1": "ionospheric_ionex",
  "science.gee.catalog.v1": "gee_catalog",
  "science.whu.clcd.v1": "whu_clcd",
  "science.tum.eot20.v1": "tum_eot20",
  "science.gebco.bathymetry-2026.v1": "gebco_2026",
  "science.noaa.etopo-2022.v1": "noaa_etopo_2022",
  "science.nasa.black-marble.v1": "nasa_black_marble",
  "science.nasa.emit.l2a.v1": "nasa_emit",
  "science.nasa.ecostress.l2t.v1": "nasa_ecostress",
  "science.dlr.enmap.l2a.v1": "dlr_enmap",
  "science.aws.sentinel-1-grd.v1": "aws_sentinel1_grd",
  "science.noaa.aws-weather.v1": "noaa_aws_weather",
  "science.overture.maps.v1": "overture_maps",
};

type DesktopResizeEdge = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";

const DESKTOP_RESIZE_HANDLES: { edge: DesktopResizeEdge; className: string }[] = [
  { edge: "n", className: "left-3 right-3 top-0 h-1.5 cursor-n-resize" },
  { edge: "s", className: "bottom-0 left-3 right-3 h-1.5 cursor-s-resize" },
  { edge: "w", className: "bottom-3 left-0 top-3 w-1.5 cursor-w-resize" },
  { edge: "e", className: "bottom-3 right-0 top-3 w-1.5 cursor-e-resize" },
  { edge: "nw", className: "left-0 top-0 h-3 w-3 cursor-nw-resize" },
  { edge: "ne", className: "right-0 top-0 h-3 w-3 cursor-ne-resize" },
  { edge: "sw", className: "bottom-0 left-0 h-3 w-3 cursor-sw-resize" },
  { edge: "se", className: "bottom-0 right-0 h-3 w-3 cursor-se-resize" },
];

function DesktopWindowResizeHandles() {
  return (
    <>
      {DESKTOP_RESIZE_HANDLES.map((handle) => (
        <div
          key={handle.edge}
          data-testid={`window-resize-handle-${handle.edge}`}
          data-no-drag
          className={cn("absolute z-[1800] bg-transparent", handle.className)}
          onPointerDown={(event) => {
            if (event.button !== 0) return;
            event.preventDefault();
            event.stopPropagation();
            void resizeNativeWindowFromEdge(handle.edge, 0, 0, 0, 0).catch(() => {});
          }}
        />
      ))}
    </>
  );
}

function bboxToGeoJsonPolygon(b: Bbox): Json {
  return {
    type: "Polygon",
    coordinates: [[
      [b.west, b.south],
      [b.east, b.south],
      [b.east, b.north],
      [b.west, b.north],
      [b.west, b.south],
    ]],
  };
}

function mapOpenGeodataResponseToScenes(
  resourceId: string,
  res: any,
  fallbackBbox: Bbox,
  fallbackDate: string,
): SceneRow[] {
  const displayTitle = localizedSourceDisplayName(resourceId, getCurrentLocale());
  const rawItems: any[] = Array.isArray(res?.items) ? res.items : [];
  if (rawItems.length > 0) {
    return rawItems.map((item: any, idx: number) => {
      const idStr = String(
        item.id || item.code || item.frame_id || item.title || `${resourceId}_${idx + 1}`,
      );
      let itemBbox: Bbox = fallbackBbox;
      if (Array.isArray(item.bbox) && item.bbox.length >= 4) {
        const w = Number(item.bbox[0]);
        const s = Number(item.bbox[1]);
        const e = Number(item.bbox[2]);
        const n = Number(item.bbox[3]);
        if (Number.isFinite(w) && Number.isFinite(s) && Number.isFinite(e) && Number.isFinite(n)) {
          itemBbox = { west: w, south: s, east: e, north: n };
        }
      } else if (typeof item.lon === "number" && typeof item.lat === "number") {
        const d = 0.15;
        itemBbox = {
          west: item.lon - d,
          south: item.lat - d,
          east: item.lon + d,
          north: item.lat + d,
        };
      }
      const geom: Json =
        item.geometry && typeof item.geometry === "object"
          ? (item.geometry as Json)
          : bboxToGeoJsonPolygon(itemBbox);
      const dtStr = String(item.datetime || item.end_date || fallbackDate || new Date().toISOString());
      const rawThumb = String(
        item.preview_data_url ||
          (idx === 0 && typeof res?.preview_data_url === "string" ? res.preview_data_url : "") ||
          item.thumbnail_url ||
          item.preview_url ||
          item.browse_url ||
          "",
      ).trim();
      const assets: SceneRow["assets"] = rawThumb
        ? [
            {
              role: "thumbnail",
              url: rawThumb,
              file_name: `${idStr}.png`,
              media_type: rawThumb.startsWith("data:image/jpeg") ? "image/jpeg" : "image/png",
              size_bytes: null,
            },
          ]
        : [];
      return {
        product_key: idStr,
        product_id: idStr,
        provider: String(res?.provider || res?.collection || displayTitle),
        dataset: resourceId,
        file_name: String(item.title || idStr),
        scene_id: idStr,
        platform: String(item.platform || displayTitle),
        product_type: String(item.product_type || item.science_level || item.asset_key || (typeof item.magnitude === "number" ? `Mw ${item.magnitude.toFixed(1)}` : "")),
        beam_mode: String(item.beam_mode || item.science_level || ""),
        polarization: String(item.polarization || ""),
        acquisition_datetime: dtStr,
        absolute_orbit: null,
        relative_orbit:
          typeof item.relative_orbit === "number"
            ? item.relative_orbit
            : typeof item.track === "number"
              ? item.track
              : null,
        path: typeof item.path === "number" ? item.path : null,
        frame: typeof item.frame === "number" ? item.frame : null,
        file_size_remote:
          typeof item.file_size_bytes === "number"
            ? item.file_size_bytes
            : typeof item.size_bytes === "number"
              ? item.size_bytes
              : null,
        orbit_direction: (() => {
          const explicit = String(item.flight_direction || item.orbit_direction || "").trim().toUpperCase();
          if (explicit === "ASCENDING" || explicit === "ASC") return "ASCENDING";
          if (explicit === "DESCENDING" || explicit === "DESC") return "DESCENDING";
          const upperId = idStr.toUpperCase();
          if (upperId.startsWith("NISAR_")) {
            const parts = upperId.split("_");
            if (parts[6] === "D") return "DESCENDING";
            if (parts[6] === "A") return "ASCENDING";
          }
          if (upperId.startsWith("S1-GUNW-D-")) return "DESCENDING";
          if (upperId.startsWith("S1-GUNW-A-")) return "ASCENDING";
          if (upperId.startsWith("LICSAR_")) {
            const frame = upperId.slice("LICSAR_".length);
            if (/^\d{1,3}D_/.test(frame)) return "DESCENDING";
            if (/^\d{1,3}A_/.test(frame)) return "ASCENDING";
          }
          return "";
        })(),
        footprint_bbox: itemBbox,
        footprint_geojson: geom,
        has_url: true,
        download_url: String(
          item.download_url || item.signed_href || item.href || item.tenv3_url || item.network_png_url || item.preview_url || res?.downloaded_file || res?.downloaded_path || res?.output_path || "",
        ),
        assets,
        native_metadata: {
          ...(item && typeof item === "object" ? item : {}),
          already_downloaded: Boolean(item?.already_downloaded),
          local_file_path: typeof item?.local_file_path === "string" ? item?.local_file_path : null,
          preview_url: rawThumb || null,
        },
      };
    });
  }
  if (
    resourceId === "science.aria.s1-gunw.v1" ||
    resourceId === "science.opera.disp-s1.v1" ||
    resourceId === "science.comet.licsar.v1" ||
    resourceId === "event.usgs.earthquake.v1" ||
    resourceId === "science.nasa.black-marble.v1" ||
    resourceId === "science.nasa.emit.l2a.v1" ||
    resourceId === "science.nasa.ecostress.l2t.v1" ||
    resourceId === "science.dlr.enmap.l2a.v1" ||
    resourceId.startsWith("science.gee.") ||
    Array.isArray(res?.items)
  ) {
    return [];
  }
  const singleId = String(
    res?.file_name || res?.collection || `${resourceId.replace(/[^a-zA-Z0-9_-]/g, "_")}_${fallbackDate}`,
  );
  const singleThumb = typeof res?.preview_data_url === "string" ? res.preview_data_url.trim() : "";
  return [
    {
      product_key: singleId,
      product_id: singleId,
      provider: String(res?.provider || displayTitle),
      dataset: resourceId,
      file_name: singleId,
      scene_id: singleId,
      platform: displayTitle,
      product_type: String(res?.analysis_center || res?.export_format || ""),
      beam_mode: "",
      polarization: "",
      acquisition_datetime: String(res?.date || fallbackDate || new Date().toISOString()),
      absolute_orbit: null,
      relative_orbit: null,
      path: null,
      frame: null,
      orbit_direction: "",
      footprint_bbox: fallbackBbox,
      footprint_geojson: bboxToGeoJsonPolygon(fallbackBbox),
      has_url: true,
      download_url: String(
        res?.downloaded_file || res?.downloaded_path || res?.output_path || res?.exported_mosaic_path || res?.mosaic_geotiff_path || res?.mosaic_vrt_path || "",
      ),
      assets: singleThumb
        ? [
            {
              role: "thumbnail",
              url: singleThumb,
              file_name: `${singleId}.png`,
              media_type: "image/png",
              size_bytes: null,
            },
          ]
        : [],
      native_metadata: {
        preview_url: singleThumb || null,
      },
    },
  ];
}

function hasTaskReceipt(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  const receipt = value as Record<string, unknown>;
  return typeof receipt.task_id === "string" && receipt.task_id.trim().length > 0
    && typeof receipt.revision === "number" && Number.isSafeInteger(receipt.revision) && receipt.revision >= 0
    && typeof receipt.created === "boolean"
    && typeof receipt.status === "string"
    && ["queued", "running", "paused", "completed", "failed", "cancelled", "interrupted"].includes(receipt.status)
    && (receipt.warning === undefined || receipt.warning === null || typeof receipt.warning === "string");
}

export function DesktopWorkbench({ dark, onToggleDark }: DesktopWorkbenchProps) {
  const { t: translate, locale } = useI18n();
  // 1. 桌面端外壳路由系统 (W5-1a)
  const [route, setRoute] = useState<ShellRoute>(() => {
    const hash = typeof window !== "undefined" ? window.location.hash : "";
    return resolveInitialRoute(hash, readLastRoute(shellStorage));
  });

  const initialHashSyncedRef = useRef(false);
  useEffect(() => {
    if (typeof window === "undefined" || initialHashSyncedRef.current) return;
    initialHashSyncedRef.current = true;
    const h = initialHashToWrite(window.location.hash, route);
    if (h) window.location.hash = h;
  }, [route]);

  const lastNonSettingsRouteRef = React.useRef<ShellRoute>(
    route === "settings" ? "agent" : route
  );

  useEffect(() => {
    if (route !== "settings") {
      lastNonSettingsRouteRef.current = route;
    }
    writeLastRoute(shellStorage, route);
  }, [route]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const handleHashChange = () => {
      const newHash = window.location.hash;
      const parsed = parseShellRoute(newHash);
      if (parsed) {
        setRoute(parsed);
      }
    };
    window.addEventListener("hashchange", handleHashChange);
    return () => window.removeEventListener("hashchange", handleHashChange);
  }, []);

  const navigate = useCallback((targetRoute: ShellRoute) => {
    if (typeof window !== "undefined") {
      window.location.hash = "#/" + targetRoute;
    }
  }, []);

  const {
    agentOpenSessionId,
    setAgentOpenSessionId,
    agentSessionMemory,
    hasActiveAgentSession,
    setHasActiveAgentSession,
    newChatNonce,
    startNewChat: handleStartNewChat,
    handleActiveSessionChange: handleAgentActiveSessionChange,
  } = useAgentSessionShell(() => navigate("agent"));

  // 宽侧栏展开状态：EOStage Desktop 默认展开 240px 侧栏
  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    try {
      const saved = window.localStorage.getItem(SIDEBAR_WIDTH_KEY);
      if (saved) {
        const num = parseInt(saved, 10);
        if (!Number.isNaN(num) && num >= 200 && num <= 360) return num;
      }
    } catch {
      // 忽略
    }
    return 240;
  });

  const readSavedSidebarExpanded = (): boolean => {
    try {
      const saved = window.localStorage.getItem(SIDEBAR_EXPANDED_KEY);
      if (saved === "0") return false;
      if (saved === "1") return true;
    } catch {
      // ignore
    }
    return true;
  };
  const preferredSidebarExpandedRef = useRef<boolean>(readSavedSidebarExpanded());
  const [sidebarExpanded, setSidebarExpanded] = useState<boolean>(() =>
    route === "workbench" ? false : preferredSidebarExpandedRef.current
  );
  const prevSidebarRouteRef = useRef<ShellRoute>(route);
  const routeRefForSidebar = useRef<ShellRoute>(route);
  routeRefForSidebar.current = route;

  useEffect(() => {
    const prev = prevSidebarRouteRef.current;
    if (prev === route) return;
    prevSidebarRouteRef.current = route;
    if (route === "workbench") {
      setSidebarExpanded(false);
    } else if (route === "settings") {
      // 设置页左栏承载分组名称（通用设置/模型设置等），进入时确保展开避免 52px 切片遮字
      setSidebarExpanded(true);
    } else if (prev === "workbench" || prev === "settings") {
      setSidebarExpanded(preferredSidebarExpandedRef.current);
    }
  }, [route]);

  const toggleSidebarExpanded = useCallback(() => {
    setSidebarExpanded((prev) => {
      const next = !prev;
      if (routeRefForSidebar.current !== "workbench") {
        preferredSidebarExpandedRef.current = next;
        try {
          window.localStorage.setItem(SIDEBAR_EXPANDED_KEY, next ? "1" : "0");
        } catch {
          // 写入失败只影响下次启动时的默认值
        }
      }
      return next;
    });
  }, []);

  // 外壳基准字号：12–22px 数字输入，记住用户选择；Ctrl + = / - / 0 调整
  const [shellFontSize, setShellFontSize] = useState<number>(() => {
    try {
      const saved = Number(window.localStorage.getItem(SHELL_FONT_SIZE_KEY));
      if (Number.isInteger(saved) && saved >= MIN_SHELL_FONT_SIZE && saved <= MAX_SHELL_FONT_SIZE) return saved;
    } catch {
      // 读取失败用默认值
    }
    return DEFAULT_SHELL_FONT_SIZE;
  });
  const changeShellFontSize = useCallback((px: number) => {
    setShellFontSize(px);
    try {
      window.localStorage.setItem(SHELL_FONT_SIZE_KEY, String(px));
    } catch {
      // 写入失败只影响下次启动时的默认值
    }
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      let next: number | null = null;
      if (e.key === "=" || e.key === "+") next = Math.min(MAX_SHELL_FONT_SIZE, shellFontSize + 1);
      else if (e.key === "-") next = Math.max(MIN_SHELL_FONT_SIZE, shellFontSize - 1);
      else if (e.key === "0") next = DEFAULT_SHELL_FONT_SIZE;
      if (next === null) return;
      e.preventDefault();
      changeShellFontSize(next);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shellFontSize, changeShellFontSize]);

  const handleOpenSessionFromNav = useCallback(
    (sessionId: string) => {
      setAgentOpenSessionId(sessionId);
      agentSessionMemory.remember(sessionId);
      setHasActiveAgentSession(true);
      if (import.meta.env.DEV && isFixtureBackend()) {
        setAgentPanelOpen(true);
      }
      navigate("agent");
    },
    [setAgentOpenSessionId, agentSessionMemory, setHasActiveAgentSession, navigate]
  );

  const handleRailNavigate = useCallback(
    (targetRoute: ShellRoute) => {
      if (targetRoute === "settings") {
        if (!settingsCategory) {
          setSettingsCategory(readLastSettingsCategory());
        }
        navigate("settings");
        return;
      }
      if (targetRoute === "agent") {
        const currentPageRoute: ShellRoute =
          route === "settings" ? lastNonSettingsRouteRef.current || "agent" : route;
        const navAction = resolveAgentRailNavigation({
          currentPageRoute,
          rememberedSessionId: agentOpenSessionId ?? agentSessionMemory.recall(),
        });
        if (navAction.kind === "stay") {
          if (route === "settings") {
            navigate("agent");
          }
          return;
        }
        navigate("agent");
        return;
      }
      navigate(targetRoute);
    },
    [route, navigate, agentOpenSessionId, agentSessionMemory]
  );

  const handleCloseSettings = useCallback(() => {
    setSettingsOpen(false);
    const fallback = lastNonSettingsRouteRef.current || "agent";
    navigate(fallback);
  }, [navigate]);

  const [scientificMode, setScientificMode] = useState<ScientificMode>("sar");
  const [drawerCollapsed, setDrawerCollapsed] = useState(false);

  // 切片 5: DEV 门控的 hash 参数直达助手栏状态
  const [assistantHashMode] = useState<"collapsed" | "expanded" | "cmdk">(() => {
    if (typeof window === "undefined" || !import.meta.env.DEV) return "collapsed";
    const h = window.location.hash;
    if (h.includes("assistant=expanded")) return "expanded";
    if (h.includes("assistant=cmdk")) return "cmdk";
    return "collapsed";
  });

  const [assistantNoChannel] = useState<boolean>(() => {
    if (typeof window === "undefined" || !import.meta.env.DEV) return false;
    return window.location.hash.includes("assistant=nochannel");
  });

  const [channelReadiness, setChannelReadiness] = useState<ModelChannelReadiness>({
    ready: true,
    disabled: false,
    reason_zh: "",
  });

  const refreshChannelReadiness = useCallback(
    async (isMounted?: () => boolean) => {
      try {
        const res = await getModelChannelReadiness();
        if (isMounted && !isMounted()) return;
        if (assistantNoChannel) {
          setChannelReadiness({
            ready: false,
            disabled: true,
            reason_zh: translate("mapWorkbench.desktop.noModelChannel"),
          });
        } else {
          setChannelReadiness(res);
        }
      } catch {
        // Ignore transient readiness probe errors during startup or route transitions.
      }
    },
    [assistantNoChannel],
  );

  useEffect(() => {
    let mounted = true;
    void refreshChannelReadiness(() => mounted);
    return () => {
      mounted = false;
    };
  }, [refreshChannelReadiness]);

  useEffect(() => {
    let mounted = true;
    const unsubscribe = subscribeChannelsChanged(() => {
      void refreshChannelReadiness(() => mounted);
    });
    return () => {
      mounted = false;
      unsubscribe();
    };
  }, [refreshChannelReadiness]);


  // 响应式监听窗口尺寸：920 宽或窄视口自动收起，宽视口自动展开
  useEffect(() => {
    const syncCollapse = () => {
      if (typeof window !== "undefined") {
        setDrawerCollapsed(window.innerWidth <= 1024);
      }
    };
    syncCollapse();
    window.addEventListener("resize", syncCollapse);
    return () => window.removeEventListener("resize", syncCollapse);
  }, []);

  // 2. 空间范围 (AOI) 核心状态
  const [showAdminModal, setShowAdminModal] = useState(false);
  const [eostageAccount, setEostageAccount] = useState<EOStageAccountStatus | null>(null);
  const [eostageLoginOpen, setEostageLoginOpen] = useState(false);
  const [eostageBillingState, setEostageBillingState] = useState<{
    open: boolean;
    tab: BillingModalTab;
  }>({ open: false, tab: "balance" });

  const officialModelsAutoSyncedRef = useRef(false);
  const refreshingAccountRef = useRef(false);
  const accountSectionMountedRef = useRef(true);

  useEffect(() => {
    accountSectionMountedRef.current = true;
    let active = true;
    const syncAccount = async (refresh = false) => {
      try {
        const st = await eostageAccountStatus(refresh);
        if (!active) return;
        setEostageAccount(st);
        if (!st.logged_in) {
          officialModelsAutoSyncedRef.current = false;
        } else if (refresh && !officialModelsAutoSyncedRef.current) {
          officialModelsAutoSyncedRef.current = true;
          void eostageAccountSyncModels()
            .then(() => {
              if (active) notifyChannelsChanged();
            })
            .catch(() => {
              /* non-blocking startup model sync */
            });
        }
        if (st.session_expired) {
          notifyChannelsChanged();
        }
      } catch {
        // non-blocking
      }
    };
    void syncAccount(false);
    const timer = window.setTimeout(() => void syncAccount(true), 1200);
    const unsub = subscribeEOStageAccountChanged(() => {
      void syncAccount(true);
    });
    const onOpenLogin = () => setEostageLoginOpen(true);
    const onOpenBilling = (ev: Event) => {
      const detail = (ev as CustomEvent<{ tab?: BillingModalTab }>).detail;
      setEostageBillingState({ open: true, tab: detail?.tab ?? "balance" });
    };
    window.addEventListener("eostage:open-eostage-login", onOpenLogin);
    window.addEventListener("eostage:open-eostage-billing", onOpenBilling);
    return () => {
      active = false;
      accountSectionMountedRef.current = false;
      window.clearTimeout(timer);
      unsub();
      window.removeEventListener("eostage:open-eostage-login", onOpenLogin);
      window.removeEventListener("eostage:open-eostage-billing", onOpenBilling);
    };
  }, []);
  const [workingAoi, setWorkingAoiState] = useState<Bbox | null>(null);
  const [workingGeometry, setWorkingGeometry] = useState<Record<string, unknown> | null>(null);
  const [aoiLabel, setAoiLabel] = useState<string>(() => translate("mapWorkbench.desktop.globalView"));
  const [drawMode, setDrawMode] = useState<WorkbenchDrawMode>("rect");
  const [drawActive, setDrawActive] = useState<boolean>(false);
  const [mapLayerKey, setMapLayerKey] = useState<MapLayerKey>("arcgisSatellite");
  const [taskWarning, setTaskWarning] = useState<string | null>(null);
  const [agentPanelOpen, setAgentPanelOpen] = useState<boolean>(() => {
    if (typeof window !== "undefined" && import.meta.env.DEV) {
      return window.location.search.includes("panel=open") || window.location.hash.includes("panel=open");
    }
    return false;
  });

  // 窄窗口 (<1040px) 打开 Agent 分栏地图/结果面板时，自动收起左侧栏以免挤压正文或表格；退出分栏/拓宽视口时恢复偏好
  const wasCompactAgentSplitRef = useRef<boolean>(false);
  useEffect(() => {
    if (typeof window === "undefined") return;
    const syncCompactAgentSidebar = () => {
      const compact = route === "agent" && agentPanelOpen && window.innerWidth < 1040;
      if (compact && !wasCompactAgentSplitRef.current) {
        wasCompactAgentSplitRef.current = true;
        setSidebarExpanded(false);
      } else if (!compact && wasCompactAgentSplitRef.current) {
        wasCompactAgentSplitRef.current = false;
        if (route !== "workbench") {
          setSidebarExpanded(route === "settings" ? true : preferredSidebarExpandedRef.current);
        }
      }
    };
    syncCompactAgentSidebar();
    window.addEventListener("resize", syncCompactAgentSidebar);
    return () => window.removeEventListener("resize", syncCompactAgentSidebar);
  }, [route, agentPanelOpen]);

  // 3. C1 契约数据源身份与多源视图 (注册表 ID 作为唯一身份)
  const [activeResourceId, setActiveResourceId] = useState<string>("science.sentinel-1.standard.v1");
  const activeResourceIdRef = useRef(activeResourceId);
  activeResourceIdRef.current = activeResourceId;
  const lastImageResourceIdRef = useRef<string>("science.sentinel-1.standard.v1");
  const [availableSources, setAvailableSources] = useState<SourceDescriptor[]>([]);
  const [sourceViews, setSourceViews] = useState<SourceView[]>([]);
  const sourceViewsRef = useRef<SourceView[]>([]);
  const clearedSourceBatchesRef = useRef<Map<string, string>>(new Map());
  const filterVisibleSourceViews = useCallback((views: SourceView[]): SourceView[] => {
    const cleared = clearedSourceBatchesRef.current;
    if (cleared.size === 0) return views;
    return views.filter((view) => {
      if (!view.batch_id) return true;
      const clearedKey = cleared.get(view.resource_id);
      if (!clearedKey) return true;
      return clearedKey !== `${view.batch_id}:${view.revision ?? 0}`;
    });
  }, []);
  useEffect(() => {
    sourceViewsRef.current = sourceViews;
  }, [sourceViews]);
  const [searchProgress, setSearchProgress] = useState<SearchProgress | null>(null);
  const [rowStates, setRowStates] = useState<Map<string, RowTaskState>>(new Map());
  const [pendingRetryAction, setPendingRetryAction] = useState<
    | { batchId: string; revision: number; itemIds: string[]; outputRoot: string;
        concurrency: number; layout: DownloadAcquisitionDraft["layout"]; scope: "row" | "checked" | "all_matching";
        excludedItemIds?: string[]; selectionDigest?: string; admissibleCount?: number;
        generation: number; includePreciseOrbit?: boolean; includeGacos?: boolean }
    | null
  >(null);
  const [inlineLoginError, setInlineLoginError] = useState<TypedError | null>(() => {
    if (typeof window === "undefined" || !import.meta.env.DEV) return null;
    if (window.location.hash.includes("needs-account") || window.location.hash.includes("inplace-login")) {
      return {
        code: "LOGIN_REQUIRED",
        message_zh: translate("mapWorkbench.desktop.earthdataRequired"),
        retryable: true,
        needs_account: "earthdata",
        needs_user_confirmation: false,
      };
    }
    return null;
  });

  // 4. 科学检索参数
  const [sensor, setSensor] = useState<string>("science.sentinel-1.standard.v1");
  const [productType, setProductType] = useState<string>("SLC");
  const [startDate, setStartDate] = useState<string>(() => {
    const d = new Date();
    d.setMonth(d.getMonth() - 3);
    return d.toISOString().split("T")[0];
  });
  const [endDate, setEndDate] = useState<string>(() => new Date().toISOString().split("T")[0]);
  const agentTimeRange = useMemo(() => ({ start: startDate, end: endDate }), [startDate, endDate]);
  const agentSourceNames = useMemo(
    () =>
      Object.fromEntries(
        availableSources.map((d) => [d.resource_id, localizedSourceDisplayName(d.resource_id, locale) || d.display_name_zh])
      ),
    [availableSources, locale]
  );
  const [orbitDirection, setOrbitDirection] = useState<"all" | "ascending" | "descending">("all");
  const [relativeOrbit, setRelativeOrbit] = useState<string>("");
  const [path, setPath] = useState<string>("");
  const [frame, setFrame] = useState<string>("");
  const [platforms, setPlatforms] = useState<string[]>([]);
  const [polarizations, setPolarizations] = useState<string[]>([]);
  const [beamModes, setBeamModes] = useState<string[]>([]);
  const [lookDirection, setLookDirection] = useState<string | null>(null);
  const [_burstStrategy, setBurstStrategy] = useState<"all" | "primary">("all");

  // 模式专属扩展状态 (A3: 仅保留真实命令使用的状态)
  const [demSource, setDemSource] = useState<string>("COP30");
  const [autoConvertEllipsoid, setAutoConvertEllipsoid] = useState<boolean>(true);
  const [orbitSourceType, setOrbitSourceType] = useState<"current" | "local">("current");
  const [localSlcPath, setLocalSlcPath] = useState<string>("");
  const [localDemPath, setLocalDemPath] = useState<string>("");
  const [inlineToolResult, setInlineToolResult] = useState<{ ok: boolean; message: string } | null>(null);

  // 光学数据源 draft 状态按 source_id 隔离维护（切换恢复，重置清空）
  const [opticalDrafts, setOpticalDrafts] = useState<Record<string, SearchFilterDraft>>({});
  const handleOpticalDraftChange = useCallback((draft: SearchFilterDraft) => {
    setOpticalDrafts((prev) => ({
      ...prev,
      [activeResourceId]: draft,
    }));
  }, [activeResourceId]);

  // 5. 候选场景集
  const [searching, setSearchingState] = useState(false);
  const searchingRef = useRef(false);
  const setSearching = useCallback((next: boolean) => {
    searchingRef.current = next;
    setSearchingState(next);
  }, []);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [failoverBanner, setFailoverBanner] = useState<{
    datasetKey: string;
    message: string;
    options: { provider: string; label: string }[];
  } | null>(null);
  const [geodataRasterOverlay, setGeodataRasterOverlay] =
    useState<GeodataRasterOverlaySpec | null>(null);
  const [waybackPreviewOverlay, setWaybackPreviewOverlay] =
    useState<GeodataRasterOverlaySpec | null>(null);
  const [, setToolOptionsNonce] = useState(0);
  useEffect(() => {
    return subscribeWorkbenchToolOptions(() => {
      setToolOptionsNonce((n) => n + 1);
      if (getWorkbenchToolOptions().xyzTileSource !== "esri_wayback") {
        setWaybackPreviewOverlay(null);
      }
    });
  }, []);
  useEffect(() => {
    if (activeResourceId !== "auxiliary.xyz-tiles.v1") {
      setWaybackPreviewOverlay(null);
    }
  }, [activeResourceId]);
  const [scenes, setScenes] = useState<SceneRow[]>([]);
  const scenesRef = useRef(scenes);
  scenesRef.current = scenes;
  const [batchTotalCount, setBatchTotalCount] = useState<number | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [loadMoreError, setLoadMoreError] = useState<string | null>(null);
  const [sourceViewLoading, setSourceViewLoading] = useState(false);
  const [sourceViewError, setSourceViewError] = useState<string | null>(null);
  const loadMoreGenerationRef = useRef(0);
  const loadMoreInFlightRef = useRef(false);
  const sourceSwitchGenerationRef = useRef(0);
  // #22 B slice 5: the delivery state of the shown batch, and the batch a
  // running search is filling once its first page is readable (it becomes
  // the source view only when it seals).
  const [batchDelivery, setBatchDelivery] = useState<BatchDelivery | null>(null);
  const [loadingBatch, setLoadingBatch] = useState<{ resourceId: string; requestId: string; batchId: string } | null>(null);
  const loadingBatchRef = useRef(loadingBatch);
  loadingBatchRef.current = loadingBatch;
  const [cancellingSearch, setCancellingSearch] = useState(false);
  // Loaded count at which a page read answered "not yet loaded"; the page is
  // read again once more rows have arrived.
  const [pageWaitingAt, setPageWaitingAt] = useState<number | null>(null);
  const pageWaitingAtRef = useRef(pageWaitingAt);
  pageWaitingAtRef.current = pageWaitingAt;
  const [selectedSceneId, setSelectedSceneId] = useState<string | null>(null);
  const [checkedSceneIds, setCheckedSceneIds] = useState<Set<string>>(new Set());
  const [allMatchingSelected, setAllMatchingSelected] = useState<boolean>(false);
  const allMatchingSelectedRef = useRef(allMatchingSelected);
  allMatchingSelectedRef.current = allMatchingSelected;
  const [excludedSceneIds, setExcludedSceneIds] = useState<Set<string>>(new Set());
  // Checked products can leave the loaded prefix as new item IDs arrive.
  // Retain their metadata for the selection summary and delivery route.
  const checkedScenesRef = useRef(new Map<string, SceneRow>());
  const displayedBatchRef = useRef<string | null>(null);
  const nonBatchScenesActiveRef = useRef<boolean>(false);
  const checkedScenes = useMemo(() => {
    const current = new Map(scenes.map((scene) => [scene.scene_id, scene]));
    return Array.from(checkedSceneIds, (id) => current.get(id) ?? checkedScenesRef.current.get(id))
      .filter((scene): scene is SceneRow => scene !== undefined);
  }, [checkedSceneIds, scenes]);
  const [storageRequiredPrompt, setStorageRequiredPrompt] = useState<boolean>(false);
  const pendingStorageResumeRef = useRef<
    null | { type: "row"; itemId: string } | { type: "checked" } | { type: "dem_plan" }
  >(null);

  // 开放科学栅格与 GEE 数据源：检索或切换选中景时同步挂载/更新地图快视图层
  useEffect(() => {
    const isOpticalGranule =
      activeResourceId.startsWith("science.sentinel-2.")
      || activeResourceId.includes("landsat")
      || activeResourceId.startsWith("science.hls.");
    const isOpenGeodataOrGee = Boolean(
      OPEN_GEODATA_RESOURCE_TO_DATASET[activeResourceId] ||
        activeResourceId === "auxiliary.xyz-tiles.v1" ||
        (!isOpticalGranule && getActiveDatasetProvider(activeResourceId) === "gee") ||
        (scientificMode === "dem" &&
          getActiveDatasetProvider(demSource === "COP30" ? "COP30" : activeResourceId) === "gee"),
    );
    if (!isOpenGeodataOrGee || scenes.length === 0) {
      setGeodataRasterOverlay(null);
      return;
    }
    const activeScene =
      (selectedSceneId ? scenes.find((s) => s.scene_id === selectedSceneId) : undefined) ??
      scenes[0];
    if (!activeScene || !activeScene.footprint_bbox) {
      setGeodataRasterOverlay(null);
      return;
    }
    const meta = (activeScene.native_metadata || {}) as Record<string, unknown>;
    const previewUrl =
      thumbnailUrlFor(activeScene) ||
      (typeof meta.preview_url === "string" && meta.preview_url.trim()
        ? meta.preview_url.trim()
        : null);
    if (!previewUrl) {
      setGeodataRasterOverlay(null);
      return;
    }
    setGeodataRasterOverlay((prev) => ({
      id: activeScene.scene_id,
      label: activeScene.file_name || activeScene.scene_id,
      previewDataUrl: previewUrl,
      bbox: activeScene.footprint_bbox,
      tileUrlTemplate: prev?.id === activeScene.scene_id ? (prev.tileUrlTemplate ?? null) : null,
    }));
  }, [activeResourceId, scientificMode, demSource, scenes, selectedSceneId]);

  // 存储提示键盘 Escape 监听与无障碍关闭
  useEffect(() => {
    if (!storageRequiredPrompt) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        pendingStorageResumeRef.current = null;
        setStorageRequiredPrompt(false);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [storageRequiredPrompt]);

  // 6. 下载队列与任务状态中心
  const [downloadDrawerOpen, setDownloadDrawerOpen] = useState(() => {
    if (typeof window === "undefined") return false;
    const h = window.location.hash;
    return (
      h.includes("drawer=download") ||
      h.includes("retry-fallback") ||
      h.includes("clear-history")
    );
  });
  const [drawerFocusTask, setDrawerFocusTask] = useState<{ taskId: string; nonce: number } | null>(null);
  const downloadDrawerOpenRef = React.useRef(downloadDrawerOpen);
  downloadDrawerOpenRef.current = downloadDrawerOpen;
  const [downloadBusy, setDownloadBusy] = useState(false);
  const [downloadConfirmationDraft, setDownloadConfirmationDraft] = useState<DownloadAcquisitionDraft | null>(null);
  const [landsatDownloadRequest, setLandsatDownloadRequest] = useState<LandsatDownloadRequest | null>(null);
  const [confirmedDownloadSettings, setConfirmedDownloadSettings] = useState<{
    batchId: string;
    outputRoot: string;
    concurrency: number;
    layout: DownloadAcquisitionDraft["layout"];
    includePreciseOrbit?: boolean;
    includeDem?: boolean;
    includeGacos?: boolean;
    opticalDelivery?: OpticalConfirmationDelivery;
  } | null>(null);
  const [companionState, setCompanionState] = useState<CompanionState>(() => ({
    preciseOrbit:
      getSavedCompanionOrbitPreference("science.sentinel-1.standard.v1")
      ?? getDefaultPreciseOrbit("science.sentinel-1.standard.v1"),
    dem: getSavedCompanionDemPreference("science.sentinel-1.standard.v1"),
    gacos: getSavedCompanionGacosPreference("science.sentinel-1.standard.v1"),
    extentMode: getSavedCompanionExtentMode("science.sentinel-1.standard.v1"),
  }));
  const [standaloneCompanionBusy, setStandaloneCompanionBusy] = useState(false);
  const companionStateRef = useRef(companionState);
  companionStateRef.current = companionState;
  const [gacosBatchStatuses, setGacosBatchStatuses] = useState<Map<number, GacosBatchSubmitState>>(() => new Map());
  const [selectedGacosBatchIndices, setSelectedGacosBatchIndices] = useState<number[] | null>(null);
  const [gacosSubmitting, setGacosSubmitting] = useState(false);

  useEffect(() => {
    let lastDates = getWorkbenchToolOptions().gacosDatesText;
    return subscribeWorkbenchToolOptions(() => {
      const snap = getWorkbenchToolOptions();
      if (snap.gacosDatesText !== lastDates) {
        lastDates = snap.gacosDatesText;
        setGacosBatchStatuses(new Map());
        setSelectedGacosBatchIndices(null);
      }
    });
  }, []);

  useEffect(() => {
    const savedOrbit = getSavedCompanionOrbitPreference(activeResourceId);
    const savedDem = getSavedCompanionDemPreference(activeResourceId);
    const savedGacos = getSavedCompanionGacosPreference(activeResourceId);
    const savedExtent = getSavedCompanionExtentMode(activeResourceId);
    setCompanionState((prev) => ({
      ...prev,
      preciseOrbit: savedOrbit ?? getDefaultPreciseOrbit(activeResourceId),
      dem: savedDem,
      gacos: savedGacos,
      extentMode: savedExtent,
    }));
  }, [activeResourceId]);

  const handleCompanionStateChange = useCallback(
    (next: CompanionState) => {
      setCompanionState(next);
      const sourceKey = getOrbitPreferenceSourceKey(activeResourceId, productType || "SLC");
      setPreciseOrbitPreference(sourceKey, next.preciseOrbit);
      setSavedCompanionOrbitPreference(activeResourceId, next.preciseOrbit);
      setSavedCompanionDemPreference(activeResourceId, next.dem);
      setSavedCompanionGacosPreference(activeResourceId, next.gacos);
    },
    [activeResourceId, productType]
  );
  // 需求 #2：单向工作台 -> Agent 会话继承与按会话严格隔离地图/结果状态
  const [activeAgentSessionId, setActiveAgentSessionId] = useState<string | null>(
    () => agentOpenSessionId ?? agentSessionMemory.recall() ?? null
  );
  const activeAgentSessionIdRef = useRef<string | null>(activeAgentSessionId);
  activeAgentSessionIdRef.current = activeAgentSessionId;
  const [agentSessionWorkspace, setAgentSessionWorkspace] = useState<AgentSessionWorkspaceState | null>(null);
  const [agentFocusBoundsRequest, setAgentFocusBoundsRequest] = useState<{ bbox: Bbox; nonce: number } | null>(null);
  const agentFocusNonceRef = useRef(0);
  const adminModalCallerRef = useRef<"workbench" | "agent">("workbench");
  const pendingResearchAfterFilterRemoveRef = useRef(false);
  const silentFallbackAttemptsRef = useRef(new Map<string, number>());
  const triggerSearchRef = useRef<
    ((execOpts?: { forceDownload?: boolean; targetItemId?: string; isSilentFallback?: boolean }) => Promise<boolean | void>) | null
  >(null);
  const effectivePageRouteRef = useRef<ShellRoute>("agent");

  // 光学 GRANULE 数据源静默多源回退顺序（用户无感知切换，彻底移除顶部手工切换条与告警横幅）
  const trySilentOpticalFallback = useCallback((resourceId: string): boolean => {
    const chain =
      resourceId === "science.sentinel-2.l2a.v1"
        ? ["mpc", "earth-search", "cdse"]
        : resourceId === "science.sentinel-2.l1c.v1"
          ? ["earth-search", "cdse"]
          : resourceId === "science.landsat.c2-l2.v1"
            ? ["mpc", "earth-search", "usgs"]
            : null;
    if (!chain) return false;
    const current = getActiveDatasetProvider(resourceId);
    const curIdx = chain.indexOf(current);
    const nextIdx = curIdx >= 0 ? curIdx + 1 : 0;
    const attempts = silentFallbackAttemptsRef.current.get(resourceId) ?? 0;
    if (nextIdx >= chain.length || attempts >= chain.length - 1) {
      return false;
    }
    const nextProvider = chain[nextIdx];
    silentFallbackAttemptsRef.current.set(resourceId, attempts + 1);
    setActiveDatasetProvider(resourceId, nextProvider);
    setFailoverBanner(null);
    setSearchError(null);
    window.setTimeout(() => {
      if (activeResourceIdRef.current === resourceId) {
        void triggerSearchRef.current?.({ isSilentFallback: true }).catch(() => {});
      }
    }, 0);
    return true;
  }, []);

  // 启动时静默将 Sentinel-2 L2A / Landsat C2 L2 默认切换至快速免鉴权 MPC，避免误路由到 GEE / CDSE
  useEffect(() => {
    for (const resId of ["science.sentinel-2.l2a.v1", "science.landsat.c2-l2.v1"]) {
      const cur = getActiveDatasetProvider(resId);
      if (cur !== "mpc") {
        setActiveDatasetProvider(resId, "mpc");
      }
    }
  }, []);

  const [opticalAoiClipSession, setOpticalAoiClipSession] = useState<{
    sourceGeneration: number;
    batchId: string;
    revision: number;
    scenes: OpticalAoiSceneLike[];
    outputRoot: string;
    concurrency: number;
    aoiGeometry: Json | Record<string, unknown> | null;
    aoiBbox: Bbox | null;
    aoiLabel: string | null;
  } | null>(null);

  // NISAR AOI 裁剪工作区与确认状态
  const [nisarSubsetCapability, setNisarSubsetCapability] = useState<DesktopNisarRemoteSubsetCapability | null>(null);
  const [nisarEffectiveCapability, setNisarEffectiveCapability] = useState<DesktopEffectiveCapabilityStatus | null>(null);
  const [nisarSubsetOpenBusy, setNisarSubsetOpenBusy] = useState(false);
  const [nisarSubsetPrepareBusy, setNisarSubsetPrepareBusy] = useState(false);
  const [nisarSubsetPrepareError, setNisarSubsetPrepareError] = useState<string | null>(null);
  const [nisarSubsetWorkspaceSession, setNisarSubsetWorkspaceSession] = useState<{
    sourceBatchRevision: number;
    sourceBatchLabel: string;
    currentAoiName: string;
    mapBbox: Bbox | null;
    rows: NisarSubsetWorkspaceRow[];
    draft: NisarSubsetWorkspaceDraft;
  } | null>(null);
  const [nisarSubsetConfirmation, setNisarSubsetConfirmation] = useState<{
    prepared: DesktopNisarPreparedSubsetBatchV2;
    taskRequest: DesktopNisarConfirmSubsetBatchTaskV2Request;
  } | null>(null);
  const [nisarMultiLevelModalSession, setNisarMultiLevelModalSession] = useState<{
    productLevel: string;
    band: "L" | "S";
    scenes: SceneRow[];
    outputRoot: string;
    batchId?: string | null;
    batchRevision?: number;
    rslcSelections?: Array<{ frequency: "A" | "B"; polarizations: string[] }>;
  } | null>(null);

  useEffect(() => {
    if (!isTauriDesktopHost()) return;
    void getNisarRemoteSubsetCapability()
      .then((res) => {
        if (res && res.ok) setNisarSubsetCapability(res.capability);
      })
      .catch(() => {});
    void getApplicationEffectiveCapability("capability.nisar-rslc.remote-subset.v1")
      .then((res) => {
        if (res && res.ok) setNisarEffectiveCapability(res.status);
      })
      .catch(() => {});
  }, []);
  const [tasks, setTasks] = useState<any[]>([]);
  const [c1Tasks, setC1Tasks] = useState<ApplicationTask[]>([]);
  const prevC1TasksRef = React.useRef<ApplicationTask[]>([]);
  const hasBaselinedTasksRef = React.useRef<boolean>(false);
  const taskListSignatureRef = React.useRef<string | null>(null);
  const [notificationState, setNotificationState] = useState<NotificationState>(() => {
    return readNotificationState(shellStorage);
  });
  const [inboxDrawerOpen, setInboxDrawerOpen] = useState(false);

  const readNotificationTaskIds = useMemo(() => {
    const unreadSet = new Set(notificationState.unreadIds);
    const ids = new Set<string>();
    for (const it of notificationState.items) {
      if (it.task_id && !unreadSet.has(it.id)) {
        ids.add(it.task_id);
      }
    }
    return ids;
  }, [notificationState]);

  const handleMarkTaskNotificationsRead = useCallback((taskId: string) => {
    setNotificationState((prev) => {
      let next = prev;
      for (const it of prev.items) {
        if (it.task_id === taskId && next.unreadIds.includes(it.id)) {
          next = markRead(next, it.id);
        }
      }
      if (next !== prev && !isFixtureBackend()) {
        writeNotificationState(shellStorage, next);
      }
      return next;
    });
  }, []);

  const handleNotificationClick = useCallback((item: NotificationItem) => {
    setNotificationState((prev) => {
      const next = markRead(prev, item.id);
      if (!isFixtureBackend()) {
        writeNotificationState(shellStorage, next);
      }
      return next;
    });
    if (item.task_id) {
      setDrawerFocusTask((prev) => ({ taskId: item.task_id!, nonce: (prev?.nonce ?? 0) + 1 }));
    }
    navigate(item.route);
  }, [navigate]);

  const handleMarkAllNotificationsRead = useCallback(() => {
    setNotificationState((prev) => {
      const next = markAllRead(prev);
      if (!isFixtureBackend()) {
        writeNotificationState(shellStorage, next);
      }
      return next;
    });
  }, []);

  const handleClearReadNotifications = useCallback(() => {
    setNotificationState((prev) => {
      const next = clearReadNotifications(prev);
      if (!isFixtureBackend()) {
        writeNotificationState(shellStorage, next);
      }
      return next;
    });
  }, []);

  // Agent 活动运行与任务追踪（跨路由保留）
  const [agentActivity, setAgentActivity] = useState<{
    liveRunIds: string[];
    agentTaskIds: string[];
  }>({
    liveRunIds: [],
    agentTaskIds: [],
  });
  const [agentActivityStatus, setAgentActivityStatus] = useState<AgentActivityStatus | null>(null);

  // 重读触发点多（启动、任务集合变化、activity_changed、运行开始/结束），晚到的旧响应不得覆盖新值
  const activityStatusGate = useMemo(() => createLatestResultGate(), []);
  const refreshAgentActivityStatus = useCallback(async () => {
    if (isFixtureBackend()) return;
    const bridge = getDesktopShellBridge();
    const agent = bridge?.agent;
    if (!agent?.activityStatus) return;
    try {
      await activityStatusGate.run(() => agent.activityStatus(), setAgentActivityStatus);
    } catch {
      // 保持前值，不抛出用户侧错误
    }
  }, [activityStatusGate]);

  // 界面在启动时调用 (C1 规则 12)
  useEffect(() => {
    void refreshAgentActivityStatus();
  }, [refreshAgentActivityStatus]);

  const [stopAllBusy, setStopAllBusy] = useState(false);
  const lastStopAllRef = useRef<{ result: AgentStopAllResult; at: number } | null>(null);

  const stopAllState = useMemo(() => {
    return stopAllAvailability({
      liveRunIds: agentActivity.liveRunIds,
      agentTaskIds: agentActivity.agentTaskIds,
      tasks: c1Tasks,
      status: !isFixtureBackend() ? agentActivityStatus : null,
    });
  }, [agentActivity, c1Tasks, agentActivityStatus, locale]);

  // 单一任务总览订阅源与动作执行器（W5-8 P0-1 / P0-2）
  const taskOverviewSource = useMemo(
    () => createTaskOverviewSource({ fetchTasks: getTasks }),
    []
  );

  const taskActionRunner = useMemo(
    () =>
      createTaskActionRunner({
        taskAction,
        fetchTasks: async () => {
          await taskOverviewSource.refreshNow();
          return taskOverviewSource.getLatest() ?? (await getTasks());
        },
      }),
    [taskOverviewSource]
  );

  const refreshTasksList = useCallback(async () => {
    await taskOverviewSource.refreshNow().catch(() => {});
    void refreshAgentActivityStatus();
  }, [taskOverviewSource, refreshAgentActivityStatus]);

  const handleAgentActivityChange = useCallback(
    (nextActivity: { liveRunIds: string[]; agentTaskIds: string[] }) => {
      setAgentActivity(nextActivity);
      void taskOverviewSource.refreshNow().catch(() => {});
    },
    [taskOverviewSource]
  );

  const handleStopAll = useCallback(async () => {
    if (stopAllBusy || !stopAllState.enabled) return;
    setStopAllBusy(true);

    try {
      let result: AgentStopAllResult;
      if (isFixtureBackend()) {
        result = stopAllFixtureFactory
          ? stopAllFixtureFactory()
          : {
              cancelled_runs: [],
              failed_runs: [],
              cancelled_tasks: [],
              failed_tasks: [],
            };
      } else {
        const bridge = getDesktopShellBridge();
        if (!bridge?.agent) {
          throw new Error(translate("mapWorkbench.desktop.bridgeUnavailable"));
        }
        result = await bridge.agent.stopAll();
      }

      const now = Date.now();
      lastStopAllRef.current = { result, at: now };
      const notifItem = stopAllNotification(result, c1Tasks, now);

      setNotificationState((prev) => {
        const next = mergeNotificationEvents(prev, [notifItem]);
        if (!isFixtureBackend()) {
          writeNotificationState(shellStorage, next);
        }
        return next;
      });

      void refreshTasksList();
    } catch (err) {
      const errText = explainActionError(err, translate("mapWorkbench.desktop.stopAllFailed"));
      const failNotif: NotificationItem = {
        id: `notif-stop-all-fail-${Date.now()}`,
        kind: "failed",
        title: translate("mapWorkbench.desktop.stopAllFailed"),
        occurred_at: Date.now(),
        route: "agent",
        detail: [errText],
      };
      setNotificationState((prev) => {
        const next = mergeNotificationEvents(prev, [failNotif]);
        if (!isFixtureBackend()) {
          writeNotificationState(shellStorage, next);
        }
        return next;
      });
    } finally {
      setStopAllBusy(false);
    }
  }, [stopAllBusy, stopAllState.enabled, c1Tasks, refreshTasksList]);

  const handleStopAllEvent = useCallback(
    (_sessionId: string, _seq: number, payload: AgentStopAllResult) => {
      const now = Date.now();
      if (shouldAddStopAllStreamEvent(payload, lastStopAllRef.current, now)) {
        const notifItem = stopAllNotification(payload, c1Tasks, now, "stream");
        setNotificationState((prev) => {
          const next = mergeNotificationEvents(prev, [notifItem]);
          if (!isFixtureBackend()) {
            writeNotificationState(shellStorage, next);
          }
          return next;
        });
      }
      void taskOverviewSource.refreshNow().catch(() => {});
    },
    [c1Tasks, taskOverviewSource]
  );
  const [creatingItemIds, setCreatingItemIds] = useState<Set<string>>(new Set());
  const [batchCreating, setBatchCreating] = useState<boolean>(false);
  const [taskOutcomeNotice, setTaskOutcomeNotice] = useState<TaskOutcomeNotice | null>(null);
  const [pendingDownloadPreparations, setPendingDownloadPreparations] = useState<DesktopPendingDownloadPreparation[]>([]);
  const batchPageInFlightRef = React.useRef<boolean>(false);
  const rowRefreshGenerationRef = React.useRef(0);
  const pollerRef = React.useRef<UnifiedPoller | null>(null);
  const activeSearchIdRef = React.useRef<string | null>(null);
  const searchRequestsRef = React.useRef(new Map<string, UnifiedSearchRequest>());
  const [batchSearchRequests, setBatchSearchRequests] = React.useState(() => new Map<string, UnifiedSearchRequest>());

  const applyFirstBatchPage = useCallback((page: BatchPageResult) => {
    if (displayedBatchRef.current !== page.batch_id) {
      checkedScenesRef.current.clear();
      setCheckedSceneIds(new Set());
      setAllMatchingSelected(false);
      setExcludedSceneIds(new Set());
      setSelectedSceneId(null);
      setDownloadConfirmationDraft(null);
      setLandsatDownloadRequest(null);
      setOpticalAoiClipSession(null);
      setPendingRetryAction(null);
      setInlineLoginError(null);
      setCreatingItemIds(new Set());
      setDownloadBusy(false);
      setBatchCreating(false);
    }
    nonBatchScenesActiveRef.current = false;
    displayedBatchRef.current = page.batch_id;
    setTaskOutcomeNotice(null);
    setInlineToolResult(null);
    rowRefreshGenerationRef.current += 1;
    loadMoreGenerationRef.current += 1;
    loadMoreInFlightRef.current = false;
    setScenes(page.scenes);
    setBatchTotalCount(page.total_count);
    setLoadingMore(false);
    setLoadMoreError(null);
    setSourceViewLoading(false);
    setSourceViewError(null);
    setPageWaitingAt(null);
    setBatchDelivery(deliveryFromPage(page));
    const nextRows = new Map<string, RowTaskState>();
    for (const row of page.rows) nextRows.set(row.item_id, row);
    setRowStates(nextRows);
  }, []);

  // Populate scenes produced by direct open-geodata or GEE routes without a backend batch.
  const activateNonBatchSceneList = useCallback((nextScenes: SceneRow[]) => {
    nonBatchScenesActiveRef.current = true;
    displayedBatchRef.current = null;
    checkedScenesRef.current.clear();
    setCheckedSceneIds(new Set());
    setAllMatchingSelected(false);
    setExcludedSceneIds(new Set());
    setScenes(nextScenes);
    setBatchTotalCount(nextScenes.length);
    setSelectedSceneId(nextScenes[0]?.scene_id ?? null);
  }, []);

  // Drop the old source immediately, before any request for the next source.
  const beginResultTransition = useCallback(() => {
    sourceSwitchGenerationRef.current += 1;
    rowRefreshGenerationRef.current += 1;
    loadMoreGenerationRef.current += 1;
    loadMoreInFlightRef.current = false;
    nonBatchScenesActiveRef.current = false;
    displayedBatchRef.current = null;
    checkedScenesRef.current.clear();
    setScenes([]);
    setBatchTotalCount(0);
    setBatchDelivery(null);
    setRowStates(new Map());
    setSelectedSceneId(null);
    setCheckedSceneIds(new Set());
    setAllMatchingSelected(false);
    setExcludedSceneIds(new Set());
    setLoadingMore(false);
    setLoadMoreError(null);
    setSourceViewLoading(true);
    setSourceViewError(null);
    setSearchError(null);
    setFailoverBanner(null);
    setGeodataRasterOverlay(null);
    setInlineToolResult(null);
    setTaskOutcomeNotice(null);
    setPageWaitingAt(null);
    setDownloadConfirmationDraft(null);
    setLandsatDownloadRequest(null);
    setOpticalAoiClipSession(null);
    setPendingRetryAction(null);
    setInlineLoginError(null);
    setCreatingItemIds(new Set());
    setDownloadBusy(false);
    setBatchCreating(false);
  }, []);

  const handleClearResults = useCallback(() => {
    const currView = sourceViewsRef.current.find((view) => view.resource_id === activeResourceId);
    if (currView?.batch_id) {
      clearedSourceBatchesRef.current.set(
        activeResourceId,
        `${currView.batch_id}:${currView.revision ?? 0}`,
      );
    }
    beginResultTransition();
    setSourceViewLoading(false);
    const nextViews = sourceViewsRef.current.filter((view) => view.resource_id !== activeResourceId);
    sourceViewsRef.current = nextViews;
    setSourceViews(nextViews);
    setSearchProgress((prev) =>
      prev?.resource_id === activeResourceId ? null : prev,
    );
    if (loadingBatchRef.current?.resourceId === activeResourceId) {
      loadingBatchRef.current = null;
      setLoadingBatch(null);
    }
  }, [activeResourceId, beginResultTransition]);

  // The batch shown for a source: the loading batch of its running search,
  // else its frozen source view.
  const batchFor = useCallback((resourceId: string): { batch_id: string; revision: number } | null => {
    const loading = loadingBatchRef.current;
    if (loading && loading.resourceId === resourceId) {
      return { batch_id: loading.batchId, revision: 0 };
    }
    const view = sourceViewsRef.current.find((candidate) => candidate.resource_id === resourceId);
    return view?.batch_id ? { batch_id: view.batch_id, revision: view.revision ?? 0 } : null;
  }, []);

  // The rendered list is sorted separately from the Store's page order. Read
  // every loaded Store page instead of inferring a page from a visible row.
  const readLoadedRowStates = useCallback(async (batchId: string, revision: number) => {
    const resourceId = activeResourceIdRef.current;
    const initialBatch = batchFor(resourceId);
    if (displayedBatchRef.current !== batchId || initialBatch?.batch_id !== batchId
      || initialBatch.revision !== revision) throw new Error("Batch view changed");
    const generation = ++rowRefreshGenerationRef.current;
    const isCurrent = () => {
      const batch = batchFor(resourceId);
      return generation === rowRefreshGenerationRef.current
        && activeResourceIdRef.current === resourceId
        && displayedBatchRef.current === batchId
        && batch?.batch_id === batchId && batch.revision === revision;
    };
    if (!isCurrent()) throw new Error("Batch view changed");
    const expectedLength = scenesRef.current.length;
    const pageGeneration = loadMoreGenerationRef.current;
    // Store permits up to 250 rows per page. Do not expose the extra rows
    // read for the final partial chunk as already loaded in the workbench.
    const refreshPageSize = 250;
    const pageCount = Math.max(1, Math.ceil(expectedLength / refreshPageSize));
    const readRows: RowTaskState[] = [];
    const readScenes: SceneRow[] = [];
    let total: number | undefined;
    let loading = false;
    for (let index = 0; index < pageCount; index += 1) {
      const page = await getBatchPage(batchId, revision, index, refreshPageSize);
      if (!isCurrent() || (total !== undefined && total !== page.total_count)) {
        throw new Error("Batch view changed during row refresh");
      }
      total = page.total_count;
      if (index === 0) loading = page.state === "loading";
      readScenes.push(...page.scenes);
      readRows.push(...page.rows);
    }
    const refreshedScenes = readScenes.slice(0, expectedLength);
    const rows = readRows.slice(0, expectedLength);
    // New IDs can shift an old visible row beyond the prefix whose task
    // states we just read. Reconcile that prefix with the same snapshot.
    if (loading && !loadMoreInFlightRef.current && pageGeneration === loadMoreGenerationRef.current) {
      setScenes((previous) => {
        if (!isCurrent() || loadMoreInFlightRef.current || pageGeneration !== loadMoreGenerationRef.current
          || previous.length !== expectedLength) return previous;
        if (previous.length === refreshedScenes.length
          && previous.every((scene, index) => scene.scene_id === refreshedScenes[index]?.scene_id)) return previous;
        return refreshedScenes;
      });
    }
    return { rows, isCurrent };
  }, [batchFor]);

  const refreshLoadedRowStates = useCallback(async (batchId: string, revision: number) => {
    const { rows, isCurrent } = await readLoadedRowStates(batchId, revision);
    setRowStates((previous) => isCurrent() ? mergeRowStates(previous, rows).next : previous);
  }, [readLoadedRowStates]);

  // Loading pages are ordered by item ID, so newly arrived items can shift
  // earlier offsets. Re-read the prefix and reject a mixed-count snapshot.
  const readBatchPrefix = useCallback(async (batchId: string, revision: number, wanted: number) => {
    const first = await getBatchPage(batchId, revision, 0, RESULT_PAGE_SIZE);
    const pages = Math.ceil(Math.min(Math.max(wanted, 1), first.total_count) / RESULT_PAGE_SIZE);
    const resultScenes = [...first.scenes];
    const rows = [...first.rows];
    for (let index = 1; index < pages; index += 1) {
      const page = await getBatchPage(batchId, revision, index, RESULT_PAGE_SIZE);
      if (page.total_count !== first.total_count) throw new Error("Batch grew during page read");
      resultScenes.push(...page.scenes);
      rows.push(...page.rows);
    }
    return { first, scenes: resultScenes.slice(0, wanted), rows: rows.slice(0, wanted) };
  }, []);

  // Reconcile the loaded prefix against the final frozen order at the seal.
  const refreshSettledBatch = useCallback(async (batchId: string, revision: number, isCurrent: () => boolean) => {
    if (!isCurrent() || displayedBatchRef.current !== batchId) return;
    const resourceId = activeResourceIdRef.current;
    const generation = ++loadMoreGenerationRef.current;
    loadMoreInFlightRef.current = true;
    setLoadingMore(true);
    try {
      const expectedLength = scenesRef.current.length;
      // Finish the partially loaded page at seal so subsequent frozen-page
      // reads start at a page boundary and do not skip the remaining rows.
      const settledLength = Math.ceil(expectedLength / RESULT_PAGE_SIZE) * RESULT_PAGE_SIZE;
      const result = await readBatchPrefix(batchId, revision, settledLength);
      if (!isCurrent() || generation !== loadMoreGenerationRef.current) return;
      const rowGeneration = ++rowRefreshGenerationRef.current;
      const stillShown = () => isCurrent() && generation === loadMoreGenerationRef.current
        && activeResourceIdRef.current === resourceId && displayedBatchRef.current === batchId;
      setScenes((previous) => stillShown() ? result.scenes : previous);
      setBatchDelivery(deliveryFromPage(result.first));
      setBatchTotalCount(result.first.total_count);
      setPageWaitingAt(null);
      setRowStates((previous) => {
        if (!stillShown() || rowGeneration !== rowRefreshGenerationRef.current) return previous;
        const next = new Map(previous);
        for (const row of result.rows) next.set(row.item_id, row);
        return next;
      });
    } finally {
      if (generation === loadMoreGenerationRef.current) {
        loadMoreInFlightRef.current = false;
        setLoadingMore(false);
      }
    }
  }, [readBatchPrefix]);

  // 任务总览订阅：外壳挂载即订阅，持续 1s 轮询，不随抽屉/页面开合或已知活动任务空闲停
  useEffect(() => {
    let mounted = true;
    const taskRowSyncer = createTaskRowSyncer({
      getActiveBatch: () => {
        const batch = batchFor(activeResourceIdRef.current);
        return batch && displayedBatchRef.current === batch.batch_id
          ? { batchId: batch.batch_id, revision: batch.revision } : null;
      },
      getBatchPage: async (batchId, revision) => {
        const result = await readLoadedRowStates(batchId, revision);
        if (!result.isCurrent()) throw new Error("Batch view changed");
        return { rows: result.rows };
      },
      setRowStates: (updater) => {
        const generation = rowRefreshGenerationRef.current;
        setRowStates((previous) => mounted && generation === rowRefreshGenerationRef.current
          ? updater(previous) : previous);
      },
      mergeRowStates,
      batchPageInFlightRef,
      isMounted: () => mounted,
    });

    const handleTaskSnapshot = (list: ApplicationTask[]) => {
      if (!mounted) return;
      if (
        !hasBaselinedTasksRef.current ||
        taskSnapshotChanged(prevC1TasksRef.current, list)
      ) {
        setC1Tasks(list);
      }
      // 非空活动状态是全部停止的唯一依据，任务集合变化后必须重读 (C1 规则 12)
      const signature = taskListSignature(list);
      if (signature !== taskListSignatureRef.current) {
        taskListSignatureRef.current = signature;
        void refreshAgentActivityStatus();
      }
      if (!hasBaselinedTasksRef.current) {
        hasBaselinedTasksRef.current = true;
        prevC1TasksRef.current = list;
      } else {
        const prevTasks = prevC1TasksRef.current;
        prevC1TasksRef.current = list;
        const newEvents = deriveTaskEvents(prevTasks, list);
        if (newEvents.length > 0) {
          setNotificationState((prev) => {
            const next = mergeNotificationEvents(prev, newEvents);
            if (!isFixtureBackend()) {
              writeNotificationState(shellStorage, next);
            }
            return next;
          });
        }
      }

      void taskRowSyncer.syncOnTaskSnapshot(list, {
        drawerOpen: downloadDrawerOpenRef.current,
      });
    };

    const unsubscribe = taskOverviewSource.subscribe(handleTaskSnapshot);

    return () => {
      mounted = false;
      unsubscribe();
    };
  }, [taskOverviewSource, readLoadedRowStates, batchFor]);

  // 检索进度轮询器：检索 250ms，终态自动停止
  useEffect(() => {
    let mounted = true;
    let polling = false;
    let consecutivePollErrors = 0;
    let trackedErrorReqId: string | null = null;

    const poller = new UnifiedPoller({
      onPollSearch: async () => {
        const reqId = activeSearchIdRef.current;
        if (!reqId || polling) return;
        if (trackedErrorReqId !== reqId) {
          trackedErrorReqId = reqId;
          consecutivePollErrors = 0;
        }
        polling = true;
        const isCurrent = () => mounted && activeSearchIdRef.current === reqId
          && activeResourceIdRef.current === activeResourceId;
        try {
          const p = await getSearchProgress(reqId);
          consecutivePollErrors = 0;
          if (!isCurrent()) return;
          setSearchProgress(p);
          if (p.resource_id === activeResourceIdRef.current && (!p.terminal || p.stage === "done")) setSearchError(null);
          // #22 B slice 5: once page 1 is readable the loading batch is shown
          // and keeps growing; arrived rows can be checked and downloaded.
          const live = loadingBatchOf(p, activeResourceId);
          if (live) {
            setBatchDelivery((previous) => deliveryFromProgress(previous, p));
            if (loadingBatchRef.current?.batchId !== live.batchId) {
              const next = { resourceId: activeResourceId, ...live };
              const page = await getBatchPage(live.batchId, 0, 0, RESULT_PAGE_SIZE);
              if (!isCurrent()) return;
              loadingBatchRef.current = next;
              setLoadingBatch(next);
              applyFirstBatchPage(page);
              setSelectedSceneId(null);
              setCheckedSceneIds(new Set());
              setAllMatchingSelected(false);
              setExcludedSceneIds(new Set());
            } else if (
              pageWaitingAtRef.current !== null
              && (p.loaded_count ?? 0) > pageWaitingAtRef.current
            ) {
              setPageWaitingAt(null);
            }
          }
          if (p.terminal) {
            poller.stopSearch();
            setSearching(false);
            const shownLoading = loadingBatchRef.current;
            const settleShownBatch = async (views: SourceView[]) => {
              sourceViewsRef.current = views;
              const currView = views.find((v) => v.resource_id === p.resource_id);
              if (!currView?.batch_id) return;
              const frozenRequest = searchRequestsRef.current.get(reqId);
              if (frozenRequest && (!p.batch_id || currView.batch_id === p.batch_id)) {
                const targetBatchId = currView.batch_id;
                setBatchSearchRequests((prev) => {
                  const next = new Map(prev);
                  next.set(targetBatchId, frozenRequest);
                  return next;
                });
                searchRequestsRef.current.delete(reqId);
              }
              if (!isCurrent() || activeResourceIdRef.current !== p.resource_id || (p.batch_id && currView.batch_id !== p.batch_id)) return;
              if (shownLoading?.batchId === currView.batch_id) {
                await refreshSettledBatch(currView.batch_id, currView.revision ?? 0,
                  () => isCurrent() && activeResourceIdRef.current === p.resource_id);
                return;
              }
              const page = await getBatchPage(currView.batch_id, currView.revision ?? 0);
              if (!isCurrent() || activeResourceIdRef.current !== p.resource_id) return;
              applyFirstBatchPage(page);
            };
            if (p.stage === "done") {
              const views = filterVisibleSourceViews(await getSourceViews());
              if (!isCurrent()) return;
              sourceViewsRef.current = views;
              setSourceViews(views);
              await settleShownBatch(views);
            } else if (p.stage === "cancelled") {
              // #22 B slice 5: rows kept by the cancel stay, marked possibly incomplete.
              const views = filterVisibleSourceViews(await getSourceViews());
              if (!isCurrent()) return;
              sourceViewsRef.current = views;
              setSourceViews(views);
              if (p.batch_id) await settleShownBatch(views);
            } else if (p.stage === "failed") {
              // The per-code text is more specific than the backend's generic
              // terminal message, so try the code alone first, passing bounded context.
              const byCode = p.error?.code
                ? formatBridgeErrorValue({ code: p.error.code, context: p.error.message_zh })
                : "";
              if (p.resource_id === activeResourceIdRef.current) {
                const isOpticalFailover =
                  p.resource_id === "science.sentinel-2.l2a.v1" ||
                  p.resource_id === "science.sentinel-2.l1c.v1" ||
                  p.resource_id === "science.landsat.c2-l2.v1";
                if (isOpticalFailover && trySilentOpticalFallback(p.resource_id)) {
                  // 静默切换备用光学源重试中，暂不暴露失败横幅
                } else {
                  setSearchError(
                    byCode && byCode !== "[object Object]"
                      ? byCode
                      : p.error?.message_zh || p.message_zh || translate("mapWorkbench.desktop.searchUnsuccessful"),
                  );
                }
              }
              const views = filterVisibleSourceViews(await getSourceViews());
              if (!isCurrent()) return;
              sourceViewsRef.current = views;
              setSourceViews(views);
              if (p.batch_id) await settleShownBatch(views);
            }
            if (isCurrent()) {
              activeSearchIdRef.current = null;
              loadingBatchRef.current = null;
              setLoadingBatch(null);
            }
          }
        } catch (error) {
          if (!isCurrent()) return;
          consecutivePollErrors += 1;
          console.warn("Search progress or result refresh error:", error);
          if (searchRequestsRef.current.get(reqId)?.resource_id === activeResourceIdRef.current) {
            setSearchError(translate("mapWorkbench.desktop.loadMoreFailed"));
          }
          if (consecutivePollErrors >= 5) {
            poller.stopSearch();
            setSearching(false);
            activeSearchIdRef.current = null;
            loadingBatchRef.current = null;
            setLoadingBatch(null);
          } else {
            poller.startSearch(reqId);
          }
        } finally {
          polling = false;
        }
      },
    });

    pollerRef.current = poller;
    if (activeSearchIdRef.current) poller.startSearch(activeSearchIdRef.current);

    return () => {
      mounted = false;
      poller.dispose();
      pollerRef.current = null;
    };
  }, [activeResourceId]);

  const searchRunTokenRef = useRef<number>(0);

  // 7. 偏好设置模态窗
  const [settingsCategory, setSettingsCategoryState] = useState<SettingsCategory | undefined>(() => {
    if (typeof window !== "undefined" && import.meta.env.DEV) {
      const match = window.location.hash.match(/#settings=([a-z_]+)/);
      if (match) {
        return normalizeSettingsCategory(match[1]);
      }
    }
    return readLastSettingsCategory();
  });
  const setSettingsCategory = useCallback((next: SettingsCategory | undefined) => {
    if (next) {
      const normalized = normalizeSettingsCategory(next);
      rememberSettingsCategory(normalized);
      setSettingsCategoryState(normalized);
      return;
    }
    setSettingsCategoryState(undefined);
  }, []);
  const [settingsOpen, setSettingsOpen] = useState(() => {
    if (typeof window === "undefined") return false;
    return window.location.hash.includes("settings");
  });

  useEffect(() => {
    if (!import.meta.env.DEV || typeof window === "undefined") return;
    const handleHashChange = () => {
      const hash = window.location.hash;
      if (hash.includes("settings")) {
        setSettingsOpen(true);
        const match = hash.match(/#settings=([a-z_]+)/);
        if (match) {
          setSettingsCategory(normalizeSettingsCategory(match[1]));
        }
      }
    };
    window.addEventListener("hashchange", handleHashChange);
    return () => window.removeEventListener("hashchange", handleHashChange);
  }, [setSettingsCategory]);

  const prevRouteRef = React.useRef<ShellRoute>(route);
  useEffect(() => {
    if (prevRouteRef.current === "settings" && route !== "settings") {
      setSettingsOpen(false);
    }
    if (route === "settings") {
      setSettingsOpen(true);
    }
    prevRouteRef.current = route;
  }, [route]);
  const [creds, setCreds] = useState<any>(null);
  const [storageRootPath, setStorageRootPath] = useState<string>("");
  const [sarDownloadPolicy, setSarDownloadPolicy] = useState<SarDownloadPolicySettings | null>(null);
  const [, setDemConcurrency] = useState<number>(2);
  const [savingConcurrency, setSavingConcurrency] = useState<boolean>(false);
  const showSavingConcurrency = useDelayedFlag(savingConcurrency);

  // 初始加载：委托给纯注入模块 startupLoad，承载一次性守卫、StrictMode 双挂载容忍、active 丢弃与各加载器错误隔离
  const startupEffect = useMemo(
    () =>
      createStartupEffect({
        listSources,
        getSourceViews,
        getBatchPage: async (batchId, revision, page, pageSize) => {
          const generation = loadMoreGenerationRef.current;
          if (sourceSwitchGenerationRef.current !== 0 || activeResourceIdRef.current !== "science.sentinel-1.standard.v1") {
            throw new Error("Startup batch view changed");
          }
          const result = await getBatchPage(batchId, revision, page, pageSize);
          if (sourceSwitchGenerationRef.current !== 0 || loadMoreGenerationRef.current !== generation
            || activeResourceIdRef.current !== "science.sentinel-1.standard.v1") {
            throw new Error("Startup batch view changed");
          }
          return result;
        },
        getWorkingAoi,
        getCredentialStatus,
        getSarDownloadPolicySettings,
        getDemTaskSettings,
        isDesktopHost: isTauriDesktopHost,
        getFixtureWorkingAoi,
        getFixtureStorageRoot,
        setAvailableSources,
        setSourceViews,
        setScenes,
        setRowStates,
        setWorkingGeometry,
        setWorkingAoiState,
        setAoiLabel,
        setCreds,
        setStorageRootPath,
        setSarDownloadPolicy,
        setDemConcurrency,
        setPolarizations,
        setBeamModes,
        setLookDirection,
        setBurstStrategy,
        onBatchPage: (page) => {
          if (typeof page.batch_id === "string" && typeof page.total_count === "number") {
            nonBatchScenesActiveRef.current = false;
            displayedBatchRef.current = page.batch_id;
            setBatchTotalCount(page.total_count);
            setBatchDelivery(deliveryFromPage(page as BatchPageResult));
          }
        },
      }),
    []
  );

  useEffect(() => {
    void loadTransferEnginePrefsFromDesktop().catch(() => {});
    return startupEffect();
  }, [startupEffect]);

  // 任务轮询心跳 (保证下载中心与顶栏 Badge 实时更新)
  useEffect(() => {
    let mounted = true;
    const fetchStatus = async () => {
      try {
        const res = await getDownloadStatus();
        if (!mounted || !res) return;

        const taskList: DownloadTaskItem[] = [];
        if (res.asf_tasks && Array.isArray(res.asf_tasks)) {
          for (const t of res.asf_tasks) {
            const isPaused = Boolean(t.paused || t.task_paused);
            const isFailed = Boolean(t.state === "failed" || t.has_failures);
            const isCompleted = t.state === "completed" || t.done === t.total;
            const status = isPaused ? "paused" : isFailed ? "failed" : isCompleted ? "completed" : "running";

            taskList.push({
              id: t.task_id || "task-unknown",
              name: t.name || (t.snapshot_scenes ? translate("mapWorkbench.desktop.sceneBatch", { scenes: t.snapshot_scenes.length }) : translate("mapWorkbench.desktop.dataDownloadTask")),
              provider: (t as any).provider || translate("mapWorkbench.desktop.unknownSource"),
              dataset: (t as any).dataset || (t as any).product_type || translate("mapWorkbench.desktop.unknownSource"),
              status,
              doneBytes: t.done_bytes || 0,
              totalBytes: typeof t.total_bytes === "number" && t.total_bytes > 0 ? t.total_bytes : null,
              speed: t.bytes_per_second ? `${(t.bytes_per_second / 1024 / 1024).toFixed(1)} MB/s` : "—",
              outputDir: t.output_dir,
              error: t.error,
            });
          }
        }
        setTasks(taskList);
      } catch (e) {
        // 忽略轮询非关键报错
      }
    };

    void fetchStatus();
    // 任务总览轮询统一收敛至 taskOverviewSource
    return () => {
      mounted = false;
    };
  }, []);

  const applyResourceSwitchFilters = useCallback(
    (oldResourceId: string, newResourceId: string) => {
      beginResultTransition();
      activeResourceIdRef.current = newResourceId;
      const oldProfile = sarFilterProfile(oldResourceId);
      const newProfile = sarFilterProfile(newResourceId);
      const cleaned = cleanStaleFilterValues(oldProfile, newProfile, {
        platforms,
        productType,
        polarizations,
        beamModes,
        lookDirection,
        orbitDirection: orbitDirection === "all" ? null : orbitDirection,
        relativeOrbit,
        path,
        frame,
      });
      setPlatforms(cleaned.platforms ?? []);
      if (newProfile?.productTypes && newProfile.productTypes.length > 0) {
        if (cleaned.productType) {
          setProductType(cleaned.productType);
        } else if (newResourceId === "science.sentinel-1.standard.v1") {
          setProductType("SLC");
        } else {
          setProductType("");
        }
      } else {
        setProductType("");
      }
      setPolarizations(cleaned.polarizations ?? []);
      setBeamModes(cleaned.beamModes ?? []);
      setLookDirection(cleaned.lookDirection ?? null);
      setOrbitDirection((cleaned.orbitDirection as any) ?? "all");
      setRelativeOrbit(cleaned.relativeOrbit ? String(cleaned.relativeOrbit) : "");
      setPath(cleaned.path ? String(cleaned.path) : "");
      setFrame(cleaned.frame ? String(cleaned.frame) : "");

      clearUnpinnedGeeMapLayers();
      setActiveResourceId(newResourceId);
      setSensor(newResourceId);
      if (!newResourceId.startsWith("dem.")) {
        lastImageResourceIdRef.current = newResourceId;
      }
    },
    [
      platforms,
      productType,
      polarizations,
      orbitDirection,
      relativeOrbit,
      path,
      frame,
      beamModes,
      lookDirection,
      beginResultTransition,
    ]
  );

  // 规则 1：切换数据源只切视图，绝不清空其它源结果（包含 NISAR L ↔ S 互切）
  const handleSelectSource = useCallback(
    async (rawResourceId: string) => {
      try {
        let newResourceId = rawResourceId;
        let companionNotice: TaskOutcomeNotice | null = null;
        if (rawResourceId === "atmospheric.gacos.ztd.v1") {
          clearUnpinnedGeeMapLayers();
          newResourceId = "atmospheric.gacos.ztd.v1";
          companionNotice = {
            tone: "neutral",
            text: "已切换至 GACOS 大气延迟改正：支持按当前 AOI 与日期主动申请天顶总延迟格网。",
          };
          if (activeResourceId === newResourceId) {
            setScientificMode("sar");
            setTaskOutcomeNotice(companionNotice);
            return;
          }
        } else if (
          rawResourceId === "orbit.sentinel1.poeorb.v1" ||
          rawResourceId === "orbit.sentinel-1.poeorb.v1"
        ) {
          clearUnpinnedGeeMapLayers();
          newResourceId = "orbit.sentinel-1.poeorb.v1";
          companionNotice = {
            tone: "neutral",
            text: "已切换至 Sentinel-1 精密轨道（POEORB/RESORB）：支持按 AOI + 日期检索下载或扫描本地影像文件夹。",
          };
          if (activeResourceId === newResourceId) {
            setScientificMode("sar");
            setTaskOutcomeNotice(companionNotice);
            return;
          }
        } else if (DEM_RESOURCE_ID_TO_CODE[rawResourceId]) {
          const mappedDem = DEM_RESOURCE_ID_TO_CODE[rawResourceId];
          if (!activeResourceIdRef.current.startsWith("dem.")) {
            lastImageResourceIdRef.current = activeResourceIdRef.current;
          }
          clearUnpinnedGeeMapLayers();
          beginResultTransition();
          setSourceViewLoading(false);
          setBatchTotalCount(null);
          activeResourceIdRef.current = rawResourceId;
          setActiveResourceId(rawResourceId);
          setScientificMode("dem");
          setDemSource(mappedDem);
          setAutoConvertEllipsoid(
            !rawResourceId.includes("ellipsoidal") && rawResourceId !== "dem.nisar-dem.v1",
          );
          setTaskOutcomeNotice({
            tone: "neutral",
            text: translate("mapWorkbench.desktop.switchedToDemMode", {
              source: localizedSourceDisplayName(rawResourceId, locale),
            }),
          });
          return;
        } else if (rawResourceId.startsWith("science.gee.")) {
          if (rawResourceId === "science.gee.catalog.v1") {
            const curOpts = getWorkbenchToolOptions();
            if (!curOpts.geeAssetId) {
              updateWorkbenchToolOptions({
                geeAssetId: "ALL",
                geeTheme: "all",
                geeSelectedPresetId: "science.gee.catalog.v1",
              });
            }
          } else {
            const presetGeeAsset = CATALOG_METADATA_REGISTRY[rawResourceId]?.geeAssetId;
            updateWorkbenchToolOptions({
              ...(presetGeeAsset ? { geeAssetId: presetGeeAsset } : {}),
              geeSelectedPresetId: rawResourceId,
            });
          }
          newResourceId = "science.gee.catalog.v1";
          const afterOpts = getWorkbenchToolOptions();
          const hasSpecificGeeAsset = Boolean(afterOpts.geeAssetId && afterOpts.geeAssetId !== "ALL");
          if (scientificMode === "gee") {
            clearUnpinnedGeeMapLayers();
            if (activeResourceIdRef.current !== newResourceId) {
              activeResourceIdRef.current = newResourceId;
              setActiveResourceId(newResourceId);
            }
            setScientificMode("gee");
            setTaskOutcomeNotice({
              tone: "neutral",
              text: hasSpecificGeeAsset
                ? `已切换 GEE 数据集：${afterOpts.geeAssetId}`
                : "已留在 GEE 云端计算工作台",
            });
            return;
          }
          if (hasSpecificGeeAsset) {
            companionNotice = {
              tone: "neutral",
              text: `已选定 GEE 数据集：${afterOpts.geeAssetId}（当前在数据检索浏览与裁剪；如需云端合成/指数请切至「GEE 云端计算与制图」）`,
            };
          }
        }
        if (isNisarLogicalResourceId(newResourceId) && !isNisarLogicalResourceId(activeResourceId)) {
          updateWorkbenchToolOptions({ nisarProductLevel: "RSLC" });
        }
        setScientificMode("sar");
        if (
          (newResourceId === "science.sentinel-2.l2a.v1" || newResourceId === "science.landsat.c2-l2.v1")
          && ["cdse", "gee"].includes(getActiveDatasetProvider(newResourceId))
        ) {
          setActiveDatasetProvider(newResourceId, "mpc");
        } else if (
          newResourceId === "science.sentinel-2.l1c.v1"
          && getActiveDatasetProvider(newResourceId) === "gee"
        ) {
          setActiveDatasetProvider(newResourceId, "earth-search");
        }
        silentFallbackAttemptsRef.current.delete(newResourceId);
        if (activeResourceId !== newResourceId) {
          updateWorkbenchToolOptions({ openGeodataSubProduct: "" });
        }
        if (newResourceId === "science.whu.clcd.v1") {
          setStartDate((prev) => (prev > "2025-12-31" || prev < "1985-01-01" ? "2020-01-01" : prev));
          setEndDate((prev) => (prev > "2025-12-31" || prev < "1985-01-01" ? "2025-12-31" : prev));
        }
        applyResourceSwitchFilters(activeResourceId, newResourceId);
        if (companionNotice) {
          setTaskOutcomeNotice(companionNotice);
        }
        const requestGeneration = sourceSwitchGenerationRef.current;

        const optSchema = SEARCH_FILTER_SCHEMA_CATALOG.schemas.find(
          (s) => s.resource_id === newResourceId
        );
        if (optSchema) {
          setOpticalDrafts((prev) => {
            if (prev[newResourceId]) return prev;
            return {
              ...prev,
              [newResourceId]: initialSearchFilterDraft(optSchema),
            };
          });
        }

        const view = batchFor(newResourceId);
        if (view?.batch_id) {
          try {
            const safeViewRevision =
              typeof view.revision === "number" ? view.revision : 0;
            const page = await getBatchPage(view.batch_id, safeViewRevision);
            if (requestGeneration !== sourceSwitchGenerationRef.current || activeResourceIdRef.current !== newResourceId) return;
            const latestView = batchFor(newResourceId);
            if (!latestView || latestView.batch_id !== view.batch_id || latestView.revision !== safeViewRevision) {
              setSourceViewLoading(false);
              setSourceViewError(translate("mapWorkbench.desktop.sourceViewLoadFailed"));
              return;
            }
            applyFirstBatchPage(page);
            setSourceViewLoading(false);
            setSelectedSceneId(null);
            setCheckedSceneIds(new Set());
            setAllMatchingSelected(false);
            setExcludedSceneIds(new Set());
          } catch (err) {
            console.warn("Load batch page error:", err);
            if (requestGeneration === sourceSwitchGenerationRef.current && activeResourceIdRef.current === newResourceId) {
              setSourceViewLoading(false);
              setSourceViewError(translate("mapWorkbench.desktop.sourceViewLoadFailed"));
            }
          }
        } else {
          // 当前源尚无批次：只显示该源的空状态，绝不影响其他源的独立视图
          setScenes([]);
          setSourceViewLoading(false);
          setBatchTotalCount(null);
          setBatchDelivery(null);
          loadMoreGenerationRef.current += 1;
          loadMoreInFlightRef.current = false;
          setLoadingMore(false);
          setLoadMoreError(null);
          setRowStates(new Map());
          setSelectedSceneId(null);
          setCheckedSceneIds(new Set());
          setAllMatchingSelected(false);
          setExcludedSceneIds(new Set());
        }
      } catch (err) {
        console.warn("Select source error:", err);
        setSourceViewLoading(false);
      }
    },
    [activeResourceId, scientificMode, productType, clearUnpinnedGeeMapLayers, applyResourceSwitchFilters, sourceViews, applyFirstBatchPage, batchFor, beginResultTransition, locale, translate]
  );

  // Source views can arrive after the user switches away from the startup
  // source. Load that selected source once its batch becomes known.
  useEffect(() => {
    if (sourceSwitchGenerationRef.current === 0 || displayedBatchRef.current !== null
      || sourceViewLoading || sourceViewError) return;
    if (!sourceViews.some((view) => view.resource_id === activeResourceId && view.batch_id)) return;
    void handleSelectSource(activeResourceId).catch(() => {});
  }, [sourceViews, activeResourceId, sourceViewLoading, sourceViewError, handleSelectSource]);

  const hydrateAgentSessionBatch = useCallback(
    async (
      sessionId: string,
      batchId: string,
      revision: number,
      resourceId: string | null,
      overrideTotalCount?: number | null,
      selectThisBatch?: boolean
    ) => {
      try {
        const page = await getBatchPage(batchId, revision, 0, 250);
        const nextRows = new Map<string, RowTaskState>();
        for (const row of page.rows) nextRows.set(row.item_id, row);
        const resolvedTotal =
          typeof overrideTotalCount === "number" && overrideTotalCount > page.total_count
            ? overrideTotalCount
            : page.total_count;
        const saved = await upsertAgentSessionBatch(sessionId, {
          batchId,
          resourceId,
          revision,
          scenes: page.scenes,
          rowStates: nextRows,
          totalCount: resolvedTotal,
          selectThisBatch,
        });
        if (activeAgentSessionIdRef.current === sessionId) {
          setAgentSessionWorkspace(saved);
        }
      } catch (err) {
        console.warn("Hydrate agent session batch error:", err);
      }
    },
    []
  );

  const handleSelectAgentBatchFilter = useCallback((filter: "all" | string) => {
    const sid = activeAgentSessionIdRef.current;
    if (!sid) return;
    const updated = setAgentSessionBatchFilter(sid, filter);
    if (updated) {
      setAgentSessionWorkspace(updated);
    }
  }, []);

  const handleAgentActiveSessionChangeIsolated = useCallback(
    (sessionId: string | null) => {
      handleAgentActiveSessionChange(sessionId);
      activeAgentSessionIdRef.current = sessionId;
      setActiveAgentSessionId(sessionId);
      if (!sessionId) {
        // Brand-new draft (`activeSessionId === null`): live-follow Manual Workbench
        setAgentSessionWorkspace(null);
        return;
      }
      const cached = peekAgentSessionWorkspaceState(sessionId);
      setAgentSessionWorkspace(cached);
      void getAgentSessionWorkspaceState(sessionId)
        .then(async (remote) => {
          if (activeAgentSessionIdRef.current !== sessionId || !remote) return;
          setAgentSessionWorkspace(remote);
          if (remote.batchId && (!remote.scenes || remote.scenes.length === 0)) {
            await hydrateAgentSessionBatch(sessionId, remote.batchId, 0, remote.resourceId);
          }
        })
        .catch(() => {});
    },
    [handleAgentActiveSessionChange, hydrateAgentSessionBatch]
  );

  const handleAgentSessionCreatedSnapshot = useCallback(
    async (newSessionId: string) => {
      const currentBatchId =
        displayedBatchRef.current ?? batchFor(activeResourceIdRef.current)?.batch_id ?? null;
      // Only inherit Manual Workbench search scenes into a newly created Agent session
      // when a working AOI is set AND the scenes intersect that AOI. Never copy stale out-of-AOI batches.
      const intersectingScenes = workingAoi
        ? scenesRef.current.filter((sc) => sceneIntersectsAoiBbox(sc, workingAoi))
        : [];
      const shouldInheritBatch = Boolean(
        workingAoi && currentBatchId && intersectingScenes.length > 0
      );
      const nextRowStates = new Map<string, RowTaskState>();
      if (shouldInheritBatch) {
        for (const sc of intersectingScenes) {
          const rs = rowStates.get(sc.scene_id);
          if (rs) nextRowStates.set(sc.scene_id, rs);
        }
      }
      const nextTotalCount = shouldInheritBatch
        ? intersectingScenes.length === scenesRef.current.length
          ? (batchTotalCount ?? intersectingScenes.length)
          : intersectingScenes.length
        : 0;
      const initialBatches: AgentSessionBatchEntry[] = shouldInheritBatch
        ? [
            {
              batchId: currentBatchId!,
              resourceId: activeResourceIdRef.current,
              revision: batchFor(activeResourceIdRef.current)?.revision ?? 0,
              scenes: intersectingScenes,
              rowStates: nextRowStates,
              totalCount: nextTotalCount,
              updatedAtMs: Date.now(),
            },
          ]
        : [];
      const saved = await setAgentSessionWorkspaceState(newSessionId, {
        workingAoi,
        workingGeometry,
        aoiLabel,
        batchId: shouldInheritBatch ? currentBatchId : null,
        resourceId: activeResourceIdRef.current,
        scenes: intersectingScenes,
        rowStates: nextRowStates,
        batchTotalCount: nextTotalCount,
        selectedSceneId:
          shouldInheritBatch &&
          selectedSceneId &&
          intersectingScenes.some((sc) => sc.scene_id === selectedSceneId)
            ? selectedSceneId
            : null,
        batches: initialBatches,
        activeBatchFilter: shouldInheritBatch ? (currentBatchId ?? "all") : "all",
      });
      activeAgentSessionIdRef.current = newSessionId;
      setActiveAgentSessionId(newSessionId);
      setAgentSessionWorkspace(saved);
    },
    [workingAoi, workingGeometry, aoiLabel, batchFor, rowStates, batchTotalCount, selectedSceneId]
  );

  const handleAgentSearchSettled = useCallback(
    async ({ resourceId }: { resourceId: string | null }) => {
      const currentSessionId = activeAgentSessionIdRef.current;
      const initialGeneration = sourceSwitchGenerationRef.current;
      const initialResource = activeResourceIdRef.current;
      let requestGeneration = initialGeneration;
      let requestResource = initialResource;
      let transitioned = false;
      try {
        const prevViews = sourceViewsRef.current;
        const nextViews = filterVisibleSourceViews(await getSourceViews());
        sourceViewsRef.current = nextViews;
        setSourceViews(nextViews);
        const picked =
          pickAgentSearchedView(prevViews, nextViews, resourceId) ??
          (resourceId
            ? nextViews.find((v) => v.resource_id === resourceId && Boolean(v.batch_id?.trim())) ?? null
            : null);

        // Inside an existing Agent session: update ONLY that session's isolated results!
        if (currentSessionId) {
          if (picked?.batch_id) {
            const safeRevision = typeof picked.revision === "number" ? picked.revision : 0;
            await hydrateAgentSessionBatch(
              currentSessionId,
              picked.batch_id,
              safeRevision,
              picked.resource_id
            );
          }
          return;
        }

        if (initialGeneration !== sourceSwitchGenerationRef.current || initialResource !== activeResourceIdRef.current) return;
        if (!picked || !picked.batch_id) {
          return;
        }
        if (picked.resource_id !== activeResourceId) {
          applyResourceSwitchFilters(activeResourceId, picked.resource_id);
        } else {
          beginResultTransition();
        }
        transitioned = true;
        requestGeneration = sourceSwitchGenerationRef.current;
        requestResource = picked.resource_id;
        const safeRevision = typeof picked.revision === "number" ? picked.revision : 0;
        const page = await getBatchPage(picked.batch_id, safeRevision);
        if (requestGeneration !== sourceSwitchGenerationRef.current || activeResourceIdRef.current !== picked.resource_id) return;
        const latestView = sourceViewsRef.current.find((view) => view.resource_id === picked.resource_id);
        if (!latestView || latestView.batch_id !== picked.batch_id || (latestView.revision ?? 0) !== safeRevision) {
          setSourceViewLoading(false);
          setSourceViewError(translate("mapWorkbench.desktop.sourceViewLoadFailed"));
          return;
        }
        applyFirstBatchPage(page);
        setSourceViewLoading(false);
        setSelectedSceneId(null);
        setCheckedSceneIds(new Set());
        setAllMatchingSelected(false);
        setExcludedSceneIds(new Set());
      } catch (err) {
        console.warn("Sync agent searched view error:", err);
        if (transitioned && requestGeneration === sourceSwitchGenerationRef.current && requestResource === activeResourceIdRef.current) {
          setSourceViewLoading(false);
          setSourceViewError(translate("mapWorkbench.desktop.sourceViewLoadFailed"));
        }
      }
    },
    [activeResourceId, applyResourceSwitchFilters, applyFirstBatchPage, beginResultTransition, hydrateAgentSessionBatch]
  );

  const handleAgentSessionBatchSettled = useCallback(
    async (info: {
      sessionId?: string | null;
      batchId: string | null;
      resourceId: string | null;
      loadedCount?: number | null;
      totalCount?: number | null;
      selectThisBatch?: boolean;
    }) => {
      const currentSessionId = info.sessionId ?? activeAgentSessionIdRef.current;
      if (info.resourceId && info.batchId) {
        clearedSourceBatchesRef.current.delete(info.resourceId);
      }
      try {
        const nextViews = filterVisibleSourceViews(await getSourceViews());
        sourceViewsRef.current = nextViews;
        setSourceViews(nextViews);
        const targetView = info.batchId
          ? nextViews.find((v) => v.batch_id === info.batchId) ?? null
          : info.resourceId
            ? nextViews.find((v) => v.resource_id === info.resourceId && Boolean(v.batch_id?.trim())) ?? null
            : null;
        const resolvedBatchId = info.batchId ?? targetView?.batch_id ?? null;
        const resolvedResourceId = info.resourceId ?? targetView?.resource_id ?? null;
        const resolvedRevision = typeof targetView?.revision === "number" ? targetView.revision : 0;
        if (currentSessionId && resolvedBatchId) {
          await hydrateAgentSessionBatch(
            currentSessionId,
            resolvedBatchId,
            resolvedRevision,
            resolvedResourceId,
            info.totalCount,
            info.selectThisBatch
          );
          return;
        }
        if (!currentSessionId) {
          await handleAgentSearchSettled({ resourceId: resolvedResourceId });
        }
      } catch (err) {
        console.warn("Sync agent session batch error:", err);
      }
    },
    [hydrateAgentSessionBatch, handleAgentSearchSettled]
  );

  const syncWorkingAoiFromStore = useCallback(async () => {
    try {
      const aoiRes = (await getWorkingAoi()) as any;
      if (!aoiRes || aoiRes.state === "no_aoi" || aoiRes.state === "empty" || !aoiRes.bbox) {
        setWorkingAoiState(null);
        setWorkingGeometry(null);
        setAoiLabel("");
        return;
      }
      const raw = aoiRes.bbox;
      const b: Bbox | null =
        Array.isArray(raw) && raw.length >= 4
          ? {
              west: Number(raw[0]),
              south: Number(raw[1]),
              east: Number(raw[2]),
              north: Number(raw[3]),
            }
          : raw && typeof raw === "object" && typeof raw.west === "number"
            ? (raw as Bbox)
            : null;
      if (!b) {
        setWorkingAoiState(null);
        setWorkingGeometry(null);
        setAoiLabel("");
        return;
      }
      setWorkingAoiState(b);
      setWorkingGeometry(
        aoiRes.geometry && typeof aoiRes.geometry === "object"
          ? (aoiRes.geometry as Record<string, unknown>)
          : null
      );
      setAoiLabel(
        typeof aoiRes.display_name === "string" && aoiRes.display_name.trim().length > 0
          ? aoiRes.display_name.trim()
          : `AOI [${b.west.toFixed(1)}, ${b.south.toFixed(1)}]`
      );
    } catch (err) {
      console.warn("Sync working AOI error:", err);
    }
  }, []);

  const handleAgentSyncWorkingAoi = useCallback(async () => {
    const sid = activeAgentSessionIdRef.current;
    if (sid) {
      const remote = await getAgentSessionWorkspaceState(sid).catch(() => null);
      if (activeAgentSessionIdRef.current === sid && remote) {
        setAgentSessionWorkspace(remote);
      }
      return;
    }
    await syncWorkingAoiFromStore();
  }, [syncWorkingAoiFromStore]);

  const handleAgentUpdateAoi = useCallback(
    (newAoi: Bbox | null, aoiName?: string) => {
      const sid = activeAgentSessionIdRef.current;
      const nextLabel = aoiName
        ? translate("mapWorkbench.desktop.updatedSuffix", { name: aoiName })
        : newAoi
          ? translate("mapWorkbench.desktop.updatedAoi")
          : translate("mapWorkbench.desktop.globalView");
      if (sid) {
        const nextGeo = newAoi
          ? {
              type: "Polygon",
              coordinates: [[
                [newAoi.west, newAoi.south],
                [newAoi.east, newAoi.south],
                [newAoi.east, newAoi.north],
                [newAoi.west, newAoi.north],
                [newAoi.west, newAoi.south],
              ]],
            }
          : null;
        void setAgentSessionWorkspaceState(sid, {
          workingAoi: newAoi,
          workingGeometry: nextGeo,
          aoiLabel: nextLabel,
        }).then((saved) => {
          if (activeAgentSessionIdRef.current === sid) setAgentSessionWorkspace(saved);
        });
        return;
      }
      setWorkingAoiState(newAoi);
      setAoiLabel(nextLabel);
    },
    []
  );

  const handleAgentSelectScene = useCallback((sceneId: string | null) => {
    const sid = activeAgentSessionIdRef.current;
    if (sid) {
      void setAgentSessionWorkspaceState(sid, { selectedSceneId: sceneId }).then((saved) => {
        if (activeAgentSessionIdRef.current === sid) setAgentSessionWorkspace(saved);
      });
      return;
    }
    setSelectedSceneId(sceneId);
  }, []);

  const syncWorkingAoiFromStoreRef = useRef(syncWorkingAoiFromStore);
  syncWorkingAoiFromStoreRef.current = syncWorkingAoiFromStore;
  const handleAgentSearchSettledRef = useRef(handleAgentSearchSettled);
  handleAgentSearchSettledRef.current = handleAgentSearchSettled;
  const handleAgentSyncWorkingAoiRef = useRef(handleAgentSyncWorkingAoi);
  handleAgentSyncWorkingAoiRef.current = handleAgentSyncWorkingAoi;

  useEffect(() => {
    const unsubAoi = subscribeDesktopHostEvent<{ session_id?: string }>(
      "workbench:aoi-changed",
      (payload) => {
        const targetSid =
          typeof payload?.session_id === "string" && payload.session_id.trim()
            ? payload.session_id.trim()
            : null;
        if (targetSid) {
          void getAgentSessionWorkspaceState(targetSid)
            .then((remote) => {
              if (activeAgentSessionIdRef.current === targetSid && remote) {
                setAgentSessionWorkspace(remote);
              }
            })
            .catch(() => {});
          return;
        }
        if (effectivePageRouteRef.current === "agent" && activeAgentSessionIdRef.current) {
          void handleAgentSyncWorkingAoiRef.current().catch(() => {});
          return;
        }
        void syncWorkingAoiFromStoreRef.current().catch(() => {});
      }
    );
    const unsubSearch = subscribeDesktopHostEvent<{ session_id?: string }>(
      "workbench:search-updated",
      (payload) => {
        const targetSid =
          typeof payload?.session_id === "string" && payload.session_id.trim()
            ? payload.session_id.trim()
            : null;
        if (targetSid && activeAgentSessionIdRef.current !== targetSid) {
          return;
        }
        if (!targetSid && effectivePageRouteRef.current !== "agent" && activeAgentSessionIdRef.current) {
          // Manual workbench search updated while user is on Manual Workbench: keep existing Agent sessions isolated!
          return;
        }
        void handleAgentSearchSettledRef.current({ resourceId: null }).catch(() => {});
      }
    );
    return () => {
      unsubAoi();
      unsubSearch();
    };
  }, []);

  // 本地目录选择动作
  const handlePickLocalSlc = useCallback(async () => {
    try {
      const res = await pickDirectory(translate("mapWorkbench.desktop.chooseLocalSlc"));
      if (res && res.ok && res.path) {
        setLocalSlcPath(res.path);
      }
    } catch {}
  }, []);

  const handlePickLocalDem = useCallback(async () => {
    try {
      const res = await pickOpenFile(
        translate("mapWorkbench.desktop.chooseLocalDem"),
        ["tif", "tiff", "hgt", "dem", "bil", "img", "vrt"]
      );
      if (res && res.ok && res.path) {
        setLocalDemPath(res.path);
        const parent = parentDirFromPath(res.path);
        if (parent && !getWorkbenchToolOptions().convertOutputDir.trim()) {
          updateWorkbenchToolOptions({ convertOutputDir: parent });
        }
      }
    } catch {}
  }, []);

  // 规则 2：发起新检索时旧批次保持可浏览，成功切换视图，失败完整保留旧批次并显示错误
  const handleTriggerSearch = useCallback(async (execOpts?: { forceDownload?: boolean; targetItemId?: string; isSilentFallback?: boolean; overrideOutputDir?: string; bypassCache?: boolean }) => {
    const isExplicitDownload = Boolean(
      execOpts && typeof execOpts === "object" && "forceDownload" in execOpts && execOpts.forceDownload,
    );
    const isSilentFallback = Boolean(
      execOpts && typeof execOpts === "object" && "isSilentFallback" in execOpts && execOpts.isSilentFallback,
    );
    const bypassCache = Boolean(
      execOpts && typeof execOpts === "object" && "bypassCache" in execOpts && execOpts.bypassCache,
    );
    const explicitTargetItemId =
      execOpts && typeof execOpts === "object" && "targetItemId" in execOpts
        ? execOpts.targetItemId
        : undefined;
    const explicitOverrideOutputDir =
      execOpts && typeof execOpts === "object" && "overrideOutputDir" in execOpts
        ? execOpts.overrideOutputDir?.trim()
        : undefined;
    if (!isExplicitDownload) {
      if (searchingRef.current && !isSilentFallback) return false;
      if (!isSilentFallback) {
        silentFallbackAttemptsRef.current.delete(activeResourceId);
      }
      setSearching(true);
    }
    const searchToken = !isExplicitDownload
      ? ++searchRunTokenRef.current
      : searchRunTokenRef.current;
    const isCurrentSearch = () =>
      isExplicitDownload || searchToken === searchRunTokenRef.current;
    const finishSearchBusy = () => {
      if (!isExplicitDownload) {
        setSearching(false);
      }
    };
    setSearchError(null);
    setInlineToolResult(null);
    if (!isExplicitDownload) {
      setTaskOutcomeNotice(null);
    }

    const resolveConfiguredNonTmpDir = (toolSpecificDir?: string | null): string =>
      nonTemporaryOutputDir(toolSpecificDir) ||
      nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
      nonTemporaryOutputDir(storageRootPath);

    const resolveVerifiedDownloadOutputDir = async (
      toolSpecificDir: string,
      optionKey: "openGeodataOutputDir" | "xyzTileOutputDir" | "demOutputDir",
    ): Promise<string | null> => {
      const existing = resolveConfiguredNonTmpDir(toolSpecificDir);
      if (existing) return existing;
      if (isTauriDesktopHost() && !isFixtureBackend()) {
        const picked = await pickDirectory("选择数据下载保存目录");
        if (picked && picked.ok && picked.path && nonTemporaryOutputDir(picked.path)) {
          const cleanPicked = picked.path.trim();
          updateWorkbenchToolOptions({ [optionKey]: cleanPicked });
          return cleanPicked;
        }
      }
      setStorageRequiredPrompt(true);
      return null;
    };

    const isItemAlreadyDownloadedInDir = (itemId: string, targetDir: string): boolean => {
      if (!itemId || !targetDir) return false;
      const cleanDir = targetDir.replace(/[\\/]+$/, "");
      const existingScene = scenesRef.current.find(
        (s) => s.scene_id === itemId || s.product_key === itemId,
      );
      const meta = (existingScene?.native_metadata || {}) as Record<string, unknown>;
      const localFile =
        typeof meta.local_file_path === "string" ? meta.local_file_path.trim() : "";
      const downloadedDir =
        typeof meta.downloaded_dir === "string" ? meta.downloaded_dir.replace(/[\\/]+$/, "") : "";
      const rowState = rowStates.get(itemId);
      if (downloadedDir && downloadedDir === cleanDir && rowState?.task_status === "completed") {
        return true;
      }
      if (
        Boolean(meta.already_downloaded) &&
        localFile &&
        (localFile.startsWith(`${cleanDir}/`) || localFile.startsWith(`${cleanDir}\\`))
      ) {
        return true;
      }
      return false;
    };

    const resolveGeeSqlRouteMode = async (
      requireConfiguredForDownload: boolean,
    ): Promise<{ ok: true; sql?: string } | { ok: false; error: string }> => {
      try {
        const statusRes: any = await queryOpenGeodata({ dataset: "gee_account_status" });
        const isConfigured = Boolean(statusRes && statusRes.ok && statusRes.configured === true);
        if (requireConfiguredForDownload && !isConfigured) {
          return {
            ok: false,
            error:
              "GEE_ACCOUNT_REQUIRED: 当前数据集官方裁剪下载需先在「设置 -> 数据服务账号」中配置 Google Earth Engine 云项目凭据",
          };
        }
        return {
          ok: true,
          sql: isConfigured || requireConfiguredForDownload ? "gee_official_rest" : undefined,
        };
      } catch {
        if (requireConfiguredForDownload) {
          return {
            ok: false,
            error:
              "GEE_ACCOUNT_REQUIRED: 当前数据集官方裁剪下载需先在「设置 -> 数据服务账号」中配置 Google Earth Engine 云项目凭据",
          };
        }
        return { ok: true, sql: undefined };
      }
    };

    try {
      // 模式 1: SAR / 光学 / 科学数据检索
      if (scientificMode === "sar") {
        if (activeResourceId === "orbit.sentinel-1.poeorb.v1") {
          const toolOpts = getWorkbenchToolOptions();
          const effectiveOrbitSub = toolOpts.orbitActiveSubMode ?? orbitSourceType;
          if (effectiveOrbitSub === "local") {
            if (!localSlcPath) {
              const errMsg = translate("mapWorkbench.desktop.chooseLocalSlcRequired");
              setSearchError(errMsg);
              setInlineToolResult({ ok: false, message: errMsg });
              finishSearchBusy();
              return false;
            }
            const scanRes = (await queryOpenGeodata({
              dataset: "scan_local_slc_dates",
              code: localSlcPath,
            })) as {
              ok?: boolean;
              source_kind?: string;
              s1_scenes?: string[];
              error?: string;
            } | null;
            if (!isCurrentSearch()) return false;
            if (!scanRes || !scanRes.ok) {
              const errMsg = scanRes?.error || "未能扫描本地 SLC 目录或 ASF 脚本中的 Sentinel-1 影像。";
              setSearchError(errMsg);
              setInlineToolResult({ ok: false, message: errMsg });
              finishSearchBusy();
              return false;
            }
            const scannedScenes = buildLocalSlcCandidateScenes(scanRes.s1_scenes ?? []);
            if (scannedScenes.length === 0) {
              const errMsg = "未在所选目录或脚本中识别出 Sentinel-1 (S1A/S1B/S1C/S1D) 影像名称。";
              setSearchError(errMsg);
              setInlineToolResult({ ok: false, message: errMsg });
              finishSearchBusy();
              return false;
            }
            const matchDir =
              scanRes.source_kind === "script_or_manifest"
                ? nonTemporaryOutputDir(toolOpts.orbitOutputDir) ||
                  parentDirFromPath(localSlcPath) ||
                  nonTemporaryOutputDir(storageRootPath) ||
                  ""
                : localSlcPath;
            const res = matchDir
              ? await matchOrbitsDirectory(matchDir, scannedScenes.slice(0, 100))
              : ({ ok: false, error: "请选择有效的本地轨道目录" } as const);
            if (!isCurrentSearch()) return false;
            const ok = Boolean(res && res.ok);
            const report = res && res.ok ? (res.report as { matched?: number; unmatched?: number; results?: Array<{ scene_id?: string; matched?: boolean }> } | undefined) : undefined;
            let msg = toolResultLine("orbit", (res ?? {}) as Record<string, unknown>);
            if (ok && report && typeof report.unmatched === "number" && report.unmatched > 0) {
              const missingIds = Array.isArray(report.results)
                ? report.results.filter((r) => r && r.matched === false && r.scene_id).map((r) => String(r.scene_id))
                : scannedScenes.map((s) => s.scene_id);
              const targetDownloadDir = nonTemporaryOutputDir(toolOpts.orbitOutputDir) || matchDir;
              if (targetDownloadDir && missingIds.length > 0 && workingGeometry && workingAoi) {
                const startRes = await startOrbitDownloadSnapshot(
                  targetDownloadDir,
                  scannedScenes,
                  missingIds.slice(0, 100),
                  sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
                );
                if (!isCurrentSearch()) return false;
                if (startRes && startRes.ok) {
                  msg = `本地已匹配 ${report.matched ?? 0} 景，已为缺失的 ${missingIds.length} 景创建精密轨道下载任务。`;
                  void taskOverviewSource.refreshNow().catch(() => {});
                } else {
                  msg = `本地已匹配 ${report.matched ?? 0} 景，尚缺 ${report.unmatched} 景（未发布或需绑定 AOI 后补齐）。`;
                }
              } else {
                msg = `本地已匹配 ${report.matched ?? 0} 景，尚缺 ${report.unmatched} 景精密轨道。`;
              }
            }
            setInlineToolResult({ ok, message: msg });
            if (ok) {
              setTaskOutcomeNotice({ tone: "neutral", text: msg });
              setDownloadDrawerOpen(true);
            } else {
              setSearchError(msg);
            }
            finishSearchBusy();
            return ok;
          }

          if (!workingGeometry || !workingAoi) {
            setSearchError(translate("mapWorkbench.desktop.aoiRequired"));
            finishSearchBusy();
            return false;
          }

          const isS1Scene = (s: SceneRow) =>
            s.dataset === "science.sentinel-1.standard.v1" ||
            s.dataset === "science.sentinel-1.slc-burst.v1" ||
            String(s.platform || "").toUpperCase().startsWith("S1") ||
            String(s.platform || "").toUpperCase().includes("SENTINEL-1");

          const checkedS1 = checkedScenes.filter(isS1Scene);
          let targetScenes: SceneRow[] =
            checkedS1.length > 0 ? checkedS1 : scenes.filter(isS1Scene);

          if (targetScenes.length === 0) {
            const existingS1View = batchFor("science.sentinel-1.standard.v1");
            if (existingS1View?.batch_id) {
              const page = await getBatchPage(
                existingS1View.batch_id,
                typeof existingS1View.revision === "number" ? existingS1View.revision : 0,
                0,
                RESULT_PAGE_SIZE,
              );
              if (!isCurrentSearch()) return false;
              targetScenes = page.scenes.filter(isS1Scene);
            }
          }

          if (targetScenes.length === 0) {
            const effectiveAoiName =
              aoiLabel && aoiLabel !== translate("mapWorkbench.desktop.globalView")
                ? aoiLabel
                : translate("mapWorkbench.desktop.mapSelectedExtent");
            const s1Req = buildUnifiedSearchRequest({
              resourceId: "science.sentinel-1.standard.v1",
              aoiGeometry: workingGeometry,
              aoiName: effectiveAoiName,
              startDate,
              endDate,
              platforms: ["SENTINEL-1"],
              productType: "SLC",
              polarizations,
              orbitDirection,
              relativeOrbit,
              path,
              frame,
              beamModes,
              lookDirection,
            });
            const s1Progress = await searchStart(s1Req);
            if (!isCurrentSearch()) {
              await cancelCatalogSearch(s1Progress.request_id).catch(() => {});
              return false;
            }
            activeSearchIdRef.current = s1Progress.request_id;
            let resolvedBatchId = s1Progress.batch_id;
            let lastTerminal = s1Progress.terminal;
            for (let attempt = 0; attempt < 100 && !resolvedBatchId && !lastTerminal; attempt += 1) {
              await new Promise((resolve) => window.setTimeout(resolve, 300));
              if (!isCurrentSearch()) {
                await cancelCatalogSearch(s1Progress.request_id).catch(() => {});
                return false;
              }
              const nextProg = await getSearchProgress(s1Progress.request_id);
              if (!isCurrentSearch()) return false;
              resolvedBatchId = nextProg.batch_id;
              lastTerminal = nextProg.terminal;
            }
            activeSearchIdRef.current = null;
            if (resolvedBatchId) {
              const views = await getSourceViews();
              if (!isCurrentSearch()) return false;
              const s1View = views.find((v) => v.batch_id === resolvedBatchId);
              const s1Page = await getBatchPage(
                resolvedBatchId,
                typeof s1View?.revision === "number" ? s1View.revision : 0,
                0,
                RESULT_PAGE_SIZE,
              );
              if (!isCurrentSearch()) return false;
              targetScenes = s1Page.scenes;
              if (targetScenes.length > 0) {
                setScenes(targetScenes);
                setBatchTotalCount(s1Page.total_count);
                setSelectedSceneId(targetScenes[0]?.scene_id ?? null);
              }
            }
          }

          if (targetScenes.length === 0) {
            const emptyMsg = "当前研究区与日期范围内未找到匹配的 Sentinel-1 影像，无法解析精密轨道";
            setInlineToolResult({ ok: false, message: emptyMsg });
            setTaskOutcomeNotice({ tone: "neutral", text: emptyMsg });
            finishSearchBusy();
            return false;
          }

          if (!explicitOverrideOutputDir) {
            const prefillOrbitDir =
              nonTemporaryOutputDir(toolOpts.orbitOutputDir) ||
              nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
              nonTemporaryOutputDir(storageRootPath) ||
              "";
            setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
              sourceBatchId: "geodata-direct-orbit",
              sourceBatchRevision: 0,
              selectionScope: targetScenes.length > 1 ? "checked" : "exact_item",
              selectedProductIds: targetScenes.map((s) => s.scene_id),
              acquisition: {
                kind: "generic_sar",
                frozenParameters: {
                  dedicated_geodata: true,
                  dedicated_action: "orbit_download",
                  dataset_label: "Sentinel-1 精密轨道 (POEORB)",
                  resource_id: "orbit.sentinel-1.poeorb.v1",
                  estimated_total_bytes: 0,
                },
              },
              destinationPrefill: prefillOrbitDir,
              layout: "direct",
              taskPolicy: sarDownloadPolicy?.mode || "default",
              concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
            }));
            finishSearchBusy();
            return true;
          }

          const targetOrbitDir = explicitOverrideOutputDir;
          if (!targetOrbitDir) {
            finishSearchBusy();
            return false;
          }

          const startRes = await startOrbitDownloadSnapshot(
            targetOrbitDir,
            targetScenes,
            targetScenes.map((s) => s.scene_id),
            sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
          );
          if (!isCurrentSearch()) return false;
          if (startRes && startRes.ok) {
            const okMsg = orbitDownloadResultLine(startRes as Record<string, unknown>, targetScenes.length);
            setInlineToolResult({ ok: true, message: okMsg });
            setTaskOutcomeNotice({ tone: "neutral", text: okMsg });
            void taskOverviewSource.refreshNow().catch(() => {});
            setDownloadDrawerOpen(true);
            finishSearchBusy();
            return true;
          }
          const failMsg = orbitDownloadResultLine((startRes ?? {}) as Record<string, unknown>, targetScenes.length);
          setInlineToolResult({ ok: false, message: failMsg });
          setSearchError(failMsg);
          finishSearchBusy();
          return false;
        }

        if (activeResourceId === "atmospheric.gacos.ztd.v1") {
          if (!workingGeometry || !workingAoi) {
            setSearchError(translate("mapWorkbench.desktop.aoiRequired"));
            finishSearchBusy();
            return false;
          }
          const toolOpts = getWorkbenchToolOptions();
          // GACOS 属官网邮件申请制，无需弹窗强制要求用户配置下载目录
          const targetGacosDir = resolveConfiguredNonTmpDir(toolOpts.openGeodataOutputDir) || ".";
          if (!isCurrentSearch()) {
            finishSearchBusy();
            return false;
          }
          setGacosSubmitting(true);
          try {
            const outcome = await executeStandaloneGacosRequest({
              outputRoot: targetGacosDir,
              manualDatesText: toolOpts.gacosDatesText,
              startDate,
              endDate,
              utcHour: Number.parseInt(toolOpts.gacosUtcHour || "10", 10),
              utcMinute: Number.parseInt(toolOpts.gacosUtcMinute || "0", 10),
              outputFormat: toolOpts.gacosOutputFormat || "geotiff",
              studyAreaName: aoiLabel,
              studyAreaGeometry: workingGeometry,
              studyAreaBbox: workingAoi,
              selectedBatchIndices: selectedGacosBatchIndices ?? undefined,
              onBatchStatusChange: (batchIdx, state) => {
                setGacosBatchStatuses((prev) => {
                  const next = new Map(prev);
                  next.set(batchIdx, state);
                  return next;
                });
              },
            });
            if (!isCurrentSearch()) return false;
            if (outcome.ok) {
              setInlineToolResult({ ok: true, message: outcome.message });
              setTaskOutcomeNotice({ tone: "neutral", text: outcome.message });
              void taskOverviewSource.refreshNow().catch(() => {});
              setDownloadDrawerOpen(true);
              finishSearchBusy();
              return true;
            }
            setInlineToolResult({ ok: false, message: outcome.error });
            setSearchError(outcome.error);
            finishSearchBusy();
            return false;
          } finally {
            setGacosSubmitting(false);
          }
        }

        const isGlobalOrbitEphemeris =
          activeResourceId === "orbit.nisar.oe.v1" ||
          activeResourceId === "orbit.copernicus.pod.v1";
        const resolvedAoi: Bbox | null =
          workingAoi ??
          (isGlobalOrbitEphemeris
            ? { west: -180, south: -90, east: 180, north: 90, crs: "EPSG:4326" }
            : null);
        const resolvedGeometry =
          workingGeometry ??
          (isGlobalOrbitEphemeris && resolvedAoi ? bboxToGeoJsonPolygon(resolvedAoi) : null);
        if (!resolvedGeometry || !resolvedAoi) {
          setSearchError(translate("mapWorkbench.desktop.aoiRequired"));
          finishSearchBusy();
          return false;
        }

        if (activeResourceId === "auxiliary.xyz-tiles.v1") {
          const toolOpts = getWorkbenchToolOptions();
          const isMbtiles = toolOpts.xyzTileExportFormat === "mbtiles";
          const modeFlag =
            isMbtiles || !toolOpts.xyzTileStitchMosaic
              ? "nomosaic"
              : "mosaic";
          const pyrFlag = toolOpts.xyzTileBuildPyramids ? "pyramids" : "nopyramids";
          const isWayback = toolOpts.xyzTileSource === "esri_wayback";
          const waybackPeriods: WaybackPeriodSelection[] = isWayback
            ? toolOpts.xyzWaybackSelectedPeriods.length > 0
              ? toolOpts.xyzWaybackSelectedPeriods
              : isValidWaybackVersionId(toolOpts.xyzWaybackVersionId)
                ? [
                    {
                      releaseNum: toolOpts.xyzWaybackVersionId,
                      dateLabel: toolOpts.xyzWaybackVersionId,
                    },
                  ]
                : []
            : [];
          if (
            isWayback &&
            (waybackPeriods.length === 0 ||
              waybackPeriods.some((p) => !isValidWaybackVersionId(p.releaseNum)))
          ) {
            setSearchError("请先在 Wayback 面板选择一个或多个有效的历史版本期次。");
            finishSearchBusy();
            return false;
          }
          const isCustom = toolOpts.xyzTileSource === "custom";
          const customUrl = toolOpts.xyzTileCustomUrl.trim();
          const srcPart = isCustom
            ? `custom|${toolOpts.xyzTileCoordSys}`
            : toolOpts.xyzTileSource;
          const draftZooms = (opticalDrafts[activeResourceId] as { xyz_zooms?: number[] } | undefined)?.xyz_zooms;
          const rawZoomSource = Array.isArray(draftZooms)
            ? draftZooms
            : Array.isArray(toolOpts.xyzTileZooms)
              ? toolOpts.xyzTileZooms
              : [toolOpts.xyzTileZoom];
          const zoomList = Array.from(
            new Set(rawZoomSource.filter((z) => typeof z === "number" && Number.isFinite(z))),
          ).sort((a, b) => a - b);
          const staticMaxZoom = getXyzSourceMaxZoom(toolOpts.xyzTileSource);
          const sourceMaxZoom =
            isWayback &&
            typeof toolOpts.xyzWaybackProbedMaxZoom === "number" &&
            Number.isFinite(toolOpts.xyzWaybackProbedMaxZoom)
              ? Math.min(staticMaxZoom, Math.max(1, Math.round(toolOpts.xyzWaybackProbedMaxZoom)))
              : staticMaxZoom;
          const precheck = estimateXyzTilesForBbox(resolvedAoi, zoomList, {
            source: toolOpts.xyzTileSource,
            exportFormat: toolOpts.xyzTileExportFormat,
            stitchMosaic: toolOpts.xyzTileStitchMosaic,
            compression: toolOpts.xyzTileCompression,
            includeOverviews: toolOpts.xyzTileBuildPyramids,
            maxZoomOverride: isWayback ? toolOpts.xyzWaybackProbedMaxZoom : undefined,
          });
          if (!precheck || zoomList.length === 0 || zoomList.some((z) => z < 1 || z > sourceMaxZoom)) {
            setSearchError(`当前图源 ${toolOpts.xyzTileSource} 仅支持 Z1–Z${sourceMaxZoom} 缩放级别，请调整已选层级后重试。`);
            finishSearchBusy();
            return false;
          }
          if (precheck.exceedsJpegDimensionLimit) {
            setSearchError(
              `JPEG 单边最大支持 65,535 px（当前网格 ${precheck.maxGridWidthPx.toLocaleString()} × ${precheck.maxGridHeightPx.toLocaleString()} px），请缩小区域或改用 GeoTIFF / COG / PNG。`,
            );
            finishSearchBusy();
            return false;
          }
          if (precheck.exceedsHardLimit) {
            setSearchError(
              `预计共需 ${precheck.totalTiles.toLocaleString()} 张瓦片，已超出当前模式上限 ${precheck.maxTilesLimit.toLocaleString()} 张。请缩小研究区、调低层级，或切换至 MBTiles / 不拼接归档模式。`,
            );
            finishSearchBusy();
            return false;
          }

          if (!explicitOverrideOutputDir) {
            const prefillOut = resolveConfiguredNonTmpDir(toolOpts.xyzTileOutputDir) || "";
            const zoomLabel = zoomList.map((z) => `Z${z}`).join(",");
            const scaledBatch = isWayback
              ? scaleWaybackEstimateForPeriods(precheck, waybackPeriods.length)
              : null;
            const confirmProductIds = isWayback
              ? waybackPeriods.flatMap((p) => zoomList.map((z) => `wayback-${p.releaseNum}-z${z}`))
              : zoomList.map((z) => `xyz-tile-z${z}`);
            const confirmLabel = isWayback
              ? `${humanPreparationTitle("auxiliary.xyz-tiles.v1")} (${waybackPeriods.length} 期 · ${zoomLabel})`
              : `${humanPreparationTitle("auxiliary.xyz-tiles.v1")} (${zoomLabel})`;
            setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
              sourceBatchId: "geodata-direct-auxiliary.xyz-tiles.v1",
              sourceBatchRevision: 0,
              selectionScope: confirmProductIds.length > 1 ? "checked" : "exact_item",
              selectedProductIds: confirmProductIds,
              acquisition: {
                kind: "generic_sar",
                frozenParameters: {
                  dedicated_geodata: true,
                  dedicated_action: "xyz_tiles",
                  dataset_label: confirmLabel,
                  resource_id: "auxiliary.xyz-tiles.v1",
                  estimated_total_bytes: scaledBatch
                    ? scaledBatch.totalEstimatedOutputBytes
                    : precheck.totalEstimatedOutputBytes,
                },
              },
              destinationPrefill: prefillOut,
              layout: "direct",
              taskPolicy: sarDownloadPolicy?.mode || "default",
              concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
            }));
            finishSearchBusy();
            return true;
          }

          const targetOut = explicitOverrideOutputDir;
          const periodList: (WaybackPeriodSelection | null)[] = isWayback
            ? waybackPeriods
            : [null];

          const allMapped: SceneRow[] = [];
          const nextRows = new Map<string, RowTaskState>();
          let totalTileCount = 0;
          let lastOkRes: any = null;
          let lastErr: string | null = null;
          let allAlreadyRecorded = true;

          const singleMultiZoomCall = isMbtiles || !toolOpts.xyzTileStitchMosaic;
          for (const period of periodList) {
            const periodSrcPart = period ? `wayback_${period.releaseNum}` : srcPart;
            const wbSuffix = period ? `_wb_${period.releaseNum}` : "";
            const periodOutDir =
              period && targetOut.trim()
                ? joinOutputSubdir(
                    targetOut,
                    formatWaybackSubfolderName(period.releaseNum, period.dateLabel),
                  )
                : targetOut;

            if (singleMultiZoomCall) {
              const primaryZoom = zoomList[zoomList.length - 1] ?? toolOpts.xyzTileZoom;
              const res: any = await queryOpenGeodata({
                dataset: "xyz_satellite_tiles",
                bbox: [resolvedAoi.west, resolvedAoi.south, resolvedAoi.east, resolvedAoi.north],
                code: isCustom ? customUrl : undefined,
                year: primaryZoom,
                zooms: zoomList,
                keyword: `${periodSrcPart}:${primaryZoom}:${modeFlag}:${toolOpts.xyzTileExportFormat}:${toolOpts.xyzTileCompression}:${pyrFlag}`,
                output_path: periodOutDir,
              });
              if (!isCurrentSearch()) return false;
              if (res && res.ok) {
                lastOkRes = res;
                totalTileCount += typeof res.tile_count === "number" ? res.tile_count : 0;
                if (res.task_already_recorded !== true) allAlreadyRecorded = false;
                const zoomSuffix = zoomList.map((z) => `z${z}`).join("-");
                const mapped = mapOpenGeodataResponseToScenes(activeResourceId, res, resolvedAoi, endDate).map((sc) => {
                  const uniqueId = `${sc.scene_id}_${zoomSuffix}${wbSuffix}`;
                  return {
                    ...sc,
                    scene_id: uniqueId,
                    product_key: uniqueId,
                    product_id: uniqueId,
                  };
                });
                allMapped.push(...mapped);
                const realTaskId =
                  typeof res.task_id === "string" && res.task_id.trim()
                    ? res.task_id.trim()
                    : `xyz-tile-task-${zoomSuffix}${wbSuffix}`;
                for (const sc of mapped) {
                  nextRows.set(sc.scene_id, {
                    item_id: sc.scene_id,
                    task_id: realTaskId,
                    task_status: "completed",
                    item_status: "completed",
                    actions: [],
                  });
                }
              } else {
                lastErr = String(res?.error || `瓦片导出未返回可用结果`);
              }
            } else {
              for (const zoom of zoomList) {
                const res: any = await queryOpenGeodata({
                  dataset: "xyz_satellite_tiles",
                  bbox: [resolvedAoi.west, resolvedAoi.south, resolvedAoi.east, resolvedAoi.north],
                  code: isCustom ? customUrl : undefined,
                  year: zoom,
                  keyword: `${periodSrcPart}:${zoom}:${modeFlag}:${toolOpts.xyzTileExportFormat}:${toolOpts.xyzTileCompression}:${pyrFlag}`,
                  output_path: periodOutDir,
                });
                if (!isCurrentSearch()) return false;
                if (res && res.ok) {
                  lastOkRes = res;
                  totalTileCount += typeof res.tile_count === "number" ? res.tile_count : 0;
                  if (res.task_already_recorded !== true) allAlreadyRecorded = false;
                  const mapped = mapOpenGeodataResponseToScenes(activeResourceId, res, resolvedAoi, endDate).map((sc) => {
                    const uniqueId =
                      zoomList.length > 1 || period
                        ? `${sc.scene_id}_z${zoom}${wbSuffix}`
                        : sc.scene_id;
                    return {
                      ...sc,
                      scene_id: uniqueId,
                      product_key: uniqueId,
                      product_id: uniqueId,
                    };
                  });
                  allMapped.push(...mapped);
                  const realTaskId =
                    typeof res.task_id === "string" && res.task_id.trim()
                      ? res.task_id.trim()
                      : `xyz-tile-task-z${zoom}${wbSuffix}`;
                  for (const sc of mapped) {
                    nextRows.set(sc.scene_id, {
                      item_id: sc.scene_id,
                      task_id: realTaskId,
                      task_status: "completed",
                      item_status: "completed",
                      actions: [],
                    });
                  }
                } else {
                  lastErr = String(res?.error || `Z${zoom} 瓦片导出未返回可用结果`);
                }
              }
            }
          }

          if (lastOkRes) {
            await taskOverviewSource.refreshNow();
            setScenes(allMapped);
            setBatchTotalCount(allMapped.length);
            setSelectedSceneId(allMapped[0]?.scene_id ?? null);
            setRowStates(nextRows);
            const zoomLabel = zoomList.map((z) => `Z${z}`).join(",");
            const overlay = extractGeodataRasterOverlayFromResponse(
              lastOkRes,
              resolvedAoi,
              `${zoomLabel} 卫星瓦片拼接图`,
            );
            if (overlay) {
              setGeodataRasterOverlay(overlay);
            }
            const resolvedMosaicPath = String(
              lastOkRes.mbtiles_path ||
                lastOkRes.zip_archive_path ||
                lastOkRes.exported_mosaic_path ||
                lastOkRes.mosaic_geotiff_path ||
                lastOkRes.mosaic_vrt_path ||
                "",
            );
            const mosaicMsg =
              periodList.length <= 1 && resolvedMosaicPath
                ? ` · 拼接大图：${resolvedMosaicPath}`
                : "";
            const shownOutDir =
              periodList.length > 1 && targetOut.trim()
                ? targetOut
                : String(lastOkRes.output_dir || targetOut);
            const okText =
              allAlreadyRecorded
                ? `该瓦片包已存在于目标目录 ${shownOutDir}，无需重复下载`
                : periodList.length > 1
                  ? `已加入下载中心并完成 ${periodList.length} 期历史影像分目录导出（共 ${totalTileCount} 张 ${zoomLabel} 瓦片）至 ${shownOutDir}`
                  : `已加入下载中心并下载 ${totalTileCount} 张 ${zoomLabel} 配准瓦片至 ${shownOutDir}${mosaicMsg}`;
            setInlineToolResult({ ok: true, message: okText });
            setTaskOutcomeNotice({ tone: "neutral", text: okText });
          } else {
            const errText = lastErr || "当前区域未返回可用瓦片";
            setInlineToolResult({ ok: false, message: errText });
            setSearchError(errText);
          }
          finishSearchBusy();
          return Boolean(lastOkRes?.ok);
        }

        setFailoverBanner(null);
        const currentProvider = getActiveDatasetProvider(activeResourceId);
        const isOpticalGranule =
          activeResourceId.startsWith("science.sentinel-2.") || activeResourceId.includes("landsat");
        if (!isOpticalGranule && currentProvider === "gee") {
          let geeAsset = "COPERNICUS/S2_SR_HARMONIZED";
          if (activeResourceId.includes("landsat")) {
            geeAsset = "LANDSAT/LC09/C02/T1_L2";
          } else if (activeResourceId.startsWith("science.sentinel-2.")) {
            geeAsset = activeResourceId === "science.sentinel-2.l1c.v1" ? "COPERNICUS/S2_HARMONIZED" : "COPERNICUS/S2_SR_HARMONIZED";
          }
          updateWorkbenchToolOptions({ geeAssetId: geeAsset });
          const geeRoute = await resolveGeeSqlRouteMode(isExplicitDownload);
          if (!geeRoute.ok) {
            setInlineToolResult({ ok: false, message: geeRoute.error });
            setSearchError(geeRoute.error);
            finishSearchBusy();
            return false;
          }
          const toolOpts = getWorkbenchToolOptions();
          const orbitPreferredDir = isGlobalOrbitEphemeris
            ? (toolOpts.orbitOutputDir || toolOpts.openGeodataOutputDir)
            : toolOpts.openGeodataOutputDir;
          let targetOut: string | undefined =
            explicitOverrideOutputDir || resolveConfiguredNonTmpDir(orbitPreferredDir) || undefined;
          if (isExplicitDownload) {
            const verifiedDir = explicitOverrideOutputDir || await resolveVerifiedDownloadOutputDir(
              orbitPreferredDir,
              "openGeodataOutputDir",
            );
            if (!verifiedDir) {
              finishSearchBusy();
              return false;
            }
            const candidateId = explicitTargetItemId || scenesRef.current[0]?.scene_id || "";
            if (!explicitOverrideOutputDir && candidateId && isItemAlreadyDownloadedInDir(candidateId, verifiedDir)) {
              const dupMsg = translate("mapWorkbench.desktop.dataAlreadyExistsInDir", { dir: verifiedDir });
              setInlineToolResult({ ok: true, message: dupMsg });
              setTaskOutcomeNotice({ tone: "neutral", text: dupMsg });
              finishSearchBusy();
              return false;
            }
            targetOut = verifiedDir;
          }
          const bandToken =
            toolOpts.geeSelectedBand && toolOpts.geeSelectedBand.trim()
              ? `|${toolOpts.geeSelectedBand.trim()}`
              : "";
          const actionSuffix = isExplicitDownload ? "download" : "search";
          const keyword = `${geeAsset}|${startDate || ""}..${endDate || ""}${bandToken}|${actionSuffix}`;

          const res: any = await queryOpenGeodata({
            dataset: "gee_catalog",
            bbox: [resolvedAoi.west, resolvedAoi.south, resolvedAoi.east, resolvedAoi.north],
            lat: (resolvedAoi.south + resolvedAoi.north) / 2,
            lng: (resolvedAoi.west + resolvedAoi.east) / 2,
            date: endDate || startDate,
            start_date: startDate || undefined,
            end_date: endDate || undefined,
            code: geeAsset,
            osm_tag: explicitTargetItemId || undefined,
            keyword,
            output_path: targetOut,
            sql: geeRoute.sql,
            gee_params: {
              use_availability_check: true,
              max_cloud: toolOpts.geeMaxCloud ?? 30,
              band: toolOpts.geeSelectedBand || undefined,
              sar_polarization: toolOpts.geeSarPolarization || undefined,
              orbit_direction: orbitDirection || undefined,
              reducer: toolOpts.geeTemporalStat || "median",
            },
          });
          if (!isCurrentSearch()) return false;
          if (res && res.ok) {
            const mapped = mapOpenGeodataResponseToScenes(activeResourceId, res, resolvedAoi, endDate || "");
            if (isExplicitDownload) {
              await taskOverviewSource.refreshNow();
              const savedPath = String(
                res.downloaded_file || res.downloaded_path || res.output_path || "",
              );
              const completedId = String(
                res.target_item_id || explicitTargetItemId || mapped[0]?.scene_id || "",
              );
              const affectedRowIds = Array.from(
                new Set(
                  [completedId, explicitTargetItemId]
                    .map((id) => String(id || "").trim())
                    .filter(Boolean),
                ),
              );
              const realTaskId =
                typeof res.task_id === "string" && res.task_id.trim()
                  ? res.task_id.trim()
                  : `gee-task-${completedId}`;
              if (scenesRef.current.length > 0 && affectedRowIds.length > 0) {
                setScenes((prev) =>
                  prev.map((sc) =>
                    affectedRowIds.includes(sc.scene_id)
                      ? {
                          ...sc,
                          native_metadata: {
                            ...(sc.native_metadata || {}),
                            already_downloaded: true,
                            local_file_path: savedPath || null,
                            downloaded_dir: targetOut || null,
                          },
                        }
                      : sc,
                  ),
                );
              } else {
                setScenes(mapped);
                setBatchTotalCount(mapped.length);
              }
              if (affectedRowIds.length > 0) {
                setRowStates((prev) => {
                  const next = new Map(prev);
                  for (const rowId of affectedRowIds) {
                    next.set(rowId, {
                      item_id: rowId,
                      task_id: realTaskId,
                      task_status: "completed",
                      item_status: "completed",
                      actions: [],
                    });
                  }
                  return next;
                });
                setCheckedSceneIds((prev) => {
                  if (!affectedRowIds.some((rowId) => prev.has(rowId))) return prev;
                  const next = new Set(prev);
                  for (const rowId of affectedRowIds) {
                    next.delete(rowId);
                  }
                  return next;
                });
              }
              const doneMsg =
                res.already_exists === true || res.task_already_recorded === true
                  ? translate("mapWorkbench.desktop.dataAlreadyExistsInDir", { dir: targetOut || "" })
                  : translate("mapWorkbench.desktop.geeDownloadSaved", { path: savedPath });
              setInlineToolResult({ ok: true, message: doneMsg });
              setTaskOutcomeNotice({ tone: "neutral", text: doneMsg });
              finishSearchBusy();
              return Boolean(savedPath);
            } else {
              activateNonBatchSceneList(mapped);
              const nextRows = new Map<string, RowTaskState>();
              for (const sc of mapped) {
                const meta = (sc.native_metadata || {}) as Record<string, unknown>;
                if (meta.already_downloaded === true) {
                  nextRows.set(sc.scene_id, {
                    item_id: sc.scene_id,
                    task_id: `gee-${sc.scene_id}`,
                    task_status: "completed",
                    item_status: "completed",
                    actions: [],
                  });
                }
              }
              setRowStates(nextRows);
              const overlay = extractGeodataRasterOverlayFromResponse(
                res,
                resolvedAoi,
                `GEE (${geeAsset})`,
              );
              if (overlay) {
                setGeodataRasterOverlay(overlay);
              }
              const summaryMsg = translate("mapWorkbench.desktop.geeSearchMatched", {
                asset: geeAsset,
                count: mapped.length,
              });
              setInlineToolResult({ ok: true, message: summaryMsg });
              setTaskOutcomeNotice({ tone: "neutral", text: summaryMsg });
              setFailoverBanner(null);
              finishSearchBusy();
              return true;
            }
          } else {
            const errMsg = String(
              res?.error || translate("mapWorkbench.desktop.geeSearchFailed", { asset: geeAsset }),
            );
            setInlineToolResult({ ok: false, message: errMsg });
            setSearchError(errMsg);
            finishSearchBusy();
            return false;
          }
        }

        const activeNisarLevel = isNisarLogicalResourceId(activeResourceId)
          ? resolveNisarProductLevel(productType || getWorkbenchToolOptions().nisarProductLevel)
          : null;
        if (
          activeNisarLevel &&
          isNisarSBandResourceId(activeResourceId) &&
          activeNisarLevel === "SME2"
        ) {
          const sBandSme2Msg = translate("mapWorkbench.desktop.nisarSme2LBandOnly");
          setInlineToolResult({ ok: false, message: sBandSme2Msg });
          setTaskOutcomeNotice({ tone: "neutral", text: sBandSme2Msg });
          finishSearchBusy();
          return false;
        }
        const nisarToolOpts = getWorkbenchToolOptions();
        const activeNisarTiers: ("validated" | "provisional")[] =
          Array.isArray(nisarToolOpts.nisarReleaseTiers) && nisarToolOpts.nisarReleaseTiers.length > 0
            ? nisarToolOpts.nisarReleaseTiers
            : ["validated", "provisional"];
        const isNisarLBandMultiProduct = Boolean(
          activeNisarLevel &&
          !isNisarSBandResourceId(activeResourceId) &&
          (activeNisarLevel !== "RSLC" || activeNisarTiers.includes("validated"))
        );
        const isNisarSBandMultiProduct = Boolean(
          activeNisarLevel &&
          isNisarSBandResourceId(activeResourceId) &&
          activeNisarLevel !== "RSLC"
        );
        const dedicatedDataset =
          OPEN_GEODATA_RESOURCE_TO_DATASET[activeResourceId] ||
          (isNisarSBandMultiProduct
            ? "bhoonidhi_sband_products"
            : isNisarLBandMultiProduct
              ? "nisar_products"
              : undefined);
        if (dedicatedDataset) {
          const toolOpts = getWorkbenchToolOptions();
          const geeRoute =
            dedicatedDataset === "gee_catalog"
              ? await resolveGeeSqlRouteMode(isExplicitDownload)
              : { ok: true as const, sql: undefined };
          if (!geeRoute.ok) {
            setInlineToolResult({ ok: false, message: geeRoute.error });
            setSearchError(geeRoute.error);
            finishSearchBusy();
            return false;
          }
          const targetRowMeta = explicitTargetItemId
            ? (scenesRef.current.find((sc) => sc.scene_id === explicitTargetItemId)?.native_metadata as Record<string, unknown> | undefined)
            : undefined;
          const rowGeeAsset =
            (typeof targetRowMeta?.gee_asset_id === "string" && targetRowMeta.gee_asset_id.trim()) ||
            (explicitTargetItemId?.startsWith("gee-public::")
              ? explicitTargetItemId.slice("gee-public::".length)
              : "");
          const themeFallbackGeeAsset =
            filterGeePublicCatalog((toolOpts.geeTheme || "all") as GeeCatalogTheme, "")[0]?.assetId ||
            "COPERNICUS/S2_SR_HARMONIZED";
          const effectiveGeeAssetId =
            rowGeeAsset ||
            (toolOpts.geeAssetId && toolOpts.geeAssetId !== "ALL"
              ? toolOpts.geeAssetId
              : themeFallbackGeeAsset);
          const orbitPreferredDir = isGlobalOrbitEphemeris
            ? (toolOpts.orbitOutputDir || toolOpts.openGeodataOutputDir)
            : toolOpts.openGeodataOutputDir;
          let targetOut: string | undefined =
            explicitOverrideOutputDir || resolveConfiguredNonTmpDir(orbitPreferredDir) || undefined;
          if (isExplicitDownload) {
            if (isSourceSearchOnly(activeResourceId)) {
              const ssoMsg = translate("mapWorkbench.desktop.enmapSearchOnlyNotice");
              setInlineToolResult({ ok: false, message: ssoMsg });
              setTaskOutcomeNotice({ tone: "danger", text: ssoMsg });
              finishSearchBusy();
              return false;
            }
            const verifiedDir = explicitOverrideOutputDir || await resolveVerifiedDownloadOutputDir(
              orbitPreferredDir,
              "openGeodataOutputDir",
            );
            if (!verifiedDir) {
              finishSearchBusy();
              return false;
            }
            const candidateId = explicitTargetItemId || scenesRef.current[0]?.scene_id || "";
            if (!explicitOverrideOutputDir && candidateId && isItemAlreadyDownloadedInDir(candidateId, verifiedDir)) {
              const dupMsg = translate("mapWorkbench.desktop.dataAlreadyExistsInDir", { dir: verifiedDir });
              setInlineToolResult({ ok: true, message: dupMsg });
              setTaskOutcomeNotice({ tone: "neutral", text: dupMsg });
              finishSearchBusy();
              return false;
            }
            targetOut = verifiedDir;
          }
          const actionSuffix = isExplicitDownload && !isSourceSearchOnly(activeResourceId) ? "download" : "search";
          const pickSub = (allowed: readonly string[], fallback: string) =>
            allowed.includes(toolOpts.openGeodataSubProduct)
              ? toolOpts.openGeodataSubProduct
              : fallback;
          let keyword = `${startDate}..${endDate}|${actionSuffix}`;
          const queryDate: string | undefined = endDate || startDate;
          const geeIsStaticAsset =
            activeResourceId === "science.gee.catalog.v1" &&
            isStaticGeeAssetId(effectiveGeeAssetId);
          const orbitToken =
            orbitDirection === "ascending"
              ? "|ASCENDING"
              : orbitDirection === "descending"
                ? "|DESCENDING"
                : "";
          if (activeResourceId === "science.esa.worldcover.v1") {
            keyword = `${pickSub(["v200", "v100"], "v200")}|${actionSuffix}`;
          } else if (activeResourceId === "science.jrc.gsw.v1") {
            keyword = `${pickSub(["occurrence", "change", "seasonality", "recurrence", "transitions", "extent"], "occurrence")}|${actionSuffix}`;
          } else if (activeResourceId === "science.alos.palsar-mosaic.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["HH", "HV", "linci", "mask"], "HH")}|${actionSuffix}`;
          } else if (activeResourceId === "science.modis.ndvi.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["250m_16_days_NDVI", "250m_16_days_EVI", "250m_16_days_VI_Quality"], "250m_16_days_NDVI")}|${actionSuffix}`;
          } else if (activeResourceId === "atmospheric.sentinel-5p.l2.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["no2", "ch4", "co", "so2", "o3", "aer_ai"], "no2")}|${actionSuffix}`;
          } else if (activeResourceId === "atmospheric.era5.reanalysis.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["hourly_insar", "hourly_full", "daily_summary"], "hourly_insar")}|${actionSuffix}`;
          } else if (activeResourceId === "science.opera.disp-s1.v1") {
            const sub = pickSub(["DISP-S1", "CSLC-S1", "RTC-S1"], "DISP-S1");
            keyword = `${startDate}..${endDate}|${sub}${orbitToken}|${actionSuffix}`;
          } else if (activeResourceId === "science.aria.s1-gunw.v1") {
            keyword = `${startDate}..${endDate}|GUNW${orbitToken}|${actionSuffix}`;
          } else if (activeResourceId === "science.comet.licsar.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["GEOCODED_UNW_COH", "UNW_GEOTIFF", "COH_GEOTIFF"], "GEOCODED_UNW_COH")}${orbitToken}|${actionSuffix}`;
          } else if (activeResourceId === "event.usgs.earthquake.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["2.5", "3.0", "4.5", "5.5", "6.0", "6.5", "7.0"], "4.5")}|${actionSuffix}`;
          } else if (activeResourceId === "geodesy.ngl.gnss.v1") {
            keyword = isExplicitDownload ? `${aoiLabel || ""}|download` : `${aoiLabel || ""}|search`;
          } else if (activeResourceId === "ionospheric.igs.gim.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["COD", "IGS", "JPL", "ESA"], "COD")}|${actionSuffix}`;
          } else if (activeResourceId === "science.gee.catalog.v1") {
            const bandSeg =
              toolOpts.geeSelectedBand && toolOpts.geeSelectedBand.trim()
                ? `|${toolOpts.geeSelectedBand.trim()}`
                : "";
            keyword = geeIsStaticAsset
              ? `${effectiveGeeAssetId}${bandSeg}||${actionSuffix}`
              : `${effectiveGeeAssetId}|${startDate}..${endDate}${bandSeg}|${actionSuffix}`;
          } else if (activeResourceId === "orbit.nisar.oe.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["ALL", "POE", "MOE", "NOE", "FOE"], "ALL")}|${actionSuffix}`;
          } else if (activeResourceId === "orbit.copernicus.pod.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["ALL", "S2", "S3"], "ALL")}|${actionSuffix}`;
          } else if (activeResourceId === "science.nasa.black-marble.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["VNP46A3", "VNP46A4", "VNP46A2"], "VNP46A3")}|${actionSuffix}`;
          } else if (activeResourceId === "science.nasa.emit.l2a.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["EMITL2ARFL", "EMITL2BMIN"], "EMITL2ARFL")}|${actionSuffix}`;
          } else if (activeResourceId === "science.nasa.ecostress.l2t.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["ECO_L2T_LSTE", "ECO_L2T_LSTE_EMIS", "ECO_L3T_JET"], "ECO_L2T_LSTE")}|${actionSuffix}`;
          } else if (activeResourceId === "science.dlr.enmap.l2a.v1") {
            keyword = `${startDate}..${endDate}|ENMAP_HSI_L2A|search`;
          } else if (activeResourceId === "science.aws.sentinel-1-grd.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["vv", "vh", "hh", "hv"], "vv")}|${actionSuffix}`;
          } else if (activeResourceId === "science.noaa.aws-weather.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["gfs", "goes19", "goes18"], "gfs")}|${actionSuffix}`;
          } else if (activeResourceId === "science.overture.maps.v1") {
            keyword = `${startDate}..${endDate}|${pickSub(["buildings", "places", "transportation", "divisions", "addresses", "base"], "buildings")}|${actionSuffix}`;
          } else if (
            dedicatedDataset === "nisar_products" ||
            dedicatedDataset === "bhoonidhi_sband_products"
          ) {
            const tierSpec =
              dedicatedDataset === "nisar_products"
                ? `|tier:${activeNisarTiers.join(",")}`
                : "";
            keyword = `${startDate}..${endDate}|${activeNisarLevel || "GCOV"}${tierSpec}${orbitToken}|${actionSuffix}`;
          }

          const isAwsOpenGeodataDataset =
            dedicatedDataset === "aws_sentinel1_grd" ||
            dedicatedDataset === "noaa_aws_weather" ||
            dedicatedDataset === "overture_maps";
          const res: any = await queryOpenGeodata({
            dataset: dedicatedDataset,
            bbox: [resolvedAoi.west, resolvedAoi.south, resolvedAoi.east, resolvedAoi.north],
            aoi_geojson: resolvedGeometry ?? undefined,
            lat: (resolvedAoi.south + resolvedAoi.north) / 2,
            lng: (resolvedAoi.west + resolvedAoi.east) / 2,
            date: geeIsStaticAsset ? undefined : queryDate,
            start_date: geeIsStaticAsset ? undefined : (startDate || undefined),
            end_date: geeIsStaticAsset ? undefined : (endDate || undefined),
            code:
              activeResourceId === "science.gee.catalog.v1"
                ? effectiveGeeAssetId
                : undefined,
            osm_tag: explicitTargetItemId || undefined,
            keyword,
            tool_name: isAwsOpenGeodataDataset
              ? (isExplicitDownload ? "download" : "search")
              : undefined,
            output_path:
              isAwsOpenGeodataDataset && !isExplicitDownload ? undefined : targetOut,
            sql: geeRoute.sql,
            ...(dedicatedDataset === "gee_catalog"
              ? {
                  gee_params: {
                    use_availability_check: true,
                    max_cloud: toolOpts.geeMaxCloud ?? 30,
                    band: toolOpts.geeSelectedBand || undefined,
                    sar_polarization: toolOpts.geeSarPolarization || undefined,
                    orbit_direction: orbitDirection || undefined,
                    reducer: toolOpts.geeTemporalStat || "median",
                  },
                }
              : {}),
          });
          if (!isCurrentSearch()) return false;
          if (res && res.ok) {
            const mapped = mapOpenGeodataResponseToScenes(activeResourceId, res, resolvedAoi, endDate);
            if (isExplicitDownload) {
              await taskOverviewSource.refreshNow();
              const savedPath = String(
                res.downloaded_file || res.downloaded_path || res.output_path || "",
              );
              const completedId = String(
                res.target_item_id || explicitTargetItemId || mapped[0]?.scene_id || "",
              );
              const affectedRowIds = Array.from(
                new Set(
                  [completedId, explicitTargetItemId]
                    .map((id) => String(id || "").trim())
                    .filter(Boolean),
                ),
              );
              const realTaskId =
                typeof res.task_id === "string" && res.task_id.trim()
                  ? res.task_id.trim()
                  : `geodata-${completedId}`;
              if (scenesRef.current.length > 0 && affectedRowIds.length > 0) {
                setScenes((prev) =>
                  prev.map((sc) =>
                    affectedRowIds.includes(sc.scene_id)
                      ? {
                          ...sc,
                          native_metadata: {
                            ...(sc.native_metadata || {}),
                            already_downloaded: true,
                            local_file_path: savedPath || null,
                            downloaded_dir: targetOut || null,
                          },
                        }
                      : sc,
                  ),
                );
              } else {
                setScenes(mapped);
                setBatchTotalCount(mapped.length);
              }
              if (affectedRowIds.length > 0) {
                setSelectedSceneId(affectedRowIds[0]);
                setRowStates((prev) => {
                  const next = new Map(prev);
                  for (const rowId of affectedRowIds) {
                    next.set(rowId, {
                      item_id: rowId,
                      task_id: realTaskId,
                      task_status: "completed",
                      item_status: "completed",
                      actions: [],
                    });
                  }
                  return next;
                });
                setCheckedSceneIds((prev) => {
                  if (!affectedRowIds.some((rowId) => prev.has(rowId))) return prev;
                  const next = new Set(prev);
                  for (const rowId of affectedRowIds) {
                    next.delete(rowId);
                  }
                  return next;
                });
              }
              const localizedActiveSourceName = localizedSourceDisplayName(activeResourceId, locale);
              const doneMsg =
                res.already_exists === true || res.task_already_recorded === true
                  ? translate("mapWorkbench.desktop.dataAlreadyExistsInDir", { dir: targetOut || "" })
                  : translate("mapWorkbench.desktop.dedicatedDownloadSaved", {
                      source: localizedActiveSourceName,
                      path: savedPath,
                    });
              setInlineToolResult({ ok: true, message: doneMsg });
              setTaskOutcomeNotice({ tone: "neutral", text: doneMsg });
              finishSearchBusy();
              return Boolean(savedPath);
            } else {
              activateNonBatchSceneList(mapped);
              const nextRows = new Map<string, RowTaskState>();
              for (const sc of mapped) {
                const meta = (sc.native_metadata || {}) as Record<string, unknown>;
                if (meta.already_downloaded === true) {
                  nextRows.set(sc.scene_id, {
                    item_id: sc.scene_id,
                    task_id: `geodata-${sc.scene_id}`,
                    task_status: "completed",
                    item_status: "completed",
                    actions: [],
                  });
                }
              }
              setRowStates(nextRows);
              const localizedActiveSourceName = localizedSourceDisplayName(activeResourceId, locale);
              const overlay = extractGeodataRasterOverlayFromResponse(
                res,
                resolvedAoi,
                activeResourceId === "science.gee.catalog.v1"
                  ? `GEE (${effectiveGeeAssetId})`
                  : localizedActiveSourceName,
              );
              if (overlay) {
                setGeodataRasterOverlay(overlay);
              }
              const authHint =
                mapped.length > 0 &&
                (activeResourceId === "science.opera.disp-s1.v1"
                  || activeResourceId === "science.aria.s1-gunw.v1"
                  || activeResourceId === "science.nasa.black-marble.v1"
                  || activeResourceId === "science.nasa.emit.l2a.v1"
                  || activeResourceId === "science.nasa.ecostress.l2t.v1")
                  ? translate("mapWorkbench.desktop.authHintEarthdataScience")
                  : mapped.length > 0 && activeResourceId === "science.dlr.enmap.l2a.v1"
                  ? translate("mapWorkbench.desktop.authHintDlrEnmap")
                  : mapped.length > 0 && activeResourceId === "orbit.nisar.oe.v1"
                  ? translate("mapWorkbench.desktop.authHintNisarOrbit")
                  : mapped.length > 0 && activeResourceId === "orbit.copernicus.pod.v1"
                  ? translate("mapWorkbench.desktop.authHintCdsePod")
                  : "";
              const emptyReason =
                locale === "zh-CN"
                  ? (typeof res.empty_reason_zh === "string" && res.empty_reason_zh.trim()
                      ? res.empty_reason_zh.trim()
                      : null)
                  : (typeof res.empty_reason_en === "string" && res.empty_reason_en.trim()
                      ? res.empty_reason_en.trim()
                      : null);
              const effectiveSourceTitle =
                (dedicatedDataset === "nisar_products" || dedicatedDataset === "bhoonidhi_sband_products") &&
                activeNisarLevel
                  ? translate("mapWorkbench.desktop.nisarBandLevelTitle", {
                      band: translate(
                        isNisarSBandResourceId(activeResourceId)
                          ? "mapWorkbench.desktop.nisarSBandLabel"
                          : "mapWorkbench.desktop.nisarLBandLabel",
                      ),
                      level: activeNisarLevel,
                    })
                  : localizedActiveSourceName;
              const summaryMsg =
                mapped.length === 0
                  ? (emptyReason || translate("mapWorkbench.desktop.dedicatedSearchEmpty", { source: effectiveSourceTitle }))
                  : translate("mapWorkbench.desktop.dedicatedSearchMatched", {
                      source: effectiveSourceTitle,
                      count: mapped.length,
                      hint: authHint,
                    });
              setInlineToolResult({ ok: true, message: summaryMsg });
              setTaskOutcomeNotice({ tone: "neutral", text: summaryMsg });
              finishSearchBusy();
              return true;
            }
          } else {
            const errMsg = String(
              res?.error ||
                translate("mapWorkbench.desktop.dedicatedSearchFailed", {
                  source: localizedSourceDisplayName(activeResourceId, locale),
                }),
            );
            setInlineToolResult({ ok: false, message: errMsg });
            setTaskOutcomeNotice({ tone: "danger", text: errMsg });
            setSearchError(errMsg);
            finishSearchBusy();
            return false;
          }
        }

        const effectiveAoiName = aoiLabel && aoiLabel !== translate("mapWorkbench.desktop.globalView") ? aoiLabel : translate("mapWorkbench.desktop.mapSelectedExtent");
        const searchReq = buildUnifiedSearchRequest({
          resourceId: activeResourceId,
          aoiGeometry: resolvedGeometry,
          aoiName: effectiveAoiName,
          startDate,
          endDate,
          platforms,
          productType,
          polarizations,
          orbitDirection,
          relativeOrbit,
          path,
          frame,
          beamModes,
          lookDirection,
          opticalDraft: opticalDrafts[activeResourceId],
        });

        clearedSourceBatchesRef.current.delete(activeResourceId);
        const progress = await searchStart(
          searchReq,
          bypassCache ? { bypassCache: true } : undefined,
        );
        if (!isCurrentSearch()) {
          await cancelCatalogSearch(progress.request_id).catch(() => {});
          return false;
        }
        searchRequestsRef.current.set(progress.request_id, searchReq);
        setSearchProgress(progress);

        // 统一轮询器驱动检索进度 (250ms，事件驱动)
        activeSearchIdRef.current = progress.request_id;
        pollerRef.current?.startSearch(progress.request_id);
      }
      // 模式 2: DEM 瓦片规划 (A3)
      else if (scientificMode === "dem") {
        if (!workingAoi) {
          const errMsg = translate("mapWorkbench.desktop.aoiRequired");
          setSearchError(errMsg);
          setInlineToolResult({ ok: false, message: errMsg });
          finishSearchBusy();
          return false;
        }

        const currentDemProvider = getActiveDatasetProvider(demSource === "COP30" ? "COP30" : activeResourceId);
        if (currentDemProvider === "gee") {
          const geeAsset = "COPERNICUS/DEM/GLO30";
          updateWorkbenchToolOptions({ geeAssetId: geeAsset });
          const geeRoute = await resolveGeeSqlRouteMode(true);
          if (!geeRoute.ok) {
            setInlineToolResult({ ok: false, message: geeRoute.error });
            setSearchError(geeRoute.error);
            finishSearchBusy();
            return false;
          }
          const toolOpts = getWorkbenchToolOptions();
          if (!explicitOverrideOutputDir) {
            const prefillDemDir = resolveConfiguredNonTmpDir(toolOpts.demOutputDir) || "";
            setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
              sourceBatchId: "geodata-direct-dem-gee",
              sourceBatchRevision: 0,
              selectionScope: "exact_item",
              selectedProductIds: [geeAsset],
              acquisition: {
                kind: "generic_sar",
                frozenParameters: {
                  dedicated_geodata: true,
                  dedicated_action: "dem_download",
                  dem_provider: "gee",
                  dataset_label: "GEE 高程切片 (COPERNICUS/DEM/GLO30)",
                  resource_id: activeResourceId,
                  estimated_total_bytes: 0,
                },
              },
              destinationPrefill: prefillDemDir,
              layout: "direct",
              taskPolicy: sarDownloadPolicy?.mode || "default",
              concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
            }));
            finishSearchBusy();
            return true;
          }
          const targetDemDir = explicitOverrideOutputDir;
          const keyword = `${geeAsset}|download`;
          const res: any = await queryOpenGeodata({
            dataset: "gee_catalog",
            bbox: [workingAoi.west, workingAoi.south, workingAoi.east, workingAoi.north],
            lat: (workingAoi.south + workingAoi.north) / 2,
            lng: (workingAoi.west + workingAoi.east) / 2,
            code: geeAsset,
            keyword,
            output_path: targetDemDir,
            sql: geeRoute.sql,
          });
          if (!isCurrentSearch()) return false;
          if (res && res.ok) {
            await taskOverviewSource.refreshNow();
            const mapped = mapOpenGeodataResponseToScenes("dem.cop30.v1", res, workingAoi, endDate || "");
            setScenes(mapped);
            setBatchTotalCount(mapped.length);
            setSelectedSceneId(mapped[0]?.scene_id ?? null);
            const savedPath = String(
              res.downloaded_file || res.downloaded_path || res.output_path || res.output_dir || "",
            );
            const realTaskId =
              typeof res.task_id === "string" && res.task_id.trim()
                ? res.task_id.trim()
                : `gee-dem-task-${Date.now()}`;
            const nextRows = new Map<string, RowTaskState>();
            if (savedPath && mapped.length > 0) {
              nextRows.set(mapped[0].scene_id, {
                item_id: mapped[0].scene_id,
                task_id: realTaskId,
                task_status: "completed",
                item_status: "completed",
                actions: [],
              });
            }
            setRowStates(nextRows);
            const overlay = extractGeodataRasterOverlayFromResponse(
              res,
              workingAoi,
              `GEE 高程 (${geeAsset})`,
            );
            if (overlay) {
              setGeodataRasterOverlay(overlay);
            }
            const okMsg =
              res.already_exists === true || res.task_already_recorded === true
                ? `该高程切片已存在于目标目录 (${targetDemDir})，无需重复下载`
                : `Google Earth Engine (${geeAsset})：高程裁剪已加入下载中心并落盘至 ${savedPath}`;
            setInlineToolResult({ ok: true, message: okMsg });
            setTaskOutcomeNotice({ tone: "neutral", text: okMsg });
            setFailoverBanner(null);
          } else {
            const errMsg = String(res?.error || `GEE 高程提取失败`);
            setInlineToolResult({ ok: false, message: errMsg });
            setSearchError(errMsg);
          }
          finishSearchBusy();
          return Boolean(res?.ok);
        }

        const toolOpts = getWorkbenchToolOptions();
        let targetDemDir = resolveConfiguredNonTmpDir(toolOpts.demOutputDir);
        if (!explicitOverrideOutputDir && !targetDemDir) {
          pendingStorageResumeRef.current = { type: "dem_plan" };
          setStorageRequiredPrompt(true);
          finishSearchBusy();
          return false;
        }
        const plan = await planDemDownloadBbox(
          workingAoi.west,
          workingAoi.east,
          workingAoi.south,
          workingAoi.north,
          targetDemDir,
          demSource
        );
        if (!isCurrentSearch()) return false;
        if (plan && plan.ok) {
          const p = (plan.plan || {}) as Record<string, unknown>;
          const lonSpan = Math.max(1, Math.floor(workingAoi.east) - Math.floor(workingAoi.west) + 1);
          const latSpan = Math.max(1, Math.floor(workingAoi.north) - Math.floor(workingAoi.south) + 1);
          const tileCount = typeof p.tile_count === "number" ? p.tile_count : lonSpan * latSpan;
          const planMsg = toolResultLine("dem", { ...(plan as Record<string, unknown>), tile_count: tileCount });
          updateWorkbenchToolOptions({
            lastDemPlanSummary: {
              ok: true,
              dataset: String(p.dataset || demSource),
              tileCount,
              sourceDatum: String(
                p.source_vertical_datum ||
                  (demSource.startsWith("COP") || demSource === "FABDEM"
                    ? "EGM2008"
                    : demSource === "NISAR_DEM" || demSource.endsWith("_ELLIPSOIDAL")
                      ? "WGS84_ELLIPSOID"
                      : "EGM96"),
              ),
              targetDatum: autoConvertEllipsoid ? "WGS84_ELLIPSOID" : String(p.source_vertical_datum || "EGM2008"),
              rawDemPath: String(p.raw_dem_path || ""),
              sarscapeDemPath: autoConvertEllipsoid ? String(p.sarscape_ready_dem_path || "") : "",
              outputDir: targetDemDir,
            },
          });
          setInlineToolResult({ ok: true, message: planMsg });
          if (!explicitOverrideOutputDir) {
            setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
              sourceBatchId: `geodata-direct-dem-${demSource}`,
              sourceBatchRevision: 0,
              selectionScope: "exact_item",
              selectedProductIds: [`dem-${demSource}-${tileCount}tiles`],
              acquisition: {
                kind: "generic_sar",
                frozenParameters: {
                  dedicated_geodata: true,
                  dedicated_action: "dem_download",
                  dem_provider: "standard",
                  dem_source: demSource,
                  auto_convert_ellipsoid: autoConvertEllipsoid,
                  plan_msg: planMsg,
                  dataset_label: `${demSource} DEM (${tileCount} 块)`,
                  resource_id: activeResourceId,
                  estimated_total_bytes: 0,
                },
              },
              destinationPrefill: targetDemDir || "",
              layout: "direct",
              taskPolicy: sarDownloadPolicy?.mode || "default",
              concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
            }));
            finishSearchBusy();
            return true;
          }
          targetDemDir = explicitOverrideOutputDir;
          if (targetDemDir) {
            let startRes = await startDemDownloadBbox(
              workingAoi.west,
              workingAoi.east,
              workingAoi.south,
              workingAoi.north,
              targetDemDir,
              demSource,
              "auto",
              autoConvertEllipsoid
            );
            if (startRes && !startRes.ok && (startRes as any).code === "DEM_AOI_BBOX_STALE") {
              startRes = await startDemDownload(
                targetDemDir,
                demSource,
                "auto",
                autoConvertEllipsoid
              );
            }
            if (startRes && startRes.ok) {
              const okMsg = demStartResultLine(startRes as Record<string, unknown>, planMsg);
              setInlineToolResult({ ok: true, message: okMsg });
              void taskOverviewSource.refreshNow().catch(() => {});
              setDownloadDrawerOpen(true);
            } else {
              const failMsg = demStartResultLine(startRes as Record<string, unknown>, planMsg);
              setInlineToolResult({ ok: false, message: failMsg });
            }
          }
        } else if (plan && !plan.ok) {
          const msg = toolResultLine("dem", plan as Record<string, unknown>);
          setInlineToolResult({ ok: false, message: msg });
          if ((demSource === "COP30" || activeResourceId === "dem.cop30.v1") && isUpstreamDemError(plan)) {
            setFailoverBanner({
              datasetKey: "COP30",
              message: "AWS Open Data 当前检索受阻，可点击切换至备用数据源通道（OpenTopography / GEE）重新检索。",
              options: [
                { provider: "opentopography", label: "切换至 OpenTopography" },
                { provider: "gee", label: "切换至 GEE" },
              ],
            });
          }
        }
        finishSearchBusy();
      }
      // 模式 3: 精密轨道匹配 (A3)
      else if (scientificMode === "orbit") {
        if (orbitSourceType === "current") {
          const targetScenes = checkedScenes.length > 0 ? checkedScenes : scenes;
          if (targetScenes.length === 0) {
            const errMsg = translate("mapWorkbench.desktop.noScenesForOrbit");
            setInlineToolResult({ ok: false, message: errMsg });
            finishSearchBusy();
            return;
          }
          if (!explicitOverrideOutputDir) {
            const prefillOrbitDir =
              nonTemporaryOutputDir(getWorkbenchToolOptions().orbitOutputDir) ||
              nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
              nonTemporaryOutputDir(storageRootPath) ||
              "";
            setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
              sourceBatchId: "geodata-direct-orbit-mode",
              sourceBatchRevision: 0,
              selectionScope: targetScenes.length > 1 ? "checked" : "exact_item",
              selectedProductIds: targetScenes.map((s) => s.scene_id),
              acquisition: {
                kind: "generic_sar",
                frozenParameters: {
                  dedicated_geodata: true,
                  dedicated_action: "orbit_download",
                  dataset_label: "Sentinel-1 精密轨道 (POEORB)",
                  resource_id: activeResourceId,
                  estimated_total_bytes: 0,
                },
              },
              destinationPrefill: prefillOrbitDir,
              layout: "direct",
              taskPolicy: sarDownloadPolicy?.mode || "default",
              concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
            }));
            finishSearchBusy();
            return true;
          }
          const targetOrbitDir = explicitOverrideOutputDir;
          if (targetOrbitDir) {
            const startRes = await startOrbitDownloadSnapshot(
              targetOrbitDir,
              targetScenes,
              targetScenes.map((s) => s.scene_id),
              sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks
            );
            if (startRes && startRes.ok) {
              const okMsg = orbitDownloadResultLine(startRes as Record<string, unknown>, targetScenes.length);
              setInlineToolResult({ ok: true, message: okMsg });
              void taskOverviewSource.refreshNow().catch(() => {});
              setDownloadDrawerOpen(true);
            } else {
              const failMsg = orbitDownloadResultLine(startRes as Record<string, unknown>, targetScenes.length);
              setInlineToolResult({ ok: false, message: failMsg });
            }
          }
        } else {
          if (!localSlcPath) {
            const errMsg = translate("mapWorkbench.desktop.chooseLocalSlcRequired");
            setSearchError(errMsg);
            setInlineToolResult({ ok: false, message: errMsg });
            finishSearchBusy();
            return;
          }
          const scanRes = (await queryOpenGeodata({
            dataset: "scan_local_slc_dates",
            code: localSlcPath,
          })) as {
            ok?: boolean;
            source_kind?: string;
            s1_scenes?: string[];
            error?: string;
          } | null;
          if (!scanRes || !scanRes.ok) {
            const errMsg = scanRes?.error || "未能扫描本地 SLC 目录或 ASF 脚本中的 Sentinel-1 影像。";
            setSearchError(errMsg);
            setInlineToolResult({ ok: false, message: errMsg });
            finishSearchBusy();
            return;
          }
          const scannedScenes = buildLocalSlcCandidateScenes(scanRes.s1_scenes ?? []);
          if (scannedScenes.length === 0) {
            const errMsg = "未在所选目录或脚本中识别出 Sentinel-1 (S1A/S1B/S1C/S1D) 影像名称。";
            setSearchError(errMsg);
            setInlineToolResult({ ok: false, message: errMsg });
            finishSearchBusy();
            return;
          }
          const toolOpts = getWorkbenchToolOptions();
          const matchDir =
            scanRes.source_kind === "script_or_manifest"
              ? nonTemporaryOutputDir(toolOpts.orbitOutputDir) ||
                parentDirFromPath(localSlcPath) ||
                nonTemporaryOutputDir(storageRootPath) ||
                ""
              : localSlcPath;
          const res = matchDir
            ? await matchOrbitsDirectory(matchDir, scannedScenes.slice(0, 100))
            : ({ ok: false, error: "请选择有效的本地轨道目录" } as const);
          if (res && res.ok) {
            const report = res.report as { matched?: number; unmatched?: number; results?: Array<{ scene_id?: string; matched?: boolean }> } | undefined;
            let msg = toolResultLine("orbit", res as Record<string, unknown>);
            if (report && typeof report.unmatched === "number" && report.unmatched > 0) {
              const missingIds = Array.isArray(report.results)
                ? report.results.filter((r) => r && r.matched === false && r.scene_id).map((r) => String(r.scene_id))
                : scannedScenes.map((s) => s.scene_id);
              const targetDownloadDir = nonTemporaryOutputDir(toolOpts.orbitOutputDir) || matchDir;
              if (targetDownloadDir && missingIds.length > 0 && workingGeometry && workingAoi) {
                const startRes = await startOrbitDownloadSnapshot(
                  targetDownloadDir,
                  scannedScenes,
                  missingIds.slice(0, 100),
                  sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
                );
                if (startRes && startRes.ok) {
                  msg = `本地已匹配 ${report.matched ?? 0} 景，已为缺失的 ${missingIds.length} 景创建精密轨道下载任务。`;
                  void taskOverviewSource.refreshNow().catch(() => {});
                } else {
                  msg = `本地已匹配 ${report.matched ?? 0} 景，尚缺 ${report.unmatched} 景精密轨道。`;
                }
              } else {
                msg = `本地已匹配 ${report.matched ?? 0} 景，尚缺 ${report.unmatched} 景精密轨道。`;
              }
            }
            setInlineToolResult({ ok: true, message: msg });
            setDownloadDrawerOpen(true);
          } else if (res && !res.ok) {
            const msg = toolResultLine("orbit", res as Record<string, unknown>);
            setInlineToolResult({ ok: false, message: msg });
          }
        }
        finishSearchBusy();
      }
      // 模式 4: 本地 DEM 基准转换 (A3)
      else if (scientificMode === "convert") {
        if (!localDemPath) {
          const errMsg = translate("mapWorkbench.desktop.chooseLocalDemRequired");
          setSearchError(errMsg);
          setInlineToolResult({ ok: false, message: errMsg });
          finishSearchBusy();
          return;
        }
        const toolOpts = getWorkbenchToolOptions();
        const resolvedDatum = resolveEffectiveDemSourceDatum(localDemPath, toolOpts.convertSourceDatum);
        const effectiveOut =
          toolOpts.convertOutputDir.trim() ||
          parentDirFromPath(localDemPath) ||
          storageRootPath.trim();
        const res = await runLocalDemConversion(
          localDemPath,
          effectiveOut,
          resolvedDatum.datum,
          toolOpts.convertOutputMode
        );
        if (res && res.ok) {
          const primaryOutput =
            (toolOpts.convertOutputMode === "ellipsoid"
              ? res.ellipsoid_dem_path
              : res.sarscape_ready_dem_path) ||
            res.ellipsoid_dem_path ||
            effectiveOut;
          updateWorkbenchToolOptions({
            lastConvertSummary: {
              ok: true,
              completed: true,
              sourceDatum: resolvedDatum.datum,
              targetDatum: "WGS84_ELLIPSOID",
              geoidModel:
                resolvedDatum.datum === "EGM96"
                  ? "EGM96"
                  : resolvedDatum.datum === "EGM2008"
                  ? "EGM2008"
                  : null,
              requiresConversion: resolvedDatum.datum !== "WGS84_ELLIPSOID",
              message: "高程转换已成功完成并写入磁盘",
              rawDemPath: String(res.raw_dem_path || localDemPath),
              ellipsoidDemPath: String(res.ellipsoid_dem_path || ""),
              sarscapeDemPath:
                toolOpts.convertOutputMode === "ellipsoid"
                  ? ""
                  : String(res.sarscape_ready_dem_path || ""),
              outputDir: effectiveOut || parentDirFromPath(String(primaryOutput || localDemPath)),
            },
          });
          const msg = toolResultLine("convert", {
            ...(res as Record<string, unknown>),
            output_path: primaryOutput,
          });
          setInlineToolResult({ ok: true, message: msg });
        } else if (res && !res.ok) {
          const msg = toolResultLine("convert", res as Record<string, unknown>);
          setInlineToolResult({ ok: false, message: msg });
        }
        finishSearchBusy();
      }
    } catch (err: any) {
      const errorObj = err?.error || err;
      const codeStr = errorObj?.code;
      const code = codeStr ? `[${codeStr}] ` : "";
      const msg = preferCodeExplanation(errorObj?.message_zh, codeStr) || errorObj?.message || translate("mapWorkbench.desktop.searchServiceFailed");
      if (scientificMode !== "sar" || activeResourceIdRef.current === activeResourceId) {
        const isOpticalFailover =
          activeResourceId === "science.sentinel-2.l2a.v1" ||
          activeResourceId === "science.sentinel-2.l1c.v1" ||
          activeResourceId === "science.landsat.c2-l2.v1";
        if (isOpticalFailover && trySilentOpticalFallback(activeResourceId)) {
          // 静默切换备用光学数据源并重试
        } else {
          setSearchError(`${code}${msg}`);
        }
      }
      if (scientificMode !== "sar") {
        const toolMsg = toolResultLine(scientificMode as any, { ok: false, error: err });
        setInlineToolResult({ ok: false, message: toolMsg });
        if (scientificMode === "dem" && (demSource === "COP30" || activeResourceId === "dem.cop30.v1") && isUpstreamDemError(err)) {
          setFailoverBanner({
            datasetKey: "COP30",
            message: "AWS Open Data 当前检索受阻，可点击切换至备用数据源通道（OpenTopography / GEE）重新检索。",
            options: [
              { provider: "opentopography", label: "切换至 OpenTopography" },
              { provider: "gee", label: "切换至 GEE" },
            ],
          });
        }
      }
      finishSearchBusy();
    }
  }, [
    scientificMode,
    activeResourceId,
    workingAoi,
    workingGeometry,
    startDate,
    endDate,
    platforms,
    productType,
    polarizations,
    orbitDirection,
    relativeOrbit,
    path,
    frame,
    beamModes,
    lookDirection,
    opticalDrafts,
    demSource,
    autoConvertEllipsoid,
    orbitSourceType,
    localSlcPath,
    localDemPath,
    scenes,
    aoiLabel,
    sarDownloadPolicy,
    storageRootPath,
    rowStates,
    taskOverviewSource,
    trySilentOpticalFallback,
    checkedScenes,
    batchFor,
    locale,
    translate,
  ]);
  triggerSearchRef.current = handleTriggerSearch;

  const handleFailoverSwitch = useCallback((targetProvider: string) => {
    const effectiveMultiId =
      scientificMode === "dem"
        ? (demSource === "COP30" ? "COP30" : activeResourceId)
        : activeResourceId;
    setActiveDatasetProvider(effectiveMultiId, targetProvider);
    setFailoverBanner(null);
    setSearchError(null);
    window.setTimeout(() => {
      void handleTriggerSearch();
    }, 50);
  }, [scientificMode, demSource, activeResourceId, handleTriggerSearch]);

  // 规则 3：行级任务操作与状态受控于 RowTaskState；建任务立即返回 TaskRef，不阻塞界面
  // 打开任务的保存目录：经任务保存目录命令（application_open_download_task_output），失败抛给调用方显示中文原因。
  // 审计 P0：演示数据绝不触达本机文件系统。
  const handleOpenTaskOutput = useCallback(async (ref: TaskOutputRef) => {
    if (isFixtureBackend() || !isTauriDesktopHost()) return;
    const result = await openApplicationTaskOutput({
      task_id: ref.task_id,
      expected_revision: ref.revision,
    });
    if (!result.ok) throw result;
  }, []);

  const findAgentBatchEntry = useCallback(
    (targetBatchId: string | null | undefined): AgentSessionBatchEntry | null => {
      if (!targetBatchId) return null;
      const ws = peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current);
      if (!ws) return null;
      const inList = ws.batches?.find((b) => b.batchId === targetBatchId) ?? null;
      if (inList) return inList;
      if (ws.batchId === targetBatchId) {
        return {
          batchId: ws.batchId,
          resourceId: ws.resourceId,
          revision: 0,
          scenes: ws.scenes ?? [],
          rowStates: ws.rowStates ?? new Map<string, RowTaskState>(),
          totalCount: ws.batchTotalCount ?? (ws.scenes?.length ?? 0),
          updatedAtMs: 0,
        };
      }
      return null;
    },
    []
  );

  const buildOpticalDownloadDraft = useCallback(
    async ({
      resourceId,
      batchId,
      revision,
      selectionScope,
      selectedProductIds,
      outputRoot,
      concurrency,
      layout,
      targetScenes,
      selectedProductCount,
      excludedProductIds,
      allMatchPreview,
    }: {
      resourceId: string;
      batchId: string;
      revision: number;
      selectionScope: "checked" | "exact_item" | "all_matching";
      selectedProductIds: string[];
      outputRoot: string;
      concurrency: number;
      layout: DownloadAcquisitionDraft["layout"];
      targetScenes: readonly SceneRow[];
      selectedProductCount?: number;
      excludedProductIds?: string[];
      allMatchPreview?: Parameters<typeof createDownloadAcquisitionDraft>[0]["allMatchPreview"];
    }): Promise<DownloadAcquisitionDraft> => {
      if (resourceId === "science.hls.l30.v2" || resourceId === "science.hls.s30.v2") {
        const isS30 = resourceId === "science.hls.s30.v2";
        const allBands = isS30 ? [...HLS_S30_SUPPORTED_BANDS] : [...HLS_L30_SUPPORTED_BANDS];
        return createDownloadAcquisitionDraft({
          sourceBatchId: batchId,
          sourceBatchRevision: revision,
          selectionScope,
          selectedProductIds,
          ...(selectedProductCount !== undefined ? { selectedProductCount } : {}),
          ...(excludedProductIds ? { excludedProductIds } : {}),
          ...(allMatchPreview ? { allMatchPreview } : {}),
          acquisition: {
            kind: "hls",
            frozenParameters: {
              hls_product: isS30 ? "S30" : "L30",
              band_preset: "all_bands",
              selected_band_keys: allBands,
            },
          },
          destinationPrefill: outputRoot,
          layout: "direct",
          taskPolicy: sarDownloadPolicy?.mode || "default",
          concurrency,
        });
      }
      const isL1c = resourceId === "science.sentinel-2.l1c.v1";
      const cdseAvailable = await resolveCdseSafeAvailability();
      const hasExplicitEarthSearchScene = targetScenes.some(
        (s) =>
          typeof s.provider === "string"
          && s.provider.includes("element84.earth-search"),
      );
      const hasExplicitMpcScene = targetScenes.some(
        (s) =>
          typeof s.provider === "string"
          && (s.provider.includes("microsoft.planetary-computer")
            || s.provider === "mpc"
            || s.provider.includes("element84.earth-search")),
      );
      const hasExplicitNonMpcScene = targetScenes.some(
        (s) =>
          typeof s.provider === "string"
          && s.provider.trim().length > 0
          && !s.provider.includes("microsoft.planetary-computer")
          && !s.provider.includes("element84.earth-search")
          && s.provider !== "mpc",
      );
      const isMpcProvider =
        !isL1c
        && (hasExplicitMpcScene
          || (!hasExplicitNonMpcScene && getActiveDatasetProvider("science.sentinel-2.l2a.v1") === "mpc"));
      const safeArchiveAvailable = cdseAvailable && !hasExplicitEarthSearchScene;
      const defaultDeliveryForm =
        safeArchiveAvailable && !isMpcProvider && !isL1c ? "safe_archive" : "analysis_bands";
      const allBands = isL1c
        ? SENTINEL2_L1C_BANDS.map((b) => b.key)
        : SENTINEL2_REFLECTANCE_BANDS.map((b) => b.key);
      return createDownloadAcquisitionDraft({
        sourceBatchId: batchId,
        sourceBatchRevision: revision,
        selectionScope,
        selectedProductIds,
        ...(selectedProductCount !== undefined ? { selectedProductCount } : {}),
        ...(excludedProductIds ? { excludedProductIds } : {}),
        ...(allMatchPreview ? { allMatchPreview } : {}),
        acquisition: {
          kind: "sentinel2",
          frozenParameters: {
            product_level: isL1c ? "l1c" : "l2a",
            delivery_form: defaultDeliveryForm,
            safe_archive_available: safeArchiveAvailable,
            analysis_bands_available: true,
            band_preset: "all_reflectance",
            selected_band_keys: allBands,
          },
        },
        destinationPrefill: outputRoot,
        layout: defaultDeliveryForm === "safe_archive" ? layout : "direct",
        taskPolicy: sarDownloadPolicy?.mode || "default",
        concurrency,
      });
    },
    [sarDownloadPolicy?.mode],
  );

  const executeDownloadTask = useCallback(async ({
    batchId,
    revision,
    itemIds,
    outputRoot,
    concurrency,
    layout,
    includePreciseOrbit,
    includeDem,
    includeGacos,
    scope,
    excludedItemIds,
    selectionDigest,
    admissibleCount,
    overrideResourceId,
    opticalDelivery,
  }: {
    batchId: string;
    revision: number;
    itemIds: string[];
    outputRoot: string;
    concurrency: number;
    layout: DownloadAcquisitionDraft["layout"];
    includePreciseOrbit?: boolean;
    includeDem?: boolean;
    includeGacos?: boolean;
    scope: "row" | "checked" | "all_matching";
    excludedItemIds?: string[];
    selectionDigest?: string;
    admissibleCount?: number;
    overrideResourceId?: string | null;
    opticalDelivery?: OpticalConfirmationDelivery;
  }) => {
    const generation = sourceSwitchGenerationRef.current;
    const originAgentSessionId =
      effectivePageRouteRef.current === "agent" ? activeAgentSessionIdRef.current : null;
    const agentBatchEntry = originAgentSessionId ? findAgentBatchEntry(batchId) : null;
    const isCurrentRequest = () => {
      if (originAgentSessionId) {
        return activeAgentSessionIdRef.current === originAgentSessionId;
      }
      const current = batchFor(activeResourceIdRef.current);
      return generation === sourceSwitchGenerationRef.current && displayedBatchRef.current === batchId
        && current?.batch_id === batchId && current.revision === revision;
    };
    const itemId = scope === "row" ? itemIds[0] : null;
    const currentResourceId =
      overrideResourceId ?? agentBatchEntry?.resourceId ?? activeResourceIdRef.current;
    const isHlsResource =
      currentResourceId === "science.hls.l30.v2" || currentResourceId === "science.hls.s30.v2";
    const isSentinel2L1c = currentResourceId === "science.sentinel-2.l1c.v1";
    const isSentinel2L2a = currentResourceId === "science.sentinel-2.l2a.v1";
    const agentWs = originAgentSessionId
      ? peekAgentSessionWorkspaceState(originAgentSessionId)
      : null;
    const targetScenesForBatch = itemIds
      .map(
        (id) =>
          scenesRef.current.find((s) => s.scene_id === id || s.product_key === id) ??
          checkedScenesRef.current.get(id) ??
          findAgentSceneBatchOwner(agentWs, id)?.scene
      )
      .filter((s): s is SceneRow => Boolean(s));
    const hasExplicitEarthSearchProvider = targetScenesForBatch.some(
      (s) =>
        typeof s.provider === "string" &&
        s.provider.includes("element84.earth-search"),
    );
    const hasExplicitMpcProvider = targetScenesForBatch.some(
      (s) =>
        typeof s.provider === "string" &&
        (s.provider.includes("microsoft.planetary-computer")
          || s.provider === "mpc"
          || s.provider.includes("element84.earth-search")),
    );
    const hasExplicitNonMpcProvider = targetScenesForBatch.some(
      (s) =>
        typeof s.provider === "string" &&
        s.provider.trim().length > 0 &&
        !s.provider.includes("microsoft.planetary-computer") &&
        !s.provider.includes("element84.earth-search") &&
        s.provider !== "mpc",
    );
    const isMpcSentinel2Resource =
      isSentinel2L2a
      && (hasExplicitMpcProvider
        || (!hasExplicitNonMpcProvider && getActiveDatasetProvider("science.sentinel-2.l2a.v1") === "mpc"));
    const useOpticalBandDelivery =
      opticalDelivery
        ? opticalDelivery.deliveryForm === "analysis_bands" || opticalDelivery.kind === "hls" || hasExplicitEarthSearchProvider
        : isHlsResource || isMpcSentinel2Resource;
    const useMpcSafeArchive =
      opticalDelivery?.deliveryForm === "safe_archive"
      && isMpcSentinel2Resource
      && !hasExplicitEarthSearchProvider;
    const prepId =
      itemId && !useOpticalBandDelivery && !useMpcSafeArchive
        ? null
        : `prep-batch-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
    const prepCount = scope === "all_matching" ? (admissibleCount ?? 1) : (itemIds.length || 1);
    if (itemId) {
      setCreatingItemIds((prev) => withCreatePending(prev, itemId));
    } else {
      setDownloadBusy(true);
      setBatchCreating(true);
    }
    if (prepId) {
      setPendingDownloadPreparations((prev) => [
        {
          id: prepId,
          title: humanPreparationTitle(currentResourceId),
          count: prepCount,
          outputRoot,
          status: "preparing",
        },
        ...prev,
      ]);
    }
    let stopOpticalObserver: (() => void) | null = null;
    try {
      let taskRef: {
        task_id: string;
        revision: number;
        created?: boolean;
        status?: string;
        warning?: string | null;
      } | null = null;
      if (useOpticalBandDelivery || useMpcSafeArchive) {
        if (opticalDelivery?.deliveryForm === "safe_archive") {
          const creds = await getCredentialStatus();
          const ensured = await ensureCdseCredentialSession(creds.ok ? creds.cdse : undefined);
          if (!ensured.ok || !ensured.value.authenticated) {
            throw {
              code: "CDSE_ACCOUNT_LOGIN_REQUIRED",
              message_zh: "需要登录 CDSE 后才能下载完整 SAFE 归档，请先在数据服务账号中配置 Copernicus Data Space 凭据。",
              retryable: false,
              needs_account: "cdse",
              needs_user_confirmation: false,
            } satisfies TypedError;
          }
        }
        const randomId = globalThis.crypto?.randomUUID?.();
        const taskId = `task-ui-${randomId || `${Date.now()}-${Math.random().toString(16).slice(2)}`}`;
        const requestId = isHlsResource
          ? `hls-batch-${taskId}`
          : isSentinel2L1c
            ? `sentinel2-l1c-batch-${taskId}`
            : useMpcSafeArchive
              ? `sentinel2-archive-${taskId}`
              : `sentinel2-batch-${taskId}`;
        if (prepId && isTauriDesktopHost()) {
          stopOpticalObserver = observeOpticalPreparation(
            requestId,
            (rid) => getDesktopShellBridge().application.opticalPreparationProgress(rid),
            (progress) => {
              if (!progress) return;
              setPendingDownloadPreparations((prev) =>
                prev.map((item) =>
                  item.id === prepId
                    ? {
                        ...item,
                        preparedCount: progress.prepared_count,
                        count: progress.requested_count,
                        phase: progress.phase,
                      }
                    : item,
                ),
              );
            },
          );
        }
        const batchContext = {
          batch_id: batchId,
          created_at: new Date().toISOString(),
          aoi_name: aoiLabel,
          aoi_geojson: null,
          query: {},
          page_index: 0,
          page_size: RESULT_PAGE_SIZE,
          revision,
        };
        const chosenBandKeys =
          opticalDelivery?.bandKeys && opticalDelivery.bandKeys.length > 0
            ? opticalDelivery.bandKeys
            : isHlsResource
              ? currentResourceId === "science.hls.s30.v2"
                ? HLS_S30_SUPPORTED_BANDS
                : HLS_L30_SUPPORTED_BANDS
              : isSentinel2L1c
                ? SENTINEL2_L1C_BANDS.map((b) => b.key)
                : SENTINEL2_REFLECTANCE_BANDS.map((b) => b.key);
        const opticalRes = useMpcSafeArchive
          ? itemIds.length === 1 && scope === "row"
            ? await startSentinel2ArchiveDownload(
                outputRoot,
                itemIds[0],
                batchContext,
                taskId,
                requestId,
                concurrency,
                layout === "sar_dataset_processing",
              )
            : await startSentinel2ArchiveBatchDownload(
                outputRoot,
                itemIds,
                batchContext,
                taskId,
                requestId,
                concurrency,
              )
          : isHlsResource
            ? await startHlsBandBatchDownload(
                outputRoot,
                itemIds,
                chosenBandKeys,
                batchContext,
                taskId,
                requestId,
                concurrency,
              )
            : isSentinel2L1c
              ? await startSentinel2L1cBandBatchDownload(
                  outputRoot,
                  itemIds,
                  chosenBandKeys,
                  batchContext,
                  taskId,
                  requestId,
                  concurrency,
                )
              : itemIds.length === 1 && scope === "row"
                ? await startSentinel2BandDownload(
                    outputRoot,
                    itemIds[0],
                    chosenBandKeys,
                    batchContext,
                    taskId,
                    requestId,
                    concurrency,
                  )
                : await startSentinel2BandBatchDownload(
                    outputRoot,
                    itemIds,
                    chosenBandKeys,
                    batchContext,
                    taskId,
                    requestId,
                    concurrency,
                  );
        if (!opticalRes.ok || !opticalRes.task_id) {
          let cdseConfigured: boolean | undefined;
          if (isSentinel2L1c || isSentinel2L2a) {
            const creds = await getCredentialStatus().catch(() => null);
            if (creds?.ok) {
              cdseConfigured = hasSavedProviderLogin(creds.cdse);
            }
          }
          const resolved = resolveOpticalTaskCreationError({
            code: opticalRes.code,
            error: opticalRes.error,
            resourceId: currentResourceId,
            deliveryForm: opticalDelivery?.deliveryForm ?? null,
            cdseConfigured,
          });
          throw {
            ...resolved,
            resolved_message_zh: resolved.message_zh,
          };
        }
        const rawRevision = opticalRes.download_status?.revision;
        const normalizedRevision =
          typeof rawRevision === "number" && Number.isSafeInteger(rawRevision) && rawRevision >= 0
            ? rawRevision
            : 1;
        const rawState = opticalRes.download_status?.state;
        const normalizedStatus =
          typeof rawState === "string"
          && ["queued", "running", "paused", "completed", "failed", "cancelled", "interrupted"].includes(rawState)
            ? rawState
            : "queued";
        const rejectedCount =
          "admission" in opticalRes && opticalRes.admission
            ? (opticalRes.admission.rejected_count ?? 0)
            : 0;
        taskRef = {
          task_id: opticalRes.task_id,
          revision: normalizedRevision,
          created: true,
          status: normalizedStatus,
          ...(rejectedCount > 0
            ? { warning: `部分影像（${rejectedCount} 景）远端签名准备失败，其余影像已入队下载。` }
            : {}),
        };
      } else {
        taskRef = scope === "all_matching"
          ? await createAllMatchTask({
              batch_id: batchId,
              expected_revision: revision,
              predicate: { version: 1 },
              excluded_item_ids: excludedItemIds ?? [],
              expected_selection_digest: selectionDigest ?? "",
              expected_admissible_count: admissibleCount ?? 0,
              output_root: outputRoot,
              concurrency,
              layout,
              ...(includePreciseOrbit !== undefined ? { include_precise_orbit: includePreciseOrbit } : {}),
            })
          : await createTask(batchId, revision, itemIds, {
          output_root: outputRoot,
          concurrency,
          layout,
          ...(includePreciseOrbit !== undefined ? { include_precise_orbit: includePreciseOrbit } : {}),
        });
      }
      if (!hasTaskReceipt(taskRef)) {
        if (prepId) {
          setPendingDownloadPreparations((prev) =>
            prev.map((item) =>
              item.id === prepId
                ? { ...item, status: "failed", error: translate("mapWorkbench.results.taskReceiptUnknown") }
                : item,
            ),
          );
        }
        if (isCurrentRequest()) {
          setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.results.taskReceiptUnknown") });
        }
        return;
      }
      await taskOverviewSource.refreshNow().catch((refreshErr) => {
        console.warn("Refresh task overview before clearing preparation error:", refreshErr);
      });
      if (prepId) {
        setPendingDownloadPreparations((prev) => prev.filter((item) => item.id !== prepId));
      }
      let notice = createTaskOutcomeNotice(taskRef);
      if (includeGacos === true) {
        const excludedSet = new Set(excludedItemIds ?? []);
        const allKnownScenes = [
          ...scenesRef.current,
          ...Array.from(checkedScenesRef.current.values()),
        ];
        const gacosItemIds = scope === "all_matching"
          ? scenesRef.current
              .filter((s) => !excludedSet.has(s.scene_id))
              .map((s) => s.scene_id)
          : itemIds;
        const gacosOutcome = await executeCompanionGacosRequest({
          outputRoot,
          batchId,
          itemIds: gacosItemIds,
          scenes: allKnownScenes,
          extentMode: companionStateRef.current.extentMode,
          studyAreaName: aoiLabel,
          studyAreaGeometry: workingGeometry,
          studyAreaBbox: workingAoi,
        });
        if (gacosOutcome.ok) {
          if (notice) {
            notice = {
              ...notice,
              text: `${notice.text} · 已同步提交 GACOS 申请（共 ${gacosOutcome.requestGroupCount} 批，可在下载中心导入结果）`,
            };
          } else {
            notice = {
              tone: "neutral",
              text: `已同步提交 GACOS 大气延迟申请（共 ${gacosOutcome.requestGroupCount} 批，可在下载中心导入结果）`,
            };
          }
        } else if (isCurrentRequest()) {
          setTaskWarning(`主影像下载已创建；GACOS 配套申请未完成：${gacosOutcome.error}`);
        }
      }
      if (includeDem === true) {
        const excludedSet = new Set(excludedItemIds ?? []);
        const allKnownScenes = [
          ...scenesRef.current,
          ...Array.from(checkedScenesRef.current.values()),
        ];
        const targetIdSet = new Set(itemIds);
        const demScenes = scope === "all_matching"
          ? allKnownScenes.filter((s) => !excludedSet.has(s.scene_id))
          : allKnownScenes.filter((s) => targetIdSet.has(s.scene_id));
        const demBbox = resolveAuxiliaryExtent(
          companionStateRef.current.extentMode,
          workingAoi,
          demScenes
        );
        if (demBbox) {
          const demRes = await startDemDownloadBbox(
            demBbox.west,
            demBbox.east,
            demBbox.south,
            demBbox.north,
            outputRoot,
            "COP30",
            "auto",
            true
          ).catch(() => null);
          if (demRes && demRes.ok && notice) {
            notice = {
              ...notice,
              text: `${notice.text} · 已同步创建 30m 参考 DEM 下载任务`,
            };
          }
        }
      }
      if (notice && isCurrentRequest()) {
        setTaskOutcomeNotice(notice);
      }
      if (taskRef && taskRef.warning && isCurrentRequest()) {
        setTaskWarning(taskRef.warning);
      }
      if (includeGacos === true || includeDem === true) {
        await taskOverviewSource.refreshNow().catch((refreshErr) => {
          console.warn("Refresh task overview after companion creation error:", refreshErr);
        });
      }
      if (originAgentSessionId) {
        void hydrateAgentSessionBatch(
          originAgentSessionId,
          batchId,
          revision,
          currentResourceId
        ).catch(() => {});
        if (scope === "checked") {
          setCheckedSceneIds((previous) => {
            const next = new Set(previous);
            for (const id of itemIds) next.delete(id);
            return next;
          });
        }
      } else if (isCurrentRequest()) {
        void refreshLoadedRowStates(batchId, revision).catch((refreshErr) => {
          console.warn("Refresh batch rows after task creation error:", refreshErr);
        });
      }
      const shownBatch = batchFor(activeResourceIdRef.current);
      if (scope === "all_matching" && generation === sourceSwitchGenerationRef.current
        && displayedBatchRef.current === batchId && shownBatch?.batch_id === batchId && shownBatch.revision === revision) {
        setAllMatchingSelected(false);
        setExcludedSceneIds(new Set());
      }
      if (scope === "checked" && generation === sourceSwitchGenerationRef.current
        && displayedBatchRef.current === batchId && shownBatch?.batch_id === batchId && shownBatch.revision === revision) {
        setCheckedSceneIds((previous) => {
          const current = batchFor(activeResourceIdRef.current);
          if (generation !== sourceSwitchGenerationRef.current || displayedBatchRef.current !== batchId
            || current?.batch_id !== batchId || current.revision !== revision) return previous;
          const next = new Set(previous);
          for (const id of itemIds) next.delete(id);
          return next;
        });
      }
    } catch (err) {
      const typedErr = err as TypedError;
      if (typedErr?.needs_account) {
        if (prepId) {
          setPendingDownloadPreparations((prev) => prev.filter((item) => item.id !== prepId));
        }
        const current = batchFor(activeResourceIdRef.current);
        if (generation === sourceSwitchGenerationRef.current && displayedBatchRef.current === batchId
          && current?.batch_id === batchId && current.revision === revision) {
          setDownloadDrawerOpen(false);
          setPendingRetryAction({ batchId, revision, itemIds: [...itemIds], excludedItemIds: excludedItemIds ? [...excludedItemIds] : undefined, selectionDigest, admissibleCount, outputRoot, concurrency, layout, includePreciseOrbit, includeGacos, scope, generation });
          setInlineLoginError(typedErr);
        }
      } else {
        const resolvedObj =
          err && typeof err === "object" ? (err as Record<string, unknown>) : null;
        const resolvedMsg =
          resolvedObj &&
          typeof resolvedObj.resolved_message_zh === "string" &&
          resolvedObj.resolved_message_zh.trim()
            ? resolvedObj.resolved_message_zh.trim()
            : null;
        const explicitAccountId =
          resolvedObj && typeof resolvedObj.actionAccountId === "string"
            ? (resolvedObj.actionAccountId as AccountId)
            : null;
        const explicitActionLabel =
          resolvedObj && typeof resolvedObj.actionLabel === "string"
            ? resolvedObj.actionLabel
            : null;
        const outcome = resolvedMsg
          ? { tone: "danger" as const, text: resolvedMsg }
          : createTaskOutcomeNotice(err);
        const fallbackAccountAction = explicitAccountId
          ? { actionAccountId: explicitAccountId, actionLabel: explicitActionLabel }
          : resolveAccountActionFromError(
              {
                ...(resolvedObj ?? {}),
                message: outcome?.text || (err instanceof Error ? err.message : undefined),
              },
              currentResourceId,
            );
        const actionAccountId = fallbackAccountAction.actionAccountId;
        const actionLabel = explicitActionLabel || fallbackAccountAction.actionLabel;
        if (prepId) {
          setPendingDownloadPreparations((prev) =>
            prev.map((item) =>
              item.id === prepId
                ? {
                    ...item,
                    status: "failed",
                    error: outcome?.text || translate("mapWorkbench.desktop.actionFailed"),
                    ...(actionAccountId ? { actionAccountId } : {}),
                    ...(actionLabel ? { actionLabel } : {}),
                  }
                : item,
            ),
          );
        }
        if (isCurrentRequest()) {
          setTaskOutcomeNotice(outcome);
        }
      }
    } finally {
      if (stopOpticalObserver) {
        stopOpticalObserver();
      }
      if (generation === sourceSwitchGenerationRef.current && displayedBatchRef.current === batchId) {
        if (itemId) {
          setCreatingItemIds((prev) => withoutCreatePending(prev, itemId));
        } else {
          setDownloadBusy(false);
          setBatchCreating(false);
        }
      }
    }
  }, [batchFor, refreshLoadedRowStates, taskOverviewSource, translate, aoiLabel, workingGeometry, workingAoi]);

  const handleOpenNisarSubsetWorkspace = useCallback(async () => {
    const resId = activeResourceIdRef.current;
    const activeNisarLevel = isNisarLogicalResourceId(resId)
      ? resolveNisarProductLevel(productType || getWorkbenchToolOptions().nisarProductLevel)
      : "RSLC";
    const currentViewForNisar = batchFor(resId);
    const hasLoadedUnifiedBatch = Boolean(
      currentViewForNisar?.batch_id && displayedBatchRef.current === currentViewForNisar.batch_id
    );
    if (isNisarLogicalResourceId(resId) && (activeNisarLevel !== "RSLC" || !hasLoadedUnifiedBatch)) {
      if (activeNisarLevel === "SME2") {
        setTaskOutcomeNotice({
          tone: "danger",
          text: "NISAR SME2 属于 L3 土壤湿度粗分辨率网格产品，不支持单景雷达/地理编码子集裁剪，请直接加入下载队列。",
        });
        return;
      }
      if (scenesRef.current.length === 0) {
        setTaskOutcomeNotice({
          tone: "danger",
          text: "请先执行 NISAR 检索并在结果中选择影像。",
        });
        return;
      }
      const checked = scenesRef.current.filter((sc) => checkedSceneIds.has(sc.scene_id));
      if (checked.length === 0) {
        setTaskOutcomeNotice({
          tone: "danger",
          text: "请先在右侧结果列表勾选至少一景 NISAR 影像，再进入 AOI 裁剪。",
        });
        return;
      }
      const targetScenes = checked;
      const initialOut =
        nonTemporaryOutputDir(getWorkbenchToolOptions().openGeodataOutputDir) ||
        nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
        nonTemporaryOutputDir(storageRootPath) ||
        "";
      setNisarMultiLevelModalSession({
        productLevel: activeNisarLevel,
        band: isNisarSBandResourceId(resId) ? "S" : "L",
        scenes: targetScenes,
        outputRoot: initialOut,
      });
      return;
    }

    const currentView = batchFor(resId);
    if (!currentView?.batch_id || displayedBatchRef.current !== currentView.batch_id) {
      setTaskOutcomeNotice({ tone: "danger", text: "请先执行 NISAR 检索并在结果中选择影像。" });
      return;
    }
    const revision = currentView.revision ?? 0;
    setNisarSubsetOpenBusy(true);
    try {
      let cap = nisarSubsetCapability;
      if (!cap && isTauriDesktopHost()) {
        const res = await getNisarRemoteSubsetCapability().catch(() => null);
        if (res && res.ok) {
          cap = res.capability;
          setNisarSubsetCapability(cap);
        }
      }
      let eff = nisarEffectiveCapability;
      if (!eff && isTauriDesktopHost()) {
        const res = await getApplicationEffectiveCapability("capability.nisar-rslc.remote-subset.v1").catch(() => null);
        if (res && res.ok) {
          eff = res.status;
          setNisarEffectiveCapability(eff);
        }
      }
      const nisarScenes = scenesRef.current.filter((s) => isNisarRslcProduct(s.sar_product));
      if (nisarScenes.length === 0) {
        setTaskOutcomeNotice({ tone: "danger", text: "当前批次没有可进入裁剪工作区的 NISAR RSLC 影像。" });
        return;
      }
      const checkedNisar = nisarScenes.filter((sc) => checkedSceneIds.has(sc.scene_id));
      if (checkedNisar.length === 0) {
        setTaskOutcomeNotice({
          tone: "danger",
          text: "请先在右侧结果列表勾选至少一景 NISAR RSLC 影像，再进入 AOI 裁剪。",
        });
        return;
      }
      const targetNisarScenes = checkedNisar;
      const firstSelections = availableNisarSubsetSelections(cap, targetNisarScenes[0].sar_product!);
      const initialOut =
        nonTemporaryOutputDir(getWorkbenchToolOptions().openGeodataOutputDir) ||
        nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
        nonTemporaryOutputDir(storageRootPath) ||
        "";
      setNisarMultiLevelModalSession({
        productLevel: "RSLC",
        band: isNisarSBandResourceId(resId) ? "S" : "L",
        scenes: targetNisarScenes,
        outputRoot: initialOut,
        batchId: currentView.batch_id,
        batchRevision: revision,
        rslcSelections: firstSelections,
      });
      setNisarSubsetPrepareError(null);
    } finally {
      setNisarSubsetOpenBusy(false);
    }
  }, [batchFor, nisarSubsetCapability, nisarEffectiveCapability, productType, sarDownloadPolicy, storageRootPath, workingGeometry, workingAoi, aoiLabel, checkedSceneIds, selectedSceneId]);

  const canRowOpticalAoiClip = useCallback(
    (scene: SceneRow) => classifyOpticalDerivedScene(scene).eligible,
    [],
  );

  const opticalClipSelectionScenes = useMemo(() => {
    return checkedScenes;
  }, [checkedScenes]);

  const opticalAoiClipDisabledReason = useMemo(() => {
    if (opticalClipSelectionScenes.length === 0) {
      return translate("panels.opticalAoiClip.blockedNoSelection");
    }
    const resolved = resolveOpticalDerivedSelection(opticalClipSelectionScenes);
    if (!resolved.ok) {
      return resolved.blockCode === "USGS_M2M_ARCHIVE_UNSUPPORTED"
        || resolved.blockCode === "CDSE_SAFE_ARCHIVE_UNSUPPORTED"
        || resolved.blockCode === "UNSUPPORTED_OPTICAL_SOURCE"
        ? translate("panels.opticalAoiClip.blockedUnsupportedSource")
        : translate("panels.opticalAoiClip.blockedNoSelection");
    }
    return null;
  }, [opticalClipSelectionScenes, translate]);

  const openOpticalAoiClipModal = useCallback(
    (targetScenes: readonly SceneRow[]) => {
      const currentView = batchFor(activeResourceIdRef.current);
      if (!currentView?.batch_id || displayedBatchRef.current !== currentView.batch_id) {
        return;
      }
      const resolved = resolveOpticalDerivedSelection(targetScenes);
      if (!resolved.ok) {
        setTaskOutcomeNotice({
          tone: "danger",
          text:
            resolved.blockCode === "USGS_M2M_ARCHIVE_UNSUPPORTED"
              || resolved.blockCode === "CDSE_SAFE_ARCHIVE_UNSUPPORTED"
              || resolved.blockCode === "UNSUPPORTED_OPTICAL_SOURCE"
              ? translate("panels.opticalAoiClip.blockedUnsupportedSource")
              : translate("panels.opticalAoiClip.blockedNoSelection"),
        });
        return;
      }
      const outputRoot =
        nonTemporaryOutputDir(getWorkbenchToolOptions().openGeodataOutputDir)
        || nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root)
        || nonTemporaryOutputDir(storageRootPath)
        || "";
      setOpticalAoiClipSession({
        sourceGeneration: sourceSwitchGenerationRef.current,
        batchId: currentView.batch_id,
        revision: currentView.revision ?? 0,
        scenes: structuredClone([...targetScenes]),
        outputRoot,
        concurrency: sarDownloadPolicy?.max_concurrent_items || getTransferEnginePrefs().max_concurrent_tasks,
        aoiGeometry: workingGeometry ? structuredClone(workingGeometry) : null,
        aoiBbox: workingAoi ? structuredClone(workingAoi) : null,
        aoiLabel: aoiLabel || null,
      });
    },
    [aoiLabel, batchFor, sarDownloadPolicy, storageRootPath, translate, workingAoi, workingGeometry],
  );

  const handleOpenBatchOpticalAoiClip = useCallback(() => {
    openOpticalAoiClipModal(opticalClipSelectionScenes);
  }, [openOpticalAoiClipModal, opticalClipSelectionScenes]);

  const handleOpenRowOpticalAoiClip = useCallback(
    (sceneId: string) => {
      const target = scenesRef.current.find((scene) => scene.scene_id === sceneId);
      if (target) openOpticalAoiClipModal([target]);
    },
    [openOpticalAoiClipModal],
  );

  const handleReviewNisarSubsetDraft = useCallback(async (draft: NisarSubsetWorkspaceDraft, checkedRows: NisarSubsetWorkspaceRow[]) => {
    const session = nisarSubsetWorkspaceSession;
    if (!session || nisarSubsetPrepareBusy) return;
    const outputRoot = nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) || nonTemporaryOutputDir(storageRootPath);
    if (!outputRoot) {
      setStorageRequiredPrompt(true);
      return;
    }
    if (!draft.requestedGeometry || !draft.selectedFrequency || draft.selectedPolarizations.length === 0) {
      setNisarSubsetPrepareError("裁剪草稿不完整，请重新检查 AOI、Frequency 和 Polarization。");
      return;
    }
    setNisarSubsetPrepareBusy(true);
    setNisarSubsetPrepareError(null);
    setNisarSubsetWorkspaceSession({ ...session, rows: structuredClone(checkedRows), draft: structuredClone(draft) });
    try {
      const preparedResult = await prepareNisarSubsetBatchV2({
        source_batch_id: draft.sourceBatchId,
        source_item_ids: session.rows
          .filter((row) => draft.selectedItemIds.includes(row.itemId))
          .map((row) => row.itemId),
        expected_workbench_revision: session.sourceBatchRevision,
        subset_draft_geometry: structuredClone(draft.requestedGeometry) as DesktopJsonObject,
        selected_frequency: draft.selectedFrequency,
        selected_polarizations: [...draft.selectedPolarizations],
      });
      if (!preparedResult.ok) throw new Error(formatBridgeError(preparedResult));
      const taskId = `nisar-subset-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
      const taskRequest: DesktopNisarConfirmSubsetBatchTaskV2Request = {
        source_batch_id: preparedResult.prepared.source_batch_id,
        expected_workbench_revision: preparedResult.prepared.workbench_revision,
        items: preparedResult.prepared.intents.map((item) => ({
          source_item_id: item.source_item_id,
          prepared_intent: structuredClone(item.intent),
        })),
        task_id: taskId,
        output_root: outputRoot,
        layout: sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct",
        concurrency: sarDownloadPolicy?.max_concurrent_items || getTransferEnginePrefs().max_concurrent_tasks,
        created_at: new Date().toISOString(),
      };
      setNisarSubsetConfirmation({
        prepared: structuredClone(preparedResult.prepared),
        taskRequest,
      });
    } catch (prepareError) {
      setNisarSubsetPrepareError("检查裁剪任务失败：" + formatBridgeError(prepareError));
    } finally {
      setNisarSubsetPrepareBusy(false);
    }
  }, [nisarSubsetWorkspaceSession, nisarSubsetPrepareBusy, sarDownloadPolicy, storageRootPath]);

  const handleConfirmNisarSubsetBatch = useCallback(async (request: DesktopNisarConfirmSubsetBatchTaskV2Request) => {
    const confirmed = await confirmNisarSubsetBatchTaskV2(request);
    if (!confirmed.ok) throw new Error(formatBridgeError(confirmed));
    const started = await startNewlyConfirmedTask(request.task_id);
    if (!started.ok) throw new Error(formatBridgeError(started));
    const projected = projectApplicationCoreSnapshot(started);
    const readyTask = projected.ok
      ? projected.core.tasks.find((task) => task.task_id === request.task_id)
      : null;
    if (
      !readyTask
      || readyTask.output_root !== request.output_root
      || readyTask.selected_count !== request.items.length
      || (readyTask.status !== "queued" && readyTask.status !== "running")
    ) {
      throw new Error("NISAR 裁剪任务尚未进入可调度状态，请刷新后重试。");
    }
    setNisarSubsetConfirmation(null);
    setNisarSubsetWorkspaceSession(null);
    setNisarSubsetPrepareError(null);
    setDownloadDrawerOpen(true);
    setTaskOutcomeNotice({ tone: "neutral", text: "NISAR AOI 裁剪任务已创建并进入下载队列。" });
    await taskOverviewSource.refreshNow();
  }, [taskOverviewSource]);

  const handleSyncGeeResults = useCallback((payload: GeeSyncedResultsPayload) => {
    const compositeId = `gee-composite::${payload.assetId}`;
    const fallbackGeom = workingGeometry ?? bboxToGeoJsonPolygon(payload.bbox);
    const estBytes =
      typeof payload.estimatedMb === "number" && payload.estimatedMb > 0
        ? Math.round(payload.estimatedMb * 1024 * 1024)
        : null;
    const thumbUrl = payload.latestArtifact?.preview_data_url || "";
    const compositeRow: SceneRow = {
      product_key: compositeId,
      product_id: compositeId,
      scene_id: compositeId,
      provider: "Google Earth Engine",
      dataset: "science.gee.catalog.v1",
      file_name: `${payload.title} · AOI 云端合成 (${payload.matchedScenes} 景)`,
      platform: payload.assetId,
      product_type: `GEE 合成 (${payload.nativeScaleM}m)`,
      beam_mode: "",
      polarization: "",
      acquisition_datetime:
        payload.endDate || payload.startDate || new Date().toISOString().slice(0, 10),
      absolute_orbit: null,
      relative_orbit: null,
      path: null,
      frame: null,
      orbit_direction: "",
      file_size_remote: estBytes,
      footprint_bbox: payload.bbox,
      footprint_geojson: fallbackGeom,
      has_url: true,
      download_url: "",
      assets: thumbUrl
        ? [
            {
              role: "thumbnail",
              url: thumbUrl,
              file_name: "gee_composite.png",
              media_type: "image/png",
              size_bytes: null,
            },
          ]
        : [],
      native_metadata: {
        gee_asset_id: payload.assetId,
        is_gee_composite: true,
        native_scale_m: payload.nativeScaleM,
        matched_scenes: payload.matchedScenes,
        total_scenes: payload.totalScenes,
        start_date: payload.startDate,
        end_date: payload.endDate,
        max_cloud: payload.maxCloud,
        tile_url_template: payload.latestArtifact?.tile_url_template ?? null,
        thumbnail_url: thumbUrl || null,
      },
    };
    const sceneRows: SceneRow[] = (payload.scenes || []).slice(0, 50).map((sc) => {
      const rowId = `gee-scene::${sc.image_id}`;
      return {
        product_key: rowId,
        product_id: rowId,
        scene_id: rowId,
        provider: "Google Earth Engine",
        dataset: "science.gee.catalog.v1",
        file_name: sc.image_id,
        platform: payload.assetId,
        product_type: `GEE 单景 (${payload.nativeScaleM}m)`,
        beam_mode: "",
        polarization: "",
        acquisition_datetime:
          sc.acquisition_date || payload.endDate || payload.startDate || "",
        absolute_orbit: null,
        relative_orbit: null,
        path: null,
        frame: null,
        orbit_direction: "",
        footprint_bbox: payload.bbox,
        footprint_geojson: fallbackGeom,
        has_url: true,
        download_url: "",
        assets: [],
        native_metadata: {
          gee_asset_id: sc.image_id,
          collection_asset_id: payload.assetId,
          native_scale_m: payload.nativeScaleM,
          cloud_cover: sc.cloud_cover ?? null,
          max_cloud: payload.maxCloud,
        },
      };
    });
    const combined = [compositeRow, ...sceneRows];
    activateNonBatchSceneList(combined);
    setRowStates(new Map());
    setSearchError(null);
    setSourceViewError(null);
    const syncMsg = `GEE 云端检索结果已同步到右侧结果清单（合成层 1 项 + 单景影像 ${sceneRows.length} 景）。`;
    setInlineToolResult({ ok: true, message: syncMsg });
    setTaskOutcomeNotice({
      tone: "neutral",
      text: `GEE 云端检索已同步：合成图层 1 项 + 单景影像 ${sceneRows.length} 景`,
    });
  }, [workingGeometry, activateNonBatchSceneList]);

  const handlePreviewGeeScene = useCallback(async (scene: SceneRow) => {
    const isComposite = scene.scene_id.startsWith("gee-composite::");
    const isScene = scene.scene_id.startsWith("gee-scene::");
    if (!isComposite && !isScene) return;
    const targetAssetId = scene.scene_id.slice(
      isComposite ? "gee-composite::".length : "gee-scene::".length,
    );
    const meta = (scene.native_metadata || {}) as Record<string, unknown>;
    const box = scene.footprint_bbox ?? workingAoi;
    if (!box) {
      setTaskOutcomeNotice({ tone: "danger", text: "请先绘制或选择研究区 AOI 再预览 GEE 图层。" });
      return;
    }
    const bboxTuple: [number, number, number, number] = [box.west, box.south, box.east, box.north];
    const reqStart = isComposite && typeof meta.start_date === "string" ? meta.start_date : undefined;
    const reqEnd = isComposite && typeof meta.end_date === "string" ? meta.end_date : undefined;
    const reqCloud = isComposite && typeof meta.max_cloud === "number" ? meta.max_cloud : undefined;
    try {
      const res = await geeCloudAnalyze({
        asset_id: targetAssetId,
        operation: "composite",
        reducer: "median",
        bbox: bboxTuple,
        ...(reqStart ? { start_date: reqStart } : {}),
        ...(reqEnd ? { end_date: reqEnd } : {}),
        ...(typeof reqCloud === "number" ? { max_cloud: reqCloud } : {}),
      });
      if (res && typeof res.artifact_id === "string") {
        const record: GeeArtifactRecord = {
          artifact_id: res.artifact_id,
          title: res.title || scene.file_name || targetAssetId,
          dataset_id: targetAssetId,
          operation: "composite",
          bbox: Array.isArray(res.bbox) && res.bbox.length === 4 ? res.bbox : bboxTuple,
          start_date: reqStart ?? null,
          end_date: reqEnd ?? null,
          preview_data_url: res.preview_data_url ?? null,
          tile_url_template: res.tile_url_template ?? null,
          vis_min: res.vis_min ?? 0,
          vis_max: res.vis_max ?? 1,
          palette: Array.isArray(res.palette) ? res.palette : [],
          unit: res.unit || "",
          stats: res.stats,
          created_at: Math.floor(Date.now() / 1000),
        };
        pushGeeArtifactToMap(record, "single");
        setScenes((prev) =>
          prev.map((row) =>
            row.scene_id === scene.scene_id
              ? {
                  ...row,
                  native_metadata: {
                    ...(row.native_metadata || {}),
                    tile_url_template: record.tile_url_template ?? null,
                    thumbnail_url: record.preview_data_url ?? null,
                  },
                }
              : row,
          ),
        );
        emitTopToast({
          tone: "success",
          text: `已将「${scene.file_name || targetAssetId}」在线图层叠加到地图`,
        });
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setTaskOutcomeNotice({ tone: "danger", text: `GEE 图层预览失败：${msg}` });
    }
  }, [workingAoi]);

  const downloadGeeResultRow = useCallback(async (itemId: string): Promise<boolean> => {
    const isComposite = itemId.startsWith("gee-composite::");
    const isScene = itemId.startsWith("gee-scene::");
    if (!isComposite && !isScene) return false;
    const targetAssetId = itemId.slice(
      isComposite ? "gee-composite::".length : "gee-scene::".length,
    );
    const targetScene = scenesRef.current.find((sc) => sc.scene_id === itemId);
    const meta = (targetScene?.native_metadata || {}) as Record<string, unknown>;
    const box = targetScene?.footprint_bbox ?? workingAoi;
    if (!box) {
      setTaskOutcomeNotice({ tone: "danger", text: "请先划定研究区 AOI 再执行 GEE 下载。" });
      return true;
    }
    const outDir =
      nonTemporaryOutputDir(getWorkbenchToolOptions().openGeodataOutputDir) ||
      nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
      nonTemporaryOutputDir(storageRootPath);
    const safeStem = targetAssetId.replace(/[^a-zA-Z0-9_-]+/g, "_").slice(0, 72);
    const explicitOutPath = outDir
      ? `${outDir.replace(/[\\/]+$/, "")}/gee_${safeStem}_${Date.now()}.tif`
      : undefined;
    const reqStart =
      isComposite
        ? (typeof meta.start_date === "string" ? meta.start_date : startDate || undefined)
        : undefined;
    const reqEnd =
      isComposite
        ? (typeof meta.end_date === "string" ? meta.end_date : endDate || undefined)
        : undefined;
    const reqCloud =
      isComposite && typeof meta.max_cloud === "number" ? meta.max_cloud : undefined;
    const res = await geeChunkedDownload({
      asset_id: targetAssetId,
      reducer: "median",
      bbox: [box.west, box.south, box.east, box.north],
      ...(reqStart ? { start_date: reqStart } : {}),
      ...(reqEnd ? { end_date: reqEnd } : {}),
      ...(typeof reqCloud === "number" ? { max_cloud: reqCloud } : {}),
      ...(explicitOutPath ? { output_path: explicitOutPath } : {}),
      use_preview_scale: false,
    });
    if (res?.quota_confirm_required === true) {
      setTaskOutcomeNotice({
        tone: "neutral",
        text: String(
          res.message_zh ||
            "该区域按原生分辨率超过 9 个分块，请在左侧「GEE 云端计算与制图」面板确认全分辨率或选用轻量预览分辨率下载。",
        ),
      });
      return true;
    }
    const savedPath = String(res?.output_path || explicitOutPath || "");
    const realTaskId = `gee-dl-${targetAssetId}`;
    setScenes((prev) =>
      prev.map((sc) =>
        sc.scene_id === itemId
          ? {
              ...sc,
              native_metadata: {
                ...(sc.native_metadata || {}),
                already_downloaded: true,
                local_file_path: savedPath || null,
              },
            }
          : sc,
      ),
    );
    setRowStates((prev) => {
      const next = new Map(prev);
      next.set(itemId, {
        item_id: itemId,
        task_id: realTaskId,
        task_status: "completed",
        item_status: "completed",
        actions: [],
      });
      return next;
    });
    await taskOverviewSource.refreshNow().catch(() => {});
    emitTopToast({
      tone: "success",
      text: savedPath ? `GEE GeoTIFF 已落盘：${savedPath}` : "GEE 分块拼接 GeoTIFF 下载完成",
    });
    return true;
  }, [workingAoi, startDate, endDate, sarDownloadPolicy, storageRootPath, taskOverviewSource]);

  const handleRowAction = useCallback(async (itemId: string, action: RowAction) => {
    if (itemId.startsWith("gee-composite::") || itemId.startsWith("gee-scene::")) {
      if (action === "open_folder") {
        const sc = scenesRef.current.find((row) => row.scene_id === itemId);
        const localFile = sc?.native_metadata?.local_file_path;
        if (typeof localFile === "string" && localFile.trim()) {
          await openPath(localFile.trim());
        } else {
          setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.desktop.taskNotFound") });
        }
        return;
      }
      if (action === "download") {
        setCreatingItemIds((prev) => withCreatePending(prev, itemId));
        setDownloadDrawerOpen(true);
        try {
          await downloadGeeResultRow(itemId);
        } catch (err) {
          setTaskOutcomeNotice({
            tone: "danger",
            text: resolveTaskActionErrorText(err, translate("mapWorkbench.desktop.actionFailed")),
          });
        } finally {
          setCreatingItemIds((prev) => withoutCreatePending(prev, itemId));
        }
      }
      return;
    }

    const rowToolOpts = getWorkbenchToolOptions();
    const rowNisarTiers: ("validated" | "provisional")[] =
      Array.isArray(rowToolOpts.nisarReleaseTiers) && rowToolOpts.nisarReleaseTiers.length > 0
        ? rowToolOpts.nisarReleaseTiers
        : ["validated", "provisional"];
    const rowNisarLevel = isNisarLogicalResourceId(activeResourceId)
      ? resolveNisarProductLevel(productType || rowToolOpts.nisarProductLevel)
      : null;
    const isNisarLBandMultiRoute = Boolean(
      rowNisarLevel &&
      !isNisarSBandResourceId(activeResourceId) &&
      (rowNisarLevel !== "RSLC" || rowNisarTiers.includes("validated")),
    );
    const isNisarSBandMultiRoute = Boolean(
      rowNisarLevel && isNisarSBandResourceId(activeResourceId) && rowNisarLevel !== "RSLC",
    );
    const dedicatedDataset =
      OPEN_GEODATA_RESOURCE_TO_DATASET[activeResourceId] ||
      (isNisarSBandMultiRoute
        ? "bhoonidhi_sband_products"
        : isNisarLBandMultiRoute
          ? "nisar_products"
          : undefined);
    const isOpticalGranule =
      activeResourceId.startsWith("science.sentinel-2.")
      || activeResourceId.includes("landsat")
      || activeResourceId.startsWith("science.hls.");
    const isGeeProvider = !isOpticalGranule && getActiveDatasetProvider(activeResourceId) === "gee";
    if (dedicatedDataset || activeResourceId === "auxiliary.xyz-tiles.v1" || isGeeProvider) {
      if (action === "download") {
        if (isSourceSearchOnly(activeResourceId)) {
          setTaskOutcomeNotice({
            tone: "danger",
            text: "DLR EnMAP L2A 仅支持公开目录检索；原始影像归档需前往 https://eoweb.dlr.de 登录下载。",
          });
          return;
        }
        if (activeResourceId === "auxiliary.xyz-tiles.v1") {
          setCreatingItemIds((prev) => withCreatePending(prev, itemId));
          try {
            await handleTriggerSearch({ forceDownload: true, targetItemId: itemId });
            void taskOverviewSource.refreshNow().catch(() => {});
          } finally {
            setCreatingItemIds((prev) => withoutCreatePending(prev, itemId));
          }
          return;
        }
        const toolOpts = getWorkbenchToolOptions();
        const directOut =
          nonTemporaryOutputDir(toolOpts.openGeodataOutputDir) ||
          nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
          nonTemporaryOutputDir(storageRootPath);
        if (!directOut) {
          pendingStorageResumeRef.current = { type: "row", itemId };
          setStorageRequiredPrompt(true);
          return;
        }
        const rowScene = scenes.find((s) => s.scene_id === itemId || s.product_key === itemId);
        const estBytes = typeof rowScene?.file_size_remote === "number" && rowScene.file_size_remote > 0
          ? rowScene.file_size_remote
          : 0;
        const activeLevel = isNisarLogicalResourceId(activeResourceId)
          ? resolveNisarProductLevel(productType || toolOpts.nisarProductLevel)
          : undefined;
        setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
          sourceBatchId: `geodata-direct-${activeResourceId}`,
          sourceBatchRevision: 0,
          selectionScope: "exact_item",
          selectedProductIds: [itemId],
          acquisition: {
            kind: "generic_sar",
            frozenParameters: {
              dedicated_geodata: true,
              dataset_label: humanPreparationTitle(activeResourceId),
              product_level: activeLevel,
              estimated_total_bytes: estBytes,
              resource_id: activeResourceId,
            },
          },
          destinationPrefill: directOut,
          layout: "direct",
          taskPolicy: sarDownloadPolicy?.mode || "default",
          concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
        }));
        return;
      }
      if (action === "open_folder") {
        const rowTaskId = rowStates.get(itemId)?.task_id;
        const rowTask = rowTaskId ? c1Tasks.find((t) => t.task_id === rowTaskId) : undefined;
        if (!rowTask) {
          setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.desktop.taskNotFound") });
          return;
        }
        try {
          await handleOpenTaskOutput({ task_id: rowTask.task_id, revision: rowTask.revision });
        } catch (err) {
          setTaskOutcomeNotice({ tone: "danger", text: resolveTaskActionErrorText(err, translate("mapWorkbench.desktop.openOutputFailed")) });
        }
      }
      return;
    }

    const isAgentIsolatedCall =
      effectivePageRouteRef.current === "agent" && activeAgentSessionIdRef.current !== null;
    const agentWs = isAgentIsolatedCall
      ? peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current)
      : null;
    const agentSceneOwner = isAgentIsolatedCall
      ? findAgentSceneBatchOwner(agentWs, itemId)
      : null;

    if (action === "open_folder" && isAgentIsolatedCall) {
      const rowTaskId = agentWs?.rowStates?.get(itemId)?.task_id;
      const rowTask = rowTaskId ? c1Tasks.find((t) => t.task_id === rowTaskId) : undefined;
      if (!rowTask) {
        setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.desktop.taskNotFound") });
        return;
      }
      try {
        await handleOpenTaskOutput({ task_id: rowTask.task_id, revision: rowTask.revision });
      } catch (err) {
        setTaskOutcomeNotice({ tone: "danger", text: resolveTaskActionErrorText(err, translate("mapWorkbench.desktop.openOutputFailed")) });
      }
      return;
    }

    const currentView = isAgentIsolatedCall
      ? agentSceneOwner
        ? {
            batch_id: agentSceneOwner.batchId,
            revision: agentSceneOwner.revision,
            resource_id: agentSceneOwner.resourceId ?? activeResourceId,
          }
        : agentWs?.batchId
          ? {
              batch_id: agentWs.batchId,
              revision: 0,
              resource_id: agentWs.resourceId ?? activeResourceId,
            }
          : null
      : batchFor(activeResourceId);

    if (
      !currentView?.batch_id ||
      (!isAgentIsolatedCall &&
        (displayedBatchRef.current !== currentView.batch_id ||
          activeResourceIdRef.current !== activeResourceId))
    ) {
      return;
    }
    const targetResourceId = isAgentIsolatedCall
      ? (agentSceneOwner?.resourceId ?? agentWs?.resourceId ?? activeResourceId)
      : activeResourceId;

    if (action === "download") {
      const outputRoot = isAgentIsolatedCall
        ? resolveAgentSessionDefaultOutputRoot(activeAgentSessionIdRef.current)
        : (nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) || nonTemporaryOutputDir(storageRootPath));
      if (!outputRoot) {
        pendingStorageResumeRef.current = { type: "row", itemId };
        setStorageRequiredPrompt(true);
        return;
      }
      // Landsat C2 L2（MPC）：进入 LandsatDownloadConfirmation（支持 COG 文件集挑选 / USGS .tar 交付与断点准备）
      const rowScene =
        agentSceneOwner?.scene ??
        scenes.find((s) => s.scene_id === itemId || s.product_key === itemId);
      if (rowScene && isMpcLandsatScene(rowScene)) {
        const batchSettings = confirmedDownloadSettings?.batchId === currentView.batch_id
          ? confirmedDownloadSettings
          : null;
        setLandsatDownloadRequest({
          sourceGeneration: sourceSwitchGenerationRef.current,
          batchContext: {
            batch_id: currentView.batch_id,
            created_at: new Date().toISOString(),
            aoi_name: (isAgentIsolatedCall ? agentWs?.aoiLabel : aoiLabel) || "",
            aoi_geojson: ((isAgentIsolatedCall ? agentWs?.workingGeometry : workingGeometry) as Json) ?? null,
            page_index: 0,
            page_size: RESULT_PAGE_SIZE,
            total_count: effectiveBatchTotalCount,
            checked_count: 1,
            revision: currentView.revision ?? 0,
            query: {},
          },
          itemIds: [itemId],
          outputRoot: batchSettings?.outputRoot ?? outputRoot,
          concurrency: batchSettings?.concurrency ?? sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
          taskPolicy: sarDownloadPolicy?.mode || "default",
          isAgentSessionBatch: isAgentIsolatedCall,
        });
        return;
      }
      const batchSettings = confirmedDownloadSettings?.batchId === currentView.batch_id
        ? confirmedDownloadSettings
        : null;
      const concurrency = batchSettings?.concurrency ?? sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks;
      const layout = batchSettings?.layout ?? (sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct");
      if (!batchSettings && isBandSelectableOpticalResource(targetResourceId)) {
        const opticalDraft = await buildOpticalDownloadDraft({
          resourceId: targetResourceId,
          batchId: currentView.batch_id,
          revision: currentView.revision ?? 0,
          selectionScope: "exact_item",
          selectedProductIds: [itemId],
          outputRoot: outputRoot,
          concurrency,
          layout,
          targetScenes: rowScene ? [rowScene] : [],
        });
        setDownloadConfirmationDraft(opticalDraft);
        return;
      }
      const productTypeStr = rowScene?.product_type?.trim().toUpperCase() || productType || "SLC";
      const orbitApplicable = isSentinel1OrbitApplicable(targetResourceId, productTypeStr);
      const sarCompanionApplicable = isSentinel1Resource(targetResourceId);
      const orbitPreference = orbitApplicable
        ? (companionStateRef.current.preciseOrbit ?? getPreciseOrbitPreference(targetResourceId, productTypeStr))
        : false;
      const gacosPreference = sarCompanionApplicable
        ? Boolean(companionStateRef.current.gacos)
        : false;
      const demPreference = sarCompanionApplicable
        ? Boolean(companionStateRef.current.dem)
        : false;
      const isBurst = productTypeStr === "SLC-BURST" || productTypeStr === "BURST";
      const sentinel1Slc = (targetResourceId === "science.sentinel-1.standard.v1" || targetResourceId.includes("sentinel-1")) && productTypeStr === "SLC";

      if (!batchSettings) {
        setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
          sourceBatchId: currentView.batch_id,
          sourceBatchRevision: currentView.revision ?? 0,
          selectionScope: "exact_item",
          selectedProductIds: [itemId],
          acquisition: {
            kind: sentinel1Slc ? "sentinel1" : isBurst ? "burst" : "generic_sar",
            frozenParameters: {
              ...(orbitApplicable
                ? {
                    allow_companion_orbit: true,
                    download_orbit: orbitPreference,
                    allow_companion_gacos: true,
                    request_gacos: gacosPreference,
                    source_key: getOrbitPreferenceSourceKey(targetResourceId, productTypeStr),
                  }
                : sarCompanionApplicable
                  ? {
                      request_gacos: gacosPreference,
                    }
                  : {}),
              ...(sarCompanionApplicable ? { request_dem: demPreference } : {}),
            },
          },
          destinationPrefill: outputRoot,
          layout,
          taskPolicy: sarDownloadPolicy?.mode || "default",
          concurrency,
        }));
        return;
      }
      setDownloadDrawerOpen(true);
      await executeDownloadTask({
        batchId: currentView.batch_id,
        revision: currentView.revision ?? 0,
        itemIds: [itemId],
        outputRoot: batchSettings.outputRoot,
        concurrency,
        layout,
        includePreciseOrbit: batchSettings.includePreciseOrbit ?? (orbitApplicable ? orbitPreference : undefined),
        includeDem: batchSettings.includeDem ?? (sarCompanionApplicable ? demPreference : undefined),
        includeGacos: batchSettings.includeGacos ?? (sarCompanionApplicable ? gacosPreference : undefined),
        scope: "row",
        overrideResourceId: targetResourceId,
        ...(batchSettings.opticalDelivery ? { opticalDelivery: batchSettings.opticalDelivery } : {}),
      });
    } else if (action === "pause" || action === "resume" || action === "retry") {
      const row = rowStates.get(itemId);
      if (row?.task_id) {
        const knownRevision = c1Tasks.find((t) => t.task_id === row.task_id)?.revision;
        try {
          await taskActionRunner.run(row.task_id, action, knownRevision);
        } catch (err) {
          const msgZh = resolveTaskActionErrorText(err, translate("mapWorkbench.desktop.actionFailed"));
          setTaskOutcomeNotice({ tone: "danger", text: msgZh });
          return;
        }
        try {
          await taskOverviewSource.refreshNow();
          const safeCurrentRevision =
            typeof currentView.revision === "number" ? currentView.revision : 0;
          // Refresh loaded pages via getBatchPage in an independent try block.
          await refreshLoadedRowStates(currentView.batch_id, safeCurrentRevision);
        } catch (refreshErr) {
          console.warn("Refresh batch page after row action error:", refreshErr);
        }
      }
    } else if (action === "open_folder") {
      // 打开该行任务的保存目录；没有对应任务时如实提示，不用编造路径兜底
      const rowTaskId = rowStates.get(itemId)?.task_id;
      const rowTask = rowTaskId ? c1Tasks.find((t) => t.task_id === rowTaskId) : undefined;
      if (!rowTask) {
        setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.desktop.taskNotFound") });
        return;
      }
      try {
        await handleOpenTaskOutput({ task_id: rowTask.task_id, revision: rowTask.revision });
      } catch (err) {
        setTaskOutcomeNotice({ tone: "danger", text: resolveTaskActionErrorText(err, translate("mapWorkbench.desktop.openOutputFailed")) });
      }
    }
  }, [sourceViews, activeResourceId, productType, rowStates, storageRootPath, sarDownloadPolicy, c1Tasks, taskActionRunner, taskOverviewSource, handleOpenTaskOutput, handleTriggerSearch, downloadGeeResultRow, scenes, aoiLabel, confirmedDownloadSettings, executeDownloadTask, batchFor, loadingBatch, refreshLoadedRowStates, translate]);

  // 批量下载勾选场景
  const handleDownloadChecked = useCallback(async () => {
    const selectedIds = Array.from(checkedSceneIds);
    if (selectedIds.some((id) => id.startsWith("gee-composite::") || id.startsWith("gee-scene::"))) {
      setDownloadDrawerOpen(true);
      setDownloadBusy(true);
      try {
        for (const id of selectedIds) {
          await downloadGeeResultRow(id);
        }
      } catch (cause) {
        setTaskOutcomeNotice({
          tone: "danger",
          text: resolveTaskActionErrorText(cause, translate("mapWorkbench.desktop.actionFailed")),
        });
      } finally {
        setDownloadBusy(false);
      }
      return;
    }

    const checkedToolOpts = getWorkbenchToolOptions();
    const checkedNisarTiers: ("validated" | "provisional")[] =
      Array.isArray(checkedToolOpts.nisarReleaseTiers) && checkedToolOpts.nisarReleaseTiers.length > 0
        ? checkedToolOpts.nisarReleaseTiers
        : ["validated", "provisional"];
    const checkedNisarLevel = isNisarLogicalResourceId(activeResourceId)
      ? resolveNisarProductLevel(productType || checkedToolOpts.nisarProductLevel)
      : null;
    const isNisarLBandMultiRoute = Boolean(
      checkedNisarLevel &&
      !isNisarSBandResourceId(activeResourceId) &&
      (checkedNisarLevel !== "RSLC" || checkedNisarTiers.includes("validated")),
    );
    const isNisarSBandMultiRoute = Boolean(
      checkedNisarLevel && isNisarSBandResourceId(activeResourceId) && checkedNisarLevel !== "RSLC",
    );
    const dedicatedDataset =
      OPEN_GEODATA_RESOURCE_TO_DATASET[activeResourceId] ||
      (isNisarSBandMultiRoute
        ? "bhoonidhi_sband_products"
        : isNisarLBandMultiRoute
          ? "nisar_products"
          : undefined);
    const isOpticalGranule =
      activeResourceId.startsWith("science.sentinel-2.")
      || activeResourceId.includes("landsat")
      || activeResourceId.startsWith("science.hls.");
    const isGeeProvider = !isOpticalGranule && getActiveDatasetProvider(activeResourceId) === "gee";
    if (dedicatedDataset || activeResourceId === "auxiliary.xyz-tiles.v1" || isGeeProvider) {
      if (isSourceSearchOnly(activeResourceId)) {
        setTaskOutcomeNotice({
          tone: "danger",
          text: "DLR EnMAP L2A 仅支持公开目录检索；原始影像归档需前往 https://eoweb.dlr.de 登录下载。",
        });
        return;
      }
      if (activeResourceId === "auxiliary.xyz-tiles.v1") {
        setDownloadBusy(true);
        try {
          await handleTriggerSearch({ forceDownload: true });
        } finally {
          setDownloadBusy(false);
        }
        return;
      }
      const selectedIds = Array.from(checkedSceneIds);
      if (selectedIds.length === 0) return;
      const toolOpts = getWorkbenchToolOptions();
      const fallbackOut =
        nonTemporaryOutputDir(toolOpts.openGeodataOutputDir) ||
        nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
        nonTemporaryOutputDir(storageRootPath);
      if (!fallbackOut) {
        pendingStorageResumeRef.current = { type: "checked" };
        setStorageRequiredPrompt(true);
        return;
      }
      const selectedSizeSum = selectedIds.reduce((sum, id) => {
        const sc = scenes.find((s) => s.scene_id === id || s.product_key === id) ?? checkedScenesRef.current.get(id);
        const bytes = typeof sc?.file_size_remote === "number" && sc.file_size_remote > 0 ? sc.file_size_remote : 0;
        return sum + bytes;
      }, 0);
      setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
        sourceBatchId: `geodata-direct-${activeResourceId}`,
        sourceBatchRevision: 0,
        selectionScope: "checked",
        selectedProductIds: selectedIds,
        acquisition: {
          kind: "generic_sar",
          frozenParameters: {
            dedicated_geodata: true,
            dataset_label: humanPreparationTitle(activeResourceId),
            product_level: checkedNisarLevel || undefined,
            estimated_total_bytes: selectedSizeSum,
            resource_id: activeResourceId,
          },
        },
        destinationPrefill: fallbackOut,
        layout: "direct",
        taskPolicy: sarDownloadPolicy?.mode || "default",
        concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
      }));
      return;
    }
    const isAgentIsolatedChecked =
      effectivePageRouteRef.current === "agent" && activeAgentSessionIdRef.current !== null;
    if (isAgentIsolatedChecked) {
      if (checkedSceneIds.size === 0) return;
      const outputRoot = resolveAgentSessionDefaultOutputRoot(activeAgentSessionIdRef.current);
      if (!outputRoot) {
        pendingStorageResumeRef.current = { type: "checked" };
        setStorageRequiredPrompt(true);
        return;
      }
      const agentWs = peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current);
      const byBatch = new Map<
        string,
        { batchId: string; revision: number; resourceId: string; ids: string[]; scenes: SceneRow[] }
      >();
      for (const id of checkedSceneIds) {
        const owner = findAgentSceneBatchOwner(agentWs, id);
        if (!owner) continue;
        const group = byBatch.get(owner.batchId) ?? {
          batchId: owner.batchId,
          revision: owner.revision,
          resourceId: owner.resourceId ?? activeResourceId,
          ids: [],
          scenes: [],
        };
        group.ids.push(id);
        group.scenes.push(owner.scene);
        byBatch.set(owner.batchId, group);
      }
      const groups = Array.from(byBatch.values());
      if (groups.length === 0) return;
      if (groups.length === 1) {
        const grp = groups[0];
        const allLandsat = grp.scenes.length > 0 && grp.scenes.every(isMpcLandsatScene);
        if (allLandsat) {
          setLandsatDownloadRequest({
            sourceGeneration: sourceSwitchGenerationRef.current,
            batchContext: {
              batch_id: grp.batchId,
              created_at: new Date().toISOString(),
              aoi_name: agentWs?.aoiLabel || aoiLabel || "",
              aoi_geojson: (agentWs?.workingGeometry as Json) ?? null,
              page_index: 0,
              page_size: RESULT_PAGE_SIZE,
              total_count: grp.ids.length,
              checked_count: grp.ids.length,
              revision: grp.revision,
              query: {},
            },
            itemIds: grp.ids,
            outputRoot,
            concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
            taskPolicy: sarDownloadPolicy?.mode || "default",
            isAgentSessionBatch: true,
          });
          return;
        }
        if (isBandSelectableOpticalResource(grp.resourceId)) {
          const opticalDraft = await buildOpticalDownloadDraft({
            resourceId: grp.resourceId,
            batchId: grp.batchId,
            revision: grp.revision,
            selectionScope: "checked",
            selectedProductIds: grp.ids,
            outputRoot,
            concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
            layout: sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct",
            targetScenes: grp.scenes,
          });
          setDownloadConfirmationDraft(opticalDraft);
          return;
        }
        const productTypeStr = grp.scenes[0]?.product_type?.trim().toUpperCase() || "SLC";
        const orbitApplicable = isSentinel1OrbitApplicable(grp.resourceId, productTypeStr);
        const sarCompanionApplicable = isSentinel1Resource(grp.resourceId);
        const orbitPreference = orbitApplicable
          ? (companionStateRef.current.preciseOrbit ??
              getPreciseOrbitPreference(grp.resourceId, productTypeStr))
          : false;
        const gacosPreference = sarCompanionApplicable ? Boolean(companionStateRef.current.gacos) : false;
        const demPreference = sarCompanionApplicable ? Boolean(companionStateRef.current.dem) : false;
        const sentinel1Slc =
          (grp.resourceId === "science.sentinel-1.standard.v1" ||
            grp.resourceId.includes("sentinel-1")) &&
          productTypeStr === "SLC";
        setDownloadConfirmationDraft(
          createDownloadAcquisitionDraft({
            sourceBatchId: grp.batchId,
            sourceBatchRevision: grp.revision,
            selectionScope: "checked",
            selectedProductIds: grp.ids,
            acquisition: {
              kind: sentinel1Slc ? "sentinel1" : "generic_sar",
              frozenParameters: {
                ...(orbitApplicable
                  ? {
                      allow_companion_orbit: true,
                      download_orbit: orbitPreference,
                      allow_companion_gacos: true,
                      request_gacos: gacosPreference,
                      source_key: getOrbitPreferenceSourceKey(grp.resourceId, productTypeStr),
                    }
                  : sarCompanionApplicable
                    ? {
                        request_gacos: gacosPreference,
                      }
                    : {}),
                ...(sarCompanionApplicable ? { request_dem: demPreference } : {}),
              },
            },
            destinationPrefill: outputRoot,
            layout: sarDownloadPolicy?.organize_by_product
              ? "sar_dataset_processing"
              : "direct",
            taskPolicy: sarDownloadPolicy?.mode || "default",
            concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
          })
        );
        return;
      }
      // 跨多数据源同时勾选下载：按数据源批次分别入队至默认存储目录
      setDownloadDrawerOpen(true);
      for (const grp of groups) {
        await executeDownloadTask({
          batchId: grp.batchId,
          revision: grp.revision,
          itemIds: grp.ids,
          outputRoot,
          concurrency: sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
          layout: sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct",
          scope: "checked",
          overrideResourceId: grp.resourceId,
        });
      }
      return;
    }

    const currentView = batchFor(activeResourceId);
    if (!currentView?.batch_id || displayedBatchRef.current !== currentView.batch_id
      || activeResourceIdRef.current !== activeResourceId || (!allMatchingSelected && checkedSceneIds.size === 0)) return;
    if (allMatchingSelected) {
      const outputRoot = nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) || nonTemporaryOutputDir(storageRootPath);
      if (!outputRoot) {
        pendingStorageResumeRef.current = { type: "checked" };
        setStorageRequiredPrompt(true);
        return;
      }
      if (isBandSelectableOpticalResource(activeResourceId) && scenes.length > 0) {
        const opticalIds = scenes
          .filter((s) => !excludedSceneIds.has(s.scene_id) && !excludedSceneIds.has(s.product_key ?? ""))
          .map((s) => s.scene_id);
        if (opticalIds.length === 0) {
          setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.results.allMatchNoAdmissible") });
          return;
        }
        const batchSettings = confirmedDownloadSettings?.batchId === currentView.batch_id
          ? confirmedDownloadSettings
          : null;
        const matchedScenes = scenes.filter((s) => opticalIds.includes(s.scene_id));
        const opticalDraft = await buildOpticalDownloadDraft({
          resourceId: activeResourceId,
          batchId: currentView.batch_id,
          revision: currentView.revision ?? 0,
          selectionScope: "checked",
          selectedProductIds: opticalIds,
          outputRoot: batchSettings?.outputRoot ?? outputRoot,
          concurrency: batchSettings?.concurrency ?? sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
          layout: sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct",
          targetScenes: matchedScenes,
        });
        setDownloadConfirmationDraft(opticalDraft);
        return;
      }
      if (scenes.length > 0 && scenes.every(isMpcLandsatScene)) {
        const mpcIds = scenes
          .filter((s) => !excludedSceneIds.has(s.scene_id) && !excludedSceneIds.has(s.product_key ?? ""))
          .map((s) => s.scene_id);
        if (mpcIds.length === 0) {
          setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.results.allMatchNoAdmissible") });
          return;
        }
        const batchSettings = confirmedDownloadSettings?.batchId === currentView.batch_id
          ? confirmedDownloadSettings
          : null;
        setLandsatDownloadRequest({
          sourceGeneration: sourceSwitchGenerationRef.current,
          batchContext: {
            batch_id: currentView.batch_id,
            created_at: new Date().toISOString(),
            aoi_name: aoiLabel || "",
            aoi_geojson: (workingGeometry as Json) ?? null,
            page_index: 0,
            page_size: RESULT_PAGE_SIZE,
            total_count: effectiveBatchTotalCount,
            checked_count: mpcIds.length,
            revision: currentView.revision ?? 0,
            query: {},
          },
          itemIds: mpcIds,
          outputRoot: batchSettings?.outputRoot ?? outputRoot,
          concurrency: batchSettings?.concurrency ?? sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
          taskPolicy: sarDownloadPolicy?.mode || "default",
        });
        return;
      }
      const batchId = currentView.batch_id;
      const revision = currentView.revision ?? 0;
      const generation = sourceSwitchGenerationRef.current;
      const excludedIds = Array.from(excludedSceneIds);
      setDownloadBusy(true);
      try {
        const preview = await previewAllMatchTask({
          batch_id: batchId,
          expected_revision: revision,
          predicate: { version: 1 },
          excluded_item_ids: excludedIds,
        });
        const latest = batchFor(activeResourceIdRef.current);
        if (generation !== sourceSwitchGenerationRef.current || displayedBatchRef.current !== batchId
          || latest?.batch_id !== batchId || latest.revision !== revision) return;
        if (preview.admissible_count <= 0) {
          setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.results.allMatchNoAdmissible") });
          return;
        }
        const batchSettings = confirmedDownloadSettings?.batchId === batchId
          ? confirmedDownloadSettings
          : null;
        const productTypeStr = productType?.trim().toUpperCase() || "SLC";
        const orbitApplicable = isSentinel1OrbitApplicable(activeResourceId, productTypeStr);
        const orbitPreference = orbitApplicable
          ? (companionStateRef.current.preciseOrbit ?? getPreciseOrbitPreference(activeResourceId, productTypeStr))
          : false;
        const gacosPreference = orbitApplicable
          ? Boolean(companionStateRef.current.gacos)
          : false;
        const isBurst = productTypeStr === "SLC-BURST" || productTypeStr === "BURST";
        const sentinel1Slc = (activeResourceId === "science.sentinel-1.standard.v1" || activeResourceId.includes("sentinel-1")) && productTypeStr === "SLC";
        setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
          sourceBatchId: batchId,
          sourceBatchRevision: revision,
          selectionScope: "all_matching",
          selectedProductIds: [],
          selectedProductCount: preview.admissible_count,
          excludedProductIds: excludedIds,
          allMatchPreview: preview,
          acquisition: {
            kind: sentinel1Slc ? "sentinel1" : isBurst ? "burst" : "generic_sar",
            frozenParameters: {
              ...(orbitApplicable
                ? {
                    allow_companion_orbit: true,
                    download_orbit: batchSettings?.includePreciseOrbit ?? orbitPreference,
                    allow_companion_gacos: true,
                    request_gacos: batchSettings?.includeGacos ?? gacosPreference,
                    source_key: getOrbitPreferenceSourceKey(activeResourceId, productTypeStr),
                  }
                : {}),
            },
          },
          destinationPrefill: batchSettings?.outputRoot ?? outputRoot,
          layout: batchSettings?.layout ?? (sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct"),
          taskPolicy: sarDownloadPolicy?.mode || "default",
          concurrency: batchSettings?.concurrency ?? sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
        }));
      } catch (err) {
        if (generation === sourceSwitchGenerationRef.current && displayedBatchRef.current === batchId) {
          setTaskOutcomeNotice(createTaskOutcomeNotice(err));
        }
      } finally {
        if (generation === sourceSwitchGenerationRef.current && displayedBatchRef.current === batchId) {
          setDownloadBusy(false);
        }
      }
      return;
    }
    if (checkedScenes.length !== checkedSceneIds.size) {
      setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.desktop.selectionMetadataUnavailable") });
      return;
    }

    const outputRoot = nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) || nonTemporaryOutputDir(storageRootPath);
    if (!outputRoot) {
      pendingStorageResumeRef.current = { type: "checked" };
      setStorageRequiredPrompt(true);
      return;
    }

    // Landsat C2 L2（MPC）勾选批量：走 LandsatDownloadConfirmation
    const landsatIds = Array.from(checkedSceneIds).filter((id) => {
      const scene = checkedScenes.find((s) => s.scene_id === id || s.product_key === id);
      return scene ? isMpcLandsatScene(scene) : false;
    });
    if (landsatIds.length > 0) {
      if (landsatIds.length !== checkedSceneIds.size) {
        setTaskOutcomeNotice({ tone: "danger", text: translate("mapWorkbench.desktop.mixedLandsat") });
        return;
      }
      const batchSettings = confirmedDownloadSettings?.batchId === currentView.batch_id
        ? confirmedDownloadSettings
        : null;
      setLandsatDownloadRequest({
        sourceGeneration: sourceSwitchGenerationRef.current,
        batchContext: {
          batch_id: currentView.batch_id,
          created_at: new Date().toISOString(),
          aoi_name: aoiLabel || "",
          aoi_geojson: (workingGeometry as Json) ?? null,
          page_index: 0,
          page_size: RESULT_PAGE_SIZE,
          total_count: effectiveBatchTotalCount,
          checked_count: landsatIds.length,
          revision: currentView.revision ?? 0,
          query: {},
        },
        itemIds: landsatIds,
        outputRoot: batchSettings?.outputRoot ?? outputRoot,
        concurrency: batchSettings?.concurrency ?? sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
        taskPolicy: sarDownloadPolicy?.mode || "default",
      });
      return;
    }

    const batchSettings = confirmedDownloadSettings?.batchId === currentView.batch_id
      ? confirmedDownloadSettings
      : null;
    if (isBandSelectableOpticalResource(activeResourceId)) {
      const opticalDraft = await buildOpticalDownloadDraft({
        resourceId: activeResourceId,
        batchId: currentView.batch_id,
        revision: currentView.revision ?? 0,
        selectionScope: "checked",
        selectedProductIds: Array.from(checkedSceneIds),
        outputRoot: batchSettings?.outputRoot ?? outputRoot,
        concurrency: batchSettings?.concurrency ?? sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
        layout: batchSettings?.layout ?? (sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct"),
        targetScenes: checkedScenes,
      });
      setDownloadConfirmationDraft(opticalDraft);
      return;
    }
    const productTypeStr = productType?.trim().toUpperCase() || "SLC";
    const orbitApplicable = isSentinel1OrbitApplicable(activeResourceId, productTypeStr);
    const sarCompanionApplicable = isSentinel1Resource(activeResourceId);
    const orbitPreference = orbitApplicable
      ? (companionStateRef.current.preciseOrbit ?? getPreciseOrbitPreference(activeResourceId, productTypeStr))
      : false;
    const gacosPreference = sarCompanionApplicable
      ? Boolean(companionStateRef.current.gacos)
      : false;
    const demPreference = sarCompanionApplicable
      ? Boolean(companionStateRef.current.dem)
      : false;
    const isBurst = productTypeStr === "SLC-BURST" || productTypeStr === "BURST";
    const sentinel1Slc = (activeResourceId === "science.sentinel-1.standard.v1" || activeResourceId.includes("sentinel-1")) && productTypeStr === "SLC";

    setDownloadConfirmationDraft(createDownloadAcquisitionDraft({
      sourceBatchId: currentView.batch_id,
      sourceBatchRevision: currentView.revision ?? 0,
      selectionScope: "checked",
      selectedProductIds: Array.from(checkedSceneIds),
      acquisition: {
        kind: sentinel1Slc ? "sentinel1" : isBurst ? "burst" : "generic_sar",
        frozenParameters: {
          ...(orbitApplicable
            ? {
                allow_companion_orbit: true,
                download_orbit: batchSettings?.includePreciseOrbit ?? orbitPreference,
                allow_companion_gacos: true,
                request_gacos: batchSettings?.includeGacos ?? gacosPreference,
                source_key: getOrbitPreferenceSourceKey(activeResourceId, productTypeStr),
              }
            : sarCompanionApplicable
              ? {
                  request_gacos: batchSettings?.includeGacos ?? gacosPreference,
                }
              : {}),
          ...(sarCompanionApplicable ? { request_dem: batchSettings?.includeDem ?? demPreference } : {}),
        },
      },
      destinationPrefill: batchSettings?.outputRoot ?? outputRoot,
      layout: batchSettings?.layout ?? (sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct"),
      taskPolicy: sarDownloadPolicy?.mode || "default",
      concurrency: batchSettings?.concurrency ?? sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks,
    }));
  }, [sourceViews, activeResourceId, productType, allMatchingSelected, excludedSceneIds, checkedSceneIds, checkedScenes, storageRootPath, sarDownloadPolicy, aoiLabel, confirmedDownloadSettings, batchFor, loadingBatch, translate]);

  useEffect(() => {
    const pending = pendingStorageResumeRef.current;
    if (!pending) return;
    if (route === "settings" || storageRequiredPrompt) return;
    const validRoot =
      nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) || nonTemporaryOutputDir(storageRootPath);
    if (!validRoot) return;
    pendingStorageResumeRef.current = null;
    if (pending.type === "row") {
      void handleRowAction(pending.itemId, "download");
    } else if (pending.type === "dem_plan") {
      void handleTriggerSearch();
    } else {
      void handleDownloadChecked();
    }
  }, [route, storageRequiredPrompt, storageRootPath, sarDownloadPolicy, handleRowAction, handleTriggerSearch, handleDownloadChecked]);

  // 重置参数
  const handleResetParams = useCallback(() => {
    setRelativeOrbit("");
    setPath("");
    setFrame("");
    setPlatforms([]);
    setPolarizations([]);
    setBeamModes([]);
    setLookDirection(null);
    setOrbitDirection("all");
    setProductType("SLC");
    setInlineToolResult(null);
    setSearchError(null);
    setFailoverBanner(null);
    const optSchema = SEARCH_FILTER_SCHEMA_CATALOG.schemas.find(
      (s) => s.resource_id === activeResourceId
    );
    if (optSchema) {
      setOpticalDrafts((prev) => ({
        ...prev,
        [activeResourceId]: initialSearchFilterDraft(optSchema),
      }));
    }
  }, [activeResourceId]);

  // 绑定与绘制 AOI
  const handleDrawAoi = useCallback(() => {
    setDrawMode("rect");
    setDrawActive(true);
  }, []);

  function formatAoiSaveError(resOrErr: unknown, defaultLabel = translate("mapWorkbench.desktop.saveAoiFailed")): string {
    if (!resOrErr) return defaultLabel;
    if (typeof resOrErr === "string") return resOrErr;
    const obj = resOrErr as Record<string, unknown>;
    const code = obj.code || obj.error_code;
    const context = typeof obj.context === "string" ? obj.context : obj.context ? JSON.stringify(obj.context) : "";
    if (code) {
      const codeText = String(code);
      return context
        ? translate("mapWorkbench.desktop.backendRejectedWithContext", { code: codeText, context })
        : translate("mapWorkbench.desktop.backendRejected", { code: codeText });
    }
    const formatted = formatBridgeErrorValue(resOrErr);
    // formatBridgeErrorValue still emits the zh-CN sentinel in every locale.
    if (formatted && formatted !== translateMessage("zh-CN", "mapWorkbench.desktop.uncertainBridgeResponse")) {
      return formatted;
    }
    return String(obj.message || obj.error || defaultLabel);
  }

  const handleClearAoi = useCallback(async () => {
    try {
      const res = await clearWorkingAoi();
      if (res && res.ok) {
        setWorkingAoiState(null);
        setWorkingGeometry(null);
        setAoiLabel(translate("mapWorkbench.desktop.globalView"));
        setDrawActive(false);
      } else {
        alert(formatAoiSaveError(res, translate("mapWorkbench.desktop.clearAoiFailed")));
      }
    } catch (err: any) {
      alert(formatAoiSaveError(err, translate("mapWorkbench.desktop.clearAoiFailed")));
    }
  }, []);

  const handleAoiChange = useCallback(async (bbox: Bbox | null) => {
    setDrawActive(false);
    if (bbox) {
      const label = translate("mapWorkbench.desktop.drawnRectangle");
      const polygonGeojson: Json = {
        type: "Polygon",
        coordinates: [[
          [bbox.west, bbox.south],
          [bbox.east, bbox.south],
          [bbox.east, bbox.north],
          [bbox.west, bbox.north],
          [bbox.west, bbox.south],
        ]],
      };
      try {
        const res = await setWorkingAoi(polygonGeojson, label, "manual_bbox");
        if (res && res.ok) {
          setWorkingAoiState(bbox);
          setWorkingGeometry(polygonGeojson as unknown as Record<string, unknown>);
          setAoiLabel(label);
        } else {
          alert(formatAoiSaveError(res, translate("mapWorkbench.desktop.saveAoiBackendFailed")));
        }
      } catch (err: any) {
        alert(formatAoiSaveError(err, translate("mapWorkbench.desktop.saveDrawnAoiFailed")));
      }
    } else {
      await handleClearAoi();
    }
  }, [handleClearAoi]);

  const handlePolygonDraw = useCallback(async (ring: [number, number][]) => {
    setDrawActive(false);
    if (!ring || ring.length < 3) return;
    const first = ring[0];
    const last = ring[ring.length - 1];
    const closedRing: [number, number][] =
      first[0] === last[0] && first[1] === last[1]
        ? ring
        : [...ring, first];

    const west = Math.min(...closedRing.map((p) => p[0]));
    const east = Math.max(...closedRing.map((p) => p[0]));
    const south = Math.min(...closedRing.map((p) => p[1]));
    const north = Math.max(...closedRing.map((p) => p[1]));

    const bbox: Bbox = { west, east, south, north, crs: "EPSG:4326" };
    const label = translate("mapWorkbench.desktop.drawnPolygon");
    const polygonGeojson: Json = {
      type: "Polygon",
      coordinates: [closedRing],
    };

    try {
      const res = await setWorkingAoi(polygonGeojson, label, "manual_polygon");
      if (res && res.ok) {
        setWorkingAoiState(bbox);
        setWorkingGeometry(polygonGeojson as unknown as Record<string, unknown>);
        setAoiLabel(label);
      } else {
        alert(formatAoiSaveError(res, translate("mapWorkbench.desktop.savePolygonFailed")));
      }
    } catch (err: any) {
      alert(formatAoiSaveError(err, translate("mapWorkbench.desktop.savePolygonFailed")));
    }
  }, []);

  const handleSelectAdminBoundary = useCallback(async (
    boundaryGeojson: Record<string, unknown> | null,
    displayName: string,
    bboxTuple: [number, number, number, number] | null
  ) => {
    const caller = adminModalCallerRef.current;
    adminModalCallerRef.current = "workbench";
    const activeSid = caller === "agent" ? activeAgentSessionIdRef.current : null;

    if (!boundaryGeojson || !bboxTuple) {
      if (activeSid) {
        const saved = await setAgentSessionWorkspaceState(activeSid, {
          workingAoi: null,
          workingGeometry: null,
          aoiLabel: displayName,
        });
        if (activeAgentSessionIdRef.current === activeSid) {
          setAgentSessionWorkspace(saved);
        }
        return;
      }
      const res = await clearWorkingAoi();
      if (!res.ok) throw new Error(formatAoiSaveError(res, translate("mapWorkbench.desktop.restoreEmptyFailed")));
      setWorkingAoiState(null);
      setWorkingGeometry(null);
      setAoiLabel(displayName);
      setSearchError(null);
      setFailoverBanner(null);
      return;
    }
    if (activeSid) {
      const bbox: Bbox = {
        west: bboxTuple[0],
        south: bboxTuple[1],
        east: bboxTuple[2],
        north: bboxTuple[3],
        crs: "EPSG:4326",
      };
      const saved = await setAgentSessionWorkspaceState(activeSid, {
        workingAoi: bbox,
        workingGeometry: boundaryGeojson,
        aoiLabel: displayName,
      });
      if (activeAgentSessionIdRef.current === activeSid) {
        setAgentSessionWorkspace(saved);
        agentFocusNonceRef.current += 1;
        setAgentFocusBoundsRequest({ bbox, nonce: agentFocusNonceRef.current });
      }
      return;
    }
    try {
      const res = await setWorkingAoi(boundaryGeojson as unknown as Json, displayName, "administrative_boundary");
      if (!res.ok) throw new Error(formatAoiSaveError(res, translate("mapWorkbench.desktop.saveAdminFailed")));
      const authBbox =
        Array.isArray((res as any).bbox) && (res as any).bbox.length === 4
          ? ((res as any).bbox as [number, number, number, number])
          : bboxTuple;
      const bbox: Bbox = {
        west: authBbox[0],
        south: authBbox[1],
        east: authBbox[2],
        north: authBbox[3],
        crs: "EPSG:4326",
      };
      setWorkingAoiState(bbox);
      setWorkingGeometry(boundaryGeojson);
      setAoiLabel(displayName);
      setSearchError(null);
      setFailoverBanner(null);
      saveAoiSearchHistory({
        label: displayName,
        bbox,
        geometry: boundaryGeojson,
        authoritySource: "administrative_boundary",
      });
    } catch (err) {
      emitTopToast({
        text: formatAoiSaveError(err, translate("mapWorkbench.desktop.saveAdminException")),
        tone: "danger",
      });
      throw err;
    }
  }, []);

  const handlePointDraw = useCallback(async (lat: number, lng: number) => {
    setDrawActive(false);
    const delta = 0.05;
    const bbox: Bbox = {
      west: lng - delta,
      east: lng + delta,
      south: lat - delta,
      north: lat + delta,
      crs: "EPSG:4326",
    };
    await handleAoiChange(bbox);
  }, [handleAoiChange]);

  const handleAgentClearAoi = useCallback(async () => {
    const sid = activeAgentSessionIdRef.current;
    if (sid) {
      setDrawActive(false);
      const saved = await setAgentSessionWorkspaceState(sid, {
        workingAoi: null,
        workingGeometry: null,
        aoiLabel: translate("mapWorkbench.desktop.globalView"),
      });
      if (activeAgentSessionIdRef.current === sid) setAgentSessionWorkspace(saved);
      return;
    }
    await handleClearAoi();
  }, [handleClearAoi]);

  const handleAgentRectDraw = useCallback(async (bbox: Bbox | null) => {
    const sid = activeAgentSessionIdRef.current;
    if (sid) {
      setDrawActive(false);
      if (!bbox) {
        await handleAgentClearAoi();
        return;
      }
      const label = translate("mapWorkbench.desktop.drawnRectangle");
      const polygonGeojson = {
        type: "Polygon",
        coordinates: [[
          [bbox.west, bbox.south],
          [bbox.east, bbox.south],
          [bbox.east, bbox.north],
          [bbox.west, bbox.north],
          [bbox.west, bbox.south],
        ]],
      };
      const saved = await setAgentSessionWorkspaceState(sid, {
        workingAoi: bbox,
        workingGeometry: polygonGeojson,
        aoiLabel: label,
      });
      if (activeAgentSessionIdRef.current === sid) setAgentSessionWorkspace(saved);
      return;
    }
    await handleAoiChange(bbox);
  }, [handleAgentClearAoi, handleAoiChange]);

  const handleAgentPolygonDraw = useCallback(async (ring: [number, number][]) => {
    const sid = activeAgentSessionIdRef.current;
    if (sid) {
      setDrawActive(false);
      if (!ring || ring.length < 3) return;
      const first = ring[0];
      const last = ring[ring.length - 1];
      const closedRing =
        first[0] === last[0] && first[1] === last[1] ? ring : [...ring, first];
      const polygonGeojson = {
        type: "Polygon",
        coordinates: [closedRing],
      };
      let west = Infinity;
      let south = Infinity;
      let east = -Infinity;
      let north = -Infinity;
      for (const [lng, lat] of closedRing) {
        if (lng < west) west = lng;
        if (lng > east) east = lng;
        if (lat < south) south = lat;
        if (lat > north) north = lat;
      }
      const bbox: Bbox = { west, south, east, north, crs: "EPSG:4326" };
      const label = translate("mapWorkbench.desktop.drawnPolygon");
      const saved = await setAgentSessionWorkspaceState(sid, {
        workingAoi: bbox,
        workingGeometry: polygonGeojson,
        aoiLabel: label,
      });
      if (activeAgentSessionIdRef.current === sid) setAgentSessionWorkspace(saved);
      return;
    }
    await handlePolygonDraw(ring);
  }, [handlePolygonDraw]);

  const handleAgentPointDraw = useCallback(async (lat: number, lng: number) => {
    const sid = activeAgentSessionIdRef.current;
    if (sid) {
      setDrawActive(false);
      const delta = 0.05;
      await handleAgentRectDraw({
        west: lng - delta,
        east: lng + delta,
        south: lat - delta,
        north: lat + delta,
        crs: "EPSG:4326",
      });
      return;
    }
    await handlePointDraw(lat, lng);
  }, [handleAgentRectDraw, handlePointDraw]);

  const handleConditionRemoved = useCallback((removedId: string) => {
    if (removedId === "aoi") {
      const shownBatchId = displayedBatchRef.current;
      const activeViewBatchId = batchFor(activeResourceIdRef.current)?.batch_id;
      if (shownBatchId || activeViewBatchId) {
        setBatchSearchRequests((prev) => {
          const hasShown = shownBatchId ? prev.has(shownBatchId) : false;
          const hasActive = activeViewBatchId ? prev.has(activeViewBatchId) : false;
          if (!hasShown && !hasActive) return prev;
          const next = new Map(prev);
          if (shownBatchId) next.delete(shownBatchId);
          if (activeViewBatchId) next.delete(activeViewBatchId);
          return next;
        });
      }
      setScenes([]);
      setBatchTotalCount(null);
      setBatchDelivery(null);
      setRowStates(new Map());
      setSelectedSceneId(null);
      setCheckedSceneIds(new Set());
      setAllMatchingSelected(false);
      setExcludedSceneIds(new Set());
      setSearchError(null);
      return;
    }
    pendingResearchAfterFilterRemoveRef.current = true;
  }, [batchFor]);

  useEffect(() => {
    if (!pendingResearchAfterFilterRemoveRef.current) return;
    pendingResearchAfterFilterRemoveRef.current = false;
    if (workingAoi && workingGeometry) {
      void handleTriggerSearch().catch(() => {});
    }
  }, [
    startDate,
    endDate,
    platforms,
    productType,
    polarizations,
    beamModes,
    lookDirection,
    orbitDirection,
    relativeOrbit,
    path,
    frame,
    opticalDrafts,
    workingAoi,
    workingGeometry,
    handleTriggerSearch,
  ]);

  // 存储路径持久化 (严格只发 4 字段有效载荷，剥离 ok 包装字段)
  const handleSaveStoragePath = useCallback(async (newPath: string): Promise<boolean> => {
    const trimmed = newPath.trim();
    if (!trimmed) {
      emitTopToast({ text: translate("mapWorkbench.desktop.storagePathRequired"), tone: "danger" });
      return false;
    }
    if (isTemporarySystemDownloadPath(trimmed)) {
      emitTopToast({ text: TEMPORARY_DOWNLOAD_PATH_ERROR_ZH, tone: "danger" });
      return false;
    }
    try {
      const payload = buildStorageSavePayload(sarDownloadPolicy, trimmed);
      const saved = await saveSarDownloadPolicySettings(payload as any);
      if (saved && saved.ok) {
        try {
          const fresh = await getSarDownloadPolicySettings();
          if (fresh && fresh.ok && fresh.default_output_root) {
            setStorageRootPath(fresh.default_output_root);
            setSarDownloadPolicy(fresh);
          } else {
            setStorageRootPath(trimmed);
            setSarDownloadPolicy(saved);
          }
        } catch {
          setStorageRootPath(trimmed);
          setSarDownloadPolicy(saved);
        }
        return true;
      } else {
        emitTopToast({
          text: formatStorageSaveError(saved, translate("mapWorkbench.desktop.saveStorageFailed")),
          tone: "danger",
        });
        return false;
      }
    } catch (err: any) {
      emitTopToast({
        text: formatStorageSaveError(err, translate("mapWorkbench.desktop.changeStorageFailed")),
        tone: "danger",
      });
      return false;
    }
  }, [sarDownloadPolicy]);

  const handlePickStoragePath = useCallback(async (): Promise<boolean> => {
    try {
      const picked = await pickDirectory(translate("mapWorkbench.desktop.chooseStorageRoot"));
      if (picked && picked.ok && picked.path) {
        const cleanPicked = picked.path.trim();
        if (isTemporarySystemDownloadPath(cleanPicked)) {
          emitTopToast({ text: TEMPORARY_DOWNLOAD_PATH_ERROR_ZH, tone: "danger" });
          return false;
        }
        const payload = buildStorageSavePayload(sarDownloadPolicy, cleanPicked);
        const saved = await saveSarDownloadPolicySettings(payload as any);
        if (saved && saved.ok) {
          setStorageRootPath(cleanPicked);
          setSarDownloadPolicy(saved);
          emitTopToast({ text: translate("settings.storage.saved"), tone: "success" });
          return true;
        } else {
          emitTopToast({
            text: formatStorageSaveError(saved, translate("mapWorkbench.desktop.saveStorageFailed")),
            tone: "danger",
          });
          return false;
        }
      }
      return false;
    } catch (err: any) {
      emitTopToast({
        text: formatStorageSaveError(err, translate("mapWorkbench.desktop.changeStorageFailed")),
        tone: "danger",
      });
      return false;
    }
  }, [sarDownloadPolicy]);

  const handleSaveDemConcurrency = useCallback(async (newVal: number): Promise<boolean> => {
    setSavingConcurrency(true);
    try {
      const [freshDem, freshPolicy] = await Promise.all([
        getDemTaskSettings().catch(() => null),
        getSarDownloadPolicySettings().catch(() => null),
      ]);
      const effectivePolicy =
        freshPolicy && (freshPolicy as any).ok
          ? (freshPolicy as SarDownloadPolicySettings)
          : sarDownloadPolicy;
      const sarPayload = {
        mode: effectivePolicy?.mode || "manual",
        organize_by_product: effectivePolicy?.organize_by_product ?? false,
        default_output_root: effectivePolicy?.default_output_root ?? storageRootPath ?? "",
        max_concurrent_items: newVal,
      };
      const [demRes, sarRes] = await Promise.all([
        freshDem && (freshDem as any).ok && (freshDem as any).max_concurrent_tasks === newVal
          ? Promise.resolve(freshDem)
          : saveDemTaskSettings({ max_concurrent_tasks: newVal }),
        effectivePolicy && effectivePolicy.max_concurrent_items === newVal
          ? Promise.resolve(effectivePolicy)
          : saveSarDownloadPolicySettings(sarPayload as any),
      ]);
      const sarOk = Boolean(sarRes && (sarRes as any).ok !== false);
      if (sarOk && sarRes) {
        setSarDownloadPolicy(sarRes as any);
      }
      if (demRes && demRes.ok && sarOk) {
        setDemConcurrency(newVal);
        return true;
      }
      return false;
    } catch {
      return false;
    } finally {
      setSavingConcurrency(false);
    }
  }, [sarDownloadPolicy, storageRootPath]);

  const refreshCreds = useCallback(() => {
    void getCredentialStatus().then(setCreds).catch(() => {});
  }, []);

  // Agent 占位安全回调 (切片 1 & 2 中 Agent 尚未接入，严禁假数据流入)
  const handleApplySpatialArtifact = useCallback(async (_artifact: AgentSpatialArtifact) => {}, []);
  const handleOneClickPipelineExecute = useCallback(async (_artifact: AgentSpatialArtifact) => {}, []);

  // 勾选管理
  const handleToggleCheckScene = useCallback((sceneId: string) => {
    if (effectivePageRouteRef.current === "agent" && activeAgentSessionIdRef.current !== null) {
      const ws = peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current);
      const owner = findAgentSceneBatchOwner(ws, sceneId);
      if (!owner) return;
      checkedScenesRef.current.set(sceneId, owner.scene);
      setCheckedSceneIds((prev) => {
        const next = new Set(prev);
        if (next.has(sceneId)) next.delete(sceneId);
        else next.add(sceneId);
        return next;
      });
      return;
    }
    const batch = batchFor(activeResourceIdRef.current);
    const hasActiveBatch = Boolean(batch && displayedBatchRef.current === batch.batch_id);
    if (!hasActiveBatch && !nonBatchScenesActiveRef.current) return;
    const scene = scenesRef.current.find((candidate) => candidate.scene_id === sceneId);
    if (!scene) return;
    if (hasActiveBatch && allMatchingSelectedRef.current) {
      setExcludedSceneIds((prev) => toggleRowInAllMatch(prev, sceneId));
      return;
    }
    checkedScenesRef.current.set(sceneId, scene);
    setCheckedSceneIds((prev) => {
      const next = new Set(prev);
      if (next.has(sceneId)) next.delete(sceneId);
      else next.add(sceneId);
      return next;
    });
  }, [batchFor]);

  const handleToggleSelectCurrentPage = useCallback((pageSceneIds: string[]) => {
    const batch = batchFor(activeResourceIdRef.current);
    const hasActiveBatch = Boolean(batch && displayedBatchRef.current === batch.batch_id);
    if ((!hasActiveBatch && !nonBatchScenesActiveRef.current) || pageSceneIds.length === 0) return;
    if (hasActiveBatch && allMatchingSelected) {
      setExcludedSceneIds((previous) => togglePageInAllMatch(previous, pageSceneIds));
      return;
    }
    const pageSet = new Set(pageSceneIds);
    for (const scene of scenes) {
      if (pageSet.has(scene.scene_id)) checkedScenesRef.current.set(scene.scene_id, scene);
    }
    setCheckedSceneIds((previous) => togglePageSelection(previous, pageSceneIds));
  }, [batchFor, allMatchingSelected, scenes]);

  const handleToggleSelectAllMatching = useCallback(() => {
    const batch = batchFor(activeResourceIdRef.current);
    if (!batch || displayedBatchRef.current !== batch.batch_id) return;
    setAllMatchingSelected((previous) => {
      const next = !previous;
      setExcludedSceneIds(new Set());
      if (next) {
        checkedScenesRef.current.clear();
        setCheckedSceneIds(new Set());
      }
      return next;
    });
  }, [batchFor]);

  const handleToggleCheckAll = useCallback(() => {
    if (effectivePageRouteRef.current === "agent" && activeAgentSessionIdRef.current !== null) {
      const ws = peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current);
      const activeScenes = ws?.scenes ?? [];
      const activeRowStates = ws?.rowStates ?? new Map<string, RowTaskState>();
      const selectable = selectableIds(activeScenes, activeRowStates);
      const selectableSet = new Set(selectable);
      for (const sc of activeScenes) {
        if (selectableSet.has(sc.scene_id)) checkedScenesRef.current.set(sc.scene_id, sc);
      }
      setCheckedSceneIds((previous) => {
        const next = new Set(previous);
        const allSelected = selectable.length > 0 && selectable.every((id) => previous.has(id));
        for (const id of selectable) {
          if (allSelected) next.delete(id);
          else next.add(id);
        }
        return next;
      });
      return;
    }
    const batch = batchFor(activeResourceIdRef.current);
    const hasActiveBatch = Boolean(batch && displayedBatchRef.current === batch.batch_id);
    if (!hasActiveBatch && !nonBatchScenesActiveRef.current) return;
    if (hasActiveBatch && allMatchingSelected) {
      setAllMatchingSelected(false);
      setExcludedSceneIds(new Set());
      checkedScenesRef.current.clear();
      setCheckedSceneIds(new Set());
      return;
    }
    const selectable = selectableIds(scenes, rowStates);
    const selectableSet = new Set(selectable);
    for (const scene of scenes) {
      if (selectableSet.has(scene.scene_id)) checkedScenesRef.current.set(scene.scene_id, scene);
    }
    setCheckedSceneIds((previous) => {
      const next = new Set(previous);
      const allSelected = selectable.length > 0 && selectable.every((id) => previous.has(id));
      for (const id of selectable) {
        if (allSelected) next.delete(id);
        else next.add(id);
      }
      return next;
    });
  }, [scenes, rowStates, batchFor, allMatchingSelected]);

  const c1TasksById = useMemo(() => {
    const map = new Map<string, ApplicationTask>();
    for (const t of c1Tasks) {
      map.set(t.task_id, t);
    }
    return map;
  }, [c1Tasks]);

  const activeDownloadCount = useMemo(() => {
    if (c1Tasks.length > 0) {
      return c1Tasks.filter(
        (t) => (t.status === "running" || t.status === "queued") && !(isGacosWebformTask(t) && t.status !== "running")
      ).length;
    }
    return tasks.filter((t: any) => t.status === "running").length;
  }, [c1Tasks, tasks]);

  const activeSourceView = useMemo(() => {
    return sourceViews.find((v) => v.resource_id === activeResourceId) || null;
  }, [sourceViews, activeResourceId]);
  const effectiveBatchTotalCount = batchTotalCount ?? activeSourceView?.item_count ?? scenes.length;

  // #22 B slice 5: only the delivery of the batch shown counts.
  const activeDelivery = useMemo(() => {
    const shownBatchId = loadingBatch?.resourceId === activeResourceId
      ? loadingBatch.batchId
      : activeSourceView?.batch_id;
    return batchDelivery && batchDelivery.batchId === shownBatchId ? batchDelivery : null;
  }, [batchDelivery, loadingBatch, activeResourceId, activeSourceView]);
  const deliveryLoading = activeDelivery?.state === "loading";

  const handleLoadMore = useCallback(async () => {
    const resourceId = activeResourceId;
    const view = batchFor(resourceId);
    const expectedLength = scenes.length;
    // A loading batch keeps growing past the count of its last page read;
    // its last, partly arrived page is read again to fill it.
    if (
      !view?.batch_id || displayedBatchRef.current !== view.batch_id ||
      activeResourceIdRef.current !== resourceId ||
      loadMoreInFlightRef.current ||
      (!deliveryLoading && expectedLength >= effectiveBatchTotalCount) ||
      (!deliveryLoading && expectedLength % RESULT_PAGE_SIZE !== 0) ||
      (deliveryLoading && pageWaitingAtRef.current !== null)
    ) return;

    const batchId = view.batch_id;
    const revision = view.revision ?? 0;
    const pageIndex = Math.floor(expectedLength / RESULT_PAGE_SIZE);
    const pageStart = pageIndex * RESULT_PAGE_SIZE;
    const requestGeneration = ++loadMoreGenerationRef.current;
    const isCurrentRequest = () =>
      loadMoreGenerationRef.current === requestGeneration && activeResourceIdRef.current === resourceId;
    loadMoreInFlightRef.current = true;
    setLoadingMore(true);
    setLoadMoreError(null);
    try {
      const refreshed = deliveryLoading
        ? await readBatchPrefix(batchId, revision, Math.ceil((expectedLength + RESULT_PAGE_SIZE) / RESULT_PAGE_SIZE) * RESULT_PAGE_SIZE)
        : null;
      const page = refreshed?.first ?? await getBatchPage(batchId, revision, pageIndex, RESULT_PAGE_SIZE);
      if (!isCurrentRequest()) return;
      const latestView = batchFor(resourceId);
      if (!latestView || latestView.batch_id !== batchId || latestView.revision !== revision) return;

      const rowGeneration = ++rowRefreshGenerationRef.current;
      setScenes((previous) => {
        if (!isCurrentRequest() || previous.length !== expectedLength) return previous;
        if (refreshed) return refreshed.scenes;
        const kept = previous.slice(0, pageStart);
        const keptIds = new Set(kept.map((scene) => scene.scene_id));
        return [...kept, ...page.scenes.filter((scene) => !keptIds.has(scene.scene_id))];
      });
      setRowStates((previous) => {
        if (!isCurrentRequest() || rowGeneration !== rowRefreshGenerationRef.current) return previous;
        const next = new Map(previous);
        for (const row of refreshed?.rows ?? page.rows) next.set(row.item_id, row);
        return next;
      });
      setBatchTotalCount(page.total_count);
      setBatchDelivery((previous) => {
        const fromPage = deliveryFromPage(page);
        // Live progress may already know more than this page read.
        if (previous?.batchId === fromPage.batchId && previous.state === "loading" && fromPage.state === "loading") {
          return {
            ...fromPage,
            loadedCount: Math.max(previous.loadedCount, fromPage.loadedCount),
            remoteTotal: fromPage.remoteTotal ?? previous.remoteTotal,
          };
        }
        return fromPage;
      });
      if (page.state === "loading" && (refreshed?.scenes.length ?? pageStart + page.scenes.length) <= expectedLength) {
        // Nothing new has arrived for this page yet: wait for progress.
        setPageWaitingAt(page.loaded_count ?? page.total_count);
      }
    } catch (err) {
      if (!isCurrentRequest()) return;
      const code = (err as { code?: unknown } | null)?.code;
      if (code === "V3_STORE_BATCH_PAGE_NOT_YET_LOADED") {
        // The page has not arrived: show it loading and read it again once
        // more rows have arrived.
        setPageWaitingAt(expectedLength);
      } else {
        setLoadMoreError(translate("mapWorkbench.desktop.loadMoreFailed"));
      }
    } finally {
      if (isCurrentRequest()) {
        loadMoreInFlightRef.current = false;
        setLoadingMore(false);
      }
    }
  }, [activeResourceId, scenes.length, effectiveBatchTotalCount, deliveryLoading, batchFor, readBatchPrefix]);

  const handleCancelSearch = useCallback(async () => {
    const requestId = activeSearchIdRef.current;
    if (!requestId) {
      searchRunTokenRef.current += 1;
      pollerRef.current?.stopSearch();
      loadingBatchRef.current = null;
      setLoadingBatch(null);
      setSearching(false);
      setLoadingMore(false);
      const stoppedMsg = "已中止当前检索";
      setInlineToolResult({ ok: false, message: stoppedMsg });
      setTaskOutcomeNotice({ tone: "neutral", text: stoppedMsg });
      return;
    }
    setCancellingSearch(true);
    try {
      await cancelCatalogSearch(requestId);
    } catch (err) {
      setTaskOutcomeNotice({
        tone: "danger",
        text: explainActionError(err, translate("mapWorkbench.desktop.actionFailed")),
      });
    } finally {
      setCancellingSearch(false);
    }
  }, []);

  const [activeSessionTitle, setActiveSessionTitle] = useState<string | null>(null);

  // 真实导航历史栈（追踪 route + sessionId + settingsCategory，支持顶栏后退/前进与 Alt+←/→）
  const navStackRef = useRef<
    Array<{ route: ShellRoute; sessionId: string | null; settingsCategory: SettingsCategory | undefined }>
  >([
    {
      route,
      sessionId: agentOpenSessionId ?? null,
      settingsCategory,
    },
  ]);
  const [navStackIndex, setNavStackIndex] = useState(0);
  const [navStackLen, setNavStackLen] = useState(1);
  const isRestoringNavRef = useRef(false);

  useEffect(() => {
    const currentSessionId = agentOpenSessionId ?? null;
    if (isRestoringNavRef.current) {
      isRestoringNavRef.current = false;
      return;
    }
    const stack = navStackRef.current;
    const head = stack[navStackIndex];
    if (
      head &&
      head.route === route &&
      head.sessionId === currentSessionId &&
      head.settingsCategory === settingsCategory
    ) {
      return;
    }
    const nextStack = stack.slice(0, navStackIndex + 1);
    nextStack.push({ route, sessionId: currentSessionId, settingsCategory });
    if (nextStack.length > 40) nextStack.shift();
    navStackRef.current = nextStack;
    setNavStackIndex(nextStack.length - 1);
    setNavStackLen(nextStack.length);
  }, [route, agentOpenSessionId, settingsCategory, navStackIndex]);

  const canGoBack = navStackIndex > 0;
  const canGoForward = navStackIndex < navStackLen - 1;

  const handleNavBack = useCallback(() => {
    if (navStackIndex <= 0) return;
    const targetIdx = navStackIndex - 1;
    const entry = navStackRef.current[targetIdx];
    if (!entry) return;
    isRestoringNavRef.current = true;
    setNavStackIndex(targetIdx);
    setSettingsCategory(entry.settingsCategory);
    setAgentOpenSessionId(entry.sessionId);
    if (entry.sessionId) {
      agentSessionMemory.remember(entry.sessionId);
      setHasActiveAgentSession(true);
    } else {
      setHasActiveAgentSession(false);
    }
    navigate(entry.route);
  }, [navStackIndex, navigate, setAgentOpenSessionId, agentSessionMemory, setHasActiveAgentSession]);

  const handleNavForward = useCallback(() => {
    if (navStackIndex >= navStackRef.current.length - 1) return;
    const targetIdx = navStackIndex + 1;
    const entry = navStackRef.current[targetIdx];
    if (!entry) return;
    isRestoringNavRef.current = true;
    setNavStackIndex(targetIdx);
    setSettingsCategory(entry.settingsCategory);
    setAgentOpenSessionId(entry.sessionId);
    if (entry.sessionId) {
      agentSessionMemory.remember(entry.sessionId);
      setHasActiveAgentSession(true);
    } else {
      setHasActiveAgentSession(false);
    }
    navigate(entry.route);
  }, [navStackIndex, navigate, setAgentOpenSessionId, agentSessionMemory, setHasActiveAgentSession]);

  // 全局真实快捷键绑定：Ctrl+B 侧栏、Ctrl+N 新会话、Ctrl+, 设置、Ctrl+1..5 切换工作台、Alt+←/→ 前进后退、Ctrl+Alt+Shift+O 重播欢迎引导页
  useEffect(() => {
    const onGlobalShortcut = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.altKey && e.shiftKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        setWelcomeOnboardingOpen(true);
        return;
      }
      if (e.altKey && !e.ctrlKey && !e.metaKey) {
        if (e.key === "ArrowLeft") {
          e.preventDefault();
          handleNavBack();
          return;
        }
        if (e.key === "ArrowRight") {
          e.preventDefault();
          handleNavForward();
          return;
        }
      }
      if (!(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey) return;
      const key = e.key.toLowerCase();
      if (key === "b") {
        e.preventDefault();
        toggleSidebarExpanded();
      } else if (key === "n") {
        e.preventDefault();
        handleStartNewChat();
        navigate("agent");
      } else if (key === ",") {
        e.preventDefault();
        navigate("settings");
      } else if (key === "1") {
        e.preventDefault();
        handleRailNavigate("agent");
      } else if (key === "2") {
        e.preventDefault();
        handleRailNavigate("workbench");
      } else if (key === "3") {
        e.preventDefault();
        handleRailNavigate("downloads");
      } else if (key === "4") {
        e.preventDefault();
        handleRailNavigate("history");
      } else if (key === "5") {
        e.preventDefault();
        handleRailNavigate("plugins");
      }
    };
    window.addEventListener("keydown", onGlobalShortcut);
    return () => window.removeEventListener("keydown", onGlobalShortcut);
  }, [
    handleNavBack,
    handleNavForward,
    toggleSidebarExpanded,
    handleStartNewChat,
    navigate,
    handleRailNavigate,
  ]);

  const effectivePageRoute: ShellRoute =
    route === "settings" ? lastNonSettingsRouteRef.current || "agent" : route;
  effectivePageRouteRef.current = effectivePageRoute;

  const prevEffectiveRouteSyncRef = useRef<ShellRoute>(effectivePageRoute);
  useEffect(() => {
    const prev = prevEffectiveRouteSyncRef.current;
    prevEffectiveRouteSyncRef.current = effectivePageRoute;
    if (prev !== "workbench" && effectivePageRoute === "workbench" && !isFixtureBackend()) {
      const view = batchFor(activeResourceIdRef.current);
      if (view?.batch_id && (displayedBatchRef.current !== view.batch_id || scenesRef.current.length === 0)) {
        const resIdAtStart = activeResourceIdRef.current;
        void getBatchPage(view.batch_id, view.revision ?? 0, 0, RESULT_PAGE_SIZE)
          .then((page) => {
            if (activeResourceIdRef.current === resIdAtStart && page.batch_id === view.batch_id) {
              applyFirstBatchPage(page);
            }
          })
          .catch(() => {});
      }
    }
  }, [effectivePageRoute, batchFor, applyFirstBatchPage]);

  // 需求 #2：新建未发送草稿（activeAgentSessionId === null）实时继承地图工作台；已有会话严格隔离
  const agentIsIsolated = activeAgentSessionId !== null;
  const agentEffectiveAoi = agentIsIsolated ? (agentSessionWorkspace?.workingAoi ?? null) : workingAoi;
  const agentEffectiveGeometry = agentIsIsolated
    ? (agentSessionWorkspace?.workingGeometry ?? null)
    : workingGeometry;
  const agentEffectiveAoiLabel = agentIsIsolated
    ? (agentSessionWorkspace?.aoiLabel ?? "")
    : aoiLabel;
  const agentEffectiveScenes = agentIsIsolated ? (agentSessionWorkspace?.scenes ?? []) : scenes;
  const agentEffectiveRowStates = agentIsIsolated
    ? (agentSessionWorkspace?.rowStates ?? new Map<string, RowTaskState>())
    : rowStates;
  const agentEffectiveBatchId = agentIsIsolated
    ? (agentSessionWorkspace?.batchId ?? null)
    : (displayedBatchRef.current ?? batchFor(activeResourceId)?.batch_id ?? null);
  const agentEffectiveSourceId = agentIsIsolated
    ? (agentSessionWorkspace?.resourceId ?? activeResourceId)
    : activeResourceId;
  const agentEffectiveBatchTotalCount = agentIsIsolated
    ? (agentSessionWorkspace?.batchTotalCount ?? agentEffectiveScenes.length)
    : effectiveBatchTotalCount;
  const agentEffectiveSelectedSceneId = agentIsIsolated
    ? (agentSessionWorkspace?.selectedSceneId ?? null)
    : selectedSceneId;

  const displayedWorkbenchScenes = useMemo(() => {
    if (
      activeResourceId === "science.nisar.s.ssar.rslc.v1" &&
      (orbitDirection === "ascending" || orbitDirection === "descending")
    ) {
      const prefix = orbitDirection === "ascending" ? "ASC" : "DESC";
      return scenes.filter((sc) => (sc.orbit_direction || "").trim().toUpperCase().startsWith(prefix));
    }
    return scenes;
  }, [scenes, activeResourceId, orbitDirection]);
  const displayedWorkbenchTotalCount =
    activeResourceId === "science.nisar.s.ssar.rslc.v1" &&
    (orbitDirection === "ascending" || orbitDirection === "descending")
      ? displayedWorkbenchScenes.length
      : effectiveBatchTotalCount;

  const currentTitle = useMemo(() => {
    return routeMeta(route).label;
  }, [route, locale]);

  const activeWorkbenchSourceLabel = useMemo(() => {
    if (scientificMode === "gee") {
      return "Google Earth Engine (GEE)";
    }
    if (scientificMode === "dem") {
      const demResId = DEM_CODE_TO_RESOURCE_ID[demSource] || activeResourceId;
      return localizedSourceDisplayName(demResId, locale) || agentSourceNames[demResId] || canonicalSourceDisplayName(demResId);
    }
    if (scientificMode === "orbit") {
      return translate("mapWorkbench.conditions.preciseOrbit");
    }
    if (scientificMode === "convert") {
      return translate("mapWorkbench.conditions.elevationConversion");
    }
    if (isNisarLogicalResourceId(activeResourceId)) {
      return localizedSourceDisplayName(activeResourceId, locale) || CATALOG_METADATA_REGISTRY[activeResourceId]?.name || canonicalSourceDisplayName(activeResourceId);
    }
    return localizedSourceDisplayName(activeResourceId, locale) || agentSourceNames[activeResourceId] || canonicalSourceDisplayName(activeResourceId);
  }, [scientificMode, demSource, activeResourceId, agentSourceNames, locale]);

  const activeRailSourceLabel = useMemo(() => {
    if (effectivePageRoute !== "agent") {
      return activeWorkbenchSourceLabel;
    }
    const projected = filterBatchCollectionByAoi(
      projectAgentSessionBatchCollection(
        agentIsIsolated ? (agentSessionWorkspace?.batches ?? []) : [],
        agentIsIsolated ? (agentSessionWorkspace?.activeBatchFilter ?? "all") : "all"
      ),
      agentEffectiveAoi
    );
    return resolveAgentSessionSourceLabel({
      batches: projected.batches,
      activeFilter: projected.activeFilter,
      scenes: agentEffectiveScenes,
      effectiveResourceId: agentEffectiveSourceId,
      fallbackSourceId: agentEffectiveSourceId,
      sourceNames: agentSourceNames,
      fallbackLabel: activeWorkbenchSourceLabel,
    });
  }, [
    effectivePageRoute,
    activeWorkbenchSourceLabel,
    agentIsIsolated,
    agentSessionWorkspace?.batches,
    agentSessionWorkspace?.activeBatchFilter,
    agentEffectiveAoi,
    agentEffectiveScenes,
    agentEffectiveSourceId,
    agentSourceNames,
  ]);

  const contextSubtitle = useMemo(() => {
    if (route === "agent") {
      return activeSessionTitle || (hasActiveAgentSession ? translate("shell.titlebar.activeSession") : translate("shell.titlebar.newSession"));
    }
    if (route === "workbench") {
      return activeWorkbenchSourceLabel;
    }
    if (route === "downloads") {
      return c1Tasks.length > 0
        ? translate("shell.titlebar.taskCount", { count: c1Tasks.length })
        : translate("shell.titlebar.noTasks");
    }
    if (route === "settings") {
      const labels: Record<string, string> = {
        appearance: translate("settings.categories.appearance"),
        models: translate("settings.categories.models"),
        auth: translate("settings.categories.auth"),
        network_proxy: translate("settings.categories.networkProxy"),
        plugins_skills: translate("settings.categories.pluginsSkills"),
        runtime_tools: translate("settings.categories.runtimeTools"),
        network: translate("settings.categories.network"),
        usage_stats: translate("settings.categories.usageStats"),
        storage: translate("settings.categories.storage"),
      };
      return (settingsCategory ? labels[settingsCategory] : undefined) || translate("settings.dialog.label");
    }
    return null;
  }, [
    route,
    activeSessionTitle,
    hasActiveAgentSession,
    activeWorkbenchSourceLabel,
    c1Tasks.length,
    settingsCategory,
    translate,
    locale,
  ]);

  const handleOpenAgentInMapWorkbench = useCallback(
    async (targetResourceId?: string | null, targetBatchId?: string | null) => {
      const ws = peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current);
      const targetAoi = ws?.workingAoi ?? workingAoi;
      const targetGeo = ws?.workingGeometry ?? workingGeometry;
      const targetLabel = ws?.aoiLabel || aoiLabel;
      if (targetAoi) {
        setWorkingAoiState(targetAoi);
        setWorkingGeometry(targetGeo ?? null);
        setAoiLabel(targetLabel || "");
        if (!isFixtureBackend() && targetGeo) {
          void setWorkingAoi(targetGeo as Json, targetLabel || "工作范围", "manual_polygon").catch(
            () => {}
          );
        }
      }
      const resolvedBatchEntry = targetBatchId
        ? ws?.batches?.find((b) => b.batchId === targetBatchId) ?? null
        : null;
      const nextResource =
        targetResourceId ??
        resolvedBatchEntry?.resourceId ??
        ws?.resourceId ??
        activeResourceIdRef.current;
      navigate("workbench");
      if (nextResource && nextResource !== activeResourceIdRef.current) {
        await handleSelectSource(nextResource).catch(() => {});
      }
      const batchToLoad = targetBatchId ?? resolvedBatchEntry?.batchId ?? ws?.batchId ?? null;
      if (batchToLoad && !isFixtureBackend()) {
        try {
          const page = await getBatchPage(
            batchToLoad,
            resolvedBatchEntry?.revision ?? 0,
            0,
            RESULT_PAGE_SIZE
          );
          applyFirstBatchPage(page);
        } catch {
          // ignore if batch already displayed via handleSelectSource
        }
      }
    },
    [workingAoi, workingGeometry, aoiLabel, navigate, handleSelectSource, applyFirstBatchPage]
  );

  const handleOpenTileDownloadWorkbench = useCallback(
    (suggestedTileSource: string) => {
      if (suggestedTileSource && suggestedTileSource !== "current") {
        const currentOpts = getWorkbenchToolOptions();
        const nextMax = getXyzSourceMaxZoom(suggestedTileSource);
        const clippedZooms = normalizeXyzZooms(
          currentOpts.xyzTileZooms?.length ? currentOpts.xyzTileZooms : [currentOpts.xyzTileZoom],
          Math.min(15, nextMax),
          nextMax,
        );
        updateWorkbenchToolOptions({
          xyzTileSource: suggestedTileSource as WorkbenchToolOptionsSnapshot["xyzTileSource"],
          xyzTileZooms: clippedZooms,
          xyzTileZoom: clippedZooms[clippedZooms.length - 1] ?? Math.min(15, nextMax),
          xyzWaybackProbedMaxZoom: null,
        });
        setWaybackPreviewOverlay(null);
        setOpticalDrafts((prev) => {
          const existing = prev["auxiliary.xyz-tiles.v1"];
          if (!existing || (!("xyz_zooms" in existing) && !("xyz_zoom" in existing))) {
            return prev;
          }
          const {
            xyz_zooms: _discardZooms,
            xyz_zoom: _discardZoom,
            ...rest
          } = existing as SearchFilterDraft & {
            xyz_zooms?: number[];
            xyz_zoom?: string;
          };
          return {
            ...prev,
            "auxiliary.xyz-tiles.v1": rest,
          };
        });
      }
      setScientificMode("sar");
      navigate("workbench");
      void handleSelectSource("auxiliary.xyz-tiles.v1").catch(() => {});
    },
    [navigate, handleSelectSource],
  );

  const aoiSummaryText = useMemo(() => {
    const activeAoi = effectivePageRoute === "agent" ? agentEffectiveAoi : workingAoi;
    const activeLabel = effectivePageRoute === "agent" ? agentEffectiveAoiLabel : aoiLabel;
    if (!activeAoi) return null;
    if (activeLabel && activeLabel.trim()) return activeLabel.trim();
    return `${activeAoi.south.toFixed(2)}°~${activeAoi.north.toFixed(2)}°N, ${activeAoi.west.toFixed(2)}°~${activeAoi.east.toFixed(2)}°E`;
  }, [effectivePageRoute, agentEffectiveAoi, agentEffectiveAoiLabel, workingAoi, aoiLabel]);

  const [dataCatalogOpen, setDataCatalogOpen] = useState(false);
  const [dataCatalogGeeOnly, setDataCatalogGeeOnly] = useState(false);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [guidedTourOpen, setGuidedTourOpen] = useState(false);
  const [guidedTourPromptOpen, setGuidedTourPromptOpen] = useState(false);
  const [tourExpandSearchSignal, setTourExpandSearchSignal] = useState(0);
  const [tourOpenAddProviderSignal, setTourOpenAddProviderSignal] = useState(0);
  const tourReturnRouteRef = useRef<ShellRoute>("agent");
  const guidedTourCatalogSummary = useMemo(
    () => computeGuidedTourCatalogSummary(availableSources),
    [availableSources]
  );
  const [welcomeOnboardingOpen, setWelcomeOnboardingOpen] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    const search = window.location.search || "";
    const hash = window.location.hash || "";
    if (search.includes("onboarding=1") || hash.includes("onboarding=1")) {
      return true;
    }
    return false;
  });
  const [onboardingResolved, setOnboardingResolved] = useState<boolean>(() => {
    if (typeof window === "undefined") return true;
    const search = window.location.search || "";
    const hash = window.location.hash || "";
    if (
      search.includes("onboarding=1") ||
      hash.includes("onboarding=1") ||
      search.includes("no_onboarding=1") ||
      hash.includes("harness") ||
      window.navigator?.userAgent?.includes("jsdom")
    ) {
      return true;
    }
    return !isTauriDesktopHost();
  });

  useEffect(() => {
    if (typeof window === "undefined") return;
    const search = window.location.search || "";
    const hash = window.location.hash || "";
    if (
      search.includes("onboarding=1") ||
      hash.includes("onboarding=1") ||
      search.includes("no_onboarding=1") ||
      hash.includes("harness") ||
      window.navigator?.userAgent?.includes("jsdom")
    ) {
      setOnboardingResolved(true);
      return;
    }
    const isDesktop = isTauriDesktopHost();
    let cancelled = false;
    void getDesktopOnboardingState()
      .then((state) => {
        if (cancelled) return;
        if (state && typeof state.completed === "boolean") {
          setWelcomeOnboardingOpen(!state.completed);
          setOnboardingResolved(true);
          return;
        }
        if (isDesktop) {
          setWelcomeOnboardingOpen(true);
          setOnboardingResolved(true);
          return;
        }
        let localCompleted = false;
        try {
          localCompleted = window.localStorage.getItem(WELCOME_ONBOARDING_STORAGE_KEY) === "1";
        } catch {
          localCompleted = false;
        }
        setWelcomeOnboardingOpen(!localCompleted);
        setOnboardingResolved(true);
      })
      .catch(() => {
        if (cancelled) return;
        setWelcomeOnboardingOpen(true);
        setOnboardingResolved(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);
  const [welcomeCacheDir, setWelcomeCacheDir] = useState<string>("");
  const [welcomeDefaultCacheDir, setWelcomeDefaultCacheDir] = useState<string>("");
  const [welcomeInstallRoot, setWelcomeInstallRoot] = useState<string>("");

  useEffect(() => {
    if (!welcomeOnboardingOpen) return;
    let cancelled = false;
    void Promise.allSettled([
      getNetworkSettings(),
      getRuntimeComponents(),
      getCacheSummary(),
    ]).then(([netRes, rtRes, cacheRes]) => {
      if (cancelled) return;
      let resolvedCustomCache = "";
      if (netRes.status === "fulfilled" && netRes.value && "ok" in netRes.value && netRes.value.ok) {
        const rawCacheDir = (netRes.value.cache_dir || "").trim();
        // Browser-preview mockNetworkSettings uses a template string containing "<user>";
        // real Tauri NetworkSettings defaults to "" (application data cache directory).
        resolvedCustomCache = rawCacheDir.includes("<user>") ? "" : rawCacheDir;
        setWelcomeCacheDir(resolvedCustomCache);
      }
      if (rtRes.status === "fulfilled" && rtRes.value?.install_root) {
        setWelcomeInstallRoot(rtRes.value.install_root.trim());
      }
      if (
        !resolvedCustomCache &&
        cacheRes.status === "fulfilled" &&
        cacheRes.value &&
        typeof cacheRes.value.cache_dir === "string"
      ) {
        setWelcomeDefaultCacheDir(cacheRes.value.cache_dir.trim());
      }
    });
    return () => {
      cancelled = true;
    };
  }, [welcomeOnboardingOpen]);

  const startGuidedTour = useCallback(() => {
    tourReturnRouteRef.current = effectivePageRouteRef.current;
    setShowAdminModal(false);
    setDataCatalogOpen(false);
    setTourOpenAddProviderSignal(0);
    setWelcomeOnboardingOpen(false);
    setGuidedTourPromptOpen(false);
    navigate("agent");
    setGuidedTourOpen(true);
  }, [navigate]);

  const handleGuidedTourStepEnter = useCallback(
    (step: GuidedTourStepConfig) => {
      setShowAdminModal(Boolean(step.openAdminModal));
      if (step.openCatalog) {
        setDataCatalogGeeOnly(false);
        setDataCatalogOpen(true);
      } else {
        setDataCatalogOpen(false);
      }
      if (step.settingsCategory) {
        setSettingsCategory(step.settingsCategory);
      }
      if (step.openAddProviderCatalog) {
        setTourOpenAddProviderSignal((n) => n + 1);
      } else {
        setTourOpenAddProviderSignal(0);
      }
      navigate(step.route);
      if (step.expandSearchPanel) {
        setTourExpandSearchSignal((n) => n + 1);
      }
    },
    [navigate, setSettingsCategory]
  );

  const finishGuidedTour = useCallback(
    (options?: { stayOnCurrent?: boolean; restoreRoute?: boolean }) => {
      setGuidedTourOpen(false);
      setGuidedTourPromptOpen(false);
      setShowAdminModal(false);
      setDataCatalogOpen(false);
      setTourOpenAddProviderSignal(0);
      try {
        if (typeof window !== "undefined") {
          window.localStorage.setItem(GUIDED_TOUR_STORAGE_KEY, "1");
        }
      } catch {
        // ignore storage errors
      }
      if (options?.stayOnCurrent) {
        return;
      }
      if (options?.restoreRoute && tourReturnRouteRef.current) {
        navigate(tourReturnRouteRef.current);
        return;
      }
      navigate("agent");
    },
    [navigate]
  );

  const handlePickWelcomeCacheDir = useCallback(async () => {
    try {
      const picked = await pickDirectory("选择缓存目录");
      if (!picked || !picked.ok || !picked.path?.trim()) return;
      const nextCacheDir = picked.path.trim();
      const fresh = await getNetworkSettings();
      const base =
        fresh && "ok" in fresh && fresh.ok
          ? fresh
          : {
              proxy_enabled: false,
              proxy_url: "",
              cache_enabled: true,
              cache_dir: "",
              cache_limit_mb: 0,
              tianditu_token: "",
              asf_ssl_verify: true,
            };
      const saved = await saveNetworkSettings({
        proxy_enabled: base.proxy_enabled,
        proxy_url: base.proxy_url,
        cache_enabled: base.cache_enabled,
        cache_dir: nextCacheDir,
        cache_limit_mb: base.cache_limit_mb,
        tianditu_token: base.tianditu_token,
        asf_ssl_verify: base.asf_ssl_verify,
      });
      if (saved && "ok" in saved && saved.ok) {
        setWelcomeCacheDir((saved.cache_dir || nextCacheDir).trim());
      }
    } catch {
      // ignore picker cancellation
    }
  }, []);

  const handleResetWelcomeCacheDir = useCallback(async () => {
    try {
      const fresh = await getNetworkSettings();
      const base =
        fresh && "ok" in fresh && fresh.ok
          ? fresh
          : {
              proxy_enabled: false,
              proxy_url: "",
              cache_enabled: true,
              cache_dir: "",
              cache_limit_mb: 0,
              tianditu_token: "",
              asf_ssl_verify: true,
            };
      const saved = await saveNetworkSettings({
        proxy_enabled: base.proxy_enabled,
        proxy_url: base.proxy_url,
        cache_enabled: base.cache_enabled,
        cache_dir: "",
        cache_limit_mb: base.cache_limit_mb,
        tianditu_token: base.tianditu_token,
        asf_ssl_verify: base.asf_ssl_verify,
      });
      if (saved && "ok" in saved && saved.ok) {
        setWelcomeCacheDir("");
        try {
          const refreshedSummary = await getCacheSummary();
          if (refreshedSummary && typeof refreshedSummary.cache_dir === "string") {
            setWelcomeDefaultCacheDir(refreshedSummary.cache_dir.trim());
          }
        } catch {
          // ignore
        }
      }
    } catch {
      // ignore
    }
  }, []);
  const [paletteSessions, setPaletteSessions] = useState<AgentSessionSummary[]>([]);
  const [paletteWorkspaces, setPaletteWorkspaces] = useState<AgentWorkspace[]>([]);
  const agentSessionManualOutputDirsRef = useRef<Map<string, string>>(new Map());
  const [downloadDuplicateMatches, setDownloadDuplicateMatches] = useState<ExistingDownloadMatch[]>([]);
  const [downloadDuplicatePolicy, setDownloadDuplicatePolicy] = useState<"skip" | "overwrite">("skip");
  const [downloadTargetOutputDir, setDownloadTargetOutputDir] = useState<string>("");

  const handleTriggerStandaloneCompanion = useCallback(async () => {
    if (standaloneCompanionBusy) return;
    const curCompanion = companionStateRef.current;
    const currentResId = activeResourceIdRef.current;
    const isS1 = isSentinel1OrbitApplicable(currentResId, productType || "SLC");
    const wantOrbit = isS1 && curCompanion.preciseOrbit;
    const wantDem = curCompanion.dem;
    const wantGacos = curCompanion.gacos;
    if (!wantOrbit && !wantDem && !wantGacos) return;

    let targetOutput =
      nonTemporaryOutputDir(downloadTargetOutputDir) ||
      nonTemporaryOutputDir(confirmedDownloadSettings?.outputRoot) ||
      nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
      nonTemporaryOutputDir(storageRootPath);
    if (!targetOutput) {
      const picked = await pickDirectory(translate("mapWorkbench.desktop.chooseTaskOutput"));
      if (!picked.ok || !picked.path?.trim()) return;
      targetOutput = picked.path.trim();
    }

    const allKnownScenes = [
      ...scenesRef.current,
      ...Array.from(checkedScenesRef.current.values()),
    ];
    const targetScenes = Array.from(checkedSceneIds)
      .map((id) => allKnownScenes.find((s) => s.scene_id === id))
      .filter((s): s is SceneRow => Boolean(s));

    setStandaloneCompanionBusy(true);
    setDownloadDrawerOpen(true);
    const notes: string[] = [];
    const errors: string[] = [];
    try {
      if (wantOrbit && targetScenes.length > 0) {
        const orbitRes = await startOrbitDownloadSnapshot(
          targetOutput,
          targetScenes.slice(0, 100),
          targetScenes.slice(0, 100).map((s) => s.scene_id),
          sarDownloadPolicy?.max_concurrent_items ?? getTransferEnginePrefs().max_concurrent_tasks
        ).catch((e) => ({ ok: false as const, error: e instanceof Error ? e.message : String(e) }));
        if (orbitRes && orbitRes.ok) {
          notes.push(`精密轨道 (${targetScenes.length} 景)`);
        } else {
          errors.push(`精密轨道：${(orbitRes as any)?.error || "创建失败"}`);
        }
      }
      if (wantDem) {
        const demBbox = resolveAuxiliaryExtent(curCompanion.extentMode, workingAoi, targetScenes);
        if (demBbox) {
          const demRes = await startDemDownloadBbox(
            demBbox.west,
            demBbox.east,
            demBbox.south,
            demBbox.north,
            targetOutput,
            "COP30",
            "auto",
            true
          ).catch((e) => ({ ok: false as const, error: e instanceof Error ? e.message : String(e) }));
          if (demRes && demRes.ok) {
            notes.push("30m 参考 DEM");
          } else {
            errors.push(`30m 参考 DEM：${(demRes as any)?.error || "创建失败"}`);
          }
        } else {
          errors.push("30m 参考 DEM：请先确定研究区或勾选影像");
        }
      }
      if (wantGacos && targetScenes.length > 0) {
        const curBatchId = batchFor(currentResId)?.batch_id ?? displayedBatchRef.current;
        const gacosRes = await executeCompanionGacosRequest({
          outputRoot: targetOutput,
          batchId: curBatchId,
          itemIds: targetScenes.map((s) => s.scene_id),
          scenes: allKnownScenes,
          extentMode: curCompanion.extentMode,
          studyAreaName: aoiLabel,
          studyAreaGeometry: workingGeometry,
          studyAreaBbox: workingAoi,
        });
        if (gacosRes.ok) {
          notes.push(`GACOS (${gacosRes.requestGroupCount} 批)`);
        } else {
          errors.push(`GACOS：${gacosRes.error}`);
        }
      }

      await taskOverviewSource.refreshNow().catch(() => {});
      if (notes.length > 0) {
        const msg = `已单独创建配套下载：${notes.join(" · ")}${errors.length > 0 ? `（部分失败：${errors.join("；")}）` : ""}`;
        setTaskOutcomeNotice({ tone: "neutral", text: msg });
        emitTopToast({ text: msg, tone: errors.length > 0 ? "neutral" : "success" });
      } else if (errors.length > 0) {
        const msg = `配套下载创建失败：${errors.join("；")}`;
        setTaskOutcomeNotice({ tone: "danger", text: msg });
        emitTopToast({ text: msg, tone: "danger" });
      }
    } finally {
      setStandaloneCompanionBusy(false);
    }
  }, [
    standaloneCompanionBusy,
    productType,
    downloadTargetOutputDir,
    confirmedDownloadSettings?.outputRoot,
    sarDownloadPolicy?.default_output_root,
    sarDownloadPolicy?.max_concurrent_items,
    storageRootPath,
    checkedSceneIds,
    workingAoi,
    workingGeometry,
    aoiLabel,
    batchFor,
    taskOverviewSource,
  ]);

  const handleOpenDataCatalog = useCallback((opts?: { geeOnly?: boolean }) => {
    setDataCatalogGeeOnly(Boolean(opts?.geeOnly));
    setDataCatalogOpen(true);
  }, []);

  const refreshPaletteSessions = useCallback(async () => {
    if (isFixtureBackend()) return;
    const agent = getDesktopShellBridge()?.agent;
    if (!agent?.sessionsList) return;
    try {
      const [list, wsList] = await Promise.all([
        agent.sessionsList(),
        agent.workspacesList ? agent.workspacesList().catch(() => []) : Promise.resolve([]),
      ]);
      setPaletteSessions(list);
      setPaletteWorkspaces(wsList);
    } catch {
      // ignore transient reload failures
    }
  }, []);

  const resolveAgentSessionDefaultOutputRoot = useCallback(
    (sessionId: string | null): string => {
      if (sessionId) {
        const manualRemembered = nonTemporaryOutputDir(agentSessionManualOutputDirsRef.current.get(sessionId));
        if (manualRemembered) return manualRemembered;
        const sess = paletteSessions.find((s) => s.session_id === sessionId);
        const remembered = nonTemporaryOutputDir(sess?.remembered_output_dir);
        if (remembered) return remembered;
        if (sess?.workspace_id) {
          const ws = paletteWorkspaces.find((w) => w.workspace_id === sess.workspace_id);
          const wsDir = nonTemporaryOutputDir(ws?.folder);
          if (wsDir) return wsDir;
        }
      }
      return (
        nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
        nonTemporaryOutputDir(storageRootPath)
      );
    },
    [paletteSessions, paletteWorkspaces, sarDownloadPolicy?.default_output_root, storageRootPath],
  );

  useEffect(() => {
    void refreshPaletteSessions();
    const unsubscribe = subscribeSessionsChanged(() => {
      void refreshPaletteSessions();
    });
    return unsubscribe;
  }, [refreshPaletteSessions, agentOpenSessionId, agentActivity.liveRunIds.length]);

  const activeSessionKey = agentOpenSessionId ?? agentSessionMemory.recall() ?? null;
  const sessionRunStatuses = useMemo(() => {
    const map: Record<string, string> = {};
    for (const s of paletteSessions) {
      if (s.last_run_status) {
        map[s.session_id] = s.last_run_status;
      }
    }
    if (activeSessionKey && agentActivity.liveRunIds.length > 0) {
      map[activeSessionKey] = "running";
    }
    return map;
  }, [paletteSessions, activeSessionKey, agentActivity.liveRunIds.length]);

  const activeSessionRunStatus = useMemo(() => {
    if (agentActivity.liveRunIds.length > 0) return "running";
    if (!activeSessionKey) return null;
    return sessionRunStatuses[activeSessionKey] ?? null;
  }, [agentActivity.liveRunIds.length, activeSessionKey, sessionRunStatuses]);

  const commandPaletteLocalPaths = useMemo(() => {
    const entries: Array<{ id: string; title: string; path: string; source?: string }> = [];
    const seen = new Set<string>();
    const addDir = (id: string, title: string, rawPath?: string | null, source?: string) => {
      const trimmed = (rawPath || "").trim();
      if (!trimmed || seen.has(trimmed)) return;
      seen.add(trimmed);
      entries.push({ id, title, path: trimmed, source });
    };
    addDir("storage-root", "默认下载保存目录", sarDownloadPolicy?.default_output_root || storageRootPath, "Storage");
    return entries;
  }, [sarDownloadPolicy?.default_output_root, storageRootPath]);

  const activeConfirmIds = downloadConfirmationDraft?.selectedProductIds ?? landsatDownloadRequest?.itemIds ?? null;
  const activeConfirmPrefill = downloadConfirmationDraft?.destinationPrefill ?? landsatDownloadRequest?.outputRoot ?? "";

  useEffect(() => {
    if (!activeConfirmIds) {
      setDownloadDuplicateMatches([]);
      setDownloadDuplicatePolicy("skip");
      setDownloadTargetOutputDir("");
      return;
    }
    setDownloadDuplicatePolicy("skip");
    setDownloadTargetOutputDir(activeConfirmPrefill || "");
  }, [downloadConfirmationDraft, landsatDownloadRequest]);

  useEffect(() => {
    if (!activeConfirmIds || isFixtureBackend()) return;
    let cancelled = false;
    const effectiveDir = (downloadTargetOutputDir || activeConfirmPrefill || "").trim();
    const checkItems: ExistingDownloadCheckItem[] = activeConfirmIds.map((id) => {
      const scene =
        scenes.find((s) => s.scene_id === id || s.product_key === id) ??
        checkedScenesRef.current.get(id) ??
        findAgentSceneBatchOwner(
          effectivePageRoute === "agent"
            ? peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current)
            : null,
          id
        )?.scene;
      const meta = (scene?.native_metadata ?? {}) as Record<string, unknown>;
      return {
        scene_id: scene?.scene_id || id,
        file_name: scene?.file_name || id,
        target_relpath:
          typeof meta.target_relpath === "string" && meta.target_relpath.trim()
            ? meta.target_relpath.trim()
            : null,
      };
    });
    if (checkItems.length === 0) {
      setDownloadDuplicateMatches([]);
      return;
    }
    defaultUnifiedClient
      .checkExistingDownloadedScenes(checkItems, effectiveDir || null)
      .then((matches) => {
        if (!cancelled) {
          setDownloadDuplicateMatches(Array.isArray(matches) ? matches : []);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setDownloadDuplicateMatches([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [activeConfirmIds, activeConfirmPrefill, downloadTargetOutputDir, scenes, effectivePageRoute]);

  const resolveDownloadIdsAfterDuplicatePolicy = useCallback(
    async (candidateIds: string[], targetDir: string): Promise<{ itemIds: string[]; skipIds: string[] }> => {
      if (downloadDuplicatePolicy !== "skip") {
        return { itemIds: candidateIds, skipIds: [] };
      }
      let matchedIds = new Set(downloadDuplicateMatches.map((m) => m.scene_id));
      const trimmedTarget = targetDir.trim();
      const previousTarget = (downloadTargetOutputDir || downloadConfirmationDraft?.destinationPrefill || "").trim();
      if (trimmedTarget && trimmedTarget !== previousTarget && candidateIds.length > 0 && !isFixtureBackend()) {
        try {
          const recheckItems: ExistingDownloadCheckItem[] = candidateIds.map((id) => {
            const scene =
              scenesRef.current.find((s) => s.scene_id === id || s.product_key === id) ??
              checkedScenesRef.current.get(id);
            const meta = (scene?.native_metadata ?? {}) as Record<string, unknown>;
            return {
              scene_id: scene?.scene_id || id,
              file_name: scene?.file_name || id,
              target_relpath:
                typeof meta.target_relpath === "string" && meta.target_relpath.trim()
                  ? meta.target_relpath.trim()
                  : null,
            };
          });
          const fresh = await defaultUnifiedClient.checkExistingDownloadedScenes(recheckItems, trimmedTarget);
          matchedIds = new Set(fresh.map((m) => m.scene_id));
        } catch {
          // fall back to latest checked duplicate matches
        }
      }
      if (matchedIds.size === 0) {
        return { itemIds: candidateIds, skipIds: [] };
      }
      const skipIds = candidateIds.filter((id) => matchedIds.has(id));
      const remaining = candidateIds.filter((id) => !matchedIds.has(id));
      return { itemIds: remaining, skipIds };
    },
    [downloadDuplicatePolicy, downloadDuplicateMatches, downloadTargetOutputDir, downloadConfirmationDraft],
  );

  useEffect(() => {
    if (!workingAoi || !workingGeometry || !aoiLabel) return;
    const trimmed = aoiLabel.trim();
    if (
      !trimmed ||
      trimmed === translate("mapWorkbench.desktop.globalView") ||
      trimmed === translate("mapWorkbench.desktop.drawnRectangle") ||
      trimmed === translate("mapWorkbench.desktop.drawnPolygon") ||
      trimmed === translate("mapWorkbench.desktop.mapSelectedExtent")
    ) {
      return;
    }
    saveAoiSearchHistory({
      label: trimmed,
      bbox: workingAoi,
      geometry: workingGeometry,
      authoritySource: "working_aoi",
    });
  }, [workingAoi, workingGeometry, aoiLabel, translate]);

  const currentTitleIcon = useMemo(() => {
    switch (route) {
      case "agent":
        return <MessageSquare className="h-4 w-4 text-sp-text-secondary" />;
      case "history":
        return <History className="h-4 w-4 text-sp-text-secondary" />;
      case "workbench":
        return <MapIcon className="h-4 w-4 text-sp-text-secondary" />;
      case "downloads":
        return <DownloadIcon className="h-4 w-4 text-sp-text-secondary" />;
      case "plugins":
        return <Blocks className="h-4 w-4 text-sp-text-secondary" />;
      case "settings":
        return <SettingsIcon className="h-4 w-4 text-sp-text-secondary" />;
    }
  }, [route]);

  return (
    <div
      data-ui-smoke-workbench="true"
      className="relative sp-modern sp-quiet-window-shell flex h-screen w-screen flex-col overflow-hidden bg-sp-canvas text-sp-text select-none"
      style={{ "--sp-font-size": `${shellFontSize}px` } as React.CSSProperties}
    >
      <DesktopWindowResizeHandles />
      {/* 1. 桌面端原生 Titlebar (EOStage 34px 极简顶栏与实时空间工作台控制) */}
      <DesktopTitlebar
        title={currentTitle}
        titleIcon={currentTitleIcon}
        leadingBadge={
          isFixtureBackend() ? (
            <span className="inline-flex items-center rounded-sp-sm border border-sp-line bg-sp-warning-bg px-2 py-0.5 text-xs font-medium text-sp-warning-text">
              DEV
            </span>
          ) : null
        }
        unreadCount={notificationState.unreadIds.length}
        notifications={notificationState.items}
        unreadIds={notificationState.unreadIds}
        onNotificationClick={handleNotificationClick}
        onMarkAllRead={handleMarkAllNotificationsRead}
        sidebarExpanded={sidebarExpanded}
        onToggleSidebar={toggleSidebarExpanded}
        activeRoute={route}
        contextSubtitle={contextSubtitle}
        canGoBack={canGoBack}
        canGoForward={canGoForward}
        onGoBack={handleNavBack}
        onGoForward={handleNavForward}
        onNavigate={handleRailNavigate}
        onStartNewChat={handleStartNewChat}
        onOpenDataCatalog={() => handleOpenDataCatalog()}
        aoiSummary={aoiSummaryText}
        onOpenAoiModal={() => {
          adminModalCallerRef.current = effectivePageRoute === "agent" ? "agent" : "workbench";
          setShowAdminModal(true);
        }}
        onClearAoi={
          effectivePageRoute === "agent"
            ? (agentEffectiveAoi ? handleAgentClearAoi : undefined)
            : (workingAoi ? handleClearAoi : undefined)
        }
        activeDownloadCount={activeDownloadCount}
        downloadTasks={c1Tasks}
        onOpenDownloadCenter={() => navigate("downloads")}
        onOpenDownloadDrawer={(taskId?: string) => {
          if (taskId) {
            setDrawerFocusTask((prev) => ({ taskId, nonce: (prev?.nonce ?? 0) + 1 }));
          }
          setDownloadDrawerOpen(true);
        }}
        newChat={
          route !== "settings" && effectivePageRoute === "agent" && !sidebarExpanded
            ? {
                enabled: hasActiveAgentSession,
                onClick: handleStartNewChat,
              }
            : undefined
        }
        panelToggle={
          route !== "settings" && effectivePageRoute === "agent"
            ? {
                open: agentPanelOpen,
                onToggle: () => setAgentPanelOpen((prev) => !prev),
                sceneCount: agentEffectiveScenes.length,
              }
            : undefined
        }
      />

      {/* 2. 主体工作区架构 (EOStage Desktop Edge-to-Edge Split: 240px Sidebar + Pure White Main Canvas) */}
      <div className="flex flex-1 min-h-0 flex-row overflow-hidden">
        <DesktopActivityRail
          activeRoute={route}
          onNavigate={handleRailNavigate}
          downloadsAlert={activeDownloadCount > 0}
          onOpenDownloads={() => navigate("downloads")}
          onOpenDataCatalog={() => handleOpenDataCatalog()}
          onOpenCommandPalette={() => setCommandPaletteOpen(true)}
          dataCatalogOpen={dataCatalogOpen}
          activeDataSourceLabel={activeRailSourceLabel || null}
          onOpenSettingsCategory={(cat) => {
            setSettingsCategory(cat);
            navigate("settings");
          }}
          settingsCategory={settingsCategory}
          onSettingsCategoryChange={setSettingsCategory}
          onCloseSettings={handleCloseSettings}
          dark={dark}
          onToggleDark={onToggleDark}
          expanded={sidebarExpanded}
          onToggleExpanded={toggleSidebarExpanded}
          sidebarWidth={sidebarWidth}
          onSidebarWidthChange={setSidebarWidth}
          onNewChat={handleStartNewChat}
          onOpenSession={handleOpenSessionFromNav}
          activeSessionId={agentOpenSessionId ?? agentSessionMemory.recall()}
          activeSessionRunStatus={activeSessionRunStatus}
          sessionRunStatuses={sessionRunStatuses}
          sessionsRefreshKey={`${agentOpenSessionId ?? ""}|${activeSessionTitle ?? ""}`}
          fontSize={shellFontSize}
          onFontSizeChange={changeShellFontSize}
          unreadCount={notificationState.unreadIds.length}
          notifications={notificationState.items}
          unreadIds={notificationState.unreadIds}
          onNotificationClick={handleNotificationClick}
          onMarkAllRead={handleMarkAllNotificationsRead}
          onClearRead={handleClearReadNotifications}
          onInboxOpenChange={setInboxDrawerOpen}
          accountStatus={eostageAccount}
          onOpenLogin={() => setEostageLoginOpen(true)}
          onOpenBilling={(tab) => setEostageBillingState({ open: true, tab: tab ?? "balance" })}
          onRefreshAccount={() => {
            if (refreshingAccountRef.current) return;
            refreshingAccountRef.current = true;
            void eostageAccountStatus(true)
              .then((st) => {
                if (!accountSectionMountedRef.current) return;
                setEostageAccount(st);
                if (!st.logged_in) {
                  officialModelsAutoSyncedRef.current = false;
                } else {
                  officialModelsAutoSyncedRef.current = true;
                  void eostageAccountSyncModels()
                    .then(() => {
                      if (accountSectionMountedRef.current) notifyChannelsChanged();
                    })
                    .catch(() => {});
                }
                if (st.session_expired) {
                  notifyChannelsChanged();
                }
              })
              .catch(() => {})
              .finally(() => {
                refreshingAccountRef.current = false;
              });
          }}
          onLogoutAccount={() => {
            officialModelsAutoSyncedRef.current = false;
            void eostageAccountLogout()
              .then((next) => {
                setEostageAccount(next);
                notifyEOStageAccountChanged();
                notifyChannelsChanged();
              })
              .catch(() => {});
          }}
          onReplayOnboarding={startGuidedTour}
          onReplayWelcome={() => setWelcomeOnboardingOpen(true)}
        />

        {/* 内容区（浮起式四圆角纯白主画布 + 左侧层叠边缘阴影） */}
        <div
          data-testid="eo-workspace-island"
          className="sp-quiet-main-plane relative flex flex-1 min-w-0 flex-col overflow-hidden bg-white dark:bg-[#18191d]"
        >
          {/* 非阻塞可关闭 TaskRef 提示条 (P0-7) */}
          {taskWarning && (
            <div className="absolute top-2 left-1/2 -translate-x-1/2 z-[1100] flex items-center gap-2 rounded-sp-md border border-sp-warning-text/30 bg-sp-warning-bg px-3.5 py-1.5 text-xs text-sp-warning-text shadow-overlay">
              <AlertCircle className="h-4 w-4 shrink-0 text-sp-warning-text" />
              <span>{taskWarning}</span>
              <button
                type="button"
                onClick={() => setTaskWarning(null)}
                className="ml-2 text-sp-warning-text hover:opacity-75 cursor-pointer"
                aria-label={translate("mapWorkbench.conditions.dismissNotice")}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          )}

          {/* 按路由装载页面 */}
          {effectivePageRoute === "agent" && (
            <div className={cn("flex flex-1 min-w-0 h-full overflow-hidden", route === "settings" && "hidden")}>
              <DesktopAgentWorkbench
                sidebarWidth={sidebarExpanded ? sidebarWidth : 52}
                defaultDownloadDir={nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) || nonTemporaryOutputDir(storageRootPath)}
                sourceNames={agentSourceNames}
                workbenchAoi={agentEffectiveAoi}
                aoiLabel={agentEffectiveAoiLabel}
                timeRange={agentTimeRange}
                sourceId={agentEffectiveSourceId}
                agentTasks={c1Tasks}
                refreshAgentTasks={async () => {
                  await taskOverviewSource.refreshNow();
                  const lastErr = taskOverviewSource.getLastError();
                  if (lastErr !== null && lastErr !== undefined) {
                    throw lastErr;
                  }
                  const latest = taskOverviewSource.getLatest();
                  if (latest === null) {
                    throw new Error(translate("mapWorkbench.desktop.refreshTasksFailed"));
                  }
                  return latest;
                }}
                openSessionId={agentOpenSessionId ?? agentSessionMemory.recall()}
                newChatNonce={newChatNonce}
                onActiveSessionChange={handleAgentActiveSessionChangeIsolated}
                onSessionCreatedSnapshot={handleAgentSessionCreatedSnapshot}
                onAgentSearchSettled={handleAgentSearchSettled}
                onAgentSessionBatchSettled={handleAgentSessionBatchSettled}
                onAgentActivityChange={handleAgentActivityChange}
                onActivityChanged={refreshAgentActivityStatus}
                onStopAllEvent={handleStopAllEvent}
                onTaskAction={async (taskId, rev, action) => {
                  if (!isFixtureBackend()) {
                    await taskActionRunner.run(taskId, action, rev);
                    await taskOverviewSource.refreshNow();
                  }
                }}
                onActiveSessionTitleChange={(title) => {
                  setActiveSessionTitle(title);
                }}
                onViewOnMap={() => {
                  setAgentPanelOpen(true);
                }}
                onUpdateWorkbenchAoi={handleAgentUpdateAoi}
                onOpenDownloadCenter={(taskId?: string) => {
                  if (taskId) setDrawerFocusTask((prev) => ({ taskId, nonce: (prev?.nonce ?? 0) + 1 }));
                  setDownloadDrawerOpen(true);
                }}
                onOpenSettings={(cat) => {
                  if (cat) setSettingsCategory((cat === "accounts" ? "auth" : cat) as SettingsCategory);
                  navigate("settings");
                }}
                onDrawRectRequest={() => {
                  setAgentPanelOpen(true);
                  setDrawMode("rect");
                  setDrawActive(true);
                }}
                onLocateAdminBoundary={() => {
                  adminModalCallerRef.current = "agent";
                  setShowAdminModal(true);
                }}
                panelOpen={agentPanelOpen}
                onPanelOpenChange={setAgentPanelOpen}
                scenes={agentEffectiveScenes}
                batchTotalCount={agentEffectiveBatchTotalCount}
                focusBoundsRequest={agentFocusBoundsRequest}
                rowStates={agentEffectiveRowStates}
                selectedSceneId={agentEffectiveSelectedSceneId}
                onSelectScene={handleAgentSelectScene}
                checkedSceneIds={checkedSceneIds}
                onToggleCheckScene={handleToggleCheckScene}
                onToggleCheckAll={handleToggleCheckAll}
                onRowAction={handleRowAction}
                creatingItemIds={creatingItemIds}
                onDownloadChecked={handleDownloadChecked}
                downloadBusy={downloadBusy}
                workingGeometry={agentEffectiveGeometry}
                geodataRasterOverlay={agentIsIsolated ? null : geodataRasterOverlay}
                batchId={agentEffectiveBatchId}
                sessionBatches={agentIsIsolated ? (agentSessionWorkspace?.batches ?? []) : undefined}
                activeBatchFilter={agentIsIsolated ? (agentSessionWorkspace?.activeBatchFilter ?? "all") : undefined}
                onSelectBatchFilter={agentIsIsolated ? handleSelectAgentBatchFilter : undefined}
                onOpenInMapWorkbench={handleOpenAgentInMapWorkbench}
                onSyncWorkingAoi={handleAgentSyncWorkingAoi}
                drawMode={drawMode}
                drawActive={drawActive}
                onDrawModeChange={setDrawMode}
                onDrawActiveChange={setDrawActive}
                onClearLayers={handleAgentClearAoi}
                onRectDraw={(bbox) => void handleAgentRectDraw(bbox)}
                onPolygonDraw={handleAgentPolygonDraw}
                onPointDraw={(lat, lng) => void handleAgentPointDraw(lat, lng)}
                mapLayerKey={mapLayerKey}
                onMapLayerChange={setMapLayerKey}
                onOpenTileDownloadWorkbench={handleOpenTileDownloadWorkbench}
                forcedSubscene={
                  import.meta.env.DEV && (window.location.hash.includes("subscene=") || window.location.search.includes("subscene="))
                    ? (((window.location.hash.includes("subscene=") ? window.location.hash : window.location.search).split("subscene=")[1]?.split("&")[0]?.split("#")[0]) as any)
                    : null
                }
              />
            </div>
          )}

          {effectivePageRoute === "history" && (
            <div className={cn("flex flex-1 min-w-0 h-full overflow-hidden", route === "settings" && "hidden")}>
              <AgentHistoryPage
                onOpenSession={(sessionId) => {
                  setAgentOpenSessionId(sessionId);
                  agentSessionMemory.remember(sessionId);
                  setHasActiveAgentSession(true);
                  handleAgentActiveSessionChangeIsolated(sessionId);
                  navigate("agent");
                }}
                onSessionDeleted={(deletedId) => {
                  if (agentOpenSessionId === deletedId) {
                    setAgentOpenSessionId(null);
                  }
                  if (agentSessionMemory.recall() === deletedId) {
                    agentSessionMemory.clear();
                    setHasActiveAgentSession(false);
                  }
                }}
              />
            </div>
          )}

          {effectivePageRoute === "downloads" && (
            <div className={cn("flex flex-1 min-w-0 h-full overflow-hidden", route === "settings" && "hidden")}>
              <DownloadCenterPage
                tasks={c1Tasks}
                onOpenFolder={handleOpenTaskOutput}
                onHistoricalMutationApplied={() => taskOverviewSource.refreshNow()}
                newTaskPolicy={sarDownloadPolicy ? { concurrency: sarDownloadPolicy.max_concurrent_items,
                  layout: sarDownloadPolicy.organize_by_product ? "sar_dataset_processing" : "direct" } : null}
                pendingPreparations={pendingDownloadPreparations}
                onDismissPreparation={(id) => setPendingDownloadPreparations((prev) => prev.filter((item) => item.id !== id))}
                onConfigureAccount={(accountId) => {
                  dispatchPendingAccountCardFocus(accountId);
                  setSettingsCategory("auth");
                  navigate("settings");
                }}
              />
            </div>
          )}

          {effectivePageRoute === "workbench" && (
            <div className={cn("flex flex-1 min-w-0 h-full overflow-hidden flex-col", route === "settings" && "hidden")}>
              {/* Failover Banner */}
              {failoverBanner && (
                <div
                  data-testid="failover-notice-banner"
                  className="shrink-0 px-3.5 py-2.5 bg-sp-warning-bg border-b border-sp-line flex items-center justify-between gap-3 text-xs text-sp-text animate-in fade-in duration-150"
                >
                  <div className="flex items-center gap-2 min-w-0 flex-1">
                    <AlertTriangle className="h-4 w-4 text-sp-warning-text shrink-0" />
                    <span className="font-medium leading-tight">
                      {failoverBanner.message}
                    </span>
                  </div>
                  <div className="flex items-center gap-2 shrink-0">
                    {failoverBanner.options.map((opt) => (
                      <button
                        key={opt.provider}
                        type="button"
                        data-testid={`failover-btn-${opt.provider}`}
                        onClick={() => handleFailoverSwitch(opt.provider)}
                        className="px-3 py-1 rounded-sp-sm bg-sp-accent-fill hover:opacity-90 text-white font-medium text-xs transition-colors shadow-xs cursor-pointer"
                      >
                        {opt.label}
                      </button>
                    ))}
                    <button
                      type="button"
                      onClick={() => setFailoverBanner(null)}
                      className="p-1 rounded text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer"
                      title="关闭提示"
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  </div>
                </div>
              )}

              <ManualWorkbench
              hideMultiProviderSelector
              forceExpandSearchSignal={tourExpandSearchSignal}
              onConditionRemoved={handleConditionRemoved}
              scientificMode={scientificMode}
              onScientificModeChange={(nextMode) => {
                if (nextMode !== scientificMode) {
                  clearUnpinnedGeeMapLayers();
                }
                if (nextMode === "dem") {
                  if (!activeResourceIdRef.current.startsWith("dem.")) {
                    lastImageResourceIdRef.current = activeResourceIdRef.current;
                  }
                  const demResId = DEM_CODE_TO_RESOURCE_ID[demSource] || "dem.cop30.v1";
                  activeResourceIdRef.current = demResId;
                  setActiveResourceId(demResId);
                  setScientificMode("dem");
                  return;
                }
                if (nextMode === "sar" && activeResourceIdRef.current.startsWith("dem.")) {
                  const restored = lastImageResourceIdRef.current || "science.sentinel-1.standard.v1";
                  setScientificMode("sar");
                  void handleSelectSource(restored).catch(() => {});
                  return;
                }
                setScientificMode(nextMode);
              }}
              onResetParams={handleResetParams}
              availableSources={availableSources}
              activeResourceId={activeResourceId}
              onSelectSource={(resourceId) => {
                void handleSelectSource(resourceId).catch(() => {});
              }}
              platforms={platforms}
              onPlatformsChange={setPlatforms}
              productType={productType}
              onProductTypeChange={setProductType}
              polarizations={polarizations}
              onPolarizationsChange={setPolarizations}
              beamModes={beamModes}
              onBeamModesChange={setBeamModes}
              lookDirection={lookDirection}
              onLookDirectionChange={setLookDirection}
              opticalDraft={opticalDrafts[activeResourceId]}
              onOpticalDraftChange={handleOpticalDraftChange}
              workingAoi={workingAoi}
              workingGeometry={workingGeometry}
              aoiLabel={aoiLabel}
              onReplaceAoi={(geojson, displayName, bboxTuple) => {
                adminModalCallerRef.current = "workbench";
                return handleSelectAdminBoundary(geojson, displayName, bboxTuple);
              }}
              onImportVectorSuccess={(geometry, bbox, label) => {
                setWorkingGeometry(geometry);
                setWorkingAoiState(bbox);
                setAoiLabel(label);
              }}
              startDate={startDate}
              endDate={endDate}
              onStartDateChange={setStartDate}
              onEndDateChange={setEndDate}
              orbitDirection={orbitDirection}
              onOrbitDirectionChange={setOrbitDirection}
              relativeOrbit={relativeOrbit}
              onRelativeOrbitChange={setRelativeOrbit}
              path={path}
              onPathChange={setPath}
              frame={frame}
              onFrameChange={setFrame}
              searching={searching}
              searchError={searchError}
              onTriggerSearch={handleTriggerSearch}
              onRefreshSearch={() => void handleTriggerSearch({ bypassCache: true })}
              activeSourceView={activeSourceView}
              frozenSearchRequest={activeSourceView?.batch_id ? batchSearchRequests.get(activeSourceView.batch_id) ?? null : null}
              demSource={demSource}
              onDemSourceChange={(nextDem) => {
                setDemSource(nextDem);
                if (scientificMode === "dem") {
                  const mappedResId = DEM_CODE_TO_RESOURCE_ID[nextDem];
                  if (mappedResId) {
                    activeResourceIdRef.current = mappedResId;
                    setActiveResourceId(mappedResId);
                  }
                }
              }}
              autoConvertEllipsoid={autoConvertEllipsoid}
              onToggleAutoConvertEllipsoid={() => setAutoConvertEllipsoid(!autoConvertEllipsoid)}
              orbitSourceType={orbitSourceType}
              onOrbitSourceTypeChange={setOrbitSourceType}
              localSlcPath={localSlcPath}
              onChangeLocalSlcPath={setLocalSlcPath}
              onPickLocalSlcPath={handlePickLocalSlc}
              localDemPath={localDemPath}
              onChangeLocalDemPath={setLocalDemPath}
              onPickLocalDemPath={handlePickLocalDem}
              inlineToolResult={inlineToolResult}
              onDismissToolResult={() => setInlineToolResult(null)}
              searchProgress={searchProgress?.resource_id === activeResourceId ? searchProgress : null}
              scenes={displayedWorkbenchScenes}
              checkedScenes={checkedScenes}
              sourceViewLoading={sourceViewLoading}
              sourceViewError={sourceViewError}
              onRetrySourceView={() => void handleSelectSource(activeResourceId).catch(() => {})}
              resultTotalCount={displayedWorkbenchTotalCount}
              loadingMore={loadingMore}
              loadMoreError={loadMoreError}
              onLoadMore={handleLoadMore}
              batchDelivery={activeDelivery}
              pageWaiting={pageWaitingAt !== null}
              onCancelSearch={handleCancelSearch}
              cancellingSearch={cancellingSearch}
              rowStates={rowStates}
              tasksById={c1TasksById}
              taskOutcomeNotice={taskOutcomeNotice}
              onDismissTaskOutcomeNotice={() => setTaskOutcomeNotice(null)}
              selectedSceneId={selectedSceneId}
              onSelectScene={setSelectedSceneId}
              checkedSceneIds={checkedSceneIds}
              onClearChecked={() => {
                checkedScenesRef.current.clear();
                setCheckedSceneIds(new Set());
                setAllMatchingSelected(false);
                setExcludedSceneIds(new Set());
              }}
              onClearResults={handleClearResults}
              onToggleCheckScene={handleToggleCheckScene}
              onToggleCheckAll={handleToggleCheckAll}
              allMatchingSelected={allMatchingSelected}
              excludedSceneIds={excludedSceneIds}
              onToggleSelectCurrentPage={handleToggleSelectCurrentPage}
              onToggleSelectAllMatching={handleToggleSelectAllMatching}
              onRowAction={handleRowAction}
              onDownloadChecked={handleDownloadChecked}
              downloadBusy={downloadBusy}
              creatingItemIds={creatingItemIds}
              batchCreating={batchCreating}
              onOpenNisarSubsetWorkspace={() => void handleOpenNisarSubsetWorkspace()}
              nisarSubsetBusy={nisarSubsetOpenBusy}
              nisarSubsetDisabledReason={
                isNisarLogicalResourceId(activeResourceId) &&
                resolveNisarProductLevel(productType || getWorkbenchToolOptions().nisarProductLevel) === "SME2"
                  ? "NISAR SME2 属于 L3 土壤湿度粗分辨率网格产品，请直接点击「加入下载队列」获取完整文件。"
                  : null
              }
              onSyncGeeResults={handleSyncGeeResults}
              onPreviewGeeScene={(scene) => void handlePreviewGeeScene(scene)}
              gacosBatchStatuses={gacosBatchStatuses}
              selectedGacosBatchIndices={selectedGacosBatchIndices}
              onToggleGacosBatchIndex={(batchIdx) => {
                const validDates = parseGacosManualDatesList(getWorkbenchToolOptions().gacosDatesText || "").validDates;
                const total = Math.max(1, splitGacosDatesIntoBatches(validDates, 20).length);
                setSelectedGacosBatchIndices((prev) => {
                  const base = prev ? new Set(prev) : new Set(Array.from({ length: total }, (_, i) => i));
                  if (base.has(batchIdx)) base.delete(batchIdx);
                  else base.add(batchIdx);
                  return Array.from(base).sort((a, b) => a - b);
                });
              }}
              onSelectAllGacosBatches={(total) => {
                setSelectedGacosBatchIndices(Array.from({ length: total }, (_, i) => i));
              }}
              onRetryGacosBatch={(batchIdx, batchDates) => {
                if (!workingGeometry || !workingAoi) return;
                const toolOpts = getWorkbenchToolOptions();
                const targetGacosDir =
                  nonTemporaryOutputDir(toolOpts.openGeodataOutputDir) ||
                  nonTemporaryOutputDir(sarDownloadPolicy?.default_output_root) ||
                  nonTemporaryOutputDir(storageRootPath) ||
                  ".";
                setGacosSubmitting(true);
                void executeStandaloneGacosRequest({
                  outputRoot: targetGacosDir,
                  manualDatesText: batchDates.join("\n"),
                  utcHour: Number.parseInt(toolOpts.gacosUtcHour || "10", 10),
                  utcMinute: Number.parseInt(toolOpts.gacosUtcMinute || "0", 10),
                  outputFormat: toolOpts.gacosOutputFormat || "geotiff",
                  studyAreaName: aoiLabel,
                  studyAreaGeometry: workingGeometry,
                  studyAreaBbox: workingAoi,
                  onBatchStatusChange: (_subIdx, state) => {
                    setGacosBatchStatuses((prev) => {
                      const next = new Map(prev);
                      next.set(batchIdx, state);
                      return next;
                    });
                  },
                })
                  .then((outcome) => {
                    if (outcome.ok) {
                      void taskOverviewSource.refreshNow().catch(() => {});
                    }
                  })
                  .finally(() => setGacosSubmitting(false));
              }}
              gacosSubmitting={gacosSubmitting}
              canBatchOpticalAoiClip
              opticalAoiClipDisabledReason={opticalAoiClipDisabledReason}
              onOpenBatchOpticalAoiClip={handleOpenBatchOpticalAoiClip}
              canRowOpticalAoiClip={canRowOpticalAoiClip}
              onOpenRowOpticalAoiClip={handleOpenRowOpticalAoiClip}
              drawMode={drawMode}
              drawActive={drawActive}
              onDrawModeChange={setDrawMode}
              onDrawActiveChange={setDrawActive}
              onClearLayers={handleClearAoi}
              onRectDraw={(bbox) => void handleAoiChange(bbox)}
              onPolygonDraw={handlePolygonDraw}
              onPointDraw={(lat, lng) => void handlePointDraw(lat, lng)}
              mapLayerKey={mapLayerKey}
              onLayerChange={setMapLayerKey}
              onOpenTileDownloadWorkbench={handleOpenTileDownloadWorkbench}
              geodataRasterOverlay={waybackPreviewOverlay ?? geodataRasterOverlay}
              onPreviewWaybackOverlay={setWaybackPreviewOverlay}
              inlineLoginError={inlineLoginError}
              onCloseInlineLogin={() => {
                setInlineLoginError(null);
                setPendingRetryAction(null);
              }}
              onSuccessInlineLogin={async () => {
                const retry = pendingRetryAction;
                setPendingRetryAction(null);
                setInlineLoginError(null);
                const current = batchFor(activeResourceIdRef.current);
                if (retry && retry.generation === sourceSwitchGenerationRef.current
                  && displayedBatchRef.current === retry.batchId
                  && current?.batch_id === retry.batchId && current.revision === retry.revision) {
                  await executeDownloadTask(retry);
                }
              }}
              onOpenSettings={() => {
                setInlineLoginError(null);
                setPendingRetryAction(null);
                setSettingsCategory("auth");
                navigate("settings");
              }}
              onOpenCatalog={(opts) => handleOpenDataCatalog(opts)}
              companionState={companionState}
              onCompanionStateChange={handleCompanionStateChange}
              onTriggerStandaloneCompanion={handleTriggerStandaloneCompanion}
              standaloneCompanionBusy={standaloneCompanionBusy}
            />
            </div>
          )}

          {effectivePageRoute === "plugins" && (
            <div className={cn("flex flex-1 min-w-0 h-full overflow-hidden", route === "settings" && "hidden")}>
              <PluginsAndSkillsPage
                onNavigate={navigate}
                onOpenRuntimeSettings={() => {
                  setSettingsCategory("runtime_tools");
                  navigate("settings");
                }}
              />
            </div>
          )}

          {route === "settings" && (
            <div className="flex flex-1 min-w-0 h-full overflow-hidden">
              <SettingsModal
                variant="page"
                open={true}
                hideNav={true}
                category={settingsCategory}
                onCategoryChange={setSettingsCategory}
                onClose={handleCloseSettings}
                initialCategory={settingsCategory}
                dark={dark}
                onToggleDark={onToggleDark}
                fontSize={shellFontSize}
                onFontSizeChange={changeShellFontSize}
                cdsePanel={<DesktopCdseAccountPanel onChanged={refreshCreds} />}
                usgsPanel={<DesktopUsgsAccountPanel onChanged={refreshCreds} />}
                bhoonidhiPanel={<DesktopBhoonidhiAccountPanel onChanged={refreshCreds} />}
                earthdataPanel={<DesktopEarthdataAccountPanel onChanged={refreshCreds} />}
                storagePanel={
                  <StorageSettingsTab
                    storageRootPath={storageRootPath}
                    onChangeStoragePath={handlePickStoragePath}
                    onSaveStoragePath={handleSaveStoragePath}
                  />
                }
                tourOpenAddProviderSignal={tourOpenAddProviderSignal}
                onSyncDemConcurrency={(limit) => handleSaveDemConcurrency(limit)}
              />
            </div>
          )}

          {/* 行政区划边界定位模态对话框 */}
          <AdminBoundaryModal
            open={showAdminModal}
            onClose={() => {
              adminModalCallerRef.current = "workbench";
              setShowAdminModal(false);
            }}
            onSelectBoundary={handleSelectAdminBoundary}
          />

          <EOStageLoginModal
            open={eostageLoginOpen}
            onClose={() => setEostageLoginOpen(false)}
            onSuccess={(next) => setEostageAccount(next)}
          />

          <EOStageBillingModal
            open={eostageBillingState.open}
            initialTab={eostageBillingState.tab}
            account={eostageAccount}
            onClose={() => setEostageBillingState((prev) => ({ ...prev, open: false }))}
            onOpenOfficialModelSettings={() => {
              setSettingsCategory("models");
              navigate("settings");
            }}
          />

          {/* 全局数据清单模态对话框（支持从左侧栏「数据清单」、顶栏菜单或 Ctrl+K 直接打开） */}
          <DataSourceCatalogModal
            isOpen={dataCatalogOpen}
            lockedToGee={dataCatalogGeeOnly}
            onClose={() => {
              setDataCatalogOpen(false);
              setDataCatalogGeeOnly(false);
            }}
            availableSources={availableSources}
            activeResourceId={activeResourceId}
            onSelectSource={(resourceId) => {
              void handleSelectSource(resourceId).catch(() => {});
              if (effectivePageRoute !== "workbench") {
                navigate("workbench");
              }
            }}
            workingBbox={workingAoi}
            onOpenAccountSettings={() => {
              setDataCatalogOpen(false);
              setSettingsCategory("auth");
              navigate("settings");
            }}
          />

          <GlobalCommandPalette
            open={commandPaletteOpen}
            onOpenChange={setCommandPaletteOpen}
            enableGlobalShortcut
            onNavigate={(nextRoute) => navigate(nextRoute)}
            onOpenScientificMode={(mode, presetTool) => {
              navigate("workbench");
              if (mode === "convert") {
                if (presetTool === "sarscape" || presetTool === "ellipsoid") {
                  updateWorkbenchToolOptions({ convertOutputMode: presetTool });
                }
                setScientificMode("convert");
                return;
              }
              if (mode === "dem") {
                if (!activeResourceIdRef.current.startsWith("dem.")) {
                  lastImageResourceIdRef.current = activeResourceIdRef.current;
                }
                const demResId = DEM_CODE_TO_RESOURCE_ID[demSource] || "dem.cop30.v1";
                activeResourceIdRef.current = demResId;
                setActiveResourceId(demResId);
                setScientificMode("dem");
                return;
              }
              setScientificMode(mode);
            }}
            onOpenSettingsCategory={(cat) => {
              setSettingsCategory(cat);
              navigate("settings");
            }}
            onOpenSession={(sessionId) => {
              handleOpenSessionFromNav(sessionId);
            }}
            onNewChat={() => {
              handleStartNewChat();
            }}
            onSelectDataSource={(resourceId) => {
              void handleSelectSource(resourceId).catch(() => {});
              if (effectivePageRoute !== "workbench" && effectivePageRoute !== "agent") {
                navigate("workbench");
              }
            }}
            onSelectSkill={() => {
              navigate("plugins");
            }}
            onOpenTask={(taskId) => {
              setDrawerFocusTask((prev) => ({ taskId, nonce: (prev?.nonce ?? 0) + 1 }));
              navigate("downloads");
            }}
            onOpenLocalPath={async (targetPath) => {
              try {
                await openPath(targetPath);
              } catch {
                setTaskOutcomeNotice({
                  tone: "danger",
                  text: translate("mapWorkbench.desktop.openOutputFailed"),
                });
              }
            }}
            sessions={paletteSessions}
            tasks={c1Tasks}
            localPaths={commandPaletteLocalPaths}
          />
        </div>
      </div>

      {/* 行下载按批次确认；完整产品下载统一走精简目的目录确认框，与 AOI 裁剪弹窗彻底分离 */}
      {downloadConfirmationDraft && (
        <DownloadDestinationConfirmation
          key={`${downloadConfirmationDraft.sourceBatchId}:${downloadConfirmationDraft.sourceBatchRevision}:${downloadConfirmationDraft.selectionScope}:${downloadConfirmationDraft.selectedProductIds.join(",")}`}
          draft={downloadConfirmationDraft}
          activeResourceId={activeResourceId}
          aoiBbox={workingAoi}
          aoiGeometry={workingGeometry}
          selectedScenes={downloadConfirmationDraft.selectedProductIds
            .map(
              (id) =>
                scenes.find((s) => s.scene_id === id || s.product_key === id) ??
                checkedScenesRef.current.get(id) ??
                findAgentSceneBatchOwner(
                  effectivePageRoute === "agent"
                    ? peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current)
                    : null,
                  id
                )?.scene
            )
            .filter((s): s is SceneRow => Boolean(s))}
          chooseDirectory={async () => {
            const picked = await pickDirectory(translate("mapWorkbench.desktop.chooseTaskOutput"));
            if (!picked.ok) throw new Error(translate("mapWorkbench.desktop.chooseDirectoryFailed"));
            return picked.path?.trim() || null;
          }}
          duplicateMatches={downloadDuplicateMatches}
          duplicatePolicy={downloadDuplicatePolicy}
          onDuplicatePolicyChange={setDownloadDuplicatePolicy}
          onOutputRootChange={setDownloadTargetOutputDir}
          onCancel={() => setDownloadConfirmationDraft(null)}
          onOpenAccountSettings={(accountId) => {
            dispatchPendingAccountCardFocus(accountId);
            setDownloadConfirmationDraft(null);
            setSettingsCategory("auth");
            navigate("settings");
          }}
          onConfirmSarCompanions={async ({ includeDem, outputDir }) => {
            setSavedCompanionDemPreference(activeResourceIdRef.current, includeDem);
            setCompanionState((prev) => ({ ...prev, dem: includeDem }));
            if (!includeDem) return;
            const allKnownScenes = [
              ...scenesRef.current,
              ...Array.from(checkedScenesRef.current.values()),
            ];
            const demBbox = resolveAuxiliaryExtent(
              companionStateRef.current.extentMode,
              workingAoi,
              allKnownScenes
            );
            if (demBbox) {
              await startDemDownloadBbox(
                demBbox.west,
                demBbox.east,
                demBbox.south,
                demBbox.north,
                outputDir,
                "COP30",
                "auto",
                true,
              ).catch(() => {});
              void taskOverviewSource.refreshNow().catch(() => {});
            }
          }}
          onConfirm={async (confirmed) => {
            if (confirmed.acquisition.frozenParameters?.dedicated_geodata === true) {
              const frozen = confirmed.acquisition.frozenParameters;
              const dedicatedAction = typeof frozen?.dedicated_action === "string"
                ? frozen.dedicated_action
                : "";
              const targetIds = [...confirmed.selectedProductIds];
              const cleanOut = confirmed.outputRoot.trim();
              if (cleanOut) {
                if (dedicatedAction === "xyz_tiles") {
                  updateWorkbenchToolOptions({ xyzTileOutputDir: cleanOut });
                } else if (dedicatedAction === "dem_download") {
                  updateWorkbenchToolOptions({ demOutputDir: cleanOut });
                } else if (dedicatedAction === "orbit_download") {
                  updateWorkbenchToolOptions({ orbitOutputDir: cleanOut });
                } else {
                  updateWorkbenchToolOptions({ openGeodataOutputDir: cleanOut });
                }
              }
              setDownloadConfirmationDraft(null);
              if (
                dedicatedAction === "xyz_tiles" ||
                dedicatedAction === "dem_download" ||
                dedicatedAction === "orbit_download"
              ) {
                setDownloadDrawerOpen(true);
                setDownloadBusy(true);
                try {
                  await handleTriggerSearch({
                    forceDownload: true,
                    overrideOutputDir: cleanOut,
                  });
                  void taskOverviewSource.refreshNow().catch(() => {});
                } finally {
                  setDownloadBusy(false);
                }
                return;
              }
              if (targetIds.length === 0) return;
              const prepId = `prep-geodata-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
              setPendingDownloadPreparations((prev) => [
                {
                  id: prepId,
                  title: humanPreparationTitle(activeResourceIdRef.current),
                  count: targetIds.length,
                  outputRoot: cleanOut,
                  status: "preparing",
                },
                ...prev,
              ]);
              setDownloadDrawerOpen(true);
              setDownloadBusy(true);
              try {
                for (const id of targetIds) {
                  await handleTriggerSearch({
                    forceDownload: true,
                    targetItemId: id,
                    overrideOutputDir: cleanOut,
                  });
                }
                await taskOverviewSource.refreshNow().catch(() => {});
                setPendingDownloadPreparations((prev) => prev.filter((item) => item.id !== prepId));
                setCheckedSceneIds((prev) => {
                  const next = new Set(prev);
                  for (const id of targetIds) next.delete(id);
                  return next;
                });
              } catch (cause) {
                const message = cause instanceof Error && cause.message.trim()
                  ? cause.message
                  : translate("mapWorkbench.desktop.actionFailed");
                setPendingDownloadPreparations((prev) =>
                  prev.map((item) => item.id === prepId ? { ...item, status: "failed", error: message } : item),
                );
              } finally {
                setDownloadBusy(false);
              }
              return;
            }
            const { itemIds, skipIds } = await resolveDownloadIdsAfterDuplicatePolicy(
              [...confirmed.selectedProductIds],
              confirmed.outputRoot,
            );
            if (confirmed.selectionScope !== "all_matching" && itemIds.length === 0) {
              setDownloadConfirmationDraft(null);
              setTaskOutcomeNotice({
                tone: "neutral",
                text: "所选影像已全部存在于目标目录，已按「跳过已有文件」策略略过。",
              });
              return;
            }
            const mergedExcludedIds = Array.from(
              new Set([
                ...(confirmed.excludedProductIds ?? []),
                ...skipIds,
              ]),
            );
            const agentConfirmBatchMatch =
              effectivePageRouteRef.current === "agent"
                ? findAgentBatchEntry(confirmed.sourceBatchId)
                : null;
            const current = batchFor(activeResourceIdRef.current);
            if (
              !agentConfirmBatchMatch &&
              (displayedBatchRef.current !== confirmed.sourceBatchId ||
                current?.batch_id !== confirmed.sourceBatchId ||
                current.revision !== confirmed.sourceBatchRevision)
            ) {
              throw new Error(translate("mapWorkbench.desktop.rowSelectionExpired"));
            }
            if (confirmed.selectionScope === "exact_item" && !itemIds[0]) {
              throw new Error(translate("mapWorkbench.desktop.rowSelectionExpired"));
            }
            const includePreciseOrbit = confirmed.includePreciseOrbit
              ?? (typeof confirmed.acquisition.frozenParameters?.download_orbit === "boolean"
                ? confirmed.acquisition.frozenParameters.download_orbit
                : undefined);
            const includeGacos = confirmed.includeGacos
              ?? (typeof confirmed.acquisition.frozenParameters?.request_gacos === "boolean"
                ? confirmed.acquisition.frozenParameters.request_gacos
                : undefined);
            if (includePreciseOrbit !== undefined) {
              const sourceKey = typeof confirmed.acquisition.frozenParameters?.source_key === "string"
                ? confirmed.acquisition.frozenParameters.source_key
                : "";
              if (sourceKey) {
                setPreciseOrbitPreference(sourceKey, includePreciseOrbit);
              }
              setSavedCompanionOrbitPreference(activeResourceIdRef.current, includePreciseOrbit);
              setCompanionState((prev) => ({ ...prev, preciseOrbit: includePreciseOrbit }));
            }
            if (includeGacos !== undefined) {
              setSavedCompanionGacosPreference(activeResourceIdRef.current, includeGacos);
              setCompanionState((prev) => ({ ...prev, gacos: includeGacos }));
            }
            const opticalParams = confirmed.acquisition.frozenParameters;
            const opticalDelivery: OpticalConfirmationDelivery | undefined =
              confirmed.acquisition.kind === "sentinel2"
                ? {
                    kind: opticalParams.product_level === "l1c" ? "sentinel2_l1c" : "sentinel2_l2a",
                    deliveryForm:
                      opticalParams.delivery_form === "analysis_bands"
                        ? "analysis_bands"
                        : "safe_archive",
                    bandKeys: Array.isArray(opticalParams.selected_band_keys)
                      ? opticalParams.selected_band_keys.filter(
                          (value): value is string => typeof value === "string",
                        )
                      : [],
                  }
                : confirmed.acquisition.kind === "hls"
                  ? {
                      kind: "hls",
                      deliveryForm: "analysis_bands",
                      bandKeys: Array.isArray(opticalParams.selected_band_keys)
                        ? opticalParams.selected_band_keys.filter(
                            (value): value is string => typeof value === "string",
                          )
                        : [],
                    }
                  : undefined;
            if (agentConfirmBatchMatch && activeAgentSessionIdRef.current && confirmed.outputRoot.trim()) {
              agentSessionManualOutputDirsRef.current.set(
                activeAgentSessionIdRef.current,
                confirmed.outputRoot.trim(),
              );
            }
            setConfirmedDownloadSettings({
              batchId: confirmed.sourceBatchId,
              outputRoot: confirmed.outputRoot,
              concurrency: confirmed.concurrency,
              layout: confirmed.layout,
              ...(includePreciseOrbit !== undefined ? { includePreciseOrbit } : {}),
              ...(includeGacos !== undefined ? { includeGacos } : {}),
              ...(typeof companionStateRef.current.dem === "boolean"
                ? { includeDem: companionStateRef.current.dem }
                : {}),
              ...(opticalDelivery ? { opticalDelivery } : {}),
            });
            setDownloadConfirmationDraft(null);
            setDownloadDrawerOpen(true);
            await executeDownloadTask({
              batchId: confirmed.sourceBatchId,
              revision: confirmed.sourceBatchRevision,
              itemIds,
              outputRoot: confirmed.outputRoot,
              concurrency: confirmed.concurrency,
              layout: confirmed.layout,
              includePreciseOrbit,
              includeGacos,
              scope: confirmed.selectionScope === "all_matching"
                ? "all_matching"
                : confirmed.selectionScope === "checked"
                ? "checked"
                : "row",
              excludedItemIds: mergedExcludedIds.length > 0 ? mergedExcludedIds : undefined,
              selectionDigest: confirmed.allMatchPreview?.selection_digest,
              admissibleCount: confirmed.allMatchPreview?.admissible_count,
              overrideResourceId: agentConfirmBatchMatch?.resourceId,
              opticalDelivery,
            });
          }}
        />
      )}

      {/* Landsat C2 L2 (MPC) 下载确认卡：COG 文件集与 USGS .tar 归档由用户显式选择，确认后直接进入下载中心 */}
      {landsatDownloadRequest && (() => {
        const req = landsatDownloadRequest;
        const resolvedLandsatScenes = req.itemIds
          .map(
            (id) =>
              scenes.find((s) => s.scene_id === id || s.product_key === id) ??
              checkedScenesRef.current.get(id) ??
              findAgentSceneBatchOwner(
                effectivePageRoute === "agent"
                  ? peekAgentSessionWorkspaceState(activeAgentSessionIdRef.current)
                  : null,
                id
              )?.scene
          )
          .filter((s): s is SceneRow => Boolean(s));
        return (
          <LandsatDownloadConfirmation
            request={req}
            activeResourceId="science.landsat.c2-l2.v1"
            selectedScenes={resolvedLandsatScenes}
            duplicateMatches={downloadDuplicateMatches}
            duplicatePolicy={downloadDuplicatePolicy}
            onDuplicatePolicyChange={setDownloadDuplicatePolicy}
            onOutputRootChange={setDownloadTargetOutputDir}
            onOpenAccountSettings={(accountId) => {
              dispatchPendingAccountCardFocus(accountId);
              setLandsatDownloadRequest(null);
              setSettingsCategory("auth");
              navigate("settings");
            }}
            isCurrent={() => {
              if (req.isAgentSessionBatch) return true;
              const current = batchFor(activeResourceIdRef.current);
              return (
                sourceSwitchGenerationRef.current === req.sourceGeneration &&
                displayedBatchRef.current === req.batchContext.batch_id &&
                current?.batch_id === req.batchContext.batch_id &&
                current.revision === (req.batchContext.revision ?? 0)
              );
            }}
            onClose={() => setLandsatDownloadRequest((current) => (current === req ? null : current))}
            onDeferToDownloadCenter={(confirmed, execute) => {
              const prepId = `prep-landsat-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
              const deliveryForm = String(
                confirmed.acquisition.frozenParameters?.delivery_form ?? "complete_asset_set"
              );
              const title =
                deliveryForm === "official_archive"
                  ? "Landsat C2 L2 · 官方原始归档 (.tar)"
                  : "Landsat C2 L2 · 完整产品（展开文件集）";
              setPendingDownloadPreparations((prev) => [
                {
                  id: prepId,
                  title,
                  count: confirmed.selectedProductIds.length || req.itemIds.length || 1,
                  outputRoot: confirmed.outputRoot,
                  status: "preparing",
                },
                ...prev,
              ]);
              setDownloadDrawerOpen(true);
              void (async () => {
                try {
                  await execute();
                  await taskOverviewSource.refreshNow().catch(() => {});
                  setPendingDownloadPreparations((prev) => prev.filter((item) => item.id !== prepId));
                } catch (cause) {
                  const message =
                    cause instanceof Error && cause.message.trim()
                      ? cause.message
                      : translate("mapWorkbench.landsat.createFailed");
                  const accountAction = resolveAccountActionFromError(cause, "science.landsat.c2-l2.v1");
                  setPendingDownloadPreparations((prev) =>
                    prev.map((item) =>
                      item.id === prepId
                        ? {
                            ...item,
                            status: "failed",
                            error: message,
                            ...(accountAction.actionAccountId ? { actionAccountId: accountAction.actionAccountId } : {}),
                            ...(accountAction.actionLabel ? { actionLabel: accountAction.actionLabel } : {}),
                          }
                        : item
                    )
                  );
                  setTaskOutcomeNotice({ tone: "danger", text: message });
                }
              })();
            }}
            onTaskCreated={async ({ admittedIds, rejectedCount }) => {
              void taskOverviewSource.refreshNow().catch(() => {});
              if (sourceSwitchGenerationRef.current === req.sourceGeneration) {
                const current = batchFor(activeResourceIdRef.current);
                if (
                  req.isAgentSessionBatch ||
                  (displayedBatchRef.current === req.batchContext.batch_id &&
                    current?.batch_id === req.batchContext.batch_id &&
                    current.revision === (req.batchContext.revision ?? 0))
                ) {
                  setTaskOutcomeNotice({
                    tone: "neutral",
                    text:
                      rejectedCount === 0
                        ? translate("mapWorkbench.desktop.landsatCreated")
                        : translate("mapWorkbench.desktop.landsatPartialAdmission"),
                  });
                  if (!req.isAgentSessionBatch) {
                    void refreshLoadedRowStates(
                      req.batchContext.batch_id,
                      req.batchContext.revision ?? 0
                    ).catch((refreshErr) => {
                      console.warn("Refresh batch rows after Landsat task creation error:", refreshErr);
                    });
                  }
                  setCheckedSceneIds((previous) => {
                    const next = new Set(previous);
                    for (const id of admittedIds) next.delete(id);
                    return next;
                  });
                }
              }
            }}
          />
        );
      })()}

      {/* 光学 COG 按需 AOI 裁剪 / 近同期镶嵌弹窗 */}
      {opticalAoiClipSession && (() => {
        const session = opticalAoiClipSession;
        const sessionKey = `${session.batchId}:${session.revision}:${session.scenes.map((s) => s.scene_id).join(",")}`;
        return (
          <OpticalAoiClipModal
            key={sessionKey}
            sourceBatchId={session.batchId}
            expectedWorkbenchRevision={session.revision}
            scenes={session.scenes}
            initialOutputRoot={session.outputRoot}
            initialConcurrency={session.concurrency}
            aoiGeometry={session.aoiGeometry}
            aoiBbox={session.aoiBbox}
            aoiLabel={session.aoiLabel}
            onClose={() => setOpticalAoiClipSession((current) => current === session ? null : current)}
            onPrecheck={precheckOpticalAoiCoverage}
            onPickDirectory={async () => {
              const picked = await pickDirectory(translate("panels.opticalAoiClip.outputDirLabel"));
              return picked && picked.ok && picked.path ? picked.path : null;
            }}
            onSubmit={async (request) => {
              const res = await startOpticalDerivedClipOrMosaicTask(request);
              if (!res.ok) {
                throw new Error(res.error || translate("mapWorkbench.landsat.createFailed"));
              }
              setOpticalAoiClipSession((current) => current === session ? null : current);
              setDownloadDrawerOpen(true);
              void taskOverviewSource.refreshNow().catch(() => {});
              setTaskOutcomeNotice({
                tone: "neutral",
                text: translate("panels.opticalAoiClip.submitCreatedNotice", {
                  path:
                    res.output_tif_paths && res.output_tif_paths.length > 1
                      ? `${res.output_tif_paths.length} 个独立 GeoTIFF（首个：${res.output_tif_paths[0]}）`
                      : res.output_tif_path || res.task_id || "",
                }),
              });
              if (sourceSwitchGenerationRef.current === session.sourceGeneration) {
                const current = batchFor(activeResourceIdRef.current);
                if (
                  displayedBatchRef.current === session.batchId
                  && current?.batch_id === session.batchId
                  && (current.revision ?? 0) === session.revision
                ) {
                  void refreshLoadedRowStates(session.batchId, session.revision).catch(() => {});
                }
              }
            }}
          />
        );
      })()}
      {/* NISAR AOI 裁剪下载工作区与确认弹窗 */}
      {nisarSubsetWorkspaceSession && !nisarSubsetConfirmation && (
        <NisarSubsetWorkspace
          key={nisarSubsetWorkspaceSession.draft.sourceBatchId + ":" + String(nisarSubsetWorkspaceSession.sourceBatchRevision)}
          initialDraft={nisarSubsetWorkspaceSession.draft}
          rows={nisarSubsetWorkspaceSession.rows}
          sourceBatchLabel={nisarSubsetWorkspaceSession.sourceBatchLabel}
          currentAoiName={nisarSubsetWorkspaceSession.currentAoiName}
          mapBbox={nisarSubsetWorkspaceSession.mapBbox}
          onClose={() => {
            setNisarSubsetWorkspaceSession(null);
            setNisarSubsetConfirmation(null);
            setNisarSubsetPrepareError(null);
          }}
          reviewBusy={nisarSubsetPrepareBusy}
          reviewError={nisarSubsetPrepareError}
          onSearchAdministrativeAoi={async (query) => {
            const result = await searchAdminBoundaries(query, "", "", "", 8, null);
            return result.ok
              ? { ok: true, results: result.results }
              : { ok: false, error: formatBridgeError(result) };
          }}
          onResolveAdministrativeAoi={async (boundary) => {
            const result = await resolveAdminBoundary(boundary);
            if ("ok" in result && result.ok === false) {
              return { ok: false, error: formatBridgeError(result) };
            }
            const resolved = result as AdminBoundary;
            const geometry = canonicalAdministrativeAoiGeometry(resolved.geometry ?? resolved.geojson);
            return geometry
              ? { ok: true, geometry }
              : { ok: false, error: "该行政区没有可用的真实多边形边界。" };
          }}
          onImportBoundary={async () => {
            const picked = await pickOpenFile("导入裁剪工作区边界", [
              "AOI boundary (*.shp;*.kml;*.kmz;*.geojson;*.json)",
            ]);
            if (!picked.ok) return { ok: false, error: formatBridgeError(picked) };
            if (!picked.path?.trim()) return { ok: false, error: "未选择边界文件。" };
            const preview = await previewAoiFile(picked.path);
            if (!preview.ok) return { ok: false, error: formatBridgeError(preview) };
            if (!preview.geojson || preview.features.length === 0) {
              return { ok: false, error: "边界文件没有可选择的 Polygon 或 MultiPolygon 要素。" };
            }
            return { ok: true, preview };
          }}
          onPreflight={async (draft) => {
            if (!draft.requestedGeometry) throw new Error("请先选择裁剪范围。");
            const result = await preflightNisarSubsetBatchV2({
              source_batch_id: draft.sourceBatchId,
              source_item_ids: nisarSubsetWorkspaceSession.rows
                .filter((row) => draft.selectedItemIds.includes(row.itemId))
                .map((row) => row.itemId),
              expected_workbench_revision: nisarSubsetWorkspaceSession.sourceBatchRevision,
              subset_draft_geometry: structuredClone(draft.requestedGeometry) as DesktopJsonObject,
            });
            if (!result.ok) throw new Error(formatBridgeError(result));
            return {
              sourceBatchId: result.preflight.source_batch_id,
              workbenchRevision: result.preflight.workbench_revision,
              eligibleItems: result.preflight.eligible_items.map((item) => ({
                sourceItemId: item.source_item_id,
                effectiveGeometry: structuredClone(item.effective_geometry),
                availableSelections: item.available_selections.map((selection) => ({
                  frequency: selection.frequency,
                  polarizations: [...selection.polarizations],
                })),
              })),
              excludedItems: result.preflight.excluded_items.map((item) => ({
                sourceItemId: item.source_item_id,
                reason: item.reason,
              })),
              commonSelections: result.preflight.common_selections.map((selection) => ({
                frequency: selection.frequency,
                polarizations: [...selection.polarizations],
              })),
            };
          }}
          onReview={handleReviewNisarSubsetDraft}
        />
      )}
      {nisarSubsetConfirmation && (
        <NisarSubsetConfirmation
          prepared={nisarSubsetConfirmation.prepared}
          outputRoot={nisarSubsetConfirmation.taskRequest.output_root}
          taskRequest={nisarSubsetConfirmation.taskRequest}
          onChooseDirectory={async () => {
            const picked = await pickDirectory("选择 NISAR 裁剪任务输出目录");
            if (!picked.ok) throw new Error(formatBridgeError(picked));
            return picked.path?.trim() || null;
          }}
          onCancel={() => setNisarSubsetConfirmation(null)}
          onConfirm={handleConfirmNisarSubsetBatch}
        />
      )}
      {nisarMultiLevelModalSession && (
        <NisarMultiLevelSubsetModal
          productLevel={nisarMultiLevelModalSession.productLevel}
          band={nisarMultiLevelModalSession.band}
          scenes={nisarMultiLevelModalSession.scenes}
          aoiBbox={workingAoi}
          aoiLabel={aoiLabel || "当前研究区"}
          initialOutputRoot={nisarMultiLevelModalSession.outputRoot}
          batchId={nisarMultiLevelModalSession.batchId}
          batchRevision={nisarMultiLevelModalSession.batchRevision}
          rslcSelections={nisarMultiLevelModalSession.rslcSelections}
          onPreflightRslcBatch={
            nisarMultiLevelModalSession.batchId && typeof nisarMultiLevelModalSession.batchRevision === "number"
              ? async ({ aoiGeometry, targetSceneIds }) => {
                  const result = await preflightNisarSubsetBatchV2({
                    source_batch_id: nisarMultiLevelModalSession.batchId!,
                    source_item_ids: targetSceneIds,
                    expected_workbench_revision: nisarMultiLevelModalSession.batchRevision!,
                    subset_draft_geometry: structuredClone(aoiGeometry) as DesktopJsonObject,
                  });
                  if (!result.ok) throw new Error(formatBridgeError(result));
                  const commonFrequencies = result.preflight.common_selections.map(
                    (sel) => `Frequency ${sel.frequency}`,
                  );
                  const commonPolarizations = Array.from(
                    new Set(result.preflight.common_selections.flatMap((sel) => sel.polarizations)),
                  );
                  return {
                    eligibleCount: result.preflight.eligible_items.length,
                    excludedCount: result.preflight.excluded_items.length,
                    commonFrequencies,
                    commonPolarizations,
                  };
                }
              : undefined
          }
          onSubmitRslcBatch={
            nisarMultiLevelModalSession.batchId && typeof nisarMultiLevelModalSession.batchRevision === "number"
              ? async ({ frequency, polarizations: subPols, aoiGeometry, outputRoot: cleanOut, estimatedSubsetBytes, targetSceneIds }) => {
                  const preparedResult = await prepareNisarSubsetBatchV2({
                    source_batch_id: nisarMultiLevelModalSession.batchId!,
                    source_item_ids: targetSceneIds,
                    expected_workbench_revision: nisarMultiLevelModalSession.batchRevision!,
                    subset_draft_geometry: structuredClone(aoiGeometry) as DesktopJsonObject,
                    selected_frequency: frequency,
                    selected_polarizations: [...subPols],
                  });
                  if (!preparedResult.ok) throw new Error(formatBridgeError(preparedResult));
                  const taskId = `nisar-subset-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`;
                  if (typeof estimatedSubsetBytes === "number" && estimatedSubsetBytes > 0) {
                    recordSubsetSizeEstimate({
                      taskId,
                      totalEstimatedBytes: estimatedSubsetBytes,
                    });
                  }
                  await handleConfirmNisarSubsetBatch({
                    source_batch_id: preparedResult.prepared.source_batch_id,
                    expected_workbench_revision: preparedResult.prepared.workbench_revision,
                    items: preparedResult.prepared.intents.map((item) => ({
                      source_item_id: item.source_item_id,
                      prepared_intent: structuredClone(item.intent),
                    })),
                    task_id: taskId,
                    output_root: cleanOut,
                    layout: sarDownloadPolicy?.organize_by_product ? "sar_dataset_processing" : "direct",
                    concurrency: sarDownloadPolicy?.max_concurrent_items || getTransferEnginePrefs().max_concurrent_tasks,
                    created_at: new Date().toISOString(),
                  });
                  setCheckedSceneIds((previous) => {
                    const next = new Set(previous);
                    for (const id of targetSceneIds) next.delete(id);
                    return next;
                  });
                }
              : undefined
          }
          onClose={() => setNisarMultiLevelModalSession(null)}
          onSubmitted={async () => {
            setDownloadDrawerOpen(true);
            await taskOverviewSource.refreshNow();
          }}
        />
      )}

      {/* 存储路径未配置就地提示卡片 (复用 StatusStateCard 样式，一句话 + 一个动作，绝无声默写盘) */}
      {storageRequiredPrompt && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label={translate("mapWorkbench.desktop.storageRequiredTitle")}
          className="fixed inset-0 z-[2500] flex items-center justify-center bg-black/20 p-4 transition-opacity duration-base ease-sp"
          onClick={() => {
            pendingStorageResumeRef.current = null;
            setStorageRequiredPrompt(false);
          }}
        >
          <div
            tabIndex={-1}
            className="relative bg-sp-surface border border-sp-line rounded-modal p-6 max-w-sm w-full shadow-modal outline-none"
            onClick={(e) => e.stopPropagation()}
          >
            <EmptyState
              kind="no-access"
              size="page"
              visual={<EmptyStateWireframe variant="no-access" />}
              title={translate("mapWorkbench.desktop.storageRequiredTitle")}
              description={translate("mapWorkbench.desktop.storageRequiredDescription")}
              primaryAction={{
                label: translate("mapWorkbench.desktop.openStorageSettings"),
                onClick: () => {
                  setStorageRequiredPrompt(false);
                  setSettingsCategory("storage");
                  navigate("settings");
                },
              }}
              secondaryAction={{
                label: translate("mapWorkbench.desktop.chooseDirectoryDirectly"),
                testId: "storage-required-pick-dir",
                onClick: () => {
                  void handlePickStoragePath().then((ok) => {
                    if (ok) {
                      setStorageRequiredPrompt(false);
                    }
                  });
                },
              }}
              className="py-2"
            />
            <button
              type="button"
              autoFocus
              onClick={() => {
                pendingStorageResumeRef.current = null;
                setStorageRequiredPrompt(false);
              }}
              className="absolute top-3 right-3 text-sp-text-muted hover:text-sp-text cursor-pointer"
              aria-label={translate("mapWorkbench.results.close")}
              title={translate("mapWorkbench.desktop.closeEsc")}
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>
      )}

      <TopCenterToastHost />
      <ProgressNotificationHost
        tasks={c1Tasks}
        pendingPreparations={pendingDownloadPreparations}
        suppressDownloads={downloadDrawerOpen || effectivePageRoute === "downloads" || inboxDrawerOpen}
        suppressAll={inboxDrawerOpen}
        readTaskIds={readNotificationTaskIds}
        onDismissTaskNotification={handleMarkTaskNotificationsRead}
        onDismissPreparation={(id) =>
          setPendingDownloadPreparations((prev) => prev.filter((item) => item.id !== id))
        }
        onOpenTask={(taskId) => {
          setDrawerFocusTask((prev) => ({ taskId, nonce: (prev?.nonce ?? 0) + 1 }));
          setDownloadDrawerOpen(true);
        }}
        onCancelTask={(task) => {
          if (!isFixtureBackend()) {
            void taskActionRunner
              .run(task.task_id, "cancel", task.revision)
              .then(() => taskOverviewSource.refreshNow())
              .catch(() => {});
          }
        }}
        onOpenDownloads={() => setDownloadDrawerOpen(true)}
      />

      {/* 3. 抽屉式任务与下载中心 */}
      <DesktopDownloadDrawer
        open={downloadDrawerOpen}
        onClose={() => setDownloadDrawerOpen(false)}
        tasks={c1Tasks}
        onOpenFolder={handleOpenTaskOutput}
        focusTaskRequest={drawerFocusTask}
        onHistoricalMutationApplied={() => taskOverviewSource.refreshNow()}
        newTaskPolicy={sarDownloadPolicy ? { concurrency: sarDownloadPolicy.max_concurrent_items,
          layout: sarDownloadPolicy.organize_by_product ? "sar_dataset_processing" : "direct" } : null}
        pendingPreparations={pendingDownloadPreparations}
        onDismissPreparation={(id) => setPendingDownloadPreparations((prev) => prev.filter((item) => item.id !== id))}
        onConfigureAccount={(accountId) => {
          dispatchPendingAccountCardFocus(accountId);
          setDownloadDrawerOpen(false);
          setSettingsCategory("auth");
          navigate("settings");
        }}
        onOpenFullPage={() => {
          setDownloadDrawerOpen(false);
          navigate("downloads");
        }}
      />

      {/* 4. 首次启动欢迎引导页（真实多源遥感影像方格 + 真实应用图标 + 语言自动识别与缓存目录设置） */}
      {!onboardingResolved && isTauriDesktopHost() && (
        <div
          data-testid="desktop-onboarding-hydrating"
          className="fixed inset-0 z-50 bg-sp-surface"
          aria-hidden="true"
        />
      )}
      <DesktopWelcomeOnboarding
        open={welcomeOnboardingOpen}
        onClose={() => {
          setWelcomeOnboardingOpen(false);
          try {
            if (typeof window !== "undefined") {
              window.localStorage.setItem(WELCOME_ONBOARDING_STORAGE_KEY, "1");
            }
          } catch {
            // ignore
          }
          void setDesktopOnboardingCompleted(true).catch(() => {});
          if (typeof window !== "undefined") {
            const search = window.location.search || "";
            const hash = window.location.hash || "";
            const suppressPrompt =
              search.includes("no_onboarding=1") ||
              hash.includes("harness") ||
              Boolean(window.navigator?.userAgent?.includes("jsdom"));
            if (!suppressPrompt) {
              try {
                if (window.localStorage.getItem(GUIDED_TOUR_STORAGE_KEY) !== "1") {
                  setGuidedTourPromptOpen(true);
                }
              } catch {
                setGuidedTourPromptOpen(true);
              }
            }
          }
        }}
        cacheDir={welcomeCacheDir}
        defaultCacheDir={welcomeDefaultCacheDir}
        installRootDir={welcomeInstallRoot}
        onPickCacheDir={handlePickWelcomeCacheDir}
        onResetCacheDir={handleResetWelcomeCacheDir}
        onOpenSettingsCategory={(cat) => {
          setSettingsCategory(cat);
          navigate("settings");
        }}
      />

      {/* 5. 主界面功能新手引导提示条与 6 步高亮交互导览 */}
      <GuidedTourPromptBanner
        open={guidedTourPromptOpen && !guidedTourOpen && !welcomeOnboardingOpen}
        onStartTour={startGuidedTour}
        onDismiss={() => {
          setGuidedTourPromptOpen(false);
          try {
            if (typeof window !== "undefined") {
              window.localStorage.setItem(GUIDED_TOUR_STORAGE_KEY, "1");
            }
          } catch {
            // ignore
          }
        }}
      />
      <DesktopGuidedTour
        open={guidedTourOpen}
        catalogSummary={guidedTourCatalogSummary}
        onStepEnter={handleGuidedTourStepEnter}
        onFinish={finishGuidedTour}
      />
    </div>
  );
}

export default DesktopWorkbench;
