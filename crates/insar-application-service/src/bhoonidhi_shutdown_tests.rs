use super::*;
use insar_download::{
    BhoonidhiAuthError, BhoonidhiLoginMaterial, BhoonidhiTokenEndpoint, BhoonidhiTokenGrant,
};
use std::sync::atomic::AtomicUsize;

struct Endpoint {
    calls: AtomicUsize,
    fail_logout: bool,
}

impl BhoonidhiTokenEndpoint for Endpoint {
    fn network_policy(&self) -> ProviderNetworkPolicy {
        ProviderNetworkPolicy::system()
    }
    fn password_grant(
        &self,
        _: &BhoonidhiLoginMaterial,
    ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
        BhoonidhiTokenGrant::new(
            "fixture-access",
            Some("fixture-refresh".to_owned()),
            Some(60),
        )
    }
    fn refresh_grant(&self, _: &str, _: &str) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
        panic!("shutdown must not refresh or reauthenticate")
    }
    fn logout(&self, refresh: &str) -> Result<(), BhoonidhiAuthError> {
        assert_eq!(refresh, "fixture-refresh");
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_logout {
            Err(BhoonidhiAuthError::Transport)
        } else {
            Ok(())
        }
    }
}

#[test]
fn bhoonidhi_shutdown_revokes_once_and_clears_even_on_transport_failure() {
    for fail_logout in [false, true] {
        let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
        let endpoint = Arc::new(Endpoint {
            calls: AtomicUsize::new(0),
            fail_logout,
        });
        let session = Arc::new(
            BhoonidhiSession::new(
                ProviderNetworkPolicy::system(),
                endpoint.clone(),
                Arc::new(SystemClock),
            )
            .unwrap(),
        );
        session
            .login(BhoonidhiLoginMaterial::new("fixture-user", "fixture-password").unwrap())
            .unwrap();
        let provider = Arc::new(BhoonidhiSessionTokenProvider::new(session.clone()));
        *service.bhoonidhi_session.lock().unwrap() = Some(BhoonidhiSessionRuntime {
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            acceptance_lease: None,
            session: session.clone(),
            provider: provider.clone(),
            admission: TransferAdmissionController::compatibility(),
        });
        let result = service.shutdown();
        assert_eq!(
            endpoint.calls.load(Ordering::SeqCst),
            1,
            "shutdown must revoke this process-owned session"
        );
        if fail_logout {
            assert_eq!(
                result.unwrap_err().code(),
                "BHOONIDHI_AUTH_TRANSPORT_FAILED"
            );
        } else {
            result.unwrap();
        }
        assert!(!session.snapshot().unwrap().authenticated);
        assert!(provider.access_token_with_generation().is_err());
        assert!(service.bhoonidhi_session.lock().unwrap().is_none());
        service.shutdown().unwrap();
        assert_eq!(
            endpoint.calls.load(Ordering::SeqCst),
            1,
            "idempotent shutdown must not repeat network requests"
        );
    }
}

#[test]
fn bhoonidhi_shutdown_does_not_construct_or_authenticate_an_unused_session() {
    let service = ApplicationService::new(ApplicationServiceConfig::liveness_only());
    service.shutdown().unwrap();
    assert!(service.bhoonidhi_session.lock().unwrap().is_none());
}
