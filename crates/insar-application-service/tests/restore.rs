use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use insar_application_service::{
    compare_retry_fallback, ApplicationService, ApplicationServiceConfig, ServiceStatus,
    UnifiedCreateTaskOptions,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, DownloadItemProgress, DownloadItemResult,
    DownloadLayout, DownloadOutcome, FrozenSearchBatch, ProviderResult, SearchWorkbenchState,
    SnapshotItem, StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreSearchBatchRequest,
    StoreTaskCommand, StoreTaskPageRequest, StoreUpdateTaskRequest, TaskOperationDescriptor,
    TaskStateCommandKind,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::LiveAsfSearch;
use insar_store::{StoreTaskHistoryPageRequest, TaskStore};
use serde_json::Map;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct NoSearch;
impl LiveAsfSearch for NoSearch {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        panic!("task fixture must never search ASF")
    }
}
struct WaitForCancelWorker;
impl DownloadWorker for WaitForCancelWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        while !control.is_cancelled() {
            thread::sleep(Duration::from_millis(5));
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root.join(work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-21T02:00:00Z".to_owned(),
        }
    }
}

fn offline_task_service(path: &PathBuf) -> ApplicationService {
    ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(path),
        Arc::new(NoSearch),
        Arc::new(WaitForCancelWorker),
    )
}

fn path(label: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "insar-application-restore-{label}-{}-{id}.sqlite3",
        std::process::id()
    ))
}

fn cleanup(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

fn seed_store(path: &PathBuf) {
    let mut store = TaskStore::open(path).expect("seed Store opens");
    let batch = FrozenSearchBatch {
        schema_version: 1,
        batch_id: "restore-batch".to_owned(),
        provider_id: "fixture.provider".to_owned(),
        created_at: "2026-07-21T00:00:00Z".to_owned(),
        aoi_name: "fixture AOI".to_owned(),
        aoi_geojson: serde_json::from_value(serde_json::json!({
            "type": "Polygon",
            "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
        }))
        .unwrap(),
        query: Map::new(),
        search_stats: None,
        candidates: vec![SnapshotItem {
            item_id: "fixture-item".to_owned(),
            provider_id: "fixture.provider".to_owned(),
            display_name: "Fixture item".to_owned(),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        }],
    };
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: SearchWorkbenchState {
                batch_id: "restore-batch".to_owned(),
                checked_item_ids: vec!["fixture-item".to_owned()],
                page_index: 2,
                page_size: 25,
                revision: 0,
            },
            activate_workbench: true,
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: "restore-task".to_owned(),
            batch_id: "restore-batch".to_owned(),
            selected_item_ids: vec!["fixture-item".to_owned()],
            checked_item_ids: vec!["fixture-item".to_owned()],
            page_index: 2,
            page_size: 25,
            output_root: PathBuf::from("fixture-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-21T00:01:00Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "restore-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "restore-pause".to_owned(),
            occurred_at: "2026-07-21T00:02:00Z".to_owned(),
        })
        .unwrap();
}

#[test]
fn restart_restore_preserves_frozen_aoi_selection_and_paused_runtime() {
    let database = path("paused");
    seed_store(&database);

    let service = ApplicationService::new(ApplicationServiceConfig::store(&database));
    let first_ready = service.startup().unwrap();
    assert_eq!(first_ready.status, ServiceStatus::StoreReady);
    let first = service.restore(false).unwrap();
    assert_eq!(first.workbench.as_ref().unwrap().page_index, 2);
    assert_eq!(first.tasks[0].aoi_name, "fixture AOI");
    assert_eq!(
        first.runtimes[0].status,
        insar_core::TaskLifecycleStatus::Paused
    );
    assert_eq!(first.runtimes[0].revision, 1);
    service.shutdown().unwrap();

    let restarted = ApplicationService::new(ApplicationServiceConfig::store(&database));
    restarted.startup().unwrap();
    let second = restarted.restore(false).unwrap();
    assert_eq!(second, first);
    cleanup(&database);
}

#[test]
fn source_views_restore_per_resource_after_legacy_workbench_switch_without_new_table() {
    let database = path("per-source-views");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        let mut first = store.load_search_batch("restore-batch").unwrap().unwrap();
        first.batch_id = "landsat-batch".to_owned();
        first.provider_id = "microsoft.planetary-computer".to_owned();
        first.query=serde_json::json!({"logical_dataset":"optical.landsat.c2.l2","native_collection_id":"landsat-c2-l2"}).as_object().unwrap().clone();
        first.candidates[0].provider_id = first.provider_id.clone();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch: first,
                workbench: SearchWorkbenchState {
                    batch_id: "landsat-batch".to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                activate_workbench: true,
            })
            .unwrap();
    }
    let service = ApplicationService::new(ApplicationServiceConfig::store(&database));
    let view = service
        .source_views()
        .unwrap()
        .into_iter()
        .find(|view| view.resource_id == "science.landsat.c2-l2.v1")
        .expect("landset source view survives restart");
    assert_eq!(view.batch_id.as_deref(), Some("landsat-batch"));
    assert_eq!(view.item_count, 1);
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn unified_create_task_from_frozen_batch_reuses_active_product_after_restart() {
    let database = path("unified-task");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        let mut batch = store.load_search_batch("restore-batch").unwrap().unwrap();
        batch.batch_id = "unified-batch".to_owned();
        batch.provider_id = "microsoft.planetary-computer".to_owned();
        batch.query = serde_json::json!({"logical_dataset":"optical.landsat.c2.l2", "provider_id":"microsoft.planetary-computer", "native_collection_id":"landsat-c2-l2"}).as_object().unwrap().clone();
        batch.candidates[0].provider_id = batch.provider_id.clone();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch,
                workbench: SearchWorkbenchState {
                    batch_id: "unified-batch".to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 25,
                    revision: 0,
                },
                activate_workbench: false,
            })
            .unwrap();
    }
    let service = offline_task_service(&database);
    let options = UnifiedCreateTaskOptions {
        output_root: PathBuf::from("fixture-output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        include_precise_orbit: None,
    };
    let first = service
        .create_task(
            "unified-batch".to_owned(),
            0,
            vec!["fixture-item".to_owned()],
            options.clone(),
        )
        .expect("task created");
    assert!(first.created);
    assert_eq!(first.status, insar_core::TaskLifecycleStatus::Queued);
    let second = service
        .create_task(
            "unified-batch".to_owned(),
            0,
            vec!["fixture-item".to_owned()],
            options.clone(),
        )
        .expect("same product reused");
    assert!(!second.created);
    assert_eq!(first.task_id, second.task_id);
    service.shutdown().unwrap();
    {
        let mut store = TaskStore::open(&database).unwrap();
        let record = store.load_task_record(&first.task_id).unwrap().unwrap();
        if record.runtime.status == insar_core::TaskLifecycleStatus::Failed {
            store
                .update_task(&StoreUpdateTaskRequest {
                    task_id: first.task_id.clone(),
                    expected_revision: record.runtime.revision,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::RetryTask,
                        item_ids: vec![],
                    },
                    event_id: "retry-before-restart".to_owned(),
                    occurred_at: "2026-07-21T02:00:00Z".to_owned(),
                })
                .expect("restore active task before dedup assertion");
        }
        assert!(!matches!(
            store
                .load_task_record(&first.task_id)
                .unwrap()
                .unwrap()
                .runtime
                .status,
            insar_core::TaskLifecycleStatus::Failed
                | insar_core::TaskLifecycleStatus::Completed
                | insar_core::TaskLifecycleStatus::Cancelled
        ));
    }
    let reopened = offline_task_service(&database);
    let third = reopened
        .create_task(
            "unified-batch".to_owned(),
            0,
            vec!["fixture-item".to_owned()],
            options,
        )
        .expect("restart reuses identity");
    assert!(!third.created);
    assert_eq!(third.task_id, first.task_id);
    reopened.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn application_service_exposes_only_the_bounded_task_page() {
    let database = path("task-page");
    seed_store(&database);

    let service = ApplicationService::new(ApplicationServiceConfig::store(&database));
    service.startup().unwrap();
    let page = service
        .task_page(&StoreTaskPageRequest {
            task_id: "restore-task".to_owned(),
            expected_revision: 1,
            page_index: 0,
            page_size: 1,
        })
        .unwrap();
    assert_eq!(page.task_id, "restore-task");
    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].item.item_id, "fixture-item");
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn application_service_exposes_one_scalar_keyset_history_page() {
    let database = path("task-history-page");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        let completed = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: 1,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkCompleted,
                    item_ids: vec!["fixture-item".to_owned()],
                },
                event_id: "history-complete".to_owned(),
                occurred_at: "2026-07-21T00:03:00Z".to_owned(),
            })
            .unwrap();
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: completed.runtime.revision,
                archived_at: "2026-07-21T00:04:00Z".to_owned(),
            })
            .unwrap();
    }

    let service = ApplicationService::new(ApplicationServiceConfig::store(&database));
    service.startup().unwrap();
    let page = service
        .task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 40,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .unwrap();
    assert_eq!(page.page_size, 40);
    assert!(!page.has_more);
    assert!(page.next_cursor.is_none());
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].archived_at, "2026-07-21T00:04:00Z");
    assert_eq!(page.items[0].task.task_id, "restore-task");
    assert_eq!(page.items[0].task.revision, 2);
    let serialized = serde_json::to_value(page).unwrap();
    assert!(serialized.pointer("/items/0/snapshot_json").is_none());
    assert!(serialized.pointer("/items/0/state_json").is_none());
    assert!(serialized.pointer("/items/0/selected_item_ids").is_none());
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn legacy_core_restore_does_not_archive_failed_task() {
    let database = path("failed-auto-history");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: 1,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec!["fixture-item".to_owned()],
                },
                event_id: "auto-failure".to_owned(),
                occurred_at: "2026-07-21T00:03:00Z".to_owned(),
            })
            .unwrap();
    }
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let snapshot = service.core_restore(false).unwrap();
    assert!(snapshot
        .core
        .tasks
        .iter()
        .any(|task| task.task_id == "restore-task"));
    assert!(service
        .task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 10,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .unwrap()
        .items
        .is_empty());
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn task_overview_archives_failed_task_and_returns_queue_and_history_entry() {
    let database = path("new-task-overview");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: 1,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec!["fixture-item".to_owned()],
                },
                event_id: "overview-failure".to_owned(),
                occurred_at: "2026-07-21T00:03:00Z".to_owned(),
            })
            .unwrap();
    }
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let overview = service.task_overview().unwrap();
    assert!(overview
        .queue
        .iter()
        .all(|task| task.task_id != "restore-task"));
    assert_eq!(overview.history_page.items.len(), 1);
    assert_eq!(overview.history_page.items[0].task_id, "restore-task");
    assert_eq!(service.task_overview().unwrap().history_page.items.len(), 1);
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn task_overview_archives_the_legacy_terminal_set_and_keeps_live_tasks() {
    let database = path("terminal-set-overview");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        for (task_id, kind) in [
            ("done-task", TaskStateCommandKind::MarkCompleted),
            ("cancelled-task", TaskStateCommandKind::CancelTask),
        ] {
            store
                .create_task(&StoreCreateTaskRequest {
                    task_id: task_id.to_owned(),
                    batch_id: "restore-batch".to_owned(),
                    selected_item_ids: vec!["fixture-item".to_owned()],
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 25,
                    output_root: PathBuf::from("fixture-output"),
                    layout: DownloadLayout::Direct,
                    concurrency: 1,
                    created_at: "2026-07-21T00:05:00Z".to_owned(),
                })
                .unwrap();
            if kind == TaskStateCommandKind::MarkCompleted {
                store
                    .save_download_result(
                        &DownloadItemResult {
                            task_id: task_id.to_owned(),
                            item_id: "fixture-item".to_owned(),
                            outcome: DownloadOutcome::Completed,
                            destination: PathBuf::from("fixture-output/fixture.zip"),
                            bytes_written: 1_190_000_000,
                            attempt_count: 1,
                            failure: None,
                            finished_at: "2026-07-21T00:05:30Z".to_owned(),
                        },
                        0,
                    )
                    .unwrap();
            }
            let item_ids = if kind == TaskStateCommandKind::MarkCompleted {
                vec!["fixture-item".to_owned()]
            } else {
                vec![]
            };
            store
                .update_task(&StoreUpdateTaskRequest {
                    task_id: task_id.to_owned(),
                    expected_revision: 0,
                    command: StoreTaskCommand { kind, item_ids },
                    event_id: format!("{task_id}-terminal"),
                    occurred_at: "2026-07-21T00:06:00Z".to_owned(),
                })
                .unwrap();
        }
    }
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let overview = service.task_overview().unwrap();
    let queued = overview
        .queue
        .iter()
        .map(|task| task.task_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(queued, ["restore-task"], "paused task stays live");
    let mut archived = overview
        .history_page
        .items
        .iter()
        .map(|item| item.task_id.as_str())
        .collect::<Vec<_>>();
    archived.sort_unstable();
    assert_eq!(archived, ["cancelled-task", "done-task"]);
    let done = overview
        .history_page
        .items
        .iter()
        .find(|item| item.task_id == "done-task")
        .unwrap();
    assert_eq!(done.transferred_bytes, 1_190_000_000);
    assert_eq!(done.total_bytes, Some(1_190_000_000));
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn failed_nisar_subset_stays_in_queue_on_task_overview() {
    let database = path("failed-nisar-overview");
    seed_store(&database);
    // Store's task type is derived from the frozen selected product and dataset.
    // Use the same read projection as the legacy terminal archiver, not provider alone.
    {
        let mut store = TaskStore::open(&database).unwrap();
        let mut batch = store.load_search_batch("restore-batch").unwrap().unwrap();
        batch.batch_id = "nisar-batch".to_owned();
        batch.provider_id = "asf.nasa.earthdata".to_owned();
        batch.query.insert(
            "dataset".to_owned(),
            serde_json::json!("sar.nisar.l.rslc.provisional.v1"),
        );
        batch.candidates[0].provider_id = batch.provider_id.clone();
        batch.candidates[0].metadata.insert(
            "delivery_kind".to_owned(),
            serde_json::json!("remote_derived_subset_v2"),
        );
        store
            .save_search_batch(&StoreSearchBatchRequest {
                batch,
                workbench: SearchWorkbenchState {
                    batch_id: "nisar-batch".to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 25,
                    revision: 0,
                },
                activate_workbench: false,
            })
            .unwrap();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: "nisar-failed".to_owned(),
                batch_id: "nisar-batch".to_owned(),
                selected_item_ids: vec!["fixture-item".to_owned()],
                checked_item_ids: vec!["fixture-item".to_owned()],
                page_index: 0,
                page_size: 25,
                output_root: PathBuf::from("fixture-output"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-07-21T00:01:00Z".to_owned(),
            })
            .unwrap();
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "nisar-failed".to_owned(),
                expected_revision: 0,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec!["fixture-item".to_owned()],
                },
                event_id: "nisar-failure".to_owned(),
                occurred_at: "2026-07-21T00:03:00Z".to_owned(),
            })
            .unwrap();
    }
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let overview = service.task_overview().unwrap();
    assert!(overview
        .queue
        .iter()
        .any(|task| task.task_id == "nisar-failed"
            && task.task_type.as_deref() == Some("nisar_subset")));
    assert!(overview
        .history_page
        .items
        .iter()
        .all(|item| item.task_id != "nisar-failed"));
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn task_overview_skips_in_flight_gacos_archive_and_continues_other_failures() {
    let database = path("gacos-in-flight-overview");
    seed_store(&database);
    let mut store = TaskStore::open(&database).unwrap();
    let mut batch = store.load_search_batch("restore-batch").unwrap().unwrap();
    batch.batch_id = "gacos-batch".to_owned();
    batch.provider_id = "gacos.webform".to_owned();
    batch.candidates[0].item_id = "request-1".to_owned();
    batch.candidates[0].provider_id = batch.provider_id.clone();
    batch.candidates[0].payload = serde_json::json!({"dates":["2024-01-02"],"hour":7,"minute":8})
        .as_object()
        .unwrap()
        .clone();
    let request = StoreSearchBatchRequest {
        batch,
        workbench: SearchWorkbenchState {
            batch_id: "gacos-batch".to_owned(),
            checked_item_ids: vec!["request-1".to_owned()],
            page_index: 0,
            page_size: 1,
            revision: 0,
        },
        activate_workbench: false,
    };
    let task = StoreCreateTaskRequest {
        task_id: "gacos-task".to_owned(),
        batch_id: "gacos-batch".to_owned(),
        selected_item_ids: vec!["request-1".to_owned()],
        checked_item_ids: vec!["request-1".to_owned()],
        page_index: 0,
        page_size: 1,
        output_root: PathBuf::from("fixture-output"),
        layout: DownloadLayout::Direct,
        concurrency: 1,
        created_at: "2026-07-21T00:01:00Z".to_owned(),
    };
    let operation = TaskOperationDescriptor {
        operation_id: "gacos-request".to_owned(),
        kind: "gacos".to_owned(),
        phase: "request".to_owned(),
        region_id: "gacos-region".to_owned(),
        approved_worker_operation: "gacos_manual_request".to_owned(),
        payload: serde_json::json!({"manual_action_required":true,"automatic_retry_allowed":false})
            .as_object()
            .unwrap()
            .clone(),
    };
    store
        .create_paused_gacos_request_task(&request, &task, &operation)
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "restore-task".to_owned(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec!["fixture-item".to_owned()],
            },
            event_id: "other-failure".to_owned(),
            occurred_at: "2026-07-21T00:04:00Z".to_owned(),
        })
        .unwrap();
    drop(store);
    // Startup recovers reserved/submitting attempts to submission_uncertain, so
    // reserve the in-flight attempt only after the service is running.
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    service.startup().unwrap();
    let mut store = TaskStore::open(&database).unwrap();
    let attempt = insar_store::GacosSubmissionReserveRequest {
        identity: insar_store::GacosSubmissionAttemptIdentity {
            task_id: "gacos-task".to_owned(),
            item_id: "request-1".to_owned(),
            attempt_id: "attempt-fixture".to_owned(),
            fingerprint_sha256: "a".repeat(64),
            account_identity_hash_sha256: "b".repeat(64),
            attempt_number: 0,
        },
        occurred_at: "2026-07-21T00:02:00Z".to_owned(),
        force_reapply: false,
    };
    store.reserve_gacos_submission_attempt(&attempt).unwrap();
    let gacos = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "gacos-task".to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: vec![],
            },
            event_id: "cancel-fixture".to_owned(),
            occurred_at: "2026-07-21T00:03:00Z".to_owned(),
        })
        .unwrap();
    assert_eq!(
        gacos.runtime.status,
        insar_core::TaskLifecycleStatus::Cancelled
    );
    drop(store);
    let overview = service.task_overview().unwrap();
    assert!(overview
        .queue
        .iter()
        .any(|task| task.task_id == "gacos-task"));
    assert!(overview
        .queue
        .iter()
        .all(|task| task.task_id != "restore-task"));
    assert!(overview
        .history_page
        .items
        .iter()
        .any(|item| item.task_id == "restore-task"));
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn fallback_comparison_reports_changed_product_ids_without_mutating_original_task() {
    let database = path("retry-comparison");
    seed_store(&database);
    let store = TaskStore::open(&database).unwrap();
    let before = store.load_task_record("restore-task").unwrap().unwrap();
    let mut refreshed = store.load_search_batch("restore-batch").unwrap().unwrap();
    refreshed.batch_id = "fresh-batch".to_owned();
    refreshed.candidates[0].item_id = "replacement-product".to_owned();
    let comparison =
        compare_retry_fallback(&before.snapshot, &refreshed).expect("compare identities only");
    assert_eq!(comparison.missing_product_ids, vec!["fixture-item"]);
    assert_eq!(comparison.added_product_ids, vec!["replacement-product"]);
    assert!(comparison.needs_user_confirmation);
    assert_eq!(
        store.load_task_record("restore-task").unwrap().unwrap(),
        before
    );
    let unchanged = store.load_search_batch("restore-batch").unwrap().unwrap();
    let identical = compare_retry_fallback(&before.snapshot, &unchanged).unwrap();
    assert!(identical.missing_product_ids.is_empty());
    assert!(identical.added_product_ids.is_empty());
    assert!(!identical.needs_user_confirmation); // Unchanged sets rebuild via snapshot revisions.
                                                 // "Added" is the refreshed batch minus the original batch, not minus the
                                                 // task's selection: an unselected original product is not new.
    let mut wider_task = before.snapshot.clone();
    let mut unselected = wider_task.candidates[0].clone();
    unselected.item_id = "unselected-original".to_owned();
    wider_task.candidates.push(unselected.clone());
    let mut wider = unchanged.clone();
    wider.batch_id = "wider-batch".to_owned();
    wider.candidates.push(unselected);
    let comparison = compare_retry_fallback(&wider_task, &wider).unwrap();
    assert!(comparison.added_product_ids.is_empty(), "{comparison:?}");
    let mut changed_query = unchanged;
    changed_query
        .query
        .insert("unexpected".to_owned(), serde_json::json!(true));
    assert!(compare_retry_fallback(&before.snapshot, &changed_query).is_err());
    drop(store);
    cleanup(&database);
}

/// L4 r10 P1-1: the download center's 重试 on a cancelled task (which the
/// task overview had moved to history) failed with V3_STORE_INVALID_SNAPSHOT
/// and 「建任务失败，请检查所选数据及输出路径」. A cancelled history task is
/// retried on its own id from its frozen snapshot, like a failed one.
#[test]
fn task_action_retry_restores_a_cancelled_history_task() {
    let database = path("unified-action-retry-cancelled");
    seed_store(&database);
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let cancelled = service
        .task_action(
            "restore-task".to_owned(),
            1,
            insar_application_service::UnifiedTaskAction::Cancel,
        )
        .expect("cancel the paused task");
    assert_eq!(cancelled.status, insar_core::TaskLifecycleStatus::Cancelled);
    // The overview the desktop polls moves the cancelled task to history.
    let overview = service.task_overview().unwrap();
    assert!(overview
        .history_page
        .items
        .iter()
        .any(|item| item.task_id == "restore-task"));
    let retried = service
        .task_action(
            "restore-task".to_owned(),
            cancelled.revision,
            insar_application_service::UnifiedTaskAction::Retry,
        )
        .expect("retry a cancelled history task");
    assert_eq!(retried.task_id, "restore-task");
    assert_eq!(retried.revision, cancelled.revision + 1);
    assert_eq!(retried.status, insar_core::TaskLifecycleStatus::Queued);
    assert!(service
        .task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 10,
            after_archived_at_epoch_ms: None,
            after_task_id: None
        })
        .unwrap()
        .items
        .is_empty());
    service.shutdown().unwrap();
    cleanup(&database);
}

/// A retry with nothing to retry says so in Chinese with its own code,
/// instead of the store's internal invalid-snapshot code.
#[test]
fn task_action_retry_with_nothing_to_retry_says_why() {
    let database = path("unified-action-retry-unavailable");
    seed_store(&database);
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let error = service
        .task_action(
            "restore-task".to_owned(),
            1,
            insar_application_service::UnifiedTaskAction::Retry,
        )
        .expect_err("a paused task has no failed or cancelled file");
    assert_eq!(error.code, "TASK_RETRY_UNAVAILABLE");
    assert!(
        error.message_zh.contains("没有失败或已取消的文件"),
        "{}",
        error.message_zh
    );
    assert!(!error.retryable);
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn task_action_retry_reports_revision_conflict_before_unavailable() {
    let database = path("unified-action-retry-stale");
    seed_store(&database);
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let error = service
        .task_action(
            "restore-task".to_owned(),
            0,
            insar_application_service::UnifiedTaskAction::Retry,
        )
        .expect_err("the revision is stale even though the paused task has no retryable items");
    assert_eq!(error.code, "V3_STORE_REVISION_CONFLICT");
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn task_action_retry_preserves_id_and_failed_history_membership() {
    let database = path("unified-action-retry");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        let failed = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: 1,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec!["fixture-item".to_owned()],
                },
                event_id: "action-failure".to_owned(),
                occurred_at: "2026-07-21T00:03:00Z".to_owned(),
            })
            .unwrap();
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: failed.runtime.revision,
                archived_at: "2026-07-21T00:04:00Z".to_owned(),
            })
            .unwrap();
    }
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    let resumed = service
        .task_action(
            "restore-task".to_owned(),
            2,
            insar_application_service::UnifiedTaskAction::Retry,
        )
        .expect("retry existing failed history identity");
    assert_eq!(resumed.task_id, "restore-task");
    assert_eq!(resumed.revision, 3);
    assert!(service
        .task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 10,
            after_archived_at_epoch_ms: None,
            after_task_id: None
        })
        .unwrap()
        .items
        .is_empty());
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn archived_failure_retries_through_service_on_the_original_task_id() {
    let database = path("retry-failed-history");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        let failed = store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: 1,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec!["fixture-item".to_owned()],
                },
                event_id: "failed-before-history".to_owned(),
                occurred_at: "2026-07-21T00:03:00Z".to_owned(),
            })
            .unwrap();
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: failed.runtime.revision,
                archived_at: "2026-07-21T00:04:00Z".to_owned(),
            })
            .unwrap();
    }
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    service.startup().unwrap();
    let request = StoreUpdateTaskRequest {
        task_id: "restore-task".to_owned(),
        expected_revision: 2,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::RetryTask,
            item_ids: vec![],
        },
        event_id: "retry-original-history".to_owned(),
        occurred_at: "2026-07-21T00:05:00Z".to_owned(),
    };
    let result = service.update_download_task(&request).unwrap();
    assert_eq!(result.core.tasks.len(), 1);
    assert_eq!(result.core.tasks[0].task_id, "restore-task");
    assert_eq!(result.core.tasks[0].revision, 3);
    assert!(service
        .task_history_page(&StoreTaskHistoryPageRequest {
            page_size: 10,
            after_archived_at_epoch_ms: None,
            after_task_id: None,
        })
        .unwrap()
        .items
        .is_empty());
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn archive_mutation_returns_fixed_ack_with_more_than_one_history_page_present() {
    let database = path("archive-fixed-ack");
    seed_store(&database);
    let target_revision = {
        let mut store = TaskStore::open(&database).unwrap();
        for index in 0..41 {
            let task_id = format!("archived-{index:02}");
            store
                .create_task(&StoreCreateTaskRequest {
                    task_id: task_id.clone(),
                    batch_id: "restore-batch".to_owned(),
                    selected_item_ids: vec!["fixture-item".to_owned()],
                    checked_item_ids: vec!["fixture-item".to_owned()],
                    page_index: 0,
                    page_size: 25,
                    output_root: PathBuf::from(format!("archive-output-{index:02}")),
                    layout: DownloadLayout::Direct,
                    concurrency: 1,
                    created_at: "2026-07-22T00:00:00Z".to_owned(),
                })
                .unwrap();
            let completed = store
                .update_task(&StoreUpdateTaskRequest {
                    task_id: task_id.clone(),
                    expected_revision: 0,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::MarkCompleted,
                        item_ids: vec!["fixture-item".to_owned()],
                    },
                    event_id: format!("complete-{index:02}"),
                    occurred_at: "2026-07-22T00:01:00Z".to_owned(),
                })
                .unwrap();
            store
                .archive_task(&StoreArchiveTaskRequest {
                    task_id,
                    expected_revision: completed.runtime.revision,
                    archived_at: format!("2026-07-22T00:02:{index:02}Z"),
                })
                .unwrap();
        }
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "restore-task".to_owned(),
                expected_revision: 1,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkCompleted,
                    item_ids: vec!["fixture-item".to_owned()],
                },
                event_id: "complete-archive-target".to_owned(),
                occurred_at: "2026-07-22T00:03:00Z".to_owned(),
            })
            .unwrap()
            .runtime
            .revision
    };

    let request = StoreArchiveTaskRequest {
        task_id: "restore-task".to_owned(),
        expected_revision: target_revision,
        archived_at: "2026-07-22T00:04:00Z".to_owned(),
    };
    let service = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    service.startup().unwrap();
    let ack = service.archive_download_task(&request).unwrap();
    assert_eq!(ack.task_id, request.task_id);
    assert_eq!(ack.revision, request.expected_revision);
    assert_eq!(ack.archived_at, request.archived_at);
    let serialized = serde_json::to_value(ack).unwrap();
    let object = serialized.as_object().expect("archive ack object");
    assert_eq!(
        object.len(),
        3,
        "archive mutation response stays fixed-size"
    );
    assert!(object.contains_key("task_id"));
    assert!(object.contains_key("revision"));
    assert!(object.contains_key("archived_at"));
    assert!(serialized.get("tasks").is_none());
    assert!(serialized.get("core").is_none());
    assert!(serialized.get("snapshot_json").is_none());
    service.shutdown().unwrap();
    cleanup(&database);
}

#[test]
fn shutdown_is_idempotent_and_does_not_reopen_the_store() {
    let database = path("shutdown");
    let service = ApplicationService::new(ApplicationServiceConfig::store(&database));
    service.startup().unwrap();
    service.shutdown().unwrap();
    service.shutdown().unwrap();
    assert_eq!(service.status().unwrap(), ServiceStatus::ShuttingDown);
    assert_eq!(
        service.restore(false).unwrap_err().code(),
        "V3_APPLICATION_SHUTTING_DOWN"
    );
    cleanup(&database);
}

#[test]
fn shutdown_and_rollback_leave_outputs_parts_and_history_untouched() {
    let root = path("rollback").with_extension("");
    fs::create_dir_all(&root).unwrap();
    let database = root.join("tasks.sqlite3");
    let output = root.join("product.bin");
    let partial = root.join("product.bin.part");
    let history = root.join("history.json");
    fs::write(&output, b"user output").unwrap();
    fs::write(&partial, b"partial bytes").unwrap();
    fs::write(&history, b"history evidence").unwrap();

    let service = ApplicationService::new(ApplicationServiceConfig::store(&database));
    service.startup().unwrap();
    service.shutdown().unwrap();

    assert_eq!(fs::read(&output).unwrap(), b"user output");
    assert_eq!(fs::read(&partial).unwrap(), b"partial bytes");
    assert_eq!(fs::read(&history).unwrap(), b"history evidence");
    assert!(database.exists());

    let _ = fs::remove_dir_all(root);
}

fn save_failure_fixture_batch(store: &mut TaskStore, batch_id: &str, item_ids: &[&str]) {
    let mut batch = store.load_search_batch("restore-batch").unwrap().unwrap();
    batch.batch_id = batch_id.to_owned();
    let template = batch.candidates[0].clone();
    batch.candidates = item_ids
        .iter()
        .map(|item_id| SnapshotItem {
            item_id: (*item_id).to_owned(),
            display_name: (*item_id).to_owned(),
            ..template.clone()
        })
        .collect();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: SearchWorkbenchState {
                batch_id: batch_id.to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 25,
                revision: 0,
            },
            activate_workbench: false,
        })
        .unwrap();
}

/// A queued task over `item_ids` with every item activated, as the scheduler
/// leaves it before its workers report. Returns the runtime revision.
fn running_fixture_task(store: &mut TaskStore, task_id: &str, item_ids: &[&str]) -> u64 {
    let batch_id = format!("{task_id}-batch");
    save_failure_fixture_batch(store, &batch_id, item_ids);
    let item_ids = item_ids
        .iter()
        .map(|id| (*id).to_owned())
        .collect::<Vec<_>>();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id,
            selected_item_ids: item_ids.clone(),
            checked_item_ids: item_ids.clone(),
            page_index: 0,
            page_size: 25,
            output_root: PathBuf::from("fixture-output"),
            layout: DownloadLayout::Direct,
            concurrency: 8,
            created_at: "2026-07-21T00:10:00Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ActivateItems,
                item_ids,
            },
            event_id: format!("{task_id}-activate"),
            occurred_at: "2026-07-21T00:11:00Z".to_owned(),
        })
        .unwrap()
        .runtime
        .revision
}

/// The engine's failure path: the worker's failed result row, then the
/// scheduler's `mark_failed` command. Returns the new runtime revision.
fn engine_fail_item(
    store: &mut TaskStore,
    task_id: &str,
    revision: u64,
    item_id: &str,
    failure: Option<(&str, &str)>,
) -> u64 {
    store
        .save_scheduler_download_result(&DownloadItemResult {
            task_id: task_id.to_owned(),
            item_id: item_id.to_owned(),
            outcome: DownloadOutcome::Failed,
            destination: PathBuf::from("fixture-output").join(item_id),
            bytes_written: 0,
            attempt_count: 1,
            failure: failure.map(|(code, reason_zh)| insar_core::DownloadFailure {
                code: code.to_owned(),
                reason_zh: reason_zh.to_owned(),
                retryable: false,
            }),
            finished_at: "2026-07-21T00:12:00Z".to_owned(),
        })
        .unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::MarkFailed,
                item_ids: vec![item_id.to_owned()],
            },
            event_id: format!("{task_id}-{item_id}-failed"),
            occurred_at: "2026-07-21T00:12:00Z".to_owned(),
        })
        .unwrap()
        .runtime
        .revision
}

const DL005_REASON: &str = "无法读取或写入下载目录，请检查目录权限和磁盘状态。";

#[test]
fn task_overview_reports_persisted_dl005_failure_after_restart() {
    let database = path("overview-failure-dl005");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        let revision = running_fixture_task(&mut store, "dl005-task", &["dl005-item"]);
        engine_fail_item(
            &mut store,
            "dl005-task",
            revision,
            "dl005-item",
            Some(("DL005", DL005_REASON)),
        );
    }
    // The legacy projection keeps its codes-only shape.
    let legacy = ApplicationService::new(ApplicationServiceConfig::store(&database));
    let legacy_task = legacy
        .core_restore(false)
        .unwrap()
        .core
        .tasks
        .into_iter()
        .find(|task| task.task_id == "dl005-task")
        .unwrap();
    assert_eq!(legacy_task.failure_codes.len(), 1);
    assert_eq!(legacy_task.failure_codes[0].code, "DL005");
    let legacy_json = serde_json::to_value(&legacy_task).unwrap();
    assert!(legacy_json.get("failure").is_none());
    assert!(!legacy_json.to_string().contains(DL005_REASON));
    legacy.shutdown().unwrap();

    let expected = insar_core::TaskFailureSummary {
        codes: vec!["DL005".to_owned()],
        message_zh: Some(DL005_REASON.to_owned()),
        failed_items: 1,
        total_items: 1,
    };
    // The second pass reads the archived entry from a fresh service.
    for _restart in 0..2 {
        let service = ApplicationService::new(ApplicationServiceConfig::store(&database));
        let overview = service.task_overview().unwrap();
        let entry = overview
            .history_page
            .items
            .iter()
            .find(|item| item.task_id == "dl005-task")
            .expect("failed task is archived into history");
        assert_eq!(entry.failure.as_ref(), Some(&expected));
        assert!(overview
            .queue
            .iter()
            .all(|task| task.task_id != "dl005-task"));
        let paused = overview
            .queue
            .iter()
            .find(|task| task.task_id == "restore-task")
            .expect("paused task stays live");
        assert_eq!(paused.failure, None);
        service.shutdown().unwrap();
    }
    cleanup(&database);
}

#[test]
fn task_overview_running_task_has_no_failure_and_partial_failures_keep_recorded_order() {
    let database = path("overview-failure-running");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        running_fixture_task(&mut store, "running-task", &["run-a", "run-b"]);
        let mut revision = running_fixture_task(
            &mut store,
            "partial-task",
            &["item-a", "item-b", "item-c", "item-d", "item-e"],
        );
        // Recorded order b, a, c, e: sorted codes would put DL003 first.
        for (item_id, failure) in [
            ("item-b", Some(("DL005", DL005_REASON))),
            ("item-a", Some(("DL003", "服务器拒绝了请求。"))),
            ("item-c", Some(("DL005", "第二个目录错误"))),
            ("item-e", None),
        ] {
            revision = engine_fail_item(&mut store, "partial-task", revision, item_id, failure);
        }
    }
    let service = ApplicationService::new(ApplicationServiceConfig::store(&database));
    let overview = service.task_overview().unwrap();
    let running = overview
        .queue
        .iter()
        .find(|task| task.task_id == "running-task")
        .unwrap();
    assert_eq!(running.status, insar_core::TaskLifecycleStatus::Running);
    assert_eq!(running.failure, None);
    let partial = overview
        .queue
        .iter()
        .find(|task| task.task_id == "partial-task")
        .expect("item-d is still active");
    assert_eq!(partial.status, insar_core::TaskLifecycleStatus::Running);
    assert_eq!(
        partial.failure,
        Some(insar_core::TaskFailureSummary {
            codes: vec![
                "DL005".to_owned(),
                "DL003".to_owned(),
                "UNKNOWN_FAILURE_CODE".to_owned(),
            ],
            message_zh: Some(DL005_REASON.to_owned()),
            failed_items: 4,
            total_items: 5,
        })
    );
    let wire = serde_json::to_value(&overview).unwrap();
    let running_wire = wire["queue"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["task_id"] == "running-task")
        .unwrap();
    assert!(running_wire["failure"].is_null());
    service.shutdown().unwrap();
    cleanup(&database);
}

/// The anonymous Planetary Computer batch of the unified create-task test:
/// the account pre-flight admits it without a session.
fn save_public_unified_batch(database: &PathBuf) {
    let mut store = TaskStore::open(database).unwrap();
    let mut batch = store.load_search_batch("restore-batch").unwrap().unwrap();
    batch.batch_id = "unified-batch".to_owned();
    batch.provider_id = "microsoft.planetary-computer".to_owned();
    batch.query = serde_json::json!({"logical_dataset":"optical.landsat.c2.l2", "provider_id":"microsoft.planetary-computer", "native_collection_id":"landsat-c2-l2"}).as_object().unwrap().clone();
    batch.candidates[0].provider_id = batch.provider_id.clone();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch,
            workbench: SearchWorkbenchState {
                batch_id: "unified-batch".to_owned(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 25,
                revision: 0,
            },
            activate_workbench: false,
        })
        .unwrap();
}

fn stored_task_ids(database: &PathBuf) -> Vec<String> {
    TaskStore::open(database)
        .unwrap()
        .core_restore(true)
        .unwrap()
        .tasks
        .into_iter()
        .map(|task| task.task_id)
        .collect()
}

fn probe_leftovers(root: &std::path::Path) -> Vec<String> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".eostage-write-probe-"))
        .collect()
}

/// A regular file at the storage-root path stands in for a root the user
/// cannot write: a Windows read-only attribute on a directory does not stop
/// file creation, and a deny ACL would need icacls outside the test process.
/// Both the file itself and a root below it must be refused before a task row.
#[test]
fn unwritable_storage_root_is_refused_before_any_task_row() {
    let database = path("storage-root-file");
    seed_store(&database);
    save_public_unified_batch(&database);
    let blocker = database.with_extension("root-file");
    fs::write(&blocker, b"not a directory").unwrap();
    let before = stored_task_ids(&database);
    let service = offline_task_service(&database);
    for root in [blocker.clone(), blocker.join("downloads")] {
        let error = service
            .create_task(
                "unified-batch".to_owned(),
                0,
                vec!["fixture-item".to_owned()],
                UnifiedCreateTaskOptions {
                    output_root: root.clone(),
                    layout: DownloadLayout::Direct,
                    concurrency: 1,
                    include_precise_orbit: None,
                },
            )
            .expect_err("an unwritable storage root is refused");
        assert_eq!(error.code, "STORAGE_ROOT_NOT_WRITABLE");
        assert!(!error.retryable);
        assert_eq!(
            error.message_zh,
            format!(
                "存储根路径不可写：{}（请检查目录权限或更换路径）",
                root.display()
            )
        );
        assert_eq!(error.needs_account, None);
    }
    service.shutdown().unwrap();
    assert_eq!(
        stored_task_ids(&database),
        before,
        "no task row was created"
    );
    assert_eq!(fs::read(&blocker).unwrap(), b"not a directory");
    let _ = fs::remove_file(&blocker);
    cleanup(&database);
}

#[test]
fn writable_storage_root_passes_and_leaves_no_probe() {
    let database = path("storage-root-writable");
    seed_store(&database);
    save_public_unified_batch(&database);
    let root = database.with_extension("root");
    fs::create_dir_all(&root).unwrap();
    let service = offline_task_service(&database);
    let created = service
        .create_task(
            "unified-batch".to_owned(),
            0,
            vec!["fixture-item".to_owned()],
            UnifiedCreateTaskOptions {
                output_root: root.clone(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                include_precise_orbit: None,
            },
        )
        .expect("a writable root admits the task");
    assert!(created.created);
    assert!(probe_leftovers(&root).is_empty());
    // A root that does not exist yet is admitted when it can be created,
    // and the probe does not create it.
    let missing = root.join("not-yet").join("downloads");
    assert!(admitted_as_existing_task(&service, &missing));
    assert!(!root.join("not-yet").exists());
    assert!(probe_leftovers(&root).is_empty());
    service.shutdown().unwrap();
    assert!(stored_task_ids(&database).contains(&created.task_id));
    let _ = fs::remove_dir_all(&root);
    cleanup(&database);
}

/// The same product already has an active task, so admission returns it
/// (`created == false`) once the storage pre-flight has passed.
fn admitted_as_existing_task(service: &ApplicationService, root: &std::path::Path) -> bool {
    service
        .create_task(
            "unified-batch".to_owned(),
            0,
            vec!["fixture-item".to_owned()],
            UnifiedCreateTaskOptions {
                output_root: root.to_path_buf(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                include_precise_orbit: None,
            },
        )
        .is_ok_and(|task| !task.created)
}

#[test]
fn production_startup_prunes_expired_and_excess_unreferenced_batches_while_preserving_pins() {
    let database = path("production-startup-prune");
    seed_store(&database);
    {
        let mut store = TaskStore::open(&database).unwrap();
        let base_batch = store.load_search_batch("restore-batch").unwrap().unwrap();
        let now_ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        // Seed an expired unreferenced batch (`2020-01-01`) + 70 fresh unreferenced batches (> 64 cap).
        for idx in 0..71_usize {
            let mut batch = base_batch.clone();
            let batch_id = if idx == 0 {
                "unreferenced-expired-batch".to_owned()
            } else {
                format!("unreferenced-fresh-{idx:03}")
            };
            batch.batch_id = batch_id.clone();
            batch.created_at = if idx == 0 {
                "2020-01-01T00:00:00Z".to_owned()
            } else {
                now_ts.clone()
            };
            batch.candidates[0].item_id = format!("item-{idx:03}");
            store
                .save_search_batch(&StoreSearchBatchRequest {
                    batch,
                    workbench: SearchWorkbenchState {
                        batch_id,
                        checked_item_ids: vec![],
                        page_index: 0,
                        page_size: 25,
                        revision: 0,
                    },
                    activate_workbench: false,
                })
                .unwrap();
        }
    }

    let prod = ApplicationService::new(ApplicationServiceConfig::business_store(&database));
    prod.startup().expect("production startup succeeds");
    prod.shutdown().unwrap();

    let store = TaskStore::open(&database).unwrap();
    // Active workbench / task batch (`restore-batch`, dated 2026-07-21) is pinned and MUST survive.
    assert!(store.load_search_batch("restore-batch").unwrap().is_some());
    // Expired batch must be evicted by Production startup TTL pruning.
    assert!(store
        .load_search_batch("unreferenced-expired-batch")
        .unwrap()
        .is_none());
    // Total disposable search cache entries must fit within the 64-entry bound.
    let stats = store.inspect_disposable_cache_stats().unwrap();
    assert!(
        stats.search_cache_entries <= TaskStore::SEARCH_CACHE_MAX_ENTRIES as u64,
        "search_cache_entries={}",
        stats.search_cache_entries
    );
    drop(store);
    cleanup(&database);
}
