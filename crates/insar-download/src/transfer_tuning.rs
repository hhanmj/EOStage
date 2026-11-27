//! Adaptive transfer tuning primitives: shared bandwidth limiter, per-host concurrency tracker,
//! dynamic work-stealing segment pool, and relative slow-connection shedding.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::Url;

use crate::file_core::{SegmentProgressEntry, SegmentedProgressMeta};

const MAX_DYNAMIC_SEGMENTS: usize = 128;
const MAX_TRACKED_HOSTS: usize = 512;
const SPLIT_BLOCK_ALIGN_BYTES: u64 = 4096;

/// Default minimum remainder chunk size (`512 KiB`) allowed on each side of a work-stealing split.
pub const DEFAULT_MIN_SPLIT_CHUNK_BYTES: u64 = 512 * 1024;
/// Default byte overlap re-read on resumed Range transfers to verify remote identity continuity.
pub const DEFAULT_RESUME_OVERLAP_BYTES: u64 = 64;

fn max_burst_bytes(rate_bps: u64) -> f64 {
    if rate_bps == 0 {
        return 0.0;
    }
    let quarter_sec = (rate_bps as f64 * 0.25).clamp(16_384.0, 2.0 * 1024.0 * 1024.0);
    quarter_sec.min((rate_bps.max(4096)) as f64)
}

fn initial_burst_bytes(rate_bps: u64) -> f64 {
    if rate_bps == 0 {
        return 0.0;
    }
    let max_b = max_burst_bytes(rate_bps);
    (rate_bps as f64 * 0.10).clamp(4096.0, 65_536.0).min(max_b)
}

#[derive(Debug)]
struct BucketState {
    tokens: f64,
    last_refill: Instant,
    initialized_nonzero: bool,
}

impl Default for BucketState {
    fn default() -> Self {
        Self {
            tokens: 0.0,
            last_refill: Instant::now(),
            initialized_nonzero: false,
        }
    }
}

/// RAII guard tracking an active unpaused download stream registered with a [`BandwidthLimiter`].
pub struct ActiveStreamGuard {
    limiter: Arc<BandwidthLimiter>,
}

impl Drop for ActiveStreamGuard {
    fn drop(&mut self) {
        self.limiter.active_streams.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Result of acquiring byte budget from a [`BandwidthLimiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquireOutcome {
    Granted(usize),
    Paused,
    Cancelled,
}

/// Live-adjustable, cancellation-aware token-bucket bandwidth limiter shared across downloads.
#[derive(Debug)]
pub struct BandwidthLimiter {
    limit_bps: AtomicU64,
    limit_generation: AtomicU64,
    active_streams: AtomicUsize,
    state: Mutex<BucketState>,
}

impl BandwidthLimiter {
    /// Creates a new limiter (`0` = unlimited).
    pub fn new(initial_limit_bps: u64) -> Self {
        let initialized_nonzero = initial_limit_bps > 0;
        let tokens = if initialized_nonzero {
            initial_burst_bytes(initial_limit_bps)
        } else {
            0.0
        };
        Self {
            limit_bps: AtomicU64::new(initial_limit_bps),
            limit_generation: AtomicU64::new(0),
            active_streams: AtomicUsize::new(0),
            state: Mutex::new(BucketState {
                tokens,
                last_refill: Instant::now(),
                initialized_nonzero,
            }),
        }
    }

    /// Current global byte-per-second ceiling (`0` = unlimited).
    pub fn limit_bps(&self) -> u64 {
        self.limit_bps.load(Ordering::Acquire)
    }

    /// Monotonic generation bumped when toggling between unlimited (`0`) and rate-limited (`> 0`).
    pub fn limit_generation(&self) -> u64 {
        self.limit_generation.load(Ordering::Acquire)
    }

    /// Number of currently unpaused active stream loops sharing this limiter.
    pub fn active_streams(&self) -> usize {
        self.active_streams.load(Ordering::Acquire)
    }

    /// Registers an active stream loop; decremented automatically when the guard drops.
    pub fn begin_stream(self: &Arc<Self>) -> ActiveStreamGuard {
        let prev_active = self.active_streams.fetch_add(1, Ordering::AcqRel);
        if prev_active == 0 {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let limit_now = self.limit_bps.load(Ordering::Acquire);
            if limit_now > 0 {
                let init_burst = initial_burst_bytes(limit_now);
                if !guard.initialized_nonzero {
                    guard.tokens = init_burst;
                    guard.initialized_nonzero = true;
                } else {
                    // Discard idle pre-transfer refill accumulation without granting
                    // extra burst on rapid reconnects.
                    guard.tokens = guard.tokens.min(init_burst);
                }
                guard.last_refill = Instant::now();
            }
        }
        ActiveStreamGuard {
            limiter: Arc::clone(self),
        }
    }

    /// Updates the speed ceiling live (`0` = unlimited), clamping buffered tokens if lowered.
    pub fn set_limit_bps(&self, new_bps: u64) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = self.limit_bps.swap(new_bps, Ordering::AcqRel);
        if (prev == 0) != (new_bps == 0) {
            self.limit_generation.fetch_add(1, Ordering::AcqRel);
        }
        let now = Instant::now();
        if new_bps == 0 {
            guard.tokens = 0.0;
            guard.initialized_nonzero = false;
            guard.last_refill = now;
        } else {
            let max_b = max_burst_bytes(new_bps);
            if !guard.initialized_nonzero {
                guard.tokens = initial_burst_bytes(new_bps);
                guard.initialized_nonzero = true;
            } else {
                let elapsed = now
                    .saturating_duration_since(guard.last_refill)
                    .as_secs_f64();
                guard.tokens = (guard.tokens + elapsed * prev as f64).min(max_b);
            }
            guard.last_refill = now;
        }
    }

    /// Acquires up to `max_bytes` read budget, waiting in short interruptible slices if needed.
    /// Returns [`AcquireOutcome::Paused`] immediately on pause and [`AcquireOutcome::Cancelled`] on
    /// cancel/abort so callers run their outer pause/checkpoint/watchdog-reset logic.
    pub fn acquire(
        &self,
        max_bytes: usize,
        cancelled: &dyn Fn() -> bool,
        paused: &dyn Fn() -> bool,
        worker_abort: Option<&AtomicBool>,
    ) -> AcquireOutcome {
        if max_bytes == 0 {
            return AcquireOutcome::Granted(0);
        }
        if cancelled() || worker_abort.is_some_and(|a| a.load(Ordering::Acquire)) {
            return AcquireOutcome::Cancelled;
        }
        if paused() {
            return AcquireOutcome::Paused;
        }
        if self.limit_bps.load(Ordering::Acquire) == 0 {
            return AcquireOutcome::Granted(max_bytes);
        }
        loop {
            if cancelled() || worker_abort.is_some_and(|a| a.load(Ordering::Acquire)) {
                return AcquireOutcome::Cancelled;
            }
            if paused() {
                return AcquireOutcome::Paused;
            }
            let limit = self.limit_bps.load(Ordering::Acquire);
            if limit == 0 {
                return AcquireOutcome::Granted(max_bytes);
            }
            let max_b = max_burst_bytes(limit);
            let pacing_slice = ((limit / 8).clamp(4096, 262_144)) as usize;
            let target = max_bytes.min(pacing_slice).min(max_b as usize).max(1);

            let wait = {
                let mut guard = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let limit_now = self.limit_bps.load(Ordering::Acquire);
                if limit_now == 0 {
                    return AcquireOutcome::Granted(max_bytes);
                }
                let max_b_now = max_burst_bytes(limit_now);
                let target_now = target.min(max_b_now as usize).max(1);
                let now = Instant::now();
                let elapsed = now
                    .saturating_duration_since(guard.last_refill)
                    .as_secs_f64();
                guard.last_refill = now;
                if !guard.initialized_nonzero {
                    guard.tokens = initial_burst_bytes(limit_now);
                    guard.initialized_nonzero = true;
                } else {
                    guard.tokens = (guard.tokens + elapsed * limit_now as f64).min(max_b_now);
                }
                if guard.tokens >= target_now as f64 {
                    guard.tokens -= target_now as f64;
                    return AcquireOutcome::Granted(target_now);
                }
                let deficit = target_now as f64 - guard.tokens;
                Duration::from_secs_f64((deficit / limit_now as f64).max(0.001))
                    .min(Duration::from_millis(25))
            };

            thread::sleep(wait);
        }
    }

    /// Returns unconsumed bytes from an over-requested socket read grant back to the token bucket.
    pub fn refund(&self, unused_bytes: usize) {
        if unused_bytes == 0 {
            return;
        }
        let limit = self.limit_bps.load(Ordering::Acquire);
        if limit == 0 {
            return;
        }
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.limit_bps.load(Ordering::Acquire) == 0 || !guard.initialized_nonzero {
            return;
        }
        guard.tokens = (guard.tokens + unused_bytes as f64).min(max_burst_bytes(limit));
    }

    /// Adjusts `StallWatchdog` threshold below per-stream quota when rate-limiting is active so
    /// user throttling never causes false stall reconnects, while keeping dead `0 B/s` detection.
    pub fn effective_stall_min_bps(&self, configured_min_bps: u64) -> u64 {
        if configured_min_bps == 0 {
            return 0;
        }
        let limit = self.limit_bps.load(Ordering::Acquire);
        if limit == 0 {
            return configured_min_bps;
        }
        let streams = self.active_streams().max(1) as u64;
        let per_stream_share = (limit / streams).max(1);
        configured_min_bps.min((per_stream_share / 4).max(1))
    }
}

static GLOBAL_BANDWIDTH_LIMITER: OnceLock<Arc<BandwidthLimiter>> = OnceLock::new();

/// Returns the process-wide shared [`BandwidthLimiter`] controlled by `TransferEnginePrefs`.
pub fn global_bandwidth_limiter() -> &'static Arc<BandwidthLimiter> {
    GLOBAL_BANDWIDTH_LIMITER.get_or_init(|| Arc::new(BandwidthLimiter::new(0)))
}

/// Canonical per-host concurrency tracking key. Loopback addresses retain `:port` so isolated
/// loopback tests on different ephemeral ports never share learned ceilings.
pub fn canonical_host_key(url: &Url) -> Option<String> {
    let host = url.host_str()?.trim().to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    if matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]") {
        let port = url.port_or_known_default().unwrap_or(0);
        Some(format!("{host}:{port}"))
    } else {
        Some(host)
    }
}

fn step_down_segment_cap(current: u8) -> u8 {
    match current {
        0..=2 => 1,
        3..=4 => 2,
        _ => 4,
    }
}

fn step_up_segment_cap(current: u8) -> Option<u8> {
    match current {
        0..=1 => Some(2),
        2..=3 => Some(4),
        4..=7 => Some(8),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct HostCapState {
    cap: u8,
    last_throttled: Instant,
    clean_completions: u32,
}

/// Runtime per-host learned concurrency tracker that steps down segment counts when an upstream
/// returns HTTP 429 / concurrency throttling and recovers smoothly after sustained clean transfers.
#[derive(Debug)]
pub struct HostConcurrencyTracker {
    step_down_debounce: Duration,
    recovery_interval: Duration,
    full_expiry: Duration,
    entries: Mutex<HashMap<String, HostCapState>>,
}

impl Default for HostConcurrencyTracker {
    fn default() -> Self {
        Self::with_timings(
            Duration::from_secs(3),
            Duration::from_secs(300),
            Duration::from_secs(1800),
        )
    }
}

impl HostConcurrencyTracker {
    /// Creates a host concurrency tracker with custom debounce/recovery timings.
    pub fn with_timings(
        step_down_debounce: Duration,
        recovery_interval: Duration,
        full_expiry: Duration,
    ) -> Self {
        Self {
            step_down_debounce,
            recovery_interval,
            full_expiry,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the learned maximum segment cap for `url`'s host, if an active limit exists.
    pub fn effective_cap(&self, url: &Url) -> Option<u8> {
        let key = canonical_host_key(url)?;
        let now = Instant::now();
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(state) = guard.get(&key) {
            if now.saturating_duration_since(state.last_throttled) >= self.full_expiry {
                guard.remove(&key);
                None
            } else {
                Some(state.cap)
            }
        } else {
            None
        }
    }

    /// Records a rate-limit signal (HTTP 429 or segmented 503 concurrency refusal) for `url`'s host,
    /// stepping down its allowed segment ceiling with burst debouncing.
    pub fn record_rate_limit(&self, url: &Url, attempted_segments: u8) -> Option<u8> {
        let key = canonical_host_key(url)?;
        let now = Instant::now();
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.retain(|_, v| now.saturating_duration_since(v.last_throttled) < self.full_expiry);

        if let Some(existing) = guard.get_mut(&key) {
            if now.saturating_duration_since(existing.last_throttled) < self.step_down_debounce {
                existing.clean_completions = 0;
                return Some(existing.cap);
            }
            let next_cap = step_down_segment_cap(existing.cap.min(attempted_segments.max(1)));
            if existing.cap > next_cap {
                existing.cap = next_cap;
            }
            existing.last_throttled = now;
            existing.clean_completions = 0;
            return Some(existing.cap);
        }

        if guard.len() >= MAX_TRACKED_HOSTS {
            if let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, v)| v.last_throttled)
                .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest_key);
            }
        }
        let next_cap = step_down_segment_cap(attempted_segments.max(1));
        guard.insert(
            key,
            HostCapState {
                cap: next_cap,
                last_throttled: now,
                clean_completions: 0,
            },
        );
        Some(next_cap)
    }

    /// Records a clean transfer completion, gradually recovering stepped-down hosts.
    pub fn record_clean_completion(&self, url: &Url) {
        let Some(key) = canonical_host_key(url) else {
            return;
        };
        let now = Instant::now();
        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(existing) = guard.get_mut(&key) else {
            return;
        };
        let elapsed = now.saturating_duration_since(existing.last_throttled);
        if elapsed >= self.full_expiry {
            guard.remove(&key);
            return;
        }
        existing.clean_completions = existing.clean_completions.saturating_add(1);
        let ready = (elapsed >= self.recovery_interval && existing.clean_completions >= 2)
            || elapsed >= self.recovery_interval.saturating_mul(2);
        if ready {
            match step_up_segment_cap(existing.cap) {
                Some(next) if next < 8 => {
                    existing.cap = next;
                    existing.last_throttled = now;
                    existing.clean_completions = 0;
                }
                _ => {
                    guard.remove(&key);
                }
            }
        }
    }
}

static GLOBAL_HOST_CONCURRENCY_TRACKER: OnceLock<Arc<HostConcurrencyTracker>> = OnceLock::new();

/// Returns the process-wide shared [`HostConcurrencyTracker`].
pub fn global_host_concurrency_tracker() -> &'static Arc<HostConcurrencyTracker> {
    GLOBAL_HOST_CONCURRENCY_TRACKER.get_or_init(|| Arc::new(HostConcurrencyTracker::default()))
}

#[derive(Debug, Clone)]
struct DynamicSegment {
    start: u64,
    end: u64,
    claimed: u64,
    written: u64,
    assigned_worker: Option<usize>,
    shed_count: u32,
}

impl DynamicSegment {
    fn claimable_offset(&self) -> u64 {
        self.start.saturating_add(self.claimed).min(self.end)
    }

    fn written_offset(&self) -> u64 {
        self.start.saturating_add(self.written).min(self.end)
    }

    fn remaining_unclaimed(&self) -> u64 {
        self.end.saturating_sub(self.claimable_offset())
    }

    fn is_written_complete(&self) -> bool {
        self.written_offset() >= self.end
    }
}

/// Snapshot returned when a worker acquires or dynamically splits a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcquiredSegment {
    pub seg_id: usize,
    pub start: u64,
    pub offset: u64,
    pub end: u64,
    pub written: u64,
    pub shed_count: u32,
}

/// Reservation returned immediately before writing received network bytes at `write_offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedWrite {
    pub write_offset: u64,
    pub allowed: u64,
    pub hit_shrunk_end: bool,
}

#[derive(Debug)]
struct PoolInner {
    total_length: u64,
    min_split_chunk_bytes: u64,
    max_segments: usize,
    segments: Vec<DynamicSegment>,
}

/// Coordinated work-stealing segment pool backing concurrent HTTP Range transfers.
#[derive(Debug)]
pub struct DynamicSegmentPool {
    inner: Mutex<PoolInner>,
    split_count: AtomicU64,
}

impl DynamicSegmentPool {
    /// Initializes a segment pool from a validated [`SegmentProgressEntry`] partition.
    pub fn new(
        total_length: u64,
        initial_segments: Vec<SegmentProgressEntry>,
        min_split_chunk_bytes: u64,
        reserve_seg0_for_worker0: bool,
    ) -> Option<Self> {
        let meta = SegmentedProgressMeta {
            total_length,
            segments: initial_segments,
        };
        if !meta.is_valid_partition() {
            return None;
        }
        let mut sorted = meta.segments;
        sorted.sort_by_key(|seg| (seg.start, seg.end));
        let mut dyn_segs: Vec<DynamicSegment> = sorted
            .into_iter()
            .enumerate()
            .map(|(idx, seg)| {
                let is_complete = seg.start.saturating_add(seg.written) >= seg.end;
                let assigned_worker = if idx == 0 && reserve_seg0_for_worker0 && !is_complete {
                    Some(0)
                } else {
                    None
                };
                DynamicSegment {
                    start: seg.start,
                    end: seg.end,
                    claimed: seg.written,
                    written: seg.written,
                    assigned_worker,
                    shed_count: 0,
                }
            })
            .collect();
        if reserve_seg0_for_worker0 {
            // Reset claimed == written explicitly for clarity.
            if let Some(first) = dyn_segs.first_mut() {
                first.claimed = first.written;
            }
        }
        Some(Self {
            inner: Mutex::new(PoolInner {
                total_length,
                min_split_chunk_bytes: min_split_chunk_bytes.max(1),
                max_segments: MAX_DYNAMIC_SEGMENTS,
                segments: dyn_segs,
            }),
            split_count: AtomicU64::new(0),
        })
    }

    /// Number of dynamic work-stealing splits performed during this transfer.
    pub fn split_count(&self) -> u64 {
        self.split_count.load(Ordering::Relaxed)
    }

    /// Acquires an unassigned incomplete segment or bisects the largest remaining segment tail.
    pub fn acquire_or_split(&self, worker_id: usize) -> Option<AcquiredSegment> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // 1. Pre-assigned segment for this worker (e.g., Worker 0's open probe stream).
        if let Some((seg_id, seg)) =
            guard.segments.iter().enumerate().find(|(_, seg)| {
                seg.assigned_worker == Some(worker_id) && !seg.is_written_complete()
            })
        {
            return Some(AcquiredSegment {
                seg_id,
                start: seg.start,
                offset: seg.written_offset(),
                end: seg.end,
                written: seg.written,
                shed_count: seg.shed_count,
            });
        }

        // 2. Unassigned incomplete segment (lowest offset first so order stays predictable).
        if let Some((seg_id, _)) = guard
            .segments
            .iter()
            .enumerate()
            .filter(|(_, seg)| seg.assigned_worker.is_none() && !seg.is_written_complete())
            .min_by_key(|(_, seg)| seg.start)
        {
            let seg = &mut guard.segments[seg_id];
            seg.claimed = seg.written;
            seg.assigned_worker = Some(worker_id);
            return Some(AcquiredSegment {
                seg_id,
                start: seg.start,
                offset: seg.written_offset(),
                end: seg.end,
                written: seg.written,
                shed_count: seg.shed_count,
            });
        }

        // 3. Dynamic work-stealing bisection of the segment with the largest unclaimed tail.
        if guard.segments.len() >= guard.max_segments {
            return None;
        }
        let min_split = guard.min_split_chunk_bytes;
        let min_total = min_split.saturating_mul(2);
        let victim = guard
            .segments
            .iter()
            .enumerate()
            .filter(|(_, seg)| seg.remaining_unclaimed() >= min_total)
            .max_by_key(|(_, seg)| seg.remaining_unclaimed())
            .map(|(idx, _)| idx)?;

        let (mid, old_end) = {
            let v = &mut guard.segments[victim];
            let cursor = v.claimable_offset();
            let rem = v.end.saturating_sub(cursor);
            let raw_mid = cursor + rem / 2;
            let aligned = (raw_mid / SPLIT_BLOCK_ALIGN_BYTES) * SPLIT_BLOCK_ALIGN_BYTES;
            let mid = if aligned >= cursor.saturating_add(min_split)
                && v.end.saturating_sub(aligned) >= min_split
            {
                aligned
            } else {
                raw_mid
            };
            let old_end = v.end;
            v.end = mid;
            (mid, old_end)
        };

        let new_id = guard.segments.len();
        guard.segments.push(DynamicSegment {
            start: mid,
            end: old_end,
            claimed: 0,
            written: 0,
            assigned_worker: Some(worker_id),
            shed_count: 0,
        });
        self.split_count.fetch_add(1, Ordering::Relaxed);
        Some(AcquiredSegment {
            seg_id: new_id,
            start: mid,
            offset: mid,
            end: old_end,
            written: 0,
            shed_count: 0,
        })
    }

    /// Reads the live request window `[written_offset, end)` for `seg_id` and aligns `claimed` to `written`.
    /// Returns `None` if `seg_id` is already complete.
    pub fn prepare_segment_request(&self, seg_id: usize) -> Option<(u64, u64, u64, u32)> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let seg = guard.segments.get_mut(seg_id)?;
        seg.claimed = seg.written;
        if seg.is_written_complete() {
            return None;
        }
        Some((seg.written_offset(), seg.end, seg.written, seg.shed_count))
    }

    /// Atomically reserves up to `read_len` bytes before positional disk write so two workers can
    /// never overlap across a dynamic split boundary.
    pub fn claim_write(&self, seg_id: usize, read_len: u64) -> ClaimedWrite {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(seg) = guard.segments.get_mut(seg_id) else {
            return ClaimedWrite {
                write_offset: 0,
                allowed: 0,
                hit_shrunk_end: true,
            };
        };
        let write_offset = seg.claimable_offset();
        let avail = seg.end.saturating_sub(write_offset);
        let allowed = read_len.min(avail);
        seg.claimed = seg.claimed.saturating_add(allowed);
        let reached_end = seg.claimable_offset() >= seg.end;
        ClaimedWrite {
            write_offset,
            allowed,
            hit_shrunk_end: allowed < read_len || reached_end,
        }
    }

    /// Commits `written_len` previously claimed bytes as durably written.
    pub fn commit_written(&self, seg_id: usize, written_len: u64) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(seg) = guard.segments.get_mut(seg_id) {
            seg.written = seg.written.saturating_add(written_len).min(seg.claimed);
        }
    }

    /// Rolls back unclaimed/failed `claimed` bytes if `write_all_at` returns an I/O error.
    pub fn rollback_claimed(&self, seg_id: usize, failed_len: u64) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(seg) = guard.segments.get_mut(seg_id) {
            seg.claimed = seg.claimed.saturating_sub(failed_len).max(seg.written);
        }
    }

    /// Remaining unclaimed bytes currently left in `seg_id` (respecting dynamic shrinks).
    pub fn live_remaining(&self, seg_id: usize) -> u64 {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .segments
            .get(seg_id)
            .map_or(0, DynamicSegment::remaining_unclaimed)
    }

    /// Releases `seg_id` back to the pool (when complete or shed) and returns `(is_complete, shed_count)`.
    pub fn release_segment(
        &self,
        seg_id: usize,
        worker_id: usize,
        increment_shed: bool,
    ) -> (bool, u32) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(seg) = guard.segments.get_mut(seg_id) else {
            return (true, 0);
        };
        seg.claimed = seg.written;
        if seg.assigned_worker == Some(worker_id) {
            seg.assigned_worker = None;
        }
        if increment_shed {
            seg.shed_count = seg.shed_count.saturating_add(1);
        }
        (seg.is_written_complete(), seg.shed_count)
    }

    /// Returns a sorted [`SegmentedProgressMeta`] snapshot using strictly durable `written` counts.
    pub fn snapshot_meta(&self) -> SegmentedProgressMeta {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut entries: Vec<SegmentProgressEntry> = guard
            .segments
            .iter()
            .map(|seg| SegmentProgressEntry {
                start: seg.start,
                end: seg.end,
                written: seg.written,
            })
            .collect();
        entries.sort_by_key(|seg| (seg.start, seg.end));
        SegmentedProgressMeta {
            total_length: guard.total_length,
            segments: entries,
        }
    }

    /// Returns `true` strictly when all `[0, total_length)` segments form a valid partition and are
    /// 100% durably written.
    pub fn is_all_complete(&self) -> bool {
        let guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let meta = {
            let mut entries: Vec<SegmentProgressEntry> = guard
                .segments
                .iter()
                .map(|seg| SegmentProgressEntry {
                    start: seg.start,
                    end: seg.end,
                    written: seg.written,
                })
                .collect();
            entries.sort_by_key(|seg| (seg.start, seg.end));
            SegmentedProgressMeta {
                total_length: guard.total_length,
                segments: entries,
            }
        };
        meta.is_valid_partition()
            && guard
                .segments
                .iter()
                .all(|seg| seg.is_written_complete() && seg.claimed == seg.written)
    }
}

/// Configuration parameters for relative slow-connection shedding during segmented transfers.
#[derive(Debug, Clone)]
pub struct RelativeSpeedConfig {
    pub min_sample_bytes: u64,
    pub grace_duration: Duration,
    pub ratio_threshold: f64,
    pub min_remaining_secs: f64,
    pub max_sheds_per_transfer: u32,
    pub max_sheds_per_segment: u32,
}

impl Default for RelativeSpeedConfig {
    fn default() -> Self {
        Self {
            min_sample_bytes: 256 * 1024,
            grace_duration: Duration::from_secs(2),
            ratio_threshold: 4.0,
            min_remaining_secs: 2.0,
            max_sheds_per_transfer: 16,
            max_sheds_per_segment: 3,
        }
    }
}

#[derive(Debug)]
struct SpeedTrackerInner {
    peers: Vec<Option<(Instant, f64)>>,
    recent_finished: VecDeque<(Instant, f64)>,
    last_limit_generation: u64,
}

/// Tracks active and recently-finished single-connection rates across concurrent segment workers
/// so a severely straggling connection (> 4x slower with > 2s remaining) is shed and re-acquired.
#[derive(Debug)]
pub struct RelativeSpeedTracker {
    config: RelativeSpeedConfig,
    inner: Mutex<SpeedTrackerInner>,
    total_sheds: AtomicU32,
}

impl RelativeSpeedTracker {
    pub fn new(
        worker_count: usize,
        config: RelativeSpeedConfig,
        initial_limit_generation: u64,
    ) -> Self {
        Self {
            config,
            inner: Mutex::new(SpeedTrackerInner {
                peers: vec![None; worker_count.max(1)],
                recent_finished: VecDeque::new(),
                last_limit_generation: initial_limit_generation,
            }),
            total_sheds: AtomicU32::new(0),
        }
    }

    pub fn min_sample_bytes(&self) -> u64 {
        self.config.min_sample_bytes
    }

    pub fn min_sample_duration(&self) -> Duration {
        (self.config.grace_duration / 4)
            .clamp(Duration::from_millis(15), Duration::from_millis(100))
    }

    fn freshness_ttl(&self) -> Duration {
        self.config
            .grace_duration
            .saturating_mul(4)
            .max(Duration::from_secs(15))
    }

    /// Clears a worker's active rate sample (e.g., on pause or stream close).
    pub fn clear_worker(&self, worker_id: usize) {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(slot) = guard.peers.get_mut(worker_id) {
            *slot = None;
        }
    }

    /// Records a cleanly completed fast segment sample so stragglers retain a recent reference even
    /// after fast workers exit.
    pub fn record_completed_stream(
        &self,
        worker_id: usize,
        bytes_this_conn: u64,
        active_elapsed: Duration,
        speed_limit_bps: u64,
        limit_generation: u64,
    ) {
        if speed_limit_bps > 0 {
            self.clear_worker(worker_id);
            return;
        }
        let min_dur = self.min_sample_duration();
        let now = Instant::now();
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(slot) = guard.peers.get_mut(worker_id) {
            *slot = None;
        }
        if limit_generation != guard.last_limit_generation {
            guard.peers.fill(None);
            guard.recent_finished.clear();
            guard.last_limit_generation = limit_generation;
            return;
        }
        if bytes_this_conn >= self.config.min_sample_bytes {
            // Clamp sub-millisecond/fast buffered drains to `min_dur` so a fast completed
            // probe segment still records a conservative upper-bounded peer reference rate
            // instead of leaving straggler workers with `best_peer_bps == 0.0`.
            let effective_elapsed = active_elapsed.max(min_dur);
            let bps = bytes_this_conn as f64 / effective_elapsed.as_secs_f64();
            let ttl = self.freshness_ttl();
            while let Some((ts, _)) = guard.recent_finished.front() {
                if now.saturating_duration_since(*ts) > ttl {
                    guard.recent_finished.pop_front();
                } else {
                    break;
                }
            }
            if guard.recent_finished.len() >= 16 {
                guard.recent_finished.pop_front();
            }
            guard.recent_finished.push_back((now, bps));
        }
    }

    /// Evaluates whether `worker_id` is relatively much slower than active/recent peer connections
    /// and should release its remaining range onto a fresh connection.
    #[allow(clippy::too_many_arguments)]
    pub fn report_and_check_shed(
        &self,
        worker_id: usize,
        bytes_this_conn: u64,
        active_elapsed: Duration,
        remaining_bytes: u64,
        seg_shed_count: u32,
        speed_limit_bps: u64,
        limit_generation: u64,
        worker_count: usize,
    ) -> bool {
        if speed_limit_bps > 0 || worker_count <= 1 {
            self.clear_worker(worker_id);
            return false;
        }

        let now = Instant::now();
        let min_dur = self.min_sample_duration();
        let ttl = self.freshness_ttl();
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if limit_generation != guard.last_limit_generation {
            guard.peers.fill(None);
            guard.recent_finished.clear();
            guard.last_limit_generation = limit_generation;
        }

        while let Some((ts, _)) = guard.recent_finished.front() {
            if now.saturating_duration_since(*ts) > ttl {
                guard.recent_finished.pop_front();
            } else {
                break;
            }
        }
        for slot in &mut guard.peers {
            if let Some((ts, _)) = *slot {
                if now.saturating_duration_since(ts) > ttl {
                    *slot = None;
                }
            }
        }

        if bytes_this_conn >= self.config.min_sample_bytes && active_elapsed >= min_dur {
            let rate = bytes_this_conn as f64 / active_elapsed.as_secs_f64();
            if let Some(slot) = guard.peers.get_mut(worker_id) {
                *slot = Some((now, rate));
            }
        }

        if active_elapsed < self.config.grace_duration
            || remaining_bytes == 0
            || seg_shed_count >= self.config.max_sheds_per_segment
        {
            return false;
        }

        let best_peer_bps = guard
            .peers
            .iter()
            .enumerate()
            .filter_map(|(idx, sample)| {
                (idx != worker_id).then_some(())?;
                sample.map(|(_, rate)| rate)
            })
            .chain(guard.recent_finished.iter().map(|(_, rate)| *rate))
            .fold(0.0_f64, f64::max);

        if best_peer_bps <= 0.0 {
            return false;
        }

        let worker_bps = bytes_this_conn as f64 / active_elapsed.as_secs_f64().max(1e-6);
        let est_remaining_secs = remaining_bytes as f64 / worker_bps.max(1.0);

        if best_peer_bps > worker_bps * self.config.ratio_threshold
            && est_remaining_secs > self.config.min_remaining_secs
        {
            let mut cur_sheds = self.total_sheds.load(Ordering::Acquire);
            while cur_sheds < self.config.max_sheds_per_transfer {
                match self.total_sheds.compare_exchange_weak(
                    cur_sheds,
                    cur_sheds + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        if let Some(slot) = guard.peers.get_mut(worker_id) {
                            *slot = None;
                        }
                        return true;
                    }
                    Err(observed) => cur_sheds = observed,
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_segment_pool_work_steals_and_clamps_shrunk_victim() {
        let pool = DynamicSegmentPool::new(
            4 * 1024 * 1024,
            vec![
                SegmentProgressEntry {
                    start: 0,
                    end: 2 * 1024 * 1024,
                    written: 0,
                },
                SegmentProgressEntry {
                    start: 2 * 1024 * 1024,
                    end: 4 * 1024 * 1024,
                    written: 2 * 1024 * 1024,
                },
            ],
            512 * 1024,
            true,
        )
        .unwrap();

        let seg0 = pool.acquire_or_split(0).unwrap();
        assert_eq!(seg0.seg_id, 0);
        assert_eq!((seg0.start, seg0.end), (0, 2 * 1024 * 1024));

        // Victim claims and writes 256 KiB before thief steals the upper half.
        let c1 = pool.claim_write(0, 256 * 1024);
        assert_eq!(c1.write_offset, 0);
        assert_eq!(c1.allowed, 256 * 1024);
        assert!(!c1.hit_shrunk_end);
        pool.commit_written(0, 256 * 1024);

        // Worker 1 acquires by bisecting segment 0!
        let stolen = pool.acquire_or_split(1).unwrap();
        assert_eq!(stolen.seg_id, 2);
        assert!(stolen.start >= 256 * 1024 + 512 * 1024);
        assert_eq!(stolen.end, 2 * 1024 * 1024);
        assert_eq!(pool.split_count(), 1);

        // Victim tries to claim all the way past the shrunk end: clamped cleanly!
        let c2 = pool.claim_write(0, 2 * 1024 * 1024);
        assert_eq!(c2.write_offset, 256 * 1024);
        assert_eq!(c2.allowed, stolen.start - 256 * 1024);
        assert!(c2.hit_shrunk_end);
        pool.commit_written(0, c2.allowed);
        pool.release_segment(0, 0, false);

        // Thief completes its split tail.
        let rem_stolen = stolen.end - stolen.start;
        let c3 = pool.claim_write(stolen.seg_id, rem_stolen);
        assert_eq!(c3.write_offset, stolen.start);
        assert_eq!(c3.allowed, rem_stolen);
        pool.commit_written(stolen.seg_id, c3.allowed);
        pool.release_segment(stolen.seg_id, 1, false);

        assert!(pool.is_all_complete());
        let meta = pool.snapshot_meta();
        assert!(meta.is_valid_partition());
        assert_eq!(meta.segments.len(), 3);
    }

    #[test]
    fn bandwidth_limiter_never_deadlocks_when_chunk_exceeds_burst_and_handles_live_changes() {
        // 32 KiB/s limit (max burst ~8 KiB) with 1 MiB read request (transfer default chunk_size).
        let limiter = Arc::new(BandwidthLimiter::new(32 * 1024));
        let _guard = limiter.begin_stream();
        let cancelled = AtomicBool::new(false);
        let paused = AtomicBool::new(false);

        let AcquireOutcome::Granted(granted) = limiter.acquire(
            1024 * 1024,
            &|| cancelled.load(Ordering::Relaxed),
            &|| paused.load(Ordering::Relaxed),
            None,
        ) else {
            panic!("must not deadlock or fail when chunk_size > burst");
        };
        assert!(granted > 0 && granted <= 8 * 1024);

        // Pausing mid-throttle immediately returns AcquireOutcome::Paused (never traps sleep loop).
        paused.store(true, Ordering::Release);
        assert_eq!(
            limiter.acquire(
                1024 * 1024,
                &|| false,
                &|| paused.load(Ordering::Relaxed),
                None
            ),
            AcquireOutcome::Paused
        );
        paused.store(false, Ordering::Release);

        // Lower limit live, then reset to 0 (unlimited).
        limiter.set_limit_bps(16 * 1024);
        let scaled = limiter.effective_stall_min_bps(32 * 1024);
        assert!((1..=4 * 1024).contains(&scaled));

        limiter.set_limit_bps(0);
        assert_eq!(
            limiter.acquire(1024 * 1024, &|| false, &|| false, None),
            AcquireOutcome::Granted(1024 * 1024)
        );
        assert_eq!(limiter.effective_stall_min_bps(32 * 1024), 32 * 1024);
    }

    #[test]
    fn bandwidth_limiter_discards_idle_pre_transfer_refill_on_first_stream() {
        let limiter = Arc::new(BandwidthLimiter::new(16 * 1024));
        // Simulate expensive HTTP client construction before transfer streams begin.
        thread::sleep(Duration::from_millis(180));
        let _guard = limiter.begin_stream();
        let AcquireOutcome::Granted(first) = limiter.acquire(16 * 1024, &|| false, &|| false, None)
        else {
            panic!("expected initial burst grant");
        };
        assert_eq!(
            first, 4096,
            "idle period before first stream must not accumulate extra burst tokens"
        );
    }

    #[test]
    fn host_concurrency_tracker_debounces_burst_and_recovers_after_clean_completions() {
        let tracker = HostConcurrencyTracker::with_timings(
            Duration::from_secs(5),
            Duration::from_millis(20),
            Duration::from_secs(60),
        );
        let url_a = Url::parse("http://127.0.0.1:41001/a.bin").unwrap();
        let url_b = Url::parse("http://127.0.0.1:41002/b.bin").unwrap();

        // First burst steps 8 -> 4; immediate second burst within debounce does NOT collapse to 1.
        assert_eq!(tracker.record_rate_limit(&url_a, 8), Some(4));
        assert_eq!(tracker.record_rate_limit(&url_a, 8), Some(4));
        assert_eq!(tracker.effective_cap(&url_b), None);

        thread::sleep(Duration::from_millis(25));
        tracker.record_clean_completion(&url_a);
        tracker.record_clean_completion(&url_a);
        // 4 -> 8 restores full capacity (removes entry).
        assert_eq!(tracker.effective_cap(&url_a), None);
    }

    #[test]
    fn relative_speed_tracker_sheds_straggler_unless_speed_limited() {
        let tracker = RelativeSpeedTracker::new(
            2,
            RelativeSpeedConfig {
                min_sample_bytes: 64 * 1024,
                grace_duration: Duration::from_millis(40),
                ratio_threshold: 4.0,
                min_remaining_secs: 0.5,
                max_sheds_per_transfer: 4,
                max_sheds_per_segment: 2,
            },
            0,
        );

        // Worker 0 runs at 1 MiB/s (256 KiB in 250ms).
        assert!(!tracker.report_and_check_shed(
            0,
            256 * 1024,
            Duration::from_millis(250),
            512 * 1024,
            0,
            0,
            0,
            2,
        ));
        // Worker 1 transferred only 4 KiB in 250ms (~16 KiB/s, >60x slower, >60s left) -> shed!
        assert!(tracker.report_and_check_shed(
            1,
            4 * 1024,
            Duration::from_millis(250),
            1024 * 1024,
            0,
            0,
            0,
            2,
        ));
        // With speed limit active, shedding is suppressed!
        assert!(!tracker.report_and_check_shed(
            1,
            4 * 1024,
            Duration::from_millis(250),
            1024 * 1024,
            0,
            64 * 1024,
            1,
            2,
        ));
    }

    #[test]
    fn record_completed_stream_clamps_sub_threshold_duration_for_peer_reference() {
        let tracker = RelativeSpeedTracker::new(
            2,
            RelativeSpeedConfig {
                min_sample_bytes: 32 * 1024,
                grace_duration: Duration::from_millis(60),
                ratio_threshold: 4.0,
                min_remaining_secs: 0.5,
                max_sheds_per_transfer: 4,
                max_sheds_per_segment: 2,
            },
            0,
        );

        // Worker 0 drained 128 KiB in 200us (< min_sample_duration 15ms); clamped to 15ms (~8.5 MiB/s).
        tracker.record_completed_stream(0, 128 * 1024, Duration::from_micros(200), 0, 0);

        // Worker 1 transferred 8 KiB in 250ms (~32 KiB/s, >4x slower, >0.5s remaining) -> shed!
        assert!(tracker.report_and_check_shed(
            1,
            8 * 1024,
            Duration::from_millis(250),
            120 * 1024,
            0,
            0,
            0,
            2,
        ));

        // With speed limit active, completed streams do not populate references and shedding stays off.
        let throttled = RelativeSpeedTracker::new(
            2,
            RelativeSpeedConfig {
                min_sample_bytes: 32 * 1024,
                grace_duration: Duration::from_millis(60),
                ratio_threshold: 4.0,
                min_remaining_secs: 0.5,
                max_sheds_per_transfer: 4,
                max_sheds_per_segment: 2,
            },
            0,
        );
        throttled.record_completed_stream(0, 128 * 1024, Duration::from_micros(200), 64 * 1024, 0);
        assert!(!throttled.report_and_check_shed(
            1,
            8 * 1024,
            Duration::from_millis(250),
            120 * 1024,
            0,
            64 * 1024,
            0,
            2,
        ));
    }
}
