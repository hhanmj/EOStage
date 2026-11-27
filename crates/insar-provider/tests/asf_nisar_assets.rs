use std::{fs, path::PathBuf};

use insar_core::{provider_product_key, AssetRole, ContractError};
use insar_provider::asf::nisar::{
    NISAR_MAX_ASSET_GRAPH_JSON_BYTES, NISAR_MAX_DEDUPLICATED_ASSETS, NISAR_MAX_FILE_NAME_BYTES,
    NISAR_MAX_LOCATOR_BYTES, NISAR_MAX_MEDIA_TYPE_BYTES, NISAR_MAX_NATIVE_ROLES,
    NISAR_MAX_NATIVE_ROLE_BYTES, NISAR_MAX_RAW_RESOURCE_ENTRIES,
};
use insar_provider::asf::{classify_nisar_assets, NisarAssetGraph};
use insar_provider::catalog::{NISAR_NATIVE_COLLECTION_ID, NISAR_PROVIDER_ID};
use serde_json::{json, Value};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-nisar-l-p1")
}

fn load(name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(fixture_root().join(name)).expect("fixture exists"))
        .expect("fixture is valid JSON")
}

fn base_assets() -> Value {
    load("granule-provisional-multi-asset-valid.json")["granule"]["assets"].clone()
}

fn records_mut(assets: &mut Value) -> &mut Vec<Value> {
    assets["records"].as_array_mut().expect("asset records")
}

fn product_key() -> String {
    provider_product_key(
        NISAR_PROVIDER_ID,
        NISAR_NATIVE_COLLECTION_ID,
        "G9900000001-ASF",
    )
    .expect("product key")
}

fn classify(assets: &Value) -> Result<NisarAssetGraph, ContractError> {
    classify_nisar_assets(assets, &product_key())
}

fn assert_code(assets: &Value, code: &str) {
    assert_eq!(classify(assets).unwrap_err().code, code);
}

fn primary_record_mut(assets: &mut Value) -> &mut Value {
    records_mut(assets)
        .iter_mut()
        .find(|record| record["normalized_role"] == "primary")
        .expect("primary record")
}

fn qa_record_mut(assets: &mut Value) -> &mut Value {
    records_mut(assets)
        .iter_mut()
        .find(|record| {
            record["file_name"]
                .as_str()
                .is_some_and(|name| name.ends_with("_QA_STATS.h5"))
        })
        .expect("QA HDF5 record")
}

fn configure_binary_auxiliary(record: &mut Value, subtypes: Vec<String>) {
    record["file_name"] = json!("NISAR_SUPPORT.bin");
    record["media_type"] = json!("application/octet-stream");
    record["normalized_role"] = json!("auxiliary");
    record["native_evidence"] = json!({
        "archive_name": "NISAR_SUPPORT.bin",
        "product_match": false,
        "url_types": ["VIEW RELATED INFORMATION"],
        "url_subtypes": subtypes,
        "formats": ["BINARY"]
    });
    let mut native_roles = vec![
        "ARCHIVE:BINARY".to_owned(),
        "VIEW RELATED INFORMATION".to_owned(),
    ];
    native_roles.extend(
        record["native_evidence"]["url_subtypes"]
            .as_array()
            .expect("subtypes")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned),
    );
    record["native_roles"] = json!(native_roles);
    record["routes"] = json!([]);
    record.as_object_mut().unwrap().remove("size_bytes");
    record.as_object_mut().unwrap().remove("checksum");
}

fn set_primary_https_locator_bytes(assets: &mut Value, target: usize) {
    let primary = primary_record_mut(assets);
    let file_name = primary["file_name"].as_str().expect("file name");
    let prefix = "https://example.invalid/";
    let suffix = format!("/{file_name}");
    let padding = target
        .checked_sub(prefix.len() + suffix.len())
        .expect("target accommodates an absolute URL");
    primary["routes"][0]["locator"] = json!(format!("{prefix}{}{suffix}", "x".repeat(padding)));
}

fn set_primary_route_count(assets: &mut Value, target: usize) {
    let primary = primary_record_mut(assets);
    let file_name = primary["file_name"].as_str().expect("file name").to_owned();
    primary["routes"] = json!((0..target)
        .map(|index| {
            if index % 2 == 0 {
                json!({
                    "scheme": "https",
                    "locator": format!("https://example.invalid/nisar/{index:03}/{file_name}")
                })
            } else {
                json!({
                    "scheme": "s3",
                    "locator": format!("s3://example-invalid/nisar/{index:03}/{file_name}")
                })
            }
        })
        .collect::<Vec<_>>());
    let represented = records_mut(assets)
        .iter()
        .map(|record| {
            usize::from(!record["native_evidence"]["archive_name"].is_null())
                + record["routes"].as_array().map_or(0, Vec::len)
        })
        .sum::<usize>();
    assets["raw_resource_count"] = json!(represented);
}

fn add_binary_auxiliaries(assets: &mut Value, target_count: usize, locator_padding: usize) {
    let current = assets["records"].as_array().expect("records").len();
    for index in current..target_count {
        let file_name = format!("NISAR_AUX_{index:03}.bin");
        let routes = if locator_padding == 0 {
            json!([])
        } else {
            json!([{
                "scheme": "https",
                "locator": format!(
                    "https://example.invalid/{}/{file_name}",
                    "x".repeat(locator_padding)
                )
            }])
        };
        records_mut(assets).push(json!({
            "asset_id": format!("aux-{index:03}"),
            "native_roles": ["ARCHIVE:BINARY", "VIEW RELATED INFORMATION"],
            "native_evidence": {
                "archive_name": file_name,
                "product_match": false,
                "url_types": ["VIEW RELATED INFORMATION"],
                "url_subtypes": ["SCIENCE SUPPORT"],
                "formats": ["BINARY"]
            },
            "normalized_role": "auxiliary",
            "file_name": file_name,
            "media_type": "application/octet-stream",
            "routes": routes
        }));
    }
    let route_count = assets["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["routes"].as_array().map_or(0, Vec::len))
        .sum::<usize>();
    assets["raw_resource_count"] = json!(target_count + route_count);
}

#[test]
fn boundary_fixture_freezes_every_asset_graph_limit_and_required_case() {
    let matrix = load("asset-graph-boundaries.json");
    assert_eq!(
        matrix["limits"],
        json!({
            "raw_resource_entries": NISAR_MAX_RAW_RESOURCE_ENTRIES,
            "deduplicated_assets": NISAR_MAX_DEDUPLICATED_ASSETS,
            "native_roles_per_asset": NISAR_MAX_NATIVE_ROLES,
            "native_role_utf8_bytes": NISAR_MAX_NATIVE_ROLE_BYTES,
            "file_name_utf8_bytes": NISAR_MAX_FILE_NAME_BYTES,
            "media_type_utf8_bytes": NISAR_MAX_MEDIA_TYPE_BYTES,
            "stable_locator_utf8_bytes": NISAR_MAX_LOCATOR_BYTES,
            "metadata_assets_json_bytes": NISAR_MAX_ASSET_GRAPH_JSON_BYTES
        })
    );
    let ids = matrix["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    for required in [
        "raw-128-dedup-valid",
        "raw-129-rejected",
        "https-s3-duplicate-flood",
        "native-roles-8-valid",
        "native-roles-9-rejected",
        "native-role-128-valid",
        "native-role-129-rejected",
        "filename-512-valid",
        "filename-513-rejected",
        "media-type-128-valid",
        "media-type-129-rejected",
        "locator-4096-valid",
        "locator-4097-rejected",
        "metadata-json-near-limit",
        "metadata-json-over-limit",
        "assets-32-valid",
        "assets-33-rejected",
        "multiple-science-hdf5",
        "primary-qa-conflict",
        "unknown-hdf5-not-primary",
        "stable-sort",
        "post-dedup-overflow",
    ] {
        assert!(ids.contains(required), "{required}");
    }
}

#[test]
fn valid_graph_has_one_primary_deduplicated_routes_and_stable_sorting() {
    let assets = base_assets();
    let graph = classify(&assets).expect("base graph is valid");
    assert_eq!(
        graph
            .assets
            .iter()
            .filter(|asset| asset.role == AssetRole::Primary)
            .count(),
        1
    );
    assert_eq!(graph.assets.len(), 9);
    assert!(graph.serialized_metadata_bytes <= NISAR_MAX_ASSET_GRAPH_JSON_BYTES);
    assert_eq!(graph.metadata_assets.len(), graph.assets.len());
    assert_eq!(
        graph.routes["science-rslc-hdf5"]
            .as_array()
            .expect("primary routes")
            .len(),
        2
    );

    let mut reversed = assets.clone();
    records_mut(&mut reversed).reverse();
    assert_eq!(classify(&reversed).unwrap(), graph);

    let mut duplicated = assets.clone();
    let primary = primary_record_mut(&mut duplicated).clone();
    let mut https = primary.clone();
    https["routes"] = json!([primary["routes"][0].clone()]);
    let mut s3 = primary;
    s3["routes"] = json!([s3["routes"][1].clone()]);
    records_mut(&mut duplicated).retain(|record| record["normalized_role"] != "primary");
    records_mut(&mut duplicated).extend([https, s3]);
    duplicated["raw_resource_count"] = json!(14);
    let deduplicated = classify(&duplicated).expect("HTTPS/S3 alternate records deduplicate");
    assert_eq!(
        deduplicated
            .assets
            .iter()
            .filter(|asset| asset.role == AssetRole::Primary)
            .count(),
        1
    );
    assert_eq!(
        deduplicated.routes["science-rslc-hdf5"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let mut conflict = duplicated;
    let duplicate_primary = records_mut(&mut conflict)
        .iter_mut()
        .filter(|record| record["normalized_role"] == "primary")
        .nth(1)
        .expect("second primary route record");
    duplicate_primary["size_bytes"] = json!(99);
    assert_code(&conflict, "ASF_ASSET_CLASSIFICATION_INVALID");
}

#[test]
fn raw_resource_count_and_native_role_evidence_cannot_understate_or_conflict() {
    let mut understated = base_assets();
    understated["raw_resource_count"] =
        json!(understated["records"].as_array().expect("records").len());
    assert_code(&understated, "ASF_ASSET_CLASSIFICATION_INVALID");

    let mut conflicting_format = base_assets();
    primary_record_mut(&mut conflicting_format)["native_evidence"]["formats"] =
        json!(["HDF5", "PNG"]);
    assert_code(&conflicting_format, "ASF_ASSET_CLASSIFICATION_INVALID");

    let mut conflicting_metadata_role = base_assets();
    let metadata = records_mut(&mut conflicting_metadata_role)
        .iter_mut()
        .find(|record| record["asset_id"] == "iso-xml")
        .expect("ISO metadata");
    metadata["native_evidence"]["url_types"] = json!(["GET DATA"]);
    assert_code(
        &conflicting_metadata_role,
        "ASF_ASSET_CLASSIFICATION_INVALID",
    );

    let mut duplicate_id = base_assets();
    let browse = records_mut(&mut duplicate_id)
        .iter_mut()
        .find(|record| record["asset_id"] == "browse-main")
        .expect("Browse asset");
    browse["asset_id"] = json!("science-rslc-hdf5");
    assert_code(&duplicate_id, "ASF_ASSET_CLASSIFICATION_INVALID");

    let mut forbidden_native_role = base_assets();
    primary_record_mut(&mut forbidden_native_role)["native_roles"]
        .as_array_mut()
        .unwrap()
        .push(json!("S-BAND"));
    assert_code(&forbidden_native_role, "ASF_ASSET_CLASSIFICATION_INVALID");
}

#[test]
fn aggregate_route_resources_enforce_the_exact_128_entry_boundary() {
    let mut duplicate_flood = base_assets();
    set_primary_route_count(&mut duplicate_flood, 64);
    let graph = classify(&duplicate_flood).expect("64 alternate HTTPS routes remain one asset");
    assert_eq!(
        graph.routes["science-rslc-hdf5"].as_array().unwrap().len(),
        64
    );

    let mut edge = base_assets();
    set_primary_route_count(&mut edge, 117);
    assert_eq!(edge["raw_resource_count"], NISAR_MAX_RAW_RESOURCE_ENTRIES);
    assert!(classify(&edge).is_ok());

    let mut over = base_assets();
    set_primary_route_count(&mut over, 118);
    let primary = primary_record_mut(&mut over);
    let repeated = primary["routes"][0].clone();
    primary["routes"] = Value::Array(vec![repeated; 118]);
    over["raw_resource_count"] = json!(NISAR_MAX_RAW_RESOURCE_ENTRIES);
    assert_code(&over, "ASF_ASSET_GRAPH_LIMIT_EXCEEDED");
}

#[test]
fn missing_ambiguous_qa_conflict_and_unknown_hdf5_fail_closed() {
    let mut missing = base_assets();
    records_mut(&mut missing).retain(|record| record["normalized_role"] != "primary");
    assert_code(&missing, "ASF_PRIMARY_ASSET_MISSING");

    let mutation = load("ambiguous-primary-hdf5.json");
    let mut ambiguous = base_assets();
    records_mut(&mut ambiguous).push(mutation["mutation"]["asset"].clone());
    ambiguous["raw_resource_count"] = json!(15);
    assert_code(&ambiguous, "ASF_PRIMARY_ASSET_AMBIGUOUS");

    let mut qa_conflict = base_assets();
    let qa = qa_record_mut(&mut qa_conflict);
    qa["normalized_role"] = json!("primary");
    qa["native_evidence"]["product_match"] = json!(true);
    qa["native_evidence"]["url_types"] = json!(["GET DATA"]);
    assert_code(&qa_conflict, "ASF_ASSET_CLASSIFICATION_INVALID");

    let mut unknown = base_assets();
    records_mut(&mut unknown).push(json!({
        "asset_id": "unknown-hdf5",
        "native_roles": ["ARCHIVE:HDF5"],
        "native_evidence": {
            "archive_name": "NISAR_UNKNOWN.h5",
            "product_match": false,
            "url_types": [],
            "url_subtypes": [],
            "formats": ["HDF5"]
        },
        "normalized_role": "auxiliary",
        "file_name": "NISAR_UNKNOWN.h5",
        "media_type": "application/x-hdf5",
        "routes": []
    }));
    unknown["raw_resource_count"] = json!(14);
    assert_code(&unknown, "ASF_ASSET_CLASSIFICATION_INVALID");
}

#[test]
fn every_scalar_and_count_limit_accepts_the_edge_and_rejects_one_over() {
    let mut raw_edge = base_assets();
    raw_edge["raw_resource_count"] = json!(NISAR_MAX_RAW_RESOURCE_ENTRIES);
    assert!(classify(&raw_edge).is_ok());
    raw_edge["raw_resource_count"] = json!(NISAR_MAX_RAW_RESOURCE_ENTRIES + 1);
    assert_code(&raw_edge, "ASF_ASSET_GRAPH_LIMIT_EXCEEDED");

    let mut roles = base_assets();
    let record = records_mut(&mut roles)
        .iter_mut()
        .find(|record| record["asset_id"] == "qa-report")
        .expect("auxiliary record");
    configure_binary_auxiliary(
        record,
        (0..(NISAR_MAX_NATIVE_ROLES - 2))
            .map(|index| format!("SCIENCE SUPPORT {index}"))
            .collect(),
    );
    assert!(classify(&roles).is_ok());
    let record = records_mut(&mut roles)
        .iter_mut()
        .find(|record| record["asset_id"] == "qa-report")
        .unwrap();
    configure_binary_auxiliary(
        record,
        (0..(NISAR_MAX_NATIVE_ROLES - 1))
            .map(|index| format!("SCIENCE SUPPORT {index}"))
            .collect(),
    );
    assert_code(&roles, "ASF_ASSET_GRAPH_LIMIT_EXCEEDED");

    let mut role_bytes = base_assets();
    let record = records_mut(&mut role_bytes)
        .iter_mut()
        .find(|record| record["asset_id"] == "qa-report")
        .unwrap();
    configure_binary_auxiliary(record, vec!["R".repeat(NISAR_MAX_NATIVE_ROLE_BYTES)]);
    assert!(classify(&role_bytes).is_ok());
    let record = records_mut(&mut role_bytes)
        .iter_mut()
        .find(|record| record["asset_id"] == "qa-report")
        .unwrap();
    configure_binary_auxiliary(record, vec!["R".repeat(NISAR_MAX_NATIVE_ROLE_BYTES + 1)]);
    assert_code(&role_bytes, "ASF_ASSET_GRAPH_LIMIT_EXCEEDED");

    for (bytes, expected_ok) in [
        (NISAR_MAX_FILE_NAME_BYTES, true),
        (NISAR_MAX_FILE_NAME_BYTES + 1, false),
    ] {
        let mut value = base_assets();
        let metadata = records_mut(&mut value)
            .iter_mut()
            .find(|record| record["asset_id"] == "iso-xml")
            .unwrap();
        let name = format!("{}.xml", "f".repeat(bytes - 4));
        metadata["file_name"] = json!(name);
        metadata["native_evidence"]["archive_name"] = metadata["file_name"].clone();
        if expected_ok {
            assert!(classify(&value).is_ok());
        } else {
            assert_code(&value, "ASF_ASSET_GRAPH_LIMIT_EXCEEDED");
        }
    }

    for (bytes, expected_ok) in [
        (NISAR_MAX_MEDIA_TYPE_BYTES, true),
        (NISAR_MAX_MEDIA_TYPE_BYTES + 1, false),
    ] {
        let mut value = base_assets();
        let record = records_mut(&mut value)
            .iter_mut()
            .find(|record| record["asset_id"] == "qa-report")
            .unwrap();
        record["file_name"] = json!("NISAR_SUPPORT.bin");
        record["media_type"] = json!(format!("application/{}", "x".repeat(bytes - 12)));
        record["native_evidence"] = json!({
            "archive_name": "NISAR_SUPPORT.bin",
            "product_match": false,
            "url_types": ["VIEW RELATED INFORMATION"],
            "url_subtypes": ["SCIENCE SUPPORT"],
            "formats": ["BINARY"]
        });
        record["native_roles"] = json!([
            "ARCHIVE:BINARY",
            "VIEW RELATED INFORMATION",
            "SCIENCE SUPPORT"
        ]);
        if expected_ok {
            assert!(classify(&value).is_ok());
        } else {
            assert_code(&value, "ASF_ASSET_GRAPH_LIMIT_EXCEEDED");
        }
    }

    let mut locator = base_assets();
    set_primary_https_locator_bytes(&mut locator, NISAR_MAX_LOCATOR_BYTES);
    assert!(classify(&locator).is_ok());
    set_primary_https_locator_bytes(&mut locator, NISAR_MAX_LOCATOR_BYTES + 1);
    assert_code(&locator, "ASF_ASSET_GRAPH_LIMIT_EXCEEDED");

    let mut assets_32 = base_assets();
    add_binary_auxiliaries(&mut assets_32, NISAR_MAX_DEDUPLICATED_ASSETS, 0);
    assert_eq!(classify(&assets_32).unwrap().assets.len(), 32);
    add_binary_auxiliaries(&mut assets_32, NISAR_MAX_DEDUPLICATED_ASSETS + 1, 0);
    assert_code(&assets_32, "ASF_ASSET_GRAPH_LIMIT_EXCEEDED");
}

#[test]
fn metadata_graph_has_a_legal_near_limit_case_and_rejects_the_next_larger_plan() {
    let mut near = None;
    let mut over = None;
    for padding in (100..=3_500).step_by(100) {
        let mut candidate = base_assets();
        add_binary_auxiliaries(&mut candidate, NISAR_MAX_DEDUPLICATED_ASSETS, padding);
        match classify(&candidate) {
            Ok(graph) if graph.serialized_metadata_bytes >= 38_912 => {
                near = Some((candidate, graph));
            }
            Ok(_) => {}
            Err(error) if error.code == "ASF_ASSET_GRAPH_LIMIT_EXCEEDED" => {
                over = Some(candidate);
                break;
            }
            Err(error) => panic!("unexpected boundary error: {error}"),
        }
    }
    let (_, near) = near.expect("a legal graph exists near the 40 KiB limit");
    assert!((38_912..=NISAR_MAX_ASSET_GRAPH_JSON_BYTES).contains(&near.serialized_metadata_bytes));
    assert_code(
        &over.expect("the next larger generated graph is rejected"),
        "ASF_ASSET_GRAPH_LIMIT_EXCEEDED",
    );
}

#[test]
fn unsafe_private_or_transient_routes_are_rejected_and_never_truncated() {
    for locator in [
        "http://example.invalid/nisar/file.h5",
        "https://user@example.invalid/nisar/file.h5",
        "https://localhost/nisar/file.h5",
        "https://127.0.0.1/nisar/file.h5",
        "https://unreviewed.example/nisar/file.h5",
        "https://example.invalid/private/file.h5",
        "https://bhoonidhi.example.invalid/nisar/file.h5",
        "https://example.invalid/nisar/file.h5?token=secret",
        "https://example.invalid/nisar/file.h5#fragment",
    ] {
        let mut value = base_assets();
        primary_record_mut(&mut value)["routes"][0] =
            json!({"scheme": "https", "locator": locator});
        assert_code(&value, "ASF_ASSET_CLASSIFICATION_INVALID");
    }
}
