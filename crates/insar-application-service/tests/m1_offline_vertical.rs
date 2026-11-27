use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use insar_application_service::{
    canonical_restore_value, ApplicationBindRegionAoiRequest, ApplicationSarSearchAndFreezeRequest,
    ApplicationService, ApplicationServiceConfig, SarDownloadPolicyMode, SarDownloadPolicySettings,
};
use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, ContractError, DownloadItemProgress,
    DownloadItemResult, DownloadItemStatus, DownloadLayout, DownloadOutcome, ProviderResult,
    SarRemoteSearchRequest, SarRemoteSearchResponse, SearchWorkbenchState, SidecarOperation,
    SidecarRequest, StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreDeleteHistoryRequest,
    StoreTaskCommand, StoreUpdateTaskRequest, StoreUpdateWorkbenchRequest, TaskLifecycleStatus,
    TaskStateCommandKind,
};
use insar_download::{
    part_path, DownloadWork, DownloadWorker, HttpTransfer, TransferAuth, TransferBackend,
    TransferOptions, TransferRequest, TransferUpdate, WorkerControl,
};
use insar_service::{Dispatcher, LiveAsfSearch};
use serde_json::{json, Map, Value};

mod common;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct TempRoot {
    root: PathBuf,
    database: PathBuf,
}

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-m1-offline-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("tasks.sqlite3");
        Self { root, database }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct LoopbackServer {
    url: String,
    worker: Option<thread::JoinHandle<()>>,
}

impl LoopbackServer {
    fn one(body: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
            stream.flush().unwrap();
        });
        Self {
            url: format!("http://{address}/S1-M1-LOOPBACK.bin"),
            worker: Some(worker),
        }
    }

    fn finish(mut self) {
        self.worker.take().unwrap().join().unwrap();
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            return request;
        }
        request.push_str(&line);
    }
}

struct FixtureSearch {
    url: String,
}

impl FixtureSearch {
    fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

impl LiveAsfSearch for FixtureSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        Err(ContractError::new("FIXTURE_ONLY", "fixture only", Map::new()).unwrap())
    }

    fn search_sar(
        &self,
        _request: &SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        Ok(serde_json::from_value(json!({
            "products": [{
                "product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-M1-LOOPBACK",
                "provider_product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-M1-LOOPBACK",
                "provider_product_id": "S1-M1-LOOPBACK",
                "product_id": "S1-M1-LOOPBACK",
                "provider": "asf.catalog",
                "dataset": "SENTINEL-1",
                "product_family": "SLC",
                "processing_level": "SLC",
                "platform": "S1A",
                "acquisition_start": "2026-07-25T00:00:00Z",
                "file_name": "S1-M1-LOOPBACK.bin",
                "file_size_remote": 8,
                "download_url": self.url
            }, {
                "product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-M1-RETRY",
                "provider_product_key": "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-M1-RETRY",
                "provider_product_id": "S1-M1-RETRY",
                "product_id": "S1-M1-RETRY",
                "provider": "asf.catalog",
                "dataset": "SENTINEL-1",
                "product_family": "SLC",
                "processing_level": "SLC",
                "platform": "S1A",
                "acquisition_start": "2026-07-25T00:01:00Z",
                "file_name": "S1-M1-RETRY.bin",
                "file_size_remote": 8,
                "download_url": self.url
            }],
            "errors": [],
            "stats": {
                "requested_limit": 1,
                "query_limit": 1,
                "candidate_count": 2,
                "returned_count": 2,
                "source": "m1-loopback-fixture"
            }
        }))
        .unwrap())
    }
}

/// Answers the first SAR search with one fetched record that could not be
/// mapped, then with the normal fixture products.
struct MappingFailureThenSuccessSearch {
    fixture: FixtureSearch,
    calls: AtomicUsize,
}

impl LiveAsfSearch for MappingFailureThenSuccessSearch {
    fn search(
        &self,
        request: &AsfRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        self.fixture.search(request, cancelled)
    }

    fn search_sar(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            return self.fixture.search_sar(request, cancelled);
        }
        Ok(serde_json::from_value(json!({
            "products": [],
            "errors": [],
            "stats": {
                "requested_limit": 1,
                "query_limit": 1,
                "candidate_count": 0,
                "returned_count": 0,
                "source": "m1-loopback-fixture",
                "fetched_record_count": 1,
                "replayed_record_count": 0,
                "mapped_record_count": 0,
                "mapping_error_count": 1,
                "aggregated_asset_record_count": 0,
                "unique_provider_product_count": 0,
                "asset_count": 0,
                "identity_conflict_count": 0,
                "truncated": false,
                "item_errors": [{
                    "code": "ASF_PRODUCT_SCOPE_MISMATCH",
                    "user_message": "fixture record could not be mapped",
                    "debug_context": {"field": "look_direction", "reason": "missing"}
                }]
            }
        }))
        .unwrap())
    }
}

#[derive(Default)]
struct ControlledWorker {
    starts: AtomicUsize,
    paused_seen: AtomicBool,
    release: AtomicBool,
    fail_next: AtomicBool,
}

impl ControlledWorker {
    fn complete_after_retry() -> Self {
        Self {
            starts: AtomicUsize::new(0),
            paused_seen: AtomicBool::new(false),
            release: AtomicBool::new(false),
            fail_next: AtomicBool::new(false),
        }
    }
}

impl DownloadWorker for ControlledWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return result(&work, DownloadOutcome::Failed);
        }
        loop {
            if control.is_cancelled() {
                return result(&work, DownloadOutcome::Cancelled);
            }
            if control.is_paused() {
                self.paused_seen.store(true, Ordering::SeqCst);
                progress(DownloadItemProgress {
                    task_id: work.task.task_id.clone(),
                    item_id: work.item.item_id.clone(),
                    status: DownloadItemStatus::Paused,
                    bytes_completed: 2,
                    total_bytes: Some(8),
                    bytes_per_second: 0.0,
                    attempt: 1,
                    updated_at: "2026-07-25T00:00:02Z".to_owned(),
                });
            } else {
                progress(DownloadItemProgress {
                    task_id: work.task.task_id.clone(),
                    item_id: work.item.item_id.clone(),
                    status: DownloadItemStatus::Active,
                    bytes_completed: 2,
                    total_bytes: Some(8),
                    bytes_per_second: 2.0,
                    attempt: 1,
                    updated_at: "2026-07-25T00:00:02Z".to_owned(),
                });
                if self.release.load(Ordering::SeqCst) {
                    return result(&work, DownloadOutcome::Completed);
                }
            }
            thread::sleep(Duration::from_millis(3));
        }
    }
}

struct LoopbackHttpWorker {
    transfer: HttpTransfer,
}

impl LoopbackHttpWorker {
    fn new() -> Self {
        Self {
            transfer: HttpTransfer::new(TransferOptions {
                max_attempts: 1,
                backoff: Duration::ZERO,
                timeout: Duration::from_secs(2),
                chunk_size: 2,
                ..TransferOptions::default()
            })
            .unwrap(),
        }
    }
}

impl DownloadWorker for LoopbackHttpWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let url = work
            .item
            .payload
            .get("download_url")
            .and_then(Value::as_str)
            .unwrap();
        let file_name = work
            .item
            .payload
            .get("file_name")
            .and_then(Value::as_str)
            .unwrap();
        let expected_size = work
            .item
            .payload
            .get("file_size_remote")
            .and_then(Value::as_u64);
        let destination = work.task.output_root.join(file_name);
        let request = TransferRequest::new(
            work.task.task_id.clone(),
            work.item.item_id.clone(),
            url,
            destination.clone(),
            expected_size,
        )
        .unwrap();
        let transfer = self.transfer.download_legacy(
            &request,
            &TransferAuth::None,
            &|| control.is_cancelled(),
            &|| control.is_paused(),
            &mut |value| {
                let TransferUpdate::Progress(value) = value else {
                    return;
                };
                progress(DownloadItemProgress {
                    task_id: work.task.task_id.clone(),
                    item_id: work.item.item_id.clone(),
                    status: if control.is_paused() {
                        DownloadItemStatus::Paused
                    } else {
                        DownloadItemStatus::Active
                    },
                    bytes_completed: value.bytes_completed,
                    total_bytes: value.total_bytes,
                    bytes_per_second: 1.0,
                    attempt: u64::from(value.attempt),
                    updated_at: "2026-07-25T00:00:02Z".to_owned(),
                });
            },
        );
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: transfer.outcome,
            destination,
            bytes_written: transfer.bytes_written,
            attempt_count: u64::from(transfer.attempt_count),
            failure: transfer.failure,
            finished_at: "2026-07-25T00:00:03Z".to_owned(),
        }
    }
}

fn result(work: &DownloadWork, outcome: DownloadOutcome) -> DownloadItemResult {
    DownloadItemResult {
        task_id: work.task.task_id.clone(),
        item_id: work.item.item_id.clone(),
        outcome,
        destination: work.task.output_root.join("fixture.bin"),
        bytes_written: 8,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-07-25T00:00:03Z".to_owned(),
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

fn bind_aoi(service: &ApplicationService, label: &str) {
    let region_id = common::ensure_fixture_region(service, label);
    let request = ApplicationBindRegionAoiRequest {
        request_id: format!("{label}-bind-aoi"),
        region_id,
        expected_revision: None,
        source_kind: "manual_bbox".to_owned(),
        geometry: aoi(),
        bbox: vec![100.0, 30.0, 101.0, 31.0],
        crs: "EPSG:4326".to_owned(),
        display_name: "M1 fixture AOI".to_owned(),
        bound_at: "2026-07-25T00:00:00Z".to_owned(),
    };
    service.bind_region_aoi(&request).unwrap();
}

fn search_request(batch_id: &str) -> ApplicationSarSearchAndFreezeRequest {
    let aoi_geojson = aoi();
    ApplicationSarSearchAndFreezeRequest {
        request_id: format!("m1-offline-search-{batch_id}"),
        search: serde_json::from_value(json!({
            "query": {
                "provider": "asf",
                "dataset": "SENTINEL-1",
                "aoi_geojson": aoi_geojson,
                "max_results": 1
            }
        }))
        .unwrap(),
        batch_id: batch_id.to_owned(),
        created_at: "2026-07-25T00:00:00Z".to_owned(),
        aoi_name: "M1 fixture AOI".to_owned(),
        aoi_geojson: Some(aoi_geojson),
        page_size: 24,
    }
}

fn freeze_and_select(service: &ApplicationService, batch_id: &str) -> String {
    bind_aoi(service, batch_id);
    let frozen = service
        .sar_search_and_freeze_legacy(&search_request(batch_id))
        .unwrap();
    let batch = frozen.restore.workbench_batch.unwrap();
    assert_eq!(batch.batch_id, batch_id);
    assert_eq!(batch.candidates.len(), 2);
    let item_id = batch.candidates[0].item_id.clone();
    assert_eq!(
        item_id,
        "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-M1-LOOPBACK"
    );
    service
        .update_search_workbench(&StoreUpdateWorkbenchRequest {
            state: SearchWorkbenchState {
                batch_id: batch_id.to_owned(),
                checked_item_ids: vec![item_id.clone()],
                page_index: 2,
                page_size: 24,
                revision: 1,
            },
            expected_revision: 0,
            updated_at: "2026-07-25T00:00:01Z".to_owned(),
        })
        .unwrap();
    item_id
}

fn create_task(
    service: &ApplicationService,
    task_id: &str,
    batch_id: &str,
    item_id: &str,
    output: PathBuf,
) {
    create_task_with_items(service, task_id, batch_id, vec![item_id.to_owned()], output);
}

fn create_task_with_items(
    service: &ApplicationService,
    task_id: &str,
    batch_id: &str,
    item_ids: Vec<String>,
    output: PathBuf,
) {
    service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id: batch_id.to_owned(),
            selected_item_ids: item_ids.clone(),
            checked_item_ids: item_ids,
            page_index: 2,
            page_size: 24,
            output_root: output,
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-07-25T00:00:02Z".to_owned(),
        })
        .unwrap();
}

fn create_task_with_download_policy(
    service: &ApplicationService,
    task_id: &str,
    batch_id: &str,
    item_id: &str,
    output: PathBuf,
    policy: &SarDownloadPolicySettings,
) {
    service
        .create_download_task_legacy(&StoreCreateTaskRequest {
            task_id: task_id.to_owned(),
            batch_id: batch_id.to_owned(),
            selected_item_ids: vec![item_id.to_owned()],
            checked_item_ids: vec![item_id.to_owned()],
            page_index: 0,
            page_size: 24,
            output_root: output,
            layout: if policy.organize_by_product {
                DownloadLayout::SarDatasetProcessing
            } else {
                DownloadLayout::Direct
            },
            concurrency: policy.max_concurrent_items,
            created_at: "2026-08-24T00:00:00Z".to_owned(),
        })
        .unwrap();
}

fn runtime(service: &ApplicationService, task_id: &str) -> insar_core::TaskRuntimeSnapshot {
    service
        .snapshot(true)
        .unwrap()
        .restore
        .runtimes
        .into_iter()
        .find(|value| value.task_id == task_id)
        .unwrap()
}

fn wait_until(label: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for {label}");
}

fn update_task(
    service: &ApplicationService,
    task_id: &str,
    kind: TaskStateCommandKind,
    item_ids: Vec<String>,
) {
    for _ in 0..20 {
        let expected_revision = runtime(service, task_id).revision;
        let request = StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision,
            command: StoreTaskCommand {
                kind: kind.clone(),
                item_ids: item_ids.clone(),
            },
            event_id: format!("m1-offline-{task_id}-{expected_revision}"),
            occurred_at: "2026-07-25T00:00:03Z".to_owned(),
        };
        match service.update_download_task(&request) {
            Ok(_) => return,
            Err(error) if error.code() == "V3_STORE_REVISION_CONFLICT" => {
                thread::sleep(Duration::from_millis(3));
            }
            Err(error) => panic!("task update failed: {error:?}"),
        }
    }
    panic!("task update remained revision-conflicted");
}

#[test]
fn loopback_search_to_history_freezes_identity_finalizes_output_and_preserves_delete() {
    let temp = TempRoot::new("http");
    let server = LoopbackServer::one(b"m1-bytes");
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(FixtureSearch::new(server.url.clone())),
        Arc::new(LoopbackHttpWorker::new()),
    );
    let item_id = freeze_and_select(&service, "m1-http-batch");
    let output = temp.root.join("output");
    create_task(
        &service,
        "m1-http-task",
        "m1-http-batch",
        &item_id,
        output.clone(),
    );

    wait_until("loopback worker result", || {
        service
            .download_snapshot("m1-http-task")
            .map(|snapshot| snapshot.results.len() == 1)
            .unwrap_or(false)
    });
    let engine = service.download_snapshot("m1-http-task").unwrap();
    assert_eq!(
        engine.results[0].outcome,
        DownloadOutcome::Completed,
        "loopback result: {:?}",
        engine.results[0].failure
    );
    assert_eq!(engine.results[0].bytes_written, 8);
    let final_output = output.join("S1-M1-LOOPBACK.bin");
    assert_eq!(fs::read(&final_output).unwrap(), b"m1-bytes");
    assert!(!part_path(&final_output).exists());

    wait_until("terminal runtime", || {
        runtime(&service, "m1-http-task").status == TaskLifecycleStatus::Completed
    });
    let terminal = runtime(&service, "m1-http-task");
    service
        .archive_download_task(&StoreArchiveTaskRequest {
            task_id: "m1-http-task".to_owned(),
            expected_revision: terminal.revision,
            archived_at: "2026-07-25T00:00:04Z".to_owned(),
        })
        .unwrap();
    service
        .delete_download_history(&StoreDeleteHistoryRequest {
            task_id: "m1-http-task".to_owned(),
            deleted_at: "2026-07-25T00:00:05Z".to_owned(),
        })
        .unwrap();
    assert!(service.snapshot(true).unwrap().restore.tasks.is_empty());
    assert_eq!(fs::read(final_output).unwrap(), b"m1-bytes");
    service.shutdown().unwrap();
    server.finish();
}

#[test]
fn fixture_runtime_controls_and_restart_remain_task_scoped_without_duplicate_workers() {
    let temp = TempRoot::new("controls");
    let first_worker = Arc::new(ControlledWorker::complete_after_retry());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(FixtureSearch::new("http://127.0.0.1:1/fixture")),
        first_worker.clone(),
    );
    let item_id = freeze_and_select(&service, "m1-control-batch");
    create_task(
        &service,
        "m1-control-task",
        "m1-control-batch",
        &item_id,
        temp.root.join("output"),
    );
    wait_until("fixture worker start", || {
        first_worker.starts.load(Ordering::SeqCst) == 1
    });

    update_task(
        &service,
        "m1-control-task",
        TaskStateCommandKind::PauseTask,
        vec![],
    );
    wait_until("worker pause", || {
        first_worker.paused_seen.load(Ordering::SeqCst)
    });
    update_task(
        &service,
        "m1-control-task",
        TaskStateCommandKind::PauseItems,
        vec![item_id.clone()],
    );
    let item_paused = runtime(&service, "m1-control-task");
    assert!(item_paused.task_paused);
    assert_eq!(
        item_paused.paused_item_ids.as_slice(),
        std::slice::from_ref(&item_id)
    );
    update_task(
        &service,
        "m1-control-task",
        TaskStateCommandKind::ResumeTask,
        vec![],
    );
    wait_until("task and item resume", || {
        let resumed = runtime(&service, "m1-control-task");
        !resumed.task_paused
            && resumed.paused_item_ids.is_empty()
            && (resumed.queued_item_ids.contains(&item_id)
                || resumed.active_item_ids.contains(&item_id))
    });
    update_task(
        &service,
        "m1-control-task",
        TaskStateCommandKind::CancelItems,
        vec![item_id.clone()],
    );
    wait_until("cancelled result", || {
        service
            .download_snapshot("m1-control-task")
            .map(|snapshot| {
                snapshot
                    .results
                    .first()
                    .map(|value| value.outcome == DownloadOutcome::Cancelled)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    });
    assert_eq!(first_worker.starts.load(Ordering::SeqCst), 1);

    first_worker.fail_next.store(true, Ordering::SeqCst);
    let retry_item_id =
        "PROVIDER::asf.catalog::COLLECTION::SENTINEL-1::ITEM::S1-M1-RETRY".to_owned();
    create_task_with_items(
        &service,
        "m1-retry-task",
        "m1-control-batch",
        vec![item_id.clone(), retry_item_id.clone()],
        temp.root.join("retry-output"),
    );
    wait_until("failed result", || {
        service
            .download_snapshot("m1-retry-task")
            .map(|snapshot| {
                snapshot
                    .results
                    .first()
                    .map(|value| value.outcome == DownloadOutcome::Failed)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    });
    wait_until("second retry item running", || {
        runtime(&service, "m1-retry-task").status == TaskLifecycleStatus::Running
            && first_worker.starts.load(Ordering::SeqCst) == 3
    });
    update_task(
        &service,
        "m1-retry-task",
        TaskStateCommandKind::RetryItems,
        vec![item_id.clone()],
    );
    first_worker.release.store(true, Ordering::SeqCst);
    wait_until("retried result", || {
        service
            .download_snapshot("m1-retry-task")
            .map(|snapshot| {
                snapshot.results.len() == 2
                    && snapshot
                        .results
                        .iter()
                        .all(|value| value.outcome == DownloadOutcome::Completed)
            })
            .unwrap_or(false)
    });
    assert_eq!(first_worker.starts.load(Ordering::SeqCst), 4);

    first_worker.release.store(false, Ordering::SeqCst);
    first_worker.paused_seen.store(false, Ordering::SeqCst);
    create_task(
        &service,
        "m1-restart-task",
        "m1-control-batch",
        &item_id,
        temp.root.join("restart-output"),
    );
    wait_until("restart fixture worker start", || {
        first_worker.starts.load(Ordering::SeqCst) == 5
    });
    update_task(
        &service,
        "m1-restart-task",
        TaskStateCommandKind::PauseTask,
        vec![],
    );
    wait_until("restart fixture worker pause", || {
        first_worker.paused_seen.load(Ordering::SeqCst)
    });
    service.shutdown().unwrap();
    assert_eq!(
        runtime_after_shutdown(&temp.database, "m1-restart-task").status,
        TaskLifecycleStatus::Paused
    );

    let restarted_worker = Arc::new(ControlledWorker {
        starts: AtomicUsize::new(0),
        paused_seen: AtomicBool::new(false),
        release: AtomicBool::new(true),
        fail_next: AtomicBool::new(false),
    });
    let restarted = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(FixtureSearch::new("http://127.0.0.1:1/fixture")),
        restarted_worker.clone(),
    );
    assert_eq!(
        runtime(&restarted, "m1-restart-task").status,
        TaskLifecycleStatus::Paused
    );
    assert_eq!(restarted_worker.starts.load(Ordering::SeqCst), 0);
    update_task(
        &restarted,
        "m1-restart-task",
        TaskStateCommandKind::ResumeTask,
        vec![],
    );
    wait_until("restarted worker start", || {
        restarted_worker.starts.load(Ordering::SeqCst) == 1
    });
    assert_eq!(first_worker.starts.load(Ordering::SeqCst), 5);
    restarted.shutdown().unwrap();
}

fn runtime_after_shutdown(database: &PathBuf, task_id: &str) -> insar_core::TaskRuntimeSnapshot {
    let service = ApplicationService::new(ApplicationServiceConfig::store(database));
    let value = runtime(&service, task_id);
    service.shutdown().unwrap();
    value
}

#[test]
fn sar_download_policy_defaults_persists_and_rejects_out_of_range_values() {
    let temp = TempRoot::new("sar-download-policy");
    let service = ApplicationService::new(ApplicationServiceConfig::store(&temp.database));
    assert_eq!(
        service.get_sar_download_policy_settings().unwrap(),
        SarDownloadPolicySettings {
            mode: SarDownloadPolicyMode::Manual,
            max_concurrent_items: 2,
            organize_by_product: false,
            default_output_root: String::new(),
        }
    );
    service
        .save_sar_download_policy_settings(&SarDownloadPolicySettings {
            mode: SarDownloadPolicyMode::Manual,
            max_concurrent_items: 4,
            organize_by_product: true,
            default_output_root: temp.root.join("downloads").to_string_lossy().into_owned(),
        })
        .unwrap();
    drop(service);

    let reopened = ApplicationService::new(ApplicationServiceConfig::store(&temp.database));
    let persisted = reopened.get_sar_download_policy_settings().unwrap();
    assert_eq!(persisted.max_concurrent_items, 4);
    assert!(persisted.organize_by_product);
    assert_eq!(
        persisted.default_output_root,
        temp.root.join("downloads").to_string_lossy()
    );
    assert!(reopened
        .save_sar_download_policy_settings(&SarDownloadPolicySettings {
            mode: SarDownloadPolicyMode::Manual,
            max_concurrent_items: 9,
            organize_by_product: false,
            default_output_root: String::new(),
        })
        .is_err());
}

#[test]
fn sar_download_policy_changes_apply_only_to_new_tasks() {
    let temp = TempRoot::new("sar-download-policy-freeze");
    let worker = Arc::new(ControlledWorker::complete_after_retry());
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(FixtureSearch::new("http://127.0.0.1:1/fixture")),
        worker,
    );
    let item_id = freeze_and_select(&service, "sar-policy-batch");
    let initial_policy = service.get_sar_download_policy_settings().unwrap();
    create_task_with_download_policy(
        &service,
        "sar-policy-task-a",
        "sar-policy-batch",
        &item_id,
        temp.root.join("task-a"),
        &initial_policy,
    );

    service
        .save_sar_download_policy_settings(&SarDownloadPolicySettings {
            mode: SarDownloadPolicyMode::Manual,
            max_concurrent_items: 4,
            organize_by_product: true,
            default_output_root: temp.root.join("downloads").to_string_lossy().into_owned(),
        })
        .unwrap();
    let updated_policy = service.get_sar_download_policy_settings().unwrap();
    create_task_with_download_policy(
        &service,
        "sar-policy-task-b",
        "sar-policy-batch",
        &item_id,
        temp.root.join("task-b"),
        &updated_policy,
    );

    let snapshot = service.snapshot(true).unwrap();
    let task_a = snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "sar-policy-task-a")
        .unwrap();
    let task_b = snapshot
        .restore
        .tasks
        .iter()
        .find(|task| task.task_id == "sar-policy-task-b")
        .unwrap();
    assert_eq!(runtime(&service, "sar-policy-task-a").concurrency, 2);
    assert_eq!(task_a.layout, DownloadLayout::Direct);
    assert_eq!(runtime(&service, "sar-policy-task-b").concurrency, 4);
    assert_eq!(task_b.layout, DownloadLayout::SarDatasetProcessing);
    service.shutdown().unwrap();
}

#[test]
fn sidecar_restore_matches_the_store_projection_used_by_tauri_after_owner_shutdown() {
    let temp = TempRoot::new("parity");
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        Arc::new(FixtureSearch::new("http://127.0.0.1:1/fixture")),
        Arc::new(ControlledWorker::complete_after_retry()),
    );
    let item_id = freeze_and_select(&service, "m1-parity-batch");
    create_task(
        &service,
        "m1-parity-task",
        "m1-parity-batch",
        &item_id,
        temp.root.join("output"),
    );
    update_task(
        &service,
        "m1-parity-task",
        TaskStateCommandKind::PauseTask,
        vec![],
    );
    let tauri_restore =
        Value::Object(canonical_restore_value(&service.snapshot(true).unwrap().restore).unwrap());
    service.shutdown().unwrap();

    let dispatcher = Dispatcher::with_asf_search_and_store_path(
        Arc::new(FixtureSearch::new("http://127.0.0.1:1/fixture")),
        temp.database.clone(),
    );
    let response = dispatcher.dispatch(SidecarRequest {
        protocol_major: 1,
        protocol_minor: 16,
        request_id: "m1-parity-restore".to_owned(),
        operation: SidecarOperation::StoreRestore,
        payload: json!({ "include_terminal": true })
            .as_object()
            .unwrap()
            .clone(),
    });
    assert!(response.ok);
    assert_eq!(
        tauri_restore,
        Value::Object(response.result),
        "Tauri's service snapshot and the Sidecar store restore have the same durable meaning"
    );
}

#[test]
fn sar_search_whose_records_could_not_be_mapped_is_searched_again() {
    // Issue #14: an empty batch frozen from records the provider could not map
    // was reused from the 24 h search cache instead of being searched again.
    let temp = TempRoot::new("mapping-failure");
    let search = Arc::new(MappingFailureThenSuccessSearch {
        fixture: FixtureSearch::new("http://127.0.0.1:9/unused"),
        calls: AtomicUsize::new(0),
    });
    let service = ApplicationService::with_fixture_business_runtime(
        ApplicationServiceConfig::business_store(&temp.database),
        search.clone(),
        Arc::new(LoopbackHttpWorker::new()),
    );
    bind_aoi(&service, "m1-mapping-failure");
    let first = service
        .sar_search_and_freeze_legacy(&search_request("m1-mapping-failure-1"))
        .unwrap();
    assert!(first.restore.workbench_batch.unwrap().candidates.is_empty());

    let second = service
        .sar_search_and_freeze_legacy(&search_request("m1-mapping-failure-2"))
        .unwrap();
    assert_eq!(
        search.calls.load(Ordering::SeqCst),
        2,
        "the unmapped empty batch must not be served from the search cache"
    );
    assert_eq!(second.restore.workbench_batch.unwrap().candidates.len(), 2);
}
