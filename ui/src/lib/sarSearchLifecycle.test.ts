import assert from "node:assert/strict";
import test from "node:test";

import {
  beginSarSearch,
  completeSarSearch,
  initialSarSearchLifecycle,
  rejectSarSearchCancellation,
  requestSarSearchCancellation,
  settleSarSearch,
  sarSearchAcceptsResult,
  sarSearchCancellationPending,
  sarSearchIsBusy,
} from "./sarSearchLifecycle.ts";

test("cancellation immediately releases foreground busy while retaining backend ownership", () => {
  const idle = initialSarSearchLifecycle();
  const running = beginSarSearch(idle, "m1-search-1");
  const cancelling = requestSarSearchCancellation(running, "m1-search-1");

  assert.equal(running.phase, "running");
  assert.equal(sarSearchAcceptsResult(running, "m1-search-1"), true);
  assert.equal(cancelling.phase, "cancelling");
  assert.equal(cancelling.requestId, "m1-search-1");
  assert.equal(cancelling.generation, running.generation + 1, "cancel invalidates generation-bound late work");
  assert.equal(sarSearchIsBusy(cancelling), false, "other foreground controls must be released immediately");
  assert.equal(sarSearchCancellationPending(cancelling), true);
  assert.equal(sarSearchAcceptsResult(cancelling, "m1-search-1"), false, "cancelled requests lose UI write authority immediately");
  assert.equal(settleSarSearch(cancelling).phase, "cancelling", "backend ownership remains until terminal");

  const cancelled = completeSarSearch(cancelling, "m1-search-1", "cancelled");
  assert.equal(cancelled.phase, "cancelled");
  assert.equal(sarSearchIsBusy(cancelled), false);
  assert.equal(settleSarSearch(cancelled).phase, "idle");
});

test("a terminal cancellation permits an immediate next request and rejects stale completion", () => {
  const first = beginSarSearch(initialSarSearchLifecycle(), "m1-search-1");
  const idle = settleSarSearch(completeSarSearch(
    requestSarSearchCancellation(first, "m1-search-1"),
    "m1-search-1",
    "cancelled",
  ));
  const second = beginSarSearch(idle, "m1-search-2");

  assert.equal(second.phase, "running");
  assert.equal(second.requestId, "m1-search-2");
  assert.equal(sarSearchAcceptsResult(second, "m1-search-2"), true);
  assert.equal(sarSearchAcceptsResult(second, "m1-search-1"), false);
  assert.equal(completeSarSearch(second, "m1-search-1", "completed"), second);
});

test("a rejected cancel returns the same request to running rather than false idle", () => {
  const running = beginSarSearch(initialSarSearchLifecycle(), "m1-search-1");
  const cancelling = requestSarSearchCancellation(running, "m1-search-1");
  const restored = rejectSarSearchCancellation(cancelling, "m1-search-1");

  assert.equal(restored.phase, "running");
  assert.equal(restored.requestId, "m1-search-1");
  assert.equal(sarSearchIsBusy(restored), true);
  assert.equal(sarSearchAcceptsResult(restored, "m1-search-1"), true);
});
