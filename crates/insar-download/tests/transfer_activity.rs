use std::time::{Duration, Instant};

use insar_download::TaskTransferActivityRegistry;

#[test]
fn production_worker_keeps_unobserved_network_measurement_unknown() {
    use insar_download::{DownloadWorker, ProductionDownloadWorker, TransferOptions};

    // Construct lazy HTTP/resolver clients only. Do not execute or resolve a download.
    let worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
    assert_eq!(worker.task_transfer_sample("no-body-observed"), None);
}

#[test]
fn unobserved_task_is_distinct_from_a_measured_zero_byte_body() {
    let tracker = TaskTransferActivityRegistry::default();
    let now = Instant::now();
    assert_eq!(tracker.sample_if_known("new", now).unwrap(), None);
    tracker.body_started("new", "item", 1, now).unwrap();
    let measured = tracker.sample_if_known("new", now).unwrap().unwrap();
    assert_eq!(measured.network_bytes_received, 0);
    assert_eq!(measured.active_count, 1);
    tracker.body_stopped("new", "item", 1, now).unwrap();
    assert_eq!(
        tracker
            .sample_if_known("new", now)
            .unwrap()
            .unwrap()
            .active_count,
        0
    );
}

#[test]
fn overflowing_network_counter_rejects_the_delta_without_changing_prior_measurement() {
    let tracker = TaskTransferActivityRegistry::default();
    let now = Instant::now();
    tracker.body_started("task", "a", 1, now).unwrap();
    tracker.body_started("task", "b", 1, now).unwrap();
    tracker
        .observe_bytes("task", "a", 1, u64::MAX, now)
        .unwrap();
    let before = tracker.sample("task", now).unwrap();
    assert_eq!(
        tracker.observe_bytes("task", "b", 1, 1, now),
        Err(insar_download::TransferActivityError::ByteCountOverflow)
    );
    assert_eq!(tracker.sample("task", now).unwrap(), before);
    assert_eq!(
        tracker.observe_bytes("task", "b", 1, 0, now).unwrap(),
        before
    );
}

#[test]
fn source_bytes_accumulate_across_tiles_retries_and_parallel_items() {
    let tracker = TaskTransferActivityRegistry::default();
    let now = Instant::now();
    tracker.body_started("dem", "window-a", 1, now).unwrap();
    tracker
        .observe_bytes("dem", "window-a", 1, 100, now)
        .unwrap();
    tracker
        .observe_bytes("dem", "window-a", 1, 100, now)
        .unwrap();
    tracker.body_stopped("dem", "window-a", 1, now).unwrap();
    // A new tile's body-local count starts at zero without losing the first tile.
    tracker.body_started("dem", "window-a", 1, now).unwrap();
    tracker.body_started("dem", "window-b", 1, now).unwrap();
    tracker
        .observe_bytes("dem", "window-a", 1, 200, now)
        .unwrap();
    tracker
        .observe_bytes("dem", "window-b", 1, 300, now)
        .unwrap();
    assert_eq!(
        tracker.sample("dem", now).unwrap().network_bytes_received,
        600
    );
    tracker.body_stopped("dem", "window-a", 1, now).unwrap();
    tracker
        .body_started_at_bytes("dem", "window-a", 2, 200, now)
        .unwrap();
    tracker
        .observe_bytes("dem", "window-a", 2, 250, now)
        .unwrap();
    let final_sample = tracker.close_task("dem", now).unwrap();
    assert_eq!(final_sample.network_bytes_received, 650);
    assert_eq!(final_sample.bytes_per_second, 0.0);
}

#[test]
fn concurrent_items_accumulate_one_task_wall_clock_union() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();

    tracker.body_started("task", "item-a", 1, base).unwrap();
    tracker
        .body_started("task", "item-b", 1, base + Duration::from_secs(2))
        .unwrap();
    tracker
        .body_stopped("task", "item-a", 1, base + Duration::from_secs(8))
        .unwrap();
    tracker
        .body_stopped("task", "item-b", 1, base + Duration::from_secs(10))
        .unwrap();

    let sample = tracker
        .sample("task", base + Duration::from_secs(10))
        .unwrap();
    assert_eq!(sample.active_millis, 10_000);
    assert_eq!(sample.active_count, 0);
    assert_eq!(sample.bytes_per_second, 0.0);
}

#[test]
fn paused_and_retry_gaps_do_not_advance_transfer_time() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();

    tracker.body_started("task", "item", 1, base).unwrap();
    tracker
        .body_stopped("task", "item", 1, base + Duration::from_secs(4))
        .unwrap();
    tracker
        .body_started("task", "item", 2, base + Duration::from_secs(14))
        .unwrap();
    tracker
        .body_stopped("task", "item", 2, base + Duration::from_secs(17))
        .unwrap();

    assert_eq!(
        tracker
            .sample("task", base + Duration::from_secs(30))
            .unwrap()
            .active_millis,
        7_000
    );
}

#[test]
fn duplicate_body_signals_are_idempotent() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();

    tracker.body_started("task", "item", 1, base).unwrap();
    tracker
        .body_started("task", "item", 1, base + Duration::from_secs(2))
        .unwrap();
    tracker
        .body_stopped("task", "item", 1, base + Duration::from_secs(5))
        .unwrap();
    tracker
        .body_stopped("task", "item", 1, base + Duration::from_secs(9))
        .unwrap();

    assert_eq!(
        tracker
            .sample("task", base + Duration::from_secs(9))
            .unwrap()
            .active_millis,
        5_000
    );
}

#[test]
fn active_byte_deltas_produce_rate_and_body_stop_resets_it() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();

    tracker.body_started("task", "item", 1, base).unwrap();
    let first = tracker
        .observe_bytes("task", "item", 1, 100, base + Duration::from_secs(1))
        .unwrap();
    assert_eq!(first.bytes_per_second, 100.0);

    let second = tracker
        .observe_bytes("task", "item", 1, 300, base + Duration::from_secs(3))
        .unwrap();
    assert_eq!(second.bytes_per_second, 100.0);

    let stopped = tracker
        .body_stopped("task", "item", 1, base + Duration::from_secs(4))
        .unwrap();
    assert_eq!(stopped.active_millis, 4_000);
    assert_eq!(stopped.bytes_per_second, 0.0);
}

#[test]
fn close_task_closes_remaining_members_once() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();

    tracker.body_started("task", "a", 1, base).unwrap();
    tracker
        .body_started("task", "b", 1, base + Duration::from_secs(1))
        .unwrap();
    let closed = tracker
        .close_task("task", base + Duration::from_secs(5))
        .unwrap();
    assert_eq!(closed.active_millis, 5_000);
    assert_eq!(closed.active_count, 0);

    let replay = tracker
        .close_task("task", base + Duration::from_secs(20))
        .unwrap();
    assert_eq!(replay.active_millis, 5_000);
}

#[test]
fn non_monotonic_time_and_regressing_bytes_fail_without_corrupting_state() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();

    tracker.body_started("task", "item", 1, base).unwrap();
    tracker
        .observe_bytes("task", "item", 1, 200, base + Duration::from_secs(2))
        .unwrap();

    assert!(tracker
        .sample("task", base + Duration::from_secs(1))
        .is_err());
    assert!(tracker
        .observe_bytes("task", "item", 1, 100, base + Duration::from_secs(3),)
        .is_err());

    let final_sample = tracker
        .body_stopped("task", "item", 1, base + Duration::from_secs(4))
        .unwrap();
    assert_eq!(final_sample.active_millis, 4_000);
    assert!(final_sample.bytes_per_second.is_finite());
}

#[test]
fn cross_item_lock_reordering_clamps_to_the_latest_task_observation() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();

    tracker.body_started("task", "a", 1, base).unwrap();
    tracker
        .body_started("task", "b", 1, base + Duration::from_secs(2))
        .unwrap();

    // Item A captured its stop instant before item B acquired the shared lock.
    // Applying it after B must not fail or move the task clock backwards.
    tracker
        .body_stopped("task", "a", 1, base + Duration::from_secs(1))
        .unwrap();
    let stopped = tracker
        .body_stopped("task", "b", 1, base + Duration::from_secs(5))
        .unwrap();

    assert_eq!(stopped.active_millis, 5_000);
}

#[test]
fn concurrent_members_keep_independent_rates_for_store_aggregation() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();

    tracker.body_started("task", "a", 1, base).unwrap();
    tracker.body_started("task", "b", 1, base).unwrap();
    let first = tracker
        .observe_bytes("task", "a", 1, 100, base + Duration::from_secs(1))
        .unwrap();
    let second = tracker
        .observe_bytes("task", "b", 1, 400, base + Duration::from_secs(2))
        .unwrap();

    assert_eq!(
        tracker.member_bytes_per_second("task", "a", 1).unwrap(),
        100.0
    );
    assert_eq!(
        tracker.member_bytes_per_second("task", "b", 1).unwrap(),
        200.0
    );
    assert_eq!(first.bytes_per_second, 100.0);
    assert_eq!(second.bytes_per_second, 300.0);
}

#[test]
fn subsecond_network_chunks_do_not_publish_spiky_instantaneous_rates() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();
    const MIB: u64 = 1024 * 1024;

    tracker.body_started("task", "item", 1, base).unwrap();
    let too_early = tracker
        .observe_bytes("task", "item", 1, MIB, base + Duration::from_millis(10))
        .unwrap();
    assert_eq!(too_early.bytes_per_second, 0.0);

    let stable = tracker
        .observe_bytes("task", "item", 1, 2 * MIB, base + Duration::from_secs(1))
        .unwrap();
    assert_eq!(stable.bytes_per_second, (2 * MIB) as f64);

    let burst = tracker
        .observe_bytes(
            "task",
            "item",
            1,
            3 * MIB,
            base + Duration::from_millis(1_010),
        )
        .unwrap();
    assert_eq!(
        burst.bytes_per_second,
        (2 * MIB) as f64,
        "a 10 ms chunk must retain the completed one-second window instead of reporting 100 MiB/s",
    );
}

#[test]
fn resumed_body_uses_the_existing_partial_length_as_its_rate_baseline() {
    let base = Instant::now();
    let tracker = TaskTransferActivityRegistry::default();
    const MIB: u64 = 1024 * 1024;

    tracker
        .body_started_at_bytes("task", "item", 1, 500 * MIB, base)
        .unwrap();
    let first = tracker
        .observe_bytes("task", "item", 1, 501 * MIB, base + Duration::from_secs(1))
        .unwrap();
    assert_eq!(first.bytes_per_second, MIB as f64);
    tracker
        .body_stopped("task", "item", 1, base + Duration::from_millis(1_100))
        .unwrap();

    tracker
        .body_started_at_bytes("task", "item", 1, 501 * MIB, base + Duration::from_secs(10))
        .unwrap();
    let resumed = tracker
        .observe_bytes("task", "item", 1, 502 * MIB, base + Duration::from_secs(11))
        .unwrap();
    assert_eq!(resumed.bytes_per_second, MIB as f64);
}
