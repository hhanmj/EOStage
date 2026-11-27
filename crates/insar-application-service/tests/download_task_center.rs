use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use insar_application_service::{
    ApplicationCatalogSearchAndFreezeRequest, ApplicationService, ApplicationServiceConfig,
    ExistingDownloadCheckItem, UnifiedTaskItemAction,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, CatalogCollectionDescriptor,
    CatalogDeliveryModel, CatalogSearchQuery, CatalogSearchResponse, CatalogSearchStats,
    ContractError, DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadLayout,
    DownloadOutcome, ProviderProduct, ProviderResult, SearchWorkbenchState, SnapshotItem,
    StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest,
    TaskLifecycleStatus, TaskStateCommandKind,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_provider::{
    asf::{map_alos2_umm_granule, CmrGranuleEnvelope},
    catalog::{
        CollectionProfileRegistry, ALOS2_FILENAME_RULE_ID, ALOS2_IDENTITY_RULE_ID,
        ALOS2_LOGICAL_DATASET, ALOS2_NATIVE_COLLECTION_ID, ALOS2_PROFILE_ID, ALOS2_PROVIDER_ID,
    },
};
use insar_service::{LiveAsfSearch, LiveCatalogSearch};
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace(PathBuf);
impl TempWorkspace {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "insar-dtc-{label}-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn db_path(&self) -> PathBuf {
        self.0.join("store.sqlite3")
    }

    fn out_dir(&self) -> PathBuf {
        self.0.join("downloads")
    }
}
impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct UnusedAsf;
impl LiveAsfSearch for UnusedAsf {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("DUMMY", "unused", Map::new()).unwrap())
    }
}

struct ReplayCatalog {
    url: Mutex<String>,
    searches: AtomicU64,
}

fn alos2_fixture_product(url: &str) -> ProviderProduct {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/fixtures/asf-alos2-p1/granule-wbd-cmr-umm.json");
    let raw_str = fs::read_to_string(path).unwrap().replace(
        "https://example.invalid/alos2/ALOS2478123150-230331-WBDR1.1__D.zip",
        url,
    );
    let raw: Value = serde_json::from_str(&raw_str).unwrap();
    let envelope = CmrGranuleEnvelope::try_from(&raw).unwrap();
    let profile = CollectionProfileRegistry.get(ALOS2_PROFILE_ID).unwrap();
    map_alos2_umm_granule(&envelope, profile).unwrap()
}

impl LiveCatalogSearch for ReplayCatalog {
    fn search_catalog(
        &self,
        _query: &CatalogSearchQuery,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<CatalogSearchResponse> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        let url = self.url.lock().unwrap().clone();
        Ok(CatalogSearchResponse {
            collection: CatalogCollectionDescriptor {
                logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
                provider_id: ALOS2_PROVIDER_ID.to_owned(),
                native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
                collection_profile_id: ALOS2_PROFILE_ID.to_owned(),
                identity_rule_id: ALOS2_IDENTITY_RULE_ID.to_owned(),
                filename_rule_id: ALOS2_FILENAME_RULE_ID.to_owned(),
                delivery_model: CatalogDeliveryModel::ArchiveFirst,
                production_enabled: false,
            },
            products: vec![alos2_fixture_product(&url)],
            stats: CatalogSearchStats {
                requested_limit: Some(25),
                matched_count: Some(1),
                request_count: 1,
                transport_part_count: 1,
                fetched_record_count: 1,
                mapped_record_count: 1,
                profile_filtered_record_count: 0,
                exact_duplicate_record_count: 0,
                unique_provider_product_count: 1,
                exact_matched_count: 1,
                returned_count: 1,
                remote_exhausted: true,
                working_set_satisfied: true,
                elapsed_ms: 1,
                truncated: false,
            },
            next_cursor: None,
            item_errors: Vec::new(),
        })
    }
}

#[derive(Default)]
struct SelectiveWorker {
    complete_urls: Mutex<Vec<String>>,
    attempts: Mutex<Vec<SnapshotItem>>,
    block_item: Mutex<Option<String>>,
}

impl DownloadWorker for SelectiveWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.attempts.lock().unwrap().push(work.item.clone());
        let item_id = work.item.item_id.clone();
        while self.block_item.lock().unwrap().as_deref() == Some(item_id.as_str()) {
            if control.is_cancelled() || control.is_paused() {
                return DownloadItemResult {
                    task_id: work.task.task_id,
                    item_id,
                    outcome: DownloadOutcome::Cancelled,
                    destination: work.task.output_root.join(&work.item.display_name),
                    bytes_written: 0,
                    attempt_count: 1,
                    failure: None,
                    finished_at: "2026-07-25T00:00:05Z".to_owned(),
                };
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        let item_json = serde_json::to_string(&work.item).unwrap_or_default();
        if self
            .complete_urls
            .lock()
            .unwrap()
            .iter()
            .any(|u| item_json.contains(u))
        {
            fs::create_dir_all(&work.task.output_root).unwrap();
            let destination = work.task.output_root.join(&work.item.display_name);
            fs::write(&destination, b"hello-scene!").unwrap();
            return DownloadItemResult {
                task_id: work.task.task_id,
                item_id,
                outcome: DownloadOutcome::Completed,
                destination,
                bytes_written: 12,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-07-25T00:00:05Z".to_owned(),
            };
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id,
            outcome: DownloadOutcome::Failed,
            destination: work.task.output_root.join(&work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: "DL002".to_owned(),
                reason_zh: "链接已失效".to_owned(),
                retryable: false,
            }),
            finished_at: "2026-07-25T00:00:05Z".to_owned(),
        }
    }
}

fn aoi() -> Map<String, Value> {
    json!({
        "type": "Polygon",
        "coordinates": [[[100.0, 30.0], [101.0, 30.0], [101.0, 31.0], [100.0, 31.0], [100.0, 30.0]]]
    })
    .as_object()
    .unwrap()
    .clone()
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("condition timed out");
}

fn catalog_search_query() -> CatalogSearchQuery {
    CatalogSearchQuery {
        logical_dataset: ALOS2_LOGICAL_DATASET.to_owned(),
        provider_id: ALOS2_PROVIDER_ID.to_owned(),
        native_collection_id: ALOS2_NATIVE_COLLECTION_ID.to_owned(),
        aoi_geojson: Some(aoi()),
        cursor: None,
        result_limit: Some(25),
        filters: json!({
            "datetime": {"start": "2023-03-31T00:00:00Z", "end": "2023-04-01T00:00:00Z"},
            "beam_mode": "WBD",
            "polarizations": ["HH", "HV"],
            "flight_direction": "DESCENDING",
            "look_direction": "RIGHT",
            "path": 161,
            "frame": 3150
        })
        .as_object()
        .unwrap()
        .clone(),
    }
}

#[test]
fn retry_fallback_replays_catalog_search_query_and_reports_eligibility() {
    let ws = TempWorkspace::new("catalog-fallback");
    let catalog = Arc::new(ReplayCatalog {
        url: Mutex::new(
            "https://example.invalid/alos2/old-v1/ALOS2478123150-230331-WBDR1.1__D.zip".to_owned(),
        ),
        searches: AtomicU64::new(0),
    });
    let worker = Arc::new(SelectiveWorker::default());
    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(ws.db_path()),
        Arc::new(UnusedAsf),
        Arc::clone(&catalog) as Arc<dyn LiveCatalogSearch>,
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
    );

    let frozen = service
        .catalog_search_and_freeze_legacy(&ApplicationCatalogSearchAndFreezeRequest {
            request_id: "cat-req-1".to_owned(),
            search: catalog_search_query(),
            batch_id: "cat-batch-1".to_owned(),
            created_at: "2026-07-25T00:00:00Z".to_owned(),
            aoi_name: "Fallback Catalog AOI".to_owned(),
            aoi_geojson: Some(aoi()),
            workbench_page_size: 25,
        })
        .unwrap();

    let item_id = frozen.restore.workbench_batch.unwrap().candidates[0]
        .item_id
        .clone();

    service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: "cat-task".to_owned(),
            batch_id: "cat-batch-1".to_owned(),
            selected_item_ids: vec![item_id.clone()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: ws.out_dir(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-25T00:00:01Z".to_owned(),
        })
        .unwrap();

    wait_until(|| {
        let store = TaskStore::open(ws.db_path()).unwrap();
        store
            .load_task_record("cat-task")
            .unwrap()
            .is_some_and(|r| r.runtime.status == TaskLifecycleStatus::Failed)
    });

    // Also add an ineligible non-search batch and a derived optical batch into Store directly.
    {
        let mut store = TaskStore::open(ws.db_path()).unwrap();
        let plain_batch: insar_core::FrozenSearchBatch = serde_json::from_value(json!({
            "schema_version": 1,
            "batch_id": "non-replayable-batch",
            "provider_id": "copernicus.dataspace",
            "created_at": "2026-07-25T00:00:00Z",
            "aoi_name": "Ineligible",
            "aoi_geojson": aoi(),
            "query": {"custom_only": true},
            "search_stats": null,
            "candidates": [{
                "item_id": "plain-1",
                "provider_id": "copernicus.dataspace",
                "display_name": "plain-1.zip",
                "acquisition_datetime": null,
                "footprint_geojson": null,
                "metadata": {},
                "payload": {}
            }]
        }))
        .unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                workbench: SearchWorkbenchState {
                    batch_id: "non-replayable-batch".to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                batch: plain_batch,
                activate_workbench: false,
            })
            .unwrap();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: "ineligible-task".to_owned(),
                batch_id: "non-replayable-batch".to_owned(),
                selected_item_ids: vec!["plain-1".to_owned()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: ws.out_dir(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-07-25T00:00:02Z".to_owned(),
            })
            .unwrap();
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "ineligible-task".to_owned(),
                expected_revision: 0,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec!["plain-1".to_owned()],
                },
                event_id: "fail-plain".to_owned(),
                occurred_at: "2026-07-25T00:00:03Z".to_owned(),
            })
            .unwrap();

        let derived_batch: insar_core::FrozenSearchBatch = serde_json::from_value(json!({
            "schema_version": 1,
            "batch_id": "derived-batch",
            "provider_id": "microsoft.planetary-computer",
            "created_at": "2026-07-25T00:00:00Z",
            "aoi_name": "Derived",
            "aoi_geojson": aoi(),
            "query": {
                "delivery_kind": "landsat_c2_l2_asset_batch",
                "derived_from_batch_id": "cat-batch-1"
            },
            "search_stats": null,
            "candidates": [{
                "item_id": "der-1",
                "provider_id": "microsoft.planetary-computer",
                "display_name": "B4.TIF",
                "acquisition_datetime": null,
                "footprint_geojson": null,
                "metadata": {},
                "payload": {}
            }]
        }))
        .unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                workbench: SearchWorkbenchState {
                    batch_id: "derived-batch".to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                batch: derived_batch,
                activate_workbench: false,
            })
            .unwrap();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: "derived-task".to_owned(),
                batch_id: "derived-batch".to_owned(),
                selected_item_ids: vec!["der-1".to_owned()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: ws.out_dir(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-07-25T00:00:04Z".to_owned(),
            })
            .unwrap();
    }

    let eligibility = service
        .task_retry_fallback_eligibility(&[
            "cat-task".to_owned(),
            "derived-task".to_owned(),
            "ineligible-task".to_owned(),
            "nonexistent".to_owned(),
        ])
        .unwrap();
    assert_eq!(
        eligibility,
        BTreeMap::from([
            ("cat-task".to_owned(), true),
            ("derived-task".to_owned(), true),
            ("ineligible-task".to_owned(), false),
            ("nonexistent".to_owned(), false),
        ])
    );

    // Refresh catalog download URL and trigger retry fallback for the CatalogSearchQuery task.
    let new_url = "https://example.invalid/alos2/fresh-v2/ALOS2478123150-230331-WBDR1.1__D.zip";
    *catalog.url.lock().unwrap() = new_url.to_owned();
    *worker.complete_urls.lock().unwrap() = vec![new_url.to_owned()];

    let outcome = service
        .task_retry_fallback("cat-task".to_owned(), None)
        .unwrap();
    assert!(!outcome.needs_user_confirmation);
    assert_eq!(catalog.searches.load(Ordering::SeqCst), 2);

    wait_until(|| {
        let store = TaskStore::open(ws.db_path()).unwrap();
        store
            .load_task_record("cat-task")
            .unwrap()
            .is_some_and(|r| r.runtime.status == TaskLifecycleStatus::Completed)
    });

    let attempts = worker.attempts.lock().unwrap();
    let cat_attempt = attempts
        .iter()
        .rev()
        .find(|item| item.item_id == item_id)
        .expect("cat-task item must have been retried");
    let last_json = serde_json::to_string(cat_attempt).unwrap();
    assert!(
        last_json.contains(new_url),
        "expected {new_url} in {last_json}"
    );
    drop(attempts);
    service.shutdown().unwrap();
}

#[test]
fn per_scene_item_actions_pause_resume_cancel_retry_preserve_siblings() {
    let ws = TempWorkspace::new("item-actions");
    let worker = Arc::new(SelectiveWorker::default());
    *worker.complete_urls.lock().unwrap() = vec!["https://ok/scene-1".to_owned()];
    *worker.block_item.lock().unwrap() = Some("item-1".to_owned());

    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(ws.db_path()),
        Arc::new(UnusedAsf),
        Arc::new(ReplayCatalog {
            url: Mutex::new("https://unused".to_owned()),
            searches: AtomicU64::new(0),
        }),
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
    );

    {
        let mut store = TaskStore::open(ws.db_path()).unwrap();
        let batch: insar_core::FrozenSearchBatch = serde_json::from_value(json!({
            "schema_version": 1,
            "batch_id": "multi-batch",
            "provider_id": "copernicus.dataspace",
            "created_at": "2026-07-25T00:00:00Z",
            "aoi_name": "Multi",
            "aoi_geojson": aoi(),
            "query": {"custom_only": true},
            "search_stats": null,
            "candidates": [
                {
                    "item_id": "item-1",
                    "provider_id": "copernicus.dataspace",
                    "display_name": "scene-1.zip",
                    "acquisition_datetime": null,
                    "footprint_geojson": null,
                    "metadata": {},
                    "payload": {"download_url": "https://ok/scene-1"}
                },
                {
                    "item_id": "item-2",
                    "provider_id": "copernicus.dataspace",
                    "display_name": "scene-2.zip",
                    "acquisition_datetime": null,
                    "footprint_geojson": null,
                    "metadata": {},
                    "payload": {"download_url": "https://ok/scene-2"}
                }
            ]
        }))
        .unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                workbench: SearchWorkbenchState {
                    batch_id: "multi-batch".to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                batch,
                activate_workbench: false,
            })
            .unwrap();
    }

    service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: "multi-task".to_owned(),
            batch_id: "multi-batch".to_owned(),
            selected_item_ids: vec!["item-1".to_owned(), "item-2".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: ws.out_dir(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-25T00:00:01Z".to_owned(),
        })
        .unwrap();

    // Wait until item-1 enters the worker and blocks.
    wait_until(|| !worker.attempts.lock().unwrap().is_empty());

    // Pause item-2 while queued -> item-1 can then complete cleanly without aborting it.
    service
        .task_item_action(
            "multi-task".to_owned(),
            None,
            "item-2".to_owned(),
            UnifiedTaskItemAction::Pause,
        )
        .unwrap();

    // Release item-1 so it completes while item-2 stays paused!
    *worker.block_item.lock().unwrap() = None;
    wait_until(|| {
        let store = TaskStore::open(ws.db_path()).unwrap();
        store
            .load_task_record("multi-task")
            .unwrap()
            .is_some_and(|r| r.runtime.completed_item_ids == vec!["item-1".to_owned()])
    });

    // Resume item-2 selectively; since its URL fails, it becomes failed & task terminates.
    service
        .task_item_action(
            "multi-task".to_owned(),
            None,
            "item-2".to_owned(),
            UnifiedTaskItemAction::Resume,
        )
        .unwrap();
    wait_until(|| {
        let store = TaskStore::open(ws.db_path()).unwrap();
        store
            .load_task_record("multi-task")
            .unwrap()
            .is_some_and(|r| r.runtime.status == TaskLifecycleStatus::Failed)
    });

    // Archive task to simulate History view, then retry ONLY failed item-2 with valid URL.
    let rev = TaskStore::open(ws.db_path())
        .unwrap()
        .load_task_record("multi-task")
        .unwrap()
        .unwrap()
        .runtime
        .revision;
    service
        .archive_download_task(&insar_core::StoreArchiveTaskRequest {
            task_id: "multi-task".to_owned(),
            expected_revision: rev,
            archived_at: "2026-07-25T00:00:10Z".to_owned(),
        })
        .unwrap();

    worker
        .complete_urls
        .lock()
        .unwrap()
        .push("https://ok/scene-2".to_owned());
    let retried_ref = service
        .task_item_action(
            "multi-task".to_owned(),
            None,
            "item-2".to_owned(),
            UnifiedTaskItemAction::Retry,
        )
        .unwrap();
    assert!(matches!(
        retried_ref.status,
        TaskLifecycleStatus::Queued | TaskLifecycleStatus::Running | TaskLifecycleStatus::Completed
    ));

    wait_until(|| {
        let store = TaskStore::open(ws.db_path()).unwrap();
        store
            .load_task_record("multi-task")
            .unwrap()
            .is_some_and(|r| r.runtime.status == TaskLifecycleStatus::Completed)
    });

    service.shutdown().unwrap();
}

#[test]
fn single_history_delete_pins_and_existing_disk_duplicate_detection_work() {
    let ws = TempWorkspace::new("history-pin-dedupe");
    let worker = Arc::new(SelectiveWorker::default());
    *worker.complete_urls.lock().unwrap() = vec!["https://ok/a".to_owned()];

    let service = ApplicationService::with_catalogue_fixture_business_runtime(
        ApplicationServiceConfig::business_store(ws.db_path()),
        Arc::new(UnusedAsf),
        Arc::new(ReplayCatalog {
            url: Mutex::new("https://unused".to_owned()),
            searches: AtomicU64::new(0),
        }),
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
    );

    {
        let mut store = TaskStore::open(ws.db_path()).unwrap();
        let batch: insar_core::FrozenSearchBatch = serde_json::from_value(json!({
            "schema_version": 1,
            "batch_id": "pin-batch",
            "provider_id": "asf.catalog",
            "created_at": "2026-07-25T00:00:00Z",
            "aoi_name": "Pin AOI",
            "aoi_geojson": aoi(),
            "query": {"custom_only": true},
            "search_stats": null,
            "candidates": [{
                "item_id": "scene-a",
                "provider_id": "asf.catalog",
                "display_name": "SCENE_A.zip",
                "acquisition_datetime": null,
                "footprint_geojson": null,
                "metadata": {},
                "payload": {"download_url": "https://ok/a"}
            }]
        }))
        .unwrap();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                workbench: SearchWorkbenchState {
                    batch_id: "pin-batch".to_owned(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                batch,
                activate_workbench: false,
            })
            .unwrap();
        // Active task cannot be deleted from History.
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: "active-task".to_owned(),
                batch_id: "pin-batch".to_owned(),
                selected_item_ids: vec!["scene-a".to_owned()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: ws.out_dir(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-07-25T00:00:01Z".to_owned(),
            })
            .unwrap();
    }

    let active_err = service
        .task_delete_history("active-task".to_owned())
        .expect_err("active task cannot be deleted");
    assert_eq!(active_err.code, "TASK_DELETE_HISTORY_ACTIVE");

    // Create a completed task producing a file on disk inside ws.out_dir().
    service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: "done-task".to_owned(),
            batch_id: "pin-batch".to_owned(),
            selected_item_ids: vec!["scene-a".to_owned()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: ws.out_dir(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-25T00:00:02Z".to_owned(),
        })
        .unwrap();

    wait_until(|| {
        let store = TaskStore::open(ws.db_path()).unwrap();
        store
            .load_task_record("done-task")
            .unwrap()
            .is_some_and(|r| r.runtime.status == TaskLifecycleStatus::Completed)
    });

    // Pin order & unpin persistence.
    let pins = service
        .set_task_pinned("done-task".to_owned(), true)
        .unwrap();
    assert_eq!(pins, vec!["done-task".to_owned()]);
    assert_eq!(
        service.list_pinned_tasks().unwrap(),
        vec!["done-task".to_owned()]
    );

    // Duplicate detection finds completed Store destination even when target_dir is another folder,
    // AND finds another pre-existing file directly inside an alternate target_dir.
    let alt_dir = ws.0.join("alt-target");
    fs::create_dir_all(&alt_dir).unwrap();
    fs::write(alt_dir.join("LOCAL_ONLY.tif"), b"raster").unwrap();

    let matches = service
        .check_existing_downloaded_scenes(
            vec![
                ExistingDownloadCheckItem {
                    scene_id: "scene-a".to_owned(),
                    file_name: "SCENE_A.zip".to_owned(),
                    target_relpath: None,
                },
                ExistingDownloadCheckItem {
                    scene_id: "scene-local".to_owned(),
                    file_name: "LOCAL_ONLY.tif".to_owned(),
                    target_relpath: None,
                },
                ExistingDownloadCheckItem {
                    scene_id: "scene-missing".to_owned(),
                    file_name: "MISSING.zip".to_owned(),
                    target_relpath: None,
                },
            ],
            Some(alt_dir.to_string_lossy().into_owned()),
        )
        .unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].scene_id, "scene-a");
    assert_eq!(matches[1].scene_id, "scene-local");

    // Traversal in target_relpath or file_name must fail closed.
    assert!(service
        .check_existing_downloaded_scenes(
            vec![ExistingDownloadCheckItem {
                scene_id: "bad".to_owned(),
                file_name: "../secret".to_owned(),
                target_relpath: None,
            }],
            Some(alt_dir.to_string_lossy().into_owned()),
        )
        .is_err());

    // Delete single unarchived completed task from History -> preserves files on disk & cleans pin.
    let delivered_file = ws.out_dir().join("SCENE_A.zip");
    assert!(delivered_file.is_file());
    service.task_delete_history("done-task".to_owned()).unwrap();
    assert!(
        delivered_file.is_file(),
        "deleting history record must never delete downloaded files"
    );
    assert!(service.list_pinned_tasks().unwrap().is_empty());

    // Idempotent on second call.
    service.task_delete_history("done-task".to_owned()).unwrap();

    service.shutdown().unwrap();
}
