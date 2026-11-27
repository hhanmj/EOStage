import assert from "node:assert/strict";
import test from "node:test";
import { parseCoreTaskSummary } from "./coreRestoreProjection.ts";

function task() {
  return {
    task_id: "network-task", batch_id: "network-batch", provider_id: "asf.sentinel1",
    created_at: "2026-09-08T00:00:00Z", status: "running", revision: 3,
    updated_at: "2026-09-08T00:00:03Z", selected_count: 1, checked_count: 0,
    active_count: 1, paused_count: 0, queued_count: 0, completed_count: 0,
    failed_count: 0, skipped_count: 0, cancelled_count: 0, terminal_bytes: 0,
    active_bytes: 10, other_bytes: 0, transferred_bytes: 10, total_bytes: 100,
    bytes_per_second: 25, transfer_active_millis: 3210,
    active_item: { item_id: "ITEM-A", bytes_completed: 10, total_bytes: 100 },
    output_root: "fixture-output", layout: "direct", operation: null, actionable: true,
  };
}

test("old and unmeasured task summaries retain unknown network bytes", () => {
  for (const source of [task(), { ...task(), network_bytes_received: null }]) {
    const parsed = parseCoreTaskSummary(source);
    assert.ok(parsed);
    assert.equal(parsed.network_bytes_received, null);
    assert.equal(parsed.transferred_bytes, 10);
    assert.equal(parsed.transfer_active_millis, 3210);
  }
});

test("measured zero and source traffic remain separate from output progress", () => {
  for (const bytes of [0, 1, 10000, Number.MAX_SAFE_INTEGER]) {
    const source = { ...task(), network_bytes_received: bytes };
    const before = structuredClone(source);
    const parsed = parseCoreTaskSummary(source);
    assert.ok(parsed);
    assert.equal(parsed.network_bytes_received, bytes);
    assert.equal(parsed.transferred_bytes, 10);
    assert.equal(parsed.total_bytes, 100);
    assert.equal(parsed.bytes_per_second, 25);
    assert.deepEqual(source, before);
  }
});

test("invalid network counters cannot enter the strict task read model", () => {
  for (const value of [-1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, "10", true, {}]) {
    assert.equal(parseCoreTaskSummary({ ...task(), network_bytes_received: value }), null);
  }
});

test("adding the network field does not weaken unrelated task validation", () => {
  const source = { ...task(), network_bytes_received: 50 };
  assert.equal(parseCoreTaskSummary({ ...source, unexpected_field: 1 }), null);
  assert.equal(parseCoreTaskSummary({ ...source, transferred_bytes: 99 }), null);
});
