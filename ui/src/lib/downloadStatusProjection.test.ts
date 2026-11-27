import assert from "node:assert/strict";
import test from "node:test";

import { projectDownloadStatusFromCoreTask } from "./downloadStatusProjection.ts";
import type { CoreTaskSummaryProjection } from "./coreRestoreProjection.ts";

test("NISAR task status preserves task type, real concurrency, and full lifecycle timestamps", () => {
  const task = {
    task_id: "task-nisar-subset",
    batch_id: "batch-1",
    provider_id: "asf.nasa.earthdata",
    dataset_id: "sar.nisar.l.rslc.provisional.v1",
    task_type: "nisar_subset",
    concurrency: 2,
    created_at: "2026-09-04T07:42:18Z",
    status: "completed",
    revision: 5,
    updated_at: "2026-09-04T07:45:21Z",
    selected_count: 1,
    checked_count: 1,
    active_count: 0,
    paused_count: 0,
    queued_count: 0,
    completed_count: 1,
    failed_count: 0,
    skipped_count: 0,
    cancelled_count: 0,
    terminal_bytes: 8,
    active_bytes: 0,
    other_bytes: 0,
    transferred_bytes: 8,
    network_bytes_received: 19,
    source_total_bytes: 42_756_734_979,
    reusable_verified_bytes: 49_686_192,
    reused_verified_bytes: 47_876_062,
    total_bytes: 8,
    bytes_per_second: 0,
    transfer_active_millis: 3_000,
    active_item: null,
    output_root: "C:\\downloads",
    layout: "direct",
    operation: null,
    actionable: true,
  } satisfies CoreTaskSummaryProjection;
  const status = projectDownloadStatusFromCoreTask(task);
  assert.equal(status.task_type, "nisar_subset");
  assert.equal(status.concurrency, 2);
  assert.equal(status.created_at, task.created_at);
  assert.equal(status.ended_at, task.updated_at);
  assert.equal(status.state, "completed");
  assert.equal(status.resume_supported, true);
  assert.equal(status.network_bytes_received, 19);
  assert.equal(status.source_total_bytes, 42_756_734_979);
  assert.equal(status.reusable_verified_bytes, 49_686_192);
  assert.equal(status.reused_verified_bytes, 47_876_062);
});

test("task status projects Store-owned aggregate and representative active telemetry", () => {
  const task = {
    task_id: "task-telemetry",
    batch_id: "batch-1",
    provider_id: "asf.sentinel1",
    dataset_id: "SENTINEL-1",
    created_at: "2026-08-15T00:00:00Z",
    status: "running",
    revision: 3,
    updated_at: "2026-08-15T00:00:03Z",
    selected_count: 3,
    delivery_requested_count: 5,
    delivery_rejected_count: 2,
    checked_count: 1,
    active_count: 1,
    paused_count: 0,
    queued_count: 1,
    completed_count: 1,
    failed_count: 0,
    skipped_count: 0,
    cancelled_count: 0,
    terminal_bytes: 100,
    active_bytes: 55,
    other_bytes: 0,
    transferred_bytes: 155,
    reusable_verified_bytes: null,
    reused_verified_bytes: null,
    total_bytes: null,
    bytes_per_second: 10,
    transfer_active_millis: 3_210,
    active_item: { item_id: "ITEM-B", bytes_completed: 25, total_bytes: 200 },
    output_root: "C:\\downloads",
    layout: "direct",
    operation: null,
    actionable: true,
  } satisfies CoreTaskSummaryProjection;
  const status = projectDownloadStatusFromCoreTask(task);

  assert.equal(status.done_bytes, 100);
  assert.equal(status.total_bytes, null);
  assert.equal(status.current_bytes, 55);
  assert.equal(status.bytes_per_second, 10);
  assert.equal(status.transfer_active_millis, 3_210);
  assert.equal(status.elapsed_seconds, 3.21);
  assert.equal(status.current_scene, "ITEM-B");
  assert.equal(status.admission_requested_count, 5);
  assert.equal(status.admission_rejected_count, 2);
  assert.equal(status.resume_supported, true);
  assert.equal(status.reusable_verified_bytes, null);
  assert.equal(status.reused_verified_bytes, null);
  assert.deepEqual(status.active_downloads, [{ scene_id: "ITEM-B", bytes: 25, expected_size: 200 }]);
  assert.notEqual(status.done_bytes + status.current_bytes, 180);
  assert.notEqual(status.done_bytes + status.current_bytes + 25, 185);
  assert.equal(status.active_progress, null);
  assert.equal(status.failure_codes, null);
});

test("task status carries the Store progress groups and failure codes for the task card", () => {
  const task = {
    task_id: "task-nisar-phase",
    batch_id: "batch-1",
    provider_id: "asf.nasa.earthdata",
    dataset_id: "sar.nisar.l.rslc.provisional.v1",
    task_type: "nisar_subset",
    created_at: "2026-10-17T00:00:00Z",
    status: "running",
    revision: 3,
    updated_at: "2026-10-17T00:00:03Z",
    selected_count: 3,
    checked_count: 0,
    active_count: 2,
    paused_count: 0,
    queued_count: 0,
    completed_count: 0,
    failed_count: 1,
    skipped_count: 0,
    cancelled_count: 0,
    terminal_bytes: 0,
    active_bytes: 10,
    other_bytes: 0,
    transferred_bytes: 10,
    total_bytes: null,
    bytes_per_second: 4,
    transfer_active_millis: 1_000,
    active_item: { item_id: "ITEM-A", bytes_completed: 10, total_bytes: null },
    active_progress: { without_progress: 0, transferring: 1, idle: 1 },
    failure_codes: [{ code: "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED", local: false }],
    output_root: "C:\\downloads",
    layout: "direct",
    operation: null,
    actionable: true,
  } satisfies CoreTaskSummaryProjection;
  const status = projectDownloadStatusFromCoreTask(task);
  assert.deepEqual(status.active_progress, { without_progress: 0, transferring: 1, idle: 1 });
  assert.deepEqual(status.failure_codes, [{ code: "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED", local: false }]);
});

test("a queued task whose only remaining items are paused projects as paused so the card can continue it", () => {
  const base = {
    task_id: "task-item-paused",
    batch_id: "batch-1",
    provider_id: "copernicus.dataspace.odata",
    dataset_id: "optical.sentinel-2.l2a",
    created_at: "2026-10-22T14:09:11Z",
    status: "queued",
    revision: 2,
    updated_at: "2026-10-22T14:09:36Z",
    selected_count: 1,
    checked_count: 1,
    active_count: 0,
    paused_count: 1,
    queued_count: 0,
    completed_count: 0,
    failed_count: 0,
    skipped_count: 0,
    cancelled_count: 0,
    terminal_bytes: 0,
    active_bytes: 0,
    other_bytes: 54_164_909,
    transferred_bytes: 54_164_909,
    total_bytes: 1_163_351_831,
    bytes_per_second: 0,
    transfer_active_millis: 22_000,
    active_item: null,
    output_root: "C:\downloads",
    layout: "direct",
    operation: null,
    actionable: true,
  } satisfies CoreTaskSummaryProjection;

  const dormant = projectDownloadStatusFromCoreTask(base);
  assert.equal(dormant.state, "paused");
  assert.equal(dormant.paused, true);
  assert.equal(dormant.task_paused, true);

  const waitingForSlot = projectDownloadStatusFromCoreTask({ ...base, paused_count: 1, queued_count: 1 });
  assert.equal(waitingForSlot.state, "queued");
  assert.equal(waitingForSlot.paused, false);
});
