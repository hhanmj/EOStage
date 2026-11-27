import assert from "node:assert/strict";
import test from "node:test";
import { landsatBrowseAvailable, landsatDownloadDisabledReason } from "./landsatCatalog.ts";
import { deriveCatalogPreview } from "./catalogPreview.ts";
import { selectBurstAssetsForDownload } from "./sarProduct.ts";

import {
  historyItemFromProjection,
  projectApplicationSnapshot,
  projectFrozenItemToScene,
  taskStatusFromDownloadSnapshot,
  taskStatusFromProjection,
  taskWorkbenchFromProjection,
} from "./applicationServiceProjection.ts";
import { isSentinel2L1cCatalogProduct } from "./sentinel2Catalog.ts";
import { opticalQuicklookPlan } from "../shell/api/opticalQuicklookLogic.ts";

const ITEM_ID = "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-M1-FIXTURE";

function serviceSnapshot() {
  const candidate = {
    item_id: ITEM_ID,
    provider_id: "asf.catalog",
    display_name: "S1-M1-FIXTURE.zip",
    acquisition_datetime: "2026-07-25T00:00:00Z",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]],
    },
    metadata: {
      provider: "asf.catalog",
      dataset: "SENTINEL-1",
      product_id: "S1-M1-FIXTURE",
      product_family: "SLC",
      scene_id: "S1-M1-FIXTURE",
      platform: "S1A",
      processing_level: "SLC",
      beam_mode: "IW",
      polarization: "VV",
      file_name: "S1-M1-FIXTURE.zip",
      file_size_remote: 4,
    },
    payload: {
      file_name: "S1-M1-FIXTURE.zip",
      file_size_remote: 4,
      download_url: "http://127.0.0.1:18080/S1-M1-FIXTURE.zip",
    },
    asset_access: null,
    execution_policy: null,
  };
  const aoi = {
    type: "Polygon",
    coordinates: [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]],
  };
  return {
    ok: true as const,
    readiness: {
      contract: "application_service_readonly_v1",
      status: "store_ready",
      store_schema_version: 1,
    },
    restore: {
      workbench_batch: {
        schema_version: 1,
        batch_id: "m1-batch",
        provider_id: "asf.catalog",
        created_at: "2026-07-25T00:00:00Z",
        aoi_name: "Fixture AOI",
        aoi_geojson: aoi,
        query: { provider: "asf", dataset: "SENTINEL-1", aoi_geojson: aoi },
        candidates: [candidate],
      },
      workbench: {
        batch_id: "m1-batch",
        checked_item_ids: [ITEM_ID],
        page_index: 1,
        page_size: 24,
        revision: 3,
      },
      tasks: [{
        schema_version: 1,
        task_id: "m1-task",
        batch_id: "m1-batch",
        provider_id: "asf.catalog",
        created_at: "2026-07-25T00:00:01Z",
        aoi_name: "Fixture AOI",
        aoi_geojson: aoi,
        query: { provider: "asf", dataset: "SENTINEL-1" },
        candidates: [candidate],
        initial_selected_item_ids: [ITEM_ID],
        output_root: "C:/fixture-output",
        layout: "sentinel1_slc",
      }],
      runtimes: [{
        task_id: "m1-task",
        status: "paused",
        selected_item_ids: [ITEM_ID],
        checked_item_ids: [ITEM_ID],
        page_index: 0,
        page_size: 24,
        active_item_ids: [],
        paused_item_ids: [ITEM_ID],
        queued_item_ids: [],
        completed_item_ids: [],
        failed_item_ids: [],
        skipped_item_ids: [],
        cancelled_item_ids: [],
        task_paused: true,
        concurrency: 2,
        revision: 7,
        updated_at: "2026-07-25T00:00:02Z",
      }],
    },
  };
}

test("M1 projection derives the catalogue and task view only from a Store snapshot", () => {
  const projection = projectApplicationSnapshot(serviceSnapshot());

  assert.ok(projection?.workbench);
  assert.equal(projection.workbench.batch_context.batch_id, "m1-batch");
  assert.equal(projection.workbench.batch_context.revision, 3);
  assert.deepEqual(projection.workbench.checked_item_ids, [ITEM_ID]);
  assert.equal(projection.workbench.scenes[0]?.product_key, ITEM_ID);
  assert.equal(projection.workbench.scenes[0]?.sar_product?.dataset, "SENTINEL-1");
  assert.deepEqual(projection.workbench.scenes[0]?.footprint_bbox, {
    west: 100,
    south: 30,
    east: 101,
    north: 31,
    crs: "EPSG:4326",
  });

  const task = projection.tasks[0]!;
  const workbench = taskWorkbenchFromProjection(task);
  const status = taskStatusFromProjection(task);
  const history = historyItemFromProjection(task);
  assert.equal(workbench.task_id, "m1-task");
  assert.deepEqual(workbench.selected_scene_ids, [ITEM_ID]);
  assert.equal(status.state, "paused");
  assert.equal(status.revision, 7);
  assert.deepEqual(status.paused_scene_ids, [ITEM_ID]);
  assert.equal(history.task_id, "m1-task");
  assert.equal(history.output_dir, "C:/fixture-output");
});

test("M1 projection rejects a partial Store payload instead of manufacturing UI truth", () => {
  const snapshot = serviceSnapshot();
  snapshot.restore.workbench_batch.candidates[0]!.metadata.dataset = 12 as unknown as string;
  assert.equal(projectApplicationSnapshot(snapshot), null);
});

test("generic frozen Polygon and MultiPolygon retain authoritative geometry and derive exact bounds", () => {
  const generic = (footprint_geojson: Record<string, unknown>) => ({
    item_id: "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::footprint-fixture",
    provider_id: "asf.catalog",
    display_name: "footprint-fixture.zip",
    acquisition_datetime: "2026-08-17T00:00:00Z",
    footprint_geojson,
    metadata: {
      provider: "asf.catalog",
      dataset: "SENTINEL-1",
      product_id: "footprint-fixture",
      product_family: "SLC",
    },
    payload: {},
  });
  const polygon = {
    type: "Polygon",
    coordinates: [[[120, 20], [121, 20], [121, 21], [120, 21], [120, 20]]],
  };
  const multiPolygon = {
    type: "MultiPolygon",
    coordinates: [
      [[[120, 20], [121, 20], [121, 21], [120, 21], [120, 20]]],
      [[[123, 22], [124, 22], [124, 23], [123, 23], [123, 22]]],
    ],
  };

  const polygonScene = projectFrozenItemToScene(generic(polygon));
  const multiPolygonScene = projectFrozenItemToScene(generic(multiPolygon));

  assert.strictEqual(polygonScene?.footprint_geojson, polygon);
  assert.deepEqual(polygonScene?.footprint_bbox, {
    west: 120, south: 20, east: 121, north: 21, crs: "EPSG:4326",
  });
  assert.strictEqual(multiPolygonScene?.footprint_geojson, multiPolygon);
  assert.deepEqual(multiPolygonScene?.footprint_bbox, {
    west: 120, south: 20, east: 124, north: 23, crs: "EPSG:4326",
  });
});

test("generic frozen malformed or non-geographic footprints fail closed", () => {
  const invalid = {
    item_id: "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::invalid-footprint",
    provider_id: "asf.catalog",
    display_name: "invalid-footprint.zip",
    acquisition_datetime: "2026-08-17T00:00:00Z",
    footprint_geojson: { type: "Polygon", coordinates: [[[120, 20], [181, 20], [120, 20]]] },
    metadata: {
      provider: "asf.catalog",
      dataset: "SENTINEL-1",
      product_id: "invalid-footprint",
      product_family: "SLC",
    },
    payload: {},
  };

  assert.equal(projectFrozenItemToScene(invalid), null);
});

// Issue #15: the shape the Rust M1 freeze (map_sar_product) writes for one
// Sentinel-1D SLC-BURST candidate, representative Store snapshot fixture.
function frozenBurstItem() {
  const productId = "S1_078090_IW3_20261012T054947_VV_F6D9-BURST";
  const itemId = `ASF::SLC-BURST::${productId}`;
  const href = "https://sentinel1-burst.asf.alaska.edu/S1D_IW_SLC__1SDV_20261012T054945_20261012T055013_004978_009653_F6D9/IW3/VV";
  return {
    item_id: itemId,
    provider_id: "asf.catalog",
    display_name: productId,
    acquisition_datetime: "2026-10-12T05:49:48.330055Z",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[5.267156, 52.119039], [4.111976, 52.236646], [4.064718, 52.031626], [5.217344, 51.923238], [5.267156, 52.119039]]],
    },
    metadata: {
      product_key: itemId,
      provider: "asf.catalog",
      dataset: "SLC-BURST",
      product_id: productId,
      product_family: "BURST",
      provider_product_key: itemId,
      provider_product_id: "G4359594348-ASF",
      identity_rule_id: "asf.sentinel-1.burst.v1",
      scene_id: productId,
      platform: "SENTINEL-1D",
      instrument: "C-SAR",
      processing_level: "L1",
      beam_mode: "IW",
      beam_swath: "IW3",
      polarization: "VV",
      flight_direction: "DESCENDING",
      relative_orbit: 37,
      absolute_orbit: 4978,
      path: 37,
      acquisition_stop: "2026-10-12T05:49:51.462723Z",
      stable_asset_keys: [
        `${itemId}::ASSET::${productId}:0.tiff`,
        `${itemId}::ASSET::${productId}:0.xml`,
      ],
      assets: [
        {
          asset_key: `${productId}:0.tiff`,
          stable_asset_key: `${itemId}::ASSET::${productId}:0.tiff`,
          access_descriptor: null,
          role: "primary",
          native_roles: ["USE SERVICE API"],
          url: `${href}/0.tiff`,
          file_name: "0.tiff",
          media_type: "image/tiff",
          size_bytes: 156057600,
        },
        {
          asset_key: `${productId}:0.xml`,
          stable_asset_key: `${itemId}::ASSET::${productId}:0.xml`,
          access_descriptor: null,
          role: "metadata",
          native_roles: ["USE SERVICE API"],
          url: `${href}/0.xml`,
          file_name: "0.xml",
          media_type: "application/xml",
          size_bytes: null,
        },
      ],
      burst: {
        burst_id: null,
        full_burst_id: "037_078090_IW3",
        relative_burst_id: 78090,
        absolute_burst_id: 10691002,
        subswath: "IW3",
        burst_index: 0,
      },
      native_metadata: { cmr_native_id: productId },
    },
    payload: { file_size_remote: 156057600 },
    asset_access: null,
    execution_policy: null,
  };
}

test("frozen SLC-BURST candidates keep their official assets and burst identity", () => {
  const item = frozenBurstItem();
  const scene = projectFrozenItemToScene(item);
  const product = scene?.sar_product;

  assert.ok(product);
  assert.equal(product.dataset, "SLC-BURST");
  assert.deepEqual(product.assets, item.metadata.assets);
  assert.deepEqual(product.burst, item.metadata.burst);
  assert.equal(product.file_size_remote, 156057600);
  assert.deepEqual(
    selectBurstAssetsForDownload(product, "primary").assets?.map((asset) => asset.file_name),
    ["0.tiff"],
  );
  assert.deepEqual(
    selectBurstAssetsForDownload(product, "all").assets?.map((asset) => asset.role),
    ["primary", "metadata"],
  );
});

test("frozen SLC-BURST candidates with malformed asset evidence fail closed", () => {
  const unknownRole = frozenBurstItem();
  unknownRole.metadata.assets[1] = { ...unknownRole.metadata.assets[1], role: "sidecar" };
  assert.equal(projectFrozenItemToScene(unknownRole), null);

  const notAList = frozenBurstItem() as unknown as { metadata: Record<string, unknown> };
  notAList.metadata.assets = { primary: "0.tiff" };
  assert.equal(projectFrozenItemToScene(notAList), null);

  const brokenBurst = frozenBurstItem() as unknown as { metadata: Record<string, unknown> };
  brokenBurst.metadata.burst = { subswath: 3 };
  assert.equal(projectFrozenItemToScene(brokenBurst), null);

  const missingUrl = frozenBurstItem();
  missingUrl.metadata.assets[0] = { ...missingUrl.metadata.assets[0], url: null as unknown as string };
  assert.equal(projectFrozenItemToScene(missingUrl), null);
});

test("a frozen per-asset Burst task item still projects for the task page", () => {
  // freeze_burst_task_items keeps the parent metadata without its asset list.
  const source = frozenBurstItem();
  const { assets: _assets, ...parentMetadata } = source.metadata;
  const primary = source.metadata.assets[0]!;
  const taskItem = {
    ...source,
    item_id: primary.stable_asset_key,
    display_name: primary.file_name,
    metadata: {
      ...parentMetadata,
      source_item_id: source.item_id,
      stable_asset_key: primary.stable_asset_key,
      asset_role: "primary",
      official_href: primary.url,
      asset_basename: primary.file_name,
      media_type: primary.media_type,
      size_bytes: primary.size_bytes,
      burst_asset_policy: "all",
    },
    payload: {
      burst_asset: true, download_url: primary.url, file_name: primary.file_name, media_type: primary.media_type,
      source_item_id: source.item_id, asset_role: "primary", burst_asset_policy: "all", file_size_remote: primary.size_bytes,
    },
  };
  const product = projectFrozenItemToScene(taskItem)?.sar_product;
  assert.ok(product);
  assert.deepEqual(product.assets, []);
  assert.deepEqual(product.burst, source.metadata.burst);
});

test("M1 projection keeps Sentinel-2 archive evidence and ignores legacy quicklook metadata", () => {
  const itemId = "PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::165ba4c9-997c-4cea-970f-33ab9bdb8084";
  const productId = "165ba4c9-997c-4cea-970f-33ab9bdb8084";
  const productName = "S2C_MSIL2A_20260726T125321_N0512_R138_T27VXL_20260726T221014.SAFE";
  const quicklookId = "10000000-0000-4000-8000-000000000001";
  const snapshot = {
    ok: true as const,
    readiness: {
      contract: "application_service_readonly_v1",
      status: "store_ready",
      store_schema_version: 1,
    },
    restore: {
      workbench_batch: {
        batch_id: "sentinel2-batch",
        provider_id: "copernicus.dataspace.odata",
        created_at: "2026-07-27T00:00:00Z",
        aoi_name: "Fixture AOI",
        aoi_geojson: {
          type: "Polygon",
          coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
        },
        query: { logical_dataset: "optical.sentinel-2.l2a" },
        candidates: [{
          item_id: itemId,
          provider_id: "copernicus.dataspace.odata",
          display_name: productName,
          acquisition_datetime: "2026-07-26T12:53:21Z",
          footprint_geojson: {
            type: "Polygon",
            coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]],
          },
          metadata: {
            logical_dataset: "optical.sentinel-2.l2a",
            native_collection_id: "SENTINEL-2",
            collection_profile_id: "cdse.sentinel-2.l2a.collection.v1",
            identity_rule_id: "cdse.sentinel-2.l2a.identity.v1",
            filename_rule_id: "cdse.sentinel-2.l2a.archive-filename.v1",
            delivery_model: "archive_first",
            resolver_policy_id: "cdse.sentinel-2.l2a.policy.v1",
            provider_product_id: productId,
            provider_product_key: itemId,
            source_kind: "sentinel2",
            product_kind: "optical_scene",
            properties: {
              product_type: "S2MSI2A",
              online: true,
              cloud_cover: 4.2,
              content_date_end: "2026-07-26T13:02:11Z",
              checksums: [{ algorithm: "MD5", value: "0123456789abcdef0123456789abcdef" }],
            },
            assets: [
              {
                asset_key: "archive",
                stable_asset_key: `${itemId}::ASSET::archive`,
                role: "primary",
                native_roles: ["archive"],
                size_bytes: 867907004,
              },
              {
                asset_key: `quicklook-${quicklookId}`,
                stable_asset_key: `${itemId}::ASSET::quicklook-${quicklookId}`,
                role: "thumbnail",
                native_roles: ["thumbnail"],
                href: `https://catalogue.dataspace.copernicus.eu/odata/v1/Assets(${quicklookId})/$value`,
                media_type: "image/jpeg",
                size_bytes: 2048,
              },
            ],
          },
          payload: {},
          asset_access: null,
          execution_policy: null,
        }],
      },
      workbench: {
        batch_id: "sentinel2-batch",
        checked_item_ids: [itemId],
        page_index: 0,
        page_size: 24,
        revision: 1,
      },
      tasks: [],
      runtimes: [],
    },
  };

  const projection = projectApplicationSnapshot(snapshot);
  const product = projection?.workbench?.scenes[0]?.sar_product;
  assert.ok(product);
  assert.equal(product.dataset, "SENTINEL-2 L2A");
  assert.equal(product.processing_level, "L2A");
  assert.equal(product.platform, "S2C");
  assert.equal(product.file_name, `${productName}.zip`);
  assert.equal(product.file_size_remote, 867907004);
  assert.equal(product.checksum, "md5:0123456789abcdef0123456789abcdef");
  assert.equal(product.download_url, null);
  assert.equal(product.assets?.length, 1);
  assert.equal(product.assets?.[0]?.role, "primary");
  assert.equal(product.assets?.some((asset) => asset.role === "thumbnail"), false);
  assert.deepEqual(product.footprint_bbox, {
    west: 110, south: 30, east: 111, north: 31, crs: "EPSG:4326",
  });
});

test("M1 projection keeps Microsoft Sentinel-2 search-only identity without archive authority", () => {
  const providerProductId = "S2B_MSIL2A_20250129T022849_R046_T51RUH_20250129T040545";
  const productName = "S2B_MSIL2A_20250129T022849_N0511_R046_T51RUH_20250129T040545.SAFE";
  const itemId = `PROVIDER::microsoft.planetary-computer::COLLECTION::sentinel-2-l2a::ITEM::${providerProductId}`;
  const scene = projectFrozenItemToScene({
    item_id: itemId,
    provider_id: "microsoft.planetary-computer",
    display_name: productName,
    acquisition_datetime: "2025-01-29T02:28:49.024Z",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[121, 24], [122, 24], [122, 25], [121, 25], [121, 24]]],
    },
    metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "sentinel-2-l2a",
      collection_profile_id: "planetary-computer.sentinel-2-l2a.scene.v1",
      identity_rule_id: "planetary-computer.sentinel-2-l2a.identity.v1",
      filename_rule_id: "planetary-computer.sentinel-2-l2a.product-uri.v1",
      delivery_model: "archive_first",
      resolver_policy_id: "planetary-computer.sentinel-2-l2a.search-only.v1",
      provider_product_id: providerProductId,
      provider_product_key: itemId,
      source_kind: "sentinel2",
      product_kind: "optical_scene",
      properties: { product_type: "S2MSI2A", cloud_cover: 43.816283 },
      assets: [],
    },
    payload: {},
    asset_access: null,
    execution_policy: null,
  });

  const product = scene?.sar_product;
  assert.ok(product);
  assert.equal(product.provider, "microsoft.planetary-computer");
  assert.equal(product.product_id, providerProductId);
  assert.equal(product.scene_id, productName);
  assert.equal(product.platform, "S2B");
  assert.equal(product.file_name, null);
  assert.equal(product.file_size_remote, null);
  assert.equal(product.download_url, null);
  assert.deepEqual(product.assets, []);
});

test("M1 projection preserves frozen Landsat scene identity and native optical fields", () => {
  const itemId = "PROVIDER::usgs.eros.m2m::COLLECTION::landsat_ot_c2_l2::ITEM::LC91230452026001LGN00";
  const snapshot = {
    ok: true as const,
    readiness: {
      contract: "application_service_readonly_v1",
      status: "store_ready",
      store_schema_version: 1,
    },
    restore: {
      workbench_batch: {
        batch_id: "landsat-batch",
        provider_id: "usgs.eros.m2m",
        created_at: "2026-07-29T00:00:00Z",
        aoi_name: "Fixture AOI",
        aoi_geojson: {
          type: "Polygon",
          coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
        },
        query: { logical_dataset: "optical.landsat.c2.l2" },
        candidates: [{
          item_id: itemId,
          provider_id: "usgs.eros.m2m",
          display_name: "LC09_L2SP_123045_20260101_20260103_02_T1",
          acquisition_datetime: "2026-01-01T00:00:00Z",
          footprint_geojson: {
            type: "Polygon",
            coordinates: [[[110, 30], [111, 30], [111, 31], [110, 30]]],
          },
          metadata: {
            logical_dataset: "optical.landsat.c2.l2",
            native_collection_id: "landsat_ot_c2_l2",
            collection_profile_id: "usgs.eros.m2m.landsat.c2.l2.scene.v1",
            identity_rule_id: "usgs.eros.m2m.landsat.c2.l2.identity.v1",
            filename_rule_id: "usgs.eros.m2m.landsat.c2.l2.full-bundle-filename.v1",
            delivery_model: "archive_first",
            provider_product_id: "LC91230452026001LGN00",
            provider_product_key: itemId,
            source_kind: "landsat",
            product_kind: "optical_scene",
            properties: {
              entity_id: "LC91230452026001LGN00",
              landsat_product_id: "LC09_L2SP_123045_20260101_20260103_02_T1",
              landsat_scene_id: "LC91230452026001LGN00",
              dataset_alias: "landsat_ot_c2_l2",
              spacecraft: "landsat_9",
              processing_level: "l2sp",
              data_type_l2: "OLI_TIRS_L2SP",
              collection_category: "T1",
              cloud_cover: 12.5,
              wrs_path: "123",
              wrs_row: "045",
              acquisition_date: "2026/01/01",
              acquisition_precision: "day",
              browse_available: true,
            },
            assets: [],
          },
          payload: {},
          asset_access: null,
          execution_policy: null,
        }],
      },
      workbench: {
        batch_id: "landsat-batch",
        checked_item_ids: [],
        page_index: 0,
        page_size: 24,
        revision: 0,
      },
      tasks: [],
      runtimes: [],
    },
  };

  const product = projectApplicationSnapshot(snapshot)?.workbench?.scenes[0]?.sar_product;
  assert.ok(product);
  assert.equal(product.product_key, itemId);
  assert.equal(product.product_id, "LC91230452026001LGN00");
  assert.equal(product.scene_id, "LC09_L2SP_123045_20260101_20260103_02_T1");
  assert.equal(product.dataset, "LANDSAT 8-9 C2 L2");
  assert.equal(product.platform, "landsat_9");
  assert.equal(product.processing_level, "l2sp");
  assert.equal(product.download_url, null);
  assert.deepEqual(product.assets, []);
  assert.equal(product.native_metadata?.properties?.wrs_path, "123");
  assert.equal(product.native_metadata?.properties?.browse_available, true);
  assert.deepEqual(product.footprint_bbox, {
    west: 110, south: 30, east: 111, north: 31, crs: "EPSG:4326",
  });

  // The same scientific scene from Microsoft retains a different source key.
  // This never mutates the legacy snapshot used above.
  const microsoft = structuredClone(snapshot);
  const batch = microsoft.restore.workbench_batch;
  const candidate = batch.candidates[0];
  const nativeId = "LC09_L2SP_123045_20260101_02_T1";
  const nativeKey = `PROVIDER::microsoft.planetary-computer::COLLECTION::landsat-c2-l2::ITEM::${nativeId}`;
  batch.provider_id = "microsoft.planetary-computer";
  candidate.provider_id = batch.provider_id;
  candidate.item_id = nativeKey;
  Object.assign(candidate.metadata, {
    native_collection_id: "landsat-c2-l2",
    collection_profile_id: "planetary-computer.landsat-c2-l2.scene.v1",
    identity_rule_id: "planetary-computer.landsat-c2-l2.identity.v1",
    filename_rule_id: "planetary-computer.landsat-c2-l2.asset-filename.v1",
    provider_product_id: nativeId,
    provider_product_key: nativeKey,
  });
  candidate.metadata.properties.entity_id = nativeId;
  candidate.metadata.properties.dataset_alias = "landsat-c2-l2";
  const native = projectApplicationSnapshot(microsoft)?.workbench?.scenes[0]?.sar_product;
  assert.ok(native);
  assert.equal(native.provider, "microsoft.planetary-computer");
  assert.equal(native.product_key, nativeKey);
  assert.equal(native.product_id, nativeId);
  assert.equal(native.scene_id, product.scene_id);
  assert.deepEqual(native.footprint_bbox, product.footprint_bbox);
  assert.equal(native.native_metadata?.properties?.landsat_scene_id, "LC91230452026001LGN00");
  assert.equal(native.download_url, null);
  assert.equal(snapshot.restore.workbench_batch.candidates[0].item_id, itemId);

  const earth = structuredClone(microsoft);
  const earthBatch = earth.restore.workbench_batch;
  const earthItem = earthBatch.candidates[0];
  const earthKey = `PROVIDER::element84.earth-search::COLLECTION::landsat-c2-l2::ITEM::${nativeId}`;
  earthBatch.provider_id = "element84.earth-search";
  earthItem.provider_id = earthBatch.provider_id;
  earthItem.item_id = earthKey;
  Object.assign(earthItem.metadata, {
    collection_profile_id: "element84.earth-search.landsat-c2-l2.scene.v1",
    identity_rule_id: "element84.earth-search.landsat-c2-l2.identity.v1",
    filename_rule_id: "element84.earth-search.landsat-c2-l2.asset-filename.v1",
    provider_product_key: earthKey,
  });
  earthItem.metadata.properties.browse_available = false;
  const earthProduct = projectApplicationSnapshot(earth)?.workbench?.scenes[0]?.sar_product;
  assert.ok(earthProduct);
  assert.equal(earthProduct.provider, "element84.earth-search");
  assert.equal(earthProduct.product_key, earthKey);
  assert.equal(earthProduct.product_id, nativeId);
  assert.equal(earthProduct.scene_id, native.scene_id);
  assert.deepEqual(earthProduct.footprint_bbox, native.footprint_bbox);
  assert.equal(earthProduct.download_url, null);
  assert.deepEqual(earthProduct.assets, []);
  assert.match(landsatDownloadDisabledReason(earthProduct) ?? "", /仅支持搜索/);
  assert.equal(landsatDownloadDisabledReason(native), null);
  assert.equal(landsatBrowseAvailable(earthProduct), false);
  assert.equal(deriveCatalogPreview(earthProduct).kind, "unavailable");
  earthItem.metadata.properties.browse_available = true;
  const forgedBrowse = projectApplicationSnapshot(earth)?.workbench?.scenes[0]?.sar_product;
  assert.ok(forgedBrowse);
  assert.equal(landsatBrowseAvailable(forgedBrowse), false);
  assert.equal(deriveCatalogPreview(forgedBrowse).kind, "unavailable");
  earthItem.metadata.collection_profile_id = "planetary-computer.landsat-c2-l2.scene.v1";
  assert.equal(projectApplicationSnapshot(earth)?.workbench?.scenes[0]?.sar_product ?? null, null);

  candidate.metadata.identity_rule_id = "usgs.eros.m2m.landsat.c2.l2.identity.v1";
  assert.equal(projectApplicationSnapshot(microsoft)?.workbench?.scenes[0]?.sar_product ?? null, null);
  candidate.metadata.identity_rule_id = "planetary-computer.landsat-c2-l2.identity.v1";
  candidate.metadata.native_collection_id = "landsat_ot_c2_l2";
  assert.equal(projectApplicationSnapshot(microsoft)?.workbench?.scenes[0]?.sar_product ?? null, null);
});

test("M1 download projection uses persisted engine progress, results, and output", () => {
  const projection = projectApplicationSnapshot(serviceSnapshot());
  const task = projection?.tasks[0];
  assert.ok(task);
  const status = taskStatusFromDownloadSnapshot(task, {
    task_id: "m1-task",
    revision: 4,
    progress: [{
      task_id: "m1-task",
      item_id: ITEM_ID,
      status: "active",
      bytes_completed: 2,
      total_bytes: 4,
      bytes_per_second: 8,
      attempt: 1,
      updated_at: "2026-07-25T00:00:03Z",
    }],
    results: [],
  });
  assert.equal(status?.current_bytes, 2);
  assert.equal(status?.current_expected_size, 4);
  assert.equal(status?.bytes_per_second, 8);

  const completed = taskStatusFromDownloadSnapshot(task, {
    task_id: "m1-task",
    revision: 5,
    progress: [],
    results: [{
      task_id: "m1-task",
      item_id: ITEM_ID,
      outcome: "completed",
      destination: "C:/fixture-output/S1-M1-FIXTURE.zip",
      bytes_written: 4,
      attempt_count: 1,
      failure: null,
      finished_at: "2026-07-25T00:00:04Z",
    }],
  });
  assert.equal(completed?.done_bytes, 4);
  assert.equal(completed?.results_path, "C:/fixture-output/S1-M1-FIXTURE.zip");
  assert.equal(completed?.log[0]?.outcome, "completed");
  assert.equal(historyItemFromProjection(task, completed).logs[0], "completed");
});

test("restricted JERS engine failure code survives status and history projection", () => {
  const snapshot = serviceSnapshot();
  snapshot.restore.tasks[0]!.candidates[0]!.metadata.dataset = "JERS-1";
  snapshot.restore.tasks[0]!.query.dataset = "JERS-1";
  const projection = projectApplicationSnapshot(snapshot);
  const task = projection?.tasks[0];
  assert.ok(task);

  const restricted = taskStatusFromDownloadSnapshot(task, {
    task_id: "m1-task",
    revision: 5,
    progress: [],
    results: [{
      task_id: "m1-task",
      item_id: ITEM_ID,
      outcome: "failed",
      destination: "C:/fixture-output/J1_TEST.zip",
      bytes_written: 0,
      attempt_count: 1,
      failure: {
        code: "ASF_DATASET_ACCESS_RESTRICTED",
        reason_zh: "JERS-1 是受限数据集。有关如何访问这些数据的信息，请联系 ASF 用户支持。",
        retryable: false,
      },
      finished_at: "2026-08-24T00:00:04Z",
    }],
  });

  assert.equal(restricted?.dataset_id, "JERS-1");
  assert.equal(restricted?.failure_code, "ASF_DATASET_ACCESS_RESTRICTED");
  assert.equal(restricted?.error, "JERS-1 是受限数据集。有关如何访问这些数据的信息，请联系 ASF 用户支持。");
  const history = historyItemFromProjection(task, restricted);
  assert.equal(history.dataset_id, "JERS-1");
  assert.equal(history.failure_code, "ASF_DATASET_ACCESS_RESTRICTED");
  assert.equal(history.detail, restricted?.error);
});

test("ordinary DL004 remains an ordinary credential failure", () => {
  const projection = projectApplicationSnapshot(serviceSnapshot());
  const task = projection?.tasks[0];
  assert.ok(task);

  const status = taskStatusFromDownloadSnapshot(task, {
    task_id: "m1-task",
    revision: 5,
    progress: [],
    results: [{
      task_id: "m1-task",
      item_id: ITEM_ID,
      outcome: "failed",
      destination: "C:/fixture-output/S1.zip",
      bytes_written: 0,
      attempt_count: 1,
      failure: { code: "DL004", reason_zh: "请配置 Earthdata 凭据。", retryable: false },
      finished_at: "2026-08-24T00:00:04Z",
    }],
  });

  assert.equal(status?.failure_code, "DL004");
  assert.equal(status?.error, null);
  assert.equal(status?.summary_line, "0/1");
});

test("M1 download projection rejects a snapshot for an unknown frozen item", () => {
  const projection = projectApplicationSnapshot(serviceSnapshot());
  const task = projection?.tasks[0];
  assert.ok(task);
  assert.equal(taskStatusFromDownloadSnapshot(task, {
    task_id: "m1-task",
    revision: 1,
    progress: [{
      task_id: "m1-task",
      item_id: "not-in-task",
      status: "active",
      bytes_completed: 1,
      total_bytes: 4,
      bytes_per_second: 1,
      attempt: 1,
      updated_at: "2026-07-25T00:00:03Z",
    }],
    results: [],
  }), null);
});

test("CDSE Sentinel-2 L1C frozen item projects with sar_product and cdse_quicklook authority", () => {
  const uuid = "3a1ec453-05ae-4aef-ac3e-148e7c5b412b";
  const itemId = `PROVIDER::copernicus.dataspace.odata::COLLECTION::SENTINEL-2::ITEM::${uuid}`;
  const displayName = "S2A_MSIL1C_20261116T030141_N0513_R032_T50SMA_20261116T045615.SAFE";
  const cdseL1cItem = {
    item_id: itemId,
    provider_id: "copernicus.dataspace.odata",
    display_name: displayName,
    acquisition_datetime: "2026-11-16T03:01:41Z",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[117.0, 31.6], [117.6, 31.6], [117.6, 32.1], [117.0, 32.1], [117.0, 31.6]]],
    },
    metadata: {
      logical_dataset: "optical.sentinel-2.l1c",
      native_collection_id: "SENTINEL-2",
      collection_profile_id: "cdse.sentinel-2.l1c.collection.v1",
      identity_rule_id: "cdse.sentinel-2.l1c.identity.v1",
      filename_rule_id: "cdse.sentinel-2.l1c.archive-filename.v1",
      provider_product_id: uuid,
      provider_product_key: itemId,
      source_kind: "sentinel2",
      product_kind: "optical_scene",
      properties: {
        product_type: "S2MSI1C",
        online: true,
        cloud_cover: 12.5,
        checksums: [{ algorithm: "MD5", value: "75f1cc48edef845e87c4dc86d415c396" }],
      },
      assets: [
        {
          asset_key: "archive",
          stable_asset_key: `ASSET::${itemId}::archive`,
          role: "primary",
          native_roles: ["archive"],
          size_bytes: 521297168,
        },
      ],
    },
    payload: {},
    asset_access: null,
    execution_policy: null,
  };

  const scene = projectFrozenItemToScene(cdseL1cItem);
  assert.ok(scene);
  assert.ok(scene.sar_product);
  assert.equal(scene.sar_product.dataset, "SENTINEL-2 L1C");
  assert.equal(scene.sar_product.processing_level, "L1C");
  assert.equal(scene.sar_product.product_family, "S2MSI1C");
  assert.equal(isSentinel2L1cCatalogProduct(scene.sar_product), true);
  assert.deepEqual(opticalQuicklookPlan(scene), { kind: "cdse_quicklook" });

  // Microsoft Planetary Computer search-only is never valid for Sentinel-2 L1C.
  const forgedMpcL1c = {
    ...cdseL1cItem,
    provider_id: "microsoft.planetary-computer",
    metadata: {
      ...cdseL1cItem.metadata,
      native_collection_id: "sentinel-2-l2a",
    },
  };
  assert.equal(projectFrozenItemToScene(forgedMpcL1c), null);
});

test("Earth Search Sentinel-2 L2A frozen item projects with public S3 thumbnail quicklook and deriveCatalogPreview candidate", () => {
  const stacId = "S2B_50RKU_20250915_0_L2A";
  const itemId = `PROVIDER::element84.earth-search::COLLECTION::sentinel-2-l2a::ITEM::${stacId}`;
  const displayName = "S2B_MSIL2A_20250915T024539_N0511_R132_T50RKU_20250915T061234.SAFE";
  const thumbUrl =
    "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-cogs/50/R/KU/2025/9/S2B_50RKU_20250915_0_L2A/thumbnail.jpg";
  const earthSearchItem = {
    item_id: itemId,
    provider_id: "element84.earth-search",
    display_name: displayName,
    acquisition_datetime: "2025-09-15T02:45:39Z",
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[117.0, 31.6], [117.6, 31.6], [117.6, 32.1], [117.0, 32.1], [117.0, 31.6]]],
    },
    metadata: {
      logical_dataset: "optical.sentinel-2.l2a",
      native_collection_id: "sentinel-2-l2a",
      collection_profile_id: "element84.earth-search.sentinel-2-l2a.scene.v1",
      identity_rule_id: "element84.earth-search.sentinel-2-l2a.identity.v1",
      filename_rule_id: "element84.earth-search.sentinel-2-l2a.product-uri.v1",
      resolver_policy_id: "element84.earth-search.sentinel-2-l2a.search-only.v1",
      provider_product_id: stacId,
      provider_product_key: itemId,
      source_kind: "sentinel2",
      product_kind: "optical_scene",
      properties: {
        product_type: "S2MSI2A",
        cloud_cover: 4.2,
      },
      assets: [
        {
          asset_key: "thumbnail",
          stable_asset_key: `ASSET::${itemId}::thumbnail`,
          role: "thumbnail",
          native_roles: ["thumbnail"],
          url: thumbUrl,
          file_name: `${stacId}_thumbnail.jpg`,
          media_type: "image/jpeg",
        },
      ],
    },
    payload: {},
    asset_access: null,
    execution_policy: null,
  };

  const scene = projectFrozenItemToScene(earthSearchItem);
  assert.ok(scene);
  assert.ok(scene.sar_product);
  assert.equal(scene.provider, "element84.earth-search");
  assert.deepEqual(opticalQuicklookPlan(scene), { kind: "mpc_tile", url: thumbUrl });
  const preview = deriveCatalogPreview(scene.sar_product);
  assert.equal(preview.kind, "candidate");
  if (preview.kind === "candidate") {
    assert.equal(preview.descriptor.render_profile_id, "earth-search.sentinel-2-l2a.thumbnail.v1");
    assert.equal(preview.url, thumbUrl);
  }
});
