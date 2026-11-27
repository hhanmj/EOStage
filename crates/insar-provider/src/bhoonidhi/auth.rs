use std::fmt;

use thiserror::Error;
use zeroize::Zeroizing;

/// Runtime-only token lease used by the Bhoonidhi catalogue request.
///
/// The generation is an in-process identity for the exact token value used by
/// one request. It is never serialized or included in diagnostics.
pub struct BhoonidhiAccessTokenLease {
    value: Zeroizing<String>,
    generation: u64,
}

impl BhoonidhiAccessTokenLease {
    pub fn new(value: Zeroizing<String>, generation: u64) -> Result<Self, BhoonidhiAuthError> {
        validate_access_token(&value)?;
        Ok(Self { value, generation })
    }

    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

impl AsRef<str> for BhoonidhiAccessTokenLease {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for BhoonidhiAccessTokenLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BhoonidhiAccessTokenLease")
            .field("value", &"[REDACTED]")
            .field("generation", &self.generation)
            .finish()
    }
}

/// Result of one generation-bound refresh attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BhoonidhiRefreshResult {
    /// The implementation has no generation-aware refresh capability.
    Unsupported,
    /// This call replaced the rejected generation.
    Refreshed,
    /// Another caller already replaced the rejected generation.
    AlreadyRefreshed,
}

/// Runtime-only source of a Bhoonidhi bearer access token.
///
/// Implementations own credential storage, session refresh and logout. The
/// catalogue client only borrows the returned token for one request and never
/// persists or includes it in diagnostics, cursors or catalogue products.
pub trait BhoonidhiAccessTokenProvider: Send + Sync {
    fn access_token(&self) -> Result<Zeroizing<String>, BhoonidhiAuthError>;

    /// Returns the token plus the generation that a request should retain
    /// until it receives an authentication rejection.
    fn access_token_with_generation(
        &self,
    ) -> Result<BhoonidhiAccessTokenLease, BhoonidhiAuthError> {
        self.access_token()
            .and_then(|value| BhoonidhiAccessTokenLease::new(value, 0))
    }

    /// Performs at most the one refresh associated with the rejected
    /// generation. Implementations without a process-owned session retain the
    /// old no-refresh behavior through `Unsupported`.
    fn refresh_after_rejection(
        &self,
        _generation: u64,
    ) -> Result<BhoonidhiRefreshResult, BhoonidhiAuthError> {
        Ok(BhoonidhiRefreshResult::Unsupported)
    }

    /// Remember rejection of the replacement bearer without another request.
    /// Providers without a process-owned session need no additional state.
    fn mark_generation_rejected(&self, _generation: u64) -> Result<(), BhoonidhiAuthError> {
        Ok(())
    }
}

/// Credential/session failures that can be reported without exposing secret
/// material. The native authentication agent can map its platform-specific
/// failures to these closed categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum BhoonidhiAuthError {
    #[error("Bhoonidhi credentials are unavailable")]
    Missing,
    #[error("Bhoonidhi authentication is not activated")]
    NotActivated,
    #[error("Bhoonidhi authentication is unavailable")]
    Unavailable,
    #[error("Bhoonidhi returned an invalid access token")]
    InvalidToken,
    #[error("Bhoonidhi credential content is invalid")]
    CredentialInvalid,
    #[error("Bhoonidhi authentication was rejected")]
    Rejected,
    #[error("Bhoonidhi active session limit was reached")]
    MaxSessions,
    #[error("Bhoonidhi authentication was rate limited")]
    RateLimited,
    #[error("Bhoonidhi authentication transport failed")]
    Transport,
    #[error("Bhoonidhi authentication timed out")]
    Timeout,
    #[error("Bhoonidhi authentication response is invalid")]
    ResponseInvalid,
    #[error("Bhoonidhi refresh token is unavailable")]
    RefreshUnavailable,
    #[error("Bhoonidhi refresh was already exhausted")]
    RefreshExhausted,
    #[error("Bhoonidhi session changed")]
    SessionChanged,
    #[error("Bhoonidhi session has no access token")]
    SessionUnavailable,
    #[error("Bhoonidhi authentication synchronization failed")]
    Synchronization,
    #[error("Bhoonidhi OAuth configuration is invalid")]
    Configuration,
}

impl BhoonidhiAuthError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "BHOONIDHI_CREDENTIALS_MISSING",
            Self::NotActivated => "BHOONIDHI_ACTIVATION_REQUIRED",
            Self::Unavailable => "BHOONIDHI_AUTH_UNAVAILABLE",
            Self::InvalidToken => "BHOONIDHI_AUTH_UNAVAILABLE",
            Self::CredentialInvalid => "BHOONIDHI_CREDENTIALS_INVALID",
            Self::Rejected => "BHOONIDHI_AUTH_REJECTED",
            Self::MaxSessions => "BHOONIDHI_MAX_SESSIONS",
            Self::RateLimited => "BHOONIDHI_RATE_LIMITED",
            Self::Transport => "BHOONIDHI_AUTH_TRANSPORT_FAILED",
            Self::Timeout => "BHOONIDHI_AUTH_TIMEOUT",
            Self::ResponseInvalid => "BHOONIDHI_AUTH_RESPONSE_INVALID",
            Self::RefreshUnavailable => "BHOONIDHI_REFRESH_UNAVAILABLE",
            Self::RefreshExhausted => "BHOONIDHI_REFRESH_EXHAUSTED",
            Self::SessionChanged => "BHOONIDHI_SESSION_CHANGED",
            Self::SessionUnavailable => "BHOONIDHI_SESSION_UNAVAILABLE",
            Self::Synchronization => "BHOONIDHI_SYNCHRONIZATION_FAILED",
            Self::Configuration => "BHOONIDHI_OAUTH_CONFIGURATION_INVALID",
        }
    }

    pub const fn reason(self) -> &'static str {
        match self {
            Self::Missing => "credentials_missing",
            Self::NotActivated => "api_not_activated",
            Self::Unavailable => "auth_unavailable",
            Self::InvalidToken => "token_invalid",
            Self::CredentialInvalid => "credentials_invalid",
            Self::Rejected => "auth_rejected",
            Self::MaxSessions => "max_sessions",
            Self::RateLimited => "rate_limited",
            Self::Transport => "auth_transport",
            Self::Timeout => "auth_timeout",
            Self::ResponseInvalid => "auth_response_invalid",
            Self::RefreshUnavailable => "refresh_unavailable",
            Self::RefreshExhausted => "refresh_exhausted",
            Self::SessionChanged => "session_changed",
            Self::SessionUnavailable => "session_unavailable",
            Self::Synchronization => "synchronization",
            Self::Configuration => "oauth_configuration",
        }
    }

    pub const fn user_message(self) -> &'static str {
        match self {
            Self::Missing => "Bhoonidhi 凭据缺失，请在设置中保存账号密码。",
            Self::NotActivated => "Bhoonidhi 账户尚未激活，请先完成账户激活。",
            Self::Unavailable => "Bhoonidhi 认证当前不可用，请稍后重试。",
            Self::InvalidToken => "Bhoonidhi 会话令牌无效，请重新发起检索。",
            Self::CredentialInvalid => "Bhoonidhi 凭据格式无效，请重新保存。",
            Self::Rejected => "Bhoonidhi 认证被服务拒绝，请检查账号状态。",
            Self::MaxSessions => "Bhoonidhi 活跃会话已达上限，请关闭其他会话后重试。",
            Self::RateLimited => "Bhoonidhi 认证请求受到限流，请稍后重试。",
            Self::Transport => "Bhoonidhi 认证网络连接失败，请检查网络或代理设置。",
            Self::Timeout => "Bhoonidhi 认证请求超时，请稍后重试。",
            Self::ResponseInvalid => "Bhoonidhi 认证响应无效，请稍后重试。",
            Self::RefreshUnavailable => "Bhoonidhi 会话无法刷新，请重新发起检索。",
            Self::RefreshExhausted => "Bhoonidhi 会话刷新已用尽，请重新发起检索。",
            Self::SessionChanged => "Bhoonidhi 会话已变更，请重新发起检索。",
            Self::SessionUnavailable => "Bhoonidhi 当前没有可用会话，请先完成登录。",
            Self::Synchronization => "Bhoonidhi 会话同步失败，请重新发起检索。",
            Self::Configuration => "Bhoonidhi OAuth 配置无效，请检查应用配置。",
        }
    }
}

impl<T> BhoonidhiAccessTokenProvider for std::sync::Arc<T>
where
    T: BhoonidhiAccessTokenProvider + ?Sized,
{
    fn access_token(&self) -> Result<Zeroizing<String>, BhoonidhiAuthError> {
        (**self).access_token()
    }

    fn access_token_with_generation(
        &self,
    ) -> Result<BhoonidhiAccessTokenLease, BhoonidhiAuthError> {
        (**self).access_token_with_generation()
    }

    fn refresh_after_rejection(
        &self,
        generation: u64,
    ) -> Result<BhoonidhiRefreshResult, BhoonidhiAuthError> {
        (**self).refresh_after_rejection(generation)
    }

    fn mark_generation_rejected(&self, generation: u64) -> Result<(), BhoonidhiAuthError> {
        (**self).mark_generation_rejected(generation)
    }
}

pub(crate) fn validate_access_token(token: &Zeroizing<String>) -> Result<(), BhoonidhiAuthError> {
    let value = token.as_str();
    if value.is_empty()
        || value != value.trim()
        || value.chars().any(char::is_control)
        || value.len() > 16 * 1024
    {
        return Err(BhoonidhiAuthError::InvalidToken);
    }
    Ok(())
}
