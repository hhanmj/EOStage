use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_application_service::{
    sign_mpc_optical_request_in_memory, ApplicationService, ApplicationServiceConfig,
    ApplicationServiceError, DemOperationPort, DemTransferEvent, OpticalDerivedCreateTaskRequest,
    OpticalDerivedPrecheckBatchRequest,
};
use insar_core::{
    FrozenSearchBatch, ProviderNetworkPolicy, SearchWorkbenchState, SnapshotItem,
    StoreSearchBatchRequest,
};
use insar_gis::{
    LazyNativeGisEngine, OpticalCloudMaskPolicy, OpticalDerivedMode, OpticalDerivedRequest,
    OpticalMosaicStrategy, OpticalResampling,
};
use serde_json::{json, Map, Value};

struct TempWorkspace {
    root: PathBuf,
    database: PathBuf,
    output_dir: PathBuf,
    fixtures_dir: PathBuf,
}

impl TempWorkspace {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "insar-app-optical-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        let fixtures_dir = root.join("fixtures");
        let output_dir = root.join("output");
        fs::create_dir_all(&fixtures_dir).expect("create fixtures dir");
        fs::create_dir_all(&output_dir).expect("create output dir");
        Self {
            database: root.join("store.sqlite"),
            root,
            output_dir,
            fixtures_dir,
        }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn load_real_gis_engine() -> LazyNativeGisEngine {
    if let Some(root) = std::env::var_os("INSAR_GIS_RUNTIME_ROOT")
        .or_else(|| std::env::var_os("INSAR_NATIVE_GIS_RUNTIME_ROOT"))
    {
        return LazyNativeGisEngine::from_product_root(PathBuf::from(root))
            .expect("INSAR_GIS_RUNTIME_ROOT should load real GIS runtime");
    }
    for rel in ["../../runtime/gis", "../../../runtime/gis"] {
        let candidate = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        if candidate.join("manifest.json").is_file() {
            return LazyNativeGisEngine::from_product_root(candidate)
                .expect("relative runtime/gis should load");
        }
    }
    panic!("real GIS runtime is required for optical_derived_delivery integration test");
}

struct RealOpticalGisPort {
    engine: LazyNativeGisEngine,
}

impl DemOperationPort for RealOpticalGisPort {
    fn call_with_transfer_progress(
        &self,
        operation: &str,
        payload: Value,
        interrupted: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(DemTransferEvent),
    ) -> Result<Value, String> {
        if operation != "optical_derived_execute" {
            return Err(format!("UNSUPPORTED_TEST_OP: {operation}"));
        }
        let mut req: OpticalDerivedRequest =
            serde_json::from_value(payload).map_err(|e| format!("SPEC_PARSE: {e}"))?;
        sign_mpc_optical_request_in_memory(&mut req, ProviderNetworkPolicy::direct(), interrupted)?;
        if let Some(parent) = req.output_tif_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("OUTPUT_MKDIR: {e}"))?;
        }
        progress(DemTransferEvent::BodyStarted);
        let mut last_bytes = 0u64;
        let result = self
            .engine
            .execute_optical_derived(&req, interrupted, &mut |prog| {
                let delta = prog.bytes_written_estimate.saturating_sub(last_bytes);
                if delta > 0 {
                    last_bytes = prog.bytes_written_estimate;
                    progress(DemTransferEvent::BytesReceived(delta));
                }
            });
        progress(DemTransferEvent::BodyStopped);
        match result {
            Ok(out) => serde_json::to_value(out).map_err(|e| format!("SERIALIZE: {e}")),
            Err(err) => Err(err.to_string()),
        }
    }

    fn call(&self, operation: &str, payload: Value) -> Result<Value, String> {
        self.call_with_transfer_progress(operation, payload, &|| false, &mut |_| {})
    }
}

fn polygon_geojson_from_bbox(bbox: [f64; 4]) -> Map<String, Value> {
    let [min_x, min_y, max_x, max_y] = bbox;
    serde_json::from_value(json!({
        "type": "Polygon",
        "coordinates": [[
            [min_x, min_y],
            [max_x, min_y],
            [max_x, max_y],
            [min_x, max_y],
            [min_x, min_y]
        ]]
    }))
    .expect("polygon geojson")
}

fn make_s2_inline_item(
    item_id: &str,
    display_name: &str,
    acquired_at: &str,
    cloud_cover: f64,
    bbox: [f64; 4],
    band_paths: (&Path, &Path, Option<&Path>),
) -> SnapshotItem {
    let (b04_path, b03_path, scl_path) = band_paths;
    let mut assets = vec![
        json!({
            "asset_key": "B04",
            "stable_href": b04_path.to_string_lossy(),
            "band_resolution_m": 200.0,
            "role": "data"
        }),
        json!({
            "asset_key": "B03",
            "stable_href": b03_path.to_string_lossy(),
            "band_resolution_m": 200.0,
            "role": "data"
        }),
    ];
    if let Some(scl) = scl_path {
        assets.push(json!({
            "asset_key": "SCL",
            "stable_href": scl.to_string_lossy(),
            "band_resolution_m": 200.0,
            "role": "cloud"
        }));
    }
    let mut metadata = Map::new();
    metadata.insert(
        "logical_dataset".to_owned(),
        Value::String("optical.sentinel-2.l2a".to_owned()),
    );
    metadata.insert(
        "native_collection_id".to_owned(),
        Value::String("sentinel-2-l2a".to_owned()),
    );
    metadata.insert("cloud_cover".to_owned(), json!(cloud_cover));
    metadata.insert("proj_epsg".to_owned(), json!(32650));
    metadata.insert("assets".to_owned(), Value::Array(assets));

    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "microsoft.planetary-computer".to_owned(),
        display_name: display_name.to_owned(),
        acquisition_datetime: Some(acquired_at.to_owned()),
        footprint_geojson: Some(polygon_geojson_from_bbox(bbox)),
        metadata,
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn wait_for_task_terminal(service: &ApplicationService, task_id: &str) -> (String, Value) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = service.core_restore(false).expect("core_restore");
        let core = serde_json::to_value(&snapshot.core).expect("snapshot json");
        if let Some(tasks) = core.get("tasks").and_then(Value::as_array) {
            if let Some(task) = tasks
                .iter()
                .find(|t| t.get("task_id").and_then(Value::as_str) == Some(task_id))
            {
                let status = task
                    .get("lifecycle_status")
                    .or_else(|| task.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
                    return (status, core);
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for optical derived task {task_id} to reach terminal state"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn invalid_reason(err: ApplicationServiceError) -> String {
    match err {
        ApplicationServiceError::InvalidRequest { reason } => reason,
        other => format!("{other:?}"),
    }
}

fn build_raster_tile(
    engine: &LazyNativeGisEngine,
    path: &Path,
    geotransform: [f64; 6],
    width: i32,
    height: i32,
    val: u16,
) {
    let pixels = vec![val; (width * height) as usize];
    engine
        .write_test_geotiff_u16(
            path,
            32650,
            geotransform,
            [width, height],
            Some(0.0),
            &pixels,
        )
        .expect("write test u16 geotiff");
}

#[test]
#[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
fn optical_derived_precheck_clip_mosaic_and_sqlite_security_e2e() {
    let temp = TempWorkspace::new("e2e");
    let engine = load_real_gis_engine();

    // EPSG:32650 covers 117E central meridian (500,000 E).
    // Grid covers E: 490_000..510_000, N: 3_510_000..3_530_000 (~116.89..117.11E, 31.72..31.91N)
    let gt = [490_000.0, 200.0, 0.0, 3_530_000.0, 0.0, -200.0];
    let scene_bbox = [116.89, 31.72, 117.11, 31.91];
    let aoi_bbox = [116.95, 31.78, 117.05, 31.85];

    let s1_b04 = temp.fixtures_dir.join("s1_b04.tif");
    let s1_b03 = temp.fixtures_dir.join("s1_b03.tif");
    let s1_scl = temp.fixtures_dir.join("s1_scl.tif");
    build_raster_tile(&engine, &s1_b04, gt, 100, 100, 1450);
    build_raster_tile(&engine, &s1_b03, gt, 100, 100, 920);

    // Paint upper-half SCL rows as high-probability cloud (9), lower-half as clear vegetation (4)
    let mut scl_data = vec![4u8; 100 * 100];
    for r in 0..50 {
        for c in 0..100 {
            scl_data[r * 100 + c] = 9;
        }
    }
    engine
        .write_test_geotiff_u8(&s1_scl, 32650, gt, [100, 100], None, &scl_data)
        .expect("write SCL");

    // Second scene 4 days later for spatial mosaic test (lower cloud cover in upper half)
    let s2_b04 = temp.fixtures_dir.join("s2_b04.tif");
    let s2_b03 = temp.fixtures_dir.join("s2_b03.tif");
    let s2_scl = temp.fixtures_dir.join("s2_scl.tif");
    build_raster_tile(&engine, &s2_b04, gt, 100, 100, 2200);
    build_raster_tile(&engine, &s2_b03, gt, 100, 100, 1100);
    engine
        .write_test_geotiff_u8(&s2_scl, 32650, gt, [100, 100], None, &vec![4u8; 100 * 100])
        .expect("write s2 SCL");

    // Scene 3: too far in time (>16 days apart from scene 1) for temporal guard test
    let mut query = Map::new();
    query.insert(
        "logical_dataset".to_owned(),
        Value::String("optical.sentinel-2.l2a".to_owned()),
    );
    query.insert(
        "provider_id".to_owned(),
        Value::String("microsoft.planetary-computer".to_owned()),
    );
    query.insert(
        "native_collection_id".to_owned(),
        Value::String("sentinel-2-l2a".to_owned()),
    );

    let item1 = make_s2_inline_item(
        "s2-item-1",
        "S2B_MSIL2A_20260501T025549_T50SNA",
        "2026-05-01T02:55:49Z",
        25.0,
        scene_bbox,
        (&s1_b04, &s1_b03, Some(&s1_scl)),
    );
    let item2 = make_s2_inline_item(
        "s2-item-2",
        "S2A_MSIL2A_20260505T025549_T50SNA",
        "2026-05-05T02:55:49Z",
        2.0,
        scene_bbox,
        (&s2_b04, &s2_b03, Some(&s2_scl)),
    );
    let item_stale = make_s2_inline_item(
        "s2-item-stale",
        "S2A_MSIL2A_20260615T025549_T50SNA",
        "2026-06-15T02:55:49Z",
        5.0,
        scene_bbox,
        (&s2_b04, &s2_b03, Some(&s2_scl)),
    );
    let item_disjoint = make_s2_inline_item(
        "s2-item-disjoint",
        "S2A_MSIL2A_DISJOINT_T49RBM",
        "2026-05-02T03:00:00Z",
        1.0,
        [100.0, 20.0, 101.0, 21.0],
        (&s2_b04, &s2_b03, Some(&s2_scl)),
    );
    let (item_usgs_tar, usgs_query) = {
        let mut item = SnapshotItem {
            item_id: "landsat-usgs-tar".to_owned(),
            provider_id: "usgs.eros.m2m".to_owned(),
            display_name: "LC09_L2SP_121038_20260501_02_T1".to_owned(),
            acquisition_datetime: Some("2026-05-01T02:30:00Z".to_owned()),
            footprint_geojson: Some(polygon_geojson_from_bbox(scene_bbox)),
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        };
        item.metadata.insert(
            "logical_dataset".to_owned(),
            Value::String("optical.landsat.c2.l2".to_owned()),
        );
        let mut q = Map::new();
        q.insert(
            "logical_dataset".to_owned(),
            Value::String("optical.landsat.c2.l2".to_owned()),
        );
        q.insert(
            "provider_id".to_owned(),
            Value::String("usgs.eros.m2m".to_owned()),
        );
        q.insert(
            "native_collection_id".to_owned(),
            Value::String("landsat_ot_c2_l2".to_owned()),
        );
        (item, q)
    };

    let fresh_created_at = chrono::Utc::now().to_rfc3339();
    {
        let mut store = insar_store::TaskStore::open(&temp.database).expect("open store");
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "opt-src-batch".to_owned(),
                    provider_id: "microsoft.planetary-computer".to_owned(),
                    created_at: fresh_created_at.clone(),
                    aoi_name: "HefeiTest".to_owned(),
                    aoi_geojson: Some(polygon_geojson_from_bbox(aoi_bbox)),
                    query: query.clone(),
                    search_stats: None,
                    candidates: vec![item1, item2, item_stale, item_disjoint],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "opt-src-batch".to_owned(),
                    checked_item_ids: vec!["s2-item-1".to_owned()],
                    page_index: 0,
                    page_size: 20,
                    revision: 1,
                },
            })
            .expect("save optical batch");

        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: false,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "usgs-src-batch".to_owned(),
                    provider_id: "usgs.eros.m2m".to_owned(),
                    created_at: fresh_created_at,
                    aoi_name: "HefeiTest".to_owned(),
                    aoi_geojson: Some(polygon_geojson_from_bbox(aoi_bbox)),
                    query: usgs_query,
                    search_stats: None,
                    candidates: vec![item_usgs_tar],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "usgs-src-batch".to_owned(),
                    checked_item_ids: vec!["landsat-usgs-tar".to_owned()],
                    page_index: 0,
                    page_size: 20,
                    revision: 0,
                },
            })
            .expect("save usgs batch");
    }

    let port = Arc::new(RealOpticalGisPort {
        engine: engine.clone(),
    });
    let service = ApplicationService::with_dem_operation_port(
        ApplicationServiceConfig::business_store(&temp.database),
        port,
    );

    // 1. Precheck intersects + disjoint + temporal span rejection + USGS tar rejection
    let precheck_ok = service
        .precheck_optical_aoi_coverage(OpticalDerivedPrecheckBatchRequest {
            source_batch_id: "opt-src-batch".to_owned(),
            expected_workbench_revision: 1,
            item_ids: vec!["s2-item-1".to_owned()],
            mode: None,
            aoi_bbox_wgs84: None,
            aoi_rings_wgs84: None,
            target_resolution_m: None,
        })
        .expect("precheck intersecting scene");
    assert!(
        precheck_ok.ok,
        "expected precheck ok: {:?}",
        precheck_ok.blocking_reasons
    );
    assert!(precheck_ok.combined_aoi_coverage_fraction > 0.99);
    assert_eq!(precheck_ok.recommended_utm_epsg, 32650);

    let precheck_disjoint = service
        .precheck_optical_aoi_coverage(OpticalDerivedPrecheckBatchRequest {
            source_batch_id: "opt-src-batch".to_owned(),
            expected_workbench_revision: 1,
            item_ids: vec!["s2-item-disjoint".to_owned()],
            mode: None,
            aoi_bbox_wgs84: None,
            aoi_rings_wgs84: None,
            target_resolution_m: None,
        })
        .expect("precheck disjoint still returns report");
    assert!(!precheck_disjoint.ok);
    assert!(
        precheck_disjoint
            .blocking_reasons
            .iter()
            .any(|r| r.contains("AOI_SCENE_DISJOINT")),
        "expected AOI_SCENE_DISJOINT, got {:?}",
        precheck_disjoint.blocking_reasons
    );

    let usgs_reason = invalid_reason(
        service
            .precheck_optical_aoi_coverage(OpticalDerivedPrecheckBatchRequest {
                source_batch_id: "usgs-src-batch".to_owned(),
                expected_workbench_revision: 0,
                item_ids: vec!["landsat-usgs-tar".to_owned()],
                mode: None,
                aoi_bbox_wgs84: None,
                aoi_rings_wgs84: None,
                target_resolution_m: None,
            })
            .expect_err("USGS tar must be rejected"),
    );
    assert!(
        usgs_reason.contains("OPTICAL_DERIVED_SOURCE_UNSUPPORTED"),
        "expected OPTICAL_DERIVED_SOURCE_UNSUPPORTED, got: {usgs_reason}"
    );

    // 2. Reject mosaic > 16 days temporal span
    let stale_reason = invalid_reason(
        service
            .create_optical_derived_task(OpticalDerivedCreateTaskRequest {
                request_id: "req-stale-mosaic".to_owned(),
                task_id: "task-stale-mosaic".to_owned(),
                source_batch_id: "opt-src-batch".to_owned(),
                expected_workbench_revision: 1,
                item_ids: vec!["s2-item-1".to_owned(), "s2-item-stale".to_owned()],
                mode: OpticalDerivedMode::NearCoincidentSpatialMosaic,
                aoi_bbox_wgs84: None,
                aoi_rings_wgs84: None,
                band_order: vec!["B04".to_owned(), "B03".to_owned()],
                target_crs: None,
                target_resolution_m: None,
                resampling: OpticalResampling::Bilinear,
                cloud_mask: OpticalCloudMaskPolicy::None,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: Some(16.0),
                overwrite: true,
                write_lineage_sidecar: true,
                write_provenance_sidecars: true,
                output_root: temp.output_dir.clone(),
                concurrency: 1,
                created_at: "2026-05-06T00:01:00Z".to_owned(),
            })
            .expect_err(">16d mosaic must fail precheck"),
    );
    assert!(
        stale_reason.contains("OPTICAL_MOSAIC_TEMPORAL_SPAN_EXCEEDED")
            || stale_reason.contains("16 天"),
        "expected temporal span rejection, got: {stale_reason}"
    );

    // 3. P0 Single-scene AOI clip E2E through DownloadScheduler + CloudToNodata (SCL)
    let created_p0 = service
        .create_optical_derived_task(OpticalDerivedCreateTaskRequest {
            request_id: "req-p0-clip".to_owned(),
            task_id: "task-p0-clip".to_owned(),
            source_batch_id: "opt-src-batch".to_owned(),
            expected_workbench_revision: 1,
            item_ids: vec!["s2-item-1".to_owned()],
            mode: OpticalDerivedMode::SingleSceneClip,
            aoi_bbox_wgs84: None,
            aoi_rings_wgs84: None,
            band_order: vec!["B04".to_owned(), "B03".to_owned()],
            target_crs: None,
            target_resolution_m: None,
            resampling: OpticalResampling::Nearest,
            cloud_mask: OpticalCloudMaskPolicy::CloudToNodata,
            mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
            max_temporal_span_days: None,
            overwrite: true,
            write_lineage_sidecar: true,
            write_provenance_sidecars: true,
            output_root: temp.output_dir.clone(),
            concurrency: 1,
            created_at: "2026-05-06T00:02:00Z".to_owned(),
        })
        .expect("create P0 optical clip task");
    // Raw vs derived filename isolation check
    let out_name = created_p0
        .output_tif_path
        .file_name()
        .expect("filename")
        .to_string_lossy()
        .to_string();
    assert!(out_name.contains("__aoi_clip_"));
    assert_ne!(out_name, "S2B_MSIL2A_20260501T025549_T50SNA");

    let (status_p0, _) = wait_for_task_terminal(&service, "task-p0-clip");
    assert_eq!(status_p0, "completed");

    assert!(created_p0.output_tif_path.is_file());
    let prov_path = PathBuf::from(format!(
        "{}.provenance.json",
        created_p0.output_tif_path.display()
    ));
    let stac_path = created_p0
        .output_tif_path
        .with_extension("source.stac.json");
    let aoi_path = created_p0.output_tif_path.with_extension("aoi.geojson");
    assert!(prov_path.is_file(), "provenance sidecar must exist");
    assert!(stac_path.is_file(), "source stac sidecar must exist");
    assert!(aoi_path.is_file(), "aoi geojson sidecar must exist");

    let prov_json: Value =
        serde_json::from_str(&fs::read_to_string(&prov_path).unwrap()).expect("provenance JSON");
    assert_eq!(
        prov_json
            .pointer("/scientific_policy/cloud_mask_policy")
            .and_then(Value::as_str),
        Some("cloud_to_nodata")
    );
    assert_eq!(
        prov_json
            .pointer("/output/readback_verified")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        prov_json
            .pointer("/output/fallback_pixels")
            .and_then(Value::as_u64),
        Some(0)
    );

    let (w, h, bands, nodata, b1_pixels) = engine
        .read_test_geotiff_u16(&created_p0.output_tif_path, 1)
        .expect("read P0 output GeoTIFF");
    assert!(w > 0 && h > 0 && bands == 2);
    assert_eq!(nodata, Some(0.0));
    assert!(
        b1_pixels.contains(&0),
        "cloudy SCL=9 rows must become NoData (0)"
    );
    assert!(
        b1_pixels.contains(&1450),
        "clear rows must preserve native 1450 reflectance"
    );

    // 5. P1 2-scene near-coincident spatial mosaic E2E
    let created_p1 = service
        .create_optical_derived_task(OpticalDerivedCreateTaskRequest {
            request_id: "req-p1-mosaic".to_owned(),
            task_id: "task-p1-mosaic".to_owned(),
            source_batch_id: "opt-src-batch".to_owned(),
            expected_workbench_revision: 1,
            item_ids: vec!["s2-item-1".to_owned(), "s2-item-2".to_owned()],
            mode: OpticalDerivedMode::NearCoincidentSpatialMosaic,
            aoi_bbox_wgs84: None,
            aoi_rings_wgs84: None,
            band_order: vec!["B04".to_owned(), "B03".to_owned()],
            target_crs: Some("EPSG:32650".to_owned()),
            target_resolution_m: Some(200.0),
            resampling: OpticalResampling::Nearest,
            cloud_mask: OpticalCloudMaskPolicy::CloudToNodata,
            mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
            max_temporal_span_days: Some(16.0),
            overwrite: true,
            write_lineage_sidecar: true,
            write_provenance_sidecars: true,
            output_root: temp.output_dir.clone(),
            concurrency: 1,
            created_at: "2026-05-06T00:03:00Z".to_owned(),
        })
        .expect("create P1 optical mosaic task");
    let (status_p1, _) = wait_for_task_terminal(&service, "task-p1-mosaic");
    assert_eq!(status_p1, "completed");
    assert!(created_p1.output_tif_path.is_file());
    let lineage_index_path = created_p1
        .output_tif_path
        .with_extension("lineage_source_index.tif");
    assert!(
        lineage_index_path.is_file(),
        "spatial mosaic must emit lineage_source_index.tif"
    );
    let prov_p1_path = PathBuf::from(format!(
        "{}.provenance.json",
        created_p1.output_tif_path.display()
    ));
    let prov_p1: Value =
        serde_json::from_str(&fs::read_to_string(&prov_p1_path).unwrap()).expect("p1 provenance");
    assert_eq!(
        prov_p1
            .pointer("/output/readback_verified")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(prov_p1
        .pointer("/output/fallback_pixels")
        .and_then(Value::as_u64)
        .is_some());

    // 5b. P1 fallback mosaic where lower-cloud primary (item_primary_cloudy = 1%) fills lower half
    // and secondary (item2 = 2%) fills upper cloudy half -> fallback_pixels > 0
    let item_primary_cloudy = make_s2_inline_item(
        "s2-item-primary-cloudy",
        "S2B_MSIL2A_20260501T025549_T50SNA_PRIMARY",
        "2026-05-01T02:55:49Z",
        1.0,
        scene_bbox,
        (&s1_b04, &s1_b03, Some(&s1_scl)),
    );
    {
        let mut store =
            insar_store::TaskStore::open(&temp.database).expect("reopen store for batch");
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: false,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "opt-fallback-batch".to_owned(),
                    provider_id: "microsoft.planetary-computer".to_owned(),
                    created_at: "2026-05-06T00:03:30Z".to_owned(),
                    aoi_name: "HefeiTest".to_owned(),
                    aoi_geojson: Some(polygon_geojson_from_bbox(aoi_bbox)),
                    query: query.clone(),
                    search_stats: None,
                    candidates: vec![
                        item_primary_cloudy,
                        make_s2_inline_item(
                            "s2-item-2",
                            "S2A_MSIL2A_20260505T025549_T50SNA",
                            "2026-05-05T02:55:49Z",
                            2.0,
                            scene_bbox,
                            (&s2_b04, &s2_b03, Some(&s2_scl)),
                        ),
                    ],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "opt-fallback-batch".to_owned(),
                    checked_item_ids: vec![
                        "s2-item-primary-cloudy".to_owned(),
                        "s2-item-2".to_owned(),
                    ],
                    page_index: 0,
                    page_size: 20,
                    revision: 0,
                },
            })
            .expect("save fallback batch");
    }
    let created_p1_fb = service
        .create_optical_derived_task(OpticalDerivedCreateTaskRequest {
            request_id: "req-p1-mosaic-fb".to_owned(),
            task_id: "task-p1-mosaic-fb".to_owned(),
            source_batch_id: "opt-fallback-batch".to_owned(),
            expected_workbench_revision: 0,
            item_ids: vec!["s2-item-primary-cloudy".to_owned(), "s2-item-2".to_owned()],
            mode: OpticalDerivedMode::NearCoincidentSpatialMosaic,
            aoi_bbox_wgs84: None,
            aoi_rings_wgs84: None,
            band_order: vec!["B04".to_owned(), "B03".to_owned()],
            target_crs: Some("EPSG:32650".to_owned()),
            target_resolution_m: Some(200.0),
            resampling: OpticalResampling::Nearest,
            cloud_mask: OpticalCloudMaskPolicy::CloudToNodata,
            mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
            max_temporal_span_days: Some(16.0),
            overwrite: true,
            write_lineage_sidecar: true,
            write_provenance_sidecars: true,
            output_root: temp.output_dir.clone(),
            concurrency: 1,
            created_at: "2026-05-06T00:04:00Z".to_owned(),
        })
        .expect("create P1 fallback mosaic task");
    let (status_p1_fb, _) = wait_for_task_terminal(&service, "task-p1-mosaic-fb");
    assert_eq!(status_p1_fb, "completed");
    let prov_fb_path = PathBuf::from(format!(
        "{}.provenance.json",
        created_p1_fb.output_tif_path.display()
    ));
    let prov_fb: Value =
        serde_json::from_str(&fs::read_to_string(&prov_fb_path).unwrap()).expect("fb provenance");
    assert_eq!(
        prov_fb
            .pointer("/output/readback_verified")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        prov_fb
            .pointer("/output/fallback_pixels")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "fallback_pixels must be > 0 when primary scene has cloud-masked region filled by secondary scene: {prov_fb}"
    );

    drop(service);

    // 6. Verify SQLite file contains zero SAS secrets & reopen preserves CoreRestore tasks
    let db_bytes = fs::read(&temp.database).expect("read sqlite file");
    let db_lossy = String::from_utf8_lossy(&db_bytes);
    assert!(!db_lossy.contains("sig="));
    assert!(!db_lossy.contains("SECRET_TOKEN"));
    assert!(!db_lossy.contains("resolved_href"));

    let reopened = ApplicationService::with_dem_operation_port(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(RealOpticalGisPort { engine }),
    );
    let restored = reopened
        .core_restore(false)
        .expect("core_restore after reopen");
    let restored_json = serde_json::to_value(&restored.core).expect("restored json");
    let tasks = restored_json
        .get("tasks")
        .and_then(Value::as_array)
        .expect("restored tasks");
    assert!(tasks
        .iter()
        .any(|t| t.get("task_id").and_then(Value::as_str) == Some("task-p0-clip")));
    assert!(tasks
        .iter()
        .any(|t| t.get("task_id").and_then(Value::as_str) == Some("task-p1-mosaic")));
}

#[test]
#[ignore = "requires optional native GIS runtime (run scripts/run_native_gis_tests.sh with INSAR_GIS_RUNTIME_ROOT)"]
fn optical_derived_rejects_signed_asset_href_before_persistence() {
    let temp = TempWorkspace::new("signed-reject");
    let scene_bbox = [116.89, 31.72, 117.11, 31.91];
    let aoi_bbox = [116.95, 31.78, 117.05, 31.85];

    let mut query = Map::new();
    query.insert(
        "logical_dataset".to_owned(),
        Value::String("optical.sentinel-2.l2a".to_owned()),
    );
    query.insert(
        "provider_id".to_owned(),
        Value::String("microsoft.planetary-computer".to_owned()),
    );
    query.insert(
        "native_collection_id".to_owned(),
        Value::String("sentinel-2-l2a".to_owned()),
    );

    let mut signed_item = make_s2_inline_item(
        "s2-item-signed-secret",
        "S2A_MSIL2A_SIGNED_SECRET",
        "2026-05-01T02:55:49Z",
        4.0,
        scene_bbox,
        (
            &temp.fixtures_dir.join("unused_b04.tif"),
            &temp.fixtures_dir.join("unused_b03.tif"),
            None,
        ),
    );
    signed_item.metadata.insert(
        "assets".to_owned(),
        json!([
            {
                "asset_key": "B04",
                "stable_href": "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/tile_B04.tif?sv=2021-06-08&sig=SECRET_TOKEN_XYZ",
                "role": "data"
            },
            {
                "asset_key": "B03",
                "stable_href": "https://sentinel2l2a01.blob.core.windows.net/sentinel2-l2/tile_B03.tif",
                "role": "data"
            }
        ]),
    );

    {
        let mut store = insar_store::TaskStore::open(&temp.database).expect("open store");
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: true,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: "signed-src-batch".to_owned(),
                    provider_id: "microsoft.planetary-computer".to_owned(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    aoi_name: "HefeiTest".to_owned(),
                    aoi_geojson: Some(polygon_geojson_from_bbox(aoi_bbox)),
                    query,
                    search_stats: None,
                    candidates: vec![signed_item],
                },
                workbench: SearchWorkbenchState {
                    batch_id: "signed-src-batch".to_owned(),
                    checked_item_ids: vec!["s2-item-signed-secret".to_owned()],
                    page_index: 0,
                    page_size: 20,
                    revision: 1,
                },
            })
            .expect("save batch");
    }

    let service = ApplicationService::with_dem_operation_port(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(RealOpticalGisPort {
            engine: load_real_gis_engine(),
        }),
    );

    let signed_reason = invalid_reason(
        service
            .create_optical_derived_task(OpticalDerivedCreateTaskRequest {
                request_id: "req-signed-reject".to_owned(),
                task_id: "task-signed-reject".to_owned(),
                source_batch_id: "signed-src-batch".to_owned(),
                expected_workbench_revision: 1,
                item_ids: vec!["s2-item-signed-secret".to_owned()],
                mode: OpticalDerivedMode::SingleSceneClip,
                aoi_bbox_wgs84: None,
                aoi_rings_wgs84: None,
                band_order: vec!["B04".to_owned(), "B03".to_owned()],
                target_crs: None,
                target_resolution_m: None,
                resampling: OpticalResampling::Bilinear,
                cloud_mask: OpticalCloudMaskPolicy::None,
                mosaic_strategy: OpticalMosaicStrategy::QualityFirst,
                max_temporal_span_days: None,
                overwrite: true,
                write_lineage_sidecar: true,
                write_provenance_sidecars: true,
                output_root: temp.output_dir.clone(),
                concurrency: 1,
                created_at: "2026-05-06T00:01:30Z".to_owned(),
            })
            .expect_err("signed SAS URL must be refused"),
    );
    assert!(
        signed_reason.contains("OPTICAL_SOURCE_REJECTED") || signed_reason.contains("SECRET"),
        "unexpected signed reason: {signed_reason}"
    );
}
