//! Downloader D3-2 (batch pre-scan): the coordinator commits items the local
//! pre-scan proved reusable in one Store transaction: one task revision, one
//! local_reuse_applied event, skipped results and decision rows. Items that
//! left the queue meanwhile are not touched.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, DownloadOutcome, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem,
    StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskStateCommandKind,
};
use insar_store::{
    StoreLocalReuseBatch, StoreLocalReuseBatchItem, StoreLocalReuseDisposition,
    StoreLocalScanState, StoreTaskLocalScan, TaskStore, STORE_LOCAL_REUSE_BATCH_LIMIT,
};
use serde_json::{json, Map};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-local-reuse-batch-{name}-{}-{nonce}.sqlite3",
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

const TASK: &str = "task-batch";

fn item_ids(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("S1-{index:03}")).collect()
}

fn seed(store: &mut TaskStore, count: usize) {
    let candidates = item_ids(count)
        .into_iter()
        .map(|item_id| SnapshotItem {
            display_name: format!("{item_id}.zip"),
            item_id,
            provider_id: "asf.sentinel1".to_owned(),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        })
        .collect::<Vec<_>>();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-prescan".to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: "2026-10-20T00:00:00Z".to_owned(),
                aoi_name: "AOI".to_owned(),
                aoi_geojson: Some(
                    json!({"type": "Polygon", "coordinates": []})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates: candidates.clone(),
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-prescan".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: TASK.to_owned(),
            batch_id: "batch-prescan".to_owned(),
            selected_item_ids: item_ids(count),
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: std::env::temp_dir().join("insar-store-local-reuse-batch-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-20T00:00:01Z".to_owned(),
        })
        .unwrap();
}

fn command(store: &mut TaskStore, kind: TaskStateCommandKind, items: &[&str], event: &str) {
    let revision = store.load_task(TASK).unwrap().unwrap().runtime.revision;
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: TASK.to_owned(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind,
                item_ids: items.iter().map(|item| (*item).to_owned()).collect(),
            },
            event_id: event.to_owned(),
            occurred_at: "2026-10-20T00:00:10Z".to_owned(),
        })
        .unwrap();
}

fn batch(items: &[&str]) -> StoreLocalReuseBatch {
    StoreLocalReuseBatch {
        task_id: TASK.to_owned(),
        items: items
            .iter()
            .map(|item| StoreLocalReuseBatchItem {
                item_id: (*item).to_owned(),
                destination: PathBuf::from(format!("D:/out/{item}.zip")),
                size_bytes: 42,
                grade: "size_and_format".to_owned(),
                landed: None,
            })
            .collect(),
        event_id: "prescan-batch-1".to_owned(),
        occurred_at: "2026-10-20T00:01:00Z".to_owned(),
    }
}

#[test]
fn a_batch_skips_the_queued_items_in_one_revision_with_one_event() {
    let database = TempDatabase::new("one-revision");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, 3);
    command(
        &mut store,
        TaskStateCommandKind::CancelItems,
        &["S1-002"],
        "cancel-2",
    );
    let before = store.load_task(TASK).unwrap().unwrap().runtime.revision;
    let applied = store
        .apply_local_reuse_batch(&batch(&["S1-000", "S1-001", "S1-002"]))
        .unwrap();
    assert_eq!(applied, vec!["S1-000".to_owned(), "S1-001".to_owned()]);
    let task = store.load_task(TASK).unwrap().unwrap();
    assert_eq!(
        task.runtime.revision,
        before + 1,
        "one revision for the batch"
    );
    assert!(task.runtime.skipped_item_ids.contains(&"S1-000".to_owned()));
    assert!(task.runtime.skipped_item_ids.contains(&"S1-001".to_owned()));
    assert!(task
        .runtime
        .cancelled_item_ids
        .contains(&"S1-002".to_owned()));
    let events = store.list_task_events(TASK).unwrap();
    let applied_events = events
        .iter()
        .filter(|event| event.event_type == "local_reuse_applied")
        .collect::<Vec<_>>();
    assert_eq!(applied_events.len(), 1);
    assert_eq!(applied_events[0].payload["items"], json!(2));
    assert!(
        !applied_events[0].payload.to_string().contains("D:/out"),
        "no paths in events"
    );
    let results = store.load_download_snapshot(TASK).unwrap().results;
    for item in ["S1-000", "S1-001"] {
        let result = results
            .iter()
            .find(|result| result.item_id == item)
            .unwrap();
        assert_eq!(result.outcome, DownloadOutcome::Skipped);
        assert_eq!(result.bytes_written, 42);
        assert_eq!(result.attempt_count, 0);
        let decision = store.task_item_local_reuse(TASK, item).unwrap().unwrap();
        assert_eq!(
            decision.disposition,
            StoreLocalReuseDisposition::Reused {
                grade: "size_and_format".to_owned()
            }
        );
    }
    assert!(store
        .task_item_local_reuse(TASK, "S1-002")
        .unwrap()
        .is_none());
}

#[test]
fn a_batch_with_no_queued_item_changes_nothing() {
    let database = TempDatabase::new("nothing");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, 2);
    command(
        &mut store,
        TaskStateCommandKind::ActivateItems,
        &["S1-000"],
        "activate-0",
    );
    command(
        &mut store,
        TaskStateCommandKind::CancelItems,
        &["S1-001"],
        "cancel-1",
    );
    let before = store.load_task(TASK).unwrap().unwrap().runtime.revision;
    let applied = store
        .apply_local_reuse_batch(&batch(&["S1-000", "S1-001"]))
        .unwrap();
    assert!(
        applied.is_empty(),
        "active and cancelled items are not reused"
    );
    assert_eq!(
        store.load_task(TASK).unwrap().unwrap().runtime.revision,
        before
    );
    assert!(store
        .list_task_events(TASK)
        .unwrap()
        .iter()
        .all(|event| event.event_type != "local_reuse_applied"));
}

#[test]
fn a_paused_task_takes_no_batch() {
    let database = TempDatabase::new("paused");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, 2);
    command(&mut store, TaskStateCommandKind::PauseTask, &[], "pause");
    let applied = store.apply_local_reuse_batch(&batch(&["S1-000"])).unwrap();
    assert!(applied.is_empty());
    let task = store.load_task(TASK).unwrap().unwrap();
    assert!(task.runtime.queued_item_ids.contains(&"S1-000".to_owned()));
}

#[test]
fn an_oversized_or_duplicate_batch_is_refused_whole() {
    let database = TempDatabase::new("refused");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, STORE_LOCAL_REUSE_BATCH_LIMIT + 1);
    let ids = item_ids(STORE_LOCAL_REUSE_BATCH_LIMIT + 1);
    let refs = ids.iter().map(String::as_str).collect::<Vec<_>>();
    assert!(store.apply_local_reuse_batch(&batch(&refs)).is_err());
    assert!(store
        .apply_local_reuse_batch(&batch(&["S1-000", "S1-000"]))
        .is_err());
    let task = store.load_task(TASK).unwrap().unwrap();
    assert!(task.runtime.skipped_item_ids.is_empty());
}

#[test]
fn the_scan_progress_row_round_trips() {
    let database = TempDatabase::new("scan-row");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, 2);
    assert!(store.task_local_scan(TASK).unwrap().is_none());
    let scan = StoreTaskLocalScan {
        task_id: TASK.to_owned(),
        state: StoreLocalScanState::Scanning,
        scanned_items: 1,
        total_items: 2,
        updated_at: "2026-10-20T00:01:00Z".to_owned(),
    };
    store.record_task_local_scan(&scan).unwrap();
    assert_eq!(store.task_local_scan(TASK).unwrap(), Some(scan.clone()));
    let done = StoreTaskLocalScan {
        state: StoreLocalScanState::Done,
        scanned_items: 2,
        ..scan
    };
    store.record_task_local_scan(&done).unwrap();
    assert_eq!(store.task_local_scan(TASK).unwrap(), Some(done));
    assert!(store
        .record_task_local_scan(&StoreTaskLocalScan {
            scanned_items: 3,
            total_items: 2,
            ..store.task_local_scan(TASK).unwrap().unwrap()
        })
        .is_err());
}

/// Downloader D3-2 (progress): the task summary carries the pre-scan
/// progress and how many items reused an existing file, and how many of
/// those only on size and a readable format (the card's progress line).
#[test]
fn the_task_summary_reports_the_scan_and_its_reuse() {
    let database = TempDatabase::new("summary");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, 3);
    assert_eq!(store.core_restore(false).unwrap().tasks[0].local_scan, None);
    store
        .record_task_local_scan(&StoreTaskLocalScan {
            task_id: TASK.to_owned(),
            state: StoreLocalScanState::Scanning,
            scanned_items: 2,
            total_items: 3,
            updated_at: "2026-10-20T00:01:00Z".to_owned(),
        })
        .unwrap();
    let mut reuse = batch(&["S1-000", "S1-001"]);
    reuse.items[1].grade = "service_checksum".to_owned();
    store.apply_local_reuse_batch(&reuse).unwrap();
    let summary = store.core_restore(false).unwrap().tasks.remove(0);
    assert_eq!(
        summary.local_scan,
        Some(insar_store::StoreTaskLocalScanSummary {
            state: "scanning".to_owned(),
            scanned_items: 2,
            total_items: 3,
            reused_count: 2,
            unverified_count: 1,
        })
    );
}
