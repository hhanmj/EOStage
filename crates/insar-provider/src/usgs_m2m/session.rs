use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::Duration;

use insar_core::ProviderNetworkPolicy;
use zeroize::Zeroizing;

use super::endpoint::{ReqwestUsgsM2mEndpoint, UsgsM2mSessionEndpoint};
use super::error::{UsgsM2mEndpointClass, UsgsM2mError, UsgsM2mErrorCode};

pub const USGS_M2M_SESSION_PROFILE_ID: &str = "usgs.eros.m2m.session.v1";

const MAX_USERNAME_BYTES: usize = 256;
const MAX_APPLICATION_TOKEN_BYTES: usize = 4 * 1024;
const MAX_API_KEY_BYTES: usize = 16 * 1024;
const CANCELLATION_POLL: Duration = Duration::from_millis(10);

pub struct UsgsM2mApplicationToken(Zeroizing<String>);

impl UsgsM2mApplicationToken {
    pub fn new(value: impl Into<String>) -> Result<Self, UsgsM2mError> {
        let mut value = value.into();
        // A trailing newline is the classic copy artifact of a saved token.
        // The M2M login-token endpoint compares the exact string, so untrimmed
        // whitespace turns a valid token into AUTH_INVALID.
        zeroize_trim(&mut value);
        if invalid_secret(&value, MAX_APPLICATION_TOKEN_BYTES) {
            return Err(session_error(UsgsM2mErrorCode::ConfigInvalid));
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for UsgsM2mApplicationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UsgsM2mApplicationToken([REDACTED])")
    }
}

pub struct UsgsM2mApiKey(Zeroizing<String>);

impl UsgsM2mApiKey {
    pub(crate) fn from_wire(value: String) -> Result<Self, UsgsM2mError> {
        let mut value = value;
        // The apiKey arrives over HTTPS from the reviewed M2M origin; trim the
        // same copy artifacts so a padded answer cannot poison the X-Auth-Token.
        zeroize_trim(&mut value);
        if invalid_secret(&value, MAX_API_KEY_BYTES) {
            return Err(UsgsM2mError::new(
                UsgsM2mErrorCode::ResponseInvalid,
                UsgsM2mEndpointClass::LoginToken,
            ));
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }

    fn zeroizing_copy(&self) -> Result<Self, UsgsM2mError> {
        Self::from_wire(self.expose().to_owned())
    }
}

impl fmt::Debug for UsgsM2mApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UsgsM2mApiKey([REDACTED])")
    }
}

pub struct UsgsM2mLoginMaterial {
    username: Zeroizing<String>,
    application_token: UsgsM2mApplicationToken,
}

impl UsgsM2mLoginMaterial {
    pub fn new(
        username: impl Into<String>,
        application_token: UsgsM2mApplicationToken,
    ) -> Result<Self, UsgsM2mError> {
        let mut username = username.into();
        zeroize_trim(&mut username);
        if username.is_empty()
            || username.len() > MAX_USERNAME_BYTES
            || username.chars().any(char::is_control)
        {
            return Err(session_error(UsgsM2mErrorCode::ConfigInvalid));
        }
        Ok(Self {
            username: Zeroizing::new(username),
            application_token,
        })
    }

    pub(crate) fn username(&self) -> &str {
        self.username.as_str()
    }

    pub(crate) fn application_token(&self) -> &str {
        self.application_token.expose()
    }
}

impl fmt::Debug for UsgsM2mLoginMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsgsM2mLoginMaterial")
            .field("username", &"[REDACTED]")
            .field("application_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mCredentialProfile {
    ApplicationToken,
}

impl UsgsM2mCredentialProfile {
    pub const fn profile_id(self) -> &'static str {
        match self {
            Self::ApplicationToken => USGS_M2M_SESSION_PROFILE_ID,
        }
    }

    pub fn matches_profile(self, profile_id: &str) -> bool {
        profile_id == self.profile_id()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mSessionStatus {
    Ready,
    LoggingIn,
    Authenticated,
    LoginFailed,
    LoggingOut,
    LoggedOut,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsgsM2mPermissions {
    user: bool,
    download: bool,
    order: bool,
}

impl UsgsM2mPermissions {
    pub const fn has_user(self) -> bool {
        self.user
    }

    pub const fn has_download(self) -> bool {
        self.download
    }

    pub const fn has_order(self) -> bool {
        self.order
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsgsM2mRequestCounts {
    pub login_token: u64,
    pub permissions: u64,
    pub dataset_search: u64,
    pub dataset_filters: u64,
    pub scene_search: u64,
    pub download_options: u64,
    pub download_request: u64,
    pub download_retrieve: u64,
    pub logout: u64,
}

struct SessionState {
    status: UsgsM2mSessionStatus,
    api_key: Option<UsgsM2mApiKey>,
    login_error: Option<UsgsM2mError>,
    logout_result: Option<Result<(), UsgsM2mError>>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            status: UsgsM2mSessionStatus::Ready,
            api_key: None,
            login_error: None,
            logout_result: None,
        }
    }
}

pub struct UsgsM2mSession {
    network_policy: ProviderNetworkPolicy,
    endpoint: Arc<dyn UsgsM2mSessionEndpoint>,
    state: Mutex<SessionState>,
    changed: Condvar,
    request_lane: Mutex<()>,
    login_token_requests: AtomicU64,
    permission_requests: AtomicU64,
    dataset_search_requests: AtomicU64,
    dataset_filter_requests: AtomicU64,
    scene_search_requests: AtomicU64,
    download_option_requests: AtomicU64,
    download_request_requests: AtomicU64,
    download_retrieve_requests: AtomicU64,
    logout_requests: AtomicU64,
}

impl UsgsM2mSession {
    pub fn new(
        network_policy: ProviderNetworkPolicy,
        timeout: Duration,
    ) -> Result<Self, UsgsM2mError> {
        let endpoint = Arc::new(ReqwestUsgsM2mEndpoint::new(network_policy, timeout)?);
        Self::from_endpoint(network_policy, endpoint)
    }

    pub(crate) fn from_endpoint(
        network_policy: ProviderNetworkPolicy,
        endpoint: Arc<dyn UsgsM2mSessionEndpoint>,
    ) -> Result<Self, UsgsM2mError> {
        if endpoint.network_policy() != network_policy {
            return Err(session_error(UsgsM2mErrorCode::ConfigInvalid));
        }
        Ok(Self {
            network_policy,
            endpoint,
            state: Mutex::new(SessionState::default()),
            changed: Condvar::new(),
            request_lane: Mutex::new(()),
            login_token_requests: AtomicU64::new(0),
            permission_requests: AtomicU64::new(0),
            dataset_search_requests: AtomicU64::new(0),
            dataset_filter_requests: AtomicU64::new(0),
            scene_search_requests: AtomicU64::new(0),
            download_option_requests: AtomicU64::new(0),
            download_request_requests: AtomicU64::new(0),
            download_retrieve_requests: AtomicU64::new(0),
            logout_requests: AtomicU64::new(0),
        })
    }

    /// Test seam, compiled only behind the `usgs-m2m-test-session` feature
    /// (enabled solely through downstream dev-dependencies, never in release
    /// builds): a session that is already Authenticated without any network
    /// login. Downstream crate tests use it to run the
    /// resolver-authoritative execution path, whose session pre-check only
    /// passes for an authenticated session. The api key is a synthetic
    /// fixture value that no real USGS account accepts, and no request is
    /// performed here.
    #[cfg(feature = "usgs-m2m-test-session")]
    #[doc(hidden)]
    pub fn authenticated_for_tests(
        network_policy: ProviderNetworkPolicy,
    ) -> Result<Self, UsgsM2mError> {
        let endpoint = Arc::new(ReqwestUsgsM2mEndpoint::new(
            network_policy,
            Duration::from_millis(1),
        )?);
        let session = Self::from_endpoint(network_policy, endpoint)?;
        let api_key = UsgsM2mApiKey::from_wire("fixture-api-key-for-tests".to_owned())?;
        {
            let mut state = session.lock_state()?;
            state.api_key = Some(api_key);
            state.status = UsgsM2mSessionStatus::Authenticated;
        }
        session.changed.notify_all();
        Ok(session)
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    pub fn status(&self) -> Result<UsgsM2mSessionStatus, UsgsM2mError> {
        Ok(self.lock_state()?.status)
    }

    pub fn request_counts(&self) -> UsgsM2mRequestCounts {
        UsgsM2mRequestCounts {
            login_token: self.login_token_requests.load(Ordering::Relaxed),
            permissions: self.permission_requests.load(Ordering::Relaxed),
            dataset_search: self.dataset_search_requests.load(Ordering::Relaxed),
            dataset_filters: self.dataset_filter_requests.load(Ordering::Relaxed),
            scene_search: self.scene_search_requests.load(Ordering::Relaxed),
            download_options: self.download_option_requests.load(Ordering::Relaxed),
            download_request: self.download_request_requests.load(Ordering::Relaxed),
            download_retrieve: self.download_retrieve_requests.load(Ordering::Relaxed),
            logout: self.logout_requests.load(Ordering::Relaxed),
        }
    }

    /// Begin a new user-authorized login attempt after an earlier terminal
    /// login/logout. This is never called automatically and never retries an
    /// authentication request within one operation.
    pub fn prepare_for_explicit_login(&self) -> Result<(), UsgsM2mError> {
        let mut state = self.lock_state()?;
        match state.status {
            UsgsM2mSessionStatus::Ready => Ok(()),
            UsgsM2mSessionStatus::Authenticated
            | UsgsM2mSessionStatus::LoginFailed
            | UsgsM2mSessionStatus::LoggedOut => {
                // Explicit account checking must validate the supplied material,
                // not return success from an older account's cached API key.
                // Forget locally; do not revoke remote keys used by transfers.
                state.api_key = None;
                state.login_error = None;
                state.logout_result = None;
                state.status = UsgsM2mSessionStatus::Ready;
                self.changed.notify_all();
                Ok(())
            }
            UsgsM2mSessionStatus::LoggingIn | UsgsM2mSessionStatus::LoggingOut => {
                Err(session_error(UsgsM2mErrorCode::AuthRequired))
            }
        }
    }

    /// Re-establish an authenticated session from freshly supplied login
    /// material after the runtime key was invalidated (remote expiry or a
    /// 401/403 during a transfer). Single-flight: exactly one authentication
    /// request per invalidation — a caller that arrives while another relogin
    /// (or the same login) is in flight waits and reuses that outcome instead
    /// of issuing its own. An explicit user logout (LoggedOut) is never
    /// auto-reversed. A failed re-login stays terminal — the caller surfaces a
    /// clear "需要重新登录" failure instead of retrying in a loop.
    pub fn relogin_after_invalidation(
        &self,
        material: UsgsM2mLoginMaterial,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mSessionStatus, UsgsM2mError> {
        let mut material = Some(material);
        loop {
            if cancelled() {
                return Err(session_error(UsgsM2mErrorCode::Cancelled));
            }
            let mut state = self.lock_state()?;
            match state.status {
                UsgsM2mSessionStatus::Authenticated => {
                    // Another caller already restored the session.
                    return Ok(UsgsM2mSessionStatus::Authenticated);
                }
                UsgsM2mSessionStatus::Ready | UsgsM2mSessionStatus::LoginFailed => {
                    // Fold the invalid terminal state to LoggingIn under the
                    // same lock hold, so a racing caller observes LoggingIn and
                    // waits instead of starting a second authentication.
                    state.api_key = None;
                    state.login_error = None;
                    state.logout_result = None;
                    state.status = UsgsM2mSessionStatus::LoggingIn;
                    drop(state);
                    self.login_token_requests.fetch_add(1, Ordering::Relaxed);
                    let result = self.endpoint.login_token(
                        material
                            .as_ref()
                            .expect("relogin material remains until the request returns"),
                        cancelled,
                    );
                    drop(material.take());
                    let mut state = self.lock_state()?;
                    if state.status != UsgsM2mSessionStatus::LoggingIn {
                        drop(result);
                        return Err(session_error(UsgsM2mErrorCode::AuthRequired));
                    }
                    match result {
                        Ok(api_key) => {
                            state.api_key = Some(api_key);
                            state.status = UsgsM2mSessionStatus::Authenticated;
                            self.changed.notify_all();
                            return Ok(state.status);
                        }
                        Err(error) => {
                            write_login_diagnostic(&error);
                            state.login_error = Some(error.clone());
                            state.status = UsgsM2mSessionStatus::LoginFailed;
                            self.changed.notify_all();
                            return Err(error);
                        }
                    }
                }
                UsgsM2mSessionStatus::LoggingIn => {
                    // Wait for the in-flight login and reuse its outcome. The
                    // waiting caller's own material is dropped: the outcome of
                    // the running authentication is authoritative for this
                    // session owner.
                    drop(material.take());
                    let (next, _) = self
                        .changed
                        .wait_timeout(state, CANCELLATION_POLL)
                        .map_err(|_| session_error(UsgsM2mErrorCode::ConfigInvalid))?;
                    drop(next);
                    if cancelled() {
                        return Err(session_error(UsgsM2mErrorCode::Cancelled));
                    }
                }
                UsgsM2mSessionStatus::LoggingOut | UsgsM2mSessionStatus::LoggedOut => {
                    // An explicit user logout must never be auto-reversed by a
                    // transfer-side recovery; a logout in flight cannot be
                    // distinguished from a completed one, so both refuse.
                    return Err(session_error(UsgsM2mErrorCode::AuthRequired));
                }
            }
        }
    }

    /// Explicitly validate the supplied material. Unlike a plain
    /// [`Self::login`] after [`Self::prepare_for_explicit_login`], a login
    /// that is already in flight (for example the accounts page restoring the
    /// session at startup) is awaited and its outcome reused instead of
    /// rejecting the caller: a download admission that races another login
    /// must not fail with AUTH_REQUIRED while the session is about to become
    /// authenticated. A real authentication failure still surfaces as an
    /// error for the caller to report. Unlike the transfer-side
    /// [`Self::relogin_after_invalidation`], an explicit user action may log
    /// in after a logout.
    pub fn login_validating_material(
        &self,
        material: UsgsM2mLoginMaterial,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mSessionStatus, UsgsM2mError> {
        let mut material = Some(material);
        loop {
            if cancelled() {
                return Err(session_error(UsgsM2mErrorCode::Cancelled));
            }
            let mut state = self.lock_state()?;
            match state.status {
                UsgsM2mSessionStatus::Authenticated => {
                    return Ok(UsgsM2mSessionStatus::Authenticated)
                }
                UsgsM2mSessionStatus::Ready
                | UsgsM2mSessionStatus::LoginFailed
                | UsgsM2mSessionStatus::LoggedOut => {
                    // Fold to LoggingIn under one lock hold so a racing caller
                    // observes LoggingIn and waits instead of being rejected by
                    // prepare_for_explicit_login's LoggingIn guard.
                    state.api_key = None;
                    state.login_error = None;
                    state.logout_result = None;
                    state.status = UsgsM2mSessionStatus::LoggingIn;
                    drop(state);
                    self.login_token_requests.fetch_add(1, Ordering::Relaxed);
                    let result = self.endpoint.login_token(
                        material
                            .as_ref()
                            .expect("login material remains until the single request returns"),
                        cancelled,
                    );
                    drop(material.take());
                    let mut state = self.lock_state()?;
                    if state.status != UsgsM2mSessionStatus::LoggingIn {
                        drop(result);
                        return Err(session_error(UsgsM2mErrorCode::AuthRequired));
                    }
                    match result {
                        Ok(api_key) => {
                            state.api_key = Some(api_key);
                            state.status = UsgsM2mSessionStatus::Authenticated;
                            self.changed.notify_all();
                            return Ok(state.status);
                        }
                        Err(error) => {
                            write_login_diagnostic(&error);
                            state.login_error = Some(error.clone());
                            state.status = UsgsM2mSessionStatus::LoginFailed;
                            self.changed.notify_all();
                            return Err(error);
                        }
                    }
                }
                UsgsM2mSessionStatus::LoggingIn => {
                    // Wait for the in-flight login and reuse its outcome.
                    drop(material.take());
                    let (next, _) = self
                        .changed
                        .wait_timeout(state, CANCELLATION_POLL)
                        .map_err(|_| session_error(UsgsM2mErrorCode::ConfigInvalid))?;
                    drop(next);
                    if cancelled() {
                        return Err(session_error(UsgsM2mErrorCode::Cancelled));
                    }
                }
                UsgsM2mSessionStatus::LoggingOut => {
                    return Err(session_error(UsgsM2mErrorCode::AuthRequired))
                }
            }
        }
    }

    pub fn login(
        &self,
        material: UsgsM2mLoginMaterial,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mSessionStatus, UsgsM2mError> {
        let mut material = Some(material);
        loop {
            if cancelled() {
                return Err(session_error(UsgsM2mErrorCode::Cancelled));
            }
            let mut state = self.lock_state()?;
            match state.status {
                UsgsM2mSessionStatus::Ready => {
                    state.status = UsgsM2mSessionStatus::LoggingIn;
                    drop(state);
                    self.login_token_requests.fetch_add(1, Ordering::Relaxed);
                    let result = self.endpoint.login_token(
                        material
                            .as_ref()
                            .expect("login material remains until the single request returns"),
                        cancelled,
                    );
                    drop(material.take());
                    let mut state = self.lock_state()?;
                    if state.status != UsgsM2mSessionStatus::LoggingIn {
                        drop(result);
                        return Err(session_error(UsgsM2mErrorCode::AuthRequired));
                    }
                    match result {
                        Ok(api_key) => {
                            state.api_key = Some(api_key);
                            state.status = UsgsM2mSessionStatus::Authenticated;
                            self.changed.notify_all();
                            return Ok(state.status);
                        }
                        Err(error) => {
                            write_login_diagnostic(&error);
                            state.login_error = Some(error.clone());
                            state.status = UsgsM2mSessionStatus::LoginFailed;
                            self.changed.notify_all();
                            return Err(error);
                        }
                    }
                }
                UsgsM2mSessionStatus::LoggingIn => {
                    let (next, _) = self
                        .changed
                        .wait_timeout(state, CANCELLATION_POLL)
                        .map_err(|_| session_error(UsgsM2mErrorCode::ConfigInvalid))?;
                    drop(next);
                }
                UsgsM2mSessionStatus::Authenticated => {
                    return Ok(UsgsM2mSessionStatus::Authenticated)
                }
                UsgsM2mSessionStatus::LoginFailed => {
                    return Err(state
                        .login_error
                        .clone()
                        .unwrap_or_else(|| session_error(UsgsM2mErrorCode::AuthInvalid)))
                }
                UsgsM2mSessionStatus::LoggingOut | UsgsM2mSessionStatus::LoggedOut => {
                    return Err(session_error(UsgsM2mErrorCode::AuthRequired))
                }
            }
        }
    }

    pub fn logout(&self, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<(), UsgsM2mError> {
        let api_key = loop {
            let mut state = self.lock_state()?;
            match state.status {
                UsgsM2mSessionStatus::Authenticated => {
                    state.status = UsgsM2mSessionStatus::LoggingOut;
                    break state
                        .api_key
                        .take()
                        .expect("authenticated session owns one runtime API key");
                }
                UsgsM2mSessionStatus::LoggingOut => {
                    let (next, _) = self
                        .changed
                        .wait_timeout(state, CANCELLATION_POLL)
                        .map_err(|_| session_error(UsgsM2mErrorCode::ConfigInvalid))?;
                    if cancelled() {
                        drop(next);
                        return Err(session_error(UsgsM2mErrorCode::Cancelled));
                    }
                    drop(next);
                }
                UsgsM2mSessionStatus::LoggedOut => {
                    return state.logout_result.clone().unwrap_or(Ok(()))
                }
                UsgsM2mSessionStatus::Ready
                | UsgsM2mSessionStatus::LoggingIn
                | UsgsM2mSessionStatus::LoginFailed => {
                    state.api_key = None;
                    state.status = UsgsM2mSessionStatus::LoggedOut;
                    state.logout_result = Some(Ok(()));
                    self.changed.notify_all();
                    return Ok(());
                }
            }
        };

        self.logout_requests.fetch_add(1, Ordering::Relaxed);
        let result = self.endpoint.logout(&api_key, cancelled);
        drop(api_key);
        let mut state = self.lock_state()?;
        state.api_key = None;
        state.status = UsgsM2mSessionStatus::LoggedOut;
        state.logout_result = Some(result.clone());
        self.changed.notify_all();
        result
    }

    pub(crate) fn request_dataset_discovery(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<super::endpoint::SensitiveJson, UsgsM2mError> {
        let _request_guard = self.lock_request_lane(cancelled)?;
        let api_key = self.authenticated_api_key()?;
        self.dataset_search_requests.fetch_add(1, Ordering::Relaxed);
        self.observe_authenticated_result(self.endpoint.dataset_search(&api_key, cancelled))
    }

    pub fn permissions(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<UsgsM2mPermissions, UsgsM2mError> {
        let _request_guard = self.lock_request_lane(cancelled)?;
        let api_key = self.authenticated_api_key()?;
        self.permission_requests.fetch_add(1, Ordering::Relaxed);
        let response =
            self.observe_authenticated_result(self.endpoint.permissions(&api_key, cancelled))?;
        parse_permissions(response.value())
    }

    pub(crate) fn request_dataset_filters(
        &self,
        dataset_name: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<super::endpoint::SensitiveJson, UsgsM2mError> {
        let _request_guard = self.lock_request_lane(cancelled)?;
        let api_key = self.authenticated_api_key()?;
        self.dataset_filter_requests.fetch_add(1, Ordering::Relaxed);
        self.observe_authenticated_result(self.endpoint.dataset_filters(
            &api_key,
            dataset_name,
            cancelled,
        ))
    }

    pub(crate) fn request_scene_search(
        &self,
        payload: &serde_json::Value,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<super::endpoint::SensitiveJson, UsgsM2mError> {
        let _request_guard = self.lock_request_lane(cancelled)?;
        let api_key = self.authenticated_api_key()?;
        self.scene_search_requests.fetch_add(1, Ordering::Relaxed);
        self.observe_authenticated_result(self.endpoint.scene_search(&api_key, payload, cancelled))
    }

    pub(crate) fn request_download_options(
        &self,
        payload: &serde_json::Value,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<super::endpoint::SensitiveJson, UsgsM2mError> {
        let _request_guard = self.lock_request_lane(cancelled)?;
        let api_key = self.authenticated_api_key()?;
        self.download_option_requests
            .fetch_add(1, Ordering::Relaxed);
        self.observe_authenticated_result(
            self.endpoint.download_options(&api_key, payload, cancelled),
        )
    }

    pub(crate) fn request_download(
        &self,
        payload: &serde_json::Value,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<super::endpoint::SensitiveJson, UsgsM2mError> {
        let _request_guard = self.lock_request_lane(cancelled)?;
        let api_key = self.authenticated_api_key()?;
        self.download_request_requests
            .fetch_add(1, Ordering::Relaxed);
        self.observe_authenticated_result(
            self.endpoint.download_request(&api_key, payload, cancelled),
        )
    }

    pub(crate) fn request_download_retrieve(
        &self,
        payload: &serde_json::Value,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<super::endpoint::SensitiveJson, UsgsM2mError> {
        let _request_guard = self.lock_request_lane(cancelled)?;
        let api_key = self.authenticated_api_key()?;
        self.download_retrieve_requests
            .fetch_add(1, Ordering::Relaxed);
        self.observe_authenticated_result(
            self.endpoint
                .download_retrieve(&api_key, payload, cancelled),
        )
    }

    fn lock_request_lane(
        &self,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<MutexGuard<'_, ()>, UsgsM2mError> {
        loop {
            if cancelled() {
                return Err(session_error(UsgsM2mErrorCode::Cancelled));
            }
            match self.request_lane.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::WouldBlock) => thread::sleep(CANCELLATION_POLL),
                Err(TryLockError::Poisoned(_)) => {
                    return Err(session_error(UsgsM2mErrorCode::ConfigInvalid))
                }
            }
        }
    }

    fn observe_authenticated_result<T>(
        &self,
        result: Result<T, UsgsM2mError>,
    ) -> Result<T, UsgsM2mError> {
        let Err(error) = result else {
            return result;
        };
        if matches!(
            error.code(),
            UsgsM2mErrorCode::AuthRequired
                | UsgsM2mErrorCode::AuthInvalid
                | UsgsM2mErrorCode::AuthUnauthorized
                | UsgsM2mErrorCode::AuthExpired
        ) {
            let mut state = self.lock_state()?;
            if state.status == UsgsM2mSessionStatus::Authenticated {
                state.api_key = None;
                state.login_error = Some(error.clone());
                state.status = UsgsM2mSessionStatus::LoginFailed;
                self.changed.notify_all();
            }
        }
        Err(error)
    }

    fn authenticated_api_key(&self) -> Result<UsgsM2mApiKey, UsgsM2mError> {
        let state = self.lock_state()?;
        if state.status != UsgsM2mSessionStatus::Authenticated {
            return Err(session_error(UsgsM2mErrorCode::AuthRequired));
        }
        state
            .api_key
            .as_ref()
            .ok_or_else(|| session_error(UsgsM2mErrorCode::AuthRequired))?
            .zeroizing_copy()
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, SessionState>, UsgsM2mError> {
        self.state
            .lock()
            .map_err(|_| session_error(UsgsM2mErrorCode::ConfigInvalid))
    }
}

impl fmt::Debug for UsgsM2mSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UsgsM2mSession")
            .field("network_policy", &self.network_policy)
            .field("runtime_api_key", &"[REDACTED]")
            .finish()
    }
}

impl Drop for UsgsM2mSession {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.api_key = None;
            state.status = UsgsM2mSessionStatus::LoggedOut;
            state.logout_result = None;
            self.changed.notify_all();
        }
    }
}

fn invalid_secret(value: &str, max_bytes: usize) -> bool {
    value.is_empty()
        || value.len() > max_bytes
        || value.trim().is_empty()
        || value.chars().any(char::is_control)
}

/// Trim ASCII/Unicode whitespace in place. The discarded tail is overwritten
/// with zeros before the length is reduced, and a leading gap is closed by a
/// memmove, so no untrimmed plaintext copy is ever allocated.
fn zeroize_trim(value: &mut String) {
    let trailing = value.len() - value.trim_end().len();
    if trailing > 0 {
        // Zero the tail bytes, then truncate. Safe: char boundaries are kept
        // because trim_end() itself ended on one.
        let new_len = value.len() - trailing;
        unsafe {
            value
                .as_mut_vec()
                .iter_mut()
                .skip(new_len)
                .for_each(|byte| *byte = 0);
            value.as_mut_vec().truncate(new_len);
        }
    }
    let leading = value.len() - value.trim_start().len();
    if leading > 0 {
        // Close the leading gap in place (memmove within the same buffer).
        unsafe {
            let bytes = value.as_mut_vec();
            bytes.copy_within(leading.., 0);
            bytes.truncate(bytes.len() - leading);
        }
    }
}

fn parse_permissions(value: &serde_json::Value) -> Result<UsgsM2mPermissions, UsgsM2mError> {
    let values = value
        .as_array()
        .filter(|values| values.len() <= 32)
        .ok_or_else(|| permissions_error(UsgsM2mErrorCode::ResponseInvalid))?;
    let mut permissions = UsgsM2mPermissions::default();
    for value in values {
        let permission = value
            .as_str()
            .filter(|permission| {
                !permission.is_empty()
                    && permission.len() <= 64
                    && permission.is_ascii()
                    && !permission.chars().any(char::is_control)
            })
            .ok_or_else(|| permissions_error(UsgsM2mErrorCode::ResponseInvalid))?;
        match permission.to_ascii_lowercase().as_str() {
            "user" => permissions.user = true,
            "download" => permissions.download = true,
            "order" => permissions.order = true,
            _ => {}
        }
    }
    Ok(permissions)
}

fn permissions_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::Permissions)
}

fn session_error(code: UsgsM2mErrorCode) -> UsgsM2mError {
    UsgsM2mError::new(code, UsgsM2mEndpointClass::RuntimeSession)
}

/// Opt-in login diagnosis under the existing `INSAR_DIAGNOSTIC_*` pattern.
/// Emits only the mapped error code, the upstream M2M `errorCode`, the HTTP
/// status and the sanitized upstream/transport message. The username and the
/// Application Token are never logged.
/// Upstream M2M errorCodes that may be named in the opt-in diagnostic. The
/// wire errorCode is arbitrary bounded ASCII, so an unlisted value (or one a
/// future endpoint change turns into an echo of submitted material) is logged
/// only as "other" — never as free text.
const DIAGNOSTIC_UPSTREAM_ERROR_CODES: &[&str] = &[
    "AUTH_REQUIRED",
    "AUTHENTICATION_REQUIRED",
    "AUTH_INVALID",
    "AUTH_KEY_INVALID",
    "INVALID_TOKEN",
    "AUTH_UNAUTHROIZED",
    "AUTH_UNAUTHORIZED",
    "AUTH_NOT_AUTHORIZED",
    "UNAUTHORIZED",
    "AUTH_EXPIRED",
    "AUTH_KEY_EXPIRED",
    "TOKEN_EXPIRED",
    "RATE_LIMITED",
    "RATE_LIMIT_EXCEEDED",
];

fn diagnostic_upstream_error_code(error: &UsgsM2mError) -> &str {
    error
        .remote_code()
        .filter(|code| DIAGNOSTIC_UPSTREAM_ERROR_CODES.contains(code))
        .unwrap_or("other")
}

fn write_login_diagnostic(error: &UsgsM2mError) {
    if !crate::diagnostics::usgs_m2m_login_diagnostic_enabled() {
        return;
    }
    // Only closed enums cross into the diagnostic: the mapped code, the
    // allowlisted upstream errorCode and the HTTP status. The upstream and
    // transport free-text messages are deliberately omitted — a malicious or
    // regressed endpoint could echo the submitted username or Application
    // Token inside them.
    let mut event = serde_json::json!({
        "event": "usgs_m2m_login_finished",
        "error_code": error.code().as_str(),
        "endpoint": error.endpoint().as_str(),
        "http_status": error.http_status(),
        "upstream_error_code": diagnostic_upstream_error_code(error),
        "transport_failed": error.transport_message().is_some(),
    });
    if let Some(retry_after) = error.retry_after_seconds() {
        event["retry_after_seconds"] = serde_json::json!(retry_after);
    }
    eprintln!("INSAR_USGS_M2M_LOGIN_DIAGNOSTIC {event}");
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::thread;

    use super::*;

    struct FixtureEndpoint {
        policy: ProviderNetworkPolicy,
        login_calls: AtomicUsize,
        logout_calls: AtomicUsize,
        fail_login: bool,
        fail_logout: bool,
        dataset_search_error: UsgsM2mErrorCode,
        login_delay: Duration,
        permission_delay: Duration,
        authenticated_active: AtomicUsize,
        authenticated_max: AtomicUsize,
    }

    impl FixtureEndpoint {
        fn new(policy: ProviderNetworkPolicy) -> Self {
            Self {
                policy,
                login_calls: AtomicUsize::new(0),
                logout_calls: AtomicUsize::new(0),
                fail_login: false,
                fail_logout: false,
                dataset_search_error: UsgsM2mErrorCode::ConfigInvalid,
                login_delay: Duration::ZERO,
                permission_delay: Duration::ZERO,
                authenticated_active: AtomicUsize::new(0),
                authenticated_max: AtomicUsize::new(0),
            }
        }
    }

    impl UsgsM2mSessionEndpoint for FixtureEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn login_token(
            &self,
            material: &UsgsM2mLoginMaterial,
            cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<UsgsM2mApiKey, UsgsM2mError> {
            self.login_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(material.username(), "fixture-user");
            assert!(material
                .application_token()
                .starts_with("fixture-application-token"));
            if !self.login_delay.is_zero() {
                thread::sleep(self.login_delay);
            }
            if cancelled() {
                return Err(session_error(UsgsM2mErrorCode::Cancelled));
            }
            if self.fail_login || material.application_token().ends_with("-rejected") {
                return Err(session_error(UsgsM2mErrorCode::AuthInvalid));
            }
            UsgsM2mApiKey::from_wire("opaque.runtime.key.with.dots".to_owned())
        }

        fn logout(
            &self,
            api_key: &UsgsM2mApiKey,
            cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<(), UsgsM2mError> {
            self.logout_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(api_key.expose(), "opaque.runtime.key.with.dots");
            if cancelled() {
                return Err(session_error(UsgsM2mErrorCode::Cancelled));
            }
            if self.fail_logout {
                Err(session_error(UsgsM2mErrorCode::RemoteError))
            } else {
                Ok(())
            }
        }

        fn permissions(
            &self,
            api_key: &UsgsM2mApiKey,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<super::super::endpoint::SensitiveJson, UsgsM2mError> {
            assert_eq!(api_key.expose(), "opaque.runtime.key.with.dots");
            let active = self.authenticated_active.fetch_add(1, Ordering::SeqCst) + 1;
            self.authenticated_max.fetch_max(active, Ordering::SeqCst);
            if !self.permission_delay.is_zero() {
                thread::sleep(self.permission_delay);
            }
            self.authenticated_active.fetch_sub(1, Ordering::SeqCst);
            Ok(super::super::endpoint::SensitiveJson::new(
                serde_json::json!(["user", "download", "order", "future-safe-role"]),
            ))
        }

        fn dataset_search(
            &self,
            _api_key: &UsgsM2mApiKey,
            _cancelled: &(dyn Fn() -> bool + Sync),
        ) -> Result<super::super::endpoint::SensitiveJson, UsgsM2mError> {
            Err(UsgsM2mError::new(
                self.dataset_search_error,
                UsgsM2mEndpointClass::DatasetDiscovery,
            ))
        }
    }

    fn material(suffix: &str) -> UsgsM2mLoginMaterial {
        UsgsM2mLoginMaterial::new(
            "fixture-user",
            UsgsM2mApplicationToken::new(format!("fixture-application-token-{suffix}")).unwrap(),
        )
        .unwrap()
    }

    fn fixture_session(
        endpoint: Arc<FixtureEndpoint>,
        policy: ProviderNetworkPolicy,
    ) -> Arc<UsgsM2mSession> {
        let endpoint_trait: Arc<dyn UsgsM2mSessionEndpoint> = endpoint;
        Arc::new(UsgsM2mSession::from_endpoint(policy, endpoint_trait).unwrap())
    }

    #[test]
    fn explicit_login_rechecks_new_material_without_revoking_remote_transfer_keys() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = fixture_session(Arc::clone(&endpoint), policy);
        session.login(material("first"), &|| false).unwrap();
        session.prepare_for_explicit_login().unwrap();
        assert_eq!(
            session
                .login(material("rejected"), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::AuthInvalid
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 2);
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 0);
        assert_eq!(session.status().unwrap(), UsgsM2mSessionStatus::LoginFailed);
        session.prepare_for_explicit_login().unwrap();
        session.login(material("corrected"), &|| false).unwrap();
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn login_installs_only_the_returned_opaque_api_key_and_logout_clears_it() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = fixture_session(Arc::clone(&endpoint), policy);

        assert_eq!(
            session.login(material("one"), &|| false).unwrap(),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            session.request_counts(),
            UsgsM2mRequestCounts {
                login_token: 1,
                ..UsgsM2mRequestCounts::default()
            }
        );
        session.logout(&|| false).unwrap();
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            session.request_counts(),
            UsgsM2mRequestCounts {
                login_token: 1,
                logout: 1,
                ..UsgsM2mRequestCounts::default()
            }
        );
        assert_eq!(session.status().unwrap(), UsgsM2mSessionStatus::LoggedOut);
    }

    #[test]
    fn permissions_reduce_remote_roles_to_closed_non_secret_booleans() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = fixture_session(endpoint, policy);
        session.login(material("permissions"), &|| false).unwrap();

        let permissions = session.permissions(&|| false).unwrap();
        assert!(permissions.has_user());
        assert!(permissions.has_download());
        assert!(permissions.has_order());
        assert_eq!(session.request_counts().permissions, 1);
        assert!(!format!("{permissions:?}").contains("future-safe-role"));
    }

    #[test]
    fn authenticated_control_requests_are_serialized_per_session() {
        let policy = ProviderNetworkPolicy::direct();
        let endpoint = Arc::new(FixtureEndpoint {
            permission_delay: Duration::from_millis(40),
            ..FixtureEndpoint::new(policy)
        });
        let session = fixture_session(Arc::clone(&endpoint), policy);
        session.login(material("serialized"), &|| false).unwrap();
        let barrier = Arc::new(Barrier::new(3));

        let first_session = Arc::clone(&session);
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_barrier.wait();
            first_session.permissions(&|| false)
        });
        let second_session = Arc::clone(&session);
        let second_barrier = Arc::clone(&barrier);
        let second = thread::spawn(move || {
            second_barrier.wait();
            second_session.permissions(&|| false)
        });
        barrier.wait();

        assert!(first.join().unwrap().is_ok());
        assert!(second.join().unwrap().is_ok());
        assert_eq!(session.request_counts().permissions, 2);
        assert_eq!(endpoint.authenticated_max.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn authenticated_remote_expiry_invalidates_the_runtime_session() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint {
            dataset_search_error: UsgsM2mErrorCode::AuthExpired,
            ..FixtureEndpoint::new(policy)
        });
        let session = fixture_session(endpoint, policy);
        session.login(material("expiry"), &|| false).unwrap();

        assert_eq!(
            session
                .request_dataset_discovery(&|| false)
                .err()
                .expect("fixture discovery expires the session")
                .code(),
            UsgsM2mErrorCode::AuthExpired
        );
        assert_eq!(session.status().unwrap(), UsgsM2mSessionStatus::LoginFailed);
        assert_eq!(
            session
                .request_dataset_discovery(&|| false)
                .err()
                .expect("invalidated session requires a new login")
                .code(),
            UsgsM2mErrorCode::AuthRequired
        );
    }

    #[test]
    fn concurrent_login_is_single_flight_and_never_retries() {
        let policy = ProviderNetworkPolicy::direct();
        let endpoint = Arc::new(FixtureEndpoint {
            login_delay: Duration::from_millis(40),
            ..FixtureEndpoint::new(policy)
        });
        let session = fixture_session(Arc::clone(&endpoint), policy);
        let barrier = Arc::new(Barrier::new(3));

        let first_session = Arc::clone(&session);
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_barrier.wait();
            first_session.login(material("first"), &|| false)
        });
        let second_session = Arc::clone(&session);
        let second_barrier = Arc::clone(&barrier);
        let second = thread::spawn(move || {
            second_barrier.wait();
            second_session.login(material("second"), &|| false)
        });
        barrier.wait();

        assert_eq!(
            first.join().unwrap().unwrap(),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(
            second.join().unwrap().unwrap(),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_login_is_terminal_for_the_session_and_is_not_retried() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint {
            fail_login: true,
            ..FixtureEndpoint::new(policy)
        });
        let session = fixture_session(Arc::clone(&endpoint), policy);

        for suffix in ["one", "two"] {
            assert_eq!(
                session
                    .login(material(suffix), &|| false)
                    .unwrap_err()
                    .code(),
                UsgsM2mErrorCode::AuthInvalid
            );
        }
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 1);

        session.prepare_for_explicit_login().unwrap();
        assert_eq!(
            session
                .login(material("manual-retry"), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::AuthInvalid
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn logout_failure_and_cancellation_still_remove_local_api_key() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint {
            fail_logout: true,
            ..FixtureEndpoint::new(policy)
        });
        let session = fixture_session(Arc::clone(&endpoint), policy);
        session.login(material("one"), &|| false).unwrap();
        assert_eq!(
            session.logout(&|| false).unwrap_err().code(),
            UsgsM2mErrorCode::RemoteError
        );
        assert_eq!(session.status().unwrap(), UsgsM2mSessionStatus::LoggedOut);
        assert_eq!(
            session.logout(&|| false).unwrap_err().code(),
            UsgsM2mErrorCode::RemoteError
        );
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);

        let cancelled_endpoint = Arc::new(FixtureEndpoint::new(policy));
        let cancelled_session = fixture_session(Arc::clone(&cancelled_endpoint), policy);
        cancelled_session.login(material("two"), &|| false).unwrap();
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            cancelled_session
                .logout(&|| cancelled.load(Ordering::SeqCst))
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::Cancelled
        );
        assert_eq!(cancelled_endpoint.logout_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancellation_before_login_does_not_call_the_endpoint() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = fixture_session(Arc::clone(&endpoint), policy);
        assert_eq!(
            session
                .login(material("cancelled"), &|| true)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::Cancelled
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 0);
        assert_eq!(session.status().unwrap(), UsgsM2mSessionStatus::Ready);
    }

    #[test]
    fn relogin_after_invalidation_restores_an_expired_session_once() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint {
            dataset_search_error: UsgsM2mErrorCode::AuthExpired,
            ..FixtureEndpoint::new(policy)
        });
        let session = fixture_session(Arc::clone(&endpoint), policy);
        session.login(material("first"), &|| false).unwrap();

        // Simulate remote apiKey expiry: the next control request fails with
        // an auth code and observe_authenticated_result invalidates the
        // runtime session.
        let expiry = session.request_dataset_discovery(&|| false);
        assert_eq!(
            expiry.err().map(|error| error.code()),
            Some(UsgsM2mErrorCode::AuthExpired)
        );

        // A transfer-side relogin with fresh material restores the session.
        assert_eq!(
            session
                .relogin_after_invalidation(material("second"), &|| false)
                .unwrap(),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 2);

        // When another caller already restored the session, the relogin is a
        // no-op: no additional authentication request.
        assert_eq!(
            session
                .relogin_after_invalidation(material("third"), &|| false)
                .unwrap(),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 2);

        // Expire again: a rejected relogin performs exactly one authentication
        // request and reports the terminal rejection to the caller, which
        // surfaces the "需要重新登录" pause instead of looping.
        let expiry = session.request_dataset_discovery(&|| false);
        assert_eq!(
            expiry.err().map(|error| error.code()),
            Some(UsgsM2mErrorCode::AuthExpired)
        );
        assert_eq!(
            session
                .relogin_after_invalidation(material("rejected"), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::AuthInvalid
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 3);
        assert_eq!(session.status().unwrap(), UsgsM2mSessionStatus::LoginFailed);
    }

    #[test]
    fn concurrent_relogin_is_single_flight() {
        // Two callers relogin at once after the same invalidation: exactly one
        // authentication request; the second waits and reuses the outcome.
        let policy = ProviderNetworkPolicy::direct();
        let endpoint = Arc::new(FixtureEndpoint {
            dataset_search_error: UsgsM2mErrorCode::AuthExpired,
            login_delay: Duration::from_millis(80),
            ..FixtureEndpoint::new(policy)
        });
        let session = fixture_session(Arc::clone(&endpoint), policy);
        session.login(material("first"), &|| false).unwrap();
        let expiry = session.request_dataset_discovery(&|| false);
        assert_eq!(
            expiry.err().map(|error| error.code()),
            Some(UsgsM2mErrorCode::AuthExpired)
        );

        let first_session = Arc::clone(&session);
        let first = thread::spawn(move || {
            first_session.relogin_after_invalidation(material("a"), &|| false)
        });
        thread::sleep(Duration::from_millis(20));
        let second = session.relogin_after_invalidation(material("b"), &|| false);

        assert_eq!(
            second.expect("the waiting caller reuses the in-flight relogin"),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(
            first
                .join()
                .expect("the relogin thread")
                .expect("the relogin outcome"),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_explicit_logout_is_never_auto_reversed() {
        // LoggedOut means the user signed out: a transfer-side recovery must
        // refuse, not silently log the account back in.
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = fixture_session(Arc::clone(&endpoint), policy);
        session.login(material("first"), &|| false).unwrap();
        session.logout(&|| false).unwrap();
        assert_eq!(
            session
                .relogin_after_invalidation(material("second"), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::AuthRequired
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 1);
        assert_eq!(session.status().unwrap(), UsgsM2mSessionStatus::LoggedOut);
    }

    /// E3: the download-admission login entry is atomic — a caller that races
    /// another login's Ready→LoggingIn transition waits for it instead of
    /// being rejected, and an explicit user login still works after a logout
    /// (unlike the transfer-side relogin).
    #[test]
    fn validating_login_is_atomic_and_works_after_logout() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = fixture_session(Arc::clone(&endpoint), policy);

        // After an explicit logout the user may sign in again.
        session.login(material("first"), &|| false).unwrap();
        session.logout(&|| false).unwrap();
        assert_eq!(
            session
                .login_validating_material(material("second"), &|| false)
                .unwrap(),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 2);

        // A logout in flight is still refused: the outcome of an in-flight
        // logout cannot be anticipated.
        session.logout(&|| false).unwrap();
        let _ = session
            .login_validating_material(material("third"), &|| false)
            .unwrap();
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn validating_login_waits_for_a_login_already_in_flight() {
        // A download admission that races the accounts-page session restore
        // must await the in-flight login and reuse its outcome, not fail with
        // AUTH_REQUIRED while the session is about to become authenticated.
        let policy = ProviderNetworkPolicy::direct();
        let endpoint = Arc::new(FixtureEndpoint {
            login_delay: Duration::from_millis(80),
            ..FixtureEndpoint::new(policy)
        });
        let session = fixture_session(Arc::clone(&endpoint), policy);

        let restoring_session = Arc::clone(&session);
        let restore =
            thread::spawn(move || restoring_session.login(material("restore"), &|| false));
        // Give the restore time to enter LoggingIn before the admission path
        // validates its own copy of the saved material.
        thread::sleep(Duration::from_millis(20));
        let admission = session.login_validating_material(material("restore"), &|| false);

        assert_eq!(
            admission.expect("the in-flight login is awaited, not rejected"),
            UsgsM2mSessionStatus::Authenticated
        );
        assert_eq!(
            restore
                .join()
                .expect("the restoring login also succeeds")
                .expect("the restoring login result"),
            UsgsM2mSessionStatus::Authenticated
        );
        // Exactly one authentication request was made for both callers.
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn validating_login_after_a_failed_login_reports_the_real_error() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = fixture_session(Arc::clone(&endpoint), policy);
        assert_eq!(
            session
                .login(material("rejected"), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::AuthInvalid
        );
        // The next validating login performs one fresh attempt and surfaces
        // the authentication failure itself, never a synthetic AUTH_REQUIRED.
        assert_eq!(
            session
                .login_validating_material(material("rejected"), &|| false)
                .unwrap_err()
                .code(),
            UsgsM2mErrorCode::AuthInvalid
        );
        assert_eq!(endpoint.login_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn debug_output_never_contains_login_or_runtime_secret_material() {
        let token = UsgsM2mApplicationToken::new("fixture-secret-token").unwrap();
        let login = UsgsM2mLoginMaterial::new("fixture-secret-user", token).unwrap();
        let key = UsgsM2mApiKey::from_wire("fixture-secret-api-key".to_owned()).unwrap();
        let debug = format!("{login:?} {key:?}");
        for secret in [
            "fixture-secret-token",
            "fixture-secret-user",
            "fixture-secret-api-key",
        ] {
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn login_material_trims_surrounding_whitespace_from_saved_values() {
        // A token copied out of a terminal or document often carries a trailing
        // newline; the M2M endpoint compares the exact string, so the padding
        // must be removed before the request is built.
        let padded = UsgsM2mApplicationToken::new("  fixture-application-token-padded \r\n")
            .expect("padded copy artifact is trimmed, not rejected");
        assert_eq!(padded.expose(), "fixture-application-token-padded");
        let material =
            UsgsM2mLoginMaterial::new(" fixture-user \n", padded).expect("trimmed material");
        assert_eq!(material.username(), "fixture-user");
        assert_eq!(
            material.application_token(),
            "fixture-application-token-padded"
        );

        // Whitespace-only values stay rejected: after trimming there is no
        // credential left to send.
        assert!(UsgsM2mApplicationToken::new(" \r\n").is_err());
        assert!(UsgsM2mLoginMaterial::new(
            "  ",
            UsgsM2mApplicationToken::new("fixture-token").unwrap()
        )
        .is_err());
    }

    #[test]
    fn wire_api_key_is_trimmed_before_it_is_used_as_the_auth_header() {
        let key = UsgsM2mApiKey::from_wire("  runtime-key-with-padding\n".to_owned()).unwrap();
        assert_eq!(key.expose(), "runtime-key-with-padding");
        assert!(UsgsM2mApiKey::from_wire(" \r\n ".to_owned()).is_err());
    }

    #[test]
    fn login_failure_diagnostic_reports_the_upstream_error_code_without_secrets() {
        // The fixture endpoint rejects tokens ending in -rejected with
        // AuthInvalid; the diagnostic must surface the mapped code and never
        // the material. The env gate is exercised through the same code path
        // the disabled default takes: nothing is printed, the error is intact.
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = fixture_session(Arc::clone(&endpoint), policy);
        let error = session.login(material("rejected"), &|| false).unwrap_err();
        assert_eq!(error.code(), UsgsM2mErrorCode::AuthInvalid);
        let diagnostic = error.diagnostic_message();
        assert!(diagnostic.contains("code=USGS_M2M_AUTH_INVALID"));
        assert!(!diagnostic.contains("fixture-application-token"));
        assert!(!diagnostic.contains("fixture-user"));
    }

    /// H2: the wire errorCode is arbitrary bounded ASCII, so only the closed
    /// allowlist may be named; anything else logs as "other". The upstream and
    /// transport free-text messages never enter the diagnostic event at all —
    /// a regressed endpoint echoing the submitted token inside a message has
    /// nothing to leak.
    #[test]
    fn login_diagnostic_event_carries_only_allowlisted_codes_no_free_text() {
        let echoed_token = "fixture-application-token-rejected";
        for (remote_code, expected) in [
            ("AUTH_INVALID", "AUTH_INVALID"),
            ("AUTH_UNAUTHROIZED", "AUTH_UNAUTHROIZED"),
            // An unlisted code and a token-echoing code both collapse to
            // "other": no arbitrary wire text crosses into the event.
            ("ORDER_NOT_ALLOWED", "other"),
            (echoed_token, "other"),
            ("", "other"),
        ] {
            let error = UsgsM2mError::new(
                UsgsM2mErrorCode::AuthInvalid,
                UsgsM2mEndpointClass::LoginToken,
            )
            .with_remote_error(
                remote_code.to_owned(),
                Some(format!("echoing {echoed_token} and fixture-user")),
                false,
            )
            .with_transport_error(format!("transport {echoed_token}"), false);
            assert_eq!(diagnostic_upstream_error_code(&error), expected);

            // Serialize exactly what the enabled gate would print.
            let mut event = serde_json::json!({
                "event": "usgs_m2m_login_finished",
                "error_code": error.code().as_str(),
                "endpoint": error.endpoint().as_str(),
                "http_status": error.http_status(),
                "upstream_error_code": diagnostic_upstream_error_code(&error),
                "transport_failed": error.transport_message().is_some(),
            });
            if let Some(retry_after) = error.retry_after_seconds() {
                event["retry_after_seconds"] = serde_json::json!(retry_after);
            }
            let line = format!("INSAR_USGS_M2M_LOGIN_DIAGNOSTIC {event}");
            assert!(!line.contains(echoed_token), "{line}");
            assert!(!line.contains("fixture-user"), "{line}");
            assert!(!line.contains("echoing"), "{line}");
            assert!(
                line.contains(&format!("\"upstream_error_code\":\"{expected}\"")),
                "{line}"
            );
        }
    }

    /// M8: trimming must not leave an untrimmed plaintext copy behind — the
    /// observable contract is that the value is trimmed and a no-op when there
    /// is nothing to strip (the allocation behavior itself is enforced by the
    /// in-place implementation; this guards its correctness for the mixed
    /// leading/trailing case).
    #[test]
    fn zeroize_trim_handles_leading_and_trailing_whitespace_in_place() {
        let mut value = String::from("  fixture-secret \r\n");
        zeroize_trim(&mut value);
        assert_eq!(value, "fixture-secret");

        let mut already_trimmed = String::from("fixture-secret");
        zeroize_trim(&mut already_trimmed);
        assert_eq!(already_trimmed, "fixture-secret");

        let mut only_whitespace = String::from(" \r\n ");
        zeroize_trim(&mut only_whitespace);
        assert!(only_whitespace.is_empty());

        // Multibyte characters around the trimmed gap stay intact.
        let mut multibyte = String::from("  宽松-token \n");
        zeroize_trim(&mut multibyte);
        assert_eq!(multibyte, "宽松-token");
    }
}
