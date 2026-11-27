use insar_application_service::{
    ApplicationAoiState, ApplicationSarSearchAndFreezeRequest, ApplicationService,
    ApplicationServiceConfig, ApplicationSetWorkingAoiRequest,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STORE: AtomicU64 = AtomicU64::new(1);

struct TempStore {
    root: PathBuf,
    database: PathBuf,
}

impl TempStore {
    fn new(label: &str) -> Self {
        let id = NEXT_STORE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-g1-working-aoi-{label}-{}-{id}",
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

#[test]
fn standalone_working_aoi_is_store_owned_and_survives_restart_without_saved_hierarchy() {
    let temp = TempStore::new("restart");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    assert_eq!(service.working_aoi_state(), ApplicationAoiState::NoAoi);

    let working = service
        .set_working_aoi(&ApplicationSetWorkingAoiRequest {
            source_kind: "manual_polygon".to_owned(),
            geometry: json!({
                "type": "Polygon",
                "coordinates": [[[110.0, 30.0], [111.0, 30.0], [111.0, 31.0], [110.0, 30.0]]]
            })
            .as_object()
            .unwrap()
            .clone(),
            bbox: vec![110.0, 30.0, 111.0, 31.0],
            crs: "EPSG:4326".to_owned(),
            display_name: "绘制范围".to_owned(),
        })
        .expect("working AOI can be selected without a Region");

    assert_eq!(working.state, ApplicationAoiState::Working);
    assert_eq!(service.working_aoi_state(), ApplicationAoiState::Working);
    assert_eq!(
        service.working_aoi().unwrap().unwrap().display_name,
        "绘制范围"
    );

    service.shutdown().unwrap();

    let reopened =
        ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    assert_eq!(reopened.working_aoi_state(), ApplicationAoiState::Working);
    assert_eq!(
        reopened.working_aoi().unwrap().unwrap().display_name,
        "绘制范围"
    );
    reopened.clear_working_aoi().expect("clear working AOI");
    assert_eq!(reopened.working_aoi_state(), ApplicationAoiState::NoAoi);
    reopened.shutdown().unwrap();

    let cleared = ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    assert_eq!(cleared.working_aoi_state(), ApplicationAoiState::NoAoi);
    assert_eq!(cleared.working_aoi().unwrap(), None);
    cleared.shutdown().unwrap();
}

#[test]
fn working_aoi_rejects_feature_wrappers_before_the_search_bridge() {
    let temp = TempStore::new("feature");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    let result = service.set_working_aoi(&ApplicationSetWorkingAoiRequest {
        source_kind: "administrative_boundary".to_owned(),
        geometry: json!({
            "type": "Feature",
            "properties": {"name": "武汉市"},
            "geometry": {
                "type": "Polygon",
                "coordinates": [[[113.9, 30.4], [114.5, 30.4], [114.5, 30.8], [113.9, 30.4]]]
            }
        })
        .as_object()
        .unwrap()
        .clone(),
        bbox: vec![113.9, 30.4, 114.5, 30.8],
        crs: "EPSG:4326".to_owned(),
        display_name: "武汉市".to_owned(),
    });

    let error = result.expect_err("Feature wrappers must not become a working search AOI");
    assert_eq!(error.code(), "V3_APPLICATION_INVALID_REQUEST");
    assert_eq!(service.working_aoi_state(), ApplicationAoiState::NoAoi);
    service.shutdown().unwrap();
}

#[test]
fn sar_search_freeze_rejects_feature_wrappers_instead_of_dropping_the_aoi() {
    let feature = json!({
        "type": "Feature",
        "properties": {"name": "武汉市"},
        "geometry": {
            "type": "Polygon",
            "coordinates": [[[113.9, 30.4], [114.5, 30.4], [114.5, 30.8], [113.9, 30.4]]]
        }
    });
    let request: ApplicationSarSearchAndFreezeRequest = serde_json::from_value(json!({
        "request_id": "admin-aoi-search",
        "search": {"query": {
            "provider": "ASF",
            "dataset": "SENTINEL-1",
            "aoi_geojson": feature,
            "max_results": 20
        }},
        "batch_id": "admin-aoi-batch",
        "created_at": "2026-08-18T00:00:00Z",
        "aoi_name": "武汉市",
        "aoi_geojson": feature,
        "page_size": 24
    }))
    .expect("fixture has the public request shape");

    let error = request
        .validate()
        .expect_err("Feature wrappers must fail before Provider query planning");
    assert!(
        error.contains("Polygon or MultiPolygon"),
        "unexpected error: {error}"
    );
}
