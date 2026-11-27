import assert from "node:assert/strict";
import test from "node:test";

import { interpolatedTaskElapsed } from "./taskMetrics.ts";

test("elapsed never invents wall time beyond the authoritative transfer sample", () => {
  assert.equal(interpolatedTaskElapsed(10, "running", 1_000, 3_500), 10);
});

test("paused and terminal elapsed remain fixed", () => {
  assert.equal(interpolatedTaskElapsed(10, "paused", 1_000, 3_500), 10);
  assert.equal(interpolatedTaskElapsed(10, "finished", 1_000, 3_500), 10);
});

test("clock skew never makes elapsed negative or move backwards", () => {
  assert.equal(interpolatedTaskElapsed(10, "running", 3_500, 1_000), 10);
  assert.equal(interpolatedTaskElapsed(-5, "running", 1_000, 2_000), 0);
});
