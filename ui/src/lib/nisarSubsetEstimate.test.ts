import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_NISAR_FULL_SCENE_BYTES,
  NISAR_METADATA_GRID_FLOOR_BYTES,
  computeNisarPolarizationScale,
  estimateNisarSceneSubsetBytes,
  estimateNisarSubsetBatch,
  formatNisarSubsetEstimateSummary,
} from "./nisarSubsetEstimate.ts";

test("clamps AOI area ratio between 0.03 and 1.0 and preserves 32 MiB grid floor", () => {
  const tinyAoi = estimateNisarSceneSubsetBytes({
    rawSceneBytes: 20_000_000_000,
    aoiIntersectionKm2: 1,
    sceneFootprintKm2: 100_000, // raw ratio 0.00001 -> clamped to 0.03
    selectedPolarizations: ["HH", "HV"],
  });
  assert.equal(tinyAoi.areaRatio, 0.03);
  assert.equal(tinyAoi.polarizationScale, 1.0);
  assert.equal(
    tinyAoi.estimatedBytes,
    Math.round(
      NISAR_METADATA_GRID_FLOOR_BYTES +
        (20_000_000_000 - NISAR_METADATA_GRID_FLOOR_BYTES) * 0.03,
    ),
  );

  const fullAoi = estimateNisarSceneSubsetBytes({
    rawSceneBytes: 20_000_000_000,
    aoiIntersectionKm2: 120_000,
    sceneFootprintKm2: 80_000, // raw ratio 1.5 -> clamped to 1.0
    selectedPolarizations: ["HH", "HV"],
  });
  assert.equal(fullAoi.areaRatio, 1.0);
  assert.equal(fullAoi.estimatedBytes, 20_000_000_000);
});

test("scales by polarization selection factor for single, dual, and quad polarization", () => {
  assert.equal(computeNisarPolarizationScale(["HH"]), 0.5);
  assert.equal(computeNisarPolarizationScale(["HH", "HV"]), 1.0);
  assert.equal(computeNisarPolarizationScale(["HH", "HV", "VV"]), 0.75);
  assert.equal(computeNisarPolarizationScale(["HH"], ["HH", "HV", "VV", "VH"]), 0.25);

  const singlePol = estimateNisarSceneSubsetBytes({
    rawSceneBytes: 16_000_000_000,
    aoiIntersectionKm2: 2_000,
    sceneFootprintKm2: 10_000, // ratio 0.20
    selectedPolarizations: ["HH"],
  });
  const dualPol = estimateNisarSceneSubsetBytes({
    rawSceneBytes: 16_000_000_000,
    aoiIntersectionKm2: 2_000,
    sceneFootprintKm2: 10_000,
    selectedPolarizations: ["HH", "HV"],
  });
  assert.ok(singlePol.estimatedBytes < dualPol.estimatedBytes);
  assert.ok(singlePol.estimatedBytes > NISAR_METADATA_GRID_FLOOR_BYTES);
});

test("estimates proportional batch size from scene footprint bbox and formats Chinese summary", () => {
  const est = estimateNisarSubsetBatch({
    scenes: [
      {
        file_size_remote: 27_000_000_000,
        footprint_bbox: { west: 83.0, south: 27.0, east: 85.5, north: 29.5 },
        polarization: "HH+HV",
      },
    ],
    aoiBbox: { west: 84.0, south: 28.0, east: 84.5, north: 28.5 }, // (0.5/2.5)^2 = 4%
    polarizations: ["HH"], // 1 of 2 pols = 0.5x => 2% data + 32 MiB floor
  });

  assert.equal(est.sceneCount, 1);
  assert.equal(est.rawBytes, 27_000_000_000);
  assert.ok(Math.abs(est.areaRatio - 0.04) < 0.005);
  assert.equal(est.polarizationScale, 0.5);
  assert.equal(est.isFallbackEstimate, true);
  assert.ok(est.estimatedSubsetBytes > 500_000_000 && est.estimatedSubsetBytes < 700_000_000);

  const text = formatNisarSubsetEstimateSummary({
    rawBytes: DEFAULT_NISAR_FULL_SCENE_BYTES,
    estimatedSubsetBytes: 2_400_000_000,
  });
  assert.equal(text, "预计裁剪后约 2.4 GB（完整包 15.0 GB，按研究区面积占比估算）");
});
