use std::fs;
use std::path::{Path, PathBuf};

use insar_core::{
    provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
    AssetExecutionAuthority, AssetExecutionPolicy, AssetResolverParameters, AssetRole,
    FrozenAssetAccess, FrozenTaskSnapshot, SnapshotItem, StableAssetReference,
};
use insar_download::{plan_sar_catalog_target, DownloadWork};
use insar_provider::catalog::{
    NISAR_COLLECTION_SHORT_NAME, NISAR_FILENAME_RULE_ID, NISAR_IDENTITY_RULE_ID,
    NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
    NISAR_RESOLVER_POLICY_ID,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub const PRODUCT_ID: &str = "G4260317833-ASF";
pub const DISPLAY_NAME: &str =
    "NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_001";
pub const FILE_NAME: &str =
    "NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_001.h5";
pub const OFFICIAL_HREF: &str = "https://nisar.asf.earthdatacloud.nasa.gov/NISAR/NISAR_L1_RSLC_PROVISIONAL_V1/NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_001/NISAR_L1_PR_RSLC_026_045_D_056_0500_SHNA_M_20260723T074816_20260723T074822_P05023_N_P_J_001.h5";
pub const PRIMARY_ASSET_KEY: &str = "science-rslc-hdf5";
pub const BROWSE_ASSET_KEY: &str = "browse-latlon";
pub const THUMBNAIL_ASSET_KEY: &str = "browse-latlon-thumbnail";
pub const QA_ASSET_KEY: &str = "qa-stats-hdf5";

pub fn sha256_hex(body: &[u8]) -> String {
    format!("{:x}", Sha256::digest(body))
}

pub fn frozen_item(body: &[u8], checksum: String) -> SnapshotItem {
    frozen_item_with_source_identity(body.len() as u64, checksum)
}

pub fn frozen_item_with_source_identity(source_size: u64, checksum: String) -> SnapshotItem {
    let product_key =
        provider_product_key(NISAR_PROVIDER_ID, NISAR_NATIVE_COLLECTION_ID, PRODUCT_ID).unwrap();
    let primary_stable_key = stable_asset_key(&product_key, PRIMARY_ASSET_KEY).unwrap();
    let browse_stable_key = stable_asset_key(&product_key, BROWSE_ASSET_KEY).unwrap();
    let thumbnail_stable_key = stable_asset_key(&product_key, THUMBNAIL_ASSET_KEY).unwrap();
    let qa_stable_key = stable_asset_key(&product_key, QA_ASSET_KEY).unwrap();
    let browse_file = format!("{DISPLAY_NAME}_LATLON.png");
    let thumbnail_file = format!("{DISPLAY_NAME}_LATLON_thumbnail.png");
    let qa_file = format!("{DISPLAY_NAME}_QA_STATS.h5");
    let browse_href = format!("https://nisar.asf.earthdatacloud.nasa.gov/BROWSE/{browse_file}");
    let thumbnail_href =
        format!("https://nisar.asf.earthdatacloud.nasa.gov/BROWSE/{thumbnail_file}");
    let qa_href = format!("https://nisar.asf.earthdatacloud.nasa.gov/NISAR/{qa_file}");
    let descriptor = AssetAccessDescriptor {
        resolver_id: "earthdata_https".to_owned(),
        scheme: AssetAccessScheme::EarthdataHttps,
        stable_href: Some(OFFICIAL_HREF.to_owned()),
        requires_credentials: true,
        credential_profile: Some("earthdata-default".to_owned()),
        resolver_parameters: AssetResolverParameters {},
    };
    let access = FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: NISAR_PROVIDER_ID.to_owned(),
            collection_id: NISAR_NATIVE_COLLECTION_ID.to_owned(),
            provider_product_id: PRODUCT_ID.to_owned(),
            provider_product_key: product_key.clone(),
            canonical_product_key: None,
            asset_key: PRIMARY_ASSET_KEY.to_owned(),
            stable_asset_key: primary_stable_key.clone(),
        },
        descriptor,
        role: AssetRole::Primary,
        file_name: Some(FILE_NAME.to_owned()),
        expected_size_bytes: Some(source_size),
        checksum_algorithm: Some("SHA256".to_owned()),
        checksum_value: Some(checksum.clone()),
    };
    let assets = json!([
        {
            "asset_key": PRIMARY_ASSET_KEY,
            "stable_asset_key": primary_stable_key,
            "role": "primary",
            "native_roles": ["ARCHIVE:HDF5", "GET DATA", "GET DATA VIA DIRECT ACCESS"],
            "file_name": FILE_NAME,
            "media_type": "application/x-hdf5",
            "size_bytes": source_size,
            "checksum": {"algorithm": "SHA-256", "value": checksum},
            "routes": [
                {"scheme": "https", "locator": OFFICIAL_HREF},
                {"scheme": "s3", "locator": format!("s3://sds-n-cumulus-prod-nisar-products/NISAR/{FILE_NAME}")}
            ],
            "native_evidence": {
                "archive_name": FILE_NAME,
                "product_match": true,
                "url_types": ["GET DATA", "GET DATA VIA DIRECT ACCESS"],
                "url_subtypes": [],
                "formats": ["HDF5"]
            }
        },
        {
            "asset_key": BROWSE_ASSET_KEY,
            "stable_asset_key": browse_stable_key,
            "role": "browse",
            "native_roles": ["ARCHIVE:PNG", "GET RELATED VISUALIZATION"],
            "file_name": browse_file,
            "media_type": "image/png",
            "size_bytes": null,
            "checksum": null,
            "routes": [{"scheme": "https", "locator": browse_href}],
            "native_evidence": {
                "archive_name": format!("{DISPLAY_NAME}_LATLON.png"),
                "product_match": false,
                "url_types": ["GET RELATED VISUALIZATION"],
                "url_subtypes": [],
                "formats": ["PNG"]
            }
        },
        {
            "asset_key": THUMBNAIL_ASSET_KEY,
            "stable_asset_key": thumbnail_stable_key,
            "role": "thumbnail",
            "native_roles": ["ARCHIVE:PNG", "GET RELATED VISUALIZATION"],
            "file_name": thumbnail_file,
            "media_type": "image/png",
            "size_bytes": null,
            "checksum": null,
            "routes": [{"scheme": "https", "locator": thumbnail_href}],
            "native_evidence": {
                "archive_name": format!("{DISPLAY_NAME}_LATLON_thumbnail.png"),
                "product_match": false,
                "url_types": ["GET RELATED VISUALIZATION"],
                "url_subtypes": [],
                "formats": ["PNG"]
            }
        },
        {
            "asset_key": QA_ASSET_KEY,
            "stable_asset_key": qa_stable_key,
            "role": "auxiliary",
            "native_roles": ["ARCHIVE:HDF5", "PRODUCT QUALITY ASSESSMENT", "VIEW RELATED INFORMATION"],
            "file_name": qa_file,
            "media_type": "application/x-hdf5",
            "size_bytes": 3,
            "checksum": {"algorithm": "MD5", "value": "900150983cd24fb0d6963f7d28e17f72"},
            "routes": [{"scheme": "https", "locator": qa_href}],
            "native_evidence": {
                "archive_name": format!("{DISPLAY_NAME}_QA_STATS.h5"),
                "product_match": false,
                "url_types": ["VIEW RELATED INFORMATION"],
                "url_subtypes": ["PRODUCT QUALITY ASSESSMENT"],
                "formats": ["HDF5"]
            }
        }
    ]);
    let properties = json!({
        "provider_product_key": product_key,
        "provider_product_id": PRODUCT_ID,
        "cmr_granule_concept_id": PRODUCT_ID,
        "producer_granule_id": DISPLAY_NAME,
        "asf_product_id": DISPLAY_NAME,
        "logical_dataset": NISAR_LOGICAL_DATASET,
        "collection_concept_id": NISAR_NATIVE_COLLECTION_ID,
        "collection_short_name": NISAR_COLLECTION_SHORT_NAME,
        "collection_version": "1",
        "collection_profile_id": NISAR_PROFILE_ID,
        "identity_rule_id": NISAR_IDENTITY_RULE_ID,
        "filename_rule_id": NISAR_FILENAME_RULE_ID,
        "resolver_policy_id": NISAR_RESOLVER_POLICY_ID,
        "processing_level": "RSLC",
        "science_level": "L1",
        "quality_status": "Provisional",
        "production_configuration": "PR",
        "band": "L",
        "frame_coverage": "PARTIAL",
        "frame": 56,
        "track": 45,
        "relative_orbit": 45,
        "flight_direction": "DESCENDING",
        "look_direction": "RIGHT",
        "range_bandwidth": "5",
        "joint_observation": true,
        "acquisition_end_utc": "2026-07-23T07:48:22Z",
        "primary_asset_key": PRIMARY_ASSET_KEY,
        "preview_available": true,
        "full_product": true,
        "frequency_a_polarization": "HH",
        "crid": "P05023",
        "product_specification_version": "1.5.0",
        "product_specification_profile_registry_version": 1,
        "pge_version": "R05.02.3"
    });
    SnapshotItem {
        item_id: access.reference.provider_product_key.clone(),
        provider_id: NISAR_PROVIDER_ID.to_owned(),
        display_name: DISPLAY_NAME.to_owned(),
        acquisition_datetime: Some("2026-07-23T07:48:16Z".to_owned()),
        footprint_geojson: Some(
            json!({
                "type": "Polygon",
                "coordinates": [[[117.0,30.0],[120.0,30.0],[120.0,32.0],[117.0,32.0],[117.0,30.0]]]
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        metadata: Map::from_iter([
            (
                "dataset".to_owned(),
                Value::String(NISAR_LOGICAL_DATASET.to_owned()),
            ),
            (
                "logical_dataset".to_owned(),
                Value::String(NISAR_LOGICAL_DATASET.to_owned()),
            ),
            (
                "native_collection_id".to_owned(),
                Value::String(NISAR_NATIVE_COLLECTION_ID.to_owned()),
            ),
            (
                "collection_profile_id".to_owned(),
                Value::String(NISAR_PROFILE_ID.to_owned()),
            ),
            (
                "identity_rule_id".to_owned(),
                Value::String(NISAR_IDENTITY_RULE_ID.to_owned()),
            ),
            (
                "filename_rule_id".to_owned(),
                Value::String(NISAR_FILENAME_RULE_ID.to_owned()),
            ),
            (
                "delivery_model".to_owned(),
                Value::String("archive_first".to_owned()),
            ),
            (
                "resolver_policy_id".to_owned(),
                Value::String(NISAR_RESOLVER_POLICY_ID.to_owned()),
            ),
            (
                "provider_product_id".to_owned(),
                Value::String(PRODUCT_ID.to_owned()),
            ),
            (
                "provider_product_key".to_owned(),
                Value::String(access.reference.provider_product_key.clone()),
            ),
            (
                "product_family".to_owned(),
                Value::String(NISAR_COLLECTION_SHORT_NAME.to_owned()),
            ),
            (
                "processing_level".to_owned(),
                Value::String("RSLC".to_owned()),
            ),
            (
                "quality_status".to_owned(),
                Value::String("Provisional".to_owned()),
            ),
            ("source_kind".to_owned(), Value::String("sar".to_owned())),
            (
                "product_kind".to_owned(),
                Value::String("sar_product".to_owned()),
            ),
            ("properties".to_owned(), properties),
            ("assets".to_owned(), assets),
        ]),
        payload: Map::new(),
        asset_access: Some(access),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: NISAR_RESOLVER_POLICY_ID.to_owned(),
            collection_profile_id: Some(NISAR_PROFILE_ID.to_owned()),
        }),
    }
}

pub fn work(output_root: &Path, item: SnapshotItem, label: &str) -> DownloadWork {
    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": format!("nisar-{label}"),
        "batch_id": format!("nisar-{label}-batch"),
        "provider_id": NISAR_PROVIDER_ID,
        "created_at": "2026-07-29T00:00:00Z",
        "aoi_name": "NISAR fixture AOI",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[117.0,30.0],[120.0,30.0],[120.0,32.0],[117.0,32.0],[117.0,30.0]]]
        },
        "query": {"logical_dataset": NISAR_LOGICAL_DATASET},
        "candidates": [item.clone()],
        "initial_selected_item_ids": [item.item_id.clone()],
        "output_root": output_root.to_string_lossy(),
        "layout": "sar_dataset_processing"
    }))
    .expect("frozen NISAR task");
    DownloadWork { task, item }
}

pub fn destination(output_root: &Path) -> PathBuf {
    let destination =
        plan_sar_catalog_target(output_root, NISAR_LOGICAL_DATASET, "RSLC", FILE_NAME).unwrap();
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    destination
}
