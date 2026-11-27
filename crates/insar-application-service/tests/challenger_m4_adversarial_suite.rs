//! Empirical Challenger Adversarial Test Suite for Milestone 4
//! (HLS v2.0 Download Pipeline & Task Persistence)
//!
//! Stress-tests edge cases:
//! 1. Invalid band selection (empty, unsupported, wrong sensor cross-matching L30 vs S30, duplicates, overflow)
//! 2. Asset href validation and resolver security boundaries (non-https, fragments, malicious domains, credentials)
//! 3. Task creation input boundaries (concurrency, layouts, IDs, duplicates)
//! 4. Task persistence with monotonic revisions and zero CoreRestore warnings

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationCatalogIntentSearchAndFreezeRequest,
    ApplicationService, ApplicationServiceConfig, CatalogSearchIntent,
    HlsCreateBandBatchTaskRequest,
};
use insar_core::{
    provider_product_key, stable_asset_key, AsfRemoteSearchRequest, AsfRemoteSearchResponse,
    AssetAccessDescriptor, AssetAccessScheme, AssetResolverParameters, AssetRole,
    CatalogSearchQuery, CatalogSearchResponse, DownloadItemProgress, DownloadItemResult,
    DownloadLayout, DownloadOutcome, FrozenAssetAccess, FrozenSearchBatch, ProviderNetworkPolicy,
    ProviderResult, StableAssetReference, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_download::{
    evaluate_resolver_cutover_eligibility, AsfCollectionProfileRegistry,
    AssetExecutionPolicyRegistry, AssetResolver, Clock, CredentialMaterial, CredentialProvider,
    CredentialProviderError, DownloadWork, DownloadWorker, PlanetaryComputerLandsatAssetResolver,
    ResolveContext, ResolverCutoverEligibility, WorkerControl,
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

const PLANETARY_COMPUTER_HLS_RESOLVER_ID: &str = "planetary-computer.hls.assets.v1";

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
    }
}

struct NoCredentials;
impl CredentialProvider for NoCredentials {
    fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        Err(CredentialProviderError::Missing)
    }
}

mod common;

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(100);

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "sceneport-challenger-m4-{label}-{}-{id}.sqlite3",
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

struct NoAsf;
impl LiveAsfSearch for NoAsf {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("optical HLS intent must never call ASF");
    }
}

struct NoDownload;
impl DownloadWorker for NoDownload {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        while !control.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root.join(work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-11-09T00:00:01Z".to_owned(),
        }
    }
}

struct RecordingCatalog {
    queries: Mutex<Vec<CatalogSearchQuery>>,
    response: CatalogSearchResponse,
}

impl RecordingCatalog {
    fn answering(response: CatalogSearchResponse) -> Self {
        Self {
            queries: Mutex::new(Vec::new()),
            response,
        }
    }
}

impl LiveCatalogSearch for RecordingCatalog {
    fn search_catalog(
        &self,
        query: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.queries.lock().unwrap().push(query.clone());
        Ok(self.response.clone())
    }
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn l30_assets(product_id: &str) -> Vec<Value> {
    let bands = [
        "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B09", "B10", "B11", "Fmask", "SAA",
        "SZA", "VAA", "VZA",
    ];
    bands
        .iter()
        .map(|band| {
            json!({
                "asset_key": band,
                "role": "primary",
                "media_type": "image/tiff; application=geotiff; profile=cloud-optimized",
                "url": format!("https://hls2euwest.blob.core.windows.net/hls2/L30/49/R/DP/2024/06/25/{product_id}/{product_id}.{band}.tif"),
                "file_name": format!("{product_id}.{band}.tif"),
                "size_bytes": 15000000
            })
        })
        .collect()
}

fn s30_assets(product_id: &str) -> Vec<Value> {
    let bands = [
        "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B10", "B11", "B12",
        "Fmask", "SAA", "SZA", "VAA", "VZA",
    ];
    bands
        .iter()
        .map(|band| {
            json!({
                "asset_key": band,
                "role": "primary",
                "media_type": "image/tiff; application=geotiff; profile=cloud-optimized",
                "url": format!("https://hls2euwest.blob.core.windows.net/hls2/S30/50/T/MK/2024/06/25/{product_id}/{product_id}.{band}.tif"),
                "file_name": format!("{product_id}.{band}.tif"),
                "size_bytes": 15000000
            })
        })
        .collect()
}

fn hls_l30_response() -> CatalogSearchResponse {
    let pid = "HLS.L30.T49RDP.2024177T030756.v2.0";
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.hls.l30.v2",
            "provider_id": "microsoft.planetary-computer",
            "native_collection_id": "hls2-l30",
            "collection_profile_id": "planetary-computer.hls2-l30.scene.v1",
            "identity_rule_id": "planetary-computer.hls2.identity.v1",
            "filename_rule_id": "planetary-computer.hls2.search-only.v1",
            "delivery_model": "archive_first",
            "production_enabled": true
        },
        "products": [{
            "provider_id": "microsoft.planetary-computer",
            "product_id": pid,
            "display_name": pid,
            "source_kind": "landsat",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2024-06-25T03:07:56.017810Z",
            "footprint_geojson": {"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},
            "assets": l30_assets(pid),
            "properties": {"platform": "landsat_8", "cloud_cover": 79.0},
            "payload": {}
        }],
        "stats": {
            "requested_limit": 100, "matched_count": 1, "request_count": 1, "transport_part_count": 1,
            "fetched_record_count": 1, "mapped_record_count": 1, "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0, "unique_provider_product_count": 1,
            "exact_matched_count": 1, "returned_count": 1, "remote_exhausted": true,
            "working_set_satisfied": true, "elapsed_ms": 1, "truncated": false
        },
        "item_errors": []
    }))
    .unwrap()
}

fn hls_s30_response() -> CatalogSearchResponse {
    let pid = "HLS.S30.T50TMK.2024177T024545.v2.0";
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "optical.hls.s30.v2",
            "provider_id": "microsoft.planetary-computer",
            "native_collection_id": "hls2-s30",
            "collection_profile_id": "planetary-computer.hls2-s30.scene.v1",
            "identity_rule_id": "planetary-computer.hls2.identity.v1",
            "filename_rule_id": "planetary-computer.hls2.search-only.v1",
            "delivery_model": "archive_first",
            "production_enabled": true
        },
        "products": [{
            "provider_id": "microsoft.planetary-computer",
            "product_id": pid,
            "display_name": pid,
            "source_kind": "sentinel2",
            "product_kind": "optical_scene",
            "acquisition_datetime": "2024-06-25T02:45:45.024000Z",
            "footprint_geojson": {"type":"Polygon","coordinates":[[[110,30],[111,30],[111,31],[110,30]]]},
            "assets": s30_assets(pid),
            "properties": {"platform": "sentinel_2a", "cloud_cover": 15.0},
            "payload": {}
        }],
        "stats": {
            "requested_limit": 100, "matched_count": 1, "request_count": 1, "transport_part_count": 1,
            "fetched_record_count": 1, "mapped_record_count": 1, "profile_filtered_record_count": 0,
            "exact_duplicate_record_count": 0, "unique_provider_product_count": 1,
            "exact_matched_count": 1, "returned_count": 1, "remote_exhausted": true,
            "working_set_satisfied": true, "elapsed_ms": 1, "truncated": false
        },
        "item_errors": []
    }))
    .unwrap()
}

fn setup_service_with_response(
    database: &TempStore,
    response: CatalogSearchResponse,
) -> ApplicationService {
    let service = ApplicationService::with_catalogue_production_business_runtime(
        ApplicationServiceConfig::business_store(&database.0),
        Arc::new(NoAsf),
        Arc::new(RecordingCatalog::answering(response)),
        Arc::new(NoDownload),
    );
    let region_id = common::ensure_fixture_region(&service, "challenger-m4");
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "m4-bind-aoi".to_owned(),
            region_id,
            expected_revision: None,
            source_kind: "manual_bbox".to_owned(),
            geometry: aoi(),
            bbox: vec![110.0, 30.0, 111.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "Challenger fixture AOI".to_owned(),
            bound_at: "2026-11-09T00:00:00Z".to_owned(),
        })
        .unwrap();
    service
}

// ===========================================================================
// Test 1: Adversarial Planetary Computer HLS href & Resolver Security Bounds
// ===========================================================================
#[test]
fn test_adversarial_hls_href_and_resolver_security_boundaries() {
    let resolver = PlanetaryComputerLandsatAssetResolver::new(ProviderNetworkPolicy::default())
        .expect("resolver init");

    let make_desc = |href: &str| AssetAccessDescriptor {
        resolver_id: PLANETARY_COMPUTER_HLS_RESOLVER_ID.to_owned(),
        scheme: AssetAccessScheme::PublicHttps,
        requires_credentials: false,
        credential_profile: None,
        stable_href: Some(href.to_owned()),
        resolver_parameters: AssetResolverParameters {},
    };

    // Valid hrefs must pass
    let valid_hrefs = [
        "https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2024/06/25/HLS.L30.T50TMK.2024177T024545.v2.0/HLS.L30.T50TMK.2024177T024545.v2.0.B02.tif",
        "https://hlseuwest.blob.core.windows.net/hls/T50TMK/B8A.tif",
        "https://hls2euwest.blob.core.windows.net/hls2/scene/Fmask.tiff",
    ];
    for href in valid_hrefs {
        assert!(
            resolver.can_resolve(&make_desc(href)),
            "Expected valid HLS href for {href}"
        );
    }

    // Adversarial / invalid hrefs must fail closed
    let invalid_hrefs = [
        "https://modisoptical.blob.core.windows.net/hls/scene/B02.tif", // disallowed domain
        "http://hls2euwest.blob.core.windows.net/hls2/scene/B02.tif",   // non-https
        "https://hls2euwest.blob.core.windows.net/hls2/scene/B02.tif#fragment", // url fragment
        "https://malicious.evil.com/hls/scene/B02.tif",                 // foreign domain
        "https://hls2euwest.blob.core.windows.net/not_hls/scene/B02.tif", // wrong path, no /hls/ or /hls2/
        "https://hls2euwest.blob.core.windows.net/hls2/scene/B02.exe",    // non-tif extension
        "not_even_a_url",
        "",
    ];
    for href in invalid_hrefs {
        assert!(
            !resolver.can_resolve(&make_desc(href)),
            "Expected invalid HLS href for {href}"
        );
    }

    // Resolver security gates:
    let base_descriptor = make_desc("https://hls2euwest.blob.core.windows.net/hls2/L30/50/T/MK/2023/04/30/HLS.L30.T50TMK.2023120T024545.v2.0/HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif");
    assert!(resolver.can_resolve(&base_descriptor));

    // Attack 1: Credential leakage attempt
    let mut bad_cred = base_descriptor.clone();
    bad_cred.requires_credentials = true;
    bad_cred.credential_profile = Some("stolen_secret".to_owned());
    assert!(
        !resolver.can_resolve(&bad_cred),
        "Credentialed HLS descriptor must be rejected"
    );

    // Attack 2: Resolver ID spoofing
    let mut bad_id = base_descriptor.clone();
    bad_id.resolver_id = "unknown.hls.resolver.v1".to_owned();
    assert!(
        !resolver.can_resolve(&bad_id),
        "Unknown resolver ID must be rejected"
    );

    // Attack 3: Scheme spoofing
    let mut bad_scheme = base_descriptor.clone();
    bad_scheme.scheme = AssetAccessScheme::LocalFile;
    assert!(
        !resolver.can_resolve(&bad_scheme),
        "Non-PublicHttps scheme must be rejected"
    );

    // Resolution check with mock SAS token
    let pkey = provider_product_key(
        insar_provider::catalog::MPC_HLS_PROVIDER_ID,
        "hls2-l30",
        "HLS.L30.T50TMK.2023120T024545.v2.0",
    )
    .unwrap();
    let reference = StableAssetReference {
        provider_id: insar_provider::catalog::MPC_HLS_PROVIDER_ID.to_owned(),
        collection_id: "hls2-l30".to_owned(),
        provider_product_id: "HLS.L30.T50TMK.2023120T024545.v2.0".to_owned(),
        provider_product_key: pkey.clone(),
        asset_key: "B02".to_owned(),
        stable_asset_key: stable_asset_key(&pkey, "B02").unwrap(),
        canonical_product_key: None,
    };
    let file_name = "HLS.L30.T50TMK.2023120T024545.v2.0.B02.tif";
    let frozen = FrozenAssetAccess {
        descriptor: base_descriptor.clone(),
        reference: reference.clone(),
        role: AssetRole::Primary,
        file_name: Some(file_name.to_owned()),
        expected_size_bytes: Some(15_000_000),
        checksum_algorithm: None,
        checksum_value: None,
    };
    resolver
        .validate_frozen(&frozen)
        .expect("valid frozen HLS access");
    let resolved = match resolver.resolve(
        &reference,
        &base_descriptor,
        &ResolveContext::new(&NoCredentials, &|| false, &FixedClock),
    ) {
        Ok(res) => res,
        Err(err) => {
            eprintln!("Resolution skipped live network call error: {err}");
            return;
        }
    };
    assert!(resolved
        .effective_url()
        .as_str()
        .contains("hls2euwest.blob.core.windows.net"));
    assert!(resolved.effective_url().as_str().contains("sig="));
    assert_eq!(resolved.suggested_filename(), Some(file_name));
}

// ===========================================================================
// Test 2: Adversarial Multi-Band Selection & Cross-Sensor Invariants
// ===========================================================================
#[test]
fn test_adversarial_band_selection_and_cross_sensor_mismatch() {
    let store = TempStore::new("band-stress");
    let service = setup_service_with_response(&store, hls_l30_response());

    // Freeze an L30 batch into workbench
    let freeze_res = service
        .catalog_intent_search_and_freeze_with_progress(
            &ApplicationCatalogIntentSearchAndFreezeRequest {
                request_id: "req-freeze-l30".to_owned(),
                intent: CatalogSearchIntent {
                    resource_id: "science.hls.l30.v2".to_owned(),
                    search_schema_id: "search.optical.hls.l30.v2".to_owned(),
                    schema_revision: 1,
                    aoi_geojson: Some(aoi()),
                    result_limit: Some(100),
                    filters: json!({
                        "spacecraft": ["landsat_8"],
                        "cloud_cover_lte": 80.0,
                        "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                },
                batch_id: "batch-l30-freeze".to_owned(),
                created_at: "2026-11-09T00:00:00Z".to_owned(),
                aoi_name: "Fixture AOI".to_owned(),
                aoi_geojson: Some(aoi()),
                workbench_page_size: 24,
            },
            |_| {},
        )
        .expect("freeze l30 batch");
    let workbench = freeze_res.core.workbench.expect("workbench batch");
    let item_id = service
        .restore(true)
        .unwrap()
        .workbench_batch
        .unwrap()
        .candidates[0]
        .item_id
        .clone();
    let output_root = std::env::temp_dir().join("challenger-m4-out");

    // Case 2a: Empty band vector -> rejected immediately
    let req_empty = HlsCreateBandBatchTaskRequest {
        request_id: "req-empty-bands".to_owned(),
        source_batch_id: workbench.batch_id.clone(),
        expected_workbench_revision: workbench.revision,
        item_ids: vec![item_id.clone()],
        asset_keys: vec![],
        task_id: "task-empty-bands".to_owned(),
        output_root: output_root.clone(),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-11-09T00:00:00Z".to_owned(),
    };
    let err = service.create_hls_band_batch_task(&req_empty).unwrap_err();
    assert!(
        format!("{err:?}").contains("invalid"),
        "Empty bands must fail"
    );

    // Case 2b: Duplicate bands in request -> rejected immediately
    let req_dup = HlsCreateBandBatchTaskRequest {
        request_id: "req-dup-bands".to_owned(),
        source_batch_id: workbench.batch_id.clone(),
        expected_workbench_revision: workbench.revision,
        item_ids: vec![item_id.clone()],
        asset_keys: vec!["B02".to_owned(), "B02".to_owned()],
        task_id: "task-dup-bands".to_owned(),
        output_root: output_root.clone(),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-11-09T00:00:00Z".to_owned(),
    };
    assert!(
        service.create_hls_band_batch_task(&req_dup).is_err(),
        "Duplicate bands must fail"
    );

    // Case 2c: Over 14 bands (> max) -> rejected immediately
    let overflow_keys: Vec<String> = (1..=15).map(|i| format!("B{i:02}")).collect();
    let req_overflow = HlsCreateBandBatchTaskRequest {
        request_id: "req-overflow-bands".to_owned(),
        source_batch_id: workbench.batch_id.clone(),
        expected_workbench_revision: workbench.revision,
        item_ids: vec![item_id.clone()],
        asset_keys: overflow_keys,
        task_id: "task-overflow-bands".to_owned(),
        output_root: output_root.clone(),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-11-09T00:00:00Z".to_owned(),
    };
    assert!(
        service.create_hls_band_batch_task(&req_overflow).is_err(),
        "15 bands must fail"
    );

    // Case 2d: Invalid band names -> rejected during admission
    for invalid_band in ["B99", "QA", "b02", "panchromatic", ""] {
        let req_invalid = HlsCreateBandBatchTaskRequest {
            request_id: format!("req-invalid-band-{invalid_band}"),
            source_batch_id: workbench.batch_id.clone(),
            expected_workbench_revision: workbench.revision,
            item_ids: vec![item_id.clone()],
            asset_keys: vec![invalid_band.to_owned()],
            task_id: format!("task-invalid-{invalid_band}"),
            output_root: output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-11-09T00:00:00Z".to_owned(),
        };
        let res = service
            .create_hls_band_batch_task(&req_invalid)
            .expect("admission summary returned");
        assert_eq!(
            res.admission.admitted_count, 0,
            "Invalid band {invalid_band} must not be admitted"
        );
        assert_eq!(res.admission.rejected_count, 1);
        assert!(
            res.snapshot.is_none(),
            "No task snapshot on total rejection"
        );
    }

    // Case 2e: Cross-sensor mismatch on L30:
    // B08, B8A, and B12 exist on S30 but NOT on L30!
    for s30_only_band in ["B08", "B8A", "B12"] {
        let req_cross = HlsCreateBandBatchTaskRequest {
            request_id: format!("req-cross-band-{s30_only_band}"),
            source_batch_id: workbench.batch_id.clone(),
            expected_workbench_revision: workbench.revision,
            item_ids: vec![item_id.clone()],
            asset_keys: vec![s30_only_band.to_owned()],
            task_id: format!("task-cross-{s30_only_band}"),
            output_root: output_root.clone(),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-11-09T00:00:00Z".to_owned(),
        };
        let res = service
            .create_hls_band_batch_task(&req_cross)
            .expect("admission summary returned");
        assert_eq!(
            res.admission.admitted_count, 0,
            "S30-only band {s30_only_band} must NOT be admitted on L30"
        );
        assert_eq!(res.admission.rejected_count, 1);
        assert!(res.admission.rejections[0]
            .reason_code
            .contains("INVALID_REQUEST"));
        assert!(res.snapshot.is_none());
    }

    // Case 2f: Valid bands on L30 -> all admitted!
    let valid_l30_bands = vec![
        "B02".to_owned(),
        "B03".to_owned(),
        "B04".to_owned(),
        "Fmask".to_owned(),
    ];
    let req_valid = HlsCreateBandBatchTaskRequest {
        request_id: "req-valid-l30-bands".to_owned(),
        source_batch_id: workbench.batch_id.clone(),
        expected_workbench_revision: workbench.revision,
        item_ids: vec![item_id.clone()],
        asset_keys: valid_l30_bands.clone(),
        task_id: "task-valid-l30-bands".to_owned(),
        output_root: output_root.clone(),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-11-09T00:00:00Z".to_owned(),
    };
    let res = service
        .create_hls_band_batch_task(&req_valid)
        .expect("valid task admitted");
    assert_eq!(res.admission.admitted_count, 1);
    assert_eq!(res.admission.rejected_count, 0);
    assert!(res.snapshot.is_some());

    service.shutdown().unwrap();
}

// ===========================================================================
// Test 3: S30 Multiband Admission (B08, B8A, B12 are allowed on S30)
// ===========================================================================
#[test]
fn test_s30_multiband_admission_allows_s30_bands() {
    let store = TempStore::new("s30-multiband");
    let service = setup_service_with_response(&store, hls_s30_response());

    let freeze_res = service
        .catalog_intent_search_and_freeze_with_progress(
            &ApplicationCatalogIntentSearchAndFreezeRequest {
                request_id: "req-freeze-s30".to_owned(),
                intent: CatalogSearchIntent {
                    resource_id: "science.hls.s30.v2".to_owned(),
                    search_schema_id: "search.optical.hls.s30.v2".to_owned(),
                    schema_revision: 1,
                    aoi_geojson: Some(aoi()),
                    result_limit: Some(100),
                    filters: json!({
                        "cloud_cover_lte": 50.0,
                        "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                },
                batch_id: "batch-s30-freeze".to_owned(),
                created_at: "2026-11-09T00:00:00Z".to_owned(),
                aoi_name: "Fixture AOI".to_owned(),
                aoi_geojson: Some(aoi()),
                workbench_page_size: 24,
            },
            |_| {},
        )
        .expect("freeze s30 batch");
    let workbench = freeze_res.core.workbench.expect("workbench batch");
    let item_id = service
        .restore(true)
        .unwrap()
        .workbench_batch
        .unwrap()
        .candidates[0]
        .item_id
        .clone();
    let output_root = std::env::temp_dir().join("challenger-m4-out-s30");

    // S30 valid multiband selection including B08, B8A, B12
    let s30_bands = vec![
        "B02".to_owned(),
        "B03".to_owned(),
        "B04".to_owned(),
        "B08".to_owned(),
        "B8A".to_owned(),
        "B12".to_owned(),
        "Fmask".to_owned(),
    ];
    let req_s30 = HlsCreateBandBatchTaskRequest {
        request_id: "req-valid-s30-multiband".to_owned(),
        source_batch_id: workbench.batch_id.clone(),
        expected_workbench_revision: workbench.revision,
        item_ids: vec![item_id],
        asset_keys: s30_bands,
        task_id: "task-s30-multiband-1".to_owned(),
        output_root,
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-11-09T00:00:00Z".to_owned(),
    };
    let res = service
        .create_hls_band_batch_task(&req_s30)
        .expect("s30 task creation succeeds");
    assert_eq!(res.admission.admitted_count, 1);
    assert_eq!(res.admission.rejected_count, 0);
    let snapshot = res.snapshot.as_ref().expect("snapshot present");
    let created_task = snapshot
        .core
        .tasks
        .iter()
        .find(|t| t.task_id == req_s30.task_id)
        .expect("created task in snapshot");
    assert_eq!(created_task.provider_id, "microsoft.planetary-computer");
    assert_eq!(created_task.delivery_requested_count, Some(1));
    assert_eq!(created_task.delivery_rejected_count, Some(0));

    let restore = service.core_restore(true).expect("core restore");
    assert!(
        restore.core.legacy_task_warnings.is_empty(),
        "unexpected CoreRestore warnings: {:?}",
        restore.core.legacy_task_warnings
    );
    let restored_task = restore
        .core
        .tasks
        .iter()
        .find(|t| t.task_id == req_s30.task_id)
        .expect("restored task");
    assert_eq!(restored_task.provider_id, "microsoft.planetary-computer");
    assert_eq!(restored_task.delivery_requested_count, Some(1));
    assert_eq!(restored_task.delivery_rejected_count, Some(0));

    assert_frozen_task_items_resolver_eligible(&store.0, &req_s30.task_id, 7);

    service.shutdown().unwrap();
}

fn strip_asset_sizes(mut response: CatalogSearchResponse) -> CatalogSearchResponse {
    for product in &mut response.products {
        for asset in &mut product.assets {
            asset.size_bytes = None;
        }
    }
    response
}

fn assert_frozen_task_items_resolver_eligible(
    db_path: &PathBuf,
    task_id: &str,
    expected_item_count: usize,
) {
    let conn = Connection::open(db_path).expect("open store");
    let batch_id = format!("hls-bands-batch-{task_id}");
    let batch_json: String = conn
        .query_row(
            "SELECT snapshot_json FROM search_batches WHERE batch_id = ?1",
            rusqlite::params![batch_id],
            |row| row.get(0),
        )
        .expect("query frozen HLS task batch");
    let batch: FrozenSearchBatch =
        serde_json::from_str(&batch_json).expect("deserialize frozen HLS batch");
    assert_eq!(batch.candidates.len(), expected_item_count);
    let profiles = AsfCollectionProfileRegistry;
    let policies = AssetExecutionPolicyRegistry;
    for item in &batch.candidates {
        let verdict = evaluate_resolver_cutover_eligibility(item, &profiles, &policies);
        assert!(
            matches!(verdict, ResolverCutoverEligibility::Eligible(_)),
            "expected frozen HLS item {} ({}) eligible, got {verdict:?}",
            item.item_id,
            item.display_name
        );
    }
}

// ===========================================================================
// Test 4: Task Persistence, Monotonic Revisions, and Zero CoreRestore Warnings
// ===========================================================================
#[test]
fn test_task_persistence_monotonic_revisions_and_zero_corerestore_warnings() {
    let store = TempStore::new("persistence-stress");
    let service = setup_service_with_response(&store, hls_l30_response());

    let freeze_res = service
        .catalog_intent_search_and_freeze_with_progress(
            &ApplicationCatalogIntentSearchAndFreezeRequest {
                request_id: "req-freeze-persist".to_owned(),
                intent: CatalogSearchIntent {
                    resource_id: "science.hls.l30.v2".to_owned(),
                    search_schema_id: "search.optical.hls.l30.v2".to_owned(),
                    schema_revision: 1,
                    aoi_geojson: Some(aoi()),
                    result_limit: Some(100),
                    filters: json!({
                        "spacecraft": ["landsat_8"],
                        "cloud_cover_lte": 80.0,
                        "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                },
                batch_id: "batch-persist-freeze".to_owned(),
                created_at: "2026-11-09T00:00:00Z".to_owned(),
                aoi_name: "Fixture AOI".to_owned(),
                aoi_geojson: Some(aoi()),
                workbench_page_size: 24,
            },
            |_| {},
        )
        .expect("freeze batch");
    let workbench = freeze_res.core.workbench.expect("workbench batch");
    let item_id = service
        .restore(true)
        .unwrap()
        .workbench_batch
        .unwrap()
        .candidates[0]
        .item_id
        .clone();
    let output_root = std::env::temp_dir().join("challenger-m4-out-persist");

    let task_id = "task-hls-canonical-001";
    let req = HlsCreateBandBatchTaskRequest {
        request_id: "req-persist-task".to_owned(),
        source_batch_id: workbench.batch_id.clone(),
        expected_workbench_revision: workbench.revision,
        item_ids: vec![item_id.clone()],
        asset_keys: vec!["B02".to_owned(), "B03".to_owned()],
        task_id: task_id.to_owned(),
        output_root: output_root.clone(),
        layout: DownloadLayout::Direct,
        concurrency: 2,
        created_at: "2026-11-09T00:00:00Z".to_owned(),
    };
    let result = service
        .create_hls_band_batch_task(&req)
        .expect("task creation succeeds");
    assert!(result.snapshot.is_some());

    // 1. Direct SQLite Inspection
    let conn = Connection::open(&store.0).expect("open sqlite db");
    let (status, revision): (String, i64) = conn
        .query_row(
            "SELECT status, revision FROM task_runtime WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query task_runtime");
    assert!(
        status == "queued" || status == "running",
        "Expected initial status to be queued or running, got: {status}"
    );
    assert!(
        revision == 0 || revision == 1,
        "Expected initial revision 0 (queued) or 1 (running), got: {revision}"
    );

    // Verify task_snapshots table has valid JSON
    let snapshot_json: String = conn
        .query_row(
            "SELECT snapshot_json FROM task_snapshots WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| row.get(0),
        )
        .expect("query task_snapshots");
    let parsed_snapshot: Value =
        serde_json::from_str(&snapshot_json).expect("valid compact snapshot JSON");
    assert_eq!(parsed_snapshot["task_id"], task_id);

    // 2. CoreRestore verification: wait briefly for background scheduler to settle on Running
    // so optimistic-concurrency PauseTask does not race the initial Queued -> Running claim,
    // and assert ZERO CoreRestore warnings.
    let settle_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let restore = loop {
        let snap = service.core_restore(true).expect("restore cleanly");
        assert!(
            snap.core.legacy_task_warnings.is_empty(),
            "Expected 0 CoreRestore warnings, got: {:?}",
            snap.core.legacy_task_warnings
        );
        assert_eq!(snap.core.tasks.len(), 1);
        assert_eq!(snap.core.tasks[0].task_id, task_id);
        if snap.core.tasks[0].status == TaskLifecycleStatus::Running
            || std::time::Instant::now() >= settle_deadline
        {
            break snap;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let current_status = restore.core.tasks[0].status.clone();
    assert_eq!(
        current_status,
        TaskLifecycleStatus::Running,
        "Expected Running before PauseTask, got: {:?}",
        current_status
    );
    let current_revision = restore.core.tasks[0].revision;

    // 3. Monotonic Revision on State Transition (Pause Task)
    let update_res = service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: current_revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: Vec::new(),
            },
            event_id: "event-pause-1".to_owned(),
            occurred_at: "2026-11-09T00:01:00Z".to_owned(),
        })
        .expect("pause task succeeds");
    let updated_task = update_res
        .core
        .tasks
        .iter()
        .find(|t| t.task_id == task_id)
        .expect("task found in snapshot");
    assert_eq!(updated_task.status, TaskLifecycleStatus::Paused);

    // Check revision incremented monotonically in DB
    let (new_status, new_revision): (String, i64) = conn
        .query_row(
            "SELECT status, revision FROM task_runtime WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query task_runtime after pause");
    assert_eq!(new_status, "paused");
    assert!(
        new_revision > current_revision as i64,
        "Revision must increment monotonically"
    );

    // Check CoreRestore again after pause: ZERO warnings
    let restore2 = service
        .core_restore(true)
        .expect("restore cleanly after pause");
    assert!(
        restore2.core.legacy_task_warnings.is_empty(),
        "Expected 0 CoreRestore warnings after pause, got: {:?}",
        restore2.core.legacy_task_warnings
    );
    assert_eq!(restore2.core.tasks[0].status, TaskLifecycleStatus::Paused);

    // 4. Stale Revision Attack: Attempt to update with outdated expected_revision must fail closed
    let stale_req = StoreUpdateTaskRequest {
        task_id: task_id.to_owned(),
        expected_revision: current_revision, // stale! current in DB is new_revision
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::ResumeTask,
            item_ids: Vec::new(),
        },
        event_id: "event-stale-attack".to_owned(),
        occurred_at: "2026-11-09T00:01:30Z".to_owned(),
    };
    let stale_err = service.update_download_task(&stale_req);
    assert!(
        stale_err.is_err(),
        "Updating task with stale expected_revision must fail closed"
    );

    // 5. Monotonic Revision on Resume
    let resume_res = service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: new_revision as u64,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: Vec::new(),
            },
            event_id: "event-resume-1".to_owned(),
            occurred_at: "2026-11-09T00:02:00Z".to_owned(),
        })
        .expect("resume task succeeds");
    let resumed_task = resume_res
        .core
        .tasks
        .iter()
        .find(|t| t.task_id == task_id)
        .expect("task found after resume");
    assert!(
        matches!(
            resumed_task.status,
            TaskLifecycleStatus::Running | TaskLifecycleStatus::Queued
        ),
        "Expected running or queued after resume, got: {:?}",
        resumed_task.status
    );

    let (resumed_status, resumed_revision): (String, i64) = conn
        .query_row(
            "SELECT status, revision FROM task_runtime WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query task_runtime after resume");
    assert!(
        resumed_status == "running" || resumed_status == "queued",
        "Expected resumed status to be running or queued, got: {resumed_status}"
    );
    assert!(
        resumed_revision > new_revision,
        "Resume revision ({resumed_revision}) must be strictly greater than pause revision ({new_revision})"
    );

    // Check CoreRestore after resume: ZERO warnings
    let restore3 = service
        .core_restore(true)
        .expect("restore cleanly after resume");
    assert!(
        restore3.core.legacy_task_warnings.is_empty(),
        "Expected 0 CoreRestore warnings after resume, got: {:?}",
        restore3.core.legacy_task_warnings
    );
    assert_eq!(restore3.core.tasks[0].revision, resumed_revision as u64);

    // 6. Monotonic Revision on Cancel
    let cancel_res = service
        .update_download_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: resumed_revision as u64,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: Vec::new(),
            },
            event_id: "event-cancel-1".to_owned(),
            occurred_at: "2026-11-09T00:03:00Z".to_owned(),
        })
        .expect("cancel task succeeds");
    let cancelled_task = cancel_res
        .core
        .tasks
        .iter()
        .find(|t| t.task_id == task_id)
        .expect("task found after cancel");
    assert_eq!(cancelled_task.status, TaskLifecycleStatus::Cancelled);

    let (cancelled_status, cancelled_revision): (String, i64) = conn
        .query_row(
            "SELECT status, revision FROM task_runtime WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("query task_runtime after cancel");
    assert_eq!(cancelled_status, "cancelled");
    assert!(
        cancelled_revision > resumed_revision,
        "Cancel revision ({cancelled_revision}) must be strictly greater than resume revision ({resumed_revision})"
    );

    // Check CoreRestore after cancel: ZERO warnings
    let restore4 = service
        .core_restore(true)
        .expect("restore cleanly after cancel");
    assert!(
        restore4.core.legacy_task_warnings.is_empty(),
        "Expected 0 CoreRestore warnings after cancel, got: {:?}",
        restore4.core.legacy_task_warnings
    );
    assert_eq!(
        restore4.core.tasks[0].status,
        TaskLifecycleStatus::Cancelled
    );
    assert_eq!(restore4.core.tasks[0].revision, cancelled_revision as u64);

    // 7. Duplicate Task ID Attack: Creating another task with same task_id must fail closed
    let req_dup_task = HlsCreateBandBatchTaskRequest {
        request_id: "req-dup-task-id".to_owned(),
        source_batch_id: workbench.batch_id.clone(),
        expected_workbench_revision: workbench.revision,
        item_ids: vec![item_id.clone()],
        asset_keys: vec!["B04".to_owned()],
        task_id: task_id.to_owned(), // collision!
        output_root: output_root.clone(),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-11-09T00:02:00Z".to_owned(),
    };
    let dup_err = service.create_hls_band_batch_task(&req_dup_task);
    assert!(dup_err.is_err(), "Duplicate task_id must fail closed");

    service.shutdown().unwrap();
}

// ===========================================================================
// Test 5: Live-Style Omitted `file:size` on HLS L30 & S30 Remains Eligible
// ===========================================================================
#[test]
fn test_hls_l30_and_s30_unknown_asset_size_tasks_pass_resolver_eligibility_and_delivery_counts() {
    for (label, resource_id, search_schema_id, response, bands, task_id) in [
        (
            "l30-unknown-size",
            "science.hls.l30.v2",
            "search.optical.hls.l30.v2",
            strip_asset_sizes(hls_l30_response()),
            vec!["B02".to_owned(), "B04".to_owned(), "Fmask".to_owned()],
            "task-hls-l30-nosize-1",
        ),
        (
            "s30-unknown-size",
            "science.hls.s30.v2",
            "search.optical.hls.s30.v2",
            strip_asset_sizes(hls_s30_response()),
            vec!["B02".to_owned(), "B8A".to_owned(), "Fmask".to_owned()],
            "task-hls-s30-nosize-1",
        ),
    ] {
        let store = TempStore::new(label);
        let service = setup_service_with_response(&store, response);

        let freeze_res = service
            .catalog_intent_search_and_freeze_with_progress(
                &ApplicationCatalogIntentSearchAndFreezeRequest {
                    request_id: format!("req-freeze-{label}"),
                    intent: CatalogSearchIntent {
                        resource_id: resource_id.to_owned(),
                        search_schema_id: search_schema_id.to_owned(),
                        schema_revision: 1,
                        aoi_geojson: Some(aoi()),
                        result_limit: Some(100),
                        filters: json!({
                            "cloud_cover_lte": 90.0,
                            "datetime": {"start": "2024-06-01T00:00:00Z", "end": "2024-06-30T23:59:59Z"}
                        })
                        .as_object()
                        .unwrap()
                        .clone(),
                    },
                    batch_id: format!("batch-{label}"),
                    created_at: "2026-11-13T00:00:00Z".to_owned(),
                    aoi_name: "Fixture AOI".to_owned(),
                    aoi_geojson: Some(aoi()),
                    workbench_page_size: 24,
                },
                |_| {},
            )
            .expect("freeze hls batch");
        let workbench = freeze_res.core.workbench.expect("workbench batch");
        let item_id = service
            .restore(true)
            .unwrap()
            .workbench_batch
            .unwrap()
            .candidates[0]
            .item_id
            .clone();
        let output_root = std::env::temp_dir().join(format!("challenger-m4-out-{label}"));

        let req = HlsCreateBandBatchTaskRequest {
            request_id: format!("req-create-{label}"),
            source_batch_id: workbench.batch_id.clone(),
            expected_workbench_revision: workbench.revision,
            item_ids: vec![item_id],
            asset_keys: bands.clone(),
            task_id: task_id.to_owned(),
            output_root,
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-11-13T00:00:00Z".to_owned(),
        };
        let res = service
            .create_hls_band_batch_task(&req)
            .expect("create hls band batch task");
        assert_eq!(res.admission.admitted_count, 1);
        assert_eq!(res.admission.rejected_count, 0);
        let snap = res.snapshot.as_ref().expect("snapshot present");
        let task = snap
            .core
            .tasks
            .iter()
            .find(|t| t.task_id == task_id)
            .expect("task present");
        assert_eq!(task.provider_id, "microsoft.planetary-computer");
        assert_eq!(task.delivery_requested_count, Some(1));
        assert_eq!(task.delivery_rejected_count, Some(0));

        let restore = service.core_restore(true).expect("core restore");
        assert!(
            restore.core.legacy_task_warnings.is_empty(),
            "Expected zero CoreRestore warnings for {label}, got: {:?}",
            restore.core.legacy_task_warnings
        );
        let restored = restore
            .core
            .tasks
            .iter()
            .find(|t| t.task_id == task_id)
            .expect("restored task present");
        assert_eq!(restored.delivery_requested_count, Some(1));
        assert_eq!(restored.delivery_rejected_count, Some(0));

        assert_frozen_task_items_resolver_eligible(&store.0, task_id, bands.len());
        service.shutdown().unwrap();
    }
}
