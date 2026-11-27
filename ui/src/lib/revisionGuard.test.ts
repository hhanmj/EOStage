import assert from "node:assert/strict";
import test from "node:test";

import { RevisionGuard } from "./revisionGuard.ts";

test("older snapshots and duplicate revisions cannot overwrite newer state", () => {
  const guard = new RevisionGuard();
  assert.equal(guard.acceptSnapshot("task-1", 2), true);
  assert.equal(guard.acceptSnapshot("task-1", 1), false);
  assert.equal(guard.acceptSnapshot("task-1", 2), false);
  assert.equal(guard.acceptSnapshot("task-2", 1), true);
});

test("reconnect allows a fresh snapshot while keeping streams isolated", () => {
  const guard = new RevisionGuard();
  assert.equal(guard.acceptSnapshot("task-1", 4), true);
  guard.reconnect("task-1");
  assert.equal(guard.acceptSnapshot("task-1", 0), true);
  assert.equal(guard.acceptSnapshot("task-2", 0), true);
});

test("duplicate and stale events are rejected with bounded event memory", () => {
  const guard = new RevisionGuard(2);
  assert.equal(guard.acceptEvent("task-1", "event-1", 1), true);
  assert.equal(guard.acceptEvent("task-1", "event-1", 1), false);
  assert.equal(guard.acceptEvent("task-1", "event-old", 0), false);
  assert.equal(guard.acceptEvent("task-1", "event-2", 1), true);
  assert.equal(guard.acceptEvent("task-1", "event-3", 2), true);
  assert.equal(guard.acceptEvent("task-2", "event-3", 0), true);
  // event-1 was evicted from the bounded duplicate set.
  assert.equal(guard.acceptEvent("task-1", "event-1", 3), true);
});

test("invalid revisions never enter the guard", () => {
  const guard = new RevisionGuard();
  assert.equal(guard.acceptSnapshot("task-1", -1), false);
  assert.equal(guard.acceptSnapshot("task-1", Number.MAX_SAFE_INTEGER + 1), false);
  assert.equal(guard.acceptEvent("task-1", "event", Number.NaN), false);
});

test("atomic snapshot batches reject any stale stream but allow unchanged siblings", () => {
  const guard = new RevisionGuard();
  assert.equal(
    guard.acceptSnapshotBatch([
      { stream: "workbench:batch-1", revision: 2 },
      { stream: "task:task-1", revision: 4 },
    ]),
    true,
  );
  assert.equal(
    guard.acceptSnapshotBatch([
      { stream: "workbench:batch-1", revision: 2 },
      { stream: "task:task-1", revision: 3 },
    ]),
    false,
  );
  assert.equal(
    guard.acceptSnapshotBatch([
      { stream: "workbench:batch-1", revision: 2 },
      { stream: "task:task-1", revision: 5 },
    ]),
    true,
  );
  assert.equal(
    guard.acceptSnapshotBatch([{ stream: "workbench:batch-1", revision: 2 }]),
    false,
  );
});
