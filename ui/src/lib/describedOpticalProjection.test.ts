import assert from "node:assert/strict";
import test from "node:test";

import { projectFrozenItemToScene } from "./applicationServiceProjection.ts";
import { projectCatalogRows } from "./catalogPreview.ts";

// Issue #3 R2 B5: frozen candidates of a schema-described optical product
// without a dedicated projection (HLS) are labelled from the reviewed schema
// and carry no URL or delivery field.

const provider = "microsoft.planetary-computer";

function hlsItem(token: "L30" | "S30", overrides: Record<string, unknown> = {}, metadataOverrides: Record<string, unknown> = {}) {
  const lower = token.toLowerCase();
  const native = `HLS.${token}.T49RDP.2024177T030756.v2.0`;
  const key = `PROVIDER::${provider}::COLLECTION::hls2-${lower}::ITEM::${native}`;
  return {
    item_id: key, provider_id: provider, display_name: native, acquisition_datetime: "2024-06-25T03:07:56.017810Z",
    footprint_geojson: { type: "Polygon", coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]] },
    metadata: {
      logical_dataset: `optical.hls.${lower}.v2`, native_collection_id: `hls2-${lower}`,
      collection_profile_id: `planetary-computer.hls2-${lower}.scene.v1`, identity_rule_id: "planetary-computer.hls2.identity.v1",
      filename_rule_id: "planetary-computer.hls2.search-only.v1", resolver_policy_id: "planetary-computer.hls2.search-only.v1",
      delivery_model: "archive_first", provider_product_id: native, provider_product_key: key,
      source_kind: token === "L30" ? "landsat" : "sentinel2", product_kind: "optical_scene",
      properties: { platform: token === "L30" ? "landsat_9" : "sentinel_2c", cloud_cover: 12.5 }, assets: [],
      ...metadataOverrides,
    },
    payload: {}, asset_access: null, execution_policy: null,
    ...overrides,
  };
}

test("HLS candidates are labelled from their schemas", () => {
  const l30 = projectFrozenItemToScene(hlsItem("L30"));
  assert.ok(l30);
  assert.equal(l30.sar_product?.dataset, "HLS L30");
  assert.equal(l30.platform, "Landsat 9");
  assert.equal(l30.sar_product?.instrument, "OLI");
  assert.equal(l30.sar_product?.processing_level, "L2 协调地表反射率");
  assert.equal(l30.sar_product?.product_family, "HLSL30（30 m）");
  assert.equal(l30.scene_id, "HLS.L30.T49RDP.2024177T030756.v2.0");
  assert.equal(l30.has_url, false);
  assert.equal(l30.sar_product?.download_url, null);
  assert.deepEqual(l30.footprint_bbox, { west: 110, south: 30, east: 111, north: 31, crs: "EPSG:4326" });

  const s30 = projectFrozenItemToScene(hlsItem("S30"));
  assert.equal(s30?.sar_product?.dataset, "HLS S30");
  assert.equal(s30?.platform, "Sentinel-2C");
  assert.equal(s30?.sar_product?.instrument, "MSI");
});

test("malformed HLS candidates are rejected instead of guessed", () => {
  for (const [label, item] of [
    ["asset listed", hlsItem("L30", {}, { assets: [{ asset_key: "B04" }] })],
    ["payload present", hlsItem("L30", { payload: { download_url: "https://example.invalid/x.tif" } })],
    ["cloud out of range", hlsItem("L30", {}, { properties: { platform: "landsat_8", cloud_cover: 140 } })],
    ["key mismatch", hlsItem("L30", {}, { provider_product_key: "PROVIDER::other" })],
    ["not an optical scene", hlsItem("L30", {}, { product_kind: "sar_product" })],
  ] as const) {
    assert.equal(projectFrozenItemToScene(item), null, label);
  }
  // An unknown platform value is shown verbatim rather than relabelled.
  const unknownPlatform = projectFrozenItemToScene(hlsItem("L30", {}, { properties: { platform: "landsat_10", cloud_cover: 1 } }));
  assert.equal(unknownPlatform?.platform, "landsat_10");
});

test("HLS result rows are optical rows with cloud cover and a search-only summary", () => {
  const product = projectFrozenItemToScene(hlsItem("S30"))?.sar_product;
  assert.ok(product);
  const [row] = projectCatalogRows([product], new Set(), null);
  assert.equal(row.kind, "optical");
  assert.equal(row.collection, "HLS S30");
  assert.equal(row.asset_summary, "");
  assert.deepEqual(row.domain_field_cells, [
    { label: "Cloud", value: "12.5%" },
    { label: "Instrument", value: "MSI" },
  ]);
});
