// Frozen catalogue evidence used only by append workspace interaction tests.
const PROVIDER_ID = "asf.nasa.earthdata";
const COLLECTION_ID = "C2853145197-ASF";
const LOGICAL_DATASET = "sar.nisar.l.rslc.provisional.v1";
const PROFILE_ID = "asf.nisar.l.rslc.provisional.v1";
const IDENTITY_RULE_ID = "asf.nisar.cmr-granule.v1";
const FILENAME_RULE_ID = "asf.nisar.rslc-hdf5.v1";
const RESOLVER_POLICY_ID = "asf.nisar.earthdata-complete-hdf5.v2";
export const geometry = {
  type: "Polygon",
  coordinates: [[[110, 30], [113, 30], [113, 33], [110, 33], [110, 30]]],
};
const readiness = {
  contract: "application.v1",
  status: "ready",
  store_schema_version: 11,
};

export function frozenNisarItem(index: number) {
  const minute = String(index % 60).padStart(2, "0");
  const productId = `G990000000${index}-ASF`;
  const displayName = `NISAR_L1_PR_RSLC_TEST_${index}`;
  const itemId = `PROVIDER::${PROVIDER_ID}::COLLECTION::${COLLECTION_ID}::ITEM::${productId}`;
  const primaryKey = "science-rslc-hdf5";
  const fileName = `${displayName}.h5`;
  const stableAssetKey = `${itemId}::ASSET::${primaryKey}`;
  const checksum = "f2c78f09a49e5f1f304b19f930dfed10f197d235e383075e8377ddaa7c33d181";
  const url = `https://nisar.asf.earthdatacloud.nasa.gov/products/${fileName}`;
  const properties = {
    provider_product_key: itemId,
    provider_product_id: productId,
    cmr_granule_concept_id: productId,
    producer_granule_id: displayName,
    logical_dataset: LOGICAL_DATASET,
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
    frame_coverage: "FULL",
    frame: 112 + index,
    track: 37,
    relative_orbit: 37,
    flight_direction: "ASCENDING",
    look_direction: "RIGHT",
    range_bandwidth: "20+5",
    joint_observation: true,
    acquisition_end_utc: `2026-06-18T10:${minute}:30Z`,
    primary_asset_key: primaryKey,
    preview_available: false,
    full_product: true,
    crid: "P05023",
    product_specification_version: "1.5.0",
    product_specification_profile_registry_version: 1,
    frequency_a_polarization: "HH+HV",
  };
  return {
    item_id: itemId,
    provider_id: PROVIDER_ID,
    display_name: displayName,
    acquisition_datetime: `2026-06-18T10:${minute}:00Z`,
    footprint_geojson: geometry,
    metadata: {
      dataset: LOGICAL_DATASET,
      logical_dataset: LOGICAL_DATASET,
      native_collection_id: COLLECTION_ID,
      collection_profile_id: PROFILE_ID,
      identity_rule_id: IDENTITY_RULE_ID,
      filename_rule_id: FILENAME_RULE_ID,
      delivery_model: "archive_first",
      resolver_policy_id: RESOLVER_POLICY_ID,
      provider_product_id: productId,
      provider_product_key: itemId,
      product_family: "NISAR_L1_RSLC_PROVISIONAL_V1",
      processing_level: "RSLC",
      quality_status: "Provisional",
      source_kind: "sar",
      product_kind: "sar_product",
      properties,
      assets: [{
        asset_key: primaryKey,
        stable_asset_key: stableAssetKey,
        role: "primary",
        native_roles: ["ARCHIVE:HDF5", "GET DATA", "GET DATA VIA DIRECT ACCESS"],
        file_name: fileName,
        media_type: "application/x-hdf5",
        size_bytes: 12_000_000_000,
        checksum: { algorithm: "SHA-256", value: checksum },
        routes: [{ scheme: "https", locator: url }],
        native_evidence: {
          archive_name: fileName,
          product_match: true,
          url_types: ["GET DATA", "GET DATA VIA DIRECT ACCESS"],
          url_subtypes: [],
          formats: ["HDF5"],
        },
      }],
    },
    payload: {},
    asset_access: {
      reference: {
        provider_id: PROVIDER_ID,
        collection_id: COLLECTION_ID,
        provider_product_id: productId,
        provider_product_key: itemId,
        canonical_product_key: null,
        asset_key: primaryKey,
        stable_asset_key: stableAssetKey,
      },
      descriptor: {
        resolver_id: "earthdata_https",
        scheme: "earthdata_https",
        stable_href: url,
        requires_credentials: true,
        credential_profile: "earthdata-default",
        resolver_parameters: {},
      },
      role: "primary",
      file_name: fileName,
      expected_size_bytes: 12_000_000_000,
      checksum_algorithm: "SHA256",
      checksum_value: checksum,
    },
    execution_policy: {
      authority: "resolver_authoritative",
      policy_id: RESOLVER_POLICY_ID,
      collection_profile_id: PROFILE_ID,
    },
  };
}
