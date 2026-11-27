//! #22 B slice 3(c): progressive delivery on the Store. Once one full page
//! of rows is staged, a search is readable as a `loading` batch served from
//! its staged rows; a page beyond the arrived rows is the typed
//! not-yet-loaded answer; the batch is sealed exactly once as complete,
//! cancelled, failed or (after a restart) interrupted, keeping the arrived
//! rows. All products are synthetic.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadLayout, FrozenSearchBatch, FrozenSearchStats, SearchBatchProgressState,
    SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest,
    WorkbenchSelectionCommand, WorkbenchSelectionCommandKind,
};
use insar_store::{
    partial_search_stats, StoreBatchPageRequest, StoreError, StoreSearchBatchStagingRequest,
    TaskStore, PROGRESSIVE_ADMISSION_DELIVERY_KIND, PROGRESSIVE_PUBLICATION_ROWS,
};
use serde_json::Map;

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-store-progressive-{label}-{}-{nonce}.sqlite3",
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

const PAGE: u64 = PROGRESSIVE_PUBLICATION_ROWS;

fn candidate(item_id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "fixture.catalog".to_owned(),
        display_name: format!("display-{item_id}"),
        acquisition_datetime: Some("2026-11-03T00:00:00Z".to_owned()),
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

/// Items `start..end`, named so the frozen item-ID order is the index order.
fn candidates(start: usize, end: usize) -> Vec<SnapshotItem> {
    (start..end)
        .map(|index| candidate(&format!("item-{index:05}")))
        .collect()
}

fn batch(batch_id: &str, candidates: Vec<SnapshotItem>) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_owned(),
        provider_id: "fixture.catalog".to_owned(),
        created_at: "2026-11-03T00:00:00Z".to_owned(),
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

fn page(batch_id: &str, page: u64, page_size: u64) -> StoreBatchPageRequest {
    StoreBatchPageRequest {
        batch_id: batch_id.to_owned(),
        revision: 0,
        page,
        page_size,
    }
}

fn begin(store: &mut TaskStore, request_id: &str, batch_id: &str) {
    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: request_id.to_owned(),
            batch_header: batch(batch_id, Vec::new()),
            workbench: workbench(batch_id),
            max_candidates: 10_000,
        })
        .unwrap();
}

fn append(store: &mut TaskStore, request_id: &str, items: &[SnapshotItem]) {
    for chunk in items.chunks(128) {
        store
            .append_search_batch_staging_candidates(request_id, chunk)
            .unwrap();
    }
}

/// Stage `arrived` rows and progressively publish the request.
fn publish_loading(store: &mut TaskStore, request_id: &str, batch_id: &str, arrived: usize) {
    begin(store, request_id, batch_id);
    append(store, request_id, &candidates(0, arrived));
    assert_eq!(
        store
            .publish_search_batch_staging_progressive(request_id, None)
            .unwrap(),
        batch_id
    );
}

fn ids(items: &[SnapshotItem]) -> Vec<&str> {
    items.iter().map(|item| item.item_id.as_str()).collect()
}

#[test]
fn page_one_is_readable_while_loading() {
    let database = TempDatabase::new("page-one");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-loading", 300);

    let first = store
        .load_batch_page(&page("batch-loading", 0, PAGE))
        .unwrap();
    assert_eq!(first.state, SearchBatchProgressState::Loading);
    assert_eq!(first.items.len(), PAGE as usize, "page 1 is complete");
    assert_eq!(
        ids(&first.items),
        ids(&candidates(0, PAGE as usize)),
        "the loading view uses the frozen item-ID order of the seal"
    );
    assert_eq!(first.rows.len(), first.items.len());
    assert_eq!(first.loaded_count, 300);
    assert_eq!(first.total_count, 300);
    assert_eq!(first.remote_total, None, "an unknown total is not invented");
    // The batch is readable without a half-written frozen batch.
    assert_eq!(store.count_search_batches().unwrap(), 0);
    assert_eq!(
        store.load_batch_candidate_count("batch-loading").unwrap(),
        Some(300)
    );
    let progress = store
        .loading_search_batch_for_request("request")
        .unwrap()
        .unwrap();
    assert_eq!(progress.batch_id, "batch-loading");
    assert_eq!(progress.loaded_count, 300);
}

#[test]
fn page_beyond_arrived_rows_is_typed_not_yet_loaded() {
    let database = TempDatabase::new("not-yet");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-loading", 300);

    // 300 rows at page size 100: page index 2 is the partially arrived tail,
    // page index 3 starts exactly at the arrived count and has not arrived.
    let tail = store
        .load_batch_page(&page("batch-loading", 2, 100))
        .unwrap();
    assert_eq!(tail.items.len(), 100);
    for beyond in [3, 7] {
        match store
            .load_batch_page(&page("batch-loading", beyond, 100))
            .unwrap_err()
        {
            StoreError::PageNotYetLoaded {
                batch_id,
                loaded_count,
            } => {
                assert_eq!(batch_id, "batch-loading");
                assert_eq!(loaded_count, 300);
            }
            other => panic!("expected PageNotYetLoaded, got {other:?}"),
        }
    }
    let error = store
        .load_batch_page(&page("batch-loading", 3, 100))
        .unwrap_err();
    assert_eq!(error.code(), "V3_STORE_BATCH_PAGE_NOT_YET_LOADED");
}

#[test]
fn appended_rows_become_pageable_and_complete_seal_keeps_classic_semantics() {
    let database = TempDatabase::new("append");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-final", 300);
    assert!(store.load_batch_page(&page("batch-final", 3, 100)).is_err());
    append(&mut store, "request", &candidates(300, 420));
    let grown = store.load_batch_page(&page("batch-final", 3, 100)).unwrap();
    assert_eq!(grown.state, SearchBatchProgressState::Loading);
    assert_eq!(grown.loaded_count, 420);
    assert_eq!(ids(&grown.items), ids(&candidates(300, 400)));

    store
        .seal_search_batch_staging_streaming("request", Some(stats(420)), true)
        .unwrap();
    let sealed = store.load_batch_page(&page("batch-final", 3, 100)).unwrap();
    assert_eq!(sealed.state, SearchBatchProgressState::Complete);
    assert_eq!(sealed.loaded_count, 420);
    assert_eq!(
        sealed.items, grown.items,
        "the loading view equals the frozen page"
    );
    assert_eq!(
        sealed.remote_total,
        Some(420),
        "the final total is recorded"
    );
    assert_eq!(store.count_staging_rows().unwrap(), 0);
    // A sealed batch keeps the classic out-of-range semantics.
    assert!(matches!(
        store
            .load_batch_page(&page("batch-final", 9, 100))
            .unwrap_err(),
        StoreError::NotFound
    ));
    assert!(store
        .loading_search_batch_for_request("request")
        .unwrap()
        .is_none());
    let frozen = store.load_search_batch("batch-final").unwrap().unwrap();
    assert_eq!(frozen.candidates.len(), 420);
}

#[test]
fn cancelled_finalize_keeps_rows_marks_incomplete_and_is_idempotent() {
    let database = TempDatabase::new("cancel");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-cancel", 260);
    store
        .record_search_batch_remote_total("request", Some(900))
        .unwrap();
    let kept = partial_search_stats(260, Some(900), "TYPED_CATALOG");
    assert!(store
        .finalize_search_batch_staging("request", kept.clone(), SearchBatchProgressState::Cancelled)
        .unwrap());
    // A racing second finalize (search thread after the cancel command) is
    // a no-op, not an error and not a second seal.
    assert!(!store
        .finalize_search_batch_staging("request", kept.clone(), SearchBatchProgressState::Failed)
        .unwrap());

    let snapshot = store
        .load_batch_page(&page("batch-cancel", 0, 100))
        .unwrap();
    assert_eq!(snapshot.state, SearchBatchProgressState::Cancelled);
    assert_eq!(snapshot.loaded_count, 260);
    assert_eq!(snapshot.remote_total, Some(900));
    assert_eq!(store.count_staging_rows().unwrap(), 0);
    let frozen = store.load_search_batch("batch-cancel").unwrap().unwrap();
    assert_eq!(frozen.candidates.len(), 260);
    assert_eq!(frozen.search_stats, Some(kept));
    assert!(
        frozen.search_stats.as_ref().unwrap().truncated,
        "a cancelled search is possibly incomplete"
    );
    // Rows kept by a cancel are selectable like any sealed batch.
    assert_eq!(
        store.load_workbench().unwrap().unwrap().batch.batch_id,
        "batch-cancel"
    );
}

#[test]
fn early_finalize_rejects_complete_or_a_claim_of_completeness() {
    let database = TempDatabase::new("reject");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-reject", 260);
    assert!(store
        .finalize_search_batch_staging(
            "request",
            partial_search_stats(260, None, "TYPED_CATALOG"),
            SearchBatchProgressState::Complete,
        )
        .is_err());
    assert!(store
        .finalize_search_batch_staging("request", stats(260), SearchBatchProgressState::Failed)
        .is_err());
    assert_eq!(
        store
            .load_batch_page(&page("batch-reject", 0, 100))
            .unwrap()
            .state,
        SearchBatchProgressState::Loading
    );
}

#[test]
fn failed_finalize_keeps_rows() {
    let database = TempDatabase::new("fail");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-failed", 250);
    store
        .finalize_search_batch_staging(
            "request",
            partial_search_stats(250, None, "TYPED_CATALOG"),
            SearchBatchProgressState::Failed,
        )
        .unwrap();
    let snapshot = store
        .load_batch_page(&page("batch-failed", 2, 100))
        .unwrap();
    assert_eq!(snapshot.state, SearchBatchProgressState::Failed);
    assert_eq!(snapshot.loaded_count, 250);
    assert_eq!(snapshot.items.len(), 50);
    assert_eq!(store.count_staging_rows().unwrap(), 0);
}

#[test]
fn restart_turns_a_loading_batch_interrupted_and_keeps_the_workbench() {
    let database = TempDatabase::new("restart");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch: batch("batch-previous", candidates(0, 3)),
                workbench: workbench("batch-previous"),
                activate_workbench: true,
            })
            .unwrap();
        publish_loading(&mut store, "request", "batch-restart", 280);
        store
            .record_search_batch_remote_total("request", Some(5_000))
            .unwrap();
    }
    let mut store = TaskStore::open(database.path()).unwrap();
    assert_eq!(store.interrupt_loading_search_batches().unwrap(), 1);
    assert_eq!(store.interrupt_loading_search_batches().unwrap(), 0);

    let snapshot = store
        .load_batch_page(&page("batch-restart", 0, 100))
        .unwrap();
    assert_eq!(
        snapshot.state,
        SearchBatchProgressState::Interrupted,
        "a loading batch never silently reads as complete after a restart"
    );
    assert_eq!(snapshot.loaded_count, 280, "arrived rows are kept");
    assert_eq!(snapshot.remote_total, Some(5_000));
    assert_eq!(store.count_staging_rows().unwrap(), 0);
    let frozen = store.load_search_batch("batch-restart").unwrap().unwrap();
    assert!(frozen.search_stats.as_ref().unwrap().truncated);
    assert_eq!(
        frozen.search_stats.as_ref().unwrap().matched_count,
        Some(5_000)
    );
    assert_eq!(
        store.load_workbench().unwrap().unwrap().batch.batch_id,
        "batch-previous",
        "a restart does not switch the active workbench"
    );
}

#[test]
fn publication_needs_one_full_page_and_is_unique() {
    let database = TempDatabase::new("threshold");
    let mut store = TaskStore::open(database.path()).unwrap();
    begin(&mut store, "request", "batch-small");
    append(&mut store, "request", &candidates(0, PAGE as usize - 1));
    assert!(store
        .publish_search_batch_staging_progressive("request", None)
        .is_err());
    assert!(matches!(
        store
            .load_batch_page(&page("batch-small", 0, 24))
            .unwrap_err(),
        StoreError::NotFound
    ));
    append(
        &mut store,
        "request",
        &candidates(PAGE as usize - 1, PAGE as usize),
    );
    store
        .publish_search_batch_staging_progressive("request", Some(7))
        .unwrap();
    assert!(matches!(
        store
            .publish_search_batch_staging_progressive("request", None)
            .unwrap_err(),
        StoreError::AlreadyExists
    ));
    assert_eq!(
        store
            .load_batch_page(&page("batch-small", 0, 24))
            .unwrap()
            .remote_total,
        Some(7)
    );
}

#[test]
fn a_loading_batch_is_browse_only_until_sealed() {
    let database = TempDatabase::new("browse-only");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-loading", 250);
    for error in [
        store.load_batch_header("batch-loading").unwrap_err(),
        store
            .load_batch_selection("batch-loading", &["item-00000".to_owned()])
            .unwrap_err(),
        store
            .load_batch_candidate_items("batch-loading", &["item-00000".to_owned()])
            .unwrap_err(),
    ] {
        assert!(
            matches!(&error, StoreError::BatchStillLoading { batch_id } if batch_id == "batch-loading"),
            "{error:?}"
        );
        assert_eq!(error.code(), "V3_STORE_BATCH_STILL_LOADING");
    }
    // An unknown batch keeps its classic answer.
    assert!(store.load_batch_header("batch-unknown").unwrap().is_none());
}

#[test]
fn abort_withdraws_an_unsealed_publication() {
    let database = TempDatabase::new("abort");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-abort", 250);
    store.abort_search_batch_staging("request").unwrap();
    assert!(store
        .search_batch_progress("batch-abort")
        .unwrap()
        .is_none());
    assert!(matches!(
        store
            .load_batch_page(&page("batch-abort", 0, 24))
            .unwrap_err(),
        StoreError::NotFound
    ));
    assert_eq!(store.interrupt_loading_search_batches().unwrap(), 0);
}

#[test]
fn a_batch_frozen_without_progressive_publication_reads_complete() {
    let database = TempDatabase::new("classic");
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch("batch-classic", candidates(0, 40)),
            workbench: workbench("batch-classic"),
            activate_workbench: true,
        })
        .unwrap();
    let snapshot = store
        .load_batch_page(&page("batch-classic", 0, 24))
        .unwrap();
    assert_eq!(snapshot.state, SearchBatchProgressState::Complete);
    assert_eq!(snapshot.loaded_count, 40);
    assert_eq!(snapshot.remote_total, None);
    assert!(store
        .search_batch_progress("batch-classic")
        .unwrap()
        .is_none());
}

// #22 B slice 5 (Owner 2026-11-03, option A): arrived rows of a loading
// batch can be downloaded row by row. Admission freezes exactly the staged
// rows into a small derived batch linked back to the loading batch; a row
// that has not arrived freezes nothing; select-all waits for the seal.

fn admission_task(task_id: &str, batch_id: &str, item_ids: &[&str]) -> StoreCreateTaskRequest {
    let ids = strings(item_ids);
    StoreCreateTaskRequest {
        task_id: task_id.to_owned(),
        batch_id: batch_id.to_owned(),
        selected_item_ids: ids.clone(),
        checked_item_ids: ids,
        page_index: 0,
        page_size: 100,
        output_root: PathBuf::from("fixture-output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-11-03T00:05:00Z".to_owned(),
    }
}

fn strings(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn row_task(page: &insar_store::StoreBatchPage, id: &str) -> Option<String> {
    page.rows
        .iter()
        .find(|row| row.item_id == id)
        .unwrap()
        .task_id
        .clone()
}

#[test]
fn arrived_rows_of_a_loading_batch_are_admitted_through_a_linked_batch() {
    let database = TempDatabase::new("admit-arrived");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-loading", 300);

    let admission = store
        .loading_admission_batch(
            "batch-loading",
            &strings(&["item-00003", "item-00001"]),
            "admission-1",
            "2026-11-03T00:05:00Z",
        )
        .unwrap()
        .expect("a loading batch yields an admission batch");
    assert_eq!(admission.batch_id, "admission-1");
    assert_eq!(
        admission.query["derived_from_batch_id"],
        serde_json::json!("batch-loading")
    );
    assert_eq!(
        admission.query["delivery_kind"],
        serde_json::json!(PROGRESSIVE_ADMISSION_DELIVERY_KIND)
    );
    assert_eq!(ids(&admission.candidates), vec!["item-00001", "item-00003"]);
    for item in &admission.candidates {
        assert_eq!(
            item.metadata["source_item_id"],
            serde_json::json!(item.item_id)
        );
        assert_eq!(
            item.metadata["note"],
            candidate(&item.item_id).metadata["note"]
        );
    }

    let task = store
        .create_task_from_loading_admission(
            "batch-loading",
            &admission,
            &admission_task("task-1", "admission-1", &["item-00001", "item-00003"]),
            None,
        )
        .unwrap();
    assert_eq!(task.snapshot.batch_id, "admission-1");

    // The loading batch itself stays loading and browse-only.
    let loading = store
        .load_batch_page(&page("batch-loading", 0, 24))
        .unwrap();
    assert_eq!(loading.state, SearchBatchProgressState::Loading);
    assert_eq!(row_task(&loading, "item-00001").as_deref(), Some("task-1"));
    assert_eq!(row_task(&loading, "item-00003").as_deref(), Some("task-1"));
    assert_eq!(row_task(&loading, "item-00002"), None);
    assert!(matches!(
        store.load_batch_header("batch-loading").unwrap_err(),
        StoreError::BatchStillLoading { .. }
    ));
    assert_eq!(
        store
            .active_task_for_products("batch-loading", &strings(&["item-00003"]))
            .unwrap()
            .map(|task| task.snapshot.task_id),
        Some("task-1".to_owned()),
        "a second admission of the same row reuses the live task"
    );

    // After the seal the same rows keep their task and still deduplicate.
    append(&mut store, "request", &candidates(300, 320));
    store
        .seal_search_batch_staging_streaming("request", Some(stats(320)), true)
        .unwrap();
    let sealed = store
        .load_batch_page(&page("batch-loading", 0, 24))
        .unwrap();
    assert_eq!(sealed.state, SearchBatchProgressState::Complete);
    assert_eq!(row_task(&sealed, "item-00001").as_deref(), Some("task-1"));
    assert_eq!(
        store
            .active_task_for_products("batch-loading", &strings(&["item-00001"]))
            .unwrap()
            .map(|task| task.snapshot.task_id),
        Some("task-1".to_owned())
    );
}

#[test]
fn a_row_that_has_not_arrived_freezes_nothing() {
    let database = TempDatabase::new("admit-unarrived");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-loading", 250);

    let error = store
        .loading_admission_batch(
            "batch-loading",
            &strings(&["item-00001", "item-00400"]),
            "admission-x",
            "2026-11-03T00:05:00Z",
        )
        .unwrap_err();
    assert!(
        matches!(&error, StoreError::RowsNotYetLoaded { batch_id } if batch_id == "batch-loading"),
        "{error:?}"
    );
    assert_eq!(error.code(), "V3_STORE_BATCH_ROWS_NOT_YET_LOADED");

    // A forged admission batch naming an unstaged row is refused in the
    // write transaction: no derived batch and no task row.
    let mut forged = store
        .loading_admission_batch(
            "batch-loading",
            &strings(&["item-00001"]),
            "admission-forged",
            "2026-11-03T00:05:00Z",
        )
        .unwrap()
        .unwrap();
    let mut extra = candidate("item-00400");
    extra
        .metadata
        .insert("source_item_id".to_owned(), serde_json::json!("item-00400"));
    forged.candidates.push(extra);
    let error = store
        .create_task_from_loading_admission(
            "batch-loading",
            &forged,
            &admission_task(
                "task-forged",
                "admission-forged",
                &["item-00001", "item-00400"],
            ),
            None,
        )
        .unwrap_err();
    assert_eq!(error.code(), "V3_STORE_BATCH_ROWS_NOT_YET_LOADED");
    // An altered row is refused as well.
    let mut altered = store
        .loading_admission_batch(
            "batch-loading",
            &strings(&["item-00002"]),
            "admission-altered",
            "2026-11-03T00:05:00Z",
        )
        .unwrap()
        .unwrap();
    altered.candidates[0].display_name = "tampered".to_owned();
    assert!(store
        .create_task_from_loading_admission(
            "batch-loading",
            &altered,
            &admission_task("task-altered", "admission-altered", &["item-00002"]),
            None,
        )
        .is_err());
    assert!(store
        .load_search_batch("admission-forged")
        .unwrap()
        .is_none());
    assert!(store
        .load_search_batch("admission-altered")
        .unwrap()
        .is_none());
    assert!(store.load_task("task-forged").unwrap().is_none());
    assert!(store.load_task("task-altered").unwrap().is_none());
}

#[test]
fn a_sealed_batch_has_no_loading_admission() {
    let database = TempDatabase::new("admit-classic");
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: batch("batch-classic", candidates(0, 40)),
            workbench: workbench("batch-classic"),
            activate_workbench: true,
        })
        .unwrap();
    for batch_id in ["batch-classic", "batch-unknown"] {
        assert!(store
            .loading_admission_batch(
                batch_id,
                &strings(&["item-00001"]),
                "admission-classic",
                "2026-11-03T00:05:00Z",
            )
            .unwrap()
            .is_none());
    }
}

#[test]
fn select_all_and_stored_selection_wait_for_the_seal() {
    let database = TempDatabase::new("select-all-loading");
    let mut store = TaskStore::open(database.path()).unwrap();
    publish_loading(&mut store, "request", "batch-loading", 250);
    for (kind, item_ids) in [
        (WorkbenchSelectionCommandKind::SelectAll, Vec::new()),
        (
            WorkbenchSelectionCommandKind::CheckItems,
            strings(&["item-00001"]),
        ),
    ] {
        let error = store
            .apply_workbench_selection(&WorkbenchSelectionCommand {
                batch_id: "batch-loading".to_owned(),
                expected_revision: 0,
                kind,
                item_ids,
                page_index: 0,
                page_size: 24,
                updated_at: "2026-11-03T00:05:00Z".to_owned(),
            })
            .unwrap_err();
        assert_eq!(error.code(), "V3_STORE_BATCH_STILL_LOADING", "{error:?}");
    }
}
