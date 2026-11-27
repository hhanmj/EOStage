//! Orbit scheduled-fetch S3 (task contract 2026-11-01, sections 3.4 and 3.9):
//! the scheduler-side recheck gate.
//!
//! RED-first plan for this slice. What this file pins down:
//!
//! - an ORB002 ("precise orbit not published yet") finish no longer fails the
//!   item: the item goes back to queued and one schedule row is written with
//!   the next check derived from acquisition + 20 d and the 12 h → 24 h →
//!   48 h → 96 h → 7 d backoff capped at the 60 d deadline;
//! - a waiting row whose `next_check_at` is in the future is not claimed (no
//!   worker start, no capacity use) and does not delay another task's items;
//! - a due row is claimed once and a successful download clears the row;
//! - crossing the deadline turns the wait into an ORB003 failure with no
//!   further requests;
//! - `auto_fetch_orbit` off keeps today's behavior (ORB002 fails the item);
//! - cancelling the item or the task clears the rows, and a terminal task is
//!   not revived;
//! - a restart restores the gate from the persisted rows;
//! - the 100 ms production poll interval constant is untouched.
//!
//! No real network is used anywhere: the provider loopback endpoint belongs to
//! slice S1's tests; here the fake worker returns ORB002/ORB003 results the
//! way `production_worker.rs` failure_result would.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadLayout, DownloadOutcome,
    FrozenSearchBatch, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreSearchBatchRequest, StoreTaskCommand, StoreUpdateTaskRequest, TaskLifecycleStatus,
    TaskStateCommandKind,
};
use insar_download::{
    DownloadScheduler, DownloadWork, DownloadWorker, OrbitRecheckClock, TaskTransferSample,
    WorkerControl,
};
use insar_store::{
    StoreCompanionPreferences, StoreOrbitRecheckRequest, StoreOrbitRecheckRow,
    StoreOrbitRecheckState, TaskStore,
};
use serde_json::Map;

const POLL: Duration = Duration::from_millis(5);
const WAIT: Duration = Duration::from_secs(5);

/// The fixed wall clock every scheduler in this file starts with. Whole
/// seconds, UTC, canonical form (the recheck table orders timestamps as text).
const CLOCK_NOW: &str = "2026-11-01T12:00:00Z";

struct TempDatabase(PathBuf);

impl TempDatabase {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "insar-orbit-recheck-{label}-{}-{nonce}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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

/// A frozen clock: `CLOCK_NOW` until the test moves it forward, which only
/// affects recheck scheduling, never the poll cadence.
struct FrozenClock {
    now_epoch_seconds: AtomicU64,
}

impl FrozenClock {
    fn new() -> Self {
        Self {
            now_epoch_seconds: AtomicU64::new(Self::seconds(CLOCK_NOW)),
        }
    }

    /// Parse a whole-second UTC timestamp the way the fixtures spell them.
    fn seconds(timestamp: &str) -> u64 {
        parse_utc_seconds(timestamp).expect("fixture timestamp must be a whole-second UTC value")
    }

    fn set(&self, timestamp: &str) {
        self.now_epoch_seconds
            .store(Self::seconds(timestamp), Ordering::SeqCst);
    }

    fn advance_to(&self, timestamp: &str) {
        self.set(timestamp);
    }
}

impl OrbitRecheckClock for FrozenClock {
    fn now_utc_seconds(&self) -> u64 {
        self.now_epoch_seconds.load(Ordering::SeqCst)
    }
}

/// Whole-second UTC RFC3339 → epoch seconds (no chrono dependency here; the
/// fixtures use a fixed 2026 window).
fn parse_utc_seconds(timestamp: &str) -> Option<u64> {
    let bytes = timestamp.as_bytes();
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let year: i64 = timestamp[0..4].parse().ok()?;
    let month: i64 = timestamp[5..7].parse().ok()?;
    let day: i64 = timestamp[8..10].parse().ok()?;
    let hour: i64 = timestamp[11..13].parse().ok()?;
    let minute: i64 = timestamp[14..16].parse().ok()?;
    let second: i64 = timestamp[17..19].parse().ok()?;
    if !(bytes[13] == b':' && bytes[16] == b':' && bytes[19] == b'Z') {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(seconds).ok()
}

/// Howard Hinnant's days_from_civil.
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

fn format_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3_600,
        (second_of_day % 3_600) / 60,
        second_of_day % 60
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn orbit_scene_item(scene_id: &str, acquisition: &str) -> SnapshotItem {
    SnapshotItem {
        item_id: scene_id.to_owned(),
        provider_id: "sentinel1.orbit".to_owned(),
        display_name: scene_id.to_owned(),
        acquisition_datetime: Some(acquisition.to_owned()),
        footprint_geojson: None,
        metadata: Map::new(),
        payload: Map::new(),
        asset_access: None,
        execution_policy: None,
    }
}

fn create_orbit_task(
    store: &mut TaskStore,
    path: &Path,
    task_id: &str,
    scene_id: &str,
    acquisition: &str,
    concurrency: u64,
) {
    let batch_id = format!("{task_id}-batch");
    store
        .save_search_batch_and_create_task(
            &StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: batch_id.clone(),
                    provider_id: "sentinel1.orbit".to_owned(),
                    created_at: "2026-11-01T00:00:00Z".to_owned(),
                    aoi_name: "Fixture AOI".to_owned(),
                    aoi_geojson: None,
                    query: Map::new(),
                    search_stats: None,
                    candidates: vec![orbit_scene_item(scene_id, acquisition)],
                },
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.clone(),
                    checked_item_ids: vec![scene_id.to_owned()],
                    page_index: 0,
                    page_size: 1,
                    revision: 0,
                },
                activate_workbench: false,
            },
            &StoreCreateTaskRequest {
                task_id: task_id.to_owned(),
                batch_id,
                selected_item_ids: vec![scene_id.to_owned()],
                checked_item_ids: vec![scene_id.to_owned()],
                page_index: 0,
                page_size: 1,
                output_root: path.parent().unwrap().join(format!("{task_id}-output")),
                layout: DownloadLayout::SentinelOrbitAuxPoeorb,
                concurrency,
                created_at: "2026-11-01T00:00:00Z".to_owned(),
            },
        )
        .unwrap();
}

fn create_plain_task(
    store: &mut TaskStore,
    path: &Path,
    task_id: &str,
    item_ids: &[&str],
    concurrency: u64,
) {
    let batch_id = format!("{task_id}-batch");
    let candidates = item_ids
        .iter()
        .map(|item_id| SnapshotItem {
            item_id: item_id.to_string(),
            provider_id: "asf.sentinel1".to_owned(),
            display_name: format!("{item_id}.zip"),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::new(),
            asset_access: None,
            execution_policy: None,
        })
        .collect::<Vec<_>>();
    store
        .save_search_batch_and_create_task(
            &StoreSearchBatchRequest {
                batch: FrozenSearchBatch {
                    schema_version: 1,
                    batch_id: batch_id.clone(),
                    provider_id: "asf.sentinel1".to_owned(),
                    created_at: "2026-11-01T00:00:00Z".to_owned(),
                    aoi_name: "Fixture AOI".to_owned(),
                    aoi_geojson: None,
                    query: Map::new(),
                    search_stats: None,
                    candidates,
                },
                workbench: SearchWorkbenchState {
                    batch_id: batch_id.clone(),
                    checked_item_ids: item_ids.iter().map(|id| id.to_string()).collect(),
                    page_index: 0,
                    page_size: item_ids.len().max(1) as u64,
                    revision: 0,
                },
                activate_workbench: false,
            },
            &StoreCreateTaskRequest {
                task_id: task_id.to_owned(),
                batch_id,
                selected_item_ids: item_ids.iter().map(|id| id.to_string()).collect(),
                checked_item_ids: item_ids.iter().map(|id| id.to_string()).collect(),
                page_index: 0,
                page_size: item_ids.len().max(1) as u64,
                output_root: path.parent().unwrap().join(format!("{task_id}-output")),
                layout: DownloadLayout::Sentinel1Slc,
                concurrency,
                created_at: "2026-11-01T00:00:00Z".to_owned(),
            },
        )
        .unwrap();
}

/// A worker whose items answer from a script: which item fails with which
/// code, and which item completes. Counts every start so a test can assert
/// "no request" (the loopback request-count assertion of the contract, at the
/// scheduler seam).
struct ScriptedWorker {
    starts: Mutex<Vec<(String, String)>>,
    /// item_id → failure code returned on every start ("ORB002", "ORB003", …).
    failures: Mutex<std::collections::HashMap<String, &'static str>>,
}

impl ScriptedWorker {
    fn new() -> Self {
        Self {
            starts: Mutex::new(Vec::new()),
            failures: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn fail_item(&self, item_id: &str, code: &'static str) {
        self.failures
            .lock()
            .unwrap()
            .insert(item_id.to_owned(), code);
    }

    fn starts_of(&self, item_id: &str) -> usize {
        self.starts
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, started_item)| started_item == item_id)
            .count()
    }

    fn total_starts(&self) -> usize {
        self.starts.lock().unwrap().len()
    }

    fn wait_for_starts(&self, count: usize) {
        let deadline = Instant::now() + WAIT;
        while self.total_starts() < count {
            assert!(
                Instant::now() < deadline,
                "worker did not start {count} items; starts so far: {:?}",
                self.starts.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl DownloadWorker for ScriptedWorker {
    fn task_transfer_sample(&self, task_id: &str) -> Option<TaskTransferSample> {
        let has_start = self
            .starts
            .lock()
            .unwrap()
            .iter()
            .any(|(started_task, _)| started_task == task_id);
        has_start.then_some(TaskTransferSample {
            network_bytes_received: 0,
            active_millis: 1,
            active_count: 0,
            bytes_per_second: 0.0,
        })
    }

    fn execute(
        &self,
        work: DownloadWork,
        _control: WorkerControl,
        _progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        {
            let mut starts = self.starts.lock().unwrap();
            starts.push((work.task.task_id.clone(), work.item.item_id.clone()));
        }
        let code = self
            .failures
            .lock()
            .unwrap()
            .get(&work.item.item_id)
            .copied();
        let (outcome, failure) = match code {
            Some(code) => (
                DownloadOutcome::Failed,
                Some(DownloadFailure {
                    code: code.to_owned(),
                    reason_zh: "fixture orbit failure".to_owned(),
                    retryable: true,
                }),
            ),
            None => (DownloadOutcome::Completed, None),
        };
        let completed = outcome == DownloadOutcome::Completed;
        DownloadItemResult {
            task_id: work.task.task_id,
            item_id: work.item.item_id.clone(),
            outcome,
            destination: work.task.output_root.join("fixture.zip"),
            bytes_written: u64::from(completed),
            attempt_count: 1,
            failure,
            finished_at: "2026-11-01T12:00:01Z".to_owned(),
        }
    }
}

fn wait_until(description: &str, check: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !check() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        thread::sleep(Duration::from_millis(3));
    }
}

fn recheck_row(store: &TaskStore, task_id: &str, item_id: &str) -> Option<StoreOrbitRecheckRow> {
    store.load_orbit_recheck(task_id, item_id).unwrap()
}

fn load_runtime(store: &TaskStore, task_id: &str) -> insar_core::TaskRuntimeSnapshot {
    let task = store.load_task_record(task_id).unwrap().unwrap();
    task.runtime
}

fn apply_command(store: &mut TaskStore, task_id: &str, kind: TaskStateCommandKind, label: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        let runtime = load_runtime(store, task_id);
        let result = store.update_task(&StoreUpdateTaskRequest {
            task_id: task_id.to_owned(),
            expected_revision: runtime.revision,
            command: StoreTaskCommand {
                kind: kind.clone(),
                item_ids: vec![],
            },
            event_id: format!("{task_id}:{label}:{}", runtime.revision),
            occurred_at: "2026-11-01T12:00:02Z".to_owned(),
        });
        if result.is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "task command {kind:?} could not win the revision race"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// The scheduler constructor under test: the plain `start` keeps real time and
/// real clock; these tests need the frozen recheck clock.
fn start_scheduler(
    database: &TempDatabase,
    worker: Arc<ScriptedWorker>,
    clock: Arc<FrozenClock>,
) -> DownloadScheduler {
    DownloadScheduler::start_with_store_and_orbit_recheck_clock(
        Arc::new(Mutex::new(TaskStore::open(database.path()).unwrap())),
        worker,
        POLL,
        clock,
    )
}

/// Seed one waiting schedule row directly through the S2 Store API, the way a
/// previous run of the scheduler would have left it.
fn seed_recheck_row(
    store: &mut TaskStore,
    task_id: &str,
    item_id: &str,
    acquisition: &str,
    attempt: u64,
    next_check_at: &str,
) {
    store
        .upsert_orbit_recheck(&StoreOrbitRecheckRequest {
            task_id: task_id.to_owned(),
            item_id: item_id.to_string(),
            scene_id: item_id.to_owned(),
            orbit_type: "POEORB".to_owned(),
            acquisition_at: acquisition.to_owned(),
            attempt,
            next_check_at: next_check_at.to_owned(),
            deadline_at: acquisition_deadline(acquisition),
            state: StoreOrbitRecheckState::Waiting,
            last_error_code: Some("ORB002".to_owned()),
            updated_at: "2026-11-01T00:00:00Z".to_owned(),
        })
        .unwrap();
}

fn acquisition_deadline(acquisition: &str) -> String {
    format_utc(parse_utc_seconds(acquisition).unwrap() + 60 * 86_400)
}

/// The scene used by most fixtures: acquired 3 days before the frozen clock,
/// so ORB002 applies and the first recheck is at acquisition + 20 d.
const SCENE: &str = "S1A_IW_SLC__1SDV_20261029T120000_20261029T120100_026700_033000_ABCD";
const SCENE_ACQUISITION: &str = "2026-10-29T12:00:00Z";
/// acquisition + 20 days.
const FIRST_CHECK: &str = "2026-11-18T12:00:00Z";
/// acquisition + 60 days (the deadline).
const DEADLINE: &str = "2026-12-28T12:00:00Z";

#[test]
fn orb002_finish_requeues_the_item_and_writes_the_first_schedule_row() {
    let database = TempDatabase::new("orb002-requeue");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    worker.fail_item(SCENE, "ORB002");
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();
    worker.wait_for_starts(1);

    // The item returns to queued and the schedule row appears with the
    // first-check time max(now, acquisition + 20 d) and the 60 d deadline.
    wait_until("the ORB002 item returns to queued", || {
        let store = TaskStore::open(database.path()).unwrap();
        let runtime = load_runtime(&store, "orbit-task");
        runtime.queued_item_ids.contains(&SCENE.to_owned())
            && runtime.active_item_ids.is_empty()
            && recheck_row(&store, "orbit-task", SCENE).is_some()
    });
    let store = TaskStore::open(database.path()).unwrap();
    let row = recheck_row(&store, "orbit-task", SCENE).unwrap();
    assert_eq!(
        row.next_check_at, FIRST_CHECK,
        "first check = acquisition + 20 d"
    );
    assert_eq!(row.deadline_at, DEADLINE, "deadline = acquisition + 60 d");
    assert_eq!(row.attempt, 1, "the first ORB002 records attempt 1");
    assert_eq!(row.state, StoreOrbitRecheckState::Waiting);
    assert_eq!(row.last_error_code.as_deref(), Some("ORB002"));
    assert_eq!(row.acquisition_at, SCENE_ACQUISITION);
    assert_eq!(row.orbit_type, "POEORB");
    // Whole seconds, UTC, no fractional part (the table orders as text).
    assert_eq!(row.next_check_at.len(), 20);
    assert!(row.updated_at.ends_with('Z') && !row.updated_at.contains('.'));

    // No second start while the row is not due: exactly one request total.
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        worker.starts_of(SCENE),
        1,
        "a not-yet-due recheck must not be dispatched again"
    );
    scheduler.shutdown();
}

#[test]
fn a_not_yet_due_item_is_not_claimed_and_does_not_delay_another_task() {
    let database = TempDatabase::new("gate-not-due");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
        // An unrelated task that must run to completion while the orbit item
        // waits: same concurrency budget shape, different task (TASK-REG-003).
        create_plain_task(&mut store, database.path(), "plain-task", &["P-1"], 1);
        // The waiting row from a previous run: due far in the future.
        seed_recheck_row(
            &mut store,
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
            FIRST_CHECK,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();

    // The unrelated task completes; the orbit item never starts.
    wait_until("the unrelated task completes", || {
        let store = TaskStore::open(database.path()).unwrap();
        load_runtime(&store, "plain-task").status == TaskLifecycleStatus::Completed
    });
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        worker.starts_of(SCENE),
        0,
        "a waiting orbit item must not occupy a worker or a capacity slot"
    );
    assert_eq!(
        worker.starts_of("P-1"),
        1,
        "the unrelated task's item must be claimed exactly once"
    );
    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        load_runtime(&store, "orbit-task").status,
        TaskLifecycleStatus::Queued,
        "the waiting item stays queued (task-level status untouched)"
    );
    scheduler.shutdown();
}

#[test]
fn a_due_item_is_claimed_once_and_a_successful_download_clears_the_row() {
    let database = TempDatabase::new("gate-due");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
        // A waiting row that is already due at the frozen clock.
        seed_recheck_row(
            &mut store,
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
            CLOCK_NOW,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();
    worker.wait_for_starts(1);

    // The worker completes this time; the row must disappear.
    wait_until("the completed item clears its schedule row", || {
        let store = TaskStore::open(database.path()).unwrap();
        let runtime = load_runtime(&store, "orbit-task");
        runtime.completed_item_ids.contains(&SCENE.to_owned())
            && runtime.active_item_ids.is_empty()
            && recheck_row(&store, "orbit-task", SCENE).is_none()
    });
    assert_eq!(
        load_runtime(&TaskStore::open(database.path()).unwrap(), "orbit-task").status,
        TaskLifecycleStatus::Completed
    );
    assert_eq!(worker.starts_of(SCENE), 1);
    scheduler.shutdown();
}

#[test]
fn repeated_orb002_backs_off_twelve_hours_then_doubles_to_a_seven_day_cap() {
    let database = TempDatabase::new("backoff");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    worker.fail_item(SCENE, "ORB002");
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();

    // The expected next-check chain. Attempt 1 uses acquisition + 20 d
    // (max(now, …)); every later attempt backs off from the moment of the
    // recheck that returned ORB002 — the clock is at that row's
    // next_check_at when the finish is processed. 12 h + 24 h + 48 h + 96 h
    // = 180 h, then the 7 d cap twice.
    let first = FrozenClock::seconds(FIRST_CHECK);
    let expected = [
        first,                             // attempt 1
        first + 12 * 3_600,                // attempt 2
        first + 36 * 3_600,                // attempt 3
        first + 84 * 3_600,                // attempt 4
        first + 180 * 3_600,               // attempt 5
        first + 180 * 3_600 + 7 * 86_400,  // attempt 6 (cap)
        first + 180 * 3_600 + 14 * 86_400, // attempt 7 (cap holds)
    ];
    for (index, expected_next) in expected.iter().enumerate() {
        let expected_next = *expected_next;
        wait_until("the item is requeued with a schedule row", || {
            let store = TaskStore::open(database.path()).unwrap();
            let runtime = load_runtime(&store, "orbit-task");
            runtime.queued_item_ids.contains(&SCENE.to_owned())
                && runtime.active_item_ids.is_empty()
                && recheck_row(&store, "orbit-task", SCENE)
                    .is_some_and(|row| row.next_check_at == format_utc(expected_next))
        });
        // Move time to the due moment and dispatch exactly one more check,
        // except after the final expected row: the item must stay gated.
        if index + 1 < expected.len() {
            clock.advance_to(&format_utc(expected_next));
            scheduler.wake();
            worker.wait_for_starts(index + 2);
        }
    }
    let store = TaskStore::open(database.path()).unwrap();
    let row = recheck_row(&store, "orbit-task", SCENE).unwrap();
    assert_eq!(row.attempt, expected.len() as u64);
    assert_eq!(row.next_check_at, format_utc(expected[expected.len() - 1]));
    scheduler.shutdown();
}

#[test]
fn crossing_the_deadline_fails_the_item_with_orb003_and_stops_requesting() {
    let database = TempDatabase::new("deadline");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    worker.fail_item(SCENE, "ORB002");
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();
    worker.wait_for_starts(1);

    // Jump past the 60 d deadline before the next check would run.
    clock.set(DEADLINE);
    scheduler.wake();
    // The still-ORB002 result now lands after the deadline: the item must
    // fail with ORB003 and the row must go exhausted.
    wait_until("the item fails terminally", || {
        let store = TaskStore::open(database.path()).unwrap();
        let runtime = load_runtime(&store, "orbit-task");
        runtime.failed_item_ids.contains(&SCENE.to_owned())
            && recheck_row(&store, "orbit-task", SCENE)
                .is_some_and(|row| row.state == StoreOrbitRecheckState::Exhausted)
    });
    let store = TaskStore::open(database.path()).unwrap();
    let runtime = load_runtime(&store, "orbit-task");
    assert_eq!(runtime.status, TaskLifecycleStatus::Failed);
    let failure = store
        .load_download_snapshot("orbit-task")
        .unwrap()
        .results
        .into_iter()
        .find(|result| result.item_id == SCENE)
        .and_then(|result| result.failure)
        .expect("the exhausted item must have a recorded ORB003 failure");
    assert_eq!(failure.code, "ORB003");
    let starts_at_deadline = worker.starts_of(SCENE);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        worker.starts_of(SCENE),
        starts_at_deadline,
        "an exhausted row must never be dispatched again"
    );
    scheduler.shutdown();
}

#[test]
fn auto_fetch_off_keeps_todays_orb002_failure_behavior() {
    let database = TempDatabase::new("auto-fetch-off");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
        store
            .save_companion_preferences(&StoreCompanionPreferences {
                source_id: "sentinel1.orbit".to_owned(),
                include_precise_orbit: true,
                auto_fetch_orbit: false,
                updated_at: Some("2026-11-01T00:00:00Z".to_owned()),
            })
            .unwrap();
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    worker.fail_item(SCENE, "ORB002");
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();
    worker.wait_for_starts(1);
    wait_until("the ORB002 item fails as before", || {
        let store = TaskStore::open(database.path()).unwrap();
        let runtime = load_runtime(&store, "orbit-task");
        runtime.failed_item_ids.contains(&SCENE.to_owned())
    });
    let store = TaskStore::open(database.path()).unwrap();
    assert!(
        recheck_row(&store, "orbit-task", SCENE).is_none(),
        "no schedule row is written when auto-fetch is off"
    );
    assert_eq!(worker.starts_of(SCENE), 1);
    scheduler.shutdown();
}

#[test]
fn cancelling_the_item_and_the_task_clears_the_schedule_rows() {
    let database = TempDatabase::new("cancel-clears");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
        // Two waiting rows on two tasks.
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task-b",
            "S1B_IW_SLC__1SDV_20261029T120000_20261029T120100_026700_033000_ABCD",
            SCENE_ACQUISITION,
            1,
        );
        seed_recheck_row(
            &mut store,
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
            FIRST_CHECK,
        );
        seed_recheck_row(
            &mut store,
            "orbit-task-b",
            "S1B_IW_SLC__1SDV_20261029T120000_20261029T120100_026700_033000_ABCD",
            SCENE_ACQUISITION,
            1,
            FIRST_CHECK,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();

    // Item-level cancel clears that item's row only.
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        let runtime = load_runtime(&store, "orbit-task");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "orbit-task".to_owned(),
                expected_revision: runtime.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::CancelItems,
                    item_ids: vec![SCENE.to_owned()],
                },
                event_id: format!("orbit-task:cancel-item:{}", runtime.revision),
                occurred_at: "2026-11-01T12:00:02Z".to_owned(),
            })
            .unwrap();
    }
    wait_until("the cancelled item's row disappears", || {
        let store = TaskStore::open(database.path()).unwrap();
        recheck_row(&store, "orbit-task", SCENE).is_none()
    });
    {
        let store = TaskStore::open(database.path()).unwrap();
        assert!(
            recheck_row(
                &store,
                "orbit-task-b",
                "S1B_IW_SLC__1SDV_20261029T120000_20261029T120100_026700_033000_ABCD"
            )
            .is_some(),
            "an item cancel must not clear another task's rows"
        );
    }

    // Whole-task cancel clears the remaining task's rows (TASK-REG-012).
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        apply_command(
            &mut store,
            "orbit-task-b",
            TaskStateCommandKind::CancelTask,
            "cancel-task",
        );
    }
    wait_until("the cancelled task's rows disappear", || {
        let store = TaskStore::open(database.path()).unwrap();
        store.load_orbit_rechecks().unwrap().is_empty()
    });
    let store = TaskStore::open(database.path()).unwrap();
    assert_eq!(
        load_runtime(&store, "orbit-task-b").status,
        TaskLifecycleStatus::Cancelled,
        "the cancelled task is terminal and must not be revived"
    );
    assert_eq!(
        worker.total_starts(),
        0,
        "waiting items must never have been dispatched"
    );
    scheduler.shutdown();
}

#[test]
fn a_restart_restores_the_gate_from_the_persisted_rows() {
    let database = TempDatabase::new("restart");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
        // A waiting row still in the future.
        seed_recheck_row(
            &mut store,
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            3,
            FIRST_CHECK,
        );
        // A waiting row already overdue.
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task-late",
            "S1C_IW_SLC__1SDV_20261029T120000_20261029T120100_026700_033000_ABCD",
            SCENE_ACQUISITION,
            1,
        );
        seed_recheck_row(
            &mut store,
            "orbit-task-late",
            "S1C_IW_SLC__1SDV_20261029T120000_20261029T120100_026700_033000_ABCD",
            SCENE_ACQUISITION,
            1,
            "2026-11-01T00:00:00Z",
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();

    // The overdue row is checked exactly once on this launch; the future row
    // is not touched.
    worker.wait_for_starts(1);
    assert_eq!(
        worker.starts_of("S1C_IW_SLC__1SDV_20261029T120000_20261029T120100_026700_033000_ABCD"),
        1
    );
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        worker.starts_of(SCENE),
        0,
        "the future row must stay gated after the restart"
    );
    // … and a second launch behaves the same way.
    scheduler.shutdown();
    let worker_two = Arc::new(ScriptedWorker::new());
    let scheduler_two = start_scheduler(&database, Arc::clone(&worker_two), Arc::clone(&clock));
    scheduler_two.wake();
    // The late item completed on the first launch (the scripted worker
    // succeeds for it), so nothing is due anymore; only the still-waiting
    // future row remains and it must not dispatch.
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        worker_two.total_starts(),
        0,
        "no recheck may run while the surviving row is in the future"
    );
    {
        let store = TaskStore::open(database.path()).unwrap();
        let row = recheck_row(&store, "orbit-task", SCENE).unwrap();
        assert_eq!(row.next_check_at, FIRST_CHECK);
        assert_eq!(row.attempt, 3);
    }
    scheduler_two.shutdown();
}

#[test]
fn orb003_finish_uses_the_existing_failure_path_without_a_schedule_row() {
    let database = TempDatabase::new("orb003-direct");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    worker.fail_item(SCENE, "ORB003");
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();
    worker.wait_for_starts(1);
    wait_until("the ORB003 item fails", || {
        let store = TaskStore::open(database.path()).unwrap();
        let runtime = load_runtime(&store, "orbit-task");
        runtime.failed_item_ids.contains(&SCENE.to_owned())
    });
    let store = TaskStore::open(database.path()).unwrap();
    assert!(
        recheck_row(&store, "orbit-task", SCENE).is_none(),
        "ORB003 is a plain failure; the scheduler writes no waiting row"
    );
    assert_eq!(worker.starts_of(SCENE), 1);
    scheduler.shutdown();
}

#[test]
fn timestamps_written_by_the_scheduler_are_whole_utc_seconds() {
    let database = TempDatabase::new("whole-seconds");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    worker.fail_item(SCENE, "ORB002");
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();
    worker.wait_for_starts(1);
    wait_until("the schedule row exists", || {
        let store = TaskStore::open(database.path()).unwrap();
        recheck_row(&store, "orbit-task", SCENE).is_some()
    });
    let store = TaskStore::open(database.path()).unwrap();
    let row = recheck_row(&store, "orbit-task", SCENE).unwrap();
    for (field, value) in [
        ("acquisition_at", &row.acquisition_at),
        ("next_check_at", &row.next_check_at),
        ("deadline_at", &row.deadline_at),
        ("updated_at", &row.updated_at),
    ] {
        assert_eq!(
            value.len(),
            20,
            "{field} must be a whole-second UTC timestamp, got {value}"
        );
        assert!(
            value.ends_with('Z') && !value.contains('.'),
            "{field}: {value}"
        );
    }
    scheduler.shutdown();
}

#[test]
fn the_production_poll_interval_constant_is_unchanged() {
    // The 100 ms production poll interval lives in the owning application
    // service; this slice must not change it. Parsing the source keeps the
    // guard in this crate's test suite without a dependency edge.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let service = Path::new(manifest).join("../../crates/insar-application-service/src/lib.rs");
    let source = fs::read_to_string(&service).unwrap();
    let line = source
        .lines()
        .find(|line| line.contains("SCHEDULER_POLL_INTERVAL"))
        .expect("the poll interval constant must exist");
    assert!(
        line.contains("Duration::from_millis(100)"),
        "the production poll interval must stay 100 ms: {line}"
    );
}

#[test]
fn item_level_pause_keeps_the_waiting_row_and_its_backoff() {
    let database = TempDatabase::new("pause-keeps-row");
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        create_orbit_task(
            &mut store,
            database.path(),
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            1,
        );
        seed_recheck_row(
            &mut store,
            "orbit-task",
            SCENE,
            SCENE_ACQUISITION,
            3,
            FIRST_CHECK,
        );
    }
    let clock = Arc::new(FrozenClock::new());
    let worker = Arc::new(ScriptedWorker::new());
    let scheduler = start_scheduler(&database, Arc::clone(&worker), Arc::clone(&clock));
    scheduler.wake();

    // Pause the single waiting item: a row-level pause is a scheduling
    // hold, not a cancellation — the schedule row must survive with its
    // attempt and next check intact, so a later resume keeps the backoff.
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        let runtime = load_runtime(&store, "orbit-task");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "orbit-task".to_owned(),
                expected_revision: runtime.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::PauseItems,
                    item_ids: vec![SCENE.to_owned()],
                },
                event_id: format!("orbit-task:pause-item:{}", runtime.revision),
                occurred_at: "2026-11-01T12:00:02Z".to_owned(),
            })
            .unwrap();
    }
    // Let several scheduler passes run over the paused item.
    thread::sleep(Duration::from_millis(250));
    {
        let store = TaskStore::open(database.path()).unwrap();
        let row = recheck_row(&store, "orbit-task", SCENE)
            .expect("a paused waiting item keeps its schedule row");
        assert_eq!(row.attempt, 3, "the backoff attempt is preserved");
        assert_eq!(row.next_check_at, FIRST_CHECK);
        assert_eq!(row.state, StoreOrbitRecheckState::Waiting);
    }

    // And once resumed, the gate still holds the item (no dispatch before
    // next_check_at): the worker must not have started once for it.
    {
        let mut store = TaskStore::open(database.path()).unwrap();
        let runtime = load_runtime(&store, "orbit-task");
        store
            .update_task(&StoreUpdateTaskRequest {
                task_id: "orbit-task".to_owned(),
                expected_revision: runtime.revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::ResumeItems,
                    item_ids: vec![SCENE.to_owned()],
                },
                event_id: format!("orbit-task:resume-item:{}", runtime.revision),
                occurred_at: "2026-11-01T12:00:03Z".to_owned(),
            })
            .unwrap();
    }
    thread::sleep(Duration::from_millis(250));
    assert_eq!(
        worker.starts_of(SCENE),
        0,
        "the resumed item is still gated by its preserved schedule row"
    );
    scheduler.shutdown();
}
