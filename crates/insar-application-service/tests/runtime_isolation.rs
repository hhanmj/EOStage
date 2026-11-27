use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use insar_application_service::{
    canonical_restore_value, ApplicationService, ApplicationServiceConfig, ApplicationServiceError,
};
use insar_core::{
    DownloadLayout, FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreAppendTaskRequest,
    StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreDeleteHistoryRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, StoreUpdateTaskViewRequest,
    StoreUpdateWorkbenchRequest, TaskStateCommandKind,
};
use insar_store::TaskStore;
use serde_json::{Map, Value};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);

impl TempStore {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "insar-application-service-runtime-isolation-{label}-{}-{id}.sqlite3",
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

fn item(item_id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "asf.catalog".to_owned(),
        display_name: format!("{item_id}.zip"),
        acquisition_datetime: Some("2026-07-25T00:00:00Z".to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn aoi() -> Map<String, Value> {
    serde_json::json!({
        "type": "Polygon",
        "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn create_request(task_id: &str) -> StoreCreateTaskRequest {
    StoreCreateTaskRequest {
        task_id: task_id.to_owned(),
        batch_id: "isolation-batch".to_owned(),
        selected_item_ids: vec!["item-one".to_owned()],
        checked_item_ids: vec!["item-one".to_owned()],
        page_index: 0,
        page_size: 20,
        output_root: PathBuf::from(format!("isolation-output-{task_id}")),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-07-25T00:00:00Z".to_owned(),
    }
}

fn cancel_task(store: &mut TaskStore, task_id: &str, event_id: &str) {
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelItems,
                item_ids: vec!["item-one".to_owned()],
            },
            event_id: event_id.to_owned(),
            occurred_at: "2026-07-25T00:00:01Z".to_owned(),
        })
        .unwrap();
}

fn seed_store(path: &PathBuf) {
    let mut store = TaskStore::open(path).unwrap();
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "isolation-batch".to_owned(),
        provider_id: "asf.catalog".to_owned(),
        created_at: "2026-07-25T00:00:00Z".to_owned(),
        aoi_name: "Isolation AOI".to_owned(),
        aoi_geojson: Some(aoi()),
        query: Map::new(),
        search_stats: None,
        candidates: vec![item("item-one"), item("item-two")],
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: SearchWorkbenchState {
                batch_id: "isolation-batch".to_owned(),
                checked_item_ids: vec!["item-one".to_owned()],
                page_index: 0,
                page_size: 20,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    store.create_task(&create_request("active-task")).unwrap();
    store
        .create_task(&create_request("archive-target"))
        .unwrap();
    cancel_task(&mut store, "archive-target", "archive-target-cancel");
    store
        .create_task(&create_request("history-target"))
        .unwrap();
    cancel_task(&mut store, "history-target", "history-target-cancel");
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "history-target".to_owned(),
            expected_revision: 1,
            archived_at: "2026-07-25T00:00:02Z".to_owned(),
        })
        .unwrap();
}

fn readonly_service(label: &str) -> (TempStore, ApplicationService) {
    let database = TempStore::new(label);
    seed_store(&database.0);
    let service = ApplicationService::new(ApplicationServiceConfig::existing_store(&database.0));
    service.startup().unwrap();
    (database, service)
}

fn restore_projections(service: &ApplicationService) -> (Map<String, Value>, Map<String, Value>) {
    (
        canonical_restore_value(&service.restore(false).unwrap()).unwrap(),
        canonical_restore_value(&service.restore(true).unwrap()).unwrap(),
    )
}

fn assert_readonly_rejection<T: std::fmt::Debug>(
    service: &ApplicationService,
    operation: impl FnOnce(&ApplicationService) -> Result<T, ApplicationServiceError>,
) {
    let before = restore_projections(service);
    let error = operation(service).unwrap_err();
    assert_eq!(error.code(), "V3_APPLICATION_BUSINESS_RUNTIME_UNAVAILABLE");
    assert_eq!(restore_projections(service), before);
}

#[test]
fn readonly_service_rejects_workbench_mutation_before_store_write() {
    let (_database, service) = readonly_service("workbench");
    assert_readonly_rejection(&service, |service| {
        service.update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: SearchWorkbenchState {
                batch_id: "isolation-batch".to_owned(),
                checked_item_ids: vec!["item-two".to_owned()],
                page_index: 1,
                page_size: 20,
                revision: 1,
            },
            expected_revision: 0,
            updated_at: "2026-07-25T00:00:03Z".to_owned(),
        })
    });
    service.shutdown().unwrap();
}

#[test]
fn readonly_service_rejects_task_creation_before_store_write() {
    let (_database, service) = readonly_service("create");
    assert_readonly_rejection(&service, |service| {
        service.create_download_task_legacy(&create_request("new-task"))
    });
    service.shutdown().unwrap();
}

#[test]
fn readonly_service_rejects_task_append_before_store_write() {
    let (_database, service) = readonly_service("append");
    assert_readonly_rejection(&service, |service| {
        service.append_download_task(&StoreAppendTaskRequest {
            task_id: "active-task".to_owned(),
            expected_revision: 0,
            item_ids: vec!["item-two".to_owned()],
            event_id: "readonly-append".to_owned(),
            occurred_at: "2026-07-25T00:00:03Z".to_owned(),
        })
    });
    service.shutdown().unwrap();
}

#[test]
fn readonly_service_rejects_task_update_before_store_write() {
    let (_database, service) = readonly_service("update");
    assert_readonly_rejection(&service, |service| {
        service.update_download_task(&StoreUpdateTaskRequest {
            task_id: "active-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "readonly-pause".to_owned(),
            occurred_at: "2026-07-25T00:00:03Z".to_owned(),
        })
    });
    service.shutdown().unwrap();
}

#[test]
fn readonly_service_rejects_task_view_mutation_before_store_write() {
    let (_database, service) = readonly_service("view");
    assert_readonly_rejection(&service, |service| {
        service.update_download_task_view(&StoreUpdateTaskViewRequest {
            task_id: "active-task".to_owned(),
            checked_item_ids: vec!["item-one".to_owned()],
            page_index: 1,
            page_size: 20,
            updated_at: "2026-07-25T00:00:03Z".to_owned(),
        })
    });
    service.shutdown().unwrap();
}

#[test]
fn readonly_service_rejects_archive_before_store_write() {
    let (_database, service) = readonly_service("archive");
    assert_readonly_rejection(&service, |service| {
        service.archive_download_task(&StoreArchiveTaskRequest {
            task_id: "archive-target".to_owned(),
            expected_revision: 1,
            archived_at: "2026-07-25T00:00:03Z".to_owned(),
        })
    });
    service.shutdown().unwrap();
}

#[test]
fn readonly_service_rejects_history_deletion_before_store_write() {
    let (_database, service) = readonly_service("history");
    assert_readonly_rejection(&service, |service| {
        service.delete_download_history(&StoreDeleteHistoryRequest {
            task_id: "history-target".to_owned(),
            deleted_at: "2026-07-25T00:00:03Z".to_owned(),
        })
    });
    service.shutdown().unwrap();
}

#[test]
fn readonly_service_rejects_nonmutating_business_operations() {
    let (_database, service) = readonly_service("business-reads");
    for error in [
        service.provider_capabilities().unwrap_err(),
        service.cancel_request("readonly-cancel").unwrap_err(),
        service.download_snapshot("active-task").unwrap_err(),
    ] {
        assert_eq!(error.code(), "V3_APPLICATION_BUSINESS_RUNTIME_UNAVAILABLE");
    }
    service.shutdown().unwrap();
}

#[test]
fn shutdown_rejects_business_mutations_without_restarting_the_store() {
    let (database, service) = readonly_service("shutdown");
    let before = restore_projections(&service);
    service.shutdown().unwrap();

    let error = service
        .update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: SearchWorkbenchState {
                batch_id: "isolation-batch".to_owned(),
                checked_item_ids: vec!["item-two".to_owned()],
                page_index: 1,
                page_size: 20,
                revision: 1,
            },
            expected_revision: 0,
            updated_at: "2026-07-25T00:00:03Z".to_owned(),
        })
        .unwrap_err();
    assert_eq!(error.code(), "V3_APPLICATION_SHUTTING_DOWN");

    let store = TaskStore::open_existing(&database.0).unwrap();
    let after = (
        canonical_restore_value(
            &store
                .restore(&insar_core::StoreRestoreRequest {
                    include_terminal: false,
                })
                .unwrap(),
        )
        .unwrap(),
        canonical_restore_value(
            &store
                .restore(&insar_core::StoreRestoreRequest {
                    include_terminal: true,
                })
                .unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(after, before);
}
