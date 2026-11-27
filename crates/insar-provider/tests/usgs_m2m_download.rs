use insar_provider::usgs_m2m::{
    map_landsat_c2_l2_full_bundle_option, UsgsM2mBundleAvailability, UsgsM2mErrorCode,
    LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM, LANDSAT_C2_L2_BUNDLE_OPTION_ID,
    LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE, LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
    LANDSAT_C2_L2_CURRENT_BUNDLE_DOWNLOAD_SYSTEM, LANDSAT_C2_L2_CURRENT_BUNDLE_PRODUCT_CODE,
};
use serde_json::{json, Value};

const DATASET_ALIAS: &str = "landsat_ot_c2_l2";
const ENTITY_ID: &str = "LC91230452026001LGN00";
const DISPLAY_ID: &str = "LC09_L2SP_123045_20260101_20260103_02_T1";

fn option_response(available: bool, bulk_available: bool) -> Value {
    json!([
        {
            "id": LANDSAT_C2_L2_BUNDLE_OPTION_ID,
            "downloadName": null,
            "displayId": DISPLAY_ID,
            "entityId": ENTITY_ID,
            "datasetId": "5e83d14f2fc39685",
            "available": available,
            "filesize": 958605073,
            "productName": LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
            "productCode": LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE,
            "bulkAvailable": bulk_available,
            "downloadSystem": LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM,
            "secondaryDownloads": [{"id":"secondary-must-not-be-selected"}],
            "fileGroups": null,
            "checksum": [{"id":"sha512", "value":"a".repeat(128)}]
        },
        {
            "id": "632210d4770592cf",
            "downloadName": null,
            "displayId": DISPLAY_ID,
            "entityId": ENTITY_ID,
            "datasetId": "5e83d14f2fc39685",
            "available": false,
            "filesize": 958605073,
            "productName": LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME,
            "productCode": "D806",
            "bulkAvailable": false,
            "downloadSystem": "dds_ms",
            "secondaryDownloads": [],
            "fileGroups": null,
            "checksum": null
        },
        {
            "id": "6448198cc7b442a4",
            "downloadName": "C2L2 Tile Product Files",
            "displayId": DISPLAY_ID,
            "entityId": ENTITY_ID,
            "datasetId": "5e83d14f2fc39685",
            "available": true,
            "filesize": 0,
            "productName": "Landsat Collection 2 Level-2 Band File",
            "productCode": "D693",
            "bulkAvailable": true,
            "downloadSystem": "folder",
            "secondaryDownloads": [],
            "fileGroups": [{"id":"group-must-not-be-selected"}],
            "checksum": [{"id":"sha512", "value":null}]
        }
    ])
}

#[test]
fn exact_full_bundle_option_is_selected_without_secondary_band_or_group_authority() {
    let option = map_landsat_c2_l2_full_bundle_option(
        &option_response(true, true),
        DATASET_ALIAS,
        ENTITY_ID,
        DISPLAY_ID,
    )
    .expect("one exact full bundle");

    assert_eq!(option.dataset_alias, DATASET_ALIAS);
    assert_eq!(option.entity_id, ENTITY_ID);
    assert_eq!(option.display_id, DISPLAY_ID);
    assert_eq!(option.option_id, LANDSAT_C2_L2_BUNDLE_OPTION_ID);
    assert_eq!(option.product_name, LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME);
    assert_eq!(option.product_code, LANDSAT_C2_L2_BUNDLE_PRODUCT_CODE);
    assert_eq!(option.download_system, LANDSAT_C2_L2_BUNDLE_DOWNLOAD_SYSTEM);
    assert_eq!(option.availability, UsgsM2mBundleAvailability::Available);
    assert_eq!(option.expected_size_bytes, Some(958605073));
    assert_eq!(option.checksum_algorithm.as_deref(), Some("sha512"));
    assert_eq!(option.checksum_value.as_deref().map(str::len), Some(128));
    assert_eq!(option.expected_file_name, format!("{DISPLAY_ID}.tar"));

    let debug = format!("{option:?}").to_ascii_lowercase();
    for forbidden in [
        "https://",
        "x-auth-token",
        "authorization",
        "requestsignature",
        "downloadid",
        "secondary-must-not-be-selected",
        "group-must-not-be-selected",
    ] {
        assert!(!debug.contains(forbidden), "{forbidden}");
    }
}

#[test]
fn current_available_bundle_variant_wins_over_unavailable_legacy_option() {
    let mut response = option_response(false, false);
    response[1]["available"] = json!(true);
    response[1]["bulkAvailable"] = json!(true);
    let option =
        map_landsat_c2_l2_full_bundle_option(&response, DATASET_ALIAS, ENTITY_ID, DISPLAY_ID)
            .expect("current reviewed bundle");

    assert_eq!(option.option_id, "632210d4770592cf");
    assert_eq!(option.product_name, LANDSAT_C2_L2_BUNDLE_PRODUCT_NAME);
    assert_eq!(
        option.product_code,
        LANDSAT_C2_L2_CURRENT_BUNDLE_PRODUCT_CODE
    );
    assert_eq!(
        option.download_system,
        LANDSAT_C2_L2_CURRENT_BUNDLE_DOWNLOAD_SYSTEM
    );
    assert_eq!(option.availability, UsgsM2mBundleAvailability::Available);
    assert_eq!(option.expected_file_name, format!("{DISPLAY_ID}.tar"));
    assert_eq!(option.expected_size_bytes, Some(958605073));
}

#[test]
fn preparable_full_bundle_is_distinct_from_unavailable_delivery() {
    let option = map_landsat_c2_l2_full_bundle_option(
        &option_response(false, true),
        DATASET_ALIAS,
        ENTITY_ID,
        DISPLAY_ID,
    )
    .expect("bulk-preparable full bundle");
    assert_eq!(option.availability, UsgsM2mBundleAvailability::Preparable);

    let error = map_landsat_c2_l2_full_bundle_option(
        &option_response(false, false),
        DATASET_ALIAS,
        ENTITY_ID,
        DISPLAY_ID,
    )
    .unwrap_err();
    assert_eq!(error.code(), UsgsM2mErrorCode::DownloadUnavailable);
}

#[test]
fn option_identity_ambiguity_and_missing_integrity_fail_closed() {
    let mut duplicate = option_response(true, true);
    let duplicate_record = duplicate.as_array().unwrap()[0].clone();
    duplicate.as_array_mut().unwrap().push(duplicate_record);
    assert_eq!(
        map_landsat_c2_l2_full_bundle_option(&duplicate, DATASET_ALIAS, ENTITY_ID, DISPLAY_ID,)
            .unwrap_err()
            .code(),
        UsgsM2mErrorCode::DownloadOptionAmbiguous
    );

    let mut forged_current_id = option_response(false, false);
    forged_current_id[1]["available"] = json!(true);
    forged_current_id[1]["bulkAvailable"] = json!(true);
    forged_current_id[1]["id"] = json!("wrong-current-option");
    assert_eq!(
        map_landsat_c2_l2_full_bundle_option(
            &forged_current_id,
            DATASET_ALIAS,
            ENTITY_ID,
            DISPLAY_ID,
        )
        .unwrap_err()
        .code(),
        UsgsM2mErrorCode::DownloadUnavailable
    );

    for mutate in [
        |value: &mut Value| value[0]["entityId"] = json!("LC80000000000000000000"),
        |value: &mut Value| value[0]["displayId"] = json!("LC08_L2SP_CONFLICT_02_T1"),
        |value: &mut Value| value[0]["datasetId"] = json!("conflicting-dataset"),
        |value: &mut Value| value[0]["productCode"] = json!("D693"),
        |value: &mut Value| value[0]["downloadSystem"] = json!("folder"),
    ] {
        let mut response = option_response(true, true);
        mutate(&mut response);
        assert!(map_landsat_c2_l2_full_bundle_option(
            &response,
            DATASET_ALIAS,
            ENTITY_ID,
            DISPLAY_ID,
        )
        .is_err());
    }

    let mut no_integrity = option_response(true, true);
    no_integrity[0]["filesize"] = json!(0);
    no_integrity[0]["checksum"] = Value::Null;
    assert_eq!(
        map_landsat_c2_l2_full_bundle_option(&no_integrity, DATASET_ALIAS, ENTITY_ID, DISPLAY_ID,)
            .unwrap_err()
            .code(),
        UsgsM2mErrorCode::DownloadIntegrityMissing
    );
}

#[test]
fn strongest_supported_checksum_is_selected_and_same_algorithm_conflicts_fail_closed() {
    let mut response = option_response(true, true);
    response[0]["checksum"] = json!([
        {"id": "md5", "value": "b".repeat(32)},
        {"id": "sha512", "value": "a".repeat(128)},
        {"id": "sha256", "value": "c".repeat(64)}
    ]);
    let option =
        map_landsat_c2_l2_full_bundle_option(&response, DATASET_ALIAS, ENTITY_ID, DISPLAY_ID)
            .expect("the strongest supported checksum is deterministic");
    assert_eq!(option.checksum_algorithm.as_deref(), Some("sha512"));
    assert_eq!(
        option.checksum_value.as_deref(),
        Some("a".repeat(128).as_str())
    );

    response[0]["checksum"] = json!([
        {"id": "sha512", "value": "a".repeat(128)},
        {"id": "sha512", "value": "b".repeat(128)}
    ]);
    assert_eq!(
        map_landsat_c2_l2_full_bundle_option(&response, DATASET_ALIAS, ENTITY_ID, DISPLAY_ID,)
            .unwrap_err()
            .code(),
        UsgsM2mErrorCode::DownloadOptionAmbiguous
    );
}
