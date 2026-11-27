use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadItemResult, DownloadLayout, DownloadOutcome, FrozenSearchBatch, SearchWorkbenchState,
    SnapshotItem, StoreArchiveTaskRequest, StoreCreateTaskRequest, StoreSearchBatchRequest,
    StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus, TaskRuntimeSnapshot,
    TaskStateCommandKind,
};
use insar_download::{DownloadScheduler, DownloadWork, DownloadWorker, WorkerControl};
use insar_store::TaskStore;
use rusqlite::Connection;
use serde_json::{json, Map};

const POLL: Duration = Duration::from_millis(5);
const WAIT: Duration = Duration::from_secs(2);

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new() -> Self {
        // A per-process sequence keeps names unique when parallel tests read
        // the same clock tick.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-terminal-cancellation-{}-{nonce}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )))
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerEvent {
    task_id: String,
    item_id: String,
}

struct PreBackendWorker {
    waiting: Sender<WorkerEvent>,
    cancelled: Sender<WorkerEvent>,
    allow_backend: AtomicBool,
    backend_calls: AtomicUsize,
    starts: Mutex<Vec<WorkerEvent>>,
}

struct ShutdownGateWorker {
    started: Sender<()>,
    cancelled: Sender<()>,
    release: Mutex<Receiver<()>>,
    exited: Sender<()>,
}

struct RetryHandoffWorker {
    started: Sender<usize>,
    release_first: Mutex<Receiver<()>>,
    starts: AtomicUsize,
}

impl DownloadWorker for RetryHandoffWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> DownloadItemResult {
        let attempt = self.starts.fetch_add(1, Ordering::SeqCst);
        self.started.send(attempt).unwrap();
        if attempt == 0 {
            self.release_first
                .lock()
                .unwrap()
                .recv_timeout(WAIT)
                .unwrap();
        } else {
            while !control.is_cancelled() {
                thread::sleep(Duration::from_millis(2));
            }
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 0,
            failure: None,
            finished_at: "2026-07-24T00:00:03Z".to_string(),
        }
    }
}

struct ResultHandoffWorker {
    started: Sender<usize>,
    cancelled: Sender<()>,
    release_first: Mutex<Receiver<()>>,
    starts: AtomicUsize,
}

impl DownloadWorker for ResultHandoffWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> DownloadItemResult {
        let attempt = self.starts.fetch_add(1, Ordering::SeqCst);
        self.started.send(attempt).unwrap();
        while !control.is_cancelled() {
            thread::sleep(Duration::from_millis(2));
        }
        if attempt == 0 {
            self.cancelled.send(()).unwrap();
            self.release_first
                .lock()
                .unwrap()
                .recv_timeout(WAIT)
                .unwrap();
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 0,
            failure: None,
            finished_at: "2026-07-24T00:00:03Z".to_string(),
        }
    }
}

struct RetryAndOtherWorker {
    started: Sender<usize>,
    cancelled: Sender<()>,
    release_first: Mutex<Receiver<()>>,
    starts: AtomicUsize,
}

impl DownloadWorker for RetryAndOtherWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> DownloadItemResult {
        if work.task.task_id == "task-unrelated-repair" {
            self.started.send(100).unwrap();
            while !control.is_cancelled() {
                thread::sleep(Duration::from_millis(2));
            }
        } else {
            let attempt = self.starts.fetch_add(1, Ordering::SeqCst);
            self.started.send(attempt).unwrap();
            while !control.is_cancelled() {
                thread::sleep(Duration::from_millis(2));
            }
            if attempt == 0 {
                self.cancelled.send(()).unwrap();
                self.release_first
                    .lock()
                    .unwrap()
                    .recv_timeout(WAIT)
                    .unwrap();
            }
        }
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 0,
            failure: None,
            finished_at: "2026-07-24T00:00:03Z".to_string(),
        }
    }
}

struct CancelledResultWorker {
    started: Sender<()>,
    release: Mutex<Receiver<()>>,
}

impl DownloadWorker for CancelledResultWorker {
    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> DownloadItemResult {
        self.started.send(()).unwrap();
        self.release.lock().unwrap().recv_timeout(WAIT).unwrap();
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 0,
            failure: None,
            finished_at: "2026-07-24T00:00:03Z".to_string(),
        }
    }
}

impl DownloadWorker for ShutdownGateWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> DownloadItemResult {
        self.started.send(()).unwrap();
        while !control.is_cancelled() {
            thread::sleep(Duration::from_millis(2));
        }
        self.cancelled.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        self.exited.send(()).unwrap();
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome: DownloadOutcome::Cancelled,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 0,
            failure: None,
            finished_at: "2026-07-24T00:00:00Z".to_string(),
        }
    }
}

impl PreBackendWorker {
    fn new() -> (Arc<Self>, Receiver<WorkerEvent>, Receiver<WorkerEvent>) {
        let (waiting_sender, waiting_receiver) = mpsc::channel();
        let (cancelled_sender, cancelled_receiver) = mpsc::channel();
        (
            Arc::new(Self {
                waiting: waiting_sender,
                cancelled: cancelled_sender,
                allow_backend: AtomicBool::new(false),
                backend_calls: AtomicUsize::new(0),
                starts: Mutex::new(Vec::new()),
            }),
            waiting_receiver,
            cancelled_receiver,
        )
    }

    fn starts(&self) -> Vec<WorkerEvent> {
        self.starts.lock().unwrap().clone()
    }
}

impl DownloadWorker for PreBackendWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        _progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> DownloadItemResult {
        let event = WorkerEvent {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
        };
        self.starts.lock().unwrap().push(event.clone());
        let _ = self.waiting.send(event.clone());
        while !control.is_cancelled() && !self.allow_backend.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(2));
        }
        let outcome = if control.is_cancelled() {
            let _ = self.cancelled.send(event);
            DownloadOutcome::Cancelled
        } else {
            self.backend_calls.fetch_add(1, Ordering::SeqCst);
            DownloadOutcome::Completed
        };
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id,
            outcome,
            destination: work.task.output_root,
            bytes_written: 0,
            attempt_count: 0,
            failure: None,
            finished_at: "2026-07-24T00:00:00Z".to_string(),
        }
    }
}

fn seed_tasks(path: &Path, tasks: &[(&str, &[&str], u64)]) {
    let mut store = TaskStore::open(path).unwrap();
    let candidate_ids: Vec<&str> = tasks
        .iter()
        .flat_map(|(_, item_ids, _)| item_ids.iter().copied())
        .collect();
    let candidates = candidate_ids
        .iter()
        .map(|item_id| SnapshotItem {
            item_id: (*item_id).to_string(),
            provider_id: "asf.sentinel1".to_string(),
            display_name: format!("{item_id}.zip"),
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
                batch_id: "batch-terminal-cancel".to_string(),
                provider_id: "asf.sentinel1".to_string(),
                created_at: "2026-07-24T00:00:00Z".to_string(),
                aoi_name: "测试区".to_string(),
                aoi_geojson: Some(
                    json!({
                        "type": "Polygon",
                        "coordinates": [[[100, 30], [101, 30], [101, 31], [100, 31], [100, 30]]]
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
                query: Map::new(),
                search_stats: None,
                candidates,
            },
            workbench: SearchWorkbenchState {
                batch_id: "batch-terminal-cancel".to_string(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                revision: 0,
            },
        })
        .unwrap();
    for (task_id, item_ids, concurrency) in tasks {
        store
            .create_task(&StoreCreateTaskRequest {
                task_id: (*task_id).to_string(),
                batch_id: "batch-terminal-cancel".to_string(),
                selected_item_ids: item_ids
                    .iter()
                    .map(|item_id| (*item_id).to_string())
                    .collect(),
                checked_item_ids: vec![],
                page_index: 0,
                page_size: 24,
                output_root: path.parent().unwrap().join(task_id),
                layout: DownloadLayout::Sentinel1Slc,
                concurrency: *concurrency,
                created_at: "2026-07-24T00:00:01Z".to_string(),
            })
            .unwrap();
    }
}

fn update_task(
    path: &Path,
    task_id: &str,
    kind: TaskStateCommandKind,
    item_ids: &[&str],
    event_id: &str,
) -> TaskRuntimeSnapshot {
    let mut store = TaskStore::open(path).unwrap();
    let current = store.load_task(task_id).unwrap().unwrap();
    store
        .update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_string(),
            expected_revision: current.runtime.revision,
            command: StoreTaskCommand {
                kind,
                item_ids: item_ids
                    .iter()
                    .map(|item_id| (*item_id).to_string())
                    .collect(),
            },
            event_id: event_id.to_string(),
            occurred_at: "2026-07-24T00:00:02Z".to_string(),
        })
        .unwrap()
        .runtime
}

fn erase_stamped_item_membership(path: &Path, event_ids: &[&str]) {
    let connection = Connection::open(path).unwrap();
    for event_id in event_ids {
        connection
            .execute(
                "UPDATE task_events SET payload_json = json_set(payload_json, '$.command.item_ids', json('[]')) WHERE event_id = ?1",
                [event_id],
            )
            .unwrap();
    }
}

fn expect_waiting(receiver: &Receiver<WorkerEvent>) -> WorkerEvent {
    receiver
        .recv_timeout(WAIT)
        .expect("worker must become live before its modeled backend entry")
}

fn wait_for_backend_calls(worker: &PreBackendWorker, expected: usize) {
    let deadline = Instant::now() + WAIT;
    while worker.backend_calls.load(Ordering::SeqCst) != expected {
        assert!(
            Instant::now() < deadline,
            "backend call count did not reach {expected}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn late_cancelled_worker_result_does_not_finish_immediately_retried_task() {
    let database = TempDatabase::new();
    seed_tasks(database.path(), &[("task-retry-handoff", &["S1-RETRY"], 1)]);
    let (started_sender, started) = mpsc::channel();
    let (release_first_sender, release_first) = mpsc::channel();
    let worker = Arc::new(RetryHandoffWorker {
        started: started_sender,
        release_first: Mutex::new(release_first),
        starts: AtomicUsize::new(0),
    });
    let scheduler =
        DownloadScheduler::start(database.path(), worker.clone(), POLL).expect("start scheduler");
    scheduler.wake();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 0);

    let cancelled = update_task(
        database.path(),
        "task-retry-handoff",
        TaskStateCommandKind::CancelTask,
        &[],
        "cancel-before-retry",
    );
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .archive_task(&StoreArchiveTaskRequest {
            task_id: "task-retry-handoff".to_owned(),
            expected_revision: cancelled.revision,
            archived_at: "2026-07-24T00:00:03Z".to_owned(),
        })
        .unwrap();
    let retried = store
        .update_task(&StoreUpdateTaskRequest {
            task_id: "task-retry-handoff".to_owned(),
            expected_revision: cancelled.revision,
            command: StoreTaskCommand {
                kind: TaskStateCommandKind::RetryTask,
                item_ids: vec![],
            },
            event_id: "retry-after-cancel".to_owned(),
            occurred_at: "2026-07-24T00:00:04Z".to_owned(),
        })
        .unwrap()
        .runtime;
    assert_eq!(retried.status, TaskLifecycleStatus::Queued);
    drop(store);

    // The old worker is still blocked. A scheduler pass must not launch the
    // new generation until that worker exits; once it does, its stale terminal
    // result must not overwrite the retried queued state.
    scheduler.wake();
    assert!(started.recv_timeout(Duration::from_millis(100)).is_err());
    release_first_sender.send(()).unwrap();
    let replacement = started.recv_timeout(WAIT);
    if replacement.is_err() {
        let state = TaskStore::open(database.path())
            .unwrap()
            .load_task("task-retry-handoff")
            .unwrap()
            .unwrap();
        scheduler.shutdown();
        panic!(
            "new worker did not start after old exit; runtime={:?}",
            state.runtime
        );
    }
    assert_eq!(replacement.unwrap(), 1);
    let current = TaskStore::open(database.path())
        .unwrap()
        .load_task("task-retry-handoff")
        .unwrap()
        .unwrap();
    assert_eq!(current.runtime.status, TaskLifecycleStatus::Running);
    assert!(current.runtime.cancelled_item_ids.is_empty());
    let result = TaskStore::open(database.path())
        .unwrap()
        .load_download_snapshot("task-retry-handoff")
        .unwrap();
    assert!(
        result.results.is_empty(),
        "the stale cancelled worker must not write a result into the retried generation"
    );
    scheduler.shutdown();
}

#[test]
fn late_cancel_preserves_existing_result_after_retry_is_active() {
    let database = TempDatabase::new();
    seed_tasks(database.path(), &[("task-retry-active", &["S1-ACTIVE"], 1)]);
    let (started_sender, started) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let worker = Arc::new(ResultHandoffWorker {
        started: started_sender,
        cancelled: cancelled_sender,
        release_first: Mutex::new(release),
        starts: AtomicUsize::new(0),
    });
    let scheduler = DownloadScheduler::start(database.path(), worker, POLL).unwrap();
    scheduler.wake();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 0);
    update_task(
        database.path(),
        "task-retry-active",
        TaskStateCommandKind::CancelItems,
        &["S1-ACTIVE"],
        "active-cancel",
    );
    scheduler.wake();
    cancelled.recv_timeout(WAIT).unwrap();
    let mut store = TaskStore::open(database.path()).unwrap();
    let prior = DownloadItemResult {
        task_id: "task-retry-active".into(),
        item_id: "S1-ACTIVE".into(),
        outcome: DownloadOutcome::Cancelled,
        destination: database.path().parent().unwrap().join("prior-result"),
        bytes_written: 37,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-07-24T00:00:01Z".into(),
    };
    store.save_scheduler_download_result(&prior).unwrap();
    drop(store);
    update_task(
        database.path(),
        "task-retry-active",
        TaskStateCommandKind::RetryItems,
        &["S1-ACTIVE"],
        "active-retry",
    );
    scheduler.wake();
    let deadline = Instant::now() + WAIT;
    loop {
        let state = TaskStore::open(database.path())
            .unwrap()
            .load_task("task-retry-active")
            .unwrap()
            .unwrap();
        if state
            .runtime
            .active_item_ids
            .contains(&"S1-ACTIVE".to_string())
        {
            break;
        }
        assert!(Instant::now() < deadline, "retry was not activated");
        thread::sleep(Duration::from_millis(2));
    }
    assert!(started.recv_timeout(Duration::from_millis(50)).is_err());
    release_sender.send(()).unwrap();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 1);
    let store = TaskStore::open(database.path()).unwrap();
    let state = store.load_task("task-retry-active").unwrap().unwrap();
    assert!(state.runtime.cancelled_item_ids.is_empty());
    assert_eq!(state.runtime.status, TaskLifecycleStatus::Running);
    assert_eq!(
        store
            .load_download_snapshot("task-retry-active")
            .unwrap()
            .results,
        vec![prior],
        "the stale result must not replace the existing download result"
    );
    drop(store);
    scheduler.shutdown();
}

#[test]
fn unreadable_retry_evidence_does_not_cancel_or_replace_active_item() {
    let database = TempDatabase::new();
    seed_tasks(database.path(), &[("task-retry-bad-event", &["S1-BAD"], 1)]);
    let (started_sender, started) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let worker = Arc::new(ResultHandoffWorker {
        started: started_sender,
        cancelled: cancelled_sender,
        release_first: Mutex::new(release),
        starts: AtomicUsize::new(0),
    });
    let scheduler = DownloadScheduler::start(database.path(), worker, POLL).unwrap();
    scheduler.wake();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 0);
    update_task(
        database.path(),
        "task-retry-bad-event",
        TaskStateCommandKind::CancelItems,
        &["S1-BAD"],
        "bad-event-cancel",
    );
    scheduler.wake();
    cancelled.recv_timeout(WAIT).unwrap();
    update_task(
        database.path(),
        "task-retry-bad-event",
        TaskStateCommandKind::RetryItems,
        &["S1-BAD"],
        "bad-event-retry",
    );
    scheduler.wake();
    let deadline = Instant::now() + WAIT;
    loop {
        let store = TaskStore::open(database.path()).unwrap();
        let state = store.load_task("task-retry-bad-event").unwrap().unwrap();
        if state
            .runtime
            .active_item_ids
            .contains(&"S1-BAD".to_string())
        {
            break;
        }
        assert!(Instant::now() < deadline, "retry was not activated");
        thread::sleep(Duration::from_millis(2));
    }
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute(
            "UPDATE task_events SET payload_json = '{}' WHERE event_id = 'bad-event-retry'",
            [],
        )
        .unwrap();
    drop(connection);
    release_sender.send(()).unwrap();
    assert!(started.recv_timeout(Duration::from_millis(250)).is_err());
    let store = TaskStore::open(database.path()).unwrap();
    let state = store.load_task("task-retry-bad-event").unwrap().unwrap();
    assert!(state.runtime.cancelled_item_ids.is_empty());
    assert_eq!(state.runtime.status, TaskLifecycleStatus::Running);
    assert!(store
        .load_download_snapshot("task-retry-bad-event")
        .unwrap()
        .results
        .is_empty());
    drop(store);
    scheduler.shutdown();
}

#[test]
fn unreadable_retry_evidence_stays_pending_past_backoff_and_retries_after_repair() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[
            ("task-retry-repair", &["S1-REPAIR"], 1),
            ("task-unrelated-repair", &["S1-OTHER-REPAIR"], 1),
        ],
    );
    update_task(
        database.path(),
        "task-unrelated-repair",
        TaskStateCommandKind::PauseTask,
        &[],
        "other-pause",
    );
    let (started_sender, started) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let worker = Arc::new(RetryAndOtherWorker {
        started: started_sender,
        cancelled: cancelled_sender,
        release_first: Mutex::new(release),
        starts: AtomicUsize::new(0),
    });
    let scheduler = DownloadScheduler::start(database.path(), worker, POLL).unwrap();
    scheduler.wake();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 0);
    update_task(
        database.path(),
        "task-retry-repair",
        TaskStateCommandKind::CancelItems,
        &["S1-REPAIR"],
        "repair-cancel",
    );
    scheduler.wake();
    cancelled.recv_timeout(WAIT).unwrap();
    let retried = update_task(
        database.path(),
        "task-retry-repair",
        TaskStateCommandKind::RetryItems,
        &["S1-REPAIR"],
        "repair-retry",
    );
    assert!(retried.queued_item_ids.contains(&"S1-REPAIR".to_string()));
    scheduler.wake();
    let deadline = Instant::now() + WAIT;
    loop {
        let state = TaskStore::open(database.path())
            .unwrap()
            .load_task("task-retry-repair")
            .unwrap()
            .unwrap();
        if state
            .runtime
            .active_item_ids
            .contains(&"S1-REPAIR".to_string())
        {
            break;
        }
        assert!(Instant::now() < deadline, "retry was not activated");
        thread::sleep(Duration::from_millis(2));
    }
    let request_json = serde_json::to_string(&StoreUpdateTaskRequest {
        task_id: "task-retry-repair".to_owned(),
        expected_revision: retried.revision - 1,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::RetryItems,
            item_ids: vec!["S1-REPAIR".to_owned()],
        },
        event_id: "repair-retry".to_owned(),
        occurred_at: "2026-07-24T00:00:02Z".to_owned(),
    })
    .unwrap();
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute(
            "UPDATE task_events SET payload_json = '{}' WHERE event_id = 'repair-retry'",
            [],
        )
        .unwrap();
    drop(connection);
    release_sender.send(()).unwrap();
    update_task(
        database.path(),
        "task-unrelated-repair",
        TaskStateCommandKind::ResumeTask,
        &[],
        "other-resume",
    );
    scheduler.wake();
    assert_eq!(
        started.recv_timeout(WAIT).unwrap(),
        100,
        "unrelated task did not start while evidence was unavailable"
    );

    // The evidence failure must hold this item past the one-second finish
    // backoff; it must not be relaunched as an active duplicate.
    let restarted = started.recv_timeout(Duration::from_millis(1_250)).ok();

    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute(
            "UPDATE task_events SET payload_json = ?1 WHERE event_id = 'repair-retry'",
            [&request_json],
        )
        .unwrap();
    drop(connection);
    scheduler.wake();
    let after_repair = if restarted.is_none() {
        started.recv_timeout(WAIT).ok()
    } else {
        None
    };
    scheduler.shutdown();
    assert_eq!(restarted, None, "the same item restarted without evidence");
    assert_eq!(after_repair, Some(1), "the pending finish was not retried");
}

#[test]
fn stale_runtime_revision_cannot_persist_or_cancel_after_cross_connection_retry() {
    let database = TempDatabase::new();
    seed_tasks(database.path(), &[("task-cas-race", &["S1-CAS"], 1)]);
    let initial = update_task(
        database.path(),
        "task-cas-race",
        TaskStateCommandKind::ActivateItems,
        &["S1-CAS"],
        "cas-start",
    );
    let started_revision = initial.revision;
    let (read_sender, read_receiver) = mpsc::channel();
    let (resume_sender, resume_receiver) = mpsc::channel();
    let finish_path = database.path().to_path_buf();
    let finish = thread::spawn(move || {
        let mut store = TaskStore::open(finish_path).unwrap();
        let read = store
            .load_scheduler_control_runtime("task-cas-race")
            .unwrap()
            .unwrap();
        let before = store.load_download_snapshot("task-cas-race").unwrap();
        read_sender.send((read.revision, before)).unwrap();
        resume_receiver.recv_timeout(WAIT).unwrap();
        let result = DownloadItemResult {
            task_id: "task-cas-race".into(),
            item_id: "S1-CAS".into(),
            outcome: DownloadOutcome::Cancelled,
            destination: std::env::temp_dir().join("cas-output"),
            bytes_written: 0,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-24T00:00:05Z".into(),
        };
        let error = store
            .finish_scheduler_cancelled_result(
                &result,
                started_revision,
                read.revision,
                "cas-finish",
                false,
            )
            .expect_err("a finish based on the old revision must fail atomically");
        (store, result, error)
    });
    let (read_revision, before) = read_receiver.recv_timeout(WAIT).unwrap();
    assert_eq!(read_revision, initial.revision);
    let cancelled = update_task(
        database.path(),
        "task-cas-race",
        TaskStateCommandKind::CancelItems,
        &["S1-CAS"],
        "cas-cancel",
    );
    let retried = update_task(
        database.path(),
        "task-cas-race",
        TaskStateCommandKind::RetryItems,
        &["S1-CAS"],
        "cas-retry",
    );
    assert!(cancelled.cancelled_item_ids.contains(&"S1-CAS".to_string()));
    assert!(retried.queued_item_ids.contains(&"S1-CAS".to_string()));
    resume_sender.send(()).unwrap();
    let (mut scheduler_store, result, error) = finish.join().unwrap();
    assert!(matches!(error, insar_store::StoreError::RevisionConflict));
    assert_eq!(
        scheduler_store
            .load_download_snapshot("task-cas-race")
            .unwrap(),
        before
    );
    let current = scheduler_store.load_task("task-cas-race").unwrap().unwrap();
    assert!(current
        .runtime
        .queued_item_ids
        .contains(&"S1-CAS".to_string()));
    assert!(current.runtime.cancelled_item_ids.is_empty());
    let settled = scheduler_store
        .finish_scheduler_cancelled_result(
            &result,
            initial.revision,
            current.runtime.revision,
            "cas-retry-finish",
            false,
        )
        .unwrap();
    assert!(matches!(
        settled,
        insar_store::SchedulerCancelledFinish::Retried(_)
    ));
    assert_eq!(
        scheduler_store
            .load_download_snapshot("task-cas-race")
            .unwrap(),
        before
    );
}

#[test]
fn unrelated_malformed_historical_event_does_not_block_cancel_finish() {
    let database = TempDatabase::new();
    seed_tasks(database.path(), &[("task-old-event", &["S1-OLD"], 1)]);
    let initial = update_task(
        database.path(),
        "task-old-event",
        TaskStateCommandKind::ActivateItems,
        &["S1-OLD"],
        "old-start",
    );
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("UPDATE task_events SET payload_json = 'not-json', error_json = 'not-json' WHERE event_id = 'old-start'", []).unwrap();
    drop(connection);
    let result = DownloadItemResult {
        task_id: "task-old-event".into(),
        item_id: "S1-OLD".into(),
        outcome: DownloadOutcome::Cancelled,
        destination: std::env::temp_dir().join("old-event-output"),
        bytes_written: 0,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-07-24T00:00:05Z".into(),
    };
    let mut store = TaskStore::open(database.path()).unwrap();
    let outcome = store
        .finish_scheduler_cancelled_result(
            &result,
            initial.revision,
            initial.revision,
            "old-finish",
            false,
        )
        .unwrap();
    assert!(matches!(
        outcome,
        insar_store::SchedulerCancelledFinish::Settled(Some(_))
    ));
    assert!(store
        .load_task("task-old-event")
        .unwrap()
        .unwrap()
        .runtime
        .cancelled_item_ids
        .contains(&"S1-OLD".to_string()));
}

#[test]
fn legacy_task_level_empty_evidence_stays_blocked() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[("task-legacy-evidence", &["S1-LEGACY"], 1)],
    );
    let initial = update_task(
        database.path(),
        "task-legacy-evidence",
        TaskStateCommandKind::ActivateItems,
        &["S1-LEGACY"],
        "legacy-start",
    );
    let cancelled = update_task(
        database.path(),
        "task-legacy-evidence",
        TaskStateCommandKind::CancelTask,
        &[],
        "legacy-cancel",
    );
    let retried = update_task(
        database.path(),
        "task-legacy-evidence",
        TaskStateCommandKind::RetryTask,
        &[],
        "legacy-retry",
    );
    assert_eq!(retried.revision, cancelled.revision + 1);
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("UPDATE task_events SET payload_json = json_set(payload_json, '$.command.item_ids', json('[]')) WHERE event_id IN ('legacy-cancel','legacy-retry')", []).unwrap();
    drop(connection);
    let result = DownloadItemResult {
        task_id: "task-legacy-evidence".into(),
        item_id: "S1-LEGACY".into(),
        outcome: DownloadOutcome::Cancelled,
        destination: std::env::temp_dir().join("legacy-output"),
        bytes_written: 0,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-07-24T00:00:05Z".into(),
    };
    let mut store = TaskStore::open(database.path()).unwrap();
    assert!(
        store
            .finish_scheduler_cancelled_result(
                &result,
                initial.revision,
                retried.revision,
                "legacy-finish",
                false
            )
            .is_err(),
        "empty legacy task-level payload is insufficient evidence"
    );
    let current = store.load_task("task-legacy-evidence").unwrap().unwrap();
    assert!(current
        .runtime
        .queued_item_ids
        .contains(&"S1-LEGACY".to_string()));
    assert!(store
        .load_download_snapshot("task-legacy-evidence")
        .unwrap()
        .results
        .is_empty());
}

#[test]
fn legacy_empty_evidence_becomes_visible_failure_without_losing_progress() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[
            ("task-legacy-exit", &["S1-LEGACY-EXIT"], 1),
            ("task-unrelated-repair", &["S1-LEGACY-OTHER"], 1),
        ],
    );
    update_task(
        database.path(),
        "task-unrelated-repair",
        TaskStateCommandKind::PauseTask,
        &[],
        "legacy-other-pause",
    );
    let (started_sender, started) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let worker = Arc::new(RetryAndOtherWorker {
        started: started_sender,
        cancelled: cancelled_sender,
        release_first: Mutex::new(release),
        starts: AtomicUsize::new(0),
    });
    let scheduler = DownloadScheduler::start(database.path(), worker, POLL).unwrap();
    scheduler.wake();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 0);
    update_task(
        database.path(),
        "task-legacy-exit",
        TaskStateCommandKind::CancelTask,
        &[],
        "legacy-exit-cancel",
    );
    scheduler.wake();
    cancelled.recv_timeout(WAIT).unwrap();
    update_task(
        database.path(),
        "task-legacy-exit",
        TaskStateCommandKind::RetryTask,
        &[],
        "legacy-exit-retry",
    );
    scheduler.wake();
    let deadline = Instant::now() + WAIT;
    loop {
        let store = TaskStore::open(database.path()).unwrap();
        if store
            .load_task("task-legacy-exit")
            .unwrap()
            .unwrap()
            .runtime
            .active_item_ids
            .contains(&"S1-LEGACY-EXIT".to_owned())
        {
            break;
        }
        assert!(Instant::now() < deadline, "retry was not activated");
        thread::sleep(Duration::from_millis(2));
    }
    let progress = insar_core::DownloadItemProgress {
        task_id: "task-legacy-exit".into(),
        item_id: "S1-LEGACY-EXIT".into(),
        status: insar_core::DownloadItemStatus::Active,
        bytes_completed: 137,
        total_bytes: Some(500),
        bytes_per_second: 9.0,
        attempt: 2,
        updated_at: "2026-07-24T00:00:03Z".into(),
    };
    let mut store = TaskStore::open(database.path()).unwrap();
    store.save_scheduler_download_progress(&progress).unwrap();
    let before = store.load_download_snapshot("task-legacy-exit").unwrap();
    drop(store);
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("UPDATE task_events SET payload_json = json_set(payload_json, '$.command.item_ids', json('[]')) WHERE event_id IN ('legacy-exit-cancel','legacy-exit-retry')", []).unwrap();
    drop(connection);
    release_sender.send(()).unwrap();
    update_task(
        database.path(),
        "task-unrelated-repair",
        TaskStateCommandKind::ResumeTask,
        &[],
        "legacy-other-resume",
    );
    scheduler.wake();
    let unrelated = started.recv_timeout(WAIT).ok();
    let deadline = Instant::now() + Duration::from_secs(5);
    let (runtime, summary, snapshot, event_page) = loop {
        scheduler.wake();
        let store = TaskStore::open(database.path()).unwrap();
        let runtime = store
            .load_task("task-legacy-exit")
            .unwrap()
            .unwrap()
            .runtime;
        if runtime.status == TaskLifecycleStatus::Failed || Instant::now() >= deadline {
            let summary = store
                .load_task_failure_summaries(&["task-legacy-exit".to_owned()], &[])
                .unwrap();
            let snapshot = store.load_download_snapshot("task-legacy-exit").unwrap();
            let event_page = store
                .load_task_event_page(&insar_store::StoreTaskEventPageRequest {
                    task_id: "task-legacy-exit".into(),
                    expected_revision: runtime.revision,
                    before_sequence: None,
                    page_size: 20,
                })
                .unwrap();
            break (runtime, summary, snapshot, event_page);
        }
        thread::sleep(Duration::from_millis(10));
    };
    scheduler.shutdown();
    assert_eq!(
        unrelated,
        Some(100),
        "an unrelated task must remain schedulable"
    );
    assert_eq!(runtime.status, TaskLifecycleStatus::Failed);
    assert_eq!(runtime.failed_item_ids, ["S1-LEGACY-EXIT"]);
    assert_eq!(snapshot, before, "the prior progress is immutable evidence");
    let failure = summary.get("task-legacy-exit").expect("visible failure");
    assert_eq!(failure.codes, ["SCHEDULER_GENERATION_UNCERTAIN"]);
    assert!(failure
        .message_zh
        .as_deref()
        .is_some_and(|message| message.contains("无法确认取消/重试代际，需重新检查任务记录")));
    assert!(event_page.items.iter().any(|event| event
        .message_zh
        .contains("无法确认取消/重试代际，需重新检查任务记录")));
}

#[test]
fn multi_item_cancel_and_retry_evidence_uses_payload_membership() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[("task-multi-event", &["S1-MULTI-A", "S1-MULTI-B"], 2)],
    );
    let initial = update_task(
        database.path(),
        "task-multi-event",
        TaskStateCommandKind::ActivateItems,
        &["S1-MULTI-A", "S1-MULTI-B"],
        "multi-start",
    );
    update_task(
        database.path(),
        "task-multi-event",
        TaskStateCommandKind::CancelItems,
        &["S1-MULTI-A", "S1-MULTI-B"],
        "multi-cancel",
    );
    let retried = update_task(
        database.path(),
        "task-multi-event",
        TaskStateCommandKind::RetryItems,
        &["S1-MULTI-A", "S1-MULTI-B"],
        "multi-retry",
    );
    let result = DownloadItemResult {
        task_id: "task-multi-event".into(),
        item_id: "S1-MULTI-B".into(),
        outcome: DownloadOutcome::Cancelled,
        destination: std::env::temp_dir().join("multi-event-output"),
        bytes_written: 0,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-07-24T00:00:05Z".into(),
    };
    let mut store = TaskStore::open(database.path()).unwrap();
    let outcome = store
        .finish_scheduler_cancelled_result(
            &result,
            initial.revision,
            retried.revision,
            "multi-finish",
            false,
        )
        .unwrap();
    assert!(matches!(
        outcome,
        insar_store::SchedulerCancelledFinish::Retried(_)
    ));
    assert!(store
        .load_download_snapshot("task-multi-event")
        .unwrap()
        .results
        .is_empty());
}

#[test]
fn pause_resume_with_cancelled_worker_does_not_masquerade_as_retry() {
    let database = TempDatabase::new();
    seed_tasks(database.path(), &[("task-pause-queue", &["S1-PAUSE"], 1)]);
    let (started_sender, started) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let worker = Arc::new(CancelledResultWorker {
        started: started_sender,
        release: Mutex::new(release),
    });
    let scheduler = DownloadScheduler::start(database.path(), worker, POLL).unwrap();
    scheduler.wake();
    started.recv_timeout(WAIT).unwrap();
    update_task(
        database.path(),
        "task-pause-queue",
        TaskStateCommandKind::PauseTask,
        &[],
        "pause-task",
    );
    update_task(
        database.path(),
        "task-pause-queue",
        TaskStateCommandKind::PauseItems,
        &["S1-PAUSE"],
        "pause-item",
    );
    let queued = update_task(
        database.path(),
        "task-pause-queue",
        TaskStateCommandKind::ResumeItems,
        &["S1-PAUSE"],
        "resume-item",
    );
    assert!(queued.queued_item_ids.contains(&"S1-PAUSE".to_string()));
    release_sender.send(()).unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let state = TaskStore::open(database.path())
            .unwrap()
            .load_task("task-pause-queue")
            .unwrap()
            .unwrap();
        if state
            .runtime
            .cancelled_item_ids
            .contains(&"S1-PAUSE".to_string())
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "queued non-retry cancellation was ignored"
        );
        thread::sleep(Duration::from_millis(2));
    }
    scheduler.shutdown();
}

#[test]
fn live_cancelled_result_still_records_after_unrelated_revision() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[("task-live-result", &["S1-LIVE-RESULT"], 1)],
    );
    let (started_sender, started) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let worker = Arc::new(CancelledResultWorker {
        started: started_sender,
        release: Mutex::new(release),
    });
    let scheduler = DownloadScheduler::start(database.path(), worker, POLL).unwrap();
    scheduler.wake();
    started.recv_timeout(WAIT).unwrap();
    let runtime = update_task(
        database.path(),
        "task-live-result",
        TaskStateCommandKind::PauseTask,
        &[],
        "unrelated-pause",
    );
    assert_eq!(runtime.active_item_ids, vec!["S1-LIVE-RESULT".to_string()]);
    release_sender.send(()).unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let store = TaskStore::open(database.path()).unwrap();
        let state = store.load_task("task-live-result").unwrap().unwrap();
        if state
            .runtime
            .cancelled_item_ids
            .contains(&"S1-LIVE-RESULT".to_string())
        {
            let results = store
                .load_download_snapshot("task-live-result")
                .unwrap()
                .results;
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].outcome, DownloadOutcome::Cancelled);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "live cancellation was not recorded"
        );
        thread::sleep(Duration::from_millis(2));
    }
    scheduler.shutdown();
}

#[test]
fn task_level_stamped_membership_excludes_an_unrelated_completed_item() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[("task-stamp-excludes", &["S1-EXCLUDED", "S1-CANCELLED"], 2)],
    );
    let initial = update_task(
        database.path(),
        "task-stamp-excludes",
        TaskStateCommandKind::ActivateItems,
        &["S1-EXCLUDED", "S1-CANCELLED"],
        "exclude-start",
    );
    update_task(
        database.path(),
        "task-stamp-excludes",
        TaskStateCommandKind::MarkCompleted,
        &["S1-EXCLUDED"],
        "exclude-complete",
    );
    update_task(
        database.path(),
        "task-stamp-excludes",
        TaskStateCommandKind::CancelTask,
        &[],
        "exclude-cancel",
    );
    let retried = update_task(
        database.path(),
        "task-stamp-excludes",
        TaskStateCommandKind::RetryTask,
        &[],
        "exclude-retry",
    );
    let result = DownloadItemResult {
        task_id: "task-stamp-excludes".into(),
        item_id: "S1-EXCLUDED".into(),
        outcome: DownloadOutcome::Cancelled,
        destination: std::env::temp_dir().join("exclude-output"),
        bytes_written: 0,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-07-24T00:00:05Z".into(),
    };
    let mut store = TaskStore::open(database.path()).unwrap();
    let outcome = store
        .finish_scheduler_cancelled_result(
            &result,
            initial.revision,
            retried.revision,
            "exclude-finish",
            false,
        )
        .unwrap();
    assert!(
        matches!(
            outcome,
            insar_store::SchedulerCancelledFinish::Settled(None)
        ),
        "unrelated item must not be classified as retried"
    );
    let current = store.load_task("task-stamp-excludes").unwrap().unwrap();
    assert!(current
        .runtime
        .completed_item_ids
        .contains(&"S1-EXCLUDED".to_string()));
    assert!(store
        .load_download_snapshot("task-stamp-excludes")
        .unwrap()
        .results
        .is_empty());
}

#[test]
fn task_level_active_handoff_uses_stamped_items_for_each_item() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[("task-level-handoff", &["S1-TASK-A", "S1-TASK-B"], 2)],
    );
    let initial = update_task(
        database.path(),
        "task-level-handoff",
        TaskStateCommandKind::ActivateItems,
        &["S1-TASK-A", "S1-TASK-B"],
        "handoff-start",
    );
    let cancelled = update_task(
        database.path(),
        "task-level-handoff",
        TaskStateCommandKind::CancelTask,
        &[],
        "handoff-cancel",
    );
    let retried = update_task(
        database.path(),
        "task-level-handoff",
        TaskStateCommandKind::RetryTask,
        &[],
        "handoff-retry",
    );
    let result = DownloadItemResult {
        task_id: "task-level-handoff".into(),
        item_id: "S1-TASK-B".into(),
        outcome: DownloadOutcome::Cancelled,
        destination: std::env::temp_dir().join("task-level-output"),
        bytes_written: 0,
        attempt_count: 1,
        failure: None,
        finished_at: "2026-07-24T00:00:05Z".into(),
    };
    let mut store = TaskStore::open(database.path()).unwrap();
    assert!(cancelled
        .cancelled_item_ids
        .contains(&"S1-TASK-B".to_string()));
    assert!(retried.queued_item_ids.contains(&"S1-TASK-B".to_string()));
    let outcome = store
        .finish_scheduler_cancelled_result(
            &result,
            initial.revision,
            retried.revision,
            "handoff-finish",
            false,
        )
        .unwrap();
    assert!(matches!(
        outcome,
        insar_store::SchedulerCancelledFinish::Retried(_)
    ));
    assert!(store
        .load_download_snapshot("task-level-handoff")
        .unwrap()
        .results
        .is_empty());
}

#[test]
fn terminal_cancellation_reaches_a_live_pre_backend_worker_without_respawn() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[("task-terminal-cancel", &["S1-CANCEL"], 1)],
    );
    let (worker, waiting, cancelled) = PreBackendWorker::new();
    let scheduler =
        DownloadScheduler::start(database.path(), worker.clone(), POLL).expect("start scheduler");
    scheduler.wake();
    expect_waiting(&waiting);

    let cancelled_runtime = update_task(
        database.path(),
        "task-terminal-cancel",
        TaskStateCommandKind::CancelItems,
        &["S1-CANCEL"],
        "cancel-live-terminal-item",
    );
    assert_eq!(cancelled_runtime.status, TaskLifecycleStatus::Cancelled);
    assert!(
        TaskStore::open(database.path())
            .unwrap()
            .load_scheduler_tasks()
            .unwrap()
            .is_empty(),
        "a terminal task must remain outside the schedulable query"
    );

    scheduler.wake();
    assert_eq!(
        cancelled.recv_timeout(WAIT).unwrap(),
        WorkerEvent {
            task_id: "task-terminal-cancel".to_string(),
            item_id: "S1-CANCEL".to_string(),
        }
    );
    assert_eq!(worker.backend_calls.load(Ordering::SeqCst), 0);

    thread::sleep(Duration::from_millis(40));
    scheduler.wake();
    thread::sleep(Duration::from_millis(40));
    assert_eq!(worker.starts().len(), 1);
    scheduler.shutdown();
}

#[test]
fn shutdown_does_not_return_while_a_cancelled_worker_can_still_touch_its_destination() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[("task-shutdown-join", &["S1-SHUTDOWN"], 1)],
    );
    let (started_sender, started) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let (release, release_receiver) = mpsc::channel();
    let (exited_sender, exited) = mpsc::channel();
    let worker = Arc::new(ShutdownGateWorker {
        started: started_sender,
        cancelled: cancelled_sender,
        release: Mutex::new(release_receiver),
        exited: exited_sender,
    });
    let scheduler = DownloadScheduler::start(database.path(), worker, POLL).unwrap();
    scheduler.wake();
    started.recv_timeout(WAIT).unwrap();

    let (shutdown_done_sender, shutdown_done) = mpsc::channel();
    let shutdown = thread::spawn(move || {
        scheduler.shutdown();
        shutdown_done_sender.send(()).unwrap();
    });
    cancelled.recv_timeout(WAIT).unwrap();
    let returned_before_worker_exit = shutdown_done
        .recv_timeout(Duration::from_millis(100))
        .is_ok();

    release.send(()).unwrap();
    exited.recv_timeout(WAIT).unwrap();
    if !returned_before_worker_exit {
        shutdown_done.recv_timeout(WAIT).unwrap();
    }
    shutdown.join().unwrap();

    assert!(
        !returned_before_worker_exit,
        "scheduler shutdown returned while a cancelled worker was still live"
    );
}

#[test]
fn targeted_cancellation_leaves_another_live_item_running() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[
            ("task-targeted-cancel", &["S1-A", "S1-B"], 2),
            ("task-unrelated", &["S1-OTHER"], 1),
        ],
    );
    let (worker, waiting, cancelled) = PreBackendWorker::new();
    let scheduler =
        DownloadScheduler::start(database.path(), worker.clone(), POLL).expect("start scheduler");
    scheduler.wake();
    let waiting_items = [
        expect_waiting(&waiting).item_id,
        expect_waiting(&waiting).item_id,
        expect_waiting(&waiting).item_id,
    ];
    assert!(waiting_items.contains(&"S1-A".to_string()));
    assert!(waiting_items.contains(&"S1-B".to_string()));
    assert!(waiting_items.contains(&"S1-OTHER".to_string()));

    let runtime = update_task(
        database.path(),
        "task-targeted-cancel",
        TaskStateCommandKind::CancelItems,
        &["S1-A"],
        "cancel-only-first-item",
    );
    assert_eq!(runtime.status, TaskLifecycleStatus::Running);
    scheduler.wake();

    assert_eq!(
        cancelled.recv_timeout(WAIT).unwrap(),
        WorkerEvent {
            task_id: "task-targeted-cancel".to_string(),
            item_id: "S1-A".to_string(),
        }
    );
    worker.allow_backend.store(true, Ordering::SeqCst);
    wait_for_backend_calls(&worker, 2);
    assert_eq!(worker.starts().len(), 3);
    scheduler.shutdown();
}

#[test]
fn missing_runtime_cancels_its_live_controls() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[("task-missing-runtime", &["S1-MISSING"], 1)],
    );
    let (worker, waiting, cancelled) = PreBackendWorker::new();
    let scheduler =
        DownloadScheduler::start(database.path(), worker, POLL).expect("start scheduler");
    scheduler.wake();
    expect_waiting(&waiting);

    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute(
            "DELETE FROM task_runtime WHERE task_id = ?1",
            ["task-missing-runtime"],
        )
        .unwrap();
    drop(connection);
    scheduler.wake();

    assert_eq!(
        cancelled.recv_timeout(WAIT).unwrap(),
        WorkerEvent {
            task_id: "task-missing-runtime".to_string(),
            item_id: "S1-MISSING".to_string(),
        }
    );
    scheduler.shutdown();
}

#[test]
fn control_runtime_error_prevents_new_claims_or_spawns() {
    let database = TempDatabase::new();
    seed_tasks(
        database.path(),
        &[
            ("task-live-runtime-error", &["S1-LIVE"], 1),
            ("task-paused-before-error", &["S1-QUEUED"], 1),
        ],
    );
    update_task(
        database.path(),
        "task-paused-before-error",
        TaskStateCommandKind::PauseTask,
        &[],
        "pause-before-scheduler",
    );

    let (worker, waiting, cancelled) = PreBackendWorker::new();
    let scheduler =
        DownloadScheduler::start(database.path(), worker.clone(), POLL).expect("start scheduler");
    scheduler.wake();
    assert_eq!(
        expect_waiting(&waiting),
        WorkerEvent {
            task_id: "task-live-runtime-error".to_string(),
            item_id: "S1-LIVE".to_string(),
        }
    );

    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute(
            "UPDATE task_runtime SET state_json = '{not-json' WHERE task_id = ?1",
            ["task-live-runtime-error"],
        )
        .unwrap();
    drop(connection);
    update_task(
        database.path(),
        "task-paused-before-error",
        TaskStateCommandKind::ResumeTask,
        &[],
        "resume-after-control-error",
    );

    scheduler.wake();
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        worker.starts(),
        vec![WorkerEvent {
            task_id: "task-live-runtime-error".to_string(),
            item_id: "S1-LIVE".to_string(),
        }],
        "a failed control reconciliation must not spawn queued work"
    );
    assert_eq!(worker.backend_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(cancelled.try_recv(), Err(TryRecvError::Empty)));
    scheduler.shutdown();
}

struct RestartCancelFixture {
    scheduler: DownloadScheduler,
    worker: Arc<ResultHandoffWorker>,
    started: Receiver<usize>,
    release: Sender<()>,
}

fn prepare_restart_cancel(path: &Path, task_id: &str, item_id: &str) -> RestartCancelFixture {
    seed_tasks(
        path,
        &[
            (task_id, &[item_id], 1),
            ("task-restart-probe", &["S1-RESTART-PROBE"], 1),
        ],
    );
    update_task(
        path,
        "task-restart-probe",
        TaskStateCommandKind::PauseTask,
        &[],
        "restart-probe-pause",
    );
    let (started_sender, started) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let (release, release_receiver) = mpsc::channel();
    let worker = Arc::new(ResultHandoffWorker {
        started: started_sender,
        cancelled: cancelled_sender,
        release_first: Mutex::new(release_receiver),
        starts: AtomicUsize::new(0),
    });
    let scheduler = DownloadScheduler::start(path, worker.clone(), POLL).unwrap();
    scheduler.wake();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 0);
    update_task(
        path,
        task_id,
        TaskStateCommandKind::CancelTask,
        &[],
        "restart-cancel",
    );
    scheduler.wake();
    cancelled.recv_timeout(WAIT).unwrap();
    update_task(
        path,
        task_id,
        TaskStateCommandKind::RetryTask,
        &[],
        "restart-retry",
    );
    scheduler.wake();
    let deadline = Instant::now() + WAIT;
    loop {
        let store = TaskStore::open(path).unwrap();
        let runtime = store.load_task(task_id).unwrap().unwrap().runtime;
        if runtime.active_item_ids == [item_id] {
            assert_eq!(runtime.status, TaskLifecycleStatus::Running);
            break;
        }
        assert!(Instant::now() < deadline, "retry was not activated");
        thread::sleep(POLL);
    }
    let mut store = TaskStore::open(path).unwrap();
    store
        .save_scheduler_download_progress(&insar_core::DownloadItemProgress {
            task_id: task_id.into(),
            item_id: item_id.into(),
            status: insar_core::DownloadItemStatus::Active,
            bytes_completed: 137,
            total_bytes: Some(500),
            bytes_per_second: 9.0,
            attempt: 2,
            updated_at: "2026-07-24T00:00:03Z".into(),
        })
        .unwrap();
    store
        .save_scheduler_download_result(&DownloadItemResult {
            task_id: task_id.into(),
            item_id: item_id.into(),
            outcome: DownloadOutcome::Cancelled,
            destination: path.parent().unwrap().join("restart-prior-result"),
            bytes_written: 37,
            attempt_count: 1,
            failure: None,
            finished_at: "2026-07-24T00:00:01Z".into(),
        })
        .unwrap();
    RestartCancelFixture {
        scheduler,
        worker,
        started,
        release,
    }
}

struct RestartProgressWorker(Arc<PreBackendWorker>);

impl DownloadWorker for RestartProgressWorker {
    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(insar_core::DownloadItemProgress),
    ) -> DownloadItemResult {
        progress(insar_core::DownloadItemProgress {
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            status: insar_core::DownloadItemStatus::Active,
            bytes_completed: 211,
            total_bytes: Some(500),
            bytes_per_second: 11.0,
            attempt: 3,
            updated_at: "2026-07-24T00:00:06Z".into(),
        });
        self.0.execute(work, control, progress)
    }
}

#[test]
fn restart_pending_cancel_uses_active_runtime_without_replaying_old_download_state() {
    let database = TempDatabase::new();
    let task_id = "task-restart-pending";
    let item_id = "S1-RESTART-PENDING";
    let RestartCancelFixture {
        scheduler,
        worker,
        started,
        release,
    } = prepare_restart_cancel(database.path(), task_id, item_id);
    {
        let connection = Connection::open(database.path()).unwrap();
        connection
            .execute(
                "UPDATE task_events SET payload_json = '{}' WHERE event_id = 'restart-retry'",
                [],
            )
            .unwrap();
    }
    let store = TaskStore::open(database.path()).unwrap();
    let before = store.load_download_snapshot(task_id).unwrap();
    let runtime = store.load_task(task_id).unwrap().unwrap().runtime;
    drop(store);
    release.send(()).unwrap();
    assert!(
        started.recv_timeout(Duration::from_millis(1_250)).is_err(),
        "the unresolved old result must stay pending"
    );
    scheduler.shutdown();
    assert_eq!(worker.starts.load(Ordering::SeqCst), 1);
    drop(worker);
    drop(started);

    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_task(task_id).unwrap().unwrap().runtime,
        runtime
    );
    assert_eq!(reopened.load_download_snapshot(task_id).unwrap(), before);
    drop(reopened);
    let (fresh_worker, waiting, _) = PreBackendWorker::new();
    let fresh = DownloadScheduler::start(
        database.path(),
        Arc::new(RestartProgressWorker(fresh_worker.clone())),
        POLL,
    )
    .unwrap();
    fresh.wake();
    assert_eq!(
        expect_waiting(&waiting),
        WorkerEvent {
            task_id: task_id.into(),
            item_id: item_id.into()
        }
    );
    // A new worker's progress must replace the old generation's sample, not
    // resurrect an in-memory pending result or reset the durable runtime.
    let deadline = Instant::now() + WAIT;
    let fresh_snapshot = loop {
        let snapshot = TaskStore::open(database.path())
            .unwrap()
            .load_download_snapshot(task_id)
            .unwrap();
        if snapshot
            .progress
            .iter()
            .any(|sample| sample.item_id == item_id && sample.bytes_completed == 211)
        {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "fresh worker progress was not recorded"
        );
        thread::sleep(POLL);
    };
    assert_eq!(fresh_snapshot.results, Vec::<DownloadItemResult>::new());
    assert_eq!(
        fresh_snapshot
            .progress
            .iter()
            .find(|sample| sample.item_id == item_id)
            .map(|sample| sample.bytes_completed),
        Some(211)
    );
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_task(task_id).unwrap().unwrap().runtime,
        runtime
    );
    assert_eq!(
        reopened.load_download_snapshot(task_id).unwrap(),
        fresh_snapshot
    );
    assert!(reopened
        .load_task_failure_summaries(&[task_id.into()], &[])
        .unwrap()
        .is_empty());
    assert!(
        reopened
            .load_scheduler_pending_cancelled_finishes()
            .unwrap()
            .is_empty(),
        "a kept result its evidence cannot settle is released after restart"
    );
    assert_eq!(fresh_worker.starts().len(), 1);
    drop(reopened);
    fresh.shutdown();
}

#[test]
fn restart_settles_a_cancelled_result_the_previous_run_could_not_record() {
    let database = TempDatabase::new();
    let task_id = "task-restart-kept";
    let item_id = "S1-RESTART-KEPT";
    seed_tasks(database.path(), &[(task_id, &[item_id], 1)]);
    let (started_sender, started) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let (release, release_receiver) = mpsc::channel();
    let worker = Arc::new(ResultHandoffWorker {
        started: started_sender,
        cancelled: cancelled_sender,
        release_first: Mutex::new(release_receiver),
        starts: AtomicUsize::new(0),
    });
    let old = DownloadScheduler::start(database.path(), worker.clone(), POLL).unwrap();
    old.wake();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 0);
    let runtime = update_task(
        database.path(),
        task_id,
        TaskStateCommandKind::CancelTask,
        &[],
        "restart-kept-cancel",
    );
    old.wake();
    cancelled.recv_timeout(WAIT).unwrap();
    // The cancel evidence cannot be read while the old run handles the result.
    let connection = Connection::open(database.path()).unwrap();
    connection
        .execute_batch("ALTER TABLE task_events RENAME TO task_events_offline")
        .unwrap();
    release.send(()).unwrap();
    let kept = |connection: &Connection| -> i64 {
        connection
            .query_row(
                "SELECT count(*) FROM scheduler_pending_cancelled_finishes WHERE task_id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap()
    };
    let deadline = Instant::now() + WAIT;
    while kept(&connection) != 1 {
        assert!(
            Instant::now() < deadline,
            "the unsettled result was not kept"
        );
        thread::sleep(POLL);
    }
    old.shutdown();
    assert_eq!(worker.starts.load(Ordering::SeqCst), 1);
    connection
        .execute_batch("ALTER TABLE task_events_offline RENAME TO task_events")
        .unwrap();
    assert!(TaskStore::open(database.path())
        .unwrap()
        .load_download_snapshot(task_id)
        .unwrap()
        .results
        .is_empty());

    let (fresh_worker, waiting, _) = PreBackendWorker::new();
    let fresh = DownloadScheduler::start(database.path(), fresh_worker.clone(), POLL).unwrap();
    fresh.wake();
    let deadline = Instant::now() + WAIT;
    let snapshot = loop {
        let snapshot = TaskStore::open(database.path())
            .unwrap()
            .load_download_snapshot(task_id)
            .unwrap();
        if !snapshot.results.is_empty() {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "the kept result was not settled after restart"
        );
        thread::sleep(POLL);
    };
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.results[0].outcome, DownloadOutcome::Cancelled);
    assert_eq!(kept(&connection), 0);
    assert!(
        waiting.recv_timeout(Duration::from_millis(300)).is_err(),
        "the cancelled item must not run again"
    );
    assert_eq!(
        TaskStore::open(database.path())
            .unwrap()
            .load_task(task_id)
            .unwrap()
            .unwrap()
            .runtime,
        runtime
    );
    assert!(fresh_worker.starts().is_empty());
    fresh.shutdown();
}

#[test]
fn restart_persisted_cancel_without_retry_does_not_spawn_cancelled_item() {
    let database = TempDatabase::new();
    let task_id = "task-restart-cancelled";
    let item_id = "S1-RESTART-CANCELLED";
    seed_tasks(
        database.path(),
        &[
            (task_id, &[item_id], 1),
            ("task-restart-probe", &["S1-RESTART-PROBE"], 1),
        ],
    );
    update_task(
        database.path(),
        "task-restart-probe",
        TaskStateCommandKind::PauseTask,
        &[],
        "restart-probe-pause",
    );
    let (old_worker, waiting, cancelled) = PreBackendWorker::new();
    let old = DownloadScheduler::start(database.path(), old_worker.clone(), POLL).unwrap();
    old.wake();
    assert_eq!(expect_waiting(&waiting).item_id, item_id);
    let runtime = update_task(
        database.path(),
        task_id,
        TaskStateCommandKind::CancelTask,
        &[],
        "restart-terminal-cancel",
    );
    old.wake();
    assert_eq!(cancelled.recv_timeout(WAIT).unwrap().item_id, item_id);
    let deadline = Instant::now() + WAIT;
    let snapshot = loop {
        let snapshot = TaskStore::open(database.path())
            .unwrap()
            .load_download_snapshot(task_id)
            .unwrap();
        if !snapshot.results.is_empty() {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "cancel result was not committed");
        thread::sleep(POLL);
    };
    assert_eq!(snapshot.results.len(), 1);
    assert_eq!(snapshot.results[0].outcome, DownloadOutcome::Cancelled);
    old.shutdown();
    assert_eq!(old_worker.starts().len(), 1);
    drop(old_worker);
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_task(task_id).unwrap().unwrap().runtime,
        runtime
    );
    assert_eq!(reopened.load_download_snapshot(task_id).unwrap(), snapshot);
    drop(reopened);

    update_task(
        database.path(),
        "task-restart-probe",
        TaskStateCommandKind::ResumeTask,
        &[],
        "restart-probe-resume",
    );
    let (fresh_worker, waiting, _) = PreBackendWorker::new();
    let fresh = DownloadScheduler::start(database.path(), fresh_worker.clone(), POLL).unwrap();
    fresh.wake();
    assert_eq!(expect_waiting(&waiting).item_id, "S1-RESTART-PROBE");
    assert!(
        waiting.recv_timeout(Duration::from_millis(1_250)).is_err(),
        "cancelled item must not execute after restart"
    );
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_task(task_id).unwrap().unwrap().runtime,
        runtime
    );
    assert_eq!(reopened.load_download_snapshot(task_id).unwrap(), snapshot);
    assert_eq!(fresh_worker.starts().len(), 1);
    drop(reopened);
    fresh.shutdown();
}

#[test]
fn restart_committed_generation_uncertain_failure_preserves_progress_and_summary() {
    let database = TempDatabase::new();
    let task_id = "task-restart-failed";
    let item_id = "S1-RESTART-FAILED";
    let RestartCancelFixture {
        scheduler,
        worker,
        started,
        release,
    } = prepare_restart_cancel(database.path(), task_id, item_id);
    let mut store = TaskStore::open(database.path()).unwrap();
    store
        .save_scheduler_download_progress(&insar_core::DownloadItemProgress {
            task_id: task_id.into(),
            item_id: item_id.into(),
            status: insar_core::DownloadItemStatus::Active,
            bytes_completed: 137,
            total_bytes: Some(500),
            bytes_per_second: 9.0,
            attempt: 2,
            updated_at: "2026-07-24T00:00:03Z".into(),
        })
        .unwrap();
    let before = store.load_download_snapshot(task_id).unwrap();
    assert_eq!(before.results, Vec::<DownloadItemResult>::new());
    assert_eq!(before.progress[0].bytes_completed, 137);
    drop(store);
    erase_stamped_item_membership(database.path(), &["restart-cancel", "restart-retry"]);
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let runtime = loop {
        let runtime = TaskStore::open(database.path())
            .unwrap()
            .load_task(task_id)
            .unwrap()
            .unwrap()
            .runtime;
        if runtime.status == TaskLifecycleStatus::Failed {
            break runtime;
        }
        assert!(
            Instant::now() < deadline,
            "generation uncertainty was not committed"
        );
        thread::sleep(POLL);
    };
    assert_eq!(runtime.failed_item_ids, [item_id]);
    let store = TaskStore::open(database.path()).unwrap();
    let summary = store
        .load_task_failure_summaries(&[task_id.into()], &[])
        .unwrap();
    assert_eq!(
        summary.get(task_id).unwrap().codes,
        ["SCHEDULER_GENERATION_UNCERTAIN"]
    );
    let events = store.list_task_events(task_id).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "mark_failed")
            .count(),
        1
    );
    assert_eq!(store.load_download_snapshot(task_id).unwrap(), before);
    drop(store);
    scheduler.shutdown();
    assert_eq!(worker.starts.load(Ordering::SeqCst), 1);
    drop(worker);
    drop(started);

    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_task(task_id).unwrap().unwrap().runtime,
        runtime
    );
    assert_eq!(reopened.load_download_snapshot(task_id).unwrap(), before);
    assert_eq!(
        reopened
            .load_task_failure_summaries(&[task_id.into()], &[])
            .unwrap(),
        summary
    );
    drop(reopened);
    update_task(
        database.path(),
        "task-restart-probe",
        TaskStateCommandKind::ResumeTask,
        &[],
        "restart-probe-resume",
    );
    let (fresh_worker, waiting, _) = PreBackendWorker::new();
    let fresh = DownloadScheduler::start(database.path(), fresh_worker.clone(), POLL).unwrap();
    fresh.wake();
    assert_eq!(expect_waiting(&waiting).item_id, "S1-RESTART-PROBE");
    assert!(
        waiting.recv_timeout(Duration::from_millis(1_250)).is_err(),
        "failed item must remain terminal"
    );
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_task(task_id).unwrap().unwrap().runtime,
        runtime
    );
    assert_eq!(reopened.load_download_snapshot(task_id).unwrap(), before);
    assert_eq!(
        reopened
            .load_task_failure_summaries(&[task_id.into()], &[])
            .unwrap(),
        summary
    );
    assert_eq!(reopened.list_task_events(task_id).unwrap(), events);
    assert_eq!(fresh_worker.starts().len(), 1);
    drop(reopened);
    fresh.shutdown();
}

#[test]
fn restart_rejected_generation_failure_is_kept_and_applied_once_the_store_recovers() {
    let database = TempDatabase::new();
    let task_id = "task-restart-rejected";
    let item_id = "S1-RESTART-REJECTED";
    let RestartCancelFixture {
        scheduler,
        worker,
        started,
        release,
    } = prepare_restart_cancel(database.path(), task_id, item_id);
    erase_stamped_item_membership(database.path(), &["restart-cancel", "restart-retry"]);
    {
        let connection = Connection::open(database.path()).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_restart_failure BEFORE INSERT ON task_events WHEN NEW.event_type='mark_failed' BEGIN SELECT RAISE(ABORT,'fixture restart write failure'); END;").unwrap();
    }
    let store = TaskStore::open(database.path()).unwrap();
    let before = store.load_download_snapshot(task_id).unwrap();
    let runtime = store.load_task(task_id).unwrap().unwrap().runtime;
    let events = store.list_task_events(task_id).unwrap();
    drop(store);
    release.send(()).unwrap();
    assert!(
        started.recv_timeout(Duration::from_millis(1_250)).is_err(),
        "rejected failure must keep the old finish pending"
    );
    scheduler.shutdown();
    assert_eq!(worker.starts.load(Ordering::SeqCst), 1);
    drop(worker);
    drop(started);
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_task(task_id).unwrap().unwrap().runtime,
        runtime
    );
    assert_eq!(reopened.load_download_snapshot(task_id).unwrap(), before);
    assert_eq!(reopened.list_task_events(task_id).unwrap(), events);
    assert!(reopened
        .load_task_failure_summaries(&[task_id.into()], &[])
        .unwrap()
        .is_empty());
    // The rejected write leaves the decision kept in the Store, not invented.
    assert_eq!(
        reopened
            .load_scheduler_pending_cancelled_finishes()
            .unwrap()
            .len(),
        1
    );
    drop(reopened);
    // Repair the test Store only after the old coordinator is gone. The kept
    // decision is applied after restart exactly as it would have been had the
    // process stayed up (legacy_generation_failed_event_write_stays_pending_
    // until_store_recovers); the old item does not start again meanwhile.
    {
        let connection = Connection::open(database.path()).unwrap();
        connection
            .execute_batch("DROP TRIGGER reject_restart_failure;")
            .unwrap();
    }
    let (fresh_worker, waiting, _) = PreBackendWorker::new();
    let fresh = DownloadScheduler::start(database.path(), fresh_worker.clone(), POLL).unwrap();
    fresh.wake();
    let deadline = Instant::now() + WAIT;
    let failed = loop {
        let runtime = TaskStore::open(database.path())
            .unwrap()
            .load_task(task_id)
            .unwrap()
            .unwrap()
            .runtime;
        if runtime.status == TaskLifecycleStatus::Failed {
            break runtime;
        }
        assert!(
            Instant::now() < deadline,
            "the kept generation failure was not applied after restart"
        );
        thread::sleep(POLL);
    };
    assert_eq!(failed.failed_item_ids, [item_id]);
    assert_eq!(failed.revision, runtime.revision + 1);
    assert!(
        waiting.recv_timeout(Duration::from_millis(300)).is_err(),
        "the failed item waits for the user's retry"
    );
    let reopened = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        reopened.load_download_snapshot(task_id).unwrap(),
        before,
        "the prior progress is immutable evidence"
    );
    let summary = reopened
        .load_task_failure_summaries(&[task_id.into()], &[])
        .unwrap();
    assert_eq!(summary[task_id].codes, ["SCHEDULER_GENERATION_UNCERTAIN"]);
    assert!(reopened
        .load_scheduler_pending_cancelled_finishes()
        .unwrap()
        .is_empty());
    assert!(fresh_worker.starts().is_empty());
    drop(reopened);
    fresh.shutdown();
}

#[test]
fn legacy_generation_failed_event_write_stays_pending_until_store_recovers() {
    let database = TempDatabase::new();
    seed_tasks(database.path(), &[("task-legacy-io", &["S1-LEGACY-IO"], 1)]);
    let (started_sender, started) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let (release_sender, release) = mpsc::channel();
    let worker = Arc::new(ResultHandoffWorker {
        started: started_sender,
        cancelled: cancelled_sender,
        release_first: Mutex::new(release),
        starts: AtomicUsize::new(0),
    });
    let scheduler = DownloadScheduler::start(database.path(), worker, POLL).unwrap();
    scheduler.wake();
    assert_eq!(started.recv_timeout(WAIT).unwrap(), 0);
    update_task(
        database.path(),
        "task-legacy-io",
        TaskStateCommandKind::CancelTask,
        &[],
        "io-cancel",
    );
    scheduler.wake();
    cancelled.recv_timeout(WAIT).unwrap();
    update_task(
        database.path(),
        "task-legacy-io",
        TaskStateCommandKind::RetryTask,
        &[],
        "io-retry",
    );
    scheduler.wake();
    let deadline = Instant::now() + WAIT;
    loop {
        let store = TaskStore::open(database.path()).unwrap();
        if store
            .load_task("task-legacy-io")
            .unwrap()
            .unwrap()
            .runtime
            .active_item_ids
            .contains(&"S1-LEGACY-IO".to_owned())
        {
            break;
        }
        assert!(Instant::now() < deadline, "retry was not activated");
        thread::sleep(Duration::from_millis(2));
    }
    let connection = Connection::open(database.path()).unwrap();
    connection.execute("UPDATE task_events SET payload_json=json_set(payload_json,'$.command.item_ids',json('[]')) WHERE event_id IN ('io-cancel','io-retry')", []).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_legacy_finish BEFORE INSERT ON task_events WHEN NEW.event_type='mark_failed' BEGIN SELECT RAISE(ABORT,'fixture write failure'); END;").unwrap();
    release_sender.send(()).unwrap();
    assert!(
        started.recv_timeout(Duration::from_millis(1250)).is_err(),
        "failed persistence must not release the pending item"
    );
    let store = TaskStore::open(database.path()).unwrap();
    let blocked = store.load_task("task-legacy-io").unwrap().unwrap().runtime;
    assert_eq!(blocked.status, TaskLifecycleStatus::Running);
    assert!(store
        .load_task_failure_summaries(&["task-legacy-io".into()], &[])
        .unwrap()
        .is_empty());
    drop(store);
    connection
        .execute_batch("DROP TRIGGER reject_legacy_finish;")
        .unwrap();
    drop(connection);
    scheduler.wake();
    let deadline = Instant::now() + WAIT;
    loop {
        let store = TaskStore::open(database.path()).unwrap();
        if store
            .load_task("task-legacy-io")
            .unwrap()
            .unwrap()
            .runtime
            .status
            == TaskLifecycleStatus::Failed
        {
            break;
        }
        assert!(Instant::now() < deadline, "pending finish did not recover");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(started.try_recv().is_err());
    scheduler.shutdown();
}
