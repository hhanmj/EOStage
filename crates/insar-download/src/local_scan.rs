//! Downloader D3-2 (batch pre-scan): the coordinator's local scan of queued
//! items. A scan job decides each queued item through
//! [`DownloadWorker::scan_local`] (local reads only: no credentials,
//! resolution or network admission) and reports back to the coordinator,
//! the only Store writer: reusable items are committed as skipped in
//! batches, every other item is released to its worker. Until released, a
//! queued item is not claimed, so a reusable item never takes a worker,
//! a concurrency slot or a network permit. Scan state lives only in the
//! coordinator: a restart scans again.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use insar_core::{FrozenTaskSnapshot, SnapshotItem, TaskRuntimeSnapshot};
use insar_store::StoreLocalReuseBatchItem;

use crate::scheduler::NisarExecutionPlanFreezeAuthority;

/// Delivers a scan report to the coordinator.
pub(crate) type LocalScanReporter = Arc<dyn Fn(LocalScanReport) + Send + Sync>;
use crate::{DownloadWork, DownloadWorker, LocalScanPhase, LocalScanVerdict, WorkerControl};

/// Scan jobs running at once in one process. One job keeps the claim order
/// of the scheduler's task order: tasks are scanned in the order the
/// scheduler visits them, so with a destination shared across tasks the
/// earlier task still claims first (as before the pre-scan). A scan reads
/// only stat, the ledger and bounded format structures; digests are not
/// computed by this job.
const MAX_SCAN_JOBS: usize = 1;
/// A report is sent after this many decisions (the Store batch limit) …
const REPORT_ITEMS: usize = insar_store::STORE_LOCAL_REUSE_BATCH_LIMIT;
/// … or this long after the previous report.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// Which job sent a report: the first-phase scan of one task, or the
/// process-wide digest job of the second phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalScanJobKind {
    Scan,
    Digest,
}

/// How a scan job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalScanEnd {
    Done,
    /// Paused, cancelled or stopped with the coordinator.
    Stopped,
}

/// One report of a scan job to the coordinator.
#[derive(Debug)]
pub(crate) struct LocalScanReport {
    pub(crate) task_id: String,
    pub(crate) kind: LocalScanJobKind,
    pub(crate) released: Vec<String>,
    pub(crate) reused: Vec<StoreLocalReuseBatchItem>,
    /// Items the first phase left for a digest; they stay unreleased.
    pub(crate) needs_digest: Vec<String>,
    /// First-phase progress (the digest job leaves both at zero).
    pub(crate) scanned: u64,
    pub(crate) total: u64,
    pub(crate) end: Option<LocalScanEnd>,
}

struct ScanJob {
    control: WorkerControl,
    handle: JoinHandle<()>,
}

#[derive(Default)]
struct TaskScan {
    released: HashSet<String>,
    /// Decided by the first phase to need a digest; not yet hashed.
    awaiting_digest: HashSet<String>,
    job: Option<ScanJob>,
}

/// The coordinator's scan state for every task.
pub(crate) struct LocalScans {
    enabled: bool,
    tasks: HashMap<String, TaskScan>,
    reporter: LocalScanReporter,
    /// The second phase: one digest job for the whole process (a stricter
    /// form of the plan's one token per volume), so hashing large files
    /// never runs in parallel and never holds a scan or worker slot.
    digest: Option<(String, ScanJob)>,
}

impl LocalScans {
    pub(crate) fn new(enabled: bool, reporter: LocalScanReporter) -> Self {
        Self {
            enabled,
            tasks: HashMap::new(),
            reporter,
            digest: None,
        }
    }

    fn running_jobs(&self) -> usize {
        self.tasks
            .values()
            .filter(|task| task.job.is_some())
            .count()
    }

    /// The queued items of `runtime` that may be claimed now, in queue order:
    /// all of them without scanning, otherwise the released ones. Starts a
    /// scan job for the unreleased items when none runs for the task and a
    /// job slot is free; returns how many items it scans, if it started one.
    pub(crate) fn claimable(
        &mut self,
        task: &FrozenTaskSnapshot,
        runtime: &TaskRuntimeSnapshot,
        worker: &Arc<dyn DownloadWorker>,
        authority: NisarExecutionPlanFreezeAuthority,
    ) -> (Vec<String>, Option<u64>) {
        if !self.enabled {
            return (runtime.queued_item_ids.clone(), None);
        }
        let running_jobs = self.running_jobs();
        let reporter = Arc::clone(&self.reporter);
        let state = self.tasks.entry(task.task_id.clone()).or_default();
        let unreleased: Vec<&String> = runtime
            .queued_item_ids
            .iter()
            .filter(|item_id| {
                !state.released.contains(*item_id) && !state.awaiting_digest.contains(*item_id)
            })
            .collect();
        let mut started = None;
        if !unreleased.is_empty() && state.job.is_none() && running_jobs < MAX_SCAN_JOBS {
            let items: Vec<SnapshotItem> = unreleased
                .iter()
                .filter_map(|item_id| {
                    task.candidates
                        .iter()
                        .find(|candidate| &candidate.item_id == *item_id)
                        .cloned()
                })
                .collect();
            let total = items.len() as u64;
            let control = WorkerControl::default();
            let job_control = control.clone();
            let worker = Arc::clone(worker);
            // The scan reads the task's plan only: no candidate list.
            let mut light = task.clone();
            light.candidates.clear();
            let job_authority = authority.clone();
            let job_reporter = Arc::clone(&reporter);
            let handle = thread::spawn(move || {
                run_scan_job(
                    LocalScanJobKind::Scan,
                    &*worker,
                    light,
                    items,
                    &job_control,
                    &job_authority,
                    &*job_reporter,
                );
            });
            state.job = Some(ScanJob { control, handle });
            started = Some(total);
        }
        let claimable: Vec<String> = runtime
            .queued_item_ids
            .iter()
            .filter(|item_id| state.released.contains(*item_id))
            .cloned()
            .collect();
        let awaiting: Vec<SnapshotItem> = runtime
            .queued_item_ids
            .iter()
            .filter(|item_id| {
                state.awaiting_digest.contains(*item_id) && !state.released.contains(*item_id)
            })
            .filter_map(|item_id| {
                task.candidates
                    .iter()
                    .find(|candidate| &candidate.item_id == item_id)
                    .cloned()
            })
            .collect();
        if self.digest.is_none() && !awaiting.is_empty() {
            let control = WorkerControl::default();
            let job_control = control.clone();
            let worker = Arc::clone(worker);
            let mut light = task.clone();
            light.candidates.clear();
            let handle = thread::spawn(move || {
                run_scan_job(
                    LocalScanJobKind::Digest,
                    &*worker,
                    light,
                    awaiting,
                    &job_control,
                    &authority,
                    &*reporter,
                );
            });
            self.digest = Some((task.task_id.clone(), ScanJob { control, handle }));
        }
        (claimable, started)
    }

    /// Applies a job report: released items may be claimed, items needing a
    /// digest wait for the digest job, decided items leave that wait; an
    /// ended job frees its slot (a later schedule continues what is left).
    pub(crate) fn report(&mut self, report: &LocalScanReport, released: Vec<String>) {
        let state = self.tasks.entry(report.task_id.clone()).or_default();
        for item_id in released
            .iter()
            .chain(report.reused.iter().map(|item| &item.item_id))
        {
            state.awaiting_digest.remove(item_id);
        }
        state
            .awaiting_digest
            .extend(report.needs_digest.iter().cloned());
        state.released.extend(released);
        if report.end.is_none() {
            return;
        }
        match report.kind {
            LocalScanJobKind::Scan => {
                if let Some(job) = state.job.take() {
                    let _ = job.handle.join();
                }
            }
            LocalScanJobKind::Digest => {
                if self
                    .digest
                    .as_ref()
                    .is_some_and(|(task_id, _)| *task_id == report.task_id)
                {
                    if let Some((_, job)) = self.digest.take() {
                        let _ = job.handle.join();
                    }
                }
            }
        }
    }

    /// Whether the task's scan still has work: a running job of either
    /// phase, or items waiting for a digest.
    pub(crate) fn scan_pending(&self, task_id: &str) -> bool {
        self.tasks
            .get(task_id)
            .is_some_and(|state| state.job.is_some() || !state.awaiting_digest.is_empty())
            || self
                .digest
                .as_ref()
                .is_some_and(|(digest_task, _)| digest_task == task_id)
    }

    /// Stops the scan of a paused task; decisions already made are kept.
    pub(crate) fn pause(&mut self, task_id: &str) {
        if let Some(job) = self.tasks.get(task_id).and_then(|task| task.job.as_ref()) {
            job.control.set_paused(true);
        }
        if let Some((digest_task, job)) = &self.digest {
            if digest_task == task_id {
                job.control.set_paused(true);
            }
        }
    }

    /// Forgets tasks the scheduler no longer sees, stopping their jobs.
    pub(crate) fn retain(&mut self, live: &HashSet<String>) {
        if let Some((digest_task, job)) = &self.digest {
            if !live.contains(digest_task) {
                job.control.cancel();
                self.digest = None;
            }
        }
        self.tasks.retain(|task_id, state| {
            let keep = live.contains(task_id);
            if !keep {
                if let Some(job) = &state.job {
                    job.control.cancel();
                }
            }
            keep
        });
    }

    /// Stops every job (coordinator shutdown).
    pub(crate) fn shutdown(&mut self) {
        if let Some((_, job)) = self.digest.take() {
            job.control.cancel();
            let _ = job.handle.join();
        }
        for state in self.tasks.values_mut() {
            if let Some(job) = state.job.take() {
                job.control.cancel();
                let _ = job.handle.join();
            }
        }
    }
}

fn run_scan_job(
    kind: LocalScanJobKind,
    worker: &dyn DownloadWorker,
    task: FrozenTaskSnapshot,
    items: Vec<SnapshotItem>,
    control: &WorkerControl,
    authority: &NisarExecutionPlanFreezeAuthority,
    reporter: &(dyn Fn(LocalScanReport) + Send + Sync),
) {
    let (phase, total) = match kind {
        LocalScanJobKind::Scan => (LocalScanPhase::Cheap, items.len() as u64),
        LocalScanJobKind::Digest => (LocalScanPhase::Digest, 0),
    };
    let mut pending = PendingReport::default();
    let mut scanned = 0_u64;
    let mut last_report = Instant::now();
    let send = |pending: &mut PendingReport, scanned: u64, end: Option<LocalScanEnd>| {
        let pending = std::mem::take(pending);
        reporter(LocalScanReport {
            task_id: task.task_id.clone(),
            kind,
            released: pending.released,
            reused: pending.reused,
            needs_digest: pending.needs_digest,
            scanned: if kind == LocalScanJobKind::Scan {
                scanned
            } else {
                0
            },
            total,
            end,
        });
    };
    for item in items {
        if control.is_cancelled() || control.is_paused() {
            send(&mut pending, scanned, Some(LocalScanEnd::Stopped));
            return;
        }
        let item_id = item.item_id.clone();
        let work = DownloadWork {
            task: task.clone(),
            item,
        };
        // A scan that panics releases its item: the worker's gate decides.
        let verdict = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            worker.scan_local(&work, phase, control, Some(authority))
        }))
        .unwrap_or(LocalScanVerdict::Release);
        match verdict {
            LocalScanVerdict::Reuse(batch_item) => pending.reused.push(*batch_item),
            // The first phase leaves a digest to the digest job.
            LocalScanVerdict::NeedsDigest if kind == LocalScanJobKind::Scan => {
                pending.needs_digest.push(item_id);
            }
            LocalScanVerdict::Release | LocalScanVerdict::NeedsDigest => {
                pending.released.push(item_id);
            }
            LocalScanVerdict::Stopped => {
                send(&mut pending, scanned, Some(LocalScanEnd::Stopped));
                return;
            }
        }
        scanned += 1;
        if pending.len() >= REPORT_ITEMS || last_report.elapsed() >= REPORT_INTERVAL {
            send(&mut pending, scanned, None);
            last_report = Instant::now();
        }
    }
    send(&mut pending, scanned, Some(LocalScanEnd::Done));
}

/// Decisions not yet reported. A report holds at most REPORT_ITEMS of them,
/// so its reusable items always fit one Store batch.
#[derive(Default)]
struct PendingReport {
    released: Vec<String>,
    reused: Vec<StoreLocalReuseBatchItem>,
    needs_digest: Vec<String>,
}

impl PendingReport {
    fn len(&self) -> usize {
        self.released.len() + self.reused.len() + self.needs_digest.len()
    }
}
