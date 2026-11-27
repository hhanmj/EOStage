use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_store::{StoreStandaloneAoiInput, TaskStore, STORE_SCHEMA_VERSION};
use rusqlite::Connection;
use serde_json::{json, Map, Value};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-standalone-aoi-{label}-{}-{nonce}.sqlite3",
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

fn geometry(west: f64) -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[
            [west, 30.0], [west + 1.0, 30.0], [west + 1.0, 31.0],
            [west, 31.0], [west, 30.0]
        ]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn input(west: f64, expected_revision: Option<u64>) -> StoreStandaloneAoiInput {
    StoreStandaloneAoiInput {
        expected_revision,
        source_kind: "administrative_boundary".to_owned(),
        geometry: geometry(west),
        bbox: vec![west, 30.0, west + 1.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "湖北省 / 武汉市".to_owned(),
        updated_at: "2026-08-18T08:09:10Z".to_owned(),
    }
}

fn dense_authoritative_geometry(position_count: usize) -> Map<String, Value> {
    let mut ring = (0..position_count)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / position_count as f64;
            json!([110.0 + 5.0 * angle.cos(), 30.0 + 5.0 * angle.sin()])
        })
        .collect::<Vec<_>>();
    ring.push(ring[0].clone());
    json!({"type": "Polygon", "coordinates": [ring]})
        .as_object()
        .unwrap()
        .clone()
}

fn downgrade_standalone_table_to_v12(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE standalone_aoi_state RENAME TO standalone_aoi_state_v13_fixture;
             CREATE TABLE standalone_aoi_state (
                 slot INTEGER PRIMARY KEY NOT NULL CHECK(slot = 1),
                 revision INTEGER NOT NULL CHECK(revision >= 0),
                 source_kind TEXT NOT NULL CHECK(source_kind IN (
                     'manual_bbox', 'manual_polygon', 'uploaded_geometry', 'administrative_boundary'
                 )),
                 geometry_json TEXT NOT NULL CHECK(length(geometry_json) BETWEEN 2 AND 262144),
                 west REAL NOT NULL CHECK(west >= -180.0 AND west < 180.0),
                 south REAL NOT NULL CHECK(south >= -90.0 AND south < 90.0),
                 east REAL NOT NULL CHECK(east > -180.0 AND east <= 180.0 AND east > west),
                 north REAL NOT NULL CHECK(north > -90.0 AND north <= 90.0 AND north > south),
                 crs TEXT NOT NULL CHECK(crs = 'EPSG:4326'),
                 display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 2048),
                 updated_at TEXT NOT NULL CHECK(length(updated_at) BETWEEN 20 AND 24)
             ) STRICT;
             INSERT INTO standalone_aoi_state SELECT * FROM standalone_aoi_state_v13_fixture;
             DROP TABLE standalone_aoi_state_v13_fixture;
             DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2;
             DROP TABLE IF EXISTS agent_approvals; DROP TABLE IF EXISTS agent_plans; DROP TABLE IF EXISTS agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 12;",
        )
        .unwrap();
}

#[test]
fn standalone_aoi_survives_replace_clear_and_reopen() {
    let database = TempDatabase::new("lifecycle");
    let mut store = TaskStore::open(database.path()).expect("open store");
    assert_eq!(store.load_standalone_aoi().unwrap(), None);

    let first = store.save_standalone_aoi(&input(113.0, None)).unwrap();
    assert_eq!(first.revision, 0);
    assert_eq!(first.bbox, vec![113.0, 30.0, 114.0, 31.0]);
    let second = store
        .save_standalone_aoi(&input(114.0, Some(first.revision)))
        .unwrap();
    assert_eq!(second.revision, 1);
    assert_eq!(second.bbox, vec![114.0, 30.0, 115.0, 31.0]);
    assert!(store
        .save_standalone_aoi(&input(115.0, Some(first.revision)))
        .is_err());
    assert_eq!(store.load_standalone_aoi().unwrap(), Some(second.clone()));
    drop(store);

    let mut reopened = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        reopened.load_standalone_aoi().unwrap(),
        Some(second.clone())
    );
    reopened
        .clear_standalone_aoi(Some(second.revision))
        .expect("clear exact revision");
    assert_eq!(reopened.load_standalone_aoi().unwrap(), None);
    drop(reopened);
    assert_eq!(
        TaskStore::open_existing(database.path())
            .unwrap()
            .load_standalone_aoi()
            .unwrap(),
        None
    );
}

#[test]
fn authoritative_aoi_storage_is_transport_bounded_not_vertex_bounded() {
    let database = TempDatabase::new("dense-authority");
    let geometry = dense_authoritative_geometry(12_000);
    let input = StoreStandaloneAoiInput {
        expected_revision: None,
        source_kind: "administrative_boundary".to_owned(),
        geometry,
        bbox: vec![105.0, 25.0, 115.0, 35.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "复杂行政区边界".to_owned(),
        updated_at: "2026-08-18T08:09:10Z".to_owned(),
    };
    let mut store = TaskStore::open(database.path()).unwrap();
    let saved = store
        .save_standalone_aoi(&input)
        .expect("dense authority must persist");
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    let stored_bytes: i64 = connection
        .query_row(
            "SELECT length(geometry_json) FROM standalone_aoi_state",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        stored_bytes > 262_144,
        "fixture must exceed the removed v12 limit"
    );
    drop(connection);

    let reopened = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(reopened.load_standalone_aoi().unwrap(), Some(saved));
}

#[test]
fn invalid_standalone_aoi_never_replaces_the_confirmed_record() {
    let database = TempDatabase::new("invalid");
    let mut store = TaskStore::open(database.path()).unwrap();
    let confirmed = store.save_standalone_aoi(&input(113.0, None)).unwrap();

    let mut invalid = Vec::new();
    invalid.push(StoreStandaloneAoiInput {
        bbox: vec![0.0, 0.0, 1.0, 1.0],
        expected_revision: Some(confirmed.revision),
        ..input(114.0, None)
    });
    invalid.push(StoreStandaloneAoiInput {
        crs: "EPSG:3857".to_owned(),
        expected_revision: Some(confirmed.revision),
        ..input(114.0, None)
    });
    invalid.push(StoreStandaloneAoiInput {
        source_kind: "provider_url".to_owned(),
        expected_revision: Some(confirmed.revision),
        ..input(114.0, None)
    });
    invalid.push(StoreStandaloneAoiInput {
        updated_at: "not-a-timestamp".to_owned(),
        expected_revision: Some(confirmed.revision),
        ..input(114.0, None)
    });
    invalid.push(StoreStandaloneAoiInput {
        display_name: "x".repeat(513),
        expected_revision: Some(confirmed.revision),
        ..input(114.0, None)
    });
    invalid.push(StoreStandaloneAoiInput {
        geometry: json!({
            "type": "Polygon",
            "coordinates": [[[0.0,0.0],[1.0,1.0],[0.0,1.0],[1.0,0.0],[0.0,0.0]]]
        })
        .as_object()
        .unwrap()
        .clone(),
        bbox: vec![0.0, 0.0, 1.0, 1.0],
        expected_revision: Some(confirmed.revision),
        ..input(114.0, None)
    });

    for candidate in invalid {
        assert!(store.save_standalone_aoi(&candidate).is_err());
        assert_eq!(
            store.load_standalone_aoi().unwrap(),
            Some(confirmed.clone())
        );
    }
}

#[test]
fn version_thirteen_migration_expands_only_the_authoritative_geometry_transport_bound() {
    let database = TempDatabase::new("migration");
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert_eq!(store.health().unwrap().schema_version, STORE_SCHEMA_VERSION);
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch("DROP TABLE IF EXISTS agent_approvals; DROP TABLE IF EXISTS agent_plans; DROP TABLE IF EXISTS agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers; DROP TABLE standalone_aoi_state; DROP TABLE IF EXISTS nisar_budget_upgrade_v2; DROP TABLE nisar_archived_recovery_v2; PRAGMA user_version = 11;")
        .unwrap();
    drop(connection);
    let reopened = TaskStore::open_existing(database.path()).expect("v11 through v13 migration");
    assert_eq!(
        reopened.health().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    assert_eq!(reopened.load_standalone_aoi().unwrap(), None);
    drop(reopened);
    TaskStore::open_existing(database.path()).expect("idempotent v13 reopen");

    let preserved = TempDatabase::new("v12-preserve");
    let saved = {
        let mut store = TaskStore::open(preserved.path()).unwrap();
        store.save_standalone_aoi(&input(113.0, None)).unwrap()
    };
    downgrade_standalone_table_to_v12(preserved.path());
    let reopened = TaskStore::open_existing(preserved.path()).expect("v12 record migrates");
    assert_eq!(
        reopened.health().unwrap().schema_version,
        STORE_SCHEMA_VERSION
    );
    assert_eq!(reopened.load_standalone_aoi().unwrap(), Some(saved));

    let collision = TempDatabase::new("rollback");
    {
        TaskStore::open(collision.path()).unwrap();
    }
    let connection = Connection::open(collision.path()).unwrap();
    connection
        .execute_batch(
            "DROP TABLE standalone_aoi_state;
             CREATE TABLE standalone_aoi_state(marker TEXT NOT NULL) STRICT;
             INSERT INTO standalone_aoi_state(marker) VALUES ('preserved');
             DROP TABLE IF EXISTS agent_approvals; DROP TABLE IF EXISTS agent_plans; DROP TABLE IF EXISTS agent_runs; DROP TABLE IF EXISTS task_snapshot_heads; DROP TABLE IF EXISTS task_item_rebinds; DROP TABLE IF EXISTS batch_task_items; DROP TABLE IF EXISTS source_views; DROP TABLE IF EXISTS search_batch_headers;
             PRAGMA user_version = 11;",
        )
        .unwrap();
    drop(connection);
    assert!(TaskStore::open_existing(collision.path()).is_err());
    let connection = Connection::open(collision.path()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    let marker: String = connection
        .query_row("SELECT marker FROM standalone_aoi_state", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(version, 11);
    assert_eq!(marker, "preserved");
}
