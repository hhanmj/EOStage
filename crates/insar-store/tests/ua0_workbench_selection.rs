use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    AppendTaskFromSourceBatchRequest, AppendTaskFromWorkbenchRequest,
    CreateTaskFromHistoricalSourceBatchRequest, CreateTaskFromWorkbenchRequest, DownloadItemStatus,
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest,
    StoreTaskCommand, StoreTaskSourceBatchPageRequest, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskStateCommandKind, WorkbenchSelectionCommand, WorkbenchSelectionCommandKind,
};
use insar_store::TaskStore;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

const BATCH_ID: &str = "ua0-selection-batch-10000";
const CREATED_AT: &str = "2026-08-13T05:00:00Z";

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
            "insar-store-ua0-selection-{}-{nonce}-{seq}.sqlite3",
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

fn candidate(index: usize) -> SnapshotItem {
    SnapshotItem {
        item_id: format!("SEL-{index:05}"),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("Sanitized selection item {index:05}"),
        acquisition_datetime: Some("2026-08-13T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: map(json!({"dataset": "SENTINEL-1 BURSTS"})),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed(database: &TempDatabase) -> TaskStore {
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: BATCH_ID.to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: CREATED_AT.to_owned(),
                aoi_name: "Sanitized selection AOI".to_owned(),
                aoi_geojson: Some(map(json!({
                    "type": "Polygon",
                    "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
                }))),
                query: map(json!({"dataset": "SENTINEL-1 BURSTS"})),
                search_stats: None,
                candidates: (0..10_000).map(candidate).collect(),
            },
            workbench: SearchWorkbenchState {
                batch_id: BATCH_ID.to_owned(),
                checked_item_ids: vec!["SEL-00000".to_owned()],
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
        })
        .unwrap();
    store
}

fn command(
    kind: WorkbenchSelectionCommandKind,
    expected_revision: u64,
    item_ids: &[&str],
) -> WorkbenchSelectionCommand {
    WorkbenchSelectionCommand {
        batch_id: BATCH_ID.to_owned(),
        expected_revision,
        kind,
        item_ids: item_ids.iter().map(|value| (*value).to_owned()).collect(),
        page_index: 0,
        page_size: 100,
        updated_at: "2026-08-13T05:01:00Z".to_owned(),
    }
}

fn ids(prefix: &str, range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|index| format!("{prefix}-{index:05}")).collect()
}

#[test]
fn selection_deltas_are_revision_guarded_persist_and_support_batch_operations() {
    let database = TempDatabase::new();
    let mut store = seed(&database);

    let summary = store
        .apply_workbench_selection(&command(
            WorkbenchSelectionCommandKind::CheckItems,
            0,
            &["SEL-00000", "SEL-00001", "SEL-00002"],
        ))
        .unwrap();
    assert_eq!(summary.revision, 1);
    assert_eq!(summary.checked_count, 3);

    let summary = store
        .apply_workbench_selection(&command(
            WorkbenchSelectionCommandKind::UncheckItems,
            1,
            &["SEL-00000"],
        ))
        .unwrap();
    assert_eq!(summary.revision, 2);
    assert_eq!(summary.checked_count, 2);

    assert_eq!(
        store
            .apply_workbench_selection(&command(
                WorkbenchSelectionCommandKind::CheckItems,
                1,
                &["SEL-00003"]
            ))
            .unwrap_err()
            .code(),
        "V3_STORE_REVISION_CONFLICT"
    );

    let mut wrong_batch = command(WorkbenchSelectionCommandKind::CheckItems, 2, &["SEL-00003"]);
    wrong_batch.batch_id = "other-batch".to_owned();
    assert_eq!(
        store
            .apply_workbench_selection(&wrong_batch)
            .unwrap_err()
            .code(),
        "V3_STORE_NOT_FOUND"
    );

    let too_many = ids("SEL", 0..251);
    let mut oversized = command(WorkbenchSelectionCommandKind::CheckItems, 2, &[]);
    oversized.item_ids = too_many;
    assert_eq!(
        store
            .apply_workbench_selection(&oversized)
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );

    let mut set_page = command(WorkbenchSelectionCommandKind::SetPage, 2, &[]);
    set_page.page_index = 5;
    set_page.page_size = 50;
    let summary = store.apply_workbench_selection(&set_page).unwrap();
    assert_eq!(summary.revision, 3);
    assert_eq!(summary.checked_count, 2);
    assert_eq!(summary.page_index, 5);
    assert_eq!(summary.page_size, 50);

    drop(store);
    let mut store = TaskStore::open(database.path()).unwrap();
    let restored = store.core_restore(false).unwrap();
    let workbench = restored.workbench.unwrap();
    assert_eq!(workbench.revision, 3);
    assert_eq!(workbench.checked_count, 2);
    assert_eq!(workbench.page_index, 5);
    assert_eq!(workbench.page_size, 50);

    let summary = store
        .apply_workbench_selection(&command(WorkbenchSelectionCommandKind::SelectAll, 3, &[]))
        .unwrap();
    assert_eq!(summary.revision, 4);
    assert_eq!(summary.checked_count, 10_000);
    let full = store.load_workbench().unwrap().unwrap();
    assert_eq!(
        full.state.checked_item_ids,
        (0..10_000)
            .map(|index| format!("SEL-{index:05}"))
            .collect::<Vec<_>>()
    );

    let summary = store
        .apply_workbench_selection(&command(WorkbenchSelectionCommandKind::ClearAll, 4, &[]))
        .unwrap();
    assert_eq!(summary.revision, 5);
    assert_eq!(summary.checked_count, 0);
}

#[test]
fn set_page_and_clear_all_do_not_parse_candidate_payload() {
    let database = TempDatabase::new();
    let store = seed(&database);
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER search_batches_are_frozen;
             UPDATE search_batches SET snapshot_json = '{not-json'
             WHERE batch_id = 'ua0-selection-batch-10000';
             CREATE TRIGGER search_batches_are_frozen
             BEFORE UPDATE ON search_batches
             BEGIN
                 SELECT RAISE(ABORT, 'frozen search batch cannot be updated');
             END;",
        )
        .unwrap();
    drop(connection);

    let mut store = TaskStore::open(database.path()).unwrap();
    let mut set_page = command(WorkbenchSelectionCommandKind::SetPage, 0, &[]);
    set_page.page_index = 7;
    set_page.page_size = 48;
    assert_eq!(
        store.apply_workbench_selection(&set_page).unwrap().revision,
        1
    );

    assert_eq!(
        store
            .apply_workbench_selection(&command(WorkbenchSelectionCommandKind::ClearAll, 1, &[]))
            .unwrap()
            .checked_count,
        0
    );
}

#[test]
fn create_task_from_workbench_consumes_exact_persisted_selection_across_pages() {
    let database = TempDatabase::new();
    let mut store = seed(&database);
    store
        .apply_workbench_selection(&command(
            WorkbenchSelectionCommandKind::CheckItems,
            0,
            &["SEL-00001", "SEL-05000", "SEL-09999"],
        ))
        .unwrap();
    let mut set_page = command(WorkbenchSelectionCommandKind::SetPage, 1, &[]);
    set_page.page_index = 7;
    set_page.page_size = 48;
    store.apply_workbench_selection(&set_page).unwrap();

    let created = store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "selection-task".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 2,
            exact_item_id: None,
            output_root: database.path().parent().unwrap().join("selection-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-13T05:02:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(
        created.snapshot.initial_selected_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-05000", "SEL-09999"]
    );
    assert_eq!(created.snapshot.candidates.len(), 10_000);
    assert_eq!(
        created.runtime.selected_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-05000", "SEL-09999"]
    );
    assert_eq!(created.runtime.page_index, 7);
    assert_eq!(created.runtime.page_size, 48);
    assert_eq!(
        created.runtime.checked_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-05000", "SEL-09999"]
    );
    let compact = store
        .load_scheduler_task("selection-task")
        .unwrap()
        .unwrap();
    assert_eq!(compact.snapshot.candidates.len(), 4);
    assert_eq!(
        compact.snapshot.initial_selected_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-05000", "SEL-09999"]
    );

    let request_value = serde_json::to_value(&CreateTaskFromWorkbenchRequest {
        task_id: "selection-task".to_owned(),
        batch_id: BATCH_ID.to_owned(),
        expected_workbench_revision: 2,
        exact_item_id: None,
        output_root: database.path().parent().unwrap().join("selection-output"),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-08-13T05:02:00Z".to_owned(),
    })
    .unwrap();
    assert!(request_value.get("selected_item_ids").is_none());
    assert!(request_value.get("checked_item_ids").is_none());
    assert!(request_value.get("item_ids").is_none());
    assert!(request_value.get("page_index").is_none());
    assert!(request_value.get("page_size").is_none());

    assert_eq!(
        store
            .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
                task_id: "stale-task".to_owned(),
                batch_id: BATCH_ID.to_owned(),
                expected_workbench_revision: 1,
                exact_item_id: None,
                output_root: database.path().parent().unwrap().join("stale-output"),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-08-13T05:02:00Z".to_owned(),
            })
            .unwrap_err()
            .code(),
        "V3_STORE_REVISION_CONFLICT"
    );

    store
        .apply_workbench_selection(&command(WorkbenchSelectionCommandKind::ClearAll, 2, &[]))
        .unwrap();
    assert_eq!(
        store
            .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
                task_id: "empty-task".to_owned(),
                batch_id: BATCH_ID.to_owned(),
                expected_workbench_revision: 3,
                exact_item_id: None,
                output_root: database.path().parent().unwrap().join("empty-output"),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-08-13T05:03:00Z".to_owned(),
            })
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );

    let exact = store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "exact-row-task".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 3,
            exact_item_id: Some("SEL-05000".to_owned()),
            output_root: database.path().parent().unwrap().join("exact-row-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-13T05:04:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(exact.runtime.selected_item_ids, ["SEL-05000"]);
    assert_eq!(exact.runtime.checked_item_ids, ["SEL-05000"]);
    let exact_summary = store
        .core_restore(false)
        .unwrap()
        .tasks
        .into_iter()
        .find(|task| task.task_id == "exact-row-task")
        .expect("exact row task summary");
    assert_eq!(
        exact_summary.dataset_id.as_deref(),
        Some("SENTINEL-1 BURSTS")
    );
    let workbench = store.load_workbench().unwrap().unwrap();
    assert_eq!(workbench.state.revision, 3);
    assert!(workbench.state.checked_item_ids.is_empty());
}

#[test]
fn global_workbench_selection_creates_and_restores_a_task_without_inventing_an_aoi() {
    let database = TempDatabase::new();
    let batch_id = "ua0-global-selection";
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: CREATED_AT.to_owned(),
                aoi_name: "未限定 AOI".to_owned(),
                aoi_geojson: None,
                query: map(json!({"dataset": "ERS", "max_results": 3})),
                search_stats: None,
                candidates: vec![SnapshotItem {
                    item_id: "ERS-GLOBAL-001".to_owned(),
                    provider_id: "asf.catalog".to_owned(),
                    display_name: "ERS-GLOBAL-001.zip".to_owned(),
                    acquisition_datetime: Some("1997-09-28T15:20:21Z".to_owned()),
                    footprint_geojson: None,
                    metadata: map(json!({"dataset": "ERS", "processing_level": "L1"})),
                    payload: map(json!({
                        "download_url": "https://datapool.asf.alaska.edu/L1/E1/ERS-GLOBAL-001.zip",
                        "file_name": "ERS-GLOBAL-001.zip",
                        "file_size_remote": 1024
                    })),
                    asset_access: None,
                    execution_policy: None,
                }],
            },
            workbench: SearchWorkbenchState {
                batch_id: batch_id.to_owned(),
                checked_item_ids: vec!["ERS-GLOBAL-001".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();

    let created = store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "global-download-task".to_owned(),
            batch_id: batch_id.to_owned(),
            expected_workbench_revision: 0,
            exact_item_id: Some("ERS-GLOBAL-001".to_owned()),
            output_root: std::env::temp_dir().join("insar-global-download"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-24T00:00:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(created.snapshot.aoi_name, "未限定 AOI");
    assert_eq!(created.snapshot.aoi_geojson, None);
    drop(store);

    let reopened = TaskStore::open_existing(database.path()).unwrap();
    let restored = reopened
        .load_task("global-download-task")
        .unwrap()
        .expect("global task restores");
    assert_eq!(restored.snapshot.aoi_geojson, None);
    assert_eq!(
        restored.snapshot.initial_selected_item_ids,
        ["ERS-GLOBAL-001"]
    );
}

#[test]
fn append_task_from_workbench_adds_only_newly_checked_items() {
    let database = TempDatabase::new();
    let mut store = seed(&database);
    store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "append-target".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 0,
            exact_item_id: None,
            output_root: database.path().parent().unwrap().join("append-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-13T05:02:00Z".to_owned(),
        })
        .unwrap();

    store
        .apply_workbench_selection(&command(
            WorkbenchSelectionCommandKind::CheckItems,
            0,
            &["SEL-00001", "SEL-09999"],
        ))
        .unwrap();
    let appended = store
        .append_task_from_workbench(&AppendTaskFromWorkbenchRequest {
            task_id: "append-target".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 1,
            event_id: "append-from-workbench-1".to_owned(),
            occurred_at: "2026-08-13T05:04:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(
        appended.runtime.selected_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-09999"]
    );
    assert_eq!(
        appended.runtime.queued_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-09999"]
    );
    assert_eq!(appended.runtime.revision, 1);

    assert_eq!(
        store
            .append_task_from_workbench(&AppendTaskFromWorkbenchRequest {
                task_id: "append-target".to_owned(),
                batch_id: BATCH_ID.to_owned(),
                expected_workbench_revision: 1,
                event_id: "append-from-workbench-2".to_owned(),
                occurred_at: "2026-08-13T05:05:00Z".to_owned(),
            })
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );

    store
        .apply_workbench_selection(&command(
            WorkbenchSelectionCommandKind::CheckItems,
            1,
            &["SEL-00002"],
        ))
        .unwrap();
    assert_eq!(
        store
            .append_task_from_workbench(&AppendTaskFromWorkbenchRequest {
                task_id: "append-target".to_owned(),
                batch_id: BATCH_ID.to_owned(),
                expected_workbench_revision: 1,
                event_id: "append-from-workbench-3".to_owned(),
                occurred_at: "2026-08-13T05:06:00Z".to_owned(),
            })
            .unwrap_err()
            .code(),
        "V3_STORE_REVISION_CONFLICT"
    );

    let appended = store
        .append_task_from_workbench(&AppendTaskFromWorkbenchRequest {
            task_id: "append-target".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 2,
            event_id: "append-from-workbench-4".to_owned(),
            occurred_at: "2026-08-13T05:07:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(
        appended.runtime.selected_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-09999", "SEL-00002"]
    );
    assert_eq!(appended.runtime.revision, 2);
}

#[test]
fn task_source_batch_page_is_bounded_revision_independent_and_never_mutates_main_workbench() {
    let database = TempDatabase::new();
    let mut store = seed(&database);
    store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "historical-source-task".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 0,
            exact_item_id: Some("SEL-00000".to_owned()),
            output_root: database
                .path()
                .parent()
                .unwrap()
                .join("historical-source-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-24T01:00:00Z".to_owned(),
        })
        .unwrap();
    let main_before = store.load_workbench().unwrap().unwrap();

    let first = store
        .load_task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "historical-source-task".to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(first.source_batch_id, BATCH_ID);
    assert_eq!(first.task_revision, 0);
    assert_eq!(first.task_status, TaskLifecycleStatus::Queued);
    assert_eq!(first.total_count, 10_000);
    assert_eq!(first.items.len(), 24);
    assert!(first.items[0].belongs_to_task);
    assert_eq!(first.items[0].task_status, Some(DownloadItemStatus::Queued));
    assert!(!first.items[6].belongs_to_task);
    assert_eq!(first.items[6].task_status, None);

    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "historical-source-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "historical-source-task:pause".to_owned(),
            occurred_at: "2026-08-24T01:01:00Z".to_owned(),
        })
        .unwrap();
    let second = store
        .load_task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "historical-source-task".to_owned(),
            page_index: 1,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(second.task_revision, 1);
    assert_eq!(second.task_status, TaskLifecycleStatus::Paused);
    assert_eq!(second.items[0].ordinal, 24);

    let main_after = store.load_workbench().unwrap().unwrap();
    assert_eq!(main_after.batch, main_before.batch);
    assert_eq!(main_after.state, main_before.state);
}

#[test]
fn task_source_mutations_append_only_to_active_tasks_and_create_successors_for_terminal_tasks() {
    let database = TempDatabase::new();
    let mut store = seed(&database);
    let output = database
        .path()
        .parent()
        .unwrap()
        .join("source-mutation-output");
    store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "active-source-task".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 0,
            exact_item_id: Some("SEL-00000".to_owned()),
            output_root: output.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-24T02:00:00Z".to_owned(),
        })
        .unwrap();
    let main_before = store.load_workbench().unwrap().unwrap();
    let appended = store
        .append_task_from_source_batch(&AppendTaskFromSourceBatchRequest {
            task_id: "active-source-task".to_owned(),
            expected_task_revision: 0,
            item_ids: vec!["SEL-00001".to_owned(), "SEL-09999".to_owned()],
            event_id: "active-source-task:append".to_owned(),
            occurred_at: "2026-08-24T02:01:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(appended.runtime.revision, 1);
    assert_eq!(appended.runtime.concurrency, 2);
    assert_eq!(appended.snapshot.output_root, output);
    assert_eq!(
        appended.runtime.selected_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-09999"]
    );
    assert_eq!(store.load_workbench().unwrap().unwrap(), main_before);
    drop(store);
    let mut store = TaskStore::open_existing(database.path()).unwrap();
    let scheduled = store
        .load_scheduler_task("active-source-task")
        .unwrap()
        .expect("appended task remains schedulable after reopen");
    assert_eq!(scheduled.snapshot.candidates.len(), 3);
    assert_eq!(
        scheduled.runtime.selected_item_ids,
        ["SEL-00000", "SEL-00001", "SEL-09999"]
    );

    store
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "failed-source-task".to_owned(),
            batch_id: BATCH_ID.to_owned(),
            expected_workbench_revision: 0,
            exact_item_id: Some("SEL-00002".to_owned()),
            output_root: database
                .path()
                .parent()
                .unwrap()
                .join("failed-source-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-24T02:02:00Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "failed-source-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec!["SEL-00002".to_owned()],
            },
            event_id: "failed-source-task:failed".to_owned(),
            occurred_at: "2026-08-24T02:03:00Z".to_owned(),
        })
        .unwrap();
    let failed_before = store.load_task("failed-source-task").unwrap().unwrap();
    assert_eq!(failed_before.runtime.status, TaskLifecycleStatus::Failed);
    assert_eq!(
        store
            .append_task_from_source_batch(&AppendTaskFromSourceBatchRequest {
                task_id: "failed-source-task".to_owned(),
                expected_task_revision: 1,
                item_ids: vec!["SEL-00003".to_owned()],
                event_id: "failed-source-task:append-rejected".to_owned(),
                occurred_at: "2026-08-24T02:04:00Z".to_owned(),
            })
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );

    let retried = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "failed-source-task".to_owned(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryItems,
                item_ids: vec!["SEL-00002".to_owned()],
            },
            event_id: "failed-source-task:retry".to_owned(),
            occurred_at: "2026-08-24T02:05:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(retried.runtime.selected_item_ids, ["SEL-00002"]);
    assert_eq!(retried.runtime.queued_item_ids, ["SEL-00002"]);

    let successor = store
        .create_task_from_historical_source_batch(&CreateTaskFromHistoricalSourceBatchRequest {
            source_task_id: "failed-source-task".to_owned(),
            expected_source_task_revision: 2,
            new_task_id: "failed-source-successor".to_owned(),
            item_ids: vec!["SEL-00003".to_owned(), "SEL-05000".to_owned()],
            output_root: database.path().parent().unwrap().join("successor-output"),
            layout: DownloadLayout::Direct,
            concurrency: 3,
            created_at: "2026-08-24T02:06:00Z".to_owned(),
            reacquire_item_ids: vec![],
        })
        .unwrap();
    assert_eq!(successor.snapshot.batch_id, BATCH_ID);
    assert_eq!(successor.runtime.concurrency, 3);
    assert_eq!(
        successor.runtime.selected_item_ids,
        ["SEL-00003", "SEL-05000"]
    );
    let failed_after = store.load_task("failed-source-task").unwrap().unwrap();
    assert_eq!(failed_after.runtime.selected_item_ids, ["SEL-00002"]);
}
