use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use insar_core::{FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest};
use insar_store::{StoreWorkbenchPageRequest, TaskStore};
use rusqlite::Connection;
use serde_json::{json, Map, Value};

const BATCH_ID: &str = "ua0-page-batch-10000";
const CREATED_AT: &str = "2026-08-13T04:00:00Z";

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
            "insar-store-ua0-page-{}-{nonce}-{seq}.sqlite3",
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

fn map(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn candidate_id(index: usize) -> String {
    match index {
        0 => "Z".to_owned(),
        1 => "A".to_owned(),
        2 => "M".to_owned(),
        _ => format!("PAGE-ITEM-{index:05}"),
    }
}

fn candidate(index: usize) -> SnapshotItem {
    SnapshotItem {
        item_id: candidate_id(index),
        provider_id: "asf.sentinel1".to_owned(),
        display_name: format!("Sanitized page item {index:05}"),
        acquisition_datetime: Some("2026-08-13T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: map(json!({
            "dataset": "SENTINEL-1 BURSTS",
            "ordinal": index
        })),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed(database: &TempDatabase) -> TaskStore {
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: BATCH_ID.to_owned(),
                provider_id: "asf.sentinel1".to_owned(),
                created_at: CREATED_AT.to_owned(),
                aoi_name: "Sanitized page AOI".to_owned(),
                aoi_geojson: Some(map(json!({
                    "type": "Polygon",
                    "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
                }))),
                query: map(json!({"dataset": "SENTINEL-1 BURSTS"})),
                search_stats: None,
                candidates: (0..10_000).map(candidate).collect(),
            },
            workbench: SearchWorkbenchState {
                batch_id: BATCH_ID.to_owned(),
                checked_item_ids: vec![
                    candidate_id(0),
                    candidate_id(5_000),
                    candidate_id(9_999),
                ],
                page_index: 0,
                page_size: 100,
                revision: 7,
            },
        })
        .unwrap();
    store
}

fn frozen_json(database: &TempDatabase) -> Vec<u8> {
    Connection::open(database.path())
        .unwrap()
        .query_row(
            "SELECT CAST(snapshot_json AS BLOB) FROM search_batches WHERE batch_id = ?1",
            [BATCH_ID],
            |row| row.get(0),
        )
        .unwrap()
}

fn request(page_index: u64, page_size: u64) -> StoreWorkbenchPageRequest {
    StoreWorkbenchPageRequest {
        batch_id: BATCH_ID.to_owned(),
        expected_revision: 7,
        page_index,
        page_size,
    }
}

#[test]
fn json1_page_reads_are_bounded_ordered_checked_and_immutable() {
    let database = TempDatabase::new();
    let store = seed(&database);
    let before = frozen_json(&database);

    for (page_index, first, last, checked) in [
        (0, 0, 99, 0),
        (50, 5_000, 5_099, 5_000),
        (99, 9_900, 9_999, 9_999),
    ] {
        let started = Instant::now();
        let page = store
            .load_workbench_page(&request(page_index, 100))
            .unwrap();
        assert!(started.elapsed().as_secs_f64() < 10.0);
        assert_eq!(page.batch_id, BATCH_ID);
        assert_eq!(page.revision, 7);
        assert_eq!(page.page_index, page_index);
        assert_eq!(page.page_size, 100);
        assert_eq!(page.total_count, 10_000);
        assert_eq!(page.checked_count, 3);
        assert_eq!(page.items.len(), 100);
        assert_eq!(page.items[0].ordinal, first);
        assert_eq!(page.items[99].ordinal, last);
        assert_eq!(page.items[0].item.item_id, candidate_id(first as usize));
        assert_eq!(page.items[99].item.item_id, candidate_id(last as usize));
        if page_index == 0 {
            let provider_ids = page
                .items
                .iter()
                .take(3)
                .map(|item| item.item.item_id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(provider_ids, ["Z", "A", "M"]);
            let mut lexicographic_ids = provider_ids.clone();
            lexicographic_ids.sort_unstable();
            assert_eq!(
                lexicographic_ids,
                ["A", "M", "Z"],
                "this fixture must fail if page materialization lexical-sorts frozen candidates"
            );
        }
        assert_eq!(
            page.items
                .iter()
                .filter(|item| item.checked)
                .map(|item| item.ordinal)
                .collect::<Vec<_>>(),
            [checked]
        );
        let serialized = serde_json::to_string(&page).unwrap();
        assert!(!serialized.contains("PAGE-ITEM-00001") || page_index == 0);
        assert!(!serialized.contains("PAGE-ITEM-09999") || page_index == 99);
        assert!(serialized.len() < before.len());
    }

    let final_empty = store.load_workbench_page(&request(100, 100)).unwrap();
    assert!(final_empty.items.is_empty());
    assert_eq!(final_empty.total_count, 10_000);
    assert_eq!(frozen_json(&database), before);

    drop(store);
    let reopened = TaskStore::open(database.path()).unwrap();
    let reopened_page = reopened.load_workbench_page(&request(0, 100)).unwrap();
    assert_eq!(
        reopened_page
            .items
            .iter()
            .take(3)
            .map(|item| item.item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["Z", "A", "M"],
        "reopening the Store must retain frozen provider order"
    );
}

#[test]
fn page_request_rejects_oversize_stale_wrong_batch_and_offset_overflow() {
    let database = TempDatabase::new();
    let store = seed(&database);

    assert_eq!(
        store
            .load_workbench_page(&request(0, 251))
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
    assert_eq!(
        store
            .load_workbench_page(&request(0, 0))
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
    let mut stale = request(0, 100);
    stale.expected_revision = 6;
    assert_eq!(
        store.load_workbench_page(&stale).unwrap_err().code(),
        "V3_STORE_REVISION_CONFLICT"
    );
    let mut wrong_batch = request(0, 100);
    wrong_batch.batch_id = "other-batch".to_owned();
    assert_eq!(
        store.load_workbench_page(&wrong_batch).unwrap_err().code(),
        "V3_STORE_NOT_FOUND"
    );
    assert_eq!(
        store
            .load_workbench_page(&request(u64::MAX, 200))
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
}

#[test]
fn malformed_page_item_fails_closed_without_materializing_a_partial_page() {
    let database = TempDatabase::new();
    let store = seed(&database);
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER search_batches_are_frozen;
             UPDATE search_batches
             SET snapshot_json = json_set(
                 snapshot_json,
                 '$.candidates[50]',
                 json('{\"provider_id\":\"asf.sentinel1\",\"display_name\":\"broken\"}')
             )
             WHERE batch_id = 'ua0-page-batch-10000';
             CREATE TRIGGER search_batches_are_frozen
             BEFORE UPDATE ON search_batches
             BEGIN
                 SELECT RAISE(ABORT, 'frozen search batch cannot be updated');
             END;",
        )
        .unwrap();
    drop(connection);

    assert_eq!(
        store
            .load_workbench_page(&request(0, 100))
            .unwrap_err()
            .code(),
        "V3_STORE_CORRUPT_ROW"
    );
}
