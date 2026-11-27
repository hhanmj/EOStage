import assert from "node:assert/strict";
import test from "node:test";

import type { SarProduct } from "./generated/v3-contracts.ts";
import {
  MPC_SENTINEL2_L2A_COLLECTION_ID,
  MPC_SENTINEL2_L2A_FILENAME_RULE_ID,
  MPC_SENTINEL2_L2A_IDENTITY_RULE_ID,
  MPC_SENTINEL2_L2A_PROFILE_ID,
  MPC_SENTINEL2_L2A_PROVIDER_ID,
  MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID,
  MPC_SENTINEL2_L2A_TILE_ORIGIN,
  SENTINEL2_L2A_COLLECTION_ID,
  SENTINEL2_L2A_LOGICAL_DATASET,
  SENTINEL2_L2A_PROVIDER_ID,
  buildSentinel2L2aCatalogQuery,
  isMicrosoftSentinel2SearchOnlyProduct,
  isSentinel2L2aCatalogProduct,
  sentinel2DownloadDisabledReason,
  sentinel2MapTilePreview,
  sentinel2ResolvedMapTilePreview,
  sentinel2Tile,
} from "./sentinel2Catalog.ts";

const aoi = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
};

test("Sentinel-2 L2A search builds one exact UTC OData query", () => {
  const result = buildSentinel2L2aCatalogQuery({
    aoiGeojson: aoi,
    startUtc: "2026-07-01",
    endUtc: "2026-07-02",
    cloudCoverLte: "12.5",
    resultLimit: 24,
  });
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.query.logical_dataset, SENTINEL2_L2A_LOGICAL_DATASET);
  assert.equal(result.query.provider_id, SENTINEL2_L2A_PROVIDER_ID);
  assert.equal(result.query.native_collection_id, SENTINEL2_L2A_COLLECTION_ID);
  assert.equal(result.query.cursor, null);
  assert.deepEqual(result.query.aoi_geojson, aoi);
  assert.deepEqual(result.query.filters, {
    product_type: "S2MSI2A",
    datetime: {
      start: "2026-07-01T00:00:00Z",
      end: "2026-07-02T00:00:00Z",
    },
    cloud_cover_lte: 12.5,
  });
});

test("Sentinel-2 L2A search rejects missing geometry, local time, reversed time, and unsafe cloud values", () => {
  const base = {
    aoiGeojson: aoi,
    startUtc: "2026-07-01T00:00:00Z",
    endUtc: "2026-07-02T00:00:00Z",
    cloudCoverLte: "",
    resultLimit: 24,
  };
  assert.equal(buildSentinel2L2aCatalogQuery({ ...base, aoiGeojson: null }).ok, false);
  assert.equal(buildSentinel2L2aCatalogQuery({ ...base, startUtc: "2026-07-01T00:00" }).ok, false);
  assert.equal(buildSentinel2L2aCatalogQuery({ ...base, startUtc: "2026-02-30T00:00:00Z" }).ok, false);
  assert.equal(buildSentinel2L2aCatalogQuery({ ...base, endUtc: base.startUtc }).ok, false);
  assert.equal(buildSentinel2L2aCatalogQuery({ ...base, cloudCoverLte: "100.01" }).ok, false);
});

test("Sentinel-2 spacecraft selections are canonical, optional, and independent of product type", () => {
  const base = { aoiGeojson: aoi, startUtc: "", endUtc: "", cloudCoverLte: "", resultLimit: 100 };
  const selected = buildSentinel2L2aCatalogQuery({ ...base, spacecraft: ["sentinel_2c", "sentinel_2a"] });
  assert.equal(selected.ok, true);
  if (selected.ok) {
    assert.deepEqual(selected.query.filters, { product_type: "S2MSI2A", spacecraft: ["sentinel_2a", "sentinel_2c"] });
  }
  const all = buildSentinel2L2aCatalogQuery({ ...base, spacecraft: [] });
  assert.equal(all.ok, true);
  if (all.ok) assert.deepEqual(all.query.filters, { product_type: "S2MSI2A" });
  assert.equal(buildSentinel2L2aCatalogQuery({ ...base, spacecraft: ["landsat_9"] }).ok, false);
});

test("Sentinel-2 blank dates mean all time while one-sided dates fail closed", () => {
  const allTime = buildSentinel2L2aCatalogQuery({
    aoiGeojson: aoi,
    startUtc: "",
    endUtc: "",
    cloudCoverLte: "",
    resultLimit: 100,
  });
  assert.equal(allTime.ok, true);
  if (allTime.ok) {
    assert.equal("datetime" in allTime.query.filters, false);
    assert.equal(allTime.query.result_limit, 100);
  }
  assert.equal(buildSentinel2L2aCatalogQuery({
    aoiGeojson: aoi,
    startUtc: "2026-07-01",
    endUtc: "",
    cloudCoverLte: "",
    resultLimit: 100,
  }).ok, false);
});

test("Sentinel-2 L2A search compares RFC3339 fractions without millisecond truncation", () => {
  const base = {
    aoiGeojson: aoi,
    startUtc: "2026-07-01T00:00:00.000000001Z",
    endUtc: "2026-07-01T00:00:00.000000002Z",
    cloudCoverLte: "",
    resultLimit: 24,
  };
  const accepted = buildSentinel2L2aCatalogQuery(base);
  assert.equal(accepted.ok, true);
  if (accepted.ok) {
    assert.deepEqual(accepted.query.filters.datetime, {
      start: base.startUtc,
      end: base.endUtc,
    });
  }
  assert.equal(
    buildSentinel2L2aCatalogQuery({
      ...base,
      startUtc: base.endUtc,
      endUtc: base.startUtc,
    }).ok,
    false,
  );
});

test("Sentinel-2 C filenames retain the stable MGRS tile", () => {
  const product = {
    file_name: "S2C_MSIL2A_20260909T041551_N0512_R090_T46SGB_20260909T075522.SAFE.zip",
    scene_id: "S2C_MSIL2A_20260909T041551_N0512_R090_T46SGB_20260909T075522.SAFE",
  } as SarProduct;
  assert.equal(sentinel2Tile(product), "T46SGB");
});

test("Sentinel-2 classification admits only reviewed CDSE or Microsoft source identities", () => {
  const cdse = {
    provider: SENTINEL2_L2A_PROVIDER_ID,
    native_metadata: {
      logical_dataset: SENTINEL2_L2A_LOGICAL_DATASET,
      native_collection_id: SENTINEL2_L2A_COLLECTION_ID,
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  } as SarProduct;
  const microsoft = {
    provider: MPC_SENTINEL2_L2A_PROVIDER_ID,
    native_metadata: {
      logical_dataset: SENTINEL2_L2A_LOGICAL_DATASET,
      native_collection_id: MPC_SENTINEL2_L2A_COLLECTION_ID,
      collection_profile_id: MPC_SENTINEL2_L2A_PROFILE_ID,
      identity_rule_id: MPC_SENTINEL2_L2A_IDENTITY_RULE_ID,
      filename_rule_id: MPC_SENTINEL2_L2A_FILENAME_RULE_ID,
      resolver_policy_id: MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID,
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  } as SarProduct;
  assert.equal(isSentinel2L2aCatalogProduct(cdse), true);
  assert.equal(isSentinel2L2aCatalogProduct(microsoft), true);
  assert.equal(isMicrosoftSentinel2SearchOnlyProduct(microsoft), true);
  assert.equal(sentinel2DownloadDisabledReason(microsoft), null);
  assert.equal(sentinel2DownloadDisabledReason(cdse), null);
  assert.equal(
    isSentinel2L2aCatalogProduct({ ...cdse, provider: "unreviewed.provider" }),
    false,
  );
  assert.equal(
    isMicrosoftSentinel2SearchOnlyProduct({
      ...microsoft,
      native_metadata: {
        ...microsoft.native_metadata,
        native_collection_id: SENTINEL2_L2A_COLLECTION_ID,
      },
    }),
    false,
  );
});

test("Microsoft Sentinel-2 true-color tiles are generated only from the exact frozen native identity", () => {
  const itemId = "S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545";
  const key = `PROVIDER::${MPC_SENTINEL2_L2A_PROVIDER_ID}::COLLECTION::${MPC_SENTINEL2_L2A_COLLECTION_ID}::ITEM::${itemId}`;
  const product = {
    product_key: key,
    provider_product_key: key,
    provider_product_id: itemId,
    product_id: itemId,
    scene_id: "S2B_MSIL2A_20250129T022849_N0511_R046_T51RUH_20250129T040545.SAFE",
    provider: MPC_SENTINEL2_L2A_PROVIDER_ID,
    native_metadata: {
      logical_dataset: SENTINEL2_L2A_LOGICAL_DATASET,
      native_collection_id: MPC_SENTINEL2_L2A_COLLECTION_ID,
      collection_profile_id: MPC_SENTINEL2_L2A_PROFILE_ID,
      identity_rule_id: MPC_SENTINEL2_L2A_IDENTITY_RULE_ID,
      filename_rule_id: MPC_SENTINEL2_L2A_FILENAME_RULE_ID,
      resolver_policy_id: MPC_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID,
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  } as SarProduct;

  const preview = sentinel2MapTilePreview(product);
  assert.ok(preview);
  assert.equal(preview.productKey, key);
  assert.equal(preview.origin, MPC_SENTINEL2_L2A_TILE_ORIGIN);
  assert.equal(preview.statusLabel, "Sentinel-2 真彩瓦片");
  assert.match(preview.urlTemplate, /^https:\/\/planetarycomputer\.microsoft\.com\/api\/data\/v1\/item\/tiles\/WebMercatorQuad\/\{z\}\/\{x\}\/\{y\}@1x\?/);
  assert.match(preview.urlTemplate, /collection=sentinel-2-l2a/);
  assert.match(preview.urlTemplate, /item=S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545/);
  assert.match(preview.urlTemplate, /assets=visual/);
  assert.match(preview.urlTemplate, /asset_bidx=visual%7C1%2C2%2C3/);
  assert.match(preview.urlTemplate, /nodata=0&format=png$/);

  assert.equal(sentinel2MapTilePreview({ ...product, product_id: itemId.replace("R046", "R047") }), null);
  assert.equal(sentinel2MapTilePreview({ ...product, provider_product_id: itemId.replace("R046", "R047") }), null);
  assert.equal(sentinel2MapTilePreview({ ...product, product_key: key + "-forged" }), null);
  assert.equal(sentinel2MapTilePreview({
    ...product,
    scene_id: "S2B_MSIL2A_20250129T022849_N0511_R047_T51RUH_20250129T040545.SAFE",
  }), null);
});

test("CDSE Sentinel-2 keeps its delivery identity while an exact MPC Item supplies only map tiles", () => {
  const productKey = "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084";
  const product = {
    product_key: productKey,
    provider_product_key: productKey,
    provider_product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
    product_id: "165ba4c9-997c-4cea-970f-33ab9bdb8084",
    scene_id: "S2C_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE",
    provider: SENTINEL2_L2A_PROVIDER_ID,
    native_metadata: {
      logical_dataset: SENTINEL2_L2A_LOGICAL_DATASET,
      native_collection_id: SENTINEL2_L2A_COLLECTION_ID,
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  } as SarProduct;
  const mapItemId = "S2C_MSIL2A_20260726T125321_R138_T27VXL_20260726T221014";

  const preview = sentinel2ResolvedMapTilePreview(product, mapItemId);
  assert.ok(preview);
  assert.equal(preview.productKey, productKey, "map source must not replace the CDSE product identity");
  assert.match(preview.urlTemplate, /collection=sentinel-2-l2a/);
  assert.match(preview.urlTemplate, /item=S2C_MSIL2A_20260726T125321_R138_T27VXL_20260726T221014/);
  assert.equal(
    sentinel2ResolvedMapTilePreview(product, mapItemId.replace("R138", "R139")),
    null,
    "a merely similar MPC Item is not an equivalent Preview source",
  );
});

test("Element 84 Earth Search Sentinel-2 L2A items validate identity and public S3 COG thumbnails", async () => {
  const {
    EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
    EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID,
    EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
    EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID,
    EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
    EARTH_SEARCH_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID,
    earthSearchSentinel2ThumbnailAsset,
    isEarthSearchSentinel2Product,
    isValidEarthSearchSentinel2ThumbnailUrl,
  } = await import("./sentinel2Catalog.ts");

  const itemId = "S2B_50RKU_20250915_0_L2A";
  const productKey = `PROVIDER::element84.earth-search::COLLECTION::sentinel-2-l2a::ITEM::${itemId}`;
  const thumbUrl =
    "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/50/R/KU/2025/9/S2B_50RKU_20250915_0_L2A/thumbnail.jpg";
  const product = {
    product_key: productKey,
    provider_product_key: productKey,
    provider_product_id: itemId,
    product_id: itemId,
    scene_id: "S2B_MSIL2A_20250915T024539_N0511_R132_T50RKU_20250915T061234.SAFE",
    provider: EARTH_SEARCH_SENTINEL2_L2A_PROVIDER_ID,
    assets: [
      {
        asset_key: "thumbnail",
        role: "thumbnail",
        url: thumbUrl,
        file_name: "thumbnail.jpg",
        media_type: "image/jpeg",
        size_bytes: null,
      },
    ],
    native_metadata: {
      logical_dataset: SENTINEL2_L2A_LOGICAL_DATASET,
      native_collection_id: EARTH_SEARCH_SENTINEL2_L2A_COLLECTION_ID,
      collection_profile_id: EARTH_SEARCH_SENTINEL2_L2A_PROFILE_ID,
      identity_rule_id: EARTH_SEARCH_SENTINEL2_L2A_IDENTITY_RULE_ID,
      filename_rule_id: EARTH_SEARCH_SENTINEL2_L2A_FILENAME_RULE_ID,
      resolver_policy_id: EARTH_SEARCH_SENTINEL2_L2A_SEARCH_ONLY_POLICY_ID,
      source_kind: "sentinel2",
      product_kind: "optical_scene",
    },
  } as unknown as SarProduct;

  assert.equal(isEarthSearchSentinel2Product(product), true);
  assert.equal(isSentinel2L2aCatalogProduct(product), true);
  assert.equal(isValidEarthSearchSentinel2ThumbnailUrl(itemId, thumbUrl), true);
  assert.equal(
    isValidEarthSearchSentinel2ThumbnailUrl(itemId, "https://evil.example.com/sentinel-s2-l2a-cogs/thumbnail.jpg"),
    false,
  );
  assert.equal(earthSearchSentinel2ThumbnailAsset(product)?.url, thumbUrl);
});
