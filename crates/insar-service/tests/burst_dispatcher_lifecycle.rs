use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use insar_core::{
    AsfRemoteSearchRequest, AsfRemoteSearchResponse, ContractError, ProviderResult,
    SarRemoteSearchRequest, SarRemoteSearchResponse, SarSearchCountRequest, SarSearchCountResponse,
};
use insar_provider::asf::search::{SarSearchProviderProgress, SarSearchProviderProgressStage};
use insar_service::{Dispatcher, LiveAsfSearch};
use serde_json::json;

#[derive(Clone, Copy)]
enum Outcome {
    Success,
    Error,
}

struct ScriptedSearch {
    search_outcomes: Mutex<VecDeque<Outcome>>,
    count_outcomes: Mutex<VecDeque<Outcome>>,
    progress: Vec<SarSearchProviderProgress>,
    search_calls: AtomicUsize,
    count_calls: AtomicUsize,
}

impl ScriptedSearch {
    fn new(
        search_outcomes: impl IntoIterator<Item = Outcome>,
        count_outcomes: impl IntoIterator<Item = Outcome>,
        progress: Vec<SarSearchProviderProgress>,
    ) -> Self {
        Self {
            search_outcomes: Mutex::new(search_outcomes.into_iter().collect()),
            count_outcomes: Mutex::new(count_outcomes.into_iter().collect()),
            progress,
            search_calls: AtomicUsize::new(0),
            count_calls: AtomicUsize::new(0),
        }
    }
}

impl LiveAsfSearch for ScriptedSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        unreachable!("dispatcher lifecycle fixture does not serve legacy ASF search")
    }

    fn search_sar_with_progress(
        &self,
        _request: &SarRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
        progress: &dyn Fn(SarSearchProviderProgress),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.search_calls.fetch_add(1, Ordering::SeqCst);
        for event in &self.progress {
            progress(*event);
        }
        match self
            .search_outcomes
            .lock()
            .expect("search outcome lock")
            .pop_front()
            .expect("scripted search outcome")
        {
            Outcome::Success => Ok(empty_search_response()),
            Outcome::Error => Err(fixture_error("FIXTURE_SEARCH_ERROR")),
        }
    }

    fn count_sar(
        &self,
        _request: &SarSearchCountRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarSearchCountResponse> {
        self.count_calls.fetch_add(1, Ordering::SeqCst);
        match self
            .count_outcomes
            .lock()
            .expect("count outcome lock")
            .pop_front()
            .expect("scripted count outcome")
        {
            Outcome::Success => Ok(SarSearchCountResponse {
                total_count: Some(17),
            }),
            Outcome::Error => Err(fixture_error("FIXTURE_COUNT_ERROR")),
        }
    }
}

struct BlockingSearch {
    entered: Mutex<mpsc::Sender<String>>,
    released: (Mutex<HashSet<String>>, Condvar),
    calls: AtomicUsize,
}

impl BlockingSearch {
    fn new(entered: mpsc::Sender<String>) -> Self {
        Self {
            entered: Mutex::new(entered),
            released: (Mutex::new(HashSet::new()), Condvar::new()),
            calls: AtomicUsize::new(0),
        }
    }

    fn release(&self, dataset: &str) {
        self.released
            .0
            .lock()
            .expect("release lock")
            .insert(dataset.to_owned());
        self.released.1.notify_all();
    }
}

impl LiveAsfSearch for BlockingSearch {
    fn search(
        &self,
        _request: &AsfRemoteSearchRequest,
        _cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<AsfRemoteSearchResponse> {
        unreachable!("dispatcher cancellation fixture does not serve legacy ASF search")
    }

    fn search_sar(
        &self,
        request: &SarRemoteSearchRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ProviderResult<SarRemoteSearchResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let dataset = request.query.dataset.clone();
        self.entered
            .lock()
            .expect("entered sender lock")
            .send(dataset.clone())
            .expect("test owns entered receiver");

        let (released, signal) = &self.released;
        let mut released = released.lock().expect("release lock");
        while !released.contains(&dataset) {
            let (next, timeout) = signal
                .wait_timeout(released, Duration::from_secs(5))
                .expect("release wait");
            released = next;
            assert!(
                !timeout.timed_out(),
                "fixture release timed out for {dataset}"
            );
        }
        drop(released);

        if cancelled() {
            Err(fixture_error("FIXTURE_SEARCH_CANCELLED"))
        } else {
            Ok(empty_search_response())
        }
    }
}

fn sar_request(dataset: &str) -> SarRemoteSearchRequest {
    serde_json::from_value(json!({"query": {"dataset": dataset}}))
        .expect("minimal SAR search request")
}

fn count_request(dataset: &str) -> SarSearchCountRequest {
    serde_json::from_value(json!({"query": {"dataset": dataset}}))
        .expect("minimal SAR count request")
}

fn empty_search_response() -> SarRemoteSearchResponse {
    serde_json::from_value(json!({
        "products": [],
        "errors": [],
        "stats": {
            "requested_limit": 1,
            "query_limit": 1,
            "candidate_count": 0,
            "returned_count": 0,
            "source": "FIXTURE"
        }
    }))
    .expect("valid empty SAR response")
}

fn fixture_error(code: &str) -> ContractError {
    ContractError::new(code, "fixture error", Default::default()).expect("valid fixture error")
}

#[test]
fn caller_request_id_is_released_after_search_success_and_provider_error() {
    let provider = Arc::new(ScriptedSearch::new(
        [Outcome::Success, Outcome::Error, Outcome::Success],
        [],
        Vec::new(),
    ));
    let dispatcher = Dispatcher::with_asf_search_for_application(provider.clone());
    let request = sar_request("SLC-BURST");

    dispatcher
        .search_sar_for_application("same-search-id", &request)
        .expect("first search succeeds");
    let error = dispatcher
        .search_sar_for_application("same-search-id", &request)
        .expect_err("second provider result is the scripted error");
    assert_eq!(error.code, "FIXTURE_SEARCH_ERROR");
    dispatcher
        .search_sar_for_application("same-search-id", &request)
        .expect("the provider error also releases the caller ID");

    assert_eq!(provider.search_calls.load(Ordering::SeqCst), 3);
    assert!(!dispatcher.cancel_request("same-search-id"));
}

#[test]
fn caller_request_id_is_released_after_count_success_and_provider_error() {
    let provider = Arc::new(ScriptedSearch::new(
        [],
        [Outcome::Success, Outcome::Error, Outcome::Success],
        Vec::new(),
    ));
    let dispatcher = Dispatcher::with_asf_search_for_application(provider.clone());
    let request = count_request("SLC-BURST");

    assert_eq!(
        dispatcher
            .count_sar_for_application("same-count-id", &request)
            .expect("first count succeeds")
            .total_count,
        Some(17)
    );
    let error = dispatcher
        .count_sar_for_application("same-count-id", &request)
        .expect_err("second provider result is the scripted error");
    assert_eq!(error.code, "FIXTURE_COUNT_ERROR");
    dispatcher
        .count_sar_for_application("same-count-id", &request)
        .expect("the provider error also releases the caller ID");

    assert_eq!(provider.count_calls.load(Ordering::SeqCst), 3);
    assert!(!dispatcher.cancel_request("same-count-id"));
}

#[test]
fn duplicate_never_replaces_original_flag_and_cancel_targets_only_exact_search_id() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let provider = Arc::new(BlockingSearch::new(entered_tx));
    let dispatcher = Arc::new(Dispatcher::with_asf_search_for_application(
        provider.clone(),
    ));

    let target_dispatcher = dispatcher.clone();
    let target = thread::spawn(move || {
        target_dispatcher.search_sar_for_application("target-request-id", &sar_request("SLC-BURST"))
    });
    assert_eq!(
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("target provider entry"),
        "SLC-BURST"
    );

    let peer_dispatcher = dispatcher.clone();
    let peer = thread::spawn(move || {
        peer_dispatcher.search_sar_for_application("peer-request-id", &sar_request("ERS"))
    });
    assert_eq!(
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("peer provider entry"),
        "ERS"
    );

    let duplicate = dispatcher
        .search_sar_for_application("target-request-id", &sar_request("SLC-BURST"))
        .expect_err("active duplicate must be rejected before provider dispatch");
    assert_eq!(duplicate.code, "ASF_REQUEST_DUPLICATE");

    assert!(dispatcher.cancel_request("target-request-id"));
    provider.release("SLC-BURST");
    let cancelled = target
        .join()
        .expect("target thread joins")
        .expect_err("original request retains the flag cancelled by exact ID");
    assert_eq!(cancelled.code, "FIXTURE_SEARCH_CANCELLED");

    provider.release("ERS");
    peer.join()
        .expect("peer thread joins")
        .expect("cancelling target ID must not cancel peer ID");

    dispatcher
        .search_sar_for_application("target-request-id", &sar_request("SLC-BURST"))
        .expect("cancelled request releases its caller ID for reuse");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 3);
    assert!(!dispatcher.cancel_request("target-request-id"));
    assert!(!dispatcher.cancel_request("peer-request-id"));
}

#[test]
fn dispatcher_forwards_provider_progress_without_rewriting_any_field() {
    let expected = vec![
        SarSearchProviderProgress {
            stage: SarSearchProviderProgressStage::ProviderResponse,
            provider_request_count: 3,
            transport_part_count: 2,
            raw_fetched_count: 41,
            mapped_count: 37,
            unique_admitted_count: 29,
            requested_limit: Some(50),
            remote_total: Some(144),
            fetched_count: 29,
            total_count: Some(50),
        },
        SarSearchProviderProgress {
            stage: SarSearchProviderProgressStage::AdmittingResults,
            provider_request_count: 4,
            transport_part_count: 2,
            raw_fetched_count: 67,
            mapped_count: 61,
            unique_admitted_count: 50,
            requested_limit: Some(50),
            remote_total: Some(144),
            fetched_count: 50,
            total_count: Some(50),
        },
    ];
    let provider = Arc::new(ScriptedSearch::new(
        [Outcome::Success],
        [],
        expected.clone(),
    ));
    let dispatcher = Dispatcher::with_asf_search_for_application(provider);
    let observed = Mutex::new(Vec::new());

    dispatcher
        .search_sar_for_application_with_progress(
            "progress-request-id",
            &sar_request("SLC-BURST"),
            &|event| observed.lock().expect("progress lock").push(event),
        )
        .expect("fixture search succeeds");

    assert_eq!(*observed.lock().expect("progress lock"), expected);
}
