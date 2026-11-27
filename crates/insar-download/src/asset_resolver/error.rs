use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveErrorKind {
    ResolverNotConfigured,
    ReferenceInvalid,
    DescriptorInvalid,
    AccessNotDurable,
    CredentialsMissing,
    AuthFailed,
    BurstCookieRequired,
    Cancelled,
    Timeout,
    Expired,
    Failed,
    RefreshBudgetExhausted,
    /// The remote service is temporarily unavailable (5xx, 429, 412) or the
    /// connection/body was lost: transient, never an identity conflict.
    Unavailable,
}

impl ResolveErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::ResolverNotConfigured => "ASSET_RESOLVER_NOT_CONFIGURED",
            Self::ReferenceInvalid => "ASSET_REFERENCE_INVALID",
            Self::DescriptorInvalid => "ASSET_ACCESS_DESCRIPTOR_INVALID",
            Self::AccessNotDurable => "ASSET_ACCESS_NOT_DURABLE",
            Self::CredentialsMissing => "ASSET_CREDENTIALS_MISSING",
            Self::AuthFailed => "ASSET_AUTH_FAILED",
            Self::BurstCookieRequired => "ASF_BURST_COOKIE_REQUIRED",
            Self::Cancelled => "ASSET_RESOLUTION_CANCELLED",
            Self::Timeout => "ASSET_RESOLUTION_TIMEOUT",
            Self::Expired => "ASSET_RESOLUTION_EXPIRED",
            Self::Failed => "ASSET_RESOLUTION_FAILED",
            Self::RefreshBudgetExhausted => "ASSET_REFRESH_BUDGET_EXHAUSTED",
            Self::Unavailable => "ASSET_RESOLUTION_UNAVAILABLE",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::ResolverNotConfigured => "未配置可处理该资产的解析器",
            Self::ReferenceInvalid => "资产稳定引用无效",
            Self::DescriptorInvalid => "资产访问描述无效",
            Self::AccessNotDurable => "资产访问地址不是可持久化的稳定定位符",
            Self::CredentialsMissing => "缺少资产解析所需的凭据",
            Self::AuthFailed => "资产解析认证失败",
            Self::BurstCookieRequired => {
                "Sentinel-1 Burst 下载需要账号密码或 .netrc 建立会话 Cookie"
            }
            Self::Cancelled => "资产解析已取消",
            Self::Timeout => "资产解析超时",
            Self::Expired => "资产解析结果已经过期",
            Self::Failed => "资产解析失败",
            Self::RefreshBudgetExhausted => "资产认证刷新次数已达到上限",
            Self::Unavailable => "资产服务暂时不可用",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResolveError {
    kind: ResolveErrorKind,
    diagnostic: Option<String>,
}

impl ResolveError {
    pub const fn new(kind: ResolveErrorKind) -> Self {
        Self {
            kind,
            diagnostic: None,
        }
    }

    pub const fn not_configured() -> Self {
        Self::new(ResolveErrorKind::ResolverNotConfigured)
    }

    pub const fn reference_invalid() -> Self {
        Self::new(ResolveErrorKind::ReferenceInvalid)
    }

    pub const fn descriptor_invalid() -> Self {
        Self::new(ResolveErrorKind::DescriptorInvalid)
    }

    pub const fn access_not_durable() -> Self {
        Self::new(ResolveErrorKind::AccessNotDurable)
    }

    pub const fn credentials_missing() -> Self {
        Self::new(ResolveErrorKind::CredentialsMissing)
    }

    pub const fn auth_failed() -> Self {
        Self::new(ResolveErrorKind::AuthFailed)
    }

    pub const fn burst_cookie_required() -> Self {
        Self::new(ResolveErrorKind::BurstCookieRequired)
    }

    pub const fn cancelled() -> Self {
        Self::new(ResolveErrorKind::Cancelled)
    }

    pub const fn timeout() -> Self {
        Self::new(ResolveErrorKind::Timeout)
    }

    pub const fn expired() -> Self {
        Self::new(ResolveErrorKind::Expired)
    }

    pub const fn failed() -> Self {
        Self::new(ResolveErrorKind::Failed)
    }

    pub const fn refresh_budget_exhausted() -> Self {
        Self::new(ResolveErrorKind::RefreshBudgetExhausted)
    }

    pub const fn unavailable() -> Self {
        Self::new(ResolveErrorKind::Unavailable)
    }

    pub fn code(&self) -> &'static str {
        self.kind.code()
    }

    pub fn kind(&self) -> ResolveErrorKind {
        self.kind
    }

    pub fn with_diagnostic(mut self, diagnostic: impl Into<String>) -> Self {
        let diagnostic = diagnostic.into();
        if !diagnostic.trim().is_empty() {
            self.diagnostic = Some(diagnostic);
        }
        self
    }

    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

impl fmt::Debug for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolveError")
            .field("code", &self.code())
            .field("diagnostic", &self.diagnostic)
            .finish()
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            self.diagnostic
                .as_deref()
                .unwrap_or_else(|| self.kind.message()),
        )
    }
}

impl std::error::Error for ResolveError {}
