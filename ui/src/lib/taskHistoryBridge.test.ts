import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
  type DesktopShellBridge,
  type DesktopTaskHistoryPageRequest,
} from "./desktopShellBridge.ts";
import { createTauriDesktopShellBridge, type TauriBridgeDependencies } from "./desktopShellBridgeTauri.ts";

const archivedAt = "2026-08-17T00:00:00Z";
const cursor = { archived_at_epoch_ms: Date.parse(archivedAt), task_id: "task-history" };

function summary(overrides: Record<string, unknown> = {}) {
  return {
    archived_at: archivedAt,
    task_id: "task-history",
    batch_id: "batch-history",
    provider_id: "asf.sentinel1",
    created_at: "2026-08-16T00:00:00Z",
    status: "completed",
    revision: 9,
    updated_at: "2026-08-17T00:00:00Z",
    selected_count: 1,
    checked_count: 1,
    active_count: 0,
    paused_count: 0,
    queued_count: 0,
    completed_count: 1,
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
    output_root: "C:\\history",
    layout: "sentinel1_slc",
    operation: {
      operation_id: "op-history",
      kind: "dem",
      phase: "download",
      region_id: "region-history",
      approved_worker_operation: "dem_download",
    },
    actionable: true,
    ...overrides,
  };
}

function dependencies(result: unknown, calls: Array<[string, unknown]>): TauriBridgeDependencies {
  return {
    invoke: async (command, args) => {
      calls.push([command, args]);
      return result;
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
}

test("Tauri history uses one exact bounded keyset command and rejects full or malformed projections", async () => {
  const request: DesktopTaskHistoryPageRequest = {
    page_size: 40,
    after_archived_at_epoch_ms: cursor.archived_at_epoch_ms,
    after_task_id: cursor.task_id,
  };
  const calls: Array<[string, unknown]> = [];
  const page = { page_size: 40, has_more: true, next_cursor: cursor, items: [summary()] };
  const bridge = createTauriDesktopShellBridge(dependencies(page, calls), "tauri://localhost/", false);
  assert.deepEqual(await bridge.application.taskHistoryPage(request), { ok: true, page });
  assert.deepEqual(calls, [["application_download_history_page", { request }]]);

  const earlyTimestampPage = {
    page_size: 40,
    has_more: false,
    next_cursor: null,
    items: [
      summary({ task_id: "task-year-0000", archived_at: "0000-01-01T00:00:00Z" }),
      summary({ task_id: "task-year-0099", archived_at: "0099-08-17T00:00:00Z" }),
    ],
  };
  const earlyTimestampBridge = createTauriDesktopShellBridge(
    dependencies(earlyTimestampPage, []),
    "tauri://localhost/",
    false,
  );
  assert.equal(
    (await earlyTimestampBridge.application.taskHistoryPage({ page_size: 40 })).ok,
    true,
    "years 0000-0099 accepted by Rust must remain valid at the strict Tauri boundary",
  );

  const invalidRequestCalls: Array<[string, unknown]> = [];
  const requestRejected = createTauriDesktopShellBridge(dependencies(page, invalidRequestCalls), "tauri://localhost/", false);
  assert.equal((await requestRejected.application.taskHistoryPage({
    page_size: 40,
    after_archived_at_epoch_ms: cursor.archived_at_epoch_ms,
    after_task_id: "x".repeat(129),
  })).ok, false);
  assert.equal(invalidRequestCalls.length, 0, "invalid cursor never invokes Tauri");

  for (const invalid of [
    { ...page, has_more: false },
    { ...page, next_cursor: { archived_at_epoch_ms: 1.5, task_id: "task-history" } },
    { ...page, items: [summary({ selected_item_ids: ["secret"] })] },
    { ...page, items: [summary({ snapshot_json: "{}" })] },
    { ...page, items: [summary({ dataset_id: "x".repeat(129) })] },
    { ...page, items: Array.from({ length: 101 }, () => summary()) },
  ]) {
    const rejected = createTauriDesktopShellBridge(dependencies(invalid, []), "tauri://localhost/", false);
    assert.equal((await rejected.application.taskHistoryPage(request)).ok, false);
  }
});

let sequence = 0;

async function importBridgeWithHistory(page: Record<string, unknown>, calls: DesktopTaskHistoryPageRequest[]) {
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
        taskHistoryPage: async (request: DesktopTaskHistoryPageRequest) => {
          calls.push(request);
          return { ok: true, page };
        },
      },
    } as unknown as DesktopShellBridge);
    sequence += 1;
    return await import(`./bridge.ts?task-history-${sequence}`);
  } finally {
    hooks.deregister();
  }
}

test("strict UI history projection preserves archived_at/revision/operation and rejects cursor drift before invoke", async () => {
  const calls: DesktopTaskHistoryPageRequest[] = [];
  const bridge = await importBridgeWithHistory(
    { page_size: 40, has_more: true, next_cursor: cursor, items: [summary()] },
    calls,
  );
  try {
    const valid = await bridge.loadApplicationTaskHistoryPage({ page_size: 40 });
    assert.equal(valid.ok, true);
    if (valid.ok) {
      assert.equal(valid.items[0].archived_at, archivedAt);
      assert.equal(valid.items[0].task.revision, 9);
      assert.equal(valid.items[0].task.operation?.operation_id, "op-history");
      assert.deepEqual(valid.next_cursor, cursor);
    }
    assert.equal(calls.length, 1);
    const archive = await bridge.getDownloadArchive();
    assert.equal(archive.ok, true);
    if (archive.ok) {
      assert.equal(archive.items[0].archived_at, archivedAt);
      assert.equal(archive.items[0].revision, 9);
      assert.equal(archive.items[0].status, "finished");
      assert.equal(archive.items[0].ts, Date.parse(archivedAt));
    }
    assert.equal(calls.length, 2);

    for (const invalid of [
      { page_size: 0 },
      { page_size: 101 },
      { page_size: 40, after_archived_at_epoch_ms: cursor.archived_at_epoch_ms },
      { page_size: 40, after_archived_at_epoch_ms: 1.5, after_task_id: cursor.task_id },
      { page_size: 40, after_archived_at_epoch_ms: Number.MAX_SAFE_INTEGER + 1, after_task_id: cursor.task_id },
      { page_size: 40, after_archived_at_epoch_ms: cursor.archived_at_epoch_ms, after_task_id: " task-history" },
      { page_size: 40, after_archived_at_epoch_ms: cursor.archived_at_epoch_ms, after_task_id: "x".repeat(129) },
    ] as DesktopTaskHistoryPageRequest[]) {
      assert.equal((await bridge.loadApplicationTaskHistoryPage(invalid)).ok, false);
    }
    assert.equal(calls.length, 2, "invalid cursors never cross the typed bridge");
  } finally {
    clearTestDesktopShellBridge();
  }
});

test("history task names use persisted dataset authority instead of provider implementation labels", async () => {
  const calls: DesktopTaskHistoryPageRequest[] = [];
  const bridge = await importBridgeWithHistory(
    {
      page_size: 40,
      has_more: false,
      next_cursor: null,
      items: [summary({
        provider_id: "asf.catalog",
        dataset_id: "SENTINEL-1",
        operation: null,
      })],
    },
    calls,
  );
  try {
    const archive = await bridge.getDownloadArchive();
    assert.equal(archive.ok, true);
    if (archive.ok) assert.equal(archive.items[0].name, "Sentinel-1 下载任务");
  } finally {
    clearTestDesktopShellBridge();
  }
});

test("strict application history projection preserves Rust-supported years 0000-0099", async () => {
  const calls: DesktopTaskHistoryPageRequest[] = [];
  const earlyArchivedAt = ["0000-01-01T00:00:00Z", "0099-08-17T00:00:00Z"];
  const bridge = await importBridgeWithHistory(
    {
      page_size: 40,
      has_more: false,
      next_cursor: null,
      items: earlyArchivedAt.map((archived_at, index) => summary({
        task_id: `task-year-${index}`,
        archived_at,
      })),
    },
    calls,
  );
  try {
    const result = await bridge.loadApplicationTaskHistoryPage({ page_size: 40 });
    assert.equal(result.ok, true);
    if (result.ok) assert.deepEqual(result.items.map((item) => item.archived_at), earlyArchivedAt);
    assert.equal(calls.length, 1);
  } finally {
    clearTestDesktopShellBridge();
  }
});
