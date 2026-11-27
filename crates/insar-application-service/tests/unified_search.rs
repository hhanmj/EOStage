use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, TaskAdmissionPreflight, UnifiedCreateTaskOptions,
    UnifiedSearchRequest,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery, CatalogSearchResponse,
    DownloadItemProgress, DownloadItemResult, DownloadLayout, FrozenSearchBatch, ProviderResult,
    SearchProgressStage, SearchWorkbenchState, SnapshotItem, StoreSearchBatchRequest, TypedError,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    sync::{mpsc, Arc, Mutex},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct TempStore(PathBuf);
impl TempStore {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "unified-search-{}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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
struct NoAsf;
impl LiveAsfSearch for NoAsf {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("optical route must not call ASF")
    }
}
struct NoDownload;
impl DownloadWorker for NoDownload {
    fn execute(
        &self,
        _: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        panic!("search must not download")
    }
}
struct Catalog {
    queries: Mutex<Vec<CatalogSearchQuery>>,
    response: CatalogSearchResponse,
}
struct FailingCatalog {
    code: &'static str,
}
impl LiveCatalogSearch for FailingCatalog {
    fn search_catalog(
        &self,
        _: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        Err(
            insar_core::ContractError::new(self.code, "offline fixture failure", Map::new())
                .unwrap(),
        )
    }
}
impl LiveCatalogSearch for Catalog {
    fn search_catalog(
        &self,
        q: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.queries.lock().unwrap().push(q.clone());
        Ok(self.response.clone())
    }
}
fn aoi() -> Map<String, Value> {
    json!({"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]})
        .as_object()
        .unwrap()
        .clone()
}
fn fixture() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection":{"logical_dataset":"optical.hls.l30.v2","provider_id":"microsoft.planetary-computer","native_collection_id":"hls2-l30","collection_profile_id":"planetary-computer.hls2-l30.scene.v1","identity_rule_id":"planetary-computer.hls2.identity.v1","filename_rule_id":"planetary-computer.hls2.search-only.v1","delivery_model":"archive_first","production_enabled":true},
        "products":[{"provider_id":"microsoft.planetary-computer","product_id":"HLS.L30.T49RDP.2024177T030756.v2.0","display_name":"HLS.L30.T49RDP.2024177T030756.v2.0","source_kind":"landsat","product_kind":"optical_scene","acquisition_datetime":"2024-06-25T03:07:56.017810Z","footprint_geojson":{"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},"assets":[],"properties":{"platform":"landsat_8","cloud_cover":79.0},"payload":{}}],
        // Owner decision 2026-11-21: streaming optical sources (including HLS L30) continue up to 2,000 after first 250.
        "stats":{"requested_limit":2000,"matched_count":1,"request_count":1,"transport_part_count":1,"fetched_record_count":1,"mapped_record_count":1,"profile_filtered_record_count":0,"exact_duplicate_record_count":0,"unique_provider_product_count":1,"exact_matched_count":1,"returned_count":1,"remote_exhausted":true,"working_set_satisfied":true,"elapsed_ms":1,"truncated":false},"item_errors":[]
    })).unwrap()
}
fn request() -> UnifiedSearchRequest {
    UnifiedSearchRequest {
        request_id: "unified-hls-1".to_owned(),
        resource_id: "science.hls.l30.v2".to_owned(),
        aoi: aoi(),
        aoi_name: "测试区（矩形）".to_owned(),
        time_range: None,
        filters: json!({"spacecraft":["landsat_8"],"cloud_cover_lte":80.0})
            .as_object()
            .unwrap()
            .clone(),
    }
}
struct BlockingPreflight {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl TaskAdmissionPreflight for BlockingPreflight {
    fn check(
        &self,
        _: &FrozenSearchBatch,
        _: &[insar_core::SnapshotItem],
        _: &[String],
    ) -> Result<(), TypedError> {
        self.entered.send(()).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        Ok(())
    }
}

#[test]
fn search_start_works_while_task_preflight_is_waiting() {
    let db = TempStore::new();
    let mut store = TaskStore::open(&db.0).unwrap();
    store.save_search_batch(&StoreSearchBatchRequest {
        batch: FrozenSearchBatch { schema_version:1,batch_id:"public-batch".to_owned(),
            provider_id:"microsoft.planetary-computer".to_owned(),created_at:"2026-01-01T00:00:00Z".to_owned(),
            aoi_name:"fixture".to_owned(),aoi_geojson:Some(aoi()),
            query:json!({"logical_dataset":"optical.landsat.c2.l2","native_collection_id":"landsat-c2-l2"}).as_object().unwrap().clone(),
            search_stats:None,candidates:vec![SnapshotItem {item_id:"public-scene".to_owned(),provider_id:"microsoft.planetary-computer".to_owned(),display_name:"scene".to_owned(),acquisition_datetime:None,footprint_geojson:None,metadata:Map::new(),payload:Map::new(),asset_access:None,execution_policy:None}]},
        workbench:SearchWorkbenchState{batch_id:"public-batch".to_owned(),checked_item_ids:vec![],page_index:0,page_size:24,revision:0},
        activate_workbench:false,
    }).unwrap();
    drop(store);
    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let preflight = BlockingPreflight {
        entered: entered_tx,
        release: Mutex::new(release_rx),
    };
    let task_service = Arc::clone(&service);
    let task = std::thread::spawn(move || {
        task_service.create_task_with_preflight(
            "public-batch".to_owned(),
            0,
            vec!["public-scene".to_owned()],
            UnifiedCreateTaskOptions {
                output_root: PathBuf::from("fixture-output"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                include_precise_orbit: None,
            },
            &preflight,
        )
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("task entered preflight");
    // If create_task holds the Store/runtime gate across preflight, search_start
    // cannot return before release; leave the worker a finite timeout to fail.
    let (started_tx, started_rx) = mpsc::channel();
    let search_service = Arc::clone(&service);
    let search = std::thread::spawn(move || {
        let _ = started_tx.send(search_service.search_start(request()));
    });
    let started = started_rx.recv_timeout(std::time::Duration::from_millis(500));
    release_tx.send(()).unwrap();
    assert!(started
        .expect("search can start while task preflights")
        .is_ok());
    let _ = search.join();
    let _ = task.join();
    service.shutdown().unwrap();
}

#[test]
fn optical_unified_start_returns_progress_and_reuses_registry_intent() {
    let db = TempStore::new();
    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    let started = service.search_start(request()).expect("search admitted");
    assert_eq!(started.request_id, "unified-hls-1");
    assert_eq!(started.resource_id, "science.hls.l30.v2");
    let terminal = (0..100)
        .find_map(|_| {
            let p = service.search_progress("unified-hls-1");
            if p.as_ref().is_some_and(|p| p.terminal) {
                p
            } else {
                std::thread::sleep(std::time::Duration::from_millis(20));
                None
            }
        })
        .expect("terminal progress");
    assert_eq!(terminal.stage, SearchProgressStage::Done);
    assert_eq!(terminal.requests_done, 1);
    assert_eq!(terminal.items_kept, 1);
    // Owner decision 2026-11-21: optical streaming sources publish page 1 (250) and continue to 2,000.
    assert_eq!(terminal.items_cap, 2_000);
    // One kept with the remote side exhausted: complete, no warning.
    assert!(!terminal.possibly_incomplete);
    let queries = catalog.queries.lock().unwrap();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].provider_id, "microsoft.planetary-computer");
    drop(queries);
    let batch = service.restore(true).unwrap().workbench_batch.unwrap();
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(batch.aoi_name, "测试区（矩形）");
    service.shutdown().unwrap();
}

#[test]
fn unified_search_requires_nonblank_aoi_name_before_provider_io() {
    let db = TempStore::new();
    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    for name in ["", "  \t "] {
        let mut request = request();
        let mut value = serde_json::to_value(&request).unwrap();
        value["aoi_name"] = json!(name);
        request = serde_json::from_value(value).expect("request accepts aoi_name");
        assert!(service.search_start(request).is_err());
    }
    assert!(catalog.queries.lock().unwrap().is_empty());
    service.shutdown().unwrap();
}
#[test]
fn search_failure_classifies_account_network_revision_not_found_and_other_codes() {
    let codes = [
        ("BHOONIDHI_UNAUTHORIZED", "账号", false),
        ("CATALOGUE_TRANSPORT_FAILURE", "网络", true),
        ("CATALOGUE_PROVIDER_TIMEOUT", "网络", true),
        ("CATALOGUE_RESPONSE_INVALID", "检索", false),
    ];
    for (index, (code, message_fragment, retryable)) in codes.into_iter().enumerate() {
        let db = TempStore::new();
        let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
            ApplicationServiceConfig::business_store(&db.0),
            Arc::new(NoAsf),
            Arc::new(FailingCatalog { code }),
            Arc::new(NoDownload),
        ));
        let mut req = request();
        req.request_id = format!("failure-{index}");
        service.search_start(req.clone()).unwrap();
        let terminal = (0..100)
            .find_map(|_| {
                let progress = service.search_progress(&req.request_id);
                if progress.as_ref().is_some_and(|progress| progress.terminal) {
                    progress
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    None
                }
            })
            .expect("failure must terminate");
        assert_eq!(terminal.stage, SearchProgressStage::Failed);
        let error = terminal.error.unwrap();
        assert_eq!(error.code, code);
        assert!(
            error.message_zh.contains(message_fragment),
            "{code}: {}",
            error.message_zh
        );
        assert_eq!(error.retryable, retryable, "{code}");
        service.shutdown().unwrap();
    }
}

/// #22 B slice 3(a): N kept by the default optical target while the
/// provider still had a continuation. Meeting the default target is a
/// normal result: frozen and reported without the possibly-incomplete flag.
#[test]
fn default_target_met_is_frozen_without_the_possibly_incomplete_flag() {
    let mut response = fixture();
    let template = response.products[0].clone();
    // Owner decision 2026-11-21: streaming optical sources stream up to 2,000 scenes.
    response.products = (0..2_000)
        .map(|index| {
            let mut product = template.clone();
            let (minute, second) = (index / 60, index % 60);
            product.product_id = format!("HLS.L30.T49RDP.2024177T03{minute:02}{second:02}.v2.0");
            product.display_name = product.product_id.clone();
            product.acquisition_datetime =
                Some(format!("2024-06-25T03:{minute:02}:{second:02}.017810Z"));
            product
        })
        .collect();
    let stats = &mut response.stats;
    stats.matched_count = Some(2_600);
    stats.fetched_record_count = 2_000;
    stats.mapped_record_count = 2_000;
    stats.unique_provider_product_count = 2_000;
    stats.exact_matched_count = 2_000;
    stats.returned_count = 2_000;
    stats.request_count = 1;
    stats.remote_exhausted = false;
    let db = TempStore::new();
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::new(Catalog {
            queries: Mutex::new(Vec::new()),
            response,
        }),
        Arc::new(NoDownload),
    ));
    service.search_start(request()).unwrap();
    let terminal = wait_terminal(&service, "unified-hls-1");
    assert_eq!(terminal.stage, SearchProgressStage::Done, "{terminal:?}");
    assert_eq!(terminal.items_kept, 2_000);
    assert!(!terminal.possibly_incomplete);
    let stats = service
        .restore(true)
        .unwrap()
        .workbench_batch
        .unwrap()
        .search_stats
        .unwrap();
    assert!(!stats.truncated, "a met default target is not a warning");
    assert_eq!(stats.frozen_candidate_count, 2_000);
    service.shutdown().unwrap();
}

struct StreamingOpticalCatalog {
    page_delay: std::time::Duration,
}

impl LiveCatalogSearch for StreamingOpticalCatalog {
    fn search_catalog(
        &self,
        _q: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        unreachable!("progressive test streams directly")
    }

    fn search_catalog_streaming(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
        sink: &mut dyn insar_service::CatalogSearchStreamSink,
        progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
    ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
        sink.attempt(query)?;
        let template = fixture().products.remove(0);
        let mut collection = fixture().collection;
        collection.logical_dataset = query.logical_dataset.clone();
        collection.provider_id = query.provider_id.clone();
        collection.native_collection_id = query.native_collection_id.clone();

        let make_products = |start: usize, count: usize| -> Vec<insar_core::ProviderProduct> {
            (start..start + count)
                .map(|index| {
                    let mut product = template.clone();
                    let (minute, second) = (index / 60, index % 60);
                    product.provider_id = query.provider_id.clone();
                    product.product_id =
                        format!("HLS.L30.T49RDP.2024177T04{minute:02}{second:02}.v2.0");
                    product.display_name = product.product_id.clone();
                    product.acquisition_datetime =
                        Some(format!("2024-06-25T04:{minute:02}:{second:02}.017810Z"));
                    product
                })
                .collect()
        };

        // Page 1: 250 scenes published immediately before slower page 2
        let page1 = make_products(0, 250);
        sink.accept(&page1)?;
        progress(
            insar_provider::asf::catalog_client::CatalogSearchProviderProgress {
                provider_request_count: 1,
                transport_part_count: 1,
                raw_fetched_count: 250,
                mapped_count: 250,
                unique_admitted_count: 250,
                exact_matched_count: 250,
                requested_limit: query.result_limit,
                remote_total: Some(600),
                elapsed_ms: 5,
            },
        );

        std::thread::sleep(self.page_delay);
        assert!(!cancelled());

        // Page 2: 200 more scenes (450 total <= 2,000 cap)
        let page2 = make_products(250, 200);
        sink.accept(&page2)?;
        progress(
            insar_provider::asf::catalog_client::CatalogSearchProviderProgress {
                provider_request_count: 2,
                transport_part_count: 1,
                raw_fetched_count: 450,
                mapped_count: 450,
                unique_admitted_count: 450,
                exact_matched_count: 450,
                requested_limit: query.result_limit,
                remote_total: Some(600),
                elapsed_ms: 35,
            },
        );

        let stats: insar_core::CatalogSearchStats = serde_json::from_value(json!({
            "requested_limit": query.result_limit,
            "matched_count": 450,
            "request_count": 2,
            "transport_part_count": 1,
            "fetched_record_count": 450,
            "mapped_record_count": 450,
            "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0,
            "unique_provider_product_count": 450,
            "exact_matched_count": 450,
            "returned_count": 450,
            "remote_exhausted": true,
            "working_set_satisfied": true,
            "elapsed_ms": 35,
            "truncated": false
        }))
        .unwrap();
        Ok(insar_service::CatalogSearchStreamOutcome::Streamed { collection, stats })
    }
}

#[test]
fn streaming_optical_search_publishes_first_250_then_completes_up_to_2000_cap() {
    let db = TempStore::new();
    let sync_events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = Arc::clone(&sync_events);
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::new(StreamingOpticalCatalog {
            page_delay: std::time::Duration::from_millis(80),
        }),
        Arc::new(NoDownload),
    ));
    service.register_search_sync_listener(Arc::new(move |payload| {
        captured.lock().unwrap().push(payload);
    }));
    service.search_start(request()).unwrap();

    // First 250 scenes must become progressively readable BEFORE terminal completion.
    let first_publish = (0..100)
        .find_map(|_| {
            let p = service.search_progress("unified-hls-1");
            if let Some(p) = p {
                if !p.terminal && p.batch_id.is_some() && p.loaded_count == Some(250) {
                    return Some(p);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            None
        })
        .expect("optical streaming must publish first 250 scenes before finishing");
    assert_eq!(first_publish.items_cap, 2_000);
    assert_eq!(first_publish.remote_total, Some(600));

    let terminal = wait_terminal(&service, "unified-hls-1");
    assert_eq!(terminal.stage, SearchProgressStage::Done, "{terminal:?}");
    assert_eq!(terminal.items_kept, 450);
    assert_eq!(terminal.items_cap, 2_000);
    assert_eq!(terminal.loaded_count, Some(450));

    let events = sync_events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|ev| ev["complete"] == json!(false) && ev["loaded_count"] == json!(250)),
        "expected incomplete 250-scene sync event: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|ev| ev["complete"] == json!(true) && ev["loaded_count"] == json!(450)),
        "expected final 450-scene sync event: {events:?}"
    );
    service.shutdown().unwrap();
}

fn wait_terminal(service: &ApplicationService, request_id: &str) -> insar_core::SearchProgress {
    (0..200)
        .find_map(|_| {
            let progress = service.search_progress(request_id);
            if progress.as_ref().is_some_and(|progress| progress.terminal) {
                progress
            } else {
                std::thread::sleep(std::time::Duration::from_millis(20));
                None
            }
        })
        .expect("search must terminate")
}

#[test]
fn source_view_keeps_successful_batch_and_persists_failure_across_restart() {
    let db = TempStore::new();
    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    service.search_start(request()).unwrap();
    assert_eq!(
        wait_terminal(&service, "unified-hls-1").stage,
        SearchProgressStage::Done
    );
    let first = service
        .source_views()
        .unwrap()
        .into_iter()
        .find(|view| view.resource_id == "science.hls.l30.v2")
        .unwrap();
    let batch_id = first
        .batch_id
        .clone()
        .expect("successful batch is the view");
    service.shutdown().unwrap();

    let failing = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::new(FailingCatalog {
            code: "CATALOGUE_TRANSPORT_FAILURE",
        }),
        Arc::new(NoDownload),
    ));
    let mut retry = request();
    retry.request_id = "unified-hls-2".to_owned();
    // A different frozen condition, so the catalogue cache cannot answer it.
    retry
        .filters
        .insert("cloud_cover_lte".to_owned(), json!(70.0));
    failing.search_start(retry).unwrap();
    assert_eq!(
        wait_terminal(&failing, "unified-hls-2").stage,
        SearchProgressStage::Failed
    );
    failing.shutdown().unwrap();

    let restarted = ApplicationService::new(ApplicationServiceConfig::store(&db.0));
    let view = restarted
        .source_views()
        .unwrap()
        .into_iter()
        .find(|view| view.resource_id == "science.hls.l30.v2")
        .unwrap();
    assert_eq!(view.batch_id.as_deref(), Some(batch_id.as_str()));
    assert_eq!(view.item_count, 1);
    assert_eq!(view.pending_request_id, None);
    assert_eq!(
        view.last_error.map(|error| error.code).as_deref(),
        Some("CATALOGUE_TRANSPORT_FAILURE"),
        "the failure is persisted, the old batch remains the view"
    );
    restarted.shutdown().unwrap();
}

#[test]
fn auxiliary_sources_return_typed_unsupported_progress_without_provider_io() {
    let db = TempStore::new();
    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    let auxiliary = service
        .list_sources()
        .unwrap()
        .into_iter()
        .filter(|source| source.search_mode == insar_core::SourceSearchMode::Auxiliary)
        .collect::<Vec<_>>();
    for family in [
        insar_core::SourceFamily::Orbit,
        insar_core::SourceFamily::Dem,
        insar_core::SourceFamily::Atmospheric,
    ] {
        assert!(
            auxiliary.iter().any(|source| source.family == family),
            "{family:?} has an auxiliary source"
        );
    }
    for (index, source) in auxiliary.iter().enumerate() {
        let mut req = request();
        req.request_id = format!("auxiliary-{index}");
        req.resource_id = source.resource_id.clone();
        req.filters = Map::new();
        let started = service
            .search_start(req.clone())
            .unwrap_or_else(|error| panic!("{}: untyped {error:?}", source.resource_id));
        assert!(started.terminal, "{}", source.resource_id);
        assert_eq!(started.stage, SearchProgressStage::Failed);
        let error = started.error.clone().expect("typed error");
        assert_eq!(error.code, "SEARCH_SOURCE_AUXILIARY_UNSUPPORTED");
        assert!(!error.retryable);
        assert!(!error.message_zh.is_empty());
        for term in ["统一检索", "Store", "unified"] {
            assert!(
                !error.message_zh.contains(term) && !started.message_zh.contains(term),
                "{}: implementation term {term} in {}",
                source.resource_id,
                error.message_zh
            );
        }
        assert_eq!(service.search_progress(&req.request_id), Some(started));
    }
    assert!(catalog.queries.lock().unwrap().is_empty());
    service.shutdown().unwrap();
}

#[test]
fn unreviewed_optical_filters_fail_before_background_provider_call() {
    let db = TempStore::new();
    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    let mut invalid = request();
    invalid
        .filters
        .insert("unreviewed_field".to_owned(), json!(true));
    assert!(service.search_start(invalid).is_err());
    assert!(catalog.queries.lock().unwrap().is_empty());
    service.shutdown().unwrap();
}

#[test]
fn restarted_service_surfaces_interrupted_batch_in_source_views_for_optical_and_typed_sar() {
    let db = TempStore::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        for (req_id, batch_id, resource_id, provider_id, logical_dataset, native_collection_id) in [
            (
                "req-hls-interrupt",
                "batch-hls-interrupted",
                "science.hls.l30.v2",
                "microsoft.planetary-computer",
                "optical.hls.l30.v2",
                "hls2-l30",
            ),
            (
                "req-nisar-interrupt",
                "batch-nisar-interrupted",
                "science.nisar.l.rslc.provisional.v1",
                "asf.nasa.earthdata",
                "sar.nisar.l.rslc.provisional.v1",
                "C2853145197-ASF",
            ),
        ] {
            store
                .mark_source_view_pending(resource_id, req_id, "2026-11-05T00:00:00Z")
                .unwrap();
            let batch_header = FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.to_owned(),
                provider_id: provider_id.to_owned(),
                created_at: "2026-11-05T00:00:01Z".to_owned(),
                aoi_name: "测试区".to_owned(),
                aoi_geojson: Some(aoi()),
                query: json!({
                    "logical_dataset": logical_dataset,
                    "provider_id": provider_id,
                    "native_collection_id": native_collection_id,
                })
                .as_object()
                .unwrap()
                .clone(),
                search_stats: None,
                candidates: Vec::new(),
            };
            let workbench = SearchWorkbenchState {
                batch_id: batch_id.to_owned(),
                checked_item_ids: Vec::new(),
                page_index: 0,
                page_size: 50,
                revision: 0,
            };
            store
                .begin_search_batch_staging(&insar_store::StoreSearchBatchStagingRequest {
                    request_id: req_id.to_owned(),
                    batch_header,
                    workbench,
                    max_candidates: 500,
                })
                .unwrap();
            let items = (0..250)
                .map(|idx| SnapshotItem {
                    item_id: format!("{batch_id}-item-{idx:03}"),
                    provider_id: provider_id.to_owned(),
                    display_name: format!("Scene {idx}"),
                    acquisition_datetime: Some("2026-10-01T00:00:00Z".to_owned()),
                    footprint_geojson: Some(aoi()),
                    metadata: Map::new(),
                    payload: Map::new(),
                    asset_access: None,
                    execution_policy: None,
                })
                .collect::<Vec<_>>();
            store
                .append_search_batch_staging_candidates(req_id, &items)
                .unwrap();
            store
                .publish_search_batch_staging_progressive(req_id, Some(500))
                .unwrap();
        }
    }

    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    let views = service.source_views().unwrap();
    let hls_view = views
        .iter()
        .find(|v| v.resource_id == "science.hls.l30.v2")
        .expect("HLS source view");
    assert_eq!(hls_view.batch_id.as_deref(), Some("batch-hls-interrupted"));
    assert_eq!(hls_view.item_count, 250);
    assert_eq!(hls_view.pending_request_id, None);

    let nisar_view = views
        .iter()
        .find(|v| v.resource_id == "science.nisar.l.rslc.provisional.v1")
        .expect("NISAR RSLC source view");
    assert_eq!(
        nisar_view.batch_id.as_deref(),
        Some("batch-nisar-interrupted")
    );
    assert_eq!(nisar_view.item_count, 250);
    assert_eq!(nisar_view.pending_request_id, None);

    let progress = service
        .search_batch_progress("batch-nisar-interrupted")
        .unwrap()
        .expect("interrupted batch progress");
    assert_eq!(
        progress.state,
        insar_core::SearchBatchProgressState::Interrupted
    );
    assert_eq!(progress.loaded_count, 250);
    assert_eq!(progress.remote_total, Some(500));
    service.shutdown().unwrap();
}

#[test]
fn invalid_source_fails_closed_without_provider_io() {
    let db = TempStore::new();
    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    let mut bad = request();
    bad.resource_id = "science.unknown.v1".to_owned();
    assert!(service.search_start(bad).is_err());
    assert!(catalog.queries.lock().unwrap().is_empty());
    service.shutdown().unwrap();
}

#[test]
fn sentinel2_l1c_and_sentinel3_standard_freeze_authoritative_snapshots_with_default_filters() {
    let rect_aoi = json!({
        "type": "Polygon",
        "coordinates": [[
            [116.2, 39.8],
            [116.5, 39.8],
            [116.5, 40.0],
            [116.2, 40.0],
            [116.2, 39.8]
        ]]
    });
    let s2_product_key = insar_core::provider_product_key(
        "copernicus.dataspace.odata",
        "SENTINEL-2",
        "e3183efa-2174-4df2-9757-5b7e04916753",
    )
    .unwrap();
    let s2_stable_asset_key = insar_core::stable_asset_key(&s2_product_key, "archive").unwrap();
    let s3_product_key = insar_core::provider_product_key(
        "copernicus.dataspace.odata",
        "SENTINEL-3",
        "e1ca9111-7d47-4c92-a490-c883db33fe12",
    )
    .unwrap();
    let s3_stable_asset_key = insar_core::stable_asset_key(&s3_product_key, "archive").unwrap();

    let s2_l1c_response: CatalogSearchResponse = serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.sentinel-2.l1c",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-2",
            "collection_profile_id": "cdse.sentinel-2.l1c.collection.v1",
            "identity_rule_id": "cdse.sentinel-2.l1c.identity.v1",
            "filename_rule_id": "cdse.sentinel-2.l1c.archive-filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": true
        },
        "products": [{
            "provider_id": "copernicus.dataspace.odata",
            "product_id": "e3183efa-2174-4df2-9757-5b7e04916753",
            "display_name": "S2B_MSIL1C_20240602T025529_N0510_R032_T50TMK_20240602T044702.SAFE",
            "source_kind": "sentinel2",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2024-06-02T02:55:29.024Z",
            "footprint_geojson": rect_aoi.clone(),
            "assets": [{
                "asset_key": "archive",
                "stable_asset_key": s2_stable_asset_key,
                "role": "primary",
                "native_roles": ["archive"],
                "size_bytes": 521297168
            }],
            "properties": {
                "product_type": "S2MSI1C",
                "online": true,
                "cloud_cover": 12.5,
                "checksums": [{"algorithm": "MD5", "value": "75f1cc48edef845e87c4dc86d415c396"}]
            },
            "payload": {}
        }],
        "stats": {
            "requested_limit": 250,
            "matched_count": 1,
            "request_count": 1,
            "transport_part_count": 1,
            "fetched_record_count": 1,
            "mapped_record_count": 1,
            "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0,
            "unique_provider_product_count": 1,
            "exact_matched_count": 1,
            "returned_count": 1,
            "remote_exhausted": true,
            "working_set_satisfied": true,
            "elapsed_ms": 5,
            "truncated": false
        },
        "item_errors": []
    }))
    .unwrap();

    let s3_response: CatalogSearchResponse = serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.sentinel-3.standard",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-3",
            "collection_profile_id": "cdse.sentinel-3.standard.collection.v1",
            "identity_rule_id": "cdse.sentinel-3.standard.identity.v1",
            "filename_rule_id": "cdse.sentinel-3.standard.archive-filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": true
        },
        "products": [{
            "provider_id": "copernicus.dataspace.odata",
            "product_id": "e1ca9111-7d47-4c92-a490-c883db33fe12",
            "display_name": "S3B_OL_2_LFR____20240602T020447_20240602T020747_20250705T231500_0179_093_331_2340_ESA_R_NT_003.SEN3",
            "source_kind": "sentinel2",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2024-06-02T02:04:47Z",
            "footprint_geojson": rect_aoi,
            "assets": [{
                "asset_key": "archive",
                "stable_asset_key": s3_stable_asset_key,
                "role": "primary",
                "native_roles": ["archive"],
                "size_bytes": 122160176
            }],
            "properties": {
                "product_type": "OL_2_LFR___",
                "online": true,
                "cloud_cover": 64.0,
                "checksums": [{"algorithm": "MD5", "value": "994acb41c0c33233c6ea90e0851b52b5"}]
            },
            "payload": {}
        }],
        "stats": {
            "requested_limit": 250,
            "matched_count": 1,
            "request_count": 1,
            "transport_part_count": 1,
            "fetched_record_count": 1,
            "mapped_record_count": 1,
            "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0,
            "unique_provider_product_count": 1,
            "exact_matched_count": 1,
            "returned_count": 1,
            "remote_exhausted": true,
            "working_set_satisfied": true,
            "elapsed_ms": 5,
            "truncated": false
        },
        "item_errors": []
    }))
    .unwrap();

    struct DualSentinelCatalog {
        s2_l1c: CatalogSearchResponse,
        s3_standard: CatalogSearchResponse,
        queries: Mutex<Vec<CatalogSearchQuery>>,
    }
    impl LiveCatalogSearch for DualSentinelCatalog {
        fn search_catalog(
            &self,
            q: &CatalogSearchQuery,
            _: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            self.queries.lock().unwrap().push(q.clone());
            match q.logical_dataset.as_str() {
                "optical.sentinel-2.l1c" => Ok(self.s2_l1c.clone()),
                "optical.sentinel-3.standard" => Ok(self.s3_standard.clone()),
                other => panic!("unexpected logical_dataset {other}"),
            }
        }
    }

    let db = TempStore::new();
    let catalog = Arc::new(DualSentinelCatalog {
        s2_l1c: s2_l1c_response,
        s3_standard: s3_response,
        queries: Mutex::new(Vec::new()),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));

    // 1. Sentinel-2 L1C with omitted product_type filter
    let s2_req: UnifiedSearchRequest = serde_json::from_value(json!({
        "request_id": "unified-s2-l1c-1",
        "resource_id": "science.sentinel-2.l1c.v1",
        "aoi": aoi(),
        "aoi_name": "北京测试区",
        "time_range": {
            "start": "2024-06-01T00:00:00Z",
            "end": "2024-06-05T00:00:00Z"
        },
        "filters": {}
    }))
    .unwrap();
    service
        .search_start(s2_req)
        .expect("S2 L1C search admitted");
    let s2_terminal = (0..150)
        .find_map(|_| {
            let p = service.search_progress("unified-s2-l1c-1");
            if p.as_ref().is_some_and(|p| p.terminal) {
                p
            } else {
                std::thread::sleep(std::time::Duration::from_millis(20));
                None
            }
        })
        .expect("S2 L1C terminal");
    assert_eq!(
        s2_terminal.stage,
        SearchProgressStage::Done,
        "{s2_terminal:?}"
    );
    assert_eq!(s2_terminal.items_kept, 1);
    let s2_batch = service.restore(true).unwrap().workbench_batch.unwrap();
    assert_eq!(s2_batch.schema_version, 1);
    assert_eq!(
        s2_batch
            .query
            .get("logical_dataset")
            .and_then(Value::as_str),
        Some("optical.sentinel-2.l1c")
    );
    assert_eq!(s2_batch.candidates.len(), 1);
    assert_eq!(
        s2_batch.candidates[0]
            .metadata
            .get("collection_profile_id")
            .and_then(Value::as_str),
        Some("cdse.sentinel-2.l1c.collection.v1")
    );

    // 2. Sentinel-3 Standard with omitted product_type filter
    let s3_req: UnifiedSearchRequest = serde_json::from_value(json!({
        "request_id": "unified-s3-std-1",
        "resource_id": "science.sentinel-3.standard.v1",
        "aoi": aoi(),
        "aoi_name": "北京测试区",
        "time_range": {
            "start": "2024-06-01T00:00:00Z",
            "end": "2024-06-05T00:00:00Z"
        },
        "filters": {}
    }))
    .unwrap();
    service
        .search_start(s3_req)
        .expect("S3 Standard search admitted");
    let s3_terminal = (0..150)
        .find_map(|_| {
            let p = service.search_progress("unified-s3-std-1");
            if p.as_ref().is_some_and(|p| p.terminal) {
                p
            } else {
                std::thread::sleep(std::time::Duration::from_millis(20));
                None
            }
        })
        .expect("S3 Standard terminal");
    assert_eq!(
        s3_terminal.stage,
        SearchProgressStage::Done,
        "{s3_terminal:?}"
    );
    assert_eq!(s3_terminal.items_kept, 1);
    let s3_batch = service.restore(true).unwrap().workbench_batch.unwrap();
    assert_eq!(s3_batch.schema_version, 1);
    assert_eq!(
        s3_batch
            .query
            .get("logical_dataset")
            .and_then(Value::as_str),
        Some("optical.sentinel-3.standard")
    );
    assert_eq!(s3_batch.candidates.len(), 1);
    assert_eq!(
        s3_batch.candidates[0]
            .metadata
            .get("collection_profile_id")
            .and_then(Value::as_str),
        Some("cdse.sentinel-3.standard.collection.v1")
    );

    service.shutdown().unwrap();

    // Verify physical SQLite persistence via independent TaskStore::open
    let reopened = TaskStore::open(&db.0).unwrap();
    let persisted = reopened
        .load_search_batch(&s3_batch.batch_id)
        .unwrap()
        .expect("persisted S3 batch in SQLite");
    assert_eq!(persisted.schema_version, 1);
    assert_eq!(persisted.candidates.len(), 1);
    let persisted_s2 = reopened
        .load_search_batch(&s2_batch.batch_id)
        .unwrap()
        .expect("persisted S2 L1C batch in SQLite");
    assert_eq!(persisted_s2.schema_version, 1);
    assert_eq!(persisted_s2.candidates.len(), 1);
}

#[test]
fn landsat_c2_l2_unified_start_supplies_restore_all_defaults_when_omitted() {
    let landsat_response: CatalogSearchResponse = serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.landsat.c2.l2",
            "provider_id": "microsoft.planetary-computer",
            "native_collection_id": "landsat-c2-l2",
            "collection_profile_id": "planetary-computer.landsat-c2-l2.scene.v1",
            "identity_rule_id": "planetary-computer.landsat-c2-l2.identity.v1",
            "filename_rule_id": "planetary-computer.landsat-c2-l2.asset-filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": true
        },
        "products": [],
        "stats": {
            // Owner decision 2026-11-21: optical streaming routes request 2,000 after first 250.
            "requested_limit": 2000,
            "matched_count": 0,
            "request_count": 1,
            "transport_part_count": 1,
            "fetched_record_count": 0,
            "mapped_record_count": 0,
            "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0,
            "unique_provider_product_count": 0,
            "exact_matched_count": 0,
            "returned_count": 0,
            "remote_exhausted": true,
            "working_set_satisfied": true,
            "elapsed_ms": 2,
            "truncated": false
        },
        "item_errors": []
    }))
    .unwrap();

    let db = TempStore::new();
    let catalog = Arc::new(Catalog {
        queries: Mutex::new(Vec::new()),
        response: landsat_response,
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(NoAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoDownload),
    ));
    let req: UnifiedSearchRequest = serde_json::from_value(json!({
        "request_id": "unified-landsat-defaults-1",
        "resource_id": "science.landsat.c2-l2.v1",
        "aoi": aoi(),
        "aoi_name": "四川省成都市锦江区",
        "time_range": {
            "start": "2024-09-01T00:00:00Z",
            "end": "2024-11-30T23:59:59Z"
        },
        "filters": {
            "cloud_cover_lte": 40.0
        }
    }))
    .unwrap();
    service
        .search_start(req)
        .expect("Landsat C2 L2 admitted with RestoreAll defaults filled");
    let terminal = wait_terminal(&service, "unified-landsat-defaults-1");
    assert_eq!(terminal.stage, SearchProgressStage::Done, "{terminal:?}");
    assert_eq!(terminal.items_kept, 0);
    let queries = catalog.queries.lock().unwrap();
    assert_eq!(queries.len(), 1);
    assert_eq!(
        queries[0].filters.get("spacecraft"),
        Some(&json!([
            "landsat_4",
            "landsat_5",
            "landsat_7",
            "landsat_8",
            "landsat_9"
        ]))
    );
    assert_eq!(
        queries[0].filters.get("processing_levels"),
        Some(&json!(["l2sp", "l2sr"]))
    );
    assert_eq!(
        queries[0].filters.get("cloud_cover_lte"),
        Some(&json!(40.0))
    );
    drop(queries);
    service.shutdown().unwrap();
}

#[test]
fn sentinel2_l1c_and_sentinel3_standard_live_public_cdse_search_persists_snapshots() {
    let beijing_aoi = json!({
        "type": "Polygon",
        "coordinates": [[[116.2, 39.8], [116.6, 39.8], [116.6, 40.1], [116.2, 40.1], [116.2, 39.8]]]
    })
    .as_object()
    .unwrap()
    .clone();

    let db = TempStore::new();
    // Real Production runtime with ZERO CDSE credentials configured
    let service = Arc::new(ApplicationService::new(
        ApplicationServiceConfig::business_store(&db.0),
    ));

    for (req_id, resource_id, expected_logical_dataset, expected_policy_id) in [
        (
            "live-s2-l1c-1",
            "science.sentinel-2.l1c.v1",
            "optical.sentinel-2.l1c",
            "cdse.sentinel-2.l1c.archive-resolver-policy.v1",
        ),
        (
            "live-s3-std-1",
            "science.sentinel-3.standard.v1",
            "optical.sentinel-3.standard",
            "cdse.sentinel-3.standard.archive-resolver-policy.v1",
        ),
    ] {
        let req: UnifiedSearchRequest = serde_json::from_value(json!({
            "request_id": req_id,
            "resource_id": resource_id,
            "aoi": beijing_aoi,
            "aoi_name": "北京市主城区",
            "time_range": {
                "start": "2024-06-01T00:00:00Z",
                "end": "2024-06-05T00:00:00Z"
            },
            "filters": {}
        }))
        .unwrap();
        service
            .search_start(req)
            .expect("live unified search admitted without credentials");
        let terminal = (0..600)
            .find_map(|_| {
                let p = service.search_progress(req_id);
                if p.as_ref().is_some_and(|p| p.terminal) {
                    p
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    None
                }
            })
            .expect("live unified search reached terminal state");
        assert_eq!(
            terminal.stage,
            SearchProgressStage::Done,
            "live {resource_id} failed: {terminal:?}"
        );
        assert!(
            terminal.items_kept >= 1,
            "expected >=1 live item for {resource_id}, got {terminal:?}"
        );
        let batch_id = terminal.batch_id.as_deref().expect("persisted batch_id");
        let batch = service.restore(true).unwrap().workbench_batch.unwrap();
        assert_eq!(batch.batch_id, batch_id);
        assert_eq!(batch.schema_version, 1);
        assert_eq!(
            batch.query.get("logical_dataset").and_then(Value::as_str),
            Some(expected_logical_dataset)
        );
        assert!(!batch.candidates.is_empty());
        assert_eq!(
            batch.candidates[0]
                .execution_policy
                .as_ref()
                .expect("production execution policy")
                .policy_id,
            expected_policy_id
        );
    }

    service.shutdown().unwrap();
}
