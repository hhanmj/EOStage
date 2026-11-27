use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskStateCommandKind,
};
use insar_store::TaskStore;
use rusqlite::Connection;
use serde_json::{json, Map};

static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

/// One private directory per test: the Store, its WAL files and the task
/// output root all live inside it. pid+nanos alone repeats across threads.
struct TempDatabase {
    directory: PathBuf,
    database: PathBuf,
}

impl TempDatabase {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "insar-scheduler-control-runtime-{}-{nonce}-{}",
            std::process::id(),
            NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let database = directory.join("store.sqlite3");
        Self {
            directory,
            database,
        }
    }

    fn path(&self) -> &Path {
        &self.database
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn temp_databases_created_together_never_share_a_path() {
    // Parallel tests in this binary each corrupt their own database; a shared
    // path let one test read another's corrupted row (TASK-REG-012 reverify).
    const THREADS: usize = 16;
    const PER_THREAD: usize = 64;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(THREADS));
    let databases = (0..THREADS)
        .map(|_| {
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                (0..PER_THREAD)
                    .map(|_| TempDatabase::new())
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flat_map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    let paths = databases
        .iter()
        .map(|database| database.path().to_path_buf())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(paths.len(), THREADS * PER_THREAD);
}

fn seed_task(store: &mut TaskStore, output_root: &Path) {
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-control-runtime".to_string(),
                provider_id: "asf.sentinel1".to_string(),
                created_at: "2026-07-24T00:00:00Z".to_string(),
                aoi_name: "测试区".to_string(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]]
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates: vec![SnapshotItem {
                    item_id: "S1-CONTROL".to_string(),
                    provider_id: "asf.sentinel1".to_string(),
                    display_name: "S1-CONTROL.zip".to_string(),
                    acquisition_datetime: None,
                    footprint_geojson: None,
                    metadata: Map::new(),
                    payload: Map::new(),
                    asset_access: None,
                    execution_policy: None,
                }],
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-control-runtime".to_string(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "task-control-runtime".to_string(),
            batch_id: "batch-control-runtime".to_string(),
            selected_item_ids: vec!["S1-CONTROL".to_string()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: output_root.join("control-runtime-output"),
            layout: DownloadLayout::Sentinel1Slc,
            concurrency: 1,
            created_at: "2026-07-24T00:00:01Z".to_string(),
        })
        .unwrap();
}

fn cancel_request(revision: u64) -> StoreUpdateTaskRequest {
    StoreUpdateTaskRequest {
        task_id: "task-control-runtime".to_string(),
        expected_revision: revision,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::CancelItems,
            item_ids: vec!["S1-CONTROL".to_string()],
        },
        event_id: "cancel-control-runtime".to_string(),
        occurred_at: "2026-07-24T00:00:02Z".to_string(),
    }
}

#[test]
fn terminal_runtime_is_available_to_control_reconciliation_but_not_scheduling() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());

    let before = store
        .load_scheduler_control_runtime("task-control-runtime")
        .unwrap()
        .unwrap();
    assert_eq!(before.status, TaskLifecycleStatus::Queued);
    assert_eq!(
        store
            .load_scheduler_control_runtime("missing-task")
            .unwrap(),
        None
    );

    let terminal = store.update_task(&cancel_request(before.revision)).unwrap();
    assert_eq!(terminal.runtime.status, TaskLifecycleStatus::Cancelled);
    assert!(store.load_scheduler_tasks().unwrap().is_empty());
    assert_eq!(
        store
            .load_scheduler_control_runtime("task-control-runtime")
            .unwrap()
            .unwrap(),
        terminal.runtime
    );
}

#[test]
fn scheduler_runtime_transition_reuses_runtime_authority_and_revision() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(database.path()).unwrap();
    seed_task(&mut store, database.path().parent().unwrap());

    let activated = store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: "task-control-runtime".to_string(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids: vec!["S1-CONTROL".to_string()],
            },
            event_id: "scheduler-activate-control-runtime".to_string(),
            occurred_at: "2026-07-24T00:00:02Z".to_string(),
        })
        .unwrap();
    assert_eq!(activated.revision, 1);
    assert_eq!(activated.status, TaskLifecycleStatus::Running);
    assert_eq!(activated.active_item_ids, ["S1-CONTROL"]);

    let completed = store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: "task-control-runtime".to_string(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["S1-CONTROL".to_string()],
            },
            event_id: "scheduler-complete-control-runtime".to_string(),
            occurred_at: "2026-07-24T00:00:03Z".to_string(),
        })
        .unwrap();
    assert_eq!(completed.revision, 2);
    assert_eq!(completed.status, TaskLifecycleStatus::Completed);
    assert_eq!(completed.completed_item_ids, ["S1-CONTROL"]);
    assert!(completed.active_item_ids.is_empty());
    assert_eq!(
        store
            .load_scheduler_control_runtime("task-control-runtime")
            .unwrap()
            .unwrap(),
        completed
    );
    assert!(store.load_scheduler_tasks().unwrap().is_empty());
}

#[test]
fn control_runtime_does_not_hydrate_the_search_batch() {
    let database = TempDatabase::new();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        seed_task(&mut store, database.path().parent().unwrap());
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER search_batches_are_frozen;
             UPDATE search_batches SET snapshot_json = '{not-json'
             WHERE batch_id = 'batch-control-runtime';
             CREATE TRIGGER search_batches_are_frozen
             BEFORE UPDATE ON search_batches
             BEGIN
                 SELECT RAISE(ABORT, 'frozen search batch cannot be updated');
             END;",
        )
        .unwrap();
    drop(connection);

    let runtime = TaskStore::open(database.path())
        .unwrap()
        .load_scheduler_control_runtime("task-control-runtime")
        .unwrap()
        .unwrap();
    assert_eq!(runtime.status, TaskLifecycleStatus::Queued);
}

#[test]
fn corrupt_control_runtime_fails_closed() {
    let database = TempDatabase::new();
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        seed_task(&mut store, database.path().parent().unwrap());
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute(
            "UPDATE task_runtime SET state_json = '{not-json' WHERE task_id = ?1",
            ["task-control-runtime"],
        )
        .unwrap();
    drop(connection);

    let error = TaskStore::open(database.path())
        .unwrap()
        .load_scheduler_control_runtime("task-control-runtime")
        .unwrap_err();
    assert_eq!(error.code(), "V3_STORE_CORRUPT_ROW");
}
