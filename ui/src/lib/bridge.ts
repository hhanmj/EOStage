// Thin wrapper around the named application boundary.
//
// The formal desktop app owns visible GIS capabilities through typed
// ApplicationService routes. Browser development remains a fixture boundary;
// it never becomes a second business runtime. The compatibility Worker is an
// internal heavy-codec engine only.

import { ADMIN_BOUNDARY_INDEX, type AdminBoundaryLite } from "@/data/adminBoundaries";
import asfCapabilitiesFixture from "../../../contracts/v3/asf-capabilities.v1.json" with { type: "json" };
import resourceRegistryFixture from "../../../contracts/v3/resource-registry.v1.json" with { type: "json" };
import { formatBridgeErrorValue } from "./bridgeErrorFormat.ts";
import { getTransferEnginePrefs } from "./transferEnginePrefs.ts";
import {
  getDesktopShellBridge,
  type DesktopAppendDownloadTaskRequest,
  type DesktopAdminBoundaryOptionsRequest,
  type DesktopAdminBoundaryResolveRequest,
  type DesktopAdminBoundarySearchRequest,
  type DesktopArchiveDownloadTaskRequest,
  type DesktopCatalogIntentSearchAndFreezeRequest,
  type DesktopCatalogSearchAndFreezeRequest,
  type DesktopCdseLoginRequest,
  type DesktopCdseNetworkMode,
  type DesktopCdseQuicklookRequest,
  type DesktopSentinel2MapPreviewRequest,
  type DesktopPreviewAssetRequest,
  type DesktopCreateDownloadTaskRequest,
  type DesktopCreateTaskFromWorkbenchRequest,
  type DesktopAppendTaskFromWorkbenchRequest,
  type DesktopAppendTaskFromSourceBatchRequest,
  type DesktopBurstAppendAssetTaskRequest,
  type DesktopBurstCreateAssetTaskRequest,
  type DesktopBurstCreateHistoricalAssetTaskRequest,
  type DesktopCreateTaskFromHistoricalSourceBatchRequest,
  type DesktopTaskSourceBatchPageRequest,
  type DesktopCreateLandsatBundleTaskRequest,
  type DesktopCreateLandsatAssetTaskRequest,
  type DesktopCreateReviewedMpcAssetTaskRequest,
  type DesktopCreateSentinel2ArchiveBatchTaskRequest,
  type DesktopSentinel2ArchiveAdmissionSummary,
  type DesktopCreateSentinel2ArchiveTaskRequest,
  type DesktopCreateSentinel2BandTaskRequest,
  type DesktopOpticalDerivedCreateTaskRequest,
  type DesktopOpticalDerivedPrecheckBatchRequest,
  type DesktopOpticalPrecheckReport,
  type DesktopOpticalPrecheckResult,
  type DesktopCreateNisarRemoteSubsetTaskRequest,
  type DesktopNisarPrepareSubsetIntentV2Request,
  type DesktopNisarPreparedSubsetIntentV2Result,
  type DesktopNisarPreflightSubsetBatchV2Request,
  type DesktopNisarPreflightSubsetBatchV2Result,
  type DesktopNisarPrepareSubsetBatchV2Request,
  type DesktopNisarPreparedSubsetBatchV2Result,
  type DesktopNisarConfirmSubsetTaskV2Request,
  type DesktopNisarConfirmSubsetBatchTaskV2Request,
  type DesktopNisarAppendSubsetBatchTaskV2Request,
  type DesktopNisarAppendContextV2Request,
  type DesktopNisarAppendPreflightV2Request,
  type DesktopNisarSuccessorTaskV2Request,
  type DesktopNisarArchivedRecoveryV2Request,
  type DesktopSourceMutationCommitStatus,
  type DesktopSourceMutationTargetStatus,
  type DesktopSourceMutationStatus,
  type DesktopSourceMutationStatusResult,
  type DesktopSourceMutationQuery,
  type DesktopDemConversionRequest,
  type DesktopDemConversionSourceRequest,
  type DesktopDemTaskSettings,
  type DesktopDemTaskPlan,
  type DesktopDemTaskPlanRequest,
  type DesktopDemTaskRequest,
  type DesktopStandaloneDemRequest,
  type DesktopStandaloneDemTaskRequest,
  type DesktopOrbitDirectoryMatchRequest,
  type DesktopSceneImportRequest,
  type DesktopGacosConfirmRequest,
  type DesktopGacosCreateImportTaskRequest,
  type DesktopGacosCreateResultTaskRequest,
  type DesktopGacosSupplyResultLinkRequest,
  type DesktopGacosSupplyResultLinkResult,
  type DesktopGacosCreateImportTaskResult,
  type DesktopGacosRequestGroup,
  type DesktopGacosTaskStatus,
  type DesktopGacosResultImportRequest,
  type DesktopGacosSubmissionResult,
  type DesktopNetworkSettings,
  type DesktopProviderConnectivityResult,
  type DesktopDeleteDownloadHistoryRequest,
  type DesktopCoreRestoreSnapshot,
  type DesktopTaskPageRequest,
  type DesktopTaskEventPageRequest,
  type DesktopTaskHistoryPageRequest,
  type DesktopOpenTaskOutputRequest,
  type DesktopListTaskConflictFilesRequest,
  type DesktopDeleteTaskConflictFilesRequest,
  type DesktopTaskConflictFileListResult,
  type DesktopTaskConflictFileDeleteResult,
  type DesktopWorkbenchPageRequest,
  type DesktopWorkbenchSelectionCommand,
  type DesktopDownloadLayout,
  type DesktopBhoonidhiAssetSizeRequest,
  type DesktopLandsatAssetDetailsRequest,
  type DesktopLandsatBrowseRequest,
  type DesktopOrbitTaskRequest,
  type DesktopStandaloneOrbitTaskRequest,
  type DesktopSarSearchAndFreezeRequest,
  type DesktopSarBackgroundCountRequest,
  type DesktopSarBackgroundCountResult,
  type DesktopSarSearchFailureDiagnostic,
  type DesktopSarDownloadPolicySettings,
  type DesktopSetWorkingAoiRequest,
  type DesktopWorkingAoi,
  type DesktopUpdateDownloadTaskRequest,
  type DesktopUpdateDownloadTaskViewRequest,
  type DesktopUpdateSearchWorkbenchRequest,
  type DesktopUsgsM2mLoginRequest,
  type DesktopWorkspaceContextResult,
} from "./desktopShellBridge.ts";
import {
  projectFrozenItemToScene,
  type ApplicationWorkbenchProjection,
} from "./applicationServiceProjection.ts";
import {
  MAX_UI_WORKBENCH_PAGE_SIZE,
  MAX_UI_TASK_PAGE_SIZE,
  projectCoreRestore,
  parseTaskPage, parseCoreTaskSummary,
  projectWorkbenchPage,
  type CoreTaskSummaryProjection,
  type CoreAoiDisplayAuthorityProjection,
  type CoreRestoreProjection,
  type CoreWorkbenchSourceAttemptProjection,
  type CoreWorkbenchSummaryProjection,
  type WorkbenchPageProjection,
  type DisplayBboxProjection,
  type TaskPageProjection,
} from "./coreRestoreProjection.ts";
import { projectDownloadStatusFromCoreTask } from "./downloadStatusProjection.ts";
import { sarTaskDisplayName, sarTaskTypeLabel } from "./sarTaskDisplayName.ts";
import { resolvedDemProvider, formalDemIdentity, type FormalDemIdentity } from "./demSources.ts";
import { canonicalDemIdentityTimestamp } from "./demIdentityTimestamp.ts";
import { compactSearchWorkbenchRows } from "./searchWorkbenchPersistence.ts";
import { NISAR_RSLC_LOGICAL_DATASET } from "./searchBatchDataset.ts";
import { completeSarSearchQuery } from "./sarSearchForm";
import { projectSarProductToScene, sceneProductKey } from "./sarProduct.ts";
import { markUiSmoke } from "./uiSmoke.ts";
import { APPROVED_LEGACY_BRIDGE_METHODS } from "./productionOperationRegistry.ts";
import { approvedExternalUrl } from "./externalNavigation.ts";
import type {
  ContractError,
  DatasetCapabilities,
  ProviderExecutionMode,
  ProviderAsset,
  ProviderMetadata,
  ProviderPlan,
  ProviderPlanItem,
  ProviderProduct,
  ProviderCapabilitiesResponse,
  FrozenOrbitAssociationV1,
  OrbitFileMetadata,
  OrbitPlatform,
  SarSourceCapability,
  SarProduct,
  SarRemoteSearchResponse,
  SarSearchQuery,
  TaskEvent,
} from "./generated/v3-contracts";

type AdminBoundaryIndex = {
  provinces: readonly AdminBoundaryLite[];
  citiesByProvince: Record<string, readonly AdminBoundaryLite[]>;
  districtsByCity: Record<string, readonly AdminBoundaryLite[]>;
};

type RegistrySarSearch = Omit<DatasetCapabilities, "dataset" | "display_name"> & {
  provider_route_id: string;
  selector: string;
  projection_kind: "legacy_asf_search" | "typed_catalog_profile";
  projection_order: number;
  logical_dataset: string | null;
  source_filters: Record<string, unknown> | null;
};

type RegistryScienceResource = {
  resource_kind: "science_dataset";
  common: {
    display_name: string;
    lifecycle: string;
    provider_routes: Array<{
      route_id: string;
      provider_id: string;
      native_collection_id: string | null;
      implementation_binding_id: string;
      lifecycle: string;
    }>;
    capabilities: Array<{
      kind: string;
      lifecycle: string;
      provider_route_ids: string[];
    }>;
  };
  science: { sar_search: RegistrySarSearch | null };
};

type ResourceRegistryFixture = {
  resources: Array<RegistryScienceResource | { resource_kind: string }>;
};

const ADMIN_INDEX = ADMIN_BOUNDARY_INDEX as unknown as AdminBoundaryIndex;
const ADMIN_NATIONAL_VALUE = "全国";
const ADMIN_ALL_VALUE = "全部";
const ADMIN_EMPTY_VALUES = new Set(["", ADMIN_ALL_VALUE, ADMIN_NATIONAL_VALUE, "不限", "中国"]);

export type AppInfo = { name: string; version: string; offline: boolean };
export type UpdateInfo = {
  ok: true;
  checked: boolean;
  update_available: boolean;
  current_version: string;
  latest_version: string;
  html_url: string;
  download_url?: string;
  download_mirrors?: string[];
  asset_name?: string;
  asset_size?: number;
  release_name?: string;
  changelog?: string;
  published_at?: string;
  online_update_supported?: boolean;
  install_mode?: "manual" | "download" | "installer" | string;
  message?: string;
};
export type ApiError = {
  ok: false;
  error: string;
  code: string | null;
  context?: string;
  diagnostic?: DesktopSarSearchFailureDiagnostic;
};
export type Json = Record<string, unknown>;
export type AppUpdateDownloadOk = {
  ok: true;
  path: string;
  folder: string;
  size: number;
  can_run: boolean;
  launched?: boolean;
  install_mode?: "download" | "installer" | string;
  message?: string;
};
export type ComponentSummary = {
  id: string;
  name: string;
  version: string;
  size_mb?: number | null;
  url?: string;
  sha256?: string;
  entry?: string;
  description?: string;
  installed: boolean;
  installed_version?: string;
  installed_path?: string;
  runtime_available: boolean;
  partial_runtime_available?: boolean;
  egm2008_grid_available?: boolean;
  state: "installed" | "bundled" | "available" | "not_configured" | string;
  can_install: boolean;
};
export type ComponentStatusOk = {
  ok: true;
  root: string;
  manifest_url: string;
  manifest_checked_at?: number | null;
  components: ComponentSummary[];
};

export type ProjectOk = {
  ok: true;
  project_id: string;
  name: string;
  safe_name: string;
};
export type RegionOk = {
  ok: true;
  region_id: string;
  name: string;
  safe_name: string;
};
export type WorkspaceOk = {
  ok: true;
  workspace_id: string;
  root: string;
  projects: ProjectOk[];
};

export type Bbox = {
  west: number;
  east: number;
  south: number;
  north: number;
  crs?: string;
};

export type ContextRegion = {
  region_id: string;
  name: string;
  safe_name: string;
  root?: string;
  has_aoi: boolean;
  bbox: Bbox | null;
  aoi_geojson?: Json | null;
  aoi_source?: string | null;
  aoi_name?: string | null;
  scene_footprint_bbox?: Bbox | null;
  scene_count: number;
  /** Native-derived display authority; absent for legacy/browser projections. */
  aoi_display?: CoreAoiDisplayAuthorityProjection | null;
};
export type Context = {
  ok: true;
  workspace: { workspace_id: string; root: string; name: string } | null;
  project: { project_id: string; name: string; safe_name: string; root?: string } | null;
  region: ContextRegion | null;
  dem_dataset?: string;
  dem_provider?: string;
} | {
  ok: false;
  error: string;
  code: string;
  workspace?: null;
  project?: null;
  region?: null;
  dem_dataset?: string;
  dem_provider?: string;
};

export type ConversionAuto = {
  requires_conversion: boolean;
  requires_geoid: boolean;
  source: string;
  target: string;
  geoid_model: string | null;
  message: string;
};

export type SceneRow = {
  /** Canonical identity on migrated SAR rows; absent on legacy snapshots. */
  product_key?: string;
  product_id?: string;
  provider?: string;
  dataset?: string;
  instrument?: string | null;
  processing_level?: string | null;
  product_family?: string;
  file_name?: string | null;
  native_metadata?: Json;
  scene_id: string;
  platform: string;
  product_type: string;
  beam_mode: string;
  polarization: string;
  acquisition_datetime: string;
  absolute_orbit: number | null;
  relative_orbit: number | null;
  path: number | null;
  frame: number | null;
  orbit_direction: string;
  file_size_remote?: number | null;
  footprint_bbox?: Bbox | null;
  footprint_geojson?: Json | null;
  /** Native render-only copy; canonical footprint_geojson is unchanged. */
  render_geometry?: Json | null;
  render_bbox?: DisplayBboxProjection | null;
  render_authority?: { region_id: string; revision: number } | null;
  has_url: boolean;
  download_url?: string;
  /** Complete canonical record retained by the unified SAR workbench. */
  sar_product?: SarProduct;
  /** Frozen delivery item IDs projected under this product in a task view. */
  task_item_ids?: string[];
  /** Frozen task assets; product identity remains the row key. */
  assets?: ProviderAsset[];
};

export type SarSearchParams = {
  dataset: string;
  platforms?: string[];
  instrument?: string | null;
  processing_levels?: string[];
  product_families?: string[];
  beam_modes?: string[];
  polarizations?: string[];
  flight_direction?: string | null;
  look_direction?: string | null;
  relative_orbit?: number | null;
  frame?: number | null;
  asf_frame?: string | null;
  path?: number | null;
  start?: string | null;
  end?: string | null;
  track_range?: SarSearchQuery["track_range"];
  frame_range?: SarSearchQuery["frame_range"];
  bbox?: Bbox | null;
  aoi_geojson?: Json | null;
  max_results?: number | null;
  use_current_aoi?: boolean;
};

export type SarCapabilitiesResult =
  | { ok: true; capabilities: ProviderCapabilitiesResponse }
  | ApiError;
export type SarSearchResult =
  | ({
      ok: true;
      executed_query: SarSearchQuery;
      /** Present only after the M1 service has frozen and persisted the batch. */
      m1_workbench?: {
        batch_context: DownloadBatchContext;
        scenes: SceneRow[];
        checked_item_ids: string[];
      };
    } & SarRemoteSearchResponse)
  | ApiError;
export type SarProductRow = SarProduct;
export type SarDatasetCapabilities = DatasetCapabilities;

export type DownloadBatchContext = {
  batch_id: string;
  source_batch_id?: string;
  task_batch_id?: string;
  created_at: string;
  aoi_name: string;
  /** CoreRestore intentionally does not restore AOI geometry; null is explicit. */
  aoi_geojson: Json | null;
  query: Json;
  page_index: number;
  page_size: number;
  /** Store revision supplied by the M1 authoritative projection. */
  revision?: number;
  /** Store-backed checked item IDs used only for M1 revision reconciliation. */
  checked_item_ids?: string[];
  /** Store-owned totals; only the current page is transported to the UI. */
  total_count?: number;
  checked_count?: number;
  candidate_count?: number;
  provider_matched_count?: number | null;
  provider_returned_count?: number | null;
  frozen_candidate_count?: number | null;
  current_page_item_count?: number | null;
  search_truncated?: boolean;
  search_source?: "ASF" | "ASF_BATCH" | "ASF_CMR" | "TYPED_CATALOG" | null;
  source_attempts?: CoreWorkbenchSourceAttemptProjection[];
  deferred?: boolean;
};

export type V3DownloadRestore =
  | {
      ok: true;
      enabled: boolean;
      workbench: {
        batch_context: DownloadBatchContext;
        scenes: SceneRow[];
        checked_item_ids: string[];
        revision: number;
      } | null;
    }
  | ApiError;

export type DownloadTaskWorkbench = {
  task_id: string;
  provider_id?: string;
  batch_context: DownloadBatchContext & {
    source_batch_id?: string;
    task_batch_id?: string;
  };
  scenes: SceneRow[];
  checked_scene_ids: string[];
  selected_scene_ids: string[];
  output_dir: string;
  download_layout?: string;
  use_product_subdirs?: boolean;
};

export type TaskWorkbenchResult =
  | { ok: true; workbench: DownloadTaskWorkbench }
  | ApiError;

export type TreeRegion = {
  region_id: string;
  name: string;
  safe_name: string;
  root?: string;
  has_aoi: boolean;
  scene_count: number;
};
export type TreeProject = {
  project_id: string;
  name: string;
  safe_name: string;
  root?: string;
  regions: TreeRegion[];
};
export type TreeOk = {
  ok: true;
  workspace: { workspace_id: string; root: string; name: string } | null;
  projects: TreeProject[];
  current_project_id: string | null;
  current_region_id: string | null;
};
export type Tree = TreeOk | ApiError;
export type PickResult = { ok: boolean; path: string };
export type ActivityEntry = { ts: string; text: string; kind: string };
export type ActivityFeed = { ok: true; activities: ActivityEntry[] };
export type DownloadArchiveItem = {
  id: string;
  name: string;
  status: string;
  detail: string;
  ts: number;
  kind?: "asf" | "orbit" | "dem" | "gacos" | string;
  approved_worker_operation?: string | null;
  dataset_id?: string | null;
  task_type?: string | null;
  task_type_identity?: string | null;
  failure_code?: string | null;
  output_dir?: string;
  total?: number;
  concurrency?: number | null;
  created_at?: string;
  ended_at?: string | null;
  use_product_subdirs?: boolean;
  use_orbit_subdir?: boolean;
  download_layout?: string;
  logs?: string[];
  task_id?: string;
  revision?: number;
  archived_at?: string;
  task_snapshot?: DemTaskSnapshot;
  snapshot?: SceneRow[];
  scene_ids?: string[];
  aoi_name?: string;
  batch_context?: DownloadBatchContext;
  elapsed_seconds?: number;
  transferred_bytes?: number;
  total_bytes?: number | null;
  source_total_bytes?: number | null;
  bytes_per_second?: number;
  completed_count?: number;
  active_count?: number;
  queued_count?: number;
  paused_count?: number;
  failed_count?: number;
  skipped_count?: number;
  cancelled_count?: number;
  log_paths?: string[];
};
export type TaskHistoryCursor = { archived_at_epoch_ms: number; task_id: string };
export type DownloadArchiveResult = { ok: true; items: DownloadArchiveItem[]; has_more?: boolean; next_cursor?: TaskHistoryCursor | null };
export type DownloadStatusOk = {
  ok: boolean;
  task_id?: string;
  revision?: number;
  name?: string;
  approved_worker_operation?: string | null;
  dataset_id?: string | null;
  task_type?: string | null;
  failure_code?: string | null;
  state: string;
  total: number;
  admission_requested_count?: number | null;
  admission_rejected_count?: number | null;
  done: number;
  concurrency?: number | null;
  created_at?: string;
  ended_at?: string | null;
  current_scene: string;
  active_downloads?: {
    scene_id: string;
    bytes: number;
    expected_size?: number | null;
    started_at?: number;
    updated_at?: number;
  }[];
  total_bytes?: number | null;
  source_total_bytes?: number | null;
  done_bytes?: number;
  current_bytes?: number;
  transferred_bytes?: number;
  network_bytes_received?: number | null;
  reusable_verified_bytes?: number | null;
  reused_verified_bytes?: number | null;
  /** Active items by what their current progress row reports (Store facts). */
  active_progress?: { without_progress: number; transferring: number; idle: number; waiting?: number } | null;
  /** Distinct stable failure codes of failed items; codes only, never reasons. */
  failure_codes?: { code: string; local: boolean; tls_certificate?: boolean; network?: boolean }[] | null;
  /** Downloader D3-2: the local pre-scan and the reuse it decided (Store facts). */
  local_scan?: {
    state: "none" | "pending" | "scanning" | "done" | "stopped";
    scanned_items: number;
    total_items: number;
    reused_count: number;
    unverified_count: number;
  } | null;
  current_expected_size?: number | null;
  bytes_per_second?: number;
  elapsed_seconds?: number;
  paused: boolean;
  cancelled: boolean;
  error: string | null;
  summary_line: string;
  results_path: string;
  output_dir?: string;
  use_product_subdirs?: boolean;
  download_layout?: string;
  succeeded?: number;
  skipped?: number;
  failed?: number;
  interrupted?: number;
  has_failures?: boolean;
  paused_scene_ids?: string[];
  selected_scene_ids?: string[];
  checked_scene_ids?: string[];
  queued_scene_ids?: string[];
  snapshot_scenes?: SceneRow[];
  aoi_name?: string;
  batch_context?: DownloadBatchContext;
  task_paused?: boolean;
  resume_supported?: boolean;
  resume_hint?: string;
  retry_supported?: boolean;
  retry_hint?: string;
  active_task_count?: number;
  asf_tasks?: DownloadStatus[];
  task_count?: number;
  task_errors?: { task_id?: string; error: string; code: string | null }[];
  restore_error?: { code: string | null; error: string };
  log: { scene_id: string; outcome: string; bytes_written: number; message?: string; detail: string; ts?: number }[];
};
export type DownloadStatusError = DownloadStatusOk;
export type DownloadStatus = DownloadStatusOk;
export type WorkflowStage = {
  id: string;
  label: string;
  nav: string;
  status: "done" | "active" | "blocked" | "running" | string;
  summary: string;
  ready: boolean;
  count: number;
};
export type WorkflowSource = {
  id: string;
  label: string;
  credential: string | null | undefined;
  status: "configured" | "needs_config" | "ready" | "waiting" | string;
  capabilities: string[];
};
export type WorkflowStatus = {
  ok: true;
  context: Context;
  stages: WorkflowStage[];
  sources: WorkflowSource[];
  download: DownloadStatus;
  next_action: { label: string; nav: string };
  counts: { projects: number; regions: number; scenes: number };
  scene_coverage?: { bbox: Bbox | null; count: number };
  dem_dataset: string;
};

export type AoiOk = {
  ok: true;
  aoi: Json;
  aoi_geojson?: Json | null;
  aoi_feature_count?: number | null;
  aoi_total_feature_count?: number | null;
  download_mode?: "merge" | "split" | string;
  region_id: string;
  region_name: string;
};
export type AoiFeaturePreview = {
  id: string;
  index: number;
  source_index?: number;
  name: string;
  area_km2?: number | null;
  bbox?: Bbox | null;
  properties: Json;
};
export type AoiPreviewOk = {
  ok: true;
  path: string;
  file_name: string;
  source_kind?: "path" | "content" | string;
  geojson?: Json | null;
  total_features: number;
  feature_preview_limit?: number;
  features_truncated?: boolean;
  fields: string[];
  display_field?: string;
  features: AoiFeaturePreview[];
};
export type AoiPreviewResult = AoiPreviewOk | ApiError;
export type AdminBoundary = {
  label: string;
  bbox: Bbox;
  name?: string;
  boundary_id?: string;
  provider?: string;
  provider_object_id?: string;
  display_name?: string;
  country_code?: string | null;
  admin_level?: string | null;
  geometry?: Json | null;
  geojson?: Json | null;
  geometry_kind?: string;
  source?: string;
  attribution?: string | null;
  cache_status?: string;
  class?: string | null;
  type?: string | null;
  osm_type?: string | null;
  osm_id?: string | number | null;
};
export type AdminBoundaryOk = {
  ok: true;
  query: string;
  results: AdminBoundary[];
  provider?: string;
  warning?: string | null;
};
export type AdminOption = {
  name: string;
  provider?: string | null;
  provider_object_id: string | null;
};
export type AdminOptionsOk = {
  ok: true;
  provider?: string;
  warning?: string | null;
  provinces: AdminOption[];
  cities: AdminOption[];
  districts: AdminOption[];
};
export type ScenesOk = {
  ok: true;
  scenes: SceneRow[];
  duplicates: string[];
  errors: { line: string; error: string }[];
  queried?: number;
  search?: {
    requested_limit?: number | null;
    query_limit?: number | null;
    total_count?: number | null;
    returned_count?: number | null;
    source?: string | null;
  };
  cache?: {
    hit?: boolean;
    key?: string;
    count?: number;
    path?: string;
  };
};
export type CheckOk = { ok: true; report: Json };
export type PlanOk = { ok: true; plan: Json };
export type PlanReportOk = { ok: true; plan: Json; report: Json; source_parse?: Json; offline?: boolean };
export type GacosPlanOptions = {
  source_batch_id?: string;
  aoi_source?: "frozen_import";
  aoi_geojson?: Json;
  aoi_name?: string;
  manual_dates?: string[];
  hour?: number;
  minute?: number;
  source_kind?: "online_selected" | "local" | "mixed" | string;
  selected_scene_ids?: string[];
  snapshot_scenes?: SceneRow[];
  local_sources?: Array<string | Json>;
  bbox?: Bbox | null;
  buffer_degrees?: number;
  max_dates_per_batch?: number;
  output_format?: "geotiff" | "binary" | string;
  user_supplied_times?: Record<string, [number, number] | { hour: number; minute: number }>;
};
export type GacosRequestSubmitResult = {
  ok: true;
  task_id: string;
  records: Json[];
  request_groups: DesktopGacosRequestGroup[];
  task_status: DesktopGacosTaskStatus;
  submitted_count: number;
  application_completed: boolean;
  manual_action_required: boolean;
  automatic_retry_allowed: false;
  message: string;
} | ApiError;
export type ConvertOk = {
  ok: true;
  dataset: string;
  auto: ConversionAuto;
  plan: Json;
  report: Json;
};
export type RunSummaryOk = {
  ok: true;
  summary_line: string;
  total: number;
  succeeded?: number;
  copied?: number;
  skipped?: number;
  failed?: number;
  interrupted?: number;
  has_failures: boolean;
  results_path: string;
  output_dir?: string;
  results: Json[];
  download?: Json;
  conversion?: Json | null;
  conversion_results_path?: string;
  raw_dem_path?: string;
  ellipsoid_dem_path?: string;
  sarscape_ready_dem_path?: string;
  logs?: string[];
  task_id?: string;
};
export type OrbitDownloadOk = {
  ok: true;
  orbit_dir: string;
  summary_line: string;
  total: number;
  succeeded: number;
  skipped: number;
  unavailable: number;
  failed: number;
  has_failures: boolean;
  results: Json[];
  report: Json;
};
export type OrbitDownloadStatus = {
  ok: true;
  task_id?: string;
  created_at?: string;
  ended_at?: string | null;
  state: string;
  total: number;
  done: number;
  concurrency?: number;
  current_scene: string;
  active_scenes?: { scene_id: string; started_at?: number }[];
  snapshot_scenes?: SceneRow[];
  selected_scene_ids?: string[];
  checked_scene_ids?: string[];
  paused_scene_ids?: string[];
  queued_scene_ids?: string[];
  aoi_name?: string;
  batch_context?: DownloadBatchContext;
  output_dir?: string;
  orbit_dir: string;
  use_orbit_subdir?: boolean;
  download_layout?: string;
  done_bytes?: number;
  bytes_per_second?: number;
  elapsed_seconds?: number;
  paused: boolean;
  cancelled: boolean;
  error: string | null;
  summary_line: string;
  succeeded: number;
  skipped: number;
  unavailable: number;
  failed: number;
  has_failures: boolean;
  results: Json[];
  log: { scene_id: string; outcome: string; detail: string; ts?: number }[];
  report?: Json | null;
  pause_hint?: string;
};
export type DemTaskSnapshot = {
  task_id: string;
  provider: string;
  dataset: string;
  bbox: Bbox;
  source_vertical_datum: string;
  target_vertical_datum: string;
  output_root: string;
  convert_policy: string;
  source_policy: string;
  proxy_policy: string;
  proxy_url?: string;
  schema_version: number;
  created_at: number;
  resume_supported: boolean;
};
export type DemTaskStatus = {
  ok: true;
  task_id: string;
  revision?: number;
  name: string;
  state: string;
  total: number;
  done: number;
  current_scene: string;
  output_dir: string;
  dataset: string;
  convert: boolean;
  raw_dem_path?: string;
  ellipsoid_dem_path?: string;
  sarscape_ready_dem_path?: string;
  results_path?: string;
  conversion_results_path?: string;
  done_bytes?: number;
  total_bytes?: number;
  bytes_per_second?: number;
  elapsed_seconds?: number;
  cancelled: boolean;
  error: string | null;
  summary_line: string;
  succeeded: number;
  skipped: number;
  failed: number;
  interrupted: number;
  has_failures: boolean;
  worker_alive?: boolean;
  queue_position: number;
  created_sequence: number;
  max_concurrent_tasks: number;
  max_concurrent_conversions: number;
  resume_supported: boolean;
  task_snapshot: DemTaskSnapshot;
  results: Json[];
  log: { scene_id: string; outcome: string; detail: string; ts?: number }[];
};
export type DemManagerStatus = {
  ok: true;
  dem_tasks: DemTaskStatus[];
  active_task_count: number;
  paused_task_count: number;
  queued_task_count: number;
  max_concurrent_tasks: number;
  max_concurrent_conversions: number;
};
export type DemTaskSettings = {
  ok: true;
  max_concurrent_tasks: number;
  max_concurrent_conversions: number;
};
export type DemTaskCommandResult = {
  ok: true;
  task_id: string;
  state: string;
} | ApiError;
export type OrbitMatchOk = {
  ok: true;
  orbit_dir: string;
  orbit_files: number;
  report: Json;
};
export type ReportOk = {
  ok: true;
  report: Json;
  reports_dir: string;
  included: string[];
  paths: Record<string, string>;
};
export type CredentialStatus = {
  ok: true;
  cdse: string;
  usgs: string;
  earthdata: string;
  earthdata_input?: {
    mode: "none" | "token" | "login" | "netrc";
    token: string;
    username: string;
    password: string;
  };
  bhoonidhi: string;
  opentopography: string;
  gacos: string;
};
export type EarthdataAuthCheck = {
  ok: true;
  configured: boolean;
  status: "missing" | "valid" | "expired" | "invalid" | "unknown" | "unavailable";
  message: string;
};
export type NetworkSettings = {
  ok: true;
  proxy_enabled: boolean;
  proxy_url: string;
  cache_enabled: boolean;
  cache_dir: string;
  cache_limit_mb: number;
  tianditu_token: string;
  asf_ssl_verify: boolean;
};
export type NetworkSettingsInput = Omit<NetworkSettings, "ok">;
export type V3StoreMigrationInfo = {
  status: string;
  code?: string | null;
  message?: string | null;
};
export type V3StoreSettings = {
  ok: true;
  active_path: string;
  database_size_bytes: number;
  disk_free_bytes: number;
  target_disk_free_bytes: number | null;
  pending_path: string | null;
  rollback_path: string | null;
  restart_required: boolean;
  last_migration: V3StoreMigrationInfo | null;
  schema_changed: false;
  legacy_network_cache_path: string;
};
export type V3StoreSettingsResult = V3StoreSettings | ApiError;
export type AsfSearchParams = {
  bbox?: Bbox | null;
  aoi_geojson?: Json | null;
  use_current_aoi?: boolean;
  start?: string;
  end?: string;
  product_type?: string;
  beam_mode?: string;
  polarization?: string;
  orbit_direction?: string;
  relative_orbit?: string | number | null;
  relative_orbit_start?: string | number | null;
  relative_orbit_end?: string | number | null;
  frame?: string | number | null;
  frame_start?: string | number | null;
  frame_end?: string | number | null;
  max_results?: string | number | null;
};
export type SimpleOk = { ok: true; [key: string]: unknown } | ApiError;
export type PublicPreviewSaveResult =
  | {
      ok: true;
      path: string;
      file_name: string;
      bytes_written: number;
    }
  | ApiError;
export type NativeWindowSize = {
  ok: true;
  width: number;
  height: number;
  x?: number;
  y?: number;
} | ApiError;
export type NativeWindowWorkArea = {
  x: number;
  y: number;
  width: number;
  height: number;
};
export type MetadataStatus = {
  ok: true;
  state: string;
  done: number;
  total: number;
  percent: number;
  message: string;
  quantitative?: boolean;
};
export type SarDownloadPolicySettings = DesktopSarDownloadPolicySettings & { ok: true };
export type UiFlagsOk = {
  ok: true;
  flags: Record<string, boolean>;
};

export type WorkspaceResult = WorkspaceOk | ApiError;
export type ProjectResult = ProjectOk | ApiError;
export type RegionResult = RegionOk | ApiError;
export type AoiResult = AoiOk | ApiError;
export type WorkingAoiOk = {
  ok: true;
  state: "working";
  source_kind: string;
  geometry: Json;
  bbox: number[];
  crs: string;
  display_name: string;
};
export type WorkingAoiResult = WorkingAoiOk | ApiError;
export type AdminBoundaryResult = AdminBoundaryOk | ApiError;
export type AdminOptionsResult = AdminOptionsOk | ApiError;
export type ScenesResult = ScenesOk | ApiError;
export type CheckResult = CheckOk | ApiError;
export type PlanResult = PlanOk | ApiError;
export type PlanReportResult = PlanReportOk | ApiError;
export type ConvertResult = ConvertOk | ApiError;
export type RunSummaryResult = RunSummaryOk | ApiError;
export type OrbitDownloadResult = OrbitDownloadOk | ApiError;
export type OrbitMatchResult = OrbitMatchOk | ApiError;
export type ReportResult = ReportOk | ApiError;

type DesktopCompatApi = {
  get_app_info: () => Promise<AppInfo>;
  check_for_update?: (force?: boolean) => Promise<UpdateInfo | ApiError>;
  download_app_update?: (
    downloadUrl?: string,
    assetName?: string,
    mirrorUrls?: string[],
    expectedSize?: number,
  ) => Promise<AppUpdateDownloadOk | ApiError>;
  get_component_status?: (refresh?: boolean) => Promise<ComponentStatusOk | ApiError>;
  install_component?: (componentId: string) => Promise<ComponentStatusOk | ApiError>;
  remove_component?: (componentId: string) => Promise<ComponentStatusOk | ApiError>;
  get_context: () => Promise<Context>;
  get_tree: () => Promise<Tree>;
  get_workflow_status: () => Promise<WorkflowStatus>;
  get_activity: (limit?: number) => Promise<ActivityFeed>;
  get_ui_flags?: () => Promise<UiFlagsOk | ApiError>;
  set_ui_flag?: (key: string, value?: boolean) => Promise<UiFlagsOk | ApiError>;
  get_network_settings: () => Promise<NetworkSettings>;
  save_network_settings: (settings: NetworkSettingsInput) => Promise<NetworkSettings | ApiError>;
  get_v3_store_settings?: () => Promise<V3StoreSettingsResult>;
  stage_v3_store_migration?: (targetDirectory: string) => Promise<V3StoreSettingsResult | SimpleOk>;
  pick_open_file: (title?: string, filters?: string[]) => Promise<PickResult>;
  pick_directory: (title?: string) => Promise<PickResult>;
  save_public_preview?: (
    url: string,
    outputDir: string,
    productId: string,
  ) => Promise<PublicPreviewSaveResult>;
  ensure_directory: (path: string) => Promise<SimpleOk>;
  open_external_url: (url: string) => Promise<SimpleOk>;
  open_path: (path: string) => Promise<SimpleOk>;
  window_minimize?: () => Promise<SimpleOk>;
  window_toggle_maximize?: (workArea?: NativeWindowWorkArea) => Promise<SimpleOk>;
  window_close?: () => Promise<SimpleOk>;
  window_get_size?: () => Promise<NativeWindowSize>;
  window_resize_from_edge?: (
    edge: string,
    startWidth: number,
    startHeight: number,
    deltaX: number,
    deltaY: number,
  ) => Promise<SimpleOk>;
  create_workspace: (root: string, name?: string | null) => Promise<WorkspaceResult>;
  add_project: (name: string) => Promise<ProjectResult>;
  select_project: (projectId: string) => Promise<ProjectResult>;
  add_region: (name: string) => Promise<RegionResult>;
  select_region: (regionId: string) => Promise<RegionResult>;
  set_region_aoi_bbox: (
    west: number,
    east: number,
    south: number,
    north: number,
  ) => Promise<AoiResult>;
  set_region_aoi_file: (path: string) => Promise<AoiResult>;
  preview_aoi_file: (path: string) => Promise<AoiPreviewResult>;
  preview_aoi_file_content?: (fileName: string, text: string) => Promise<AoiPreviewResult>;
  preview_aoi_file_bytes?: (fileName: string, base64Data: string) => Promise<AoiPreviewResult>;
  set_region_aoi_file_features: (
    path: string,
    featureIds?: string[],
    nameField?: string,
    downloadMode?: "merge" | "split",
  ) => Promise<AoiResult>;
  set_region_aoi_geojson: (geojson: Json) => Promise<AoiResult>;
  set_region_aoi_geojson_features?: (
    geojson: Json,
    featureIds?: string[],
    nameField?: string,
    downloadMode?: "merge" | "split",
  ) => Promise<AoiResult>;
  search_admin_boundaries: (
    query?: string,
    province?: string,
    city?: string,
    district?: string,
    limit?: number,
  ) => Promise<AdminBoundaryResult>;
  get_admin_options: (province?: string, city?: string) => Promise<AdminOptionsResult>;
  clear_region_aoi?: () => Promise<SimpleOk>;
  import_scenes_text: (text: string) => Promise<ScenesResult>;
  import_scenes_file: (path: string) => Promise<ScenesResult>;
  import_scenes_directory: (path: string) => Promise<ScenesResult>;
  preview_scenes_file?: (path: string) => Promise<ScenesResult>;
  preview_scenes_directory?: (path: string) => Promise<ScenesResult>;
  clear_orbit_candidate_scenes?: () => Promise<SimpleOk>;
  search_asf_scenes: (params: AsfSearchParams) => Promise<ScenesResult>;
  cancel_asf_search?: () => Promise<SimpleOk>;
  list_scenes: () => Promise<ScenesOk | ApiError>;
  clear_scenes: () => Promise<SimpleOk>;
  clear_map_layers?: () => Promise<SimpleOk>;
  get_metadata_status: () => Promise<MetadataStatus>;
  check_scenes: () => Promise<CheckResult>;
  match_orbits_directory: (orbitDir: string) => Promise<OrbitMatchResult>;
  download_orbits: (outputDir?: string, sceneIds?: string[], useOrbitSubdir?: boolean) => Promise<OrbitDownloadResult>;
  start_orbit_download: (
    outputDir?: string,
    sceneIds?: string[],
    maxConcurrent?: number,
    useOrbitSubdir?: boolean,
  ) => Promise<{ ok: boolean; error?: string; code?: string }>;
  start_orbit_download_snapshot?: (
    outputDir?: string,
    scenesSnapshot?: SceneRow[],
    sceneIds?: string[],
    maxConcurrent?: number,
    useOrbitSubdir?: boolean,
    aoiName?: string,
    batchContext?: DownloadBatchContext | null,
  ) => Promise<{ ok: boolean; error?: string; code?: string; task_id?: string }>;
  pause_orbit_download: () => Promise<{ ok: boolean; error?: string; code?: string }>;
  resume_orbit_download: () => Promise<{ ok: boolean; error?: string; code?: string }>;
  stop_orbit_download: () => Promise<{ ok: boolean; error?: string; code?: string }>;
  get_orbit_download_status: () => Promise<OrbitDownloadStatus>;
  plan_asf_download: (outputDir?: string, sceneIds?: string[], useProductSubdirs?: boolean) => Promise<PlanResult>;
  start_asf_download: (
    outputDir?: string,
    credentialSource?: string,
    maxConcurrent?: number,
    sceneIds?: string[],
    useProductSubdirs?: boolean,
  ) => Promise<{ ok: boolean; error?: string; code?: string; task_id?: string }>;
  start_asf_download_snapshot?: (
    outputDir?: string,
    scenesSnapshot?: SceneRow[],
    sceneIds?: string[],
    credentialSource?: string,
    maxConcurrent?: number,
    useProductSubdirs?: boolean,
    aoiName?: string,
    taskId?: string,
    batchContext?: DownloadBatchContext | null,
  ) => Promise<{ ok: boolean; error?: string; code?: string; task_id?: string }>;
  start_sar_download_snapshot?: (
    outputDir?: string,
    productsSnapshot?: SarProduct[],
    productIds?: string[],
    maxConcurrent?: number,
    organizeByDataset?: boolean,
    batchContext?: DownloadBatchContext | null,
    taskId?: string,
  ) => Promise<{ ok: boolean; error?: string; code?: string; task_id?: string }>;
  append_asf_download: (
    outputDir?: string,
    maxExtraWorkers?: number,
    sceneIds?: string[],
    taskId?: string,
  ) => Promise<{ ok: boolean; error?: string; code?: string; appended?: number; skipped?: number; concurrency?: number }>;
  append_asf_download_snapshot?: (
    outputDir?: string,
    scenesSnapshot?: SceneRow[],
    sceneIds?: string[],
    maxExtraWorkers?: number,
    taskId?: string,
  ) => Promise<{ ok: boolean; error?: string; code?: string; appended?: number; skipped?: number; concurrency?: number }>;
  pause_asf_scenes: (sceneIds?: string[], taskId?: string) => Promise<{ ok: boolean; error?: string; code?: string; paused?: number }>;
  resume_asf_scenes: (sceneIds?: string[], taskId?: string) => Promise<{ ok: boolean; error?: string; code?: string; resumed?: number }>;
  pause_asf_download: (taskId?: string) => Promise<{ ok: boolean; error?: string; code?: string }>;
  resume_asf_download: (taskId?: string) => Promise<{ ok: boolean; error?: string; code?: string }>;
  stop_asf_download: (taskId?: string) => Promise<{ ok: boolean; error?: string; code?: string }>;
  retry_asf_download: (taskId?: string) => Promise<{ ok: boolean; error?: string; code?: string }>;
  get_download_status: () => Promise<DownloadStatus>;
  plan_dem_download: (outputDir?: string, dataset?: string) => Promise<PlanReportResult>;
  plan_dem_download_bbox: (
    west: number,
    east: number,
    south: number,
    north: number,
    outputDir?: string,
    dataset?: string,
  ) => Promise<PlanReportResult>;
  plan_gacos_request: (outputDir?: string, options?: GacosPlanOptions) => Promise<PlanReportResult>;
  gacos_request_submit?: (
    plan: Json,
    confirm?: boolean,
    forceReapply?: boolean,
    continueAfterFailure?: boolean,
  ) => Promise<GacosRequestSubmitResult>;
  gacos_request_ledger?: () => Promise<{ ok: true; records: Json[] } | ApiError>;
  plan_dem_conversion: (outputDir?: string) => Promise<ConvertResult>;
  plan_local_dem_conversion: (
    inputPath: string,
    outputDir?: string,
    sourceVerticalDatum?: string,
  ) => Promise<ConvertResult>;
  plan_dem_conversion_bbox: (
    west: number,
    east: number,
    south: number,
    north: number,
    outputDir?: string,
    dataset?: string,
  ) => Promise<ConvertResult>;
  run_dem_download: (
    outputDir?: string,
    dataset?: string,
    keySource?: string,
    convert?: boolean,
  ) => Promise<RunSummaryResult>;
  run_dem_download_bbox: (
    west: number,
    east: number,
    south: number,
    north: number,
    outputDir?: string,
    dataset?: string,
    keySource?: string,
    convert?: boolean,
  ) => Promise<RunSummaryResult>;
  start_dem_download?: (
    outputDir?: string,
    dataset?: string,
    keySource?: string,
    convert?: boolean,
    provider?: string,
  ) => Promise<DemTaskCommandResult>;
  start_dem_download_bbox?: (
    west: number,
    east: number,
    south: number,
    north: number,
    outputDir?: string,
    dataset?: string,
    keySource?: string,
    convert?: boolean,
    provider?: string,
  ) => Promise<DemTaskCommandResult>;
  stop_dem_download?: (taskId: string) => Promise<DemTaskCommandResult>;
  pause_dem_download?: (taskId: string) => Promise<DemTaskCommandResult>;
  resume_dem_download?: (taskId: string) => Promise<DemTaskCommandResult>;
  retry_dem_download?: (taskId: string) => Promise<DemTaskCommandResult>;
  get_dem_download_status?: () => Promise<DemManagerStatus | ApiError>;
  get_dem_task_settings?: () => Promise<DemTaskSettings | ApiError>;
  save_dem_task_settings?: (settings: {
    max_concurrent_tasks: number;
  }) => Promise<DemTaskSettings | ApiError>;
  run_dem_conversion: (outputDir?: string) => Promise<RunSummaryResult>;
  run_local_dem_conversion: (
    inputPath: string,
    outputDir?: string,
    sourceVerticalDatum?: string,
    outputMode?: "ellipsoid" | "sarscape",
  ) => Promise<RunSummaryResult>;
  run_dem_conversion_bbox: (
    west: number,
    east: number,
    south: number,
    north: number,
    outputDir?: string,
    dataset?: string,
  ) => Promise<RunSummaryResult>;
  set_dem_dataset: (dataset: string) => Promise<{ ok: boolean; dataset?: string; error?: string; code?: string }>;
  get_download_archive: () => Promise<DownloadArchiveResult | ApiError>;
  save_download_archive: (items: DownloadArchiveItem[]) => Promise<DownloadArchiveResult | ApiError>;
  delete_download_archive_item: (item: DownloadArchiveItem) => Promise<DownloadArchiveResult | ApiError>;
  get_credential_status: () => Promise<CredentialStatus>;
  check_earthdata_auth: (force?: boolean) => Promise<EarthdataAuthCheck | ApiError>;
  save_earthdata_token: (token: string) => Promise<SimpleOk>;
  save_earthdata_login: (username: string, password: string) => Promise<SimpleOk>;
  clear_earthdata_credentials: () => Promise<SimpleOk>;
  save_bhoonidhi_login: (username: string, password: string) => Promise<SimpleOk>;
  clear_bhoonidhi_credentials: () => Promise<SimpleOk>;
  save_opentopography_key: (apiKey: string) => Promise<SimpleOk>;
  clear_opentopography_key: () => Promise<SimpleOk>;
  save_gacos_email: (email: string) => Promise<SimpleOk>;
  clear_gacos_email: () => Promise<SimpleOk>;
  generate_report: (outputDir?: string) => Promise<ReportResult>;
  get_v3_providers?: () => Promise<V3ProvidersResult>;
  get_sar_provider_capabilities?: () => Promise<SarCapabilitiesResult>;
  search_sar_products?: (
    params: SarSearchParams | SarSearchQuery,
  ) => Promise<SarSearchResult>;
  get_v3_download_restore?: () => Promise<V3DownloadRestore>;
  get_asf_task_workbench?: (taskId: string) => Promise<TaskWorkbenchResult>;
  save_v3_search_workbench?: (
    scenes: Array<SceneRow | SarProductRow>,
    checkedItemIds: string[],
    batchContext: DownloadBatchContext,
  ) => Promise<SimpleOk>;
  save_v3_task_view?: (
    taskId: string,
    checkedItemIds: string[],
    pageIndex: number,
    pageSize: number,
  ) => Promise<SimpleOk>;
  v3_search_provider?: (providerId: string, query?: V3SearchRequest) => Promise<V3SearchResult>;
  v3_plan_provider?: (
    providerId: string,
    query?: V3PlanRequest,
    products?: V3ProviderProduct[],
  ) => Promise<V3PlanResult>;
  gacos_webform_dry_run?: (payload: V3GacosSubmission, email?: string) => Promise<GacosWebFormDryRunResult>;
  gacos_webform_submit?: (
    payload: V3GacosSubmission,
    email?: string,
    confirm?: boolean,
    forceReapply?: boolean,
    continueAfterFailure?: boolean,
  ) => Promise<GacosWebFormSubmitResult>;
  preview_aoi_file_bundle?: (files: { name: string; base64: string }[]) => Promise<AoiPreviewResult>;

};

export function hasBridge(): boolean {
  if (testCompatibilityApi() !== null) return true;
  try {
    return getDesktopShellBridge().kind === "tauri";
  } catch {
    return false;
  }
}

function testCompatibilityApi(): DesktopCompatApi | null {
  if (typeof window === "undefined") return null;
  const host = window as typeof window & {
    __insarDesktopCompatibility?: { api?: DesktopCompatApi };
  };
  // This property is only installed by the test harness. The formal Tauri
  // bundle does not create it, so checking the injected object itself keeps
  // late test injection working without reintroducing a production fallback.
  return host.__insarDesktopCompatibility?.api ?? null;
}

const api = (): DesktopCompatApi => {
  const testApi = testCompatibilityApi();
  if (testApi) return testApi;
  const shell = getDesktopShellBridge();
  const compatibility = shell.compatibility;
  return new Proxy({} as DesktopCompatApi, {
    get: (_target, property: string | symbol) => {
      if (typeof property !== "string") return undefined;
      return (...args: unknown[]) => {
        if (shell.kind === "tauri" && !TAURI_APPROVED_LEGACY_METHODS.has(property)) {
          return tauriLegacyUnavailable(property);
        }
        return compatibility.callLegacy(property, args);
      };
    },
  });
};

const TAURI_APPROVED_LEGACY_METHODS = new Set<string>(APPROVED_LEGACY_BRIDGE_METHODS);

type TauriLegacyError = {
  ok: false;
  error: string;
  code: "COMPAT_WORKER_OPERATION_UNSUPPORTED";
};

function tauriLegacyUnavailable(method: string): TauriLegacyError {
  return {
    ok: false,
    error: `当前单一 Tauri 宿主未迁移旧接口：${method}`,
    code: "COMPAT_WORKER_OPERATION_UNSUPPORTED",
  };
}

/**
 * The single Tauri host is selected before App mounts. A browser fixture never
 * becomes a business runtime.
 */
export function isTauriDesktopHost(): boolean {
  try {
    return getDesktopShellBridge().kind === "tauri";
  } catch {
    return false;
  }
}

function tauriCredentialBridge() {
  try {
    const shell = getDesktopShellBridge();
    return shell.kind === "tauri" ? shell.credentials ?? null : null;
  } catch {
    // Browser fixtures and the legacy pywebview bridge do not register the
    // Tauri shell. Keep their credential path intact instead of turning a
    // normal fixture call into a host-registration exception.
    return null;
  }
}

function applicationError(value: unknown): ApiError {
  const record = recordValue(value);
  return {
    ok: false,
    error:
      typeof record?.error === "string"
        ? record.error
        : "ApplicationService 返回了无效错误",
    code: typeof record?.code === "string" ? record.code : null,
    context: typeof record?.context === "string" ? record.context : undefined,
    diagnostic: record?.diagnostic as DesktopSarSearchFailureDiagnostic | undefined,
  };
}

function recordValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function numberValue(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function normalizeAdminBoundary(value: unknown, providerFallback: string): AdminBoundary | null {
  const record = recordValue(value);
  if (!record) return null;
  const bboxValue = recordValue(record.bbox);
  const bbox = bboxValue
    ? {
        west: numberValue(bboxValue.west) ?? Number.NaN,
        east: numberValue(bboxValue.east) ?? Number.NaN,
        south: numberValue(bboxValue.south) ?? Number.NaN,
        north: numberValue(bboxValue.north) ?? Number.NaN,
        crs: typeof bboxValue.crs === "string" ? bboxValue.crs : "EPSG:4326",
      }
    : null;
  const rawGeometry = record.geometry ?? record.geojson;
  const geometry = rawGeometry == null ? null : recordValue(rawGeometry);
  if (
    !bbox ||
    (rawGeometry != null && !geometry) ||
    !Number.isFinite(bbox.west) ||
    !Number.isFinite(bbox.east) ||
    !Number.isFinite(bbox.south) ||
    !Number.isFinite(bbox.north)
  ) {
    return null;
  }
  const provider = typeof record.provider === "string" ? record.provider : providerFallback;
  const displayName =
    (typeof record.display_name === "string" && record.display_name.trim()) ||
    (typeof record.label === "string" && record.label.trim()) ||
    (typeof record.name === "string" && record.name.trim()) ||
    "";
  if (!displayName) return null;
  const name = (typeof record.name === "string" && record.name.trim()) || displayName;
  const objectId =
    (typeof record.provider_object_id === "string" && record.provider_object_id.trim()) ||
    (typeof record.osm_id === "string" && record.osm_id.trim()) ||
    (typeof record.osm_id === "number" ? String(record.osm_id) : "") ||
    displayName;
  const boundaryId =
    (typeof record.boundary_id === "string" && record.boundary_id.trim()) ||
    `${provider}:${objectId}`;
  const geometryKind =
    typeof record.geometry_kind === "string"
      ? record.geometry_kind
      : geometry && typeof geometry.type === "string"
        ? geometry.type.toLowerCase()
        : undefined;
  return {
    label: displayName,
    bbox,
    name,
    boundary_id: boundaryId,
    provider,
    provider_object_id: objectId,
    display_name: displayName,
    country_code: typeof record.country_code === "string" ? record.country_code : null,
    admin_level: typeof record.admin_level === "string" ? record.admin_level : null,
    geometry: geometry as Json | null,
    geojson: geometry as Json | null,
    geometry_kind: geometryKind,
    source: typeof record.source === "string" ? record.source : provider,
    attribution: typeof record.attribution === "string" ? record.attribution : null,
    cache_status: typeof record.cache_status === "string" ? record.cache_status : "unknown",
    class: typeof record.class === "string" ? record.class : "boundary",
    type: typeof record.type === "string" ? record.type : "administrative",
    osm_type: typeof record.osm_type === "string" ? record.osm_type : null,
    osm_id: typeof record.osm_id === "string" || typeof record.osm_id === "number" ? record.osm_id : null,
  };
}

function normalizeAdminBoundaryResult(value: unknown, providerFallback: string): AdminBoundaryResult {
  const record = recordValue(value);
  if (!record || record.ok !== true || !Array.isArray(record.results)) {
    return {
      ok: false,
      error: String(record?.error ?? "ApplicationService 未返回有效行政区结果"),
      code: typeof record?.code === "string" ? record.code : "ADMIN_BOUNDARY_INVALID",
    };
  }
  const provider = typeof record.provider === "string" ? record.provider : providerFallback;
  const results = record.results.map((item) => normalizeAdminBoundary(item, provider));
  if (results.some((item) => item === null)) {
    return {
      ok: false,
      error: "ApplicationService 返回了无效的行政边界候选",
      code: "BOUNDARY_PROVIDER_RESPONSE_INVALID",
    };
  }
  return {
    ok: true,
    query: typeof record.query === "string" ? record.query : "",
    provider,
    warning: typeof record.warning === "string" ? record.warning : null,
    results: results as AdminBoundary[],
  };
}

type RestoreFields = {
  workspace_context: unknown;
  region_aoi: unknown;
  region_aois: unknown[];
  dem_source_identity?: unknown;
};

function restoreFieldsFromRecord(value: Record<string, unknown>): RestoreFields {
  return {
    workspace_context: value.workspace_context,
    region_aoi: value.region_aoi,
    region_aois: Array.isArray(value.region_aois) ? value.region_aois : [],
  };
}

function stableUiNodeId(prefix: string, value: string, maxLength = 128): string {
  const encodedLength = Math.max(1, maxLength - prefix.length - 1);
  const encoded = Array.from(value.trim())
    .map((character) => /[a-z0-9]/i.test(character) ? character.toLowerCase() : `u${character.codePointAt(0)?.toString(16) ?? "0"}`)
    .join("")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, encodedLength);
  return `${prefix}-${encoded || "default"}`;
}

function safeNodeName(value: string): string {
  return value.trim().replace(/[^a-zA-Z0-9_-]+/g, "_").replace(/^_+|_+$/g, "") || "node";
}

function restoredRegionAoi(
  snapshot: Extract<DesktopCoreRestoreSnapshot, { ok: true }>,
  regionId: string | undefined,
): Record<string, unknown> | null {
  const payload = snapshot.core;
  const candidates = Array.isArray(payload.region_aois)
    ? payload.region_aois.flatMap((value) => {
        const record = recordValue(value);
        return record ? [record] : [];
      })
    : [];
  const match = candidates.find((value) => typeof regionId === "string" && value.region_id === regionId);
  if (match) return match;
  const legacy = recordValue(payload.region_aoi);
  return legacy && (regionId === undefined || legacy.region_id === regionId) ? legacy : null;
}

function contextWorkspaceResult(result: DesktopWorkspaceContextResult): WorkspaceResult {
  if (!result.ok) return applicationError(result);
  const context = result.context;
  const projects = Array.isArray(context.projects)
    ? context.projects.flatMap((value) => {
        const project = recordValue(value);
        return project && typeof project.project_id === "string" && typeof project.project_name === "string"
          ? [{ ok: true as const, project_id: project.project_id, name: project.project_name, safe_name: safeNodeName(project.project_name) }]
          : [];
      })
    : [];
  return {
    ok: true,
    workspace_id: String(context.workspace_id),
    root: String(context.workspace_root),
    projects,
  };
}

function contextProjectResult(result: DesktopWorkspaceContextResult): ProjectResult {
  if (!result.ok) return applicationError(result);
  const context = result.context;
  if (typeof context.project_id !== "string" || typeof context.project_name !== "string") {
    return { ok: false, error: "项目上下文未返回稳定 project_id", code: "V3_APPLICATION_INVALID_SNAPSHOT" };
  }
  return {
    ok: true,
    project_id: context.project_id,
    name: context.project_name,
    safe_name: safeNodeName(context.project_name),
  };
}

function contextRegionResult(result: DesktopWorkspaceContextResult): RegionResult {
  if (!result.ok) return applicationError(result);
  const context = result.context;
  if (typeof context.region_id !== "string" || typeof context.region_name !== "string") {
    return { ok: false, error: "区域上下文未返回稳定 region_id", code: "V3_APPLICATION_INVALID_SNAPSHOT" };
  }
  return {
    ok: true,
    region_id: context.region_id,
    name: context.region_name,
    safe_name: safeNodeName(context.region_name),
  };
}

function contextFromRestoreFields(fields: RestoreFields): Context {
  const persistedIdentity = recordValue(fields.dem_source_identity);
  const restoredIdentity = persistedIdentity &&
      typeof persistedIdentity.dataset === "string" && typeof persistedIdentity.provider === "string"
    ? formalDemIdentity(persistedIdentity.dataset, persistedIdentity.provider)
    : null;
  const demDataset = restoredIdentity && restoredIdentity.dataset === persistedIdentity?.dataset &&
      restoredIdentity.provider === persistedIdentity?.provider
    ? restoredIdentity.dataset
    : "COP30";
  const demProvider = restoredIdentity && restoredIdentity.dataset === persistedIdentity?.dataset &&
      restoredIdentity.provider === persistedIdentity?.provider
    ? restoredIdentity.provider
    : "COPERNICUS_AWS";
  if (fields.dem_source_identity !== null && fields.dem_source_identity !== undefined &&
      (!restoredIdentity || restoredIdentity.dataset !== persistedIdentity?.dataset ||
       restoredIdentity.provider !== persistedIdentity?.provider)) {
    return {
      ok: false,
      error: "Store 中的 DEM dataset/provider 身份不一致",
      code: "V3_APPLICATION_INVALID_SNAPSHOT",
    };
  }
  const context = recordValue(fields.workspace_context);
  if (!context) {
    return { ok: true, workspace: null, project: null, region: null, dem_dataset: demDataset, dem_provider: demProvider };
  }
  const workspace = {
    workspace_id: String(context.workspace_id),
    root: String(context.workspace_root),
    name: String(context.workspace_name),
  };
  const project = typeof context.project_id === "string" && typeof context.project_name === "string"
    ? {
        project_id: context.project_id,
        name: context.project_name,
        safe_name: safeNodeName(context.project_name),
        root: `${workspace.root}\\${safeNodeName(context.project_name)}`,
      }
    : null;
  const candidates = fields.region_aois.flatMap((value) => {
    const candidate = recordValue(value);
    return candidate ? [candidate] : [];
  });
  const aoi = candidates.find((value) => value.region_id === context.region_id)
    ?? (recordValue(fields.region_aoi)?.region_id === context.region_id ? recordValue(fields.region_aoi) : null);
  const region = typeof context.region_id === "string" && typeof context.region_name === "string"
    ? {
        region_id: context.region_id,
        name: context.region_name,
        safe_name: safeNodeName(context.region_name),
        root: `${project?.root ?? workspace.root}\\${safeNodeName(context.region_name)}`,
        has_aoi: aoi?.region_id === context.region_id,
        bbox: Array.isArray(aoi?.bbox) && aoi.bbox.length >= 4
          ? { west: Number(aoi.bbox[0]), south: Number(aoi.bbox[1]), east: Number(aoi.bbox[2]), north: Number(aoi.bbox[3]), crs: "EPSG:4326" }
          : null,
        aoi_geojson: aoi?.geometry as Json | undefined,
        aoi_source: typeof aoi?.source_kind === "string" ? aoi.source_kind : null,
        aoi_name: typeof aoi?.display_name === "string" ? aoi.display_name : null,
        scene_count: 0,
      }
    : null;
  markUiSmoke("aoi_restored_after_restart", region?.has_aoi ?? false);
  return { ok: true, workspace, project, region, dem_dataset: demDataset, dem_provider: demProvider };
}

function contextFromCoreRestore(
  core: CoreRestoreProjection,
  aoiDisplay: CoreAoiDisplayAuthorityProjection | null = null,
): Context {
  const context = contextFromRestoreFields(core);
  if (!context.ok || !context.region || !aoiDisplay || aoiDisplay.region_id !== context.region.region_id) {
    return context;
  }
  return {
    ...context,
    region: { ...context.region, aoi_display: aoiDisplay },
  };
}

function treeFromRestoreFields(fields: RestoreFields): Tree {
  const context = recordValue(fields.workspace_context);
  if (!context) {
    return {
      ok: true,
      workspace: null,
      projects: [],
      current_project_id: null,
      current_region_id: null,
    };
  }
  const workspaceId = typeof context.workspace_id === "string" ? context.workspace_id : null;
  const workspaceRoot = typeof context.workspace_root === "string" ? context.workspace_root : null;
  const workspaceName = typeof context.workspace_name === "string" ? context.workspace_name : null;
  if (!workspaceId || !workspaceRoot || !workspaceName) return applicationProjectionError();

  const aoiByRegion = new Map<string, Record<string, unknown>>();
  if (Array.isArray(fields.region_aois)) {
    for (const value of fields.region_aois) {
      const aoi = recordValue(value);
      if (typeof aoi?.region_id === "string") aoiByRegion.set(aoi.region_id, aoi);
    }
  }
  const legacyAoi = recordValue(fields.region_aoi);
  if (typeof legacyAoi?.region_id === "string" && !aoiByRegion.has(legacyAoi.region_id)) {
    aoiByRegion.set(legacyAoi.region_id, legacyAoi);
  }
  const projects = Array.isArray(context.projects)
    ? context.projects.flatMap((value) => {
        const project = recordValue(value);
        if (typeof project?.project_id !== "string" || typeof project.project_name !== "string") return [];
        const nestedRegions = Array.isArray(project.regions)
          ? project.regions.flatMap((item) => {
              const region = recordValue(item);
              if (
                typeof region?.project_id !== "string"
                || region.project_id !== project.project_id
                || typeof region.region_id !== "string"
                || typeof region.region_name !== "string"
              ) return [];
              return [{
                region_id: region.region_id,
                name: region.region_name,
                safe_name: typeof region.safe_name === "string"
                  ? region.safe_name
                  : safeNodeName(region.region_name),
                root: `${workspaceRoot}\\${typeof project.safe_name === "string" ? project.safe_name : safeNodeName(String(project.project_name))}\\${typeof region.safe_name === "string" ? region.safe_name : safeNodeName(String(region.region_name))}`,
                has_aoi: aoiByRegion.has(region.region_id),
                scene_count: 0,
              }];
            })
          : [];
        return [{
          project_id: project.project_id,
          name: project.project_name,
          safe_name: typeof project.safe_name === "string" ? project.safe_name : safeNodeName(project.project_name),
          root: `${workspaceRoot}\\${typeof project.safe_name === "string" ? project.safe_name : safeNodeName(project.project_name)}`,
          regions: nestedRegions,
        }];
      })
    : [];
  const legacyRegions = Array.isArray(context.regions)
    ? context.regions.flatMap((value) => {
        const region = recordValue(value);
        if (typeof region?.region_id !== "string" || typeof region.region_name !== "string") return [];
        return [{
          region_id: region.region_id,
          name: region.region_name,
          safe_name: safeNodeName(region.region_name),
          root: `${workspaceRoot}\\${safeNodeName(region.region_name)}`,
          has_aoi: aoiByRegion.has(region.region_id),
          scene_count: 0,
        }];
      })
    : [];
  const currentProjectId = typeof context.project_id === "string" ? context.project_id : null;
  const currentRegionId = typeof context.region_id === "string" ? context.region_id : null;
  const projectWithRegions = currentProjectId ?? projects[0]?.project_id ?? null;
  return {
    ok: true,
    workspace: { workspace_id: workspaceId, root: workspaceRoot, name: workspaceName },
    projects: projects.map((project) => {
      if (project.regions.length > 0 || project.project_id !== projectWithRegions) return project;
      return {
        ...project,
        regions: legacyRegions.filter((region) => {
          const raw = context.regions;
          const candidate = Array.isArray(raw)
            ? raw.find((item) => recordValue(item)?.region_id === region.region_id)
            : null;
          return recordValue(candidate)?.project_id === project.project_id;
        }),
      };
    }),
    current_project_id: currentProjectId,
    current_region_id: currentRegionId,
  };
}

function treeFromCoreRestore(core: CoreRestoreProjection): Tree {
  return treeFromRestoreFields(core);
}

function m1LegacyCommandError(value: { error: string; code: string | null }) {
  return value.code === null
    ? { ok: false as const, error: value.error }
    : { ok: false as const, error: value.error, code: value.code };
}

function applicationProjectionError(): ApiError {
  return {
    ok: false,
    error: "Tauri application service returned an invalid Store projection",
    code: "V3_APPLICATION_INVALID_SNAPSHOT",
  };
}

function sceneRowKey(scene: SceneRow): string {
  return sceneProductKey(scene);
}

function sceneDownloadItemId(scene: SceneRow): string {
  return sceneRowKey(scene);
}

type CoreTaskResult =
  | { ok: true; core: CoreRestoreProjection; task: CoreTaskSummaryProjection }
  | ApiError;

async function coreTaskForId(taskId: string, includeTerminal = true): Promise<CoreTaskResult> {
  const restored = await restoreApplicationCore(includeTerminal);
  if (!restored.ok) return restored;
  const task = restored.core.tasks.find((candidate) => candidate.task_id === taskId);
  return task
    ? { ok: true, core: restored.core, task }
    : { ok: false, error: "Task summary is unavailable", code: "V3_STORE_NOT_FOUND" };
}

function historyItemFromCoreSummary(task: CoreTaskSummaryProjection, archivedAt: string): DownloadArchiveItem {
  const done = task.completed_count + task.skipped_count;
  const fullySuccessful = done >= task.selected_count
    && task.selected_count > 0
    && task.failed_count === 0
    && task.cancelled_count === 0;
  const status = fullySuccessful || task.status === "completed" ? "finished" : task.status;
  const kind = task.operation?.kind === "dem"
    || task.operation?.kind === "orbit"
    || task.operation?.kind === "gacos"
    ? task.operation.kind
    : "asf";
  const name = task.operation?.kind === "dem"
    ? "DEM 正式任务"
    : task.operation?.kind === "orbit"
      ? "POEORB 正式任务"
      : task.operation?.kind === "gacos"
        ? "GACOS 请求"
        : sarTaskDisplayName({ dataset_id: task.dataset_id, name: task.provider_id });
  return {
    id: `asf-task:${task.task_id}`,
    task_id: task.task_id,
    revision: task.revision,
    approved_worker_operation: task.operation?.approved_worker_operation ?? null,
    archived_at: archivedAt,
    name,
    dataset_id: task.dataset_id,
    task_type: task.operation?.kind === "gacos" ? "GACOS request" : sarTaskTypeLabel(task),
    task_type_identity: task.task_type ?? null,
    concurrency: task.concurrency ?? null,
    created_at: task.created_at,
    ended_at: task.updated_at,
    status,
    detail: `${done}/${task.selected_count}`,
    kind,
    output_dir: task.output_root,
    total: task.selected_count,
    download_layout: task.layout,
    elapsed_seconds: task.transfer_active_millis / 1_000,
    transferred_bytes: task.transferred_bytes,
    total_bytes: task.total_bytes,
    source_total_bytes: task.source_total_bytes ?? null,
    bytes_per_second: task.bytes_per_second,
    completed_count: task.completed_count,
    active_count: task.active_count,
    queued_count: task.queued_count,
    paused_count: task.paused_count,
    failed_count: task.failed_count,
    skipped_count: task.skipped_count,
    cancelled_count: task.cancelled_count,
    ts: Date.parse(task.updated_at) || Date.parse(archivedAt) || 0,
  };
}

function normalizeResolvedAdminBoundary(value: unknown, providerFallback: string): AdminBoundary | null {
  const boundary = normalizeAdminBoundary(value, providerFallback);
  return boundary && recordValue(boundary.geometry ?? boundary.geojson) ? boundary : null;
}

function taskWorkbenchFromCoreSnapshot(
  snapshot: DesktopCoreRestoreSnapshot,
  taskId: string,
  scenes: SceneRow[],
  pageSelectedIds: string[],
  batchContext: DownloadBatchContext,
): { ok: true; workbench: DownloadTaskWorkbench } | ApiError {
  const projected = projectApplicationCoreSnapshot(snapshot);
  if (!projected.ok) return projected;
  const task = projected.core.tasks.find((candidate) => candidate.task_id === taskId);
  if (!task) return applicationProjectionError();
  const selectedPageIds = scenes
    .filter((scene) => {
      const key = sceneRowKey(scene);
      return pageSelectedIds.includes(key) || pageSelectedIds.includes(sceneDownloadItemId(scene));
    })
    .map(sceneRowKey);
  return {
    ok: true,
    workbench: {
      task_id: task.task_id,
      provider_id: task.provider_id,
      batch_context: {
        ...batchContext,
        source_batch_id: batchContext.batch_id,
        task_batch_id: task.task_id,
        revision: task.revision,
        checked_count: task.checked_count,
        candidate_count: batchContext.candidate_count ?? batchContext.total_count,
      },
      // Only the page already hydrated by the UI is retained here. The Store
      // summary remains authoritative for the cross-page task counts.
      scenes,
      checked_scene_ids: selectedPageIds,
      selected_scene_ids: selectedPageIds,
      output_dir: task.output_root,
      download_layout: task.layout,
      use_product_subdirs: task.layout !== "direct",
    },
  };
}

function createdDownloadStatusFromCoreTask(summary: CoreTaskSummaryProjection): DownloadStatus {
  const task = projectDownloadStatusFromCoreTask(summary);
  return {
    ...task,
    asf_tasks: [task],
    active_task_count: task.ok && task.state === "running" ? 1 : 0,
    task_count: 1,
    task_errors: task.ok
      ? []
      : [{ task_id: task.task_id, error: task.error ?? "下载任务状态读取失败", code: null }],
  };
}

function createdDownloadStatusFromCoreSnapshot(
  snapshot: DesktopCoreRestoreSnapshot,
  taskId: string,
): DownloadStatus | null {
  const projected = projectApplicationCoreSnapshot(snapshot);
  if (!projected.ok) return null;
  const summary = projected.core.tasks.find((candidate) => candidate.task_id === taskId);
  return summary ? createdDownloadStatusFromCoreTask(summary) : null;
}

type OpticalTaskCreationError = {
  ok: false;
  error: string;
  code?: string;
};

type OpticalTaskCreationReceipt =
  | { ok: true; task_id: string; download_status: DownloadStatus }
  | OpticalTaskCreationError;

function opticalTaskCreationReceiptFromSummary(
  task: CoreTaskSummaryProjection,
  taskId: string,
  expectedBatchId: string,
): OpticalTaskCreationReceipt {
  if (task.task_id !== taskId || task.batch_id !== expectedBatchId || task.selected_count <= 0) {
    return m1LegacyCommandError(applicationProjectionError());
  }
  return {
    ok: true,
    task_id: taskId,
    download_status: createdDownloadStatusFromCoreTask(task),
  };
}

/** A successful mutation already proves creation. An active search page is not
 * part of that transaction and may now belong to a different search entirely.
 * Details are read separately from the immutable task identity, never by replaying create. */
function opticalTaskCreationReceipt(
  snapshot: DesktopCoreRestoreSnapshot,
  taskId: string,
  expectedBatchId: string,
): OpticalTaskCreationReceipt {
  const projected = projectApplicationCoreSnapshot(snapshot);
  if (!projected.ok) return m1LegacyCommandError(projected);
  const task = projected.core.tasks.find((candidate) => candidate.task_id === taskId);
  if (!task) return m1LegacyCommandError(applicationProjectionError());
  return opticalTaskCreationReceiptFromSummary(task, taskId, expectedBatchId);
}

function opticalCreationOutcomeMayBeUnknown(error: OpticalTaskCreationError): boolean {
  return error.code === "V3_APPLICATION_UNAVAILABLE"
    || error.code === "V3_APPLICATION_INVALID_SNAPSHOT";
}

/** A lost or malformed mutation acknowledgement is an outcome-unknown read problem.
 * Reconcile the already-chosen task identity from Store. Never replay the mutation. */
async function recoverCommittedOpticalTask(
  fallback: OpticalTaskCreationError,
  taskId: string,
  expectedBatchId: string,
): Promise<OpticalTaskCreationReceipt> {
  if (!opticalCreationOutcomeMayBeUnknown(fallback)) return fallback;
  const restored = await coreTaskForId(taskId, true);
  if (!restored.ok) return fallback;
  const receipt = opticalTaskCreationReceiptFromSummary(restored.task, taskId, expectedBatchId);
  return receipt.ok ? receipt : fallback;
}

async function opticalTaskCreationReceiptOrRecover(
  snapshot: DesktopCoreRestoreSnapshot,
  taskId: string,
  expectedBatchId: string,
): Promise<OpticalTaskCreationReceipt> {
  const receipt = opticalTaskCreationReceipt(snapshot, taskId, expectedBatchId);
  return receipt.ok
    ? receipt
    : recoverCommittedOpticalTask(receipt, taskId, expectedBatchId);
}

async function recoverOpticalCreationFailure(
  failure: { ok: false; error: string; code: string | null },
  taskId: string,
  expectedBatchId: string,
): Promise<OpticalTaskCreationReceipt> {
  const projected = m1LegacyCommandError(failure);
  return recoverCommittedOpticalTask(projected, taskId, expectedBatchId);
}

async function recoverOpticalDeliveryAdmission(
  taskId: string,
): Promise<DesktopSentinel2ArchiveAdmissionSummary | undefined> {
  try {
    const result = await getDesktopShellBridge().application.taskDeliveryAdmission(taskId);
    return result.ok ? result.admission ?? undefined : undefined;
  } catch {
    return undefined;
  }
}

async function taskAndSearchWorkbenchFromCoreSnapshot(
  snapshot: DesktopCoreRestoreSnapshot,
  taskId: string,
  pageSelectedIds: string[],
  batchContext: DownloadBatchContext,
): Promise<{ ok: true; workbench: DownloadTaskWorkbench; search_workbench: ApplicationWorkbenchProjection } | ApiError> {
  const projected = projectApplicationCoreSnapshot(snapshot);
  if (!projected.ok) return projected;
  const summary = projected.core.workbench;
  if (!summary) return applicationProjectionError();
  const baseContext = batchContextFromCoreWorkbench(summary);
  const pageRequest = {
    batch_id: summary.batch_id,
    expected_revision: summary.revision,
    page_index: summary.page_index,
    page_size: MAX_UI_WORKBENCH_PAGE_SIZE,
  };
  const page = await loadApplicationWorkbenchPage(pageRequest, projected.aoi_display?.envelope);
  const searchWorkbench = applicationWorkbenchFromCorePage(projected.core, page, {
    request: pageRequest,
    base: baseContext,
    aoiAuthority: projected.aoi_display,
  });
  if (!searchWorkbench) return page.ok ? applicationProjectionError() : page;
  const task = taskWorkbenchFromCoreSnapshot(
    snapshot,
    taskId,
    searchWorkbench.scenes,
    pageSelectedIds,
    searchWorkbench.batch_context ?? batchContext,
  );
  return task.ok
    ? { ok: true, workbench: task.workbench, search_workbench: searchWorkbench }
    : task;
}

let activeM1SearchRequestId: string | null = null;

// Manual/native scene imports are a session projection until the user freezes
// a SearchBatch. Keep the last bounded projection available to another normal
// page in the same desktop session (for example Download -> local EOF match),
// without turning it into Store or task authority.
let transientImportedScenes: SceneRow[] = [];

function m1EventId(kind: string, taskId: string): string {
  const random = globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `m1-${kind}-${taskId}-${random}`.slice(0, 128);
}

async function m1UpdateTask(
  taskId: string,
  kind: DesktopUpdateDownloadTaskRequest["command"]["kind"],
  itemIds: string[],
) {
  const current = await coreTaskForId(taskId, true);
  if (!current.ok) return current;
  const result = await getDesktopShellBridge().application.updateDownloadTask({
    task_id: taskId,
    expected_revision: current.task.revision,
    command: { kind, item_ids: itemIds },
    event_id: m1EventId(kind, taskId),
    occurred_at: new Date().toISOString(),
  });
  if (!result.ok) return applicationError(result);
  const projection = projectApplicationCoreSnapshot(result);
  return projection.ok ? { ok: true as const, core: projection.core } : projection;
}

export function formatBridgeError(
  error: unknown,
  context: { dataset?: string | null } = {},
): string {
  return formatBridgeErrorValue(error, context);
}

function notifyContextChanged() {
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event("insar-context-changed"));
  }
}

let bridgeSeen = false;
export function waitForBridgeReady(timeoutMs = 1800): Promise<boolean> {
  if (typeof window === "undefined") return Promise.resolve(false);
  if (hasBridge()) return Promise.resolve(true);
  return new Promise((resolve) => {
    let settled = false;
    const finish = (ready: boolean) => {
      if (settled) return;
      settled = true;
      window.clearInterval(intervalId);
      window.clearTimeout(timeoutId);
      window.removeEventListener("insar-desktop-ready", onReady);
      window.removeEventListener("insar-context-changed", onReady);
      if (ready && !bridgeSeen) {
        bridgeSeen = true;
        notifyContextChanged();
      }
      resolve(ready);
    };
    const onReady = () => {
      if (hasBridge()) finish(true);
    };
    const intervalId = window.setInterval(() => {
      if (hasBridge()) finish(true);
    }, 80);
    const timeoutId = window.setTimeout(() => finish(hasBridge()), timeoutMs);
    window.addEventListener("insar-desktop-ready", onReady);
    window.addEventListener("insar-context-changed", onReady);
  });
}

export function watchBridgeReady(): () => void {
  if (typeof window === "undefined") return () => undefined;
  if (hasBridge()) {
    bridgeSeen = true;
    notifyContextChanged();
    return () => undefined;
  }
  const id = window.setInterval(() => {
    if (hasBridge() && !bridgeSeen) {
      bridgeSeen = true;
      notifyContextChanged();
      window.clearInterval(id);
    }
  }, 250);
  return () => window.clearInterval(id);
}

// --------------------------------------------------------------- mock state
function mockSafeName(value: string): string | null {
  const safe = value
    .trim()
    .replace(/[^A-Za-z0-9_]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return safe || null;
}

const MOCK_BBOX: Bbox = {
  west: 73.5,
  east: 135.1,
  south: 18.0,
  north: 53.6,
  crs: "EPSG:4326",
};

const MOCK_SCENE_DATES = ["20240312", "20240324", "20240405", "20240417", "20240429", "20240511"];

function mockFootprintBbox(i: number, base: Bbox = MOCK_BBOX): Bbox {
  const width = Math.max(0.02, base.east - base.west);
  const height = Math.max(0.02, base.north - base.south);
  const step = Math.min(width, height) * 0.12;
  return {
    west: base.west + step * i,
    east: base.east + step * i,
    south: base.south + step * (i % 3) * 0.45,
    north: base.north + step * (i % 3) * 0.45,
    crs: "EPSG:4326",
  };
}

function polygonFromBbox(bbox: Bbox): Json {
  return {
    type: "Polygon",
    coordinates: [
      [
        [bbox.west, bbox.south],
        [bbox.east, bbox.south],
        [bbox.east, bbox.north],
        [bbox.west, bbox.north],
        [bbox.west, bbox.south],
      ],
    ],
  };
}

function bboxFromGeojson(geojson: Json): Bbox {
  const coords: [number, number][] = [];
  const stack: unknown[] = [(geojson.geometry as Json | undefined)?.coordinates ?? geojson.coordinates];
  let visited = 0;
  while (stack.length) {
    visited += 1;
    if (visited > 1_000_000) return MOCK_BBOX;
    const value = stack.pop();
    if (!Array.isArray(value)) continue;
    if (
      value.length >= 2 &&
      typeof value[0] === "number" &&
      typeof value[1] === "number"
    ) {
      coords.push([value[0], value[1]]);
      continue;
    }
    for (const item of value) stack.push(item);
  }
  if (!coords.length) return MOCK_BBOX;
  let west = coords[0]![0];
  let east = west;
  let south = coords[0]![1];
  let north = south;
  for (let index = 1; index < coords.length; index += 1) {
    const [longitude, latitude] = coords[index]!;
    west = Math.min(west, longitude);
    east = Math.max(east, longitude);
    south = Math.min(south, latitude);
    north = Math.max(north, latitude);
  }
  return {
    west,
    east,
    south,
    north,
    crs: "EPSG:4326",
  };
}

// Browser-preview state starts empty so a delayed legacy compatibility Worker bridge never flashes
// stale demo data into the packaged desktop app.
const mock: {
  workspace: { workspace_id: string; root: string; name: string } | null;
  project: { project_id: string; name: string; safe_name: string; root?: string } | null;
  region: ContextRegion | null;
  standaloneSceneCount: number;
} = {
  workspace: null,
  project: null,
  region: null,
  standaloneSceneCount: 0,
};

let mockDataset = "COP30";
let mockCredentials: CredentialStatus = {
  ok: true,
  cdse: "none",
  usgs: "none",
  earthdata: "none",
  earthdata_input: { mode: "none", token: "", username: "", password: "" },
  bhoonidhi: "none",
  opentopography: "none",
  gacos: "none",
};
let mockNetworkSettings: NetworkSettings = {
  ok: true,
  proxy_enabled: false,
  proxy_url: "",
  cache_enabled: true,
  cache_dir: "C:\\Users\\<user>\\AppData\\Local\\EOStage\\cache",
  cache_limit_mb: 10240,
  tianditu_token: "",
  asf_ssl_verify: true,
};
let mockV3StoreSettings: V3StoreSettings = {
  ok: true,
  active_path: "C:\\Users\\<user>\\AppData\\Local\\EOStage\\v3\\tasks.sqlite3",
  database_size_bytes: 0,
  disk_free_bytes: 0,
  target_disk_free_bytes: null,
  pending_path: null,
  rollback_path: null,
  restart_required: false,
  last_migration: null,
  schema_changed: false,
  legacy_network_cache_path: "C:\\Users\\<user>\\AppData\\Local\\EOStage\\cache\\asf_search",
};

function mockSourceDatum(ds: string): string {
  if (ds.endsWith("_ELLIPSOIDAL") || ds === "NISAR_DEM") return "WGS84_ELLIPSOID";
  if (ds === "COP30" || ds === "COP90" || ds === "FABDEM") return "EGM2008";
  return "EGM96";
}

function demSourceStem(ds: string): string {
  const key = ds.toUpperCase();
  const stems: Record<string, string> = {
    SRTM_GL3: "SRTM90m",
    SRTM_GL1: "SRTM30m",
    SRTM_GL1_ELLIPSOIDAL: "SRTM30m",
    AW3D30: "AW3D30m",
    AW3D30_ELLIPSOIDAL: "AW3D30m",
    COP90: "COP90m",
    COP30: "COP30m",
    FABDEM: "FABDEM30m",
    GEM15_DTM: "GEM15DTM15m",
    GEM15_DSM: "GEM15DSM15m",
    NASADEM: "NASADEM",
    NISAR_DEM: "NISARDEM30m",
    ASTER_GDEM_V3: "ASTERGDEMv3",
  };
  return stems[key] ?? key.replace(/[^A-Za-z0-9]+/g, "_");
}

export type BoundedCoreRestoreResult =
  | { ok: true; core: CoreRestoreProjection; aoi_display: CoreAoiDisplayAuthorityProjection | null }
  | ApiError;

export type BoundedWorkbenchPageResult =
  | { ok: true; page: WorkbenchPageProjection; scenes: SceneRow[]; checked_item_ids: string[] }
  | ApiError;

export type ApplicationWorkbenchPageBinding = {
  request: Pick<DesktopWorkbenchPageRequest, "batch_id" | "expected_revision" | "page_index" | "page_size">;
  base: Pick<DownloadBatchContext, "batch_id" | "revision">;
  aoiAuthority: Pick<CoreAoiDisplayAuthorityProjection, "region_id" | "revision"> | null;
};

export type BoundedTaskPageResult =
  | { ok: true; page: TaskPageProjection; scenes: SceneRow[]; checked_item_ids: string[] }
  | ApiError;

export type TaskEventProjection = {
  sequence: number;
  occurred_at: string;
  level: "info" | "warning" | "success" | "error";
  event_kind: "started" | "progress" | "paused" | "resumed" | "completed" | "failed" | "cancelled" | "retried" | "updated";
  item_label: string | null;
  code: string | null;
  message_zh: string;
};

export type BoundedTaskEventPageResult =
  | {
      ok: true;
      task_id: string;
      revision: number;
      page_size: number;
      has_more: boolean;
      next_before_sequence: number | null;
      items: TaskEventProjection[];
    }
  | ApiError;

export function projectApplicationCoreSnapshot(
  snapshot: DesktopCoreRestoreSnapshot,
): BoundedCoreRestoreResult {
  const projection = projectCoreRestore(snapshot);
  return projection.ok ? projection : applicationError(projection);
}

export function batchContextFromCoreWorkbench(
  summary: CoreWorkbenchSummaryProjection,
): DownloadBatchContext {
  return {
    batch_id: summary.batch_id,
    created_at: summary.created_at,
    aoi_name: summary.aoi_name,
    aoi_geojson: null,
    query: {
      provider: summary.query.provider,
      dataset: summary.query.dataset,
      ...(summary.query.dataset === NISAR_RSLC_LOGICAL_DATASET
        ? { logical_dataset: summary.query.dataset }
        : {}),
      start: summary.query.start,
      end: summary.query.end,
      max_results: summary.query.target_count,
    },
    page_index: summary.page_index,
    page_size: MAX_UI_WORKBENCH_PAGE_SIZE,
    revision: summary.revision,
    total_count: summary.candidate_count,
    candidate_count: summary.candidate_count,
    checked_count: summary.checked_count,
    provider_matched_count: summary.search_stats?.matched_count ?? null,
    provider_returned_count: summary.search_stats?.provider_returned_count ?? null,
    frozen_candidate_count: summary.search_stats?.frozen_candidate_count ?? summary.candidate_count,
    current_page_item_count: summary.current_page_item_count,
    search_truncated: summary.search_stats?.truncated ?? false,
    search_source: summary.search_stats?.source ?? null,
    source_attempts: summary.search_stats?.source_attempts ?? [],
    deferred: summary.deferred,
  };
}

export function applicationWorkbenchFromCorePage(
  core: CoreRestoreProjection,
  page: BoundedWorkbenchPageResult,
  binding?: ApplicationWorkbenchPageBinding,
): ApplicationWorkbenchProjection | null {
  if (
    !page.ok ||
    !core.workbench ||
    core.workbench.batch_id !== page.page.batch_id ||
    core.workbench.revision !== page.page.revision
  ) return null;
  if (binding && !validateApplicationWorkbenchPageBinding(page, binding.request, binding.base, binding.aoiAuthority)) return null;
  const region = core.region_aoi;
  const regionId = region && typeof region.region_id === "string" && region.region_id.length > 0
    ? region.region_id
    : null;
  const regionRevision = region && typeof region.revision === "number" && Number.isSafeInteger(region.revision) && region.revision >= 0
    ? region.revision
    : null;
  const render = page.page.render;
  if (
    render &&
    (regionId === null || regionRevision === null || render.region_id !== regionId || render.revision !== regionRevision)
  ) return null;
  const summary = core.workbench;
  const batchContext = {
    ...batchContextFromCoreWorkbench(summary),
    page_index: page.page.page_index,
    page_size: page.page.page_size,
    revision: summary.revision,
    total_count: page.page.total_count,
    candidate_count: page.page.total_count,
    checked_count: page.page.checked_count,
    current_page_item_count: page.page.items.length,
  };
  return {
    batch_context: batchContext,
    scenes: page.scenes,
    checked_item_ids: page.checked_item_ids,
  };
}

export function validateApplicationWorkbenchPageBinding(
  page: BoundedWorkbenchPageResult,
  request: Pick<DesktopWorkbenchPageRequest, "batch_id" | "expected_revision" | "page_index" | "page_size">,
  base: Pick<DownloadBatchContext, "batch_id" | "revision">,
  aoiAuthority: Pick<CoreAoiDisplayAuthorityProjection, "region_id" | "revision"> | null,
): boolean {
  if (!page.ok || page.page.batch_id !== request.batch_id || page.page.batch_id !== base.batch_id) return false;
  if (page.page.revision !== request.expected_revision || page.page.revision !== base.revision) return false;
  if (page.page.page_index !== request.page_index || page.page.page_size !== request.page_size) return false;
  if (page.page.items.length > page.page.page_size || page.page.total_count < page.page.items.length) return false;
  const render = page.page.render;
  if (!aoiAuthority) return render === null;
  return render !== null && render.region_id === aoiAuthority.region_id && render.revision === aoiAuthority.revision;
}

export async function restoreApplicationCore(includeTerminal = false): Promise<BoundedCoreRestoreResult> {
  const result = await getDesktopShellBridge().application.coreRestore(includeTerminal);
  return projectApplicationCoreSnapshot(result);
}

export async function loadApplicationWorkbenchPage(
  request: DesktopWorkbenchPageRequest,
  aoiEnvelope?: CoreAoiDisplayAuthorityProjection["envelope"],
): Promise<BoundedWorkbenchPageResult> {
  const result = await getDesktopShellBridge().application.workbenchPage(request);
  const projection = projectWorkbenchPage(result, aoiEnvelope);
  if (!projection.ok) return applicationError(projection);
  const scenes = projection.page.items.map((entry) => projectFrozenItemToScene(entry.item));
  if (scenes.some((scene) => scene === null)) {
    return applicationProjectionError();
  }
  const render = projection.page.render;
  const renderByItem = new Map(render?.items.map((item) => [item.item_id, item]) ?? []);
  const renderedScenes = (scenes as SceneRow[]).map((scene, index) => {
    const item = renderByItem.get(projection.page.items[index]!.item.item_id);
    if (!item || !render) return scene;
    return {
      ...scene,
      render_geometry: item.render_geometry,
      render_bbox: item.render_bbox,
      render_authority: { region_id: render.region_id, revision: render.revision },
    };
  });
  return {
    ok: true,
    page: projection.page,
    scenes: renderedScenes,
    checked_item_ids: projection.page.items.filter((entry) => entry.checked).map((entry) => entry.item.item_id),
  };
}

function gacosTaskPageKind(page: TaskPageProjection): "request" | "import" | null {
  if (page.items.length === 0) return null;
  const providers = new Set(page.items.map((entry) => entry.item.provider_id));
  if (providers.size !== 1) return null;
  const provider = page.items[0]!.item.provider_id;
  if (provider === "gacos.webform") return "request";
  if (provider === "gacos.local-result" || provider === "gacos.manual-result") return "import";
  return null;
}

export async function loadApplicationTaskPage(
  request: DesktopTaskPageRequest,
): Promise<BoundedTaskPageResult> {
  const result = await getDesktopShellBridge().application.taskPage({
    ...request,
    page_size: Math.min(MAX_UI_TASK_PAGE_SIZE, Math.max(1, request.page_size)),
  });
  if (!result.ok) return result;
  const page = parseTaskPage(result.page);
  if (!page || page.task_id !== request.task_id || page.page_index !== request.page_index
    || page.page_size !== Math.min(MAX_UI_TASK_PAGE_SIZE, Math.max(1, request.page_size))) {
    return applicationProjectionError();
  }
  const checkedItemIds = page.items.filter((entry) => entry.checked).map((entry) => entry.item.item_id);
  if (gacosTaskPageKind(page)) {
    // GACOS request/result items are task context, not SAR catalogue rows.
    // Keep their bounded task page available without inventing SAR metadata.
    return { ok: true, page, scenes: [], checked_item_ids: checkedItemIds };
  }
  const scenes = page.items.map((entry) => projectFrozenItemToScene(entry.item));
  return {
    ok: true,
    page,
    // Delivery files need not be catalogue scenes. Preserve the complete task
    // page; never invent a scene or expose a partial compatibility selection.
    scenes: scenes.every((scene): scene is SceneRow => scene !== null) ? scenes : [],
    checked_item_ids: checkedItemIds,
  };
}

export async function loadApplicationTaskEventPage(
  request: DesktopTaskEventPageRequest,
): Promise<BoundedTaskEventPageResult> {
  const pageSize = Math.min(100, Math.max(1, request.page_size));
  const result = await getDesktopShellBridge().application.taskEventPage({
    ...request,
    before_sequence: request.before_sequence ?? null,
    page_size: pageSize,
  });
  if (!result.ok) return result;
  const page = result.page;
  const rows = Array.isArray(page.items) ? page.items : null;
  if (!rows || page.task_id !== request.task_id || !Number.isSafeInteger(page.revision)
    || Number(page.revision) < 0 ||
    page.page_size !== pageSize || typeof page.has_more !== "boolean" || rows.length > pageSize) {
    return applicationProjectionError();
  }
  const items = rows.map((value) => {
    const row = recordValue(value);
    if (!row || !Number.isSafeInteger(row.sequence) || typeof row.occurred_at !== "string" ||
      typeof row.level !== "string" || typeof row.event_kind !== "string" ||
      (row.item_label !== null && typeof row.item_label !== "string") ||
      (row.code !== null && typeof row.code !== "string") || typeof row.message_zh !== "string") return null;
    return row as TaskEventProjection;
  });
  if (items.some((item) => item === null)) return applicationProjectionError();
  return {
    ok: true,
    task_id: page.task_id as string,
    revision: page.revision as number,
    page_size: pageSize,
    has_more: page.has_more as boolean,
    next_before_sequence: page.next_before_sequence as number | null,
    items: items as TaskEventProjection[],
  };
}

export async function openApplicationTaskOutput(
  request: DesktopOpenTaskOutputRequest,
): Promise<{ ok: true; task_id: string; revision: number } | ApiError> {
  return getDesktopShellBridge().application.openTaskOutput(request);
}

export async function listApplicationTaskConflictFiles(
  request: DesktopListTaskConflictFilesRequest,
): Promise<DesktopTaskConflictFileListResult> {
  return getDesktopShellBridge().application.listTaskConflictFiles(request);
}

export async function deleteApplicationTaskConflictFiles(
  request: DesktopDeleteTaskConflictFilesRequest,
): Promise<DesktopTaskConflictFileDeleteResult> {
  return getDesktopShellBridge().application.deleteTaskConflictFiles(request);
}

export async function loadApplicationTaskHistoryPage(
  request: DesktopTaskHistoryPageRequest,
): Promise<{ ok: true; page_size: number; has_more: boolean; next_cursor: TaskHistoryCursor | null; items: Array<{ archived_at: string; task: CoreTaskSummaryProjection }> } | ApiError> {
  const pageSize = request.page_size;
  const cursorPresent = request.after_archived_at_epoch_ms != null || request.after_task_id != null;
  if (
    !Number.isSafeInteger(pageSize) || pageSize < 1 || pageSize > 100 ||
    (cursorPresent && !validTaskHistoryCursor(request.after_archived_at_epoch_ms, request.after_task_id))
  ) return applicationProjectionError();
  const result = await getDesktopShellBridge().application.taskHistoryPage({
    page_size: pageSize,
    after_archived_at_epoch_ms: request.after_archived_at_epoch_ms ?? null,
    after_task_id: request.after_task_id ?? null,
  });
  if (!result.ok) return result;
  const page = result.page;
  const items = page.items;
  const cursorRecord = recordValue(page.next_cursor);
  if (
    !Array.isArray(items) || page.page_size !== pageSize || typeof page.has_more !== "boolean" ||
    items.length > pageSize ||
    (page.has_more && !cursorRecord) || (!page.has_more && page.next_cursor !== null) ||
    (cursorRecord && !validTaskHistoryCursor(cursorRecord.archived_at_epoch_ms, cursorRecord.task_id))
  ) return applicationProjectionError();
  const summaries = items.map((value) => {
    const item = recordValue(value);
    if (!item || !validTaskHistoryTimestamp(item.archived_at)) return null;
    const { archived_at: archivedAt, ...taskValue } = item;
    const task = parseCoreTaskSummary(taskValue);
    return task ? { archived_at: archivedAt as string, task } : null;
  });
  if (summaries.some((item) => item === null)) return applicationProjectionError();
  const cursor = recordValue(page.next_cursor);
  return { ok: true, page_size: pageSize, has_more: page.has_more, next_cursor: cursor ? { archived_at_epoch_ms: cursor.archived_at_epoch_ms as number, task_id: cursor.task_id as string } : null, items: summaries as Array<{ archived_at: string; task: CoreTaskSummaryProjection }> };
}

function validTaskHistoryTimestamp(value: unknown): value is string {
  if (typeof value !== "string" || value.length < 20 || value.length > 30) return false;
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?Z$/.exec(value);
  if (!match) return false;
  const [, yearText, monthText, dayText, hourText, minuteText, secondText] = match;
  const [year, month, day, hour, minute, second] = [yearText, monthText, dayText, hourText, minuteText, secondText].map(Number);
  if (month < 1 || month > 12 || day < 1 || hour > 23 || minute > 59 || second > 59) return false;
  const date = new Date(0);
  date.setUTCHours(0, 0, 0, 0);
  date.setUTCFullYear(year, month - 1, day);
  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day;
}

function validTaskHistoryCursor(archivedAtEpochMs: unknown, taskId: unknown): boolean {
  return typeof archivedAtEpochMs === "number" && Number.isSafeInteger(archivedAtEpochMs) &&
    archivedAtEpochMs >= -62_167_219_200_000 && archivedAtEpochMs <= 253_402_300_799_999 &&
    typeof taskId === "string" && Array.from(taskId).length >= 1 && Array.from(taskId).length <= 128 && taskId.trim() === taskId &&
    !/[\u0000-\u001f\u007f]/.test(taskId);
}

export async function updateApplicationWorkbenchSelection(
  command: DesktopWorkbenchSelectionCommand,
) {
  return getDesktopShellBridge().application.updateWorkbenchSelection(command);
}

export async function createApplicationTaskFromWorkbench(
  request: DesktopCreateTaskFromWorkbenchRequest,
) {
  return getDesktopShellBridge().application.createTaskFromWorkbench(request);
}

export async function appendApplicationTaskFromWorkbench(
  request: DesktopAppendTaskFromWorkbenchRequest,
) {
  return getDesktopShellBridge().application.appendTaskFromWorkbench(request);
}

export async function loadApplicationTaskSourceBatchPage(
  request: DesktopTaskSourceBatchPageRequest,
) {
  return getDesktopShellBridge().application.taskSourceBatchPage(request);
}

export async function appendApplicationTaskFromSourceBatch(
  request: DesktopAppendTaskFromSourceBatchRequest,
) {
  return getDesktopShellBridge().application.appendTaskFromSourceBatch(request);
}

export async function createApplicationTaskFromHistoricalSourceBatch(
  request: DesktopCreateTaskFromHistoricalSourceBatchRequest,
) {
  return getDesktopShellBridge().application.createTaskFromHistoricalSourceBatch(request);
}

export async function createApplicationBurstAssetTask(
  request: DesktopBurstCreateAssetTaskRequest,
) {
  const result = await getDesktopShellBridge().application.createBurstAssetTask(request);
  if (!result.ok) return result;
  // The committed task is the receipt; its Store summary carries the status.
  const downloadStatus = createdDownloadStatusFromCoreSnapshot(result, request.task_id);
  return downloadStatus
    ? { ok: true as const, task_id: request.task_id, download_status: downloadStatus }
    : m1LegacyCommandError(applicationProjectionError());
}

export async function appendApplicationBurstAssetTask(
  request: DesktopBurstAppendAssetTaskRequest,
) {
  return getDesktopShellBridge().application.appendBurstAssetTask(request);
}

export async function createApplicationHistoricalBurstAssetTask(
  request: DesktopBurstCreateHistoricalAssetTaskRequest,
) {
  return getDesktopShellBridge().application.createHistoricalBurstAssetTask(request);
}

type FormalTaskProjection = CoreTaskSummaryProjection;
type FormalTaskRegionSnapshot = {
  ok: true;
  region: ContextRegion;
  tasks: FormalTaskProjection[];
  taskIds: string[];
};

/**
 * Rebuilds formal task identity from the Store-backed projection. The maps used
 * while a task is being created are only a fast-path cache and are deliberately
 * not authoritative after a desktop restart.
 */
async function formalTaskIdsForCurrentRegion(
  kind: "dem" | "orbit",
): Promise<FormalTaskRegionSnapshot | ApiError> {
  const context = await getContext();
  if (!context.ok) return context;
  if (!context.region) {
    return {
      ok: false,
      error: "当前没有可恢复的研究区正式任务",
      code: "FORMAL_TASK_REGION_UNSET",
    };
  }
  const current = await restoreApplicationCore(false);
  if (!current.ok) return current;
  const region = context.region;
  const tasks = current.core.tasks.filter((task) =>
    task.operation?.kind === kind && task.operation.region_id === region.region_id,
  );
  return {
    ok: true,
    region,
    tasks,
    taskIds: tasks.map((task) => task.task_id),
  };
}

export { type FormalDemIdentity, formalDemIdentity };

function mockReportsDir(): string {
  const root = mock.workspace?.root ?? "workspace_preview";
  const safe = mock.region?.safe_name ?? "region";
  return `${root}\\${safe}\\07_reports`;
}

function mockActiveSceneCount(): number {
  return mock.region?.scene_count ?? mock.standaloneSceneCount;
}

function mockActiveSafeName(): string {
  return mock.region?.safe_name ?? "standalone_download";
}

function ensureMockRegion(): ContextRegion {
  if (!mock.workspace) {
    mock.workspace = {
      workspace_id: "ws_preview",
      root: "workspace_preview",
      name: "预览工作区",
    };
  }
  if (!mock.project) {
    mock.project = {
      project_id: "proj_default_task",
      name: "默认任务",
      safe_name: "default_task",
      root: `${mock.workspace.root}\\default_task`,
    };
  }
  if (!mock.region) {
    mock.region = {
      region_id: "region_default_area",
      name: "默认区域",
      safe_name: "default_area",
      root: `${mock.project.root}\\default_area`,
      has_aoi: false,
      bbox: null,
      scene_count: 0,
    };
  }
  return mock.region;
}

function isEmptyAdminValue(value: string) {
  return ADMIN_EMPTY_VALUES.has(value.trim());
}

function allPreviewDistricts(province: string): AdminBoundaryLite[] {
  const cityRows = ADMIN_INDEX.citiesByProvince[province] ?? [];
  return cityRows.flatMap((city) => ADMIN_INDEX.districtsByCity[`${province}|${city[0]}`] ?? []);
}

// ------------------------------------------------------------------- app/ctx
export async function getAppInfo(): Promise<AppInfo> {
  return getDesktopShellBridge().app.getInfo();
}

/** Read-only Rust Application Service access used by the Tauri adapter.
 *
 * The formal Tauri path routes the still-legacy GIS calls through the managed
 * compatibility Worker. Browser Fixture injection remains test-only; it never
 * fabricates Store data or creates a second business state source.
 */
export async function getApplicationServiceReadiness() {
  return getDesktopShellBridge().application.getReadiness();
}

export async function restoreApplicationServiceSnapshot(includeTerminal = false) {
  return restoreApplicationCore(includeTerminal);
}

/** Named M1 business operations stay behind the one DesktopShellBridge. */
export async function getApplicationProviderCapabilities() {
  return getDesktopShellBridge().application.providerCapabilities();
}

export async function applicationSarSearchAndFreeze(
  request: DesktopSarSearchAndFreezeRequest,
) {
  return getDesktopShellBridge().application.sarSearchAndFreeze(request);
}

export async function applicationCatalogSearchAndFreeze(
  request: DesktopCatalogSearchAndFreezeRequest,
) {
  return getDesktopShellBridge().application.catalogSearchAndFreeze(request);
}

/** Issue #3 R2 B4: search a product by route-neutral intent; the application plans the route. */
export async function applicationCatalogIntentSearchAndFreeze(
  request: DesktopCatalogIntentSearchAndFreezeRequest,
) {
  return getDesktopShellBridge().application.catalogIntentSearchAndFreeze(request);
}

/** Session-only CDSE desktop operations. Login input is passed through once. */
export async function getCdseSessionStatus() {
  return getDesktopShellBridge().application.cdseSessionStatus();
}

export async function loginCdseSession(request: DesktopCdseLoginRequest) {
  return getDesktopShellBridge().application.cdseLogin(request);
}

export async function logoutCdseSession() {
  return getDesktopShellBridge().application.cdseLogout();
}

export async function getCdseNetworkPolicy() {
  return getDesktopShellBridge().application.cdseNetworkPolicy();
}

export async function setCdseNetworkPolicy(networkMode: DesktopCdseNetworkMode) {
  return getDesktopShellBridge().application.setCdseNetworkPolicy(networkMode);
}

export async function runCdseNetworkDiagnostic(networkMode: DesktopCdseNetworkMode) {
  return getDesktopShellBridge().application.cdseNetworkDiagnostic(networkMode);
}

export async function getPreviewAsset(request: DesktopPreviewAssetRequest) {
  return getDesktopShellBridge().application.previewAsset(request);
}

export async function getCdseQuicklook(request: DesktopCdseQuicklookRequest) {
  return getDesktopShellBridge().application.cdseQuicklook(request);
}

export async function getSentinel2MapPreview(request: DesktopSentinel2MapPreviewRequest) {
  return getDesktopShellBridge().application.sentinel2MapPreview(request);
}

/** Runtime-only USGS M2M operations; login material is passed through once. */
export async function getUsgsSessionStatus() {
  return getDesktopShellBridge().application.usgsSessionStatus();
}

export async function loginUsgsSession(request: DesktopUsgsM2mLoginRequest) {
  return getDesktopShellBridge().application.usgsLogin(request);
}

export async function logoutUsgsSession() {
  return getDesktopShellBridge().application.usgsLogout();
}

export async function getLandsatBrowse(request: DesktopLandsatBrowseRequest) {
  return getDesktopShellBridge().application.landsatBrowse(request);
}

export async function getLandsatAssetDetails(request: DesktopLandsatAssetDetailsRequest) {
  return getDesktopShellBridge().application.landsatAssetDetails(request);
}

export async function getBhoonidhiAssetSize(request: DesktopBhoonidhiAssetSizeRequest) {
  return getDesktopShellBridge().application.bhoonidhiAssetSize(request);
}

export async function createApplicationLandsatBundleTask(
  request: DesktopCreateLandsatBundleTaskRequest,
) {
  return getDesktopShellBridge().application.createLandsatBundleTask(request);
}

export async function createApplicationLandsatAssetTask(
  request: DesktopCreateLandsatAssetTaskRequest,
) {
  return getDesktopShellBridge().application.createLandsatAssetTask(request);
}

export async function createApplicationReviewedMpcAssetTask(
  request: DesktopCreateReviewedMpcAssetTaskRequest,
) {
  return getDesktopShellBridge().application.createReviewedMpcAssetTask(request);
}

export async function createApplicationSentinel2ArchiveTask(
  request: DesktopCreateSentinel2ArchiveTaskRequest,
) {
  return getDesktopShellBridge().application.createSentinel2ArchiveTask(request);
}

export async function createApplicationSentinel2ArchiveBatchTask(
  request: DesktopCreateSentinel2ArchiveBatchTaskRequest,
) {
  return getDesktopShellBridge().application.createSentinel2ArchiveBatchTask(request);
}

export async function createApplicationSentinel2BandTask(
  request: DesktopCreateSentinel2BandTaskRequest,
) {
  return getDesktopShellBridge().application.createSentinel2BandTask(request);
}

function formalTaskRouteUnavailable(): ApiError {
  return {
    ok: false,
    error: "当前桌面宿主未暴露正式 DEM/Orbit 任务路由",
    code: "FORMAL_TASK_ROUTE_UNAVAILABLE",
  };
}

export async function planApplicationDemTask(request: DesktopDemTaskPlanRequest) {
  const route = getDesktopShellBridge().application.planDemTask;
  return route ? route(request) : formalTaskRouteUnavailable();
}

export async function createApplicationDemTask(request: DesktopDemTaskRequest) {
  const route = getDesktopShellBridge().application.createDemTask;
  return route ? route(request) : formalTaskRouteUnavailable();
}

export async function runApplicationStandaloneDem(request: DesktopStandaloneDemRequest) {
  const route = getDesktopShellBridge().application.standaloneDem;
  return route ? route(request) : formalTaskRouteUnavailable();
}

async function createApplicationStandaloneDemTask(
  request: DesktopStandaloneDemTaskRequest,
): Promise<DemTaskCommandResult> {
  const route = getDesktopShellBridge().application.createStandaloneDemTask;
  if (!route) return formalTaskRouteUnavailable();
  const result = await route(request);
  if ("ok" in result && result.ok === false) return applicationError(result);
  return { ok: true, task_id: request.task_id, state: request.convert ? "running" : "queued" };
}

async function importApplicationScenes(request: DesktopSceneImportRequest): Promise<ScenesResult> {
  const route = getDesktopShellBridge().application.importScenes;
  if (!route) return formalTaskRouteUnavailable();
  const result = await route(request);
  if ("ok" in result && result.ok === false) return applicationError(result);
  const value = recordValue(result);
  if (!value || value.ok !== true || !Array.isArray(value.scenes)) {
    return { ok: false, error: "ApplicationService 未返回有效场景导入结果", code: "SCENE_IMPORT_INVALID" };
  }
  const scenes = value.scenes as SceneRow[];
  transientImportedScenes = scenes;
  return {
    ok: true,
    scenes,
    duplicates: Array.isArray(value.duplicates) ? value.duplicates.filter((item): item is string => typeof item === "string") : [],
    errors: Array.isArray(value.errors)
      ? value.errors.filter((item): item is { line: string; error: string } => {
          const record = recordValue(item);
          return typeof record?.line === "string" && typeof record.error === "string";
        })
      : [],
    queried: typeof value.queried === "number" ? value.queried : undefined,
    search: recordValue(value.search) as ScenesOk["search"],
  };
}

async function matchApplicationOrbits(
  request: DesktopOrbitDirectoryMatchRequest,
): Promise<OrbitMatchResult> {
  const route = getDesktopShellBridge().application.matchOrbitsDirectory;
  if (!route) return formalTaskRouteUnavailable();
  const result = await route(request);
  if ("ok" in result && result.ok === false) return applicationError(result);
  const value = recordValue(result);
  if (!value || value.ok !== true || typeof value.orbit_dir !== "string") {
    return { ok: false, error: "ApplicationService 未返回有效轨道匹配结果", code: "ORBIT_MATCH_INVALID" };
  }
  return {
    ok: true,
    orbit_dir: value.orbit_dir,
    orbit_files: typeof value.orbit_files === "number" ? value.orbit_files : 0,
    report: (recordValue(value.report) ?? {}) as Json,
  };
}

async function importApplicationGacosResult(
  request: DesktopGacosResultImportRequest,
): Promise<Json | ApiError> {
  const route = getDesktopShellBridge().application.gacosImportResult;
  if (!route) return demProtocolError("gacos_import_result");
  const result = await route(request);
  if ("ok" in result && result.ok === false) return applicationError(result);
  const value = recordValue(result);
  return value?.ok === true
    ? value as Json
    : { ok: false, error: "ApplicationService 未返回有效 GACOS 导入结果", code: "GACOS_IMPORT_INVALID" };
}

export async function handoffApplicationDemConversion(request: DesktopDemConversionRequest) {
  const route = getDesktopShellBridge().application.handoffDemConversion;
  return route ? route(request) : formalTaskRouteUnavailable();
}

export async function resolveApplicationDemConversionSource(
  request: DesktopDemConversionSourceRequest,
) {
  const route = getDesktopShellBridge().application.resolveDemConversionSource;
  return route ? route(request) : formalTaskRouteUnavailable();
}

export async function createApplicationCompanionOrbitTask(sourceTaskId: string) {
  const route = getDesktopShellBridge().application.createCompanionOrbitTask;
  return route ? route(sourceTaskId) : formalTaskRouteUnavailable();
}

export async function createApplicationOrbitTask(request: DesktopOrbitTaskRequest) {
  const route = getDesktopShellBridge().application.createOrbitTask;
  return route ? route(request) : formalTaskRouteUnavailable();
}

export async function createApplicationStandaloneOrbitTask(
  request: DesktopStandaloneOrbitTaskRequest,
) {
  const route = getDesktopShellBridge().application.createStandaloneOrbitTask;
  return route ? route(request) : formalTaskRouteUnavailable();
}

export async function getNisarRemoteSubsetCapability() {
  return getDesktopShellBridge().application.nisarRemoteSubsetCapability();
}

export async function getApplicationEffectiveCapability(capabilityId: string) {
  return getDesktopShellBridge().application.effectiveCapability(capabilityId);
}

export async function createNisarRemoteSubsetTask(
  request: DesktopCreateNisarRemoteSubsetTaskRequest,
) {
  return getDesktopShellBridge().application.createNisarRemoteSubsetTask(request);
}

export async function prepareNisarSubsetIntentV2(
  request: DesktopNisarPrepareSubsetIntentV2Request,
): Promise<DesktopNisarPreparedSubsetIntentV2Result> {
  return getDesktopShellBridge().application.prepareNisarSubsetIntentV2(request);
}

export async function confirmNisarSubsetTaskV2(
  request: DesktopNisarConfirmSubsetTaskV2Request,
) {
  return getDesktopShellBridge().application.confirmNisarSubsetTaskV2(request);
}

export async function preflightNisarSubsetBatchV2(
  request: DesktopNisarPreflightSubsetBatchV2Request,
): Promise<DesktopNisarPreflightSubsetBatchV2Result> {
  return getDesktopShellBridge().application.preflightNisarSubsetBatchV2(request);
}

export async function prepareNisarSubsetBatchV2(
  request: DesktopNisarPrepareSubsetBatchV2Request,
): Promise<DesktopNisarPreparedSubsetBatchV2Result> {
  return getDesktopShellBridge().application.prepareNisarSubsetBatchV2(request);
}

export async function confirmNisarSubsetBatchTaskV2(
  request: DesktopNisarConfirmSubsetBatchTaskV2Request,
) {
  return getDesktopShellBridge().application.confirmNisarSubsetBatchTaskV2(request);
}

export async function appendNisarSubsetBatchTaskV2(
  request: DesktopNisarAppendSubsetBatchTaskV2Request,
) {
  return getDesktopShellBridge().application.appendNisarSubsetBatchTaskV2(request);
}

export async function readNisarSubsetAppendContextV2(request: DesktopNisarAppendContextV2Request) {
  return getDesktopShellBridge().application.readNisarSubsetAppendContextV2(request);
}

export async function preflightNisarSubsetAppendV2(request: DesktopNisarAppendPreflightV2Request) {
  return getDesktopShellBridge().application.preflightNisarSubsetAppendV2(request);
}

export async function createNisarSubsetSuccessorTaskV2(request: DesktopNisarSuccessorTaskV2Request) {
  return getDesktopShellBridge().application.createNisarSubsetSuccessorTaskV2(request);
}

export async function readNisarArchivedRecoveryContextV2(sourceTaskId: string, newTaskId: string) {
  return getDesktopShellBridge().application.readNisarArchivedRecoveryContextV2(sourceTaskId, newTaskId);
}

export async function createNisarArchivedRecoveryV2(request: import("./desktopShellBridge").DesktopNisarArchivedRecoveryV2Request) {
  return getDesktopShellBridge().application.createNisarArchivedRecoveryV2(request);
}

export async function resumeNisarSubsetTask(taskId: string) {
  return getDesktopShellBridge().application.resumeNisarSubsetTask(taskId);
}

export async function startNewlyConfirmedTask(taskId: string) {
  return getDesktopShellBridge().application.startNewlyConfirmedTask(taskId);
}

export async function cancelApplicationRequest(requestId: string) {
  return getDesktopShellBridge().application.cancelRequest(requestId);
}

export async function getSarSearchProgress(requestId: string) {
  return getDesktopShellBridge().application.sarSearchProgress(requestId);
}

export async function updateApplicationSearchWorkbench(
  request: DesktopUpdateSearchWorkbenchRequest,
) {
  return getDesktopShellBridge().application.updateSearchWorkbench(request);
}

export async function createApplicationDownloadTask(
  request: DesktopCreateDownloadTaskRequest,
) {
  return getDesktopShellBridge().application.createDownloadTask(request);
}

export async function appendApplicationDownloadTask(
  request: DesktopAppendDownloadTaskRequest,
) {
  return getDesktopShellBridge().application.appendDownloadTask(request);
}

export async function updateApplicationDownloadTask(
  request: DesktopUpdateDownloadTaskRequest,
) {
  return getDesktopShellBridge().application.updateDownloadTask(request);
}

export async function updateApplicationDownloadTaskView(
  request: DesktopUpdateDownloadTaskViewRequest,
) {
  return getDesktopShellBridge().application.updateDownloadTaskView(request);
}

export async function archiveApplicationDownloadTask(
  request: DesktopArchiveDownloadTaskRequest,
) {
  return getDesktopShellBridge().application.archiveDownloadTask(request);
}

export async function deleteApplicationDownloadHistory(
  request: DesktopDeleteDownloadHistoryRequest,
) {
  return getDesktopShellBridge().application.deleteDownloadHistory(request);
}

export async function checkForUpdate(force = false): Promise<UpdateInfo | ApiError> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("check_for_update");
  if (hasBridge()) {
    const checker = api().check_for_update;
    if (typeof checker === "function") return checker(force);
  }
  return {
    ok: true,
    checked: false,
    update_available: false,
    current_version: "2.1.9",
    latest_version: "2.1.9",
    html_url: "https://github.com/hhanmj/EOStage/releases/latest",
    release_name: "EOStage 2.2.0-beta.0",
    changelog: "Update checks run only in the packaged desktop app.",
    published_at: "",
    message: "Update checks run only in the packaged desktop app.",
  };
}

export async function downloadAppUpdate(
  downloadUrl = "",
  assetName = "",
  mirrorUrls: string[] = [],
  expectedSize = 0,
): Promise<AppUpdateDownloadOk | ApiError> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("download_app_update");
  if (hasBridge()) {
    const downloader = api().download_app_update;
    if (typeof downloader === "function") return downloader(downloadUrl, assetName, mirrorUrls, expectedSize);
  }
  return {
    ok: false,
    error: "在线下载更新包只在桌面版中可用。",
    code: "GUI003",
  };
}

export async function getComponentStatus(refresh = false): Promise<ComponentStatusOk | ApiError> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("get_component_status");
  if (hasBridge()) {
    const getter = api().get_component_status;
    if (typeof getter === "function") return getter(refresh);
  }
  return {
    ok: true,
    root: "C:\\Users\\You\\AppData\\Local\\EOStage\\components",
    manifest_url: "https://github.com/hhanmj/EOStage/releases/latest/download/components-manifest.json",
    components: [
      {
        id: "dem-gdal",
        name: "DEM/GDAL 高程基准组件",
        version: "2.1.9",
        size_mb: 205,
        description: "GDAL/rasterio/numpy/PROJ 与 EGM96、EGM2008 高程基准数据。",
        installed: false,
        runtime_available: false,
        partial_runtime_available: false,
        egm2008_grid_available: false,
        state: "available",
        can_install: true,
      },
    ],
  };
}

export async function installComponent(componentId: string): Promise<ComponentStatusOk | ApiError> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("install_component");
  if (hasBridge()) {
    const installer = api().install_component;
    if (typeof installer === "function") return installer(componentId);
  }
  return { ok: false, error: "组件安装只在桌面版中可用。", code: "GUI003" };
}

export async function removeComponent(componentId: string): Promise<ComponentStatusOk | ApiError> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("remove_component");
  if (hasBridge()) {
    const remover = api().remove_component;
    if (typeof remover === "function") return remover(componentId);
  }
  return { ok: false, error: "组件移除只在桌面版中可用。", code: "GUI003" };
}

export async function getContext(): Promise<Context> {
  if (isTauriDesktopHost()) {
    const snapshot = await restoreApplicationCore(false);
    return snapshot.ok
      ? contextFromCoreRestore(snapshot.core, snapshot.aoi_display)
      : {
          ok: false,
          error: snapshot.error,
          code: snapshot.code ?? "V3_APPLICATION_UNAVAILABLE",
        };
  }
  if (hasBridge()) {
    const res = await api().get_context();
    if (res.ok) notifyContextChanged();
    return res;
  }
  return {
    ok: true,
    workspace: mock.workspace,
    project: mock.project,
    region: mock.region,
    dem_dataset: mockDataset,
  };
}

export async function setDemDataset(
  dataset: string,
  requestedProvider = "auto",
): Promise<{ ok: boolean; dataset?: string; provider?: string; error?: string; code?: string }> {
  if (isTauriDesktopHost()) {
    const identity = formalDemIdentity(dataset, requestedProvider);
    if (!identity) {
      return { ok: false, error: "该 DEM 数据集/provider 组合未在 ResourceRegistry 中登记", code: "RESOURCE_REGISTRY_UNAVAILABLE" };
    }
    const setter = getDesktopShellBridge().application.setDemSourceIdentity;
    if (!setter) {
      return { ok: false, error: "正式 DEM identity 路由不可用", code: "V3_APPLICATION_UNAVAILABLE" };
    }
    const updatedAt = canonicalDemIdentityTimestamp(new Date().toISOString());
    if (!updatedAt) {
      return { ok: false, error: "DEM identity 时间戳不可规范化", code: "V3_APPLICATION_INVALID_REQUEST" };
    }
    const confirmed = await setter({
      dataset: identity.dataset,
      provider: identity.provider,
      updated_at: updatedAt,
    });
    if (!confirmed.ok) {
      return { ok: false, error: confirmed.error, code: confirmed.code ?? undefined };
    }
    notifyContextChanged();
    return { ok: true, dataset: confirmed.identity.dataset, provider: confirmed.identity.provider };
  }
  if (hasBridge()) {
    const res = await api().set_dem_dataset(dataset);
    if (res.ok) notifyContextChanged();
    return res;
  }
  mockDataset = dataset;
  notifyContextChanged();
  return { ok: true, dataset };
}

export async function getTree(): Promise<Tree> {
  if (isTauriDesktopHost()) {
    const snapshot = await restoreApplicationCore(false);
    return snapshot.ok ? treeFromCoreRestore(snapshot.core) : snapshot;
  }
  if (hasBridge()) return api().get_tree();
  const projects: TreeProject[] = [];
  if (mock.project) {
    projects.push({
      ...mock.project,
      regions: mock.region
        ? [
            {
              region_id: mock.region.region_id,
              name: mock.region.name,
              safe_name: mock.region.safe_name,
              has_aoi: mock.region.has_aoi,
              scene_count: mock.region.scene_count,
            },
          ]
        : [],
    });
  }
  return {
    ok: true,
    workspace: mock.workspace,
    projects,
    current_project_id: mock.project?.project_id ?? null,
    current_region_id: mock.region?.region_id ?? null,
  };
}

export async function getWorkflowStatus(): Promise<WorkflowStatus> {
  if (hasBridge() && !isTauriDesktopHost()) return api().get_workflow_status();
  const context = await getContext();
  const tree = await getTree();
  const download = await getDownloadStatus();
  const workspaceReady = !!context.workspace && !!context.project && !!context.region;
  const aoiReady = workspaceReady && !!context.region?.has_aoi;
  const sceneCount = context.region?.scene_count ?? mock.standaloneSceneCount;
  const scenesReady = sceneCount > 0;
  const dlActive = download.state === "running" || download.state === "paused";
  const creds = await getCredentialStatus();
  const credentialsReady = [creds.earthdata, creds.opentopography, creds.gacos].every(
    (v) => v !== "none" && v !== "unavailable",
  );
  const stages: WorkflowStage[] = [
    {
      id: "settings",
      label: "设置",
      nav: "settings",
      status: credentialsReady ? "done" : "active",
      summary: "配置 Earthdata、OpenTopography 和 GACOS。",
      ready: credentialsReady,
      count: credentialsReady ? 1 : 0,
    },
    {
      id: "workspace",
      label: "项目",
      nav: "workspace",
      status: workspaceReady ? "done" : "active",
      summary: "创建或选择当前项目和研究区。",
      ready: workspaceReady,
      count: workspaceReady ? 1 : 0,
    },
    {
      id: "aoi",
      label: "AOI",
      nav: "aoi",
      status: aoiReady ? "done" : workspaceReady ? "active" : "blocked",
      summary: "绘制或导入处理范围。",
      ready: aoiReady,
      count: aoiReady ? 1 : 0,
    },
    {
      id: "scenes",
      label: "影像",
      nav: "download",
      status: scenesReady ? "done" : "active",
      summary: "导入 ASF 购物车或本地目录；AOI 可选绑定。",
      ready: scenesReady,
      count: sceneCount,
    },
    {
      id: "downloads",
      label: "下载",
      nav: "download",
      status: dlActive ? "running" : download.state === "finished" ? "done" : scenesReady ? "active" : "blocked",
      summary: "规划并执行 SLC、DEM 和 GACOS。",
      ready: scenesReady,
      count: download.done,
    },
    {
      id: "convert",
      label: "DEM 转换",
      nav: "convert",
      status: aoiReady ? "active" : "blocked",
      summary: "生成 SARscape 可用的椭球高 DEM。",
      ready: aoiReady,
      count: 0,
    },
    {
      id: "report",
      label: "报告",
      nav: "report",
      status: scenesReady ? "active" : "blocked",
      summary: "生成报告、清单和警告表。",
      ready: scenesReady,
      count: 0,
    },
  ];
  const next_action = !credentialsReady
    ? { label: "配置下载凭据", nav: "settings" }
    : !workspaceReady
      ? { label: "创建项目", nav: "workspace" }
      : !scenesReady
        ? { label: "导入影像", nav: "download" }
        : !aoiReady
          ? { label: "可选绑定 AOI", nav: "aoi" }
          : dlActive
            ? { label: "查看下载", nav: "download" }
            : { label: "生成报告", nav: "report" };
  return {
    ok: true,
    context,
    stages,
    sources: [
      {
        id: "asf",
        label: "Sentinel-1",
        credential: creds.earthdata,
        status: ["none", "unavailable"].includes(creds.earthdata) ? "needs_config" : "configured",
        capabilities: ["cart import", "consistency check", "real download"],
      },
      {
        id: "dem",
        label: "OpenTopography DEM",
        credential: creds.opentopography,
        status: ["none", "unavailable"].includes(creds.opentopography) ? "needs_config" : "configured",
        capabilities: ["request plan", "real download", "vertical conversion"],
      },
      {
        id: "gacos",
        label: "GACOS",
        credential: creds.gacos,
        status: ["none", "unavailable"].includes(creds.gacos) ? "needs_config" : "configured",
        capabilities: ["request plan", "email result import"],
      },
      {
        id: "report",
        label: "Preparation report",
        credential: "local",
        status: scenesReady ? "ready" : "waiting",
        capabilities: ["html", "markdown", "manifest", "warnings"],
      },
    ],
    download,
    next_action,
    counts: {
      projects: tree.ok ? tree.projects.length : 0,
      regions: tree.ok ? tree.projects.reduce((n, p) => n + p.regions.length, 0) : 0,
      scenes: sceneCount,
    },
    scene_coverage: {
      bbox: context.region?.scene_footprint_bbox ?? null,
      count: context.region?.scene_footprint_bbox ? context.region.scene_count : 0,
    },
    dem_dataset: context.dem_dataset ?? "COP30",
  };
}

export async function getActivity(limit = 12): Promise<ActivityFeed> {
  if (hasBridge() && !isTauriDesktopHost()) return api().get_activity(limit);
  return { ok: true, activities: [] };
}

export async function getUiFlags(): Promise<UiFlagsOk | ApiError> {
  if (!hasBridge()) await waitForBridgeReady();
  if (hasBridge() && !isTauriDesktopHost()) {
    const getter = api().get_ui_flags;
    if (typeof getter === "function") return getter();
  }
  if (typeof window === "undefined") return { ok: true, flags: {} };
  return {
    ok: true,
    flags: {
      workbench_tour_seen: window.localStorage.getItem("eostage:workbench-tour-seen") === "1",
    },
  };
}

export async function setUiFlag(key: string, value = true): Promise<UiFlagsOk | ApiError> {
  if (!hasBridge()) await waitForBridgeReady();
  if (hasBridge() && !isTauriDesktopHost()) {
    const setter = api().set_ui_flag;
    if (typeof setter === "function") return setter(key, value);
  }
  if (typeof window !== "undefined") {
    window.localStorage.setItem(`eostage:${key.replace(/_/g, "-")}`, value ? "1" : "0");
  }
  return getUiFlags();
}

let mockDownloadArchive: DownloadArchiveItem[] = [];

export async function getDownloadArchive(cursor?: TaskHistoryCursor | null): Promise<DownloadArchiveResult | ApiError> {
  if (isTauriDesktopHost()) {
    const history = await loadApplicationTaskHistoryPage({ page_size: 40, after_archived_at_epoch_ms: cursor?.archived_at_epoch_ms ?? null, after_task_id: cursor?.task_id ?? null });
    return history.ok ? { ok: true, items: history.items.map((item) => historyItemFromCoreSummary(item.task, item.archived_at)), has_more: history.has_more, next_cursor: history.next_cursor } : history;
  }
  if (hasBridge()) return api().get_download_archive();
  return { ok: true, items: mockDownloadArchive };
}

export async function saveDownloadArchive(
  items: DownloadArchiveItem[],
): Promise<DownloadArchiveResult | ApiError> {
  if (isTauriDesktopHost()) return getDownloadArchive();
  if (hasBridge()) return api().save_download_archive(items);
  mockDownloadArchive = items.slice(0, 40);
  return { ok: true, items: mockDownloadArchive };
}

export async function deleteDownloadArchiveItem(
  item: DownloadArchiveItem,
): Promise<DownloadArchiveResult | ApiError> {
  if (isTauriDesktopHost()) {
    const taskId = (item.task_id ?? item.id.replace(/^asf-task:/, "")).trim();
    if (!taskId) return { ok: false, error: "History task identity is unavailable", code: "V3_APPLICATION_INVALID_REQUEST" };
    const result = await getDesktopShellBridge().application.deleteDownloadHistory({
      task_id: taskId,
      deleted_at: new Date().toISOString(),
    });
    return result.ok ? getDownloadArchive() : applicationError(result);
  }
  if (hasBridge()) return api().delete_download_archive_item(item);
  const key = `${item.kind || ""}:${(item.output_dir || item.id).replace(/[\\/]+$/, "").toLowerCase()}`;
  mockDownloadArchive = mockDownloadArchive.filter((row) => {
    const rowKey = `${row.kind || ""}:${(row.output_dir || row.id).replace(/[\\/]+$/, "").toLowerCase()}`;
    return row.id !== item.id && rowKey !== key;
  });
  return { ok: true, items: mockDownloadArchive };
}

export async function getNetworkSettings(): Promise<NetworkSettings | ApiError> {
  if (isTauriDesktopHost()) {
    const route = getDesktopShellBridge().application.getNetworkSettings;
    if (!route) return demProtocolError("get_network_settings");
    const result = await route();
    if ("ok" in result && result.ok === false) return applicationError(result);
    const settings = result as DesktopNetworkSettings;
    return { ok: true, ...settings };
  }
  if (hasBridge()) return api().get_network_settings();
  return mockNetworkSettings;
}

export async function saveNetworkSettings(
  settings: NetworkSettingsInput,
): Promise<NetworkSettings | ApiError> {
  const cleanSettings: DesktopNetworkSettings = {
    proxy_enabled: Boolean(settings.proxy_enabled),
    proxy_url: typeof settings.proxy_url === "string" ? settings.proxy_url : "",
    cache_enabled: settings.cache_enabled !== false,
    cache_dir: typeof settings.cache_dir === "string" ? settings.cache_dir : "",
    cache_limit_mb: Number.isFinite(Number(settings.cache_limit_mb)) ? Number(settings.cache_limit_mb) : 0,
    tianditu_token: typeof settings.tianditu_token === "string" ? settings.tianditu_token : "",
    asf_ssl_verify: settings.asf_ssl_verify !== false,
  };
  if (isTauriDesktopHost()) {
    const route = getDesktopShellBridge().application.saveNetworkSettings;
    if (!route) return demProtocolError("save_network_settings");
    const result = await route(cleanSettings);
    if ("ok" in result && result.ok === false) return applicationError(result);
    const saved = result as DesktopNetworkSettings;
    return { ok: true, ...saved };
  }
  if (hasBridge()) return api().save_network_settings(cleanSettings);
  mockNetworkSettings = {
    ok: true,
    ...cleanSettings,
  };
  return mockNetworkSettings;
}

export type ProviderConnectivityOutcome = {
  ok: true;
  results: {
    id: string;
    site: string;
    ok: boolean;
    latency_ms: number;
    http_status: number | null;
    error_code: string;
  }[];
};

export async function probeProviderConnectivity(): Promise<
  ProviderConnectivityOutcome | ApiError
> {
  if (isTauriDesktopHost()) {
    const route = getDesktopShellBridge().application.probeProviderConnectivity;
    if (!route) return demProtocolError("probe_provider_connectivity");
    const result = await route();
    if ("ok" in result && result.ok === false) return applicationError(result);
    const results = result as DesktopProviderConnectivityResult[];
    return { ok: true, results };
  }
  if (hasBridge()) {
    return demProtocolError("probe_provider_connectivity");
  }
  // Fixture/demo backend cannot reach providers; report every listed
  // endpoint as unavailable instead of pretending connectivity. The ids and
  // sites mirror the backend's fixed probe targets.
  const FALLBACK_PROBE_ENDPOINTS: { id: string; site: string }[] = [
    { id: "cdse", site: "https://dataspace.copernicus.eu/" },
    { id: "earthdata", site: "https://search.asf.alaska.edu/" },
    { id: "bhoonidhi", site: "https://bhoonidhi.nrsc.gov.in/" },
    { id: "usgs_m2m", site: "https://earthexplorer.usgs.gov/" },
    { id: "mpc", site: "https://planetarycomputer.microsoft.com/" },
    { id: "opentopography", site: "https://portal.opentopography.org/" },
    { id: "gee", site: "https://earthengine.google.com/" },
    { id: "gacos", site: "http://www.gacos.net/" },
    { id: "comet_licsar", site: "https://comet.nerc.ac.uk/COMET-LiCS-portal/" },
  ];
  return {
    ok: true,
    results: FALLBACK_PROBE_ENDPOINTS.map((endpoint) => ({
      id: endpoint.id,
      site: endpoint.site,
      ok: false,
      latency_ms: 0,
      http_status: null,
      error_code: "transport_unavailable",
    })),
  };
}


export async function pickOpenFile(
  title = "",
  filters: string[] = [],
): Promise<PickResult> {
  return getDesktopShellBridge().dialog.pickOpenFile(title, filters);
}

export async function pickDirectory(title = ""): Promise<PickResult> {
  return getDesktopShellBridge().dialog.pickDirectory(title);
}

export async function savePublicPreview(
  url: string,
  outputDir: string,
  productId: string,
): Promise<PublicPreviewSaveResult> {
  if (isTauriDesktopHost()) {
    const route = getDesktopShellBridge().application.savePreview;
    if (!route) return tauriLegacyUnavailable("save_public_preview");
    const result = await route({
      data_url: url,
      output_dir: outputDir.trim(),
      file_name: productId.trim(),
    });
    return result.ok ? result : applicationError(result);
  }
  if (hasBridge() && typeof api().save_public_preview === "function") {
    return api().save_public_preview!(url, outputDir, productId);
  }
  return { ok: false, error: "桌面 Preview 保存服务不可用", code: "GUI003" };
}

export async function ensureDirectory(path: string): Promise<SimpleOk> {
  if (isTauriDesktopHost()) {
    const trimmed = path.trim();
    if (!trimmed) return { ok: false, error: "目录路径不能为空", code: "GUI003" };
    const result = await getDesktopShellBridge().application.workspacePrepareLocation({
      request_id: `workspace_prepare_location_${Date.now()}`,
      path: trimmed,
    });
    return result.ok ? { ok: true, path: result.path } : applicationError(result);
  }
  if (hasBridge()) return api().ensure_directory(path);
  const trimmed = path.trim();
  if (!trimmed) return { ok: false, error: "目录路径不能为空", code: "GUI003" };
  return { ok: true, path: trimmed };
}

export async function openExternalUrl(url: string): Promise<SimpleOk> {
  const approved = approvedExternalUrl(url);
  if (!approved) {
    return { ok: false, error: "仅允许打开 HTTP 或 HTTPS 外部链接", code: "GUI003" };
  }
  if (isTauriDesktopHost()) {
    const route = getDesktopShellBridge().application.openExternalUrl;
    if (!route) return demProtocolError("open_external_url");
    return route(approved);
  }
  if (hasBridge()) return api().open_external_url(approved);
  if (typeof window !== "undefined") window.open(approved, "_blank", "noopener,noreferrer");
  return { ok: true, url: approved };
}

export async function openPath(path: string): Promise<SimpleOk> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("open_path");
  if (hasBridge()) return api().open_path(path);
  return { ok: true, path };
}

export async function minimizeNativeWindow(): Promise<SimpleOk> {
  return getDesktopShellBridge().window.minimize();
}

export async function toggleNativeWindowMaximize(): Promise<SimpleOk> {
  return getDesktopShellBridge().window.toggleMaximize();
}

export async function startNativeWindowDrag(): Promise<SimpleOk> {
  const drag = getDesktopShellBridge().window.startDragging;
  if (typeof drag === "function") {
    return drag();
  }
  return { ok: true };
}

export async function closeNativeWindow(): Promise<SimpleOk> {
  return getDesktopShellBridge().window.close();
}

export async function getNativeWindowSize(): Promise<NativeWindowSize> {
  return getDesktopShellBridge().window.getSize();
}

export async function resizeNativeWindowFromEdge(
  edge: string,
  startWidth: number,
  startHeight: number,
  deltaX: number,
  deltaY: number,
): Promise<SimpleOk> {
  return getDesktopShellBridge().window.resizeFromEdge(
    edge,
    startWidth,
    startHeight,
    deltaX,
    deltaY,
  );
}

// ------------------------------------------------------------ workspace tree
export async function createWorkspace(
  root: string,
  name?: string,
): Promise<WorkspaceResult> {
  if (isTauriDesktopHost()) {
    const trimmed = root.trim();
    if (!trimmed) return { ok: false, error: "工作区根路径不能为空", code: "GUI003" };
    const result = await getDesktopShellBridge().application.workspaceCreate({
      request_id: `workspace_create_${Date.now()}`,
      workspace_id: stableUiNodeId("workspace", trimmed),
      workspace_root: trimmed,
      workspace_name: name?.trim() || trimmed.split(/[\\/]/).filter(Boolean).pop() || "InSAR workspace",
    });
    if (result.ok) notifyContextChanged();
    return contextWorkspaceResult(result);
  }
  if (hasBridge()) {
    const res = await api().create_workspace(root, name ?? null);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const trimmed = root.trim();
  if (!trimmed) return { ok: false, error: "工作区根路径不能为空", code: "GUI003" };
  if (mock.workspace?.root === trimmed) {
    if (name) mock.workspace = { ...mock.workspace, name };
    notifyContextChanged();
    return { ok: true, workspace_id: mock.workspace.workspace_id, root: trimmed, projects: [] };
  }
  mock.workspace = { workspace_id: "ws_preview", root: trimmed, name: name ?? "" };
  mock.project = null;
  mock.region = null;
  notifyContextChanged();
  return { ok: true, workspace_id: "ws_preview", root: trimmed, projects: [] };
}

export async function addProject(name: string): Promise<ProjectResult> {
  if (isTauriDesktopHost()) {
    const context = await currentTauriWorkspaceProjection();
    if (!context.ok || !context.workspace_id) return context.ok ? { ok: false, error: "工作区上下文缺少稳定 ID", code: "V3_APPLICATION_INVALID_SNAPSHOT" } : context;
    const result = await getDesktopShellBridge().application.projectCreate({
      request_id: `project_create_${Date.now()}`,
      parent_id: context.workspace_id,
      expected_revision: context.revision,
      name,
    });
    if (result.ok) notifyContextChanged();
    return contextProjectResult(result);
  }
  if (hasBridge()) {
    const res = await api().add_project(name);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const safe = mockSafeName(name);
  if (!safe) {
    return { ok: false, error: "项目名至少需包含一个字母、数字或下划线", code: "GUI003" };
  }
  const root = `${mock.workspace?.root ?? "C:\\InSAR"}\\${safe}`;
  mock.project = { project_id: `proj_${safe}`, name, safe_name: safe, root };
  mock.region = null;
  notifyContextChanged();
  return { ok: true, project_id: `proj_${safe}`, name, safe_name: safe };
}

export async function selectProject(projectId: string): Promise<ProjectResult> {
  if (isTauriDesktopHost()) {
    const context = await currentTauriWorkspaceProjection();
    if (!context.ok || !context.workspace_id) return context.ok ? { ok: false, error: "工作区上下文缺少稳定 ID", code: "V3_APPLICATION_INVALID_SNAPSHOT" } : context;
    const result = await getDesktopShellBridge().application.projectSelect({
      request_id: `project_select_${Date.now()}`,
      parent_id: context.workspace_id,
      expected_revision: context.revision,
      node_id: projectId,
    });
    if (result.ok) notifyContextChanged();
    return contextProjectResult(result);
  }
  if (hasBridge()) {
    const res = await api().select_project(projectId);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const safe = projectId.replace(/^proj_/, "");
  mock.project = {
    project_id: projectId,
    name: safe,
    safe_name: safe,
    root: `${mock.workspace?.root ?? "C:\\InSAR"}\\${safe}`,
  };
  notifyContextChanged();
  return { ok: true, project_id: projectId, name: safe, safe_name: safe };
}

export async function addRegion(name: string): Promise<RegionResult> {
  if (isTauriDesktopHost()) {
    const context = await currentTauriWorkspaceProjection();
    if (!context.ok || !context.project_id) return context.ok ? { ok: false, error: "请先选择项目", code: "V3_APPLICATION_INVALID_REQUEST" } : context;
    const result = await getDesktopShellBridge().application.regionCreate({
      request_id: `region_create_${Date.now()}`,
      parent_id: context.project_id,
      expected_revision: context.revision,
      name,
    });
    if (result.ok) {
      notifyContextChanged();
      markUiSmoke("region_created_or_selected", true);
    }
    return contextRegionResult(result);
  }
  if (hasBridge()) {
    const res = await api().add_region(name);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const safe = mockSafeName(name);
  if (!safe) {
    return { ok: false, error: "区域名至少需包含一个字母、数字或下划线", code: "GUI003" };
  }
  mock.region = {
    region_id: `region_${safe}`,
    name,
    safe_name: safe,
    root: `${mock.project?.root ?? mock.workspace?.root ?? "C:\\InSAR"}\\${safe}`,
    has_aoi: false,
    bbox: null,
    scene_count: 0,
  };
  notifyContextChanged();
  markUiSmoke("region_created_or_selected", true);
  return { ok: true, region_id: `region_${safe}`, name, safe_name: safe };
}

export async function selectRegion(regionId: string): Promise<RegionResult> {
  if (isTauriDesktopHost()) {
    const context = await currentTauriWorkspaceProjection();
    if (!context.ok || !context.project_id) return context.ok ? { ok: false, error: "请先选择项目", code: "V3_APPLICATION_INVALID_REQUEST" } : context;
    const result = await getDesktopShellBridge().application.regionSelect({
      request_id: `region_select_${Date.now()}`,
      parent_id: context.project_id,
      expected_revision: context.revision,
      node_id: regionId,
    });
    if (result.ok) {
      notifyContextChanged();
      markUiSmoke("region_created_or_selected", true);
    }
    return contextRegionResult(result);
  }
  if (hasBridge()) {
    const res = await api().select_region(regionId);
    if (res.ok) notifyContextChanged();
    return res;
  }
  if (!mock.region || mock.region.region_id !== regionId) {
    return { ok: false, error: "未知区域", code: "GUI002" };
  }
  notifyContextChanged();
  markUiSmoke('region_created_or_selected', true);
  return {
    ok: true,
    region_id: mock.region.region_id,
    name: mock.region.name,
    safe_name: mock.region.safe_name,
  };
}

// --------------------------------------------------------------------- AOI
async function currentTauriRegionId(): Promise<string | ApiError> {
  const context = await getContext();
  if (!context.ok) return context;
  const regionId = context.region?.region_id;
  return regionId
    ? regionId
    : { ok: false, error: "请先打开工作区并选择稳定研究区", code: "V3_APPLICATION_INVALID_REQUEST" };
}

let mockWorkingAoi: WorkingAoiOk | null = null;

export async function setWorkingAoi(
  geometry: Json,
  displayName = "工作范围",
  sourceKind = "manual_polygon",
): Promise<WorkingAoiResult> {
  const bbox = bboxFromGeojson(geometry);
  if (isTauriDesktopHost()) {
    const setter = getDesktopShellBridge().application.setWorkingAoi;
    if (typeof setter !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露工作 AOI 命令", code: "GUI003" };
    }
    const request: DesktopSetWorkingAoiRequest = {
      source_kind: sourceKind,
      geometry: (recordValue(geometry) ?? {}) as Record<string, unknown>,
      bbox: [bbox.west, bbox.south, bbox.east, bbox.north],
      crs: "EPSG:4326",
      display_name: displayName,
    };
    const value: DesktopWorkingAoi | ApiError = await setter(request);
    if ("ok" in value) {
      return value;
    }
    const working = {
      ok: true as const,
      state: "working" as const,
      source_kind: value.source_kind,
      geometry: value.geometry,
      bbox: value.bbox,
      crs: value.crs,
      display_name: value.display_name,
    };
    notifyContextChanged();
    return working;
  }
  if (hasBridge()) {
    return {
      ok: false,
      error: "当前正式桌面路由未提供 working AOI 命令",
      code: "GUI003",
    };
  }
  mockWorkingAoi = {
    ok: true,
    state: "working",
    source_kind: sourceKind,
    geometry,
    bbox: [bbox.west, bbox.south, bbox.east, bbox.north],
    crs: "EPSG:4326",
    display_name: displayName,
  };
  notifyContextChanged();
  return mockWorkingAoi;
}

export async function clearWorkingAoi(): Promise<{ ok: true } | ApiError> {
  if (isTauriDesktopHost()) {
    const clearer = getDesktopShellBridge().application.clearWorkingAoi;
    if (typeof clearer !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露工作 AOI 清除命令", code: "GUI003" };
    }
    const value = await clearer();
    if (value.ok) notifyContextChanged();
    return value;
  }
  if (hasBridge()) {
    return { ok: false, error: "当前正式桌面路由未提供 working AOI 命令", code: "GUI003" };
  }
  mockWorkingAoi = null;
  notifyContextChanged();
  return { ok: true };
}

export async function getWorkingAoi(): Promise<WorkingAoiResult | { ok: true; state: "no_aoi" }> {
  if (isTauriDesktopHost()) {
    const getter = getDesktopShellBridge().application.getWorkingAoi;
    if (typeof getter !== "function") {
      return { ok: true, state: "no_aoi" };
    }
    const value = await getter();
    if (value === null) return { ok: true, state: "no_aoi" };
    if ("ok" in value) return value;
    return {
      ok: true,
      state: "working",
      source_kind: value.source_kind,
      geometry: value.geometry,
      bbox: value.bbox,
      crs: value.crs,
      display_name: value.display_name,
    };
  }
  if (mockWorkingAoi) return mockWorkingAoi;
  return { ok: true, state: "no_aoi" };
}

type ExactOperationAoiResult =
  | { ok: true; geometry: Json; display_name: string; bbox: Bbox }
  | ApiError;

function exactOperationBbox(value: unknown): Bbox | null {
  const record = recordValue(value);
  const candidate = Array.isArray(value)
    ? { west: value[0], south: value[1], east: value[2], north: value[3], crs: undefined }
    : record;
  if (!candidate) return null;
  const bbox = {
    west: Number(candidate.west),
    east: Number(candidate.east),
    south: Number(candidate.south),
    north: Number(candidate.north),
    crs: typeof candidate.crs === "string" ? candidate.crs : "EPSG:4326",
  };
  return [bbox.west, bbox.east, bbox.south, bbox.north].every(Number.isFinite)
    && bbox.west < bbox.east
    && bbox.south < bbox.north
    ? bbox
    : null;
}

async function exactOperationAoi(context?: Context): Promise<ExactOperationAoiResult> {
  const resolvedContext = context ?? await getContext();
  if (!resolvedContext.ok) return resolvedContext;
  if (resolvedContext.region) {
    const geometry = resolvedContext.region.aoi_geojson;
    const displayName = resolvedContext.region.aoi_name;
    const bbox = exactOperationBbox(resolvedContext.region.bbox);
    if (!resolvedContext.region.has_aoi || !recordValue(geometry) || !displayName?.trim() || !bbox) {
      return {
        ok: false,
        error: "当前研究区没有可用于处理的持久化 AOI",
        code: "V3_APPLICATION_INVALID_REQUEST",
      };
    }
    return { ok: true, geometry: geometry as Json, display_name: displayName, bbox };
  }
  const working = await getWorkingAoi();
  if (!working.ok) return working;
  if (working.state !== "working") {
    return {
      ok: false,
      error: "请先设置并持久化当前 AOI",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  const bbox = exactOperationBbox(working.bbox);
  if (!bbox) {
    return {
      ok: false,
      error: "当前持久化 AOI 的 bbox 无效",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  return {
    ok: true,
    geometry: working.geometry,
    display_name: working.display_name,
    bbox,
  };
}

type TauriWorkspaceProjection = {
  ok: true;
  workspace_id: string;
  project_id: string | null;
  region_id: string | null;
  revision: number;
} | ApiError;

async function currentTauriWorkspaceProjection(): Promise<TauriWorkspaceProjection> {
  const snapshot = await restoreApplicationCore(false);
  if (!snapshot.ok) return snapshot;
  const context = recordValue(snapshot.core.workspace_context);
  const revision = typeof context?.revision === "number" && Number.isSafeInteger(context.revision)
    ? context.revision
    : null;
  if (!context || typeof context.workspace_id !== "string" || revision === null || revision < 0) {
    return {
      ok: false,
      error: "Tauri application service returned an invalid workspace projection",
      code: "V3_APPLICATION_INVALID_SNAPSHOT",
    };
  }
  return {
    ok: true,
    workspace_id: context.workspace_id,
    project_id: typeof context.project_id === "string" ? context.project_id : null,
    region_id: typeof context.region_id === "string" ? context.region_id : null,
    revision,
  };
}

export async function setRegionAoiBbox(
  west: number,
  east: number,
  south: number,
  north: number,
  displayName = "矩形选区",
): Promise<AoiResult> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    const regionId = await currentTauriRegionId();
    if (typeof regionId !== "string") return regionId;
    const bbox: Bbox = { west, east, south, north, crs: "EPSG:4326" };
    const geometry = {
      type: "Polygon",
      coordinates: [
        [
          [west, south],
          [east, south],
          [east, north],
          [west, north],
          [west, south],
        ],
      ],
    };
    const requestId = `bind_aoi_bbox_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
    const snapshot = await bridge.application.bindRegionAoi({
      request_id: requestId,
      region_id: regionId,
      source_kind: "manual_bbox",
      geometry,
      bbox: [west, south, east, north],
      crs: "EPSG:4326",
      display_name: `${displayName} (${west.toFixed(2)}, ${south.toFixed(2)})`,
      bound_at: new Date().toISOString(),
    });
    if (snapshot.ok) {
      notifyContextChanged();
      markUiSmoke("aoi_bind_command_completed", true);
      const restoredAoi = restoredRegionAoi(snapshot, regionId);
      markUiSmoke("aoi_persisted_in_snapshot", restoredAoi !== null);
      return {
        ok: true,
        aoi: { source: "MANUAL_BBOX", role: "PROCESSING_AOI", bbox },
        aoi_geojson: geometry as Json,
        region_id: typeof restoredAoi?.region_id === "string" ? restoredAoi.region_id : regionId,
        region_name: typeof restoredAoi?.display_name === "string" ? restoredAoi.display_name : displayName,
      };
    }
    return { ok: false, error: snapshot.error, code: snapshot.code ?? null };
  }
  if (hasBridge()) {
    const res = await api().set_region_aoi_bbox(west, east, south, north);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const region = ensureMockRegion();
  if (!(west < east)) {
    return { ok: false, error: "west must be strictly less than east", code: "AOI001" };
  }
  if (!(south < north)) {
    return { ok: false, error: "south must be strictly less than north", code: "AOI001" };
  }
  const bbox: Bbox = { west, east, south, north, crs: "EPSG:4326" };
  mock.region = { ...region, has_aoi: true, bbox, aoi_geojson: null };
  notifyContextChanged();
  return {
    ok: true,
    aoi: { source: "MANUAL_BBOX", role: "PROCESSING_AOI", bbox },
    region_id: mock.region.region_id,
    region_name: mock.region.name,
  };
}

export async function setRegionAoiFile(path: string): Promise<AoiResult> {
  if (isTauriDesktopHost()) {
    const preview = await previewAoiFile(path);
    if (!preview.ok) return { ok: false, error: preview.error, code: preview.code };
    if (!preview.geojson) {
      return { ok: false, error: "桌面 Worker 未返回可绑定的 AOI 几何", code: "V3_APPLICATION_INVALID_SNAPSHOT" };
    }
    const featureIds = preview.features ? preview.features.map((f) => f.id) : [];
    return setRegionAoiGeojsonFeatures(preview.geojson, featureIds, preview.display_field || "name");
  }
  if (hasBridge()) {
    const res = await api().set_region_aoi_file(path);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const region = ensureMockRegion();
  if (!path.trim()) return { ok: false, error: "请提供矢量文件路径", code: "AOI001" };
  mock.region = { ...region, has_aoi: true, bbox: MOCK_BBOX, aoi_geojson: null };
  notifyContextChanged();
  return {
    ok: true,
    aoi: { source: "VECTOR_FILE", role: "PROCESSING_AOI", bbox: MOCK_BBOX, geometry_path: path },
    aoi_geojson: polygonFromBbox(MOCK_BBOX),
    aoi_feature_count: 1,
    region_id: mock.region.region_id,
    region_name: mock.region.name,
  };
}

export async function previewAoiFile(path: string): Promise<AoiPreviewResult> {
  if (isTauriDesktopHost()) {
    const runner = getDesktopShellBridge().application.aoiImport;
    if (typeof runner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露 AOI 导入接口", code: "GUI003" };
    }
    const value = recordValue(await runner({ source: "path", path }));
    if (value?.ok === true && Array.isArray(value.features) && typeof value.total_features === "number") {
      return value as unknown as AoiPreviewResult;
    }
    return {
      ok: false,
      error: String(value?.error ?? "ApplicationService 未返回有效 AOI 预览"),
      code: typeof value?.code === "string" ? value.code : "AOI_PREVIEW_INVALID",
    };
  }
  if (hasBridge()) return api().preview_aoi_file(path);
  if (!path.trim()) return { ok: false, error: "请提供矢量文件路径", code: "AOI001" };
  return {
    ok: true,
    path,
    file_name: path.split(/[\\/]/).pop() || "boundary.geojson",
    total_features: 3,
    fields: ["name", "type"],
    display_field: "name",
    features: [
      { id: "0", index: 1, name: "示例边界 A", area_km2: 120.5, bbox: MOCK_BBOX, properties: { name: "示例边界 A", type: "county" } },
      { id: "1", index: 2, name: "示例边界 B", area_km2: 86.2, bbox: MOCK_BBOX, properties: { name: "示例边界 B", type: "county" } },
      { id: "2", index: 3, name: "示例边界 C", area_km2: 214.8, bbox: MOCK_BBOX, properties: { name: "示例边界 C", type: "county" } },
    ],
  };
}

export async function setWorkingAoiFile(path: string): Promise<WorkingAoiResult> {
  const preview = await previewAoiFile(path);
  if (!preview.ok) return preview;
  if (!preview.geojson) {
    return { ok: false, error: "文件中没有可用的面几何", code: "AOI_PREVIEW_INVALID" };
  }
  return setWorkingAoi(preview.geojson, preview.file_name || "文件工作范围", "uploaded_geometry");
}

export async function setWorkingAoiGeojsonFeatures(
  geojson: Json,
  featureIds: string[],
  nameField = "",
  downloadMode: "merge" | "split" = "merge",
  displayName = "矢量文件选区",
): Promise<WorkingAoiResult> {
  if (isTauriDesktopHost()) {
    const normalizer = getDesktopShellBridge().application.aoiNormalize;
    if (typeof normalizer !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露 AOI 归一化接口", code: "GUI003" };
    }
    const record = recordValue(await normalizer({
      geojson,
      feature_ids: featureIds,
      name_field: nameField,
      download_mode: downloadMode,
    }));
    if (!record) {
      return { ok: false, error: "ApplicationService 未返回有效 AOI 归一化结果", code: "AOI_NORMALIZE_INVALID" };
    }
    if (record.ok !== true) {
      return {
        ok: false,
        error: String(record.error ?? "AOI 要素归一化失败"),
        code: typeof record.code === "string" ? record.code : "AOI_NORMALIZE_INVALID",
      };
    }
    const normalizedGeojson = (record.geojson ?? record.aoi_geojson ?? geojson) as Json;
    return setWorkingAoi(normalizedGeojson, displayName, "uploaded_geometry");
  }

  return setWorkingAoi(geojson, displayName, "uploaded_geometry");
}

export async function setWorkingAoiFileFeatures(
  path: string,
  featureIds: string[],
  nameField = "",
  downloadMode: "merge" | "split" = "merge",
  displayName = "矢量文件选区",
): Promise<WorkingAoiResult> {
  const preview = await previewAoiFile(path);
  if (!preview.ok) return preview;
  if (!preview.geojson) {
    return { ok: false, error: "文件中没有可用的面几何", code: "AOI_PREVIEW_INVALID" };
  }
  return setWorkingAoiGeojsonFeatures(
    preview.geojson,
    featureIds,
    nameField || preview.display_field || "name",
    downloadMode,
    displayName,
  );
}

export async function previewAoiFileContent(fileName: string, text: string): Promise<AoiPreviewResult> {
  if (isTauriDesktopHost()) {
    const runner = getDesktopShellBridge().application.aoiImport;
    if (typeof runner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露 AOI 导入接口", code: "GUI003" };
    }
    const value = recordValue(await runner({ source: "text", file_name: fileName, text }));
    if (value?.ok === true && Array.isArray(value.features) && typeof value.total_features === "number") {
      return value as unknown as AoiPreviewResult;
    }
    return {
      ok: false,
      error: String(value?.error ?? "ApplicationService 未返回有效 AOI 预览"),
      code: typeof value?.code === "string" ? value.code : "AOI_PREVIEW_INVALID",
    };
  }
  if (hasBridge()) {
    const previewer = api().preview_aoi_file_content;
    if (!previewer) return { ok: false, error: "当前版本不支持拖拽读取文件内容，请使用上传本地边界按钮。", code: "AOI001" };
    return previewer(fileName, text);
  }
  return previewAoiFile(fileName);
}

export async function previewAoiFileBytes(fileName: string, base64Data: string): Promise<AoiPreviewResult> {
  if (isTauriDesktopHost()) {
    const runner = getDesktopShellBridge().application.aoiImport;
    if (typeof runner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露 AOI 导入接口", code: "GUI003" };
    }
    const value = recordValue(await runner({ source: "bytes", file_name: fileName, base64_data: base64Data }));
    if (value?.ok === true && Array.isArray(value.features) && typeof value.total_features === "number") {
      return value as unknown as AoiPreviewResult;
    }
    return {
      ok: false,
      error: String(value?.error ?? "ApplicationService 未返回有效 AOI 预览"),
      code: typeof value?.code === "string" ? value.code : "AOI_PREVIEW_INVALID",
    };
  }
  if (hasBridge()) {
    const previewer = api().preview_aoi_file_bytes;
    if (!previewer) return { ok: false, error: "当前版本不支持拖拽读取该格式，请使用上传本地边界按钮。", code: "AOI001" };
    return previewer(fileName, base64Data);
  }
  return previewAoiFile(fileName);
}

export async function setRegionAoiFileFeatures(
  path: string,
  featureIds: string[],
  nameField = "",
  downloadMode: "merge" | "split" = "merge",
): Promise<AoiResult> {
  if (isTauriDesktopHost()) {
    const preview = await previewAoiFile(path);
    if (!preview.ok) return { ok: false, error: preview.error, code: preview.code };
    if (!preview.geojson) {
      return { ok: false, error: "桌面 Worker 未返回可绑定的 AOI 几何", code: "V3_APPLICATION_INVALID_SNAPSHOT" };
    }
    return setRegionAoiGeojsonFeatures(preview.geojson, featureIds, nameField || preview.display_field || "name", downloadMode);
  }
  if (hasBridge()) {
    const res = await api().set_region_aoi_file_features(path, featureIds, nameField, downloadMode);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const region = ensureMockRegion();
  mock.region = { ...region, has_aoi: true, bbox: MOCK_BBOX, aoi_geojson: polygonFromBbox(MOCK_BBOX) };
  notifyContextChanged();
  return {
    ok: true,
    aoi: { source: "VECTOR_FILE", role: "PROCESSING_AOI", bbox: MOCK_BBOX, geometry_path: path },
    aoi_geojson: polygonFromBbox(MOCK_BBOX),
    aoi_feature_count: featureIds.length || 3,
    aoi_total_feature_count: 3,
    download_mode: downloadMode,
    region_id: mock.region.region_id,
    region_name: mock.region.name,
  };
}

export async function setRegionAoiGeojson(
  geojson: Json,
  displayName = "多边形选区",
  sourceKind: "manual_polygon" | "uploaded_geometry" | "administrative_boundary" = "manual_polygon",
): Promise<AoiResult> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    const regionId = await currentTauriRegionId();
    if (typeof regionId !== "string") return regionId;
    const bbox = bboxFromGeojson(geojson);
    const geometryObj =
      geojson && typeof geojson === "object" && (geojson as Record<string, unknown>).type === "Feature"
        ? ((geojson as Record<string, unknown>).geometry as Record<string, unknown>)
        : (geojson as Record<string, unknown>);
    const requestId = `bind_aoi_geojson_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
    const snapshot = await bridge.application.bindRegionAoi({
      request_id: requestId,
      region_id: regionId,
      source_kind: sourceKind,
      geometry: geometryObj,
      bbox: [bbox.west, bbox.south, bbox.east, bbox.north],
      crs: "EPSG:4326",
      display_name: displayName,
      bound_at: new Date().toISOString(),
    });
    if (snapshot.ok) {
      notifyContextChanged();
      markUiSmoke("aoi_bind_command_completed", true);
      const restoredAoi = restoredRegionAoi(snapshot, regionId);
      markUiSmoke("aoi_persisted_in_snapshot", restoredAoi !== null);
      return {
        ok: true,
        aoi: { source: "MANUAL_BBOX", role: "PROCESSING_AOI", bbox },
        aoi_geojson: geojson,
        region_id: typeof restoredAoi?.region_id === "string" ? restoredAoi.region_id : regionId,
        region_name: typeof restoredAoi?.display_name === "string" ? restoredAoi.display_name : displayName,
      };
    }
    return { ok: false, error: snapshot.error, code: snapshot.code ?? null };
  }
  if (hasBridge()) {
    const res = await api().set_region_aoi_geojson(geojson);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const region = ensureMockRegion();
  const bbox = bboxFromGeojson(geojson);
  mock.region = { ...region, has_aoi: true, bbox, aoi_geojson: geojson };
  notifyContextChanged();
  return {
    ok: true,
    aoi: { source: "MANUAL_BBOX", role: "PROCESSING_AOI", bbox },
    aoi_geojson: geojson,
    region_id: mock.region.region_id,
    region_name: mock.region.name,
  };
}

export async function setRegionAoiGeojsonFeatures(
  geojson: Json,
  featureIds: string[],
  nameField = "",
  downloadMode: "merge" | "split" = "merge",
): Promise<AoiResult> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    const regionId = await currentTauriRegionId();
    if (typeof regionId !== "string") return regionId;
    const normalizer = bridge.application.aoiNormalize;
    if (typeof normalizer !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露 AOI 归一化接口", code: "GUI003" };
    }
    const record = recordValue(await normalizer({
      geojson,
      feature_ids: featureIds,
      name_field: nameField,
      download_mode: downloadMode,
    }));
    if (!record) {
      return { ok: false, error: "ApplicationService 未返回有效 AOI 归一化结果", code: "AOI_NORMALIZE_INVALID" };
    }
    if (record.ok !== true) {
      return { ok: false, error: String(record.error ?? "set_region_aoi_geojson_features failed"), code: record.code != null ? String(record.code) : null };
    }
    const normalizedGeojson = (record.geojson ?? record.aoi_geojson ?? geojson) as Json;
    const bbox = (record.bbox as Bbox | undefined) ?? bboxFromGeojson(normalizedGeojson);
    const geometryObj =
      (record.geometry as Record<string, unknown> | undefined) ??
      (normalizedGeojson && typeof normalizedGeojson === "object" && (normalizedGeojson as Record<string, unknown>).type === "Feature"
        ? ((normalizedGeojson as Record<string, unknown>).geometry as Record<string, unknown>)
        : (normalizedGeojson as Record<string, unknown>));

    const requestId = `bind_aoi_features_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
    const snapshot = await bridge.application.bindRegionAoi({
      request_id: requestId,
      region_id: regionId,
      source_kind: "uploaded_geometry",
      geometry: geometryObj,
      bbox: [bbox.west, bbox.south, bbox.east, bbox.north],
      crs: "EPSG:4326",
      display_name: (record.region_name as string) || "矢量文件选区",
      bound_at: new Date().toISOString(),
    });
    if (snapshot.ok) {
      notifyContextChanged();
      markUiSmoke("aoi_bind_command_completed", true);
      const restoredAoi = restoredRegionAoi(snapshot, regionId);
      markUiSmoke("aoi_persisted_in_snapshot", restoredAoi !== null);
      return {
        ok: true,
        aoi: { source: "VECTOR_FILE", role: "PROCESSING_AOI", bbox },
        aoi_geojson: normalizedGeojson,
        aoi_feature_count: (typeof record.feature_count === "number"
          ? record.feature_count
          : typeof record.aoi_feature_count === "number"
            ? record.aoi_feature_count
            : featureIds.length || 1),
        download_mode: downloadMode,
        region_id: typeof restoredAoi?.region_id === "string" ? restoredAoi.region_id : regionId,
        region_name: typeof restoredAoi?.display_name === "string" ? restoredAoi.display_name : "矢量文件选区",
      };
    }
    return { ok: false, error: snapshot.error, code: snapshot.code ?? null };
  }
  if (hasBridge()) {
    const binder = api().set_region_aoi_geojson_features;
    if (!binder) return setRegionAoiGeojson(geojson);
    const res = await binder(geojson, featureIds, nameField, downloadMode);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const region = ensureMockRegion();
  const bbox = bboxFromGeojson(geojson);
  mock.region = { ...region, has_aoi: true, bbox, aoi_geojson: geojson };
  notifyContextChanged();
  return {
    ok: true,
    aoi: { source: "VECTOR_FILE", role: "PROCESSING_AOI", bbox },
    aoi_geojson: geojson,
    aoi_feature_count: featureIds.length || 1,
    download_mode: downloadMode,
    region_id: mock.region.region_id,
    region_name: mock.region.name,
  };
}

export async function searchAdminBoundaries(
  query = "",
  province = "",
  city = "",
  district = "",
  limit = 8,
  providerObjectId: string | null = null,
): Promise<AdminBoundaryResult> {
  if (isTauriDesktopHost()) {
    const runner = getDesktopShellBridge().application.adminBoundarySearch;
    if (typeof runner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露行政区搜索接口", code: "GUI003" };
    }
    const exactProviderObjectId = providerObjectId ?? null;
    if (
      exactProviderObjectId !== null
      && (
        exactProviderObjectId.trim() !== exactProviderObjectId
        || (!/^\d{6}$/.test(exactProviderObjectId) && !/^relation:[1-9]\d*$/.test(exactProviderObjectId))
      )
    ) {
      return { ok: false, error: "行政区身份无效，请重新选择行政区", code: "BOUNDARY_PROVIDER_IDENTITY_INVALID" };
    }
    const request: DesktopAdminBoundarySearchRequest = {
      query,
      province,
      city,
      district,
      provider_object_id: exactProviderObjectId,
      limit,
    };
    return normalizeAdminBoundaryResult(await runner(request), "application-service");
  }
  if (hasBridge()) {
    return normalizeAdminBoundaryResult(
      await api().search_admin_boundaries(query, province, city, district, limit),
      "legacy",
    );
  }
  // The browser fixture intentionally has no administrative geometry source.
  // The bundled China index is options-only; manufacturing a Polygon from its
  // bbox would make a rectangle look like an authoritative boundary.
  return {
    ok: false,
    error: "浏览器预览不提供行政区真实边界，请运行正式 Tauri 验证软件后通过 DataV/Nominatim 检索。",
    code: "ADMIN_BOUNDARY_DESKTOP_REQUIRED",
  };
}

export async function resolveAdminBoundary(boundary: AdminBoundary): Promise<AdminBoundary | ApiError> {
  const existingGeometry = recordValue(boundary.geometry ?? boundary.geojson);
  if (existingGeometry) return boundary;
  if (!isTauriDesktopHost()) {
    return { ok: false, error: "当前行政边界候选没有可解析的真实几何", code: "BOUNDARY_PROVIDER_IDENTITY_INVALID" };
  }
  const provider = boundary.provider?.trim() ?? "";
  const providerObjectId = boundary.provider_object_id?.trim() ?? "";
  if (!provider || !providerObjectId) {
    return { ok: false, error: "行政边界候选缺少稳定 Provider 身份", code: "BOUNDARY_PROVIDER_IDENTITY_INVALID" };
  }
  const runner = getDesktopShellBridge().application.adminBoundaryResolve;
  if (typeof runner !== "function") {
    return { ok: false, error: "当前桌面 API 未暴露行政边界解析接口", code: "GUI003" };
  }
  const request: DesktopAdminBoundaryResolveRequest = {
    provider,
    provider_object_id: providerObjectId,
  };
  const response = await runner(request);
  if (recordValue(response)?.ok === false) {
    return applicationError(response);
  }
  const resolved = normalizeResolvedAdminBoundary(response, provider);
  if (!resolved || resolved.provider_object_id !== providerObjectId || resolved.provider !== provider) {
    return { ok: false, error: "行政边界解析结果与候选身份不一致", code: "BOUNDARY_PROVIDER_RESPONSE_INVALID" };
  }
  return resolved;
}

function adminOptionArray(
  value: unknown,
  allowLegacyNames: boolean,
  requireDataVIdentity: boolean,
): AdminOption[] | null {
  if (!Array.isArray(value)) return null;
  const options: AdminOption[] = [];
  for (const item of value) {
    if (allowLegacyNames && typeof item === "string") {
      const name = item.trim();
      if (!name) return null;
      options.push({ name, provider: null, provider_object_id: null });
      continue;
    }
    const record = recordValue(item);
    const name = typeof record?.name === "string" ? record.name.trim() : "";
    if (!name) return null;
    const rawProvider = record?.provider;
    const provider = rawProvider == null
      ? null
      : typeof rawProvider === "string" && ["datav", "nominatim"].includes(rawProvider)
        ? rawProvider
        : undefined;
    if (provider === undefined) return null;
    const rawIdentity = record?.provider_object_id;
    const providerObjectId = rawIdentity == null
      ? null
      : typeof rawIdentity === "string" && rawIdentity.trim() === rawIdentity
        && (
          ((provider ?? (requireDataVIdentity ? "datav" : null)) === "datav" && /^\d{6}$/.test(rawIdentity))
          || (provider === "nominatim" && /^relation:[1-9]\d*$/.test(rawIdentity))
        )
          ? rawIdentity
          : undefined;
    if (providerObjectId === undefined) return null;
    if (requireDataVIdentity && !["全部", "不限"].includes(name) && providerObjectId === null) {
      return null;
    }
    options.push({
      name,
      provider: provider ?? (providerObjectId && requireDataVIdentity ? "datav" : null),
      provider_object_id: providerObjectId,
    });
  }
  return options;
}

export async function getAdminOptions(province = "", city = ""): Promise<AdminOptionsResult> {
  if (isTauriDesktopHost()) {
    const runner = getDesktopShellBridge().application.adminOptions;
    if (typeof runner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露行政区选项接口", code: "GUI003" };
    }
    const request: DesktopAdminBoundaryOptionsRequest = { province, city };
    const value = recordValue(await runner(request));
    const requireDataVIdentity = value?.provider === "datav";
    const provinces = adminOptionArray(value?.provinces, false, requireDataVIdentity);
    const cities = adminOptionArray(value?.cities, false, requireDataVIdentity);
    const districts = adminOptionArray(value?.districts, false, requireDataVIdentity);
    if (
      value?.ok === true &&
      provinces !== null &&
      cities !== null &&
      districts !== null
    ) {
      return {
        ok: true,
        provider: typeof value.provider === "string" ? value.provider : undefined,
        warning: typeof value.warning === "string" ? value.warning : null,
        provinces,
        cities,
        districts,
      };
    }
    return {
      ok: false,
      error: String(value?.error ?? "ApplicationService 未返回有效行政区选项"),
      code: typeof value?.code === "string" ? value.code : "ADMIN_OPTIONS_INVALID",
    };
  }
  // The legacy/browser adapters retain their explicit compatibility/index
  // behavior outside the normal production Tauri surface.
  if (hasBridge() && !isTauriDesktopHost()) {
    const value = recordValue(await api().get_admin_options(province, city));
    const provinces = adminOptionArray(value?.provinces, true, false);
    const cities = adminOptionArray(value?.cities, true, false);
    const districts = adminOptionArray(value?.districts, true, false);
    if (value?.ok === true && provinces && cities && districts) {
      return {
        ok: true,
        provider: typeof value.provider === "string" ? value.provider : undefined,
        warning: typeof value.warning === "string" ? value.warning : null,
        provinces,
        cities,
        districts,
      };
    }
    return {
      ok: false,
      error: String(value?.error ?? "Legacy 行政区选项无效"),
      code: typeof value?.code === "string" ? value.code : "ADMIN_OPTIONS_INVALID",
    };
  }
  const clean = (value: string) => value.trim();
  const provinceName = isEmptyAdminValue(province) ? "" : clean(province);
  const cityName = isEmptyAdminValue(city) ? "" : clean(city);
  const provinces = ADMIN_INDEX.provinces.map((item) => item[0]);
  const provinceKey =
    provinces.find((item) => item === provinceName) ??
    provinces.find((item) => provinceName && (item.includes(provinceName) || provinceName.includes(item))) ??
    "";
  const cityRows = provinceKey
    ? [...(ADMIN_INDEX.citiesByProvince[provinceKey] ?? [])]
    : Object.values(ADMIN_INDEX.citiesByProvince).flatMap((items) => [...items]);
  const cityNames = Array.from(new Set(cityRows.map((item) => item[0]))).sort();
  const cityKey =
    cityNames.find((item) => item === cityName) ??
    cityNames.find((item) => cityName && (item.includes(cityName) || cityName.includes(item))) ??
    "";
  const districtRows =
    provinceKey && cityKey
      ? [...(ADMIN_INDEX.districtsByCity[`${provinceKey}|${cityKey}`] ?? [])]
      : provinceKey
        ? allPreviewDistricts(provinceKey)
        : Object.values(ADMIN_INDEX.districtsByCity).flatMap((items) => [...items]);
  const districts = Array.from(new Set(districtRows.map((item) => item[0]))).sort();
  return {
    ok: true,
    provinces: provinces.map((name) => ({ name, provider: null, provider_object_id: null })),
    cities: cityNames.map((name) => ({ name, provider: null, provider_object_id: null })),
    districts: districts.map((name) => ({ name, provider: null, provider_object_id: null })),
  };
}

export async function clearRegionAoi(): Promise<SimpleOk> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    const regionId = await currentTauriRegionId();
    if (typeof regionId !== "string") return regionId;
    const requestId = `clear_aoi_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
    const res = await bridge.application.clearRegionAoi({ request_id: requestId, region_id: regionId });
    if (res.ok) {
      notifyContextChanged();
      return { ok: true };
    }
    return { ok: false, error: res.error, code: res.code ?? null };
  }
  if (hasBridge() && api().clear_region_aoi) return api().clear_region_aoi!();
  return { ok: true };
}

// ------------------------------------------------------------------ SCENES
function mockScene(i: number): SceneRow {
  const date = MOCK_SCENE_DATES[i % MOCK_SCENE_DATES.length];
  const footprint = mockFootprintBbox(i);
  return {
    scene_id: `S1A_IW_SLC__1SDV_${date}T223805_${date}T223832_0529${14 + i}_0667A5`,
    platform: "S1A",
    product_type: "SLC",
    beam_mode: "IW",
    polarization: "DV",
    acquisition_datetime: `${date.slice(0, 4)}-${date.slice(4, 6)}-${date.slice(6, 8)}T22:38:05Z`,
    absolute_orbit: 52914 + i * 175,
    relative_orbit: 11,
    path: 11,
    frame: 468,
    orbit_direction: i % 2 === 0 ? "DESCENDING" : "ASCENDING",
    file_size_remote: 7_800_000_000 + i * 210_000_000,
    footprint_bbox: footprint,
    footprint_geojson: polygonFromBbox(footprint),
    has_url: false,
  };
}

export async function importScenesText(text: string): Promise<ScenesResult> {
  if (isTauriDesktopHost()) {
    const result = await importApplicationScenes({ source: "text", text });
    if (result.ok) notifyContextChanged();
    return result;
  }
  if (hasBridge()) {
    const res = await api().import_scenes_text(text);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const lines = text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
  if (!lines.length) {
    return { ok: false, error: "请粘贴至少一个 Sentinel-1 颗粒名或下载链接", code: "ASF001" };
  }
  const valid = lines.filter((l) => /S1[ABCD]_(SM|IW|EW|WV)_(SLC|GRD|RAW|OCN)/i.test(l));
  const errors = lines
    .filter((l) => !/S1[ABCD]_(SM|IW|EW|WV)_(SLC|GRD|RAW|OCN)/i.test(l))
    .map((line) => ({ line, error: "不是 Sentinel-1 颗粒名" }));
  if (!valid.length) {
    return { ok: false, error: "未能解析出任何有效的 Sentinel-1 场景", code: "ASF001" };
  }
  const scenes = valid.map((_, i) => mockScene(i));
  if (mock.region) mock.region = { ...mock.region, scene_count: scenes.length };
  else mock.standaloneSceneCount = scenes.length;
  notifyContextChanged();
  return { ok: true, scenes, duplicates: [], errors };
}

export async function importScenesFile(path: string): Promise<ScenesResult> {
  if (isTauriDesktopHost()) {
    const result = await importApplicationScenes({ source: "file", path });
    if (result.ok) notifyContextChanged();
    return result;
  }
  if (hasBridge()) {
    const res = await api().import_scenes_file(path);
    if (res.ok) notifyContextChanged();
    return res;
  }
  if (!path.trim()) return { ok: false, error: "请提供购物车文件路径", code: "ASF001" };
  const scenes = [mockScene(0), mockScene(1), mockScene(2)];
  const n = scenes.length;
  if (mock.region) mock.region = { ...mock.region, scene_count: scenes.length };
  else mock.standaloneSceneCount = scenes.length;
  notifyContextChanged();
  return {
    ok: true,
    scenes,
    duplicates: [],
    errors: [],
    queried: scenes.length,
    search: {
      requested_limit: n,
      query_limit: n,
      total_count: Math.max(n, scenes.length),
      returned_count: scenes.length,
      source: "mock",
    },
  };
}

export async function previewScenesFile(path: string): Promise<ScenesResult> {
  if (isTauriDesktopHost()) return importApplicationScenes({ source: "file", path });
  if (hasBridge()) {
    const previewer = api().preview_scenes_file;
    if (typeof previewer === "function") return previewer(path);
    return api().import_scenes_file(path);
  }
  if (!path.trim()) return { ok: false, error: "请提供购物车文件路径", code: "ASF001" };
  const scenes = [mockScene(0), mockScene(1), mockScene(2)];
  return { ok: true, scenes, duplicates: [], errors: [] };
}

export async function importScenesDirectory(path: string): Promise<ScenesResult> {
  if (isTauriDesktopHost()) {
    const result = await importApplicationScenes({ source: "directory", path });
    if (result.ok) notifyContextChanged();
    return result;
  }
  if (hasBridge()) {
    const res = await api().import_scenes_directory(path);
    if (res.ok) notifyContextChanged();
    return res;
  }
  if (!path.trim()) return { ok: false, error: "请提供 Sentinel-1 数据目录", code: "ASF001" };
  const scenes = [mockScene(0), mockScene(1)];
  if (mock.region) mock.region = { ...mock.region, scene_count: scenes.length };
  else mock.standaloneSceneCount = scenes.length;
  notifyContextChanged();
  return { ok: true, scenes, duplicates: [], errors: [] };
}

export async function previewScenesDirectory(path: string): Promise<ScenesResult> {
  if (isTauriDesktopHost()) return importApplicationScenes({ source: "directory", path });
  if (hasBridge()) {
    const previewer = api().preview_scenes_directory;
    if (typeof previewer === "function") return previewer(path);
    return api().import_scenes_directory(path);
  }
  if (!path.trim()) return { ok: false, error: "请提供 Sentinel-1 数据目录", code: "ASF001" };
  const scenes = [mockScene(0), mockScene(1)];
  return { ok: true, scenes, duplicates: [], errors: [] };
}

export async function clearOrbitCandidateScenes(): Promise<SimpleOk> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("clear_orbit_candidate_scenes");
  if (hasBridge()) {
    const clearer = api().clear_orbit_candidate_scenes;
    if (typeof clearer === "function") return clearer();
  }
  return { ok: true };
}

export async function searchAsfScenes(params: AsfSearchParams): Promise<ScenesResult> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("search_asf_scenes");
  if (hasBridge()) {
    const res = await api().search_asf_scenes(params);
    if (res.ok) notifyContextChanged();
    return res;
  }
  const n = Math.min(Number(params.max_results || 120) || 120, 1000);
  const product = String(params.product_type || "SLC").toUpperCase();
  const splitFilter = (value: unknown) =>
    String(value || "")
      .split(",")
      .map((item) => item.trim().toUpperCase())
      .filter(Boolean);
  const beams = splitFilter(params.beam_mode);
  const polarizations = splitFilter(params.polarization);
  const orbitDirections = splitFilter(params.orbit_direction);
  const scenes = Array.from({ length: Math.min(n, 6) }, (_, i) => ({
    ...mockScene(i),
    product_type: product,
    beam_mode: beams[i % Math.max(1, beams.length)] || mockScene(i).beam_mode,
    polarization: polarizations[i % Math.max(1, polarizations.length)] || mockScene(i).polarization,
    has_url: true,
    orbit_direction:
      orbitDirections.length > 0
        ? orbitDirections[i % orbitDirections.length]
        : i % 2 === 0
          ? "DESCENDING"
          : "ASCENDING",
    path: params.relative_orbit ? Number(params.relative_orbit) : 11 + i,
    relative_orbit: params.relative_orbit ? Number(params.relative_orbit) : 11 + i,
    frame: params.frame ? Number(params.frame) : 468 + i,
    footprint_bbox: mockFootprintBbox(i, params.bbox ?? mock.region?.bbox ?? MOCK_BBOX),
    footprint_geojson: polygonFromBbox(mockFootprintBbox(i, params.bbox ?? mock.region?.bbox ?? MOCK_BBOX)),
  }));
  if (mock.region) {
    const footprint = scenes[0]?.footprint_bbox ?? mock.region.scene_footprint_bbox;
    mock.region = { ...mock.region, scene_count: scenes.length, scene_footprint_bbox: footprint };
  } else {
    mock.standaloneSceneCount = scenes.length;
  }
  notifyContextChanged();
  return { ok: true, scenes, duplicates: [], errors: [] };
}

export async function cancelAsfSearch(): Promise<SimpleOk> {
  if (isTauriDesktopHost()) {
    const requestId = activeM1SearchRequestId;
    if (!requestId) return { ok: true, cancelled: false };
    const result = await getDesktopShellBridge().application.cancelRequest(requestId);
    return result.ok ? { ok: true, cancelled: result.cancelled } : applicationError(result);
  }
  if (hasBridge()) {
    const cancel = api().cancel_asf_search;
    if (typeof cancel === "function") return cancel();
  }
  return { ok: true, cancelled: true };
}

export async function listScenes(): Promise<ScenesResult> {
  if (isTauriDesktopHost()) {
    const result = await restoreApplicationCore(false);
    if (!result.ok) return result;
    const summary = result.core.workbench;
    if (!summary) return { ok: true, scenes: [], duplicates: [], errors: [] };
    const baseContext = batchContextFromCoreWorkbench(summary);
    const pageRequest = {
      batch_id: summary.batch_id,
      expected_revision: summary.revision,
      page_index: summary.page_index,
      page_size: MAX_UI_WORKBENCH_PAGE_SIZE,
    };
    const page = await loadApplicationWorkbenchPage(pageRequest, result.aoi_display?.envelope);
    const workbench = applicationWorkbenchFromCorePage(result.core, page, {
      request: pageRequest,
      base: baseContext,
      aoiAuthority: result.aoi_display,
    });
    if (!workbench) return page.ok ? applicationProjectionError() : page;
    return {
      ok: true,
      scenes: workbench.scenes,
      duplicates: [],
      errors: [],
    };
  }
  if (hasBridge()) return api().list_scenes();
  const n = mockActiveSceneCount();
  return { ok: true, scenes: Array.from({ length: n }, (_, i) => mockScene(i)), duplicates: [], errors: [] };
}

export async function clearScenes(): Promise<SimpleOk> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("clear_scenes");
  if (hasBridge()) {
    const res = await api().clear_scenes();
    if (res.ok) notifyContextChanged();
    return res;
  }
  const cleared = mockActiveSceneCount();
  if (mock.region) mock.region = { ...mock.region, scene_count: 0, scene_footprint_bbox: null };
  mock.standaloneSceneCount = 0;
  notifyContextChanged();
  return { ok: true, cleared };
}

export async function getMetadataStatus(): Promise<MetadataStatus> {
  if (isTauriDesktopHost()) {
    return {
      ok: true,
      state: "unavailable",
      done: 0,
      total: 0,
      percent: 0,
      message: "正式检索由 ApplicationService SearchBatch 提供状态。",
    };
  }
  if (hasBridge()) return api().get_metadata_status();
  return {
    ok: true,
    state: "idle",
    done: 0,
    total: 0,
    percent: 0,
    message: "",
  };
}

export async function checkScenes(): Promise<CheckResult> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("check_scenes");
  if (hasBridge()) return api().check_scenes();
  const n = mockActiveSceneCount();
  if (!n) return { ok: false, error: "请先导入场景", code: "ASF001" };
  return {
    ok: true,
    report: {
      total_scenes: n,
      valid_scenes: n,
      has_errors: false,
      has_warnings: true,
      issues: [
        {
          code: "SCENE_URL_MISSING",
          severity: "WARNING",
          message: "scene has no download URL",
          scene_id: mockScene(0).scene_id,
        },
      ],
      summary: { scene_count: n },
    },
  };
}

function mockOrbitReport(sceneCount: number, matched = sceneCount): Json {
  return {
    total_scenes: sceneCount,
    matched_scenes: matched,
    unmatched_scenes: Math.max(0, sceneCount - matched),
    results: Array.from({ length: sceneCount }, (_, i) => ({
      scene_id: mockScene(i).scene_id,
      is_matched: i < matched,
      matched_orbit:
        i < matched
          ? {
              file_name:
                "S1A_OPER_AUX_POEORB_OPOD_20240313T120000_V20240312T000000_20240313T000000.EOF",
              orbit_type: "POEORB",
            }
          : null,
    })),
    issues: [],
    summary: {
      orbit_files: matched,
      scenes: sceneCount,
      matched,
      unmatched: Math.max(0, sceneCount - matched),
      orbit_types: ["POEORB"],
    },
  };
}

const formalOrbitTaskIds = new Map<string, string[]>();

function normalizeOrbitSceneId(sceneId: string): string {
  const trimmed = sceneId.trim();
  const markerIdx = trimmed.lastIndexOf("::ITEM::");
  const tail = markerIdx >= 0 ? trimmed.slice(markerIdx + "::ITEM::".length).trim() : trimmed;
  return tail.replace(/-(?:SLC|GRD_HD|GRD_HS|GRD_MD|GRD_MS|GRD_FD|RAW|OCN)$/i, "");
}

function formalOrbitPlatform(sceneId: string): OrbitPlatform {
  const normalized = normalizeOrbitSceneId(sceneId);
  const match = /^S1([ABCD])[_-]/i.exec(normalized);
  return match ? (`S1${match[1].toUpperCase()}` as OrbitPlatform) : "UNKNOWN";
}

function orbitSceneInputs(
  scenes: SceneRow[],
): DesktopOrbitDirectoryMatchRequest["scenes"] {
  return scenes.map((scene) => {
    const seconds = Date.parse(scene.acquisition_datetime);
    const normalizedId = normalizeOrbitSceneId(scene.scene_id);
    return {
      scene_id: normalizedId,
      platform: formalOrbitPlatform(normalizedId),
      acquisition_unix_seconds: Number.isFinite(seconds) ? Math.floor(seconds / 1000) : null,
    };
  });
}

function formalOrbitAssociation(sceneId: string, index: number): FrozenOrbitAssociationV1 {
  const platform = formalOrbitPlatform(sceneId);
  const epoch = 1_710_000_000 + index * 86_400;
  return {
    association_version: 1,
    source_item_id: sceneId,
    orbit_transfer_item_id: sceneId,
    requested_orbit_type: "POEORB",
    source_platform: platform,
    source_acquisition_start: epoch,
    source_acquisition_stop: epoch + 3_600,
    matched_orbit_metadata: {
      file_name: `${platform}_OPER_AUX_POEORB_OPOD_20240313T120000_V20240312T000000_20240313T000000.EOF`,
      platform,
      orbit_type: "POEORB",
      creation_unix_seconds: epoch + 7_200,
      validity_start_unix_seconds: epoch - 3_600,
      validity_stop_unix_seconds: epoch + 7_200,
    } satisfies OrbitFileMetadata,
    selection_policy_id: "sentinel1.poeorb.frozen-association.v1",
  };
}

async function startFormalOrbitTasks(
  outputDir: string,
  sceneIds: string[],
  maxConcurrent: number,
  useOrbitSubdir: boolean,
  scenesSnapshot: SceneRow[] = [],
): Promise<DemTaskCommandResult> {
  const ids = sceneIds.map((value) => value.trim()).filter(Boolean);
  if (!ids.length) {
    return { ok: false, error: "请先选择需要下载 POEORB 的 SAR 影像", code: "ASF001" };
  }

  const standaloneRoute = getDesktopShellBridge().application.createStandaloneOrbitTask;
  if (standaloneRoute) {
    let root = outputDir.trim();
    let regionIdForTracker: string | null = null;
    if (!root) {
      const context = await getContext();
      if (context.ok) {
        root = context.region?.root || context.project?.root || context.workspace?.root || "";
        regionIdForTracker = context.region?.region_id ?? null;
      }
    }
    if (!root) {
      return { ok: false, error: "请先选择精密轨道下载保存目录", code: "ORB001" };
    }
    const sceneInputs = ids.map((requestedId) => {
      const frozenScene = scenesSnapshot.find(
        (scene) =>
          scene.scene_id === requestedId ||
          scene.product_id === requestedId ||
          scene.product_key === requestedId,
      );
      return {
        scene_id: normalizeOrbitSceneId(frozenScene?.scene_id ?? requestedId),
        acquisition_datetime: frozenScene?.acquisition_datetime ?? null,
      };
    });
    const invalidScene = sceneInputs.find(
      (entry) => formalOrbitPlatform(entry.scene_id) === "UNKNOWN",
    );
    if (invalidScene) {
      return {
        ok: false,
        error: `无法从冻结场景身份解析 Sentinel-1 平台：${invalidScene.scene_id}`,
        code: "ORB001",
      };
    }
    const taskId = stableUiNodeId(
      "orbit-task-standalone",
      globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`,
      64,
    );
    const result = await createApplicationStandaloneOrbitTask({
      request_id: `orbit-req-${taskId}`,
      task_id: taskId,
      scenes: sceneInputs,
      output_root: root,
      use_orbit_subdir: useOrbitSubdir,
      created_at: new Date().toISOString(),
    });
    if (!result.ok) return result;
    if (regionIdForTracker) {
      formalOrbitTaskIds.set(regionIdForTracker, [taskId]);
    }
    return { ok: true, task_id: taskId, state: "queued" };
  }

  const context = await getContext();
  if (!context.ok) {
    return { ok: false, error: context.error, code: context.code };
  }
  if (!context.workspace || !context.project || !context.region || !context.region.has_aoi) {
    return { ok: false, error: "请先打开工作区、项目、研究区并绑定 AOI", code: "V3_APPLICATION_INVALID_REQUEST" };
  }
  const root = outputDir.trim() || context.region.root || context.project.root || context.workspace.root;
  const taskIds: string[] = [];
  for (const [index, requestedId] of ids.entries()) {
    const frozenScene = scenesSnapshot.find((scene) =>
      scene.scene_id === requestedId
      || scene.product_id === requestedId
      || scene.product_key === requestedId,
    );
    const sceneId = normalizeOrbitSceneId(frozenScene?.scene_id ?? requestedId);
    if (formalOrbitPlatform(sceneId) === "UNKNOWN") {
      return {
        ok: false,
        error: `无法从冻结场景身份解析 Sentinel-1 平台：${sceneId}`,
        code: "ORB001",
      };
    }
    const taskId = stableUiNodeId("orbit-task", `${context.region.region_id}-${sceneId}-${Date.now()}-${index}`);
    const result = await createApplicationOrbitTask({
      request_id: `orbit-task-${taskId}`,
      task_id: taskId,
      region_id: context.region.region_id,
      association: formalOrbitAssociation(sceneId, index),
      output_root: useOrbitSubdir ? `${root}\\Sentinel_Orbit\\AUX_POEORB` : root,
      created_at: new Date().toISOString(),
      concurrency: Math.max(
        1,
        Math.min(Math.trunc(maxConcurrent) || getTransferEnginePrefs().max_concurrent_tasks, 10),
      ),
    });
    if (!result.ok) return result;
    taskIds.push(taskId);
  }
  formalOrbitTaskIds.set(context.region.region_id, taskIds);
  return { ok: true, task_id: taskIds[0], state: "queued" };
}

export async function matchOrbitsDirectory(
  orbitDir: string,
  scenes: SceneRow[] = [],
): Promise<OrbitMatchResult> {
  if (isTauriDesktopHost()) {
    if (!orbitDir.trim()) return { ok: false, error: "请提供精密轨道目录", code: "ORB001" };
    const localScenes = scenes.length ? scenes : transientImportedScenes;
    if (localScenes.length) {
      return matchApplicationOrbits({
        orbit_dir: orbitDir,
        scenes: orbitSceneInputs(localScenes),
      });
    }
    const restored = await restoreApplicationCore(false);
    if (!restored.ok) return restored;
    const workbench = restored.core.workbench;
    if (!workbench) {
      return { ok: false, error: "请先导入或检索 Sentinel-1 场景", code: "ASF001" };
    }
    const page = await loadApplicationWorkbenchPage({
      batch_id: workbench.batch_id,
      expected_revision: workbench.revision,
      page_index: workbench.page_index,
      page_size: MAX_UI_WORKBENCH_PAGE_SIZE,
    });
    if (!page.ok) return page;
    return matchApplicationOrbits({
      orbit_dir: orbitDir,
      scenes: orbitSceneInputs(page.scenes),
    });
  }
  if (hasBridge()) return api().match_orbits_directory(orbitDir);
  const n = mockActiveSceneCount();
  if (!n) return { ok: false, error: "请先导入 ASF 场景或本地 SLC 目录", code: "ASF001" };
  if (!orbitDir.trim()) return { ok: false, error: "请提供精密轨道目录", code: "ORB001" };
  return {
    ok: true,
    orbit_dir: orbitDir,
    orbit_files: n,
    report: mockOrbitReport(n),
  };
}

export async function downloadOrbits(
  outputDir = "",
  sceneIds: string[] = [],
  useOrbitSubdir = false,
): Promise<OrbitDownloadResult> {
  if (isTauriDesktopHost()) {
    const result = await startFormalOrbitTasks(
      outputDir,
      sceneIds,
      getTransferEnginePrefs().max_concurrent_tasks,
      useOrbitSubdir,
    );
    if (!result.ok) return result;
    const context = await getContext();
    const n = sceneIds.filter(Boolean).length;
    return {
      ok: true,
      orbit_dir: useOrbitSubdir ? `${context.ok && context.region?.root ? context.region.root : outputDir}\\Sentinel_Orbit\\AUX_POEORB` : outputDir,
      summary_line: `${n} orbit task(s) accepted by the shared Store/Scheduler/Download Engine`,
      total: n,
      succeeded: 0,
      skipped: 0,
      unavailable: 0,
      failed: 0,
      has_failures: false,
      results: [],
      report: { task_id: result.task_id ?? null, association: "FrozenOrbitAssociationV1" },
    };
  }
  if (hasBridge()) return api().download_orbits(outputDir, sceneIds, useOrbitSubdir);
  const selectedIds = sceneIds.filter(Boolean);
  const n = selectedIds.length || mockActiveSceneCount();
  if (!n) return { ok: false, error: "请先导入 ASF 场景或本地 SLC 目录", code: "ASF001" };
  const root = outputDir || mock.workspace?.root || "C:\\InSAR";
  const orbitDir = useOrbitSubdir ? `${root}\\Sentinel_Orbit\\AUX_POEORB` : root;
  return {
    ok: true,
    orbit_dir: orbitDir,
    summary_line: `${n} downloaded, 0 skipped, 0 unavailable, 0 failed`,
    total: n,
    succeeded: n,
    skipped: 0,
    unavailable: 0,
    failed: 0,
    has_failures: false,
    results: Array.from({ length: n }, (_, i) => ({
      scene_id: selectedIds[i] ?? mockScene(i).scene_id,
      outcome: "success",
      orbit_file: `S1A_OPER_AUX_POEORB_OPOD_202403${13 + i}T120000_V202403${12 + i}T000000_202403${13 + i}T000000.EOF`,
      orbit_type: "POEORB",
      path: `${orbitDir}\\orbit.EOF`,
      bytes_written: 1024,
      message: "已从 ASF 下载 POEORB 精密轨道文件。",
    })),
    report: mockOrbitReport(n),
  };
}

let mockOrbitState: OrbitDownloadStatus = {
  ok: true,
  state: "idle",
  total: 0,
  done: 0,
  concurrency: 10,
  current_scene: "",
  active_scenes: [],
  orbit_dir: "",
  done_bytes: 0,
  bytes_per_second: 0,
  elapsed_seconds: 0,
  paused: false,
  cancelled: false,
  error: null,
  summary_line: "",
  succeeded: 0,
  skipped: 0,
  unavailable: 0,
  failed: 0,
  has_failures: false,
  results: [],
  log: [],
  report: null,
  pause_hint: "轨道文件较小，暂停/结束会在当前 EOF 请求结束后生效。",
};

export async function startOrbitDownload(
  outputDir = "",
  sceneIds: string[] = [],
  maxConcurrent = getTransferEnginePrefs().max_concurrent_tasks,
  useOrbitSubdir = false,
): Promise<{ ok: boolean; error?: string; code?: string | null; task_id?: string }> {
  if (isTauriDesktopHost()) return startFormalOrbitTasks(outputDir, sceneIds, maxConcurrent, useOrbitSubdir);
  if (hasBridge()) return api().start_orbit_download(outputDir, sceneIds, maxConcurrent, useOrbitSubdir);
  const selectedIds = sceneIds.filter(Boolean);
  const n = selectedIds.length || mockActiveSceneCount();
  if (!n) return { ok: false, error: "请先导入 ASF 场景或本地 SLC 目录", code: "ASF001" };
  const root = outputDir || mock.region?.root || mock.project?.root || mock.workspace?.root || "C:\\InSAR";
  const orbitDir = useOrbitSubdir ? `${root}\\Sentinel_Orbit\\AUX_POEORB` : root;
  const results = Array.from({ length: n }, (_, i) => ({
    scene_id: selectedIds[i] ?? mockScene(i).scene_id,
    outcome: "success",
    orbit_file: `S1A_OPER_AUX_POEORB_OPOD_202403${13 + i}T120000_V202403${12 + i}T000000_202403${13 + i}T000000.EOF`,
    orbit_type: "POEORB",
    path: `${orbitDir}\\orbit.EOF`,
    bytes_written: 1024,
    message: "mock POEORB ok",
  }));
  const completedAt = new Date().toISOString();
  mockOrbitState = {
    ...mockOrbitState,
    created_at: completedAt,
    ended_at: completedAt,
    state: "finished",
    total: n,
    done: n,
    concurrency: Math.max(1, Math.min(Number(maxConcurrent) || 10, 10)),
    current_scene: "",
    active_scenes: [],
    orbit_dir: orbitDir,
    use_orbit_subdir: useOrbitSubdir,
    download_layout: useOrbitSubdir ? "orbit_subdir" : "flat",
    done_bytes: n * 1024,
    bytes_per_second: n * 1024,
    paused: false,
    cancelled: false,
    error: null,
    summary_line: `${n} downloaded, 0 skipped, 0 unavailable, 0 failed`,
    succeeded: n,
    skipped: 0,
    unavailable: 0,
    failed: 0,
    has_failures: false,
    results,
    log: results.map((r) => ({
      scene_id: String(r.scene_id),
      outcome: String(r.outcome),
      detail: `${r.outcome}: ${r.message}`,
    })),
    report: mockOrbitReport(n),
  };
  return { ok: true };
}

export async function startOrbitDownloadSnapshot(
  outputDir = "",
  scenesSnapshot: SceneRow[] = [],
  sceneIds: string[] = [],
  maxConcurrent = getTransferEnginePrefs().max_concurrent_tasks,
  useOrbitSubdir = false,
  aoiName = "",
  batchContext: DownloadBatchContext | null = null,
): Promise<{ ok: boolean; error?: string; code?: string | null; task_id?: string }> {
  if (isTauriDesktopHost()) {
    const ids = sceneIds.length ? sceneIds : scenesSnapshot.map((scene) => scene.product_key ?? scene.scene_id);
    return startFormalOrbitTasks(outputDir, ids, maxConcurrent, useOrbitSubdir, scenesSnapshot);
  }
  if (hasBridge()) {
    const startSnapshot = api().start_orbit_download_snapshot;
    if (startSnapshot) return startSnapshot(outputDir, scenesSnapshot, sceneIds, maxConcurrent, useOrbitSubdir, aoiName, batchContext);
  }
  return startOrbitDownload(outputDir, sceneIds, maxConcurrent, useOrbitSubdir);
}

export async function pauseOrbitDownload(): Promise<{ ok: boolean; error?: string; code?: string }> {
  if (isTauriDesktopHost()) {
    const persisted = await formalTaskIdsForCurrentRegion("orbit");
    if (!persisted.ok) return { ok: false, error: persisted.error, code: persisted.code ?? undefined };
    const taskIds = persisted.tasks
      .filter((task) => !["completed", "failed", "cancelled", "interrupted"].includes(task.status))
      .map((task) => task.task_id);
    for (const taskId of taskIds) {
      const result = await formalTaskCommand(taskId, "pause_task");
      if (!result.ok) return { ok: false, error: result.error, code: result.code ?? undefined };
    }
    return { ok: true };
  }
  if (hasBridge()) return api().pause_orbit_download();
  mockOrbitState = { ...mockOrbitState, state: "paused", paused: true };
  return { ok: true };
}

export async function resumeOrbitDownload(): Promise<{ ok: boolean; error?: string; code?: string }> {
  if (isTauriDesktopHost()) {
    const persisted = await formalTaskIdsForCurrentRegion("orbit");
    if (!persisted.ok) return { ok: false, error: persisted.error, code: persisted.code ?? undefined };
    const taskIds = persisted.tasks
      .filter((task) => !["completed", "failed", "cancelled", "interrupted"].includes(task.status))
      .map((task) => task.task_id);
    for (const taskId of taskIds) {
      const result = await formalTaskCommand(taskId, "resume_task");
      if (!result.ok) return { ok: false, error: result.error, code: result.code ?? undefined };
    }
    return { ok: true };
  }
  if (hasBridge()) return api().resume_orbit_download();
  mockOrbitState = { ...mockOrbitState, state: "running", paused: false };
  return { ok: true };
}

export async function stopOrbitDownload(): Promise<{ ok: boolean; error?: string; code?: string }> {
  if (isTauriDesktopHost()) {
    const persisted = await formalTaskIdsForCurrentRegion("orbit");
    if (!persisted.ok) return { ok: false, error: persisted.error, code: persisted.code ?? undefined };
    const taskIds = persisted.tasks
      .filter((task) => !["completed", "failed", "cancelled", "interrupted"].includes(task.status))
      .map((task) => task.task_id);
    for (const taskId of taskIds) {
      const result = await formalTaskCommand(taskId, "cancel_task");
      if (!result.ok) return { ok: false, error: result.error, code: result.code ?? undefined };
    }
    return { ok: true };
  }
  if (hasBridge()) return api().stop_orbit_download();
  mockOrbitState = { ...mockOrbitState, state: "cancelled", cancelled: true, paused: false };
  return { ok: true };
}

export async function getOrbitDownloadStatus(): Promise<OrbitDownloadStatus> {
  if (isTauriDesktopHost()) {
    const persisted = await formalTaskIdsForCurrentRegion("orbit");
    if (!persisted.ok) {
      if (persisted.code === "FORMAL_TASK_REGION_UNSET") {
        return {
          ...mockOrbitState,
          state: "idle",
          task_id: undefined,
          total: 0,
          done: 0,
          current_scene: "",
          active_scenes: [],
          orbit_dir: "",
          done_bytes: 0,
          bytes_per_second: 0,
          elapsed_seconds: 0,
          paused: false,
          cancelled: false,
          error: null,
          summary_line: "",
          succeeded: 0,
          skipped: 0,
          unavailable: 0,
          failed: 0,
          has_failures: false,
          results: [],
          log: [],
          report: null,
        };
      }
      return {
        ...mockOrbitState,
        state: "failed",
        error: persisted.error,
        summary_line: `POEORB 恢复失败：${persisted.error}`,
      };
    }
    const tasks = persisted.tasks.filter((task) => task.operation?.kind === "orbit");
    const taskIds = tasks.map((task) => task.task_id);
    const [singleTask] = tasks;
    if (!tasks.length) {
      return {
        ...mockOrbitState,
        state: "idle",
        orbit_dir: persisted.region.root ?? "",
        error: null,
        summary_line: "当前研究区没有正式 POEORB 任务",
      };
    }
    const total = Math.max(1, tasks.reduce((sum, task) => sum + Math.max(1, task.selected_count), 0));
    const done = Math.min(total, tasks.reduce((sum, task) =>
      sum + task.completed_count + task.skipped_count, 0));
    const failed = tasks.reduce((sum, task) => sum + task.failed_count + task.cancelled_count, 0);
    const lifecycle = tasks.map((task) => task.status);
    const fullySuccessful = tasks.length > 0
      && done >= total
      && tasks.every((task) => task.selected_count > 0 && task.failed_count === 0 && task.cancelled_count === 0);
    const state: string = fullySuccessful
      ? "finished"
      : lifecycle.some((value) => value === "cancelled" || value === "interrupted")
      ? "cancelled"
      : lifecycle.some((value) => value === "failed") || failed > 0
        ? "failed"
        : lifecycle.some((value) => value === "paused")
          ? "paused"
          : lifecycle.some((value) => value === "running")
            ? "running"
            : lifecycle.some((value) => value === "queued")
              ? "queued"
              : "finished";
    const createdAt = tasks
      .map((task) => task.created_at)
      .filter(Boolean)
      .sort()[0];
    const endedAt = tasks.length > 0 && tasks.every((task) =>
      ["completed", "failed", "cancelled", "interrupted"].includes(task.status),
    )
      ? tasks.map((task) => task.updated_at).filter(Boolean).sort().at(-1)
      : null;
    return {
      ok: true,
      created_at: createdAt,
      ended_at: endedAt,
      state,
      task_id: singleTask ? taskIds.find((taskId) => taskId === singleTask.task_id) : undefined,
      concurrency: tasks.length === 1 ? singleTask?.concurrency ?? undefined : undefined,
      total,
      done,
      current_scene: "",
      active_scenes: [],
      selected_scene_ids: [],
      checked_scene_ids: [],
      paused_scene_ids: [],
      queued_scene_ids: [],
      output_dir: singleTask?.output_root,
      orbit_dir: persisted.region.root ?? "",
      download_layout: singleTask?.layout,
      done_bytes: 0,
      bytes_per_second: 0,
      elapsed_seconds: 0,
      paused: state === "paused",
      cancelled: state === "cancelled",
      error: failed > 0 ? "一个或多个 POEORB 任务失败" : null,
      summary_line: `${done}/${total} 个正式 POEORB 任务`,
      succeeded: tasks.reduce((sum, task) => sum + task.completed_count, 0),
      skipped: tasks.reduce((sum, task) => sum + task.skipped_count, 0),
      unavailable: 0,
      failed,
      has_failures: failed > 0,
      results: [],
      log: [],
      report: {
        task_ids: taskIds,
        region_id: persisted.region.region_id,
        route: "CoreRestore task summaries; exact item progress is not restored",
      },
    };
  }
  if (hasBridge()) return api().get_orbit_download_status();
  return mockOrbitState;
}

// ---------------------------------------------------------------- DOWNLOAD
function mockAsfPlan(): Json {
  const n = mockActiveSceneCount();
  const items = Array.from({ length: n }, (_, i) => {
    const s = mockScene(i);
    return {
      scene_id: s.scene_id,
      platform: "S1A",
      acquisition_datetime: s.acquisition_datetime,
      product: "SLC",
      beam: "IW",
      polarization: "DV",
      url_status: "missing",
      status: "MISSING_URL",
      expected_filename: `${s.scene_id}.zip`,
    };
  });
  return {
    region_safe_name: mockActiveSafeName(),
    scene_count: n,
    planned_count: 0,
    missing_url_count: n,
    credential_required: true,
    slc_directory: `${mock.workspace?.root ?? "C:\\InSAR"}\\SAR_Data\\SLC`,
    items,
  };
}

function mockDemPlan(
  dataset: string,
  bbox: Bbox = mock.region?.bbox ?? MOCK_BBOX,
  safe = mock.region?.safe_name ?? "standalone_dem",
  root = mock.workspace?.root ?? "C:\\InSAR",
): Json {
  const b = bbox;
  return {
    dataset,
    provider: "OPENTOPOGRAPHY",
    request_bbox: {
      west: b.west - 0.05,
      east: b.east + 0.05,
      south: b.south - 0.05,
      north: b.north + 0.05,
    },
    processing_bbox: b,
    buffer_degrees: 0.05,
    source_vertical_datum: mockSourceDatum(dataset),
    target_vertical_datum: "WGS84_ELLIPSOID",
    raw_dem_path: `${root}\\${demSourceStem(dataset)}.tif`,
    ellipsoid_dem_path: `${root}\\${demSourceStem(dataset)}_ellipsoid.tif`,
    sarscape_ready_dem_path: `${root}\\${demSourceStem(dataset)}_dem`,
  };
}

function mockReport(hasErrors = false): Json {
  return {
    has_errors: hasErrors,
    has_warnings: false,
    issues: [],
    summary: { overall_status: hasErrors ? "blocked" : "ready" },
  };
}

export async function planAsfDownload(
  outputDir = "",
  sceneIds: string[] = [],
  useProductSubdirs = false,
): Promise<PlanResult> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("plan_asf_download");
  if (hasBridge()) return api().plan_asf_download(outputDir, sceneIds, useProductSubdirs);
  if (!(sceneIds.length || mockActiveSceneCount())) {
    return { ok: false, error: "请先在『影像核查』导入场景", code: "ASF001" };
  }
  return { ok: true, plan: mockAsfPlan() };
}

let mockDlState: DownloadStatus = {
  ok: true,
    state: "idle",
    total: 0,
    done: 0,
    current_scene: "",
    total_bytes: null,
    done_bytes: 0,
    current_bytes: 0,
    current_expected_size: null,
    bytes_per_second: 0,
    elapsed_seconds: 0,
    concurrency: 1,
    paused: false,
  cancelled: false,
    error: null,
    summary_line: "",
    results_path: "",
    succeeded: 0,
    skipped: 0,
    failed: 0,
    interrupted: 0,
    has_failures: false,
    paused_scene_ids: [],
    resume_supported: true,
    resume_hint: "暂停或强制结束会保留 .part；再次开始同一输出目录会断点续传。",
    retry_supported: false,
    retry_hint: "只重试失败/中断的场景；已完成文件会跳过，.part 文件会继续续传。",
    log: [],
};

export async function startAsfDownload(
  outputDir = "",
  credentialSource = "auto",
  maxConcurrent = 1,
  sceneIds: string[] = [],
  useProductSubdirs = false,
): Promise<{ ok: boolean; error?: string; code?: string; task_id?: string }> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("start_asf_download");
  if (hasBridge()) return api().start_asf_download(outputDir, credentialSource, maxConcurrent, sceneIds, useProductSubdirs);
  const n = sceneIds.length || mockActiveSceneCount();
  if (!n) return { ok: false, error: "请先导入场景", code: "ASF001" };
  if (mockCredentials.earthdata === "none" || mockCredentials.earthdata === "unavailable") {
    return {
      ok: false,
      error: "开始 Sentinel-1 下载前，请先在设置中保存 Earthdata Token 或账号密码。",
      code: "DL004",
    };
  }
  mockDlState = {
    ok: true,
    state: "finished",
    total: n,
    done: n,
    current_scene: "",
    total_bytes: n * 1024,
    done_bytes: n * 1024,
    current_bytes: 0,
    current_expected_size: null,
    bytes_per_second: 1024,
    elapsed_seconds: 1,
    concurrency: maxConcurrent,
    paused: false,
    cancelled: false,
    error: null,
    summary_line: `${n} 已下载, 0 跳过, 0 失败, 0 中断`,
    results_path: "",
    output_dir: outputDir,
    use_product_subdirs: useProductSubdirs,
    download_layout: useProductSubdirs ? "product_subdirs" : "flat",
    succeeded: n,
    skipped: 0,
    failed: 0,
    interrupted: 0,
    has_failures: false,
    resume_supported: true,
    resume_hint: "暂停或强制结束会保留 .part；再次开始同一输出目录会断点续传。",
    retry_supported: false,
    retry_hint: "只重试失败/中断的场景；已完成文件会跳过，.part 文件会继续续传。",
    log: [],
  };
  return { ok: true };
}

export async function startAsfDownloadSnapshot(
  outputDir = "",
  scenesSnapshot: SceneRow[] = [],
  sceneIds: string[] = [],
  credentialSource = "auto",
  maxConcurrent = 1,
  useProductSubdirs = false,
  aoiName = "",
  taskId = "",
  batchContext: DownloadBatchContext | null = null,
  exactItemId: string | null = null,
): Promise<{ ok: boolean; error?: string; code?: string; task_id?: string; workbench?: DownloadTaskWorkbench; download_status?: DownloadStatus }> {
  if (isTauriDesktopHost()) {
    if (!taskId || !batchContext) {
      return { ok: false, error: "单一 Tauri 宿主下载 requires a frozen Store batch", code: "V3_APPLICATION_INVALID_REQUEST" };
    }
    const result = await getDesktopShellBridge().application.createTaskFromWorkbench({
      task_id: taskId,
      batch_id: batchContext.batch_id,
      expected_workbench_revision: batchContext.revision ?? 0,
      exact_item_id: exactItemId,
      output_root: outputDir,
      layout: useProductSubdirs ? "sentinel1_slc" : "direct",
      concurrency: Math.max(1, Math.trunc(maxConcurrent) || 1),
      created_at: new Date().toISOString(),
    });
    if (!result.ok) return m1LegacyCommandError(result);
    const task = taskWorkbenchFromCoreSnapshot(result, taskId, scenesSnapshot, sceneIds, batchContext);
    const downloadStatus = createdDownloadStatusFromCoreSnapshot(result, taskId);
    return task.ok && downloadStatus
      ? { ok: true, task_id: taskId, workbench: task.workbench, download_status: downloadStatus }
      : m1LegacyCommandError(task.ok ? applicationProjectionError() : task);
  }
  if (hasBridge()) {
    const starter = api().start_asf_download_snapshot;
    if (typeof starter === "function") {
      return starter(outputDir, scenesSnapshot, sceneIds, credentialSource, maxConcurrent, useProductSubdirs, aoiName, taskId, batchContext);
    }
    return api().start_asf_download(outputDir, credentialSource, maxConcurrent, sceneIds, useProductSubdirs);
  }
  const n = sceneIds.length || scenesSnapshot.length || mockActiveSceneCount();
  return startAsfDownload(outputDir, credentialSource, maxConcurrent, Array.from({ length: n }, (_, i) => sceneIds[i] || `snapshot_${i}`), useProductSubdirs);
}

export async function startSarDownloadSnapshot(
  outputDir = "",
  productsSnapshot: SarProduct[] = [],
  productIds: string[] = [],
  maxConcurrent = 1,
  organizeByDataset = false,
  batchContext: DownloadBatchContext | null = null,
  taskId = "",
  exactItemId: string | null = null,
): Promise<{ ok: boolean; error?: string; code?: string; task_id?: string; workbench?: DownloadTaskWorkbench; download_status?: DownloadStatus }> {
  if (isTauriDesktopHost()) {
    const scenes = productsSnapshot.map(projectSarProductToScene);
    if (!taskId || !batchContext) {
      return { ok: false, error: "单一 Tauri 宿主下载 requires a frozen Store batch", code: "V3_APPLICATION_INVALID_REQUEST" };
    }
    const result = await getDesktopShellBridge().application.createTaskFromWorkbench({
      task_id: taskId,
      batch_id: batchContext.batch_id,
      expected_workbench_revision: batchContext.revision ?? 0,
      exact_item_id: exactItemId,
      output_root: outputDir,
      layout: organizeByDataset ? "sar_dataset_processing" : "direct",
      concurrency: Math.max(1, Math.trunc(maxConcurrent) || 1),
      created_at: new Date().toISOString(),
    });
    if (!result.ok) return m1LegacyCommandError(result);
    const task = taskWorkbenchFromCoreSnapshot(result, taskId, scenes, productIds, batchContext);
    const downloadStatus = createdDownloadStatusFromCoreSnapshot(result, taskId);
    return task.ok && downloadStatus
      ? { ok: true, task_id: taskId, workbench: task.workbench, download_status: downloadStatus }
      : m1LegacyCommandError(task.ok ? applicationProjectionError() : task);
  }
  if (hasBridge()) {
    const starter = api().start_sar_download_snapshot;
    if (typeof starter === "function") {
      return starter(outputDir, productsSnapshot, productIds, maxConcurrent, organizeByDataset, batchContext, taskId);
    }
    return { ok: false, error: "Desktop API does not expose SAR downloads", code: "GUI003" };
  }
  return { ok: true, task_id: `mock-sar-${Date.now()}` };
}

export async function startLandsatBundleDownload(
  outputDir: string,
  itemId: string,
  maxConcurrent: number,
  batchContext: DownloadBatchContext | null,
  taskId: string,
  requestId: string,
  organizeByProduct = false,
): Promise<{
  ok: boolean;
  error?: string;
  code?: string;
  task_id?: string;
  workbench?: DownloadTaskWorkbench;
  search_workbench?: ApplicationWorkbenchProjection;
  download_status?: DownloadStatus;
}> {
  if (!isTauriDesktopHost()) {
    return {
      ok: false,
      error: "Landsat bundle downloads require the 单一 Tauri 业务宿主",
      code: "V3_APPLICATION_UNAVAILABLE",
    };
  }
  if (!outputDir || !itemId || !batchContext || !taskId || !requestId) {
    return {
      ok: false,
      error: "Landsat bundle download requires one frozen Store scene",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  const result = await createApplicationLandsatBundleTask({
    request_id: requestId,
    source_batch_id: batchContext.batch_id,
    item_id: itemId,
    task_id: taskId,
    output_root: outputDir,
    layout: organizeByProduct ? "sar_dataset_processing" : "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1)),
    created_at: new Date().toISOString(),
  });
  const expectedBatchId = `landsat-delivery-${taskId}`;
  if (!result.ok) return recoverOpticalCreationFailure(result, taskId, expectedBatchId);
  return opticalTaskCreationReceiptOrRecover(result, taskId, expectedBatchId);
}

export async function startLandsatAssetDownload(
  outputDir: string,
  itemId: string,
  maxConcurrent: number,
  batchContext: DownloadBatchContext | null,
  taskId: string,
  requestId: string,
  assetKeys: readonly string[] = [],
): Promise<{
  ok: boolean;
  error?: string;
  code?: string;
  task_id?: string;
  workbench?: DownloadTaskWorkbench;
  search_workbench?: ApplicationWorkbenchProjection;
  download_status?: DownloadStatus;
}> {
  if (!outputDir || !itemId || !batchContext || !taskId || !requestId) {
    return {
      ok: false,
      error: "Landsat 公开产品下载需要一个已冻结的检索景。",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  const result = await createApplicationLandsatAssetTask({
    request_id: requestId,
    source_batch_id: batchContext.batch_id,
    item_id: itemId,
    asset_keys: Array.from(new Set(assetKeys.map((key) => key.trim()).filter(Boolean))),
    task_id: taskId,
    output_root: outputDir,
    layout: "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1)),
    created_at: new Date().toISOString(),
  });
  const expectedBatchId = `landsat-assets-${taskId}`;
  if (!result.ok) return recoverOpticalCreationFailure(result, taskId, expectedBatchId);
  return opticalTaskCreationReceiptOrRecover(result, taskId, expectedBatchId);
}

export async function startLandsatAssetBatchDownload(
  outputDir: string,
  itemIds: readonly string[],
  batchContext: DownloadBatchContext,
  taskId: string,
  requestId: string,
  maxConcurrent = 2,
  assetKeys?: readonly string[],
) {
  return startLandsatBatchDownload(outputDir, itemIds, batchContext, taskId, requestId, maxConcurrent, "assets", assetKeys);
}

export async function startLandsatBundleBatchDownload(
  outputDir: string,
  itemIds: readonly string[],
  batchContext: DownloadBatchContext,
  taskId: string,
  requestId: string,
  maxConcurrent = 2,
) {
  return startLandsatBatchDownload(outputDir, itemIds, batchContext, taskId, requestId, maxConcurrent, "archive");
}

async function startLandsatBatchDownload(
  outputDir: string,
  itemIds: readonly string[],
  batchContext: DownloadBatchContext,
  taskId: string,
  requestId: string,
  maxConcurrent: number,
  delivery: "assets" | "archive",
  assetKeys?: readonly string[],
): Promise<{ ok: boolean; task_id?: string; download_status?: DownloadStatus; error?: string; code?: string;
  admission?: import("./desktopShellBridge").DesktopOpticalDeliveryAdmission }> {
  const useStoreCheckedSelection = itemIds.length === 0;
  const checkedCount = batchContext.checked_count ?? 0;
  if (!outputDir.trim() || !batchContext.batch_id || !Number.isSafeInteger(batchContext.revision)
    || (useStoreCheckedSelection
      ? !Number.isSafeInteger(checkedCount) || checkedCount < 1 || checkedCount > 250
      : itemIds.length > 250 || new Set(itemIds).size !== itemIds.length || itemIds.some(id => !id.trim()))
    || !taskId || !requestId) {
    return { ok: false, code: "V3_APPLICATION_INVALID_REQUEST", error: "下载批次或勾选身份不完整，请重新选择。" };
  }
  const application = getDesktopShellBridge().application;
  const create = delivery === "archive" ? application.createLandsatBundleBatchTask : application.createLandsatAssetBatchTask;
  const result = await create({
    request_id: requestId, source_batch_id: batchContext.batch_id,
    expected_workbench_revision: batchContext.revision!, item_ids: [...itemIds],
    ...(delivery === "assets" && assetKeys && assetKeys.length > 0 ? { asset_keys: [...assetKeys] } : {}),
    task_id: taskId, output_root: outputDir, layout: "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent))), created_at: new Date().toISOString(),
  });
  const prefix = delivery === "archive" ? "landsat-bundles-batch-" : "landsat-assets-batch-";
  const expectedBatchId = `${prefix}${taskId}`;
  if (!result.ok) {
    const recovered = await recoverOpticalCreationFailure(result, taskId, expectedBatchId);
    if (!recovered.ok) return recovered;
    const admission = await recoverOpticalDeliveryAdmission(taskId);
    return admission ? { ...recovered, admission } : recovered;
  }
  if (useStoreCheckedSelection && result.admission.requested_count !== checkedCount) {
    return m1LegacyCommandError(applicationProjectionError());
  }
  if (result.snapshot === null) return { ok: false, code: "OPTICAL_ADMISSION_ALL_REJECTED",
    error: "所有影像准备失败，未创建任务。", admission: result.admission };
  const receipt = await opticalTaskCreationReceiptOrRecover(result.snapshot, taskId, expectedBatchId);
  return receipt.ok ? { ...receipt, admission: result.admission } : receipt;
}

export function loadTaskDeliveryAdmission(taskId: string) {
  return getDesktopShellBridge().application.taskDeliveryAdmission(taskId);
}

export async function startSentinel2ArchiveDownload(
  outputDir: string,
  itemId: string,
  batchContext: DownloadBatchContext | null,
  taskId: string,
  requestId: string,
  maxConcurrent = 2,
  organizeByProduct = false,
): Promise<{
  ok: boolean;
  error?: string;
  code?: string;
  task_id?: string;
  workbench?: DownloadTaskWorkbench;
  search_workbench?: ApplicationWorkbenchProjection;
  download_status?: DownloadStatus;
}> {
  if (!outputDir || !itemId || !batchContext || !taskId || !requestId) {
    return {
      ok: false,
      error: "Sentinel-2 SAFE 下载需要一个已冻结的 Microsoft 检索景。",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  const result = await createApplicationSentinel2ArchiveTask({
    request_id: requestId,
    source_batch_id: batchContext.batch_id,
    item_id: itemId,
    task_id: taskId,
    output_root: outputDir,
    layout: organizeByProduct ? "sar_dataset_processing" : "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1)),
    created_at: new Date().toISOString(),
  });
  const expectedBatchId = `sentinel2-archive-${taskId}`;
  if (!result.ok) return recoverOpticalCreationFailure(result, taskId, expectedBatchId);
  return opticalTaskCreationReceiptOrRecover(result, taskId, expectedBatchId);
}

export async function startSentinel2ArchiveBatchDownload(
  outputDir: string,
  itemIds: readonly string[],
  batchContext: DownloadBatchContext | null,
  taskId: string,
  requestId: string,
  maxConcurrent = 2,
): Promise<{
  ok: boolean;
  error?: string;
  code?: string;
  task_id?: string;
  workbench?: DownloadTaskWorkbench;
  search_workbench?: ApplicationWorkbenchProjection;
  download_status?: DownloadStatus;
  admission?: DesktopSentinel2ArchiveAdmissionSummary;
}> {
  const uniqueItemIds = [...new Set(itemIds.map((itemId) => itemId.trim()).filter(Boolean))];
  const useStoreCheckedSelection = uniqueItemIds.length === 0;
  const checkedCount = batchContext?.checked_count ?? 0;
  if (
    !outputDir
    || !batchContext
    || !Number.isSafeInteger(batchContext.revision)
    || !taskId
    || !requestId
    || (useStoreCheckedSelection
      ? !Number.isSafeInteger(checkedCount) || checkedCount < 1 || checkedCount > 250
      : uniqueItemIds.length > 250)
  ) {
    return {
      ok: false,
      error: "Sentinel-2 SAFE 批量下载需要 1–250 个已冻结的 Microsoft 检索景。",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  const result = await createApplicationSentinel2ArchiveBatchTask({
    request_id: requestId,
    source_batch_id: batchContext.batch_id,
    expected_workbench_revision: batchContext.revision!,
    item_ids: uniqueItemIds,
    task_id: taskId,
    output_root: outputDir,
    layout: "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1)),
    created_at: new Date().toISOString(),
  });
  const expectedBatchId = `sentinel2-archive-batch-${taskId}`;
  if (!result.ok) {
    const recovered = await recoverOpticalCreationFailure(result, taskId, expectedBatchId);
    if (!recovered.ok) return recovered;
    const admission = await recoverOpticalDeliveryAdmission(taskId);
    return admission ? { ...recovered, admission } : recovered;
  }
  if (useStoreCheckedSelection) {
    if (result.admission.requested_count !== checkedCount) {
      return m1LegacyCommandError(applicationProjectionError());
    }
  } else {
    const rejected = new Set(result.admission.rejections.map((item) => item.source_item_id));
    const admittedSourceItemIds = uniqueItemIds.filter((itemId) => !rejected.has(itemId));
    if (admittedSourceItemIds.length !== result.admission.admitted_count) {
      return m1LegacyCommandError(applicationProjectionError());
    }
  }
  if (result.snapshot === null) return { ok: false, code: "OPTICAL_ADMISSION_ALL_REJECTED",
    error: "所有影像准备失败，未创建任务。", admission: result.admission };
  const receipt = await opticalTaskCreationReceiptOrRecover(result.snapshot, taskId, expectedBatchId);
  return receipt.ok ? { ...receipt, admission: result.admission } : receipt;
}

export async function startSentinel2BandBatchDownload(
  outputDir: string, itemIds: readonly string[], assetKeys: readonly string[],
  batchContext: DownloadBatchContext, taskId: string, requestId: string, maxConcurrent = 2,
): Promise<{ ok: boolean; task_id?: string; download_status?: DownloadStatus; error?: string; code?: string;
  admission?: import("./desktopShellBridge").DesktopOpticalDeliveryAdmission }> {
  const useStoreCheckedSelection = itemIds.length === 0;
  const checkedCount = batchContext.checked_count ?? 0;
  if (!outputDir.trim() || !batchContext.batch_id || !Number.isSafeInteger(batchContext.revision)
    || (useStoreCheckedSelection
      ? !Number.isSafeInteger(checkedCount) || checkedCount < 1 || checkedCount > 250
      : itemIds.length > 250 || new Set(itemIds).size !== itemIds.length || itemIds.some(id => !id.trim()))
    || !taskId || !requestId || !assetKeys.length) {
    return { ok: false, code: "V3_APPLICATION_INVALID_REQUEST", error: "下载批次或波段选择不完整，请重新选择。" };
  }
  const result = await getDesktopShellBridge().application.createSentinel2BandBatchTask({
    request_id: requestId, source_batch_id: batchContext.batch_id, expected_workbench_revision: batchContext.revision!,
    item_ids: [...itemIds], asset_keys: [...assetKeys], task_id: taskId, output_root: outputDir, layout: "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1)), created_at: new Date().toISOString(),
  });
  const expectedBatchId = `sentinel2-bands-batch-${taskId}`;
  if (!result.ok) {
    const recovered = await recoverOpticalCreationFailure(result, taskId, expectedBatchId);
    if (!recovered.ok) return recovered;
    const admission = await recoverOpticalDeliveryAdmission(taskId);
    return admission ? { ...recovered, admission } : recovered;
  }
  if (useStoreCheckedSelection && result.admission.requested_count !== checkedCount) {
    return m1LegacyCommandError(applicationProjectionError());
  }
  if (result.snapshot === null) return { ok: false, code: "OPTICAL_ADMISSION_ALL_REJECTED",
    error: "所有影像准备失败，未创建任务。", admission: result.admission };
  const receipt = await opticalTaskCreationReceiptOrRecover(result.snapshot, taskId, expectedBatchId);
  return receipt.ok ? { ...receipt, admission: result.admission } : receipt;
}

export async function startSentinel2L1cBandBatchDownload(
  outputDir: string,
  itemIds: readonly string[],
  assetKeys: readonly string[],
  batchContext: DownloadBatchContext,
  taskId: string,
  requestId: string,
  maxConcurrent = 2,
): Promise<{
  ok: boolean;
  task_id?: string;
  download_status?: DownloadStatus;
  error?: string;
  code?: string;
  admission?: import("./desktopShellBridge").DesktopOpticalDeliveryAdmission;
}> {
  const useStoreCheckedSelection = itemIds.length === 0;
  const checkedCount = batchContext.checked_count ?? 0;
  if (
    !outputDir.trim()
    || !batchContext.batch_id
    || !Number.isSafeInteger(batchContext.revision)
    || (useStoreCheckedSelection
      ? !Number.isSafeInteger(checkedCount) || checkedCount < 1 || checkedCount > 250
      : itemIds.length > 250 || new Set(itemIds).size !== itemIds.length || itemIds.some((id) => !id.trim()))
    || !taskId
    || !requestId
    || !assetKeys.length
  ) {
    return { ok: false, code: "V3_APPLICATION_INVALID_REQUEST", error: "下载批次或 L1C 波段选择不完整，请重新选择。" };
  }
  const createS2L1cBatch = getDesktopShellBridge().application.createSentinel2L1cBandBatchTask;
  if (!createS2L1cBatch) {
    return { ok: false, code: "V3_APPLICATION_UNAVAILABLE", error: "当前桌面宿主不支持 Sentinel-2 L1C 分波段任务创建。" };
  }
  const result = await createS2L1cBatch({
    request_id: requestId,
    source_batch_id: batchContext.batch_id,
    expected_workbench_revision: batchContext.revision!,
    item_ids: [...itemIds],
    asset_keys: [...assetKeys],
    task_id: taskId,
    output_root: outputDir,
    layout: "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1)),
    created_at: new Date().toISOString(),
  });
  const expectedBatchId = `sentinel2-l1c-bands-batch-${taskId}`;
  if (!result.ok) {
    const recovered = await recoverOpticalCreationFailure(result, taskId, expectedBatchId);
    if (!recovered.ok) return recovered;
    const admission = await recoverOpticalDeliveryAdmission(taskId);
    return admission ? { ...recovered, admission } : recovered;
  }
  if (useStoreCheckedSelection && result.admission.requested_count !== checkedCount) {
    return m1LegacyCommandError(applicationProjectionError());
  }
  if (result.snapshot === null) {
    return {
      ok: false,
      code: "OPTICAL_ADMISSION_ALL_REJECTED",
      error: "所有影像 L1C 波段定位失败，未创建任务。",
      admission: result.admission,
    };
  }
  const receipt = await opticalTaskCreationReceiptOrRecover(result.snapshot, taskId, expectedBatchId);
  return receipt.ok ? { ...receipt, admission: result.admission } : receipt;
}

export const HLS_L30_SUPPORTED_BANDS: readonly string[] = Object.freeze([
  "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B09", "B10", "B11", "Fmask",
]);

export const HLS_S30_SUPPORTED_BANDS: readonly string[] = Object.freeze([
  "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B10", "B11", "B12", "Fmask",
]);

export async function startHlsBandBatchDownload(
  outputDir: string,
  itemIds: readonly string[],
  assetKeys: readonly string[],
  batchContext: DownloadBatchContext,
  taskId: string,
  requestId: string,
  maxConcurrent = 2,
): Promise<{
  ok: boolean;
  task_id?: string;
  download_status?: DownloadStatus;
  error?: string;
  code?: string;
  admission?: import("./desktopShellBridge").DesktopOpticalDeliveryAdmission;
}> {
  const useStoreCheckedSelection = itemIds.length === 0;
  const checkedCount = batchContext.checked_count ?? 0;
  if (
    !outputDir.trim()
    || !batchContext.batch_id
    || !Number.isSafeInteger(batchContext.revision)
    || (useStoreCheckedSelection
      ? !Number.isSafeInteger(checkedCount) || checkedCount < 1 || checkedCount > 250
      : itemIds.length > 250 || new Set(itemIds).size !== itemIds.length || itemIds.some((id) => !id.trim()))
    || !taskId
    || !requestId
    || !assetKeys.length
  ) {
    return { ok: false, code: "V3_APPLICATION_INVALID_REQUEST", error: "下载批次或波段选择不完整，请重新选择。" };
  }
  const createHlsBatch = getDesktopShellBridge().application.createHlsBandBatchTask;
  if (!createHlsBatch) {
    return { ok: false, code: "V3_APPLICATION_UNAVAILABLE", error: "当前桌面宿主不支持 HLS 多波段任务创建。" };
  }
  const result = await createHlsBatch({
    request_id: requestId,
    source_batch_id: batchContext.batch_id,
    expected_workbench_revision: batchContext.revision!,
    item_ids: [...itemIds],
    asset_keys: [...assetKeys],
    task_id: taskId,
    output_root: outputDir,
    layout: "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1)),
    created_at: new Date().toISOString(),
  });
  const expectedBatchId = `hls-bands-batch-${taskId}`;
  if (!result.ok) {
    const recovered = await recoverOpticalCreationFailure(result, taskId, expectedBatchId);
    if (!recovered.ok) return recovered;
    const admission = await recoverOpticalDeliveryAdmission(taskId);
    return admission ? { ...recovered, admission } : recovered;
  }
  if (useStoreCheckedSelection && result.admission.requested_count !== checkedCount) {
    return m1LegacyCommandError(applicationProjectionError());
  }
  if (result.snapshot === null) {
    return {
      ok: false,
      code: "OPTICAL_ADMISSION_ALL_REJECTED",
      error: "所有影像准备失败，未创建任务。",
      admission: result.admission,
    };
  }
  const receipt = await opticalTaskCreationReceiptOrRecover(result.snapshot, taskId, expectedBatchId);
  return receipt.ok ? { ...receipt, admission: result.admission } : receipt;
}

export async function startSentinel2BandDownload(
  outputDir: string,
  itemId: string,
  assetKeys: readonly string[],
  batchContext: DownloadBatchContext | null,
  taskId: string,
  requestId: string,
  maxConcurrent = 4,
): Promise<{
  ok: boolean;
  error?: string;
  code?: string;
  task_id?: string;
  workbench?: DownloadTaskWorkbench;
  search_workbench?: ApplicationWorkbenchProjection;
  download_status?: DownloadStatus;
}> {
  const uniqueAssetKeys = [...new Set(assetKeys.map((key) => key.trim()).filter(Boolean))];
  if (!outputDir || !itemId || !batchContext || !taskId || !requestId || uniqueAssetKeys.length === 0) {
    return {
      ok: false,
      error: "Sentinel-2 分析波段下载需要一个已冻结场景和至少一个已审核波段。",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  const result = await createApplicationSentinel2BandTask({
    request_id: requestId,
    source_batch_id: batchContext.batch_id,
    item_id: itemId,
    asset_keys: uniqueAssetKeys,
    task_id: taskId,
    output_root: outputDir,
    layout: "direct",
    concurrency: Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1)),
    created_at: new Date().toISOString(),
  });
  const expectedBatchId = `sentinel2-bands-${taskId}`;
  if (!result.ok) return recoverOpticalCreationFailure(result, taskId, expectedBatchId);
  return opticalTaskCreationReceiptOrRecover(result, taskId, expectedBatchId);
}

export type {
  DesktopOpticalDerivedCreateTaskRequest,
  DesktopOpticalDerivedPrecheckBatchRequest,
  DesktopOpticalPrecheckReport,
  DesktopOpticalPrecheckResult,
};

export async function precheckOpticalAoiCoverage(
  request: DesktopOpticalDerivedPrecheckBatchRequest,
): Promise<DesktopOpticalPrecheckResult> {
  return getDesktopShellBridge().application.precheckOpticalAoiCoverage(request);
}

export async function startOpticalDerivedClipOrMosaicTask(
  request: DesktopOpticalDerivedCreateTaskRequest,
): Promise<{
  ok: boolean;
  error?: string;
  code?: string;
  task_id?: string;
  derived_batch_id?: string;
  derived_item_id?: string;
  output_tif_path?: string;
  output_tif_paths?: string[];
  precheck?: DesktopOpticalPrecheckReport;
  download_status?: DownloadStatus;
}> {
  const expectedBatchId = `optderived-${request.task_id}`;
  const result = await getDesktopShellBridge().application.createOpticalDerivedTask(request);
  if (!result.ok) {
    return recoverOpticalCreationFailure(result, request.task_id, expectedBatchId);
  }
  const receipt = await opticalTaskCreationReceiptOrRecover(
    result.snapshot,
    request.task_id,
    result.derived_batch_id,
  );
  return receipt.ok
    ? {
        ...receipt,
        precheck: result.precheck,
        derived_batch_id: result.derived_batch_id,
        derived_item_id: result.derived_item_id,
        output_tif_path: result.output_tif_path,
        output_tif_paths: result.output_tif_paths,
      }
    : receipt;
}

export async function startNisarRemoteSubsetDownload(
  outputDir: string,
  itemId: string,
  frequency: "A" | "B",
  selectedPolarizations: string[],
  batchContext: DownloadBatchContext | null,
  taskId: string,
  requestId: string,
  maxConcurrent = 2,
  organizeByProduct = false,
): Promise<{
  ok: boolean;
  error?: string;
  code?: string;
  task_id?: string;
  workbench?: DownloadTaskWorkbench;
  search_workbench?: ApplicationWorkbenchProjection;
  download_status?: DownloadStatus;
}> {
  if (
    !isTauriDesktopHost()
    || !outputDir
    || !itemId
    || !batchContext
    || !taskId
    || !requestId
    || !batchContext.aoi_geojson
    || !Array.isArray(selectedPolarizations)
    || selectedPolarizations.length === 0
  ) {
    return {
      ok: false,
      error: "NISAR Remote Subset requires one frozen Store item, AOI, and polarization",
      code: "NISAR_SUBSET_REQUEST_INVALID",
    };
  }
  const boundedConcurrency = Math.min(8, Math.max(1, Math.trunc(maxConcurrent) || 1));
  const prepared = await prepareNisarSubsetIntentV2({
    source_batch_id: batchContext.batch_id,
    source_item_id: itemId,
    expected_workbench_revision: batchContext.revision ?? 0,
    subset_draft_geometry: batchContext.aoi_geojson,
    selected_frequency: frequency,
    selected_polarizations: [...new Set(selectedPolarizations)].sort(),
  });
  if (!prepared.ok) return m1LegacyCommandError(prepared);
  const result = await confirmNisarSubsetTaskV2({
    source_batch_id: prepared.prepared.source_batch_id,
    source_item_id: prepared.prepared.source_item_id,
    expected_workbench_revision: prepared.prepared.workbench_revision,
    prepared_intent: prepared.prepared.intent,
    task_id: taskId,
    output_root: outputDir,
    layout: organizeByProduct ? "sar_dataset_processing" : "direct",
    concurrency: boundedConcurrency,
    created_at: new Date().toISOString(),
  });
  if (!result.ok) return m1LegacyCommandError(result);
  const started = await startNewlyConfirmedTask(taskId);
  if (!started.ok) return m1LegacyCommandError(started);
  const bounded = await taskAndSearchWorkbenchFromCoreSnapshot(started, taskId, [itemId], batchContext);
  const downloadStatus = createdDownloadStatusFromCoreSnapshot(started, taskId);
  return bounded.ok
    ? downloadStatus
      ? { ok: true, task_id: taskId, workbench: bounded.workbench, search_workbench: bounded.search_workbench, download_status: downloadStatus }
      : m1LegacyCommandError(applicationProjectionError())
    : m1LegacyCommandError(bounded);
}

export async function appendAsfDownload(
  outputDir = "",
  maxExtraWorkers = 0,
  sceneIds: string[] = [],
  taskId = "",
): Promise<{ ok: boolean; error?: string; code?: string; appended?: number; skipped?: number; concurrency?: number }> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("append_asf_download");
  if (hasBridge()) return api().append_asf_download(outputDir, maxExtraWorkers, sceneIds, taskId);
  const n = sceneIds.length || 0;
  if (!n) return { ok: false, error: "请先勾选要追加下载的 SAR 影像", code: "ASF001" };
  mockDlState = {
    ...mockDlState,
    state: mockDlState.state === "idle" ? "running" : mockDlState.state,
    total: mockDlState.total + n,
    concurrency: Math.max(mockDlState.concurrency ?? 1, (mockDlState.concurrency ?? 1) + Math.min(n, Math.max(0, maxExtraWorkers))),
  };
  return { ok: true, appended: n, skipped: 0, concurrency: mockDlState.concurrency ?? 1 };
}

export async function appendAsfDownloadSnapshot(
  outputDir = "",
  scenesSnapshot: SceneRow[] = [],
  sceneIds: string[] = [],
  maxExtraWorkers = 0,
  taskId = "",
  batchContext: DownloadBatchContext | null = null,
): Promise<{ ok: boolean; error?: string; code?: string; appended?: number; skipped?: number; concurrency?: number }> {
  if (isTauriDesktopHost()) {
    if (!taskId || !batchContext || !sceneIds.length) {
      return { ok: false, error: "追加下载 requires the active Store Workbench selection", code: "V3_APPLICATION_INVALID_REQUEST" };
    }
    const result = await getDesktopShellBridge().application.appendTaskFromWorkbench({
      task_id: taskId,
      batch_id: batchContext.batch_id,
      expected_workbench_revision: batchContext.revision ?? 0,
      event_id: m1EventId("append_workbench", taskId),
      occurred_at: new Date().toISOString(),
    });
    if (!result.ok) return m1LegacyCommandError(result);
    const projected = projectApplicationCoreSnapshot(result);
    if (!projected.ok) return m1LegacyCommandError(projected);
    const task = projected.core.tasks.find((candidate) => candidate.task_id === taskId);
    return task
      ? { ok: true, appended: sceneIds.length, skipped: 0 }
      : { ok: false, error: "Task summary is unavailable after append", code: "V3_APPLICATION_INVALID_SNAPSHOT" };
  }
  if (hasBridge()) {
    const appender = api().append_asf_download_snapshot;
    if (typeof appender === "function") {
      return appender(outputDir, scenesSnapshot, sceneIds, maxExtraWorkers, taskId);
    }
    return api().append_asf_download(outputDir, maxExtraWorkers, sceneIds, taskId);
  }
  return appendAsfDownload(outputDir, maxExtraWorkers, sceneIds, taskId);
}

export async function pauseAsfScenes(sceneIds: string[] = [], taskId = ""): Promise<{ ok: boolean; error?: string; code?: string; paused?: number }> {
  if (isTauriDesktopHost()) {
    const itemIds = sceneIds.filter(Boolean);
    if (!taskId || !itemIds.length) return { ok: false, error: "当前页没有可暂停的冻结条目", code: "V3_APPLICATION_INVALID_REQUEST" };
    const result = await m1UpdateTask(taskId, "pause_items", itemIds);
    return result.ok ? { ok: true, paused: itemIds.length } : m1LegacyCommandError(result);
  }
  if (hasBridge()) return api().pause_asf_scenes(sceneIds, taskId);
  const paused = new Set([...(mockDlState.paused_scene_ids ?? []), ...sceneIds]);
  mockDlState = { ...mockDlState, paused_scene_ids: Array.from(paused), state: "paused", paused: true };
  return { ok: true, paused: sceneIds.length };
}

export async function resumeAsfScenes(sceneIds: string[] = [], taskId = ""): Promise<{ ok: boolean; error?: string; code?: string; resumed?: number }> {
  if (isTauriDesktopHost()) {
    const itemIds = sceneIds.filter(Boolean);
    if (!taskId || !itemIds.length) return { ok: false, error: "当前页没有可继续的冻结条目", code: "V3_APPLICATION_INVALID_REQUEST" };
    const result = await m1UpdateTask(taskId, "resume_items", itemIds);
    return result.ok ? { ok: true, resumed: itemIds.length } : m1LegacyCommandError(result);
  }
  if (hasBridge()) return api().resume_asf_scenes(sceneIds, taskId);
  const targets = new Set(sceneIds);
  mockDlState = {
    ...mockDlState,
    paused_scene_ids: (mockDlState.paused_scene_ids ?? []).filter((id) => !targets.has(id)),
    state: "running",
    paused: false,
  };
  return { ok: true, resumed: sceneIds.length };
}

export async function pauseAsfDownload(taskId = ""): Promise<{ ok: boolean; error?: string; code?: string }> {
  if (isTauriDesktopHost()) {
    const result = await m1UpdateTask(taskId, "pause_task", []);
    return result.ok ? { ok: true } : m1LegacyCommandError(result);
  }
  if (hasBridge()) return api().pause_asf_download(taskId);
  if (mockDlState.state !== "running") return { ok: false, error: "当前没有进行中的下载", code: "GUI004" };
  mockDlState = { ...mockDlState, state: "paused", paused: true };
  return { ok: true };
}

export async function resumeAsfDownload(taskId = ""): Promise<{ ok: boolean; error?: string; code?: string }> {
  if (isTauriDesktopHost()) {
    const result = await m1UpdateTask(taskId, "resume_task", []);
    return result.ok ? { ok: true } : m1LegacyCommandError(result);
  }
  if (hasBridge()) return api().resume_asf_download(taskId);
  if (mockDlState.state !== "paused") return { ok: false, error: "下载未处于暂停状态", code: "GUI004" };
  mockDlState = { ...mockDlState, state: "running", paused: false };
  return { ok: true };
}

export async function stopAsfDownload(taskId = ""): Promise<{ ok: boolean; error?: string; code?: string }> {
  if (isTauriDesktopHost()) {
    const result = await m1UpdateTask(taskId, "cancel_task", []);
    return result.ok ? { ok: true } : m1LegacyCommandError(result);
  }
  if (hasBridge()) return api().stop_asf_download(taskId);
  mockDlState = { ...mockDlState, state: "cancelled", cancelled: true };
  return { ok: true };
}

export async function retryAsfDownload(taskId = ""): Promise<{ ok: boolean; error?: string; code?: string }> {
  if (isTauriDesktopHost()) {
    const current = await coreTaskForId(taskId, true);
    if (!current.ok) return m1LegacyCommandError(current);
    if (current.task.failed_count + current.task.cancelled_count === 0) {
      return { ok: false, error: "No failed or cancelled items are eligible for retry", code: "V3_APPLICATION_INVALID_REQUEST" };
    }
    const result = await m1UpdateTask(taskId, "retry_task", []);
    return result.ok ? { ok: true } : m1LegacyCommandError(result);
  }
  if (hasBridge()) return api().retry_asf_download(taskId);
  if (!mockDlState.has_failures) return { ok: false, error: "当前没有失败或中断的 ASF 场景可重试", code: "GUI004" };
  mockDlState = { ...mockDlState, state: "running", paused: false, cancelled: false };
  return { ok: true };
}

export async function getDownloadStatus(): Promise<DownloadStatus> {
  if (isTauriDesktopHost()) {
    // Status hydration is strictly mutable/live; terminal history has its own page.
    const result = await restoreApplicationCore(false);
    if (!result.ok) {
      return {
        ok: false,
        state: "restore_error",
        total: 0,
        done: 0,
        current_scene: "",
        paused: false,
        cancelled: false,
        error: result.error,
        summary_line: result.error,
        results_path: "",
        log: [],
        restore_error: { code: result.code ?? null, error: result.error },
      };
    }
    const taskSummaries = result.core.tasks.filter((task) => task.operation?.kind !== "dem");
    const tasks: DownloadStatus[] = taskSummaries.map(projectDownloadStatusFromCoreTask);
    const activeTaskCount = tasks.filter((task) => task.ok && task.state === "running").length;
    const pausedTaskCount = tasks.filter((task) => task.ok && task.state === "paused").length;
    const failedTaskCount = tasks.filter((task) => task.ok && task.has_failures).length;
    const safeIntegerSum = (values: readonly number[]): number | null => values.reduce<number | null>(
      (sum, value) => sum === null || !Number.isSafeInteger(value) || !Number.isSafeInteger(sum + value)
        ? null
        : sum + value,
      0,
    );
    const safeFiniteSum = (values: readonly number[]): number | null => values.reduce<number | null>(
      (sum, value) => sum === null || !Number.isFinite(value) || !Number.isFinite(sum + value)
        ? null
        : sum + value,
      0,
    );
    const total = safeIntegerSum(tasks.map((task) => task.ok ? task.total : 0));
    const done = safeIntegerSum(tasks.map((task) => task.ok ? task.done : 0));
    const invalidTelemetry = () => ({
      ok: false,
      state: "restore_error",
      total: 0,
      done: 0,
      current_scene: "",
      paused: false,
      cancelled: false,
      error: "下载遥测字节总计超出安全范围",
      summary_line: "下载遥测字节总计无效",
      results_path: "",
      log: [],
      restore_error: { code: "V3_APPLICATION_INVALID_TELEMETRY", error: "下载遥测字节总计无效" },
    } satisfies DownloadStatus);
    if (total === null || done === null) return invalidTelemetry();
    const state = activeTaskCount > 0
      ? "running"
      : pausedTaskCount > 0
        ? "paused"
        : failedTaskCount > 0
          ? "failed"
          : tasks.length > 0 && done >= total
            ? "finished"
            : "idle";
    const taskErrors = tasks
      .filter((task): task is DownloadStatusError => !task.ok)
      .map((task) => ({ task_id: undefined, error: task.error ?? "下载任务状态读取失败", code: task.restore_error?.code ?? null }));
    const allTotalsKnown = taskSummaries.every((task) => task.total_bytes !== null);
    const knownTotalBytes = allTotalsKnown
      ? safeIntegerSum(taskSummaries.map((task) => task.total_bytes!))
      : null;
    const totalBytes = allTotalsKnown
      ? knownTotalBytes
      : null;
    const doneBytes = safeIntegerSum(taskSummaries.map((task) => task.terminal_bytes));
    const currentBytes = safeIntegerSum(taskSummaries.flatMap((task) => [task.active_bytes, task.other_bytes]));
    const transferredBytes = safeIntegerSum(taskSummaries.map((task) => task.transferred_bytes));
    const bytesPerSecond = safeFiniteSum(taskSummaries.map((task) => task.bytes_per_second));
    if (knownTotalBytes === null && allTotalsKnown || doneBytes === null || currentBytes === null || transferredBytes === null || bytesPerSecond === null ||
      !Number.isSafeInteger(doneBytes + currentBytes) || doneBytes + currentBytes !== transferredBytes) {
      return invalidTelemetry();
    }
    const compareCodePoints = (left: string, right: string): number => {
      const leftPoints = Array.from(left);
      const rightPoints = Array.from(right);
      for (let index = 0; index < Math.min(leftPoints.length, rightPoints.length); index += 1) {
        const difference = leftPoints[index]!.codePointAt(0)! - rightPoints[index]!.codePointAt(0)!;
        if (difference !== 0) return difference;
      }
      return leftPoints.length - rightPoints.length;
    };
    const representative = taskSummaries
      .filter((task) => task.active_item !== null)
      .sort((left, right) => compareCodePoints(left.task_id, right.task_id))[0]?.active_item ?? null;
    return {
      ok: true,
      state,
      total,
      done,
      current_scene: representative?.item_id ?? "",
      active_downloads: representative
        ? [{ scene_id: representative.item_id, bytes: representative.bytes_completed, expected_size: representative.total_bytes }]
        : [],
      total_bytes: totalBytes,
      done_bytes: doneBytes,
      current_bytes: currentBytes,
      transferred_bytes: transferredBytes,
      bytes_per_second: bytesPerSecond,
      paused: state === "paused",
      cancelled: false,
      error: taskErrors.length ? taskErrors.map((entry) => entry.error).join("；") : null,
      summary_line: tasks.length
        ? `${done}/${total}（${tasks.length} 个任务；运行 ${activeTaskCount}，暂停 ${pausedTaskCount}）`
        : "",
      results_path: "",
      asf_tasks: tasks,
      active_task_count: activeTaskCount,
      task_count: tasks.length,
      task_errors: taskErrors,
      log: tasks.flatMap((task) => task.ok ? task.log : []),
    };
  }
  if (hasBridge()) return api().get_download_status();
  return mockDlState;
}

async function planFormalDemTask(
  outputDir: string,
  dataset: string,
  requestedBbox?: Bbox,
  requestedProvider = "auto",
): Promise<PlanReportResult> {
  const context = await getContext();
  if (!context.ok) return context;
  if (!context.workspace || !context.project || !context.region || !context.region.has_aoi || !context.region.bbox) {
    return { ok: false, error: "请先绑定当前研究区 AOI", code: "AOI001" };
  }
  if (
    requestedBbox &&
    [
      requestedBbox.west !== context.region.bbox.west,
      requestedBbox.east !== context.region.bbox.east,
      requestedBbox.south !== context.region.bbox.south,
      requestedBbox.north !== context.region.bbox.north,
    ].some(Boolean)
  ) {
    return { ok: false, error: "手工 bbox 与当前研究区持久化 AOI 不一致，请先绑定该 AOI", code: "AOI001" };
  }
  const root = outputDir.trim() || context.region.root || context.project.root || context.workspace.root;
  const identity = formalDemIdentity(dataset, requestedProvider);
  if (!identity) {
    return { ok: false, error: "该 DEM 数据集未在 ResourceRegistry 中登记正式任务路由", code: "RESOURCE_REGISTRY_UNAVAILABLE" };
  }
  const taskId = stableUiNodeId("dem-plan", `${context.region.region_id}-${dataset}`);
  const plan = await planApplicationDemTask({
    request_id: `dem-plan-${taskId}`,
    task_id: taskId,
    region_id: context.region.region_id,
    resource_id: identity.resource_id,
    capability_id: identity.capability_id,
    dataset: identity.dataset,
    provider: identity.provider,
    operation: "prepare_sarscape",
    input_path: joinDemPath(root, `${demSourceStem(dataset)}.tif`),
    output_root: root,
    created_at: new Date().toISOString(),
  });
  if (!plan.ok) return plan;
  return {
    ok: true,
    plan: {
      request_id: plan.plan.request_id,
      task_id: plan.plan.task_id,
      region_id: plan.plan.region_id,
      resource_id: plan.plan.resource_id,
      capability_id: plan.plan.capability_id,
      dataset: plan.plan.dataset,
      provider: plan.plan.provider,
      input_path: plan.plan.input_path,
      output_root: plan.plan.output_root,
      source_vertical_datum: plan.plan.source_vertical_datum ?? null,
      target_vertical_datum: plan.plan.target_vertical_datum ?? null,
      output_mode: plan.plan.output_mode ?? null,
      operation: plan.plan.operation,
      created_at: plan.plan.created_at,
      raw_dem_path: plan.plan.input_path,
      sarscape_ready_dem_path: joinDemPath(plan.plan.output_root, `${demSourceStem(dataset)}_dem`),
    },
    report: {
      source: "ApplicationService.ResourceRegistry.runtime_projection",
      task_route: "Store/Scheduler/Download Engine",
      provider_network: false,
    },
  };
}

export async function planDemDownload(
  outputDir = "",
  dataset = "COP30",
  provider = "auto",
): Promise<PlanReportResult> {
  if (isTauriDesktopHost()) return planFormalDemTask(outputDir, dataset, undefined, provider);
  if (hasBridge()) return api().plan_dem_download(outputDir, dataset);
  if (!mock.region?.has_aoi) {
    return { ok: false, error: "请先在『区域 AOI』设置处理范围（bbox）", code: "AOI001" };
  }
  mockDataset = dataset;
  return { ok: true, plan: mockDemPlan(dataset), report: mockReport() };
}

export async function planDemDownloadBbox(
  west: number,
  east: number,
  south: number,
  north: number,
  outputDir = "",
  dataset = "COP30",
  provider = "auto",
): Promise<PlanReportResult> {
  if (isTauriDesktopHost()) {
    if (!(west < east) || !(south < north)) {
      return { ok: false, error: "bbox 范围无效，请检查 west/east/south/north", code: "AOI001" };
    }
    const identity = formalDemIdentity(dataset, provider);
    if (!identity) {
      return { ok: false, error: "该 DEM 数据集未在 ResourceRegistry 中登记正式任务路由", code: "RESOURCE_REGISTRY_UNAVAILABLE" };
    }
    const root = standaloneDemOutputRoot(outputDir);
    const bbox = { west, east, south, north, crs: "EPSG:4326" } satisfies Bbox;
    const planned = await planStandaloneDemConversion(
      joinDemPath(root, `${demSourceStem(identity.dataset)}_dem.tif`),
      root,
      identity.dataset,
      identity.provider,
      "auto",
      bbox,
    );
    if (!planned.ok) return planned;
    return {
      ok: true,
      plan: planned.plan,
      report: {
        ...planned.report,
        task_route: "Store/Scheduler/Download Engine",
        standalone: true,
      },
    };
  }
  if (hasBridge()) {
    return api().plan_dem_download_bbox(west, east, south, north, outputDir, dataset);
  }
  if (!(west < east) || !(south < north)) {
    return { ok: false, error: "bbox 范围无效，请检查 west/east/south/north", code: "AOI001" };
  }
  mockDataset = dataset;
  const bbox: Bbox = { west, east, south, north, crs: "EPSG:4326" };
  return {
    ok: true,
    plan: mockDemPlan(dataset, bbox, "standalone_dem", outputDir || mock.workspace?.root || "C:\\InSAR"),
    report: mockReport(),
  };
}

export async function runDemDownload(
  outputDir = "",
  dataset = "COP30",
  keySource = "auto",
  convert = true,
  provider = "auto",
): Promise<RunSummaryResult> {
  if (isTauriDesktopHost()) {
    const task = await startDemDownload(outputDir, dataset, keySource, convert, provider);
    if (!task.ok) return task;
    return {
      ok: true,
      summary_line: `正式 DEM 任务 ${task.task_id} 已提交；转换由独立 conversion handoff 负责`,
      total: 1,
      succeeded: 0,
      skipped: 0,
      failed: 0,
      interrupted: 0,
      has_failures: false,
      results_path: "",
      results: [{ task_id: task.task_id, state: task.state }],
    };
  }
  if (hasBridge()) return api().run_dem_download(outputDir, dataset, keySource, convert);
  const plan = await planDemDownload(outputDir, dataset);
  if (!plan.ok) return plan;
  if (!convert) {
    return {
      ok: true,
      summary_line: "1 downloaded, 0 skipped, 0 failed, 0 interrupted",
      total: 1,
      succeeded: 1,
      skipped: 0,
      failed: 0,
      interrupted: 0,
      has_failures: false,
      results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_download_results.txt`,
      results: [
        {
          region_safe_name: mock.region?.safe_name ?? "region",
          dataset,
          outcome: "success",
          bytes_written: 1024,
          message: "mock DEM ok",
        },
      ],
      raw_dem_path: String(plan.plan.raw_dem_path ?? ""),
      ellipsoid_dem_path: "",
      sarscape_ready_dem_path: "",
    };
  }
  const conversion = await runDemConversion(outputDir);
  return {
    ok: true,
    summary_line: `下载：1 downloaded, 0 skipped, 0 failed, 0 interrupted；转换：${
      conversion.ok ? conversion.summary_line : "未执行"
    }`,
    total: 1,
    succeeded: 1,
    skipped: 0,
    failed: 0,
    interrupted: 0,
    has_failures: conversion.ok ? conversion.has_failures : true,
    results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_download_results.txt`,
    results: [
      {
        region_safe_name: mock.region?.safe_name ?? "region",
        dataset,
        outcome: "success",
        bytes_written: 1024,
        message: "mock DEM ok",
      },
    ],
    download: {
      summary_line: "1 downloaded, 0 skipped, 0 failed, 0 interrupted",
      results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_download_results.txt`,
    },
    conversion: conversion.ok ? conversion : null,
    conversion_results_path: conversion.ok ? conversion.results_path : "",
    raw_dem_path: String(plan.plan.raw_dem_path ?? ""),
    ellipsoid_dem_path: String(plan.plan.ellipsoid_dem_path ?? ""),
    sarscape_ready_dem_path: String(plan.plan.sarscape_ready_dem_path ?? ""),
  };
}

const mockDemTasks = new Map<string, DemTaskStatus>();
let mockDemTaskSequence = 0;
let mockDemTaskSettings: DemTaskSettings = {
  ok: true,
  max_concurrent_tasks: 2,
  max_concurrent_conversions: 1,
};

function demDevelopmentMockEnabled(): boolean {
  const nodeEnv = (globalThis as { process?: { env?: Record<string, string | undefined> } })
    .process?.env?.NODE_ENV;
  return (typeof import.meta.env !== "undefined" && import.meta.env.DEV) || nodeEnv === "test";
}

function demProtocolError(method: string): ApiError {
  return {
    ok: false,
    error: `Desktop API does not expose ${method}`,
    code: "GUI003",
  };
}

function demTaskError(error: string, code = "GUI004"): DemTaskCommandResult {
  return { ok: false, error, code };
}

function validateDemTaskCommandResult(
  result: unknown,
  method: string,
): DemTaskCommandResult {
  if (typeof result !== "object" || result === null) {
    return demProtocolError(method);
  }
  const envelope = result as Record<string, unknown>;
  if (envelope.ok === false) return result as ApiError;
  if (
    envelope.ok !== true ||
    typeof envelope.task_id !== "string" ||
    envelope.task_id.trim() === "" ||
    typeof envelope.state !== "string" ||
    envelope.state.trim() === ""
  ) {
    return {
      ok: false,
      error: `Desktop API ${method} returned ok without non-empty task_id and state`,
      code: "GUI003",
    };
  }
  return {
    ok: true,
    task_id: envelope.task_id.trim(),
    state: envelope.state.trim(),
  };
}

function mockDemActiveCount(): number {
  return Array.from(mockDemTasks.values()).filter(
    (task) => task.state === "running" || task.state === "cancelling",
  ).length;
}

function scheduleMockDemTasks(): void {
  let active = mockDemActiveCount();
  for (const task of mockDemTasks.values()) {
    if (active >= mockDemTaskSettings.max_concurrent_tasks) break;
    if (task.state !== "queued") continue;
    task.state = "running";
    task.current_scene = task.dataset;
    task.worker_alive = true;
    task.summary_line = `正在下载 ${task.dataset}`;
    active += 1;
  }
}

function mockDemManagerStatus(): DemManagerStatus {
  const queued = Array.from(mockDemTasks.values()).filter((task) => task.state === "queued");
  const queuePositions = new Map(queued.map((task, index) => [task.task_id, index + 1]));
  const tasks = Array.from(mockDemTasks.values()).map((task) => ({
    ...task,
    max_concurrent_tasks: mockDemTaskSettings.max_concurrent_tasks,
    max_concurrent_conversions: mockDemTaskSettings.max_concurrent_conversions,
    queue_position: queuePositions.get(task.task_id) ?? 0,
    task_snapshot: { ...task.task_snapshot, bbox: { ...task.task_snapshot.bbox } },
    results: [...task.results],
    log: [...task.log],
  }));
  return {
    ok: true,
    dem_tasks: tasks,
    active_task_count: tasks.filter(
      (task) => task.state === "running" || task.state === "cancelling",
    ).length,
    paused_task_count: tasks.filter((task) => task.state === "paused").length,
    queued_task_count: tasks.filter((task) => task.state === "queued").length,
    max_concurrent_tasks: mockDemTaskSettings.max_concurrent_tasks,
    max_concurrent_conversions: mockDemTaskSettings.max_concurrent_conversions,
  };
}

function startMockDemDownload(
  outputDir: string,
  dataset: string,
  keySource: string,
  convert: boolean,
  bbox?: Bbox,
  requestedProvider?: string,
): DemTaskCommandResult {
  mockDemTaskSequence += 1;
  const taskId = `dem-mock-${Date.now()}-${mockDemTaskSequence}`;
  const root = outputDir || mock.workspace?.root || "C:\\InSAR";
  const state = mockDemActiveCount() < mockDemTaskSettings.max_concurrent_tasks ? "running" : "queued";
  const taskBbox = bbox ?? mock.region?.bbox ?? {
    west: 110.22,
    east: 110.52,
    south: 30.92,
    north: 31.14,
    crs: "EPSG:4326",
  };
  const createdAt = Date.now();
  const provider = resolvedDemProvider(dataset, requestedProvider) ?? "OPENTOPOGRAPHY";
  const snapshot: DemTaskSnapshot = {
    task_id: taskId,
    provider,
    dataset,
    bbox: { ...taskBbox, crs: taskBbox.crs || "EPSG:4326" },
    source_vertical_datum: mockSourceDatum(dataset),
    target_vertical_datum: "WGS84_ELLIPSOID",
    output_root: root,
    convert_policy: convert ? "convert" : "download_only",
    source_policy: keySource,
    proxy_policy: "system",
    schema_version: 1,
    created_at: createdAt,
    resume_supported: provider === "COPERNICUS_AWS",
  };
  mockDemTasks.set(taskId, {
    ok: true,
    task_id: taskId,
    name: "DEM 下载任务",
    state,
    total: 1,
    done: 0,
    current_scene: state === "running" ? dataset : "",
    output_dir: root,
    dataset,
    convert,
    raw_dem_path: `${root}\\${demSourceStem(dataset)}.tif`,
    ellipsoid_dem_path: convert ? `${root}\\${demSourceStem(dataset)}_ellipsoid.tif` : "",
    sarscape_ready_dem_path: convert ? `${root}\\${demSourceStem(dataset)}_dem` : "",
    results_path: "",
    conversion_results_path: "",
    done_bytes: 0,
    total_bytes: 0,
    bytes_per_second: 0,
    elapsed_seconds: 0,
    cancelled: false,
    error: null,
    summary_line: state === "running" ? `正在下载 ${dataset}` : "等待下载并发槽位",
    succeeded: 0,
    skipped: 0,
    failed: 0,
    interrupted: 0,
    has_failures: false,
    worker_alive: state === "running",
    queue_position: 0,
    created_sequence: mockDemTaskSequence,
    max_concurrent_tasks: mockDemTaskSettings.max_concurrent_tasks,
    max_concurrent_conversions: mockDemTaskSettings.max_concurrent_conversions,
    resume_supported: false,
    task_snapshot: snapshot,
    results: [],
    log: [
      {
        scene_id: dataset,
        outcome: state === "running" ? "started" : "queued",
        detail: state === "running" ? `开始 DEM 下载：${dataset}` : "DEM 任务已排队",
        ts: createdAt,
      },
    ],
  });
  return { ok: true, task_id: taskId, state };
}

async function formalTaskCommand(
  taskId: string,
  kind: DesktopUpdateDownloadTaskRequest["command"]["kind"],
): Promise<DemTaskCommandResult> {
  const current = await coreTaskForId(taskId, true);
  if (!current.ok) return current;
  if (kind === "cancel_items" || kind === "retry_items") {
    return {
      ok: false,
      error: "该正式任务命令需要精确条目 ID；当前 CoreRestore 只提供任务摘要，已拒绝枚举完整条目。",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  const result = await m1UpdateTask(taskId, kind, []);
  if (!result.ok) return result;
  const updated = result.core.tasks.find((candidate) => candidate.task_id === taskId);
  return { ok: true, task_id: taskId, state: updated?.status ?? "queued" };
}

async function startTauriDemForCurrentAoi(
  outputDir: string,
  dataset: string,
  convert: boolean,
  provider: string,
  expectedBbox?: Bbox,
): Promise<DemTaskCommandResult> {
  const context = await getContext();
  if (!context.ok) return context;
  const aoi = await exactOperationAoi(context);
  if (!aoi.ok) return aoi;
  if (expectedBbox) {
    const requested = [expectedBbox.west, expectedBbox.east, expectedBbox.south, expectedBbox.north];
    const authoritative = [aoi.bbox.west, aoi.bbox.east, aoi.bbox.south, aoi.bbox.north];
    if (!requested.every(Number.isFinite) || requested.some((value, index) => value !== authoritative[index])) {
      return {
        ok: false,
        error: "DEM 显式 bbox 与当前 Store 权威 AOI 不一致，请重新从当前 AOI 发起任务",
        code: "DEM_AOI_BBOX_STALE",
      };
    }
  }
  const identity = formalDemIdentity(dataset, provider);
  if (!identity) {
    return {
      ok: false,
      error: "该 DEM 数据集未在 ResourceRegistry 中登记正式任务路由",
      code: "RESOURCE_REGISTRY_UNAVAILABLE",
    };
  }
  const root = standaloneDemOutputRoot(outputDir);
  const taskId = stableUiNodeId(
    "dem-task-standalone",
    globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`,
    64,
  );
  return createApplicationStandaloneDemTask({
    request_id: `dem-standalone-${taskId}`,
    task_id: taskId,
    dataset: identity.dataset,
    provider: identity.provider,
    aoi_geojson: (recordValue(aoi.geometry) ?? {}) as Record<string, unknown>,
    aoi_name: aoi.display_name.trim(),
    output_root: root,
    convert,
    // This is per-task window concurrency. Whole DEM tasks are admitted by the
    // native scheduler from its persisted limit, never by this request field.
    concurrency: 1,
    created_at: new Date().toISOString(),
  });
}

export async function startDemDownload(
  outputDir = "",
  dataset = "COP30",
  keySource = "auto",
  convert = true,
  provider = "auto",
): Promise<DemTaskCommandResult> {
  if (isTauriDesktopHost()) {
    return startTauriDemForCurrentAoi(outputDir, dataset, convert, provider);
  }
  if (hasBridge()) {
    const method = api().start_dem_download;
    return typeof method === "function"
      ? validateDemTaskCommandResult(
          await method(outputDir, dataset, keySource, convert, provider),
          "start_dem_download",
        )
      : demProtocolError("start_dem_download");
  }
  if (!demDevelopmentMockEnabled()) return demProtocolError("start_dem_download");
  return startMockDemDownload(outputDir, dataset, keySource, convert, undefined, provider);
}

export async function startDemDownloadBbox(
  west: number,
  east: number,
  south: number,
  north: number,
  outputDir = "",
  dataset = "COP30",
  keySource = "auto",
  convert = true,
  provider = "auto",
): Promise<DemTaskCommandResult> {
  if (isTauriDesktopHost()) {
    return startTauriDemForCurrentAoi(outputDir, dataset, convert, provider, {
      west,
      east,
      south,
      north,
      crs: "EPSG:4326",
    });
  }
  if (hasBridge()) {
    const method = api().start_dem_download_bbox;
    return typeof method === "function"
      ? validateDemTaskCommandResult(
          await method(west, east, south, north, outputDir, dataset, keySource, convert, provider),
          "start_dem_download_bbox",
        )
      : demProtocolError("start_dem_download_bbox");
  }
  if (!demDevelopmentMockEnabled()) return demProtocolError("start_dem_download_bbox");
  if (!(west < east) || !(south < north)) {
    return demTaskError("bbox 范围无效，请检查 west/east/south/north", "AOI001");
  }
  return startMockDemDownload(
    outputDir,
    dataset,
    keySource,
    convert,
    {
      west,
      east,
      south,
      north,
      crs: "EPSG:4326",
    },
    provider,
  );
}

export async function stopDemDownload(taskId: string): Promise<DemTaskCommandResult> {
  const stableId = taskId.trim();
  if (!stableId) return demTaskError("DEM task_id 不能为空");
  if (isTauriDesktopHost()) return formalTaskCommand(stableId, "cancel_task");
  if (hasBridge()) {
    const method = api().stop_dem_download;
    return typeof method === "function"
      ? validateDemTaskCommandResult(await method(stableId), "stop_dem_download")
      : demProtocolError("stop_dem_download");
  }
  if (!demDevelopmentMockEnabled()) return demProtocolError("stop_dem_download");
  const task = mockDemTasks.get(stableId);
  if (!task) return demTaskError("未找到指定的 DEM 任务", "DEM_TASK_NOT_FOUND");
  if (["finished", "failed", "cancelled", "interrupted"].includes(task.state)) {
    return demTaskError("指定的 DEM 任务当前不可取消");
  }
  task.state = "cancelled";
  task.cancelled = true;
  task.worker_alive = false;
  task.current_scene = "";
  task.summary_line = "DEM 下载已中断";
  task.log.push({ scene_id: task.dataset, outcome: "cancelled", detail: "DEM 下载已中断", ts: Date.now() });
  scheduleMockDemTasks();
  return { ok: true, task_id: stableId, state: task.state };
}

export async function pauseDemDownload(taskId: string): Promise<DemTaskCommandResult> {
  const stableId = taskId.trim();
  if (!stableId) return demTaskError("DEM task_id 不能为空");
  if (isTauriDesktopHost()) return formalTaskCommand(stableId, "pause_task");
  if (hasBridge()) {
    const method = api().pause_dem_download;
    return typeof method === "function"
      ? validateDemTaskCommandResult(await method(stableId), "pause_dem_download")
      : demProtocolError("pause_dem_download");
  }
  if (!demDevelopmentMockEnabled()) return demProtocolError("pause_dem_download");
  const task = mockDemTasks.get(stableId);
  if (!task) return demTaskError("未找到指定的 DEM 任务", "DEM_TASK_NOT_FOUND");
  if (task.state !== "running" && task.state !== "queued") {
    return demTaskError("指定的 DEM 任务当前不可暂停");
  }
  task.state = "paused";
  task.worker_alive = task.worker_alive && task.current_scene !== "";
  task.current_scene = "";
  task.summary_line = "DEM 下载已暂停";
  scheduleMockDemTasks();
  return { ok: true, task_id: stableId, state: task.state };
}

export async function resumeDemDownload(taskId: string): Promise<DemTaskCommandResult> {
  const stableId = taskId.trim();
  if (!stableId) return demTaskError("DEM task_id 不能为空");
  if (isTauriDesktopHost()) return formalTaskCommand(stableId, "resume_task");
  if (hasBridge()) {
    const method = api().resume_dem_download;
    return typeof method === "function"
      ? validateDemTaskCommandResult(await method(stableId), "resume_dem_download")
      : demProtocolError("resume_dem_download");
  }
  if (!demDevelopmentMockEnabled()) return demProtocolError("resume_dem_download");
  const task = mockDemTasks.get(stableId);
  if (!task) return demTaskError("未找到指定的 DEM 任务", "DEM_TASK_NOT_FOUND");
  if (task.state !== "paused" && task.state !== "queued") {
    return demTaskError("指定的 DEM 任务当前未暂停");
  }
  task.state = "queued";
  task.worker_alive = false;
  task.summary_line = "等待下载并发槽位";
  scheduleMockDemTasks();
  return { ok: true, task_id: stableId, state: task.state };
}

export async function retryDemDownload(taskId: string): Promise<DemTaskCommandResult> {
  const stableId = taskId.trim();
  if (!stableId) return demTaskError("DEM task_id 不能为空");
  if (isTauriDesktopHost()) return formalTaskCommand(stableId, "retry_task");
  if (hasBridge()) {
    const method = api().retry_dem_download;
    return typeof method === "function"
      ? validateDemTaskCommandResult(await method(stableId), "retry_dem_download")
      : demProtocolError("retry_dem_download");
  }
  if (!demDevelopmentMockEnabled()) return demProtocolError("retry_dem_download");
  const task = mockDemTasks.get(stableId);
  if (!task) return demTaskError("未找到指定的 DEM 任务", "DEM_TASK_NOT_FOUND");
  if (!["failed", "cancelled", "interrupted"].includes(task.state)) {
    return demTaskError("指定的 DEM 任务当前不可重试");
  }
  task.state = "queued";
  task.cancelled = false;
  task.error = null;
  task.failed = 0;
  task.interrupted = 0;
  task.has_failures = false;
  task.summary_line = "等待下载并发槽位";
  scheduleMockDemTasks();
  return { ok: true, task_id: stableId, state: task.state };
}

function formalDemTaskStatus(
  task: FormalTaskProjection,
  region: ContextRegion,
  createdSequence: number,
  queuePosition: number,
): DemTaskStatus {
  const dataset = task.dataset_id || "DEM";
  const provider = task.operation?.source_provider || task.provider_id || "application";
  const outputRoot = task.output_root || region.root || "";
  const inputPath = task.operation?.input_path?.trim()
    || joinDemPath(outputRoot, `${demSourceStem(dataset)}.tif`);
  const convert = task.operation?.convert
    ?? (task.operation?.phase === "conversion"
      || task.operation?.approved_worker_operation === "dem_convert");
  const sourceVerticalDatum = task.operation?.source_vertical_datum?.trim()
    || standaloneDemSourceDatum(inputPath, dataset, "auto");
  const stem = standaloneDemStem(inputPath, dataset);
  const requiresVerticalConversion = convert && sourceVerticalDatum !== "WGS84_ELLIPSOID";
  const bbox = region.bbox ?? MOCK_BBOX;
  const done = task.completed_count + task.skipped_count;
  const failures = task.failed_count + task.cancelled_count;
  const total = Math.max(1, task.selected_count);
  const summaryLine = `${Math.min(total, done)}/${total}`;
  const taskSnapshot: DemTaskSnapshot = {
    task_id: task.task_id,
    provider,
    dataset,
    bbox,
    source_vertical_datum: sourceVerticalDatum,
    target_vertical_datum: "WGS84_ELLIPSOID",
    output_root: outputRoot,
    convert_policy: convert ? "convert" : "download_only",
    source_policy: "application",
    proxy_policy: "product_owned",
    schema_version: 1,
    created_at: Date.parse(task.created_at) / 1_000 || createdSequence,
    resume_supported: provider === "COPERNICUS_AWS",
  };
  const resultsPath = convert ? joinDemPath(outputRoot, `${stem}_conversion_results.json`) : "";
  return {
    ok: true,
    task_id: task.task_id,
    name: `DEM ${dataset} 正式任务`,
    revision: task.revision,
    state: task.status === "completed" ? "finished" : task.status,
    total,
    done: Math.min(total, done),
    current_scene: dataset,
    output_dir: outputRoot,
    dataset,
    convert,
    raw_dem_path: inputPath,
    ellipsoid_dem_path: requiresVerticalConversion ? joinDemPath(outputRoot, `${stem}_ellipsoid.tif`) : "",
    sarscape_ready_dem_path: convert ? joinDemPath(outputRoot, `${stem}_dem`) : "",
    results_path: resultsPath,
    conversion_results_path: convert ? resultsPath : "",
    // Network source bytes are distinct from the final cropped raster size.
    // Older hosts/tasks lack this measurement; keep their previous projection.
    done_bytes: task.network_bytes_received ?? task.transferred_bytes,
    total_bytes: task.network_bytes_received != null
      ? undefined : task.total_bytes ?? undefined,
    bytes_per_second: task.bytes_per_second,
    elapsed_seconds: task.transfer_active_millis / 1000,
    cancelled: task.status === "cancelled",
    error: null,
    summary_line: summaryLine,
    succeeded: task.completed_count,
    skipped: task.skipped_count,
    failed: task.failed_count,
    interrupted: task.cancelled_count,
    has_failures: failures > 0,
    worker_alive: task.status === "running",
    queue_position: queuePosition,
    created_sequence: createdSequence,
    max_concurrent_tasks: 1,
    max_concurrent_conversions: 1,
    resume_supported: provider === "COPERNICUS_AWS",
    task_snapshot: taskSnapshot,
    results: [],
    log: [],
  };
}

export async function getDemDownloadStatus(): Promise<DemManagerStatus | ApiError> {
  if (isTauriDesktopHost()) {
    const restored = await restoreApplicationCore(false);
    if (!restored.ok) return restored;
    const context = contextFromCoreRestore(restored.core, restored.aoi_display);
    const region: ContextRegion = context.ok && context.region
      ? context.region
      : {
          region_id: "standalone-dem",
          name: "独立 DEM",
          safe_name: "standalone_dem",
          root: undefined,
          has_aoi: false,
          bbox: null,
          scene_count: 0,
        };
    const tasks = restored.core.tasks.filter((task) => task.operation?.kind === "dem");
    const queued = tasks.filter((task) => task.status === "queued");
    const queuePositions = new Map(queued.map((task, index) => [task.task_id, index + 1]));
    const entries: DemTaskStatus[] = [];
    for (const [index, task] of tasks.entries()) {
      // List polling is fixed-summary only. Per-item/task snapshots are a
      // detail action, never an N+1 companion to the status hydrate.
      entries.push(formalDemTaskStatus(task, region, index + 1, queuePositions.get(task.task_id) ?? 0));
    }
    const settings = await getDemTaskSettings();
    if (!settings.ok) return settings;
    for (const task of entries) {
      task.max_concurrent_tasks = settings.max_concurrent_tasks;
      task.max_concurrent_conversions = settings.max_concurrent_conversions;
    }
    return {
      ok: true,
      dem_tasks: entries,
      active_task_count: entries.filter((task) => task.state === "running" || task.state === "cancelling").length,
      paused_task_count: entries.filter((task) => task.state === "paused").length,
      queued_task_count: entries.filter((task) => task.state === "queued").length,
      max_concurrent_tasks: settings.max_concurrent_tasks,
      max_concurrent_conversions: settings.max_concurrent_conversions,
    };
  }
  if (hasBridge()) {
    const method = api().get_dem_download_status;
    return typeof method === "function" ? method() : demProtocolError("get_dem_download_status");
  }
  return demDevelopmentMockEnabled()
    ? mockDemManagerStatus()
    : demProtocolError("get_dem_download_status");
}

export async function getDemTaskSettings(): Promise<DemTaskSettings | ApiError> {
  if (isTauriDesktopHost()) {
    const route = getDesktopShellBridge().application.getDemTaskSettings;
    if (!route) return demProtocolError("get_dem_task_settings");
    const result = await route();
    if ("ok" in result && result.ok === false) return applicationError(result);
    const settings = result as DesktopDemTaskSettings;
    if (
      !Number.isInteger(settings.max_concurrent_tasks) ||
      !Number.isInteger(settings.max_concurrent_conversions)
    ) {
      return demProtocolError("get_dem_task_settings");
    }
    return { ok: true, ...settings };
  }
  if (hasBridge()) {
    const method = api().get_dem_task_settings;
    return typeof method === "function" ? method() : demProtocolError("get_dem_task_settings");
  }
  return demDevelopmentMockEnabled()
    ? { ...mockDemTaskSettings }
    : demProtocolError("get_dem_task_settings");
}

export async function getSarDownloadPolicySettings(
): Promise<SarDownloadPolicySettings | ApiError> {
  const route = getDesktopShellBridge().application.getSarDownloadPolicySettings;
  if (!route) return demProtocolError("get_sar_download_policy_settings");
  const result = await route();
  if ("ok" in result && result.ok === false) return applicationError(result);
  const settings = result as DesktopSarDownloadPolicySettings;
  return { ok: true, ...settings };
}

export async function saveSarDownloadPolicySettings(
  settings: DesktopSarDownloadPolicySettings,
): Promise<SarDownloadPolicySettings | ApiError> {
  const route = getDesktopShellBridge().application.saveSarDownloadPolicySettings;
  if (!route) return demProtocolError("save_sar_download_policy_settings");
  const result = await route(settings);
  if ("ok" in result && result.ok === false) return applicationError(result);
  const saved = result as DesktopSarDownloadPolicySettings;
  return { ok: true, ...saved };
}

export async function saveDemTaskSettings(
  settings: { max_concurrent_tasks: number },
): Promise<DemTaskSettings | ApiError> {
  if (isTauriDesktopHost()) {
    const route = getDesktopShellBridge().application.saveDemTaskSettings;
    if (!route) return demProtocolError("save_dem_task_settings");
    const result = await route({
      max_concurrent_tasks: settings.max_concurrent_tasks,
      max_concurrent_conversions: 1,
    });
    if ("ok" in result && result.ok === false) return applicationError(result);
    const saved = result as DesktopDemTaskSettings;
    if (
      !Number.isInteger(saved.max_concurrent_tasks) ||
      !Number.isInteger(saved.max_concurrent_conversions)
    ) {
      return demProtocolError("save_dem_task_settings");
    }
    return { ok: true, ...saved };
  }
  if (hasBridge()) {
    const method = api().save_dem_task_settings;
    return typeof method === "function"
      ? method(settings)
      : demProtocolError("save_dem_task_settings");
  }
  if (!demDevelopmentMockEnabled()) return demProtocolError("save_dem_task_settings");
  const limit = settings.max_concurrent_tasks;
  if (!Number.isInteger(limit) || limit < 1 || limit > 10) {
    return {
      ok: false,
      error: "DEM 下载并发必须是 1 到 10 的整数",
      code: "DEM_TASK_SETTINGS",
    };
  }
  mockDemTaskSettings = { ...mockDemTaskSettings, max_concurrent_tasks: limit };
  scheduleMockDemTasks();
  return { ...mockDemTaskSettings };
}

export async function runDemDownloadBbox(
  west: number,
  east: number,
  south: number,
  north: number,
  outputDir = "",
  dataset = "COP30",
  keySource = "auto",
  convert = true,
): Promise<RunSummaryResult> {
  if (isTauriDesktopHost()) {
    const task = await startDemDownloadBbox(
      west,
      east,
      south,
      north,
      outputDir,
      dataset,
      keySource,
      convert,
      "auto",
    );
    if (!task.ok) return task;
    return {
      ok: true,
      summary_line: `正式 DEM 任务 ${task.task_id} 已提交；转换由独立 conversion handoff 负责`,
      total: 1,
      succeeded: 0,
      skipped: 0,
      failed: 0,
      interrupted: 0,
      has_failures: false,
      results_path: "",
      results: [{ task_id: task.task_id, state: task.state }],
    };
  }
  if (hasBridge()) {
    return api().run_dem_download_bbox(west, east, south, north, outputDir, dataset, keySource, convert);
  }
  const plan = await planDemDownloadBbox(west, east, south, north, outputDir, dataset);
  if (!plan.ok) return plan;
  if (!convert) {
    return {
      ok: true,
      summary_line: "1 downloaded, 0 skipped, 0 failed, 0 interrupted",
      total: 1,
      succeeded: 1,
      skipped: 0,
      failed: 0,
      interrupted: 0,
      has_failures: false,
      results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_download_results.txt`,
      results: [
        {
          region_safe_name: "standalone_dem",
          dataset,
          outcome: "success",
          bytes_written: 1024,
          message: "mock DEM ok",
        },
      ],
      raw_dem_path: String(plan.plan.raw_dem_path ?? ""),
      ellipsoid_dem_path: "",
      sarscape_ready_dem_path: "",
    };
  }
  const conversion = await runDemConversionBbox(west, east, south, north, outputDir, dataset);
  return {
    ok: true,
    summary_line: `下载：1 downloaded, 0 skipped, 0 failed, 0 interrupted；转换：${
      conversion.ok ? conversion.summary_line : "未执行"
    }`,
    total: 1,
    succeeded: 1,
    skipped: 0,
    failed: 0,
    interrupted: 0,
    has_failures: conversion.ok ? conversion.has_failures : true,
    results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_download_results.txt`,
    results: [
      {
        region_safe_name: "standalone_dem",
        dataset,
        outcome: "success",
        bytes_written: 1024,
        message: "mock DEM ok",
      },
    ],
    download: {
      summary_line: "1 downloaded, 0 skipped, 0 failed, 0 interrupted",
      results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_download_results.txt`,
    },
    conversion: conversion.ok ? conversion : null,
    conversion_results_path: conversion.ok ? conversion.results_path : "",
    raw_dem_path: String(plan.plan.raw_dem_path ?? ""),
    ellipsoid_dem_path: String(plan.plan.ellipsoid_dem_path ?? ""),
    sarscape_ready_dem_path: String(plan.plan.sarscape_ready_dem_path ?? ""),
  };
}

export async function captureGacosAoi(): Promise<ExactOperationAoiResult> {
  const aoi = await exactOperationAoi();
  return aoi.ok ? structuredClone(aoi) : aoi;
}

export async function planGacosRequest(
  outputDir = "",
  options: GacosPlanOptions = {},
): Promise<PlanReportResult> {
  if (isTauriDesktopHost()) {
    const context = await getContext();
    if (!context.ok) return context;
    const frozenAoi = options.aoi_source === "frozen_import";
    if (frozenAoi && (!recordValue(options.aoi_geojson) || !options.aoi_name?.trim())) {
      return { ok: false, error: "请先导入并冻结 GACOS 的 AOI", code: "GACOS_AOI_MISSING" };
    }
    const aoi = frozenAoi
      ? { ok: true as const, geometry: structuredClone(options.aoi_geojson!), display_name: options.aoi_name!.trim() }
      : await exactOperationAoi(context);
    if (!aoi.ok) return aoi;
    const snapshotScenes = Array.isArray(options.snapshot_scenes) ? options.snapshot_scenes : [];
    const selectedSceneIds = options.selected_scene_ids ?? [];
    const selectedIds = new Set(selectedSceneIds);
    if (options.source_kind === "online_selected" && (selectedIds.size === 0 || selectedIds.size !== selectedSceneIds.length)) {
      return { ok: false, error: "GACOS 在线来源要求非空且唯一的已勾选场景 ID", code: "GACOS_SELECTION_INVALID" };
    }
    const scenes = options.source_kind === "online_selected"
      ? snapshotScenes.filter((scene) => selectedIds.has(scene.scene_id))
      : snapshotScenes;
    if (options.source_kind === "online_selected" && scenes.length !== selectedIds.size) {
      return { ok: false, error: "GACOS 已勾选场景与当前冻结快照不一致", code: "GACOS_SELECTION_STALE" };
    }
    if (!scenes.length && !(options.local_sources ?? []).length
      && options.source_kind !== "manual_dates" && options.source_kind !== "online_batch") {
      return { ok: false, error: "GACOS 规划至少需要一个冻结场景或本地来源", code: "GAC001" };
    }
    const planner = getDesktopShellBridge().application.gacosPlan;
    if (typeof planner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露离线 GACOS 规划接口", code: "GUI003" };
    }
    const outputRoot = outputDir.trim()
      || context.region?.root
      || context.project?.root
      || context.workspace?.root;
    if (!outputRoot) {
      return { ok: false, error: "GACOS 规划需要确认输出目录", code: "V3_APPLICATION_INVALID_REQUEST" };
    }
    const { bbox: _clientBbox, ...plannerOptions } = options;
    void _clientBbox;
    const result = recordValue(await planner({
      ...plannerOptions,
      request_id: `gacos-plan-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      output_dir: outputRoot,
      region_id: frozenAoi ? null : context.region?.region_id ?? null,
      region_safe_name: frozenAoi ? aoi.display_name : context.region?.safe_name ?? aoi.display_name,
      aoi_geojson: aoi.geometry,
      aoi_name: aoi.display_name.trim(),
      scenes,
    }));
    if (result?.ok === true && recordValue(result.plan) && recordValue(result.report)) {
      return {
        ok: true,
        plan: result.plan as Json,
        report: result.report as Json,
        offline: result.offline === true,
      };
    }
    if (result?.ok === false) return applicationError(result);
    return {
      ok: false,
      error: "兼容 Worker 未返回有效 GACOS 规划",
      code: "GACOS_PLAN_INVALID",
    };
  }
  if (hasBridge()) return api().plan_gacos_request(outputDir, options);
  if (!mock.region?.has_aoi) {
    return { ok: false, error: "请先在『区域 AOI』设置处理范围（bbox）", code: "AOI001" };
  }
  if (!mock.region.scene_count) {
    return { ok: false, error: "GACOS 需要场景日期，请先导入影像", code: "GAC001" };
  }
  return {
    ok: true,
    plan: {
      region_safe_name: mock.region.safe_name,
      unique_dates: ["2024-03-12", "2024-03-24"],
      processing_windows: [mock.region.bbox],
      request_windows: [mock.region.bbox],
      buffer_degrees: 0.05,
      batches: [{ dates: ["2024-03-12", "2024-03-24"] }],
      output_directory: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\${mock.region.safe_name}\\GACOS\\requests`,
    },
    report: mockReport(),
  };
}

function gacosSubmissionResult(
  value: DesktopGacosSubmissionResult,
): GacosRequestSubmitResult {
  if (!value.ok) return applicationError(value);
  return {
    ok: true,
    task_id: value.task_id,
    records: value.records as unknown as Json[],
    request_groups: value.request_groups,
    task_status: value.task_status,
    submitted_count: value.submitted_count,
    application_completed: value.application_completed,
    manual_action_required: value.manual_action_required,
    automatic_retry_allowed: false,
    message: value.message,
  };
}

function gacosPreparedHandles(plan: Json):
  | { prepared_batch_id: string; plan_fingerprint_sha256: string }
  | ApiError {
  const planRecord = recordValue(plan);
  const preparedBatchId = typeof planRecord?.prepared_batch_id === "string"
    ? planRecord.prepared_batch_id.trim()
    : "";
  const fingerprint = typeof planRecord?.plan_fingerprint_sha256 === "string"
    ? planRecord.plan_fingerprint_sha256.trim()
    : "";
  if (!preparedBatchId || preparedBatchId.length > 128 || !/^[a-f0-9]{64}$/.test(fingerprint)) {
    return {
      ok: false,
      error: "GACOS 计划缺少有效的 prepared_batch_id 或 plan_fingerprint_sha256，请重新生成冻结计划。",
      code: "GACOS_REQUEST_INVALID",
    };
  }
  return { prepared_batch_id: preparedBatchId, plan_fingerprint_sha256: fingerprint };
}

function gacosTaskId(fingerprint: string, forceReapply: boolean): string | null {
  if (!forceReapply) return `gacos-request-${fingerprint}`;
  const uuid = globalThis.crypto?.randomUUID?.();
  return uuid ? `gacos-request-${fingerprint}-${uuid}` : null;
}

export async function submitGacosRequest(
  plan: Json,
  confirm = false,
  forceReapply = false,
  continueAfterFailure = false,
): Promise<GacosRequestSubmitResult> {
  if (!confirm) return { ok: false, error: "需要最终确认后才能提交 GACOS 请求", code: "GACOS_CONFIRMATION_REQUIRED" };
  void continueAfterFailure;
  if (!isTauriDesktopHost()) {
    return {
      ok: false,
      error: "GACOS live submit 只在 native Tauri 中可用",
      code: "GACOS_TRANSPORT_UNAVAILABLE",
    };
  }
  const handles = gacosPreparedHandles(plan);
  if ("ok" in handles) return handles;
  const taskId = gacosTaskId(handles.plan_fingerprint_sha256, forceReapply);
  if (!taskId) {
    return {
      ok: false,
      error: "无法生成稳定的 GACOS task_id，请重启应用后重试。",
      code: "GACOS_REQUEST_INVALID",
    };
  }
  const route = getDesktopShellBridge().application.gacosSubmitRequest;
  if (!route) {
    return {
      ok: false,
      error: "当前桌面 API 未暴露 GACOS 提交接口，请更新到支持 GACOS 请求任务的版本。",
      code: "GACOS_TRANSPORT_UNAVAILABLE",
    };
  }
  const request: DesktopGacosConfirmRequest = {
    prepared_batch_id: handles.prepared_batch_id,
    plan_fingerprint_sha256: handles.plan_fingerprint_sha256,
    task_id: taskId,
    confirm: true,
    force_reapply: forceReapply,
  };
  const result = await route(request);
  return gacosSubmissionResult(result);
}

export async function getGacosRequestStatus(taskId: string): Promise<GacosRequestSubmitResult> {
  const stableTaskId = taskId.trim();
  if (!stableTaskId || stableTaskId.length > 128) {
    return { ok: false, error: "GACOS task_id 无效，无法读取持久申请状态。", code: "GACOS_REQUEST_INVALID" };
  }
  if (!isTauriDesktopHost()) {
    return {
      ok: false,
      error: "GACOS 请求状态只在 native Tauri 中可用",
      code: "GACOS_TRANSPORT_UNAVAILABLE",
    };
  }
  const route = getDesktopShellBridge().application.gacosRequestStatus;
  if (!route) {
    return {
      ok: false,
      error: "当前桌面 API 未暴露 GACOS 状态接口，请更新到支持 GACOS 请求任务的版本。",
      code: "GACOS_TRANSPORT_UNAVAILABLE",
    };
  }
  return gacosSubmissionResult(await route(stableTaskId));
}

export async function getGacosLedger(): Promise<{ ok: true; records: Json[] } | ApiError> {
  if (isTauriDesktopHost()) {
    const route = getDesktopShellBridge().application.gacosLedger;
    if (!route) return demProtocolError("gacos_ledger");
    const result = await route();
    if ("ok" in result && result.ok === false) return applicationError(result);
    const value = recordValue(result);
    if (!value || value.ok !== true || !Array.isArray(value.records)) {
      return { ok: false, error: "ApplicationService 未返回有效 GACOS ledger", code: "GACOS_LEDGER_INVALID" };
    }
    return {
      ok: true,
      records: value.records.filter((item): item is Json => recordValue(item) !== null) as Json[],
    };
  }
  return {
    ok: false,
    error: "当前浏览器/兼容桥未暴露产品 GACOS ledger",
    code: "GACOS_LEDGER_UNAVAILABLE",
  };
}

export async function importGacosResult(
  ztdPath: string,
  rscPath: string,
  outputDir = "",
): Promise<Json | ApiError> {
  if (!isTauriDesktopHost()) {
    return { ok: false, error: "GACOS 结果导入只在 native Tauri 中可用", code: "GACOS_IMPORT_UNAVAILABLE" };
  }
  return importApplicationGacosResult({
    ztd_path: ztdPath,
    rsc_path: rscPath,
    output_dir: outputDir || null,
  });
}

export async function createGacosImportTask(
  request: DesktopGacosCreateImportTaskRequest,
): Promise<DesktopGacosCreateImportTaskResult> {
  if (!isTauriDesktopHost()) {
    return {
      ok: false,
      error: "GACOS 结果导入只在 native Tauri 中可用",
      code: "GACOS_IMPORT_UNAVAILABLE",
    };
  }
  const route = getDesktopShellBridge().application.gacosCreateImportTask;
  if (!route) {
    return {
      ok: false,
      error: "当前桌面 API 未暴露 GACOS 结果任务接口，请更新到支持结果任务的版本。",
      code: "GACOS_TRANSPORT_UNAVAILABLE",
    };
  }
  return route(request);
}

export async function createGacosResultTask(
  request: DesktopGacosCreateResultTaskRequest,
): Promise<DesktopGacosCreateImportTaskResult> {
  if (!isTauriDesktopHost()) {
    return {
      ok: false,
      error: "GACOS 结果下载仅在桌面应用中可用",
      code: "GACOS_IMPORT_UNAVAILABLE",
    };
  }
  const route = getDesktopShellBridge().application.gacosCreateResultTask;
  if (!route) {
    return {
      ok: false,
      error: "当前桌面 API 未暴露 GACOS 结果任务接口，请更新到支持结果任务的版本。",
      code: "GACOS_TRANSPORT_UNAVAILABLE",
    };
  }
  return route(request);
}

export async function supplyGacosResultLink(
  request: DesktopGacosSupplyResultLinkRequest,
): Promise<DesktopGacosSupplyResultLinkResult> {
  if (!isTauriDesktopHost()) {
    return {
      ok: false,
      error: "GACOS 结果下载仅在桌面应用中可用",
      code: "GACOS_IMPORT_UNAVAILABLE",
    };
  }
  const route = getDesktopShellBridge().application.gacosSupplyResultLink;
  if (!route) {
    return {
      ok: false,
      error: "当前桌面 API 未暴露 GACOS 结果任务接口，请更新到支持结果任务的版本。",
      code: "GACOS_TRANSPORT_UNAVAILABLE",
    };
  }
  return route(request);
}

async function planFormalDemConversion(
  inputPath: string,
  outputDir: string,
  dataset: string,
  requestedBbox?: Bbox,
  sourceVerticalDatum = "auto",
  outputMode: "ellipsoid" | "sarscape" = "sarscape",
): Promise<ConvertResult> {
  const context = await getContext();
  if (!context.ok) return context;
  if (!context.workspace || !context.project || !context.region || !context.region.has_aoi) {
    return { ok: false, error: "请先绑定当前研究区 AOI", code: "AOI001" };
  }
  if (
    requestedBbox &&
    [
      requestedBbox.west !== context.region.bbox?.west,
      requestedBbox.east !== context.region.bbox?.east,
      requestedBbox.south !== context.region.bbox?.south,
      requestedBbox.north !== context.region.bbox?.north,
    ].some(Boolean)
  ) {
    return { ok: false, error: "手工 bbox 与当前研究区持久化 AOI 不一致，请先绑定该 AOI", code: "AOI001" };
  }
  const root = outputDir.trim() || context.region.root || context.project.root || context.workspace.root;
  const identity = formalDemIdentity(dataset);
  if (!identity) {
    return { ok: false, error: "该 DEM 数据集未在 ResourceRegistry 中登记正式任务路由", code: "RESOURCE_REGISTRY_UNAVAILABLE" };
  }
  const source = inputPath.trim() || joinDemPath(root, `${demSourceStem(dataset)}.tif`);
  const taskId = stableUiNodeId("dem-conversion-plan", `${context.region.region_id}-${source}`);
  const plan = await planApplicationDemTask({
    request_id: `dem-conversion-plan-${taskId}`,
    task_id: taskId,
    region_id: context.region.region_id,
    resource_id: identity.resource_id,
    capability_id: identity.capability_id,
    dataset: identity.dataset,
    provider: identity.provider,
    operation: "convert",
    input_path: source,
    output_root: root,
    source_vertical_datum: sourceVerticalDatum === "auto" ? null : sourceVerticalDatum,
    target_vertical_datum: "WGS84_ELLIPSOID",
    output_mode: outputMode,
    created_at: new Date().toISOString(),
  });
  if (!plan.ok) return plan;
  return {
    ok: true,
    dataset,
    auto: {
      requires_conversion: true,
      requires_geoid: true,
      source: "runtime_projection",
      target: "WGS84_ELLIPSOID",
      geoid_model: null,
      message: "正式 DEM 转换任务将由 ApplicationService 交给共享 Store/Scheduler/Download Engine。",
    },
    plan: {
      request_id: plan.plan.request_id,
      task_id: plan.plan.task_id,
      region_id: plan.plan.region_id,
      resource_id: plan.plan.resource_id,
      capability_id: plan.plan.capability_id,
      dataset: plan.plan.dataset,
      provider: plan.plan.provider,
      input_path: plan.plan.input_path,
      output_root: plan.plan.output_root,
      source_vertical_datum: plan.plan.source_vertical_datum ?? null,
      target_vertical_datum: plan.plan.target_vertical_datum ?? null,
      output_mode: plan.plan.output_mode ?? outputMode,
      operation: plan.plan.operation,
      created_at: plan.plan.created_at,
      sarscape_ready_dem_path: joinDemPath(plan.plan.output_root, `${demSourceStem(dataset)}_dem`),
    },
    report: {
      source: "ApplicationService.ResourceRegistry.runtime_projection",
      task_route: "Store/Scheduler/Download Engine -> approved conversion handoff",
      provider_network: false,
    },
  };
}

export async function planDemConversion(outputDir = ""): Promise<ConvertResult> {
  if (isTauriDesktopHost()) {
    const context = await getContext();
    if (!context.ok) return context;
    return planFormalDemConversion("", outputDir, context.dem_dataset ?? "COP30");
  }
  if (hasBridge()) return api().plan_dem_conversion(outputDir);
  if (!mock.region?.has_aoi) {
    return { ok: false, error: "请先在『区域 AOI』设置处理范围（bbox）", code: "AOI001" };
  }
  const dataset = mockDataset;
  const safe = mock.region.safe_name;
  const root = mock.workspace?.root ?? "C:\\InSAR";
  const source = mockSourceDatum(dataset);
  const requires = source !== "WGS84_ELLIPSOID";
  const geoid = !requires ? null : source === "EGM96" ? "EGM96" : "EGM2008";
  const ds = dataset.toLowerCase();

  const steps = requires
    ? [
        {
          step_type: "VERTICAL_DATUM_CONVERSION",
          description: `${source} 正高 → WGS84 椭球高（geoid: ${geoid}）`,
          requires_geoid: true,
          geoid_model: geoid,
        },
        { step_type: "COPY_TO_SARSCAPE_READY", description: "导出 SARscape ENVI _dem + .hdr + .sml" },
      ]
    : [
        {
          step_type: "COPY_TO_SARSCAPE_READY",
          description: "该 DEM 已是椭球高，直接导出 SARscape ENVI _dem + .hdr + .sml",
        },
      ];

  const auto: ConversionAuto = {
    requires_conversion: requires,
    requires_geoid: requires,
    source,
    target: "WGS84_ELLIPSOID",
    geoid_model: geoid,
    message: requires
      ? `检测到高程基准为 ${source}（正高），将自动转换为 WGS84_ELLIPSOID，采用大地水准面模型 ${geoid}（系统自动选择）。`
      : `该 DEM 高程基准已为 ${source}（椭球高），无需垂直基准转换，将导出 SARscape ENVI _dem + .hdr + .sml。`,
  };

  return {
    ok: true,
    dataset,
    auto,
    plan: {
      dataset,
      source_vertical_datum: source,
      target_vertical_datum: "WGS84_ELLIPSOID",
      requires_conversion: requires,
      requires_geoid: requires,
      raw_dem_path: `${root}\\${demSourceStem(dataset)}.tif`,
      ellipsoid_dem_path: `${root}\\${demSourceStem(dataset)}_ellipsoid.tif`,
      sarscape_ready_dem_path: `${root}\\${demSourceStem(dataset)}_dem`,
      steps,
    },
    report: mockReport(),
  };
}

function isWindowsRuntime(): boolean {
  if (typeof navigator !== "undefined") {
    const nav = navigator as { platform?: string; userAgent?: string };
    const hint = `${nav.platform || ""} ${nav.userAgent || ""}`.toLowerCase();
    if (hint.trim().length > 0) {
      return /\bwin(?:32|64|dows|nt)?\b/.test(hint);
    }
  }
  const nodeProcess = (globalThis as { process?: { platform?: string } }).process;
  if (typeof nodeProcess?.platform === "string") {
    return nodeProcess.platform === "win32";
  }
  return false;
}

export function joinDemPath(root: string, leaf: string): string {
  const trimmedRoot = root.trim().replace(/[\\/]+$/, "");
  const cleanLeaf = leaf.replace(/^[\\/]+/, "");
  if (!trimmedRoot) return cleanLeaf;
  const useWindowsSep = /^[A-Za-z]:/.test(trimmedRoot) || (trimmedRoot.includes("\\") && !trimmedRoot.startsWith("/"));
  return `${trimmedRoot}${useWindowsSep ? "\\" : "/"}${cleanLeaf}`;
}

export function standaloneDemOutputRoot(outputDir: string): string {
  return outputDir.trim() || (isWindowsRuntime() ? "C:\\InSAR\\standalone-dem" : "/eostage/standalone-dem");
}

function standaloneDemSourceDatum(inputPath: string, dataset: string, requested: string): string {
  const normalized = requested.trim().toUpperCase();
  if (normalized && normalized !== "AUTO") return normalized;
  const datasetKey = dataset.trim().toUpperCase();
  if (datasetKey === "COP30" || datasetKey === "COP90" || datasetKey === "FABDEM") return "EGM2008";
  if (datasetKey === "SRTM_GL1_ELLIPSOIDAL" || datasetKey === "AW3D30_ELLIPSOIDAL" || datasetKey === "NISAR_DEM") return "WGS84_ELLIPSOID";
  if (["NASADEM", "SRTM_GL3", "SRTM_GL1", "AW3D30", "GEM15_DTM", "GEM15_DSM", "ASTER_GDEM_V3", "ASTGTM"].includes(datasetKey)) return "EGM96";
  const lower = inputPath.toLowerCase();
  if (lower.includes("ellipsoid")) return "WGS84_ELLIPSOID";
  if (/egm2008|cop30|cop90|fabdem|copernicus|glo[-_ ]?30|glo[-_ ]?90/.test(lower)) return "EGM2008";
  if (/egm96|nasadem|srtm|aw3d30|gem15|gem-15|aster|astgtm/.test(lower)) return "EGM96";
  return "UNKNOWN";
}

function standaloneDemStem(inputPath: string, dataset: string): string {
  const raw = inputPath.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") || dataset || "local_dem";
  const stem = raw
    .replace(/[^A-Za-z0-9_]+/g, "_")
    .replace(/_(ELLIPSOID|DEM)$/i, "")
    .replace(/^_+|_+$/g, "");
  return stem || "local_dem";
}

function standaloneDemError(raw: unknown, fallback: string): ApiError {
  const record = recordValue(raw);
  return {
    ok: false,
    error: typeof record?.error === "string" ? record.error : fallback,
    code: typeof record?.code === "string" ? record.code : "NATIVE_DEM_OPERATION_FAILED",
  };
}

async function planStandaloneDemConversion(
  inputPath: string,
  outputDir: string,
  dataset: string,
  provider: string,
  sourceVerticalDatum: string,
  bbox?: Bbox,
): Promise<ConvertResult> {
  if (!inputPath.trim()) return { ok: false, error: "请选择本地 DEM 文件", code: "DEM004" };
  const identity = formalDemIdentity(dataset, provider);
  if (!identity) {
    return { ok: false, error: "该 DEM 数据集未在 ResourceRegistry 中登记正式任务路由", code: "RESOURCE_REGISTRY_UNAVAILABLE" };
  }
  const root = standaloneDemOutputRoot(outputDir);
  const source = standaloneDemSourceDatum(inputPath, dataset, sourceVerticalDatum);
  if (source === "UNKNOWN") {
    return {
      ok: false,
      error: "无法自动识别本地 DEM 的源高程基准；请明确选择 EGM96、EGM2008 或 WGS84 椭球高后重试。",
      code: "DEM_SOURCE_DATUM_REQUIRED",
    };
  }
  const response = await runApplicationStandaloneDem({
    request_id: stableUiNodeId("standalone-dem-plan", `${dataset}-${inputPath}`),
    dataset: identity.dataset,
    provider: identity.provider,
    operation: "prepare_sarscape",
    input_path: inputPath,
    output_root: root,
    bbox: bbox ? [bbox.west, bbox.south, bbox.east, bbox.north] : null,
    source_vertical_datum: source,
    target_vertical_datum: "WGS84_ELLIPSOID",
    output_mode: "sarscape",
  });
  if ("ok" in response && response.ok === false) return applicationError(response);
  const envelope = recordValue(response);
  const operationResult = recordValue(envelope?.result);
  if (!envelope || envelope.ok !== true || !operationResult || operationResult.ok === false) {
    return standaloneDemError(response, "Native DEM 计划失败");
  }
  const stem = standaloneDemStem(inputPath, dataset);
  const requires = source !== "WGS84_ELLIPSOID";
  const geoid = requires ? (source === "EGM2008" ? "EGM2008" : "EGM96") : null;
  const plan = {
    dataset: identity.dataset,
    provider: identity.provider,
    raw_dem_path: inputPath,
    source_vertical_datum: source,
    target_vertical_datum: "WGS84_ELLIPSOID",
    requires_conversion: requires,
    requires_geoid: requires,
    ellipsoid_dem_path: joinDemPath(root, `${stem}_ellipsoid.tif`),
    sarscape_ready_dem_path: joinDemPath(root, `${stem}_dem`),
    bbox: bbox ?? null,
    ...(recordValue(operationResult.plan) ?? {}),
  };
  const steps = requires
    ? [
        {
          step_type: "VERTICAL_DATUM_CONVERSION",
          description: `${source} → WGS84_ELLIPSOID（geoid: ${geoid}）`,
          requires_geoid: true,
          geoid_model: geoid,
        },
        { step_type: "COPY_TO_SARSCAPE_READY", description: "导出 SARscape ENVI _dem + .hdr + .sml" },
      ]
    : [{ step_type: "COPY_TO_SARSCAPE_READY", description: "导出 SARscape ENVI _dem + .hdr + .sml" }];
  return {
    ok: true,
    dataset: identity.dataset,
    auto: {
      requires_conversion: requires,
      requires_geoid: requires,
      source,
      target: "WGS84_ELLIPSOID",
      geoid_model: geoid,
      message: requires
        ? `检测到高程基准为 ${source}，将转换为 WGS84_ELLIPSOID，采用产品内置 ${geoid}。`
        : "该 DEM 已按 WGS84 椭球高处理，将导出 SARscape ENVI _dem + .hdr + .sml。",
    },
    plan: { ...plan, steps },
    report: {
      source: "ApplicationService.NativeGisEngine",
      native: true,
      offline: true,
      bbox: bbox ?? null,
      provider: identity.provider,
    },
  };
}

async function runStandaloneDemConversion(
  inputPath: string,
  outputDir: string,
  dataset: string,
  provider: string,
  sourceVerticalDatum: string,
  outputMode: "ellipsoid" | "sarscape",
  bbox?: Bbox,
): Promise<RunSummaryResult> {
  if (!inputPath.trim()) return { ok: false, error: "请选择本地 DEM 文件", code: "DEM004" };
  const identity = formalDemIdentity(dataset, provider);
  if (!identity) {
    return { ok: false, error: "该 DEM 数据集未在 ResourceRegistry 中登记正式任务路由", code: "RESOURCE_REGISTRY_UNAVAILABLE" };
  }
  const root = standaloneDemOutputRoot(outputDir);
  const source = standaloneDemSourceDatum(inputPath, dataset, sourceVerticalDatum);
  if (source === "UNKNOWN") {
    return {
      ok: false,
      error: "无法自动识别本地 DEM 的源高程基准；请明确选择 EGM96、EGM2008 或 WGS84 椭球高后重试。",
      code: "DEM_SOURCE_DATUM_REQUIRED",
    };
  }
  const response = await runApplicationStandaloneDem({
    request_id: stableUiNodeId("standalone-dem-convert", `${dataset}-${inputPath}-${outputMode}`),
    dataset: identity.dataset,
    provider: identity.provider,
    operation: "convert",
    input_path: inputPath,
    output_root: root,
    bbox: bbox ? [bbox.west, bbox.south, bbox.east, bbox.north] : null,
    source_vertical_datum: source,
    target_vertical_datum: "WGS84_ELLIPSOID",
    output_mode: outputMode,
  });
  if ("ok" in response && response.ok === false) return applicationError(response);
  const envelope = recordValue(response);
  const result = recordValue(envelope?.result);
  if (!envelope || envelope.ok !== true || !result || result.ok === false) {
    return standaloneDemError(response, "Native DEM 转换失败");
  }
  const ellipsoid = typeof result.ellipsoid_path === "string" ? result.ellipsoid_path : "";
  const sarscape = typeof result.sarscape_path === "string" ? result.sarscape_path : "";
  return {
    ok: true,
    summary_line: "Native DEM 转换完成（1 converted, 0 copied, 0 skipped, 0 failed）",
    total: 1,
    succeeded: 1,
    copied: 0,
    skipped: 0,
    failed: 0,
    interrupted: 0,
    has_failures: false,
    results_path: joinDemPath(root, "dem_convert_results.json"),
    raw_dem_path: inputPath,
    ellipsoid_dem_path: ellipsoid,
    sarscape_ready_dem_path: outputMode === "ellipsoid" ? "" : sarscape,
    results: [{
      dataset: identity.dataset,
      outcome: "success",
      output_path: outputMode === "ellipsoid" ? ellipsoid : sarscape,
      bytes_written: typeof result.bytes_written === "number" ? result.bytes_written : 0,
      source_vertical_datum: source,
      target_vertical_datum: "WGS84_ELLIPSOID",
      bbox: bbox ?? null,
    }],
  };
}

export async function planLocalDemConversion(
  inputPath: string,
  outputDir = "",
  sourceVerticalDatum = "auto",
): Promise<ConvertResult> {
  if (isTauriDesktopHost()) {
    return planStandaloneDemConversion(inputPath, outputDir, "USER_LOCAL", "LOCAL_FILE", sourceVerticalDatum);
  }
  if (hasBridge()) {
    return api().plan_local_dem_conversion(inputPath, outputDir, sourceVerticalDatum);
  }
  if (!inputPath.trim()) return { ok: false, error: "请选择本地 DEM 文件", code: "DEM004" };
  const safe = inputPath
    .split(/[\\/]/)
    .pop()
    ?.replace(/\.[^.]+$/, "")
    .replace(/[^A-Za-z0-9_]+/g, "_")
    .toLowerCase() || "local_dem";
  const source = standaloneDemSourceDatum(inputPath, "USER_LOCAL", sourceVerticalDatum);
  if (source === "UNKNOWN") {
    return {
      ok: false,
      error: "无法自动识别本地 DEM 的源高程基准；请明确选择 EGM96、EGM2008 或 WGS84 椭球高后重试。",
      code: "DEM_SOURCE_DATUM_REQUIRED",
    };
  }
  const requires = source !== "WGS84_ELLIPSOID";
  const geoid = !requires ? null : source === "EGM96" ? "EGM96" : "EGM2008";
  const root = outputDir || mock.region?.root || mock.project?.root || mock.workspace?.root || "C:\\InSAR";
  return {
    ok: true,
    dataset: "USER_LOCAL",
    auto: {
      requires_conversion: requires,
      requires_geoid: requires,
      source,
      target: "WGS84_ELLIPSOID",
      geoid_model: geoid,
      message: requires
        ? `检测到或选择高程基准为 ${source}，将转换为 WGS84_ELLIPSOID。`
        : "该 DEM 已按椭球高处理，将导出 SARscape ENVI _dem + .hdr + .sml。",
    },
    plan: {
      dataset: "USER_LOCAL",
      raw_dem_path: inputPath,
      ellipsoid_dem_path: `${root}\\${safe}_ellipsoid.tif`,
      sarscape_ready_dem_path: `${root}\\${safe}_dem`,
      steps: requires
        ? [
            {
              step_type: "VERTICAL_DATUM_CONVERSION",
              description: `${source} → WGS84_ELLIPSOID`,
              requires_geoid: true,
              geoid_model: geoid,
            },
            { step_type: "COPY_TO_SARSCAPE_READY", description: "导出 SARscape ENVI _dem + .hdr + .sml" },
          ]
        : [{ step_type: "COPY_TO_SARSCAPE_READY", description: "导出 SARscape ENVI _dem + .hdr + .sml" }],
    },
    report: mockReport(),
  };
}

export async function planDemConversionBbox(
  west: number,
  east: number,
  south: number,
  north: number,
  outputDir = "",
  dataset = "COP30",
): Promise<ConvertResult> {
  if (isTauriDesktopHost()) {
    if (!(west < east) || !(south < north)) {
      return { ok: false, error: "bbox 范围无效，请检查 west/east/south/north", code: "AOI001" };
    }
    const root = standaloneDemOutputRoot(outputDir);
    return planStandaloneDemConversion(
      joinDemPath(root, `${demSourceStem(dataset)}.tif`),
      root,
      dataset,
      dataset === "COP30" || dataset === "COP90" ? "COPERNICUS_AWS" : "OPENTOPOGRAPHY",
      "auto",
      { west, east, south, north, crs: "EPSG:4326" },
    );
  }
  if (hasBridge()) {
    return api().plan_dem_conversion_bbox(west, east, south, north, outputDir, dataset);
  }
  const dem = await planDemDownloadBbox(west, east, south, north, outputDir, dataset);
  if (!dem.ok) return dem;
  const source = mockSourceDatum(dataset);
  const requires = source !== "WGS84_ELLIPSOID";
  const geoid = !requires ? null : source === "EGM96" ? "EGM96" : "EGM2008";
  return {
    ok: true,
    dataset,
    auto: {
      requires_conversion: requires,
      requires_geoid: requires,
      source,
      target: "WGS84_ELLIPSOID",
      geoid_model: geoid,
      message: requires
        ? `检测到高程基准为 ${source}（正高），将自动转换为 WGS84_ELLIPSOID，采用大地水准面模型 ${geoid}（系统自动选择）。`
        : `该 DEM 高程基准已为 ${source}（椭球高），无需垂直基准转换，将导出 SARscape ENVI _dem + .hdr + .sml。`,
    },
    plan: {
      ...mockDemPlan(
        dataset,
        { west, east, south, north, crs: "EPSG:4326" },
        "standalone_dem",
        outputDir || mock.workspace?.root || "C:\\InSAR",
      ),
      requires_conversion: requires,
      requires_geoid: requires,
      steps: requires
        ? [
            {
              step_type: "VERTICAL_DATUM_CONVERSION",
              description: `${source} 正高 → WGS84 椭球高（geoid: ${geoid}）`,
              requires_geoid: true,
              geoid_model: geoid,
            },
            { step_type: "COPY_TO_SARSCAPE_READY", description: "导出 SARscape ENVI _dem + .hdr + .sml" },
          ]
        : [
            {
              step_type: "COPY_TO_SARSCAPE_READY",
              description: "该 DEM 已是椭球高，直接导出 SARscape ENVI _dem + .hdr + .sml",
            },
          ],
    },
    report: mockReport(),
  };
}

async function runFormalDemConversionHandoff(_outputDir: string): Promise<RunSummaryResult> {
  const context = await getContext();
  if (!context.ok) return context;
  if (!context.workspace || !context.project || !context.region) {
    return { ok: false, error: "请先打开工作区、项目和研究区", code: "V3_APPLICATION_INVALID_REQUEST" };
  }
  const dataset = context.dem_dataset ?? "COP30";
  const provider = context.dem_provider ?? "COPERNICUS_AWS";
  const identity = formalDemIdentity(dataset, provider);
  if (identity && (identity.dataset !== dataset || identity.provider !== provider)) {
    return { ok: false, error: "Store 中的 DEM dataset/provider 身份不一致", code: "V3_APPLICATION_INVALID_SNAPSHOT" };
  }
  if (!identity) {
    return { ok: false, error: "该 DEM 数据集未在 ResourceRegistry 中登记正式任务路由", code: "RESOURCE_REGISTRY_UNAVAILABLE" };
  }
  const resolved = await resolveApplicationDemConversionSource({
    region_id: context.region.region_id,
    dataset: identity.dataset,
    provider: identity.provider,
    resource_id: identity.resource_id,
    capability_id: identity.capability_id,
  });
  if (!resolved.ok) return applicationError(resolved);
  if (!resolved.source) {
    return {
      ok: false,
      error: "DEM 转换必须先有一个已完成的正式 DEM 源任务",
      code: "FORMAL_TASK_SOURCE_INCOMPLETE",
    };
  }
  const conversionTaskId = stableUiNodeId("dem-conversion", `${context.region.region_id}-${dataset}-${Date.now()}`);
  const result = await handoffApplicationDemConversion({
    request_id: `dem-conversion-${conversionTaskId}`,
    source_task_id: resolved.source.task_id,
    conversion_task_id: conversionTaskId,
    region_id: context.region.region_id,
    resource_id: identity.resource_id,
    capability_id: identity.capability_id,
    dataset: identity.dataset,
    provider: identity.provider,
    input_path: resolved.source.input_path,
    output_root: resolved.source.output_root,
    source_vertical_datum: null,
    target_vertical_datum: "WGS84_ELLIPSOID",
    output_mode: "sarscape",
    created_at: new Date().toISOString(),
    concurrency: 1,
  });
  if (!result.ok) return applicationError(result);
  return {
    ok: true,
    summary_line: `正式 DEM 转换任务 ${conversionTaskId} 已提交；源任务 ${resolved.source.task_id} 已由 Store 标量解析器确认`,
    total: 1,
    succeeded: 0,
    skipped: 0,
    failed: 0,
    interrupted: 0,
    has_failures: false,
    results_path: "",
    conversion_results_path: "",
    results: [{ task_id: conversionTaskId, source_task_id: resolved.source.task_id, state: "queued" }],
  };
}

export async function runDemConversion(outputDir = ""): Promise<RunSummaryResult> {
  if (isTauriDesktopHost()) return runFormalDemConversionHandoff(outputDir);
  if (hasBridge()) return api().run_dem_conversion(outputDir);
  const plan = await planDemConversion(outputDir);
  if (!plan.ok) return plan;
  return {
    ok: true,
    summary_line: plan.auto.requires_conversion
      ? "1 converted, 0 copied, 0 skipped, 0 failed"
      : "0 converted, 1 copied, 0 skipped, 0 failed",
    total: 1,
    succeeded: plan.auto.requires_conversion ? 1 : 0,
    copied: plan.auto.requires_conversion ? 0 : 1,
    skipped: 0,
    failed: 0,
    has_failures: false,
    results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_convert_results.txt`,
    results: [
      {
        region_safe_name: mock.region?.safe_name ?? "region",
        dataset: plan.dataset,
        outcome: plan.auto.requires_conversion ? "success" : "copied",
        output_path: String(plan.plan.sarscape_ready_dem_path ?? ""),
        message: "mock conversion ok",
      },
    ],
  };
}

export async function runLocalDemConversion(
  inputPath: string,
  outputDir = "",
  sourceVerticalDatum = "auto",
  outputMode: "ellipsoid" | "sarscape" = "sarscape",
): Promise<RunSummaryResult> {
  if (isTauriDesktopHost()) {
    return runStandaloneDemConversion(
      inputPath,
      outputDir,
      "USER_LOCAL",
      "LOCAL_FILE",
      sourceVerticalDatum,
      outputMode,
    );
  }
  if (hasBridge()) {
    return api().run_local_dem_conversion(inputPath, outputDir, sourceVerticalDatum, outputMode);
  }
  const plan = await planLocalDemConversion(inputPath, outputDir, sourceVerticalDatum);
  if (!plan.ok) return plan;
  return {
    ok: true,
    summary_line: plan.auto.requires_conversion
      ? "1 converted, 0 copied, 0 skipped, 0 failed"
      : "0 converted, 1 copied, 0 skipped, 0 failed",
    total: 1,
    succeeded: plan.auto.requires_conversion ? 1 : 0,
    copied: plan.auto.requires_conversion ? 0 : 1,
    skipped: 0,
    failed: 0,
    has_failures: false,
    results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_convert_results.txt`,
    raw_dem_path: String(plan.plan.raw_dem_path ?? inputPath),
    ellipsoid_dem_path: String(plan.plan.ellipsoid_dem_path ?? ""),
    sarscape_ready_dem_path: outputMode === "ellipsoid" ? "" : String(plan.plan.sarscape_ready_dem_path ?? ""),
    results: [
      {
        region_safe_name: mock.region?.safe_name ?? "local_dem",
        dataset: "USER_LOCAL",
        outcome: plan.auto.requires_conversion ? "success" : "copied",
        output_path:
          outputMode === "ellipsoid"
            ? String(plan.plan.ellipsoid_dem_path ?? "")
            : String(plan.plan.sarscape_ready_dem_path ?? ""),
        message: "mock local DEM conversion ok",
      },
    ],
  };
}

export async function runDemConversionBbox(
  west: number,
  east: number,
  south: number,
  north: number,
  outputDir = "",
  dataset = "COP30",
): Promise<RunSummaryResult> {
  if (isTauriDesktopHost()) {
    if (!(west < east) || !(south < north)) {
      return { ok: false, error: "bbox 范围无效，请检查 west/east/south/north", code: "AOI001" };
    }
    const root = standaloneDemOutputRoot(outputDir);
    return runStandaloneDemConversion(
      joinDemPath(root, `${demSourceStem(dataset)}.tif`),
      root,
      dataset,
      dataset === "COP30" || dataset === "COP90" ? "COPERNICUS_AWS" : "OPENTOPOGRAPHY",
      "auto",
      "sarscape",
      { west, east, south, north, crs: "EPSG:4326" },
    );
  }
  if (hasBridge()) {
    return api().run_dem_conversion_bbox(west, east, south, north, outputDir, dataset);
  }
  const plan = await planDemConversionBbox(west, east, south, north, outputDir, dataset);
  if (!plan.ok) return plan;
  return {
    ok: true,
    summary_line: plan.auto.requires_conversion
      ? "1 converted, 0 copied, 0 skipped, 0 failed"
      : "0 converted, 1 copied, 0 skipped, 0 failed",
    total: 1,
    succeeded: plan.auto.requires_conversion ? 1 : 0,
    copied: plan.auto.requires_conversion ? 0 : 1,
    skipped: 0,
    failed: 0,
    has_failures: false,
    results_path: `${outputDir || mock.workspace?.root || "C:\\InSAR"}\\dem_convert_results.txt`,
    results: [
      {
        region_safe_name: "standalone_dem",
        dataset,
        outcome: plan.auto.requires_conversion ? "success" : "copied",
        output_path: String(plan.plan.sarscape_ready_dem_path ?? ""),
        message: "mock conversion ok",
      },
    ],
  };
}

export async function getCredentialStatus(): Promise<CredentialStatus> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.getStatus();
    if (result.ok) {
      return { ok: true, ...result.status };
    }
    return {
      ok: true,
      cdse: "unavailable",
      usgs: "unavailable",
      earthdata: "unavailable",
      earthdata_input: { mode: "none", token: "", username: "", password: "" },
      bhoonidhi: "unavailable",
      opentopography: "unavailable",
      gacos: "unavailable",
    };
  }
  if (hasBridge()) return api().get_credential_status();
  return mockCredentials;
}

export async function saveCdseLogin(username: string, password: string): Promise<SimpleOk> {
  const credentials = tauriCredentialBridge();
  if (credentials) {
    const result = await credentials.saveCdseLogin(username, password);
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, status: result.status };
  }
  if (!username.trim() || !password) {
    return { ok: false, error: "CDSE 用户名和密码不能为空", code: "DL004" };
  }
  mockCredentials = { ...mockCredentials, cdse: `login:${username.trim()[0] ?? "*"}***` };
  notifyContextChanged();
  return { ok: true, status: mockCredentials };
}

export async function clearCdseCredentials(): Promise<SimpleOk> {
  const credentials = tauriCredentialBridge();
  if (credentials) {
    const result = await credentials.clearCdse();
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, removed: result.removed, status: result.status };
  }
  mockCredentials = { ...mockCredentials, cdse: "none" };
  notifyContextChanged();
  return { ok: true, removed: true, status: mockCredentials };
}

export async function saveUsgsLogin(username: string, applicationToken: string): Promise<SimpleOk> {
  const credentials = tauriCredentialBridge();
  if (credentials) {
    const result = await credentials.saveUsgsLogin(username, applicationToken);
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, status: result.status };
  }
  if (!username.trim() || !applicationToken) {
    return { ok: false, error: "USGS 用户名和 Application Token 不能为空", code: "DL004" };
  }
  mockCredentials = { ...mockCredentials, usgs: `login:${username.trim()[0] ?? "*"}***` };
  notifyContextChanged();
  return { ok: true, status: mockCredentials };
}

export async function clearUsgsCredentials(): Promise<SimpleOk> {
  const credentials = tauriCredentialBridge();
  if (credentials) {
    const result = await credentials.clearUsgs();
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, removed: result.removed, status: result.status };
  }
  mockCredentials = { ...mockCredentials, usgs: "none" };
  notifyContextChanged();
  return { ok: true, removed: true, status: mockCredentials };
}

export async function checkEarthdataAuth(force = false): Promise<EarthdataAuthCheck | ApiError> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.checkEarthdataAuth(force);
    return result.ok ? result : { ok: false, error: result.error, code: result.code };
  }
  if (hasBridge()) return api().check_earthdata_auth(force);
  const configured = !["none", "unavailable"].includes(mockCredentials.earthdata);
  return {
    ok: true,
    configured,
    status: configured ? "valid" : "missing",
    message: configured ? "Earthdata/ASF 凭据正常。" : "未保存 Earthdata/ASF 凭据。",
  };
}

export async function saveEarthdataToken(token: string): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.saveEarthdataToken(token);
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().save_earthdata_token(token);
    if (res.ok) notifyContextChanged();
    return res;
  }
  if (!token.trim()) return { ok: false, error: "Token 不能为空", code: "DL004" };
  mockCredentials = {
    ...mockCredentials,
    earthdata: "token",
    earthdata_input: {
      mode: "token",
      token: "",
      username: "",
      password: "",
    },
  };
  notifyContextChanged();
  return { ok: true, status: mockCredentials };
}

export async function saveEarthdataLogin(username: string, password: string): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.saveEarthdataLogin(username, password);
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().save_earthdata_login(username, password);
    if (res.ok) notifyContextChanged();
    return res;
  }
  if (!username.trim() || !password) {
    return { ok: false, error: "用户名和密码都不能为空", code: "DL004" };
  }
  mockCredentials = {
    ...mockCredentials,
    earthdata: `login:${username.trim()[0] ?? "*"}***`,
    earthdata_input: {
      mode: "login",
      token: "",
      username: "",
      password: "",
    },
  };
  notifyContextChanged();
  return { ok: true, status: mockCredentials };
}

export async function clearEarthdataCredentials(): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.clearEarthdata();
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, removed: result.removed, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().clear_earthdata_credentials();
    if (res.ok) notifyContextChanged();
    return res;
  }
  mockCredentials = {
    ...mockCredentials,
    earthdata: "none",
    earthdata_input: { mode: "none", token: "", username: "", password: "" },
  };
  notifyContextChanged();
  return { ok: true, removed: true, status: mockCredentials };
}

export async function checkBhoonidhiAuth(): Promise<import("./desktopShellBridge").DesktopBhoonidhiAuthResult> {
  const credentials = tauriCredentialBridge();
  return credentials ? credentials.checkBhoonidhiAuth()
    : { ok: false, code: "V3_APPLICATION_UNAVAILABLE", error: "此检测需要原生桌面会话。" };
}

export async function getBhoonidhiSessionStatus(): Promise<import("./desktopShellBridge").DesktopBhoonidhiSessionStatusResult> {
  const credentials = tauriCredentialBridge();
  return credentials ? credentials.getBhoonidhiSessionStatus()
    : { ok: false, code: "V3_APPLICATION_UNAVAILABLE", error: "此状态需要原生桌面会话。" };
}

export async function revealSavedCredential(field: import("./desktopShellBridge").DesktopCredentialField): Promise<import("./desktopShellBridge").DesktopCredentialRevealResult> {
  const credentials = tauriCredentialBridge();
  return credentials ? credentials.reveal(field)
    : { ok: false, code: "V3_APPLICATION_UNAVAILABLE", error: "此读取需要原生桌面凭据存储。" };
}

export async function saveBhoonidhiLogin(username: string, password: string): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.saveBhoonidhiLogin(username, password);
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().save_bhoonidhi_login(username, password);
    if (res.ok) notifyContextChanged();
    return res;
  }
  if (!username.trim() || !password) {
    return { ok: false, error: "Bhoonidhi 用户名和密码不能为空", code: "DL004" };
  }
  mockCredentials = {
    ...mockCredentials,
    bhoonidhi: "login:" + (username.trim()[0] ?? "*") + "***",
  };
  notifyContextChanged();
  return { ok: true, status: mockCredentials };
}

export async function clearBhoonidhiCredentials(): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.clearBhoonidhi();
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, removed: result.removed, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().clear_bhoonidhi_credentials();
    if (res.ok) notifyContextChanged();
    return res;
  }
  mockCredentials = { ...mockCredentials, bhoonidhi: "none" };
  notifyContextChanged();
  return { ok: true, removed: true, status: mockCredentials };
}

export async function saveOpentopographyKey(apiKey: string): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.saveOpentopographyKey(apiKey);
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().save_opentopography_key(apiKey);
    if (res.ok) notifyContextChanged();
    return res;
  }
  if (!apiKey.trim()) return { ok: false, error: "API Key 不能为空", code: "DEM005" };
  mockCredentials = { ...mockCredentials, opentopography: "set" };
  notifyContextChanged();
  return { ok: true, status: mockCredentials };
}

export async function clearOpentopographyKey(): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.clearOpentopographyKey();
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, removed: result.removed, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().clear_opentopography_key();
    if (res.ok) notifyContextChanged();
    return res;
  }
  mockCredentials = { ...mockCredentials, opentopography: "none" };
  notifyContextChanged();
  return { ok: true, removed: true, status: mockCredentials };
}

export async function saveGacosEmail(email: string): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.saveGacosEmail(email);
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().save_gacos_email(email);
    if (res.ok) notifyContextChanged();
    return res;
  }
  if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email.trim())) {
    return { ok: false, error: "邮箱格式不正确", code: "GAC003" };
  }
  const [local, domain] = email.trim().split("@");
  mockCredentials = { ...mockCredentials, gacos: `${local[0] ?? "*"}***@${domain}` };
  notifyContextChanged();
  return { ok: true, status: mockCredentials };
}

export async function clearGacosEmail(): Promise<SimpleOk> {
  const desktopCredentials = tauriCredentialBridge();
  if (desktopCredentials) {
    const result = await desktopCredentials.clearGacosEmail();
    if (!result.ok) return { ok: false, error: result.error, code: result.code };
    notifyContextChanged();
    return { ok: true, removed: result.removed, status: result.status };
  }
  if (hasBridge()) {
    const res = await api().clear_gacos_email();
    if (res.ok) notifyContextChanged();
    return res;
  }
  mockCredentials = { ...mockCredentials, gacos: "none" };
  notifyContextChanged();
  return { ok: true, removed: true, status: mockCredentials };
}

export async function generateReport(outputDir = ""): Promise<ReportResult> {
  if (isTauriDesktopHost()) {
    const context = await getContext();
    if (!context.ok) return context;
    const runner = getDesktopShellBridge().application.reportGenerate;
    if (typeof runner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露报告生成接口", code: "GUI003" };
    }
    const result = recordValue(await runner({
      output_dir: outputDir,
      region_safe_name: context.region?.safe_name ?? "current_region",
      region_id: context.region?.region_id ?? null,
    }));
    if (result?.ok === true && recordValue(result.report) && typeof result.reports_dir === "string" && Array.isArray(result.included) && recordValue(result.paths)) {
      return result as unknown as ReportResult;
    }
    return {
      ok: false,
      error: String(result?.error ?? "ApplicationService 未返回有效报告"),
      code: typeof result?.code === "string" ? result.code : "REPORT_INVALID",
    };
  }
  if (hasBridge()) return api().generate_report(outputDir);
  if (!mock.region) return { ok: false, error: "请先创建或选择区域", code: "GUI002" };
  const dir = mockReportsDir();
  const safe = mock.region.safe_name;
  const included: string[] = [];
  if (mock.region.scene_count > 0) included.push("scene_check");
  if (mock.region.has_aoi) included.push("dem_planning", "dem_conversion");
  if (mock.region.has_aoi && mock.region.scene_count > 0) included.push("gacos_planning");
  return {
    ok: true,
    included,
    report: {
      title: `InSAR data preparation report: ${safe}`,
      created_at: new Date().toISOString(),
      ...(mockReport() as object),
    },
    reports_dir: dir,
    paths: {
      json: `${dir}\\${safe}_data_preparation_report.json`,
      markdown: `${dir}\\${safe}_data_preparation_report.md`,
      html: `${dir}\\${safe}_data_preparation_report.html`,
      manifest: `${dir}\\${safe}_manifest.csv`,
      warnings: `${dir}\\${safe}_warnings.csv`,
    },
  };
}

// --------------------------------------------------------------------- V3
export type ProviderInfo = Omit<ProviderMetadata, "source_kind" | "capabilities"> & {
  source_kind: string;
  capabilities: string[];
};
export type V3ProviderProduct = Omit<
  ProviderProduct,
  "source_kind" | "product_kind" | "acquisition_datetime" | "assets" | "payload"
> & {
  source_kind: string;
  product_kind: string;
  acquisition_datetime?: string | null;
  assets: any[];
  payload?: Record<string, unknown>;
  [key: string]: unknown;
};
export type V3PlanItem = Omit<ProviderPlanItem, "expected_size_bytes" | "properties"> & {
  expected_size_bytes?: number | null;
  properties?: Record<string, unknown>;
  [key: string]: unknown;
};
export type { ContractError, TaskEvent };

export type V3ProvidersResult = {
  ok: boolean;
  providers?: ProviderInfo[];
  error?: string;
  code?: string;
};

export type V3SearchRequest = {
  bbox?: Bbox;
  filters?: Record<string, unknown>;
  [key: string]: unknown;
};

export type V3SearchResult = {
  ok: boolean;
  products?: V3ProviderProduct[];
  error?: string;
  code?: string;
};

export type V3PlanRequest = {
  bbox?: Bbox;
  output_root?: string;
  filters?: Record<string, unknown>;
  [key: string]: unknown;
};

export type V3GacosFormFields = {
  N: string;
  S: string;
  W: string;
  E: string;
  H: string;
  M: string;
  date: string;
  type: string;
  seq: string;
  [key: string]: unknown;
};

export type V3GacosSubmissionBatch = {
  batch_id: string;
  batch_index: number;
  batch_count: number;
  date_count: number;
  dates: string[];
  date_text: string;
  bbox: Bbox | null;
  method: string;
  endpoint: string;
  content_type: string;
  form_fields: V3GacosFormFields;
  required_sensitive_fields: string[];
  [key: string]: unknown;
};

export type V3ExecutionMode = ProviderExecutionMode;

export type V3GacosSubmission = {
  kind: string;
  provider: string;
  execution_mode: V3ExecutionMode;
  portal_url: string;
  submit_endpoint: string;
  method: string;
  content_type: string;
  output_format: string;
  requires_user_confirmation: boolean;
  requires_email_delivery: boolean;
  email_field: string;
  result_delivery: string;
  download_link_handling: string;
  batches: V3GacosSubmissionBatch[];
  [key: string]: unknown;
};

export type V3Plan = Omit<
  ProviderPlan,
  | "items"
  | "requires_credentials"
  | "manual_action_required"
  | "execution_mode"
  | "submission"
  | "legacy_plan"
  | "legacy_report"
> & {
  items: V3PlanItem[];
  requires_credentials?: boolean;
  manual_action_required?: boolean;
  execution_mode?: V3ExecutionMode;
  submission?: V3GacosSubmission;
  legacy_plan?: Record<string, unknown>;
  legacy_report?: Record<string, unknown>;
  [key: string]: unknown;
};

export type V3PlanResult = {
  ok: boolean;
  plan?: V3Plan;
  error?: string;
  code?: string;
};

export type GacosWebFormDryRunBatch = {
  batch_id?: string;
  batch_index?: number;
  form_fields_masked: Record<string, unknown>;
  form_fields_real: Record<string, unknown> | null;
  curl_preview_masked: string;
  curl_preview_real: string;
};

export type GacosWebFormDryRunResult = {
  ok: boolean;
  offline?: boolean;
  provider?: string;
  portal_url?: string;
  submit_endpoint?: string;
  batches?: GacosWebFormDryRunBatch[];
  error?: string;
  code?: string;
};

export type GacosWebFormSubmitResult = {
  ok: boolean;
  error?: string;
  code?: string;
};

export async function getV3Providers(): Promise<V3ProvidersResult> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("get_v3_providers");
  if (hasBridge()) {
    const getter = api().get_v3_providers;
    if (typeof getter === "function") return getter();
    return { ok: false, error: "当前桌面 API 未暴露 get_v3_providers", code: "GUI003" };
  }
  return { ok: true, providers: mockV3Providers() };
}

export async function getSarProviderCapabilities(): Promise<SarCapabilitiesResult> {
  if (isTauriDesktopHost()) {
    const result = await getDesktopShellBridge().application.providerCapabilities();
    if (!result.ok) return applicationError(result);
    const capabilities = result.capabilities;
    if (
      typeof capabilities.provider_id !== "string" ||
      typeof capabilities.available !== "boolean" ||
      !Array.isArray(capabilities.datasets) ||
      !Array.isArray(capabilities.sources) ||
      typeof capabilities.source_version !== "string"
    ) {
      return applicationProjectionError();
    }
    return { ok: true, capabilities: capabilities as ProviderCapabilitiesResponse };
  }
  if (hasBridge()) {
    const getter = api().get_sar_provider_capabilities;
    if (typeof getter === "function") return getter();
    return { ok: false, error: "Desktop API does not expose SAR capabilities", code: "GUI003" };
  }
  return mockSarProviderCapabilities();
}

function hasSarCapabilitiesApi(): boolean {
  return hasBridge() && typeof api().get_sar_provider_capabilities === "function";
}

let sarCapabilitiesApiSeen = false;

function observeSarCapabilitiesApi(): boolean {
  const ready = hasSarCapabilitiesApi();
  if (ready && !sarCapabilitiesApiSeen) {
    sarCapabilitiesApiSeen = true;
    notifyContextChanged();
  }
  return ready;
}

function waitForSarCapabilitiesApi(timeoutMs: number): Promise<boolean> {
  if (typeof window === "undefined") return Promise.resolve(false);
  if (observeSarCapabilitiesApi()) return Promise.resolve(true);
  return new Promise((resolve) => {
    let settled = false;
    const finish = (ready: boolean) => {
      if (settled) return;
      settled = true;
      window.clearInterval(intervalId);
      window.clearTimeout(timeoutId);
      window.removeEventListener("insar-desktop-ready", onReady);
      resolve(ready);
    };
    const onReady = () => {
      if (observeSarCapabilitiesApi()) finish(true);
    };
    const intervalId = window.setInterval(onReady, 40);
    const timeoutId = window.setTimeout(() => finish(observeSarCapabilitiesApi()), timeoutMs);
    window.addEventListener("insar-desktop-ready", onReady);
  });
}

export type SarCapabilitiesLoadOptions = {
  timeoutMs?: number;
  allowDevelopmentMock?: boolean;
};

let sarCapabilitiesLoadInFlight: Promise<SarCapabilitiesResult> | null = null;

/**
 * Resolve the capability surface at the desktop boundary.
 *
 * Legacy compatibility Worker injects its API after the page starts loading, so checking once can
 * leave the preview capability set mounted for the whole desktop session. Wait
 * for that late bridge first. Only a Vite development preview may fall back to
 * the browser capability set; packaged builds report an explicit bridge error.
 */
export function loadSarProviderCapabilities(
  options: SarCapabilitiesLoadOptions = {},
): Promise<SarCapabilitiesResult> {
  if (sarCapabilitiesLoadInFlight) return sarCapabilitiesLoadInFlight;
  const request = (async (): Promise<SarCapabilitiesResult> => {
    if (isTauriDesktopHost()) return getSarProviderCapabilities();
    const timeoutMs = options.timeoutMs ?? 1800;
    const allowDevelopmentMock =
      options.allowDevelopmentMock ??
      (typeof import.meta.env !== "undefined" && import.meta.env.DEV);
    if (await waitForSarCapabilitiesApi(timeoutMs)) return getSarProviderCapabilities();
    if (allowDevelopmentMock) return mockSarProviderCapabilities();
    return {
      ok: false,
      error: "SAR capabilities unavailable: desktop bridge was not ready before timeout",
      code: "GUI003",
    };
  })();
  const trackedRequest = request.finally(() => {
    if (sarCapabilitiesLoadInFlight === trackedRequest) {
      sarCapabilitiesLoadInFlight = null;
    }
  });
  sarCapabilitiesLoadInFlight = trackedRequest;
  return trackedRequest;
}

function mockSarProviderCapabilities(): SarCapabilitiesResult {
  const compatibility = JSON.parse(
    JSON.stringify(asfCapabilitiesFixture),
  ) as ProviderCapabilitiesResponse;
  const bindings = (
    resourceRegistryFixture as unknown as ResourceRegistryFixture
  ).resources.flatMap((resource) => {
    if (resource.resource_kind !== "science_dataset") return [];
    const scienceResource = resource as RegistryScienceResource;
    const search = scienceResource.science.sar_search;
    if (scienceResource.common.lifecycle !== "implemented" || search === null) return [];
    const route = scienceResource.common.provider_routes.find(
      (candidate) =>
        candidate.route_id === search.provider_route_id && candidate.lifecycle === "implemented",
    );
    const capability = scienceResource.common.capabilities.find(
      (candidate) =>
        candidate.kind === "search" &&
        candidate.lifecycle === "implemented" &&
        candidate.provider_route_ids.includes(search.provider_route_id),
    );
    if (route === undefined || capability === undefined) return [];
    return [{ resource: scienceResource, search, route }];
  });
  bindings.sort((left, right) => left.search.projection_order - right.search.projection_order);
  const datasets = bindings.flatMap(({ resource, search, route }): DatasetCapabilities[] => {
    if (search.projection_kind !== "legacy_asf_search" || route.provider_id !== "asf.catalog") {
      return [];
    }
    const {
      provider_route_id: _providerRouteId,
      selector,
      projection_kind: _projectionKind,
      projection_order: _projectionOrder,
      logical_dataset: _logicalDataset,
      source_filters: _sourceFilters,
      ...capabilities
    } = search;
    return [{ dataset: selector, display_name: resource.common.display_name, ...capabilities }];
  });
  const sources: SarSourceCapability[] = bindings.map(({ resource, search, route }) => {
    const typed = search.projection_kind === "typed_catalog_profile";
    return {
      selector: search.selector,
      display_name: resource.common.display_name,
      search_kind: search.projection_kind,
      logical_dataset: search.logical_dataset,
      provider_id: route.provider_id,
      native_collection_id: typed ? route.native_collection_id : null,
      collection_profile_id: typed ? route.implementation_binding_id : null,
      available: compatibility.available,
      disabled_reason: compatibility.reason,
      filters: typed
        ? (search.source_filters ?? {})
        : {
            legacy_dataset: search.selector,
            dataset_capabilities: datasets.find((item) => item.dataset === search.selector),
          },
      supported_actions: search.supported_actions.map((action) => action.action),
    };
  });
  return {
    ok: true,
    capabilities: {
      ...compatibility,
      datasets,
      sources,
    },
  };
}

export type M1SarSearchFreezeOptions = {
  request_id: string;
  batch_id: string;
  created_at: string;
  aoi_name: string;
  aoi_geojson: Json | null;
  page_size: number;
};

export async function searchSarProducts(
  params: SarSearchParams | SarSearchQuery,
  m1Options?: M1SarSearchFreezeOptions,
): Promise<SarSearchResult> {
  if (isTauriDesktopHost()) {
    if (!m1Options || typeof params !== "object" || params === null) {
      return {
        ok: false,
        error: "单一 Tauri 宿主检索 requires an explicit frozen AOI and batch identity",
        code: "V3_APPLICATION_INVALID_REQUEST",
      };
    }
    const query = completeSarSearchQuery(params);
    activeM1SearchRequestId = m1Options.request_id;
    const result = await getDesktopShellBridge().application.sarSearchAndFreeze({
      request_id: m1Options.request_id,
      search: { query: query as unknown as Json },
      batch_id: m1Options.batch_id,
      created_at: m1Options.created_at,
      aoi_name: m1Options.aoi_name,
      aoi_geojson: m1Options.aoi_geojson,
      page_size: m1Options.page_size,
    });
    if (activeM1SearchRequestId === m1Options.request_id) activeM1SearchRequestId = null;
    if (!result.ok) return applicationError(result);
    const core = projectApplicationCoreSnapshot(result);
    if (!core.ok || !core.core.workbench) return core.ok ? applicationProjectionError() : core;
    const summary = core.core.workbench;
    const baseContext = batchContextFromCoreWorkbench(summary);
    const pageRequest = {
      batch_id: summary.batch_id,
      expected_revision: summary.revision,
      page_index: summary.page_index,
      page_size: MAX_UI_WORKBENCH_PAGE_SIZE,
    };
    const page = await loadApplicationWorkbenchPage(pageRequest, core.aoi_display?.envelope);
    const workbench = applicationWorkbenchFromCorePage(core.core, page, {
      request: pageRequest,
      base: baseContext,
      aoiAuthority: core.aoi_display,
    });
    if (!workbench) return page.ok ? applicationProjectionError() : page;
    const products = workbench.scenes
      .map((scene) => scene.sar_product)
      .filter((product): product is SarProduct => Boolean(product));
    const limit = typeof query.max_results === "number" ? query.max_results : products.length;
    return {
      ok: true,
      products,
      errors: [],
      executed_query: query,
      stats: {
        requested_limit: limit,
        query_limit: limit,
        total_count: workbench.batch_context.total_count ?? products.length,
        candidate_count: workbench.batch_context.candidate_count ?? products.length,
        returned_count: products.length,
        source: "application_store",
        cmr_enriched: null,
      },
      m1_workbench: workbench,
    };
  }
  if (hasBridge()) {
    const searcher = api().search_sar_products;
    if (typeof searcher === "function") return searcher(params);
    return { ok: false, error: "Desktop API does not expose SAR search", code: "GUI003" };
  }
  return {
    ok: true,
    products: [],
    errors: [],
    executed_query: completeSarSearchQuery(params),
    stats: {
      requested_limit: params.max_results ?? 1000,
      query_limit: params.max_results ?? 1000,
      total_count: 0,
      candidate_count: 0,
      returned_count: 0,
      source: "mock",
      cmr_enriched: null,
    },
  };
}

export async function countFrozenSarBatchInBackground(
  request: DesktopSarBackgroundCountRequest,
): Promise<DesktopSarBackgroundCountResult> {
  if (!isTauriDesktopHost()) {
    return { ok: true, applied: false, total_count: null, snapshot: null };
  }
  return getDesktopShellBridge().application.sarBackgroundCount(request);
}

export async function restoreV3Downloads(): Promise<V3DownloadRestore> {
  if (isTauriDesktopHost()) {
    const result = await restoreApplicationCore(false);
    if (!result.ok) return result;
    const summary = result.core.workbench;
    if (!summary) {
      return { ok: true, enabled: true, workbench: null };
    }
    const baseContext = batchContextFromCoreWorkbench(summary);
    const pageRequest = {
      batch_id: summary.batch_id,
      expected_revision: summary.revision,
      page_index: summary.page_index,
      page_size: MAX_UI_WORKBENCH_PAGE_SIZE,
    };
    const page = await loadApplicationWorkbenchPage(pageRequest, result.aoi_display?.envelope);
    const workbench = applicationWorkbenchFromCorePage(result.core, page, {
      request: pageRequest,
      base: baseContext,
      aoiAuthority: result.aoi_display,
    });
    if (!workbench) return page.ok ? applicationProjectionError() : page;
    return {
      ok: true,
      enabled: true,
      workbench: workbench
        ? {
            batch_context: workbench.batch_context,
            scenes: workbench.scenes,
            checked_item_ids: workbench.checked_item_ids,
            revision: workbench.batch_context.revision ?? 0,
          }
        : null,
    };
  }
  if (hasBridge()) {
    const getter = api().get_v3_download_restore;
    if (getter) return getter();
  }
  return { ok: true, enabled: false, workbench: null };
}

export async function getV3StoreSettings(): Promise<V3StoreSettingsResult> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("get_v3_store_settings");
  if (hasBridge()) {
    const getter = api().get_v3_store_settings;
    if (!getter) {
      return {
        ok: false,
        error: "当前桌面后端不支持 Rust 任务数据库位置管理。",
        code: "V3_STORE_SETTINGS_UNAVAILABLE",
      };
    }
    return getter();
  }
  return mockV3StoreSettings;
}

export async function stageV3StoreMigration(
  targetDirectory: string,
): Promise<V3StoreSettingsResult | SimpleOk> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("stage_v3_store_migration");
  if (hasBridge()) {
    const stage = api().stage_v3_store_migration;
    if (!stage) {
      return {
        ok: false,
        error: "当前桌面后端不支持 Rust 任务数据库迁移。",
        code: "V3_STORE_SETTINGS_UNAVAILABLE",
      };
    }
    return stage(targetDirectory);
  }
  const target = targetDirectory.replace(/[\\/]+$/, "");
  mockV3StoreSettings = {
    ...mockV3StoreSettings,
    pending_path: `${target}\\tasks.sqlite3`,
    restart_required: true,
  };
  return mockV3StoreSettings;
}

export async function getAsfTaskWorkbench(taskId: string): Promise<TaskWorkbenchResult> {
  if (isTauriDesktopHost()) {
    const result = await coreTaskForId(taskId, true);
    if (!result.ok) return result;
    const page = await loadApplicationTaskPage({
      task_id: result.task.task_id,
      expected_revision: result.task.revision,
      page_index: 0,
      page_size: MAX_UI_TASK_PAGE_SIZE,
    });
    if (!page.ok) return page;
    const selectedSceneIds = page.scenes.map(sceneRowKey);
    const checkedSceneIds = page.scenes
      .filter((scene) => page.checked_item_ids.includes(sceneDownloadItemId(scene)))
      .map(sceneRowKey);
    const aoiName = result.task.operation?.kind === "dem"
      ? "DEM 任务"
      : result.task.operation?.kind === "orbit"
        ? "轨道任务"
        : "任务 AOI";
    return {
      ok: true,
      workbench: {
        task_id: result.task.task_id,
        provider_id: result.task.provider_id,
        batch_context: {
          batch_id: result.task.batch_id,
          source_batch_id: result.task.batch_id,
          task_batch_id: result.task.task_id,
          created_at: result.task.created_at,
          aoi_name: aoiName,
          aoi_geojson: null,
          query: { provider: result.task.provider_id },
          page_index: page.page.page_index,
          page_size: page.page.page_size,
          revision: page.page.revision,
          total_count: page.page.total_count,
          candidate_count: page.page.total_count,
          checked_count: page.checked_item_ids.length,
          checked_item_ids: page.checked_item_ids,
        },
        scenes: page.scenes,
        checked_scene_ids: checkedSceneIds,
        selected_scene_ids: selectedSceneIds,
        output_dir: result.task.output_root,
        download_layout: result.task.layout,
        use_product_subdirs: result.task.layout !== "direct",
      },
    };
  }
  if (hasBridge()) {
    const getter = api().get_asf_task_workbench;
    if (typeof getter === "function") return getter(taskId);
    return {
      ok: false,
      error: "Desktop API does not expose the task workbench reader",
      code: "GUI003",
    };
  }
  return {
    ok: false,
    error: "任务工作台只能从桌面任务存储读取",
    code: "GUI003",
  };
}

export async function saveV3SearchWorkbench(
  scenes: SceneRow[],
  checkedItemIds: string[],
  batchContext: DownloadBatchContext,
): Promise<SimpleOk & { m1_workbench?: ApplicationWorkbenchProjection }> {
  if (isTauriDesktopHost()) {
    return {
      ok: false,
      error: "Tauri Workbench selection is Store-owned; use the typed selection command instead of full-workbench persistence.",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  if (hasBridge()) {
    const saver = api().save_v3_search_workbench;
    if (saver) return saver(compactSearchWorkbenchRows(scenes), checkedItemIds, batchContext);
  }
  return { ok: true };
}

export { compactSearchWorkbenchRows } from "./searchWorkbenchPersistence.ts";

export async function saveV3TaskView(
  taskId: string,
  checkedItemIds: string[],
  pageIndex: number,
  pageSize: number,
): Promise<SimpleOk & { m1_workbench?: DownloadTaskWorkbench }> {
  if (isTauriDesktopHost()) {
    return {
      ok: false,
      error: "Tauri task-page selection is Store-owned; this legacy full-task view writer is disabled until a bounded task-page command exists.",
      code: "V3_APPLICATION_INVALID_REQUEST",
    };
  }
  if (hasBridge()) {
    const saver = api().save_v3_task_view;
    if (saver) return saver(taskId, checkedItemIds, pageIndex, pageSize);
    return { ok: false, error: "Desktop API does not expose task-view persistence", code: "GUI003" };
  }
  return { ok: true };
}

/** Archive a terminal M1 task without deleting its user output. */
export async function archiveM1DownloadTask(
  taskId: string,
  expectedRevision: number,
): Promise<SimpleOk> {
  if (!isTauriDesktopHost()) return { ok: false, error: "单一 Tauri 宿主未就绪", code: "V3_APPLICATION_UNAVAILABLE" };
  const result = await getDesktopShellBridge().application.archiveDownloadTask({
    task_id: taskId,
    expected_revision: expectedRevision,
    archived_at: new Date().toISOString(),
  });
  return result.ok ? { ok: true } : applicationError(result);
}

export async function searchV3Provider(
  providerId: string,
  query: V3SearchRequest = {},
): Promise<V3SearchResult> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("v3_search_provider");
  if (hasBridge()) {
    const searcher = api().v3_search_provider;
    if (typeof searcher === "function") return searcher(providerId, query);
    return { ok: false, error: "当前桌面 API 未暴露 v3_search_provider", code: "GUI003" };
  }
  if (!mockV3Providers().some((provider) => provider.provider_id === providerId)) {
    return { ok: false, error: `unknown provider '${providerId}'`, code: "GUI003" };
  }
  return { ok: true, products: mockV3Product(providerId, query) };
}

export async function planV3Provider(
  providerId: string,
  query: V3PlanRequest = {},
  products?: V3ProviderProduct[],
): Promise<V3PlanResult> {
  if (isTauriDesktopHost()) return tauriLegacyUnavailable("v3_plan_provider");
  if (hasBridge()) {
    const planner = api().v3_plan_provider;
    if (typeof planner === "function") return planner(providerId, query, products);
    return { ok: false, error: "当前桌面 API 未暴露 v3_plan_provider", code: "GUI003" };
  }
  if (!mockV3Providers().some((provider) => provider.provider_id === providerId)) {
    return { ok: false, error: `unknown provider '${providerId}'`, code: "GUI003" };
  }
  const bbox = v3QueryBbox(query);
  const outputRoot = v3OutputRoot(query);
  
  let productList = products?.length ? products : mockV3Product(providerId, query);
  const manualDates = query.filters?.dates;
  const hasManualDates = Array.isArray(manualDates) && manualDates.length > 0;
  if (providerId === "gacos.atmosphere" && !productList.length && hasManualDates) {
    productList = manualDates.map((d: string) => ({
      provider_id: providerId,
      product_id: `GACOS:${d}`,
      display_name: `GACOS:${d}`,
      source_kind: "gacos",
      product_kind: "ztd",
      acquisition_datetime: `${d.substring(0, 4)}-${d.substring(4, 6)}-${d.substring(6, 8)}T00:00:00Z`,
      footprint_bbox: bbox,
      assets: [],
      properties: {},
    }));
  }

  if ((providerId === "opentopography.dem" || providerId === "gacos.atmosphere") && !bbox) {
    return { ok: false, error: "provider planning requires processing_aoi or bbox", code: "AOI001" };
  }
  if (providerId === "gacos.atmosphere" && !productList.length && !mockActiveSceneCount() && !hasManualDates) {
    return { ok: false, error: "GACOS planning requires at least one scene", code: "GAC001" };
  }
  if (providerId === "opentopography.dem") {
    const dataset = String(query.filters?.dataset || productList[0]?.properties?.dataset || "COP30");
    return {
      ok: true,
      plan: {
        provider_id: providerId,
        source_kind: "dem",
        output_root: outputRoot,
        items: [
          { product_id: `DEM:${dataset}:raw`, target_path: `${outputRoot}\\DEM\\${dataset}.tif`, role: "primary", status: "planned" },
          { product_id: `DEM:${dataset}:ellipsoid`, target_path: `${outputRoot}\\DEM\\${dataset}_ellipsoid.tif`, role: "derived", status: "planned" },
          { product_id: `DEM:${dataset}:sarscape`, target_path: `${outputRoot}\\DEM\\${dataset}_dem`, role: "derived", status: "planned" },
        ],
        requires_credentials: true,
        manual_action_required: false,
      },
    };
  }
  const items = productList.map((product: any) => ({
    product_id: product.product_id || product.display_name || "product",
    target_path:
      providerId === "gacos.atmosphere"
        ? `${outputRoot}\\GACOS\\requests\\${String(product.product_id || "GACOS:mock").replace(/^GACOS:/, "")}.ztd`
        : `${outputRoot}\\SAR_Data\\SLC\\${product.product_id || "scene"}.zip`,
    role: "primary",
    status: "planned",
  }));
  const isGacos = providerId === "gacos.atmosphere";
  const gacosDates = productList
    .map((product: any) => String(product.product_id || "").replace(/^GACOS:/, "").trim())
    .filter(Boolean);
  const gacosDateText = gacosDates.join("\n");
  return {
    ok: true,
    plan: {
      provider_id: providerId,
      source_kind: isGacos ? "gacos" : "sentinel1",
      output_root: outputRoot,
      items,
      requires_credentials: true,
      manual_action_required: isGacos,
      execution_mode: isGacos ? "browser_assisted_web_form" : "direct_download",
      ...(isGacos
        ? {
            submission: {
              kind: "web_form_submission",
              provider: "gacos",
              execution_mode: "browser_assisted_web_form",
              portal_url: "http://www.gacos.net/",
              submit_endpoint: "http://www.gacos.net/M/action_page.php",
              method: "POST",
              content_type: "application/x-www-form-urlencoded",
              output_format: "geotiff",
              requires_user_confirmation: true,
              requires_email_delivery: true,
              email_field: "email",
              result_delivery: "email_link",
              download_link_handling: "paste_email_link_then_import",
              batches: [
                {
                  batch_id: "mock-gacos-batch-1",
                  batch_index: 1,
                  batch_count: 1,
                  date_count: gacosDates.length,
                  dates: gacosDates,
                  date_text: gacosDateText,
                  bbox,
                  method: "POST",
                  endpoint: "http://www.gacos.net/M/action_page.php",
                  content_type: "application/x-www-form-urlencoded",
                  form_fields: {
                    N: String(bbox?.north ?? ""),
                    S: String(bbox?.south ?? ""),
                    W: String(bbox?.west ?? ""),
                    E: String(bbox?.east ?? ""),
                    H: "0",
                    M: "0",
                    date: gacosDateText,
                    type: "2",
                    seq: "OSM Map",
                  },
                  required_sensitive_fields: ["email"],
                },
              ],
            },
          }
        : {}),
    },
  };
}

export async function gacosWebFormDryRun(
  payload: V3GacosSubmission,
  _email = "",
): Promise<GacosWebFormDryRunResult> {
  if (isTauriDesktopHost()) {
    const runner = getDesktopShellBridge().application.gacosDryRun;
    if (typeof runner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露 GACOS dry-run 接口", code: "GUI003" };
    }
    const result = recordValue(await runner({ submission: payload as unknown as Json }));
    if (result?.ok === true && Array.isArray(result.batches)) {
      return {
        ok: true,
        offline: result.offline === true,
        provider: typeof result.provider === "string" ? result.provider : "GACOS",
        portal_url: typeof result.portal_url === "string" ? result.portal_url : undefined,
        submit_endpoint: typeof result.submit_endpoint === "string" ? result.submit_endpoint : undefined,
        batches: result.batches as GacosWebFormDryRunResult["batches"],
      };
    }
    return {
      ok: false,
      error: String(result?.error ?? "兼容 Worker 未返回有效 GACOS dry-run"),
      code: typeof result?.code === "string" ? result.code : "GACOS_DRY_RUN_INVALID",
    };
  }
  if (hasBridge()) {
    const runner = api().gacos_webform_dry_run;
    if (typeof runner === "function") return runner(payload);
    return { ok: false, error: "当前桌面 API 未暴露 gacos_webform_dry_run", code: "GUI003" };
  }
  // Mock browser implementation
  const batches = payload.batches || [];
  const dryBatches = batches.map((batch: any) => {
    const maskedFields = { ...batch.form_fields };
    if (batch.required_sensitive_fields?.includes("email")) {
      maskedFields.email = "<EMAIL>";
    }
    const masked_form_data = Object.entries(maskedFields)
      .map(([k, v]) => `-d "${k}=${encodeURIComponent(String(v))}"`)
      .join(" \\\n  ");
    const masked_curl = `curl -X POST "${batch.endpoint || "http://www.gacos.net/M/action_page.php"}" \\\n  -H "Content-Type: application/x-www-form-urlencoded" \\\n  ${masked_form_data}`;

    return {
      batch_id: batch.batch_id,
      batch_index: batch.batch_index,
      form_fields_masked: maskedFields,
      form_fields_real: null,
      curl_preview_masked: masked_curl,
      curl_preview_real: "",
    };
  });
  return {
    ok: true,
    provider: "gacos",
    batches: dryBatches,
  };
}

export async function gacosWebFormSubmit(
  payload: V3GacosSubmission | undefined,
  _email = "",
  confirm = false,
  forceReapply = false,
  continueAfterFailure = false,
): Promise<GacosWebFormSubmitResult> {
  if (!payload) return { ok: false, error: "缺少 GACOS 提交参数", code: "GAC001" };
  void payload;
  void forceReapply;
  void continueAfterFailure;
  return confirm
    ? { ok: false, error: "当前版本只允许 GACOS 离线规划与 dry-run，未实现 live submit", code: "GACOS_LIVE_SUBMIT_UNAVAILABLE" }
    : { ok: false, error: "需要最终确认后才能提交 GACOS 请求", code: "GAC003" };
}

export async function previewAoiFileBundle(
  files: { name: string; base64: string }[],
): Promise<AoiPreviewResult> {
  if (isTauriDesktopHost()) {
    const runner = getDesktopShellBridge().application.aoiImport;
    if (typeof runner !== "function") {
      return { ok: false, error: "当前桌面 API 未暴露 AOI Bundle 导入接口", code: "GUI003" };
    }
    const value = recordValue(await runner({ source: "bundle", files }));
    if (value?.ok === true && Array.isArray(value.features) && typeof value.total_features === "number") {
      return value as unknown as AoiPreviewResult;
    }
    return {
      ok: false,
      error: String(value?.error ?? "兼容 Worker 未返回有效 AOI Bundle 预览"),
      code: typeof value?.code === "string" ? value.code : "AOI_PREVIEW_INVALID",
    };
  }
  if (hasBridge()) {
    const previewer = api().preview_aoi_file_bundle;
    if (typeof previewer === "function") return previewer(files);
    return { ok: false, error: "当前桌面 API 未暴露 preview_aoi_file_bundle", code: "GUI003" };
  }
  return { ok: false, error: "Mock preview_aoi_file_bundle not implemented", code: "GUI003" };
}

function mockV3Providers() {
  return [
    { provider_id: "asf.sentinel1", display_name: "ASF Sentinel-1", source_kind: "sentinel1", version: "0.1.0", capabilities: ["search", "plan", "download", "credentials"] },
    { provider_id: "opentopography.dem", display_name: "OpenTopography DEM", source_kind: "dem", version: "0.1.0", capabilities: ["search", "plan", "download", "credentials"] },
    { provider_id: "gacos.atmosphere", display_name: "GACOS Atmospheric Delay", source_kind: "gacos", version: "0.1.0", capabilities: ["search", "plan", "credentials", "atmospheric_correction"] },
  ];
}

function mockV3Product(providerId: string, query: any) {
  const bbox = v3QueryBbox(query) || { west: 110, east: 111, south: 30, north: 31, crs: "EPSG:4326" };
  if (providerId === "asf.sentinel1") {
    const sources = query.filters?.scene_sources || ["S1A_IW_SLC__1SDV_20240101T000000_20240101T000027_052000_064ABC_1234"];
    return sources.map((id: string) => ({
      provider_id: providerId,
      product_id: id,
      display_name: id,
      source_kind: "sentinel1",
      product_kind: "sar_slc",
      acquisition_datetime: "2024-01-01T00:00:00Z",
      footprint_bbox: bbox,
      assets: [{ role: "primary", file_name: `${id}.zip`, media_type: "application/zip", url: "https://example.invalid" }],
      properties: { platform: "S1A", product_type: "SLC", beam_mode: "IW", polarization: "VV" },
    }));
  }
  if (providerId === "opentopography.dem") {
    const dataset = query.filters?.dataset || "COP30";
    return [{
      provider_id: providerId,
      product_id: `DEM:${dataset}`,
      display_name: `${dataset} DEM`,
      source_kind: "dem",
      product_kind: "dem_raster",
      footprint_bbox: bbox,
      assets: [{ role: "primary", file_name: `${dataset}.tif`, media_type: "image/tiff" }],
      properties: { dataset, provider: "OPENTOPOGRAPHY" },
    }];
  }
  if (providerId === "gacos.atmosphere") {
    const dates = ["20240101", "20240113"];
    return dates.map((day) => ({
      provider_id: providerId,
      product_id: `GACOS:${day}`,
      display_name: `GACOS ZTD ${day}`,
      source_kind: "gacos",
      product_kind: "atmosphere_delay",
      acquisition_datetime: `${day.slice(0,4)}-${day.slice(4,6)}-${day.slice(6,8)}T00:00:00Z`,
      footprint_bbox: bbox,
      assets: [{ role: "primary", file_name: `${day}.ztd`, media_type: "application/octet-stream" }],
      properties: { date: `${day.slice(0,4)}-${day.slice(4,6)}-${day.slice(6,8)}` },
    }));
  }
  return [];
}

function v3QueryBbox(query: any) {
  if (query?.bbox) return query.bbox;
  return mock.region?.bbox || null;
}

export type {
  DesktopSourceMutationCommitStatus,
  DesktopSourceMutationTargetStatus,
  DesktopSourceMutationStatus,
  DesktopSourceMutationStatusResult,
  DesktopSourceMutationQuery,
  DesktopNisarArchivedRecoveryV2Request,
};

function querySourceStatusFallback(): DesktopSourceMutationStatusResult {
  return {
    ok: false,
    error: "来源操作状态查询需要原生桌面应用支持",
    code: "V3_APPLICATION_UNAVAILABLE",
  };
}

export async function querySourceMutationStatus(
  query: DesktopSourceMutationQuery,
): Promise<DesktopSourceMutationStatusResult> {
  if (!isTauriDesktopHost()) return querySourceStatusFallback();
  const bridge = getDesktopShellBridge();
  if (typeof bridge.application.querySourceMutationStatus === "function") {
    return bridge.application.querySourceMutationStatus(query);
  }
  return querySourceStatusFallback();
}

export async function querySourceAppendStatus(
  request: DesktopAppendTaskFromSourceBatchRequest,
): Promise<DesktopSourceMutationStatusResult> {
  if (!isTauriDesktopHost()) return querySourceStatusFallback();
  const bridge = getDesktopShellBridge();
  if (typeof bridge.application.querySourceAppendStatus === "function") {
    return bridge.application.querySourceAppendStatus(request);
  }
  return querySourceStatusFallback();
}

export async function queryHistoricalSuccessorStatus(
  request: DesktopCreateTaskFromHistoricalSourceBatchRequest,
): Promise<DesktopSourceMutationStatusResult> {
  if (!isTauriDesktopHost()) return querySourceStatusFallback();
  const bridge = getDesktopShellBridge();
  if (typeof bridge.application.queryHistoricalSuccessorStatus === "function") {
    return bridge.application.queryHistoricalSuccessorStatus(request);
  }
  return querySourceStatusFallback();
}

export async function queryBurstAppendStatus(
  request: DesktopBurstAppendAssetTaskRequest,
): Promise<DesktopSourceMutationStatusResult> {
  if (!isTauriDesktopHost()) return querySourceStatusFallback();
  const bridge = getDesktopShellBridge();
  if (typeof bridge.application.queryBurstAppendStatus === "function") {
    return bridge.application.queryBurstAppendStatus(request);
  }
  return querySourceStatusFallback();
}

export async function queryBurstSuccessorStatus(
  request: DesktopBurstCreateHistoricalAssetTaskRequest,
): Promise<DesktopSourceMutationStatusResult> {
  if (!isTauriDesktopHost()) return querySourceStatusFallback();
  const bridge = getDesktopShellBridge();
  if (typeof bridge.application.queryBurstSuccessorStatus === "function") {
    return bridge.application.queryBurstSuccessorStatus(request);
  }
  return querySourceStatusFallback();
}

export async function queryNisarSuccessorStatus(
  request: DesktopNisarSuccessorTaskV2Request,
): Promise<DesktopSourceMutationStatusResult> {
  if (!isTauriDesktopHost()) return querySourceStatusFallback();
  const bridge = getDesktopShellBridge();
  if (typeof bridge.application.queryNisarSuccessorStatus === "function") {
    return bridge.application.queryNisarSuccessorStatus(request);
  }
  return querySourceStatusFallback();
}

export async function queryNisarArchivedRecoveryStatus(
  request: DesktopNisarArchivedRecoveryV2Request,
): Promise<DesktopSourceMutationStatusResult> {
  if (!isTauriDesktopHost()) return querySourceStatusFallback();
  const bridge = getDesktopShellBridge();
  if (typeof bridge.application.queryNisarArchivedRecoveryStatus === "function") {
    return bridge.application.queryNisarArchivedRecoveryStatus(request);
  }
  return querySourceStatusFallback();
}

export async function queryReacquireStatus(
  request: DesktopUpdateDownloadTaskRequest,
): Promise<DesktopSourceMutationStatusResult> {
  if (!isTauriDesktopHost()) return querySourceStatusFallback();
  const bridge = getDesktopShellBridge();
  if (typeof bridge.application.queryReacquireStatus === "function") {
    return bridge.application.queryReacquireStatus(request);
  }
  return querySourceStatusFallback();
}

function v3OutputRoot(query: any) {
  if (query?.output_root) return query.output_root;
  return mock.region?.root || mock.project?.root || mock.workspace?.root || "C:\\InSAR";
}

export interface RuntimeComponentItem {
  id: string;
  name: string;
  kind: string;
  version: string;
  install_root: string;
  status: "ready" | "partial" | "missing" | string;
  file_count: number;
  total_bytes: number;
  verified: boolean;
  detail: string;
  capabilities: string[];
}

export interface RuntimeComponentsReport {
  ok: boolean;
  install_root: string;
  manifest_found: boolean;
  app_version: string;
  host_platform?: string;
  components: RuntimeComponentItem[];
}

export interface ExtensionPlatformPackage {
  package: string;
  sha256: string;
  size: number;
}

export interface ExtensionRegistrySkillPayload {
  id: string;
  name_zh: string;
  name_en: string;
  category: string;
  description_zh: string;
  description_en: string;
  prompt_template: string;
}

export interface ExtensionRegistryEntry {
  id: string;
  name: string;
  kind: "component" | "skill" | string;
  version: string;
  description: string;
  package: string;
  sha256: string;
  size: number;
  install_root: string;
  capabilities: string[];
  platforms?: string[];
  platform_packages?: Record<string, ExtensionPlatformPackage>;
  skill?: ExtensionRegistrySkillPayload | null;
}

export interface ExtensionRegistryReport {
  ok: boolean;
  source_url: string;
  updated_at: string;
  entries: ExtensionRegistryEntry[];
  blocked_count: number;
}

export interface RuntimeComponentInstallRequest {
  component_id: string;
  source: string;
  expected_sha256?: string | null;
}

export interface RuntimeComponentInstallResult {
  ok: boolean;
  component_id: string;
  version: string;
  sha256: string;
  bytes_installed: number;
  install_path: string;
  status: string;
  detail: string;
}

export interface ExtensionMirrorPreset {
  id: string;
  labelKey: string;
  url: string;
}

export const EXTENSION_MIRROR_PRESETS: readonly ExtensionMirrorPreset[] = [
  {
    id: "gitcode",
    labelKey: "shell.pluginsPage.mirrorGitcode",
    url: "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/raw/registry.json",
  },
  {
    id: "github",
    labelKey: "shell.pluginsPage.mirrorGithub",
    url: "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json",
  },
  {
    id: "local",
    labelKey: "shell.pluginsPage.mirrorLocal",
    url: "local",
  },
];

const EXTENSION_MIRROR_PRESET_STORAGE_KEY = "eostage.runtime.mirrorPreset.v1";
const EXTENSION_REGISTRY_URL_STORAGE_KEY = "eostage.runtime.registryUrl.v1";
export const EXTENSION_MIRROR_CHANGED_EVENT = "eostage:extension-mirror-changed";

export function normalizeExtensionMirrorSelection(
  presetId?: string | null,
  registryUrl?: string | null,
): { presetId: string; url: string } {
  const rawPreset = (presetId || "").trim();
  const rawUrl = (registryUrl || "").trim();
  if (rawPreset === "cloudflare" || rawUrl.includes("eostage-hub.pages.dev")) {
    return { presetId: "gitcode", url: EXTENSION_MIRROR_PRESETS[0].url };
  }
  const matchedPreset = EXTENSION_MIRROR_PRESETS.find((p) => p.id === rawPreset);
  if (matchedPreset) {
    return {
      presetId: matchedPreset.id,
      url: rawUrl || matchedPreset.url,
    };
  }
  if (rawUrl) {
    const byUrl = EXTENSION_MIRROR_PRESETS.find((p) => p.url === rawUrl);
    return {
      presetId: byUrl ? byUrl.id : "custom",
      url: rawUrl,
    };
  }
  return { presetId: "gitcode", url: EXTENSION_MIRROR_PRESETS[0].url };
}

export function loadPersistedExtensionMirror(): { presetId: string; url: string } {
  if (typeof window === "undefined" || !window.localStorage) {
    return { presetId: "gitcode", url: EXTENSION_MIRROR_PRESETS[0].url };
  }
  try {
    const savedPreset = window.localStorage.getItem(EXTENSION_MIRROR_PRESET_STORAGE_KEY);
    const savedUrl = window.localStorage.getItem(EXTENSION_REGISTRY_URL_STORAGE_KEY);
    return normalizeExtensionMirrorSelection(savedPreset, savedUrl);
  } catch {
    return { presetId: "gitcode", url: EXTENSION_MIRROR_PRESETS[0].url };
  }
}

export function savePersistedExtensionMirror(presetId: string, url: string): void {
  if (typeof window === "undefined" || !window.localStorage) return;
  try {
    const normalized = normalizeExtensionMirrorSelection(presetId, url);
    window.localStorage.setItem(EXTENSION_MIRROR_PRESET_STORAGE_KEY, normalized.presetId);
    window.localStorage.setItem(EXTENSION_REGISTRY_URL_STORAGE_KEY, normalized.url);
    const EventCtor = window.CustomEvent || (typeof CustomEvent === "function" ? CustomEvent : null);
    if (typeof window.dispatchEvent === "function") {
      if (EventCtor) {
        window.dispatchEvent(new EventCtor(EXTENSION_MIRROR_CHANGED_EVENT, { detail: normalized }));
      } else if (typeof window.Event === "function") {
        window.dispatchEvent(new window.Event(EXTENSION_MIRROR_CHANGED_EVENT));
      }
    }
  } catch {
    // ignore storage errors
  }
}

export async function fetchExtensionRegistry(
  registryUrl?: string,
): Promise<ExtensionRegistryReport> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (
      bridge?.application &&
      typeof (bridge.application as any).fetchExtensionRegistry === "function"
    ) {
      return await (bridge.application as any).fetchExtensionRegistry(registryUrl);
    }
  }
  const baseUrl =
    registryUrl && registryUrl.trim()
      ? registryUrl.trim()
      : "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/raw/registry.json";
  const baseDir =
    baseUrl === "local"
      ? "_hub_dist/"
      : baseUrl.includes("/")
        ? baseUrl.slice(0, baseUrl.lastIndexOf("/") + 1)
        : `${baseUrl}/`;
  return {
    ok: true,
    source_url: baseUrl,
    updated_at: "2026-11-05T12:00:00Z",
    blocked_count: 0,
    entries: [
      {
        id: "vertical-grid-egm96",
        name: "EGM96 大地水准面高程基准数据",
        kind: "component",
        version: "1.0.0",
        description: "NGA 15' 全球大地水准面高程基准网格 (EPSG:5773)",
        package: `${baseDir}components/vertical-grid-egm96-1.0.0.zip`,
        sha256: "db493027562c9b004d7220fa881f5603adada4e1c5029b933fa7de4547b0e78d",
        size: 2710815,
        install_root: "runtime/vertical-grids",
        capabilities: ["egm96"],
        platforms: ["windows-x86_64", "macos-aarch64", "macos-x86_64", "linux-x86_64", "linux-aarch64"],
      },
      {
        id: "vertical-grid-egm2008",
        name: "EGM2008 2.5分高精度全球大地水准面",
        kind: "component",
        version: "1.0.0",
        description: "NGA 2.5' 高精度全球大地水准面网格 (EPSG:3855)",
        package: `${baseDir}components/vertical-grid-egm2008-1.0.0.zip`,
        sha256: "4191d471eefebf24091b56dbc604353cb3b8cf8cc70e448bb9ae56a272bef17a",
        size: 80585622,
        install_root: "runtime/vertical-grids",
        capabilities: ["egm2008"],
        platforms: ["windows-x86_64", "macos-aarch64", "macos-x86_64", "linux-x86_64", "linux-aarch64"],
      },
      {
        id: "compat-worker",
        name: "Compatibility Worker 兼容性隔离工作进程",
        kind: "component",
        version: "1.0.0",
        description: "受控隔离进程（SARscape 交付与特定矢量/DEM 兼容桥接）",
        package: `${baseDir}components/compat-worker-1.0.0.zip`,
        sha256: "fd643c2412da8ba7799b422edc111ab7763b113f9f1857eb4e8b2a8838e202e8",
        size: 64903286,
        install_root: "runtime/compat-worker",
        capabilities: ["bounded-compatibility-operations"],
        platforms: ["windows-x86_64"],
      },
      {
        id: "skill-alos2-stripmap-insar",
        name: "ALOS-2 PALSAR-2 L波段植被穿透形变监测工作流",
        kind: "skill",
        version: "1.0.0",
        description: "针对高植被覆盖山区滑坡与同震形变的 ALOS-2 L波段条带模式检索与配对提示词技能",
        package: `${baseDir}skills/skill-alos2-stripmap-insar.json`,
        sha256: "8c3e1b4d9a6f0e2c5b7d8e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b",
        size: 1420,
        install_root: "localStorage",
        capabilities: ["sar", "alos2"],
        skill: {
          id: "hub-skill-alos2-stripmap-insar",
          name_zh: "ALOS-2 PALSAR-2 L波段植被穿透形变监测工作流",
          name_en: "ALOS-2 PALSAR-2 L-band Vegetation Penetration InSAR Workflow",
          category: "sar",
          description_zh:
            "针对高植被覆盖山区滑坡与同震形变的 ALOS-2 L波段条带模式（SM1/SM3）检索、极化筛选与干涉对组合工作流。",
          description_en:
            "ALOS-2 L-band stripmap search and interferometric pairing workflow for densely vegetated landslide and coseismic deformation.",
          prompt_template:
            "你是一位精通 L 波段（波长约 23.6 cm）合成孔径雷达干涉测量（InSAR）的专家。在高植被覆盖山区或大梯度同震形变场景下，C 波段容易发生时间失相干，请引导用户完成 ALOS-2 PALSAR-2 数据准备：\n1. 确认目标区域（working_aoi 或 context_aoi）与重点监测时段；\n2. 使用 search_start 检索 ALOS-2 产品，优先筛选 Fine Beam (SM1/SM3, HH 或 HH+HV 极化)，固定同一升降轨方向（ASCENDING 或 DESCENDING）及一致的 Path/Frame；\n3. 结合 COP30/NASADEM 高程数据，规划配套的椭球高 DEM 转换任务；\n4. 通过 propose_plan 提交完整检索与下载计划供用户在确认卡上审批。",
        },
      },
    ],
  };
}

export async function installRuntimeComponent(
  request: RuntimeComponentInstallRequest,
): Promise<RuntimeComponentInstallResult> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (
      bridge?.application &&
      typeof (bridge.application as any).installRuntimeComponent === "function"
    ) {
      return await (bridge.application as any).installRuntimeComponent(request);
    }
  }
  if (!request.source || !request.source.trim()) {
    throw new Error("安装包路径或镜像 URL 不能为空");
  }
  return {
    ok: true,
    component_id: request.component_id,
    version: "1.0.0",
    sha256:
      request.expected_sha256 ||
      "db493027562c9b004d7220fa881f5603adada4e1c5029b933fa7de4547b0e78d",
    bytes_installed: 2710815,
    install_path: `runtime/${request.component_id}`,
    status: "ready",
    detail: `已从 ${request.source} 完成校验并原子安装`,
  };
}

export async function getRuntimeComponents(): Promise<RuntimeComponentsReport> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).getRuntimeComponents === "function") {
      try {
        return await (bridge.application as any).getRuntimeComponents();
      } catch {
        // fallback
      }
    }
  }
  return {
    ok: true,
    install_root: "D:\\EOStage",
    manifest_found: true,
    app_version: "2.2.0-beta.0",
    components: [
      {
        id: "eostage-core",
        name: "EOStage 主程序与核心服务",
        kind: "application",
        version: "2.2.0-beta.0",
        install_root: ".",
        status: "ready",
        file_count: 1,
        total_bytes: 31457280,
        verified: true,
        detail: "Tauri 宿主与 Rust 核心服务",
        capabilities: ["desktop-ui", "workspace", "settings", "credential-keyring", "catalogue", "task-store", "download-orchestration"],
      },
      {
        id: "native-gis-runtime",
        name: "Native GIS 高级地理信息引擎 (GDAL / PROJ)",
        kind: "runtime",
        version: "3.9.0",
        install_root: "runtime/gis",
        status: "ready",
        file_count: 18,
        total_bytes: 83886080,
        verified: true,
        detail: "包含 GDAL/PROJ 私有动态库与投影定义",
        capabilities: ["dem-gdal-proj", "gtiff", "envi", "sarscape-output"],
      },
      {
        id: "vertical-grid-egm96",
        name: "EGM96 大地水准面高程基准数据",
        kind: "scientific_resource",
        version: "1.0.0",
        install_root: "runtime/vertical-grids",
        status: "ready",
        file_count: 1,
        total_bytes: 2936012,
        verified: true,
        detail: "全球高程基准网格 (15' 栅格, 2.80 MB)",
        capabilities: ["egm96"],
      },
      {
        id: "vertical-grid-egm2008",
        name: "EGM2008 2.5分高精度全球大地水准面",
        kind: "scientific_resource",
        version: "1.0.0",
        install_root: "runtime/vertical-grids",
        status: "missing",
        file_count: 0,
        total_bytes: 0,
        verified: false,
        detail: "EGM2008.tif 缺失 (可选高精度组件，约 117 MB)",
        capabilities: ["egm2008"],
      },
      {
        id: "compat-worker",
        name: "Compatibility Worker 兼容性隔离工作进程",
        kind: "managed_runtime",
        version: "artifact-defined",
        install_root: "runtime/compat-worker",
        status: "ready",
        file_count: 2,
        total_bytes: 15728640,
        verified: true,
        detail: "独立受控进程 (15.00 MB)",
        capabilities: ["bounded-compatibility-operations"],
      },
    ],
  };
}

export interface ManagedPluginBundledSkillEntry {
  id: string;
  name: string;
  description: string;
  path: string;
  prompt: string;
}

export interface ManagedPluginBundledConnectorEntry {
  id: string;
  name: string;
  transport: string;
  endpoint: string;
  description: string;
}

export interface ManagedPluginPackage {
  id: string;
  name: string;
  version: string;
  author: string;
  license: string;
  discipline: string;
  description: string;
  tags: string[];
  enabled: boolean;
  installed: boolean;
  is_official: boolean;
  install_path: string;
  sha256: string;
  total_bytes: number;
  file_count: number;
  files: string[];
  bundled_skills: ManagedPluginBundledSkillEntry[];
  bundled_connector?: ManagedPluginBundledConnectorEntry | null;
  bundled_connectors?: ManagedPluginBundledConnectorEntry[];
}

export type PluginPackageEntry = ManagedPluginPackage;

export interface CustomConnectorBackendRecord {
  id: string;
  name: string;
  transport: "stdio" | "http" | string;
  endpoint: string;
  description: string;
  enabled: boolean;
}

export interface CustomSkillBackendRecord {
  id: string;
  name: string;
  description: string;
  prompt: string;
  author: string;
  license: string;
  version: string;
  slash_command?: string;
  updated_at?: string;
  enabled?: boolean;
}

export interface ApplicationPluginsStatusResponse {
  ok: boolean;
  app_data_dir: string;
  plugins_dir: string;
  runtime_plugins_dir: string;
  installed_plugins: ManagedPluginPackage[];
  available_plugins: ManagedPluginPackage[];
  custom_connectors: CustomConnectorBackendRecord[];
  custom_skills: CustomSkillBackendRecord[];
}

export type PluginsStatusResponse = ApplicationPluginsStatusResponse;

export interface PluginInstallRequest {
  plugin_id?: string | null;
  source_path?: string | null;
  raw_content?: string | null;
  fallback_name?: string | null;
  raw_name?: string | null;
}

export interface PluginMutationResponse {
  ok: boolean;
  message: string;
  installed_plugin?: ManagedPluginPackage | null;
  status: ApplicationPluginsStatusResponse;
}

export interface ModelUsageBreakdownRow {
  route: string;
  channel_id: string;
  model_id: string;
  run_count: number;
  model_calls: number;
  tool_calls: number;
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  total_tokens: number;
}

export interface ApplicationUsageStatisticsResponse {
  ok: boolean;
  collected_at: string;
  data_root: string;
  sqlite_size_bytes: number;
  disk_free_bytes?: number | null;
  workspace_count: number;
  session_count: number;
  run_count: number;
  completed_run_count: number;
  tool_call_count: number;
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
  cached_tokens: number;
  total_tokens: number;
  model_breakdown: ModelUsageBreakdownRow[];
  download_total_tasks: number;
  download_completed_tasks: number;
  download_active_tasks: number;
  download_failed_tasks: number;
  download_transferred_bytes: number;
  download_selected_items: number;
  installed_plugins_count: number;
  custom_skills_count: number;
}

export async function getPluginsStatus(): Promise<ApplicationPluginsStatusResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).getPluginsStatus === "function") {
      return await (bridge.application as any).getPluginsStatus();
    }
  }
  return null;
}

export async function installPluginPackage(
  request: PluginInstallRequest,
): Promise<PluginMutationResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).installPluginPackage === "function") {
      return await (bridge.application as any).installPluginPackage(request);
    }
  }
  return null;
}

export async function setPluginEnabledBackend(
  pluginId: string,
  enabled: boolean,
): Promise<PluginMutationResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).setPluginEnabled === "function") {
      return await (bridge.application as any).setPluginEnabled(pluginId, enabled);
    }
  }
  return null;
}

export async function uninstallPluginPackageBackend(
  pluginId: string,
): Promise<PluginMutationResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).uninstallPluginPackage === "function") {
      return await (bridge.application as any).uninstallPluginPackage(pluginId);
    }
  }
  return null;
}

export async function saveCustomConnectorBackend(
  connector: CustomConnectorBackendRecord,
): Promise<ApplicationPluginsStatusResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).saveCustomConnector === "function") {
      return await (bridge.application as any).saveCustomConnector(connector);
    }
  }
  return null;
}

export async function deleteCustomConnectorBackend(
  connectorId: string,
): Promise<ApplicationPluginsStatusResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).deleteCustomConnector === "function") {
      return await (bridge.application as any).deleteCustomConnector(connectorId);
    }
  }
  return null;
}

export async function saveCustomSkillBackend(
  skill: CustomSkillBackendRecord,
): Promise<ApplicationPluginsStatusResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).saveCustomSkill === "function") {
      return await (bridge.application as any).saveCustomSkill(skill);
    }
  }
  return null;
}

export async function deleteCustomSkillBackend(
  skillId: string,
): Promise<ApplicationPluginsStatusResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).deleteCustomSkill === "function") {
      return await (bridge.application as any).deleteCustomSkill(skillId);
    }
  }
  return null;
}

export async function installCustomSkillFromUrlBackend(
  url: string,
  fallbackName?: string | null,
): Promise<ApplicationPluginsStatusResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (
      bridge?.application &&
      typeof (bridge.application as any).installCustomSkillFromUrl === "function"
    ) {
      return await (bridge.application as any).installCustomSkillFromUrl(url, fallbackName ?? null);
    }
  }
  return null;
}

export interface McpToolProbeInfo {
  name: string;
  qualified_name: string;
  description: string;
  input_schema: Record<string, unknown>;
}

export interface McpConnectorProbeResult {
  ok: boolean;
  connector_id: string;
  transport: string;
  server_name?: string | null;
  server_version?: string | null;
  tools: McpToolProbeInfo[];
  error?: string | null;
}

export async function probeCustomConnectorToolsBackend(
  connectorId: string,
): Promise<McpConnectorProbeResult | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (
      bridge?.application &&
      typeof (bridge.application as any).probeCustomConnectorTools === "function"
    ) {
      return await (bridge.application as any).probeCustomConnectorTools(connectorId);
    }
  }
  return null;
}

export async function installCustomSkillFromFolderBackend(
  folderPath: string,
): Promise<ApplicationPluginsStatusResponse | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (
      bridge?.application &&
      typeof (bridge.application as any).installCustomSkillFromFolder === "function"
    ) {
      return await (bridge.application as any).installCustomSkillFromFolder(folderPath);
    }
  }
  return null;
}

export interface CondaGeoInstallResult {
  ok: boolean;
  package_id: string;
  env_name: string;
  command_preview: string;
  stdout: string;
  stderr: string;
  detail: string;
  report?: any;
}

export async function installCondaGeoPackageBackend(request: {
  package_id: string;
  conda_env?: string | null;
  confirmed: boolean;
}): Promise<CondaGeoInstallResult | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (
      bridge?.application &&
      typeof (bridge.application as any).condaInstallGeoPackage === "function"
    ) {
      return await (bridge.application as any).condaInstallGeoPackage(request);
    }
  }
  return null;
}

export async function loadAgentDisabledItemsBackend(): Promise<{
  disabled_providers: string[];
  disabled_models: string[];
  disabled_skills: string[];
} | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.agent && typeof bridge.agent.disabledItemsGet === "function") {
      return await bridge.agent.disabledItemsGet();
    }
  }
  return null;
}

export async function saveAgentDisabledItemsBackend(items: {
  disabled_providers: string[];
  disabled_models: string[];
  disabled_skills: string[];
}): Promise<boolean> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.agent && typeof bridge.agent.disabledItemsSet === "function") {
      await bridge.agent.disabledItemsSet(items);
      return true;
    }
  }
  return false;
}

export async function getApplicationUsageStatistics(): Promise<ApplicationUsageStatisticsResponse> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).getUsageStatistics === "function") {
      return await (bridge.application as any).getUsageStatistics();
    }
  }
  return {
    ok: true,
    collected_at: new Date().toISOString(),
    data_root: "D:\\EOStage",
    sqlite_size_bytes: 0,
    workspace_count: 1,
    session_count: 0,
    run_count: 0,
    completed_run_count: 0,
    tool_call_count: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    reasoning_tokens: 0,
    cached_tokens: 0,
    total_tokens: 0,
    model_breakdown: [],
    download_total_tasks: 0,
    download_completed_tasks: 0,
    download_active_tasks: 0,
    download_failed_tasks: 0,
    download_transferred_bytes: 0,
    download_selected_items: 0,
    installed_plugins_count: 0,
    custom_skills_count: 0,
  };
}

export interface GisRasterProcessRequest {
  sources: string[];
  output_path: string;
  operation:
    | "clip"
    | "translate"
    | "reproject"
    | "mosaic"
    | "slope"
    | "aspect"
    | "hillshade"
    | "tpi"
    | "tri"
    | "roughness"
    | "ndvi"
    | "ndwi"
    | "nbr"
    | "band_math";
  crop_bbox?: [number, number, number, number] | null;
  band_a?: number;
  band_b?: number;
  z_factor?: number | null;
  azimuth_deg?: number | null;
  altitude_deg?: number | null;
  target_crs?: string | null;
  resampling?: string | null;
  output_format?: string | null;
  preserve_dtype?: boolean | null;
}

export interface GisVectorProcessRequest {
  input_path: string;
  output_path: string;
  operation: "buffer" | "convert" | string;
  distance_m?: number | null;
  quad_segs?: number | null;
}

export interface SharedRtkInfo {
  installed: boolean;
  source: string;
  executable_path?: string | null;
  version?: string | null;
  detail: string;
}

export interface LogDehydratorInfo {
  builtin_ready: boolean;
  mode: "auto" | "builtin_only" | string;
  shared_rtk: SharedRtkInfo;
  capabilities: string[];
  detail: string;
}

export interface DehydrationSummary {
  engine: string;
  raw_bytes: number;
  kept_bytes: number;
  raw_lines: number;
  kept_lines: number;
  collapsed_progress_lines: number;
  collapsed_duplicate_warnings: number;
  head_tail_elided_lines: number;
}

export interface ExternalCondaEnvInfo {
  id: string;
  name: string;
  source: string;
  env_prefix: string;
  python_path: string;
  python_version?: string | null;
  geo_packages: string[];
  verified: boolean;
  detail: string;
}

export interface ExternalGisToolchainInfo {
  id: "rustgis" | "geolibre" | "qgis" | "arcgis_pro" | string;
  display_name: string;
  installed: boolean;
  source: string;
  executable_path?: string | null;
  gui_path?: string | null;
  version?: string | null;
  mcp_capable: boolean;
  headless_capable: boolean;
  gui_capable: boolean;
  capabilities: string[];
  detail: string;
}

export interface ExternalGisPrefs {
  custom_paths: Record<string, string>;
  preferred_toolchain?: string | null;
  conda_custom_envs?: Record<string, string>;
  preferred_conda_env?: string | null;
  rtk_custom_path?: string | null;
  dehydrator_mode?: "auto" | "builtin_only" | string | null;
  extension_mirror_preset?: string | null;
  extension_registry_url?: string | null;
}

export interface ExternalGisInspectReport {
  ok: boolean;
  platform: string;
  prefs: ExternalGisPrefs;
  toolchains: ExternalGisToolchainInfo[];
  conda_environments?: ExternalCondaEnvInfo[];
  log_dehydrator?: LogDehydratorInfo;
}

export interface ExternalGisExecuteRequest {
  toolchain: "rustgis" | "geolibre" | "qgis" | "arcgis_pro" | "conda" | string;
  action: "version" | "list_algorithms" | "run_algorithm" | "deliver_project" | string;
  algorithm?: string | null;
  conda_env?: string | null;
  params?: Record<string, unknown>;
  input_paths?: string[];
  output_path?: string | null;
  open_gui?: boolean;
  timeout_seconds?: number | null;
}

export interface ExternalGisExecuteResult {
  ok: boolean;
  toolchain: string;
  action: string;
  exit_code?: number | null;
  stdout: string;
  stderr: string;
  output_path?: string | null;
  project_path?: string | null;
  catalog_path?: string | null;
  gui_launched: boolean;
  detail: string;
  artifacts?: string[];
  dehydration?: DehydrationSummary | null;
}

export interface LbsCoordTransformRequest {
  from: "wgs84" | "gcj02" | "bd09" | "cgcs2000" | string;
  to: "wgs84" | "gcj02" | "bd09" | "cgcs2000" | string;
  points: Array<[number, number]>;
}

export interface MapGisConvertRequest {
  input_path: string;
  output_path: string;
  scale?: number | null;
}

export interface OpenGeodataQueryRequest {
  dataset:
    | "open_dem_tiles"
    | "mca_xzqh"
    | "mca_placename"
    | "datav_bound"
    | "open_meteo_era5"
    | "overpass_osm"
    | "worldpop"
    | "whitebox_run"
    | "duckdb_sql"
    | string;
  code?: string | null;
  keyword?: string | null;
  bbox?: [number, number, number, number] | null;
  aoi_geojson?: Json | null;
  lat?: number | null;
  lng?: number | null;
  date?: string | null;
  start_date?: string | null;
  end_date?: string | null;
  osm_tag?: string | null;
  dem_product?: "cop30" | "cop90" | "nasadem" | "aw3d30" | string | null;
  iso3?: string | null;
  year?: number | null;
  output_path?: string | null;
  full_children?: boolean | null;
  tool_name?: string | null;
  tool_args?: string[] | null;
  sql?: string | null;
  gee_params?: Json | null;
  zooms?: number[] | null;
}

export async function inspectGisRasterSource(source: string): Promise<Json | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).gisRasterInspect === "function") {
      return await (bridge.application as any).gisRasterInspect({ source });
    }
  }
  return null;
}

export interface RasterPixelProbeBandValue {
  band: number;
  name: string;
  dtype: string;
  raw_value: number | null;
  scale: number;
  offset: number;
  physical_value: number | null;
  nodata: number | null;
  is_nodata: boolean;
  qa_semantics: string | null;
}

export interface RasterPixelProbeResult {
  ok: boolean;
  source: string;
  lon: number;
  lat: number;
  projected_coord: [number, number];
  crs: string | null;
  width: number;
  height: number;
  in_bounds: boolean;
  col: number;
  row: number;
  bands: RasterPixelProbeBandValue[];
}

export async function probeGisRasterPixel(
  source: string,
  lon: number,
  lat: number,
): Promise<RasterPixelProbeResult | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof bridge.application.gisRasterPixelProbe === "function") {
      return (await bridge.application.gisRasterPixelProbe({ source, lon, lat })) as RasterPixelProbeResult;
    }
  }
  return null;
}

export async function processGisRasterWindow(request: GisRasterProcessRequest): Promise<Json | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).gisRasterProcess === "function") {
      return await (bridge.application as any).gisRasterProcess(request);
    }
  }
  return null;
}

export async function processGisVector(request: GisVectorProcessRequest): Promise<Json | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.gisVectorProcess === "function") {
    return await bridge.application.gisVectorProcess(request);
  }
  return null;
}

export async function inspectExternalGis(
  probeVersions = false,
): Promise<ExternalGisInspectReport | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.externalGisInspect === "function") {
    return await bridge.application.externalGisInspect({ probe_versions: probeVersions });
  }
  return null;
}

export async function saveExternalGisPrefs(
  prefs: ExternalGisPrefs,
): Promise<ExternalGisInspectReport | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.externalGisSavePrefs === "function") {
    return await bridge.application.externalGisSavePrefs(prefs);
  }
  return null;
}

export async function executeExternalGis(
  request: ExternalGisExecuteRequest,
): Promise<ExternalGisExecuteResult | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.externalGisExecute === "function") {
    return await bridge.application.externalGisExecute(request);
  }
  return null;
}

export async function transformLbsCoordinates(request: LbsCoordTransformRequest): Promise<Json | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).lbsCoordTransform === "function") {
      return await (bridge.application as any).lbsCoordTransform(request);
    }
  }
  return null;
}

export async function inspectMapGisFile(path: string): Promise<Json | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).mapgisInspect === "function") {
      return await (bridge.application as any).mapgisInspect({ path });
    }
  }
  return null;
}

export async function convertMapGisFile(request: MapGisConvertRequest): Promise<Json | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).mapgisConvert === "function") {
      return await (bridge.application as any).mapgisConvert(request);
    }
  }
  return null;
}

export async function queryOpenGeodata(request: OpenGeodataQueryRequest): Promise<Json | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof (bridge.application as any).openGeodataQuery === "function") {
      return await (bridge.application as any).openGeodataQuery(request);
    }
  }
  return null;
}

export async function fetchGeodataPreviewDataUrl(
  url: string,
): Promise<{ ok: boolean; data_url?: string; error?: string }> {
  const bridge = getDesktopShellBridge();
  if (
    bridge?.application &&
    typeof bridge.application.fetchGeodataPreviewDataUrl === "function"
  ) {
    return await bridge.application.fetchGeodataPreviewDataUrl(url);
  }
  return { ok: false, error: "desktop_only" };
}

export interface ExperimentProjectItemView {
  item_id: string;
  task_id: string;
  provider_id: string;
  dataset_id: string;
  task_type: string | null;
  output_root: string;
  primary_artifact_path: string;
  provenance_path: string | null;
  size_bytes: number;
  sha256: string;
  integrity_status: "verified" | "missing_or_modified" | string;
  license_id: string;
  is_noncommercial: boolean;
  bbox: [number, number, number, number] | null;
  added_at: string;
}

export interface ExperimentProjectSnapshot {
  project_id: string;
  title: string;
  max_items: number;
  item_count: number;
  has_noncommercial_restricted_data: boolean;
  aoi_display_name: string | null;
  aoi_bbox: [number, number, number, number] | null;
  items: ExperimentProjectItemView[];
  updated_at: string;
}

export interface ExperimentProjectExportResult {
  project_id: string;
  export_dir: string;
  manifest_path: string;
  aoi_geojson_path: string;
  footprints_geojson_path: string;
  readme_path: string;
  verified_count: number;
  missing_or_modified_count: number;
  has_noncommercial_restricted_data: boolean;
}

export async function getExperimentProject(): Promise<ExperimentProjectSnapshot | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof bridge.application.experimentProjectGet === "function") {
      return (await bridge.application.experimentProjectGet()) as ExperimentProjectSnapshot;
    }
  }
  return null;
}

export async function addTaskToExperimentProject(taskId: string): Promise<ExperimentProjectSnapshot | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof bridge.application.experimentProjectAddTask === "function") {
      return (await bridge.application.experimentProjectAddTask(taskId)) as ExperimentProjectSnapshot;
    }
  }
  return null;
}

export async function removeExperimentProjectItem(itemId: string): Promise<ExperimentProjectSnapshot | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof bridge.application.experimentProjectRemoveItem === "function") {
      return (await bridge.application.experimentProjectRemoveItem(itemId)) as ExperimentProjectSnapshot;
    }
  }
  return null;
}

export async function exportExperimentProjectBundle(
  exportDir?: string | null,
): Promise<ExperimentProjectExportResult | null> {
  if (isTauriDesktopHost()) {
    const bridge = getDesktopShellBridge();
    if (bridge?.application && typeof bridge.application.experimentProjectExport === "function") {
      return (await bridge.application.experimentProjectExport(exportDir)) as ExperimentProjectExportResult;
    }
  }
  return null;
}

export async function getDesktopOnboardingState(): Promise<{
  completed: boolean;
  returning_user?: boolean;
} | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.desktopOnboardingStateGet === "function") {
    return await bridge.application.desktopOnboardingStateGet();
  }
  return null;
}

export async function setDesktopOnboardingCompleted(
  completed: boolean,
): Promise<{ completed: boolean } | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.desktopOnboardingStateSet === "function") {
    return await bridge.application.desktopOnboardingStateSet(completed);
  }
  return null;
}

const PREFERRED_RESOURCE_STORAGE_KEY = "sp_preferred_active_resource_id";

export async function getPreferredActiveResourceId(): Promise<string | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.desktopActiveResourceGet === "function") {
    return await bridge.application.desktopActiveResourceGet();
  }
  if (!isTauriDesktopHost() && typeof window !== "undefined") {
    try {
      const saved = window.localStorage.getItem(PREFERRED_RESOURCE_STORAGE_KEY);
      return saved && saved.trim() ? saved.trim() : null;
    } catch {
      return null;
    }
  }
  return null;
}

export async function setPreferredActiveResourceId(
  resourceId: string,
): Promise<string | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.desktopActiveResourceSet === "function") {
    return await bridge.application.desktopActiveResourceSet(resourceId);
  }
  if (!isTauriDesktopHost() && typeof window !== "undefined") {
    try {
      const trimmed = resourceId.trim();
      if (trimmed) {
        window.localStorage.setItem(PREFERRED_RESOURCE_STORAGE_KEY, trimmed);
        return trimmed;
      }
      window.localStorage.removeItem(PREFERRED_RESOURCE_STORAGE_KEY);
    } catch {
      return null;
    }
  }
  return null;
}

export async function getCacheSummary(): Promise<{
  ok?: boolean;
  cache_dir?: string;
} | null> {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.getCacheSummary === "function") {
    return await bridge.application.getCacheSummary();
  }
  return null;
}

export async function getDiagnosticsState() {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.diagnosticsGetState === "function") {
    return await bridge.application.diagnosticsGetState();
  }
  return { enabled: false, total_count: 0, pending_count: 0 };
}

export async function setDiagnosticsEnabled(enabled: boolean) {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.diagnosticsSetEnabled === "function") {
    return await bridge.application.diagnosticsSetEnabled(enabled);
  }
  return { enabled, total_count: 0, pending_count: 0 };
}

export async function recordDiagnosticReport(input: {
  category: string;
  error_code?: string | null;
  http_status?: number | null;
  source?: string | null;
  message: string;
  stack?: string | null;
}) {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.diagnosticsRecord === "function") {
    return await bridge.application.diagnosticsRecord(input);
  }
  return null;
}

export async function listDiagnosticReports() {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.diagnosticsList === "function") {
    return await bridge.application.diagnosticsList();
  }
  return [];
}

export async function clearDiagnosticReports() {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.diagnosticsClear === "function") {
    return await bridge.application.diagnosticsClear();
  }
  return { enabled: false, total_count: 0, pending_count: 0 };
}

export async function uploadDiagnosticReports() {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.diagnosticsUpload === "function") {
    return await bridge.application.diagnosticsUpload();
  }
  return {
    attempted: 0,
    uploaded: 0,
    failed: 0,
    message_zh: "当前运行环境不支持上报诊断记录。",
  };
}

export async function checkAppUpdate() {
  const bridge = getDesktopShellBridge();
  if (bridge?.application && typeof bridge.application.checkAppUpdate === "function") {
    return await bridge.application.checkAppUpdate();
  }
  return null;
}

function _noopEndMarker() {
  return {
    custom_skills_count: 0,
  };
}

