use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_store::{TaskStore, STORE_SCHEMA_VERSION};
use rusqlite::Connection;

struct TempDatabase {
    path: PathBuf,
}

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self {
            path: std::env::temp_dir().join(format!(
                "insar-store-{label}-{}-{nonce}.sqlite3",
                std::process::id()
            )),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let path = PathBuf::from(format!("{}{}", self.path.display(), suffix));
            let _ = fs::remove_file(path);
        }
    }
}

const V1_TASK_EVENTS_SQL: &str = "CREATE TABLE IF NOT EXISTS task_snapshots(task_id TEXT PRIMARY KEY NOT NULL) STRICT; CREATE TABLE IF NOT EXISTS task_events(event_id TEXT PRIMARY KEY NOT NULL, task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE, sequence INTEGER NOT NULL CHECK(sequence >= 0), occurred_at TEXT NOT NULL, event_type TEXT NOT NULL, item_id TEXT, payload_json TEXT NOT NULL CHECK(length(payload_json) > 1), error_json TEXT, UNIQUE(task_id, sequence)) STRICT;";

fn seed_legacy_marker(database: &TempDatabase, version: i64, value: &str) {
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(&format!(
            "CREATE TABLE marker(value TEXT NOT NULL) STRICT;\
             INSERT INTO marker(value) VALUES ('{value}'); {V1_TASK_EVENTS_SQL}"
        ))
        .unwrap();
    connection
        .pragma_update(None, "user_version", version)
        .unwrap();
}

#[test]
fn new_database_migrates_atomically_and_sets_durable_pragmas() {
    let database = TempDatabase::new("migration");
    let store = TaskStore::open(database.path()).unwrap();
    let health = store.health().unwrap();

    assert_eq!(health.schema_version, STORE_SCHEMA_VERSION);
    assert_eq!(health.journal_mode.to_ascii_lowercase(), "wal");
    assert!(health.foreign_keys);
    assert!(health.busy_timeout_millis >= 5_000);
    assert_eq!(health.synchronous, 2, "SQLite FULL synchronous is required");

    let expected = [
        "administrative_boundary_cache",
        "agent_approvals",
        "agent_plans",
        "agent_runs",
        "asf_search_cache",
        "batch_task_items",
        "companion_preferences",
        "dem_source_identity",
        "download_engine_state",
        "download_progress",
        "download_results",
        "gacos_submission_attempts",
        "landed_artifacts",
        "nisar_archived_recovery_v2",
        "nisar_budget_upgrade_refusal_v2",
        "nisar_budget_upgrade_v2",
        "nisar_subset_execution_anchors_v2",
        "orbit_recheck_schedule",
        "region_aoi_state",
        "sar_search_batch_cache",
        "sar_search_batch_count",
        "scheduler_pending_cancelled_finishes",
        "search_batch_candidates",
        "search_batch_headers",
        "search_batch_progress",
        "search_batch_staging",
        "search_batch_staging_candidates",
        "search_batches",
        "source_index_backfill",
        "source_mutation_receipts",
        "source_receipt_event_slots",
        "source_receipt_schema_marker",
        "source_views",
        "standalone_aoi_state",
        "task_events",
        "task_history",
        "task_history_dem_source_authority",
        "task_history_summary",
        "task_item_local_reuse",
        "task_item_rebinds",
        "task_local_scan",
        "task_membership_additions",
        "task_runtime",
        "task_snapshot_heads",
        "task_snapshots",
        "task_tombstones",
        "task_transfer_telemetry",
        "task_view_state",
        "workbench_state",
        "workspace_context",
    ];
    assert_eq!(store.table_names().unwrap(), expected);
}

#[test]
fn database_reopens_at_the_same_schema_version() {
    let database = TempDatabase::new("reopen");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }

    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.health().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    assert_eq!(reopened.table_names().unwrap().len(), 50);
}

#[test]
fn version_twenty_seven_upgrade_preserves_rows_and_keeps_unrelated_snapshots_frozen() {
    let database = TempDatabase::new("v27-source-size-trigger");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO search_batches(batch_id, provider_id, created_at, snapshot_json)
            VALUES ('legacy-v27-batch', 'fixture.provider', '2026-10-16T00:00:00Z', '{"legacy":"batch"}');
            INSERT INTO task_snapshots(
                task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json
            ) VALUES (
                'legacy-v27-task', 'legacy-v27-batch', 'fixture.provider',
                '2026-10-16T00:00:00Z', 'legacy-v27-output', 'direct',
                '{"legacy":"snapshot"}'
            );
            INSERT INTO task_runtime(task_id, status, revision, updated_at, state_json)
            VALUES (
                'legacy-v27-task', 'paused', 7, '2026-10-16T00:00:01Z',
                '{"legacy":"runtime"}'
            );
            DROP TRIGGER task_snapshots_are_frozen;
            CREATE TRIGGER task_snapshots_are_frozen
            BEFORE UPDATE ON task_snapshots
            BEGIN
                SELECT RAISE(ABORT, 'frozen task snapshot cannot be updated');
            END;
            DROP TABLE nisar_budget_upgrade_v2;
            DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
            PRAGMA user_version = 27;
            "#,
        )
        .unwrap();
    let before_snapshot: String = connection
        .query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id='legacy-v27-task'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let before_runtime: (String, i64, String) = connection
        .query_row(
            "SELECT status, revision, state_json FROM task_runtime WHERE task_id='legacy-v27-task'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    drop(connection);

    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT snapshot_json FROM task_snapshots WHERE task_id='legacy-v27-task'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        before_snapshot
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT status, revision, state_json FROM task_runtime WHERE task_id='legacy-v27-task'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
            )
            .unwrap(),
        before_runtime
    );
    assert!(connection
        .execute(
            "UPDATE task_snapshots SET snapshot_json='{\"legacy\":\"mutated\"}' WHERE task_id='legacy-v27-task'",
            [],
        )
        .is_err());
    assert_eq!(
        connection
            .query_row(
                "SELECT snapshot_json FROM task_snapshots WHERE task_id='legacy-v27-task'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        before_snapshot
    );
}

#[test]
fn version_twenty_three_to_twenty_five_adds_gacos_attempt_ledger_without_rewriting_task_state() {
    let database = TempDatabase::new("v23-to-v25-gacos");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "INSERT INTO search_batches(batch_id, provider_id, created_at, snapshot_json)
             VALUES ('legacy-batch', 'gacos.webform', '2026-09-06T00:00:00Z', '{\"legacy\":\"batch\"}');
             INSERT INTO task_snapshots(
                 task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json
             ) VALUES (
                 'legacy-task', 'legacy-batch', 'gacos.webform',
                 '2026-09-06T00:00:00Z', 'legacy-output', 'direct',
                 '{\"legacy\":\"snapshot\"}'
             );
             INSERT INTO task_runtime(task_id, status, revision, updated_at, state_json)
             VALUES (
                 'legacy-task', 'paused', 7, '2026-09-06T00:00:01Z',
                 '{\"legacy\":\"runtime\"}'
             );
             DROP TABLE gacos_submission_attempts;
             DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2;
             DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 23;",
        )
        .unwrap();
    drop(connection);

    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    assert!(store
        .table_names()
        .unwrap()
        .contains(&"gacos_submission_attempts".to_owned()));
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    let snapshot: (String, String, String) = connection
        .query_row(
            "SELECT batch_id, provider_id, snapshot_json
             FROM task_snapshots WHERE task_id = 'legacy-task'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        snapshot,
        (
            "legacy-batch".to_owned(),
            "gacos.webform".to_owned(),
            "{\"legacy\":\"snapshot\"}".to_owned(),
        )
    );
    let runtime: (String, i64, String, String) = connection
        .query_row(
            "SELECT status, revision, updated_at, state_json
             FROM task_runtime WHERE task_id = 'legacy-task'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        runtime,
        (
            "paused".to_owned(),
            7,
            "2026-09-06T00:00:01Z".to_owned(),
            "{\"legacy\":\"runtime\"}".to_owned(),
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM gacos_submission_attempts",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn version_twenty_four_migrates_gacos_attempts_without_losing_existing_rows() {
    let database = TempDatabase::new("v24-to-v25-gacos");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO search_batches(batch_id, provider_id, created_at, snapshot_json)
            VALUES ('legacy-batch', 'gacos.webform', '2026-09-06T00:00:00Z', '{"legacy":"batch"}');
            INSERT INTO task_snapshots(
                task_id, batch_id, provider_id, created_at, output_root, layout, snapshot_json
            ) VALUES (
                'legacy-task', 'legacy-batch', 'gacos.webform',
                '2026-09-06T00:00:00Z', 'legacy-output', 'direct',
                '{"legacy":"snapshot"}'
            );
            DROP TABLE gacos_submission_attempts;
            DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2;
             DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
            CREATE TABLE gacos_submission_attempts (
                attempt_id TEXT PRIMARY KEY NOT NULL
                    CHECK(length(attempt_id) BETWEEN 1 AND 256 AND trim(attempt_id) = attempt_id),
                task_id TEXT NOT NULL REFERENCES task_snapshots(task_id) ON DELETE CASCADE
                    CHECK(length(task_id) BETWEEN 1 AND 128 AND trim(task_id) = task_id),
                item_id TEXT NOT NULL
                    CHECK(length(item_id) BETWEEN 1 AND 2048 AND trim(item_id) = item_id),
                fingerprint_sha256 TEXT NOT NULL
                    CHECK(length(fingerprint_sha256) = 64
                      AND fingerprint_sha256 = lower(fingerprint_sha256)
                      AND fingerprint_sha256 NOT GLOB '*[^0-9a-f]*'),
                account_identity_hash_sha256 TEXT NOT NULL
                    CHECK(length(account_identity_hash_sha256) = 64
                      AND account_identity_hash_sha256 = lower(account_identity_hash_sha256)
                      AND account_identity_hash_sha256 NOT GLOB '*[^0-9a-f]*'),
                attempt_number INTEGER NOT NULL CHECK(attempt_number >= 0),
                state TEXT NOT NULL CHECK(state IN (
                    'reserved', 'submitting', 'manual_verification_required',
                    'submission_uncertain', 'rejected'
                )),
                reason_code TEXT CHECK(reason_code IS NULL OR (
                    length(reason_code) BETWEEN 1 AND 128
                    AND trim(reason_code) = reason_code
                    AND reason_code NOT GLOB '*[^A-Za-z0-9_.:-]*'
                )),
                http_status INTEGER CHECK(http_status IS NULL OR http_status BETWEEN 100 AND 599),
                automatic_retry_allowed INTEGER NOT NULL DEFAULT 0
                    CHECK(automatic_retry_allowed = 0),
                created_at TEXT NOT NULL CHECK(length(created_at) BETWEEN 20 AND 24),
                updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 24),
                CHECK(
                    (state IN ('reserved', 'submitting')
                        AND reason_code IS NULL AND http_status IS NULL)
                    OR (state = 'manual_verification_required'
                        AND reason_code IS NOT NULL AND http_status BETWEEN 200 AND 299)
                    OR (state = 'submission_uncertain'
                        AND reason_code IS NOT NULL
                        AND (http_status IS NULL OR http_status BETWEEN 300 AND 399 OR http_status = 408 OR http_status = 429
                             OR http_status BETWEEN 500 AND 599))
                    OR (state = 'rejected'
                        AND reason_code IS NOT NULL
                        AND (http_status IS NULL OR (
                            http_status BETWEEN 100 AND 499
                            AND http_status NOT IN (408, 429)
                        )))
                )
            ) STRICT;
            INSERT INTO gacos_submission_attempts(
                attempt_id, task_id, item_id, fingerprint_sha256,
                account_identity_hash_sha256, attempt_number, state,
                reason_code, http_status, automatic_retry_allowed,
                created_at, updated_at
            ) VALUES (
                'legacy-attempt', 'legacy-task', 'legacy-item',
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                0, 'manual_verification_required', 'ACCEPTED', 202, 0,
                '2026-09-06T00:00:00Z', '2026-09-06T00:00:01Z'
            );
            PRAGMA user_version = 24;
            "#,
        )
        .unwrap();
    drop(connection);

    let reopened = TaskStore::open_existing(database.path()).expect("v24 GACOS migration");
    assert_eq!(
        reopened.health().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    drop(reopened);

    let connection = Connection::open(database.path()).unwrap();
    let row: (String, String, Option<i64>, i64) = connection
        .query_row(
            "SELECT state, reason_code, http_status, automatic_retry_allowed
             FROM gacos_submission_attempts WHERE attempt_id = 'legacy-attempt'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "manual_verification_required".to_owned(),
            "ACCEPTED".to_owned(),
            Some(202),
            0,
        )
    );
    assert_eq!(
        connection
            .execute(
                "INSERT INTO gacos_submission_attempts(
                     attempt_id, task_id, item_id, fingerprint_sha256,
                     account_identity_hash_sha256, attempt_number, state,
                     reason_code, http_status, automatic_retry_allowed,
                     created_at, updated_at
                 ) VALUES (
                     'new-submitted', 'legacy-task', 'legacy-item-2',
                     'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                     'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                     0, 'submitted', 'EXPLICIT_SUCCESS_EVIDENCE', 200, 0,
                     '2026-09-06T00:00:00Z', '2026-09-06T00:00:01Z'
                 )",
                [],
            )
            .unwrap(),
        1
    );
    assert!(connection
        .execute(
            "INSERT INTO gacos_submission_attempts(
                 attempt_id, task_id, item_id, fingerprint_sha256,
                 account_identity_hash_sha256, attempt_number, state,
                 reason_code, http_status, automatic_retry_allowed,
                 created_at, updated_at
             ) VALUES (
                 'invalid-submitted-null-status', 'legacy-task', 'legacy-item-3',
                 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',
                 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
                 0, 'submitted', 'EXPLICIT_SUCCESS_EVIDENCE', NULL, 0,
                 '2026-09-06T00:00:00Z', '2026-09-06T00:00:01Z'
             )",
            [],
        )
        .is_err());
}

#[test]
fn version_seventeen_rejects_an_incompatible_preexisting_boundary_cache_table() {
    let database = TempDatabase::new("v17-boundary-cache-preflight");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE administrative_boundary_cache;
             CREATE TABLE administrative_boundary_cache (
                 provider TEXT NOT NULL,
                 provider_object_id TEXT NOT NULL,
                 geometry_json TEXT NOT NULL,
                 PRIMARY KEY(provider, provider_object_id)
             ) STRICT;
             PRAGMA user_version = 16;",
        )
        .unwrap();
    drop(connection);

    let error = TaskStore::open_existing(database.path()).unwrap_err();
    assert_eq!(error.code(), "V3_STORE_INVALID_SNAPSHOT");
    let connection = Connection::open(database.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 16, "failed migration must not promote the schema");
}

#[test]
fn newer_schema_is_rejected_without_mutating_the_database() {
    let database = TempDatabase::new("future");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .pragma_update(None, "user_version", STORE_SCHEMA_VERSION + 1)
        .unwrap();
    drop(connection);

    let error = TaskStore::open(database.path()).unwrap_err();
    assert_eq!(error.code(), "V3_STORE_SCHEMA_NEWER");

    let connection = Connection::open(database.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, STORE_SCHEMA_VERSION + 1);
}

#[test]
fn version_thirteen_adds_the_typed_sar_batch_pointer_without_touching_legacy_cache() {
    let database = TempDatabase::new("v13-to-v14");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE search_batches (
                batch_id TEXT PRIMARY KEY NOT NULL,
                provider_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                snapshot_json TEXT NOT NULL
            ) STRICT;
            CREATE TABLE asf_search_cache (
                cache_key TEXT PRIMARY KEY NOT NULL,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                entry_json TEXT NOT NULL
            ) STRICT;
            CREATE TABLE task_snapshots(task_id TEXT PRIMARY KEY NOT NULL) STRICT;
            CREATE TABLE task_runtime(task_id TEXT PRIMARY KEY NOT NULL) STRICT;
            CREATE TABLE task_history_summary(task_id TEXT PRIMARY KEY NOT NULL) STRICT;
            INSERT INTO asf_search_cache VALUES('legacy',1,1,'{}');",
        )
        .unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 13).unwrap();
    drop(connection);

    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    assert!(store
        .table_names()
        .unwrap()
        .contains(&"sar_search_batch_cache".to_owned()));
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    let legacy: String = connection
        .query_row("SELECT entry_json FROM asf_search_cache", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(legacy, "{}");
}

#[test]
fn invalid_version_thirteen_rolls_back_the_v14_migration() {
    let database = TempDatabase::new("v13-invalid");
    seed_legacy_marker(&database, 13, "preserved-v13");
    assert!(TaskStore::open(database.path()).is_err());
    let connection = Connection::open(database.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 13);
    let table_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='sar_search_batch_cache'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_count, 0);
}

#[test]
fn version_one_database_migrates_in_place_without_changing_existing_rows() {
    let database = TempDatabase::new("v1-to-v2");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE marker(value TEXT NOT NULL) STRICT;\
             INSERT INTO marker(value) VALUES ('preserved');",
        )
        .unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    drop(connection);

    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM marker", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "preserved");
}

#[test]
fn version_two_database_adds_asf_cache_without_changing_existing_rows() {
    let database = TempDatabase::new("v2-to-v3");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE marker(value TEXT NOT NULL) STRICT;\
             INSERT INTO marker(value) VALUES ('v2-preserved');",
        )
        .unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 2).unwrap();
    drop(connection);

    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    assert!(store
        .table_names()
        .unwrap()
        .contains(&"asf_search_cache".to_string()));
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM marker", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "v2-preserved");
}

#[test]
fn version_five_singleton_aoi_migrates_to_region_and_workspace_authority() {
    let database = TempDatabase::new("v5-aoi");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE region_aoi_state (
                slot INTEGER PRIMARY KEY NOT NULL CHECK(slot = 1),
                region_id TEXT NOT NULL,
                revision INTEGER NOT NULL,
                source_kind TEXT NOT NULL,
                geometry_json TEXT NOT NULL,
                bbox_json TEXT NOT NULL,
                crs TEXT NOT NULL,
                display_name TEXT NOT NULL,
                bound_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            ) STRICT;
            INSERT INTO region_aoi_state
                (slot, region_id, revision, source_kind, geometry_json, bbox_json,
                 crs, display_name, bound_at, updated_at)
            VALUES
                (1, 'legacy-region', 3, 'manual_bbox',
                 '{\"type\":\"Polygon\"}', '[100.0,30.0,101.0,31.0]',
                 'EPSG:4326', 'Legacy AOI',
                 '2026-08-01T00:00:00Z', '2026-08-01T00:00:01Z');",
        )
        .unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 5).unwrap();
    drop(connection);

    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    let context = store
        .load_workspace_context()
        .unwrap()
        .expect("migration must create workspace authority");
    assert_eq!(context.workspace_id, "workspace-default");
    assert_eq!(context.region_id.as_deref(), Some("legacy-region"));
    assert_eq!(context.regions.len(), 1);
    let aois = store.load_region_aois("workspace-default").unwrap();
    assert_eq!(aois.len(), 1);
    assert_eq!(aois[0].region_id, "legacy-region");
    assert_eq!(aois[0].revision, 3);
}

#[test]
fn existing_store_migrates_version_one_without_creating_a_new_database() {
    let database = TempDatabase::new("existing-v1");
    seed_legacy_marker(&database, 1, "existing-v1-preserved");

    let store = TaskStore::open_existing(database.path()).unwrap();

    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM marker", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "existing-v1-preserved");
}

#[test]
fn existing_store_migrates_version_two_without_creating_a_new_database() {
    let database = TempDatabase::new("existing-v2");
    seed_legacy_marker(&database, 2, "existing-v2-preserved");

    let store = TaskStore::open_existing(database.path()).unwrap();

    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    assert!(store
        .table_names()
        .unwrap()
        .contains(&"asf_search_cache".to_string()));
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    let value: String = connection
        .query_row("SELECT value FROM marker", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "existing-v2-preserved");
}

#[test]
fn version_six_results_migrate_with_nullable_last_known_total_bytes() {
    let database = TempDatabase::new("v6-download-results");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE download_results (
                task_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                outcome TEXT NOT NULL,
                finished_at TEXT NOT NULL,
                result_json TEXT NOT NULL,
                PRIMARY KEY(task_id, item_id)
            ) STRICT;
            CREATE TABLE download_progress (
                task_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                status TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                progress_json TEXT NOT NULL,
                PRIMARY KEY(task_id, item_id)
            ) STRICT;
            INSERT INTO download_progress(task_id, item_id, status, updated_at, progress_json)
            VALUES ('old-task', 'old-item', 'active', '2026-08-15T00:00:00Z',
                '{\"bytes_completed\":25,\"bytes_per_second\":0}');
            INSERT INTO download_results(task_id, item_id, outcome, finished_at, result_json)
            VALUES ('old-task', 'old-item', 'cancelled', '2026-08-15T00:00:00Z', '{\"bytes_written\":25}');",
        )
        .unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 6).unwrap();
    drop(connection);

    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    let total: Option<i64> = connection
        .query_row(
            "SELECT last_known_total_bytes FROM download_results WHERE task_id = 'old-task'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        total, None,
        "pre-migration terminal rows remain honestly unknown"
    );
}

#[test]
fn version_seven_download_rows_gain_queryable_scalars_without_losing_known_totals() {
    let database = TempDatabase::new("v7-download-telemetry");
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE download_progress (
                task_id TEXT NOT NULL, item_id TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL,
                progress_json TEXT NOT NULL, PRIMARY KEY(task_id, item_id)
            ) STRICT;
            CREATE TABLE download_results (
                task_id TEXT NOT NULL, item_id TEXT NOT NULL, outcome TEXT NOT NULL, finished_at TEXT NOT NULL,
                result_json TEXT NOT NULL, last_known_total_bytes INTEGER, PRIMARY KEY(task_id, item_id)
            ) STRICT;
            INSERT INTO download_progress(task_id, item_id, status, updated_at, progress_json)
            VALUES ('task-v7', 'item-v7', 'active', '2026-08-17T00:00:00Z',
                '{\"bytes_completed\":25,\"total_bytes\":200,\"bytes_per_second\":10.0}');
            INSERT INTO download_results(task_id, item_id, outcome, finished_at, result_json, last_known_total_bytes)
            VALUES ('task-v7-result', 'item-v7-result', 'cancelled', '2026-08-17T00:00:00Z',
                '{\"bytes_written\":25}', 200);",
        )
        .unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 7).unwrap();
    drop(connection);

    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    let progress: (i64, Option<i64>, f64, Option<i64>) = connection
        .query_row(
            "SELECT bytes_completed, total_bytes, bytes_per_second, last_known_total_bytes FROM download_progress",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(progress, (25, Some(200), 10.0, Some(200)));
    let result_bytes: i64 = connection
        .query_row("SELECT bytes_written FROM download_results", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(result_bytes, 25);
}

#[test]
fn version_seven_telemetry_preflight_accepts_i64_max_and_rejects_invalid_rows_without_promoting() {
    let valid = TempDatabase::new("v7-telemetry-i64-max");
    let connection = Connection::open(valid.path()).unwrap();
    connection.execute_batch(
        "CREATE TABLE download_progress (task_id TEXT NOT NULL, item_id TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL, progress_json TEXT NOT NULL, PRIMARY KEY(task_id, item_id)) STRICT;
         CREATE TABLE download_results (task_id TEXT NOT NULL, item_id TEXT NOT NULL, outcome TEXT NOT NULL, finished_at TEXT NOT NULL, result_json TEXT NOT NULL, last_known_total_bytes INTEGER, PRIMARY KEY(task_id, item_id)) STRICT;
         INSERT INTO download_progress VALUES ('task', 'item', 'active', '2026-08-17T00:00:00Z', '{\"bytes_completed\":9223372036854775807,\"total_bytes\":9223372036854775807,\"bytes_per_second\":0}');
         INSERT INTO download_results VALUES ('task', 'result', 'cancelled', '2026-08-17T00:00:00Z', '{\"bytes_written\":9223372036854775807}', NULL);"
    ).unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 7).unwrap();
    drop(connection);
    let store = TaskStore::open_existing(valid.path()).expect("i64 max telemetry migrates");
    assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    drop(store);

    for (label, progress, result) in [
        ("malformed", "{", "{\"bytes_written\":1}"),
        (
            "null-required",
            "{\"bytes_completed\":null,\"bytes_per_second\":0}",
            "{\"bytes_written\":1}",
        ),
        (
            "wrong-type",
            "{\"bytes_completed\":1,\"bytes_per_second\":\"fast\"}",
            "{\"bytes_written\":1}",
        ),
        (
            "above-i64",
            "{\"bytes_completed\":9223372036854775808,\"bytes_per_second\":0}",
            "{\"bytes_written\":1}",
        ),
        (
            "nonfinite-rate",
            "{\"bytes_completed\":1,\"bytes_per_second\":1e999}",
            "{\"bytes_written\":1}",
        ),
        (
            "bad-result",
            "{\"bytes_completed\":1,\"bytes_per_second\":0}",
            "{\"bytes_written\":null}",
        ),
    ] {
        let database = TempDatabase::new(label);
        let connection = Connection::open(database.path()).unwrap();
        connection.execute_batch(&format!(
            "CREATE TABLE download_progress (task_id TEXT NOT NULL, item_id TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL, progress_json TEXT NOT NULL, PRIMARY KEY(task_id, item_id)) STRICT;
             CREATE TABLE download_results (task_id TEXT NOT NULL, item_id TEXT NOT NULL, outcome TEXT NOT NULL, finished_at TEXT NOT NULL, result_json TEXT NOT NULL, last_known_total_bytes INTEGER, PRIMARY KEY(task_id, item_id)) STRICT;
             INSERT INTO download_progress VALUES ('task', 'item', 'active', '2026-08-17T00:00:00Z', '{progress}');
             INSERT INTO download_results VALUES ('task', 'result', 'cancelled', '2026-08-17T00:00:00Z', '{result}', NULL);"
        )).unwrap();
        connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
        connection.pragma_update(None, "user_version", 7).unwrap();
        drop(connection);
        assert!(
            TaskStore::open_existing(database.path()).is_err(),
            "{label} must fail closed"
        );
        let connection = Connection::open(database.path()).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7, "{label} rollback preserves user_version");
        let scalar_columns: i64 = connection.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('download_progress') WHERE name = 'bytes_completed'", [], |row| row.get(0)
        ).unwrap();
        assert_eq!(scalar_columns, 0, "{label} rollback preserves v7 schema");
    }
}

#[test]
fn partial_version_six_download_schema_fails_closed_without_promotion() {
    let database = TempDatabase::new("partial-v6-download-schema");
    let connection = Connection::open(database.path()).unwrap();
    connection.execute_batch(
        "CREATE TABLE download_progress (task_id TEXT NOT NULL, item_id TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL, progress_json TEXT NOT NULL, PRIMARY KEY(task_id, item_id)) STRICT;"
    ).unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 6).unwrap();
    drop(connection);
    let err = match TaskStore::open_existing(database.path()) {
        Ok(_) => panic!("partial v6 download schema must fail closed"),
        Err(err) => err,
    };
    assert!(
        format!("{err:?}").contains("schema v8 migration requires download telemetry tables"),
        "{err:?}"
    );
    let connection = Connection::open(database.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 6);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='download_results'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn version_seven_missing_mandatory_download_table_fails_closed() {
    let database = TempDatabase::new("v7-missing-download-table");
    let connection = Connection::open(database.path()).unwrap();
    connection.execute_batch(
        "CREATE TABLE download_progress (task_id TEXT NOT NULL, item_id TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL, progress_json TEXT NOT NULL, PRIMARY KEY(task_id, item_id)) STRICT;"
    ).unwrap();
    connection.execute_batch(V1_TASK_EVENTS_SQL).unwrap();
    connection.pragma_update(None, "user_version", 7).unwrap();
    drop(connection);
    let err = match TaskStore::open_existing(database.path()) {
        Ok(_) => panic!("v7 missing mandatory download table must fail closed"),
        Err(err) => err,
    };
    assert!(
        format!("{err:?}").contains("schema v8 migration requires download telemetry tables"),
        "{err:?}"
    );
    let connection = Connection::open(database.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 7);
}

#[test]
fn existing_store_does_not_promote_a_zero_byte_file_to_a_valid_store() {
    let database = TempDatabase::new("existing-empty-file");
    fs::write(database.path(), []).unwrap();

    let error = TaskStore::open_existing(database.path()).unwrap_err();

    assert_eq!(error.code(), "V3_STORE_EMPTY");
    assert_eq!(fs::metadata(database.path()).unwrap().len(), 0);
}

#[test]
fn schema_42_and_43_verify_triggers_and_protocol_and_fail_closed_on_incomplete_schema() {
    let database = TempDatabase::new("schema-42-43");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(STORE_SCHEMA_VERSION, 43);
        assert_eq!(store.health().unwrap().schema_version, 43);
    }

    let connection = Connection::open(database.path()).unwrap();
    let trigger_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND ((tbl_name = 'task_events' AND name IN ('source_receipt_event_reservation_insert', 'source_receipt_event_reservation_update')) OR (tbl_name = 'source_mutation_receipts' AND name IN ('source_receipt_no_replace', 'source_receipt_no_delete', 'source_receipt_no_update', 'source_receipt_reserve_slot')) OR (tbl_name = 'source_receipt_event_slots' AND name IN ('source_receipt_slot_no_replace', 'source_receipt_slot_no_delete', 'source_receipt_slot_no_update')))",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(trigger_count, 9);
    let protocol: i64 = connection
        .query_row(
            "SELECT protocol FROM source_receipt_schema_marker WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(protocol, 2);

    // Re-entrant upgrade from 41 over an already-migrated schema 43 database converges cleanly.
    connection.pragma_update(None, "user_version", 41).unwrap();
    drop(connection);
    let reopened = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(reopened.health().unwrap().schema_version, 43);
    drop(reopened);

    // Experimental schema 43 missing a reservation trigger fails closed on both open_existing and open_verification_read_only.
    let missing_trigger = TempDatabase::new("schema-43-missing-trigger");
    {
        let _store = TaskStore::open(missing_trigger.path()).unwrap();
    }
    let connection = Connection::open(missing_trigger.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER source_receipt_event_reservation_insert;
             PRAGMA wal_checkpoint(TRUNCATE);
             PRAGMA journal_mode = DELETE;",
        )
        .unwrap();
    drop(connection);
    assert!(TaskStore::open_verification_read_only(missing_trigger.path()).is_err());
    assert!(TaskStore::open_existing(missing_trigger.path()).is_err());

    // Experimental schema 43 with incompatible protocol fails closed on both open_existing and open_verification_read_only.
    let bad_protocol = TempDatabase::new("schema-43-bad-protocol");
    {
        let _store = TaskStore::open(bad_protocol.path()).unwrap();
    }
    let connection = Connection::open(bad_protocol.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE source_receipt_schema_marker;
             CREATE TABLE source_receipt_schema_marker (
                 singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                 protocol INTEGER NOT NULL
             ) STRICT;
             INSERT INTO source_receipt_schema_marker(singleton, protocol) VALUES (1, 1);
             PRAGMA wal_checkpoint(TRUNCATE);
             PRAGMA journal_mode = DELETE;",
        )
        .unwrap();
    drop(connection);
    assert!(TaskStore::open_verification_read_only(bad_protocol.path()).is_err());
    assert!(TaskStore::open_existing(bad_protocol.path()).is_err());

    // Experimental schema 43 with a same-name trigger whose body differs fails closed on both open paths,
    // and upgrading from 42 replaces the stale trigger definition cleanly.
    let altered_trigger = TempDatabase::new("schema-43-altered-trigger");
    {
        let _store = TaskStore::open(altered_trigger.path()).unwrap();
    }
    let connection = Connection::open(altered_trigger.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER source_receipt_event_reservation_insert;
             CREATE TRIGGER source_receipt_event_reservation_insert
             BEFORE INSERT ON task_events
             BEGIN SELECT 1; END;
             PRAGMA wal_checkpoint(TRUNCATE);
             PRAGMA journal_mode = DELETE;",
        )
        .unwrap();
    drop(connection);
    assert!(TaskStore::open_verification_read_only(altered_trigger.path()).is_err());
    assert!(TaskStore::open_existing(altered_trigger.path()).is_err());

    let connection = Connection::open(altered_trigger.path()).unwrap();
    connection.pragma_update(None, "user_version", 42).unwrap();
    drop(connection);
    let repaired = TaskStore::open_existing(altered_trigger.path()).unwrap();
    assert_eq!(repaired.health().unwrap().schema_version, 43);
    drop(repaired);

    // A schema 42 database missing task_events fails closed instead of silently recreating the authority table.
    let missing_events = TempDatabase::new("schema-42-missing-events");
    {
        let _store = TaskStore::open(missing_events.path()).unwrap();
    }
    let connection = Connection::open(missing_events.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE task_events;
             PRAGMA user_version = 42;",
        )
        .unwrap();
    drop(connection);
    let err = match TaskStore::open_existing(missing_events.path()) {
        Ok(_) => panic!("missing task_events must fail closed"),
        Err(err) => err,
    };
    assert!(
        format!("{err:?}").contains("schema v43 migration requires task events"),
        "{err:?}"
    );

    // A schema 42 database with a pre-existing incompatible `source_mutation_receipts` table
    // fails closed inside the migration transaction without promoting `user_version` to 43.
    let altered_v42_table = TempDatabase::new("schema-42-altered-receipts-table");
    {
        let _store = TaskStore::open(altered_v42_table.path()).unwrap();
    }
    let connection = Connection::open(altered_v42_table.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER source_receipt_no_replace;
             DROP TRIGGER source_receipt_no_delete;
             DROP TRIGGER source_receipt_no_update;
             DROP TRIGGER source_receipt_reserve_slot;
             DROP TABLE source_mutation_receipts;
             CREATE TABLE source_mutation_receipts (
                 operation TEXT NOT NULL,
                 operation_key TEXT NOT NULL,
                 PRIMARY KEY (operation, operation_key)
             ) STRICT;
             PRAGMA user_version = 42;",
        )
        .unwrap();
    drop(connection);
    let err = match TaskStore::open_existing(altered_v42_table.path()) {
        Ok(_) => panic!("incompatible v42 source_mutation_receipts table must fail closed"),
        Err(err) => err,
    };
    assert!(
        format!("{err:?}").contains("source receipt schema is incomplete"),
        "{err:?}"
    );
    let connection = Connection::open(altered_v42_table.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(
        version, 42,
        "failed schema 42 -> 43 verification must roll back without promoting user_version"
    );
}

#[test]
fn schema_v43_tolerates_sqlite_renamed_table_trigger_quotes_and_rejects_tampered_triggers() {
    let database = TempDatabase::new("schema-43-renamed-table-quotes");
    {
        let _store = TaskStore::open(database.path()).unwrap();
    }
    // Rename task_events and rename back, causing SQLite to rewrite triggers with quoted identifiers.
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE task_events RENAME TO task_events_offline;
             ALTER TABLE task_events_offline RENAME TO task_events;",
        )
        .unwrap();
    drop(connection);

    // Opening the store should succeed despite SQLite's quote rewrites.
    let reopened = TaskStore::open(database.path());
    assert!(
        reopened.is_ok(),
        "reopening store after table rename should succeed: {:?}",
        reopened.err()
    );

    // But if trigger logic is genuinely tampered with, it must still be rejected.
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER source_receipt_slot_no_delete;
             CREATE TRIGGER source_receipt_slot_no_delete BEFORE DELETE ON source_receipt_event_slots
             BEGIN SELECT 1; END;",
        )
        .unwrap();
    drop(connection);

    let err = match TaskStore::open(database.path()) {
        Ok(_) => panic!("tampered trigger logic must fail closed"),
        Err(err) => err,
    };
    assert!(
        format!("{err:?}").contains("source receipt schema is incomplete"),
        "{err:?}"
    );
}
