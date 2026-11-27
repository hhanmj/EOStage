import assert from "node:assert/strict";
import test from "node:test";

import {
  markQueueTaskAccepted,
  markQueueTaskStarting,
  reconcilePreparedTasks,
  type PreparedQueueTask,
} from "./downloadQueueLifecycle.ts";

type Task = PreparedQueueTask & { label: string };

const tasks: Task[] = [
  { id: "task-first", status: "pending", label: "first search" },
  { id: "task-second", status: "pending", label: "second search" },
];

test("each repeated-search task keeps its own stable ID while backend preparation starts", () => {
  const next = markQueueTaskStarting(tasks, "task-second");

  assert.deepEqual(next, [
    tasks[0],
    { ...tasks[1], status: "starting", error: null },
  ]);
  assert.equal(tasks[1].status, "pending");
});

test("an accepted placeholder remains visible until the matching backend task is observed", () => {
  const accepted = markQueueTaskAccepted(tasks, "task-first");

  assert.equal(accepted[0].status, "accepted");
  assert.deepEqual(reconcilePreparedTasks(accepted, new Set(["other-task"])), accepted);
  assert.deepEqual(reconcilePreparedTasks(accepted, new Set(["task-first"])), [tasks[1]]);
});

test("a failed placeholder is not hidden by an unrelated or stale backend status", () => {
  const failed: Task[] = [
    { id: "task-first", status: "failed", label: "first", error: "failed" },
  ];

  assert.deepEqual(reconcilePreparedTasks(failed, new Set(["task-first"])), failed);
});
