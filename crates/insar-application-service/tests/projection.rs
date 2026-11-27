use insar_application_service::{
    canonical_restore_value, canonical_snapshot_value, ApplicationService,
    ApplicationServiceConfig, ReadonlyServiceSnapshot, ServiceStatus,
};
use insar_core::StoreRestoreResponse;
use serde_json::json;

#[test]
fn canonical_restore_has_no_transport_envelope_fields() {
    let restore = StoreRestoreResponse {
        workbench_batch: None,
        workbench: None,
        tasks: vec![],
        runtimes: vec![],
        region_aoi: None,
        region_aois: vec![],
        workspace_context: None,
    };
    let value = canonical_restore_value(&restore).unwrap();
    assert_eq!(value["tasks"], json!([]));
    assert!(!value.contains_key("request_id"));
    assert!(!value.contains_key("protocol_minor"));
    assert!(!value.contains_key("terminal"));
}

#[test]
fn canonical_snapshot_round_trips_without_transport_specific_fields() {
    let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
    let snapshot = ReadonlyServiceSnapshot {
        readiness: service.startup().unwrap(),
        restore: StoreRestoreResponse {
            workbench_batch: None,
            workbench: None,
            tasks: vec![],
            runtimes: vec![],
            region_aoi: None,
            region_aois: vec![],
            workspace_context: None,
        },
    };
    assert_eq!(snapshot.readiness.status, ServiceStatus::LivenessOnly);
    let value = canonical_snapshot_value(&snapshot).unwrap();
    assert_eq!(value["readiness"]["status"], "liveness_only");
    assert!(value.get("request_id").is_none());
    assert!(value.get("terminal").is_none());
    let decoded: ReadonlyServiceSnapshot = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, snapshot);
}
