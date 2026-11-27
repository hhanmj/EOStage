//! Bind a manually supplied delivery link to a frozen request and the existing Store.
use super::gacos_result_worker::{FrozenGacosResult, GacosResultError};
use super::gacos_service::project_gacos_result_parent_parameters;
use super::{ApplicationService, ApplicationServiceError, GacosRequestError};
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, TaskOperationDescriptor,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GacosCreateResultTaskRequest {
    pub task_id: String,
    pub source_request_task_id: String,
    pub source_request_item_id: String,
    pub result_url: String,
}
fn invalid() -> ApplicationServiceError {
    GacosRequestError::Invalid.into()
}
fn link_error(error: GacosResultError) -> ApplicationServiceError {
    match error {
        GacosResultError::UnreviewedLinkHost => GacosRequestError::ResultLinkHostUnreviewed,
        GacosResultError::LinkHashMismatch | GacosResultError::LinkConflict => {
            GacosRequestError::ResultLinkMismatch
        }
        GacosResultError::CredentialUnavailable => {
            GacosRequestError::ResultLinkCredentialUnavailable
        }
        _ => GacosRequestError::ResultLinkInvalid,
    }
    .into()
}
fn digest(value: &Value) -> Result<String, ApplicationServiceError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| invalid())?)
    ))
}
impl ApplicationService {
    pub fn gacos_create_result_task(
        &self,
        request: &GacosCreateResultTaskRequest,
    ) -> Result<Value, ApplicationServiceError> {
        self.with_business_runtime(|_| Ok(()))?;
        if request.task_id.is_empty()
            || request.task_id.len() > 120
            || !request
                .task_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
            || request.task_id == request.source_request_task_id
        {
            return Err(invalid());
        }
        let parent = self.with_business_runtime(|runtime| {
            let store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            store
                .load_task_record(&request.source_request_task_id)?
                .ok_or_else(invalid)
        })?;
        let operation = parent.snapshot.operation.as_ref().ok_or_else(invalid)?;
        insar_core::validate_gacos_manual_request_operation(operation).map_err(|_| invalid())?;
        if !parent
            .snapshot
            .initial_selected_item_ids
            .contains(&request.source_request_item_id)
        {
            return Err(invalid());
        }
        let source = parent
            .snapshot
            .candidates
            .iter()
            .find(|item| item.item_id == request.source_request_item_id)
            .ok_or_else(invalid)?;
        if source.provider_id != "gacos.webform" {
            return Err(invalid());
        }
        let parent_parameters = project_gacos_result_parent_parameters(&source.payload)?;
        let mut value = json!({
            "schema_version": 1,
            "source_request_task_id": request.source_request_task_id,
            "source_request_item_id": request.source_request_item_id,
            "bbox": parent_parameters.bbox,
            "dates": parent_parameters.dates,
            "hour": parent_parameters.hour,
            "minute": parent_parameters.minute,
        });
        let (result_link_sha256, result_file_name) = self
            .gacos_result_links
            .insert(&request.result_url)
            .map_err(link_error)?;
        value["result_link_sha256"] = json!(result_link_sha256);
        value["result_file_name"] = json!(result_file_name);
        let manifest: FrozenGacosResult =
            serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        manifest.validate().map_err(|_| invalid())?;
        let fingerprint = digest(&value)?;
        let batch_id = format!("gacos-fetch-{}", fingerprint);
        let item_id = format!("gacos-delivery-{}", fingerprint);
        let created_at = parent.snapshot.created_at.clone();
        let output_root = parent.snapshot.output_root.join("gacos-results");
        if !output_root.is_absolute() {
            return Err(invalid());
        }
        let operation = TaskOperationDescriptor {
            operation_id: format!("{}:fetch", request.task_id), kind: "gacos".to_owned(),
            phase: "execute".to_owned(), region_id: parent.snapshot.batch_id.clone(),
            approved_worker_operation: "gacos_fetch_result".to_owned(),
            payload: json!({"source_request_task_id": request.source_request_task_id,
                "source_request_item_id": request.source_request_item_id, "manifest_sha256": fingerprint}).as_object().unwrap().clone(),
        };
        insar_core::validate_gacos_result_operation(&operation).map_err(|_| invalid())?;
        let item = SnapshotItem { item_id: item_id.clone(), provider_id: "gacos.manual-result".to_owned(),
            display_name: "GACOS result download".to_owned(), acquisition_datetime: None,
            footprint_geojson: parent.snapshot.aoi_geojson.clone(),
            metadata: json!({"date_count": manifest.dates.len(), "requested_utc": {"hour": manifest.hour, "minute": manifest.minute}, "observed_utc": null}).as_object().unwrap().clone(),
            payload: json!({"gacos_result_manifest": value}).as_object().unwrap().clone(),
            asset_access: None, execution_policy: None };
        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: batch_id.clone(),
            provider_id: "gacos.manual-result".to_owned(),
            created_at: created_at.clone(),
            aoi_name: parent.snapshot.aoi_name.clone(),
            aoi_geojson: parent.snapshot.aoi_geojson.clone(),
            query: json!({"gacos_result_manifest": value})
                .as_object()
                .unwrap()
                .clone(),
            search_stats: None,
            candidates: vec![item],
        };
        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;
            // Recheck the immutable parent after hashing, including history tombstones.
            let current = store
                .load_task_record(&request.source_request_task_id)?
                .ok_or_else(invalid)?;
            if current.snapshot != parent.snapshot {
                return Err(invalid());
            }
            let task_created_at = store
                .load_task_record(&request.task_id)?
                .map(|task| task.snapshot.created_at)
                .unwrap_or_else(|| {
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                });
            let task = StoreCreateTaskRequest {
                task_id: request.task_id.clone(),
                batch_id: batch_id.clone(),
                selected_item_ids: vec![item_id.clone()],
                checked_item_ids: vec![item_id.clone()],
                page_index: 0,
                page_size: 1,
                output_root: output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: task_created_at,
            };
            let batch_request = StoreSearchBatchRequest {
                batch,
                workbench: SearchWorkbenchState {
                    batch_id,
                    checked_item_ids: vec![item_id],
                    page_index: 0,
                    page_size: 1,
                    revision: 0,
                },
                activate_workbench: false,
            };
            let stored = store.save_search_batch_and_create_task_with_projection(
                &batch_request,
                &task,
                Some(&operation),
                None,
            )?;
            Ok(
                json!({"ok": true, "task_id": stored.snapshot.task_id, "output_root": output_root,
                "status": stored.runtime.status, "source": "native-gacos-manual-result"}),
            )
        })
    }
}

/// Re-supply the same delivery link after restart; never mutate a frozen locator identity.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GacosSupplyResultLinkRequest {
    pub task_id: String,
    pub result_url: String,
}
impl ApplicationService {
    pub fn gacos_supply_result_link(
        &self,
        request: &GacosSupplyResultLinkRequest,
    ) -> Result<Value, ApplicationServiceError> {
        self.with_business_runtime(|runtime| {
            let store = runtime.store.lock().map_err(|_| ApplicationServiceError::StatePoisoned)?;
            let task = store.load_task_record(&request.task_id)?.ok_or_else(invalid)?;
            let operation = task.snapshot.operation.as_ref().ok_or_else(invalid)?;
            insar_core::validate_gacos_result_operation(operation).map_err(|_| invalid())?;
            if task.snapshot.initial_selected_item_ids.len() != 1 {
                return Err(invalid());
            }
            let item = task.snapshot.candidates.iter()
                .find(|item| task.snapshot.initial_selected_item_ids.contains(&item.item_id))
                .ok_or_else(invalid)?;
            let value = item.payload.get("gacos_result_manifest").ok_or_else(invalid)?;
            if operation.payload.get("manifest_sha256").and_then(Value::as_str)
                != Some(digest(value)?.as_str()) {
                return Err(invalid());
            }
            let manifest: FrozenGacosResult = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
            manifest.validate().map_err(|_| invalid())?;
            self.gacos_result_links.supply(&manifest.result_link_sha256, &request.result_url)
                .map_err(link_error)?;
            Ok(json!({"ok": true, "task_id": request.task_id, "source": "native-gacos-manual-result"}))
        })
    }
}
