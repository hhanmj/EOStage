import assert from "node:assert/strict";
import test from "node:test";

import type {
  DownloadItemStatus,
  StoreTaskPageItem,
} from "../../lib/generated/v3-contracts.ts";
import type { TaskLifecycleStatus } from "./types.ts";
import { sceneItemLifecycleStatus, taskPageToSceneItems } from "./sceneProgress.ts";

function pageItem(
  status: DownloadItemStatus,
  overrides: Partial<StoreTaskPageItem> = {},
): StoreTaskPageItem {
  return {
    ordinal: 0,
    item: {
      item_id: "item-1",
      provider_id: "asf",
      display_name: "S1A_SCENE_001",
      acquisition_datetime: null,
      footprint_geojson: null,
      metadata: {},
      payload: {},
    },
    checked: true,
    status,
    bytes_completed: 128,
    total_bytes: 1024,
    source_total_bytes: 2048,
    bytes_per_second: 0,
    attempt: 0,
    destination: null,
    failure: null,
    finished_at: null,
    retryable: false,
    ...overrides,
  } as StoreTaskPageItem;
}

test("scene item statuses map to the lifecycle states understood by task rows", () => {
  const cases: Array<[string, TaskLifecycleStatus]> = [
    ["queued", "queued"],
    ["active", "running"],
    ["paused", "paused"],
    ["completed", "completed"],
    ["failed", "failed"],
    ["skipped", "completed"],
    ["cancelled", "cancelled"],
    ["interrupted", "interrupted"],
    ["future-status", "queued"],
  ];

  for (const [status, expected] of cases) {
    assert.equal(sceneItemLifecycleStatus(status), expected, status);
  }
});

test("task pages use display names and preserve per-scene progress", () => {
  const items = taskPageToSceneItems({
    items: [
      pageItem("active", {
        bytes_completed: 512,
        total_bytes: 1024,
        source_total_bytes: 2048,
      }),
      pageItem("skipped", {
        item: {
          ...pageItem("skipped").item,
          item_id: "item-fallback",
          display_name: "   ",
        },
        bytes_completed: 2048,
        total_bytes: null,
        source_total_bytes: 2048,
      }),
    ],
  });

  assert.deepEqual(items, [
    {
      scene_id: "S1A_SCENE_001",
      status: "running",
      done_bytes: 512,
      total_bytes: 1024,
    },
    {
      scene_id: "item-fallback",
      status: "completed",
      done_bytes: 2048,
      total_bytes: 2048,
    },
  ]);
});

test("missing item progress falls back to zero and then to the source total", () => {
  assert.deepEqual(
    taskPageToSceneItems({
      items: [
        pageItem("queued", {
          bytes_completed: undefined,
          total_bytes: null,
          source_total_bytes: null,
        }),
      ],
    }),
    [{
      scene_id: "S1A_SCENE_001",
      status: "queued",
      done_bytes: 0,
      total_bytes: null,
    }],
  );
});

test("orbit recheck stays on its own scene and preserves item identity and failure details", () => {
  const firstRecheck: StoreTaskPageItem["orbit_recheck"] = {
    orbit_type: "POEORB",
    state: "waiting",
    attempt: 2,
    next_check_at: "2026-11-22T00:00:00Z",
    deadline_at: "2026-12-25T00:00:00Z",
    expected_file_name: null,
  };
  const secondRecheck: StoreTaskPageItem["orbit_recheck"] = {
    ...firstRecheck,
    attempt: 4,
    next_check_at: "2026-11-23T00:00:00Z",
    expected_file_name: "SECOND.EOF",
  };
  const items = taskPageToSceneItems({
    items: [
      pageItem("queued", { orbit_recheck: firstRecheck }),
      pageItem("active", {
        item: {
          ...pageItem("active").item,
          item_id: "item-2",
          display_name: "S1A_SCENE_002",
        },
        orbit_recheck: secondRecheck,
        failure: { code: "ORB001", reason_zh: "第二次检查失败", retryable: true },
      }),
    ],
  });

  assert.equal(items[0].item_id, "item-1");
  assert.equal(items[0].orbit_recheck, firstRecheck);
  assert.equal(items[1].item_id, "item-2");
  assert.equal(items[1].scene_id, "S1A_SCENE_002");
  assert.equal(items[1].orbit_recheck, secondRecheck);
  assert.equal(items[1].failure?.reason_zh, "第二次检查失败");
  assert.notEqual(items[0].orbit_recheck, items[1].orbit_recheck);
});
