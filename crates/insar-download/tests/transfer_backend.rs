use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use insar_core::{DownloadFailure, DownloadOutcome};
use insar_download::{
    AdmittedTransferBackend, AuthRefreshPolicyId, AuthRejection, ResolvedAsset, ResolvedMethod,
    ResolvedTransferResult, ResolvedTransferSession, ResponseHostClass,
    TransferAdmissionController, TransferAuth, TransferBackend, TransferBackendKind,
    TransferBackendSelector, TransferProgress, TransferRequest, TransferResult, TransferUpdate,
};

struct BackendStats {
    legacy_calls: AtomicUsize,
    resolved_calls: AtomicUsize,
    cancel_probes: AtomicUsize,
    pause_probes: AtomicUsize,
    progress_callbacks: AtomicUsize,
    sessions: Mutex<Vec<usize>>,
}

impl Default for BackendStats {
    fn default() -> Self {
        Self {
            legacy_calls: AtomicUsize::new(0),
            resolved_calls: AtomicUsize::new(0),
            cancel_probes: AtomicUsize::new(0),
            pause_probes: AtomicUsize::new(0),
            progress_callbacks: AtomicUsize::new(0),
            sessions: Mutex::new(Vec::new()),
        }
    }
}

struct RecordingBackend {
    available: bool,
    legacy_results: Mutex<VecDeque<TransferResult>>,
    resolved_results: Mutex<VecDeque<ResolvedTransferResult>>,
    stats: BackendStats,
}

impl RecordingBackend {
    fn new(
        available: bool,
        legacy_results: Vec<TransferResult>,
        resolved_results: Vec<ResolvedTransferResult>,
    ) -> Self {
        Self {
            available,
            legacy_results: Mutex::new(legacy_results.into()),
            resolved_results: Mutex::new(resolved_results.into()),
            stats: BackendStats::default(),
        }
    }

    fn completed() -> TransferResult {
        TransferResult {
            outcome: DownloadOutcome::Completed,
            bytes_written: 5,
            attempt_count: 1,
            failure: None,
        }
    }

    fn failed() -> TransferResult {
        TransferResult {
            outcome: DownloadOutcome::Failed,
            bytes_written: 2,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "fixture backend failure".to_string(),
                retryable: true,
            }),
        }
    }

    fn invoke_callbacks(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) {
        self.stats.cancel_probes.fetch_add(1, Ordering::SeqCst);
        let _ = cancelled();
        self.stats.pause_probes.fetch_add(1, Ordering::SeqCst);
        let _ = paused();
        self.stats.progress_callbacks.fetch_add(1, Ordering::SeqCst);
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
    }
}

impl TransferBackend for RecordingBackend {
    fn kind(&self) -> TransferBackendKind {
        TransferBackendKind::NativeSingleStream
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn download_legacy(
        &self,
        _request: &TransferRequest,
        _auth: &TransferAuth,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        self.stats.legacy_calls.fetch_add(1, Ordering::SeqCst);
        self.invoke_callbacks(cancelled, paused, progress);
        self.legacy_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(Self::completed)
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).unwrap()
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        self.stats.resolved_calls.fetch_add(1, Ordering::SeqCst);
        self.stats
            .sessions
            .lock()
            .unwrap()
            .push(session as *mut ResolvedTransferSession as usize);
        self.invoke_callbacks(cancelled, paused, progress);
        self.resolved_results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| ResolvedTransferResult::Finished(Self::completed()))
    }
}

fn request() -> TransferRequest {
    TransferRequest::new(
        "backend-task",
        "backend-item",
        "https://example.invalid/product.bin",
        PathBuf::from("backend-output.bin"),
        Some(5),
    )
    .unwrap()
}

fn asset() -> ResolvedAsset {
    ResolvedAsset::new("https://example.invalid/product.bin", ResolvedMethod::Get).unwrap()
}

#[test]
fn unavailable_preferred_backend_selects_native_without_invocation() {
    let native = Arc::new(RecordingBackend::new(
        true,
        vec![RecordingBackend::completed()],
        vec![],
    ));
    let unavailable = Arc::new(RecordingBackend::new(false, vec![], vec![]));
    let selector =
        TransferBackendSelector::with_preferred_for_test(native.clone(), unavailable.clone());
    let cancel_queries = AtomicUsize::new(0);
    let pause_queries = AtomicUsize::new(0);
    let progress_callbacks = AtomicUsize::new(0);

    let result = selector.select().download_legacy(
        &request(),
        &TransferAuth::None,
        &|| {
            cancel_queries.fetch_add(1, Ordering::SeqCst);
            false
        },
        &|| {
            pause_queries.fetch_add(1, Ordering::SeqCst);
            false
        },
        &mut |update| {
            if matches!(update, TransferUpdate::Progress(_)) {
                progress_callbacks.fetch_add(1, Ordering::SeqCst);
            }
        },
    );

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(native.stats.legacy_calls.load(Ordering::SeqCst), 1);
    assert_eq!(unavailable.stats.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(cancel_queries.load(Ordering::SeqCst), 1);
    assert_eq!(pause_queries.load(Ordering::SeqCst), 1);
    assert_eq!(progress_callbacks.load(Ordering::SeqCst), 1);
}

#[test]
fn selected_backend_result_is_not_replayed_through_native() {
    let native = Arc::new(RecordingBackend::new(true, vec![], vec![]));
    let preferred = Arc::new(RecordingBackend::new(
        true,
        vec![RecordingBackend::failed()],
        vec![],
    ));
    let selector =
        TransferBackendSelector::with_preferred_for_test(native.clone(), preferred.clone());

    let result = selector.select().download_legacy(
        &request(),
        &TransferAuth::None,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(preferred.stats.legacy_calls.load(Ordering::SeqCst), 1);
    assert_eq!(native.stats.legacy_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn admitted_backend_forwards_one_selected_result_without_replay() {
    let controller = TransferAdmissionController::compatibility();
    let selected = RecordingBackend::new(true, vec![RecordingBackend::failed()], vec![]);
    let admitted = AdmittedTransferBackend::new(&selected, controller.clone());

    let result = admitted.download_legacy(
        &request(),
        &TransferAuth::None,
        &|| false,
        &|| false,
        &mut |_| {},
    );

    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(selected.stats.legacy_calls.load(Ordering::SeqCst), 1);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.grants, 1);
    assert_eq!(snapshot.releases, 1);
    assert_eq!(snapshot.active_global, 0);
    assert_eq!(snapshot.queued_global, 0);
}

#[test]
fn selected_resolver_backend_reuses_one_session_and_forwards_callbacks_once_per_attempt() {
    let native = Arc::new(RecordingBackend::new(true, vec![], vec![]));
    let preferred = Arc::new(RecordingBackend::new(
        true,
        vec![],
        vec![
            ResolvedTransferResult::AuthRejected(AuthRejection {
                status_code: 401,
                response_host_class: ResponseHostClass::Approved,
                refresh_candidate: true,
            }),
            ResolvedTransferResult::Finished(RecordingBackend::completed()),
        ],
    ));
    let selector =
        TransferBackendSelector::with_preferred_for_test(native.clone(), preferred.clone());
    let backend = selector.select();
    let mut session = backend.resolved_session();
    let callback_count = AtomicUsize::new(0);

    let first = backend.download_resolved(
        &request(),
        &asset(),
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |update| {
            if matches!(update, TransferUpdate::Progress(_)) {
                callback_count.fetch_add(1, Ordering::SeqCst);
            }
        },
    );
    let second = backend.download_resolved(
        &request(),
        &asset(),
        AuthRefreshPolicyId::EarthdataStandardV1,
        &mut session,
        &|| false,
        &|| false,
        &mut |update| {
            if matches!(update, TransferUpdate::Progress(_)) {
                callback_count.fetch_add(1, Ordering::SeqCst);
            }
        },
    );

    assert!(matches!(first, ResolvedTransferResult::AuthRejected(_)));
    assert!(matches!(
        second,
        ResolvedTransferResult::Finished(ref result) if result.outcome == DownloadOutcome::Completed
    ));
    assert_eq!(preferred.stats.resolved_calls.load(Ordering::SeqCst), 2);
    assert_eq!(native.stats.resolved_calls.load(Ordering::SeqCst), 0);
    assert_eq!(callback_count.load(Ordering::SeqCst), 2);
    let sessions = preferred.stats.sessions.lock().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0], sessions[1]);
}
