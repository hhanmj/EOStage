use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::num::NonZeroUsize;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use insar_core::{DownloadFailure, DownloadOutcome};
use reqwest::Url;

use crate::transfer_backend::ValidatedTransferDispatch;
use crate::{
    ArtifactValidationPlan, AuthRefreshPolicyId, ResolvedAsset, ResolvedTransferResult,
    ResolvedTransferSession, TransferAuth, TransferBackend, TransferBackendKind, TransferRequest,
    TransferResult, TransferUpdate, ValidatedResolvedTransferResult,
};

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub enum TransferAdmissionPolicy {
    Compatibility,
    Finite {
        global_limit: NonZeroUsize,
        per_host_limit: NonZeroUsize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[doc(hidden)]
pub struct OpaqueHostLabel(u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[doc(hidden)]
pub struct AdmissionWaitReasonCounts {
    pub global_limited: u64,
    pub host_limited: u64,
    pub paused: u64,
    pub no_grantable_waiter: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub struct TransferAdmissionSnapshot {
    pub policy: TransferAdmissionPolicy,
    pub active_global: usize,
    pub active_by_host: BTreeMap<OpaqueHostLabel, usize>,
    pub queued_global: usize,
    pub queued_by_host: BTreeMap<OpaqueHostLabel, usize>,
    pub paused_waiters: usize,
    pub grants: u64,
    pub releases: u64,
    pub cancellations_before_grant: u64,
    pub prestart_releases: u64,
    pub waited_count: u64,
    pub total_wait_ms: u64,
    pub max_wait_ms: u64,
    pub max_active_global: usize,
    pub live_max_active_by_host: BTreeMap<OpaqueHostLabel, usize>,
    pub waiting_reason_counts: AdmissionWaitReasonCounts,
    pub failed_closed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdmissionAcquireError {
    Cancelled,
    FailedClosed,
}

#[derive(Clone, Eq, PartialEq, Hash)]
pub(crate) struct NormalizedOrigin {
    scheme: OriginScheme,
    host: OriginHost,
    port: u16,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
enum OriginScheme {
    Http,
    Https,
}

#[derive(Clone, Eq, PartialEq, Hash)]
enum OriginHost {
    Dns(String),
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
}

impl NormalizedOrigin {
    pub(crate) fn from_url(url: &Url) -> Option<Self> {
        let scheme = match url.scheme() {
            "http" => OriginScheme::Http,
            "https" => OriginScheme::Https,
            _ => return None,
        };
        let host = url.host_str()?;
        let host = if let Ok(address) = host.parse::<Ipv4Addr>() {
            OriginHost::Ipv4(address)
        } else if let Ok(address) = host.parse::<Ipv6Addr>() {
            OriginHost::Ipv6(address)
        } else {
            OriginHost::Dns(host.to_ascii_lowercase())
        };
        Some(Self {
            scheme,
            host,
            port: url.port_or_known_default()?,
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
struct HostKey(u64);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
struct TaskKey(u64);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
struct WaiterId(u64);

#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
struct PermitToken(u64);

struct HostState {
    queued: usize,
    active: usize,
    live_max_active: usize,
}

struct TaskQueue {
    waiters: VecDeque<WaiterId>,
}

struct AdmissionWaiter {
    id: WaiterId,
    task_key: TaskKey,
    task_identity: String,
    host: HostKey,
    enqueued_at: Instant,
}

struct QueuedWaiter {
    waiter: AdmissionWaiter,
    paused: bool,
}

struct AdmissionState {
    failed_closed: bool,
    next_identity: u64,
    active_global: usize,
    origins: HashMap<NormalizedOrigin, HostKey>,
    hosts: HashMap<HostKey, HostState>,
    tasks_by_identity: HashMap<String, TaskKey>,
    tasks: HashMap<TaskKey, TaskQueue>,
    task_ring: VecDeque<TaskKey>,
    waiters: HashMap<WaiterId, QueuedWaiter>,
    permits: HashMap<PermitToken, HostKey>,
    grants: u64,
    releases: u64,
    cancellations_before_grant: u64,
    prestart_releases: u64,
    waited_count: u64,
    total_wait_ms: u64,
    max_wait_ms: u64,
    max_active_global: usize,
}

impl Default for AdmissionState {
    fn default() -> Self {
        Self {
            failed_closed: false,
            next_identity: 1,
            active_global: 0,
            origins: HashMap::new(),
            hosts: HashMap::new(),
            tasks_by_identity: HashMap::new(),
            tasks: HashMap::new(),
            task_ring: VecDeque::new(),
            waiters: HashMap::new(),
            permits: HashMap::new(),
            grants: 0,
            releases: 0,
            cancellations_before_grant: 0,
            prestart_releases: 0,
            waited_count: 0,
            total_wait_ms: 0,
            max_wait_ms: 0,
            max_active_global: 0,
        }
    }
}

impl AdmissionState {
    fn allocate_identity(&mut self) -> Result<u64, ()> {
        let identity = self.next_identity;
        self.next_identity = identity.checked_add(1).ok_or(())?;
        Ok(identity)
    }

    fn fail_closed(&mut self) {
        self.failed_closed = true;
        self.active_global = 0;
        self.origins.clear();
        self.hosts.clear();
        self.tasks_by_identity.clear();
        self.tasks.clear();
        self.task_ring.clear();
        self.waiters.clear();
        self.permits.clear();
    }

    fn register_waiter(
        &mut self,
        origin: &NormalizedOrigin,
        task_identity: &str,
    ) -> Result<AdmissionWaiter, ()> {
        if self.failed_closed {
            return Err(());
        }

        let host_key = self.origins.get(origin).copied();
        let task_key = self.tasks_by_identity.get(task_identity).copied();
        let new_host = if host_key.is_none() {
            Some(HostKey(self.allocate_identity()?))
        } else {
            None
        };
        let new_task = if task_key.is_none() {
            Some(TaskKey(self.allocate_identity()?))
        } else {
            None
        };
        let waiter_id = WaiterId(self.allocate_identity()?);
        let host = host_key.or(new_host).ok_or(())?;
        let task_key = task_key.or(new_task).ok_or(())?;

        if let Some(host_key) = new_host {
            self.origins.insert(origin.clone(), host_key);
            self.hosts.insert(
                host_key,
                HostState {
                    queued: 0,
                    active: 0,
                    live_max_active: 0,
                },
            );
        }
        if let Some(task_key) = new_task {
            self.tasks_by_identity
                .insert(task_identity.to_string(), task_key);
            self.tasks.insert(
                task_key,
                TaskQueue {
                    waiters: VecDeque::new(),
                },
            );
            self.task_ring.push_back(task_key);
        }

        let waiter = AdmissionWaiter {
            id: waiter_id,
            task_key,
            task_identity: task_identity.to_string(),
            host,
            enqueued_at: Instant::now(),
        };
        self.insert_waiter(waiter, false)?;
        Ok(self
            .waiters
            .get(&waiter_id)
            .ok_or(())?
            .waiter
            .clone_for_transfer())
    }

    fn insert_waiter(&mut self, waiter: AdmissionWaiter, paused: bool) -> Result<(), ()> {
        let host = self.hosts.get_mut(&waiter.host).ok_or(())?;
        host.queued = host.queued.checked_add(1).ok_or(())?;
        let queue = self.tasks.get_mut(&waiter.task_key).ok_or(())?;
        if queue.waiters.iter().any(|id| *id == waiter.id) {
            return Err(());
        }
        let index = queue
            .waiters
            .iter()
            .position(|id| *id > waiter.id)
            .unwrap_or(queue.waiters.len());
        queue.waiters.insert(index, waiter.id);
        if self
            .waiters
            .insert(waiter.id, QueuedWaiter { waiter, paused })
            .is_some()
        {
            return Err(());
        }
        Ok(())
    }

    fn set_waiter_paused(&mut self, waiter_id: WaiterId, paused: bool) -> Result<bool, ()> {
        let waiter = self.waiters.get_mut(&waiter_id).ok_or(())?;
        let changed = waiter.paused != paused;
        waiter.paused = paused;
        Ok(changed)
    }

    fn select_grantable(&self, policy: TransferAdmissionPolicy) -> Result<Option<WaiterId>, ()> {
        if self.failed_closed {
            return Err(());
        }
        if self.waiters.is_empty() != self.task_ring.is_empty()
            || self.tasks.len() != self.task_ring.len()
        {
            return Err(());
        }
        let ring_len = self.task_ring.len();
        for task_key in self.task_ring.iter().take(ring_len) {
            let task = self.tasks.get(task_key).ok_or(())?;
            if task.waiters.is_empty() {
                return Err(());
            }
            for waiter_id in &task.waiters {
                let queued = self.waiters.get(waiter_id).ok_or(())?;
                if queued.paused || !self.host_can_grant(queued.waiter.host, policy)? {
                    continue;
                }
                return Ok(Some(*waiter_id));
            }
        }
        Ok(None)
    }

    fn host_can_grant(&self, host: HostKey, policy: TransferAdmissionPolicy) -> Result<bool, ()> {
        match policy {
            TransferAdmissionPolicy::Compatibility => Ok(true),
            TransferAdmissionPolicy::Finite {
                global_limit,
                per_host_limit,
            } => Ok(self.active_global < global_limit.get()
                && self.hosts.get(&host).ok_or(())?.active < per_host_limit.get()),
        }
    }

    fn grant(&mut self, waiter_id: WaiterId) -> Result<(PermitToken, AdmissionWaiter), ()> {
        let queued = self.waiters.remove(&waiter_id).ok_or(())?;
        let waiter = queued.waiter;
        let task_has_waiters = self.remove_waiter_from_task(&waiter)?;
        let host = self.hosts.get_mut(&waiter.host).ok_or(())?;
        host.queued = host.queued.checked_sub(1).ok_or(())?;

        let token = PermitToken(self.allocate_identity()?);
        self.active_global = self.active_global.checked_add(1).ok_or(())?;
        self.max_active_global = self.max_active_global.max(self.active_global);
        let host = self.hosts.get_mut(&waiter.host).ok_or(())?;
        host.active = host.active.checked_add(1).ok_or(())?;
        host.live_max_active = host.live_max_active.max(host.active);
        if self.permits.insert(token, waiter.host).is_some() {
            return Err(());
        }
        self.grants = self.grants.saturating_add(1);
        let waited_ms = u64::try_from(waiter.enqueued_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.waited_count = self.waited_count.saturating_add(1);
        self.total_wait_ms = self.total_wait_ms.saturating_add(waited_ms);
        self.max_wait_ms = self.max_wait_ms.max(waited_ms);
        if task_has_waiters {
            self.rotate_task(waiter.task_key)?;
        }
        Ok((token, waiter))
    }

    fn cancel_waiter(&mut self, waiter_id: WaiterId) -> Result<(), ()> {
        let queued = self.waiters.remove(&waiter_id).ok_or(())?;
        self.remove_waiter_from_task(&queued.waiter)?;
        let host = self.hosts.get_mut(&queued.waiter.host).ok_or(())?;
        host.queued = host.queued.checked_sub(1).ok_or(())?;
        self.cancellations_before_grant = self.cancellations_before_grant.saturating_add(1);
        self.cleanup_host(queued.waiter.host);
        Ok(())
    }

    fn remove_waiter_from_task(&mut self, waiter: &AdmissionWaiter) -> Result<bool, ()> {
        let is_empty = {
            let task = self.tasks.get_mut(&waiter.task_key).ok_or(())?;
            let index = task
                .waiters
                .iter()
                .position(|id| *id == waiter.id)
                .ok_or(())?;
            task.waiters.remove(index);
            task.waiters.is_empty()
        };
        if is_empty {
            self.tasks.remove(&waiter.task_key);
            self.remove_task_from_ring(waiter.task_key)?;
            if self.tasks_by_identity.get(&waiter.task_identity) == Some(&waiter.task_key) {
                self.tasks_by_identity.remove(&waiter.task_identity);
            }
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn remove_task_from_ring(&mut self, task_key: TaskKey) -> Result<(), ()> {
        let position = self
            .task_ring
            .iter()
            .position(|key| *key == task_key)
            .ok_or(())?;
        self.task_ring.remove(position);
        Ok(())
    }

    fn rotate_task(&mut self, task_key: TaskKey) -> Result<(), ()> {
        self.remove_task_from_ring(task_key)?;
        self.task_ring.push_back(task_key);
        Ok(())
    }

    fn release_token(&mut self, token: PermitToken) -> Result<(), ()> {
        let host_key = self.permits.remove(&token).ok_or(())?;
        self.active_global = self.active_global.checked_sub(1).ok_or(())?;
        let host = self.hosts.get_mut(&host_key).ok_or(())?;
        host.active = host.active.checked_sub(1).ok_or(())?;
        self.releases = self.releases.saturating_add(1);
        self.cleanup_host(host_key);
        Ok(())
    }

    fn release_before_delegate(&mut self, token: PermitToken) -> Result<(), ()> {
        self.release_token(token)?;
        self.prestart_releases = self.prestart_releases.saturating_add(1);
        Ok(())
    }

    fn requeue_after_prestart_pause(
        &mut self,
        token: PermitToken,
        mut waiter: AdmissionWaiter,
    ) -> Result<AdmissionWaiter, ()> {
        let host_key = *self.permits.get(&token).ok_or(())?;
        if host_key != waiter.host {
            return Err(());
        }
        self.permits.remove(&token).ok_or(())?;
        self.active_global = self.active_global.checked_sub(1).ok_or(())?;
        let host = self.hosts.get_mut(&host_key).ok_or(())?;
        host.active = host.active.checked_sub(1).ok_or(())?;
        self.releases = self.releases.saturating_add(1);
        self.prestart_releases = self.prestart_releases.saturating_add(1);

        if let Some(existing_task) = self.tasks_by_identity.get(&waiter.task_identity).copied() {
            waiter.task_key = existing_task;
        } else {
            self.tasks_by_identity
                .insert(waiter.task_identity.clone(), waiter.task_key);
            self.tasks.insert(
                waiter.task_key,
                TaskQueue {
                    waiters: VecDeque::new(),
                },
            );
            self.task_ring.push_back(waiter.task_key);
        }
        let waiter_id = waiter.id;
        self.insert_waiter(waiter, true)?;
        let queued = self.waiters.get(&waiter_id).ok_or(())?;
        Ok(queued.waiter.clone_for_transfer())
    }

    fn cleanup_host(&mut self, host_key: HostKey) {
        let remove = self
            .hosts
            .get(&host_key)
            .is_some_and(|host| host.queued == 0 && host.active == 0);
        if remove {
            self.hosts.remove(&host_key);
            self.origins.retain(|_, key| *key != host_key);
        }
    }

    fn snapshot(&self, policy: TransferAdmissionPolicy) -> TransferAdmissionSnapshot {
        let mut active_by_host = BTreeMap::new();
        let mut queued_by_host = BTreeMap::new();
        let mut live_max_active_by_host = BTreeMap::new();
        for (host, state) in &self.hosts {
            let label = OpaqueHostLabel(host.0);
            active_by_host.insert(label, state.active);
            queued_by_host.insert(label, state.queued);
            live_max_active_by_host.insert(label, state.live_max_active);
        }
        let mut waiting_reason_counts = AdmissionWaitReasonCounts::default();
        let mut paused_waiters = 0;
        for queued in self.waiters.values() {
            if queued.paused {
                paused_waiters += 1;
                waiting_reason_counts.paused = waiting_reason_counts.paused.saturating_add(1);
                continue;
            }
            match policy {
                TransferAdmissionPolicy::Compatibility => {
                    waiting_reason_counts.no_grantable_waiter =
                        waiting_reason_counts.no_grantable_waiter.saturating_add(1);
                }
                TransferAdmissionPolicy::Finite {
                    global_limit,
                    per_host_limit: _,
                } if self.active_global >= global_limit.get() => {
                    waiting_reason_counts.global_limited =
                        waiting_reason_counts.global_limited.saturating_add(1);
                }
                TransferAdmissionPolicy::Finite {
                    global_limit: _,
                    per_host_limit,
                } if self
                    .hosts
                    .get(&queued.waiter.host)
                    .is_some_and(|host| host.active >= per_host_limit.get()) =>
                {
                    waiting_reason_counts.host_limited =
                        waiting_reason_counts.host_limited.saturating_add(1);
                }
                TransferAdmissionPolicy::Finite { .. } => {
                    waiting_reason_counts.no_grantable_waiter =
                        waiting_reason_counts.no_grantable_waiter.saturating_add(1);
                }
            }
        }
        TransferAdmissionSnapshot {
            policy,
            active_global: self.active_global,
            active_by_host,
            queued_global: self.waiters.len(),
            queued_by_host,
            paused_waiters,
            grants: self.grants,
            releases: self.releases,
            cancellations_before_grant: self.cancellations_before_grant,
            prestart_releases: self.prestart_releases,
            waited_count: self.waited_count,
            total_wait_ms: self.total_wait_ms,
            max_wait_ms: self.max_wait_ms,
            max_active_global: self.max_active_global,
            live_max_active_by_host,
            waiting_reason_counts,
            failed_closed: self.failed_closed,
        }
    }
}

impl AdmissionWaiter {
    fn clone_for_transfer(&self) -> Self {
        Self {
            id: self.id,
            task_key: self.task_key,
            task_identity: self.task_identity.clone(),
            host: self.host,
            enqueued_at: self.enqueued_at,
        }
    }
}

#[doc(hidden)]
pub struct TransferAdmissionController {
    policy: TransferAdmissionPolicy,
    state: Mutex<AdmissionState>,
    changed: Condvar,
}

impl TransferAdmissionController {
    pub fn compatibility() -> Arc<Self> {
        Arc::new(Self::new(TransferAdmissionPolicy::Compatibility))
    }

    #[doc(hidden)]
    pub fn finite_for_test(global_limit: NonZeroUsize, per_host_limit: NonZeroUsize) -> Arc<Self> {
        Arc::new(Self::new(TransferAdmissionPolicy::Finite {
            global_limit,
            per_host_limit,
        }))
    }

    fn new(policy: TransferAdmissionPolicy) -> Self {
        Self {
            policy,
            state: Mutex::new(AdmissionState::default()),
            changed: Condvar::new(),
        }
    }

    pub const fn policy(&self) -> TransferAdmissionPolicy {
        self.policy
    }

    pub fn snapshot(&self) -> TransferAdmissionSnapshot {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.fail_closed();
                self.changed.notify_all();
                state
            }
        };
        state.snapshot(self.policy)
    }

    #[doc(hidden)]
    pub fn exhaust_ids_for_test(&self) {
        match self.state.lock() {
            Ok(mut state) => state.next_identity = u64::MAX,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.fail_closed();
                self.changed.notify_all();
            }
        }
    }

    fn lock_for_acquire(&self) -> Result<MutexGuard<'_, AdmissionState>, AdmissionAcquireError> {
        match self.state.lock() {
            Ok(state) => Ok(state),
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.fail_closed();
                self.changed.notify_all();
                Err(AdmissionAcquireError::FailedClosed)
            }
        }
    }

    fn acquire(
        self: &Arc<Self>,
        origin: &NormalizedOrigin,
        task_identity: &str,
        existing: Option<AdmissionWaiter>,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
    ) -> Result<AdmissionPermit, AdmissionAcquireError> {
        if existing.is_none() && cancelled() {
            return Err(AdmissionAcquireError::Cancelled);
        }
        let waiter = match existing {
            Some(waiter) => waiter,
            None => {
                let mut state = self.lock_for_acquire()?;
                let registered = state.register_waiter(origin, task_identity);
                let Ok(waiter) = registered else {
                    state.fail_closed();
                    self.changed.notify_all();
                    return Err(AdmissionAcquireError::FailedClosed);
                };
                self.changed.notify_all();
                waiter
            }
        };

        loop {
            let cancelled_now = cancelled();
            let paused_now = paused();
            let mut state = self.lock_for_acquire()?;
            if state.failed_closed {
                return Err(AdmissionAcquireError::FailedClosed);
            }
            if cancelled_now {
                if state.cancel_waiter(waiter.id).is_err() {
                    state.fail_closed();
                    self.changed.notify_all();
                    return Err(AdmissionAcquireError::FailedClosed);
                }
                self.changed.notify_all();
                return Err(AdmissionAcquireError::Cancelled);
            }
            match state.set_waiter_paused(waiter.id, paused_now) {
                Ok(true) => self.changed.notify_all(),
                Ok(false) => {}
                Err(()) => {
                    state.fail_closed();
                    self.changed.notify_all();
                    return Err(AdmissionAcquireError::FailedClosed);
                }
            }

            let selected = match state.select_grantable(self.policy) {
                Ok(selected) => selected,
                Err(()) => {
                    state.fail_closed();
                    self.changed.notify_all();
                    return Err(AdmissionAcquireError::FailedClosed);
                }
            };
            if selected == Some(waiter.id) {
                let (token, granted_waiter) = match state.grant(waiter.id) {
                    Ok(value) => value,
                    Err(()) => {
                        state.fail_closed();
                        self.changed.notify_all();
                        return Err(AdmissionAcquireError::FailedClosed);
                    }
                };
                self.changed.notify_all();
                return Ok(AdmissionPermit {
                    controller: Arc::clone(self),
                    token: Some(token),
                    waiter: Some(granted_waiter),
                });
            }

            let waited = self.changed.wait_timeout(state, WAIT_POLL_INTERVAL);
            match waited {
                Ok((_state, _)) => {}
                Err(poisoned) => {
                    let (mut state, _) = poisoned.into_inner();
                    state.fail_closed();
                    self.changed.notify_all();
                    return Err(AdmissionAcquireError::FailedClosed);
                }
            }
        }
    }
}

struct AdmissionPermit {
    controller: Arc<TransferAdmissionController>,
    token: Option<PermitToken>,
    waiter: Option<AdmissionWaiter>,
}

impl AdmissionPermit {
    fn release_before_delegate(mut self) -> Result<(), AdmissionAcquireError> {
        let token = self
            .token
            .take()
            .ok_or(AdmissionAcquireError::FailedClosed)?;
        let mut state = self.controller.lock_for_acquire()?;
        if state.failed_closed {
            return Err(AdmissionAcquireError::FailedClosed);
        }
        if state.release_before_delegate(token).is_err() {
            state.fail_closed();
            self.controller.changed.notify_all();
            return Err(AdmissionAcquireError::FailedClosed);
        }
        self.controller.changed.notify_all();
        Ok(())
    }

    fn requeue_for_prestart_pause(mut self) -> Result<AdmissionWaiter, AdmissionAcquireError> {
        let token = self
            .token
            .take()
            .ok_or(AdmissionAcquireError::FailedClosed)?;
        let waiter = self
            .waiter
            .take()
            .ok_or(AdmissionAcquireError::FailedClosed)?;
        let mut state = self.controller.lock_for_acquire()?;
        if state.failed_closed {
            return Err(AdmissionAcquireError::FailedClosed);
        }
        let requeued = match state.requeue_after_prestart_pause(token, waiter) {
            Ok(waiter) => waiter,
            Err(()) => {
                state.fail_closed();
                self.controller.changed.notify_all();
                return Err(AdmissionAcquireError::FailedClosed);
            }
        };
        self.controller.changed.notify_all();
        Ok(requeued)
    }
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        match self.controller.state.lock() {
            Ok(mut state) => {
                if state.failed_closed {
                    return;
                }
                if state.release_token(token).is_err() {
                    state.fail_closed();
                }
                self.controller.changed.notify_all();
            }
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.fail_closed();
                self.controller.changed.notify_all();
            }
        }
    }
}

#[doc(hidden)]
pub struct AdmittedTransferBackend<'a> {
    inner: &'a dyn TransferBackend,
    controller: Arc<TransferAdmissionController>,
}

impl<'a> AdmittedTransferBackend<'a> {
    pub fn new(
        inner: &'a dyn TransferBackend,
        controller: Arc<TransferAdmissionController>,
    ) -> Self {
        Self { inner, controller }
    }

    fn acquire_for_delegate(
        &self,
        request: &TransferRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
    ) -> Result<AdmissionPermit, AdmissionAcquireError> {
        let origin = request
            .admission_origin()
            .ok_or(AdmissionAcquireError::FailedClosed)?;
        let mut requeued = None;
        loop {
            let permit = self.controller.acquire(
                &origin,
                &request.task_id,
                requeued.take(),
                cancelled,
                paused,
            )?;
            if cancelled() {
                permit.release_before_delegate()?;
                return Err(AdmissionAcquireError::Cancelled);
            }
            if paused() {
                requeued = Some(permit.requeue_for_prestart_pause()?);
                continue;
            }
            return Ok(permit);
        }
    }
}

impl TransferBackend for AdmittedTransferBackend<'_> {
    fn kind(&self) -> TransferBackendKind {
        self.inner.kind()
    }

    fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    fn download_legacy(
        &self,
        request: &TransferRequest,
        auth: &TransferAuth,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        let _permit = match self.acquire_for_delegate(request, cancelled, paused) {
            Ok(permit) => permit,
            Err(AdmissionAcquireError::Cancelled) => return interrupted_before_start(),
            Err(AdmissionAcquireError::FailedClosed) => return admission_failure(),
        };
        self.inner
            .download_legacy(request, auth, cancelled, paused, progress)
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        self.inner.resolved_session()
    }

    fn download_resolved(
        &self,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        policy: AuthRefreshPolicyId,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        let _permit = match self.acquire_for_delegate(request, cancelled, paused) {
            Ok(permit) => permit,
            Err(AdmissionAcquireError::Cancelled) => {
                return ResolvedTransferResult::Finished(interrupted_before_start())
            }
            Err(AdmissionAcquireError::FailedClosed) => {
                return ResolvedTransferResult::Finished(admission_failure())
            }
        };
        self.inner
            .download_resolved(request, asset, policy, session, cancelled, paused, progress)
    }

    #[allow(private_interfaces)]
    fn download_resolved_validated(
        &self,
        dispatch: &ValidatedTransferDispatch,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        policy: AuthRefreshPolicyId,
        plan: &ArtifactValidationPlan,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ValidatedResolvedTransferResult {
        let _permit = match self.acquire_for_delegate(request, cancelled, paused) {
            Ok(permit) => permit,
            Err(AdmissionAcquireError::Cancelled) => {
                return ValidatedResolvedTransferResult::seal(
                    ResolvedTransferResult::Finished(interrupted_before_start()),
                    request,
                    plan,
                    session,
                    dispatch,
                )
            }
            Err(AdmissionAcquireError::FailedClosed) => {
                return ValidatedResolvedTransferResult::seal(
                    ResolvedTransferResult::Finished(admission_failure()),
                    request,
                    plan,
                    session,
                    dispatch,
                )
            }
        };
        self.inner.download_resolved_validated(
            dispatch, request, asset, policy, plan, session, cancelled, paused, progress,
        )
    }
}

fn interrupted_before_start() -> TransferResult {
    TransferResult {
        outcome: DownloadOutcome::Interrupted,
        bytes_written: 0,
        attempt_count: 0,
        failure: Some(DownloadFailure {
            code: "DL001".to_string(),
            reason_zh: "下载在传输开始前已中断，未发起文件请求。".to_string(),
            retryable: true,
        }),
    }
}

fn admission_failure() -> TransferResult {
    TransferResult {
        outcome: DownloadOutcome::Failed,
        bytes_written: 0,
        attempt_count: 0,
        failure: Some(DownloadFailure {
            code: "DL005".to_string(),
            reason_zh: "下载准入控制器不可用，未开始传输。".to_string(),
            retryable: true,
        }),
    }
}
