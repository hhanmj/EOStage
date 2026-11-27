import assert from "node:assert/strict";
import test from "node:test";
import {
  ETA_WARMUP_MS,
  LOADING_MIN_VISIBLE_MS,
  REFRESH_TURN_MS,
  computeFullTurnStopMs,
  computeRemainingVisibleHoldMs,
  estimateDownloadTaskEta,
  estimateRemainingEta,
  formatEtaSecondsZh,
} from "./loadingStateMachine.ts";

test("computeRemainingVisibleHoldMs enforces 500ms minimum hold", () => {
  assert.equal(computeRemainingVisibleHoldMs(1000, 1000), LOADING_MIN_VISIBLE_MS);
  assert.equal(computeRemainingVisibleHoldMs(1000, 1300), 200);
  assert.equal(computeRemainingVisibleHoldMs(1000, 1500), 0);
  assert.equal(computeRemainingVisibleHoldMs(1000, 1900), 0);
});

test("computeFullTurnStopMs completes at least one full rotation and lands on integer turn boundary", () => {
  // Instant or short wait completes the first 640ms turn cleanly.
  assert.equal(computeFullTurnStopMs(1000, 1000), REFRESH_TURN_MS);
  assert.equal(computeFullTurnStopMs(1000, 1120), REFRESH_TURN_MS - 120);
  // Already past turn 1: wait remaining time to land cleanly at 1280ms (turn 2).
  assert.equal(computeFullTurnStopMs(1000, 1700), REFRESH_TURN_MS * 2 - 700);
  // Exact boundary stops immediately with 0 remainder.
  assert.equal(computeFullTurnStopMs(1000, 1000 + REFRESH_TURN_MS), 0);
  assert.equal(computeFullTurnStopMs(1000, 1000 + REFRESH_TURN_MS * 2), 0);
});

test("estimateRemainingEta warms up with '估算中…' under 1s and switches to ETA once measurable", () => {
  assert.equal(estimateRemainingEta({ startedAtMs: 1000, nowMs: 1500, completed: 0, total: null }).kind, "hidden");
  assert.equal(estimateRemainingEta({ startedAtMs: 1000, nowMs: 2500, completed: 100, total: 100 }).kind, "hidden");

  const warmup = estimateRemainingEta({
    startedAtMs: 1000,
    nowMs: 1000 + ETA_WARMUP_MS - 100,
    completed: 10,
    total: 100,
    ratePerSecond: 10,
  });
  assert.equal(warmup.kind, "estimating");
  assert.equal(warmup.textZh, "估算中…");

  const zeroRate = estimateRemainingEta({
    startedAtMs: 1000,
    nowMs: 3000,
    completed: 0,
    total: 100,
    ratePerSecond: 0,
  });
  assert.equal(zeroRate.kind, "estimating");
  assert.equal(zeroRate.textZh, "估算中…");

  const active = estimateRemainingEta({
    startedAtMs: 1000,
    nowMs: 2500,
    completed: 50,
    total: 200,
    ratePerSecond: 10,
  });
  assert.equal(active.kind, "eta");
  assert.equal(active.secondsLeft, 15);
  assert.equal(active.textZh, "约 15 秒剩余");

  assert.equal(formatEtaSecondsZh(125), "约 2 分 5 秒剩余");
  assert.equal(formatEtaSecondsZh(3660), "约 1 小时 1 分剩余");
});

test("estimateDownloadTaskEta prefers reported bytes and falls back to multi-item count", () => {
  assert.equal(
    estimateDownloadTaskEta(
      { task_id: "t1", status: "paused", done_bytes: 100, total_bytes: 1000 },
      1000,
      3000
    ).kind,
    "hidden"
  );

  const bytesWarmup = estimateDownloadTaskEta(
    { task_id: "t2", status: "running", done_bytes: 100, total_bytes: 1000, bytes_per_second: 100 },
    1000,
    1400
  );
  assert.equal(bytesWarmup.kind, "estimating");
  assert.equal(bytesWarmup.textZh, "估算中…");

  const bytesEta = estimateDownloadTaskEta(
    { task_id: "t2", status: "running", done_bytes: 200, total_bytes: 1000, bytes_per_second: 100 },
    1000,
    2500
  );
  assert.equal(bytesEta.kind, "eta");
  assert.equal(bytesEta.secondsLeft, 8);
  assert.equal(bytesEta.textZh, "约 8 秒剩余");

  // Archived summary rows with bytes_reported=false fall back cleanly to item count when >1.
  const itemEta = estimateDownloadTaskEta(
    {
      task_id: "t3",
      status: "running",
      done_bytes: 0,
      total_bytes: 500,
      bytes_reported: false,
      items_count: 5,
      completed_items_count: 1,
    },
    1000,
    3000
  );
  assert.equal(itemEta.kind, "eta");
  assert.equal(itemEta.secondsLeft, 8);
});
