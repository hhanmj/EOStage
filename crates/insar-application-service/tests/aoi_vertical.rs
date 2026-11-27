use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationClearRegionAoiRequest,
    ApplicationSarSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    ApplicationSetWorkingAoiRequest, WorkspaceSelectNodeRequest,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, ContractError, ProviderResult,
    SarRemoteSearchRequest, SarRemoteSearchResponse,
};
use insar_provider::asf::query::{plan_sar_query, AoiPlanMode};
use insar_service::LiveAsfSearch;
use serde_json::{json, Map, Value};

mod common;

static NEXT_ID: AtomicU64 = AtomicU64::new(100);

struct TempStore {
    root: PathBuf,
    database: PathBuf,
}

impl TempStore {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-aoi-vertical-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("tasks.sqlite3");
        Self { root, database }
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct DummySearch;

impl LiveAsfSearch for DummySearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("DUMMY", "dummy search", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        _request: &SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        Ok(serde_json::from_value(json!({
            "products": [],
            "errors": [],
            "stats": {
                "requested_limit": 10,
                "query_limit": 10,
                "candidate_count": 0,
                "returned_count": 0,
                "source": "dummy"
            }
        }))
        .unwrap())
    }
}

struct RecordingSearch {
    request: Arc<Mutex<Option<SarRemoteSearchRequest>>>,
}

impl LiveAsfSearch for RecordingSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("DUMMY", "dummy search", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        request: &SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        *self.request.lock().unwrap() = Some(request.clone());
        Ok(serde_json::from_value(json!({
            "products": [],
            "errors": [],
            "stats": {
                "requested_limit": request.query.max_results,
                "query_limit": request.query.max_results,
                "candidate_count": 0,
                "returned_count": 0,
                "source": "ASF"
            }
        }))
        .unwrap())
    }
}

fn sample_polygon_geometry() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn spatial_authority_polygon_geometry() -> Map<String, Value> {
    // Reduced from the live DataV 420000 boundary used by desktop acceptance.
    // The concave ring is intentionally not equivalent to its surrounding DEM
    // envelope, so this cross-feature regression catches bbox substitution.
    json!({
        "type": "Polygon",
        "coordinates": [[
            [108.36246974074932, 29.822904140486955],
            [110.4, 29.032624051704644],
            [114.71388869786357, 29.3999513022449],
            [116.12996340596769, 31.1],
            [111.99619022065494, 33.27754193617368],
            [110.8, 31.2],
            [108.36246974074932, 29.822904140486955]
        ]]
    })
    .as_object()
    .unwrap()
    .clone()
}

#[test]
fn test_aoi_bbox_binding_and_restart_restore() {
    let temp = TempStore::new("bbox");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    let region_id = common::ensure_fixture_region(&service, "bbox");

    let initial_snapshot = service.snapshot(false).unwrap();
    assert!(initial_snapshot.restore.region_aoi.is_none());

    let bind_req = ApplicationBindRegionAoiRequest {
        request_id: "bind-bbox-1".to_owned(),
        region_id: region_id.clone(),
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: sample_polygon_geometry(),
        bbox: vec![100.0, 30.0, 101.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "矩形选区 (100.00, 30.00)".to_owned(),
        bound_at: "2026-08-05T00:00:00Z".to_owned(),
    };

    let snapshot = service.bind_region_aoi(&bind_req).unwrap();
    let aoi = snapshot.core.region_aoi.as_ref().unwrap();
    assert_eq!(aoi.region_id, region_id);
    assert_eq!(aoi.revision, 0);
    assert_eq!(aoi.source_kind, "manual_bbox");
    assert_eq!(aoi.bbox, vec![100.0, 30.0, 101.0, 31.0]);
    assert_eq!(aoi.display_name, "矩形选区 (100.00, 30.00)");

    service.shutdown().unwrap();

    let reopened =
        ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    let restored_snapshot = reopened.snapshot(false).unwrap();
    let restored_aoi = restored_snapshot.restore.region_aoi.unwrap();
    assert_eq!(restored_aoi.region_id, aoi.region_id);
    assert_eq!(restored_aoi.revision, 0);
    assert_eq!(restored_aoi.bbox, vec![100.0, 30.0, 101.0, 31.0]);
    reopened.shutdown().unwrap();
}

#[test]
fn test_aoi_polygon_binding_and_restart_restore() {
    let temp = TempStore::new("polygon");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    let region_id = common::ensure_fixture_region(&service, "polygon");

    let poly_geom = json!({
        "type": "Polygon",
        "coordinates": [[[105.0, 35.0], [106.0, 35.0], [106.5, 36.0], [105.0, 36.0], [105.0, 35.0]]]
    })
    .as_object()
    .unwrap()
    .clone();

    let bind_req = ApplicationBindRegionAoiRequest {
        request_id: "bind-poly-1".to_owned(),
        region_id,
        expected_revision: None,
        source_kind: "manual_polygon".to_owned(),
        geometry: poly_geom.clone(),
        bbox: vec![105.0, 35.0, 106.5, 36.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "多边形选区".to_owned(),
        bound_at: "2026-08-05T00:01:00Z".to_owned(),
    };

    let snapshot = service.bind_region_aoi(&bind_req).unwrap();
    let aoi = snapshot.core.region_aoi.as_ref().unwrap();
    assert_eq!(aoi.source_kind, "manual_polygon");
    assert_eq!(aoi.geometry, poly_geom);
    assert_eq!(aoi.bbox, vec![105.0, 35.0, 106.5, 36.0]);

    service.shutdown().unwrap();

    let reopened =
        ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    let restored = reopened
        .snapshot(false)
        .unwrap()
        .restore
        .region_aoi
        .unwrap();
    assert_eq!(restored.geometry, poly_geom);
    assert_eq!(restored.bbox, vec![105.0, 35.0, 106.5, 36.0]);
    reopened.shutdown().unwrap();
}

#[test]
fn test_aoi_admin_boundary_binding_and_explicit_search_aoi() {
    let temp = TempStore::new("admin");
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(DummySearch),
        Arc::new(
            insar_download::ProductionDownloadWorker::new(
                insar_download::TransferOptions::default(),
            )
            .unwrap(),
        ),
    );
    let region_id = common::ensure_fixture_region(&service, "admin");

    // An explicit AOI can freeze a search before it is bound to a named Region.
    let search_req = ApplicationSarSearchAndFreezeRequest {
        request_id: "search-no-aoi".to_owned(),
        search: serde_json::from_value(json!({
            "query": {
                "provider": "asf",
                "dataset": "SENTINEL-1",
                "aoi_geojson": sample_polygon_geometry(),
                "max_results": 10
            }
        }))
        .unwrap(),
        batch_id: "batch-no-aoi".to_owned(),
        created_at: "2026-08-05T00:00:00Z".to_owned(),
        aoi_name: "No AOI".to_owned(),
        aoi_geojson: Some(sample_polygon_geometry()),
        page_size: 24,
    };
    let standalone_search = service
        .sar_search_and_freeze_legacy(&search_req)
        .expect("explicit AOI search does not require a saved Region");
    assert!(standalone_search.restore.workbench_batch.is_some());

    // Bind admin boundary AOI
    let bind_req = ApplicationBindRegionAoiRequest {
        request_id: "bind-admin-1".to_owned(),
        region_id,
        expected_revision: None,
        source_kind: "administrative_boundary".to_owned(),
        geometry: sample_polygon_geometry(),
        bbox: vec![100.0, 30.0, 101.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "四川省 / 成都市".to_owned(),
        bound_at: "2026-08-05T00:02:00Z".to_owned(),
    };
    let snapshot = service.bind_region_aoi(&bind_req).unwrap();
    assert_eq!(
        snapshot.core.region_aoi.unwrap().display_name,
        "四川省 / 成都市"
    );

    // Now search with bound AOI succeeds
    let valid_search = service.sar_search_and_freeze_legacy(&search_req).unwrap();
    assert!(valid_search.restore.workbench_batch.is_some());

    service.shutdown().unwrap();
}

fn translated_polygon_geometry() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[110.0, 35.0], [111.0, 35.0], [111.0, 36.0], [110.0, 36.0], [110.0, 35.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn search_request(suffix: &str) -> ApplicationSarSearchAndFreezeRequest {
    serde_json::from_value(json!({
        "request_id": format!("aoi-transition-{suffix}"),
        "search": {"query": {
            "provider": "ASF",
            "dataset": "SENTINEL-1",
            "max_results": 20
        }},
        "batch_id": format!("aoi-transition-batch-{suffix}"),
        "created_at": "2026-08-18T00:00:00Z",
        "aoi_name": "No AOI",
        "aoi_geojson": null,
        "page_size": 24
    }))
    .unwrap()
}

#[test]
fn working_administrative_aoi_reaches_asf_intersection_plan_and_frozen_batch() {
    let temp = TempStore::new("working-admin-search");
    let recorded = Arc::new(Mutex::new(None));
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(RecordingSearch {
            request: Arc::clone(&recorded),
        }),
        Arc::new(
            insar_download::ProductionDownloadWorker::new(
                insar_download::TransferOptions::default(),
            )
            .unwrap(),
        ),
    );
    let geometry = spatial_authority_polygon_geometry();
    service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "administrative_boundary".to_owned(),
            geometry: geometry.clone(),
            bbox: vec![
                108.36246974074932,
                29.032624051704644,
                116.12996340596769,
                33.27754193617368,
            ],
            crs: "EPSG:4326".to_owned(),
            display_name: "Hubei".to_owned(),
        })
        .expect("direct administrative polygon becomes the working AOI");

    let request: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
        "request_id": "working-admin-search",
        "search": {"query": {
            "provider": "ASF",
            "dataset": "SENTINEL-1",
            "processing_levels": ["SLC"],
            "beam_modes": ["IW"],
            "polarizations": ["VV+VH"],
            "flight_direction": "DESCENDING",
            "aoi_geojson": geometry,
            "bbox": {
                "west": 107.8,
                "east": 116.7,
                "south": 28.5,
                "north": 33.8,
                "crs": "EPSG:4326"
            },
            "max_results": 20
        }},
        "batch_id": "working-admin-batch",
        "created_at": "2026-08-18T00:00:00Z",
        "aoi_name": "Hubei",
        "aoi_geojson": geometry,
        "page_size": 24
    }))
    .unwrap();

    let snapshot = service
        .sar_search_and_freeze_legacy(&request)
        .expect("working AOI is injected before the Provider request");
    let provider_request = recorded
        .lock()
        .unwrap()
        .clone()
        .expect("Provider receives exactly one request");
    assert_eq!(provider_request.query.aoi_geojson, Some(geometry.clone()));
    assert_eq!(provider_request.query.max_results, Some(20));
    let plan = plan_sar_query(&provider_request.query).expect("ASF query plans");
    assert_eq!(plan.aoi_mode, AoiPlanMode::Exact);
    let intersects_with = plan
        .params
        .get("intersectsWith")
        .expect("actual AOI becomes intersectsWith");
    assert!(intersects_with.starts_with("POLYGON"));
    assert!(intersects_with.contains("110.8 31.2"));
    assert!(!intersects_with.contains("107.8 28.5"));
    assert_eq!(plan.params.get("beamMode").map(String::as_str), Some("IW"));
    assert_eq!(
        plan.params.get("polarization").map(String::as_str),
        Some("VV+VH")
    );
    assert_eq!(
        plan.params.get("flightDirection").map(String::as_str),
        Some("DESCENDING")
    );

    let batch = snapshot
        .restore
        .workbench_batch
        .expect("search freezes a workbench batch");
    assert_eq!(batch.aoi_name, "Hubei");
    assert_eq!(batch.aoi_geojson, Some(geometry));
    assert_eq!(batch.query.get("max_results"), Some(&json!(20)));
    service.shutdown().unwrap();

    let reopened_recorded = Arc::new(Mutex::new(None));
    let reopened = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(RecordingSearch {
            request: Arc::clone(&reopened_recorded),
        }),
        Arc::new(
            insar_download::ProductionDownloadWorker::new(
                insar_download::TransferOptions::default(),
            )
            .unwrap(),
        ),
    );
    let restored = reopened
        .working_aoi()
        .expect("persisted standalone AOI can be loaded after restart")
        .expect("standalone AOI remains available after restart");
    assert_eq!(restored.geometry, spatial_authority_polygon_geometry());
    assert_eq!(restored.display_name, "Hubei");

    let mut reopened_request = request.clone();
    reopened_request.request_id = "working-admin-search-restart".to_owned();
    reopened_request.batch_id = "working-admin-batch-restart".to_owned();
    let reopened_snapshot = reopened
        .sar_search_and_freeze_legacy(&reopened_request)
        .expect("persisted standalone AOI is projected through cached search after restart");
    let reopened_batch = reopened_snapshot
        .restore
        .workbench_batch
        .expect("cached search restores the frozen batch");
    assert_eq!(reopened_batch.aoi_name, "Hubei");
    assert_eq!(
        reopened_batch.aoi_geojson,
        Some(spatial_authority_polygon_geometry())
    );
    assert_eq!(
        reopened_batch.query.get("aoi_geojson"),
        Some(&Value::Object(spatial_authority_polygon_geometry()))
    );
    assert!(
        reopened_recorded.lock().unwrap().is_none(),
        "complete persistent SAR cache should restore AOI without repeating provider work"
    );
    reopened.shutdown().unwrap();
}

#[test]
fn burst_search_uses_authoritative_geojson_without_the_ui_bbox_projection() {
    let temp = TempStore::new("burst-dual-spatial-projection");
    let recorded = Arc::new(Mutex::new(None));
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(RecordingSearch {
            request: Arc::clone(&recorded),
        }),
        Arc::new(
            insar_download::ProductionDownloadWorker::new(
                insar_download::TransferOptions::default(),
            )
            .unwrap(),
        ),
    );
    let geometry = spatial_authority_polygon_geometry();
    let request: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
        "request_id": "burst-dual-spatial-projection",
        "search": {"query": {
            "provider": "ASF",
            "dataset": "SLC-BURST",
            "processing_levels": ["BURST"],
            "start": "2026-05-26",
            "end": "2026-08-26",
            "aoi_geojson": geometry,
            "bbox": {
                "west": 108.36246974074932,
                "south": 29.032624051704644,
                "east": 116.12996340596769,
                "north": 33.27754193617368,
                "crs": "EPSG:4326"
            },
            "max_results": 3
        }},
        "batch_id": "burst-dual-spatial-batch",
        "created_at": "2026-08-26T00:00:00Z",
        "aoi_name": "行政区边界",
        "aoi_geojson": geometry,
        "page_size": 24
    }))
    .unwrap();

    let snapshot = service
        .sar_search_and_freeze_legacy(&request)
        .expect("Burst search normalizes the redundant UI bbox before Provider dispatch");
    let provider_request = recorded
        .lock()
        .unwrap()
        .clone()
        .expect("Provider receives the normalized Burst request");
    assert_eq!(provider_request.query.aoi_geojson, Some(geometry.clone()));
    assert_eq!(provider_request.query.bbox, None);
    let batch = snapshot
        .restore
        .workbench_batch
        .expect("normalized Burst search freezes a SearchBatch");
    assert_eq!(batch.aoi_geojson, Some(geometry));
    assert_eq!(batch.query.get("bbox"), Some(&Value::Null));
    service.shutdown().unwrap();
}

#[test]
fn selected_region_owns_search_aoi_and_standalone_reactivates_only_after_leaving_region() {
    let temp = TempStore::new("region-standalone-transition");
    let recorded = Arc::new(Mutex::new(None));
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(RecordingSearch {
            request: Arc::clone(&recorded),
        }),
        Arc::new(
            insar_download::ProductionDownloadWorker::new(
                insar_download::TransferOptions::default(),
            )
            .unwrap(),
        ),
    );
    let standalone_geometry = sample_polygon_geometry();
    service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "manual_polygon".to_owned(),
            geometry: standalone_geometry.clone(),
            bbox: vec![100.0, 30.0, 101.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "standalone".to_owned(),
        })
        .unwrap();
    let region_id = common::ensure_fixture_region(&service, "region-standalone-transition");
    let region_geometry = translated_polygon_geometry();
    service
        .bind_region_aoi(&ApplicationBindRegionAoiRequest {
            request_id: "bind-region-owner".to_owned(),
            region_id: region_id.clone(),
            expected_revision: None,
            source_kind: "administrative_boundary".to_owned(),
            geometry: region_geometry.clone(),
            bbox: vec![110.0, 35.0, 111.0, 36.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "saved region".to_owned(),
            bound_at: "2026-08-18T00:00:00Z".to_owned(),
        })
        .unwrap();

    service
        .sar_search_and_freeze_legacy(&search_request("saved-region"))
        .unwrap();
    assert_eq!(
        recorded
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|request| request.query.aoi_geojson.clone()),
        Some(region_geometry)
    );

    service
        .clear_region_aoi(&ApplicationClearRegionAoiRequest {
            request_id: "clear-region-owner".to_owned(),
            region_id: region_id.clone(),
            expected_revision: Some(0),
        })
        .unwrap();
    *recorded.lock().unwrap() = None;
    let no_region_aoi = service
        .sar_search_and_freeze_legacy(&search_request("selected-region-no-aoi"))
        .unwrap();
    assert_eq!(
        recorded.lock().unwrap().as_ref().unwrap().query.aoi_geojson,
        None,
        "a selected Region with no AOI must not fall through to standalone"
    );
    assert_eq!(
        no_region_aoi.restore.workbench_batch.unwrap().aoi_geojson,
        None
    );

    let context = service
        .core_restore(false)
        .unwrap()
        .core
        .workspace_context
        .unwrap();
    let project_id = context.project_id.clone().unwrap();
    service
        .project_select(&WorkspaceSelectNodeRequest {
            request_id: "leave-region-owner".to_owned(),
            parent_id: context.workspace_id,
            expected_revision: context.revision,
            node_id: project_id,
        })
        .unwrap();
    *recorded.lock().unwrap() = None;
    service
        .sar_search_and_freeze_legacy(&search_request("standalone-reactivated"))
        .unwrap();
    assert_eq!(
        recorded
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|request| request.query.aoi_geojson.clone()),
        Some(standalone_geometry)
    );
    service.shutdown().unwrap();
}

#[test]
fn test_aoi_failure_boundaries() {
    let temp = TempStore::new("failures");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    let region_id = common::ensure_fixture_region(&service, "failures");

    // 1. Invalid bbox (too few items)
    let invalid_bbox_req = ApplicationBindRegionAoiRequest {
        request_id: "bind-bad-bbox".to_owned(),
        region_id: region_id.clone(),
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: sample_polygon_geometry(),
        bbox: vec![100.0, 30.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "Bad Bbox".to_owned(),
        bound_at: "2026-08-05T00:00:00Z".to_owned(),
    };
    assert!(service.bind_region_aoi(&invalid_bbox_req).is_err());

    let invalid_topology_req = ApplicationBindRegionAoiRequest {
        request_id: "bind-bow-tie".to_owned(),
        region_id: region_id.clone(),
        expected_revision: None,
        source_kind: "manual_polygon".to_owned(),
        geometry: json!({
            "type": "Polygon",
            "coordinates": [[[100.0,30.0],[101.0,31.0],[100.0,31.0],[101.0,30.0],[100.0,30.0]]]
        })
        .as_object()
        .unwrap()
        .clone(),
        bbox: vec![100.0, 30.0, 101.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "Invalid Topology".to_owned(),
        bound_at: "2026-08-05T00:00:00Z".to_owned(),
    };
    assert!(service.bind_region_aoi(&invalid_topology_req).is_err());
    assert!(service
        .core_restore(false)
        .unwrap()
        .core
        .region_aoi
        .is_none());

    // 2. Initial binding
    let bind_req = ApplicationBindRegionAoiRequest {
        request_id: "bind-ok".to_owned(),
        region_id: region_id.clone(),
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: sample_polygon_geometry(),
        bbox: vec![100.0, 30.0, 101.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "Valid Bbox".to_owned(),
        bound_at: "2026-08-05T00:00:00Z".to_owned(),
    };
    let snap1 = service.bind_region_aoi(&bind_req).unwrap();
    assert_eq!(snap1.core.region_aoi.as_ref().unwrap().revision, 0);

    // 3. Stale revision conflict (expected revision 999 instead of 0)
    let stale_req = ApplicationBindRegionAoiRequest {
        request_id: "bind-stale".to_owned(),
        region_id: region_id.clone(),
        expected_revision: Some(999),
        source_kind: "manual_bbox".to_owned(),
        geometry: sample_polygon_geometry(),
        bbox: vec![100.0, 30.0, 101.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "Stale Bbox".to_owned(),
        bound_at: "2026-08-05T00:01:00Z".to_owned(),
    };
    assert!(service.bind_region_aoi(&stale_req).is_err());

    // 4. Clear AOI
    let clear_req = ApplicationClearRegionAoiRequest {
        request_id: "clear-1".to_owned(),
        region_id,
        expected_revision: Some(0),
    };
    let snap_cleared = service.clear_region_aoi(&clear_req).unwrap();
    assert!(snap_cleared.core.region_aoi.is_none());

    service.shutdown().unwrap();

    // Restart and verify clear state persisted
    let reopened =
        ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    assert!(reopened
        .snapshot(false)
        .unwrap()
        .restore
        .region_aoi
        .is_none());
    reopened.shutdown().unwrap();
}
