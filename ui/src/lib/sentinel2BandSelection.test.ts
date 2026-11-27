import assert from "node:assert/strict";
import test from "node:test";

import {
  SENTINEL2_REFLECTANCE_BANDS,
  normalizeSentinel2BandKeys,
  sentinel2BandDefinition,
  sentinel2BandKeysForPreset,
  validSentinel2BandKeys,
} from "./sentinel2BandSelection.ts";

test("Sentinel-2 reflectance band contract keeps native resolution groups", () => {
  assert.equal(SENTINEL2_REFLECTANCE_BANDS.length, 12);
  assert.deepEqual(
    SENTINEL2_REFLECTANCE_BANDS.filter((band) => band.resolutionM === 10).map((band) => band.key),
    ["B02", "B03", "B04", "B08"],
  );
  assert.deepEqual(
    SENTINEL2_REFLECTANCE_BANDS.filter((band) => band.resolutionM === 20).map((band) => band.key),
    ["B05", "B06", "B07", "B8A", "B11", "B12"],
  );
  assert.deepEqual(
    SENTINEL2_REFLECTANCE_BANDS.filter((band) => band.resolutionM === 60).map((band) => band.key),
    ["B01", "B09"],
  );
  assert.equal(sentinel2BandDefinition("SCL"), null);
});

test("Sentinel-2 presets are scientific selections rather than provider assets", () => {
  assert.deepEqual(sentinel2BandKeysForPreset("true_color"), ["B02", "B03", "B04"]);
  assert.deepEqual(sentinel2BandKeysForPreset("vegetation"), ["B04", "B08"]);
  assert.deepEqual(sentinel2BandKeysForPreset("all_10m"), ["B02", "B03", "B04", "B08"]);
  assert.equal(sentinel2BandKeysForPreset("all_reflectance").length, 12);
});

test("Sentinel-2 custom band keys are bounded, unique and canonicalized", () => {
  assert.deepEqual(normalizeSentinel2BandKeys(["B11", "SCL", "B02", "B11"]), ["B02", "B11"]);
  assert.equal(validSentinel2BandKeys(["B02", "B11"]), true);
  assert.equal(validSentinel2BandKeys([]), false);
  assert.equal(validSentinel2BandKeys(["B02", "B02"]), false);
  assert.equal(validSentinel2BandKeys(["SCL"]), false);
});
