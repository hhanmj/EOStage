use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_store::{StoredAdministrativeBoundary, TaskStore};
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
            "insar-admin-boundary-cache-{label}-{}-{nonce}.sqlite3",
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
    .expect("geometry object")
    .clone()
}

fn entry(provider: &str, provider_object_id: &str) -> StoredAdministrativeBoundary {
    StoredAdministrativeBoundary {
        provider: provider.to_owned(),
        provider_object_id: provider_object_id.to_owned(),
        name: "巴东县".to_owned(),
        display_name: "湖北省 / 恩施土家族苗族自治州 / 巴东县".to_owned(),
        country_code: Some("CN".to_owned()),
        admin_level: Some("district".to_owned()),
        geometry: geometry(109.0),
        bbox: [109.0, 30.0, 110.0, 31.0],
        source: "DataV areas_v3".to_owned(),
        attribution: None,
        cached_at: "2026-08-22T00:00:00Z".to_owned(),
    }
}

#[test]
fn resolved_boundary_round_trips_across_store_reopen() {
    let database = TempDatabase::new("roundtrip");
    let expected = entry("datav", "422823");
    {
        let mut store = TaskStore::open(database.path()).expect("open store");
        store
            .upsert_administrative_boundary(&expected)
            .expect("persist validated boundary");
    }

    let reopened = TaskStore::open_existing(database.path()).expect("reopen store");
    let restored = reopened
        .load_administrative_boundary("datav", "422823")
        .expect("load boundary")
        .expect("cached boundary exists");
    assert_eq!(restored, expected);
}

#[test]
fn cache_rejects_invalid_identity_topology_and_non_derived_bbox() {
    let database = TempDatabase::new("write-validation");
    let mut store = TaskStore::open(database.path()).expect("open store");

    let mut invalid_datav = entry("datav", "relation:1");
    assert!(store
        .upsert_administrative_boundary(&invalid_datav)
        .is_err());

    invalid_datav = entry("nominatim", "relation:1");
    invalid_datav.geometry = json!({
        "type": "Polygon",
        "coordinates": [[[109.0, 30.0], [110.0, 31.0], [109.0, 31.0], [110.0, 30.0], [109.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone();
    assert!(store
        .upsert_administrative_boundary(&invalid_datav)
        .is_err());

    let mut wrong_bbox = entry("nominatim", "way:2");
    wrong_bbox.bbox[2] = 111.0;
    assert!(store.upsert_administrative_boundary(&wrong_bbox).is_err());
}

#[test]
fn cached_geometry_is_revalidated_when_loaded() {
    let database = TempDatabase::new("load-validation");
    {
        let mut store = TaskStore::open(database.path()).expect("open store");
        store
            .upsert_administrative_boundary(&entry("nominatim", "relation:7"))
            .expect("persist valid boundary");
    }

    let connection = Connection::open(database.path()).expect("open raw database");
    connection
        .execute(
            "UPDATE administrative_boundary_cache SET geometry_json = ?1 \
             WHERE provider = 'nominatim' AND provider_object_id = 'relation:7'",
            [json!({"type": "Point", "coordinates": [109.0, 30.0]}).to_string()],
        )
        .expect("tamper cache fixture");
    drop(connection);

    let reopened = TaskStore::open_existing(database.path()).expect("reopen store");
    assert!(reopened
        .load_administrative_boundary("nominatim", "relation:7")
        .is_err());
}
