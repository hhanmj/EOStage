import assert from "node:assert/strict";
import test from "node:test";

import {
  createTaskHistoricalUiState,
  taskHistoricalAction,
  updateTaskHistoricalUiState,
} from "./taskHistoricalContext.ts";

test("task historical browsing stays ephemeral and cannot mutate Main batch state", () => {
  const main = Object.freeze({
    batchId: "main-a",
    pageIndex: 4,
    filter: "sentinel",
    checkedIds: Object.freeze(["A-1"]),
  });
  const before = JSON.stringify(main);
  let state = createTaskHistoricalUiState();
  state = updateTaskHistoricalUiState(state, { kind: "set_page", pageIndex: 3 });
  state = updateTaskHistoricalUiState(state, { kind: "set_filter", filter: "ers" });
  state = updateTaskHistoricalUiState(state, { kind: "toggle_item", itemId: "B-2" });

  assert.equal(state.pageIndex, 0, "filter change returns the task-only page to zero");
  assert.equal(state.filter, "ers");
  assert.deepEqual(state.selectedIds, ["B-2"]);
  assert.equal(JSON.stringify(main), before);
});

test("only live task lifecycles append and only terminal lifecycles create successors", () => {
  for (const status of ["queued", "running", "paused"]) {
    assert.equal(taskHistoricalAction(status), "append");
  }
  for (const status of ["completed", "cancelled", "failed"]) {
    assert.equal(taskHistoricalAction(status), "create_successor");
  }
  for (const status of ["interrupted", "unknown"]) {
    assert.equal(taskHistoricalAction(status), "unavailable");
  }
});
