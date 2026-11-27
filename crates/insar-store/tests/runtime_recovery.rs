use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout,
    DownloadOutcome, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem,
    StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreDeleteHistoryRequest,
    StoreRestoreRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreTaskPageRequest,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
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
            "insar-store-recovery-{}-{nonce}-{seq}.sqlite3",
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

fn seed_task(store: &mut TaskStore, output_root: &Path) {
    let candidates = ["S1-A", "S1-B", "S1-C"]
        .into_iter()
        .map(|item_id| SnapshotItem {
            item_id: item_id.to_string(),
            provider_id: "asf.sentinel1".to_string(),
            display_name: item_id.to_string(),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        })
        .collect();
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "batch-recovery".to_string(),
        provider_id: "asf.sentinel1".to_string(),
        created_at: "2024-01-15T00:00:00Z".to_string(),
        aoi_name: "四川省雅安市".to_string(),
        aoi_geojson: Some(map(json!({
            "type": "Polygon",
            "coordinates": [[[102.2, 29.7], [103.2, 29.7], [103.2, 30.7], [102.2, 30.7], [102.2, 29.7]]]
        }))),
        query: map(json!({"product_type": "SLC"})),
        search_stats: None,
        candidates,
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch,
            workbench: SearchWorkbenchState {
                batch_id: "batch-recovery".to_string(),
                checked_item_ids: vec!["S1-A".to_string(), "S1-C".to_string()],
                page_index: 7,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-recovery".to_string(),
            batch_id: "batch-recovery".to_string(),
            selected_item_ids: vec!["S1-A".to_string(), "S1-B".to_string()],
            checked_item_ids: vec!["S1-A".to_string()],
            page_index: 7,
            page_size: 24,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 2,
            created_at: "2024-01-15T01:00:00Z".to_string(),
        })
        .unwrap();
}

fn update(
    revision: u64,
    kind: TaskStateCommandKind,
    item_ids: &[&str],
    event_id: &str,
) -> StoreUpdateTaskRequest {
    StoreUpdateTaskRequest {
        task_id: "task-recovery".to_string(),
        expected_revision: revision,
        command: StoreTaskCommand {
            kind,
            item_ids: item_ids.iter().map(|value| (*value).to_string()).collect(),
        },
        event_id: event_id.to_string(),
        occurred_at: format!("2024-01-15T01:00:0{}Z", revision + 1),
    }
}

#[test]
fn transitions_are_atomic_idempotent_and_revision_guarded() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());

    let pause_item = update(0, TaskStateCommandKind::PauseItems, &["S1-A"], "event-1");
    let paused = store.update_task(&pause_item).unwrap();
    assert_eq!(paused.runtime.revision, 1);
    assert_eq!(paused.runtime.paused_item_ids, ["S1-A"]);
    assert_eq!(paused.runtime.queued_item_ids, ["S1-B"]);
    assert!(paused.runtime.active_item_ids.is_empty());

    let replay = store.update_task(&pause_item).unwrap();
    assert_eq!(replay.runtime, paused.runtime);
    assert_eq!(store.list_task_events("task-recovery").unwrap().len(), 2);

    let resumed = store
        .update_task(&update(
            1,
            TaskStateCommandKind::ResumeItems,
            &["S1-A"],
            "event-2",
        ))
        .unwrap();
    assert!(resumed.runtime.paused_item_ids.is_empty());
    assert_eq!(resumed.runtime.queued_item_ids, ["S1-A", "S1-B"]);
    assert!(resumed.runtime.active_item_ids.is_empty());
    assert!(resumed.runtime.active_item_ids.len() <= resumed.runtime.concurrency as usize);

    let stale = update(1, TaskStateCommandKind::PauseTask, &[], "event-stale");
    assert_eq!(
        store.update_task(&stale).unwrap_err().code(),
        "V3_STORE_REVISION_CONFLICT"
    );
    assert_eq!(store.list_task_events("task-recovery").unwrap().len(), 3);

    let collision = update(2, TaskStateCommandKind::PauseTask, &[], "event-1");
    assert_eq!(
        store.update_task(&collision).unwrap_err().code(),
        "V3_STORE_ALREADY_EXISTS"
    );
}

#[test]
fn task_level_events_stamp_effective_items_and_keep_strict_replay() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());
    store
        .update_task(&update(
            0,
            TaskStateCommandKind::ActivateItems,
            &["S1-A"],
            "stamp-activate",
        ))
        .unwrap();
    store
        .update_task(&update(
            1,
            TaskStateCommandKind::PauseItems,
            &["S1-B"],
            "stamp-pause",
        ))
        .unwrap();
    let cancel = update(2, TaskStateCommandKind::CancelTask, &[], "stamp-cancel");
    let cancelled = store.update_task(&cancel).unwrap();
    let event = store
        .list_task_events("task-recovery")
        .unwrap()
        .into_iter()
        .find(|e| e.event_id == "stamp-cancel")
        .unwrap();
    assert_eq!(
        event.payload["command"]["item_ids"],
        json!(["S1-A", "S1-B"])
    );
    assert_eq!(
        store.update_task(&cancel).unwrap().runtime,
        cancelled.runtime
    );
    let mut conflict = cancel.clone();
    conflict.occurred_at = "2024-01-15T01:00:59Z".into();
    assert_eq!(
        store.update_task(&conflict).unwrap_err().code(),
        "V3_STORE_ALREADY_EXISTS"
    );
    let mut collision_task = cancel.clone();
    collision_task.task_id = "other-task".into();
    assert_eq!(
        store.update_task(&collision_task).unwrap_err().code(),
        "V3_STORE_ALREADY_EXISTS"
    );
    let mut collision_items = cancel.clone();
    collision_items.command.item_ids = vec!["S1-A".into()];
    assert_eq!(
        store.update_task(&collision_items).unwrap_err().code(),
        "V3_STORE_ALREADY_EXISTS"
    );
    let retry = update(
        cancelled.runtime.revision,
        TaskStateCommandKind::RetryTask,
        &[],
        "stamp-retry",
    );
    let retried = store.update_task(&retry).unwrap();
    let event = store
        .list_task_events("task-recovery")
        .unwrap()
        .into_iter()
        .find(|e| e.event_id == "stamp-retry")
        .unwrap();
    assert_eq!(
        event.payload["command"]["item_ids"],
        json!(["S1-A", "S1-B"])
    );
    assert_eq!(store.update_task(&retry).unwrap().runtime, retried.runtime);
    let mut conflict = retry.clone();
    conflict.expected_revision += 1;
    assert_eq!(
        store.update_task(&conflict).unwrap_err().code(),
        "V3_STORE_ALREADY_EXISTS"
    );
}

#[test]
fn task_level_stamp_captures_active_paused_queued_and_failed_items() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-mixed-stamp".into(),
            batch_id: "batch-recovery".into(),
            selected_item_ids: vec!["S1-A".into(), "S1-B".into(), "S1-C".into()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: database.path().parent().unwrap().to_path_buf(),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 2,
            created_at: "2024-01-15T01:00:00Z".into(),
        })
        .unwrap();
    let mut action = update(
        0,
        TaskStateCommandKind::ActivateItems,
        &["S1-A"],
        "mixed-activate",
    );
    action.task_id = "task-mixed-stamp".into();
    store.update_task(&action).unwrap();
    let mut action = update(
        1,
        TaskStateCommandKind::PauseItems,
        &["S1-B"],
        "mixed-pause",
    );
    action.task_id = "task-mixed-stamp".into();
    store.update_task(&action).unwrap();
    let mut action = update(2, TaskStateCommandKind::CancelTask, &[], "mixed-cancel");
    action.task_id = "task-mixed-stamp".into();
    let cancelled = store.update_task(&action).unwrap();
    let events = store.list_task_events("task-mixed-stamp").unwrap();
    let cancel = events
        .iter()
        .find(|e| e.event_id == "mixed-cancel")
        .unwrap();
    assert_eq!(
        cancel.payload["command"]["item_ids"],
        json!(["S1-A", "S1-B", "S1-C"])
    );
    assert_eq!(cancelled.runtime.cancelled_item_ids.len(), 3);

    let mut action = update(
        cancelled.runtime.revision,
        TaskStateCommandKind::RetryTask,
        &[],
        "mixed-retry",
    );
    action.task_id = "task-mixed-stamp".into();
    let retried = store.update_task(&action).unwrap();
    let mut action = update(
        retried.runtime.revision,
        TaskStateCommandKind::MarkFailed,
        &["S1-A"],
        "mixed-failed",
    );
    action.task_id = "task-mixed-stamp".into();
    let failed = store.update_task(&action).unwrap();
    let mut action = update(
        failed.runtime.revision,
        TaskStateCommandKind::CancelItems,
        &["S1-B"],
        "mixed-cancel-item",
    );
    action.task_id = "task-mixed-stamp".into();
    let mixed = store.update_task(&action).unwrap();
    let mut action = update(
        mixed.runtime.revision,
        TaskStateCommandKind::RetryTask,
        &[],
        "mixed-retry-failed-cancelled",
    );
    action.task_id = "task-mixed-stamp".into();
    store.update_task(&action).unwrap();
    let events = store.list_task_events("task-mixed-stamp").unwrap();
    let retry = events
        .iter()
        .find(|e| e.event_id == "mixed-retry-failed-cancelled")
        .unwrap();
    assert_eq!(
        retry.payload["command"]["item_ids"],
        json!(["S1-A", "S1-B"])
    );
}

#[test]
fn completed_plus_cancelled_items_make_the_task_cancelled_not_completed() {
    let database = TempDatabase::new();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        seed_task(&mut store, database.path().parent().unwrap());
        store
            .update_task(&update(
                0,
                TaskStateCommandKind::MarkCompleted,
                &["S1-A"],
                "event-completed",
            ))
            .unwrap();
        let mixed = store
            .update_task(&update(
                1,
                TaskStateCommandKind::CancelItems,
                &["S1-B"],
                "event-cancelled",
            ))
            .unwrap();
        assert_eq!(mixed.runtime.status, TaskLifecycleStatus::Cancelled);
    }

    let reopened = TaskStore::open(database.path()).unwrap();
    let restored = reopened.load_task("task-recovery").unwrap().unwrap();
    assert_eq!(restored.runtime.completed_item_ids, ["S1-A"]);
    assert_eq!(restored.runtime.cancelled_item_ids, ["S1-B"]);
    assert_eq!(restored.runtime.status, TaskLifecycleStatus::Cancelled);
}

#[test]
fn paused_task_reopens_from_its_own_snapshot_and_preserves_ui_state() {
    let database = TempDatabase::new();
    let output_root = database.path().parent().unwrap().join("用户选择目录");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        seed_task(&mut store, &output_root);
        store
            .update_task(&update(
                0,
                TaskStateCommandKind::PauseItems,
                &["S1-A"],
                "event-item-pause",
            ))
            .unwrap();
        let paused = store
            .update_task(&update(
                1,
                TaskStateCommandKind::PauseTask,
                &[],
                "event-task-pause",
            ))
            .unwrap();
        assert_eq!(paused.runtime.status, TaskLifecycleStatus::Paused);
    }

    let reopened = TaskStore::open(database.path()).unwrap();
    let restored = reopened.load_task("task-recovery").unwrap().unwrap();
    assert_eq!(restored.snapshot.aoi_name, "四川省雅安市");
    assert_eq!(restored.snapshot.candidates.len(), 3);
    assert_eq!(
        restored.snapshot.initial_selected_item_ids,
        ["S1-A", "S1-B"]
    );
    assert_eq!(restored.snapshot.output_root, output_root);
    assert_eq!(restored.snapshot.layout, DownloadLayout::Sentinel1Slc);
    assert_eq!(restored.runtime.checked_item_ids, ["S1-A"]);
    assert_eq!(restored.runtime.page_index, 7);
    assert_eq!(restored.runtime.paused_item_ids, ["S1-A"]);
    assert_eq!(restored.runtime.queued_item_ids, ["S1-B"]);
    assert!(restored.runtime.task_paused);
    assert_eq!(restored.runtime.revision, 2);
    assert_eq!(restored.runtime.status, TaskLifecycleStatus::Paused);
}

#[test]
fn a_task_left_with_only_paused_items_is_paused_and_survives_reopen() {
    let database = TempDatabase::new();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        seed_task(&mut store, database.path().parent().unwrap());
        store
            .update_task(&update(
                0,
                TaskStateCommandKind::ActivateItems,
                &["S1-A", "S1-B"],
                "event-activate",
            ))
            .unwrap();
        let one_paused = store
            .update_task(&update(
                1,
                TaskStateCommandKind::PauseItems,
                &["S1-A"],
                "event-item-pause",
            ))
            .unwrap();
        assert_eq!(one_paused.runtime.status, TaskLifecycleStatus::Running);
        let last_runnable_done = store
            .update_task(&update(
                2,
                TaskStateCommandKind::MarkCompleted,
                &["S1-B"],
                "event-completed",
            ))
            .unwrap();
        assert_eq!(
            last_runnable_done.runtime.status,
            TaskLifecycleStatus::Paused
        );
        assert!(!last_runnable_done.runtime.task_paused);
        assert_eq!(last_runnable_done.runtime.paused_item_ids, ["S1-A"]);
    }

    let reopened = TaskStore::open(database.path()).unwrap();
    let restored = reopened.load_task("task-recovery").unwrap().unwrap();
    assert_eq!(restored.runtime.status, TaskLifecycleStatus::Paused);
    assert!(reopened
        .load_scheduler_tasks()
        .unwrap()
        .iter()
        .any(|task| task.runtime.task_id == "task-recovery"));
}

#[test]
fn pausing_every_remaining_item_pauses_the_task() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());
    let paused = store
        .update_task(&update(
            0,
            TaskStateCommandKind::PauseItems,
            &["S1-A", "S1-B"],
            "event-pause-all-items",
        ))
        .unwrap();
    assert_eq!(paused.runtime.status, TaskLifecycleStatus::Paused);
    assert!(!paused.runtime.task_paused);

    let resumed = store
        .update_task(&update(
            1,
            TaskStateCommandKind::ResumeItems,
            &["S1-A"],
            "event-resume-one",
        ))
        .unwrap();
    assert_eq!(resumed.runtime.status, TaskLifecycleStatus::Queued);
}

#[test]
fn active_restore_is_compact_but_full_restore_hydrates_the_source_batch() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());

    let active = store
        .restore(&StoreRestoreRequest {
            include_terminal: false,
        })
        .unwrap();
    let full = store
        .restore(&StoreRestoreRequest {
            include_terminal: true,
        })
        .unwrap();

    assert_eq!(active.tasks[0].candidates.len(), 2);
    assert_eq!(full.tasks[0].candidates.len(), 3);
}

#[test]
fn scheduler_restore_does_not_deserialize_the_active_search_workbench() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());
    drop(store);

    let connection = rusqlite::Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER search_batches_are_frozen;
             UPDATE search_batches SET snapshot_json = '{not-json'
             WHERE batch_id = 'batch-recovery';
             CREATE TRIGGER search_batches_are_frozen
             BEFORE UPDATE ON search_batches
             BEGIN
                 SELECT RAISE(ABORT, 'frozen search batch cannot be updated');
             END;",
        )
        .unwrap();
    drop(connection);

    let restored = TaskStore::open(database.path())
        .unwrap()
        .load_scheduler_tasks()
        .unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(
        restored[0]
            .snapshot
            .candidates
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["S1-A", "S1-B"],
    );
}

#[test]
fn execution_policy_round_trips_in_existing_snapshot_json_without_schema_change() {
    let database = TempDatabase::new();
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asset_execution_policy/sentinel1_standard_cases.json");
    let fixture: Value = serde_json::from_str(&fs::read_to_string(fixture_path).unwrap()).unwrap();
    let resolver_value = fixture["base_item"].clone();
    let resolver: SnapshotItem = serde_json::from_value(resolver_value).unwrap();
    let missing: SnapshotItem = serde_json::from_value(json!({
        "item_id": "legacy-missing-policy",
        "provider_id": "asf.catalog",
        "display_name": "legacy-missing-policy.zip",
        "metadata": {},
        "payload": {"download_url": "https://example.invalid/legacy-missing-policy.zip"}
    }))
    .unwrap();
    let explicit_legacy: SnapshotItem = serde_json::from_value(json!({
        "item_id": "explicit-legacy-policy",
        "provider_id": "asf.catalog",
        "display_name": "explicit-legacy-policy.zip",
        "metadata": {"dataset": "SENTINEL-1", "processing_level": "RAW"},
        "payload": {"download_url": "https://example.invalid/explicit-legacy-policy.zip"},
        "execution_policy": {
            "authority": "legacy",
            "policy_id": "legacy.v1",
            "collection_profile_id": null
        }
    }))
    .unwrap();
    let expected_policy_json = vec![
        serde_json::to_value(&missing.execution_policy).unwrap(),
        serde_json::to_value(&explicit_legacy.execution_policy).unwrap(),
        serde_json::to_value(&resolver.execution_policy).unwrap(),
    ];
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "policy-round-trip-batch".to_string(),
        provider_id: "asf.catalog".to_string(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
        aoi_name: "fixture-aoi".to_string(),
        aoi_geojson: Some(map(json!({"type": "Polygon", "coordinates": []}))),
        query: map(json!({"dataset": "SENTINEL-1"})),
        search_stats: None,
        candidates: vec![missing, explicit_legacy, resolver],
    };
    let selected_item_ids = batch
        .candidates
        .iter()
        .map(|item| item.item_id.clone())
        .collect::<Vec<_>>();

    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: false,
                workbench: SearchWorkbenchState {
                    batch_id: batch.batch_id.clone(),
                    checked_item_ids: selected_item_ids.clone(),
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                batch,
            })
            .unwrap();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: "policy-round-trip-task".to_string(),
                batch_id: "policy-round-trip-batch".to_string(),
                selected_item_ids: selected_item_ids.clone(),
                checked_item_ids: selected_item_ids,
                page_index: 0,
                page_size: 24,
                output_root: database.path().parent().unwrap().join("policy-output"),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-07-18T00:00:01Z".to_string(),
            })
            .unwrap();
    }

    let old_gate = std::env::var_os("INSAR_RESOLVER_CUTOVER_ASF_SENTINEL1_STANDARD");
    std::env::set_var("INSAR_RESOLVER_CUTOVER_ASF_SENTINEL1_STANDARD", "0");
    let reopened = TaskStore::open(database.path()).unwrap();
    let restored = reopened
        .load_task("policy-round-trip-task")
        .unwrap()
        .unwrap();
    match old_gate {
        Some(value) => std::env::set_var("INSAR_RESOLVER_CUTOVER_ASF_SENTINEL1_STANDARD", value),
        None => std::env::remove_var("INSAR_RESOLVER_CUTOVER_ASF_SENTINEL1_STANDARD"),
    }

    let restored_policy_json = restored
        .snapshot
        .candidates
        .iter()
        .map(|item| serde_json::to_value(&item.execution_policy).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(restored_policy_json, expected_policy_json);

    let connection = rusqlite::Connection::open(database.path()).unwrap();
    let mut statement = connection
        .prepare("PRAGMA table_info(task_snapshots)")
        .unwrap();
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        columns,
        [
            "task_id",
            "batch_id",
            "provider_id",
            "created_at",
            "output_root",
            "layout",
            "snapshot_json",
        ]
    );
}

fn legacy_generation_fixture() -> (TempDatabase, TaskStore, u64, u64, DownloadItemResult) {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());
    let started = store
        .update_task(&update(
            0,
            TaskStateCommandKind::ActivateItems,
            &["S1-A"],
            "legacy-start",
        ))
        .unwrap()
        .runtime
        .revision;
    let cancelled = store
        .update_task(&update(
            started,
            TaskStateCommandKind::CancelTask,
            &[],
            "legacy-cancel",
        ))
        .unwrap()
        .runtime
        .revision;
    let retried = store
        .update_task(&update(
            cancelled,
            TaskStateCommandKind::RetryTask,
            &[],
            "legacy-retry",
        ))
        .unwrap()
        .runtime
        .revision;
    let active = store
        .update_task(&update(
            retried,
            TaskStateCommandKind::ActivateItems,
            &["S1-A"],
            "legacy-active",
        ))
        .unwrap()
        .runtime
        .revision;
    rusqlite::Connection::open(database.path()).unwrap().execute(
        "UPDATE task_events SET payload_json=json_set(payload_json,'$.command.item_ids',json('[]')) WHERE event_id IN ('legacy-cancel','legacy-retry')", [],
    ).unwrap();
    let result = DownloadItemResult {
        task_id: "task-recovery".into(),
        item_id: "S1-A".into(),
        outcome: DownloadOutcome::Cancelled,
        destination: database.path().parent().unwrap().join("legacy-output"),
        bytes_written: 0,
        attempt_count: 1,
        failure: None,
        finished_at: "2024-01-15T01:00:09Z".into(),
    };
    (database, store, started, active, result)
}

#[test]
fn legacy_generation_failure_is_atomic_visible_and_preserves_progress_after_reopen() {
    let (database, mut store, started, active, result) = legacy_generation_fixture();
    let progress = DownloadItemProgress {
        task_id: result.task_id.clone(),
        item_id: result.item_id.clone(),
        status: DownloadItemStatus::Active,
        bytes_completed: 137,
        total_bytes: Some(500),
        bytes_per_second: 9.0,
        attempt: 2,
        updated_at: "2024-01-15T01:00:08Z".into(),
    };
    store.save_scheduler_download_progress(&progress).unwrap();
    let before = store.load_download_snapshot(&result.task_id).unwrap();
    assert!(store
        .finish_scheduler_cancelled_result(&result, started, active, "strict-refusal", false)
        .is_err());
    let finish = store
        .finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            active,
            "legacy-visible",
            false,
        )
        .unwrap();
    assert!(matches!(
        finish,
        insar_store::SchedulerCancelledFinish::Settled(Some(_))
    ));
    drop(store);
    let mut store = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        store.load_download_snapshot(&result.task_id).unwrap(),
        before
    );
    let runtime = store.load_task(&result.task_id).unwrap().unwrap().runtime;
    assert_eq!(runtime.failed_item_ids, ["S1-A"]);
    assert_eq!(runtime.queued_item_ids, ["S1-B"]);
    let summary = store
        .load_task_failure_summaries(std::slice::from_ref(&result.task_id), &[])
        .unwrap();
    assert_eq!(
        summary[&result.task_id].codes,
        ["SCHEDULER_GENERATION_UNCERTAIN"]
    );
    assert_eq!(
        summary[&result.task_id].message_zh.as_deref(),
        Some("无法确认取消/重试代际，需重新检查任务记录")
    );
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: result.task_id.clone(),
            expected_revision: runtime.revision,
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(page.items[0].status, DownloadItemStatus::Failed);
    assert_eq!(page.items[0].bytes_completed, 137);
    assert_eq!(page.items[0].total_bytes, Some(500));
    assert_eq!(page.items[0].bytes_per_second, 0.0);
    assert_eq!(
        page.items[0].failure.as_ref().unwrap().code,
        "SCHEDULER_GENERATION_UNCERTAIN"
    );
    assert!(page.items[1].failure.is_none());
    let core = store.core_restore(true).unwrap();
    let task = core
        .tasks
        .iter()
        .find(|task| task.task_id == result.task_id)
        .unwrap();
    assert_eq!(
        task.failure_codes
            .iter()
            .map(|failure| failure.code.as_str())
            .collect::<Vec<_>>(),
        ["SCHEDULER_GENERATION_UNCERTAIN"]
    );
    let events = store
        .load_task_event_page(&insar_store::StoreTaskEventPageRequest {
            task_id: result.task_id.clone(),
            expected_revision: runtime.revision,
            before_sequence: None,
            page_size: 20,
        })
        .unwrap();
    assert!(events.items.iter().any(|event| event.code.as_deref()
        == Some("SCHEDULER_GENERATION_UNCERTAIN")
        && event.message_zh == "无法确认取消/重试代际，需重新检查任务记录"));
    // A later ordinary failure of the same item replaces the event authority.
    let retried = store
        .update_task(&update(
            runtime.revision,
            TaskStateCommandKind::RetryItems,
            &["S1-A"],
            "after-visible-retry",
        ))
        .unwrap()
        .runtime;
    let ordinary = DownloadItemResult {
        outcome: DownloadOutcome::Failed,
        failure: Some(DownloadFailure {
            code: "ORDINARY_FAILURE".into(),
            reason_zh: "ordinary fixture failure".into(),
            retryable: true,
        }),
        ..result.clone()
    };
    store.save_scheduler_download_result(&ordinary).unwrap();
    store
        .update_scheduler_runtime(&update(
            retried.revision,
            TaskStateCommandKind::MarkFailed,
            &["S1-A"],
            "after-visible-failed",
        ))
        .unwrap();
    let summary = store
        .load_task_failure_summaries(std::slice::from_ref(&result.task_id), &[])
        .unwrap();
    assert_eq!(summary[&result.task_id].codes, ["ORDINARY_FAILURE"]);
    let core = store.core_restore(true).unwrap();
    let task = core
        .tasks
        .iter()
        .find(|task| task.task_id == result.task_id)
        .unwrap();
    assert_eq!(
        task.failure_codes
            .iter()
            .map(|failure| failure.code.as_str())
            .collect::<Vec<_>>(),
        ["ORDINARY_FAILURE"]
    );
    let current = store.load_task(&result.task_id).unwrap().unwrap().runtime;
    let page = store
        .load_task_page(&StoreTaskPageRequest {
            task_id: result.task_id,
            expected_revision: current.revision,
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(
        page.items[0].failure.as_ref().unwrap().code,
        "ORDINARY_FAILURE"
    );
}

#[test]
fn legacy_generation_failures_are_item_scoped_and_latest_ordinary_failure_wins_after_reopen() {
    let (database, mut store, started, active, result) = legacy_generation_fixture();
    let old_a = DownloadItemResult {
        outcome: DownloadOutcome::Failed,
        failure: Some(DownloadFailure {
            code: "OLD_A_FAILURE".into(),
            reason_zh: "old A fixture failure".into(),
            retryable: true,
        }),
        ..result.clone()
    };
    store.save_scheduler_download_result(&old_a).unwrap();
    store
        .finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            active,
            "mixed-uncertain-a",
            false,
        )
        .unwrap();
    let ordinary_b = DownloadItemResult {
        item_id: "S1-B".into(),
        failure: Some(DownloadFailure {
            code: "ORDINARY_B_FAILURE".into(),
            reason_zh: "ordinary B fixture failure".into(),
            retryable: true,
        }),
        ..old_a.clone()
    };
    store.save_scheduler_download_result(&ordinary_b).unwrap();
    let revision = store
        .load_task(&result.task_id)
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    let mixed = store
        .update_scheduler_runtime(&update(
            revision,
            TaskStateCommandKind::MarkFailed,
            &["S1-B"],
            "mixed-ordinary-b",
        ))
        .unwrap();
    let download = store.load_download_snapshot(&result.task_id).unwrap();
    assert_eq!(
        download.results.iter().find(|row| row.item_id == "S1-A"),
        Some(&old_a)
    );

    for _ in 0..2 {
        let summary = store
            .load_task_failure_summaries(std::slice::from_ref(&result.task_id), &[])
            .unwrap();
        assert_eq!(
            summary[&result.task_id].codes,
            ["SCHEDULER_GENERATION_UNCERTAIN", "ORDINARY_B_FAILURE"]
        );
        let core = store.core_restore(true).unwrap();
        let task = core
            .tasks
            .iter()
            .find(|task| task.task_id == result.task_id)
            .unwrap();
        assert_eq!(
            task.failure_codes
                .iter()
                .map(|code| code.code.as_str())
                .collect::<Vec<_>>(),
            ["ORDINARY_B_FAILURE", "SCHEDULER_GENERATION_UNCERTAIN"]
        );
        let page = store
            .load_task_page(&StoreTaskPageRequest {
                task_id: result.task_id.clone(),
                expected_revision: mixed.revision,
                page_index: 0,
                page_size: 24,
            })
            .unwrap();
        assert_eq!(
            page.items[0].failure.as_ref().unwrap().code,
            "SCHEDULER_GENERATION_UNCERTAIN"
        );
        assert_eq!(
            page.items[1].failure.as_ref().unwrap().code,
            "ORDINARY_B_FAILURE"
        );
        let events = store
            .load_task_event_page(&insar_store::StoreTaskEventPageRequest {
                task_id: result.task_id.clone(),
                expected_revision: mixed.revision,
                before_sequence: None,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(
            events
                .items
                .iter()
                .find(|event| event.sequence == mixed.revision - 1)
                .unwrap()
                .code
                .as_deref(),
            Some("SCHEDULER_GENERATION_UNCERTAIN")
        );
        assert_eq!(
            events
                .items
                .iter()
                .find(|event| event.sequence == mixed.revision)
                .unwrap()
                .code
                .as_deref(),
            Some("ORDINARY_B_FAILURE")
        );
        drop(store);
        store = TaskStore::open(database.path()).unwrap();
    }

    let retried = store
        .update_task(&update(
            mixed.revision,
            TaskStateCommandKind::RetryItems,
            &["S1-A"],
            "mixed-retry-a",
        ))
        .unwrap()
        .runtime;
    // Ordinary MarkFailed deliberately carries no error_json and does not
    // replace the old result. It must stop the older uncertain event winning.
    let ordinary = store
        .update_scheduler_runtime(&update(
            retried.revision,
            TaskStateCommandKind::MarkFailed,
            &["S1-A"],
            "mixed-latest-a",
        ))
        .unwrap();
    let latest = store
        .list_task_events(&result.task_id)
        .unwrap()
        .into_iter()
        .find(|event| event.event_id == "mixed-latest-a")
        .unwrap();
    assert!(latest.error.is_none());
    for _ in 0..2 {
        assert_eq!(
            store.load_download_snapshot(&result.task_id).unwrap(),
            download
        );
        let summary = store
            .load_task_failure_summaries(std::slice::from_ref(&result.task_id), &[])
            .unwrap();
        assert_eq!(
            summary[&result.task_id].codes,
            ["ORDINARY_B_FAILURE", "OLD_A_FAILURE"]
        );
        assert_eq!(
            summary[&result.task_id].message_zh.as_deref(),
            Some("ordinary B fixture failure")
        );
        let core = store.core_restore(true).unwrap();
        let task = core
            .tasks
            .iter()
            .find(|task| task.task_id == result.task_id)
            .unwrap();
        assert_eq!(
            task.failure_codes
                .iter()
                .map(|code| code.code.as_str())
                .collect::<Vec<_>>(),
            ["OLD_A_FAILURE", "ORDINARY_B_FAILURE"]
        );
        let page = store
            .load_task_page(&StoreTaskPageRequest {
                task_id: result.task_id.clone(),
                expected_revision: ordinary.revision,
                page_index: 0,
                page_size: 24,
            })
            .unwrap();
        assert_eq!(
            page.items[0].failure.as_ref().unwrap().code,
            "OLD_A_FAILURE"
        );
        assert_eq!(
            page.items[1].failure.as_ref().unwrap().code,
            "ORDINARY_B_FAILURE"
        );
        let events = store
            .load_task_event_page(&insar_store::StoreTaskEventPageRequest {
                task_id: result.task_id.clone(),
                expected_revision: ordinary.revision,
                before_sequence: None,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(
            events
                .items
                .iter()
                .find(|event| event.sequence == ordinary.revision)
                .unwrap()
                .code
                .as_deref(),
            Some("OLD_A_FAILURE")
        );
        assert_eq!(
            events
                .items
                .iter()
                .find(|event| event.sequence == mixed.revision)
                .unwrap()
                .code
                .as_deref(),
            Some("ORDINARY_B_FAILURE")
        );
        // The historical uncertain event remains visible only on its own row.
        assert_eq!(
            events
                .items
                .iter()
                .filter(|event| event.code.as_deref() == Some("SCHEDULER_GENERATION_UNCERTAIN"))
                .count(),
            1
        );
        drop(store);
        store = TaskStore::open(database.path()).unwrap();
    }
}

#[test]
fn legacy_generation_fresh_pair_wins_but_corrupt_payload_never_does() {
    let (database, mut store, started, active, result) = legacy_generation_fixture();
    let cancelled = store
        .update_task(&update(
            active,
            TaskStateCommandKind::CancelItems,
            &["S1-A"],
            "fresh-cancel",
        ))
        .unwrap()
        .runtime;
    let retried = store
        .update_task(&update(
            cancelled.revision,
            TaskStateCommandKind::RetryItems,
            &["S1-A"],
            "fresh-retry",
        ))
        .unwrap()
        .runtime;
    let before = store.load_download_snapshot(&result.task_id).unwrap();
    let finish = store
        .finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            retried.revision,
            "fresh-finish",
            false,
        )
        .unwrap();
    assert!(matches!(
        finish,
        insar_store::SchedulerCancelledFinish::Retried(_)
    ));
    assert_eq!(
        store.load_download_snapshot(&result.task_id).unwrap(),
        before
    );
    rusqlite::Connection::open(database.path())
        .unwrap()
        .execute(
            "UPDATE task_events SET payload_json='{}' WHERE event_id='legacy-cancel'",
            [],
        )
        .unwrap();
    assert!(matches!(
        store.finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            retried.revision,
            "corrupt-finish",
            false
        ),
        Err(insar_store::StoreError::Corrupt(_))
    ));
    assert_eq!(
        store.load_task(&result.task_id).unwrap().unwrap().runtime,
        retried
    );
    assert_eq!(
        store.load_download_snapshot(&result.task_id).unwrap(),
        before
    );
}

#[test]
fn legacy_generation_cross_connection_retry_is_revision_fenced() {
    let (database, mut store, started, observed, result) = legacy_generation_fixture();
    let mut other = TaskStore::open(database.path()).unwrap();
    let cancelled = other
        .update_task(&update(
            observed,
            TaskStateCommandKind::CancelItems,
            &["S1-A"],
            "race-cancel",
        ))
        .unwrap()
        .runtime;
    let retried = other
        .update_task(&update(
            cancelled.revision,
            TaskStateCommandKind::RetryItems,
            &["S1-A"],
            "race-retry",
        ))
        .unwrap()
        .runtime;
    assert!(matches!(
        store.finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            observed,
            "stale-visible",
            false
        ),
        Err(insar_store::StoreError::RevisionConflict)
    ));
    assert_eq!(
        store.load_task(&result.task_id).unwrap().unwrap().runtime,
        retried
    );
    assert!(matches!(
        store
            .finish_scheduler_cancelled_result_with_legacy_policy(
                &result,
                started,
                retried.revision,
                "race-finish",
                false
            )
            .unwrap(),
        insar_store::SchedulerCancelledFinish::Retried(_)
    ));
    assert!(store
        .load_download_snapshot(&result.task_id)
        .unwrap()
        .results
        .is_empty());
}

#[test]
fn legacy_generation_event_write_failure_rolls_back_and_busy_stays_retryable() {
    let (database, mut store, started, active, result) = legacy_generation_fixture();
    let before = store.load_task(&result.task_id).unwrap().unwrap().runtime;
    let download = store.load_download_snapshot(&result.task_id).unwrap();
    let connection = rusqlite::Connection::open(database.path()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_generation_failure BEFORE INSERT ON task_events WHEN NEW.event_type='mark_failed' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(matches!(
        store.finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            active,
            "rollback-visible",
            false
        ),
        Err(insar_store::StoreError::Database(_))
    ));
    assert_eq!(
        store.load_task(&result.task_id).unwrap().unwrap().runtime,
        before
    );
    assert_eq!(
        store.load_download_snapshot(&result.task_id).unwrap(),
        download
    );
    assert!(!store
        .list_task_events(&result.task_id)
        .unwrap()
        .iter()
        .any(|event| event.event_id == "rollback-visible"));
    connection
        .execute_batch("DROP TRIGGER reject_generation_failure; BEGIN IMMEDIATE;")
        .unwrap();
    let error = store
        .finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            active,
            "busy-visible",
            false,
        )
        .unwrap_err();
    assert!(error.is_database_busy());
    connection.execute_batch("ROLLBACK;").unwrap();
    store
        .finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            active,
            "recovered-visible",
            false,
        )
        .unwrap();
    assert_eq!(
        store
            .load_task(&result.task_id)
            .unwrap()
            .unwrap()
            .runtime
            .failed_item_ids,
        ["S1-A"]
    );
}

#[test]
fn legacy_generation_only_a_complete_pair_after_the_last_empty_stamp_proves_retry() {
    let (database, mut store, started, active, result) = legacy_generation_fixture();
    let cancelled = store
        .update_task(&update(
            active,
            TaskStateCommandKind::CancelItems,
            &["S1-A"],
            "pair-cancel",
        ))
        .unwrap()
        .runtime;
    let retried = store
        .update_task(&update(
            cancelled.revision,
            TaskStateCommandKind::RetryItems,
            &["S1-A"],
            "pair-retry",
        ))
        .unwrap()
        .runtime;
    let cancelled = store
        .update_task(&update(
            retried.revision,
            TaskStateCommandKind::CancelTask,
            &[],
            "later-empty-cancel",
        ))
        .unwrap()
        .runtime;
    let retried = store
        .update_task(&update(
            cancelled.revision,
            TaskStateCommandKind::RetryTask,
            &[],
            "later-empty-retry",
        ))
        .unwrap()
        .runtime;
    let active = store
        .update_task(&update(
            retried.revision,
            TaskStateCommandKind::ActivateItems,
            &["S1-A"],
            "pair-active",
        ))
        .unwrap()
        .runtime;
    rusqlite::Connection::open(database.path()).unwrap().execute("UPDATE task_events SET payload_json=json_set(payload_json,'$.command.item_ids',json('[]')) WHERE event_id IN ('later-empty-cancel','later-empty-retry')", []).unwrap();
    // The first pair cannot prove a successor after the later ambiguity.
    assert!(store
        .finish_scheduler_cancelled_result(&result, started, active.revision, "pair-strict", false)
        .is_err());
    store
        .finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            active.revision,
            "pair-visible",
            false,
        )
        .unwrap();
    assert_eq!(
        store
            .load_task(&result.task_id)
            .unwrap()
            .unwrap()
            .runtime
            .failed_item_ids,
        ["S1-A"]
    );
}

fn generation_persistence_rows(database: &TempDatabase) -> Vec<String> {
    let connection = rusqlite::Connection::open(database.path()).unwrap();
    let mut statement = connection.prepare(
        "SELECT json_array(task_id,status,revision,updated_at,state_json) FROM task_runtime
         UNION ALL SELECT json_array(task_id,batch_id,provider_id,created_at,output_root,layout,snapshot_json) FROM task_snapshots
         UNION ALL SELECT json_array(event_id,task_id,sequence,occurred_at,event_type,item_id,payload_json,error_json) FROM task_events
         ORDER BY 1",
    ).unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
}

#[test]
fn legacy_generation_incomplete_pairs_and_crossed_kinds_stay_pending_without_writes() {
    for mutation in [
        "DELETE FROM task_events WHERE event_id='legacy-retry'",
        "DELETE FROM task_events WHERE event_id='legacy-cancel'",
        "UPDATE task_events SET payload_json=json_set(payload_json,'$.command.item_ids',json('[\"S1-A\",\"S1-B\"]')) WHERE event_id='legacy-retry'",
        "UPDATE task_events SET payload_json=json_set(payload_json,'$.command.item_ids',json('[\"S1-A\",\"S1-B\"]')) WHERE event_id='legacy-cancel'",
        "UPDATE task_events SET event_type='cancel_items' WHERE event_id='legacy-cancel'",
        "UPDATE task_events SET event_type='retry_items' WHERE event_id='legacy-retry'",
        "UPDATE task_events SET event_type='cancel_items',payload_json=json_set(payload_json,'$.command.item_ids',json('[\"S1-A\",\"S1-B\"]')) WHERE event_id='legacy-cancel'",
        "UPDATE task_events SET event_type='retry_items',payload_json=json_set(payload_json,'$.command.item_ids',json('[\"S1-A\",\"S1-B\"]')) WHERE event_id='legacy-retry'",
    ] {
        let (database, mut store, started, active, result) = legacy_generation_fixture();
        rusqlite::Connection::open(database.path()).unwrap().execute(mutation, []).unwrap();
        let rows = generation_persistence_rows(&database);
        let before = store.load_task(&result.task_id).unwrap().unwrap().runtime;
        let download = store.load_download_snapshot(&result.task_id).unwrap();
        // Repeated pending attempts must not mutate runtime, events, immutable
        // snapshots or download rows, including after reopening the Store.
        for attempt in 0..2 {
            let error = store.finish_scheduler_cancelled_result_with_legacy_policy(
                &result, started, active, &format!("pending-{attempt}"), false,
            ).unwrap_err();
            assert!(matches!(error, insar_store::StoreError::InvalidSnapshot { .. }), "{mutation}: {error:?}");
            assert_eq!(generation_persistence_rows(&database), rows, "{mutation}");
            assert_eq!(store.load_task(&result.task_id).unwrap().unwrap().runtime, before, "{mutation}");
            assert_eq!(store.load_download_snapshot(&result.task_id).unwrap(), download, "{mutation}");
            drop(store);
            store = TaskStore::open(database.path()).unwrap();
        }
    }
}

#[test]
fn legacy_generation_damaged_identity_and_item_evidence_never_become_visible_failures() {
    for mutation in [
        "payload_json='{}'",
        "payload_json=json_set(payload_json,'$.task_id','foreign-task')",
        "payload_json=json_set(payload_json,'$.event_id','foreign-event')",
        "payload_json=json_set(payload_json,'$.expected_revision',99)",
        "payload_json=json_set(payload_json,'$.occurred_at','2024-01-15T02:00:00Z')",
        "payload_json=json_set(payload_json,'$.command.kind','retry_task')",
        "payload_json=json_set(payload_json,'$.command.item_ids',json('[\"foreign-item\"]'))",
        "payload_json=json_set(payload_json,'$.command.item_ids',json('[\"S1-A\",\"S1-A\"]'))",
        "event_type='cancel_items', payload_json=json_set(payload_json,'$.command.kind','cancel_items')",
    ] {
        let (database, mut store, started, active, result) = legacy_generation_fixture();
        let before = store.load_task(&result.task_id).unwrap().unwrap().runtime;
        let snapshot = store.load_download_snapshot(&result.task_id).unwrap();
        rusqlite::Connection::open(database.path()).unwrap().execute(&format!("UPDATE task_events SET {mutation} WHERE event_id='legacy-cancel'"), []).unwrap();
        assert!(store.finish_scheduler_cancelled_result_with_legacy_policy(&result, started, active, "damaged-finish", false).is_err(), "{mutation}");
        assert_eq!(store.load_task(&result.task_id).unwrap().unwrap().runtime, before, "{mutation}");
        assert_eq!(store.load_download_snapshot(&result.task_id).unwrap(), snapshot, "{mutation}");
    }
}

#[test]
fn legacy_generation_paused_and_queued_items_are_not_forced_failed() {
    for kind in [
        TaskStateCommandKind::PauseTask,
        TaskStateCommandKind::PauseItems,
    ] {
        let (_database, mut store, started, active, result) = legacy_generation_fixture();
        let item_ids = if kind == TaskStateCommandKind::PauseItems {
            vec!["S1-A"]
        } else {
            vec![]
        };
        let paused = store
            .update_task(&update(active, kind, &item_ids, "generation-pause"))
            .unwrap()
            .runtime;
        assert!(store
            .finish_scheduler_cancelled_result_with_legacy_policy(
                &result,
                started,
                paused.revision,
                "paused-visible",
                false
            )
            .is_err());
        assert_eq!(
            store.load_task(&result.task_id).unwrap().unwrap().runtime,
            paused
        );
    }
    let (_database, mut store, started, active, result) = legacy_generation_fixture();
    let paused = store
        .update_task(&update(
            active,
            TaskStateCommandKind::PauseItems,
            &["S1-A"],
            "generation-item-pause",
        ))
        .unwrap()
        .runtime;
    let queued = store
        .update_task(&update(
            paused.revision,
            TaskStateCommandKind::ResumeItems,
            &["S1-A"],
            "generation-item-resume",
        ))
        .unwrap()
        .runtime;
    assert!(store
        .finish_scheduler_cancelled_result_with_legacy_policy(
            &result,
            started,
            queued.revision,
            "queued-visible",
            false
        )
        .is_err());
    assert_eq!(
        store.load_task(&result.task_id).unwrap().unwrap().runtime,
        queued
    );
}

#[test]
fn legacy_generation_tombstone_wins_before_runtime_or_evidence_reads() {
    let (database, mut store, started, active, result) = legacy_generation_fixture();
    let cancelled = store
        .update_task(&update(
            active,
            TaskStateCommandKind::CancelTask,
            &[],
            "delete-cancel",
        ))
        .unwrap()
        .runtime;
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: result.task_id.clone(),
            expected_revision: cancelled.revision,
            archived_at: "2024-01-15T01:00:08Z".into(),
        })
        .unwrap();
    store
        .delete_history(&StoreDeleteHistoryRequest {
            task_id: result.task_id.clone(),
            deleted_at: "2024-01-15T01:00:09Z".into(),
        })
        .unwrap();
    assert!(matches!(
        store
            .finish_scheduler_cancelled_result_with_legacy_policy(
                &result,
                started,
                active,
                "deleted-visible",
                false
            )
            .unwrap(),
        insar_store::SchedulerCancelledFinish::Tombstoned
    ));
    assert!(TaskStore::open(database.path())
        .unwrap()
        .restore(&StoreRestoreRequest {
            include_terminal: true
        })
        .unwrap()
        .tasks
        .is_empty());
}
