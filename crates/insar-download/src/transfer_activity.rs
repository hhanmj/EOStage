use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RECENT_STALL_WINDOW: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskTransferSample {
    /// Actual HTTP body bytes observed in this worker lifetime, across tiles
    /// and attempts. Not the size of a cropped or converted output artifact.
    pub network_bytes_received: u64,
    pub active_millis: u64,
    pub active_count: u32,
    pub bytes_per_second: f64,
}

impl Default for TaskTransferSample {
    fn default() -> Self {
        Self {
            network_bytes_received: 0,
            active_millis: 0,
            active_count: 0,
            bytes_per_second: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferActivityError {
    EmptyIdentifier,
    LockPoisoned,
    NonMonotonicTime,
    RegressingBytes,
    DurationOverflow,
    ActiveCountOverflow,
    ByteCountOverflow,
}

impl fmt::Display for TransferActivityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyIdentifier => "transfer activity identifiers must not be empty",
            Self::LockPoisoned => "transfer activity state is unavailable",
            Self::NonMonotonicTime => "transfer activity time moved backwards",
            Self::RegressingBytes => "transfer activity bytes moved backwards",
            Self::ByteCountOverflow => "transfer byte count exceeds the supported range",
            Self::DurationOverflow => "transfer activity duration exceeds the supported range",
            Self::ActiveCountOverflow => {
                "transfer activity member count exceeds the supported range"
            }
        })
    }
}

impl std::error::Error for TransferActivityError {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TransferMember {
    item_id: String,
    attempt: u32,
}

#[derive(Clone, Copy, Debug)]
struct MemberSample {
    bytes_completed: u64,
    observed_at: Instant,
    rate_window_started_at: Instant,
    rate_window_started_bytes: u64,
    bytes_per_second: f64,
}

#[derive(Debug, Default)]
struct TaskActivityState {
    network_bytes_received: u64,
    active: HashSet<TransferMember>,
    members: HashMap<TransferMember, MemberSample>,
    interval_started_at: Option<Instant>,
    accumulated: Duration,
    last_observed_at: Option<Instant>,
    recent_stalls: VecDeque<Instant>,
}

#[derive(Debug, Default)]
pub struct TaskTransferActivityRegistry {
    tasks: Mutex<HashMap<String, TaskActivityState>>,
}

/// Internal binding for sequential response bodies belonging to one task item.
/// Each body starts at zero: checkpoint reuse and previous attempts are not reads.
#[derive(Clone)]
pub(crate) struct TaskBodyActivity {
    registry: Arc<TaskTransferActivityRegistry>,
    task_id: String,
    item_id: String,
}

impl TaskBodyActivity {
    pub(crate) fn new(
        registry: Arc<TaskTransferActivityRegistry>,
        task_id: String,
        item_id: String,
    ) -> Self {
        Self {
            registry,
            task_id,
            item_id,
        }
    }

    pub(crate) fn start(&self) -> TaskBodyActivityGuard<'_> {
        let _ = self
            .registry
            .body_started(&self.task_id, &self.item_id, 0, Instant::now());
        TaskBodyActivityGuard {
            activity: self,
            received: 0,
        }
    }
}

pub(crate) struct TaskBodyActivityGuard<'a> {
    activity: &'a TaskBodyActivity,
    received: u64,
}

impl TaskBodyActivityGuard<'_> {
    pub(crate) fn observe(&mut self, count: usize) {
        self.received = self.received.saturating_add(count as u64);
        let _ = self.activity.registry.observe_bytes(
            &self.activity.task_id,
            &self.activity.item_id,
            0,
            self.received,
            Instant::now(),
        );
    }
}

impl Drop for TaskBodyActivityGuard<'_> {
    fn drop(&mut self) {
        let _ = self.activity.registry.body_stopped(
            &self.activity.task_id,
            &self.activity.item_id,
            0,
            Instant::now(),
        );
    }
}

impl TaskTransferActivityRegistry {
    pub fn body_started(
        &self,
        task_id: &str,
        item_id: &str,
        attempt: u32,
        now: Instant,
    ) -> Result<TaskTransferSample, TransferActivityError> {
        self.body_started_at_bytes(task_id, item_id, attempt, 0, now)
    }

    pub fn body_started_at_bytes(
        &self,
        task_id: &str,
        item_id: &str,
        attempt: u32,
        bytes_completed: u64,
        now: Instant,
    ) -> Result<TaskTransferSample, TransferActivityError> {
        validate_identifiers(task_id, item_id)?;
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| TransferActivityError::LockPoisoned)?;
        let state = tasks.entry(task_id.to_owned()).or_default();
        let now = observe_task_time(state, now);
        let member = TransferMember {
            item_id: item_id.to_owned(),
            attempt,
        };
        if !state.active.insert(member.clone()) {
            return task_sample(state, now);
        }
        if state.active.len() == 1 {
            state.interval_started_at = Some(now);
        }
        state.members.insert(
            member,
            MemberSample {
                bytes_completed,
                observed_at: now,
                rate_window_started_at: now,
                rate_window_started_bytes: bytes_completed,
                bytes_per_second: 0.0,
            },
        );
        task_sample(state, now)
    }

    pub fn observe_bytes(
        &self,
        task_id: &str,
        item_id: &str,
        attempt: u32,
        bytes_completed: u64,
        now: Instant,
    ) -> Result<TaskTransferSample, TransferActivityError> {
        validate_identifiers(task_id, item_id)?;
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| TransferActivityError::LockPoisoned)?;
        let state = tasks
            .get_mut(task_id)
            .ok_or(TransferActivityError::NonMonotonicTime)?;
        let member = TransferMember {
            item_id: item_id.to_owned(),
            attempt,
        };
        if !state.active.contains(&member) {
            return Err(TransferActivityError::NonMonotonicTime);
        }
        let prior = state
            .members
            .get(&member)
            .copied()
            .ok_or(TransferActivityError::NonMonotonicTime)?;
        if now < prior.observed_at {
            return Err(TransferActivityError::NonMonotonicTime);
        }
        if bytes_completed < prior.bytes_completed {
            return Err(TransferActivityError::RegressingBytes);
        }
        let elapsed = now
            .checked_duration_since(prior.rate_window_started_at)
            .ok_or(TransferActivityError::NonMonotonicTime)?;
        state.network_bytes_received = state
            .network_bytes_received
            .checked_add(bytes_completed - prior.bytes_completed)
            .ok_or(TransferActivityError::ByteCountOverflow)?;
        let (rate, rate_window_started_at, rate_window_started_bytes) =
            if elapsed < Duration::from_secs(1) {
                (
                    prior.bytes_per_second,
                    prior.rate_window_started_at,
                    prior.rate_window_started_bytes,
                )
            } else {
                let byte_delta = bytes_completed
                    .checked_sub(prior.rate_window_started_bytes)
                    .ok_or(TransferActivityError::RegressingBytes)?;
                (
                    finite_rate(byte_delta as f64 / elapsed.as_secs_f64()),
                    now,
                    bytes_completed,
                )
            };
        state.members.insert(
            member,
            MemberSample {
                bytes_completed,
                observed_at: now,
                rate_window_started_at,
                rate_window_started_bytes,
                bytes_per_second: rate,
            },
        );
        let task_now = observe_task_time(state, now);
        task_sample(state, task_now)
    }

    pub fn body_stopped(
        &self,
        task_id: &str,
        item_id: &str,
        attempt: u32,
        now: Instant,
    ) -> Result<TaskTransferSample, TransferActivityError> {
        validate_identifiers(task_id, item_id)?;
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| TransferActivityError::LockPoisoned)?;
        let state = tasks.entry(task_id.to_owned()).or_default();
        let now = observe_task_time(state, now);
        let member = TransferMember {
            item_id: item_id.to_owned(),
            attempt,
        };
        if !state.active.remove(&member) {
            return task_sample(state, now);
        }
        state.members.remove(&member);
        if state.active.is_empty() {
            close_interval(state, now)?;
        }
        task_sample(state, now)
    }

    pub fn sample(
        &self,
        task_id: &str,
        now: Instant,
    ) -> Result<TaskTransferSample, TransferActivityError> {
        Ok(self.sample_if_known(task_id, now)?.unwrap_or_default())
    }

    pub fn sample_if_known(
        &self,
        task_id: &str,
        now: Instant,
    ) -> Result<Option<TaskTransferSample>, TransferActivityError> {
        if task_id.trim().is_empty() {
            return Err(TransferActivityError::EmptyIdentifier);
        }
        let tasks = self
            .tasks
            .lock()
            .map_err(|_| TransferActivityError::LockPoisoned)?;
        let Some(state) = tasks.get(task_id) else {
            return Ok(None);
        };
        validate_monotonic(state, now)?;
        task_sample(state, now).map(Some)
    }

    pub fn member_bytes_per_second(
        &self,
        task_id: &str,
        item_id: &str,
        attempt: u32,
    ) -> Result<f64, TransferActivityError> {
        validate_identifiers(task_id, item_id)?;
        let tasks = self
            .tasks
            .lock()
            .map_err(|_| TransferActivityError::LockPoisoned)?;
        let member = TransferMember {
            item_id: item_id.to_owned(),
            attempt,
        };
        tasks
            .get(task_id)
            .and_then(|state| state.members.get(&member))
            .map(|sample| finite_rate(sample.bytes_per_second))
            .ok_or(TransferActivityError::NonMonotonicTime)
    }

    pub fn record_stall_reconnect(
        &self,
        task_id: &str,
        now: Instant,
    ) -> Result<(), TransferActivityError> {
        if task_id.trim().is_empty() {
            return Err(TransferActivityError::EmptyIdentifier);
        }
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| TransferActivityError::LockPoisoned)?;
        let state = tasks.entry(task_id.to_owned()).or_default();
        prune_recent_stalls(state, now);
        state.recent_stalls.push_back(now);
        Ok(())
    }

    pub fn recent_stall_count(
        &self,
        task_id: &str,
        now: Instant,
    ) -> Result<u32, TransferActivityError> {
        if task_id.trim().is_empty() {
            return Err(TransferActivityError::EmptyIdentifier);
        }
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| TransferActivityError::LockPoisoned)?;
        let Some(state) = tasks.get_mut(task_id) else {
            return Ok(0);
        };
        prune_recent_stalls(state, now);
        Ok(u32::try_from(state.recent_stalls.len()).unwrap_or(u32::MAX))
    }

    pub fn close_task(
        &self,
        task_id: &str,
        now: Instant,
    ) -> Result<TaskTransferSample, TransferActivityError> {
        if task_id.trim().is_empty() {
            return Err(TransferActivityError::EmptyIdentifier);
        }
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| TransferActivityError::LockPoisoned)?;
        let Some(state) = tasks.get_mut(task_id) else {
            return Ok(TaskTransferSample::default());
        };
        let now = observe_task_time(state, now);
        state.recent_stalls.clear();
        if !state.active.is_empty() {
            close_interval(state, now)?;
            state.active.clear();
            state.members.clear();
        }
        task_sample(state, now)
    }
}

fn prune_recent_stalls(state: &mut TaskActivityState, now: Instant) {
    while let Some(&front) = state.recent_stalls.front() {
        if now.saturating_duration_since(front) > RECENT_STALL_WINDOW {
            state.recent_stalls.pop_front();
        } else {
            break;
        }
    }
}

fn validate_identifiers(task_id: &str, item_id: &str) -> Result<(), TransferActivityError> {
    if task_id.trim().is_empty() || item_id.trim().is_empty() {
        Err(TransferActivityError::EmptyIdentifier)
    } else {
        Ok(())
    }
}

fn validate_monotonic(
    state: &TaskActivityState,
    now: Instant,
) -> Result<(), TransferActivityError> {
    if state.last_observed_at.is_some_and(|prior| now < prior) {
        Err(TransferActivityError::NonMonotonicTime)
    } else {
        Ok(())
    }
}

fn observe_task_time(state: &mut TaskActivityState, now: Instant) -> Instant {
    let effective = state
        .last_observed_at
        .map_or(now, |prior| std::cmp::max(prior, now));
    state.last_observed_at = Some(effective);
    effective
}

fn close_interval(
    state: &mut TaskActivityState,
    now: Instant,
) -> Result<(), TransferActivityError> {
    let Some(started_at) = state.interval_started_at.take() else {
        return Ok(());
    };
    let elapsed = now
        .checked_duration_since(started_at)
        .ok_or(TransferActivityError::NonMonotonicTime)?;
    state.accumulated = state
        .accumulated
        .checked_add(elapsed)
        .ok_or(TransferActivityError::DurationOverflow)?;
    Ok(())
}

fn task_sample(
    state: &TaskActivityState,
    now: Instant,
) -> Result<TaskTransferSample, TransferActivityError> {
    let elapsed = match state.interval_started_at {
        Some(started_at) => state
            .accumulated
            .checked_add(
                now.checked_duration_since(started_at)
                    .ok_or(TransferActivityError::NonMonotonicTime)?,
            )
            .ok_or(TransferActivityError::DurationOverflow)?,
        None => state.accumulated,
    };
    let active_millis =
        u64::try_from(elapsed.as_millis()).map_err(|_| TransferActivityError::DurationOverflow)?;
    let active_count = u32::try_from(state.active.len())
        .map_err(|_| TransferActivityError::ActiveCountOverflow)?;
    let bytes_per_second = if active_count == 0 {
        0.0
    } else {
        finite_rate(
            state
                .members
                .values()
                .map(|member| member.bytes_per_second)
                .sum(),
        )
    };
    Ok(TaskTransferSample {
        network_bytes_received: state.network_bytes_received,
        active_millis,
        active_count,
        bytes_per_second,
    })
}

fn finite_rate(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_stalls_track_within_window_and_prune_expired() {
        let registry = TaskTransferActivityRegistry::default();
        let t0 = Instant::now();
        registry.record_stall_reconnect("task-1", t0).unwrap();
        registry
            .record_stall_reconnect("task-1", t0 + Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            registry
                .recent_stall_count("task-1", t0 + Duration::from_secs(20))
                .unwrap(),
            2
        );
        // Past 120s window: only the second stall stays recent.
        assert_eq!(
            registry
                .recent_stall_count("task-1", t0 + Duration::from_secs(125))
                .unwrap(),
            1
        );
        // Past both windows: resets to 0.
        assert_eq!(
            registry
                .recent_stall_count("task-1", t0 + Duration::from_secs(140))
                .unwrap(),
            0
        );
    }
}
