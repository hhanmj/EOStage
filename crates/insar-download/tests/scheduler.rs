use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout,
    DownloadOutcome, FrozenSearchBatch, NisarSubsetDependencyEvidenceAuthorityV2,
    NisarSubsetDependencyRelationV2, NisarSubsetFrequency, NisarSubsetFrequencyCapabilityV2,
    NisarSubsetPlanMetadataEvidenceV2, NisarSubsetPlanV2, NisarSubsetPreparedIntentBuildInputV2,
    NisarSubsetPreparedIntentV2, NisarSubsetQualityStatusV2, NisarSubsetSourceAssetIdentityV2,
    NisarSubsetSourceCapabilityV2, NisarSubsetSourceDependencyEdgeV2,
    NisarSubsetSourceDependencyGraphV2, NisarSubsetSourceDependencyNodeKindV2,
    NisarSubsetSourceDependencyNodeV2, NisarSubsetSourceProductIdentityV2, SearchWorkbenchState,
    SnapshotItem, StoreCreateTaskRequest, StoreSearchBatchRequest, StoreTaskCommand,
    StoreUpdateTaskRequest, TaskLifecycleStatus, TaskStateCommandKind,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT,
    NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
};
use insar_download::{
    clear_active_transfer_engine_prefs_override, set_active_transfer_engine_prefs,
    DownloadScheduler, DownloadWork, DownloadWorker, NisarExecutionPlanFreezeAuthority,
    NisarExecutionPlanFreezeError, NisarExecutionPlanReadyV2, TaskTransferSample,
    TransferEnginePrefs, WorkerControl,
};

static SCHEDULER_PREFS_TEST_LOCK: Mutex<()> = Mutex::new(());

struct ActiveSchedulerPrefsGuard;

impl Drop for ActiveSchedulerPrefsGuard {
    fn drop(&mut self) {
        clear_active_transfer_engine_prefs_override();
    }
}
use insar_store::{
    embed_nisar_subset_prepared_intent_v2_envelope, parse_nisar_subset_plan_v2_envelope, TaskStore,
};
use rusqlite::Connection;
use serde_json::{json, Map};

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        // A per-process sequence keeps names unique when parallel tests read
        // the same clock tick (two tests once shared one new database).
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-scheduler-{}-{nonce}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )))
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = fs::remove_file(format!("{}{}", self.0.display(), suffix));
        }
    }
}

#[derive(Default)]
struct WorkerStats {
    current_by_task: HashMap<String, usize>,
    max_by_task: HashMap<String, usize>,
    ticks_by_task: HashMap<String, usize>,
    global_current: usize,
    global_max: usize,
    starts: Vec<(String, String)>,
}

#[derive(Default)]
struct FakeWorker(Mutex<WorkerStats>);

impl DownloadWorker for FakeWorker {
    fn task_transfer_sample(&self, task_id: &str) -> Option<TaskTransferSample> {
        self.0
            .lock()
            .unwrap()
            .starts
            .iter()
            .any(|(started_task_id, _)| started_task_id == task_id)
            .then_some(TaskTransferSample {
                network_bytes_received: 0,
                active_millis: 1_234,
                active_count: 0,
                bytes_per_second: 0.0,
            })
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        {
            let mut stats = self.0.lock().unwrap();
            stats.global_current += 1;
            stats.global_max = stats.global_max.max(stats.global_current);
            let current = {
                let value = stats
                    .current_by_task
                    .entry(work.task.task_id.clone())
                    .or_default();
                *value += 1;
                *value
            };
            stats
                .max_by_task
                .entry(work.task.task_id.clone())
                .and_modify(|value| *value = (*value).max(current))
                .or_insert(current);
            stats
                .starts
                .push((work.task.task_id.clone(), work.item.item_id.clone()));
        }
        progress(DownloadItemProgress {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            status: DownloadItemStatus::Active,
            bytes_completed: 1,
            total_bytes: Some(1),
            bytes_per_second: 1.0,
            attempt: 1,
            updated_at: "2026-07-14T01:00:00Z".to_string(),
        });
        let until = Instant::now() + Duration::from_millis(60);
        while Instant::now() < until && !control.is_cancelled() {
            if !control.is_paused() {
                thread::sleep(Duration::from_millis(2));
            }
        }
        {
            let mut stats = self.0.lock().unwrap();
            stats.global_current -= 1;
            *stats.current_by_task.get_mut(&work.task.task_id).unwrap() -= 1;
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome: if control.is_cancelled() {
                DownloadOutcome::Cancelled
            } else {
                DownloadOutcome::Completed
            },
            destination: work
                .task
                .output_root
                .join(format!("{}.zip", work.item.item_id)),
            bytes_written: 1,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-14T01:00:01Z".to_string(),
        }
    }
}

fn seed(path: &Path) {
    let mut store = TaskStore::open(path).unwrap();
    let candidates = (1..=5)
        .map(|index| SnapshotItem {
            item_id: format!("S1-{index}"),
            provider_id: "asf.sentinel1".to_string(),
            display_name: format!("S1-{index}.zip"),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        })
        .collect();
    store.save_search_batch(&StoreSearchBatchRequest {
        activate_workbench: true,
        batch: FrozenSearchBatch {
            schema_version: 1,
            batch_id: "batch-scheduler".to_string(),
            provider_id: "asf.sentinel1".to_string(),
            created_at: "2026-07-14T00:00:00Z".to_string(),
            aoi_name: "测试区".to_string(),
            aoi_geojson: Some(
                json!({"type":"Polygon","coordinates":[[[100,30],[101,30],[101,31],[100,31],[100,30]]]}).as_object().unwrap().clone(),
            ),
            query: Map::new(),
            search_stats: None,
            candidates,
        },
        workbench: SearchWorkbenchState {
            batch_id: "batch-scheduler".to_string(),
            checked_item_ids: vec![], page_index: 0, page_size: 24, revision: 0,
        },
    }).unwrap();
    for (task_id, ids, concurrency) in [
        ("task-one", vec!["S1-1", "S1-2", "S1-3"], 1),
        ("task-two", vec!["S1-4", "S1-5"], 2),
    ] {
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: task_id.to_string(),
                batch_id: "batch-scheduler".to_string(),
                selected_item_ids: ids.iter().map(|value| (*value).to_string()).collect(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: path.parent().unwrap().join(task_id),
                layout: DownloadLayout::Sentinel1Slc,
                concurrency,
                created_at: "2026-07-14T00:01:00Z".to_string(),
            })
            .unwrap();
    }
}

struct PauseAwareWorker {
    stats: Mutex<WorkerStats>,
    ticks_per_item: usize,
    tick_delay: Duration,
}
#[derive(Default)]
struct InterruptOnPauseWorker {
    starts: Mutex<HashMap<String, usize>>,
}

impl InterruptOnPauseWorker {
    fn starts(&self, task_id: &str) -> usize {
        self.starts
            .lock()
            .unwrap()
            .get(task_id)
            .copied()
            .unwrap_or(0)
    }
}

impl DownloadWorker for InterruptOnPauseWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let start_number = {
            let mut starts = self.starts.lock().unwrap();
            let value = starts.entry(work.task.task_id.clone()).or_default();
            *value += 1;
            *value
        };
        progress(DownloadItemProgress {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            status: DownloadItemStatus::Active,
            bytes_completed: start_number as u64,
            total_bytes: Some(2),
            bytes_per_second: 1.0,
            attempt: start_number as u64,
            updated_at: "2026-07-14T01:00:00Z".to_owned(),
        });
        let outcome = if start_number == 1 {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !control.is_paused() && !control.is_cancelled() {
                assert!(
                    Instant::now() < deadline,
                    "worker never observed task pause"
                );
                thread::sleep(Duration::from_millis(2));
            }
            if control.is_cancelled() {
                DownloadOutcome::Cancelled
            } else {
                DownloadOutcome::Interrupted
            }
        } else {
            DownloadOutcome::Completed
        };
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome,
            destination: work
                .task
                .output_root
                .join(format!("{}.zip", work.item.item_id)),
            bytes_written: start_number as u64,
            attempt_count: start_number as u64,
            failure: None,
            finished_at: "2026-07-14T01:00:01Z".to_owned(),
        }
    }
}

#[test]
fn scheduler_adds_worker_deltas_to_each_persisted_lifetime_baseline() {
    struct MeasuredWorker(FakeWorker);
    impl DownloadWorker for MeasuredWorker {
        fn task_transfer_sample(&self, task_id: &str) -> Option<TaskTransferSample> {
            self.0.task_transfer_sample(task_id).map(|mut sample| {
                sample.network_bytes_received = 250;
                sample
            })
        }
        fn execute(
            &self,
            work: DownloadWork,
            control: WorkerControl,
            progress: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            self.0.execute(work, control, progress)
        }
    }
    let database = TempDatabase::new();
    seed(&database.0);
    let mut store = TaskStore::open(&database.0).unwrap();
    for (task_id, millis, bytes) in [("task-one", 1000, 10000), ("task-two", 5000, 30000)] {
        store
            .save_task_transfer_telemetry_with_network(
                &insar_store::StoreTaskTransferTelemetryUpdate {
                    task_id: task_id.to_owned(),
                    expected_revision: 0,
                    transfer_active_millis: millis,
                    updated_at: "2026-07-14T00:02:00Z".to_owned(),
                },
                Some(bytes),
            )
            .unwrap();
    }
    drop(store);
    let worker = Arc::new(MeasuredWorker(FakeWorker::default()));
    let scheduler =
        DownloadScheduler::start(&database.0, worker, Duration::from_millis(5)).unwrap();
    scheduler.wake();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let store = TaskStore::open(&database.0).unwrap();
        let one = store.task_transfer_checkpoint_baseline("task-one").unwrap();
        let two = store.task_transfer_checkpoint_baseline("task-two").unwrap();
        if (one == (2234, 10250) && two == (6234, 30250)) || Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    scheduler.shutdown();
    let reopened = TaskStore::open(&database.0).unwrap();
    assert_eq!(
        reopened
            .task_transfer_checkpoint_baseline("task-one")
            .unwrap(),
        (2234, 10250)
    );
    assert_eq!(
        reopened
            .task_transfer_checkpoint_baseline("task-two")
            .unwrap(),
        (6234, 30250)
    );
}

struct StableTelemetryWorker {
    sample: Mutex<(u64, u64)>,
    samples_taken: AtomicUsize,
    release: AtomicBool,
}

impl StableTelemetryWorker {
    fn new(millis: u64, bytes: u64) -> Self {
        Self {
            sample: Mutex::new((millis, bytes)),
            samples_taken: AtomicUsize::new(0),
            release: AtomicBool::new(false),
        }
    }

    fn wait_for_samples(&self, additional: usize, scheduler: &DownloadScheduler) {
        let target = self.samples_taken.load(Ordering::Acquire) + additional;
        let deadline = Instant::now() + Duration::from_secs(8);
        while self.samples_taken.load(Ordering::Acquire) < target {
            assert!(Instant::now() < deadline, "telemetry sampling stopped");
            scheduler.wake();
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl DownloadWorker for StableTelemetryWorker {
    fn task_transfer_sample(&self, task_id: &str) -> Option<TaskTransferSample> {
        if task_id != "task-one" {
            return None;
        }
        let (active_millis, network_bytes_received) = *self.sample.lock().unwrap();
        self.samples_taken.fetch_add(1, Ordering::Release);
        Some(TaskTransferSample {
            active_millis,
            network_bytes_received,
            active_count: 0,
            bytes_per_second: 0.0,
        })
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        while !self.release.load(Ordering::Acquire)
            && !control.is_cancelled()
            && !control.is_paused()
        {
            thread::sleep(Duration::from_millis(2));
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome: if self.release.load(Ordering::Acquire) {
                DownloadOutcome::Completed
            } else {
                DownloadOutcome::Interrupted
            },
            destination: work
                .task
                .output_root
                .join(format!("{}.zip", work.item.item_id)),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-14T01:00:01Z".to_owned(),
        }
    }
}

#[test]
fn scheduler_does_not_rewrite_unchanged_transfer_samples() {
    let database = TempDatabase::new();
    seed(&database.0);
    let audit = Connection::open(&database.0).unwrap();
    audit.execute_batch(
        "CREATE TABLE telemetry_write_audit (millis INTEGER, bytes INTEGER);
         CREATE TRIGGER audit_telemetry_insert AFTER INSERT ON task_transfer_telemetry
         WHEN NEW.task_id = 'task-one' BEGIN
           INSERT INTO telemetry_write_audit VALUES (NEW.transfer_active_millis, NEW.network_bytes_received);
         END;
         CREATE TRIGGER audit_telemetry_update AFTER UPDATE ON task_transfer_telemetry
         WHEN NEW.task_id = 'task-one' BEGIN
           INSERT INTO telemetry_write_audit VALUES (NEW.transfer_active_millis, NEW.network_bytes_received);
         END;",
    ).unwrap();
    let worker = Arc::new(StableTelemetryWorker::new(100, 1000));
    let scheduler =
        DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5)).unwrap();
    worker.wait_for_samples(20, &scheduler);
    *worker.sample.lock().unwrap() = (101, 1000);
    worker.wait_for_samples(20, &scheduler);
    *worker.sample.lock().unwrap() = (101, 1001);
    worker.wait_for_samples(20, &scheduler);
    worker.release.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let complete: bool = audit
            .query_row(
                "SELECT status = 'completed' FROM task_runtime WHERE task_id = 'task-one'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if complete {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal telemetry was not observed"
        );
        thread::sleep(Duration::from_millis(5));
    }
    scheduler.shutdown();
    let writes = audit
        .prepare("SELECT millis, bytes FROM telemetry_write_audit ORDER BY rowid")
        .unwrap()
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    eprintln!(
        "telemetry samples={} successful_writes={}",
        worker.samples_taken.load(Ordering::Acquire),
        writes.len()
    );
    assert_eq!(writes, vec![(100, 1000), (101, 1000), (101, 1001)],
        "unchanged raw samples must not cause another telemetry write after a successful checkpoint");
}

#[test]
fn scheduler_transfer_sample_cache_is_not_reused_after_restart() {
    let database = TempDatabase::new();
    seed(&database.0);
    for (index, expected) in [(0, (100, 1000)), (1, (200, 2000))] {
        if index > 0 {
            let mut store = TaskStore::open(&database.0).unwrap();
            apply_task_command_with_retry(
                &mut store,
                "task-one",
                TaskStateCommandKind::ResumeTask,
                "resume-for-new-worker",
            );
        }
        // Identical raw samples belong to two independent worker lifetimes.
        let worker = Arc::new(StableTelemetryWorker::new(100, 1000));
        let scheduler =
            DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5))
                .unwrap();
        worker.wait_for_samples(20, &scheduler);
        let mut store = TaskStore::open(&database.0).unwrap();
        assert_eq!(
            store.task_transfer_checkpoint_baseline("task-one").unwrap(),
            expected
        );
        for task_id in ["task-one", "task-two"] {
            if store.load_task(task_id).unwrap().unwrap().runtime.status
                != TaskLifecycleStatus::Paused
            {
                apply_task_command_with_retry(
                    &mut store,
                    task_id,
                    TaskStateCommandKind::PauseTask,
                    &format!("pause-before-restart-{index}-{task_id}"),
                );
            }
        }
        drop(store);
        scheduler.shutdown();
    }
}

impl PauseAwareWorker {
    fn new_with_delay(ticks_per_item: usize, tick_delay: Duration) -> Self {
        Self {
            stats: Mutex::new(WorkerStats::default()),
            ticks_per_item,
            tick_delay,
        }
    }

    fn task_ticks(&self, task_id: &str) -> usize {
        self.stats
            .lock()
            .unwrap()
            .ticks_by_task
            .get(task_id)
            .copied()
            .unwrap_or(0)
    }

    fn task_starts(&self, task_id: &str) -> usize {
        self.stats
            .lock()
            .unwrap()
            .starts
            .iter()
            .filter(|(started_task_id, _)| started_task_id == task_id)
            .count()
    }
}

impl DownloadWorker for PauseAwareWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        {
            let mut stats = self.stats.lock().unwrap();
            stats.global_current += 1;
            stats.global_max = stats.global_max.max(stats.global_current);
            let current = {
                let value = stats
                    .current_by_task
                    .entry(work.task.task_id.clone())
                    .or_default();
                *value += 1;
                *value
            };
            stats
                .max_by_task
                .entry(work.task.task_id.clone())
                .and_modify(|value| *value = (*value).max(current))
                .or_insert(current);
            stats
                .starts
                .push((work.task.task_id.clone(), work.item.item_id.clone()));
        }

        let mut ticks = 0;
        while ticks < self.ticks_per_item && !control.is_cancelled() {
            if control.is_paused() {
                thread::sleep(Duration::from_millis(2));
                continue;
            }
            ticks += 1;
            {
                let mut stats = self.stats.lock().unwrap();
                *stats
                    .ticks_by_task
                    .entry(work.task.task_id.clone())
                    .or_default() += 1;
            }
            progress(DownloadItemProgress {
                task_id: work.task.task_id.clone(),
                item_id: work.item.item_id.clone(),
                status: DownloadItemStatus::Active,
                bytes_completed: ticks as u64,
                total_bytes: Some(self.ticks_per_item as u64),
                bytes_per_second: 1.0,
                attempt: 1,
                updated_at: "2026-07-14T01:00:00Z".to_string(),
            });
            thread::sleep(self.tick_delay);
        }

        {
            let mut stats = self.stats.lock().unwrap();
            stats.global_current -= 1;
            *stats.current_by_task.get_mut(&work.task.task_id).unwrap() -= 1;
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome: if control.is_cancelled() {
                DownloadOutcome::Cancelled
            } else {
                DownloadOutcome::Completed
            },
            destination: work
                .task
                .output_root
                .join(format!("{}.zip", work.item.item_id)),
            bytes_written: ticks as u64,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-14T01:00:01Z".to_string(),
        }
    }
}

#[derive(Default)]
struct CheckpointFaultWorker {
    task_one_started: AtomicBool,
    release_task_one: AtomicBool,
    starts: Mutex<Vec<String>>,
}

impl CheckpointFaultWorker {
    fn started(&self, task_id: &str) -> bool {
        self.starts
            .lock()
            .unwrap()
            .iter()
            .any(|started| started == task_id)
    }
}

impl DownloadWorker for CheckpointFaultWorker {
    fn task_transfer_sample(&self, task_id: &str) -> Option<TaskTransferSample> {
        self.started(task_id).then_some(TaskTransferSample {
            network_bytes_received: 0,
            active_millis: 4_321,
            active_count: 0,
            bytes_per_second: 0.0,
        })
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.starts.lock().unwrap().push(work.task.task_id.clone());
        if work.task.task_id == "task-one" {
            self.task_one_started.store(true, Ordering::Release);
        }
        progress(DownloadItemProgress {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            status: DownloadItemStatus::Active,
            bytes_completed: 1,
            total_bytes: Some(1),
            bytes_per_second: 1.0,
            attempt: 1,
            updated_at: "2026-07-14T01:00:00Z".to_owned(),
        });
        while work.task.task_id == "task-one"
            && !self.release_task_one.load(Ordering::Acquire)
            && !control.is_cancelled()
        {
            thread::sleep(Duration::from_millis(2));
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome: if control.is_cancelled() {
                DownloadOutcome::Cancelled
            } else {
                DownloadOutcome::Completed
            },
            destination: work
                .task
                .output_root
                .join(format!("{}.zip", work.item.item_id)),
            bytes_written: 1,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-14T01:00:01Z".to_owned(),
        }
    }
}

fn corrupt_active_workbench(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER search_batches_are_frozen;
             UPDATE search_batches
             SET snapshot_json = '{not-json'
             WHERE batch_id = 'batch-scheduler';
             CREATE TRIGGER search_batches_are_frozen
             BEFORE UPDATE ON search_batches
             BEGIN
                 SELECT RAISE(ABORT, 'frozen search batch cannot be updated');
             END;",
        )
        .unwrap();
}

#[test]
fn scheduler_does_not_reload_the_unrelated_workbench() {
    let database = TempDatabase::new();
    seed(&database.0);
    corrupt_active_workbench(&database.0);

    let scheduler_tasks = TaskStore::open(&database.0)
        .unwrap()
        .load_scheduler_tasks()
        .unwrap();
    assert_eq!(scheduler_tasks.len(), 2);

    let worker = Arc::new(FakeWorker::default());
    let scheduler =
        DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5)).unwrap();
    scheduler.wake();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let connection = Connection::open(&database.0).unwrap();
        let completed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM task_runtime WHERE task_id IN ('task-one', 'task-two') AND status = 'completed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        if completed == 2 {
            break;
        }
        if Instant::now() >= deadline {
            let statuses = connection
                .prepare("SELECT task_id, status, state_json FROM task_runtime ORDER BY task_id")
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let starts = worker.0.lock().unwrap().starts.clone();
            panic!(
                "scheduler must not deserialize the unrelated active workbench; statuses={statuses:?}; starts={starts:?}"
            );
        }
        thread::sleep(Duration::from_millis(10));
    }

    scheduler.shutdown();
    assert_eq!(worker.0.lock().unwrap().starts.len(), 5);
    assert!(
        TaskStore::open(&database.0)
            .unwrap()
            .load_scheduler_tasks()
            .unwrap()
            .is_empty(),
        "terminal tasks must leave the scheduler hot path even before history archiving",
    );
    let store = TaskStore::open(&database.0).unwrap();
    assert_eq!(
        store
            .load_download_snapshot("task-one")
            .unwrap()
            .results
            .len(),
        3,
        "progress/result persistence must not deserialize the catalogue batch",
    );
    assert_eq!(
        store
            .load_download_snapshot("task-two")
            .unwrap()
            .results
            .len(),
        2,
        "terminal results must not sit behind failed catalogue hydration",
    );
}

#[test]
fn independent_tasks_run_concurrently_without_exceeding_each_task_limit() {
    let database = TempDatabase::new();
    seed(&database.0);
    let worker = Arc::new(FakeWorker::default());
    let scheduler =
        DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5)).unwrap();
    scheduler.wake();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let store = TaskStore::open(&database.0).unwrap();
        let one = store.load_task("task-one").unwrap().unwrap();
        let two = store.load_task("task-two").unwrap().unwrap();
        if one.runtime.status == TaskLifecycleStatus::Completed
            && two.runtime.status == TaskLifecycleStatus::Completed
        {
            assert_eq!(
                store
                    .load_download_snapshot("task-one")
                    .unwrap()
                    .results
                    .len(),
                3
            );
            assert_eq!(
                store
                    .load_download_snapshot("task-two")
                    .unwrap()
                    .results
                    .len(),
                2
            );
            let summaries = store.core_restore(true).unwrap().tasks;
            assert_eq!(
                summaries
                    .iter()
                    .find(|task| task.task_id == "task-one")
                    .unwrap()
                    .transfer_active_millis,
                1_234
            );
            assert_eq!(
                summaries
                    .iter()
                    .find(|task| task.task_id == "task-two")
                    .unwrap()
                    .transfer_active_millis,
                1_234
            );
            break;
        }
        assert!(Instant::now() < deadline, "scheduler did not finish");
        thread::sleep(Duration::from_millis(10));
    }
    scheduler.shutdown();
    let stats = worker.0.lock().unwrap();
    assert_eq!(stats.max_by_task["task-one"], 1);
    assert!(stats.max_by_task["task-two"] <= 2);
    assert!(stats.global_max >= 2, "one task must not block another");
    assert_eq!(stats.starts.len(), 5);
}

/// Review of Issue #8 slice 6 (P2, fixed in slice 7): when the terminal
/// result of a finished item cannot be recorded (here the Store refuses the
/// completion event), the item stays active and the scheduler used to
/// dispatch it again at once, in a loop. It now waits 1 s, doubling, before
/// the next dispatch, and the item completes once the Store accepts it.
#[test]
fn unrecorded_completion_is_redispatched_with_bounded_backoff() {
    let database = TempDatabase::new();
    seed(&database.0);
    let connection = Connection::open(&database.0).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER block_mark_completed BEFORE INSERT ON task_events \
             WHEN NEW.event_type = 'mark_completed' \
             BEGIN SELECT RAISE(ABORT, 'injected completion failure'); END;",
        )
        .unwrap();
    let worker = Arc::new(FakeWorker::default());
    let scheduler =
        DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5)).unwrap();
    scheduler.wake();
    thread::sleep(Duration::from_millis(2_500));
    let first_item_starts = worker
        .0
        .lock()
        .unwrap()
        .starts
        .iter()
        .filter(|(task_id, item_id)| task_id == "task-one" && item_id == "S1-1")
        .count();
    assert!(
        (1..=3).contains(&first_item_starts),
        "{first_item_starts} dispatches of one unrecorded item in 2.5 s"
    );
    connection
        .execute_batch("DROP TRIGGER block_mark_completed;")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let store = TaskStore::open(&database.0).unwrap();
        let one = store.load_task("task-one").unwrap().unwrap();
        let two = store.load_task("task-two").unwrap().unwrap();
        if one.runtime.status == TaskLifecycleStatus::Completed
            && two.runtime.status == TaskLifecycleStatus::Completed
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the recorded completions did not settle: {:?} {:?}",
            one.runtime,
            two.runtime
        );
        thread::sleep(Duration::from_millis(20));
    }
    scheduler.shutdown();
}

#[test]
fn pausing_one_task_does_not_block_another_and_resume_reuses_its_capacity() {
    let database = TempDatabase::new();
    seed(&database.0);
    let mut store = TaskStore::open(&database.0).unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-one".to_string(),
            expected_revision: 0,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "pause-before-start".to_string(),
            occurred_at: "2026-07-14T00:02:00Z".to_string(),
        })
        .unwrap();
    drop(store);
    let worker = Arc::new(FakeWorker::default());
    let scheduler =
        DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let store = TaskStore::open(&database.0).unwrap();
        if store.load_task("task-two").unwrap().unwrap().runtime.status
            == TaskLifecycleStatus::Completed
        {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(worker
        .0
        .lock()
        .unwrap()
        .starts
        .iter()
        .all(|(task_id, _)| task_id != "task-one"));

    let mut store = TaskStore::open(&database.0).unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-one".to_string(),
            expected_revision: 1,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: vec![],
            },
            event_id: "resume-after-second".to_string(),
            occurred_at: "2026-07-14T00:03:00Z".to_string(),
        })
        .unwrap();
    drop(store);
    scheduler.wake();
    loop {
        let store = TaskStore::open(&database.0).unwrap();
        if store.load_task("task-one").unwrap().unwrap().runtime.status
            == TaskLifecycleStatus::Completed
        {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    scheduler.shutdown();
}

/// Apply a task command that targets the active scheduler revision, retrying
/// with a fresh revision when the scheduler advances the task between the
/// read and the write. The scheduler legitimately commits progress revisions
/// while a task is active, so under parallel test execution the single-shot
/// compare-and-swap can lose the race.
fn apply_task_command_with_retry(
    store: &mut TaskStore,
    task_id: &str,
    kind: TaskStateCommandKind,
    event_id: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let current = store.load_task(task_id).unwrap().unwrap();
        let result = store.update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: current.runtime.revision,
            command: StoreTaskCommand {
                kind: kind.clone(),
                item_ids: vec![],
            },
            event_id: event_id.to_owned(),
            occurred_at: "2026-07-14T00:02:00Z".to_owned(),
        });
        if result.is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "task command {kind:?} could not win a revision race against the active scheduler"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn pausing_an_active_task_stops_progress_without_claiming_another_item() {
    let database = TempDatabase::new();
    seed(&database.0);
    // A 2 ms worker progress cadence commits a new task revision almost
    // continuously, so a compare-and-swap pause cannot deterministically land
    // between commits under parallel test execution. A 50 ms tick delay still
    // proves pausing an active task while leaving a large idle window for the
    // pause command to win the revision race.
    let worker = Arc::new(PauseAwareWorker::new_with_delay(
        80,
        Duration::from_millis(50),
    ));
    let scheduler =
        DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5)).unwrap();
    scheduler.wake();

    let start_deadline = Instant::now() + Duration::from_secs(10);
    while worker.task_ticks("task-one") < 5 || worker.task_ticks("task-two") < 5 {
        assert!(Instant::now() < start_deadline, "workers did not begin");
        thread::sleep(Duration::from_millis(5));
    }
    let task_two_ticks_before_pause = worker.task_ticks("task-two");

    let mut store = TaskStore::open(&database.0).unwrap();
    apply_task_command_with_retry(
        &mut store,
        "task-one",
        TaskStateCommandKind::PauseTask,
        "pause-active-task",
    );
    let starts_when_paused = worker.task_starts("task-one");
    drop(store);
    scheduler.wake();

    // Wait until the paused task's tick count stays stable for longer than one
    // full worker tick period (50 ms), then re-check after a settle window so
    // a tick that was already in flight cannot flip the frozen assertion.
    let pause_deadline = Instant::now() + Duration::from_secs(5);
    let mut paused_ticks = worker.task_ticks("task-one");
    let mut last_change = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(25));
        let current = worker.task_ticks("task-one");
        if current != paused_ticks {
            paused_ticks = current;
            last_change = Instant::now();
        } else if last_change.elapsed() >= Duration::from_millis(120) {
            break;
        }
        assert!(
            Instant::now() < pause_deadline,
            "active task did not observe pause within one controlled worker cycle"
        );
    }
    thread::sleep(Duration::from_millis(60));
    assert_eq!(worker.task_ticks("task-one"), paused_ticks);
    assert_eq!(
        worker.task_starts("task-one"),
        starts_when_paused,
        "paused task must not be started again after the pause lands"
    );

    let task_two_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let store = TaskStore::open(&database.0).unwrap();
        if store.load_task("task-two").unwrap().unwrap().runtime.status
            == TaskLifecycleStatus::Completed
        {
            break;
        }
        assert!(
            Instant::now() < task_two_deadline,
            "the independent task stopped while task-one was paused"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(worker.task_ticks("task-two") > task_two_ticks_before_pause);

    let mut store = TaskStore::open(&database.0).unwrap();
    apply_task_command_with_retry(
        &mut store,
        "task-one",
        TaskStateCommandKind::ResumeTask,
        "resume-active-task",
    );
    drop(store);
    scheduler.wake();

    let completion_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let store = TaskStore::open(&database.0).unwrap();
        if store.load_task("task-one").unwrap().unwrap().runtime.status
            == TaskLifecycleStatus::Completed
        {
            break;
        }
        assert!(
            Instant::now() < completion_deadline,
            "resumed task did not finish"
        );
        thread::sleep(Duration::from_millis(10));
    }
    scheduler.shutdown();
    assert_eq!(worker.stats.lock().unwrap().max_by_task["task-one"], 1);
}

#[test]
fn interrupted_active_item_from_task_pause_becomes_durably_paused_and_resumable() {
    let database = TempDatabase::new();
    seed(&database.0);
    let mut store = TaskStore::open(&database.0).unwrap();
    apply_task_command_with_retry(
        &mut store,
        "task-two",
        TaskStateCommandKind::PauseTask,
        "keep-task-two-out-of-this-test",
    );
    drop(store);

    let worker = Arc::new(InterruptOnPauseWorker::default());
    let scheduler =
        DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5)).unwrap();
    scheduler.wake();
    let started_deadline = Instant::now() + Duration::from_secs(5);
    while worker.starts("task-one") == 0 {
        assert!(Instant::now() < started_deadline, "worker did not start");
        thread::sleep(Duration::from_millis(2));
    }

    let mut store = TaskStore::open(&database.0).unwrap();
    apply_task_command_with_retry(
        &mut store,
        "task-one",
        TaskStateCommandKind::PauseTask,
        "pause-interrupting-worker",
    );
    drop(store);
    scheduler.wake();

    let paused_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let task = TaskStore::open(&database.0)
            .unwrap()
            .load_task("task-one")
            .unwrap()
            .unwrap();
        if task.runtime.paused_item_ids.contains(&"S1-1".to_owned())
            && !task.runtime.active_item_ids.contains(&"S1-1".to_owned())
        {
            assert_eq!(task.runtime.status, TaskLifecycleStatus::Paused);
            break;
        }
        assert!(
            Instant::now() < paused_deadline,
            "interrupted active item never became durable paused: active={:?} paused={:?}",
            task.runtime.active_item_ids,
            task.runtime.paused_item_ids
        );
        thread::sleep(Duration::from_millis(5));
    }

    let mut store = TaskStore::open(&database.0).unwrap();
    let current = store.load_task("task-one").unwrap().unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-one".to_owned(),
            expected_revision: current.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeTask,
                item_ids: vec![],
            },
            event_id: "resume-interrupted-task".to_owned(),
            occurred_at: "2026-07-14T00:04:00Z".to_owned(),
        })
        .unwrap();
    let current = store.load_task("task-one").unwrap().unwrap();
    let paused = current.runtime.paused_item_ids.clone();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-one".to_owned(),
            expected_revision: current.runtime.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::ResumeItems,
                item_ids: paused,
            },
            event_id: "resume-interrupted-items".to_owned(),
            occurred_at: "2026-07-14T00:04:01Z".to_owned(),
        })
        .unwrap();
    drop(store);
    scheduler.wake();

    let completion_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let task = TaskStore::open(&database.0)
            .unwrap()
            .load_task("task-one")
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Completed {
            break;
        }
        assert!(
            Instant::now() < completion_deadline,
            "resumed interrupted task did not complete: {:?}",
            task.runtime
        );
        thread::sleep(Duration::from_millis(5));
    }
    scheduler.shutdown();
    assert!(
        worker.starts("task-one") >= 4,
        "first item should restart after pause"
    );
}
#[test]
fn failed_transfer_checkpoint_does_not_starve_other_tasks_and_retries_after_terminal() {
    let database = TempDatabase::new();
    seed(&database.0);
    let mut store = TaskStore::open(&database.0).unwrap();
    apply_task_command_with_retry(
        &mut store,
        "task-two",
        TaskStateCommandKind::PauseTask,
        "pause-task-two-before-checkpoint-fault",
    );
    drop(store);
    Connection::open(&database.0)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_task_one_transfer_checkpoint
             BEFORE INSERT ON task_transfer_telemetry
             WHEN NEW.task_id = 'task-one'
             BEGIN
                 SELECT RAISE(ABORT, 'injected task-one telemetry failure');
             END;",
        )
        .unwrap();

    let worker = Arc::new(CheckpointFaultWorker::default());
    let scheduler =
        DownloadScheduler::start(&database.0, worker.clone(), Duration::from_millis(5)).unwrap();
    scheduler.wake();

    let task_one_start_deadline = Instant::now() + Duration::from_secs(5);
    while !worker.task_one_started.load(Ordering::Acquire) {
        assert!(
            Instant::now() < task_one_start_deadline,
            "task-one did not start"
        );
        thread::sleep(Duration::from_millis(5));
    }

    let mut store = TaskStore::open(&database.0).unwrap();
    apply_task_command_with_retry(
        &mut store,
        "task-two",
        TaskStateCommandKind::ResumeTask,
        "resume-task-two-during-checkpoint-fault",
    );
    drop(store);
    scheduler.wake();

    let independent_deadline = Instant::now() + Duration::from_secs(2);
    while !worker.started("task-two") {
        assert!(
            Instant::now() < independent_deadline,
            "task-two was starved by task-one's transfer checkpoint failure"
        );
        thread::sleep(Duration::from_millis(5));
    }
    let task_two_completion_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let task = TaskStore::open(&database.0)
            .unwrap()
            .load_task("task-two")
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Completed {
            break;
        }
        assert!(
            Instant::now() < task_two_completion_deadline,
            "independent task did not complete during task-one's checkpoint failure"
        );
        thread::sleep(Duration::from_millis(5));
    }

    worker.release_task_one.store(true, Ordering::Release);
    scheduler.wake();
    let task_one_completion_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let task = TaskStore::open(&database.0)
            .unwrap()
            .load_task("task-one")
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Completed {
            break;
        }
        assert!(
            Instant::now() < task_one_completion_deadline,
            "task-one did not reach terminal state while checkpointing failed"
        );
        thread::sleep(Duration::from_millis(5));
    }

    let connection = Connection::open(&database.0).unwrap();
    let checkpoint_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM task_transfer_telemetry WHERE task_id = 'task-one'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(checkpoint_count, 0);
    connection
        .execute_batch("DROP TRIGGER fail_task_one_transfer_checkpoint;")
        .unwrap();
    drop(connection);
    scheduler.wake();

    let retry_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let connection = Connection::open(&database.0).unwrap();
        let transfer_active_millis: Option<i64> = connection
            .query_row(
                "SELECT transfer_active_millis FROM task_transfer_telemetry WHERE task_id = 'task-one'",
                [],
                |row| row.get(0),
            )
            .ok();
        if transfer_active_millis == Some(4_321) {
            break;
        }
        assert!(
            Instant::now() < retry_deadline,
            "terminal task transfer checkpoint was forgotten after its worker was removed"
        );
        thread::sleep(Duration::from_millis(5));
    }

    scheduler.shutdown();
}

const NISAR_V2_ITEM_ID: &str = "PROVIDER::asf::ITEM::G9900000001-ASF";
const NISAR_V2_ITEM_ID_2: &str = "PROVIDER::asf::ITEM::G9900000002-ASF";

fn nisar_map(value: serde_json::Value) -> Map<String, serde_json::Value> {
    value.as_object().unwrap().clone()
}

fn nisar_rectangle(west: f64, south: f64, east: f64, north: f64) -> Map<String, serde_json::Value> {
    nisar_map(json!({
        "type": "Polygon",
        "coordinates": [[[west, south], [east, south], [east, north], [west, north], [west, south]]]
    }))
}

fn nisar_dependency_graph() -> NisarSubsetSourceDependencyGraphV2 {
    use NisarSubsetDependencyRelationV2 as Relation;
    use NisarSubsetSourceDependencyNodeKindV2 as Kind;

    let science = "/science/LSAR/RSLC/swaths/frequencyA/HH";
    let dependencies = [
        (
            "/science/LSAR/RSLC/metadata/geolocationGrid/frequencyA/coordinateX",
            Kind::CoordinateDataset,
            Relation::Coordinate,
        ),
        (
            "/science/LSAR/RSLC/swaths/frequencyA/zeroDopplerTime",
            Kind::DimensionScale,
            Relation::DimensionScale,
        ),
        (
            "/science/LSAR/RSLC/swaths/frequencyA/inputDataExceptionMask",
            Kind::ValidityMask,
            Relation::ValidityMask,
        ),
        (
            "/science/LSAR/identification/granuleId",
            Kind::IdentificationMetadata,
            Relation::RequiredMetadata,
        ),
        (
            "/science/LSAR/identification/productVersion",
            Kind::ProvenanceMetadata,
            Relation::Provenance,
        ),
        ("/", Kind::Attribute, Relation::RequiredAttribute),
        (
            "/science/LSAR/RSLC/metadata/orbit",
            Kind::OtherScientificDependency,
            Relation::OtherScientificDependency,
        ),
    ];
    let mut nodes = vec![NisarSubsetSourceDependencyNodeV2 {
        source_path: science.to_owned(),
        kind: Kind::ScienceDataset,
        attribute_name: None,
    }];
    let mut edges = Vec::new();
    for (dependency_path, kind, relation) in dependencies {
        nodes.push(NisarSubsetSourceDependencyNodeV2 {
            source_path: dependency_path.to_owned(),
            kind,
            attribute_name: (kind == Kind::Attribute).then(|| "Conventions".to_owned()),
        });
        edges.push(NisarSubsetSourceDependencyEdgeV2 {
            dependent_path: science.to_owned(),
            dependency_path: dependency_path.to_owned(),
            relation,
        });
    }
    NisarSubsetSourceDependencyGraphV2 {
        graph_contract: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_CONTRACT.to_owned(),
        schema_version: NISAR_SUBSET_SOURCE_DEPENDENCY_GRAPH_V2_SCHEMA_VERSION,
        evidence_authority: NisarSubsetDependencyEvidenceAuthorityV2::ReviewedSourceEvidenceV1,
        evidence_sha256: "c".repeat(64),
        nodes,
        edges,
    }
}

fn nisar_prepared_intent() -> NisarSubsetPreparedIntentV2 {
    nisar_prepared_intent_for(NISAR_V2_ITEM_ID, "G9900000001-ASF", "NISAR_SOURCE.h5")
}

fn nisar_prepared_intent_for(
    item_id: &str,
    provider_product_id: &str,
    file_name: &str,
) -> NisarSubsetPreparedIntentV2 {
    NisarSubsetPreparedIntentV2::build(NisarSubsetPreparedIntentBuildInputV2 {
        source_product_identity: NisarSubsetSourceProductIdentityV2 {
            provider_id: "asf.nasa.earthdata".to_owned(),
            collection_id: "C2853145197-ASF".to_owned(),
            collection_profile_id: "asf.nisar.l.rslc.provisional.v1".to_owned(),
            provider_product_id: provider_product_id.to_owned(),
            provider_product_key: item_id.to_owned(),
            product_specification_version: "1.5.0".to_owned(),
            crid: "P05023".to_owned(),
            quality_status: NisarSubsetQualityStatusV2::Provisional,
        },
        source_asset_identity: NisarSubsetSourceAssetIdentityV2 {
            asset_key: "science-rslc-hdf5".to_owned(),
            stable_asset_key: format!("{item_id}::ASSET::science-rslc-hdf5"),
            file_name: file_name.to_owned(),
            checksum: None,
        },
        source_capability: NisarSubsetSourceCapabilityV2 {
            evidence_sha256: "b".repeat(64),
            frequencies: vec![NisarSubsetFrequencyCapabilityV2 {
                frequency: NisarSubsetFrequency::A,
                available_polarizations: vec!["HH".to_owned()],
            }],
        },
        source_footprint: nisar_rectangle(0.0, 0.0, 10.0, 10.0),
        requested_geometry: nisar_rectangle(1.0, 1.0, 2.0, 2.0),
        selected_frequency: NisarSubsetFrequency::A,
        selected_polarizations: vec!["HH".to_owned()],
    })
    .unwrap()
}

fn nisar_plan(intent: NisarSubsetPreparedIntentV2) -> NisarSubsetPlanV2 {
    NisarSubsetPlanV2::build_from_prepared_intent(
        intent,
        nisar_dependency_graph(),
        NisarSubsetPlanMetadataEvidenceV2 {
            source_estimate: None,
            transfer_estimate: None,
            derived_estimate: None,
            warnings: vec!["fixture-only reviewed source evidence".to_owned()],
        },
    )
    .unwrap()
}

fn nisar_source_item() -> SnapshotItem {
    nisar_source_item_for(NISAR_V2_ITEM_ID)
}

fn nisar_source_item_for(item_id: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: item_id.to_owned(),
        provider_id: "asf.nasa.earthdata".to_owned(),
        display_name: "NISAR source".to_owned(),
        acquisition_datetime: Some("2026-07-26T15:58:03Z".to_owned()),
        footprint_geojson: Some(nisar_rectangle(0.0, 0.0, 10.0, 10.0)),
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn seed_nisar_paused_task(
    store: &mut TaskStore,
    database: &Path,
    task_id: &str,
) -> (NisarSubsetPreparedIntentV2, NisarSubsetPlanV2) {
    let intent = nisar_prepared_intent();
    let plan = nisar_plan(intent.clone());
    let source = nisar_source_item();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "nisar-plan-freeze-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-08-30T00:00:00Z".to_owned(),
                aoi_name: "NISAR fixture AOI".to_owned(),
                aoi_geojson: Some(nisar_rectangle(1.0, 1.0, 2.0, 2.0)),
                query: Map::new(),
                search_stats: None,
                candidates: vec![source.clone()],
            },
            workbench: SearchWorkbenchState {
                batch_id: "nisar-plan-freeze-batch".to_owned(),
                checked_item_ids: vec![NISAR_V2_ITEM_ID.to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    let mut task_item = source;
    task_item
        .metadata
        .insert("source_item_id".to_owned(), json!(NISAR_V2_ITEM_ID));
    embed_nisar_subset_prepared_intent_v2_envelope(&mut task_item.metadata, &intent).unwrap();
    store
        .create_paused_task_with_explicit_items(
            &StoreCreateTaskRequest {
                task_id: task_id.to_owned(),
                batch_id: "nisar-plan-freeze-batch".to_owned(),
                selected_item_ids: vec![NISAR_V2_ITEM_ID.to_owned()],
                checked_item_ids: vec![NISAR_V2_ITEM_ID.to_owned()],
                page_index: 0,
                page_size: 24,
                output_root: database.with_extension("output"),
                layout: DownloadLayout::Direct,
                concurrency: 1,
                created_at: "2026-08-30T00:00:01Z".to_owned(),
            },
            0,
            &[NISAR_V2_ITEM_ID.to_owned()],
            &[task_item],
        )
        .unwrap();
    (intent, plan)
}

fn resume_nisar_task_for_scheduler(store: &mut TaskStore, task_id: &str) {
    for (expected_revision, kind, item_ids, event) in [
        (0, TaskStateCommandKind::ResumeTask, vec![], "resume-task"),
        (
            1,
            TaskStateCommandKind::ResumeItems,
            vec![NISAR_V2_ITEM_ID.to_owned()],
            "resume-item",
        ),
    ] {
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision,
                command: StoreTaskCommand { kind, item_ids },
                event_id: format!("{task_id}:{event}"),
                occurred_at: "2026-08-30T00:00:02Z".to_owned(),
            })
            .unwrap();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PlanWorkerCommand {
    Propose,
    Retry,
    EmitProgress,
    Finish,
}

#[derive(Debug)]
enum PlanWorkerEvent {
    Started,
    Frozen(NisarSubsetPlanV2),
    Retried(NisarSubsetPlanV2),
    Rejected(String),
    ProgressEmitted,
}

struct PlanFreezeFixtureWorker {
    plan: NisarSubsetPlanV2,
    commands: Mutex<mpsc::Receiver<PlanWorkerCommand>>,
    events: mpsc::Sender<PlanWorkerEvent>,
}

impl PlanFreezeFixtureWorker {
    fn next_command(&self) -> PlanWorkerCommand {
        self.commands
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .expect("plan fixture command deadline exceeded")
    }

    fn request(&self, work: &DownloadWork) -> NisarExecutionPlanReadyV2 {
        NisarExecutionPlanReadyV2 {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            prepared_intent_sha256: self.plan.prepared_intent_sha256.clone(),
            proposed_plan: self.plan.clone(),
        }
    }

    fn result(work: &DownloadWork, outcome: DownloadOutcome) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            outcome: outcome.clone(),
            destination: work.task.output_root.join("NISAR_DERIVED.h5"),
            bytes_written: u64::from(outcome == DownloadOutcome::Completed) * 2,
            attempt_count: 1,
            failure: (outcome == DownloadOutcome::Failed).then_some(DownloadFailure {
                code: "NISAR_PLAN_FREEZE_REJECTED".to_owned(),
                reason_zh: "NISAR execution plan freeze rejected".to_owned(),
                retryable: false,
            }),
            finished_at: "2026-08-30T00:00:05Z".to_owned(),
        }
    }

    fn rejection_code(error: &NisarExecutionPlanFreezeError) -> String {
        match error {
            NisarExecutionPlanFreezeError::Store(error) => error.code().to_owned(),
            NisarExecutionPlanFreezeError::CoordinatorUnavailable => {
                "NISAR_PLAN_FREEZE_COORDINATOR_UNAVAILABLE".to_owned()
            }
            NisarExecutionPlanFreezeError::ExecutionStopped => {
                "NISAR_PLAN_FREEZE_EXECUTION_STOPPED".to_owned()
            }
        }
    }
}

impl DownloadWorker for PlanFreezeFixtureWorker {
    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        plan_freeze: &NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        self.events.send(PlanWorkerEvent::Started).unwrap();
        assert_eq!(self.next_command(), PlanWorkerCommand::Propose);
        let frozen = match plan_freeze.freeze(self.request(&work), &control) {
            Ok(frozen) => frozen,
            Err(error) => {
                self.events
                    .send(PlanWorkerEvent::Rejected(Self::rejection_code(&error)))
                    .unwrap();
                return Self::result(&work, DownloadOutcome::Failed);
            }
        };
        self.events.send(PlanWorkerEvent::Frozen(frozen)).unwrap();

        loop {
            match self.next_command() {
                PlanWorkerCommand::Propose => panic!("plan may be proposed only once per fixture"),
                PlanWorkerCommand::Retry => {
                    let frozen = match plan_freeze.freeze(self.request(&work), &control) {
                        Ok(frozen) => frozen,
                        Err(error) => {
                            self.events
                                .send(PlanWorkerEvent::Rejected(Self::rejection_code(&error)))
                                .unwrap();
                            return Self::result(&work, DownloadOutcome::Failed);
                        }
                    };
                    self.events.send(PlanWorkerEvent::Retried(frozen)).unwrap();
                }
                PlanWorkerCommand::EmitProgress => {
                    progress(DownloadItemProgress {
                        task_id: work.task.task_id.clone(),
                        item_id: work.item.item_id.clone(),
                        status: DownloadItemStatus::Active,
                        bytes_completed: 1,
                        total_bytes: Some(2),
                        bytes_per_second: 1.0,
                        attempt: 1,
                        updated_at: "2026-08-30T00:00:04Z".to_owned(),
                    });
                    self.events.send(PlanWorkerEvent::ProgressEmitted).unwrap();
                }
                PlanWorkerCommand::Finish => {
                    return Self::result(&work, DownloadOutcome::Completed);
                }
            }
        }
    }

    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        Self::result(&work, DownloadOutcome::Failed)
    }
}

fn plan_fixture_worker(
    plan: NisarSubsetPlanV2,
) -> (
    Arc<PlanFreezeFixtureWorker>,
    mpsc::Sender<PlanWorkerCommand>,
    mpsc::Receiver<PlanWorkerEvent>,
) {
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    (
        Arc::new(PlanFreezeFixtureWorker {
            plan,
            commands: Mutex::new(command_receiver),
            events: event_sender,
        }),
        command_sender,
        event_receiver,
    )
}

fn receive_plan_event(events: &mpsc::Receiver<PlanWorkerEvent>) -> PlanWorkerEvent {
    events
        .recv_timeout(Duration::from_secs(5))
        .expect("plan fixture event deadline exceeded")
}

#[test]
fn scheduler_freezes_and_reloads_the_active_plan_before_progress_and_keeps_new_revision_live() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(&database.0).unwrap();
    let (intent, plan) = seed_nisar_paused_task(&mut store, &database.0, "nisar-plan-lifecycle");
    resume_nisar_task_for_scheduler(&mut store, "nisar-plan-lifecycle");
    let shared_store = Arc::new(Mutex::new(store));
    let (worker, commands, events) = plan_fixture_worker(plan.clone());
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&shared_store),
        worker,
        Duration::from_millis(5),
    );
    scheduler.wake();

    assert!(matches!(
        receive_plan_event(&events),
        PlanWorkerEvent::Started
    ));
    {
        let store = shared_store.lock().unwrap();
        let active = store
            .load_scheduler_task("nisar-plan-lifecycle")
            .unwrap()
            .unwrap();
        assert_eq!(active.runtime.status, TaskLifecycleStatus::Running);
        assert_eq!(active.runtime.active_item_ids, [NISAR_V2_ITEM_ID]);
        assert_eq!(active.runtime.revision, 3);
        assert!(
            parse_nisar_subset_plan_v2_envelope(&active.snapshot.candidates[0].metadata)
                .unwrap()
                .is_none()
        );
    }

    commands.send(PlanWorkerCommand::Propose).unwrap();
    match receive_plan_event(&events) {
        PlanWorkerEvent::Frozen(frozen) => assert_eq!(frozen, plan),
        event => panic!("unexpected first freeze event: {event:?}"),
    }
    {
        let store = shared_store.lock().unwrap();
        let frozen = store
            .load_scheduler_task("nisar-plan-lifecycle")
            .unwrap()
            .unwrap();
        assert_eq!(frozen.runtime.status, TaskLifecycleStatus::Running);
        assert!(!frozen.runtime.task_paused);
        assert_eq!(frozen.runtime.active_item_ids, [NISAR_V2_ITEM_ID]);
        assert_eq!(frozen.runtime.revision, 4);
        let envelope = parse_nisar_subset_plan_v2_envelope(&frozen.snapshot.candidates[0].metadata)
            .unwrap()
            .unwrap();
        assert_eq!(
            envelope.prepared_intent_sha256,
            intent.canonical_intent_sha256
        );
        assert_eq!(envelope.plan().unwrap(), plan);
    }

    commands.send(PlanWorkerCommand::Retry).unwrap();
    match receive_plan_event(&events) {
        PlanWorkerEvent::Retried(frozen) => assert_eq!(frozen, plan),
        event => panic!("unexpected idempotent retry event: {event:?}"),
    }
    assert_eq!(
        shared_store
            .lock()
            .unwrap()
            .load_scheduler_task("nisar-plan-lifecycle")
            .unwrap()
            .unwrap()
            .runtime
            .revision,
        4,
    );

    commands.send(PlanWorkerCommand::EmitProgress).unwrap();
    assert!(matches!(
        receive_plan_event(&events),
        PlanWorkerEvent::ProgressEmitted
    ));
    let progress_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = shared_store
            .lock()
            .unwrap()
            .load_download_snapshot("nisar-plan-lifecycle")
            .unwrap();
        if snapshot.progress.len() == 1 {
            assert_eq!(snapshot.progress[0].bytes_completed, 1);
            assert_eq!(snapshot.revision, 1);
            assert_eq!(
                shared_store
                    .lock()
                    .unwrap()
                    .load_scheduler_task("nisar-plan-lifecycle")
                    .unwrap()
                    .unwrap()
                    .runtime
                    .revision,
                4,
            );
            break;
        }
        assert!(
            Instant::now() < progress_deadline,
            "post-freeze progress did not persist with the new revision"
        );
        thread::sleep(Duration::from_millis(5));
    }

    commands.send(PlanWorkerCommand::Finish).unwrap();
    let completion_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let task = shared_store
            .lock()
            .unwrap()
            .load_task_record("nisar-plan-lifecycle")
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Completed {
            break;
        }
        assert!(
            Instant::now() < completion_deadline,
            "post-freeze result did not complete from the new revision"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        shared_store
            .lock()
            .unwrap()
            .load_download_snapshot("nisar-plan-lifecycle")
            .unwrap()
            .results
            .len(),
        1,
    );
    scheduler.shutdown();
}

#[test]
fn scheduler_plan_ready_after_pause_fails_closed_without_freezing_or_progress() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(&database.0).unwrap();
    let (_, plan) = seed_nisar_paused_task(&mut store, &database.0, "nisar-plan-pause");
    resume_nisar_task_for_scheduler(&mut store, "nisar-plan-pause");
    let shared_store = Arc::new(Mutex::new(store));
    let (worker, commands, events) = plan_fixture_worker(plan);
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&shared_store),
        worker,
        Duration::from_millis(5),
    );
    scheduler.wake();
    assert!(matches!(
        receive_plan_event(&events),
        PlanWorkerEvent::Started
    ));

    shared_store
        .lock()
        .unwrap()
        .update_task(&StoreUpdateTaskRequest {
            task_id: "nisar-plan-pause".to_owned(),
            expected_revision: 3,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::PauseTask,
                item_ids: vec![],
            },
            event_id: "nisar-plan-pause:pause-before-freeze".to_owned(),
            occurred_at: "2026-08-30T00:00:03Z".to_owned(),
        })
        .unwrap();
    commands.send(PlanWorkerCommand::Propose).unwrap();
    match receive_plan_event(&events) {
        PlanWorkerEvent::Rejected(code) => assert_eq!(code, "NISAR_PLAN_FREEZE_EXECUTION_STOPPED"),
        event => panic!("pause race unexpectedly froze a plan: {event:?}"),
    }
    let task = shared_store
        .lock()
        .unwrap()
        .load_task_record("nisar-plan-pause")
        .unwrap()
        .unwrap();
    assert!(
        parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
            .unwrap()
            .is_none()
    );
    scheduler.shutdown();
}

#[test]
fn scheduler_plan_ready_after_cancel_fails_closed_without_freezing_or_progress() {
    let database = TempDatabase::new();
    let mut store = TaskStore::open(&database.0).unwrap();
    let (_, plan) = seed_nisar_paused_task(&mut store, &database.0, "nisar-plan-cancel");
    resume_nisar_task_for_scheduler(&mut store, "nisar-plan-cancel");
    let shared_store = Arc::new(Mutex::new(store));
    let (worker, commands, events) = plan_fixture_worker(plan);
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&shared_store),
        worker,
        Duration::from_millis(5),
    );
    scheduler.wake();
    assert!(matches!(
        receive_plan_event(&events),
        PlanWorkerEvent::Started
    ));

    shared_store
        .lock()
        .unwrap()
        .update_task(&StoreUpdateTaskRequest {
            task_id: "nisar-plan-cancel".to_owned(),
            expected_revision: 3,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::CancelTask,
                item_ids: vec![],
            },
            event_id: "nisar-plan-cancel:cancel-before-freeze".to_owned(),
            occurred_at: "2026-08-30T00:00:03Z".to_owned(),
        })
        .unwrap();
    commands.send(PlanWorkerCommand::Propose).unwrap();
    match receive_plan_event(&events) {
        PlanWorkerEvent::Rejected(code) => assert!(matches!(
            code.as_str(),
            "V3_STORE_NOT_FOUND" | "NISAR_PLAN_FREEZE_EXECUTION_STOPPED"
        )),
        event => panic!("cancel race unexpectedly froze a plan: {event:?}"),
    }
    let task = shared_store
        .lock()
        .unwrap()
        .load_task_record("nisar-plan-cancel")
        .unwrap()
        .unwrap();
    assert!(
        parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
            .unwrap()
            .is_none()
    );
    scheduler.shutdown();
}

struct MultiPlanFreezeWorker {
    plans: HashMap<String, NisarSubsetPlanV2>,
    events: mpsc::Sender<(String, bool)>,
}

impl DownloadWorker for MultiPlanFreezeWorker {
    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        plan_freeze: &NisarExecutionPlanFreezeAuthority,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let plan = self.plans.get(&work.item.item_id).unwrap().clone();
        let frozen = plan_freeze.freeze(
            NisarExecutionPlanReadyV2 {
                task_id: work.task.task_id.clone(),
                item_id: work.item.item_id.clone(),
                prepared_intent_sha256: plan.prepared_intent_sha256.clone(),
                proposed_plan: plan,
            },
            &control,
        );
        self.events
            .send((work.item.item_id.clone(), frozen.is_ok()))
            .unwrap();
        DownloadItemResult {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            outcome: if frozen.is_ok() {
                DownloadOutcome::Completed
            } else {
                DownloadOutcome::Failed
            },
            destination: work
                .task
                .output_root
                .join(format!("{}.h5", work.item.item_id.replace(':', "_"))),
            bytes_written: u64::from(frozen.is_ok()),
            attempt_count: 1,
            failure: frozen.err().map(|error| DownloadFailure {
                code: match error {
                    NisarExecutionPlanFreezeError::Store(error) => error.code().to_owned(),
                    NisarExecutionPlanFreezeError::CoordinatorUnavailable => {
                        "NISAR_PLAN_FREEZE_COORDINATOR_UNAVAILABLE".to_owned()
                    }
                    NisarExecutionPlanFreezeError::ExecutionStopped => {
                        "NISAR_PLAN_FREEZE_EXECUTION_STOPPED".to_owned()
                    }
                },
                reason_zh: "NISAR execution plan freeze rejected".to_owned(),
                retryable: false,
            }),
            finished_at: "2026-09-04T00:00:05Z".to_owned(),
        }
    }

    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Failed,
            destination: work.task.output_root.join("unexpected-direct.h5"),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-09-04T00:00:05Z".to_owned(),
        }
    }
}

#[test]
fn scheduler_freezes_each_active_nisar_item_plan_independently() {
    let database = TempDatabase::new();
    let task_id = "nisar-plan-multi-item";
    let intent_one = nisar_prepared_intent();
    let intent_two =
        nisar_prepared_intent_for(NISAR_V2_ITEM_ID_2, "G9900000002-ASF", "NISAR_SOURCE_TWO.h5");
    let plan_one = nisar_plan(intent_one.clone());
    let plan_two = nisar_plan(intent_two.clone());
    let source_one = nisar_source_item();
    let source_two = nisar_source_item_for(NISAR_V2_ITEM_ID_2);
    let mut task_item_one = source_one.clone();
    task_item_one
        .metadata
        .insert("source_item_id".to_owned(), json!(NISAR_V2_ITEM_ID));
    embed_nisar_subset_prepared_intent_v2_envelope(&mut task_item_one.metadata, &intent_one)
        .unwrap();
    let mut task_item_two = source_two.clone();
    task_item_two
        .metadata
        .insert("source_item_id".to_owned(), json!(NISAR_V2_ITEM_ID_2));
    embed_nisar_subset_prepared_intent_v2_envelope(&mut task_item_two.metadata, &intent_two)
        .unwrap();

    let mut store = TaskStore::open(&database.0).unwrap();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "nisar-plan-freeze-batch".to_owned(),
                provider_id: "asf.nasa.earthdata".to_owned(),
                created_at: "2026-09-04T00:00:00Z".to_owned(),
                aoi_name: "NISAR fixture AOI".to_owned(),
                aoi_geojson: Some(nisar_rectangle(1.0, 1.0, 2.0, 2.0)),
                query: Map::new(),
                search_stats: None,
                candidates: vec![source_one, source_two],
            },
            workbench: SearchWorkbenchState {
                batch_id: "nisar-plan-freeze-batch".to_owned(),
                checked_item_ids: vec![NISAR_V2_ITEM_ID.to_owned()],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    store
        .create_paused_task_with_explicit_items(
            &StoreCreateTaskRequest {
                task_id: task_id.to_owned(),
                batch_id: "nisar-plan-freeze-batch".to_owned(),
                selected_item_ids: vec![NISAR_V2_ITEM_ID.to_owned(), NISAR_V2_ITEM_ID_2.to_owned()],
                checked_item_ids: vec![NISAR_V2_ITEM_ID.to_owned(), NISAR_V2_ITEM_ID_2.to_owned()],
                page_index: 0,
                page_size: 24,
                output_root: database.0.with_extension("multi-output"),
                layout: DownloadLayout::Direct,
                concurrency: 2,
                created_at: "2026-09-04T00:00:01Z".to_owned(),
            },
            0,
            &[NISAR_V2_ITEM_ID.to_owned(), NISAR_V2_ITEM_ID_2.to_owned()],
            &[task_item_one, task_item_two],
        )
        .unwrap();
    for (expected_revision, kind, item_ids, event) in [
        (0, TaskStateCommandKind::ResumeTask, vec![], "resume-task"),
        (
            1,
            TaskStateCommandKind::ResumeItems,
            vec![NISAR_V2_ITEM_ID.to_owned(), NISAR_V2_ITEM_ID_2.to_owned()],
            "resume-items",
        ),
    ] {
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision,
                command: StoreTaskCommand { kind, item_ids },
                event_id: format!("{task_id}:{event}"),
                occurred_at: "2026-09-04T00:00:02Z".to_owned(),
            })
            .unwrap();
    }

    let shared_store = Arc::new(Mutex::new(store));
    let (event_sender, event_receiver) = mpsc::channel();
    let worker = Arc::new(MultiPlanFreezeWorker {
        plans: HashMap::from([
            (NISAR_V2_ITEM_ID.to_owned(), plan_one.clone()),
            (NISAR_V2_ITEM_ID_2.to_owned(), plan_two.clone()),
        ]),
        events: event_sender,
    });
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&shared_store),
        worker,
        Duration::from_millis(5),
    );
    scheduler.wake();

    let mut frozen_items = HashMap::new();
    for _ in 0..2 {
        let (item_id, frozen) = event_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("both NISAR items must reach plan freeze");
        frozen_items.insert(item_id, frozen);
    }
    assert_eq!(frozen_items.get(NISAR_V2_ITEM_ID), Some(&true));
    assert_eq!(frozen_items.get(NISAR_V2_ITEM_ID_2), Some(&true));

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let task = shared_store
            .lock()
            .unwrap()
            .load_task_record(task_id)
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Completed {
            assert_eq!(task.runtime.completed_item_ids.len(), 2);
            assert_eq!(
                parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[0].metadata)
                    .unwrap()
                    .unwrap()
                    .plan()
                    .unwrap(),
                plan_one
            );
            assert_eq!(
                parse_nisar_subset_plan_v2_envelope(&task.snapshot.candidates[1].metadata)
                    .unwrap()
                    .unwrap()
                    .plan()
                    .unwrap(),
                plan_two
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "multi-item task did not complete"
        );
        thread::sleep(Duration::from_millis(5));
    }
    scheduler.shutdown();
}

#[test]
fn smart_concurrency_with_live_sample_completes_multi_wave_and_pause_resume_under_concurrency_two()
{
    let database = TempDatabase::new();
    let mut store = TaskStore::open(&database.0).unwrap();
    let candidates = (1..=6)
        .map(|index| SnapshotItem {
            item_id: format!("L9-{index}"),
            provider_id: "microsoft.planetary-computer".to_string(),
            display_name: format!("L9-{index}.tif"),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        })
        .collect();
    store
        .save_search_batch(&StoreSearchBatchRequest {
            activate_workbench: true,
            batch: FrozenSearchBatch {
                schema_version: 1,
                batch_id: "batch-landsat-wave".to_string(),
                provider_id: "microsoft.planetary-computer".to_string(),
                created_at: "2026-11-17T00:00:00Z".to_string(),
                aoi_name: "合肥市".to_string(),
                aoi_geojson: Some(
                    json!({"type":"Polygon","coordinates":[[[117,31],[118,31],[118,32],[117,32],[117,31]]]})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates,
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-landsat-wave".to_string(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    let task_id = "task-landsat-wave";
    let item_ids: Vec<String> = (1..=6).map(|index| format!("L9-{index}")).collect();
    store
        .create_task(&StoreCreateTaskRequest {
            task_id: task_id.to_string(),
            batch_id: "batch-landsat-wave".to_string(),
            selected_item_ids: item_ids,
            checked_item_ids: vec![],
            page_index: 0,
            page_size: 24,
            output_root: database.0.parent().unwrap().join(task_id),
            layout: DownloadLayout::Direct,
            concurrency: 2,
            created_at: "2026-11-17T00:01:00Z".to_string(),
        })
        .unwrap();
    let shared_store = Arc::new(Mutex::new(store));
    let worker = Arc::new(FakeWorker::default());
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&shared_store),
        worker.clone(),
        Duration::from_millis(5),
    );
    scheduler.wake();

    // Wait until wave 1 (2 items) has started, which activates FakeWorker::task_transfer_sample.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if worker.0.lock().unwrap().starts.len() >= 2 {
            break;
        }
        assert!(Instant::now() < deadline, "wave 1 items did not start");
        thread::sleep(Duration::from_millis(5));
    }

    // Pause mid-task and verify resumed waves stay capped at runtime.concurrency (2) and finish all 6 items.
    {
        let mut store = shared_store.lock().unwrap();
        apply_task_command_with_retry(
            &mut store,
            task_id,
            TaskStateCommandKind::PauseTask,
            "pause-wave",
        );
        let paused = store.load_task(task_id).unwrap().unwrap();
        assert_eq!(paused.runtime.status, TaskLifecycleStatus::Paused);
        apply_task_command_with_retry(
            &mut store,
            task_id,
            TaskStateCommandKind::ResumeTask,
            "resume-wave",
        );
    }
    scheduler.wake();

    let finish_deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let task = shared_store
            .lock()
            .unwrap()
            .load_task_record(task_id)
            .unwrap()
            .unwrap();
        if task.runtime.status == TaskLifecycleStatus::Completed {
            assert_eq!(task.runtime.completed_item_ids.len(), 6);
            break;
        }
        assert!(
            Instant::now() < finish_deadline,
            "multi-wave task stalled after resume: status={:?}, active={:?}, queued={:?}, completed={:?}",
            task.runtime.status,
            task.runtime.active_item_ids,
            task.runtime.queued_item_ids,
            task.runtime.completed_item_ids
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        worker
            .0
            .lock()
            .unwrap()
            .max_by_task
            .get(task_id)
            .copied()
            .unwrap_or(0)
            <= 2
    );
    scheduler.shutdown();
}

#[test]
fn smart_concurrency_honors_eight_when_healthy_and_throttles_on_recent_stalls() {
    let _lock = SCHEDULER_PREFS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    set_active_transfer_engine_prefs(TransferEnginePrefs {
        max_concurrent_tasks: 8,
        smart_concurrency: true,
        ..TransferEnginePrefs::default()
    });
    let _prefs_guard = ActiveSchedulerPrefsGuard;

    struct StallAwareWorker {
        stats: Mutex<WorkerStats>,
        stalls_by_task: HashMap<String, u32>,
    }

    impl DownloadWorker for StallAwareWorker {
        fn task_transfer_sample(&self, task_id: &str) -> Option<TaskTransferSample> {
            self.stats
                .lock()
                .unwrap()
                .starts
                .iter()
                .any(|(started_task_id, _)| started_task_id == task_id)
                .then_some(TaskTransferSample {
                    network_bytes_received: 16 * 1024 * 1024,
                    active_millis: 1_000,
                    active_count: 1,
                    bytes_per_second: 16_777_216.0,
                })
        }

        fn task_recent_stall_count(&self, task_id: &str) -> u32 {
            self.stalls_by_task.get(task_id).copied().unwrap_or(0)
        }

        fn execute(
            &self,
            work: DownloadWork,
            _control: WorkerControl,
            _progress: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            {
                let mut stats = self.stats.lock().unwrap();
                stats.global_current += 1;
                let current = {
                    let entry = stats
                        .current_by_task
                        .entry(work.task.task_id.clone())
                        .or_default();
                    *entry += 1;
                    *entry
                };
                stats
                    .max_by_task
                    .entry(work.task.task_id.clone())
                    .and_modify(|v| *v = (*v).max(current))
                    .or_insert(current);
                stats
                    .starts
                    .push((work.task.task_id.clone(), work.item.item_id.clone()));
            }
            thread::sleep(Duration::from_millis(60));
            {
                let mut stats = self.stats.lock().unwrap();
                stats.global_current = stats.global_current.saturating_sub(1);
                if let Some(cur) = stats.current_by_task.get_mut(&work.task.task_id) {
                    *cur = cur.saturating_sub(1);
                }
            }
            DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id.clone(),
                outcome: DownloadOutcome::Completed,
                destination: work
                    .task
                    .output_root
                    .join(format!("{}.zip", work.item.item_id)),
                bytes_written: 1,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-11-18T00:01:01Z".to_string(),
            }
        }
    }

    let database = TempDatabase::new();
    let mut store = TaskStore::open(&database.0).unwrap();
    let item_ids: Vec<String> = (1..=8).map(|idx| format!("S1-STALL-{idx}")).collect();
    for (batch_id, task_id) in [
        ("batch-healthy", "task-healthy-8"),
        ("batch-stalled", "task-stalled-8"),
    ] {
        let candidates = item_ids
            .iter()
            .map(|id| SnapshotItem {
                item_id: format!("{batch_id}-{id}"),
                provider_id: "asf.sentinel1".to_string(),
                display_name: id.clone(),
                acquisition_datetime: Some("2026-11-18T00:00:00Z".to_string()),
                footprint_geojson: None,
                metadata: Map::new(),
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            })
            .collect::<Vec<_>>();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: false,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: batch_id.to_string(),
                    provider_id: "asf.sentinel1".to_string(),
                    created_at: "2026-11-18T00:00:00Z".to_string(),
                    aoi_name: "Test AOI".to_string(),
                    aoi_geojson: Some(
                        json!({"type":"Polygon","coordinates":[[[117,31],[118,31],[118,32],[117,32],[117,31]]]})
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                    query: Map::new(),
                    search_stats: None,
                    candidates,
                },
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.to_string(),
                    checked_item_ids: vec![],
                    page_index: 0,
                    page_size: 24,
                    revision: 0,
                },
            })
            .unwrap();
        let selected: Vec<String> = item_ids
            .iter()
            .map(|id| format!("{batch_id}-{id}"))
            .collect();
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: task_id.to_string(),
                batch_id: batch_id.to_string(),
                selected_item_ids: selected,
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: database.0.parent().unwrap().join(task_id),
                layout: DownloadLayout::Direct,
                concurrency: 8,
                created_at: "2026-11-18T00:01:00Z".to_string(),
            })
            .unwrap();
    }

    let mut stalls = HashMap::new();
    stalls.insert("task-healthy-8".to_string(), 0);
    stalls.insert("task-stalled-8".to_string(), 2);
    let worker = Arc::new(StallAwareWorker {
        stats: Mutex::new(WorkerStats::default()),
        stalls_by_task: stalls,
    });
    let shared_store = Arc::new(Mutex::new(store));
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&shared_store),
        worker.clone(),
        Duration::from_millis(5),
    );
    scheduler.wake();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let store = shared_store.lock().unwrap();
        let h_done = store
            .load_task_record("task-healthy-8")
            .unwrap()
            .unwrap()
            .runtime
            .status
            == TaskLifecycleStatus::Completed;
        let s_done = store
            .load_task_record("task-stalled-8")
            .unwrap()
            .unwrap()
            .runtime
            .status
            == TaskLifecycleStatus::Completed;
        if h_done && s_done {
            break;
        }
        assert!(Instant::now() < deadline, "tasks did not complete in time");
        drop(store);
        thread::sleep(Duration::from_millis(10));
    }

    let max_by_task = worker.stats.lock().unwrap().max_by_task.clone();
    assert_eq!(
        max_by_task.get("task-healthy-8").copied().unwrap_or(0),
        8,
        "healthy task with 0 stalls must use full configured concurrency=8"
    );
    assert!(
        max_by_task.get("task-stalled-8").copied().unwrap_or(0) <= 4,
        "stalled task with recent_stall_count=2 must throttle to <= 4, got {:?}",
        max_by_task.get("task-stalled-8")
    );
    scheduler.shutdown();
}

#[test]
fn tier1_runtime_cap_throttles_existing_task_without_breaking_task_reg_003() {
    let _lock = SCHEDULER_PREFS_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    set_active_transfer_engine_prefs(TransferEnginePrefs {
        max_concurrent_tasks: 2,
        smart_concurrency: false,
        ..TransferEnginePrefs::default()
    });
    let _prefs_guard = ActiveSchedulerPrefsGuard;

    #[derive(Default)]
    struct TrackingWorker(Mutex<WorkerStats>);

    impl DownloadWorker for TrackingWorker {
        fn execute(
            &self,
            work: DownloadWork,
            _control: WorkerControl,
            _progress: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            {
                let mut stats = self.0.lock().unwrap();
                stats.global_current += 1;
                stats.global_max = stats.global_max.max(stats.global_current);
                let current = {
                    let entry = stats
                        .current_by_task
                        .entry(work.task.task_id.clone())
                        .or_default();
                    *entry += 1;
                    *entry
                };
                stats
                    .max_by_task
                    .entry(work.task.task_id.clone())
                    .and_modify(|v| *v = (*v).max(current))
                    .or_insert(current);
                stats
                    .starts
                    .push((work.task.task_id.clone(), work.item.item_id.clone()));
            }
            thread::sleep(Duration::from_millis(80));
            {
                let mut stats = self.0.lock().unwrap();
                stats.global_current = stats.global_current.saturating_sub(1);
                if let Some(cur) = stats.current_by_task.get_mut(&work.task.task_id) {
                    *cur = cur.saturating_sub(1);
                }
            }
            DownloadItemResult {
                task_id: work.task.task_id,
                item_id: work.item.item_id.clone(),
                outcome: DownloadOutcome::Completed,
                destination: work
                    .task
                    .output_root
                    .join(format!("{}.zip", work.item.item_id)),
                bytes_written: 1,
                attempt_count: 1,
                failure: None,
                finished_at: "2026-11-20T00:01:01Z".to_string(),
            }
        }
    }

    let database = TempDatabase::new();
    let mut store = TaskStore::open(&database.0).unwrap();
    for (batch_id, task_id, count, concurrency) in [
        ("batch-cap-1", "task-cap-one", 6usize, 6u64),
        ("batch-cap-2", "task-cap-two", 4usize, 4u64),
    ] {
        let item_ids: Vec<String> = (1..=count).map(|i| format!("{batch_id}-S{i}")).collect();
        let candidates = item_ids
            .iter()
            .map(|id| SnapshotItem {
                item_id: id.clone(),
                provider_id: "asf.sentinel1".to_string(),
                display_name: id.clone(),
                acquisition_datetime: Some("2026-11-20T00:00:00Z".to_string()),
                footprint_geojson: None,
                metadata: Map::new(),
                payload: Map::new(),
                asset_access: None,
                execution_policy: None,
            })
            .collect::<Vec<_>>();
        store
            .save_search_batch(&StoreSearchBatchRequest {
                activate_workbench: false,
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: batch_id.to_string(),
                    provider_id: "asf.sentinel1".to_string(),
                    created_at: "2026-11-20T00:00:00Z".to_string(),
                    aoi_name: "Test AOI".to_string(),
                    aoi_geojson: Some(
                        json!({"type":"Polygon","coordinates":[[[117,31],[118,31],[118,32],[117,32],[117,31]]]})
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                    query: Map::new(),
                    search_stats: None,
                    candidates,
                },
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.to_string(),
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
                batch_id: batch_id.to_string(),
                selected_item_ids: item_ids.clone(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: database.0.parent().unwrap().join(task_id),
                layout: DownloadLayout::Direct,
                concurrency,
                created_at: "2026-11-20T00:01:00Z".to_string(),
            })
            .unwrap();
    }

    let worker = Arc::new(TrackingWorker::default());
    let shared_store = Arc::new(Mutex::new(store));
    let scheduler = DownloadScheduler::start_with_store(
        Arc::clone(&shared_store),
        worker.clone(),
        Duration::from_millis(5),
    );
    scheduler.wake();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let store = shared_store.lock().unwrap();
        let one_done = store
            .load_task_record("task-cap-one")
            .unwrap()
            .unwrap()
            .runtime
            .status
            == TaskLifecycleStatus::Completed;
        let two_done = store
            .load_task_record("task-cap-two")
            .unwrap()
            .unwrap()
            .runtime
            .status
            == TaskLifecycleStatus::Completed;
        if one_done && two_done {
            break;
        }
        assert!(Instant::now() < deadline, "tasks did not complete in time");
        drop(store);
        thread::sleep(Duration::from_millis(10));
    }

    let stats = worker.0.lock().unwrap();
    assert!(
        stats.max_by_task.get("task-cap-one").copied().unwrap_or(0) <= 2,
        "runtime Tier-1 override=2 must cap persisted concurrency=6 task to <= 2, got {:?}",
        stats.max_by_task
    );
    assert!(
        stats.max_by_task.get("task-cap-two").copied().unwrap_or(0) <= 2,
        "runtime Tier-1 override=2 must cap second task to <= 2, got {:?}",
        stats.max_by_task
    );
    assert!(
        stats.global_max >= 3,
        "TASK-REG-003: independent tasks targeting different output directories must execute concurrently (global_max={})",
        stats.global_max
    );
    drop(stats);
    scheduler.shutdown();
}
