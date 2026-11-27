import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
} from "./desktopShellBridge.ts";
import { createBrowserFixtureDesktopShellBridge } from "./desktopShellBridgeFixture.ts";

let bridgeImportSequence = 0;

const frozenGeometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
};

function coreSnapshot(globalAoi: Record<string, unknown> | null) {
  return {
    ok: true as const,
    readiness: { contract: "application.v1", status: "ready", store_schema_version: 8 },
    core: {
      readiness: { schema_version: 8, ready: true },
      workspace_context: {
        workspace_id: "workspace-1",
        workspace_root: "C:\\InSAR",
        workspace_name: "InSAR",
        project_id: "project-1",
        project_name: "Project",
        region_id: "region-1",
        region_name: "Region",
      },
      region_aoi: globalAoi,
      region_aois: globalAoi ? [globalAoi] : [],
      dem_source_identity: null,
      tasks: [],
      legacy_task_warnings: [],
      workbench: null,
    },
  };
}

async function importBridgeForTest(
  globalAoiRef: { current: Record<string, unknown> | null },
  planCalls: Array<Record<string, unknown>>,
) {
  const hooks = registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "@/data/adminBoundaries") {
        return { url: new URL("../data/adminBoundaries.ts", import.meta.url).href, shortCircuit: true };
      }
      if (specifier === "./sarSearchForm") {
        return { url: new URL("./sarSearchForm.ts", context.parentURL).href, shortCircuit: true };
      }
      return nextResolve(specifier, context);
    },
  });
  try {
    const base = createBrowserFixtureDesktopShellBridge();
    const shell: DesktopShellBridge = {
      ...base,
      kind: "tauri",
      application: {
        ...base.application,
        coreRestore: async () => coreSnapshot(globalAoiRef.current),
        gacosPlan: async (payload) => {
          planCalls.push(structuredClone(payload));
          return {
            ok: true,
            plan: {
              prepared_batch_id: "prepared-gacos",
              plan_fingerprint_sha256: "a".repeat(64),
            },
            report: { accepted: true },
            offline: true,
          };
        },
        gacosSubmitRequest: async (request) => ({
          ok: true,
          task_id: request.task_id,
          records: [],
          request_groups: [],
          task_status: "completed",
          submitted_count: 1,
          application_completed: true,
          manual_action_required: false,
          automatic_retry_allowed: false,
          message: "Request recorded",
        }),
      },
    };
    registerTestDesktopShellBridge(shell);
    bridgeImportSequence += 1;
    return await import(`./bridge.ts?gacos-plan-${bridgeImportSequence}`);
  } finally {
    hooks.deregister();
  }
}

test("GACOS frozen AOI plans keep manual dates and online batch identity independent of global AOI", async () => {
  const globalAoiRef: { current: Record<string, unknown> | null } = { current: null };
  const planCalls: Array<Record<string, unknown>> = [];
  const bridge = await importBridgeForTest(globalAoiRef, planCalls);
  try {
    const manual = await bridge.planGacosRequest("C:\\GACOS\\manual", {
      aoi_source: "frozen_import",
      aoi_geojson: frozenGeometry,
      aoi_name: "Draft AOI",
      source_kind: "manual_dates",
      manual_dates: ["20240101", "20240229"],
      hour: 6,
      minute: 7,
      output_format: "binary",
      selected_scene_ids: ["stale-scene"],
      snapshot_scenes: [],
    });

    assert.equal(manual.ok, true);
    assert.deepEqual(planCalls[0], {
      aoi_source: "frozen_import",
      aoi_geojson: frozenGeometry,
      aoi_name: "Draft AOI",
      source_kind: "manual_dates",
      manual_dates: ["20240101", "20240229"],
      hour: 6,
      minute: 7,
      output_format: "binary",
      selected_scene_ids: ["stale-scene"],
      snapshot_scenes: [],
      request_id: planCalls[0]?.request_id,
      output_dir: "C:\\GACOS\\manual",
      region_id: null,
      region_safe_name: "Draft AOI",
      scenes: [],
    });
    assert.equal(typeof planCalls[0]?.request_id, "string");

    globalAoiRef.current = {
      region_id: "region-1",
      geometry: { type: "Polygon", coordinates: [[[1, 1], [2, 1], [2, 2], [1, 1]]] },
      bbox: [1, 1, 2, 2],
      source_kind: "global",
      display_name: "Changed global AOI",
    };
    const online = await bridge.planGacosRequest("C:\\GACOS\\online", {
      aoi_source: "frozen_import",
      aoi_geojson: frozenGeometry,
      aoi_name: "Draft AOI",
      source_kind: "online_batch",
      source_batch_id: "sentinel-batch-latest",
      output_format: "geotiff",
      selected_scene_ids: ["stale-scene"],
      snapshot_scenes: [],
    });

    assert.equal(online.ok, true);
    assert.equal(planCalls[1]?.aoi_name, "Draft AOI");
    assert.deepEqual(planCalls[1]?.aoi_geojson, frozenGeometry);
    assert.equal(planCalls[1]?.aoi_source, "frozen_import");
    assert.equal(planCalls[1]?.source_kind, "online_batch");
    assert.equal(planCalls[1]?.source_batch_id, "sentinel-batch-latest");
    assert.equal(planCalls[1]?.output_format, "geotiff");
    assert.deepEqual(planCalls[1]?.scenes, []);
    assert.equal(planCalls[1]?.region_id, null);
    assert.equal(planCalls[1]?.region_safe_name, "Draft AOI");

    const submitted = await bridge.submitGacosRequest(
      {
        prepared_batch_id: "prepared-gacos",
        plan_fingerprint_sha256: "a".repeat(64),
      },
      true,
    );
    assert.equal(submitted.ok, true);
    if (submitted.ok) {
      assert.equal(submitted.manual_action_required, false);
      assert.equal(submitted.application_completed, true);
    }
  } finally {
    clearTestDesktopShellBridge();
  }
});
