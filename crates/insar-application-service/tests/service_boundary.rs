use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use insar_application_service::{
    require_store_readiness_capability, ApplicationService, ApplicationServiceConfig,
    ServiceStatus, StoreTaskEventPageRequest, StoreTaskOutputDirectoryRequest,
    STORE_STARTUP_READINESS_CAPABILITY,
};
use insar_store::STORE_SCHEMA_VERSION;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn temp_store_path(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "insar-application-service-{label}-{}-{id}.sqlite3",
        std::process::id()
    ))
}

fn cleanup(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("sqlite3-wal"));
    let _ = fs::remove_file(path.with_extension("sqlite3-shm"));
}

#[test]
fn production_startup_keeps_independent_cdse_and_bhoonidhi_routes_offline() {
    let path = temp_store_path("mixed-network-routes");
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&path));
    let readiness = service
        .startup()
        .expect("production worker with independent routes");
    assert_eq!(readiness.status, ServiceStatus::StoreReady);
    cleanup(&path);
}

#[test]
fn configured_store_is_opened_and_health_checked_before_ready() {
    let path = temp_store_path("ready");
    let service = ApplicationService::new(ApplicationServiceConfig::store(&path));
    assert_eq!(service.status().unwrap(), ServiceStatus::Created);
    let readiness = service.startup().expect("valid Store starts");
    assert_eq!(readiness.status, ServiceStatus::StoreReady);
    assert_eq!(readiness.store_schema_version, Some(STORE_SCHEMA_VERSION));
    assert_eq!(service.startup().unwrap(), readiness);
    assert_eq!(service.status().unwrap(), ServiceStatus::StoreReady);
    cleanup(&path);
}

#[test]
fn empty_store_returns_honest_empty_restore() {
    let path = temp_store_path("empty");
    let service = ApplicationService::new(ApplicationServiceConfig::store(&path));
    service.startup().unwrap();
    let response = service.restore(false).expect("empty Store restores");
    assert!(response.workbench.is_none());
    assert!(response.workbench_batch.is_none());
    assert!(response.tasks.is_empty());
    assert!(response.runtimes.is_empty());
    cleanup(&path);
}

#[test]
fn storeless_liveness_does_not_claim_store_ready() {
    let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
    let readiness = service.startup().expect("liveness starts without a Store");
    assert_eq!(readiness.status, ServiceStatus::LivenessOnly);
    let error = service.restore(false).unwrap_err();
    assert_eq!(error.code(), "V3_STORE_NOT_READY");
}

#[test]
fn corrupt_store_is_a_typed_startup_failure() {
    let path = temp_store_path("corrupt");
    fs::write(&path, b"not a sqlite database").unwrap();
    let service = ApplicationService::new(ApplicationServiceConfig::store(&path));
    let error = service.startup().unwrap_err();
    assert_eq!(error.code(), "V3_STORE_DATABASE_ERROR");
    cleanup(&path);
}

#[test]
fn existing_store_rejects_missing_path_without_creating_database_or_parent() {
    let root = std::env::temp_dir().join(format!(
        "insar-application-service-missing-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let path = root.join("nested").join("tasks.sqlite3");
    let service = ApplicationService::new(ApplicationServiceConfig::existing_store(&path));

    let error = service.startup().unwrap_err();

    assert_eq!(error.code(), "V3_STORE_MISSING");
    assert!(!path.exists());
    assert!(!root.exists());
}

#[test]
fn existing_store_opens_a_preexisting_database_and_keeps_migration_compatibility() {
    let path = temp_store_path("existing");
    let _created = insar_store::TaskStore::open(&path).expect("seed Store opens");
    drop(_created);

    let service = ApplicationService::new(ApplicationServiceConfig::existing_store(&path));
    let readiness = service.startup().expect("existing Store starts");

    assert_eq!(readiness.status, ServiceStatus::StoreReady);
    assert_eq!(readiness.store_schema_version, Some(STORE_SCHEMA_VERSION));
    cleanup(&path);
}

#[test]
fn required_store_never_accepts_liveness_only_capability() {
    let error = require_store_readiness_capability(&["legacy_liveness"], true).unwrap_err();
    assert_eq!(error.code(), "V3_STORE_READINESS_UNSUPPORTED");
    assert!(
        require_store_readiness_capability(&[STORE_STARTUP_READINESS_CAPABILITY], true).is_ok()
    );
}

#[test]
fn task_event_and_output_authorities_cross_only_narrow_service_routes() {
    let path = temp_store_path("task-detail-authority");
    let service = ApplicationService::new(ApplicationServiceConfig::store(&path));
    service.startup().expect("store starts");

    let events = service
        .task_event_page(&StoreTaskEventPageRequest {
            task_id: "missing-task".to_owned(),
            expected_revision: 0,
            before_sequence: None,
            page_size: 40,
        })
        .expect_err("missing task event authority");
    assert_eq!(events.code(), "V3_STORE_NOT_FOUND");
    let output = service
        .resolve_task_output_directory(&StoreTaskOutputDirectoryRequest {
            task_id: "missing-task".to_owned(),
            expected_revision: 0,
        })
        .expect_err("missing task output authority");
    assert_eq!(output.code(), "V3_STORE_NOT_FOUND");
    cleanup(&path);
}
