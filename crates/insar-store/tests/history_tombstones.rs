use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreAppendTaskRequest,
    StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreDeleteHistoryRequest,
    StoreRestoreRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskStateCommandKind,
};
use insar_store::TaskStore;
use serde_json::{json, Map};

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
            "insar-store-history-{}-{nonce}-{seq}.sqlite3",
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

fn seed(store: &mut TaskStore, output_root: &Path) {
    let candidates = ["S1-A", "S1-B"]
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
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-history".to_string(),
                provider_id: "asf.sentinel1".to_string(),
                created_at: "2024-01-15T00:00:00Z".to_string(),
                aoi_name: "云南省昭通市".to_string(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[103.0, 26.8], [104.0, 26.8], [104.0, 27.8], [103.0, 27.8], [103.0, 26.8]]]
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
                query: json!({"product_type": "SLC"})
                    .as_object()
                    .unwrap()
                    .clone(),
                search_stats: None,
                candidates,
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-history".to_string(),
                checked_item_ids: vec!["S1-A".to_string()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-history".to_string(),
            batch_id: "batch-history".to_string(),
            selected_item_ids: vec!["S1-A".to_string()],
            checked_item_ids: vec!["S1-A".to_string()],
            page_index: 0,
            page_size: 24,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 2,
            created_at: "2024-01-15T01:00:00Z".to_string(),
        })
        .unwrap();
}

#[test]
fn snapshot_append_keeps_original_directory_and_rejects_foreign_items() {
    let database = TempDatabase::new();
    let output_root = database.path().parent().unwrap().join("原下载目录");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, &output_root);

    let appended = store
        .append_task_items(&StoreAppendTaskRequest {
            task_id: "task-history".to_string(),
            expected_revision: 0,
            item_ids: vec!["S1-B".to_string()],
            event_id: "append-1".to_string(),
            occurred_at: "2024-01-15T01:00:01Z".to_string(),
        })
        .unwrap();
    assert_eq!(appended.snapshot.output_root, output_root);
    assert_eq!(appended.snapshot.layout, DownloadLayout::Sentinel1Slc);
    assert_eq!(appended.runtime.selected_item_ids, ["S1-A", "S1-B"]);
    assert_eq!(appended.runtime.queued_item_ids, ["S1-A", "S1-B"]);
    let core = store.core_restore(false).unwrap();
    assert!(
        core.tasks.iter().any(|task| task.task_id == "task-history"),
        "current task was degraded after append: {:?}",
        core.legacy_task_warnings
    );

    let error = store
        .append_task_items(&StoreAppendTaskRequest {
            task_id: "task-history".to_string(),
            expected_revision: 1,
            item_ids: vec!["FOREIGN".to_string()],
            event_id: "append-invalid".to_string(),
            occurred_at: "2024-01-15T01:00:02Z".to_string(),
        })
        .unwrap_err();
    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    assert_eq!(
        store
            .load_task("task-history")
            .unwrap()
            .unwrap()
            .runtime
            .revision,
        1
    );
}

#[test]
fn archived_failure_retry_reuses_task_and_event_identity_across_repeated_failures() {
    let database = TempDatabase::new();
    let output_root = database.path().parent().unwrap().join("retry-output");
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, &output_root);
    for attempt in 0..2 {
        let failed = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "task-history".to_string(),
                expected_revision: attempt * 2,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec!["S1-A".to_string()],
                },
                event_id: format!("failure-{attempt}"),
                occurred_at: format!("2024-01-15T02:00:0{}Z", attempt * 2),
            })
            .unwrap();
        assert_eq!(
            failed.runtime.status,
            insar_core::TaskLifecycleStatus::Failed
        );
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: "task-history".to_string(),
                expected_revision: failed.runtime.revision,
                archived_at: format!("2024-01-15T02:00:0{}Z", attempt * 2),
            })
            .unwrap();
        assert!(store.core_restore(false).unwrap().tasks.is_empty());
        let history = store.list_history().unwrap();
        assert_eq!(history.len(), 1, "a second failure must not add a record");
        assert_eq!(history[0].task.snapshot.task_id, "task-history");
        assert_eq!(history[0].task.runtime.revision, failed.runtime.revision);
        if attempt == 0 {
            let request = StoreUpdateTaskRequest {
                task_id: "task-history".to_string(),
                expected_revision: failed.runtime.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::RetryTask,
                    item_ids: vec![],
                },
                event_id: "retry-original-task".to_string(),
                occurred_at: "2024-01-15T02:00:01Z".to_string(),
            };
            let wrong_revision = StoreUpdateTaskRequest {
                expected_revision: 0,
                event_id: "retry-stale".to_string(),
                ..request.clone()
            };
            assert_eq!(
                store.update_task(&wrong_revision).unwrap_err().code(),
                "V3_STORE_REVISION_CONFLICT"
            );
            let wrong_command = StoreUpdateTaskRequest {
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::RetryItems,
                    item_ids: vec!["S1-A".to_string()],
                },
                event_id: "retry-item-archived".to_string(),
                ..request.clone()
            };
            assert_eq!(
                store.update_task(&wrong_command).unwrap_err().code(),
                "V3_STORE_INVALID_SNAPSHOT"
            );
            let retried = store.update_task(&request).unwrap();
            assert_eq!(retried.snapshot.task_id, "task-history");
            assert_eq!(retried.runtime.queued_item_ids, ["S1-A"]);
            assert_eq!(
                store.update_task(&request).unwrap().runtime.revision,
                retried.runtime.revision
            );
            assert!(store.list_history().unwrap().is_empty());
            assert_eq!(store.core_restore(false).unwrap().tasks.len(), 1);
        }
    }
    let events = store.list_task_events("task-history").unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "retry_task")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "mark_failed")
            .count(),
        2
    );
    drop(store);
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(reopened.list_history().unwrap().len(), 1);
    assert!(reopened.core_restore(false).unwrap().tasks.is_empty());
}

#[test]
fn retry_event_replay_after_rearchive_does_not_report_restored_task() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, database.path().parent().unwrap());
    let failed = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-history".into(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec!["S1-A".into()],
            },
            event_id: "archive-replay-first-failure".into(),
            occurred_at: "2024-01-15T02:00:00Z".into(),
        })
        .unwrap();
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-history".into(),
            expected_revision: failed.runtime.revision,
            archived_at: "2024-01-15T02:00:01Z".into(),
        })
        .unwrap();
    let retry = StoreUpdateTaskRequest {
        task_id: "task-history".into(),
        expected_revision: failed.runtime.revision,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::RetryTask,
            item_ids: vec![],
        },
        event_id: "archive-replay-original-retry".into(),
        occurred_at: "2024-01-15T02:00:02Z".into(),
    };
    let retried = store.update_task(&retry).unwrap();
    assert_eq!(store.update_task(&retry).unwrap().runtime, retried.runtime);
    let failed_again = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-history".into(),
            expected_revision: retried.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec!["S1-A".into()],
            },
            event_id: "archive-replay-second-failure".into(),
            occurred_at: "2024-01-15T02:00:03Z".into(),
        })
        .unwrap();
    assert_eq!(
        store.update_task(&retry).unwrap().runtime,
        failed_again.runtime
    );
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-history".into(),
            expected_revision: failed_again.runtime.revision,
            archived_at: "2024-01-15T02:00:04Z".into(),
        })
        .unwrap();
    assert!(
        store.update_task(&retry).is_err(),
        "an old retry event must not report an archived task as restored"
    );
    assert_eq!(store.list_history().unwrap().len(), 1);
    let mut collision = retry.clone();
    collision.occurred_at = "2024-01-15T02:00:09Z".into();
    assert_eq!(
        store.update_task(&collision).unwrap_err().code(),
        "V3_STORE_ALREADY_EXISTS"
    );
    store
        .delete_history(&StoreDeleteHistoryRequest {
            task_id: "task-history".into(),
            deleted_at: "2024-01-15T02:00:05Z".into(),
        })
        .unwrap();
    assert!(
        store.update_task(&retry).is_err(),
        "tombstoned task must not replay as restored"
    );
}

#[test]
fn completed_and_deleted_history_cannot_be_retried() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed(&mut store, database.path().parent().unwrap());
    let completed = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-history".to_string(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["S1-A".to_string()],
            },
            event_id: "finished-task".to_string(),
            occurred_at: "2024-01-15T02:00:00Z".to_string(),
        })
        .unwrap();
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-history".to_string(),
            expected_revision: completed.runtime.revision,
            archived_at: "2024-01-15T02:00:01Z".to_string(),
        })
        .unwrap();
    let retry = StoreUpdateTaskRequest {
        task_id: "task-history".to_string(),
        expected_revision: completed.runtime.revision,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::RetryTask,
            item_ids: vec![],
        },
        event_id: "retry-completed".to_string(),
        occurred_at: "2024-01-15T02:00:02Z".to_string(),
    };
    assert_eq!(
        store.update_task(&retry).unwrap_err().code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
    assert_eq!(store.list_history().unwrap().len(), 1);
    store
        .delete_history(&StoreDeleteHistoryRequest {
            task_id: "task-history".to_string(),
            deleted_at: "2024-01-15T02:00:03Z".to_string(),
        })
        .unwrap();
    assert_eq!(
        store.update_task(&retry).unwrap_err().code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
    assert!(store.list_history().unwrap().is_empty());
}

#[test]
fn archived_history_is_tombstoned_without_deleting_user_output_and_cannot_reappear() {
    let database = TempDatabase::new();
    let output_root = database.path().parent().unwrap().join("用户成果目录");
    fs::create_dir_all(&output_root).unwrap();
    let user_file = output_root.join("keep-user-product.zip");
    fs::write(&user_file, b"user product").unwrap();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        seed(&mut store, &output_root);
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "task-history".to_string(),
                expected_revision: 0,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkCompleted,
                    item_ids: vec!["S1-A".to_string()],
                },
                event_id: "complete-1".to_string(),
                occurred_at: "2024-01-15T02:00:00Z".to_string(),
            })
            .unwrap();
        let invalid_timestamp = store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: "task-history".to_string(),
                expected_revision: 1,
                archived_at: "2024-01-15T02:00:01.1234567890Z".to_string(),
            })
            .unwrap_err();
        assert_eq!(invalid_timestamp.code(), "V3_STORE_INVALID_SNAPSHOT");
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: "task-history".to_string(),
                expected_revision: 1,
                archived_at: "2024-01-15T02:00:01Z".to_string(),
            })
            .unwrap();
        assert!(store
            .restore(&StoreRestoreRequest {
                include_terminal: false,
            })
            .unwrap()
            .tasks
            .is_empty());
        assert_eq!(store.list_history().unwrap().len(), 1);
        let closed_error = store
            .append_task_items(&StoreAppendTaskRequest {
                task_id: "task-history".to_string(),
                expected_revision: 1,
                item_ids: vec!["S1-B".to_string()],
                event_id: "append-after-archive".to_string(),
                occurred_at: "2024-01-15T02:00:01Z".to_string(),
            })
            .unwrap_err();
        assert_eq!(closed_error.code(), "V3_STORE_INVALID_SNAPSHOT");

        store
            .delete_history(&StoreDeleteHistoryRequest {
                task_id: "task-history".to_string(),
                deleted_at: "2024-01-15T02:00:02Z".to_string(),
            })
            .unwrap();
        assert!(store.list_history().unwrap().is_empty());
        assert!(user_file.is_file());
    }

    let reopened = TaskStore::open(database.path()).unwrap();
    assert!(reopened
        .restore(&StoreRestoreRequest {
            include_terminal: true,
        })
        .unwrap()
        .tasks
        .is_empty());
    assert!(reopened.list_history().unwrap().is_empty());
    assert!(user_file.is_file());
}
