//! #22 B slice 1: schema 39 keeps one row per candidate of a frozen search
//! batch, written in the same transaction as the existing `snapshot_json`
//! blob (dual write). The blob stays the read authority; batches frozen
//! before schema 39 have no rows and keep working unchanged.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, FrozenSearchStats, SearchWorkbenchState, SnapshotItem,
    StoreCreateTaskRequest, StoreSearchBatchRequest,
};
use insar_store::{StoreBatchPageRequest, StoreSearchBatchStagingRequest, TaskStore};
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
            "insar-store-candidate-rows-{label}-{}-{nonce}.sqlite3",
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

fn candidate(item_id: &str) -> SnapshotItem {
    let mut metadata = Map::new();
    metadata.insert("frame".to_owned(), json!("0042"));
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "fixture.catalog".to_owned(),
        display_name: format!("scene {item_id}"),
        acquisition_datetime: Some("2026-08-29T01:02:03Z".to_owned()),
        footprint_geojson: None,
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn batch(batch_id: &str, item_ids: &[&str]) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_owned(),
        provider_id: "fixture.catalog".to_owned(),
        created_at: "2026-08-29T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: None,
        query: Map::new(),
        search_stats: None,
        candidates: item_ids.iter().map(|id| candidate(id)).collect(),
    }
}

fn workbench(batch_id: &str) -> SearchWorkbenchState {
    SearchWorkbenchState {
        batch_id: batch_id.to_owned(),
        checked_item_ids: Vec::new(),
        page_index: 0,
        page_size: 24,
        revision: 0,
    }
}

fn request(batch: FrozenSearchBatch, activate_workbench: bool) -> StoreSearchBatchRequest {
    let workbench = workbench(&batch.batch_id);
    StoreSearchBatchRequest {
        batch,
        workbench,
        activate_workbench,
    }
}

fn stats(count: u64) -> FrozenSearchStats {
    FrozenSearchStats {
        matched_count: Some(count),
        fetched_record_count: Some(count),
        mapped_record_count: Some(count),
        provider_returned_count: count,
        frozen_candidate_count: count,
        truncated: false,
        source_attempts: vec![],
        source: "TYPED_CATALOG".to_owned(),
    }
}

/// `(ordinal, item_id, item_json)` rows of one batch in ordinal order.
fn rows(path: &Path, batch_id: &str) -> Vec<(i64, String, String)> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT ordinal, item_id, item_json FROM search_batch_candidates \
             WHERE batch_id = ?1 ORDER BY ordinal",
        )
        .unwrap();
    statement
        .query_map([batch_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn row_count(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM search_batch_candidates", [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// Rows equal the blob's `candidates` array element by element, in order.
fn assert_rows_match_blob(path: &Path, batch_id: &str) {
    let connection = Connection::open(path).unwrap();
    let blob: Vec<(i64, String, String)> = connection
        .prepare(
            "SELECT CAST(c.key AS INTEGER), json_extract(c.value, '$.item_id'), json(c.value) \
             FROM search_batches b, json_each(b.snapshot_json, '$.candidates') c \
             WHERE b.batch_id = ?1 ORDER BY CAST(c.key AS INTEGER)",
        )
        .unwrap()
        .query_map([batch_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let stored: Vec<(i64, String, String)> = connection
        .prepare(
            "SELECT ordinal, item_id, json(item_json) FROM search_batch_candidates \
             WHERE batch_id = ?1 ORDER BY ordinal",
        )
        .unwrap()
        .query_map([batch_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stored, blob);
}

/// Turns a current Store back into a schema-38 Store: the row table did not
/// exist yet, so batches frozen then have only their blob.
fn rewind_to_v38(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("DROP TABLE search_batch_candidates;")
        .unwrap();
    connection.pragma_update(None, "user_version", 38).unwrap();
}

#[test]
fn schema_39_creates_the_candidate_row_table() {
    let database = TempDatabase::new("schema");
    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(insar_store::STORE_SCHEMA_VERSION, 43);
    assert_eq!(store.health().unwrap().schema_version, 43);
    assert!(store
        .table_names()
        .unwrap()
        .iter()
        .any(|name| name == "search_batch_candidates"));
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('search_batch_candidates') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(columns, ["batch_id", "ordinal", "item_id", "item_json"]);
}

#[test]
fn saving_a_batch_writes_its_candidates_as_rows_in_frozen_order() {
    let database = TempDatabase::new("save");
    let mut store = TaskStore::open(database.path()).unwrap();
    // Deliberately not in item-ID order: the frozen order is the blob order.
    store
        .save_search_batch(&request(batch("b1", &["zeta", "alpha", "mid"]), true))
        .unwrap();
    store
        .save_search_batch(&request(batch("empty", &[]), false))
        .unwrap();

    let stored = rows(database.path(), "b1");
    assert_eq!(
        stored
            .iter()
            .map(|(ordinal, id, _)| (*ordinal, id.as_str()))
            .collect::<Vec<_>>(),
        [(0, "zeta"), (1, "alpha"), (2, "mid")]
    );
    let decoded: SnapshotItem = serde_json::from_str(&stored[1].2).unwrap();
    assert_eq!(decoded, candidate("alpha"));
    assert_rows_match_blob(database.path(), "b1");
    assert!(rows(database.path(), "empty").is_empty());

    // The blob read path is unchanged.
    let loaded = store.load_search_batch("b1").unwrap().unwrap();
    assert_eq!(loaded, batch("b1", &["zeta", "alpha", "mid"]));
}

#[test]
fn sealing_staged_candidates_writes_the_sealed_order_as_rows() {
    let database = TempDatabase::new("seal");
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: "request".to_owned(),
            batch_header: batch("sealed", &[]),
            workbench: workbench("sealed"),
            max_candidates: 3,
        })
        .unwrap();
    store
        .append_search_batch_staging_candidates("request", &[candidate("c"), candidate("a")])
        .unwrap();
    store
        .append_search_batch_staging_candidates("request", &[candidate("b")])
        .unwrap();
    assert_eq!(row_count(database.path()), 0, "staging publishes no rows");

    store
        .seal_search_batch_staging("request", Some(stats(3)), true)
        .unwrap();

    let sealed = store.load_search_batch("sealed").unwrap().unwrap();
    let blob_order = sealed
        .candidates
        .iter()
        .map(|item| item.item_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        rows(database.path(), "sealed")
            .into_iter()
            .map(|(_, id, _)| id)
            .collect::<Vec<_>>(),
        blob_order
    );
    assert_rows_match_blob(database.path(), "sealed");
}

#[test]
fn re_saving_a_batch_does_not_duplicate_or_replace_its_rows() {
    let database = TempDatabase::new("resave");
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&request(batch("b1", &["one", "two"]), true))
        .unwrap();
    let first = rows(database.path(), "b1");

    // Identical re-save is accepted and changes nothing.
    store
        .save_search_batch(&request(batch("b1", &["one", "two"]), true))
        .unwrap();
    assert_eq!(rows(database.path(), "b1"), first);

    // A different batch under the same ID is refused and leaves the rows.
    assert!(store
        .save_search_batch(&request(batch("b1", &["one", "two", "three"]), true))
        .is_err());
    assert_eq!(rows(database.path(), "b1"), first);
    assert_eq!(row_count(database.path()), 2);
}

#[test]
fn a_failed_combined_save_leaves_neither_the_batch_nor_its_rows() {
    let database = TempDatabase::new("rollback");
    let mut store = TaskStore::open(database.path()).unwrap();
    let result = store.save_search_batch_and_create_task(
        &request(batch("rolled-back", &["one", "two"]), true),
        &StoreCreateTaskRequest {
            task_id: "task".to_owned(),
            batch_id: "rolled-back".to_owned(),
            // Not a candidate of the batch: task creation fails after the
            // batch and its rows were written in the same transaction.
            selected_item_ids: vec!["missing".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: PathBuf::from("fixture-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-08-29T01:00:00Z".to_owned(),
        },
    );
    assert!(result.is_err());
    assert!(store.load_search_batch("rolled-back").unwrap().is_none());
    assert_eq!(row_count(database.path()), 0);
}

#[test]
fn candidate_rows_are_frozen_and_follow_their_batch_on_delete() {
    let database = TempDatabase::new("frozen");
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&request(batch("unreferenced", &["one", "two"]), false))
        .unwrap();
    drop(store);

    let connection = Connection::open(database.path()).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    assert!(connection
        .execute(
            "UPDATE search_batch_candidates SET item_json = '{}' WHERE batch_id = 'unreferenced'",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO search_batch_candidates(batch_id, ordinal, item_id, item_json) \
             VALUES ('unreferenced', 0, 'other', '{\"a\":1}')",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO search_batch_candidates(batch_id, ordinal, item_id, item_json) \
             VALUES ('unknown-batch', 0, 'one', '{\"a\":1}')",
            [],
        )
        .is_err());
    // The Store has no batch deletion today; if one is added, the rows go
    // with their batch instead of being left behind.
    connection
        .execute(
            "DELETE FROM search_batches WHERE batch_id = 'unreferenced'",
            [],
        )
        .unwrap();
    assert_eq!(row_count(database.path()), 0);
}

#[test]
fn a_v38_store_upgrades_without_rows_for_old_batches_and_old_batches_keep_working() {
    let database = TempDatabase::new("upgrade");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&request(batch("old", &["b", "a"]), true))
            .unwrap();
    }
    rewind_to_v38(database.path());

    let mut store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(store.health().unwrap().schema_version, 43);
    assert_eq!(row_count(database.path()), 0, "no lazy backfill");

    let old = store.load_search_batch("old").unwrap().unwrap();
    assert_eq!(old, batch("old", &["b", "a"]));
    let page = store
        .load_batch_page(&StoreBatchPageRequest {
            batch_id: "old".to_owned(),
            revision: 0,
            page: 0,
            page_size: 2,
        })
        .unwrap();
    assert_eq!(page.total_count, 2);
    assert_eq!(
        store.load_workbench().unwrap().unwrap().batch.batch_id,
        "old"
    );
    // Re-saving the unchanged old batch does not backfill it either.
    store
        .save_search_batch(&request(batch("old", &["b", "a"]), true))
        .unwrap();
    assert!(rows(database.path(), "old").is_empty());

    store
        .save_search_batch(&request(batch("new", &["x", "y"]), true))
        .unwrap();
    assert_rows_match_blob(database.path(), "new");
    assert_eq!(row_count(database.path()), 2);
}
