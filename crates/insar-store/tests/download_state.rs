use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout,
    DownloadOutcome, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreTaskPageRequest, StoreUpdateTaskRequest,
    TaskStateCommandKind,
};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-download-{}-{nonce}-{seq}.sqlite3",
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

fn object(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn seed(store: &mut TaskStore, root: &Path) {
    let item = SnapshotItem {
        item_id: "S1-A".to_string(),
        provider_id: "asf.sentinel1".to_string(),
        display_name: "S1-A.zip".to_string(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: object(json!({"file_name": "S1-A.zip"})),
        asset_access: None,
        execution_policy: None,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-download".to_string(),
                provider_id: "asf.sentinel1".to_string(),
                created_at: "2026-07-14T00:00:00Z".to_string(),
                aoi_name: "测试区".to_string(),
                aoi_geojson: Some(object(json!({
                    "type": "Polygon",
                    "coordinates": [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]]
                }))),
                query: object(json!({"product_type": "SLC"})),
                search_stats: None,
                candidates: vec![item],
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-download".to_string(),
                checked_item_ids: vec!["S1-A".to_string()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-download".to_string(),
            batch_id: "batch-download".to_string(),
            selected_item_ids: vec!["S1-A".to_string()],
            checked_item_ids: vec!["S1-A".to_string()],
            page_index: 0,
            page_size: 24,
            output_root: root.to_path_buf(),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 2,
            created_at: "2026-07-14T00:01:00Z".to_string(),
        })
        .unwrap();
}

#[test]
fn progress_and_terminal_results_are_atomic_revisioned_and_restart_safe() {
    let database = TempDatabase::new();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        seed(&mut store, database.path().parent().unwrap());
        let empty = store.load_download_snapshot("task-download").unwrap();
        assert_eq!(empty.revision, 0);
        assert!(empty.progress.is_empty());

        let progress = DownloadItemProgress {
            task_id: "task-download".to_string(),
            item_id: "S1-A".to_string(),
            status: DownloadItemStatus::Active,
            bytes_completed: 2048,
            total_bytes: Some(4096),
            bytes_per_second: 512.0,
            attempt: 1,
            updated_at: "2026-07-14T00:02:00Z".to_string(),
        };
        let saved = store.save_download_progress(&progress, 0).unwrap();
        assert_eq!(saved.revision, 1);
        assert_eq!(saved.progress.as_slice(), std::slice::from_ref(&progress));
        assert_eq!(
            store
                .save_download_progress(&progress, 0)
                .unwrap_err()
                .code(),
            "V3_STORE_REVISION_CONFLICT"
        );

        let result = DownloadItemResult {
            task_id: "task-download".to_string(),
            item_id: "S1-A".to_string(),
            outcome: DownloadOutcome::Failed,
            destination: database.path().with_extension("zip"),
            bytes_written: 2048,
            attempt_count: 3,
            failure: Some(DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "网络连接暂时失败".to_string(),
                retryable: true,
            }),
            finished_at: "2026-07-14T00:03:00Z".to_string(),
        };
        let saved = store.save_download_result(&result, 1).unwrap();
        assert_eq!(saved.revision, 2);
        assert!(saved.progress.is_empty());
        assert_eq!(saved.results, [result]);
    }

    let reopened = TaskStore::open(database.path()).unwrap();
    let restored = reopened.load_download_snapshot("task-download").unwrap();
    assert_eq!(restored.revision, 2);
    assert!(restored.progress.is_empty());
    assert_eq!(restored.results[0].failure.as_ref().unwrap().code, "DL005");
}

#[test]
fn scheduler_progress_hot_path_shares_revision_and_total_authority() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, database.path().parent().unwrap());

    let first = DownloadItemProgress {
        task_id: "task-download".to_string(),
        item_id: "S1-A".to_string(),
        status: DownloadItemStatus::Active,
        bytes_completed: 1024,
        total_bytes: Some(4096),
        bytes_per_second: 512.0,
        attempt: 1,
        updated_at: "2026-07-14T00:02:00Z".to_string(),
    };
    assert_eq!(
        store.save_scheduler_download_progress(&first).unwrap(),
        1,
        "Scheduler hot path must advance the same download-engine revision"
    );
    let after_scheduler = store.load_download_snapshot("task-download").unwrap();
    assert_eq!(after_scheduler.revision, 1);
    assert_eq!(
        after_scheduler.progress.as_slice(),
        std::slice::from_ref(&first)
    );

    let second = DownloadItemProgress {
        bytes_completed: 2048,
        total_bytes: None,
        updated_at: "2026-07-14T00:02:01Z".to_string(),
        ..first
    };
    let saved = store.save_download_progress(&second, 1).unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.progress.len(), 1);
    assert_eq!(saved.progress[0].bytes_completed, 2048);
    assert_eq!(
        saved.progress[0].total_bytes,
        Some(4096),
        "unknown later totals must preserve the Scheduler-persisted known total"
    );
    let result = DownloadItemResult {
        task_id: "task-download".to_string(),
        item_id: "S1-A".to_string(),
        outcome: DownloadOutcome::Completed,
        destination: database.path().with_extension("zip"),
        bytes_written: 4096,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-07-14T00:02:02Z".to_string(),
    };
    assert_eq!(
        store.save_scheduler_download_result(&result).unwrap(),
        3,
        "Scheduler terminal hot path must continue the same download-engine revision"
    );
    let terminal = store.load_download_snapshot("task-download").unwrap();
    assert_eq!(terminal.revision, 3);
    assert!(terminal.progress.is_empty());
    assert_eq!(terminal.results.as_slice(), std::slice::from_ref(&result));
}

#[test]
fn cancelled_progress_is_preserved_but_projected_as_stopped_after_reopen() {
    let database = TempDatabase::new();
    let progress = DownloadItemProgress {
        task_id: "task-download".to_string(),
        item_id: "S1-A".to_string(),
        status: DownloadItemStatus::Active,
        bytes_completed: 2048,
        total_bytes: Some(4096),
        bytes_per_second: 512.0,
        attempt: 2,
        updated_at: "2026-07-14T00:02:00Z".to_string(),
    };
    let cancelled_revision = {
        let mut store = TaskStore::open(database.path()).unwrap();
        seed(&mut store, database.path().parent().unwrap());
        store.save_scheduler_download_progress(&progress).unwrap();
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "task-download".to_string(),
                expected_revision: 0,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::CancelItems,
                    item_ids: vec!["S1-A".to_string()],
                },
                event_id: "projection-cancel".to_string(),
                occurred_at: "2026-07-14T00:03:00Z".to_string(),
            })
            .unwrap()
            .runtime
            .revision
    };
    let store = TaskStore::open(database.path()).unwrap();
    let raw = store.load_download_snapshot("task-download").unwrap();
    assert_eq!(raw.progress, [progress]);
    assert!(raw.results.is_empty(), "no worker result can be invented");
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "task-download".to_string(),
            expected_revision: cancelled_revision,
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(page.items.len(), 1);
    let item = &page.items[0];
    assert_eq!(item.status, DownloadItemStatus::Cancelled);
    assert_eq!(item.bytes_completed, 2048);
    assert_eq!(item.total_bytes, Some(4096));
    assert_eq!(item.attempt, 2);
    assert_eq!(item.bytes_per_second, 0.0);
    assert_eq!(item.destination, None);
    assert_eq!(item.finished_at, None);
    let core = store.core_restore(true).unwrap();
    let summary = core
        .tasks
        .iter()
        .find(|task| task.task_id == "task-download")
        .unwrap();
    assert_eq!(summary.bytes_per_second, 0.0);
    assert_eq!(summary.active_count, 0);
    assert_eq!(summary.cancelled_count, 1);
    assert_eq!(summary.network_bytes_received, None);
}

#[test]
fn progress_rejects_unknown_task_items_and_nonfinite_rates() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, database.path().parent().unwrap());
    let invalid = DownloadItemProgress {
        task_id: "task-download".to_string(),
        item_id: "UNKNOWN".to_string(),
        status: DownloadItemStatus::Active,
        bytes_completed: 0,
        total_bytes: None,
        bytes_per_second: f64::NAN,
        attempt: 0,
        updated_at: "2026-07-14T00:02:00Z".to_string(),
    };
    assert_eq!(
        store
            .save_download_progress(&invalid, 0)
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
}
