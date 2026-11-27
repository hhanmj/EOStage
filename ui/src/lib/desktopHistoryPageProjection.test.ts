import assert from "node:assert/strict";
import test from "node:test";
import type { CoreTaskSummaryProjection } from "./coreRestoreProjection.ts";
import {
  dedupeAndMergeHistoryPageTasks,
  mapCoreTaskSummaryToApplicationTask,
  mapCoreTaskSummaryToStoreTaskSummary,
  mapHistoryPageItemsToApplicationTasks,
  mergeOverviewAndHistoryPageTasks,
  mergeOverviewWithHistoryPageTasks,
  projectHistoryTaskPageToApplicationTasks,
  projectHistoryTaskSummaryToApplicationTask,
  type DesktopValidatedHistoryTaskEntry,
} from "./desktopHistoryPageProjection.ts";
import type { ApplicationTask } from "../shell/api/types.ts";

function makeCoreTaskSummary(
  overrides: Partial<CoreTaskSummaryProjection> = {},
): CoreTaskSummaryProjection {
  return {
    task_id: "task-hist-1",
    batch_id: "batch-hist-1",
    provider_id: "asf.catalog",
    dataset_id: "SENTINEL-1",
    concurrency: 2,
    task_type: null,
    created_at: "2026-10-20T08:00:00Z",
    status: "completed",
    revision: 4,
    updated_at: "2026-10-20T08:30:00Z",
    selected_count: 3,
    delivery_requested_count: 3,
    delivery_rejected_count: 0,
    checked_count: 3,
    active_count: 0,
    paused_count: 0,
    queued_count: 0,
    completed_count: 3,
    failed_count: 0,
    skipped_count: 0,
    cancelled_count: 0,
    terminal_bytes: 0,
    active_bytes: 0,
    other_bytes: 0,
    transferred_bytes: 0,
    total_bytes: null,
    source_total_bytes: 12_000_000,
    bytes_per_second: 0,
    transfer_active_millis: 15_000,
    active_item: null,
    output_root: "D:/InSAR_Data/s1",
    layout: "direct",
    operation: null,
    actionable: false,
    ...overrides,
  };
}

function makeApplicationTask(
  overrides: Partial<ApplicationTask> & { task_id: string },
): ApplicationTask {
  return {
    revision: 1,
    category: "scene_download",
    name: `Task ${overrides.task_id}`,
    provider: "ASF",
    source_provider_id: "asf.catalog",
    task_type: null,
    dataset: "SENTINEL-1",
    status: "completed",
    done_bytes: 0,
    total_bytes: null,
    output_dir: "D:/InSAR_Data/s1",
    created_at: "2026-10-20T08:00:00Z",
    updated_at: "2026-10-20T08:30:00Z",
    failure: null,
    pause_reason: null,
    source_task_id: null,
    auxiliary_kind: null,
    ...overrides,
  };
}

test("mapCoreTaskSummaryToStoreTaskSummary maps core fields and never fabricates failure or pause_reason", () => {
  const core = makeCoreTaskSummary({
    task_id: "task-failed-hist",
    status: "failed",
    failed_count: 2,
    completed_count: 1,
    failure_codes: [{ code: "DISK_LOW", local: true }],
  });

  const storeSummary = mapCoreTaskSummaryToStoreTaskSummary(core);

  assert.equal(storeSummary.task_id, "task-failed-hist");
  assert.equal(storeSummary.batch_id, core.batch_id);
  assert.equal(storeSummary.provider_id, "asf.catalog");
  assert.equal(storeSummary.dataset_id, "SENTINEL-1");
  assert.equal(storeSummary.status, "failed");
  assert.equal(storeSummary.revision, 4);
  assert.equal(storeSummary.selected_count, 3);
  assert.equal(storeSummary.completed_count, 1);
  assert.equal(storeSummary.failed_count, 2);
  assert.equal(storeSummary.output_root, "D:/InSAR_Data/s1");
  assert.equal(storeSummary.failure, null);
  assert.equal(storeSummary.pause_reason, null);
  assert.equal(storeSummary.source_task_id, null);
  assert.equal(storeSummary.auxiliary_kind, null);
});

test("projectHistoryTaskSummaryToApplicationTask accepts both raw CoreTaskSummaryProjection and { archived_at, task } entry with bytes_reported: false", () => {
  const core = makeCoreTaskSummary({
    task_id: "task-hist-direct",
    dataset_id: "SENTINEL-1",
    transferred_bytes: 0,
    total_bytes: null,
  });
  const entry: DesktopValidatedHistoryTaskEntry = {
    archived_at: "2026-10-20T09:00:00Z",
    task: core,
  };

  const fromDirect = projectHistoryTaskSummaryToApplicationTask(core);
  const fromEntry = projectHistoryTaskSummaryToApplicationTask(entry);
  const fromAlias = mapCoreTaskSummaryToApplicationTask(entry);

  assert.deepEqual(fromEntry, fromDirect);
  assert.deepEqual(fromAlias, fromDirect);
  assert.equal(fromDirect.task_id, "task-hist-direct");
  assert.equal(fromDirect.category, "scene_download");
  assert.equal(fromDirect.bytes_reported, false);
  assert.equal(fromDirect.failure, null);
  assert.equal(fromDirect.pause_reason, null);
  assert.equal(fromDirect.source_task_id, null);
  assert.equal(fromDirect.auxiliary_kind, null);
  assert.equal(fromDirect.output_dir, "D:/InSAR_Data/s1");
});

test("projectHistoryTaskPageToApplicationTasks preserves page order and maps dem.* dataset_id to dem_convert", () => {
  const items: DesktopValidatedHistoryTaskEntry[] = [
    {
      archived_at: "2026-10-21T10:00:00Z",
      task: makeCoreTaskSummary({
        task_id: "task-dem-1",
        provider_id: "copernicus.dem",
        dataset_id: "dem.cop30.v1",
      }),
    },
    {
      archived_at: "2026-10-20T10:00:00Z",
      task: makeCoreTaskSummary({
        task_id: "task-sar-2",
        provider_id: "asf.catalog",
        dataset_id: "SLC-BURST",
        concurrency: undefined,
        delivery_requested_count: undefined,
        delivery_rejected_count: undefined,
        source_total_bytes: undefined,
      }),
    },
  ];

  const projected = projectHistoryTaskPageToApplicationTasks(items);
  assert.deepEqual(mapHistoryPageItemsToApplicationTasks(items), projected);
  assert.equal(projected.length, 2);
  assert.equal(projected[0]?.task_id, "task-dem-1");
  assert.equal(projected[0]?.category, "dem_convert");
  assert.equal(projected[0]?.bytes_reported, false);
  assert.equal(projected[1]?.task_id, "task-sar-2");
  assert.equal(projected[1]?.category, "scene_download");
  assert.equal(projected[1]?.bytes_reported, false);
});

test("mergeOverviewAndHistoryPageTasks keeps overview's newer snapshot on overlap while preserving historyPage order", () => {
  const overviewTasks: ApplicationTask[] = [
    makeApplicationTask({ task_id: "queue-running", status: "running", revision: 5, done_bytes: 500 }),
    makeApplicationTask({ task_id: "queue-paused", status: "paused", revision: 3 }),
    makeApplicationTask({
      task_id: "hist-2",
      status: "failed",
      revision: 9,
      bytes_reported: false,
      failure: { message_zh: "磁盘空间不足。", codes: ["DISK_LOW"], failed_items: 1, total_items: 2 },
    }),
    makeApplicationTask({
      task_id: "hist-1",
      status: "completed",
      revision: 8,
      bytes_reported: false,
    }),
  ];

  const historyPageTasks: ApplicationTask[] = [
    makeApplicationTask({ task_id: "hist-1", status: "completed", revision: 4, bytes_reported: false }),
    makeApplicationTask({ task_id: "hist-2", status: "failed", revision: 4, bytes_reported: false, failure: null }),
    makeApplicationTask({ task_id: "hist-3", status: "cancelled", revision: 2, bytes_reported: false }),
  ];

  const merged = mergeOverviewAndHistoryPageTasks(overviewTasks, historyPageTasks);
  assert.deepEqual(mergeOverviewWithHistoryPageTasks(overviewTasks, historyPageTasks), merged);
  assert.deepEqual(dedupeAndMergeHistoryPageTasks(overviewTasks, historyPageTasks), merged);

  assert.deepEqual(
    merged.map((t) => t.task_id),
    ["queue-running", "queue-paused", "hist-1", "hist-2", "hist-3"],
  );
  // Overlapping history tasks take the newer overview snapshot (e.g. carrying failure summary & latest revision)
  assert.equal(merged[2]?.revision, 8);
  assert.equal(merged[3]?.revision, 9);
  assert.equal(merged[3]?.failure?.codes?.[0], "DISK_LOW");
  assert.equal(merged[4]?.revision, 2);
});

test("mergeOverviewAndHistoryPageTasks keeps active/queued/paused overview tasks in the queue prefix even if present in stale historyPageTasks", () => {
  const overviewTasks: ApplicationTask[] = [
    makeApplicationTask({ task_id: "task-resumed", status: "queued", revision: 10 }),
    makeApplicationTask({ task_id: "hist-fresh", status: "completed", revision: 6, bytes_reported: false }),
  ];

  const staleHistoryPageTasks: ApplicationTask[] = [
    makeApplicationTask({ task_id: "hist-old-1", status: "completed", revision: 2, bytes_reported: false }),
    makeApplicationTask({ task_id: "task-resumed", status: "failed", revision: 3, bytes_reported: false }),
    makeApplicationTask({ task_id: "hist-old-2", status: "cancelled", revision: 1, bytes_reported: false }),
  ];

  const merged = mergeOverviewAndHistoryPageTasks(overviewTasks, staleHistoryPageTasks);

  assert.deepEqual(
    merged.map((t) => ({ id: t.task_id, status: t.status, revision: t.revision })),
    [
      { id: "task-resumed", status: "queued", revision: 10 },
      { id: "hist-fresh", status: "completed", revision: 6 },
      { id: "hist-old-1", status: "completed", revision: 2 },
      { id: "hist-old-2", status: "cancelled", revision: 1 },
    ],
  );
});
