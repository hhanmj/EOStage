import assert from "node:assert/strict";
import test from "node:test";

import type { SceneRow, SarProductRow } from "./bridge.ts";
import { compactSearchWorkbenchRows } from "./searchWorkbenchPersistence.ts";

const product = {
  product_key: "ASF::ERS::ERS-1",
  product_id: "ERS-1",
  scene_id: "ERS-1",
  provider: "ASF",
  dataset: "ERS",
  platform: "ERS-1",
  instrument: "SAR",
  product_family: "SLC",
  processing_level: "L1",
  beam_mode: null,
  beam_swath: null,
  polarization: null,
  flight_direction: null,
  look_direction: null,
  acquisition_start: null,
  acquisition_stop: null,
  absolute_orbit: null,
  relative_orbit: null,
  path: null,
  frame: null,
  asf_frame: null,
  file_name: "ERS-1.DAT",
  download_url: "https://datapool.asf.alaska.edu/ERS/ERS-1.DAT",
  file_size_remote: null,
  checksum: null,
  footprint_geojson: null,
  footprint_bbox: null,
  native_metadata: { source: "ASF" },
  local_path: null,
  burst: null,
  assets: [],
} satisfies SarProductRow;

test("online search persistence sends one canonical product instead of a duplicate scene envelope", () => {
  const scene = {
    scene_id: product.scene_id,
    sar_product: product,
    platform: "ERS-1",
  } as SceneRow;

  const [persisted] = compactSearchWorkbenchRows([scene]);

  assert.strictEqual(persisted, product);
  assert.equal("sar_product" in persisted, false);
});

test("legacy imported rows remain compatible with search workbench persistence", () => {
  const legacy = { scene_id: "S1A_LEGACY", platform: "S1A" } as SceneRow;

  assert.strictEqual(compactSearchWorkbenchRows([legacy])[0], legacy);
});
