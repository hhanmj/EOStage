use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{AsfCacheEntry, AsfSceneMetadata, AsfSearchQuery};
use insar_store::TaskStore;
use rusqlite::Connection;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-asf-cache-{label}-{}-{nonce}.sqlite3",
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

fn query(limit: u64) -> AsfSearchQuery {
    AsfSearchQuery {
        version: 1,
        dataset: "SENTINEL-1".to_string(),
        bbox: None,
        aoi: String::new(),
        aoi_geojson: None,
        start: "2024-01-01".to_string(),
        end: "2024-12-31".to_string(),
        product_type: "SLC".to_string(),
        beam_mode: "IW".to_string(),
        polarization: String::new(),
        orbit_direction: String::new(),
        relative_orbit: None,
        frame: None,
        max_results: Some(limit),
    }
}

fn scene(id: &str, acquired: &str) -> AsfSceneMetadata {
    AsfSceneMetadata {
        scene_id: id.to_string(),
        url: None,
        platform: "S1A".to_string(),
        product_type: "SLC".to_string(),
        beam_mode: "IW".to_string(),
        polarization: "DV".to_string(),
        acquisition_datetime: acquired.to_string(),
        absolute_orbit: Some(50_000),
        path: Some(42),
        frame: Some(456),
        orbit_direction: Some("ASCENDING".to_string()),
        file_size_remote: None,
        footprint_bbox: None,
        footprint_geojson: None,
        provider_product_id: None,
        file_name: None,
        processing_level: None,
        acquisition_stop: None,
        browse_url: None,
        thumbnail_url: None,
        checksum_algorithm: None,
        checksum_value: None,
        center_latitude: None,
        center_longitude: None,
    }
}

fn entry(scenes: Vec<AsfSceneMetadata>, complete: bool) -> AsfCacheEntry {
    AsfCacheEntry {
        cache_key: "stable-cache-key".to_string(),
        query: query(scenes.len() as u64),
        scenes,
        complete,
        total_count: Some(2),
    }
}

#[test]
fn cache_entries_expire_by_updated_time() {
    let database = TempDatabase::new("expiry");
    let mut store = TaskStore::open(database.path()).unwrap();
    let cached = entry(vec![scene("new", "2024-03-01T00:00:00Z")], false);

    store.upsert_asf_cache_entry(&cached, 1_000).unwrap();

    assert_eq!(
        store.load_fresh_asf_cache_entries(1_200, 200).unwrap(),
        vec![cached]
    );
    assert!(store
        .load_fresh_asf_cache_entries(1_201, 200)
        .unwrap()
        .is_empty());
}

#[test]
fn cache_round_trip_preserves_optional_preview_metadata_without_schema_changes() {
    let database = TempDatabase::new("preview-metadata");
    let mut store = TaskStore::open(database.path()).unwrap();
    let mut preview = scene("preview", "2024-03-01T00:00:00Z");
    preview.provider_product_id = Some("provider-preview".to_string());
    preview.processing_level = Some("GRD_HD".to_string());
    preview.browse_url =
        Some("https://datapool.asf.alaska.edu/BROWSE/provider-preview.jpg".to_string());
    preview.checksum_algorithm = Some("md5".to_string());
    preview.checksum_value = Some("0123456789abcdef0123456789abcdef".to_string());
    let cached = entry(vec![preview], false);

    store.upsert_asf_cache_entry(&cached, 1_000).unwrap();
    let restored = store
        .load_fresh_asf_cache_entries(1_001, 200)
        .unwrap()
        .remove(0);

    assert_eq!(restored, cached);
    assert_eq!(
        restored.scenes[0].browse_url.as_deref(),
        Some("https://datapool.asf.alaska.edu/BROWSE/provider-preview.jpg")
    );
}

#[test]
fn cache_upsert_merges_deduplicates_and_sorts_scenes() {
    let database = TempDatabase::new("merge");
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .upsert_asf_cache_entry(
            &entry(vec![scene("old", "2024-01-01T00:00:00Z")], false),
            1_000,
        )
        .unwrap();
    store
        .upsert_asf_cache_entry(
            &entry(
                vec![
                    scene("new", "2024-03-01T00:00:00Z"),
                    scene("old", "2024-01-01T00:00:00Z"),
                ],
                true,
            ),
            1_100,
        )
        .unwrap();

    let loaded = store.load_fresh_asf_cache_entries(1_100, 0).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].scenes.len(), 2);
    assert_eq!(loaded[0].scenes[0].scene_id, "new");
    assert!(loaded[0].complete);
}

#[test]
fn cache_prune_removes_only_expired_rows() {
    let database = TempDatabase::new("prune");
    let mut store = TaskStore::open(database.path()).unwrap();
    let mut old = entry(vec![scene("old", "2024-01-01T00:00:00Z")], false);
    old.cache_key = "old-key".to_string();
    let mut fresh = entry(vec![scene("fresh", "2024-02-01T00:00:00Z")], false);
    fresh.cache_key = "fresh-key".to_string();
    store.upsert_asf_cache_entry(&old, 100).unwrap();
    store.upsert_asf_cache_entry(&fresh, 200).unwrap();

    assert_eq!(store.prune_expired_asf_cache_entries(250, 100).unwrap(), 1);
    assert_eq!(
        store.load_fresh_asf_cache_entries(250, 100).unwrap()[0].cache_key,
        "fresh-key"
    );
}

#[test]
fn corrupt_cache_json_is_reported_not_skipped() {
    let database = TempDatabase::new("corrupt");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .upsert_asf_cache_entry(
                &entry(vec![scene("one", "2024-01-01T00:00:00Z")], false),
                100,
            )
            .unwrap();
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute("UPDATE asf_search_cache SET entry_json = '{broken'", [])
        .unwrap();
    drop(connection);

    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        store
            .load_fresh_asf_cache_entries(100, 0)
            .unwrap_err()
            .code(),
        "V3_STORE_CORRUPT_ROW"
    );
}
