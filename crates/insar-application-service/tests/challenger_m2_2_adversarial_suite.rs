//! Challenger 2 Empirical Test Suite: M2 (7 Missing Data Sources) Adversarial Challenge
//! Covers:
//! 1. ASTER GDEM v3 resolution in `dem_identity` across provider permutations, canonical casing, and invalid routes.
//! 2. DEM spatial resolution, datum semantics, and sample density checks.
//! 3. Unified search property-based and adversarial boundary input fuzzing across all 7 new sources.
//! 4. Strict rejection of unreviewed / conflicting filters and duplicate request IDs.

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, StoreDemSourceIdentity, UnifiedSearchRequest,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery, CatalogSearchResponse,
    DownloadItemProgress, DownloadItemResult, ProviderResult, ResourceDescriptor, ResourceRegistry,
    SearchProgressStage,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
};

static COUNTER: AtomicU64 = AtomicU64::new(5000);

struct TempStore(PathBuf);
impl TempStore {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "challenger2-m2-test-{}-{}.sqlite3",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
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

struct MockAsf;
impl LiveAsfSearch for MockAsf {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("Search must not call ASF directly")
    }
}

fn catalog_fixture() -> CatalogSearchResponse {
    serde_json::from_value(json!({
        "collection": {
            "logical_dataset": "sar.opera.s1.disp.v1",
            "provider_id": "asf.nasa.earthdata",
            "native_collection_id": "C2834279090-ASF",
            "collection_profile_id": "asf.opera.s1.collection.v1",
            "identity_rule_id": "asf.opera.s1.identity.v1",
            "filename_rule_id": "asf.opera.s1.filename.v1",
            "delivery_model": "archive_first",
            "production_enabled": true
        },
        "products": [],
        "stats": {
            "requested_limit": 100,
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
            "elapsed_ms": 1,
            "truncated": false
        },
        "item_errors": []
    }))
    .unwrap()
}

struct MockCatalog {
    pub query_count: Mutex<usize>,
    pub response: CatalogSearchResponse,
}
impl LiveCatalogSearch for MockCatalog {
    fn search_catalog(
        &self,
        _q: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        *self.query_count.lock().unwrap() += 1;
        Ok(self.response.clone())
    }
}

struct MockDownload;
impl DownloadWorker for MockDownload {
    fn execute(
        &self,
        _: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        panic!("Download worker must never be invoked during search verification")
    }
}

fn valid_geojson_polygon() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [
            [[113.0, 28.0], [114.0, 28.0], [114.0, 29.0], [113.0, 29.0], [113.0, 28.0]]
        ]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn create_test_service() -> (TempStore, Arc<ApplicationService>, Arc<MockCatalog>) {
    let db = TempStore::new();
    let catalog = Arc::new(MockCatalog {
        query_count: Mutex::new(0),
        response: catalog_fixture(),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(MockAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(MockDownload),
    ));
    (db, service, catalog)
}

// =========================================================================
// 1. ASTER GDEM v3 dem_identity Provider Permutation Verification
// =========================================================================

#[test]
fn test_aster_gdem_v3_dem_identity_provider_permutations() {
    let (_db, service, _catalog) = create_test_service();

    // Canonical dataset aliases and providers for ASTER GDEM v3
    let canonical_datasets = ["ASTER_GDEM_V3", "ASTGTM"];
    let canonical_providers = ["EARTHDATA", "LPDAAC", "ASF"];

    // 1. Verify all 2 x 3 = 6 canonical permutations succeed
    for dataset in &canonical_datasets {
        for provider in &canonical_providers {
            let identity = StoreDemSourceIdentity {
                dataset: (*dataset).to_owned(),
                provider: (*provider).to_owned(),
                updated_at: "2026-11-09T00:00:00Z".to_owned(),
            };
            let result = service.set_dem_source_identity(&identity);
            assert!(
                result.is_ok(),
                "Expected dem_identity to accept canonical dataset '{}' and provider '{}', got err: {:?}",
                dataset,
                provider,
                result.err()
            );
            let confirmed = result.unwrap();
            assert_eq!(confirmed.dataset, *dataset);
            assert_eq!(confirmed.provider, *provider);
        }
    }

    // 2. Canonical invariant check: Non-uppercase or untrimmed strings MUST be rejected
    // by `canonical_selected_dem_source_identity` (which enforces strict uppercase & trim)
    let non_canonical_pairs = [
        ("aster_gdem_v3", "EARTHDATA"),
        ("ASTGTM", "earthdata"),
        ("  ASTER_GDEM_V3  ", "ASF"),
        ("ASTGTM", "  LPDAAC  "),
        ("\tASTER_GDEM_V3", "EARTHDATA"),
        ("ASTER_GDEM_V3", "lpdaac\n"),
    ];

    for (dataset, provider) in &non_canonical_pairs {
        let identity = StoreDemSourceIdentity {
            dataset: (*dataset).to_owned(),
            provider: (*provider).to_owned(),
            updated_at: "2026-11-09T00:00:00Z".to_owned(),
        };
        let result = service.set_dem_source_identity(&identity);
        assert!(
            result.is_err(),
            "Non-canonical pair ({}, {}) must be rejected by canonical_selected_dem_source_identity",
            dataset,
            provider
        );
    }

    // 3. Invalid provider permutations: must reject and return error
    let invalid_permutations = [
        ("ASTER_GDEM_V3", "COPERNICUS_AWS"),
        ("ASTER_GDEM_V3", "OPENTOPOGRAPHY"),
        ("ASTER_GDEM_V3", "NISAR_DEM_ASF"),
        ("ASTGTM", "COPERNICUS_AWS"),
        ("ASTGTM", "OPENTOPOGRAPHY"),
        ("ASTGTM", "NISAR_DEM_ASF"),
        ("ASTER_GDEM_V3", "asf.nasa.earthdata"), // wire provider ID is not the short enum
        ("ASTER_GDEM_V3", "NASA"),
        ("ASTER_GDEM_V3", "LP_DAAC"),
        ("ASTER_GDEM_V3", ""),
        ("ASTER_GDEM_V3", "   "),
        ("ASTER_GDEM_V3", "UNKNOWN_PROVIDER"),
        ("ASTER_GDEM", "EARTHDATA"), // Missing _V3 suffix
        ("ASTER", "LPDAAC"),
        ("dem.aster-gdem.v3", "EARTHDATA"), // Resource ID is not dataset key
        ("ASTGTM.v003", "EARTHDATA"),       // Collection ID is not dataset key
        ("COP30", "LPDAAC"),
        ("COP30", "EARTHDATA"),
        ("COP90", "LPDAAC"),
        ("NASADEM", "EARTHDATA"),
        ("AW3D30", "ASF"),
    ];

    for (dataset, provider) in &invalid_permutations {
        let identity = StoreDemSourceIdentity {
            dataset: (*dataset).to_owned(),
            provider: (*provider).to_owned(),
            updated_at: "2026-11-09T00:00:00Z".to_owned(),
        };
        let result = service.set_dem_source_identity(&identity);
        assert!(
            result.is_err(),
            "Expected dem_identity to REJECT invalid permutation ({}, {}), but got Ok",
            dataset,
            provider
        );
    }

    service.shutdown().unwrap();
}

// =========================================================================
// 2. ASTER GDEM v3 Semantics, Resolution and Metadata Verification
// =========================================================================

#[test]
fn test_aster_gdem_v3_semantics_and_resolution() {
    let registry = ResourceRegistry::embedded().expect("embedded registry valid");
    let aster_res = registry
        .resources
        .iter()
        .find(|r| r.common().resource_id == "dem.aster-gdem.v3")
        .expect("dem.aster-gdem.v3 must be present in registry");

    let ResourceDescriptor::Dem { dem, common } = aster_res else {
        panic!("dem.aster-gdem.v3 must be ResourceDescriptor::Dem");
    };

    // Verify nominal resolution
    assert_eq!(
        dem.nominal_resolution, 30.0,
        "ASTER GDEM v3 nominal resolution must be 30m"
    );
    assert_eq!(dem.resolution_unit, "meters");
    assert_eq!(dem.horizontal_crs, "EPSG:4326");
    assert_eq!(dem.vertical_datum, "EGM96");
    assert_eq!(dem.runtime_dataset_id, "ASTER_GDEM_V3");

    // Verify provider route details
    assert_eq!(common.provider_routes.len(), 1);
    let route = &common.provider_routes[0];
    assert_eq!(route.route_id, "route.lpdaac.aster-gdem-v3.earthdata.v1");
    assert_eq!(route.provider_id, "asf.nasa.earthdata");
    assert_eq!(route.native_collection_id.as_deref(), Some("ASTGTM.v003"));
    assert_eq!(route.endpoint_class, "earthdata_cloud_cog");

    // Verify capability details
    assert_eq!(common.capabilities.len(), 1);
    let cap = &common.capabilities[0];
    assert_eq!(
        cap.capability_id,
        "capability.dem.aster-gdem-v3.download.v1"
    );
    assert_eq!(
        cap.asset_schema_id.as_deref(),
        Some("assets.dem.aster.geotiff.v1")
    );
    assert_eq!(
        cap.delivery_profile_id.as_deref(),
        Some("delivery.dem.aster.tile.v1")
    );

    // Verify delivery profile
    assert_eq!(common.delivery_profiles.len(), 1);
    let deliv = &common.delivery_profiles[0];
    assert_eq!(deliv.delivery_profile_id, "delivery.dem.aster.tile.v1");
    assert!(deliv.supports_range);
    assert!(deliv.supports_resume);
    assert!(deliv.atomic_promotion);
}

// =========================================================================
// 3. Property-Based & Adversarial Boundary Inputs across Unified Search
// =========================================================================

#[test]
fn test_unified_search_boundary_fuzzing_all_7_new_sources() {
    let (_db, service, catalog) = create_test_service();

    let all_7_new_sources = [
        "science.opera.disp-s1.v1",
        "science.comet.licsar.v1",
        "dem.aster-gdem.v3",
        "event.usgs.earthquake.v1",
        "geodesy.ngl.gnss.v1",
        "atmospheric.era5.reanalysis.v1",
        "ionospheric.igs.gim.v1",
    ];

    for resource_id in &all_7_new_sources {
        // --- Adversarial request_id checks ---
        let bad_request_ids = [
            "",
            "   ",
            "\t",
            "\n",
            " leading_space",
            "trailing_space ",
            "req\x00null",
            "req\nnewline",
            "req\rcarriage",
            &"A".repeat(129), // Max allowed is 128
        ];

        for bad_id in &bad_request_ids {
            let req = UnifiedSearchRequest {
                request_id: bad_id.to_string(),
                resource_id: (*resource_id).to_owned(),
                aoi: valid_geojson_polygon(),
                aoi_name: "TestAOI".to_owned(),
                time_range: None,
                filters: Map::new(),
            };
            assert!(
                service.search_start(req).is_err(),
                "Expected rejection for bad request_id on {}",
                resource_id
            );
        }

        // --- Boundary aoi_name checks ---
        let bad_names = [
            "",
            "   ",
            "\t\n ",
            &"N".repeat(161), // Max allowed is 160
        ];

        for bad_name in &bad_names {
            let req = UnifiedSearchRequest {
                request_id: format!("req-fuzz-name-{}", COUNTER.fetch_add(1, Ordering::Relaxed)),
                resource_id: (*resource_id).to_owned(),
                aoi: valid_geojson_polygon(),
                aoi_name: bad_name.to_string(),
                time_range: None,
                filters: Map::new(),
            };
            assert!(
                service.search_start(req).is_err(),
                "Expected rejection for bad aoi_name on {}",
                resource_id
            );
        }

        // --- Malformed AOI GeoJSON fuzzing ---
        let malformed_aois: &[Value] = &[
            json!({}),
            json!({"type": "Point", "coordinates": [113.0, 28.0]}),
            json!({"type": "LineString", "coordinates": [[113.0, 28.0], [114.0, 29.0]]}),
            json!({"type": "MultiPoint", "coordinates": [[113.0, 28.0]]}),
            json!({"type": "Polygon", "coordinates": []}),
            json!({"type": "Polygon", "coordinates": [[[113.0, 28.0], [114.0, 28.0], [114.0, 29.0]]]}), // < 4 points
            json!({"type": "Polygon", "coordinates": [[[113.0, 28.0], [114.0, 28.0], [114.0, 29.0], [113.0, 28.5]]]}), // unclosed
            json!({"type": "Polygon", "coordinates": [[[185.0, 28.0], [186.0, 28.0], [186.0, 29.0], [185.0, 28.0]]]}), // lon > 180
            json!({"type": "Polygon", "coordinates": [[[-185.0, 28.0], [-184.0, 28.0], [-184.0, 29.0], [-185.0, 28.0]]]}), // lon < -180
            json!({"type": "Polygon", "coordinates": [[[113.0, 95.0], [114.0, 95.0], [114.0, 96.0], [113.0, 95.0]]]}), // lat > 90
            json!({"type": "Polygon", "coordinates": [[[113.0, -95.0], [114.0, -95.0], [114.0, -96.0], [113.0, -95.0]]]}), // lat < -90
        ];

        for bad_aoi in malformed_aois {
            let req = UnifiedSearchRequest {
                request_id: format!("req-fuzz-aoi-{}", COUNTER.fetch_add(1, Ordering::Relaxed)),
                resource_id: (*resource_id).to_owned(),
                aoi: bad_aoi.as_object().unwrap_or(&Map::new()).clone(),
                aoi_name: "TestAOI".to_owned(),
                time_range: None,
                filters: Map::new(),
            };
            let res = service.search_start(req);
            if *resource_id == "science.opera.disp-s1.v1"
                || *resource_id == "science.comet.licsar.v1"
            {
                assert!(
                    res.is_err(),
                    "Expected rejection for malformed AOI on SAR resource {}",
                    resource_id
                );
            } else {
                // Auxiliary sources intercept before catalogue AOI validation and return terminal progress
                let progress =
                    res.expect("Auxiliary source should intercept with terminal progress");
                assert!(progress.terminal);
                assert_eq!(progress.stage, SearchProgressStage::Failed);
            }
        }
    }

    // Ensure zero catalogue queries were triggered during fuzz failures
    assert_eq!(*catalog.query_count.lock().unwrap(), 0);
    service.shutdown().unwrap();
}

// =========================================================================
// 4. Strict SAR and Auxiliary Filter Validation
// =========================================================================

#[test]
fn test_strict_filter_rejection_and_auxiliary_guidance() {
    let (_db, service, catalog) = create_test_service();

    // 1. OPERA S1: Unreviewed filter rejected
    let mut bad_opera = Map::new();
    bad_opera.insert("unknown_opera_key".to_owned(), json!(123));
    let req = UnifiedSearchRequest {
        request_id: "req-bad-opera-01".to_owned(),
        resource_id: "science.opera.disp-s1.v1".to_owned(),
        aoi: valid_geojson_polygon(),
        aoi_name: "OPERA_AOI".to_owned(),
        time_range: None,
        filters: bad_opera,
    };
    let res = service.search_start(req);
    assert!(res.is_err());
    assert!(format!("{:?}", res.err().unwrap()).contains("unreviewed SAR catalogue filter"));

    // 2. OPERA S1: Fixed filter conflict rejected
    let mut conflict_opera = Map::new();
    conflict_opera.insert("platform".to_owned(), json!("ALOS-2")); // Fixed is SENTINEL-1
    let req = UnifiedSearchRequest {
        request_id: "req-conflict-opera-01".to_owned(),
        resource_id: "science.opera.disp-s1.v1".to_owned(),
        aoi: valid_geojson_polygon(),
        aoi_name: "OPERA_AOI".to_owned(),
        time_range: None,
        filters: conflict_opera,
    };
    let res = service.search_start(req);
    assert!(res.is_err());
    assert!(format!("{:?}", res.err().unwrap()).contains("SAR fixed filter conflicts with source"));

    // 3. LiCSAR: Unreviewed filter rejected
    let mut bad_licsar = Map::new();
    bad_licsar.insert("polarization".to_owned(), json!("HH")); // Not in LiCSAR supported_keys
    let req = UnifiedSearchRequest {
        request_id: "req-bad-licsar-01".to_owned(),
        resource_id: "science.comet.licsar.v1".to_owned(),
        aoi: valid_geojson_polygon(),
        aoi_name: "LiCSAR_AOI".to_owned(),
        time_range: None,
        filters: bad_licsar,
    };
    let res = service.search_start(req);
    assert!(res.is_err());
    assert!(format!("{:?}", res.err().unwrap()).contains("unreviewed SAR catalogue filter"));

    // 4. All 5 Auxiliary Sources: Terminal progress with code SEARCH_SOURCE_AUXILIARY_UNSUPPORTED
    let aux_expectations = [
        ("dem.aster-gdem.v3", "DEM 无需在这里检索"),
        (
            "event.usgs.earthquake.v1",
            "USGS 地震事件目录无需在影像工作台检索",
        ),
        (
            "geodesy.ngl.gnss.v1",
            "NGL 全球 GNSS 基准站位移序列无需在影像工作台检索",
        ),
        (
            "atmospheric.era5.reanalysis.v1",
            "ERA5 大气延迟场无需在影像工作台检索",
        ),
        (
            "ionospheric.igs.gim.v1",
            "IGS GIM 电离层格网无需在影像工作台检索",
        ),
    ];

    for (res_id, msg_part) in &aux_expectations {
        let req = UnifiedSearchRequest {
            request_id: format!(
                "req-aux-terminal-{}",
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ),
            resource_id: (*res_id).to_owned(),
            aoi: valid_geojson_polygon(),
            aoi_name: "AuxAOI".to_owned(),
            time_range: None,
            filters: Map::new(),
        };
        let progress = service
            .search_start(req)
            .expect("Auxiliary must return Ok(SearchProgress)");
        assert!(progress.terminal);
        assert_eq!(progress.stage, SearchProgressStage::Failed);
        let err = progress.error.expect("Typed error required");
        assert_eq!(err.code, "SEARCH_SOURCE_AUXILIARY_UNSUPPORTED");
        assert!(err.message_zh.contains(msg_part));
        assert!(!err.retryable);
    }

    assert_eq!(*catalog.query_count.lock().unwrap(), 0);
    service.shutdown().unwrap();
}

// =========================================================================
// 5. Duplicate request_id Prevention and Idempotent State Machine
// =========================================================================

#[test]
fn test_duplicate_request_id_rejection() {
    let (_db, service, _catalog) = create_test_service();

    let req = UnifiedSearchRequest {
        request_id: "duplicate-test-req-001".to_owned(),
        resource_id: "dem.aster-gdem.v3".to_owned(),
        aoi: valid_geojson_polygon(),
        aoi_name: "TestAOI".to_owned(),
        time_range: None,
        filters: Map::new(),
    };

    // First call succeeds
    let first = service.search_start(req.clone());
    assert!(first.is_ok(), "First search_start must succeed");

    // Second call with identical request_id must fail
    let second = service.search_start(req);
    assert!(
        second.is_err(),
        "Second search_start with same request_id must fail"
    );
    let err_str = format!("{:?}", second.err().unwrap());
    assert!(
        err_str.contains("request_id already exists"),
        "Error must specify duplicate request_id, got: {}",
        err_str
    );

    service.shutdown().unwrap();
}

// =========================================================================
// 6. Unknown Resource ID and Injection Attack Rejection
// =========================================================================

#[test]
fn test_unknown_and_injected_resource_ids_fail_closed() {
    let (_db, service, catalog) = create_test_service();

    let malicious_resource_ids = [
        "science.unknown.v1",
        "dem.aster-gdem.v4", // Non-existent version
        "'; DROP TABLE task_runtime;--",
        "../../contracts/v3/resource-registry.v1.json",
        "<script>alert('xss')</script>",
        "science.opera.disp-s1.v1\x00injected",
    ];

    for bad_res_id in &malicious_resource_ids {
        let req = UnifiedSearchRequest {
            request_id: format!("req-malicious-{}", COUNTER.fetch_add(1, Ordering::Relaxed)),
            resource_id: (*bad_res_id).to_owned(),
            aoi: valid_geojson_polygon(),
            aoi_name: "AttackAOI".to_owned(),
            time_range: None,
            filters: Map::new(),
        };

        let result = service.search_start(req);
        assert!(
            result.is_err(),
            "Expected error for malicious resource_id '{}', but got Ok",
            bad_res_id
        );
    }

    assert_eq!(*catalog.query_count.lock().unwrap(), 0);
    service.shutdown().unwrap();
}
