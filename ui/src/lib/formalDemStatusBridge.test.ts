import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
} from "./desktopShellBridge.ts";

let sequence = 0;

async function importBridgeForDem(calls: { core: boolean[]; taskPage: number }) {
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
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        coreRestore: async (includeTerminal = false) => {
          calls.core.push(includeTerminal);
          return {
            ok: true,
            readiness: { contract: "application.v1", status: "ready", store_schema_version: 8 },
            core: {
              readiness: { schema_version: 8, ready: true },
              workspace_context: null,
              region_aoi: null,
              region_aois: [],
              tasks: [{
                task_id: "dem-running",
                batch_id: "dem-batch",
                provider_id: "COP30",
                created_at: "2026-08-17T00:00:00Z",
                status: "running",
                revision: 4,
                updated_at: "2026-08-17T00:00:01Z",
                selected_count: 1,
                checked_count: 1,
                active_count: 1,
                paused_count: 0,
                queued_count: 0,
                completed_count: 0,
                failed_count: 0,
                skipped_count: 0,
                cancelled_count: 0,
                terminal_bytes: 0,
                active_bytes: 12,
                other_bytes: 0,
                transferred_bytes: 12,
                total_bytes: 100,
                bytes_per_second: 5,
                transfer_active_millis: 1_500,
                active_item: { item_id: "tile", bytes_completed: 12, total_bytes: 100 },
                output_root: "C:\\dem",
                layout: "direct",
                operation: {
                  operation_id: "dem-op",
                  kind: "dem",
                  phase: "download",
                  region_id: "standalone-dem",
                  approved_worker_operation: "dem_download",
                },
                actionable: true,
              }],
              legacy_task_warnings: [],
              workbench: null,
            },
          };
        },
        taskPage: async () => {
          calls.taskPage += 1;
          throw new Error("list status must not load task detail");
        },
        getDemTaskSettings: async () => ({
          max_concurrent_tasks: 2,
          max_concurrent_conversions: 1,
        }),
      },
    } as unknown as DesktopShellBridge);
    sequence += 1;
    return await import(`./bridge.ts?formal-dem-status-${sequence}`);
  } finally {
    hooks.deregister();
  }
}

async function importBridgeWithoutFormalRegion() {
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
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        coreRestore: async () => ({
          ok: true,
          readiness: { contract: "application.v1", status: "ready", store_schema_version: 15 },
          core: {
            readiness: { schema_version: 15, ready: true },
            workspace_context: null,
            region_aoi: null,
            region_aois: [],
            dem_source_identity: null,
            tasks: [],
            legacy_task_warnings: [],
            workbench: null,
          },
        }),
      },
    } as unknown as DesktopShellBridge);
    sequence += 1;
    return await import(`./bridge.ts?formal-orbit-empty-${sequence}`);
  } finally {
    hooks.deregister();
  }
}

test("formal DEM list status performs one CoreRestore(false), no terminal restore, and zero task-page N+1 reads", async () => {
  const calls = { core: [] as boolean[], taskPage: 0 };
  const bridge = await importBridgeForDem(calls);
  try {
    const status = await bridge.getDemDownloadStatus();
    assert.equal(status.ok, true);
    if (status.ok) {
      assert.equal(status.dem_tasks.length, 1);
      assert.equal(status.dem_tasks[0].task_id, "dem-running");
      assert.equal(status.dem_tasks[0].state, "running");
    }
    assert.deepEqual(calls.core, [false]);
    assert.equal(calls.taskPage, 0);

    const context = await bridge.getContext();
    assert.equal(context.ok, true);
    assert.deepEqual(calls.core, [false, false], "ordinary context restore is active-only too");
    assert.equal(calls.core.filter(Boolean).length, 0);
    assert.equal(calls.taskPage, 0);
  } finally {
    clearTestDesktopShellBridge();
  }
});

test("restart without a formal region keeps Orbit idle instead of fabricating a failed task", async () => {
  const bridge = await importBridgeWithoutFormalRegion();
  try {
    const status = await bridge.getOrbitDownloadStatus();
    assert.equal(status.state, "idle");
    assert.equal(status.error, null);
    assert.equal(status.task_id, undefined);
    assert.equal(status.total, 0);
  } finally {
    clearTestDesktopShellBridge();
  }
});
