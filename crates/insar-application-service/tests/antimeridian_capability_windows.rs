use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use insar_application_service::{
    ApplicationSarSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    ApplicationSetWorkingAoiRequest, CapabilityRuntimeObservations, DemOperationPort,
    StandaloneDemAoiTaskRequest, StoreTaskPageRequest,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, BuildCapabilityState, ContractError,
    DownloadItemProgress, DownloadItemResult, DownloadOutcome, ProviderResult,
    RuntimeReadinessState, SarRemoteSearchRequest, SarRemoteSearchResponse,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::LiveAsfSearch;
use serde_json::{json, Map, Value};

static NEXT_STORE: AtomicU64 = AtomicU64::new(1);

struct TempStore {
    root: PathBuf,
    database: PathBuf,
}

impl TempStore {
    fn new(label: &str) -> Self {
        let sequence = NEXT_STORE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-antimeridian-capability-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Self {
            database: root.join("tasks.sqlite3"),
            root,
        }
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct EmptySearch;

impl LiveAsfSearch for EmptySearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("ANTIMERIDIAN_FIXTURE", "fixture only", Map::new()).unwrap())
    }
}

struct SentinelSearch;

impl LiveAsfSearch for SentinelSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("SENTINEL_FIXTURE", "fixture only", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        _request: &SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        Ok(serde_json::from_value(json!({
            "products": [
                {
                    "product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-FROZEN-A",
                    "provider_product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-FROZEN-A",
                    "provider_product_id": "S1-FROZEN-A",
                    "product_id": "S1-FROZEN-A",
                    "scene_id": "S1-FROZEN-A",
                    "provider": "asf.catalog",
                    "dataset": "SENTINEL-1",
                    "product_family": "SLC",
                    "processing_level": "SLC",
                    "platform": "S1A",
                    "acquisition_start": "2024-02-29T07:08:00Z",
                    "file_name": "S1-FROZEN-A.zip",
                    "download_url": "https://datapool.asf.alaska.edu/SLC/S1-FROZEN-A.zip"
                },
                {
                    "product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-FROZEN-B",
                    "provider_product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-FROZEN-B",
                    "provider_product_id": "S1-FROZEN-B",
                    "product_id": "S1-FROZEN-B",
                    "scene_id": "S1-FROZEN-B",
                    "provider": "asf.catalog",
                    "dataset": "SENTINEL-1",
                    "product_family": "SLC",
                    "processing_level": "SLC",
                    "platform": "S1A",
                    "acquisition_start": "2024-03-12T07:08:00Z",
                    "file_name": "S1-FROZEN-B.zip",
                    "download_url": "https://datapool.asf.alaska.edu/SLC/S1-FROZEN-B.zip"
                }
            ],
            "errors": [],
            "stats": {
                "requested_limit": 2,
                "query_limit": 2,
                "candidate_count": 2,
                "returned_count": 2,
                "source": "ASF",
                "matched_count": 2,
                "fetched_record_count": 2,
                "mapped_record_count": 2,
                "truncated": false
            }
        }))
        .unwrap())
    }
}

struct BaseWorker;

impl DownloadWorker for BaseWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Completed,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-08-19T00:00:01Z".to_owned(),
        }
    }
}

#[derive(Default)]
struct RecordingDemPort {
    calls: Mutex<Vec<(String, Value)>>,
    failure_message: Option<&'static str>,
    fail_window: Option<u64>,
    malformed_window: Option<u64>,
}

impl DemOperationPort for RecordingDemPort {
    fn call(&self, operation: &str, payload: Value) -> Result<Value, String> {
        self.calls
            .lock()
            .unwrap()
            .push((operation.to_owned(), payload.clone()));
        if operation == "dem_download" && self.fail_window == payload["window_index"].as_u64() {
            return Err(self
                .failure_message
                .unwrap_or("WINDOW_FETCH_FAILED")
                .to_owned());
        }
        if operation == "dem_download" && self.malformed_window == payload["window_index"].as_u64()
        {
            return Ok(json!({}));
        }
        Ok(json!({"ok": true, "bytes_written": 4096}))
    }
}

fn fixture(id: &str) -> Map<String, Value> {
    let root: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/geospatial/antimeridian_aoi_cases.json"
    ))
    .unwrap();
    root["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == id)
        .and_then(|case| case["geometry"].as_object())
        .unwrap()
        .clone()
}

fn service(temp: &TempStore) -> (ApplicationService, Arc<RecordingDemPort>) {
    service_with_port(temp, Arc::new(RecordingDemPort::default()))
}

fn service_with_port(
    temp: &TempStore,
    port: Arc<RecordingDemPort>,
) -> (ApplicationService, Arc<RecordingDemPort>) {
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database)
            .with_capability_runtime_observations(
                CapabilityRuntimeObservations::default()
                    .with_build(
                        "dem-provider.copernicus-aws.v1",
                        BuildCapabilityState::Available,
                    )
                    .with_runtime(
                        "dem-provider.copernicus-aws.v1",
                        RuntimeReadinessState::Ready,
                    ),
            ),
        Arc::new(EmptySearch),
        Arc::new(BaseWorker),
    )
    .attach_dem_operation_port(Arc::clone(&port) as Arc<dyn DemOperationPort>);
    (service, port)
}

fn sentinel_service(temp: &TempStore) -> ApplicationService {
    ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(SentinelSearch),
        Arc::new(BaseWorker),
    )
}

fn bind(service: &ApplicationService, geometry: Map<String, Value>, name: &str, bbox: Vec<f64>) {
    service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "administrative_boundary".to_owned(),
            geometry,
            bbox,
            crs: "EPSG:4326".to_owned(),
            display_name: name.to_owned(),
        })
        .unwrap();
}

fn request(
    temp: &TempStore,
    geometry: Map<String, Value>,
    name: &str,
) -> StandaloneDemAoiTaskRequest {
    StandaloneDemAoiTaskRequest {
        request_id: "dem-antimeridian-request".to_owned(),
        task_id: "dem-antimeridian".to_owned(),
        // 90 m: the 30 m New Zealand windows (50400 x 46800 samples each) exceed
        // the native mosaic limit and are refused before a task exists.
        dataset: "COP90".to_owned(),
        provider: "COPERNICUS_AWS".to_owned(),
        aoi_geojson: geometry,
        aoi_name: name.to_owned(),
        output_root: temp.root.join("dem-output"),
        convert: false,
        concurrency: 2,
        created_at: "2026-08-19T00:00:00Z".to_owned(),
    }
}

#[test]
fn standalone_dem_aoi_task_id_boundary_matches_ui_generated_identity() {
    let temp = TempStore::new("dem-task-id-boundary");
    let (service, _) = service(&temp);
    let geometry = fixture("new-zealand");
    bind(
        &service,
        geometry.clone(),
        "New Zealand",
        vec![-179.0, -47.0, 179.0, -34.0],
    );

    let mut accepted = request(&temp, geometry.clone(), "New Zealand");
    accepted.request_id = "dem-task-id-64-request".to_owned();
    accepted.task_id = "d".repeat(64);
    service
        .create_standalone_dem_aoi_task(&accepted)
        .expect("the UI-bounded 64-byte task identity must be accepted");

    let mut rejected = request(&temp, geometry, "New Zealand");
    rejected.request_id = "dem-task-id-65-request".to_owned();
    rejected.task_id = "d".repeat(65);
    let error = service
        .create_standalone_dem_aoi_task(&rejected)
        .expect_err("task identities longer than the native path segment limit must be rejected");
    assert_eq!(error.code(), "V3_APPLICATION_INVALID_REQUEST");
    assert!(matches!(
        error,
        insar_application_service::ApplicationServiceError::InvalidRequest { reason }
            if reason == "standalone DEM AOI task identity is invalid"
    ));
}

fn task_page(service: &ApplicationService, task_id: &str) -> insar_core::StoreTaskPage {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let revision = service
            .core_restore(false)
            .unwrap()
            .core
            .tasks
            .iter()
            .find(|task| task.task_id == task_id)
            .unwrap()
            .revision;
        match service.task_page(&StoreTaskPageRequest {
            task_id: task_id.to_owned(),
            expected_revision: revision,
            page_index: 0,
            page_size: 10,
        }) {
            Ok(page) => return page,
            Err(error)
                if error.code() == "V3_STORE_REVISION_CONFLICT" && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("task page failed: {error:?}"),
        }
    }
}

#[test]
fn new_zealand_dem_is_one_task_with_two_bounded_items() {
    let temp = TempStore::new("nz-dem");
    let (service, port) = service(&temp);
    let geometry = fixture("new-zealand");
    bind(
        &service,
        geometry.clone(),
        "New Zealand",
        vec![-179.0, -47.0, 179.0, -34.0],
    );

    let snapshot = service
        .create_standalone_dem_aoi_task(&request(&temp, geometry, "New Zealand"))
        .unwrap();
    let task = snapshot
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "dem-antimeridian")
        .unwrap();
    assert_eq!(task.selected_count, 2);
    let page = task_page(&service, &task.task_id);
    assert_eq!(page.total_count, 2);
    assert_eq!(page.items.len(), 2);
    assert_eq!(
        page.items[0].item.display_name,
        "COP90_EGM2008_20260819_5eb2f326_west.tif"
    );
    assert_eq!(
        page.items[1].item.display_name,
        "COP90_EGM2008_20260819_5eb2f326_east.tif"
    );
    for (index, item) in page.items.iter().enumerate() {
        let bbox = item.item.payload["bbox"].as_object().unwrap();
        let west = bbox["west"].as_f64().unwrap();
        let east = bbox["east"].as_f64().unwrap();
        assert!(west < east);
        assert!(east - west < 30.0);
        assert_eq!(item.item.payload["window_index"], index as u64);
        assert_eq!(item.item.payload["window_count"], 2);
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    while port
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(operation, _)| operation == "dem_finalize_windows")
        .count()
        < 1
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    let calls = port.calls.lock().unwrap().clone();
    let mut executed = calls
        .iter()
        .filter(|(operation, _)| operation == "dem_download")
        .map(|(_, payload)| payload.clone())
        .collect::<Vec<_>>();
    executed.sort_by_key(|payload| payload["window_index"].as_u64().unwrap());
    assert_eq!(executed.len(), 2);
    assert!(executed[0]["input_path"]
        .as_str()
        .unwrap()
        .ends_with("COP90_EGM2008_20260819_5eb2f326_west.tif"));
    assert!(executed[1]["input_path"]
        .as_str()
        .unwrap()
        .ends_with("COP90_EGM2008_20260819_5eb2f326_east.tif"));
    assert_ne!(executed[0]["input_path"], executed[1]["input_path"]);
    assert_eq!(executed[0]["bbox"]["west"], 166.0);
    assert_eq!(executed[0]["bbox"]["east"], 180.0);
    assert_eq!(executed[1]["bbox"]["west"], -180.0);
    assert_eq!(executed[1]["bbox"]["east"], -175.0);

    let finalizations = calls
        .iter()
        .filter(|(operation, _)| operation == "dem_finalize_windows")
        .collect::<Vec<_>>();
    assert_eq!(finalizations.len(), 1);
    let finalization = &finalizations[0].1;
    assert_eq!(finalization["window_inputs"].as_array().unwrap().len(), 2);
    assert!(finalization["merged_input_path"]
        .as_str()
        .unwrap()
        .ends_with("COP90_EGM2008_20260819_5eb2f326.tif"));
    assert_eq!(finalization["source_vertical_datum"], "EGM2008");
    assert_eq!(finalization["target_vertical_datum"], "WGS84_ELLIPSOID");
    assert_eq!(finalization["convert"], false);
}

#[test]
fn failed_second_window_keeps_one_task_and_never_finalizes_partial_dem() {
    let temp = TempStore::new("failed-window");
    let port = Arc::new(RecordingDemPort {
        calls: Mutex::new(Vec::new()),
        fail_window: Some(1),
        failure_message: None,
        malformed_window: None,
    });
    let (service, port) = service_with_port(&temp, port);
    let geometry = fixture("new-zealand");
    bind(
        &service,
        geometry.clone(),
        "New Zealand",
        vec![-179.0, -47.0, 179.0, -34.0],
    );
    let stale_root = temp
        .root
        .join("dem-output")
        .join(".eostage")
        .join("dem-tasks")
        .join("dem-antimeridian");
    fs::create_dir_all(&stale_root).unwrap();
    fs::write(
        stale_root.join("COP90_dem_20260819_5eb2f326_west.tif"),
        b"stale-west",
    )
    .unwrap();
    fs::write(
        stale_root.join("COP90_dem_20260819_5eb2f326_east.tif"),
        b"stale-east",
    )
    .unwrap();
    service
        .create_standalone_dem_aoi_task(&request(&temp, geometry, "New Zealand"))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    let task = loop {
        let task = service
            .core_restore(true)
            .unwrap()
            .core
            .tasks
            .into_iter()
            .find(|task| task.task_id == "dem-antimeridian")
            .unwrap();
        if task.completed_count + task.failed_count == 2 || Instant::now() >= deadline {
            break task;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(task.selected_count, 2);
    assert_eq!(task.completed_count, 1);
    assert_eq!(task.failed_count, 1);
    assert_eq!(
        port.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(operation, _)| operation == "dem_finalize_windows")
            .count(),
        0
    );
    assert_eq!(
        fs::read(stale_root.join("COP90_dem_20260819_5eb2f326_west.tif")).unwrap(),
        b"stale-west"
    );
    assert_eq!(
        fs::read(stale_root.join("COP90_dem_20260819_5eb2f326_east.tif")).unwrap(),
        b"stale-east"
    );
}

#[test]
fn ordinary_hubei_dem_remains_one_item() {
    let temp = TempStore::new("hubei-dem");
    let (service, _port) = service(&temp);
    let geometry = fixture("hubei");
    bind(
        &service,
        geometry.clone(),
        "湖北省",
        vec![108.0, 29.0, 116.0, 34.0],
    );

    let snapshot = service
        .create_standalone_dem_aoi_task(&request(&temp, geometry, "湖北省"))
        .unwrap();
    let task = snapshot
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "dem-antimeridian")
        .unwrap();
    assert_eq!(task.selected_count, 1);
    let page = task_page(&service, &task.task_id);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].item.payload["bbox"]["west"], 108.0);
    assert_eq!(page.items[0].item.payload["bbox"]["east"], 116.0);
}

fn dense_ring(west: f64, south: f64, east: f64, north: f64) -> Vec<Vec<f64>> {
    let corners = [
        [west, south],
        [east, south],
        [east, north],
        [west, north],
        [west, south],
    ];
    let mut ring = Vec::new();
    for edge in corners.windows(2) {
        for step in 0..200 {
            let t = f64::from(step) / 200.0;
            ring.push(vec![
                edge[0][0] + (edge[1][0] - edge[0][0]) * t,
                edge[0][1] + (edge[1][1] - edge[0][1]) * t,
            ]);
        }
    }
    ring.push(vec![west, south]);
    ring
}

#[test]
fn detailed_valid_multipart_dem_uses_one_exact_rectangular_request() {
    let geometry = json!({"type":"MultiPolygon", "coordinates":[
        [dense_ring(100.0,30.0,100.01,30.01), dense_ring(100.002,30.002,100.008,30.008)],
        [dense_ring(100.003,30.003,100.007,30.007)]
    ]})
    .as_object()
    .unwrap()
    .clone();
    let authoritative = insar_core::validated_authoritative_polygonal_geojson(&geometry)
        .expect("detailed mainland multipolygon is valid AOI authority");
    assert!(
        insar_core::wrapped_aoi_envelope(&authoritative.geometry).is_none(),
        "fixture must reproduce the unrelated projected-topology budget rejection"
    );
    let temp = TempStore::new("detailed-multipart-dem");
    let (service, _) = service(&temp);
    bind(
        &service,
        geometry.clone(),
        "Detailed test AOI",
        vec![100.0, 30.0, 100.01, 30.01],
    );
    let snapshot = service
        .create_standalone_dem_aoi_task(&request(&temp, geometry, "Detailed test AOI"))
        .expect(
            "an admitted non-crossing AOI must produce its rectangle without render projection",
        );
    let task = snapshot
        .core
        .tasks
        .iter()
        .find(|task| task.task_id == "dem-antimeridian")
        .unwrap();
    assert_eq!(task.selected_count, 1);
    let page = task_page(&service, &task.task_id);
    assert_eq!(
        page.items[0].item.payload["bbox"],
        json!({"west":100.0,"south":30.0,"east":100.01,"north":30.01,"crs":"EPSG:4326"})
    );
}

#[test]
fn native_dem_failure_categories_reach_store_without_raw_context() {
    for (raw, expected_code) in [
        (
            "NATIVE_DEM_OUTPUT_LIMIT: fixture-private-context",
            "NATIVE_DEM_OUTPUT_LIMIT",
        ),
        (
            "NATIVE_DEM_CREDENTIAL_REQUIRED: fixture-private-context",
            "NATIVE_DEM_CREDENTIAL_REQUIRED",
        ),
        (
            "UNRECOGNIZED_FAILURE: fixture-private-context",
            "NATIVE_DEM_OPERATION_FAILED",
        ),
    ] {
        let temp = TempStore::new("native-dem-failure-category");
        let port = Arc::new(RecordingDemPort {
            failure_message: Some(raw),
            fail_window: Some(0),
            ..RecordingDemPort::default()
        });
        let (service, _) = service_with_port(&temp, port);
        let geometry = fixture("hubei");
        bind(
            &service,
            geometry.clone(),
            "湖北省",
            vec![108.0, 29.0, 116.0, 34.0],
        );
        service
            .create_standalone_dem_aoi_task(&request(&temp, geometry, "湖北省"))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let failure = loop {
            let page = task_page(&service, "dem-antimeridian");
            if let Some(failure) = page.items.first().and_then(|item| item.failure.clone()) {
                break failure;
            }
            assert!(
                Instant::now() < deadline,
                "native failure must be persisted"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(failure.code, expected_code);
        assert!(!failure.reason_zh.contains("fixture-private-context"));
        assert!(
            !failure.retryable,
            "configuration/size/unknown errors must not auto-retry"
        );
    }
}

#[test]
fn dem_operation_response_requires_explicit_true_success() {
    let temp = TempStore::new("malformed-dem-response");
    let port = Arc::new(RecordingDemPort {
        calls: Mutex::new(Vec::new()),
        fail_window: None,
        malformed_window: Some(0),
        failure_message: None,
    });
    let (service, _port) = service_with_port(&temp, port);
    let geometry = fixture("hubei");
    bind(
        &service,
        geometry.clone(),
        "湖北省",
        vec![108.0, 29.0, 116.0, 34.0],
    );
    service
        .create_standalone_dem_aoi_task(&request(&temp, geometry, "湖北省"))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let task = service
            .core_restore(true)
            .unwrap()
            .core
            .tasks
            .into_iter()
            .find(|task| task.task_id == "dem-antimeridian")
            .unwrap();
        if task.failed_count == 1 {
            assert_eq!(task.completed_count, 0);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "malformed native response did not fail closed"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn stale_dem_aoi_geometry_or_display_identity_fails_closed() {
    let temp = TempStore::new("stale-dem");
    let (service, _port) = service(&temp);
    let hubei = fixture("hubei");
    bind(
        &service,
        hubei.clone(),
        "湖北省",
        vec![108.0, 29.0, 116.0, 34.0],
    );

    let stale_geometry = service
        .create_standalone_dem_aoi_task(&request(&temp, fixture("new-zealand"), "湖北省"))
        .unwrap_err();
    assert_eq!(stale_geometry.code(), "V3_APPLICATION_INVALID_REQUEST");

    let stale_name = service
        .create_standalone_dem_aoi_task(&request(&temp, hubei, "Hubei"))
        .unwrap_err();
    assert_eq!(stale_name.code(), "V3_APPLICATION_INVALID_REQUEST");
    assert!(service.core_restore(false).unwrap().core.tasks.is_empty());
}

#[test]
fn gacos_uses_two_store_derived_windows_under_one_plan_identity() {
    let temp = TempStore::new("nz-gacos");
    let (service, _port) = service(&temp);
    let geometry = fixture("new-zealand");
    bind(
        &service,
        geometry.clone(),
        "New Zealand",
        vec![-179.0, -47.0, 179.0, -34.0],
    );

    let response = service
        .gacos_plan(&json!({
            "request_id": "gacos-new-zealand",
            "aoi_geojson": geometry,
            "aoi_name": "New Zealand",
            "bbox": {"west": -179.0, "south": -47.0, "east": 179.0, "north": -34.0},
            "output_dir": temp.root.join("gacos").to_string_lossy(),
            "source_kind": "online_selected",
            "selected_scene_ids": ["S1"],
            "scenes": [{"scene_id": "S1", "acquisition_datetime": "2024-01-02T03:04:05Z"}]
        }))
        .unwrap();
    assert_eq!(response["report"]["valid"], false);
    assert_eq!(response["report"]["has_errors"], true);
    assert!(response["report"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .all(|issue| issue["code"] == "GACOS_BBOX_TOO_LARGE"));
    let plan = &response["plan"];
    assert_eq!(plan["request_id"], "gacos-new-zealand");
    assert_eq!(plan["processing_windows"].as_array().unwrap().len(), 2);
    assert_eq!(plan["request_windows"].as_array().unwrap().len(), 2);
    assert!(plan.get("processing_bbox").is_none());
    assert!(plan.get("request_bbox").is_none());
    for window in plan["processing_windows"].as_array().unwrap() {
        let west = window["west"].as_f64().unwrap();
        let east = window["east"].as_f64().unwrap();
        assert!(west < east);
        assert!(east - west < 30.0);
    }
    assert_eq!(plan["request_windows"][0]["west"], 165.95);
    assert_eq!(plan["request_windows"][0]["east"], 180.0);
    assert_eq!(plan["request_windows"][1]["west"], -180.0);
    assert_eq!(plan["request_windows"][1]["east"], -174.95);
    assert_eq!(plan["request_windows"][0]["south"], -47.05);
    assert_eq!(plan["request_windows"][0]["north"], -33.95);
    assert!(plan["batches"].as_array().unwrap().iter().all(|batch| {
        batch["processing_windows"].as_array().unwrap().len() == 2
            && batch["request_windows"].as_array().unwrap().len() == 2
            && batch["requests"].as_array().unwrap().len() == 2
    }));
    let requests = plan["batches"][0]["requests"].as_array().unwrap();
    assert_eq!(requests[0]["bbox"], plan["request_windows"][0]);
    assert_eq!(requests[1]["bbox"], plan["request_windows"][1]);
}

#[test]
fn gacos_requires_exact_selected_snapshot_and_normalizes_rfc3339_to_utc() {
    let temp = TempStore::new("hubei-gacos-utc");
    let (service, _port) = service(&temp);
    let geometry = fixture("hubei");
    bind(
        &service,
        geometry.clone(),
        "湖北省",
        vec![108.0, 29.0, 116.0, 34.0],
    );

    let rejected = service
        .gacos_plan(&json!({
            "request_id": "gacos-empty-selection",
            "aoi_geojson": geometry.clone(),
            "aoi_name": "湖北省",
            "output_dir": temp.root.join("gacos").to_string_lossy(),
            "source_kind": "online_selected",
            "selected_scene_ids": [],
            "scenes": [{"scene_id": "S1", "acquisition_datetime": "2024-01-02T03:04:05Z"}]
        }))
        .unwrap_err();
    assert_eq!(rejected.code(), "V3_APPLICATION_INVALID_REQUEST");

    let response = service
        .gacos_plan(&json!({
            "request_id": "gacos-utc-offset",
            "aoi_geojson": geometry,
            "aoi_name": "湖北省",
            "output_dir": temp.root.join("gacos").to_string_lossy(),
            "source_kind": "online_selected",
            "selected_scene_ids": ["S1"],
            "scenes": [{"scene_id": "S1", "acquisition_datetime": "2024-01-02T23:30:00-05:00"}]
        }))
        .unwrap();
    assert_eq!(response["report"]["valid"], true);
    assert_eq!(response["plan"]["unique_dates"], json!(["2024-01-03"]));
    assert_eq!(response["plan"]["batches"][0]["hour"], 4);
    assert_eq!(response["plan"]["batches"][0]["minute"], 30);
    assert_eq!(
        response["plan"]["batches"][0]["requests"][0]["form_fields"]["date"],
        "20240103"
    );
}

#[test]
fn gacos_date_only_scene_requires_and_accepts_explicit_utc_time() {
    let temp = TempStore::new("hubei-gacos-date-only");
    let (service, _port) = service(&temp);
    let geometry = fixture("hubei");
    bind(
        &service,
        geometry.clone(),
        "湖北省",
        vec![108.0, 29.0, 116.0, 34.0],
    );
    let base = json!({
        "request_id": "gacos-date-only",
        "aoi_geojson": geometry,
        "aoi_name": "湖北省",
        "output_dir": temp.root.join("gacos").to_string_lossy(),
        "source_kind": "online_selected",
        "selected_scene_ids": ["S1"],
        "scenes": [{"scene_id": "S1", "acquisition_datetime": "2024-01-02"}]
    });
    let blocked = service.gacos_plan(&base).unwrap();
    assert_eq!(blocked["report"]["valid"], false);
    assert_eq!(blocked["plan"]["date_only_count"], 1);
    assert!(blocked["report"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "GACOS_TIME_REQUIRED"));

    let mut supplied = base;
    supplied["request_id"] = Value::String("gacos-date-only-supplied".to_owned());
    supplied["user_supplied_times"] = json!({"date_only": [7, 8]});
    let accepted = service.gacos_plan(&supplied).unwrap();
    assert_eq!(accepted["report"]["valid"], true);
    assert_eq!(accepted["plan"]["date_only_count"], 0);
    assert_eq!(accepted["plan"]["batches"][0]["hour"], 7);
    assert_eq!(accepted["plan"]["batches"][0]["minute"], 8);
}

fn gacos_evidence_plan(service: &ApplicationService, temp: &TempStore, scenes: Value) -> Value {
    let geometry = fixture("hubei");
    bind(
        service,
        geometry.clone(),
        "湖北省",
        vec![108.0, 29.0, 116.0, 34.0],
    );
    let selected = scenes
        .as_array()
        .unwrap()
        .iter()
        .map(|scene| scene["scene_id"].clone())
        .collect::<Vec<_>>();
    service
        .gacos_plan(&json!({
            "request_id": "gacos-temporal-evidence",
            "aoi_geojson": geometry,
            "aoi_name": "湖北省",
            "output_dir": temp.root.join("gacos").to_string_lossy(),
            "source_kind": "online_selected",
            "selected_scene_ids": selected,
            "scenes": scenes
        }))
        .unwrap()
}

#[test]
fn gacos_merges_stable_aliases_and_prefers_exact_time_over_date_only() {
    let temp = TempStore::new("gacos-merge-aliases");
    let (service, _) = service(&temp);
    let result = gacos_evidence_plan(
        &service,
        &temp,
        json!([
            {"scene_id": "alias-one", "stable_scene_id": "same-scene", "acquisition_datetime": "2024-01-02"},
            {"scene_id": "alias-two", "stable_scene_id": "same-scene", "acquisition_datetime": "2024-01-02T07:08:09Z"}
        ]),
    );
    assert_eq!(result["report"]["valid"], true);
    assert_eq!(result["plan"]["selected_scene_count"], 2);
    assert_eq!(result["plan"]["recognized_scene_count"], 1);
    assert_eq!(result["plan"]["duplicate_scene_count"], 1);
    assert_eq!(result["plan"]["date_only_count"], 0);
    assert_eq!(result["plan"]["batches"][0]["hour"], 7);
    assert_eq!(result["plan"]["batches"][0]["minute"], 8);
    assert_eq!(result["plan"]["batches"][0]["date_count"], 1);
}

#[test]
fn gacos_conflicting_stable_scene_times_block_submission_and_emit_no_conflicted_request() {
    let temp = TempStore::new("gacos-time-conflict");
    let (service, _) = service(&temp);
    let result = gacos_evidence_plan(
        &service,
        &temp,
        json!([
            {"scene_id": "alias-one", "stable_scene_id": "same-scene", "acquisition_datetime": "2024-01-02T07:08:00Z"},
            {"scene_id": "alias-two", "stable_scene_id": "same-scene", "acquisition_datetime": "2024-01-02T07:09:00Z"}
        ]),
    );
    assert_eq!(result["report"]["valid"], false);
    assert_eq!(result["plan"]["time_conflict_count"], 1);
    assert_eq!(result["plan"]["batch_count"], 0);
    assert!(result["report"]["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "GACOS_TIME_CONFLICT"));
}

#[test]
fn gacos_keeps_utc_groups_and_twenty_date_batches_with_provider_wire_dates() {
    let temp = TempStore::new("gacos-batch-date-format");
    let (service, _) = service(&temp);
    let mut scenes = (1..=21)
        .map(|day| {
            json!({
                "scene_id": format!("morning-{day}"),
                "acquisition_datetime": format!("2024-01-{day:02}T03:04:00Z")
            })
        })
        .collect::<Vec<_>>();
    scenes.push(json!({"scene_id": "evening", "acquisition_datetime": "2024-01-01T15:16:00Z"}));
    let result = gacos_evidence_plan(&service, &temp, json!(scenes));
    assert_eq!(result["report"]["valid"], true);
    assert_eq!(result["plan"]["time_group_count"], 2);
    assert_eq!(result["plan"]["batch_count"], 3);
    assert_eq!(result["report"]["estimated_minimum_seconds"], 40);
    let batches = result["plan"]["batches"].as_array().unwrap();
    assert_eq!(batches[0]["date_count"], 20);
    assert_eq!(batches[1]["date_count"], 1);
    assert_eq!(batches[2]["hour"], 15);
    for batch in batches {
        let dates = batch["requests"][0]["form_fields"]["date"]
            .as_str()
            .unwrap();
        assert_eq!(
            dates.lines().count() as u64,
            batch["date_count"].as_u64().unwrap()
        );
        assert!(dates
            .lines()
            .all(|date| date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit())));
    }
}

#[test]
fn gacos_nullable_acquisition_field_keeps_explicit_fallback_date() {
    let temp = TempStore::new("gacos-null-time");
    let (service, _) = service(&temp);
    let result = gacos_evidence_plan(
        &service,
        &temp,
        json!([
            {"scene_id": "nullable", "acquisition_datetime": null, "date": "2024-01-02"}
        ]),
    );
    assert_eq!(result["report"]["valid"], false);
    assert_eq!(result["plan"]["unique_dates"], json!(["2024-01-02"]));
    assert_eq!(result["plan"]["date_only_count"], 1);
    assert!(result["plan"]["batches"][0]["hour"].is_null());
}

#[test]
fn gacos_manual_dates_allow_no_scene_snapshot_and_normalize_calendar_dates() {
    let temp = TempStore::new("gacos-manual-dates");
    let (service, _) = service(&temp);
    let response = service
        .gacos_plan(&json!({
            "request_id": "gacos-manual-dates",
            "aoi_geojson": fixture("hubei"),
            "aoi_name": "手填 AOI",
            "aoi_source": "frozen_import",
            "source_kind": "manual_dates",
            "manual_dates": ["20240229", "20240102", "20240229"],
            "hour": 7,
            "minute": 8,
            "output_format": "binary"
        }))
        .unwrap();
    assert_eq!(response["report"]["valid"], true);
    assert_eq!(response["plan"]["selected_scene_count"], 0);
    assert_eq!(response["plan"]["source_scope"], "manual_dates");
    assert_eq!(
        response["plan"]["unique_dates"],
        json!(["2024-01-02", "2024-02-29"])
    );
    let request = &response["plan"]["batches"][0]["requests"][0];
    assert_eq!(request["form_fields"]["date"], "20240102\n20240229");
    assert_eq!(request["form_fields"]["type"], "1");
    assert_eq!(request["form_fields"]["output_format"], "binary");
    assert_eq!(response["plan"]["batches"][0]["hour"], 7);
    assert_eq!(response["plan"]["batches"][0]["minute"], 8);
}

#[test]
fn gacos_manual_dates_reject_invalid_calendar_dates_and_utc_time() {
    let temp = TempStore::new("gacos-manual-invalid");
    let (service, _) = service(&temp);
    let base = json!({
        "request_id": "gacos-manual-invalid",
        "aoi_geojson": fixture("hubei"),
        "aoi_name": "手填 AOI",
        "aoi_source": "frozen_import",
        "source_kind": "manual_dates",
        "manual_dates": ["20240229"],
        "hour": 7,
        "minute": 8
    });
    let mut invalid_date = base.clone();
    invalid_date["manual_dates"] = json!(["20240230"]);
    assert!(service.gacos_plan(&invalid_date).is_err());
    invalid_date["manual_dates"] = json!(["00000101"]);
    assert!(service.gacos_plan(&invalid_date).is_err());
    let mut invalid_hour = base.clone();
    invalid_hour["hour"] = json!(24);
    assert!(service.gacos_plan(&invalid_hour).is_err());
    let mut invalid_minute = base;
    invalid_minute["minute"] = json!(60);
    assert!(service.gacos_plan(&invalid_minute).is_err());
}

#[test]
fn gacos_output_format_is_part_of_the_frozen_plan_identity() {
    let temp = TempStore::new("gacos-output-format");
    let (service, _) = service(&temp);
    let output_dir = temp.root.join("gacos-output").to_string_lossy().to_string();
    let plan_for = |request_id: &str, output_format: &str| {
        service
            .gacos_plan(&json!({
                "request_id": request_id,
                "aoi_geojson": fixture("hubei"),
                "aoi_name": "手填 AOI",
                "aoi_source": "frozen_import",
                "source_kind": "manual_dates",
                "manual_dates": ["20240229"],
                "hour": 7,
                "minute": 8,
                "output_format": output_format,
                "output_dir": output_dir
            }))
            .unwrap()
    };
    let geotiff = plan_for("gacos-geotiff", "geotiff");
    let binary = plan_for("gacos-binary", "binary");
    assert_eq!(
        geotiff["plan"]["batches"][0]["requests"][0]["form_fields"]["type"],
        "2"
    );
    assert_eq!(
        binary["plan"]["batches"][0]["requests"][0]["form_fields"]["type"],
        "1"
    );
    assert_eq!(geotiff["plan"]["output_format"], "geotiff");
    assert_eq!(binary["plan"]["output_format"], "binary");
    assert_ne!(
        geotiff["plan"]["prepared_batch_id"],
        binary["plan"]["prepared_batch_id"]
    );
}

#[test]
fn gacos_frozen_aoi_survives_a_later_global_aoi_change() {
    let temp = TempStore::new("gacos-frozen-aoi");
    let (service, _) = service(&temp);
    let frozen_geometry = fixture("hubei");
    let output_dir = temp.root.join("gacos-output").to_string_lossy().to_string();
    let payload = json!({
        "request_id": "gacos-frozen-aoi",
        "aoi_geojson": frozen_geometry.clone(),
        "aoi_name": "冻结 AOI",
        "aoi_source": "frozen_import",
        "source_kind": "manual_dates",
        "manual_dates": ["20240229"],
        "hour": 7,
        "minute": 8,
        "output_dir": output_dir
    });
    let first = service.gacos_plan(&payload).unwrap();
    bind(
        &service,
        fixture("new-zealand"),
        "后续全局 AOI",
        vec![-179.0, -47.0, 179.0, -34.0],
    );
    let second = service.gacos_plan(&payload).unwrap();
    assert_eq!(second["report"]["valid"], true);
    assert_eq!(second["plan"]["aoi_source"], "frozen_import");
    assert_eq!(
        second["plan"]["aoi_geojson"].as_object(),
        Some(&frozen_geometry)
    );
    assert_eq!(
        first["plan"]["prepared_batch_id"],
        second["plan"]["prepared_batch_id"]
    );
}

#[test]
fn gacos_online_batch_uses_the_complete_sentinel1_search_freeze() {
    let temp = TempStore::new("gacos-online-batch");
    let service = sentinel_service(&temp);
    let geometry = fixture("hubei");
    let source_batch_id = "s1-frozen-batch".to_owned();
    let _snapshot = service
        .sar_search_and_freeze(&ApplicationSarSearchAndFreezeRequest {
            request_id: "sentinel-search-for-gacos".to_owned(),
            search: serde_json::from_value(json!({
                "query": {
                    "provider": "asf",
                    "dataset": "SENTINEL-1",
                    "max_results": 2,
                    "aoi_geojson": geometry.clone()
                }
            }))
            .unwrap(),
            batch_id: source_batch_id.clone(),
            created_at: "2026-09-06T00:00:00Z".to_owned(),
            aoi_name: "Search AOI".to_owned(),
            aoi_geojson: Some(geometry.clone()),
            page_size: 1,
        })
        .unwrap();
    let response = service
        .gacos_plan(&json!({
            "request_id": "gacos-online-batch",
            "aoi_geojson": geometry,
            "aoi_name": "GACOS AOI",
            "aoi_source": "frozen_import",
            "source_kind": "online_batch",
            "source_batch_id": source_batch_id,
            "scenes": [{"scene_id": "forged-page-scene", "acquisition_datetime": "2020-01-01T00:00:00Z"}],
            "output_dir": temp.root.join("gacos-output").to_string_lossy().to_string()
        }))
        .unwrap();
    assert_eq!(response["report"]["valid"], true);
    assert_eq!(response["plan"]["source_kind"], "online_batch");
    assert_eq!(response["plan"]["source_scope"], "all");
    assert_eq!(response["plan"]["source_batch_candidate_count"], 2);
    assert_eq!(
        response["plan"]["source_provenance"]["provider_id"],
        "asf.catalog"
    );
    assert_eq!(response["plan"]["selected_scene_count"], 2);
    assert_eq!(
        response["plan"]["unique_dates"],
        json!(["2024-02-29", "2024-03-12"])
    );
    assert_eq!(response["plan"]["batches"][0]["date_count"], 2);
    assert_eq!(response["plan"]["batches"][0]["hour"], 7);
    assert_eq!(response["plan"]["batches"][0]["minute"], 8);
    assert_eq!(
        response["plan"]["batches"][0]["requests"][0]["form_fields"]["date"],
        "20240229\n20240312"
    );
}
