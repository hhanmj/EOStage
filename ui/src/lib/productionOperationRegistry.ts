export type ProductionOperationStatus =
  | "MIGRATED"
  | "APPROVED_COMPAT"
  | "UNSUPPORTED"
  | "DEAD_LEGACY";

export type ProductionUiReachability = "reachable" | "disabled" | "not_reachable";

export type ProductionOperationRecord = {
  operation_id: string;
  ui_reachability: ProductionUiReachability;
  ui_component_function: string;
  bridge_function: string | null;
  desktop_shell_bridge_api: string | null;
  tauri_command: string | null;
  application_service_owner: string | null;
  compatibility_worker_operation: string | null;
  store_engine_owner: string | null;
  status: ProductionOperationStatus;
  evidence: string;
};

/** Normal production Tauri has no approved legacy renderer methods. */
export const APPROVED_LEGACY_BRIDGE_METHODS = [] as const;

export const PRODUCTION_OPERATION_REGISTRY: readonly ProductionOperationRecord[] = [
  {
    operation_id: "app-shell",
    ui_reachability: "reachable",
    ui_component_function: "App/Workbench shell, dialogs, window controls",
    bridge_function: "getAppInfo/pickOpenFile/pickDirectory/window methods",
    desktop_shell_bridge_api: "app/dialog/window",
    tauri_command: "shell_app_info + Tauri plugin APIs",
    application_service_owner: null,
    compatibility_worker_operation: null,
    store_engine_owner: "Tauri shell/plugin",
    status: "MIGRATED",
    evidence: "ui/src/App.tsx; ui/src/lib/desktopShellBridgeTauri.ts",
  },
  {
    operation_id: "credentials",
    ui_reachability: "reachable",
    ui_component_function: "Settings/Workbench credential status and mutation controls",
    bridge_function: "credential status/save/clear functions",
    desktop_shell_bridge_api: "credentials.*",
    tauri_command: "credential_*",
    application_service_owner: null,
    compatibility_worker_operation: null,
    store_engine_owner: "desktop credential/keyring service",
    status: "MIGRATED",
    evidence: "ui/src/lib/desktopShellBridgeTauri.ts; src-tauri/src/commands.rs",
  },
  {
    operation_id: "catalogue-and-capability",
    ui_reachability: "reachable",
    ui_component_function: "Workbench provider capability, search, freeze, and cancel",
    bridge_function: "application provider/catalogue/SAR/cancel functions",
    desktop_shell_bridge_api: "application.* named APIs",
    tauri_command: "application_effective_capability/application_*_search_and_freeze/application_cancel_request",
    application_service_owner: "ApplicationService capability and Dispatcher methods",
    compatibility_worker_operation: null,
    store_engine_owner: "Registry/provider + SearchBatch/Workbench Store",
    status: "MIGRATED",
    evidence: "ui/src/lib/bridge.ts; src-tauri/src/lib.rs; crates/insar-application-service/src/lib.rs",
  },
  {
    operation_id: "workspace-project-region",
    ui_reachability: "reachable",
    ui_component_function: "Workspace open/create/select and Project/Region hierarchy",
    bridge_function: "workspaceOpen/projectCreate/projectSelect/regionCreate/regionSelect",
    desktop_shell_bridge_api: "application.workspaceOpen/projectCreate/projectSelect/regionCreate/regionSelect",
    tauri_command: "application_workspace_open/application_project_*/application_region_*",
    application_service_owner: "ApplicationService workspace/project/region context",
    compatibility_worker_operation: null,
    store_engine_owner: "TaskStore workspace_context and hierarchy projection",
    status: "MIGRATED",
    evidence: "crates/insar-store/src/repository.rs; crates/insar-application-service/src/lib.rs; src-tauri/src/commands.rs; ui/src/lib/bridge.ts workspace_prepare_location",
  },
  {
    operation_id: "workspace-context-restore",
    ui_reachability: "reachable",
    ui_component_function: "Workspace/Workbench startup context and restart restore projection",
    bridge_function: "getContext/getTree/restoreApplicationCore/loadApplicationWorkbenchPage",
    desktop_shell_bridge_api: "application.coreRestore/application.workbenchPage",
    tauri_command: "application_core_restore/application_workbench_page",
    application_service_owner: "ApplicationService CoreRestore/page and Store-owned WorkspaceContextState",
    compatibility_worker_operation: null,
    store_engine_owner: "TaskStore workspace_context + per-region region_aoi_state",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/lib.rs; crates/insar-store/src/history.rs; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "aoi-persistence",
    ui_reachability: "reachable",
    ui_component_function: "Workbench/AOI bind, clear, and restore",
    bridge_function: "setRegionAoiBbox/setRegionAoiGeojson/clearRegionAoi",
    desktop_shell_bridge_api: "application.bindRegionAoi/clearRegionAoi",
    tauri_command: "application_bind_region_aoi/application_clear_region_aoi",
    application_service_owner: "ApplicationService bind_region_aoi/clear_region_aoi",
    compatibility_worker_operation: null,
    store_engine_owner: "per-workspace, per-region region_aoi_state",
    status: "MIGRATED",
    evidence: "crates/insar-store/src/schema.rs; ui/src/lib/bridge.ts; crates/insar-application-service/tests/rh1_workspace_aoi.rs",
  },
  {
    operation_id: "aoi-import-normalization",
    ui_reachability: "reachable",
    ui_component_function: "AOI file/content/bytes/bundle preview and feature normalization",
    bridge_function: "previewAoiFile* / previewAoiFileBundle / setRegionAoiGeojsonFeatures",
    desktop_shell_bridge_api: "application.aoiImport/application.aoiNormalize",
    tauri_command: "application_aoi_import/application_aoi_normalize",
    application_service_owner: "ApplicationService normalization policy + product-owned insar-gis VectorCodec",
    compatibility_worker_operation: null,
    store_engine_owner: "Store only after typed AOI bind",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/capability_services.rs; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "aoi-boundary-search",
    ui_reachability: "reachable",
    ui_component_function: "Workbench global administrative/place candidate discovery and selected geometry preview",
    bridge_function: "searchAdminBoundaries / resolveAdminBoundary",
    desktop_shell_bridge_api: "application.adminBoundarySearch/application.adminBoundaryResolve",
    tauri_command: "application_admin_boundary_search/application_admin_boundary_resolve",
    application_service_owner: "ApplicationService BoundaryService with Nominatim/DataV provider adapters",
    compatibility_worker_operation: null,
    store_engine_owner: null,
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/boundary_service.rs; crates/insar-provider/src/boundary.rs; ui/src/pages/Workbench.tsx",
  },
  {
    operation_id: "china-admin-options",
    ui_reachability: "reachable",
    ui_component_function: "Workbench China-only province/city/district selector options",
    bridge_function: "getAdminOptions",
    desktop_shell_bridge_api: "application.adminOptions",
    tauri_command: "application_admin_options",
    application_service_owner: "ApplicationService BoundaryService with DataV/local options fallback",
    compatibility_worker_operation: null,
    store_engine_owner: null,
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/boundary_service.rs; crates/insar-provider/src/boundary.rs; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "shared-task-restore-create-control",
    ui_reachability: "reachable",
    ui_component_function: "Workbench restore, task create, append, control, archive, and history",
    bridge_function: "restoreApplicationCore/loadApplicationWorkbenchPage/exact task reads/application task functions",
    desktop_shell_bridge_api: "application.coreRestore/application.workbenchPage/application.taskPage/exact bounded task mutation APIs",
    tauri_command: "application_core_restore/application_workbench_page/application_download_task_page/application_*_task",
    application_service_owner: "ApplicationService CoreRestore, exact task reads, and bounded task mutation",
    compatibility_worker_operation: null,
    store_engine_owner: "TaskStore + shared Scheduler + Download Engine",
    status: "MIGRATED",
    evidence: "ui/src/lib/applicationServiceProjection.ts; crates/insar-application-service/src/lib.rs",
  },
  {
    operation_id: "dem-formal-task",
    ui_reachability: "reachable",
    ui_component_function: "Workbench/Download DEM plan, task controls, restore, and conversion handoff",
    bridge_function: "planDemDownload/startDemDownload/formalTaskCommand/runFormalDemConversionHandoff/standaloneDem",
    desktop_shell_bridge_api: "application.planDemTask/createDemTask/createStandaloneDemTask/handoffDemConversion/standaloneDem/taskPage",
    tauri_command: "application_plan_dem_task/application_create_dem_task/application_create_standalone_dem_task/application_resolve_dem_conversion_source/application_handoff_dem_conversion/application_standalone_dem/application_download_task_page",
    application_service_owner: "ApplicationService ResourceRegistry/semantics + product-owned insar-gis NativeGisEngine",
    compatibility_worker_operation: null,
    store_engine_owner: "FrozenTaskSnapshot operation descriptor + shared Store/Scheduler/Download Engine",
    status: "MIGRATED",
    evidence: "crates/insar-gis/src/raster.rs; src-tauri/src/application.rs; crates/insar-application-service/tests/rh1_dem_orbit_vertical.rs; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "dem-global-settings",
    ui_reachability: "reachable",
    ui_component_function: "Workbench global DEM concurrency editor",
    bridge_function: "getDemTaskSettings/saveDemTaskSettings",
    desktop_shell_bridge_api: "application.getDemTaskSettings/saveDemTaskSettings",
    tauri_command: "application_get_dem_task_settings/application_save_dem_task_settings",
    application_service_owner: "ApplicationService product-owned sidecar default; value is frozen into each new task",
    compatibility_worker_operation: null,
    store_engine_owner: "TaskStore task concurrency + shared Scheduler; no global DEM queue",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/lib.rs; src-tauri/src/commands.rs; ui/src/pages/Settings.tsx; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "orbit-formal-task",
    ui_reachability: "reachable",
    ui_component_function: "Workbench/Download Sentinel-1 POEORB association, task, control, and status",
    bridge_function: "startOrbitDownload/startOrbitDownloadSnapshot/formalTaskCommand",
    desktop_shell_bridge_api: "application.createOrbitTask/application.createStandaloneOrbitTask",
    tauri_command: "application_create_orbit_task/application_create_standalone_orbit_task",
    application_service_owner: "ApplicationService FrozenOrbitAssociation task route",
    compatibility_worker_operation: null,
    store_engine_owner: "FrozenOrbitAssociationV1 + OrbitHttpClient + ProductionDownloadWorker + shared Scheduler/Store",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/tests/rh1_dem_orbit_vertical.rs; crates/insar-download/src/production_worker.rs",
  },
  {
    operation_id: "gacos-plan-dry-run",
    ui_reachability: "reachable",
    ui_component_function: "Workbench GACOS plan and web-form dry-run preview",
    bridge_function: "planGacosRequest/gacosWebFormDryRun/getGacosLedger",
    desktop_shell_bridge_api: "application.gacosPlan/application.gacosDryRun/application.gacosLedger",
    tauri_command: "application_gacos_plan/application_gacos_dry_run/application_gacos_ledger",
    application_service_owner: "ApplicationService native offline GACOS planner and redacted dry-run",
    compatibility_worker_operation: null,
    store_engine_owner: null,
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/capability_services.rs; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "gacos-result-task",
    ui_reachability: "reachable",
    ui_component_function: "Workbench GACOS local-file result import, email-link result download, and credential repair",
    bridge_function: "createGacosImportTask/createGacosResultTask/supplyGacosResultLink",
    desktop_shell_bridge_api: "application.gacosCreateImportTask/application.gacosCreateResultTask/application.gacosSupplyResultLink",
    tauri_command: "application_gacos_create_import_task/application_gacos_create_result_task/application_gacos_supply_result_link",
    application_service_owner: "ApplicationService frozen-parent GACOS result task admission and credential supply",
    compatibility_worker_operation: null,
    store_engine_owner: "TaskStore + shared Scheduler + ProductionDownloadWorker",
    status: "MIGRATED",
    evidence: "ui/src/pages/Workbench.tsx; ui/src/lib/bridge.ts; crates/insar-application-service/src/gacos_result_service.rs",
  },
  {
    operation_id: "gacos-live-submit",
    ui_reachability: "reachable",
    ui_component_function: "Workbench GACOS native confirmation, request task, durable status, and cancel",
    bridge_function: "submitGacosRequest/getGacosRequestStatus",
    desktop_shell_bridge_api: "application.gacosSubmitRequest/application.gacosRequestStatus",
    tauri_command: "application_gacos_submit_request/application_gacos_request_status",
    application_service_owner: "ApplicationService native one-shot GACOS submission and durable status projection",
    compatibility_worker_operation: null,
    store_engine_owner: "GACOS submission attempt ledger + paused request TaskStore operation",
    status: "MIGRATED",
    evidence: "src-tauri/src/application.rs; src-tauri/src/commands.rs; ui/src/lib/desktopShellBridgeTauri.ts; ui/src/pages/Workbench.tsx",
  },
  {
    operation_id: "landsat-public-planetary-computer",
    ui_reachability: "reachable",
    ui_component_function: "Workbench Landsat C2 L2 public asset download",
    bridge_function: "startLandsatAssetDownload",
    desktop_shell_bridge_api: "application.createLandsatAssetTask",
    tauri_command: "application_create_landsat_asset_task",
    application_service_owner: "ApplicationService.create_landsat_asset_task",
    compatibility_worker_operation: null,
    store_engine_owner: "PlanetaryComputerLandsatClient + shared Store/Scheduler/Download Engine",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/lib.rs; ui/src/pages/Workbench.tsx",
  },
  {
    operation_id: "reviewed-mpc-cross-domain-cog-delivery",
    ui_reachability: "not_reachable",
    ui_component_function: "Not exposed in Workbench UI yet",
    bridge_function: "createApplicationReviewedMpcAssetTask",
    desktop_shell_bridge_api: "application.createReviewedMpcAssetTask",
    tauri_command: "application_create_reviewed_mpc_asset_task",
    application_service_owner: "ApplicationService.create_reviewed_mpc_asset_task",
    compatibility_worker_operation: null,
    store_engine_owner: "MpcReviewedCatalogClient + reviewed MPC evidence resolver + shared Store/Scheduler/Download Engine",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/tests/reviewed_mpc_catalog_freeze.rs; crates/insar-download/tests/planetary_computer_reviewed_execution.rs",
  },
  {
    operation_id: "legacy-scene-import-and-check",
    ui_reachability: "reachable",
    ui_component_function: "Download/SceneImportSection and Workbench manual ASF/local source/check controls",
    bridge_function: "importScenesText/importScenesFile/importScenesDirectory/previewScenes*/searchAsfScenes/checkScenes",
    desktop_shell_bridge_api: "application.importScenes",
    tauri_command: "application_import_scenes",
    application_service_owner: "ApplicationService bounded native Sentinel-1 parser; SearchBatch remains the persisted task authority",
    compatibility_worker_operation: null,
    store_engine_owner: "No legacy scene mutation; formal SearchBatch/TaskStore owns production state",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/lib.rs; src-tauri/src/commands.rs; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "standalone-asf-download",
    ui_reachability: "not_reachable",
    ui_component_function: "Download standalone ASF planning/download card",
    bridge_function: "planAsfDownload/startAsfDownload/appendAsfDownload",
    desktop_shell_bridge_api: null,
    tauri_command: null,
    application_service_owner: "ApplicationService.create_download_task from frozen SearchBatch",
    compatibility_worker_operation: null,
    store_engine_owner: "TaskStore + shared Scheduler + Download Engine",
    status: "DEAD_LEGACY",
    evidence: "ui/src/pages/Download.tsx; ui/src/lib/bridge.ts tauriLegacyUnavailable",
  },
  {
    operation_id: "orbit-directory-match",
    ui_reachability: "reachable",
    ui_component_function: "Download local EOF directory scan/match card",
    bridge_function: "matchOrbitsDirectory",
    desktop_shell_bridge_api: "application.matchOrbitsDirectory",
    tauri_command: "application_match_orbits_directory",
    application_service_owner: "ApplicationService bounded local directory adapter + insar-core OrbitMatch",
    compatibility_worker_operation: null,
    store_engine_owner: "FrozenOrbitAssociationV1 + shared Scheduler/Download Engine",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/lib.rs; crates/insar-core/src/orbit.rs; src-tauri/src/commands.rs; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "map-spatial-presentation-clear",
    ui_reachability: "reachable",
    ui_component_function: "WorkbenchMap 删除所有图层 control", // i18n-exempt: internal operation registry metadata
    bridge_function: "clearWorkingAoi",
    desktop_shell_bridge_api: "application.clearWorkingAoi",
    tauri_command: "application_clear_working_aoi",
    application_service_owner: "ApplicationService standalone Working AOI mutation; Workbench owns transient spatial presentation",
    compatibility_worker_operation: null,
    store_engine_owner: "Store preserves frozen SearchBatch, catalogue selection, products, and download tasks",
    status: "MIGRATED",
    evidence: "ui/src/pages/Workbench.tsx; ui/src/lib/bridge.ts; src-tauri/src/commands.rs",
  },
  {
    operation_id: "legacy-scene-status-and-clear",
    ui_reachability: "not_reachable",
    ui_component_function: "Legacy metadata status and scene-clear actions",
    bridge_function: "getMetadataStatus/clearScenes/checkScenes",
    desktop_shell_bridge_api: null,
    tauri_command: null,
    application_service_owner: "ApplicationService SearchBatch/task snapshot routes",
    compatibility_worker_operation: null,
    store_engine_owner: "Store-owned frozen workbench and task projection",
    status: "DEAD_LEGACY",
    evidence: "ui/src/pages/Workbench.tsx; ui/src/lib/bridge.ts tauriLegacyUnavailable",
  },
  {
    operation_id: "legacy-system-update-components",
    ui_reachability: "not_reachable",
    ui_component_function: "Workbench update checker and online component installer",
    bridge_function: "checkForUpdate/downloadAppUpdate/getComponentStatus/installComponent/removeComponent",
    desktop_shell_bridge_api: null,
    tauri_command: null,
    application_service_owner: null,
    compatibility_worker_operation: null,
    store_engine_owner: null,
    status: "DEAD_LEGACY",
    evidence: "ui/src/pages/Workbench.tsx; ui/src/lib/bridge.ts tauriLegacyUnavailable",
  },
  {
    operation_id: "network-proxy-cache-settings",
    ui_reachability: "reachable",
    ui_component_function: "Workbench product-owned proxy/cache editor and user-configured Tianditu token",
    bridge_function: "getNetworkSettings/saveNetworkSettings/getV3StoreSettings/stageV3StoreMigration",
    desktop_shell_bridge_api: "application.getNetworkSettings/saveNetworkSettings",
    tauri_command: "application_get_network_settings/application_save_network_settings",
    application_service_owner: "ApplicationService product-owned network-settings sidecar; provider admission remains contract-owned",
    compatibility_worker_operation: null,
    store_engine_owner: "Product-owned settings sidecar; no Store schema mutation",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/lib.rs; src-tauri/src/commands.rs; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "landsat-usgs-compat",
    ui_reachability: "not_reachable",
    ui_component_function: "Workbench USGS EarthExplorer/M2M compatibility session panel",
    bridge_function: "getUsgsSessionStatus/loginUsgsSession/logoutUsgsSession",
    desktop_shell_bridge_api: "application.usgs* (compatibility only; not normal Landsat path)",
    tauri_command: "application_usgs_*",
    application_service_owner: "Planetary Computer Landsat route for normal public assets",
    compatibility_worker_operation: null,
    store_engine_owner: "PlanetaryComputerLandsatClient + shared task engine",
    status: "DEAD_LEGACY",
    evidence: "ui/src/pages/Workbench.tsx; normal Landsat path uses startLandsatAssetDownload",
  },
  {
    operation_id: "legacy-report-generation",
    ui_reachability: "not_reachable",
    ui_component_function: "Retired Report view and generic report generation action",
    bridge_function: "generateReport",
    desktop_shell_bridge_api: null,
    tauri_command: null,
    application_service_owner: null,
    compatibility_worker_operation: null,
    store_engine_owner: null,
    status: "DEAD_LEGACY",
    evidence: "ui/src/lib/bridge.ts tauriLegacyUnavailable; ui/src/components/Sidebar.tsx",
  },
  {
    operation_id: "report-generation",
    ui_reachability: "reachable",
    ui_component_function: "Report page offline report projection",
    bridge_function: "generateReport",
    desktop_shell_bridge_api: "application.reportGenerate",
    tauri_command: "application_report_generate",
    application_service_owner: "ApplicationService report_generate",
    compatibility_worker_operation: null,
    store_engine_owner: "Application-owned report projection; no Worker authority",
    status: "MIGRATED",
    evidence: "crates/insar-application-service/src/capability_services.rs; ui/src/pages/Report.tsx; ui/src/lib/bridge.ts",
  },
  {
    operation_id: "legacy-v3-debug-store",
    ui_reachability: "not_reachable",
    ui_component_function: "Development-only V3 provider debug and store migration controls",
    bridge_function: "getV3Providers/searchV3Provider/planV3Provider/stageV3StoreMigration",
    desktop_shell_bridge_api: null,
    tauri_command: null,
    application_service_owner: "Formal catalogue and task routes only",
    compatibility_worker_operation: null,
    store_engine_owner: "Formal Store/Scheduler/Download Engine routes",
    status: "DEAD_LEGACY",
    evidence: "ui/src/pages/Workbench.tsx V3_PROVIDER_DEBUG_ENABLED; ui/src/lib/bridge.ts tauriLegacyUnavailable",
  },
  {
    operation_id: "retired-legacy-views",
    ui_reachability: "not_reachable",
    ui_component_function: "Overview/Scenes/Report/developer-only legacy surfaces",
    bridge_function: "legacy source-only exports",
    desktop_shell_bridge_api: null,
    tauri_command: null,
    application_service_owner: null,
    compatibility_worker_operation: null,
    store_engine_owner: null,
    status: "DEAD_LEGACY",
    evidence: "ui/src/App.tsx; ui/src/components/Sidebar.tsx",
  },
] as const;

export function validateProductionOperationRegistry(
  records: readonly ProductionOperationRecord[] = PRODUCTION_OPERATION_REGISTRY,
): string[] {
  const errors: string[] = [];
  const ids = new Set<string>();
  for (const record of records) {
    if (!record.operation_id.trim()) errors.push("operation_id is required");
    if (!record.ui_component_function.trim()) {
      errors.push(`UI component/function is required: ${record.operation_id}`);
    }
    if (!record.status.trim()) errors.push(`status is required: ${record.operation_id}`);
    if (!record.evidence.trim()) errors.push(`evidence is required: ${record.operation_id}`);
    if (ids.has(record.operation_id)) errors.push(`duplicate operation_id: ${record.operation_id}`);
    ids.add(record.operation_id);
    if (record.ui_reachability === "reachable" && record.status === "UNSUPPORTED") {
      errors.push(`reachable operation is unsupported: ${record.operation_id}`);
    }
    if (record.ui_reachability === "reachable" && record.status !== "DEAD_LEGACY") {
      const hasRoute = Boolean(record.desktop_shell_bridge_api || record.compatibility_worker_operation);
      if (!hasRoute) errors.push(`reachable operation has no typed or approved route: ${record.operation_id}`);
    }
    if (record.ui_reachability === "disabled" && record.status !== "UNSUPPORTED") {
      errors.push(`disabled operation must be unsupported: ${record.operation_id}`);
    }
    if (record.status === "APPROVED_COMPAT" && !record.compatibility_worker_operation) {
      errors.push(`approved compatibility operation has no Worker owner: ${record.operation_id}`);
    }
    if (record.ui_reachability === "reachable") {
      const forbiddenWorkerAuthority = /administrative_boundary_lookup|aoi_normalize|gacos_plan|gacos_form_dry_run|report_generate/;
      if (record.compatibility_worker_operation && forbiddenWorkerAuthority.test(record.compatibility_worker_operation)) {
        errors.push(`normal production operation delegates product authority to Worker: ${record.operation_id}`);
      }
      if (record.desktop_shell_bridge_api?.includes("compatibility.callLegacy")) {
        errors.push(`normal production operation uses compatibility.callLegacy: ${record.operation_id}`);
      }
    }
  }
  return errors;
}
