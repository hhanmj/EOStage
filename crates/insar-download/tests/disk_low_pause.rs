//! Downloads pause when the storage drive runs low (PM decision 2026-10-26,
//! A1 §3.1), through the user's pause transition with a typed reason.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout, DownloadOutcome,
    FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskPauseReason, TaskPauseReasonKind, TaskStateCommandKind,
};
use insar_download::disk_guard::{
    drive_label, volume_query_path, DiskProbeFailureLog, FreeSpaceProbe, VolumeSpace,
};
use insar_download::{DownloadScheduler, DownloadWork, DownloadWorker, WorkerControl};
use insar_store::TaskStore;
use serde_json::Map;

const GIB: u64 = 1 << 30;
const TOTAL: u64 = 20 * GIB;
const TASK: &str = "disk-task";

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-disk-low-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A drive whose free bytes the test sets; total is fixed at 20 GiB, so the
/// absolute floor (2 GiB) is larger than 5 % (1 GiB).
struct FakeVolume {
    free: AtomicU64,
    probes: AtomicUsize,
}

impl FakeVolume {
    fn new(free: u64) -> Arc<Self> {
        Arc::new(Self {
            free: AtomicU64::new(free),
            probes: AtomicUsize::new(0),
        })
    }
}

impl FreeSpaceProbe for FakeVolume {
    fn volume_space(&self, _path: &Path) -> Option<VolumeSpace> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        Some(VolumeSpace {
            free_bytes: self.free.load(Ordering::SeqCst),
            total_bytes: TOTAL,
        })
    }
}

/// Writes `file_bytes` in `chunk` steps (counted, not stored), reporting
/// progress after each one and stopping as soon as it is paused.
struct ChunkWorker {
    file_bytes: u64,
    chunk: u64,
    tick: Duration,
    starts: AtomicUsize,
    written: AtomicU64,
}

impl ChunkWorker {
    fn new(file_bytes: u64, chunk: u64) -> Arc<Self> {
        Arc::new(Self {
            file_bytes,
            chunk,
            tick: Duration::from_millis(20),
            starts: AtomicUsize::new(0),
            written: AtomicU64::new(0),
        })
    }
}

impl DownloadWorker for ChunkWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let mut done = 0_u64;
        let outcome = loop {
            if control.is_cancelled() {
                break DownloadOutcome::Cancelled;
            }
            if control.is_paused() {
                break DownloadOutcome::Interrupted;
            }
            if done >= self.file_bytes {
                break DownloadOutcome::Completed;
            }
            let step = self.chunk.min(self.file_bytes - done);
            done += step;
            self.written.fetch_add(step, Ordering::SeqCst);
            progress(DownloadItemProgress {
                task_id: work.task.task_id.clone(),
                item_id: work.item.item_id.clone(),
                status: DownloadItemStatus::Active,
                bytes_completed: done,
                total_bytes: Some(self.file_bytes),
                bytes_per_second: 1.0,
                attempt: 1,
                updated_at: "2026-10-26T00:00:01Z".to_owned(),
            });
            thread::sleep(self.tick);
        };
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome,
            destination: work.task.output_root.join(&work.item.item_id),
            bytes_written: done,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-10-26T00:00:02Z".to_owned(),
        }
    }
}

fn seed(store: &mut TaskStore, output_root: &Path) {
    let item_id = "item-0".to_owned();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "disk-batch".to_owned(),
                provider_id: "fixture.provider".to_owned(),
                created_at: "2026-10-26T00:00:00Z".to_owned(),
                aoi_name: "fixture AOI".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                search_stats: None,
                candidates: vec![SnapshotItem {
                    item_id: item_id.clone(),
                    provider_id: "fixture.provider".to_owned(),
                    display_name: item_id.clone(),
                    acquisition_datetime: None,
                    footprint_geojson: None,
                    metadata: Map::new(),
                    payload: Map::new(),
                    asset_access: None,
                    execution_policy: None,
                }],
            },
            workbench: SearchWorkbenchState {
                batch_id: "disk-batch".to_owned(),
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
            task_id: TASK.to_owned(),
            batch_id: "disk-batch".to_owned(),
            selected_item_ids: vec![item_id.clone()],
            checked_item_ids: vec![item_id],
            page_index: 0,
            page_size: 25,
            output_root: output_root.to_path_buf(),
            layout: DownloadLayout::Direct,
            concurrency: 1,
            created_at: "2026-10-26T00:00:01Z".to_owned(),
        })
        .unwrap();
}

// Field order is drop order: the Store connection closes before the
// directory holding its file is removed.
struct Harness {
    scheduler: Option<DownloadScheduler>,
    store: Arc<Mutex<TaskStore>>,
    dir: TempDir,
}

impl Harness {
    fn start(label: &str, volume: Arc<FakeVolume>, worker: Arc<ChunkWorker>) -> Self {
        let dir = TempDir::new(label);
        let mut store = TaskStore::open(dir.0.join("store.sqlite3")).unwrap();
        seed(&mut store, &dir.0.join("out"));
        let store = Arc::new(Mutex::new(store));
        let scheduler = DownloadScheduler::start_with_store_and_free_space_probe(
            Arc::clone(&store),
            worker,
            Duration::from_millis(10),
            volume,
        );
        scheduler.wake();
        Self {
            scheduler: Some(scheduler),
            store,
            dir,
        }
    }

    fn task(&self) -> insar_store::StoredTask {
        self.store.lock().unwrap().load_task(TASK).unwrap().unwrap()
    }

    fn reason(&self) -> Option<TaskPauseReason> {
        self.store
            .lock()
            .unwrap()
            .load_task_pause_reasons(&[TASK.to_owned()])
            .unwrap()
            .remove(TASK)
    }

    fn wait_until(&self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn user_command(&self, kind: TaskStateCommandKind, event_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let mut store = self.store.lock().unwrap();
            let revision = store.load_task(TASK).unwrap().unwrap().runtime.revision;
            let result = store.update_task(&StoreUpdateTaskRequest {
                task_id: TASK.to_owned(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind: kind.clone(),
                    item_ids: vec![],
                },
                event_id: event_id.to_owned(),
                occurred_at: "2026-10-26T00:03:00Z".to_owned(),
            });
            drop(store);
            if result.is_ok() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{kind:?} lost every revision race"
            );
            thread::sleep(Duration::from_millis(2));
        }
        self.scheduler.as_ref().unwrap().wake();
    }

    fn expected_drive(&self) -> Option<String> {
        drive_label(&self.dir.0.join("out"))
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(scheduler) = self.scheduler.take() {
            scheduler.shutdown();
        }
    }
}

fn wait_stable(counter: &AtomicU64) -> u64 {
    let mut last = counter.load(Ordering::SeqCst);
    let mut since = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        thread::sleep(Duration::from_millis(20));
        let now = counter.load(Ordering::SeqCst);
        if now != last {
            last = now;
            since = Instant::now();
        } else if since.elapsed() >= Duration::from_millis(200) {
            return last;
        }
        assert!(Instant::now() < deadline, "the worker kept writing");
    }
}

#[test]
fn below_the_absolute_floor_no_item_is_opened_and_the_task_pauses_with_its_reason() {
    let volume = FakeVolume::new(GIB);
    let worker = ChunkWorker::new(4 * GIB, GIB);
    let harness = Harness::start("absolute-claim", Arc::clone(&volume), Arc::clone(&worker));
    harness.wait_until("the disk-low pause", |h| h.task().runtime.task_paused);

    let task = harness.task();
    assert_eq!(task.runtime.status, TaskLifecycleStatus::Paused);
    assert_eq!(
        harness.reason(),
        Some(TaskPauseReason {
            kind: TaskPauseReasonKind::DiskLow,
            free_bytes: GIB,
            threshold_bytes: 2 * GIB,
            drive: harness.expected_drive(),
        })
    );
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        worker.starts.load(Ordering::SeqCst),
        0,
        "no item was opened"
    );
    assert_eq!(
        worker.written.load(Ordering::SeqCst),
        0,
        "no byte was written"
    );
    let events = harness
        .store
        .lock()
        .unwrap()
        .list_task_events(TASK)
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "pause_task")
            .count(),
        1,
        "one pause event, not one per scheduling tick"
    );
}

#[test]
fn dropping_below_the_absolute_floor_while_writing_pauses_and_writing_stops() {
    let volume = FakeVolume::new(10 * GIB);
    // 1 MiB chunks of a 1 GiB file: the file rule (remaining + 1 GiB) stays
    // below 10 GiB, so only the absolute floor can pause it.
    let worker = ChunkWorker::new(GIB, 1 << 20);
    let harness = Harness::start("absolute-write", Arc::clone(&volume), Arc::clone(&worker));
    harness.wait_until("writing to begin", |_| {
        worker.written.load(Ordering::SeqCst) >= 4 << 20
    });
    volume.free.store(GIB, Ordering::SeqCst);
    harness.wait_until("the disk-low pause", |h| h.task().runtime.task_paused);
    let stopped_at = wait_stable(&worker.written);
    assert!(stopped_at < GIB, "the file did not finish");
    harness.wait_until("the item to be durably paused", |h| {
        h.task().runtime.paused_item_ids == vec!["item-0".to_owned()]
    });
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        worker.written.load(Ordering::SeqCst),
        stopped_at,
        "no further bytes"
    );
    assert_eq!(worker.starts.load(Ordering::SeqCst), 1);
    let reason = harness.reason().expect("typed reason");
    assert_eq!(reason.free_bytes, GIB);
    assert_eq!(reason.threshold_bytes, 2 * GIB);
}

#[test]
fn free_space_below_the_remaining_file_plus_headroom_pauses() {
    // 10 GiB free is above the 2 GiB floor, so the item is opened; the 16 GiB
    // file needs remaining + 1 GiB, which 10 GiB cannot hold.
    let volume = FakeVolume::new(10 * GIB);
    let worker = ChunkWorker::new(16 * GIB, 1 << 20);
    let harness = Harness::start("file-rule", Arc::clone(&volume), Arc::clone(&worker));
    harness.wait_until("the disk-low pause", |h| h.task().runtime.task_paused);
    assert_eq!(
        worker.starts.load(Ordering::SeqCst),
        1,
        "opened above the floor"
    );
    let stopped_at = wait_stable(&worker.written);
    assert!(stopped_at < 16 * GIB);
    let reason = harness.reason().expect("typed reason");
    assert_eq!(reason.kind, TaskPauseReasonKind::DiskLow);
    assert_eq!(reason.free_bytes, 10 * GIB);
    // The first forwarded progress reported one chunk done.
    assert_eq!(reason.threshold_bytes, 16 * GIB - (1 << 20) + GIB);
}

#[test]
fn enough_free_space_lets_the_task_complete_without_a_reason() {
    let volume = FakeVolume::new(10 * GIB);
    let worker = ChunkWorker::new(64 << 20, 16 << 20);
    let harness = Harness::start("enough", Arc::clone(&volume), Arc::clone(&worker));
    harness.wait_until("completion", |h| {
        h.task().runtime.status == TaskLifecycleStatus::Completed
    });
    assert_eq!(worker.written.load(Ordering::SeqCst), 64 << 20);
    assert_eq!(harness.reason(), None);
    assert!(
        volume.probes.load(Ordering::SeqCst) > 0,
        "the guard measured the drive"
    );
}

#[test]
fn a_resume_that_bypasses_the_service_while_low_is_paused_again_and_runs_once_space_frees() {
    let volume = FakeVolume::new(GIB);
    let worker = ChunkWorker::new(64 << 20, 16 << 20);
    let harness = Harness::start("resume", Arc::clone(&volume), Arc::clone(&worker));
    harness.wait_until("the disk-low pause", |h| h.task().runtime.task_paused);

    // A raw Store resume (no service pre-check) on a still-low drive: the
    // engine re-checks before opening the item and pauses again.
    harness.user_command(TaskStateCommandKind::ResumeTask, "raw-resume-low");
    harness.wait_until("the second disk-low pause", |h| {
        h.task().runtime.task_paused && h.reason().is_some()
    });
    assert_eq!(worker.starts.load(Ordering::SeqCst), 0);

    volume.free.store(10 * GIB, Ordering::SeqCst);
    harness.user_command(TaskStateCommandKind::ResumeTask, "resume-after-free");
    harness.wait_until("completion", |h| {
        h.task().runtime.status == TaskLifecycleStatus::Completed
    });
    assert_eq!(harness.reason(), None);
    assert_eq!(worker.written.load(Ordering::SeqCst), 64 << 20);
}

/// The guard is an engine invariant, not an Agent decision: the engine crate
/// cannot name an Agent permission mode (no dependency on the Agent crate),
/// and neither the guard, the scheduler nor the shared policy mentions one.
#[test]
fn the_engine_guard_never_sees_an_agent_permission_mode() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        !manifest.contains("insar-agent"),
        "insar-download must not depend on the Agent"
    );
    for (name, source) in [
        ("disk_guard.rs", include_str!("../src/disk_guard.rs")),
        ("scheduler.rs", include_str!("../src/scheduler.rs")),
        (
            "storage_guard.rs",
            include_str!("../../insar-core/src/storage_guard.rs"),
        ),
    ] {
        for term in [
            "permission_mode",
            "PermissionMode",
            "confirm_changes",
            "auto_approve",
            "full_access",
            "auto_by_mode",
        ] {
            assert!(!source.contains(term), "{name} mentions {term}");
        }
    }
}

/// The one test that measures the real volume: the operating system's probe
/// reports sane numbers for the temp directory and for a not-yet-created
/// output root below it.
#[test]
fn the_system_probe_reports_sane_numbers_for_the_temp_directory() {
    use insar_download::disk_guard::SystemFreeSpaceProbe;
    let temp = std::env::temp_dir();
    let space = SystemFreeSpaceProbe
        .volume_space(&temp)
        .expect("the temp volume is measurable");
    assert!(space.total_bytes >= GIB, "{space:?}");
    assert!(space.free_bytes <= space.total_bytes, "{space:?}");
    let missing = temp.join("insar-disk-low-missing").join("out");
    assert!(!missing.exists());
    let ancestor = SystemFreeSpaceProbe
        .volume_space(&missing)
        .expect("a missing output root is measured at its nearest existing ancestor");
    assert_eq!(ancestor.total_bytes, space.total_bytes);
    #[cfg(windows)]
    assert!(
        drive_label(&temp).is_some_and(|drive| drive.ends_with(':')),
        "{:?}",
        drive_label(&temp)
    );
}

/// Test builds of this crate (dev-dependency feature test-plenty-disk-space)
/// default to a volume with plenty of space, so no existing scheduler test
/// measures or pauses on the development drive.
#[test]
fn test_builds_default_to_a_probe_with_plenty_of_space() {
    let space = insar_download::disk_guard::default_free_space_probe()
        .volume_space(Path::new("C:\\"))
        .expect("the test probe always answers");
    assert!(space.free_bytes >= 1 << 50, "{space:?}");
}

/// Answers after 300 ms with plenty of space, like a slow network volume.
struct SlowVolume {
    probes: AtomicUsize,
}

impl FreeSpaceProbe for SlowVolume {
    fn volume_space(&self, _path: &Path) -> Option<VolumeSpace> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(300));
        Some(VolumeSpace {
            free_bytes: 10 * GIB,
            total_bytes: TOTAL,
        })
    }
}

/// Both measuring points (before opening an item and while writing) run
/// outside the shared Store lock: with a probe that takes 300 ms, an
/// unrelated Store read never waits anywhere near that long.
#[test]
fn a_slow_probe_never_blocks_store_readers() {
    let dir = TempDir::new("slow-probe");
    let mut store = TaskStore::open(dir.0.join("store.sqlite3")).unwrap();
    seed(&mut store, &dir.0.join("out"));
    let store = Arc::new(Mutex::new(store));
    let volume = Arc::new(SlowVolume {
        probes: AtomicUsize::new(0),
    });
    let worker = ChunkWorker::new(GIB, 1 << 20);
    let scheduler = DownloadScheduler::start_with_store_and_free_space_probe(
        Arc::clone(&store),
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
        Duration::from_millis(10),
        Arc::clone(&volume) as Arc<dyn FreeSpaceProbe>,
    );
    scheduler.wake();
    let mut slowest = Duration::ZERO;
    let deadline = Instant::now() + Duration::from_millis(3500);
    while Instant::now() < deadline {
        let started = Instant::now();
        {
            let store = store.lock().unwrap();
            let _ = store.load_task(TASK).unwrap();
        }
        slowest = slowest.max(started.elapsed());
        thread::sleep(Duration::from_millis(5));
    }
    scheduler.shutdown();
    let probes = volume.probes.load(Ordering::SeqCst);
    assert!(probes >= 3, "both measuring points ran ({probes} probes)");
    assert!(worker.starts.load(Ordering::SeqCst) == 1, "the item opened");
    assert!(
        slowest < Duration::from_millis(150),
        "a Store read waited {slowest:?} behind a 300 ms probe"
    );
}

/// What `GetDiskFreeSpaceExW` (Windows) or `statvfs` (Unix) reporting nothing
/// amounts to: the volume cannot be measured.
struct UnknownVolume;

impl FreeSpaceProbe for UnknownVolume {
    fn volume_space(&self, _path: &Path) -> Option<VolumeSpace> {
        None
    }
}

/// An unmeasurable output root: unknown means allow, and the coordinator logs
/// one line for the task (no path) however often it looks. `\\server\share` is
/// a Windows UNC root; on Unix a UNC path is an ordinary (relative) file name,
/// so the platform's equivalent unmeasurable root is an absolute path whose
/// volume query fails and whose label is the filesystem root.
#[test]
fn an_unmeasurable_unc_root_is_allowed_and_logged_once_without_its_path() {
    let dir = TempDir::new("unc");
    #[cfg(windows)]
    let unmeasurable_root = PathBuf::from(r"\\server\share");
    #[cfg(not(windows))]
    let unmeasurable_root = PathBuf::from("/insar-unmeasurable-share");
    let mut store = TaskStore::open(dir.0.join("store.sqlite3")).unwrap();
    seed(&mut store, &unmeasurable_root);
    let store = Arc::new(Mutex::new(store));
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink_lines = Arc::clone(&lines);
    let worker = ChunkWorker::new(64 << 20, 16 << 20);
    let scheduler = DownloadScheduler::start_with_store_free_space_probe_and_log(
        Arc::clone(&store),
        Arc::clone(&worker) as Arc<dyn DownloadWorker>,
        Duration::from_millis(10),
        Arc::new(UnknownVolume),
        Arc::new(move |line: &str| sink_lines.lock().unwrap().push(line.to_owned())),
    );
    scheduler.wake();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let status = store
            .lock()
            .unwrap()
            .load_task(TASK)
            .unwrap()
            .unwrap()
            .runtime
            .status;
        if status == TaskLifecycleStatus::Completed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the unmeasurable task did not complete"
        );
        thread::sleep(Duration::from_millis(10));
    }
    scheduler.shutdown();
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("task_id=disk-task"), "{}", lines[0]);
    #[cfg(windows)]
    assert!(
        !lines[0].contains("server") && !lines[0].contains('\\'),
        "{}",
        lines[0]
    );
    #[cfg(not(windows))]
    assert!(
        !lines[0].contains("insar-unmeasurable") && !lines[0].contains('/'),
        "{}",
        lines[0]
    );
    assert_eq!(worker.written.load(Ordering::SeqCst), 64 << 20);
    #[cfg(windows)]
    assert_eq!(
        drive_label(&unmeasurable_root).as_deref(),
        Some(r"\\server\share")
    );
    #[cfg(not(windows))]
    assert_eq!(drive_label(&unmeasurable_root).as_deref(), Some("/"));
}

#[test]
fn the_failure_log_emits_one_line_per_task() {
    let mut log = DiskProbeFailureLog::default();
    assert!(log
        .observe("a")
        .is_some_and(|line| line.ends_with("task_id=a")));
    assert_eq!(log.observe("a"), None);
    assert!(log.observe("b").is_some());
}

/// The volume query always ends with the platform's separator:
/// `GetDiskFreeSpaceExW` accepts a UNC root (`\\server\share`) only with a
/// trailing `\`, and `statvfs` receives an absolute path with a trailing `/`.
#[test]
fn volume_queries_always_end_with_a_separator() {
    #[cfg(windows)]
    {
        assert_eq!(
            volume_query_path(Path::new(r"\\server\share")),
            std::ffi::OsString::from(r"\\server\share\")
        );
        assert_eq!(
            volume_query_path(Path::new(r"C:\data")),
            std::ffi::OsString::from(r"C:\data\")
        );
        assert_eq!(
            volume_query_path(Path::new(r"C:\")),
            std::ffi::OsString::from(r"C:\")
        );
    }
    #[cfg(not(windows))]
    {
        assert_eq!(
            volume_query_path(Path::new("/mnt/data")),
            std::ffi::OsString::from("/mnt/data/")
        );
        assert_eq!(
            volume_query_path(Path::new("/mnt/data/")),
            std::ffi::OsString::from("/mnt/data/")
        );
        assert_eq!(
            volume_query_path(Path::new("/")),
            std::ffi::OsString::from("/")
        );
    }
}
