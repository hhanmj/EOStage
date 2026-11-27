use std::fmt;
use std::io::Read;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use insar_core::ProviderNetworkPolicy;
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::asset_resolver::cdse::CDSE_CREDENTIAL_PROFILE;
use crate::asset_resolver::{
    Clock, CredentialMaterial, CredentialProvider, CredentialProviderError,
};

pub const CDSE_IDENTITY_TOKEN_ENDPOINT: &str =
    "https://identity.dataspace.copernicus.eu/auth/realms/CDSE/protocol/openid-connect/token";
pub const CDSE_OAUTH_CLIENT_ID: &str = "cdse-public";

const MAX_LOGIN_FIELD_BYTES: usize = 4 * 1024;
const MAX_TOKEN_BYTES: usize = 16 * 1024;
const MAX_TOKEN_RESPONSE_BYTES: u64 = 64 * 1024;
const MAX_TOKEN_LIFETIME_SECONDS: u64 = 24 * 60 * 60;
const IDENTITY_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IDENTITY_CONTROL_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum CdseAuthError {
    #[error("CDSE OAuth 配置无效")]
    Configuration,
    #[error("CDSE 登录信息无效")]
    LoginMaterialInvalid,
    #[error("CDSE OAuth 传输失败")]
    Transport,
    #[error("CDSE OAuth 请求超时")]
    Timeout,
    #[error("CDSE OAuth 被服务拒绝")]
    Rejected,
    #[error("CDSE OAuth 响应无效")]
    ResponseInvalid,
    #[error("CDSE 刷新令牌不可用")]
    RefreshUnavailable,
    #[error("CDSE 当前令牌已完成一次刷新尝试")]
    RefreshExhausted,
    #[error("CDSE 会话已被注销或替换")]
    SessionChanged,
    #[error("CDSE 会话没有可用访问令牌")]
    SessionUnavailable,
    #[error("CDSE 访问令牌已过期")]
    AccessExpired,
    #[error("CDSE OAuth 会话同步失败")]
    Synchronization,
}

impl CdseAuthError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Configuration => "CDSE_OAUTH_CONFIGURATION_INVALID",
            Self::LoginMaterialInvalid => "CDSE_OAUTH_LOGIN_INVALID",
            Self::Transport => "CDSE_OAUTH_TRANSPORT_FAILED",
            Self::Timeout => "CDSE_OAUTH_TIMEOUT",
            Self::Rejected => "CDSE_OAUTH_REJECTED",
            Self::ResponseInvalid => "CDSE_OAUTH_RESPONSE_INVALID",
            Self::RefreshUnavailable => "CDSE_OAUTH_REFRESH_UNAVAILABLE",
            Self::RefreshExhausted => "CDSE_OAUTH_REFRESH_EXHAUSTED",
            Self::SessionChanged => "CDSE_OAUTH_SESSION_CHANGED",
            Self::SessionUnavailable => "CDSE_OAUTH_SESSION_UNAVAILABLE",
            Self::AccessExpired => "CDSE_OAUTH_ACCESS_EXPIRED",
            Self::Synchronization => "CDSE_OAUTH_SYNCHRONIZATION_FAILED",
        }
    }
}

/// Caller-owned CDSE password-grant material. It intentionally has no
/// serialization implementation and zeroizes all fields when dropped.
pub struct CdseLoginMaterial {
    username: Zeroizing<String>,
    password: Zeroizing<String>,
    totp: Option<Zeroizing<String>>,
}

impl CdseLoginMaterial {
    pub fn new(
        username: impl Into<String>,
        password: impl Into<String>,
        totp: Option<String>,
    ) -> Result<Self, CdseAuthError> {
        let username = username.into();
        let password = password.into();
        if invalid_login_field(&username, true) || invalid_login_field(&password, false) {
            return Err(CdseAuthError::LoginMaterialInvalid);
        }
        let totp = match totp {
            None => None,
            Some(value) if value.is_empty() => None,
            Some(value)
                if value.len() >= 6
                    && value.len() <= 12
                    && value.as_bytes().iter().all(u8::is_ascii_digit) =>
            {
                Some(Zeroizing::new(value))
            }
            Some(_) => return Err(CdseAuthError::LoginMaterialInvalid),
        };
        Ok(Self {
            username: Zeroizing::new(username),
            password: Zeroizing::new(password),
            totp,
        })
    }

    fn username(&self) -> &str {
        self.username.as_str()
    }

    fn password(&self) -> &str {
        self.password.as_str()
    }

    fn totp(&self) -> Option<&str> {
        self.totp.as_ref().map(|value| value.as_str())
    }
}

impl fmt::Debug for CdseLoginMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CdseLoginMaterial")
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .field("totp", &self.totp.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

fn invalid_login_field(value: &str, trim_required: bool) -> bool {
    value.is_empty()
        || value.len() > MAX_LOGIN_FIELD_BYTES
        || value.contains(['\r', '\n', '\0'])
        || (trim_required && value.trim().is_empty())
}

/// Token data returned by a CDSE endpoint. It is runtime-only and all secret
/// fields are zeroized when ownership moves or the value is dropped.
pub struct CdseTokenGrant {
    access_token: Zeroizing<String>,
    refresh_token: Option<Zeroizing<String>>,
    expires_in: Option<Duration>,
}

impl CdseTokenGrant {
    pub fn new(
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_in_seconds: Option<u64>,
    ) -> Result<Self, CdseAuthError> {
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
    ) -> Result<Self, CdseAuthError> {
        if invalid_token(access_token.as_str()) {
            return Err(CdseAuthError::ResponseInvalid);
        }
        if refresh_token
            .as_ref()
            .is_some_and(|value| invalid_token(value.as_str()))
        {
            return Err(CdseAuthError::ResponseInvalid);
        }
        let expires_in = match expires_in_seconds {
            Some(seconds) if seconds == 0 || seconds > MAX_TOKEN_LIFETIME_SECONDS => {
                return Err(CdseAuthError::ResponseInvalid)
            }
            Some(seconds) => Some(Duration::from_secs(seconds)),
            None => None,
        };
        Ok(Self {
            access_token,
            refresh_token,
            expires_in,
        })
    }
}

impl fmt::Debug for CdseTokenGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CdseTokenGrant")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

fn invalid_token(value: &str) -> bool {
    value.is_empty() || value.len() > MAX_TOKEN_BYTES || value.contains(['\r', '\n', '\0'])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdseTokenSnapshot {
    pub authenticated: bool,
    pub generation: Option<u64>,
    pub refresh_available: bool,
    pub expires_in_seconds: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdseRefreshOutcome {
    NotNeeded(CdseTokenSnapshot),
    Refreshed(CdseTokenSnapshot),
    AlreadyRefreshed(CdseTokenSnapshot),
}

/// Injectable CDSE OAuth endpoint. Production construction is exact-origin;
/// tests use this port to stay completely offline.
pub trait CdseTokenEndpoint: Send + Sync {
    fn network_policy(&self) -> ProviderNetworkPolicy;
    fn password_grant(&self, material: &CdseLoginMaterial)
        -> Result<CdseTokenGrant, CdseAuthError>;
    fn refresh_grant(&self, refresh_token: &str) -> Result<CdseTokenGrant, CdseAuthError>;
}

pub struct ReqwestCdseTokenEndpoint {
    client: Client,
    endpoint: Url,
    network_policy: ProviderNetworkPolicy,
}

impl ReqwestCdseTokenEndpoint {
    pub fn new(network_policy: ProviderNetworkPolicy) -> Result<Self, CdseAuthError> {
        let endpoint =
            Url::parse(CDSE_IDENTITY_TOKEN_ENDPOINT).map_err(|_| CdseAuthError::Configuration)?;
        validate_identity_endpoint(&endpoint)?;
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
        .map_err(|_| CdseAuthError::Configuration)?;
        Ok(Self {
            client,
            endpoint,
            network_policy,
        })
    }

    pub const fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    fn exchange(&self, fields: &[(&str, &str)]) -> Result<CdseTokenGrant, CdseAuthError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .form(fields)
            .send()
            .map_err(map_request_error)?;
        if !response.status().is_success() {
            return Err(map_status_error(response.status()));
        }
        parse_token_response(response)
    }
}

impl CdseTokenEndpoint for ReqwestCdseTokenEndpoint {
    fn network_policy(&self) -> ProviderNetworkPolicy {
        self.network_policy
    }

    fn password_grant(
        &self,
        material: &CdseLoginMaterial,
    ) -> Result<CdseTokenGrant, CdseAuthError> {
        self.exchange(&password_grant_fields(material))
    }

    fn refresh_grant(&self, refresh_token: &str) -> Result<CdseTokenGrant, CdseAuthError> {
        self.exchange(&refresh_grant_fields(refresh_token))
    }
}

fn password_grant_fields(material: &CdseLoginMaterial) -> Vec<(&str, &str)> {
    let mut fields = vec![
        ("client_id", CDSE_OAUTH_CLIENT_ID),
        ("username", material.username()),
        ("password", material.password()),
        ("grant_type", "password"),
    ];
    if let Some(totp) = material.totp() {
        fields.push(("totp", totp));
    }
    fields
}

fn refresh_grant_fields(refresh_token: &str) -> [(&str, &str); 3] {
    [
        ("client_id", CDSE_OAUTH_CLIENT_ID),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ]
}

fn validate_identity_endpoint(endpoint: &Url) -> Result<(), CdseAuthError> {
    if endpoint.scheme() != "https"
        || endpoint.host_str() != Some("identity.dataspace.copernicus.eu")
        || endpoint.port_or_known_default() != Some(443)
        || endpoint.path() != "/auth/realms/CDSE/protocol/openid-connect/token"
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(CdseAuthError::Configuration);
    }
    Ok(())
}

fn map_request_error(error: reqwest::Error) -> CdseAuthError {
    if error.is_timeout() {
        CdseAuthError::Timeout
    } else {
        CdseAuthError::Transport
    }
}

fn map_status_error(status: StatusCode) -> CdseAuthError {
    if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        CdseAuthError::Rejected
    } else if status == StatusCode::REQUEST_TIMEOUT || status == StatusCode::GATEWAY_TIMEOUT {
        CdseAuthError::Timeout
    } else {
        CdseAuthError::Transport
    }
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
                formatter.write_str("a CDSE OAuth token string")
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
    access_token: WireSecret,
    #[serde(default)]
    refresh_token: Option<WireSecret>,
    #[serde(default)]
    expires_in: Option<u64>,
}

fn parse_token_response(response: Response) -> Result<CdseTokenGrant, CdseAuthError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_TOKEN_RESPONSE_BYTES)
    {
        return Err(CdseAuthError::ResponseInvalid);
    }
    let mut body = Zeroizing::new(Vec::new());
    response
        .take(MAX_TOKEN_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| CdseAuthError::Transport)?;
    if body.len() as u64 > MAX_TOKEN_RESPONSE_BYTES {
        return Err(CdseAuthError::ResponseInvalid);
    }
    let TokenResponse {
        access_token,
        refresh_token,
        expires_in,
    } = serde_json::from_slice(&body).map_err(|_| CdseAuthError::ResponseInvalid)?;
    CdseTokenGrant::from_secret_parts(
        access_token.0,
        refresh_token.map(|token| token.0),
        expires_in,
    )
}

struct SessionTokens {
    access_token: Zeroizing<String>,
    refresh_token: Option<Zeroizing<String>>,
    expires_at: Option<SystemTime>,
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
}

/// Runtime-only CDSE OAuth session. It never owns durable credential storage;
/// callers explicitly supply login material and may explicitly logout.
pub struct CdseOAuthSession {
    network_policy: ProviderNetworkPolicy,
    endpoint: Arc<dyn CdseTokenEndpoint>,
    clock: Arc<dyn Clock>,
    state: Mutex<SessionState>,
    changed: Condvar,
}

impl CdseOAuthSession {
    pub fn new(
        network_policy: ProviderNetworkPolicy,
        endpoint: Arc<dyn CdseTokenEndpoint>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, CdseAuthError> {
        if endpoint.network_policy() != network_policy {
            return Err(CdseAuthError::Configuration);
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

    pub fn login(&self, material: CdseLoginMaterial) -> Result<CdseTokenSnapshot, CdseAuthError> {
        let epoch = self.clear_state()?;
        let grant = self.endpoint.password_grant(&material)?;
        let mut state = self.lock_state()?;
        if state.epoch != epoch {
            return Err(CdseAuthError::SessionChanged);
        }
        install_grant(&mut state, grant, self.clock.as_ref())?;
        self.changed.notify_all();
        Ok(snapshot(&state, self.clock.as_ref()))
    }

    pub fn snapshot(&self) -> Result<CdseTokenSnapshot, CdseAuthError> {
        let state = self.lock_state()?;
        Ok(snapshot(&state, self.clock.as_ref()))
    }

    pub fn refresh_after_rejection(
        &self,
        generation: u64,
    ) -> Result<CdseRefreshOutcome, CdseAuthError> {
        self.refresh_generation(generation)
    }

    pub fn refresh_if_expired(&self) -> Result<CdseRefreshOutcome, CdseAuthError> {
        let state = self.lock_state()?;
        let Some(tokens) = state.tokens.as_ref() else {
            return Err(CdseAuthError::SessionUnavailable);
        };
        if !is_expired(tokens, self.clock.as_ref()) {
            return Ok(CdseRefreshOutcome::NotNeeded(snapshot(
                &state,
                self.clock.as_ref(),
            )));
        }
        let generation = tokens.generation;
        drop(state);
        self.refresh_generation(generation)
    }

    pub fn bearer_token_for_download(&self) -> Result<(u64, Zeroizing<String>), CdseAuthError> {
        let _ = self.refresh_if_expired()?;
        let state = self.lock_state()?;
        let tokens = state
            .tokens
            .as_ref()
            .ok_or(CdseAuthError::SessionUnavailable)?;
        Ok((
            tokens.generation,
            Zeroizing::new(tokens.access_token.to_string()),
        ))
    }

    pub fn logout(&self) -> Result<(), CdseAuthError> {
        self.clear_state().map(|_| ())
    }

    fn clear_state(&self) -> Result<u64, CdseAuthError> {
        let mut state = self.lock_state()?;
        state.epoch = state
            .epoch
            .checked_add(1)
            .ok_or(CdseAuthError::Synchronization)?;
        state.tokens = None;
        state.refreshing = None;
        state.failed_refresh = None;
        self.changed.notify_all();
        Ok(state.epoch)
    }

    fn refresh_generation(
        &self,
        required_generation: u64,
    ) -> Result<CdseRefreshOutcome, CdseAuthError> {
        let (attempt, refresh_token) = {
            let mut state = self.lock_state()?;
            loop {
                let Some(tokens) = state.tokens.as_ref() else {
                    return Err(CdseAuthError::SessionUnavailable);
                };
                if tokens.generation != required_generation {
                    return Ok(CdseRefreshOutcome::AlreadyRefreshed(snapshot(
                        &state,
                        self.clock.as_ref(),
                    )));
                }
                let attempt = RefreshAttempt {
                    epoch: state.epoch,
                    generation: tokens.generation,
                };
                if state.failed_refresh == Some(attempt) {
                    return Err(CdseAuthError::RefreshExhausted);
                }
                if state.refreshing == Some(attempt) {
                    state = self
                        .changed
                        .wait(state)
                        .map_err(|_| CdseAuthError::Synchronization)?;
                    continue;
                }
                let Some(refresh_token) = tokens.refresh_token.as_ref() else {
                    return Err(CdseAuthError::RefreshUnavailable);
                };
                let refresh_token = refresh_token.to_string();
                state.refreshing = Some(attempt);
                break (attempt, Zeroizing::new(refresh_token));
            }
        };

        let refreshed = self.endpoint.refresh_grant(refresh_token.as_str());
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
            return Err(CdseAuthError::SessionChanged);
        }
        match refreshed {
            Ok(grant) => {
                install_grant(&mut state, grant, self.clock.as_ref())?;
                self.changed.notify_all();
                Ok(CdseRefreshOutcome::Refreshed(snapshot(
                    &state,
                    self.clock.as_ref(),
                )))
            }
            Err(error) => {
                state.failed_refresh = Some(attempt);
                self.changed.notify_all();
                Err(error)
            }
        }
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, SessionState>, CdseAuthError> {
        self.state
            .lock()
            .map_err(|_| CdseAuthError::Synchronization)
    }
}

impl CredentialProvider for CdseOAuthSession {
    fn resolve(&self, profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
        if profile != CDSE_CREDENTIAL_PROFILE {
            return Err(CredentialProviderError::Missing);
        }
        let state = self
            .lock_state()
            .map_err(|_| CredentialProviderError::Failed)?;
        let tokens = state
            .tokens
            .as_ref()
            .ok_or(CredentialProviderError::Missing)?;
        if is_expired(tokens, self.clock.as_ref()) {
            return Err(CredentialProviderError::Timeout);
        }
        CredentialMaterial::bearer(tokens.access_token.to_string())
    }
}

impl fmt::Debug for CdseOAuthSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CdseOAuthSession")
            .field("network_policy", &self.network_policy)
            .field("runtime_material", &"[REDACTED]")
            .finish()
    }
}

fn install_grant(
    state: &mut SessionState,
    grant: CdseTokenGrant,
    clock: &dyn Clock,
) -> Result<(), CdseAuthError> {
    state.next_generation = state
        .next_generation
        .checked_add(1)
        .ok_or(CdseAuthError::Synchronization)?;
    let expires_at = match grant.expires_in {
        Some(duration) => Some(
            clock
                .now()
                .checked_add(duration)
                .ok_or(CdseAuthError::ResponseInvalid)?,
        ),
        None => None,
    };
    state.tokens = Some(SessionTokens {
        access_token: grant.access_token,
        refresh_token: grant.refresh_token,
        expires_at,
        generation: state.next_generation,
    });
    state.failed_refresh = None;
    Ok(())
}

fn is_expired(tokens: &SessionTokens, clock: &dyn Clock) -> bool {
    tokens
        .expires_at
        .is_some_and(|expires_at| expires_at <= clock.now())
}

fn snapshot(state: &SessionState, clock: &dyn Clock) -> CdseTokenSnapshot {
    match state.tokens.as_ref() {
        None => CdseTokenSnapshot {
            authenticated: false,
            generation: None,
            refresh_available: false,
            expires_in_seconds: None,
        },
        Some(tokens) => CdseTokenSnapshot {
            authenticated: true,
            generation: Some(tokens.generation),
            refresh_available: tokens.refresh_token.is_some(),
            expires_in_seconds: tokens.expires_at.map(|expires_at| {
                expires_at
                    .duration_since(clock.now())
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0)
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;
    use std::thread;

    use super::*;

    struct FixedClock(SystemTime);

    impl Clock for FixedClock {
        fn now(&self) -> SystemTime {
            self.0
        }
    }

    struct FixtureEndpoint {
        policy: ProviderNetworkPolicy,
        password_calls: AtomicUsize,
        refresh_calls: AtomicUsize,
        reject_refresh: bool,
    }

    impl FixtureEndpoint {
        fn new(policy: ProviderNetworkPolicy) -> Self {
            Self {
                policy,
                password_calls: AtomicUsize::new(0),
                refresh_calls: AtomicUsize::new(0),
                reject_refresh: false,
            }
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

    impl CdseTokenEndpoint for FixtureEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn password_grant(
            &self,
            material: &CdseLoginMaterial,
        ) -> Result<CdseTokenGrant, CdseAuthError> {
            self.password_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(material.username(), "fixture-user");
            assert_eq!(material.password(), "fixture-password");
            assert_eq!(material.totp(), Some("123456"));
            CdseTokenGrant::new(
                "fixture-access-one",
                Some("fixture-refresh-one".to_owned()),
                Some(60),
            )
        }

        fn refresh_grant(&self, refresh_token: &str) -> Result<CdseTokenGrant, CdseAuthError> {
            self.refresh_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(refresh_token, "fixture-refresh-one");
            if self.reject_refresh {
                return Err(CdseAuthError::Rejected);
            }
            thread::sleep(Duration::from_millis(20));
            CdseTokenGrant::new(
                "fixture-access-two",
                Some("fixture-refresh-two".to_owned()),
                Some(60),
            )
        }
    }

    impl CdseTokenEndpoint for BlockingRefreshEndpoint {
        fn network_policy(&self) -> ProviderNetworkPolicy {
            self.policy
        }

        fn password_grant(
            &self,
            material: &CdseLoginMaterial,
        ) -> Result<CdseTokenGrant, CdseAuthError> {
            assert_eq!(material.username(), "fixture-user");
            assert_eq!(material.password(), "fixture-password");
            assert_eq!(material.totp(), Some("123456"));
            CdseTokenGrant::new(
                "fixture-access-one",
                Some("fixture-refresh-one".to_owned()),
                Some(60),
            )
        }

        fn refresh_grant(&self, refresh_token: &str) -> Result<CdseTokenGrant, CdseAuthError> {
            assert_eq!(refresh_token, "fixture-refresh-one");
            self.refresh_started.wait();
            self.release_refresh.wait();
            CdseTokenGrant::new(
                "fixture-access-two",
                Some("fixture-refresh-two".to_owned()),
                Some(60),
            )
        }
    }

    fn session(
        endpoint: Arc<FixtureEndpoint>,
        policy: ProviderNetworkPolicy,
    ) -> Arc<CdseOAuthSession> {
        Arc::new(
            CdseOAuthSession::new(
                policy,
                endpoint,
                Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
            )
            .expect("matching fixture policy"),
        )
    }

    fn login_material() -> CdseLoginMaterial {
        CdseLoginMaterial::new(
            "fixture-user",
            "fixture-password",
            Some("123456".to_owned()),
        )
        .expect("valid fixture login")
    }

    #[test]
    fn password_grant_form_has_exact_fields_and_only_adds_supplied_totp() {
        let with_totp = login_material();
        assert_eq!(
            password_grant_fields(&with_totp),
            vec![
                ("client_id", "cdse-public"),
                ("username", "fixture-user"),
                ("password", "fixture-password"),
                ("grant_type", "password"),
                ("totp", "123456"),
            ]
        );

        let without_totp = CdseLoginMaterial::new("fixture-user", "fixture-password", None)
            .expect("valid fixture login without TOTP");
        assert_eq!(
            password_grant_fields(&without_totp),
            vec![
                ("client_id", "cdse-public"),
                ("username", "fixture-user"),
                ("password", "fixture-password"),
                ("grant_type", "password"),
            ]
        );
    }

    #[test]
    fn refresh_grant_form_has_exact_fields() {
        assert_eq!(
            refresh_grant_fields("fixture-refresh-one"),
            [
                ("client_id", "cdse-public"),
                ("refresh_token", "fixture-refresh-one"),
                ("grant_type", "refresh_token"),
            ]
        );
    }

    #[test]
    fn token_response_moves_wire_secrets_directly_into_zeroizing_grant_storage() {
        let wire: TokenResponse = serde_json::from_str(
            r#"{
                "access_token": "fixture-access-wire",
                "refresh_token": "fixture-refresh-wire",
                "expires_in": 60
            }"#,
        )
        .expect("valid token response");
        let TokenResponse {
            access_token,
            refresh_token,
            expires_in,
        } = wire;
        let grant = CdseTokenGrant::from_secret_parts(
            access_token.0,
            refresh_token.map(|token| token.0),
            expires_in,
        )
        .expect("valid token grant");

        let debug = format!("{grant:?}");
        assert!(!debug.contains("fixture-access-wire"));
        assert!(!debug.contains("fixture-refresh-wire"));
        assert!(serde_json::from_str::<TokenResponse>(r#"{"access_token": 1}"#).is_err());
    }

    #[test]
    fn login_is_runtime_only_and_debug_never_exposes_login_or_token_material() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = session(Arc::clone(&endpoint), policy);
        let snapshot = session.login(login_material()).expect("login succeeds");

        assert!(snapshot.authenticated);
        assert_eq!(snapshot.generation, Some(1));
        assert!(snapshot.refresh_available);
        assert_eq!(endpoint.password_calls.load(Ordering::SeqCst), 1);
        let debug = format!("{session:?} {:?}", login_material());
        for secret in [
            "fixture-user",
            "fixture-password",
            "123456",
            "fixture-access-one",
            "fixture-refresh-one",
        ] {
            assert!(
                !debug.contains(secret),
                "secret leaked through Debug: {secret}"
            );
        }
        assert!(session.resolve(CDSE_CREDENTIAL_PROFILE).is_ok());
    }

    #[test]
    fn refresh_is_single_flight_and_bounded_per_generation() {
        let policy = ProviderNetworkPolicy::direct();
        let endpoint = Arc::new(FixtureEndpoint::new(policy));
        let session = session(Arc::clone(&endpoint), policy);
        let generation = session
            .login(login_material())
            .expect("login succeeds")
            .generation
            .expect("generation");

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
            .any(|outcome| matches!(outcome, CdseRefreshOutcome::Refreshed(_))));
        assert!(outcomes
            .iter()
            .any(|outcome| matches!(outcome, CdseRefreshOutcome::AlreadyRefreshed(_))));
        assert_eq!(session.snapshot().unwrap().generation, Some(2));
    }

    #[test]
    fn rejected_refresh_is_not_retried_for_the_same_generation_and_logout_clears_state() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(FixtureEndpoint {
            reject_refresh: true,
            ..FixtureEndpoint::new(policy)
        });
        let session = session(Arc::clone(&endpoint), policy);
        let generation = session.login(login_material()).unwrap().generation.unwrap();

        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(CdseAuthError::Rejected)
        );
        assert_eq!(
            session.refresh_after_rejection(generation),
            Err(CdseAuthError::RefreshExhausted)
        );
        assert_eq!(endpoint.refresh_calls.load(Ordering::SeqCst), 1);
        session.logout().expect("logout");
        assert!(!session.snapshot().unwrap().authenticated);
        assert!(matches!(
            session.resolve(CDSE_CREDENTIAL_PROFILE),
            Err(CredentialProviderError::Missing)
        ));
    }

    #[test]
    fn logout_invalidates_an_in_flight_refresh_and_clears_runtime_credentials() {
        let policy = ProviderNetworkPolicy::system();
        let endpoint = Arc::new(BlockingRefreshEndpoint::new(policy));
        let session_endpoint: Arc<dyn CdseTokenEndpoint> = endpoint.clone();
        let session = Arc::new(
            CdseOAuthSession::new(
                policy,
                session_endpoint,
                Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
            )
            .expect("matching fixture policy"),
        );
        let generation = session
            .login(login_material())
            .expect("login succeeds")
            .generation
            .expect("generation");

        let refreshing_session = Arc::clone(&session);
        let refreshing =
            thread::spawn(move || refreshing_session.refresh_after_rejection(generation));
        endpoint.refresh_started.wait();
        session.logout().expect("logout");
        endpoint.release_refresh.wait();

        assert_eq!(
            refreshing.join().expect("refresh thread"),
            Err(CdseAuthError::SessionChanged)
        );
        assert!(matches!(
            session.resolve(CDSE_CREDENTIAL_PROFILE),
            Err(CredentialProviderError::Missing)
        ));
        assert_eq!(
            session.snapshot().expect("snapshot after logout"),
            CdseTokenSnapshot {
                authenticated: false,
                generation: None,
                refresh_available: false,
                expires_in_seconds: None,
            }
        );
    }

    #[test]
    fn endpoint_configuration_is_exact_and_route_bound() {
        let direct_policy = ProviderNetworkPolicy::direct();
        let endpoint = ReqwestCdseTokenEndpoint::new(direct_policy).expect("reviewed endpoint");
        assert_eq!(endpoint.network_policy(), direct_policy);
        let system_policy = ProviderNetworkPolicy::system();
        let system_endpoint = ReqwestCdseTokenEndpoint::new(system_policy)
            .expect("reviewed endpoint with the system route");
        assert_eq!(system_endpoint.network_policy(), system_policy);
        assert_eq!(CDSE_IDENTITY_TOKEN_ENDPOINT, "https://identity.dataspace.copernicus.eu/auth/realms/CDSE/protocol/openid-connect/token");
        assert_eq!(CDSE_OAUTH_CLIENT_ID, "cdse-public");
        assert!(CdseOAuthSession::new(
            system_policy,
            Arc::new(endpoint),
            Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
        )
        .is_err());
        assert!(CdseOAuthSession::new(
            direct_policy,
            Arc::new(system_endpoint),
            Arc::new(FixedClock(SystemTime::UNIX_EPOCH)),
        )
        .is_err());
    }

    #[test]
    fn malformed_login_and_token_material_fail_closed() {
        assert_eq!(
            CdseLoginMaterial::new("", "password", None).unwrap_err(),
            CdseAuthError::LoginMaterialInvalid
        );
        assert_eq!(
            CdseLoginMaterial::new("user", "password", Some("abc".to_owned())).unwrap_err(),
            CdseAuthError::LoginMaterialInvalid
        );
        assert_eq!(
            CdseTokenGrant::new("", None, Some(60)).unwrap_err(),
            CdseAuthError::ResponseInvalid
        );
        assert_eq!(
            CdseTokenGrant::new("token", None, Some(0)).unwrap_err(),
            CdseAuthError::ResponseInvalid
        );
    }
}
