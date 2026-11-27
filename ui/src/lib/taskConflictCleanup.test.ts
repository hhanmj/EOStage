import assert from "node:assert/strict";
import test from "node:test";
import type { TaskConflictFile } from "./generated/v3-contracts.ts";
import {
  conflictCleanupErrorText,
  conflictDeleteSummary,
  conflictFileExpectations,
  conflictFilePreview,
  taskConflictCleanupIsActive,
} from "./taskConflictCleanup.ts";

const file = (index: number): TaskConflictFile => ({
  path: `folder/file-${index}.part.eostage-conflict-test`,
  size_bytes: 1_000_000 + index,
  modified_at_epoch_ms: 100 + index,
  recorded: true,
});

test("cleanup eligibility includes queued/running tasks and active items", () => {
  assert.equal(taskConflictCleanupIsActive({ status: "running", scene_items: [] }), true);
  assert.equal(taskConflictCleanupIsActive({ status: "queued", scene_items: [] }), true);
  assert.equal(taskConflictCleanupIsActive({ status: "paused", scene_items: [
    { scene_id: "item", status: "running", done_bytes: 0 },
  ] }), true);
  assert.equal(taskConflictCleanupIsActive({ status: "paused", scene_items: [
    { scene_id: "item", status: "paused", done_bytes: 0 },
  ] }), false);
  assert.equal(taskConflictCleanupIsActive({ status: "completed", scene_items: null }), false);
});

test("deletion expectations include every listed path, size, and mtime without recorded", () => {
  const files = Array.from({ length: 7 }, (_, index) => file(index));
  assert.deepEqual(conflictFileExpectations(files), files.map(({ path, size_bytes, modified_at_epoch_ms }) => ({
    path, size_bytes, modified_at_epoch_ms,
  })));
  assert.deepEqual(conflictFilePreview(files), { files: files.slice(0, 5), hiddenCount: 2 });
  assert.deepEqual(conflictFilePreview(files.slice(0, 2)), { files: files.slice(0, 2), hiddenCount: 0 });
});

test("delete summary counts only deleted outcomes as deleted", () => {
  assert.deepEqual(conflictDeleteSummary([
    { path: "a", outcome: "deleted" },
    { path: "b", outcome: "deleted" },
    { path: "c", outcome: "skipped_missing" },
    { path: "d", outcome: "skipped_changed" },
    { path: "e", outcome: "skipped_unsafe" },
  ]), { deleted: 2, skipped: 3 });
  assert.deepEqual(conflictDeleteSummary([]), { deleted: 0, skipped: 0 });
});

test("inline errors retain the operation and technical cause", () => {
  assert.equal(conflictCleanupErrorText({ ok: false, error: "disk unavailable", code: "IO" }, "Delete failed"), "Delete failed: disk unavailable");
  assert.equal(conflictCleanupErrorText(new Error("permission denied"), "Delete failed"), "Delete failed: permission denied");
  assert.equal(conflictCleanupErrorText(null, "Delete failed"), "Delete failed");
});
