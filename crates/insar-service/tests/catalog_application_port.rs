use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery, CatalogSearchResponse,
    ContractError, ProviderNetworkPolicy, ProviderResult, SarRemoteSearchRequest,
    SarRemoteSearchResponse,
};
use insar_provider::catalog::{
    RouteUse, BHOONIDHI_LOGICAL_DATASET, BHOONIDHI_NATIVE_COLLECTION_ID, BHOONIDHI_PROVIDER_ID,
};
use insar_service::{
    CatalogSourceAttemptOutcome, CatalogSourceFallbackReason, Dispatcher, LiveAsfSearch,
    LiveCatalogSearch, ProductionCatalogRouter,
};
use serde_json::json;

#[derive(Default)]
struct FixtureAsfSearch {
    sar_calls: AtomicUsize,
}

struct DuplicateAsfSearch {
    entered: AtomicBool,
    release: AtomicBool,
    calls: AtomicUsize,
}

impl LiveAsfSearch for FixtureAsfSearch {
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
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.sar_calls.fetch_add(1, Ordering::SeqCst);
        Err(ContractError::new(
            "ASF_FIXTURE_REACHED",
            "fixture ASF search reached",
            Default::default(),
        )
        .unwrap())
    }
}

impl LiveAsfSearch for DuplicateAsfSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        unreachable!("duplicate SAR fixture does not serve legacy ASF search")
    }

    fn search_sar(
        &self,
        _request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call > 0 {
            return Err(ContractError::new(
                "ASF_DUPLICATE_REACHED_PROVIDER",
                "duplicate fixture reached the provider",
                Default::default(),
            )
            .unwrap());
        }
        self.entered.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) && !cancelled() {
            thread::sleep(Duration::from_millis(2));
        }
        Err(ContractError::new(
            "ASF_FIRST_REQUEST_RELEASED",
            "first fixture request released",
            Default::default(),
        )
        .unwrap())
    }
}

struct FixtureCatalog {
    entered: AtomicBool,
    release: AtomicBool,
    calls: AtomicUsize,
}

impl FixtureCatalog {
    fn ready() -> Self {
        Self {
            entered: AtomicBool::new(false),
            release: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
        }
    }

    fn blocking() -> Self {
        Self {
            entered: AtomicBool::new(false),
            release: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        }
    }
}

impl LiveCatalogSearch for FixtureCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            if cancelled() {
                return Err(ContractError::new(
                    "CATALOGUE_REQUEST_CANCELLED",
                    "fixture catalogue cancelled",
                    Default::default(),
                )
                .unwrap());
            }
            thread::sleep(Duration::from_millis(2));
        }
        if cancelled() {
            return Err(ContractError::new(
                "CATALOGUE_REQUEST_CANCELLED",
                "fixture catalogue cancelled",
                Default::default(),
            )
            .unwrap());
        }
        Ok(response())
    }
}

struct FallbackCatalog {
    calls: Mutex<Vec<String>>,
    first_error: ContractError,
}

impl FallbackCatalog {
    fn new(code: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            first_error: ContractError::new(code, "fixture provider failure", Default::default())
                .unwrap(),
        }
    }
}

impl LiveCatalogSearch for FallbackCatalog {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        let mut calls = self.calls.lock().unwrap();
        calls.push(query.provider_id.clone());
        if calls.len() == 1 {
            return Err(self.first_error.clone());
        }
        Ok(response())
    }
}

fn mpc_landsat_query() -> CatalogSearchQuery {
    let mut query = landsat_query();
    query.provider_id = "microsoft.planetary-computer".to_owned();
    query.native_collection_id = "landsat-c2-l2".to_owned();
    query
}

fn query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "optical.sentinel-2.l2a",
        "provider_id": "copernicus.dataspace.odata",
        "native_collection_id": "SENTINEL-2",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        },
        "result_limit": 2,
        "filters": {
            "product_type": "S2MSI2A",
            "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"}
        }
    }))
    .unwrap()
}

fn landsat_query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": "optical.landsat.c2.l2",
        "provider_id": "usgs.eros.m2m",
        "native_collection_id": "landsat_ot_c2_l2",
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        },
        "result_limit": 2,
        "filters": {
            "cloud_cover_lte": 40.0,
            "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"},
            "spacecraft": ["landsat_8", "landsat_9"],
            "processing_levels": ["l2sp", "l2sr"]
        }
    }))
    .unwrap()
}

fn typed_query(
    logical_dataset: &str,
    provider_id: &str,
    native_collection_id: &str,
) -> CatalogSearchQuery {
    let mut query = query();
    query.logical_dataset = logical_dataset.to_owned();
    query.provider_id = provider_id.to_owned();
    query.native_collection_id = native_collection_id.to_owned();
    query
}

fn bhoonidhi_query() -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": BHOONIDHI_LOGICAL_DATASET,
        "provider_id": BHOONIDHI_PROVIDER_ID,
        "native_collection_id": BHOONIDHI_NATIVE_COLLECTION_ID,
        "aoi_geojson": {
            "type": "Polygon",
            "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
        },
        "result_limit": 2,
        "filters": {
            "online": "Y",
            "mode": "W",
            "polarization": "CP",
            "datetime": {"start": "2026-01-01T00:00:00Z", "end": "2026-01-02T00:00:00Z"}
        }
    }))
    .unwrap()
}

fn sar_request() -> SarRemoteSearchRequest {
    serde_json::from_value(json!({
        "query": {"dataset": "ERS"}
    }))
    .expect("minimal SAR request")
}

fn response() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.sentinel-2.l2a",
            "provider_id": "copernicus.dataspace.odata",
            "native_collection_id": "SENTINEL-2",
            "collection_profile_id": "cdse.sentinel-2.l2a.collection.v1",
            "identity_rule_id": "cdse.sentinel-2.l2a.identity.v1",
            "filename_rule_id": "cdse.sentinel-2.l2a.archive-filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": false
        },
        "products": [],
        "stats": {
            "requested_limit": 2,
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
            "elapsed_ms": 0,
            "truncated": false
        },
        "item_errors": []
    }))
    .unwrap()
}

#[test]
fn dispatcher_without_catalogue_port_remains_unavailable() {
    let dispatcher =
        Dispatcher::with_asf_search_for_application(Arc::new(FixtureAsfSearch::default()));
    let error = match dispatcher.search_catalog_for_application("missing-port", &query()) {
        Ok(_) => panic!("catalogue port must remain unavailable"),
        Err(error) => error,
    };
    assert_eq!(error.code, "CATALOGUE_CAPABILITY_UNAVAILABLE");
}

#[test]
fn production_constructor_registers_the_fixed_cdse_route_without_network_io() {
    let dispatcher =
        Dispatcher::try_new_for_application_with_network_policy(ProviderNetworkPolicy::direct())
            .expect("production construction is static and must not contact CDSE");
    assert_eq!(
        dispatcher
            .catalogue_route_use()
            .expect("production catalogue route"),
        RouteUse::Production
    );

    for (label, invalid) in [
        ("unknown-dataset", {
            let mut value = query();
            value.logical_dataset = "optical.sentinel-3.olci".to_owned();
            value
        }),
        ("provider-mismatch", {
            let mut value = query();
            value.provider_id = "asf.catalog".to_owned();
            value
        }),
        ("collection-mismatch", {
            let mut value = query();
            value.native_collection_id = "SENTINEL-1".to_owned();
            value
        }),
    ] {
        let error = match dispatcher
            .search_catalog_for_application(&format!("production-{label}"), &invalid)
        {
            Ok(_) => panic!("invalid routes must fail before an HTTP request: {label}"),
            Err(error) => error,
        };
        assert_eq!(error.code, "CATALOGUE_ROUTE_UNAVAILABLE", "{label}");
    }
}

#[test]
fn production_catalog_router_dispatches_only_the_two_exact_optical_datasets() {
    let cdse = Arc::new(FixtureCatalog::ready());
    let usgs = Arc::new(FixtureCatalog::ready());
    let asf = Arc::new(FixtureCatalog::ready());
    let router = ProductionCatalogRouter::new(cdse.clone(), usgs.clone(), asf.clone());

    router.search_catalog(&query(), &|| false).unwrap();
    assert_eq!(cdse.calls.load(Ordering::SeqCst), 1);
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 0);

    router.search_catalog(&landsat_query(), &|| false).unwrap();
    assert_eq!(cdse.calls.load(Ordering::SeqCst), 1);
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 1);

    for mut invalid in [query(), landsat_query()] {
        invalid.provider_id = "wrong.provider".to_owned();
        let error = router.search_catalog(&invalid, &|| false).unwrap_err();
        assert_eq!(error.code, "CATALOGUE_ROUTE_UNAVAILABLE");
    }
    assert_eq!(cdse.calls.load(Ordering::SeqCst), 1);
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn typed_alos2_and_nisar_routes_reach_only_the_asf_catalogue_port() {
    let cdse = Arc::new(FixtureCatalog::ready());
    let usgs = Arc::new(FixtureCatalog::ready());
    let asf = Arc::new(FixtureCatalog::ready());
    let router = ProductionCatalogRouter::new(cdse.clone(), usgs.clone(), asf.clone());
    for (logical_dataset, provider_id, collection_id) in [
        (
            "sar.alos-2.palsar-2.scansar.l1.1.v1",
            "asf.nasa.earthdata",
            "C3315903479-ASF",
        ),
        (
            "sar.nisar.l.rslc.provisional.v1",
            "asf.nasa.earthdata",
            "C2853145197-ASF",
        ),
    ] {
        router
            .search_catalog(
                &typed_query(logical_dataset, provider_id, collection_id),
                &|| false,
            )
            .unwrap();
    }
    assert_eq!(asf.calls.load(Ordering::SeqCst), 2);
    assert_eq!(cdse.calls.load(Ordering::SeqCst), 0);
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 0);

    let mut wrong = typed_query(
        "sar.alos-2.palsar-2.scansar.l1.1.v1",
        "asf.nasa.earthdata",
        "C3315903479-ASF",
    );
    wrong.provider_id = "asf.generic".to_owned();
    assert_eq!(
        router.search_catalog(&wrong, &|| false).unwrap_err().code,
        "CATALOGUE_ROUTE_UNAVAILABLE"
    );
    assert_eq!(asf.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn landsat_microsoft_and_usgs_requests_never_alias_the_same_runtime_port() {
    let cdse = Arc::new(FixtureCatalog::ready());
    let usgs = Arc::new(FixtureCatalog::ready());
    let asf = Arc::new(FixtureCatalog::ready());
    let microsoft = Arc::new(FixtureCatalog::ready());
    let router = ProductionCatalogRouter::new(cdse.clone(), usgs.clone(), asf.clone())
        .with_planetary_computer(microsoft.clone());
    let mut native = landsat_query();
    native.provider_id = insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID.to_owned();
    native.native_collection_id = insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID.to_owned();
    router.search_catalog(&native, &|| false).unwrap();
    assert_eq!(microsoft.calls.load(Ordering::SeqCst), 1);
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 0);
    router.search_catalog(&landsat_query(), &|| false).unwrap();
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 1);
    assert_eq!(microsoft.calls.load(Ordering::SeqCst), 1);
    for mut mixed in [native.clone(), landsat_query()] {
        mixed.native_collection_id = if mixed.provider_id == native.provider_id {
            landsat_query().native_collection_id
        } else {
            native.native_collection_id.clone()
        };
        assert_eq!(
            router.search_catalog(&mixed, &|| false).unwrap_err().code,
            "CATALOGUE_ROUTE_UNAVAILABLE"
        );
    }
    assert_eq!(microsoft.calls.load(Ordering::SeqCst), 1);
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 1);
    assert_eq!(cdse.calls.load(Ordering::SeqCst), 0);
    assert_eq!(asf.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_microsoft_or_usgs_runtime_does_not_enable_cross_source_fallback() {
    let cdse = Arc::new(FixtureCatalog::ready());
    let usgs = Arc::new(FixtureCatalog::ready());
    let asf = Arc::new(FixtureCatalog::ready());
    let microsoft = Arc::new(FixtureCatalog::ready());
    let mut native = landsat_query();
    native.provider_id = insar_provider::catalog::MPC_LANDSAT_PROVIDER_ID.to_owned();
    native.native_collection_id = insar_provider::catalog::MPC_LANDSAT_COLLECTION_ID.to_owned();
    let old_only = ProductionCatalogRouter::new(cdse.clone(), usgs.clone(), asf.clone());
    assert_eq!(
        old_only
            .search_catalog(&native, &|| false)
            .unwrap_err()
            .code,
        "CATALOGUE_ROUTE_UNAVAILABLE"
    );
    let native_only = ProductionCatalogRouter::without_landsat(cdse.clone(), asf.clone())
        .with_planetary_computer(microsoft.clone());
    assert_eq!(
        native_only
            .search_catalog(&landsat_query(), &|| false)
            .unwrap_err()
            .code,
        "CATALOGUE_ROUTE_UNAVAILABLE"
    );
    assert_eq!(microsoft.calls.load(Ordering::SeqCst), 0);
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 0);
    assert_eq!(cdse.calls.load(Ordering::SeqCst), 0);
    assert_eq!(asf.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn typed_bhoonidhi_route_requires_injected_port_and_never_uses_asf() {
    let cdse = Arc::new(FixtureCatalog::ready());
    let asf = Arc::new(FixtureCatalog::ready());
    let bhoonidhi = Arc::new(FixtureCatalog::ready());
    let router = ProductionCatalogRouter::without_landsat(cdse.clone(), asf.clone())
        .with_bhoonidhi(bhoonidhi.clone());

    router
        .search_catalog(&bhoonidhi_query(), &|| false)
        .expect("injected Bhoonidhi route reaches its typed port");
    assert_eq!(bhoonidhi.calls.load(Ordering::SeqCst), 1);
    assert_eq!(asf.calls.load(Ordering::SeqCst), 0);
    assert_eq!(cdse.calls.load(Ordering::SeqCst), 0);

    let unavailable = ProductionCatalogRouter::without_landsat(cdse.clone(), asf.clone())
        .search_catalog(&bhoonidhi_query(), &|| false)
        .expect_err("Bhoonidhi stays unavailable without an injected client");
    assert_eq!(unavailable.code, "CATALOGUE_ROUTE_UNAVAILABLE");
    assert_eq!(asf.calls.load(Ordering::SeqCst), 0);
}

/// Issue #3 R2 B5 review: the HLS arm reaches only its injected port; without
/// it the route is unavailable and no other provider is tried.
#[test]
fn hls_route_requires_its_injected_port_and_never_uses_another_provider() {
    let cdse = Arc::new(FixtureCatalog::ready());
    let asf = Arc::new(FixtureCatalog::ready());
    let reviewed = Arc::new(FixtureCatalog::ready());
    let hls = Arc::new(FixtureCatalog::ready());
    let mut query = landsat_query();
    query.logical_dataset = "optical.hls.l30.v2".to_owned();
    query.provider_id = "microsoft.planetary-computer".to_owned();
    query.native_collection_id = "hls2-l30".to_owned();
    let router = ProductionCatalogRouter::without_landsat(cdse.clone(), asf.clone())
        .with_mpc_reviewed(reviewed.clone())
        .with_mpc_hls(hls.clone());
    router
        .search_catalog(&query, &|| false)
        .expect("injected HLS route reaches its port");
    assert_eq!(hls.calls.load(Ordering::SeqCst), 1);

    // A collection that does not belong to the logical dataset is not HLS.
    let mut crossed = query.clone();
    crossed.native_collection_id = "hls2-s30".to_owned();
    assert_eq!(
        router.search_catalog(&crossed, &|| false).unwrap_err().code,
        "CATALOGUE_ROUTE_UNAVAILABLE"
    );
    assert_eq!(hls.calls.load(Ordering::SeqCst), 1);

    let unavailable = ProductionCatalogRouter::without_landsat(cdse.clone(), asf.clone())
        .with_mpc_reviewed(reviewed.clone())
        .search_catalog(&query, &|| false)
        .expect_err("HLS stays unavailable without an injected client");
    assert_eq!(unavailable.code, "CATALOGUE_ROUTE_UNAVAILABLE");
    for other in [&cdse, &asf, &reviewed] {
        assert_eq!(other.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn typed_application_search_never_enters_generic_asf_search() {
    let asf = Arc::new(FixtureAsfSearch::default());
    let catalogue = Arc::new(FixtureCatalog::ready());
    let dispatcher =
        Dispatcher::with_production_catalogue_for_application(asf.clone(), catalogue.clone());
    let operation = dispatcher
        .search_catalog_for_application(
            "typed-alos2",
            &typed_query(
                "sar.alos-2.palsar-2.scansar.l1.1.v1",
                "asf.nasa.earthdata",
                "C3315903479-ASF",
            ),
        )
        .unwrap();
    assert_eq!(operation.response.stats.returned_count, 0);
    assert_eq!(catalogue.calls.load(Ordering::SeqCst), 1);
    assert_eq!(asf.sar_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn production_catalogue_route_accepts_only_exact_sentinel2_and_preserves_asf_dispatch() {
    let asf = Arc::new(FixtureAsfSearch::default());
    let catalogue = Arc::new(FixtureCatalog::ready());
    let dispatcher =
        Dispatcher::with_production_catalogue_for_application(asf.clone(), catalogue.clone());

    let operation = dispatcher
        .search_catalog_for_application("exact-sentinel2", &query())
        .expect("the exact Sentinel-2 route reaches only the catalogue port");
    assert_eq!(operation.response.stats.returned_count, 0);
    assert_eq!(catalogue.calls.load(Ordering::SeqCst), 1);
    drop(operation);

    for (label, invalid) in [
        ("unknown-dataset", {
            let mut value = query();
            value.logical_dataset = "optical.sentinel-3.olci".to_owned();
            value
        }),
        ("provider-mismatch", {
            let mut value = query();
            value.provider_id = "asf.catalog".to_owned();
            value
        }),
        ("collection-mismatch", {
            let mut value = query();
            value.native_collection_id = "SENTINEL-1".to_owned();
            value
        }),
    ] {
        let calls_before = catalogue.calls.load(Ordering::SeqCst);
        let error =
            match dispatcher.search_catalog_for_application(&format!("closed-{label}"), &invalid) {
                Ok(_) => panic!("mismatched routes must not fall back to any provider: {label}"),
                Err(error) => error,
            };
        assert_eq!(error.code, "CATALOGUE_ROUTE_UNAVAILABLE", "{label}");
        assert_eq!(
            catalogue.calls.load(Ordering::SeqCst),
            calls_before,
            "{label}"
        );
    }

    let error = dispatcher
        .search_sar_for_application("asf-port", &sar_request())
        .expect_err("adding CDSE catalogue routing must not replace ASF dispatch");
    assert_eq!(error.code, "ASF_FIXTURE_REACHED");
    assert_eq!(asf.sar_calls.load(Ordering::SeqCst), 1);
    assert_eq!(catalogue.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn duplicate_active_catalogue_request_is_rejected_without_overwrite() {
    let catalogue = Arc::new(FixtureCatalog::blocking());
    let dispatcher = Arc::new(Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        catalogue.clone(),
    ));
    let catalogue_query = query();
    let worker_dispatcher = Arc::clone(&dispatcher);
    let handle = thread::spawn(move || {
        worker_dispatcher.search_catalog_for_application("duplicate-id", &catalogue_query)
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !catalogue.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(catalogue.entered.load(Ordering::Acquire));
    let error = match dispatcher.search_catalog_for_application("duplicate-id", &query()) {
        Ok(_) => panic!("duplicate catalogue request must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code, "CATALOGUE_REQUEST_DUPLICATE");
    assert!(dispatcher.cancel_request("duplicate-id"));
    let error = match handle.join().unwrap() {
        Ok(_) => panic!("cancelled catalogue request must fail"),
        Err(error) => error,
    };
    assert_eq!(error.code, "CATALOGUE_REQUEST_CANCELLED");
}

#[test]
fn duplicate_active_sar_request_is_rejected_before_a_second_provider_call() {
    let asf = Arc::new(DuplicateAsfSearch {
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    let dispatcher = Arc::new(Dispatcher::with_asf_search_for_application(asf.clone()));
    let worker = Arc::clone(&dispatcher);
    let handle = thread::spawn(move || {
        worker.search_sar_for_application("duplicate-sar-id", &sar_request())
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !asf.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(asf.entered.load(Ordering::Acquire));

    let duplicate = dispatcher
        .search_sar_for_application("duplicate-sar-id", &sar_request())
        .expect_err("duplicate SAR request must fail before provider dispatch");
    asf.release.store(true, Ordering::Release);
    let first = handle.join().unwrap().unwrap_err();

    assert_eq!(duplicate.code, "ASF_REQUEST_DUPLICATE");
    assert_eq!(asf.calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.code, "ASF_FIRST_REQUEST_RELEASED");
}

#[test]
fn one_catalogue_lease_covers_bounded_provider_fallback() {
    let catalogue = Arc::new(FallbackCatalog::new("TRANSIENT_FIXTURE"));
    let dispatcher = Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        catalogue.clone(),
    );
    let queries = vec![mpc_landsat_query(), landsat_query()];
    let registered = dispatcher
        .register_catalog_search_candidates_for_application("fallback-success", &queries)
        .unwrap();
    let operation = registered
        .execute_candidates_with_progress(
            &|error| {
                (error.code == "TRANSIENT_FIXTURE")
                    .then_some(CatalogSourceFallbackReason::TransientNetwork)
            },
            &|_| {},
        )
        .unwrap();
    assert_eq!(operation.query.provider_id, "usgs.eros.m2m");
    assert_eq!(operation.source_attempts.len(), 2);
    assert_eq!(
        operation.source_attempts[0].outcome,
        CatalogSourceAttemptOutcome::Fallback(CatalogSourceFallbackReason::TransientNetwork)
    );
    assert_eq!(
        operation.source_attempts[0].provider_id,
        "microsoft.planetary-computer"
    );
    assert_eq!(
        operation.source_attempts[1].outcome,
        CatalogSourceAttemptOutcome::Succeeded
    );
    assert_eq!(operation.source_attempts[1].provider_id, "usgs.eros.m2m");
    assert_eq!(
        catalogue.calls.lock().unwrap().as_slice(),
        ["microsoft.planetary-computer", "usgs.eros.m2m"]
    );
    assert!(dispatcher.cancel_request("fallback-success"));
    drop(operation);
    assert!(!dispatcher.cancel_request("fallback-success"));
}

#[test]
fn terminal_provider_failure_never_enters_backup_source() {
    let catalogue = Arc::new(FallbackCatalog::new("IDENTITY_FIXTURE"));
    let dispatcher = Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        catalogue.clone(),
    );
    let registered = dispatcher
        .register_catalog_search_candidates_for_application(
            "fallback-terminal",
            &[mpc_landsat_query(), landsat_query()],
        )
        .unwrap();
    let error = match registered.execute_candidates_with_progress(
        &|error| {
            (error.code == "TRANSIENT_FIXTURE")
                .then_some(CatalogSourceFallbackReason::TransientNetwork)
        },
        &|_| {},
    ) {
        Ok(_) => panic!("terminal provider failure must not publish a result"),
        Err(error) => error,
    };
    assert_eq!(error.code, "IDENTITY_FIXTURE");
    assert_eq!(
        catalogue.calls.lock().unwrap().as_slice(),
        ["microsoft.planetary-computer"]
    );
    assert!(!dispatcher.cancel_request("fallback-terminal"));
}

#[test]
fn cancellation_between_failed_primary_and_backup_prevents_second_provider_call() {
    let catalogue = Arc::new(FallbackCatalog::new("TRANSIENT_FIXTURE"));
    let dispatcher = Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        catalogue.clone(),
    );
    let registered = dispatcher
        .register_catalog_search_candidates_for_application(
            "fallback-cancel",
            &[mpc_landsat_query(), landsat_query()],
        )
        .unwrap();
    let error = match registered.execute_candidates_with_progress(
        &|_| {
            assert!(dispatcher.cancel_request("fallback-cancel"));
            Some(CatalogSourceFallbackReason::TransientNetwork)
        },
        &|_| {},
    ) {
        Ok(_) => panic!("cancelled fallback must not publish a result"),
        Err(error) => error,
    };
    assert_eq!(error.code, "CATALOGUE_REQUEST_CANCELLED");
    assert_eq!(
        catalogue.calls.lock().unwrap().as_slice(),
        ["microsoft.planetary-computer"]
    );
    assert!(!dispatcher.cancel_request("fallback-cancel"));
}

#[test]
fn catalogue_lease_arbitrates_cancel_and_publication_then_cleans_up() {
    let dispatcher = Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        Arc::new(FixtureCatalog::ready()),
    );
    let operation = dispatcher
        .search_catalog_for_application("cancel-before-publication", &query())
        .unwrap();
    assert!(dispatcher.cancel_request("cancel-before-publication"));
    let error = operation.lease.begin_publication().unwrap_err();
    assert_eq!(error.code, "CATALOGUE_REQUEST_CANCELLED");
    drop(operation);
    assert!(!dispatcher.cancel_request("cancel-before-publication"));

    let operation = dispatcher
        .search_catalog_for_application("publish-wins", &query())
        .unwrap();
    operation.lease.begin_publication().unwrap();
    assert!(!dispatcher.cancel_request("publish-wins"));
    drop(operation);
    assert!(!dispatcher.cancel_request("publish-wins"));
}

#[test]
fn parent_cancellation_before_nested_execution_preserves_other_leases_and_duplicate_guards() {
    let catalogue = Arc::new(FixtureCatalog::ready());
    let dispatcher = Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        catalogue.clone(),
    );
    let nested = dispatcher
        .register_catalog_search_for_application("parent-child", &query())
        .unwrap();
    let unrelated = dispatcher
        .register_catalog_search_for_application("unrelated-search", &query())
        .unwrap();
    assert!(dispatcher
        .register_catalog_search_for_application("parent-child", &query())
        .is_err());
    let error = match nested.execute_with_parent_cancellation(&|| true) {
        Ok(_) => panic!("a cancelled parent must prevent provider entry"),
        Err(error) => error,
    };
    assert_eq!(error.code, "CATALOGUE_REQUEST_CANCELLED");
    assert_eq!(catalogue.calls.load(Ordering::SeqCst), 0);
    assert!(
        !dispatcher.cancel_request("parent-child"),
        "child lease must be released"
    );
    let result = unrelated.execute().unwrap();
    result.lease.begin_publication().unwrap();
    assert_eq!(catalogue.calls.load(Ordering::SeqCst), 1);
    drop(result);
    assert!(!dispatcher.has_active_catalogue_requests());
    let retry = dispatcher
        .register_catalog_search_for_application("parent-child", &query())
        .unwrap()
        .execute_with_parent_cancellation(&|| false)
        .unwrap();
    drop(retry);
    assert!(!dispatcher.has_active_catalogue_requests());
}

#[test]
fn parent_cancellation_at_provider_return_discards_response_and_releases_lease() {
    struct CancelOnReturn(Arc<AtomicBool>);
    impl LiveCatalogSearch for CancelOnReturn {
        fn search_catalog(
            &self,
            _query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            self.0.store(true, Ordering::Release);
            Ok(response())
        }
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let dispatcher = Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        Arc::new(CancelOnReturn(Arc::clone(&cancelled))),
    );
    let nested = dispatcher
        .register_catalog_search_for_application("late-parent-child", &query())
        .unwrap();
    let error = match nested.execute_with_parent_cancellation(&|| cancelled.load(Ordering::Acquire))
    {
        Ok(_) => panic!("a late cancelled parent must not publish the returned result"),
        Err(error) => error,
    };
    assert_eq!(error.code, "CATALOGUE_REQUEST_CANCELLED");
    assert!(!dispatcher.has_active_catalogue_requests());
}

fn response_product() -> insar_core::ProviderProduct {
    response_product_with_id("fixture-product")
}

fn response_product_with_id(product_id: &str) -> insar_core::ProviderProduct {
    serde_json::from_value(json!({
        "provider_id": "copernicus.dataspace.odata",
        "product_id": product_id,
        "display_name": "Fixture product",
        "source_kind": "sentinel2",
        "product_kind": "optical_scene",
        "assets": []
    }))
    .unwrap()
}

/// #22 B slice 3(b) review fix 1: the production router must forward
/// `search_catalog_streaming` to the matching route port, not fall back to the
/// trait default whole-response path. A migrated per-client streaming override
/// must run when reached through the real `ProductionCatalogRouter`.
#[test]
fn production_router_forwards_streaming_to_the_matching_route_port() {
    /// A migrated client: streaming override that reports `Streamed` and
    /// records the products it handed to the sink in two bounded chunks.
    struct StreamingCatalog {
        calls: AtomicUsize,
        accepted: Mutex<Vec<usize>>,
    }
    impl LiveCatalogSearch for StreamingCatalog {
        fn search_catalog(
            &self,
            _query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            panic!("the router must forward streaming to the migrated client override");
        }
        fn search_catalog_streaming(
            &self,
            query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
            sink: &mut dyn insar_service::CatalogSearchStreamSink,
            _progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
        ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            sink.attempt(query)?;
            for chunk_len in [2_usize, 1] {
                sink.accept(&vec![response_product(); chunk_len])?;
                self.accepted.lock().unwrap().push(chunk_len);
            }
            Ok(insar_service::CatalogSearchStreamOutcome::Streamed {
                collection: response().collection,
                stats: response().stats,
            })
        }
    }
    struct RecordingSink {
        attempts: usize,
        accepted: usize,
    }
    impl insar_service::CatalogSearchStreamSink for RecordingSink {
        fn accept(&mut self, products: &[insar_core::ProviderProduct]) -> ProviderResult<()> {
            self.accepted += products.len();
            Ok(())
        }
        fn attempt(&mut self, _query: &CatalogSearchQuery) -> ProviderResult<()> {
            self.attempts += 1;
            Ok(())
        }
    }

    let cdse: Arc<dyn LiveCatalogSearch> = Arc::new(StreamingCatalog {
        calls: AtomicUsize::new(0),
        accepted: Mutex::new(Vec::new()),
    });
    let usgs = Arc::new(FixtureCatalog::ready());
    let asf = Arc::new(FixtureCatalog::ready());
    // The Earth Search route is a migrated streaming client; prove the router
    // forwards to it rather than to the trait default whole-response path.
    let earth_search: Arc<dyn LiveCatalogSearch> = Arc::new(StreamingCatalog {
        calls: AtomicUsize::new(0),
        accepted: Mutex::new(Vec::new()),
    });
    let router = ProductionCatalogRouter::without_landsat(cdse.clone(), asf.clone())
        .with_earth_search(earth_search.clone());
    let mut sink = RecordingSink {
        attempts: 0,
        accepted: 0,
    };
    let mut landsat = landsat_query();
    landsat.provider_id = insar_provider::catalog::EARTH_SEARCH_LANDSAT_PROVIDER_ID.to_owned();
    landsat.native_collection_id =
        insar_provider::catalog::EARTH_SEARCH_LANDSAT_COLLECTION_ID.to_owned();
    let outcome = router
        .search_catalog_streaming(&landsat, &|| false, &mut sink, &|_| {})
        .expect("the real production router must execute the streaming search");
    assert!(
        matches!(outcome, insar_service::CatalogSearchStreamOutcome::Streamed { .. }),
        "a migrated client reached through the production router must report Streamed, not the trait default WholeResponse"
    );
    assert_eq!(sink.attempts, 1, "exactly one source attempt must run");
    assert_eq!(
        sink.accepted, 3,
        "all streamed products must reach the sink"
    );
    assert_eq!(usgs.calls.load(Ordering::SeqCst), 0);
    assert_eq!(asf.calls.load(Ordering::SeqCst), 0);
}

/// #22 B slice 3(b) review fix 3: a strict-streaming fail-closed on a later
/// page (after pages were already flushed) triggers the whole-response retry.
/// The retry must call `sink.attempt(query)` first so the sink resets its
/// staged rows from the failed stream; otherwise the re-staged same items
/// collide with the already-staged ones.
#[test]
fn streaming_cap_order_retry_resets_the_sink_before_restaging() {
    fn cap_order_error() -> ContractError {
        ContractError::new(
            "STAC_RESPONSE_INVALID",
            "fixture mid-stream sort change",
            serde_json::Map::from_iter([(
                "reason".to_owned(),
                serde_json::Value::String("streaming_cap_order".to_owned()),
            )]),
        )
        .unwrap()
    }
    /// Streams one chunk of two products, then fails closed with the typed
    /// `streaming_cap_order` reason after that page was flushed (a later
    /// continuation changed the sort). `search_catalog` then succeeds with
    /// the same products, as the whole-response retry must.
    struct LateCapOrderCatalog;
    impl LiveCatalogSearch for LateCapOrderCatalog {
        fn search_catalog(
            &self,
            _query: &CatalogSearchQuery,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> ProviderResult<CatalogSearchResponse> {
            // The whole-response retry returns the same products the failed
            // stream had already flushed (plus the pages it never reached).
            let mut whole = response();
            whole.products = vec![
                response_product_with_id("fixture-product"),
                response_product_with_id("fixture-product-2"),
            ];
            Ok(whole)
        }
        fn search_catalog_streaming(
            &self,
            query: &CatalogSearchQuery,
            cancelled: &(dyn Fn() -> bool + Sync),
            sink: &mut dyn insar_service::CatalogSearchStreamSink,
            progress: &dyn Fn(insar_provider::asf::catalog_client::CatalogSearchProviderProgress),
        ) -> ProviderResult<insar_service::CatalogSearchStreamOutcome> {
            // Mirror the migrated per-client overrides exactly: attempt, run
            // the provider stream, and on the typed cap-order fail-closed
            // (fired on a LATER page, after pages were flushed) retry on the
            // whole-response path through the same shared retry helper.
            sink.attempt(query)?;
            let streamed = (|| {
                sink.accept(&[
                    response_product_with_id("fixture-product"),
                    response_product_with_id("fixture-product-2"),
                ])?;
                Err::<(), _>(cap_order_error())
            })();
            match streamed {
                Ok(()) => unreachable!("the fixture stream always fails closed"),
                Err(error) if insar_service::is_streaming_cap_order(&error) => {
                    // The production retry helper: sink.attempt(query)
                    // resets staged rows before the whole response is fetched
                    // and handed over as one chunk.
                    insar_service::retry_whole_response_streamed(
                        self, query, cancelled, sink, progress,
                    )
                }
                Err(error) => Err(error),
            }
        }
    }
    /// A stateful sink like the application's staging sink: rows staged by a
    /// failed source must be dropped when the retry begins, or re-staging
    /// the same items collides (`AlreadyExists` at the Store).
    struct StagingSink {
        attempts: usize,
        staged: Vec<String>,
    }
    impl insar_service::CatalogSearchStreamSink for StagingSink {
        fn accept(&mut self, products: &[insar_core::ProviderProduct]) -> ProviderResult<()> {
            for product in products {
                if self.staged.contains(&product.product_id) {
                    return Err(ContractError::new(
                        "STORE_ALREADY_EXISTS",
                        "目录产品身份重复",
                        Default::default(),
                    )
                    .unwrap());
                }
                self.staged.push(product.product_id.clone());
            }
            Ok(())
        }
        fn attempt(&mut self, _query: &CatalogSearchQuery) -> ProviderResult<()> {
            self.attempts += 1;
            self.staged.clear();
            Ok(())
        }
    }

    let dispatcher = Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        Arc::new(LateCapOrderCatalog),
    );
    let registered = dispatcher
        .register_catalog_search_for_application("cap-order-retry", &query())
        .unwrap();
    let mut sink = StagingSink {
        attempts: 0,
        staged: Vec::new(),
    };
    let streamed = registered
        .execute_candidates_streaming(&|_| None, &mut sink, &|_| {})
        .expect("the whole-response retry must succeed after the typed fail-closed");
    assert!(
        matches!(
            streamed.outcome,
            insar_service::CatalogSearchStreamOutcome::WholeResponse { .. }
        ),
        "the retry lands on the whole-response path"
    );
    assert_eq!(sink.attempts, 2, "the retry must re-run sink.attempt first");
    assert_eq!(
        sink.staged.len(),
        2,
        "the retried whole response stages exactly its products once"
    );
}

/// #22 B slice 3(b) review fix 8: the streamed nested execution must honor
/// the parent cancellation signal like the classic path. A parent cancelled
/// at provider return must discard the streamed result and release the lease.
#[test]
fn parent_cancellation_at_streamed_provider_return_discards_result() {
    struct StreamOnReturn(Arc<AtomicBool>);
    impl LiveCatalogSearch for StreamOnReturn {
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
            sink.accept(&[response_product()])?;
            // Cancel the parent after the stream flushed, before the return.
            self.0.store(true, Ordering::Release);
            let response = response();
            Ok(insar_service::CatalogSearchStreamOutcome::WholeResponse { response })
        }
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let dispatcher = Dispatcher::with_search_ports_for_application(
        Arc::new(FixtureAsfSearch::default()),
        Arc::new(StreamOnReturn(Arc::clone(&cancelled))),
    );
    let nested = dispatcher
        .register_catalog_search_for_application("late-parent-stream", &query())
        .unwrap();
    struct NoopSink;
    impl insar_service::CatalogSearchStreamSink for NoopSink {
        fn accept(&mut self, _products: &[insar_core::ProviderProduct]) -> ProviderResult<()> {
            Ok(())
        }
    }
    let error = match nested.execute_candidates_streaming_with_cancellation(
        &|_| None,
        &mut NoopSink,
        &|_| {},
        &|| cancelled.load(Ordering::Acquire),
    ) {
        Ok(_) => panic!("a late cancelled parent must not publish the streamed result"),
        Err(error) => error,
    };
    assert_eq!(error.code, "CATALOGUE_REQUEST_CANCELLED");
    assert!(!dispatcher.has_active_catalogue_requests());
}
