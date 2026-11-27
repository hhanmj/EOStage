use std::fmt;
use std::io::Read;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use insar_core::ProviderNetworkPolicy;
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

/// Fixed production origin documented by the Bhoonidhi API specification.
pub const BHOONIDHI_API_ORIGIN: &str = "https://bhoonidhi-api.nrsc.gov.in";
pub const BHOONIDHI_TOKEN_ENDPOINT: &str = "https://bhoonidhi-api.nrsc.gov.in/auth/token";
pub const BHOONIDHI_LOGOUT_ENDPOINT: &str = "https://bhoonidhi-api.nrsc.gov.in/auth/logout";
pub const BHOONIDHI_AUTH_MAX_RESPONSE_BYTES: u64 = 64 * 1024;

const MAX_LOGIN_FIELD_BYTES: usize = 4 * 1024;
const MAX_TOKEN_BYTES: usize = 16 * 1024;
const MAX_TOKEN_RESPONSE_BYTES: u64 = BHOONIDHI_AUTH_MAX_RESPONSE_BYTES;
const MAX_TOKEN_LIFETIME_SECONDS: u64 = 24 * 60 * 60;
const IDENTITY_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IDENTITY_CONTROL_TIMEOUT: Duration = Duration::from_secs(45);
/// Design 2026-10-17 §4: a token generation gets at most three automatic
/// refresh grant attempts while they fail transiently (timeout, transport);
/// the process sends at most nine automatic refresh grants per rolling hour.
const REFRESH_GRANT_ATTEMPTS_PER_GENERATION: u32 = 3;
const AUTOMATIC_REFRESH_GRANTS_PER_WINDOW: usize = 9;
const AUTOMATIC_REFRESH_WINDOW: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum BhoonidhiAuthError {
    #[error("Bhoonidhi OAuth 配置无效")]
    Configuration,
    #[error("未找到 Bhoonidhi 凭据")]
    Missing,
    #[error("Bhoonidhi 凭据内容无效")]
    CredentialInvalid,
    #[error("Bhoonidhi 认证被服务拒绝")]
    Rejected,
    #[error("Bhoonidhi 活跃会话数已达上限")]
    MaxSessions,
    #[error("Bhoonidhi 账户尚未激活")]
    ActivationRequired,
    #[error("Bhoonidhi 认证请求触发限流")]
    RateLimited,
    #[error("Bhoonidhi OAuth 传输失败")]
    Transport,
    #[error("Bhoonidhi OAuth 请求超时")]
    Timeout,
    #[error("Bhoonidhi OAuth 响应无效")]
    ResponseInvalid,
    #[error("Bhoonidhi 刷新令牌不可用")]
    RefreshUnavailable,
    #[error("Bhoonidhi 当前令牌的自动刷新已不可再尝试")]
    RefreshExhausted,
    /// Process-local limit on automatic refresh grants (source=local).
    #[error("Bhoonidhi 自动刷新已达本地上限（每小时 9 次）")]
    LocalRefreshRateLimited,
    #[error("Bhoonidhi 会话已被注销或替换")]
    SessionChanged,
    #[error("Bhoonidhi 会话没有可用访问令牌")]
    SessionUnavailable,
    #[error("Bhoonidhi OAuth 会话同步失败")]
    Synchronization,
}

impl BhoonidhiAuthError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Configuration => "BHOONIDHI_OAUTH_CONFIGURATION_INVALID",
            Self::Missing => "BHOONIDHI_CREDENTIALS_MISSING",
            Self::CredentialInvalid => "BHOONIDHI_CREDENTIALS_INVALID",
            Self::Rejected => "BHOONIDHI_AUTH_REJECTED",
            Self::MaxSessions => "BHOONIDHI_MAX_SESSIONS",
            Self::ActivationRequired => "BHOONIDHI_ACTIVATION_REQUIRED",
            Self::RateLimited => "BHOONIDHI_RATE_LIMITED",
            Self::Transport => "BHOONIDHI_AUTH_TRANSPORT_FAILED",
            Self::Timeout => "BHOONIDHI_AUTH_TIMEOUT",
            Self::ResponseInvalid => "BHOONIDHI_AUTH_RESPONSE_INVALID",
            Self::RefreshUnavailable => "BHOONIDHI_REFRESH_UNAVAILABLE",
            Self::RefreshExhausted => "BHOONIDHI_REFRESH_EXHAUSTED",
            Self::LocalRefreshRateLimited => "BHOONIDHI_REFRESH_LOCAL_RATE_LIMITED",
            Self::SessionChanged => "BHOONIDHI_SESSION_CHANGED",
            Self::SessionUnavailable => "BHOONIDHI_SESSION_UNAVAILABLE",
            Self::Synchronization => "BHOONIDHI_SYNCHRONIZATION_FAILED",
        }
    }
}

/// Caller-owned login material. It is accepted only for the duration of the
/// password grant and is never serializable or persisted by this module.
pub struct BhoonidhiLoginMaterial {
    user_id: Zeroizing<String>,
    password: Zeroizing<String>,
}

impl BhoonidhiLoginMaterial {
    pub fn new(
        user_id: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, BhoonidhiAuthError> {
        Self::from_secret_parts(
            Zeroizing::new(user_id.into()),
            Zeroizing::new(password.into()),
        )
    }

    pub(crate) fn from_secret_parts(
        user_id: Zeroizing<String>,
        password: Zeroizing<String>,
    ) -> Result<Self, BhoonidhiAuthError> {
        if invalid_user_id(user_id.as_str()) || invalid_password(password.as_str()) {
            return Err(BhoonidhiAuthError::CredentialInvalid);
        }
        Ok(Self { user_id, password })
    }

    fn user_id(&self) -> &str {
        self.user_id.as_str()
    }

    fn password(&self) -> &str {
        self.password.as_str()
    }
}

impl fmt::Debug for BhoonidhiLoginMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BhoonidhiLoginMaterial")
            .field("user_id", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .finish()
    }
}

fn invalid_user_id(value: &str) -> bool {
    value.is_empty()
        || value.len() > MAX_LOGIN_FIELD_BYTES
        || value.contains(['\r', '\n', '\0'])
        || value.trim().is_empty()
}

fn invalid_password(value: &str) -> bool {
    value.is_empty() || value.len() > MAX_LOGIN_FIELD_BYTES || value.contains(['\r', '\n', '\0'])
}

/// Runtime-only access material returned by [`BhoonidhiSession::access_token`].
/// The generation lets a caller bind a subsequent rejection refresh to the
/// token it actually used, while the secret remains zeroizing and redacted.
pub struct BhoonidhiAccessToken {
    value: Zeroizing<String>,
    generation: u64,
}

impl BhoonidhiAccessToken {
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

impl AsRef<str> for BhoonidhiAccessToken {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for BhoonidhiAccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BhoonidhiAccessToken")
            .field("value", &"[REDACTED]")
            .field("generation", &self.generation)
            .finish()
    }
}

/// Token material returned by an auth endpoint. The server's `userId` is
/// checked against the request and is therefore not retained a second time.
pub struct BhoonidhiTokenGrant {
    access_token: Zeroizing<String>,
    refresh_token: Zeroizing<String>,
    expires_in: Duration,
}

impl BhoonidhiTokenGrant {
    pub fn new(
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_in_seconds: Option<u64>,
    ) -> Result<Self, BhoonidhiAuthError> {
        Self::from_secret_parts(
            Zeroizing::new(access_token.into()),
            refresh_token.map(Zeroizing::new),
            expires_in_seconds,
        )
    }

    fn from_secret_parts(
        access_token: Zeroizing<String>,
        refresh_token: Option<Zeroizing<String>>,
        expires_in_seconds: Option<u64>,
    ) -> Result<Self, BhoonidhiAuthError> {
        if invalid_token(access_token.as_str()) {
            return Err(BhoonidhiAuthError::ResponseInvalid);
        }
        let refresh_token = refresh_token.ok_or(BhoonidhiAuthError::ResponseInvalid)?;
        if invalid_token(refresh_token.as_str()) {
            return Err(BhoonidhiAuthError::ResponseInvalid);
        }
        let expires_in_seconds = expires_in_seconds
            .filter(|seconds| (1..=MAX_TOKEN_LIFETIME_SECONDS).contains(seconds))
            .ok_or(BhoonidhiAuthError::ResponseInvalid)?;
        Ok(Self {
            access_token,
            refresh_token,
            expires_in: Duration::from_secs(expires_in_seconds),
        })
    }
}

impl fmt::Debug for BhoonidhiTokenGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BhoonidhiTokenGrant")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

fn invalid_token(value: &str) -> bool {
    value.is_empty() || value.len() > MAX_TOKEN_BYTES || value.contains(['\r', '\n', '\0'])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhoonidhiTokenSnapshot {
    pub authenticated: bool,
    pub generation: Option<u64>,
    pub refresh_available: bool,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BhoonidhiRefreshOutcome {
    NotNeeded(BhoonidhiTokenSnapshot),
    Refreshed(BhoonidhiTokenSnapshot),
    AlreadyRefreshed(BhoonidhiTokenSnapshot),
}

/// Native auth boundary used by the session. Production uses the reqwest
/// implementation below; tests inject an offline endpoint and make no HTTP
/// requests.
pub trait BhoonidhiTokenEndpoint: Send + Sync {
    fn network_policy(&self) -> ProviderNetworkPolicy;
    fn password_grant(
        &self,
        material: &BhoonidhiLoginMaterial,
    ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError>;
    fn refresh_grant(
        &self,
        user_id: &str,
        refresh_token: &str,
    ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError>;
    fn logout(&self, refresh_token: &str) -> Result<(), BhoonidhiAuthError>;
}

pub struct ReqwestBhoonidhiTokenEndpoint {
    client: Client,
    token_endpoint: Url,
    logout_endpoint: Url,
    network_policy: ProviderNetworkPolicy,
}

impl ReqwestBhoonidhiTokenEndpoint {
    pub fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, BhoonidhiAuthError> {
        let token_endpoint =
            Url::parse(BHOONIDHI_TOKEN_ENDPOINT).map_err(|_| BhoonidhiAuthError::Configuration)?;
        let logout_endpoint =
            Url::parse(BHOONIDHI_LOGOUT_ENDPOINT).map_err(|_| BhoonidhiAuthError::Configuration)?;
        validate_endpoint(&token_endpoint, "/auth/token")?;
        validate_endpoint(&logout_endpoint, "/auth/logout")?;

        let client = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .cookie_store(false)
            .referer(false)
            .connect_timeout(IDENTITY_CONNECT_TIMEOUT)
            .timeout(IDENTITY_CONTROL_TIMEOUT);
        let client = insar_provider::network_proxy::blocking_builder_with_proxy(
            client,
            network_policy.route(),
        )
        .build()
        .map_err(|_| BhoonidhiAuthError::Configuration)?;

        Ok(Self {
            client,
            token_endpoint,
            logout_endpoint,
            network_policy,
        })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    fn exchange(
        &self,
        body: Zeroizing<Vec<u8>>,
        expected_user_id: &str,
    ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(600));
            }
            let send_result = self
                .client
                .post(self.token_endpoint.clone())
                .header(CONTENT_TYPE, "application/json")
                .body(body.as_slice().to_vec())
                .send();
            match send_result {
                Ok(response) => {
                    if response.status().is_server_error() && attempt < 2 {
                        continue;
                    }
                    if !response.status().is_success() {
                        let status = response.status();
                        let body = read_bounded_response(response)?;
                        return Err(map_status_error(status, &body));
                    }
                    return parse_token_response(response, expected_user_id);
                }
                Err(err) => {
                    last_err = Some(map_request_error(err));
                }
            }
        }
        Err(last_err.unwrap_or(BhoonidhiAuthError::Transport))
    }
}

impl BhoonidhiTokenEndpoint for ReqwestBhoonidhiTokenEndpoint {
    fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    fn password_grant(
        &self,
        material: &BhoonidhiLoginMaterial,
    ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
        self.exchange(password_request_json(material)?, material.user_id())
    }

    fn refresh_grant(
        &self,
        user_id: &str,
        refresh_token: &str,
    ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
        if invalid_user_id(user_id) || invalid_token(refresh_token) {
            return Err(BhoonidhiAuthError::Configuration);
        }
        self.exchange(refresh_request_json(user_id, refresh_token)?, user_id)
    }

    fn logout(&self, refresh_token: &str) -> Result<(), BhoonidhiAuthError> {
        if invalid_token(refresh_token) {
            return Err(BhoonidhiAuthError::Configuration);
        }
        let mut bearer = Zeroizing::new(String::from("Bearer "));
        bearer.push_str(refresh_token);
        let mut header = HeaderValue::from_str(bearer.as_str())
            .map_err(|_| BhoonidhiAuthError::Configuration)?;
        header.set_sensitive(true);
        let response = self
            .client
            .post(self.logout_endpoint.clone())
            .header(AUTHORIZATION, header)
            .send()
            .map_err(map_request_error)?;
        let status = response.status();
        let body = read_bounded_response(response)?;
        if !status.is_success() {
            return Err(map_status_error(status, &body));
        }
        Ok(())
    }
}

impl fmt::Debug for ReqwestBhoonidhiTokenEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReqwestBhoonidhiTokenEndpoint")
            .field("network_policy", &self.network_policy)
            .field("origin", &BHOONIDHI_API_ORIGIN)
            .finish()
    }
}

pub type ReqwestBhoonidhiAuthEndpoint = ReqwestBhoonidhiTokenEndpoint;

fn validate_endpoint(endpoint: &Url, expected_path: &str) -> Result<(), BhoonidhiAuthError> {
    if endpoint.scheme() != "https"
        || endpoint.host_str() != Some("bhoonidhi-api.nrsc.gov.in")
        || endpoint.port_or_known_default() != Some(443)
        || endpoint.path() != expected_path
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(BhoonidhiAuthError::Configuration);
    }
    Ok(())
}

#[derive(Serialize)]
struct PasswordGrantRequest<'a> {
    #[serde(rename = "userId")]
    user_id: &'a str,
    password: &'a str,
    grant_type: &'static str,
}

#[derive(Serialize)]
struct RefreshGrantRequest<'a> {
    #[serde(rename = "userId")]
    user_id: &'a str,
    refresh_token: &'a str,
    grant_type: &'static str,
}

fn password_request_json(
    material: &BhoonidhiLoginMaterial,
) -> Result<Zeroizing<Vec<u8>>, BhoonidhiAuthError> {
    serde_json::to_vec(&PasswordGrantRequest {
        user_id: material.user_id(),
        password: material.password(),
        grant_type: "password",
    })
    .map(Zeroizing::new)
    .map_err(|_| BhoonidhiAuthError::Configuration)
}

fn refresh_request_json(
    user_id: &str,
    refresh_token: &str,
) -> Result<Zeroizing<Vec<u8>>, BhoonidhiAuthError> {
    serde_json::to_vec(&RefreshGrantRequest {
        user_id,
        refresh_token,
        grant_type: "refresh_token",
    })
    .map(Zeroizing::new)
    .map_err(|_| BhoonidhiAuthError::Configuration)
}

struct WireSecret(Zeroizing<String>);

impl<'de> Deserialize<'de> for WireSecret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct WireSecretVisitor;

        impl<'de> Visitor<'de> for WireSecretVisitor {
            type Value = Zeroizing<String>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Bhoonidhi OAuth string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Zeroizing::new(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Zeroizing::new(value))
            }
        }

        deserializer.deserialize_string(WireSecretVisitor).map(Self)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(rename = "userId")]
    user_id: WireSecret,
    access_token: WireSecret,
    token_type: WireSecret,
    expires_in: u64,
    refresh_token: WireSecret,
}

fn parse_token_response(
    response: Response,
    expected_user_id: &str,
) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
    let body = read_bounded_response(response)?;
    parse_token_body(&body, expected_user_id)
}

fn read_bounded_response(response: Response) -> Result<Zeroizing<Vec<u8>>, BhoonidhiAuthError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_TOKEN_RESPONSE_BYTES)
    {
        return Err(BhoonidhiAuthError::ResponseInvalid);
    }
    let mut body = Zeroizing::new(Vec::new());
    response
        .take(MAX_TOKEN_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| BhoonidhiAuthError::Transport)?;
    if body.len() as u64 > MAX_TOKEN_RESPONSE_BYTES {
        return Err(BhoonidhiAuthError::ResponseInvalid);
    }
    Ok(body)
}

fn parse_token_body(
    body: &[u8],
    expected_user_id: &str,
) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
    if body.len() as u64 > MAX_TOKEN_RESPONSE_BYTES {
        return Err(BhoonidhiAuthError::ResponseInvalid);
    }
    if invalid_user_id(expected_user_id) {
        return Err(BhoonidhiAuthError::Configuration);
    }
    let TokenResponse {
        user_id,
        access_token,
        token_type,
        expires_in,
        refresh_token,
    } = serde_json::from_slice(body).map_err(|_| BhoonidhiAuthError::ResponseInvalid)?;
    if user_id.0.as_str() != expected_user_id || !token_type.0.eq_ignore_ascii_case("Bearer") {
        return Err(BhoonidhiAuthError::ResponseInvalid);
    }
    BhoonidhiTokenGrant::from_secret_parts(access_token.0, Some(refresh_token.0), Some(expires_in))
}

fn map_request_error(error: reqwest::Error) -> BhoonidhiAuthError {
    if error.is_timeout() {
        BhoonidhiAuthError::Timeout
    } else {
        BhoonidhiAuthError::Transport
    }
}

fn map_status_error(status: StatusCode, body: &[u8]) -> BhoonidhiAuthError {
    if contains_ascii_case_insensitive(body, b"activation")
        || contains_ascii_case_insensitive(body, b"activate account")
        || contains_ascii_case_insensitive(body, b"account not active")
    {
        return BhoonidhiAuthError::ActivationRequired;
    }
    match status {
        StatusCode::TOO_MANY_REQUESTS => BhoonidhiAuthError::RateLimited,
        StatusCode::FORBIDDEN => BhoonidhiAuthError::MaxSessions,
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED => BhoonidhiAuthError::Rejected,
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => BhoonidhiAuthError::Timeout,
        // 412 is a download-concurrency concern elsewhere in the stack. It
        // intentionally has no OAuth-specific precondition error here.
        _ => BhoonidhiAuthError::Transport,
    }
}

fn contains_ascii_case_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.windows(needle.len()).any(|window| {
            window
                .iter()
                .zip(needle)
                .all(|(left, right)| left.eq_ignore_ascii_case(right))
        })
}

#[cfg(all(test, feature = "bhoonidhi-auth-revoke-acceptance"))]
mod acceptance_revoke_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{channel, Receiver, Sender};

    struct BlockingRevokeEndpoint {
        started: Sender<()>,
        release: Mutex<Receiver<()>>,
        logout_calls: AtomicUsize,
    }

    impl BhoonidhiTokenEndpoint for BlockingRevokeEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            ProviderNetworkPolicy::system()
        }
        fn password_grant(
            &self,
            _: &BhoonidhiLoginMaterial,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            BhoonidhiTokenGrant::new("fixture-access", Some("fixture-refresh".into()), Some(1200))
        }
        fn refresh_grant(
            &self,
            _: &str,
            _: &str,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            panic!("concurrent refresh must be fenced before endpoint I/O")
        }
        fn logout(&self, _: &str) -> Result<(), BhoonidhiAuthError> {
            self.logout_calls.fetch_add(1, Ordering::SeqCst);
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            Ok(())
        }
    }

    #[test]
    fn acceptance_revoke_inflight_fences_refresh_and_never_clears_new_login() {
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel();
        let endpoint = Arc::new(BlockingRevokeEndpoint {
            started: started_tx,
            release: Mutex::new(release_rx),
            logout_calls: AtomicUsize::new(0),
        });
        let session = Arc::new(
            BhoonidhiSession::new(
                ProviderNetworkPolicy::system(),
                endpoint.clone(),
                Arc::new(crate::SystemClock),
            )
            .unwrap(),
        );
        let original = session
            .login(BhoonidhiLoginMaterial::new("fixture-user", "fixture-password").unwrap())
            .unwrap()
            .generation
            .unwrap();
        let worker_session = session.clone();
        let worker = std::thread::spawn(move || {
            worker_session.acceptance_revoke_current_generation(original)
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            session.refresh_after_rejection(original),
            Err(BhoonidhiAuthError::SessionChanged)
        ));
        // This would deadlock if revocation held the session mutex during I/O.
        let replacement = session
            .login(BhoonidhiLoginMaterial::new("fixture-user", "fixture-password").unwrap())
            .unwrap()
            .generation
            .unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap(),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert_ne!(original, replacement);
        assert_eq!(session.snapshot().unwrap().generation, Some(replacement));
        assert!(session.snapshot().unwrap().authenticated);
        assert_eq!(
            session.acceptance_revoke_current_generation(replacement),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
    }
}

struct SessionTokens {
    user_id: Zeroizing<String>,
    access_token: Zeroizing<String>,
    refresh_token: Zeroizing<String>,
    expires_at: SystemTime,
    generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RefreshAttempt {
    epoch: u64,
    generation: u64,
}

#[derive(Default)]
struct SessionState {
    epoch: u64,
    next_generation: u64,
    tokens: Option<SessionTokens>,
    refreshing: Option<RefreshAttempt>,
    failed_refresh: Option<RefreshAttempt>,
    failed_refresh_cause: Option<BhoonidhiAuthError>,
    /// Transient refresh failures of one generation; not terminal by themselves.
    transient_refresh: Option<(RefreshAttempt, u32)>,
    /// Start times of automatic refresh grants in the rolling window.
    automatic_grants: std::collections::VecDeque<SystemTime>,
    last_recovery_revocation_confirmed: Option<bool>,
    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    acceptance_revoke_consumed: bool,
    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    acceptance_revoke_inflight: Option<RefreshAttempt>,
}

/// Process-owned Bhoonidhi session. Login material is supplied explicitly;
/// access and refresh tokens never cross a durable Store or serialization
/// boundary. The OAuth endpoint is single-flight for each token generation.
pub struct BhoonidhiSession {
    network_policy: ProviderNetworkPolicy,
    endpoint: Arc<dyn BhoonidhiTokenEndpoint>,
    clock: Arc<dyn crate::Clock>,
    state: Mutex<SessionState>,
    changed: Condvar,
}

pub type BhoonidhiOAuthSession = BhoonidhiSession;

impl BhoonidhiSession {
    #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
    pub(crate) fn acceptance_revoke_current_generation(
        &self,
        expected_generation: u64,
    ) -> Result<(), BhoonidhiAuthError> {
        let (attempt, refresh_token) = {
            let mut state = self.lock_state()?;
            let tokens = state
                .tokens
                .as_ref()
                .ok_or(BhoonidhiAuthError::SessionUnavailable)?;
            if tokens.generation != expected_generation
                || state.acceptance_revoke_consumed
                || state.refreshing.is_some()
                || state.failed_refresh.is_some()
                || is_expired(tokens, self.clock.as_ref())
            {
                return Err(BhoonidhiAuthError::SessionChanged);
            }
            let attempt = RefreshAttempt {
                epoch: state.epoch,
                generation: tokens.generation,
            };
            let refresh_token = Zeroizing::new(tokens.refresh_token.to_string());
            // Consume before I/O, including timeout and unknown outcomes. Login
            // and credential invalidation never replenish this one-shot budget.
            state.acceptance_revoke_consumed = true;
            state.acceptance_revoke_inflight = Some(attempt);
            (attempt, refresh_token)
        };
        let revoked = self.endpoint.logout(refresh_token.as_str());
        let mut state = self.lock_state()?;
        if state.acceptance_revoke_inflight == Some(attempt) {
            state.acceptance_revoke_inflight = None;
        }
        self.changed.notify_all();
        if state.epoch != attempt.epoch
            || state
                .tokens
                .as_ref()
                .is_none_or(|tokens| tokens.generation != attempt.generation)
        {
            return Err(BhoonidhiAuthError::SessionChanged);
        }
        revoked
    }

    pub fn new(
        network_policy: ProviderNetworkPolicy,
        endpoint: Arc<dyn BhoonidhiTokenEndpoint>,
        clock: Arc<dyn crate::Clock>,
    ) -> Result<Self, BhoonidhiAuthError> {
        if endpoint.network_policy() != network_policy {
            return Err(BhoonidhiAuthError::Configuration);
        }
        Ok(Self {
            network_policy,
            endpoint,
            clock,
            state: Mutex::new(SessionState::default()),
            changed: Condvar::new(),
        })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    pub fn login(
        &self,
        material: BhoonidhiLoginMaterial,
    ) -> Result<BhoonidhiTokenSnapshot, BhoonidhiAuthError> {
        let epoch = self.clear_state()?;
        let user_id = material.user_id.clone();
        let grant = self.endpoint.password_grant(&material)?;
        let mut state = self.lock_state()?;
        if state.epoch != epoch {
            return Err(BhoonidhiAuthError::SessionChanged);
        }
        install_grant(&mut state, user_id, grant, self.clock.as_ref())?;
        self.changed.notify_all();
        Ok(snapshot(&state, self.clock.as_ref()))
    }

    pub fn snapshot(&self) -> Result<BhoonidhiTokenSnapshot, BhoonidhiAuthError> {
        let state = self.lock_state()?;
        Ok(snapshot(&state, self.clock.as_ref()))
    }

    pub fn access_token(&self) -> Result<BhoonidhiAccessToken, BhoonidhiAuthError> {
        let state = self.lock_state()?;
        if let Some(error) = failed_generation_cause(&state) {
            return Err(error);
        }
        let tokens = state
            .tokens
            .as_ref()
            .ok_or(BhoonidhiAuthError::SessionUnavailable)?;
        if is_expired(tokens, self.clock.as_ref()) {
            return Err(BhoonidhiAuthError::Timeout);
        }
        Ok(BhoonidhiAccessToken {
            value: Zeroizing::new(tokens.access_token.to_string()),
            generation: tokens.generation,
        })
    }

    /// Refresh only when the currently installed access token is already
    /// expired. Generation fencing keeps concurrent callers single-flight.
    pub fn refresh_if_expired(&self) -> Result<BhoonidhiRefreshOutcome, BhoonidhiAuthError> {
        let snapshot = self.snapshot()?;
        if snapshot.authenticated {
            return Ok(BhoonidhiRefreshOutcome::NotNeeded(snapshot));
        }
        let generation = snapshot
            .generation
            .ok_or(BhoonidhiAuthError::SessionUnavailable)?;
        self.refresh_generation(generation)
    }

    pub fn refresh_after_rejection(
        &self,
        generation: u64,
    ) -> Result<BhoonidhiRefreshOutcome, BhoonidhiAuthError> {
        self.refresh_generation(generation)
    }

    /// A refreshed bearer was itself rejected. Never reuse it based only on
    /// its local expiry; an older response must not invalidate a newer login.
    pub fn mark_generation_rejected(&self, generation: u64) -> Result<(), BhoonidhiAuthError> {
        let mut state = self.lock_state()?;
        if state
            .tokens
            .as_ref()
            .is_some_and(|tokens| tokens.generation == generation)
        {
            let attempt = RefreshAttempt {
                epoch: state.epoch,
                generation,
            };
            if state.failed_refresh != Some(attempt) {
                state.failed_refresh = Some(attempt);
                state.failed_refresh_cause = Some(BhoonidhiAuthError::Rejected);
            }
            self.changed.notify_all();
        }
        Ok(())
    }

    /// Safe recovery metadata only: false also covers a rejected revocation,
    /// which is not evidence that the server revoked the old session.
    pub fn last_explicit_recovery_revocation_confirmed(
        &self,
    ) -> Result<Option<bool>, BhoonidhiAuthError> {
        Ok(self.lock_state()?.last_recovery_revocation_confirmed)
    }

    pub(crate) fn failed_generation_error(
        &self,
    ) -> Result<Option<BhoonidhiAuthError>, BhoonidhiAuthError> {
        let state = self.lock_state()?;
        Ok(failed_generation_cause(&state))
    }

    /// The caller serializes and budgets explicit user recovery. Revoke the
    /// old session before a password grant; unknown revocation outcomes stop.
    pub(crate) fn reauthenticate_failed_generation(
        &self,
        material: BhoonidhiLoginMaterial,
    ) -> Result<(), BhoonidhiAuthError> {
        let (attempt, refresh_token) = {
            let mut state = self.lock_state()?;
            failed_generation_cause(&state).ok_or(BhoonidhiAuthError::SessionChanged)?;
            let tokens = state
                .tokens
                .as_ref()
                .ok_or(BhoonidhiAuthError::SessionUnavailable)?;
            let attempt = RefreshAttempt {
                epoch: state.epoch,
                generation: tokens.generation,
            };
            let refresh_token = Zeroizing::new(tokens.refresh_token.to_string());
            state.last_recovery_revocation_confirmed = None;
            (attempt, refresh_token)
        };
        let revoked = self.endpoint.logout(refresh_token.as_str());
        let epoch = {
            let mut state = self.lock_state()?;
            if state.epoch != attempt.epoch
                || state
                    .tokens
                    .as_ref()
                    .is_none_or(|tokens| tokens.generation != attempt.generation)
            {
                return Err(BhoonidhiAuthError::SessionChanged);
            }
            state.last_recovery_revocation_confirmed = Some(revoked.is_ok());
            if let Err(error) = revoked {
                if error != BhoonidhiAuthError::Rejected {
                    return Err(error);
                }
            }
            state.epoch = state
                .epoch
                .checked_add(1)
                .ok_or(BhoonidhiAuthError::Synchronization)?;
            state.tokens = None;
            state.refreshing = None;
            state.failed_refresh = None;
            state.failed_refresh_cause = None;
            self.changed.notify_all();
            state.epoch
        };
        let user_id = material.user_id.clone();
        let grant = self.endpoint.password_grant(&material)?;
        let mut state = self.lock_state()?;
        if state.epoch != epoch {
            return Err(BhoonidhiAuthError::SessionChanged);
        }
        install_grant(&mut state, user_id, grant, self.clock.as_ref())?;
        self.changed.notify_all();
        Ok(())
    }

    /// Clears local state before making the revocation request. A concurrent
    /// login may therefore replace the session safely while logout is in
    /// flight; the old refresh token is the only material sent to the server.
    pub fn logout(&self) -> Result<(), BhoonidhiAuthError> {
        let refresh_token = self.clear_state_and_take_refresh()?;
        if let Some(refresh_token) = refresh_token {
            self.endpoint.logout(refresh_token.as_str())?;
        }
        Ok(())
    }

    /// Invalidates local tokens without contacting the OAuth endpoint.
    ///
    /// Credential-manager mutations use this hook after their revision changes
    /// so an old access or refresh token cannot survive into a new login epoch.
    pub fn invalidate_local(&self) -> Result<(), BhoonidhiAuthError> {
        self.clear_state().map(|_| ())
    }

    fn clear_state(&self) -> Result<u64, BhoonidhiAuthError> {
        let mut state = self.lock_state()?;
        state.epoch = state
            .epoch
            .checked_add(1)
            .ok_or(BhoonidhiAuthError::Synchronization)?;
        state.tokens = None;
        state.refreshing = None;
        state.failed_refresh = None;
        state.failed_refresh_cause = None;
        self.changed.notify_all();
        Ok(state.epoch)
    }

    fn clear_state_and_take_refresh(
        &self,
    ) -> Result<Option<Zeroizing<String>>, BhoonidhiAuthError> {
        let mut state = self.lock_state()?;
        state.epoch = state
            .epoch
            .checked_add(1)
            .ok_or(BhoonidhiAuthError::Synchronization)?;
        let refresh_token = state.tokens.take().map(|tokens| tokens.refresh_token);
        state.refreshing = None;
        state.failed_refresh = None;
        state.failed_refresh_cause = None;
        self.changed.notify_all();
        Ok(refresh_token)
    }

    fn refresh_generation(
        &self,
        required_generation: u64,
    ) -> Result<BhoonidhiRefreshOutcome, BhoonidhiAuthError> {
        let (attempt, user_id, refresh_token) = {
            let mut state = self.lock_state()?;
            loop {
                let Some(tokens) = state.tokens.as_ref() else {
                    return Err(BhoonidhiAuthError::SessionUnavailable);
                };
                if tokens.generation != required_generation {
                    return Ok(BhoonidhiRefreshOutcome::AlreadyRefreshed(snapshot(
                        &state,
                        self.clock.as_ref(),
                    )));
                }
                let attempt = RefreshAttempt {
                    epoch: state.epoch,
                    generation: tokens.generation,
                };
                #[cfg(feature = "bhoonidhi-auth-revoke-acceptance")]
                if state.acceptance_revoke_inflight == Some(attempt) {
                    return Err(BhoonidhiAuthError::SessionChanged);
                }
                if state.failed_refresh == Some(attempt) {
                    return Err(BhoonidhiAuthError::RefreshExhausted);
                }
                if state.refreshing == Some(attempt) {
                    state = self
                        .changed
                        .wait(state)
                        .map_err(|_| BhoonidhiAuthError::Synchronization)?;
                    continue;
                }
                let user_id = Zeroizing::new(tokens.user_id.to_string());
                let refresh_token = Zeroizing::new(tokens.refresh_token.to_string());
                let now = self.clock.now();
                while state.automatic_grants.front().is_some_and(|started| {
                    now.duration_since(*started)
                        .is_ok_and(|age| age >= AUTOMATIC_REFRESH_WINDOW)
                }) {
                    state.automatic_grants.pop_front();
                }
                if state.automatic_grants.len() >= AUTOMATIC_REFRESH_GRANTS_PER_WINDOW {
                    // Local limit: no request is sent and the generation stays
                    // refreshable once the window has room again.
                    return Err(BhoonidhiAuthError::LocalRefreshRateLimited);
                }
                state.automatic_grants.push_back(now);
                state.refreshing = Some(attempt);
                break (attempt, user_id, refresh_token);
            }
        };

        let refreshed = self
            .endpoint
            .refresh_grant(user_id.as_str(), refresh_token.as_str());
        let mut state = self.lock_state()?;
        if state.refreshing == Some(attempt) {
            state.refreshing = None;
        }
        if state.epoch != attempt.epoch
            || state
                .tokens
                .as_ref()
                .is_none_or(|tokens| tokens.generation != attempt.generation)
        {
            self.changed.notify_all();
            return Err(BhoonidhiAuthError::SessionChanged);
        }
        match refreshed {
            Ok(grant) => match install_grant(&mut state, user_id, grant, self.clock.as_ref()) {
                Ok(()) => {
                    self.changed.notify_all();
                    Ok(BhoonidhiRefreshOutcome::Refreshed(snapshot(
                        &state,
                        self.clock.as_ref(),
                    )))
                }
                Err(error) => {
                    state.failed_refresh = Some(attempt);
                    state.failed_refresh_cause = Some(error);
                    self.changed.notify_all();
                    Err(error)
                }
            },
            Err(error @ (BhoonidhiAuthError::Timeout | BhoonidhiAuthError::Transport)) => {
                // Transient: the generation stays refreshable until its third
                // failed grant; then it is terminal with this last real cause.
                let failures = match state.transient_refresh {
                    Some((previous, count)) if previous == attempt => count.saturating_add(1),
                    _ => 1,
                };
                if failures >= REFRESH_GRANT_ATTEMPTS_PER_GENERATION {
                    state.transient_refresh = None;
                    state.failed_refresh = Some(attempt);
                    state.failed_refresh_cause = Some(error);
                } else {
                    state.transient_refresh = Some((attempt, failures));
                }
                self.changed.notify_all();
                Err(error)
            }
            Err(error) => {
                state.failed_refresh = Some(attempt);
                state.failed_refresh_cause = Some(error);
                self.changed.notify_all();
                Err(error)
            }
        }
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, SessionState>, BhoonidhiAuthError> {
        self.state
            .lock()
            .map_err(|_| BhoonidhiAuthError::Synchronization)
    }
}

impl fmt::Debug for BhoonidhiSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BhoonidhiSession")
            .field("network_policy", &self.network_policy)
            .field("runtime_material", &"[REDACTED]")
            .finish()
    }
}

fn install_grant(
    state: &mut SessionState,
    user_id: Zeroizing<String>,
    grant: BhoonidhiTokenGrant,
    clock: &dyn crate::Clock,
) -> Result<(), BhoonidhiAuthError> {
    let expires_at = clock
        .now()
        .checked_add(grant.expires_in)
        .ok_or(BhoonidhiAuthError::ResponseInvalid)?;
    let generation = state
        .next_generation
        .checked_add(1)
        .ok_or(BhoonidhiAuthError::Synchronization)?;
    state.next_generation = generation;
    state.tokens = Some(SessionTokens {
        user_id,
        access_token: grant.access_token,
        refresh_token: grant.refresh_token,
        expires_at,
        generation,
    });
    state.failed_refresh = None;
    state.failed_refresh_cause = None;
    state.transient_refresh = None;
    Ok(())
}

fn failed_generation_cause(state: &SessionState) -> Option<BhoonidhiAuthError> {
    let tokens = state.tokens.as_ref()?;
    (state.failed_refresh
        == Some(RefreshAttempt {
            epoch: state.epoch,
            generation: tokens.generation,
        }))
    .then_some(
        state
            .failed_refresh_cause
            .unwrap_or(BhoonidhiAuthError::RefreshExhausted),
    )
}

fn is_expired(tokens: &SessionTokens, clock: &dyn crate::Clock) -> bool {
    tokens.expires_at <= clock.now()
}

fn snapshot(state: &SessionState, clock: &dyn crate::Clock) -> BhoonidhiTokenSnapshot {
    match state.tokens.as_ref() {
        None => BhoonidhiTokenSnapshot {
            authenticated: false,
            generation: None,
            refresh_available: false,
            expires_in_seconds: None,
        },
        Some(tokens) => BhoonidhiTokenSnapshot {
            authenticated: !is_expired(tokens, clock) && failed_generation_cause(state).is_none(),
            generation: Some(tokens.generation),
            refresh_available: failed_generation_cause(state).is_none(),
            expires_in_seconds: Some(
                tokens
                    .expires_at
                    .duration_since(clock.now())
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0),
            ),
        },
    }
}

#[cfg(test)]
#[path = "bhoonidhi_route_tests.rs"]
mod route_tests;

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::thread;

    use super::*;

    struct FixedClock(SystemTime);

    impl crate::Clock for FixedClock {
        fn now(&self) -> SystemTime {
            self.0
        }
    }

    struct FixtureEndpoint {
        policy: ProviderNetworkPolicy,
        password_calls: AtomicUsize,
        refresh_calls: AtomicUsize,
        logout_calls: AtomicUsize,
        reject_refresh: AtomicBool,
    }

    impl FixtureEndpoint {
        fn new(policy: ProviderNetworkPolicy) -> Self {
            Self {
                policy,
                password_calls: AtomicUsize::new(0),
                refresh_calls: AtomicUsize::new(0),
                logout_calls: AtomicUsize::new(0),
                reject_refresh: AtomicBool::new(false),
            }
        }
    }

    impl BhoonidhiTokenEndpoint for FixtureEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn password_grant(
            &self,
            material: &BhoonidhiLoginMaterial,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            self.password_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(material.user_id(), "fixture-user");
            assert_eq!(material.password(), "fixture-password");
            BhoonidhiTokenGrant::new(
                "fixture-access-one",
                Some("fixture-refresh-one".to_owned()),
                Some(60),
            )
        }

        fn refresh_grant(
            &self,
            user_id: &str,
            refresh_token: &str,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            self.refresh_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(user_id, "fixture-user");
            assert_eq!(refresh_token, "fixture-refresh-one");
            if self.reject_refresh.load(Ordering::SeqCst) {
                return Err(BhoonidhiAuthError::Rejected);
            }
            thread::sleep(Duration::from_millis(20));
            BhoonidhiTokenGrant::new(
                "fixture-access-two",
                Some("fixture-refresh-two".to_owned()),
                Some(60),
            )
        }

        fn logout(&self, refresh_token: &str) -> Result<(), BhoonidhiAuthError> {
            self.logout_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(refresh_token, "fixture-refresh-one");
            Ok(())
        }
    }

    struct BlockingRefreshEndpoint {
        policy: ProviderNetworkPolicy,
        refresh_started: Barrier,
        release_refresh: Barrier,
    }

    impl BlockingRefreshEndpoint {
        fn new(policy: ProviderNetworkPolicy) -> Self {
            Self {
                policy,
                refresh_started: Barrier::new(2),
                release_refresh: Barrier::new(2),
            }
        }
    }

    impl BhoonidhiTokenEndpoint for BlockingRefreshEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn password_grant(
            &self,
            _material: &BhoonidhiLoginMaterial,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            BhoonidhiTokenGrant::new(
                "fixture-access-one",
                Some("fixture-refresh-one".to_owned()),
                Some(60),
            )
        }

        fn refresh_grant(
            &self,
            _user_id: &str,
            _refresh_token: &str,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            self.refresh_started.wait();
            self.release_refresh.wait();
            BhoonidhiTokenGrant::new(
                "fixture-access-two",
                Some("fixture-refresh-two".to_owned()),
                Some(60),
            )
        }

        fn logout(&self, _refresh_token: &str) -> Result<(), BhoonidhiAuthError> {
            Ok(())
        }
    }

    struct BlockingLoginEndpoint {
        policy: ProviderNetworkPolicy,
        login_started: Barrier,
        release_login: Barrier,
    }

    impl BlockingLoginEndpoint {
        fn new(policy: ProviderNetworkPolicy) -> Self {
            Self {
                policy,
                login_started: Barrier::new(2),
                release_login: Barrier::new(2),
            }
        }
    }

    impl BhoonidhiTokenEndpoint for BlockingLoginEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn password_grant(
            &self,
            _material: &BhoonidhiLoginMaterial,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            self.login_started.wait();
            self.release_login.wait();
            BhoonidhiTokenGrant::new(
                "fixture-access-one",
                Some("fixture-refresh-one".to_owned()),
                Some(60),
            )
        }

        fn refresh_grant(
            &self,
            _user_id: &str,
            _refresh_token: &str,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            Err(BhoonidhiAuthError::RefreshUnavailable)
        }

        fn logout(&self, _refresh_token: &str) -> Result<(), BhoonidhiAuthError> {
            Ok(())
        }
    }

    fn session(
        endpoint: Arc<dyn BhoonidhiTokenEndpoint>,
        policy: ProviderNetworkPolicy,
    ) -> Arc<BhoonidhiSession> {
        Arc::new(
            BhoonidhiSession::new(
                policy,
                endpoint,
                Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
            )
            .expect("matching fixture policy"),
        )
    }

    fn login_material() -> BhoonidhiLoginMaterial {
        BhoonidhiLoginMaterial::new("fixture-user", "fixture-password")
            .expect("valid fixture login")
    }

    #[test]
    fn password_grant_json_uses_the_documented_wire_names() {
        let material = login_material();
        let body = password_request_json(&material).expect("request JSON");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("JSON object");
        assert_eq!(
            body,
            serde_json::json!({
                "userId": "fixture-user",
                "password": "fixture-password",
                "grant_type": "password"
            })
        );
    }

    #[test]
    fn refresh_json_uses_snake_case_refresh_token_and_grant_type() {
        let body =
            refresh_request_json("fixture-user", "fixture-refresh-one").expect("request JSON");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("JSON object");
        assert_eq!(
            body,
            serde_json::json!({
                "userId": "fixture-user",
                "refresh_token": "fixture-refresh-one",
                "grant_type": "refresh_token"
            })
        );
    }

    #[test]
    fn auth_recovery_failed_refresh_never_reports_or_returns_old_access() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = session(endpoint.clone(), policy);
        let generation = session.login(login_material()).unwrap().generation.unwrap();
        endpoint.reject_refresh.store(true, Ordering::SeqCst);
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::Rejected)
        );
        assert!(!session.snapshot().unwrap().authenticated);
        assert_eq!(
            session.access_token().unwrap_err(),
            BhoonidhiAuthError::Rejected
        );
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn auth_recovery_late_rejection_cannot_invalidate_new_login() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = session(endpoint, policy);
        let old = session.login(login_material()).unwrap().generation.unwrap();
        let new = session.login(login_material()).unwrap().generation.unwrap();
        session.mark_generation_rejected(old).unwrap();
        assert!(session.snapshot().unwrap().authenticated);
        assert_eq!(session.access_token().unwrap().generation(), new);
        session.mark_generation_rejected(new).unwrap();
        assert_eq!(
            session.access_token().unwrap_err(),
            BhoonidhiAuthError::Rejected
        );
        assert!(!session.snapshot().unwrap().refresh_available);
    }

    #[test]
    fn auth_recovery_inflight_password_cannot_override_logout_epoch() {
        struct Endpoint {
            calls: AtomicUsize,
            started: Barrier,
            release: Barrier,
        }
        impl BhoonidhiTokenEndpoint for Endpoint {
            fn network_policy(&self) -> ProviderNetworkPolicy {
                ProviderNetworkPolicy::system()
            }
            fn password_grant(
                &self,
                _: &BhoonidhiLoginMaterial,
            ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
                if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
                    self.started.wait();
                    self.release.wait();
                }
                BhoonidhiTokenGrant::new(
                    "fixture-access",
                    Some("fixture-refresh".to_owned()),
                    Some(60),
                )
            }
            fn refresh_grant(
                &self,
                _: &str,
                _: &str,
            ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
                Err(BhoonidhiAuthError::Rejected)
            }
            fn logout(&self, _: &str) -> Result<(), BhoonidhiAuthError> {
                Ok(())
            }
        }
        let endpoint = Arc::new(Endpoint {
            calls: AtomicUsize::new(0),
            started: Barrier::new(2),
            release: Barrier::new(2),
        });
        let session = session(endpoint.clone(), ProviderNetworkPolicy::system());
        let generation = session.login(login_material()).unwrap().generation.unwrap();
        session.mark_generation_rejected(generation).unwrap();
        let other = session.clone();
        let recovery =
            thread::spawn(move || other.reauthenticate_failed_generation(login_material()));
        endpoint.started.wait();
        session.logout().unwrap();
        endpoint.release.wait();
        assert_eq!(
            recovery.join().unwrap(),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert!(!session.snapshot().unwrap().authenticated);
        assert_eq!(
            session.access_token().unwrap_err(),
            BhoonidhiAuthError::SessionUnavailable
        );
    }

    #[test]
    fn token_response_requires_exact_identity_type_and_bounded_expiry() {
        let body = br#"{
            "userId":"fixture-user",
            "access_token":"fixture-access-one",
            "token_type":"Bearer",
            "expires_in":1200,
            "refresh_token":"fixture-refresh-one",
            "context":{"ignored":true}
        }"#;
        let grant = parse_token_body(body, "fixture-user").expect("valid response");
        let debug = format!("{grant:?}");
        for secret in ["fixture-user", "fixture-access-one", "fixture-refresh-one"] {
            assert!(!debug.contains(secret));
        }
        assert!(parse_token_body(
            br#"{"userId":"fixture-user","access_token":"a","token_type":"Basic","expires_in":1200,"refresh_token":"r"}"#,
            "fixture-user"
        )
        .is_err());
        assert!(parse_token_body(
            br#"{"userId":"other-user","access_token":"a","token_type":"Bearer","expires_in":1200,"refresh_token":"r"}"#,
            "fixture-user"
        )
        .is_err());
        assert!(parse_token_body(
            br#"{"userId":"fixture-user","access_token":"a","token_type":"Bearer","expires_in":0,"refresh_token":"r"}"#,
            "fixture-user"
        )
        .is_err());
    }

    #[test]
    fn response_and_status_limits_fail_closed_without_an_auth_precondition_variant() {
        let oversized = vec![b' '; (MAX_TOKEN_RESPONSE_BYTES as usize) + 1];
        assert_eq!(
            parse_token_body(&oversized, "fixture-user").unwrap_err(),
            BhoonidhiAuthError::ResponseInvalid
        );
        assert_eq!(
            map_status_error(StatusCode::FORBIDDEN, b"maximum sessions reached"),
            BhoonidhiAuthError::MaxSessions
        );
        assert_eq!(
            map_status_error(StatusCode::TOO_MANY_REQUESTS, b"slow down"),
            BhoonidhiAuthError::RateLimited
        );
        assert_eq!(
            map_status_error(StatusCode::BAD_REQUEST, b"activation required"),
            BhoonidhiAuthError::ActivationRequired
        );
        assert_eq!(
            map_status_error(StatusCode::PRECONDITION_FAILED, b"precondition"),
            BhoonidhiAuthError::Transport
        );
    }

    #[test]
    fn login_is_runtime_only_and_access_debug_is_redacted() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = session(endpoint.clone(), policy);
        let snapshot = session.login(login_material()).expect("login succeeds");
        assert!(snapshot.authenticated);
        assert_eq!(snapshot.generation, Some(1));
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
        for _ in 0..8 {
            let token = session.access_token().expect("cached access token");
            assert_eq!(token.as_str(), "fixture-access-one");
        }
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
        let token = session.access_token().expect("access token");
        let debug = format!("{session:?} {token:?} {:?}", login_material());
        for secret in [
            "fixture-user",
            "fixture-password",
            "fixture-access-one",
            "fixture-refresh-one",
        ] {
            assert!(!debug.contains(secret));
        }
        assert_eq!(token.as_str(), "fixture-access-one");
        assert_eq!(token.generation(), 1);
    }

    #[test]
    fn refresh_is_single_flight_and_bounded_per_generation() {
        let policy = ProviderNetworkPolicy::direct();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = session(endpoint.clone(), policy);
        let generation = session.login(login_material()).unwrap().generation.unwrap();

        let barrier = Arc::new(Barrier::new(3));
        let first_session = Arc::clone(&session);
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_barrier.wait();
            first_session.refresh_after_rejection(generation)
        });
        let second_session = Arc::clone(&session);
        let second_barrier = Arc::clone(&barrier);
        let second = thread::spawn(move || {
            second_barrier.wait();
            second_session.refresh_after_rejection(generation)
        });
        barrier.wait();

        let outcomes = [
            first
                .join()
                .expect("first refresh thread")
                .expect("first refresh"),
            second
                .join()
                .expect("second refresh thread")
                .expect("second refresh"),
        ];
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);
        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, BhoonidhiRefreshOutcome::Refreshed(_))));
        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, BhoonidhiRefreshOutcome::AlreadyRefreshed(_))));
        assert_eq!(session.snapshot().unwrap().generation, Some(2));
    }

    /// Refresh grants answer from a script; `Ok` installs a new token pair.
    struct ScriptedRefreshEndpoint {
        policy: ProviderNetworkPolicy,
        script: Mutex<std::collections::VecDeque<Result<(), BhoonidhiAuthError>>>,
        refresh_calls: AtomicUsize,
    }

    impl ScriptedRefreshEndpoint {
        fn new(
            policy: ProviderNetworkPolicy,
            script: impl IntoIterator<Item = Result<(), BhoonidhiAuthError>>,
        ) -> Self {
            Self {
                policy,
                script: Mutex::new(script.into_iter().collect()),
                refresh_calls: AtomicUsize::new(0),
            }
        }
    }

    impl BhoonidhiTokenEndpoint for ScriptedRefreshEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn password_grant(
            &self,
            _material: &BhoonidhiLoginMaterial,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            BhoonidhiTokenGrant::new(
                "fixture-access-one",
                Some("fixture-refresh-one".to_owned()),
                Some(60),
            )
        }

        fn refresh_grant(
            &self,
            _user_id: &str,
            _refresh_token: &str,
        ) -> Result<BhoonidhiTokenGrant, BhoonidhiAuthError> {
            let call = self.refresh_calls.fetch_add(1, Ordering::SeqCst);
            self.script.lock().unwrap().pop_front().unwrap_or(Ok(()))?;
            BhoonidhiTokenGrant::new(
                format!("fixture-access-{call}"),
                Some(format!("fixture-refresh-{call}")),
                Some(60),
            )
        }

        fn logout(&self, _refresh_token: &str) -> Result<(), BhoonidhiAuthError> {
            Ok(())
        }
    }

    struct SteppingClock(std::sync::atomic::AtomicU64);

    impl crate::Clock for SteppingClock {
        fn now(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH + Duration::from_secs(self.0.load(Ordering::SeqCst))
        }
    }

    fn scripted_session(
        script: impl IntoIterator<Item = Result<(), BhoonidhiAuthError>>,
    ) -> (
        Arc<BhoonidhiSession>,
        Arc<ScriptedRefreshEndpoint>,
        Arc<SteppingClock>,
    ) {
        let policy = ProviderNetworkPolicy::direct();
        let endpoint = Arc::new(ScriptedRefreshEndpoint::new(policy, script));
        let clock = Arc::new(SteppingClock(std::sync::atomic::AtomicU64::new(1_000)));
        let session =
            Arc::new(BhoonidhiSession::new(policy, endpoint.clone(), clock.clone()).unwrap());
        session.login(login_material()).unwrap();
        (session, endpoint, clock)
    }

    /// Design 2026-10-17 §4: a refresh timeout or transport failure no longer
    /// fails the generation; the same generation may be refreshed again.
    #[test]
    fn transient_refresh_failures_keep_the_generation_refreshable() {
        let (session, endpoint, _) = scripted_session([
            Err(BhoonidhiAuthError::Timeout),
            Err(BhoonidhiAuthError::Transport),
            Ok(()),
        ]);
        let generation = session.snapshot().unwrap().generation.unwrap();
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::Timeout)
        );
        assert_eq!(session.failed_generation_error().unwrap(), None);
        assert!(session.snapshot().unwrap().refresh_available);
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::Transport)
        );
        assert!(matches!(
            session.refresh_after_rejection(generation),
            Ok(BhoonidhiRefreshOutcome::Refreshed(_))
        ));
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 3);
        assert_eq!(session.access_token().unwrap().generation(), generation + 1);
    }

    /// The third transient failure of one generation is terminal and keeps the
    /// last real cause; no further grant is sent for that generation.
    #[test]
    fn third_transient_refresh_failure_is_terminal_with_the_last_real_cause() {
        let (session, endpoint, _) = scripted_session([
            Err(BhoonidhiAuthError::Timeout),
            Err(BhoonidhiAuthError::Timeout),
            Err(BhoonidhiAuthError::Timeout),
        ]);
        let generation = session.snapshot().unwrap().generation.unwrap();
        for _ in 0..3 {
            assert_eq!(
                session.refresh_after_rejection(generation),
                Err(BhoonidhiAuthError::Timeout)
            );
        }
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::RefreshExhausted)
        );
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            session.access_token().unwrap_err(),
            BhoonidhiAuthError::Timeout
        );
        assert!(!session.snapshot().unwrap().refresh_available);
    }

    /// A rejected refresh stays terminal at once (unchanged rule).
    #[test]
    fn rejected_refresh_remains_terminal_without_another_grant() {
        let (session, endpoint, _) = scripted_session([Err(BhoonidhiAuthError::Rejected)]);
        let generation = session.snapshot().unwrap().generation.unwrap();
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::Rejected)
        );
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::RefreshExhausted)
        );
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);
    }

    /// At most nine automatic refresh grants per rolling hour in this process;
    /// the tenth is refused locally without a request, and the window recovers.
    #[test]
    fn automatic_refresh_grants_are_limited_to_nine_per_rolling_hour() {
        let (session, endpoint, clock) = scripted_session([]);
        for _ in 0..9 {
            let generation = session.snapshot().unwrap().generation.unwrap();
            assert!(matches!(
                session.refresh_after_rejection(generation),
                Ok(BhoonidhiRefreshOutcome::Refreshed(_))
            ));
        }
        let generation = session.snapshot().unwrap().generation.unwrap();
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::LocalRefreshRateLimited)
        );
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 9);
        assert_eq!(session.failed_generation_error().unwrap(), None);
        clock.0.fetch_add(3_600, Ordering::SeqCst);
        assert!(matches!(
            session.refresh_after_rejection(generation),
            Ok(BhoonidhiRefreshOutcome::Refreshed(_))
        ));
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn rejected_refresh_is_not_retried_and_logout_revokes_and_clears() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        endpoint.reject_refresh.store(true, Ordering::SeqCst);
        let session = session(endpoint.clone(), policy);
        let generation = session.login(login_material()).unwrap().generation.unwrap();

        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::Rejected)
        );
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(BhoonidhiAuthError::RefreshExhausted)
        );
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);
        session.logout().expect("logout");
        assert_eq!(endpoint.logout_calls.load(Ordering::SeqCst), 1);
        assert!(!session.snapshot().unwrap().authenticated);
        assert_eq!(
            session.access_token().unwrap_err(),
            BhoonidhiAuthError::SessionUnavailable
        );
    }

    #[test]
    fn logout_invalidates_an_in_flight_refresh() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(BlockingRefreshEndpoint::new(policy));
        let session = session(endpoint.clone(), policy);
        let generation = session.login(login_material()).unwrap().generation.unwrap();

        let refreshing_session = Arc::clone(&session);
        let refreshing =
            thread::spawn(move || refreshing_session.refresh_after_rejection(generation));
        endpoint.refresh_started.wait();
        session.logout().expect("logout");
        endpoint.release_refresh.wait();

        assert_eq!(
            refreshing.join().expect("refresh thread"),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert!(!session.snapshot().unwrap().authenticated);
    }

    #[test]
    fn local_invalidation_invalidates_an_in_flight_login() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(BlockingLoginEndpoint::new(policy));
        let session = session(endpoint.clone(), policy);
        let login_session = Arc::clone(&session);
        let login = thread::spawn(move || login_session.login(login_material()));
        endpoint.login_started.wait();

        session.invalidate_local().expect("local invalidation");
        endpoint.release_login.wait();

        assert_eq!(
            login.join().expect("login thread"),
            Err(BhoonidhiAuthError::SessionChanged)
        );
        assert!(!session.snapshot().unwrap().authenticated);
    }

    #[test]
    fn endpoint_configuration_is_exact_and_route_bound() {
        let direct_policy = ProviderNetworkPolicy::direct();
        let endpoint =
            ReqwestBhoonidhiTokenEndpoint::new(direct_policy).expect("reviewed endpoint");
        assert_eq!(endpoint.network_policy(), direct_policy);
        let system_policy = ProviderNetworkPolicy::system();
        let system_endpoint = ReqwestBhoonidhiTokenEndpoint::new(system_policy)
            .expect("reviewed endpoint with system route");
        assert_eq!(system_endpoint.network_policy(), system_policy);
        assert_eq!(
            BHOONIDHI_TOKEN_ENDPOINT,
            "https://bhoonidhi-api.nrsc.gov.in/auth/token"
        );
        assert_eq!(
            BHOONIDHI_LOGOUT_ENDPOINT,
            "https://bhoonidhi-api.nrsc.gov.in/auth/logout"
        );
        assert!(BhoonidhiSession::new(
            system_policy,
            Arc::new(endpoint),
            Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
        )
        .is_err());
        assert!(BhoonidhiSession::new(
            direct_policy,
            Arc::new(system_endpoint),
            Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
        )
        .is_err());
    }

    #[test]
    fn malformed_login_and_token_material_fail_closed() {
        assert_eq!(
            BhoonidhiLoginMaterial::new("", "password").unwrap_err(),
            BhoonidhiAuthError::CredentialInvalid
        );
        assert_eq!(
            BhoonidhiLoginMaterial::new("user", "password\n").unwrap_err(),
            BhoonidhiAuthError::CredentialInvalid
        );
        assert_eq!(
            BhoonidhiTokenGrant::new("", None, Some(60)).unwrap_err(),
            BhoonidhiAuthError::ResponseInvalid
        );
        assert_eq!(
            BhoonidhiTokenGrant::new("token", Some("refresh".to_owned()), Some(0)).unwrap_err(),
            BhoonidhiAuthError::ResponseInvalid
        );
    }
}
