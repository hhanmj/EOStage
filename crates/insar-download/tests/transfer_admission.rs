use std::num::NonZeroUsize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use insar_core::DownloadOutcome;
use insar_download::{
    AdmittedTransferBackend, AuthRefreshPolicyId, ResolvedAsset, ResolvedTransferResult,
    ResolvedTransferSession, TransferAdmissionController, TransferAdmissionPolicy, TransferAuth,
    TransferBackend, TransferBackendKind, TransferProgress, TransferRequest, TransferResult,
    TransferUpdate,
};

#[derive(Default)]
struct GateState {
    entered: usize,
    released: bool,
}

#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

impl Gate {
    fn enter_and_wait(&self) {
        let mut state = self.state.lock().unwrap();
        state.entered += 1;
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_for_entered(&self, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while state.entered < expected {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("backend did not enter before the deterministic deadline");
            let (next, timeout) = self.changed.wait_timeout(state, remaining).unwrap();
            state = next;
            assert!(
                !timeout.timed_out(),
                "backend did not enter before the deterministic deadline"
            );
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        self.changed.notify_all();
    }
}

struct TestBackend {
    calls: AtomicUsize,
    ordinary_resolved_calls: AtomicUsize,
    progress_callbacks: AtomicUsize,
    gate: Option<Arc<Gate>>,
    panic_on_call: bool,
}

impl TestBackend {
    fn completed() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            ordinary_resolved_calls: AtomicUsize::new(0),
            progress_callbacks: AtomicUsize::new(0),
            gate: None,
            panic_on_call: false,
        }
    }

    fn blocked(gate: Arc<Gate>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            ordinary_resolved_calls: AtomicUsize::new(0),
            progress_callbacks: AtomicUsize::new(0),
            gate: Some(gate),
            panic_on_call: false,
        }
    }

    fn panicking() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            ordinary_resolved_calls: AtomicUsize::new(0),
            progress_callbacks: AtomicUsize::new(0),
            gate: None,
            panic_on_call: true,
        }
    }

    fn invoke(&self, progress: &mut dyn FnMut(TransferUpdate)) -> TransferResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.progress_callbacks.fetch_add(1, Ordering::SeqCst);
        progress(TransferUpdate::BodyStarted {
            attempt: 1,
            bytes_completed: 0,
        });
        progress(TransferUpdate::Progress(TransferProgress {
            bytes_completed: 5,
            total_bytes: Some(5),
            attempt: 1,
        }));
        progress(TransferUpdate::BodyStopped { attempt: 1 });
        if let Some(gate) = &self.gate {
            gate.enter_and_wait();
        }
        assert!(!self.panic_on_call, "fixture backend panic");
        completed_result()
    }
}

impl TransferBackend for TestBackend {
    fn kind(&self) -> TransferBackendKind {
        TransferBackendKind::NativeSingleStream
    }

    fn is_available(&self) -> bool {
        true
    }

    fn download_legacy(
        &self,
        _request: &TransferRequest,
        _auth: &TransferAuth,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        self.invoke(progress)
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).unwrap()
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        _session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        self.ordinary_resolved_calls.fetch_add(1, Ordering::SeqCst);
        ResolvedTransferResult::Finished(self.invoke(progress))
    }
}

fn completed_result() -> TransferResult {
    TransferResult {
        outcome: DownloadOutcome::Completed,
        bytes_written: 5,
        attempt_count: 1,
        failure: None,
    }
}

fn request(task: &str, item: &str, url: &str) -> TransferRequest {
    TransferRequest::new(
        task,
        item,
        url,
        PathBuf::from(format!("{item}.bin")),
        Some(5),
    )
    .unwrap()
}

fn run_legacy(
    controller: Arc<TransferAdmissionController>,
    backend: &TestBackend,
    request: &TransferRequest,
    cancelled: &(dyn Fn() -> bool + Sync),
    paused: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(TransferUpdate),
) -> TransferResult {
    AdmittedTransferBackend::new(backend, controller).download_legacy(
        request,
        &TransferAuth::None,
        cancelled,
        paused,
        progress,
    )
}

fn spawn_legacy(
    controller: Arc<TransferAdmissionController>,
    backend: Arc<TestBackend>,
    task: &'static str,
    item: &'static str,
    url: &'static str,
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
) -> thread::JoinHandle<TransferResult> {
    thread::spawn(move || {
        let request = request(task, item, url);
        run_legacy(
            controller,
            backend.as_ref(),
            &request,
            &|| cancelled.load(Ordering::SeqCst),
            &|| paused.load(Ordering::SeqCst),
            &mut |_| {},
        )
    })
}

fn wait_until(message: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "{message} before the deterministic deadline"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn finite(global_limit: usize, per_host_limit: usize) -> Arc<TransferAdmissionController> {
    TransferAdmissionController::finite_for_test(
        NonZeroUsize::new(global_limit).unwrap(),
        NonZeroUsize::new(per_host_limit).unwrap(),
    )
}

fn assert_idle(controller: &TransferAdmissionController) {
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.active_global, 0);
    assert_eq!(snapshot.queued_global, 0);
    assert!(snapshot.active_by_host.is_empty());
    assert!(snapshot.queued_by_host.is_empty());
    assert!(snapshot.live_max_active_by_host.is_empty());
}

#[test]
fn compatibility_controller_delegates_once_and_returns_to_idle() {
    let controller = TransferAdmissionController::compatibility();
    let backend = TestBackend::completed();
    let progress = AtomicUsize::new(0);

    let result = run_legacy(
        Arc::clone(&controller),
        &backend,
        &request(
            "compatibility-task",
            "compatibility-item",
            "https://example.test/path?signature=redacted",
        ),
        &|| false,
        &|| false,
        &mut |update| {
            if matches!(update, TransferUpdate::Progress(_)) {
                progress.fetch_add(1, Ordering::SeqCst);
            }
        },
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.progress_callbacks.load(Ordering::SeqCst), 1);
    assert_eq!(progress.load(Ordering::SeqCst), 1);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.policy, TransferAdmissionPolicy::Compatibility);
    assert_eq!(snapshot.grants, 1);
    assert_eq!(snapshot.releases, 1);
    assert!(!snapshot.failed_closed);
    assert_idle(&controller);
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("example.test"));
    assert!(!debug.contains("compatibility-task"));
    assert!(!debug.contains("signature"));
}

#[test]
fn finite_host_block_does_not_consume_global_capacity() {
    let controller = finite(2, 1);
    let first_gate = Arc::new(Gate::default());
    let first = Arc::new(TestBackend::blocked(Arc::clone(&first_gate)));
    let blocked_same_host = Arc::new(TestBackend::completed());
    let other_host = TestBackend::completed();
    let cancelled = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));

    let first_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&first),
        "first-task",
        "first-item",
        "https://host-a.test/first",
        Arc::clone(&cancelled),
        Arc::clone(&paused),
    );
    first_gate.wait_for_entered(1);

    let blocked_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&blocked_same_host),
        "blocked-task",
        "blocked-item",
        "https://host-a.test/second",
        Arc::clone(&cancelled),
        Arc::clone(&paused),
    );
    wait_until("same-host waiter did not queue", || {
        controller.snapshot().queued_global == 1
    });
    assert_eq!(controller.snapshot().active_global, 1);

    let result = run_legacy(
        Arc::clone(&controller),
        &other_host,
        &request("other-task", "other-item", "https://host-b.test/allowed"),
        &|| false,
        &|| false,
        &mut |_| {},
    );
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(other_host.calls.load(Ordering::SeqCst), 1);
    assert_eq!(controller.snapshot().active_global, 1);

    first_gate.release();
    assert_eq!(
        first_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    assert_eq!(
        blocked_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    assert_eq!(blocked_same_host.calls.load(Ordering::SeqCst), 1);
    assert_idle(&controller);
}

#[test]
fn saturated_host_release_does_not_permanently_bypass_oldest_same_task_waiter() {
    let controller = finite(2, 1);
    let first_gate = Arc::new(Gate::default());
    let old_gate = Arc::new(Gate::default());
    let middle_gate = Arc::new(Gate::default());
    let first = Arc::new(TestBackend::blocked(Arc::clone(&first_gate)));
    let oldest = Arc::new(TestBackend::blocked(Arc::clone(&old_gate)));
    let middle = Arc::new(TestBackend::blocked(Arc::clone(&middle_gate)));
    let later = Arc::new(TestBackend::completed());
    let cancelled = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));

    let first_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&first),
        "holder-task",
        "holder-item",
        "https://host-a.test/holder",
        Arc::clone(&cancelled),
        Arc::clone(&paused),
    );
    first_gate.wait_for_entered(1);

    let oldest_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&oldest),
        "shared-task",
        "oldest-item",
        "https://host-a.test/oldest",
        Arc::clone(&cancelled),
        Arc::clone(&paused),
    );
    wait_until("oldest waiter did not queue", || {
        controller.snapshot().queued_global == 1
    });

    let middle_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&middle),
        "shared-task",
        "middle-item",
        "https://host-b.test/middle",
        Arc::clone(&cancelled),
        Arc::clone(&paused),
    );
    middle_gate.wait_for_entered(1);

    let later_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&later),
        "shared-task",
        "later-item",
        "https://host-c.test/later",
        Arc::clone(&cancelled),
        Arc::clone(&paused),
    );
    wait_until("later waiter did not queue", || {
        controller.snapshot().queued_global >= 2
    });

    first_gate.release();
    old_gate.wait_for_entered(1);
    assert_eq!(later.calls.load(Ordering::SeqCst), 0);

    old_gate.release();
    middle_gate.release();
    assert_eq!(
        first_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    assert_eq!(
        oldest_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    assert_eq!(
        middle_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    assert_eq!(
        later_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    assert_eq!(later.calls.load(Ordering::SeqCst), 1);
    assert_idle(&controller);
}

#[test]
fn queued_cancellation_removes_waiter_without_backend_call() {
    let controller = finite(1, 1);
    let gate = Arc::new(Gate::default());
    let holder = Arc::new(TestBackend::blocked(Arc::clone(&gate)));
    let cancelled = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    let holder_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&holder),
        "holder-task",
        "holder-item",
        "https://holder.test/file",
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    gate.wait_for_entered(1);

    let queued = Arc::new(TestBackend::completed());
    let queued_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&queued),
        "cancel-task",
        "cancel-item",
        "https://queued.test/file",
        Arc::clone(&cancelled),
        Arc::clone(&paused),
    );
    wait_until("cancelled waiter did not queue", || {
        controller.snapshot().queued_global == 1
    });
    cancelled.store(true, Ordering::SeqCst);

    let result = queued_handle.join().unwrap();
    assert_eq!(result.outcome, DownloadOutcome::Interrupted);
    assert_eq!(result.attempt_count, 0);
    assert_eq!(queued.calls.load(Ordering::SeqCst), 0);
    assert_eq!(controller.snapshot().cancellations_before_grant, 1);

    gate.release();
    assert_eq!(
        holder_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    assert_idle(&controller);
}

#[test]
fn prestart_pause_requeues_then_delegates_once_after_resume() {
    let controller = TransferAdmissionController::compatibility();
    let backend = Arc::new(TestBackend::completed());
    let allow_resume = Arc::new(AtomicBool::new(false));
    let pause_probes = Arc::new(AtomicUsize::new(0));
    let request = request("pause-task", "pause-item", "https://pause.test/file");
    let controller_for_thread = Arc::clone(&controller);
    let backend_for_thread = Arc::clone(&backend);
    let resume_for_thread = Arc::clone(&allow_resume);
    let probes_for_thread = Arc::clone(&pause_probes);

    let handle = thread::spawn(move || {
        run_legacy(
            controller_for_thread,
            backend_for_thread.as_ref(),
            &request,
            &|| false,
            &|| {
                let probe = probes_for_thread.fetch_add(1, Ordering::SeqCst);
                probe > 0 && !resume_for_thread.load(Ordering::SeqCst)
            },
            &mut |_| {},
        )
    });

    wait_until("prestart pause did not requeue", || {
        let snapshot = controller.snapshot();
        snapshot.prestart_releases == 1 && snapshot.queued_global == 1
    });
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    allow_resume.store(true, Ordering::SeqCst);

    assert_eq!(handle.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.grants, 2);
    assert_eq!(snapshot.releases, 2);
    assert_idle(&controller);
}

#[test]
fn repeated_distinct_task_and_origin_cycles_leave_no_registry_growth() {
    let controller = TransferAdmissionController::compatibility();
    let backend = TestBackend::completed();

    for index in 0..32 {
        let result = run_legacy(
            Arc::clone(&controller),
            &backend,
            &request(
                &format!("cycle-task-{index}"),
                &format!("cycle-item-{index}"),
                &format!("https://cycle-{index}.test/file"),
            ),
            &|| false,
            &|| false,
            &mut |_| {},
        );
        assert_eq!(result.outcome, DownloadOutcome::Completed);
        assert_idle(&controller);
    }

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.grants, 32);
    assert_eq!(snapshot.releases, 32);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 32);
}

#[test]
fn origin_normalization_coalesces_default_port_and_distinguishes_transport_hosts() {
    let controller = finite(6, 2);
    let gate = Arc::new(Gate::default());
    let cancelled = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    let requests = [
        ("dns-a", "https://EXAMPLE.test:443/one"),
        ("dns-b", "https://example.test/two"),
        ("http", "http://example.test/three"),
        ("port", "https://example.test:444/four"),
        ("ipv4", "https://127.0.0.1/five"),
        ("ipv6", "https://[::1]/six"),
    ];
    let mut handles = Vec::new();
    for (index, (item, url)) in requests.into_iter().enumerate() {
        handles.push(spawn_legacy(
            Arc::clone(&controller),
            Arc::new(TestBackend::blocked(Arc::clone(&gate))),
            "origin-task",
            item,
            url,
            Arc::clone(&cancelled),
            Arc::clone(&paused),
        ));
        let expected = index + 1;
        gate.wait_for_entered(expected);
    }

    let snapshot = controller.snapshot();
    assert_eq!(snapshot.active_global, 6);
    assert_eq!(snapshot.active_by_host.len(), 5);
    assert_eq!(
        snapshot
            .active_by_host
            .values()
            .filter(|count| **count == 2)
            .count(),
        1
    );
    assert!(
        snapshot
            .active_by_host
            .values()
            .filter(|count| **count == 1)
            .count()
            == 4
    );

    gate.release();
    for handle in handles {
        assert_eq!(handle.join().unwrap().outcome, DownloadOutcome::Completed);
    }
    assert_idle(&controller);
}

#[test]
fn id_exhaustion_fails_closed_without_backend_or_identity_leakage() {
    let controller = TransferAdmissionController::compatibility();
    let backend = TestBackend::completed();
    controller.exhaust_ids_for_test();

    let result = run_legacy(
        Arc::clone(&controller),
        &backend,
        &request(
            "secret-task-identity",
            "secret-item-identity",
            "https://secret-host.test/path?token=not-exposed",
        ),
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.attempt_count, 0);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    let snapshot = controller.snapshot();
    assert!(snapshot.failed_closed);
    assert_idle(&controller);
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("secret-host"));
    assert!(!debug.contains("secret-task"));
    assert!(!debug.contains("token"));
}

#[test]
fn paused_waiter_uses_no_capacity_until_resume() {
    let controller = finite(1, 1);
    let gate = Arc::new(Gate::default());
    let holder = Arc::new(TestBackend::blocked(Arc::clone(&gate)));
    let paused_backend = Arc::new(TestBackend::completed());
    let paused = Arc::new(AtomicBool::new(true));
    let holder_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&holder),
        "holder-task",
        "holder-item",
        "https://holder.test/file",
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    gate.wait_for_entered(1);

    let paused_handle = spawn_legacy(
        Arc::clone(&controller),
        Arc::clone(&paused_backend),
        "paused-task",
        "paused-item",
        "https://paused.test/file",
        Arc::new(AtomicBool::new(false)),
        Arc::clone(&paused),
    );
    wait_until("paused waiter did not register", || {
        let snapshot = controller.snapshot();
        snapshot.queued_global == 1 && snapshot.paused_waiters == 1
    });
    assert_eq!(controller.snapshot().active_global, 1);
    assert_eq!(paused_backend.calls.load(Ordering::SeqCst), 0);

    gate.release();
    assert_eq!(
        holder_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    wait_until("paused waiter retained capacity after release", || {
        let snapshot = controller.snapshot();
        snapshot.active_global == 0 && snapshot.queued_global == 1 && snapshot.paused_waiters == 1
    });
    assert_eq!(paused_backend.calls.load(Ordering::SeqCst), 0);

    paused.store(false, Ordering::SeqCst);
    assert_eq!(
        paused_handle.join().unwrap().outcome,
        DownloadOutcome::Completed
    );
    assert_eq!(paused_backend.calls.load(Ordering::SeqCst), 1);
    assert_idle(&controller);
}

#[test]
fn cancellation_observed_after_grant_releases_without_delegating() {
    let controller = TransferAdmissionController::compatibility();
    let backend = TestBackend::completed();
    let probes = AtomicUsize::new(0);

    let result = run_legacy(
        Arc::clone(&controller),
        &backend,
        &request("cancel-task", "cancel-item", "https://cancel.test/file"),
        &|| probes.fetch_add(1, Ordering::SeqCst) >= 2,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Interrupted);
    assert_eq!(result.attempt_count, 0);
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.grants, 1);
    assert_eq!(snapshot.releases, 1);
    assert_eq!(snapshot.prestart_releases, 1);
    assert_eq!(snapshot.cancellations_before_grant, 0);
    assert_idle(&controller);
}

#[test]
fn finite_policy_keeps_a_per_host_limit_above_global_limit_unchanged() {
    let global_limit = NonZeroUsize::new(1).unwrap();
    let per_host_limit = NonZeroUsize::new(2).unwrap();
    let controller = TransferAdmissionController::finite_for_test(global_limit, per_host_limit);

    assert_eq!(
        controller.policy(),
        TransferAdmissionPolicy::Finite {
            global_limit,
            per_host_limit,
        }
    );
}

#[test]
fn backend_panic_releases_the_permit_once() {
    let controller = TransferAdmissionController::compatibility();
    let backend = TestBackend::panicking();

    let result = catch_unwind(AssertUnwindSafe(|| {
        run_legacy(
            Arc::clone(&controller),
            &backend,
            &request("panic-task", "panic-item", "https://panic.test/file"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
    }));

    assert!(result.is_err());
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.grants, 1);
    assert_eq!(snapshot.releases, 1);
    assert_idle(&controller);
}
