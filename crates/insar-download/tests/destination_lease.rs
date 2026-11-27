use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use insar_download::{
    destination_lease_key_for_qualification_test, DestinationLeaseController,
    DestinationLeaseError, DestinationLeaseGuard, DestinationLeaseSnapshot, OpaqueDestinationKey,
    WorkerControl,
};

const WAIT: Duration = Duration::from_secs(2);

fn controller() -> Arc<DestinationLeaseController> {
    DestinationLeaseController::new_for_qualification_test()
}

fn key(label: u64) -> OpaqueDestinationKey {
    destination_lease_key_for_qualification_test(label)
}

fn acquire(
    controller: &Arc<DestinationLeaseController>,
    destination: OpaqueDestinationKey,
) -> Result<DestinationLeaseGuard, DestinationLeaseError> {
    controller.acquire_for_qualification_test(destination, || false, || false)
}

fn wait_for_snapshot(
    controller: &DestinationLeaseController,
    predicate: impl Fn(DestinationLeaseSnapshot) -> bool,
) {
    let deadline = Instant::now() + WAIT;
    loop {
        let snapshot = controller.snapshot_for_qualification_test();
        if predicate(snapshot) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "destination lease qualification wait timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn assert_idle(controller: &DestinationLeaseController) {
    let snapshot = controller.snapshot_for_qualification_test();
    assert_eq!(snapshot.live_destinations(), 0);
    assert_eq!(snapshot.queued_waiters(), 0);
    assert_eq!(snapshot.active_leases(), 0);
    assert!(!snapshot.failed_closed());
}

fn send_result(
    sender: mpsc::Sender<Result<(), DestinationLeaseError>>,
    result: Result<DestinationLeaseGuard, DestinationLeaseError>,
) {
    let result = result.map(|guard| {
        drop(guard);
    });
    sender.send(result).unwrap();
}

#[test]
fn same_destination_waiter_cannot_overlap_the_owner() {
    let controller = controller();
    let destination = key(1);
    let owner = acquire(&controller, destination).unwrap();
    let (sender, receiver) = mpsc::channel();
    let worker_controller = Arc::clone(&controller);
    let worker = thread::spawn(move || {
        send_result(sender, acquire(&worker_controller, destination));
    });

    wait_for_snapshot(&controller, |snapshot| snapshot.queued_waiters() == 1);
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    drop(owner);
    assert_eq!(receiver.recv_timeout(WAIT).unwrap(), Ok(()));
    worker.join().unwrap();
    assert_idle(&controller);
}

#[test]
fn different_destinations_acquire_concurrently_without_global_capacity() {
    let controller = controller();
    let first = acquire(&controller, key(1)).unwrap();
    let second = acquire(&controller, key(2)).unwrap();

    let snapshot = controller.snapshot_for_qualification_test();
    assert_eq!(snapshot.live_destinations(), 2);
    assert_eq!(snapshot.active_leases(), 2);
    assert_eq!(snapshot.queued_waiters(), 0);
    drop(second);
    drop(first);
    assert_idle(&controller);
}

#[test]
fn same_destination_waiters_are_strict_fifo() {
    let controller = controller();
    let destination = key(1);
    let owner = acquire(&controller, destination).unwrap();
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut workers = Vec::new();

    for position in 1..=3 {
        let worker_controller = Arc::clone(&controller);
        let worker_order = Arc::clone(&order);
        workers.push(thread::spawn(move || {
            let guard = acquire(&worker_controller, destination).unwrap();
            worker_order.lock().unwrap().push(position);
            drop(guard);
        }));
        wait_for_snapshot(&controller, |snapshot| {
            snapshot.queued_waiters() == position
        });
    }

    drop(owner);
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
    assert_idle(&controller);
}

#[test]
fn queued_cancellation_removes_the_waiter_and_does_not_strand_a_follower() {
    let controller = controller();
    let destination = key(1);
    let owner = acquire(&controller, destination).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let (cancel_sender, cancel_receiver) = mpsc::channel();
    let cancelled_controller = Arc::clone(&controller);
    let cancelled_flag = Arc::clone(&cancelled);
    let cancelled_worker = thread::spawn(move || {
        let result = cancelled_controller.acquire_for_qualification_test(
            destination,
            || cancelled_flag.load(Ordering::Acquire),
            || false,
        );
        send_result(cancel_sender, result);
    });
    wait_for_snapshot(&controller, |snapshot| snapshot.queued_waiters() == 1);

    let (follower_sender, follower_receiver) = mpsc::channel();
    let follower_controller = Arc::clone(&controller);
    let follower = thread::spawn(move || {
        send_result(follower_sender, acquire(&follower_controller, destination));
    });
    wait_for_snapshot(&controller, |snapshot| snapshot.queued_waiters() == 2);

    cancelled.store(true, Ordering::Release);
    assert_eq!(
        cancel_receiver.recv_timeout(WAIT).unwrap(),
        Err(DestinationLeaseError::Cancelled)
    );
    cancelled_worker.join().unwrap();
    wait_for_snapshot(&controller, |snapshot| snapshot.queued_waiters() == 1);

    drop(owner);
    assert_eq!(follower_receiver.recv_timeout(WAIT).unwrap(), Ok(()));
    follower.join().unwrap();
    assert_idle(&controller);
}

#[test]
fn queued_pause_never_acquires_until_resumed() {
    let controller = controller();
    let paused = Arc::new(AtomicBool::new(true));
    let (sender, receiver) = mpsc::channel();
    let worker_controller = Arc::clone(&controller);
    let worker_paused = Arc::clone(&paused);
    let worker = thread::spawn(move || {
        let result = worker_controller.acquire_for_qualification_test(
            key(1),
            || false,
            || worker_paused.load(Ordering::Acquire),
        );
        send_result(sender, result);
    });

    wait_for_snapshot(&controller, |snapshot| snapshot.queued_waiters() == 1);
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
    paused.store(false, Ordering::Release);
    assert_eq!(receiver.recv_timeout(WAIT).unwrap(), Ok(()));
    worker.join().unwrap();
    assert_idle(&controller);
}

#[test]
fn paused_owner_retains_its_destination_until_guard_release() {
    let controller = controller();
    let owner_paused = Arc::new(AtomicBool::new(false));
    let owner_paused_probe = Arc::clone(&owner_paused);
    let owner = controller
        .acquire_for_qualification_test(
            key(1),
            || false,
            || owner_paused_probe.load(Ordering::Acquire),
        )
        .unwrap();
    owner_paused.store(true, Ordering::Release);

    let (sender, receiver) = mpsc::channel();
    let worker_controller = Arc::clone(&controller);
    let worker = thread::spawn(move || {
        send_result(sender, acquire(&worker_controller, key(1)));
    });
    wait_for_snapshot(&controller, |snapshot| snapshot.queued_waiters() == 1);
    assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());

    drop(owner);
    assert_eq!(receiver.recv_timeout(WAIT).unwrap(), Ok(()));
    worker.join().unwrap();
    assert_idle(&controller);
}

#[test]
fn drop_during_unwind_releases_once_and_restores_idle_state() {
    let controller = controller();
    let unwind_controller = Arc::clone(&controller);
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let _guard = acquire(&unwind_controller, key(1)).unwrap();
        panic!("intentional destination lease qualification unwind");
    }));
    assert!(result.is_err());
    assert_idle(&controller);

    let guard = acquire(&controller, key(1)).unwrap();
    drop(guard);
    assert_idle(&controller);
}

#[test]
fn poison_invariant_and_counter_fail_closed_before_grant() {
    let exhausted = controller();
    exhausted.force_id_exhaustion_for_qualification_test();
    assert!(matches!(
        acquire(&exhausted, key(1)),
        Err(DestinationLeaseError::Unavailable)
    ));
    let exhausted_snapshot = exhausted.snapshot_for_qualification_test();
    assert!(exhausted_snapshot.failed_closed());
    assert_eq!(exhausted_snapshot.live_destinations(), 0);

    let invariant = controller();
    invariant.force_invariant_failure_for_qualification_test();
    assert!(matches!(
        acquire(&invariant, key(1)),
        Err(DestinationLeaseError::InvariantViolation)
    ));
    assert!(invariant.snapshot_for_qualification_test().failed_closed());

    let poisoned = controller();
    poisoned.poison_for_qualification_test();
    assert!(matches!(
        acquire(&poisoned, key(1)),
        Err(DestinationLeaseError::Unavailable)
    ));
    assert!(poisoned.snapshot_for_qualification_test().failed_closed());
}

#[test]
fn repeated_distinct_destination_cycles_leave_no_live_state() {
    let controller = controller();
    for label in 1..=128 {
        let guard = acquire(&controller, key(label)).unwrap();
        drop(guard);
        assert_idle(&controller);
    }
}

#[test]
fn diagnostics_redact_destination_identity_and_external_labels() {
    let controller = controller();
    let destination = key(777);
    controller.force_invariant_failure_for_qualification_test();
    let error = acquire(&controller, destination).unwrap_err();
    let snapshot = controller.snapshot_for_qualification_test();

    let rendered = format!("{controller:?} {destination:?} {snapshot:?} {error:?} {error}");
    assert!(!rendered.contains("777"));
    assert!(!rendered.contains("task-sensitive"));
    assert!(!rendered.contains("item-sensitive"));
    assert!(!rendered.contains("private-path"));
}

#[test]
fn independent_task_model_serializes_one_destination_but_not_another() {
    let controller = controller();
    let shared = key(1);
    let unrelated = key(2);
    let (owner_ready_sender, owner_ready_receiver) = mpsc::channel();
    let (owner_release_sender, owner_release_receiver) = mpsc::channel();
    let owner_controller = Arc::clone(&controller);
    let owner = thread::spawn(move || {
        let guard = acquire(&owner_controller, shared).unwrap();
        owner_ready_sender.send(()).unwrap();
        owner_release_receiver.recv_timeout(WAIT).unwrap();
        drop(guard);
    });
    owner_ready_receiver.recv_timeout(WAIT).unwrap();

    let (same_sender, same_receiver) = mpsc::channel();
    let same_controller = Arc::clone(&controller);
    let same_destination = thread::spawn(move || {
        send_result(same_sender, acquire(&same_controller, shared));
    });
    wait_for_snapshot(&controller, |snapshot| snapshot.queued_waiters() == 1);

    let (other_sender, other_receiver) = mpsc::channel();
    let other_controller = Arc::clone(&controller);
    let other_destination = thread::spawn(move || {
        send_result(other_sender, acquire(&other_controller, unrelated));
    });
    assert_eq!(other_receiver.recv_timeout(WAIT).unwrap(), Ok(()));
    assert!(same_receiver
        .recv_timeout(Duration::from_millis(100))
        .is_err());

    owner_release_sender.send(()).unwrap();
    assert_eq!(same_receiver.recv_timeout(WAIT).unwrap(), Ok(()));
    owner.join().unwrap();
    same_destination.join().unwrap();
    other_destination.join().unwrap();
    assert_idle(&controller);
}

#[test]
fn runtime_identity_uses_the_complete_lexically_normalized_planned_path() {
    let controller = controller();
    let root =
        std::env::temp_dir().join(format!("insar-destination-identity-{}", std::process::id()));
    let planned = root.join("target.bin");
    let lexical_alias = root.join("staging").join("..").join("target.bin");
    let different = root.join("target-copy.bin");
    let owner = controller
        .acquire_for_worker(&planned, &WorkerControl::default())
        .unwrap();

    let (same_sender, same_receiver) = mpsc::channel();
    let same_controller = Arc::clone(&controller);
    let same_waiter = thread::spawn(move || {
        let guard = same_controller
            .acquire_for_worker(&lexical_alias, &WorkerControl::default())
            .unwrap();
        drop(guard);
        same_sender.send(()).unwrap();
    });
    wait_for_snapshot(&controller, |snapshot| snapshot.queued_waiters() == 1);
    assert!(same_receiver
        .recv_timeout(Duration::from_millis(100))
        .is_err());

    let unrelated = controller
        .acquire_for_worker(&different, &WorkerControl::default())
        .unwrap();
    assert_eq!(
        controller.snapshot_for_qualification_test().active_leases(),
        2
    );
    drop(unrelated);
    drop(owner);

    same_receiver.recv_timeout(WAIT).unwrap();
    same_waiter.join().unwrap();
    assert_idle(&controller);
}

#[test]
fn runtime_lease_has_one_worker_owner_and_no_io_or_global_boundary() {
    let lease_source = include_str!("../src/destination_lease.rs");
    let worker_source = include_str!("../src/production_worker.rs");
    assert!(!lease_source.contains("std::fs"));
    assert!(!lease_source.contains("reqwest"));
    assert!(!lease_source.contains("static "));
    assert!(worker_source.contains("destination_lease: Arc<DestinationLeaseController>"));
    assert!(worker_source.contains("acquire_for_worker(destination, control)"));

    for production_source in [
        include_str!("../src/file_core.rs"),
        include_str!("../src/http_transfer.rs"),
        include_str!("../src/resolver_execution.rs"),
        include_str!("../src/safe_promotion.rs"),
        include_str!("../src/scheduler.rs"),
        include_str!("../src/transfer_admission.rs"),
        include_str!("../src/transfer_backend.rs"),
    ] {
        assert!(!production_source.contains("DestinationLeaseController"));
        assert!(!production_source.contains("destination_lease_key_for_qualification_test"));
    }
}
