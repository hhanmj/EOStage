import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  projectApplicationSnapshot,
  projectFrozenItemToScene,
} from "./applicationServiceProjection.ts";
import {
  NISAR_DATASET_LABEL,
  NISAR_LOGICAL_DATASET,
  isNisarCatalogProduct,
  nisarProductFromFrozenItem,
  nisarProperties,
} from "./asfNisarCatalog.ts";
import {
  PreviewRequestCoordinator,
  deriveCatalogPreview,
  isReviewedPublicPreviewUrl,
  projectCatalogRows,
  type PreviewRuntimeState,
} from "./catalogPreview.ts";
import { catalogProductInfoText } from "./catalogProductInfo.ts";
import { MAP_TILE_Z_INDEX } from "./mapLayerOrder.ts";

const PROVIDER_ID = "asf.nasa.earthdata";
const COLLECTION_ID = "C2853145197-ASF";
const PROFILE_ID = "asf.nisar.l.rslc.provisional.v1";
const IDENTITY_RULE_ID = "asf.nisar.cmr-granule.v1";
const FILENAME_RULE_ID = "asf.nisar.rslc-hdf5.v1";
const RESOLVER_POLICY_ID = "asf.nisar.earthdata-complete-hdf5.v2";
const PRODUCT_ID = "G9900000001-ASF";
const DISPLAY_NAME = "NISAR_L1_PR_RSLC_SYNTHETIC_P05023_0001";
const ITEM_ID = `PROVIDER::${PROVIDER_ID}::COLLECTION::${COLLECTION_ID}::ITEM::${PRODUCT_ID}`;
const PRIMARY_KEY = "science-rslc-hdf5";
const PRIMARY_FILE = `${DISPLAY_NAME}.h5`;
const PRIMARY_SIZE = 12_000_000_000;
const SHA256 = "f2c78f09a49e5f1f304b19f930dfed10f197d235e383075e8377ddaa7c33d181";
const PRIMARY_URL = `https://nisar.asf.earthdatacloud.nasa.gov/products/${PRIMARY_FILE}`;
const BROWSE_KEY = "browse-main";
const BROWSE_FILE = `${DISPLAY_NAME}_LATLON.png`;
const BROWSE_URL = `https://nisar.asf.earthdatacloud.nasa.gov/browse/${BROWSE_FILE}`;
const THUMBNAIL_KEY = "browse-thumbnail";
const THUMBNAIL_FILE = `${DISPLAY_NAME}_thumbnail.png`;
const THUMBNAIL_URL = `https://nisar.asf.earthdatacloud.nasa.gov/browse/${THUMBNAIL_FILE}`;

function primaryGraphAsset() {
  return {
    asset_key: PRIMARY_KEY,
    stable_asset_key: `${ITEM_ID}::ASSET::${PRIMARY_KEY}`,
    role: "primary",
    native_roles: ["ARCHIVE:HDF5", "GET DATA", "GET DATA VIA DIRECT ACCESS"],
    file_name: PRIMARY_FILE,
    media_type: "application/x-hdf5",
    size_bytes: PRIMARY_SIZE,
    checksum: { algorithm: "SHA-256", value: SHA256 },
    routes: [{ scheme: "https", locator: PRIMARY_URL }],
    native_evidence: {
      archive_name: PRIMARY_FILE,
      product_match: true,
      url_types: ["GET DATA", "GET DATA VIA DIRECT ACCESS"],
      url_subtypes: [],
      formats: ["HDF5"],
    },
  };
}

function imageGraphAsset(kind: "preview" | "thumbnail") {
  const thumbnail = kind === "thumbnail";
  const assetKey = thumbnail ? THUMBNAIL_KEY : BROWSE_KEY;
  const fileName = thumbnail ? THUMBNAIL_FILE : BROWSE_FILE;
  const locator = thumbnail ? THUMBNAIL_URL : BROWSE_URL;
  return {
    asset_key: assetKey,
    stable_asset_key: `${ITEM_ID}::ASSET::${assetKey}`,
    role: kind,
    native_roles: ["ARCHIVE:PNG", "GET RELATED VISUALIZATION"],
    file_name: fileName,
    media_type: "image/png",
    size_bytes: null,
    checksum: null,
    routes: [{ scheme: "https", locator }],
    native_evidence: {
      archive_name: fileName,
      product_match: false,
      url_types: ["GET RELATED VISUALIZATION"],
      url_subtypes: [],
      formats: ["PNG"],
    },
  };
}

function serviceSnapshot(options: {
  frequencyA?: "HH" | "HH+HV" | null;
  frequencyB?: "HH" | "HH+HV" | "VV" | null;
  frameCoverage?: "FULL" | "PARTIAL";
  jointObservation?: boolean;
  browse?: boolean;
  thumbnail?: boolean;
} = {}) {
  const frequencyA = options.frequencyA === undefined ? "HH+HV" : options.frequencyA;
  const frequencyB = options.frequencyB === undefined ? "HH" : options.frequencyB;
  const frameCoverage = options.frameCoverage ?? "PARTIAL";
  const jointObservation = options.jointObservation ?? true;
  const includeBrowse = options.browse ?? true;
  const includeThumbnail = options.thumbnail ?? true;
  const footprint = {
    type: "Polygon",
    coordinates: [[[117, 30], [120, 30], [120, 32], [117, 32], [117, 30]]],
  };
  const properties: Record<string, unknown> = {
    provider_product_key: ITEM_ID,
    provider_product_id: PRODUCT_ID,
    cmr_granule_concept_id: PRODUCT_ID,
    producer_granule_id: DISPLAY_NAME,
    asf_product_id: DISPLAY_NAME,
    logical_dataset: NISAR_LOGICAL_DATASET,
    collection_concept_id: COLLECTION_ID,
    collection_short_name: "NISAR_L1_RSLC_PROVISIONAL_V1",
    collection_version: "1",
    collection_profile_id: PROFILE_ID,
    identity_rule_id: IDENTITY_RULE_ID,
    filename_rule_id: FILENAME_RULE_ID,
    resolver_policy_id: RESOLVER_POLICY_ID,
    processing_level: "RSLC",
    science_level: "L1",
    quality_status: "Provisional",
    production_configuration: "PR",
    band: "L",
    crid: "P05023",
    product_specification_version: "1.5.0",
    product_specification_profile_registry_version: 1,
    pge_version: "5.0.23",
    frame_coverage: frameCoverage,
    frame: 112,
    track: 37,
    relative_orbit: 37,
    flight_direction: "ASCENDING",
    look_direction: "RIGHT",
    range_bandwidth: "20+5",
    joint_observation: jointObservation,
    acquisition_end_utc: "2026-06-18T10:01:30Z",
    primary_asset_key: PRIMARY_KEY,
    preview_available: includeBrowse,
    full_product: true,
  };
  if (frequencyA !== null) properties.frequency_a_polarization = frequencyA;
  if (frequencyB !== null) properties.frequency_b_polarization = frequencyB;

  const assets = [primaryGraphAsset()];
  if (includeBrowse) assets.push(imageGraphAsset("preview"));
  if (includeThumbnail) assets.push(imageGraphAsset("thumbnail"));
  const descriptor = {
    resolver_id: "earthdata_https",
    scheme: "earthdata_https",
    stable_href: PRIMARY_URL,
    requires_credentials: true,
    credential_profile: "earthdata-default",
    resolver_parameters: {},
  };
  const candidate = {
    item_id: ITEM_ID,
    provider_id: PROVIDER_ID,
    display_name: DISPLAY_NAME,
    acquisition_datetime: "2026-06-18T10:00:00Z",
    footprint_geojson: footprint,
    metadata: {
      dataset: NISAR_LOGICAL_DATASET,
      logical_dataset: NISAR_LOGICAL_DATASET,
      native_collection_id: COLLECTION_ID,
      collection_profile_id: PROFILE_ID,
      identity_rule_id: IDENTITY_RULE_ID,
      filename_rule_id: FILENAME_RULE_ID,
      delivery_model: "archive_first",
      resolver_policy_id: RESOLVER_POLICY_ID,
      provider_product_id: PRODUCT_ID,
      provider_product_key: ITEM_ID,
      product_family: "NISAR_L1_RSLC_PROVISIONAL_V1",
      processing_level: "RSLC",
      quality_status: "Provisional",
      source_kind: "sar",
      product_kind: "sar_product",
      properties,
      assets,
    },
    payload: {},
    asset_access: {
      reference: {
        provider_id: PROVIDER_ID,
        collection_id: COLLECTION_ID,
        provider_product_id: PRODUCT_ID,
        provider_product_key: ITEM_ID,
        canonical_product_key: null,
        asset_key: PRIMARY_KEY,
        stable_asset_key: `${ITEM_ID}::ASSET::${PRIMARY_KEY}`,
      },
      descriptor,
      role: "primary",
      file_name: PRIMARY_FILE,
      expected_size_bytes: PRIMARY_SIZE,
      checksum_algorithm: "SHA256",
      checksum_value: SHA256,
    },
    execution_policy: {
      authority: "resolver_authoritative",
      policy_id: RESOLVER_POLICY_ID,
      collection_profile_id: PROFILE_ID,
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
        batch_id: "nisar-batch",
        provider_id: PROVIDER_ID,
        created_at: "2026-07-29T00:00:00Z",
        aoi_name: "NISAR fixture AOI",
        aoi_geojson: footprint,
        query: { logical_dataset: NISAR_LOGICAL_DATASET },
        candidates: [candidate],
      },
      workbench: {
        batch_id: "nisar-batch",
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

test("NISAR frozen projection preserves Provisional RSLC science, complete HDF5, and footprint truth", () => {
  const projection = projectApplicationSnapshot(serviceSnapshot());
  const scene = projection?.workbench?.scenes[0];
  const product = scene?.sar_product;
  assert.ok(product);
  assert.equal(isNisarCatalogProduct(product), true);
  assert.equal(product.dataset, NISAR_LOGICAL_DATASET);
  assert.equal(product.platform, "NISAR");
  assert.equal(product.instrument, "L-SAR");
  assert.equal(product.processing_level, "RSLC");
  assert.equal(product.product_family, "NISAR_L1_RSLC_PROVISIONAL_V1");
  assert.equal(product.file_name, PRIMARY_FILE);
  assert.equal(product.file_size_remote, PRIMARY_SIZE);
  assert.equal(product.checksum, `sha256:${SHA256}`);
  assert.equal(product.relative_orbit, 37);
  assert.equal(product.path, 37);
  assert.equal(product.frame, 112);
  assert.equal(product.assets?.find((asset) => asset.role === "primary")?.url, null);
  assert.equal(product.assets?.find((asset) => asset.role === "preview")?.url, BROWSE_URL);
  assert.equal(product.assets?.find((asset) => asset.role === "thumbnail")?.url, THUMBNAIL_URL);
  assert.deepEqual(scene.footprint_bbox, {
    west: 117,
    east: 120,
    south: 30,
    north: 32,
    crs: "EPSG:4326",
  });
  assert.deepEqual(scene.footprint_geojson, serviceSnapshot().restore.workbench_batch.candidates[0].footprint_geojson);

  const properties = nisarProperties(product);
  assert.ok(properties);
  assert.equal(properties.quality_status, "Provisional");
  assert.equal(properties.frame_coverage, "PARTIAL");
  assert.equal(properties.frequency_a_polarization, "HH+HV");
  assert.equal(properties.frequency_b_polarization, "HH");
  assert.equal(properties.band, "L");
  assert.equal(properties.range_bandwidth, "20+5");
  assert.equal(properties.joint_observation, true);

  const [row] = projectCatalogRows([product], new Set([ITEM_ID]), ITEM_ID);
  assert.equal(row.collection, NISAR_DATASET_LABEL);
  assert.equal(row.processing_level, "RSLC · Provisional");
  assert.equal(row.file_size_label, "12.0 GB");
  assert.equal(row.asset_summary, "完整 HDF5");
  assert.deepEqual(row.domain_field_cells, [
    { label: "Direction", value: "ASCENDING" },
    { label: "Coverage", value: "PARTIAL" },
    { label: "Freq A", value: "HH+HV" },
    { label: "Freq B (L)", value: "HH" },
    { label: "Range BW", value: "20+5" },
    { label: "Joint L/S", value: "Yes · metadata only" },
  ]);
  assert.equal(row.selected, true);
  assert.equal(row.active, true);

  const preview = deriveCatalogPreview(product);
  assert.equal(preview.kind, "candidate");
  if (preview.kind === "candidate") {
    assert.equal(preview.asset.role, "preview");
    assert.equal(preview.url, BROWSE_URL);
    assert.equal(preview.descriptor.preview_kind, "quicklook");
    assert.equal(
      preview.descriptor.render_profile_id,
      "asf.nisar.complete-rslc.main-latlon.v1",
    );
    assert.deepEqual(preview.descriptor.source_asset_keys, [`${ITEM_ID}::ASSET::${BROWSE_KEY}`]);
  }

  const info = catalogProductInfoText(product, row);
  assert.match(info, /^processing_level: RSLC/m);
  assert.match(info, /^quality_status: Provisional/m);
  assert.match(info, /^frame_coverage: PARTIAL/m);
  assert.match(info, /^frequency_a_polarization: HH\+HV/m);
  assert.match(info, /^frequency_b_polarization_l_band: HH/m);
  assert.match(info, /^range_bandwidth: 20\+5/m);
  assert.match(info, /^joint_l_s_observation: true \(metadata only; not an ASF S-band product\)/m);
  assert.match(info, /^primary_hdf5_file: NISAR_L1_PR_RSLC_SYNTHETIC_P05023_0001\.h5/m);
  assert.match(info, /^primary_hdf5_size_bytes: 12000000000/m);
  assert.match(info, new RegExp(`^sha256_checksum: ${SHA256}$`, "m"));
  assert.doesNotMatch(info, /^download_url:/m);
});

test("task-page projection preserves NISAR source identity while accepting Store-owned subset execution annotations", () => {
  const snapshot = structuredClone(serviceSnapshot());
  const candidate = snapshot.restore.workbench_batch.candidates[0];
  Object.assign(candidate.metadata, {
    delivery_kind: "remote_derived_subset_v2",
    source_item_id: ITEM_ID,
    nisar_subset_prepared_intent_v2_envelope: { schema_version: 2, sha256: "a".repeat(64) },
    nisar_subset_plan_v2_envelope: { schema_version: 2, sha256: "b".repeat(64) },
  });

  assert.equal(nisarProductFromFrozenItem(candidate), null, "catalogue validator remains closed");
  const scene = projectFrozenItemToScene(candidate);
  assert.equal(scene?.product_key, ITEM_ID);
  assert.equal(scene?.file_name, PRIMARY_FILE);
  assert.equal(scene?.native_metadata?.delivery_kind, "remote_derived_subset_v2");
  assert.deepEqual(scene?.native_metadata?.nisar_subset_plan_v2_envelope, {
    schema_version: 2,
    sha256: "b".repeat(64),
  });
});

test("NISAR frozen projection accepts production metadata mirrors only when they match scientific properties", () => {
  const snapshot = structuredClone(serviceSnapshot());
  const candidate = snapshot.restore.workbench_batch.candidates[0];
  const properties = candidate.metadata.properties;
  Object.assign(candidate.metadata, {
    band: properties.band,
    science_level: properties.science_level,
    production_configuration: properties.production_configuration,
    crid: properties.crid,
    product_specification_version: properties.product_specification_version,
    frequency_a_polarization: properties.frequency_a_polarization,
    frequency_b_polarization: properties.frequency_b_polarization,
  });

  assert.ok(nisarProductFromFrozenItem(candidate));
  Reflect.set(candidate.metadata, "band", "S");
  assert.equal(nisarProductFromFrozenItem(candidate), null);
});

test("Frequency B-only stays L-band and never creates an S-band action or inferred Frequency A", () => {
  const projection = projectApplicationSnapshot(serviceSnapshot({
    frequencyA: null,
    frequencyB: "VV",
    frameCoverage: "FULL",
    jointObservation: false,
  }));
  const product = projection?.workbench?.scenes[0]?.sar_product;
  assert.ok(product);
  const properties = nisarProperties(product);
  assert.ok(properties);
  assert.equal(Object.hasOwn(properties, "frequency_a_polarization"), false);
  assert.equal(properties.frequency_b_polarization, "VV");
  assert.equal(properties.band, "L");
  assert.match(product.polarization ?? "", /Frequency B VV \(L-band\)/);
  assert.doesNotMatch(product.polarization ?? "", /S-band/);

  const [row] = projectCatalogRows([product], new Set(), null);
  assert.deepEqual(row.domain_field_cells.slice(0, 4), [
    { label: "Direction", value: "ASCENDING" },
    { label: "Coverage", value: "FULL" },
    { label: "Freq A", value: "—" },
    { label: "Freq B (L)", value: "VV" },
  ]);
  const info = catalogProductInfoText(product, row);
  assert.doesNotMatch(info, /^frequency_a_polarization:/m);
  assert.match(info, /^frequency_b_polarization_l_band: VV/m);
  assert.match(info, /^joint_l_s_observation: false \(metadata only; not an ASF S-band product\)/m);
});

test("active NISAR requires one Provider Preview and never falls back to thumbnail or HDF5", async () => {
  const projection = projectApplicationSnapshot(serviceSnapshot({ browse: false, thumbnail: true }));
  const product = projection?.workbench?.scenes[0]?.sar_product;
  assert.ok(product);
  assert.equal(product.assets?.some((asset) => asset.role === "thumbnail"), true);
  assert.equal(product.assets?.some((asset) => asset.role === "primary"), true);

  const source = deriveCatalogPreview(product);
  assert.equal(source.kind, "unavailable");
  if (source.kind === "unavailable") {
    assert.match(source.reason, /NISAR.*Provider Preview/i);
    assert.match(source.reason, /thumbnail.*HDF5/i);
  }
  let loads = 0;
  const published: PreviewRuntimeState[] = [];
  const coordinator = new PreviewRequestCoordinator((state) => published.push(state));
  const state = await coordinator.request(source, async () => {
    loads += 1;
  });
  assert.equal(loads, 0);
  assert.equal(state.status, "unavailable");
  assert.equal(state.url, null);
  assert.deepEqual(published.map((entry) => entry.status), ["unavailable"]);
});

test("NISAR keeps bounded role integrity and consumes the unique Provider Preview from a richer graph", () => {
  const snapshot = structuredClone(serviceSnapshot());
  const candidate = snapshot.restore.workbench_batch.candidates[0];
  const browse = candidate.metadata.assets.find((asset) => asset.role === "preview");
  assert.ok(browse);
  const mainKey = "browse-latlon";
  const mainFile = `${DISPLAY_NAME}_LATLON.png`;
  const mainUrl = `https://nisar.asf.earthdatacloud.nasa.gov/browse/${mainFile}`;
  browse.asset_key = mainKey;
  browse.stable_asset_key = `${ITEM_ID}::ASSET::${mainKey}`;
  browse.file_name = mainFile;
  browse.native_evidence.archive_name = mainFile;
  browse.routes[0].locator = mainUrl;
  browse.size_bytes = 388_689;
  browse.checksum = {
    algorithm: "MD5",
    value: "9d30d6438dc2ab9e242cb8756a2205fa",
  };

  const additionalBrowse = structuredClone(browse);
  additionalBrowse.role = "browse";
  const additionalKey = "browse-native";
  const additionalFile = `${DISPLAY_NAME}_NATIVE.png`;
  additionalBrowse.asset_key = additionalKey;
  additionalBrowse.stable_asset_key = `${ITEM_ID}::ASSET::${additionalKey}`;
  additionalBrowse.file_name = additionalFile;
  additionalBrowse.native_evidence.archive_name = additionalFile;
  additionalBrowse.routes[0].locator =
    `https://nisar.asf.earthdatacloud.nasa.gov/browse/${additionalFile}`;
  candidate.metadata.assets.splice(2, 0, additionalBrowse);

  const product = nisarProductFromFrozenItem(candidate);
  assert.ok(product);
  assert.equal(product.assets?.filter((asset) => asset.role === "preview").length, 1);
  assert.equal(product.assets?.filter((asset) => asset.role === "browse").length, 1);
  assert.equal(product.assets?.find((asset) => asset.asset_key === mainKey)?.size_bytes, 388_689);
  const preview = deriveCatalogPreview(product);
  assert.equal(preview.kind, "candidate");
  if (preview.kind === "candidate") {
    assert.equal(preview.asset.asset_key, mainKey);
    assert.equal(preview.url, mainUrl);
  }
});

test("NISAR projection rejects collection, scope, frequency, and primary integrity drift", () => {
  type Candidate = ReturnType<typeof serviceSnapshot>["restore"]["workbench_batch"]["candidates"][number];
  const cases: Array<[string, (candidate: Candidate) => void]> = [
    ["Beta quality", (candidate) => {
      candidate.metadata.properties.quality_status = "Beta";
    }],
    ["S-band scope", (candidate) => {
      candidate.metadata.properties.band = "S";
    }],
    ["missing CRID", (candidate) => {
      Reflect.deleteProperty(candidate.metadata.properties, "crid");
    }],
    ["unknown CRID", (candidate) => {
      candidate.metadata.properties.crid = "P99999";
    }],
    ["CRID/specification mismatch", (candidate) => {
      candidate.metadata.properties.product_specification_version = "1.2.1";
    }],
    ["missing product specification profile registry version", (candidate) => {
      Reflect.deleteProperty(
        candidate.metadata.properties,
        "product_specification_profile_registry_version",
      );
    }],
    ["product specification profile registry version mismatch", (candidate) => {
      candidate.metadata.properties.product_specification_profile_registry_version = 2;
    }],
    ["frequency-less", (candidate) => {
      Reflect.deleteProperty(candidate.metadata.properties, "frequency_a_polarization");
      Reflect.deleteProperty(candidate.metadata.properties, "frequency_b_polarization");
    }],
    ["explicit null frequency", (candidate) => {
      candidate.metadata.properties.frequency_a_polarization = null;
    }],
    ["primary filename", (candidate) => {
      candidate.asset_access.file_name = `${DISPLAY_NAME}_different.h5`;
    }],
    ["primary bytes", (candidate) => {
      candidate.asset_access.expected_size_bytes += 1;
    }],
    ["primary checksum", (candidate) => {
      candidate.asset_access.checksum_value = `0${SHA256.slice(1)}`;
    }],
    ["execution policy", (candidate) => {
      candidate.execution_policy.policy_id = "asf.nisar.unreviewed.v1";
    }],
    ["unreviewed primary host", (candidate) => {
      const locator = `https://evil.example/${PRIMARY_FILE}`;
      candidate.metadata.assets[0].routes[0].locator = locator;
      candidate.asset_access.descriptor.stable_href = locator;
    }],
    ["Browse extension evidence", (candidate) => {
      const browse = candidate.metadata.assets.find((asset) => asset.role === "preview");
      assert.ok(browse);
      const fileName = `${DISPLAY_NAME}_BROWSE.txt`;
      browse.file_name = fileName;
      browse.media_type = "image/jpeg";
      browse.native_roles[0] = "ARCHIVE:JPG";
      browse.native_evidence.archive_name = fileName;
      browse.native_evidence.formats = ["JPG"];
      browse.routes[0].locator =
        `https://nisar.asf.earthdatacloud.nasa.gov/browse/${fileName}`;
    }],
    ["S-band asset evidence", (candidate) => {
      candidate.metadata.assets[0].native_evidence.url_types.push("S-BAND PRODUCT");
    }],
  ];

  for (const [label, mutate] of cases) {
    const snapshot = structuredClone(serviceSnapshot());
    const candidate = snapshot.restore.workbench_batch.candidates[0];
    mutate(candidate);
    assert.equal(nisarProductFromFrozenItem(candidate), null, label);
    assert.equal(projectApplicationSnapshot(snapshot), null, `${label} application projection`);
  }
});

test("NISAR Preview host is exact while Workbench, Preview state, and map z-order stay shared", () => {
  assert.equal(isReviewedPublicPreviewUrl(BROWSE_URL), true);
  assert.equal(
    isReviewedPublicPreviewUrl(`https://nisar.asf.earthdatacloud.nasa.gov.evil.example/${BROWSE_FILE}`),
    false,
  );
  assert.equal(
    isReviewedPublicPreviewUrl(`http://nisar.asf.earthdatacloud.nasa.gov/${BROWSE_FILE}`),
    false,
  );
  assert.equal(
    isReviewedPublicPreviewUrl(`https://nisar.asf.earthdatacloud.nasa.gov:443/${BROWSE_FILE}`),
    false,
  );
  assert.equal(
    isReviewedPublicPreviewUrl(`https://user@nisar.asf.earthdatacloud.nasa.gov/${BROWSE_FILE}`),
    false,
  );
  assert.equal(isReviewedPublicPreviewUrl(`${BROWSE_URL}?token=placeholder`), false);
  assert.equal(isReviewedPublicPreviewUrl(`${BROWSE_URL}#preview`), false);

  const mapSource = readFileSync(new URL("../components/WorkbenchMap.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(mapSource, /NISAR.*ImageOverlay|nisar.*render_profile/i);
  assert.ok(MAP_TILE_Z_INDEX.base < MAP_TILE_Z_INDEX.imageAnnotation);
  assert.ok(MAP_TILE_Z_INDEX.imageAnnotation < MAP_TILE_Z_INDEX.vectorAnnotation);
  assert.ok(MAP_TILE_Z_INDEX.vectorAnnotation < MAP_TILE_Z_INDEX.preview);
});
