use std::ffi::OsString;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use insar_core::{
    DownloadLayout, DownloadOutcome, FrozenSearchBatch, FrozenTaskSnapshot, SearchWorkbenchState,
    SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskStateCommandKind,
};
use insar_download::{
    finalize_part, part_path, AuthRefreshPolicyId, DestinationLeaseController, DownloadScheduler,
    DownloadWork, DownloadWorker, ProductionDownloadWorker, ResolvedAsset, ResolvedTransferResult,
    ResolvedTransferSession, ResolverExecutionRuntime, TransferAuth, TransferBackend,
    TransferBackendKind, TransferOptions, TransferRequest, TransferResult, TransferUpdate,
    WorkerControl,
};
use insar_store::TaskStore;
use serde_json::{json, Map};
use sha2::{Digest, Sha256};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("insar-destination-ownership-cutover-{label}-{id}"));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("insar-destination-ownership-{id}.sqlite3")))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

fn scheduler_item() -> SnapshotItem {
    serde_json::from_value(json!({
        "item_id": "shared-item",
        "provider_id": "asf.sentinel1",
        "display_name": "shared.zip",
        "metadata": {},
        "payload": {
            "download_url": "https://datapool.asf.alaska.edu/SLC/frozen/shared.zip",
            "file_name": "shared.zip",
            "file_size_remote": 12
        }
    }))
    .unwrap()
}

fn seed_scheduler_tasks(path: &Path, output_root: &Path) {
    let item = scheduler_item();
    let mut store = TaskStore::open(path).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "destination-ownership-batch".to_string(),
                provider_id: "asf.sentinel1".to_string(),
                created_at: "2026-07-24T00:00:00Z".to_string(),
                aoi_name: "AOI".to_string(),
                aoi_geojson: Some(
                    json!({"type": "Polygon", "coordinates": []})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates: vec![item.clone()],
            },
            workbench: SearchWorkbenchState {
                batch_id: "destination-ownership-batch".to_string(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    for task_id in ["lease-task-one", "lease-task-two"] {
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: task_id.to_string(),
                batch_id: "destination-ownership-batch".to_string(),
                selected_item_ids: vec![item.item_id.clone()],
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: output_root.to_path_buf(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-07-24T00:00:01Z".to_string(),
            })
            .unwrap();
    }
}

fn update_scheduler_task(path: &Path, task_id: &str, kind: TaskStateCommandKind, event_id: &str) {
    let mut store = TaskStore::open(path).unwrap();
    let task = store.load_task(task_id).unwrap().unwrap();
    let item_ids = if kind == TaskStateCommandKind::CancelItems {
        vec!["shared-item".to_string()]
    } else {
        vec![]
    };
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_string(),
            expected_revision: task.runtime.revision,
            command: StoreTaskCommand { kind, item_ids },
            event_id: event_id.to_string(),
            occurred_at: "2026-07-24T00:00:02Z".to_string(),
        })
        .unwrap();
}

fn backup_slot(destination: &Path, slot: usize) -> PathBuf {
    let file_name = destination.file_name().unwrap();
    let digest = Sha256::digest(file_name.as_encoded_bytes());
    let suffix = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let slot_suffix = if slot == 0 {
        String::new()
    } else {
        format!("-{slot}")
    };
    destination
        .parent()
        .unwrap()
        .join(format!(".insar-promotion-{suffix}{slot_suffix}.backup"))
}

static TOKEN_ENV_LOCK: Mutex<()> = Mutex::new(());

struct TestEarthdataToken {
    previous: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl TestEarthdataToken {
    fn install() -> Self {
        let lock = TOKEN_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("EARTHDATA_TOKEN");
        std::env::set_var("EARTHDATA_TOKEN", "FIXTURE_TOKEN_D4A2B");
        Self {
            previous,
            _lock: lock,
        }
    }
}

impl Drop for TestEarthdataToken {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("EARTHDATA_TOKEN", value),
            None => std::env::remove_var("EARTHDATA_TOKEN"),
        }
    }
}

struct ActiveBackendCall<'a>(&'a AtomicUsize);

impl Drop for ActiveBackendCall<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct BlockingLegacyBackend {
    calls: AtomicUsize,
    current: AtomicUsize,
    maximum: AtomicUsize,
    entered: Mutex<usize>,
    entered_changed: Condvar,
    released: Mutex<bool>,
    release_changed: Condvar,
    panic_once: AtomicBool,
    reinspection_destination: Option<PathBuf>,
    reinspection_observed: AtomicBool,
}

impl BlockingLegacyBackend {
    fn new(panic_once: bool) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            current: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
            entered: Mutex::new(0),
            entered_changed: Condvar::new(),
            released: Mutex::new(false),
            release_changed: Condvar::new(),
            panic_once: AtomicBool::new(panic_once),
            reinspection_destination: None,
            reinspection_observed: AtomicBool::new(false),
        }
    }

    fn with_reinspection(destination: PathBuf) -> Self {
        let mut backend = Self::new(false);
        backend.reinspection_destination = Some(destination);
        backend
    }

    fn wait_for_entered(&self, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut entered = self.entered.lock().unwrap();
        while *entered < expected {
            let timeout = deadline.saturating_duration_since(Instant::now());
            let (next, _) = self.entered_changed.wait_timeout(entered, timeout).unwrap();
            entered = next;
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {expected} backend entries"
            );
        }
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.release_changed.notify_all();
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn maximum(&self) -> usize {
        self.maximum.load(Ordering::SeqCst)
    }

    fn reinspection_observed(&self) -> bool {
        self.reinspection_observed.load(Ordering::SeqCst)
    }
}

impl TransferBackend for BlockingLegacyBackend {
    fn kind(&self) -> TransferBackendKind {
        TransferBackendKind::NativeSingleStream
    }

    fn is_available(&self) -> bool {
        true
    }

    fn download_legacy(
        &self,
        _request: &TransferRequest,
        _auth: &TransferAuth,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(destination) = &self.reinspection_destination {
            if call == 1 {
                fs::write(destination, b"first-output").unwrap();
            } else {
                self.reinspection_observed
                    .store(fs::metadata(destination).is_ok(), Ordering::SeqCst);
            }
        }
        let active = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        let _active = ActiveBackendCall(&self.current);
        {
            let mut entered = self.entered.lock().unwrap();
            *entered += 1;
            self.entered_changed.notify_all();
        }
        if self.panic_once.swap(false, Ordering::SeqCst) {
            panic!("intentional destination-ownership backend unwind");
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut released = self.released.lock().unwrap();
        while !*released {
            let timeout = deadline.saturating_duration_since(Instant::now());
            let (next, _) = self
                .release_changed
                .wait_timeout(released, timeout)
                .unwrap();
            released = next;
            assert!(
                Instant::now() < deadline,
                "timed out waiting for backend release"
            );
        }
        TransferResult {
            outcome: DownloadOutcome::Completed,
            bytes_written: 12,
            attempt_count: 1,
            failure: None,
        }
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).unwrap()
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        _session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        panic!("legacy destination-ownership fixture must not run a resolver route");
    }
}

fn legacy_work(task_id: &str, item_id: &str, output_root: &Path) -> DownloadWork {
    let item: SnapshotItem = serde_json::from_value(json!({
        "item_id": item_id,
        "provider_id": "asf.sentinel1",
        "display_name": "shared.zip",
        "metadata": {},
        "payload": {
            "download_url": "https://datapool.asf.alaska.edu/SLC/frozen/shared.zip",
            "file_name": "shared.zip",
            "file_size_remote": 12
        }
    }))
    .unwrap();
    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": task_id,
        "batch_id": format!("batch-{task_id}"),
        "provider_id": "asf.sentinel1",
        "created_at": "2026-07-24T00:00:00Z",
        "aoi_name": "AOI",
        "aoi_geojson": {"type": "Polygon", "coordinates": []},
        "query": {"dataset": "SENTINEL-1"},
        "candidates": [item.clone()],
        "initial_selected_item_ids": [item.item_id.clone()],
        "output_root": output_root,
        "layout": "direct"
    }))
    .unwrap();
    DownloadWork { task, item }
}

fn legacy_worker(
    backend: Arc<dyn TransferBackend>,
    controller: Arc<DestinationLeaseController>,
) -> ProductionDownloadWorker {
    ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(
            ResolverExecutionRuntime::production().unwrap(),
            backend,
        )
        .with_destination_lease_controller_for_test(controller)
}

fn wait_for_snapshot(
    controller: &DestinationLeaseController,
    predicate: impl Fn() -> bool,
    message: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        let snapshot = controller.snapshot_for_qualification_test();
        assert!(
            Instant::now() < deadline,
            "{message}; snapshot={snapshot:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
    let snapshot = controller.snapshot_for_qualification_test();
    assert!(
        !snapshot.failed_closed(),
        "lease controller must remain available"
    );
}

fn assert_idle(controller: &DestinationLeaseController) {
    let snapshot = controller.snapshot_for_qualification_test();
    assert_eq!(snapshot.live_destinations(), 0);
    assert_eq!(snapshot.queued_waiters(), 0);
    assert_eq!(snapshot.active_leases(), 0);
    assert!(!snapshot.failed_closed());
}

#[test]
fn recovery_backup_blocks_live_finalization_without_destroying_survivors() {
    let root = TempRoot::new("recovery-residue");
    let destination = root.path().join("scene.zip");
    let part = part_path(&destination);
    let backup = backup_slot(&destination, 0);
    fs::write(&part, b"verified-new").unwrap();
    fs::write(&backup, b"previous-valid").unwrap();

    let error = finalize_part(&destination, Some(12))
        .expect_err("an unresolved prior destination backup must block live finalization");

    assert_eq!(error.code(), "DL005");
    assert!(error.reason_zh().contains("恢复"));
    assert!(!destination.exists());
    assert_eq!(fs::read(&part).unwrap(), b"verified-new");
    assert_eq!(fs::read(&backup).unwrap(), b"previous-valid");
}

#[test]
fn exhausted_backup_slots_fail_without_removing_the_previous_destination_or_part() {
    let root = TempRoot::new("slot-exhaustion");
    let destination = root.path().join("scene.zip");
    let part = part_path(&destination);
    fs::write(&destination, b"previous-valid").unwrap();
    fs::write(&part, b"verified-new").unwrap();
    for slot in 0..32 {
        fs::write(backup_slot(&destination, slot), b"occupied-backup").unwrap();
    }

    let error = finalize_part(&destination, Some(12))
        .expect_err("live finalization must not bypass occupied safe-promotion backup slots");

    assert_eq!(error.code(), "DL005");
    assert_eq!(fs::read(&destination).unwrap(), b"previous-valid");
    assert_eq!(fs::read(&part).unwrap(), b"verified-new");
}

#[test]
fn legacy_same_destination_backend_calls_do_not_overlap() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("legacy-same");
    let controller = DestinationLeaseController::new_for_qualification_test();
    let backend = Arc::new(BlockingLegacyBackend::new(false));
    let worker = Arc::new(legacy_worker(backend.clone(), Arc::clone(&controller)));

    let first_work = legacy_work("legacy-task-one", "legacy-item-one", root.path());
    let second_work = legacy_work("legacy-task-two", "legacy-item-two", root.path());
    let first_worker = Arc::clone(&worker);
    let first = thread::spawn(move || {
        first_worker.execute(first_work, WorkerControl::default(), &mut |_| {})
    });
    backend.wait_for_entered(1);

    let second_worker = Arc::clone(&worker);
    let second = thread::spawn(move || {
        second_worker.execute(second_work, WorkerControl::default(), &mut |_| {})
    });
    wait_for_snapshot(
        &controller,
        || {
            controller
                .snapshot_for_qualification_test()
                .queued_waiters()
                == 1
        },
        "same planned destination must queue before its backend call",
    );
    assert_eq!(backend.calls(), 1);
    assert_eq!(backend.maximum(), 1);

    backend.release();
    assert_eq!(first.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_eq!(second.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_eq!(backend.calls(), 2);
    assert_eq!(backend.maximum(), 1);
    assert_idle(&controller);
}

#[test]
fn legacy_different_destinations_can_enter_backends_concurrently() {
    let _token = TestEarthdataToken::install();
    let first_root = TempRoot::new("legacy-first");
    let second_root = TempRoot::new("legacy-second");
    let controller = DestinationLeaseController::new_for_qualification_test();
    let backend = Arc::new(BlockingLegacyBackend::new(false));
    let worker = Arc::new(legacy_worker(backend.clone(), Arc::clone(&controller)));

    let first_worker = Arc::clone(&worker);
    let first = thread::spawn(move || {
        first_worker.execute(
            legacy_work("legacy-task-first", "legacy-item-first", first_root.path()),
            WorkerControl::default(),
            &mut |_| {},
        )
    });
    let second_worker = Arc::clone(&worker);
    let second = thread::spawn(move || {
        second_worker.execute(
            legacy_work(
                "legacy-task-second",
                "legacy-item-second",
                second_root.path(),
            ),
            WorkerControl::default(),
            &mut |_| {},
        )
    });

    backend.wait_for_entered(2);
    assert_eq!(backend.maximum(), 2);
    backend.release();
    assert_eq!(first.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_eq!(second.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_idle(&controller);
}

#[test]
fn successor_rechecks_destination_after_waiting_for_owner() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("legacy-reinspect");
    let destination = root.path().join("shared.zip");
    let controller = DestinationLeaseController::new_for_qualification_test();
    let backend = Arc::new(BlockingLegacyBackend::with_reinspection(
        destination.clone(),
    ));
    let worker = Arc::new(legacy_worker(backend.clone(), Arc::clone(&controller)));

    let first_worker = Arc::clone(&worker);
    let first_root = root.path().to_path_buf();
    let first = thread::spawn(move || {
        first_worker.execute(
            legacy_work("legacy-reinspect-first", "legacy-item-first", &first_root),
            WorkerControl::default(),
            &mut |_| {},
        )
    });
    backend.wait_for_entered(1);

    let second_worker = Arc::clone(&worker);
    let second_root = root.path().to_path_buf();
    let second = thread::spawn(move || {
        second_worker.execute(
            legacy_work(
                "legacy-reinspect-second",
                "legacy-item-second",
                &second_root,
            ),
            WorkerControl::default(),
            &mut |_| {},
        )
    });
    wait_for_snapshot(
        &controller,
        || {
            controller
                .snapshot_for_qualification_test()
                .queued_waiters()
                == 1
        },
        "successor did not wait for the first destination owner",
    );
    assert_eq!(backend.calls(), 1);

    backend.release();
    assert_eq!(first.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_eq!(second.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_eq!(backend.calls(), 2);
    // Downloader D3 (local-first dedup): the successor re-inspects the
    // destination only after waiting for its owner, and now decides what it
    // finds before transferring. The first owner's output is not a readable
    // ZIP, so it is kept under a conflict name and the item downloads again;
    // the second transfer therefore no longer sees a file at the destination.
    assert!(!backend.reinspection_observed());
    let conflicts = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("shared.zip.eostage-conflict-")
        })
        .collect::<Vec<_>>();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(fs::read(&conflicts[0]).unwrap(), b"first-output");
    assert_idle(&controller);
}

#[test]
fn queued_scheduler_cancellation_never_enters_the_legacy_backend() {
    let _token = TestEarthdataToken::install();
    let output_root = TempRoot::new("scheduler-cancel-output");
    let database = TempDatabase::new();
    seed_scheduler_tasks(database.path(), output_root.path());
    let controller = DestinationLeaseController::new_for_qualification_test();
    let backend = Arc::new(BlockingLegacyBackend::new(false));
    let worker = Arc::new(legacy_worker(backend.clone(), Arc::clone(&controller)));
    let scheduler =
        DownloadScheduler::start(database.path(), worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();

    backend.wait_for_entered(1);
    wait_for_snapshot(
        &controller,
        || {
            controller
                .snapshot_for_qualification_test()
                .queued_waiters()
                == 1
        },
        "second scheduler item did not queue for the shared destination",
    );
    update_scheduler_task(
        database.path(),
        "lease-task-two",
        TaskStateCommandKind::CancelItems,
        "cancel-queued-shared-destination",
    );
    scheduler.wake();
    wait_for_snapshot(
        &controller,
        || {
            let snapshot = controller.snapshot_for_qualification_test();
            snapshot.active_leases() == 1 && snapshot.queued_waiters() == 0
        },
        "cancelled destination waiter was not removed",
    );
    assert_eq!(backend.calls(), 1);

    backend.release();
    wait_for_snapshot(
        &controller,
        || controller.snapshot_for_qualification_test().active_leases() == 0,
        "first destination owner did not release after completion",
    );
    scheduler.shutdown();
    assert_eq!(backend.calls(), 1);
    assert_idle(&controller);
}

#[test]
fn queued_pause_waits_and_paused_owner_keeps_the_destination_lease() {
    let _token = TestEarthdataToken::install();
    let output_root = TempRoot::new("scheduler-pause-output");
    let database = TempDatabase::new();
    seed_scheduler_tasks(database.path(), output_root.path());
    let controller = DestinationLeaseController::new_for_qualification_test();
    let backend = Arc::new(BlockingLegacyBackend::new(false));
    let worker = Arc::new(legacy_worker(backend.clone(), Arc::clone(&controller)));
    let scheduler =
        DownloadScheduler::start(database.path(), worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();

    backend.wait_for_entered(1);
    wait_for_snapshot(
        &controller,
        || {
            let snapshot = controller.snapshot_for_qualification_test();
            snapshot.active_leases() == 1 && snapshot.queued_waiters() == 1
        },
        "shared-destination scheduler setup did not establish owner and waiter",
    );
    update_scheduler_task(
        database.path(),
        "lease-task-one",
        TaskStateCommandKind::PauseTask,
        "pause-owner-shared-destination",
    );
    update_scheduler_task(
        database.path(),
        "lease-task-two",
        TaskStateCommandKind::PauseTask,
        "pause-waiter-shared-destination",
    );
    scheduler.wake();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(backend.calls(), 1);
    assert_eq!(
        controller.snapshot_for_qualification_test().active_leases(),
        1
    );

    backend.release();
    wait_for_snapshot(
        &controller,
        || {
            let snapshot = controller.snapshot_for_qualification_test();
            snapshot.active_leases() == 0 && snapshot.queued_waiters() == 1
        },
        "paused queued item acquired the destination before resume",
    );
    assert_eq!(backend.calls(), 1);

    update_scheduler_task(
        database.path(),
        "lease-task-two",
        TaskStateCommandKind::ResumeTask,
        "resume-waiter-shared-destination",
    );
    scheduler.wake();
    backend.wait_for_entered(2);
    scheduler.shutdown();
    assert_eq!(backend.calls(), 2);
    assert_idle(&controller);
}

#[test]
fn backend_unwind_releases_the_worker_destination_guard() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("legacy-unwind");
    let controller = DestinationLeaseController::new_for_qualification_test();
    let backend = Arc::new(BlockingLegacyBackend::new(true));
    let worker = legacy_worker(backend.clone(), Arc::clone(&controller));

    let unwind = panic::catch_unwind(AssertUnwindSafe(|| {
        worker.execute(
            legacy_work("legacy-task-panic", "legacy-item-panic", root.path()),
            WorkerControl::default(),
            &mut |_| {},
        )
    }));
    assert!(unwind.is_err());
    assert_idle(&controller);

    backend.release();
    let result = worker.execute(
        legacy_work(
            "legacy-task-after-panic",
            "legacy-item-after-panic",
            root.path(),
        ),
        WorkerControl::default(),
        &mut |_| {},
    );
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_idle(&controller);
}

#[test]
fn cutover_requires_live_worker_ownership_and_no_finalization_predelete() {
    let finalization = include_str!("../src/file_core.rs");
    let worker = include_str!("../src/production_worker.rs");

    assert!(
        !finalization.contains("fs::remove_file(destination)"),
        "safe finalization must not pre-delete the canonical destination"
    );
    assert!(
        worker.contains("DestinationLeaseController"),
        "the shared ProductionDownloadWorker must own the live destination controller"
    );
}
