use std::fs;
use std::path::PathBuf;

use insar_core::{
    validate_frozen_task_snapshot_v3, validate_frozen_task_snapshot_v3_graph, FrozenTaskSnapshotV3,
    FrozenTaskSnapshotV3CompanionCoverageSubject, FrozenTaskSnapshotV3GraphError,
    FrozenTaskSnapshotV3GraphIdentity, FrozenTaskSnapshotV3GraphLocation,
    FrozenTaskSnapshotV3GraphReference, FrozenTaskSnapshotV3OrbitIssue,
    FrozenTaskSnapshotV3TransferGraphIssue, FrozenTaskSnapshotV3ValidationError,
};
use serde_json::{Map, Value};

const GRAPH_FIXTURES: [&str; 27] = [
    "single-sentinel1-poeorb-valid.json",
    "multiple-sentinel1-shared-poeorb-valid.json",
    "duplicate-source-id.json",
    "duplicate-transfer-id.json",
    "duplicate-association.json",
    "duplicate-selected-id.json",
    "duplicate-transfer-attachment.json",
    "missing-source.json",
    "missing-transfer-source.json",
    "orphan-orbit-association.json",
    "missing-association-transfer.json",
    "association-targets-primary.json",
    "wrong-requested-orbit-type.json",
    "wrong-matched-orbit-type.json",
    "wrong-platform.json",
    "unknown-source-platform.json",
    "invalid-source-interval.json",
    "invalid-time-coverage.json",
    "orbit-filename-mismatch.json",
    "association-set-mismatch.json",
    "unused-companion-transfer.json",
    "companion-required-false.json",
    "wrong-asset-role.json",
    "wrong-companion-role.json",
    "selected-product-reference-mismatch.json",
    "selected-source-missing-primary.json",
    "multiple-poeorb-associations.json",
];

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/m1-b1b-transfer-graph")
        .join(name)
}

fn read_fixture(name: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(fixture_path(name)).expect("fixture is readable"))
        .expect("fixture is JSON")
}

fn object<'a>(value: &'a Value, label: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{label} is an object"))
}

fn string_field<'a>(value: &'a Map<String, Value>, field: &str) -> &'a str {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{field} is a string"))
}

fn usize_field(value: &Map<String, Value>, field: &str) -> usize {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|number| usize::try_from(number).ok())
        .unwrap_or_else(|| panic!("{field} is a usize"))
}

fn value_field<'a>(value: &'a Map<String, Value>, field: &str) -> &'a Value {
    value
        .get(field)
        .unwrap_or_else(|| panic!("{field} is present"))
}

fn base_payload(base: &str) -> Value {
    let name = match base {
        "single" => "single-sentinel1-poeorb-valid.json",
        "shared" => "multiple-sentinel1-shared-poeorb-valid.json",
        other => panic!("unknown graph fixture base: {other}"),
    };
    read_fixture(name)
        .get("payload")
        .cloned()
        .expect("base fixture contains a payload")
}

fn set_value(payload: &mut Value, path: &str, value: Value) {
    *payload
        .pointer_mut(path)
        .unwrap_or_else(|| panic!("mutation path exists: {path}")) = value;
}

fn append_copy(payload: &mut Value, from: &str, path: &str) {
    let copied = payload
        .pointer(from)
        .unwrap_or_else(|| panic!("copy source exists: {from}"))
        .clone();
    payload
        .pointer_mut(path)
        .unwrap_or_else(|| panic!("copy target exists: {path}"))
        .as_array_mut()
        .unwrap_or_else(|| panic!("copy target is an array: {path}"))
        .push(copied);
}

fn apply_mutations(payload: &mut Value, fixture: &Value) {
    let Some(mutations) = fixture.get("mutations") else {
        return;
    };
    for mutation in mutations
        .as_array()
        .expect("fixture mutations are an array")
    {
        let mutation = object(mutation, "fixture mutation");
        match string_field(mutation, "op") {
            "set" => set_value(
                payload,
                string_field(mutation, "path"),
                value_field(mutation, "value").clone(),
            ),
            "append_copy" => append_copy(
                payload,
                string_field(mutation, "from"),
                string_field(mutation, "path"),
            ),
            other => panic!("unknown fixture mutation: {other}"),
        }
    }
}

fn fixture_payload(fixture: &Value) -> Value {
    if let Some(payload) = fixture.get("payload") {
        return payload.clone();
    }
    let fixture = object(fixture, "fixture");
    base_payload(string_field(fixture, "base"))
}

fn expected_location(value: &Value) -> FrozenTaskSnapshotV3GraphLocation {
    let value = object(value, "graph location");
    match string_field(value, "kind") {
        "source_item" => FrozenTaskSnapshotV3GraphLocation::SourceItem {
            index: usize_field(value, "index"),
        },
        "initial_selected_item_id" => FrozenTaskSnapshotV3GraphLocation::InitialSelectedItemId {
            index: usize_field(value, "index"),
        },
        "transfer_item" => FrozenTaskSnapshotV3GraphLocation::TransferItem {
            index: usize_field(value, "index"),
        },
        "transfer_source_item" => FrozenTaskSnapshotV3GraphLocation::TransferSourceItem {
            transfer_index: usize_field(value, "transfer_index"),
            source_index: usize_field(value, "source_index"),
        },
        "orbit_association" => FrozenTaskSnapshotV3GraphLocation::OrbitAssociation {
            index: usize_field(value, "index"),
        },
        other => panic!("unknown graph location: {other}"),
    }
}

fn expected_identity(value: &str) -> FrozenTaskSnapshotV3GraphIdentity {
    match value {
        "source_item_id" => FrozenTaskSnapshotV3GraphIdentity::SourceItemId,
        "transfer_item_id" => FrozenTaskSnapshotV3GraphIdentity::TransferItemId,
        "selected_item_id" => FrozenTaskSnapshotV3GraphIdentity::SelectedItemId,
        "transfer_source_item_id" => FrozenTaskSnapshotV3GraphIdentity::TransferSourceItemId,
        other => panic!("unknown graph identity: {other}"),
    }
}

fn expected_reference(value: &str) -> FrozenTaskSnapshotV3GraphReference {
    match value {
        "initial_selected_source" => FrozenTaskSnapshotV3GraphReference::InitialSelectedSource,
        "transfer_source" => FrozenTaskSnapshotV3GraphReference::TransferSource,
        "association_source" => FrozenTaskSnapshotV3GraphReference::AssociationSource,
        "association_orbit_transfer" => {
            FrozenTaskSnapshotV3GraphReference::AssociationOrbitTransfer
        }
        other => panic!("unknown graph reference: {other}"),
    }
}

fn expected_orbit_issue(value: &str) -> FrozenTaskSnapshotV3OrbitIssue {
    match value {
        "requested_orbit_type_not_poeorb" => {
            FrozenTaskSnapshotV3OrbitIssue::RequestedOrbitTypeNotPoeorb
        }
        "matched_orbit_type_not_poeorb" => {
            FrozenTaskSnapshotV3OrbitIssue::MatchedOrbitTypeNotPoeorb
        }
        "unknown_source_platform" => FrozenTaskSnapshotV3OrbitIssue::UnknownSourcePlatform,
        "platform_mismatch" => FrozenTaskSnapshotV3OrbitIssue::PlatformMismatch,
        "invalid_source_interval" => FrozenTaskSnapshotV3OrbitIssue::InvalidSourceInterval,
        "validity_not_covered" => FrozenTaskSnapshotV3OrbitIssue::ValidityNotCovered,
        "file_name_mismatch" => FrozenTaskSnapshotV3OrbitIssue::FileNameMismatch,
        other => panic!("unknown orbit issue: {other}"),
    }
}

fn expected_transfer_issue(value: &Value) -> FrozenTaskSnapshotV3TransferGraphIssue {
    let value = object(value, "transfer graph issue");
    match string_field(value, "kind") {
        "selected_source_set_mismatch" => {
            FrozenTaskSnapshotV3TransferGraphIssue::SelectedSourceSetMismatch
        }
        "selected_product_coverage" => {
            FrozenTaskSnapshotV3TransferGraphIssue::SelectedProductCoverage {
                source: expected_location(value_field(value, "source")),
                primary_transfer_count: usize_field(value, "primary_transfer_count"),
            }
        }
        "selected_product_source_count" => {
            FrozenTaskSnapshotV3TransferGraphIssue::SelectedProductSourceCount {
                transfer: expected_location(value_field(value, "transfer")),
                source_count: usize_field(value, "source_count"),
            }
        }
        "selected_product_reference_mismatch" => {
            FrozenTaskSnapshotV3TransferGraphIssue::SelectedProductReferenceMismatch {
                transfer: expected_location(value_field(value, "transfer")),
                source: expected_location(value_field(value, "source")),
            }
        }
        "selected_product_role_must_be_primary" => {
            FrozenTaskSnapshotV3TransferGraphIssue::SelectedProductRoleMustBePrimary {
                transfer: expected_location(value_field(value, "transfer")),
            }
        }
        "required_companion_role_must_be_auxiliary" => {
            FrozenTaskSnapshotV3TransferGraphIssue::RequiredCompanionRoleMustBeAuxiliary {
                transfer: expected_location(value_field(value, "transfer")),
            }
        }
        "required_companion_must_be_required" => {
            FrozenTaskSnapshotV3TransferGraphIssue::RequiredCompanionMustBeRequired {
                transfer: expected_location(value_field(value, "transfer")),
            }
        }
        "association_targets_non_companion" => {
            FrozenTaskSnapshotV3TransferGraphIssue::AssociationTargetsNonCompanion {
                association: expected_location(value_field(value, "association")),
                transfer: expected_location(value_field(value, "transfer")),
            }
        }
        "attachment_association_set_mismatch" => {
            FrozenTaskSnapshotV3TransferGraphIssue::AttachmentAssociationSetMismatch {
                transfer: expected_location(value_field(value, "transfer")),
            }
        }
        other => panic!("unknown transfer graph issue: {other}"),
    }
}

fn expected_companion_subject(value: &Value) -> FrozenTaskSnapshotV3CompanionCoverageSubject {
    let value = object(value, "companion coverage subject");
    match string_field(value, "kind") {
        "source_item" => FrozenTaskSnapshotV3CompanionCoverageSubject::SourceItem {
            source: expected_location(value_field(value, "source")),
        },
        "required_companion_transfer" => {
            FrozenTaskSnapshotV3CompanionCoverageSubject::RequiredCompanionTransfer {
                transfer: expected_location(value_field(value, "transfer")),
            }
        }
        other => panic!("unknown companion coverage subject: {other}"),
    }
}

fn expected_error(expected: &Value) -> FrozenTaskSnapshotV3GraphError {
    let error = object(
        expected
            .get("error")
            .expect("rejected fixture has an expected error"),
        "expected error",
    );
    match string_field(error, "variant") {
        "duplicate_identity" => FrozenTaskSnapshotV3GraphError::DuplicateIdentity {
            identity: expected_identity(string_field(error, "identity")),
            first: expected_location(value_field(error, "first")),
            duplicate: expected_location(value_field(error, "duplicate")),
        },
        "missing_reference" => FrozenTaskSnapshotV3GraphError::MissingReference {
            reference: expected_reference(string_field(error, "reference")),
            at: expected_location(value_field(error, "at")),
        },
        "invalid_orbit_match" => FrozenTaskSnapshotV3GraphError::InvalidOrbitMatch {
            association: expected_location(value_field(error, "association")),
            issue: expected_orbit_issue(string_field(error, "issue")),
        },
        "invalid_transfer_graph" => FrozenTaskSnapshotV3GraphError::InvalidTransferGraph {
            issue: expected_transfer_issue(value_field(error, "issue")),
        },
        "invalid_companion_coverage" => FrozenTaskSnapshotV3GraphError::InvalidCompanionCoverage {
            subject: expected_companion_subject(value_field(error, "subject")),
            association_count: usize_field(error, "association_count"),
        },
        other => panic!("unknown graph error variant: {other}"),
    }
}

fn valid_snapshot() -> FrozenTaskSnapshotV3 {
    let fixture = read_fixture("single-sentinel1-poeorb-valid.json");
    serde_json::from_value(fixture_payload(&fixture)).expect("valid fixture payload")
}

#[test]
fn frozen_task_snapshot_v3_graph_fixtures_match_complete_typed_errors() {
    for name in GRAPH_FIXTURES {
        let fixture = read_fixture(name);
        let expected = object(
            fixture
                .get("expected")
                .expect("fixture has expected result"),
            "fixture expected",
        );
        let accepted = expected
            .get("accepted")
            .and_then(Value::as_bool)
            .expect("fixture expected.accepted is boolean");
        let mut payload = fixture_payload(&fixture);
        apply_mutations(&mut payload, &fixture);
        let snapshot: FrozenTaskSnapshotV3 =
            serde_json::from_value(payload).expect("fixture has a V3 payload");
        let structurally_valid = expected
            .get("structurally_valid")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        match snapshot.validate() {
            Ok(()) if structurally_valid => {}
            Err(_) if !structurally_valid => {
                assert!(matches!(
                    validate_frozen_task_snapshot_v3(&snapshot),
                    Err(FrozenTaskSnapshotV3ValidationError::Structural)
                ));
            }
            Ok(()) => panic!("{name} unexpectedly passed structural validation"),
            Err(error) => panic!("{name} unexpectedly failed structural validation: {error}"),
        }

        match validate_frozen_task_snapshot_v3_graph(&snapshot) {
            Ok(()) if accepted => {}
            Ok(()) => panic!("{name} unexpectedly passed graph validation"),
            Err(error) if !accepted => assert_eq!(
                error,
                expected_error(&Value::Object(expected.clone())),
                "{name}"
            ),
            Err(error) => panic!("{name} unexpectedly rejected: {error:?}"),
        }
    }
}

#[test]
fn graph_errors_keep_only_safe_positions_and_omit_external_values() {
    let mut snapshot = valid_snapshot();
    let untrusted_id = "EXTERNAL-ID-MARKER://fixture.invalid/not-a-real-path?token=fixture";
    snapshot.transfer_items[0].transfer_item_id = untrusted_id.to_string();
    snapshot.transfer_items[1].transfer_item_id = untrusted_id.to_string();

    let error = validate_frozen_task_snapshot_v3_graph(&snapshot)
        .expect_err("duplicate external transfer ID is rejected");

    assert!(matches!(
        error,
        FrozenTaskSnapshotV3GraphError::DuplicateIdentity {
            identity: FrozenTaskSnapshotV3GraphIdentity::TransferItemId,
            first: FrozenTaskSnapshotV3GraphLocation::TransferItem { index: 0 },
            duplicate: FrozenTaskSnapshotV3GraphLocation::TransferItem { index: 1 },
        }
    ));
    assert_eq!(
        error.to_string(),
        "frozen V3 task graph validation failed: duplicate identity"
    );
    assert!(!format!("{error:?}").contains(untrusted_id));
    assert!(!error.to_string().contains(untrusted_id));
}

#[test]
fn unified_v3_validator_checks_structure_before_the_graph() {
    let mut snapshot = valid_snapshot();
    snapshot.source_items.clear();

    let error = validate_frozen_task_snapshot_v3(&snapshot)
        .expect_err("empty source items fail structural validation");

    assert!(matches!(
        error,
        FrozenTaskSnapshotV3ValidationError::Structural
    ));
}
