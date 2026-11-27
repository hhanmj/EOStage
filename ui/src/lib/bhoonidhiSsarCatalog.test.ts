import assert from "node:assert/strict";
import test from "node:test";

import type { ProviderAsset, SarProduct } from "./generated/v3-contracts.ts";
import {
  BHOONIDHI_COLLECTION_ID,
  BHOONIDHI_CREDENTIAL_PROFILE,
  BHOONIDHI_DOWNLOAD_URL,
  BHOONIDHI_FILENAME_RULE_ID,
  BHOONIDHI_IDENTITY_RULE_ID,
  BHOONIDHI_LOGICAL_DATASET,
  BHOONIDHI_PROFILE_ID,
  BHOONIDHI_PUBLIC_ASSET_RESOLVER_ID,
  BHOONIDHI_PROVIDER_ID,
  BHOONIDHI_RESOLVER_POLICY_ID,
  BHOONIDHI_SSAR_PREVIEW_PROFILE,
  bhoonidhiSsarSubsetSelection,
  isBhoonidhiSsarCatalogProduct,
} from "./bhoonidhiSsarCatalog.ts";
import {
  deriveCatalogPreview,
  projectCatalogRows,
} from "./catalogPreview.ts";
import {
  buildSceneFullCopyText,
  buildSceneRowMetaDisplay,
  catalogProductInfoText,
} from "./catalogProductInfo.ts";
import { projectFrozenItemToScene } from "./applicationServiceProjection.ts";

const ITEM_ID = "NISAR_S1_PR_RSLC_029_077_D_086_2500_CRNA_A_20260830T132132_20260830T132141_P00500_N_P_I_001";
const THUMBNAIL_URL = "https://bhoonidhi.nrsc.gov.in/stac/PRODUCTJPGS/NISAR/SSAR/2026/AUG/30/fixture.jpg";

function asset(role: ProviderAsset["role"], url: string): ProviderAsset {
  return {
    asset_key: role,
    stable_asset_key: `${BHOONIDHI_PROVIDER_ID}:${BHOONIDHI_COLLECTION_ID}:${ITEM_ID}:${role}`,
    access_descriptor: {
      resolver_id: "isro.nrsc.bhoonidhi.public-asset.v1",
      scheme: "public_https",
      stable_href: url,
      requires_credentials: false,
      credential_profile: null,
      resolver_parameters: {},
    },
    role,
    native_roles: [role],
    url,
    file_name: url.split("/").at(-1) ?? null,
    media_type: role === "thumbnail" ? "image/jpeg" : "application/json",
    size_bytes: null,
  };
}

function product(): SarProduct {
  const properties = {
    logical_dataset: BHOONIDHI_LOGICAL_DATASET,
    native_collection_id: BHOONIDHI_COLLECTION_ID,
    collection_profile_id: BHOONIDHI_PROFILE_ID,
    provider_collection: BHOONIDHI_COLLECTION_ID,
    ID: ITEM_ID,
    ProductCode: "NISAR_S1_PR_RSLC",
    product_code: "NISAR_S1_PR_RSLC",
    SoftwareVersion: "1.4.00",
    software_version: "1.4.00",
    Producer: "NRSC",
    producer: "NRSC",
    Platform: "NISAR",
    Instrument: "SSAR",
    Scene_Identifier: "2026242132100_77_86",
    scene_identifier: "2026242132100_77_86",
    provider_product_id: ITEM_ID,
    provider_product_key: ITEM_ID,
    processing_level: "RSLC",
    platform: "NISAR",
    instrument: "SSAR",
    Polarization: "CP",
    polarization: "CP",
    Mode: "W",
    mode: "W",
    Coverage: "P",
    AcquistionMode: "J",
    acquisition_mode: "J",
    Online: "Y",
    Frame: 86,
    Track: 77,
    Node: "Descending",
  };
  return {
    product_key: `PROVIDER::${BHOONIDHI_PROVIDER_ID}::COLLECTION::${BHOONIDHI_COLLECTION_ID}::ITEM::${ITEM_ID}`,
    provider_product_key: ITEM_ID,
    provider_product_id: ITEM_ID,
    canonical_product_key: null,
    identity_rule_id: "isro.nrsc.bhoonidhi.nisar.s.ssar.rslc.identity.v1",
    product_id: ITEM_ID,
    scene_id: "2026242132100_77_86",
    provider: BHOONIDHI_PROVIDER_ID,
    dataset: BHOONIDHI_LOGICAL_DATASET,
    platform: "NISAR",
    instrument: "SSAR",
    processing_level: "RSLC",
    product_family: BHOONIDHI_COLLECTION_ID,
    beam_mode: null,
    beam_swath: null,
    polarization: "CP",
    flight_direction: "DESCENDING",
    look_direction: "LEFT",
    relative_orbit: 77,
    absolute_orbit: 5712,
    frame: 86,
    asf_frame: null,
    path: 77,
    acquisition_start: "2026-08-30T13:21:32Z",
    acquisition_stop: null,
    footprint_geojson: {
      type: "Polygon",
      coordinates: [[[74, 5], [77, 5], [77, 6], [74, 5]]],
    },
    footprint_bbox: { west: 74, east: 77, south: 5, north: 6, crs: "EPSG:4326" },
    download_url: null,
    file_name: `${ITEM_ID}.h5`,
    file_size_remote: null,
    checksum: null,
    local_path: null,
    native_metadata: { ...properties, properties },
    burst: null,
    assets: [asset("metadata", "https://bhoonidhi.nrsc.gov.in/stac/fixture.met"), asset("thumbnail", THUMBNAIL_URL)],
  };
}

test("Bhoonidhi S-RSLC uses the public thumbnail preview and preserves CP/W as provider values", () => {
  const ssar = product();
  assert.equal(isBhoonidhiSsarCatalogProduct(ssar), true);
  const preview = deriveCatalogPreview(ssar);
  assert.equal(preview.kind, "candidate");
  if (preview.kind !== "candidate") return;
  assert.equal(preview.asset.role, "thumbnail");
  assert.equal(preview.url, THUMBNAIL_URL);
  assert.equal(preview.descriptor.render_profile_id, BHOONIDHI_SSAR_PREVIEW_PROFILE);
  assert.equal(preview.descriptor.credential_requirement, "none");

  const [row] = projectCatalogRows([ssar], new Set([ssar.product_key]), ssar.product_key);
  assert.equal(row.collection, "NISAR S-band SSAR RSLC");
  assert.equal(row.asset_summary, "完整 HDF5");
  assert.deepEqual(row.domain_field_cells, [
    { label: "Direction", value: "DESCENDING" },
    { label: "Coverage", value: "P" },
    { label: "Polarization", value: "CP" },
    { label: "Mode", value: "W" },
    { label: "Acquisition", value: "J" },
    { label: "Online", value: "Y" },
  ]);
  assert.equal(row.domain_field_cells.some((cell) => cell.label.includes("Freq")), false);

  const info = catalogProductInfoText(ssar, row);
  assert.match(info, /^provider_polarization: CP$/m);
  assert.match(info, /^provider_mode: W$/m);
  assert.match(info, /^coverage: P$/m);
  assert.match(info, /^preview: thumbnail JPEG \(仅显示\)$/m);
  assert.doesNotMatch(info, /frequency_[ab]|joint_l_s|ASF/);
});

test("Bhoonidhi S-RSLC exposes the reviewed SSAR science channels for subset planning", () => {
  const ssar = product();
  assert.deepEqual(bhoonidhiSsarSubsetSelection(ssar), {
    frequency: "A",
    polarizations: ["RH", "RV"],
  });
  const properties = ssar.native_metadata.properties as Record<string, unknown>;
  assert.equal(bhoonidhiSsarSubsetSelection({
    ...ssar,
    native_metadata: {
      ...ssar.native_metadata,
      Polarization: "DH",
      properties: { ...properties, Polarization: "DH" },
    },
  }), null);
});

test("Bhoonidhi S-RSLC identity fails closed when collection/profile drifts", () => {
  const ssar = product();
  assert.equal(isBhoonidhiSsarCatalogProduct({
    ...ssar,
    native_metadata: {
      ...ssar.native_metadata,
      collection_profile_id: "other",
      properties: { ...(ssar.native_metadata.properties as Record<string, unknown>), collection_profile_id: "other" },
    },
  }), false);
  assert.equal(isBhoonidhiSsarCatalogProduct({ ...ssar, provider: "asf.nasa.earthdata" }), false);
});

test("Bhoonidhi S-RSLC frozen snapshots retain the primary resolver and public metadata assets", () => {
  const ssar = product();
  const itemId = ssar.product_key;
  const primaryFileName = ITEM_ID + ".h5";
  const item = {
    item_id: itemId,
    provider_id: BHOONIDHI_PROVIDER_ID,
    display_name: ssar.scene_id!,
    acquisition_datetime: ssar.acquisition_start,
    footprint_geojson: ssar.footprint_geojson,
    metadata: {
      dataset: BHOONIDHI_LOGICAL_DATASET,
      logical_dataset: BHOONIDHI_LOGICAL_DATASET,
      native_collection_id: BHOONIDHI_COLLECTION_ID,
      collection_profile_id: BHOONIDHI_PROFILE_ID,
      identity_rule_id: BHOONIDHI_IDENTITY_RULE_ID,
      filename_rule_id: BHOONIDHI_FILENAME_RULE_ID,
      delivery_model: "archive_first",
      resolver_policy_id: BHOONIDHI_RESOLVER_POLICY_ID,
      provider_product_id: ITEM_ID,
      provider_product_key: itemId,
      product_family: BHOONIDHI_COLLECTION_ID,
      processing_level: "RSLC",
      band: "S",
      source_kind: "sar",
      product_kind: "sar_slc",
      primary_asset_key: "data",
      primary_file_name: primaryFileName,
      footprint_bbox: ssar.footprint_bbox,
      properties: ssar.native_metadata.properties,
      assets: ssar.assets,
    },
    payload: {},
    asset_access: {
      reference: {
        provider_id: BHOONIDHI_PROVIDER_ID,
        collection_id: BHOONIDHI_COLLECTION_ID,
        provider_product_id: ITEM_ID,
        provider_product_key: itemId,
        canonical_product_key: null,
        asset_key: "data",
        stable_asset_key: itemId + "::ASSET::data",
      },
      descriptor: {
        resolver_id: BHOONIDHI_RESOLVER_POLICY_ID,
        scheme: "oauth_bearer_https",
        stable_href: BHOONIDHI_DOWNLOAD_URL,
        requires_credentials: true,
        credential_profile: BHOONIDHI_CREDENTIAL_PROFILE,
        resolver_parameters: {},
      },
      role: "primary",
      file_name: primaryFileName,
      expected_size_bytes: null,
      checksum_algorithm: null,
      checksum_value: null,
    },
    execution_policy: {
      authority: "resolver_authoritative",
      policy_id: BHOONIDHI_RESOLVER_POLICY_ID,
      collection_profile_id: BHOONIDHI_PROFILE_ID,
    },
  };

  const scene = projectFrozenItemToScene(item);
  assert.ok(scene);
  assert.equal(isBhoonidhiSsarCatalogProduct(scene.sar_product), true);
  assert.equal(scene.sar_product.relative_orbit, 77);
  assert.equal(scene.sar_product.path, 77);
  assert.equal(scene.sar_product.frame, 86);
  assert.equal(scene.sar_product.beam_mode, "W");
  assert.equal(scene.sar_product.polarization, "CP");
  assert.equal(scene.sar_product.flight_direction, "DESCENDING");
  assert.equal(scene.sar_product.assets?.length, 3);
  const primary = scene.sar_product.assets?.find((asset) => asset.role === "primary");
  assert.equal(primary?.access_descriptor?.resolver_id, BHOONIDHI_RESOLVER_POLICY_ID);
  const thumbnail = scene.sar_product.assets?.find((asset) => asset.role === "thumbnail");
  assert.equal(thumbnail?.access_descriptor?.resolver_id, BHOONIDHI_PUBLIC_ASSET_RESOLVER_ID);
  const preview = deriveCatalogPreview(scene.sar_product);
  assert.equal(preview.kind, "candidate");
  if (preview.kind === "candidate") assert.equal(preview.url, THUMBNAIL_URL);
});

test("catalogProductInfo hardening: antimeridian bbox center wrap, placeholder 0 track fallback, and sar_product NISAR tier", () => {
  const antimeridianText = buildSceneFullCopyText({
    scene_id: "S1A_IW_SLC_ANTIMERIDIAN",
    file_name: "S1A_IW_SLC_ANTIMERIDIAN",
    platform: "Sentinel-1A",
    product_type: "SLC",
    beam_mode: "IW",
    polarization: "VV",
    acquisition_datetime: "2026-10-01T00:00:00Z",
    absolute_orbit: null,
    relative_orbit: 0,
    path: 0,
    frame: null,
    orbit_direction: "ASCENDING",
    footprint_bbox: { west: 179, south: 50, east: -179, north: 52, crs: "EPSG:4326" },
    has_url: true,
    sar_product: {
      provider_id: "asf",
      dataset: "SENTINEL-1",
      product_id: "S1A_IW_SLC_ANTIMERIDIAN",
      scene_id: "S1A_IW_SLC_ANTIMERIDIAN",
      file_name: "S1A_IW_SLC_ANTIMERIDIAN.zip",
      platform: "Sentinel-1A",
      product_family: "SLC",
      relative_orbit: 88,
      assets: [],
      native_metadata: {},
    },
  } as unknown as Parameters<typeof buildSceneFullCopyText>[0]);
  assert.ok(antimeridianText.includes("180.0000°E"), antimeridianText);
  assert.ok(antimeridianText.includes("Track: 88"), antimeridianText);

  const meta = buildSceneRowMetaDisplay({
    scene_id: "NISAR_L2_GCOV_BETA_001",
    file_name: "NISAR_L2_GCOV_BETA_001",
    platform: "NISAR",
    product_type: "GCOV",
    beam_mode: "",
    polarization: "HH+HV",
    acquisition_datetime: "2026-10-01T00:00:00Z",
    absolute_orbit: null,
    relative_orbit: null,
    path: null,
    frame: null,
    orbit_direction: "",
    has_url: true,
    sar_product: {
      provider_id: "asf",
      dataset: "NISAR",
      product_id: "NISAR_L2_GCOV_BETA_001",
      scene_id: "NISAR_L2_GCOV_BETA_001",
      file_name: "NISAR_L2_GCOV_BETA_001.h5",
      platform: "NISAR",
      product_family: "GCOV",
      assets: [],
      native_metadata: { collection_short_name: "NISAR_L2_GCOV_BETA_V2" },
    },
  } as unknown as Parameters<typeof buildSceneRowMetaDisplay>[0]);
  assert.equal(meta.tier, "provisional");
});
