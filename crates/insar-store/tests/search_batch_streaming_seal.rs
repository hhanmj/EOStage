//! #22 B slice 3(b): the seal streams staged candidates row-by-row instead of
//! collecting the whole batch in memory. RED first: these tests exercise the
//! streaming seal (begin → append chunks → seal) for 0/1/250/251 candidates,
//! prove the published blob is byte-identical to the classic whole-batch
//! seal, and prove a failed or aborted staging leaves no batch, no candidate
//! rows and no staging residue.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    FrozenSearchBatch, FrozenSearchStats, SearchWorkbenchState, SnapshotItem,
    StoreSearchBatchRequest,
};
use insar_store::{StoreSearchBatchStagingRequest, TaskStore};
use serde_json::Map;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-streaming-seal-{label}-{}-{nonce}.sqlite3",
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
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "fixture.catalog".to_owned(),
        display_name: format!("display-{item_id}"),
        acquisition_datetime: Some("2026-08-29T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::from_iter([(
            "note".to_owned(),
            serde_json::Value::String(format!("metadata-{item_id}")),
        )]),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn candidates(count: usize) -> Vec<SnapshotItem> {
    (0..count)
        .map(|index| candidate(&format!("item-{index:05}")))
        .collect()
}

fn batch(batch_id: &str, candidates: Vec<SnapshotItem>) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_owned(),
        provider_id: "fixture.catalog".to_owned(),
        created_at: "2026-08-29T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: None,
        query: Map::new(),
        search_stats: None,
        candidates,
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

fn staged_rows(store: &TaskStore) -> usize {
    store
        .count_staging_rows()
        .expect("staging count must not fail")
}

fn candidate_rows(store: &TaskStore, batch_id: &str) -> usize {
    store
        .count_batch_candidate_rows(batch_id)
        .expect("candidate row count must not fail")
}

/// Stage `items` in bounded chunks like the application service does, then
/// seal with the streaming seal. Returns the store for further assertions.
fn stage_and_seal(store: &mut TaskStore, request_id: &str, batch_id: &str, items: &[SnapshotItem]) {
    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: request_id.to_owned(),
            batch_header: batch(batch_id, Vec::new()),
            workbench: workbench(batch_id),
            max_candidates: 10_000,
        })
        .unwrap();
    for chunk in items.chunks(128) {
        store
            .append_search_batch_staging_candidates(request_id, chunk)
            .unwrap();
    }
    store
        .seal_search_batch_staging_streaming(request_id, Some(stats(items.len() as u64)), true)
        .unwrap();
}

#[test]
fn streaming_seal_agrees_with_the_classic_bounded_blob_seal() {
    for count in [0_usize, 1, 250, 251] {
        let classic = TempDatabase::new("classic");
        let mut classic_store = TaskStore::open(classic.path()).unwrap();
        let items = candidates(count);
        // The classic whole-batch seal: stage everything, then seal with the
        // collected candidate vector in one shot.
        classic_store
            .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
                request_id: "classic-request".to_owned(),
                batch_header: batch(&format!("classic-{count}"), Vec::new()),
                workbench: workbench(&format!("classic-{count}")),
                max_candidates: 10_000,
            })
            .unwrap();
        for chunk in items.chunks(128) {
            classic_store
                .append_search_batch_staging_candidates("classic-request", chunk)
                .unwrap();
        }
        classic_store
            .seal_search_batch_staging("classic-request", Some(stats(items.len() as u64)), true)
            .unwrap();

        let streaming = TempDatabase::new("streaming");
        let mut streaming_store = TaskStore::open(streaming.path()).unwrap();
        stage_and_seal(
            &mut streaming_store,
            "request",
            &format!("streaming-{count}"),
            &items,
        );

        // The published blob must be byte-identical to the classic seal's.
        let classic_json = json_without_batch_id(
            classic_store
                .query_snapshot_json(&format!("classic-{count}"))
                .unwrap()
                .as_deref(),
        );
        let streaming_json = json_without_batch_id(
            streaming_store
                .query_snapshot_json(&format!("streaming-{count}"))
                .unwrap()
                .as_deref(),
        );
        assert_eq!(
            classic_json, streaming_json,
            "blob must stay identical for {count} candidates"
        );
        // Rows and blob agree on count and frozen order.
        let sealed = streaming_store
            .load_search_batch(&format!("streaming-{count}"))
            .unwrap()
            .unwrap_or_else(|| panic!("streaming batch for {count} must be published"));
        assert_eq!(sealed.candidates.len(), count);
        assert_eq!(
            sealed
                .candidates
                .iter()
                .map(|item| item.item_id.as_str())
                .collect::<Vec<_>>(),
            items
                .iter()
                .map(|item| item.item_id.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(sealed.search_stats, Some(stats(count as u64)));
        assert_eq!(
            candidate_rows(&streaming_store, &format!("streaming-{count}")),
            count
        );
        assert_eq!(staged_rows(&streaming_store), 0, "staging is cleaned");
        assert_eq!(
            streaming_store
                .load_workbench()
                .unwrap()
                .unwrap()
                .batch
                .batch_id,
            format!("streaming-{count}")
        );
    }
}

/// The blob embeds the batch id; replace it so two databases can be compared.
fn json_without_batch_id(json: Option<&str>) -> String {
    json.unwrap_or_default()
        .replace("streaming-", "batch-")
        .replace("classic-", "batch-")
}

#[test]
fn streaming_seal_publishes_the_staged_item_id_order() {
    let database = TempDatabase::new("order");
    let mut store = TaskStore::open(database.path()).unwrap();
    // Deliberately not in item-ID order across chunks: the frozen order is
    // the staged item-ID order, exactly like the classic seal.
    let items = vec![candidate("zeta"), candidate("alpha"), candidate("mid")];
    stage_and_seal(&mut store, "request", "ordered", &items);
    let sealed = store.load_search_batch("ordered").unwrap().unwrap();
    assert_eq!(
        sealed
            .candidates
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "mid", "zeta"]
    );
}

#[test]
fn a_failed_streaming_seal_leaves_no_batch_rows_or_staging_residue() {
    let database = TempDatabase::new("rollback");
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch("old-batch", vec![candidate("old")]),
            workbench: workbench("old-batch"),
            activate_workbench: true,
        })
        .unwrap();

    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: "request-fail".to_owned(),
            batch_header: batch("never-published", Vec::new()),
            workbench: workbench("never-published"),
            max_candidates: 10_000,
        })
        .unwrap();
    store
        .append_search_batch_staging_candidates("request-fail", &candidates(251))
        .unwrap();
    // Sealing with statistics that disagree with the staged count must fail
    // and roll the whole publication back.
    assert!(store
        .seal_search_batch_staging_streaming("request-fail", Some(stats(250)), true)
        .is_err());

    assert!(store
        .load_search_batch("never-published")
        .unwrap()
        .is_none());
    assert_eq!(candidate_rows(&store, "never-published"), 0);
    // The failed seal itself leaves no half-published rows behind, but the
    // staging area keeps its rows until an explicit abort cleans them.
    assert_eq!(staged_rows(&store), 251);
    store.abort_search_batch_staging("request-fail").unwrap();
    assert_eq!(staged_rows(&store), 0, "abort removes all staging residue");
    assert_eq!(
        store.load_workbench().unwrap().unwrap().batch.batch_id,
        "old-batch",
        "the previous workbench survives"
    );
}

#[test]
fn cancelling_before_the_streaming_seal_leaves_nothing_staged() {
    let database = TempDatabase::new("cancel");
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: "request-cancel".to_owned(),
            batch_header: batch("cancelled-batch", Vec::new()),
            workbench: workbench("cancelled-batch"),
            max_candidates: 4,
        })
        .unwrap();
    store
        .append_search_batch_staging_candidates("request-cancel", &[candidate("one")])
        .unwrap();
    // A cancellation aborts the staging: no batch, no rows, no residue.
    store.abort_search_batch_staging("request-cancel").unwrap();
    assert!(store
        .load_search_batch("cancelled-batch")
        .unwrap()
        .is_none());
    assert_eq!(candidate_rows(&store, "cancelled-batch"), 0);
    assert_eq!(staged_rows(&store), 0);
    assert!(store.load_workbench().unwrap().is_none());
}

#[test]
fn the_streaming_seal_reads_staged_rows_in_bounded_batches() {
    // The seal must not collect the whole staged set in memory: a counting
    // seam on the store proves the seal's read batch is bounded.
    let database = TempDatabase::new("bounded");
    let mut store = TaskStore::open(database.path()).unwrap();
    let items = candidates(251);
    stage_and_seal(&mut store, "request", "bounded", &items);
    assert_eq!(candidate_rows(&store, "bounded"), 251);
    let max_observed = store.take_max_seal_read_batch();
    assert!(
        max_observed <= 256,
        "the seal's read batch must stay bounded, saw {max_observed}"
    );
    assert!(
        max_observed >= 1,
        "the seal must have read at least one batch"
    );
}

/// #22 B slice 3(b) review fix 2: a streaming seal variant with a catalogue
/// cache pointer must exist, publish the same batch/rows/blob as the classic
/// cache-pointer seal, and write the cache pointer row.
#[test]
fn streaming_cache_pointer_seal_publishes_batch_and_pointer() {
    let database = TempDatabase::new("stream-cache");
    let mut store = TaskStore::open(database.path()).unwrap();
    let items = candidates(120);
    let request_id = "request-stream-cache";
    let batch_id = "stream-cache-batch";
    let query = insar_core::CatalogSearchQuery {
        logical_dataset: "optical.fixture.l2".to_owned(),
        provider_id: "fixture.stac".to_owned(),
        native_collection_id: "fixture-l2".to_owned(),
        aoi_geojson: None,
        cursor: None,
        result_limit: Some(120),
        filters: serde_json::Map::new(),
    };
    // The frozen batch query must round-trip as the same typed query for the
    // cache pointer identity check.
    let mut header = batch(batch_id, Vec::new());
    header.query = serde_json::to_value(&query)
        .unwrap()
        .as_object()
        .unwrap()
        .clone();
    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: request_id.to_owned(),
            batch_header: header,
            workbench: workbench(batch_id),
            max_candidates: 10_000,
        })
        .unwrap();
    for chunk in items.chunks(128) {
        store
            .append_search_batch_staging_candidates(request_id, chunk)
            .unwrap();
    }
    let cache_key = insar_core::catalog_search_cache_key(&query).unwrap();
    let now_epoch = 1_800_000_000_u64;
    store
        .seal_catalog_search_batch_staging_with_cache_streaming(
            request_id,
            Some(stats(items.len() as u64)),
            true,
            &query,
            &cache_key,
            now_epoch,
        )
        .expect("the streaming cache-pointer seal must exist and succeed");
    assert_eq!(candidate_rows(&store, batch_id), 120);
    assert_eq!(staged_rows(&store), 0);
    // The cache pointer row must exist and be complete.
    let cached = store
        .load_catalog_cache_batch(&query, now_epoch, 60)
        .expect("cache load must succeed");
    let cached = cached.expect("the cache pointer must resolve to the batch");
    assert_eq!(cached.candidates.len(), 120);
    // The read-batch seam proves the streaming seal (not the classic one) ran.
    let max_observed = store.take_max_seal_read_batch();
    assert!(
        max_observed >= 1,
        "the streaming seal must have read batches"
    );
}

/// #22 B slice 3(b) review fix 5: the streaming seal must not decode the
/// whole snapshot blob back into a FrozenSearchBatch. A decode-count seam on
/// the store proves no full decode happens; the header is validated in Rust
/// and the (empty) workbench selection is validated against staged ids in SQL.
#[test]
fn streaming_seal_never_decodes_the_whole_blob() {
    let database = TempDatabase::new("no-decode");
    let mut store = TaskStore::open(database.path()).unwrap();
    let items = candidates(600);
    stage_and_seal(&mut store, "request-no-decode", "no-decode-batch", &items);
    assert_eq!(candidate_rows(&store, "no-decode-batch"), 600);
    let decodes = store.take_seal_blob_decode_count();
    assert_eq!(
        decodes, 0,
        "the streaming seal must not decode the whole snapshot blob"
    );
    // The published batch still round-trips correctly for real readers.
    let batch = store
        .load_search_batch("no-decode-batch")
        .unwrap()
        .expect("the sealed batch must load");
    assert_eq!(batch.candidates.len(), 600);
}
