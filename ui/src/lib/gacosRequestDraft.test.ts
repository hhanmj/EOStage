import assert from "node:assert/strict";
import test from "node:test";

import {
  initialGacosRequestDraft,
  validateGacosRequestDraft,
  type GacosRequestDraft,
} from "./gacosRequestDraft.ts";

const polygon = {
  type: "Polygon",
  coordinates: [[[110, 20], [111, 20], [111, 21], [110, 20]]],
};

function draft(overrides: Partial<GacosRequestDraft> = {}): GacosRequestDraft {
  return {
    ...structuredClone(initialGacosRequestDraft),
    aoi: { name: "当前 AOI", geometry: structuredClone(polygon) },
    ...overrides,
  };
}

test("manual GACOS dates work without a Sentinel-1 source and deduplicate valid calendar days", () => {
  const result = validateGacosRequestDraft(draft({
    mode: "manual_dates",
    sourceBatch: null,
    datesText: "20260901\n20260901\n20260228",
    hour: "0",
    minute: "5",
  }));

  assert.equal(result.valid, true);
  assert.deepEqual(result.dates, ["20260901", "20260228"]);
  assert.deepEqual(result.errors, []);
});

test("manual GACOS dates reject malformed and impossible calendar values", () => {
  const result = validateGacosRequestDraft(draft({
    mode: "manual_dates",
    datesText: "2026-09-01\n20260230",
    hour: "24",
    minute: "60",
  }));

  assert.equal(result.valid, false);
  assert.match(result.errors.join("\n"), /8 位 YYYYMMDD/);
  assert.match(result.errors.join("\n"), /不是有效的日历日期/);
  assert.match(result.errors.join("\n"), /UTC 小时/);
  assert.match(result.errors.join("\n"), /UTC 分钟/);
});

test("online GACOS validation requires an explicitly frozen source batch", () => {
  const withoutSource = validateGacosRequestDraft(draft({ mode: "online_batch", sourceBatch: null }));
  assert.equal(withoutSource.valid, false);
  assert.match(withoutSource.errors.join("\n"), /导入最新 Sentinel-1/);

  const frozen = validateGacosRequestDraft(draft({
    mode: "online_batch",
    sourceBatch: { batchId: "batch-1", label: "Sentinel-1 最新结果", totalCount: 3 },
  }));
  assert.equal(frozen.valid, true);
});
