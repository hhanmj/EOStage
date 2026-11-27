import assert from "node:assert/strict";
import test from "node:test";

import {
  createTauriDesktopShellBridge,
  type TauriBridgeDependencies,
} from "./desktopShellBridgeTauri.ts";
import type {
  DesktopAppendTaskFromSourceBatchRequest,
  DesktopBurstAppendAssetTaskRequest,
  DesktopBurstCreateAssetTaskRequest,
  DesktopBurstCreateHistoricalAssetTaskRequest,
  DesktopCreateTaskFromHistoricalSourceBatchRequest,
  DesktopTaskPageRequest,
  DesktopTaskSourceBatchPageRequest,
} from "./desktopShellBridge.ts";

test("Tauri exposes a bounded task page through the named application command", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_download_task_page") {
        return {
          task_id: "task-page",
          revision: 3,
          page_index: 0,
          page_size: 100,
          total_count: 10_000,
          items: [],
        };
      }
      return {};
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
  const request: DesktopTaskPageRequest = {
    task_id: "task-page",
    expected_revision: 3,
    page_index: 0,
    page_size: 100,
  };

  assert.deepEqual(await bridge.application.taskPage(request), {
    ok: true,
    page: {
      task_id: "task-page",
      revision: 3,
      page_index: 0,
      page_size: 100,
      total_count: 10_000,
      items: [],
    },
  });
  assert.deepEqual(calls, [["application_download_task_page", { request }]]);
});

test("Tauri routes Burst create, append and terminal successor with explicit source product IDs", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      return {
        readiness: { contract: "application.v1", status: "ready", store_schema_version: 19 },
        core: { tasks: [], history: [], workbench: null },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
  const create: DesktopBurstCreateAssetTaskRequest = {
    request_id: "burst-create",
    source_batch_id: "batch-frozen",
    selected_product_ids: ["BURST-A"],
    asset_policy: "all",
    expected_workbench_revision: 9,
    task_id: "burst-task",
    output_root: "C:/downloads",
    layout: "direct",
    concurrency: 2,
    created_at: "2026-08-26T00:00:00Z",
  };
  const append: DesktopBurstAppendAssetTaskRequest = {
    request_id: "burst-append",
    task_id: "burst-task",
    selected_product_ids: ["BURST-B"],
    expected_task_revision: 12,
    event_id: "burst-append-event",
    occurred_at: "2026-08-26T00:01:00Z",
  };
  const successor: DesktopBurstCreateHistoricalAssetTaskRequest = {
    request_id: "burst-successor",
    source_task_id: "burst-task",
    expected_source_task_revision: 14,
    selected_product_ids: ["BURST-C"],
    new_task_id: "burst-task-new",
    output_root: "C:/downloads",
    layout: "direct",
    concurrency: 2,
    created_at: "2026-08-26T00:02:00Z",
  };

  await bridge.application.createBurstAssetTask(create);
  await bridge.application.appendBurstAssetTask(append);
  await bridge.application.createHistoricalBurstAssetTask(successor);

  assert.deepEqual(calls, [
    ["application_create_burst_asset_task", { request: create }],
    ["application_append_burst_asset_task", { request: append }],
    ["application_create_historical_burst_asset_task", { request: successor }],
  ]);
});

test("Tauri keeps task source reads revision-independent and routes explicit-ID mutations", async () => {
  const calls: Array<[string, Record<string, unknown> | undefined]> = [];
  const dependencies: TauriBridgeDependencies = {
    invoke: async (command, args) => {
      calls.push([command, args]);
      if (command === "application_load_task_source_batch_page") {
        return {
          task_id: "task-source",
          source_batch_id: "batch-frozen",
          task_revision: 7,
          task_status: "paused",
          page_index: 1,
          page_size: 24,
          total_count: 300,
          items: [],
        };
      }
      return {
        readiness: { contract: "application.v1", status: "ready", store_schema_version: 19 },
        core: { tasks: [], history: [], workbench: null },
      };
    },
    getCurrentWindow: () => ({
      minimize: async () => undefined,
      toggleMaximize: async () => undefined,
      close: async () => undefined,
      innerSize: async () => ({ width: 100, height: 100 }),
      startResizeDragging: async () => undefined,
    }),
    open: async () => null,
  };
  const bridge = createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false);
  const read: DesktopTaskSourceBatchPageRequest = {
    task_id: "task-source",
    page_index: 1,
    page_size: 24,
  };
  const append: DesktopAppendTaskFromSourceBatchRequest = {
    task_id: "task-source",
    expected_task_revision: 7,
    item_ids: ["ITEM-B"],
    event_id: "append-b",
    occurred_at: "2026-08-25T00:00:00Z",
  };
  const successor: DesktopCreateTaskFromHistoricalSourceBatchRequest = {
    source_task_id: "task-source",
    expected_source_task_revision: 7,
    new_task_id: "task-successor",
    item_ids: ["ITEM-C"],
    output_root: "C:/downloads",
    layout: "direct",
    concurrency: 2,
    created_at: "2026-08-25T00:01:00Z",
  };

  assert.deepEqual(await bridge.application.taskSourceBatchPage(read), {
    ok: true,
    page: {
      task_id: "task-source",
      source_batch_id: "batch-frozen",
      task_revision: 7,
      task_status: "paused",
      page_index: 1,
      page_size: 24,
      total_count: 300,
      items: [],
    },
  });
  await bridge.application.appendTaskFromSourceBatch(append);
  await bridge.application.createTaskFromHistoricalSourceBatch(successor);
  assert.deepEqual(calls, [
    ["application_load_task_source_batch_page", { request: read }],
    ["application_append_task_from_source_batch", { request: append }],
    ["application_create_task_from_historical_source_batch", { request: successor }],
  ]);
});
