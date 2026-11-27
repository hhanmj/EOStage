import assert from "node:assert/strict";
import test from "node:test";
import type { SearchProgress } from "../../lib/generated/v3-contracts.ts";
import {
  RESULT_PAGE_SIZE,
  deliveryCounts,
  deliveryFromPage,
  deliveryFromProgress,
  isStoppedEarly,
  loadingBatchOf,
  pageArrival,
  selectAllMatchingAvailable,
  type BatchDelivery,
} from "./batchDeliveryLogic.ts";
import { createTaskErrorText } from "./resultListLogic.ts";

const loading = (loadedCount: number, remoteTotal: number | null): BatchDelivery => ({
  batchId: "batch-1",
  state: "loading",
  loadedCount,
  remoteTotal,
});

function progress(overrides: Partial<SearchProgress>): SearchProgress {
  return {
    request_id: "request-1",
    resource_id: "science.hls.l30.v2",
    stage: "requesting",
    requests_done: 1,
    items_kept: 0,
    items_cap: 10_000,
    elapsed_ms: 10,
    message_zh: "",
    terminal: false,
    error: null,
    ...overrides,
  };
}

test("while loading the provider total leads and the loaded count rides alongside", () => {
  assert.deepEqual(deliveryCounts(loading(250, 713), 250), {
    headline: 713,
    loaded: { loaded: 250, total: 713 },
    overCeiling: null,
    pageable: 713,
  });
});

test("without a provider total the loaded count is the headline", () => {
  const counts = deliveryCounts(loading(300, null), 300);
  assert.equal(counts.headline, 300);
  assert.deepEqual(counts.loaded, { loaded: 300, total: null });
  assert.equal(counts.pageable, 300);
});

test("a total above the ceiling asks to narrow and paging stops at 10,000", () => {
  const counts = deliveryCounts(loading(500, 25_000), 500);
  assert.equal(counts.headline, 25_000);
  assert.deepEqual(counts.overCeiling, { total: 25_000, limit: 10_000 });
  assert.equal(counts.pageable, 10_000);
});

test("a sealed batch shows its frozen count and no loaded line", () => {
  const sealed: BatchDelivery = { batchId: "batch-1", state: "cancelled", loadedCount: 320, remoteTotal: 713 };
  assert.deepEqual(deliveryCounts(sealed, 320), {
    headline: 320,
    loaded: null,
    overCeiling: null,
    pageable: 320,
  });
  assert.deepEqual(deliveryCounts(null, 88), { headline: 88, loaded: null, overCeiling: null, pageable: 88 });
});

test("cancelled, failed and interrupted batches are possibly incomplete; complete and loading are not", () => {
  for (const state of ["cancelled", "failed", "interrupted"] as const) {
    assert.equal(isStoppedEarly({ ...loading(1, null), state }), true, state);
  }
  assert.equal(isStoppedEarly({ ...loading(1, null), state: "complete" }), false);
  assert.equal(isStoppedEarly(loading(1, null)), false);
  assert.equal(isStoppedEarly(null), false);
});

test("select-all over every match waits for the seal", () => {
  assert.equal(selectAllMatchingAvailable(loading(250, 713)), false);
  assert.equal(selectAllMatchingAvailable({ ...loading(713, 713), state: "complete" }), true);
  assert.equal(selectAllMatchingAvailable(null), true);
});

test("a non-terminal search names its batch only once page 1 is readable", () => {
  const resource = "science.hls.l30.v2";
  assert.equal(loadingBatchOf(progress({}), resource), null);
  assert.deepEqual(loadingBatchOf(progress({ batch_id: "batch-1" }), resource), {
    requestId: "request-1",
    batchId: "batch-1",
  });
  assert.equal(loadingBatchOf(progress({ batch_id: "batch-1" }), "other"), null);
  assert.equal(loadingBatchOf(progress({ batch_id: "batch-1", terminal: true, stage: "done" }), resource), null);
});

test("progress folds into the delivery state of its batch", () => {
  const live = deliveryFromProgress(null, progress({ batch_id: "batch-1", loaded_count: 250, remote_total: 713 }));
  assert.deepEqual(live, loading(250, 713));
  const more = deliveryFromProgress(live, progress({ batch_id: "batch-1", loaded_count: 500 }));
  assert.deepEqual(more, loading(500, 713));
  assert.equal(
    deliveryFromProgress(more, progress({ batch_id: "batch-1", terminal: true, stage: "cancelled", loaded_count: 520 }))?.state,
    "cancelled",
  );
  assert.equal(
    deliveryFromProgress(more, progress({ batch_id: "batch-1", terminal: true, stage: "done", loaded_count: 713 }))?.state,
    "complete",
  );
  assert.equal(deliveryFromProgress(more, progress({})), more, "progress without a batch changes nothing");
});

test("a page is ready, partially here, or pending", () => {
  assert.equal(RESULT_PAGE_SIZE, 250);
  assert.equal(pageArrival(250, 1, 713), "ready");
  assert.equal(pageArrival(250, 2, 713), "pending");
  assert.equal(pageArrival(320, 2, 713), "partial");
  assert.equal(pageArrival(500, 2, 713), "ready");
  assert.equal(pageArrival(288, 2, 288), "ready", "the last short page of a sealed batch is complete");
  assert.equal(pageArrival(50, 1, 713, 50), "ready");
  assert.equal(pageArrival(50, 2, 713, 50), "pending");
  assert.equal(pageArrival(70, 2, 713, 50), "partial");
});

test("a batch page reports its delivery; a page without a state is complete", () => {
  assert.deepEqual(
    deliveryFromPage({ batch_id: "batch-1", total_count: 300, state: "interrupted", loaded_count: 300, remote_total: 713 }),
    { batchId: "batch-1", state: "interrupted", loadedCount: 300, remoteTotal: 713 },
  );
  assert.deepEqual(deliveryFromPage({ batch_id: "batch-1", total_count: 88 }), {
    batchId: "batch-1",
    state: "complete",
    loadedCount: 88,
    remoteTotal: null,
  });
});

test("admission refusals of a loading batch read as waiting, not as a failure", () => {
  assert.equal(
    createTaskErrorText({ code: "V3_STORE_BATCH_ROWS_NOT_YET_LOADED", message_zh: "x" }),
    "所选的行尚未载入，请稍后再试",
  );
  assert.equal(createTaskErrorText({ code: "V3_STORE_BATCH_STILL_LOADING" }), "载入完成后可全选");
});
