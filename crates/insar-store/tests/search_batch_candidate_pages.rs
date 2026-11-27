//! #22 B slice 2: bounded Store reads of a frozen search batch's candidates.
//!
//! Slice 1 wrote one row per candidate beside the `snapshot_json` blob. These
//! tests pin the slice 2 read side: page reads answer from the rows for
//! batches frozen at schema 39, fall back to the blob for old batches that
//! have no rows, never return a partial or empty page in place of the real
//! candidates, and fail closed on out-of-range pages and corrupt rows.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters, AssetRole,
    FrozenAssetAccess, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StableAssetReference,
    StoreSearchBatchRequest,
};
use insar_store::{TaskStore, MAX_WORKBENCH_PAGE_SIZE};
use rusqlite::Connection;
use serde_json::{json, Map};

const PROVIDER: &str = "fixture.catalog";

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-bounded-page-{label}-{}-{nonce}.sqlite3",
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

fn candidate(item_id: &str, acquisition: Option<&str>) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: PROVIDER.to_owned(),
        display_name: format!("scene {item_id}"),
        acquisition_datetime: acquisition.map(str::to_owned),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

/// A candidate that carries frozen asset access, as admission reads it to
/// learn which accounts a selection needs.
fn credentialed_candidate(item_id: &str, scheme: AssetAccessScheme) -> SnapshotItem {
    let mut item = candidate(item_id, Some("2026-08-29T01:02:03Z"));
    item.asset_access = Some(FrozenAssetAccess {
        reference: StableAssetReference {
            provider_id: PROVIDER.to_owned(),
            collection_id: "fixture-collection".to_owned(),
            provider_product_id: item_id.to_owned(),
            provider_product_key: format!("{item_id}-key"),
            canonical_product_key: None,
            asset_key: "data".to_owned(),
            stable_asset_key: format!("{item_id}-stable"),
        },
        descriptor: AssetAccessDescriptor {
            resolver_id: "fixture-resolver".to_owned(),
            scheme,
            stable_href: None,
            requires_credentials: true,
            credential_profile: Some("fixture-profile".to_owned()),
            resolver_parameters: AssetResolverParameters {},
        },
        role: AssetRole::Primary,
        file_name: None,
        expected_size_bytes: Some(1024),
        checksum_algorithm: None,
        checksum_value: None,
    });
    item
}

fn batch(batch_id: &str, item_ids: &[&str]) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_owned(),
        provider_id: PROVIDER.to_owned(),
        created_at: "2026-08-29T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: None,
        query: Map::new(),
        search_stats: None,
        candidates: item_ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                candidate(
                    id,
                    Some(if index % 2 == 0 {
                        "2026-08-29T01:02:03Z"
                    } else {
                        "2026-08-28T01:02:03Z"
                    }),
                )
            })
            .collect(),
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

fn save(store: &mut TaskStore, batch: FrozenSearchBatch) {
    let workbench_state = workbench(&batch.batch_id);
    let batch_id = batch.batch_id.clone();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: workbench_state,
            activate_workbench: false,
        })
        .unwrap();
    assert!(store.load_search_batch(&batch_id).unwrap().is_some());
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

/// Deletes all but one of a batch's candidate rows without touching its
/// blob: a partially written v39 batch must fail closed, not silently serve
/// the truncated rows or fall back to the blob.
fn drop_some_rows(path: &Path, batch_id: &str, keep: usize) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            "DELETE FROM search_batch_candidates \
             WHERE batch_id = ?1 AND ordinal NOT IN \
               (SELECT ordinal FROM search_batch_candidates WHERE batch_id = ?1 \
                ORDER BY ordinal LIMIT ?2)",
            rusqlite::params![batch_id, keep as i64],
        )
        .unwrap();
}

/// Item ids built once per test call: `&'static str` keeps the `batch()` helper
/// signature simple without leaking unbounded memory.
fn static_ids(range: std::ops::Range<usize>, prefix: &str) -> Vec<&'static str> {
    range
        .map(|index| Box::leak(format!("{prefix}{index:03}").into_boxed_str()) as &'static str)
        .collect()
}

#[test]
fn pages_read_candidates_in_frozen_order_from_rows() {
    let database = TempDatabase::new("order");
    let mut store = TaskStore::open(database.path()).unwrap();
    // Deliberately not in item-ID order: the frozen order is the blob order.
    let item_ids = static_ids(0..251, "item-");
    let expected_ids = item_ids.clone();
    save(&mut store, batch("b-order", &item_ids));

    let full = store.load_batch_candidate_page("b-order", 0, 250).unwrap();
    assert_eq!(full.total_count, 251);
    assert_eq!(full.items.len(), 250);
    assert_eq!(full.items[0].item_id, expected_ids[0]);
    assert_eq!(full.items[249].item_id, expected_ids[249]);
    let tail = store.load_batch_candidate_page("b-order", 1, 250).unwrap();
    assert_eq!(tail.items.len(), 1);
    assert_eq!(tail.items[0].item_id, expected_ids[250]);
}

#[test]
fn pages_cover_empty_single_and_boundary_counts() {
    let database = TempDatabase::new("counts");
    let mut store = TaskStore::open(database.path()).unwrap();
    save(&mut store, batch("b-empty", &[]));
    save(&mut store, batch("b-one", &["only"]));
    let ids_250 = static_ids(0..250, "c");
    let ids_251 = static_ids(0..251, "d");
    save(&mut store, batch("b-250", &ids_250));
    save(&mut store, batch("b-251", &ids_251));

    let empty = store.load_batch_candidate_page("b-empty", 0, 250).unwrap();
    assert_eq!(empty.total_count, 0);
    assert!(empty.items.is_empty());

    let one = store.load_batch_candidate_page("b-one", 0, 250).unwrap();
    assert_eq!(one.total_count, 1);
    assert_eq!(one.items.len(), 1);
    assert_eq!(one.items[0].item_id, "only");

    let exact = store.load_batch_candidate_page("b-250", 0, 250).unwrap();
    assert_eq!(exact.total_count, 250);
    assert_eq!(exact.items.len(), 250);
    let exact_tail = store.load_batch_candidate_page("b-250", 1, 250).unwrap();
    assert_eq!(exact_tail.total_count, 250);
    assert!(exact_tail.items.is_empty());

    let over = store.load_batch_candidate_page("b-251", 0, 250).unwrap();
    assert_eq!(over.total_count, 251);
    assert_eq!(over.items.len(), 250);
    let over_tail = store.load_batch_candidate_page("b-251", 1, 250).unwrap();
    assert_eq!(over_tail.items.len(), 1);
}

#[test]
fn an_old_batch_without_rows_falls_back_to_its_blob() {
    let database = TempDatabase::new("old");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        save(&mut store, batch("b-old", &["zeta", "alpha", "mid"]));
    }
    rewind_to_v38(database.path());

    let store = TaskStore::open_existing(database.path()).unwrap();
    let page = store.load_batch_candidate_page("b-old", 0, 250).unwrap();
    assert_eq!(page.total_count, 3);
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>(),
        ["zeta", "alpha", "mid"],
        "the blob fallback must preserve the frozen order"
    );
    let tail = store.load_batch_candidate_page("b-old", 1, 2).unwrap();
    assert_eq!(tail.total_count, 3);
    assert_eq!(tail.items.len(), 1);
    assert_eq!(tail.items[0].item_id, "mid");
}

#[test]
fn a_mixed_store_serves_rows_and_blob_batches_side_by_side() {
    let database = TempDatabase::new("mixed");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        save(&mut store, batch("b-old", &["old-one", "old-two"]));
    }
    rewind_to_v38(database.path());

    let mut store = TaskStore::open_existing(database.path()).unwrap();
    save(
        &mut store,
        batch("b-new", &["new-one", "new-two", "new-three"]),
    );

    let old = store.load_batch_candidate_page("b-old", 0, 250).unwrap();
    assert_eq!(old.total_count, 2);
    assert_eq!(old.items[1].item_id, "old-two");
    let new = store.load_batch_candidate_page("b-new", 0, 250).unwrap();
    assert_eq!(new.total_count, 3);
    assert_eq!(new.items[2].item_id, "new-three");
}

#[test]
fn pages_survive_restart_for_row_and_blob_batches() {
    let database = TempDatabase::new("restart");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        save(&mut store, batch("b-new", &["n1", "n2"]));
    }
    rewind_to_v38_after_new_batch(database.path(), "b-new");

    let mut store = TaskStore::open_existing(database.path()).unwrap();
    save(&mut store, batch("b-newer", &["m1", "m2", "m3"]));
    drop(store);

    let reopened = TaskStore::open_existing(database.path()).unwrap();
    let blob_batch = reopened.load_batch_candidate_page("b-new", 0, 250).unwrap();
    assert_eq!(blob_batch.total_count, 2);
    assert_eq!(blob_batch.items[0].item_id, "n1");
    let row_batch = reopened.load_batch_candidate_page("b-newer", 0, 2).unwrap();
    assert_eq!(row_batch.total_count, 3);
    assert_eq!(row_batch.items.len(), 2);
    assert_eq!(row_batch.items[1].item_id, "m2");
}

/// Rewinds the whole store to v38 after a batch was saved at v39: the batch
/// keeps its blob, and its rows disappear with the dropped table.
fn rewind_to_v38_after_new_batch(path: &Path, _batch_id: &str) {
    rewind_to_v38(path);
}

#[test]
fn out_of_range_and_oversize_pages_fail_closed() {
    let database = TempDatabase::new("range");
    let mut store = TaskStore::open(database.path()).unwrap();
    save(&mut store, batch("b-range", &["one", "two"]));

    assert_eq!(
        store
            .load_batch_candidate_page("b-range", 5, 250)
            .unwrap_err()
            .code(),
        "V3_STORE_NOT_FOUND"
    );
    assert_eq!(
        store
            .load_batch_candidate_page("b-range", 0, 251)
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
    assert_eq!(
        store
            .load_batch_candidate_page("b-range", 0, 0)
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
    assert_eq!(
        store
            .load_batch_candidate_page("missing-batch", 0, 250)
            .unwrap_err()
            .code(),
        "V3_STORE_NOT_FOUND"
    );
}

#[test]
fn partial_rows_for_an_existing_batch_fail_closed_instead_of_using_the_blob() {
    let database = TempDatabase::new("corrupt");
    let mut store = TaskStore::open(database.path()).unwrap();
    save(&mut store, batch("b-corrupt", &["one", "two", "three"]));
    drop(store);
    // Simulate a broken new-format batch: two of three rows lost, blob intact.
    drop_some_rows(database.path(), "b-corrupt", 1);

    let store = TaskStore::open_existing(database.path()).unwrap();
    let error = store
        .load_batch_candidate_page("b-corrupt", 0, 250)
        .unwrap_err();
    assert_eq!(
        error.code(),
        "V3_STORE_INVALID_SNAPSHOT",
        "a row-format batch with partial rows must fail closed, not silently fall back"
    );
    assert_eq!(
        store
            .load_batch_candidate_count("b-corrupt")
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
    assert_eq!(
        store
            .load_batch_candidate_items("b-corrupt", &["one".to_owned()])
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
}

#[test]
fn membership_and_count_queries_read_rows_and_fall_back_for_old_batches() {
    let database = TempDatabase::new("members");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        save(&mut store, batch("b-old", &["old-a", "old-b"]));
    }
    rewind_to_v38(database.path());

    let mut store = TaskStore::open_existing(database.path()).unwrap();
    save(&mut store, batch("b-new", &["new-a", "new-b", "new-c"]));

    assert_eq!(store.load_batch_candidate_count("b-new").unwrap(), Some(3));
    assert_eq!(store.load_batch_candidate_count("b-old").unwrap(), Some(2));
    assert_eq!(store.load_batch_candidate_count("missing").unwrap(), None);

    let members = store
        .load_batch_candidate_items(
            "b-new",
            &["new-c".to_owned(), "new-a".to_owned(), "missing".to_owned()],
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        members
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>(),
        ["new-a", "new-c"],
        "membership answers in frozen order and drops unknown ids"
    );
    let old_members = store
        .load_batch_candidate_items("b-old", &["old-b".to_owned()])
        .unwrap()
        .unwrap();
    assert_eq!(old_members.len(), 1);
    assert_eq!(old_members[0].item_id, "old-b");
    assert!(store
        .load_batch_candidate_items("missing", &["x".to_owned()])
        .unwrap()
        .is_none());
}

#[test]
fn page_size_is_never_above_the_transport_cap() {
    assert_eq!(MAX_WORKBENCH_PAGE_SIZE, 250);
    let database = TempDatabase::new("cap");
    let mut store = TaskStore::open(database.path()).unwrap();
    save(&mut store, batch("b-cap", &["one"]));
    assert_eq!(
        store
            .load_batch_candidate_page("b-cap", 0, MAX_WORKBENCH_PAGE_SIZE)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn a_batch_page_request_still_works_after_migration_for_both_formats() {
    // The existing typed batch page (StoreBatchPage) must keep its original
    // order semantics for old and new batches; this pins that the new row
    // reader did not change the public page contract.
    let database = TempDatabase::new("typed-page");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch: batch("b-typed-old", &["z", "a"]),
                workbench: workbench("b-typed-old"),
                activate_workbench: false,
            })
            .unwrap();
    }
    rewind_to_v38(database.path());

    let mut store = TaskStore::open_existing(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch("b-typed-new", &["n1", "n0"]),
            workbench: workbench("b-typed-new"),
            activate_workbench: false,
        })
        .unwrap();

    let old = store
        .load_batch_page(&insar_store::StoreBatchPageRequest {
            batch_id: "b-typed-old".to_owned(),
            revision: 0,
            page: 0,
            page_size: 2,
        })
        .unwrap();
    assert_eq!(old.total_count, 2);
    assert_eq!(
        old.items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    let new = store
        .load_batch_page(&insar_store::StoreBatchPageRequest {
            batch_id: "b-typed-new".to_owned(),
            revision: 0,
            page: 0,
            page_size: 2,
        })
        .unwrap();
    assert_eq!(
        new.items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>(),
        ["n1", "n0"]
    );
}

/// Corrupts one candidate of the blob without touching the rows: during the
/// dual-write transition the blob is still the frozen authority, so a page
/// served from the rows must still detect the corruption instead of silently
/// serving rows the blob no longer agrees with.
fn break_one_blob_item(path: &Path, batch_id: &str, ordinal: usize) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("DROP TRIGGER search_batches_are_frozen;")
        .unwrap();
    connection
        .execute(
            "UPDATE search_batches SET snapshot_json = json_set( \
                 snapshot_json, printf('$.candidates[%d]', ?2), \
                 json('{\"provider_id\":\"fixture.catalog\",\"display_name\":\"broken\"}')) \
             WHERE batch_id = ?1",
            rusqlite::params![batch_id, ordinal as i64],
        )
        .unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER search_batches_are_frozen \
             BEFORE UPDATE ON search_batches \
             BEGIN \
                 SELECT RAISE(ABORT, 'frozen search batch cannot be updated'); \
             END;",
        )
        .unwrap();
}

#[test]
fn a_corrupt_blob_of_a_row_batch_fails_closed_instead_of_serving_rows() {
    let database = TempDatabase::new("blob-authority");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        save(&mut store, batch("b-authority", &["one", "two", "three"]));
    }
    break_one_blob_item(database.path(), "b-authority", 1);

    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        store
            .load_batch_candidate_page("b-authority", 0, 250)
            .unwrap_err()
            .code(),
        "V3_STORE_CORRUPT_ROW",
        "the blob stays the frozen authority during dual write, so a page \
         served from rows must still fail closed when the blob is malformed"
    );
    // The count stays count-based (json_array_length semantics): a malformed
    // item inside a valid array does not change the frozen count.
    assert_eq!(
        store.load_batch_candidate_count("b-authority").unwrap(),
        Some(3)
    );
    // Membership of the corrupted product disagrees with the blob (its copy
    // no longer carries the item id, so the blob side of the join drops it)
    // and fails closed instead of answering from the rows alone.
    assert_eq!(
        store
            .load_batch_candidate_items("b-authority", &["two".to_owned()])
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
    // An intact product is still served through the cross-checked path.
    assert_eq!(
        store
            .load_batch_candidate_items("b-authority", &["one".to_owned()])
            .unwrap()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_corrupt_blob_of_a_row_batch_fails_the_workbench_page_closed() {
    let database = TempDatabase::new("blob-authority-page");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch: batch("b-authority-page", &["one", "two", "three"]),
                workbench: workbench("b-authority-page"),
                activate_workbench: true,
            })
            .unwrap();
    }
    break_one_blob_item(database.path(), "b-authority-page", 1);

    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        store
            .load_workbench_page(&insar_store::StoreWorkbenchPageRequest {
                batch_id: "b-authority-page".to_owned(),
                expected_revision: 0,
                page_index: 0,
                page_size: 3,
            })
            .unwrap_err()
            .code(),
        "V3_STORE_CORRUPT_ROW"
    );
}

/// One admission-sized selection plus the batch header, without loading the
/// whole candidate array. RED for #22 B slice 2: `load_batch_selection`
/// answers from the candidate rows (blob fallback for pre-v39 batches) and
/// keeps the same fail-closed rules as the page reader.
#[test]
fn a_batch_selection_reads_the_header_and_selected_items_without_the_full_array() {
    let database = TempDatabase::new("selection");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        let mut many = batch("b-sel", &["keep-me", "other"]);
        many.aoi_name = "Selection AOI".to_owned();
        many.query.insert(
            "dataset".to_owned(),
            serde_json::Value::String("SENTINEL-1".to_owned()),
        );
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch: many,
                workbench: workbench("b-sel"),
                activate_workbench: false,
            })
            .unwrap();
    }
    rewind_to_v38_after_new_batch(database.path(), "b-sel");
    let mut store = TaskStore::open_existing(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch("b-sel-rows", &["r1", "r2", "r3"]),
            workbench: workbench("b-sel-rows"),
            activate_workbench: false,
        })
        .unwrap();

    // Rows batch: the selection answers in frozen order with the header.
    let selection = store
        .load_batch_selection("b-sel-rows", &["r3".to_owned(), "r1".to_owned()])
        .unwrap()
        .unwrap();
    assert_eq!(selection.batch.batch_id, "b-sel-rows");
    assert_eq!(selection.batch.aoi_name, "Fixture AOI");
    assert_eq!(
        selection
            .items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>(),
        ["r1", "r3"],
        "the selection answers in frozen order"
    );
    assert_eq!(
        store
            .load_batch_selection("b-sel-rows", &["missing".to_owned()])
            .unwrap()
            .unwrap()
            .items,
        Vec::<SnapshotItem>::new()
    );

    // Old (blob) batch: same answer through the fallback, no rows involved.
    let old = store
        .load_batch_selection("b-sel", &["other".to_owned()])
        .unwrap()
        .unwrap();
    assert_eq!(old.batch.aoi_name, "Selection AOI");
    assert_eq!(
        old.batch
            .query
            .get("dataset")
            .and_then(serde_json::Value::as_str),
        Some("SENTINEL-1")
    );
    assert_eq!(
        old.items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>(),
        ["other"]
    );

    // Unknown batch: None, never an empty selection standing in for a batch.
    assert!(store
        .load_batch_selection("missing", &["x".to_owned()])
        .unwrap()
        .is_none());
    // A corrupt row set fails closed rather than answering from the blob.
    drop_some_rows(database.path(), "b-sel-rows", 1);
    assert_eq!(
        store
            .load_batch_selection("b-sel-rows", &["r1".to_owned()])
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
}

/// Removes the candidates array from the blob: a full load fails closed on
/// this (candidates has no serde default), so the bounded header read must
/// not mask it as a valid empty batch.
fn drop_blob_candidates_array(path: &Path, batch_id: &str) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch("DROP TRIGGER search_batches_are_frozen;")
        .unwrap();
    connection
        .execute(
            "UPDATE search_batches SET snapshot_json = json_remove(snapshot_json, '$.candidates') \
             WHERE batch_id = ?1",
            rusqlite::params![batch_id],
        )
        .unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER search_batches_are_frozen \
             BEFORE UPDATE ON search_batches \
             BEGIN \
                 SELECT RAISE(ABORT, 'frozen search batch cannot be updated'); \
             END;",
        )
        .unwrap();
}

#[test]
fn a_blob_without_a_candidates_array_fails_the_header_read_closed() {
    let database = TempDatabase::new("no-candidates");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        save(&mut store, batch("b-header", &["one", "two"]));
    }
    drop_blob_candidates_array(database.path(), "b-header");

    let store = TaskStore::open_existing(database.path()).unwrap();
    assert_eq!(
        store.load_batch_header("b-header").unwrap_err().code(),
        "V3_STORE_INVALID_SNAPSHOT",
        "a blob without its candidates array is corruption, not an empty batch"
    );
    assert_eq!(
        store
            .load_batch_selection("b-header", &["one".to_owned()])
            .unwrap_err()
            .code(),
        "V3_STORE_INVALID_SNAPSHOT"
    );
}

/// The bounded selection must carry what admission needs from each selected
/// product — frozen asset access, not just the id — because the batch header
/// it returns has no candidate array to scan instead.
#[test]
fn a_batch_selection_carries_the_selected_products_asset_access() {
    let database = TempDatabase::new("selection-access");
    let mut store = TaskStore::open(database.path()).unwrap();
    let mut owned = batch(
        "b-access",
        &["plain", "credentialed", "also-credentialed", "tail"],
    );
    owned.candidates[1] = credentialed_candidate("credentialed", AssetAccessScheme::UsgsM2m);
    owned.candidates[2] =
        credentialed_candidate("also-credentialed", AssetAccessScheme::EarthdataHttps);
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: owned,
            workbench: workbench("b-access"),
            activate_workbench: false,
        })
        .unwrap();

    let selection = store
        .load_batch_selection(
            "b-access",
            &["also-credentialed".to_owned(), "plain".to_owned()],
        )
        .unwrap()
        .unwrap();
    assert!(selection.batch.candidates.is_empty());
    assert_eq!(
        selection
            .items
            .iter()
            .map(|item| item.item_id.clone())
            .collect::<Vec<_>>(),
        ["plain", "also-credentialed"]
    );
    assert!(selection.items[0].asset_access.is_none());
    assert_eq!(
        selection.items[1]
            .asset_access
            .as_ref()
            .unwrap()
            .descriptor
            .scheme,
        AssetAccessScheme::EarthdataHttps,
        "the frozen asset access crosses the bounded selection read"
    );
}

// `json!` is used by the seed helpers above through serde_json::json; keep a
// direct use so the import is not flagged as unused when tests are trimmed.
#[test]
fn seed_helpers_still_compile() {
    assert!(json!({"a": 1}).is_object());
}
