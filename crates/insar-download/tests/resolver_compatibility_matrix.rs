use std::fs;
use std::path::PathBuf;

use insar_core::SnapshotItem;
use insar_download::{
    classify_frozen_execution, route_frozen_execution, AsfCollectionProfileRegistry,
    AssetExecutionPolicyRegistry,
};
use serde_json::{json, Value};

fn cases() -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asset_execution_policy/legacy_compatibility_cases.json");
    let fixture: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    fixture["cases"].as_array().unwrap().clone()
}

fn item(case: &Value) -> SnapshotItem {
    let name = case["name"].as_str().unwrap();
    let mut value = json!({
        "item_id": format!("legacy-compat::{name}"),
        "provider_id": case["provider_id"],
        "display_name": format!("{name}.dat"),
        "metadata": {
            "dataset": case["dataset"],
            "processing_level": case["processing_level"],
            "identity_rule_id": if name == "unknown_profile_metadata" {
                "asf.future.unknown.v1"
            } else {
                "legacy.compatibility.v1"
            }
        },
        "payload": {
            "download_url": format!("https://example.invalid/{name}.dat"),
            "file_name": format!("{name}.dat")
        }
    });
    if case["policy"] == "legacy" {
        value["execution_policy"] = json!({
            "authority": "legacy",
            "policy_id": "legacy.v1",
            "collection_profile_id": case["collection_profile_id"]
        });
    }
    serde_json::from_value(value).unwrap()
}

#[test]
fn every_compatibility_class_executes_one_legacy_route_and_no_resolver_route() {
    for case in cases() {
        let name = case["name"].as_str().unwrap().to_string();
        let item = item(&case);
        let execution = classify_frozen_execution(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        );
        let mut legacy_calls = 0;
        let mut resolver_calls = 0;
        let mut invalid_calls = 0;
        let route = route_frozen_execution(
            execution,
            (),
            |_| {
                legacy_calls += 1;
                "legacy"
            },
            |_, _| {
                resolver_calls += 1;
                "resolver"
            },
            |_, _| {
                invalid_calls += 1;
                "invalid"
            },
        );

        assert_eq!(route, "legacy", "{name}");
        assert_eq!(legacy_calls, 1, "{name}");
        assert_eq!(resolver_calls, 0, "{name}");
        assert_eq!(invalid_calls, 0, "{name}");
    }
}
