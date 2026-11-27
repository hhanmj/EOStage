import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";

import type { SarProduct } from "./generated/v3-contracts.ts";
import {
  LANDSAT_C2_L2_COLLECTION_ID,
  LANDSAT_C2_L2_LOGICAL_DATASET,
  LANDSAT_C2_L2_PROVIDER_ID,
} from "./landsatCatalog.ts";
import {
  SENTINEL2_L2A_COLLECTION_ID,
  SENTINEL2_L2A_LOGICAL_DATASET,
  SENTINEL2_L2A_PROVIDER_ID,
} from "./sentinel2Catalog.ts";
import {
  opticalHistoryRestoreRoute,
  resolveOpticalHistoryRestoreSelection,
} from "./taskRestore.ts";

test("restore failures remain visible in the aggregate task status", () => {
  const source = readFileSync(
    fileURLToPath(new URL("./bridge.ts", import.meta.url)),
    "utf8",
  );
  assert.match(source, /state: "restore_error"/);
  assert.match(source, /restore_error: \{ code: result\.code \?\? null, error: result\.error \}/);
});

function opticalProduct(
  provider: string,
  logicalDataset: string,
  collection: string,
  sourceKind: string,
): SarProduct {
  return {
    provider,
    native_metadata: {
      logical_dataset: logicalDataset,
      native_collection_id: collection,
      source_kind: sourceKind,
      product_kind: "optical_scene",
    },
  } as SarProduct;
}

test("optical history restore requires one exact provider-bound route", () => {
  const sentinel2 = opticalProduct(
    SENTINEL2_L2A_PROVIDER_ID,
    SENTINEL2_L2A_LOGICAL_DATASET,
    SENTINEL2_L2A_COLLECTION_ID,
    "sentinel2",
  );
  const landsat = opticalProduct(
    LANDSAT_C2_L2_PROVIDER_ID,
    LANDSAT_C2_L2_LOGICAL_DATASET,
    LANDSAT_C2_L2_COLLECTION_ID,
    "landsat",
  );
  assert.equal(opticalHistoryRestoreRoute([sentinel2]), "sentinel2");
  assert.equal(opticalHistoryRestoreRoute([landsat]), "landsat");
  assert.equal(
    opticalHistoryRestoreRoute([
      opticalProduct(SENTINEL2_L2A_PROVIDER_ID, "", SENTINEL2_L2A_COLLECTION_ID, "sentinel2"),
    ]),
    "unsupported_optical",
  );
  assert.equal(
    opticalHistoryRestoreRoute([
      opticalProduct(LANDSAT_C2_L2_PROVIDER_ID, "", LANDSAT_C2_L2_COLLECTION_ID, "landsat"),
    ]),
    "unsupported_optical",
  );
  assert.equal(
    opticalHistoryRestoreRoute([{ ...sentinel2, provider: "unreviewed.provider" }]),
    "unsupported_optical",
  );
  assert.equal(opticalHistoryRestoreRoute([sentinel2, landsat]), "unsupported_optical");
  assert.equal(opticalHistoryRestoreRoute([]), null);
});

test("optical history restore keeps only the selected task members", () => {
  const sourceRows = ["scene-1", "scene-2", "scene-3"].map((id) => ({
    row_id: id,
    item_id: "item-" + id.slice(-1),
    product: opticalProduct(
      SENTINEL2_L2A_PROVIDER_ID,
      SENTINEL2_L2A_LOGICAL_DATASET,
      SENTINEL2_L2A_COLLECTION_ID,
      "sentinel2",
    ),
  }));
  const resolved = resolveOpticalHistoryRestoreSelection(["scene-1", "scene-3"], sourceRows);
  assert.equal(resolved.valid, true);
  assert.equal(resolved.route, "sentinel2");
  assert.deepEqual(resolved.rows.map((row) => row.row_id), ["scene-1", "scene-3"]);
  assert.deepEqual(resolved.item_ids, ["item-1", "item-3"]);
  assert.equal(resolved.products.length, 2);
});

test("optical history restore fails closed for missing rows and incomplete optical metadata", () => {
  const incomplete = opticalProduct(
    SENTINEL2_L2A_PROVIDER_ID,
    "",
    SENTINEL2_L2A_COLLECTION_ID,
    "sentinel2",
  );
  const incompleteResult = resolveOpticalHistoryRestoreSelection(
    ["scene-1"],
    [{ row_id: "scene-1", item_id: "item-1", product: incomplete }],
  );
  assert.equal(incompleteResult.valid, true);
  assert.equal(incompleteResult.route, "unsupported_optical");
  assert.equal(incompleteResult.reason, "unsupported_optical");

  const missingRow = resolveOpticalHistoryRestoreSelection(
    ["scene-1", "scene-missing"],
    [{ row_id: "scene-1", item_id: "item-1", product: incomplete }],
  );
  assert.equal(missingRow.valid, false);
  assert.equal(missingRow.reason, "missing_row");

  const duplicateMapping = resolveOpticalHistoryRestoreSelection(
    ["scene-1", "item-1"],
    [{ row_id: "scene-1", item_id: "item-1", product: incomplete }],
  );
  assert.equal(duplicateMapping.valid, false);
  assert.equal(duplicateMapping.reason, "duplicate_mapping");

  const missingProduct = resolveOpticalHistoryRestoreSelection(
    ["scene-1"],
    [{ row_id: "scene-1", item_id: "item-1", product: null, optical_hint: true }],
  );
  assert.equal(missingProduct.valid, true);
  assert.equal(missingProduct.route, "unsupported_optical");
});
