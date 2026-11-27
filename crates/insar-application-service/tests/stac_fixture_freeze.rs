//! Offline end-to-end admission/publication tests. All products and URLs are
//! synthetic; no Provider transport or desktop credential lookup is used.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, AssetRole, CatalogSearchQuery,
    CatalogSearchResponse, DataSourceKind, DownloadItemProgress, DownloadItemResult,
    FrozenSearchBatch, PreviewAssetRequest, ProductKind, ProviderResult, StoreRestoreRequest,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::catalog::{
    map_stac_complete_search_response, CatalogDeliveryModel, DatasetRoutePolicyRegistry, RouteUse,
    StacAssetRoleBinding, StacCollectionBinding, StacOfflineFixtureAdmission, StacRouteProfile,
};
use insar_service::{Dispatcher, LiveAsfSearch, LiveCatalogSearch};
use insar_store::{StoreWorkbenchPageRequest, TaskStore};
use serde_json::{json, Map, Value};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);
impl TempStore {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "sceneport-stac-freeze-{}-{}",
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

fn geometry(multi: bool) -> Map<String, Value> {
    let ring = json!([
        [120.0, 20.0],
        [121.0, 20.0],
        [121.0, 21.0],
        [120.0, 21.0],
        [120.0, 20.0]
    ]);
    if multi {
        json!({"type":"MultiPolygon","coordinates":[[ring]]})
    } else {
        json!({"type":"Polygon","coordinates":[ring]})
    }
    .as_object()
    .unwrap()
    .clone()
}

fn route() -> StacRouteProfile {
    StacRouteProfile {
        provider_id: "fixture.stac".into(),
        endpoint: "https://catalog.example.test/stac/v1/".into(),
        search_path: "search".into(),
        page_cap: 100,
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
fn request(label: &str, multi: bool) -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: format!("request-{label}"),
        batch_id: format!("batch-{label}"),
        created_at: "2026-09-11T00:00:00Z".into(),
        aoi_name: "Synthetic test area".into(),
        aoi_geojson: Some(geometry(multi)),
        workbench_page_size: 24,
        search: CatalogSearchQuery {
            logical_dataset: "optical.fixture.l2".into(),
            provider_id: "fixture.stac".into(),
            native_collection_id: "fixture-l2".into(),
            aoi_geojson: Some(geometry(multi)),
            cursor: None,
            result_limit: Some(1),
            filters: Map::new(),
        },
    }
}
fn response(query: &CatalogSearchQuery) -> CatalogSearchResponse {
    map_stac_complete_search_response(&route(), &binding(), query, &json!({
        "type":"FeatureCollection", "numberMatched":10, "numberReturned":1,
        "features":[{
            "type":"Feature", "stac_version":"1.1.0", "id":"item-1", "collection":"fixture-l2",
            "geometry":query.aoi_geojson,
            "properties":{"datetime":"2026-09-10T12:00:00Z","scene:id":"native-1","title":"Synthetic scene","proj:epsg":32651},
            "assets":{"data":{"href":"https://data.example.test/scene.tif","roles":["data"],"file:size":4096}},
            "links":[]
        }],
        "links":[{"rel":"next","href":"https://catalog.example.test/stac/v1/search?page=2"}]
    })).unwrap()
}

#[derive(Clone, Copy, Default)]
enum Fault {
    #[default]
    None,
    Collection,
    Incomplete,
    Identity,
    Geometry,
    Asset,
    Oversized,
}
#[derive(Default)]
struct FixtureCatalog {
    calls: AtomicU64,
    fault: Mutex<Fault>,
    after_response: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
impl LiveCatalogSearch for FixtureCatalog {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        assert!(!cancelled());
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut result = response(query);
        match *self.fault.lock().unwrap() {
            Fault::None => {}
            Fault::Collection => {
                result.collection.collection_profile_id = "unreviewed-profile".into()
            }
            Fault::Incomplete => {
                result.stats.working_set_satisfied = false;
                result.next_cursor = Some("unconsumed-page".into());
            }
            Fault::Identity => {
                result.products[0]
                    .properties
                    .insert("provider_product_key".into(), json!("forged"));
            }
            Fault::Geometry => {
                result.products[0].footprint_geojson = Some(
                    json!({"type":"Polygon","coordinates":[]})
                        .as_object()
                        .unwrap()
                        .clone(),
                );
            }
            Fault::Asset => {
                result.products[0].assets[0].url =
                    Some("https://evil.example.test/data.tif".into());
            }
            Fault::Oversized => {
                result.products[0]
                    .properties
                    .get_mut("stac_properties")
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert("description".into(), json!("x".repeat(70 * 1024)));
            }
        }
        if let Some(hook) = self.after_response.lock().unwrap().clone() {
            hook();
        }
        Ok(result)
    }
}

fn service(
    database: &TempStore,
    provider: Arc<FixtureCatalog>,
    worker: Arc<NoDownload>,
) -> ApplicationService {
    ApplicationService::with_offline_stac_fixture_business_runtime(
        ApplicationServiceConfig::business_store(database.database()),
        Arc::new(NoAsf),
        provider,
        worker,
        admission(),
    )
}
fn batch(service: &ApplicationService) -> FrozenSearchBatch {
    service.restore(true).unwrap().workbench_batch.unwrap()
}

struct CrossDomainCatalog {
    route: StacRouteProfile,
    binding: StacCollectionBinding,
}

impl LiveCatalogSearch for CrossDomainCatalog {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        assert!(!cancelled());
        map_stac_complete_search_response(
            &self.route,
            &self.binding,
            query,
            &json!({
                "type":"FeatureCollection",
                "numberMatched":1,
                "numberReturned":1,
                "features":[{
                    "type":"Feature",
                    "stac_version":"1.1.0",
                    "id":"cross-domain-item-1",
                    "collection":self.binding.native_collection_id,
                    "geometry":query.aoi_geojson,
                    "properties":{
                        "datetime":"2026-09-10T12:00:00Z",
                        "platform":"fixture-platform",
                        "proj:epsg":4326
                    },
                    "assets":{
                        "data":{
                            "href":"https://data.example.test/cross-domain.tif",
                            "roles":["data"],
                            "type":"image/tiff; application=geotiff; profile=cloud-optimized",
                            "file:size":8192
                        }
                    },
                    "links":[]
                }],
                "links":[]
            }),
        )
    }
}

fn cross_domain_binding(
    logical_dataset: &str,
    native_collection_id: &str,
    source_kind: DataSourceKind,
    product_kind: ProductKind,
) -> StacCollectionBinding {
    StacCollectionBinding {
        logical_dataset: logical_dataset.into(),
        provider_id: "fixture.stac.cross-domain".into(),
        native_collection_id: native_collection_id.into(),
        collection_profile_id: format!("fixture.stac.{native_collection_id}.v1"),
        identity_rule_id: "fixture.stac.item-id.v1".into(),
        filename_rule_id: "fixture.stac.asset-filename.v1".into(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        source_kind,
        product_kind,
        production_enabled: false,
        product_id_property: None,
        display_name_property: None,
        queryables: vec![],
        asset_role_bindings: vec![StacAssetRoleBinding {
            stac_asset_key: "data".into(),
            role: AssetRole::Primary,
            required: true,
            allowed_hosts: vec!["data.example.test".into()],
        }],
    }
}

fn cross_domain_route() -> StacRouteProfile {
    StacRouteProfile {
        provider_id: "fixture.stac.cross-domain".into(),
        endpoint: "https://catalog.example.test/stac/v1/".into(),
        search_path: "search".into(),
        page_cap: 10,
        accepted_stac_versions: vec!["1.0.0".into(), "1.1.0".into()],
    }
}

fn cross_domain_request(
    label: &str,
    logical_dataset: &str,
    native_collection_id: &str,
) -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: format!("request-cross-domain-{label}"),
        batch_id: format!("batch-cross-domain-{label}"),
        created_at: "2026-09-19T00:00:00Z".into(),
        aoi_name: "Cross-domain fixture area".into(),
        aoi_geojson: Some(geometry(false)),
        workbench_page_size: 24,
        search: CatalogSearchQuery {
            logical_dataset: logical_dataset.into(),
            provider_id: "fixture.stac.cross-domain".into(),
            native_collection_id: native_collection_id.into(),
            aoi_geojson: Some(geometry(false)),
            cursor: None,
            result_limit: Some(1),
            filters: Map::new(),
        },
    }
}

#[test]
fn generic_stac_freeze_preserves_sar_and_dem_product_kinds_without_execution_authority() {
    for (
        label,
        logical_dataset,
        native_collection_id,
        source_kind,
        product_kind,
        expected_source,
        expected_product,
    ) in [
        (
            "sar",
            "sar.fixture.sentinel1-grd",
            "fixture-sentinel1-grd",
            DataSourceKind::Sentinel1,
            ProductKind::SarGrd,
            "sentinel1",
            "sar_grd",
        ),
        (
            "dem",
            "dem.fixture.copdem-glo30",
            "fixture-copdem-glo30",
            DataSourceKind::Dem,
            ProductKind::DemRaster,
            "dem",
            "dem_raster",
        ),
    ] {
        let database = TempStore::new();
        let binding = cross_domain_binding(
            logical_dataset,
            native_collection_id,
            source_kind,
            product_kind,
        );
        let route = cross_domain_route();
        let admission = Arc::new(
            StacOfflineFixtureAdmission::new_for_test(route.clone(), binding.clone()).unwrap(),
        );
        let provider = Arc::new(CrossDomainCatalog { route, binding });
        let worker = Arc::new(NoDownload::default());
        let service = ApplicationService::with_offline_stac_fixture_business_runtime(
            ApplicationServiceConfig::business_store(database.database()),
            Arc::new(NoAsf),
            provider,
            worker.clone(),
            admission,
        );
        let request = cross_domain_request(label, logical_dataset, native_collection_id);
        service.catalog_search_and_freeze(&request).unwrap();
        let frozen = batch(&service);
        frozen.validate().unwrap();
        assert_eq!(frozen.candidates.len(), 1);
        let item = &frozen.candidates[0];
        assert_eq!(item.metadata["source_kind"], expected_source);
        assert_eq!(item.metadata["product_kind"], expected_product);
        assert_eq!(item.metadata["logical_dataset"], logical_dataset);
        assert_eq!(item.metadata["native_collection_id"], native_collection_id);
        assert_eq!(item.metadata["assets"][0]["size_bytes"], 8192);
        assert!(item.asset_access.is_none());
        assert!(item.execution_policy.is_none());
        assert!(item.payload.is_empty());
        assert!(!serde_json::to_string(item)
            .unwrap()
            .contains("https://data.example.test"));
        assert_eq!(worker.0.load(Ordering::SeqCst), 0);

        let original = serde_json::to_vec(&frozen).unwrap();
        service.shutdown().unwrap();
        drop(service);
        let store = TaskStore::open(database.database()).unwrap();
        let restored = store
            .restore(&StoreRestoreRequest {
                include_terminal: true,
            })
            .unwrap()
            .workbench_batch
            .unwrap();
        assert_eq!(serde_json::to_vec(&restored).unwrap(), original);
    }
}

#[test]
fn normal_publication_and_store_reopen_preserve_both_geometry_types() {
    for multi in [false, true] {
        let database = TempStore::new();
        let provider = Arc::new(FixtureCatalog::default());
        let worker = Arc::new(NoDownload::default());
        let service = service(&database, provider.clone(), worker.clone());
        let request = request("accepted", multi);
        service.catalog_search_and_freeze(&request).unwrap();
        let original = batch(&service);
        original.validate().unwrap();
        assert_eq!(original.aoi_geojson, request.aoi_geojson);
        assert_eq!(
            original.query,
            serde_json::to_value(&request.search)
                .unwrap()
                .as_object()
                .unwrap()
                .clone()
        );
        assert_eq!(original.candidates.len(), 1);
        let item = &original.candidates[0];
        assert_eq!(item.footprint_geojson, Some(geometry(multi)));
        assert_eq!(
            item.acquisition_datetime.as_deref(),
            Some("2026-09-10T12:00:00Z")
        );
        assert_eq!(item.metadata["stac_properties"]["proj:epsg"], 32651);
        assert_eq!(item.metadata["assets"][0]["size_bytes"], 4096);
        assert!(
            item.asset_access.is_none()
                && item.execution_policy.is_none()
                && item.payload.is_empty()
        );
        assert!(!serde_json::to_string(item)
            .unwrap()
            .contains("https://data.example.test"));
        assert_eq!(
            original.search_stats.as_ref().unwrap().matched_count,
            Some(10)
        );
        let page = service
            .workbench_page(&StoreWorkbenchPageRequest {
                batch_id: request.batch_id,
                expected_revision: 0,
                page_index: 0,
                page_size: 24,
            })
            .unwrap();
        assert_eq!(page.page.total_count, 1);
        assert_eq!(&page.page.items[0].item, item);
        assert_eq!(page.page.checked_count, 0);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(worker.0.load(Ordering::SeqCst), 0);
        service.shutdown().unwrap();
        drop(service);
        let store = TaskStore::open(database.database()).unwrap();
        let restored = store
            .restore(&StoreRestoreRequest {
                include_terminal: true,
            })
            .unwrap();
        assert_eq!(
            serde_json::to_vec(&restored.workbench_batch.unwrap()).unwrap(),
            serde_json::to_vec(&original).unwrap()
        );
        assert!(restored.tasks.is_empty());
    }
}

#[test]
fn dispatcher_requires_local_admission_before_calling_provider() {
    let query = request("dispatcher", false).search;
    for production in [false, true] {
        let provider = Arc::new(FixtureCatalog::default());
        let dispatcher = if production {
            Dispatcher::with_production_catalogue_for_application(Arc::new(NoAsf), provider.clone())
        } else {
            Dispatcher::with_search_ports_for_application(Arc::new(NoAsf), provider.clone())
        };
        let error = dispatcher
            .search_catalog_for_application("no-admission", &query)
            .err()
            .unwrap();
        assert_eq!(error.code, "CATALOGUE_ROUTE_UNAVAILABLE");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
    let provider = Arc::new(FixtureCatalog::default());
    let dispatcher = Dispatcher::with_offline_stac_fixture_for_application(
        Arc::new(NoAsf),
        provider.clone(),
        admission(),
    );
    let result = dispatcher
        .search_catalog_for_application("admitted", &query)
        .unwrap();
    assert_eq!(result.response.products.len(), 1);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(DatasetRoutePolicyRegistry
        .resolve(
            &query.logical_dataset,
            &query.provider_id,
            &query.native_collection_id,
            RouteUse::OfflineFixture
        )
        .is_err());
}

#[test]
fn application_normal_offline_and_production_constructors_do_not_admit_fixture() {
    for production in [false, true] {
        let database = TempStore::new();
        let provider = Arc::new(FixtureCatalog::default());
        let service = if production {
            ApplicationService::with_catalogue_production_business_runtime(
                ApplicationServiceConfig::business_store(database.database()),
                Arc::new(NoAsf),
                provider.clone(),
                Arc::new(NoDownload::default()),
            )
        } else {
            ApplicationService::with_catalogue_fixture_business_runtime(
                ApplicationServiceConfig::business_store(database.database()),
                Arc::new(NoAsf),
                provider.clone(),
                Arc::new(NoDownload::default()),
            )
        };
        assert!(service
            .catalog_search_and_freeze(&request("denied", false))
            .is_err());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
        assert!(service.restore(true).unwrap().workbench_batch.is_none());
        service.shutdown().unwrap();
    }
}

#[test]
fn both_admission_gates_reject_production_mode_and_untrusted_bindings() {
    let query = request("mode", false).search;
    let admission = admission();
    assert!(admission
        .validate_query(&query, RouteUse::Production)
        .is_err());
    assert!(admission
        .validate_response(&query, &response(&query), RouteUse::Production)
        .is_err());
    assert!(admission
        .validate_product(&response(&query).products[0], RouteUse::Production)
        .is_err());
    let mut enabled = binding();
    enabled.production_enabled = true;
    assert!(StacOfflineFixtureAdmission::new_for_test(route(), enabled).is_err());
    let mut collision = binding();
    collision.logical_dataset = "optical.sentinel-2.l2a".into();
    assert!(StacOfflineFixtureAdmission::new_for_test(route(), collision).is_err());
    let mut real_provider = binding();
    real_provider.provider_id = "copernicus.dataspace.odata".into();
    let mut real_route = route();
    real_route.provider_id = real_provider.provider_id.clone();
    assert!(StacOfflineFixtureAdmission::new_for_test(real_route, real_provider).is_err());
}

#[test]
fn invalid_request_scope_and_aoi_never_reach_provider() {
    let database = TempStore::new();
    let provider = Arc::new(FixtureCatalog::default());
    let service = service(&database, provider.clone(), Arc::new(NoDownload::default()));
    for mutation in 0..5 {
        let mut req = request("bad-request", false);
        match mutation {
            0 => req.search.provider_id = "asf.catalog".into(),
            1 => req.search.native_collection_id = "OTHER".into(),
            2 => {
                req.search.filters.insert("unreviewed".into(), json!(true));
            }
            3 => req.aoi_geojson = Some(geometry(true)),
            _ => req.workbench_page_size = 251,
        }
        assert!(service.catalog_search_and_freeze(&req).is_err());
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(service.restore(true).unwrap().workbench_batch.is_none());
    service.shutdown().unwrap();
}

#[test]
fn fixture_admission_cannot_be_supplied_by_serialized_requests_or_shadow_static_routes() {
    let req = request("wire-grant", false);
    for field in ["offline_stac_fixture", "fixture_admission", "route_use"] {
        let mut encoded = serde_json::to_value(&req).unwrap();
        encoded
            .as_object_mut()
            .unwrap()
            .insert(field.into(), json!(true));
        assert!(
            serde_json::from_value::<ApplicationCatalogSearchAndFreezeRequest>(encoded).is_err()
        );
    }
    // Even an ordinary valid wire request does not carry the process-local grant.
    assert!(req.validate_for(RouteUse::OfflineFixture).is_err());
    assert!(req.validate_for(RouteUse::Production).is_err());

    let registry = DatasetRoutePolicyRegistry;
    // Sentinel-2 L2A now has more than one reviewed source, so the legacy
    // single-route projection is intentionally ambiguous. Pin the exact static
    // CDSE route whose non-shadowing behavior this test actually owns.
    let original = *registry
        .resolve(
            "optical.sentinel-2.l2a",
            "copernicus.dataspace.odata",
            "SENTINEL-2",
            RouteUse::OfflineFixture,
        )
        .unwrap();
    let provider = Arc::new(FixtureCatalog::default());
    let dispatcher = Dispatcher::with_offline_stac_fixture_for_application(
        Arc::new(NoAsf),
        provider.clone(),
        admission(),
    );
    let mut registered_query = req.search;
    registered_query.logical_dataset = original.logical_dataset.into();
    registered_query.provider_id = original.provider_id.into();
    registered_query.native_collection_id = original.native_collection_id.into();
    // Registration uses the original static route; do not call a real provider.
    drop(
        dispatcher
            .register_catalog_search_for_application("static-route", &registered_query)
            .unwrap(),
    );
    registered_query.provider_id = "fixture.stac".into();
    let denied = dispatcher
        .register_catalog_search_for_application("wrong-provider", &registered_query)
        .err()
        .unwrap();
    assert_eq!(denied.code, "CATALOGUE_ROUTE_UNAVAILABLE");
    assert_eq!(
        *registry
            .resolve(
                original.logical_dataset,
                original.provider_id,
                original.native_collection_id,
                RouteUse::OfflineFixture,
            )
            .unwrap(),
        original
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn response_gate_rejects_forged_asset_identity_time_and_count_claims() {
    let query = request("forged-facts", false).search;
    let admission = admission();
    for mutation in 0..6 {
        let mut forged = response(&query);
        match mutation {
            0 => forged.products[0].assets[0].stable_asset_key = Some("forged-key".into()),
            1 => forged.products[0].assets[0].role = AssetRole::Thumbnail,
            2 => forged.products[0].acquisition_datetime = Some("2026-09-09T12:00:00Z".into()),
            3 => forged.collection.production_enabled = true,
            4 => forged.stats.requested_limit = Some(2),
            _ => forged.stats.matched_count = Some(0),
        }
        assert!(admission
            .validate_response(&query, &forged, RouteUse::OfflineFixture)
            .is_err());
    }
}

#[test]
fn malformed_responses_never_replace_a_published_batch() {
    let database = TempStore::new();
    let provider = Arc::new(FixtureCatalog::default());
    let service = service(&database, provider.clone(), Arc::new(NoDownload::default()));
    service
        .catalog_search_and_freeze(&request("original", false))
        .unwrap();
    let original = batch(&service);
    for (n, fault) in [
        Fault::Collection,
        Fault::Incomplete,
        Fault::Identity,
        Fault::Geometry,
        Fault::Asset,
        Fault::Oversized,
    ]
    .into_iter()
    .enumerate()
    {
        *provider.fault.lock().unwrap() = fault;
        assert!(service
            .catalog_search_and_freeze(&request(&format!("fault-{n}"), false))
            .is_err());
        assert_eq!(batch(&service), original);
    }
    // Same query was not trusted via the persistent cache under a test grant.
    assert_eq!(provider.calls.load(Ordering::SeqCst), 7);
    service.shutdown().unwrap();
}

#[test]
fn cancellation_after_provider_return_does_not_publish_or_leak_lease() {
    let database = TempStore::new();
    let provider = Arc::new(FixtureCatalog::default());
    let service = Arc::new(service(
        &database,
        provider.clone(),
        Arc::new(NoDownload::default()),
    ));
    let weak = Arc::downgrade(&service);
    let request = request("cancel", false);
    let id = request.request_id.clone();
    *provider.after_response.lock().unwrap() = Some(Arc::new(move || {
        assert!(weak.upgrade().unwrap().cancel_request(&id).unwrap());
    }));
    let error = service.catalog_search_and_freeze(&request).unwrap_err();
    assert!(format!("{error:?}").contains("CATALOGUE_REQUEST_CANCELLED"));
    assert!(service.restore(true).unwrap().workbench_batch.is_none());
    *provider.after_response.lock().unwrap() = None;
    service.catalog_search_and_freeze(&request).unwrap();
    assert_eq!(batch(&service).batch_id, request.batch_id);
    service.shutdown().unwrap();
}

#[test]
fn persisted_result_does_not_restore_admission_or_bypass_new_mapping_checks() {
    let database = TempStore::new();
    let provider = Arc::new(FixtureCatalog::default());
    let first = service(&database, provider.clone(), Arc::new(NoDownload::default()));
    first
        .catalog_search_and_freeze(&request("before-restart", false))
        .unwrap();
    let original = batch(&first);
    first.shutdown().unwrap();
    drop(first);
    let plain = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(database.database()),
        Arc::new(NoAsf),
        provider.clone(),
        Arc::new(NoDownload::default()),
    );
    assert_eq!(batch(&plain), original);
    assert!(plain
        .catalog_search_and_freeze(&request("not-restored", false))
        .is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    plain.shutdown().unwrap();
    drop(plain);
    let admitted = service(&database, provider.clone(), Arc::new(NoDownload::default()));
    *provider.fault.lock().unwrap() = Fault::Identity;
    assert!(admitted
        .catalog_search_and_freeze(&request("rechecked", false))
        .is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(batch(&admitted), original);
    admitted.shutdown().unwrap();
}

#[test]
fn discovery_assets_never_grant_preview_or_registered_download_provider() {
    let database = TempStore::new();
    let provider = Arc::new(FixtureCatalog::default());
    let worker = Arc::new(NoDownload::default());
    let service = service(&database, provider, worker.clone());
    service
        .catalog_search_and_freeze(&request("preview", false))
        .unwrap();
    let frozen = batch(&service);
    let item = &frozen.candidates[0];
    assert!(
        item.asset_access.is_none() && item.execution_policy.is_none() && item.payload.is_empty()
    );
    assert!(service
        .preview_asset(PreviewAssetRequest {
            request_id: "fixture-preview".into(),
            batch_id: frozen.batch_id,
            item_id: item.item_id.clone(),
        })
        .is_err());
    assert_eq!(worker.0.load(Ordering::SeqCst), 0);
    service.shutdown().unwrap();
}

#[test]
fn normal_task_path_and_real_worker_reject_discovery_without_any_transfer() {
    use insar_core::{CreateTaskFromWorkbenchRequest, DownloadLayout, DownloadOutcome};
    use insar_download::{ProductionDownloadWorker, TransferOptions};
    use std::time::{Duration, Instant};

    let database = TempStore::new();
    let provider = Arc::new(FixtureCatalog::default());
    // Client construction does not read credential material or issue requests.
    // The normal worker rejects fixture.stac in its closed provider dispatch
    // before URL resolution, authentication or filesystem transfer preparation.
    let worker = Arc::new(ProductionDownloadWorker::new(TransferOptions::default()).unwrap());
    let service = ApplicationService::with_offline_stac_fixture_business_runtime(
        ApplicationServiceConfig::business_store(database.database()),
        Arc::new(NoAsf),
        provider,
        worker,
        admission(),
    );
    service
        .catalog_search_and_freeze(&request("task-rejection", false))
        .unwrap();
    let frozen = batch(&service);
    let item = &frozen.candidates[0];
    let output = database.0.join("must-not-exist");
    service
        .create_task_from_workbench(&CreateTaskFromWorkbenchRequest {
            task_id: "fixture-download-attempt".into(),
            batch_id: frozen.batch_id.clone(),
            expected_workbench_revision: 0,
            exact_item_id: Some(item.item_id.clone()),
            output_root: output.clone(),
            layout: DownloadLayout::SarDatasetProcessing,
            concurrency: 1,
            created_at: "2026-09-11T00:00:01Z".into(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        let snapshot = service
            .download_snapshot("fixture-download-attempt")
            .unwrap();
        if let Some(result) = snapshot.results.into_iter().next() {
            break result;
        }
        assert!(
            Instant::now() < deadline,
            "normal worker must terminate without transfer"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.bytes_written, 0);
    let failure = result.failure.unwrap();
    assert_eq!(failure.code, "DL005");
    assert!(!failure.retryable);
    assert!(failure.reason_zh.contains("该数据源暂不支持直接下载"));
    assert!(
        !output.exists(),
        "no transfer/output directory may be created"
    );
    assert_eq!(batch(&service), frozen);
    service.shutdown().unwrap();
}
