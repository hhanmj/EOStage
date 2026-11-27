use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout,
    DownloadOutcome, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreTaskPageRequest,
    StoreTaskSourceBatchPageRequest, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskStateCommandKind,
};
use insar_store::{StoreTaskTransferTelemetryUpdate, TaskStore};
use serde_json::{json, Map};

static NEXT_TEMP_DB_ID: AtomicU64 = AtomicU64::new(0);

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let seq = NEXT_TEMP_DB_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-bounded-task-lifecycle-{}-{nonce}-{seq}.sqlite3",
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

fn item(item_id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("{item_id}.zip"),
        acquisition_datetime: Some("2026-08-15T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed_task(store: &mut TaskStore, output_root: &Path) {
    seed_task_with_selected(
        store,
        output_root,
        vec![
            "ITEM-A".to_owned(),
            "ITEM-B".to_owned(),
            "ITEM-C".to_owned(),
        ],
    );
}

fn seed_task_with_selected(
    store: &mut TaskStore,
    output_root: &Path,
    selected_item_ids: Vec<String>,
) {
    let batch_id = "bounded-task-batch";
    let task_id = "bounded-task";
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: "2026-08-15T00:00:00Z".to_owned(),
                aoi_name: "Bounded AOI".to_owned(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]]
                    })
                    .as_object()
                    .expect("polygon")
                    .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates: vec![item("ITEM-A"), item("ITEM-B"), item("ITEM-C")],
            },
            workbench: SearchWorkbenchState {
                batch_id: batch_id.to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 100,
                revision: 0,
            },
        })
        .expect("save batch");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id: batch_id.to_owned(),
            selected_item_ids,
            checked_item_ids: vec!["ITEM-B".to_owned()],
            page_index: 0,
            page_size: 100,
            output_root: output_root.join("bounded-output"),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-08-15T00:00:01Z".to_owned(),
        })
        .expect("create task");
}

#[test]
fn bounded_task_page_returns_only_one_page_with_item_progress() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, database.path().parent().expect("parent"));

    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 42,
                total_bytes: Some(100),
                bytes_per_second: 7.5,
                attempt: 2,
                updated_at: "2026-08-15T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("save progress");
    let activated = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "bounded-activate".to_owned(),
            occurred_at: "2026-08-15T00:00:03Z".to_owned(),
        })
        .expect("activate persisted progress");

    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: activated.runtime.revision,
            page_index: 0,
            page_size: 2,
        })
        .expect("load bounded task page");

    assert_eq!(page.total_count, 3);
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].item.item_id, "ITEM-A");
    assert_eq!(page.items[1].item.item_id, "ITEM-B");
    assert_eq!(page.items[1].status, DownloadItemStatus::Active);
    assert_eq!(page.items[1].bytes_completed, 42);
    assert_eq!(page.items[1].total_bytes, Some(100));
}

#[test]
fn task_level_cancel_and_retry_expand_inside_store_without_item_ids() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, database.path().parent().expect("parent"));

    let cancelled = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: vec![],
            },
            event_id: "bounded-cancel".to_owned(),
            occurred_at: "2026-08-15T00:00:02Z".to_owned(),
        })
        .expect("cancel task");
    assert_eq!(cancelled.runtime.status, TaskLifecycleStatus::Cancelled);
    assert_eq!(cancelled.runtime.cancelled_item_ids.len(), 3);

    let retried = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryTask,
                item_ids: vec![],
            },
            event_id: "bounded-retry".to_owned(),
            occurred_at: "2026-08-15T00:00:03Z".to_owned(),
        })
        .expect("retry task");
    assert_eq!(retried.runtime.status, TaskLifecycleStatus::Queued);
    assert_eq!(retried.runtime.queued_item_ids.len(), 3);
    assert!(retried.runtime.cancelled_item_ids.is_empty());
}

#[test]
fn core_restore_task_summary_exposes_bounded_aggregate_live_telemetry() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, database.path().parent().expect("parent"));

    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-A".to_owned(),
                outcome: DownloadOutcome::Completed,
                destination: database.path().parent().unwrap().join("ITEM-A.zip"),
                bytes_written: 100,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-15T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("save completed result");
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 25,
                total_bytes: Some(200),
                bytes_per_second: 10.0,
                attempt: 1,
                updated_at: "2026-08-15T00:00:03Z".to_owned(),
            },
            1,
        )
        .expect("save active progress");
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-C".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 30,
                total_bytes: Some(300),
                bytes_per_second: 12.0,
                attempt: 1,
                updated_at: "2026-08-15T00:00:04Z".to_owned(),
            },
            2,
        )
        .expect("save second active progress");
    let completed = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["ITEM-A".to_owned()],
            },
            event_id: "mark-item-a-complete".to_owned(),
            occurred_at: "2026-08-15T00:00:05Z".to_owned(),
        })
        .expect("mark completed");
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: completed.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["ITEM-B".to_owned(), "ITEM-C".to_owned()],
            },
            event_id: "activate-live-items".to_owned(),
            occurred_at: "2026-08-15T00:00:06Z".to_owned(),
        })
        .expect("activate live items");

    let core = store.core_restore(true).expect("restore");
    let task = core
        .tasks
        .iter()
        .find(|task| task.task_id == "bounded-task")
        .expect("task summary");
    assert_eq!(task.terminal_bytes, 100);
    assert_eq!(task.active_bytes, 55);
    assert_eq!(task.other_bytes, 0);
    assert_eq!(task.transferred_bytes, 155);
    assert_eq!(task.total_bytes, Some(600));
    assert_eq!(task.bytes_per_second, 22.0);
    assert_eq!(
        task.active_item.as_ref().map(|item| item.item_id.as_str()),
        Some("ITEM-B")
    );
    assert_eq!(
        task.active_item.as_ref().map(|item| item.bytes_completed),
        Some(25),
    );
    assert_eq!(
        task.active_item.as_ref().and_then(|item| item.total_bytes),
        Some(200)
    );

    let serialized = serde_json::to_value(task).expect("serialize summary");
    assert!(serialized.get("candidates").is_none());
    assert!(serialized.get("progress").is_none());
    assert!(serialized.get("results").is_none());
}

#[test]
fn global_task_pause_reclassifies_active_members_without_losing_transfer_totals() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task_with_selected(
        &mut store,
        database.path().parent().expect("parent"),
        vec!["ITEM-B".to_owned()],
    );
    let active = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "activate-before-global-pause".to_owned(),
            occurred_at: "2026-08-20T00:00:01Z".to_owned(),
        })
        .expect("activate item");
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 25,
                total_bytes: Some(200),
                bytes_per_second: 10.0,
                attempt: 1,
                updated_at: "2026-08-20T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("save active progress");
    store
        .save_task_transfer_telemetry(&StoreTaskTransferTelemetryUpdate {
            task_id: "bounded-task".to_owned(),
            expected_revision: active.runtime.revision,
            transfer_active_millis: 3_210,
            updated_at: "2026-08-20T00:00:03Z".to_owned(),
        })
        .expect("save active transfer clock");
    let paused = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: active.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "global-pause-with-active-member".to_owned(),
            occurred_at: "2026-08-20T00:00:04Z".to_owned(),
        })
        .expect("pause whole task");
    assert!(paused.runtime.task_paused);
    assert_eq!(paused.runtime.active_item_ids, vec!["ITEM-B"]);

    let task = store
        .core_restore(false)
        .expect("restore globally paused task")
        .tasks
        .into_iter()
        .find(|task| task.task_id == "bounded-task")
        .expect("bounded task summary");
    assert_eq!(task.status, TaskLifecycleStatus::Paused);
    assert_eq!(task.active_count, 0);
    assert_eq!(task.paused_count, 1);
    assert_eq!(task.active_bytes, 0);
    assert_eq!(task.other_bytes, 25);
    assert_eq!(task.transferred_bytes, 25);
    assert_eq!(task.total_bytes, Some(200));
    assert_eq!(task.bytes_per_second, 0.0);
    assert_eq!(task.transfer_active_millis, 3_210);
    assert!(task.active_item.is_none());
}

#[test]
fn core_restore_keeps_newly_active_item_visible_before_first_progress_sample() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task_with_selected(
        &mut store,
        database.path().parent().expect("parent"),
        vec!["ITEM-B".to_owned()],
    );

    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "activate-before-first-progress".to_owned(),
            occurred_at: "2026-08-17T00:00:01Z".to_owned(),
        })
        .expect("activate without progress");

    let core = store
        .core_restore(false)
        .expect("newly active task remains a valid live projection");
    let task = core
        .tasks
        .iter()
        .find(|task| task.task_id == "bounded-task")
        .expect("newly active task is visible");
    assert_eq!(task.status, TaskLifecycleStatus::Running);
    assert_eq!(task.active_count, 1);
    assert_eq!(task.active_bytes, 0);
    assert_eq!(task.transferred_bytes, 0);
    assert_eq!(task.total_bytes, None);
    assert_eq!(task.bytes_per_second, 0.0);
    let active = task
        .active_item
        .as_ref()
        .expect("representative active item");
    assert_eq!(active.item_id, "ITEM-B");
    assert_eq!(active.bytes_completed, 0);
    assert_eq!(active.total_bytes, None);
}

#[test]
fn paused_and_cancelled_before_result_keep_durable_progress_in_transferred_bytes() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task_with_selected(
        &mut store,
        database.path().parent().expect("parent"),
        vec!["ITEM-B".to_owned()],
    );
    let active = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "activate-for-transfer".to_owned(),
            occurred_at: "2026-08-17T00:00:01Z".to_owned(),
        })
        .expect("activate");
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 25,
                total_bytes: Some(200),
                bytes_per_second: 10.0,
                attempt: 1,
                updated_at: "2026-08-17T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("progress");
    let paused = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: active.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "pause-for-transfer".to_owned(),
            occurred_at: "2026-08-17T00:00:03Z".to_owned(),
        })
        .expect("pause");
    let core = store.core_restore(true).expect("paused restore");
    let task = core
        .tasks
        .iter()
        .find(|task| task.task_id == "bounded-task")
        .unwrap();
    assert_eq!(task.terminal_bytes, 0);
    assert_eq!(task.active_bytes, 0);
    assert_eq!(task.other_bytes, 25);
    assert_eq!(task.transferred_bytes, 25);
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: paused.runtime.revision,
            page_index: 0,
            page_size: 3,
        })
        .unwrap();
    assert_eq!(page.items[0].bytes_completed, 25);
    assert_eq!(page.items[0].total_bytes, Some(200));
    assert_eq!(page.items[0].bytes_per_second, 0.0);

    let cancelled = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: paused.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "cancel-before-result".to_owned(),
            occurred_at: "2026-08-17T00:00:04Z".to_owned(),
        })
        .expect("cancel before result");
    let core = store.core_restore(true).expect("cancelled restore");
    let task = core
        .tasks
        .iter()
        .find(|task| task.task_id == "bounded-task")
        .unwrap();
    assert_eq!(task.terminal_bytes, 25);
    assert_eq!(task.active_bytes, 0);
    assert_eq!(task.other_bytes, 0);
    assert_eq!(task.transferred_bytes, 25);
    assert_eq!(task.total_bytes, Some(200));
    assert_eq!(task.bytes_per_second, 0.0);
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: cancelled.runtime.revision,
            page_index: 0,
            page_size: 3,
        })
        .expect("cancelled page");
    assert_eq!(page.items[0].bytes_completed, 25);
    assert_eq!(page.items[0].total_bytes, Some(200));
    assert_eq!(page.items[0].bytes_per_second, 0.0);
}

#[test]
fn task_page_uses_runtime_state_before_stale_progress_or_results_and_survives_reopen() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, database.path().parent().expect("parent"));

    let activated = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "activate-item-b".to_owned(),
            occurred_at: "2026-08-15T00:00:01Z".to_owned(),
        })
        .expect("activate item");
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 25,
                total_bytes: Some(200),
                bytes_per_second: 10.0,
                attempt: 1,
                updated_at: "2026-08-15T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("save live progress");
    let paused = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: activated.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "pause-item-b".to_owned(),
            occurred_at: "2026-08-15T00:00:03Z".to_owned(),
        })
        .expect("pause item");
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: activated.runtime.revision,
            page_index: 0,
            page_size: 3,
        })
        .expect("paused page");
    assert_eq!(page.revision, paused.runtime.revision);
    assert_eq!(page.items[1].status, DownloadItemStatus::Paused);
    assert_eq!(page.items[1].bytes_completed, 25);
    assert_eq!(page.items[1].total_bytes, Some(200));
    assert_eq!(page.items[1].bytes_per_second, 0.0);

    let cancelled = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: paused.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "cancel-item-b".to_owned(),
            occurred_at: "2026-08-15T00:00:04Z".to_owned(),
        })
        .expect("cancel item before result");
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: cancelled.runtime.revision,
            page_index: 0,
            page_size: 3,
        })
        .expect("cancelled page");
    assert_eq!(page.items[1].status, DownloadItemStatus::Cancelled);
    assert_eq!(page.items[1].bytes_completed, 25);
    assert_eq!(page.items[1].total_bytes, Some(200));
    assert_eq!(page.items[1].bytes_per_second, 0.0);

    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                outcome: DownloadOutcome::Cancelled,
                destination: database.path().parent().unwrap().join("ITEM-B.zip"),
                bytes_written: 25,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: "NISAR_SUBSET_V2_CANCELLED".to_owned(),
                    reason_zh: "legacy synthetic cancellation marker".to_owned(),
                    retryable: false,
                }),
                finished_at: "2026-08-15T00:00:05Z".to_owned(),
            },
            1,
        )
        .expect("save cancelled result");
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: cancelled.runtime.revision,
            page_index: 0,
            page_size: 3,
        })
        .expect("legacy cancelled result page");
    assert!(page.items[1].failure.is_none());
    let source_page = store
        .load_task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "bounded-task".to_owned(),
            page_index: 0,
            page_size: 3,
        })
        .expect("legacy cancelled source page");
    assert!(source_page.items[1].failure.is_none());
    let retried = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: cancelled.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "retry-item-b".to_owned(),
            occurred_at: "2026-08-15T00:00:06Z".to_owned(),
        })
        .expect("retry item");
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: retried.runtime.revision,
            page_index: 0,
            page_size: 3,
        })
        .expect("retry page ignores stale terminal result");
    assert_eq!(page.items[1].status, DownloadItemStatus::Queued);
    assert_eq!(page.items[1].bytes_per_second, 0.0);
}

#[test]
fn retry_progress_without_a_new_total_preserves_the_last_known_total_through_interruption_and_reopen(
) {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, database.path().parent().expect("parent"));

    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 25,
                total_bytes: Some(200),
                bytes_per_second: 10.0,
                attempt: 1,
                updated_at: "2026-08-15T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("save known progress");
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                outcome: DownloadOutcome::Cancelled,
                destination: database.path().parent().unwrap().join("ITEM-B.zip"),
                bytes_written: 25,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-15T00:00:03Z".to_owned(),
            },
            1,
        )
        .expect("save cancelled result");
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 25,
                total_bytes: None,
                bytes_per_second: 0.0,
                attempt: 2,
                updated_at: "2026-08-15T00:00:04Z".to_owned(),
            },
            2,
        )
        .expect("retry progress retains previous total");
    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 25,
                total_bytes: None,
                bytes_per_second: 0.0,
                attempt: 2,
                updated_at: "2026-08-15T00:00:04Z".to_owned(),
            },
            3,
        )
        .expect("successive unknown progress retains previous total");
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                outcome: DownloadOutcome::Interrupted,
                destination: database.path().parent().unwrap().join("ITEM-B.zip"),
                bytes_written: 25,
                attempt_count: 2,
                failure: None,
                finished_at: "2026-08-15T00:00:05Z".to_owned(),
            },
            4,
        )
        .expect("save interrupted result");

    drop(store);
    let reopened = TaskStore::open(database.path()).expect("reopen");
    let page = reopened
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            page_index: 0,
            page_size: 3,
        })
        .expect("reopened page");
    assert_eq!(page.items[1].total_bytes, Some(200));
}

#[test]
fn terminal_task_page_keeps_last_known_total_and_old_rows_remain_honest() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).expect("open store");
    seed_task(&mut store, database.path().parent().expect("parent"));

    store
        .save_download_progress(
            &DownloadItemProgress {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                status: DownloadItemStatus::Active,
                bytes_completed: 25,
                total_bytes: Some(200),
                bytes_per_second: 10.0,
                attempt: 1,
                updated_at: "2026-08-15T00:00:02Z".to_owned(),
            },
            0,
        )
        .expect("save progress");
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-B".to_owned(),
                outcome: DownloadOutcome::Cancelled,
                destination: database.path().parent().unwrap().join("ITEM-B.zip"),
                bytes_written: 25,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-15T00:00:03Z".to_owned(),
            },
            1,
        )
        .expect("save terminal result");
    store
        .save_download_result(
            &DownloadItemResult {
                task_id: "bounded-task".to_owned(),
                item_id: "ITEM-C".to_owned(),
                outcome: DownloadOutcome::Interrupted,
                destination: database.path().parent().unwrap().join("ITEM-C.zip"),
                bytes_written: 7,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-08-15T00:00:04Z".to_owned(),
            },
            2,
        )
        .expect("save old-style terminal result");
    let cancelled = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelItems,
                item_ids: vec!["ITEM-B".to_owned()],
            },
            event_id: "terminal-cancel-b".to_owned(),
            occurred_at: "2026-08-15T00:00:05Z".to_owned(),
        })
        .expect("runtime cancellation");

    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: cancelled.runtime.revision,
            page_index: 0,
            page_size: 3,
        })
        .expect("load page");
    assert_eq!(page.items[1].status, DownloadItemStatus::Cancelled);
    assert_eq!(page.items[1].bytes_completed, 25);
    assert_eq!(page.items[1].total_bytes, Some(200));
    assert_eq!(page.items[1].bytes_per_second, 0.0);
    assert_eq!(page.items[2].status, DownloadItemStatus::Queued);
    assert_eq!(
        page.items[2].total_bytes, None,
        "old result rows do not invent totals"
    );
    assert_eq!(page.items[2].bytes_per_second, 0.0);

    drop(store);
    let reopened = TaskStore::open(database.path()).expect("reopen migrated store");
    let page = reopened
        .load_task_page(&StoreTaskPageRequest {
            task_id: "bounded-task".to_owned(),
            expected_revision: cancelled.runtime.revision,
            page_index: 0,
            page_size: 3,
        })
        .expect("reload page after reopen");
    assert_eq!(page.items[1].total_bytes, Some(200));
    assert_eq!(page.items[2].total_bytes, None);
}
