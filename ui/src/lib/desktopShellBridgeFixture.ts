import type { DesktopShellBridge } from "./desktopShellBridge.ts";

export type BrowserFixtureHost = {
  close?: () => void;
  getSize?: () => { width: number; height: number };
};

function browserSize(): { width: number; height: number } {
  return {
    width: typeof window !== "undefined" ? window.innerWidth : 1320,
    height: typeof window !== "undefined" ? window.innerHeight : 880,
  };
}

function closeBrowserWindow(): void {
  if (typeof window !== "undefined") window.close();
}

export function createBrowserFixtureDesktopShellBridge(
  host: BrowserFixtureHost = {},
): DesktopShellBridge {
  return {
    kind: "browser_fixture",
    ready: async () => true,
    application: {
      querySourceMutationStatus: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      querySourceAppendStatus: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      queryHistoricalSuccessorStatus: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      queryBurstAppendStatus: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      queryBurstSuccessorStatus: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      queryNisarSuccessorStatus: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      queryNisarArchivedRecoveryStatus: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      queryReacquireStatus: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      getReadiness: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      coreRestore: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      workbenchPage: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      taskPage: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      taskSourceBatchPage: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      sarSearchProgress: async () => ({
        ok: false,
        error: "Browser Fixture has no live SAR progress",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      taskHistoryPage: async () => ({ ok: true, page: { page_size: 40, has_more: false, next_cursor: null, items: [] } }),
      taskEventPage: async (request) => ({ ok: true, page: { task_id: request.task_id, revision: request.expected_revision, page_size: request.page_size, has_more: false, next_before_sequence: null, items: [] } }),
      openTaskOutput: async (request) => ({ ok: true, task_id: request.task_id, revision: request.expected_revision }),
      listTaskConflictFiles: async (request) => ({
        ok: true,
        list: { task_id: request.task_id, files: [], total_count: 0, total_size_bytes: 0 },
      }),
      deleteTaskConflictFiles: async (request) => ({
        ok: true,
        result: { task_id: request.task_id, outcomes: [] },
      }),
      updateWorkbenchSelection: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createTaskFromWorkbench: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      appendTaskFromWorkbench: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      appendTaskFromSourceBatch: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createTaskFromHistoricalSourceBatch: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createBurstAssetTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      appendBurstAssetTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createHistoricalBurstAssetTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      workspacePrepareLocation: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      workspaceOpen: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      workspaceCreate: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      projectCreate: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      projectSelect: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      regionCreate: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      regionSelect: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      bindRegionAoi: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      clearRegionAoi: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      providerCapabilities: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      sarSearchAndFreeze: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      sarBackgroundCount: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      catalogSearchAndFreeze: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      catalogIntentSearchAndFreeze: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cancelRequest: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseSessionStatus: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseLogin: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseLogout: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseNetworkPolicy: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      setCdseNetworkPolicy: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseNetworkDiagnostic: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      previewAsset: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      cdseQuicklook: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      sentinel2MapPreview: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      usgsSessionStatus: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      usgsLogin: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      usgsLogout: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      landsatBrowse: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      landsatAssetDetails: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      bhoonidhiAssetSize: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createLandsatBundleTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createLandsatAssetTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createReviewedMpcAssetTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createLandsatBundleBatchTask: async () => ({
        ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createLandsatAssetBatchTask: async () => ({
        ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE",
      }),
      taskDeliveryAdmission: async () => ({ ok: true, admission: null }),
      opticalPreparationProgress: async () => ({ ok: true, progress: null }),
      createSentinel2ArchiveTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createSentinel2ArchiveBatchTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createSentinel2BandTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createSentinel2BandBatchTask: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      createHlsBandBatchTask: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      precheckOpticalAoiCoverage: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createOpticalDerivedTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      effectiveCapability: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      nisarRemoteSubsetCapability: async () => ({
        ok: true,
        capability: {
          feature_id: "nisar_remote_subset_v1",
          visible: true,
          enabled: false,
          implementation_scope: "offline_only",
          offline_verified: true,
          live_verified: false,
          production_enabled: false,
          default_enabled: false,
          output_authority: "client_derived",
          supported_product_type: "RSLC",
          supported_specification_versions: ["1.5.0"],
          supported_frequencies: ["A", "B"],
          supported_polarizations: { A: ["HH", "HV"], B: ["HH", "HV", "VV"] },
          reason_code: "NISAR_SUBSET_NOT_ENABLED",
        },
      }),
      createNisarRemoteSubsetTask: async () => ({
        ok: false,
        error: "NISAR Remote Subset is not enabled",
        code: "NISAR_SUBSET_NOT_ENABLED",
      }),
      prepareNisarSubsetIntentV2: async () => ({
        ok: false,
        error: "NISAR Remote Subset is not enabled",
        code: "NISAR_SUBSET_NOT_ENABLED",
      }),
      preflightNisarSubsetBatchV2: async () => ({
        ok: false,
        error: "NISAR Remote Subset is not enabled",
        code: "NISAR_SUBSET_NOT_ENABLED",
      }),
      prepareNisarSubsetBatchV2: async () => ({
        ok: false,
        error: "NISAR Remote Subset is not enabled",
        code: "NISAR_SUBSET_NOT_ENABLED",
      }),
      confirmNisarSubsetTaskV2: async () => ({
        ok: false,
        error: "NISAR Remote Subset is not enabled",
        code: "NISAR_SUBSET_NOT_ENABLED",
      }),
      confirmNisarSubsetBatchTaskV2: async () => ({
        ok: false,
        error: "NISAR Remote Subset is not enabled",
        code: "NISAR_SUBSET_NOT_ENABLED",
      }),
      appendNisarSubsetBatchTaskV2: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      readNisarSubsetAppendContextV2: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      preflightNisarSubsetAppendV2: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      createNisarSubsetSuccessorTaskV2: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      readNisarArchivedRecoveryContextV2: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      createNisarArchivedRecoveryV2: async () => ({ ok: false, error: "Browser Fixture has no application Store", code: "V3_APPLICATION_UNAVAILABLE" }),
      resumeNisarSubsetTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      startNewlyConfirmedTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      updateSearchWorkbench: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      createDownloadTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      appendDownloadTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      updateDownloadTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      updateDownloadTaskView: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      archiveDownloadTask: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      deleteDownloadHistory: async () => ({
        ok: false,
        error: "Browser Fixture has no application Store",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
      desktopOnboardingStateGet: async () => ({
        completed: true,
        returning_user: true,
      }),
      desktopOnboardingStateSet: async (completed: boolean) => ({
        completed,
      }),
      diagnosticsGetState: async () => ({
        enabled: false,
        total_count: 0,
        pending_count: 0,
      }),
      diagnosticsSetEnabled: async (enabled: boolean) => ({
        enabled,
        total_count: 0,
        pending_count: 0,
      }),
      diagnosticsRecord: async (input: any) => ({
        report_id: "diag-fixture",
        created_at: new Date().toISOString(),
        app_version: "2.2.0-beta.0",
        os: "fixture",
        arch: "fixture",
        category: input?.category || "frontend_error",
        error_code: input?.error_code || "UNSPECIFIED",
        http_status: input?.http_status ?? null,
        source: input?.source || "fixture",
        sanitized_message: String(input?.message || ""),
        sanitized_stack: String(input?.stack || ""),
        uploaded: false,
      }),
      diagnosticsList: async () => [],
      diagnosticsClear: async () => ({
        enabled: false,
        total_count: 0,
        pending_count: 0,
      }),
      diagnosticsUpload: async () => ({
        attempted: 0,
        uploaded: 0,
        failed: 0,
        message_zh: "没有待上报的本地诊断记录。",
      }),
      checkAppUpdate: async () => ({
        current_version: "2.2.0-beta.0",
        latest_version: null,
        has_update: false,
        release_url: "https://github.com/hhanmj/EOStage/releases",
        release_notes: null,
        checked_at: new Date().toISOString(),
        status: "up_to_date",
        message_zh: "当前已是最新版本（2.2.0-beta.0）。",
      }),
    },
    compatibility: {
      capabilities: async () => ({
        ok: false,
        error: "Browser Fixture has no compatibility Worker",
        code: "COMPAT_WORKER_UNAVAILABLE",
      }),
      callLegacy: async () => ({
        ok: false,
        error: "Browser Fixture has no compatibility Worker",
        code: "COMPAT_WORKER_UNAVAILABLE",
      }),
    },
    app: {
      getInfo: async () => ({ name: "EOStage", version: "2.2.0-beta.0", offline: true }),
    },
    window: {
      minimize: async () => ({ ok: true }),
      toggleMaximize: async () => ({ ok: true }),
      close: async () => {
        (host.close ?? closeBrowserWindow)();
        return { ok: true };
      },
      getSize: async () => {
        const size = (host.getSize ?? browserSize)();
        return { ok: true, width: size.width, height: size.height };
      },
      startDragging: async () => ({ ok: true }),
      resizeFromEdge: async () => ({ ok: true }),
    },
    dialog: {
      pickOpenFile: async () => ({ ok: true, path: "" }),
      pickDirectory: async () => ({ ok: true, path: "" }),
    },
  };
}
