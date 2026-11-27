import assert from "node:assert/strict";
import test from "node:test";

import { MAX_UI_TASK_PAGE_SIZE, parseTaskPage } from "./coreRestoreProjection.ts";
import { projectTaskPagePresentation } from "./taskPagePresentation.ts";

function taskPagePayload(count = 1) {
  return {
    task_id: "task-page",
    revision: 3,
    page_index: 0,
    page_size: 100,
    total_count: 10_000,
    items: Array.from({ length: count }, (_, index) => ({
      ordinal: index,
      checked: index === 0,
      item: {
        item_id: `item-${index}`,
        provider_id: "ASF",
        display_name: `scene-${index}`,
        acquisition_datetime: null,
        footprint_geojson: null,
        metadata: {},
        payload: {},
        asset_access: null,
        execution_policy: null,
      },
      status: index === 0 ? "active" : "queued",
      bytes_completed: index === 0 ? 42 : 0,
      total_bytes: index === 0 ? 100 : null,
      bytes_per_second: index === 0 ? 7.5 : 0,
      attempt: index === 0 ? 2 : 0,
      destination: null,
      failure: null,
      finished_at: null,
      retryable: false,
    })),
  };
}

function taskPagePayloadWithByteFacts(
  networkBytesReceived: unknown,
  sourceTotalBytes: unknown,
  itemSourceTotalBytes: unknown,
) {
  const payload = taskPagePayload(1) as ReturnType<typeof taskPagePayload> & Record<string, unknown>;
  payload.network_bytes_received = networkBytesReceived;
  payload.source_total_bytes = sourceTotalBytes;
  Object.assign(payload.items[0]!, { source_total_bytes: itemSourceTotalBytes });
  return payload;
}

test("bounded task page parses item progress without accepting a full candidate payload", () => {
  const parsed = parseTaskPage(taskPagePayload(2));
  assert.ok(parsed);
  assert.equal(parsed.items.length, 2);
  assert.equal(parsed.items[0]?.status, "active");
  assert.equal(parsed.items[0]?.bytes_completed, 42);
  assert.equal(parsed.items[1]?.status, "queued");

  const oversized = taskPagePayload(MAX_UI_TASK_PAGE_SIZE + 1);
  assert.equal(parseTaskPage(oversized), null);

  const fullSnapshot = taskPagePayload(1) as Record<string, unknown>;
  fullSnapshot.candidates = [{ item_id: "must-not-cross" }];
  assert.equal(parseTaskPage(fullSnapshot), null);
});

test("orbit recheck projection preserves backend state and rejects malformed scientific scheduling fields", () => {
  const payload = taskPagePayload(1) as ReturnType<typeof taskPagePayload> & { items: Array<Record<string, unknown>> };
  const recheck = {
    orbit_type: "POEORB",
    state: "waiting",
    attempt: 2,
    next_check_at: "2026-11-22T00:00:00Z",
    deadline_at: "2026-12-25T00:00:00Z",
    expected_file_name: null,
  };
  Object.assign(payload.items[0], { orbit_recheck: recheck });
  const parsed = parseTaskPage(payload);
  assert.ok(parsed);
  assert.deepEqual(parsed.items[0].orbit_recheck, recheck);

  Object.assign(payload.items[0], { orbit_recheck: { ...recheck, attempt: -1 } });
  assert.equal(parseTaskPage(payload), null);
  Object.assign(payload.items[0], { orbit_recheck: { ...recheck, orbit_type: "RESORB" } });
  assert.equal(parseTaskPage(payload), null);
  Object.assign(payload.items[0], { orbit_recheck: { ...recheck, invented_field: true } });
  assert.equal(parseTaskPage(payload), null);
});

test("task page byte facts preserve source and network truth independently from output totals", () => {
  const parsed = parseTaskPage(taskPagePayloadWithByteFacts(321, 654, 987));
  assert.ok(parsed);
  assert.equal(parsed.network_bytes_received, 321);
  assert.equal(parsed.source_total_bytes, 654);
  assert.equal(parsed.items[0]?.source_total_bytes, 987);

  const presentation = projectTaskPagePresentation(parsed);
  assert.equal(presentation.networkBytesReceived, 321);
  assert.equal(presentation.sourceTotalBytes, 654);
  assert.equal(presentation.items[0]?.sourceTotalBytes, 987);
  assert.equal(presentation.items[0]?.totalBytes, 100);
  assert.equal(presentation.totalCount, 10_000);
  assert.equal(presentation.pageIndex, 0);
  assert.equal(presentation.pageSize, 100);
});

test("task page byte facts normalize omitted and null values to null while preserving zero", () => {
  const omitted = parseTaskPage(taskPagePayload());
  assert.ok(omitted);
  assert.equal(omitted.network_bytes_received, null);
  assert.equal(omitted.source_total_bytes, null);
  assert.equal(omitted.items[0]?.source_total_bytes, null);

  const nullable = parseTaskPage(taskPagePayloadWithByteFacts(null, null, null));
  assert.ok(nullable);
  assert.equal(nullable.network_bytes_received, null);
  assert.equal(nullable.source_total_bytes, null);
  assert.equal(nullable.items[0]?.source_total_bytes, null);

  const zero = parseTaskPage(taskPagePayloadWithByteFacts(0, 0, 0));
  assert.ok(zero);
  assert.equal(zero.network_bytes_received, 0);
  assert.equal(zero.source_total_bytes, 0);
  assert.equal(zero.items[0]?.source_total_bytes, 0);
});

test("task page byte facts reject invalid integers and retain closed schemas", () => {
  const invalidValues = [
    -1,
    1.5,
    "1",
    true,
    Number.NaN,
    Number.POSITIVE_INFINITY,
    Number.MAX_SAFE_INTEGER + 1,
  ];
  for (const invalid of invalidValues) {
    assert.equal(
      parseTaskPage(taskPagePayloadWithByteFacts(invalid, null, null)),
      null,
      `network_bytes_received accepted ${String(invalid)}`,
    );
    assert.equal(
      parseTaskPage(taskPagePayloadWithByteFacts(null, invalid, null)),
      null,
      `source_total_bytes accepted ${String(invalid)}`,
    );
    assert.equal(
      parseTaskPage(taskPagePayloadWithByteFacts(null, null, invalid)),
      null,
      `item source_total_bytes accepted ${String(invalid)}`,
    );
  }

  const unknownTaskField = taskPagePayloadWithByteFacts(null, null, null);
  unknownTaskField.unexpected_byte_fact = 1;
  assert.equal(parseTaskPage(unknownTaskField), null);

  const unknownItemField = taskPagePayloadWithByteFacts(null, null, null);
  Object.assign(unknownItemField.items[0]!, { unexpected_byte_fact: 1 });
  assert.equal(parseTaskPage(unknownItemField), null);
});

test("task page items carry their local reuse decision strictly (downloader D3)", () => {
  const withDecision = (localReuse: unknown) => {
    const payload = taskPagePayload(1);
    Object.assign(payload.items[0]!, { local_reuse: localReuse });
    return payload;
  };
  const reused = parseTaskPage(withDecision({
    disposition: "reused",
    grade: "size_and_format",
    conflict_reason: null,
    conflict_file_name: null,
  }));
  assert.ok(reused);
  assert.deepEqual(reused.items[0]?.local_reuse, {
    disposition: "reused",
    grade: "size_and_format",
    conflict_reason: null,
    conflict_file_name: null,
  });
  assert.deepEqual(projectTaskPagePresentation(reused).items[0]?.localReuse, reused.items[0]?.local_reuse);

  const renamed = parseTaskPage(withDecision({
    disposition: "renamed_conflict",
    conflict_reason: "size_mismatch",
    conflict_file_name: "scene.zip.eostage-conflict-20261020T010203Z",
  }));
  assert.ok(renamed);
  assert.deepEqual(renamed.items[0]?.local_reuse, {
    disposition: "renamed_conflict",
    grade: null,
    conflict_reason: "size_mismatch",
    conflict_file_name: "scene.zip.eostage-conflict-20261020T010203Z",
  });

  const omitted = parseTaskPage(taskPagePayload(1));
  assert.equal(omitted?.items[0]?.local_reuse, null);
  assert.equal(parseTaskPage(withDecision(null))?.items[0]?.local_reuse, null);

  for (const invalid of [
    { disposition: "guessed" },
    { disposition: "reused", grade: "probably" },
    { disposition: "reused", unexpected: 1 },
    { grade: "size_and_format" },
    { disposition: "renamed_conflict", conflict_file_name: "" },
    "reused",
  ]) {
    assert.equal(parseTaskPage(withDecision(invalid)), null, `accepted ${JSON.stringify(invalid)}`);
  }
});
