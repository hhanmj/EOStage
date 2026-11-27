//! Freeze an explicitly selected local result into the existing task authority.
use super::gacos_import_worker::{FrozenGacosFile, FrozenGacosImport};
use super::gacos_service::project_gacos_result_parent_parameters;
use super::{ApplicationService, ApplicationServiceError, GacosRequestError};
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, TaskOperationDescriptor,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GacosCreateImportTaskRequest {
    pub task_id: String,
    pub source_request_task_id: String,
    pub source_request_item_id: String,
    pub source_paths: Vec<PathBuf>,
}
fn invalid() -> ApplicationServiceError {
    GacosRequestError::Invalid.into()
}
fn digest(value: &Value) -> Result<String, ApplicationServiceError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| invalid())?)
    ))
}
fn freeze_file(path: &Path) -> Result<FrozenGacosFile, ApplicationServiceError> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    let path = path.canonicalize().map_err(|_| invalid())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?
        .to_owned();
    let date_token = name
        .strip_suffix(".ztd.rsc")
        .or_else(|| name.strip_suffix(".ztd"))
        .or_else(|| name.strip_suffix(".ztd.tif"))
        .or_else(|| name.strip_suffix(".tif"))
        .ok_or_else(invalid)?;
    if date_token.len() != 8
        || !date_token.bytes().all(|byte| byte.is_ascii_digit())
        || chrono::NaiveDate::parse_from_str(date_token, "%Y%m%d").is_err()
    {
        return Err(invalid());
    }
    let mut file = std::fs::File::open(&path).map_err(|_| invalid())?;
    let metadata = file.metadata().map_err(|_| invalid())?;
    if !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > insar_provider::gacos_artifact::GACOS_MAX_ARTIFACT_BYTES
    {
        return Err(invalid());
    }
    if name.ends_with(".rsc")
        && metadata.len() > insar_provider::gacos_artifact::GACOS_MAX_RSC_HEADER_BYTES as u64
    {
        return Err(invalid());
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut size = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| invalid())?;
        if count == 0 {
            break;
        }
        size = size.checked_add(count as u64).ok_or_else(invalid)?;
        if size > metadata.len() {
            return Err(invalid());
        }
        hasher.update(&buffer[..count]);
    }
    if size != metadata.len() {
        return Err(invalid());
    }
    Ok(FrozenGacosFile {
        path,
        name,
        size_bytes: size,
        sha256: format!("{:x}", hasher.finalize()),
    })
}
impl ApplicationService {
    pub fn gacos_create_import_task(
        &self,
        request: &GacosCreateImportTaskRequest,
    ) -> Result<Value, ApplicationServiceError> {
        self.with_business_runtime(|_| Ok(()))?;
        if request.task_id.is_empty()
            || request.task_id.len() > 120
            || !request
                .task_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:".contains(&c))
            || request.task_id == request.source_request_task_id
            || request.source_paths.is_empty()
            || request.source_paths.len() > 40
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
        let mut files = request
            .source_paths
            .iter()
            .map(|path| freeze_file(path))
            .collect::<Result<Vec<_>, _>>()?;
        files.sort_by(|left, right| left.name.cmp(&right.name));
        if files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            != files.len()
        {
            return Err(invalid());
        }
        value["files"] = serde_json::to_value(files).map_err(|_| invalid())?;
        let manifest: FrozenGacosImport =
            serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        manifest.validate().map_err(|_| invalid())?;
        let fingerprint = digest(&value)?;
        let batch_id = format!("gacos-import-{}", fingerprint);
        let item_id = format!("gacos-result-{}", fingerprint);
        let created_at = parent.snapshot.created_at.clone();
        let output_root = parent.snapshot.output_root.join("gacos-results");
        if !output_root.is_absolute() {
            return Err(invalid());
        }
        let operation = TaskOperationDescriptor {
            operation_id: format!("{}:import", request.task_id), kind: "gacos".to_owned(),
            phase: "execute".to_owned(), region_id: parent.snapshot.batch_id.clone(),
            approved_worker_operation: "gacos_import_result".to_owned(),
            payload: json!({"source_request_task_id": request.source_request_task_id,
                "source_request_item_id": request.source_request_item_id, "manifest_sha256": fingerprint}).as_object().unwrap().clone(),
        };
        insar_core::validate_gacos_import_operation(&operation).map_err(|_| invalid())?;
        let item = SnapshotItem { item_id: item_id.clone(), provider_id: "gacos.local-result".to_owned(),
            display_name: "GACOS result import".to_owned(), acquisition_datetime: None,
            footprint_geojson: parent.snapshot.aoi_geojson.clone(),
            metadata: json!({"date_count": manifest.dates.len(), "requested_utc": {"hour": manifest.hour, "minute": manifest.minute}, "observed_utc": null}).as_object().unwrap().clone(),
            payload: json!({"gacos_import_manifest": value}).as_object().unwrap().clone(),
            asset_access: None, execution_policy: None };
        let batch = FrozenSearchBatch {
            schema_version: 1,
            batch_id: batch_id.clone(),
            provider_id: "gacos.local-result".to_owned(),
            created_at: created_at.clone(),
            aoi_name: parent.snapshot.aoi_name.clone(),
            aoi_geojson: parent.snapshot.aoi_geojson.clone(),
            query: json!({"gacos_import_manifest": value})
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
                "status": stored.runtime.status, "source": "native-gacos-local-import"}),
            )
        })
    }
}
