import type {
  DesktopAppInfo,
  DesktopCommandResult,
  DesktopPickResult,
  DesktopShellBridge,
  DesktopWindowSize,
  DesktopWindowWorkArea,
} from "./desktopShellBridge.ts";

export type PywebviewShellApi = {
  get_app_info: () => Promise<DesktopAppInfo>;
  pick_open_file: (title?: string, filters?: string[]) => Promise<DesktopPickResult>;
  pick_directory: (title?: string) => Promise<DesktopPickResult>;
  window_minimize?: () => Promise<DesktopCommandResult>;
  window_toggle_maximize?: (
    workArea?: DesktopWindowWorkArea,
  ) => Promise<DesktopCommandResult>;
  window_close?: () => Promise<DesktopCommandResult>;
  window_get_size?: () => Promise<DesktopWindowSize>;
  window_resize_from_edge?: (
    edge: string,
    startWidth: number,
    startHeight: number,
    deltaX: number,
    deltaY: number,
  ) => Promise<DesktopCommandResult>;
};

export type PywebviewShellFallbacks = {
  close?: () => void;
  getSize?: () => { width: number; height: number };
  getWorkArea?: () => DesktopWindowWorkArea | undefined;
};

function unavailableApplicationError() {
  return {
    ok: false as const,
    error: "pywebview application service route is not enabled",
    code: "V3_APPLICATION_UNAVAILABLE",
  };
}

function browserSize(): { width: number; height: number } {
  return {
    width: typeof window !== "undefined" ? window.innerWidth : 1320,
    height: typeof window !== "undefined" ? window.innerHeight : 880,
  };
}

function closeBrowserWindow(): void {
  if (typeof window !== "undefined") window.close();
}

function browserWorkArea(): DesktopWindowWorkArea | undefined {
  if (typeof window === "undefined" || typeof window.screen === "undefined") return undefined;
  const screen = window.screen as Screen & { availLeft?: number; availTop?: number };
  const x = Number.isFinite(screen.availLeft) ? Number(screen.availLeft) : 0;
  const y = Number.isFinite(screen.availTop) ? Number(screen.availTop) : 0;
  const width = Math.max(
    920,
    Math.floor(Number(screen.availWidth || window.innerWidth || 1200)),
  );
  const height = Math.max(
    620,
    Math.floor(Number(screen.availHeight || window.innerHeight || 800)) - 1,
  );
  return { x: Math.round(x), y: Math.round(y), width, height };
}

export function createPywebviewDesktopShellBridge(
  api: PywebviewShellApi,
  fallbacks: PywebviewShellFallbacks = {},
): DesktopShellBridge {
  return {
    kind: "pywebview",
    ready: async () => true,
    application: {
      querySourceMutationStatus: async () => unavailableApplicationError(),
      querySourceAppendStatus: async () => unavailableApplicationError(),
      queryHistoricalSuccessorStatus: async () => unavailableApplicationError(),
      queryBurstAppendStatus: async () => unavailableApplicationError(),
      queryBurstSuccessorStatus: async () => unavailableApplicationError(),
      queryNisarSuccessorStatus: async () => unavailableApplicationError(),
      queryNisarArchivedRecoveryStatus: async () => unavailableApplicationError(),
      queryReacquireStatus: async () => unavailableApplicationError(),
      getReadiness: async () => ({
        ok: false,
        error: "pywebview application service route is not enabled",
        code: "V3_APPLICATION_UNAVAILABLE",
      }),
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
      providerCapabilities: async () => unavailableApplicationError(),
      bindRegionAoi: async () => unavailableApplicationError(),
      clearRegionAoi: async () => unavailableApplicationError(),
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
      taskDeliveryAdmission: async () => unavailableApplicationError(),
      opticalPreparationProgress: async () => unavailableApplicationError(),
      createReviewedMpcAssetTask: async () => unavailableApplicationError(),
      createSentinel2ArchiveTask: async () => unavailableApplicationError(),
      createSentinel2ArchiveBatchTask: async () => unavailableApplicationError(),
      createSentinel2BandTask: async () => unavailableApplicationError(),
      createSentinel2BandBatchTask: async () => unavailableApplicationError(),
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
    },
    compatibility: {
      capabilities: async () => ({
        ok: false,
        error: "Legacy desktop compatibility Worker is unavailable",
        code: "COMPAT_WORKER_UNAVAILABLE",
      }),
      callLegacy: async () => ({
        ok: false,
        error: "Legacy desktop compatibility Worker is unavailable",
        code: "COMPAT_WORKER_UNAVAILABLE",
      }),
    },
    app: {
      getInfo: () => api.get_app_info(),
    },
    window: {
      minimize: () => api.window_minimize?.() ?? Promise.resolve({ ok: true }),
      toggleMaximize: () =>
        api.window_toggle_maximize?.(
          (fallbacks.getWorkArea ?? browserWorkArea)(),
        ) ?? Promise.resolve({ ok: true }),
      close: async () => {
        if (api.window_close) return api.window_close();
        (fallbacks.close ?? closeBrowserWindow)();
        return { ok: true };
      },
      getSize: async () => {
        if (api.window_get_size) return api.window_get_size();
        const size = (fallbacks.getSize ?? browserSize)();
        return { ok: true, width: size.width, height: size.height };
      },
      resizeFromEdge: (edge, startWidth, startHeight, deltaX, deltaY) =>
        api.window_resize_from_edge?.(
          edge,
          startWidth,
          startHeight,
          deltaX,
          deltaY,
        ) ?? Promise.resolve({ ok: true }),
    },
    dialog: {
      pickOpenFile: (title = "", filters = []) => api.pick_open_file(title, filters),
      pickDirectory: (title = "") => api.pick_directory(title),
    },
  };
}
