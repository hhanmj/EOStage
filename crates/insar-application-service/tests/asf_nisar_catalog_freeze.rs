use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationCatalogSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig,
};
use insar_core::{
    provider_product_key, stable_asset_key, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AssetRole, CatalogCollectionDescriptor, CatalogDeliveryModel, CatalogSearchQuery,
    CatalogSearchResponse, CatalogSearchStats, ContractError, DownloadItemProgress,
    DownloadItemResult, DownloadLayout, ProviderProduct, ProviderResult, SearchWorkbenchState,
    StoreCreateTaskRequest, StoreUpdateWorkbenchRequest, WorkbenchSelectionCommand,
    WorkbenchSelectionCommandKind,
};
use insar_download::{
    evaluate_resolver_cutover_eligibility, AsfCollectionProfileRegistry,
    AssetExecutionPolicyRegistry, DownloadWork, DownloadWorker, ResolverCutoverEligibility,
    WorkerControl,
};
use insar_provider::{
    asf::{map_nisar_rslc_umm_granule, CmrGranuleEnvelope},
    catalog::{
        CollectionProfileRegistry, NISAR_FILENAME_RULE_ID, NISAR_IDENTITY_RULE_ID,
        NISAR_LOGICAL_DATASET, NISAR_NATIVE_COLLECTION_ID,
        NISAR_P05023_PRODUCT_SPECIFICATION_VERSION,
        NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION, NISAR_PROFILE_ID, NISAR_PROVIDER_ID,
        NISAR_RESOLVER_POLICY_ID, NISAR_RESOLVER_POLICY_ID_V2,
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
            "insar-nisar-freeze-{label}-{}-{id}.sqlite3",
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

fn fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-nisar-l-p1/granule-provisional-multi-asset-valid.json");
    let mut fixture: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("NISAR fixture exists"))
            .expect("NISAR fixture is valid JSON");
    fixture["collection"]["cmr_provider_id"] = json!("ASF");
    fixture
}

fn product() -> ProviderProduct {
    let envelope = CmrGranuleEnvelope::try_from(&fixture()).unwrap();
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    map_nisar_rslc_umm_granule(&envelope, profile).expect("valid NISAR product")
}

fn captured_product() -> ProviderProduct {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asf/nisar/provisional-rslc/granule-partial.json");
    let captured: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("captured NISAR fixture exists"))
            .expect("captured NISAR fixture is valid JSON");
    let raw = captured["cmr_response"]["items"][0].clone();
    let envelope = CmrGranuleEnvelope::try_from(&raw).unwrap();
    let profile = CollectionProfileRegistry
        .get(NISAR_PROFILE_ID)
        .expect("NISAR profile exists");
    map_nisar_rslc_umm_granule(&envelope, profile).expect("captured NISAR product maps")
}

fn aoi(multi: bool) -> Map<String, Value> {
    let value = if multi {
        json!({
            "type":"MultiPolygon",
            "coordinates":[[[[117.0,30.0],[120.0,30.0],[120.0,32.0],[117.0,32.0],[117.0,30.0]]]]
        })
    } else {
        json!({
            "type":"Polygon",
            "coordinates":[[[117.0,30.0],[120.0,30.0],[120.0,32.0],[117.0,32.0],[117.0,30.0]]]
        })
    };
    value.as_object().unwrap().clone()
}

fn search(multi: bool) -> CatalogSearchQuery {
    serde_json::from_value(json!({
        "logical_dataset": NISAR_LOGICAL_DATASET,
        "provider_id": NISAR_PROVIDER_ID,
        "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
        "aoi_geojson": aoi(multi),
        "cursor": null,
        "result_limit": 25,
        "filters": {
            "datetime": {"start":"2026-06-18T00:00:00Z", "end":"2026-06-19T00:00:00Z"},
            "production_configuration": "PR",
            "frame_coverage": "PARTIAL",
            "frequency_a_polarization": "HH+HV",
            "frequency_b_polarization": "HH",
            "range_bandwidth": "20+5",
            "joint_observation": true,
            "flight_direction": "ASCENDING",
            "look_direction": "RIGHT",
            "track": 37,
            "frame": 112
        }
    }))
    .expect("closed NISAR catalogue query")
}

fn request(label: &str, multi: bool) -> ApplicationCatalogSearchAndFreezeRequest {
    ApplicationCatalogSearchAndFreezeRequest {
        request_id: label.to_owned(),
        search: search(multi),
        batch_id: format!("{label}-batch"),
        created_at: "2026-07-29T00:00:00Z".to_owned(),
        aoi_name: "NISAR fixture AOI".to_owned(),
        aoi_geojson: Some(aoi(multi)),
        workbench_page_size: 24,
    }
}

fn collection() -> CatalogCollectionDescriptor {
    CatalogCollectionDescriptor {
        logical_dataset: NISAR_LOGICAL_DATASET.to_owned(),
        provider_id: NISAR_PROVIDER_ID.to_owned(),
        native_collection_id: NISAR_NATIVE_COLLECTION_ID.to_owned(),
        collection_profile_id: NISAR_PROFILE_ID.to_owned(),
        identity_rule_id: NISAR_IDENTITY_RULE_ID.to_owned(),
        filename_rule_id: NISAR_FILENAME_RULE_ID.to_owned(),
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

fn counting_service(
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
        bbox: vec![117.0, 30.0, 120.0, 32.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "NISAR fixture AOI".to_owned(),
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
fn freezes_rich_asset_graph_once_and_only_the_primary_hdf5_is_executable() {
    let database = TempStore::new("valid");
    let original = product();
    let primary = original
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::Primary)
        .unwrap()
        .clone();
    let rich_assets = original.properties["assets"].clone();
    let service = service(&database, response(original.clone(), false));

    let snapshot = service
        .catalog_search_and_freeze_legacy(&request("valid", false))
        .expect("valid NISAR complete response freezes atomically");
    let batch = snapshot.restore.workbench_batch.expect("one active batch");
    assert_eq!(batch.candidates.len(), 1);
    let candidate = &batch.candidates[0];
    let expected_item_id = provider_product_key(
        NISAR_PROVIDER_ID,
        NISAR_NATIVE_COLLECTION_ID,
        &original.product_id,
    )
    .unwrap();
    assert_eq!(candidate.item_id, expected_item_id);
    assert_eq!(candidate.provider_id, NISAR_PROVIDER_ID);
    assert_eq!(candidate.metadata["dataset"], NISAR_LOGICAL_DATASET);
    assert_eq!(candidate.metadata["logical_dataset"], NISAR_LOGICAL_DATASET);
    assert_eq!(
        candidate.metadata["collection_profile_id"],
        NISAR_PROFILE_ID
    );
    assert_eq!(
        candidate.metadata["resolver_policy_id"],
        NISAR_RESOLVER_POLICY_ID
    );
    assert_eq!(candidate.metadata["source_kind"], "sar");
    assert_eq!(candidate.metadata["product_kind"], "sar_product");
    assert_eq!(
        candidate.metadata["product_family"],
        "NISAR_L1_RSLC_PROVISIONAL_V1"
    );
    assert_eq!(candidate.metadata["processing_level"], "RSLC");
    assert_eq!(candidate.metadata["quality_status"], "Provisional");
    assert_eq!(
        candidate.metadata["resolver_policy_id"],
        NISAR_RESOLVER_POLICY_ID_V2
    );
    assert_eq!(candidate.metadata["properties"]["crid"], "P05023");
    assert_eq!(
        candidate.metadata["properties"]["product_specification_version"],
        NISAR_P05023_PRODUCT_SPECIFICATION_VERSION
    );
    assert_eq!(
        candidate.metadata["properties"]["product_specification_profile_registry_version"],
        NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION
    );
    assert_eq!(candidate.metadata["assets"], rich_assets);
    assert!(candidate.metadata["properties"].get("assets").is_none());
    assert!(candidate.metadata["properties"]
        .get("asset_integrity")
        .is_none());
    assert!(candidate.metadata["properties"]
        .get("asset_routes")
        .is_none());
    assert!(candidate.payload.is_empty());

    let access = candidate
        .asset_access
        .as_ref()
        .expect("the unique science HDF5 is executable");
    let asset_key = primary.asset_key.as_deref().unwrap();
    assert_eq!(access.reference.provider_id, NISAR_PROVIDER_ID);
    assert_eq!(access.reference.collection_id, NISAR_NATIVE_COLLECTION_ID);
    assert_eq!(access.reference.provider_product_id, original.product_id);
    assert_eq!(access.reference.provider_product_key, expected_item_id);
    assert_eq!(access.reference.asset_key, asset_key);
    assert_eq!(
        access.reference.stable_asset_key,
        primary.stable_asset_key.unwrap()
    );
    assert_eq!(access.descriptor, primary.access_descriptor.unwrap());
    assert_eq!(access.role, AssetRole::Primary);
    assert_eq!(access.file_name, primary.file_name);
    assert_eq!(access.expected_size_bytes, Some(12_000_000_000));
    assert_eq!(access.checksum_algorithm.as_deref(), Some("SHA256"));
    assert_eq!(
        access.checksum_value.as_deref(),
        Some("f2c78f09a49e5f1f304b19f930dfed10f197d235e383075e8377ddaa7c33d181")
    );
    let policy = candidate
        .execution_policy
        .as_ref()
        .expect("NISAR execution policy is frozen");
    assert_eq!(policy.policy_id, NISAR_RESOLVER_POLICY_ID);
    assert_eq!(
        policy.collection_profile_id.as_deref(),
        Some(NISAR_PROFILE_ID)
    );
    service.shutdown().unwrap();
}

#[test]
fn provider_preview_survives_store_and_scheduler_compaction_and_download_stays_eligible() {
    let database = TempStore::new("preview-contract-alignment");
    let provider_product = captured_product();
    assert_eq!(
        provider_product
            .assets
            .iter()
            .filter(|asset| asset.role == AssetRole::Primary)
            .count(),
        1
    );
    assert_eq!(
        provider_product
            .assets
            .iter()
            .filter(|asset| asset.role == AssetRole::Preview)
            .count(),
        1
    );

    let frozen_aoi = json!({
        "type": "Polygon",
        "coordinates": [[[-179.0, 63.0], [-173.0, 63.0], [-173.0, 66.0], [-179.0, 66.0], [-179.0, 63.0]]]
    })
    .as_object()
    .unwrap()
    .clone();
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(FixtureAsf),
        Arc::new(StaticCatalog {
            response: response(provider_product, false),
        }),
        Arc::new(FixtureWorker),
    );
    let region_id = common::ensure_fixture_region(&service, "preview-contract-alignment");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "preview-contract-alignment-bind-aoi".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: frozen_aoi.clone(),
            bbox: vec![-179.0, 63.0, -173.0, 66.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "NISAR captured fixture AOI".to_owned(),
            bound_at: "2026-07-29T00:00:00Z".to_owned(),
        })
        .unwrap();
    let operation = ApplicationCatalogSearchAndFreezeRequest {
        request_id: "preview-contract-alignment".to_owned(),
        search: serde_json::from_value(json!({
            "logical_dataset": NISAR_LOGICAL_DATASET,
            "provider_id": NISAR_PROVIDER_ID,
            "native_collection_id": NISAR_NATIVE_COLLECTION_ID,
            "aoi_geojson": frozen_aoi.clone(),
            "cursor": null,
            "result_limit": 25,
            "filters": {
                "datetime": {"start":"2026-07-23T07:48:00Z", "end":"2026-07-23T07:49:00Z"},
                "production_configuration": "PR",
                "frame_coverage": "PARTIAL",
                "frequency_a_polarization": "HH",
                "range_bandwidth": "5",
                "joint_observation": false,
                "flight_direction": "DESCENDING",
                "look_direction": "LEFT",
                "track": 45,
                "frame": 56
            }
        }))
        .unwrap(),
        batch_id: "preview-contract-alignment-batch".to_owned(),
        created_at: "2026-07-29T00:00:00Z".to_owned(),
        aoi_name: "NISAR captured fixture AOI".to_owned(),
        aoi_geojson: Some(frozen_aoi),
        workbench_page_size: 24,
    };
    let frozen = service
        .catalog_search_and_freeze_legacy(&operation)
        .expect("Provider Primary + Preview graph freezes atomically");
    let batch = frozen.restore.workbench_batch.expect("one frozen batch");
    let candidate = batch.candidates[0].clone();
    let original_bytes = serde_json::to_vec(&candidate).unwrap();
    let frozen_assets = candidate.metadata["assets"].as_array().unwrap();
    assert_eq!(
        frozen_assets
            .iter()
            .filter(|asset| asset["role"] == "preview")
            .count(),
        1
    );
    assert_eq!(candidate.metadata["properties"]["preview_available"], true);
    let access = candidate.asset_access.as_ref().expect("primary access");
    assert_eq!(access.role, AssetRole::Primary);

    service
        .update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: SearchWorkbenchState {
                batch_id: batch.batch_id.clone(),
                checked_item_ids: vec![candidate.item_id.clone()],
                page_index: 0,
                page_size: 24,
                revision: 1,
            },
            expected_revision: 0,
            updated_at: "2026-07-29T00:00:01Z".to_owned(),
        })
        .unwrap();
    service.shutdown().unwrap();

    let mut store = TaskStore::open(&database.0).unwrap();
    let stored_batch = store
        .load_search_batch(&batch.batch_id)
        .unwrap()
        .expect("Provider graph roundtrips through Store");
    assert_eq!(stored_batch.candidates[0], candidate);
    assert_eq!(
        serde_json::to_vec(&stored_batch.candidates[0]).unwrap(),
        original_bytes
    );
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "preview-contract-alignment-task".to_owned(),
            batch_id: batch.batch_id,
            selected_item_ids: vec![candidate.item_id.clone()],
            checked_item_ids: vec![candidate.item_id.clone()],
            page_index: 0,
            page_size: 24,
            output_root: std::env::temp_dir().join("nisar-preview-contract-alignment-output"),
            layout: DownloadLayout::SarDatasetProcessing,
            concurrency: 1,
            created_at: "2026-07-29T00:00:02Z".to_owned(),
        })
        .unwrap();
    let compact = store
        .load_scheduler_task("preview-contract-alignment-task")
        .unwrap()
        .expect("scheduler compact snapshot");
    assert_eq!(compact.snapshot.candidates.len(), 1);
    let compact_candidate = &compact.snapshot.candidates[0];
    assert_eq!(compact_candidate, &candidate);
    assert_eq!(
        serde_json::to_vec(compact_candidate).unwrap(),
        original_bytes
    );
    assert_eq!(
        compact_candidate.metadata["assets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|asset| asset["role"] == "preview")
            .count(),
        1
    );

    let eligibility = evaluate_resolver_cutover_eligibility(
        compact_candidate,
        &AsfCollectionProfileRegistry,
        &AssetExecutionPolicyRegistry,
    );
    let ResolverCutoverEligibility::Eligible(eligible) = eligibility else {
        panic!("the compact Provider Preview graph must remain Download-eligible: {eligibility:?}");
    };
    assert_eq!(eligible.policy.approved_asset_roles, &[AssetRole::Primary]);
    assert_eq!(eligible.frozen_access.role, AssetRole::Primary);
    assert_eq!(
        eligible.frozen_access.reference.asset_key,
        access.reference.asset_key
    );
}

#[test]
fn accepts_complete_multipolygon_duplicates_and_polygon_local_filter_counts() {
    let mut duplicate = response(product(), true);
    duplicate.stats.fetched_record_count = 2;
    duplicate.stats.mapped_record_count = 2;
    duplicate.stats.exact_duplicate_record_count = 1;
    let database = TempStore::new("multipolygon-duplicate");
    let duplicate_service = service(&database, duplicate);
    duplicate_service
        .catalog_search_and_freeze_legacy(&request("multipolygon-duplicate", true))
        .expect("cross-polygon exact duplicate is a complete batch");
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
fn accepts_frequency_a_only_b_only_and_dual_l_band_products() {
    for (label, removed_filter, removed_property) in [
        (
            "frequency-a-only",
            "frequency_b_polarization",
            "frequency_b_polarization",
        ),
        (
            "frequency-b-only",
            "frequency_a_polarization",
            "frequency_a_polarization",
        ),
    ] {
        let mut accepted = product();
        accepted.properties.remove(removed_property);
        let database = TempStore::new(label);
        let service = service(&database, response(accepted, false));
        let mut operation = request(label, false);
        operation.search.filters.remove(removed_filter);
        service
            .catalog_search_and_freeze_legacy(&operation)
            .expect("one declared L-band frequency remains a complete product");
        service.shutdown().unwrap();
    }

    let database = TempStore::new("frequency-dual");
    let service = service(&database, response(product(), false));
    service
        .catalog_search_and_freeze_legacy(&request("frequency-dual", false))
        .expect("dual-frequency L-band product remains valid");
    service.shutdown().unwrap();
}

#[test]
fn rejects_quality_scope_and_frequency_drift_with_stable_codes() {
    let mut beta = product();
    beta.properties["quality_status"] = json!("Beta");
    assert_rejected(
        "beta",
        response(beta, false),
        false,
        "NISAR_QUALITY_STATUS_MISMATCH",
    );

    for (label, key, value) in [
        ("product-type", "processing_level", json!("RIFG")),
        ("production", "production_configuration", json!("OD")),
        ("band", "band", json!("S")),
    ] {
        let mut invalid = product();
        invalid.properties[key] = value;
        assert_rejected(
            label,
            response(invalid, false),
            false,
            "ASF_PRODUCT_SCOPE_MISMATCH",
        );
    }

    let mut frequencyless = product();
    frequencyless.properties.remove("frequency_a_polarization");
    frequencyless.properties.remove("frequency_b_polarization");
    assert_rejected(
        "frequencyless",
        response(frequencyless, false),
        false,
        "ASF_PRODUCT_SCOPE_MISMATCH",
    );
}

#[test]
fn rejects_missing_unknown_or_mismatched_product_specification_profiles() {
    let mut missing_crid = product();
    missing_crid.properties.remove("crid");
    assert_rejected(
        "missing-crid",
        response(missing_crid, false),
        false,
        "ASF_PRODUCT_SCOPE_MISMATCH",
    );

    let mut unknown_crid = product();
    unknown_crid.properties["crid"] = json!("P99999");
    assert_rejected(
        "unknown-crid",
        response(unknown_crid, false),
        false,
        "ASF_PRODUCT_SCOPE_MISMATCH",
    );

    let mut mismatched_specification = product();
    mismatched_specification.properties["product_specification_version"] = json!("1.2.1");
    assert_rejected(
        "mismatched-specification",
        response(mismatched_specification, false),
        false,
        "ASF_PRODUCT_SCOPE_MISMATCH",
    );

    let mut missing_registry_version = product();
    missing_registry_version
        .properties
        .remove("product_specification_profile_registry_version");
    assert_rejected(
        "missing-profile-registry-version",
        response(missing_registry_version, false),
        false,
        "ASF_PRODUCT_SCOPE_MISMATCH",
    );

    let mut mismatched_registry_version = product();
    mismatched_registry_version.properties["product_specification_profile_registry_version"] =
        json!(NISAR_PRODUCT_SPECIFICATION_PROFILE_REGISTRY_VERSION + 1);
    assert_rejected(
        "mismatched-profile-registry-version",
        response(mismatched_registry_version, false),
        false,
        "ASF_PRODUCT_SCOPE_MISMATCH",
    );
}

#[test]
fn rejects_primary_and_cross_graph_conflicts_without_store_writes() {
    let mut missing = product();
    missing
        .assets
        .retain(|asset| asset.role != AssetRole::Primary);
    assert_rejected(
        "missing-primary",
        response(missing, false),
        false,
        "ASF_PRIMARY_ASSET_MISSING",
    );

    let mut ambiguous = product();
    let mut second = ambiguous
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::Primary)
        .unwrap()
        .clone();
    let item_id = ambiguous.properties["provider_product_key"]
        .as_str()
        .unwrap();
    let second_key = "second-science-hdf5";
    second.asset_key = Some(second_key.to_owned());
    second.stable_asset_key = Some(stable_asset_key(item_id, second_key).unwrap());
    second.file_name = Some("NISAR_L1_PR_RSLC_SYNTHETIC_P05023_0002.h5".to_owned());
    ambiguous.assets.push(second);
    assert_rejected(
        "ambiguous-primary",
        response(ambiguous, false),
        false,
        "ASF_PRIMARY_ASSET_AMBIGUOUS",
    );

    let mut ancillary_descriptor = product();
    let descriptor = ancillary_descriptor
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::Primary)
        .unwrap()
        .access_descriptor
        .clone();
    ancillary_descriptor
        .assets
        .iter_mut()
        .find(|asset| asset.role == AssetRole::Preview)
        .unwrap()
        .access_descriptor = descriptor;
    assert_rejected(
        "ancillary-descriptor",
        response(ancillary_descriptor, false),
        false,
        "ASF_ASSET_CLASSIFICATION_INVALID",
    );

    let mut route_conflict = product();
    let primary_key = route_conflict.properties["primary_asset_key"]
        .as_str()
        .unwrap()
        .to_owned();
    route_conflict.properties["asset_routes"][&primary_key][0]["locator"] =
        json!("https://example.invalid/nisar/wrong.h5");
    assert_rejected(
        "route-conflict",
        response(route_conflict, false),
        false,
        "ASF_ASSET_CLASSIFICATION_INVALID",
    );

    let mut s_band_evidence = product();
    s_band_evidence.properties["assets"][0]["native_evidence"]["url_types"] =
        json!(["GET DATA", "S-BAND PRODUCT"]);
    assert_rejected(
        "s-band-asset-evidence",
        response(s_band_evidence, false),
        false,
        "ASF_ASSET_CLASSIFICATION_INVALID",
    );

    let mut checksum_without_size = product();
    let browse_key = checksum_without_size
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::Preview)
        .unwrap()
        .asset_key
        .clone()
        .unwrap();
    let browse = checksum_without_size.properties["assets"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|asset| asset["asset_key"] == browse_key)
        .unwrap();
    browse["checksum"] = json!({
        "algorithm":"MD5",
        "value":"00000000000000000000000000000000"
    });
    checksum_without_size.properties["asset_integrity"][&browse_key] = json!({
        "file_name": browse["file_name"],
        "size_bytes": null,
        "algorithm":"MD5",
        "value":"00000000000000000000000000000000"
    });
    assert_rejected(
        "checksum-without-size",
        response(checksum_without_size, false),
        false,
        "ASF_ASSET_CLASSIFICATION_INVALID",
    );
}

#[test]
fn rejects_incomplete_counts_graph_limits_and_resource_bound_without_store_writes() {
    let mut continuation = response(product(), false);
    continuation.next_cursor = Some("internal-page".to_owned());
    continuation.stats.truncated = true;
    assert_rejected(
        "continuation",
        continuation,
        false,
        "CATALOGUE_RESPONSE_INCOMPLETE",
    );

    let mut wrong_count = response(product(), false);
    wrong_count.stats.mapped_record_count = 0;
    assert_rejected(
        "wrong-count",
        wrong_count,
        false,
        "CATALOGUE_RESPONSE_INVALID",
    );

    let mut too_many_assets = product();
    let template = too_many_assets.properties["assets"][0].clone();
    too_many_assets.properties["assets"] =
        Value::Array((0..33).map(|_| template.clone()).collect());
    assert_rejected(
        "asset-count-limit",
        response(too_many_assets, false),
        false,
        "ASF_ASSET_GRAPH_LIMIT_EXCEEDED",
    );

    let mut oversized_graph = product();
    oversized_graph.properties["assets"][0]["padding"] = json!("x".repeat(41_000));
    assert_rejected(
        "asset-json-limit",
        response(oversized_graph, false),
        false,
        "ASF_ASSET_GRAPH_LIMIT_EXCEEDED",
    );

    let mut oversized_candidate = product();
    let mut ring = vec![
        json!([117.0, 30.0]),
        json!([120.0, 30.0]),
        json!([120.0, 32.0]),
    ];
    ring.extend((0..6_000).map(|_| json!([118.0, 31.0])));
    ring.push(json!([117.0, 30.0]));
    oversized_candidate.footprint_geojson = json!({
        "type":"Polygon",
        "coordinates":[ring]
    })
    .as_object()
    .cloned();
    assert_rejected(
        "candidate-size",
        response(oversized_candidate, false),
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
fn cancellation_after_provider_return_prevents_nisar_publication() {
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
    let worker = {
        let service = Arc::clone(&service);
        thread::spawn(move || service.catalog_search_and_freeze_legacy(&operation))
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
    assert_empty_store(&service);
    service.shutdown().unwrap();
}

#[test]
fn identical_nisar_typed_query_reuses_cache_resets_workbench_and_survives_store_reopen() {
    let database = TempStore::new("typed-cache-reopen");
    let calls = Arc::new(AtomicU64::new(0));
    let service = counting_service(&database, response(product(), false), Arc::clone(&calls));
    bind_aoi(&service, "typed-cache-reopen", false);

    let first_progress = std::sync::Mutex::new(Vec::new());
    service
        .catalog_search_and_freeze_with_progress(&request("typed-cache-first", false), |update| {
            first_progress
                .lock()
                .unwrap()
                .push(update.provider_request_count);
        })
        .expect("first typed query freezes provider candidates");
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        first_progress.lock().unwrap().last().copied().flatten(),
        Some(1),
    );
    let first_batch = service
        .snapshot(true)
        .unwrap()
        .restore
        .workbench_batch
        .expect("first frozen workbench batch");
    let first_item_id = first_batch
        .candidates
        .first()
        .expect("fixture has one candidate")
        .item_id
        .clone();

    let checked = service
        .update_workbench_selection(&WorkbenchSelectionCommand {
            batch_id: first_batch.batch_id.clone(),
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
            batch_id: first_batch.batch_id.clone(),
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
    let second_progress = std::sync::Mutex::new(Vec::new());
    service
        .catalog_search_and_freeze_with_progress(&second_request, |update| {
            second_progress
                .lock()
                .unwrap()
                .push(update.provider_request_count);
        })
        .expect("identical typed query freezes a new batch from cache");
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "cache hit must not call the provider again",
    );
    assert_eq!(
        second_progress.lock().unwrap().last().copied().flatten(),
        Some(0),
    );
    let second_snapshot = service.snapshot(true).unwrap();
    let second_batch = second_snapshot
        .restore
        .workbench_batch
        .as_ref()
        .expect("cached frozen workbench batch");
    let second_workbench = second_snapshot
        .restore
        .workbench
        .as_ref()
        .expect("cached workbench selection state");
    assert_ne!(second_batch.batch_id, first_batch.batch_id);
    assert_eq!(second_batch.query, first_batch.query);
    assert_eq!(second_batch.candidates, first_batch.candidates);
    assert_eq!(second_workbench.batch_id, second_batch.batch_id);
    assert_eq!(second_workbench.revision, 0);
    assert!(second_workbench.checked_item_ids.is_empty());
    assert_eq!(second_workbench.page_index, 0);
    assert_eq!(second_workbench.page_size, 11);
    service.shutdown().unwrap();

    let reopened_store = TaskStore::open(database.0.as_path()).unwrap();
    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let reopened_batch = reopened_store
        .load_catalog_cache_batch(&search(false), now_epoch, 24 * 60 * 60)
        .unwrap()
        .expect("NISAR cache pointer survives Store reopen");
    assert_eq!(reopened_batch.query, first_batch.query);
    assert_eq!(reopened_batch.candidates, first_batch.candidates);
    drop(reopened_store);

    let reopened_calls = Arc::new(AtomicU64::new(0));
    let reopened_service = counting_service(
        &database,
        response(product(), false),
        Arc::clone(&reopened_calls),
    );
    let mut reopened_request = request("typed-cache-after-reopen", false);
    reopened_request.workbench_page_size = 13;
    let reopened_progress = std::sync::Mutex::new(Vec::new());
    reopened_service
        .catalog_search_and_freeze_with_progress(&reopened_request, |update| {
            reopened_progress
                .lock()
                .unwrap()
                .push(update.provider_request_count);
        })
        .expect("NISAR cache hit works after reopening ApplicationService");
    assert_eq!(reopened_calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        reopened_progress.lock().unwrap().last().copied().flatten(),
        Some(0),
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
    assert_ne!(reopened_active_batch.batch_id, first_batch.batch_id);
    assert_eq!(reopened_active_batch.query, first_batch.query);
    assert_eq!(reopened_active_batch.candidates, first_batch.candidates);
    assert_eq!(reopened_workbench.batch_id, reopened_active_batch.batch_id);
    assert!(reopened_workbench.checked_item_ids.is_empty());
    assert_eq!(reopened_workbench.page_index, 0);
    assert_eq!(reopened_workbench.page_size, 13);
    assert_eq!(reopened_workbench.revision, 0);
    reopened_service.shutdown().unwrap();
}
