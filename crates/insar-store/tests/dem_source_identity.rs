use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_store::{
    canonical_utc_identity_timestamp, StoreDemSourceIdentity, TaskStore, STORE_SCHEMA_VERSION,
};
use rusqlite::Connection;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-dem-source-identity-{label}-{}-{nonce}.sqlite3",
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

fn cop90() -> StoreDemSourceIdentity {
    StoreDemSourceIdentity {
        dataset: "COP90".to_owned(),
        provider: "OPENTOPOGRAPHY".to_owned(),
        updated_at: "2026-08-17T09:10:11Z".to_owned(),
    }
}

#[test]
fn selected_dem_identity_is_absent_by_default_and_survives_reopen_in_core_restore() {
    let database = TempDatabase::new("reopen");
    let mut store = TaskStore::open(database.path()).expect("open store");
    assert_eq!(store.core_restore(false).unwrap().dem_source_identity, None);

    let selected = cop90();
    store
        .save_dem_source_identity(&selected)
        .expect("save exact identity");
    assert_eq!(
        store.core_restore(false).unwrap().dem_source_identity,
        Some(selected.clone())
    );
    drop(store);

    let reopened = TaskStore::open_existing(database.path()).expect("reopen store");
    assert_eq!(
        reopened.health().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    assert_eq!(
        reopened.core_restore(false).unwrap().dem_source_identity,
        Some(selected)
    );
}

#[test]
fn invalid_identity_does_not_replace_the_last_confirmed_selection() {
    let database = TempDatabase::new("reject");
    let mut store = TaskStore::open(database.path()).unwrap();
    let selected = cop90();
    store.save_dem_source_identity(&selected).unwrap();

    for invalid in [
        StoreDemSourceIdentity {
            dataset: "cop90".to_owned(),
            ..selected.clone()
        },
        StoreDemSourceIdentity {
            provider: " OPENTOPOGRAPHY".to_owned(),
            ..selected.clone()
        },
        StoreDemSourceIdentity {
            updated_at: "not-a-timestamp".to_owned(),
            ..selected.clone()
        },
    ] {
        assert!(store.save_dem_source_identity(&invalid).is_err());
        assert_eq!(
            store.core_restore(false).unwrap().dem_source_identity,
            Some(selected.clone()),
            "failed set must not lie about persisted selection"
        );
    }
}

#[test]
fn identity_timestamp_has_one_canonical_utc_form_and_survives_reopen() {
    let database = TempDatabase::new("timestamp-canonical");
    let mut store = TaskStore::open(database.path()).unwrap();
    for (input, expected) in [
        ("2026-08-17T09:10:11.000Z", "2026-08-17T09:10:11Z"),
        ("2026-08-17T09:10:11.010Z", "2026-08-17T09:10:11.01Z"),
        ("2026-08-17T09:10:11.120Z", "2026-08-17T09:10:11.12Z"),
        ("2026-08-17T09:10:11.123Z", "2026-08-17T09:10:11.123Z"),
    ] {
        assert_eq!(
            canonical_utc_identity_timestamp(input).as_deref(),
            Some(expected)
        );
        store
            .save_dem_source_identity(&StoreDemSourceIdentity {
                dataset: "COP90".to_owned(),
                provider: "OPENTOPOGRAPHY".to_owned(),
                updated_at: input.to_owned(),
            })
            .expect("normalize and persist identity timestamp");
        assert_eq!(
            store
                .core_restore(false)
                .unwrap()
                .dem_source_identity
                .unwrap()
                .updated_at,
            expected
        );
    }
    for invalid in [
        "2026-08-17T09:10:11.1234Z",
        "2026-08-17T09:10:11+00:00",
        "2026-08-17T09:10:11Z\n",
    ] {
        assert_eq!(canonical_utc_identity_timestamp(invalid), None);
    }
    drop(store);
    let reopened = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        reopened
            .core_restore(false)
            .unwrap()
            .dem_source_identity
            .unwrap()
            .updated_at,
        "2026-08-17T09:10:11.123Z"
    );
}

#[test]
fn version_eleven_migration_is_idempotent_and_rolls_back_on_schema_collision() {
    let database = TempDatabase::new("migration");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE dem_source_identity;
             DROP TABLE standalone_aoi_state;
             DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2;
             DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 10;",
        )
        .unwrap();
    drop(connection);

    let reopened = TaskStore::open_existing(database.path()).expect("v10 to v11 migration");
    assert_eq!(
        reopened.health().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    assert_eq!(
        reopened.core_restore(false).unwrap().dem_source_identity,
        None
    );
    drop(reopened);
    TaskStore::open_existing(database.path()).expect("idempotent v11 reopen");

    let collision = TempDatabase::new("rollback");
    {
        let store = TaskStore::open(collision.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = Connection::open(collision.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE dem_source_identity;
             CREATE TABLE dem_source_identity(marker TEXT NOT NULL) STRICT;
             INSERT INTO dem_source_identity(marker) VALUES ('preserved');
             DROP TABLE agent_approvals; DROP TABLE agent_plans; DROP TABLE agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 10;",
        )
        .unwrap();
    drop(connection);

    assert!(TaskStore::open_existing(collision.path()).is_err());
    let connection = Connection::open(collision.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let marker: String = connection
        .query_row("SELECT marker FROM dem_source_identity", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 10);
    assert_eq!(marker, "preserved");
}
