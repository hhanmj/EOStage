use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadOutcome, FrozenTaskSnapshot,
    NisarSubsetPlanV2, SnapshotItem, StoreTaskCommand, StoreUpdateTaskRequest, TaskRuntimeSnapshot,
    TaskStateCommandKind,
};
use insar_store::{
    parse_nisar_subset_plan_v2_envelope, parse_nisar_subset_prepared_intent_v2_envelope,
    SchedulerCancelledFinish, SchedulerPendingCancelledFinish,
    StoreAttachNisarSubsetExecutionPlanV2Request, StoreError,
    StoreFreezeNisarSubsetSourceSizeV2Request, StoreTaskTransferTelemetryUpdate, StoredTask,
    TaskStore,
};
#[cfg(feature = "nisar-subset-production")]
use insar_store::{
    StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request, StoredNisarSubsetExecutionAnchorV2,
};

use crate::disk_guard::{
    default_free_space_probe, stderr_disk_probe_log, CachedDiskVerdict, CoordinatorDiskGuard,
    DiskProbeLogSink, FreeSpaceProbe,
};
use crate::transfer_engine_prefs::{
    active_transfer_engine_prefs, active_transfer_engine_prefs_override, SmartConcurrencyInputs,
    SmartConcurrencyTuner,
};
use crate::TaskTransferSample;
use insar_core::TaskPauseReason;

const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(1);
const TRANSFER_CHECKPOINT_RETRY_INITIAL: Duration = Duration::from_millis(25);
const TRANSFER_CHECKPOINT_RETRY_MAX: Duration = Duration::from_millis(500);

// Orbit scheduled-fetch S3 (task contract 2026-11-01 §2.4): the first recheck
// waits for the expected publication moment, then the backoff grows from
// each finished check, all capped by the waiting deadline.
const ORBIT_RECHECK_FIRST_DELAY: u64 = 20 * 86_400;
const ORBIT_RECHECK_DEADLINE: u64 = 60 * 86_400;
const ORBIT_RECHECK_BACKOFF_INITIAL: u64 = 12 * 3_600;
const ORBIT_RECHECK_BACKOFF_MAX: u64 = 7 * 86_400;

/// The wall clock the orbit recheck schedule is computed against. Production
/// uses the system clock; tests substitute a controllable one so no test ever
/// sleeps for a real backoff duration.
pub trait OrbitRecheckClock: Send + Sync + 'static {
    fn now_utc_seconds(&self) -> u64;
}

/// The production wall clock (whole UTC seconds).
#[derive(Default)]
pub struct SystemOrbitRecheckClock;

impl OrbitRecheckClock for SystemOrbitRecheckClock {
    fn now_utc_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

/// One in-memory view of a persisted `orbit_recheck_schedule` row, kept
/// nearest-due-first by `load_orbit_rechecks`. Only `Waiting` rows gate
/// dispatch; `Exhausted`/`Cancelled` rows stay persisted for their audit
/// trail and never release work. The deadline is recomputed from the frozen
/// acquisition time (§2.4) rather than carried, so a clock change cannot
/// strand an item past its deadline without a decision.
#[derive(Clone, Debug)]
struct OrbitRecheckEntry {
    task_id: String,
    item_id: String,
    scene_id: String,
    acquisition_at: String,
    attempt: u64,
    next_check_at: u64,
    waiting: bool,
}

impl OrbitRecheckEntry {
    fn from_row(row: insar_store::StoreOrbitRecheckRow) -> Option<Self> {
        // A row the Store cannot order as text is a fact about the row, not a
        // reason to block unrelated tasks: it is skipped (and stays durable).
        let next_check_at = parse_utc_seconds(&row.next_check_at)?;
        parse_utc_seconds(&row.deadline_at)?;
        Some(Self {
            task_id: row.task_id,
            item_id: row.item_id,
            scene_id: row.scene_id,
            acquisition_at: row.acquisition_at,
            attempt: row.attempt,
            next_check_at,
            waiting: row.state == insar_store::StoreOrbitRecheckState::Waiting,
        })
    }

    fn key(&self) -> (String, String) {
        (self.task_id.clone(), self.item_id.clone())
    }
}

/// Whole-second canonical UTC timestamps (`YYYY-MM-DDTHH:MM:SSZ`), the exact
/// form `orbit_recheck_schedule` orders as text.
fn parse_utc_seconds(timestamp: &str) -> Option<u64> {
    let bytes = timestamp.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let year: i64 = timestamp.get(0..4)?.parse().ok()?;
    let month: i64 = timestamp.get(5..7)?.parse().ok()?;
    let day: i64 = timestamp.get(8..10)?.parse().ok()?;
    let hour: i64 = timestamp.get(11..13)?.parse().ok()?;
    let minute: i64 = timestamp.get(14..16)?.parse().ok()?;
    let second: i64 = timestamp.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).ok()
}

fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// Epoch seconds → `YYYY-MM-DDTHH:MM:SSZ` (reuses the file's civil calendar).
fn format_utc_seconds(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX / 2);
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3_600,
        (second_of_day % 3_600) / 60,
        second_of_day % 60
    )
}

/// The backoff of the schedule row after the `attempt`-th unfinished check
/// (§2.4): the row with attempt 2 waits 12 h, doubling to 24 h, 48 h, 96 h,
/// and every row from attempt 6 on waits the 7 d cap.
fn orbit_recheck_backoff(attempt: u64) -> u64 {
    let exponent = attempt.saturating_sub(2).min(4);
    ORBIT_RECHECK_BACKOFF_INITIAL
        .saturating_mul(1_u64 << exponent)
        .min(ORBIT_RECHECK_BACKOFF_MAX)
}

/// The schedule state the coordinator keeps in memory between Store passes.
#[derive(Default)]
struct OrbitRecheckGate {
    /// `attempt`-th check rows, keyed by (task_id, item_id).
    entries: HashMap<(String, String), OrbitRecheckEntry>,
}

impl OrbitRecheckGate {
    /// Reload every row in one query. Callers hold the Store lock; a read
    /// failure is logged and keeps the previous view (fail-open, like the
    /// cancelled-finish restore: a broken Store must not block unrelated
    /// tasks).
    fn refresh(&mut self, store: &TaskStore) {
        let rows = match store.load_orbit_rechecks() {
            Ok(rows) => rows,
            Err(error) => {
                eprintln!(
                    "orbit recheck schedule reload failed: code={}; the previous gate view is kept",
                    error.code()
                );
                return;
            }
        };
        self.entries.clear();
        for row in rows {
            if let Some(entry) = OrbitRecheckEntry::from_row(row) {
                self.entries.insert(entry.key(), entry);
            }
        }
    }

    /// A waiting row whose next check is still in the future holds its item
    /// (§3.4: per item, no capacity, no worker, no local scan).
    fn holds(&self, task_id: &str, item_id: &str, now: u64) -> bool {
        self.entries
            .get(&(task_id.to_owned(), item_id.to_owned()))
            .is_some_and(|entry| entry.waiting && entry.next_check_at > now)
    }

    /// The nearest future next-check of the whole table, for the receive
    /// timeout (§3.4: the poll interval constant stays untouched).
    fn next_due(&self, now: u64) -> Option<u64> {
        self.entries
            .values()
            .filter(|entry| entry.waiting && entry.next_check_at > now)
            .map(|entry| entry.next_check_at)
            .min()
    }
}

#[derive(Default)]
struct ProgressForwardingGate {
    last_sent_at: Option<Instant>,
    last_status: Option<insar_core::DownloadItemStatus>,
    last_rate_positive: bool,
}

impl ProgressForwardingGate {
    fn should_forward(&mut self, progress: &DownloadItemProgress, now: Instant) -> bool {
        let first = self.last_sent_at.is_none();
        let status_changed = self.last_status.as_ref() != Some(&progress.status);
        let terminal_bytes = progress
            .total_bytes
            .is_some_and(|total| progress.bytes_completed >= total);
        // Review 2026-10-17 round 2, P2-2: a transfer that stopped (a zero rate
        // after a forwarded positive rate) is a phase change the task card
        // shows at once, for example the end of a NISAR science transfer.
        let transfer_stopped = self.last_rate_positive && progress.bytes_per_second <= 0.0;
        let interval_elapsed = self
            .last_sent_at
            .is_some_and(|last| now.saturating_duration_since(last) >= PROGRESS_PERSIST_INTERVAL);
        let forward =
            first || status_changed || terminal_bytes || transfer_stopped || interval_elapsed;
        if forward {
            self.last_sent_at = Some(now);
            self.last_status = Some(progress.status.clone());
            self.last_rate_positive = progress.bytes_per_second > 0.0;
        }
        forward
    }
}

/// Review 2026-10-17 round 4 (fixed in Issue #8 slice 6, extended in slice 7):
/// when the terminal result of a finished item cannot be recorded (a
/// budget-migration refusal whose evidence transaction and unretained
/// fallback both fail, or an ordinary result whose runtime transition the
/// Store refuses, for example when it is unwritable), the item stays Active
/// in the Store and `schedule` would dispatch the same work again at once, in
/// a loop. Such an item now waits before its next dispatch, 1 s doubling to
/// 60 s, so a failing Store is retried at a bounded rate; a recorded result
/// clears the wait.
#[derive(Default)]
struct UnrecordedFinishBackoff {
    waits: HashMap<(String, String), (Instant, u32)>,
    blocked: BTreeSet<(String, String)>,
}

impl UnrecordedFinishBackoff {
    const INITIAL: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(60);

    fn delay(failures: u32) -> Duration {
        Self::INITIAL
            .saturating_mul(1_u32 << failures.min(6))
            .min(Self::MAX)
    }

    /// The number of waits is not capped: a Store that stays unwritable is
    /// retried every 60 s for as long as the coordinator runs, which costs one
    /// dispatch a minute and recovers on its own once the Store accepts the
    /// result. The first unrecorded finish of a streak is reported once on
    /// the local diagnostic stream (no Store write, which is failing).
    fn observe(&mut self, key: (String, String), recorded: bool, now: Instant) {
        if recorded {
            self.waits.remove(&key);
            return;
        }
        let failures = self.waits.get(&key).map_or(0, |(_, failures)| *failures);
        if failures == 0 {
            eprintln!(
                "download result could not be recorded; retrying with backoff: task_id={} item_id={}",
                key.0, key.1
            );
        }
        self.waits.insert(
            key,
            (now + Self::delay(failures), failures.saturating_add(1)),
        );
    }

    fn holds(&self, key: &(String, String), now: Instant) -> bool {
        self.blocked.contains(key) || self.waits.get(key).is_some_and(|(until, _)| now < *until)
    }
}

#[derive(Clone, Debug)]
pub struct DownloadWork {
    pub task: FrozenTaskSnapshot,
    pub item: SnapshotItem,
}

/// Exact durable boundaries inside the one-time NISAR budget migration where a
/// test may simulate a crash, a lifecycle race or a concurrent Store writer.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarBudgetMigrationStageForTest {
    /// Proof and target are derived; the Store transaction has not started.
    BeforeStorePrepare,
    /// Proof and pending anchor are durable; local bytes are still the predecessor.
    AfterStorePrepare,
    /// The target checkpoint is durable locally; the Store anchor is still pending.
    AfterLocalTarget,
    /// The final anchor CAS committed the target; reconciliation has not returned.
    AfterCommit,
    /// The coordinator read the runtime revision for a refused migration and
    /// has not started the Store refusal transaction yet.
    RefusalRevisionRead,
}

#[cfg(test)]
pub(crate) type NisarBudgetMigrationHookForTest =
    Arc<dyn Fn(NisarBudgetMigrationStageForTest, &WorkerControl) + Send + Sync>;

#[derive(Clone, Default)]
pub struct WorkerControl {
    paused: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    // A one-worker completion disposition, never durable lifecycle authority.
    preserve_prior_result_during_budget_migration: Arc<AtomicBool>,
    #[cfg(test)]
    budget_migration_hook: Arc<Mutex<Option<NisarBudgetMigrationHookForTest>>>,
}

impl WorkerControl {
    #[cfg(test)]
    pub(crate) fn set_budget_migration_hook_for_test(&self, hook: NisarBudgetMigrationHookForTest) {
        *self
            .budget_migration_hook
            .lock()
            .expect("test migration hook lock") = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn run_budget_migration_hook_for_test(
        &self,
        stage: NisarBudgetMigrationStageForTest,
    ) {
        // Clone first so a simulated crash cannot poison the hook slot.
        let hook = self
            .budget_migration_hook
            .lock()
            .expect("test migration hook lock")
            .clone();
        if let Some(hook) = hook {
            hook(stage, self);
        }
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn begin_nisar_budget_migration(&self) {
        self.preserve_prior_result_during_budget_migration
            .store(true, Ordering::SeqCst);
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn complete_nisar_budget_migration(&self) {
        self.preserve_prior_result_during_budget_migration
            .store(false, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn set_paused(&self, value: bool) {
        self.paused.store(value, Ordering::SeqCst);
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn set_paused_for_test(&self, value: bool) {
        self.set_paused(value);
    }

    #[cfg(test)]
    pub(crate) fn cancel_for_test(&self) {
        self.cancel();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NisarExecutionPlanReadyV2 {
    pub task_id: String,
    pub item_id: String,
    pub prepared_intent_sha256: String,
    pub proposed_plan: NisarSubsetPlanV2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NisarSourceSizeReadyV2 {
    pub task_id: String,
    pub item_id: String,
    pub provider_id: String,
    pub collection_id: String,
    pub provider_product_id: String,
    pub provider_product_key: String,
    pub asset_key: String,
    pub stable_asset_key: String,
    pub prepared_intent_sha256: String,
    pub expected_size_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum NisarExecutionPlanFreezeError {
    #[error("NISAR execution plan Store freeze failed: {0}")]
    Store(#[from] StoreError),
    #[error("NISAR execution plan coordinator is unavailable")]
    CoordinatorUnavailable,
    #[error("NISAR execution plan freeze stopped before acknowledgement")]
    ExecutionStopped,
}

#[derive(Debug, thiserror::Error)]
pub enum NisarSourceSizeFreezeError {
    #[error("NISAR source-size Store freeze failed: {0}")]
    Store(#[from] StoreError),
    #[error("NISAR source-size coordinator is unavailable")]
    CoordinatorUnavailable,
    #[error("NISAR source-size freeze stopped before acknowledgement")]
    ExecutionStopped,
}

#[cfg(feature = "nisar-subset-production")]
#[derive(Debug, thiserror::Error)]
pub(crate) enum NisarExecutionAnchorAuthorityError {
    #[error("NISAR execution anchor Store operation failed: {0}")]
    Store(#[from] StoreError),
    #[error("NISAR execution anchor coordinator is unavailable")]
    CoordinatorUnavailable,
    #[error("NISAR execution anchor operation stopped with the worker lifecycle")]
    ExecutionStopped,
}

/// Downloader D3: why a local reuse Store operation did not complete.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LocalReuseAuthorityError {
    #[error("local reuse Store operation failed: {0}")]
    Store(#[from] StoreError),
    #[error("local reuse coordinator is unavailable")]
    CoordinatorUnavailable,
    #[error("local reuse operation stopped with the worker lifecycle")]
    ExecutionStopped,
}

/// Downloader D3: the ledger row of a planned destination and the item's
/// earlier local reuse decision.
pub(crate) type LocalReuseLoaded = (
    Option<insar_store::StoreLandedArtifact>,
    Option<insar_store::StoreTaskItemLocalReuse>,
);

/// Downloader D3: local reuse facts the coordinator writes, in this order:
/// ledger invalidation, ledger row, decision removal, decision row.
#[derive(Debug, Clone, Default)]
pub(crate) struct LocalReuseRecord {
    pub(crate) invalidate: Option<(String, String)>,
    pub(crate) landed: Option<insar_store::StoreLandedArtifact>,
    pub(crate) clear_decision: Option<(String, String)>,
    pub(crate) decision: Option<insar_store::StoreTaskItemLocalReuse>,
}

fn apply_local_reuse_record(
    store: &mut TaskStore,
    record: &LocalReuseRecord,
) -> Result<(), LocalReuseAuthorityError> {
    if let Some((path_key, reason)) = &record.invalidate {
        store.invalidate_landed_artifact(path_key, reason)?;
    }
    if let Some(landed) = &record.landed {
        store.record_landed_artifact(landed)?;
    }
    if let Some((task_id, item_id)) = &record.clear_decision {
        store.clear_task_item_local_reuse(task_id, item_id)?;
    }
    if let Some(decision) = &record.decision {
        store.record_task_item_local_reuse(decision)?;
    }
    Ok(())
}

fn receive_local_reuse<T>(
    receiver: &Receiver<Result<T, LocalReuseAuthorityError>>,
    stopped: &dyn Fn() -> bool,
) -> Result<T, LocalReuseAuthorityError> {
    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => return result,
            Err(RecvTimeoutError::Timeout) if stopped() => {
                return Err(LocalReuseAuthorityError::ExecutionStopped)
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(LocalReuseAuthorityError::CoordinatorUnavailable)
            }
        }
    }
}

#[derive(Clone)]
pub struct NisarExecutionPlanFreezeAuthority {
    sender: Sender<Message>,
    coordinator_stopping: Arc<AtomicBool>,
}

impl NisarExecutionPlanFreezeAuthority {
    /// Downloader D3: reads the ledger row of a planned destination and the
    /// item's earlier local reuse decision; a pause or cancellation stops it.
    pub(crate) fn load_local_reuse(
        &self,
        path_key: &str,
        task_id: &str,
        item_id: &str,
        control: &WorkerControl,
    ) -> Result<LocalReuseLoaded, LocalReuseAuthorityError> {
        let stopped = || {
            self.coordinator_stopping.load(Ordering::SeqCst)
                || control.is_cancelled()
                || control.is_paused()
        };
        if stopped() {
            return Err(LocalReuseAuthorityError::ExecutionStopped);
        }
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::LocalReuseLoad {
                path_key: path_key.to_owned(),
                task_id: task_id.to_owned(),
                item_id: item_id.to_owned(),
                response: response_sender,
            })
            .map_err(|_| LocalReuseAuthorityError::CoordinatorUnavailable)?;
        receive_local_reuse(&response_receiver, &stopped)
    }

    /// Downloader D3: writes local reuse facts. A pause or cancellation does
    /// not stop the write: the reuse or rename it describes already happened.
    pub(crate) fn record_local_reuse(
        &self,
        record: LocalReuseRecord,
    ) -> Result<(), LocalReuseAuthorityError> {
        let stopped = || self.coordinator_stopping.load(Ordering::SeqCst);
        if stopped() {
            return Err(LocalReuseAuthorityError::ExecutionStopped);
        }
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::LocalReuseRecord {
                record: Box::new(record),
                response: response_sender,
            })
            .map_err(|_| LocalReuseAuthorityError::CoordinatorUnavailable)?;
        receive_local_reuse(&response_receiver, &stopped)
    }
}

impl NisarExecutionPlanFreezeAuthority {
    pub fn freeze(
        &self,
        request: NisarExecutionPlanReadyV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetPlanV2, NisarExecutionPlanFreezeError> {
        if control.is_cancelled()
            || control.is_paused()
            || self.coordinator_stopping.load(Ordering::SeqCst)
        {
            return Err(NisarExecutionPlanFreezeError::ExecutionStopped);
        }
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::NisarExecutionPlanReady {
                request: Box::new(request),
                response: response_sender,
            })
            .map_err(|_| NisarExecutionPlanFreezeError::CoordinatorUnavailable)?;
        loop {
            match response_receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(_result)
                    if control.is_cancelled()
                        || control.is_paused()
                        || self.coordinator_stopping.load(Ordering::SeqCst) =>
                {
                    return Err(NisarExecutionPlanFreezeError::ExecutionStopped);
                }
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout)
                    if control.is_cancelled()
                        || control.is_paused()
                        || self.coordinator_stopping.load(Ordering::SeqCst) =>
                {
                    return Err(NisarExecutionPlanFreezeError::ExecutionStopped);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(NisarExecutionPlanFreezeError::CoordinatorUnavailable);
                }
            }
        }
    }

    pub(crate) fn freeze_source_size(
        &self,
        request: NisarSourceSizeReadyV2,
        control: &WorkerControl,
    ) -> Result<u64, NisarSourceSizeFreezeError> {
        if control.is_cancelled()
            || control.is_paused()
            || self.coordinator_stopping.load(Ordering::SeqCst)
        {
            return Err(NisarSourceSizeFreezeError::ExecutionStopped);
        }
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::NisarSourceSizeReady {
                request: Box::new(request),
                response: response_sender,
            })
            .map_err(|_| NisarSourceSizeFreezeError::CoordinatorUnavailable)?;
        loop {
            match response_receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(_result)
                    if control.is_cancelled()
                        || control.is_paused()
                        || self.coordinator_stopping.load(Ordering::SeqCst) =>
                {
                    return Err(NisarSourceSizeFreezeError::ExecutionStopped);
                }
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout)
                    if control.is_cancelled()
                        || control.is_paused()
                        || self.coordinator_stopping.load(Ordering::SeqCst) =>
                {
                    return Err(NisarSourceSizeFreezeError::ExecutionStopped);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(NisarSourceSizeFreezeError::CoordinatorUnavailable)
                }
            }
        }
    }

    #[cfg(feature = "nisar-subset-production")]
    fn receive_execution_anchor_result<T>(
        &self,
        response_receiver: Receiver<Result<T, NisarExecutionAnchorAuthorityError>>,
        control: &WorkerControl,
        allow_worker_stop: bool,
    ) -> Result<T, NisarExecutionAnchorAuthorityError> {
        loop {
            match response_receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout)
                    if self.coordinator_stopping.load(Ordering::SeqCst)
                        || (!allow_worker_stop
                            && (control.is_cancelled() || control.is_paused())) =>
                {
                    return Err(NisarExecutionAnchorAuthorityError::ExecutionStopped);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable)
                }
            }
        }
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn load_execution_anchor_v2(
        &self,
        task_id: &str,
        item_id: &str,
        anchor_kind: &str,
        control: &WorkerControl,
    ) -> Result<Option<StoredNisarSubsetExecutionAnchorV2>, NisarExecutionAnchorAuthorityError>
    {
        // Science checkpoints must persist Interrupted after a worker stop.
        // Source access, plan freezing, and writer anchors remain stopped.
        let allow_worker_stop =
            anchor_kind == insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND;
        if self.coordinator_stopping.load(Ordering::SeqCst)
            || (!allow_worker_stop && (control.is_cancelled() || control.is_paused()))
        {
            return Err(NisarExecutionAnchorAuthorityError::ExecutionStopped);
        }
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::NisarExecutionAnchorLoad {
                task_id: task_id.to_owned(),
                item_id: item_id.to_owned(),
                anchor_kind: anchor_kind.to_owned(),
                response: response_sender,
            })
            .map_err(|_| NisarExecutionAnchorAuthorityError::CoordinatorUnavailable)?;
        self.receive_execution_anchor_result(response_receiver, control, allow_worker_stop)
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn load_archived_recovery_v2(
        &self,
        task_id: &str,
        item_id: &str,
        control: &WorkerControl,
    ) -> Result<
        Option<insar_store::StoredNisarArchivedRecoveryV2>,
        NisarExecutionAnchorAuthorityError,
    > {
        if self.coordinator_stopping.load(Ordering::SeqCst)
            || control.is_cancelled()
            || control.is_paused()
        {
            return Err(NisarExecutionAnchorAuthorityError::ExecutionStopped);
        }
        let (response, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::NisarArchivedRecoveryLoad {
                task_id: task_id.to_owned(),
                item_id: item_id.to_owned(),
                response,
            })
            .map_err(|_| NisarExecutionAnchorAuthorityError::CoordinatorUnavailable)?;
        self.receive_execution_anchor_result(receiver, control, false)
    }

    /// Records an accepted workspace rebinding as one Store task event
    /// (design 2026-10-17 §4); returns whether a new event was written.
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn record_workspace_rebinding_v2(
        &self,
        request: insar_store::StoreNisarWorkspaceRebindingEventV2Request,
        control: &WorkerControl,
    ) -> Result<bool, NisarExecutionAnchorAuthorityError> {
        if self.coordinator_stopping.load(Ordering::SeqCst)
            || control.is_cancelled()
            || control.is_paused()
        {
            return Err(NisarExecutionAnchorAuthorityError::ExecutionStopped);
        }
        let (response, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::NisarWorkspaceRebindingObserved {
                request: Box::new(request),
                response,
            })
            .map_err(|_| NisarExecutionAnchorAuthorityError::CoordinatorUnavailable)?;
        self.receive_execution_anchor_result(receiver, control, false)
    }

    /// Records a NISAR V2 writer's long output wait as one Store task event
    /// (downloader enhancement D1b); returns whether a new event was written.
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn record_output_wait_v2(
        &self,
        request: insar_store::StoreNisarOutputWaitEventV2Request,
        control: &WorkerControl,
    ) -> Result<bool, NisarExecutionAnchorAuthorityError> {
        if self.coordinator_stopping.load(Ordering::SeqCst)
            || control.is_cancelled()
            || control.is_paused()
        {
            return Err(NisarExecutionAnchorAuthorityError::ExecutionStopped);
        }
        let (response, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::NisarOutputWaitObserved {
                request: Box::new(request),
                response,
            })
            .map_err(|_| NisarExecutionAnchorAuthorityError::CoordinatorUnavailable)?;
        self.receive_execution_anchor_result(receiver, control, false)
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn compare_and_swap_execution_anchor_v2(
        &self,
        request: StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request,
        control: &WorkerControl,
    ) -> Result<bool, NisarExecutionAnchorAuthorityError> {
        let allow_worker_stop =
            request.anchor_kind == insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND;
        if self.coordinator_stopping.load(Ordering::SeqCst)
            || (!allow_worker_stop && (control.is_cancelled() || control.is_paused()))
        {
            return Err(NisarExecutionAnchorAuthorityError::ExecutionStopped);
        }
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Message::NisarExecutionAnchorCompareAndSwap {
                request: Box::new(request),
                response: response_sender,
            })
            .map_err(|_| NisarExecutionAnchorAuthorityError::CoordinatorUnavailable)?;
        self.receive_execution_anchor_result(response_receiver, control, allow_worker_stop)
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn science_checkpoint_anchor_v2(
        &self,
        task_id: &str,
        item_id: &str,
        control: &WorkerControl,
    ) -> Arc<dyn crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorV2> {
        Arc::new(StoreBackedScienceCheckpointAnchorV2 {
            authority: self.clone(),
            task_id: task_id.to_owned(),
            item_id: item_id.to_owned(),
            control: control.clone(),
        })
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn writer_recovery_anchor_v2(
        &self,
        task_id: &str,
        item_id: &str,
        control: &WorkerControl,
    ) -> Arc<dyn crate::nisar_subset_writer::NisarSubsetWriterRecoveryAnchorV2> {
        Arc::new(StoreBackedWriterRecoveryAnchorV2 {
            authority: self.clone(),
            task_id: task_id.to_owned(),
            item_id: item_id.to_owned(),
            control: control.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn disconnected_for_test() -> Self {
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        Self {
            sender,
            coordinator_stopping: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[cfg(feature = "nisar-subset-production")]
struct StoreBackedScienceCheckpointAnchorV2 {
    authority: NisarExecutionPlanFreezeAuthority,
    task_id: String,
    item_id: String,
    control: WorkerControl,
}

#[cfg(feature = "nisar-subset-production")]
impl crate::nisar_science_transfer_v2::science_checkpoint_anchor_sealed::Sealed
    for StoreBackedScienceCheckpointAnchorV2
{
}

#[cfg(feature = "nisar-subset-production")]
impl crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorV2
    for StoreBackedScienceCheckpointAnchorV2
{
    fn load_budget_upgrade_proof(
        &self,
    ) -> Result<
        Option<insar_store::StoredNisarBudgetUpgradeProofV2>,
        crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2,
    > {
        let (response, receiver) = mpsc::sync_channel(1);
        self.authority
            .sender
            .send(Message::NisarBudgetUpgradeProofLoad {
                task_id: self.task_id.clone(),
                item_id: self.item_id.clone(),
                response,
            })
            .map_err(|_| {
                crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::RecoveryInvalid
            })?;
        self.authority
            .receive_execution_anchor_result(receiver, &self.control, true)
            .map_err(|error| science_anchor_authority_error(error, &self.control))
    }

    fn load_budget_upgrade_context(
        &self,
    ) -> Result<
        insar_store::StoreNisarBudgetUpgradeContextV2,
        crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2,
    > {
        let (response, receiver) = mpsc::sync_channel(1);
        self.authority
            .sender
            .send(Message::NisarBudgetUpgradeContextLoad {
                task_id: self.task_id.clone(),
                item_id: self.item_id.clone(),
                response,
            })
            .map_err(|_| {
                crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::RecoveryInvalid
            })?;
        self.authority
            .receive_execution_anchor_result(receiver, &self.control, false)
            .map_err(|error| science_anchor_authority_error(error, &self.control))
    }

    fn prepare_budget_upgrade(
        &self,
        proof: &insar_store::StoredNisarBudgetUpgradeProofV2,
        replacement: &crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2,
    ) -> Result<bool, crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2> {
        if self.control.is_cancelled() || self.control.is_paused() {
            return Err(science_anchor_authority_error(
                NisarExecutionAnchorAuthorityError::ExecutionStopped,
                &self.control,
            ));
        }
        if proof.context().task_id != self.task_id || proof.context().item_id != self.item_id {
            return Err(crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let next = StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
            task_id: self.task_id.clone(),
            item_id: self.item_id.clone(),
            anchor_kind: insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND.to_owned(),
            lineage_key_sha256: replacement.lineage_key_sha256().to_owned(),
            expected_record_sha256: Some(proof.context().science_anchor.record_sha256.clone()),
            record_sha256: replacement.record_sha256().to_owned(),
            canonical_json: replacement.canonical_json()?,
        };
        let (response, receiver) = mpsc::sync_channel(1);
        self.authority
            .sender
            .send(Message::NisarBudgetUpgradePrepare {
                proof: Box::new(proof.clone()),
                next: Box::new(next),
                response,
            })
            .map_err(|_| {
                crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::RecoveryInvalid
            })?;
        self.authority
            .receive_execution_anchor_result(receiver, &self.control, true)
            .map_err(|error| science_anchor_authority_error(error, &self.control))
    }

    fn load(
        &self,
        lineage_key_sha256: &str,
    ) -> Result<
        Option<crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2>,
        crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2,
    > {
        let stored = self
            .authority
            .load_execution_anchor_v2(
                &self.task_id,
                &self.item_id,
                insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
                &self.control,
            )
            .map_err(|error| science_anchor_authority_error(error, &self.control))?;
        stored
            .map(|stored| {
                let record = crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2::from_canonical_json(
                    &stored.canonical_json,
                )?;
                if stored.lineage_key_sha256 != lineage_key_sha256
                    || record.lineage_key_sha256() != lineage_key_sha256
                    || record.record_sha256() != stored.record_sha256
                {
                    return Err(crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
                }
                Ok(record)
            })
            .transpose()
    }

    fn compare_and_swap(
        &self,
        lineage_key_sha256: &str,
        expected: Option<
            &crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2,
        >,
        replacement: &crate::nisar_science_transfer_v2::NisarSubsetScienceCheckpointAnchorRecordV2,
    ) -> Result<bool, crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2> {
        if replacement.lineage_key_sha256() != lineage_key_sha256
            || expected.is_some_and(|record| record.lineage_key_sha256() != lineage_key_sha256)
        {
            return Err(crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        self.authority
            .compare_and_swap_execution_anchor_v2(
                StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
                    task_id: self.task_id.clone(),
                    item_id: self.item_id.clone(),
                    anchor_kind: insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
                        .to_owned(),
                    lineage_key_sha256: lineage_key_sha256.to_owned(),
                    expected_record_sha256: expected
                        .map(|record| record.record_sha256().to_owned()),
                    record_sha256: replacement.record_sha256().to_owned(),
                    canonical_json: replacement.canonical_json()?,
                },
                &self.control,
            )
            .map_err(|error| science_anchor_authority_error(error, &self.control))
    }
}

/// Review 2026-10-17 round 2, P1-2: only a revision conflict, a busy or
/// locked database and an unavailable coordinator are transient
/// (`Persistence`); a stopped coordinator or worker is a lifecycle stop. Every
/// other Store refusal (an unsupported proof version, a corrupt or invalid
/// row, a missing task or item, another database refusal) keeps the Store's own
/// code and is terminal. Review 2026-10-17 round 3, P2-2: a full disk, an I/O
/// error, a database file SQLite cannot open and a file-system error are local
/// storage failures that clear once disk space is freed or the disk repaired:
/// transient (`LocalStorageFailure`) with a local diagnostic, never terminal.
#[cfg(feature = "nisar-subset-production")]
pub(crate) fn science_anchor_authority_error(
    error: NisarExecutionAnchorAuthorityError,
    control: &WorkerControl,
) -> crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 {
    use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;
    match error {
        NisarExecutionAnchorAuthorityError::Store(
            StoreError::NisarSubsetExecutionAnchorInvalid
            | StoreError::NisarSubsetExecutionAnchorLineageConflict,
        ) => ScienceError::RecoveryInvalid,
        NisarExecutionAnchorAuthorityError::ExecutionStopped if control.is_cancelled() => {
            ScienceError::Cancelled
        }
        NisarExecutionAnchorAuthorityError::ExecutionStopped => ScienceError::Paused,
        NisarExecutionAnchorAuthorityError::CoordinatorUnavailable
        | NisarExecutionAnchorAuthorityError::Store(StoreError::RevisionConflict) => {
            ScienceError::Persistence
        }
        NisarExecutionAnchorAuthorityError::Store(error) if error.is_database_busy() => {
            ScienceError::Persistence
        }
        NisarExecutionAnchorAuthorityError::Store(error) => match error.local_storage_failure() {
            Some(cause) => ScienceError::LocalStorageFailure {
                store_code: error.code(),
                cause,
            },
            None => ScienceError::StoreRecordRejected {
                store_code: error.code(),
            },
        },
    }
}

#[cfg(feature = "nisar-subset-production")]
struct StoreBackedWriterRecoveryAnchorV2 {
    authority: NisarExecutionPlanFreezeAuthority,
    task_id: String,
    item_id: String,
    control: WorkerControl,
}

#[cfg(feature = "nisar-subset-production")]
impl crate::nisar_subset_writer::writer_recovery_anchor_sealed::Sealed
    for StoreBackedWriterRecoveryAnchorV2
{
}

#[cfg(feature = "nisar-subset-production")]
impl crate::nisar_subset_writer::NisarSubsetWriterRecoveryAnchorV2
    for StoreBackedWriterRecoveryAnchorV2
{
    fn load(
        &self,
        lineage_key_sha256: &str,
    ) -> Result<
        Option<crate::nisar_subset_writer::NisarSubsetWriterRecoveryAnchorRecordV2>,
        crate::nisar_subset_writer::NisarSubsetWriteError,
    > {
        let stored = self
            .authority
            .load_execution_anchor_v2(
                &self.task_id,
                &self.item_id,
                insar_store::NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
                &self.control,
            )
            .map_err(|error| match error {
                NisarExecutionAnchorAuthorityError::ExecutionStopped => {
                    crate::nisar_subset_writer::NisarSubsetWriteError::Cancelled
                }
                _ => crate::nisar_subset_writer::NisarSubsetWriteError::Persistence,
            })?;
        stored
            .map(|stored| {
                let record = crate::nisar_subset_writer::NisarSubsetWriterRecoveryAnchorRecordV2::from_canonical_json(
                    &stored.canonical_json,
                )?;
                if stored.lineage_key_sha256 != lineage_key_sha256
                    || record.lineage_key_sha256() != lineage_key_sha256
                    || record.record_sha256() != stored.record_sha256
                {
                    return Err(crate::nisar_subset_writer::NisarSubsetWriteError::RecoveryInvalid);
                }
                Ok(record)
            })
            .transpose()
    }

    fn compare_and_swap(
        &self,
        lineage_key_sha256: &str,
        expected: Option<&crate::nisar_subset_writer::NisarSubsetWriterRecoveryAnchorRecordV2>,
        replacement: &crate::nisar_subset_writer::NisarSubsetWriterRecoveryAnchorRecordV2,
    ) -> Result<bool, crate::nisar_subset_writer::NisarSubsetWriteError> {
        if replacement.lineage_key_sha256() != lineage_key_sha256
            || expected.is_some_and(|record| record.lineage_key_sha256() != lineage_key_sha256)
        {
            return Err(crate::nisar_subset_writer::NisarSubsetWriteError::RecoveryInvalid);
        }
        self.authority
            .compare_and_swap_execution_anchor_v2(
                StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request {
                    task_id: self.task_id.clone(),
                    item_id: self.item_id.clone(),
                    anchor_kind: insar_store::NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND
                        .to_owned(),
                    lineage_key_sha256: lineage_key_sha256.to_owned(),
                    expected_record_sha256: expected
                        .map(|record| record.record_sha256().to_owned()),
                    record_sha256: replacement.record_sha256().to_owned(),
                    canonical_json: replacement.canonical_json()?,
                },
                &self.control,
            )
            .map_err(|error| match error {
                NisarExecutionAnchorAuthorityError::Store(
                    StoreError::NisarSubsetExecutionAnchorInvalid
                    | StoreError::NisarSubsetExecutionAnchorLineageConflict,
                ) => crate::nisar_subset_writer::NisarSubsetWriteError::RecoveryInvalid,
                NisarExecutionAnchorAuthorityError::ExecutionStopped => {
                    crate::nisar_subset_writer::NisarSubsetWriteError::Cancelled
                }
                NisarExecutionAnchorAuthorityError::Store(_)
                | NisarExecutionAnchorAuthorityError::CoordinatorUnavailable => {
                    crate::nisar_subset_writer::NisarSubsetWriteError::Persistence
                }
            })
    }
}

/// Downloader D3-2: how much local work one pre-scan call may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalScanPhase {
    /// stat, ledger and bounded format checks; no digest.
    Cheap,
    /// May compute a digest (checksum or ledger re-hash).
    Digest,
}

/// Downloader D3-2: what the local pre-scan found for one queued item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalScanVerdict {
    /// The worker decides: no local plan, nothing there, a conflict to
    /// rename, a busy destination, an asset with a validator...
    Release,
    /// A digest is needed to decide.
    NeedsDigest,
    /// Reusable now; the coordinator commits it in a batch.
    Reuse(Box<insar_store::StoreLocalReuseBatchItem>),
    /// A pause or cancellation stopped the scan.
    Stopped,
}

pub trait DownloadWorker: Send + Sync + 'static {
    /// Downloader D3-2: decides a queued item locally, without credentials,
    /// resolution, network admission, renames or Store writes. The default
    /// leaves every item to the worker.
    fn scan_local(
        &self,
        work: &DownloadWork,
        phase: LocalScanPhase,
        control: &WorkerControl,
        store: Option<&NisarExecutionPlanFreezeAuthority>,
    ) -> LocalScanVerdict {
        let _ = (work, phase, control, store);
        LocalScanVerdict::Release
    }

    /// Downloader D3-2: whether the coordinator pre-scans this worker's
    /// queued items. The default keeps every queued item claimable at once.
    fn supports_local_scan(&self) -> bool {
        false
    }

    /// Publish binding-specific local readiness to the owning application
    /// service. The default keeps existing fixture workers source-compatible;
    /// an empty result means that this worker owns no declared capability.
    fn capability_runtime_observations(&self) -> Vec<(String, insar_core::RuntimeReadinessState)> {
        Vec::new()
    }

    /// Fixed task-level telemetry sampled by the one scheduler coordinator.
    /// Fixture and non-network workers own no transfer clock by default.
    fn task_transfer_sample(&self, _task_id: &str) -> Option<TaskTransferSample> {
        None
    }

    /// Recent stall reconnect events recorded for `task_id`, used by smart concurrency throttling.
    fn task_recent_stall_count(&self, _task_id: &str) -> u32 {
        0
    }

    /// Execute with access to the Scheduler-owned NISAR V2 plan-freeze handshake.
    /// Existing workers remain unchanged until they explicitly enter V2 planning.
    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        plan_freeze: &NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        let _ = plan_freeze;
        self.execute(work, control, progress)
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult;
}

/// The one durable Store lease used by a downloads-enabled desktop runtime.
/// The scheduler serializes its Store reads/writes through this mutex rather than
/// reopening a competing SQLite connection for the same runtime.
pub type SharedTaskStore = Arc<Mutex<TaskStore>>;

enum Message {
    Wake,
    Progress(DownloadItemProgress),
    /// Downloader D3-2: a pre-scan job reports decisions and progress.
    LocalScanReport(Box<crate::local_scan::LocalScanReport>),
    /// Downloader D3: a worker reads local reuse facts.
    LocalReuseLoad {
        path_key: String,
        task_id: String,
        item_id: String,
        response: mpsc::SyncSender<Result<LocalReuseLoaded, LocalReuseAuthorityError>>,
    },
    /// Downloader D3: a worker records local reuse facts.
    LocalReuseRecord {
        record: Box<LocalReuseRecord>,
        response: mpsc::SyncSender<Result<(), LocalReuseAuthorityError>>,
    },
    #[cfg(feature = "nisar-subset-production")]
    NisarBudgetUpgradeProofLoad {
        task_id: String,
        item_id: String,
        response: mpsc::SyncSender<
            Result<
                Option<insar_store::StoredNisarBudgetUpgradeProofV2>,
                NisarExecutionAnchorAuthorityError,
            >,
        >,
    },
    #[cfg(feature = "nisar-subset-production")]
    NisarBudgetUpgradeContextLoad {
        task_id: String,
        item_id: String,
        response: mpsc::SyncSender<
            Result<
                insar_store::StoreNisarBudgetUpgradeContextV2,
                NisarExecutionAnchorAuthorityError,
            >,
        >,
    },
    #[cfg(feature = "nisar-subset-production")]
    NisarBudgetUpgradePrepare {
        proof: Box<insar_store::StoredNisarBudgetUpgradeProofV2>,
        next: Box<StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request>,
        response: mpsc::SyncSender<Result<bool, NisarExecutionAnchorAuthorityError>>,
    },
    NisarExecutionPlanReady {
        request: Box<NisarExecutionPlanReadyV2>,
        response: mpsc::SyncSender<Result<NisarSubsetPlanV2, NisarExecutionPlanFreezeError>>,
    },
    NisarSourceSizeReady {
        request: Box<NisarSourceSizeReadyV2>,
        response: mpsc::SyncSender<Result<u64, NisarSourceSizeFreezeError>>,
    },
    #[cfg(feature = "nisar-subset-production")]
    NisarWorkspaceRebindingObserved {
        request: Box<insar_store::StoreNisarWorkspaceRebindingEventV2Request>,
        response: mpsc::SyncSender<Result<bool, NisarExecutionAnchorAuthorityError>>,
    },
    #[cfg(feature = "nisar-subset-production")]
    NisarOutputWaitObserved {
        request: Box<insar_store::StoreNisarOutputWaitEventV2Request>,
        response: mpsc::SyncSender<Result<bool, NisarExecutionAnchorAuthorityError>>,
    },
    #[cfg(feature = "nisar-subset-production")]
    NisarArchivedRecoveryLoad {
        task_id: String,
        item_id: String,
        response: mpsc::SyncSender<
            Result<
                Option<insar_store::StoredNisarArchivedRecoveryV2>,
                NisarExecutionAnchorAuthorityError,
            >,
        >,
    },
    #[cfg(feature = "nisar-subset-production")]
    NisarExecutionAnchorLoad {
        task_id: String,
        item_id: String,
        anchor_kind: String,
        response: mpsc::SyncSender<
            Result<Option<StoredNisarSubsetExecutionAnchorV2>, NisarExecutionAnchorAuthorityError>,
        >,
    },
    #[cfg(feature = "nisar-subset-production")]
    NisarExecutionAnchorCompareAndSwap {
        request: Box<StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request>,
        response: mpsc::SyncSender<Result<bool, NisarExecutionAnchorAuthorityError>>,
    },
    Finished(DownloadItemResult),
    Shutdown,
}

struct PendingCancelledFinish {
    result: DownloadItemResult,
    started_revision: u64,
    preserve: bool,
    retry_at: Instant,
    failures: u32,
    last_diagnostic: Option<Instant>,
    /// The Store holds this result, so a restart picks it up again.
    persisted: bool,
    /// Loaded from the Store at startup: its worker belonged to an earlier
    /// process and can no longer write.
    restored: bool,
}

impl PendingCancelledFinish {
    fn new(result: DownloadItemResult, started_revision: u64, preserve: bool) -> Self {
        Self {
            result,
            started_revision,
            preserve,
            retry_at: Instant::now(),
            failures: 0,
            last_diagnostic: None,
            persisted: false,
            restored: false,
        }
    }
}

/// Picks up the cancelled results an earlier run could not settle. Their
/// items stay blocked until each one is settled or given up.
fn restore_pending_cancelled_finishes(
    store: &Mutex<TaskStore>,
    pending_cancelled: &mut HashMap<(String, String), PendingCancelledFinish>,
    finish_backoff: &mut UnrecordedFinishBackoff,
) {
    let loaded = match store.lock() {
        Ok(store) => store.load_scheduler_pending_cancelled_finishes(),
        Err(_) => {
            eprintln!(
                "download cancelled finish restore skipped: the Store lock is unavailable; unsettled cancelled results from the previous run were not loaded"
            );
            return;
        }
    };
    let rows = match loaded {
        Ok(rows) => rows,
        Err(error) => {
            eprintln!(
                "download cancelled finish restore failed: code={}; unsettled cancelled results from the previous run were not loaded",
                error.code()
            );
            return;
        }
    };
    for row in rows {
        let key = (row.result.task_id.clone(), row.result.item_id.clone());
        let mut pending =
            PendingCancelledFinish::new(row.result, row.started_revision, row.preserve_result);
        pending.persisted = true;
        pending.restored = true;
        finish_backoff.blocked.insert(key.clone());
        pending_cancelled.insert(key, pending);
    }
}

/// A Store that could not be read or written may recover; anything else is
/// a fact about the evidence that retrying will not change.
fn store_error_is_transient(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::Database(_) | StoreError::Filesystem(_) | StoreError::RevisionConflict
    )
}

struct RunningWorker {
    control: WorkerControl,
    handle: JoinHandle<()>,
    snapshot: FrozenTaskSnapshot,
    runtime_revision: u64,
}

#[cfg(test)]
pub(crate) struct NisarPlanFreezeHandlerGate {
    entered: AtomicBool,
    released: AtomicBool,
}

#[cfg(test)]
impl NisarPlanFreezeHandlerGate {
    pub(crate) fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.entered.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "scheduler freeze handler did not receive the queued request"
            );
            thread::yield_now();
        }
    }

    pub(crate) fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }

    fn enter_and_wait(&self) {
        self.entered.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.released.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "scheduler freeze handler gate was not released"
            );
            thread::yield_now();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferCheckpointDiagnosticStatus {
    Failed,
    Recovered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TransferCheckpointDiagnostic {
    checkpoint_id: u64,
    status: TransferCheckpointDiagnosticStatus,
    error_code: &'static str,
    terminal: bool,
    retry_count: u32,
}

trait TransferCheckpointDiagnosticSink: Send + Sync + 'static {
    fn record(&self, diagnostic: TransferCheckpointDiagnostic);
}

struct StderrTransferCheckpointDiagnosticSink;

impl TransferCheckpointDiagnosticSink for StderrTransferCheckpointDiagnosticSink {
    fn record(&self, diagnostic: TransferCheckpointDiagnostic) {
        match diagnostic.status {
            TransferCheckpointDiagnosticStatus::Failed => eprintln!(
                "download transfer checkpoint failed: checkpoint_id={} error_code={} terminal={} retry_count={}",
                diagnostic.checkpoint_id,
                diagnostic.error_code,
                diagnostic.terminal,
                diagnostic.retry_count,
            ),
            TransferCheckpointDiagnosticStatus::Recovered => eprintln!(
                "download transfer checkpoint recovered: checkpoint_id={} prior_error_code={} terminal={} retry_count={}",
                diagnostic.checkpoint_id,
                diagnostic.error_code,
                diagnostic.terminal,
                diagnostic.retry_count,
            ),
        }
    }
}

struct PendingTransferCheckpoint {
    checkpoint_id: u64,
    terminal: bool,
    retry_count: u32,
    next_retry_at: Instant,
    last_error_code: Option<&'static str>,
}

struct TransferCheckpointSampleState {
    lifetime_baseline: (u64, u64),
    last_successful_raw: Option<(u64, u64)>,
}

#[derive(Default)]
struct TransferCheckpointTracker {
    next_checkpoint_id: u64,
    pending: HashMap<String, PendingTransferCheckpoint>,
}

impl TransferCheckpointTracker {
    fn request(&mut self, task_id: &str, terminal: bool, now: Instant) {
        if let Some(pending) = self.pending.get_mut(task_id) {
            pending.terminal |= terminal;
            if pending.retry_count == 0 {
                pending.next_retry_at = now;
            }
            return;
        }
        self.next_checkpoint_id = self.next_checkpoint_id.saturating_add(1);
        self.pending.insert(
            task_id.to_owned(),
            PendingTransferCheckpoint {
                checkpoint_id: self.next_checkpoint_id,
                terminal,
                retry_count: 0,
                next_retry_at: now,
                last_error_code: None,
            },
        );
    }

    fn due_task_ids(&self, now: Instant) -> Vec<String> {
        self.pending
            .iter()
            .filter(|(_, pending)| pending.next_retry_at <= now)
            .map(|(task_id, _)| task_id.clone())
            .collect()
    }

    fn observe_result(
        &mut self,
        task_id: &str,
        result: Result<(), StoreError>,
        now: Instant,
        diagnostics: &dyn TransferCheckpointDiagnosticSink,
    ) {
        match result {
            Ok(()) => {
                let Some(recovered) = self.pending.remove(task_id) else {
                    return;
                };
                if let Some(error_code) = recovered.last_error_code {
                    diagnostics.record(TransferCheckpointDiagnostic {
                        checkpoint_id: recovered.checkpoint_id,
                        status: TransferCheckpointDiagnosticStatus::Recovered,
                        error_code,
                        terminal: recovered.terminal,
                        retry_count: recovered.retry_count,
                    });
                }
            }
            Err(error) => {
                let Some(pending) = self.pending.get_mut(task_id) else {
                    return;
                };
                let error_code = error.code();
                pending.retry_count = pending.retry_count.saturating_add(1);
                if pending.last_error_code != Some(error_code) {
                    diagnostics.record(TransferCheckpointDiagnostic {
                        checkpoint_id: pending.checkpoint_id,
                        status: TransferCheckpointDiagnosticStatus::Failed,
                        error_code,
                        terminal: pending.terminal,
                        retry_count: pending.retry_count,
                    });
                }
                pending.last_error_code = Some(error_code);
                pending.next_retry_at = now + transfer_checkpoint_retry_delay(pending.retry_count);
            }
        }
    }

    fn receive_timeout(&self, now: Instant, poll_interval: Duration) -> Duration {
        self.pending
            .values()
            .map(|pending| pending.next_retry_at.saturating_duration_since(now))
            .min()
            .map_or(poll_interval, |retry_delay| retry_delay.min(poll_interval))
    }
}

fn transfer_checkpoint_retry_delay(retry_count: u32) -> Duration {
    let exponent = retry_count.saturating_sub(1).min(8);
    let multiplier = 1_u32 << exponent;
    TRANSFER_CHECKPOINT_RETRY_INITIAL
        .saturating_mul(multiplier)
        .min(TRANSFER_CHECKPOINT_RETRY_MAX)
}

pub struct DownloadScheduler {
    sender: Sender<Message>,
    coordinator: Option<JoinHandle<()>>,
    coordinator_stopping: Arc<AtomicBool>,
    dem_limits: Arc<AtomicU64>,
}

/// Whole-task admission limits, independent of each task's item concurrency.
/// Persistence belongs to ApplicationService; this is its current runtime projection.
#[derive(Clone, Copy, Debug)]
pub struct DemTaskLimits {
    max_tasks: u64,
    max_conversions: u64,
}

impl Default for DemTaskLimits {
    fn default() -> Self {
        Self {
            max_tasks: 2,
            max_conversions: 1,
        }
    }
}

impl DemTaskLimits {
    pub fn new(max_tasks: u64, max_conversions: u64) -> Option<Self> {
        ((1..=10).contains(&max_tasks) && max_conversions == 1).then_some(Self {
            max_tasks,
            max_conversions,
        })
    }

    fn packed(self) -> u64 {
        self.max_tasks | (self.max_conversions << 8)
    }
    fn load(value: &AtomicU64) -> Self {
        let value = value.load(Ordering::Acquire);
        Self {
            max_tasks: value & 0xff,
            max_conversions: value >> 8,
        }
    }
}

fn is_dem_task(task: &FrozenTaskSnapshot) -> bool {
    task.operation
        .as_ref()
        .is_some_and(|operation| operation.kind == "dem")
}

fn is_dem_conversion(task: &FrozenTaskSnapshot) -> bool {
    task.operation.as_ref().is_some_and(|operation| {
        operation.kind == "dem" && operation.approved_worker_operation != "dem_download"
    })
}

fn dem_task_has_slot(
    task: &FrozenTaskSnapshot,
    running: &HashMap<(String, String), RunningWorker>,
    limits: DemTaskLimits,
) -> bool {
    if !is_dem_task(task) {
        return true;
    }
    // Keep a slot until every worker actually exits, even if Store already says
    // paused/cancelled. Never park extra native threads merely to wait for a slot.
    let active: BTreeSet<&str> = running
        .values()
        .filter(|worker| is_dem_task(&worker.snapshot))
        .map(|worker| worker.snapshot.task_id.as_str())
        .collect();
    if active.contains(task.task_id.as_str()) {
        return true;
    }
    if active.len() >= limits.max_tasks as usize {
        return false;
    }
    !is_dem_conversion(task)
        || running
            .values()
            .filter(|worker| is_dem_conversion(&worker.snapshot))
            .map(|worker| worker.snapshot.task_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            < limits.max_conversions as usize
}

impl DownloadScheduler {
    pub fn start(
        store_path: impl AsRef<Path>,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
    ) -> Result<Self, StoreError> {
        let store = Arc::new(Mutex::new(TaskStore::open(store_path)?));
        Ok(Self::start_with_store(store, worker, poll_interval))
    }

    /// Start against a Store connection supplied by the owning application
    /// runtime. The scheduler never opens that path itself.
    pub fn start_with_store(
        store: SharedTaskStore,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
    ) -> Self {
        Self::start_with_checkpoint_diagnostics(
            store,
            worker,
            poll_interval,
            Arc::new(StderrTransferCheckpointDiagnosticSink),
        )
    }

    fn start_with_checkpoint_diagnostics(
        store: SharedTaskStore,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
        diagnostics: Arc<dyn TransferCheckpointDiagnosticSink>,
    ) -> Self {
        Self::start_internal(
            store,
            worker,
            poll_interval,
            diagnostics,
            DemTaskLimits::default(),
            default_free_space_probe(),
            stderr_disk_probe_log(),
            #[cfg(test)]
            None,
            Arc::new(SystemOrbitRecheckClock),
        )
    }

    /// Start with a substituted free-space probe (tests and embedders that
    /// measure storage themselves); every other constructor uses
    /// `disk_guard::default_free_space_probe`, the operating system's view in
    /// production builds.
    pub fn start_with_store_and_free_space_probe(
        store: SharedTaskStore,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
        probe: Arc<dyn FreeSpaceProbe>,
    ) -> Self {
        Self::start_with_store_free_space_probe_and_log(
            store,
            worker,
            poll_interval,
            probe,
            stderr_disk_probe_log(),
        )
    }

    /// `start_with_store_and_free_space_probe` with a substituted sink for the
    /// one-per-task "volume cannot be measured" diagnostic line.
    pub fn start_with_store_free_space_probe_and_log(
        store: SharedTaskStore,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
        probe: Arc<dyn FreeSpaceProbe>,
        log: DiskProbeLogSink,
    ) -> Self {
        Self::start_internal(
            store,
            worker,
            poll_interval,
            Arc::new(StderrTransferCheckpointDiagnosticSink),
            DemTaskLimits::default(),
            probe,
            log,
            #[cfg(test)]
            None,
            Arc::new(SystemOrbitRecheckClock),
        )
    }

    /// Load persisted limits before the coordinator can admit recovered work.
    pub fn start_with_store_and_dem_task_limits(
        store: SharedTaskStore,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
        limits: DemTaskLimits,
    ) -> Self {
        Self::start_internal(
            store,
            worker,
            poll_interval,
            Arc::new(StderrTransferCheckpointDiagnosticSink),
            limits,
            default_free_space_probe(),
            stderr_disk_probe_log(),
            #[cfg(test)]
            None,
            Arc::new(SystemOrbitRecheckClock),
        )
    }

    /// Orbit scheduled-fetch S3 (task contract 2026-11-01 §3.4): start with a
    /// substituted orbit recheck clock. Production keeps the system clock via
    /// the other constructors; tests use a controllable one so no test sleeps
    /// for a real backoff duration. The poll interval itself is unchanged.
    pub fn start_with_store_and_orbit_recheck_clock(
        store: SharedTaskStore,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
        orbit_clock: Arc<dyn OrbitRecheckClock>,
    ) -> Self {
        Self::start_internal(
            store,
            worker,
            poll_interval,
            Arc::new(StderrTransferCheckpointDiagnosticSink),
            DemTaskLimits::default(),
            default_free_space_probe(),
            stderr_disk_probe_log(),
            #[cfg(test)]
            None,
            orbit_clock,
        )
    }

    /// Raising the limit wakes queued tasks; lowering drains existing work without
    /// aborting downloads or changing any immutable task/file parameters.
    pub fn update_dem_task_limits(&self, limits: DemTaskLimits) {
        self.dem_limits.store(limits.packed(), Ordering::Release);
        self.wake();
    }

    #[cfg(test)]
    pub(crate) fn start_with_store_and_nisar_freeze_handler_gate_for_test(
        store: SharedTaskStore,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
    ) -> (Self, Arc<NisarPlanFreezeHandlerGate>) {
        let gate = Arc::new(NisarPlanFreezeHandlerGate {
            entered: AtomicBool::new(false),
            released: AtomicBool::new(false),
        });
        let scheduler = Self::start_internal(
            store,
            worker,
            poll_interval,
            Arc::new(StderrTransferCheckpointDiagnosticSink),
            DemTaskLimits::default(),
            default_free_space_probe(),
            stderr_disk_probe_log(),
            Some(Arc::clone(&gate)),
            Arc::new(SystemOrbitRecheckClock),
        );
        (scheduler, gate)
    }

    #[allow(clippy::too_many_arguments)]
    fn start_internal(
        store: SharedTaskStore,
        worker: Arc<dyn DownloadWorker>,
        poll_interval: Duration,
        diagnostics: Arc<dyn TransferCheckpointDiagnosticSink>,
        limits: DemTaskLimits,
        disk_probe: Arc<dyn FreeSpaceProbe>,
        disk_log: DiskProbeLogSink,
        #[cfg(test)] freeze_handler_gate: Option<Arc<NisarPlanFreezeHandlerGate>>,
        orbit_clock: Arc<dyn OrbitRecheckClock>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker_sender = sender.clone();
        let coordinator_stopping = Arc::new(AtomicBool::new(false));
        let worker_coordinator_stopping = Arc::clone(&coordinator_stopping);
        let dem_limits = Arc::new(AtomicU64::new(limits.packed()));
        let coordinator_dem_limits = Arc::clone(&dem_limits);
        let coordinator = thread::Builder::new()
            .name("insar-download-coordinator".to_string())
            .spawn(move || {
                coordinate(
                    store,
                    worker,
                    worker_sender,
                    receiver,
                    poll_interval,
                    diagnostics,
                    orbit_clock,
                    CoordinatorControl {
                        stopping: worker_coordinator_stopping,
                        dem_limits: coordinator_dem_limits,
                        disk_probe,
                        disk_log,
                        #[cfg(test)]
                        freeze_handler_gate,
                    },
                )
            })
            .expect("download coordinator thread creation must succeed");
        Self {
            sender,
            coordinator: Some(coordinator),
            coordinator_stopping,
            dem_limits,
        }
    }

    pub fn wake(&self) {
        let _ = self.sender.send(Message::Wake);
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.coordinator_stopping.store(true, Ordering::SeqCst);
        let _ = self.sender.send(Message::Shutdown);
        if let Some(coordinator) = self.coordinator.take() {
            let _ = coordinator.join();
        }
    }
}

impl Drop for DownloadScheduler {
    fn drop(&mut self) {
        self.stop();
    }
}

struct CoordinatorControl {
    stopping: Arc<AtomicBool>,
    dem_limits: Arc<AtomicU64>,
    disk_probe: Arc<dyn FreeSpaceProbe>,
    disk_log: DiskProbeLogSink,
    #[cfg(test)]
    freeze_handler_gate: Option<Arc<NisarPlanFreezeHandlerGate>>,
}

#[allow(clippy::too_many_arguments)]
fn coordinate(
    store: SharedTaskStore,
    worker: Arc<dyn DownloadWorker>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    poll_interval: Duration,
    diagnostics: Arc<dyn TransferCheckpointDiagnosticSink>,
    orbit_clock: Arc<dyn OrbitRecheckClock>,
    control: CoordinatorControl,
) {
    let mut running: HashMap<(String, String), RunningWorker> = HashMap::new();
    let mut scans = {
        let sender = sender.clone();
        crate::local_scan::LocalScans::new(
            worker.supports_local_scan(),
            Arc::new(move |report| {
                let _ = sender.send(Message::LocalScanReport(Box::new(report)));
            }),
        )
    };
    let mut checkpoints = TransferCheckpointTracker::default();
    let mut transfer_samples = HashMap::new();
    let mut finish_backoff = UnrecordedFinishBackoff::default();
    let mut pending_cancelled: HashMap<(String, String), PendingCancelledFinish> = HashMap::new();
    restore_pending_cancelled_finishes(&store, &mut pending_cancelled, &mut finish_backoff);
    let mut orbit_gate = OrbitRecheckGate::default();
    let mut disk = CoordinatorDiskGuard::new(
        Arc::clone(&control.disk_probe),
        Arc::clone(&control.disk_log),
    );
    loop {
        // §3.4: the nearest future orbit recheck folds into the existing
        // receive-timeout computation so a due check wakes the coordinator
        // without touching the poll interval constant.
        let orbit_wake = orbit_gate
            .next_due(orbit_clock.now_utc_seconds())
            .map(|due| Duration::from_secs(due - orbit_clock.now_utc_seconds()));
        let timeout = orbit_wake
            .map_or_else(
                || checkpoints.receive_timeout(Instant::now(), poll_interval),
                |wake| {
                    checkpoints
                        .receive_timeout(Instant::now(), poll_interval)
                        .min(wake)
                },
            )
            .max(Duration::from_millis(1));
        let receive_timeout = pending_cancelled
            .values()
            .map(|pending| pending.retry_at.saturating_duration_since(Instant::now()))
            .fold(timeout, Duration::min);
        let message = receiver.recv_timeout(receive_timeout);
        disk.begin_iteration();
        let mut run_schedule = match message {
            Ok(Message::Shutdown) => {
                control.stopping.store(true, Ordering::SeqCst);
                scans.shutdown();
                running.values().for_each(|worker| worker.control.cancel());
                for (_, worker) in running.drain() {
                    let _ = worker.handle.join();
                }
                return;
            }
            Ok(Message::Progress(progress)) => {
                // Measured before the Store lock: a slow volume never blocks
                // Store readers.
                let low = disk_reason_while_writing(&mut disk, &running, &progress);
                if let Ok(mut store) = store.lock() {
                    persist_progress(&mut store, &progress);
                    if let Some(reason) = low {
                        if pause_unless_paused(&mut store, &progress.task_id, &reason) {
                            let _ = reconcile_live_controls(&store, &mut running);
                        }
                    }
                }
                checkpoints.request(&progress.task_id, false, Instant::now());
                false
            }
            Ok(Message::LocalScanReport(report)) => {
                if let Ok(mut store) = store.lock() {
                    let released = commit_local_scan_report(&mut store, &report);
                    scans.report(&report, released);
                    record_local_scan_progress(
                        &mut store,
                        &report,
                        scans.scan_pending(&report.task_id),
                    );
                }
                true
            }
            Ok(Message::LocalReuseLoad {
                path_key,
                task_id,
                item_id,
                response,
            }) => {
                let result = store.lock().map_or_else(
                    |_| Err(LocalReuseAuthorityError::CoordinatorUnavailable),
                    |store| -> Result<LocalReuseLoaded, LocalReuseAuthorityError> {
                        Ok((
                            store.current_landed_artifact(&path_key)?,
                            store.task_item_local_reuse(&task_id, &item_id)?,
                        ))
                    },
                );
                let _ = response.send(result);
                false
            }
            Ok(Message::LocalReuseRecord { record, response }) => {
                let result = store.lock().map_or_else(
                    |_| Err(LocalReuseAuthorityError::CoordinatorUnavailable),
                    |mut store| apply_local_reuse_record(&mut store, &record),
                );
                let _ = response.send(result);
                false
            }
            Ok(Message::NisarExecutionPlanReady { request, response }) => {
                #[cfg(test)]
                if let Some(gate) = control.freeze_handler_gate.as_ref() {
                    gate.enter_and_wait();
                }
                let result = match store.lock() {
                    Ok(mut store) => {
                        let result = freeze_nisar_execution_plan_v2(&mut store, *request);
                        let _ = reconcile_live_controls(&store, &mut running);
                        result
                    }
                    Err(_) => Err(NisarExecutionPlanFreezeError::CoordinatorUnavailable),
                };
                let _ = response.send(result);
                false
            }
            Ok(Message::NisarSourceSizeReady { request, response }) => {
                let result = match store.lock() {
                    Ok(mut store) => {
                        let result = freeze_nisar_source_size_v2(&mut store, *request);
                        let _ = reconcile_live_controls(&store, &mut running);
                        result
                    }
                    Err(_) => Err(NisarSourceSizeFreezeError::CoordinatorUnavailable),
                };
                let _ = response.send(result);
                false
            }
            #[cfg(feature = "nisar-subset-production")]
            Ok(Message::NisarWorkspaceRebindingObserved { request, response }) => {
                let result = store.lock().map_or_else(
                    |_| Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable),
                    |mut store| {
                        store
                            .record_nisar_workspace_rebinding_event_v2(&request)
                            .map_err(Into::into)
                    },
                );
                let _ = response.send(result);
                false
            }
            #[cfg(feature = "nisar-subset-production")]
            Ok(Message::NisarOutputWaitObserved { request, response }) => {
                let result = store.lock().map_or_else(
                    |_| Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable),
                    |mut store| {
                        store
                            .record_nisar_output_wait_event_v2(&request)
                            .map_err(Into::into)
                    },
                );
                let _ = response.send(result);
                false
            }
            #[cfg(feature = "nisar-subset-production")]
            Ok(Message::NisarArchivedRecoveryLoad {
                task_id,
                item_id,
                response,
            }) => {
                let result = store.lock().map_or_else(
                    |_| Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable),
                    |store| {
                        store
                            .load_nisar_archived_recovery_v2(&task_id, &item_id)
                            .map_err(Into::into)
                    },
                );
                let _ = response.send(result);
                false
            }
            #[cfg(feature = "nisar-subset-production")]
            Ok(Message::NisarExecutionAnchorLoad {
                task_id,
                item_id,
                anchor_kind,
                response,
            }) => {
                let result = store.lock().map_or_else(
                    |_| Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable),
                    |store| {
                        store
                            .load_nisar_subset_execution_anchor_v2(&task_id, &item_id, &anchor_kind)
                            .map_err(Into::into)
                    },
                );
                let _ = response.send(result);
                false
            }
            #[cfg(feature = "nisar-subset-production")]
            Ok(Message::NisarExecutionAnchorCompareAndSwap { request, response }) => {
                let result = store.lock().map_or_else(
                    |_| Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable),
                    |mut store| {
                        store
                            .compare_and_swap_nisar_subset_execution_anchor_v2(&request)
                            .map_err(Into::into)
                    },
                );
                let _ = response.send(result);
                false
            }
            #[cfg(feature = "nisar-subset-production")]
            Ok(Message::NisarBudgetUpgradeProofLoad {
                task_id,
                item_id,
                response,
            }) => {
                let result = store.lock().map_or_else(
                    |_| Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable),
                    |store| {
                        store
                            .load_nisar_budget_upgrade_proof_v2(&task_id, &item_id)
                            .map_err(Into::into)
                    },
                );
                let _ = response.send(result);
                false
            }
            #[cfg(feature = "nisar-subset-production")]
            Ok(Message::NisarBudgetUpgradeContextLoad {
                task_id,
                item_id,
                response,
            }) => {
                let result = store.lock().map_or_else(
                    |_| Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable),
                    |store| {
                        store
                            .load_nisar_budget_upgrade_context_v2(&task_id, &item_id)
                            .map_err(Into::into)
                    },
                );
                let _ = response.send(result);
                false
            }
            #[cfg(feature = "nisar-subset-production")]
            Ok(Message::NisarBudgetUpgradePrepare {
                proof,
                next,
                response,
            }) => {
                let result = store.lock().map_or_else(
                    |_| Err(NisarExecutionAnchorAuthorityError::CoordinatorUnavailable),
                    |mut store| {
                        store
                            .prepare_nisar_budget_upgrade_v2(&proof, &next)
                            .map_err(Into::into)
                    },
                );
                let _ = response.send(result);
                false
            }
            Ok(Message::Finished(result)) => {
                let finished_worker = running
                    .remove(&(result.task_id.clone(), result.item_id.clone()))
                    .map(|worker| {
                        let _ = worker.handle.join();
                        // Read after join, including when the worker panicked.
                        let preserve = worker
                            .control
                            .preserve_prior_result_during_budget_migration
                            .load(Ordering::SeqCst);
                        (
                            worker.snapshot,
                            preserve,
                            worker.control,
                            worker.runtime_revision,
                        )
                    });
                let preserve = finished_worker
                    .as_ref()
                    .is_some_and(|(_, preserve, _, _)| *preserve);
                let finished_control = finished_worker
                    .as_ref()
                    .map(|(_, _, control, _)| control.clone());
                let finished_revision = finished_worker
                    .as_ref()
                    .map(|(_, _, _, revision)| *revision);
                let cached_snapshot = finished_worker.map(|(snapshot, _, _, _)| snapshot);
                let scheduled_directly = if result.outcome == DownloadOutcome::Cancelled {
                    if let Some(started_revision) = finished_revision {
                        let key = (result.task_id.clone(), result.item_id.clone());
                        finish_backoff.blocked.insert(key.clone());
                        pending_cancelled.insert(
                            key,
                            PendingCancelledFinish::new(result.clone(), started_revision, preserve),
                        );
                    } else {
                        // The worker bookkeeping vanished unexpectedly. Do not
                        // infer a generation or allow this key to be respawned.
                        finish_backoff
                            .blocked
                            .insert((result.task_id.clone(), result.item_id.clone()));
                    }
                    false
                } else if let Ok(mut store) = store.lock() {
                    // Orbit scheduled-fetch S3 (§3.4): an ORB002 finish is a
                    // "not published yet" verdict, not a failure. When the
                    // source's auto-fetch preference is on, the item is
                    // requeued with a persisted recheck row in one pass; with
                    // auto-fetch off it keeps today's failure behavior. ORB003
                    // and every other code use the existing paths below.
                    if result.outcome == DownloadOutcome::Failed
                        && result.failure.as_ref().is_some_and(|f| f.code == "ORB002")
                        && cached_snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot.layout == insar_core::DownloadLayout::SentinelOrbitAuxPoeorb
                                || snapshot.provider_id == "sentinel1.orbit"
                        })
                        && store
                            .load_companion_preferences(
                                cached_snapshot
                                    .as_ref()
                                    .expect("snapshot checked above")
                                    .provider_id
                                    .as_str(),
                            )
                            .map(|preferences| preferences.auto_fetch_orbit)
                            .unwrap_or(true)
                        && finish_orbit_recheck(
                            &mut store,
                            &result,
                            &mut orbit_gate,
                            orbit_clock.now_utc_seconds(),
                            &mut finish_backoff,
                        )
                    {
                        checkpoints.request(&result.task_id, true, Instant::now());
                        let _ = sender.send(Message::Wake);
                        false
                    } else {
                        let updated_runtime = if preserve {
                            let runtime = finish_budget_migration(
                                &mut store,
                                &result,
                                finished_control.as_ref(),
                            );
                            if result.outcome == DownloadOutcome::Failed {
                                finish_backoff.observe(
                                    (result.task_id.clone(), result.item_id.clone()),
                                    runtime.is_some(),
                                    Instant::now(),
                                );
                            }
                            runtime
                        } else {
                            persist_result(&mut store, &result);
                            let runtime = finish_runtime(&mut store, &result);
                            if result.outcome != DownloadOutcome::Interrupted {
                                let recorded = runtime.is_some()
                                    || !finished_item_still_active(&store, &result);
                                finish_backoff.observe(
                                    (result.task_id.clone(), result.item_id.clone()),
                                    recorded,
                                    Instant::now(),
                                );
                            }
                            runtime
                        };
                        if let Some(updated_runtime) = updated_runtime {
                            if let Some(updated_task) = stored_task_for_updated_runtime(
                                &store,
                                cached_snapshot,
                                updated_runtime,
                            ) {
                                schedule_stored_task(
                                    &mut store,
                                    updated_task,
                                    &worker,
                                    &sender,
                                    &control.stopping,
                                    &mut running,
                                    DemTaskLimits::load(&control.dem_limits),
                                    &finish_backoff,
                                    &mut scans,
                                    &mut disk,
                                    &orbit_gate,
                                    orbit_clock.now_utc_seconds(),
                                );
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                } else {
                    false
                };
                checkpoints.request(&result.task_id, true, Instant::now());
                !scheduled_directly
            }
            Ok(Message::Wake) | Err(RecvTimeoutError::Timeout) => true,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Volumes a scheduling pass asked about are measured here, outside
        // the Store lock, and that pass is repeated.
        if disk.refresh_wanted() {
            run_schedule = true;
        }
        if let Ok(mut store) = store.lock() {
            let now = Instant::now();
            if retry_pending_cancelled_finishes(
                &mut store,
                &mut pending_cancelled,
                &mut finish_backoff,
                now,
            ) {
                run_schedule = true;
            }
            let task_ids = running
                .keys()
                .map(|(task_id, _)| task_id.clone())
                .collect::<BTreeSet<_>>();
            for task_id in task_ids {
                checkpoints.request(&task_id, false, Instant::now());
            }
            let checkpoint_now = Instant::now();
            for task_id in checkpoints.due_task_ids(checkpoint_now) {
                let result = checkpoint_task_transfer_activity(
                    &mut store,
                    worker.as_ref(),
                    &task_id,
                    &mut transfer_samples,
                );
                checkpoints.observe_result(&task_id, result, checkpoint_now, diagnostics.as_ref());
            }
            // Orbit scheduled-fetch S3 (§3.4): the gate view is refreshed once
            // per scheduling pass, alongside the scheduler tasks — never one
            // table query per tick. Waiting rows whose item left the
            // schedulable set (item cancel, whole-task cancel, archive) are
            // cleared here; a due row is claimed exactly once by the pass.
            orbit_gate.refresh(&store);
            reconcile_orbit_recheck_cancellations(&mut store, &orbit_gate);
            // Progress changes bytes, not available concurrency. Re-scanning every
            // task after each persisted progress sample can backlog Finished messages
            // under active transfers. Wake/Finished/idle ticks remain the scheduling
            // authority for claiming new work.
            if run_schedule {
                schedule(
                    &mut store,
                    &worker,
                    &sender,
                    &control.stopping,
                    &mut running,
                    DemTaskLimits::load(&control.dem_limits),
                    &finish_backoff,
                    &mut scans,
                    &mut disk,
                    &orbit_gate,
                    orbit_clock.now_utc_seconds(),
                );
            }
        }
        if disk.has_wanted() {
            let _ = sender.send(Message::Wake);
        }
    }
}

fn retry_pending_cancelled_finishes(
    store: &mut TaskStore,
    pending_cancelled: &mut HashMap<(String, String), PendingCancelledFinish>,
    finish_backoff: &mut UnrecordedFinishBackoff,
    now: Instant,
) -> bool {
    let mut run_schedule = false;
    let ready = pending_cancelled
        .iter()
        .filter(|(_, pending)| pending.retry_at <= now)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for key in ready {
        let Some(pending) = pending_cancelled.get_mut(&key) else {
            continue;
        };
        let outcome = match store.load_scheduler_control_runtime(&pending.result.task_id) {
            Ok(runtime) => store.finish_scheduler_cancelled_result_with_legacy_policy(
                &pending.result,
                pending.started_revision,
                runtime.as_ref().map_or(0, |runtime| runtime.revision),
                &event_id(&pending.result.task_id, &pending.result.item_id),
                pending.preserve,
            ),
            Err(error) => match store.confirm_scheduler_task_tombstone(&pending.result.task_id) {
                Ok(true) => Ok(SchedulerCancelledFinish::Tombstoned),
                _ => Err(error),
            },
        };
        match outcome {
            Ok(SchedulerCancelledFinish::Retried(_)) | Ok(SchedulerCancelledFinish::Settled(_)) => {
                pending_cancelled.remove(&key);
                finish_backoff.blocked.remove(&key);
                finish_backoff.observe(key, true, now);
                run_schedule = true;
            }
            Ok(SchedulerCancelledFinish::Tombstoned) => {
                // The settle transaction clears the row, but a tombstone found
                // after a failed runtime read did not run it.
                if pending.persisted
                    && store
                        .forget_scheduler_pending_cancelled_finish(&key.0, &key.1)
                        .is_err()
                {
                    pending.retry_at = now + UnrecordedFinishBackoff::delay(pending.failures);
                    pending.failures = pending.failures.saturating_add(1);
                    continue;
                }
                pending_cancelled.remove(&key);
                finish_backoff.blocked.remove(&key);
                finish_backoff.observe(key, true, now);
                run_schedule = true;
            }
            // A previous process left this result, and its evidence cannot
            // settle it. That worker is gone and can no longer write, so the
            // item is released to run as a new generation instead of staying
            // blocked across every restart. The old result is not recorded.
            Err(error)
                if pending.restored
                    && !store_error_is_transient(&error)
                    && store
                        .forget_scheduler_pending_cancelled_finish(&key.0, &key.1)
                        .is_ok() =>
            {
                eprintln!(
                    "download cancelled finish released after restart: task_id={} item_id={} code={}; the earlier run's cancelled result could not be settled and was not recorded",
                    key.0,
                    key.1,
                    error.code(),
                );
                pending_cancelled.remove(&key);
                finish_backoff.blocked.remove(&key);
                run_schedule = true;
            }
            Err(error) => {
                if !pending.persisted {
                    pending.persisted = store
                        .record_scheduler_pending_cancelled_finish(
                            &SchedulerPendingCancelledFinish {
                                result: pending.result.clone(),
                                started_revision: pending.started_revision,
                                preserve_result: pending.preserve,
                            },
                        )
                        .is_ok();
                }
                let next = now + UnrecordedFinishBackoff::delay(pending.failures);
                let failures = pending.failures.saturating_add(1);
                pending.retry_at = next;
                pending.failures = failures;
                if pending
                    .last_diagnostic
                    .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(60))
                {
                    eprintln!(
                        "download cancelled finish pending: task_id={} item_id={} code={} retry_count={} kept_across_restart={}; evidence may need repair",
                        key.0,
                        key.1,
                        error.code(),
                        failures,
                        pending.persisted,
                    );
                    pending.last_diagnostic = Some(now);
                }
            }
        }
    }
    run_schedule
}

/// Orbit scheduled-fetch S3 (§3.4): settle one ORB002 "not published yet"
/// finish. Returns true when the item was requeued with an updated schedule
/// row; false keeps the existing failure handling (auto-fetch off, or a
/// Store refusal whose ordinary backoff path is the right answer).
///
/// One pass under the Store lock:
/// - the schedule row is upserted with `attempt + 1`, the first check at
///   `max(now, acquisition + 20 d)` and later checks backed off 12 h → 24 h
///   → 48 h → 96 h → 7 d cap from the check that just finished;
/// - when the next check would pass the `acquisition + 60 d` deadline, the
///   item instead fails terminally as ORB003 and the row is exhausted;
/// - the item is put back to queued with the user-facing `RetryItems`
///   transition, so pause/cancel/restart semantics stay untouched;
/// - the key is explicitly removed from `finish_backoff` (§3.1: the most
///   easily missed spot — a key left blocked there would never be
///   dispatched again, deadline or not).
fn finish_orbit_recheck(
    store: &mut TaskStore,
    result: &DownloadItemResult,
    orbit_gate: &mut OrbitRecheckGate,
    now: u64,
    finish_backoff: &mut UnrecordedFinishBackoff,
) -> bool {
    let key = (result.task_id.clone(), result.item_id.clone());
    let previous = orbit_gate.entries.get(&key).cloned();
    let acquisition_at = previous
        .as_ref()
        .map(|entry| entry.acquisition_at.clone())
        .or_else(|| {
            store
                .load_scheduler_task(&result.task_id)
                .ok()
                .flatten()
                .and_then(|stored| {
                    stored
                        .snapshot
                        .candidates
                        .into_iter()
                        .find(|item| item.item_id == result.item_id)
                        .and_then(|item| item.acquisition_datetime)
                })
        });
    let Some(acquisition_at) = acquisition_at else {
        eprintln!(
            "orbit recheck finish without a scene acquisition time: task_id={} item_id={}; keeping the failure path",
            result.task_id, result.item_id
        );
        return false;
    };
    let acquisition = parse_utc_seconds(&acquisition_at);
    let deadline = acquisition.map(|seconds| seconds + ORBIT_RECHECK_DEADLINE);
    if acquisition.is_none() || deadline.is_none() {
        eprintln!(
            "orbit recheck acquisition timestamp is not a whole-second UTC value: task_id={} item_id={}; keeping the failure path",
            result.task_id, result.item_id
        );
        return false;
    }
    let acquisition = acquisition.unwrap_or_default();
    let deadline = deadline.unwrap_or_default();
    let attempt = previous.as_ref().map_or(1, |entry| entry.attempt + 1);
    let updated_at = format_utc_seconds(now);
    // A recheck whose next interval would pass the deadline fails as ORB003
    // instead: the wait is over, only a manual retry can revive the item.
    let next_check_at = previous
        .as_ref()
        .map(|_| now.saturating_add(orbit_recheck_backoff(attempt)))
        .unwrap_or_else(|| now.max(acquisition + ORBIT_RECHECK_FIRST_DELAY));
    if next_check_at > deadline {
        let orb003 = DownloadItemResult {
            task_id: result.task_id.clone(),
            item_id: result.item_id.clone(),
            outcome: DownloadOutcome::Failed,
            destination: result.destination.clone(),
            bytes_written: 0,
            attempt_count: result.attempt_count,
            failure: Some(DownloadFailure {
                code: "ORB003".to_owned(),
                reason_zh: "精密轨道等待已超上限，请手动补下（等待发布已超过 60 天）".to_owned(),
                retryable: true,
            }),
            finished_at: updated_at.clone(),
        };
        let exhausted = insar_store::StoreOrbitRecheckRequest {
            task_id: result.task_id.clone(),
            item_id: result.item_id.clone(),
            scene_id: previous
                .as_ref()
                .map(|entry| entry.scene_id.clone())
                .unwrap_or_else(|| result.item_id.clone()),
            orbit_type: "POEORB".to_owned(),
            acquisition_at: acquisition_at.clone(),
            attempt,
            next_check_at: format_utc_seconds(next_check_at),
            deadline_at: format_utc_seconds(deadline),
            state: insar_store::StoreOrbitRecheckState::Exhausted,
            last_error_code: Some("ORB003".to_owned()),
            updated_at: updated_at.clone(),
        };
        if persist_result_and_orbit_recheck(store, &orb003, Some(&exhausted))
            .and_then(|runtime| {
                apply_command_with_revision(
                    store,
                    &runtime,
                    TaskStateCommandKind::MarkFailed,
                    &exhausted,
                    "orbit-exhausted",
                    &updated_at,
                )
            })
            .is_some()
        {
            orbit_gate.refresh(store);
            finish_backoff.observe(key, true, Instant::now());
            return true;
        }
        return false;
    }
    let request = insar_store::StoreOrbitRecheckRequest {
        task_id: result.task_id.clone(),
        item_id: result.item_id.clone(),
        scene_id: previous
            .as_ref()
            .map(|entry| entry.scene_id.clone())
            .unwrap_or_else(|| result.item_id.clone()),
        orbit_type: "POEORB".to_owned(),
        acquisition_at: acquisition_at.clone(),
        attempt,
        next_check_at: format_utc_seconds(next_check_at),
        deadline_at: format_utc_seconds(deadline),
        state: insar_store::StoreOrbitRecheckState::Waiting,
        last_error_code: Some("ORB002".to_owned()),
        updated_at: updated_at.clone(),
    };
    // The ORB002 diagnostic itself is recorded as the item's current result
    // row (§2.2: overwritten by the later success). The item is still Active
    // at finish time, so the requeue is the same pair of transitions a
    // user-facing retry uses: MarkFailed (Active → Failed, with the ORB002
    // row visible in between), then RetryItems (Failed → Queued).
    let requeued = persist_result_and_orbit_recheck(store, result, Some(&request))
        .and_then(|runtime| {
            apply_command_with_revision(
                store,
                &runtime,
                TaskStateCommandKind::MarkFailed,
                &request,
                "orbit-recheck-failed",
                &updated_at,
            )
        })
        .and_then(|runtime| {
            apply_command_with_revision(
                store,
                &runtime,
                TaskStateCommandKind::RetryItems,
                &request,
                "orbit-recheck-requeue",
                &updated_at,
            )
        })
        .is_some();
    if requeued {
        orbit_gate.refresh(store);
        finish_backoff.blocked.remove(&key);
        finish_backoff.observe(key, true, Instant::now());
    }
    requeued
}

/// Persist the item result and upsert one schedule row, then return the
/// current runtime revision (an immediate re-read keeps the requeue honest
/// against a concurrent transition).
fn persist_result_and_orbit_recheck(
    store: &mut TaskStore,
    result: &DownloadItemResult,
    recheck: Option<&insar_store::StoreOrbitRecheckRequest>,
) -> Option<TaskRuntimeSnapshot> {
    persist_result(store, result);
    if let Some(recheck) = recheck {
        if let Err(error) = store.upsert_orbit_recheck(recheck) {
            eprintln!(
                "orbit recheck schedule write failed: task_id={} item_id={} code={}",
                result.task_id,
                result.item_id,
                error.code()
            );
            return None;
        }
    }
    store
        .load_scheduler_control_runtime(&result.task_id)
        .ok()
        .flatten()
}

/// Apply one runtime command against the revision just read, retrying once
/// on a lost revision race (a concurrent pause or cancellation wins and is
/// respected). Returns the runtime after the applied command.
fn apply_command_with_revision(
    store: &mut TaskStore,
    runtime: &TaskRuntimeSnapshot,
    kind: TaskStateCommandKind,
    recheck: &insar_store::StoreOrbitRecheckRequest,
    label: &str,
    occurred_at: &str,
) -> Option<TaskRuntimeSnapshot> {
    let mut request = StoreUpdateTaskRequest {
        task_id: recheck.task_id.clone(),
        expected_revision: runtime.revision,
        command: StoreTaskCommand {
            kind,
            item_ids: vec![recheck.item_id.clone()],
        },
        event_id: event_id(&recheck.task_id, &format!("{}:{label}", recheck.item_id)),
        occurred_at: occurred_at.to_owned(),
    };
    let mut attempts = 0;
    loop {
        match store.update_task(&request) {
            Ok(_) => {
                return store
                    .load_scheduler_control_runtime(&recheck.task_id)
                    .ok()
                    .flatten();
            }
            Err(StoreError::RevisionConflict) if attempts == 0 => {
                // A concurrent transition advanced the revision; re-read and
                // retry once so the Store applies the current state.
                attempts += 1;
                let runtime = store
                    .load_scheduler_control_runtime(&recheck.task_id)
                    .ok()
                    .flatten()?;
                request.expected_revision = runtime.revision;
            }
            Err(_) => return None,
        }
    }
}

/// Waiting rows whose item is no longer in the schedulable, queued state are
/// terminal for this engine: clear them (TASK-REG-012). `RetryItems` and
/// `ReacquireItems` move items back to queued, so their rows survive and keep
/// gating. A task that left the scheduler set entirely (cancel/archive) has
/// its remaining rows removed in one call.
fn reconcile_orbit_recheck_cancellations(store: &mut TaskStore, orbit_gate: &OrbitRecheckGate) {
    let mut item_clears: Vec<(String, String)> = Vec::new();
    let mut task_clears: Vec<String> = Vec::new();
    for entry in orbit_gate.entries.values() {
        if !entry.waiting {
            continue;
        }
        match store.load_scheduler_task(&entry.task_id) {
            Ok(Some(stored)) => {
                // A pause (task-level keeps items queued; item-level moves
                // them to paused) is a scheduling hold, not a cancellation:
                // the row survives so a later resume keeps the attempt and
                // backoff. Only an item that left the schedulable sets
                // entirely (cancelled, terminal, task removed) clears it.
                if !stored.runtime.queued_item_ids.contains(&entry.item_id)
                    && !stored.runtime.paused_item_ids.contains(&entry.item_id)
                {
                    item_clears.push((entry.task_id.clone(), entry.item_id.clone()));
                }
            }
            Ok(None) => task_clears.push(entry.task_id.clone()),
            Err(_) => {}
        }
    }
    for (task_id, item_id) in item_clears {
        if let Err(error) = store.clear_orbit_recheck(&task_id, &item_id) {
            eprintln!(
                "orbit recheck clear failed: task_id={} item_id={} code={}",
                task_id,
                item_id,
                error.code()
            );
        }
    }
    for task_id in task_clears {
        if let Err(error) = store.cancel_orbit_rechecks_for_task(&task_id) {
            eprintln!(
                "orbit recheck task clear failed: task_id={} code={}",
                task_id,
                error.code()
            );
        }
    }
}

fn freeze_nisar_source_size_v2(
    store: &mut TaskStore,
    request: NisarSourceSizeReadyV2,
) -> Result<u64, NisarSourceSizeFreezeError> {
    let current = store
        .load_scheduler_task(&request.task_id)?
        .ok_or(StoreError::NotFound)?;
    if current.runtime.status != insar_core::TaskLifecycleStatus::Running
        || current.runtime.task_paused
        || !current
            .runtime
            .selected_item_ids
            .iter()
            .any(|item_id| item_id == &request.item_id)
        || !current
            .runtime
            .active_item_ids
            .iter()
            .any(|item_id| item_id == &request.item_id)
    {
        return Err(StoreError::NisarSubsetSourceSizeStateInvalid.into());
    }
    store.freeze_nisar_subset_source_size_v2(&StoreFreezeNisarSubsetSourceSizeV2Request {
        task_id: request.task_id.clone(),
        item_id: request.item_id.clone(),
        provider_id: request.provider_id,
        collection_id: request.collection_id,
        provider_product_id: request.provider_product_id,
        provider_product_key: request.provider_product_key,
        asset_key: request.asset_key,
        stable_asset_key: request.stable_asset_key,
        prepared_intent_sha256: request.prepared_intent_sha256,
        expected_revision: current.runtime.revision,
        occurred_at: timestamp_now(),
        expected_size_bytes: request.expected_size_bytes,
    })?;
    let reloaded = store
        .load_scheduler_task(&request.task_id)?
        .ok_or(StoreError::NotFound)?;
    if reloaded.runtime.status != insar_core::TaskLifecycleStatus::Running
        || reloaded.runtime.task_paused
        || !reloaded
            .runtime
            .active_item_ids
            .iter()
            .any(|item_id| item_id == &request.item_id)
    {
        return Err(StoreError::NisarSubsetSourceSizeStateInvalid.into());
    }
    reloaded
        .snapshot
        .candidates
        .iter()
        .find(|item| item.item_id == request.item_id)
        .and_then(|item| item.asset_access.as_ref())
        .and_then(|access| access.expected_size_bytes)
        .filter(|size| *size == request.expected_size_bytes)
        .ok_or(StoreError::NisarSubsetSourceSizeIdentityMismatch.into())
}

fn freeze_nisar_execution_plan_v2(
    store: &mut TaskStore,
    request: NisarExecutionPlanReadyV2,
) -> Result<NisarSubsetPlanV2, NisarExecutionPlanFreezeError> {
    let current = store
        .load_scheduler_task(&request.task_id)?
        .ok_or(StoreError::NotFound)?;
    if current.runtime.status != insar_core::TaskLifecycleStatus::Running
        || current.runtime.task_paused
        || !current
            .runtime
            .active_item_ids
            .iter()
            .any(|item_id| item_id == &request.item_id)
    {
        return Err(StoreError::NisarSubsetExecutionPlanAttachStateInvalid.into());
    }
    let current_item = current
        .snapshot
        .candidates
        .iter()
        .find(|item| item.item_id == request.item_id)
        .ok_or(StoreError::NisarSubsetPreparedIntentIdentityMismatch)?;
    let intent = parse_nisar_subset_prepared_intent_v2_envelope(&current_item.metadata)?
        .ok_or(StoreError::NisarSubsetPreparedIntentMissing)?;
    if intent.canonical_intent_sha256 != request.prepared_intent_sha256
        || request.proposed_plan.prepared_intent_sha256 != request.prepared_intent_sha256
    {
        return Err(StoreError::NisarSubsetExecutionPlanIntentMismatch.into());
    }

    store.attach_nisar_subset_execution_plan_v2(&StoreAttachNisarSubsetExecutionPlanV2Request {
        task_id: request.task_id.clone(),
        item_id: request.item_id.clone(),
        prepared_intent_sha256: request.prepared_intent_sha256.clone(),
        expected_revision: current.runtime.revision,
        occurred_at: timestamp_now(),
        plan: request.proposed_plan.clone(),
    })?;

    let reloaded = store
        .load_scheduler_task(&request.task_id)?
        .ok_or(StoreError::NotFound)?;
    if reloaded.runtime.status != insar_core::TaskLifecycleStatus::Running
        || reloaded.runtime.task_paused
        || !reloaded
            .runtime
            .active_item_ids
            .iter()
            .any(|item_id| item_id == &request.item_id)
    {
        return Err(StoreError::NisarSubsetExecutionPlanAttachStateInvalid.into());
    }
    let frozen_item = reloaded
        .snapshot
        .candidates
        .iter()
        .find(|item| item.item_id == request.item_id)
        .ok_or(StoreError::NisarSubsetPreparedIntentIdentityMismatch)?;
    let frozen = parse_nisar_subset_plan_v2_envelope(&frozen_item.metadata)?
        .ok_or(StoreError::NisarSubsetExecutionPlanInvalid)?;
    let plan = frozen.plan()?;
    if frozen.prepared_intent_sha256 != request.prepared_intent_sha256
        || frozen.canonical_semantic_plan_digest
            != request.proposed_plan.canonical_semantic_plan_digest
    {
        return Err(StoreError::NisarSubsetExecutionPlanAlreadyFrozenConflict.into());
    }
    Ok(plan)
}

#[allow(clippy::too_many_arguments)]
fn schedule(
    store: &mut TaskStore,
    worker: &Arc<dyn DownloadWorker>,
    sender: &Sender<Message>,
    coordinator_stopping: &Arc<AtomicBool>,
    running: &mut HashMap<(String, String), RunningWorker>,
    dem_limits: DemTaskLimits,
    finish_backoff: &UnrecordedFinishBackoff,
    scans: &mut crate::local_scan::LocalScans,
    disk: &mut CoordinatorDiskGuard,
    orbit_gate: &OrbitRecheckGate,
    orbit_now: u64,
) {
    if reconcile_live_controls(store, running).is_err() {
        return;
    }
    let Ok(stored) = store.load_scheduler_tasks() else {
        return;
    };
    scans.retain(
        &stored
            .iter()
            .map(|task| task.runtime.task_id.clone())
            .collect(),
    );
    for stored_task in stored {
        schedule_stored_task(
            store,
            stored_task,
            worker,
            sender,
            coordinator_stopping,
            running,
            dem_limits,
            finish_backoff,
            scans,
            disk,
            orbit_gate,
            orbit_now,
        );
    }
}

/// Downloader D3-2: commits the reusable items of a scan report (one Store
/// batch). Returns the items released to their workers: the report's
/// releases, plus reusable items whose batch the Store refused (their
/// workers' gates decide them instead).
fn commit_local_scan_report(
    store: &mut TaskStore,
    report: &crate::local_scan::LocalScanReport,
) -> Vec<String> {
    let mut released = report.released.clone();
    if !report.reused.is_empty() {
        let batch = insar_store::StoreLocalReuseBatch {
            task_id: report.task_id.clone(),
            items: report.reused.clone(),
            event_id: event_id(&report.task_id, "local-reuse"),
            occurred_at: timestamp_now(),
        };
        if store.apply_local_reuse_batch(&batch).is_err() {
            released.extend(report.reused.iter().map(|item| item.item_id.clone()));
        }
    }
    released
}

/// Downloader D3-2: records the scan progress after a report. The first
/// phase reports k/N; the digest job keeps the recorded counts. The scan is
/// done only when neither phase has work left for the task.
fn record_local_scan_progress(
    store: &mut TaskStore,
    report: &crate::local_scan::LocalScanReport,
    scan_pending: bool,
) {
    use crate::local_scan::{LocalScanEnd, LocalScanJobKind};
    use insar_store::StoreLocalScanState;
    let (scanned_items, total_items) = match report.kind {
        LocalScanJobKind::Scan => (report.scanned.min(report.total), report.total),
        LocalScanJobKind::Digest => match store.task_local_scan(&report.task_id) {
            Ok(Some(row)) => (row.scanned_items, row.total_items),
            _ => return,
        },
    };
    let state = match report.end {
        Some(LocalScanEnd::Stopped) => StoreLocalScanState::Stopped,
        Some(LocalScanEnd::Done) if !scan_pending => StoreLocalScanState::Done,
        Some(LocalScanEnd::Done) | None => StoreLocalScanState::Scanning,
    };
    let _ = store.record_task_local_scan(&insar_store::StoreTaskLocalScan {
        task_id: report.task_id.clone(),
        state,
        scanned_items,
        total_items,
        updated_at: timestamp_now(),
    });
}

#[allow(clippy::too_many_arguments)]
fn schedule_stored_task(
    store: &mut TaskStore,
    stored_task: StoredTask,
    worker: &Arc<dyn DownloadWorker>,
    sender: &Sender<Message>,
    coordinator_stopping: &Arc<AtomicBool>,
    running: &mut HashMap<(String, String), RunningWorker>,
    dem_limits: DemTaskLimits,
    finish_backoff: &UnrecordedFinishBackoff,
    scans: &mut crate::local_scan::LocalScans,
    disk: &mut CoordinatorDiskGuard,
    orbit_gate: &OrbitRecheckGate,
    orbit_now: u64,
) {
    let runtime = stored_task.runtime;
    let task = stored_task.snapshot;
    if runtime.task_paused {
        scans.pause(&task.task_id);
        return;
    }
    if !dem_task_has_slot(&task, running, dem_limits) {
        return;
    }
    if would_open_item(&runtime, running) {
        // Before any item is opened for writing: no worker starts on a low
        // drive. This pass holds the Store lock, so it only reads a
        // measurement younger than one second; otherwise one is taken outside
        // the lock and the task is scheduled again right after.
        match disk.cached_verdict(&task.task_id, &task.output_root, Instant::now()) {
            CachedDiskVerdict::Pending => return,
            CachedDiskVerdict::Allow => {}
            CachedDiskVerdict::Low(reason) => {
                if pause_for_disk_low(store, &runtime.task_id, runtime.revision, &reason) {
                    let _ = reconcile_live_controls(store, running);
                }
                return;
            }
        }
    }
    for item_id in &runtime.active_item_ids {
        spawn_if_missing(
            &task,
            item_id,
            runtime.revision,
            worker,
            sender,
            coordinator_stopping,
            running,
            finish_backoff,
        );
    }
    let engine_prefs = active_transfer_engine_prefs();
    let base_ceiling: u64 = match active_transfer_engine_prefs_override() {
        Some(prefs) => runtime
            .concurrency
            .min(u64::from(prefs.max_concurrent_tasks.max(1))),
        None => runtime.concurrency,
    };
    let effective_concurrency: u64 = if !engine_prefs.smart_concurrency || base_ceiling <= 1 {
        base_ceiling
    } else {
        let configured_u32 = u32::try_from(base_ceiling).unwrap_or(u32::MAX);
        // TASK-REG-016: read cached volume space only (never probe under the Store lock);
        // missing/stale roots are marked wanted and refreshed outside the Store lock.
        let now = Instant::now();
        let free_space = disk.cached_free_space(&task.output_root, now);
        let sample = worker.task_transfer_sample(&task.task_id);
        let observed_throughput_bps = sample
            .map(|s| {
                s.network_bytes_received
                    .saturating_mul(1000)
                    .checked_div(s.active_millis)
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let recent_stall_count = worker.task_recent_stall_count(&task.task_id);
        let recommended = SmartConcurrencyTuner::recommend_concurrency(&SmartConcurrencyInputs {
            configured_concurrency: configured_u32,
            smart_concurrency_enabled: engine_prefs.smart_concurrency,
            free_space,
            observed_throughput_bps,
            recent_stall_count,
        });
        // TaskRuntimeSnapshot::validate enforces active_item_ids.len() <= runtime.concurrency,
        // so adaptive concurrency and runtime Tier-1 overrides may throttle below
        // runtime.concurrency, never exceed the task's persisted concurrency ceiling.
        u64::from(recommended).min(base_ceiling)
    };
    let capacity = usize::try_from(effective_concurrency)
        .unwrap_or(usize::MAX)
        .saturating_sub(runtime.active_item_ids.len());
    // Downloader D3-2: only queued items the pre-scan released are claimed.
    let (claimable, scanning) = scans.claimable(
        &task,
        &runtime,
        worker,
        NisarExecutionPlanFreezeAuthority {
            sender: sender.clone(),
            coordinator_stopping: Arc::clone(coordinator_stopping),
        },
    );
    if let Some(total) = scanning {
        let _ = store.record_task_local_scan(&insar_store::StoreTaskLocalScan {
            task_id: task.task_id.clone(),
            state: insar_store::StoreLocalScanState::Scanning,
            scanned_items: 0,
            total_items: total,
            updated_at: timestamp_now(),
        });
    }
    let claims: Vec<String> = claimable
        .into_iter()
        .filter(|item_id| {
            !finish_backoff
                .blocked
                .contains(&(task.task_id.clone(), item_id.clone()))
        })
        // Orbit scheduled-fetch S3 (§3.4): the claim gate. A waiting recheck
        // row whose next check is in the future stays out of the claims, so
        // it occupies no capacity slot, triggers no ActivateItems and no
        // local scan, and cannot delay another task's items.
        .filter(|item_id| !orbit_gate.holds(&task.task_id, item_id, orbit_now))
        .take(capacity)
        .collect();
    if claims.is_empty() {
        return;
    }
    let request = StoreUpdateTaskRequest {
        task_id: runtime.task_id.clone(),
        expected_revision: runtime.revision,
        command: StoreTaskCommand {
            kind: TaskStateCommandKind::ActivateItems,
            item_ids: claims.clone(),
        },
        event_id: event_id(&runtime.task_id, "activate"),
        occurred_at: timestamp_now(),
    };
    let Ok(activated) = store.update_scheduler_runtime(&request) else {
        return;
    };
    for item_id in claims {
        let key = (task.task_id.clone(), item_id.clone());
        if let Some(worker) = running.get(&key) {
            worker.control.set_paused(false);
        } else {
            spawn_if_missing(
                &task,
                &item_id,
                activated.revision,
                worker,
                sender,
                coordinator_stopping,
                running,
                finish_backoff,
            );
        }
    }
}

/// Whether scheduling this task would start a worker: an active item without
/// one (recovery) or a queued item with a free concurrency slot.
fn would_open_item(
    runtime: &TaskRuntimeSnapshot,
    running: &HashMap<(String, String), RunningWorker>,
) -> bool {
    let unstarted_active = runtime
        .active_item_ids
        .iter()
        .any(|item_id| !running.contains_key(&(runtime.task_id.clone(), item_id.clone())));
    let free_slot =
        usize::try_from(runtime.concurrency).unwrap_or(usize::MAX) > runtime.active_item_ids.len();
    unstarted_active || (free_slot && !runtime.queued_item_ids.is_empty())
}

/// The user's pause transition with the typed reason on its event. A lost
/// revision race is retried at the next check, not forced.
fn pause_for_disk_low(
    store: &mut TaskStore,
    task_id: &str,
    expected_revision: u64,
    reason: &TaskPauseReason,
) -> bool {
    store
        .pause_task_with_reason(
            &StoreUpdateTaskRequest {
                task_id: task_id.to_owned(),
                expected_revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::PauseTask,
                    item_ids: vec![],
                },
                event_id: event_id(task_id, "disk-low-pause"),
                occurred_at: timestamp_now(),
            },
            reason,
        )
        .is_ok()
}

/// At the forwarded progress cadence of a writing item, before the Store
/// lock: the pause reason when the drive is below the floor, counting the
/// file's unwritten bytes when its size is known.
fn disk_reason_while_writing(
    disk: &mut CoordinatorDiskGuard,
    running: &HashMap<(String, String), RunningWorker>,
    progress: &DownloadItemProgress,
) -> Option<TaskPauseReason> {
    if progress.status != insar_core::DownloadItemStatus::Active {
        return None;
    }
    let worker = running.get(&(progress.task_id.clone(), progress.item_id.clone()))?;
    let remaining = progress
        .total_bytes
        .map(|total| total.saturating_sub(progress.bytes_completed));
    disk.verdict_while_writing(&progress.task_id, &worker.snapshot.output_root, remaining)
}

/// Under the Store lock: pause through the user's transition unless the task
/// is already paused.
fn pause_unless_paused(store: &mut TaskStore, task_id: &str, reason: &TaskPauseReason) -> bool {
    match store.load_scheduler_control_runtime(task_id) {
        Ok(Some(runtime)) if !runtime.task_paused => {
            pause_for_disk_low(store, task_id, runtime.revision, reason)
        }
        _ => false,
    }
}

/// Synchronize controls that are already live before deciding whether new work
/// may be claimed. The runtime-only Store query intentionally sees terminal
/// rows, while `load_scheduler_tasks` remains the separate scheduling authority.
fn reconcile_live_controls(
    store: &TaskStore,
    running: &mut HashMap<(String, String), RunningWorker>,
) -> Result<(), StoreError> {
    let task_ids: BTreeSet<String> = running.keys().map(|(task_id, _)| task_id.clone()).collect();
    let mut runtimes = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        runtimes.push((
            task_id.clone(),
            store.load_scheduler_control_runtime(&task_id)?,
        ));
    }

    for (task_id, runtime) in runtimes {
        match runtime {
            Some(runtime) => update_controls(&runtime, running),
            None => cancel_controls_for_task(&task_id, running),
        }
    }
    Ok(())
}

fn update_controls(
    runtime: &TaskRuntimeSnapshot,
    running: &mut HashMap<(String, String), RunningWorker>,
) {
    for ((task_id, item_id), worker) in running.iter() {
        if task_id == &runtime.task_id {
            worker
                .control
                .set_paused(runtime.task_paused || runtime.paused_item_ids.contains(item_id));
            if runtime.cancelled_item_ids.contains(item_id) {
                worker.control.cancel();
            }
        }
    }
}

fn cancel_controls_for_task(task_id: &str, running: &HashMap<(String, String), RunningWorker>) {
    for ((running_task_id, _), worker) in running {
        if running_task_id == task_id {
            worker.control.cancel();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_if_missing(
    task: &FrozenTaskSnapshot,
    item_id: &str,
    runtime_revision: u64,
    worker: &Arc<dyn DownloadWorker>,
    sender: &Sender<Message>,
    coordinator_stopping: &Arc<AtomicBool>,
    running: &mut HashMap<(String, String), RunningWorker>,
    finish_backoff: &UnrecordedFinishBackoff,
) {
    let key = (task.task_id.clone(), item_id.to_string());
    if running.contains_key(&key) || finish_backoff.holds(&key, Instant::now()) {
        return;
    }
    let Some(item) = task
        .candidates
        .iter()
        .find(|item| item.item_id == item_id)
        .cloned()
    else {
        return;
    };
    let control = WorkerControl::default();
    let worker_control = control.clone();
    let worker = Arc::clone(worker);
    let sender = sender.clone();
    let plan_freeze = NisarExecutionPlanFreezeAuthority {
        sender: sender.clone(),
        coordinator_stopping: Arc::clone(coordinator_stopping),
    };
    let cached_snapshot = task.clone();
    let work = DownloadWork {
        task: cached_snapshot.clone(),
        item,
    };
    let handle = thread::spawn(move || {
        let task_id = work.task.task_id.clone();
        let item_id = work.item.item_id.clone();
        let destination = work.task.output_root.clone();
        let mut progress_gate = ProgressForwardingGate::default();
        let mut report = |progress| {
            if progress_gate.should_forward(&progress, Instant::now()) {
                let _ = sender.send(Message::Progress(progress));
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            worker.execute_with_authority(work, worker_control, &plan_freeze, &mut report)
        }))
        .unwrap_or_else(|_| DownloadItemResult {
            task_id,
            item_id,
            outcome: DownloadOutcome::Failed,
            destination,
            bytes_written: 0,
            attempt_count: 0,
            failure: Some(DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "Rust 下载工作线程异常终止".to_string(),
                retryable: true,
            }),
            finished_at: timestamp_now(),
        });
        let _ = sender.send(Message::Finished(result));
    });
    running.insert(
        key,
        RunningWorker {
            control,
            handle,
            snapshot: cached_snapshot,
            runtime_revision,
        },
    );
}

fn persist_progress(store: &mut TaskStore, progress: &DownloadItemProgress) {
    let _ = store.save_scheduler_download_progress(progress);
}

fn persist_result(store: &mut TaskStore, result: &DownloadItemResult) {
    let _ = store.save_scheduler_download_result(result);
}

fn checkpoint_task_transfer_activity(
    store: &mut TaskStore,
    worker: &dyn DownloadWorker,
    task_id: &str,
    samples: &mut HashMap<String, TransferCheckpointSampleState>,
) -> Result<(), StoreError> {
    let Some(sample) = worker.task_transfer_sample(task_id) else {
        return Ok(());
    };
    let raw = (sample.active_millis, sample.network_bytes_received);
    // Anchor RPCs also visit this path. During non-network work the raw sample
    // is stable; do not hydrate the full task or rewrite identical telemetry.
    // Cache only successful commits, in the same coordinator lifetime as its
    // baseline, so failures/terminal retries and process restarts retain truth.
    if samples
        .get(task_id)
        .is_some_and(|state| state.last_successful_raw == Some(raw))
    {
        return Ok(());
    }
    let Some(expected_revision) = store.scheduler_transfer_checkpoint_revision(task_id)? else {
        samples.remove(task_id);
        return Ok(());
    };
    let state = match samples.entry(task_id.to_owned()) {
        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(TransferCheckpointSampleState {
                lifetime_baseline: store.task_transfer_checkpoint_baseline(task_id)?,
                last_successful_raw: None,
            })
        }
    };
    let baseline = state.lifetime_baseline;
    let active_millis = baseline
        .0
        .checked_add(sample.active_millis)
        .ok_or_else(|| StoreError::InvalidSnapshot {
            reason: "transfer time overflow".into(),
        })?;
    let received = baseline
        .1
        .checked_add(sample.network_bytes_received)
        .ok_or_else(|| StoreError::InvalidSnapshot {
            reason: "network byte counter overflow".into(),
        })?;
    store.save_task_transfer_telemetry_with_network(
        &StoreTaskTransferTelemetryUpdate {
            task_id: task_id.to_owned(),
            expected_revision,
            transfer_active_millis: active_millis,
            updated_at: timestamp_now(),
        },
        Some(received),
    )?;
    state.last_successful_raw = Some(raw);
    Ok(())
}

fn stored_task_for_updated_runtime(
    store: &TaskStore,
    cached_snapshot: Option<FrozenTaskSnapshot>,
    runtime: TaskRuntimeSnapshot,
) -> Option<StoredTask> {
    if let Some(snapshot) = cached_snapshot {
        let candidate_ids = snapshot
            .candidates
            .iter()
            .map(|item| item.item_id.as_str())
            .collect::<BTreeSet<_>>();
        if runtime
            .selected_item_ids
            .iter()
            .all(|item_id| candidate_ids.contains(item_id.as_str()))
        {
            return Some(StoredTask { snapshot, runtime });
        }
    }
    store.load_scheduler_task(&runtime.task_id).ok().flatten()
}

fn finish_budget_migration(
    store: &mut TaskStore,
    result: &DownloadItemResult,
    #[cfg_attr(not(test), allow(unused_variables))] control: Option<&WorkerControl>,
) -> Option<TaskRuntimeSnapshot> {
    match result.outcome {
        DownloadOutcome::Failed => {
            let request = |revision, label: &str| StoreUpdateTaskRequest {
                task_id: result.task_id.clone(),
                expected_revision: revision,
                command: StoreTaskCommand {
                    kind: TaskStateCommandKind::MarkFailed,
                    item_ids: vec![result.item_id.clone()],
                },
                event_id: event_id(&result.task_id, &format!("{}:{label}", result.item_id)),
                occurred_at: result.finished_at.clone(),
            };
            // The Store first retains the exact rows this local failure
            // replaces, then shows it, in one transaction. Review 2026-10-17
            // round 2, P2-1: a concurrent pause, cancellation or sibling
            // transition can advance the runtime revision after it was read.
            // That conflict is not a reason to fall back: re-read the revision
            // and retry once, so the Store applies the current state (a
            // task-level pause becomes PauseItems and replaces nothing).
            let finish_retained = |store: &mut TaskStore| {
                let revision = store
                    .load_scheduler_control_runtime(&result.task_id)
                    .ok()??
                    .revision;
                #[cfg(test)]
                if let Some(control) = control {
                    control.run_budget_migration_hook_for_test(
                        NisarBudgetMigrationStageForTest::RefusalRevisionRead,
                    );
                }
                Some(store.finish_nisar_budget_upgrade_rejection(
                    &request(revision, "budget-migration"),
                    result,
                ))
            };
            let outcome = match finish_retained(store)? {
                Err(StoreError::RevisionConflict) => finish_retained(store)?,
                outcome => outcome,
            };
            match outcome {
                Ok(runtime) => Some(runtime),
                // That transaction rolled back as a whole. Leaving the item
                // Active would let `schedule` dispatch the same migration again
                // in a loop, so finish it without the evidence: no download row
                // is replaced (the row the evidence should have retained stays
                // as it is), the diagnostic goes to the task event and a
                // concurrent pause or cancellation still wins.
                Err(error) => {
                    let fallback = unretained_rejection_result(result, &error);
                    let revision = store
                        .load_scheduler_control_runtime(&result.task_id)
                        .ok()??
                        .revision;
                    store
                        .finish_nisar_budget_upgrade_rejection_unretained(
                            &request(revision, "budget-migration-unretained"),
                            &fallback,
                        )
                        .ok()
                }
            }
        }
        DownloadOutcome::Interrupted | DownloadOutcome::Cancelled => finish_runtime(store, result),
        // A success while still guarded is an implementation invariant violation.
        DownloadOutcome::Completed | DownloadOutcome::Skipped => None,
    }
}

/// The failure recorded when the refusal evidence transaction failed. The
/// original local diagnostic is kept and the Store error is appended, so the
/// record never claims that evidence was retained; no download row is
/// overwritten in this case.
fn unretained_rejection_result(
    result: &DownloadItemResult,
    error: &StoreError,
) -> DownloadItemResult {
    let mut fallback = result.clone();
    if let Some(failure) = fallback.failure.as_mut() {
        failure.reason_zh = format!(
            "{}（本地存储未能写入拒绝证据，原下载记录未被覆盖 [store_error={}]）",
            failure.reason_zh,
            error.code()
        );
    }
    fallback
}

/// Whether an item whose worker just returned a terminal result is still
/// Active in the Store (or the runtime cannot be read): its result was not
/// recorded and dispatching it again at once would loop.
fn finished_item_still_active(store: &TaskStore, result: &DownloadItemResult) -> bool {
    match store.load_scheduler_control_runtime(&result.task_id) {
        Ok(Some(runtime)) => runtime.active_item_ids.contains(&result.item_id),
        Ok(None) => false,
        Err(_) => true,
    }
}

fn finish_runtime(
    store: &mut TaskStore,
    result: &DownloadItemResult,
) -> Option<TaskRuntimeSnapshot> {
    let Ok(Some(runtime)) = store.load_scheduler_control_runtime(&result.task_id) else {
        return None;
    };
    let terminal = runtime.completed_item_ids.contains(&result.item_id)
        || runtime.failed_item_ids.contains(&result.item_id)
        || runtime.skipped_item_ids.contains(&result.item_id)
        || runtime.cancelled_item_ids.contains(&result.item_id);
    if terminal {
        return None;
    }
    if result.outcome == DownloadOutcome::Interrupted {
        // Task-level pause sets task_paused while the active worker is still in
        // flight. When that worker cooperatively stops, make the durable item
        // state match the recoverable transfer/science checkpoint. Item-level
        // PauseItems has already performed this move before the worker exits.
        if runtime.paused_item_ids.contains(&result.item_id) {
            return None;
        }
        if runtime.task_paused && runtime.active_item_ids.contains(&result.item_id) {
            return store
                .update_scheduler_runtime(&StoreUpdateTaskRequest {
                    task_id: result.task_id.clone(),
                    expected_revision: runtime.revision,
                    command: StoreTaskCommand {
                        kind: TaskStateCommandKind::PauseItems,
                        item_ids: vec![result.item_id.clone()],
                    },
                    event_id: event_id(&result.task_id, &format!("{}:pause", result.item_id)),
                    occurred_at: result.finished_at.clone(),
                })
                .ok();
        }
        return None;
    }
    let kind = match result.outcome {
        DownloadOutcome::Completed => TaskStateCommandKind::MarkCompleted,
        DownloadOutcome::Skipped => TaskStateCommandKind::MarkSkipped,
        DownloadOutcome::Failed => TaskStateCommandKind::MarkFailed,
        DownloadOutcome::Cancelled => TaskStateCommandKind::CancelItems,
        DownloadOutcome::Interrupted => return None,
    };
    store
        .update_scheduler_runtime(&StoreUpdateTaskRequest {
            task_id: result.task_id.clone(),
            expected_revision: runtime.revision,
            command: StoreTaskCommand {
                kind,
                item_ids: vec![result.item_id.clone()],
            },
            event_id: event_id(&result.task_id, &result.item_id),
            occurred_at: result.finished_at.clone(),
        })
        .ok()
}

fn event_id(task_id: &str, label: &str) -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("engine:{task_id}:{label}:{nonce}")
}

pub(crate) fn timestamp_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX / 2);
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
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

#[cfg(all(test, feature = "nisar-subset-production"))]
#[path = "scheduler_budget_migration_tests.rs"]
mod budget_migration_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use insar_core::DownloadItemStatus;

    #[derive(Default)]
    struct RecordingTransferCheckpointDiagnostics {
        events: Mutex<Vec<TransferCheckpointDiagnostic>>,
    }

    impl TransferCheckpointDiagnosticSink for RecordingTransferCheckpointDiagnostics {
        fn record(&self, diagnostic: TransferCheckpointDiagnostic) {
            self.events.lock().unwrap().push(diagnostic);
        }
    }

    fn progress(status: DownloadItemStatus, completed: u64, total: u64) -> DownloadItemProgress {
        DownloadItemProgress {
            task_id: "task".to_owned(),
            item_id: "item".to_owned(),
            status,
            bytes_completed: completed,
            total_bytes: Some(total),
            bytes_per_second: 1.0,
            attempt: 1,
            updated_at: "2026-07-29T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn progress_forwarding_preserves_first_state_change_interval_and_terminal_updates() {
        let started = Instant::now();
        let mut gate = ProgressForwardingGate::default();
        assert!(gate.should_forward(&progress(DownloadItemStatus::Active, 1, 100), started));
        assert!(!gate.should_forward(
            &progress(DownloadItemStatus::Active, 2, 100),
            started + Duration::from_millis(10),
        ));
        assert!(gate.should_forward(
            &progress(DownloadItemStatus::Paused, 2, 100),
            started + Duration::from_millis(20),
        ));
        assert!(gate.should_forward(
            &progress(DownloadItemStatus::Active, 3, 100),
            started + Duration::from_millis(30),
        ));
        assert!(!gate.should_forward(
            &progress(DownloadItemStatus::Active, 4, 100),
            started + Duration::from_millis(280),
        ));
        assert!(gate.should_forward(
            &progress(DownloadItemStatus::Active, 5, 100),
            started + Duration::from_millis(1_030),
        ));
        assert!(gate.should_forward(
            &progress(DownloadItemStatus::Active, 100, 100),
            started + Duration::from_millis(1_031),
        ));
    }

    #[test]
    fn checkpoint_diagnostics_preserve_store_error_code_without_retry_spam() {
        let started = Instant::now();
        let diagnostics = RecordingTransferCheckpointDiagnostics::default();
        let mut checkpoints = TransferCheckpointTracker::default();
        checkpoints.request("private-task-id", false, started);
        checkpoints.observe_result(
            "private-task-id",
            Err(StoreError::Database(rusqlite::Error::InvalidQuery)),
            started,
            &diagnostics,
        );
        checkpoints.request(
            "private-task-id",
            true,
            started + TRANSFER_CHECKPOINT_RETRY_INITIAL,
        );
        checkpoints.observe_result(
            "private-task-id",
            Err(StoreError::Database(rusqlite::Error::InvalidQuery)),
            started + TRANSFER_CHECKPOINT_RETRY_INITIAL,
            &diagnostics,
        );
        checkpoints.observe_result(
            "private-task-id",
            Ok(()),
            started + Duration::from_millis(100),
            &diagnostics,
        );

        assert_eq!(
            *diagnostics.events.lock().unwrap(),
            vec![
                TransferCheckpointDiagnostic {
                    checkpoint_id: 1,
                    status: TransferCheckpointDiagnosticStatus::Failed,
                    error_code: "V3_STORE_DATABASE_ERROR",
                    terminal: false,
                    retry_count: 1,
                },
                TransferCheckpointDiagnostic {
                    checkpoint_id: 1,
                    status: TransferCheckpointDiagnosticStatus::Recovered,
                    error_code: "V3_STORE_DATABASE_ERROR",
                    terminal: true,
                    retry_count: 2,
                },
            ]
        );
        assert!(checkpoints.pending.is_empty());
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn stopped_worker_anchor_rpc_finishes_science_checkpoint_but_rejects_writer_anchor() {
        for cancelled in [false, true] {
            let (sender, receiver) = mpsc::channel();
            let authority = NisarExecutionPlanFreezeAuthority {
                sender,
                coordinator_stopping: Arc::new(AtomicBool::new(false)),
            };
            let control = WorkerControl::default();
            if cancelled {
                control.cancel();
            } else {
                control.set_paused(true);
            }
            assert!(matches!(
                authority.load_execution_anchor_v2(
                    "task",
                    "item",
                    insar_store::NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
                    &control,
                ),
                Err(NisarExecutionAnchorAuthorityError::ExecutionStopped)
            ));
            assert!(matches!(
                receiver.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            let (result_sender, result_receiver) = mpsc::sync_channel(1);
            let worker = thread::spawn(move || {
                result_sender
                    .send(authority.load_execution_anchor_v2(
                        "task",
                        "item",
                        insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
                        &control,
                    ))
                    .unwrap();
            });
            let response = match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
                Message::NisarExecutionAnchorLoad { response, .. } => response,
                _ => panic!("expected science checkpoint anchor RPC"),
            };
            // Cross multiple RPC polling intervals while the worker is stopped.
            assert!(matches!(
                result_receiver.recv_timeout(Duration::from_millis(150)),
                Err(RecvTimeoutError::Timeout)
            ));
            response.send(Ok(None)).unwrap();
            assert!(matches!(
                result_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .unwrap(),
                Ok(None)
            ));
            worker.join().unwrap();
        }
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn active_anchor_rpc_observes_scheduler_shutdown_without_coordinator_response() {
        let (sender, receiver) = mpsc::channel();
        let coordinator_stopping = Arc::new(AtomicBool::new(false));
        let authority = NisarExecutionPlanFreezeAuthority {
            sender,
            coordinator_stopping: Arc::clone(&coordinator_stopping),
        };
        let control = WorkerControl::default();
        let worker_control = control.clone();
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let result = authority.load_execution_anchor_v2(
                "task",
                "item",
                insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
                &worker_control,
            );
            result_sender.send(result).unwrap();
        });
        let pending_response = match receiver.recv_timeout(Duration::from_secs(1)).unwrap() {
            Message::NisarExecutionAnchorLoad {
                task_id,
                item_id,
                anchor_kind,
                response,
            } => {
                assert_eq!(task_id, "task");
                assert_eq!(item_id, "item");
                assert_eq!(
                    anchor_kind,
                    insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
                );
                response
            }
            _ => panic!("expected active execution-anchor RPC"),
        };
        coordinator_stopping.store(true, Ordering::SeqCst);
        assert!(matches!(
            result_receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
            Err(NisarExecutionAnchorAuthorityError::ExecutionStopped)
        ));
        drop(pending_response);
        worker.join().unwrap();
    }

    /// Review 2026-10-17 round 2, P1-2: only a revision conflict, a busy or
    /// locked database and an unavailable coordinator are transient. Every
    /// other Store refusal is terminal and keeps the Store's own code.
    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn science_anchor_store_refusals_keep_their_code_and_only_conflict_busy_or_coordinator_are_transient(
    ) {
        use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;
        let sqlite = |code| {
            StoreError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ))
        };
        let control = WorkerControl::default();
        for transient in [
            NisarExecutionAnchorAuthorityError::CoordinatorUnavailable,
            NisarExecutionAnchorAuthorityError::Store(StoreError::RevisionConflict),
            NisarExecutionAnchorAuthorityError::Store(sqlite(rusqlite::ffi::SQLITE_BUSY)),
            NisarExecutionAnchorAuthorityError::Store(sqlite(rusqlite::ffi::SQLITE_LOCKED)),
        ] {
            assert_eq!(
                science_anchor_authority_error(transient, &control),
                ScienceError::Persistence
            );
        }
        let corrupt = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        for (error, code) in [
            (
                StoreError::NisarBudgetUpgradeProofVersionUnsupported { found: 3 },
                "V3_STORE_NISAR_BUDGET_UPGRADE_PROOF_VERSION_UNSUPPORTED",
            ),
            (StoreError::Corrupt(corrupt), "V3_STORE_CORRUPT_ROW"),
            (StoreError::NotFound, "V3_STORE_NOT_FOUND"),
            (
                StoreError::InvalidSnapshot {
                    reason: "invalid".into(),
                },
                "V3_STORE_INVALID_SNAPSHOT",
            ),
            (
                sqlite(rusqlite::ffi::SQLITE_CONSTRAINT),
                "V3_STORE_DATABASE_ERROR",
            ),
        ] {
            let mapped = science_anchor_authority_error(
                NisarExecutionAnchorAuthorityError::Store(error),
                &control,
            );
            assert_eq!(
                mapped,
                ScienceError::StoreRecordRejected { store_code: code }
            );
            assert_eq!(mapped.code(), code);
        }
        assert_eq!(
            science_anchor_authority_error(
                NisarExecutionAnchorAuthorityError::Store(
                    StoreError::NisarSubsetExecutionAnchorInvalid
                ),
                &control,
            ),
            ScienceError::RecoveryInvalid
        );
        let paused = WorkerControl::default();
        paused.set_paused(true);
        assert_eq!(
            science_anchor_authority_error(
                NisarExecutionAnchorAuthorityError::ExecutionStopped,
                &paused
            ),
            ScienceError::Paused
        );
        let cancelled = WorkerControl::default();
        cancelled.cancel();
        assert_eq!(
            science_anchor_authority_error(
                NisarExecutionAnchorAuthorityError::ExecutionStopped,
                &cancelled
            ),
            ScienceError::Cancelled
        );
    }

    /// Review 2026-10-17 round 3, P2-2: a full disk, an I/O error, a database
    /// file SQLite cannot open and a file-system error clear once disk space is
    /// freed or the disk is repaired, so they are transient with a local cause.
    /// Corruption and schema or version refusals stay terminal with their code.
    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn science_anchor_local_storage_failures_are_transient_and_corruption_or_schema_stay_terminal()
    {
        use crate::nisar_science_transfer_v2::NisarSubsetScienceTransferErrorV2 as ScienceError;
        let sqlite = |code| {
            StoreError::Database(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ))
        };
        let control = WorkerControl::default();
        for (local, store_code, cause) in [
            (
                sqlite(rusqlite::ffi::SQLITE_FULL),
                "V3_STORE_DATABASE_ERROR",
                "disk_full",
            ),
            (
                sqlite(rusqlite::ffi::SQLITE_IOERR),
                "V3_STORE_DATABASE_ERROR",
                "io_error",
            ),
            (
                sqlite(rusqlite::ffi::SQLITE_IOERR_WRITE),
                "V3_STORE_DATABASE_ERROR",
                "io_error",
            ),
            (
                sqlite(rusqlite::ffi::SQLITE_CANTOPEN),
                "V3_STORE_DATABASE_ERROR",
                "cannot_open",
            ),
            (
                StoreError::Filesystem(std::io::Error::other("injected file-system failure")),
                "V3_STORE_FILESYSTEM_ERROR",
                "filesystem",
            ),
        ] {
            let mapped = science_anchor_authority_error(
                NisarExecutionAnchorAuthorityError::Store(local),
                &control,
            );
            assert!(
                !matches!(mapped, ScienceError::StoreRecordRejected { .. }),
                "{mapped:?}"
            );
            assert_eq!(
                mapped.code(),
                ScienceError::Persistence.code(),
                "{mapped:?}"
            );
            assert_eq!(
                mapped,
                ScienceError::LocalStorageFailure { store_code, cause }
            );
        }
        for (error, code) in [
            (
                sqlite(rusqlite::ffi::SQLITE_CORRUPT),
                "V3_STORE_DATABASE_ERROR",
            ),
            (
                sqlite(rusqlite::ffi::SQLITE_NOTADB),
                "V3_STORE_DATABASE_ERROR",
            ),
            (
                StoreError::NewerSchema {
                    found: 31,
                    supported: 30,
                },
                "V3_STORE_SCHEMA_NEWER",
            ),
            (
                StoreError::UnsupportedSnapshotVersion,
                "V3_STORE_UNSUPPORTED_SNAPSHOT_VERSION",
            ),
        ] {
            assert_eq!(
                science_anchor_authority_error(
                    NisarExecutionAnchorAuthorityError::Store(error),
                    &control,
                ),
                ScienceError::StoreRecordRejected { store_code: code }
            );
        }
    }

    /// Review 2026-10-17 round 2, P2-2: the end of a transfer (a zero rate after
    /// a forwarded positive rate) is forwarded at once, so the task card leaves
    /// the transfer phase without waiting for the next throttle interval.
    #[test]
    fn progress_gate_forwards_a_stopped_transfer_at_once() {
        let progress = |bytes_completed, bytes_per_second| DownloadItemProgress {
            task_id: "task".to_owned(),
            item_id: "item".to_owned(),
            status: insar_core::DownloadItemStatus::Active,
            bytes_completed,
            total_bytes: None,
            bytes_per_second,
            attempt: 1,
            updated_at: "2026-10-17T00:00:00Z".to_owned(),
        };
        let mut gate = ProgressForwardingGate::default();
        let start = Instant::now();
        assert!(gate.should_forward(&progress(10, 5.0), start));
        assert!(
            !gate.should_forward(&progress(20, 5.0), start + Duration::from_millis(100)),
            "throttled"
        );
        assert!(
            gate.should_forward(&progress(20, 0.0), start + Duration::from_millis(200)),
            "a stopped transfer is forwarded at once"
        );
        assert!(
            !gate.should_forward(&progress(20, 0.0), start + Duration::from_millis(300)),
            "an unchanged idle state stays throttled"
        );
    }

    #[test]
    fn checkpoint_retry_backoff_is_exponential_and_capped() {
        assert_eq!(
            transfer_checkpoint_retry_delay(1),
            TRANSFER_CHECKPOINT_RETRY_INITIAL
        );
        assert_eq!(
            transfer_checkpoint_retry_delay(2),
            Duration::from_millis(50)
        );
        assert_eq!(
            transfer_checkpoint_retry_delay(u32::MAX),
            TRANSFER_CHECKPOINT_RETRY_MAX
        );
    }
}
