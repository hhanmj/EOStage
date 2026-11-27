import assert from "node:assert/strict";
import test from "node:test";

import { projectApplicationSnapshot } from "./applicationServiceProjection.ts";
import {
  ALOS2_DATASET_LABEL,
  ALOS2_LOGICAL_DATASET,
  alos2ProductFromFrozenItem,
  isAlos2CatalogProduct,
} from "./asfAlos2Catalog.ts";
import { deriveCatalogPreview, projectCatalogRows } from "./catalogPreview.ts";

const LOGICAL_DATASET = "sar.alos-2.palsar-2.scansar.l1.1.v1";
const PROVIDER_ID = "asf.nasa.earthdata";
const IDENTITY_RULE_ID = "asf.alos-2.cmr-granule.v1";
const FILENAME_RULE_ID = "asf.alos-2.scansar-zip.v1";
const PRODUCT_ID = "G3463311647-ASF";
const ITEM_ID = `PROVIDER::${PROVIDER_ID}::COLLECTION::C3315903479-ASF::ITEM::${PRODUCT_ID}`;
const DISPLAY_NAME = "ALOS2478123150-230331-WBDR1.1__D";
const FILE_NAME = `${DISPLAY_NAME}.zip`;
const SIZE_BYTES = 60_018_405_388;
const MD5 = "0f558597e19d964f24ca3a4bff11696c";

function serviceSnapshot(options: {
  beamMode?: "WBS" | "WBD";
  stableHref?: string | null;
} = {}) {
  const beamMode = options.beamMode ?? "WBD";
  const footprint = {
    type: "Polygon",
    coordinates: [[[110, 30], [111, 30], [111, 31], [110, 31], [110, 30]]],
  };
  const stableHref = options.stableHref === undefined
    ? `https://datapool.asf.alaska.edu/ALOS2/${FILE_NAME}`
    : options.stableHref;
  const descriptor = {
    resolver_id: "earthdata_https",
    scheme: "earthdata_https",
    stable_href: stableHref,
    requires_credentials: true,
    credential_profile: "earthdata-default",
    resolver_parameters: {},
  };
  const candidate = {
    item_id: ITEM_ID,
    provider_id: PROVIDER_ID,
    display_name: DISPLAY_NAME,
    acquisition_datetime: "2023-03-31T19:20:46Z",
    footprint_geojson: footprint,
    metadata: {
      dataset: LOGICAL_DATASET,
      logical_dataset: LOGICAL_DATASET,
      native_collection_id: "C3315903479-ASF",
      collection_profile_id: "asf.alos-2.palsar-2.scansar.l1.1.v1",
      identity_rule_id: IDENTITY_RULE_ID,
      filename_rule_id: FILENAME_RULE_ID,
      delivery_model: "archive_first",
      resolver_policy_id: "asf.alos-2.earthdata-complete-zip.v1",
      provider_product_id: PRODUCT_ID,
      provider_product_key: ITEM_ID,
      product_family: "ALOS2_L1_PSR2",
      processing_level: "1.1",
      source_kind: "sar",
      product_kind: "sar_product",
      properties: {
        provider_product_key: ITEM_ID,
        provider_product_id: PRODUCT_ID,
        cmr_granule_concept_id: PRODUCT_ID,
        cmr_native_id: DISPLAY_NAME,
        producer_granule_id: "ALOS2478123150-230331",
        logical_dataset: LOGICAL_DATASET,
        collection_concept_id: "C3315903479-ASF",
        collection_short_name: "ALOS2_L1_PSR2",
        collection_version: "1",
        collection_profile_id: "asf.alos-2.palsar-2.scansar.l1.1.v1",
        identity_rule_id: IDENTITY_RULE_ID,
        filename_rule_id: FILENAME_RULE_ID,
        resolver_policy_id: "asf.alos-2.earthdata-complete-zip.v1",
        platform: "ALOS-2",
        instrument: "PALSAR-2",
        processing_level: "1.1",
        beam_mode: beamMode,
        polarizations: beamMode === "WBS" ? ["HH"] : ["HH", "HV"],
        acquisition_end_utc: "2023-03-31T19:21:36Z",
        preview_available: false,
        asset_integrity: {
          [FILE_NAME]: {
            file_name: FILE_NAME,
            size_bytes: SIZE_BYTES,
            algorithm: "MD5",
            value: MD5,
          },
        },
        asset_routes: {
          [FILE_NAME]: [{
            scheme: "https",
            locator: `https://datapool.asf.alaska.edu/ALOS2/${FILE_NAME}`,
          }],
        },
      },
      assets: [{
        asset_key: FILE_NAME,
        stable_asset_key: `${ITEM_ID}::ASSET::${FILE_NAME}`,
        access_descriptor: descriptor,
        role: "primary",
        native_roles: ["archive"],
        url: null,
        file_name: FILE_NAME,
        media_type: "application/zip",
        size_bytes: SIZE_BYTES,
      }],
    },
    payload: {},
    asset_access: {
      reference: {
        provider_id: PROVIDER_ID,
        collection_id: "C3315903479-ASF",
        provider_product_id: PRODUCT_ID,
        provider_product_key: ITEM_ID,
        canonical_product_key: null,
        asset_key: FILE_NAME,
        stable_asset_key: `${ITEM_ID}::ASSET::${FILE_NAME}`,
      },
      descriptor,
      role: "primary",
      file_name: FILE_NAME,
      expected_size_bytes: SIZE_BYTES,
      checksum_algorithm: "MD5",
      checksum_value: MD5,
    },
    execution_policy: {
      authority: "resolver_authoritative",
      policy_id: "asf.alos-2.earthdata-complete-zip.v1",
      collection_profile_id: "asf.alos-2.palsar-2.scansar.l1.1.v1",
    },
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
        batch_id: "alos2-batch",
        provider_id: PROVIDER_ID,
        created_at: "2026-07-29T00:00:00Z",
        aoi_name: "Fixture AOI",
        aoi_geojson: footprint,
        query: { logical_dataset: LOGICAL_DATASET },
        candidates: [candidate],
      },
      workbench: {
        batch_id: "alos2-batch",
        checked_item_ids: [ITEM_ID],
        page_index: 0,
        page_size: 24,
        revision: 1,
      },
      tasks: [],
      runtimes: [],
    },
  };
}

test("ALOS-2 frozen products keep exact row, Preview, and interactive footprint truth", () => {
  assert.equal(ALOS2_LOGICAL_DATASET, LOGICAL_DATASET);
  const projection = projectApplicationSnapshot(serviceSnapshot());
  const scene = projection?.workbench?.scenes[0];
  const product = scene?.sar_product;
  assert.ok(product);
  assert.equal(isAlos2CatalogProduct(product), true);
  assert.equal(product.dataset, ALOS2_LOGICAL_DATASET);
  assert.equal(product.platform, "ALOS-2");
  assert.equal(product.instrument, "PALSAR-2");
  assert.equal(product.processing_level, "1.1");
  assert.equal(product.beam_mode, "WBD");
  assert.equal(product.polarization, "HH+HV");
  assert.equal(product.file_name, FILE_NAME);
  assert.equal(product.file_size_remote, SIZE_BYTES);
  assert.equal(product.checksum, `md5:${MD5}`);
  assert.equal(product.assets?.length, 1);
  assert.equal(product.assets?.[0]?.role, "primary");
  assert.equal(product.assets?.[0]?.url, null);
  assert.deepEqual(scene.footprint_bbox, {
    west: 110,
    east: 111,
    south: 30,
    north: 31,
    crs: "EPSG:4326",
  });
  assert.deepEqual(scene.footprint_geojson, serviceSnapshot().restore.workbench_batch.candidates[0].footprint_geojson);

  const [row] = projectCatalogRows([product], new Set([ITEM_ID]), ITEM_ID);
  assert.equal(row.collection, ALOS2_DATASET_LABEL);
  assert.equal(row.platform, "ALOS-2");
  assert.equal(row.processing_level, "Level 1.1");
  assert.equal(row.file_size_label, "60.0 GB");
  assert.equal(row.asset_summary, "完整 ZIP");
  assert.deepEqual(row.domain_field_cells, [
    { label: "Direction", value: "—" },
    { label: "Beam", value: "WBD" },
    { label: "Polarization", value: "HH+HV" },
  ]);
  assert.equal(row.footprint_available, true);
  assert.equal(row.selected, true);
  assert.equal(row.active, true);

  const preview = deriveCatalogPreview(product);
  assert.equal(preview.kind, "unavailable");
  if (preview.kind === "unavailable") {
    assert.match(preview.reason, /ALOS-2.*Browse/);
    assert.doesNotMatch(preview.reason, /ZIP.*Preview.*可用/);
  }
});

test("ALOS-2 WBS frozen products preserve the HH-only capability matrix", () => {
  const projection = projectApplicationSnapshot(serviceSnapshot({ beamMode: "WBS" }));
  const product = projection?.workbench?.scenes[0]?.sar_product;
  assert.ok(product);
  assert.equal(product.beam_mode, "WBS");
  assert.equal(product.polarization, "HH");

  const [row] = projectCatalogRows([product], new Set(), null);
  assert.deepEqual(row.domain_field_cells, [
    { label: "Direction", value: "—" },
    { label: "Beam", value: "WBS" },
    { label: "Polarization", value: "HH" },
  ]);
});

test("ALOS-2 projection rejects a primary archive without a durable locator", () => {
  const snapshot = serviceSnapshot({ stableHref: null });
  const candidate = snapshot.restore.workbench_batch.candidates[0];
  assert.equal(alos2ProductFromFrozenItem(candidate), null);
});

test("ALOS-2 projection rejects drift across the frozen route and execution identity", () => {
  type Candidate = ReturnType<typeof serviceSnapshot>["restore"]["workbench_batch"]["candidates"][number];
  const cases: Array<[string, (candidate: Candidate) => void]> = [
    ["asset route", (candidate) => {
      candidate.metadata.properties.asset_routes[FILE_NAME][0].locator =
        "https://datapool.asf.alaska.edu/ALOS2/different.zip";
    }],
    ["frozen reference", (candidate) => {
      candidate.asset_access.reference.provider_id = "asf.catalog";
    }],
    ["frozen descriptor", (candidate) => {
      candidate.asset_access.descriptor.stable_href =
        "https://datapool.asf.alaska.edu/ALOS2/different.zip";
    }],
    ["execution policy", (candidate) => {
      candidate.execution_policy.policy_id = "asf.alos-2.unreviewed.v1";
    }],
    ["frozen size", (candidate) => {
      candidate.asset_access.expected_size_bytes += 1;
    }],
    ["frozen checksum algorithm", (candidate) => {
      candidate.asset_access.checksum_algorithm = "SHA256";
    }],
    ["frozen checksum value", (candidate) => {
      candidate.asset_access.checksum_value = "1f558597e19d964f24ca3a4bff11696c";
    }],
  ];

  for (const [label, mutate] of cases) {
    const snapshot = structuredClone(serviceSnapshot());
    const candidate = snapshot.restore.workbench_batch.candidates[0];
    mutate(candidate);
    assert.equal(alos2ProductFromFrozenItem(candidate), null, label);
    assert.equal(projectApplicationSnapshot(snapshot), null, `${label} application projection`);
  }
});

test("ALOS-2 projection rejects graph-consistent but non-canonical frozen identities", () => {
  type Candidate = ReturnType<typeof serviceSnapshot>["restore"]["workbench_batch"]["candidates"][number];
  const cases: Array<[string, (candidate: Candidate) => void]> = [
    ["archive basename", (candidate) => {
      const wrongHref = "https://datapool.asf.alaska.edu/ALOS2/different.zip";
      candidate.metadata.properties.asset_routes[FILE_NAME][0].locator = wrongHref;
      candidate.metadata.assets[0].access_descriptor.stable_href = wrongHref;
      candidate.asset_access.descriptor.stable_href = wrongHref;
    }],
    ["provider product key formula", (candidate) => {
      const wrongItemId = `${ITEM_ID}-OTHER`;
      const wrongStableKey = `${wrongItemId}::ASSET::${FILE_NAME}`;
      candidate.item_id = wrongItemId;
      candidate.metadata.provider_product_key = wrongItemId;
      candidate.metadata.properties.provider_product_key = wrongItemId;
      candidate.metadata.assets[0].stable_asset_key = wrongStableKey;
      candidate.asset_access.reference.provider_product_key = wrongItemId;
      candidate.asset_access.reference.stable_asset_key = wrongStableKey;
    }],
    ["stable asset key formula", (candidate) => {
      const wrongStableKey = `${ITEM_ID}::ASSET::different.zip`;
      candidate.metadata.assets[0].stable_asset_key = wrongStableKey;
      candidate.asset_access.reference.stable_asset_key = wrongStableKey;
    }],
    ["bounded route fields", (candidate) => {
      Reflect.set(candidate.metadata.properties.asset_routes[FILE_NAME][0], "unexpected", true);
    }],
    ["bounded integrity fields", (candidate) => {
      Reflect.set(candidate.metadata.properties.asset_integrity[FILE_NAME], "unexpected", true);
    }],
  ];

  for (const [label, mutate] of cases) {
    const candidate = structuredClone(serviceSnapshot().restore.workbench_batch.candidates[0]);
    mutate(candidate);
    assert.equal(alos2ProductFromFrozenItem(candidate), null, label);
  }
});

test("ALOS-2 projection rejects missing or non-polygon footprint truth", () => {
  const cases: Array<[string, unknown]> = [
    ["missing", null],
    ["wrong geometry", { type: "LineString", coordinates: [[110, 30], [111, 31]] }],
    ["empty polygon", { type: "Polygon", coordinates: [] }],
    ["out of range", { type: "Polygon", coordinates: [[[181, 30], [182, 31]]] }],
  ];

  for (const [label, footprint] of cases) {
    const snapshot = structuredClone(serviceSnapshot());
    Reflect.set(snapshot.restore.workbench_batch.candidates[0], "footprint_geojson", footprint);
    assert.equal(projectApplicationSnapshot(snapshot), null, label);
  }
});
