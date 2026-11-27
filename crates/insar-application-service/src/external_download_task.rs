//! Persist completed open-geodata / auxiliary downloads into the unified TaskStore
//! so Download Center (`task_overview`), folder actions (`resolve_task_output_directory`),
//! duplicate-download prevention, and usage statistics all share one source of truth.

use std::path::PathBuf;

use chrono::{SecondsFormat, Utc};
use insar_core::{
    DownloadItemResult, DownloadLayout, DownloadOutcome, FrozenSearchBatch, SearchWorkbenchState,
    SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_store::{StoreTaskHistoryPageRequest, StoreTaskTransferTelemetryUpdate};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::{ApplicationService, ApplicationServiceError};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordExternalDownloadTaskRequest {
    pub dataset: String,
    pub provider_label: String,
    pub item_id: String,
    pub display_name: String,
    pub acquisition_datetime: Option<String>,
    pub bbox: Option<[f64; 4]>,
    pub output_root: PathBuf,
    pub saved_file_path: PathBuf,
    pub bytes_written: u64,
    pub elapsed_millis: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedExternalDownloadTask {
    pub task_id: String,
    pub revision: u64,
    pub output_root: PathBuf,
    pub already_recorded: bool,
}

fn sanitize_scalar(raw: &str, fallback: &str, max_len: usize) -> String {
    let mut cleaned = String::with_capacity(raw.len().min(max_len));
    for ch in raw.trim().chars() {
        if cleaned.len() >= max_len {
            break;
        }
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            cleaned.push(ch);
        } else if ch.is_whitespace() || ch == '/' || ch == ':' {
            cleaned.push('_');
        }
    }
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn build_bbox_polygon_geojson(bbox: Option<[f64; 4]>) -> Map<String, Value> {
    let [b0, b1, b2, b3] = bbox.unwrap_or([117.0, 31.0, 117.5, 31.5]);
    let min_lon = b0.min(b2).clamp(-180.0, 179.999);
    let max_lon = b0.max(b2).max(min_lon + 1e-4).clamp(-179.999, 180.0);
    let min_lat = b1.min(b3).clamp(-90.0, 89.999);
    let max_lat = b1.max(b3).max(min_lat + 1e-4).clamp(-89.999, 90.0);
    json!({
        "type": "Polygon",
        "coordinates": [[
            [min_lon, min_lat],
            [max_lon, min_lat],
            [max_lon, max_lat],
            [min_lon, max_lat],
            [min_lon, min_lat]
        ]]
    })
    .as_object()
    .cloned()
    .unwrap_or_default()
}

impl ApplicationService {
    pub fn record_external_download_task(
        &self,
        request: &RecordExternalDownloadTaskRequest,
    ) -> Result<RecordedExternalDownloadTask, ApplicationServiceError> {
        self.startup()?;
        if !request.output_root.is_absolute()
            || !request.saved_file_path.is_absolute()
            || request.bytes_written == 0
        {
            return Err(ApplicationServiceError::InvalidRequest {
                reason:
                    "external download output_root/saved_file_path must be absolute and non-empty"
                        .to_owned(),
            });
        }

        let clean_dataset = sanitize_scalar(&request.dataset, "open_geodata", 48);
        let provider_id = format!("open_geodata.{clean_dataset}");
        let item_id = sanitize_scalar(&request.item_id, &clean_dataset, 112);
        let display_name = {
            let trimmed = request.display_name.trim();
            if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
                request
                    .saved_file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&item_id)
                    .to_owned()
            } else {
                trimmed.chars().take(180).collect()
            }
        };
        let provider_label = {
            let trimmed = request.provider_label.trim();
            if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
                "Open Geodata".to_owned()
            } else {
                trimmed.chars().take(96).collect()
            }
        };

        self.with_business_runtime(|runtime| {
            let mut store = runtime
                .store
                .lock()
                .map_err(|_| ApplicationServiceError::StatePoisoned)?;

            // Reuse existing completed task if the same item was already downloaded into the same output_root.
            let existing_queue = store.core_restore(false)?.tasks;
            for summary in &existing_queue {
                if summary.provider_id == provider_id
                    && summary.status == TaskLifecycleStatus::Completed
                    && summary.output_root == request.output_root
                {
                    if let Some(record) = store.load_task_record(&summary.task_id)? {
                        if record.runtime.completed_item_ids.contains(&item_id) {
                            return Ok(RecordedExternalDownloadTask {
                                task_id: summary.task_id.clone(),
                                revision: summary.revision,
                                output_root: summary.output_root.clone(),
                                already_recorded: true,
                            });
                        }
                    }
                }
            }
            let history = store.load_task_history_page(&StoreTaskHistoryPageRequest {
                page_size: 50,
                after_archived_at_epoch_ms: None,
                after_task_id: None,
            })?;
            for entry in &history.items {
                if entry.task.provider_id == provider_id
                    && entry.task.status == TaskLifecycleStatus::Completed
                    && entry.task.output_root == request.output_root
                {
                    if let Some(record) = store.load_task_record(&entry.task.task_id)? {
                        if record.runtime.completed_item_ids.contains(&item_id) {
                            return Ok(RecordedExternalDownloadTask {
                                task_id: entry.task.task_id.clone(),
                                revision: entry.task.revision,
                                output_root: entry.task.output_root.clone(),
                                already_recorded: true,
                            });
                        }
                    }
                }
            }

            let now_sec = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
            let short_uuid = &Uuid::new_v4().simple().to_string()[..12];
            let task_id = format!("geodata-{clean_dataset}-{short_uuid}");
            let batch_id = format!("geodata-batch-{}", Uuid::new_v4().simple());
            let polygon = build_bbox_polygon_geojson(request.bbox);

            let mut metadata = Map::new();
            metadata.insert("dataset".to_owned(), Value::String(clean_dataset.clone()));
            metadata.insert(
                "logical_dataset".to_owned(),
                Value::String(clean_dataset.clone()),
            );
            metadata.insert(
                "product_family".to_owned(),
                Value::String("OPEN_GEODATA".to_owned()),
            );
            metadata.insert("provider".to_owned(), Value::String(provider_label));
            metadata.insert("size_bytes".to_owned(), json!(request.bytes_written));

            let mut payload = Map::new();
            payload.insert("item_id".to_owned(), Value::String(item_id.clone()));
            payload.insert(
                "downloaded_file".to_owned(),
                Value::String(request.saved_file_path.to_string_lossy().to_string()),
            );

            let snapshot_item = SnapshotItem {
                item_id: item_id.clone(),
                provider_id: provider_id.clone(),
                display_name,
                acquisition_datetime: request
                    .acquisition_datetime
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
                    .map(str::to_owned),
                footprint_geojson: Some(polygon.clone()),
                metadata,
                payload,
                asset_access: None,
                execution_policy: None,
            };

            let mut query_map = Map::new();
            query_map.insert("dataset".to_owned(), Value::String(clean_dataset.clone()));
            query_map.insert("logical_dataset".to_owned(), Value::String(clean_dataset));

            let batch = FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.clone(),
                provider_id,
                created_at: now_sec.clone(),
                aoi_name: "研究区范围".to_owned(),
                aoi_geojson: Some(polygon),
                query: query_map,
                search_stats: None,
                candidates: vec![snapshot_item],
            };

            let batch_req = StoreSearchBatchRequest {
                batch,
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.clone(),
                    checked_item_ids: vec![item_id.clone()],
                    page_index: 0,
                    page_size: 1,
                    revision: 0,
                },
                activate_workbench: false,
            };

            let create_req = StoreCreateTaskRequest {
                task_id: task_id.clone(),
                batch_id,
                selected_item_ids: vec![item_id.clone()],
                checked_item_ids: vec![item_id.clone()],
                page_index: 0,
                page_size: 1,
                output_root: request.output_root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: now_sec.clone(),
            };

            let stored = store.save_search_batch_and_create_task(&batch_req, &create_req)?;
            let activated = store.update_scheduler_runtime(&StoreUpdateTaskRequest {
                task_id: task_id.clone(),
                expected_revision: stored.runtime.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::ActivateItems,
                    item_ids: vec![item_id.clone()],
                },
                event_id: format!("geodata:{task_id}:activate"),
                occurred_at: now_sec.clone(),
            })?;

            let dl_rev = store.save_scheduler_download_result(&DownloadItemResult {
                task_id: task_id.clone(),
                item_id: item_id.clone(),
                outcome: DownloadOutcome::Completed,
                destination: request.saved_file_path.clone(),
                bytes_written: request.bytes_written,
                attempt_count: 1,
                failure: None,
                finished_at: now_sec.clone(),
            })?;

            store.save_task_transfer_telemetry_with_network(
                &StoreTaskTransferTelemetryUpdate {
                    task_id: task_id.clone(),
                    expected_revision: dl_rev,
                    transfer_active_millis: request.elapsed_millis.max(50),
                    updated_at: now_sec.clone(),
                },
                Some(request.bytes_written),
            )?;

            let completed = store.update_scheduler_runtime(&StoreUpdateTaskRequest {
                task_id: task_id.clone(),
                expected_revision: activated.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkCompleted,
                    item_ids: vec![item_id],
                },
                event_id: format!("geodata:{task_id}:complete"),
                occurred_at: now_sec,
            })?;

            Ok(RecordedExternalDownloadTask {
                task_id,
                revision: completed.revision,
                output_root: request.output_root.clone(),
                already_recorded: false,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApplicationServiceConfig;
    use insar_store::StoreTaskOutputDirectoryRequest;

    #[test]
    fn record_external_download_task_persists_completed_task_and_deduplicates_same_root() {
        let dir = std::env::temp_dir().join(format!(
            "test_ext_dl_{}_{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("tempdir");
        let store_path = dir.join("tasks.sqlite3");
        let output_root = dir.join("downloads");
        std::fs::create_dir_all(&output_root).expect("create downloads dir");
        let saved_file = output_root.join("LiCSAR_061A_05958_131313_20220214_20220226.geo.cc.tif");
        std::fs::write(&saved_file, b"II*\0dummy_geotiff_payload_bytes").expect("write tif");

        let service = ApplicationService::new(ApplicationServiceConfig::business_store(store_path));
        service.startup().expect("service startup");

        let req = RecordExternalDownloadTaskRequest {
            dataset: "comet_licsar".to_owned(),
            provider_label: "COMET LiCSAR (JASMIN)".to_owned(),
            item_id: "LiCSAR_061A_05958_131313_20220214_20220226".to_owned(),
            display_name: "LiCSAR_061A_05958_131313_20220214_20220226.geo.cc.tif".to_owned(),
            acquisition_datetime: Some("2022-02-26T00:00:00Z".to_owned()),
            bbox: Some([117.1, 31.7, 117.4, 32.0]),
            output_root: output_root.clone(),
            saved_file_path: saved_file.clone(),
            bytes_written: 31,
            elapsed_millis: 320,
        };

        let first = service
            .record_external_download_task(&req)
            .expect("first record");
        assert!(!first.already_recorded);
        assert!(first.task_id.starts_with("geodata-comet_licsar-"));

        let overview = service.task_overview().expect("task overview");
        let matched = overview
            .history_page
            .items
            .iter()
            .find(|t| t.task_id == first.task_id)
            .expect("completed task in Download Center overview");
        assert_eq!(matched.status, TaskLifecycleStatus::Completed);
        assert_eq!(matched.completed_count, 1);
        assert_eq!(matched.transferred_bytes, 31);
        assert_eq!(matched.output_root, output_root);

        let resolved_dir = service
            .resolve_task_output_directory(&StoreTaskOutputDirectoryRequest {
                task_id: first.task_id.clone(),
                expected_revision: matched.revision,
            })
            .expect("resolve output directory");
        assert_eq!(resolved_dir.output_root, output_root);

        let second = service
            .record_external_download_task(&req)
            .expect("second duplicate record");
        assert!(second.already_recorded);
        assert_eq!(second.task_id, first.task_id);
        let overview2 = service.task_overview().expect("overview 2");
        assert_eq!(
            overview2.queue.len() + overview2.history_page.items.len(),
            1
        );

        let _ = service.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
