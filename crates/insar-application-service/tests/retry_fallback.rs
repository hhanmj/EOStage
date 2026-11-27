use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use insar_application_service::{ApplicationService, ApplicationServiceConfig};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, ContractError, DownloadFailure,
    DownloadItemProgress, DownloadItemResult, DownloadLayout, DownloadOutcome, ProviderResult,
    SarRemoteSearchRequest, SarRemoteSearchResponse, SearchWorkbenchState, SnapshotItem,
    SourceView, StoreCreateTaskRequest, StoreSearchBatchRequest, TaskLifecycleStatus,
};
use insar_download::{DownloadWork, DownloadWorker, WorkerControl};
use insar_service::LiveAsfSearch;
use insar_store::TaskStore;
use serde_json::{json, Map, Value};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TempStore(PathBuf);
impl TempStore {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "retry-fallback-{}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for TempStore {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

/// ASF answers with whatever products (id, URL) the test currently sets, or
/// with the failure it sets: "network" (transport error) or "partial" (an
/// incomplete answer that also reports errors).
struct Catalog {
    products: Mutex<Vec<(&'static str, &'static str)>>,
    searches: AtomicU64,
    failure: Mutex<Option<&'static str>>,
}
impl LiveAsfSearch for Catalog {
    fn search(
        &self,
        _: &AsfRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("DUMMY", "unused", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        _: &SarRemoteSearchRequest,
        _: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        let failure = *self.failure.lock().unwrap();
        if failure == Some("network") {
            return Err(
                ContractError::new("CATALOGUE_TRANSPORT_FAILURE", "fixture", Map::new()).unwrap(),
            );
        }
        let errors = if failure == Some("partial") {
            vec![serde_json::to_value(
                ContractError::new("ASF_PAGE_FAILED", "fixture page", Map::new()).unwrap(),
            )
            .unwrap()]
        } else {
            vec![]
        };
        let products = self
            .products
            .lock()
            .unwrap()
            .iter()
            .map(|(id, url)| {
                json!({
                    "product_key": key(id),
                    "provider_product_key": key(id),
                    "provider_product_id": id,
                    "product_id": id,
                    "provider": "asf.catalog",
                    "dataset": "SENTINEL-1",
                    "product_family": "SLC",
                    "processing_level": "SLC",
                    "platform": "S1A",
                    "acquisition_start": "2026-07-25T00:00:00Z",
                    "file_name": format!("{id}.zip"),
                    "file_size_remote": 8,
                    "download_url": url
                })
            })
            .collect::<Vec<_>>();
        let count = products.len();
        Ok(serde_json::from_value(json!({
            "products": products,
            "errors": errors,
            "stats": {
                "requested_limit": 10,
                "query_limit": 10,
                "candidate_count": count,
                "returned_count": count,
                "source": "retry-fallback-fixture"
            }
        }))
        .unwrap())
    }
}

fn key(id: &str) -> String {
    format!("PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::{id}")
}

/// Every download fails unless its product is listed in `complete`; the
/// frozen item each attempt used is recorded.
#[derive(Default)]
struct FailingWorker {
    attempts: Mutex<Vec<SnapshotItem>>,
    complete: Mutex<Vec<String>>,
}
impl DownloadWorker for FailingWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _: WorkerControl,
        _: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.attempts.lock().unwrap().push(work.item.clone());
        if self.complete.lock().unwrap().contains(&work.item.item_id) {
            let destination = work.task.output_root.join(&work.item.display_name);
            std::fs::create_dir_all(&work.task.output_root).unwrap();
            std::fs::write(&destination, b"8 bytes!").unwrap();
            return DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id.clone(),
                outcome: DownloadOutcome::Completed,
                destination,
                bytes_written: 8,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-07-25T00:00:05Z".to_owned(),
            };
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome: DownloadOutcome::Failed,
            destination: work.task.output_root.join(&work.item.display_name),
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: "DL002".to_owned(),
                reason_zh: "离线夹具下载失败".to_owned(),
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

struct Fixture {
    db: TempStore,
    catalog: Arc<Catalog>,
    worker: Arc<FailingWorker>,
    service: ApplicationService,
}

/// A failed task with products A and B, frozen from batch `orig`.
fn failed_task() -> Fixture {
    failed_task_completing(&[])
}

/// Like `failed_task`, but the listed products download successfully.
fn failed_task_completing(complete: &[&str]) -> Fixture {
    let db = TempStore::new();
    let catalog = Arc::new(Catalog {
        products: Mutex::new(vec![("S1-A", "https://old/a"), ("S1-B", "https://old/b")]),
        searches: AtomicU64::new(0),
        failure: Mutex::new(None),
    });
    let worker = Arc::new(FailingWorker::default());
    *worker.complete.lock().unwrap() = complete.iter().map(|id| key(id)).collect();
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&db.0),
        Arc::clone(&catalog) as Arc<dyn LiveAsfSearch>,
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
    );
    service
        .sar_search_and_freeze(
            &serde_json::from_value(json!({
                "request_id": "orig-search",
                "search": {"query": {"provider": "asf", "dataset": "SENTINEL-1",
                                     "aoi_geojson": aoi(), "max_results": 10}},
                "batch_id": "orig",
                "created_at": "2026-07-25T00:00:00Z",
                "aoi_name": "Fallback AOI",
                "aoi_geojson": aoi(),
                "page_size": 24
            }))
            .unwrap(),
        )
        .unwrap();
    service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: "t".to_owned(),
            batch_id: "orig".to_owned(),
            selected_item_ids: vec![key("S1-A"), key("S1-B")],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: std::env::temp_dir().join("retry-fallback-output"),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-25T00:00:02Z".to_owned(),
        })
        .unwrap();
    wait_status(&db, TaskLifecycleStatus::Failed);
    Fixture {
        db,
        catalog,
        worker,
        service,
    }
}

fn wait_status(db: &TempStore, status: TaskLifecycleStatus) {
    for _ in 0..300 {
        let store = TaskStore::open(&db.0).unwrap();
        if store.load_task_record("t").unwrap().unwrap().runtime.status == status {
            return;
        }
        drop(store);
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("task never reached {status:?}");
}

fn url(item: &SnapshotItem) -> String {
    serde_json::to_string(item).unwrap()
}

#[test]
fn unchanged_products_rebuild_in_the_same_task_from_a_fresh_frozen_batch() {
    let fixture = failed_task();
    *fixture.catalog.products.lock().unwrap() =
        vec![("S1-A", "https://new/a"), ("S1-B", "https://new/b")];
    let attempts_before = fixture.worker.attempts.lock().unwrap().len();
    let outcome = fixture
        .service
        .task_retry_fallback("t".to_owned(), None)
        .expect("rebuilt in place");
    assert!(!outcome.needs_user_confirmation);
    assert_eq!(outcome.task.task_id, "t");
    assert!(!outcome.task.created);
    assert_eq!(outcome.rebuilt_item_ids, [key("S1-A"), key("S1-B")]);
    assert!(outcome.missing_product_ids.is_empty());
    assert_ne!(outcome.refreshed_batch_id, "orig");
    // The scheduler retries the same task with the refreshed frozen items.
    for _ in 0..300 {
        if fixture.worker.attempts.lock().unwrap().len() >= attempts_before + 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let store = TaskStore::open(&fixture.db.0).unwrap();
    let refreshed = store
        .load_search_batch(&outcome.refreshed_batch_id)
        .unwrap()
        .expect("refreshed batch is frozen");
    let retried = fixture.worker.attempts.lock().unwrap()[attempts_before..].to_vec();
    assert_eq!(retried.len(), 2);
    for attempt in &retried {
        let fresh = refreshed
            .candidates
            .iter()
            .find(|item| item.item_id == attempt.item_id)
            .unwrap();
        assert_eq!(url(attempt), url(fresh));
        assert!(url(attempt).contains("https://new/"));
    }
    assert_eq!(
        store
            .load_task_record("t")
            .unwrap()
            .unwrap()
            .snapshot
            .batch_id,
        "orig"
    );
    assert_eq!(
        store.core_restore(false).unwrap().tasks.len(),
        1,
        "no second task is created"
    );
    assert!(store
        .load_search_batch("orig")
        .unwrap()
        .unwrap()
        .candidates
        .iter()
        .all(|item| url(item).contains("https://old/")));
    drop(store);
    fixture.service.shutdown().unwrap();
}

#[test]
fn missing_products_need_confirmation_bound_to_the_refreshed_batch() {
    let fixture = failed_task();
    *fixture.catalog.products.lock().unwrap() =
        vec![("S1-A", "https://new/a"), ("S1-C", "https://new/c")];
    let first = fixture
        .service
        .task_retry_fallback("t".to_owned(), None)
        .expect("comparison");
    assert!(first.needs_user_confirmation);
    assert_eq!(first.missing_product_ids, [key("S1-B")]);
    assert_eq!(first.added_product_ids, [key("S1-C")]);
    assert!(first.rebuilt_item_ids.is_empty());
    assert_eq!(first.task.status, TaskLifecycleStatus::Failed);
    let store = TaskStore::open(&fixture.db.0).unwrap();
    assert_eq!(store.task_snapshot_revision("t").unwrap(), 0, "no mutation");
    drop(store);
    let searches = fixture.catalog.searches.load(Ordering::SeqCst);
    let confirmed = fixture
        .service
        .task_retry_fallback("t".to_owned(), Some(first.refreshed_batch_id.clone()))
        .expect("confirmed rebuild");
    assert_eq!(
        fixture.catalog.searches.load(Ordering::SeqCst),
        searches,
        "confirmation reuses the frozen batch, no second search"
    );
    assert!(!confirmed.needs_user_confirmation);
    assert_eq!(confirmed.rebuilt_item_ids, [key("S1-A")]);
    assert_eq!(confirmed.task.task_id, "t");
    let store = TaskStore::open(&fixture.db.0).unwrap();
    assert_eq!(store.task_snapshot_revision("t").unwrap(), 1);
    let task = store.load_task_record("t").unwrap().unwrap();
    assert!(!task
        .snapshot
        .candidates
        .iter()
        .any(|item| item.item_id == key("S1-C")));
    drop(store);
    fixture.service.shutdown().unwrap();
}

fn search_views(fixture: &Fixture) -> Vec<SourceView> {
    fixture
        .service
        .source_views()
        .unwrap()
        .into_iter()
        .filter(|view| view.batch_id.is_some())
        .collect()
}

fn workbench_batch(fixture: &Fixture) -> String {
    TaskStore::open(&fixture.db.0)
        .unwrap()
        .load_workbench()
        .unwrap()
        .expect("legacy workbench slot")
        .state
        .batch_id
}

#[test]
fn fallback_and_derived_batches_leave_source_views_and_workbench_slot_unchanged() {
    let fixture = failed_task();
    let before = search_views(&fixture);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].batch_id.as_deref(), Some("orig"));
    assert_eq!(workbench_batch(&fixture), "orig");

    // Fallback that needs confirmation, then the confirmed rebuild.
    *fixture.catalog.products.lock().unwrap() =
        vec![("S1-A", "https://new/a"), ("S1-C", "https://new/c")];
    let first = fixture
        .service
        .task_retry_fallback("t".to_owned(), None)
        .unwrap();
    assert!(first.needs_user_confirmation);
    assert_eq!(search_views(&fixture), before, "offered fallback batch");
    fixture
        .service
        .task_retry_fallback("t".to_owned(), Some(first.refreshed_batch_id))
        .unwrap();
    assert_eq!(search_views(&fixture), before, "confirmed fallback batch");
    assert_eq!(workbench_batch(&fixture), "orig");

    // A derived delivery batch with the same route keys and a task on it.
    let mut store = TaskStore::open(&fixture.db.0).unwrap();
    let mut derived = store.load_search_batch("orig").unwrap().unwrap();
    derived.batch_id = "derived-delivery".to_owned();
    derived.created_at = "2099-01-01T00:00:00Z".to_owned();
    derived
        .query
        .insert("derived_from_batch_id".to_owned(), json!("orig"));
    store
        .save_search_batch_and_create_task(
            &StoreSearchBatchRequest {
                workbench: SearchWorkbenchState {
                    batch_id: derived.batch_id.clone(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
                batch: derived,
                activate_workbench: false,
            },
            &StoreCreateTaskRequest {
                task_id: "derived-task".to_owned(),
                batch_id: "derived-delivery".to_owned(),
                selected_item_ids: vec![key("S1-A")],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: std::env::temp_dir().join("retry-fallback-derived"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2099-01-01T00:00:01Z".to_owned(),
            },
        )
        .unwrap();
    drop(store);
    assert_eq!(search_views(&fixture), before, "derived delivery batch");
    assert_eq!(workbench_batch(&fixture), "orig");
    fixture.service.shutdown().unwrap();
}

#[test]
fn confirmation_must_name_the_latest_fallback_batch_for_this_failure() {
    let fixture = failed_task();
    *fixture.catalog.products.lock().unwrap() =
        vec![("S1-A", "https://new/a"), ("S1-C", "https://new/c")];
    let first = fixture
        .service
        .task_retry_fallback("t".to_owned(), None)
        .unwrap();
    let second = fixture
        .service
        .task_retry_fallback("t".to_owned(), None)
        .unwrap();
    assert!(first.needs_user_confirmation && second.needs_user_confirmation);
    assert_ne!(first.refreshed_batch_id, second.refreshed_batch_id);
    for stale in [first.refreshed_batch_id.clone(), "orig".to_owned()] {
        let error = fixture
            .service
            .task_retry_fallback("t".to_owned(), Some(stale.clone()))
            .expect_err("stale confirmation");
        assert_eq!(
            error.code, "RETRY_FALLBACK_CONFIRMATION_MISMATCH",
            "{stale}"
        );
    }
    let store = TaskStore::open(&fixture.db.0).unwrap();
    assert_eq!(store.task_snapshot_revision("t").unwrap(), 0, "no mutation");
    drop(store);
    let confirmed = fixture
        .service
        .task_retry_fallback("t".to_owned(), Some(second.refreshed_batch_id))
        .expect("latest offer confirms");
    assert_eq!(confirmed.rebuilt_item_ids, [key("S1-A")]);
    fixture.service.shutdown().unwrap();
}

#[test]
fn partially_completed_task_rebuilds_only_its_failed_products() {
    let fixture = failed_task_completing(&["S1-A"]);
    let store = TaskStore::open(&fixture.db.0).unwrap();
    let task = store.load_task_record("t").unwrap().unwrap();
    assert_eq!(task.runtime.completed_item_ids, [key("S1-A")]);
    drop(store);
    // The completed product is no longer returned; only S1-B is retried.
    *fixture.catalog.products.lock().unwrap() = vec![("S1-B", "https://new/b")];
    let outcome = fixture
        .service
        .task_retry_fallback("t".to_owned(), None)
        .expect("rebuilt without confirmation");
    assert!(!outcome.needs_user_confirmation);
    assert!(outcome.missing_product_ids.is_empty());
    assert_eq!(outcome.rebuilt_item_ids, [key("S1-B")]);
    fixture.service.shutdown().unwrap();
}

#[test]
fn failed_re_search_reports_fallback_errors_and_leaves_the_task() {
    let fixture = failed_task();
    *fixture.catalog.failure.lock().unwrap() = Some("network");
    let error = fixture
        .service
        .task_retry_fallback("t".to_owned(), None)
        .expect_err("transport failure");
    assert!(error.retryable, "{error:?}");
    assert!(
        !error.message_zh.contains("建任务"),
        "fallback wording, not task creation: {}",
        error.message_zh
    );
    assert!(
        error.message_zh.contains("原任务未改动"),
        "{}",
        error.message_zh
    );
    *fixture.catalog.failure.lock().unwrap() = Some("partial");
    let error = fixture
        .service
        .task_retry_fallback("t".to_owned(), None)
        .expect_err("incomplete answer");
    assert_eq!(error.code, "RETRY_FALLBACK_SEARCH_INCOMPLETE");
    assert!(error.retryable);
    let store = TaskStore::open(&fixture.db.0).unwrap();
    assert_eq!(store.task_snapshot_revision("t").unwrap(), 0);
    assert_eq!(store.retry_fallback_offer("t").unwrap(), None);
    assert_eq!(
        store.load_task_record("t").unwrap().unwrap().runtime.status,
        TaskLifecycleStatus::Failed
    );
    drop(store);
    fixture.service.shutdown().unwrap();
}

fn plain_batch(batch_id: &str, provider_id: &str) -> insar_core::FrozenSearchBatch {
    serde_json::from_value(json!({
        "schema_version": 1,
        "batch_id": batch_id,
        "provider_id": provider_id,
        "created_at": "2026-07-25T00:00:00Z",
        "aoi_name": "Fallback AOI",
        "aoi_geojson": aoi(),
        "query": {"provider": "asf", "dataset": "SENTINEL-1"},
        "search_stats": null,
        "candidates": [{
            "item_id": "P-1",
            "provider_id": provider_id,
            "display_name": "P-1",
            "acquisition_datetime": null,
            "footprint_geojson": null,
            "metadata": {},
            "payload": {"download_url": "https://old/p1"}
        }]
    }))
    .unwrap()
}

#[test]
fn only_failed_asf_product_tasks_can_use_the_fallback() {
    let db = TempStore::new();
    {
        let mut store = TaskStore::open(&db.0).unwrap();
        for (batch_id, provider_id, task_id) in [
            ("asf-batch", "asf.catalog", "queued-task"),
            ("mpc-batch", "microsoft.planetary-computer", "mpc-task"),
        ] {
            store
                .save_search_batch(&StoreSearchBatchRequest {
                    workbench: SearchWorkbenchState {
                        batch_id: batch_id.to_owned(),
                        checked_item_ids: vec![],
                        page_index: 0,
                        page_size: 24,
                        revision: 0,
                    },
                    batch: plain_batch(batch_id, provider_id),
                    activate_workbench: false,
                })
                .unwrap();
            store
                .create_task(&StoreCreateTaskRequest {
                    task_id: task_id.to_owned(),
                    batch_id: batch_id.to_owned(),
                    selected_item_ids: vec!["P-1".to_owned()],
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    output_root: std::env::temp_dir().join("retry-fallback-scope"),
                    layout: DownloadLayout::Direct,
                    concurrency: 1,
                    created_at: "2026-07-25T00:00:01Z".to_owned(),
                })
                .unwrap();
        }
        store
            .update_task(&insar_core::StoreUpdateTaskRequest {
                task_id: "mpc-task".to_owned(),
                expected_revision: 0,
                command: insar_core::StoreTaskCommand {
                    kind: insar_core::TaskStateCommandKind::MarkFailed,
                    item_ids: vec!["P-1".to_owned()],
                },
                event_id: "fail-mpc".to_owned(),
                occurred_at: "2026-07-25T00:00:02Z".to_owned(),
            })
            .unwrap();
    }
    // No business runtime: neither case may reach a provider or the scheduler.
    let service = ApplicationService::new(ApplicationServiceConfig::store(&db.0));
    let not_failed = service
        .task_retry_fallback("queued-task".to_owned(), None)
        .expect_err("queued task");
    assert_eq!(not_failed.code, "RETRY_FALLBACK_TASK_NOT_FAILED");
    let unsupported = service
        .task_retry_fallback("mpc-task".to_owned(), None)
        .expect_err("catalogue task");
    assert_eq!(unsupported.code, "RETRY_FALLBACK_UNSUPPORTED");
    for error in [&not_failed, &unsupported] {
        assert!(!error.retryable);
        assert!(!error.message_zh.contains("Store"), "{}", error.message_zh);
    }
    service.shutdown().unwrap();
}
