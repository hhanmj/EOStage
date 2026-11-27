use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mErrorCode {
    ConfigInvalid,
    AuthRequired,
    AuthInvalid,
    AuthUnauthorized,
    AuthExpired,
    RateLimited,
    RedirectRejected,
    TransportFailed,
    Timeout,
    ResponseTooLarge,
    ResponseInvalid,
    RemoteError,
    DatasetNotFound,
    DatasetAmbiguous,
    DatasetConflict,
    SceneConflict,
    SceneLimitExceeded,
    PaginationInvalid,
    DownloadOptionInvalid,
    DownloadOptionAmbiguous,
    DownloadUnavailable,
    DownloadIntegrityMissing,
    DownloadIdentityConflict,
    DownloadPreparationInvalid,
    Cancelled,
}

impl UsgsM2mErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigInvalid => "USGS_M2M_CONFIG_INVALID",
            Self::AuthRequired => "USGS_M2M_AUTH_REQUIRED",
            Self::AuthInvalid => "USGS_M2M_AUTH_INVALID",
            Self::AuthUnauthorized => "USGS_M2M_AUTH_UNAUTHORIZED",
            Self::AuthExpired => "USGS_M2M_AUTH_EXPIRED",
            Self::RateLimited => "USGS_M2M_RATE_LIMITED",
            Self::RedirectRejected => "USGS_M2M_REDIRECT_REJECTED",
            Self::TransportFailed => "USGS_M2M_TRANSPORT_FAILED",
            Self::Timeout => "USGS_M2M_TIMEOUT",
            Self::ResponseTooLarge => "USGS_M2M_RESPONSE_TOO_LARGE",
            Self::ResponseInvalid => "USGS_M2M_RESPONSE_INVALID",
            Self::RemoteError => "USGS_M2M_REMOTE_ERROR",
            Self::DatasetNotFound => "USGS_M2M_DATASET_NOT_FOUND",
            Self::DatasetAmbiguous => "USGS_M2M_DATASET_AMBIGUOUS",
            Self::DatasetConflict => "USGS_M2M_DATASET_CONFLICT",
            Self::SceneConflict => "USGS_M2M_SCENE_CONFLICT",
            Self::SceneLimitExceeded => "USGS_M2M_SCENE_LIMIT_EXCEEDED",
            Self::PaginationInvalid => "USGS_M2M_PAGINATION_INVALID",
            Self::DownloadOptionInvalid => "USGS_M2M_DOWNLOAD_OPTION_INVALID",
            Self::DownloadOptionAmbiguous => "USGS_M2M_DOWNLOAD_OPTION_AMBIGUOUS",
            Self::DownloadUnavailable => "USGS_M2M_DOWNLOAD_UNAVAILABLE",
            Self::DownloadIntegrityMissing => "USGS_M2M_DOWNLOAD_INTEGRITY_MISSING",
            Self::DownloadIdentityConflict => "USGS_M2M_DOWNLOAD_IDENTITY_CONFLICT",
            Self::DownloadPreparationInvalid => "USGS_M2M_DOWNLOAD_PREPARATION_INVALID",
            Self::Cancelled => "USGS_M2M_CANCELLED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsgsM2mEndpointClass {
    LoginToken,
    Permissions,
    DatasetDiscovery,
    DatasetFilters,
    SceneSearch,
    DownloadOptions,
    DownloadRequest,
    DownloadRetrieve,
    BrowseFetch,
    Logout,
    RuntimeSession,
    DiscoveryRouter,
    SceneSearchRouter,
}

impl UsgsM2mEndpointClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LoginToken => "login_token",
            Self::Permissions => "permissions",
            Self::DatasetDiscovery => "dataset_discovery",
            Self::DatasetFilters => "dataset_filters",
            Self::SceneSearch => "scene_search",
            Self::DownloadOptions => "download_options",
            Self::DownloadRequest => "download_request",
            Self::DownloadRetrieve => "download_retrieve",
            Self::BrowseFetch => "browse_fetch",
            Self::Logout => "logout",
            Self::RuntimeSession => "runtime_session",
            Self::DiscoveryRouter => "discovery_router",
            Self::SceneSearchRouter => "scene_search_router",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsgsM2mError {
    code: UsgsM2mErrorCode,
    endpoint: UsgsM2mEndpointClass,
    http_status: Option<u16>,
    retry_after_seconds: Option<u64>,
    detail: Option<&'static str>,
    remote_code: Option<String>,
    remote_message: Option<String>,
    remote_message_redacted: bool,
    transport_message: Option<String>,
    transport_message_redacted: bool,
    // Boxed so the error stays under Clippy's large-Err bound: every USGS
    // call returns it, and the shape is only set on a rejected answer.
    #[allow(clippy::box_collection)]
    response_shape: Option<Box<String>>,
}

impl UsgsM2mError {
    pub const fn new(code: UsgsM2mErrorCode, endpoint: UsgsM2mEndpointClass) -> Self {
        Self {
            code,
            endpoint,
            http_status: None,
            retry_after_seconds: None,
            detail: None,
            remote_code: None,
            remote_message: None,
            remote_message_redacted: false,
            transport_message: None,
            transport_message_redacted: false,
            response_shape: None,
        }
    }

    /// Attach the sanitized shape of the answer that failed to parse (key
    /// names, array lengths, typed error codes, URL host and path shape).
    pub(crate) fn with_response_shape(mut self, shape: String) -> Self {
        self.response_shape = Some(Box::new(shape));
        self
    }

    pub fn response_shape(&self) -> Option<&str> {
        self.response_shape.as_deref().map(String::as_str)
    }

    pub(crate) const fn with_http_status(mut self, status: u16) -> Self {
        self.http_status = Some(status);
        self
    }

    pub(crate) const fn with_retry_after(mut self, seconds: Option<u64>) -> Self {
        self.retry_after_seconds = seconds;
        self
    }

    pub(crate) const fn with_detail(mut self, detail: &'static str) -> Self {
        if self.detail.is_none() {
            self.detail = Some(detail);
        }
        self
    }

    pub(crate) fn with_remote_error(
        mut self,
        remote_code: String,
        remote_message: Option<String>,
        remote_message_redacted: bool,
    ) -> Self {
        self.remote_code = Some(remote_code);
        self.remote_message = remote_message;
        self.remote_message_redacted = remote_message_redacted;
        self
    }

    pub(crate) fn with_transport_error(
        mut self,
        transport_message: String,
        transport_message_redacted: bool,
    ) -> Self {
        if !transport_message.trim().is_empty() {
            self.transport_message = Some(transport_message);
        }
        self.transport_message_redacted = transport_message_redacted;
        self
    }

    pub const fn code(&self) -> UsgsM2mErrorCode {
        self.code
    }

    pub const fn code_str(&self) -> &'static str {
        self.code.as_str()
    }

    pub const fn endpoint(&self) -> UsgsM2mEndpointClass {
        self.endpoint
    }

    pub const fn http_status(&self) -> Option<u16> {
        self.http_status
    }

    pub const fn retry_after_seconds(&self) -> Option<u64> {
        self.retry_after_seconds
    }

    pub const fn detail(&self) -> Option<&'static str> {
        self.detail
    }

    pub fn remote_code(&self) -> Option<&str> {
        self.remote_code.as_deref()
    }

    pub fn remote_message(&self) -> Option<&str> {
        self.remote_message.as_deref()
    }

    pub const fn remote_message_redacted(&self) -> bool {
        self.remote_message_redacted
    }

    pub fn transport_message(&self) -> Option<&str> {
        self.transport_message.as_deref()
    }

    pub const fn transport_message_redacted(&self) -> bool {
        self.transport_message_redacted
    }

    pub fn diagnostic_message(&self) -> String {
        let mut parts = vec![format!(
            "USGS M2M endpoint={} code={}",
            self.endpoint.as_str(),
            self.code.as_str()
        )];
        if let Some(status) = self.http_status {
            parts.push(format!("http_status={status}"));
        }
        if let Some(remote_code) = self.remote_code.as_deref() {
            parts.push(format!("upstream_error_code={remote_code}"));
        }
        if let Some(message) = self.remote_message.as_deref() {
            parts.push(format!("upstream_error_message={message}"));
        }
        if self.remote_message_redacted {
            parts.push("secret_material_redacted=true".to_owned());
        }
        if let Some(message) = self.transport_message.as_deref() {
            parts.push(format!("transport_error={message}"));
        }
        if self.transport_message_redacted {
            parts.push("transport_secret_material_redacted=true".to_owned());
        }
        if let Some(detail) = self.detail {
            parts.push(format!("detail={detail}"));
        }
        if let Some(shape) = self.response_shape() {
            parts.push(format!("response_shape={shape}"));
        }
        parts.join(" ")
    }

    pub(crate) fn from_remote_code(endpoint: UsgsM2mEndpointClass, remote_code: &str) -> Self {
        let code = match remote_code {
            "AUTH_REQUIRED" | "AUTHENTICATION_REQUIRED" => UsgsM2mErrorCode::AuthRequired,
            "AUTH_INVALID" | "AUTH_KEY_INVALID" | "INVALID_TOKEN" => UsgsM2mErrorCode::AuthInvalid,
            "AUTH_UNAUTHORIZED" | "AUTH_NOT_AUTHORIZED" | "UNAUTHORIZED" => {
                UsgsM2mErrorCode::AuthUnauthorized
            }
            "AUTH_EXPIRED" | "AUTH_KEY_EXPIRED" | "TOKEN_EXPIRED" => UsgsM2mErrorCode::AuthExpired,
            "RATE_LIMITED" | "RATE_LIMIT_EXCEEDED" => UsgsM2mErrorCode::RateLimited,
            _ => UsgsM2mErrorCode::RemoteError,
        };
        Self::new(code, endpoint)
    }
}

impl fmt::Display for UsgsM2mError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic_message())
    }
}

impl Error for UsgsM2mError {}
