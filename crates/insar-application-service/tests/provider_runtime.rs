use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationSarSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, ContractError, DownloadItemProgress,
    DownloadItemResult, DownloadItemStatus, DownloadLayout, DownloadOutcome, ProviderResult,
    SarRemoteSearchRequest, SarRemoteSearchResponse, SarSourceSearchKind, SearchWorkbenchState,
    StoreCreateTaskRequest, StoreUpdateWorkbenchRequest,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::LiveAsfSearch;
use serde_json::json;

mod common;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-provider-runtime-{label}-{}-{id}.sqlite3",
            std::process::id()
        )))
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

#[derive(Default)]
struct FixtureSearch {
    entered: AtomicBool,
    block: AtomicBool,
}

impl FixtureSearch {
    fn blocking() -> Self {
        Self {
            entered: AtomicBool::new(false),
            block: AtomicBool::new(true),
        }
    }
}

impl LiveAsfSearch for FixtureSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Default::default()).unwrap())
    }

    fn search_sar(
        &self,
        _request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.entered.store(true, Ordering::Release);
        while self.block.load(Ordering::Acquire) {
            if cancelled() {
                return Err(ContractError::new(
                    "DL001",
                    "fixture search cancelled",
                    Default::default(),
                )
                .unwrap());
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(serde_json::from_value(json!({
            "products": [{
                "product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-FIXTURE",
                "provider_product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-FIXTURE",
                "provider_product_id": "S1-FIXTURE",
                "product_id": "S1-FIXTURE",
                "provider": "asf.catalog",
                "dataset": "SENTINEL-1",
                "product_family": "SLC",
                "processing_level": "SLC",
                "platform": "S1A",
                "acquisition_start": "2026-07-01T00:00:00Z",
                "file_name": "S1-FIXTURE.zip",
                "file_size_remote": 4,
                "download_url": "http://127.0.0.1:18080/S1-FIXTURE.zip"
            }],
            "errors": [],
            "stats": {
                "requested_limit": 1,
                "query_limit": 1,
                "candidate_count": 1,
                "returned_count": 1,
                "source": "fixture"
            }
        }))
        .unwrap())
    }
}

#[derive(Default)]
struct FixtureWorker {
    starts: Mutex<Vec<(String, String)>>,
}

impl DownloadWorker for FixtureWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.starts
            .lock()
            .unwrap()
            .push((work.task.task_id.clone(), work.item.item_id.clone()));
        progress(DownloadItemProgress {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            status: DownloadItemStatus::Active,
            bytes_completed: 4,
            total_bytes: Some(4),
            bytes_per_second: 4.0,
            attempt: 1,
            updated_at: "2026-07-25T00:00:02Z".to_owned(),
        });
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: if control.is_cancelled() {
                DownloadOutcome::Cancelled
            } else {
                DownloadOutcome::Completed
            },
            destination: work.task.output_root.join("S1-FIXTURE.zip"),
            bytes_written: 4,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-25T00:00:03Z".to_owned(),
        }
    }
}

fn aoi() -> serde_json::Map<String, serde_json::Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn bind_aoi(service: &ApplicationService, label: &str) {
    let region_id = common::ensure_fixture_region(service, label);
    let request = ApplicationBindRegionAoiRequest {
        request_id: format!("{label}-bind-aoi"),
        region_id,
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: aoi(),
        bbox: vec![100.0, 30.0, 101.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "Provider fixture AOI".to_owned(),
        bound_at: "2026-07-25T00:00:00Z".to_owned(),
    };
    service.bind_region_aoi(&request).unwrap();
}

fn search_request(request_id: &str) -> ApplicationSarSearchAndFreezeRequest {
    let aoi_geojson = aoi();
    ApplicationSarSearchAndFreezeRequest {
        request_id: request_id.to_owned(),
        search: serde_json::from_value(json!({
            "query": {
                "provider": "asf",
                "dataset": "SENTINEL-1",
                "aoi_geojson": aoi_geojson,
                "max_results": 1
            }
        }))
        .unwrap(),
        batch_id: "m1-fixture-batch".to_owned(),
        created_at: "2026-07-25T00:00:00Z".to_owned(),
        aoi_name: "Fixture AOI".to_owned(),
        aoi_geojson: Some(aoi_geojson),
        page_size: 24,
    }
}

#[test]
fn fixture_business_runtime_freezes_search_then_schedules_the_same_store_task() {
    let database = TempStore::new("freeze");
    let search = Arc::new(FixtureSearch::default());
    let worker = Arc::new(FixtureWorker::default());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        search.clone(),
        worker.clone(),
    );
    bind_aoi(&service, "search-freeze");

    let capabilities = service.provider_capabilities().unwrap();
    assert!(!capabilities.datasets.is_empty());
    let alos1 = capabilities
        .sources
        .iter()
        .find(|source| source.selector == "ALOS PALSAR")
        .expect("legacy ALOS-1 source is projected");
    assert_eq!(alos1.display_name, "ALOS PALSAR");
    assert_eq!(alos1.search_kind, SarSourceSearchKind::LegacyAsfSearch);
    assert_eq!(alos1.filters["legacy_dataset"], "ALOS PALSAR");
    assert_eq!(
        alos1.filters["dataset_capabilities"]["dataset"],
        "ALOS PALSAR"
    );
    assert_eq!(
        alos1.filters["dataset_capabilities"]["platforms"][0],
        "ALOS"
    );
    let alos2 = capabilities
        .sources
        .iter()
        .find(|source| source.selector == "ALOS-2")
        .expect("typed ALOS-2 source is projected");
    assert_eq!(alos2.search_kind, SarSourceSearchKind::TypedCatalogProfile);
    assert_eq!(
        alos2.logical_dataset.as_deref(),
        Some("sar.alos-2.palsar-2.scansar.l1.1.v1")
    );
    assert_eq!(alos2.provider_id.as_deref(), Some("asf.nasa.earthdata"));
    assert_eq!(
        alos2.native_collection_id.as_deref(),
        Some("C3315903479-ASF")
    );
    assert_eq!(
        alos2.collection_profile_id.as_deref(),
        Some("asf.alos-2.palsar-2.scansar.l1.1.v1")
    );
    assert!(!alos2.available);
    assert_eq!(
        alos2.disabled_reason.as_deref(),
        Some("CATALOGUE_ROUTE_UNAVAILABLE")
    );
    let nisar = capabilities
        .sources
        .iter()
        .find(|source| source.selector == "NISAR")
        .expect("typed NISAR source is projected");
    assert_eq!(nisar.search_kind, SarSourceSearchKind::TypedCatalogProfile);
    assert_eq!(
        nisar.logical_dataset.as_deref(),
        Some("sar.nisar.l.rslc.provisional.v1")
    );
    assert_eq!(nisar.provider_id.as_deref(), Some("asf.nasa.earthdata"));
    assert_eq!(
        nisar.native_collection_id.as_deref(),
        Some("C2853145197-ASF")
    );
    assert_eq!(
        nisar.collection_profile_id.as_deref(),
        Some("asf.nisar.l.rslc.provisional.v1")
    );
    assert!(!nisar.available);
    assert_eq!(
        nisar.disabled_reason.as_deref(),
        Some("CATALOGUE_ROUTE_UNAVAILABLE")
    );
    let frozen = service
        .sar_search_and_freeze_legacy(&search_request("search-freeze"))
        .unwrap();
    let batch = frozen.restore.workbench_batch.unwrap();
    assert_eq!(batch.batch_id, "m1-fixture-batch");
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(
        batch.candidates[0].item_id,
        "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-FIXTURE"
    );
    assert_eq!(
        batch.candidates[0].payload["download_url"],
        "http://127.0.0.1:18080/S1-FIXTURE.zip"
    );
    assert!(!batch.candidates[0].metadata.contains_key("native_metadata"));

    let checked_id = batch.candidates[0].item_id.clone();
    let selection = service
        .update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: vec![checked_id.clone()],
                page_index: 1,
                page_size: 24,
                revision: 1,
            },
            expected_revision: 0,
            updated_at: "2026-07-25T00:00:01Z".to_owned(),
        })
        .unwrap();
    assert_eq!(selection.core.workbench.as_ref().unwrap().revision, 1);

    service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: "m1-fixture-task".to_owned(),
            batch_id: batch.batch_id,
            selected_item_ids: vec![checked_id],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: database.0.with_extension("output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-25T00:00:01Z".to_owned(),
        })
        .unwrap();

    let deadline = SystemTime::now() + Duration::from_secs(2);
    while worker.starts.lock().unwrap().is_empty() && SystemTime::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(worker.starts.lock().unwrap().len(), 1);
    let deadline = SystemTime::now() + Duration::from_secs(2);
    while service
        .download_snapshot("m1-fixture-task")
        .map(|snapshot| snapshot.results.len())
        .unwrap_or_default()
        != 1
        && SystemTime::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
    let snapshot = service.download_snapshot("m1-fixture-task").unwrap();
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.results[0].outcome, DownloadOutcome::Completed);
    service.shutdown().unwrap();
}

#[test]
fn cancelled_fixture_search_never_persists_a_partial_batch() {
    let database = TempStore::new("cancel");
    let search = Arc::new(FixtureSearch::blocking());
    let service = Arc::new(ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        search.clone(),
        Arc::new(FixtureWorker::default()),
    ));
    bind_aoi(service.as_ref(), "cancel-search");
    let operation = search_request("cancel-search");
    let thread_service = Arc::clone(&service);
    let handle = thread::spawn(move || thread_service.sar_search_and_freeze_legacy(&operation));
    let deadline = SystemTime::now() + Duration::from_secs(2);
    while !search.entered.load(Ordering::Acquire) && SystemTime::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(search.entered.load(Ordering::Acquire));
    assert!(service.cancel_request("cancel-search").unwrap());
    let error = handle.join().unwrap().unwrap_err();
    assert_eq!(error.code(), "DL001");
    let snapshot = service.snapshot(true).unwrap();
    assert!(snapshot.restore.workbench_batch.is_none());
    assert!(snapshot.restore.tasks.is_empty());
    service.shutdown().unwrap();
}
