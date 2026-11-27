use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{
    ArtifactValidationPlan, AuthRefreshPolicyId, HttpTransfer, ResolvedAsset,
    ResolvedTransferResult, ResolvedTransferSession, TransferAuth, TransferRequest, TransferResult,
    TransferUpdate,
};

struct ValidationAttestation {
    task_id: String,
    item_id: String,
    destination: PathBuf,
    binding: crate::ArtifactValidatorBinding,
    plan_identity: Arc<()>,
    session_identity: Arc<()>,
    dispatch_identity: Arc<()>,
}

// Intentionally crate-private even though the public trait names it. This is the
// sealing boundary for only the validated hook: downstream backends can still
// implement ordinary transfers, but safe external code cannot name this type to
// override validated dispatch and launder a native backend's attestation.
pub(crate) struct ValidatedTransferDispatch {
    identity: Arc<()>,
}

impl ValidatedTransferDispatch {
    pub(crate) fn new() -> Self {
        Self {
            identity: Arc::new(()),
        }
    }

    fn identity(&self) -> Arc<()> {
        Arc::clone(&self.identity)
    }

    fn matches(&self, identity: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.identity, identity)
    }
}

/// Opaque result of a validator-bound transfer invocation.
///
/// Validator-bound dispatch is crate-internal. Safe external backend
/// implementations retain the fail-closed default and cannot override it.
#[doc(hidden)]
pub struct ValidatedResolvedTransferResult {
    result: ResolvedTransferResult,
    attestation: ValidationAttestation,
}

impl fmt::Debug for ValidatedResolvedTransferResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedResolvedTransferResult")
            .field("result", &self.result)
            .finish_non_exhaustive()
    }
}

impl ValidatedResolvedTransferResult {
    /// Inspect the transfer outcome without exposing or consuming its attestation.
    pub const fn result(&self) -> &ResolvedTransferResult {
        &self.result
    }

    pub(crate) fn seal(
        result: ResolvedTransferResult,
        request: &TransferRequest,
        plan: &ArtifactValidationPlan,
        session: &ResolvedTransferSession,
        dispatch: &ValidatedTransferDispatch,
    ) -> Self {
        Self {
            result,
            attestation: ValidationAttestation {
                task_id: request.task_id.clone(),
                item_id: request.item_id.clone(),
                destination: request.destination.clone(),
                binding: plan.binding(),
                plan_identity: plan.attestation_identity(),
                session_identity: session.attestation_identity(),
                dispatch_identity: dispatch.identity(),
            },
        }
    }

    pub(crate) fn into_verified_result(
        self,
        request: &TransferRequest,
        plan: &ArtifactValidationPlan,
        session: &ResolvedTransferSession,
        dispatch: &ValidatedTransferDispatch,
    ) -> Option<ResolvedTransferResult> {
        let attestation = &self.attestation;
        (attestation.task_id == request.task_id
            && attestation.item_id == request.item_id
            && attestation.destination == request.destination
            && attestation.binding == plan.binding()
            && plan.matches_attestation_identity(&attestation.plan_identity)
            && session.matches_attestation_identity(&attestation.session_identity)
            && dispatch.matches(&attestation.dispatch_identity))
        .then_some(self.result)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub enum TransferBackendKind {
    NativeSingleStream,
}

#[doc(hidden)]
pub trait TransferBackend: Send + Sync {
    fn kind(&self) -> TransferBackendKind;

    fn is_available(&self) -> bool;

    fn download_legacy(
        &self,
        request: &TransferRequest,
        auth: &TransferAuth,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult;

    fn resolved_session(&self) -> ResolvedTransferSession;

    #[allow(clippy::too_many_arguments)]
    fn download_resolved(
        &self,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        policy: AuthRefreshPolicyId,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult;

    #[allow(clippy::too_many_arguments)]
    // See `ValidatedTransferDispatch`: the narrower parameter visibility is an
    // intentional security boundary, not an accidental public API leak.
    #[allow(private_interfaces)]
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
        ValidatedResolvedTransferResult::seal(
            ResolvedTransferResult::Finished(
                plan.unavailable_transfer_result(&request.destination, session.attempts_used()),
            ),
            request,
            plan,
            session,
            dispatch,
        )
    }
}

#[doc(hidden)]
pub struct TransferBackendSelector {
    native: Arc<dyn TransferBackend>,
    preferred_for_test: Option<Arc<dyn TransferBackend>>,
}

impl TransferBackendSelector {
    pub fn native(native: Arc<dyn TransferBackend>) -> Self {
        Self {
            native,
            preferred_for_test: None,
        }
    }

    #[doc(hidden)]
    pub fn with_preferred_for_test(
        native: Arc<dyn TransferBackend>,
        preferred_for_test: Arc<dyn TransferBackend>,
    ) -> Self {
        Self {
            native,
            preferred_for_test: Some(preferred_for_test),
        }
    }

    pub fn select(&self) -> &dyn TransferBackend {
        self.preferred_for_test
            .as_deref()
            .filter(|backend| backend.is_available())
            .unwrap_or(self.native.as_ref())
    }
}

impl TransferBackend for HttpTransfer {
    fn kind(&self) -> TransferBackendKind {
        TransferBackendKind::NativeSingleStream
    }

    fn is_available(&self) -> bool {
        true
    }

    fn download_legacy(
        &self,
        request: &TransferRequest,
        auth: &TransferAuth,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        HttpTransfer::download_authenticated(self, request, auth, cancelled, paused, progress)
    }

    fn resolved_session(&self) -> ResolvedTransferSession {
        HttpTransfer::resolved_session(self)
    }

    fn download_resolved(
        &self,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        policy: AuthRefreshPolicyId,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        HttpTransfer::download_resolved(
            self, request, asset, policy, session, cancelled, paused, progress,
        )
    }

    #[allow(private_interfaces)]
    fn download_resolved_validated(
        &self,
        dispatch: &ValidatedTransferDispatch,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        policy: AuthRefreshPolicyId,
        plan: &ArtifactValidationPlan,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ValidatedResolvedTransferResult {
        let result = HttpTransfer::download_resolved_validated(
            self, request, asset, policy, plan, session, cancelled, paused, progress,
        );
        ValidatedResolvedTransferResult::seal(result, request, plan, session, dispatch)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use insar_core::DownloadOutcome;

    use super::*;
    use crate::{
        AdmittedTransferBackend, ArtifactValidatorBinding, ArtifactValidatorId, ResolvedMethod,
        TransferAdmissionController,
    };

    fn fixture_request(task_id: &str, item_id: &str, destination: &str) -> TransferRequest {
        TransferRequest::new(
            task_id,
            item_id,
            "https://example.invalid/product.h5",
            PathBuf::from(destination),
            Some(5),
        )
        .unwrap()
    }

    fn completed() -> ResolvedTransferResult {
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Completed,
            bytes_written: 5,
            attempt_count: 1,
            failure: None,
        })
    }

    fn sealed(
        request: &TransferRequest,
        plan: &ArtifactValidationPlan,
        session: &ResolvedTransferSession,
        dispatch: &ValidatedTransferDispatch,
    ) -> ValidatedResolvedTransferResult {
        ValidatedResolvedTransferResult::seal(completed(), request, plan, session, dispatch)
    }

    #[test]
    fn validated_result_requires_every_bound_identity_component() {
        let request = fixture_request("task", "item", "product.h5");
        let plan = ArtifactValidationPlan::unavailable_for_test();
        let session = ResolvedTransferSession::new(3).unwrap();
        let dispatch = ValidatedTransferDispatch::new();

        assert!(matches!(
            sealed(&request, &plan, &session, &dispatch).into_verified_result(
                &request,
                &plan,
                &session,
                &dispatch
            ),
            Some(ResolvedTransferResult::Finished(result))
                if result.outcome == DownloadOutcome::Completed
        ));

        let wrong_task = fixture_request("other-task", "item", "product.h5");
        assert!(sealed(&request, &plan, &session, &dispatch)
            .into_verified_result(&wrong_task, &plan, &session, &dispatch)
            .is_none());

        let wrong_item = fixture_request("task", "other-item", "product.h5");
        assert!(sealed(&request, &plan, &session, &dispatch)
            .into_verified_result(&wrong_item, &plan, &session, &dispatch)
            .is_none());

        let wrong_destination = fixture_request("task", "item", "other-product.h5");
        assert!(sealed(&request, &plan, &session, &dispatch)
            .into_verified_result(&wrong_destination, &plan, &session, &dispatch)
            .is_none());

        let wrong_binding = ArtifactValidatorBinding::new_for_test(
            ArtifactValidatorId::new_for_test("fixture.other-validator"),
            7,
        );
        let wrong_binding_plan = plan.with_binding_for_test(wrong_binding);
        assert!(sealed(&request, &plan, &session, &dispatch)
            .into_verified_result(&request, &wrong_binding_plan, &session, &dispatch)
            .is_none());

        let other_plan_with_same_binding = ArtifactValidationPlan::unavailable_for_test();
        assert_eq!(other_plan_with_same_binding.binding(), plan.binding());
        assert!(sealed(&request, &plan, &session, &dispatch)
            .into_verified_result(&request, &other_plan_with_same_binding, &session, &dispatch,)
            .is_none());

        let other_session = ResolvedTransferSession::new(3).unwrap();
        assert!(sealed(&request, &plan, &session, &dispatch)
            .into_verified_result(&request, &plan, &other_session, &dispatch)
            .is_none());

        let other_dispatch = ValidatedTransferDispatch::new();
        assert!(sealed(&request, &plan, &session, &dispatch)
            .into_verified_result(&request, &plan, &session, &other_dispatch)
            .is_none());
    }

    struct AttestingBackend {
        calls: AtomicUsize,
    }

    impl TransferBackend for AttestingBackend {
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
            panic!("validated admission fixture must not invoke Legacy")
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
            _progress: &mut dyn FnMut(TransferUpdate),
        ) -> ResolvedTransferResult {
            panic!("validated admission fixture must not invoke ordinary resolved transfer")
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            ValidatedResolvedTransferResult::seal(completed(), request, plan, session, dispatch)
        }
    }

    #[test]
    fn admission_forwards_one_attested_result_without_resealing_or_replay() {
        let backend = AttestingBackend {
            calls: AtomicUsize::new(0),
        };
        let controller = TransferAdmissionController::compatibility();
        let admitted = AdmittedTransferBackend::new(&backend, Arc::clone(&controller));
        let request = fixture_request("admitted-task", "admitted-item", "admitted.h5");
        let asset =
            ResolvedAsset::new("https://example.invalid/product.h5", ResolvedMethod::Get).unwrap();
        let plan = ArtifactValidationPlan::unavailable_for_test();
        let mut session = admitted.resolved_session();
        let dispatch = ValidatedTransferDispatch::new();

        let result = admitted.download_resolved_validated(
            &dispatch,
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &plan,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        let result = result
            .into_verified_result(&request, &plan, &session, &dispatch)
            .expect("admission must return the delegate's valid attestation");

        assert!(matches!(
            result,
            ResolvedTransferResult::Finished(result)
                if result.outcome == DownloadOutcome::Completed
        ));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.grants, 1);
        assert_eq!(snapshot.releases, 1);
        assert_eq!(snapshot.active_global, 0);
    }
}
