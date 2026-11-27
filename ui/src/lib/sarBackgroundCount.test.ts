import assert from "node:assert/strict";
import test from "node:test";
import type { DownloadBatchContext } from "./bridge.ts";
import type { DesktopSarBackgroundCountResult } from "./desktopShellBridge.ts";
import { mergeFrozenSarMatchedCount } from "./sarBackgroundCount.ts";
import { parseSarMaxResultsInput } from "./sarSearchForm.ts";
import { catalogSearchCountSummary } from "./catalogSearchCountSummary.ts";

const scope = { batch_id: "batch-A", expected_workbench_revision: 7 };
function context(limit = 250): DownloadBatchContext {
  return { batch_id: "batch-A", created_at: "2026-09-08T00:00:00Z", aoi_name: "AOI-A",
    aoi_geojson: null, query: { dataset: "SENTINEL-1", max_results: limit },
    revision: 7, page_index: 2, page_size: 24, checked_item_ids: ["item-A"],
    checked_count: 1, total_count: limit, frozen_candidate_count: limit, provider_matched_count: null };
}
// This fixture exercises the merge after bridge validation, not schema admission.
function counted(total = 12_500, batch = "batch-A", revision = 7): DesktopSarBackgroundCountResult {
  return { ok: true, applied: true, total_count: total, snapshot: { core: { workbench: {
    batch_id: batch, revision, search_stats: { matched_count: total },
  } } } } as DesktopSarBackgroundCountResult;
}

test("blank loads at most 250 and a larger background total never grows the working set", () => {
  const limit = parseSarMaxResultsInput("   ");
  assert.equal(limit, 250);
  const before = context(limit!);
  const after = mergeFrozenSarMatchedCount(before, scope, counted())!;
  assert.deepEqual(after, { ...before, provider_matched_count: 12_500 });
  assert.equal(after.query, before.query);
  assert.equal(after.checked_item_ids, before.checked_item_ids);
  assert.equal(after.total_count, 250);
  const labels = catalogSearchCountSummary({ targetCount: limit,
    providerMatchedCount: after.provider_matched_count!, providerReturnedCount: 250,
    frozenCandidateCount: after.frozen_candidate_count!, currentPageCount: 24,
    checkedCount: 1, truncated: false });
  assert.equal(labels.matched, "候选总量 12500");
  assert.equal(labels.frozen, "已载入 250");
  assert.equal(labels.current, "当前显示 24");
});

test("explicit N is retained independently of total, page and selection", () => {
  for (const n of [1, 30, 250, 617, 10_000]) {
    assert.equal(parseSarMaxResultsInput(String(n)), n);
    const before = context(n);
    assert.deepEqual(mergeFrozenSarMatchedCount(before, scope, counted()),
      { ...before, provider_matched_count: 12_500 });
  }
});

test("stale batch and stale revision cannot overwrite the current context", () => {
  for (const before of [null, { ...context(), batch_id: "new-batch" }, { ...context(), revision: 8 }]) {
    assert.equal(mergeFrozenSarMatchedCount(before, scope, counted()), before);
  }
  for (const result of [counted(12_500, "other"), counted(12_500, "batch-A", 8)]) {
    const before = context();
    assert.equal(mergeFrozenSarMatchedCount(before, scope, result), before);
  }
});

test("failed, unsupported and malformed counts preserve unknown rather than manufacture zero", () => {
  const cases: DesktopSarBackgroundCountResult[] = [
    { ok: false, code: "COUNT_UNAVAILABLE", error: "count failed" },
    { ok: true, applied: false, total_count: null, snapshot: null },
    { ok: true, applied: true, total_count: 0, snapshot: null },
    ...[-1, NaN, Infinity, 1.5, Number.MAX_SAFE_INTEGER + 1, 249].map(n => counted(n)),
  ];
  for (const result of cases) {
    const before = context();
    assert.equal(mergeFrozenSarMatchedCount(before, scope, result), before);
    assert.equal(before.provider_matched_count, null);
  }
});

test("a real zero count is displayed for an empty result set", () => {
  const after = mergeFrozenSarMatchedCount(context(0), scope, counted(0));
  assert.equal(after?.provider_matched_count, 0);
});

test("a delayed count uses the latest context rather than restoring an old selection/page", async () => {
  let resolve!: (value: DesktopSarBackgroundCountResult) => void;
  const pending = new Promise<DesktopSarBackgroundCountResult>(done => { resolve = done; });
  let current = context();
  const completion = pending.then(result => { current = mergeFrozenSarMatchedCount(current, scope, result)!; });
  const selected = ["item-B", "item-C"];
  current = { ...current, page_index: 5, checked_item_ids: selected, checked_count: 2 };
  resolve(counted());
  await completion;
  assert.equal(current.page_index, 5);
  assert.equal(current.checked_item_ids, selected);
  assert.equal(current.checked_count, 2);
  assert.equal(current.provider_matched_count, 12_500);
});
