//! Downloader enhancement D1 (2026-10-18): admission of the NISAR V2 writer.
//!
//! L4 round 3 ran one task with concurrency 2 whose items share one output
//! directory. One non-blocking lock covered the whole directory and was taken
//! only after about ten full hashes of the science material, so the second
//! writer failed with NISAR_SUBSET_V2_OUTPUT_BUSY and the scheduler marked the
//! item failed. Admission now happens at the writer entry, before any material
//! hash, in one lock order for every writer:
//!
//! 1. the process-local destination lease of the artifact (waitable, FIFO);
//! 2. the cross-process lock of the artifact,
//!    `.eostage-locks/nisar-subset-writer-v2-<artifact id>.lock`, keyed like
//!    its candidate `.nisar-subset-writer-v2-<artifact id>.writing` (polled).
//!    The hidden `.eostage-locks` directory of the output root keeps lock
//!    files out of the user's artifacts and is the meeting point of every
//!    profile and build writing into that root. Lock files are never removed:
//!    on Unix a removed lock file would let a later writer lock a new file
//!    while the holder still holds the old one;
//! 3. a shared lock on the directory lock of earlier builds,
//!    `.nisar-subset-writer-v2.lock` in the output root (polled): an earlier
//!    build takes that file exclusively while it writes, so it never writes
//!    beside this writer, and writers of this build do not serialize on it.
//!    Temporary restriction (review of D1/D2, downloader enhancement D1b): the
//!    file stays in the output root because earlier builds (before D1,
//!    5a46a8e3) look for it there. It is needed only while an earlier build
//!    may still write into an output root; exit when every task an earlier
//!    build created has finished, at the latest with the next major version,
//!    whose planning reviews it;
//! 4. one process-wide output slot (default 1, configurable with
//!    `EOSTAGE_NISAR_WRITER_SLOTS` from 1 to 8), granted in arrival order and
//!    separate from the network admission of transfers.
//!
//! Writers of different artifacts therefore only share the output slots, and
//! writers of one artifact serialize. Every wait observes pause and
//! cancellation within one poll interval and returns `Cancelled`, which the
//! worker reports as a pause, never as a failure. The caller is told once when
//! a wait starts (and why) and once when the writer is admitted after a wait,
//! so the task card can show 等待写出. A writer that has waited longer than
//! its threshold (5 minutes) is told once more, with the reason it waits at
//! that moment and the holder's process id when it can be read (review of
//! D1/D2, downloader enhancement D1b): a hung program or an earlier build
//! holding the output is then recorded instead of only showing 等待写出.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::destination_lease::{
    DestinationLeaseController, DestinationLeaseError, DestinationLeaseGuard,
};
use crate::hdf5_range_transfer::{
    acquire_hdf5_workspace_lock, try_acquire_hdf5_workspace_lock_shared, Hdf5RangeTransferError,
    Hdf5WorkspaceLockV1,
};
use crate::pinned_directory::PinnedDirectory;
use crate::NisarSubsetWriteError;

/// Writers producing output at the same time in one process, unless
/// `EOSTAGE_NISAR_WRITER_SLOTS` configures another number.
pub(crate) const NISAR_WRITER_V2_DEFAULT_OUTPUT_SLOTS: usize = 1;
/// The largest configurable number of output slots.
pub(crate) const NISAR_WRITER_V2_MAX_OUTPUT_SLOTS: usize = 8;
/// The environment setting of the number of output slots (review of D1/D2,
/// downloader enhancement D1b): an integer from 1 to 8, read when the
/// production worker is created; there is no UI setting.
pub(crate) const NISAR_WRITER_V2_OUTPUT_SLOTS_ENV: &str = "EOSTAGE_NISAR_WRITER_SLOTS";
/// The directory lock of earlier builds (one lock for the whole directory).
pub(crate) const EARLIER_BUILD_DIRECTORY_LOCK_NAME: &str = ".nisar-subset-writer-v2.lock";
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);
const SLOT_WAIT_INTERVAL: Duration = Duration::from_millis(50);

/// How long a writer waits before it records why it waits.
pub(crate) const NISAR_WRITER_V2_LONG_WAIT: Duration = Duration::from_secs(5 * 60);

/// Why a writer waits for its output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarWriterWaitReasonV2 {
    /// Another writer of this process produces the same artifact (the
    /// destination lease).
    SameArtifactInProcess,
    /// Another process holds the lock of the artifact.
    SameArtifactOtherProcess,
    /// An earlier build writes into the output directory (its directory lock).
    EarlierBuild,
    /// Every output slot of this process is in use.
    OutputSlot,
}

impl NisarWriterWaitReasonV2 {
    /// The reason in the local diagnostic line.
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::SameArtifactInProcess => "same_artifact_in_process",
            Self::SameArtifactOtherProcess => "same_artifact_other_process",
            Self::EarlierBuild => "earlier_build",
            Self::OutputSlot => "output_slot",
        }
    }
}

/// What the caller is told about admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarWriterAdmissionEventV2 {
    /// The writer started waiting, and why.
    Waiting { reason: NisarWriterWaitReasonV2 },
    /// The writer has waited longer than its threshold (once per admission):
    /// why it waits now, the holder's process id when it could be read, and
    /// how long it has waited.
    LongWait {
        reason: NisarWriterWaitReasonV2,
        holder_pid: Option<u32>,
        waited: Duration,
    },
    /// The writer was admitted after waiting.
    Admitted,
}

/// The hidden directory of the output root that holds the artifact locks.
pub(crate) const NISAR_WRITER_LOCK_DIRECTORY_NAME: &str = ".eostage-locks";

/// The cross-process lock of one artifact inside the lock directory, keyed
/// like its candidate.
pub(crate) fn nisar_writer_artifact_lock_name_v2(derived_artifact_id: &str) -> String {
    format!("nisar-subset-writer-v2-{derived_artifact_id}.lock")
}

/// The holder note next to an artifact lock: the process id of the writer that
/// last took the lock. A waiter reads it to name the holder; on Windows a
/// locked file cannot be read by another handle.
fn nisar_writer_holder_note_name_v2(derived_artifact_id: &str) -> String {
    format!("nisar-subset-writer-v2-{derived_artifact_id}.holder")
}

/// Records this process as the holder (best effort: only a diagnostic). The
/// note is opened like a lock file, without following a link and refusing a
/// file with another name (a hard link), so it never writes through to
/// another file (review of D1b, downloader enhancement D1c, P3).
fn write_holder_note(path: &Path) {
    let _ = crate::hdf5_range_transfer::write_note_nofollow(
        path,
        &format!("pid={}\n", std::process::id()),
    );
}

/// The process id in a holder note (`pid=<id>`) or in an earlier build's lock
/// file (`<id>`), when the file can be read.
fn read_holder_pid(path: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    let value = text.trim();
    value.strip_prefix("pid=").unwrap_or(value).parse().ok()
}

/// Tells the observer when a wait starts and, once, when it has lasted longer
/// than the threshold.
struct WaitTracker<'a> {
    observer: &'a mut dyn FnMut(NisarWriterAdmissionEventV2),
    long_wait_after: Duration,
    started: Option<Instant>,
    long_wait_reported: bool,
}

impl WaitTracker<'_> {
    /// Called before every wait, outside any lock.
    fn waiting(&mut self, reason: NisarWriterWaitReasonV2, holder_pid: &dyn Fn() -> Option<u32>) {
        let now = Instant::now();
        let started = match self.started {
            Some(started) => started,
            None => {
                self.started = Some(now);
                (self.observer)(NisarWriterAdmissionEventV2::Waiting { reason });
                now
            }
        };
        let waited = now.saturating_duration_since(started);
        if !self.long_wait_reported && waited >= self.long_wait_after {
            self.long_wait_reported = true;
            (self.observer)(NisarWriterAdmissionEventV2::LongWait {
                reason,
                holder_pid: holder_pid(),
                waited,
            });
        }
    }

    fn admitted(&mut self) {
        if self.started.is_some() {
            (self.observer)(NisarWriterAdmissionEventV2::Admitted);
        }
    }
}

/// Opens (creating it if needed) the lock directory of `output_root`. A link
/// or junction named like it must not move the locks elsewhere: the directory
/// must be the real child `<output root>/.eostage-locks`. The unresolved
/// path is checked first, and the opened (canonical) directory must be
/// exactly that child, so a junction to a sibling inside the output root is
/// refused as well (review of D1b, downloader enhancement D1c, P3).
fn open_lock_directory(
    output_root: &PinnedDirectory,
) -> Result<PinnedDirectory, NisarSubsetWriteError> {
    let path = output_root
        .join_component(NISAR_WRITER_LOCK_DIRECTORY_NAME)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(NisarSubsetWriteError::OutputUnsafe);
        }
    }
    let directory =
        PinnedDirectory::create(&path).map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    if directory.path() != output_root.path().join(NISAR_WRITER_LOCK_DIRECTORY_NAME) {
        return Err(NisarSubsetWriteError::OutputUnsafe);
    }
    hide_lock_directory(directory.path());
    Ok(directory)
}

/// Marks the lock directory hidden on Windows (best effort; the leading dot
/// hides it elsewhere).
#[cfg(windows)]
fn hide_lock_directory(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, INVALID_FILE_ATTRIBUTES,
    };
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives both calls.
    unsafe {
        let attributes = GetFileAttributesW(wide.as_ptr());
        if attributes != INVALID_FILE_ATTRIBUTES && attributes & FILE_ATTRIBUTE_HIDDEN == 0 {
            SetFileAttributesW(wide.as_ptr(), attributes | FILE_ATTRIBUTE_HIDDEN);
        }
    }
}

#[cfg(not(windows))]
fn hide_lock_directory(_path: &Path) {}

/// The configured number of output slots: absent is the default, an integer
/// from 1 to 8 is used as it is, anything else is refused with a message.
pub(crate) fn parse_nisar_writer_output_slots(value: Option<&str>) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(NISAR_WRITER_V2_DEFAULT_OUTPUT_SLOTS);
    };
    match value.trim().parse::<usize>() {
        Ok(slots) if (1..=NISAR_WRITER_V2_MAX_OUTPUT_SLOTS).contains(&slots) => Ok(slots),
        _ => Err(format!(
            "{NISAR_WRITER_V2_OUTPUT_SLOTS_ENV} must be an integer from 1 to \
             {NISAR_WRITER_V2_MAX_OUTPUT_SLOTS}"
        )),
    }
}

/// The output slots of this process from `EOSTAGE_NISAR_WRITER_SLOTS`. An
/// invalid value falls back to the default and is reported once on the local
/// diagnostic stream.
pub(crate) fn nisar_writer_output_slots_from_environment() -> Arc<NisarWriterOutputSlotsV2> {
    let value = match std::env::var(NISAR_WRITER_V2_OUTPUT_SLOTS_ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => Some(String::new()),
    };
    let capacity = parse_nisar_writer_output_slots(value.as_deref()).unwrap_or_else(|message| {
        eprintln!(
            "NISAR_SUBSET_V2_WRITER_SLOTS_INVALID {message}; using \
             {NISAR_WRITER_V2_DEFAULT_OUTPUT_SLOTS}"
        );
        NISAR_WRITER_V2_DEFAULT_OUTPUT_SLOTS
    });
    NisarWriterOutputSlotsV2::new(capacity)
}

/// Process-wide output slots of the NISAR V2 writer: how many writers produce
/// output at the same time. Granted in arrival order (review of D1/D2,
/// downloader enhancement D1b): a writer arriving when a slot frees up queues
/// behind the writers already waiting. Separate from the network admission of
/// transfers.
#[derive(Debug)]
pub(crate) struct NisarWriterOutputSlotsV2 {
    capacity: usize,
    state: Mutex<OutputSlotState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct OutputSlotState {
    in_use: usize,
    next_ticket: u64,
    /// Tickets of the waiting writers in arrival order.
    queue: VecDeque<u64>,
}

impl NisarWriterOutputSlotsV2 {
    /// A capacity of zero is treated as one: a writer always gets a slot.
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            capacity: capacity.max(1),
            state: Mutex::new(OutputSlotState::default()),
            changed: Condvar::new(),
        })
    }

    fn state(&self) -> MutexGuard<'_, OutputSlotState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a slot in arrival order. `waiting` is called before every wait,
    /// outside the lock; `stopped` ends this writer's wait alone and frees its
    /// place in the queue. The place is also freed when `waiting` or `stopped`
    /// panics (review of D1b, downloader enhancement D1c): the scheduler catches
    /// a worker panic and the process continues, so a ticket left at the head
    /// of the queue would block every later writer forever.
    fn acquire(
        self: &Arc<Self>,
        stopped: &dyn Fn() -> bool,
        waiting: &mut dyn FnMut(),
    ) -> Result<NisarWriterOutputSlotGuardV2, NisarSubsetWriteError> {
        let ticket = {
            let mut state = self.state();
            let ticket = state.next_ticket;
            state.next_ticket = state.next_ticket.wrapping_add(1);
            state.queue.push_back(ticket);
            ticket
        };
        // Declared before the state guard, so on any early return or unwind
        // the guard unlocks first and the ticket then leaves the queue.
        let mut queued = QueuedTicket {
            slots: self,
            ticket: Some(ticket),
        };
        let mut state = self.state();
        loop {
            if stopped() {
                drop(state);
                drop(queued);
                return Err(NisarSubsetWriteError::Cancelled);
            }
            if state.queue.front() == Some(&ticket) && state.in_use < self.capacity {
                state.queue.pop_front();
                state.in_use += 1;
                queued.ticket = None;
                drop(state);
                // The next waiter may fit into another free slot.
                self.changed.notify_all();
                return Ok(NisarWriterOutputSlotGuardV2 {
                    slots: Arc::clone(self),
                });
            }
            drop(state);
            waiting();
            state = self.state();
            if state.queue.front() == Some(&ticket) && state.in_use < self.capacity {
                continue;
            }
            state = self
                .changed
                .wait_timeout(state, SLOT_WAIT_INTERVAL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The configured number of slots.
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// Slots in use (test observation only).
    #[cfg(test)]
    pub(crate) fn in_use_for_test(&self) -> usize {
        self.state().in_use
    }

    /// Takes one slot as another writer would (test only).
    #[cfg(test)]
    pub(crate) fn hold_for_test(self: &Arc<Self>) -> NisarWriterOutputSlotGuardV2 {
        self.acquire(&|| false, &mut || {})
            .expect("an unstopped slot acquisition succeeds")
    }
}

/// The place of one waiting writer in the queue, removed when dropped unless
/// the writer was granted a slot.
struct QueuedTicket<'a> {
    slots: &'a NisarWriterOutputSlotsV2,
    ticket: Option<u64>,
}

impl Drop for QueuedTicket<'_> {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket.take() {
            let mut state = self.slots.state();
            state.queue.retain(|queued| *queued != ticket);
            drop(state);
            self.slots.changed.notify_all();
        }
    }
}

/// One output slot, released when dropped.
#[derive(Debug)]
pub(crate) struct NisarWriterOutputSlotGuardV2 {
    slots: Arc<NisarWriterOutputSlotsV2>,
}

impl Drop for NisarWriterOutputSlotGuardV2 {
    fn drop(&mut self) {
        let mut state = self.slots.state();
        state.in_use = state.in_use.saturating_sub(1);
        drop(state);
        self.slots.changed.notify_all();
    }
}

/// Everything one admitted writer holds until it returns. Fields drop in
/// declaration order, the reverse of the acquisition order.
pub(crate) struct NisarWriterAdmissionV2 {
    _slot: NisarWriterOutputSlotGuardV2,
    _earlier_build_lock: Hdf5WorkspaceLockV1,
    _artifact_lock: Hdf5WorkspaceLockV1,
    _lock_directory: PinnedDirectory,
    _lease: DestinationLeaseGuard,
}

/// Admits one writer of `derived_artifact_id` into `output_root` (see the
/// module documentation for the order). `stopped` is polled by every wait; a
/// wait longer than `long_wait_after` is reported once.
pub(crate) fn admit_nisar_writer_v2(
    output_root: &PinnedDirectory,
    destination_lease: &Arc<DestinationLeaseController>,
    slots: &Arc<NisarWriterOutputSlotsV2>,
    derived_artifact_id: &str,
    long_wait_after: Duration,
    stopped: &dyn Fn() -> bool,
    observer: &mut dyn FnMut(NisarWriterAdmissionEventV2),
) -> Result<NisarWriterAdmissionV2, NisarSubsetWriteError> {
    if stopped() {
        return Err(NisarSubsetWriteError::Cancelled);
    }
    output_root
        .verify()
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let artifact_path = output_root
        .canonical_component(&format!("{derived_artifact_id}.h5"))
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let lock_directory = open_lock_directory(output_root)?;
    let artifact_lock_path = lock_directory
        .join_component(&nisar_writer_artifact_lock_name_v2(derived_artifact_id))
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let holder_note_path = lock_directory
        .join_component(&nisar_writer_holder_note_name_v2(derived_artifact_id))
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let earlier_build_lock_path = output_root
        .join_component(EARLIER_BUILD_DIRECTORY_LOCK_NAME)
        .map_err(|_| NisarSubsetWriteError::OutputUnsafe)?;
    let mut tracker = WaitTracker {
        observer,
        long_wait_after,
        started: None,
        long_wait_reported: false,
    };
    let this_process = || Some(std::process::id());
    let lease = destination_lease
        .acquire_for_runtime_observed(&artifact_path, stopped, &mut || {
            tracker.waiting(
                NisarWriterWaitReasonV2::SameArtifactInProcess,
                &this_process,
            )
        })
        .map_err(|error| match error {
            DestinationLeaseError::Cancelled => NisarSubsetWriteError::Cancelled,
            DestinationLeaseError::InvalidIdentity
            | DestinationLeaseError::Unavailable
            | DestinationLeaseError::InvariantViolation => NisarSubsetWriteError::OutputUnsafe,
        })?;
    let artifact_lock = poll_lock(
        &artifact_lock_path,
        LockMode::Exclusive,
        stopped,
        &mut || {
            tracker.waiting(NisarWriterWaitReasonV2::SameArtifactOtherProcess, &|| {
                read_holder_pid(&holder_note_path)
            })
        },
    )?;
    write_holder_note(&holder_note_path);
    let earlier_build_lock = poll_lock(
        &earlier_build_lock_path,
        LockMode::Shared,
        stopped,
        &mut || {
            tracker.waiting(NisarWriterWaitReasonV2::EarlierBuild, &|| {
                read_holder_pid(&earlier_build_lock_path)
            })
        },
    )?;
    let slot = slots.acquire(stopped, &mut || {
        tracker.waiting(NisarWriterWaitReasonV2::OutputSlot, &this_process)
    })?;
    tracker.admitted();
    Ok(NisarWriterAdmissionV2 {
        _slot: slot,
        _earlier_build_lock: earlier_build_lock,
        _artifact_lock: artifact_lock,
        _lock_directory: lock_directory,
        _lease: lease,
    })
}

#[derive(Clone, Copy)]
enum LockMode {
    Exclusive,
    Shared,
}

/// Polls one lock file until it is granted or `stopped`; a lock held by
/// another writer is waited for, any other lock failure is an unsafe output
/// directory.
fn poll_lock(
    path: &Path,
    mode: LockMode,
    stopped: &dyn Fn() -> bool,
    waiting: &mut dyn FnMut(),
) -> Result<Hdf5WorkspaceLockV1, NisarSubsetWriteError> {
    loop {
        if stopped() {
            return Err(NisarSubsetWriteError::Cancelled);
        }
        let attempt = match mode {
            LockMode::Exclusive => acquire_hdf5_workspace_lock(path),
            LockMode::Shared => try_acquire_hdf5_workspace_lock_shared(path),
        };
        match attempt {
            Ok(lock) => return Ok(lock),
            Err(Hdf5RangeTransferError::WorkspaceBusy) => {
                waiting();
                std::thread::sleep(LOCK_POLL_INTERVAL);
            }
            Err(_) => return Err(NisarSubsetWriteError::OutputUnsafe),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use NisarWriterAdmissionEventV2 as Event;

    const STILL_WAITING: Duration = Duration::from_millis(300);

    struct TempOutput(std::path::PathBuf);

    impl TempOutput {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "insar-writer-admission-{label}-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempOutput {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn admit(
        root: &PinnedDirectory,
        lease: &Arc<DestinationLeaseController>,
        slots: &Arc<NisarWriterOutputSlotsV2>,
        artifact: &str,
        stopped: &dyn Fn() -> bool,
        events: &mut Vec<Event>,
    ) -> Result<NisarWriterAdmissionV2, NisarSubsetWriteError> {
        admit_after(
            root,
            lease,
            slots,
            artifact,
            NISAR_WRITER_V2_LONG_WAIT,
            stopped,
            events,
        )
    }

    fn admit_after(
        root: &PinnedDirectory,
        lease: &Arc<DestinationLeaseController>,
        slots: &Arc<NisarWriterOutputSlotsV2>,
        artifact: &str,
        long_wait_after: Duration,
        stopped: &dyn Fn() -> bool,
        events: &mut Vec<Event>,
    ) -> Result<NisarWriterAdmissionV2, NisarSubsetWriteError> {
        admit_nisar_writer_v2(
            root,
            lease,
            slots,
            artifact,
            long_wait_after,
            stopped,
            &mut |event| events.push(event),
        )
    }

    use NisarWriterWaitReasonV2 as Reason;

    const ARTIFACT_A: &str = "nisar-rslc-subset-aaaaaaaaaaaaaaaaaaaaaaaa";
    const ARTIFACT_B: &str = "nisar-rslc-subset-bbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn different_artifacts_are_admitted_together_and_one_artifact_serializes() {
        let output = TempOutput::new("artifacts");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(2);
        let mut events = Vec::new();
        let first = admit(&root, &lease, &slots, ARTIFACT_A, &|| false, &mut events).unwrap();
        let other = admit(&root, &lease, &slots, ARTIFACT_B, &|| false, &mut events).unwrap();
        assert!(
            events.is_empty(),
            "different artifacts never wait: {events:?}"
        );
        assert_eq!(slots.in_use_for_test(), 2);
        assert!(output
            .0
            .join(NISAR_WRITER_LOCK_DIRECTORY_NAME)
            .join(nisar_writer_artifact_lock_name_v2(ARTIFACT_A))
            .is_file());
        thread::scope(|scope| {
            let (root, lease, slots) = (&root, &lease, &slots);
            let second = scope.spawn(move || {
                let mut events = Vec::new();
                let admitted =
                    admit(root, lease, slots, ARTIFACT_A, &|| false, &mut events).is_ok();
                (admitted, events)
            });
            thread::sleep(STILL_WAITING);
            assert!(
                !second.is_finished(),
                "a second writer of one artifact waits"
            );
            drop(first);
            let (admitted, events) = second.join().unwrap();
            assert!(admitted);
            assert_eq!(
                events,
                vec![
                    Event::Waiting {
                        reason: Reason::SameArtifactInProcess
                    },
                    Event::Admitted
                ]
            );
        });
        drop(other);
        assert_eq!(slots.in_use_for_test(), 0);
    }

    #[test]
    fn an_earlier_build_and_this_writer_never_write_in_one_directory_together() {
        let output = TempOutput::new("earlier-build");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(2);
        let directory_lock = output.0.join(EARLIER_BUILD_DIRECTORY_LOCK_NAME);
        let mut events = Vec::new();
        let first = admit(&root, &lease, &slots, ARTIFACT_A, &|| false, &mut events).unwrap();
        let other = admit(&root, &lease, &slots, ARTIFACT_B, &|| false, &mut events).unwrap();
        assert!(
            events.is_empty(),
            "writers of this build share the directory lock"
        );
        assert!(
            matches!(
                acquire_hdf5_workspace_lock(&directory_lock),
                Err(Hdf5RangeTransferError::WorkspaceBusy)
            ),
            "an earlier build cannot start writing beside this writer"
        );
        drop(first);
        drop(other);
        let earlier_build = acquire_hdf5_workspace_lock(&directory_lock).unwrap();
        let stop = AtomicBool::new(false);
        thread::scope(|scope| {
            let (root, lease, slots, stop) = (&root, &lease, &slots, &stop);
            let waiting = scope.spawn(move || {
                let mut events = Vec::new();
                let stopped = || stop.load(Ordering::SeqCst);
                let error = admit(root, lease, slots, ARTIFACT_A, &stopped, &mut events).err();
                (error, events)
            });
            thread::sleep(STILL_WAITING);
            assert!(
                !waiting.is_finished(),
                "this writer waits for the earlier build"
            );
            let stopped_at = Instant::now();
            stop.store(true, Ordering::SeqCst);
            let (error, events) = waiting.join().unwrap();
            assert!(stopped_at.elapsed() < Duration::from_secs(1));
            assert_eq!(error, Some(NisarSubsetWriteError::Cancelled));
            assert_eq!(
                events,
                vec![Event::Waiting {
                    reason: Reason::EarlierBuild
                }]
            );
        });
        drop(earlier_build);
        let mut events = Vec::new();
        let admitted = admit(&root, &lease, &slots, ARTIFACT_A, &|| false, &mut events);
        assert!(admitted.is_ok() && events.is_empty());
        drop(admitted);
        assert_eq!(slots.in_use_for_test(), 0);
    }

    #[test]
    fn output_slots_admit_in_turn_and_a_stopped_waiter_takes_no_slot() {
        let output = TempOutput::new("slots");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(NISAR_WRITER_V2_DEFAULT_OUTPUT_SLOTS);
        let mut events = Vec::new();
        let first = admit(&root, &lease, &slots, ARTIFACT_A, &|| false, &mut events).unwrap();
        let stop = AtomicBool::new(false);
        thread::scope(|scope| {
            let (root, lease, slots, stop) = (&root, &lease, &slots, &stop);
            let stopped_waiter = scope.spawn(move || {
                let mut events = Vec::new();
                let stopped = || stop.load(Ordering::SeqCst);
                let error = admit(root, lease, slots, ARTIFACT_B, &stopped, &mut events).err();
                (error, events)
            });
            thread::sleep(STILL_WAITING);
            assert!(
                !stopped_waiter.is_finished(),
                "the second artifact waits for a slot"
            );
            let stopped_at = Instant::now();
            stop.store(true, Ordering::SeqCst);
            let (error, events) = stopped_waiter.join().unwrap();
            assert!(stopped_at.elapsed() < Duration::from_secs(1));
            assert_eq!(error, Some(NisarSubsetWriteError::Cancelled));
            assert_eq!(
                events,
                vec![Event::Waiting {
                    reason: Reason::OutputSlot
                }]
            );
        });
        assert_eq!(slots.in_use_for_test(), 1, "a stopped waiter took no slot");
        thread::scope(|scope| {
            let (root, lease, slots) = (&root, &lease, &slots);
            let second = scope.spawn(move || {
                let mut events = Vec::new();
                let admitted =
                    admit(root, lease, slots, ARTIFACT_B, &|| false, &mut events).is_ok();
                (admitted, events)
            });
            thread::sleep(STILL_WAITING);
            assert!(!second.is_finished());
            drop(first);
            let (admitted, events) = second.join().unwrap();
            assert!(admitted);
            assert_eq!(
                events,
                vec![
                    Event::Waiting {
                        reason: Reason::OutputSlot
                    },
                    Event::Admitted
                ]
            );
        });
        assert_eq!(slots.in_use_for_test(), 0);
    }

    #[test]
    fn a_stopped_worker_is_not_admitted() {
        let output = TempOutput::new("stopped");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(1);
        let mut events = Vec::new();
        assert_eq!(
            admit(&root, &lease, &slots, ARTIFACT_A, &|| true, &mut events).err(),
            Some(NisarSubsetWriteError::Cancelled)
        );
        assert!(events.is_empty());
        assert_eq!(slots.in_use_for_test(), 0);
    }

    /// Review of D1/D2 (downloader enhancement D1b, P1-1): the output slots
    /// admit waiters in arrival order. A writer that arrives right when a slot
    /// is released queues behind the writer already waiting instead of taking
    /// the slot first.
    #[test]
    fn output_slots_admit_waiters_in_arrival_order() {
        for round in 0..5 {
            let slots = NisarWriterOutputSlotsV2::new(1);
            let order = Mutex::new(Vec::new());
            let held = slots.hold_for_test();
            thread::scope(|scope| {
                let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
                let (slots_ref, order_ref) = (&slots, &order);
                let first = scope.spawn(move || {
                    let slot = slots_ref
                        .acquire(&|| false, &mut || {
                            let _ = waiting_tx.send(());
                        })
                        .unwrap();
                    order_ref.lock().unwrap().push("first waiter");
                    thread::sleep(Duration::from_millis(20));
                    drop(slot);
                });
                waiting_rx
                    .recv_timeout(Duration::from_secs(10))
                    .expect("the first writer waits for the slot");
                thread::sleep(Duration::from_millis(100));
                drop(held);
                let slot = slots.acquire(&|| false, &mut || {}).unwrap();
                order.lock().unwrap().push("late arrival");
                drop(slot);
                first.join().unwrap();
            });
            assert_eq!(
                *order.lock().unwrap(),
                vec!["first waiter", "late arrival"],
                "round {round}"
            );
            assert_eq!(slots.in_use_for_test(), 0);
        }
    }
    #[test]
    fn the_output_slot_setting_accepts_one_to_eight_and_defaults_to_one() {
        assert_eq!(parse_nisar_writer_output_slots(None), Ok(1));
        assert_eq!(parse_nisar_writer_output_slots(Some("2")), Ok(2));
        assert_eq!(parse_nisar_writer_output_slots(Some(" 8 ")), Ok(8));
        for invalid in ["0", "9", "-1", "two", "", "1.5"] {
            assert!(
                parse_nisar_writer_output_slots(Some(invalid)).is_err(),
                "{invalid:?} is refused"
            );
        }
    }

    /// Configured to two slots, two writers produce output at the same time
    /// and a third one waits for the first slot that frees up.
    #[test]
    fn two_configured_slots_admit_two_writers_and_queue_the_third() {
        let slots =
            NisarWriterOutputSlotsV2::new(parse_nisar_writer_output_slots(Some("2")).unwrap());
        assert_eq!(slots.capacity(), 2);
        let mut waited = false;
        let first = slots.acquire(&|| false, &mut || waited = true).unwrap();
        let second = slots.acquire(&|| false, &mut || waited = true).unwrap();
        assert!(!waited, "two writers are admitted without waiting");
        assert_eq!(slots.in_use_for_test(), 2);
        thread::scope(|scope| {
            let slots_ref = &slots;
            let third = scope.spawn(move || {
                let mut announced = false;
                let slot = slots_ref
                    .acquire(&|| false, &mut || announced = true)
                    .unwrap();
                drop(slot);
                announced
            });
            thread::sleep(STILL_WAITING);
            assert!(!third.is_finished(), "the third writer waits");
            drop(first);
            assert!(third.join().unwrap(), "the third writer reported its wait");
        });
        drop(second);
        assert_eq!(slots.in_use_for_test(), 0);
    }
    /// A link or junction named like the lock directory would move the locks
    /// out of the output root, where writers of another profile would not
    /// meet them: admission refuses it as an unsafe output directory.
    #[test]
    fn a_lock_directory_that_points_elsewhere_is_refused() {
        let output = TempOutput::new("lock-directory-link");
        let elsewhere = TempOutput::new("lock-directory-target");
        let link = output.0.join(NISAR_WRITER_LOCK_DIRECTORY_NAME);
        #[cfg(windows)]
        let linked = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&elsewhere.0)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&elsewhere.0, &link).is_ok();
        assert!(linked, "the test created the link");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(1);
        let mut events = Vec::new();
        assert_eq!(
            admit(&root, &lease, &slots, ARTIFACT_A, &|| false, &mut events).err(),
            Some(NisarSubsetWriteError::OutputUnsafe)
        );
        assert!(std::fs::read_dir(&elsewhere.0).unwrap().next().is_none());
        assert_eq!(slots.in_use_for_test(), 0);
    }
    /// Review of D1/D2 (downloader enhancement D1b, P2-3): a writer that waits
    /// longer than its threshold is told once why it waits now and who holds
    /// what it waits for. For the output slot the holder is this process.
    #[test]
    fn a_long_wait_for_an_output_slot_is_reported_once_with_this_process() {
        let output = TempOutput::new("long-wait-slot");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(1);
        let held = slots.hold_for_test();
        let (events, waited_long) = thread::scope(|scope| {
            let (root, lease, slots) = (&root, &lease, &slots);
            let writer = scope.spawn(move || {
                let mut events = Vec::new();
                let admitted = admit_after(
                    root,
                    lease,
                    slots,
                    ARTIFACT_A,
                    Duration::from_millis(200),
                    &|| false,
                    &mut events,
                );
                drop(admitted.unwrap());
                events
            });
            thread::sleep(Duration::from_millis(600));
            drop(held);
            let events = writer.join().unwrap();
            let waited_long = events.iter().any(|event| {
                matches!(event, Event::LongWait { waited, .. } if *waited >= Duration::from_millis(200))
            });
            (events, waited_long)
        });
        assert_eq!(events.len(), 3, "{events:?}");
        assert_eq!(
            events[0],
            Event::Waiting {
                reason: Reason::OutputSlot
            }
        );
        assert!(matches!(
            events[1],
            Event::LongWait {
                reason: Reason::OutputSlot,
                holder_pid: Some(pid),
                ..
            } if pid == std::process::id()
        ));
        assert!(waited_long);
        assert_eq!(events[2], Event::Admitted);
    }

    /// Another process holding the artifact lock is named by the holder note
    /// it wrote when it took the lock.
    #[test]
    fn a_long_wait_for_an_artifact_lock_names_the_holder_process() {
        let output = TempOutput::new("long-wait-artifact");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lock_directory = output.0.join(NISAR_WRITER_LOCK_DIRECTORY_NAME);
        std::fs::create_dir_all(&lock_directory).unwrap();
        let held = acquire_hdf5_workspace_lock(
            &lock_directory.join(nisar_writer_artifact_lock_name_v2(ARTIFACT_A)),
        )
        .unwrap();
        std::fs::write(
            lock_directory.join(nisar_writer_holder_note_name_v2(ARTIFACT_A)),
            "pid=4242\n",
        )
        .unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(1);
        let stop = AtomicBool::new(false);
        let events = thread::scope(|scope| {
            let (root, lease, slots, stop) = (&root, &lease, &slots, &stop);
            let writer = scope.spawn(move || {
                let mut events = Vec::new();
                let stopped = || stop.load(Ordering::SeqCst);
                let error = admit_after(
                    root,
                    lease,
                    slots,
                    ARTIFACT_A,
                    Duration::from_millis(200),
                    &stopped,
                    &mut events,
                )
                .err();
                assert_eq!(error, Some(NisarSubsetWriteError::Cancelled));
                events
            });
            thread::sleep(Duration::from_millis(600));
            stop.store(true, Ordering::SeqCst);
            writer.join().unwrap()
        });
        drop(held);
        assert_eq!(
            events,
            vec![
                Event::Waiting {
                    reason: Reason::SameArtifactOtherProcess
                },
                Event::LongWait {
                    reason: Reason::SameArtifactOtherProcess,
                    holder_pid: Some(4242),
                    waited: match events.get(1) {
                        Some(Event::LongWait { waited, .. }) => *waited,
                        _ => Duration::ZERO,
                    },
                },
            ]
        );
    }

    /// An earlier build holding its directory lock is reported as such. Its
    /// process id is in the locked file, which Windows does not let another
    /// handle read, so it is unknown there.
    #[test]
    fn a_long_wait_for_an_earlier_build_is_reported_as_an_earlier_build() {
        let output = TempOutput::new("long-wait-earlier");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let earlier_build =
            acquire_hdf5_workspace_lock(&output.0.join(EARLIER_BUILD_DIRECTORY_LOCK_NAME)).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(1);
        let stop = AtomicBool::new(false);
        let events = thread::scope(|scope| {
            let (root, lease, slots, stop) = (&root, &lease, &slots, &stop);
            let writer = scope.spawn(move || {
                let mut events = Vec::new();
                let stopped = || stop.load(Ordering::SeqCst);
                let _ = admit_after(
                    root,
                    lease,
                    slots,
                    ARTIFACT_A,
                    Duration::from_millis(200),
                    &stopped,
                    &mut events,
                );
                events
            });
            thread::sleep(Duration::from_millis(600));
            stop.store(true, Ordering::SeqCst);
            writer.join().unwrap()
        });
        drop(earlier_build);
        assert_eq!(events.len(), 2, "{events:?}");
        let Event::LongWait {
            reason, holder_pid, ..
        } = events[1]
        else {
            panic!("a long wait was reported: {events:?}");
        };
        assert_eq!(reason, Reason::EarlierBuild);
        if cfg!(windows) {
            assert_eq!(holder_pid, None);
        } else {
            assert_eq!(holder_pid, Some(std::process::id()));
        }
    }

    /// A wait shorter than the threshold reports no long wait.
    #[test]
    fn a_short_wait_reports_no_long_wait() {
        let output = TempOutput::new("short-wait");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(1);
        let held = slots.hold_for_test();
        let events = thread::scope(|scope| {
            let (root, lease, slots) = (&root, &lease, &slots);
            let writer = scope.spawn(move || {
                let mut events = Vec::new();
                drop(admit(root, lease, slots, ARTIFACT_A, &|| false, &mut events).unwrap());
                events
            });
            thread::sleep(STILL_WAITING);
            drop(held);
            writer.join().unwrap()
        });
        assert_eq!(
            events,
            vec![
                Event::Waiting {
                    reason: Reason::OutputSlot
                },
                Event::Admitted
            ]
        );
    }
    /// Review of D1b (downloader enhancement D1c, P2): a waiting writer whose
    /// callback panics (the scheduler catches worker panics and the process
    /// keeps running) must not leave its ticket at the head of the queue,
    /// where it would block every later writer forever.
    #[test]
    fn a_panicking_wait_callback_leaves_no_ticket_in_the_queue() {
        let slots = NisarWriterOutputSlotsV2::new(1);
        let held = slots.hold_for_test();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            slots.acquire(&|| false, &mut || panic!("the wait observer failed"))
        }));
        assert!(panicked.is_err());
        drop(held);
        let started = Instant::now();
        let gave_up = || started.elapsed() > Duration::from_secs(2);
        let mut waited = false;
        let slot = slots.acquire(&gave_up, &mut || waited = true);
        assert!(slot.is_ok(), "a dead ticket blocked the queue");
        assert!(!waited, "the free slot was granted at once");
        drop(slot);
        assert_eq!(slots.in_use_for_test(), 0);
    }
    /// Review of D1b (downloader enhancement D1c, P3): a link or junction
    /// named like the lock directory that points to a sibling inside the
    /// output root resolves to a direct child of the root as well; it is
    /// refused because the lock directory must be exactly
    /// <output root>/.eostage-locks.
    #[test]
    fn a_lock_directory_link_to_a_sibling_inside_the_output_root_is_refused() {
        let output = TempOutput::new("lock-directory-sibling");
        let sibling = output.0.join("sibling");
        std::fs::create_dir_all(&sibling).unwrap();
        let link = output.0.join(NISAR_WRITER_LOCK_DIRECTORY_NAME);
        #[cfg(windows)]
        let linked = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&sibling)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&sibling, &link).is_ok();
        assert!(linked, "the test created the link");
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(1);
        let mut events = Vec::new();
        assert_eq!(
            admit(&root, &lease, &slots, ARTIFACT_A, &|| false, &mut events).err(),
            Some(NisarSubsetWriteError::OutputUnsafe)
        );
        assert!(std::fs::read_dir(&sibling).unwrap().next().is_none());
        assert_eq!(slots.in_use_for_test(), 0);
    }

    /// D1c (review P3): the holder note is written without following a link.
    /// A hard link named like the note shares another file's content, so
    /// writing through it would overwrite that file; the note is skipped
    /// instead (it is only a diagnostic) and admission proceeds.
    #[test]
    fn the_holder_note_is_not_written_through_a_link() {
        let output = TempOutput::new("holder-note-link");
        let lock_directory = output.0.join(NISAR_WRITER_LOCK_DIRECTORY_NAME);
        std::fs::create_dir_all(&lock_directory).unwrap();
        let victim = output.0.join("victim.txt");
        std::fs::write(&victim, b"victim").unwrap();
        std::fs::hard_link(
            &victim,
            lock_directory.join(nisar_writer_holder_note_name_v2(ARTIFACT_A)),
        )
        .unwrap();
        let root = PinnedDirectory::create(&output.0).unwrap();
        let lease = DestinationLeaseController::new();
        let slots = NisarWriterOutputSlotsV2::new(1);
        let mut events = Vec::new();
        let admitted = admit(&root, &lease, &slots, ARTIFACT_A, &|| false, &mut events);
        assert!(admitted.is_ok(), "the note is only a diagnostic");
        drop(admitted);
        assert_eq!(std::fs::read(&victim).unwrap(), b"victim");
    }
}
