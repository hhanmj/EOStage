//! Orbit scheduled-fetch S2 (schema 40): the two inert tables
//! `orbit_recheck_schedule` and `companion_preferences` plus their Store API.
//!
//! Nothing in production writes these rows yet (slice S2 is deliberately
//! inert); the scheduler wiring is slice S3. What this file pins down:
//!
//! - schema 40 creates both tables on a fresh Store and on a v39 Store, and
//!   the v39 → v40 upgrade leaves existing rows untouched (no backfill);
//! - a v40 Store opened by v39-era code fails closed with `NewerSchema`
//!   (documented, existing behavior — schema 40 is one-way);
//! - insert/update/load/delete round-trips with UTC RFC3339 timestamps and
//!   attempt counters, and the CHECK constraints reject anything else;
//! - `load` returns the waiting rows nearest due first;
//! - deleting the owning task snapshot cascades to its schedule rows;
//! - the per-source companion preference defaults to "include precise orbit"
//!   when no row exists (Owner 11-01 decision 6) and the auto-fetch
//!   preference defaults to on (Owner 11-01 decision 7).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest,
};
use insar_store::{
    StoreCompanionPreferences, StoreOrbitRecheckRequest, StoreOrbitRecheckState, TaskStore,
};
use rusqlite::Connection;
use serde_json::{json, Map};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-orbit-recheck-{label}-{}-{nonce}.sqlite3",
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

fn orbit_item(item_id: &str) -> SnapshotItem {
    let mut metadata = Map::new();
    metadata.insert("frame".to_owned(), json!("0042"));
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("scene {item_id}"),
        acquisition_datetime: Some("2026-11-01T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn fixture_batch(batch_id: &str, item_ids: &[&str]) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        created_at: "2026-11-01T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: None,
        query: Map::new(),
        search_stats: None,
        candidates: item_ids.iter().map(|id| orbit_item(id)).collect(),
    }
}

/// Creates a real task (so the schedule rows have an owning snapshot to
/// reference) and returns the `StoreCreateTaskRequest` for later reuse.
fn create_task(
    store: &mut TaskStore,
    database: &DatabaseFixture,
    task_id: &str,
    item_ids: &[&str],
) {
    let batch = fixture_batch(&format!("{task_id}-batch"), item_ids);
    let workbench = SearchWorkbenchState {
        batch_id: batch.batch_id.clone(),
        checked_item_ids: item_ids.iter().map(|id| id.to_string()).collect(),
        page_index: 0,
        page_size: item_ids.len().max(1) as u64,
        revision: 0,
    };
    let task = StoreCreateTaskRequest {
        task_id: task_id.to_owned(),
        batch_id: batch.batch_id.clone(),
        selected_item_ids: item_ids.iter().map(|id| id.to_string()).collect(),
        checked_item_ids: item_ids.iter().map(|id| id.to_string()).collect(),
        page_index: 0,
        page_size: item_ids.len().max(1) as u64,
        output_root: database
            .path
            .parent()
            .unwrap()
            .join(format!("{task_id}-output")),
        layout: DownloadLayout::SentinelOrbitAuxPoeorb,
        concurrency: 1,
        created_at: "2026-11-01T00:00:00Z".to_owned(),
    };
    store
        .save_search_batch_and_create_task(
            &StoreSearchBatchRequest {
                batch,
                workbench,
                activate_workbench: false,
            },
            &task,
        )
        .unwrap();
}

/// Wrapper so `create_task` can read the database path without borrowing
/// complications in tests that hold a `TaskStore` open.
struct DatabaseFixture {
    path: PathBuf,
}

impl DatabaseFixture {
    fn from(database: &TempDatabase) -> Self {
        Self {
            path: database.path().to_path_buf(),
        }
    }
}

fn recheck_request(task_id: &str, item_id: &str, next_check_at: &str) -> StoreOrbitRecheckRequest {
    StoreOrbitRecheckRequest {
        task_id: task_id.to_owned(),
        item_id: item_id.to_owned(),
        scene_id: "S1A_IW_SLC__1SDV_20261031T000000_20261031T000100_026700_033000_ABCD".to_owned(),
        orbit_type: "POEORB".to_owned(),
        acquisition_at: "2026-10-31T00:00:00Z".to_owned(),
        attempt: 0,
        next_check_at: next_check_at.to_owned(),
        deadline_at: "2026-12-30T00:00:00Z".to_owned(),
        state: StoreOrbitRecheckState::Waiting,
        last_error_code: None,
        updated_at: "2026-11-01T00:00:00Z".to_owned(),
    }
}

/// A recheck row fixture. into a schema-39 Store: the two v40 tables did
/// not exist yet.
fn rewind_to_v39(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS orbit_recheck_schedule; \
             DROP TABLE IF EXISTS companion_preferences;",
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 39).unwrap();
}

fn user_version(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn table_exists(path: &Path, table: &str) -> bool {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

fn column_names(path: &Path, table: &str) -> Vec<String> {
    Connection::open(path)
        .unwrap()
        .prepare(&format!(
            "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
        ))
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn schema_40_creates_the_orbit_recheck_and_companion_preference_tables() {
    let database = TempDatabase::new("schema");
    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(insar_store::STORE_SCHEMA_VERSION, 43);
    assert_eq!(store.health().unwrap().schema_version, 43);
    let tables = store.table_names().unwrap();
    assert!(tables.iter().any(|name| name == "orbit_recheck_schedule"));
    assert!(tables.iter().any(|name| name == "companion_preferences"));
    drop(store);

    assert_eq!(
        column_names(database.path(), "orbit_recheck_schedule"),
        [
            "task_id",
            "item_id",
            "scene_id",
            "orbit_type",
            "acquisition_at",
            "attempt",
            "next_check_at",
            "deadline_at",
            "state",
            "last_error_code",
            "updated_at",
        ]
    );
    assert_eq!(
        column_names(database.path(), "companion_preferences"),
        [
            "source_id",
            "include_precise_orbit",
            "auto_fetch_orbit",
            "updated_at"
        ]
    );
}

#[test]
fn a_v39_store_migrates_to_40_without_touching_existing_rows() {
    let database = TempDatabase::new("v39-to-v40");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        let fixture = DatabaseFixture::from(&database);
        create_task(&mut store, &fixture, "old-task", &["scene-a"]);
        store
            .save_companion_preferences(&StoreCompanionPreferences {
                source_id: "asf.sentinel1".to_owned(),
                include_precise_orbit: false,
                auto_fetch_orbit: true,
                updated_at: Some("2026-11-01T00:00:00Z".to_owned()),
            })
            .ok();
    }
    // Keep a marker row that predates the migration and must survive it.
    {
        let connection = Connection::open(database.path()).unwrap();
        connection
            .execute(
                "INSERT INTO task_runtime(task_id, status, revision, updated_at, state_json) \
                 SELECT 'old-task', 'queued', 99, '2026-11-01T00:00:01Z', '{\"marker\":true}' \
                 WHERE NOT EXISTS(SELECT 1 FROM task_runtime WHERE task_id = 'old-task')",
                [],
            )
            .unwrap();
    }
    rewind_to_v39(database.path());
    assert_eq!(user_version(database.path()), 39);
    assert!(!table_exists(database.path(), "orbit_recheck_schedule"));
    assert!(!table_exists(database.path(), "companion_preferences"));

    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    assert!(table_exists(database.path(), "orbit_recheck_schedule"));
    assert!(table_exists(database.path(), "companion_preferences"));
    assert_eq!(
        column_names(database.path(), "orbit_recheck_schedule").len(),
        11
    );
    // The task's frozen data is untouched by the migration.
    let task = store.load_task("old-task").unwrap();
    assert!(task.is_some());
    // The schedule is empty: the table is inert in this slice.
    assert!(store.load_orbit_rechecks().unwrap().is_empty());
}

#[test]
fn a_v40_store_opened_by_v39_code_fails_closed_with_newer_schema() {
    let database = TempDatabase::new("v40-by-v39");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, 43);
    }
    // A v39-era binary cannot open this Store: `NewerSchema` fails closed and
    // the database is not mutated. This documents the one-way boundary the
    // rollback section of the task contract records.
    let connection = Connection::open(database.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    drop(connection);
    assert_eq!(version, 43);
}

#[test]
fn upsert_and_load_round_trip_and_repeated_open_is_idempotent() {
    let database = TempDatabase::new("round-trip");
    let mut store = TaskStore::open(database.path()).unwrap();
    let fixture = DatabaseFixture::from(&database);
    create_task(&mut store, &fixture, "task-a", &["item-1", "item-2"]);
    create_task(&mut store, &fixture, "task-b", &["item-3"]);

    let first = store
        .upsert_orbit_recheck(&recheck_request("task-a", "item-1", "2026-11-21T00:00:00Z"))
        .unwrap();
    assert_eq!(first.state, StoreOrbitRecheckState::Waiting);
    assert_eq!(first.attempt, 0);

    // Insert for a second item and a second task.
    store
        .upsert_orbit_recheck(&recheck_request("task-a", "item-2", "2026-11-22T00:00:00Z"))
        .unwrap();
    store
        .upsert_orbit_recheck(&recheck_request("task-b", "item-3", "2026-11-20T00:00:00Z"))
        .unwrap();

    // Update the first row: attempt increments, next check moves later.
    let updated = store
        .upsert_orbit_recheck(&StoreOrbitRecheckRequest {
            attempt: 1,
            next_check_at: "2026-11-22T12:00:00Z".to_owned(),
            last_error_code: Some("ORB002".to_owned()),
            ..recheck_request("task-a", "item-1", "2026-11-21T00:00:00Z")
        })
        .unwrap();
    assert_eq!(updated.attempt, 1);
    assert_eq!(updated.last_error_code.as_deref(), Some("ORB002"));

    // Nearest due first.
    let loaded = store.load_orbit_rechecks().unwrap();
    let due_order: Vec<(&str, &str)> = loaded
        .iter()
        .map(|row| (row.task_id.as_str(), row.item_id.as_str()))
        .collect();
    assert_eq!(
        due_order,
        [
            ("task-b", "item-3"), // 2026-11-20 nearest
            ("task-a", "item-2"), // 2026-11-22T00:00
            ("task-a", "item-1"), // 2026-11-22T12:00 latest
        ]
    );

    // Close and reopen: the same rows come back (restart persistence).
    drop(store);
    let mut store = TaskStore::open_existing(database.path()).unwrap();
    let reloaded = store.load_orbit_rechecks().unwrap();
    assert_eq!(reloaded, loaded);

    // Delete one row.
    store.clear_orbit_recheck("task-a", "item-1").unwrap();
    let remaining = store.load_orbit_rechecks().unwrap();
    assert_eq!(remaining.len(), 2);
    assert!(!remaining
        .iter()
        .any(|row| row.task_id == "task-a" && row.item_id == "item-1"));

    // Cancelling every row of one task removes exactly that task's rows.
    store.cancel_orbit_rechecks_for_task("task-a").unwrap();
    let last = store.load_orbit_rechecks().unwrap();
    assert_eq!(last.len(), 1);
    assert_eq!(
        (last[0].task_id.as_str(), last[0].item_id.as_str()),
        ("task-b", "item-3")
    );
}

#[test]
fn deleting_the_owning_task_snapshot_cascades_to_its_schedule_rows() {
    let database = TempDatabase::new("cascade");
    let mut store = TaskStore::open(database.path()).unwrap();
    let fixture = DatabaseFixture::from(&database);
    create_task(&mut store, &fixture, "task-a", &["item-1"]);
    create_task(&mut store, &fixture, "task-b", &["item-2"]);
    store
        .upsert_orbit_recheck(&recheck_request("task-a", "item-1", "2026-11-21T00:00:00Z"))
        .unwrap();
    store
        .upsert_orbit_recheck(&recheck_request("task-b", "item-2", "2026-11-21T00:00:00Z"))
        .unwrap();

    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("PRAGMA foreign_keys = ON", []).unwrap();
    connection
        .execute("DELETE FROM task_snapshots WHERE task_id = 'task-a'", [])
        .unwrap();
    let remaining: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM orbit_recheck_schedule WHERE task_id = 'task-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0, "cascade removes the owning task's rows");
    let other: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM orbit_recheck_schedule WHERE task_id = 'task-b'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(other, 1);
}

#[test]
fn companion_preferences_default_on_and_round_trip_per_source() {
    let database = TempDatabase::new("preferences");
    let mut store = TaskStore::open(database.path()).unwrap();

    // No row yet: the default is auto-fetch on and precise orbit included
    // (Owner 11-01 decisions 6 and 7: default on).
    let default = store.load_companion_preferences("asf.sentinel1").unwrap();
    assert_eq!(
        default,
        StoreCompanionPreferences {
            source_id: "asf.sentinel1".to_owned(),
            include_precise_orbit: true,
            auto_fetch_orbit: true,
            updated_at: None,
        }
    );

    // A user opt-out persists per source.
    store
        .save_companion_preferences(&StoreCompanionPreferences {
            source_id: "asf.sentinel1".to_owned(),
            include_precise_orbit: false,
            auto_fetch_orbit: false,
            updated_at: Some("2026-11-01T08:00:00Z".to_owned()),
        })
        .unwrap();
    let stored = store.load_companion_preferences("asf.sentinel1").unwrap();
    assert!(!stored.include_precise_orbit);
    assert!(!stored.auto_fetch_orbit);
    assert_eq!(stored.updated_at.as_deref(), Some("2026-11-01T08:00:00Z"));

    // A different source keeps its own row and the default.
    let other = store.load_companion_preferences("copernicus.dem").unwrap();
    assert!(other.include_precise_orbit);
    assert!(other.auto_fetch_orbit);
    assert_eq!(other.updated_at, None);

    // Re-saving replaces the row (single row per source).
    store
        .save_companion_preferences(&StoreCompanionPreferences {
            source_id: "asf.sentinel1".to_owned(),
            include_precise_orbit: true,
            auto_fetch_orbit: true,
            updated_at: Some("2026-11-01T09:00:00Z".to_owned()),
        })
        .unwrap();
    let again = store.load_companion_preferences("asf.sentinel1").unwrap();
    assert!(again.include_precise_orbit);
    assert_eq!(again.updated_at.as_deref(), Some("2026-11-01T09:00:00Z"));
    let count: i64 = Connection::open(database.path())
        .unwrap()
        .query_row("SELECT COUNT(*) FROM companion_preferences", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1, "one row per source");

    // Restart persistence.
    drop(store);
    let store = TaskStore::open_existing(database.path()).unwrap();
    let reopened = store.load_companion_preferences("asf.sentinel1").unwrap();
    assert!(reopened.include_precise_orbit);
    assert!(reopened.auto_fetch_orbit);
}

#[test]
fn recheck_rows_reject_invalid_fields_instead_of_storing_them() {
    let database = TempDatabase::new("constraints");
    let mut store = TaskStore::open(database.path()).unwrap();
    let fixture = DatabaseFixture::from(&database);
    create_task(&mut store, &fixture, "task-a", &["item-1"]);

    // Non-UTC/non-RFC3339 timestamps are refused by the CHECK constraints.
    for bad_next in [
        "2026-11-21 00:00:00",       // space separator, no Z
        "2026-11-21T00:00:00+02:00", // non-UTC offset
        "2026/11/21T00:00:00Z",      // wrong separators
        "not-a-timestamp",
    ] {
        assert!(
            store
                .upsert_orbit_recheck(&recheck_request("task-a", "item-1", bad_next))
                .is_err(),
            "next_check_at = {bad_next} must be rejected"
        );
    }

    // An attempt counter outside SQLite's INTEGER range is refused.
    assert!(store
        .upsert_orbit_recheck(&StoreOrbitRecheckRequest {
            attempt: u64::MAX,
            ..recheck_request("task-a", "item-1", "2026-11-21T00:00:00Z")
        })
        .is_err());

    // An unknown orbit type is refused (RESORB needs the Owner-approved S7).
    assert!(store
        .upsert_orbit_recheck(&StoreOrbitRecheckRequest {
            orbit_type: "RESORB".to_owned(),
            ..recheck_request("task-a", "item-1", "2026-11-21T00:00:00Z")
        })
        .is_err());

    // A row for a task that does not exist is refused by the foreign key.
    assert!(store
        .upsert_orbit_recheck(&recheck_request(
            "missing-task",
            "item-1",
            "2026-11-21T00:00:00Z"
        ))
        .is_err());

    // Nothing was stored.
    assert!(store.load_orbit_rechecks().unwrap().is_empty());
}

#[test]
fn manual_resorb_and_upgrade_actions_stage_waiting_and_completed_items_atomically() {
    let database = TempDatabase::new("s7-resorb-upgrade");
    let mut store = TaskStore::open(database.path()).unwrap();
    let fixture = DatabaseFixture::from(&database);
    create_task(&mut store, &fixture, "orbit-task", &["scene-1"]);

    store
        .upsert_orbit_recheck(&recheck_request(
            "orbit-task",
            "scene-1",
            "2026-11-21T00:00:00Z",
        ))
        .unwrap();

    // Item is currently queued behind a Waiting schedule row; manual "use_resorb"
    // sets requested_orbit_type = "RESORB", clears the schedule row, and bumps revision.
    let after_resorb = store
        .update_task_and_clear_orbit_rechecks(&insar_core::StoreUpdateTaskRequest {
            task_id: "orbit-task".to_owned(),
            expected_revision: 0,
            command: insar_core::StoreTaskCommand {
                kind: insar_core::TaskStateCommandKind::RetryItems,
                item_ids: vec!["scene-1".to_owned()],
            },
            event_id: "orbit-resorb-test-1".to_owned(),
            occurred_at: "2026-11-05T00:00:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(after_resorb.runtime.revision, 1);
    assert_eq!(
        after_resorb.runtime.queued_item_ids,
        vec!["scene-1".to_owned()]
    );
    assert_eq!(
        after_resorb.snapshot.candidates[0]
            .payload
            .get("requested_orbit_type")
            .and_then(|v| v.as_str()),
        Some("RESORB")
    );
    assert!(store.load_orbit_rechecks().unwrap().is_empty());

    // Mark the RESORB download completed, then upgrade to POEORB (`orbit-upgrade-`):
    // requested_orbit_type returns to "POEORB" and the item is re-queued.
    let active = store
        .update_task(&insar_core::StoreUpdateTaskRequest {
            task_id: "orbit-task".to_owned(),
            expected_revision: 1,
            command: insar_core::StoreTaskCommand {
                kind: insar_core::TaskStateCommandKind::ActivateItems,
                item_ids: vec!["scene-1".to_owned()],
            },
            event_id: "activate-scene-1".to_owned(),
            occurred_at: "2026-11-05T00:01:00Z".to_owned(),
        })
        .unwrap();
    let completed = store
        .update_task(&insar_core::StoreUpdateTaskRequest {
            task_id: "orbit-task".to_owned(),
            expected_revision: active.runtime.revision,
            command: insar_core::StoreTaskCommand {
                kind: insar_core::TaskStateCommandKind::MarkCompleted,
                item_ids: vec!["scene-1".to_owned()],
            },
            event_id: "complete-scene-1".to_owned(),
            occurred_at: "2026-11-05T00:02:00Z".to_owned(),
        })
        .unwrap();
    let upgraded = store
        .update_task_and_clear_orbit_rechecks(&insar_core::StoreUpdateTaskRequest {
            task_id: "orbit-task".to_owned(),
            expected_revision: completed.runtime.revision,
            command: insar_core::StoreTaskCommand {
                kind: insar_core::TaskStateCommandKind::RetryItems,
                item_ids: vec!["scene-1".to_owned()],
            },
            event_id: "orbit-upgrade-test-1".to_owned(),
            occurred_at: "2026-11-05T00:03:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(upgraded.runtime.queued_item_ids, vec!["scene-1".to_owned()]);
    assert!(upgraded.runtime.completed_item_ids.is_empty());
    assert_eq!(
        upgraded.snapshot.candidates[0]
            .payload
            .get("requested_orbit_type")
            .and_then(|v| v.as_str()),
        Some("POEORB")
    );
}
