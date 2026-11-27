//! Downloader D3 (local-first dedup), worker gate: an existing planned file
//! is decided inside the destination lease and before credentials, Provider
//! resolution and network admission. A reusable file finishes the item as
//! skipped without any transfer; a conflicting file is renamed aside, never
//! deleted or overwritten, and the planned file is downloaded.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use insar_core::{
    CreateTaskFromHistoricalSourceBatchRequest, DownloadLayout, DownloadOutcome, FrozenSearchBatch,
    FrozenTaskSnapshot, SearchWorkbenchState, SnapshotItem, StoreArchiveTaskRequest,
    StoreCreateTaskRequest, StoreSearchBatchRequest, TaskLifecycleStatus,
};
use insar_download::{
    AuthRefreshPolicyId, DownloadScheduler, DownloadWork, DownloadWorker, ProductionDownloadWorker,
    ResolvedAsset, ResolvedTransferResult, ResolvedTransferSession, ResolverExecutionRuntime,
    TransferAuth, TransferBackend, TransferBackendKind, TransferOptions, TransferRequest,
    TransferResult, TransferUpdate, WorkerControl,
};
use insar_store::{StoreLocalReuseDisposition, TaskStore};
use serde_json::{json, Map};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "insar-local-reuse-gate-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
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

static TOKEN_ENV_LOCK: Mutex<()> = Mutex::new(());

/// An Earthdata token in the environment, so a transfer that does run never
/// reads the user's credential store.
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
        std::env::set_var("EARTHDATA_TOKEN", "FIXTURE_TOKEN_D3_LOCAL_REUSE");
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

/// A Legacy backend that counts calls and installs fixed bytes at the
/// planned destination, as a verified landing would.
struct LandingBackend {
    calls: AtomicUsize,
    landed: Vec<u8>,
}

impl LandingBackend {
    fn new(landed: &[u8]) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            landed: landed.to_vec(),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl TransferBackend for LandingBackend {
    fn kind(&self) -> TransferBackendKind {
        TransferBackendKind::NativeSingleStream
    }

    fn is_available(&self) -> bool {
        true
    }

    fn download_legacy(
        &self,
        request: &TransferRequest,
        _auth: &TransferAuth,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            !request.destination.exists(),
            "a transfer never starts over an existing planned file"
        );
        fs::write(&request.destination, &self.landed).unwrap();
        TransferResult {
            outcome: DownloadOutcome::Completed,
            bytes_written: self.landed.len() as u64,
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
        panic!("the Legacy local reuse fixture never runs a resolver route");
    }
}

/// A stored ZIP of one entry: a file the format check accepts.
fn zip_bytes(name: &str, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
    out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(data);
    let directory_offset = out.len() as u32;
    let mut central = Vec::new();
    central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
    central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    central.extend_from_slice(&0_u32.to_le_bytes());
    central.extend_from_slice(&(data.len() as u32).to_le_bytes());
    central.extend_from_slice(&(data.len() as u32).to_le_bytes());
    central.extend_from_slice(&(name.len() as u16).to_le_bytes());
    central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    central.extend_from_slice(&0_u32.to_le_bytes());
    central.extend_from_slice(name.as_bytes());
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&directory_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out
}

fn legacy_item(size: Option<u64>) -> SnapshotItem {
    let mut payload = json!({
        "download_url": "https://datapool.asf.alaska.edu/SLC/frozen/shared.zip",
        "file_name": "shared.zip"
    });
    if let Some(size) = size {
        payload["file_size_remote"] = json!(size);
    }
    serde_json::from_value(json!({
        "item_id": "shared-item",
        "provider_id": "asf.sentinel1",
        "display_name": "shared.zip",
        "metadata": {},
        "payload": payload
    }))
    .unwrap()
}

fn legacy_work(task_id: &str, output_root: &Path, size: Option<u64>) -> DownloadWork {
    let item = legacy_item(size);
    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": task_id,
        "batch_id": format!("batch-{task_id}"),
        "provider_id": "asf.sentinel1",
        "created_at": "2026-10-20T00:00:00Z",
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

fn legacy_worker(backend: Arc<dyn TransferBackend>) -> ProductionDownloadWorker {
    ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(
            ResolverExecutionRuntime::production().unwrap(),
            backend,
        )
}

fn conflict_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("shared.zip.eostage-conflict-")
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn a_reusable_existing_file_is_skipped_before_credentials_and_network() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("reuse");
    let existing = zip_bytes("payload.bin", b"previously downloaded");
    let destination = root.path().join("shared.zip");
    fs::write(&destination, &existing).unwrap();
    let backend = Arc::new(LandingBackend::new(b"never"));
    let worker = legacy_worker(backend.clone());

    let result = worker.execute(
        legacy_work("reuse-task", root.path(), Some(existing.len() as u64)),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(
        result.outcome,
        DownloadOutcome::Skipped,
        "{:?}",
        result.failure
    );
    assert_eq!(result.bytes_written, existing.len() as u64);
    assert_eq!(result.attempt_count, 0, "no transfer attempt");
    assert_eq!(backend.calls(), 0, "no transfer started");
    assert_eq!(
        fs::read(&destination).unwrap(),
        existing,
        "the file is untouched"
    );
    assert!(conflict_files(root.path()).is_empty());
}

#[test]
fn a_conflicting_existing_file_is_renamed_aside_then_downloaded() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("conflict");
    let fresh = zip_bytes("payload.bin", b"freshly downloaded payload");
    let destination = root.path().join("shared.zip");
    fs::write(&destination, b"an older, shorter file").unwrap();
    let backend = Arc::new(LandingBackend::new(&fresh));
    let worker = legacy_worker(backend.clone());

    let result = worker.execute(
        legacy_work("conflict-task", root.path(), Some(fresh.len() as u64)),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(
        result.outcome,
        DownloadOutcome::Completed,
        "{:?}",
        result.failure
    );
    assert_eq!(backend.calls(), 1);
    assert_eq!(fs::read(&destination).unwrap(), fresh);
    let conflicts = conflict_files(root.path());
    assert_eq!(
        conflicts.len(),
        1,
        "the older file is kept under its conflict name"
    );
    assert_eq!(fs::read(&conflicts[0]).unwrap(), b"an older, shorter file");
}

#[test]
fn an_existing_file_of_unknown_size_is_never_reused() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("unknown-size");
    let existing = zip_bytes("payload.bin", b"readable, but nothing proves it complete");
    let fresh = zip_bytes("payload.bin", b"downloaded");
    let destination = root.path().join("shared.zip");
    fs::write(&destination, &existing).unwrap();
    let backend = Arc::new(LandingBackend::new(&fresh));
    let worker = legacy_worker(backend.clone());

    let result = worker.execute(
        legacy_work("unknown-size-task", root.path(), None),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(
        result.outcome,
        DownloadOutcome::Completed,
        "{:?}",
        result.failure
    );
    assert_eq!(backend.calls(), 1);
    assert_eq!(fs::read(&destination).unwrap(), fresh);
    let conflicts = conflict_files(root.path());
    assert_eq!(conflicts.len(), 1);
    assert_eq!(fs::read(&conflicts[0]).unwrap(), existing);
}

fn seed_task(store: &mut TaskStore, task_id: &str, output_root: &Path, size: u64) {
    let item = legacy_item(Some(size));
    let batch_id = format!("batch-{task_id}");
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.clone(),
                provider_id: "asf.sentinel1".to_string(),
                created_at: "2026-10-20T00:00:00Z".to_string(),
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
                batch_id: batch_id.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_string(),
            batch_id,
            selected_item_ids: vec![item.item_id.clone()],
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-20T00:00:01Z".to_string(),
        })
        .unwrap();
}

fn wait_for_status(database: &Path, task_id: &str, status: TaskLifecycleStatus) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let store = TaskStore::open(database).unwrap();
        let task = store.load_task(task_id).unwrap().unwrap();
        if task.runtime.status == status {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{task_id} did not reach {status:?}: {:?}",
            task.runtime.status
        );
        drop(store);
        thread::sleep(Duration::from_millis(20));
    }
}

/// The landing of the first task is recorded in the Store ledger; a later
/// task finds the same file unchanged and reuses it without reading it (its
/// content is not even a ZIP, so no format check could have accepted it).
#[test]
fn the_landing_ledger_reuses_an_unchanged_file_without_reading_it() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("ledger");
    let database_root = TempRoot::new("ledger-database");
    let database = database_root.path().join("store.sqlite3");
    let landed = b"verified at landing, not a zip".to_vec();
    {
        let mut store = TaskStore::open(&database).unwrap();
        seed_task(&mut store, "ledger-first", root.path(), landed.len() as u64);
    }
    let backend = Arc::new(LandingBackend::new(&landed));
    let worker = Arc::new(legacy_worker(backend.clone()));
    let scheduler = DownloadScheduler::start(&database, worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();
    wait_for_status(&database, "ledger-first", TaskLifecycleStatus::Completed);
    assert_eq!(backend.calls(), 1);
    let destination = root.path().join("shared.zip");
    {
        let mut store = TaskStore::open(&database).unwrap();
        let ledger = store
            .current_landed_artifact(&destination.to_string_lossy())
            .unwrap()
            .expect("the landing is recorded");
        assert_eq!(ledger.size_bytes, landed.len() as u64);
        assert_eq!(ledger.provider_id, "asf.sentinel1");
        assert_eq!(ledger.product_id, "shared-item");
        assert_eq!(ledger.source_task_id.as_deref(), Some("ledger-first"));
        seed_task(
            &mut store,
            "ledger-second",
            root.path(),
            landed.len() as u64,
        );
    }
    scheduler.wake();
    wait_for_status(&database, "ledger-second", TaskLifecycleStatus::Completed);
    scheduler.shutdown();
    assert_eq!(backend.calls(), 1, "the second task did not transfer");
    let store = TaskStore::open(&database).unwrap();
    let decision = store
        .task_item_local_reuse("ledger-second", "shared-item")
        .unwrap()
        .expect("the reuse is recorded");
    assert_eq!(
        decision.disposition,
        StoreLocalReuseDisposition::Reused {
            grade: "ledger_identity".to_owned()
        }
    );
    assert_eq!(decision.size_bytes, Some(landed.len() as u64));
    assert_eq!(fs::read(&destination).unwrap(), landed);
}

/// Downloader D3 (Owner decision 2026-10-20): 重新下载 of a reused item in a
/// terminal task creates a successor in the same folder; its worker renames
/// the existing file aside and keeps it, then downloads the item again.
#[test]
fn reacquiring_a_reused_item_keeps_the_original_and_downloads_again() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("reacquire");
    let database_root = TempRoot::new("reacquire-database");
    let database = database_root.path().join("store.sqlite3");
    let existing = zip_bytes("payload.bin", b"reused without content evidence");
    let fresh = zip_bytes("payload.bin", b"downloaded again, same size!!!!");
    assert_eq!(existing.len(), fresh.len());
    let destination = root.path().join("shared.zip");
    fs::write(&destination, &existing).unwrap();
    {
        let mut store = TaskStore::open(&database).unwrap();
        seed_task(
            &mut store,
            "reacquire-source",
            root.path(),
            existing.len() as u64,
        );
    }
    let backend = Arc::new(LandingBackend::new(&fresh));
    let worker = Arc::new(legacy_worker(backend.clone()));
    let scheduler = DownloadScheduler::start(&database, worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();
    wait_for_status(
        &database,
        "reacquire-source",
        TaskLifecycleStatus::Completed,
    );
    assert_eq!(backend.calls(), 0, "the first task reused the file");
    {
        let mut store = TaskStore::open(&database).unwrap();
        let reused = store
            .task_item_local_reuse("reacquire-source", "shared-item")
            .unwrap()
            .unwrap();
        assert_eq!(
            reused.disposition,
            StoreLocalReuseDisposition::Reused {
                grade: "size_and_format".to_owned()
            }
        );
        let revision = store
            .load_task("reacquire-source")
            .unwrap()
            .unwrap()
            .runtime
            .revision;
        store
            .archive_task(&StoreArchiveTaskRequest {
                task_id: "reacquire-source".to_owned(),
                expected_revision: revision,
                archived_at: "2026-10-20T00:10:00Z".to_owned(),
            })
            .unwrap();
        store
            .create_task_from_historical_source_batch(&CreateTaskFromHistoricalSourceBatchRequest {
                source_task_id: "reacquire-source".to_owned(),
                expected_source_task_revision: revision,
                new_task_id: "reacquire-successor".to_owned(),
                item_ids: vec!["shared-item".to_owned()],
                output_root: root.path().to_path_buf(),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-10-20T00:11:00Z".to_owned(),
                reacquire_item_ids: vec!["shared-item".to_owned()],
            })
            .unwrap();
    }
    scheduler.wake();
    wait_for_status(
        &database,
        "reacquire-successor",
        TaskLifecycleStatus::Completed,
    );
    scheduler.shutdown();
    assert_eq!(
        backend.calls(),
        1,
        "the successor downloaded the item again"
    );
    assert_eq!(fs::read(&destination).unwrap(), fresh);
    let conflicts = conflict_files(root.path());
    assert_eq!(
        conflicts.len(),
        1,
        "the original is kept under its conflict name"
    );
    assert_eq!(fs::read(&conflicts[0]).unwrap(), existing);
    let store = TaskStore::open(&database).unwrap();
    let decision = store
        .task_item_local_reuse("reacquire-successor", "shared-item")
        .unwrap()
        .unwrap();
    match decision.disposition {
        StoreLocalReuseDisposition::RenamedConflict {
            reason,
            conflict_file_name,
        } => {
            assert_eq!(reason, "reacquire");
            assert_eq!(
                conflict_file_name,
                conflicts[0].file_name().unwrap().to_string_lossy()
            );
        }
        other => panic!("{other:?}"),
    }
}

/// Downloader D3-2 (pre-scan): the scan decides a queued item locally and
/// never renames, transfers or records anything itself.
#[test]
fn the_pre_scan_reports_a_reusable_legacy_file_without_touching_it() {
    use insar_download::{LocalScanPhase, LocalScanVerdict};
    let root = TempRoot::new("scan-reuse");
    let existing = zip_bytes("payload.bin", b"previously downloaded");
    let destination = root.path().join("shared.zip");
    fs::write(&destination, &existing).unwrap();
    let backend = Arc::new(LandingBackend::new(b"never"));
    let worker = legacy_worker(backend.clone());
    let work = legacy_work("scan-task", root.path(), Some(existing.len() as u64));
    let verdict = worker.scan_local(
        &work,
        LocalScanPhase::Cheap,
        &WorkerControl::default(),
        None,
    );
    let LocalScanVerdict::Reuse(item) = verdict else {
        panic!("expected a reuse, got {verdict:?}");
    };
    assert_eq!(item.item_id, "shared-item");
    assert_eq!(item.destination, destination);
    assert_eq!(item.size_bytes, existing.len() as u64);
    assert_eq!(item.grade, "size_and_format");
    assert!(item.landed.is_none());
    assert_eq!(backend.calls(), 0);
    assert_eq!(fs::read(&destination).unwrap(), existing);
}

#[test]
fn the_pre_scan_releases_conflicts_unknown_sizes_and_busy_destinations_untouched() {
    use insar_download::{DestinationLeaseController, LocalScanPhase, LocalScanVerdict};
    let root = TempRoot::new("scan-release");
    let destination = root.path().join("shared.zip");
    fs::write(&destination, b"an older, shorter file").unwrap();
    let controller = DestinationLeaseController::new_for_qualification_test();
    let backend = Arc::new(LandingBackend::new(b"never"));
    let worker = legacy_worker(backend.clone())
        .with_destination_lease_controller_for_test(Arc::clone(&controller));
    let control = WorkerControl::default();
    // A size conflict is the worker's to rename: the scan only releases it.
    let conflict = legacy_work("scan-conflict", root.path(), Some(1024));
    assert_eq!(
        worker.scan_local(&conflict, LocalScanPhase::Cheap, &control, None),
        LocalScanVerdict::Release
    );
    // An unknown size never reuses.
    let unknown = legacy_work("scan-unknown", root.path(), None);
    assert_eq!(
        worker.scan_local(&unknown, LocalScanPhase::Cheap, &control, None),
        LocalScanVerdict::Release
    );
    // A destination another owner holds is left to its worker.
    let reusable = zip_bytes("payload.bin", b"reusable");
    fs::write(&destination, &reusable).unwrap();
    let owner = controller
        .acquire_for_worker(&destination, &control)
        .unwrap();
    let busy = legacy_work("scan-busy", root.path(), Some(reusable.len() as u64));
    assert_eq!(
        worker.scan_local(&busy, LocalScanPhase::Cheap, &control, None),
        LocalScanVerdict::Release
    );
    drop(owner);
    assert_eq!(fs::read(&destination).unwrap(), reusable);
    assert!(
        conflict_files(root.path()).is_empty(),
        "the scan never renames"
    );
    assert_eq!(backend.calls(), 0);
}

fn scan_items(sizes: &[Option<u64>]) -> Vec<SnapshotItem> {
    sizes
        .iter()
        .enumerate()
        .map(|(index, size)| {
            let mut payload = json!({
                "download_url": format!("https://datapool.asf.alaska.edu/SLC/frozen/scene-{index}.zip"),
                "file_name": format!("scene-{index}.zip")
            });
            if let Some(size) = size {
                payload["file_size_remote"] = json!(size);
            }
            serde_json::from_value(json!({
                "item_id": format!("scene-{index}"),
                "provider_id": "asf.sentinel1",
                "display_name": format!("scene-{index}.zip"),
                "metadata": {},
                "payload": payload
            }))
            .unwrap()
        })
        .collect()
}

fn seed_scan_task(
    store: &mut TaskStore,
    task_id: &str,
    output_root: &Path,
    items: &[SnapshotItem],
) {
    let batch_id = format!("batch-{task_id}");
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: batch_id.clone(),
                provider_id: "asf.sentinel1".to_string(),
                created_at: "2026-10-20T00:00:00Z".to_string(),
                aoi_name: "AOI".to_string(),
                aoi_geojson: Some(
                    json!({"type": "Polygon", "coordinates": []})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates: items.to_vec(),
            },
            workbench: SearchWorkbenchState {
                batch_id: batch_id.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_string(),
            batch_id,
            selected_item_ids: items.iter().map(|item| item.item_id.clone()).collect(),
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-10-20T00:00:01Z".to_string(),
        })
        .unwrap();
}

fn activated_items(store: &TaskStore, task_id: &str) -> Vec<String> {
    store
        .list_task_events(task_id)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "activate_items")
        .flat_map(|event| {
            event.payload["command"]["item_ids"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect()
}

/// Downloader D3-2 (batch pre-scan): with every planned file already on disk,
/// the coordinator's scan commits them as skipped in batches before any item
/// is claimed: no worker runs, no transfer starts.
#[test]
fn the_pre_scan_reuses_existing_files_before_any_item_is_claimed() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("prescan-all");
    let database_root = TempRoot::new("prescan-all-database");
    let database = database_root.path().join("store.sqlite3");
    let contents = (0..6)
        .map(|index| {
            zip_bytes(
                &format!("p{index}.bin"),
                format!("existing {index}").as_bytes(),
            )
        })
        .collect::<Vec<_>>();
    for (index, content) in contents.iter().enumerate() {
        fs::write(root.path().join(format!("scene-{index}.zip")), content).unwrap();
    }
    let items = scan_items(
        &contents
            .iter()
            .map(|content| Some(content.len() as u64))
            .collect::<Vec<_>>(),
    );
    {
        let mut store = TaskStore::open(&database).unwrap();
        seed_scan_task(&mut store, "prescan-all", root.path(), &items);
    }
    let backend = Arc::new(LandingBackend::new(b"never"));
    let worker = Arc::new(legacy_worker(backend.clone()));
    let scheduler = DownloadScheduler::start(&database, worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();
    wait_for_status(&database, "prescan-all", TaskLifecycleStatus::Completed);
    scheduler.shutdown();
    assert_eq!(backend.calls(), 0);
    let store = TaskStore::open(&database).unwrap();
    assert!(
        activated_items(&store, "prescan-all").is_empty(),
        "no item was claimed"
    );
    assert!(store
        .list_task_events("prescan-all")
        .unwrap()
        .iter()
        .any(|event| event.event_type == "local_reuse_applied"));
    let scan = store
        .task_local_scan("prescan-all")
        .unwrap()
        .expect("scan progress");
    assert_eq!(scan.state, insar_store::StoreLocalScanState::Done);
    assert_eq!((scan.scanned_items, scan.total_items), (6, 6));
    for index in 0..6 {
        let decision = store
            .task_item_local_reuse("prescan-all", &format!("scene-{index}"))
            .unwrap()
            .unwrap();
        assert_eq!(
            decision.disposition,
            StoreLocalReuseDisposition::Reused {
                grade: "size_and_format".to_owned()
            }
        );
    }
}

/// Downloader D3-2: a missing file is released to its worker while the scan
/// reuses the others; only the missing item is claimed and transferred.
#[test]
fn the_pre_scan_releases_missing_items_and_reuses_the_rest() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("prescan-mixed");
    let database_root = TempRoot::new("prescan-mixed-database");
    let database = database_root.path().join("store.sqlite3");
    let landed = zip_bytes("p.bin", b"downloaded now");
    let mut sizes = Vec::new();
    for index in 0..4 {
        let content = zip_bytes(
            &format!("p{index}.bin"),
            format!("existing {index}").as_bytes(),
        );
        fs::write(root.path().join(format!("scene-{index}.zip")), &content).unwrap();
        sizes.push(Some(content.len() as u64));
    }
    sizes.push(Some(landed.len() as u64));
    let items = scan_items(&sizes);
    {
        let mut store = TaskStore::open(&database).unwrap();
        seed_scan_task(&mut store, "prescan-mixed", root.path(), &items);
    }
    let backend = Arc::new(LandingBackend::new(&landed));
    let worker = Arc::new(legacy_worker(backend.clone()));
    let scheduler = DownloadScheduler::start(&database, worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();
    wait_for_status(&database, "prescan-mixed", TaskLifecycleStatus::Completed);
    scheduler.shutdown();
    assert_eq!(backend.calls(), 1, "only the missing item transferred");
    let store = TaskStore::open(&database).unwrap();
    assert_eq!(
        activated_items(&store, "prescan-mixed"),
        vec!["scene-4".to_owned()]
    );
    assert_eq!(fs::read(root.path().join("scene-4.zip")).unwrap(), landed);
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A ledger row for a file that was copied or restored: same size and
/// SHA-256, another identity, so only a re-hash proves it.
fn moved_file_ledger_row(
    destination: &Path,
    item_id: &str,
    content: &[u8],
) -> insar_store::StoreLandedArtifact {
    insar_store::StoreLandedArtifact {
        path_key: destination.to_string_lossy().into_owned(),
        provider_id: "asf.sentinel1".to_owned(),
        product_id: item_id.to_owned(),
        size_bytes: content.len() as u64,
        sha256: Some(sha256_hex(content)),
        service_checksum: None,
        identity: insar_store::StoreFileIdentity {
            platform: if cfg!(windows) { "windows" } else { "unix" }.to_owned(),
            volume_id: 1,
            file_id: 1,
            modified: Some(1),
            changed: Some(1),
            stable: true,
        },
        evidence: insar_store::StoreLandingEvidence::Landing,
        source_task_id: Some("earlier-task".to_owned()),
        source_item_id: Some(item_id.to_owned()),
        verified_at: "2026-10-01T00:00:00Z".to_owned(),
    }
}

/// Downloader D3-2 (second phase): a file the ledger knows by SHA-256 but
/// under another identity needs a digest. The coordinator's digest job
/// hashes it before any claim: a match is reused in the batch with no
/// worker, transfer or activate_items event.
#[test]
fn the_pre_scan_digest_phase_reuses_a_rehashed_file_without_a_worker() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("prescan-digest");
    let database_root = TempRoot::new("prescan-digest-database");
    let database = database_root.path().join("store.sqlite3");
    let content = zip_bytes("p0.bin", b"restored from a backup");
    let destination = root.path().join("scene-0.zip");
    fs::write(&destination, &content).unwrap();
    let items = scan_items(&[Some(content.len() as u64)]);
    {
        let mut store = TaskStore::open(&database).unwrap();
        seed_scan_task(&mut store, "prescan-digest", root.path(), &items);
        store
            .record_landed_artifact(&moved_file_ledger_row(&destination, "scene-0", &content))
            .unwrap();
    }
    let backend = Arc::new(LandingBackend::new(b"never"));
    let worker = Arc::new(legacy_worker(backend.clone()));
    let scheduler = DownloadScheduler::start(&database, worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();
    wait_for_status(&database, "prescan-digest", TaskLifecycleStatus::Completed);
    scheduler.shutdown();
    assert_eq!(backend.calls(), 0);
    let store = TaskStore::open(&database).unwrap();
    assert!(
        activated_items(&store, "prescan-digest").is_empty(),
        "hashed before any claim"
    );
    assert_eq!(
        store
            .task_item_local_reuse("prescan-digest", "scene-0")
            .unwrap()
            .unwrap()
            .disposition,
        StoreLocalReuseDisposition::Reused {
            grade: "ledger_rehash".to_owned()
        }
    );
    let scan = store.task_local_scan("prescan-digest").unwrap().unwrap();
    assert_eq!(scan.state, insar_store::StoreLocalScanState::Done);
}

/// Downloader D3-2 (second phase): a re-hash that does not match releases
/// the item; its worker's gate renames the file aside and downloads.
#[test]
fn the_pre_scan_digest_phase_releases_a_mismatch_to_its_worker() {
    let _token = TestEarthdataToken::install();
    let root = TempRoot::new("prescan-digest-mismatch");
    let database_root = TempRoot::new("prescan-digest-mismatch-database");
    let database = database_root.path().join("store.sqlite3");
    let recorded = zip_bytes("p0.bin", b"what the ledger saw");
    let changed = zip_bytes("p0.bin", b"what is there nowxx");
    assert_eq!(recorded.len(), changed.len());
    let destination = root.path().join("scene-0.zip");
    fs::write(&destination, &changed).unwrap();
    let items = scan_items(&[Some(recorded.len() as u64)]);
    {
        let mut store = TaskStore::open(&database).unwrap();
        seed_scan_task(&mut store, "prescan-digest-mismatch", root.path(), &items);
        store
            .record_landed_artifact(&moved_file_ledger_row(&destination, "scene-0", &recorded))
            .unwrap();
    }
    let backend = Arc::new(LandingBackend::new(&recorded));
    let worker = Arc::new(legacy_worker(backend.clone()));
    let scheduler = DownloadScheduler::start(&database, worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();
    wait_for_status(
        &database,
        "prescan-digest-mismatch",
        TaskLifecycleStatus::Completed,
    );
    scheduler.shutdown();
    assert_eq!(backend.calls(), 1);
    let store = TaskStore::open(&database).unwrap();
    assert_eq!(
        activated_items(&store, "prescan-digest-mismatch"),
        vec!["scene-0".to_owned()]
    );
    let kept = fs::read_dir(root.path())
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("scene-0.zip.eostage-conflict-")
        })
        .count();
    assert_eq!(kept, 1, "the changed file is kept aside");
}

fn reusable_scan_items(root: &Path, count: usize) -> Vec<SnapshotItem> {
    let sizes = (0..count)
        .map(|index| {
            let content = zip_bytes(
                &format!("p{index}.bin"),
                format!("existing {index}").as_bytes(),
            );
            fs::write(root.join(format!("scene-{index}.zip")), &content).unwrap();
            Some(content.len() as u64)
        })
        .collect::<Vec<_>>();
    scan_items(&sizes)
}

/// One scheduler over `count` reusable items: the time from the first wake
/// to the completed task (the first phase decides every item, the batches
/// commit them).
fn timed_prescan(label: &str, count: usize) -> Duration {
    let root = TempRoot::new(label);
    let database_root = TempRoot::new(&format!("{label}-database"));
    let database = database_root.path().join("store.sqlite3");
    let items = reusable_scan_items(root.path(), count);
    {
        let mut store = TaskStore::open(&database).unwrap();
        seed_scan_task(&mut store, label, root.path(), &items);
    }
    let backend = Arc::new(LandingBackend::new(b"never"));
    let worker = Arc::new(legacy_worker(backend.clone()));
    let scheduler = DownloadScheduler::start(&database, worker, Duration::from_millis(5)).unwrap();
    let started = Instant::now();
    scheduler.wake();
    wait_for_status(&database, label, TaskLifecycleStatus::Completed);
    let elapsed = started.elapsed();
    scheduler.shutdown();
    assert_eq!(backend.calls(), 0);
    let store = TaskStore::open(&database).unwrap();
    assert!(activated_items(&store, label).is_empty());
    elapsed
}

/// Downloader D3-2 measurement (run explicitly with --ignored --nocapture):
/// the first-phase time of 250 reusable items, seven runs.
#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn measure_prescan_phase_one_for_250_items() {
    let mut samples = (0..7)
        .map(|run| timed_prescan(&format!("prescan-measure-{run}"), 250).as_secs_f64() * 1000.0)
        .collect::<Vec<_>>();
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let p95_index = ((samples.len() as f64) * 0.95).ceil() as usize - 1;
    println!(
        "PRESCAN_250_MS samples={samples:?} median={median:.1} p95={:.1} max={:.1}",
        samples[p95_index.min(samples.len() - 1)],
        samples[samples.len() - 1]
    );
}

/// Downloader D3-2 acceptance: 250 reusable items are decided by the first
/// phase and committed within 3 s, with no worker and no transfer.
/// Measured 2026-10-20 on the development host (14 runs, two processes,
/// verification logs 054-055): median 670 ms, P95 816 ms, max 816 ms
/// (about 2.7 ms per item, including the test's 20 ms status polling). The
/// budget is about 3.7 times the observed maximum, to absorb a loaded
/// machine without hiding a per-item regression such as a claim, a digest
/// or a network round trip per item.
#[test]
fn the_pre_scan_decides_250_reusable_items_within_its_budget() {
    let elapsed = timed_prescan("prescan-budget", 250);
    assert!(
        elapsed <= Duration::from_secs(3),
        "250 reusable items took {elapsed:?}"
    );
}
