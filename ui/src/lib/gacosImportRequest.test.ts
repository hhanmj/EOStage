import assert from "node:assert/strict";
import test from "node:test";

import {
  collectGacosImportRequestChoices,
  reconcileGacosImportRequestKey,
  type GacosImportRequestChoice,
} from "./gacosImportRequest.ts";

const taskId = "gacos-task-1";
const groups = [
  { item_id: "gacos-item-1", dates: ["2026-09-01"], hour: 1, minute: 2, bbox: [110, 20, 111, 21] },
  { item_id: "gacos-item-2", dates: ["2026-09-02"], hour: 3, minute: 4, bbox: [112, 22, 113, 23] },
  { item_id: "gacos-item-3", dates: ["2026-09-03"], hour: 5, minute: 6, bbox: [114, 24, 115, 25] },
];

test("partial GACOS attempts retain every frozen UTC request group", () => {
  const choices = collectGacosImportRequestChoices([], [], {
    task_id: taskId,
    task_status: "running",
    records: [{
      attempt_id: "attempt-1",
      task_id: taskId,
      item_id: "gacos-item-1",
      state: "manual_verification_required",
    }],
    request_groups: groups,
  }, taskId);

  assert.deepEqual(choices.map((choice) => choice.itemId), [
    "gacos-item-1",
    "gacos-item-2",
    "gacos-item-3",
  ]);
  assert.equal(choices[0]?.state, "manual_verification_required");
  assert.equal(choices[1]?.state, "running");
  assert.match(choices[1]?.label ?? "", /2026-09-02 · UTC 03:04 · 范围 112\.0000, 22\.0000, 113\.0000, 23\.0000/);
});

test("unknown GACOS attempt IDs cannot become import choices", () => {
  const choices = collectGacosImportRequestChoices([], [], {
    task_id: taskId,
    task_status: "failed",
    records: [{ item_id: "not-frozen", state: "rejected" }],
    request_groups: groups.slice(0, 1),
  }, taskId);

  assert.deepEqual(choices.map((choice) => choice.itemId), ["gacos-item-1"]);
});

test("stale selection is replaced when request choices shrink to one", () => {
  const choice: GacosImportRequestChoice = {
    key: taskId + "|gacos-item-2",
    taskId,
    itemId: "gacos-item-2",
    state: "running",
    label: "2026-09-02 · UTC 03:04",
  };
  assert.equal(reconcileGacosImportRequestKey("old-task|old-item", [choice]), choice.key);
  assert.equal(reconcileGacosImportRequestKey(choice.key, [choice]), choice.key);
  assert.equal(reconcileGacosImportRequestKey(choice.key, []), "");
});
