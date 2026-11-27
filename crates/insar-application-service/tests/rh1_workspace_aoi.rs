use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use insar_application_service::{
    ApplicationBindRegionAoiRequest, ApplicationService, ApplicationServiceConfig,
    ApplicationServiceError, WorkspaceOpenRequest, WorkspacePrepareLocationRequest,
};
use serde_json::{json, Map, Value};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct TempStore {
    root: PathBuf,
    database: PathBuf,
}

impl TempStore {
    fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-rh1-workspace-aoi-{}-{id}",
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

fn geometry(west: f64) -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[west, 30.0], [west + 1.0, 30.0], [west + 1.0, 31.0], [west, 31.0], [west, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn bind(region_id: &str, west: f64) -> ApplicationBindRegionAoiRequest {
    ApplicationBindRegionAoiRequest {
        request_id: format!("bind-{region_id}"),
        region_id: region_id.to_owned(),
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: geometry(west),
        bbox: vec![west, 30.0, west + 1.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: region_id.to_owned(),
        bound_at: "2026-08-11T00:00:00Z".to_owned(),
    }
}

#[test]
fn workspace_open_and_aoi_are_keyed_by_stable_region_id() {
    let temp = TempStore::new();
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));

    let context = service
        .workspace_open(&WorkspaceOpenRequest {
            request_id: "workspace-open-1".to_owned(),
            workspace_id: "ws-rh1".to_owned(),
            workspace_root: temp.root.clone(),
            workspace_name: "RH1 workspace".to_owned(),
            expected_revision: None,
        })
        .unwrap();
    assert_eq!(context.workspace_id, "ws-rh1");
    assert!(context.revision >= 1);

    let project = service
        .project_create(&insar_application_service::WorkspaceNamedNodeRequest {
            request_id: "project-create-1".to_owned(),
            parent_id: "ws-rh1".to_owned(),
            expected_revision: context.revision,
            name: "RH1 project".to_owned(),
        })
        .unwrap();
    assert!(project.project_id.is_some());
    let project_id = project.project_id.clone().unwrap();
    let first_region = service
        .region_create(&insar_application_service::WorkspaceNamedNodeRequest {
            request_id: "region-create-a".to_owned(),
            parent_id: project_id.clone(),
            expected_revision: project.revision,
            name: "Region A".to_owned(),
        })
        .unwrap();
    let first_region_id = first_region.region_id.clone().unwrap();
    let second_region = service
        .region_create(&insar_application_service::WorkspaceNamedNodeRequest {
            request_id: "region-create-b".to_owned(),
            parent_id: project_id.clone(),
            expected_revision: first_region.revision,
            name: "Region B".to_owned(),
        })
        .unwrap();
    let second_region_id = second_region.region_id.clone().unwrap();
    assert_ne!(first_region_id, second_region_id);

    let stale = service
        .region_select(&insar_application_service::WorkspaceSelectNodeRequest {
            request_id: "region-select-stale".to_owned(),
            parent_id: project_id.clone(),
            expected_revision: context.revision,
            node_id: first_region_id.clone(),
        })
        .unwrap_err();
    assert!(matches!(
        stale,
        ApplicationServiceError::WorkspaceRevisionConflict { .. }
    ));

    // Region B is active after the second create; a bind addressed to Region A
    // must not silently switch the active region or write under the wrong UI
    // context.
    assert!(service
        .bind_region_aoi(&bind(&first_region_id, 100.0))
        .is_err());

    let first = service
        .region_select(&insar_application_service::WorkspaceSelectNodeRequest {
            request_id: "region-select-a".to_owned(),
            parent_id: project_id.clone(),
            expected_revision: second_region.revision,
            node_id: first_region_id.clone(),
        })
        .unwrap();
    assert_eq!(first.region_id, Some(first_region_id.clone()));
    let first = service
        .bind_region_aoi(&bind(&first_region_id, 100.0))
        .unwrap();
    let second = service
        .region_select(&insar_application_service::WorkspaceSelectNodeRequest {
            request_id: "region-select-b".to_owned(),
            parent_id: project_id,
            expected_revision: first.core.workspace_context.as_ref().unwrap().revision,
            node_id: second_region_id.clone(),
        })
        .unwrap();
    assert_eq!(second.region_id, Some(second_region_id.clone()));
    let second = service
        .bind_region_aoi(&bind(&second_region_id, 110.0))
        .unwrap();
    assert_eq!(first.core.region_aoi.unwrap().region_id, first_region_id);
    assert_eq!(second.core.region_aoi.unwrap().region_id, second_region_id);

    let restored = service.restore(false).unwrap();
    let region_ids: Vec<_> = restored
        .region_aois
        .iter()
        .map(|aoi| aoi.region_id.as_str())
        .collect();
    assert_eq!(
        region_ids,
        vec![first_region_id.as_str(), second_region_id.as_str()]
    );
    assert_eq!(restored.workspace_context.unwrap().workspace_id, "ws-rh1");

    service.shutdown().unwrap();
    let reopened =
        ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    let restored = reopened.restore(false).unwrap();
    assert_eq!(restored.region_aois.len(), 2);
    assert_eq!(
        restored.workspace_context.unwrap().region_id,
        Some(second_region_id)
    );
    reopened.shutdown().unwrap();
}

#[test]
fn workspace_prepare_location_is_application_owned_and_creates_the_requested_root() {
    let temp = TempStore::new();
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&temp.database));
    let nested = temp.root.join("prepared").join("workspace");

    let prepared = service
        .workspace_prepare_location(&WorkspacePrepareLocationRequest {
            request_id: "workspace-prepare-1".to_owned(),
            path: nested.clone(),
        })
        .unwrap();

    assert_eq!(prepared.path, nested);
    assert!(prepared.path.is_dir());
    assert!(service
        .workspace_prepare_location(&WorkspacePrepareLocationRequest {
            request_id: "workspace-prepare-relative".to_owned(),
            path: PathBuf::from("relative-workspace"),
        })
        .is_err());
    service.shutdown().unwrap();
}
