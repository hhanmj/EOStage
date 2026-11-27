use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreUpdateTaskViewRequest, StoreUpdateWorkbenchRequest,
};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

static NEXT_TEMP_DB_ID: AtomicU64 = AtomicU64::new(0);

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = NEXT_TEMP_DB_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-store-isolation-{}-{nonce}-{seq}.sqlite3",
            std::process::id()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn map(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn batch(batch_id: &str, aoi_name: &str, prefix: &str, count: usize) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_string(),
        provider_id: "asf.sentinel1".to_string(),
        created_at: "2024-01-15T00:00:00Z".to_string(),
        aoi_name: aoi_name.to_string(),
        aoi_geojson: Some(map(json!({
            "type": "Polygon",
            "coordinates": [[[73.5, 18.2], [134.8, 18.2], [134.8, 53.6], [73.5, 53.6], [73.5, 18.2]]]
        }))),
        query: map(json!({"product_type": "SLC", "batch": batch_id})),
        search_stats: None,
        candidates: (0..count)
            .map(|index| SnapshotItem {
                item_id: format!("{prefix}{index:03}"),
                provider_id: "asf.sentinel1".to_string(),
                display_name: format!("candidate {index}"),
                acquisition_datetime: None,
                footprint_geojson: None,
                metadata: Map::new(),
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            })
            .collect(),
    }
}

fn workbench(batch_id: &str, checked: &[&str], page_index: u64) -> SearchWorkbenchState {
    SearchWorkbenchState {
        batch_id: batch_id.to_string(),
        checked_item_ids: checked.iter().map(|value| (*value).to_string()).collect(),
        page_index,
        page_size: 24,
        revision: 0,
    }
}

fn create_task(
    task_id: &str,
    batch_id: &str,
    selected: &[&str],
    checked: &[&str],
    page_index: u64,
    output_root: &Path,
) -> StoreCreateTaskRequest {
    StoreCreateTaskRequest {
        task_id: task_id.to_string(),
        batch_id: batch_id.to_string(),
        selected_item_ids: selected.iter().map(|value| (*value).to_string()).collect(),
        checked_item_ids: checked.iter().map(|value| (*value).to_string()).collect(),
        page_index,
        page_size: 24,
        output_root: output_root.to_path_buf(),
        layout: DownloadLayout::Sentinel1Slc,
        concurrency: 3,
        created_at: "2024-01-15T01:00:00Z".to_string(),
    }
}

#[test]
fn workbench_and_later_tasks_cannot_mutate_a_frozen_task_snapshot() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    let output = database
        .path()
        .parent()
        .unwrap()
        .join("shared-download-output");
    let first_batch = batch("batch-1", "全国", "S1-", 500);
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: first_batch,
            workbench: workbench("batch-1", &["S1-000", "S1-499"], 20),
        })
        .unwrap();
    store
        .create_task(&create_task(
            "task-1",
            "batch-1",
            &["S1-000", "S1-499"],
            &["S1-499"],
            20,
            &output,
        ))
        .unwrap();

    let changed = SearchWorkbenchState {
        batch_id: "batch-1".to_string(),
        checked_item_ids: vec!["S1-123".to_string()],
        page_index: 3,
        page_size: 24,
        revision: 1,
    };
    store
        .update_workbench(&changed, 0, "2024-01-15T02:00:00Z")
        .unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch("batch-2", "上海市长宁区", "NEW-", 1),
            workbench: workbench("batch-2", &["NEW-000"], 0),
        })
        .unwrap();
    store
        .create_task(&create_task(
            "task-2",
            "batch-2",
            &["NEW-000"],
            &["NEW-000"],
            0,
            &output,
        ))
        .unwrap();

    let first = store.load_task("task-1").unwrap().unwrap();
    let second = store.load_task("task-2").unwrap().unwrap();
    assert_eq!(first.snapshot.batch_id, "batch-1");
    assert_eq!(first.snapshot.aoi_name, "全国");
    assert_eq!(first.snapshot.candidates.len(), 500);
    assert_eq!(
        first.snapshot.initial_selected_item_ids,
        ["S1-000", "S1-499"]
    );
    assert_eq!(first.runtime.checked_item_ids, ["S1-499"]);
    assert_eq!(first.runtime.page_index, 20);
    assert_eq!(first.snapshot.output_root, output);
    assert_eq!(second.snapshot.output_root, first.snapshot.output_root);
    assert_ne!(second.snapshot.task_id, first.snapshot.task_id);

    let current = store.load_workbench().unwrap().unwrap();
    assert_eq!(current.batch.batch_id, "batch-2");
    assert_eq!(current.batch.aoi_name, "上海市长宁区");
}

#[test]
fn invalid_selection_rolls_back_without_creating_a_partial_task() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch("batch-1", "全国", "S1-", 2),
            workbench: workbench("batch-1", &["S1-000"], 0),
        })
        .unwrap();

    let error = store
        .create_task(&create_task(
            "task-invalid",
            "batch-1",
            &["missing"],
            &["S1-000"],
            0,
            database.path().parent().unwrap(),
        ))
        .unwrap_err();

    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    assert!(store.load_task("task-invalid").unwrap().is_none());
}

#[test]
fn derived_batch_and_selected_task_commit_or_roll_back_as_one_unit() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch("source-batch", "Source AOI", "SRC-", 2),
            workbench: workbench("source-batch", &["SRC-000"], 0),
        })
        .unwrap();

    let mut derived = batch("delivery-batch", "Derived AOI before rollback", "DLV-", 2);
    derived.query.insert(
        "derived_from_batch_id".to_owned(),
        Value::String("source-batch".to_owned()),
    );
    let mut task = create_task(
        "delivery-task",
        "delivery-batch",
        &["missing"],
        &["missing"],
        0,
        database.path().parent().unwrap(),
    );
    task.layout = DownloadLayout::Direct;
    let request = StoreSearchBatchRequest {
        activate_workbench: true,
        batch: derived.clone(),
        workbench: workbench("delivery-batch", &["DLV-000"], 0),
    };

    let error = store
        .save_search_batch_and_create_task(&request, &task)
        .unwrap_err();
    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    assert!(store.load_task("delivery-task").unwrap().is_none());
    assert_eq!(
        store.load_workbench().unwrap().unwrap().batch.batch_id,
        "source-batch"
    );

    // A changed retry with the same derived batch ID proves that no hidden
    // search-batch row survived the failed transaction.
    derived.aoi_name = "Derived AOI after rollback".to_owned();
    let retry = StoreSearchBatchRequest {
        activate_workbench: true,
        batch: derived,
        workbench: workbench("delivery-batch", &["DLV-000"], 0),
    };
    task.selected_item_ids = vec!["DLV-000".to_owned()];
    task.checked_item_ids = vec!["DLV-000".to_owned()];
    store
        .save_search_batch_and_create_task(&retry, &task)
        .unwrap();

    let active = store.load_workbench().unwrap().unwrap();
    assert_eq!(active.batch.batch_id, "delivery-batch");
    assert_eq!(active.batch.aoi_name, "Derived AOI after rollback");
    let restored = store.load_task("delivery-task").unwrap().unwrap();
    assert_eq!(restored.snapshot.batch_id, "delivery-batch");
    assert_eq!(restored.snapshot.initial_selected_item_ids, ["DLV-000"]);
    assert_eq!(restored.snapshot.layout, DownloadLayout::Direct);
    drop(store);

    let reopened = TaskStore::open(database.path()).unwrap();
    let restored = reopened.load_task("delivery-task").unwrap().unwrap();
    assert_eq!(restored.snapshot.batch_id, "delivery-batch");
    assert_eq!(restored.runtime.queued_item_ids, ["DLV-000"]);
    assert_eq!(
        reopened.load_workbench().unwrap().unwrap().batch.batch_id,
        "delivery-batch"
    );
}

#[test]
fn workbench_update_request_persists_selection_and_rejects_stale_revision() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch("batch-1", "AOI", "S1-", 3),
            workbench: workbench("batch-1", &["S1-000"], 0),
        })
        .unwrap();
    let request = StoreUpdateWorkbenchRequest {
        state: SearchWorkbenchState {
            batch_id: "batch-1".to_string(),
            checked_item_ids: vec!["S1-002".to_string()],
            page_index: 1,
            page_size: 24,
            revision: 1,
        },
        expected_revision: 0,
        updated_at: "2024-01-15T02:00:00Z".to_string(),
    };

    store.update_workbench_request(&request).unwrap();
    let restored = store.load_workbench().unwrap().unwrap();
    assert_eq!(restored.state.checked_item_ids, ["S1-002"]);
    assert_eq!(restored.state.page_index, 1);
    assert_eq!(restored.state.revision, 1);

    assert_eq!(
        store.update_workbench_request(&request).unwrap_err().code(),
        "V3_STORE_REVISION_CONFLICT"
    );
}

#[test]
fn inactive_orbit_batch_does_not_replace_the_search_workbench() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch("asf-batch", "ASF AOI", "S1-", 1),
            workbench: workbench("asf-batch", &["S1-000"], 0),
        })
        .unwrap();
    let mut orbit = batch("orbit-batch", "Orbit AOI", "ORB-", 1);
    orbit.provider_id = "esa.sentinel1_orbit".to_string();
    for candidate in &mut orbit.candidates {
        candidate.provider_id = orbit.provider_id.clone();
    }
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: false,
            batch: orbit,
            workbench: workbench("orbit-batch", &["ORB-000"], 0),
        })
        .unwrap();

    let restored = store.load_workbench().unwrap().unwrap();
    assert_eq!(restored.batch.batch_id, "asf-batch");
    assert_eq!(restored.state.checked_item_ids, ["S1-000"]);
}

#[test]
fn task_view_update_persists_checked_candidates_without_consuming_lifecycle_revision() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch("batch-view", "AOI", "S1-", 4),
            workbench: workbench("batch-view", &["S1-000"], 0),
        })
        .unwrap();
    store
        .create_task(&create_task(
            "task-view",
            "batch-view",
            &["S1-000"],
            &["S1-000"],
            0,
            database.path().parent().unwrap(),
        ))
        .unwrap();

    let updated = store
        .update_task_view(&StoreUpdateTaskViewRequest {
            task_id: "task-view".to_string(),
            checked_item_ids: vec!["S1-002".to_string(), "S1-003".to_string()],
            page_index: 3,
            page_size: 24,
            updated_at: "2024-01-15T02:00:00Z".to_string(),
        })
        .unwrap();

    assert_eq!(updated.runtime.checked_item_ids, ["S1-002", "S1-003"]);
    assert_eq!(updated.runtime.page_index, 3);
    assert_eq!(updated.runtime.page_size, 24);
    assert_eq!(updated.runtime.revision, 0);

    let reopened = TaskStore::open(database.path()).unwrap();
    let restored = reopened.load_task("task-view").unwrap().unwrap();
    assert_eq!(restored.runtime.checked_item_ids, ["S1-002", "S1-003"]);
    assert_eq!(restored.runtime.page_index, 3);
    assert_eq!(restored.runtime.revision, 0);
}

#[test]
fn task_view_update_rejects_ids_outside_the_frozen_candidate_set() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch("batch-view-invalid", "AOI", "S1-", 2),
            workbench: workbench("batch-view-invalid", &["S1-000"], 0),
        })
        .unwrap();
    store
        .create_task(&create_task(
            "task-view-invalid",
            "batch-view-invalid",
            &["S1-000"],
            &["S1-000"],
            0,
            database.path().parent().unwrap(),
        ))
        .unwrap();

    let error = store
        .update_task_view(&StoreUpdateTaskViewRequest {
            task_id: "task-view-invalid".to_string(),
            checked_item_ids: vec!["OTHER-DATASET-COLLISION".to_string()],
            page_index: 1,
            page_size: 24,
            updated_at: "2024-01-15T02:00:00Z".to_string(),
        })
        .unwrap_err();

    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    let restored = store.load_task("task-view-invalid").unwrap().unwrap();
    assert_eq!(restored.runtime.checked_item_ids, ["S1-000"]);
    assert_eq!(restored.runtime.page_index, 0);
}

#[test]
fn late_task_view_update_cannot_overwrite_a_newer_checkbox_or_page_state() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: batch("batch-view-order", "AOI", "S1-", 3),
            workbench: workbench("batch-view-order", &["S1-000"], 0),
        })
        .unwrap();
    store
        .create_task(&create_task(
            "task-view-order",
            "batch-view-order",
            &["S1-000"],
            &["S1-000"],
            0,
            database.path().parent().unwrap(),
        ))
        .unwrap();

    store
        .update_task_view(&StoreUpdateTaskViewRequest {
            task_id: "task-view-order".to_string(),
            checked_item_ids: vec!["S1-002".to_string()],
            page_index: 5,
            page_size: 24,
            updated_at: "2024-01-15T02:00:02Z".to_string(),
        })
        .unwrap();
    let late = store
        .update_task_view(&StoreUpdateTaskViewRequest {
            task_id: "task-view-order".to_string(),
            checked_item_ids: vec!["S1-001".to_string()],
            page_index: 1,
            page_size: 24,
            updated_at: "2024-01-15T02:00:01Z".to_string(),
        })
        .unwrap();

    assert_eq!(late.runtime.checked_item_ids, ["S1-002"]);
    assert_eq!(late.runtime.page_index, 5);
    let restored = store.load_task("task-view-order").unwrap().unwrap();
    assert_eq!(restored.runtime.checked_item_ids, ["S1-002"]);
    assert_eq!(restored.runtime.page_index, 5);
}
