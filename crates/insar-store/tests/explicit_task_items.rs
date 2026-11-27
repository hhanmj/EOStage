use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadItemProgress, DownloadItemStatus, DownloadLayout, FrozenSearchBatch,
    SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest,
    StoreTaskCommand, StoreTaskSourceBatchPageRequest, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskStateCommandKind,
};
use insar_store::{
    StoreAppendExplicitTaskItemsRequest, StoreCreateHistoricalExplicitTaskRequest, TaskStore,
};
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
            "insar-store-explicit-items-{}-{nonce}-{seq}.sqlite3",
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

fn product(item_id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: item_id.to_owned(),
        acquisition_datetime: Some("2026-08-20T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: map(json!({"dataset": "SLC-BURST"})),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn asset(item_id: &str, source_item_id: &str, role: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: format!("{source_item_id}-{role}"),
        acquisition_datetime: Some("2026-08-20T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: map(json!({
            "source_item_id": source_item_id,
            "stable_asset_key": item_id,
            "asset_role": role,
            "burst_asset_policy": "all"
        })),
        payload: map(json!({"burst_asset": true})),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed(store: &mut TaskStore) {
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "burst-source-batch".to_owned(),
                provider_id: "asf.catalog".to_owned(),
                created_at: "2026-08-26T00:00:00Z".to_owned(),
                aoi_name: "Burst AOI".to_owned(),
                aoi_geojson: None,
                query: map(json!({"dataset": "SLC-BURST"})),
                search_stats: None,
                candidates: vec![product("BURST-A"), product("BURST-B")],
            },
            workbench: SearchWorkbenchState {
                batch_id: "burst-source-batch".to_owned(),
                checked_item_ids: vec!["BURST-A".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
}

fn task_request(output_root: &Path, item_ids: Vec<String>) -> StoreCreateTaskRequest {
    StoreCreateTaskRequest {
        task_id: "burst-asset-task".to_owned(),
        batch_id: "burst-source-batch".to_owned(),
        selected_item_ids: item_ids.clone(),
        checked_item_ids: item_ids,
        page_index: 0,
        page_size: 24,
        output_root: output_root.to_owned(),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-08-26T01:00:00Z".to_owned(),
    }
}

#[test]
fn explicit_task_items_keep_the_source_batch_and_restore_for_scheduler_execution() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store);
    let explicit_items = vec![
        asset("asset-a-primary", "BURST-A", "primary"),
        asset("asset-a-metadata", "BURST-A", "metadata"),
    ];
    let item_ids = explicit_items
        .iter()
        .map(|item| item.item_id.clone())
        .collect();

    let created = store
        .create_task_with_explicit_items(
            &task_request(database.path().parent().unwrap(), item_ids),
            0,
            &["BURST-A".to_owned()],
            &explicit_items,
        )
        .unwrap();

    assert_eq!(created.snapshot.batch_id, "burst-source-batch");
    assert_eq!(created.snapshot.candidates, explicit_items);
    assert_eq!(
        created.runtime.selected_item_ids,
        ["asset-a-primary", "asset-a-metadata"]
    );
    assert_eq!(
        store
            .load_workbench()
            .unwrap()
            .unwrap()
            .state
            .checked_item_ids,
        ["BURST-A"]
    );

    drop(store);
    let mut reopened = TaskStore::open(database.path()).unwrap();
    let full = reopened.load_task("burst-asset-task").unwrap().unwrap();
    let scheduler = reopened
        .load_scheduler_task("burst-asset-task")
        .unwrap()
        .unwrap();
    assert_eq!(full.snapshot.batch_id, "burst-source-batch");
    assert_eq!(full.snapshot.candidates, explicit_items);
    assert_eq!(scheduler.snapshot.candidates, explicit_items);

    for (revision, item_id, bytes_completed, total_bytes, bytes_per_second) in [
        (0, "asset-a-primary", 10, 100, 1.0),
        (1, "asset-a-metadata", 20, 200, 2.0),
    ] {
        reopened
            .save_download_progress(
                &DownloadItemProgress {
                    task_id: "burst-asset-task".to_owned(),
                    item_id: item_id.to_owned(),
                    status: DownloadItemStatus::Active,
                    bytes_completed,
                    total_bytes: Some(total_bytes),
                    bytes_per_second,
                    attempt: 1,
                    updated_at: "2026-08-26T01:01:00Z".to_owned(),
                },
                revision,
            )
            .unwrap();
    }
    reopened
        .update_task(&StoreUpdateTaskRequest {
            task_id: "burst-asset-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["asset-a-primary".to_owned()],
            },
            event_id: "burst-asset-task:activate-primary".to_owned(),
            occurred_at: "2026-08-26T01:01:00Z".to_owned(),
        })
        .unwrap();

    let source = reopened
        .load_task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "burst-asset-task".to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert_eq!(source.source_batch_id, "burst-source-batch");
    assert_eq!(source.total_count, 2);
    assert_eq!(source.items[0].item.item_id, "BURST-A");
    assert!(source.items[0].belongs_to_task);
    assert!(source.items[0].checked);
    assert_eq!(
        source.items[0].task_status,
        Some(DownloadItemStatus::Active)
    );
    assert_eq!(source.items[0].bytes_completed, 30);
    assert_eq!(source.items[0].total_bytes, Some(300));
    assert_eq!(source.items[0].bytes_per_second, 3.0);
    assert_eq!(source.items[1].item.item_id, "BURST-B");
    assert!(!source.items[1].belongs_to_task);
    assert!(source
        .items
        .iter()
        .all(|item| !item.item.item_id.starts_with("asset-")));
}

#[test]
fn explicit_task_assets_keep_independent_terminal_state_and_retry_in_place() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store);
    let explicit_items = vec![
        asset("asset-a-primary", "BURST-A", "primary"),
        asset("asset-a-metadata", "BURST-A", "metadata"),
    ];
    let item_ids = explicit_items
        .iter()
        .map(|item| item.item_id.clone())
        .collect::<Vec<_>>();
    store
        .create_task_with_explicit_items(
            &task_request(database.path().parent().unwrap(), item_ids.clone()),
            0,
            &["BURST-A".to_owned()],
            &explicit_items,
        )
        .unwrap();

    for (revision, kind, item_ids, event_id) in [
        (
            0,
            TaskStateCommandKind::ActivateItems,
            item_ids.clone(),
            "burst-terminal:activate",
        ),
        (
            1,
            TaskStateCommandKind::MarkCompleted,
            vec!["asset-a-primary".to_owned()],
            "burst-terminal:primary-completed",
        ),
        (
            2,
            TaskStateCommandKind::MarkFailed,
            vec!["asset-a-metadata".to_owned()],
            "burst-terminal:metadata-failed",
        ),
    ] {
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "burst-asset-task".to_owned(),
                expected_revision: revision,
                command: StoreTaskCommand { kind, item_ids },
                event_id: event_id.to_owned(),
                occurred_at: "2026-08-26T01:02:00Z".to_owned(),
            })
            .unwrap();
    }
    let failed = store.load_task("burst-asset-task").unwrap().unwrap();
    assert_eq!(failed.snapshot.batch_id, "burst-source-batch");
    assert_eq!(failed.snapshot.candidates, explicit_items);
    assert_eq!(failed.runtime.status, TaskLifecycleStatus::Failed);
    assert_eq!(failed.runtime.completed_item_ids, ["asset-a-primary"]);
    assert_eq!(failed.runtime.failed_item_ids, ["asset-a-metadata"]);

    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "burst-asset-task".to_owned(),
            expected_revision: 3,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryItems,
                item_ids: vec!["asset-a-metadata".to_owned()],
            },
            event_id: "burst-terminal:metadata-retry".to_owned(),
            occurred_at: "2026-08-26T01:03:00Z".to_owned(),
        })
        .unwrap();
    let retrying = store.load_task("burst-asset-task").unwrap().unwrap();
    assert_eq!(retrying.snapshot.batch_id, "burst-source-batch");
    assert_eq!(retrying.runtime.completed_item_ids, ["asset-a-primary"]);
    assert!(retrying.runtime.failed_item_ids.is_empty());
    assert_eq!(retrying.runtime.queued_item_ids, ["asset-a-metadata"]);

    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "burst-asset-task".to_owned(),
            expected_revision: 4,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["asset-a-metadata".to_owned()],
            },
            event_id: "burst-terminal:metadata-completed".to_owned(),
            occurred_at: "2026-08-26T01:04:00Z".to_owned(),
        })
        .unwrap();
    let completed = store.load_task("burst-asset-task").unwrap().unwrap();
    assert_eq!(completed.snapshot.batch_id, "burst-source-batch");
    assert_eq!(completed.runtime.status, TaskLifecycleStatus::Completed);
    assert_eq!(
        completed.runtime.completed_item_ids,
        ["asset-a-metadata", "asset-a-primary"]
    );
    assert!(completed.runtime.failed_item_ids.is_empty());
}

#[test]
fn ordinary_task_creation_still_rejects_items_outside_the_source_batch() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store);

    let error = store
        .create_task(&task_request(
            database.path().parent().unwrap(),
            vec!["asset-a-primary".to_owned()],
        ))
        .unwrap_err();

    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    assert!(store.load_task("burst-asset-task").unwrap().is_none());
}

#[test]
fn typed_explicit_append_adds_frozen_children_and_source_membership_atomically() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store);
    let initial = vec![
        asset("asset-a-primary", "BURST-A", "primary"),
        asset("asset-a-metadata", "BURST-A", "metadata"),
    ];
    store
        .create_task_with_explicit_items(
            &task_request(
                database.path().parent().unwrap(),
                initial.iter().map(|item| item.item_id.clone()).collect(),
            ),
            0,
            &["BURST-A".to_owned()],
            &initial,
        )
        .unwrap();
    let appended = vec![
        asset("asset-b-primary", "BURST-B", "primary"),
        asset("asset-b-metadata", "BURST-B", "metadata"),
    ];

    let task = store
        .append_explicit_task_items(&StoreAppendExplicitTaskItemsRequest {
            task_id: "burst-asset-task".to_owned(),
            expected_revision: 0,
            event_id: "burst-asset-task:append-b".to_owned(),
            occurred_at: "2026-08-26T01:02:00Z".to_owned(),
            source_item_ids: vec!["BURST-B".to_owned()],
            task_items: appended.clone(),
        })
        .unwrap();

    assert_eq!(task.runtime.revision, 1);
    assert_eq!(task.snapshot.candidates.len(), 4);
    assert_eq!(
        &task.snapshot.candidates[2..],
        appended.as_slice(),
        "the supplied immutable child ordering is retained"
    );
    let source = store
        .load_task_source_batch_page(&StoreTaskSourceBatchPageRequest {
            task_id: "burst-asset-task".to_owned(),
            page_index: 0,
            page_size: 24,
        })
        .unwrap();
    assert!(source.items[0].belongs_to_task);
    assert!(source.items[1].belongs_to_task);
    assert_eq!(
        store
            .load_scheduler_task("burst-asset-task")
            .unwrap()
            .unwrap()
            .snapshot
            .candidates
            .len(),
        4
    );
}

#[test]
fn historical_explicit_successor_accepts_only_terminal_lifecycle_states_without_mutating_source() {
    for (label, expected_status, commands, allowed) in [
        (
            "completed",
            TaskLifecycleStatus::Completed,
            vec![
                (TaskStateCommandKind::MarkCompleted, vec!["asset-a-primary"]),
                (
                    TaskStateCommandKind::MarkCompleted,
                    vec!["asset-a-metadata"],
                ),
            ],
            true,
        ),
        (
            "cancelled",
            TaskLifecycleStatus::Cancelled,
            vec![(TaskStateCommandKind::CancelTask, vec![])],
            true,
        ),
        (
            "failed",
            TaskLifecycleStatus::Failed,
            vec![
                (TaskStateCommandKind::MarkFailed, vec!["asset-a-primary"]),
                (
                    TaskStateCommandKind::MarkCompleted,
                    vec!["asset-a-metadata"],
                ),
            ],
            true,
        ),
        ("queued", TaskLifecycleStatus::Queued, vec![], false),
        (
            "running",
            TaskLifecycleStatus::Running,
            vec![(TaskStateCommandKind::ActivateItems, vec!["asset-a-primary"])],
            false,
        ),
        (
            "paused",
            TaskLifecycleStatus::Paused,
            vec![(TaskStateCommandKind::PauseTask, vec![])],
            false,
        ),
    ] {
        let database = TempDatabase::new();
        let mut store = TaskStore::open(database.path()).unwrap();
        seed(&mut store);
        let initial = vec![
            asset("asset-a-primary", "BURST-A", "primary"),
            asset("asset-a-metadata", "BURST-A", "metadata"),
        ];
        store
            .create_task_with_explicit_items(
                &task_request(
                    database.path().parent().unwrap(),
                    initial.iter().map(|item| item.item_id.clone()).collect(),
                ),
                0,
                &["BURST-A".to_owned()],
                &initial,
            )
            .unwrap();
        for (revision, (kind, item_ids)) in commands.into_iter().enumerate() {
            store
                .update_task(&StoreUpdateTaskRequest {
                    task_id: "burst-asset-task".to_owned(),
                    expected_revision: revision as u64,
                    command: StoreTaskCommand {
                        kind,
                        item_ids: item_ids.into_iter().map(str::to_owned).collect(),
                    },
                    event_id: format!("burst-asset-task:{label}:{revision}"),
                    occurred_at: "2026-08-26T01:03:00Z".to_owned(),
                })
                .unwrap();
        }
        let source_before = store.load_task("burst-asset-task").unwrap().unwrap();
        assert_eq!(source_before.runtime.status, expected_status, "{label}");
        let successor_items = vec![
            asset("asset-b-primary", "BURST-B", "primary"),
            asset("asset-b-metadata", "BURST-B", "metadata"),
        ];
        let mut successor_request = task_request(
            database.path().parent().unwrap(),
            successor_items
                .iter()
                .map(|item| item.item_id.clone())
                .collect(),
        );
        successor_request.task_id = "burst-successor".to_owned();
        let result =
            store.create_historical_explicit_task(&StoreCreateHistoricalExplicitTaskRequest {
                source_task_id: "burst-asset-task".to_owned(),
                expected_source_task_revision: source_before.runtime.revision,
                task: successor_request,
                source_item_ids: vec!["BURST-B".to_owned()],
                task_items: successor_items.clone(),
            });

        if allowed {
            let successor = result.unwrap();
            assert_eq!(successor.snapshot.batch_id, "burst-source-batch", "{label}");
            assert_eq!(successor.snapshot.candidates, successor_items, "{label}");
            assert_eq!(successor.runtime.revision, 0, "{label}");
        } else {
            assert_eq!(
                result.unwrap_err().code(),
                "V3_STORE_INVALID_SNAPSHOT",
                "{label}"
            );
            assert!(
                store.load_task("burst-successor").unwrap().is_none(),
                "{label}"
            );
        }
        let source_after = store.load_task("burst-asset-task").unwrap().unwrap();
        assert_eq!(source_after, source_before, "{label}: source task changed");
        assert_eq!(
            source_after.runtime.selected_item_ids,
            ["asset-a-primary", "asset-a-metadata"],
            "{label}: source membership changed"
        );
    }
}
