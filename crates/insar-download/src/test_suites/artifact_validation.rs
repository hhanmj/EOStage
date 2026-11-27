use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::nisar_support::{frozen_item, sha256_hex, work, OFFICIAL_HREF};
use insar_core::{
    AssetAccessDescriptor, AssetAccessScheme, DownloadOutcome, FrozenAssetAccess,
    StableAssetReference,
};
use insar_download::{
    evaluate_resolver_cutover_eligibility, ArtifactValidationContext, ArtifactValidationError,
    ArtifactValidationPlan, ArtifactValidatorBinding, ArtifactValidatorId,
    ArtifactValidatorRegistry, AsfCollectionProfileRegistry, AssetExecutionPolicyRegistry,
    AssetResolver, AssetResolverRegistry, AuthRefreshPolicyId, AuthRejection, Clock,
    CompletedArtifactValidator, CredentialMaterial, CredentialProvider, CredentialProviderError,
    DownloadWorker, FrozenExecution, ProductionDownloadWorker, ResolveContext, ResolveError,
    ResolvedAsset, ResolvedMethod, ResolvedTransferResult, ResolvedTransferSession,
    ResolverCutoverEligibility, ResolverExecutionRuntime, ResponseHostClass, ShadowAccessIntent,
    ShadowResolveContext, TransferAuth, TransferBackend, TransferBackendKind, TransferOptions,
    TransferRequest, TransferResult, TransferUpdate, ValidatedResolvedTransferResult,
    WorkerControl, ASF_SENTINEL1_STANDARD_POLICY_ID, CDSE_SENTINEL2_L2A_POLICY_ID,
    NISAR_RSLC_STRUCTURE_VALIDATOR_ID, NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
    NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION,
};
use insar_provider::catalog::{
    ALOS2_RESOLVER_POLICY_ID, NISAR_RESOLVER_POLICY_ID_V1, NISAR_RESOLVER_POLICY_ID_V2,
};

use crate::transfer_backend::ValidatedTransferDispatch;

const BODY: &[u8] = b"\x89HDF\r\n\x1a\nartifact-validation-policy";
const UNAVAILABLE: &str = "NISAR_HDF5_VALIDATION_UNAVAILABLE";

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH
    }
}

#[derive(Default)]
struct RecordingCredentials {
    calls: AtomicUsize,
}

impl CredentialProvider for RecordingCredentials {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        assert_eq!(profile, "earthdata-default");
        self.calls.fetch_add(1, Ordering::SeqCst);
        CredentialMaterial::bearer("ARTIFACT_VALIDATION_FIXTURE_TOKEN")
    }
}

#[derive(Default)]
struct RecordingResolver {
    calls: AtomicUsize,
}

impl AssetResolver for RecordingResolver {
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
        _reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        context.check_cancelled()?;
        let profile = descriptor
            .credential_profile
            .as_deref()
            .ok_or_else(ResolveError::descriptor_invalid)?;
        let credential = context
            .credential_provider()
            .resolve(profile)
            .map_err(|_| ResolveError::credentials_missing())?;
        let mut asset = ResolvedAsset::new(OFFICIAL_HREF, ResolvedMethod::Get)?;
        credential.apply_to(&mut asset)?;
        Ok(asset)
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::failed())
    }
}

#[derive(Default)]
struct OrdinaryOnlyBackend {
    legacy_calls: AtomicUsize,
    ordinary_resolved_calls: AtomicUsize,
}

impl OrdinaryOnlyBackend {
    fn completed() -> TransferResult {
        TransferResult {
            outcome: DownloadOutcome::Completed,
            bytes_written: BODY.len() as u64,
            attempt_count: 1,
            failure: None,
        }
    }
}

impl TransferBackend for OrdinaryOnlyBackend {
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
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        self.legacy_calls.fetch_add(1, Ordering::SeqCst);
        Self::completed()
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).expect("bounded fixture session")
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        _session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        self.ordinary_resolved_calls.fetch_add(1, Ordering::SeqCst);
        ResolvedTransferResult::Finished(Self::completed())
    }

    // Deliberately do not implement download_resolved_validated. The trait's
    // fail-closed default is the compatibility contract under test.
}

#[derive(Default)]
struct MismatchedAttestationBackend {
    validated_calls: AtomicUsize,
}

impl TransferBackend for MismatchedAttestationBackend {
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
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        panic!("validated NISAR fixture must not invoke Legacy")
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).expect("bounded fixture session")
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        _session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        panic!("validated NISAR fixture must not invoke ordinary resolved transfer")
    }

    fn download_resolved_validated(
        &self,
        dispatch: &ValidatedTransferDispatch,
        request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        _plan: &ArtifactValidationPlan,
        session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ValidatedResolvedTransferResult {
        self.validated_calls.fetch_add(1, Ordering::SeqCst);
        let forged_plan = ArtifactValidationPlan::unavailable_for_test();
        ValidatedResolvedTransferResult::seal(
            ResolvedTransferResult::Finished(OrdinaryOnlyBackend::completed()),
            request,
            &forged_plan,
            session,
            dispatch,
        )
    }
}

#[derive(Default)]
struct RefreshRecordingBackend {
    validated_calls: AtomicUsize,
    legacy_calls: AtomicUsize,
    ordinary_resolved_calls: AtomicUsize,
    plan_addresses: Mutex<Vec<usize>>,
    session_addresses: Mutex<Vec<usize>>,
}

impl TransferBackend for RefreshRecordingBackend {
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
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        self.legacy_calls.fetch_add(1, Ordering::SeqCst);
        OrdinaryOnlyBackend::completed()
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(3).expect("bounded fixture session")
    }

    fn download_resolved(
        &self,
        _request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        _session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        self.ordinary_resolved_calls.fetch_add(1, Ordering::SeqCst);
        ResolvedTransferResult::Finished(OrdinaryOnlyBackend::completed())
    }

    fn download_resolved_validated(
        &self,
        dispatch: &ValidatedTransferDispatch,
        request: &TransferRequest,
        _asset: &ResolvedAsset,
        _policy: AuthRefreshPolicyId,
        plan: &ArtifactValidationPlan,
        session: &mut ResolvedTransferSession,
        _cancelled: &(dyn Fn() -> bool + Sync),
        _paused: &(dyn Fn() -> bool + Sync),
        _progress: &mut dyn FnMut(TransferUpdate),
    ) -> ValidatedResolvedTransferResult {
        self.plan_addresses
            .lock()
            .unwrap()
            .push(plan as *const ArtifactValidationPlan as usize);
        self.session_addresses
            .lock()
            .unwrap()
            .push(session as *mut ResolvedTransferSession as usize);
        let result = if self.validated_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            ResolvedTransferResult::AuthRejected(AuthRejection {
                status_code: 401,
                response_host_class: ResponseHostClass::Approved,
                refresh_candidate: true,
            })
        } else {
            ResolvedTransferResult::Finished(OrdinaryOnlyBackend::completed())
        };
        ValidatedResolvedTransferResult::seal(result, request, plan, session, dispatch)
    }
}

struct ReportingValidator {
    reported_id: &'static str,
    reported_version: u32,
    validation_calls: AtomicUsize,
}

impl ReportingValidator {
    fn new(reported_id: &'static str, reported_version: u32) -> Self {
        Self {
            reported_id,
            reported_version,
            validation_calls: AtomicUsize::new(0),
        }
    }
}

impl CompletedArtifactValidator for ReportingValidator {
    fn id(&self) -> ArtifactValidatorId {
        ArtifactValidatorId::new_for_test(self.reported_id)
    }

    fn version(&self) -> u32 {
        self.reported_version
    }

    fn validate(
        &self,
        _candidate_path: &Path,
        _asset: &ResolvedAsset,
        _context: &ArtifactValidationContext,
    ) -> Result<(), ArtifactValidationError> {
        self.validation_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn item() -> insar_core::SnapshotItem {
    frozen_item(BODY, sha256_hex(BODY))
}

fn output_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "insar-artifact-validation-{label}-{}",
        std::process::id()
    ))
}

fn runtime_with_registry(
    registry: ArtifactValidatorRegistry,
    resolver: Arc<RecordingResolver>,
    credentials: Arc<RecordingCredentials>,
) -> ResolverExecutionRuntime {
    let mut resolvers = AssetResolverRegistry::new();
    resolvers
        .register(resolver)
        .expect("one fixture Earthdata resolver");
    ResolverExecutionRuntime::with_dependencies_and_artifact_validators_for_test(
        resolvers,
        credentials,
        Arc::new(FixedClock),
        registry,
    )
}

fn run_worker<B: TransferBackend + 'static>(
    label: &str,
    registry: ArtifactValidatorRegistry,
    resolver: Arc<RecordingResolver>,
    credentials: Arc<RecordingCredentials>,
    backend: Arc<B>,
) -> insar_core::DownloadItemResult {
    let runtime = runtime_with_registry(registry, resolver, credentials);
    let worker = ProductionDownloadWorker::new(TransferOptions::default())
        .expect("fixture worker")
        .with_resolver_runtime_and_transfer(runtime, backend);
    worker.execute(
        work(&output_root(label), item(), label),
        WorkerControl::default(),
        &mut |_| {},
    )
}

fn assert_unavailable(result: &insar_core::DownloadItemResult, label: &str) {
    assert_eq!(result.outcome, DownloadOutcome::Failed, "{label}");
    assert_eq!(
        result.failure.as_ref().map(|failure| failure.code.as_str()),
        Some(UNAVAILABLE),
        "{label}"
    );
    assert!(
        result
            .failure
            .as_ref()
            .is_some_and(|failure| !failure.retryable),
        "{label}"
    );
}

fn registry_with(
    registered_id: &'static str,
    registered_version: u32,
    validator: Arc<ReportingValidator>,
) -> ArtifactValidatorRegistry {
    ArtifactValidatorRegistry::with_validator_for_test(
        ArtifactValidatorBinding::new_for_test(
            ArtifactValidatorId::new_for_test(registered_id),
            registered_version,
        ),
        validator,
    )
}

#[test]
fn nisar_policy_binds_exact_structure_validator_and_other_policies_bind_none() {
    let policies = AssetExecutionPolicyRegistry;
    let legacy_binding = policies
        .get(NISAR_RESOLVER_POLICY_ID_V1)
        .expect("legacy NISAR execution policy")
        .artifact_validator_binding()
        .expect("legacy NISAR always binds structural validation");
    assert_eq!(legacy_binding.id(), NISAR_RSLC_STRUCTURE_VALIDATOR_ID);
    assert_eq!(
        legacy_binding.version(),
        NISAR_RSLC_STRUCTURE_VALIDATOR_VERSION
    );
    assert_eq!(legacy_binding.version(), 1);

    let binding = policies
        .get(NISAR_RESOLVER_POLICY_ID_V2)
        .expect("P05023 NISAR execution policy")
        .artifact_validator_binding()
        .expect("P05023 NISAR always binds structural validation");
    assert_eq!(binding.id(), NISAR_RSLC_STRUCTURE_VALIDATOR_ID);
    assert_eq!(binding.id().as_str(), "nisar.rslc.structure");
    assert_eq!(binding.version(), NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION);
    assert_eq!(binding.version(), 2);

    for policy_id in [
        ASF_SENTINEL1_STANDARD_POLICY_ID,
        CDSE_SENTINEL2_L2A_POLICY_ID,
        ALOS2_RESOLVER_POLICY_ID,
    ] {
        let policy = policies.get(policy_id).expect("existing non-NISAR policy");
        assert!(
            policy.artifact_validator_binding().is_none(),
            "{policy_id} must retain byte-for-byte transfer compatibility"
        );
    }
}

#[test]
fn eligibility_derives_one_opaque_private_plan_without_persisting_caller_fields() {
    let frozen = item();
    let before = serde_json::to_string(&frozen).expect("snapshot serializes");
    assert!(!before.contains(NISAR_RSLC_STRUCTURE_VALIDATOR_ID.as_str()));
    assert!(!before.contains("artifact_validation"));

    let result = evaluate_resolver_cutover_eligibility(
        &frozen,
        &AsfCollectionProfileRegistry,
        &AssetExecutionPolicyRegistry,
    );
    let ResolverCutoverEligibility::Eligible(eligible) = result else {
        panic!("exact frozen NISAR item must derive a validated execution plan");
    };
    assert!(eligible.has_artifact_validation_plan());
    let plan = eligible
        .artifact_validation_plan()
        .expect("NISAR eligibility owns the only plan");
    assert_eq!(plan.binding().id(), NISAR_RSLC_STRUCTURE_VALIDATOR_ID);
    assert_eq!(
        plan.binding().version(),
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION
    );
    assert!(matches!(
        plan.context(),
        ArtifactValidationContext::NisarRslc(_)
    ));

    // Eligibility owns an opaque plan; the persisted/generated SnapshotItem is
    // unchanged and offers no caller-supplied binding, context, or plan field.
    let after = serde_json::to_string(&frozen).expect("snapshot still serializes");
    assert_eq!(after, before);
}

#[test]
fn invalid_registry_bindings_fail_before_credentials_resolver_backend_or_validator() {
    let expected_id = NISAR_RSLC_STRUCTURE_VALIDATOR_ID;
    let expected_version = NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION;
    let cases = [
        ("missing", ArtifactValidatorRegistry::empty_for_test(), None),
        {
            let validator = Arc::new(ReportingValidator::new("fixture.unknown", 1));
            (
                "unknown-id",
                registry_with("fixture.unknown", 1, Arc::clone(&validator)),
                Some(validator),
            )
        },
        {
            let validator = Arc::new(ReportingValidator::new(
                expected_id.as_str(),
                expected_version + 1,
            ));
            (
                "wrong-version",
                registry_with(
                    expected_id.as_str(),
                    expected_version + 1,
                    Arc::clone(&validator),
                ),
                Some(validator),
            )
        },
        {
            let validator = Arc::new(ReportingValidator::new(
                "fixture.self-mismatch",
                expected_version,
            ));
            (
                "self-mismatched-id",
                registry_with(
                    expected_id.as_str(),
                    expected_version,
                    Arc::clone(&validator),
                ),
                Some(validator),
            )
        },
        {
            let validator = Arc::new(ReportingValidator::new(
                expected_id.as_str(),
                expected_version + 1,
            ));
            (
                "self-mismatched-version",
                registry_with(
                    expected_id.as_str(),
                    expected_version,
                    Arc::clone(&validator),
                ),
                Some(validator),
            )
        },
    ];

    for (label, registry, validator) in cases {
        let resolver = Arc::new(RecordingResolver::default());
        let credentials = Arc::new(RecordingCredentials::default());
        let backend = Arc::new(OrdinaryOnlyBackend::default());
        let result = run_worker(
            label,
            registry,
            Arc::clone(&resolver),
            Arc::clone(&credentials),
            Arc::clone(&backend),
        );

        assert_unavailable(&result, label);
        assert_eq!(credentials.calls.load(Ordering::SeqCst), 0, "{label}");
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0, "{label}");
        assert_eq!(backend.legacy_calls.load(Ordering::SeqCst), 0, "{label}");
        assert_eq!(
            backend.ordinary_resolved_calls.load(Ordering::SeqCst),
            0,
            "{label}"
        );
        if let Some(validator) = validator {
            assert_eq!(
                validator.validation_calls.load(Ordering::SeqCst),
                0,
                "{label}"
            );
        }
    }
}

#[test]
fn external_style_backend_without_internal_dispatch_cannot_forge_validated_completion() {
    let validator = Arc::new(ReportingValidator::new(
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID.as_str(),
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
    ));
    let registry = registry_with(
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID.as_str(),
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
        Arc::clone(&validator),
    );
    let resolver = Arc::new(RecordingResolver::default());
    let credentials = Arc::new(RecordingCredentials::default());
    let backend = Arc::new(OrdinaryOnlyBackend::default());

    // This backend implements only the externally nameable ordinary methods.
    // Downstream safe Rust cannot name the crate-private validated dispatch
    // challenge, so this call must use the trait's fail-closed default.
    let result = run_worker(
        "ordinary-backend-bypass",
        registry,
        Arc::clone(&resolver),
        Arc::clone(&credentials),
        Arc::clone(&backend),
    );

    assert_unavailable(&result, "ordinary-backend-bypass");
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(backend.ordinary_resolved_calls.load(Ordering::SeqCst), 0);
    assert_eq!(validator.validation_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn resolver_rejects_mismatched_plan_attestation_from_an_internal_backend() {
    let validator = Arc::new(ReportingValidator::new(
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID.as_str(),
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
    ));
    let registry = registry_with(
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID.as_str(),
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
        validator,
    );
    let resolver = Arc::new(RecordingResolver::default());
    let credentials = Arc::new(RecordingCredentials::default());
    let backend = Arc::new(MismatchedAttestationBackend::default());

    let result = run_worker(
        "forged-validated-completion",
        registry,
        resolver,
        credentials,
        Arc::clone(&backend),
    );

    assert_unavailable(&result, "forged-validated-completion");
    assert_eq!(backend.validated_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn auth_refresh_reuses_the_identical_policy_plan_and_transfer_session() {
    let validator = Arc::new(ReportingValidator::new(
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID.as_str(),
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
    ));
    let registry = registry_with(
        NISAR_RSLC_STRUCTURE_VALIDATOR_ID.as_str(),
        NISAR_RSLC_STRUCTURE_VALIDATOR_V2_VERSION,
        validator,
    );
    let resolver = Arc::new(RecordingResolver::default());
    let credentials = Arc::new(RecordingCredentials::default());
    let runtime = runtime_with_registry(registry, Arc::clone(&resolver), Arc::clone(&credentials));
    let frozen = item();
    let FrozenExecution::ResolverAuthoritative(eligible) = runtime.classify(&frozen) else {
        panic!("exact NISAR item must retain resolver authority");
    };
    let expected_plan_address = eligible
        .artifact_validation_plan()
        .map(|plan| plan as *const ArtifactValidationPlan as usize)
        .expect("NISAR eligibility owns its policy-derived plan");
    let backend = RefreshRecordingBackend::default();

    let result = runtime
        .execute_resolver_authoritative(
            &backend,
            &eligible,
            "artifact-validation-refresh",
            "artifact-validation-refresh-item",
            output_root("auth-refresh").join("product.h5"),
            &|| false,
            &|| false,
            &mut |_| {},
        )
        .expect("one approved refresh succeeds");

    assert_eq!(result.outcome, DownloadOutcome::Completed);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(credentials.calls.load(Ordering::SeqCst), 2);
    assert_eq!(backend.validated_calls.load(Ordering::SeqCst), 2);
    assert_eq!(backend.legacy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(backend.ordinary_resolved_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        *backend.plan_addresses.lock().unwrap(),
        vec![expected_plan_address, expected_plan_address]
    );
    let sessions = backend.session_addresses.lock().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0], sessions[1]);
}
