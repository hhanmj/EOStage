use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use insar_application_service::{ApplicationService, ApplicationServiceConfig};
use insar_core::{
    DownloadItemResult, DownloadLayout, DownloadOutcome, FrozenSearchBatch, SnapshotItem,
    StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskStateCommandKind,
};
use insar_store::{StoreStandaloneAoiInput, TaskStore};
use serde_json::{json, Map, Value};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard(PathBuf);
impl TempDirGuard {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "insar-exp-proj-bundle-{label}-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn seed_completed_task(
    store: &mut TaskStore,
    idx: usize,
    provider_id: &str,
    dataset_id: &str,
    output_root: &PathBuf,
    artifact_name: &str,
    content: &[u8],
) -> String {
    fs::create_dir_all(output_root).unwrap();
    let artifact_path = output_root.join(artifact_name);
    fs::write(&artifact_path, content).unwrap();

    let batch_id = format!("batch-{idx}");
    let item_id = format!("item-{idx}");
    let task_id = format!("task-{idx}");
    let created_at = format!("2026-11-14T00:{:02}:00Z", idx % 60);

    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.clone(),
        provider_id: provider_id.to_owned(),
        created_at: created_at.clone(),
        aoi_name: "蜀山区".to_owned(),
        aoi_geojson: Some(
            json!({
                "type": "Polygon",
                "coordinates": [[[117.15, 31.80], [117.28, 31.80], [117.28, 31.88], [117.15, 31.88], [117.15, 31.80]]]
            })
            .as_object()
            .unwrap()
            .clone(),
        ),
        query: json!({ "logical_dataset": dataset_id })
            .as_object()
            .unwrap()
            .clone(),
        search_stats: None,
        candidates: vec![SnapshotItem {
            item_id: item_id.clone(),
            provider_id: provider_id.to_owned(),
            display_name: artifact_name.to_owned(),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::<String, Value>::new(),
            payload: Map::<String, Value>::new(),
            asset_access: None,
            execution_policy: None,
        }],
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: insar_core::SearchWorkbenchState {
                batch_id: batch_id.clone(),
                checked_item_ids: vec![item_id.clone()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();

    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.clone(),
            batch_id,
            selected_item_ids: vec![item_id.clone()],
            checked_item_ids: vec![item_id.clone()],
            page_index: 0,
            page_size: 24,
            output_root: output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: created_at.clone(),
        })
        .unwrap();

    store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: task_id.clone(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec![item_id.clone()],
            },
            event_id: format!("ev-act-{idx}"),
            occurred_at: created_at.clone(),
        })
        .unwrap();

    store
        .save_scheduler_download_result(&DownloadItemResult {
            task_id: task_id.clone(),
            item_id: item_id.clone(),
            outcome: DownloadOutcome::Completed,
            destination: artifact_path,
            bytes_written: content.len() as u64,
            attempt_count: 1,
            failure: None,
            finished_at: created_at.clone(),
        })
        .unwrap();

    store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: task_id.clone(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec![item_id],
            },
            event_id: format!("ev-done-{idx}"),
            occurred_at: created_at,
        })
        .unwrap();

    task_id
}

#[test]
fn experiment_project_manages_tasks_verifies_sha256_enforces_cap_and_exports_bundle() {
    let root = TempDirGuard::new("full");
    let store_path = root.0.join("tasks.sqlite3");

    {
        let mut store = TaskStore::open(&store_path).unwrap();
        let geom: Map<String, Value> = json!({
            "type": "Polygon",
            "coordinates": [[[117.15, 31.80], [117.28, 31.80], [117.28, 31.88], [117.15, 31.88], [117.15, 31.80]]]
        })
        .as_object()
        .unwrap()
        .clone();
        store
            .save_standalone_aoi(&StoreStandaloneAoiInput {
                expected_revision: None,
                source_kind: "administrative_boundary".to_owned(),
                geometry: geom,
                bbox: vec![117.15, 31.80, 117.28, 31.88],
                crs: "EPSG:4326".to_owned(),
                display_name: "合肥市蜀山区".to_owned(),
                updated_at: "2026-11-14T00:00:00Z".to_owned(),
            })
            .unwrap();

        seed_completed_task(
            &mut store,
            0,
            "MPC",
            "optical.sentinel2.l2a",
            &root.0.join("out-0"),
            "s2_clip.tif",
            b"sentinel2-cog-bytes-0",
        );
        seed_completed_task(
            &mut store,
            1,
            "FABDEM_PUBLIC",
            "dem.fabdem.v1",
            &root.0.join("out-1"),
            "fabdem_crop.tif",
            b"fabdem-dem-bytes-1",
        );
        for i in 2..=32 {
            seed_completed_task(
                &mut store,
                i,
                "MPC",
                "optical.sentinel2.l2a",
                &root.0.join(format!("out-{i}")),
                &format!("scene_{i}.tif"),
                format!("raster-bytes-{i}").as_bytes(),
            );
        }
    }

    let service = ApplicationService::new(ApplicationServiceConfig::store(&store_path));

    let empty = service.experiment_project_get().unwrap();
    assert_eq!(empty.item_count, 0);
    assert_eq!(empty.max_items, 32);
    assert_eq!(empty.aoi_display_name.as_deref(), Some("合肥市蜀山区"));

    // Add task-0 (S2) and verify idempotent re-add
    let s1 = service.experiment_project_add_task("task-0").unwrap();
    assert_eq!(s1.item_count, 1);
    assert!(!s1.has_noncommercial_restricted_data);
    assert_eq!(s1.items[0].integrity_status, "verified");

    let s1_repeat = service.experiment_project_add_task("task-0").unwrap();
    assert_eq!(s1_repeat.item_count, 1);

    // Add task-1 (FABDEM -> CC BY-NC-SA 4.0)
    let s2 = service.experiment_project_add_task("task-1").unwrap();
    assert_eq!(s2.item_count, 2);
    assert!(s2.has_noncommercial_restricted_data);

    // Export 4 files
    let export_dir = root.0.join("exported_bundle");
    let exported = service
        .experiment_project_export(Some(export_dir.clone()))
        .unwrap();
    assert_eq!(exported.verified_count, 2);
    assert_eq!(exported.missing_or_modified_count, 0);
    assert!(exported.has_noncommercial_restricted_data);
    assert!(export_dir.join("experiment-project.v1.json").is_file());
    assert!(export_dir.join("study_area.aoi.geojson").is_file());
    assert!(export_dir.join("scene_footprints.geojson").is_file());
    let readme = fs::read_to_string(export_dir.join("README_EXPERIMENT.md")).unwrap();
    assert!(readme.contains("CC BY-NC-SA 4.0"));

    // Tamper task-0 file on disk -> integrity becomes missing_or_modified
    fs::write(root.0.join("out-0/s2_clip.tif"), b"tampered-bytes!!").unwrap();
    let after_tamper = service.experiment_project_get().unwrap();
    assert_eq!(
        after_tamper
            .items
            .iter()
            .find(|i| i.task_id == "task-0")
            .unwrap()
            .integrity_status,
        "missing_or_modified"
    );
    let export_tampered = service
        .experiment_project_export(Some(root.0.join("exported_tampered")))
        .unwrap();
    assert_eq!(export_tampered.verified_count, 1);
    assert_eq!(export_tampered.missing_or_modified_count, 1);

    // Fill up to 32 items and confirm 33rd task (task-32) fails closed
    for i in 2..32 {
        service
            .experiment_project_add_task(&format!("task-{i}"))
            .unwrap();
    }
    let full = service.experiment_project_get().unwrap();
    assert_eq!(full.item_count, 32);
    let overflow = service.experiment_project_add_task("task-32");
    assert!(overflow.is_err());

    // Remove item reduces count back to 31
    let removed = service.experiment_project_remove_item("task-0").unwrap();
    assert_eq!(removed.item_count, 31);
}
