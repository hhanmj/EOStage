//! Runtime adapters for the process-owned Bhoonidhi session.
//!
//! The catalogue crate deliberately depends on a very small token boundary.
//! This module keeps the native session behind that boundary while retaining a
//! generation-aware path for the download/catalogue owner to bind one refresh
//! to the token that was rejected.  No adapter method serializes or logs token
//! material.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::credentials::{bhoonidhi_credential_read_guard, bhoonidhi_credential_revision};

use insar_provider::bhoonidhi::{
    BhoonidhiAccessTokenLease as CatalogAccessTokenLease, BhoonidhiAccessTokenProvider,
    BhoonidhiAuthError as CatalogAuthError, BhoonidhiRefreshResult as CatalogRefreshResult,
};
use zeroize::Zeroizing;

use crate::{
    read_desktop_bhoonidhi_login_material, BhoonidhiAccessToken, BhoonidhiAuthError,
    BhoonidhiCredentialError, BhoonidhiRefreshOutcome, BhoonidhiSession, CredentialMaterial,
    CredentialProvider, CredentialProviderError, BHOONIDHI_CREDENTIAL_PROFILE,
};

trait DesktopLoginLoader: Send + Sync {
    fn load(&self) -> Result<crate::BhoonidhiLoginMaterial, BhoonidhiAuthError>;
}

struct NativeDesktopLoginLoader;

impl DesktopLoginLoader for NativeDesktopLoginLoader {
    fn load(&self) -> Result<crate::BhoonidhiLoginMaterial, BhoonidhiAuthError> {
        read_desktop_bhoonidhi_login_material().map_err(map_desktop_credential_error)
    }
}

struct DesktopLoginState {
    attempted: bool,
    failure: Option<BhoonidhiAuthError>,
    disabled: bool,
    observed_revision: u64,
    last_attempt: Option<Instant>,
    recovery_attempts: VecDeque<Instant>,
}

impl DesktopLoginState {
    fn new() -> Self {
        Self {
            attempted: false,
            failure: None,
            disabled: false,
            observed_revision: bhoonidhi_credential_revision(),
            last_attempt: None,
            recovery_attempts: VecDeque::new(),
        }
    }
}

/// Shared lazy-login state. The mutex is deliberately held across the
/// credential-manager read and password grant so clones cannot create an
/// authentication storm for one process-owned session.
struct DesktopLoginGate {
    loader: Arc<dyn DesktopLoginLoader>,
    state: Mutex<DesktopLoginState>,
}

impl DesktopLoginGate {
    fn native() -> Self {
        Self::with_loader(Arc::new(NativeDesktopLoginLoader))
    }

    fn with_loader(loader: Arc<dyn DesktopLoginLoader>) -> Self {
        Self {
            loader,
            state: Mutex::new(DesktopLoginState::new()),
        }
    }
}

/// Bridges the process-owned native session to the protected catalogue token
/// boundary.
///
/// The adapter owns only an [`Arc`] to the session.  Session single-flight,
/// refresh, logout and race guards remain in [`BhoonidhiSession`].
#[derive(Clone)]
pub struct BhoonidhiSessionTokenProvider {
    session: Arc<BhoonidhiSession>,
    desktop_login: Option<Arc<DesktopLoginGate>>,
    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    acceptance_profile: Option<Arc<std::path::PathBuf>>,
}

impl BhoonidhiSessionTokenProvider {
    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    /// Acceptance host must establish exclusive ownership of this isolated
    /// profile before binding it. Never bind an untrusted UI-supplied path.
    pub fn with_auth_revoke_acceptance_profile(
        mut self,
        profile: std::path::PathBuf,
    ) -> Result<Self, BhoonidhiAuthError> {
        if !profile.is_absolute() || !profile.is_dir() {
            return Err(BhoonidhiAuthError::Configuration);
        }
        let profile = profile
            .canonicalize()
            .map_err(|_| BhoonidhiAuthError::Configuration)?;
        if self
            .acceptance_profile
            .as_ref()
            .is_some_and(|bound| bound.as_ref() != &profile)
        {
            return Err(BhoonidhiAuthError::Configuration);
        }
        self.acceptance_profile = Some(Arc::new(profile));
        Ok(self)
    }

    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    /// Read only: no lazy login, credential load, refresh or token disclosure.
    pub fn acceptance_current_generation(
        &self,
        profile: &std::path::Path,
    ) -> Result<u64, BhoonidhiAuthError> {
        self.validate_acceptance_profile(profile)?;
        let _credential_guard =
            bhoonidhi_credential_read_guard().map_err(|_| BhoonidhiAuthError::Synchronization)?;
        let gate = self
            .desktop_login
            .as_ref()
            .ok_or(BhoonidhiAuthError::Configuration)?;
        let state = gate
            .state
            .lock()
            .map_err(|_| BhoonidhiAuthError::Synchronization)?;
        if state.disabled || state.observed_revision != bhoonidhi_credential_revision() {
            return Err(BhoonidhiAuthError::SessionChanged);
        }
        let snapshot = self.session.snapshot()?;
        if !snapshot.authenticated {
            return Err(BhoonidhiAuthError::SessionUnavailable);
        }
        snapshot
            .generation
            .ok_or(BhoonidhiAuthError::SessionUnavailable)
    }

    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    /// One official revocation attempt for this session owner. The host must
    /// first quiesce other work sharing this provider. Local generation and
    /// expiry remain intact so subsequent ordinary refresh observes the server.
    pub fn acceptance_revoke_current_generation(
        &self,
        profile: &std::path::Path,
        expected_generation: u64,
    ) -> Result<(), BhoonidhiAuthError> {
        self.validate_acceptance_profile(profile)?;
        let _credential_guard =
            bhoonidhi_credential_read_guard().map_err(|_| BhoonidhiAuthError::Synchronization)?;
        let gate = self
            .desktop_login
            .as_ref()
            .ok_or(BhoonidhiAuthError::Configuration)?;
        let state = gate
            .state
            .lock()
            .map_err(|_| BhoonidhiAuthError::Synchronization)?;
        if state.disabled || state.observed_revision != bhoonidhi_credential_revision() {
            return Err(BhoonidhiAuthError::SessionChanged);
        }
        self.session
            .acceptance_revoke_current_generation(expected_generation)
    }

    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    fn validate_acceptance_profile(
        &self,
        profile: &std::path::Path,
    ) -> Result<(), BhoonidhiAuthError> {
        let bound = self
            .acceptance_profile
            .as_ref()
            .ok_or(BhoonidhiAuthError::Configuration)?;
        if !profile.is_absolute()
            || !profile.is_dir()
            || profile
                .canonicalize()
                .map_err(|_| BhoonidhiAuthError::Configuration)?
                != **bound
        {
            return Err(BhoonidhiAuthError::Configuration);
        }
        Ok(())
    }

    pub fn new(session: Arc<BhoonidhiSession>) -> Self {
        Self {
            session,
            desktop_login: None,
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            acceptance_profile: None,
        }
    }

    /// Enables one-shot lazy login from the native desktop credential store.
    ///
    /// The session remains process-owned and the gate is shared by every
    /// clone. No credential-manager read or authentication request occurs
    /// until a caller explicitly asks for an access token.
    pub fn with_desktop_login(mut self) -> Self {
        if self.desktop_login.is_none() {
            self.desktop_login = Some(Arc::new(DesktopLoginGate::native()));
        }
        self
    }

    #[cfg(test)]
    fn with_desktop_login_loader(
        session: Arc<BhoonidhiSession>,
        loader: Arc<dyn DesktopLoginLoader>,
    ) -> Self {
        Self {
            session,
            desktop_login: Some(Arc::new(DesktopLoginGate::with_loader(loader))),
            #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
            acceptance_profile: None,
        }
    }

    pub fn session(&self) -> &Arc<BhoonidhiSession> {
        &self.session
    }

    /// Explicit session check: reuse the process owner, never expose a token.
    pub fn check_authentication(&self) -> Result<(), BhoonidhiAuthError> {
        let _credential_guard =
            bhoonidhi_credential_read_guard().map_err(|_| BhoonidhiAuthError::Synchronization)?;
        if let Some(gate) = self.desktop_login.as_ref() {
            self.sync_desktop_credential_revision(gate)?;
            self.ensure_desktop_login_with_retry(true)?;
        }
        self.access_token_from_runtime().map(|_| ())
    }

    /// Returns the current runtime token and its session generation.
    ///
    /// Callers that send a request should retain the generation only in
    /// process memory and pass it to [`Self::refresh_after_rejection`] if that
    /// exact request receives an authentication rejection.
    pub fn access_token_with_generation(&self) -> Result<BhoonidhiAccessToken, BhoonidhiAuthError> {
        let Some(gate) = self.desktop_login.as_ref() else {
            return self.access_token_from_runtime();
        };
        // A successful credential mutation waits for this read guard. Keeping
        // it through the session operation prevents a late grant from crossing
        // the mutation boundary; the revision check invalidates any token
        // installed before the mutation acquired its write lock.
        let _credential_guard =
            bhoonidhi_credential_read_guard().map_err(|_| BhoonidhiAuthError::Synchronization)?;
        self.sync_desktop_credential_revision(gate)?;
        match self.access_token_from_runtime() {
            Ok(token) => Ok(token),
            Err(BhoonidhiAuthError::SessionUnavailable) => {
                self.ensure_desktop_login()?;
                self.access_token_from_runtime()
            }
            Err(error) => Err(error),
        }
    }

    fn access_token_from_runtime(&self) -> Result<BhoonidhiAccessToken, BhoonidhiAuthError> {
        if let Some(error) = self.session.failed_generation_error()? {
            return Err(error);
        }
        match self.session.access_token() {
            Ok(token) => Ok(token),
            Err(BhoonidhiAuthError::Timeout) => {
                self.session.refresh_if_expired()?;
                self.session.access_token()
            }
            Err(error) => Err(error),
        }
    }

    /// Refreshes only when `generation` is still the generation used by the
    /// rejected request.  If another caller already replaced it, the session
    /// returns `AlreadyRefreshed` without another authentication request.
    pub fn refresh_after_rejection(
        &self,
        generation: u64,
    ) -> Result<BhoonidhiRefreshOutcome, BhoonidhiAuthError> {
        if let Some(gate) = self.desktop_login.as_ref() {
            let _credential_guard = bhoonidhi_credential_read_guard()
                .map_err(|_| BhoonidhiAuthError::Synchronization)?;
            if self.sync_desktop_credential_revision(gate)? {
                return Err(BhoonidhiAuthError::SessionChanged);
            }
        }
        self.session.refresh_after_rejection(generation)
    }

    /// Convenience form for callers that retained the token used by a
    /// rejected request.  Only its non-secret generation reaches the session.
    pub fn refresh_after_token_rejection(
        &self,
        token: &BhoonidhiAccessToken,
    ) -> Result<BhoonidhiRefreshOutcome, BhoonidhiAuthError> {
        self.refresh_after_rejection(token.generation())
    }

    /// Logs out the process-owned session and clears its local runtime state.
    pub fn logout(&self) -> Result<(), BhoonidhiAuthError> {
        if let Some(gate) = self.desktop_login.as_ref() {
            let _credential_guard = bhoonidhi_credential_read_guard()
                .map_err(|_| BhoonidhiAuthError::Synchronization)?;
            self.sync_desktop_credential_revision(gate)?;
            let mut state = gate
                .state
                .lock()
                .map_err(|_| BhoonidhiAuthError::Synchronization)?;
            // Disable before clearing the session. A concurrent caller that
            // was waiting for this gate must not re-read credentials after an
            // explicit logout, even if the revocation request fails.
            state.disabled = true;
            state.attempted = true;
            state.failure = None;
        }
        self.session.logout()
    }

    fn sync_desktop_credential_revision(
        &self,
        gate: &DesktopLoginGate,
    ) -> Result<bool, BhoonidhiAuthError> {
        synchronize_desktop_revision(gate, bhoonidhi_credential_revision(), || {
            self.session.invalidate_local()
        })
    }

    fn ensure_desktop_login(&self) -> Result<(), BhoonidhiAuthError> {
        self.ensure_desktop_login_with_retry(false)
    }

    fn ensure_desktop_login_with_retry(&self, explicit: bool) -> Result<(), BhoonidhiAuthError> {
        let Some(gate) = self.desktop_login.as_ref() else {
            return Err(BhoonidhiAuthError::SessionUnavailable);
        };
        self.sync_desktop_credential_revision(gate)?;
        let mut state = gate
            .state
            .lock()
            .map_err(|_| BhoonidhiAuthError::Synchronization)?;
        if state.disabled {
            return Err(BhoonidhiAuthError::SessionChanged);
        }

        if let Some(error) = self.session.failed_generation_error()? {
            if !explicit
                || !matches!(
                    error,
                    BhoonidhiAuthError::Timeout
                        | BhoonidhiAuthError::Transport
                        | BhoonidhiAuthError::Rejected
                        | BhoonidhiAuthError::RefreshUnavailable
                        | BhoonidhiAuthError::RefreshExhausted
                )
            {
                return Err(error);
            }
            admit_explicit_recovery(&mut state)?;
            state.attempted = true;
            state.last_attempt = Some(Instant::now());
            let result = gate
                .loader
                .load()
                .and_then(|material| self.session.reauthenticate_failed_generation(material));
            state.failure = result.as_ref().err().copied();
            return result;
        }

        // A clone may have observed SessionUnavailable just before another
        // clone completed the first login. Re-check while holding the gate so
        // only one password grant is ever issued for this session owner.
        match self.session.access_token() {
            Ok(_) => return Ok(()),
            Err(BhoonidhiAuthError::SessionUnavailable) => {}
            Err(BhoonidhiAuthError::Timeout) => {
                self.session.refresh_if_expired()?;
                return self.session.access_token().map(|_| ());
            }
            Err(error) => return Err(error),
        }
        if state.attempted {
            let retryable = matches!(
                state.failure,
                Some(BhoonidhiAuthError::Timeout | BhoonidhiAuthError::Transport)
            );
            let cooled_down = state
                .last_attempt
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(5));
            let can_auto_retry = retryable && cooled_down && state.recovery_attempts.is_empty();
            if (!explicit && !can_auto_retry) || !retryable {
                return Err(state
                    .failure
                    .unwrap_or(BhoonidhiAuthError::SessionUnavailable));
            }
            if !cooled_down {
                return Err(BhoonidhiAuthError::RateLimited);
            }
            // A failed recovery password grant has no usable old session.
            // Explicit retries still share the recovery budget; automatic
            // lazy login remains blocked by `attempted` above.
            if !state.recovery_attempts.is_empty() {
                admit_explicit_recovery(&mut state)?;
            }
        }
        state.attempted = true;
        state.last_attempt = Some(Instant::now());
        state.failure = None;
        let result = gate
            .loader
            .load()
            .and_then(|material| self.session.login(material).map(|_| ()));
        if let Err(error) = result {
            state.failure = Some(error);
        }
        result
    }
}

fn synchronize_desktop_revision(
    gate: &DesktopLoginGate,
    revision: u64,
    invalidate: impl FnOnce() -> Result<(), BhoonidhiAuthError>,
) -> Result<bool, BhoonidhiAuthError> {
    let mut state = gate
        .state
        .lock()
        .map_err(|_| BhoonidhiAuthError::Synchronization)?;
    if state.observed_revision == revision {
        return Ok(false);
    }
    // Publish the revision only after old tokens are invalidated, while the
    // gate excludes every other revision-aware reader. On failure the gate
    // remains unchanged so a later caller must retry synchronization.
    invalidate()?;
    state.observed_revision = revision;
    state.attempted = false;
    state.failure = None;
    state.disabled = false;
    state.last_attempt = None;
    Ok(true)
}

fn admit_explicit_recovery(state: &mut DesktopLoginState) -> Result<(), BhoonidhiAuthError> {
    admit_explicit_recovery_at(state, Instant::now())
}

fn admit_explicit_recovery_at(
    state: &mut DesktopLoginState,
    now: Instant,
) -> Result<(), BhoonidhiAuthError> {
    const WINDOW: Duration = Duration::from_secs(3600);
    const COOLDOWN: Duration = Duration::from_secs(360);
    while state
        .recovery_attempts
        .front()
        .is_some_and(|at| now.duration_since(*at) >= WINDOW)
    {
        state.recovery_attempts.pop_front();
    }
    if state.recovery_attempts.len() >= 3
        || state
            .recovery_attempts
            .back()
            .is_some_and(|at| now.duration_since(*at) < COOLDOWN)
    {
        return Err(BhoonidhiAuthError::RateLimited);
    }
    state.recovery_attempts.push_back(now);
    Ok(())
}

impl fmt::Debug for BhoonidhiSessionTokenProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BhoonidhiSessionTokenProvider")
            .field("session", &"process-owned")
            .field("runtime_material", &"[REDACTED]")
            .finish()
    }
}

impl BhoonidhiAccessTokenProvider for BhoonidhiSessionTokenProvider {
    fn mark_generation_rejected(&self, generation: u64) -> Result<(), CatalogAuthError> {
        let _guard =
            bhoonidhi_credential_read_guard().map_err(|_| CatalogAuthError::Synchronization)?;
        if let Some(gate) = self.desktop_login.as_ref() {
            if self
                .sync_desktop_credential_revision(gate)
                .map_err(map_catalog_auth_error)?
            {
                return Err(CatalogAuthError::SessionChanged);
            }
        }
        self.session
            .mark_generation_rejected(generation)
            .map_err(map_catalog_auth_error)
    }

    fn access_token(&self) -> Result<Zeroizing<String>, CatalogAuthError> {
        let token = self
            .access_token_with_generation()
            .map_err(map_catalog_auth_error)?;
        Ok(Zeroizing::new(token.as_str().to_owned()))
    }

    fn access_token_with_generation(&self) -> Result<CatalogAccessTokenLease, CatalogAuthError> {
        let token = BhoonidhiSessionTokenProvider::access_token_with_generation(self)
            .map_err(map_catalog_auth_error)?;
        CatalogAccessTokenLease::new(
            Zeroizing::new(token.as_str().to_owned()),
            token.generation(),
        )
        .map_err(|_| CatalogAuthError::InvalidToken)
    }

    fn refresh_after_rejection(
        &self,
        generation: u64,
    ) -> Result<CatalogRefreshResult, CatalogAuthError> {
        let outcome = BhoonidhiSessionTokenProvider::refresh_after_rejection(self, generation)
            .map_err(map_catalog_auth_error)?;
        Ok(match outcome {
            BhoonidhiRefreshOutcome::Refreshed(_) => CatalogRefreshResult::Refreshed,
            BhoonidhiRefreshOutcome::AlreadyRefreshed(_)
            | BhoonidhiRefreshOutcome::NotNeeded(_) => CatalogRefreshResult::AlreadyRefreshed,
        })
    }
}

/// Lets the same process-owned session satisfy the existing download resolver
/// credential-provider seam when a resolver is invoked through a
/// [`crate::asset_resolver::ResolveContext`].
impl CredentialProvider for BhoonidhiSessionTokenProvider {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        if profile != BHOONIDHI_CREDENTIAL_PROFILE {
            return Err(CredentialProviderError::Missing);
        }
        let token = self
            .access_token_with_generation()
            .map_err(map_credential_provider_error)?;
        CredentialMaterial::bearer(token.as_str().to_owned())
    }
}

/// Short alias for callers that prefer to describe the object as a credential
/// provider.  Both names refer to the same adapter and session state.
pub type BhoonidhiRuntimeCredentialProvider = BhoonidhiSessionTokenProvider;

fn map_catalog_auth_error(error: BhoonidhiAuthError) -> CatalogAuthError {
    match error {
        BhoonidhiAuthError::Missing => CatalogAuthError::Missing,
        BhoonidhiAuthError::ActivationRequired => CatalogAuthError::NotActivated,
        BhoonidhiAuthError::CredentialInvalid => CatalogAuthError::CredentialInvalid,
        BhoonidhiAuthError::Rejected => CatalogAuthError::Rejected,
        BhoonidhiAuthError::MaxSessions => CatalogAuthError::MaxSessions,
        BhoonidhiAuthError::RateLimited | BhoonidhiAuthError::LocalRefreshRateLimited => {
            CatalogAuthError::RateLimited
        }
        BhoonidhiAuthError::Transport => CatalogAuthError::Transport,
        BhoonidhiAuthError::Timeout => CatalogAuthError::Timeout,
        BhoonidhiAuthError::ResponseInvalid => CatalogAuthError::ResponseInvalid,
        BhoonidhiAuthError::RefreshUnavailable => CatalogAuthError::RefreshUnavailable,
        BhoonidhiAuthError::RefreshExhausted => CatalogAuthError::RefreshExhausted,
        BhoonidhiAuthError::SessionChanged => CatalogAuthError::SessionChanged,
        BhoonidhiAuthError::SessionUnavailable => CatalogAuthError::SessionUnavailable,
        BhoonidhiAuthError::Synchronization => CatalogAuthError::Synchronization,
        BhoonidhiAuthError::Configuration => CatalogAuthError::Configuration,
    }
}

fn map_credential_provider_error(error: BhoonidhiAuthError) -> CredentialProviderError {
    match error {
        BhoonidhiAuthError::Missing => CredentialProviderError::Missing,
        BhoonidhiAuthError::Timeout => CredentialProviderError::Timeout,
        BhoonidhiAuthError::ActivationRequired
        | BhoonidhiAuthError::CredentialInvalid
        | BhoonidhiAuthError::Rejected
        | BhoonidhiAuthError::MaxSessions
        | BhoonidhiAuthError::RateLimited
        | BhoonidhiAuthError::LocalRefreshRateLimited
        | BhoonidhiAuthError::Transport
        | BhoonidhiAuthError::ResponseInvalid
        | BhoonidhiAuthError::RefreshUnavailable
        | BhoonidhiAuthError::RefreshExhausted
        | BhoonidhiAuthError::SessionChanged
        | BhoonidhiAuthError::SessionUnavailable
        | BhoonidhiAuthError::Synchronization
        | BhoonidhiAuthError::Configuration => CredentialProviderError::Failed,
    }
}

fn map_desktop_credential_error(error: BhoonidhiCredentialError) -> BhoonidhiAuthError {
    match error {
        BhoonidhiCredentialError::Missing => BhoonidhiAuthError::Missing,
        BhoonidhiCredentialError::Unavailable => BhoonidhiAuthError::SessionUnavailable,
        BhoonidhiCredentialError::Invalid => BhoonidhiAuthError::CredentialInvalid,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Barrier, Mutex};
    use std::thread;
    use std::time::{Duration, SystemTime};

    use insar_core::ProviderNetworkPolicy;

    use super::*;
    use crate::{BhoonidhiLoginMaterial, BhoonidhiTokenEndpoint, BhoonidhiTokenGrant};

    struct FixtureClock;

    impl crate::Clock for FixtureClock {
        fn now(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH
        }
    }

    struct AdjustableClock {
        seconds: AtomicU64,
    }

    impl AdjustableClock {
        fn new(seconds: u64) -> Self {
            Self {
                seconds: AtomicU64::new(seconds),
            }
        }

        fn set(&self, seconds: u64) {
            self.seconds.store(seconds, Ordering::SeqCst);
        }
    }

    impl crate::Clock for AdjustableClock {
        fn now(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(self.seconds.load(Ordering::SeqCst))
        }
    }

    struct FixtureEndpoint {
        policy: ProviderNetworkPolicy,
        password_calls: AtomicUsize,
        refresh_calls: AtomicUsize,
        password_failure: Mutex<Option<BhoonidhiAuthError>>,
        refresh_failure: Mutex<Option<BhoonidhiAuthError>>,
        logout_calls: AtomicUsize,
        logout_failure: Mutex<Option<BhoonidhiAuthError>>,
    }

    impl FixtureEndpoint {
        fn new(policy: ProviderNetworkPolicy) -> Self {
            Self {
                policy,
                password_calls: AtomicUsize::new(0),
                refresh_calls: AtomicUsize::new(0),
                password_failure: Mutex::new(None),
                refresh_failure: Mutex::new(None),
                logout_calls: AtomicUsize::new(0),
                logout_failure: Mutex::new(None),
            }
        }
    }

    impl BhoonidhiTokenEndpoint for FixtureEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn password_grant(
            &self,
            _material: &BhoonidhiLoginMaterial,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            self.password_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = *self.password_failure.lock().unwrap() {
                return Err(error);
            }
            BhoonidhiTokenGrant::new(
                "fixture-access-one",
                Some("fixture-refresh-one".to_owned()),
                Some(1200),
            )
        }

        fn refresh_grant(
            &self,
            _user_id: &str,
            _refresh_token: &str,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            self.refresh_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = *self.refresh_failure.lock().unwrap() {
                return Err(error);
            }
            BhoonidhiTokenGrant::new(
                "fixture-access-two",
                Some("fixture-refresh-two".to_owned()),
                Some(1200),
            )
        }

        fn logout(&self, _refresh_token: &str) -> Result<(), BhoonidhiAuthError> {
            self.logout_calls.fetch_add(1, Ordering::SeqCst);
            self.logout_failure.lock().unwrap().map_or(Ok(()), Err)
        }
    }

    #[derive(Clone, Copy)]
    enum TestLoaderState {
        Missing,
        Ready,
        Failure(BhoonidhiAuthError),
    }

    struct TestLoginLoader {
        state: Mutex<TestLoaderState>,
        calls: AtomicUsize,
    }

    impl TestLoginLoader {
        fn new(state: TestLoaderState) -> Self {
            Self {
                state: Mutex::new(state),
                calls: AtomicUsize::new(0),
            }
        }

        fn set(&self, state: TestLoaderState) {
            *self.state.lock().expect("test loader state") = state;
        }
    }

    impl DesktopLoginLoader for TestLoginLoader {
        fn load(&self) -> Result<BhoonidhiLoginMaterial, BhoonidhiAuthError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match *self.state.lock().expect("test loader state") {
                TestLoaderState::Missing => Err(BhoonidhiAuthError::Missing),
                TestLoaderState::Failure(error) => Err(error),
                TestLoaderState::Ready => {
                    BhoonidhiLoginMaterial::new("fixture-user", "fixture-password")
                }
            }
        }
    }

    static CREDENTIAL_RECOVERY_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    #[test]
    fn acceptance_revoke_retains_generation_and_requires_explicit_recovery() {
        let _guard = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Ready));
        let profile = std::env::current_dir().unwrap().canonicalize().unwrap();
        let provider = lazy_provider(loader, endpoint.clone())
            .with_auth_revoke_acceptance_profile(profile.clone())
            .unwrap();
        provider.check_authentication().unwrap();
        let before = provider.session.snapshot().unwrap();
        let generation = provider.acceptance_current_generation(&profile).unwrap();
        provider
            .acceptance_revoke_current_generation(&profile, generation)
            .unwrap();
        assert_eq!(provider.session.snapshot().unwrap(), before);
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            provider
                .clone()
                .acceptance_revoke_current_generation(&profile, generation),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        *endpoint.refresh_failure.lock().unwrap() = Some(BhoonidhiAuthError::Rejected);
        assert!(provider.refresh_after_rejection(generation).is_err());
        assert!(!provider.session.snapshot().unwrap().authenticated);
        assert!(provider.access_token_with_generation().is_err());
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
        *endpoint.logout_failure.lock().unwrap() = Some(BhoonidhiAuthError::Rejected);
        provider.check_authentication().unwrap();
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 2);
        let restored = provider.acceptance_current_generation(&profile).unwrap();
        assert_ne!(generation, restored);
        assert_eq!(
            provider.acceptance_revoke_current_generation(&profile, restored),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 2);
    }

    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    #[test]
    fn acceptance_revoke_rejects_unbound_wrong_profile_and_stale_generation_before_io() {
        let _guard = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let profile = std::env::current_dir().unwrap().canonicalize().unwrap();
        let provider = lazy_provider(
            Arc::new(TestLoginLoader::new(TestLoaderState::Ready)),
            endpoint.clone(),
        );
        provider.check_authentication().unwrap();
        let generation = provider.session.snapshot().unwrap().generation.unwrap();
        assert_eq!(
            provider.acceptance_revoke_current_generation(&profile, generation),
            Err(BhoonidhiAuthError::Configuration)
        );
        let provider = provider
            .with_auth_revoke_acceptance_profile(profile.clone())
            .unwrap();
        assert_eq!(
            provider.acceptance_revoke_current_generation(profile.parent().unwrap(), generation),
            Err(BhoonidhiAuthError::Configuration)
        );
        assert_eq!(
            provider.acceptance_revoke_current_generation(&profile, generation + 1),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 0);
        provider
            .acceptance_revoke_current_generation(&profile, generation)
            .unwrap();
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    #[test]
    fn acceptance_revoke_unknown_outcome_consumes_once_without_local_mutation() {
        let _guard = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        *endpoint.logout_failure.lock().unwrap() = Some(BhoonidhiAuthError::Timeout);
        let profile = std::env::current_dir().unwrap().canonicalize().unwrap();
        let provider = lazy_provider(
            Arc::new(TestLoginLoader::new(TestLoaderState::Ready)),
            endpoint.clone(),
        )
        .with_auth_revoke_acceptance_profile(profile.clone())
        .unwrap();
        provider.check_authentication().unwrap();
        let before = provider.session.snapshot().unwrap();
        let generation = before.generation.unwrap();
        assert_eq!(
            provider.acceptance_revoke_current_generation(&profile, generation),
            Err(BhoonidhiAuthError::Timeout)
        );
        assert_eq!(provider.session.snapshot().unwrap(), before);
        assert_eq!(
            provider.acceptance_revoke_current_generation(&profile, generation),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
    }

    fn lazy_provider(
        loader: Arc<TestLoginLoader>,
        endpoint: Arc<FixtureEndpoint>,
    ) -> BhoonidhiSessionTokenProvider {
        let policy = ProviderNetworkPolicy::system();
        let session =
            Arc::new(BhoonidhiSession::new(policy, endpoint, Arc::new(FixtureClock)).unwrap());
        BhoonidhiSessionTokenProvider::with_desktop_login_loader(session, loader)
    }

    fn mark_credentials_changed() {
        crate::credentials::mark_bhoonidhi_credentials_changed_for_test();
    }

    fn provider() -> (BhoonidhiSessionTokenProvider, Arc<FixtureEndpoint>) {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = Arc::new(
            BhoonidhiSession::new(policy, endpoint.clone(), Arc::new(FixtureClock)).unwrap(),
        );
        session
            .login(BhoonidhiLoginMaterial::new("fixture-user", "fixture-password").unwrap())
            .unwrap();
        (BhoonidhiSessionTokenProvider::new(session), endpoint)
    }

    #[test]
    fn auth_recovery_explicit_check_replaces_failed_generation_once() {
        let _lock = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Ready));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(loader, endpoint.clone());
        let generation = provider
            .access_token_with_generation()
            .unwrap()
            .generation();
        *endpoint.refresh_failure.lock().unwrap() = Some(BhoonidhiAuthError::Rejected);
        assert_eq!(
            provider.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::Rejected)
        );
        provider.check_authentication().unwrap();
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
        assert!(
            provider
                .access_token_with_generation()
                .unwrap()
                .generation()
                > generation
        );
    }

    #[test]
    fn auth_recovery_unknown_revocation_blocks_password_grant() {
        let _lock = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Ready));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(loader, endpoint.clone());
        let generation = provider
            .access_token_with_generation()
            .unwrap()
            .generation();
        *endpoint.refresh_failure.lock().unwrap() = Some(BhoonidhiAuthError::Rejected);
        *endpoint.logout_failure.lock().unwrap() = Some(BhoonidhiAuthError::Timeout);
        assert_eq!(
            provider.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::Rejected)
        );
        assert_eq!(
            provider.check_authentication(),
            Err(BhoonidhiAuthError::Timeout)
        );
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            provider.session().snapshot().unwrap().generation,
            Some(generation)
        );
        assert_eq!(
            provider
                .session()
                .last_explicit_recovery_revocation_confirmed()
                .unwrap(),
            Some(false)
        );
        assert_eq!(
            provider.check_authentication(),
            Err(BhoonidhiAuthError::RateLimited)
        );
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn auth_recovery_rejected_revocation_is_not_reported_as_confirmed() {
        let _lock = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(
            Arc::new(TestLoginLoader::new(TestLoaderState::Ready)),
            endpoint.clone(),
        );
        let old = provider
            .access_token_with_generation()
            .unwrap()
            .generation();
        provider.session().mark_generation_rejected(old).unwrap();
        *endpoint.logout_failure.lock().unwrap() = Some(BhoonidhiAuthError::Rejected);
        provider.check_authentication().unwrap();
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            provider
                .session()
                .last_explicit_recovery_revocation_confirmed()
                .unwrap(),
            Some(false)
        );
        assert!(
            provider
                .access_token_with_generation()
                .unwrap()
                .generation()
                > old
        );
    }

    #[test]
    fn auth_recovery_automatic_access_never_logs_in_and_terminal_causes_remain() {
        let _lock = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        for cause in [
            BhoonidhiAuthError::Timeout,
            BhoonidhiAuthError::Transport,
            BhoonidhiAuthError::MaxSessions,
            BhoonidhiAuthError::RateLimited,
            BhoonidhiAuthError::ActivationRequired,
            BhoonidhiAuthError::ResponseInvalid,
        ] {
            let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
            let provider = lazy_provider(
                Arc::new(TestLoginLoader::new(TestLoaderState::Ready)),
                endpoint.clone(),
            );
            let old = provider
                .access_token_with_generation()
                .unwrap()
                .generation();
            *endpoint.refresh_failure.lock().unwrap() = Some(cause);
            assert_eq!(provider.refresh_after_rejection(old), Err(cause));
            let transient = matches!(
                cause,
                BhoonidhiAuthError::Timeout | BhoonidhiAuthError::Transport
            );
            // Design 2026-10-17 §4 changes the former single-attempt rule for
            // transient causes: the generation stays refreshable and fails only
            // on its third grant, keeping that last real cause. Terminal causes
            // are unchanged.
            let grants = if transient {
                for _ in 0..2 {
                    assert_eq!(provider.refresh_after_rejection(old), Err(cause));
                }
                assert_eq!(
                    provider.refresh_after_rejection(old),
                    Err(BhoonidhiAuthError::RefreshExhausted)
                );
                3
            } else {
                provider.session().mark_generation_rejected(old).unwrap();
                1
            };
            for _ in 0..3 {
                assert_eq!(provider.access_token_with_generation().unwrap_err(), cause);
            }
            if !transient {
                assert_eq!(provider.check_authentication(), Err(cause));
            }
            assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
            assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), grants);
            assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn auth_recovery_concurrent_checks_share_one_reauthentication() {
        let _lock = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(
            Arc::new(TestLoginLoader::new(TestLoaderState::Ready)),
            endpoint.clone(),
        );
        let old = provider
            .access_token_with_generation()
            .unwrap()
            .generation();
        provider.session().mark_generation_rejected(old).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let provider = provider.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    provider.check_authentication()
                })
            })
            .collect();
        barrier.wait();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
        provider.logout().unwrap();
        assert_eq!(
            provider.check_authentication(),
            Err(BhoonidhiAuthError::SessionChanged)
        );
    }

    #[test]
    fn auth_recovery_revision_sync_holds_gate_until_invalidation_finishes() {
        let gate =
            DesktopLoginGate::with_loader(Arc::new(TestLoginLoader::new(TestLoaderState::Ready)));
        let revision = gate.state.lock().unwrap().observed_revision + 1;
        assert!(synchronize_desktop_revision(&gate, revision, || {
            assert!(
                matches!(
                    gate.state.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "a concurrent reader must not observe the new revision before invalidation"
            );
            Ok(())
        })
        .unwrap());
        assert_eq!(gate.state.lock().unwrap().observed_revision, revision);
        assert!(
            !synchronize_desktop_revision(&gate, revision, || panic!("already synchronized"))
                .unwrap()
        );
    }

    #[test]
    fn auth_recovery_revision_sync_failure_keeps_previous_gate_state() {
        let gate =
            DesktopLoginGate::with_loader(Arc::new(TestLoginLoader::new(TestLoaderState::Ready)));
        let revision = {
            let mut state = gate.state.lock().unwrap();
            state.attempted = true;
            state.disabled = true;
            state.failure = Some(BhoonidhiAuthError::Rejected);
            state.observed_revision + 1
        };
        assert_eq!(
            synchronize_desktop_revision(&gate, revision, || Err(
                BhoonidhiAuthError::Synchronization
            )),
            Err(BhoonidhiAuthError::Synchronization)
        );
        let state = gate.state.lock().unwrap();
        assert_eq!(state.observed_revision, revision - 1);
        assert!(state.attempted && state.disabled);
        assert_eq!(state.failure, Some(BhoonidhiAuthError::Rejected));
    }

    #[test]
    fn auth_recovery_budget_is_rolling_and_counts_failed_admissions() {
        let mut state = DesktopLoginState::new();
        admit_explicit_recovery(&mut state).unwrap();
        assert_eq!(
            admit_explicit_recovery(&mut state),
            Err(BhoonidhiAuthError::RateLimited)
        );
        // Two hours ahead: an Instant an hour in the past cannot be represented on a host booted under an hour ago.
        let now = Instant::now() + Duration::from_secs(7200);
        state.recovery_attempts = VecDeque::from([
            now - Duration::from_secs(1200),
            now - Duration::from_secs(800),
            now - Duration::from_secs(400),
        ]);
        assert_eq!(
            admit_explicit_recovery_at(&mut state, now),
            Err(BhoonidhiAuthError::RateLimited)
        );
        state.recovery_attempts[0] = now - Duration::from_secs(3601);
        admit_explicit_recovery_at(&mut state, now).unwrap();
        assert_eq!(state.recovery_attempts.len(), 3);
    }

    #[test]
    fn auth_recovery_password_failure_stays_typed_and_never_auto_retries() {
        let _lock = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        for cause in [
            BhoonidhiAuthError::MaxSessions,
            BhoonidhiAuthError::RateLimited,
            BhoonidhiAuthError::Timeout,
        ] {
            let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
            let provider = lazy_provider(
                Arc::new(TestLoginLoader::new(TestLoaderState::Ready)),
                endpoint.clone(),
            );
            let old = provider
                .access_token_with_generation()
                .unwrap()
                .generation();
            provider.session().mark_generation_rejected(old).unwrap();
            *endpoint.password_failure.lock().unwrap() = Some(cause);
            assert_eq!(provider.check_authentication(), Err(cause));
            assert_eq!(provider.access_token_with_generation().unwrap_err(), cause);
            let next = provider.check_authentication().unwrap_err();
            assert_eq!(
                next,
                if cause == BhoonidhiAuthError::Timeout {
                    BhoonidhiAuthError::RateLimited
                } else {
                    cause
                }
            );
            assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 2);
            assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn expired_access_token_is_refreshed_before_the_next_request() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let clock = Arc::new(AdjustableClock::new(0));
        let session =
            Arc::new(BhoonidhiSession::new(policy, endpoint.clone(), clock.clone()).unwrap());
        session
            .login(BhoonidhiLoginMaterial::new("fixture-user", "fixture-password").unwrap())
            .unwrap();

        clock.set(1201);
        let expired = session.snapshot().unwrap();
        assert!(!expired.authenticated);
        assert!(expired.refresh_available);
        assert_eq!(expired.expires_in_seconds, Some(0));

        let provider = BhoonidhiSessionTokenProvider::new(session);
        let refreshed = provider.access_token_with_generation().unwrap();
        assert_eq!(refreshed.as_str(), "fixture-access-two");
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);

        let reused = provider.access_token_with_generation().unwrap();
        assert_eq!(reused.as_str(), "fixture-access-two");
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);
    }

    /// Design 2026-10-17 §4: an expired token whose refresh timed out is
    /// refreshed again on the next request instead of failing permanently.
    #[test]
    fn expired_access_token_refresh_retries_after_a_transient_failure() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let clock = Arc::new(AdjustableClock::new(0));
        let session =
            Arc::new(BhoonidhiSession::new(policy, endpoint.clone(), clock.clone()).unwrap());
        session
            .login(BhoonidhiLoginMaterial::new("fixture-user", "fixture-password").unwrap())
            .unwrap();
        clock.set(1201);
        let provider = BhoonidhiSessionTokenProvider::new(session);
        *endpoint.refresh_failure.lock().unwrap() = Some(BhoonidhiAuthError::Timeout);
        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Timeout
        );
        *endpoint.refresh_failure.lock().unwrap() = None;
        let refreshed = provider.access_token_with_generation().unwrap();
        assert_eq!(refreshed.as_str(), "fixture-access-two");
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn credential_revision_recovers_cached_missing_with_the_same_provider() {
        let _test_lock = CREDENTIAL_RECOVERY_TEST_LOCK
            .lock()
            .expect("credential recovery test lock");
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Missing));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(Arc::clone(&loader), Arc::clone(&endpoint));

        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Missing
        );
        loader.set(TestLoaderState::Ready);
        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Missing
        );

        mark_credentials_changed();
        let token = provider
            .access_token_with_generation()
            .expect("same provider recovers after credential save");
        assert_eq!(token.as_str(), "fixture-access-one");
        assert_eq!(loader.calls.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn explicit_auth_retries_transient_failure_only_after_cooldown() {
        let _lock = CREDENTIAL_RECOVERY_TEST_LOCK.lock().unwrap();
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Failure(
            BhoonidhiAuthError::Timeout,
        )));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(Arc::clone(&loader), Arc::clone(&endpoint));
        assert_eq!(
            provider.check_authentication(),
            Err(BhoonidhiAuthError::Timeout)
        );
        loader.set(TestLoaderState::Ready);
        assert_eq!(
            provider.check_authentication(),
            Err(BhoonidhiAuthError::RateLimited)
        );
        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Timeout
        );
        provider
            .desktop_login
            .as_ref()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .last_attempt = Some(Instant::now() - Duration::from_secs(6));
        provider.check_authentication().unwrap();
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
        provider.logout().unwrap();
        assert_eq!(
            provider.check_authentication(),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn credential_revision_recovers_a_rejected_login_after_new_credentials() {
        let _test_lock = CREDENTIAL_RECOVERY_TEST_LOCK
            .lock()
            .expect("credential recovery test lock");
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Failure(
            BhoonidhiAuthError::Rejected,
        )));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(Arc::clone(&loader), Arc::clone(&endpoint));

        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Rejected
        );
        loader.set(TestLoaderState::Ready);
        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Rejected
        );

        mark_credentials_changed();
        assert_eq!(
            provider
                .access_token_with_generation()
                .expect("replacement credentials recover")
                .as_str(),
            "fixture-access-one"
        );
        assert_eq!(loader.calls.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_credential_save_does_not_clear_cached_auth_failure() {
        let _test_lock = CREDENTIAL_RECOVERY_TEST_LOCK
            .lock()
            .expect("credential recovery test lock");
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Failure(
            BhoonidhiAuthError::Rejected,
        )));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(Arc::clone(&loader), Arc::clone(&endpoint));

        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Rejected
        );
        loader.set(TestLoaderState::Ready);
        assert_eq!(
            crate::store_desktop_bhoonidhi_credentials("", "fixture-password").unwrap_err(),
            crate::bhoonidhi_credentials::BhoonidhiCredentialError::Invalid
        );
        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Rejected
        );
        assert_eq!(loader.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cloned_providers_share_one_lazy_login_flight() {
        let _test_lock = CREDENTIAL_RECOVERY_TEST_LOCK
            .lock()
            .expect("credential recovery test lock");
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Ready));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(Arc::clone(&loader), Arc::clone(&endpoint));
        let barrier = Arc::new(Barrier::new(3));
        let first_barrier = Arc::clone(&barrier);
        let first_provider = provider.clone();
        let first = thread::spawn(move || {
            first_barrier.wait();
            first_provider.access_token_with_generation()
        });
        let second_barrier = Arc::clone(&barrier);
        let second_provider = provider.clone();
        let second = thread::spawn(move || {
            second_barrier.wait();
            second_provider.access_token_with_generation()
        });
        barrier.wait();

        assert_eq!(
            first.join().expect("first login thread").unwrap().as_str(),
            "fixture-access-one"
        );
        assert_eq!(
            second
                .join()
                .expect("second login thread")
                .unwrap()
                .as_str(),
            "fixture-access-one"
        );
        assert_eq!(loader.calls.load(Ordering::SeqCst), 1);
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn clearing_credentials_invalidates_the_old_token_before_next_access() {
        let _test_lock = CREDENTIAL_RECOVERY_TEST_LOCK
            .lock()
            .expect("credential recovery test lock");
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Ready));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(Arc::clone(&loader), Arc::clone(&endpoint));
        let token = provider
            .access_token_with_generation()
            .expect("initial login");
        assert_eq!(token.as_str(), "fixture-access-one");

        loader.set(TestLoaderState::Missing);
        mark_credentials_changed();
        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::Missing
        );
        assert!(!provider.session().snapshot().unwrap().authenticated);
    }

    #[test]
    fn explicit_logout_does_not_auto_relogin() {
        let _test_lock = CREDENTIAL_RECOVERY_TEST_LOCK
            .lock()
            .expect("credential recovery test lock");
        let loader = Arc::new(TestLoginLoader::new(TestLoaderState::Ready));
        let endpoint = Arc::new(FixtureEndpoint::new(ProviderNetworkPolicy::system()));
        let provider = lazy_provider(Arc::clone(&loader), Arc::clone(&endpoint));
        provider
            .access_token_with_generation()
            .expect("initial login");
        provider.logout().expect("explicit logout");

        assert_eq!(
            provider.access_token_with_generation().unwrap_err(),
            BhoonidhiAuthError::SessionChanged
        );
        assert_eq!(loader.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn catalog_adapter_returns_zeroizing_token_without_debug_leak() {
        let provider = provider().0;
        let token = <BhoonidhiSessionTokenProvider as BhoonidhiAccessTokenProvider>::access_token(
            &provider,
        )
        .unwrap();
        assert_eq!(token.as_str(), "fixture-access-one");
        assert!(!format!("{provider:?}").contains("fixture-access-one"));
    }

    #[test]
    fn refresh_is_bound_to_the_rejected_generation() {
        let (provider, endpoint) = provider();
        let token = provider.access_token_with_generation().unwrap();
        let outcome = provider.refresh_after_token_rejection(&token).unwrap();
        assert!(matches!(outcome, BhoonidhiRefreshOutcome::Refreshed(_)));
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);

        let newer = provider.access_token_with_generation().unwrap();
        assert_eq!(newer.as_str(), "fixture-access-two");
        let already = provider
            .refresh_after_rejection(token.generation())
            .unwrap();
        assert!(matches!(
            already,
            BhoonidhiRefreshOutcome::AlreadyRefreshed(_)
        ));
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn resolver_credential_profile_is_closed() {
        let provider = provider().0;
        assert!(provider.resolve(BHOONIDHI_CREDENTIAL_PROFILE).is_ok());
        assert_eq!(
            provider.resolve("other-profile").unwrap_err(),
            CredentialProviderError::Missing
        );
    }
}
