//! Free-space guard of the download engine (PM decision 2026-10-26, A1 §3.1):
//! a task pauses when the drive of its output root runs low. The policy is
//! `insar_core::storage_guard::DiskLowPolicy`; this module only measures.
//! Nothing here takes an Agent permission mode: the guard runs for every task
//! whatever started it.

use std::path::{Component, Path};

use insar_core::storage_guard::DiskLowPolicy;
use insar_core::TaskPauseReason;

/// Free and total bytes of one volume as seen by this process (quota-aware).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeSpace {
    pub free_bytes: u64,
    pub total_bytes: u64,
}

/// Measures the volume holding a path. `None` means unknown, and an unknown
/// volume is never refused: the write itself still reports a real failure.
pub trait FreeSpaceProbe: Send + Sync {
    fn volume_space(&self, path: &Path) -> Option<VolumeSpace>;
}

/// The operating system's view (`GetDiskFreeSpaceExW` / `statvfs`) of the
/// nearest existing ancestor of the path, since an output root is created
/// only when its first file is written.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemFreeSpaceProbe;

impl FreeSpaceProbe for SystemFreeSpaceProbe {
    fn volume_space(&self, path: &Path) -> Option<VolumeSpace> {
        let existing = path
            .ancestors()
            .filter(|candidate| !candidate.as_os_str().is_empty())
            .find(|candidate| candidate.is_dir())?;
        platform_volume_space(existing)
    }
}

/// The probe every default construction uses: the operating system's view in
/// production builds; in test builds (this crate's unit tests, or the
/// `test-plenty-disk-space` feature that dependants enable only through
/// dev-dependencies) a volume with plenty of space, so a nearly full
/// development drive never pauses an unrelated test.
pub fn default_free_space_probe() -> std::sync::Arc<dyn FreeSpaceProbe> {
    #[cfg(any(test, feature = "test-plenty-disk-space"))]
    {
        std::sync::Arc::new(test_support::TestFreeSpaceProbe)
    }
    #[cfg(not(any(test, feature = "test-plenty-disk-space")))]
    {
        std::sync::Arc::new(SystemFreeSpaceProbe)
    }
}

/// Test builds only: the default probe and a process-wide knob for the one
/// kind of test that must drive a default-constructed service low.
#[cfg(any(test, feature = "test-plenty-disk-space"))]
pub mod test_support {
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{FreeSpaceProbe, VolumeSpace};

    const PLENTY: u64 = u64::MAX;
    /// Total size reported once a test sets the free bytes: 20 GiB, so the
    /// 2 GiB absolute floor is the larger one.
    pub const TEST_VOLUME_TOTAL_BYTES: u64 = 20 << 30;
    static FREE_BYTES: AtomicU64 = AtomicU64::new(PLENTY);

    /// `None` restores plenty of space. Process-wide: use it only in a test
    /// binary of its own.
    pub fn set_default_probe_free_bytes(free_bytes: Option<u64>) {
        FREE_BYTES.store(free_bytes.unwrap_or(PLENTY), Ordering::SeqCst);
    }

    #[derive(Clone, Copy, Debug, Default)]
    pub struct TestFreeSpaceProbe;

    impl FreeSpaceProbe for TestFreeSpaceProbe {
        fn volume_space(&self, _path: &Path) -> Option<VolumeSpace> {
            match FREE_BYTES.load(Ordering::SeqCst) {
                PLENTY => Some(VolumeSpace {
                    free_bytes: 1 << 50,
                    total_bytes: 1 << 51,
                }),
                free_bytes => Some(VolumeSpace {
                    free_bytes,
                    total_bytes: TEST_VOLUME_TOTAL_BYTES,
                }),
            }
        }
    }
}

/// The drive or volume prefix shown to the user (`F:`, `\\server\share`) or
/// the root directory; `None` when the path names neither (the UI shows
/// 未知盘符).
pub fn drive_label(path: &Path) -> Option<String> {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => Some(prefix.as_os_str().to_string_lossy().into_owned()),
        Some(Component::RootDir) => Some(std::path::MAIN_SEPARATOR.to_string()),
        _ => None,
    }
}

/// The pause reason when the volume of `output_root` is below the policy's
/// floor, `remaining_file_bytes` being the unwritten bytes of the current
/// file when known.
pub fn disk_low_reason(
    probe: &dyn FreeSpaceProbe,
    policy: &DiskLowPolicy,
    output_root: &Path,
    remaining_file_bytes: Option<u64>,
) -> Option<TaskPauseReason> {
    let space = probe.volume_space(output_root)?;
    evaluate_space(policy, space, output_root, remaining_file_bytes)
}

/// `disk_low_reason` for a measurement already taken.
pub fn evaluate_space(
    policy: &DiskLowPolicy,
    space: VolumeSpace,
    output_root: &Path,
    remaining_file_bytes: Option<u64>,
) -> Option<TaskPauseReason> {
    policy.evaluate(
        space.free_bytes,
        space.total_bytes,
        remaining_file_bytes,
        drive_label(output_root).as_deref(),
    )
}

/// The reason to refuse a resume: one measurement of the output volume
/// against the drive floor, raised to the threshold recorded on the task's
/// engine pause (which may include the file rule) while that pause names the
/// same drive. An unmeasurable volume never refuses.
pub fn disk_low_resume_reason(
    probe: &dyn FreeSpaceProbe,
    policy: &DiskLowPolicy,
    output_root: &Path,
    recorded: Option<&TaskPauseReason>,
) -> Option<TaskPauseReason> {
    let space = probe.volume_space(output_root)?;
    let drive = drive_label(output_root);
    let recorded_threshold = recorded
        .filter(|reason| reason.drive == drive)
        .map(|reason| reason.threshold_bytes);
    insar_core::storage_guard::below(
        space.free_bytes,
        policy.resume_threshold_bytes(space.total_bytes, recorded_threshold),
        drive.as_deref(),
    )
}

/// One diagnostic line per task per coordinator lifetime when the output
/// volume cannot be measured (the guard then allows the download). The line
/// names the task only, never a path.
#[derive(Debug, Default)]
pub struct DiskProbeFailureLog {
    logged: std::collections::HashSet<String>,
}

impl DiskProbeFailureLog {
    /// The line to emit the first time `task_id` is observed, else `None`.
    pub fn observe(&mut self, task_id: &str) -> Option<String> {
        self.logged.insert(task_id.to_owned()).then(|| {
            format!(
                "free space of the output volume is unknown; the disk-low guard allows downloads: task_id={task_id}"
            )
        })
    }
}

/// The string handed to `GetDiskFreeSpaceExW`: a UNC root (`\\server\share`)
/// is only accepted with a trailing separator, and a trailing separator is
/// harmless for every other directory, so one is always present.
pub fn volume_query_path(directory: &Path) -> std::ffi::OsString {
    let mut query = directory.as_os_str().to_owned();
    let text = query.to_string_lossy();
    if !(text.ends_with('\\') || text.ends_with('/')) {
        query.push(std::path::MAIN_SEPARATOR_STR);
    }
    query
}

#[cfg(windows)]
fn platform_volume_space(directory: &Path) -> Option<VolumeSpace> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let wide = volume_query_path(directory)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    let mut available = 0_u64;
    let mut total = 0_u64;
    let mut free = 0_u64;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call
    // and the three out-pointers refer to live, writable u64 locals.
    let status =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, &mut total, &mut free) };
    (status != 0).then_some(VolumeSpace {
        free_bytes: available,
        total_bytes: total,
    })
}

#[cfg(unix)]
#[allow(clippy::useless_conversion)]
fn platform_volume_space(directory: &Path) -> Option<VolumeSpace> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(directory.as_os_str().as_bytes()).ok()?;
    let mut facts = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `facts` is writable storage for
    // exactly one statvfs record that is read only after success.
    let status = unsafe { libc::statvfs(path.as_ptr(), facts.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    // SAFETY: statvfs returned success, so the record is initialized.
    let facts = unsafe { facts.assume_init() };
    let unit = u64::from(facts.f_frsize);
    Some(VolumeSpace {
        free_bytes: u64::from(facts.f_bavail).checked_mul(unit)?,
        total_bytes: u64::from(facts.f_blocks).checked_mul(unit)?,
    })
}

#[cfg(not(any(windows, unix)))]
fn platform_volume_space(_directory: &Path) -> Option<VolumeSpace> {
    None
}

/// Where the coordinator writes its probe-failure line.
pub type DiskProbeLogSink = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// The production sink, like the scheduler's other bounded diagnostics.
pub fn stderr_disk_probe_log() -> DiskProbeLogSink {
    std::sync::Arc::new(|line: &str| eprintln!("{line}"))
}

/// What the scheduling pass may do for a task about to open an item.
pub(crate) enum CachedDiskVerdict {
    /// No measurement younger than `FRESH_FOR`: one is taken outside the
    /// Store lock before the next pass, and no item opens in this one.
    Pending,
    Allow,
    Low(TaskPauseReason),
}

/// The coordinator's view of output volumes. Every measurement is taken
/// outside the shared Store lock: the progress path measures before locking,
/// and the scheduling pass (which runs under the lock) only reads measurements
/// younger than one second, asking for the rest to be refreshed first.
pub(crate) struct CoordinatorDiskGuard {
    probe: std::sync::Arc<dyn FreeSpaceProbe>,
    log: DiskProbeLogSink,
    policy: DiskLowPolicy,
    measured:
        std::collections::HashMap<std::path::PathBuf, (std::time::Instant, Option<VolumeSpace>)>,
    wanted: std::collections::BTreeSet<std::path::PathBuf>,
    failures: DiskProbeFailureLog,
    iteration_start: std::time::Instant,
}

impl CoordinatorDiskGuard {
    pub(crate) const FRESH_FOR: std::time::Duration = std::time::Duration::from_secs(1);

    pub(crate) fn new(probe: std::sync::Arc<dyn FreeSpaceProbe>, log: DiskProbeLogSink) -> Self {
        Self {
            probe,
            log,
            policy: DiskLowPolicy::default(),
            measured: std::collections::HashMap::new(),
            wanted: std::collections::BTreeSet::new(),
            failures: DiskProbeFailureLog::default(),
            iteration_start: std::time::Instant::now(),
        }
    }

    /// Marks the start of a coordinator iteration: a pause decision needs a
    /// measurement taken after this point, so a volume the user has just
    /// freed is never paused on a reading from before the resume.
    pub(crate) fn begin_iteration(&mut self) {
        self.iteration_start = std::time::Instant::now();
    }

    /// Call without the Store lock. Returns whether anything was measured.
    pub(crate) fn refresh_wanted(&mut self) -> bool {
        if self.wanted.is_empty() {
            return false;
        }
        for root in std::mem::take(&mut self.wanted) {
            let space = self.probe.volume_space(&root);
            self.measured
                .insert(root, (std::time::Instant::now(), space));
        }
        true
    }

    pub(crate) fn has_wanted(&self) -> bool {
        !self.wanted.is_empty()
    }

    /// Safe under the Store lock: never measures.
    pub(crate) fn cached_verdict(
        &mut self,
        task_id: &str,
        output_root: &Path,
        now: std::time::Instant,
    ) -> CachedDiskVerdict {
        match self.measured.get(output_root) {
            Some((at, space)) if now.saturating_duration_since(*at) < Self::FRESH_FOR => {
                let current = *at >= self.iteration_start;
                match *space {
                    Some(space) => match evaluate_space(&self.policy, space, output_root, None) {
                        None => CachedDiskVerdict::Allow,
                        Some(reason) if current => CachedDiskVerdict::Low(reason),
                        Some(_) => {
                            self.wanted.insert(output_root.to_path_buf());
                            CachedDiskVerdict::Pending
                        }
                    },
                    None => {
                        self.note_unknown(task_id);
                        CachedDiskVerdict::Allow
                    }
                }
            }
            _ => {
                self.wanted.insert(output_root.to_path_buf());
                CachedDiskVerdict::Pending
            }
        }
    }

    /// Wait-free free-space lookup safe under the Store lock (`TASK-REG-016`):
    /// returns a fresh cached reading (< `FRESH_FOR`) without probing, or queues
    /// `output_root` to be measured outside the Store lock on the next coordinator pass.
    pub(crate) fn cached_free_space(
        &mut self,
        output_root: &Path,
        now: std::time::Instant,
    ) -> Option<VolumeSpace> {
        if let Some((at, space)) = self.measured.get(output_root) {
            if now.saturating_duration_since(*at) < Self::FRESH_FOR {
                return *space;
            }
        }
        self.wanted.insert(output_root.to_path_buf());
        None
    }

    /// Call without the Store lock: a fresh measurement for a writing item.
    pub(crate) fn verdict_while_writing(
        &mut self,
        task_id: &str,
        output_root: &Path,
        remaining_file_bytes: Option<u64>,
    ) -> Option<TaskPauseReason> {
        let space = self.probe.volume_space(output_root);
        self.measured.insert(
            output_root.to_path_buf(),
            (std::time::Instant::now(), space),
        );
        match space {
            Some(space) => evaluate_space(&self.policy, space, output_root, remaining_file_bytes),
            None => {
                self.note_unknown(task_id);
                None
            }
        }
    }

    fn note_unknown(&mut self, task_id: &str) {
        if let Some(line) = self.failures.observe(task_id) {
            (self.log)(&line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct CountingProbe {
        calls: AtomicUsize,
        space: Option<VolumeSpace>,
    }

    impl FreeSpaceProbe for CountingProbe {
        fn volume_space(&self, _path: &Path) -> Option<VolumeSpace> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.space
        }
    }

    #[test]
    fn cached_free_space_never_probes_and_triggers_external_refresh() {
        let probe = Arc::new(CountingProbe {
            calls: AtomicUsize::new(0),
            space: Some(VolumeSpace {
                free_bytes: 5 * 1024 * 1024 * 1024,
                total_bytes: 100 * 1024 * 1024 * 1024,
            }),
        });
        let mut guard = CoordinatorDiskGuard::new(probe.clone(), Arc::new(|_| {}));
        let root = Path::new("/tmp/eostage-waitfree-disk-guard");

        // Under the Store lock: cached lookup never calls probe and marks root wanted.
        assert_eq!(
            guard.cached_free_space(root, std::time::Instant::now()),
            None
        );
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        assert!(guard.has_wanted());

        // Outside the Store lock: refresh_wanted measures once and populates cached lookup.
        assert!(guard.refresh_wanted());
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert!(!guard.has_wanted());

        let cached = guard.cached_free_space(root, std::time::Instant::now());
        assert_eq!(
            cached,
            Some(VolumeSpace {
                free_bytes: 5 * 1024 * 1024 * 1024,
                total_bytes: 100 * 1024 * 1024 * 1024,
            })
        );
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }
}
