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
            "insar-store-search-staging-{label}-{}-{nonce}.sqlite3",
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
        display_name: item_id.to_owned(),
        acquisition_datetime: None,
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
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

#[test]
fn staging_is_invisible_until_atomic_seal() {
    let database = TempDatabase::new("seal");
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
            request_id: "request-all".to_owned(),
            batch_header: batch("new-batch", Vec::new()),
            workbench: workbench("new-batch"),
            max_candidates: 2,
        })
        .unwrap();
    store
        .append_search_batch_staging_candidates(
            "request-all",
            &[candidate("candidate-b"), candidate("candidate-a")],
        )
        .unwrap();

    assert!(store.load_search_batch("new-batch").unwrap().is_none());
    assert_eq!(
        store.load_workbench().unwrap().unwrap().batch.batch_id,
        "old-batch"
    );

    store
        .seal_search_batch_staging("request-all", Some(stats(2)), true)
        .unwrap();

    let sealed = store.load_search_batch("new-batch").unwrap().unwrap();
    assert_eq!(
        sealed
            .candidates
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<Vec<_>>(),
        ["candidate-a", "candidate-b"]
    );
    assert_eq!(sealed.search_stats, Some(stats(2)));
    assert_eq!(
        store.load_workbench().unwrap().unwrap().batch.batch_id,
        "new-batch"
    );
}

#[test]
fn abort_and_candidate_cap_preserve_the_previous_active_workbench() {
    let database = TempDatabase::new("abort");
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
            request_id: "request-capped".to_owned(),
            batch_header: batch("never-published", Vec::new()),
            workbench: workbench("never-published"),
            max_candidates: 1,
        })
        .unwrap();

    assert!(store
        .append_search_batch_staging_candidates(
            "request-capped",
            &[candidate("one"), candidate("two")],
        )
        .is_err());
    store.abort_search_batch_staging("request-capped").unwrap();

    assert!(store
        .load_search_batch("never-published")
        .unwrap()
        .is_none());
    assert_eq!(
        store.load_workbench().unwrap().unwrap().batch.batch_id,
        "old-batch"
    );
}
