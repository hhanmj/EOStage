use insar_core::{validate_gacos_manual_request_operation, TaskOperationDescriptor};
use serde_json::{Map, Value};

fn operation(
    kind: &str,
    phase: &str,
    worker: &str,
    payload: Map<String, Value>,
) -> TaskOperationDescriptor {
    TaskOperationDescriptor {
        operation_id: "gacos-attempt-1".to_owned(),
        kind: kind.to_owned(),
        phase: phase.to_owned(),
        region_id: "region-1".to_owned(),
        approved_worker_operation: worker.to_owned(),
        payload,
    }
}

#[test]
fn gacos_manual_request_requires_the_exact_operation_tuple() {
    let valid = operation("gacos", "request", "gacos_manual_request", Map::new());
    assert!(valid.validate().is_ok());
    assert!(validate_gacos_manual_request_operation(&valid).is_ok());

    for (kind, phase, worker) in [
        ("dem", "request", "gacos_manual_request"),
        ("gacos", "download", "gacos_manual_request"),
        ("gacos", "request", "dem_download"),
    ] {
        let candidate = operation(kind, phase, worker, Map::new());
        assert!(
            candidate.validate().is_ok(),
            "each independently declared enum value should remain valid"
        );
        assert!(validate_gacos_manual_request_operation(&candidate).is_err());
    }
}

#[test]
fn existing_operation_tuples_remain_accepted_by_generated_validation() {
    for (kind, phase, worker) in [
        ("dem", "execute", "dem_prepare_sarscape"),
        ("dem", "download", "dem_download"),
        ("orbit", "download", "orbit_download"),
    ] {
        assert!(operation(kind, phase, worker, Map::new())
            .validate()
            .is_ok());
    }
}

#[test]
fn gacos_manual_request_rejects_personal_mutable_and_sensitive_payload_keys() {
    for key in [
        "email",
        "submission_state",
        "password",
        "token",
        "cookie",
        "authorization",
        "secret",
    ] {
        let mut payload = Map::new();
        payload.insert(key.to_owned(), Value::String("placeholder".to_owned()));
        let candidate = operation("gacos", "request", "gacos_manual_request", payload);
        assert!(validate_gacos_manual_request_operation(&candidate).is_err());
    }
}

#[test]
fn gacos_import_and_manual_submission_authorities_are_not_interchangeable() {
    let payload = serde_json::json!({"source_request_task_id":"parent","source_request_item_id":"item","manifest_sha256":"a".repeat(64)}).as_object().unwrap().clone();
    let valid = operation("gacos", "execute", "gacos_import_result", payload.clone());
    assert!(insar_core::validate_gacos_import_operation(&valid).is_ok());
    assert!(validate_gacos_manual_request_operation(&valid).is_err());
    for (kind, phase, worker) in [
        ("dem", "execute", "gacos_import_result"),
        ("gacos", "request", "gacos_import_result"),
        ("gacos", "execute", "gacos_manual_request"),
    ] {
        assert!(insar_core::validate_gacos_import_operation(&operation(
            kind,
            phase,
            worker,
            payload.clone()
        ))
        .is_err());
    }
    for key in [
        "source_request_task_id",
        "source_request_item_id",
        "manifest_sha256",
    ] {
        let mut missing = valid.clone();
        missing.payload.remove(key);
        assert!(insar_core::validate_gacos_import_operation(&missing).is_err());
        for value in ["", " ", "\t", " padded", "padded "] {
            let mut invalid = valid.clone();
            invalid
                .payload
                .insert(key.to_owned(), Value::String(value.to_owned()));
            assert!(insar_core::validate_gacos_import_operation(&invalid).is_err());
        }
    }
}

#[test]
fn manually_supplied_result_fetch_cannot_authorize_form_post_or_persist_links() {
    let payload = serde_json::json!({"source_request_task_id":"parent","source_request_item_id":"item","manifest_sha256":"b".repeat(64)}).as_object().unwrap().clone();
    let valid = operation("gacos", "execute", "gacos_fetch_result", payload.clone());
    assert!(insar_core::validate_gacos_result_operation(&valid).is_ok());
    assert!(insar_core::validate_gacos_import_operation(&valid).is_err());
    assert!(validate_gacos_manual_request_operation(&valid).is_err());
    for (kind, phase, worker) in [
        ("dem", "execute", "gacos_fetch_result"),
        ("gacos", "request", "gacos_fetch_result"),
        ("gacos", "execute", "gacos_manual_request"),
        ("gacos", "execute", "gacos_import_result"),
    ] {
        assert!(insar_core::validate_gacos_result_operation(&operation(
            kind,
            phase,
            worker,
            payload.clone()
        ))
        .is_err());
    }
    for key in ["result_url", "email", "submission_state", "token", "path"] {
        let mut candidate = valid.clone();
        candidate.payload.insert(
            key.to_owned(),
            Value::String("private-placeholder".to_owned()),
        );
        assert!(insar_core::validate_gacos_result_operation(&candidate).is_err());
    }
    for key in [
        "source_request_task_id",
        "source_request_item_id",
        "manifest_sha256",
    ] {
        let mut missing = valid.clone();
        missing.payload.remove(key);
        assert!(insar_core::validate_gacos_result_operation(&missing).is_err());
        let mut blank = valid.clone();
        blank
            .payload
            .insert(key.to_owned(), Value::String(" ".to_owned()));
        assert!(insar_core::validate_gacos_result_operation(&blank).is_err());
    }
}
