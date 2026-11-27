import assert from "node:assert/strict";
import test from "node:test";

import {
  burstAssetItemIds,
  canonicalProductKey,
  mergeProducts,
  normalizeLegacyScene,
  productKey,
  productsForKeys,
  projectSarProductToScene,
  sceneProductKey,
  selectBurstAssetsForDownload,
  supportsProductAction,
} from "./sarProduct.ts";

test("same scene alias in different datasets remains distinct", () => {
  const sentinel = {
    product_key: "ASF::SENTINEL-1::same",
    scene_id: "same",
    dataset: "SENTINEL-1",
  };
  const ers = {
    product_key: "ASF::ERS::same",
    scene_id: "same",
    dataset: "ERS",
  };

  const merged = mergeProducts([sentinel, ers] as never);

  assert.equal(merged.length, 2);
  assert.deepEqual(merged.map(productKey), [sentinel.product_key, ers.product_key]);
});

test("selection resolves by product key rather than scene alias", () => {
  const rows = [
    { product_key: "ASF::SENTINEL-1::same", scene_id: "same" },
    { product_key: "ASF::ERS::same", scene_id: "same" },
  ];

  assert.deepEqual(
    productsForKeys(rows as never, ["ASF::ERS::same"]).map(productKey),
    ["ASF::ERS::same"],
  );
});

test("legacy broad GRD becomes a canonical mission-neutral row", () => {
  const row = normalizeLegacyScene({
    scene_id: "LEGACY_GRD",
    platform: "S1A",
    product_type: "GRD",
    beam_mode: "IW",
    polarization: "DV",
    acquisition_datetime: "2024-01-01T10:00:00Z",
    absolute_orbit: 52000,
    relative_orbit: 88,
    path: 88,
    frame: 456,
    orbit_direction: "ASCENDING",
    has_url: true,
    download_url: "https://datapool.asf.alaska.edu/GRD/LEGACY_GRD.zip",
  });

  assert.equal(row.product_key, "ASF::SENTINEL-1::LEGACY_GRD");
  assert.equal(row.product_family, "GRD");
  assert.equal(row.processing_level, null);
  assert.equal(row.native_metadata.legacy_scene_id, "LEGACY_GRD");
});

test("legacy projection preserves an already-scoped product key and dataset", () => {
  const row = normalizeLegacyScene({
    product_key: "ASF::ERS::ERS_PRODUCT",
    scene_id: "ERS_PRODUCT",
    platform: "ERS-1",
    product_type: "L1",
    beam_mode: "SAR",
    polarization: "VV",
    acquisition_datetime: "1992-01-01T00:00:00Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: 12,
    orbit_direction: "DESCENDING",
    has_url: false,
  });

  assert.equal(row.product_key, "ASF::ERS::ERS_PRODUCT");
});

test("legacy scene UI identity migrates to the same canonical product key as its catalogue row", () => {
  const legacyScene = {
    scene_id: "S1A_RESTORED",
    platform: "S1A",
    product_type: "SLC",
    beam_mode: "IW",
    polarization: "VV",
    acquisition_datetime: "2024-01-01T00:00:00Z",
    absolute_orbit: 1,
    relative_orbit: 2,
    path: 2,
    frame: 3,
    orbit_direction: "ASCENDING",
    has_url: true,
  };
  const normalized = normalizeLegacyScene(legacyScene);

  assert.equal(sceneProductKey(legacyScene), normalized.product_key);
  assert.equal(
    sceneProductKey({ ...legacyScene, sar_product: normalized }),
    normalized.product_key,
  );
});

test("TypeScript product key matches Rust and Python escaping", () => {
  assert.equal(
    canonicalProductKey("asf", "ers", "A::B%1"),
    "ASF::ERS::A%3A%3AB%251",
  );
});

test("canonical product projection keeps the complete mission-neutral record", () => {
  const product = normalizeLegacyScene({
    scene_id: "S1A_TEST",
    platform: "S1A",
    product_type: "SLC",
    beam_mode: "IW",
    polarization: "VV+VH",
    acquisition_datetime: "2024-01-01T00:00:00Z",
    absolute_orbit: 1,
    relative_orbit: 2,
    path: 2,
    frame: 3,
    orbit_direction: "ASCENDING",
    has_url: true,
  });

  const projected = projectSarProductToScene(product);

  assert.equal(projected.product_key, product.product_key);
  assert.equal(projected.scene_id, "S1A_TEST");
  assert.equal(projected.product_type, "SLC");
  assert.strictEqual(projected.sar_product, product);
});

test("orbit action is gated by the complete capability rule", () => {
  const product = normalizeLegacyScene({
    scene_id: "S1A_TEST",
    platform: "S1A",
    product_type: "SLC",
    beam_mode: "IW",
    polarization: "VV",
    acquisition_datetime: "2024-01-01T00:00:00Z",
    absolute_orbit: 1,
    relative_orbit: 2,
    path: 2,
    frame: 3,
    orbit_direction: "ASCENDING",
    has_url: true,
  });
  const capabilities = {
    supported_actions: [{
      action: "orbit_match",
      platforms: ["S1A"],
      product_families: ["SLC"],
      required_fields: ["acquisition_start"],
    }],
  };

  assert.equal(supportsProductAction(product, capabilities as never, "orbit_match"), true);
  assert.equal(
    supportsProductAction({ ...product, product_family: "OCN" }, capabilities as never, "orbit_match"),
    false,
  );
  assert.equal(
    supportsProductAction({ ...product, acquisition_start: null }, capabilities as never, "orbit_match"),
    false,
  );
});

test("Burst asset policy preserves independent TIFF/XML identities and sizes", () => {
  const burst = {
    product_key: "ASF::SLC-BURST::BURST-ONE",
    dataset: "SLC-BURST",
    assets: [
      {
        role: "primary",
        url: "https://sentinel1-burst.asf.alaska.edu/a/1.tiff",
        file_name: "one.tiff",
        media_type: "image/tiff",
        size_bytes: 100,
      },
      {
        role: "metadata",
        url: "https://sentinel1-burst.asf.alaska.edu/a/1.xml",
        file_name: "one.xml",
        media_type: "application/xml",
        size_bytes: null,
      },
    ],
  } as never;
  const frozenBefore = JSON.stringify(burst);

  assert.deepEqual(burstAssetItemIds(burst, "all"), [
    "ASF::SLC-BURST::BURST-ONE::asset::primary",
    "ASF::SLC-BURST::BURST-ONE::asset::metadata",
  ]);
  assert.deepEqual(
    selectBurstAssetsForDownload(burst, "primary").assets?.map((asset) => asset.file_name),
    ["one.tiff"],
  );
  assert.equal(
    (selectBurstAssetsForDownload(burst, "all").assets ?? [])
      .reduce((total, asset) => total + (asset.size_bytes ?? 0), 0),
    100,
  );
  assert.equal(JSON.stringify(burst), frozenBefore, "role filtering must not mutate frozen source assets");
  assert.deepEqual(
    selectBurstAssetsForDownload(burst, "all").assets?.map((asset) => asset.role),
    ["primary", "metadata"],
    "all policy must preserve each frozen role exactly once",
  );
});
