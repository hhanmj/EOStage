use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskPauseReason, TaskPauseReasonKind, TaskStateCommandKind,
};
use insar_store::{StoreTaskEventPageRequest, TaskStore};
use serde_json::Map;

const GIB: u64 = 1 << 30;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-task-pause-reason-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        )))
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn seed_task(store: &mut TaskStore, task_id: &str) {
    let batch_id = format!("{task_id}-batch");
    let item_ids = vec![format!("{task_id}-item-0"), format!("{task_id}-item-1")];
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.clone(),
                provider_id: "fixture.provider".to_owned(),
                created_at: "2026-10-26T00:00:00Z".to_owned(),
                aoi_name: "fixture AOI".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: item_ids
                    .iter()
                    .map(|item_id| SnapshotItem {
                        item_id: item_id.clone(),
                        provider_id: "fixture.provider".to_owned(),
                        display_name: item_id.clone(),
                        acquisition_datetime: None,
                        footprint_geojson: None,
                        metadata: Map::new(),
                        payload: Map::new(),
                        asset_access: None,
                        execution_policy: None,
                    })
                    .collect(),
            },
            workbench: SearchWorkbenchState {
                batch_id: batch_id.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 25,
                revision: 0,
            },
            activate_workbench: false,
        })
        .expect("save batch");
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id,
            selected_item_ids: item_ids.clone(),
            checked_item_ids: item_ids,
            page_index: 0,
            page_size: 25,
            output_root: PathBuf::from("fixture-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-26T00:00:01Z".to_owned(),
        })
        .expect("create task");
}

fn reason() -> TaskPauseReason {
    TaskPauseReason {
        kind: TaskPauseReasonKind::DiskLow,
        free_bytes: GIB,
        threshold_bytes: 2 * GIB,
        drive: Some("F:".to_owned()),
    }
}

fn command(
    store: &TaskStore,
    task_id: &str,
    kind: TaskStateCommandKind,
    event_id: &str,
) -> StoreUpdateTaskRequest {
    StoreUpdateTaskRequest {
        task_id: task_id.to_owned(),
        expected_revision: store.load_task(task_id).unwrap().unwrap().runtime.revision,
        command: StoreTaskCommand {
            kind,
            item_ids: vec![],
        },
        event_id: event_id.to_owned(),
        occurred_at: "2026-10-26T00:02:00Z".to_owned(),
    }
}

#[test]
fn engine_pause_is_the_user_transition_plus_a_persisted_reason() {
    let database = TempDatabase::new("transition");
    let mut store = TaskStore::open(&database.0).expect("open store");
    seed_task(&mut store, "disk-task");
    seed_task(&mut store, "user-task");
    let before = store
        .load_task("disk-task")
        .unwrap()
        .unwrap()
        .runtime
        .revision;

    let request = command(
        &store,
        "disk-task",
        TaskStateCommandKind::PauseTask,
        "engine-disk-low",
    );
    let paused = store
        .pause_task_with_reason(&request, &reason())
        .expect("engine pause");
    assert!(paused.runtime.task_paused);
    assert_eq!(paused.runtime.status, TaskLifecycleStatus::Paused);
    assert_eq!(paused.runtime.revision, before + 1);
    // Replaying the same event is idempotent, as for every task command.
    let replayed = store
        .pause_task_with_reason(&request, &reason())
        .expect("replay");
    assert_eq!(replayed.runtime.revision, before + 1);

    let user = command(
        &store,
        "user-task",
        TaskStateCommandKind::PauseTask,
        "user-pause",
    );
    store.update_task(&user).expect("user pause");

    let events = store.list_task_events("disk-task").unwrap();
    let last = events.last().unwrap();
    assert_eq!(last.event_type, "pause_task");
    assert_eq!(last.payload["pause_reason"]["kind"], "disk_low");
    assert_eq!(last.payload["pause_reason"]["free_bytes"], GIB);

    // Survives reopening the Store (restart).
    drop(store);
    let store = TaskStore::open(&database.0).expect("reopen store");
    let reasons = store
        .load_task_pause_reasons(&["disk-task".to_owned(), "user-task".to_owned()])
        .unwrap();
    assert_eq!(reasons.get("disk-task"), Some(&reason()));
    assert_eq!(reasons.get("user-task"), None, "a user pause has no reason");
}

#[test]
fn resume_or_a_later_user_pause_clears_the_reason() {
    let database = TempDatabase::new("clears");
    let mut store = TaskStore::open(&database.0).expect("open store");
    seed_task(&mut store, "task");
    let ids = ["task".to_owned()];

    let pause = command(&store, "task", TaskStateCommandKind::PauseTask, "engine-1");
    store.pause_task_with_reason(&pause, &reason()).unwrap();
    assert!(store
        .load_task_pause_reasons(&ids)
        .unwrap()
        .contains_key("task"));

    let resume = command(
        &store,
        "task",
        TaskStateCommandKind::ResumeTask,
        "user-resume",
    );
    store.update_task(&resume).unwrap();
    assert!(store.load_task_pause_reasons(&ids).unwrap().is_empty());

    let pause = command(&store, "task", TaskStateCommandKind::PauseTask, "engine-2");
    store.pause_task_with_reason(&pause, &reason()).unwrap();
    let user = command(
        &store,
        "task",
        TaskStateCommandKind::PauseTask,
        "user-pause",
    );
    store.update_task(&user).unwrap();
    assert!(store.load_task_pause_reasons(&ids).unwrap().is_empty());
}

#[test]
fn a_reason_is_refused_on_any_command_but_a_task_level_pause() {
    let database = TempDatabase::new("refused");
    let mut store = TaskStore::open(&database.0).expect("open store");
    seed_task(&mut store, "task");
    let resume = command(&store, "task", TaskStateCommandKind::ResumeTask, "bad");
    assert!(store.pause_task_with_reason(&resume, &reason()).is_err());
    let mut items = command(&store, "task", TaskStateCommandKind::PauseTask, "bad-items");
    items.command.item_ids = vec!["task-item-0".to_owned()];
    assert!(store.pause_task_with_reason(&items, &reason()).is_err());
    assert!(
        !store
            .load_task("task")
            .unwrap()
            .unwrap()
            .runtime
            .task_paused
    );
}

#[test]
fn the_event_page_names_the_disk_low_pause() {
    let database = TempDatabase::new("event-page");
    let mut store = TaskStore::open(&database.0).expect("open store");
    seed_task(&mut store, "task");
    let pause = command(&store, "task", TaskStateCommandKind::PauseTask, "engine-1");
    let paused = store.pause_task_with_reason(&pause, &reason()).unwrap();
    let page = store
        .load_task_event_page(&StoreTaskEventPageRequest {
            task_id: "task".to_owned(),
            expected_revision: paused.runtime.revision,
            before_sequence: None,
            page_size: 10,
        })
        .unwrap();
    let newest = &page.items[0];
    assert_eq!(newest.event_kind, "paused");
    assert_eq!(newest.level, "warning");
    assert_eq!(newest.code.as_deref(), Some("DISK_LOW"));
    assert_eq!(newest.message_zh, "磁盘空间不足，任务已自动暂停");
}

/// The schema 33 `agent_approvals` table (before W3e's permission modes).
const APPROVALS_V33_SQL: &str = "DROP TABLE agent_approvals;
CREATE TABLE agent_approvals (
    approval_id TEXT PRIMARY KEY NOT NULL CHECK(length(approval_id) BETWEEN 1 AND 128),
    plan_id TEXT NOT NULL REFERENCES agent_plans(plan_id) ON DELETE CASCADE,
    plan_hash_seen TEXT NOT NULL CHECK(length(plan_hash_seen) = 64),
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    decision TEXT NOT NULL CHECK(decision IN ('approved','rejected')),
    decided_at TEXT NOT NULL,
    decided_via TEXT NOT NULL CHECK(decided_via = 'desktop_card')
) STRICT;
CREATE INDEX agent_approvals_plan_idx ON agent_approvals(plan_id, decided_at);
PRAGMA user_version = 33;";

/// The pause reason lives in the `pause_task` event payload, which the W3e
/// schema 34 upgrade does not touch: a reason recorded before the upgrade is
/// read back after it, and an engine pause on the upgraded Store records one.
#[test]
fn pause_reasons_coexist_with_the_schema_34_upgrade() {
    let database = TempDatabase::new("schema-34");
    {
        let mut store = TaskStore::open(&database.0).expect("open store");
        assert_eq!(store.health().unwrap().schema_version, 43);
        seed_task(&mut store, "before");
        seed_task(&mut store, "after");
        let pause = command(
            &store,
            "before",
            TaskStateCommandKind::PauseTask,
            "engine-before",
        );
        store.pause_task_with_reason(&pause, &reason()).unwrap();
    }
    rusqlite::Connection::open(&database.0)
        .unwrap()
        .execute_batch(APPROVALS_V33_SQL)
        .expect("rewind to schema 33");

    let mut store = TaskStore::open_existing(&database.0).expect("upgrade to schema 37");
    assert_eq!(store.health().unwrap().schema_version, 43);
    let pause = command(
        &store,
        "after",
        TaskStateCommandKind::PauseTask,
        "engine-after",
    );
    store.pause_task_with_reason(&pause, &reason()).unwrap();
    let reasons = store
        .load_task_pause_reasons(&["before".to_owned(), "after".to_owned()])
        .unwrap();
    assert_eq!(reasons.get("before"), Some(&reason()));
    assert_eq!(reasons.get("after"), Some(&reason()));
}
