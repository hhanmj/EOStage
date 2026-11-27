export type UiSmokeReport = {
  react_mounted: boolean;
  workbench_rendered: boolean;
  application_service_ready: boolean;
  leaflet_initialized: boolean;
  map_width: number;
  map_height: number;
  map_resized: boolean;
  tile_error_handled: boolean;
  aoi_overlay_rendered: boolean;
  unhandled_js_errors: number;
  csp_violations: number;
  v3_application_unavailable_seen: boolean;
  region_created_or_selected: boolean;
  aoi_bind_command_completed: boolean;
  aoi_persisted_in_snapshot: boolean;
  aoi_restored_after_restart: boolean;
  search_request_uses_persisted_aoi: boolean;
  workspace_open_completed: boolean;
  project_region_context_completed: boolean;
  per_region_aoi_completed: boolean;
  first_region_aoi_id: string | null;
  second_region_aoi_id: string | null;
  first_region_aoi_revision: number | null;
  second_region_aoi_revision: number | null;
  dem_plan_task_id: string | null;
  dem_task_control_completed: boolean;
  dem_task_control_revision: number | null;
  dem_task_completed_id: string | null;
  dem_conversion_task_id: string | null;
  dem_conversion_completed_id: string | null;
  orbit_task_id: string | null;
  orbit_completed_id: string | null;
  gacos_plan_offline: boolean;
  gacos_dry_run_offline: boolean;
};

type UiSmokeKey = keyof UiSmokeReport;

const initialReport = (): UiSmokeReport => ({
  react_mounted: false,
  workbench_rendered: false,
  application_service_ready: false,
  leaflet_initialized: false,
  map_width: 0,
  map_height: 0,
  map_resized: false,
  tile_error_handled: false,
  aoi_overlay_rendered: false,
  unhandled_js_errors: 0,
  csp_violations: 0,
  v3_application_unavailable_seen: false,
  region_created_or_selected: false,
  aoi_bind_command_completed: false,
  aoi_persisted_in_snapshot: false,
  aoi_restored_after_restart: false,
    search_request_uses_persisted_aoi: false,
    workspace_open_completed: false,
    project_region_context_completed: false,
    per_region_aoi_completed: false,
    first_region_aoi_id: null,
    second_region_aoi_id: null,
    first_region_aoi_revision: null,
    second_region_aoi_revision: null,
    dem_plan_task_id: null,
    dem_task_control_completed: false,
    dem_task_control_revision: null,
    dem_task_completed_id: null,
  dem_conversion_task_id: null,
  dem_conversion_completed_id: null,
  orbit_task_id: null,
  orbit_completed_id: null,
  gacos_plan_offline: false,
  gacos_dry_run_offline: false,
});

let enabled = false;
let report = initialReport();
let submitted = false;
let submitReport: ((value: UiSmokeReport) => Promise<void>) | null = null;

function complete(): boolean {
  return report.react_mounted
    && report.workbench_rendered
    && report.application_service_ready
    && report.leaflet_initialized
    && report.map_width > 0
    && report.map_height > 0
    && report.map_resized
    && report.tile_error_handled
    && report.aoi_overlay_rendered
    && report.unhandled_js_errors === 0
    && report.csp_violations === 0
    && !report.v3_application_unavailable_seen
    && report.region_created_or_selected
    && report.aoi_bind_command_completed
    && report.aoi_persisted_in_snapshot
    && report.aoi_restored_after_restart
    && report.search_request_uses_persisted_aoi
    && report.workspace_open_completed
    && report.project_region_context_completed
    && report.per_region_aoi_completed
    && Boolean(report.first_region_aoi_id)
    && Boolean(report.second_region_aoi_id)
    && report.first_region_aoi_id !== report.second_region_aoi_id
    && report.first_region_aoi_revision !== null
    && report.second_region_aoi_revision !== null
    && Boolean(report.dem_plan_task_id)
    && report.dem_task_control_completed
    && report.dem_task_control_revision !== null
    && Boolean(report.dem_task_completed_id)
    && Boolean(report.dem_conversion_task_id)
    && Boolean(report.dem_conversion_completed_id)
    && Boolean(report.orbit_task_id)
    && Boolean(report.orbit_completed_id)
    && report.gacos_plan_offline
    && report.gacos_dry_run_offline;
}

function maybeSubmit(): void {
  if (!enabled || submitted || !submitReport || !complete()) return;
  submitted = true;
  const value = { ...report };
  const submit = submitReport;
  window.setTimeout(() => {
    void submit(value).catch(() => {
      submitted = false;
    });
  }, 120);
}

export function configureUiSmoke(
  smokeEnabled: boolean,
  submit?: (value: UiSmokeReport) => Promise<void>,
): void {
  enabled = smokeEnabled;
  report = initialReport();
  submitted = false;
  submitReport = submit ?? null;
}

/**
 * Shell route the packaged smoke starts on. Its report requires the Leaflet
 * map checks, and in the W5 shell only the workbench page mounts the map at
 * startup (the agent page keeps it in a closed side panel).
 */
export const UI_SMOKE_SHELL_HASH = "#/workbench";

/** Hash to set before the first render in smoke mode, or null when already there. */
export function uiSmokeStartupHash(currentHash: string): string | null {
  return currentHash === UI_SMOKE_SHELL_HASH ? null : UI_SMOKE_SHELL_HASH;
}

export function isUiSmokeMode(): boolean {
  return enabled;
}

export function markUiSmoke<K extends UiSmokeKey>(key: K, value: UiSmokeReport[K]): void {
  if (!enabled) return;
  report = { ...report, [key]: value };
  maybeSubmit();
}

export function recordUiSmokeError(): void {
  if (!enabled) return;
  report = { ...report, unhandled_js_errors: report.unhandled_js_errors + 1 };
}

export function recordUiSmokeCspViolation(): void {
  if (!enabled) return;
  report = { ...report, csp_violations: report.csp_violations + 1 };
}

export function installUiSmokeObservers(): () => void {
  if (!enabled) return () => undefined;
  const onError = () => recordUiSmokeError();
  const onUnhandledRejection = () => recordUiSmokeError();
  const onSecurityPolicyViolation = () => recordUiSmokeCspViolation();
  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onUnhandledRejection);
  window.addEventListener("securitypolicyviolation", onSecurityPolicyViolation);
  return () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onUnhandledRejection);
    window.removeEventListener("securitypolicyviolation", onSecurityPolicyViolation);
  };
}

export function uiSmokeReportForTest(): UiSmokeReport {
  return { ...report };
}
