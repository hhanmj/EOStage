use std::fs;
use std::path::PathBuf;

use insar_core::SnapshotItem;
use insar_download::{
    evaluate_resolver_cutover_eligibility, AsfCollectionProfileRegistry,
    AssetExecutionPolicyRegistry, EligibilityReason, ResolverCutoverEligibility,
};
use serde_json::{Map, Value};

struct Fixture {
    base_item: Value,
    cases: Vec<Case>,
}

struct Case {
    name: String,
    mutations: Map<String, Value>,
    expected: String,
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asset_execution_policy/sentinel1_standard_cases.json");
    let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let object = value.as_object().expect("fixture root must be an object");
    let cases = object["cases"]
        .as_array()
        .expect("fixture cases must be an array")
        .iter()
        .map(|case| {
            let case = case.as_object().expect("fixture case must be an object");
            Case {
                name: case["name"].as_str().unwrap().to_string(),
                mutations: case["mutations"].as_object().unwrap().clone(),
                expected: case["expected"].as_str().unwrap().to_string(),
            }
        })
        .collect();
    Fixture {
        base_item: object["base_item"].clone(),
        cases,
    }
}

fn apply_mutation(root: &mut Value, path: &str, value: Value) {
    let mut parts = path.split('.').peekable();
    let mut current = root;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current
                .as_object_mut()
                .expect("mutation parent must be an object")
                .insert(part.to_string(), value);
            return;
        }
        current = current
            .get_mut(part)
            .unwrap_or_else(|| panic!("missing mutation path: {path}"));
    }
}

fn classified(result: ResolverCutoverEligibility) -> String {
    match result {
        ResolverCutoverEligibility::Eligible(_) => "eligible".to_string(),
        ResolverCutoverEligibility::Ineligible(reason) => {
            format!("ineligible:{}", reason.code())
        }
        ResolverCutoverEligibility::Invalid(_) => "invalid".to_string(),
    }
}

#[test]
fn reviewed_sentinel1_policy_matrix_is_exact() {
    let fixture = fixture();
    let profiles = AsfCollectionProfileRegistry;
    let policies = AssetExecutionPolicyRegistry;

    for case in fixture.cases {
        let mut value = fixture.base_item.clone();
        for (path, mutation) in case.mutations {
            apply_mutation(&mut value, &path, mutation);
        }
        let item: SnapshotItem = serde_json::from_value(value).unwrap();
        let actual = evaluate_resolver_cutover_eligibility(&item, &profiles, &policies);
        assert_eq!(classified(actual), case.expected, "{}", case.name);
    }
}

#[test]
fn raw_is_not_misclassified_as_a_standard_resolver_product() {
    let fixture = fixture();
    let case = fixture
        .cases
        .iter()
        .find(|case| case.name == "raw")
        .unwrap();
    let mut value = fixture.base_item;
    for (path, mutation) in &case.mutations {
        apply_mutation(&mut value, path, mutation.clone());
    }
    let item: SnapshotItem = serde_json::from_value(value).unwrap();

    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Ineligible(EligibilityReason::ProcessingLevel)
    ));
}

#[test]
fn tampered_resolver_policy_is_invalid_not_legacy() {
    let fixture = fixture();
    let case = fixture
        .cases
        .iter()
        .find(|case| case.name == "tampered_policy")
        .unwrap();
    let mut value = fixture.base_item;
    for (path, mutation) in &case.mutations {
        apply_mutation(&mut value, path, mutation.clone());
    }
    let item: SnapshotItem = serde_json::from_value(value).unwrap();

    assert!(matches!(
        evaluate_resolver_cutover_eligibility(
            &item,
            &AsfCollectionProfileRegistry,
            &AssetExecutionPolicyRegistry,
        ),
        ResolverCutoverEligibility::Invalid(_)
    ));
}
