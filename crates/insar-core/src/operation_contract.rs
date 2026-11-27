use crate::generated::TaskOperationDescriptor;

const GACOS_KIND: &str = "gacos";
const GACOS_REQUEST_PHASE: &str = "request";
const GACOS_MANUAL_REQUEST_WORKER: &str = "gacos_manual_request";

/// Validate the exact operation tuple used for a GACOS manual request.
///
/// The generated validator checks the three fields independently. This entry point
/// adds the product-owned tuple constraint and reuses the generated payload privacy
/// validation without freezing the future request envelope.
pub fn validate_gacos_manual_request_operation(
    operation: &TaskOperationDescriptor,
) -> Result<(), String> {
    operation.validate()?;
    if operation.kind != GACOS_KIND
        || operation.phase != GACOS_REQUEST_PHASE
        || operation.approved_worker_operation != GACOS_MANUAL_REQUEST_WORKER
    {
        return Err(
            "GACOS manual request operation must use the declared kind, phase, and worker"
                .to_owned(),
        );
    }
    if operation.payload.keys().any(|key| {
        key.eq_ignore_ascii_case("email") || key.eq_ignore_ascii_case("submission_state")
    }) {
        return Err("GACOS manual request payload contains a forbidden field".to_owned());
    }
    Ok(())
}

/// A local import is schedulable; a provider form submission is never schedulable.
pub fn validate_gacos_import_operation(operation: &TaskOperationDescriptor) -> Result<(), String> {
    operation.validate()?;
    if operation.kind != GACOS_KIND
        || operation.phase != "execute"
        || operation.approved_worker_operation != "gacos_import_result"
    {
        return Err("GACOS import operation tuple is invalid".to_owned());
    }
    validate_gacos_result_identity(operation)
}

/// Download a manually supplied result; this never authorizes a web-form POST.
pub fn validate_gacos_result_operation(operation: &TaskOperationDescriptor) -> Result<(), String> {
    operation.validate()?;
    if operation.kind != GACOS_KIND
        || operation.phase != "execute"
        || operation.approved_worker_operation != "gacos_fetch_result"
    {
        return Err("GACOS result operation tuple is invalid".to_owned());
    }
    if operation.payload.len() != 3 {
        return Err("GACOS result operation contains unsupported fields".to_owned());
    }
    validate_gacos_result_identity(operation)
}

fn validate_gacos_result_identity(operation: &TaskOperationDescriptor) -> Result<(), String> {
    for key in [
        "source_request_task_id",
        "source_request_item_id",
        "manifest_sha256",
    ] {
        if operation
            .payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .is_none_or(|value| value.trim().is_empty() || value.trim() != value)
        {
            return Err("GACOS import source identity is missing".to_owned());
        }
    }
    let fingerprint = operation.payload["manifest_sha256"]
        .as_str()
        .unwrap_or_default();
    if fingerprint.len() != 64
        || !fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("GACOS import manifest hash is invalid".to_owned());
    }
    if operation.payload.keys().any(|key| {
        key.eq_ignore_ascii_case("email") || key.eq_ignore_ascii_case("submission_state")
    }) {
        return Err("GACOS import payload contains a forbidden field".to_owned());
    }
    Ok(())
}
