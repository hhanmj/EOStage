import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import {
  createApplicationDemTask,
  createApplicationOrbitTask,
  gacosWebFormDryRun,
  handoffApplicationDemConversion,
  planApplicationDemTask,
  planGacosRequest,
  setRegionAoiBbox,
  type Bbox,
  type Json,
  type SceneRow,
  type V3GacosSubmission,
} from "./bridge.ts";
import type {
  DesktopCoreRestoreSnapshot,
  DesktopUpdateDownloadTaskRequest,
} from "./desktopShellBridge.ts";
import { getDesktopShellBridge } from "./desktopShellBridge.ts";
import type { CatalogSearchQuery } from "./generated/v3-contracts.ts";
import type { FrozenOrbitAssociationV1 } from "./generated/v3-contracts.ts";
import { isUiSmokeMode, markUiSmoke } from "./uiSmoke.ts";

const SMOKE_AOI_BBOX: Bbox = {
  west: 116.1,
  east: 116.5,
  south: 39.7,
  north: 40.0,
  crs: "EPSG:4326",
};
const SMOKE_SECOND_AOI_BBOX: Bbox = {
  west: 116.6,
  east: 117.0,
  south: 39.2,
  north: 39.5,
  crs: "EPSG:4326",
};

const ALOS2_LOGICAL_DATASET = "sar.alos-2.palsar-2.scansar.l1.1.v1";
const ALOS2_PROVIDER_ID = "asf.nasa.earthdata";
const ALOS2_NATIVE_COLLECTION_ID = "C3315903479-ASF";
const SMOKE_WORKSPACE_ID = "ui-smoke-workspace";
const SMOKE_WORKSPACE_ROOT = "C:\\EOStage\\ui-smoke";
const SMOKE_PROJECT_NAME = "UI Smoke Project";
const SMOKE_REGION_NAME = "UI Smoke Region A";
const SMOKE_SECOND_REGION_NAME = "UI Smoke Region B";
const SMOKE_DEM_TASK_ID = "ui-smoke-dem-task";
const SMOKE_DEM_CONVERSION_TASK_ID = "ui-smoke-dem-conversion";
const SMOKE_ORBIT_TASK_ID = "ui-smoke-orbit-task";

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
}

async function checkpoint(step: string): Promise<void> {
  try {
    await tauriInvoke("ui_smoke_checkpoint", { step });
  } catch {
    // Checkpoints are diagnostics only; the typed smoke report remains the
    // authoritative success contract.
  }
}

function sameGeometry(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  if (
    typeof left !== "object" || typeof right !== "object"
    || left === null || right === null
  ) {
    return false;
  }
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left) && Array.isArray(right)
      && left.length === right.length
      && left.every((value, index) => sameGeometry(value, right[index]));
  }
  const leftKeys = Object.keys(left as Record<string, unknown>);
  const rightKeys = Object.keys(right as Record<string, unknown>);
  return leftKeys.length === rightKeys.length
    && leftKeys.every((key) =>
      sameGeometry(
        (left as Record<string, unknown>)[key],
        (right as Record<string, unknown>)[key],
      ),
    );
}

let started = false;

type RegionAoiState = {
  region_id?: string;
  revision?: number;
  source_kind?: string;
  geometry?: Json;
};

type SmokeRegionContext = {
  firstRegionId: string;
  secondRegionId: string;
};

function persistedRegionAoi(snapshot: DesktopCoreRestoreSnapshot): RegionAoiState | null {
  if (!snapshot.ok) return null;
  const context = snapshot.core.workspace_context;
  const regionId = context && typeof context === "object" && !Array.isArray(context)
    ? (context as Record<string, unknown>).region_id
    : undefined;
  const regionAois = Array.isArray(snapshot.core.region_aois)
    ? snapshot.core.region_aois.filter(
        (value): value is RegionAoiState =>
          typeof value === "object" && value !== null && !Array.isArray(value),
      )
    : [];
  const match = regionAois.find(
    (value) => typeof regionId === "string" && value.region_id === regionId,
  );
  if (match) return match;
  const legacy = snapshot.core.region_aoi;
  return legacy && typeof legacy === "object" && !Array.isArray(legacy)
    && (typeof regionId !== "string" || (legacy as RegionAoiState).region_id === regionId)
    ? legacy as RegionAoiState
    : null;
}

function recordValue(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function numberValue(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function bboxGeometry(bbox: Bbox): Json {
  return {
    type: "Polygon",
    coordinates: [[
      [bbox.west, bbox.south],
      [bbox.east, bbox.south],
      [bbox.east, bbox.north],
      [bbox.west, bbox.north],
      [bbox.west, bbox.south],
    ]],
  };
}

function regionIdFromContextResult(result: { ok: true; context: Record<string, unknown> } | { ok: false }): string | null {
  return result.ok ? stringValue(result.context.region_id) : null;
}

function regionIdsFromSnapshot(snapshot: DesktopCoreRestoreSnapshot): SmokeRegionContext | null {
  if (!snapshot.ok) return null;
  const context = recordValue(snapshot.core.workspace_context);
  const regions = Array.isArray(context?.regions)
    ? context.regions.map(recordValue).filter((value): value is Record<string, unknown> => value !== null)
    : [];
  const first = regions.find((value) => value.region_name === SMOKE_REGION_NAME);
  const second = regions.find((value) => value.region_name === SMOKE_SECOND_REGION_NAME);
  const firstRegionId = stringValue(first?.region_id);
  const secondRegionId = stringValue(second?.region_id);
  return firstRegionId && secondRegionId && firstRegionId !== secondRegionId
    ? { firstRegionId, secondRegionId }
    : null;
}

async function selectSmokeRegion(regionId: string, requestId: string): Promise<boolean> {
  const application = getDesktopShellBridge().application;
  const snapshot = await application.coreRestore(true);
  const context = snapshot.ok ? recordValue(snapshot.core.workspace_context) : null;
  const projectId = stringValue(context?.project_id);
  const revision = numberValue(context?.revision);
  if (!projectId || revision === null) return false;
  const result = await application.regionSelect({
    request_id: requestId,
    parent_id: projectId,
    expected_revision: revision,
    node_id: regionId,
  });
  return result.ok;
}

async function readSmokeRegionAoi(
  regionId: string,
  expectedGeometry: Json,
): Promise<RegionAoiState | null> {
  if (!(await selectSmokeRegion(regionId, `ui-smoke-read-aoi-${regionId}`))) return null;
  const snapshot = await getDesktopShellBridge().application.coreRestore(true);
  if (!snapshot.ok) return null;
  const aoi = persistedRegionAoi(snapshot);
  if (!aoi || aoi.region_id !== regionId || !sameGeometry(aoi.geometry, expectedGeometry)) return null;
  return aoi;
}

async function bindSmokeRegionAoi(
  regionId: string,
  bbox: Bbox,
  displayName: string,
  requestId: string,
): Promise<RegionAoiState | null> {
  if (!(await selectSmokeRegion(regionId, `${requestId}-select`))) return null;
  const bound = await setRegionAoiBbox(
    bbox.west,
    bbox.east,
    bbox.south,
    bbox.north,
    displayName,
  );
  if (!bound.ok) return null;
  return readSmokeRegionAoi(regionId, bboxGeometry(bbox));
}

async function openSmokeWorkspaceContext(): Promise<SmokeRegionContext | null> {
  const application = getDesktopShellBridge().application;
  const workspace = await application.workspaceOpen({
    request_id: "ui-smoke-workspace-open",
    workspace_id: SMOKE_WORKSPACE_ID,
    workspace_root: SMOKE_WORKSPACE_ROOT,
    workspace_name: "UI Smoke Workspace",
  });
  if (!workspace.ok) return null;
  const workspaceContext = recordValue(workspace.context);
  const workspaceId = stringValue(workspaceContext?.workspace_id);
  const workspaceRevision = numberValue(workspaceContext?.revision);
  if (!workspaceId || workspaceRevision === null) return null;
  const project = await application.projectCreate({
    request_id: "ui-smoke-project-create",
    parent_id: workspaceId,
    expected_revision: workspaceRevision,
    name: SMOKE_PROJECT_NAME,
  });
  if (!project.ok) return null;
  const projectContext = recordValue(project.context);
  const projectId = stringValue(projectContext?.project_id);
  const projectRevision = numberValue(projectContext?.revision);
  if (!projectId || projectRevision === null) return null;
  const firstRegion = await application.regionCreate({
    request_id: "ui-smoke-region-create",
    parent_id: projectId,
    expected_revision: projectRevision,
    name: SMOKE_REGION_NAME,
  });
  const firstRegionId = regionIdFromContextResult(firstRegion);
  if (!firstRegion.ok || !firstRegionId) return null;
  const firstRegionContext = recordValue(firstRegion.context);
  const firstRegionRevision = numberValue(firstRegionContext?.revision);
  if (firstRegionRevision === null) return null;
  const secondRegion = await application.regionCreate({
    request_id: "ui-smoke-second-region-create",
    parent_id: projectId,
    expected_revision: firstRegionRevision,
    name: SMOKE_SECOND_REGION_NAME,
  });
  const secondRegionId = regionIdFromContextResult(secondRegion);
  return secondRegionId && secondRegionId !== firstRegionId
    ? { firstRegionId, secondRegionId }
    : null;
}

function taskRuntime(
  snapshot: DesktopCoreRestoreSnapshot,
  taskId: string,
): { revision: number; status: string } | null {
  if (!snapshot.ok) return null;
  if (!Array.isArray(snapshot.core.tasks)) return null;
  const task = snapshot.core.tasks
    .map(recordValue)
    .find((value) => value?.task_id === taskId);
  if (!task) return null;
  const revision = numberValue(task.revision);
  const status = stringValue(task.status);
  return revision === null || !status ? null : { revision, status };
}

async function updateFormalTask(
  taskId: string,
  kind: Extract<DesktopUpdateDownloadTaskRequest["command"]["kind"], "pause_task" | "resume_task">,
): Promise<number | null> {
  const application = getDesktopShellBridge().application;
  let lastAttempt = "not_started";
  for (let attempt = 0; attempt < 12; attempt += 1) {
    const snapshot = await application.coreRestore(true);
    const runtime = taskRuntime(snapshot, taskId);
    if (!runtime) {
      lastAttempt = `runtime_missing_${attempt}`;
    } else if (runtime.status === "completed" || runtime.status === "failed" || runtime.status === "cancelled") {
      lastAttempt = `terminal_${runtime.status}_${runtime.revision}`;
    } else {
      const result = await application.updateDownloadTask({
        task_id: taskId,
        expected_revision: runtime.revision,
        // Pause/resume are task-level Store commands and must not carry the
        // selected item IDs. Item-scoped commands use item_ids explicitly.
        command: { kind, item_ids: [] },
        event_id: `ui-smoke-${kind}-${taskId}-${attempt}`,
        occurred_at: new Date().toISOString(),
      });
      if (result.ok) {
        const updated = taskRuntime(result, taskId);
        if (updated) {
          await checkpoint(`phase_b:${kind}:ok:${updated.revision}`);
          return updated.revision;
        }
        lastAttempt = `result_runtime_missing_${runtime.revision}`;
      } else {
        lastAttempt = `error_${result.code}_${runtime.revision}`;
      }
    }
    await delay(20);
  }
  await checkpoint(`phase_b:${kind}:exhausted:${lastAttempt}`);
  return null;
}

async function waitForFormalCompletion(taskId: string): Promise<boolean> {
  const application = getDesktopShellBridge().application;
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    const core = await application.coreRestore(true);
    const runtime = taskRuntime(core, taskId);
    if (runtime) {
      const page = await application.taskPage({
        task_id: taskId,
        expected_revision: runtime.revision,
        page_index: 0,
        page_size: 1,
      });
      if (page.ok) {
        const items = page.page && recordValue(page.page)?.items;
        if (Array.isArray(items) && items.some((value) => recordValue(value)?.status === "completed")) {
          return true;
        }
      }
    }
    await delay(35);
  }
  return false;
}

const SMOKE_SCENE: SceneRow = {
  scene_id: "S1A_UI_SMOKE_SCENE",
  product_key: "sentinel1:ui-smoke",
  product_id: "S1A_UI_SMOKE_SCENE",
  provider: "asf.sentinel1",
  dataset: "sar.sentinel-1.grd",
  instrument: "SAR",
  processing_level: "L1",
  product_family: "GRD",
  file_name: "S1A_UI_SMOKE_SCENE.SAFE",
  native_metadata: {},
  platform: "S1A",
  product_type: "GRD",
  beam_mode: "IW",
  polarization: "VV",
  acquisition_datetime: "2026-08-12T00:00:00Z",
  absolute_orbit: 1,
  relative_orbit: 1,
  path: 1,
  frame: 1,
  orbit_direction: "ASCENDING",
  file_size_remote: 0,
  footprint_bbox: SMOKE_AOI_BBOX,
  footprint_geojson: null,
  has_url: false,
  task_item_ids: [],
  assets: [],
};

function smokeOrbitAssociation(): FrozenOrbitAssociationV1 {
  return {
    association_version: 1,
    source_item_id: SMOKE_SCENE.scene_id,
    orbit_transfer_item_id: "orbit-poeorb-ui-smoke",
    requested_orbit_type: "POEORB",
    source_platform: "S1A",
    source_acquisition_start: 1_700_000_000,
    source_acquisition_stop: 1_700_001_000,
    matched_orbit_metadata: {
      file_name: "S1A_OPER_AUX_POEORB.EOF",
      platform: "S1A",
      orbit_type: "POEORB",
      creation_unix_seconds: 1_700_002_000,
      validity_start_unix_seconds: 1_699_999_000,
      validity_stop_unix_seconds: 1_700_010_000,
    },
    selection_policy_id: "sentinel1.poeorb.frozen.v1",
  };
}

function smokeGacosSubmission(): V3GacosSubmission {
  return {
    kind: "web_form_submission",
    provider: "gacos",
    execution_mode: "browser_assisted_web_form",
    portal_url: "http://www.gacos.net/",
    submit_endpoint: "http://www.gacos.net/M/action_page.php",
    method: "POST",
    content_type: "application/x-www-form-urlencoded",
    output_format: "geotiff",
    requires_user_confirmation: true,
    requires_email_delivery: false,
    email_field: "email",
    result_delivery: "manual_download",
    download_link_handling: "manual",
    batches: [{
      batch_id: "ui-smoke-gacos-batch",
      batch_index: 1,
      batch_count: 1,
      date_count: 1,
      dates: ["2026-08-12"],
      date_text: "2026-08-12",
      bbox: SMOKE_AOI_BBOX,
      method: "POST",
      endpoint: "http://www.gacos.net/M/action_page.php",
      content_type: "application/x-www-form-urlencoded",
      form_fields: {
        N: "40.0",
        S: "39.7",
        W: "116.1",
        E: "116.5",
        H: "0",
        M: "0",
        date: "2026-08-12",
        type: "2",
        seq: "UI smoke",
      },
      required_sensitive_fields: [],
    }],
  };
}

/**
 * Phase A: launch on an isolated Store, bind the processing AOI through the
 * formal Tauri command, confirm the AOI was written to the Store, then signal
 * the harness to shut the application down normally.
 */
async function runPhaseA(): Promise<void> {
  await checkpoint("phase_a:start");
  await delay(300);
  const smokeContext = await openSmokeWorkspaceContext();
  if (!smokeContext) {
    await checkpoint("phase_a:workspace_context_failed");
    return;
  }
  await checkpoint("phase_a:workspace_context_ok");
  markUiSmoke("workspace_open_completed", true);
  markUiSmoke("project_region_context_completed", true);
  const firstAoi = await bindSmokeRegionAoi(
    smokeContext.firstRegionId,
    SMOKE_AOI_BBOX,
    "Smoke AOI A",
    "ui-smoke-first-aoi",
  );
  const secondAoi = await bindSmokeRegionAoi(
    smokeContext.secondRegionId,
    SMOKE_SECOND_AOI_BBOX,
    "Smoke AOI B",
    "ui-smoke-second-aoi",
  );
  if (!firstAoi || !secondAoi) {
    await checkpoint("phase_a:aoi_bind_failed");
    return;
  }
  await checkpoint("phase_a:aoi_bind_ok");
  await delay(150);
  const firstRestored = await readSmokeRegionAoi(
    smokeContext.firstRegionId,
    bboxGeometry(SMOKE_AOI_BBOX),
  );
  const secondRestored = await readSmokeRegionAoi(
    smokeContext.secondRegionId,
    bboxGeometry(SMOKE_SECOND_AOI_BBOX),
  );
  if (!firstRestored || !secondRestored) {
    await checkpoint("phase_a:aoi_restore_failed");
    return;
  }
  await checkpoint("phase_a:aoi_restore_ok");
  markUiSmoke("region_created_or_selected", true);
  markUiSmoke("aoi_bind_command_completed", true);
  markUiSmoke("aoi_persisted_in_snapshot", true);
  // Leave Region B active so phase B proves that workspace_open restores the
  // currently selected region, then it switches back to Region A and returns.
  markUiSmoke("first_region_aoi_id", smokeContext.firstRegionId);
  markUiSmoke("second_region_aoi_id", smokeContext.secondRegionId);
  markUiSmoke("first_region_aoi_revision", numberValue(firstRestored.revision));
  markUiSmoke("second_region_aoi_revision", numberValue(secondRestored.revision));
  // The harness relaunches the packaged EXE for phase B on this same Store.
  await checkpoint("phase_a:complete_requested");
  void tauriInvoke("ui_smoke_phase_a_complete").catch(() => undefined);
}

/**
 * Phase B: restart on the same Store without rebinding, restore the AOI from
 * the Store snapshot, run a real search through the loopback catalogue
 * provider, and prove the provider actually received the restored geometry.
 */
async function runPhaseB(): Promise<void> {
  await checkpoint("phase_b:start");
  await delay(300);
  const snapshot = await getDesktopShellBridge().application.coreRestore(true);
  if (!snapshot.ok) {
    await checkpoint("phase_b:restore_failed");
    return;
  }
  await checkpoint("phase_b:restore_ok");
  const smokeContext = regionIdsFromSnapshot(snapshot);
  if (!smokeContext) {
    await checkpoint("phase_b:region_context_missing");
    return;
  }
  await checkpoint("phase_b:region_context_ok");
  const regionAoi = persistedRegionAoi(snapshot);
  if (!regionAoi || !regionAoi.geometry || regionAoi.region_id !== smokeContext.secondRegionId) {
    await checkpoint("phase_b:active_region_aoi_missing");
    return;
  }
  await checkpoint("phase_b:active_region_aoi_ok");
  const workspace = await getDesktopShellBridge().application.workspaceOpen({
    request_id: "ui-smoke-workspace-reopen",
    workspace_id: SMOKE_WORKSPACE_ID,
    workspace_root: SMOKE_WORKSPACE_ROOT,
    workspace_name: "UI Smoke Workspace",
  });
  if (!workspace.ok) {
    await checkpoint("phase_b:workspace_open_failed");
    return;
  }
  await checkpoint("phase_b:workspace_open_ok");
  markUiSmoke("workspace_open_completed", true);
  markUiSmoke("project_region_context_completed", true);

  const firstRestored = await readSmokeRegionAoi(
    smokeContext.firstRegionId,
    bboxGeometry(SMOKE_AOI_BBOX),
  );
  const secondRestored = await readSmokeRegionAoi(
    smokeContext.secondRegionId,
    bboxGeometry(SMOKE_SECOND_AOI_BBOX),
  );
  if (!firstRestored || !secondRestored) {
    await checkpoint("phase_b:per_region_aoi_restore_failed");
    return;
  }
  await checkpoint("phase_b:per_region_aoi_restore_ok");
  markUiSmoke("aoi_restored_after_restart", true);
  // Phase A performed the bind command; these marks are emitted only after
  // Phase B observes both persisted results under their stable region IDs.
  markUiSmoke("aoi_bind_command_completed", true);
  markUiSmoke("aoi_persisted_in_snapshot", true);
  markUiSmoke("region_created_or_selected", true);
  markUiSmoke("per_region_aoi_completed", true);
  markUiSmoke("first_region_aoi_id", smokeContext.firstRegionId);
  markUiSmoke("second_region_aoi_id", smokeContext.secondRegionId);
  markUiSmoke("first_region_aoi_revision", numberValue(firstRestored.revision));
  markUiSmoke("second_region_aoi_revision", numberValue(secondRestored.revision));

  const geometry = regionAoi.geometry;
  const search: CatalogSearchQuery = {
    logical_dataset: ALOS2_LOGICAL_DATASET,
    provider_id: ALOS2_PROVIDER_ID,
    native_collection_id: ALOS2_NATIVE_COLLECTION_ID,
    aoi_geojson: geometry,
    cursor: null,
    result_limit: 25,
    filters: {
      datetime: { start: "2023-03-31T00:00:00Z", end: "2023-04-01T00:00:00Z" },
      beam_mode: "WBD",
      polarizations: ["HH", "HV"],
      flight_direction: "DESCENDING",
      look_direction: "RIGHT",
      path: 161,
      frame: 3150,
    },
  };
  const requestId = `ui-smoke-search-${Date.now()}`;
  const result = await getDesktopShellBridge().application.catalogSearchAndFreeze({
    request_id: requestId,
    search,
    batch_id: `ui-smoke-search-batch-${Date.now()}`,
    created_at: new Date().toISOString(),
    aoi_name: "UI smoke persisted AOI",
    aoi_geojson: geometry,
    workbench_page_size: 24,
  });
  if (!result.ok) {
    await checkpoint("phase_b:catalog_search_failed");
    return;
  }
  await checkpoint("phase_b:catalog_search_ok");
  const captured = await tauriInvoke<CatalogSearchQuery | null>(
    "ui_smoke_captured_search_query",
  );
  markUiSmoke(
    "search_request_uses_persisted_aoi",
    Boolean(captured) && sameGeometry(captured?.aoi_geojson ?? null, geometry),
  );
  await checkpoint(captured ? "phase_b:captured_query_ok" : "phase_b:captured_query_missing");

  const context = await getDesktopShellBridge().application.coreRestore(true);
  const contextRecord = context.ok ? recordValue(context.core.workspace_context) : null;
  const regionId = stringValue(contextRecord?.region_id);
  const outputRoot = `${SMOKE_WORKSPACE_ROOT}\\formal-operations`;
  if (!regionId) {
    await checkpoint("phase_b:active_region_id_missing");
    return;
  }
  await checkpoint("phase_b:active_region_id_ok");

  const demPlan = await planApplicationDemTask({
    request_id: "ui-smoke-dem-plan",
    task_id: SMOKE_DEM_TASK_ID,
    region_id: regionId,
    resource_id: "dem.cop30.v1",
    capability_id: "capability.dem.cop30.aws-download.v1",
    dataset: "COP30",
    provider: "COPERNICUS_AWS",
    operation: "prepare_sarscape",
    input_path: `${outputRoot}\\source-dem.tif`,
    output_root: outputRoot,
    created_at: new Date().toISOString(),
  });
  if (!demPlan.ok) {
    await checkpoint("phase_b:dem_plan_failed");
    return;
  }
  await checkpoint("phase_b:dem_plan_ok");
  markUiSmoke("dem_plan_task_id", demPlan.plan.task_id);
  const demCreated = await createApplicationDemTask({ plan: demPlan.plan, concurrency: 1 });
  if (!demCreated.ok) {
    await checkpoint("phase_b:dem_create_failed");
    return;
  }
  await checkpoint("phase_b:dem_create_ok");
  const pausedRevision = await updateFormalTask(SMOKE_DEM_TASK_ID, "pause_task");
  await delay(25);
  const resumedRevision = await updateFormalTask(SMOKE_DEM_TASK_ID, "resume_task");
  if (pausedRevision === null || resumedRevision === null || resumedRevision <= pausedRevision) {
    await checkpoint("phase_b:dem_control_failed");
    return;
  }
  await checkpoint("phase_b:dem_control_ok");
  markUiSmoke("dem_task_control_completed", true);
  markUiSmoke("dem_task_control_revision", resumedRevision);
  if (!(await waitForFormalCompletion(SMOKE_DEM_TASK_ID))) {
    await checkpoint("phase_b:dem_completion_failed");
    return;
  }
  await checkpoint("phase_b:dem_completion_ok");
  markUiSmoke("dem_task_completed_id", SMOKE_DEM_TASK_ID);

  const demConversion = await handoffApplicationDemConversion({
    request_id: "ui-smoke-dem-conversion-handoff",
    source_task_id: SMOKE_DEM_TASK_ID,
    conversion_task_id: SMOKE_DEM_CONVERSION_TASK_ID,
    region_id: regionId,
    resource_id: "dem.cop30.v1",
    capability_id: "capability.dem.cop30.aws-download.v1",
    dataset: "COP30",
    provider: "COPERNICUS_AWS",
    input_path: `${outputRoot}\\source-dem.tif`,
    output_root: outputRoot,
    created_at: new Date().toISOString(),
    concurrency: 1,
  });
  if (!demConversion.ok) {
    await checkpoint("phase_b:dem_conversion_create_failed");
    return;
  }
  await checkpoint("phase_b:dem_conversion_create_ok");
  markUiSmoke("dem_conversion_task_id", SMOKE_DEM_CONVERSION_TASK_ID);
  if (!(await waitForFormalCompletion(SMOKE_DEM_CONVERSION_TASK_ID))) {
    await checkpoint("phase_b:dem_conversion_completion_failed");
    return;
  }
  await checkpoint("phase_b:dem_conversion_completion_ok");
  markUiSmoke("dem_conversion_completed_id", SMOKE_DEM_CONVERSION_TASK_ID);

  const orbit = await createApplicationOrbitTask({
    request_id: "ui-smoke-orbit-task-request",
    task_id: SMOKE_ORBIT_TASK_ID,
    region_id: regionId,
    association: smokeOrbitAssociation(),
    output_root: `${outputRoot}\\Sentinel_Orbit\\AUX_POEORB`,
    created_at: new Date().toISOString(),
    concurrency: 1,
  });
  if (!orbit.ok) {
    await checkpoint("phase_b:orbit_create_failed");
    return;
  }
  await checkpoint("phase_b:orbit_create_ok");
  markUiSmoke("orbit_task_id", SMOKE_ORBIT_TASK_ID);
  if (!(await waitForFormalCompletion(SMOKE_ORBIT_TASK_ID))) {
    await checkpoint("phase_b:orbit_completion_failed");
    return;
  }
  await checkpoint("phase_b:orbit_completion_ok");
  markUiSmoke("orbit_completed_id", SMOKE_ORBIT_TASK_ID);

  const gacosPlan = await planGacosRequest(outputRoot, {
    source_kind: "online_selected",
    selected_scene_ids: [SMOKE_SCENE.scene_id],
    snapshot_scenes: [SMOKE_SCENE],
    bbox: SMOKE_AOI_BBOX,
    output_format: "geotiff",
  });
  if (!gacosPlan.ok || gacosPlan.offline !== true) {
    await checkpoint("phase_b:gacos_plan_failed");
    return;
  }
  await checkpoint("phase_b:gacos_plan_ok");
  markUiSmoke("gacos_plan_offline", true);
  const gacosDryRun = await gacosWebFormDryRun(smokeGacosSubmission());
  if (!gacosDryRun.ok || gacosDryRun.offline !== true) {
    await checkpoint("phase_b:gacos_dry_run_failed");
    return;
  }
  markUiSmoke("gacos_dry_run_offline", true);
  await checkpoint("phase_b:operations_complete");
}

/**
 * Packaged UI smoke driver for the AOI binding vertical.
 *
 * The smoke is a true two-process flow driven by the packaging harness:
 * phase A binds and persists the AOI on an isolated Store, then the packaged
 * EXE restarts on the same Store; phase B restores the AOI without rebinding
 * and runs a real search through a loopback catalogue provider, proving the
 * provider received the restored persisted geometry. No public network
 * request is made.
 */
export async function runUiSmokeAoiVertical(): Promise<void> {
  if (!isUiSmokeMode() || started) return;
  started = true;
  try {
    const phase = await tauriInvoke<string>("ui_smoke_phase");
    if (phase === "phase_a") {
      await runPhaseA();
    } else if (phase === "phase_b") {
      await runPhaseB();
    }
  } catch {
    // A failed smoke run leaves the report incomplete; the host watchdog
    // reports it truthfully instead of fabricating success.
  }
}
