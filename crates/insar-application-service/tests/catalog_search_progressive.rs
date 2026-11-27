//! #22 B slice 3(c): progressive delivery through the ApplicationService.
//! Once the first full page (250 rows) is staged the search is readable as a
//! loading batch while the provider keeps streaming; a page that has not
//! arrived is the typed not-yet-loaded answer; cancel and a mid-way failure
//! keep the arrived rows marked possibly incomplete; a restart turns a
//! loading batch interrupted; progress names the batch, the loaded count and
//! the provider total; only all-streaming sources default to the 10,000
//! ceiling. All products and URLs are synthetic.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    StoreBatchPage, StoreBatchPageRequest,
};
use insar_core::{
    ApplicationSearchProgressStage, AsfRemoteSearchRequest, AsfRemoteSearchResponse, AssetRole,
    CatalogSearchQuery, CatalogSearchResponse, DataSourceKind, DownloadItemProgress,
    DownloadItemResult, ProductKind, ProviderResult, SearchBatchProgressState,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::asf::catalog_client::CatalogSearchProviderProgress;
use insar_provider::catalog::{
    map_stac_complete_search_response, CatalogDeliveryModel, StacAssetRoleBinding,
    StacCollectionBinding, StacOfflineFixtureAdmission, StacRouteProfile,
};
use insar_service::{
    CatalogSearchStreamOutcome, CatalogSearchStreamSink, LiveAsfSearch, LiveCatalogSearch,
};
use insar_store::TaskStore;
use serde_json::{json, Value};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);
impl TempStore {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sceneport-progressive-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
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
        created_at: "2026-11-03T00:00:00Z".into(),
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

fn response(query: &CatalogSearchQuery, count: usize) -> CatalogSearchResponse {
    let features: Vec<Value> = (0..count).map(feature).collect();
    map_stac_complete_search_response(
        &route(),
        &binding(),
        query,
        &json!({
            "type":"FeatureCollection",
            "numberMatched": count,
            "numberReturned": count,
            "features": features,
            "links":[]
        }),
    )
    .unwrap()
}

type ServiceSlot = Arc<OnceLock<Weak<ApplicationService>>>;
type ChunkHook = Box<dyn Fn(&ApplicationService, usize) + Send + Sync>;

/// Streams `count` products in `chunk_size` chunks, reports provider
/// progress (with the provider total) after each chunk, runs `on_chunk`
/// with the service between chunks, honours cancellation between chunks and
/// optionally fails after `fail_after` products.
struct ScriptedCatalog {
    count: usize,
    chunk_size: usize,
    fail_after: Option<usize>,
    service: ServiceSlot,
    on_chunk: ChunkHook,
}

impl LiveCatalogSearch for ScriptedCatalog {
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
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn CatalogSearchStreamSink,
        progress: &dyn Fn(CatalogSearchProviderProgress),
    ) -> ProviderResult<CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let response = response(query, self.count);
        let mut streamed = 0_usize;
        for (index, chunk) in response.products.chunks(self.chunk_size).enumerate() {
            if cancelled() {
                return Err(insar_core::ContractError::new(
                    "STAC_SEARCH_CANCELLED",
                    "fixture cancelled",
                    Default::default(),
                )
                .unwrap());
            }
            if self.fail_after.is_some_and(|limit| streamed >= limit) {
                return Err(insar_core::ContractError::new(
                    "STAC_SERVER_ERROR",
                    "fixture mid-stream failure",
                    Default::default(),
                )
                .unwrap());
            }
            sink.accept(chunk)?;
            streamed += chunk.len();
            progress(CatalogSearchProviderProgress {
                provider_request_count: index as u64 + 1,
                transport_part_count: 1,
                raw_fetched_count: streamed as u64,
                mapped_count: streamed as u64,
                unique_admitted_count: streamed as u64,
                exact_matched_count: streamed as u64,
                requested_limit: query.result_limit,
                remote_total: Some(self.count as u64),
                elapsed_ms: 10,
            });
            if let Some(service) = self.service.get().and_then(Weak::upgrade) {
                (self.on_chunk)(&service, streamed);
            }
        }
        Ok(CatalogSearchStreamOutcome::Streamed {
            collection: response.collection,
            stats: response.stats,
        })
    }
}

fn service_with(
    database: &TempStore,
    count: usize,
    fail_after: Option<usize>,
    on_chunk: ChunkHook,
) -> Arc<ApplicationService> {
    let slot: ServiceSlot = Arc::new(OnceLock::new());
    let catalog = Arc::new(ScriptedCatalog {
        count,
        chunk_size: 100,
        fail_after,
        service: Arc::clone(&slot),
        on_chunk,
    });
    let service = Arc::new(
        ApplicationService::with_offline_stac_fixture_business_runtime(
            ApplicationServiceConfig::business_store(database.database()),
            Arc::new(NoAsf),
            catalog,
            Arc::new(NoDownload::default()),
            Arc::new(StacOfflineFixtureAdmission::new_for_test(route(), binding()).unwrap()),
        ),
    );
    slot.set(Arc::downgrade(&service)).unwrap();
    service
}

fn page(batch_id: &str, page: u64, page_size: u64) -> StoreBatchPageRequest {
    StoreBatchPageRequest {
        batch_id: batch_id.to_owned(),
        revision: 0,
        page,
        page_size,
    }
}

fn store_page(database: &TempStore, batch_id: &str, index: u64) -> StoreBatchPage {
    TaskStore::open(database.database())
        .unwrap()
        .load_batch_page(&page(batch_id, index, 100))
        .unwrap()
}

#[derive(Debug)]
enum Observed {
    Page(SearchBatchProgressState, u64, usize),
    Error(String),
}

#[test]
fn page_one_is_readable_through_the_service_while_the_provider_streams() {
    let database = TempStore::new();
    let observed = Arc::new(Mutex::new(Vec::<(usize, Observed, Observed)>::new()));
    let sink = Arc::clone(&observed);
    let service = service_with(
        &database,
        600,
        None,
        Box::new(move |service, streamed| {
            let observe = |index| match service.batch_page(&page("batch-live", index, 250)) {
                Ok(page) => Observed::Page(page.state, page.loaded_count, page.items.len()),
                Err(error) => Observed::Error(error.code().to_owned()),
            };
            sink.lock()
                .unwrap()
                .push((streamed, observe(0), observe(2)));
        }),
    );
    service
        .catalog_search_and_freeze(&request("live", 600))
        .unwrap();
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 6);
    for (streamed, first, third) in observed.iter() {
        if *streamed < 250 {
            assert!(
                matches!(first, Observed::Error(code) if code == "V3_STORE_NOT_FOUND"),
                "nothing is readable before the first full page: {first:?}"
            );
            continue;
        }
        assert!(
            matches!(first, Observed::Page(SearchBatchProgressState::Loading, loaded, 250) if *loaded == *streamed as u64),
            "page 1 is readable while loading at {streamed}: {first:?}"
        );
        if *streamed <= 500 {
            assert!(
                matches!(third, Observed::Error(code) if code == "V3_STORE_BATCH_PAGE_NOT_YET_LOADED"),
                "a page that has not arrived is typed at {streamed}: {third:?}"
            );
        } else {
            assert!(matches!(
                third,
                Observed::Page(SearchBatchProgressState::Loading, 600, 100)
            ));
        }
    }
    let sealed = store_page(&database, "batch-live", 0);
    assert_eq!(sealed.state, SearchBatchProgressState::Complete);
    assert_eq!(sealed.loaded_count, 600);
    assert_eq!(sealed.remote_total, Some(600));
    let store = TaskStore::open(database.database()).unwrap();
    assert_eq!(store.count_staging_rows().unwrap(), 0);
    assert!(
        !store
            .load_search_batch("batch-live")
            .unwrap()
            .unwrap()
            .search_stats
            .unwrap()
            .truncated
    );
    drop(store);
    service.shutdown().unwrap();
}

#[test]
fn progress_names_the_batch_loaded_count_and_provider_total() {
    let database = TempStore::new();
    let service = service_with(&database, 400, None, Box::new(|_, _| {}));
    let updates = Mutex::new(Vec::new());
    service
        .catalog_search_and_freeze_with_progress(&request("counts", 400), |update| {
            updates.lock().unwrap().push(update);
        })
        .unwrap();
    service.shutdown().unwrap();
    let updates = updates.into_inner().unwrap();
    let fetching: Vec<_> = updates
        .iter()
        .filter(|update| update.stage == ApplicationSearchProgressStage::Fetching)
        .collect();
    assert_eq!(fetching.len(), 4);
    for update in &fetching {
        assert_eq!(update.remote_total, Some(400), "total first: {update:?}");
        let admitted = update.admitted_count.unwrap();
        if admitted < 250 {
            assert_eq!(update.batch_id, None, "not readable yet: {update:?}");
            assert_eq!(update.loaded_count, None);
        } else {
            assert_eq!(update.batch_id.as_deref(), Some("batch-counts"));
            assert_eq!(update.loaded_count, Some(admitted));
        }
    }
    let terminal = updates.last().unwrap();
    assert_eq!(terminal.stage, ApplicationSearchProgressStage::Completed);
    assert_eq!(terminal.batch_id.as_deref(), Some("batch-counts"));
    assert_eq!(terminal.loaded_count, Some(400));
}

#[test]
fn cancel_command_keeps_arrived_rows_and_marks_them_possibly_incomplete() {
    let database = TempStore::new();
    let outcome = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&outcome);
    let service = service_with(
        &database,
        900,
        None,
        Box::new(move |service, streamed| {
            if streamed == 300 {
                *slot.lock().unwrap() = Some(service.cancel_catalog_search("request-cancel"));
            }
        }),
    );
    let updates = Mutex::new(Vec::new());
    let error = service
        .catalog_search_and_freeze_with_progress(&request("cancel", 900), |update| {
            updates.lock().unwrap().push(update);
        })
        .unwrap_err();
    assert!(format!("{error:?}").contains("CANCEL"), "{error:?}");
    let outcome = outcome.lock().unwrap().take().unwrap().unwrap();
    assert!(outcome.cancelled, "the background fetch was signalled");
    assert_eq!(outcome.batch_id.as_deref(), Some("batch-cancel"));
    assert_eq!(outcome.state, Some(SearchBatchProgressState::Cancelled));
    let terminal = updates.into_inner().unwrap().pop().unwrap();
    assert_eq!(terminal.stage, ApplicationSearchProgressStage::Cancelled);
    assert_eq!(terminal.batch_id.as_deref(), Some("batch-cancel"));
    assert_eq!(terminal.loaded_count, Some(300));

    // A second cancel after the search ended reports the kept batch.
    let again = service.cancel_catalog_search("request-cancel").unwrap();
    assert!(!again.cancelled);
    assert_eq!(again.state, Some(SearchBatchProgressState::Cancelled));
    service.shutdown().unwrap();

    let kept = store_page(&database, "batch-cancel", 2);
    assert_eq!(kept.state, SearchBatchProgressState::Cancelled);
    assert_eq!(kept.loaded_count, 300, "rows that arrived are kept");
    assert_eq!(kept.remote_total, Some(900));
    let store = TaskStore::open(database.database()).unwrap();
    let frozen = store.load_search_batch("batch-cancel").unwrap().unwrap();
    assert!(frozen.search_stats.as_ref().unwrap().truncated);
    assert_eq!(store.count_staging_rows().unwrap(), 0);
}

#[test]
fn cancel_before_the_first_full_page_leaves_no_batch() {
    let database = TempStore::new();
    let outcome = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&outcome);
    let service = service_with(
        &database,
        900,
        None,
        Box::new(move |service, streamed| {
            if streamed == 100 {
                *slot.lock().unwrap() = Some(service.cancel_catalog_search("request-early"));
            }
        }),
    );
    assert!(service
        .catalog_search_and_freeze(&request("early", 900))
        .is_err());
    let outcome = outcome.lock().unwrap().take().unwrap().unwrap();
    assert!(outcome.cancelled);
    assert_eq!(outcome.batch_id, None);
    assert_eq!(outcome.state, None);
    service.shutdown().unwrap();
    let store = TaskStore::open(database.database()).unwrap();
    assert_eq!(store.count_search_batches().unwrap(), 0);
    assert_eq!(store.count_staging_rows().unwrap(), 0);
}

#[test]
fn failure_after_the_first_page_keeps_rows_with_a_typed_error() {
    let database = TempStore::new();
    let service = service_with(&database, 900, Some(300), Box::new(|_, _| {}));
    let updates = Mutex::new(Vec::new());
    let error = service
        .catalog_search_and_freeze_with_progress(&request("fail", 900), |update| {
            updates.lock().unwrap().push(update);
        })
        .unwrap_err();
    assert!(
        format!("{error:?}").contains("STAC_SERVER_ERROR"),
        "{error:?}"
    );
    let terminal = updates.into_inner().unwrap().pop().unwrap();
    assert_eq!(terminal.stage, ApplicationSearchProgressStage::Failed);
    assert_eq!(terminal.batch_id.as_deref(), Some("batch-fail"));
    service.shutdown().unwrap();

    let kept = store_page(&database, "batch-fail", 0);
    assert_eq!(kept.state, SearchBatchProgressState::Failed);
    assert_eq!(kept.loaded_count, 300);
    let store = TaskStore::open(database.database()).unwrap();
    assert!(
        store
            .load_search_batch("batch-fail")
            .unwrap()
            .unwrap()
            .search_stats
            .unwrap()
            .truncated
    );
    assert_eq!(store.count_staging_rows().unwrap(), 0);
}

#[test]
fn service_start_turns_a_loading_batch_interrupted() {
    let database = TempStore::new();
    // The first process published page 1 and then died: its staging and
    // loading state are all that is left in the Store.
    let service = service_with(
        &database,
        900,
        None,
        Box::new(|_, streamed| {
            if streamed == 300 {
                panic!("simulated process death after page 1");
            }
        }),
    );
    let search = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        service.catalog_search_and_freeze(&request("restart", 900))
    }));
    assert!(search.is_err(), "the simulated death unwinds the search");
    drop(service);
    {
        let store = TaskStore::open(database.database()).unwrap();
        assert_eq!(
            store
                .load_batch_page(&page("batch-restart", 0, 100))
                .unwrap()
                .state,
            SearchBatchProgressState::Loading
        );
    }
    let restarted = service_with(&database, 0, None, Box::new(|_, _| {}));
    restarted.core_restore(false).unwrap();
    let page = restarted
        .batch_page(&page("batch-restart", 0, 100))
        .unwrap();
    assert_eq!(
        page.state,
        SearchBatchProgressState::Interrupted,
        "a loading batch never silently reads as complete after a restart"
    );
    assert_eq!(page.loaded_count, 300);
    restarted.shutdown().unwrap();
}

#[test]
fn source_defaults_are_unchanged_while_the_ceiling_default_is_held() {
    // #22 B slice 3(c): the streaming eligibility is computed (unit tests in
    // unified_search.rs) but the 10,000 default is held until the pinned HLS
    // expectations are re-decided; no source default moves in this slice.
    let sources = ApplicationService::new(ApplicationServiceConfig::liveness_only())
        .list_sources()
        .unwrap();
    let cap = |id: &str| {
        sources
            .iter()
            .find(|source| source.resource_id == id)
            .unwrap_or_else(|| panic!("{id} is listed"))
            .result_cap
    };
    // Owner decision 2026-11-13: catalogue sources default to 250 like SAR.
    assert_eq!(cap("science.hls.l30.v2"), 250);
    assert_eq!(cap("science.sentinel-2.l2a.v1"), 250);
    assert_eq!(cap("science.landsat.c2-l2.v1"), 250);
    assert_eq!(cap("science.nisar.l.rslc.provisional.v1"), 250);
    assert_eq!(cap("science.sentinel-1.slc-burst.v1"), 250);
}

#[test]
fn integrity_failure_after_publication_withdraws_the_rows() {
    // A provider that returned an untrustworthy response (here: its stats
    // claim more matches than it mapped) fails closed with no batch, rows or
    // staging, even though page 1 had already become readable.
    struct InconsistentCatalog;
    impl LiveCatalogSearch for InconsistentCatalog {
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
            _: &(dyn Fn() -> bool + Sync),
            sink: &mut dyn CatalogSearchStreamSink,
            _: &dyn Fn(CatalogSearchProviderProgress),
        ) -> ProviderResult<CatalogSearchStreamOutcome> {
            sink.attempt(query)?;
            let response = response(query, 300);
            for chunk in response.products.chunks(100) {
                sink.accept(chunk)?;
            }
            let mut stats = response.stats;
            stats.returned_count = 301;
            Ok(CatalogSearchStreamOutcome::Streamed {
                collection: response.collection,
                stats,
            })
        }
    }
    let database = TempStore::new();
    let service = ApplicationService::with_offline_stac_fixture_business_runtime(
        ApplicationServiceConfig::business_store(database.database()),
        Arc::new(NoAsf),
        Arc::new(InconsistentCatalog),
        Arc::new(NoDownload::default()),
        Arc::new(StacOfflineFixtureAdmission::new_for_test(route(), binding()).unwrap()),
    );
    assert!(service
        .catalog_search_and_freeze(&request("inconsistent", 300))
        .is_err());
    service.shutdown().unwrap();
    let store = TaskStore::open(database.database()).unwrap();
    assert_eq!(store.count_search_batches().unwrap(), 0);
    assert_eq!(store.count_staging_rows().unwrap(), 0);
    assert!(store
        .search_batch_progress("batch-inconsistent")
        .unwrap()
        .is_none());
}
