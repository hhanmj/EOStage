import assert from "node:assert/strict";
import test from "node:test";

import { resolveActualAoi, type ActualAoi } from "./aoiSelection.ts";
import {
  catalogSearchDecision,
  downloadStartDecision,
} from "./operationPrerequisites.ts";

const oldRegionGeometry = {
  type: "Polygon",
  coordinates: [[[100, 20], [101, 20], [101, 21], [100, 20]]],
};
const newWorkingGeometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
};

function actualAoi(identity: string, geometry: unknown): ActualAoi {
  return {
    identity,
    source: "fixture",
    level: null,
    displayName: identity,
    geometry,
    bbox: null,
  };
}

test("SEARCH-01 ASF global search dispatches with null AOI and no output-directory prerequisite", () => {
  const decision = catalogSearchDecision({
    provider: "asf",
    aoiGeojson: null,
    filtersValid: true,
  });

  assert.deepEqual(decision, {
    ok: true,
    request: { provider: "asf", aoi_geojson: null },
  });
  assert.equal("outputDirectory" in decision.request, false);
  assert.equal("output_root" in decision.request, false);
});

test("SEARCH-02 ASF search dispatches the exact working AOI without an output directory", () => {
  const decision = catalogSearchDecision({
    provider: "asf",
    aoiGeojson: newWorkingGeometry,
    filtersValid: true,
  });

  assert.equal(decision.ok, true);
  if (decision.ok) assert.equal(decision.request.aoi_geojson, newWorkingGeometry);
});

test("SEARCH-03 missing output blocks download start but never catalogue search", () => {
  const search = catalogSearchDecision({
    provider: "asf",
    aoiGeojson: null,
    filtersValid: true,
  });
  const download = downloadStartDecision({
    outputDirectory: "",
    selectedProductCount: 2,
  });

  assert.equal(search.ok, true);
  assert.deepEqual(download, {
    ok: false,
    code: "DOWNLOAD_OUTPUT_REQUIRED",
    message: "开始下载前请选择输出目录。",
  });
});

test("SEARCH-04 provider-specific AOI errors never mention an output directory", () => {
  for (const [provider, label] of [
    ["sentinel-2", "Sentinel-2"],
    ["landsat", "Landsat"],
  ] as const) {
    const decision = catalogSearchDecision({
      provider,
      aoiGeojson: null,
      filtersValid: true,
    });
    assert.equal(decision.ok, false);
    if (!decision.ok) {
      assert.equal(decision.code, "CATALOG_SEARCH_AOI_REQUIRED");
      assert.match(decision.message, new RegExp(label));
      assert.doesNotMatch(decision.message, /输出目录|output/i);
    }
  }
});

test("SEARCH-05 a newly bound working AOI is the exact catalogue request geometry", () => {
  const resolved = resolveActualAoi({
    workingAoi: actualAoi("working:new", newWorkingGeometry),
    persistedRegionAoi: actualAoi("region:old", oldRegionGeometry),
  });
  const decision = catalogSearchDecision({
    provider: "asf",
    aoiGeojson: resolved?.geometry ?? null,
    filtersValid: true,
  });

  assert.equal(decision.ok, true);
  if (decision.ok) {
    assert.equal(decision.request.aoi_geojson, newWorkingGeometry);
    assert.notEqual(decision.request.aoi_geojson, oldRegionGeometry);
  }
});

test("invalid catalogue filters use a search-specific typed reason", () => {
  const decision = catalogSearchDecision({
    provider: "asf",
    aoiGeojson: null,
    filtersValid: false,
  });

  assert.deepEqual(decision, {
    ok: false,
    code: "CATALOG_SEARCH_INVALID_INPUT",
    message: "检索参数不完整，请检查数据集和必要筛选条件。",
  });
});
