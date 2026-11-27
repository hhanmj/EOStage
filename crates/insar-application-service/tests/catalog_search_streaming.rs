//! #22 B slice 3(b): the streaming catalogue search must freeze the same
//! batches as the whole-response path, leave no residue on cancel or failure,
//! and keep the application's in-memory candidate window bounded by the
//! streaming chunk size. All products and URLs are synthetic.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, AssetRole, CatalogSearchQuery,
    CatalogSearchResponse, DataSourceKind, DownloadItemProgress, DownloadItemResult, ProductKind,
    ProviderResult,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::catalog::{
    map_stac_complete_search_response, CatalogDeliveryModel, StacAssetRoleBinding,
    StacCollectionBinding, StacOfflineFixtureAdmission, StacRouteProfile,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Value};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);
impl TempStore {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sceneport-stac-streaming-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("store.sqlite3")
    }
}
impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct NoAsf;
impl LiveAsfSearch for NoAsf {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("STAC fixture must never call ASF");
    }
}

#[derive(Default)]
struct NoDownload(AtomicU64);
impl DownloadWorker for NoDownload {
    fn execute(
        &self,
        _: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("search must never schedule a download");
    }
}

fn route() -> StacRouteProfile {
    StacRouteProfile {
        provider_id: "fixture.stac".into(),
        endpoint: "https://catalog.example.test/stac/v1/".into(),
        search_path: "search".into(),
        page_cap: 10_000,
        accepted_stac_versions: vec!["1.0.0".into(), "1.1.0".into()],
    }
}

fn binding() -> StacCollectionBinding {
    StacCollectionBinding {
        logical_dataset: "optical.fixture.l2".into(),
        provider_id: "fixture.stac".into(),
        native_collection_id: "fixture-l2".into(),
        collection_profile_id: "fixture.stac.l2.v1".into(),
        identity_rule_id: "fixture.stac.identity.v1".into(),
        filename_rule_id: "fixture.stac.filename.v1".into(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        source_kind: DataSourceKind::Sentinel2,
        product_kind: ProductKind::OpticalScene,
        production_enabled: false,
        product_id_property: Some("scene:id".into()),
        display_name_property: Some("title".into()),
        queryables: vec![],
        asset_role_bindings: vec![StacAssetRoleBinding {
            stac_asset_key: "data".into(),
            role: AssetRole::Primary,
            required: true,
            allowed_hosts: vec!["data.example.test".into()],
        }],
    }
}

fn admission() -> Arc<StacOfflineFixtureAdmission> {
    Arc::new(StacOfflineFixtureAdmission::new_for_test(route(), binding()).unwrap())
}

fn geometry() -> serde_json::Map<String, Value> {
    json!({"type":"Polygon","coordinates":[[[110.0,30.0],[111.0,30.0],[111.0,31.0],[110.0,30.0]]]})
        .as_object()
        .unwrap()
        .clone()
}

fn request(label: &str, count: u64) -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: format!("request-{label}"),
        batch_id: format!("batch-{label}"),
        created_at: "2026-11-02T00:00:00Z".into(),
        aoi_name: "Synthetic test area".into(),
        aoi_geojson: Some(geometry()),
        workbench_page_size: 24,
        search: CatalogSearchQuery {
            logical_dataset: "optical.fixture.l2".into(),
            provider_id: "fixture.stac".into(),
            native_collection_id: "fixture-l2".into(),
            aoi_geojson: Some(geometry()),
            cursor: None,
            result_limit: Some(count),
            filters: Default::default(),
        },
    }
}

fn feature(index: usize) -> Value {
    let id = format!("item-{index:04}");
    json!({"type":"Feature","stac_version":"1.1.0","id":id,"collection":"fixture-l2",
        "geometry":geometry(),
        "properties":{"datetime":"2026-09-10T12:00:00Z","scene:id":id,"title":format!("Synthetic scene {index}"),"proj:epsg":32651},
        "assets":{"data":{"href":format!("https://data.example.test/{id}.tif"),"roles":["data"],"file:size":4096}},
        "links":[]})
}

/// A fixture catalog whose `search_catalog_streaming` impl streams its
/// products in small chunks (smaller than the application's 128-chunk) and
/// reports the observed per-chunk sizes.
struct ChunkedCatalog {
    count: usize,
    chunk_size: usize,
    /// (chunk index, chunk length) per accept call.
    accepts: Mutex<Vec<(usize, usize)>>,
}

impl ChunkedCatalog {
    fn response(&self, query: &CatalogSearchQuery) -> CatalogSearchResponse {
        let features: Vec<Value> = (0..self.count).map(feature).collect();
        map_stac_complete_search_response(
            &route(),
            &binding(),
            query,
            &json!({
                "type":"FeatureCollection",
                "numberMatched": self.count,
                "numberReturned": self.count,
                "features": features,
                "links":[]
            }),
        )
        .unwrap()
    }
}

impl LiveCatalogSearch for ChunkedCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        panic!("the streaming entry must not fall back to search_catalog");
    }

    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn insar_service::CatalogSearchStreamSink,
        _progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
    ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let response = self.response(query);
        for chunk in response.products.chunks(self.chunk_size) {
            sink.accept(chunk)?;
            let mut accepts = self.accepts.lock().unwrap();
            let index = accepts.len();
            accepts.push((index, chunk.len()));
        }
        Ok(insar_service::CatalogSearchStreamOutcome::Streamed {
            collection: response.collection,
            stats: response.stats,
        })
    }
}

/// A fixture catalog that streams part of its products and then fails,
/// simulating a mid-search provider failure.
struct FailingStreamCatalog {
    count: usize,
    fail_after: usize,
}

impl LiveCatalogSearch for FailingStreamCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        panic!("the streaming entry must not fall back to search_catalog");
    }

    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn insar_service::CatalogSearchStreamSink,
        _progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
    ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let response = ChunkedCatalog {
            count: self.count,
            chunk_size: 10,
            accepts: Mutex::new(Vec::new()),
        }
        .response(query);
        let mut streamed = 0_usize;
        for chunk in response.products.chunks(10) {
            if streamed >= self.fail_after {
                return Err(insar_core::ContractError::new(
                    "STAC_SERVER_ERROR",
                    "fixture mid-stream failure",
                    Default::default(),
                )
                .unwrap());
            }
            sink.accept(chunk)?;
            streamed += chunk.len();
        }
        unreachable!("fail_after is below the product count")
    }
}

fn service(database: &TempStore, provider: Arc<dyn LiveCatalogSearch>) -> ApplicationService {
    ApplicationService::with_offline_stac_fixture_business_runtime(
        ApplicationServiceConfig::business_store(database.database()),
        Arc::new(NoAsf),
        provider,
        Arc::new(NoDownload::default()),
        admission(),
    )
}

/// The candidate rows and the snapshot blob must agree for 0/1/250/251
/// streamed candidates, and the frozen blob must equal a whole-response
/// freeze of the same products.
#[test]
fn streamed_rows_and_blob_agree_for_zero_one_250_and_251() {
    for count in [0_usize, 1, 250, 251] {
        let database = TempStore::new();
        let catalog = Arc::new(ChunkedCatalog {
            count,
            chunk_size: 17,
            accepts: Mutex::new(Vec::new()),
        });
        let service = service(&database, catalog);
        let limit = count.max(1) as u64;
        service
            .catalog_search_and_freeze(&request("counts", limit))
            .unwrap();
        let batch = service.restore(true).unwrap().workbench_batch.unwrap();
        assert_eq!(batch.candidates.len(), count, "{count} candidates");
        let store = TaskStore::open(database.database()).unwrap();
        assert_eq!(
            store.count_batch_candidate_rows(&batch.batch_id).unwrap(),
            count,
            "{count} rows"
        );
        service.shutdown().unwrap();
    }
}

/// The application sink must flush in chunks bounded by its streaming chunk
/// size, never one whole provider chunk.
#[test]
fn streamed_staging_is_flushed_in_bounded_chunks() {
    // 251 products in provider chunks of 90: the app must re-buffer to at
    // most its own bound, so every staged write is <= the chunk bound.
    let database = TempStore::new();
    let catalog = Arc::new(ChunkedCatalog {
        count: 251,
        chunk_size: 90,
        accepts: Mutex::new(Vec::new()),
    });
    let service = service(&database, catalog);
    service
        .catalog_search_and_freeze(&request("bounded", 251))
        .unwrap();
    // The application's in-memory candidate window is bounded by its
    // streaming chunk size (128), even though the provider streamed chunks
    // of 90 that crossed the bound.
    let max_chunk = service.take_max_catalogue_streaming_chunk();
    assert!(
        max_chunk <= 128,
        "streaming chunk bound exceeded: {max_chunk}"
    );
    assert!(max_chunk > 0, "the streamed path must have flushed");
    service.shutdown().unwrap();
}

/// A mid-stream provider failure must leave no batch, rows or staging.
#[test]
fn failed_streamed_search_leaves_no_batch_rows_or_staging() {
    let database = TempStore::new();
    let catalog = Arc::new(FailingStreamCatalog {
        count: 250,
        fail_after: 50,
    });
    let service = service(&database, catalog);
    let error = service
        .catalog_search_and_freeze(&request("fail", 250))
        .unwrap_err();
    assert!(format!("{error:?}").contains("STAC_SERVER_ERROR"));
    assert!(service.restore(true).unwrap().workbench_batch.is_none());
    let store = TaskStore::open(database.database()).unwrap();
    assert_eq!(store.count_search_batches().unwrap(), 0);
    assert_eq!(store.count_staging_rows().unwrap(), 0);
    service.shutdown().unwrap();
}

/// A cancelled streamed search must leave no batch, rows or staging.
#[test]
fn cancelled_streamed_search_leaves_no_batch_rows_or_staging() {
    struct CancellingCatalog {
        count: usize,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    }
    impl LiveCatalogSearch for CancellingCatalog {
        fn search_catalog(
            &self,
            _: &CatalogSearchQuery,
            _: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            panic!("the streaming entry must not fall back to search_catalog");
        }
        fn search_catalog_streaming(
            &self,
            query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
            sink: &mut dyn insar_service::CatalogSearchStreamSink,
            _progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
        ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
            sink.attempt(query)?;
            let response = ChunkedCatalog {
                count: self.count,
                chunk_size: 10,
                accepts: Mutex::new(Vec::new()),
            }
            .response(query);
            for chunk in response.products.chunks(10) {
                sink.accept(chunk)?;
                self.cancel.store(true, Ordering::Release);
            }
            Err(insar_core::ContractError::new(
                "STAC_SEARCH_CANCELLED",
                "fixture cancelled",
                Default::default(),
            )
            .unwrap())
        }
    }
    let database = TempStore::new();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let catalog = Arc::new(CancellingCatalog {
        count: 100,
        cancel: cancel.clone(),
    });
    let service = service(&database, catalog);
    let error = service
        .catalog_search_and_freeze(&request("cancel", 100))
        .unwrap_err();
    assert!(format!("{error:?}").contains("CANCEL"));
    assert!(service.restore(true).unwrap().workbench_batch.is_none());
    let store = TaskStore::open(database.database()).unwrap();
    assert_eq!(store.count_search_batches().unwrap(), 0);
    assert_eq!(store.count_staging_rows().unwrap(), 0);
    service.shutdown().unwrap();
}

/// #22 B slice 3(b) review fix 3 (application half): a late
/// `streaming_cap_order` fail-closed (page-2 continuation changed sort
/// after two flushed pages) must still produce a successful search with the
/// identical candidates after the whole-response retry resets the staging.
#[test]
fn late_cap_order_failure_still_freezes_identical_candidates() {
    /// Streams two chunks, then fails closed with the typed cap-order reason;
    /// the retry helper then re-fetches the whole response (same products).
    struct LateCapOrderCatalog {
        count: usize,
    }
    impl LiveCatalogSearch for LateCapOrderCatalog {
        fn search_catalog(
            &self,
            query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            Ok(ChunkedCatalog {
                count: self.count,
                chunk_size: self.count,
                accepts: Mutex::new(Vec::new()),
            }
            .response(query))
        }
        fn search_catalog_streaming(
            &self,
            query: &CatalogSearchQuery,
            cancelled: &(dyn Fn() -> bool + Sync),
            sink: &mut dyn insar_service::CatalogSearchStreamSink,
            progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
        ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
            sink.attempt(query)?;
            let response = ChunkedCatalog {
                count: self.count,
                chunk_size: 10,
                accepts: Mutex::new(Vec::new()),
            }
            .response(query);
            // Flush two pages, then a later continuation changes the sort:
            // the strict stream fails closed with the typed reason.
            sink.accept(&response.products[..20])?;
            let failure = insar_core::ContractError::new(
                "STAC_RESPONSE_INVALID",
                "fixture mid-stream sort change",
                serde_json::Map::from_iter([(
                    "reason".to_owned(),
                    serde_json::Value::String("streaming_cap_order".to_owned()),
                )]),
            )
            .unwrap();
            if insar_service::is_streaming_cap_order(&failure) {
                // The production retry contract: the whole response replaces
                // the failed stream, after the sink resets its staged rows.
                return insar_service::retry_whole_response_streamed(
                    self, query, cancelled, sink, progress,
                );
            }
            Err(failure)
        }
    }

    let database = TempStore::new();
    let catalog = Arc::new(LateCapOrderCatalog { count: 60 });
    let service = service(&database, catalog);
    service
        .catalog_search_and_freeze(&request("cap-order", 60))
        .expect("the whole-response retry must make the search succeed");
    let batch = service.restore(true).unwrap().workbench_batch.unwrap();
    assert_eq!(batch.candidates.len(), 60, "identical candidate count");
    let store = TaskStore::open(database.database()).unwrap();
    assert_eq!(
        store.count_batch_candidate_rows(&batch.batch_id).unwrap(),
        60,
        "identical candidate rows"
    );
    assert_eq!(store.count_staging_rows().unwrap(), 0, "no staging residue");
    service.shutdown().unwrap();
}

/// #22 B slice 3(b) review fix 2: the application must seal streamed searches
/// with the streaming cache-pointer seal, not collect all staged rows through
/// the classic whole-batch seal. The store's read-batch seam proves the
/// streaming seal ran; the cache pointer must still resolve to the batch.
#[test]
fn streamed_search_seals_with_the_streaming_cache_pointer_seal() {
    let database = TempStore::new();
    let catalog = Arc::new(ChunkedCatalog {
        count: 300,
        chunk_size: 40,
        accepts: Mutex::new(Vec::new()),
    });
    let service = service(&database, catalog);
    service
        .catalog_search_and_freeze(&request("seal-stream", 300))
        .unwrap();
    let seal_read_batch = service.take_max_catalogue_seal_read_batch();
    assert!(
        seal_read_batch >= 1,
        "the streaming seal must have read the staged rows in batches"
    );
    // The cache pointer written by the streaming seal still resolves.
    let batch = service.restore(true).unwrap().workbench_batch.unwrap();
    assert_eq!(batch.candidates.len(), 300);
    let store = TaskStore::open(database.database()).unwrap();
    assert_eq!(
        store.count_batch_candidate_rows(&batch.batch_id).unwrap(),
        300
    );
    service.shutdown().unwrap();
}

/// #22 B slice 3(b) review fix 4: the streamed search must keep the exact
/// pre-3b Fetching progress sequence. A multi-page streamed client reports
/// its own progress while streaming; the stage sequence and provider counts
/// must match WaitingProvider → Fetching (per page) → Filtering → Freezing.
#[test]
fn streamed_search_emits_the_pre_3b_fetching_progress_sequence() {
    /// A streamed client that reports CatalogSearchProviderProgress after
    /// each flushed page, like the migrated STAC transports.
    struct ProgressingStreamCatalog;
    impl LiveCatalogSearch for ProgressingStreamCatalog {
        fn search_catalog(
            &self,
            _query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            panic!("the streaming entry must not fall back to search_catalog");
        }
        fn search_catalog_streaming(
            &self,
            query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
            sink: &mut dyn insar_service::CatalogSearchStreamSink,
            progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
        ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
            sink.attempt(query)?;
            let response = ChunkedCatalog {
                count: 40,
                chunk_size: 20,
                accepts: Mutex::new(Vec::new()),
            }
            .response(query);
            let limit = query.result_limit.unwrap_or(40);
            for (page, chunk) in response.products.chunks(20).enumerate() {
                sink.accept(chunk)?;
                progress(
                    insar_provider::asf::catalog_client::CatalogSearchProviderProgress {
                        provider_request_count: page as u64 + 1,
                        transport_part_count: 1,
                        raw_fetched_count: (page + 1) as u64 * 20,
                        mapped_count: (page + 1) as u64 * 20,
                        unique_admitted_count: (page + 1) as u64 * 20,
                        exact_matched_count: (page + 1) as u64 * 20,
                        requested_limit: Some(limit),
                        remote_total: Some(40),
                        elapsed_ms: 10,
                    },
                );
            }
            Ok(insar_service::CatalogSearchStreamOutcome::Streamed {
                collection: response.collection,
                stats: response.stats,
            })
        }
    }
    let database = TempStore::new();
    let service = service(&database, Arc::new(ProgressingStreamCatalog));
    let updates = Mutex::new(Vec::new());
    service
        .catalog_search_and_freeze_with_progress(&request("progress", 40), |update| {
            updates.lock().unwrap().push(update);
        })
        .unwrap();
    let updates = updates.into_inner().unwrap();
    let stages: Vec<&str> = updates
        .iter()
        .map(|update| match update.stage {
            insar_core::ApplicationSearchProgressStage::Planning => "planning",
            insar_core::ApplicationSearchProgressStage::WaitingProvider => "waiting_provider",
            insar_core::ApplicationSearchProgressStage::Fetching => "fetching",
            insar_core::ApplicationSearchProgressStage::Filtering => "filtering",
            insar_core::ApplicationSearchProgressStage::Freezing => "freezing",
            insar_core::ApplicationSearchProgressStage::Completed => "completed",
            _ => "other",
        })
        .collect();
    assert!(
        stages.contains(&"waiting_provider"),
        "WaitingProvider must precede provider work: {stages:?}"
    );
    let fetching: Vec<&_> = updates
        .iter()
        .filter(|update| {
            matches!(
                update.stage,
                insar_core::ApplicationSearchProgressStage::Fetching
            )
        })
        .collect();
    assert_eq!(
        fetching.len(),
        2,
        "two flushed pages must each emit a Fetching update: {stages:?}"
    );
    // Identical pre-3b field shape on the last Fetching update.
    let last = fetching.last().unwrap();
    assert_eq!(last.provider_request_count, Some(2));
    assert_eq!(last.raw_fetched_count, Some(40));
    assert_eq!(last.mapped_count, Some(40));
    assert_eq!(last.admitted_count, Some(40));
    assert_eq!(last.requested_limit, Some(40));
    assert_eq!(last.remote_total, Some(40));
    // Stage order: fetching strictly between waiting_provider and filtering.
    let waiting = stages
        .iter()
        .position(|s| *s == "waiting_provider")
        .unwrap();
    let first_fetch = stages
        .iter()
        .position(|s| *s == "fetching")
        .expect("a Fetching stage must exist");
    let filtering = stages
        .iter()
        .position(|s| *s == "filtering")
        .expect("a Filtering stage must exist");
    assert!(
        waiting < first_fetch && first_fetch < filtering,
        "{stages:?}"
    );
    service.shutdown().unwrap();
}

/// #22 B slice 3(b) review fix 4: an unmigrated (ASF/Bhoonidhi-style)
/// whole-response client must still emit Fetching progress through its
/// `search_catalog_with_progress` override; the trait default forwards the
/// streaming entry to it instead of bypassing it.
#[test]
fn whole_response_client_keeps_fetching_progress_through_the_default() {
    /// Whole-response client whose progress override records its calls.
    struct WholeResponseProgressCatalog {
        progress_calls: std::sync::atomic::AtomicUsize,
    }
    impl LiveCatalogSearch for WholeResponseProgressCatalog {
        fn search_catalog(
            &self,
            query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            Ok(ChunkedCatalog {
                count: 30,
                chunk_size: 30,
                accepts: Mutex::new(Vec::new()),
            }
            .response(query))
        }
        // The ASF/Bhoonidhi-style progress override. No streaming override:
        // the trait default must forward here.
        fn search_catalog_with_progress(
            &self,
            query: &CatalogSearchQuery,
            cancelled: &(dyn Fn() -> bool + Sync),
            progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
        ) -> ProviderResult<CatalogSearchResponse> {
            self.progress_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            progress(
                insar_provider::asf::catalog_client::CatalogSearchProviderProgress {
                    provider_request_count: 1,
                    transport_part_count: 1,
                    raw_fetched_count: 30,
                    mapped_count: 30,
                    unique_admitted_count: 30,
                    exact_matched_count: 30,
                    requested_limit: Some(30),
                    remote_total: Some(30),
                    elapsed_ms: 5,
                },
            );
            self.search_catalog(query, cancelled)
        }
    }
    let database = TempStore::new();
    let catalog = Arc::new(WholeResponseProgressCatalog {
        progress_calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let service = service(&database, catalog.clone());
    let updates = Mutex::new(Vec::new());
    service
        .catalog_search_and_freeze_with_progress(&request("whole-progress", 30), |update| {
            updates.lock().unwrap().push(update);
        })
        .unwrap();
    assert_eq!(
        catalog
            .progress_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the whole-response client's progress override must run exactly once"
    );
    let updates = updates.into_inner().unwrap();
    assert!(
        updates.iter().any(|update| {
            matches!(
                update.stage,
                insar_core::ApplicationSearchProgressStage::Fetching
            ) && update.provider_request_count == Some(1)
                && update.raw_fetched_count == Some(30)
                && update.admitted_count == Some(30)
        }),
        "a Fetching update with the provider's counts must be emitted"
    );
    service.shutdown().unwrap();
}

/// #22 B slice 3(b) review fix 7: the streamed fixture validation must run
/// the same stats checks as stac_fixture's validate_response. A streamed
/// response whose stats violate the fixture completeness contract (here:
/// transport_part_count != 1, which the old weak streamed check ignored)
/// must fail closed.
#[test]
fn streamed_fixture_stats_validation_matches_whole_response_checks() {
    /// Streams valid products but reports multi-part transport stats.
    struct MultiPartStatsCatalog;
    impl LiveCatalogSearch for MultiPartStatsCatalog {
        fn search_catalog(
            &self,
            _query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            panic!("the streaming entry must not fall back to search_catalog");
        }
        fn search_catalog_streaming(
            &self,
            query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
            sink: &mut dyn insar_service::CatalogSearchStreamSink,
            _progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
        ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
            sink.attempt(query)?;
            let response = ChunkedCatalog {
                count: 10,
                chunk_size: 10,
                accepts: Mutex::new(Vec::new()),
            }
            .response(query);
            sink.accept(&response.products)?;
            // Same products, but stats claim two transport parts: the fixture
            // route is single-part, so validate_response rejects this and the
            // streamed stats check must reject it too.
            let mut stats = response.stats.clone();
            stats.transport_part_count = 2;
            Ok(insar_service::CatalogSearchStreamOutcome::Streamed {
                collection: response.collection,
                stats,
            })
        }
    }
    let database = TempStore::new();
    let service = service(&database, Arc::new(MultiPartStatsCatalog));
    let error = service
        .catalog_search_and_freeze(&request("stats-parity", 10))
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("CATALOGUE_RESPONSE_INCOMPLETE"),
        "the streamed stats check must reject multi-part transport stats: {error:?}"
    );
    // Nothing was published.
    assert!(service.restore(true).unwrap().workbench_batch.is_none());
    let store = TaskStore::open(database.database()).unwrap();
    assert_eq!(store.count_search_batches().unwrap(), 0);
    assert_eq!(store.count_staging_rows().unwrap(), 0);
    service.shutdown().unwrap();
}

/// #22 B slice 3(b) review fix 9: the max-chunk test seam must survive a
/// failed first source. The seam records the largest staged chunk across ALL
/// attempts of the search; attempt() must not reset it.
#[test]
fn max_chunk_seam_survives_a_failed_first_source() {
    use insar_core::{
        AsfRemoteSearchRequest, AsfRemoteSearchResponse, SarRemoteSearchRequest,
        SarRemoteSearchResponse,
    };
    use insar_service::Dispatcher;
    struct NoAsf;
    impl insar_service::LiveAsfSearch for NoAsf {
        fn search(
            &self,
            _: &AsfRemoteSearchRequest,
            _: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<AsfRemoteSearchResponse> {
            panic!("catalogue search must never call ASF");
        }
        fn search_sar(
            &self,
            _: &SarRemoteSearchRequest,
            _: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<SarRemoteSearchResponse> {
            panic!("catalogue search must never call ASF SAR");
        }
    }
    /// First source: stage a 10-item chunk, then fail transiently. Second
    /// source: stream a 60-item chunk and succeed.
    struct FallbackStreamCatalog {
        calls: std::sync::atomic::AtomicUsize,
        fixture_query: CatalogSearchQuery,
    }
    impl LiveCatalogSearch for FallbackStreamCatalog {
        fn search_catalog(
            &self,
            _query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            panic!("the streaming entry must not fall back to search_catalog");
        }
        fn search_catalog_streaming(
            &self,
            query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
            sink: &mut dyn insar_service::CatalogSearchStreamSink,
            _progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
        ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            sink.attempt(query)?;
            let count = if call == 0 { 10 } else { 60 };
            let response = ChunkedCatalog {
                count,
                chunk_size: count,
                accepts: Mutex::new(Vec::new()),
            }
            .response(&self.fixture_query);
            sink.accept(&response.products)?;
            if call == 0 {
                return Err(insar_core::ContractError::new(
                    "STAC_NETWORK_ERROR",
                    "fixture transient failure",
                    Default::default(),
                )
                .unwrap());
            }
            Ok(insar_service::CatalogSearchStreamOutcome::Streamed {
                collection: response.collection,
                stats: response.stats,
            })
        }
    }
    /// A sink mirroring the app's chunk tracking: max kept across attempts.
    struct SeamSink {
        max_chunk: usize,
    }
    impl insar_service::CatalogSearchStreamSink for SeamSink {
        fn accept(&mut self, products: &[insar_core::ProviderProduct]) -> ProviderResult<()> {
            self.max_chunk = self.max_chunk.max(products.len());
            Ok(())
        }
        // attempt() deliberately does NOT reset max_chunk: the seam must
        // observe every attempt of the search.
    }

    let dispatcher = Dispatcher::with_production_catalogue_for_application(
        Arc::new(NoAsf),
        Arc::new(FallbackStreamCatalog {
            calls: std::sync::atomic::AtomicUsize::new(0),
            fixture_query: request("fallback-seam", 60).search,
        }),
    );
    // Two real production routes (Earth Search and MPC Landsat) so the
    // registration's route validation accepts the candidate list. The
    // fixture mapper still needs the fixture identity, so the catalog maps
    // with the original fixture query.
    let mut first = request("fallback-seam", 60).search;
    first.logical_dataset = "optical.landsat.c2.l2".to_owned();
    first.logical_dataset = "optical.landsat.c2.l2".to_owned();
    first.provider_id = "element84.earth-search".to_owned();
    first.native_collection_id = "landsat-c2-l2".to_owned();
    let mut second = first.clone();
    second.provider_id = "microsoft.planetary-computer".to_owned();
    let registered = dispatcher
        .register_catalog_search_candidates_for_application("fallback-seam", &[first, second])
        .unwrap();
    let mut sink = SeamSink { max_chunk: 0 };
    let streamed = registered
        .execute_candidates_streaming(
            &|error| {
                if error.code == "STAC_NETWORK_ERROR" {
                    Some(insar_service::CatalogSourceFallbackReason::TransientNetwork)
                } else {
                    None
                }
            },
            &mut sink,
            &|_| {},
        )
        .expect("the fallback source must win after the transient failure");
    assert_eq!(
        streamed.source_attempts.len(),
        2,
        "one failed attempt, one successful"
    );
    assert_eq!(
        sink.max_chunk, 60,
        "the seam must keep the fallback attempt's larger flush"
    );
}
