//! Empirical Challenger Stress Harness for Milestone 2: 7 Missing Data Sources Application Service
//! Adversarially stress-tests auxiliary sources, SAR sources, coordinate validation, date ranges,
//! unreviewed filter rejection, and error handling with zero panics.

use insar_application_service::{
    ApplicationService, ApplicationServiceConfig, UnifiedSearchRequest,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogSearchQuery, CatalogSearchResponse,
    DownloadItemProgress, DownloadItemResult, ProviderResult, SearchProgressStage, TimeRange,
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

static NEXT: AtomicU64 = AtomicU64::new(1000);

struct TempStore(PathBuf);
impl TempStore {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "challenger-m2-service-stress-{}-{}.sqlite3",
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

fn asf_fixture() -> AsfRemoteSearchResponse {
    serde_json::from_value(json!({
        "scenes": [],
        "errors": [],
        "stats": {
            "requested_limit": 250,
            "returned_count": 0,
            "elapsed_ms": 1
        }
    }))
    .unwrap()
}

fn catalog_fixture() -> CatalogSearchResponse {
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

struct MockAsf;
impl LiveAsfSearch for MockAsf {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Ok(asf_fixture())
    }
}

struct MockCatalog {
    pub queries: Mutex<Vec<CatalogSearchQuery>>,
}
impl LiveCatalogSearch for MockCatalog {
    fn search_catalog(
        &self,
        q: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.queries.lock().unwrap().push(q.clone());
        Ok(catalog_fixture())
    }
}

struct NoopDownload;
impl DownloadWorker for NoopDownload {
    fn execute(
        &self,
        _: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        panic!("Search must not trigger downloads")
    }
}

fn valid_polygon() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [
            [[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 31.0], [110.0, 30.0]]
        ]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn make_service() -> (TempStore, Arc<ApplicationService>, Arc<MockCatalog>) {
    let db = TempStore::new();
    let catalog = Arc::new(MockCatalog {
        queries: Mutex::new(Vec::new()),
    });
    let service = Arc::new(ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::new(MockAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::new(NoopDownload),
    ));
    (db, service, catalog)
}

#[test]
fn test_all_5_auxiliary_sources_return_clean_terminal_progress_with_chinese_guidance() {
    let (_db, service, _catalog) = make_service();

    let auxiliary_cases = &[
        (
            "dem.aster-gdem.v3",
            "DEM 无需在这里检索，请按当前范围直接创建 DEM 下载任务",
        ),
        (
            "event.usgs.earthquake.v1",
            "USGS 地震事件目录无需在影像工作台检索，请在地震事件面板中触发同震配对计算",
        ),
        (
            "geodesy.ngl.gnss.v1",
            "NGL 全球 GNSS 基准站位移序列无需在影像工作台检索，请在参考点面板中自动检索与匹配",
        ),
        (
            "atmospheric.era5.reanalysis.v1",
            "ERA5 大气延迟场无需在影像工作台检索，请在气象校正面板中按影像过境时刻与范围提取",
        ),
        (
            "ionospheric.igs.gim.v1",
            "IGS GIM 电离层格网无需在影像工作台检索，请在电离层校正面板中按影像日期与年积日提取",
        ),
    ];

    for (idx, (resource_id, expected_msg)) in auxiliary_cases.iter().enumerate() {
        let req = UnifiedSearchRequest {
            request_id: format!("req-aux-{idx}"),
            resource_id: (*resource_id).to_owned(),
            aoi: valid_polygon(),
            aoi_name: "测试研究区".to_owned(),
            time_range: Some(TimeRange {
                start: Some("2024-01-01T00:00:00Z".to_owned()),
                end: Some("2024-01-10T23:59:59Z".to_owned()),
            }),
            filters: Map::new(),
        };

        let progress = service.search_start(req.clone()).unwrap_or_else(|e| {
            panic!(
                "Expected Ok(SearchProgress) for {}, got Err({:?})",
                resource_id, e
            )
        });

        assert!(
            progress.terminal,
            "Auxiliary search must return terminal progress for {}",
            resource_id
        );
        assert_eq!(progress.stage, SearchProgressStage::Failed);
        assert_eq!(progress.resource_id, *resource_id);

        let err = progress.error.clone().expect("Typed error must be present");
        assert_eq!(err.code, "SEARCH_SOURCE_AUXILIARY_UNSUPPORTED");
        assert_eq!(err.message_zh, *expected_msg);
        assert!(!err.retryable);

        // Verification of query retrieval from service state
        let retrieved = service.search_progress(&req.request_id);
        assert_eq!(retrieved, Some(progress));
    }

    service.shutdown().unwrap();
}

#[test]
fn test_auxiliary_sources_adversarial_input_rejection_no_panic() {
    let (_db, service, _catalog) = make_service();

    let auxiliary_ids = &[
        "dem.aster-gdem.v3",
        "event.usgs.earthquake.v1",
        "geodesy.ngl.gnss.v1",
        "atmospheric.era5.reanalysis.v1",
        "ionospheric.igs.gim.v1",
    ];

    for resource_id in auxiliary_ids {
        // 1. Invalid request_id (empty, whitespace, excessive length)
        for bad_req_id in ["", "   ", "\t\n", &"x".repeat(500)] {
            let req = UnifiedSearchRequest {
                request_id: bad_req_id.to_string(),
                resource_id: (*resource_id).to_owned(),
                aoi: valid_polygon(),
                aoi_name: "valid_name".to_owned(),
                time_range: None,
                filters: Map::new(),
            };
            let res = service.search_start(req);
            assert!(
                res.is_err(),
                "Expected error for bad request_id '{}' on {}",
                bad_req_id,
                resource_id
            );
        }

        // 2. Blank AOI name
        for blank_name in ["", "   ", "\t\t"] {
            let req = UnifiedSearchRequest {
                request_id: format!("req-bad-name-{}", NEXT.fetch_add(1, Ordering::Relaxed)),
                resource_id: (*resource_id).to_owned(),
                aoi: valid_polygon(),
                aoi_name: blank_name.to_string(),
                time_range: None,
                filters: Map::new(),
            };
            let res = service.search_start(req);
            assert!(
                res.is_err(),
                "Blank aoi_name must be rejected for {}",
                resource_id
            );
        }

        // 3. Corrupted / Invalid AOI GeoJSON
        let corrupt_aois: &[Value] = &[
            json!({}),
            json!({"type": "Point", "coordinates": [100.0, 30.0]}),
            json!({"type": "Polygon", "coordinates": []}),
            json!({"type": "Polygon", "coordinates": [[[100.0, 30.0], [101.0, 30.0]]]}), // < 4 coords
            json!({"type": "Polygon", "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [102.0, 32.0]]]}), // unclosed
            json!({"type": "Polygon", "coordinates": [[[200.0, 30.0], [201.0, 30.0], [201.0, 31.0], [200.0, 30.0]]]}), // lon > 180
            json!({"type": "Polygon", "coordinates": [[[100.0, 95.0], [101.0, 95.0], [101.0, 96.0], [100.0, 95.0]]]}), // lat > 90
            json!({"type": "Polygon", "coordinates": [[[100.0, -95.0], [101.0, -95.0], [101.0, -96.0], [100.0, -95.0]]]}), // lat < -90
        ];

        for bad_aoi in corrupt_aois {
            let req = UnifiedSearchRequest {
                request_id: format!("req-bad-aoi-{}", NEXT.fetch_add(1, Ordering::Relaxed)),
                resource_id: (*resource_id).to_owned(),
                aoi: bad_aoi.as_object().unwrap_or(&Map::new()).clone(),
                aoi_name: "AOI".to_owned(),
                time_range: None,
                filters: Map::new(),
            };
            // Auxiliary sources intercept before polygon validation to provide terminal guidance,
            // or fail cleanly without panic.
            let res = service.search_start(req);
            match res {
                Ok(progress) => {
                    assert!(progress.terminal, "Auxiliary search must be terminal");
                    assert_eq!(
                        progress.error.as_ref().map(|e| e.code.as_str()),
                        Some("SEARCH_SOURCE_AUXILIARY_UNSUPPORTED")
                    );
                }
                Err(e) => {
                    assert!(
                        format!("{:?}", e).contains("InvalidRequest"),
                        "Expected InvalidRequest, got {:?}",
                        e
                    );
                }
            }
        }

        // 4. Invalid time ranges (start > end, malformed dates)
        let bad_time_ranges = &[
            TimeRange {
                start: Some("2024-02-01T00:00:00Z".to_owned()),
                end: Some("2024-01-01T00:00:00Z".to_owned()), // inverted
            },
            TimeRange {
                start: Some("not-a-date".to_owned()),
                end: Some("2024-01-01T00:00:00Z".to_owned()),
            },
            TimeRange {
                start: Some("2024-01-01T00:00:00Z".to_owned()),
                end: Some("2024-99-99T00:00:00Z".to_owned()),
            },
        ];

        for bad_tr in bad_time_ranges {
            let req = UnifiedSearchRequest {
                request_id: format!("req-bad-tr-{}", NEXT.fetch_add(1, Ordering::Relaxed)),
                resource_id: (*resource_id).to_owned(),
                aoi: valid_polygon(),
                aoi_name: "AOI".to_owned(),
                time_range: Some(bad_tr.clone()),
                filters: Map::new(),
            };
            // Auxiliary sources do not panic on invalid dates; they return clean terminal unsupported progress
            let res = service.search_start(req);
            match res {
                Ok(progress) => {
                    assert!(progress.terminal, "Auxiliary search must be terminal");
                    assert_eq!(
                        progress.error.as_ref().map(|e| e.code.as_str()),
                        Some("SEARCH_SOURCE_AUXILIARY_UNSUPPORTED")
                    );
                }
                Err(e) => {
                    assert!(
                        format!("{:?}", e).contains("InvalidRequest"),
                        "Expected InvalidRequest, got {:?}",
                        e
                    );
                }
            }
        }
    }

    service.shutdown().unwrap();
}

#[test]
fn test_sar_opera_and_licsar_service_behavior_and_filters() {
    let (_db, service, _catalog) = make_service();

    // 1. Valid OPERA S1 search
    let mut opera_filters = Map::new();
    opera_filters.insert("flight_direction".to_owned(), json!("ASCENDING"));

    let opera_req = UnifiedSearchRequest {
        request_id: "req-opera-valid-1".to_owned(),
        resource_id: "science.opera.disp-s1.v1".to_owned(),
        aoi: valid_polygon(),
        aoi_name: "OPERA_Area".to_owned(),
        time_range: Some(TimeRange {
            start: Some("2024-01-01T00:00:00Z".to_owned()),
            end: Some("2024-01-31T23:59:59Z".to_owned()),
        }),
        filters: opera_filters,
    };

    let started = service
        .search_start(opera_req)
        .expect("Valid OPERA search should start");
    assert_eq!(started.stage, SearchProgressStage::Planning);
    assert_eq!(started.resource_id, "science.opera.disp-s1.v1");

    // 2. Unreviewed filter on OPERA S1 must be strictly rejected
    let mut bad_opera_filters = Map::new();
    bad_opera_filters.insert("unreviewed_filter_key".to_owned(), json!("injected"));

    let bad_opera_req = UnifiedSearchRequest {
        request_id: "req-opera-bad-1".to_owned(),
        resource_id: "science.opera.disp-s1.v1".to_owned(),
        aoi: valid_polygon(),
        aoi_name: "OPERA_Area".to_owned(),
        time_range: None,
        filters: bad_opera_filters,
    };

    let err = service.search_start(bad_opera_req).unwrap_err();
    assert!(
        format!("{:?}", err).contains("unreviewed SAR catalogue filter"),
        "Expected unreviewed filter error, got {:?}",
        err
    );

    // 3. Conflicting fixed filter on OPERA S1 must be rejected
    let mut conflict_opera_filters = Map::new();
    conflict_opera_filters.insert("platform".to_owned(), json!("RADARSAT-2")); // Fixed is SENTINEL-1

    let conflict_opera_req = UnifiedSearchRequest {
        request_id: "req-opera-conflict-1".to_owned(),
        resource_id: "science.opera.disp-s1.v1".to_owned(),
        aoi: valid_polygon(),
        aoi_name: "OPERA_Area".to_owned(),
        time_range: None,
        filters: conflict_opera_filters,
    };

    let err = service.search_start(conflict_opera_req).unwrap_err();
    assert!(
        format!("{:?}", err).contains("SAR fixed filter conflicts with source"),
        "Expected fixed filter conflict error, got {:?}",
        err
    );

    // 4. Valid LiCSAR search
    let mut licsar_filters = Map::new();
    licsar_filters.insert("frame".to_owned(), json!(1024));

    let licsar_req = UnifiedSearchRequest {
        request_id: "req-licsar-valid-1".to_owned(),
        resource_id: "science.comet.licsar.v1".to_owned(),
        aoi: valid_polygon(),
        aoi_name: "LiCSAR_Area".to_owned(),
        time_range: Some(TimeRange {
            start: Some("2024-02-01T00:00:00Z".to_owned()),
            end: Some("2024-02-15T23:59:59Z".to_owned()),
        }),
        filters: licsar_filters,
    };

    let started_licsar = service
        .search_start(licsar_req)
        .expect("Valid LiCSAR search should start");
    assert_eq!(started_licsar.stage, SearchProgressStage::Planning);
    assert_eq!(started_licsar.resource_id, "science.comet.licsar.v1");

    // 5. Unreviewed filter on LiCSAR must be strictly rejected
    let mut bad_licsar_filters = Map::new();
    bad_licsar_filters.insert("polarization".to_owned(), json!("HH")); // Not in LiCSAR supported_keys

    let bad_licsar_req = UnifiedSearchRequest {
        request_id: "req-licsar-bad-1".to_owned(),
        resource_id: "science.comet.licsar.v1".to_owned(),
        aoi: valid_polygon(),
        aoi_name: "LiCSAR_Area".to_owned(),
        time_range: None,
        filters: bad_licsar_filters,
    };

    let err = service.search_start(bad_licsar_req).unwrap_err();
    assert!(
        format!("{:?}", err).contains("unreviewed SAR catalogue filter"),
        "Expected unreviewed filter error for LiCSAR, got {:?}",
        err
    );

    // 6. Corrupt AOI on SAR resources MUST be rejected by recognized_request
    for sar_id in ["science.opera.disp-s1.v1", "science.comet.licsar.v1"] {
        let corrupt_sar_req = UnifiedSearchRequest {
            request_id: format!("req-corrupt-aoi-{}", NEXT.fetch_add(1, Ordering::Relaxed)),
            resource_id: (*sar_id).to_owned(),
            aoi: json!({"type": "Point", "coordinates": [100.0, 30.0]})
                .as_object()
                .unwrap()
                .clone(),
            aoi_name: "CorruptAOI".to_owned(),
            time_range: None,
            filters: Map::new(),
        };
        let err = service.search_start(corrupt_sar_req).unwrap_err();
        assert!(
            format!("{:?}", err).contains("aoi must be an authoritative Polygon or MultiPolygon"),
            "Expected polygonal GeoJSON error for {}, got {:?}",
            sar_id,
            err
        );
    }

    service.shutdown().unwrap();
}
