import assert from "node:assert/strict";
import test from "node:test";

import {
  GOLDEN_MAP_SCENE_LIMIT,
  workbenchMapProjectionRequest,
} from "./workbenchMapProjection.ts";

test("a 300-scene batch keeps the 24-row dock page but projects the Golden 200 map footprints", () => {
  const request = workbenchMapProjectionRequest({
    batch_id: "batch-300",
    revision: 7,
    candidate_count: 300,
    dock_page_size: 24,
  });

  assert.equal(GOLDEN_MAP_SCENE_LIMIT, 200);
  assert.deepEqual(request, {
    batch_id: "batch-300",
    expected_revision: 7,
    page_index: 0,
    page_size: 200,
  });
});

test("small batches request only their admitted footprint count", () => {
  assert.deepEqual(workbenchMapProjectionRequest({
    batch_id: "batch-12",
    revision: 1,
    candidate_count: 12,
    dock_page_size: 24,
  }), {
    batch_id: "batch-12",
    expected_revision: 1,
    page_index: 0,
    page_size: 12,
  });
});

test("an invalid dock projection cannot authorize a map page", () => {
  assert.equal(workbenchMapProjectionRequest({
    batch_id: "batch-invalid",
    revision: 1,
    candidate_count: 300,
    dock_page_size: 0,
  }), null);
});
