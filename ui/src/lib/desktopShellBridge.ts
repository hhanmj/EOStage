import type {
  SourceDescriptor,
  SourceView,
  UnifiedSearchRequest,
  SearchProgress,
  TaskRef,
  AccountSessionStatus,
  StoreBatchPage,
  TaskOverview,
  CreateTaskOptions,
} from "../shell/api/types.ts";
import type { DesktopAgentBridge } from "../shell/api/agentBridgeTypes.ts";
import type {
  AppendTaskFromSourceBatchRequest,
  ApplicationSearchProgress,
  ApplicationSearchProgressStage,
  CatalogSearchQuery,
  CreateTaskFromHistoricalSourceBatchRequest,
  DownloadLayout,
  FrozenOrbitAssociationV1,
  PreviewAssetRequest,
  ResolvedPreviewAsset,
  SarDownloadPolicySettings,
  SearchCancelOutcome,
  StoreTaskSourceBatchPage,
  StoreTaskSourceBatchPageRequest,
  TaskConflictFileDeleteResult,
  TaskConflictFileExpectation,
  TaskConflictFileList,
  TaskRetryFallbackOutcome,
} from "./generated/v3-contracts.ts";

export type DesktopShellKind = "tauri" | "pywebview" | "browser_fixture" | "unavailable";

export type DesktopCompatibilityBridge = {
  capabilities(): Promise<DesktopJsonObject>;
  /** Explicit legacy/test adapter only; normal Tauri production routes reject it. */
  callLegacy(method: string, args: unknown[]): Promise<unknown>;
};

export type DesktopAppInfo = {
  name: string;
  version: string;
  offline: boolean;
};

export type DesktopCommandResult =
  | { ok: true; [key: string]: unknown }
  | { ok: false; error: string; code: string | null };

export type DesktopPickResult = {
  ok: boolean;
  path: string;
  error?: string;
  code?: string | null;
};

export type DesktopWindowSize =
  | { ok: true; width: number; height: number; x?: number; y?: number }
  | { ok: false; error: string; code: string | null };

export type DesktopWindowWorkArea = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type DesktopSarSearchRangeDiagnostic = {
  start: string | null;
  end: string | null;
};

export type DesktopSarSearchFailureDiagnostic = {
  operation: "sar_search_and_freeze";
  route: "legacy_asf_search";
  sanitized_reason: string;
  dataset: string;
  query_shape: {
    platform_count: number;
    processing_level_count: number;
    product_family_count: number;
    beam_mode_count: number;
    polarization_count: number;
    has_instrument: boolean;
    has_relative_orbit: boolean;
    has_frame: boolean;
    has_asf_frame: boolean;
    has_path: boolean;
    has_track_range: boolean;
    has_frame_range: boolean;
    has_aoi_geojson: boolean;
    has_aoi_wkt: boolean;
    has_bbox: boolean;
    has_burst: boolean;
  };
  aoi_state: "none" | "geojson" | "bbox" | "wkt";
  date_range: DesktopSarSearchRangeDiagnostic;
  filter_state: {
    platforms: string[];
    instrument: string | null;
    processing_levels: string[];
    product_families: string[];
    beam_modes: string[];
    polarizations: string[];
    flight_direction: string | null;
    look_direction: string | null;
    relative_orbit: number | null;
    frame: number | null;
    asf_frame: string | null;
    path: number | null;
    track_range: DesktopSarSearchRangeDiagnostic | null;
    frame_range: DesktopSarSearchRangeDiagnostic | null;
  };
  max_results: number | null;
};

export type DesktopApplicationError = {
  ok: false;
  error: string;
  code: string | null;
  context?: string;
  diagnostic?: DesktopSarSearchFailureDiagnostic;
};

export type DesktopCredentialStatus = {
  cdse: string;
  usgs: string;
  earthdata: string;
  earthdata_input: {
    mode: "none" | "token" | "login" | "netrc";
    token: string;
    username: string;
    password: string;
  };
  bhoonidhi: string;
  opentopography: string;
  gacos: string;
};

export type DesktopCredentialStatusResult =
  | { ok: true; status: DesktopCredentialStatus }
  | DesktopApplicationError;

export type DesktopCredentialMutationResult =
  | { ok: true; status: DesktopCredentialStatus; removed: boolean }
  | DesktopApplicationError;

export type DesktopEarthdataAuthResult =
  | {
      ok: true;
      configured: boolean;
      status: "missing" | "valid" | "expired" | "invalid" | "unknown" | "unavailable";
      message: string;
    }
  | DesktopApplicationError;

export type DesktopCredentialField = "cdse_username" | "cdse_password"
  | "usgs_username" | "usgs_application_token"
  | "bhoonidhi_username" | "bhoonidhi_password"
  | "earthdata_username" | "earthdata_password" | "earthdata_token" | "opentopography_key" | "gacos_email";
export type DesktopCredentialRevealResult = { ok: true; value: string | null } | DesktopApplicationError;
export type DesktopBhoonidhiAuthResult = { ok: true; session_available: true; catalogue_verified: false } | DesktopApplicationError;

export type DesktopBhoonidhiSessionStatus = {
  authenticated: boolean;
  refresh_available: boolean;
  expires_in_seconds: number | null;
};

export type DesktopBhoonidhiSessionStatusResult =
  | { ok: true; status: DesktopBhoonidhiSessionStatus }
  | DesktopApplicationError;

export type DesktopCredentialBridge = {
  readForEdit(field: DesktopCredentialField): Promise<DesktopCredentialRevealResult>;
  readAgentChannelKey?(channelId: string): Promise<DesktopCredentialRevealResult>;
  updateField(field: DesktopCredentialField, value: string): Promise<{ ok: true } | DesktopApplicationError>;
  checkBhoonidhiAuth(): Promise<DesktopBhoonidhiAuthResult>;
  getBhoonidhiSessionStatus(): Promise<DesktopBhoonidhiSessionStatusResult>;
  reveal(field: DesktopCredentialField): Promise<DesktopCredentialRevealResult>;
  getStatus(): Promise<DesktopCredentialStatusResult>;
  checkEarthdataAuth(force?: boolean): Promise<DesktopEarthdataAuthResult>;
  saveCdseLogin(username: string, password: string): Promise<DesktopCredentialMutationResult>;
  clearCdse(): Promise<DesktopCredentialMutationResult>;
  saveUsgsLogin(username: string, applicationToken: string): Promise<DesktopCredentialMutationResult>;
  clearUsgs(): Promise<DesktopCredentialMutationResult>;
  saveEarthdataToken(token: string): Promise<DesktopCredentialMutationResult>;
  saveEarthdataLogin(username: string, password: string): Promise<DesktopCredentialMutationResult>;
  clearEarthdata(): Promise<DesktopCredentialMutationResult>;
  saveBhoonidhiLogin(username: string, password: string): Promise<DesktopCredentialMutationResult>;
  clearBhoonidhi(): Promise<DesktopCredentialMutationResult>;
  saveOpentopographyKey(apiKey: string): Promise<DesktopCredentialMutationResult>;
  clearOpentopographyKey(): Promise<DesktopCredentialMutationResult>;
  saveGacosEmail(email: string): Promise<DesktopCredentialMutationResult>;
  clearGacosEmail(): Promise<DesktopCredentialMutationResult>;
};

export type DesktopApplicationReadiness =
  | {
      ok: true;
      contract: string;
      status: string;
      store_schema_version: number | null;
    }
  | DesktopApplicationError;

/** Full snapshot shape retained only for explicit legacy projection tests. */
export type DesktopLegacyApplicationSnapshot =
  | {
      ok: true;
      readiness: {
        contract: string;
        status: string;
        store_schema_version: number | null;
      };
      restore: Record<string, unknown>;
    }
  | DesktopApplicationError;

export type DesktopJsonObject = Record<string, unknown>;

export type DesktopNativeJsonResult = DesktopJsonObject | DesktopApplicationError;

export type DesktopAoiImportRequest = {
  source: "path" | "text" | "bytes" | "bundle";
  path?: string;
  file_name?: string;
  text?: string;
  base64_data?: string;
  files?: Array<{ name: string; base64: string }>;
};

export type DesktopAoiNormalizeRequest = {
  geojson: DesktopJsonObject;
  feature_ids?: string[];
  name_field?: string;
  download_mode?: "merge" | "split" | string;
};

export type DesktopAdminBoundarySearchRequest = {
  query?: string;
  province?: string;
  city?: string;
  district?: string;
  provider_object_id?: string | null;
  limit?: number;
};

export type DesktopAdminBoundaryOptionsRequest = {
  province?: string;
  city?: string;
};

export type DesktopCoreRestoreSnapshot =
  | {
      ok: true;
      readiness: {
        contract: string;
        status: string;
        store_schema_version: number | null;
      };
      /** Candidate-free Store-owned CoreRestore projection. */
      core: DesktopJsonObject;
      /** Optional native-derived, read-only AOI display authority. */
      aoi_display?: DesktopJsonObject | null;
    }
  | DesktopApplicationError;

export type DesktopWorkbenchPageRequest = {
  batch_id: string;
  expected_revision: number;
  page_index: number;
  page_size: number;
};

export type DesktopWorkbenchPageSnapshot =
  | {
      ok: true;
      readiness: {
        contract: string;
        status: string;
        store_schema_version: number | null;
      };
      /** One bounded Store-owned page; never a full candidate collection. */
      page: DesktopJsonObject;
      /** Optional native render-only copies for this page/AOI revision. */
      render?: DesktopJsonObject | null;
  }
  | DesktopApplicationError;

export type DesktopTaskPageRequest = {
  task_id: string;
  expected_revision: number;
  page_index: number;
  page_size: number;
};

export type DesktopTaskPageSnapshot =
  | {
      ok: true;
      /** One bounded task item page; no complete task candidate array. */
      page: DesktopJsonObject;
    }
  | DesktopApplicationError;

export type DesktopTaskSourceBatchPageRequest = StoreTaskSourceBatchPageRequest;

export type DesktopTaskSourceBatchPageSnapshot =
  | { ok: true; page: StoreTaskSourceBatchPage }
  | DesktopApplicationError;

export type DesktopAppendTaskFromSourceBatchRequest = AppendTaskFromSourceBatchRequest;

export type DesktopCreateTaskFromHistoricalSourceBatchRequest =
  CreateTaskFromHistoricalSourceBatchRequest;

export type DesktopBurstAssetPolicy = "all" | "primary" | "metadata";

export type DesktopBurstCreateAssetTaskRequest = {
  request_id: string;
  source_batch_id: string;
  selected_product_ids: string[];
  asset_policy: DesktopBurstAssetPolicy;
  expected_workbench_revision: number;
  task_id: string;
  output_root: string;
  layout: DownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopBurstAppendAssetTaskRequest = {
  request_id: string;
  task_id: string;
  selected_product_ids: string[];
  expected_task_revision: number;
  event_id: string;
  occurred_at: string;
};

export type DesktopBurstCreateHistoricalAssetTaskRequest = {
  request_id: string;
  source_task_id: string;
  expected_source_task_revision: number;
  selected_product_ids: string[];
  new_task_id: string;
  output_root: string;
  layout: DownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopTaskEventPageRequest = {
  task_id: string;
  expected_revision: number;
  before_sequence?: number | null;
  page_size: number;
};

export type DesktopTaskEventPageSnapshot =
  | {
      ok: true;
      /** Bounded safe event summaries; raw Store payload/error JSON is excluded. */
      page: DesktopJsonObject;
    }
  | DesktopApplicationError;

export type DesktopOpenTaskOutputRequest = {
  task_id: string;
  expected_revision: number;
};

export type DesktopOpenTaskOutputResult =
  | { ok: true; task_id: string; revision: number }
  | DesktopApplicationError;

export type DesktopListTaskConflictFilesRequest = {
  task_id: string;
};

export type DesktopDeleteTaskConflictFilesRequest = {
  task_id: string;
  expected: TaskConflictFileExpectation[];
};

export type DesktopTaskConflictFileListResult =
  | { ok: true; list: TaskConflictFileList }
  | DesktopApplicationError;

export type DesktopTaskConflictFileDeleteResult =
  | { ok: true; result: TaskConflictFileDeleteResult }
  | DesktopApplicationError;

export type DesktopTaskHistoryPageRequest = {
  page_size: number;
  after_archived_at_epoch_ms?: number | null;
  after_task_id?: string | null;
};

export type DesktopAdminBoundaryResolveRequest = {
  "provider": string;
  provider_object_id: string;
};

export type DesktopTaskHistoryPageSnapshot =
  | {
      ok: true;
      /** Fixed summary rows only; task items stay behind taskPage. */
      page: DesktopJsonObject;
    }
  | DesktopApplicationError;

export type DesktopWorkbenchSelectionCommand = {
  batch_id: string;
  expected_revision: number;
  kind: "check_items" | "uncheck_items" | "select_all" | "clear_all" | "set_page";
  item_ids: string[];
  page_index: number;
  page_size: number;
  updated_at: string;
};

export type DesktopWorkbenchSelectionSummary = {
  batch_id: string;
  revision: number;
  checked_count: number;
  page_index: number;
  page_size: number;
};

export type DesktopWorkbenchSelectionResult =
  | { ok: true; summary: DesktopWorkbenchSelectionSummary }
  | DesktopApplicationError;

export type DesktopWorkspaceOpenRequest = {
  request_id: string;
  workspace_id: string;
  workspace_root: string;
  workspace_name: string;
  expected_revision?: number | null;
};

export type DesktopWorkspaceCreateRequest = DesktopWorkspaceOpenRequest;

export type DesktopWorkspacePrepareLocationRequest = {
  request_id: string;
  path: string;
};

export type DesktopWorkspaceNamedNodeRequest = {
  request_id: string;
  parent_id: string;
  expected_revision: number;
  name: string;
};

export type DesktopWorkspaceSelectNodeRequest = {
  request_id: string;
  parent_id: string;
  expected_revision: number;
  node_id: string;
};

export type DesktopWorkspaceContextResult =
  | { ok: true; context: DesktopJsonObject }
  | DesktopApplicationError;

export type DesktopWorkspacePrepareLocationResult =
  | { ok: true; path: string }
  | DesktopApplicationError;

export type DesktopDemTaskOperation = "inspect" | "prepare_sarscape" | "convert";

export type DesktopDemTaskPlanRequest = {
  request_id: string;
  task_id: string;
  region_id: string;
  resource_id: string;
  capability_id: string;
  dataset: string;
  "provider": string;
  operation: DesktopDemTaskOperation;
  input_path: string;
  output_root: string;
  source_vertical_datum?: string | null;
  target_vertical_datum?: string | null;
  output_mode?: "ellipsoid" | "sarscape" | string | null;
  created_at: string;
};

export type DesktopDemTaskPlan = DesktopDemTaskPlanRequest;

export type DesktopDemTaskPlanResult =
  | { ok: true; plan: DesktopDemTaskPlan }
  | DesktopApplicationError;

export type DesktopDemTaskRequest = {
  plan: DesktopDemTaskPlan;
  concurrency: number;
};

export type DesktopDemTaskSettings = {
  max_concurrent_tasks: number;
  max_concurrent_conversions: number;
};

export type DesktopStandaloneDemRequest = {
  request_id: string;
  dataset: string;
  "provider": string;
  operation: DesktopDemTaskOperation;
  input_path: string;
  output_root: string;
  bbox?: number[] | null;
  source_vertical_datum?: string | null;
  target_vertical_datum?: string | null;
  output_mode?: string | null;
};

export type DesktopStandaloneDemTaskRequest = {
  request_id: string;
  task_id: string;
  dataset: string;
  "provider": string;
  aoi_geojson: DesktopJsonObject;
  aoi_name: string;
  output_root: string;
  convert: boolean;
  concurrency: number;
  created_at: string;
};

export type DesktopSceneImportRequest = {
  source: "text" | "file" | "directory";
  text?: string;
  path?: string;
};

export type DesktopOrbitSceneInput = {
  scene_id: string;
  platform: "S1A" | "S1B" | "S1C" | "S1D" | "UNKNOWN";
  acquisition_unix_seconds?: number | null;
};

export type DesktopOrbitDirectoryMatchRequest = {
  orbit_dir: string;
  scenes: DesktopOrbitSceneInput[];
};

export type DesktopGacosResultImportRequest = {
  ztd_path: string;
  rsc_path: string;
  output_dir?: string | null;
};

export type DesktopGacosCreateImportTaskRequest = {
  task_id: string;
  source_request_task_id: string;
  source_request_item_id: string;
  source_paths: string[];
};

export type DesktopGacosCreateResultTaskRequest = {
  task_id: string;
  source_request_task_id: string;
  source_request_item_id: string;
  result_url: string;
};
export type DesktopGacosSupplyResultLinkRequest = { task_id: string; result_url: string };
export type DesktopGacosSupplyResultLinkResult =
  | { ok: true; task_id: string; source: string }
  | DesktopApplicationError;

export type DesktopGacosCreateImportTaskResult =
  | {
      ok: true;
      task_id: string;
      output_root: string;
      status: string;
      source: string;
    }
  | DesktopApplicationError;

export type DesktopGacosConfirmRequest = {
  prepared_batch_id: string;
  plan_fingerprint_sha256: string;
  task_id: string;
  confirm: boolean;
  force_reapply: boolean;
};

export type DesktopGacosSubmissionState =
  | "reserved"
  | "submitting"
  | "submitted"
  | "manual_verification_required"
  | "submission_uncertain"
  | "rejected";

export type DesktopGacosTaskStatus =
  | "queued"
  | "running"
  | "paused"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted";

export type DesktopGacosRequestGroup = {
  item_id: string;
  dates: string[];
  hour: number;
  minute: number;
  bbox: [number, number, number, number];
};

export type DesktopGacosSubmissionRecord = {
  attempt_id: string;
  task_id: string;
  item_id: string;
  attempt_number: number;
  state: DesktopGacosSubmissionState;
  reason_code: string | null;
  http_status: number | null;
  automatic_retry_allowed: false;
  created_at: string;
  updated_at: string;
};

export type DesktopGacosSubmissionResult =
  | {
      ok: true;
      task_id: string;
      records: DesktopGacosSubmissionRecord[];
      request_groups: DesktopGacosRequestGroup[];
      task_status: DesktopGacosTaskStatus;
      submitted_count: number;
      application_completed: boolean;
      manual_action_required: boolean;
      automatic_retry_allowed: false;
      message: string;
    }
  | DesktopApplicationError;

export type DesktopNetworkSettings = {
  proxy_enabled: boolean;
  proxy_url: string;
  cache_enabled: boolean;
  cache_dir: string;
  cache_limit_mb: number;
  tianditu_token: string;
  asf_ssl_verify: boolean;
};

export type DesktopProviderConnectivityResult = {
  id: string;
  site: string;
  ok: boolean;
  latency_ms: number;
  http_status: number | null;
  error_code: string;
};

export type DesktopDemConversionRequest = {
  request_id: string;
  source_task_id: string;
  conversion_task_id: string;
  region_id: string;
  resource_id: string;
  capability_id: string;
  dataset: string;
  "provider": string;
  input_path: string;
  output_root: string;
  source_vertical_datum?: string | null;
  target_vertical_datum?: string | null;
  output_mode?: "ellipsoid" | "sarscape" | string | null;
  created_at: string;
  concurrency: number;
};

export type DesktopOrbitTaskRequest = {
  request_id: string;
  task_id: string;
  region_id: string;
  association: FrozenOrbitAssociationV1;
  output_root: string;
  created_at: string;
  concurrency: number;
};

export type DesktopStandaloneOrbitSceneInput = {
  scene_id: string;
  acquisition_datetime?: string | null;
};

export type DesktopStandaloneOrbitTaskRequest = {
  request_id: string;
  task_id: string;
  scenes: DesktopStandaloneOrbitSceneInput[];
  output_root: string;
  use_orbit_subdir: boolean;
  created_at: string;
};

export type DesktopBindRegionAoiRequest = {
  request_id: string;
  region_id: string;
  expected_revision?: number | null;
  source_kind: "manual_bbox" | "manual_polygon" | "uploaded_geometry" | "administrative_boundary";
  geometry: Record<string, unknown>;
  bbox: number[];
  crs: "EPSG:4326";
  display_name: string;
  bound_at: string;
};

export type DesktopClearRegionAoiRequest = {
  request_id: string;
  region_id: string;
  expected_revision?: number | null;
};

export type DesktopSarSearchAndFreezeRequest = {
  request_id: string;
  search: { query: DesktopJsonObject };
  batch_id: string;
  created_at: string;
  aoi_name: string;
  aoi_geojson: DesktopJsonObject | null;
  page_size: number;
};

export type DesktopCatalogSearchAndFreezeRequest = {
  request_id: string;
  search: CatalogSearchQuery;
  batch_id: string;
  created_at: string;
  aoi_name: string;
  aoi_geojson: DesktopJsonObject | null;
  workbench_page_size: number;
};

/**
 * Issue #3 R2 B4: a route-neutral catalogue search intent. The caller names the
 * product and the reviewed search filter schema revision it rendered; the
 * application plans the route from the Registry and keeps its fallback plan.
 */
export type CatalogSearchIntent = {
  resource_id: string;
  search_schema_id: string;
  schema_revision: number;
  aoi_geojson: DesktopJsonObject | null;
  result_limit: number | null;
  filters: Record<string, unknown>;
};

export type DesktopCatalogIntentSearchAndFreezeRequest = {
  request_id: string;
  intent: CatalogSearchIntent;
  batch_id: string;
  created_at: string;
  aoi_name: string;
  aoi_geojson: DesktopJsonObject | null;
  workbench_page_size: number;
};

/** Write-only input. The bridge never returns or stores these values. */
export type DesktopCdseLoginRequest = {
  username: string;
  password: string;
  totp?: string;
};

export type DesktopCdseNetworkMode = "system" | "direct";

export type DesktopCdseSessionStatus = {
  network_mode: DesktopCdseNetworkMode;
  authenticated: boolean;
  refresh_available: boolean;
  expires_in_seconds: number | null;
};

export type DesktopCdseNetworkDiagnosticStage =
  | "dns_unavailable"
  | "tcp_unavailable"
  | "tls_handshake_failed"
  | "proxy_configuration_rejected"
  | "timeout"
  | "http_reached"
  | "transport_unavailable";

export type DesktopCdseNetworkDiagnostic = {
  network_mode: DesktopCdseNetworkMode;
  stage: DesktopCdseNetworkDiagnosticStage;
};

export type DesktopCdseSessionStatusResult =
  | { ok: true; status: DesktopCdseSessionStatus }
  | DesktopApplicationError;

export type DesktopCdseNetworkPolicyResult =
  | { ok: true; network_mode: DesktopCdseNetworkMode }
  | DesktopApplicationError;

export type DesktopCdseNetworkDiagnosticResult =
  | { ok: true; diagnostic: DesktopCdseNetworkDiagnostic }
  | DesktopApplicationError;

/** Shared Preview request/response are generated from the canonical v3 contract. */
export type DesktopPreviewAssetRequest = PreviewAssetRequest;
export type DesktopResolvedPreviewAsset = ResolvedPreviewAsset;
export type DesktopPreviewAssetResult =
  | { ok: true; preview: DesktopResolvedPreviewAsset }
  | DesktopApplicationError;

export type DesktopPreviewSaveRequest = {
  data_url: string;
  output_dir: string;
  file_name: string;
};
export type DesktopPreviewSaveResult =
  | { ok: true; path: string; file_name: string; bytes_written: number }
  | DesktopApplicationError;

/** Frozen identifiers only; the UI never supplies a preview URL or credential. */
export type DesktopCdseQuicklookRequest = {
  request_id: string;
  batch_id: string;
  item_id: string;
};

export type DesktopCdseQuicklook = {
  media_type: "image/jpeg" | "image/png" | "image/webp";
  data_url: string;
  byte_length: number;
};

export type DesktopCdseQuicklookResult =
  | { ok: true; quicklook: DesktopCdseQuicklook }
  | DesktopApplicationError;

export type DesktopSentinel2MapPreviewRequest = {
  request_id: string;
  batch_id: string;
  item_id: string;
};

export type DesktopSentinel2MapPreview = {
  product_key: string;
  map_item_id: string;
};

export type DesktopSentinel2MapPreviewResult =
  | { ok: true; preview: DesktopSentinel2MapPreview }
  | DesktopApplicationError;

/** Write-only input; implementations must not return or persist either field. */
export type DesktopUsgsM2mLoginRequest = {
  username: string;
  application_token: string;
};

export type DesktopUsgsM2mSessionState =
  | "ready"
  | "logging_in"
  | "authenticated"
  | "login_failed"
  | "logging_out"
  | "logged_out";

export type DesktopUsgsM2mSessionStatus = {
  state: DesktopUsgsM2mSessionState;
  authenticated: boolean;
};

export type DesktopUsgsM2mSessionStatusResult =
  | { ok: true; status: DesktopUsgsM2mSessionStatus }
  | DesktopApplicationError;

/** Frozen identifiers only; no browse URL or authorization crosses from UI. */
export type DesktopLandsatBrowseRequest = {
  request_id: string;
  batch_id: string;
  item_id: string;
};

export type DesktopLandsatBrowse = DesktopCdseQuicklook;

export type DesktopLandsatBrowseResult =
  | { ok: true; browse: DesktopLandsatBrowse }
  | DesktopApplicationError;

export type DesktopLandsatAssetDetailsRequest = {
  request_id: string;
  batch_id: string;
  item_id: string;
};

export type DesktopLandsatAssetDetailRole = "primary" | "auxiliary" | "metadata";

export type DesktopLandsatAssetDetail = {
  asset_key: string;
  file_name: string;
  role: DesktopLandsatAssetDetailRole;
  size_bytes: number;
};

export type DesktopLandsatAssetDetails = {
  product_key: string;
  display_id: string;
  processing_level: string;
  total_size_bytes: number;
  assets: DesktopLandsatAssetDetail[];
};

export type DesktopLandsatAssetDetailsResult =
  | { ok: true; details: DesktopLandsatAssetDetails }
  | DesktopApplicationError;

export type DesktopBhoonidhiAssetSizeRequest = {
  request_id: string;
  batch_id: string;
  item_id: string;
};

export type DesktopBhoonidhiAssetSize = {
  product_key: string;
  size_bytes: number;
};

export type DesktopBhoonidhiAssetSizeResult =
  | { ok: true; size: DesktopBhoonidhiAssetSize }
  | DesktopApplicationError;

/** Frozen scene identity plus task settings; no URL or credential crosses from UI. */
export type DesktopCreateLandsatBundleTaskRequest = {
  request_id: string;
  source_batch_id: string;
  item_id: string;
  task_id: string;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopCreateLandsatAssetTaskRequest = DesktopCreateLandsatBundleTaskRequest & {
  asset_keys: string[];
};

export type DesktopCreateLandsatAssetBatchTaskRequest = Omit<DesktopCreateLandsatBundleTaskRequest, "item_id"> & {
  item_ids: string[];
  asset_keys?: string[];
  expected_workbench_revision: number;
};

export type DesktopOpticalDeliveryAdmission = DesktopSentinel2ArchiveAdmissionSummary;
export type DesktopLandsatAssetBatchTaskResult =
  | { ok: true; snapshot: DesktopCoreRestoreSnapshot | null; admission: DesktopOpticalDeliveryAdmission }
  | DesktopApplicationError;
export type DesktopTaskDeliveryAdmissionResult =
  | { ok: true; admission: DesktopOpticalDeliveryAdmission | null }
  | DesktopApplicationError;

export type DesktopCreateReviewedMpcAssetTaskRequest = {
  request_id: string;
  source_batch_id: string;
  item_id: string;
  asset_keys: string[];
  task_id: string;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopCreateSentinel2ArchiveTaskRequest = {
  request_id: string;
  source_batch_id: string;
  item_id: string;
  task_id: string;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopCreateSentinel2ArchiveBatchTaskRequest =
  Omit<DesktopCreateSentinel2ArchiveTaskRequest, "item_id"> & {
    /** Empty item_ids consumes the Store-owned checked set at this exact revision. */
    expected_workbench_revision: number;
    item_ids: string[];
  };

export type DesktopSentinel2ArchiveAdmissionRejection = {
  source_item_id: string;
  reason_code: string;
  reason_detail?: string;
};

export type DesktopSentinel2ArchiveAdmissionSummary = {
  requested_count: number;
  admitted_count: number;
  rejected_count: number;
  rejections: DesktopSentinel2ArchiveAdmissionRejection[];
};

export type DesktopSentinel2ArchiveBatchTaskResult = DesktopLandsatAssetBatchTaskResult;

export type DesktopCreateSentinel2BandTaskRequest = DesktopCreateSentinel2ArchiveTaskRequest & {
  asset_keys: string[];
};

export type DesktopCreateSentinel2BandBatchTaskRequest = DesktopCreateLandsatAssetBatchTaskRequest & {
  asset_keys: string[];
};
export type DesktopSentinel2BandBatchTaskResult = DesktopLandsatAssetBatchTaskResult;

export type DesktopCreateHlsBandBatchTaskRequest = DesktopCreateLandsatAssetBatchTaskRequest & {
  asset_keys: string[];
};
export type DesktopHlsBandBatchTaskResult = DesktopLandsatAssetBatchTaskResult;

export type DesktopNisarRemoteSubsetCapability = {
  feature_id: "nisar_remote_subset_v1";
  visible: boolean;
  enabled: boolean;
  implementation_scope: "offline_only" | "live_range_writer_store_v1";
  offline_verified: boolean;
  live_verified: boolean;
  production_enabled: boolean;
  default_enabled: false;
  output_authority: "client_derived";
  supported_product_type: "RSLC";
  supported_specification_versions: string[];
  supported_frequencies: Array<"A" | "B">;
  supported_polarizations: Partial<Record<"A" | "B", string[]>>;
  reason_code: string | null;
};

export type DesktopNisarRemoteSubsetCapabilityResult =
  | { ok: true; capability: DesktopNisarRemoteSubsetCapability }
  | DesktopApplicationError;

export type DesktopOpticalDerivedMode =
  | "single_scene_clip"
  | "batch_single_scene_clip"
  | "near_coincident_spatial_mosaic";

export type DesktopOpticalResampling = "nearest" | "bilinear" | "cubic";

export type DesktopOpticalCloudMaskPolicy = "none" | "cloud_to_nodata";

export type DesktopOpticalMosaicStrategy = "quality_first" | "latest_first";

export type DesktopOpticalSceneCoverageReport = {
  scene_id: string;
  intersects_aoi: boolean;
  aoi_coverage_fraction: number;
  scene_epsg?: number | null;
  acquired_at?: string | null;
  cloud_cover_pct?: number | null;
};

export type DesktopOpticalPrecheckReport = {
  ok: boolean;
  recommended_mode: DesktopOpticalDerivedMode;
  aoi_bbox_wgs84: [number, number, number, number];
  aoi_center_wgs84: [number, number];
  recommended_utm_epsg: number;
  crosses_utm_zones: boolean;
  distinct_utm_zones: number[];
  scene_count: number;
  intersecting_scene_count: number;
  combined_aoi_coverage_fraction: number;
  temporal_span_days?: number | null;
  within_near_coincident_limit: boolean;
  estimated_output_pixels_per_band: number;
  warnings: string[];
  blocking_reasons: string[];
  scene_coverages: DesktopOpticalSceneCoverageReport[];
};

export type DesktopOpticalPrecheckResult =
  | { ok: true; report: DesktopOpticalPrecheckReport }
  | DesktopApplicationError;

export type DesktopOpticalDerivedPrecheckBatchRequest = {
  source_batch_id: string;
  expected_workbench_revision: number;
  item_ids: string[];
  mode?: DesktopOpticalDerivedMode | null;
  aoi_bbox_wgs84?: [number, number, number, number] | null;
  aoi_rings_wgs84?: number[][][] | null;
  target_resolution_m?: number | null;
};

export type DesktopOpticalDerivedCreateTaskRequest = {
  request_id: string;
  task_id: string;
  source_batch_id: string;
  expected_workbench_revision: number;
  item_ids: string[];
  mode: DesktopOpticalDerivedMode;
  aoi_bbox_wgs84?: [number, number, number, number] | null;
  aoi_rings_wgs84?: number[][][] | null;
  band_order: string[];
  target_crs?: string | null;
  target_resolution_m?: number | null;
  resampling: DesktopOpticalResampling;
  cloud_mask: DesktopOpticalCloudMaskPolicy;
  mosaic_strategy: DesktopOpticalMosaicStrategy;
  max_temporal_span_days?: number | null;
  overwrite: boolean;
  write_lineage_sidecar: boolean;
  write_provenance_sidecars?: boolean;
  output_root: string;
  concurrency: number;
  created_at: string;
};

export type DesktopOpticalDerivedTaskCreatedResult =
  | {
      ok: true;
      precheck: DesktopOpticalPrecheckReport;
      task_id: string;
      derived_batch_id: string;
      derived_item_id: string;
      output_tif_path: string;
      output_tif_paths?: string[];
      snapshot: DesktopCoreRestoreSnapshot;
    }
  | DesktopApplicationError;

export type DesktopEffectiveCapabilityState =
  | "executable"
  | "blocked_build"
  | "blocked_runtime"
  | "blocked_credential"
  | "blocked_admission"
  | "blocked_policy"
  | "reserved"
  | "retired";

export type DesktopEffectiveCapabilityReason =
  | "ready"
  | "static_reserved"
  | "static_retired"
  | "build_unavailable_or_unknown"
  | "runtime_not_ready_or_unknown"
  | "credential_missing_or_unknown"
  | "admission_missing_rejected_or_unknown"
  | "policy_denied_or_unknown";

export type DesktopEffectiveCapabilityStatus = {
  resource_id: string;
  capability: DesktopJsonObject & {
    capability_id: string;
    kind: string;
    lifecycle: string;
  };
  inputs: {
    build: "available" | "unavailable" | null;
    runtime: "ready" | "not_ready" | null;
    credential: "ready" | "missing" | null;
    admission: "accepted" | "rejected" | null;
    policy: "allowed" | "denied" | null;
  };
  projection: {
    state: DesktopEffectiveCapabilityState;
    reason: DesktopEffectiveCapabilityReason;
  };
};

export type DesktopSetWorkingAoiRequest = {
  source_kind: string;
  geometry: DesktopJsonObject;
  bbox: number[];
  crs: "EPSG:4326";
  display_name: string;
};

export type DesktopWorkingAoi = {
  state: "no_aoi" | "preview" | "working" | "saved_region";
  source_kind: string;
  geometry: DesktopJsonObject;
  bbox: number[];
  crs: string;
  display_name: string;
};

export type DesktopEffectiveCapabilityResult =
  | { ok: true; status: DesktopEffectiveCapabilityStatus }
  | DesktopApplicationError;

export type DesktopCreateNisarRemoteSubsetTaskRequest = {
  request_id: string;
  source_batch_id: string;
  item_id: string;
  task_id: string;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  frequency: "A" | "B";
  polarization: string;
  created_at: string;
};

/** Formal V2 bridge boundary for multi-polarization NISAR AOI subsets. */
export type DesktopNisarPrepareSubsetIntentV2Request = {
  source_batch_id: string;
  source_item_id: string;
  expected_workbench_revision: number;
  subset_draft_geometry: DesktopJsonObject;
  selected_frequency: "A" | "B";
  selected_polarizations: string[];
};

export type DesktopNisarPreparedSubsetIntentV2 = {
  source_batch_id: string;
  source_item_id: string;
  workbench_revision: number;
  intent: DesktopJsonObject;
};

export type DesktopNisarConfirmSubsetTaskV2Request = {
  source_batch_id: string;
  source_item_id: string;
  expected_workbench_revision: number;
  prepared_intent: DesktopJsonObject;
  task_id: string;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopNisarPreparedSubsetIntentV2Result =
  | { ok: true; prepared: DesktopNisarPreparedSubsetIntentV2 }
  | DesktopApplicationError;

/** Read-only authority check for one bounded, ordered NISAR subset batch. */
export type DesktopNisarPreflightSubsetBatchV2Request = {
  source_batch_id: string;
  source_item_ids: string[];
  expected_workbench_revision: number;
  subset_draft_geometry: DesktopJsonObject;
};

export type DesktopNisarSubsetSelectionV2 = {
  frequency: "A" | "B";
  polarizations: string[];
};

export type DesktopNisarPreflightEligibleItemV2 = {
  source_item_id: string;
  effective_geometry: DesktopJsonObject;
  available_selections: DesktopNisarSubsetSelectionV2[];
};

export type DesktopNisarPreflightSubsetBatchV2 = {
  source_batch_id: string;
  workbench_revision: number;
  eligible_items: DesktopNisarPreflightEligibleItemV2[];
  excluded_items: DesktopNisarSubsetBatchExcludedItemV2[];
  common_selections: DesktopNisarSubsetSelectionV2[];
};

export type DesktopNisarPreflightSubsetBatchV2Result =
  | { ok: true; preflight: DesktopNisarPreflightSubsetBatchV2 }
  | DesktopApplicationError;

/** Formal V2 bridge boundary for one bounded, ordered NISAR subset batch. */
export type DesktopNisarPrepareSubsetBatchV2Request = {
  source_batch_id: string;
  source_item_ids: string[];
  expected_workbench_revision: number;
  subset_draft_geometry: DesktopJsonObject;
  selected_frequency: "A" | "B";
  selected_polarizations: string[];
};

export type DesktopNisarSubsetBatchExcludedItemV2 = {
  source_item_id: string;
  reason: "no_overlap";
};

export type DesktopNisarPreparedSubsetBatchV2 = {
  source_batch_id: string;
  workbench_revision: number;
  intents: DesktopNisarPreparedSubsetIntentV2[];
  excluded_items: DesktopNisarSubsetBatchExcludedItemV2[];
};

export type DesktopNisarPreparedSubsetBatchV2Result =
  | { ok: true; prepared: DesktopNisarPreparedSubsetBatchV2 }
  | DesktopApplicationError;

export type DesktopNisarConfirmSubsetBatchItemV2 = {
  source_item_id: string;
  prepared_intent: DesktopJsonObject;
};

export type DesktopNisarConfirmSubsetBatchTaskV2Request = {
  source_batch_id: string;
  expected_workbench_revision: number;
  items: DesktopNisarConfirmSubsetBatchItemV2[];
  task_id: string;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopNisarAppendSubsetBatchTaskV2Request = {
  task_id: string;
  expected_task_revision: number;
  source_item_ids: string[];
  event_id: string;
  occurred_at: string;
};

export type DesktopNisarAppendContextV2Request = { task_id: string };
export type DesktopNisarAppendContextV2 = {
  task_id: string;
  source_batch_id: string;
  task_revision: number;
  task_status: "queued" | "running" | "paused" | "completed" | "failed" | "cancelled";
  frozen_aoi_geojson: DesktopJsonObject;
  selected_frequency: "A" | "B";
  selected_polarizations: string[];
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  original_item_ids?: string[];
};
export type DesktopNisarAppendContextV2Result =
  | { ok: true; context: DesktopNisarAppendContextV2 }
  | DesktopApplicationError;
export type DesktopNisarAppendPreflightV2Request = {
  task_id: string;
  expected_task_revision: number;
  source_item_ids: string[];
};
export type DesktopNisarAppendExclusionReason = "no_overlap" | "frequency_incompatible" | "polarization_incompatible";
export type DesktopNisarAppendPreflightV2 = {
  task_id: string;
  source_batch_id: string;
  task_revision: number;
  eligible_items: DesktopNisarPreflightEligibleItemV2[];
  excluded_items: Array<{ source_item_id: string; reason: DesktopNisarAppendExclusionReason }>;
  common_selections: DesktopNisarSubsetSelectionV2[];
};
export type DesktopNisarAppendPreflightV2Result =
  | { ok: true; preflight: DesktopNisarAppendPreflightV2 }
  | DesktopApplicationError;
export type DesktopNisarSuccessorTaskV2Request = {
  source_task_id: string;
  expected_task_revision: number;
  source_item_ids: string[];
  new_task_id: string;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  created_at: string;
  recreate_original_items?: boolean;
};

export type DesktopNisarArchivedRecoveryV2Request = {
  source_task_id: string;
  source_item_id: string;
  expected_source_revision: number;
  new_task_id: string;
  created_at: string;
};
export type DesktopNisarArchivedRecoveryContextV2 = Omit<DesktopNisarArchivedRecoveryV2Request, "created_at"> & { output_root: string };
export type DesktopNisarArchivedRecoveryContextV2Result = { ok: true; context: DesktopNisarArchivedRecoveryContextV2 } | DesktopApplicationError;

export type DesktopUpdateSearchWorkbenchRequest = {
  state: {
    batch_id: string;
    checked_item_ids: string[];
    page_index: number;
    page_size: number;
    revision: number;
  };
  expected_revision: number;
  updated_at: string;
};

export type DesktopDownloadLayout =
  | "direct"
  | "sentinel1_slc"
  | "sentinel_orbit_aux_poeorb"
  | "sar_dataset_processing";

export type DesktopCreateDownloadTaskRequest = {
  task_id: string;
  batch_id: string;
  selected_item_ids: string[];
  checked_item_ids: string[];
  page_index: number;
  page_size: number;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopAppendDownloadTaskRequest = {
  task_id: string;
  expected_revision: number;
  item_ids: string[];
  event_id: string;
  occurred_at: string;
};

export type DesktopCreateTaskFromWorkbenchRequest = {
  task_id: string;
  batch_id: string;
  expected_workbench_revision: number;
  exact_item_id?: string | null;
  output_root: string;
  layout: DesktopDownloadLayout;
  concurrency: number;
  created_at: string;
};

export type DesktopAppendTaskFromWorkbenchRequest = {
  task_id: string;
  batch_id: string;
  expected_workbench_revision: number;
  event_id: string;
  occurred_at: string;
};

export type DesktopUpdateDownloadTaskRequest = {
  task_id: string;
  expected_revision: number;
  command: {
    kind:
      | "pause_task"
      | "resume_task"
      | "retry_task"
      | "cancel_task"
      | "activate_items"
      | "pause_items"
      | "resume_items"
      | "retry_items"
      | "mark_completed"
      | "mark_failed"
      | "mark_skipped"
      | "cancel_items"
      | "reacquire_items";
    item_ids: string[];
  };
  event_id: string;
  occurred_at: string;
};

export type DesktopUpdateDownloadTaskViewRequest = {
  task_id: string;
  checked_item_ids: string[];
  page_index: number;
  page_size: number;
  updated_at: string;
};

export type DesktopArchiveDownloadTaskRequest = {
  task_id: string;
  expected_revision: number;
  archived_at: string;
};

export type DesktopSourceMutationCommitStatus =
  | "committed"
  | "legacy_committed"
  | "event_committed"
  | "conflict"
  | "unknown";

export type DesktopSourceMutationTargetStatus =
  | "present"
  | "tombstoned"
  | "missing";

export type DesktopSourceMutationStatus = {
  commit: DesktopSourceMutationCommitStatus;
  target: DesktopSourceMutationTargetStatus;
};

export type DesktopSourceMutationStatusResult =
  | { ok: true; status: DesktopSourceMutationStatus }
  | DesktopApplicationError;

export type DesktopSourceMutationQuery =
  | { kind: "source_append"; request: DesktopAppendTaskFromSourceBatchRequest }
  | { kind: "historical_successor"; request: DesktopCreateTaskFromHistoricalSourceBatchRequest }
  | { kind: "burst_append"; request: DesktopBurstAppendAssetTaskRequest }
  | { kind: "burst_successor"; request: DesktopBurstCreateHistoricalAssetTaskRequest }
  | { kind: "nisar_append"; request: DesktopNisarAppendSubsetBatchTaskV2Request }
  | { kind: "nisar_successor"; request: DesktopNisarSuccessorTaskV2Request }
  | { kind: "nisar_archived_recovery"; request: DesktopNisarArchivedRecoveryV2Request }
  | { kind: "reacquire"; request: DesktopUpdateDownloadTaskRequest };

export type DesktopSarDownloadPolicySettings = SarDownloadPolicySettings;
export type DesktopOpenExternalUrlResult =
  | { ok: true; url: string }
  | DesktopApplicationError;

export type DesktopSarBackgroundCountRequest = {
  request_id: string;
  batch_id: string;
  expected_workbench_revision: number;
};

export type DesktopSarBackgroundCountResult =
  | {
      ok: true;
      applied: boolean;
      total_count: number | null;
      snapshot: Extract<DesktopCoreRestoreSnapshot, { ok: true }> | null;
    }
  | DesktopApplicationError;

export type DesktopSarSearchProgressStage = ApplicationSearchProgressStage;
export type DesktopSarSearchProgress = ApplicationSearchProgress;

export type DesktopSarSearchProgressResult =
  | { ok: true; progress: DesktopSarSearchProgress | null }
  | DesktopApplicationError;

export type DesktopDemConversionSourceRequest = {
  region_id: string;
  dataset: string;
  "provider": string;
  resource_id: string;
  capability_id: string;
};

export type DesktopDemConversionSource = {
  task_id: string;
  revision: number;
  region_id: string;
  dataset: string;
  "provider": string;
  resource_id: string;
  capability_id: string;
  input_path: string;
  output_root: string;
};

export type DesktopDemConversionSourceResult =
  | { ok: true; source: DesktopDemConversionSource | null }
  | DesktopApplicationError;

export type DesktopDemSourceIdentity = {
  dataset: string;
  "provider": string;
  updated_at: string;
};

export type DesktopDemSourceIdentityResult =
  | { ok: true; identity: DesktopDemSourceIdentity }
  | DesktopApplicationError;

export type DesktopArchiveDownloadTaskResult =
  | {
      ok: true;
      archive: {
        task_id: string;
        revision: number;
        archived_at: string;
      };
    }
  | DesktopApplicationError;

export type DesktopDeleteDownloadHistoryRequest = {
  task_id: string;
  deleted_at: string;
};

export type DesktopProviderCapabilities =
  | { ok: true; capabilities: DesktopJsonObject }
  | DesktopApplicationError;


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

export type DesktopApplicationBridge = {
  getRuntimeComponents?(): Promise<RuntimeComponentsReport>;
  fetchExtensionRegistry?(registryUrl?: string): Promise<ExtensionRegistryReport>;
  installRuntimeComponent?(
    request: RuntimeComponentInstallRequest,
  ): Promise<RuntimeComponentInstallResult>;
  getPluginsStatus?(): Promise<any>;
  installPluginPackage?(request: any): Promise<any>;
  setPluginEnabled?(pluginId: string, enabled: boolean): Promise<any>;
  uninstallPluginPackage?(pluginId: string): Promise<any>;
  saveCustomConnector?(connector: any): Promise<any>;
  deleteCustomConnector?(connectorId: string): Promise<any>;
  probeCustomConnectorTools?(connectorId: string): Promise<any>;
  saveCustomSkill?(skill: any): Promise<any>;
  deleteCustomSkill?(skillId: string): Promise<any>;
  installCustomSkillFromUrl?(url: string, fallbackName?: string | null): Promise<any>;
  installCustomSkillFromFolder?(folderPath: string): Promise<any>;
  getUsageStatistics?(): Promise<any>;
  gisRasterInspect?(request: any): Promise<any>;
  gisRasterPixelProbe?(request: { source: string; lon: number; lat: number }): Promise<any>;
  gisRasterProcess?(request: any): Promise<any>;
  gisVectorProcess?(request: any): Promise<any>;
  externalGisInspect?(request?: any): Promise<any>;
  externalGisSavePrefs?(prefs: any): Promise<any>;
  externalGisExecute?(request: any): Promise<any>;
  condaInstallGeoPackage?(request: {
    package_id: string;
    conda_env?: string | null;
    confirmed: boolean;
  }): Promise<any>;
  lbsCoordTransform?(request: any): Promise<any>;
  mapgisInspect?(request: any): Promise<any>;
  mapgisConvert?(request: any): Promise<any>;
  openGeodataQuery?(request: any): Promise<any>;
  fetchGeodataPreviewDataUrl?(url: string): Promise<{ ok: boolean; data_url?: string; error?: string }>;
  experimentProjectGet?(): Promise<any>;
  experimentProjectAddTask?(taskId: string): Promise<any>;
  experimentProjectRemoveItem?(itemId: string): Promise<any>;
  experimentProjectExport?(exportDir?: string | null): Promise<any>;
  desktopOnboardingStateGet?(): Promise<{ completed: boolean; returning_user?: boolean }>;
  desktopOnboardingStateSet?(completed: boolean): Promise<{ completed: boolean }>;
  desktopActiveResourceGet?(): Promise<string | null>;
  desktopActiveResourceSet?(resourceId: string): Promise<string | null>;
  getCacheSummary?(): Promise<{ ok?: boolean; cache_dir?: string } | null>;
  diagnosticsGetState?(): Promise<{ enabled: boolean; total_count: number; pending_count: number }>;
  diagnosticsSetEnabled?(enabled: boolean): Promise<{ enabled: boolean; total_count: number; pending_count: number }>;
  diagnosticsRecord?(input: {
    category: string;
    error_code?: string | null;
    http_status?: number | null;
    source?: string | null;
    message: string;
    stack?: string | null;
  }): Promise<any>;
  diagnosticsList?(): Promise<any[]>;
  diagnosticsClear?(): Promise<{ enabled: boolean; total_count: number; pending_count: number }>;
  diagnosticsUpload?(): Promise<{
    attempted: number;
    uploaded: number;
    failed: number;
    message_zh: string;
  }>;
  checkAppUpdate?(): Promise<{
    current_version: string;
    latest_version: string | null;
    has_update: boolean;
    release_url: string | null;
    release_notes: string | null;
    checked_at: string;
    status: string;
    message_zh: string;
  }>;
  getReadiness(): Promise<DesktopApplicationReadiness>;
  coreRestore(includeTerminal?: boolean): Promise<DesktopCoreRestoreSnapshot>;
  workbenchPage(request: DesktopWorkbenchPageRequest): Promise<DesktopWorkbenchPageSnapshot>;
  taskPage(request: DesktopTaskPageRequest): Promise<DesktopTaskPageSnapshot>;
  taskSourceBatchPage(
    request: DesktopTaskSourceBatchPageRequest,
  ): Promise<DesktopTaskSourceBatchPageSnapshot>;
  taskHistoryPage(request: DesktopTaskHistoryPageRequest): Promise<DesktopTaskHistoryPageSnapshot>;
  taskEventPage(request: DesktopTaskEventPageRequest): Promise<DesktopTaskEventPageSnapshot>;
  openTaskOutput(request: DesktopOpenTaskOutputRequest): Promise<DesktopOpenTaskOutputResult>;
  listTaskConflictFiles(
    request: DesktopListTaskConflictFilesRequest,
  ): Promise<DesktopTaskConflictFileListResult>;
  deleteTaskConflictFiles(
    request: DesktopDeleteTaskConflictFilesRequest,
  ): Promise<DesktopTaskConflictFileDeleteResult>;
  updateWorkbenchSelection(
    command: DesktopWorkbenchSelectionCommand,
  ): Promise<DesktopWorkbenchSelectionResult>;
  createTaskFromWorkbench(
    request: DesktopCreateTaskFromWorkbenchRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  appendTaskFromWorkbench(
    request: DesktopAppendTaskFromWorkbenchRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  appendTaskFromSourceBatch(
    request: DesktopAppendTaskFromSourceBatchRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createTaskFromHistoricalSourceBatch(
    request: DesktopCreateTaskFromHistoricalSourceBatchRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createBurstAssetTask(
    request: DesktopBurstCreateAssetTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  appendBurstAssetTask(
    request: DesktopBurstAppendAssetTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createHistoricalBurstAssetTask(
    request: DesktopBurstCreateHistoricalAssetTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  workspacePrepareLocation(
    request: DesktopWorkspacePrepareLocationRequest,
  ): Promise<DesktopWorkspacePrepareLocationResult>;
  workspaceOpen(request: DesktopWorkspaceOpenRequest): Promise<DesktopWorkspaceContextResult>;
  workspaceCreate(request: DesktopWorkspaceCreateRequest): Promise<DesktopWorkspaceContextResult>;
  projectCreate(request: DesktopWorkspaceNamedNodeRequest): Promise<DesktopWorkspaceContextResult>;
  projectSelect(request: DesktopWorkspaceSelectNodeRequest): Promise<DesktopWorkspaceContextResult>;
  regionCreate(request: DesktopWorkspaceNamedNodeRequest): Promise<DesktopWorkspaceContextResult>;
  regionSelect(request: DesktopWorkspaceSelectNodeRequest): Promise<DesktopWorkspaceContextResult>;
  /** Present on the formal Tauri route; legacy/browser adapters do not own these tasks. */
  aoiImport?(request: DesktopAoiImportRequest): Promise<DesktopNativeJsonResult>;
  aoiNormalize?(request: DesktopAoiNormalizeRequest): Promise<DesktopNativeJsonResult>;
  adminBoundarySearch?(request: DesktopAdminBoundarySearchRequest): Promise<DesktopNativeJsonResult>;
  adminBoundaryResolve?(request: DesktopAdminBoundaryResolveRequest): Promise<DesktopNativeJsonResult>;
  adminOptions?(request: DesktopAdminBoundaryOptionsRequest): Promise<DesktopNativeJsonResult>;
  gacosPlan?(payload: DesktopJsonObject): Promise<DesktopNativeJsonResult>;
  gacosDryRun?(payload: DesktopJsonObject): Promise<DesktopNativeJsonResult>;
  reportGenerate?(payload: DesktopJsonObject): Promise<DesktopNativeJsonResult>;
  getDemTaskSettings?(): Promise<DesktopDemTaskSettings | DesktopApplicationError>;
  saveDemTaskSettings?(
    settings: DesktopDemTaskSettings,
  ): Promise<DesktopDemTaskSettings | DesktopApplicationError>;
  getSarDownloadPolicySettings?(): Promise<DesktopSarDownloadPolicySettings | DesktopApplicationError>;
  saveSarDownloadPolicySettings?(
    settings: DesktopSarDownloadPolicySettings,
  ): Promise<DesktopSarDownloadPolicySettings | DesktopApplicationError>;
  openExternalUrl?(url: string): Promise<DesktopOpenExternalUrlResult>;
  standaloneDem?(
    request: DesktopStandaloneDemRequest,
  ): Promise<DesktopNativeJsonResult>;
  createStandaloneDemTask?(
    request: DesktopStandaloneDemTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  importScenes?(request: DesktopSceneImportRequest): Promise<DesktopNativeJsonResult>;
  matchOrbitsDirectory?(
    request: DesktopOrbitDirectoryMatchRequest,
  ): Promise<DesktopNativeJsonResult>;
  gacosLedger?(): Promise<DesktopNativeJsonResult>;
  gacosImportResult?(
    request: DesktopGacosResultImportRequest,
  ): Promise<DesktopNativeJsonResult>;
  gacosCreateImportTask?(
    request: DesktopGacosCreateImportTaskRequest,
  ): Promise<DesktopGacosCreateImportTaskResult>;
  gacosCreateResultTask?(
    request: DesktopGacosCreateResultTaskRequest,
  ): Promise<DesktopGacosCreateImportTaskResult>;
  gacosSupplyResultLink?(
    request: DesktopGacosSupplyResultLinkRequest,
  ): Promise<DesktopGacosSupplyResultLinkResult>;
  gacosSubmitRequest?(
    request: DesktopGacosConfirmRequest,
  ): Promise<DesktopGacosSubmissionResult>;
  gacosRequestStatus?(taskId: string): Promise<DesktopGacosSubmissionResult>;
  getNetworkSettings?(): Promise<DesktopNetworkSettings | DesktopApplicationError>;
  saveNetworkSettings?(
    settings: DesktopNetworkSettings,
  ): Promise<DesktopNetworkSettings | DesktopApplicationError>;
  probeProviderConnectivity?(): Promise<
    DesktopProviderConnectivityResult[] | DesktopApplicationError
  >;
  planDemTask?(request: DesktopDemTaskPlanRequest): Promise<DesktopDemTaskPlanResult>;
  createDemTask?(request: DesktopDemTaskRequest): Promise<DesktopCoreRestoreSnapshot>;
  setDemSourceIdentity?(
    identity: DesktopDemSourceIdentity,
  ): Promise<DesktopDemSourceIdentityResult>;
  resolveDemConversionSource?(
    request: DesktopDemConversionSourceRequest,
  ): Promise<DesktopDemConversionSourceResult>;
  handoffDemConversion?(request: DesktopDemConversionRequest): Promise<DesktopCoreRestoreSnapshot>;
  createOrbitTask?(request: DesktopOrbitTaskRequest): Promise<DesktopCoreRestoreSnapshot>;
  createStandaloneOrbitTask?(
    request: DesktopStandaloneOrbitTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createCompanionOrbitTask?(sourceTaskId: string): Promise<DesktopCoreRestoreSnapshot>;
  providerCapabilities(): Promise<DesktopProviderCapabilities>;
  bindRegionAoi(request: DesktopBindRegionAoiRequest): Promise<DesktopCoreRestoreSnapshot>;
  clearRegionAoi(request: DesktopClearRegionAoiRequest): Promise<DesktopCoreRestoreSnapshot>;
  setWorkingAoi?(
    request: DesktopSetWorkingAoiRequest,
  ): Promise<DesktopWorkingAoi | DesktopApplicationError>;
  clearWorkingAoi?(): Promise<{ ok: true } | DesktopApplicationError>;
  getWorkingAoi?(): Promise<DesktopWorkingAoi | null | DesktopApplicationError>;
  sarSearchAndFreeze(request: DesktopSarSearchAndFreezeRequest): Promise<DesktopCoreRestoreSnapshot>;
  sarBackgroundCount(request: DesktopSarBackgroundCountRequest): Promise<DesktopSarBackgroundCountResult>;
  sarSearchProgress(requestId: string): Promise<DesktopSarSearchProgressResult>;
  catalogSearchAndFreeze(
    request: DesktopCatalogSearchAndFreezeRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  catalogIntentSearchAndFreeze(
    request: DesktopCatalogIntentSearchAndFreezeRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  cancelRequest(requestId: string): Promise<{ ok: true; cancelled: boolean } | DesktopApplicationError>;
  cdseSessionStatus(): Promise<DesktopCdseSessionStatusResult>;
  cdseLogin(request: DesktopCdseLoginRequest): Promise<DesktopCdseSessionStatusResult>;
  cdseLogout(): Promise<DesktopCdseSessionStatusResult>;
  cdseNetworkPolicy(): Promise<DesktopCdseNetworkPolicyResult>;
  setCdseNetworkPolicy(
    networkMode: DesktopCdseNetworkMode,
  ): Promise<DesktopCdseSessionStatusResult>;
  cdseNetworkDiagnostic(
    networkMode: DesktopCdseNetworkMode,
  ): Promise<DesktopCdseNetworkDiagnosticResult>;
  previewAsset(request: DesktopPreviewAssetRequest): Promise<DesktopPreviewAssetResult>;
  savePreview?(request: DesktopPreviewSaveRequest): Promise<DesktopPreviewSaveResult>;
  cdseQuicklook(request: DesktopCdseQuicklookRequest): Promise<DesktopCdseQuicklookResult>;
  sentinel2MapPreview(
    request: DesktopSentinel2MapPreviewRequest,
  ): Promise<DesktopSentinel2MapPreviewResult>;
  usgsSessionStatus(): Promise<DesktopUsgsM2mSessionStatusResult>;
  usgsLogin(request: DesktopUsgsM2mLoginRequest): Promise<DesktopUsgsM2mSessionStatusResult>;
  usgsLogout(): Promise<DesktopUsgsM2mSessionStatusResult>;
  landsatBrowse(request: DesktopLandsatBrowseRequest): Promise<DesktopLandsatBrowseResult>;
  landsatAssetDetails(request: DesktopLandsatAssetDetailsRequest): Promise<DesktopLandsatAssetDetailsResult>;
  bhoonidhiAssetSize(request: DesktopBhoonidhiAssetSizeRequest): Promise<DesktopBhoonidhiAssetSizeResult>;
  createLandsatBundleTask(
    request: DesktopCreateLandsatBundleTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createLandsatAssetTask(
    request: DesktopCreateLandsatAssetTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createLandsatAssetBatchTask(request: DesktopCreateLandsatAssetBatchTaskRequest): Promise<DesktopLandsatAssetBatchTaskResult>;
  createLandsatBundleBatchTask(request: DesktopCreateLandsatAssetBatchTaskRequest): Promise<DesktopLandsatAssetBatchTaskResult>;
  taskDeliveryAdmission(taskId: string): Promise<DesktopTaskDeliveryAdmissionResult>;
  opticalPreparationProgress(requestId: string): Promise<import("./opticalPreparation").OpticalPreparationResult>;
  createReviewedMpcAssetTask(
    request: DesktopCreateReviewedMpcAssetTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createSentinel2ArchiveTask(
    request: DesktopCreateSentinel2ArchiveTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createSentinel2ArchiveBatchTask(
    request: DesktopCreateSentinel2ArchiveBatchTaskRequest,
  ): Promise<DesktopSentinel2ArchiveBatchTaskResult>;
  createSentinel2BandTask(
    request: DesktopCreateSentinel2BandTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createSentinel2BandBatchTask(request: DesktopCreateSentinel2BandBatchTaskRequest): Promise<DesktopSentinel2BandBatchTaskResult>;
  createSentinel2L1cBandBatchTask?(request: DesktopCreateSentinel2BandBatchTaskRequest): Promise<DesktopSentinel2BandBatchTaskResult>;
  createHlsBandBatchTask?(request: DesktopCreateHlsBandBatchTaskRequest): Promise<DesktopHlsBandBatchTaskResult>;
  precheckOpticalAoiCoverage(
    request: DesktopOpticalDerivedPrecheckBatchRequest,
  ): Promise<DesktopOpticalPrecheckResult>;
  createOpticalDerivedTask(
    request: DesktopOpticalDerivedCreateTaskRequest,
  ): Promise<DesktopOpticalDerivedTaskCreatedResult>;
  effectiveCapability(capabilityId: string): Promise<DesktopEffectiveCapabilityResult>;
  nisarRemoteSubsetCapability(): Promise<DesktopNisarRemoteSubsetCapabilityResult>;
  createNisarRemoteSubsetTask(
    request: DesktopCreateNisarRemoteSubsetTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  prepareNisarSubsetIntentV2(
    request: DesktopNisarPrepareSubsetIntentV2Request,
  ): Promise<DesktopNisarPreparedSubsetIntentV2Result>;
  preflightNisarSubsetBatchV2(
    request: DesktopNisarPreflightSubsetBatchV2Request,
  ): Promise<DesktopNisarPreflightSubsetBatchV2Result>;
  prepareNisarSubsetBatchV2(
    request: DesktopNisarPrepareSubsetBatchV2Request,
  ): Promise<DesktopNisarPreparedSubsetBatchV2Result>;
  confirmNisarSubsetTaskV2(
    request: DesktopNisarConfirmSubsetTaskV2Request,
  ): Promise<DesktopCoreRestoreSnapshot>;
  confirmNisarSubsetBatchTaskV2(
    request: DesktopNisarConfirmSubsetBatchTaskV2Request,
  ): Promise<DesktopCoreRestoreSnapshot>;
  appendNisarSubsetBatchTaskV2(
    request: DesktopNisarAppendSubsetBatchTaskV2Request,
  ): Promise<DesktopCoreRestoreSnapshot>;
  readNisarSubsetAppendContextV2(request: DesktopNisarAppendContextV2Request): Promise<DesktopNisarAppendContextV2Result>;
  preflightNisarSubsetAppendV2(request: DesktopNisarAppendPreflightV2Request): Promise<DesktopNisarAppendPreflightV2Result>;
  createNisarSubsetSuccessorTaskV2(request: DesktopNisarSuccessorTaskV2Request): Promise<DesktopCoreRestoreSnapshot>;
  readNisarArchivedRecoveryContextV2(sourceTaskId: string, newTaskId: string): Promise<DesktopNisarArchivedRecoveryContextV2Result>;
  createNisarArchivedRecoveryV2(request: DesktopNisarArchivedRecoveryV2Request): Promise<DesktopCoreRestoreSnapshot>;
  resumeNisarSubsetTask(taskId: string): Promise<DesktopCoreRestoreSnapshot>;
  startNewlyConfirmedTask(taskId: string): Promise<DesktopCoreRestoreSnapshot>;
  updateSearchWorkbench(
    request: DesktopUpdateSearchWorkbenchRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  createDownloadTask(
    request: DesktopCreateDownloadTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  appendDownloadTask(
    request: DesktopAppendDownloadTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  updateDownloadTask(
    request: DesktopUpdateDownloadTaskRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  updateDownloadTaskView(
    request: DesktopUpdateDownloadTaskViewRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  archiveDownloadTask(
    request: DesktopArchiveDownloadTaskRequest,
  ): Promise<DesktopArchiveDownloadTaskResult>;
  deleteDownloadHistory(
    request: DesktopDeleteDownloadHistoryRequest,
  ): Promise<DesktopCoreRestoreSnapshot>;
  querySourceAppendStatus(
    request: DesktopAppendTaskFromSourceBatchRequest,
  ): Promise<DesktopSourceMutationStatusResult>;
  queryHistoricalSuccessorStatus(
    request: DesktopCreateTaskFromHistoricalSourceBatchRequest,
  ): Promise<DesktopSourceMutationStatusResult>;
  queryBurstAppendStatus(
    request: DesktopBurstAppendAssetTaskRequest,
  ): Promise<DesktopSourceMutationStatusResult>;
  queryBurstSuccessorStatus(
    request: DesktopBurstCreateHistoricalAssetTaskRequest,
  ): Promise<DesktopSourceMutationStatusResult>;
  queryNisarSuccessorStatus(
    request: DesktopNisarSuccessorTaskV2Request,
  ): Promise<DesktopSourceMutationStatusResult>;
  queryNisarArchivedRecoveryStatus(
    request: DesktopNisarArchivedRecoveryV2Request,
  ): Promise<DesktopSourceMutationStatusResult>;
  queryReacquireStatus(
    request: DesktopUpdateDownloadTaskRequest,
  ): Promise<DesktopSourceMutationStatusResult>;
  querySourceMutationStatus(
    query: DesktopSourceMutationQuery,
  ): Promise<DesktopSourceMutationStatusResult>;
  unified?: DesktopUnifiedBridge;
};

export type DesktopUnifiedBridge = {
  listSources(): Promise<SourceDescriptor[]>;
  sourceViews(): Promise<SourceView[]>;
  searchStart(request: UnifiedSearchRequest, options?: { bypassCache?: boolean }): Promise<SearchProgress>;
  searchProgress(requestId: string): Promise<SearchProgress | null>;
  searchCancel(requestId: string): Promise<boolean>;
  /**
   * #22 B slice 3(c): cancel a catalogue search; rows of a search that already
   * showed its first page are kept (state cancelled, possibly incomplete).
   */
  cancelCatalogSearch?(requestId: string): Promise<SearchCancelOutcome>;
  /** A page of a loading batch that has not arrived rejects with V3_STORE_BATCH_PAGE_NOT_YET_LOADED. */
  batchPage(batchId: string, revision: number, page: number, pageSize: number): Promise<StoreBatchPage>;
  createTask(batchId: string, revision: number, itemIds: string[], options: CreateTaskOptions): Promise<TaskRef>;
  previewAllMatchTask?(
    request: import("../shell/api/types.ts").BatchAllMatchPreviewRequest,
  ): Promise<import("../shell/api/types.ts").BatchAllMatchPreview>;
  createAllMatchTask?(
    request: import("../shell/api/types.ts").BatchAllMatchCreateTaskRequest,
  ): Promise<import("../shell/api/types.ts").BatchAllMatchTaskOutcome>;
  taskAction(taskId: string, revision: number, action: "pause" | "resume" | "retry" | "cancel" | "archive"): Promise<TaskRef>;
  taskItemAction?(
    taskId: string,
    revision: number | null,
    itemId: string,
    action: import("../shell/api/types.ts").UnifiedTaskItemAction,
  ): Promise<TaskRef>;
  taskDeleteHistory?(taskId: string): Promise<void>;
  taskSetPinned?(taskId: string, pinned: boolean): Promise<string[]>;
  listPinnedTasks?(): Promise<string[]>;
  taskRetryFallback(taskId: string, confirmRefreshedBatchId: string | null): Promise<TaskRetryFallbackOutcome>;
  taskRetryFallbackEligibility?(taskIds: string[]): Promise<Record<string, boolean>>;
  checkExistingDownloadedScenes?(
    items: import("../shell/api/types.ts").ExistingDownloadCheckItem[],
    targetDir?: string | null,
  ): Promise<import("../shell/api/types.ts").ExistingDownloadMatch[]>;
  taskOverview(): Promise<TaskOverview>;
  accountSessionStatuses(): Promise<AccountSessionStatus[]>;
  testNetworkConnection(): Promise<string>;
};


export type { DesktopAgentBridge };

export interface DesktopShellBridge {
  readonly kind: DesktopShellKind;
  ready(): Promise<boolean>;
  /** Present only for Tauri shells; pywebview keeps its legacy API boundary. */
  readonly credentials?: DesktopCredentialBridge;
  readonly application: DesktopApplicationBridge;
  readonly compatibility: DesktopCompatibilityBridge;
  readonly agent?: DesktopAgentBridge;
  readonly app: {
    getInfo(): Promise<DesktopAppInfo>;
  };
  readonly window: {
    minimize(): Promise<DesktopCommandResult>;
    toggleMaximize(): Promise<DesktopCommandResult>;
    close(): Promise<DesktopCommandResult>;
    getSize(): Promise<DesktopWindowSize>;
    startDragging?(): Promise<DesktopCommandResult>;
    resizeFromEdge(
      edge: string,
      startWidth: number,
      startHeight: number,
      deltaX: number,
      deltaY: number,
    ): Promise<DesktopCommandResult>;
  };
  readonly dialog: {
    pickOpenFile(title?: string, filters?: string[]): Promise<DesktopPickResult>;
    pickDirectory(title?: string): Promise<DesktopPickResult>;
  };
}

export type DesktopShellSelection = {
  testMode?: boolean;
  testAdapter?: DesktopShellBridge | null;
  tauriAdapter?: DesktopShellBridge | null;
  tauriOriginValidated?: boolean;
  browserFixtureAllowed?: boolean;
  browserFixture?: DesktopShellBridge | null;
};

function unavailableResult(): DesktopCommandResult {
  return {
    ok: false,
    error: "Desktop shell bridge is unavailable",
    code: "GUI003",
  };
}

function unavailableWindowSize(): DesktopWindowSize {
  return {
    ok: false,
    error: "Desktop shell bridge is unavailable",
    code: "GUI003",
  };
}

function unavailableApplicationError(): DesktopApplicationError {
  return {
    ok: false,
    error: "Desktop application service is unavailable",
    code: "V3_APPLICATION_UNAVAILABLE",
  };
}

function createUnavailableDesktopShellBridge(): DesktopShellBridge {
  return {
    kind: "unavailable",
    ready: async () => false,
    application: {
      bindRegionAoi: async () => unavailableApplicationError(),
      clearRegionAoi: async () => unavailableApplicationError(),

      getReadiness: async () => unavailableApplicationError(),
      coreRestore: async () => unavailableApplicationError(),
      workbenchPage: async () => unavailableApplicationError(),
      taskPage: async () => unavailableApplicationError(),
      taskSourceBatchPage: async () => unavailableApplicationError(),
      taskHistoryPage: async () => unavailableApplicationError(),
      taskEventPage: async () => unavailableApplicationError(),
      openTaskOutput: async () => unavailableApplicationError(),
      listTaskConflictFiles: async () => unavailableApplicationError(),
      deleteTaskConflictFiles: async () => unavailableApplicationError(),
      updateWorkbenchSelection: async () => unavailableApplicationError(),
      createTaskFromWorkbench: async () => unavailableApplicationError(),
      appendTaskFromWorkbench: async () => unavailableApplicationError(),
      appendTaskFromSourceBatch: async () => unavailableApplicationError(),
      createTaskFromHistoricalSourceBatch: async () => unavailableApplicationError(),
      createBurstAssetTask: async () => unavailableApplicationError(),
      appendBurstAssetTask: async () => unavailableApplicationError(),
      createHistoricalBurstAssetTask: async () => unavailableApplicationError(),
      workspacePrepareLocation: async () => unavailableApplicationError(),
      workspaceOpen: async () => unavailableApplicationError(),
      workspaceCreate: async () => unavailableApplicationError(),
      projectCreate: async () => unavailableApplicationError(),
      projectSelect: async () => unavailableApplicationError(),
      regionCreate: async () => unavailableApplicationError(),
      regionSelect: async () => unavailableApplicationError(),
      planDemTask: async () => unavailableApplicationError(),
      createDemTask: async () => unavailableApplicationError(),
      setDemSourceIdentity: async () => unavailableApplicationError(),
      resolveDemConversionSource: async () => unavailableApplicationError(),
      handoffDemConversion: async () => unavailableApplicationError(),
      createOrbitTask: async () => unavailableApplicationError(),
      createStandaloneOrbitTask: async () => unavailableApplicationError(),
      providerCapabilities: async () => unavailableApplicationError(),
      sarSearchAndFreeze: async () => unavailableApplicationError(),
      sarBackgroundCount: async () => unavailableApplicationError(),
      sarSearchProgress: async () => unavailableApplicationError(),
      catalogSearchAndFreeze: async () => unavailableApplicationError(),
      catalogIntentSearchAndFreeze: async () => unavailableApplicationError(),
      cancelRequest: async () => unavailableApplicationError(),
      cdseSessionStatus: async () => unavailableApplicationError(),
      cdseLogin: async () => unavailableApplicationError(),
      cdseLogout: async () => unavailableApplicationError(),
      cdseNetworkPolicy: async () => unavailableApplicationError(),
      setCdseNetworkPolicy: async () => unavailableApplicationError(),
      cdseNetworkDiagnostic: async () => unavailableApplicationError(),
      previewAsset: async () => unavailableApplicationError(),
      cdseQuicklook: async () => unavailableApplicationError(),
      sentinel2MapPreview: async () => unavailableApplicationError(),
      usgsSessionStatus: async () => unavailableApplicationError(),
      usgsLogin: async () => unavailableApplicationError(),
      usgsLogout: async () => unavailableApplicationError(),
      landsatBrowse: async () => unavailableApplicationError(),
      landsatAssetDetails: async () => unavailableApplicationError(),
      bhoonidhiAssetSize: async () => unavailableApplicationError(),
      createLandsatBundleTask: async () => unavailableApplicationError(),
      createLandsatAssetTask: async () => unavailableApplicationError(),
      createLandsatAssetBatchTask: async () => unavailableApplicationError(),
      createLandsatBundleBatchTask: async () => unavailableApplicationError(),
      opticalPreparationProgress: async () => unavailableApplicationError(),
      taskDeliveryAdmission: async () => unavailableApplicationError(),
      createReviewedMpcAssetTask: async () => unavailableApplicationError(),
      createSentinel2ArchiveTask: async () => unavailableApplicationError(),
      createSentinel2ArchiveBatchTask: async () => unavailableApplicationError(),
      createSentinel2BandTask: async () => unavailableApplicationError(),
      createSentinel2BandBatchTask: async () => unavailableApplicationError(),
      createSentinel2L1cBandBatchTask: async () => unavailableApplicationError(),
      createHlsBandBatchTask: async () => unavailableApplicationError(),
      precheckOpticalAoiCoverage: async () => unavailableApplicationError(),
      createOpticalDerivedTask: async () => unavailableApplicationError(),
      effectiveCapability: async () => unavailableApplicationError(),
      nisarRemoteSubsetCapability: async () => unavailableApplicationError(),
      createNisarRemoteSubsetTask: async () => unavailableApplicationError(),
      prepareNisarSubsetIntentV2: async () => unavailableApplicationError(),
      preflightNisarSubsetBatchV2: async () => unavailableApplicationError(),
      prepareNisarSubsetBatchV2: async () => unavailableApplicationError(),
      confirmNisarSubsetTaskV2: async () => unavailableApplicationError(),
      confirmNisarSubsetBatchTaskV2: async () => unavailableApplicationError(),
      appendNisarSubsetBatchTaskV2: async () => unavailableApplicationError(),
      readNisarSubsetAppendContextV2: async () => unavailableApplicationError(),
      preflightNisarSubsetAppendV2: async () => unavailableApplicationError(),
      createNisarSubsetSuccessorTaskV2: async () => unavailableApplicationError(),
      readNisarArchivedRecoveryContextV2: async () => unavailableApplicationError(),
      createNisarArchivedRecoveryV2: async () => unavailableApplicationError(),
      resumeNisarSubsetTask: async () => unavailableApplicationError(),
      startNewlyConfirmedTask: async () => unavailableApplicationError(),
      updateSearchWorkbench: async () => unavailableApplicationError(),
      createDownloadTask: async () => unavailableApplicationError(),
      appendDownloadTask: async () => unavailableApplicationError(),
      updateDownloadTask: async () => unavailableApplicationError(),
      updateDownloadTaskView: async () => unavailableApplicationError(),
      archiveDownloadTask: async () => unavailableApplicationError(),
      deleteDownloadHistory: async () => unavailableApplicationError(),
      querySourceAppendStatus: async () => unavailableApplicationError(),
      queryHistoricalSuccessorStatus: async () => unavailableApplicationError(),
      queryBurstAppendStatus: async () => unavailableApplicationError(),
      queryBurstSuccessorStatus: async () => unavailableApplicationError(),
      queryNisarSuccessorStatus: async () => unavailableApplicationError(),
      queryNisarArchivedRecoveryStatus: async () => unavailableApplicationError(),
      queryReacquireStatus: async () => unavailableApplicationError(),
      querySourceMutationStatus: async () => unavailableApplicationError(),
    },
    compatibility: {
      capabilities: async () => ({
        ok: false,
        error: "Desktop compatibility Worker is unavailable",
        code: "COMPAT_WORKER_UNAVAILABLE",
      }),
      callLegacy: async () => ({
        ok: false,
        error: "Desktop compatibility Worker is unavailable",
        code: "COMPAT_WORKER_UNAVAILABLE",
      }),
    },
    app: {
      getInfo: async () => ({
        name: "EOStage",
        version: "unavailable",
        offline: true,
      }),
    },
    window: {
      minimize: async () => unavailableResult(),
      toggleMaximize: async () => unavailableResult(),
      close: async () => unavailableResult(),
      getSize: async () => unavailableWindowSize(),
      startDragging: async () => unavailableResult(),
      resizeFromEdge: async () => unavailableResult(),
    },
    dialog: {
      pickOpenFile: async () => ({
        ok: false,
        path: "",
        error: "Desktop shell bridge is unavailable",
        code: "GUI003",
      }),
      pickDirectory: async () => ({
        ok: false,
        path: "",
        error: "Desktop shell bridge is unavailable",
        code: "GUI003",
      }),
    },
  };
}

export function selectDesktopShellBridge(selection: DesktopShellSelection): DesktopShellBridge {
  if (selection.testMode === true && selection.testAdapter) return selection.testAdapter;
  if (
    selection.tauriOriginValidated === true &&
    selection.tauriAdapter?.kind === "tauri"
  ) {
    return selection.tauriAdapter;
  }
  if (selection.browserFixtureAllowed === true) {
    if (selection.browserFixture) return selection.browserFixture;
  }
  return createUnavailableDesktopShellBridge();
}

let registeredTauriAdapter: DesktopShellBridge | null = null;
let registeredTestAdapter: DesktopShellBridge | null = null;

export function registerTauriDesktopShellBridge(
  adapter: DesktopShellBridge,
  origin: string,
  isLocalApplicationOrigin: (candidate: string) => boolean,
): boolean {
  if (adapter.kind !== "tauri" || !isLocalApplicationOrigin(origin)) return false;
  registeredTauriAdapter = adapter;
  return true;
}

/** Test-only bridge injection. The formal Tauri entry never calls this. */
export function registerTestDesktopShellBridge(adapter: DesktopShellBridge): void {
  registeredTestAdapter = adapter;
}

export function clearTestDesktopShellBridge(): void {
  registeredTestAdapter = null;
}

export function getDesktopShellBridge(): DesktopShellBridge {
  if (registeredTauriAdapter) return registeredTauriAdapter;
  if (registeredTestAdapter) return registeredTestAdapter;
  throw new Error("Tauri desktop shell bridge is not registered");
}

export function isTauriDesktopHost(): boolean {
  try {
    return getDesktopShellBridge().kind === "tauri";
  } catch {
    return false;
  }
}
