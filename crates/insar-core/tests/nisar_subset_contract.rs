use std::fs;

use insar_core::{
    embed_request_manifest, parse_request_manifest, validate_request_against_snapshot_item,
    AssetAccessDescriptor, AssetAccessScheme, AssetExecutionAuthority, AssetExecutionPolicy,
    AssetResolverParameters, AssetRole, FrozenAssetAccess, NisarSubsetArtifactManifestV1,
    NisarSubsetManifestError, NisarSubsetRangeState, NisarSubsetRequestManifestV1,
    NisarSubsetTransferManifestV1, NisarSubsetTransferState, SnapshotItem, StableAssetReference,
    NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES, NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES,
    NISAR_SUBSET_MAX_RANGES, NISAR_SUBSET_MAX_SAFE_INTEGER,
    NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES,
};
use serde_json::{json, Map, Value};

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/../../docs/fixtures/asf-nisar-subset-p1/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&fs::read_to_string(path).expect("fixture exists")).expect("valid JSON")
}

fn invalid_request(value: Value) -> bool {
    match serde_json::from_value::<NisarSubsetRequestManifestV1>(value) {
        Ok(manifest) => manifest.validate().is_err(),
        Err(_) => true,
    }
}

fn invalid_transfer(value: Value) -> bool {
    match serde_json::from_value::<NisarSubsetTransferManifestV1>(value) {
        Ok(manifest) => manifest.validate().is_err(),
        Err(_) => true,
    }
}

fn invalid_artifact(value: Value) -> bool {
    match serde_json::from_value::<NisarSubsetArtifactManifestV1>(value) {
        Ok(manifest) => manifest.validate().is_err(),
        Err(_) => true,
    }
}

fn invalid_fixture(case: &Value) -> Value {
    fixture(case["fixture"].as_str().expect("invalid fixture path"))
}

fn snapshot_for_request(request: &NisarSubsetRequestManifestV1) -> SnapshotItem {
    let source = &request.source;
    SnapshotItem {
        item_id: source.provider_product_key.clone(),
        provider_id: source.provider_id.clone(),
        display_name: source.provider_product_id.clone(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::from_iter([(
            "collection_profile_id".to_owned(),
            json!(source.collection_profile_id),
        )]),
        payload: Map::new(),
        asset_access: Some(FrozenAssetAccess {
            reference: StableAssetReference {
                provider_id: source.provider_id.clone(),
                collection_id: source.collection_id.clone(),
                provider_product_id: source.provider_product_id.clone(),
                provider_product_key: source.provider_product_key.clone(),
                canonical_product_key: None,
                asset_key: "science-rslc-hdf5".to_owned(),
                stable_asset_key: source.stable_asset_key.clone(),
            },
            descriptor: AssetAccessDescriptor {
                resolver_id: "earthdata_https".to_owned(),
                scheme: AssetAccessScheme::EarthdataHttps,
                stable_href: None,
                requires_credentials: false,
                credential_profile: None,
                resolver_parameters: AssetResolverParameters {},
            },
            role: AssetRole::Primary,
            file_name: Some(source.file_name.clone()),
            expected_size_bytes: Some(source.expected_size_bytes),
            checksum_algorithm: source
                .checksum
                .as_ref()
                .map(|checksum| format!("{:?}", checksum.algorithm).to_ascii_uppercase()),
            checksum_value: source
                .checksum
                .as_ref()
                .map(|checksum| checksum.value.clone()),
        }),
        execution_policy: Some(AssetExecutionPolicy {
            authority: AssetExecutionAuthority::ResolverAuthoritative,
            policy_id: "asf.nisar.earthdata-complete-hdf5.v2".to_owned(),
            collection_profile_id: Some(source.collection_profile_id.clone()),
        }),
    }
}

#[test]
fn all_three_manifest_contracts_validate_from_the_shared_fixture_set() {
    let request: NisarSubsetRequestManifestV1 =
        serde_json::from_value(fixture("request-manifest-v1.json")).expect("request fixture");
    request.validate().expect("request validates");
    assert!(request.canonical_json().unwrap().len() < 32 * 1024);
    assert_eq!(
        request.sha256().unwrap(),
        "2f276343dbf74fd2b92ad60ed0d34e41eb50b25d861be682579880fd2097199b"
    );

    let transfer: NisarSubsetTransferManifestV1 =
        serde_json::from_value(fixture("transfer-manifest-v1.json")).expect("transfer fixture");
    transfer.validate().expect("transfer validates");
    assert_eq!(
        transfer.sha256().unwrap(),
        "2e935cec7f6beb49b18b3fd51aea286a99307f5a55d9211b8b3dc4a680947adf"
    );

    let artifact: NisarSubsetArtifactManifestV1 =
        serde_json::from_value(fixture("artifact-manifest-v1.json")).expect("artifact fixture");
    artifact.validate().expect("artifact validates");
    assert_eq!(artifact.output_authority, "client_derived");
    assert!(artifact.complete);
    assert_eq!(
        artifact.sha256().unwrap(),
        "247d13e7ccf9f94efda3bca3e110512b2cbff1c3140200008a21fc489aa99977"
    );
}

#[test]
fn shared_invalid_fixture_index_executes_in_rust() {
    let cases = fixture("invalid/cases.json");
    for case in cases.as_array().unwrap() {
        let value = invalid_fixture(case);
        let target = case["target"].as_str().unwrap();
        let invalid = match target {
            "request" => invalid_request(value),
            "transfer" => invalid_transfer(value),
            "artifact" => invalid_artifact(value),
            other => panic!("unknown invalid fixture target {other}"),
        };
        assert!(invalid, "invalid fixture did not fail: {}", case["case"]);
    }
}

#[test]
fn request_manifest_digest_is_stable_when_semantically_sorted_windows_are_reordered() {
    let request: NisarSubsetRequestManifestV1 =
        serde_json::from_value(fixture("request-manifest-v1.json")).expect("request fixture");
    let mut reordered = request.clone();
    let mut second = reordered.mapping.windows[0].clone();
    second.component_id = 1;
    second.line_start = 400;
    second.line_end_exclusive = 600;
    second.expected_shape = [200, 512];
    reordered.request.aoi_component_count = 2;
    reordered.mapping.windows.push(second);
    reordered.mapping.windows.reverse();
    let mut sorted = reordered.clone();
    sorted.mapping.windows.sort_by_key(|window| {
        (
            window.component_id,
            window.dataset_path.clone(),
            window.line_start,
            window.sample_start,
        )
    });
    assert_eq!(reordered.sha256().unwrap(), sorted.sha256().unwrap());

    let mut changed = sorted;
    changed.mapping.windows[0].line_start += 1;
    changed.mapping.windows[0].expected_shape[0] -= 1;
    assert_ne!(request.sha256().unwrap(), changed.sha256().unwrap());
}

#[test]
fn unknown_sensitive_and_scope_fields_fail_closed_without_full_download_fallback() {
    let mut unknown = fixture("request-manifest-v1.json");
    unknown["unknown_field"] = json!(true);
    assert!(invalid_request(unknown));

    let mut sensitive = fixture("request-manifest-v1.json");
    sensitive["source"]["token"] = json!("not-a-secret");
    assert!(invalid_request(sensitive));

    let mut wrong_scope = fixture("request-manifest-v1.json");
    wrong_scope["output_authority"] = json!("provider_generated");
    assert!(invalid_request(wrong_scope));

    let mut full_download = fixture("request-manifest-v1.json");
    full_download["request"]["subset_method"] = json!("full_download_crop");
    assert!(invalid_request(full_download));
}

#[test]
fn windows_ranges_and_artifact_completion_are_bounded() {
    let mut window = fixture("request-manifest-v1.json");
    window["mapping"]["windows"][0]["line_end_exclusive"] = json!(1001);
    assert!(invalid_request(window));

    let mut mixed_frequency = fixture("request-manifest-v1.json");
    mixed_frequency["mapping"]["windows"][0]["frequency"] = json!("B");
    assert!(invalid_request(mixed_frequency));

    let mut overlapping = fixture("transfer-manifest-v1.json");
    overlapping["ranges"][1]["start"] = json!(512);
    assert!(invalid_transfer(overlapping));

    let mut false_complete = fixture("artifact-manifest-v1.json");
    false_complete["complete"] = json!(false);
    assert!(invalid_artifact(false_complete));

    let mut failed_science = fixture("artifact-manifest-v1.json");
    failed_science["science_validation"]["metadata"]["status"] = json!("not_run");
    assert!(invalid_artifact(failed_science));
}

#[test]
fn request_manifest_round_trips_through_task_specific_snapshot_metadata() {
    let request: NisarSubsetRequestManifestV1 =
        serde_json::from_value(fixture("request-manifest-v1.json")).expect("request fixture");
    let mut metadata = Map::new();
    embed_request_manifest(&mut metadata, &request).expect("embed request manifest");

    let mut item = snapshot_for_request(&request);
    item.metadata = metadata;
    item.validate().expect("snapshot item validates");
    let encoded = serde_json::to_vec(&item).expect("snapshot item serializes");
    let restored: SnapshotItem = serde_json::from_slice(&encoded).expect("snapshot item restores");
    let parsed = parse_request_manifest(&restored.metadata).expect("namespaced manifest parses");
    assert_eq!(parsed, request);
    validate_request_against_snapshot_item(&parsed, &restored).unwrap();

    let ordinary = Map::<String, Value>::new();
    assert_eq!(
        parse_request_manifest(&ordinary),
        Err(NisarSubsetManifestError::ManifestMissing)
    );
}

#[test]
fn transfer_and_artifact_contracts_match_request_identity_and_science_extent() {
    let request: NisarSubsetRequestManifestV1 =
        serde_json::from_value(fixture("request-manifest-v1.json")).unwrap();
    let transfer: NisarSubsetTransferManifestV1 =
        serde_json::from_value(fixture("transfer-manifest-v1.json")).unwrap();
    let artifact: NisarSubsetArtifactManifestV1 =
        serde_json::from_value(fixture("artifact-manifest-v1.json")).unwrap();

    transfer.validate_against_request(&request).unwrap();
    artifact
        .validate_against_request_and_transfer(&request, &transfer)
        .unwrap();

    let mut changed_transfer = transfer.clone();
    changed_transfer.source_identity.provider_product_id = "different-product".to_owned();
    assert!(changed_transfer.validate_against_request(&request).is_err());

    let mut changed_artifact = artifact.clone();
    changed_artifact.dataset_windows[0].polarization = "VV".to_owned();
    assert!(changed_artifact.validate().is_err());

    let mut unrelated_mask = artifact.clone();
    unrelated_mask.validity_masks[0].dataset_path =
        "/science/LSAR/RSLC/swaths/frequencyA/HH/inputDataExceptionMask".to_owned();
    assert!(unrelated_mask
        .validate_against_request_and_transfer(&request, &transfer)
        .is_err());

    let mut mismatched_specification = artifact;
    mismatched_specification
        .validator
        .product_specification_version = "1.5.1".to_owned();
    assert!(mismatched_specification.validate().is_err());
}

#[test]
fn canonical_limits_numeric_order_and_safe_integer_bound_are_shared_contracts() {
    let mut numeric = fixture("request-manifest-v1.json");
    let template = numeric["mapping"]["windows"][0].clone();
    let mut window_two = template.clone();
    window_two["line_start"] = json!(2);
    window_two["line_end_exclusive"] = json!(3);
    window_two["expected_shape"] = json!([1, 512]);
    let mut window_ten = template.clone();
    window_ten["line_start"] = json!(10);
    window_ten["line_end_exclusive"] = json!(11);
    window_ten["expected_shape"] = json!([1, 512]);
    numeric["mapping"]["windows"] = json!([window_ten, window_two]);
    let manifest: NisarSubsetRequestManifestV1 = serde_json::from_value(numeric).unwrap();
    let canonical = manifest.canonical_json().unwrap();
    assert!(
        canonical.find("\"line_start\":2").unwrap() < canonical.find("\"line_start\":10").unwrap()
    );

    let mut same_prefix = fixture("request-manifest-v1.json");
    let mut first = template.clone();
    first["line_start"] = json!(2);
    first["line_end_exclusive"] = json!(3);
    first["expected_shape"] = json!([1, 512]);
    let mut second = first.clone();
    second["line_end_exclusive"] = json!(4);
    second["expected_shape"] = json!([2, 512]);
    same_prefix["mapping"]["windows"] = json!([second.clone(), first.clone()]);
    let first_order: NisarSubsetRequestManifestV1 = serde_json::from_value(same_prefix).unwrap();
    let mut reversed = first_order.clone();
    reversed.mapping.windows.reverse();
    assert_eq!(first_order.sha256().unwrap(), reversed.sha256().unwrap());

    let mut equal_ratio = fixture("request-manifest-v1.json");
    equal_ratio["expected_output"]["estimated_remote_bytes"] = json!(100_000_000_u64);
    equal_ratio["expected_output"]["estimated_output_bytes"] = json!(1);
    let equal_ratio: NisarSubsetRequestManifestV1 = serde_json::from_value(equal_ratio).unwrap();
    equal_ratio.validate().unwrap();

    let mut tiny_ratio = fixture("request-manifest-v1.json");
    tiny_ratio["expected_output"]["estimated_remote_bytes"] = json!(1);
    tiny_ratio["expected_output"]["estimated_output_bytes"] = json!(1);
    let tiny_ratio: NisarSubsetRequestManifestV1 = serde_json::from_value(tiny_ratio).unwrap();
    tiny_ratio.validate().unwrap();

    let mut near_safe = fixture("request-manifest-v1.json");
    near_safe["source"]["expected_size_bytes"] = json!(NISAR_SUBSET_MAX_SAFE_INTEGER);
    near_safe["expected_output"]["full_product_bytes"] = json!(NISAR_SUBSET_MAX_SAFE_INTEGER);
    near_safe["expected_output"]["estimated_remote_bytes"] =
        json!(NISAR_SUBSET_MAX_SAFE_INTEGER - 1);
    let near_safe: NisarSubsetRequestManifestV1 = serde_json::from_value(near_safe).unwrap();
    near_safe.validate().unwrap();

    let mut too_large_integer = fixture("request-manifest-v1.json");
    too_large_integer["source"]["expected_size_bytes"] = json!(NISAR_SUBSET_MAX_SAFE_INTEGER + 1);
    let too_large_integer: NisarSubsetRequestManifestV1 =
        serde_json::from_value(too_large_integer).unwrap();
    assert!(too_large_integer.validate().is_err());

    let mut oversized = fixture("request-manifest-v1.json");
    let mut windows = Vec::new();
    let oversized_swath = "s".repeat(128);
    oversized["request"]["swath"] = json!(oversized_swath.clone());
    oversized["request"]["aoi_component_count"] = json!(NISAR_SUBSET_MAX_COMPONENTS_FOR_TEST);
    for component in 0..NISAR_SUBSET_MAX_COMPONENTS_FOR_TEST {
        let mut window = template.clone();
        window["component_id"] = json!(component);
        window["swath"] = json!(oversized_swath.clone());
        window["dtype"] = json!("d".repeat(64));
        windows.push(window);
    }
    oversized["mapping"]["windows"] = Value::Array(windows);
    let oversized: NisarSubsetRequestManifestV1 = serde_json::from_value(oversized).unwrap();
    assert_eq!(
        oversized.validate(),
        Err(NisarSubsetManifestError::ManifestTooLarge)
    );
    assert_eq!(
        embed_request_manifest(&mut Map::new(), &oversized),
        Err(NisarSubsetManifestError::ManifestTooLarge)
    );
    assert_eq!(
        insar_core::parse_canonical_json::<Value>(
            &"{".repeat(NISAR_SUBSET_MAX_CANONICAL_JSON_BYTES + 1)
        ),
        Err(NisarSubsetManifestError::ManifestTooLarge)
    );
}

const NISAR_SUBSET_MAX_COMPONENTS_FOR_TEST: u16 = 64;

#[test]
fn transfer_state_matrix_and_rfc3339_validation_fail_closed() {
    let base = fixture("transfer-manifest-v1.json");

    let mut planned = base.clone();
    planned["state"] = json!("PLANNED");
    planned["received_bytes"] = json!(0);
    for range in planned["ranges"].as_array_mut().unwrap() {
        range["state"] = json!("PLANNED");
        range["received_bytes"] = json!(0);
        if let Some(object) = range.as_object_mut() {
            object.remove("sha256");
        }
    }
    let planned: NisarSubsetTransferManifestV1 = serde_json::from_value(planned).unwrap();
    planned.validate().unwrap();

    let complete: NisarSubsetTransferManifestV1 = serde_json::from_value(base.clone()).unwrap();
    complete.validate().unwrap();

    let mut failed = base.clone();
    failed["state"] = json!("FAILED");
    failed["ranges"][1]["state"] = json!("FAILED");
    failed["ranges"][1]["received_bytes"] = json!(0);
    failed["received_bytes"] = failed["ranges"][0]["received_bytes"].clone();
    failed["ranges"][1]
        .as_object_mut()
        .unwrap()
        .remove("sha256");
    let failed: NisarSubsetTransferManifestV1 = serde_json::from_value(failed).unwrap();
    failed.validate().unwrap();

    for state in ["INTERRUPTED", "SOURCE_CHANGED"] {
        let mut recoverable = base.clone();
        recoverable["state"] = json!(state);
        recoverable["ranges"][1]["state"] = json!("PLANNED");
        recoverable["ranges"][1]["received_bytes"] = json!(0);
        recoverable["ranges"][1]
            .as_object_mut()
            .unwrap()
            .remove("sha256");
        recoverable["received_bytes"] = recoverable["ranges"][0]["received_bytes"].clone();
        let recoverable: NisarSubsetTransferManifestV1 =
            serde_json::from_value(recoverable).unwrap();
        recoverable.validate().unwrap();
    }

    let mut invalid_state = base.clone();
    invalid_state["state"] = json!("IN_PROGRESS");
    invalid_state["received_bytes"] = json!(0);
    for range in invalid_state["ranges"].as_array_mut().unwrap() {
        range["state"] = json!("PLANNED");
        range["received_bytes"] = json!(0);
        range.as_object_mut().unwrap().remove("sha256");
    }
    let invalid_state: NisarSubsetTransferManifestV1 =
        serde_json::from_value(invalid_state).unwrap();
    assert!(invalid_state.validate().is_err());

    let mut invalid_date = base;
    invalid_date["created_at"] = json!("2026-13-01T00:00:00Z");
    assert!(invalid_transfer(invalid_date));
}

#[test]
fn official_science_paths_frequency_b_polarizations_and_frequency_masks_are_closed() {
    let official = serde_json::from_str::<Value>(
        &fs::read_to_string(format!(
            "{}/../../docs/fixtures/asf-nisar-subset-p0/official-single-scene-result.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap();
    for dataset in official["science"]["datasets"].as_array().unwrap() {
        let parts = dataset.as_str().unwrap().split('/').collect::<Vec<_>>();
        let mut value = fixture("request-manifest-v1.json");
        let frequency = parts[0].strip_prefix("frequency").unwrap();
        value["request"]["frequency"] = json!(frequency);
        value["request"]["polarization"] = json!(parts[1]);
        value["mapping"]["windows"][0]["frequency"] = json!(frequency);
        value["mapping"]["windows"][0]["polarization"] = json!(parts[1]);
        value["mapping"]["windows"][0]["dataset_path"] = json!(format!(
            "/science/LSAR/RSLC/swaths/{}/{}",
            parts[0], parts[1]
        ));
        let request: NisarSubsetRequestManifestV1 = serde_json::from_value(value).unwrap();
        request.validate().unwrap();
    }

    for polarization in ["HV", "VV"] {
        let mut value = fixture("request-manifest-v1.json");
        value["request"]["frequency"] = json!("B");
        value["request"]["polarization"] = json!(polarization);
        value["mapping"]["windows"][0]["frequency"] = json!("B");
        value["mapping"]["windows"][0]["polarization"] = json!(polarization);
        value["mapping"]["windows"][0]["dataset_path"] = json!(format!(
            "/science/LSAR/RSLC/swaths/frequencyB/{polarization}"
        ));
        let request: NisarSubsetRequestManifestV1 = serde_json::from_value(value).unwrap();
        request.validate().unwrap();
    }

    let mut invalid_polarization = fixture("request-manifest-v1.json");
    invalid_polarization["request"]["polarization"] = json!("VH");
    invalid_polarization["mapping"]["windows"][0]["polarization"] = json!("VH");
    invalid_polarization["mapping"]["windows"][0]["dataset_path"] =
        json!("/science/LSAR/RSLC/swaths/frequencyA/VH");
    assert!(invalid_request(invalid_polarization));

    let artifact: NisarSubsetArtifactManifestV1 =
        serde_json::from_value(fixture("artifact-manifest-v1.json")).unwrap();
    assert_eq!(
        artifact.validity_masks[0].dataset_path,
        "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask"
    );
    let mut polarization_child = artifact;
    polarization_child.validity_masks[0].dataset_path =
        "/science/LSAR/RSLC/swaths/frequencyA/HH/inputDataExceptionMask".to_owned();
    assert!(polarization_child.validate().is_err());
    for mask in official["science"]["qa_masks"].as_array().unwrap() {
        let path = format!("/science/LSAR/RSLC/swaths/{}", mask.as_str().unwrap());
        assert!(matches!(
            path.as_str(),
            "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask"
                | "/science/LSAR/RSLC/swaths/frequencyB/inputDataExceptionMask"
        ));
    }
}

#[test]
fn request_binds_to_the_frozen_official_nisar_primary_hdf5_identity() {
    let request: NisarSubsetRequestManifestV1 =
        serde_json::from_value(fixture("request-manifest-v1.json")).unwrap();
    let item = snapshot_for_request(&request);
    validate_request_against_snapshot_item(&request, &item).unwrap();

    type SnapshotMutation = Box<dyn Fn(&mut SnapshotItem)>;
    let mut mutations: Vec<SnapshotMutation> = vec![
        Box::new(|item| item.provider_id = "asf".to_owned()),
        Box::new(|item| {
            item.asset_access.as_mut().unwrap().reference.collection_id = "wrong".to_owned()
        }),
        Box::new(|item| {
            item.asset_access
                .as_mut()
                .unwrap()
                .reference
                .provider_product_id = "wrong".to_owned()
        }),
        Box::new(|item| {
            item.asset_access
                .as_mut()
                .unwrap()
                .reference
                .provider_product_key = "wrong".to_owned()
        }),
        Box::new(|item| {
            item.asset_access
                .as_mut()
                .unwrap()
                .reference
                .stable_asset_key = "wrong".to_owned()
        }),
        Box::new(|item| {
            item.asset_access.as_mut().unwrap().file_name = Some("wrong.h5".to_owned())
        }),
        Box::new(|item| item.asset_access.as_mut().unwrap().expected_size_bytes = Some(1)),
        Box::new(|item| item.asset_access.as_mut().unwrap().checksum_value = Some("0".repeat(64))),
        Box::new(|item| item.asset_access.as_mut().unwrap().role = AssetRole::Auxiliary),
        Box::new(|item| {
            item.execution_policy
                .as_mut()
                .unwrap()
                .collection_profile_id = Some("asf.sentinel1".to_owned())
        }),
    ];
    for mutate in mutations.drain(..) {
        let mut changed = item.clone();
        mutate(&mut changed);
        assert!(validate_request_against_snapshot_item(&request, &changed).is_err());
    }
    let embedded = serde_json::to_string(&request)
        .unwrap()
        .to_ascii_lowercase();
    for forbidden in [
        "http://",
        "https://",
        "credential_profile",
        "authorization",
        "cookie",
    ] {
        assert!(!embedded.contains(forbidden));
    }
}

#[test]
fn artifact_requires_a_completed_transfer_and_exact_output_identity() {
    let request: NisarSubsetRequestManifestV1 =
        serde_json::from_value(fixture("request-manifest-v1.json")).unwrap();
    let transfer: NisarSubsetTransferManifestV1 =
        serde_json::from_value(fixture("transfer-manifest-v1.json")).unwrap();
    let artifact: NisarSubsetArtifactManifestV1 =
        serde_json::from_value(fixture("artifact-manifest-v1.json")).unwrap();

    for state in [
        NisarSubsetTransferState::InProgress,
        NisarSubsetTransferState::Interrupted,
        NisarSubsetTransferState::SourceChanged,
        NisarSubsetTransferState::Failed,
    ] {
        let mut incomplete = transfer.clone();
        incomplete.state = state;
        incomplete.ranges[1].state = if incomplete.state == NisarSubsetTransferState::Failed {
            NisarSubsetRangeState::Failed
        } else {
            NisarSubsetRangeState::Planned
        };
        incomplete.ranges[1].received_bytes = 0;
        incomplete.ranges[1].sha256 = None;
        incomplete.received_bytes = incomplete.ranges[0].received_bytes;
        let mut rebound = artifact.clone();
        rebound.transfer_manifest_sha256 = incomplete.sha256().unwrap();
        assert!(rebound
            .validate_against_request_and_transfer(&request, &incomplete)
            .is_err());
    }

    let mut short = transfer.clone();
    short.received_bytes -= 1;
    assert!(short.validate().is_err());
    let mut non_complete_range = transfer.clone();
    non_complete_range.ranges[1].state = NisarSubsetRangeState::Planned;
    assert!(non_complete_range.validate().is_err());

    let mut wrong_id = artifact.clone();
    wrong_id.artifact_id = "other-derived-subset".to_owned();
    assert!(wrong_id
        .validate_against_request_and_transfer(&request, &transfer)
        .is_err());
    let mut wrong_name = artifact;
    wrong_name.artifact_file_name = "other_subset.h5".to_owned();
    assert!(wrong_name
        .validate_against_request_and_transfer(&request, &transfer)
        .is_err());
}

#[test]
fn request_transfer_and_artifact_have_separate_finite_size_budgets() {
    let mut transfer: NisarSubsetTransferManifestV1 =
        serde_json::from_value(fixture("transfer-manifest-v1.json")).unwrap();
    transfer.ranges = (0..NISAR_SUBSET_MAX_RANGES)
        .map(|index| insar_core::NisarSubsetByteRangeV1 {
            start: index as u64,
            end_exclusive: index as u64 + 1,
            expected_bytes: 1,
            received_bytes: 1,
            sha256: Some(format!("{index:064x}")),
            state: NisarSubsetRangeState::Complete,
        })
        .collect();
    transfer.expected_remote_bytes = NISAR_SUBSET_MAX_RANGES as u64;
    transfer.received_bytes = NISAR_SUBSET_MAX_RANGES as u64;
    transfer.validate().unwrap();
    assert!(
        transfer.canonical_json().unwrap().len() <= NISAR_SUBSET_MAX_TRANSFER_CANONICAL_JSON_BYTES
    );
    let mut too_many_ranges = transfer.clone();
    too_many_ranges
        .ranges
        .push(too_many_ranges.ranges.last().unwrap().clone());
    assert!(too_many_ranges.validate().is_err());

    let mut artifact: NisarSubsetArtifactManifestV1 =
        serde_json::from_value(fixture("artifact-manifest-v1.json")).unwrap();
    let window = artifact.dataset_windows[0].clone();
    let mask = artifact.validity_masks[0].clone();
    artifact.dataset_windows = (0..64)
        .map(|component| {
            let mut value = window.clone();
            value.component_id = component;
            value
        })
        .collect();
    artifact.validity_masks = (0..64)
        .map(|component| {
            let mut value = mask.clone();
            value.component_id = component;
            value.sha256 = format!("{component:064x}");
            value
        })
        .collect();
    artifact.validate().unwrap();
    assert!(
        artifact.canonical_json().unwrap().len() <= NISAR_SUBSET_MAX_ARTIFACT_CANONICAL_JSON_BYTES
    );
    let mut too_many_windows = artifact.clone();
    too_many_windows.dataset_windows.push(window);
    assert!(too_many_windows.validate().is_err());
    let mut too_many_masks = artifact;
    too_many_masks.validity_masks.push(mask);
    assert!(too_many_masks.validate().is_err());
}

#[test]
fn request_component_coverage_duplicates_and_optional_masks_fail_closed() {
    let base = fixture("request-manifest-v1.json");
    let mut outside = base.clone();
    outside["mapping"]["windows"][0]["component_id"] = json!(1);
    assert!(invalid_request(outside));

    let mut missing = base.clone();
    missing["request"]["aoi_component_count"] = json!(2);
    assert!(invalid_request(missing));

    let mut duplicate_window = base.clone();
    duplicate_window["mapping"]["windows"] = json!([
        duplicate_window["mapping"]["windows"][0].clone(),
        duplicate_window["mapping"]["windows"][0].clone()
    ]);
    assert!(invalid_request(duplicate_window));

    let mut duplicate_mask = base;
    let mut second = duplicate_mask["mapping"]["windows"][0].clone();
    second["polarization"] = json!("HV");
    second["dataset_path"] = json!("/science/LSAR/RSLC/swaths/frequencyA/HV");
    duplicate_mask["mapping"]["windows"]
        .as_array_mut()
        .unwrap()
        .push(second);
    assert!(invalid_request(duplicate_mask));

    let mut request: NisarSubsetRequestManifestV1 =
        serde_json::from_value(fixture("request-manifest-v1.json")).unwrap();
    let mut transfer: NisarSubsetTransferManifestV1 =
        serde_json::from_value(fixture("transfer-manifest-v1.json")).unwrap();
    let mut artifact: NisarSubsetArtifactManifestV1 =
        serde_json::from_value(fixture("artifact-manifest-v1.json")).unwrap();
    request.mapping.windows[0].validity_mask_required = false;
    transfer.request_manifest_sha256 = request.sha256().unwrap();
    artifact.request_manifest_sha256 = request.sha256().unwrap();
    artifact.dataset_windows = request.mapping.windows.clone();
    artifact.validity_masks.clear();
    artifact.transfer_manifest_sha256 = transfer.sha256().unwrap();
    artifact
        .validate_against_request_and_transfer(&request, &transfer)
        .unwrap();
}

#[test]
fn transfer_timestamp_order_is_compared_as_utc_instants() {
    let base = fixture("transfer-manifest-v1.json");
    for (created, updated) in [
        ("2026-08-01T00:00:00Z", "2026-08-01T00:00:00Z"),
        ("2026-08-01T00:00:00Z", "2026-08-01T00:00:01Z"),
        ("2026-08-01T01:00:00+01:00", "2026-08-01T00:00:00Z"),
    ] {
        let mut value = base.clone();
        value["created_at"] = json!(created);
        value["updated_at"] = json!(updated);
        let transfer: NisarSubsetTransferManifestV1 = serde_json::from_value(value).unwrap();
        transfer.validate().unwrap();
    }
    for (created, updated) in [
        ("2026-08-01T00:00:01Z", "2026-08-01T00:00:00Z"),
        ("2026-08-01T01:00:01+01:00", "2026-08-01T00:00:00Z"),
    ] {
        let mut value = base.clone();
        value["created_at"] = json!(created);
        value["updated_at"] = json!(updated);
        assert!(invalid_transfer(value));
    }
}

fn build_parity_case(builder: &str) -> Value {
    let mut base = fixture("request-manifest-v1.json");
    let template = base["mapping"]["windows"][0].clone();
    match builder {
        "two_windows_numeric" | "two_windows_numeric_reversed" => {
            let mut first = template.clone();
            first["line_start"] = json!(2);
            first["line_end_exclusive"] = json!(3);
            first["expected_shape"] = json!([1, 512]);
            let mut second = template.clone();
            second["line_start"] = json!(10);
            second["line_end_exclusive"] = json!(11);
            second["expected_shape"] = json!([1, 512]);
            base["mapping"]["windows"] = if builder == "two_windows_numeric" {
                json!([second, first])
            } else {
                json!([first, second])
            };
            base
        }
        "ratio_equal_one" => {
            base["expected_output"]["estimated_remote_bytes"] = json!(100_000_000_u64);
            base["expected_output"]["estimated_output_bytes"] = json!(1);
            base
        }
        "ratio_very_small" => {
            base["expected_output"]["estimated_remote_bytes"] = json!(1);
            base["expected_output"]["estimated_output_bytes"] = json!(1);
            base
        }
        "near_max_safe_integer" => {
            base["source"]["expected_size_bytes"] = json!(NISAR_SUBSET_MAX_SAFE_INTEGER);
            base["expected_output"]["full_product_bytes"] = json!(NISAR_SUBSET_MAX_SAFE_INTEGER);
            base["expected_output"]["estimated_remote_bytes"] =
                json!(NISAR_SUBSET_MAX_SAFE_INTEGER - 1);
            base
        }
        "same_prefix_different_end" => {
            let mut first = template.clone();
            first["line_start"] = json!(2);
            first["line_end_exclusive"] = json!(3);
            first["expected_shape"] = json!([1, 512]);
            let mut second = first.clone();
            second["line_end_exclusive"] = json!(4);
            second["expected_shape"] = json!([2, 512]);
            base["mapping"]["windows"] = json!([second, first]);
            base
        }
        "unicode_bmp_supplementary_order" => {
            let mut first = template.clone();
            first["dtype"] = json!("\u{ffff}");
            first["line_start"] = json!(2);
            first["line_end_exclusive"] = json!(3);
            first["expected_shape"] = json!([1, 512]);
            let mut second = template.clone();
            second["dtype"] = json!("\u{10000}");
            second["line_start"] = json!(10);
            second["line_end_exclusive"] = json!(11);
            second["expected_shape"] = json!([1, 512]);
            base["mapping"]["windows"] = json!([second, first]);
            base
        }
        "oversized_request" => {
            let swath = "s".repeat(128);
            base["request"]["swath"] = json!(swath.clone());
            base["request"]["aoi_component_count"] = json!(NISAR_SUBSET_MAX_COMPONENTS_FOR_TEST);
            base["mapping"]["windows"] = Value::Array(
                (0..NISAR_SUBSET_MAX_COMPONENTS_FOR_TEST)
                    .map(|component| {
                        let mut window = template.clone();
                        window["component_id"] = json!(component);
                        window["swath"] = json!(swath.clone());
                        window["dtype"] = json!("d".repeat(64));
                        window
                    })
                    .collect(),
            );
            base
        }
        "invalid_mapping_algorithm" => {
            base["mapping"]["algorithm"] = json!("other_algorithm_v1");
            base
        }
        "invalid_mapping_version" => {
            base["mapping"]["algorithm_version"] = json!(2);
            base
        }
        "negative_margin" => {
            base["mapping"]["windows"][0]["margin_lines"] = json!(-1);
            base
        }
        "artifact_invalid_polarization"
        | "mask_path_backslash"
        | "source_validator_specification_mismatch" => {
            let mut artifact = fixture("artifact-manifest-v1.json");
            match builder {
                "artifact_invalid_polarization" => {
                    artifact["dataset_windows"][0]["polarization"] = json!("VV")
                }
                "mask_path_backslash" => {
                    artifact["validity_masks"][0]["dataset_path"] =
                        json!("\\science\\inputDataExceptionMask")
                }
                _ => artifact["validator"]["product_specification_version"] = json!("1.5.1"),
            }
            artifact
        }
        other => panic!("unknown parity builder {other}"),
    }
}

#[test]
fn cross_language_canonical_parity_fixture_executes_all_required_edges() {
    let parity = fixture("canonical-parity-v1.json");
    for case in parity["valid_cases"].as_array().unwrap() {
        let manifest: NisarSubsetRequestManifestV1 =
            serde_json::from_value(build_parity_case(case["builder"].as_str().unwrap())).unwrap();
        assert_eq!(
            manifest.sha256().unwrap(),
            case["expected_sha256"].as_str().unwrap(),
            "{}",
            case["case"]
        );
    }
    for case in parity["invalid_cases"].as_array().unwrap() {
        let value = build_parity_case(case["builder"].as_str().unwrap());
        let invalid = match case["target"].as_str().unwrap() {
            "request" => invalid_request(value),
            "artifact" => invalid_artifact(value),
            other => panic!("unknown parity target {other}"),
        };
        assert!(invalid, "{}", case["case"]);
    }
}
