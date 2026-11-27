import type {
  SourceDescriptor,
  SourceView,
  UnifiedSearchRequest,
  SearchProgress,
  TaskRef,
  TaskRetryFallbackOutcome,
  AccountSessionStatus,
  StoreBatchPage,
  TaskOverview,
  CreateTaskOptions,
} from "../shell/api/types.ts";
import type {
  AgentSessionSummary,
  AgentSessionTruncateResult,
  AgentWorkspace,
  AgentSessionExport,
  AgentSessionExportFormat,
  AgentWorkItem,
  AgentSendContext,
  AgentStreamPage,
  AgentRunSummary,
  AgentRunDetail,
  AgentChannelView,
  AgentChannelSave,
  AgentChannelTest,
  AgentPermissionMode,
  AgentPermissionModeView,
  AgentStopAllResult,
  AgentActivityStatus,
  AgentDisabledItems,
} from "../shell/api/agentBridgeTypes.ts";
import { t } from "../i18n/index.ts";
import { explainMessageKey } from "../shell/api/errorExplanations.ts";
import { parseTaskOverview } from "../shell/api/taskOverviewParser.ts";
import type {
  SearchCancelOutcome,
  TaskConflictFileDeleteResult,
  TaskConflictFileList,
} from "./generated/v3-contracts.ts";
import { parseAccountSessionStatuses } from "./accountSessions.ts";
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { getCurrentWindow as tauriGetCurrentWindow } from "@tauri-apps/api/window";
import { open as tauriOpen } from "@tauri-apps/plugin-dialog";
import { canonicalDemIdentityTimestamp } from "./demIdentityTimestamp.ts";
import { parseOpticalDeliveryAdmission } from "./opticalDeliveryAdmission.ts";
import { isCredentialEditorField, validCredentialEditorValue } from "./credentialEditor.ts";
import { parseOpticalPreparation, type OpticalPreparationResult } from "./opticalPreparation.ts";
import { validSentinel2BandKeys, validSentinel2L1cBandKeys } from "./sentinel2BandSelection.ts";
import type {
  DesktopAppInfo,
  DesktopAdminBoundaryOptionsRequest,
  DesktopAdminBoundaryResolveRequest,
  DesktopAdminBoundarySearchRequest,
  DesktopAoiImportRequest,
  DesktopAoiNormalizeRequest,
  DesktopAppendDownloadTaskRequest,
  DesktopArchiveDownloadTaskRequest,
  DesktopArchiveDownloadTaskResult,
  DesktopBindRegionAoiRequest,
  DesktopCatalogIntentSearchAndFreezeRequest,
  DesktopCatalogSearchAndFreezeRequest,
  DesktopCdseLoginRequest,
  DesktopClearRegionAoiRequest,
  DesktopCdseNetworkDiagnostic,
  DesktopCdseNetworkDiagnosticResult,
  DesktopCdseNetworkMode,
  DesktopCdseNetworkPolicyResult,
  DesktopCdseQuicklook,
  DesktopCdseQuicklookRequest,
  DesktopCdseQuicklookResult,
  DesktopSentinel2MapPreview,
  DesktopSentinel2MapPreviewRequest,
  DesktopSentinel2MapPreviewResult,
  DesktopCdseSessionStatus,
  DesktopCdseSessionStatusResult,
  DesktopCommandResult,
  DesktopCompatibilityBridge,
  DesktopCredentialBridge,
  DesktopCredentialMutationResult,
  DesktopCredentialStatus,
  DesktopCredentialStatusResult,
  DesktopCreateDownloadTaskRequest,
  DesktopCreateTaskFromWorkbenchRequest,
  DesktopAppendTaskFromWorkbenchRequest,
  DesktopAppendTaskFromSourceBatchRequest,
  DesktopBurstAppendAssetTaskRequest,
  DesktopBurstCreateAssetTaskRequest,
  DesktopBurstCreateHistoricalAssetTaskRequest,
  DesktopCreateTaskFromHistoricalSourceBatchRequest,
  DesktopCreateLandsatBundleTaskRequest,
  DesktopCreateLandsatAssetTaskRequest,
  DesktopCreateLandsatAssetBatchTaskRequest,
  DesktopCreateSentinel2BandBatchTaskRequest,
  DesktopCreateHlsBandBatchTaskRequest,
  DesktopLandsatAssetBatchTaskResult,
  DesktopHlsBandBatchTaskResult,
  DesktopTaskDeliveryAdmissionResult,
  DesktopCreateReviewedMpcAssetTaskRequest,
  DesktopCreateSentinel2ArchiveBatchTaskRequest,
  DesktopSentinel2ArchiveBatchTaskResult,
  DesktopCreateSentinel2ArchiveTaskRequest,
  DesktopCreateSentinel2BandTaskRequest,
  DesktopOpticalDerivedCreateTaskRequest,
  DesktopOpticalDerivedPrecheckBatchRequest,
  DesktopOpticalDerivedTaskCreatedResult,
  DesktopOpticalPrecheckReport,
  DesktopOpticalSceneCoverageReport,
  DesktopCreateNisarRemoteSubsetTaskRequest,
  DesktopNisarPrepareSubsetIntentV2Request,
  DesktopNisarPreparedSubsetIntentV2,
  DesktopNisarPreparedSubsetIntentV2Result,
  DesktopNisarPreflightSubsetBatchV2Request,
  DesktopNisarPreflightSubsetBatchV2,
  DesktopNisarPreflightSubsetBatchV2Result,
  DesktopNisarPrepareSubsetBatchV2Request,
  DesktopNisarPreparedSubsetBatchV2,
  DesktopNisarPreparedSubsetBatchV2Result,
  DesktopNisarConfirmSubsetTaskV2Request,
  DesktopNisarConfirmSubsetBatchTaskV2Request,
  DesktopNisarAppendSubsetBatchTaskV2Request,
  DesktopNisarAppendContextV2Request,
  DesktopNisarAppendContextV2,
  DesktopNisarAppendContextV2Result,
  DesktopNisarAppendPreflightV2Request,
  DesktopNisarAppendPreflightV2,
  DesktopNisarAppendPreflightV2Result,
  DesktopNisarSuccessorTaskV2Request,
  DesktopNisarArchivedRecoveryV2Request,
  DesktopSourceMutationCommitStatus,
  DesktopSourceMutationTargetStatus,
  DesktopSourceMutationStatus,
  DesktopSourceMutationStatusResult,
  DesktopSourceMutationQuery,
  DesktopDemConversionRequest,
  DesktopDemConversionSourceRequest,
  DesktopDemConversionSourceResult,
  DesktopDemSourceIdentity,
  DesktopDemSourceIdentityResult,
  DesktopDemTaskSettings,
  DesktopDemTaskPlan,
  DesktopDemTaskPlanRequest,
  DesktopDemTaskPlanResult,
  DesktopDemTaskRequest,
  DesktopStandaloneDemRequest,
  DesktopStandaloneDemTaskRequest,
  DesktopOrbitDirectoryMatchRequest,
  DesktopSceneImportRequest,
  DesktopGacosConfirmRequest,
  DesktopGacosCreateImportTaskRequest,
  DesktopGacosCreateResultTaskRequest,
  DesktopGacosSupplyResultLinkRequest,
  DesktopGacosSupplyResultLinkResult,
  DesktopGacosCreateImportTaskResult,
  DesktopGacosRequestGroup,
  DesktopGacosResultImportRequest,
  DesktopGacosSubmissionResult,
  DesktopGacosSubmissionState,
  DesktopGacosTaskStatus,
  DesktopNetworkSettings,
  DesktopProviderConnectivityResult,
  DesktopDeleteDownloadHistoryRequest,
  DesktopJsonObject,
  DesktopNativeJsonResult,
  DesktopEffectiveCapabilityReason,
  DesktopEffectiveCapabilityState,
  DesktopEffectiveCapabilityResult,
  DesktopEffectiveCapabilityStatus,
  DesktopEarthdataAuthResult,
  DesktopBhoonidhiAssetSize,
  DesktopBhoonidhiAssetSizeRequest,
  DesktopBhoonidhiAssetSizeResult,
  DesktopLandsatAssetDetails,
  DesktopLandsatAssetDetailsRequest,
  DesktopLandsatAssetDetailsResult,
  DesktopLandsatBrowseRequest,
  DesktopLandsatBrowseResult,
  DesktopNisarRemoteSubsetCapability,
  DesktopNisarRemoteSubsetCapabilityResult,
  DesktopOrbitTaskRequest,
  DesktopStandaloneOrbitTaskRequest,
  DesktopApplicationError,
  DesktopApplicationReadiness,
  DesktopBhoonidhiSessionStatus,
  DesktopBhoonidhiSessionStatusResult,
  DesktopCoreRestoreSnapshot,
  DesktopTaskPageRequest,
  DesktopTaskPageSnapshot,
  DesktopTaskSourceBatchPageRequest,
  DesktopTaskSourceBatchPageSnapshot,
  DesktopTaskEventPageRequest,
  DesktopTaskEventPageSnapshot,
  DesktopTaskHistoryPageRequest,
  DesktopTaskHistoryPageSnapshot,
  DesktopOpenTaskOutputRequest,
  DesktopOpenTaskOutputResult,
  DesktopListTaskConflictFilesRequest,
  DesktopDeleteTaskConflictFilesRequest,
  DesktopTaskConflictFileListResult,
  DesktopTaskConflictFileDeleteResult,
  DesktopPickResult,
  DesktopPreviewAssetRequest,
  DesktopPreviewAssetResult,
  DesktopPreviewSaveRequest,
  DesktopPreviewSaveResult,
  DesktopResolvedPreviewAsset,
  DesktopProviderCapabilities,
  DesktopSarBackgroundCountRequest,
  DesktopSarBackgroundCountResult,
  DesktopSarSearchAndFreezeRequest,
  DesktopSarSearchProgressResult,
  DesktopSarSearchProgressStage,
  DesktopSarSearchFailureDiagnostic,
  DesktopSarDownloadPolicySettings,
  DesktopSetWorkingAoiRequest,
  DesktopShellBridge,
  DesktopUpdateDownloadTaskRequest,
  DesktopUpdateDownloadTaskViewRequest,
  DesktopUpdateSearchWorkbenchRequest,
  DesktopWorkbenchPageRequest,
  DesktopWorkbenchPageSnapshot,
  DesktopWorkbenchSelectionCommand,
  DesktopWorkbenchSelectionResult,
  DesktopUsgsM2mLoginRequest,
  DesktopUsgsM2mSessionStatus,
  DesktopUsgsM2mSessionStatusResult,
  DesktopWorkspaceContextResult,
  DesktopWorkspaceCreateRequest,
  DesktopWorkspaceNamedNodeRequest,
  DesktopWorkspaceOpenRequest,
  DesktopWorkspacePrepareLocationRequest,
  DesktopWorkspacePrepareLocationResult,
  DesktopWorkspaceSelectNodeRequest,
  DesktopWorkingAoi,
  DesktopWindowSize,
} from "./desktopShellBridge.ts";

type ResizeDirection =
  | "North"
  | "NorthEast"
  | "East"
  | "SouthEast"
  | "South"
  | "SouthWest"
  | "West"
  | "NorthWest";

type TauriWindow = {
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  close(): Promise<void>;
  innerSize(): Promise<{ width: number; height: number }>;
  startDragging?(): Promise<void>;
  startResizeDragging(direction: ResizeDirection): Promise<void>;
};

export type TauriBridgeDependencies = {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  getCurrentWindow: () => TauriWindow;
  open: (options: Record<string, unknown>) => Promise<unknown>;
};

function unavailableResult(): DesktopCommandResult {
  return {
    ok: false,
    error: "Tauri shell origin is unavailable",
    code: "GUI000",
  };
}

function unavailableSize(): DesktopWindowSize {
  return {
    ok: false,
    error: "Tauri shell origin is unavailable",
    code: "GUI000",
  };
}

function windowOperationFailed(): DesktopWindowSize {
  return {
    ok: false,
    error: "Tauri shell operation failed",
    code: "GUI003",
  };
}

function unavailablePick(): DesktopPickResult {
  return {
    ok: false,
    path: "",
    error: "Tauri shell origin is unavailable",
    code: "GUI000",
  };
}

function operationFailed(): DesktopCommandResult {
  return {
    ok: false,
    error: "Tauri shell operation failed",
    code: "GUI003",
  };
}

function pickFailed(): DesktopPickResult {
  return {
    ok: false,
    path: "",
    error: "Tauri dialog operation failed",
    code: "GUI003",
  };
}

function applicationFailed(reason?: unknown): DesktopApplicationError {
  const value = asRecord(reason);
  const projected: DesktopApplicationError = {
    ok: false,
    error: typeof value?.message_key === "string"
      ? explainMessageKey(value.message_key) ?? value.message_key
      : "Tauri application service operation failed",
    code: typeof value?.code === "string"
      ? value.code
      : "V3_APPLICATION_UNAVAILABLE",
  };
  const context = safeProviderMessage(value?.context, 240) ?? safeDiagnosticScalar(value?.context, 240);
  const diagnostic = parseSarSearchFailureDiagnostic(value?.diagnostic);
  if (context && !diagnostic) projected.context = context;
  if (
    diagnostic
    && (value?.context === undefined || value.context === null || value.context === diagnostic.sanitized_reason)
  ) {
    projected.diagnostic = diagnostic;
  }
  return projected;
}

const ALLOWED_SOURCE_MUTATION_COMMIT_STATUSES = new Set<DesktopSourceMutationCommitStatus>([
  "committed",
  "legacy_committed",
  "event_committed",
  "conflict",
  "unknown",
]);

const ALLOWED_SOURCE_MUTATION_TARGET_STATUSES = new Set<DesktopSourceMutationTargetStatus>([
  "present",
  "tombstoned",
  "missing",
]);

function parseSourceMutationStatus(value: unknown): DesktopSourceMutationStatusResult {
  const record = asRecord(value);
  if (!record || !hasOnlyKeys(record, ["commit", "target"])) {
    return applicationFailed();
  }
  const commit = record.commit as DesktopSourceMutationCommitStatus;
  const target = record.target as DesktopSourceMutationTargetStatus;
  if (
    !ALLOWED_SOURCE_MUTATION_COMMIT_STATUSES.has(commit) ||
    !ALLOWED_SOURCE_MUTATION_TARGET_STATUSES.has(target)
  ) {
    return applicationFailed();
  }
  return {
    ok: true,
    status: {
      commit,
      target,
    },
  };
}

function nisarSubsetRequestFailed(): DesktopApplicationError {
  return {
    ok: false,
    error: t("shared.desktopBridge.invalidNisarSubsetBatch"),
    code: "NISAR_SUBSET_V2_REQUEST_INVALID",
  };
}

function hasOnlyKeys(record: Record<string, unknown>, keys: readonly string[]): boolean {
  const allowed = new Set(keys);
  return Object.keys(record).every((key) => allowed.has(key))
    && keys.every((key) => Object.prototype.hasOwnProperty.call(record, key));
}

function safeDiagnosticScalar(value: unknown, maxLength = 80): string | null {
  if (typeof value !== "string" || value.length < 1 || value.length > maxLength || value.trim() !== value) {
    return null;
  }
  if (
    /password|token|secret|authorization|bearer|https?:\/\//i.test(value)
    || !/^[A-Za-z0-9\s_\-.,:;()+\=><]+$/.test(value)
  ) {
    return null;
  }
  return value;
}

function safeProviderMessage(value: unknown, maxLength = 240): string | null {
  if (typeof value !== "string" || value.length < 1 || value.length > maxLength || value.trim() !== value) {
    return null;
  }
  if (
    /password|token|secret|authorization|bearer|https?:\/\//i.test(value)
    || /applicationservice|desktop api|worker|tauri|rust|sidecar|v3_|gui|does not expose|not implemented|task summary|task-view|snapshot|internal|ownership/i.test(value)
    || /[A-Za-z]:[\\/]/.test(value)
    || /[\\/]/.test(value)
    || !/^[\p{L}\p{N}\s_.,:;()+=><\-，。；：！？、]+$/u.test(value)
  ) {
    return null;
  }
  return value;
}

function diagnosticStringArray(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.length > 16) return null;
  const parsed = value.map((item) => safeDiagnosticScalar(item));
  return parsed.every((item): item is string => item !== null) ? parsed : null;
}

function nullableDiagnosticString(value: unknown, maxLength = 80): string | null | undefined {
  if (value === null) return null;
  return safeDiagnosticScalar(value, maxLength) ?? undefined;
}

function nullableDiagnosticInteger(value: unknown): number | null | undefined {
  if (value === null) return null;
  return typeof value === "number" && Number.isSafeInteger(value) ? value : undefined;
}

function parseSarSearchRangeDiagnostic(value: unknown): { start: string | null; end: string | null } | null {
  const record = asRecord(value);
  if (!record || !hasOnlyKeys(record, ["start", "end"])) return null;
  const start = nullableDiagnosticString(record.start, 48);
  const end = nullableDiagnosticString(record.end, 48);
  return start !== undefined && end !== undefined ? { start, end } : null;
}

function parseSarSearchFailureDiagnostic(value: unknown): DesktopSarSearchFailureDiagnostic | null {
  const record = asRecord(value);
  const queryShape = asRecord(record?.query_shape);
  const dateRange = parseSarSearchRangeDiagnostic(record?.date_range);
  const filters = asRecord(record?.filter_state);
  const queryShapeKeys = [
    "platform_count",
    "processing_level_count",
    "product_family_count",
    "beam_mode_count",
    "polarization_count",
    "has_instrument",
    "has_relative_orbit",
    "has_frame",
    "has_asf_frame",
    "has_path",
    "has_track_range",
    "has_frame_range",
    "has_aoi_geojson",
    "has_aoi_wkt",
    "has_bbox",
    "has_burst",
  ] as const;
  const filterKeys = [
    "platforms",
    "instrument",
    "processing_levels",
    "product_families",
    "beam_modes",
    "polarizations",
    "flight_direction",
    "look_direction",
    "relative_orbit",
    "frame",
    "asf_frame",
    "path",
    "track_range",
    "frame_range",
  ] as const;
  const diagnosticKeys = [
    "operation",
    "route",
    "sanitized_reason",
    "dataset",
    "query_shape",
    "aoi_state",
    "date_range",
    "filter_state",
    "max_results",
  ] as const;
  if (
    !record
    || !queryShape
    || !filters
    || !dateRange
    || !hasOnlyKeys(record, diagnosticKeys)
    || !hasOnlyKeys(queryShape, queryShapeKeys)
    || !hasOnlyKeys(filters, filterKeys)
    || record.operation !== "sar_search_and_freeze"
    || record.route !== "legacy_asf_search"
    || !["none", "geojson", "bbox", "wkt"].includes(String(record.aoi_state))
  ) {
    return null;
  }
  const sanitizedReason = safeDiagnosticScalar(record.sanitized_reason, 240);
  const dataset = safeDiagnosticScalar(record.dataset);
  const counts = queryShapeKeys.slice(0, 5).map((key) => queryShape[key]);
  const booleans = queryShapeKeys.slice(5).map((key) => queryShape[key]);
  const platforms = diagnosticStringArray(filters.platforms);
  const processingLevels = diagnosticStringArray(filters.processing_levels);
  const productFamilies = diagnosticStringArray(filters.product_families);
  const beamModes = diagnosticStringArray(filters.beam_modes);
  const polarizations = diagnosticStringArray(filters.polarizations);
  const instrument = nullableDiagnosticString(filters.instrument);
  const flightDirection = nullableDiagnosticString(filters.flight_direction);
  const lookDirection = nullableDiagnosticString(filters.look_direction);
  const asfFrame = nullableDiagnosticString(filters.asf_frame);
  const relativeOrbit = nullableDiagnosticInteger(filters.relative_orbit);
  const frame = nullableDiagnosticInteger(filters.frame);
  const path = nullableDiagnosticInteger(filters.path);
  const trackRange = filters.track_range === null ? null : parseSarSearchRangeDiagnostic(filters.track_range);
  const frameRange = filters.frame_range === null ? null : parseSarSearchRangeDiagnostic(filters.frame_range);
  const maxResults = record.max_results === null
    ? null
    : typeof record.max_results === "number"
      && Number.isSafeInteger(record.max_results)
      && record.max_results >= 1
      && record.max_results <= 10_000
      ? record.max_results
      : undefined;
  if (
    !sanitizedReason
    || !dataset
    || counts.some((item) => typeof item !== "number" || !Number.isSafeInteger(item) || item < 0 || item > 16)
    || booleans.some((item) => typeof item !== "boolean")
    || !platforms
    || !processingLevels
    || !productFamilies
    || !beamModes
    || !polarizations
    || instrument === undefined
    || flightDirection === undefined
    || lookDirection === undefined
    || asfFrame === undefined
    || relativeOrbit === undefined
    || frame === undefined
    || path === undefined
    || (filters.track_range !== null && !trackRange)
    || (filters.frame_range !== null && !frameRange)
    || maxResults === undefined
  ) {
    return null;
  }
  return {
    operation: "sar_search_and_freeze",
    route: "legacy_asf_search",
    sanitized_reason: sanitizedReason,
    dataset,
    query_shape: queryShape as DesktopSarSearchFailureDiagnostic["query_shape"],
    aoi_state: record.aoi_state as DesktopSarSearchFailureDiagnostic["aoi_state"],
    date_range: dateRange,
    filter_state: {
      platforms,
      instrument,
      processing_levels: processingLevels,
      product_families: productFamilies,
      beam_modes: beamModes,
      polarizations,
      flight_direction: flightDirection,
      look_direction: lookDirection,
      relative_orbit: relativeOrbit,
      frame,
      asf_frame: asfFrame,
      path,
      track_range: trackRange,
      frame_range: frameRange,
    },
    max_results: maxResults,
  };
}

function parseCredentialStatus(value: unknown): DesktopCredentialStatus | null {
  const record = asRecord(value);
  const input = asRecord(record?.earthdata_input);
  const mode = input?.mode;
  if (
    !record ||
    record.ok !== true ||
    typeof record.cdse !== "string" ||
    typeof record.usgs !== "string" ||
    typeof record.earthdata !== "string" ||
    !input ||
    !["none", "token", "login", "netrc"].includes(mode as string) ||
    typeof input.token !== "string" ||
    typeof input.username !== "string" ||
    typeof input.password !== "string" ||
    typeof record.bhoonidhi !== "string" ||
    typeof record.opentopography !== "string" ||
    typeof record.gacos !== "string"
  ) {
    return null;
  }
  return {
    cdse: record.cdse,
    usgs: record.usgs,
    earthdata: record.earthdata,
    earthdata_input: {
      mode: mode as DesktopCredentialStatus["earthdata_input"]["mode"],
      token: input.token,
      username: input.username,
      password: input.password,
    },
    bhoonidhi: record.bhoonidhi,
    opentopography: record.opentopography,
    gacos: record.gacos,
  };
}

function parseCredentialMutation(value: unknown): DesktopCredentialMutationResult {
  const record = asRecord(value);
  // DesktopCredentialMutation nests status; credential_status returns it directly.
  const status = parseCredentialStatus(record?.status);
  if (!record || record.ok !== true || !status || typeof record.removed !== "boolean") {
    return {
      ok: false,
      error: t("shared.desktopBridge.credentialStatusUnparseable"),
      code: "CREDENTIAL_RESPONSE_INVALID",
    };
  }
  return { ok: true, status, removed: record.removed };
}

function parseEarthdataAuth(value: unknown): DesktopEarthdataAuthResult {
  const record = asRecord(value);
  const statuses = ["missing", "valid", "expired", "invalid", "unknown", "unavailable"];
  if (
    !record ||
    record.ok !== true ||
    typeof record.configured !== "boolean" ||
    typeof record.status !== "string" ||
    !statuses.includes(record.status) ||
    typeof record.message !== "string"
  ) {
    return applicationFailed();
  }
  return {
    ok: true,
    configured: record.configured,
    status: record.status as Exclude<DesktopEarthdataAuthResult, DesktopApplicationError>['status'],
    message: record.message,
  };
}

type ApplicationCommand =
  | "application_list_sources"
  | "application_source_views"
  | "application_search_start"
  | "application_search_progress"
  | "application_search_cancel"
  | "application_batch_page"
  | "application_create_task"
  | "application_preview_all_match_task"
  | "application_create_all_match_task"
  | "application_task_action"
  | "application_task_overview"
  | "application_account_session_statuses"
  | "application_test_network_connection"
  | "application_probe_provider_connectivity"
  | "application_core_restore"
  | "application_workbench_page"
  | "application_download_task_page"
  | "application_load_task_source_batch_page"
  | "application_download_history_page"
  | "application_download_task_event_page"
  | "application_open_download_task_output"
  | "application_open_external_url"
  | "application_update_workbench_selection"
  | "application_workspace_open"
  | "application_project_create"
  | "application_project_select"
  | "application_region_create"
  | "application_region_select"
  | "application_plan_dem_task"
  | "application_create_dem_task"
  | "application_resolve_dem_conversion_source"
  | "application_handoff_dem_conversion"
  | "application_get_dem_task_settings"
  | "application_save_dem_task_settings"
  | "application_standalone_dem"
  | "application_create_standalone_dem_task"
  | "application_import_scenes"
  | "application_match_orbits_directory"
  | "application_gacos_ledger"
  | "application_gacos_import_result"
  | "application_gacos_create_import_task"
  | "application_gacos_submit_request"
  | "application_gacos_request_status"
  | "application_get_network_settings"
  | "application_save_network_settings"
  | "application_create_orbit_task"
  | "application_create_standalone_orbit_task"
  | "application_create_companion_orbit_task"
  | "application_bind_region_aoi"
  | "application_clear_region_aoi"
  | "application_provider_capabilities"
  | "application_sar_search_and_freeze"
  | "application_sar_background_count"
  | "application_catalog_search_and_freeze"
  | "application_catalog_intent_search_and_freeze"
  | "application_cancel_request"
  | "application_cancel_catalog_search"
  | "application_cdse_session_status"
  | "application_cdse_login"
  | "application_cdse_logout"
  | "application_cdse_network_policy"
  | "application_set_cdse_network_policy"
  | "application_cdse_network_diagnostic"
  | "application_preview_asset"
  | "application_save_preview_asset"
  | "application_cdse_quicklook"
  | "application_sentinel2_map_preview"
  | "application_usgs_session_status"
  | "application_usgs_login"
  | "application_usgs_logout"
  | "application_landsat_browse"
  | "application_landsat_asset_details"
  | "application_bhoonidhi_asset_size"
  | "application_create_landsat_bundle_task"
  | "application_create_landsat_asset_task"
  | "application_create_reviewed_mpc_asset_task"
  | "application_create_sentinel2_archive_batch_task"
  | "application_create_sentinel2_archive_task"
  | "application_create_sentinel2_band_task"
  | "application_precheck_optical_aoi_coverage"
  | "application_create_optical_derived_task"
  | "application_effective_capability"
  | "application_nisar_remote_subset_capability"
  | "application_create_nisar_remote_subset_task"
  | "application_prepare_nisar_subset_intent_v2"
  | "application_prepare_nisar_subset_batch_v2"
  | "application_confirm_nisar_subset_task_v2"
  | "application_confirm_nisar_subset_batch_task_v2"
  | "application_append_nisar_subset_batch_task_v2"
  | "application_create_nisar_subset_successor_task_v2"
  | "application_read_nisar_archived_recovery_context_v2"
  | "application_create_nisar_archived_recovery_v2"
  | "application_resume_nisar_subset_task"
  | "application_start_newly_confirmed_task"
  | "application_update_search_workbench"
  | "application_create_download_task"
  | "application_append_download_task"
  | "application_create_task_from_workbench"
  | "application_append_task_from_workbench"
  | "application_append_task_from_source_batch"
  | "application_create_task_from_historical_source_batch"
  | "application_create_burst_asset_task"
  | "application_append_burst_asset_task"
  | "application_create_historical_burst_asset_task"
  | "application_update_download_task"
  | "application_update_download_task_view"
  | "application_archive_download_task"
  | "application_delete_download_history"
  | "application_task_retry_fallback"
  | "application_task_item_action"
  | "application_task_delete_history"
  | "application_task_set_pinned"
  | "application_list_pinned_tasks"
  | "application_task_retry_fallback_eligibility"
  | "check_existing_downloaded_scenes"
  | "application_open_geodata_query"
  | "geodata_fetch_preview_data_url"

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function nonNegativeSafeInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function parseCoreSnapshotValue(raw: unknown): DesktopCoreRestoreSnapshot {
  const value = asRecord(raw);
  if (value?.ok === false) return applicationFailed(value);
  const readiness = asRecord(value?.readiness);
  const core = asRecord(value?.core);
  const aoiDisplay = value?.aoi_display === undefined
    ? undefined
    : value.aoi_display === null
      ? null
      : asRecord(value.aoi_display);
  if (
    !value ||
    !readiness ||
    !core ||
    typeof readiness.contract !== "string" ||
    typeof readiness.status !== "string" ||
    (readiness.store_schema_version !== null && typeof readiness.store_schema_version !== "number") ||
    Array.isArray(core.candidates) ||
    value.aoi_display !== undefined && value.aoi_display !== null && !aoiDisplay
  ) {
    return applicationFailed();
  }
  return {
    ok: true,
    readiness: {
      contract: readiness.contract,
      status: readiness.status,
      store_schema_version: readiness.store_schema_version as number | null,
    },
    core,
    ...(aoiDisplay === undefined ? {} : { aoi_display: aoiDisplay }),
  };
}

function parseOpticalAssetBatchTaskResult(rawValue: unknown, itemIds: readonly string[]): DesktopLandsatAssetBatchTaskResult {
  const raw = asRecord(rawValue);
  if (raw?.ok === false) return applicationFailed(raw);
  if (!raw || !hasOnlyKeys(raw, ["snapshot", "admission"])) return applicationFailed();
  const admission = parseOpticalDeliveryAdmission(raw.admission);
  const explicitIds = itemIds.length > 0;
  if (!admission
    || (explicitIds && admission.requested_count !== itemIds.length)
    || (!explicitIds && (admission.requested_count < 1 || admission.requested_count > MAX_NISAR_SUBSET_BATCH_ITEMS))
    || (explicitIds && admission.rejections.some(row => !itemIds.includes(row.source_item_id)))) return applicationFailed();
  if (raw.snapshot === null) return admission.admitted_count === 0
    ? { ok: true, snapshot: null, admission } : applicationFailed();
  const snapshot = parseCoreSnapshotValue(raw.snapshot);
  return snapshot.ok && admission.admitted_count > 0 ? { ok: true, snapshot, admission } : applicationFailed();
}

function parseSentinel2ArchiveBatchTaskResult(
  raw: unknown,
  request: DesktopCreateSentinel2ArchiveBatchTaskRequest,
): DesktopSentinel2ArchiveBatchTaskResult {
  const result = parseOpticalAssetBatchTaskResult(raw, request.item_ids);
  if (!result.ok || result.snapshot === null) return result;
  if (!result.snapshot.ok) return applicationFailed();
  const tasks = result.snapshot.core.tasks;
  const matches = Array.isArray(tasks) ? tasks.map(asRecord).filter(task => task?.task_id === request.task_id) : [];
  const task = matches[0];
  if (matches.length !== 1 || !task || task.batch_id !== `sentinel2-archive-batch-${request.task_id}`
    || task.provider_id !== "copernicus.dataspace.odata" || task.dataset_id !== "optical.sentinel-2.l2a"
    || task.selected_count !== result.admission.admitted_count
    || task.delivery_requested_count !== result.admission.requested_count
    || task.delivery_rejected_count !== result.admission.rejected_count) return applicationFailed();
  return result;
}

function parseOpticalSceneCoverageReport(rawValue: unknown): DesktopOpticalSceneCoverageReport | null {
  const rec = asRecord(rawValue);
  if (
    !rec
    || typeof rec.scene_id !== "string"
    || !rec.scene_id.trim()
    || typeof rec.intersects_aoi !== "boolean"
    || typeof rec.aoi_coverage_fraction !== "number"
    || !Number.isFinite(rec.aoi_coverage_fraction)
  ) {
    return null;
  }
  const sceneEpsg =
    rec.scene_epsg === undefined || rec.scene_epsg === null
      ? null
      : typeof rec.scene_epsg === "number" && Number.isInteger(rec.scene_epsg)
      ? rec.scene_epsg
      : undefined;
  const acquiredAt =
    rec.acquired_at === undefined || rec.acquired_at === null
      ? null
      : typeof rec.acquired_at === "string"
      ? rec.acquired_at
      : undefined;
  const cloudCoverPct =
    rec.cloud_cover_pct === undefined || rec.cloud_cover_pct === null
      ? null
      : typeof rec.cloud_cover_pct === "number" && Number.isFinite(rec.cloud_cover_pct)
      ? rec.cloud_cover_pct
      : undefined;
  if (sceneEpsg === undefined || acquiredAt === undefined || cloudCoverPct === undefined) {
    return null;
  }
  return {
    scene_id: rec.scene_id,
    intersects_aoi: rec.intersects_aoi,
    aoi_coverage_fraction: rec.aoi_coverage_fraction,
    scene_epsg: sceneEpsg,
    acquired_at: acquiredAt,
    cloud_cover_pct: cloudCoverPct,
  };
}

function parseOpticalPrecheckReport(rawValue: unknown): DesktopOpticalPrecheckReport | null {
  const rec = asRecord(rawValue);
  if (!rec || rec.ok === false && typeof rec.code === "string" && typeof rec.message_key === "string") {
    return null;
  }
  if (
    !rec
    || typeof rec.ok !== "boolean"
    || (rec.recommended_mode !== "single_scene_clip"
      && rec.recommended_mode !== "batch_single_scene_clip"
      && rec.recommended_mode !== "near_coincident_spatial_mosaic")
    || !Array.isArray(rec.aoi_bbox_wgs84)
    || rec.aoi_bbox_wgs84.length !== 4
    || !rec.aoi_bbox_wgs84.every((n) => typeof n === "number" && Number.isFinite(n))
    || !Array.isArray(rec.aoi_center_wgs84)
    || rec.aoi_center_wgs84.length !== 2
    || !rec.aoi_center_wgs84.every((n) => typeof n === "number" && Number.isFinite(n))
    || typeof rec.recommended_utm_epsg !== "number"
    || !Number.isInteger(rec.recommended_utm_epsg)
    || typeof rec.crosses_utm_zones !== "boolean"
    || !Array.isArray(rec.distinct_utm_zones)
    || !rec.distinct_utm_zones.every((n) => typeof n === "number" && Number.isInteger(n))
    || typeof rec.scene_count !== "number"
    || !Number.isSafeInteger(rec.scene_count)
    || typeof rec.intersecting_scene_count !== "number"
    || !Number.isSafeInteger(rec.intersecting_scene_count)
    || typeof rec.combined_aoi_coverage_fraction !== "number"
    || !Number.isFinite(rec.combined_aoi_coverage_fraction)
    || typeof rec.within_near_coincident_limit !== "boolean"
    || typeof rec.estimated_output_pixels_per_band !== "number"
    || !Number.isSafeInteger(rec.estimated_output_pixels_per_band)
    || !Array.isArray(rec.warnings)
    || !rec.warnings.every((item) => typeof item === "string")
    || !Array.isArray(rec.blocking_reasons)
    || !rec.blocking_reasons.every((item) => typeof item === "string")
    || !Array.isArray(rec.scene_coverages)
  ) {
    return null;
  }
  const temporalSpan =
    rec.temporal_span_days === undefined || rec.temporal_span_days === null
      ? null
      : typeof rec.temporal_span_days === "number" && Number.isFinite(rec.temporal_span_days)
      ? rec.temporal_span_days
      : undefined;
  if (temporalSpan === undefined) return null;
  const coverages: DesktopOpticalSceneCoverageReport[] = [];
  for (const rawCov of rec.scene_coverages) {
    const cov = parseOpticalSceneCoverageReport(rawCov);
    if (!cov) return null;
    coverages.push(cov);
  }
  return {
    ok: rec.ok,
    recommended_mode: rec.recommended_mode,
    aoi_bbox_wgs84: [
      rec.aoi_bbox_wgs84[0] as number,
      rec.aoi_bbox_wgs84[1] as number,
      rec.aoi_bbox_wgs84[2] as number,
      rec.aoi_bbox_wgs84[3] as number,
    ],
    aoi_center_wgs84: [rec.aoi_center_wgs84[0] as number, rec.aoi_center_wgs84[1] as number],
    recommended_utm_epsg: rec.recommended_utm_epsg,
    crosses_utm_zones: rec.crosses_utm_zones,
    distinct_utm_zones: rec.distinct_utm_zones as number[],
    scene_count: rec.scene_count,
    intersecting_scene_count: rec.intersecting_scene_count,
    combined_aoi_coverage_fraction: rec.combined_aoi_coverage_fraction,
    temporal_span_days: temporalSpan,
    within_near_coincident_limit: rec.within_near_coincident_limit,
    estimated_output_pixels_per_band: rec.estimated_output_pixels_per_band,
    warnings: rec.warnings as string[],
    blocking_reasons: rec.blocking_reasons as string[],
    scene_coverages: coverages,
  };
}

function parseOpticalDerivedTaskCreatedResult(
  rawValue: unknown,
  request: DesktopOpticalDerivedCreateTaskRequest,
): DesktopOpticalDerivedTaskCreatedResult {
  const raw = asRecord(rawValue);
  if (raw?.ok === false) return applicationFailed(raw);
  if (
    !raw
    || typeof raw.task_id !== "string"
    || raw.task_id !== request.task_id
    || typeof raw.derived_batch_id !== "string"
    || raw.derived_batch_id !== `optderived-${request.task_id}`
    || typeof raw.derived_item_id !== "string"
    || !raw.derived_item_id.trim()
    || typeof raw.output_tif_path !== "string"
    || !raw.output_tif_path.trim()
  ) {
    return applicationFailed();
  }
  const precheck = parseOpticalPrecheckReport(raw.precheck);
  const snapshot = parseCoreSnapshotValue(raw.snapshot);
  if (!precheck || !snapshot.ok) {
    return applicationFailed();
  }
  const outputTifPaths = Array.isArray(raw.output_tif_paths)
    ? raw.output_tif_paths.filter((p): p is string => typeof p === "string" && p.trim().length > 0)
    : undefined;
  return {
    ok: true,
    precheck,
    task_id: raw.task_id,
    derived_batch_id: raw.derived_batch_id,
    derived_item_id: raw.derived_item_id,
    output_tif_path: raw.output_tif_path,
    output_tif_paths: outputTifPaths,
    snapshot,
  };
}

const MIN_HISTORY_CURSOR_EPOCH_MS = -62_167_219_200_000;
const MAX_HISTORY_CURSOR_EPOCH_MS = 253_402_300_799_999;

function validTaskHistoryCursorEpoch(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) &&
    value >= MIN_HISTORY_CURSOR_EPOCH_MS && value <= MAX_HISTORY_CURSOR_EPOCH_MS;
}

function validTaskHistoryTaskId(value: unknown): value is string {
  return validBoundedDesktopScalar(value, 128);
}

function validGacosAttemptId(value: unknown): value is string {
  // insar-store::gacos_submission::canonical_attempt_id uses 256 ASCII bytes.
  // A normal task:item:attempt identity is already 159 bytes; it is not a task ID.
  return typeof value === "string" && value.length >= 1 && value.length <= 256
    && !/[^A-Za-z0-9_.:-]/.test(value);
}

function validDemIdentityCode(value: unknown): value is string {
  return validTaskHistoryTaskId(value) && value === value.toUpperCase() && /^[A-Z0-9_.-]+$/.test(value);
}

function validBoundedDesktopScalar(value: unknown, maxLength: number): value is string {
  if (typeof value !== "string") return false;
  const length = Array.from(value).length;
  return length >= 1 && length <= maxLength && value.trim() === value &&
    !/[\u0000-\u001f\u007f-\u009f]/.test(value);
}

const GACOS_SUBMISSION_STATES: readonly DesktopGacosSubmissionState[] = [
  "reserved",
  "submitting",
  "submitted",
  "manual_verification_required",
  "submission_uncertain",
  "rejected",
];

const GACOS_TASK_STATUSES: readonly DesktopGacosTaskStatus[] = [
  "queued",
  "running",
  "paused",
  "completed",
  "failed",
  "cancelled",
  "interrupted",
];

function gacosSubmissionResponseFailed(): DesktopApplicationError {
  return {
    ok: false,
    error: t("shared.desktopBridge.gacosRecordUnparseable"),
    code: "GACOS_RESPONSE_INVALID",
  };
}

function parseGacosSubmissionResult(value: unknown): DesktopGacosSubmissionResult {
  const record = asRecord(value);
  if (
    !record
    || !hasOnlyKeys(record, [
      "ok", "task_id", "records", "request_groups", "task_status", "manual_action_required", "automatic_retry_allowed", "message",
      ...(Object.prototype.hasOwnProperty.call(record, "submitted_count") ? ["submitted_count"] : []),
      ...(Object.prototype.hasOwnProperty.call(record, "application_completed") ? ["application_completed"] : []),
    ])
    || record.ok !== true
    || !validTaskHistoryTaskId(record.task_id)
    || !Array.isArray(record.records)
    || !Array.isArray(record.request_groups)
    || typeof record.task_status !== "string"
    || !GACOS_TASK_STATUSES.includes(record.task_status as DesktopGacosTaskStatus)
    || typeof record.manual_action_required !== "boolean"
    || record.automatic_retry_allowed !== false
    || !validBoundedDesktopScalar(record.message, 512)
  ) {
    return gacosSubmissionResponseFailed();
  }
  const requestGroups = record.request_groups.map((value) => {
    const group = asRecord(value);
    const dates = Array.isArray(group?.dates)
      ? group.dates.map((date) => validBoundedDesktopScalar(date, 32) ? date : null)
      : null;
    const hour = typeof group?.hour === "number" && Number.isSafeInteger(group.hour) ? group.hour : null;
    const minute = typeof group?.minute === "number" && Number.isSafeInteger(group.minute) ? group.minute : null;
    const bbox = Array.isArray(group?.bbox) && group.bbox.length === 4
      ? group.bbox.map((coordinate) => typeof coordinate === "number" && Number.isFinite(coordinate) ? coordinate : null)
      : null;
    if (
      !group
      || !hasOnlyKeys(group, ["item_id", "dates", "hour", "minute", "bbox"])
      || !validTaskHistoryTaskId(group.item_id)
      || !dates
      || dates.length === 0
      || !dates.every((date): date is string => date !== null)
      || hour === null
      || hour < 0
      || hour > 23
      || minute === null
      || minute < 0
      || minute > 59
      || !bbox
      || !bbox.every((coordinate): coordinate is number => coordinate !== null)
    ) {
      return null;
    }
    return {
      item_id: group.item_id,
      dates,
      hour,
      minute,
      bbox: bbox as [number, number, number, number],
    } satisfies DesktopGacosRequestGroup;
  });
  if (!requestGroups.every((item): item is DesktopGacosRequestGroup => item !== null)) {
    return gacosSubmissionResponseFailed();
  }
  const records = record.records.map((value) => {
    const item = asRecord(value);
    const attemptNumber = nonNegativeSafeInteger(item?.attempt_number);
    const httpStatus = item?.http_status === null
      ? null
      : nonNegativeSafeInteger(item?.http_status);
    const reasonCode = item?.reason_code === null
      ? null
      : validBoundedDesktopScalar(item?.reason_code, 128)
        ? item.reason_code
        : undefined;
    if (
      !item
      || !hasOnlyKeys(item, [
        "attempt_id", "task_id", "item_id", "attempt_number", "state", "reason_code",
        "http_status", "automatic_retry_allowed", "created_at", "updated_at",
      ])
      || !validGacosAttemptId(item.attempt_id)
      || !validTaskHistoryTaskId(item.task_id)
      || item.task_id !== record.task_id
      || !validTaskHistoryTaskId(item.item_id)
      || attemptNumber === null
      || typeof item.state !== "string"
      || !GACOS_SUBMISSION_STATES.includes(item.state as DesktopGacosSubmissionState)
      || reasonCode === undefined
      || httpStatus === undefined
      || (httpStatus !== null && (httpStatus < 100 || httpStatus > 599))
      || item.automatic_retry_allowed !== false
      || !validBoundedDesktopScalar(item.created_at, 64)
      || !validBoundedDesktopScalar(item.updated_at, 64)
    ) {
      return null;
    }
    return {
      attempt_id: item.attempt_id,
      task_id: item.task_id,
      item_id: item.item_id,
      attempt_number: attemptNumber,
      state: item.state as DesktopGacosSubmissionState,
      reason_code: reasonCode,
      http_status: httpStatus,
      automatic_retry_allowed: false as const,
      created_at: item.created_at,
      updated_at: item.updated_at,
    };
  });
  if (!records.every((item): item is Exclude<typeof records[number], null> => item !== null)) {
    return gacosSubmissionResponseFailed();
  }
  // Older native responses lacked the application summary and never emitted
  // Submitted. Their unconfirmed state must remain readable after an update.
  const submittedCount = record.submitted_count === undefined ? 0 : nonNegativeSafeInteger(record.submitted_count);
  const applicationCompleted = record.application_completed === undefined ? false : record.application_completed;
  if (
    submittedCount === null || submittedCount === undefined || submittedCount > requestGroups.length
    || typeof applicationCompleted !== "boolean"
    || (applicationCompleted && (submittedCount === 0 || submittedCount !== requestGroups.length || record.manual_action_required))
    || (records.some((item) => item.state === "submitted") && record.submitted_count === undefined)
  ) return gacosSubmissionResponseFailed();
  return {
    ok: true,
    task_id: record.task_id,
    records,
    request_groups: requestGroups,
    task_status: record.task_status as DesktopGacosTaskStatus,
    submitted_count: submittedCount,
    application_completed: applicationCompleted,
    manual_action_required: record.manual_action_required,
    automatic_retry_allowed: false,
    message: record.message,
  };
}

function validGacosResultLink(value: unknown): value is string {
  if (!validBoundedDesktopScalar(value, 4096)) return false;
  try {
    const url = new URL(value);
    return ["http:", "https:"].includes(url.protocol)
      && ["gacos.net", "www.gacos.net"].includes(url.hostname)
      && !url.username && !url.password && !url.port && !url.hash
      && url.pathname !== "/";
  } catch { return false; }
}

function parseGacosCreateImportTaskResult(value: unknown, source = "native-gacos-local-import"): DesktopGacosCreateImportTaskResult {
  const record = asRecord(value);
  if (
    !record
    || !hasOnlyKeys(record, ["ok", "task_id", "output_root", "status", "source"])
    || record.ok !== true
    || !validTaskHistoryTaskId(record.task_id)
    || !validBoundedDesktopScalar(record.output_root, 4096)
    || !absoluteNisarOutputRoot(record.output_root)
    || !validBoundedDesktopScalar(record.status, 64)
    || record.source !== source
  ) {
    return applicationFailed();
  }
  return {
    ok: true,
    task_id: record.task_id,
    output_root: record.output_root,
    status: record.status,
    source: record.source,
  };
}

function parseDemConversionSource(value: unknown): DesktopDemConversionSourceResult {
  if (value === null) return { ok: true, source: null };
  const record = asRecord(value);
  if (
    !record ||
    Object.keys(record).length !== 9 ||
    !validTaskHistoryTaskId(record.task_id) ||
    nonNegativeSafeInteger(record.revision) === null ||
    !validTaskHistoryTaskId(record.region_id) ||
    !validTaskHistoryTaskId(record.dataset) ||
    !validTaskHistoryTaskId(record.provider) ||
    !validTaskHistoryTaskId(record.resource_id) ||
    !validTaskHistoryTaskId(record.capability_id) ||
    !validBoundedDesktopScalar(record.input_path, 4096) ||
    !validBoundedDesktopScalar(record.output_root, 4096)
  ) {
    return applicationFailed();
  }
  return {
    ok: true,
    source: {
      task_id: record.task_id,
      revision: record.revision as number,
      region_id: record.region_id,
      dataset: record.dataset,
      provider: record.provider,
      resource_id: record.resource_id,
      capability_id: record.capability_id,
      input_path: record.input_path,
      output_root: record.output_root,
    },
  };
}

function validTaskHistoryTimestamp(value: unknown): value is string {
  if (typeof value !== "string" || value.length < 20 || value.length > 30) return false;
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?Z$/.exec(value);
  if (!match) return false;
  const [year, month, day, hour, minute, second] = match.slice(1, 7).map(Number);
  if (month < 1 || month > 12 || day < 1 || hour > 23 || minute > 59 || second > 59) return false;
  const date = new Date(0);
  date.setUTCHours(0, 0, 0, 0);
  date.setUTCFullYear(year, month - 1, day);
  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day;
}

function parseDemSourceIdentity(value: unknown): DesktopDemSourceIdentity | null {
  const record = asRecord(value);
  const updatedAt = canonicalDemIdentityTimestamp(record?.updated_at);
  if (!record || Object.keys(record).length !== 3 ||
      !validDemIdentityCode(record.dataset) || !validDemIdentityCode(record.provider) ||
      updatedAt === null || updatedAt !== record.updated_at) {
    return null;
  }
  return {
    dataset: record.dataset,
    provider: record.provider,
    updated_at: updatedAt,
  };
}

function parseNisarRemoteSubsetCapability(
  value: unknown,
): DesktopNisarRemoteSubsetCapability | null {
  const record = asRecord(value);
  const polarizations = asRecord(record?.supported_polarizations);
  if (
    !record ||
    record.feature_id !== "nisar_remote_subset_v1" ||
    typeof record.visible !== "boolean" ||
    typeof record.enabled !== "boolean" ||
    (record.implementation_scope !== "offline_only" &&
      record.implementation_scope !== "live_range_writer_store_v1") ||
    typeof record.offline_verified !== "boolean" ||
    typeof record.live_verified !== "boolean" ||
    typeof record.production_enabled !== "boolean" ||
    record.default_enabled !== false ||
    record.output_authority !== "client_derived" ||
    record.supported_product_type !== "RSLC" ||
    !Array.isArray(record.supported_specification_versions) ||
    !record.supported_specification_versions.every((item) => typeof item === "string") ||
    !Array.isArray(record.supported_frequencies) ||
    !record.supported_frequencies.every((item) => item === "A" || item === "B") ||
    !polarizations ||
    ![polarizations.A, polarizations.B].every(
      (items) => Array.isArray(items) && items.every((item) => typeof item === "string"),
    ) ||
    (record.reason_code !== null && typeof record.reason_code !== "string")
  ) {
    return null;
  }
  return record as DesktopNisarRemoteSubsetCapability;
}

function parseNisarPreparedSubsetIntentV2(
  value: unknown,
): DesktopNisarPreparedSubsetIntentV2 | null {
  const record = asRecord(value);
  if (
    !record
    || typeof record.source_batch_id !== "string"
    || typeof record.source_item_id !== "string"
    || typeof record.workbench_revision !== "number"
    || !Number.isSafeInteger(record.workbench_revision)
    || record.workbench_revision < 0
    || !asRecord(record.intent)
  ) return null;
  return record as unknown as DesktopNisarPreparedSubsetIntentV2;
}

function parseNisarPreparedSubsetIntentV2Result(value: unknown): DesktopNisarPreparedSubsetIntentV2Result {
  const prepared = parseNisarPreparedSubsetIntentV2(value);
  return prepared ? { ok: true, prepared } : applicationFailed();
}

const NISAR_BATCH_JSON_LIMIT_BYTES = 2 * 1024 * 1024;
const MAX_NISAR_SUBSET_BATCH_ITEMS = 10_000;
const NISAR_FORBIDDEN_TRANSPORT_FIELD = /(?:url|secret|credential|token|authorization|password|cookie|href|locator)/i;

function hasForbiddenTransportField(value: unknown, seen = new Set<object>()): boolean {
  if (Array.isArray(value)) {
    if (seen.has(value)) return false;
    seen.add(value);
    return value.some((item) => hasForbiddenTransportField(item, seen));
  }
  const record = asRecord(value);
  if (!record) return false;
  if (seen.has(record)) return false;
  seen.add(record);
  return Object.entries(record).some(([key, nested]) => (
    NISAR_FORBIDDEN_TRANSPORT_FIELD.test(key) || hasForbiddenTransportField(nested, seen)
  ));
}

function boundedSafeNisarObject(value: unknown): value is DesktopJsonObject {
  const record = asRecord(value);
  if (!record || hasForbiddenTransportField(record)) return false;
  try {
    const serialized = JSON.stringify(record);
    if (serialized === undefined) return false;
    return new TextEncoder().encode(serialized).byteLength <= NISAR_BATCH_JSON_LIMIT_BYTES;
  } catch {
    return false;
  }
}

function validNisarBatchIds(ids: unknown): ids is string[] {
  if (!Array.isArray(ids) || ids.length < 1 || ids.length > MAX_NISAR_SUBSET_BATCH_ITEMS) return false;
  return ids.every((id) => typeof id === "string" && id.trim().length > 0)
    && new Set(ids).size === ids.length;
}

function validOpticalBatchIds(ids: unknown): ids is string[] {
  if (!Array.isArray(ids) || ids.length > MAX_NISAR_SUBSET_BATCH_ITEMS) return false;
  return ids.every((id) => typeof id === "string" && id.trim().length > 0)
    && new Set(ids).size === ids.length;
}

function validNisarRevision(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function parseNisarSelections(
  value: unknown,
  allowEmpty = false,
): Array<{ frequency: "A" | "B"; polarizations: string[] }> | null {
  if (!Array.isArray(value) || (!allowEmpty && value.length < 1) || value.length > 2) return null;
  const frequencies = new Set<string>();
  const selections: Array<{ frequency: "A" | "B"; polarizations: string[] }> = [];
  for (const raw of value) {
    const selection = asRecord(raw);
    if (
      !selection
      || !hasOnlyKeys(selection, ["frequency", "polarizations"])
      || (selection.frequency !== "A" && selection.frequency !== "B")
      || frequencies.has(selection.frequency)
      || !Array.isArray(selection.polarizations)
      || selection.polarizations.length < 1
      || selection.polarizations.some((item) => typeof item !== "string" || item.trim().length === 0)
      || new Set(selection.polarizations).size !== selection.polarizations.length
    ) return null;
    frequencies.add(selection.frequency);
    selections.push({ frequency: selection.frequency, polarizations: [...selection.polarizations] as string[] });
  }
  return selections;
}

function parseNisarPreflightSubsetBatchV2(
  value: unknown,
  request: DesktopNisarPreflightSubsetBatchV2Request,
): DesktopNisarPreflightSubsetBatchV2 | null {
  const batch = asRecord(value);
  if (
    !batch
    || !hasOnlyKeys(batch, ["source_batch_id", "workbench_revision", "eligible_items", "excluded_items", "common_selections"])
    || batch.source_batch_id !== request.source_batch_id
    || batch.workbench_revision !== request.expected_workbench_revision
    || !validNisarRevision(batch.workbench_revision)
    || !Array.isArray(batch.eligible_items)
    || batch.eligible_items.length < 1
    || batch.eligible_items.length > MAX_NISAR_SUBSET_BATCH_ITEMS
    || !Array.isArray(batch.excluded_items)
    || batch.excluded_items.length > MAX_NISAR_SUBSET_BATCH_ITEMS
  ) return null;
  const commonSelections = parseNisarSelections(batch.common_selections, true);
  if (!commonSelections) return null;

  const expectedIds = new Set(request.source_item_ids);
  const covered = new Set<string>();
  const eligibleItems: DesktopNisarPreflightSubsetBatchV2["eligible_items"] = [];
  for (const raw of batch.eligible_items) {
    const item = asRecord(raw);
    const availableSelections = item ? parseNisarSelections(item.available_selections) : null;
    if (
      !item
      || !hasOnlyKeys(item, ["source_item_id", "effective_geometry", "available_selections"])
      || typeof item.source_item_id !== "string"
      || !expectedIds.has(item.source_item_id)
      || covered.has(item.source_item_id)
      || !boundedSafeNisarObject(item.effective_geometry)
      || !availableSelections
    ) return null;
    covered.add(item.source_item_id);
    eligibleItems.push({
      source_item_id: item.source_item_id,
      effective_geometry: item.effective_geometry,
      available_selections: availableSelections,
    });
  }
  const excludedItems: DesktopNisarPreflightSubsetBatchV2["excluded_items"] = [];
  for (const raw of batch.excluded_items) {
    const item = asRecord(raw);
    if (
      !item
      || !hasOnlyKeys(item, ["source_item_id", "reason"])
      || typeof item.source_item_id !== "string"
      || !expectedIds.has(item.source_item_id)
      || covered.has(item.source_item_id)
      || item.reason !== "no_overlap"
    ) return null;
    covered.add(item.source_item_id);
    excludedItems.push({ source_item_id: item.source_item_id, reason: "no_overlap" });
  }
  if (covered.size !== expectedIds.size) return null;
  return {
    source_batch_id: batch.source_batch_id,
    workbench_revision: batch.workbench_revision,
    eligible_items: eligibleItems,
    excluded_items: excludedItems,
    common_selections: commonSelections,
  };
}

function parseNisarPreparedSubsetBatchV2(
  value: unknown,
  request: DesktopNisarPrepareSubsetBatchV2Request,
): DesktopNisarPreparedSubsetBatchV2 | null {
  const batch = asRecord(value);
  if (
    !batch
    || !hasOnlyKeys(batch, ["source_batch_id", "workbench_revision", "intents", "excluded_items"])
    || batch.source_batch_id !== request.source_batch_id
    || batch.workbench_revision !== request.expected_workbench_revision
    || !validNisarRevision(batch.workbench_revision)
    || !Array.isArray(batch.intents)
    || batch.intents.length < 1
    || batch.intents.length > MAX_NISAR_SUBSET_BATCH_ITEMS
    || !Array.isArray(batch.excluded_items)
    || batch.excluded_items.length > MAX_NISAR_SUBSET_BATCH_ITEMS
  ) return null;

  const expectedIds = new Set(request.source_item_ids);
  const covered = new Set<string>();
  const intents: DesktopNisarPreparedSubsetIntentV2[] = [];
  for (const raw of batch.intents) {
    const item = asRecord(raw);
    if (
      !item
      || !hasOnlyKeys(item, ["source_batch_id", "source_item_id", "workbench_revision", "intent"])
      || item.source_batch_id !== request.source_batch_id
      || typeof item.source_item_id !== "string"
      || !expectedIds.has(item.source_item_id)
      || covered.has(item.source_item_id)
      || item.workbench_revision !== request.expected_workbench_revision
      || !validNisarRevision(item.workbench_revision)
      || !boundedSafeNisarObject(item.intent)
    ) return null;
    covered.add(item.source_item_id);
    intents.push(item as unknown as DesktopNisarPreparedSubsetIntentV2);
  }

  const excludedItems: Array<{ source_item_id: string; reason: "no_overlap" }> = [];
  for (const raw of batch.excluded_items) {
    const item = asRecord(raw);
    if (
      !item
      || !hasOnlyKeys(item, ["source_item_id", "reason"])
      || typeof item.source_item_id !== "string"
      || !expectedIds.has(item.source_item_id)
      || covered.has(item.source_item_id)
      || item.reason !== "no_overlap"
    ) return null;
    covered.add(item.source_item_id);
    excludedItems.push(item as { source_item_id: string; reason: "no_overlap" });
  }
  if (covered.size !== expectedIds.size) return null;
  return {
    source_batch_id: batch.source_batch_id,
    workbench_revision: batch.workbench_revision,
    intents,
    excluded_items: excludedItems,
  };
}

function validNisarPreflightBatchRequest(request: DesktopNisarPreflightSubsetBatchV2Request): boolean {
  const record = asRecord(request);
  return Boolean(
    record
    && hasOnlyKeys(record, [
      "source_batch_id",
      "source_item_ids",
      "expected_workbench_revision",
      "subset_draft_geometry",
    ])
    && typeof record.source_batch_id === "string"
    && record.source_batch_id.trim().length > 0
    && validNisarBatchIds(record.source_item_ids)
    && validNisarRevision(record.expected_workbench_revision)
    && boundedSafeNisarObject(record.subset_draft_geometry)
    && !hasForbiddenTransportField(record)
  );
}

function validNisarPrepareBatchRequest(request: DesktopNisarPrepareSubsetBatchV2Request): boolean {
  const record = asRecord(request);
  return Boolean(
    record
    && hasOnlyKeys(record, [
      "source_batch_id",
      "source_item_ids",
      "expected_workbench_revision",
      "subset_draft_geometry",
      "selected_frequency",
      "selected_polarizations",
    ])
    && typeof record.source_batch_id === "string"
    && record.source_batch_id.trim().length > 0
    && validNisarBatchIds(record.source_item_ids)
    && validNisarRevision(record.expected_workbench_revision)
    && boundedSafeNisarObject(record.subset_draft_geometry)
    && (record.selected_frequency === "A" || record.selected_frequency === "B")
    && Array.isArray(record.selected_polarizations)
    && record.selected_polarizations.length > 0
    && record.selected_polarizations.every((item) => typeof item === "string" && item.trim().length > 0)
    && !hasForbiddenTransportField(record)
  );
}

function absoluteNisarOutputRoot(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0
    && /^(?:[A-Za-z]:[\\/]|\\\\|\/)/.test(value.trim());
}

function validNisarCreatedAt(value: unknown): value is string {
  return typeof value === "string"
    && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z$/.test(value)
    && Number.isFinite(Date.parse(value));
}

function validNisarConfirmBatchRequest(request: DesktopNisarConfirmSubsetBatchTaskV2Request): boolean {
  const record = asRecord(request);
  if (
    !record
    || !hasOnlyKeys(record, [
      "source_batch_id",
      "expected_workbench_revision",
      "items",
      "task_id",
      "output_root",
      "layout",
      "concurrency",
      "created_at",
    ])
    || typeof record.source_batch_id !== "string"
    || record.source_batch_id.trim().length === 0
    || !validNisarRevision(record.expected_workbench_revision)
    || !Array.isArray(record.items)
    || record.items.length < 1
    || record.items.length > MAX_NISAR_SUBSET_BATCH_ITEMS
    || !record.items.every((raw) => {
      const item = asRecord(raw);
      return Boolean(
        item
        && hasOnlyKeys(item, ["source_item_id", "prepared_intent"])
        && typeof item.source_item_id === "string"
        && item.source_item_id.trim().length > 0
        && boundedSafeNisarObject(item.prepared_intent),
      );
    })
    || new Set(record.items.map((raw) => (raw as Record<string, unknown>).source_item_id)).size !== record.items.length
    || typeof record.task_id !== "string"
    || record.task_id.trim().length === 0
    || !absoluteNisarOutputRoot(record.output_root)
    || !["direct", "sentinel1_slc", "sentinel_orbit_aux_poeorb", "sar_dataset_processing"].includes(record.layout as string)
    || typeof record.concurrency !== "number"
    || !Number.isSafeInteger(record.concurrency)
    || record.concurrency < 1
    || record.concurrency > 8
    || !validNisarCreatedAt(record.created_at)
  ) return false;
  return !hasForbiddenTransportField(record);
}

function validNisarAppendBatchRequest(request: DesktopNisarAppendSubsetBatchTaskV2Request): boolean {
  const record = asRecord(request);
  return Boolean(
    record
    && hasOnlyKeys(record, [
      "task_id",
      "expected_task_revision",
      "source_item_ids",
      "event_id",
      "occurred_at",
    ])
    && typeof record.task_id === "string"
    && record.task_id.trim().length > 0
    && validNisarRevision(record.expected_task_revision)
    && validNisarBatchIds(record.source_item_ids)
    && typeof record.event_id === "string"
    && record.event_id.trim().length > 0
    && validNisarCreatedAt(record.occurred_at)
    && !hasForbiddenTransportField(record)
  );
}

function parseNisarAppendContext(value: unknown, taskId: string): DesktopNisarAppendContextV2 | null {
  const context = asRecord(value);
  if (!context || !hasOnlyKeys(context, ["task_id", "source_batch_id", "task_revision", "task_status", "frozen_aoi_geojson", "selected_frequency", "selected_polarizations", "output_root", "layout", "concurrency", ...(context.original_item_ids === undefined ? [] : ["original_item_ids"])])
    || context.task_id !== taskId || typeof context.source_batch_id !== "string" || !context.source_batch_id.trim()
    || !validNisarRevision(context.task_revision)
    || !["queued", "running", "paused", "completed", "failed", "cancelled"].includes(String(context.task_status))
    || !boundedSafeNisarObject(context.frozen_aoi_geojson)
    || !parseNisarSelections([{ frequency: context.selected_frequency, polarizations: context.selected_polarizations }])
    || !absoluteNisarOutputRoot(context.output_root)
    || !["direct", "sar_dataset_processing"].includes(String(context.layout))
    || !Number.isSafeInteger(context.concurrency) || Number(context.concurrency) < 1 || Number(context.concurrency) > 8
    || (context.original_item_ids !== undefined && !validNisarBatchIds(context.original_item_ids))
    || hasForbiddenTransportField(context)) return null;
  return context as DesktopNisarAppendContextV2;
}

function parseNisarAppendPreflight(value: unknown, request: DesktopNisarAppendPreflightV2Request): DesktopNisarAppendPreflightV2 | null {
  const response = asRecord(value);
  if (!response || !hasOnlyKeys(response, ["task_id", "source_batch_id", "task_revision", "eligible_items", "excluded_items", "common_selections"])
    || response.task_id !== request.task_id || response.task_revision !== request.expected_task_revision
    || typeof response.source_batch_id !== "string" || !response.source_batch_id.trim()
    || !Array.isArray(response.eligible_items) || !Array.isArray(response.excluded_items)
    || response.eligible_items.length + response.excluded_items.length !== request.source_item_ids.length
    || hasForbiddenTransportField(response)) return null;
  const common = parseNisarSelections(response.common_selections, true);
  if (!common) return null;
  const requested = new Set(request.source_item_ids);
  const covered = new Set<string>();
  for (const raw of response.eligible_items) {
    const item = asRecord(raw);
    if (!item || !hasOnlyKeys(item, ["source_item_id", "effective_geometry", "available_selections"])
      || typeof item.source_item_id !== "string" || !requested.has(item.source_item_id) || covered.has(item.source_item_id)
      || !boundedSafeNisarObject(item.effective_geometry) || !parseNisarSelections(item.available_selections)) return null;
    covered.add(item.source_item_id);
  }
  for (const raw of response.excluded_items) {
    const item = asRecord(raw);
    if (!item || !hasOnlyKeys(item, ["source_item_id", "reason"])
      || typeof item.source_item_id !== "string" || !requested.has(item.source_item_id) || covered.has(item.source_item_id)
      || !["no_overlap", "frequency_incompatible", "polarization_incompatible"].includes(String(item.reason))) return null;
    covered.add(item.source_item_id);
  }
  return covered.size === requested.size ? response as DesktopNisarAppendPreflightV2 : null;
}

function parseNisarPreflightSubsetBatchV2Result(
  value: unknown,
  request: DesktopNisarPreflightSubsetBatchV2Request,
): DesktopNisarPreflightSubsetBatchV2Result {
  const preflight = parseNisarPreflightSubsetBatchV2(value, request);
  return preflight ? { ok: true, preflight } : applicationFailed();
}

function parseNisarPreparedSubsetBatchV2Result(
  value: unknown,
  request: DesktopNisarPrepareSubsetBatchV2Request,
): DesktopNisarPreparedSubsetBatchV2Result {
  const prepared = parseNisarPreparedSubsetBatchV2(value, request);
  return prepared ? { ok: true, prepared } : applicationFailed();
}

function parseWorkspaceContext(value: unknown): DesktopWorkspaceContextResult {
  const context = asRecord(value);
  const projects = context && Array.isArray(context.projects)
    ? context.projects.map(asRecord)
    : [];
  const hierarchyValid = projects.every((project) => {
    if (
      !project
      || project.workspace_id !== context?.workspace_id
      || typeof project.project_id !== "string"
      || typeof project.project_name !== "string"
      || typeof project.safe_name !== "string"
      || !Array.isArray(project.regions)
    ) return false;
    return project.regions.every((value) => {
      const region = asRecord(value);
      return Boolean(
        region
        && region.project_id === project.project_id
        && typeof region.region_id === "string"
        && typeof region.region_name === "string"
        && typeof region.safe_name === "string",
      );
    });
  });
  if (
    !context ||
    typeof context.schema_version !== "number" ||
    !Number.isSafeInteger(context.schema_version) ||
    context.schema_version < 1 ||
    typeof context.workspace_id !== "string" ||
    typeof context.workspace_root !== "string" ||
    typeof context.workspace_name !== "string" ||
    typeof context.revision !== "number" ||
    !Number.isSafeInteger(context.revision) ||
    context.revision < 0 ||
    typeof context.updated_at !== "string" ||
    !Array.isArray(context.projects) ||
    !Array.isArray(context.regions) ||
    !hierarchyValid
  ) {
    return applicationFailed();
  }
  return { ok: true, context };
}

function parseWorkspacePreparedLocation(value: unknown): DesktopWorkspacePrepareLocationResult {
  const record = asRecord(value);
  if (!record || typeof record.path !== "string" || record.path.trim() === "") {
    return applicationFailed();
  }
  return { ok: true, path: record.path };
}

function parseArchiveDownloadTaskAck(
  value: unknown,
  request: DesktopArchiveDownloadTaskRequest,
): DesktopArchiveDownloadTaskResult {
  const record = asRecord(value);
  if (!record || Object.keys(record).length !== 3) return applicationFailed();
  const keys = new Set(Object.keys(record));
  if (!keys.has("task_id") || !keys.has("revision") || !keys.has("archived_at")) {
    return applicationFailed();
  }
  if (
    record.task_id !== request.task_id ||
    record.revision !== request.expected_revision ||
    typeof record.task_id !== "string" ||
    !validTaskHistoryTaskId(record.task_id) ||
    typeof record.revision !== "number" ||
    !Number.isSafeInteger(record.revision) ||
    record.revision < 0 ||
    typeof record.archived_at !== "string" ||
    !validTaskHistoryTimestamp(record.archived_at) ||
    Date.parse(record.archived_at) !== Date.parse(request.archived_at)
  ) {
    return applicationFailed();
  }
  return {
    ok: true,
    archive: {
      task_id: record.task_id,
      revision: record.revision,
      archived_at: record.archived_at,
    },
  };
}

function parseDemTaskPlan(value: unknown): DesktopDemTaskPlan | null {
  const record = asRecord(value);
  if (
    !record ||
    typeof record.request_id !== "string" ||
    typeof record.task_id !== "string" ||
    typeof record.region_id !== "string" ||
    typeof record.resource_id !== "string" ||
    typeof record.capability_id !== "string" ||
    typeof record.dataset !== "string" ||
    typeof record.provider !== "string" ||
    !["inspect", "prepare_sarscape", "convert"].includes(record.operation as string) ||
    typeof record.input_path !== "string" ||
    typeof record.output_root !== "string" ||
    typeof record.created_at !== "string"
  ) {
    return null;
  }
  return {
    request_id: record.request_id,
    task_id: record.task_id,
    region_id: record.region_id,
    resource_id: record.resource_id,
    capability_id: record.capability_id,
    dataset: record.dataset,
    provider: record.provider,
    operation: record.operation as DesktopDemTaskPlan["operation"],
    input_path: record.input_path,
    output_root: record.output_root,
    source_vertical_datum:
      typeof record.source_vertical_datum === "string" ? record.source_vertical_datum : null,
    target_vertical_datum:
      typeof record.target_vertical_datum === "string" ? record.target_vertical_datum : null,
    output_mode: typeof record.output_mode === "string" ? record.output_mode : null,
    created_at: record.created_at,
  };
}

function parseEffectiveCapabilityStatus(
  value: unknown,
  expectedCapabilityId: string,
): DesktopEffectiveCapabilityStatus | null {
  const record = asRecord(value);
  const capability = asRecord(record?.capability);
  const inputs = asRecord(record?.inputs);
  const projection = asRecord(record?.projection);
  const states: DesktopEffectiveCapabilityState[] = [
    "executable",
    "blocked_build",
    "blocked_runtime",
    "blocked_credential",
    "blocked_admission",
    "blocked_policy",
    "reserved",
    "retired",
  ];
  const reasons: DesktopEffectiveCapabilityReason[] = [
    "ready",
    "static_reserved",
    "static_retired",
    "build_unavailable_or_unknown",
    "runtime_not_ready_or_unknown",
    "credential_missing_or_unknown",
    "admission_missing_rejected_or_unknown",
    "policy_denied_or_unknown",
  ];
  const isOptional = (candidate: unknown, values: readonly string[]) =>
    candidate === null || (typeof candidate === "string" && values.includes(candidate));
  if (
    !record ||
    typeof record.resource_id !== "string" ||
    !capability ||
    capability.capability_id !== expectedCapabilityId ||
    typeof capability.kind !== "string" ||
    typeof capability.lifecycle !== "string" ||
    !inputs ||
    !isOptional(inputs.build, ["available", "unavailable"]) ||
    !isOptional(inputs.runtime, ["ready", "not_ready"]) ||
    !isOptional(inputs.credential, ["ready", "missing"]) ||
    !isOptional(inputs.admission, ["accepted", "rejected"]) ||
    !isOptional(inputs.policy, ["allowed", "denied"]) ||
    !projection ||
    typeof projection.state !== "string" ||
    !states.includes(projection.state as DesktopEffectiveCapabilityState) ||
    typeof projection.reason !== "string" ||
    !reasons.includes(projection.reason as DesktopEffectiveCapabilityReason)
  ) {
    return null;
  }
  return {
    resource_id: record.resource_id,
    capability: capability as DesktopEffectiveCapabilityStatus["capability"],
    inputs: {
      build: inputs.build as DesktopEffectiveCapabilityStatus["inputs"]["build"],
      runtime: inputs.runtime as DesktopEffectiveCapabilityStatus["inputs"]["runtime"],
      credential: inputs.credential as DesktopEffectiveCapabilityStatus["inputs"]["credential"],
      admission: inputs.admission as DesktopEffectiveCapabilityStatus["inputs"]["admission"],
      policy: inputs.policy as DesktopEffectiveCapabilityStatus["inputs"]["policy"],
    },
    projection: {
      state: projection.state as DesktopEffectiveCapabilityState,
      reason: projection.reason as DesktopEffectiveCapabilityReason,
    },
  };
}

function parseCdseNetworkMode(value: unknown): DesktopCdseNetworkMode | null {
  return value === "system" || value === "direct" ? value : null;
}

function parseCdseSessionStatus(value: unknown): DesktopCdseSessionStatus | null {
  const record = asRecord(value);
  if (!record) return null;
  const networkMode = parseCdseNetworkMode(record.network_mode);
  const expiresInSeconds = record.expires_in_seconds;
  if (
    !networkMode ||
    typeof record.authenticated !== "boolean" ||
    typeof record.refresh_available !== "boolean" ||
    (expiresInSeconds !== null &&
      (!Number.isSafeInteger(expiresInSeconds) || Number(expiresInSeconds) < 0))
  ) {
    return null;
  }
  return {
    network_mode: networkMode,
    authenticated: record.authenticated,
    refresh_available: record.refresh_available,
    expires_in_seconds: expiresInSeconds as number | null,
  };
}

function parseUsgsSessionStatus(value: unknown): DesktopUsgsM2mSessionStatus | null {
  const record = asRecord(value);
  if (!record || typeof record.authenticated !== "boolean") return null;
  const states = [
    "ready",
    "logging_in",
    "authenticated",
    "login_failed",
    "logging_out",
    "logged_out",
  ] as const;
  if (!states.includes(record.state as typeof states[number])) return null;
  if (record.authenticated !== (record.state === "authenticated")) return null;
  return {
    state: record.state as DesktopUsgsM2mSessionStatus["state"],
    authenticated: record.authenticated,
  };
}

function parseBhoonidhiSessionStatus(value: unknown): DesktopBhoonidhiSessionStatus | null {
  const record = asRecord(value);
  const expiresInSeconds = record?.expires_in_seconds;
  if (
    !record ||
    typeof record.authenticated !== "boolean" ||
    typeof record.refresh_available !== "boolean" ||
    (expiresInSeconds !== null &&
      (!Number.isSafeInteger(expiresInSeconds) || Number(expiresInSeconds) < 0))
  ) {
    return null;
  }
  return {
    authenticated: record.authenticated,
    refresh_available: record.refresh_available,
    expires_in_seconds: expiresInSeconds as number | null,
  };
}

function parseCdseNetworkDiagnostic(value: unknown): DesktopCdseNetworkDiagnostic | null {
  const record = asRecord(value);
  const networkMode = parseCdseNetworkMode(record?.network_mode);
  const stage = record?.stage;
  if (!networkMode || typeof stage !== "string") return null;
  if (![
    "dns_unavailable",
    "tcp_unavailable",
    "tls_handshake_failed",
    "proxy_configuration_rejected",
    "timeout",
    "http_reached",
    "transport_unavailable",
  ].includes(stage)) {
    return null;
  }
  return {
    network_mode: networkMode,
    stage: stage as DesktopCdseNetworkDiagnostic["stage"],
  };
}

const MAX_CDSE_QUICKLOOK_BYTES = 16 * 1024 * 1024;
const MAX_CDSE_QUICKLOOK_BASE64_CHARS = Math.ceil(MAX_CDSE_QUICKLOOK_BYTES / 3) * 4;

function standardBase64Value(code: number): number {
  if (code >= 65 && code <= 90) return code - 65;
  if (code >= 97 && code <= 122) return code - 97 + 26;
  if (code >= 48 && code <= 57) return code - 48 + 52;
  if (code === 43) return 62;
  if (code === 47) return 63;
  return -1;
}

function parseCdseQuicklook(value: unknown): DesktopCdseQuicklook | null {
  const record = asRecord(value);
  if (!record) return null;
  const mediaType = record.media_type;
  const dataUrl = record.data_url;
  const byteLength = record.byte_length;
  if (
    (mediaType !== "image/jpeg" && mediaType !== "image/png" && mediaType !== "image/webp") ||
    typeof dataUrl !== "string" ||
    typeof byteLength !== "number" ||
    !Number.isSafeInteger(byteLength) ||
    byteLength <= 0 ||
    byteLength > MAX_CDSE_QUICKLOOK_BYTES
  ) {
    return null;
  }

  const prefix = `data:${mediaType};base64,`;
  if (!dataUrl.startsWith(prefix)) return null;
  const encoded = dataUrl.slice(prefix.length);
  if (
    encoded.length === 0 ||
    encoded.length > MAX_CDSE_QUICKLOOK_BASE64_CHARS ||
    encoded.length % 4 !== 0
  ) {
    return null;
  }

  const padding = encoded.endsWith("==") ? 2 : encoded.endsWith("=") ? 1 : 0;
  const contentLength = encoded.length - padding;
  for (let index = 0; index < contentLength; index += 1) {
    if (standardBase64Value(encoded.charCodeAt(index)) < 0) return null;
  }
  for (let index = contentLength; index < encoded.length; index += 1) {
    if (encoded.charCodeAt(index) !== 61) return null;
  }
  if (
    (padding === 1 && standardBase64Value(encoded.charCodeAt(contentLength - 1)) % 4 !== 0) ||
    (padding === 2 && standardBase64Value(encoded.charCodeAt(contentLength - 1)) % 16 !== 0)
  ) {
    return null;
  }

  const decodedLength = (encoded.length / 4) * 3 - padding;
  if (decodedLength !== byteLength) return null;
  return {
    media_type: mediaType,
    data_url: dataUrl,
    byte_length: byteLength,
  };
}

function parseSentinel2MapPreview(value: unknown): DesktopSentinel2MapPreview | null {
  const record = asRecord(value);
  if (
    !record
    || !hasOnlyKeys(record, ["product_key", "map_item_id"])
    || !validBoundedDesktopScalar(record.product_key, 512)
    || !validBoundedDesktopScalar(record.map_item_id, 200)
    || !/^S2[ABC]_MSIL2A_\d{8}T\d{6}_R\d{3}_T\d{2}[A-Z]{3}_\d{8}T\d{6}$/.test(record.map_item_id)
  ) {
    return null;
  }
  return {
    product_key: record.product_key,
    map_item_id: record.map_item_id,
  };
}

function parseBhoonidhiAssetSize(value: unknown): DesktopBhoonidhiAssetSize | null {
  const record = asRecord(value);
  const sizeBytes = nonNegativeSafeInteger(record?.size_bytes);
  if (
    !record
    || !hasOnlyKeys(record, ["product_key", "size_bytes"])
    || !validBoundedDesktopScalar(record.product_key, 1024)
    || sizeBytes === null
    || sizeBytes <= 0
  ) {
    return null;
  }
  return { product_key: record.product_key, size_bytes: sizeBytes };
}

function parseLandsatAssetDetails(value: unknown): DesktopLandsatAssetDetails | null {
  const record = asRecord(value);
  if (
    !record
    || !hasOnlyKeys(record, ["product_key", "display_id", "processing_level", "total_size_bytes", "assets"])
    || !validBoundedDesktopScalar(record.product_key, 512)
    || !validBoundedDesktopScalar(record.display_id, 128)
    || !validBoundedDesktopScalar(record.processing_level, 16)
    || !Array.isArray(record.assets)
    || record.assets.length === 0
    || record.assets.length > 64
  ) {
    return null;
  }
  const totalSizeBytes = nonNegativeSafeInteger(record.total_size_bytes);
  if (totalSizeBytes === null || totalSizeBytes <= 0) return null;
  const assets = record.assets.map((value) => {
    const asset = asRecord(value);
    const sizeBytes = nonNegativeSafeInteger(asset?.size_bytes);
    if (
      !asset
      || !hasOnlyKeys(asset, ["asset_key", "file_name", "role", "size_bytes"])
      || !validBoundedDesktopScalar(asset.asset_key, 96)
      || !validBoundedDesktopScalar(asset.file_name, 240)
      || !["primary", "auxiliary", "metadata"].includes(String(asset.role))
      || sizeBytes === null
      || sizeBytes <= 0
    ) {
      return null;
    }
    return {
      asset_key: asset.asset_key,
      file_name: asset.file_name,
      role: asset.role as "primary" | "auxiliary" | "metadata",
      size_bytes: sizeBytes,
    };
  });
  if (!assets.every((asset): asset is NonNullable<typeof asset> => asset !== null)) return null;
  if (assets.reduce((sum, asset) => sum + asset.size_bytes, 0) !== totalSizeBytes) return null;
  return {
    product_key: record.product_key,
    display_id: record.display_id,
    processing_level: record.processing_level,
    total_size_bytes: totalSizeBytes,
    assets,
  };
}

function parsePreviewSaveResult(value: unknown): DesktopPreviewSaveResult {
  const record = asRecord(value);
  const bytesWritten = nonNegativeSafeInteger(record?.bytes_written);
  if (
    !record
    || !hasOnlyKeys(record, ["path", "file_name", "bytes_written"])
    || !absoluteNisarOutputRoot(record.path)
    || !validBoundedDesktopScalar(record.file_name, 240)
    || bytesWritten === null
    || bytesWritten <= 0
  ) {
    return applicationFailed();
  }
  return {
    ok: true,
    path: record.path,
    file_name: record.file_name,
    bytes_written: bytesWritten,
  };
}

function parsePreviewAsset(value: unknown): DesktopResolvedPreviewAsset | null {
  const record = asRecord(value);
  const descriptor = asRecord(record?.descriptor);
  const image = parseCdseQuicklook(value);
  const productKey = record?.product_key;
  const sourceAssetRole = record?.source_asset_role;
  const sourceAssetKey = record?.source_asset_key;
  const sourceAssetKeys = descriptor?.source_asset_keys;
  if (
    !record ||
    !descriptor ||
    !image ||
    typeof productKey !== "string" ||
    !productKey.trim() ||
    typeof sourceAssetKey !== "string" ||
    !sourceAssetKey.trim() ||
    !["preview", "browse", "thumbnail"].includes(String(sourceAssetRole)) ||
    !Array.isArray(sourceAssetKeys) ||
    sourceAssetKeys.length === 0 ||
    !sourceAssetKeys.every((key) => typeof key === "string" && Boolean(key.trim())) ||
    !sourceAssetKeys.includes(sourceAssetKey) ||
    ![
      "quicklook",
      "thumbnail",
      "single_asset_image",
      "rgb_asset_composite",
      "map_tile_overlay",
      "local_generated_preview",
    ].includes(String(descriptor.preview_kind)) ||
    typeof descriptor.render_profile_id !== "string" ||
    !descriptor.render_profile_id.trim() ||
    descriptor.media_type !== image.media_type ||
    !["none", "provider_credentials"].includes(String(descriptor.credential_requirement)) ||
    !["no_store", "session", "persistent_public"].includes(String(descriptor.cache_policy)) ||
    typeof descriptor.provenance !== "string" ||
    !descriptor.provenance.trim()
  ) {
    return null;
  }
  return {
    product_key: productKey,
    descriptor: {
      preview_kind: descriptor.preview_kind as DesktopResolvedPreviewAsset["descriptor"]["preview_kind"],
      source_asset_keys: [...sourceAssetKeys] as string[],
      render_profile_id: descriptor.render_profile_id,
      media_type: image.media_type,
      credential_requirement: descriptor.credential_requirement as DesktopResolvedPreviewAsset["descriptor"]["credential_requirement"],
      cache_policy: descriptor.cache_policy as DesktopResolvedPreviewAsset["descriptor"]["cache_policy"],
      provenance: descriptor.provenance,
    },
    source_asset_role: sourceAssetRole as DesktopResolvedPreviewAsset["source_asset_role"],
    source_asset_key: sourceAssetKey,
    media_type: image.media_type,
    data_url: image.data_url,
    byte_length: image.byte_length,
  };
}

function resizeDirection(edge: string): ResizeDirection | null {
  const normalized = edge.trim().toLowerCase();
  const values: Record<string, ResizeDirection> = {
    n: "North",
    ne: "NorthEast",
    e: "East",
    se: "SouthEast",
    s: "South",
    sw: "SouthWest",
    w: "West",
    nw: "NorthWest",
    north: "North",
    northeast: "NorthEast",
    east: "East",
    southeast: "SouthEast",
    south: "South",
    southwest: "SouthWest",
    west: "West",
    northwest: "NorthWest",
    top: "North",
    "top-right": "NorthEast",
    right: "East",
    "bottom-right": "SouthEast",
    bottom: "South",
    "bottom-left": "SouthWest",
    left: "West",
    "top-left": "NorthWest",
  };
  return values[normalized] ?? null;
}

function selectedPath(value: unknown): string | null | undefined {
  if (value === null) return null;
  if (typeof value === "string") return value;
  if (Array.isArray(value) && value.every((item) => typeof item === "string")) {
    if (value.length === 0) return null;
    if (value.length === 1) return value[0] as string;
  }
  return undefined;
}

export type TauriDialogFilter = {
  name: string;
  extensions: string[];
};

/**
 * Translate the existing UI filter vocabulary into Tauri's extension-only
 * filter shape. Returning null means the caller must reject the request
 * without opening a broad, unfiltered native dialog.
 */
export function parseDialogFilters(filters?: string[]): TauriDialogFilter[] | null | undefined {
  if (!filters || filters.length === 0) return undefined;
  const translated: TauriDialogFilter[] = [];
  for (const filter of filters) {
    if (typeof filter !== "string") return null;
    const match = /^\s*([^()]+?)\s*\(([^()]*)\)\s*$/.exec(filter);
    if (!match) return null;
    const name = match[1].trim();
    const patterns = match[2]
      .split(";")
      .map((pattern) => pattern.trim())
      .filter(Boolean);
    if (!name || patterns.length === 0) return null;

    const extensions: string[] = [];
    for (const pattern of patterns) {
      if (pattern === "*.*") {
        extensions.push("*");
        continue;
      }
      const extension = pattern.startsWith("*.")
        ? pattern.slice(2)
        : pattern.startsWith(".")
          ? pattern.slice(1)
          : "";
      if (!extension || !/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(extension)) return null;
      extensions.push(extension);
    }
    translated.push({ name, extensions });
  }
  return translated;
}

function defaultDependencies(): TauriBridgeDependencies {
  return {
    invoke: (command, args) => tauriInvoke(command, args),
    getCurrentWindow: () => tauriGetCurrentWindow() as unknown as TauriWindow,
    open: (options) => tauriOpen(options as Parameters<typeof tauriOpen>[0]),
  };
}

export function isAllowedTauriOrigin(origin: string, development: boolean): boolean {
  try {
    const url = new URL(origin);
    const packaged =
      (url.protocol === "tauri:" && url.hostname === "localhost" && url.port === "") ||
      ((url.protocol === "http:" || url.protocol === "https:") &&
        url.hostname === "tauri.localhost" &&
        url.port === "");
    if (packaged) return true;
    return (
      development &&
      url.protocol === "http:" &&
      url.hostname === "127.0.0.1" &&
      url.port === "5173"
    );
  } catch {
    return false;
  }
}

export function createTauriDesktopShellBridge(
  dependencies: TauriBridgeDependencies = defaultDependencies(),
  origin = typeof location === "undefined" ? "" : location.origin,
  development = false,
): DesktopShellBridge {
  const allowed = isAllowedTauriOrigin(origin, development);
  if (!allowed) {
    return {
      kind: "unavailable",
      ready: async () => false,
      application: {
        querySourceMutationStatus: async () => applicationFailed(),
        querySourceAppendStatus: async () => applicationFailed(),
        queryHistoricalSuccessorStatus: async () => applicationFailed(),
        queryBurstAppendStatus: async () => applicationFailed(),
        queryBurstSuccessorStatus: async () => applicationFailed(),
        queryNisarSuccessorStatus: async () => applicationFailed(),
        queryNisarArchivedRecoveryStatus: async () => applicationFailed(),
        queryReacquireStatus: async () => applicationFailed(),
        getReadiness: async () => applicationFailed(),
        coreRestore: async () => applicationFailed(),
        workbenchPage: async () => applicationFailed(),
      taskPage: async () => applicationFailed(),
      taskSourceBatchPage: async () => applicationFailed(),
        taskHistoryPage: async () => applicationFailed(),
        taskEventPage: async () => applicationFailed(),
        openTaskOutput: async () => applicationFailed(),
        listTaskConflictFiles: async () => applicationFailed(),
        deleteTaskConflictFiles: async () => applicationFailed(),
        updateWorkbenchSelection: async () => applicationFailed(),
        createTaskFromWorkbench: async () => applicationFailed(),
      appendTaskFromWorkbench: async () => applicationFailed(),
      appendTaskFromSourceBatch: async () => applicationFailed(),
      createTaskFromHistoricalSourceBatch: async () => applicationFailed(),
      createBurstAssetTask: async () => applicationFailed(),
      appendBurstAssetTask: async () => applicationFailed(),
      createHistoricalBurstAssetTask: async () => applicationFailed(),
        workspacePrepareLocation: async () => applicationFailed(),
        workspaceOpen: async () => applicationFailed(),
        workspaceCreate: async () => applicationFailed(),
        projectCreate: async () => applicationFailed(),
        projectSelect: async () => applicationFailed(),
        regionCreate: async () => applicationFailed(),
        regionSelect: async () => applicationFailed(),
        planDemTask: async () => applicationFailed(),
        createDemTask: async () => applicationFailed(),
        handoffDemConversion: async () => applicationFailed(),
        createOrbitTask: async () => applicationFailed(),
        createStandaloneOrbitTask: async () => applicationFailed(),
        providerCapabilities: async () => applicationFailed(),
        bindRegionAoi: async () => applicationFailed(),
        clearRegionAoi: async () => applicationFailed(),
        sarSearchAndFreeze: async () => applicationFailed(),
        sarBackgroundCount: async () => applicationFailed(),
        sarSearchProgress: async () => applicationFailed(),
        catalogSearchAndFreeze: async () => applicationFailed(),
        catalogIntentSearchAndFreeze: async () => applicationFailed(),
        cancelRequest: async () => applicationFailed(),
        cdseSessionStatus: async () => applicationFailed(),
        cdseLogin: async () => applicationFailed(),
        cdseLogout: async () => applicationFailed(),
        cdseNetworkPolicy: async () => applicationFailed(),
        setCdseNetworkPolicy: async () => applicationFailed(),
        cdseNetworkDiagnostic: async () => applicationFailed(),
        previewAsset: async () => applicationFailed(),
        cdseQuicklook: async () => applicationFailed(),
        sentinel2MapPreview: async () => applicationFailed(),
        usgsSessionStatus: async () => applicationFailed(),
        usgsLogin: async () => applicationFailed(),
        usgsLogout: async () => applicationFailed(),
        landsatBrowse: async () => applicationFailed(),
        landsatAssetDetails: async () => applicationFailed(),
        bhoonidhiAssetSize: async () => applicationFailed(),
        createLandsatBundleTask: async () => applicationFailed(),
        createLandsatAssetTask: async () => applicationFailed(),
        createLandsatAssetBatchTask: async () => applicationFailed(),
        createLandsatBundleBatchTask: async () => applicationFailed(),
        taskDeliveryAdmission: async () => applicationFailed(),
        opticalPreparationProgress: async () => applicationFailed(),
        createReviewedMpcAssetTask: async () => applicationFailed(),
        createSentinel2ArchiveTask: async () => applicationFailed(),
        createSentinel2ArchiveBatchTask: async () => applicationFailed(),
        createSentinel2BandTask: async () => applicationFailed(),
        createSentinel2BandBatchTask: async () => applicationFailed(),
        createSentinel2L1cBandBatchTask: async () => applicationFailed(),
        createHlsBandBatchTask: async () => applicationFailed(),
        precheckOpticalAoiCoverage: async () => applicationFailed(),
        createOpticalDerivedTask: async () => applicationFailed(),
        effectiveCapability: async () => applicationFailed(),
        nisarRemoteSubsetCapability: async () => applicationFailed(),
        createNisarRemoteSubsetTask: async () => applicationFailed(),
        prepareNisarSubsetIntentV2: async () => applicationFailed(),
        preflightNisarSubsetBatchV2: async () => applicationFailed(),
        prepareNisarSubsetBatchV2: async () => applicationFailed(),
        confirmNisarSubsetTaskV2: async () => applicationFailed(),
        confirmNisarSubsetBatchTaskV2: async () => applicationFailed(),
        appendNisarSubsetBatchTaskV2: async () => applicationFailed(),
        readNisarSubsetAppendContextV2: async () => applicationFailed(),
        preflightNisarSubsetAppendV2: async () => applicationFailed(),
        createNisarSubsetSuccessorTaskV2: async () => applicationFailed(),
        readNisarArchivedRecoveryContextV2: async () => applicationFailed(),
        createNisarArchivedRecoveryV2: async () => applicationFailed(),
        resumeNisarSubsetTask: async () => applicationFailed(),
        startNewlyConfirmedTask: async () => applicationFailed(),
        updateSearchWorkbench: async () => applicationFailed(),
        createDownloadTask: async () => applicationFailed(),
        appendDownloadTask: async () => applicationFailed(),
        updateDownloadTask: async () => applicationFailed(),
        updateDownloadTaskView: async () => applicationFailed(),
        archiveDownloadTask: async () => applicationFailed(),
        deleteDownloadHistory: async () => applicationFailed(),
      },
      compatibility: {
        capabilities: async () => ({
          ok: false,
          error: "Tauri shell origin is unavailable",
          code: "GUI000",
        }),
        callLegacy: async () => ({
          ok: false,
          error: "Tauri shell origin is unavailable",
          code: "GUI000",
        }),
      },
      app: {
        getInfo: async (): Promise<DesktopAppInfo> => ({
          name: "EOStage",
          version: "unavailable",
          offline: true,
        }),
      },
      window: {
        minimize: async () => unavailableResult(),
        toggleMaximize: async () => unavailableResult(),
        close: async () => unavailableResult(),
        getSize: async () => unavailableSize(),
        startDragging: async () => unavailableResult(),
        resizeFromEdge: async () => unavailableResult(),
      },
      dialog: {
        pickOpenFile: async () => unavailablePick(),
        pickDirectory: async () => unavailablePick(),
      },
    };
  }

  const run = async (operation: () => Promise<void>): Promise<DesktopCommandResult> => {
    try {
      await operation();
      return { ok: true };
    } catch {
      return operationFailed();
    }
  };

  const pick = async (options: Record<string, unknown>): Promise<DesktopPickResult> => {
    try {
      const result = selectedPath(await dependencies.open(options));
      if (result === null) return { ok: true, path: "" };
      if (typeof result === "string") return { ok: true, path: result };
      return pickFailed();
    } catch {
      return pickFailed();
    }
  };

  const workspaceContext = async (
    command:
      | "application_workspace_open"
      | "application_workspace_create"
      | "application_project_create"
      | "application_project_select"
      | "application_region_create"
      | "application_region_select",
    request:
      | DesktopWorkspaceOpenRequest
      | DesktopWorkspaceCreateRequest
      | DesktopWorkspaceNamedNodeRequest
      | DesktopWorkspaceSelectNodeRequest,
  ): Promise<DesktopWorkspaceContextResult> => {
    try {
      return parseWorkspaceContext(await dependencies.invoke(command, { request }));
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const setWorkingAoi = async (
    request: DesktopSetWorkingAoiRequest,
  ): Promise<DesktopWorkingAoi | DesktopApplicationError> => {
    try {
      const value = asRecord(await dependencies.invoke("application_set_working_aoi", { request }));
      if (
        value?.state === "working" &&
        typeof value.source_kind === "string" &&
        asRecord(value.geometry) &&
        Array.isArray(value.bbox) &&
        value.bbox.every((item) => typeof item === "number") &&
        typeof value.crs === "string" &&
        typeof value.display_name === "string"
      ) {
        return value as unknown as DesktopWorkingAoi;
      }
      return applicationFailed(value);
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const clearWorkingAoi = async (): Promise<{ ok: true } | DesktopApplicationError> => {
    try {
      const value = await dependencies.invoke("application_clear_working_aoi");
      return value === null || value === undefined ? { ok: true } : applicationFailed(value);
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const getWorkingAoi = async (): Promise<DesktopWorkingAoi | null | DesktopApplicationError> => {
    try {
      const value = await dependencies.invoke("application_working_aoi");
      if (value === null) return null;
      const record = asRecord(value);
      if (
        record?.state === "working" &&
        typeof record.source_kind === "string" &&
        asRecord(record.geometry) &&
        Array.isArray(record.bbox) &&
        record.bbox.every((item) => typeof item === "number") &&
        typeof record.crs === "string" &&
        typeof record.display_name === "string"
      ) {
        return record as unknown as DesktopWorkingAoi;
      }
      return applicationFailed(value);
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const querySourceMutationStatus = async (
    query: DesktopSourceMutationQuery,
  ): Promise<DesktopSourceMutationStatusResult> => {
    if (!query || typeof query !== "object" || !query.request || typeof query.request !== "object") {
      return applicationFailed();
    }
    try {
      let command: string;
      switch (query.kind) {
        case "source_append":
          command = "application_query_source_append_status";
          break;
        case "historical_successor":
          command = "application_query_historical_successor_status";
          break;
        case "burst_append":
          command = "application_query_burst_append_status";
          break;
        case "burst_successor":
          command = "application_query_burst_successor_status";
          break;
        case "nisar_append":
          command = "application_query_nisar_append_status";
          break;
        case "nisar_successor":
          command = "application_query_nisar_successor_status";
          break;
        case "nisar_archived_recovery":
          command = "application_query_nisar_archived_recovery_status";
          break;
        case "reacquire":
          command = "application_query_reacquire_status";
          break;
        default:
          return applicationFailed();
      }
      const raw = await dependencies.invoke(command, { request: query.request });
      return parseSourceMutationStatus(raw);
    } catch {
      return applicationFailed();
    }
  };

  const querySourceAppendStatus = (
    request: DesktopAppendTaskFromSourceBatchRequest,
  ): Promise<DesktopSourceMutationStatusResult> =>
    querySourceMutationStatus({ kind: "source_append", request });

  const queryHistoricalSuccessorStatus = (
    request: DesktopCreateTaskFromHistoricalSourceBatchRequest,
  ): Promise<DesktopSourceMutationStatusResult> =>
    querySourceMutationStatus({ kind: "historical_successor", request });

  const queryBurstAppendStatus = (
    request: DesktopBurstAppendAssetTaskRequest,
  ): Promise<DesktopSourceMutationStatusResult> =>
    querySourceMutationStatus({ kind: "burst_append", request });

  const queryBurstSuccessorStatus = (
    request: DesktopBurstCreateHistoricalAssetTaskRequest,
  ): Promise<DesktopSourceMutationStatusResult> =>
    querySourceMutationStatus({ kind: "burst_successor", request });

  const queryNisarSuccessorStatus = (
    request: DesktopNisarSuccessorTaskV2Request,
  ): Promise<DesktopSourceMutationStatusResult> =>
    querySourceMutationStatus({ kind: "nisar_successor", request });

  const queryNisarArchivedRecoveryStatus = (
    request: DesktopNisarArchivedRecoveryV2Request,
  ): Promise<DesktopSourceMutationStatusResult> =>
    querySourceMutationStatus({ kind: "nisar_archived_recovery", request });

  const queryReacquireStatus = (
    request: DesktopUpdateDownloadTaskRequest,
  ): Promise<DesktopSourceMutationStatusResult> =>
    querySourceMutationStatus({ kind: "reacquire", request });

  const coreSnapshot = async (
    command: ApplicationCommand,
    args?: Record<string, unknown>,
  ): Promise<DesktopCoreRestoreSnapshot> => {
    try {
      return parseCoreSnapshotValue(await dependencies.invoke(command, args));
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const boundedCoreRestore = async (
    command:
      | "application_core_restore"
      | "application_create_task_from_workbench"
      | "application_append_task_from_workbench"
      | "application_append_task_from_source_batch"
      | "application_create_task_from_historical_source_batch"
      | "application_create_burst_asset_task"
      | "application_append_burst_asset_task"
      | "application_create_historical_burst_asset_task",
    args?: Record<string, unknown>,
  ): Promise<DesktopCoreRestoreSnapshot> => {
    try {
      const value = asRecord(await dependencies.invoke(command, args));
      const readiness = asRecord(value?.readiness);
      const core = asRecord(value?.core);
      const aoiDisplay = value?.aoi_display === undefined
        ? undefined
        : value.aoi_display === null
          ? null
          : asRecord(value.aoi_display);
      if (
        !value ||
        !readiness ||
        !core ||
        typeof readiness.contract !== "string" ||
        typeof readiness.status !== "string" ||
        (readiness.store_schema_version !== null &&
          typeof readiness.store_schema_version !== "number") ||
        Array.isArray(core.candidates) ||
        value?.aoi_display !== undefined && value.aoi_display !== null && !aoiDisplay
      ) {
        return applicationFailed();
      }
      return {
        ok: true,
        readiness: {
          contract: readiness.contract,
          status: readiness.status,
          store_schema_version: readiness.store_schema_version as number | null,
        },
        core,
        ...(aoiDisplay === undefined ? {} : { aoi_display: aoiDisplay }),
      };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const boundedWorkbenchPage = async (
    request: DesktopWorkbenchPageRequest,
  ): Promise<DesktopWorkbenchPageSnapshot> => {
    try {
      const value = asRecord(
        await dependencies.invoke("application_workbench_page", { request: { request } }),
      );
      const readiness = asRecord(value?.readiness);
      const page = asRecord(value?.page);
      const items = page && page.items;
      const render = value?.render === undefined
        ? undefined
        : value.render === null
          ? null
          : asRecord(value.render);
      if (
        !value ||
        !readiness ||
        !page ||
        typeof readiness.contract !== "string" ||
        typeof readiness.status !== "string" ||
        (readiness.store_schema_version !== null &&
          typeof readiness.store_schema_version !== "number") ||
        !Array.isArray(items) ||
        items.length > 250 ||
        Array.isArray(page.candidates) ||
        value?.render !== undefined && value.render !== null && !render
      ) {
        return applicationFailed();
      }
      return {
        ok: true,
        readiness: {
          contract: readiness.contract,
          status: readiness.status,
          store_schema_version: readiness.store_schema_version as number | null,
        },
        page,
        ...(render === undefined ? {} : { render }),
      };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const boundedTaskPage = async (
    request: DesktopTaskPageRequest,
  ): Promise<DesktopTaskPageSnapshot> => {
    try {
      const page = asRecord(
        await dependencies.invoke("application_download_task_page", { request }),
      );
      const items = page?.items;
      if (
        !page ||
        page.task_id !== request.task_id ||
        nonNegativeSafeInteger(page.revision) === null ||
        nonNegativeSafeInteger(page.page_index) !== request.page_index ||
        nonNegativeSafeInteger(page.page_size) !== request.page_size ||
        nonNegativeSafeInteger(page.total_count) === null ||
        !Array.isArray(items) ||
        items.length > 200 ||
        items.length > Number(page.page_size) ||
        Array.isArray(page.candidates)
      ) {
        return applicationFailed();
      }
      return { ok: true, page };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const boundedTaskSourceBatchPage = async (
    request: DesktopTaskSourceBatchPageRequest,
  ): Promise<DesktopTaskSourceBatchPageSnapshot> => {
    try {
      const page = asRecord(
        await dependencies.invoke("application_load_task_source_batch_page", { request }),
      );
      const items = page?.items;
      if (
        !page
        || page.task_id !== request.task_id
        || typeof page.source_batch_id !== "string" || !page.source_batch_id.trim()
        || nonNegativeSafeInteger(page.task_revision) === null
        || typeof page.task_status !== "string"
        || page.page_index !== request.page_index
        || page.page_size !== request.page_size
        || nonNegativeSafeInteger(page.total_count) === null
        || !Array.isArray(items)
        || items.length > 200
        || items.length > Number(page.page_size)
        || Array.isArray(page.candidates)
      ) {
        return applicationFailed();
      }
      const itemIds = new Set<string>();
      for (const value of items) {
        const entry = asRecord(value);
        const item = asRecord(entry?.item);
        if (!entry || !item || nonNegativeSafeInteger(entry.ordinal) === null
          || typeof entry.belongs_to_task !== "boolean"
          || typeof item.item_id !== "string" || !item.item_id.trim() || itemIds.has(item.item_id)
          || typeof item.provider_id !== "string" || !item.provider_id.trim()
          || typeof item.display_name !== "string"
          || !(item.acquisition_datetime === null || typeof item.acquisition_datetime === "string")
          || !asRecord(item.metadata) || !asRecord(item.payload)) return applicationFailed();
        itemIds.add(item.item_id);
      }
      return {
        ok: true,
        page: page as Extract<DesktopTaskSourceBatchPageSnapshot, { ok: true }>["page"],
      };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const sarSearchProgress = async (requestId: string): Promise<DesktopSarSearchProgressResult> => {
    if (!requestId || requestId.trim() !== requestId || requestId.length > 128) {
      return applicationFailed();
    }
    try {
      const raw = await dependencies.invoke("application_sar_search_progress", { requestId });
      if (raw === null) return { ok: true, progress: null };
      const value = asRecord(raw);
      const allowedStages = new Set([
        "validating",
        "planning",
        "waiting_provider",
        "fetching",
        "filtering",
        "freezing",
        "completed",
        "failed",
        "cancelling",
        "cancelled",
      ]);
      const allowedKeys = new Set([
        "request_id",
        "sequence",
        "stage",
        "provider_request_count",
        "raw_fetched_count",
        "mapped_count",
        "exact_matched_count",
        "admitted_count",
        "requested_limit",
        "remote_total",
        "elapsed_ms",
        "message",
        "terminal",
      ]);
      const optionalCount = (raw: unknown) => {
        if (raw === undefined || raw === null) return { ok: true, value: null as number | null };
        const parsed = nonNegativeSafeInteger(raw);
        return parsed === null
          ? { ok: false, value: null as number | null }
          : { ok: true, value: parsed };
      };
      const providerRequestCount = optionalCount(value?.provider_request_count);
      const rawFetchedCount = optionalCount(value?.raw_fetched_count);
      const mappedCount = optionalCount(value?.mapped_count);
      const exactMatchedCount = optionalCount(value?.exact_matched_count);
      const admittedCount = optionalCount(value?.admitted_count);
      const requestedLimit = optionalCount(value?.requested_limit);
      const remoteTotal = optionalCount(value?.remote_total);
      const sequence = nonNegativeSafeInteger(value?.sequence);
      const elapsedMs = nonNegativeSafeInteger(value?.elapsed_ms);
      const terminalStage = value?.stage === "completed"
        || value?.stage === "failed"
        || value?.stage === "cancelled";
      const invalidCountOrder = (
        mappedCount.value !== null
        && rawFetchedCount.value !== null
        && mappedCount.value > rawFetchedCount.value
      ) || (
        exactMatchedCount.value !== null
        && mappedCount.value !== null
        && exactMatchedCount.value > mappedCount.value
      ) || (
        admittedCount.value !== null
        && exactMatchedCount.value !== null
        && admittedCount.value > exactMatchedCount.value
      ) || (
        admittedCount.value !== null
        && requestedLimit.value !== null
        && admittedCount.value > requestedLimit.value
      );
      if (
        !value
        || Object.keys(value).some((key) => !allowedKeys.has(key))
        || value.request_id !== requestId
        || sequence === null
        || sequence < 1
        || typeof value.stage !== "string"
        || !allowedStages.has(value.stage)
        || !providerRequestCount.ok
        || !rawFetchedCount.ok
        || !mappedCount.ok
        || !exactMatchedCount.ok
        || !admittedCount.ok
        || !requestedLimit.ok
        || (requestedLimit.value !== null
          && (requestedLimit.value < 1 || requestedLimit.value > 10_000))
        || !remoteTotal.ok
        || elapsedMs === null
        || invalidCountOrder
        || typeof value.message !== "string"
        || value.message.length < 1
        || value.message.length > 160
        || typeof value.terminal !== "boolean"
        || value.terminal !== terminalStage
      ) return applicationFailed();
      return {
        ok: true,
        progress: {
          request_id: requestId,
          sequence,
          stage: value.stage as DesktopSarSearchProgressStage,
          provider_request_count: providerRequestCount.value,
          raw_fetched_count: rawFetchedCount.value,
          mapped_count: mappedCount.value,
          exact_matched_count: exactMatchedCount.value,
          admitted_count: admittedCount.value,
          requested_limit: requestedLimit.value,
          remote_total: remoteTotal.value,
          elapsed_ms: elapsedMs,
          message: value.message,
          terminal: value.terminal,
        },
      };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const boundedTaskHistoryPage = async (
    request: DesktopTaskHistoryPageRequest,
  ): Promise<DesktopTaskHistoryPageSnapshot> => {
    try {
      const hasEpoch = request.after_archived_at_epoch_ms != null;
      const hasTaskId = request.after_task_id != null;
      if (
        !Number.isSafeInteger(request.page_size) || request.page_size < 1 || request.page_size > 100 ||
        hasEpoch !== hasTaskId ||
        (hasEpoch && (!validTaskHistoryCursorEpoch(request.after_archived_at_epoch_ms) ||
          !validTaskHistoryTaskId(request.after_task_id)))
      ) return applicationFailed();
      const page = asRecord(
        await dependencies.invoke("application_download_history_page", { request }),
      );
      const items = page?.items;
      const cursor = asRecord(page?.next_cursor);
      const cursorValid = cursor !== null &&
        validTaskHistoryCursorEpoch(cursor.archived_at_epoch_ms) &&
        validTaskHistoryTaskId(cursor.task_id);
      if (
        !page ||
        nonNegativeSafeInteger(page.page_size) === null ||
        Number(page.page_size) < 1 || Number(page.page_size) > 100 ||
        typeof page.has_more !== "boolean" ||
        (page.has_more === true && !cursorValid) ||
        (page.has_more === false && page.next_cursor !== null) ||
        !Array.isArray(items) ||
        items.length > 100 ||
        items.length > Number(page.page_size) ||
        Array.isArray(page.candidates) ||
        items.some((item) => {
          const row = asRecord(item);
          const datasetId = row?.dataset_id;
          const datasetIdValid = datasetId === undefined || datasetId === null || (
            typeof datasetId === "string" && datasetId.length > 0 &&
            Array.from(datasetId).length <= 128 && datasetId.trim() === datasetId &&
            !/[\u0000-\u001f\u007f]/u.test(datasetId)
          );
          return !row || !validTaskHistoryTimestamp(row.archived_at) ||
            !datasetIdValid ||
            Array.isArray(row.candidates) || Array.isArray(row.selected_item_ids) ||
            typeof row.snapshot_json === "string" || typeof row.state_json === "string";
        })
      ) return applicationFailed();
      return { ok: true, page };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const boundedTaskEventPage = async (
    request: DesktopTaskEventPageRequest,
  ): Promise<DesktopTaskEventPageSnapshot> => {
    const beforeSequence = request.before_sequence ?? null;
    if (
      !validTaskHistoryTaskId(request.task_id) ||
      nonNegativeSafeInteger(request.expected_revision) === null ||
      (beforeSequence !== null && nonNegativeSafeInteger(beforeSequence) === null) ||
      !Number.isSafeInteger(request.page_size) || request.page_size < 1 || request.page_size > 100
    ) return applicationFailed();
    try {
      const page = asRecord(await dependencies.invoke("application_download_task_event_page", {
        request: { ...request, before_sequence: beforeSequence },
      }));
      const items = page?.items;
      const allowedLevels = new Set(["info", "warning", "success", "error"]);
      const allowedKinds = new Set([
        "started", "progress", "paused", "resumed", "completed", "failed",
        "cancelled", "retried", "updated",
      ]);
      const validOptionalScalar = (value: unknown, max: number): boolean => value === null || (
        typeof value === "string" && value.length > 0 && Array.from(value).length <= max &&
        value.trim() === value && !/[\u0000-\u001f\u007f]/u.test(value)
      );
      if (
        !page || !hasOnlyKeys(page, [
          "task_id", "revision", "page_size", "has_more", "next_before_sequence", "items",
        ]) ||
        page.task_id !== request.task_id ||
        // Event reads observe current Store state; only mutations use revision CAS.
        nonNegativeSafeInteger(page.revision) === null ||
        nonNegativeSafeInteger(page.page_size) !== request.page_size ||
        typeof page.has_more !== "boolean" ||
        (page.has_more === true && nonNegativeSafeInteger(page.next_before_sequence) === null) ||
        (page.has_more === false && page.next_before_sequence !== null) ||
        !Array.isArray(items) || items.length > request.page_size || items.length > 100 ||
        items.some((item) => {
          const row = asRecord(item);
          return !row || !hasOnlyKeys(row, [
            "sequence", "occurred_at", "level", "event_kind", "item_label", "code", "message_zh",
          ]) ||
            nonNegativeSafeInteger(row.sequence) === null ||
            !validTaskHistoryTimestamp(row.occurred_at) ||
            typeof row.level !== "string" || !allowedLevels.has(row.level) ||
            typeof row.event_kind !== "string" || !allowedKinds.has(row.event_kind) ||
            !validOptionalScalar(row.item_label, 256) ||
            !validOptionalScalar(row.code, 64) ||
            typeof row.message_zh !== "string" || !validOptionalScalar(row.message_zh, 128);
        })
      ) return applicationFailed();
      return { ok: true, page };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const openTaskOutput = async (
    request: DesktopOpenTaskOutputRequest,
  ): Promise<DesktopOpenTaskOutputResult> => {
    if (!validTaskHistoryTaskId(request.task_id) || nonNegativeSafeInteger(request.expected_revision) === null) {
      return applicationFailed();
    }
    try {
      const ack = asRecord(await dependencies.invoke("application_open_download_task_output", { request }));
      if (!ack || !hasOnlyKeys(ack, ["task_id", "revision"]) ||
        ack.task_id !== request.task_id || nonNegativeSafeInteger(ack.revision) !== request.expected_revision) {
        return applicationFailed();
      }
      return { ok: true, task_id: request.task_id, revision: request.expected_revision };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const listTaskConflictFiles = async (
    request: DesktopListTaskConflictFilesRequest,
  ): Promise<DesktopTaskConflictFileListResult> => {
    if (!validTaskHistoryTaskId(request.task_id)) return applicationFailed();
    try {
      const list = asRecord(await dependencies.invoke("application_list_task_conflict_files", { request }));
      const files = list?.files;
      if (
        !list || !hasOnlyKeys(list, ["task_id", "files", "total_count", "total_size_bytes"]) ||
        list.task_id !== request.task_id ||
        nonNegativeSafeInteger(list.total_count) === null ||
        nonNegativeSafeInteger(list.total_size_bytes) === null ||
        !Array.isArray(files) || files.length > 10000 || files.length > (list.total_count as number) ||
        files.some((file) => {
          const row = asRecord(file);
          return !row || !hasOnlyKeys(row, ["path", "size_bytes", "modified_at_epoch_ms", "recorded"]) ||
            typeof row.path !== "string" || row.path.length === 0 || Array.from(row.path).length > 4096 ||
            row.path.trim() !== row.path ||
            nonNegativeSafeInteger(row.size_bytes) === null ||
            nonNegativeSafeInteger(row.modified_at_epoch_ms) === null ||
            typeof row.recorded !== "boolean";
        })
      ) return applicationFailed();
      return { ok: true, list: list as unknown as TaskConflictFileList };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const deleteTaskConflictFiles = async (
    request: DesktopDeleteTaskConflictFilesRequest,
  ): Promise<DesktopTaskConflictFileDeleteResult> => {
    if (
      !validTaskHistoryTaskId(request.task_id) ||
      !Array.isArray(request.expected) || request.expected.length > 10000 ||
      request.expected.some((item) =>
        typeof item?.path !== "string" || item.path.length === 0 || Array.from(item.path).length > 4096 ||
        item.path.trim() !== item.path ||
        nonNegativeSafeInteger(item?.size_bytes) === null ||
        nonNegativeSafeInteger(item?.modified_at_epoch_ms) === null,
      )
    ) return applicationFailed();
    try {
      const result = asRecord(await dependencies.invoke("application_delete_task_conflict_files", { request }));
      const outcomes = result?.outcomes;
      if (
        !result || !hasOnlyKeys(result, ["task_id", "outcomes"]) ||
        result.task_id !== request.task_id ||
        !Array.isArray(outcomes) || outcomes.length > 10000 || outcomes.length > request.expected.length ||
        outcomes.some((outcome) => {
          const row = asRecord(outcome);
          return !row || !hasOnlyKeys(row, ["path", "outcome"]) ||
            typeof row.path !== "string" || row.path.length === 0 ||
            typeof row.outcome !== "string" ||
            !["deleted", "skipped_missing", "skipped_changed", "skipped_unsafe"].includes(row.outcome);
        })
      ) return applicationFailed();
      return { ok: true, result: result as unknown as TaskConflictFileDeleteResult };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const nativeJson = async (
    command:
      | "application_aoi_import"
      | "application_aoi_normalize"
      | "application_admin_boundary_search"
      | "application_admin_boundary_resolve"
      | "application_admin_options"
      | "application_gacos_plan"
      | "application_gacos_dry_run"
      | "application_report_generate"
      | "application_standalone_dem"
      | "application_import_scenes"
      | "application_match_orbits_directory"
      | "application_gacos_ledger"
      | "application_gacos_import_result",
    args?: Record<string, unknown>,
  ): Promise<DesktopNativeJsonResult> => {
    try {
      const value = asRecord(await dependencies.invoke(command, args));
      return value ?? applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const demTaskSettings = async (): Promise<DesktopDemTaskSettings | DesktopApplicationError> => {
    try {
      const value = asRecord(await dependencies.invoke("application_get_dem_task_settings"));
      const maxTasks = nonNegativeSafeInteger(value?.max_concurrent_tasks);
      const maxConversions = nonNegativeSafeInteger(value?.max_concurrent_conversions);
      if (
        maxTasks === null ||
        maxConversions === null ||
        maxTasks < 1 ||
        maxTasks > 10 ||
        maxConversions !== 1
      ) {
        return applicationFailed();
      }
      return {
        max_concurrent_tasks: maxTasks,
        max_concurrent_conversions: maxConversions,
      };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const saveDemTaskSettings = async (
    settings: DesktopDemTaskSettings,
  ): Promise<DesktopDemTaskSettings | DesktopApplicationError> => {
    try {
      const value = asRecord(
        await dependencies.invoke("application_save_dem_task_settings", { settings }),
      );
      const maxTasks = nonNegativeSafeInteger(value?.max_concurrent_tasks);
      const maxConversions = nonNegativeSafeInteger(value?.max_concurrent_conversions);
      if (
        maxTasks === null ||
        maxConversions === null ||
        maxTasks < 1 ||
        maxTasks > 10 ||
        maxConversions !== 1
      ) {
        return applicationFailed();
      }
      return {
        max_concurrent_tasks: maxTasks,
        max_concurrent_conversions: maxConversions,
      };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const parseSarDownloadPolicySettings = (
    value: unknown,
  ): DesktopSarDownloadPolicySettings | DesktopApplicationError => {
    const record = asRecord(value);
    const concurrency = nonNegativeSafeInteger(record?.max_concurrent_items);
    if (
      !record
      || !hasOnlyKeys(record, ["mode", "max_concurrent_items", "organize_by_product", "default_output_root"])
      || record.mode !== "manual"
      || concurrency === null
      || concurrency < 1
      || concurrency > 8
      || typeof record.organize_by_product !== "boolean"
      || typeof record.default_output_root !== "string"
    ) {
      return applicationFailed();
    }
    return {
      mode: "manual",
      max_concurrent_items: concurrency,
      organize_by_product: record.organize_by_product,
      default_output_root: record.default_output_root,
    };
  };

  const getSarDownloadPolicySettings = async (
  ): Promise<DesktopSarDownloadPolicySettings | DesktopApplicationError> => {
    try {
      return parseSarDownloadPolicySettings(
        await dependencies.invoke("application_get_sar_download_policy_settings"),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const saveSarDownloadPolicySettings = async (
    settings: DesktopSarDownloadPolicySettings,
  ): Promise<DesktopSarDownloadPolicySettings | DesktopApplicationError> => {
    try {
      return parseSarDownloadPolicySettings(
        await dependencies.invoke("application_save_sar_download_policy_settings", { settings }),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const openExternalUrl = async (url: string) => {
    try {
      const record = asRecord(await dependencies.invoke("application_open_external_url", { url }));
      return record?.url === url
        ? { ok: true as const, url }
        : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const standaloneDem = (request: DesktopStandaloneDemRequest): Promise<DesktopNativeJsonResult> =>
    nativeJson("application_standalone_dem", { request });

  const createStandaloneDemTask = (request: DesktopStandaloneDemTaskRequest) =>
    coreSnapshot("application_create_standalone_dem_task", { request });

  const importScenes = (request: DesktopSceneImportRequest): Promise<DesktopNativeJsonResult> =>
    nativeJson("application_import_scenes", { request });

  const matchOrbitsDirectory = (
    request: DesktopOrbitDirectoryMatchRequest,
  ): Promise<DesktopNativeJsonResult> =>
    nativeJson("application_match_orbits_directory", { request });

  const gacosLedger = (): Promise<DesktopNativeJsonResult> =>
    nativeJson("application_gacos_ledger");

  const gacosImportResult = (
    request: DesktopGacosResultImportRequest,
  ): Promise<DesktopNativeJsonResult> =>
    nativeJson("application_gacos_import_result", { request });

  const gacosCreateImportTask = async (
    request: DesktopGacosCreateImportTaskRequest,
  ): Promise<DesktopGacosCreateImportTaskResult> => {
    const value = asRecord(request as unknown);
    if (
      !value
      || !hasOnlyKeys(value, ["task_id", "source_request_task_id", "source_request_item_id", "source_paths"])
      || !validTaskHistoryTaskId(value.task_id)
      || !validTaskHistoryTaskId(value.source_request_task_id)
      || !validTaskHistoryTaskId(value.source_request_item_id)
      || value.task_id === value.source_request_task_id
      || !Array.isArray(value.source_paths)
      || value.source_paths.length < 1
      || value.source_paths.length > 40
      || value.source_paths.some((path) => !validBoundedDesktopScalar(path, 4096) || !absoluteNisarOutputRoot(path))
      || new Set(value.source_paths).size !== value.source_paths.length
    ) {
      return applicationFailed();
    }
    try {
      return parseGacosCreateImportTaskResult(
        await dependencies.invoke("application_gacos_create_import_task", { request }),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const gacosCreateResultTask = async (
    request: DesktopGacosCreateResultTaskRequest,
  ): Promise<DesktopGacosCreateImportTaskResult> => {
    const value = asRecord(request as unknown);
    if (!value
      || !hasOnlyKeys(value, ["task_id", "source_request_task_id", "source_request_item_id", "result_url"])
      || !validTaskHistoryTaskId(value.task_id)
      || !validTaskHistoryTaskId(value.source_request_task_id)
      || !validTaskHistoryTaskId(value.source_request_item_id)
      || value.task_id === value.source_request_task_id
      || !validGacosResultLink(value.result_url)) return applicationFailed();
    try {
      const result = parseGacosCreateImportTaskResult(
        await dependencies.invoke("application_gacos_create_result_task", { request }),
        "native-gacos-manual-result",
      );
      return result.ok && result.task_id === request.task_id ? result : applicationFailed();
    } catch (error) { return applicationFailed(error); }
  };

  const gacosSupplyResultLink = async (
    request: DesktopGacosSupplyResultLinkRequest,
  ): Promise<DesktopGacosSupplyResultLinkResult> => {
    const value = asRecord(request as unknown);
    if (!value || !hasOnlyKeys(value, ["task_id", "result_url"])
      || !validTaskHistoryTaskId(value.task_id) || !validGacosResultLink(value.result_url)) return applicationFailed();
    try {
      const result = asRecord(await dependencies.invoke("application_gacos_supply_result_link", { request }));
      if (!result || !hasOnlyKeys(result, ["ok", "task_id", "source"])
        || result.ok !== true || result.task_id !== request.task_id
        || result.source !== "native-gacos-manual-result") return applicationFailed();
      return { ok: true, task_id: request.task_id, source: result.source };
    } catch (error) { return applicationFailed(error); }
  };

  const gacosSubmitRequest = async (
    request: DesktopGacosConfirmRequest,
  ): Promise<DesktopGacosSubmissionResult> => {
    const value = asRecord(request as unknown);
    if (
      !value
      || !hasOnlyKeys(value, [
        "prepared_batch_id", "plan_fingerprint_sha256", "task_id", "confirm", "force_reapply",
      ])
      || !validTaskHistoryTaskId(value.prepared_batch_id)
      || typeof value.plan_fingerprint_sha256 !== "string"
      || !/^[a-f0-9]{64}$/.test(value.plan_fingerprint_sha256)
      || !validTaskHistoryTaskId(value.task_id)
      || value.confirm !== true
      || typeof value.force_reapply !== "boolean"
    ) {
      return applicationFailed();
    }
    try {
      return parseGacosSubmissionResult(
        await dependencies.invoke("application_gacos_submit_request", { request }),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const gacosRequestStatus = async (taskId: string): Promise<DesktopGacosSubmissionResult> => {
    if (!validTaskHistoryTaskId(taskId)) return applicationFailed();
    try {
      return parseGacosSubmissionResult(
        await dependencies.invoke("application_gacos_request_status", { taskId }),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const parseNetworkSettings = (
    value: unknown,
  ): DesktopNetworkSettings | DesktopApplicationError => {
    const record = asRecord(value);
    if (
      !record ||
      typeof record.proxy_enabled !== "boolean" ||
      typeof record.proxy_url !== "string" ||
      typeof record.cache_enabled !== "boolean" ||
      typeof record.cache_dir !== "string" ||
      typeof record.cache_limit_mb !== "number" ||
      typeof record.asf_ssl_verify !== "boolean"
    ) {
      return applicationFailed();
    }
    return {
      proxy_enabled: record.proxy_enabled,
      proxy_url: record.proxy_url,
      cache_enabled: record.cache_enabled,
      cache_dir: record.cache_dir,
      cache_limit_mb: record.cache_limit_mb,
      tianditu_token: typeof record.tianditu_token === "string" ? record.tianditu_token : "",
      asf_ssl_verify: record.asf_ssl_verify,
    };
  };

  const getNetworkSettings = async (): Promise<DesktopNetworkSettings | DesktopApplicationError> => {
    try {
      return parseNetworkSettings(await dependencies.invoke("application_get_network_settings"));
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const saveNetworkSettings = async (
    settings: DesktopNetworkSettings,
  ): Promise<DesktopNetworkSettings | DesktopApplicationError> => {
    try {
      const sanitizedSettings: DesktopNetworkSettings = {
        proxy_enabled: Boolean(settings.proxy_enabled),
        proxy_url: typeof settings.proxy_url === "string" ? settings.proxy_url : "",
        cache_enabled: Boolean(settings.cache_enabled),
        cache_dir: typeof settings.cache_dir === "string" ? settings.cache_dir : "",
        cache_limit_mb: Number.isFinite(settings.cache_limit_mb) ? Number(settings.cache_limit_mb) : 0,
        tianditu_token: typeof settings.tianditu_token === "string" ? settings.tianditu_token : "",
        asf_ssl_verify: settings.asf_ssl_verify !== false,
      };
      return parseNetworkSettings(
        await dependencies.invoke("application_save_network_settings", {
          settings: sanitizedSettings,
        }),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const parseProviderConnectivityResult = (
    value: unknown,
  ): DesktopProviderConnectivityResult[] | DesktopApplicationError => {
    if (!Array.isArray(value)) return applicationFailed();
    const results: DesktopProviderConnectivityResult[] = [];
    for (const entry of value) {
      const record = asRecord(entry);
      if (
        !record ||
        typeof record.id !== "string" ||
        typeof record.site !== "string" ||
        typeof record.ok !== "boolean" ||
        typeof record.latency_ms !== "number" ||
        typeof record.error_code !== "string"
      ) {
        return applicationFailed();
      }
      results.push({
        id: record.id,
        site: record.site,
        ok: record.ok,
        latency_ms: record.latency_ms,
        http_status:
          record.http_status === null || record.http_status === undefined
            ? null
            : typeof record.http_status === "number"
              ? record.http_status
              : null,
        error_code: record.error_code,
      });
    }
    return results;
  };

  const probeProviderConnectivity = async (): Promise<
    DesktopProviderConnectivityResult[] | DesktopApplicationError
  > => {
    try {
      return parseProviderConnectivityResult(
        await dependencies.invoke("application_probe_provider_connectivity"),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const selectionSummary = async (
    command: DesktopWorkbenchSelectionCommand,
  ): Promise<DesktopWorkbenchSelectionResult> => {
    try {
      const value = asRecord(
        await dependencies.invoke("application_update_workbench_selection", { command }),
      );
      const revision = nonNegativeSafeInteger(value?.revision);
      const checkedCount = nonNegativeSafeInteger(value?.checked_count);
      const pageIndex = nonNegativeSafeInteger(value?.page_index);
      const pageSize = nonNegativeSafeInteger(value?.page_size);
      if (
        !value ||
        typeof value.batch_id !== "string" ||
        revision === null ||
        checkedCount === null ||
        pageIndex === null ||
        pageSize === null ||
        pageSize < 1 ||
        pageSize > 250
      ) {
        return applicationFailed();
      }
      return {
        ok: true,
        summary: {
          batch_id: value.batch_id,
          revision,
          checked_count: checkedCount,
          page_index: pageIndex,
          page_size: pageSize,
        },
      };
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const workspacePrepareLocation = async (
    request: DesktopWorkspacePrepareLocationRequest,
  ): Promise<DesktopWorkspacePrepareLocationResult> => {
    try {
      return parseWorkspacePreparedLocation(
        await dependencies.invoke("application_workspace_prepare_location", { request }),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const planDemTask = async (
    request: DesktopDemTaskPlanRequest,
  ): Promise<DesktopDemTaskPlanResult> => {
    try {
      const plan = parseDemTaskPlan(
        await dependencies.invoke("application_plan_dem_task", { request }),
      );
      return plan ? { ok: true, plan } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const cdseStatus = async (
    command:
      | "application_cdse_session_status"
      | "application_cdse_login"
      | "application_cdse_logout"
      | "application_set_cdse_network_policy",
    args?: Record<string, unknown>,
  ): Promise<DesktopCdseSessionStatusResult> => {
    try {
      const status = parseCdseSessionStatus(await dependencies.invoke(command, args));
      return status ? { ok: true, status } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const cdseNetworkPolicy = async (): Promise<DesktopCdseNetworkPolicyResult> => {
    try {
      const networkMode = parseCdseNetworkMode(
        await dependencies.invoke("application_cdse_network_policy"),
      );
      return networkMode ? { ok: true, network_mode: networkMode } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const cdseNetworkDiagnostic = async (
    networkMode: DesktopCdseNetworkMode,
  ): Promise<DesktopCdseNetworkDiagnosticResult> => {
    try {
      const diagnostic = parseCdseNetworkDiagnostic(
        await dependencies.invoke("application_cdse_network_diagnostic", { mode: networkMode }),
      );
      return diagnostic ? { ok: true, diagnostic } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const previewAsset = async (
    request: DesktopPreviewAssetRequest,
  ): Promise<DesktopPreviewAssetResult> => {
    try {
      const preview = parsePreviewAsset(
        await dependencies.invoke("application_preview_asset", { request }),
      );
      return preview ? { ok: true, preview } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const savePreview = async (
    request: DesktopPreviewSaveRequest,
  ): Promise<DesktopPreviewSaveResult> => {
    const value = asRecord(request as unknown);
    if (
      !value
      || !hasOnlyKeys(value, ["data_url", "output_dir", "file_name"])
      || !validBoundedDesktopScalar(value.data_url, 24 * 1024 * 1024)
      || !/^data:image\/(?:jpeg|png|webp);base64,[A-Za-z0-9+/]*={0,2}$/.test(value.data_url)
      || !absoluteNisarOutputRoot(value.output_dir)
      || !validBoundedDesktopScalar(value.file_name, 240)
    ) {
      return applicationFailed();
    }
    try {
      return parsePreviewSaveResult(
        await dependencies.invoke("application_save_preview_asset", { request }),
      );
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const cdseQuicklook = async (
    request: DesktopCdseQuicklookRequest,
  ): Promise<DesktopCdseQuicklookResult> => {
    try {
      const quicklook = parseCdseQuicklook(
        await dependencies.invoke("application_cdse_quicklook", { request }),
      );
      return quicklook ? { ok: true, quicklook } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const sentinel2MapPreview = async (
    request: DesktopSentinel2MapPreviewRequest,
  ): Promise<DesktopSentinel2MapPreviewResult> => {
    try {
      const preview = parseSentinel2MapPreview(
        await dependencies.invoke("application_sentinel2_map_preview", { request }),
      );
      return preview ? { ok: true, preview } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const usgsStatus = async (
    command:
      | "application_usgs_session_status"
      | "application_usgs_login"
      | "application_usgs_logout",
    args?: Record<string, unknown>,
  ): Promise<DesktopUsgsM2mSessionStatusResult> => {
    try {
      const status = parseUsgsSessionStatus(await dependencies.invoke(command, args));
      return status ? { ok: true, status } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const landsatBrowse = async (
    request: DesktopLandsatBrowseRequest,
  ): Promise<DesktopLandsatBrowseResult> => {
    try {
      const browse = parseCdseQuicklook(
        await dependencies.invoke("application_landsat_browse", { request }),
      );
      return browse ? { ok: true, browse } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const landsatAssetDetails = async (
    request: DesktopLandsatAssetDetailsRequest,
  ): Promise<DesktopLandsatAssetDetailsResult> => {
    try {
      const details = parseLandsatAssetDetails(
        await dependencies.invoke("application_landsat_asset_details", { request }),
      );
      return details ? { ok: true, details } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };
  const bhoonidhiAssetSize = async (
    request: DesktopBhoonidhiAssetSizeRequest,
  ): Promise<DesktopBhoonidhiAssetSizeResult> => {
    try {
      const size = parseBhoonidhiAssetSize(
        await dependencies.invoke("application_bhoonidhi_asset_size", { request }),
      );
      return size ? { ok: true, size } : applicationFailed();
    } catch (error) {
      return applicationFailed(error);
    }
  };


  const compatibility: DesktopCompatibilityBridge = {
    capabilities: async () => ({
      ok: false,
      error: t("shared.desktopBridge.pythonWorkerRemoved"),
      code: "COMPAT_WORKER_REMOVED",
    }),
    callLegacy: async (method: string) => ({
      ok: false,
      error: t("shared.desktopBridge.legacyMethodRejected", { method }),
      code: "COMPAT_WORKER_OPERATION_UNSUPPORTED",
    }),
  };
  const credentialMutation = async (
    command: string,
    args?: Record<string, unknown>,
  ): Promise<DesktopCredentialMutationResult> => {
    try {
      return parseCredentialMutation(await dependencies.invoke(command, args));
    } catch (error) {
      return applicationFailed(error);
    }
  };

  const credentials: DesktopCredentialBridge = {
    readForEdit: async (field) => {
      if (!isCredentialEditorField(field)) return applicationFailed();
      try {
        const result = asRecord(await dependencies.invoke("credential_read_for_edit", { field }));
        return result?.ok === true && hasOnlyKeys(result, ["ok", "value"])
          && (result.value === null || validCredentialEditorValue(result.value))
          ? { ok: true, value: result.value as string | null } : applicationFailed();
      } catch { return applicationFailed(); }
    },
    readAgentChannelKey: async (channelId) => {
      const trimmed = typeof channelId === "string" ? channelId.trim() : "";
      if (!trimmed) return { ok: true, value: null };
      try {
        const result = asRecord(await dependencies.invoke("credential_agent_channel_key_read", { channelId: trimmed }));
        return result?.ok === true && hasOnlyKeys(result, ["ok", "value"])
          && (result.value === null || typeof result.value === "string")
          ? { ok: true, value: result.value as string | null } : applicationFailed();
      } catch { return applicationFailed(); }
    },
    updateField: async (field, value) => {
      if (!isCredentialEditorField(field) || !validCredentialEditorValue(value)) {
        return { ok: false, error: t("shared.desktopBridge.credentialInputInvalid"), code: "DESKTOP_CREDENTIAL_INPUT_INVALID" };
      }
      try {
        const result = asRecord(await dependencies.invoke("credential_update_field", { field, value }));
        // Never reflect a native error payload: it may contain submitted material.
        return result?.ok === true && hasOnlyKeys(result, ["ok"])
          ? { ok: true } : { ok: false, error: t("shared.desktopBridge.credentialAutoSaveFailed"), code: "DESKTOP_CREDENTIAL_STORE_UNAVAILABLE" };
      } catch { return { ok: false, error: t("shared.desktopBridge.credentialAutoSaveFailed"), code: "DESKTOP_CREDENTIAL_STORE_UNAVAILABLE" }; }
    },
    checkBhoonidhiAuth: async () => {
      try {
        const value = await dependencies.invoke("credential_bhoonidhi_auth") as Record<string, unknown> | null;
        return value?.ok === true && value.session_available === true && value.catalogue_verified === false
          ? { ok: true, session_available: true, catalogue_verified: false } : applicationFailed();
      } catch (error) { return applicationFailed(error); }
    },
    getBhoonidhiSessionStatus: async (): Promise<DesktopBhoonidhiSessionStatusResult> => {
      try {
        const status = parseBhoonidhiSessionStatus(
          await dependencies.invoke("credential_bhoonidhi_session_status"),
        );
        return status ? { ok: true, status } : applicationFailed();
      } catch (error) {
        return applicationFailed(error);
      }
    },
    reveal: async (field) => {
      try {
        const value = await dependencies.invoke("credential_reveal", { field }) as Record<string, unknown> | null;
        return value?.ok === true && (value.value === null || typeof value.value === "string")
          ? { ok: true, value: value.value } : applicationFailed();
      } catch {
        // Never reflect a rejected reveal payload: it might contain material.
        return applicationFailed();
      }
    },
    getStatus: async (): Promise<DesktopCredentialStatusResult> => {
      try {
        const status = parseCredentialStatus(await dependencies.invoke("credential_status"));
        return status ? { ok: true, status } : applicationFailed();
      } catch (error) {
        return applicationFailed(error);
      }
    },
    checkEarthdataAuth: async (force = false): Promise<DesktopEarthdataAuthResult> => {
      try {
        return parseEarthdataAuth(
          await dependencies.invoke("credential_earthdata_auth", { force }),
        );
      } catch (error) {
        return applicationFailed(error);
      }
    },
    saveCdseLogin: (username, password) =>
      credentialMutation("credential_save_cdse_login", { username, password }),
    clearCdse: () => credentialMutation("credential_clear_cdse"),
    saveUsgsLogin: (username, applicationToken) =>
      credentialMutation("credential_save_usgs_login", { username, applicationToken }),
    clearUsgs: () => credentialMutation("credential_clear_usgs"),
    saveEarthdataToken: (token) =>
      credentialMutation("credential_save_earthdata_token", { token }),
    saveEarthdataLogin: (username, password) =>
      credentialMutation("credential_save_earthdata_login", { username, password }),
    clearEarthdata: () => credentialMutation("credential_clear_earthdata"),
    saveBhoonidhiLogin: (username, password) =>
      credentialMutation("credential_save_bhoonidhi_login", { username, password }),
    clearBhoonidhi: () => credentialMutation("credential_clear_bhoonidhi"),
    saveOpentopographyKey: (apiKey) =>
      credentialMutation("credential_save_opentopography_key", { apiKey }),
    clearOpentopographyKey: () => credentialMutation("credential_clear_opentopography_key"),
    saveGacosEmail: (email) => credentialMutation("credential_save_gacos_email", { email }),
    clearGacosEmail: () => credentialMutation("credential_clear_gacos_email"),
  };

  return {
    kind: "tauri",
    ready: async () => true,
    credentials,
    application: {
      getRuntimeComponents: async (): Promise<any> => {
        return tauriInvoke("application_runtime_components_inspect");
      },
      fetchExtensionRegistry: async (registryUrl?: string): Promise<any> => {
        return tauriInvoke("application_extension_registry_fetch", {
          registryUrl: registryUrl ?? null,
        });
      },
      installRuntimeComponent: async (request: any): Promise<any> => {
        return tauriInvoke("application_runtime_component_install", { request });
      },
      getPluginsStatus: async (): Promise<any> => {
        return tauriInvoke("application_plugins_status");
      },
      installPluginPackage: async (request: any): Promise<any> => {
        return tauriInvoke("application_plugin_install", { request });
      },
      setPluginEnabled: async (pluginId: string, enabled: boolean): Promise<any> => {
        return tauriInvoke("application_plugin_set_enabled", { pluginId, enabled });
      },
      uninstallPluginPackage: async (pluginId: string): Promise<any> => {
        return tauriInvoke("application_plugin_uninstall", { pluginId });
      },
      saveCustomConnector: async (connector: any): Promise<any> => {
        return tauriInvoke("application_custom_connector_save", { connector });
      },
      deleteCustomConnector: async (connectorId: string): Promise<any> => {
        return tauriInvoke("application_custom_connector_delete", { connectorId });
      },
      probeCustomConnectorTools: async (connectorId: string): Promise<any> => {
        return tauriInvoke("application_custom_connector_probe_tools", { connectorId });
      },
      saveCustomSkill: async (skill: any): Promise<any> => {
        return tauriInvoke("application_custom_skill_save", { skill });
      },
      deleteCustomSkill: async (skillId: string): Promise<any> => {
        return tauriInvoke("application_custom_skill_delete", { skillId });
      },
      installCustomSkillFromUrl: async (url: string, fallbackName?: string | null): Promise<any> => {
        return tauriInvoke("application_install_custom_skill_from_url", {
          url,
          fallbackName: fallbackName ?? null,
        });
      },
      installCustomSkillFromFolder: async (folderPath: string): Promise<any> => {
        return tauriInvoke("application_install_custom_skill_from_folder", { folderPath });
      },
      getUsageStatistics: async (): Promise<any> => {
        return tauriInvoke("application_usage_statistics");
      },
      gisRasterInspect: async (request: any): Promise<any> => {
        return tauriInvoke("application_gis_raster_inspect", { request });
      },
      gisRasterPixelProbe: async (request: { source: string; lon: number; lat: number }): Promise<any> => {
        return tauriInvoke("application_gis_raster_pixel_probe", { request });
      },
      gisRasterProcess: async (request: any): Promise<any> => {
        return tauriInvoke("application_gis_raster_process", { request });
      },
      gisVectorProcess: async (request: any): Promise<any> => {
        return tauriInvoke("application_gis_vector_process", { request });
      },
      externalGisInspect: async (request?: any): Promise<any> => {
        return tauriInvoke("application_external_gis_inspect", {
          request: request ?? null,
        });
      },
      externalGisSavePrefs: async (prefs: any): Promise<any> => {
        return tauriInvoke("application_external_gis_save_prefs", { prefs });
      },
      externalGisExecute: async (request: any): Promise<any> => {
        return tauriInvoke("application_external_gis_execute", { request });
      },
      condaInstallGeoPackage: async (request: {
        package_id: string;
        conda_env?: string | null;
        confirmed: boolean;
      }): Promise<any> => {
        return tauriInvoke("application_conda_install_geo_package", { request });
      },
      lbsCoordTransform: async (request: any): Promise<any> => {
        return tauriInvoke("application_lbs_coord_transform", { request });
      },
      mapgisInspect: async (request: any): Promise<any> => {
        return tauriInvoke("application_mapgis_inspect", { request });
      },
      mapgisConvert: async (request: any): Promise<any> => {
        return tauriInvoke("application_mapgis_convert", { request });
      },
      experimentProjectGet: async (): Promise<any> => {
        return tauriInvoke("application_experiment_project_get");
      },
      experimentProjectAddTask: async (taskId: string): Promise<any> => {
        return tauriInvoke("application_experiment_project_add_task", { taskId });
      },
      experimentProjectRemoveItem: async (itemId: string): Promise<any> => {
        return tauriInvoke("application_experiment_project_remove_item", { itemId });
      },
      experimentProjectExport: async (exportDir?: string | null): Promise<any> => {
        return tauriInvoke("application_experiment_project_export", {
          exportDir: exportDir ?? null,
        });
      },
      openGeodataQuery: async (request: any): Promise<any> => {
        return tauriInvoke("application_open_geodata_query", { request });
      },
      fetchGeodataPreviewDataUrl: async (
        url: string,
      ): Promise<{ ok: boolean; data_url?: string; error?: string }> => {
        const raw = (await dependencies.invoke("geodata_fetch_preview_data_url", {
          url,
        })) as Record<string, unknown> | null | undefined;
        if (raw && raw.ok === true && typeof raw.data_url === "string" && raw.data_url) {
          return { ok: true, data_url: raw.data_url };
        }
        return {
          ok: false,
          error: typeof raw?.error === "string" ? raw.error : "PREVIEW_FETCH_FAILED",
        };
      },
      desktopOnboardingStateGet: async (): Promise<{ completed: boolean; returning_user?: boolean }> => {
        return (await dependencies.invoke("desktop_onboarding_state_get")) as {
          completed: boolean;
          returning_user?: boolean;
        };
      },
      desktopOnboardingStateSet: async (completed: boolean): Promise<{ completed: boolean }> => {
        return (await dependencies.invoke("desktop_onboarding_state_set", { completed })) as {
          completed: boolean;
        };
      },
      desktopActiveResourceGet: async (): Promise<string | null> => {
        return ((await dependencies.invoke("desktop_active_resource_get")) as string | null) ?? null;
      },
      desktopActiveResourceSet: async (resourceId: string): Promise<string | null> => {
        return (
          ((await dependencies.invoke("desktop_active_resource_set", {
            resourceId,
          })) as string | null) ?? null
        );
      },
      getCacheSummary: async (): Promise<{ ok?: boolean; cache_dir?: string } | null> => {
        return ((await dependencies.invoke("get_cache_summary")) as {
          ok?: boolean;
          cache_dir?: string;
        } | null) ?? null;
      },
      diagnosticsGetState: async () => {
        return (await dependencies.invoke("shell_diagnostics_get_state")) as {
          enabled: boolean;
          total_count: number;
          pending_count: number;
        };
      },
      diagnosticsSetEnabled: async (enabled: boolean) => {
        return (await dependencies.invoke("shell_diagnostics_set_enabled", { enabled })) as {
          enabled: boolean;
          total_count: number;
          pending_count: number;
        };
      },
      diagnosticsRecord: async (input: {
        category: string;
        error_code?: string | null;
        http_status?: number | null;
        source?: string | null;
        message: string;
        stack?: string | null;
      }) => {
        return await dependencies.invoke("shell_diagnostics_record", { input });
      },
      diagnosticsList: async () => {
        return (await dependencies.invoke("shell_diagnostics_list")) as any[];
      },
      diagnosticsClear: async () => {
        return (await dependencies.invoke("shell_diagnostics_clear")) as {
          enabled: boolean;
          total_count: number;
          pending_count: number;
        };
      },
      diagnosticsUpload: async () => {
        return (await dependencies.invoke("shell_diagnostics_upload")) as {
          attempted: number;
          uploaded: number;
          failed: number;
          message_zh: string;
        };
      },
      checkAppUpdate: async () => {
        return (await dependencies.invoke("shell_check_app_update")) as {
          current_version: string;
          latest_version: string | null;
          has_update: boolean;
          release_url: string | null;
          release_notes: string | null;
          checked_at: string;
          status: string;
          message_zh: string;
        };
      },
    getReadiness: async (): Promise<DesktopApplicationReadiness> => {
        try {
          const value = (await dependencies.invoke("application_service_readiness")) as Record<
            string,
            unknown
          >;
          if (
            typeof value.contract !== "string" ||
            typeof value.status !== "string" ||
            (value.store_schema_version !== null &&
              typeof value.store_schema_version !== "number")
          ) {
            return applicationFailed();
          }
          return {
            ok: true,
            contract: value.contract,
            status: value.status,
            store_schema_version: value.store_schema_version as number | null,
          };
        } catch {
          return applicationFailed();
        }
      },
      coreRestore: async (includeTerminal = false): Promise<DesktopCoreRestoreSnapshot> =>
        boundedCoreRestore("application_core_restore", { includeTerminal }),
      sarSearchProgress,
      workbenchPage: (request: DesktopWorkbenchPageRequest): Promise<DesktopWorkbenchPageSnapshot> =>
        boundedWorkbenchPage(request),
      taskPage: (request: DesktopTaskPageRequest): Promise<DesktopTaskPageSnapshot> =>
        boundedTaskPage(request),
      taskSourceBatchPage: (
        request: DesktopTaskSourceBatchPageRequest,
      ): Promise<DesktopTaskSourceBatchPageSnapshot> => boundedTaskSourceBatchPage(request),
      taskHistoryPage: (request: DesktopTaskHistoryPageRequest): Promise<DesktopTaskHistoryPageSnapshot> =>
        boundedTaskHistoryPage(request),
      taskEventPage: (request: DesktopTaskEventPageRequest): Promise<DesktopTaskEventPageSnapshot> =>
        boundedTaskEventPage(request),
      openTaskOutput: (request: DesktopOpenTaskOutputRequest): Promise<DesktopOpenTaskOutputResult> =>
        openTaskOutput(request),
      listTaskConflictFiles: (
        request: DesktopListTaskConflictFilesRequest,
      ): Promise<DesktopTaskConflictFileListResult> => listTaskConflictFiles(request),
      deleteTaskConflictFiles: (
        request: DesktopDeleteTaskConflictFilesRequest,
      ): Promise<DesktopTaskConflictFileDeleteResult> => deleteTaskConflictFiles(request),
      updateWorkbenchSelection: (
        command: DesktopWorkbenchSelectionCommand,
      ): Promise<DesktopWorkbenchSelectionResult> => selectionSummary(command),
      createTaskFromWorkbench: (request: DesktopCreateTaskFromWorkbenchRequest) =>
        boundedCoreRestore("application_create_task_from_workbench", { request }),
      appendTaskFromWorkbench: (request: DesktopAppendTaskFromWorkbenchRequest) =>
        boundedCoreRestore("application_append_task_from_workbench", { request }),
      appendTaskFromSourceBatch: (request: DesktopAppendTaskFromSourceBatchRequest) =>
        boundedCoreRestore("application_append_task_from_source_batch", { request }),
      createTaskFromHistoricalSourceBatch: (
        request: DesktopCreateTaskFromHistoricalSourceBatchRequest,
      ) => boundedCoreRestore("application_create_task_from_historical_source_batch", { request }),
      createBurstAssetTask: (request: DesktopBurstCreateAssetTaskRequest) =>
        boundedCoreRestore("application_create_burst_asset_task", { request }),
      appendBurstAssetTask: (request: DesktopBurstAppendAssetTaskRequest) =>
        boundedCoreRestore("application_append_burst_asset_task", { request }),
      createHistoricalBurstAssetTask: (
        request: DesktopBurstCreateHistoricalAssetTaskRequest,
      ) => boundedCoreRestore("application_create_historical_burst_asset_task", { request }),
      workspacePrepareLocation,
      workspaceOpen: (request: DesktopWorkspaceOpenRequest) =>
        workspaceContext("application_workspace_open", request),
      workspaceCreate: (request: DesktopWorkspaceCreateRequest) =>
        workspaceContext("application_workspace_create", request),
      projectCreate: (request: DesktopWorkspaceNamedNodeRequest) =>
        workspaceContext("application_project_create", request),
      projectSelect: (request: DesktopWorkspaceSelectNodeRequest) =>
        workspaceContext("application_project_select", request),
      regionCreate: (request: DesktopWorkspaceNamedNodeRequest) =>
        workspaceContext("application_region_create", request),
      regionSelect: (request: DesktopWorkspaceSelectNodeRequest) =>
        workspaceContext("application_region_select", request),
      planDemTask,
      createDemTask: (request: DesktopDemTaskRequest) =>
        coreSnapshot("application_create_dem_task", { request }),
      setDemSourceIdentity: async (
        identity: DesktopDemSourceIdentity,
      ): Promise<DesktopDemSourceIdentityResult> => {
        if (!parseDemSourceIdentity(identity)) return applicationFailed();
        try {
          const confirmed = parseDemSourceIdentity(
            await dependencies.invoke("application_set_dem_source_identity", { identity }),
          );
          if (!confirmed || confirmed.dataset !== identity.dataset ||
              confirmed.provider !== identity.provider || confirmed.updated_at !== identity.updated_at) {
            return applicationFailed();
          }
          return { ok: true, identity: confirmed };
        } catch (error) {
          return applicationFailed(error);
        }
      },
      resolveDemConversionSource: async (
        request: DesktopDemConversionSourceRequest,
      ): Promise<DesktopDemConversionSourceResult> => {
        if ([
          request.region_id,
          request.dataset,
          request.provider,
          request.resource_id,
          request.capability_id,
        ].some((value) => !validTaskHistoryTaskId(value))) return applicationFailed();
        try {
          const result = parseDemConversionSource(
            await dependencies.invoke("application_resolve_dem_conversion_source", { request }),
          );
          if (
            result.ok && result.source &&
            (result.source.region_id !== request.region_id ||
              result.source.dataset !== request.dataset ||
              result.source.provider !== request.provider ||
              result.source.resource_id !== request.resource_id ||
              result.source.capability_id !== request.capability_id)
          ) return applicationFailed();
          return result;
        } catch (error) {
          return applicationFailed(error);
        }
      },
      handoffDemConversion: (request: DesktopDemConversionRequest) =>
        coreSnapshot("application_handoff_dem_conversion", { request }),
      createCompanionOrbitTask: (sourceTaskId: string) =>
        coreSnapshot("application_create_companion_orbit_task", { sourceTaskId }),
      createOrbitTask: (request: DesktopOrbitTaskRequest) =>
        coreSnapshot("application_create_orbit_task", { request }),
      createStandaloneOrbitTask: (request: DesktopStandaloneOrbitTaskRequest) =>
        coreSnapshot("application_create_standalone_orbit_task", { request }),
      providerCapabilities: async (): Promise<DesktopProviderCapabilities> => {
        try {
          const capabilities = asRecord(
            await dependencies.invoke("application_provider_capabilities"),
          );
          if (!capabilities || typeof capabilities.provider_id !== "string") {
            return applicationFailed();
          }
          return { ok: true, capabilities };
        } catch {
          return applicationFailed();
        }
      },
      aoiImport: (request: DesktopAoiImportRequest) =>
        nativeJson("application_aoi_import", { request }),
      aoiNormalize: (request: DesktopAoiNormalizeRequest) =>
        nativeJson("application_aoi_normalize", { request }),
      adminBoundarySearch: (request: DesktopAdminBoundarySearchRequest) =>
        nativeJson("application_admin_boundary_search", { request }),
      adminBoundaryResolve: (request: DesktopAdminBoundaryResolveRequest) =>
        nativeJson("application_admin_boundary_resolve", { request }),
      adminOptions: (request: DesktopAdminBoundaryOptionsRequest) =>
        nativeJson("application_admin_options", { request }),
      gacosPlan: (payload: DesktopJsonObject) =>
        nativeJson("application_gacos_plan", { payload }),
      gacosDryRun: (payload: DesktopJsonObject) =>
        nativeJson("application_gacos_dry_run", { payload }),
      getDemTaskSettings: demTaskSettings,
      saveDemTaskSettings,
      getSarDownloadPolicySettings,
      saveSarDownloadPolicySettings,
      openExternalUrl,
      standaloneDem,
      createStandaloneDemTask,
      importScenes,
      matchOrbitsDirectory,
      gacosLedger,
      gacosImportResult,
      gacosCreateImportTask,
      gacosCreateResultTask,
      gacosSupplyResultLink,
      gacosSubmitRequest,
      gacosRequestStatus,
      getNetworkSettings,
      saveNetworkSettings,
      probeProviderConnectivity,
      reportGenerate: (payload: DesktopJsonObject) =>
        nativeJson("application_report_generate", { payload }),
      bindRegionAoi: (request: DesktopBindRegionAoiRequest) =>
        coreSnapshot("application_bind_region_aoi", { request }),
      clearRegionAoi: (request: DesktopClearRegionAoiRequest) =>
        coreSnapshot("application_clear_region_aoi", { request }),
      setWorkingAoi,
      clearWorkingAoi,
      getWorkingAoi,
      sarSearchAndFreeze: (request: DesktopSarSearchAndFreezeRequest) =>
        coreSnapshot("application_sar_search_and_freeze", { request }),
      sarBackgroundCount: async (
        request: DesktopSarBackgroundCountRequest,
      ): Promise<DesktopSarBackgroundCountResult> => {
        try {
          const value = asRecord(await dependencies.invoke("application_sar_background_count", { request }));
          const totalCount = value?.total_count;
          const snapshot = value?.snapshot === null ? null : parseCoreSnapshotValue(value?.snapshot);
          if (
            !value ||
            typeof value.applied !== "boolean" ||
            (totalCount !== null && (
              typeof totalCount !== "number" || !Number.isSafeInteger(totalCount) || totalCount < 0
            )) ||
            (snapshot !== null && !snapshot.ok) ||
            (value.applied && snapshot === null)
          ) {
            return applicationFailed();
          }
          return {
            ok: true,
            applied: value.applied,
            total_count: totalCount as number | null,
            snapshot,
          };
        } catch {
          return applicationFailed();
        }
      },
      catalogSearchAndFreeze: (request: DesktopCatalogSearchAndFreezeRequest) =>
        coreSnapshot("application_catalog_search_and_freeze", { request }),
      catalogIntentSearchAndFreeze: (request: DesktopCatalogIntentSearchAndFreezeRequest) =>
        coreSnapshot("application_catalog_intent_search_and_freeze", { request }),
      cancelRequest: async (requestId: string) => {
        try {
          const cancelled = await dependencies.invoke("application_cancel_request", { requestId });
          return typeof cancelled === "boolean"
            ? { ok: true as const, cancelled }
            : applicationFailed();
        } catch {
          return applicationFailed();
        }
      },
      cdseSessionStatus: () => cdseStatus("application_cdse_session_status"),
      cdseLogin: (request: DesktopCdseLoginRequest) =>
        cdseStatus("application_cdse_login", { request }),
      cdseLogout: () => cdseStatus("application_cdse_logout"),
      cdseNetworkPolicy,
      setCdseNetworkPolicy: (networkMode: DesktopCdseNetworkMode) =>
        cdseStatus("application_set_cdse_network_policy", { mode: networkMode }),
      cdseNetworkDiagnostic,
      previewAsset,
      savePreview,
      cdseQuicklook,
      sentinel2MapPreview,
      usgsSessionStatus: () => usgsStatus("application_usgs_session_status"),
      usgsLogin: (request: DesktopUsgsM2mLoginRequest) =>
        usgsStatus("application_usgs_login", { request }),
      usgsLogout: () => usgsStatus("application_usgs_logout"),
      landsatBrowse,
      landsatAssetDetails,
      bhoonidhiAssetSize,
      createLandsatBundleTask: (request: DesktopCreateLandsatBundleTaskRequest) =>
        coreSnapshot("application_create_landsat_bundle_task", { request }),
      createLandsatAssetTask: (request: DesktopCreateLandsatAssetTaskRequest) =>
        coreSnapshot("application_create_landsat_asset_task", { request }),
      createLandsatAssetBatchTask: async (request: DesktopCreateLandsatAssetBatchTaskRequest): Promise<DesktopLandsatAssetBatchTaskResult> => {
        if (!validOpticalBatchIds(request.item_ids) || !validNisarRevision(request.expected_workbench_revision)) return applicationFailed();
        try {
          return parseOpticalAssetBatchTaskResult(await dependencies.invoke("application_create_landsat_asset_batch_task", { request }), request.item_ids);
        } catch (error) { return applicationFailed(error); }
      },
      createLandsatBundleBatchTask: async (request: DesktopCreateLandsatAssetBatchTaskRequest): Promise<DesktopLandsatAssetBatchTaskResult> => {
        if (!validOpticalBatchIds(request.item_ids) || !validNisarRevision(request.expected_workbench_revision)) return applicationFailed();
        try {
          const result = parseOpticalAssetBatchTaskResult(await dependencies.invoke("application_create_landsat_bundle_batch_task", { request }), request.item_ids);
          if (!result.ok || result.snapshot === null) return result;
          if (!result.snapshot.ok) return applicationFailed(result.snapshot);
          const tasks = result.snapshot.core.tasks;
          const matches = Array.isArray(tasks) ? tasks.map(asRecord).filter(task => task?.task_id === request.task_id) : [];
          const task = matches[0];
          if (matches.length !== 1 || !task || task.batch_id !== `landsat-bundles-batch-${request.task_id}`
            || (task.provider_id !== "usgs.eros.m2m" && task.provider_id !== "usgs.m2m")
            || task.dataset_id !== "optical.landsat.c2.l2"
            || task.selected_count !== result.admission.admitted_count
            || task.delivery_requested_count !== result.admission.requested_count
            || task.delivery_rejected_count !== result.admission.rejected_count) return applicationFailed();
          return result;
        } catch (error) { return applicationFailed(error); }
      },
      taskDeliveryAdmission: async (taskId: string): Promise<DesktopTaskDeliveryAdmissionResult> => {
        try {
          const raw = await dependencies.invoke("application_task_delivery_admission", { taskId });
          if (raw === null) return { ok: true, admission: null };
          const admission = parseOpticalDeliveryAdmission(raw);
          return admission ? { ok: true, admission } : applicationFailed(raw);
        } catch (error) { return applicationFailed(error); }
      },
      opticalPreparationProgress: async (requestId: string): Promise<OpticalPreparationResult> => {
        if (!validBoundedDesktopScalar(requestId, 2048)) return applicationFailed();
        try {
          const raw = await dependencies.invoke("application_optical_preparation_progress", { requestId });
          if (raw === null) return { ok: true, progress: null };
          const progress = parseOpticalPreparation(raw, requestId);
          return progress ? { ok: true, progress } : applicationFailed();
        } catch (error) { return applicationFailed(error); }
      },
      createReviewedMpcAssetTask: (request: DesktopCreateReviewedMpcAssetTaskRequest) =>
        coreSnapshot("application_create_reviewed_mpc_asset_task", { request }),
      createSentinel2ArchiveTask: (request: DesktopCreateSentinel2ArchiveTaskRequest) =>
        coreSnapshot("application_create_sentinel2_archive_task", { request }),
      createSentinel2ArchiveBatchTask: async (
        request: DesktopCreateSentinel2ArchiveBatchTaskRequest,
      ): Promise<DesktopSentinel2ArchiveBatchTaskResult> => {
        if (!validOpticalBatchIds(request.item_ids) || !validNisarRevision(request.expected_workbench_revision)) return applicationFailed();
        try {
          return parseSentinel2ArchiveBatchTaskResult(
            await dependencies.invoke("application_create_sentinel2_archive_batch_task", { request }),
            request,
          );
        } catch (error) {
          return applicationFailed(error);
        }
      },
      createSentinel2BandTask: (request: DesktopCreateSentinel2BandTaskRequest) =>
        coreSnapshot("application_create_sentinel2_band_task", { request }),
      createSentinel2BandBatchTask: async (request: DesktopCreateSentinel2BandBatchTaskRequest): Promise<DesktopLandsatAssetBatchTaskResult> => {
        if (!validOpticalBatchIds(request.item_ids) || !validNisarRevision(request.expected_workbench_revision)) return applicationFailed();
        if (!Array.isArray(request.asset_keys) || !validSentinel2BandKeys(request.asset_keys)) return applicationFailed();
        try {
          const result = parseOpticalAssetBatchTaskResult(await dependencies.invoke("application_create_sentinel2_band_batch_task", { request }), request.item_ids);
          if (!result.ok || result.snapshot === null) return result;
          if (!result.snapshot.ok) return applicationFailed(result.snapshot);
          const tasks = result.snapshot.core.tasks;
          const matches = Array.isArray(tasks) ? tasks.map(asRecord).filter(task => task?.task_id === request.task_id) : [];
          const task = matches[0];
          if (matches.length !== 1 || !task || task.batch_id !== `sentinel2-bands-batch-${request.task_id}`
            || (task.provider_id !== "microsoft.planetary-computer" && task.provider_id !== "element84.earth-search")
            || task.dataset_id !== "optical.sentinel-2.l2a"
            || task.selected_count !== result.admission.admitted_count * request.asset_keys.length
            || task.delivery_requested_count !== result.admission.requested_count
            || task.delivery_rejected_count !== result.admission.rejected_count) return applicationFailed();
          return result;
        } catch (error) { return applicationFailed(error); }
      },
      createSentinel2L1cBandBatchTask: async (request: DesktopCreateSentinel2BandBatchTaskRequest): Promise<DesktopLandsatAssetBatchTaskResult> => {
        if (!validOpticalBatchIds(request.item_ids) || !validNisarRevision(request.expected_workbench_revision)) return applicationFailed();
        if (!Array.isArray(request.asset_keys) || !validSentinel2L1cBandKeys(request.asset_keys)) return applicationFailed();
        try {
          const result = parseOpticalAssetBatchTaskResult(await dependencies.invoke("application_create_sentinel2_l1c_band_batch_task", { request }), request.item_ids);
          if (!result.ok || result.snapshot === null) return result;
          if (!result.snapshot.ok) return applicationFailed(result.snapshot);
          const tasks = result.snapshot.core.tasks;
          const matches = Array.isArray(tasks) ? tasks.map(asRecord).filter(task => task?.task_id === request.task_id) : [];
          const task = matches[0];
          if (matches.length !== 1 || !task || task.batch_id !== `sentinel2-l1c-bands-batch-${request.task_id}`
            || task.provider_id !== "copernicus.dataspace.odata" || task.dataset_id !== "optical.sentinel-2.l1c"
            || task.selected_count !== result.admission.admitted_count * request.asset_keys.length
            || task.delivery_requested_count !== result.admission.requested_count
            || task.delivery_rejected_count !== result.admission.rejected_count) return applicationFailed();
          return result;
        } catch (error) { return applicationFailed(error); }
      },
      createHlsBandBatchTask: async (request: DesktopCreateHlsBandBatchTaskRequest): Promise<DesktopHlsBandBatchTaskResult> => {
        const hlsAllowedBands = new Set(["B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B10", "B11", "B12", "Fmask"]);
        if (!validOpticalBatchIds(request.item_ids) || !validNisarRevision(request.expected_workbench_revision)) return applicationFailed();
        if (
          !Array.isArray(request.asset_keys)
          || request.asset_keys.length < 1
          || request.asset_keys.length > 14
          || new Set(request.asset_keys).size !== request.asset_keys.length
          || request.asset_keys.some((key) => typeof key !== "string" || !hlsAllowedBands.has(key))
        ) return applicationFailed();
        try {
          const result = parseOpticalAssetBatchTaskResult(await dependencies.invoke("application_create_hls_band_batch_task", { request }), request.item_ids);
          if (!result.ok || result.snapshot === null) return result;
          if (!result.snapshot.ok) return applicationFailed(result.snapshot);
          const tasks = result.snapshot.core.tasks;
          const matches = Array.isArray(tasks) ? tasks.map(asRecord).filter(task => task?.task_id === request.task_id) : [];
          const task = matches[0];
          if (matches.length !== 1 || !task || task.batch_id !== `hls-bands-batch-${request.task_id}`
            || task.provider_id !== "microsoft.planetary-computer"
            || (task.dataset_id !== "optical.hls.l30.v2" && task.dataset_id !== "optical.hls.s30.v2")
            || task.selected_count !== result.admission.admitted_count * request.asset_keys.length
            || task.delivery_requested_count !== result.admission.requested_count
            || task.delivery_rejected_count !== result.admission.rejected_count) return applicationFailed();
          return result;
        } catch (error) { return applicationFailed(error); }
      },
      precheckOpticalAoiCoverage: async (
        request: DesktopOpticalDerivedPrecheckBatchRequest,
      ) => {
        if (
          !validBoundedDesktopScalar(request.source_batch_id, 256)
          || !validNisarRevision(request.expected_workbench_revision)
          || !validOpticalBatchIds(request.item_ids)
          || request.item_ids.length < 1
          || request.item_ids.length > 64
          || (request.target_resolution_m !== undefined
            && request.target_resolution_m !== null
            && (typeof request.target_resolution_m !== "number"
              || !Number.isFinite(request.target_resolution_m)
              || request.target_resolution_m <= 0))
        ) {
          return applicationFailed();
        }
        try {
          const raw = await dependencies.invoke("application_precheck_optical_aoi_coverage", { request });
          const report = parseOpticalPrecheckReport(raw);
          return report ? { ok: true, report } : applicationFailed(raw);
        } catch (error) {
          return applicationFailed(error);
        }
      },
      createOpticalDerivedTask: async (
        request: DesktopOpticalDerivedCreateTaskRequest,
      ) => {
        if (
          !validBoundedDesktopScalar(request.request_id, 80)
          || !validBoundedDesktopScalar(request.task_id, 72)
          || !validBoundedDesktopScalar(request.source_batch_id, 256)
          || !validNisarRevision(request.expected_workbench_revision)
          || !validOpticalBatchIds(request.item_ids)
          || request.item_ids.length < 1
          || request.item_ids.length > 64
          || !Array.isArray(request.band_order)
          || request.band_order.length < 1
          || request.band_order.length > 16
          || new Set(request.band_order).size !== request.band_order.length
          || request.band_order.some((b) => typeof b !== "string" || !validBoundedDesktopScalar(b, 24))
          || !validBoundedDesktopScalar(request.output_root, 4096)
          || typeof request.concurrency !== "number"
          || !Number.isSafeInteger(request.concurrency)
          || request.concurrency < 1
          || request.concurrency > 8
          || !validBoundedDesktopScalar(request.created_at, 64)
        ) {
          return applicationFailed();
        }
        try {
          const raw = await dependencies.invoke("application_create_optical_derived_task", { request });
          return parseOpticalDerivedTaskCreatedResult(raw, request);
        } catch (error) {
          return applicationFailed(error);
        }
      },
      effectiveCapability: async (capabilityId: string): Promise<DesktopEffectiveCapabilityResult> => {
        try {
          const status = parseEffectiveCapabilityStatus(
            await dependencies.invoke("application_effective_capability", { capabilityId }),
            capabilityId,
          );
          return status ? { ok: true, status } : applicationFailed();
        } catch (error) {
          return applicationFailed(error);
        }
      },
      nisarRemoteSubsetCapability: async (): Promise<DesktopNisarRemoteSubsetCapabilityResult> => {
        try {
          const capability = parseNisarRemoteSubsetCapability(
            await dependencies.invoke("application_nisar_remote_subset_capability"),
          );
          return capability ? { ok: true, capability } : applicationFailed();
        } catch (error) {
          return applicationFailed(error);
        }
      },
      createNisarRemoteSubsetTask: (request: DesktopCreateNisarRemoteSubsetTaskRequest) =>
        coreSnapshot("application_create_nisar_remote_subset_task", { request }),
      prepareNisarSubsetIntentV2: async (
        request: DesktopNisarPrepareSubsetIntentV2Request,
      ): Promise<DesktopNisarPreparedSubsetIntentV2Result> => {
        try {
          return parseNisarPreparedSubsetIntentV2Result(
            await dependencies.invoke("application_prepare_nisar_subset_intent_v2", { request }),
          );
        } catch (error) {
          return applicationFailed(error);
        }
      },
      preflightNisarSubsetBatchV2: async (
        request: DesktopNisarPreflightSubsetBatchV2Request,
      ): Promise<DesktopNisarPreflightSubsetBatchV2Result> => {
        if (!validNisarPreflightBatchRequest(request)) return nisarSubsetRequestFailed();
        try {
          return parseNisarPreflightSubsetBatchV2Result(
            await dependencies.invoke("application_preflight_nisar_subset_batch_v2", { request }),
            request,
          );
        } catch (error) {
          return applicationFailed(error);
        }
      },
      prepareNisarSubsetBatchV2: async (
        request: DesktopNisarPrepareSubsetBatchV2Request,
      ): Promise<DesktopNisarPreparedSubsetBatchV2Result> => {
        if (!validNisarPrepareBatchRequest(request)) return nisarSubsetRequestFailed();
        try {
          return parseNisarPreparedSubsetBatchV2Result(
            await dependencies.invoke("application_prepare_nisar_subset_batch_v2", { request }),
            request,
          );
        } catch (error) {
          return applicationFailed(error);
        }
      },
      confirmNisarSubsetTaskV2: (request: DesktopNisarConfirmSubsetTaskV2Request) =>
        coreSnapshot("application_confirm_nisar_subset_task_v2", { request }),
      confirmNisarSubsetBatchTaskV2: async (
        request: DesktopNisarConfirmSubsetBatchTaskV2Request,
      ): Promise<DesktopCoreRestoreSnapshot> => {
        if (!validNisarConfirmBatchRequest(request)) return nisarSubsetRequestFailed();
        return coreSnapshot("application_confirm_nisar_subset_batch_task_v2", { request });
      },
      appendNisarSubsetBatchTaskV2: (request: DesktopNisarAppendSubsetBatchTaskV2Request) => {
        if (!validNisarAppendBatchRequest(request)) return Promise.resolve(nisarSubsetRequestFailed());
        return coreSnapshot("application_append_nisar_subset_batch_task_v2", { request });
      },
      readNisarSubsetAppendContextV2: async (request: DesktopNisarAppendContextV2Request): Promise<DesktopNisarAppendContextV2Result> => {
        if (!asRecord(request) || !hasOnlyKeys(request, ["task_id"]) || typeof request.task_id !== "string" || !request.task_id.trim()) return nisarSubsetRequestFailed();
        try {
          const context = parseNisarAppendContext(await dependencies.invoke("application_read_nisar_subset_append_context_v2", { request }), request.task_id);
          return context ? { ok: true, context } : applicationFailed();
        } catch (error) { return applicationFailed(error); }
      },
      preflightNisarSubsetAppendV2: async (request: DesktopNisarAppendPreflightV2Request): Promise<DesktopNisarAppendPreflightV2Result> => {
        if (!asRecord(request) || !hasOnlyKeys(request, ["task_id", "expected_task_revision", "source_item_ids"])
          || typeof request.task_id !== "string" || !request.task_id.trim() || !validNisarRevision(request.expected_task_revision)
          || !validNisarBatchIds(request.source_item_ids)) return nisarSubsetRequestFailed();
        try {
          const preflight = parseNisarAppendPreflight(await dependencies.invoke("application_preflight_nisar_subset_append_v2", { request }), request);
          return preflight ? { ok: true, preflight } : applicationFailed();
        } catch (error) { return applicationFailed(error); }
      },
      createNisarSubsetSuccessorTaskV2: (request: DesktopNisarSuccessorTaskV2Request) => {
        const record = asRecord(request);
        if (!record || !hasOnlyKeys(record, ["source_task_id", "expected_task_revision", "source_item_ids", "new_task_id", "output_root", "layout", "concurrency", "created_at", ...(record.recreate_original_items === undefined ? [] : ["recreate_original_items"])])
          || typeof request.source_task_id !== "string" || !request.source_task_id.trim()
          || typeof request.new_task_id !== "string" || !request.new_task_id.trim() || request.new_task_id === request.source_task_id
          || !validNisarRevision(request.expected_task_revision) || !validNisarBatchIds(request.source_item_ids)
          || !absoluteNisarOutputRoot(request.output_root) || !["direct", "sar_dataset_processing"].includes(request.layout)
          || !Number.isSafeInteger(request.concurrency) || request.concurrency < 1 || request.concurrency > 8
          || !validNisarCreatedAt(request.created_at)
          || (request.recreate_original_items !== undefined && typeof request.recreate_original_items !== "boolean")
          || hasForbiddenTransportField(request)) return Promise.resolve(nisarSubsetRequestFailed());
        return coreSnapshot("application_create_nisar_subset_successor_task_v2", { request });
      },
      resumeNisarSubsetTask: (taskId: string) =>
        coreSnapshot("application_resume_nisar_subset_task", { taskId }),
      readNisarArchivedRecoveryContextV2: async (sourceTaskId: string, newTaskId: string) => {
        if (!sourceTaskId.trim() || !/^[A-Za-z0-9_-]{1,128}$/.test(newTaskId) || sourceTaskId === newTaskId) return nisarSubsetRequestFailed();
        try {
          const context = asRecord(await dependencies.invoke("application_read_nisar_archived_recovery_context_v2", { sourceTaskId, newTaskId }));
          if (!context || !hasOnlyKeys(context, ["source_task_id", "source_item_id", "expected_source_revision", "new_task_id", "output_root"])
            || context.source_task_id !== sourceTaskId || context.new_task_id !== newTaskId
            || typeof context.source_item_id !== "string" || !context.source_item_id.trim()
            || !Number.isSafeInteger(context.expected_source_revision) || (context.expected_source_revision as number) < 0
            || typeof context.output_root !== "string" || !absoluteNisarOutputRoot(context.output_root)) return applicationFailed();
          return { ok: true, context: context as import("./desktopShellBridge").DesktopNisarArchivedRecoveryContextV2 };
        } catch (error) { return applicationFailed(error); }
      },
      createNisarArchivedRecoveryV2: (request: import("./desktopShellBridge").DesktopNisarArchivedRecoveryV2Request) => {
        const record = asRecord(request);
        if (!record || !hasOnlyKeys(record, ["source_task_id", "source_item_id", "expected_source_revision", "new_task_id", "created_at"])
          || typeof request.source_task_id !== "string" || !request.source_task_id.trim()
          || typeof request.source_item_id !== "string" || !request.source_item_id.trim()
          || !Number.isSafeInteger(request.expected_source_revision) || request.expected_source_revision < 0
          || typeof request.new_task_id !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(request.new_task_id) || request.new_task_id === request.source_task_id
          || !validNisarCreatedAt(request.created_at) || hasForbiddenTransportField(request)) return Promise.resolve(nisarSubsetRequestFailed());
        return coreSnapshot("application_create_nisar_archived_recovery_v2", { request });
      },
      startNewlyConfirmedTask: (taskId: string) =>
        coreSnapshot("application_start_newly_confirmed_task", { taskId }),
      updateSearchWorkbench: (request: DesktopUpdateSearchWorkbenchRequest) =>
        coreSnapshot("application_update_search_workbench", { request }),
      createDownloadTask: (request: DesktopCreateDownloadTaskRequest) =>
        coreSnapshot("application_create_download_task", { request }),
      appendDownloadTask: (request: DesktopAppendDownloadTaskRequest) =>
        coreSnapshot("application_append_download_task", { request }),
      updateDownloadTask: (request: DesktopUpdateDownloadTaskRequest) =>
        coreSnapshot("application_update_download_task", { request }),
      updateDownloadTaskView: (request: DesktopUpdateDownloadTaskViewRequest) =>
        coreSnapshot("application_update_download_task_view", { request }),
      archiveDownloadTask: async (request: DesktopArchiveDownloadTaskRequest) => {
        try {
          return parseArchiveDownloadTaskAck(
            await dependencies.invoke("application_archive_download_task", { request }),
            request,
          );
        } catch (error) {
          return applicationFailed(error);
        }
      },
      deleteDownloadHistory: (request: DesktopDeleteDownloadHistoryRequest) =>
        coreSnapshot("application_delete_download_history", { request }),
      querySourceAppendStatus,
      queryHistoricalSuccessorStatus,
      queryBurstAppendStatus,
      queryBurstSuccessorStatus,
      queryNisarSuccessorStatus,
      queryNisarArchivedRecoveryStatus,
      queryReacquireStatus,
      querySourceMutationStatus,
      unified: {
        listSources: () => dependencies.invoke("application_list_sources") as Promise<SourceDescriptor[]>,
        sourceViews: () => dependencies.invoke("application_source_views") as Promise<SourceView[]>,
        searchStart: (request: UnifiedSearchRequest, options?: { bypassCache?: boolean }) =>
          dependencies.invoke(
            "application_search_start",
            options?.bypassCache ? { request, bypassCache: true } : { request },
          ) as Promise<SearchProgress>,
        searchProgress: (requestId: string) => dependencies.invoke("application_search_progress", { requestId }).then((r) => (r as SearchProgress | null) ?? null),
        searchCancel: (requestId: string) => dependencies.invoke("application_search_cancel", { requestId }) as Promise<boolean>,
        cancelCatalogSearch: (requestId: string) =>
          dependencies.invoke("application_cancel_catalog_search", { requestId }) as Promise<SearchCancelOutcome>,
        batchPage: (batchId: string, revision: number, page: number, pageSize: number) =>
          dependencies.invoke("application_batch_page", { batchId, revision, page, pageSize }) as Promise<StoreBatchPage>,
        createTask: (batchId: string, revision: number, itemIds: string[], options: CreateTaskOptions) =>
          dependencies.invoke("application_create_task", { batchId, revision, itemIds, options }) as Promise<TaskRef>,
        previewAllMatchTask: async (request) => {
          const excluded = Array.isArray(request.excluded_item_ids) ? request.excluded_item_ids : [];
          const rawPredicate = request.predicate;
          const predicateVersion = typeof rawPredicate?.version === "number" ? rawPredicate.version : 1;
          const rustPredicate: Record<string, unknown> = {};
          if (rawPredicate?.orbit_direction) rustPredicate.orbit_direction = rawPredicate.orbit_direction;
          if (typeof rawPredicate?.relative_orbit === "number") rustPredicate.relative_orbit = rawPredicate.relative_orbit;
          const raw = (await dependencies.invoke("application_preview_all_match_task", {
            request: {
              batch_id: request.batch_id,
              revision: request.expected_revision,
              predicate_version: predicateVersion,
              predicate: rustPredicate,
              exclusions: excluded,
            },
          })) as Record<string, unknown>;
          const matchedCount = typeof raw.matched_count === "number" ? raw.matched_count : 0;
          const excludedCount = excluded.length;
          return {
            batch_id: typeof raw.batch_id === "string" ? raw.batch_id : request.batch_id,
            revision: typeof raw.revision === "number" ? raw.revision : request.expected_revision,
            batch_total_count: typeof raw.batch_total_count === "number" ? raw.batch_total_count : matchedCount + excludedCount,
            matched_count: matchedCount,
            excluded_count: typeof raw.excluded_count === "number" ? raw.excluded_count : excludedCount,
            skipped_active_count: typeof raw.skipped_active_count === "number" ? raw.skipped_active_count : 0,
            rejected_count: typeof raw.rejected_count === "number" ? raw.rejected_count : 0,
            admissible_count: typeof raw.admissible_count === "number" ? raw.admissible_count : 0,
            known_bytes: typeof raw.known_bytes === "number" ? raw.known_bytes : 0,
            unknown_size_count: typeof raw.unknown_size_count === "number" ? raw.unknown_size_count : 0,
            selection_digest: typeof raw.selection_digest === "string"
              ? raw.selection_digest
              : typeof raw.membership_digest === "string"
              ? raw.membership_digest
              : "",
          };
        },
        createAllMatchTask: async (request) => {
          const excluded = Array.isArray(request.excluded_item_ids) ? request.excluded_item_ids : [];
          const rawPredicate = request.predicate;
          const predicateVersion = typeof rawPredicate?.version === "number" ? rawPredicate.version : 1;
          const rustPredicate: Record<string, unknown> = {};
          if (rawPredicate?.orbit_direction) rustPredicate.orbit_direction = rawPredicate.orbit_direction;
          if (typeof rawPredicate?.relative_orbit === "number") rustPredicate.relative_orbit = rawPredicate.relative_orbit;
          const options: Record<string, unknown> = {
            output_root: request.output_root,
            layout: request.layout ?? "direct",
            concurrency: request.concurrency ?? 2,
          };
          if (typeof request.include_precise_orbit === "boolean") {
            options.include_precise_orbit = request.include_precise_orbit;
          }
          const raw = (await dependencies.invoke("application_create_all_match_task", {
            request: {
              batch_id: request.batch_id,
              revision: request.expected_revision,
              predicate_version: predicateVersion,
              predicate: rustPredicate,
              exclusions: excluded,
              expected_membership_digest: request.expected_selection_digest,
              options,
            },
          })) as Record<string, unknown>;
          const taskObj = (raw.task && typeof raw.task === "object" ? raw.task : raw) as Record<string, unknown>;
          return {
            task_id: typeof taskObj.task_id === "string" ? taskObj.task_id : "",
            revision: typeof taskObj.revision === "number" ? taskObj.revision : (typeof raw.revision === "number" ? raw.revision : 0),
            status: (typeof taskObj.status === "string" ? taskObj.status : "queued") as import("../shell/api/types.ts").TaskLifecycleStatus,
            created: typeof taskObj.created === "boolean" ? taskObj.created : true,
            warning: typeof taskObj.warning === "string" ? taskObj.warning : null,
            matched_count: typeof raw.matched_count === "number" ? raw.matched_count : 0,
            created_count: typeof raw.created_count === "number" ? raw.created_count : 0,
            pending_count: typeof raw.pending_count === "number" ? raw.pending_count : 0,
            skipped_count: typeof raw.skipped_count === "number" ? raw.skipped_count : 0,
            rejected_count: typeof raw.rejected_count === "number" ? raw.rejected_count : 0,
          };
        },
        taskAction: (taskId: string, revision: number, action: "pause" | "resume" | "retry" | "cancel" | "archive") =>
          dependencies.invoke("application_task_action", { taskId, revision, action }) as Promise<TaskRef>,
        taskItemAction: (
          taskId: string,
          revision: number | null,
          itemId: string,
          action: import("../shell/api/types.ts").UnifiedTaskItemAction,
        ) =>
          dependencies.invoke("application_task_item_action", {
            taskId,
            revision,
            itemId,
            action,
          }) as Promise<TaskRef>,
        taskDeleteHistory: (taskId: string) =>
          dependencies.invoke("application_task_delete_history", { taskId }) as Promise<void>,
        taskSetPinned: (taskId: string, pinned: boolean) =>
          dependencies.invoke("application_task_set_pinned", { taskId, pinned }) as Promise<string[]>,
        listPinnedTasks: () =>
          dependencies.invoke("application_list_pinned_tasks") as Promise<string[]>,
        taskRetryFallback: (taskId: string, confirmRefreshedBatchId: string | null) =>
          dependencies.invoke("application_task_retry_fallback", {
            taskId,
            confirmRefreshedBatchId,
          }) as Promise<TaskRetryFallbackOutcome>,
        taskRetryFallbackEligibility: (taskIds: string[]) =>
          dependencies.invoke("application_task_retry_fallback_eligibility", {
            taskIds,
          }) as Promise<Record<string, boolean>>,
        checkExistingDownloadedScenes: (
          items: import("../shell/api/types.ts").ExistingDownloadCheckItem[],
          targetDir: string | null = null,
        ) =>
          dependencies.invoke("check_existing_downloaded_scenes", {
            items,
            targetDir,
          }) as Promise<import("../shell/api/types.ts").ExistingDownloadMatch[]>,
        taskOverview: async () => {
          const raw = await dependencies.invoke("application_task_overview");
          return parseTaskOverview(raw);
        },
        accountSessionStatuses: async () => {
          const raw = await dependencies.invoke("application_account_session_statuses");
          return parseAccountSessionStatuses(raw);
        },
        testNetworkConnection: () => dependencies.invoke("application_test_network_connection") as Promise<string>,
      },
    },
    compatibility,
    agent: {
      sessionsList: (options?: { includeArchived?: boolean }) =>
        dependencies.invoke("agent_sessions_list", {
          includeArchived: options?.includeArchived ?? false,
        }) as Promise<AgentSessionSummary[]>,
      sessionsCreate: (title?: string | null, workspaceId?: string | null) =>
        dependencies.invoke("agent_sessions_create", {
          title: title ?? null,
          workspaceId: workspaceId ?? null,
        }) as Promise<AgentSessionSummary>,
      sessionsDelete: (sessionId: string) =>
        dependencies.invoke("agent_sessions_delete", { sessionId }) as Promise<void>,
      sessionsRename: (sessionId: string, title: string) =>
        dependencies.invoke("agent_sessions_rename", { sessionId, title }) as Promise<AgentSessionSummary>,
      sessionsSetPinned: (sessionId: string, pinned: boolean) =>
        dependencies.invoke("agent_sessions_set_pinned", { sessionId, pinned }) as Promise<AgentSessionSummary>,
      sessionsSetArchived: (sessionId: string, archived: boolean) =>
        dependencies.invoke("agent_sessions_set_archived", { sessionId, archived }) as Promise<AgentSessionSummary>,
      sessionsMove: (sessionId: string, workspaceId: string | null) =>
        dependencies.invoke("agent_sessions_move", { sessionId, workspaceId }) as Promise<AgentSessionSummary>,
      sessionsExport: (sessionId: string, format: AgentSessionExportFormat) =>
        dependencies.invoke("agent_sessions_export", { sessionId, format }) as Promise<AgentSessionExport>,
      sessionsExportTo: (sessionId: string, format: AgentSessionExportFormat, directory: string) =>
        dependencies.invoke("agent_sessions_export_to", { sessionId, format, directory }) as Promise<string>,
      workspacesList: () => dependencies.invoke("agent_workspaces_list") as Promise<AgentWorkspace[]>,
      workspacesCreate: (name: string, folder?: string | null) =>
        dependencies.invoke("agent_workspaces_create", { name, folder: folder ?? null }) as Promise<AgentWorkspace>,
      workspacesUpdate: (
        workspaceId: string,
        change: { name?: string; folder?: string | null },
      ) =>
        dependencies.invoke("agent_workspaces_update", {
          workspaceId,
          name: change.name ?? null,
          folder: typeof change.folder === "string" ? change.folder : null,
          clearFolder: change.folder === null,
        }) as Promise<AgentWorkspace>,
      workspacesDelete: (workspaceId: string) =>
        dependencies.invoke("agent_workspaces_delete", { workspaceId }) as Promise<void>,
      send: (sessionId: string, text: string, context?: AgentSendContext | null) =>
        dependencies.invoke("agent_send", { sessionId, text, context: context ?? null }) as Promise<string>,
      streamPoll: (sessionId: string, cursor: number) =>
        dependencies.invoke("agent_stream_poll", { sessionId, cursor }) as Promise<AgentStreamPage>,
      planApprove: (planId: string, snapshot: unknown, outputDir?: string | null) =>
        dependencies.invoke("agent_plan_approve", { planId, snapshot, outputDir: outputDir ?? null }) as Promise<void>,
      planReject: (planId: string) =>
        dependencies.invoke("agent_plan_reject", { planId }) as Promise<void>,
      runCancel: (runId: string) =>
        dependencies.invoke("agent_run_cancel", { runId }) as Promise<boolean>,
      runsList: (sessionId: string) =>
        dependencies.invoke("agent_runs_list", { sessionId }) as Promise<AgentRunSummary[]>,
      runsOpen: (runId: string) =>
        dependencies.invoke("agent_runs_open", { runId }) as Promise<AgentRunDetail>,
      sessionWork: (sessionId: string) =>
        dependencies.invoke("agent_session_work", { sessionId }) as Promise<AgentWorkItem[]>,
      sessionTruncate: (sessionId: string, beforeSeq: number, restoreAoi?: boolean) =>
        dependencies.invoke("agent_session_truncate", {
          sessionId,
          beforeSeq,
          restoreAoi: restoreAoi ?? true,
        }) as Promise<AgentSessionTruncateResult>,
      channelsGet: () => dependencies.invoke("agent_channels_get") as Promise<AgentChannelView[]>,
      channelsSave: (channel: AgentChannelSave) =>
        dependencies.invoke("agent_channels_save", { channel }) as Promise<AgentChannelView[]>,
      channelsTest: (channelId: string) =>
        dependencies.invoke("agent_channels_test", { channelId }) as Promise<AgentChannelTest>,
      channelsModels: (channelId: string) =>
        dependencies.invoke("agent_channels_models", { channelId }) as Promise<string[]>,
      channelsTestDraft: (draft) =>
        dependencies.invoke("agent_channels_test_draft", { draft }) as Promise<AgentChannelTest>,
      channelsProbeDraft: (draft) =>
        dependencies.invoke("agent_channels_probe_draft", { draft }) as Promise<{ result: AgentChannelTest; models: string[] }>,
      channelsSetDefault: (channelId: string) =>
        dependencies.invoke("agent_channels_set_default", { channelId }) as Promise<AgentChannelView[]>,
      channelsDelete: (channelId: string) =>
        dependencies.invoke("agent_channels_delete", { channelId }) as Promise<AgentChannelView[]>,
      channelsRename: (oldId: string, newId: string, apiKey?: string | null) =>
        dependencies.invoke("agent_channel_rename", {
          oldId,
          newId,
          apiKey: apiKey ?? null,
        }) as Promise<AgentChannelView[]>,
      disabledItemsGet: () =>
        dependencies.invoke("agent_disabled_items_get") as Promise<AgentDisabledItems>,
      disabledItemsSet: (items: AgentDisabledItems) =>
        dependencies.invoke("agent_disabled_items_set", { items }) as Promise<AgentDisabledItems>,
      permissionModeGet: (sessionId: string | null) =>
        dependencies.invoke("agent_permission_mode_get", { sessionId: sessionId ?? null }) as Promise<AgentPermissionModeView>,
      permissionModeSetGlobal: (mode: AgentPermissionMode) =>
        dependencies.invoke("agent_permission_mode_set_global", { mode }) as Promise<AgentPermissionModeView>,
      permissionModeSetSession: (sessionId: string, mode: AgentPermissionMode | null) =>
        dependencies.invoke("agent_permission_mode_set_session", { sessionId, mode: mode ?? null }) as Promise<AgentPermissionModeView>,
      stopAll: () =>
        dependencies.invoke("agent_stop_all") as Promise<AgentStopAllResult>,
      activityStatus: () =>
        dependencies.invoke("agent_activity_status") as Promise<AgentActivityStatus>,
    },
    app: {
      getInfo: async () => {
        try {
          const value = (await dependencies.invoke("shell_app_info")) as Record<
            string,
            unknown
          >;
          if (typeof value.name !== "string" || typeof value.version !== "string") {
            throw new Error("invalid app info");
          }
          return { name: value.name, version: value.version, offline: true };
        } catch {
          return { name: "EOStage", version: "unavailable", offline: true };
        }
      },
    },
    window: {
      minimize: () => run(() => dependencies.getCurrentWindow().minimize()),
      toggleMaximize: async () => {
        if (typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent)) {
          try {
            await dependencies.invoke("shell_window_toggle_maximize");
            return { ok: true };
          } catch {
            // Fall through to standard Tauri window API
          }
        }
        return run(() => dependencies.getCurrentWindow().toggleMaximize());
      },
      close: () => run(() => dependencies.getCurrentWindow().close()),
      getSize: async () => {
        try {
          const size = await dependencies.getCurrentWindow().innerSize();
          return { ok: true, width: size.width, height: size.height };
        } catch {
          return windowOperationFailed();
        }
      },
      startDragging: () =>
        run(async () => {
          const currentWindow = dependencies.getCurrentWindow();
          if (typeof currentWindow.startDragging === "function") {
            await currentWindow.startDragging();
          }
        }),
      resizeFromEdge: async (edge) => {
        const direction = resizeDirection(edge);
        if (!direction) return operationFailed();
        return run(() => dependencies.getCurrentWindow().startResizeDragging(direction));
      },
    },
    dialog: {
      pickOpenFile: (title, filters) => {
        const translatedFilters = parseDialogFilters(filters);
        if (translatedFilters === null) return Promise.resolve(pickFailed());
        return pick({
          title,
          multiple: false,
          directory: false,
          ...(translatedFilters ? { filters: translatedFilters } : {}),
        });
      },
      pickDirectory: (title) => pick({ title, multiple: false, directory: true }),
    },
  };
}
