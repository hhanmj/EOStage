import assert from "node:assert/strict";
import test from "node:test";

import {
  orbitBatchContext,
  orbitSnapshotAoiName,
  orbitSnapshotAoiProvenance,
} from "./orbitWorkspace.ts";

const sourceBatch = {
  batch_id: "sar-search-1",
  created_at: "2026-07-20T00:00:00Z",
  aoi_name: "贵州省黔东南州",
  aoi_geojson: { type: "Polygon", coordinates: [] },
  query: { dataset: "SENTINEL-1" },
  page_index: 0,
  page_size: 24,
};

test("online Orbit context freezes the source SAR batch and AOI", () => {
  const frozen = orbitBatchContext(sourceBatch);

  assert.equal(frozen?.batch_id, "orbit-sar-search-1");
  assert.equal(frozen?.aoi_name, "贵州省黔东南州");
  assert.equal(frozen?.query.source_batch_id, "sar-search-1");
  assert.equal(frozen?.query.orbit_type, "POEORB");
  assert.equal(orbitSnapshotAoiProvenance(frozen), "随源 SAR 批次冻结");
});

test("Orbit task AOI prefers existing and frozen context over a later current label", () => {
  assert.equal(
    orbitSnapshotAoiName(null, sourceBatch, "后来切换的 AOI"),
    "贵州省黔东南州",
  );
  assert.equal(
    orbitSnapshotAoiName("旧任务 AOI", null, "后来切换的 AOI"),
    "旧任务 AOI",
  );
  assert.equal(orbitSnapshotAoiName(null, null, "当前 AOI"), "当前 AOI");
});

test("manual Orbit imports keep explicit import-time AOI provenance", () => {
  const manual = {
    ...sourceBatch,
    batch_id: "orbit-import-1",
    query: { source: "orbit_scene_import", orbit_type: "POEORB" },
  };

  assert.equal(orbitSnapshotAoiProvenance(manual), "导入时冻结");
});
