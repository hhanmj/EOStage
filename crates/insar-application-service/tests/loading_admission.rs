//! #22 B slice 5 (Owner decision 2026-11-03, option A): while a search batch
//! is loading, rows that have arrived can be downloaded row by row (and a
//! checked set of arrived rows together) through the unified task entry; a
//! row that has not arrived is refused and freezes nothing; the same row is
//! never admitted twice, before or after the batch seals. All products are
//! synthetic.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, StoreBatchPageRequest, UnifiedCreateTaskOptions,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, DownloadItemProgress, DownloadItemResult,
    DownloadLayout, DownloadOutcome, FrozenSearchBatch, FrozenSearchStats, ProviderResult,
    SearchBatchProgressState, SearchWorkbenchState, SnapshotItem, TaskLifecycleStatus,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::LiveAsfSearch;
use insar_store::{StoreSearchBatchStagingRequest, TaskStore};
use serde_json::{json, Map};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct NoSearch;
impl LiveAsfSearch for NoSearch {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("admission fixture must never search")
    }
}

/// Holds every scheduled item until shutdown cancels it.
struct WaitForCancelWorker;
impl DownloadWorker for WaitForCancelWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        while !control.is_cancelled() {
            thread::sleep(Duration::from_millis(5));
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root.join(work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-11-03T02:00:00Z".to_owned(),
        }
    }
}

struct TempDatabase(PathBuf);
impl TempDatabase {
    fn new(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "insar-application-loading-admission-{label}-{}-{}.sqlite3",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

const PROVIDER: &str = "microsoft.planetary-computer";

fn item(index: usize) -> SnapshotItem {
    SnapshotItem {
        item_id: format!("item-{index:05}"),
        provider_id: PROVIDER.to_owned(),
        display_name: format!("Synthetic scene {index}"),
        acquisition_datetime: Some("2026-11-03T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn header(batch_id: &str) -> FrozenSearchBatch {
    FrozenSearchBatch {
        schema_version: 1,
        batch_id: batch_id.to_owned(),
        provider_id: PROVIDER.to_owned(),
        created_at: "2026-11-03T00:00:00Z".to_owned(),
        aoi_name: "Synthetic AOI".to_owned(),
        aoi_geojson: None,
        query: json!({
            "logical_dataset": "optical.landsat.c2.l2",
            "provider_id": PROVIDER,
            "native_collection_id": "landsat-c2-l2"
        })
        .as_object()
        .unwrap()
        .clone(),
        search_stats: None,
        candidates: Vec::new(),
    }
}

/// Stage `arrived` rows of a request and publish it as a loading batch, as
/// the streaming search does once page 1 has arrived.
fn publish_loading(database: &PathBuf, request_id: &str, batch_id: &str, arrived: usize) {
    let mut store = TaskStore::open(database).unwrap();
    store
        .begin_search_batch_staging(&StoreSearchBatchStagingRequest {
            request_id: request_id.to_owned(),
            batch_header: header(batch_id),
            workbench: SearchWorkbenchState {
                batch_id: batch_id.to_owned(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
            max_candidates: 10_000,
        })
        .unwrap();
    let items: Vec<SnapshotItem> = (0..arrived).map(item).collect();
    for chunk in items.chunks(128) {
        store
            .append_search_batch_staging_candidates(request_id, chunk)
            .unwrap();
    }
    store
        .publish_search_batch_staging_progressive(request_id, Some(900))
        .unwrap();
}

fn service(database: &PathBuf) -> ApplicationService {
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(database),
        Arc::new(NoSearch),
        Arc::new(WaitForCancelWorker),
    );
    // Start the runtime first: service start seals any loading batch as
    // interrupted, so the loading batch below belongs to this process.
    service.health().unwrap();
    service
}

fn options() -> UnifiedCreateTaskOptions {
    UnifiedCreateTaskOptions {
        output_root: PathBuf::from("fixture-output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        include_precise_orbit: None,
    }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn page(service: &ApplicationService, batch_id: &str) -> insar_application_service::StoreBatchPage {
    service
        .batch_page(&StoreBatchPageRequest {
            batch_id: batch_id.to_owned(),
            revision: 0,
            page: 0,
            page_size: 50,
        })
        .unwrap()
}

fn row_task(page: &insar_application_service::StoreBatchPage, item_id: &str) -> Option<String> {
    page.rows
        .iter()
        .find(|row| row.item_id == item_id)
        .and_then(|row| row.task_id.clone())
}

#[test]
fn an_arrived_row_of_a_loading_batch_downloads_and_is_never_admitted_twice() {
    let database = TempDatabase::new("row");
    let service = service(&database.0);
    publish_loading(&database.0, "request-row", "batch-row", 300);

    let first = service
        .create_task("batch-row".to_owned(), 0, ids(&["item-00007"]), options())
        .expect("an arrived row is admitted while the batch loads");
    assert!(first.created);
    assert_eq!(first.status, TaskLifecycleStatus::Queued);

    let loading = page(&service, "batch-row");
    assert_eq!(loading.state, SearchBatchProgressState::Loading);
    assert_eq!(
        row_task(&loading, "item-00007"),
        Some(first.task_id.clone())
    );
    assert_eq!(row_task(&loading, "item-00008"), None);

    let again = service
        .create_task("batch-row".to_owned(), 0, ids(&["item-00007"]), options())
        .expect("the same row is reused");
    assert!(!again.created);
    assert_eq!(again.task_id, first.task_id);

    // The batch seals; the row keeps its task and is still not admitted twice.
    {
        let mut store = TaskStore::open(&database.0).unwrap();
        let more: Vec<SnapshotItem> = (300..320).map(item).collect();
        store
            .append_search_batch_staging_candidates("request-row", &more)
            .unwrap();
        store
            .seal_search_batch_staging_streaming(
                "request-row",
                Some(FrozenSearchStats {
                    matched_count: Some(320),
                    fetched_record_count: Some(320),
                    mapped_record_count: Some(320),
                    provider_returned_count: 320,
                    frozen_candidate_count: 320,
                    truncated: false,
                    source_attempts: vec![],
                    source: "TYPED_CATALOG".to_owned(),
                }),
                true,
            )
            .unwrap();
    }
    let sealed = page(&service, "batch-row");
    assert_eq!(sealed.state, SearchBatchProgressState::Complete);
    assert_eq!(row_task(&sealed, "item-00007"), Some(first.task_id.clone()));
    let after_seal = service
        .create_task("batch-row".to_owned(), 0, ids(&["item-00007"]), options())
        .expect("the sealed batch reuses the admission task");
    assert!(!after_seal.created);
    assert_eq!(after_seal.task_id, first.task_id);
    service.shutdown().unwrap();
}

#[test]
fn checked_arrived_rows_are_admitted_together() {
    let database = TempDatabase::new("checked");
    let service = service(&database.0);
    publish_loading(&database.0, "request-checked", "batch-checked", 260);
    let task = service
        .create_task(
            "batch-checked".to_owned(),
            0,
            ids(&["item-00259", "item-00002"]),
            options(),
        )
        .expect("checked arrived rows are admitted");
    assert!(task.created);
    let loading = page(&service, "batch-checked");
    assert_eq!(row_task(&loading, "item-00002"), Some(task.task_id.clone()));
    service.shutdown().unwrap();
}

#[test]
fn a_row_that_has_not_arrived_is_refused_and_creates_nothing() {
    let database = TempDatabase::new("unarrived");
    let service = service(&database.0);
    publish_loading(&database.0, "request-late", "batch-late", 250);
    let error = service
        .create_task(
            "batch-late".to_owned(),
            0,
            ids(&["item-00001", "item-00600"]),
            options(),
        )
        .expect_err("a row that has not arrived is never frozen");
    assert_eq!(error.code, "V3_STORE_BATCH_ROWS_NOT_YET_LOADED");
    let loading = page(&service, "batch-late");
    assert!(loading.rows.iter().all(|row| row.task_id.is_none()));
    service.shutdown().unwrap();
    let store = TaskStore::open(&database.0).unwrap();
    assert!(store
        .active_task_for_products("batch-late", &ids(&["item-00001"]))
        .unwrap()
        .is_none());
}
