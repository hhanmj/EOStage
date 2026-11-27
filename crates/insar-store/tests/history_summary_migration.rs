use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskStateCommandKind,
};
use insar_store::{StoreTaskHistoryPageRequest, TaskStore, STORE_SCHEMA_VERSION};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};

const BATCH_ID: &str = "history-summary-batch";

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-history-summary-{label}-{}-{nonce}.sqlite3",
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

fn candidate(index: usize, large_native: bool) -> SnapshotItem {
    let mut metadata = Map::new();
    if large_native {
        metadata.insert(
            "native_private_payload".to_owned(),
            Value::String("X".repeat(2_000_000)),
        );
    }
    SnapshotItem {
        item_id: format!("ITEM-{index:03}"),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("Item {index:03}"),
        acquisition_datetime: Some("2026-08-17T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn task_request(index: usize, output_root: &Path) -> StoreCreateTaskRequest {
    StoreCreateTaskRequest {
        task_id: format!("task-{index:03}"),
        batch_id: BATCH_ID.to_owned(),
        selected_item_ids: vec![format!("ITEM-{index:03}")],
        checked_item_ids: vec![format!("ITEM-{index:03}")],
        page_index: 0,
        page_size: 24,
        output_root: output_root.join(format!("task-{index:03}")),
        layout: DownloadLayout::Sentinel1Slc,
        concurrency: 1,
        created_at: format!("2026-08-16T00:00:{index:02}Z"),
    }
}

fn seed_tasks(database: &TempDatabase, count: usize) {
    let mut store = TaskStore::open(database.path()).expect("open Store");
    let candidates = (0..count)
        .map(|index| candidate(index, index == 0))
        .collect::<Vec<_>>();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: BATCH_ID.to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: "2026-08-16T00:00:00Z".to_owned(),
                aoi_name: "History AOI".to_owned(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
                    })
                    .as_object()
                    .expect("AOI")
                    .clone(),
                ),
                query: json!({"dataset": "SENTINEL-1"})
                    .as_object()
                    .expect("dataset query")
                    .clone(),
                search_stats: None,
                candidates,
            },
            workbench: SearchWorkbenchState {
                batch_id: BATCH_ID.to_owned(),
                checked_item_ids: vec!["ITEM-000".to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .expect("save batch");
    for index in 0..count {
        store
            .create_task(&task_request(
                index,
                database.path().parent().expect("parent"),
            ))
            .expect("create task");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: format!("task-{index:03}"),
                expected_revision: 0,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkCompleted,
                    item_ids: vec![format!("ITEM-{index:03}")],
                },
                event_id: format!("complete-{index:03}"),
                occurred_at: "2026-08-17T00:00:00Z".to_owned(),
            })
            .expect("complete task");
    }
}

fn archive(store: &mut TaskStore, index: usize, archived_at: &str) {
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: format!("task-{index:03}"),
            expected_revision: 1,
            archived_at: archived_at.to_owned(),
        })
        .expect("archive task");
}

fn downgrade_to_v8_without_summary(database: &TempDatabase) -> Connection {
    let connection = Connection::open(database.path()).expect("raw connection");
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS task_history_dem_source_authority;
             DROP TABLE IF EXISTS task_history_summary;
             DROP TABLE IF EXISTS dem_source_identity;
             DROP TABLE IF EXISTS standalone_aoi_state;
             DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE IF EXISTS nisar_archived_recovery_v2;
             DROP TABLE IF EXISTS agent_approvals; DROP TABLE IF EXISTS agent_plans; DROP TABLE IF EXISTS agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 8;",
        )
        .expect("downgrade summary fixture");
    connection
}

#[test]
fn version_eight_backfill_materializes_only_valid_rows_and_orders_normalized_time() {
    let database = TempDatabase::new("v8-backfill");
    seed_tasks(&database, 4);
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        archive(&mut store, 0, "2026-08-17T00:00:00Z");
        archive(&mut store, 1, "2026-08-17T00:00:00.1Z");
        archive(&mut store, 2, "2026-08-17T00:00:00.020Z");
        archive(&mut store, 3, "2026-08-17T00:00:00.100Z");
    }

    let connection = downgrade_to_v8_without_summary(&database);
    let snapshot: String = connection
        .query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = 'task-000'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let runtime: String = connection
        .query_row(
            "SELECT state_json FROM task_runtime WHERE task_id = 'task-000'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut snapshot: Value = serde_json::from_str(&snapshot).unwrap();
    snapshot["task_id"] = json!("invalid-unknown-field");
    snapshot["unknown_top_level"] = json!(true);
    let mut runtime: Value = serde_json::from_str(&runtime).unwrap();
    runtime["task_id"] = json!("invalid-unknown-field");
    connection.execute(
        "INSERT INTO task_snapshots(task_id,batch_id,provider_id,created_at,output_root,layout,snapshot_json)
         VALUES (?1,?2,'asf.sentinel1','2026-08-16T00:00:59Z','invalid-output','sentinel1_slc',?3)",
        params!["invalid-unknown-field", BATCH_ID, snapshot.to_string()],
    ).unwrap();
    connection
        .execute(
            "INSERT INTO task_runtime(task_id,status,revision,updated_at,state_json)
         VALUES (?1,'completed',1,'2026-08-17T00:00:00Z',?2)",
            params!["invalid-unknown-field", runtime.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_history(task_id,archived_at,layout,record_json)
         VALUES (?1,'2026-08-17T00:01:00Z','sentinel1_slc','{}')",
            ["invalid-unknown-field"],
        )
        .unwrap();
    drop(connection);

    let store = TaskStore::open_existing(database.path()).expect("v8 migration");
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    let page = store
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 10,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .unwrap();
    let ids = page
        .items
        .iter()
        .map(|item| item.task.task_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["task-001", "task-003", "task-002", "task-000"]);
    assert!(page
        .items
        .iter()
        .all(|item| item.task.dataset_id.as_deref() == Some("SENTINEL-1")));
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.archived_at.as_str())
            .collect::<Vec<_>>(),
        [
            "2026-08-17T00:00:00.1Z",
            "2026-08-17T00:00:00.1Z",
            "2026-08-17T00:00:00.02Z",
            "2026-08-17T00:00:00Z",
        ],
        "archive timestamps are canonical display scalars while ordering uses epoch milliseconds",
    );
    assert!(page
        .items
        .iter()
        .all(|item| item.task.task_id != "invalid-unknown-field"));
    assert!(
        serde_json::to_vec(&page).unwrap().len() < 16_384,
        "2 MB private metadata never enters the scalar page"
    );

    let first_tie_page = store
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 1,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .unwrap();
    assert_eq!(first_tie_page.items[0].task.task_id, "task-001");
    let tie_cursor = first_tie_page.next_cursor.expect("LIMIT+1 tie cursor");
    let second_tie_page = store
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 1,
            after_archived_at_epoch_ms: Some(tie_cursor.archived_at_epoch_ms),
            after_task_id: Some(tie_cursor.task_id),
        })
        .unwrap();
    assert_eq!(second_tie_page.items[0].task.task_id, "task-003");
    drop(store);

    let reopened = TaskStore::open_existing(database.path()).expect("idempotent reopen");
    assert_eq!(
        reopened
            .load_task_history_page(&StoreTaskHistoryPageRequest {
                page_size: 10,
                after_archived_at_epoch_ms: None,
                after_task_id: None,
            })
            .unwrap()
            .items
            .len(),
        4,
    );
}

#[test]
fn archive_history_and_fixed_summary_commit_or_rollback_together() {
    let database = TempDatabase::new("archive-atomic");
    seed_tasks(&database, 1);
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_history_summary BEFORE INSERT ON task_history_summary
         BEGIN SELECT RAISE(ABORT, 'summary rejected'); END;",
        )
        .unwrap();
    drop(connection);

    let mut store = TaskStore::open(database.path()).unwrap();
    assert!(store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-000".to_owned(),
            expected_revision: 1,
            archived_at: "2026-08-17T00:00:00.1234Z".to_owned(),
        })
        .is_err());
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    let history_count: i64 = connection
        .query_row("SELECT count(*) FROM task_history", [], |row| row.get(0))
        .unwrap();
    let summary_count: i64 = connection
        .query_row("SELECT count(*) FROM task_history_summary", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!((history_count, summary_count), (0, 0));
}

#[test]
fn failed_version_nine_schema_creation_rolls_back_without_promotion() {
    let database = TempDatabase::new("v9-rollback");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = downgrade_to_v8_without_summary(&database);
    connection
        .execute_batch(
            "CREATE TABLE task_history_summary(marker TEXT NOT NULL) STRICT;
             INSERT INTO task_history_summary(marker) VALUES ('preserved');",
        )
        .unwrap();
    drop(connection);

    assert!(TaskStore::open_existing(database.path()).is_err());
    let connection = Connection::open(database.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let marker: String = connection
        .query_row("SELECT marker FROM task_history_summary", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 8);
    assert_eq!(marker, "preserved");
}

/// Orbit scheduled-fetch S4 (review finding 7): the archived history
/// projection carries the companion identity — `auxiliary_kind` derived from
/// the frozen `approved_worker_operation` (AUX_POEORB for orbit_download)
/// and `source_task_id` from the surviving task snapshot's operation
/// payload — while an ordinary archived task projects neither.
#[test]
fn archived_companion_history_projects_auxiliary_kind_and_source_task_id() {
    let database = TempDatabase::new("archived-companion-projection");
    let output_root = database.path().parent().expect("parent").to_path_buf();
    let mut store = TaskStore::open(database.path()).expect("open Store");

    let scene = "S1A_IW_SLC__1SDV_20240312T000000_20240312T000027_052931_066A01_0000";
    let orbit_item = SnapshotItem {
        item_id: scene.to_owned(),
        provider_id: "sentinel1.orbit".to_owned(),
        display_name: scene.to_owned(),
        acquisition_datetime: Some("2024-03-12T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    };
    let companion_batch_id = "companion-orbit-batch".to_owned();
    // The companion task: an orbit operation whose payload persists the
    // source image task, the way create_companion_orbit_task freezes it.
    store
        .save_search_batch_and_create_task_with_projection(
            &StoreSearchBatchRequest {
                activate_workbench: false,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: companion_batch_id.clone(),
                    provider_id: "sentinel1.orbit".to_owned(),
                    created_at: "2026-11-01T00:00:00Z".to_owned(),
                    aoi_name: "Companion AOI".to_owned(),
                    aoi_geojson: None,
                    query: json!({"source_task_id": "task-unified-source"})
                        .as_object()
                        .expect("query")
                        .clone(),
                    search_stats: None,
                    candidates: vec![orbit_item],
                },
                workbench: SearchWorkbenchState {
                    batch_id: companion_batch_id.clone(),
                    checked_item_ids: vec![scene.to_owned()],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            },
            &StoreCreateTaskRequest {
                task_id: "companion-orbit-task".to_owned(),
                batch_id: companion_batch_id,
                selected_item_ids: vec![scene.to_owned()],
                checked_item_ids: vec![scene.to_owned()],
                page_index: 0,
                page_size: 24,
                output_root: output_root.join("companion-output"),
                layout: DownloadLayout::SentinelOrbitAuxPoeorb,
                concurrency: 1,
                created_at: "2026-11-01T00:00:00Z".to_owned(),
            },
            Some(&insar_core::TaskOperationDescriptor {
                operation_id: "companion-orbit-task:poeorb".to_owned(),
                kind: "orbit".to_owned(),
                phase: "download".to_owned(),
                region_id: "companion-orbit".to_owned(),
                approved_worker_operation: "orbit_download".to_owned(),
                payload: json!({
                    "source_task_id": "task-unified-source",
                    "resource_id": "orbit.sentinel-1.poeorb.v1",
                    "capability_id": "capability.sentinel-1.poeorb.download.v1"
                })
                .as_object()
                .expect("operation payload")
                .clone(),
            }),
            None,
        )
        .expect("create companion task");

    // An ordinary archived task for the None side of the projection;
    // seed_tasks creates and completes it.
    seed_tasks(&database, 1);

    // Complete the companion so it can be archived.
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "companion-orbit-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkCompleted,
                item_ids: vec![scene.to_owned()],
            },
            event_id: "complete-companion-orbit-task".to_owned(),
            occurred_at: "2026-11-01T01:00:00Z".to_owned(),
        })
        .expect("complete companion");
    for (task_id, archived_at) in [
        ("companion-orbit-task", "2026-11-01T02:00:00Z"),
        ("task-000", "2026-11-01T03:00:00Z"),
    ] {
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision: 1,
                archived_at: archived_at.to_owned(),
            })
            .expect("archive task");
    }

    let page = store
        .load_task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 10,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .expect("read history page");
    assert_eq!(page.items.len(), 2);

    let companion = page
        .items
        .iter()
        .find(|item| item.task.task_id == "companion-orbit-task")
        .expect("the archived companion is in history");
    assert_eq!(
        companion.task.auxiliary_kind.as_deref(),
        Some("AUX_POEORB"),
        "auxiliary_kind derives from the frozen approved_worker_operation"
    );
    assert_eq!(
        companion.task.source_task_id.as_deref(),
        Some("task-unified-source"),
        "source_task_id projects from the surviving snapshot's operation payload"
    );

    let ordinary = page
        .items
        .iter()
        .find(|item| item.task.task_id == "task-000")
        .expect("the ordinary archived task is in history");
    assert_eq!(ordinary.task.auxiliary_kind, None);
    assert_eq!(ordinary.task.source_task_id, None);
}
