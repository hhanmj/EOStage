use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationCatalogSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig, SarSearchProgressStage, SarSearchProgressUpdate,
};
use insar_core::{
    provider_product_key, stable_asset_key, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AssetRole, CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery,
    CatalogSearchResponse, CatalogSearchStats, ContractError, DownloadItemProgress,
    DownloadItemResult, ProviderProduct, ProviderResult, WorkbenchSelectionCommand,
    WorkbenchSelectionCommandKind,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::{
    asf::{map_alos2_umm_granule, CmrGranuleEnvelope},
    catalog::{
        CollectionProfileRegistry, ALOS2_FILENAME_RULE_ID, ALOS2_IDENTITY_RULE_ID,
        ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
        ALOS2_RESOLVER_POLICY_ID,
    },
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

mod common;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-alos2-freeze-{label}-{}-{id}.sqlite3",
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
struct FixtureAsf;

impl LiveAsfSearch for FixtureAsf {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }
}

struct FixtureWorker;

impl DownloadWorker for FixtureWorker {
    fn execute(
        &self,
        _work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        panic!("catalogue freeze must not execute download work")
    }
}

struct StaticCatalog {
    response: CatalogSearchResponse,
}

struct PanicOnceCatalog {
    calls: AtomicUsize,
    response: CatalogSearchResponse,
}

struct FailOnceCatalog {
    calls: AtomicUsize,
    response: CatalogSearchResponse,
}

impl LiveCatalogSearch for FailOnceCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
            return Err(ContractError::new(
                "FIXTURE_PROVIDER_FAILURE",
                "fixture provider failure",
                Map::new(),
            )
            .unwrap());
        }
        Ok(self.response.clone())
    }
}

impl LiveCatalogSearch for PanicOnceCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
            panic!("simulated provider panic after catalogue registration");
        }
        Ok(self.response.clone())
    }
}

impl LiveCatalogSearch for StaticCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        if cancelled() {
            return Err(ContractError::new(
                "CATALOGUE_REQUEST_CANCELLED",
                "fixture cancelled",
                Map::new(),
            )
            .unwrap());
        }
        Ok(self.response.clone())
    }
}

struct CountingCatalog {
    response: CatalogSearchResponse,
    calls: Arc<AtomicU64>,
}

impl LiveCatalogSearch for CountingCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if cancelled() {
            return Err(ContractError::new(
                "CATALOGUE_REQUEST_CANCELLED",
                "fixture cancelled",
                Map::new(),
            )
            .unwrap());
        }
        Ok(self.response.clone())
    }
}

struct PublicationGate {
    entered: AtomicBool,
    release: AtomicBool,
}

impl PublicationGate {
    fn wait(&self) {
        self.entered.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-alos2-p1")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("ALOS-2 fixture exists"))
        .expect("ALOS-2 fixture is valid JSON")
}

fn product() -> ProviderProduct {
    let envelope = CmrGranuleEnvelope::try_from(&fixture("granule-wbd-cmr-umm.json")).unwrap();
    let profile = CollectionProfileRegistry
        .get(ALOS2_PROFILE_ID)
        .expect("ALOS-2 profile exists");
    map_alos2_umm_granule(&envelope, profile).expect("valid ALOS-2 product")
}

fn aoi(multi: bool) -> Map<String, Value> {
    let value = if multi {
        json!({
            "type":"MultiPolygon",
            "coordinates":[[[[-114.0,21.0],[-109.0,21.0],[-109.0,26.0],[-114.0,26.0],[-114.0,21.0]]]]
        })
    } else {
        json!({
            "type":"Polygon",
            "coordinates":[[[-114.0,21.0],[-109.0,21.0],[-109.0,26.0],[-114.0,26.0],[-114.0,21.0]]]
        })
    };
    value.as_object().unwrap().clone()
}

fn search(multi: bool) -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(aoi(multi)),
        cursor: None,
        result_limit: Some(25),
        filters: json!({
            "datetime":{"start":"2023-03-31T00:00:00Z","end":"2023-04-01T00:00:00Z"},
            "beam_mode":"WBD",
            "polarizations":["HH","HV"],
            "flight_direction":"DESCENDING",
            "look_direction":"RIGHT",
            "path":161,
            "frame":3150
        })
        .as_object()
        .unwrap()
        .clone(),
    }
}

fn request(label: &str, multi: bool) -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: label.to_owned(),
        search: search(multi),
        batch_id: format!("{label}-batch"),
        created_at: "2026-07-29T00:00:00Z".to_owned(),
        aoi_name: "ALOS-2 fixture AOI".to_owned(),
        aoi_geojson: Some(aoi(multi)),
        workbench_page_size: 24,
    }
}

fn collection() -> CatalogCollectionDescriptor {
    CatalogCollectionDescriptor {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        collection_profile_id: ALOS2_PROFILE_ID.to_owned(),
        identity_rule_id: ALOS2_IDENTITY_RULE_ID.to_owned(),
        filename_rule_id: ALOS2_FILENAME_RULE_ID.to_owned(),
        delivery_model: CatalogDeliveryModel::ArchiveFirst,
        production_enabled: false,
    }
}

fn response(product: ProviderProduct, multi: bool) -> CatalogSearchResponse {
    CatalogSearchResponse {
        collection: collection(),
        products: vec![product],
        stats: CatalogSearchStats {
            requested_limit: Some(25),
            matched_count: (!multi).then_some(1),
            request_count: 1,
            transport_part_count: if multi { 2 } else { 1 },
            fetched_record_count: 1,
            mapped_record_count: 1,
            profile_filtered_record_count: 0,
            exact_duplicate_record_count: 0,
            unique_provider_product_count: 1,
            exact_matched_count: 1,
            returned_count: 1,
            remote_exhausted: true,
            working_set_satisfied: true,
            elapsed_ms: 1,
            truncated: false,
        },
        next_cursor: None,
        item_errors: Vec::new(),
    }
}

#[test]
fn asf_catalogue_search_without_hierarchy_uses_no_spatial_filter() {
    let database = TempStore::new("no-aoi");
    let service = service_without_hierarchy(&database, response(product(), false));
    let mut request = request("no-aoi-search", false);
    request.aoi_name = "未限定 AOI".to_owned();
    request.aoi_geojson = None;
    request.search.aoi_geojson = None;

    service
        .catalog_search_and_freeze(&request)
        .expect("ASF catalogue search is independent of saved hierarchy");
    let snapshot = service.snapshot(true).expect("bounded snapshot");
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("search batch is published");
    assert!(batch.aoi_geojson.is_none());
    assert_eq!(batch.aoi_name, "未限定 AOI");
    service.shutdown().unwrap();
}

#[test]
fn typed_catalogue_progress_terminates_after_success() {
    let database = TempStore::new("progress-success");
    let service = service_without_hierarchy(&database, response(product(), false));
    let progress = Mutex::new(Vec::<SarSearchProgressUpdate>::new());
    service
        .catalog_search_and_freeze_with_progress(&request("progress-success", false), |update| {
            progress.lock().unwrap().push(update);
        })
        .expect("typed catalogue search succeeds");
    let progress = progress.lock().unwrap();
    assert_eq!(
        progress.first().unwrap().stage,
        SarSearchProgressStage::Validating
    );
    assert_eq!(progress.first().unwrap().provider_request_count, None);
    assert_eq!(progress.first().unwrap().raw_fetched_count, None);
    assert_eq!(progress.first().unwrap().admitted_count, None);
    assert_eq!(
        progress.last().unwrap().stage,
        SarSearchProgressStage::Completed
    );
    assert!(progress.last().unwrap().terminal);
    assert_eq!(
        progress
            .iter()
            .map(|update| update.sequence)
            .collect::<Vec<_>>(),
        (1..=progress.len() as u64).collect::<Vec<_>>()
    );
    let post_provider = progress.iter().filter(|update| {
        matches!(
            &update.stage,
            SarSearchProgressStage::Filtering
                | SarSearchProgressStage::Freezing
                | SarSearchProgressStage::Completed
        )
    });
    assert!(post_provider.into_iter().all(|update| {
        update.provider_request_count == Some(1)
            && update.raw_fetched_count == Some(1)
            && update.mapped_count == Some(1)
            && update.exact_matched_count == Some(1)
            && update.admitted_count == Some(1)
            && update.requested_limit == Some(25)
            && update.remote_total == Some(1)
    }));
    service.shutdown().unwrap();
}

#[test]
fn provider_panic_does_not_poison_policy_gate_or_store_authority() {
    let database = TempStore::new("provider-panic-recovery");
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(PanicOnceCatalog {
            calls: AtomicUsize::new(0),
            response: response(product(), false),
        }),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "provider-panic-recovery", false);

    let first = catch_unwind(AssertUnwindSafe(|| {
        service.catalog_search_and_freeze(&request("provider-panic-first", false))
    }));
    assert!(
        first.is_err(),
        "fixture must reproduce an internal provider panic"
    );
    assert!(
        service
            .snapshot(true)
            .expect("Store remains readable after the provider panic")
            .restore
            .workbench_batch
            .is_none(),
        "a panicking provider must not publish a partial SearchBatch"
    );

    service
        .catalog_search_and_freeze(&request("provider-panic-second", false))
        .expect("the next valid ALOS-2 search succeeds without restarting the service");
    let frozen = service
        .snapshot(true)
        .expect("Store remains authoritative")
        .restore
        .workbench_batch
        .expect("the second search publishes its complete SearchBatch");
    assert_eq!(frozen.batch_id, "provider-panic-second-batch");
    service.shutdown().unwrap();
}

#[test]
fn provider_failure_does_not_block_the_next_valid_search() {
    let database = TempStore::new("provider-failure-recovery");
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(FailOnceCatalog {
            calls: AtomicUsize::new(0),
            response: response(product(), false),
        }),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "provider-failure-recovery", false);

    let first = service
        .catalog_search_and_freeze(&request("provider-failure-first", false))
        .expect_err("the fixture's first provider request fails");
    assert_eq!(first.code(), "V3_PROVIDER_OPERATION_FAILED");
    assert_empty_store(&service);

    service
        .catalog_search_and_freeze(&request("provider-failure-second", false))
        .expect("the next valid search succeeds without restarting the service");
    assert_eq!(
        service
            .snapshot(true)
            .unwrap()
            .restore
            .workbench_batch
            .unwrap()
            .batch_id,
        "provider-failure-second-batch"
    );
    service.shutdown().unwrap();
}

#[test]
fn invalid_query_returns_a_typed_error_and_does_not_block_the_next_valid_search() {
    let database = TempStore::new("invalid-query-recovery");
    let service = service(&database, response(product(), false));
    let mut invalid = request("invalid-query-first", false);
    invalid.search.result_limit = Some(0);

    let first = service
        .catalog_search_and_freeze(&invalid)
        .expect_err("page_size zero is rejected before provider execution");
    assert_eq!(first.code(), "V3_APPLICATION_INVALID_REQUEST");
    assert_empty_store(&service);

    service
        .catalog_search_and_freeze(&request("invalid-query-second", false))
        .expect("the next valid search succeeds without restarting the service");
    assert_eq!(
        service
            .snapshot(true)
            .unwrap()
            .restore
            .workbench_batch
            .unwrap()
            .batch_id,
        "invalid-query-second-batch"
    );
    service.shutdown().unwrap();
}

fn service(database: &TempStore, response: CatalogSearchResponse) -> ApplicationService {
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog { response }),
        Arc::new(FixtureWorker),
    );
    bind_aoi(&service, "fixture", false);
    service
}

fn service_without_hierarchy(
    database: &TempStore,
    response: CatalogSearchResponse,
) -> ApplicationService {
    ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog { response }),
        Arc::new(FixtureWorker),
    )
}

fn counting_service_without_hierarchy(
    database: &TempStore,
    response: CatalogSearchResponse,
    calls: Arc<AtomicU64>,
) -> ApplicationService {
    ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(CountingCatalog { response, calls }),
        Arc::new(FixtureWorker),
    )
}

fn bind_aoi(service: &ApplicationService, label: &str, multi: bool) {
    let region_id = common::ensure_fixture_region(service, label);
    let request = ApplicationBindRegionAoiRequest {
        request_id: format!("{label}-bind-aoi"),
        region_id,
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: aoi(multi),
        bbox: vec![-114.0, 21.0, -109.0, 26.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "ALOS-2 fixture AOI".to_owned(),
        bound_at: "2026-07-29T00:00:00Z".to_owned(),
    };
    service.bind_region_aoi(&request).unwrap();
}

fn assert_empty_store(service: &ApplicationService) {
    let snapshot = service.snapshot(true).unwrap();
    assert!(snapshot.restore.workbench_batch.is_none());
    assert!(snapshot.restore.workbench.is_none());
    assert!(snapshot.restore.tasks.is_empty());
}

fn assert_rejected(label: &str, response: CatalogSearchResponse, multi: bool, expected_code: &str) {
    let database = TempStore::new(label);
    let service = service(&database, response);
    let error = service
        .catalog_search_and_freeze_legacy(&request(label, multi))
        .unwrap_err();
    assert_eq!(error.code(), expected_code, "{label}: {error:?}");
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

#[test]
fn freezes_one_alos2_zip_and_preserves_the_non_secret_asset_graph() {
    let database = TempStore::new("valid");
    let mut original = product();
    original.properties.insert(
        "spatial_transport_mode".to_owned(),
        Value::String("cmr_exterior_conservative_exact_post_filter".to_owned()),
    );
    let provider_asset = original.assets[0].clone();
    let provider_properties = original.properties.clone();
    let service = service(&database, response(original.clone(), false));

    let snapshot = service
        .catalog_search_and_freeze_legacy(&request("valid", false))
        .expect("valid ALOS-2 complete response freezes atomically");
    let batch = snapshot.restore.workbench_batch.expect("one active batch");
    let search_stats = batch
        .search_stats
        .as_ref()
        .expect("typed catalogue search facts are frozen separately from candidates");
    assert_eq!(search_stats.matched_count, Some(1));
    assert_eq!(search_stats.fetched_record_count, Some(1));
    assert_eq!(search_stats.mapped_record_count, Some(1));
    assert_eq!(search_stats.provider_returned_count, 1);
    assert_eq!(search_stats.frozen_candidate_count, 1);
    assert!(!search_stats.truncated);
    assert_eq!(search_stats.source, "TYPED_CATALOG");
    assert_eq!(batch.candidates.len(), 1);
    let candidate = &batch.candidates[0];
    let expected_item_id = provider_product_key(
        ALOS2_PROVIDER_ID,
        ALOS2_NATIVE_COLLECTION_ID,
        &original.product_id,
    )
    .unwrap();
    assert_eq!(candidate.item_id, expected_item_id);
    assert_eq!(candidate.provider_id, ALOS2_PROVIDER_ID);
    assert_eq!(candidate.metadata["dataset"], ALOS2_LOGICAL_DATASET);
    assert_eq!(candidate.metadata["logical_dataset"], ALOS2_LOGICAL_DATASET);
    assert_eq!(
        candidate.metadata["collection_profile_id"],
        ALOS2_PROFILE_ID
    );
    assert_eq!(
        candidate.metadata["identity_rule_id"],
        ALOS2_IDENTITY_RULE_ID
    );
    assert_eq!(
        candidate.metadata["resolver_policy_id"],
        ALOS2_RESOLVER_POLICY_ID
    );
    assert_eq!(candidate.metadata["source_kind"], "sar");
    assert_eq!(candidate.metadata["product_kind"], "sar_product");
    assert_eq!(candidate.metadata["product_family"], "ALOS2_L1_PSR2");
    assert_eq!(candidate.metadata["processing_level"], "1.1");
    assert_eq!(
        candidate.metadata["properties"],
        Value::Object(provider_properties)
    );
    assert_eq!(
        candidate.metadata["assets"],
        serde_json::to_value(&original.assets).unwrap()
    );
    assert_eq!(candidate.metadata["properties"]["preview_available"], false);
    assert_eq!(
        candidate.metadata["properties"]["spatial_transport_mode"],
        "cmr_exterior_conservative_exact_post_filter"
    );
    assert!(candidate.payload.is_empty());

    let access = candidate
        .asset_access
        .as_ref()
        .expect("the unique ZIP is frozen as executable access");
    let asset_key = provider_asset.asset_key.as_deref().unwrap();
    assert_eq!(access.reference.provider_id, ALOS2_PROVIDER_ID);
    assert_eq!(access.reference.collection_id, ALOS2_NATIVE_COLLECTION_ID);
    assert_eq!(access.reference.provider_product_id, original.product_id);
    assert_eq!(access.reference.provider_product_key, expected_item_id);
    assert_eq!(access.reference.asset_key, asset_key);
    assert_eq!(
        access.reference.stable_asset_key,
        provider_asset.stable_asset_key.unwrap()
    );
    assert_eq!(access.descriptor, provider_asset.access_descriptor.unwrap());
    assert_eq!(access.role, AssetRole::Primary);
    assert_eq!(access.file_name, provider_asset.file_name);
    assert_eq!(access.expected_size_bytes, Some(60_018_405_388));
    assert_eq!(access.checksum_algorithm.as_deref(), Some("MD5"));
    assert_eq!(
        access.checksum_value.as_deref(),
        Some("0f558597e19d964f24ca3a4bff11696c")
    );
    let policy = candidate
        .execution_policy
        .as_ref()
        .expect("ALOS-2 execution policy is frozen");
    assert_eq!(policy.policy_id, ALOS2_RESOLVER_POLICY_ID);
    assert_eq!(
        policy.collection_profile_id.as_deref(),
        Some(ALOS2_PROFILE_ID)
    );
    service.shutdown().unwrap();
}

#[test]
fn identical_alos2_typed_query_reuses_cache_resets_workbench_and_survives_store_reopen() {
    let database = TempStore::new("typed-cache-reopen");
    let calls = Arc::new(AtomicU64::new(0));
    let service = counting_service_without_hierarchy(
        &database,
        response(product(), false),
        Arc::clone(&calls),
    );

    let first_progress = Mutex::new(Vec::<SarSearchProgressUpdate>::new());
    service
        .catalog_search_and_freeze_with_progress(&request("typed-cache-first", false), |update| {
            first_progress.lock().unwrap().push(update);
        })
        .expect("first ALOS-2 typed query freezes provider candidates");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        first_progress
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .provider_request_count,
        Some(1)
    );
    let first_batch = service
        .snapshot(true)
        .unwrap()
        .restore
        .workbench_batch
        .expect("first frozen ALOS-2 workbench batch");
    let first_batch_id = first_batch.batch_id.clone();
    let first_item_id = first_batch
        .candidates
        .first()
        .expect("fixture has one candidate")
        .item_id
        .clone();

    let checked = service
        .update_workbench_selection(&WorkbenchSelectionCommand {
            batch_id: first_batch_id.clone(),
            expected_revision: 0,
            kind: WorkbenchSelectionCommandKind::CheckItems,
            item_ids: vec![first_item_id.clone()],
            page_index: 0,
            page_size: 24,
            updated_at: "2026-07-29T00:00:01Z".to_owned(),
        })
        .expect("first batch accepts an exact candidate selection");
    assert_eq!(checked.revision, 1);
    let paged = service
        .update_workbench_selection(&WorkbenchSelectionCommand {
            batch_id: first_batch_id.clone(),
            expected_revision: 1,
            kind: WorkbenchSelectionCommandKind::SetPage,
            item_ids: Vec::new(),
            page_index: 4,
            page_size: 7,
            updated_at: "2026-07-29T00:00:02Z".to_owned(),
        })
        .expect("first batch accepts a page update");
    assert_eq!(paged.revision, 2);
    let dirty_workbench = service
        .snapshot(true)
        .unwrap()
        .restore
        .workbench
        .expect("dirty workbench state is persisted");
    assert_eq!(dirty_workbench.checked_item_ids, vec![first_item_id]);
    assert_eq!(dirty_workbench.page_index, 4);
    assert_eq!(dirty_workbench.page_size, 7);
    assert_eq!(dirty_workbench.revision, 2);

    let mut second_request = request("typed-cache-second", false);
    second_request.workbench_page_size = 11;
    let second_progress = Mutex::new(Vec::<SarSearchProgressUpdate>::new());
    service
        .catalog_search_and_freeze_with_progress(&second_request, |update| {
            second_progress.lock().unwrap().push(update);
        })
        .expect("identical ALOS-2 typed query freezes a fresh batch from cache");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        second_progress
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .provider_request_count,
        Some(0)
    );
    let second_snapshot = service.snapshot(true).unwrap();
    let second_batch = second_snapshot
        .restore
        .workbench_batch
        .as_ref()
        .expect("cached ALOS-2 workbench batch");
    let second_workbench = second_snapshot
        .restore
        .workbench
        .as_ref()
        .expect("cached ALOS-2 workbench state");
    assert_ne!(second_batch.batch_id, first_batch.batch_id);
    assert_eq!(second_batch.query, first_batch.query);
    assert_eq!(second_batch.candidates, first_batch.candidates);
    assert_eq!(second_workbench.batch_id, second_batch.batch_id);
    assert!(second_workbench.checked_item_ids.is_empty());
    assert_eq!(second_workbench.page_index, 0);
    assert_eq!(second_workbench.page_size, 11);
    assert_eq!(second_workbench.revision, 0);
    service.shutdown().unwrap();

    let reopened_store = TaskStore::open(database.0.as_path()).unwrap();
    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let reopened_batch = reopened_store
        .load_catalog_cache_batch(&search(false), now_epoch, 24 * 60 * 60)
        .unwrap()
        .expect("ALOS-2 cache pointer survives Store reopen");
    assert_eq!(reopened_batch.query, first_batch.query);
    assert_eq!(reopened_batch.candidates, first_batch.candidates);
    drop(reopened_store);

    let reopened_calls = Arc::new(AtomicU64::new(0));
    let reopened_service = counting_service_without_hierarchy(
        &database,
        response(product(), false),
        Arc::clone(&reopened_calls),
    );
    let mut reopened_request = request("typed-cache-after-reopen", false);
    reopened_request.workbench_page_size = 13;
    let reopened_progress = Mutex::new(Vec::<SarSearchProgressUpdate>::new());
    reopened_service
        .catalog_search_and_freeze_with_progress(&reopened_request, |update| {
            reopened_progress.lock().unwrap().push(update);
        })
        .expect("ALOS-2 cache hit works after reopening ApplicationService");
    assert_eq!(reopened_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        reopened_progress
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .provider_request_count,
        Some(0)
    );
    let reopened_snapshot = reopened_service.snapshot(true).unwrap();
    let reopened_active_batch = reopened_snapshot
        .restore
        .workbench_batch
        .as_ref()
        .expect("reopened service publishes a fresh cached batch");
    let reopened_workbench = reopened_snapshot
        .restore
        .workbench
        .as_ref()
        .expect("reopened service publishes fresh workbench state");
    assert_eq!(reopened_active_batch.query, first_batch.query);
    assert_eq!(reopened_active_batch.candidates, first_batch.candidates);
    assert_eq!(reopened_workbench.batch_id, reopened_active_batch.batch_id);
    assert!(reopened_workbench.checked_item_ids.is_empty());
    assert_eq!(reopened_workbench.page_index, 0);
    assert_eq!(reopened_workbench.page_size, 13);
    assert_eq!(reopened_workbench.revision, 0);
    reopened_service.shutdown().unwrap();
}

#[test]
fn accepts_multipolygon_duplicates_and_complete_batch_local_filter_counts() {
    let mut duplicate = response(product(), true);
    duplicate.stats.fetched_record_count = 2;
    duplicate.stats.mapped_record_count = 2;
    duplicate.stats.exact_duplicate_record_count = 1;
    let database = TempStore::new("multipolygon-duplicate");
    let duplicate_service = service(&database, duplicate);
    let snapshot = duplicate_service
        .catalog_search_and_freeze_legacy(&request("multipolygon-duplicate", true))
        .expect("cross-polygon exact duplicate is a complete batch");
    assert_eq!(
        snapshot.restore.workbench_batch.unwrap().candidates.len(),
        1
    );
    duplicate_service.shutdown().unwrap();

    let mut local_filter = response(product(), false);
    local_filter.stats.matched_count = Some(2);
    local_filter.stats.fetched_record_count = 2;
    local_filter.stats.mapped_record_count = 2;
    local_filter.stats.unique_provider_product_count = 2;
    let database = TempStore::new("local-filter");
    let service = service(&database, local_filter);
    service
        .catalog_search_and_freeze_legacy(&request("local-filter", false))
        .expect("complete upstream batch may return one locally accepted product");
    service.shutdown().unwrap();
}

#[test]
fn accepts_bounded_candidate_scan_telemetry_above_the_frozen_result_limit() {
    let mut scanned = response(product(), false);
    scanned.stats.requested_limit = Some(1);
    scanned.stats.matched_count = Some(1_287);
    scanned.stats.request_count = 3;
    scanned.stats.fetched_record_count = 750;
    scanned.stats.mapped_record_count = 750;
    scanned.stats.unique_provider_product_count = 750;
    scanned.stats.exact_matched_count = 343;
    scanned.stats.remote_exhausted = false;

    let database = TempStore::new("candidate-scan-above-result-limit");
    let service = service(&database, scanned);
    let mut operation = request("candidate-scan-above-result-limit", false);
    operation.search.result_limit = Some(1);
    service
        .catalog_search_and_freeze_legacy(&operation)
        .expect("bounded candidate telemetry must not be confused with frozen result count");
    service.shutdown().unwrap();
}

#[test]
fn accepts_polygon_hole_exact_post_filter_counts_without_remote_matched_count() {
    let geometry = json!({
        "type": "Polygon",
        "coordinates": [
            [[-114.0,21.0],[-109.0,21.0],[-109.0,26.0],[-114.0,26.0],[-114.0,21.0]],
            [[-113.9,25.5],[-113.5,25.5],[-113.5,25.8],[-113.9,25.8],[-113.9,25.5]]
        ]
    })
    .as_object()
    .unwrap()
    .clone();
    let mut filtered = response(product(), false);
    filtered.stats.matched_count = None;
    filtered.stats.fetched_record_count = 2;
    filtered.stats.mapped_record_count = 2;
    filtered.stats.unique_provider_product_count = 2;
    let database = TempStore::new("polygon-hole-local-filter");
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog { response: filtered }),
        Arc::new(FixtureWorker),
    );
    let region_id = common::ensure_fixture_region(&service, "polygon-hole-local-filter");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "polygon-hole-bind".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_polygon".to_owned(),
            geometry: geometry.clone(),
            bbox: vec![-114.0, 21.0, -109.0, 26.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "ALOS-2 hole fixture AOI".to_owned(),
            bound_at: "2026-07-29T00:00:00Z".to_owned(),
        })
        .unwrap();
    let mut operation = request("polygon-hole-local-filter", false);
    operation.search.aoi_geojson = Some(geometry.clone());
    operation.aoi_geojson = Some(geometry);

    service
        .catalog_search_and_freeze_legacy(&operation)
        .expect("a hole AOI accepts a complete conservative batch with exact local filtering");
    service.shutdown().unwrap();
}

#[test]
fn freezes_a_bounded_working_set_when_remote_total_is_larger() {
    let mut bounded = response(product(), false);
    bounded.stats.requested_limit = Some(1);
    bounded.stats.matched_count = Some(549_855);
    bounded.stats.remote_exhausted = false;
    let database = TempStore::new("bounded-remote-total");
    let service = service(&database, bounded);
    let mut operation = request("bounded-remote-total", false);
    operation.search.result_limit = Some(1);

    let snapshot = service
        .catalog_search_and_freeze_legacy(&operation)
        .expect("remote total does not invalidate a finite working set");
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("bounded batch freezes");
    let stats = batch.search_stats.expect("typed search stats freeze");
    assert_eq!(stats.matched_count, Some(549_855));
    assert_eq!(stats.fetched_record_count, Some(1));
    assert_eq!(stats.frozen_candidate_count, 1);
    // #22 B slice 3(a): meeting a finite target below the 10,000 ceiling is
    // a normal result, not possibly incomplete; the remote total stays.
    assert!(!stats.truncated);
    assert_eq!(batch.candidates.len(), 1);
    service.shutdown().unwrap();
}

#[test]
fn zero_hit_response_freezes_an_empty_search_batch_without_error() {
    let mut empty = response(product(), false);
    empty.products.clear();
    empty.stats.matched_count = Some(0);
    empty.stats.fetched_record_count = 0;
    empty.stats.mapped_record_count = 0;
    empty.stats.unique_provider_product_count = 0;
    empty.stats.exact_matched_count = 0;
    empty.stats.returned_count = 0;
    empty.stats.remote_exhausted = true;
    empty.stats.working_set_satisfied = true;
    let database = TempStore::new("zero-hit");
    let service = service(&database, empty);

    let snapshot = service
        .catalog_search_and_freeze_legacy(&request("zero-hit", false))
        .expect("zero hits are a successful bounded catalogue result");
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("empty SearchBatch is still frozen");
    assert!(batch.candidates.is_empty());
    let stats = batch
        .search_stats
        .expect("zero-hit stats remain authoritative");
    assert_eq!(stats.matched_count, Some(0));
    assert_eq!(stats.frozen_candidate_count, 0);
    assert!(!stats.truncated);
    service.shutdown().unwrap();
}

#[test]
fn rejects_primary_count_incomplete_response_candidate_size_and_resource_bound_without_store_writes(
) {
    let mut missing = product();
    missing.assets.clear();
    assert_rejected(
        "missing-primary",
        response(missing, false),
        false,
        "ASF_PRIMARY_ASSET_MISSING",
    );

    let mut ambiguous = product();
    let mut second = ambiguous.assets[0].clone();
    let item_id = ambiguous.properties["provider_product_key"]
        .as_str()
        .unwrap();
    let second_key = "second-authoritative.zip";
    second.asset_key = Some(second_key.to_owned());
    second.stable_asset_key = Some(stable_asset_key(item_id, second_key).unwrap());
    second.file_name = Some(second_key.to_owned());
    second.access_descriptor.as_mut().unwrap().stable_href =
        Some(format!("https://example.invalid/alos2/{second_key}"));
    ambiguous.assets.push(second);
    assert_rejected(
        "ambiguous-primary",
        response(ambiguous, false),
        false,
        "ASF_PRIMARY_ASSET_AMBIGUOUS",
    );

    let mut mismatched_locator = product();
    let asset_key = mismatched_locator.assets[0]
        .asset_key
        .as_deref()
        .unwrap()
        .to_owned();
    let wrong_href = "https://example.invalid/alos2/not-the-authoritative-archive.zip";
    mismatched_locator.assets[0]
        .access_descriptor
        .as_mut()
        .unwrap()
        .stable_href = Some(wrong_href.to_owned());
    let routes = mismatched_locator.properties["asset_routes"][&asset_key]
        .as_array_mut()
        .unwrap();
    routes
        .iter_mut()
        .find(|route| route["scheme"] == "https")
        .unwrap()["locator"] = Value::String(wrong_href.to_owned());
    assert_rejected(
        "locator-basename-mismatch",
        response(mismatched_locator, false),
        false,
        "ASF_ASSET_CLASSIFICATION_INVALID",
    );

    let mut continuation = response(product(), false);
    continuation.next_cursor = Some("internal-page".to_owned());
    continuation.stats.truncated = true;
    assert_rejected(
        "continuation",
        continuation,
        false,
        "CATALOGUE_RESPONSE_INCOMPLETE",
    );

    let mut item_error = response(product(), false);
    item_error
        .item_errors
        .push(ContractError::new("FIXTURE_ITEM", "fixture mapping failure", Map::new()).unwrap());
    item_error.stats.matched_count = Some(2);
    item_error.stats.fetched_record_count = 2;
    assert_rejected(
        "item-error",
        item_error,
        false,
        "CATALOGUE_RESPONSE_INCOMPLETE",
    );

    let mut oversized = product();
    oversized
        .properties
        .insert("padding".to_owned(), Value::String("x".repeat(70_000)));
    assert_rejected(
        "candidate-size",
        response(oversized, false),
        false,
        "CATALOGUE_PRODUCT_INVALID",
    );

    let mut over_limit = response(product(), false);
    over_limit.stats.matched_count = Some(40_001);
    over_limit.stats.fetched_record_count = 40_001;
    over_limit.stats.mapped_record_count = 40_001;
    over_limit.stats.exact_duplicate_record_count = 40_000;
    assert_rejected(
        "result-limit",
        over_limit,
        false,
        "CATALOGUE_RESULT_LIMIT_EXCEEDED",
    );
}

#[test]
fn rejects_independent_truncation_count_and_collection_profile_failures() {
    let mut truncated = response(product(), false);
    truncated.stats.truncated = true;
    assert_rejected(
        "truncated-only",
        truncated,
        false,
        "CATALOGUE_RESPONSE_INVALID",
    );

    let mut wrong_count = response(product(), false);
    wrong_count.stats.mapped_record_count = 0;
    assert_rejected(
        "wrong-count",
        wrong_count,
        false,
        "CATALOGUE_RESPONSE_INVALID",
    );

    let mut wrong_profile = response(product(), false);
    wrong_profile.collection.collection_profile_id = "asf.alos-palsar.v2".to_owned();
    assert_rejected(
        "wrong-profile",
        wrong_profile,
        false,
        "CATALOGUE_RESPONSE_INVALID",
    );
}

#[test]
fn cancellation_after_provider_return_prevents_alos2_publication() {
    let database = TempStore::new("cancel-publication");
    let gate = Arc::new(PublicationGate {
        entered: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let hook_gate = Arc::clone(&gate);
    let hook: Arc<dyn Fn() + Send + Sync> = Arc::new(move || hook_gate.wait());
    let service = Arc::new(
        ApplicationService::with_catalogue_fixture_business_runtime_with_publication_hook(
            ApplicationServiceConfig::business_store(&database.0),
            Arc::new(FixtureAsf),
            Arc::new(StaticCatalog {
                response: response(product(), false),
            }),
            Arc::new(FixtureWorker),
            Some(hook),
        ),
    );
    bind_aoi(service.as_ref(), "cancel-publication", false);
    let operation = request("cancel-publication", false);
    let progress = Arc::new(Mutex::new(Vec::<SarSearchProgressUpdate>::new()));
    let worker = {
        let service = Arc::clone(&service);
        let progress = Arc::clone(&progress);
        thread::spawn(move || {
            service.catalog_search_and_freeze_with_progress(&operation, |update| {
                progress.lock().unwrap().push(update);
            })
        })
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !gate.entered.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(gate.entered.load(Ordering::Acquire));
    assert!(service.cancel_request("cancel-publication").unwrap());
    gate.release.store(true, Ordering::Release);
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.code(), "CATALOGUE_REQUEST_CANCELLED");
    let progress = progress.lock().unwrap();
    assert_eq!(
        progress.last().unwrap().stage,
        SarSearchProgressStage::Cancelled
    );
    assert_eq!(
        progress[progress.len() - 2].stage,
        SarSearchProgressStage::Cancelling
    );
    assert!(progress.last().unwrap().terminal);
    assert_eq!(progress.last().unwrap().provider_request_count, Some(1));
    assert_eq!(progress.last().unwrap().raw_fetched_count, Some(1));
    assert_eq!(progress.last().unwrap().mapped_count, Some(1));
    assert_eq!(progress.last().unwrap().exact_matched_count, Some(1));
    assert_eq!(progress.last().unwrap().admitted_count, Some(1));
    assert_eq!(progress.last().unwrap().requested_limit, Some(25));
    assert_eq!(progress.last().unwrap().remote_total, Some(1));
    assert_empty_store(&service);
    service.shutdown().unwrap();
}
