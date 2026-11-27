//! Internal D4A2A qualification seam for process-local destination ownership.

use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use crate::WorkerControl;

const WAIT_INTERVAL: Duration = Duration::from_millis(50);

/// An opaque process-local identity for a physical destination.
///
/// The contained label is intentionally unavailable to callers and its debug
/// representation is redacted. D4A2A does not resolve path aliases or establish
/// any cross-process identity.
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
#[doc(hidden)]
pub struct OpaqueDestinationKey(u64);

impl fmt::Debug for OpaqueDestinationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueDestinationKey { redacted: true }")
    }
}

/// Builds an opaque key for deterministic qualification tests only.
#[doc(hidden)]
pub fn destination_lease_key_for_qualification_test(label: u64) -> OpaqueDestinationKey {
    OpaqueDestinationKey(label)
}

/// A complete lexical destination identity retained only in process memory.
///
/// This deliberately represents the planned path rather than a hash. It does
/// not resolve symlink, case, short-name, or cross-process aliases.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct RuntimeDestinationIdentity {
    normalized: PathBuf,
}

impl RuntimeDestinationIdentity {
    fn from_planned_path(path: &Path) -> Result<Self, DestinationLeaseError> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|_| DestinationLeaseError::InvalidIdentity)?
                .join(path)
        };
        let normalized = normalize_lexical_path(&absolute)?;
        Ok(Self { normalized })
    }
}

impl fmt::Debug for RuntimeDestinationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeDestinationIdentity { redacted: true }")
    }
}

#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
enum DestinationIdentity {
    Qualification(OpaqueDestinationKey),
    Runtime(RuntimeDestinationIdentity),
}

impl fmt::Debug for DestinationIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DestinationIdentity { redacted: true }")
    }
}

fn normalize_lexical_path(path: &Path) -> Result<PathBuf, DestinationLeaseError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(DestinationLeaseError::InvalidIdentity);
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    if normalized.as_os_str().is_empty() {
        Err(DestinationLeaseError::InvalidIdentity)
    } else {
        Ok(normalized)
    }
}

/// A process-local, in-memory destination lease controller.
#[doc(hidden)]
pub struct DestinationLeaseController {
    state: Mutex<LeaseState>,
    changed: Condvar,
}

impl fmt::Debug for DestinationLeaseController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DestinationLeaseController { redacted: true }")
    }
}

/// A non-cloneable owned destination lease released by `Drop`.
#[doc(hidden)]
pub struct DestinationLeaseGuard {
    controller: Arc<DestinationLeaseController>,
    token: Option<LeaseToken>,
}

impl fmt::Debug for DestinationLeaseGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DestinationLeaseGuard { redacted: true }")
    }
}

impl Drop for DestinationLeaseGuard {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            self.controller.release(token);
        }
    }
}

/// Bounded diagnostics for D4A2A qualification tests.
#[derive(Clone, Copy, Eq, PartialEq)]
#[doc(hidden)]
pub struct DestinationLeaseSnapshot {
    live_destinations: usize,
    queued_waiters: usize,
    active_leases: usize,
    failed_closed: bool,
}

impl DestinationLeaseSnapshot {
    #[doc(hidden)]
    pub fn live_destinations(&self) -> usize {
        self.live_destinations
    }

    #[doc(hidden)]
    pub fn queued_waiters(&self) -> usize {
        self.queued_waiters
    }

    #[doc(hidden)]
    pub fn active_leases(&self) -> usize {
        self.active_leases
    }

    #[doc(hidden)]
    pub fn failed_closed(&self) -> bool {
        self.failed_closed
    }
}

impl fmt::Debug for DestinationLeaseSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DestinationLeaseSnapshot")
            .field("live_destinations", &self.live_destinations)
            .field("queued_waiters", &self.queued_waiters)
            .field("active_leases", &self.active_leases)
            .field("failed_closed", &self.failed_closed)
            .finish()
    }
}

/// Typed, non-secret acquisition failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub enum DestinationLeaseError {
    Cancelled,
    InvalidIdentity,
    Unavailable,
    InvariantViolation,
}

impl fmt::Display for DestinationLeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Cancelled => "destination lease acquisition was cancelled",
            Self::InvalidIdentity => "destination lease identity is invalid",
            Self::Unavailable => "destination lease controller is unavailable",
            Self::InvariantViolation => "destination lease controller invariant failed",
        };
        formatter.write_str(message)
    }
}

impl Error for DestinationLeaseError {}

impl DestinationLeaseController {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(LeaseState::new()),
            changed: Condvar::new(),
        })
    }

    /// Constructs the runtime-only controller used by focused qualification tests.
    #[doc(hidden)]
    pub fn new_for_qualification_test() -> Arc<Self> {
        Self::new()
    }

    /// Acquires a production lease using only WorkerControl atomic probes.
    ///
    /// This API intentionally accepts no external callbacks. A queued paused
    /// worker stays queued, while an owner keeps its guard through pause.
    #[doc(hidden)]
    pub fn acquire_for_worker(
        self: &Arc<Self>,
        destination: &Path,
        control: &WorkerControl,
    ) -> Result<DestinationLeaseGuard, DestinationLeaseError> {
        let identity = RuntimeDestinationIdentity::from_planned_path(destination)?;
        self.acquire(
            DestinationIdentity::Runtime(identity),
            || control.is_cancelled(),
            || control.is_paused(),
        )
    }

    /// Downloader D3-2: takes the lease of an idle destination for the local
    /// pre-scan without waiting. A destination with an owner or a waiter is
    /// busy (`Ok(None)`) and stays with its worker.
    pub(crate) fn try_acquire_for_scan(
        self: &Arc<Self>,
        destination: &Path,
    ) -> Result<Option<DestinationLeaseGuard>, DestinationLeaseError> {
        let identity = DestinationIdentity::Runtime(RuntimeDestinationIdentity::from_planned_path(
            destination,
        )?);
        let mut state = self.lock_state()?;
        if let Some(error) = state.failure.error() {
            return Err(error);
        }
        if state.destinations.contains_key(&identity) {
            return Ok(None);
        }
        let Some(next_waiter_id) = state.next_waiter_id.checked_add(1) else {
            state.fail_closed(FailureState::Unavailable);
            drop(state);
            self.changed.notify_all();
            return Err(DestinationLeaseError::Unavailable);
        };
        let owner = WaiterId(state.next_waiter_id);
        state.next_waiter_id = next_waiter_id;
        state.destinations.insert(
            identity.clone(),
            DestinationEntry {
                owner: Some(owner),
                waiters: VecDeque::new(),
            },
        );
        Ok(Some(DestinationLeaseGuard {
            controller: Arc::clone(self),
            token: Some(LeaseToken {
                destination: identity,
                waiter: owner,
            }),
        }))
    }

    pub(crate) fn acquire_for_runtime<C>(
        self: &Arc<Self>,
        destination: &Path,
        is_cancelled: C,
    ) -> Result<DestinationLeaseGuard, DestinationLeaseError>
    where
        C: Fn() -> bool,
    {
        let identity = RuntimeDestinationIdentity::from_planned_path(destination)?;
        self.acquire(DestinationIdentity::Runtime(identity), is_cancelled, || {
            false
        })
    }

    /// Like `acquire_for_runtime`, and calls `waiting`, outside the controller
    /// mutex, before the first wait for another owner and after every wait
    /// interval while it waits (downloader enhancement D1: the NISAR writer
    /// shows that it waits for its output; D1b: and why, once it waits long).
    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn acquire_for_runtime_observed<C>(
        self: &Arc<Self>,
        destination: &Path,
        is_cancelled: C,
        waiting: &mut dyn FnMut(),
    ) -> Result<DestinationLeaseGuard, DestinationLeaseError>
    where
        C: Fn() -> bool,
    {
        let identity = RuntimeDestinationIdentity::from_planned_path(destination)?;
        self.acquire_observed(
            DestinationIdentity::Runtime(identity),
            is_cancelled,
            || false,
            waiting,
        )
    }

    /// Acquires one destination lease while observing qualification probes.
    ///
    /// A queued paused waiter stays queued and does not acquire. A paused owner
    /// continues to hold its lease until its guard drops. Neither callback runs
    /// while the controller mutex is held.
    #[doc(hidden)]
    pub fn acquire_for_qualification_test<C, P>(
        self: &Arc<Self>,
        destination: OpaqueDestinationKey,
        is_cancelled: C,
        is_paused: P,
    ) -> Result<DestinationLeaseGuard, DestinationLeaseError>
    where
        C: Fn() -> bool,
        P: Fn() -> bool,
    {
        self.acquire(
            DestinationIdentity::Qualification(destination),
            is_cancelled,
            is_paused,
        )
    }

    fn acquire<C, P>(
        self: &Arc<Self>,
        destination: DestinationIdentity,
        is_cancelled: C,
        is_paused: P,
    ) -> Result<DestinationLeaseGuard, DestinationLeaseError>
    where
        C: Fn() -> bool,
        P: Fn() -> bool,
    {
        self.acquire_observed(destination, is_cancelled, is_paused, &mut || {})
    }

    fn acquire_observed<C, P>(
        self: &Arc<Self>,
        destination: DestinationIdentity,
        is_cancelled: C,
        is_paused: P,
        waiting: &mut dyn FnMut(),
    ) -> Result<DestinationLeaseGuard, DestinationLeaseError>
    where
        C: Fn() -> bool,
        P: Fn() -> bool,
    {
        let waiter = self.register_waiter(destination.clone(), &is_cancelled)?;
        let mut announced = false;

        loop {
            if is_cancelled() {
                self.remove_waiter(&destination, waiter);
                return Err(DestinationLeaseError::Cancelled);
            }
            let paused = is_paused();

            let mut state = self.lock_state()?;
            if let Some(error) = state.failure.error() {
                return Err(error);
            }
            if !state.destinations.contains_key(&destination) {
                state.fail_closed(FailureState::Invariant);
                drop(state);
                self.changed.notify_all();
                return Err(DestinationLeaseError::InvariantViolation);
            }

            let can_grant = state.destinations.get(&destination).is_some_and(|entry| {
                !paused && entry.owner.is_none() && entry.waiters.front() == Some(&waiter)
            });
            if can_grant {
                let Some(entry) = state.destinations.get_mut(&destination) else {
                    state.fail_closed(FailureState::Invariant);
                    drop(state);
                    self.changed.notify_all();
                    return Err(DestinationLeaseError::InvariantViolation);
                };
                if entry.waiters.pop_front() != Some(waiter) {
                    state.fail_closed(FailureState::Invariant);
                    drop(state);
                    self.changed.notify_all();
                    return Err(DestinationLeaseError::InvariantViolation);
                }
                entry.owner = Some(waiter);
                return Ok(DestinationLeaseGuard {
                    controller: Arc::clone(self),
                    token: Some(LeaseToken {
                        destination,
                        waiter,
                    }),
                });
            }
            if !announced {
                announced = true;
                drop(state);
                waiting();
                continue;
            }

            match self.changed.wait_timeout(state, WAIT_INTERVAL) {
                Ok((state, _)) => drop(state),
                Err(poisoned) => {
                    let (mut state, _) = poisoned.into_inner();
                    state.fail_closed(FailureState::Unavailable);
                    drop(state);
                    self.changed.notify_all();
                    return Err(DestinationLeaseError::Unavailable);
                }
            }
            waiting();
        }
    }

    /// Returns bounded state only; no destination identity is exposed.
    #[doc(hidden)]
    pub fn snapshot_for_qualification_test(&self) -> DestinationLeaseSnapshot {
        match self.state.lock() {
            Ok(state) => DestinationLeaseSnapshot::from_state(&state),
            Err(_) => DestinationLeaseSnapshot {
                live_destinations: 0,
                queued_waiters: 0,
                active_leases: 0,
                failed_closed: true,
            },
        }
    }

    /// Forces checked waiter-label exhaustion for focused fail-closed evidence.
    #[doc(hidden)]
    pub fn force_id_exhaustion_for_qualification_test(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.next_waiter_id = u64::MAX;
            self.changed.notify_all();
        }
    }

    /// Forces an invariant failure for focused fail-closed evidence.
    #[doc(hidden)]
    pub fn force_invariant_failure_for_qualification_test(&self) {
        self.fail_closed_for_qualification_test(FailureState::Invariant);
    }

    /// Poisons the controller mutex for focused fail-closed evidence.
    #[doc(hidden)]
    pub fn poison_for_qualification_test(&self) {
        let _ = panic::catch_unwind(AssertUnwindSafe(|| {
            let _state = self
                .state
                .lock()
                .expect("new qualification controller mutex");
            panic!("intentional destination lease qualification poison");
        }));
    }

    fn register_waiter<C>(
        &self,
        destination: DestinationIdentity,
        is_cancelled: &C,
    ) -> Result<WaiterId, DestinationLeaseError>
    where
        C: Fn() -> bool,
    {
        if is_cancelled() {
            return Err(DestinationLeaseError::Cancelled);
        }

        let mut state = self.lock_state()?;
        if let Some(error) = state.failure.error() {
            return Err(error);
        }
        let next_waiter_id = match state.next_waiter_id.checked_add(1) {
            Some(next_waiter_id) => next_waiter_id,
            None => {
                state.fail_closed(FailureState::Unavailable);
                drop(state);
                self.changed.notify_all();
                return Err(DestinationLeaseError::Unavailable);
            }
        };
        let waiter = WaiterId(state.next_waiter_id);
        state.next_waiter_id = next_waiter_id;
        state
            .destinations
            .entry(destination)
            .or_default()
            .waiters
            .push_back(waiter);
        self.changed.notify_all();
        Ok(waiter)
    }

    fn remove_waiter(&self, destination: &DestinationIdentity, waiter: WaiterId) {
        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(_) => return,
        };
        if !state.failure.is_open() {
            return;
        }
        let Some(entry) = state.destinations.get_mut(destination) else {
            state.fail_closed(FailureState::Invariant);
            drop(state);
            self.changed.notify_all();
            return;
        };
        if entry.owner == Some(waiter) {
            state.fail_closed(FailureState::Invariant);
            drop(state);
            self.changed.notify_all();
            return;
        }
        let queued_before = entry.waiters.len();
        entry.waiters.retain(|candidate| *candidate != waiter);
        if entry.waiters.len() == queued_before {
            state.fail_closed(FailureState::Invariant);
            drop(state);
            self.changed.notify_all();
            return;
        }
        state.remove_destination_if_idle(destination);
        drop(state);
        self.changed.notify_all();
    }

    fn release(&self, token: LeaseToken) {
        let mut state = match self.lock_state() {
            Ok(state) => state,
            Err(_) => return,
        };
        if !state.failure.is_open() {
            return;
        }
        let Some(entry) = state.destinations.get_mut(&token.destination) else {
            state.fail_closed(FailureState::Invariant);
            drop(state);
            self.changed.notify_all();
            return;
        };
        if entry.owner != Some(token.waiter) {
            state.fail_closed(FailureState::Invariant);
            drop(state);
            self.changed.notify_all();
            return;
        }
        entry.owner = None;
        state.remove_destination_if_idle(&token.destination);
        drop(state);
        self.changed.notify_all();
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, LeaseState>, DestinationLeaseError> {
        match self.state.lock() {
            Ok(state) => Ok(state),
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.fail_closed(FailureState::Unavailable);
                drop(state);
                self.changed.notify_all();
                Err(DestinationLeaseError::Unavailable)
            }
        }
    }

    fn fail_closed_for_qualification_test(&self, failure: FailureState) {
        match self.state.lock() {
            Ok(mut state) => {
                state.fail_closed(failure);
                drop(state);
                self.changed.notify_all();
            }
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.fail_closed(FailureState::Unavailable);
                drop(state);
                self.changed.notify_all();
            }
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct WaiterId(u64);

#[derive(Clone)]
struct LeaseToken {
    destination: DestinationIdentity,
    waiter: WaiterId,
}

#[derive(Default)]
struct DestinationEntry {
    owner: Option<WaiterId>,
    waiters: VecDeque<WaiterId>,
}

struct LeaseState {
    destinations: BTreeMap<DestinationIdentity, DestinationEntry>,
    next_waiter_id: u64,
    failure: FailureState,
}

impl LeaseState {
    fn new() -> Self {
        Self {
            destinations: BTreeMap::new(),
            next_waiter_id: 1,
            failure: FailureState::Open,
        }
    }

    fn fail_closed(&mut self, failure: FailureState) {
        if self.failure.is_open() {
            self.failure = failure;
        }
        self.destinations.clear();
    }

    fn remove_destination_if_idle(&mut self, destination: &DestinationIdentity) {
        let is_idle = self
            .destinations
            .get(destination)
            .is_some_and(|entry| entry.owner.is_none() && entry.waiters.is_empty());
        if is_idle {
            self.destinations.remove(destination);
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FailureState {
    Open,
    Unavailable,
    Invariant,
}

impl FailureState {
    fn is_open(self) -> bool {
        self == Self::Open
    }

    fn error(self) -> Option<DestinationLeaseError> {
        match self {
            Self::Open => None,
            Self::Unavailable => Some(DestinationLeaseError::Unavailable),
            Self::Invariant => Some(DestinationLeaseError::InvariantViolation),
        }
    }
}

impl DestinationLeaseSnapshot {
    fn from_state(state: &LeaseState) -> Self {
        Self {
            live_destinations: state.destinations.len(),
            queued_waiters: state
                .destinations
                .values()
                .map(|entry| entry.waiters.len())
                .sum(),
            active_leases: state
                .destinations
                .values()
                .filter(|entry| entry.owner.is_some())
                .count(),
            failed_closed: !state.failure.is_open(),
        }
    }
}
