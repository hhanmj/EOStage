use std::collections::VecDeque;
use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime};

use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, DownloadFailure, DownloadOutcome, FrozenTaskSnapshot,
    SnapshotItem,
};
use insar_download::{
    classify_frozen_execution, AsfCollectionProfileRegistry, AssetExecutionPolicyRegistry,
    AssetResolver, AssetResolverRegistry, AuthRefreshPolicyId, AuthRejection, Clock,
    CredentialMaterial, CredentialProvider, CredentialProviderError, DestinationLeaseController,
    DownloadWork, DownloadWorker, FrozenExecution, ProductionDownloadWorker, ResolveContext,
    ResolveError, ResolvedAsset, ResolvedMethod, ResolvedTransferResult, ResolvedTransferSession,
    ResolverExecutionRuntime, ResponseHostClass, ShadowAccessIntent, ShadowResolveContext,
    TransferAdmissionController, TransferAuth, TransferBackend, TransferBackendKind,
    TransferOptions, TransferProgress, TransferRequest, TransferResult, TransferUpdate,
    WorkerControl,
};
use serde_json::{json, Value};

fn base_item_value() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/asset_execution_policy/sentinel1_standard_cases.json");
    serde_json::from_str::<Value>(&fs::read_to_string(path).unwrap()).unwrap()["base_item"].clone()
}

fn item_with_policy(policy: Value) -> SnapshotItem {
    let mut value = base_item_value();
    value["execution_policy"] = policy;
    serde_json::from_value(value).unwrap()
}

struct FixedClock;

struct TempOutputRoot(PathBuf);

impl TempOutputRoot {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "insar-resolver-destination-lease-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn as_string(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempOutputRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

struct CountingCredentials {
    calls: AtomicUsize,
}

impl CountingCredentials {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl CredentialProvider for CountingCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "earthdata-default");
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        CredentialMaterial::bearer(format!("FIXTURE_TOKEN_{call}"))
    }
}

struct FailingCredentials {
    calls: AtomicUsize,
    fail_on: usize,
}

impl FailingCredentials {
    fn new(fail_on: usize) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail_on,
        }
    }
}

impl CredentialProvider for FailingCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "earthdata-default");
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.fail_on {
            Err(CredentialProviderError::Failed)
        } else {
            CredentialMaterial::bearer(format!("FIXTURE_TOKEN_{call}"))
        }
    }
}

struct RefreshGate {
    state: Mutex<RefreshGateState>,
    changed: Condvar,
}

struct RefreshGateState {
    entered: bool,
    released: bool,
}

impl RefreshGate {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(RefreshGateState {
                entered: false,
                released: false,
            }),
            changed: Condvar::new(),
        })
    }

    fn block_refresh(&self) {
        let mut state = self.state.lock().unwrap();
        state.entered = true;
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_for_entered(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while !state.entered {
            if Instant::now() >= deadline {
                return false;
            }
            let timeout = deadline.saturating_duration_since(Instant::now());
            let (next, _) = self.changed.wait_timeout(state, timeout).unwrap();
            state = next;
        }
        true
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.released = true;
        drop(state);
        self.changed.notify_all();
    }
}

struct SequenceResolver {
    calls: AtomicUsize,
    drift_on_second: bool,
    cancel_after_second: Option<Arc<AtomicBool>>,
    fail_on_first: bool,
    refresh_gate: Option<Arc<RefreshGate>>,
}

impl SequenceResolver {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            drift_on_second: false,
            cancel_after_second: None,
            fail_on_first: false,
            refresh_gate: None,
        }
    }

    fn with_refresh_gate(gate: Arc<RefreshGate>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            drift_on_second: false,
            cancel_after_second: None,
            fail_on_first: false,
            refresh_gate: Some(gate),
        }
    }

    fn with_second_size_drift() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            drift_on_second: true,
            cancel_after_second: None,
            fail_on_first: false,
            refresh_gate: None,
        }
    }

    fn with_cancel_after_second(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            drift_on_second: false,
            cancel_after_second: Some(cancelled),
            fail_on_first: false,
            refresh_gate: None,
        }
    }

    fn failing() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            drift_on_second: false,
            cancel_after_second: None,
            fail_on_first: true,
            refresh_gate: None,
        }
    }
}

impl AssetResolver for SequenceResolver {
    fn resolver_id(&self) -> &'static str {
        "earthdata_https"
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::EarthdataHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        descriptor.scheme == AssetAccessScheme::EarthdataHttps
    }

    fn resolve(
        &self,
        _reference: &insar_core::StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        context.check_cancelled()?;
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_on_first && call == 1 {
            return Err(ResolveError::failed());
        }
        if call == 2 {
            if let Some(gate) = &self.refresh_gate {
                gate.block_refresh();
            }
        }
        let profile = descriptor
            .credential_profile
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let credential = context
            .credential_provider()
            .resolve(profile)
            .map_err(|_| ResolveError::credentials_missing())?;
        let mut asset = ResolvedAsset::new(
            &format!("https://datapool.asf.alaska.edu/SLC/runtime-{call}/fixture.zip"),
            ResolvedMethod::Get,
        )?
        .with_suggested_filename(Some("fixture.zip".to_string()));
        if self.drift_on_second && call == 2 {
            asset = asset.with_expected_size(Some(2048));
        }
        if call == 2 {
            if let Some(cancelled) = &self.cancel_after_second {
                cancelled.store(true, Ordering::SeqCst);
            }
        }
        credential.apply_to(&mut asset)?;
        Ok(asset)
    }

    fn plan_shadow(
        &self,
        _frozen: &insar_core::FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::failed())
    }
}

struct FakeTransfer {
    available: bool,
    outcomes: Mutex<VecDeque<ResolvedTransferResult>>,
    calls: AtomicUsize,
    legacy_calls: AtomicUsize,
    cancel_probes: AtomicUsize,
    pause_probes: AtomicUsize,
    progress_callbacks: AtomicUsize,
    sessions: Mutex<Vec<usize>>,
    urls: Mutex<Vec<String>>,
    cancel_after_first: Option<Arc<AtomicBool>>,
}

impl FakeTransfer {
    fn new(outcomes: Vec<ResolvedTransferResult>) -> Self {
        Self {
            available: true,
            outcomes: Mutex::new(outcomes.into()),
            calls: AtomicUsize::new(0),
            legacy_calls: AtomicUsize::new(0),
            cancel_probes: AtomicUsize::new(0),
            pause_probes: AtomicUsize::new(0),
            progress_callbacks: AtomicUsize::new(0),
            sessions: Mutex::new(Vec::new()),
            urls: Mutex::new(Vec::new()),
            cancel_after_first: None,
        }
    }

    fn with_cancel_after_first(mut self, cancelled: Arc<AtomicBool>) -> Self {
        self.cancel_after_first = Some(cancelled);
        self
    }

    fn unavailable() -> Self {
        let mut transfer = Self::new(vec![]);
        transfer.available = false;
        transfer
    }
}

impl TransferBackend for FakeTransfer {
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
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        self.legacy_calls.fetch_add(1, Ordering::SeqCst);
        TransferResult {
            outcome: DownloadOutcome::Failed,
            bytes_written: 0,
            attempt_count: 0,
            failure: Some(DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "fixture Legacy route must not run".to_string(),
                retryable: false,
            }),
        }
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).unwrap()
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.cancel_probes.fetch_add(1, Ordering::SeqCst);
        let _ = cancelled();
        self.pause_probes.fetch_add(1, Ordering::SeqCst);
        let _ = paused();
        self.progress_callbacks.fetch_add(1, Ordering::SeqCst);
        let attempt = call as u32;
        progress(TransferUpdate::BodyStarted {
            attempt,
            bytes_completed: 0,
        });
        progress(TransferUpdate::Progress(TransferProgress {
            bytes_completed: 1024,
            total_bytes: Some(1024),
            attempt,
        }));
        progress(TransferUpdate::BodyStopped { attempt });
        self.sessions
            .lock()
            .unwrap()
            .push(session as *mut ResolvedTransferSession as usize);
        self.urls
            .lock()
            .unwrap()
            .push(asset.effective_url().to_string());
        if call == 1 {
            if let Some(cancelled) = &self.cancel_after_first {
                cancelled.store(true, Ordering::SeqCst);
            }
        }
        self.outcomes.lock().unwrap().pop_front().unwrap()
    }
}

fn finished() -> ResolvedTransferResult {
    ResolvedTransferResult::Finished(TransferResult {
        outcome: DownloadOutcome::Completed,
        bytes_written: 1024,
        attempt_count: 2,
        failure: None,
    })
}

fn auth_rejected(status_code: u16, refresh_candidate: bool) -> ResolvedTransferResult {
    ResolvedTransferResult::AuthRejected(AuthRejection {
        status_code,
        response_host_class: ResponseHostClass::Approved,
        refresh_candidate,
    })
}

fn fixture_runtime() -> (
    ResolverExecutionRuntime,
    Arc<SequenceResolver>,
    Arc<CountingCredentials>,
) {
    let resolver = Arc::new(SequenceResolver::new());
    let credentials = Arc::new(CountingCredentials::new());
    let mut registry = AssetResolverRegistry::new();
    registry.register(resolver.clone()).unwrap();
    let runtime = ResolverExecutionRuntime::with_dependencies(
        registry,
        credentials.clone(),
        Arc::new(FixedClock),
    );
    (runtime, resolver, credentials)
}

fn runtime_with_resolver(
    resolver: Arc<SequenceResolver>,
) -> (ResolverExecutionRuntime, Arc<CountingCredentials>) {
    let credentials = Arc::new(CountingCredentials::new());
    let mut registry = AssetResolverRegistry::new();
    registry.register(resolver).unwrap();
    (
        ResolverExecutionRuntime::with_dependencies(
            registry,
            credentials.clone(),
            Arc::new(FixedClock),
        ),
        credentials,
    )
}

fn runtime_with_credentials(credentials: Arc<dyn CredentialProvider>) -> ResolverExecutionRuntime {
    ResolverExecutionRuntime::with_dependencies(
        AssetResolverRegistry::with_standard_resolvers().unwrap(),
        credentials,
        Arc::new(FixedClock),
    )
}

fn eligible() -> Box<insar_download::EligibleResolverAsset> {
    let item: SnapshotItem = serde_json::from_value(base_item_value()).unwrap();
    let FrozenExecution::ResolverAuthoritative(eligible) = classify_frozen_execution(
        &item,
        &AsfCollectionProfileRegistry,
        &AssetExecutionPolicyRegistry,
    ) else {
        panic!("fixture must be Resolver eligible");
    };
    eligible
}

fn resolver_work(output_root: &str) -> DownloadWork {
    let mut item_value = base_item_value();
    item_value["payload"] = json!({
        "download_url": "http://127.0.0.1:9/legacy-must-not-run",
        "file_name": "legacy.zip",
        "file_size_remote": 1024
    });
    let item: SnapshotItem = serde_json::from_value(item_value).unwrap();
    let task: FrozenTaskSnapshot = serde_json::from_value(json!({
        "schema_version": 1,
        "task_id": "resolver-production-task",
        "batch_id": "resolver-production-batch",
        "provider_id": "asf.catalog",
        "created_at": "2026-07-18T00:00:00Z",
        "aoi_name": "AOI",
        "aoi_geojson": {"type": "Polygon", "coordinates": []},
        "query": {"dataset": "SENTINEL-1"},
        "candidates": [item.clone()],
        "initial_selected_item_ids": [item.item_id.clone()],
        "output_root": output_root,
        "layout": "direct"
    }))
    .unwrap();
    DownloadWork { task, item }
}

fn distinct_resolver_work(output_root: &str, task_id: &str) -> DownloadWork {
    let mut work = resolver_work(output_root);
    work.task.task_id = task_id.to_string();
    work.task.batch_id = format!("batch-{task_id}");
    work
}

#[test]
fn frozen_policy_is_the_only_production_route_authority() {
    let profiles = AsfCollectionProfileRegistry;
    let policies = AssetExecutionPolicyRegistry;
    let resolver: SnapshotItem = serde_json::from_value(base_item_value()).unwrap();
    assert!(matches!(
        classify_frozen_execution(&resolver, &profiles, &policies),
        FrozenExecution::ResolverAuthoritative(_)
    ));

    let explicit_legacy = item_with_policy(json!({
        "authority": "legacy",
        "policy_id": "legacy.v1",
        "collection_profile_id": "asf.sentinel-1.standard.v1"
    }));
    assert!(matches!(
        classify_frozen_execution(&explicit_legacy, &profiles, &policies),
        FrozenExecution::ExplicitLegacy
    ));

    let missing = item_with_policy(Value::Null);
    assert!(matches!(
        classify_frozen_execution(&missing, &profiles, &policies),
        FrozenExecution::MissingPolicyCompatibility
    ));

    let tampered = item_with_policy(json!({
        "authority": "resolver_authoritative",
        "policy_id": "unknown.resolver.v1",
        "collection_profile_id": "asf.sentinel-1.standard.v1"
    }));
    assert!(matches!(
        classify_frozen_execution(&tampered, &profiles, &policies),
        FrozenExecution::Invalid(_)
    ));
}

#[test]
fn production_worker_reports_ready_only_after_runtime_construction() {
    let worker = ProductionDownloadWorker::new(TransferOptions::default()).unwrap();
    assert!(worker.resolver_runtime_ready());
}

#[test]
fn runtime_readiness_and_classification_do_not_read_credentials_or_resolve() {
    let (runtime, resolver, credentials) = fixture_runtime();
    assert!(runtime.is_ready());
    let item: SnapshotItem = serde_json::from_value(base_item_value()).unwrap();
    assert!(matches!(
        runtime.classify(&item),
        FrozenExecution::ResolverAuthoritative(_)
    ));
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn approved_401_reresolves_once_and_reuses_the_same_transfer_session() {
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = FakeTransfer::new(vec![auth_rejected(401, true), finished()]);
    let result = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .unwrap();

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 2);
    let sessions = transfer.sessions.lock().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0], sessions[1]);
    let urls = transfer.urls.lock().unwrap();
    assert_ne!(urls[0], urls[1]);
}

#[test]
fn ordinary_403_is_terminal_without_credential_refresh() {
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = FakeTransfer::new(vec![auth_rejected(403, false)]);
    let error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();

    assert_eq!(error.code(), "DL004");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn credential_provider_failures_remain_dl004_before_and_during_refresh() {
    let first_credentials = Arc::new(FailingCredentials::new(1));
    let runtime = runtime_with_credentials(first_credentials.clone());
    let transfer = FakeTransfer::new(vec![]);
    let first_error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(first_error.code(), "DL004");
    assert_eq!(first_credentials.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 0);

    let refresh_credentials = Arc::new(FailingCredentials::new(2));
    let runtime = runtime_with_credentials(refresh_credentials.clone());
    let transfer = FakeTransfer::new(vec![auth_rejected(401, true)]);
    let refresh_error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(refresh_error.code(), "DL004");
    assert_eq!(refresh_credentials.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn inconsistent_unapproved_refresh_candidate_is_still_terminal() {
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = FakeTransfer::new(vec![ResolvedTransferResult::AuthRejected(AuthRejection {
        status_code: 401,
        response_host_class: ResponseHostClass::Unapproved,
        refresh_candidate: true,
    })]);
    let error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();

    assert_eq!(error.code(), "DL004");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn production_worker_executes_resolver_authority_without_legacy_url_fallback() {
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = Arc::new(FakeTransfer::new(vec![finished()]));
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer.clone());
    let result = worker.execute(resolver_work("."), WorkerControl::default(), &mut |_| {});

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn resolver_refresh_releases_then_reacquires_the_same_admitted_backend_and_session() {
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = Arc::new(FakeTransfer::new(vec![
        auth_rejected(401, true),
        finished(),
    ]));
    let admission = TransferAdmissionController::finite_for_test(
        NonZeroUsize::new(1).unwrap(),
        NonZeroUsize::new(1).unwrap(),
    );
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer.clone())
        .with_transfer_admission_controller_for_test(Arc::clone(&admission));

    let result = worker.execute(resolver_work("."), WorkerControl::default(), &mut |_| {});

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transfer.legacy_calls.load(Ordering::SeqCst), 0);
    let sessions = transfer.sessions.lock().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0], sessions[1]);
    drop(sessions);
    let snapshot = admission.snapshot();
    assert_eq!(snapshot.grants, 2);
    assert_eq!(snapshot.releases, 2);
    assert_eq!(snapshot.active_global, 0);
    assert_eq!(snapshot.queued_global, 0);
    assert!(snapshot.active_by_host.is_empty());
    assert!(snapshot.queued_by_host.is_empty());
}

#[test]
fn unavailable_preferred_backend_uses_native_resolver_route_without_legacy_fallback() {
    let (runtime, _resolver, _credentials) = fixture_runtime();
    let native = Arc::new(FakeTransfer::new(vec![finished()]));
    let unavailable = Arc::new(FakeTransfer::unavailable());
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_backends_for_test(runtime, native.clone(), unavailable.clone());

    let result = worker.execute(resolver_work("."), WorkerControl::default(), &mut |_| {});

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(native.calls.load(Ordering::SeqCst), 1);
    assert_eq!(unavailable.calls.load(Ordering::SeqCst), 0);
    assert_eq!(native.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(unavailable.legacy_calls.load(Ordering::SeqCst), 0);
    let urls = native.urls.lock().unwrap();
    assert_eq!(urls.len(), 1);
    assert!(urls[0].contains("runtime-1/fixture.zip"));
    assert!(!urls[0].contains("legacy-must-not-run"));
}

#[test]
fn resolver_backend_delegation_forwards_controls_and_progress_once() {
    let (runtime, _resolver, _credentials) = fixture_runtime();
    let transfer = Arc::new(FakeTransfer::new(vec![finished()]));
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer.clone());
    let progress_callbacks = AtomicUsize::new(0);

    let result = worker.execute(resolver_work("."), WorkerControl::default(), &mut |_| {
        progress_callbacks.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.cancel_probes.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.pause_probes.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.progress_callbacks.load(Ordering::SeqCst), 1);
    assert_eq!(progress_callbacks.load(Ordering::SeqCst), 1);
}

#[test]
fn refreshed_size_conflict_is_dl002_and_never_reaches_second_transfer() {
    let resolver = Arc::new(SequenceResolver::with_second_size_drift());
    let (runtime, credentials) = runtime_with_resolver(resolver.clone());
    let transfer = FakeTransfer::new(vec![auth_rejected(401, true)]);
    let error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();

    assert_eq!(error.code(), "DL002");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn controlled_403_refreshes_once_and_second_rejection_is_terminal() {
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = FakeTransfer::new(vec![auth_rejected(403, true), finished()]);
    let result = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 2);

    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = FakeTransfer::new(vec![auth_rejected(401, true), auth_rejected(403, true)]);
    let error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(error.code(), "DL004");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn cancellation_is_checked_before_and_around_reresolution() {
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = FakeTransfer::new(vec![]);
    let error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| true,
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(error.code(), "DL001");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 0);

    let cancelled = Arc::new(AtomicBool::new(false));
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = FakeTransfer::new(vec![auth_rejected(401, true)])
        .with_cancel_after_first(cancelled.clone());
    let error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| cancelled.load(Ordering::SeqCst),
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(error.code(), "DL001");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);

    let cancelled = Arc::new(AtomicBool::new(false));
    let resolver = Arc::new(SequenceResolver::with_cancel_after_second(
        cancelled.clone(),
    ));
    let (runtime, credentials) = runtime_with_resolver(resolver.clone());
    let transfer = FakeTransfer::new(vec![auth_rejected(401, true)]);
    let error = runtime
        .execute_resolver_authoritative(
            &transfer,
            &eligible(),
            "fixture-task",
            "fixture-item",
            PathBuf::from("fixture-output.zip"),
            &|| cancelled.load(Ordering::SeqCst),
            &|| false,
            &mut |_| {},
        )
        .unwrap_err();
    assert_eq!(error.code(), "DL001");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 2);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn resolver_refresh_keeps_one_destination_lease_across_the_backend_gap() {
    let output_root = TempOutputRoot::new();
    let gate = RefreshGate::new();
    let resolver = Arc::new(SequenceResolver::with_refresh_gate(Arc::clone(&gate)));
    let (runtime, _credentials) = runtime_with_resolver(Arc::clone(&resolver));
    let transfer = Arc::new(FakeTransfer::new(vec![
        auth_rejected(401, true),
        finished(),
        finished(),
    ]));
    let controller = DestinationLeaseController::new_for_qualification_test();
    let worker = Arc::new(
        ProductionDownloadWorker::new(TransferOptions::default())
            .unwrap()
            .with_resolver_runtime_and_transfer(runtime, transfer.clone())
            .with_destination_lease_controller_for_test(Arc::clone(&controller)),
    );
    let root = output_root.as_string();

    let first_worker = Arc::clone(&worker);
    let first_work = distinct_resolver_work(&root, "refresh-task-one");
    let first = std::thread::spawn(move || {
        first_worker.execute(first_work, WorkerControl::default(), &mut |_| {})
    });
    if !gate.wait_for_entered() {
        let result = first.join().unwrap();
        panic!(
            "resolver refresh did not enter controlled gap: outcome={:?}, resolver_calls={}, transfer_calls={}",
            result.outcome,
            resolver.calls.load(Ordering::SeqCst),
            transfer.calls.load(Ordering::SeqCst)
        );
    }

    let second_worker = Arc::clone(&worker);
    let second_work = distinct_resolver_work(&root, "refresh-task-two");
    let second = std::thread::spawn(move || {
        second_worker.execute(second_work, WorkerControl::default(), &mut |_| {})
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while controller
        .snapshot_for_qualification_test()
        .queued_waiters()
        != 1
    {
        assert!(
            Instant::now() < deadline,
            "same destination competitor did not queue during resolver refresh"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);

    gate.release();
    assert_eq!(first.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_eq!(second.join().unwrap().outcome, DownloadOutcome::Completed);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 3);
    let snapshot = controller.snapshot_for_qualification_test();
    assert_eq!(snapshot.live_destinations(), 0);
    assert_eq!(snapshot.queued_waiters(), 0);
    assert_eq!(snapshot.active_leases(), 0);
    assert!(!snapshot.failed_closed());
}

#[test]
fn resolver_failure_and_invalid_policy_never_invoke_transfer_or_legacy_fallback() {
    let resolver = Arc::new(SequenceResolver::failing());
    let (runtime, credentials) = runtime_with_resolver(resolver.clone());
    let transfer = Arc::new(FakeTransfer::new(vec![]));
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer.clone());
    let result = worker.execute(resolver_work("."), WorkerControl::default(), &mut |_| {});
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.failure.unwrap().code, "DL005");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 0);

    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = Arc::new(FakeTransfer::new(vec![]));
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer.clone());
    let mut work = resolver_work(".");
    work.item.execution_policy = Some(
        serde_json::from_value(json!({
            "authority": "resolver_authoritative",
            "policy_id": "unknown.resolver.v1",
            "collection_profile_id": "asf.sentinel-1.standard.v1"
        }))
        .unwrap(),
    );
    work.task.candidates[0] = work.item.clone();
    let result = worker.execute(work, WorkerControl::default(), &mut |_| {});
    assert_eq!(result.outcome, DownloadOutcome::Failed);
    assert_eq!(result.failure.unwrap().code, "DL005");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 0);
}

/// Downloader D3 (local-first dedup): a resolver asset whose planned file
/// already exists with the frozen checksum finishes as skipped before any
/// resolution, credential or transfer.
#[test]
fn resolver_existing_file_matching_its_frozen_checksum_is_skipped_without_resolution() {
    use sha2::{Digest, Sha256};
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = Arc::new(FakeTransfer::new(vec![]));
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer.clone());
    let output = TempOutputRoot::new();
    let content = vec![7_u8; 1024];
    fs::write(output.0.join("fixture.zip"), &content).unwrap();
    let mut work = resolver_work(&output.as_string());
    let digest = Sha256::digest(&content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    work.item.asset_access.as_mut().unwrap().checksum_value = Some(digest);

    let result = worker.execute(work, WorkerControl::default(), &mut |_| {});

    assert_eq!(
        result.outcome,
        DownloadOutcome::Skipped,
        "{:?}",
        result.failure
    );
    assert_eq!(result.attempt_count, 0);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(output.0.join("fixture.zip")).unwrap(), content);
}

/// Downloader D3 (local-first dedup, Owner rule 2026-10-20): an existing file
/// whose checksum differs from the frozen one is no longer a terminal DL002;
/// it is renamed aside, kept, and the asset is downloaded.
#[test]
fn resolver_existing_file_with_another_checksum_is_renamed_aside_then_downloaded() {
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = Arc::new(FakeTransfer::new(vec![finished()]));
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer.clone());
    let output = TempOutputRoot::new();
    let content = vec![9_u8; 1024];
    fs::write(output.0.join("fixture.zip"), &content).unwrap();

    let result = worker.execute(
        resolver_work(&output.as_string()),
        WorkerControl::default(),
        &mut |_| {},
    );

    assert_eq!(
        result.outcome,
        DownloadOutcome::Completed,
        "{:?}",
        result.failure
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 1);
    let conflicts = fs::read_dir(&output.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("fixture.zip.eostage-conflict-")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        conflicts.len(),
        1,
        "the differing file is kept under its conflict name"
    );
    assert_eq!(fs::read(&conflicts[0]).unwrap(), content);
}

/// Downloader D3-2 (pre-scan): a frozen checksum needs a digest, which only
/// the scan's second phase computes; a mismatch is released untouched.
#[test]
fn resolver_pre_scan_digests_only_in_its_second_phase_and_never_renames() {
    use insar_download::{LocalScanPhase, LocalScanVerdict};
    use sha2::{Digest, Sha256};
    let (runtime, resolver, credentials) = fixture_runtime();
    let transfer = Arc::new(FakeTransfer::new(vec![]));
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .unwrap()
        .with_resolver_runtime_and_transfer(runtime, transfer.clone());
    let output = TempOutputRoot::new();
    let content = vec![5_u8; 1024];
    fs::write(output.0.join("fixture.zip"), &content).unwrap();
    let mut work = resolver_work(&output.as_string());
    let digest = Sha256::digest(&content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    work.item.asset_access.as_mut().unwrap().checksum_value = Some(digest);
    let control = WorkerControl::default();
    assert_eq!(
        worker.scan_local(&work, LocalScanPhase::Cheap, &control, None),
        LocalScanVerdict::NeedsDigest
    );
    let LocalScanVerdict::Reuse(item) =
        worker.scan_local(&work, LocalScanPhase::Digest, &control, None)
    else {
        panic!("a matching digest reuses");
    };
    assert_eq!(item.grade, "service_checksum");
    assert_eq!(item.size_bytes, 1024);
    assert!(item.landed.is_some(), "a digest proves a ledger row");
    let other = resolver_work(&output.as_string());
    assert_eq!(
        worker.scan_local(&other, LocalScanPhase::Digest, &control, None),
        LocalScanVerdict::Release
    );
    assert_eq!(
        fs::read(output.0.join("fixture.zip")).unwrap(),
        content,
        "never renamed"
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 0);
    assert_eq!(transfer.calls.load(Ordering::SeqCst), 0);
}
