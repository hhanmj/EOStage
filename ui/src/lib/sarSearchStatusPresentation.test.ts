import assert from "node:assert/strict";
import test from "node:test";

import {
  sarSearchStatusPresentation,
  sarSearchTargetCount,
} from "./sarSearchStatusPresentation.ts";

test("projects finite generic and typed result limits without reading provider page size", () => {
  assert.equal(sarSearchTargetCount({ max_results: 20 }), 20);
  assert.equal(sarSearchTargetCount({ result_limit: 500 }), 500);
  assert.equal(sarSearchTargetCount({ page_size: 500 }), null);
});

test("does not manufacture a target from ALL or an invalid limit", () => {
  assert.equal(sarSearchTargetCount({ result_limit: null }), null);
  assert.equal(sarSearchTargetCount({ max_results: 0 }), null);
  assert.equal(sarSearchTargetCount({ max_results: 10_001 }), null);
});

test("left search status is live-only and disappears after completion", () => {
  assert.deepEqual(
    sarSearchStatusPresentation({
      busy: true,
      metadataStatus: {
        state: "running",
        done: 5,
        total: 20,
        percent: 25,
        message: "正在请求 ASF 检索接口",
      },
    }),
    {
      message: "正在请求 ASF 检索接口",
      percent: 25,
      indeterminate: false,
    },
  );

  assert.equal(
    sarSearchStatusPresentation({
      busy: false,
      metadataStatus: null,
    }),
    null,
  );
});

test("backend stage ordinals remain indeterminate and never become a fake 40 percent", () => {
  assert.deepEqual(
    sarSearchStatusPresentation({
      busy: true,
      metadataStatus: {
        state: "running",
        done: 2,
        total: 5,
        percent: 40,
        message: "正在等待 ASF 返回元数据",
        quantitative: false,
      },
    }),
    {
      message: "正在等待 ASF 返回元数据",
      percent: 0,
      indeterminate: true,
    },
  );
});

test("application progress keeps transport-only ALL work indeterminate and shows a real elapsed heartbeat", () => {
  assert.deepEqual(
    sarSearchStatusPresentation({
      busy: true,
      metadataStatus: null,
      applicationProgress: {
        request_id: "typed-1",
        sequence: 3,
        stage: "planning",
        provider_request_count: 2,
        raw_fetched_count: 400,
        mapped_count: 0,
        exact_matched_count: 0,
        admitted_count: 0,
        requested_limit: null,
        remote_total: 900,
        elapsed_ms: 1_650,
        message: "正在拆分饱和时间窗",
        terminal: false,
      },
      heartbeatElapsedMs: 2_100,
    }),
    {
      message: "正在拆分饱和时间窗 · 已等待 2 秒",
      percent: 0,
      indeterminate: true,
    },
  );
});

test("application progress uses finite N only and keeps ALL indeterminate", () => {
  const base = {
    request_id: "search-1",
    sequence: 2,
    stage: "fetching" as const,
    provider_request_count: 1,
    raw_fetched_count: 8,
    mapped_count: 8,
    exact_matched_count: 8,
    admitted_count: 8,
    elapsed_ms: 500,
    message: "正在获取",
    terminal: false,
  };
  assert.deepEqual(sarSearchStatusPresentation({
    busy: true,
    metadataStatus: null,
    applicationProgress: { ...base, requested_limit: 20, remote_total: 100 },
  }), { message: "正在获取", percent: 40, indeterminate: false });
  assert.deepEqual(sarSearchStatusPresentation({
    busy: true,
    metadataStatus: null,
    applicationProgress: { ...base, requested_limit: null, remote_total: 40 },
  }), { message: "正在获取", percent: 0, indeterminate: true });
});

test("cancelling overrides a stale fetching sample immediately", () => {
  assert.deepEqual(sarSearchStatusPresentation({
    busy: true,
    cancelling: true,
    metadataStatus: null,
    sourceLabel: "Bhoonidhi / NRSC",
    applicationProgress: {
      request_id: "cancel-1",
      sequence: 5,
      stage: "fetching",
      provider_request_count: 3,
      raw_fetched_count: 300,
      mapped_count: 200,
      exact_matched_count: 100,
      admitted_count: 100,
      requested_limit: 200,
      remote_total: 500,
      elapsed_ms: 900,
      message: "已载入 100 / 200",
      terminal: false,
    },
  }), { message: "正在停止 Bhoonidhi / NRSC 检索", percent: 0, indeterminate: true });
});

test("source-aware fallback status never invents ASF for typed providers", () => {
  assert.deepEqual(sarSearchStatusPresentation({
    busy: true,
    sourceLabel: "NISAR S-band · Bhoonidhi/NRSC",
    metadataStatus: null,
  }), {
    message: "正在准备 NISAR S-band · Bhoonidhi/NRSC 检索",
    percent: 0,
    indeterminate: true,
  });
});

test("success error and cancel terminal states cannot leave the ASF busy presentation active", () => {
  for (const state of ["completed", "failed", "cancelled"]) {
    assert.equal(
      sarSearchStatusPresentation({
        busy: false,
        metadataStatus: {
          state,
          done: state === "completed" ? 1 : 0,
          total: 1,
          percent: state === "completed" ? 100 : 0,
          message: `terminal:${state}`,
        },
      }),
      null,
      `${state} must not retain a live spinner`,
    );
  }
});
