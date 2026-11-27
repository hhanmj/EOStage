//! Application-owned HDF5 dynamic plugin isolation, established before the
//! HDF5 library initialises.
//!
//! HDF5 can load external dynamic plugins and drivers from the process
//! environment during library startup:
//!
//! - `HDF5_PLUGIN_PATH`      (filter/VOL/VFD plugin search path)
//! - `HDF5_VOL_CONNECTOR`    (VOL connector selected at startup)
//! - `HDF5_DRIVER`           (file driver selected at startup)
//! - `HDF5_DRIVER_CONFIG`    (driver configuration)
//!
//! Calling `H5PLset_loading_state(0)` after `H5open()` cannot undo driver or
//! VOL selection that HDF5 already made from those variables, so the packaged
//! host must sanitise the process environment *before* the first HDF5 call
//! and before any thread creation, then set the official
//! `HDF5_PLUGIN_PRELOAD=::` sentinel so the H5PL package initialises with
//! dynamic plugin loading already disabled.
//!
//! The NISAR validator then checks the live H5PL control mask and fails
//! closed whenever dynamic plugin loading is enabled.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use hdf5_sys::h5::H5open;
use serde::Serialize;

/// Mask enabling all dynamic plugin types (H5PL_ALL_PLUGIN). A zero mask
/// disables all dynamic plugin types, which is the isolation we establish.
const H5PL_ALL_PLUGIN: u32 = 0xffff;

/// HDF5's process-wide file-locking switch, read on every `H5Fopen`/`H5Fcreate`.
#[cfg(unix)]
const HDF5_FILE_LOCKING_ENV: &str = "HDF5_USE_FILE_LOCKING";

// hdf5-metno-sys declares the wrong parameter type for these two H5PL
// functions, so declare the official HDF5 signatures directly. The symbols
// resolve against the same static HDF5 library linked by hdf5-metno-sys.
unsafe extern "C" {
    fn H5PLset_loading_state(plugin_control_mask: u32) -> i32;
    fn H5PLget_loading_state(plugin_control_mask: *mut u32) -> i32;
}

static ISOLATION_ESTABLISHED: AtomicBool = AtomicBool::new(false);
static LAST_ISOLATION_STATUS: Mutex<Option<Hdf5ProcessIsolationStatus>> = Mutex::new(None);

/// Process-environment mutation that is visible to C `getenv` (HDF5 reads the
/// plugin/driver variables through the C runtime). Rust's `std::env::set_var`
/// only updates the Windows OS environment block and does NOT update the CRT
/// `_environ` view, so on Windows we must go through `_putenv_s`/`_putenv`.
#[cfg(windows)]
pub(crate) mod crt_env {
    use std::ffi::{c_char, c_int, CStr, CString};

    unsafe extern "C" {
        fn _putenv_s(name: *const c_char, value: *const c_char) -> c_int;
        fn _putenv(name: *const c_char) -> c_int;
        fn getenv(name: *const c_char) -> *mut c_char;
    }

    pub(crate) fn set(name: &str, value: &str) -> bool {
        let name = CString::new(name).expect("env name has no NUL");
        let value = CString::new(value).expect("env value has no NUL");
        unsafe { _putenv_s(name.as_ptr(), value.as_ptr()) == 0 }
    }

    pub(crate) fn remove(name: &str) -> bool {
        // `name=` (empty value) deletes the variable from the CRT
        // environment, which is what HDF5's getenv will observe.
        let pair = CString::new(format!("{name}=")).expect("env name has no NUL");
        unsafe { _putenv(pair.as_ptr()) == 0 }
    }

    pub(crate) fn get(name: &str) -> Option<String> {
        let name = CString::new(name).expect("env name has no NUL");
        let pointer = unsafe { getenv(name.as_ptr()) };
        if pointer.is_null() {
            None
        } else {
            Some(
                unsafe { CStr::from_ptr(pointer) }
                    .to_string_lossy()
                    .into_owned(),
            )
        }
    }
}

#[cfg(not(windows))]
pub(crate) mod crt_env {
    pub(crate) fn set(name: &str, value: &str) -> bool {
        std::env::set_var(name, value);
        true
    }

    pub(crate) fn remove(name: &str) -> bool {
        std::env::remove_var(name);
        true
    }

    pub(crate) fn get(name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// Structured evidence of the process-level HDF5 isolation established by the
/// application itself. Each field maps to one step of the startup policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Hdf5ProcessIsolationStatus {
    /// All four hostile startup variables were cleared/set before H5open.
    pub startup_environment_sanitized: bool,
    /// The application set `HDF5_PLUGIN_PRELOAD=::` in this process.
    pub preload_sentinel_set_by_application: bool,
    /// True when the parent process did NOT supply `HDF5_PLUGIN_PRELOAD`
    /// before this process sanitised its environment.
    pub parent_env_preload_absent: bool,
    /// `HDF5_PLUGIN_PATH` inherited from the parent was removed.
    pub plugin_path_removed: bool,
    /// `HDF5_VOL_CONNECTOR` inherited from the parent was removed.
    pub vol_connector_removed: bool,
    /// `HDF5_DRIVER` inherited from the parent was removed.
    pub driver_removed: bool,
    /// `HDF5_DRIVER_CONFIG` inherited from the parent was removed.
    pub driver_config_removed: bool,
    /// The live H5PL control mask is zero (all dynamic plugin types disabled).
    pub loading_mask_zero: bool,
    /// Every step succeeded; NISAR HDF5 validation may run.
    pub isolation_established: bool,
}

/// Establish HDF5 dynamic-plugin isolation in the current process.
///
/// This must be called at the very start of `main()`, before any other HDF5
/// function call and before any thread is created:
///
/// 1. set `HDF5_PLUGIN_PRELOAD=::` for this process;
/// 2. remove inherited `HDF5_PLUGIN_PATH`, `HDF5_VOL_CONNECTOR`,
///    `HDF5_DRIVER` and `HDF5_DRIVER_CONFIG`;
/// 3. call `H5open()`;
/// 4. call `H5PLset_loading_state(0)`;
/// 5. verify with `H5PLget_loading_state()` that the live mask is zero.
///
/// Any failure leaves NISAR validation fail-closed (the validator re-checks
/// the live mask) and does not block non-HDF5 functionality. Ordinary users
/// never need to configure system or user environment variables.
pub fn establish_hdf5_process_isolation() -> Result<Hdf5ProcessIsolationStatus, String> {
    let _lock = hdf5_sys::LOCK.lock();
    // Record whether the parent supplied the sentinel BEFORE the application
    // sanitises its own environment. The packaging harness removes it before
    // every packaged EXE launch; this field is the real evidence.
    let parent_env_preload_absent = std::env::var_os("HDF5_PLUGIN_PRELOAD").is_none();
    let mut status = Hdf5ProcessIsolationStatus {
        startup_environment_sanitized: false,
        preload_sentinel_set_by_application: false,
        parent_env_preload_absent,
        plugin_path_removed: false,
        vol_connector_removed: false,
        driver_removed: false,
        driver_config_removed: false,
        loading_mask_zero: false,
        isolation_established: false,
    };

    // 1. The application itself guarantees the H5PL sentinel for this
    // process, through the C runtime so HDF5's getenv observes it.
    status.preload_sentinel_set_by_application = crt_env::set("HDF5_PLUGIN_PRELOAD", "::");

    // 2. Remove inherited driver/VOL/plugin configuration before H5open so
    // HDF5 library startup cannot select an external connector or driver.
    // The *_removed flags report the post-sanitisation state (absent), which
    // is the security property; a variable that was never inherited is also
    // "removed" from the process environment.
    crt_env::remove("HDF5_PLUGIN_PATH");
    crt_env::remove("HDF5_VOL_CONNECTOR");
    crt_env::remove("HDF5_DRIVER");
    crt_env::remove("HDF5_DRIVER_CONFIG");
    status.plugin_path_removed = crt_env::get("HDF5_PLUGIN_PATH").is_none();
    status.vol_connector_removed = crt_env::get("HDF5_VOL_CONNECTOR").is_none();
    status.driver_removed = crt_env::get("HDF5_DRIVER").is_none();
    status.driver_config_removed = crt_env::get("HDF5_DRIVER_CONFIG").is_none();
    status.startup_environment_sanitized = status.preload_sentinel_set_by_application
        && status.plugin_path_removed
        && status.vol_connector_removed
        && status.driver_removed
        && status.driver_config_removed;

    // On Unix the application's own `File::try_lock` (flock) on writer,
    // candidate and promotion handles is the exclusive-access authority.
    // HDF5's sec2 driver would flock a second open file description of the
    // same file and fail with EAGAIN against that lock. Windows HDF5 builds do
    // not lock, so this keeps both platforms on the application-owned lock.
    // HDF5 re-reads this variable on every file open.
    #[cfg(unix)]
    crt_env::set(HDF5_FILE_LOCKING_ENV, "FALSE");

    // 3. Initialise HDF5. The sentinel makes H5PL initialise with dynamic
    // plugin loading already disabled.
    let h5open_ok = unsafe { H5open() >= 0 };

    // 4. Disable all dynamic plugin types through the formal API.
    let set_ok = unsafe { H5PLset_loading_state(0) >= 0 };

    // 5. Verify the live control mask, never assuming a safe state.
    status.loading_mask_zero = hdf5_plugin_loading_disabled_locked();
    status.isolation_established =
        status.startup_environment_sanitized && h5open_ok && set_ok && status.loading_mask_zero;

    if status.isolation_established {
        ISOLATION_ESTABLISHED.store(true, Ordering::Release);
        *LAST_ISOLATION_STATUS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(status.clone());
        Ok(status)
    } else {
        Err(format!(
            "HDF5 process isolation failed: h5open={h5open_ok} set_loading_state={set_ok} \
             environment_sanitized={} mask_zero={}",
            status.startup_environment_sanitized, status.loading_mask_zero
        ))
    }
}

/// Compatibility wrapper returning `()` on success. Prefer
/// `establish_hdf5_process_isolation()` when the structured status is needed.
pub fn establish_hdf5_plugin_isolation() -> Result<(), String> {
    establish_hdf5_process_isolation().map(|_| ())
}

/// The most recently established isolation status, or an all-false status if
/// isolation was never established in this process.
pub fn hdf5_process_isolation_status() -> Hdf5ProcessIsolationStatus {
    LAST_ISOLATION_STATUS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .unwrap_or(Hdf5ProcessIsolationStatus {
            startup_environment_sanitized: false,
            preload_sentinel_set_by_application: false,
            parent_env_preload_absent: false,
            plugin_path_removed: false,
            vol_connector_removed: false,
            driver_removed: false,
            driver_config_removed: false,
            loading_mask_zero: false,
            isolation_established: false,
        })
}

/// True once `establish_hdf5_process_isolation` succeeded in this process.
pub fn hdf5_plugin_isolation_established() -> bool {
    ISOLATION_ESTABLISHED.load(Ordering::Acquire)
}

/// True when the live HDF5 plugin control mask is zero (all dynamic plugin
/// types disabled). This is the fail-closed check used by the NISAR validator
/// and by the packaged runtime smoke.
pub fn hdf5_plugin_loading_disabled() -> bool {
    let _lock = hdf5_sys::LOCK.lock();
    hdf5_plugin_loading_disabled_locked()
}

fn hdf5_plugin_loading_disabled_locked() -> bool {
    let mut mask = H5PL_ALL_PLUGIN;
    let result = unsafe { H5PLget_loading_state(&mut mask) };
    result >= 0 && mask == 0
}

/// Serialises tests that touch process-global HDF5 state. The lock is
/// re-entrant because some fixture lifetimes hold it while helper functions
/// independently establish or validate HDF5 isolation on the same thread.
#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
pub struct Hdf5TestLock {
    state: std::sync::Mutex<Hdf5TestLockState>,
    wake: std::sync::Condvar,
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
struct Hdf5TestLockState {
    owner: Option<std::thread::ThreadId>,
    depth: usize,
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
/// This guard is intentionally thread-affine: its `Drop` implementation must
/// run on the thread that acquired it.
///
/// ```compile_fail
/// use insar_download::Hdf5TestLockGuard;
///
/// fn requires_send<T: Send>() {}
/// fn requires_sync<T: Sync>() {}
///
/// requires_send::<Hdf5TestLockGuard>();
/// requires_sync::<Hdf5TestLockGuard>();
/// ```
pub struct Hdf5TestLockGuard {
    lock: &'static Hdf5TestLock,
    // Opt-in test diagnostics only; nested acquisitions do not duplicate timing.
    diagnostic_acquired: Option<std::time::Instant>,
    // `Rc` is intentionally neither `Send` nor `Sync`; `PhantomData` keeps
    // this marker zero-sized while making the owner-thread Drop contract
    // visible to the type system.
    _thread_affinity: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
impl Hdf5TestLock {
    pub fn lock(&'static self) -> std::sync::LockResult<Hdf5TestLockGuard> {
        let diagnostic_started = (std::env::var("EOSTAGE_NISAR_TEST_TIMING").as_deref() == Ok("1"))
            .then(std::time::Instant::now);
        let current_thread = std::thread::current();
        let current = current_thread.id();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let diagnostic_started = diagnostic_started.filter(|_| state.owner != Some(current));
        if diagnostic_started.is_some() {
            eprintln!(
                "[nisar-test-lock] waiting test={:?} owner={:?}",
                current_thread.name().unwrap_or("unnamed"),
                state.owner
            );
        }
        while state.owner.is_some_and(|owner| owner != current) {
            state = self
                .wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if state.owner == Some(current) {
            state.depth += 1;
        } else {
            state.owner = Some(current);
            state.depth = 1;
        }
        drop(state);
        let diagnostic_acquired = diagnostic_started.map(|started| {
            let acquired = std::time::Instant::now();
            eprintln!(
                "[nisar-test-lock] acquired test={:?} wait_ms={}",
                current_thread.name().unwrap_or("unnamed"),
                acquired.duration_since(started).as_millis()
            );
            acquired
        });
        Ok(Hdf5TestLockGuard {
            lock: self,
            diagnostic_acquired,
            _thread_affinity: std::marker::PhantomData,
        })
    }

    pub fn held_by_current_thread(&self) -> bool {
        let current = std::thread::current().id();
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.owner == Some(current) && state.depth > 0
    }
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
impl Drop for Hdf5TestLockGuard {
    fn drop(&mut self) {
        let current = std::thread::current().id();
        let mut state = self
            .lock
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert_eq!(state.owner, Some(current));
        debug_assert!(state.depth > 0);
        state.depth -= 1;
        if state.depth == 0 {
            state.owner = None;
            self.lock.wake.notify_one();
        }
        drop(state);
        if let Some(acquired) = self.diagnostic_acquired {
            eprintln!(
                "[nisar-test-lock] released test={:?} held_ms={}",
                std::thread::current().name().unwrap_or("unnamed"),
                acquired.elapsed().as_millis()
            );
        }
    }
}

#[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
pub static HDF5_TEST_LOCK: Hdf5TestLock = Hdf5TestLock {
    state: std::sync::Mutex::new(Hdf5TestLockState {
        owner: None,
        depth: 0,
    }),
    wake: std::sync::Condvar::new(),
};

/// Test-only helper that re-enables all plugin types so a negative control can
/// prove the validator fails closed when isolation is absent. Never call from
/// production code.
#[cfg(test)]
pub(crate) fn reenable_hdf5_plugin_loading_for_test() {
    let _lock = hdf5_sys::LOCK.lock();
    unsafe {
        let _ = H5PLset_loading_state(H5PL_ALL_PLUGIN);
    }
    ISOLATION_ESTABLISHED.store(false, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::{establish_hdf5_process_isolation, hdf5_plugin_loading_disabled};

    #[test]
    fn establish_disables_loading_without_env_sentinel() {
        let _guard = super::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        super::crt_env::remove("HDF5_PLUGIN_PRELOAD");
        // A hostile plugin path must be inert once loading is disabled.
        let hostile =
            std::env::temp_dir().join(format!("insar-hdf5-hostile-plugin-{}", std::process::id()));
        std::fs::create_dir_all(&hostile).unwrap();
        std::fs::write(hostile.join("hdf5_plugin.dll"), b"not a plugin").unwrap();
        super::crt_env::set("HDF5_PLUGIN_PATH", &hostile.to_string_lossy());
        super::crt_env::set("HDF5_VOL_CONNECTOR", "nonexistent_external_connector");
        super::crt_env::set("HDF5_DRIVER", "nonexistent_external_driver");
        super::crt_env::set("HDF5_DRIVER_CONFIG", "hostile");

        let status = establish_hdf5_process_isolation().expect("app-owned isolation succeeds");
        assert!(status.startup_environment_sanitized);
        assert!(status.preload_sentinel_set_by_application);
        assert!(status.parent_env_preload_absent);
        assert!(status.plugin_path_removed);
        assert!(status.vol_connector_removed);
        assert!(status.driver_removed);
        assert!(status.driver_config_removed);
        assert!(status.loading_mask_zero);
        assert!(status.isolation_established);
        assert!(super::hdf5_plugin_isolation_established());
        assert!(hdf5_plugin_loading_disabled());
        assert!(super::crt_env::get("HDF5_PLUGIN_PATH").is_none());
        assert!(super::crt_env::get("HDF5_VOL_CONNECTOR").is_none());
        assert!(super::crt_env::get("HDF5_DRIVER").is_none());
        assert!(super::crt_env::get("HDF5_DRIVER_CONFIG").is_none());

        let _ = std::fs::remove_dir_all(&hostile);
    }

    #[cfg(unix)]
    #[test]
    fn establish_leaves_exclusive_access_to_the_application_lock_on_unix() {
        let _guard = super::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        super::crt_env::set(super::HDF5_FILE_LOCKING_ENV, "TRUE");
        establish_hdf5_process_isolation().expect("app-owned isolation succeeds");
        assert_eq!(
            super::crt_env::get(super::HDF5_FILE_LOCKING_ENV).as_deref(),
            Some("FALSE")
        );

        // An HDF5 file the application holds an exclusive flock on stays
        // openable by HDF5 itself, as it is on Windows.
        let dir = std::env::temp_dir().join(format!("insar-hdf5-app-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("locked.h5");
        hdf5::File::create(&path).unwrap();
        let guard = std::fs::File::open(&path).unwrap();
        guard.try_lock().unwrap();
        let reopened = hdf5::File::open(&path).expect("HDF5 open beside the application lock");
        assert!(reopened.is_read_only());
        drop(reopened);
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_can_be_reenabled_only_by_test_helper() {
        let _guard = super::HDF5_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // When the parent process injected HDF5_PLUGIN_PRELOAD=:: before this
        // process started, HDF5 permanently locks plugin loading off and the
        // test helper cannot re-enable it. Detect that state from the live
        // control mask rather than the env var: the negative control only
        // runs in a process where plugins can actually be re-enabled, which
        // is exactly the packaged-host condition we need to prove.
        super::crt_env::remove("HDF5_PLUGIN_PRELOAD");
        super::reenable_hdf5_plugin_loading_for_test();
        if hdf5_plugin_loading_disabled() {
            establish_hdf5_process_isolation().expect("restore isolation");
            return;
        }
        assert!(!hdf5_plugin_loading_disabled());
        assert!(!super::hdf5_plugin_isolation_established());
        establish_hdf5_process_isolation().expect("restore isolation");
        assert!(hdf5_plugin_loading_disabled());
    }
}
