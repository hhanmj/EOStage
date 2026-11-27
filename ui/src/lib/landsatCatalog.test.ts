import assert from "node:assert/strict";
import test from "node:test";

import {
  LANDSAT_C2_L2_COLLECTION_ID,
  LANDSAT_C2_L2_LOGICAL_DATASET,
  LANDSAT_C2_L2_PROVIDER_ID,
  MPC_LANDSAT_PROVIDER_ID,
  MPC_LANDSAT_COLLECTION_ID,
  EARTH_SEARCH_LANDSAT_PROVIDER_ID,
  EARTH_SEARCH_LANDSAT_COLLECTION_ID,
  landsatAssetDetailsAvailable,
  landsatSourceIdentity,
  landsatMapTilePreview,
  buildLandsatC2L2CatalogQuery,
} from "./landsatCatalog.ts";
import type { SarProduct } from "./generated/v3-contracts.ts";

const aoi = {
  type: "Polygon",
  coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
};

test("Landsat C2 L2 search builds one exact dataset query", () => {
  const result = buildLandsatC2L2CatalogQuery({
    aoiGeojson: aoi,
    startUtc: "2023-03-24",
    endUtc: "2023-03-27",
    cloudCoverLte: "35",
    spacecraft: ["landsat_8", "landsat_9"],
    processingLevels: ["l2sp", "l2sr"],
    resultLimit: 24,
  });
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.query.logical_dataset, LANDSAT_C2_L2_LOGICAL_DATASET);
  assert.equal(result.query.provider_id, MPC_LANDSAT_PROVIDER_ID);
  assert.equal(result.query.native_collection_id, MPC_LANDSAT_COLLECTION_ID);
  assert.deepEqual(result.query.filters, {
    datetime: {
      start: "2023-03-24T00:00:00Z",
      end: "2023-03-27T00:00:00Z",
    },
    cloud_cover_lte: 35,
    spacecraft: ["landsat_8", "landsat_9"],
    processing_levels: ["l2sp", "l2sr"],
  });
});

test("new source binding is Microsoft while legacy source identity remains readable and exact", () => {
  const microsoft = landsatSourceIdentity(MPC_LANDSAT_PROVIDER_ID, MPC_LANDSAT_COLLECTION_ID);
  const legacy = landsatSourceIdentity(LANDSAT_C2_L2_PROVIDER_ID, LANDSAT_C2_L2_COLLECTION_ID);
  assert.equal(microsoft?.profileId, "planetary-computer.landsat-c2-l2.scene.v1");
  assert.equal(legacy?.profileId, "usgs.eros.m2m.landsat.c2.l2.scene.v1");
  assert.equal(landsatSourceIdentity(MPC_LANDSAT_PROVIDER_ID, LANDSAT_C2_L2_COLLECTION_ID), null);
  assert.equal(landsatSourceIdentity(LANDSAT_C2_L2_PROVIDER_ID, MPC_LANDSAT_COLLECTION_ID), null);
  assert.equal(landsatSourceIdentity("unknown", MPC_LANDSAT_COLLECTION_ID), null);
  assert.equal(
    landsatSourceIdentity(EARTH_SEARCH_LANDSAT_PROVIDER_ID, EARTH_SEARCH_LANDSAT_COLLECTION_ID)?.profileId,
    "element84.earth-search.landsat-c2-l2.scene.v1",
  );
  assert.equal(landsatSourceIdentity(EARTH_SEARCH_LANDSAT_PROVIDER_ID, LANDSAT_C2_L2_COLLECTION_ID), null);
});

test("Microsoft Landsat map preview is source-bound and uses the frozen STAC item identity", () => {
  const key = "PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::LC09_L2SP_123045_20260101_02_T1";
  const product = {
    product_key: key,
    provider_product_key: key,
    provider_product_id: "LC09_L2SP_123045_20260101_02_T1",
    product_id: "LC09_L2SP_123045_20260101_02_T1",
    scene_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
    provider: MPC_LANDSAT_PROVIDER_ID,
    dataset: "LANDSAT 8-9 C2 L2",
    platform: "landsat_9",
    instrument: "OLI/TIRS",
    processing_level: "l2sp",
    product_family: "OLI_TIRS_L2SP",
    beam_mode: null,
    polarization: null,
    flight_direction: null,
    relative_orbit: null,
    absolute_orbit: null,
    frame: null,
    asf_frame: null,
    path: null,
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
    },
    footprint_bbox: null,
    file_name: null,
    file_size_remote: null,
    download_url: null,
    native_metadata: {
      logical_dataset: LANDSAT_C2_L2_LOGICAL_DATASET,
      native_collection_id: MPC_LANDSAT_COLLECTION_ID,
      source_kind: "landsat",
      product_kind: "optical_scene",
      properties: {
        entity_id: "LC09_L2SP_123045_20260101_02_T1",
        browse_available: true,
      },
    },
    assets: [],
  } as SarProduct;

  assert.equal(landsatAssetDetailsAvailable(product), true);
  const preview = landsatMapTilePreview(product);
  assert.ok(preview);
  assert.equal(preview.productKey, key);
  assert.equal(preview.origin, "https://planetarycomputer.microsoft.com");
  assert.match(preview.urlTemplate, /^https:\/\/planetarycomputer\.microsoft\.com\/api\/data\/v1\/item\/tiles\/WebMercatorQuad\/\{z\}\/\{x\}\/\{y\}@1x\?/);
  assert.match(preview.urlTemplate, /collection=landsat-c2-l2/);
  assert.match(preview.urlTemplate, /item=LC09_L2SP_123045_20260101_02_T1/);
  assert.match(preview.urlTemplate, /assets=red&assets=green&assets=blue/);
  assert.match(preview.urlTemplate, /color_formula=gamma\+RGB\+2\.7%2C\+saturation\+1\.5%2C\+sigmoidal\+RGB\+15\+0\.55/);
  assert.match(preview.urlTemplate, /nodata=0&format=png$/);

  const legacy = { ...product, provider: LANDSAT_C2_L2_PROVIDER_ID };
  assert.equal(landsatAssetDetailsAvailable(legacy), false);
  assert.equal(landsatMapTilePreview(legacy), null);
  const searchOnly = { ...product, provider: EARTH_SEARCH_LANDSAT_PROVIDER_ID };
  assert.equal(landsatAssetDetailsAvailable(searchOnly), false);
  assert.equal(landsatMapTilePreview(searchOnly), null);
  const wrongProduct = { ...product, product_id: "LC08_L2SP_047027_20201204_02_T1" };
  assert.equal(landsatAssetDetailsAvailable(wrongProduct), false);
  assert.equal(landsatMapTilePreview(wrongProduct), null);
  const wrongProviderProduct = { ...product, provider_product_id: "LC08_L2SP_047027_20201204_02_T1" };
  assert.equal(landsatAssetDetailsAvailable(wrongProviderProduct), false);
  assert.equal(landsatMapTilePreview(wrongProviderProduct), null);
  assert.equal(landsatMapTilePreview({
    ...product,
    native_metadata: {
      ...product.native_metadata,
      properties: { entity_id: "LC09_L2SP_123045_20260101_02_T1", browse_available: false },
    },
  }), null);
  assert.equal(landsatAssetDetailsAvailable({
    ...product,
    native_metadata: {
      ...product.native_metadata,
      properties: { entity_id: "LC09_L2SP_123045_20260101_02_T1", browse_available: false },
    },
  }), true, "asset details do not depend on browse availability");
  assert.equal(landsatMapTilePreview({
    ...product,
    native_metadata: { ...product.native_metadata, native_collection_id: LANDSAT_C2_L2_COLLECTION_ID },
  }), null);
  assert.equal(landsatAssetDetailsAvailable({
    ...product,
    native_metadata: { ...product.native_metadata, native_collection_id: LANDSAT_C2_L2_COLLECTION_ID },
  }), false);
  assert.equal(landsatMapTilePreview({
    ...product,
    native_metadata: {
      ...product.native_metadata,
      properties: { entity_id: "../../not-an-item", browse_available: true },
    },
  }), null);
});

test("Landsat query rejects missing geometry and unreviewed mission or level values", () => {
  const base = {
    aoiGeojson: aoi,
    startUtc: "2023-03-24T00:00:00Z",
    endUtc: "2023-03-27T00:00:00Z",
    cloudCoverLte: "",
    spacecraft: ["landsat_9"],
    processingLevels: ["l2sp"],
    resultLimit: 24,
  };
  assert.equal(buildLandsatC2L2CatalogQuery({ ...base, aoiGeojson: null }).ok, false);
  assert.equal(buildLandsatC2L2CatalogQuery({ ...base, spacecraft: ["landsat_7"] }).ok, true);
  assert.equal(buildLandsatC2L2CatalogQuery({ ...base, spacecraft: ["landsat_3"] }).ok, false);
  assert.equal(buildLandsatC2L2CatalogQuery({ ...base, processingLevels: ["l1tp"] }).ok, false);
  assert.equal(buildLandsatC2L2CatalogQuery({ ...base, cloudCoverLte: "100.1" }).ok, false);
});

test("Landsat blank dates mean all time with the bounded optical limit", () => {
  const allTime = buildLandsatC2L2CatalogQuery({
    aoiGeojson: aoi,
    startUtc: "",
    endUtc: "",
    cloudCoverLte: "",
    spacecraft: ["landsat_8", "landsat_9"],
    processingLevels: ["l2sp", "l2sr"],
    resultLimit: 100,
  });
  assert.equal(allTime.ok, true);
  if (allTime.ok) {
    assert.equal("datetime" in allTime.query.filters, false);
    assert.equal(allTime.query.result_limit, 100);
  }
  assert.equal(buildLandsatC2L2CatalogQuery({
    aoiGeojson: aoi,
    startUtc: "",
    endUtc: "2023-03-27",
    cloudCoverLte: "",
    spacecraft: ["landsat_9"],
    processingLevels: ["l2sp"],
    resultLimit: 100,
  }).ok, false);
});
