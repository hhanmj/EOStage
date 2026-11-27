import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
} from "./desktopShellBridge.ts";

let sequence = 0;

async function importBridgeWithTaskTelemetry(overrides: {
  taskA?: Record<string, unknown>;
  taskB?: Record<string, unknown>;
} = {}) {
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
          readiness: { contract: "application.v1", status: "ready", store_schema_version: 8 },
          core: {
            readiness: { schema_version: 8, ready: true },
            workspace_context: null,
            region_aoi: null,
            region_aois: [],
            tasks: [
              {
                task_id: "task-a",
                batch_id: "batch-a",
                provider_id: "asf.sentinel1",
                created_at: "2026-08-17T00:00:00Z",
                status: "running",
                revision: 1,
                updated_at: "2026-08-17T00:00:03Z",
                selected_count: 3,
                checked_count: 0,
                active_count: 2,
                paused_count: 0,
                queued_count: 0,
                completed_count: 1,
                failed_count: 0,
                skipped_count: 0,
                cancelled_count: 0,
                terminal_bytes: 100,
                active_bytes: 55,
                other_bytes: 0,
                transferred_bytes: 155,
                total_bytes: 600,
                bytes_per_second: 22,
                transfer_active_millis: 4_000,
                active_item: { item_id: "ITEM-B", bytes_completed: 25, total_bytes: 200 },
                output_root: "C:\\downloads",
                layout: "direct",
                operation: null,
                actionable: true,
                ...overrides.taskA,
              },
              {
                task_id: "task-b",
                batch_id: "batch-b",
                provider_id: "asf.sentinel1",
                created_at: "2026-08-17T00:00:00Z",
                status: "running",
                revision: 1,
                updated_at: "2026-08-17T00:00:03Z",
                selected_count: 1,
                checked_count: 0,
                active_count: 1,
                paused_count: 0,
                queued_count: 0,
                completed_count: 0,
                failed_count: 0,
                skipped_count: 0,
                cancelled_count: 0,
                terminal_bytes: 0,
                active_bytes: 5,
                other_bytes: 0,
                transferred_bytes: 5,
                total_bytes: null,
                bytes_per_second: 2,
                transfer_active_millis: 1_000,
                active_item: { item_id: "ITEM-D", bytes_completed: 5, total_bytes: null },
                output_root: "C:\\downloads",
                layout: "direct",
                operation: null,
                actionable: true,
                ...overrides.taskB,
              },
            ],
            legacy_task_warnings: [],
            workbench: null,
          },
        }),
      },
    } as unknown as DesktopShellBridge);
    sequence += 1;
    return await import(`./bridge.ts?download-status-${sequence}`);
  } finally {
    hooks.deregister();
  }
}

async function importBridgeForCreatedTask(
  onCreate?: (request: Record<string, unknown>) => void,
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
  const core = {
    readiness: { schema_version: 11, ready: true },
    workspace_context: null,
    region_aoi: null,
    region_aois: [],
    dem_source_identity: null,
    tasks: [{
      task_id: "task-created",
      batch_id: "batch-created",
      provider_id: "asf.sentinel1",
      created_at: "2026-08-18T00:00:00Z",
      status: "queued",
      revision: 0,
      updated_at: "2026-08-18T00:00:00Z",
      selected_count: 1,
      checked_count: 1,
      active_count: 0,
      paused_count: 0,
      queued_count: 1,
      completed_count: 0,
      failed_count: 0,
      skipped_count: 0,
      cancelled_count: 0,
      terminal_bytes: 0,
      active_bytes: 0,
      other_bytes: 0,
      transferred_bytes: 0,
      total_bytes: null,
      bytes_per_second: 0,
      transfer_active_millis: 0,
      active_item: null,
      output_root: "C:\\downloads",
      layout: "direct",
      operation: null,
      actionable: true,
    }],
    legacy_task_warnings: [],
    workbench: null,
  };
  try {
    registerTestDesktopShellBridge({
      kind: "tauri",
      application: {
        createTaskFromWorkbench: async (request) => {
          onCreate?.(request as unknown as Record<string, unknown>);
          return {
            ok: true,
            readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
            core,
          };
        },
        coreRestore: async () => ({
          ok: true,
          readiness: { contract: "application.v1", status: "ready", store_schema_version: 11 },
          core: { ...core, tasks: [] },
        }),
      },
    } as unknown as DesktopShellBridge);
    sequence += 1;
    return await import(`./bridge.ts?created-download-status-${sequence}`);
  } finally {
    hooks.deregister();
  }
}

test("getDownloadStatus folds bounded task scalars without manufacturing aggregate byte truth", async () => {
  const bridge = await importBridgeWithTaskTelemetry();
  try {
    const status = await bridge.getDownloadStatus();
    assert.equal(status.done_bytes, 100);
    assert.equal(status.current_bytes, 60);
    assert.equal(status.transferred_bytes, 160);
    assert.equal(status.done_bytes! + status.current_bytes!, status.transferred_bytes);
    assert.equal(status.total_bytes, null);
    assert.equal(status.bytes_per_second, 24);
    assert.equal(status.current_scene, "ITEM-B");
    assert.deepEqual(status.active_downloads, [{ scene_id: "ITEM-B", bytes: 25, expected_size: 200 }]);
  } finally {
    clearTestDesktopShellBridge();
  }
});

test("getDownloadStatus rejects known-total, item-count, and rate fold overflow", async () => {
  for (const overrides of [
    { taskA: { total_bytes: Number.MAX_SAFE_INTEGER }, taskB: { total_bytes: Number.MAX_SAFE_INTEGER } },
    { taskA: { selected_count: Number.MAX_SAFE_INTEGER }, taskB: { selected_count: Number.MAX_SAFE_INTEGER } },
    { taskA: { bytes_per_second: Number.MAX_VALUE }, taskB: { bytes_per_second: Number.MAX_VALUE } },
  ]) {
    const bridge = await importBridgeWithTaskTelemetry(overrides);
    try {
      const status = await bridge.getDownloadStatus();
      assert.equal(status.ok, false);
      assert.equal(status.state, "restore_error");
    } finally {
      clearTestDesktopShellBridge();
    }
  }
});

test("task creation returns the authoritative queued status without a second restore", async () => {
  const bridge = await importBridgeForCreatedTask();
  try {
    const created = await bridge.startAsfDownloadSnapshot(
      "C:\\downloads",
      [],
      ["fixture-item"],
      "auto",
      1,
      false,
      "河北省",
      "task-created",
      {
        batch_id: "batch-created",
        created_at: "2026-08-18T00:00:00Z",
        aoi_name: "河北省",
        aoi_geojson: null,
        query: { provider: "ASF", dataset: "SENTINEL-1", max_results: 20 },
        page_index: 0,
        page_size: 20,
        revision: 0,
        total_count: 20,
        candidate_count: 20,
        checked_count: 1,
      },
    );
    const status = (created as unknown as { download_status?: { asf_tasks?: Array<{ task_id?: string; state: string }> } }).download_status;
    assert.equal(created.ok, true);
    assert.equal(status?.asf_tasks?.[0]?.task_id, "task-created");
    assert.equal(status?.asf_tasks?.[0]?.state, "queued");
  } finally {
    clearTestDesktopShellBridge();
  }
});

test("single-row download preserves its exact item without changing the persisted checked set", async () => {
  let request: Record<string, unknown> | null = null;
  const bridge = await importBridgeForCreatedTask((value) => { request = value; });
  try {
    const created = await bridge.startAsfDownloadSnapshot(
      "C:\\downloads",
      [],
      ["fixture-item"],
      "auto",
      1,
      false,
      "河北省",
      "task-created",
      {
        batch_id: "batch-created",
        created_at: "2026-08-18T00:00:00Z",
        aoi_name: "河北省",
        aoi_geojson: null,
        query: { provider: "ASF", dataset: "SENTINEL-1", max_results: 20 },
        page_index: 0,
        page_size: 20,
        revision: 0,
        total_count: 20,
        candidate_count: 20,
        checked_count: 0,
      },
      "fixture-item",
    );
    assert.equal(created.ok, true);
    assert.equal(request?.exact_item_id, "fixture-item");
  } finally {
    clearTestDesktopShellBridge();
  }
});
