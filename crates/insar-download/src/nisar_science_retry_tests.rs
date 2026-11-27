//! Bounded automatic retry of one science Range after a transient failure
//! (design 2026-10-17 §3 and §6). Loopback responses only: the real executor,
//! the real HTTP 206 validation path and the in-memory checkpoint anchor.
use super::*;
use crate::nisar_science_transfer_v2::{
    advance_legacy_science_state_v2_for_test, wait_for_science_retry_with_clock,
    LegacyScienceChangeV2ForTest, NisarScienceFailureDiagnosticV2,
    NisarScienceTransferProgressV2 as Progress, NisarSubsetScienceRangeExecutorV2,
    NisarSubsetScienceRemoteSourceV2, NisarSubsetScienceTransferErrorV2 as Error,
    ScienceRetryClock,
};
use std::sync::atomic::AtomicBool;

/// Serves the scripted responses in order and records every request. Extra
/// connections are recorded and closed without a reply, so a missing request
/// never blocks the test and an unexpected one is visible.
struct RetryScienceServer {
    url_root: String,
    done: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl RetryScienceServer {
    fn start(responses: Vec<Vec<u8>>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url_root = format!("http://{}", listener.local_addr().unwrap());
        let done = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (worker_done, captured) = (Arc::clone(&done), Arc::clone(&requests));
        let worker = thread::spawn(move || {
            let mut script = responses.into_iter();
            let deadline = Instant::now() + Duration::from_secs(120);
            while !worker_done.load(Ordering::SeqCst) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream.set_nodelay(true).unwrap();
                        captured
                            .lock()
                            .unwrap()
                            .push(read_science_request(&mut stream));
                        if let Some(response) = script.next() {
                            let _ = stream.write_all(&response);
                            let _ = stream.flush();
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("retry science server accept failed: {error}"),
                }
            }
        });
        Self {
            url_root,
            done,
            requests,
            worker: Some(worker),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn finish(mut self) -> Vec<String> {
        self.done.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
        self.requests.lock().unwrap().clone()
    }
}

struct RetryCase {
    _case: ReadyCase,
    work: DownloadWork,
    completed: NisarSubsetPlanningCompleteV2,
    bootstrap: Arc<NisarSubsetLocalHdf5MetadataBootstrapV2>,
    source_path: PathBuf,
    ranges: Vec<NisarSubsetPhysicalRangeV2>,
    /// Shared so a test hook can inject a later Store failpoint.
    anchor: Arc<ScienceCheckpointAnchorFixture>,
}

fn retry_case(label: &str) -> RetryCase {
    let (case, intent, bootstrap, source_path) =
        concrete_fixture(label, NisarSubsetFrequency::A, &["HH"]);
    let mut work = work(&case, &intent);
    let completed = plan_and_freeze_nisar_subset_v2_with(
        &work,
        &WorkerControl::default(),
        bootstrap.as_ref(),
        |ready, _| Ok(ready.proposed_plan),
    )
    .unwrap();
    embed_nisar_subset_plan_v2_envelope(&mut work.item.metadata, completed.frozen_plan()).unwrap();
    let ranges = completed
        .physical_execution_plan()
        .execution_transfer_ranges()
        .unwrap();
    assert!(ranges.len() >= 5, "the retry cases need several Ranges");
    RetryCase {
        _case: case,
        work,
        completed,
        bootstrap,
        source_path,
        ranges,
        anchor: Arc::new(ScienceCheckpointAnchorFixture::default()),
    }
}

impl RetryCase {
    fn physical(&self) -> &NisarSubsetPhysicalExecutionPlanV2 {
        self.completed.physical_execution_plan()
    }

    fn ok(&self, index: usize) -> Vec<u8> {
        let range = &self.ranges[index];
        exact_science_response(
            range,
            self.physical().source_size(),
            &self.physical().source_validator().value,
            &source_range_bytes(&self.source_path, range),
        )
    }

    fn ok_from(&self, first: usize) -> Vec<Vec<u8>> {
        (first..self.ranges.len())
            .map(|index| self.ok(index))
            .collect()
    }

    /// Full 206 headers, then the connection closes after half of the body.
    fn truncated(&self, index: usize) -> Vec<u8> {
        let mut response = self.ok(index);
        let body = self.ranges[index].len().unwrap() as usize;
        assert!(body >= 2);
        response.truncate(response.len() - body / 2);
        response
    }

    fn executor(&self, server: &RetryScienceServer) -> NisarSubsetScienceRangeExecutorV2 {
        NisarSubsetScienceRangeExecutorV2::new(science_http(), self.asset(server))
            .unwrap()
            .with_retry_backoff_for_test(Arc::new(|_, _| Duration::ZERO))
    }

    fn asset(&self, server: &RetryScienceServer) -> ResolvedAsset {
        science_asset(
            &server.url_root,
            self.physical(),
            Some(&self.physical().source_validator().value),
            None,
            None,
        )
    }

    fn authority(&self, server: &RetryScienceServer) -> StaticScienceAuthorityFixture<'_> {
        StaticScienceAuthorityFixture {
            plan: self.physical().clone(),
            statistics: self.completed.bootstrap_statistics().clone(),
            workspace: self.bootstrap.science_workspace_v2(),
            remote_source: NisarSubsetScienceRemoteSourceV2::from_frozen_authority(
                self.asset(server).effective_url(),
                &self.work,
                &self.completed,
                self.physical(),
            )
            .unwrap(),
            anchor: &self.anchor,
        }
    }

    fn checkpoint(&self) -> Value {
        serde_json::from_str(
            &self
                .bootstrap
                .science_workspace_v2()
                .load_science_checkpoint_v2()
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }

    fn requests_for(&self, requests: &[String], index: usize) -> usize {
        let range = &self.ranges[index];
        let expected = format!("bytes={}-{}", range.start(), range.end_exclusive() - 1);
        requests
            .iter()
            .filter(|request| science_request_header(request, "Range") == Some(expected.as_str()))
            .count()
    }
}

fn status_only(status: &str) -> Vec<u8> {
    science_response(status, &[("Content-Length", "0".to_owned())], &[])
}

fn counter(checkpoint: &Value, key: &str) -> u64 {
    checkpoint[key].as_u64().unwrap_or_else(|| panic!("{key}"))
}

/// A transient failure of one Range is retried once and the Range then
/// completes: two requests with the identical Range, one failed attempt charged
/// (never refunded) and every Range committed exactly once.
#[test]
fn science_retry_v2_recovers_from_503_connection_reset_and_truncated_body() {
    for kind in ["503", "reset", "truncated"] {
        let case = retry_case(&format!("science-retry-v2-recover-{kind}"));
        let failed = match kind {
            "503" => status_only("503 Service Unavailable"),
            "reset" => Vec::new(),
            "truncated" => case.truncated(1),
            _ => unreachable!(),
        };
        let mut script = vec![case.ok(0), failed];
        script.extend(case.ok_from(1));
        let server = RetryScienceServer::start(script);
        let executor = case.executor(&server);
        let result = executor
            .execute(
                &case.work,
                &case.completed,
                &case.authority(&server),
                &WorkerControl::default(),
            )
            .unwrap_or_else(|error| panic!("{kind}: {error:?}"));
        assert!(result.complete(), "{kind}");
        let requests = server.finish();
        assert_eq!(requests.len(), case.ranges.len() + 1, "{kind}");
        assert_eq!(
            case.requests_for(&requests, 1),
            2,
            "{kind}: same Range retried"
        );
        for index in (0..case.ranges.len()).filter(|index| *index != 1) {
            assert_eq!(
                case.requests_for(&requests, index),
                1,
                "{kind}: Range {index}"
            );
        }
        let checkpoint = case.checkpoint();
        assert_eq!(
            counter(&checkpoint, "failed_science_request_count"),
            1,
            "{kind}"
        );
        assert_eq!(
            counter(&checkpoint, "indeterminate_science_request_count"),
            0
        );
        assert_eq!(
            counter(&checkpoint, "committed_science_request_count"),
            case.ranges.len() as u64,
            "{kind}"
        );
        let total: u64 = case.ranges.iter().map(|range| range.len().unwrap()).sum();
        assert_eq!(
            counter(&checkpoint, "committed_science_response_body_bytes"),
            total,
            "{kind}"
        );
        assert_eq!(
            executor.last_failure_diagnostic(),
            None,
            "{kind}: success clears it"
        );
    }
}

/// HTTP 200 for a Range is never retried and never falls back to a full scene.
#[test]
fn science_retry_v2_treats_http_200_as_terminal_without_retry() {
    let case = retry_case("science-retry-v2-http-200");
    let full = science_response(
        "200 OK",
        &[
            ("ETag", case.physical().source_validator().value.clone()),
            ("Content-Length", "1".to_owned()),
        ],
        b"x",
    );
    let server = RetryScienceServer::start(vec![full]);
    let executor = case.executor(&server);
    let error = executor
        .execute(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
        )
        .unwrap_err();
    assert_eq!(error, Error::RangeUnsupported);
    assert_eq!(server.finish().len(), 1);
    let diagnostic = executor.last_failure_diagnostic().unwrap();
    assert_eq!(diagnostic.http_status, Some(200));
    assert_eq!(diagnostic.attempts, 1);
    assert!(!diagnostic.transient);
}

/// A Range still rejected with 401 after one successful refresh is terminal and
/// keeps the real HTTP status; it is not retried as a transient failure.
#[test]
fn science_retry_v2_rejection_after_refresh_is_terminal_with_its_status() {
    let case = retry_case("science-retry-v2-401-after-refresh");
    let server = RetryScienceServer::start(vec![
        status_only("401 Unauthorized"),
        status_only("401 Unauthorized"),
    ]);
    let executor = case.executor(&server);
    let mut refreshes = 0;
    let error = executor
        .execute_with_progress_and_auth_refresh(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
            &mut |_| {},
            &mut |_, _| {
                refreshes += 1;
                Ok(case.asset(&server))
            },
        )
        .unwrap_err();
    assert_eq!(error, Error::ResponseInvalid);
    assert_eq!(refreshes, 1);
    assert_eq!(server.finish().len(), 2);
    let diagnostic = executor.last_failure_diagnostic().unwrap();
    assert_eq!(diagnostic.http_status, Some(401));
    assert_eq!(diagnostic.attempts, 1);
    assert!(!diagnostic.transient);
}

/// Three attempts of one Range all answered 503: the Range fails after the
/// first attempt plus two automatic retries, with the real status and count.
#[test]
fn science_retry_v2_three_503_attempts_end_with_status_and_attempt_count() {
    let case = retry_case("science-retry-v2-503-exhausted");
    let server = RetryScienceServer::start(vec![
        status_only("503 Service Unavailable"),
        status_only("503 Service Unavailable"),
        status_only("503 Service Unavailable"),
    ]);
    let executor = case.executor(&server);
    let error = executor
        .execute(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
        )
        .unwrap_err();
    assert_eq!(error, Error::Transport);
    let requests = server.finish();
    assert_eq!(requests.len(), 3);
    assert_eq!(case.requests_for(&requests, 0), 3);
    assert_eq!(
        executor.last_failure_diagnostic(),
        Some(NisarScienceFailureDiagnosticV2 {
            range_index: 0,
            range_count: case.ranges.len(),
            attempts: 3,
            automatic_retries: 2,
            http_status: Some(503),
            transport_cause: Some("retryable_http_status"),
            transient: true,
            auth_refresh_failure: false,
            budget_stopped: false,
        })
    );
    let checkpoint = case.checkpoint();
    assert_eq!(counter(&checkpoint, "failed_science_request_count"), 3);
    assert!(checkpoint["pending_attempt"].is_null());
    assert_eq!(checkpoint["state"], "Interrupted");
}

/// At most eight automatic retries per execution: four Ranges that each need
/// two retries use them all, so the next Range's first failure is terminal.
#[test]
fn science_retry_v2_execution_wide_retry_cap_is_eight() {
    let case = retry_case("science-retry-v2-execution-cap");
    let mut script = Vec::new();
    for index in 0..4 {
        script.push(status_only("503 Service Unavailable"));
        script.push(status_only("502 Bad Gateway"));
        script.push(case.ok(index));
    }
    script.push(status_only("503 Service Unavailable"));
    let server = RetryScienceServer::start(script);
    let executor = case.executor(&server);
    let error = executor
        .execute(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
        )
        .unwrap_err();
    assert_eq!(error, Error::Transport);
    assert_eq!(server.finish().len(), 13);
    let diagnostic = executor.last_failure_diagnostic().unwrap();
    assert_eq!(diagnostic.range_index, 4);
    assert_eq!(diagnostic.attempts, 1);
    assert_eq!(diagnostic.automatic_retries, 8);
    assert_eq!(
        counter(&case.checkpoint(), "committed_science_request_count"),
        4
    );
}

/// A pause or cancel during the retry backoff takes effect at once (well within
/// 200 ms of the request) and a later continue fetches only the remaining Ranges.
#[test]
fn science_retry_v2_pause_or_cancel_during_backoff_is_immediate_and_resume_fetches_remaining() {
    for cancel in [false, true] {
        let case = retry_case(&format!("science-retry-v2-backoff-stop-{cancel}"));
        let mut script = vec![case.ok(0), status_only("503 Service Unavailable")];
        script.extend(case.ok_from(1));
        let server = RetryScienceServer::start(script);
        // The real 5 s backoff: only an immediate stop keeps this test fast.
        let executor =
            NisarSubsetScienceRangeExecutorV2::new(science_http(), case.asset(&server)).unwrap();
        let control = WorkerControl::default();
        let stop_control = control.clone();
        let requests = Arc::clone(&server.requests);
        let stopper = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while requests.lock().unwrap().len() < 2 {
                assert!(
                    Instant::now() < deadline,
                    "the failed attempt never arrived"
                );
                thread::sleep(Duration::from_millis(2));
            }
            thread::sleep(Duration::from_millis(300));
            if cancel {
                stop_control.cancel_for_test();
            } else {
                stop_control.set_paused_for_test(true);
            }
            Instant::now()
        });
        let error = executor
            .execute(
                &case.work,
                &case.completed,
                &case.authority(&server),
                &control,
            )
            .unwrap_err();
        let returned = Instant::now();
        let stopped_at = stopper.join().unwrap();
        assert_eq!(
            error,
            if cancel {
                Error::Cancelled
            } else {
                Error::Paused
            }
        );
        assert!(
            returned.duration_since(stopped_at) < Duration::from_millis(200),
            "stop took {:?}",
            returned.duration_since(stopped_at)
        );
        assert_eq!(server.request_count(), 2, "nothing is sent after the stop");
        let checkpoint = case.checkpoint();
        assert!(checkpoint["pending_attempt"].is_null());
        assert_eq!(checkpoint["completed_ranges"].as_array().unwrap().len(), 1);
        if !cancel {
            let resumed = case.executor(&server);
            let result = resumed
                .execute(
                    &case.work,
                    &case.completed,
                    &case.authority(&server),
                    &WorkerControl::default(),
                )
                .unwrap();
            assert!(result.complete());
        }
        let requests = server.finish();
        assert_eq!(
            case.requests_for(&requests, 0),
            1,
            "a completed Range is never refetched"
        );
        if !cancel {
            assert_eq!(requests.len(), case.ranges.len() + 1);
        }
    }
}

/// When the remaining request budget covers only the failed attempt, the retry
/// stops before anything is sent and keeps the last real cause.
#[test]
fn science_retry_v2_stops_before_sending_when_the_budget_cannot_cover_a_retry() {
    let case = retry_case("science-retry-v2-budget-stop");
    // One server keeps the frozen URL identity across both executions.
    let server =
        RetryScienceServer::start(vec![case.ok(0), status_only("503 Service Unavailable")]);
    // First execution: commit Range 0, then pause so a checkpoint exists.
    let control = WorkerControl::default();
    let paused = control.clone();
    let first = case
        .executor(&server)
        .with_after_commit_hook_for_test(Arc::new(move |_| paused.set_paused_for_test(true)));
    assert_eq!(
        first
            .execute(
                &case.work,
                &case.completed,
                &case.authority(&server),
                &control
            )
            .unwrap_err(),
        Error::Paused
    );
    assert_eq!(server.request_count(), 1);
    // Charge already-spent failed requests so exactly one request slot remains.
    let checkpoint = case.checkpoint();
    let budget = case.physical().budget_projection();
    let limit = budget
        .max_science_request_count()
        .min(budget.max_request_count() - counter(&checkpoint, "committed_planning_request_count"));
    let spent = counter(&checkpoint, "reserved_science_request_count");
    let (lineage, record) = case.anchor.state.lock().unwrap().clone().unwrap();
    let advanced = advance_legacy_science_state_v2_for_test(
        case.bootstrap.science_workspace_v2(),
        &record,
        &[LegacyScienceChangeV2ForTest::ChargeFailedRequests(
            limit - spent - 1,
        )],
        false,
    );
    *case.anchor.state.lock().unwrap() = Some((lineage, advanced));
    let executor = case.executor(&server);
    let error = executor
        .execute(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
        )
        .unwrap_err();
    assert_eq!(error, Error::CombinedBudgetExceeded);
    assert_eq!(server.finish().len(), 2, "the retry is never sent");
    let diagnostic = executor.last_failure_diagnostic().unwrap();
    assert!(diagnostic.budget_stopped);
    assert_eq!(diagnostic.http_status, Some(503));
    assert_eq!(diagnostic.range_index, 1);
    assert_eq!(diagnostic.attempts, 1);
}

/// Commits Range 0 in a first execution, then charges already-spent failed
/// requests so exactly `remaining` request slots are left for the next one.
fn leave_request_slots(case: &RetryCase, server: &RetryScienceServer, remaining: u64) {
    let control = WorkerControl::default();
    let paused = control.clone();
    let first = case
        .executor(server)
        .with_after_commit_hook_for_test(Arc::new(move |_| paused.set_paused_for_test(true)));
    assert_eq!(
        first
            .execute(
                &case.work,
                &case.completed,
                &case.authority(server),
                &control
            )
            .unwrap_err(),
        Error::Paused
    );
    let checkpoint = case.checkpoint();
    let budget = case.physical().budget_projection();
    let limit = budget
        .max_science_request_count()
        .min(budget.max_request_count() - counter(&checkpoint, "committed_planning_request_count"));
    let spent = counter(&checkpoint, "reserved_science_request_count");
    let (lineage, record) = case.anchor.state.lock().unwrap().clone().unwrap();
    let advanced = advance_legacy_science_state_v2_for_test(
        case.bootstrap.science_workspace_v2(),
        &record,
        &[LegacyScienceChangeV2ForTest::ChargeFailedRequests(
            limit - spent - remaining,
        )],
        false,
    );
    *case.anchor.state.lock().unwrap() = Some((lineage, advanced));
}

/// Review 2026-10-17 round 2, P1-1 (recover, then exhaust): Range 1 recovers
/// after one 503; Range 2 then cannot reserve its first request. That budget
/// stop has no cause of its own, so no diagnostic of the recovered Range may
/// remain to be reported as "Range 2/N HTTP 503, retry later".
#[test]
fn science_retry_v2_recovered_range_leaves_no_diagnostic_for_a_later_budget_stop() {
    let case = retry_case("science-retry-v2-recover-then-exhaust");
    let server = RetryScienceServer::start(vec![
        case.ok(0),
        status_only("503 Service Unavailable"),
        case.ok(1),
    ]);
    leave_request_slots(&case, &server, 2);
    let executor = case.executor(&server);
    let error = executor
        .execute(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
        )
        .unwrap_err();
    assert_eq!(error, Error::CombinedBudgetExceeded);
    let requests = server.finish();
    assert_eq!(requests.len(), 3, "Range 2 is never sent");
    assert_eq!(case.requests_for(&requests, 1), 2, "Range 1 recovered");
    assert_eq!(
        counter(&case.checkpoint(), "committed_science_request_count"),
        2
    );
    assert_eq!(executor.last_failure_diagnostic(), None);
    assert_eq!(executor.failure_diagnostic_for(error), None);
}

/// Review 2026-10-17 round 2, P1-1 (recover, then a local error): after Range 1
/// recovers, a local Store failure on Range 2's reservation ends the execution.
/// The returned local error keeps no diagnostic of the earlier remote failure.
#[test]
fn science_retry_v2_recovered_range_leaves_no_diagnostic_for_a_later_local_error() {
    let case = retry_case("science-retry-v2-recover-then-local");
    let mut script = vec![case.ok(0), status_only("503 Service Unavailable")];
    script.extend(case.ok_from(1));
    let server = RetryScienceServer::start(script);
    let anchor = Arc::clone(&case.anchor);
    let executor = case
        .executor(&server)
        .with_after_commit_hook_for_test(Arc::new(move |index| {
            if index == 1 {
                anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
            }
        }));
    let error = executor
        .execute(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
        )
        .unwrap_err();
    assert_eq!(error, Error::Persistence);
    let requests = server.finish();
    assert_eq!(requests.len(), 3, "Range 2 is never sent");
    assert_eq!(case.requests_for(&requests, 1), 2, "Range 1 recovered");
    assert_eq!(executor.last_failure_diagnostic(), None);
    assert_eq!(executor.failure_diagnostic_for(error), None);
}

/// Review 2026-10-17 round 2, P1-1 (same Range, other class): a transient 503
/// is due for a retry, but the retry's durable reservation fails locally. The
/// local error is reported as itself, never with the 503's retryable summary.
#[test]
fn science_retry_v2_transient_cause_is_not_reported_for_a_local_error_of_the_same_range() {
    let case = retry_case("science-retry-v2-transient-then-local");
    let server =
        RetryScienceServer::start(vec![case.ok(0), status_only("503 Service Unavailable")]);
    let anchor = Arc::clone(&case.anchor);
    let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), case.asset(&server))
        .unwrap()
        .with_retry_backoff_for_test(Arc::new(move |_, _| {
            anchor.fail_after_pending_cas(ScienceAnchorPendingFailpoint::Reservation);
            Duration::ZERO
        }));
    let error = executor
        .execute(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
        )
        .unwrap_err();
    assert_eq!(error, Error::Persistence);
    assert_eq!(server.finish().len(), 2, "the retry is never sent");
    assert_eq!(executor.last_failure_diagnostic(), None);
    assert_eq!(executor.failure_diagnostic_for(error), None);
    assert_eq!(executor.failure_diagnostic_for(Error::Transport), None);
}

/// The diagnostic of a failure that did end the execution is reported only for
/// that exact error.
#[test]
fn science_retry_v2_diagnostic_is_bound_to_the_error_it_describes() {
    let case = retry_case("science-retry-v2-diagnostic-binding");
    let server = RetryScienceServer::start(vec![
        status_only("503 Service Unavailable"),
        status_only("503 Service Unavailable"),
        status_only("503 Service Unavailable"),
    ]);
    let executor = case.executor(&server);
    let error = executor
        .execute(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
        )
        .unwrap_err();
    assert_eq!(error, Error::Transport);
    server.finish();
    let diagnostic = executor.failure_diagnostic_for(Error::Transport).unwrap();
    assert_eq!((diagnostic.range_index, diagnostic.attempts), (0, 3));
    for other in [
        Error::Persistence,
        Error::RecoveryInvalid,
        Error::CombinedBudgetExceeded,
    ] {
        assert_eq!(executor.failure_diagnostic_for(other), None, "{other:?}");
    }
}

/// Review 2026-10-17 round 2, P2-2: the end of the science transfer is
/// reported right after the last Range commit, before the final verification,
/// so the task card shows the output phase at once instead of after that
/// verification. A continue whose science is already complete reports only
/// the end of the transfer, without any byte.
#[test]
fn science_transfer_v2_reports_transfer_complete_before_the_final_verification() {
    let case = retry_case("science-transfer-complete-event");
    let server = RetryScienceServer::start(case.ok_from(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let at_final = Arc::new(Mutex::new(None));
    let (hook_events, hook_at_final) = (Arc::clone(&events), Arc::clone(&at_final));
    let executor = case
        .executor(&server)
        .with_before_final_revalidation_hook_for_test(Arc::new(move || {
            *hook_at_final.lock().unwrap() = Some(hook_events.lock().unwrap().clone());
        }));
    let result = executor
        .execute_with_progress(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
            &mut |event| events.lock().unwrap().push(event),
        )
        .unwrap();
    assert!(result.complete());
    let at_final: Vec<Progress> = at_final.lock().unwrap().clone().unwrap();
    assert_eq!(at_final.last(), Some(&Progress::TransferComplete));
    assert_eq!(
        at_final
            .iter()
            .filter(|event| **event == Progress::TransferComplete)
            .count(),
        1
    );
    let bytes: u64 = at_final
        .iter()
        .map(|event| match event {
            Progress::Bytes(bytes) => *bytes,
            Progress::TransferComplete => 0,
        })
        .sum();
    let total: u64 = case.ranges.iter().map(|range| range.len().unwrap()).sum();
    assert_eq!(bytes, total);
    assert_eq!(
        *events.lock().unwrap(),
        at_final,
        "nothing is reported after the final verification starts"
    );
    let mut resumed = Vec::new();
    let result = case
        .executor(&server)
        .execute_with_progress(
            &case.work,
            &case.completed,
            &case.authority(&server),
            &WorkerControl::default(),
            &mut |event| resumed.push(event),
        )
        .unwrap();
    assert!(result.complete());
    assert_eq!(resumed, vec![Progress::TransferComplete]);
    assert_eq!(server.finish().len(), case.ranges.len());
}

/// Review 2026-10-17 round 2, P2-4: with the real backoff the first automatic
/// retry of a Range waits 5 s and the second 20 s; an HTTP 429 waits at least
/// 30 s for both. The waits are observed through an injected wait, so the
/// test never sleeps.
#[test]
fn science_retry_v2_backoff_waits_5_then_20_seconds_and_429_at_least_30() {
    for (label, status, expected) in [
        (
            "science-retry-v2-backoff-503",
            "503 Service Unavailable",
            [5, 20],
        ),
        (
            "science-retry-v2-backoff-429",
            "429 Too Many Requests",
            [30, 30],
        ),
    ] {
        let case = retry_case(label);
        let mut script = vec![status_only(status), status_only(status)];
        script.extend(case.ok_from(0));
        let server = RetryScienceServer::start(script);
        let waits = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&waits);
        let executor = NisarSubsetScienceRangeExecutorV2::new(science_http(), case.asset(&server))
            .unwrap()
            .with_retry_wait_for_test(Arc::new(move |delay, _| {
                recorded.lock().unwrap().push(delay);
                false
            }));
        // The injected wait replaces the interruptible sleep entirely.
        let result = executor
            .execute(
                &case.work,
                &case.completed,
                &case.authority(&server),
                &WorkerControl::default(),
            )
            .unwrap();
        assert!(result.complete(), "{label}");
        assert_eq!(
            *waits.lock().unwrap(),
            expected.map(Duration::from_secs).to_vec(),
            "{label}"
        );
        assert_eq!(case.requests_for(&server.finish(), 0), 3, "{label}");
    }
}

/// Sleeping advances virtual time at once; the hook runs after every sleep.
struct VirtualRetryClock {
    start: Instant,
    elapsed: Mutex<Duration>,
    after_sleep: Box<dyn Fn(Duration) + Send + Sync>,
}

impl VirtualRetryClock {
    fn new(after_sleep: impl Fn(Duration) + Send + Sync + 'static) -> Self {
        Self {
            start: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
            after_sleep: Box::new(after_sleep),
        }
    }

    fn elapsed(&self) -> Duration {
        *self.elapsed.lock().unwrap()
    }
}

impl ScienceRetryClock for VirtualRetryClock {
    fn now(&self) -> Instant {
        self.start + self.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        let elapsed = {
            let mut elapsed = self.elapsed.lock().unwrap();
            *elapsed += duration;
            *elapsed
        };
        (self.after_sleep)(elapsed);
    }
}

/// Review 2026-10-17 round 2, P2-4: the interruptible wait spends the full
/// backoff on its clock (20 s; 30 s for a 429) and ends at once when a pause
/// arrives, measured on an injected clock without sleeping.
#[test]
fn science_retry_v2_backoff_wait_spends_the_full_delay_and_stops_on_pause() {
    for seconds in [20, 30] {
        let clock = VirtualRetryClock::new(|_| {});
        let real = Instant::now();
        assert!(!wait_for_science_retry_with_clock(
            Duration::from_secs(seconds),
            &WorkerControl::default(),
            &clock
        ));
        assert_eq!(clock.elapsed(), Duration::from_secs(seconds));
        assert!(real.elapsed() < Duration::from_secs(1));
    }
    let control = WorkerControl::default();
    let pausing = control.clone();
    let clock = VirtualRetryClock::new(move |elapsed| {
        if elapsed >= Duration::from_secs(10) {
            pausing.set_paused_for_test(true);
        }
    });
    assert!(wait_for_science_retry_with_clock(
        Duration::from_secs(20),
        &control,
        &clock
    ));
    let elapsed = clock.elapsed();
    assert!(
        elapsed >= Duration::from_secs(10)
            && elapsed <= Duration::from_secs(10) + Duration::from_millis(20),
        "{elapsed:?}"
    );
}
