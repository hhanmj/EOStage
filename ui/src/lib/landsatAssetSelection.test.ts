import assert from "node:assert/strict";
import test from "node:test";

import {
  landsatAssetKeysForPreset,
  landsatHasSurfaceTemperature,
  landsatSelectionSizeBytes,
} from "./landsatAssetSelection.ts";
import type { DesktopLandsatAssetDetail } from "./desktopShellBridge.ts";

const key = (asset_key: string, size_bytes = 10): DesktopLandsatAssetDetail => ({
  asset_key,
  file_name: `${asset_key}.dat`,
  role: asset_key.startsWith("mtl") || asset_key === "ang" ? "metadata" : "primary",
  size_bytes,
});

const L2SP = [
  "coastal", "blue", "green", "red", "nir08", "swir16", "swir22",
  "lwir11", "drad", "emis", "emsd", "trad", "urad", "atran", "cdist", "qa",
  "qa_pixel", "qa_radsat", "qa_aerosol", "ang", "mtl.txt", "mtl.xml",
].map((assetKey, index) => key(assetKey, index + 1));

test("Landsat L2SP presets reproduce the official 22/13/14 file group counts", () => {
  assert.equal(landsatAssetKeysForPreset(L2SP, "all").length, 22);
  assert.equal(landsatAssetKeysForPreset(L2SP, "surface_reflectance").length, 13);
  assert.equal(landsatAssetKeysForPreset(L2SP, "surface_temperature").length, 14);
  assert.equal(landsatHasSurfaceTemperature(L2SP), true);
});

test("Landsat L2SR exposes reflectance but not an invented temperature group", () => {
  const l2sr = L2SP.filter((asset) => ![
    "lwir11", "drad", "emis", "emsd", "trad", "urad", "atran", "cdist", "qa",
  ].includes(asset.asset_key));
  assert.equal(l2sr.length, 13);
  assert.equal(landsatAssetKeysForPreset(l2sr, "surface_reflectance").length, 13);
  assert.deepEqual(landsatAssetKeysForPreset(l2sr, "surface_temperature"), []);
  assert.equal(landsatHasSurfaceTemperature(l2sr), false);
});

test("selection size sums only exact frozen asset keys", () => {
  assert.equal(landsatSelectionSizeBytes(L2SP, ["red", "nir08"]), 4 + 5);
  assert.equal(landsatSelectionSizeBytes(L2SP, ["missing"]), 0);
});
