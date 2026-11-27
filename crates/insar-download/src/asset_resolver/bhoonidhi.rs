//! Exact, runtime-only resolver for a Bhoonidhi NISAR S-band RSLC product.
//!
//! Catalogue responses intentionally do not carry a product download URL,
//! size, or checksum.  The resolver therefore builds the exact `/download`
//! URL from the frozen provider identity and asks an injected network owner to
//! perform a bounded `Range: bytes=0-0` probe.  The probe supplies the positive
//! product size and the strong, opaque ETag used by the transfer owner.

use std::error::Error as StdError;
use std::fmt;
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use super::bhoonidhi_probe_rejection::{record_probe_rejection, rejection_facts};
use super::types::ResolvedHostPolicy;
use super::{
    AssetResolver, ResolveContext, ResolveError, ResolvedAsset, ResolvedMethod, ShadowAccessIntent,
    ShadowResolveContext,
};
use crate::BHOONIDHI_CREDENTIAL_PROFILE;
use insar_core::{
    provider_product_key, sanitize_external_error_message, stable_asset_key, AssetAccessDescriptor,
    AssetAccessScheme, AssetRole, FrozenAssetAccess, StableAssetReference,
};
use insar_provider::bhoonidhi::{
    BhoonidhiAccessTokenProvider, BhoonidhiAuthError as CatalogAuthError, BhoonidhiRefreshResult,
    BHOONIDHI_COLLECTION_ID, BHOONIDHI_PROVIDER_ID, BHOONIDHI_RESOLVER_POLICY_ID,
};
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, LAST_MODIFIED, RANGE};
use reqwest::redirect::Policy;
use reqwest::Url;

pub const BHOONIDHI_DOWNLOAD_RESOLVER_ID: &str = BHOONIDHI_RESOLVER_POLICY_ID;
pub const BHOONIDHI_DOWNLOAD_ORIGIN: &str = "https://bhoonidhi-api.nrsc.gov.in";
pub const BHOONIDHI_DOWNLOAD_PATH: &str = "/download";
pub const BHOONIDHI_DOWNLOAD_URL: &str = "https://bhoonidhi-api.nrsc.gov.in/download";
pub const BHOONIDHI_DOWNLOAD_ASSET_KEY: &str = "data";

/// The probe is deliberately one byte.  It establishes the total size and
/// validator before the existing transfer engine admits or resumes bytes.
pub const BHOONIDHI_DOWNLOAD_PROBE_RANGE: &str = "bytes=0-0";
pub const BHOONIDHI_DOWNLOAD_PROBE_MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const BHOONIDHI_DOWNLOAD_PROBE_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const BHOONIDHI_DOWNLOAD_PROBE_TOTAL_TIMEOUT: Duration = Duration::from_secs(45);
pub const BHOONIDHI_MAX_CONCURRENT_DOWNLOADS: usize = 3;
pub(crate) const BHOONIDHI_PROBE_FORBIDDEN_DIAGNOSTIC_ZH: &str =
    "Bhoonidhi 拒绝下载该影像（HTTP 403）：该景可能已被平台重新处理并更新编号，请重新检索最新结果后再试";

/// A bounded, redacted failure from the injected download-header probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BhoonidhiDownloadProbeError {
    Configuration,
    Transport {
        operation: &'static str,
        message: String,
        source_chain: Vec<String>,
        secret_material_redacted: bool,
    },
    Timeout {
        operation: &'static str,
        message: String,
        source_chain: Vec<String>,
        secret_material_redacted: bool,
    },
    ResponseRead {
        operation: &'static str,
        message: String,
        source_chain: Vec<String>,
        secret_material_redacted: bool,
    },
    Cancelled,
    ResponseTooLarge,
    UnexpectedUrl,
    Status(u16),
    ContentRange,
    ContentLength,
    BodyLength,
    EtagMissing,
    EtagWeak,
    EtagInvalid,
    LastModifiedInvalid,
}

impl BhoonidhiDownloadProbeError {
    fn from_external_error(
        timeout: bool,
        operation: &'static str,
        error: &(dyn StdError + 'static),
    ) -> Self {
        let (message, source_chain, secret_material_redacted) =
            sanitize_external_error_chain(error);
        if timeout {
            Self::Timeout {
                operation,
                message,
                source_chain,
                secret_material_redacted,
            }
        } else {
            Self::Transport {
                operation,
                message,
                source_chain,
                secret_material_redacted,
            }
        }
    }

    fn transport_message(operation: &'static str, message: &str) -> Self {
        let (message, secret_material_redacted) = sanitize_external_error_message(message);
        Self::Transport {
            operation,
            message,
            source_chain: Vec::new(),
            secret_material_redacted,
        }
    }

    fn response_read(error: &(dyn StdError + 'static)) -> Self {
        let (message, source_chain, secret_material_redacted) =
            sanitize_external_error_chain(error);
        Self::ResponseRead {
            operation: "reading Bhoonidhi download probe response",
            message,
            source_chain,
            secret_material_redacted,
        }
    }

    pub fn operation(&self) -> Option<&'static str> {
        match self {
            Self::Transport { operation, .. }
            | Self::Timeout { operation, .. }
            | Self::ResponseRead { operation, .. } => Some(*operation),
            _ => None,
        }
    }

    pub fn source_chain(&self) -> Option<&[String]> {
        match self {
            Self::Transport { source_chain, .. }
            | Self::Timeout { source_chain, .. }
            | Self::ResponseRead { source_chain, .. } => Some(source_chain),
            _ => None,
        }
    }
}

impl fmt::Display for BhoonidhiDownloadProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration => formatter.write_str("Bhoonidhi probe configuration was invalid"),
            Self::Transport {
                operation,
                message,
                source_chain,
                secret_material_redacted,
            }
            | Self::Timeout {
                operation,
                message,
                source_chain,
                secret_material_redacted,
            }
            | Self::ResponseRead {
                operation,
                message,
                source_chain,
                secret_material_redacted,
            } => write_probe_failure(
                formatter,
                message,
                source_chain,
                operation,
                *secret_material_redacted,
            ),
            Self::Cancelled => formatter.write_str("Bhoonidhi probe was cancelled"),
            Self::ResponseTooLarge => {
                formatter.write_str("Bhoonidhi probe response exceeded its byte bound")
            }
            Self::UnexpectedUrl => {
                formatter.write_str("Bhoonidhi probe response URL was not the exact download URL")
            }
            Self::Status(status) => {
                write!(formatter, "Bhoonidhi probe returned HTTP status {status}")
            }
            Self::ContentRange => formatter.write_str("Bhoonidhi probe Content-Range was invalid"),
            Self::ContentLength => {
                formatter.write_str("Bhoonidhi probe Content-Length was invalid")
            }
            Self::BodyLength => formatter.write_str("Bhoonidhi probe body length was invalid"),
            Self::EtagMissing => {
                formatter.write_str("Bhoonidhi probe did not return a strong ETag")
            }
            Self::EtagWeak => formatter.write_str("Bhoonidhi probe returned a weak ETag"),
            Self::EtagInvalid => formatter.write_str("Bhoonidhi probe ETag syntax was invalid"),
            Self::LastModifiedInvalid => {
                formatter.write_str("Bhoonidhi probe Last-Modified header was invalid")
            }
        }
    }
}

impl StdError for BhoonidhiDownloadProbeError {}

fn sanitize_external_error_chain(error: &(dyn StdError + 'static)) -> (String, Vec<String>, bool) {
    let (message, mut secret_material_redacted) =
        sanitize_external_error_message(&error.to_string());
    let mut source_chain = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        let (safe_cause, cause_redacted) = sanitize_external_error_message(&cause.to_string());
        secret_material_redacted |= cause_redacted;
        if !safe_cause.is_empty()
            && safe_cause != message
            && source_chain.last() != Some(&safe_cause)
        {
            source_chain.push(safe_cause);
        }
        source = cause.source();
    }
    (message, source_chain, secret_material_redacted)
}

fn write_probe_failure(
    formatter: &mut fmt::Formatter<'_>,
    message: &str,
    source_chain: &[String],
    operation: &str,
    secret_material_redacted: bool,
) -> fmt::Result {
    formatter.write_str(message)?;
    for cause in source_chain {
        write!(formatter, "; caused by: {cause}")?;
    }
    write!(formatter, " (while {operation})")?;
    if secret_material_redacted {
        formatter.write_str(" [secret material redacted]")?;
    }
    Ok(())
}

static BHOONIDHI_DOWNLOAD_SLOTS: OnceLock<Mutex<usize>> = OnceLock::new();

fn download_slot_count() -> &'static Mutex<usize> {
    BHOONIDHI_DOWNLOAD_SLOTS.get_or_init(|| Mutex::new(0))
}

/// Process-wide permit for one active Bhoonidhi download or header probe.
///
/// The provider limits one user/IP to three concurrent downloads. Callers
/// should hold this permit for the complete request, including response body
/// consumption. Acquisition fails fast with the provider's documented 412
/// concurrency status; it never waits or retries.
#[derive(Debug)]
pub struct BhoonidhiDownloadPermit;

impl Drop for BhoonidhiDownloadPermit {
    fn drop(&mut self) {
        if let Ok(mut active) = download_slot_count().lock() {
            *active = active.saturating_sub(1);
        }
    }
}

/// Claims one of the three process-wide Bhoonidhi download slots.
///
/// The returned permit is intentionally usable by the full-download owner as
/// well as [`ReqwestBhoonidhiDownloadProbe`], so a probe cannot bypass the
/// same local concurrency policy used for the subsequent transfer.
pub fn try_acquire_bhoonidhi_download_slot(
) -> Result<BhoonidhiDownloadPermit, BhoonidhiDownloadProbeError> {
    let mut active = download_slot_count().lock().map_err(|error| {
        BhoonidhiDownloadProbeError::transport_message(
            "acquiring Bhoonidhi download slot",
            &error.to_string(),
        )
    })?;
    if *active >= BHOONIDHI_MAX_CONCURRENT_DOWNLOADS {
        return Err(BhoonidhiDownloadProbeError::Status(412));
    }
    *active += 1;
    Ok(BhoonidhiDownloadPermit)
}

pub(crate) fn is_bhoonidhi_download_host(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("bhoonidhi-api.nrsc.gov.in"))
}

fn parse_u64_header(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

/// Parses Bhoonidhi APISIX rate-limit headers (`x-ratelimit-remaining`,
/// `x-ratelimit-reset`, and optional `retry-after`) into a bounded wait
/// duration (+1s safety margin, clamped to `121s`).
pub(crate) fn parse_rate_limit_wait_from_headers(
    status: u16,
    headers: &reqwest::header::HeaderMap,
) -> Option<Duration> {
    let retry_after = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(crate::transient_retry::parse_retry_after_header);
    let reset = parse_u64_header(headers, "x-ratelimit-reset")
        .map(|secs| Duration::from_secs(secs.min(120)));
    let remaining = parse_u64_header(headers, "x-ratelimit-remaining");

    if status == 429 {
        let base = retry_after
            .into_iter()
            .chain(reset)
            .max()
            .unwrap_or(Duration::from_secs(60));
        return Some(
            base.saturating_add(Duration::from_secs(1))
                .min(Duration::from_secs(121)),
        );
    }
    if remaining == Some(0) {
        let base = reset.unwrap_or(Duration::from_secs(60));
        return Some(
            base.saturating_add(Duration::from_secs(1))
                .min(Duration::from_secs(121)),
        );
    }
    None
}

static BHOONIDHI_RATE_LIMIT_UNTIL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

fn rate_limit_deadline() -> &'static Mutex<Option<Instant>> {
    BHOONIDHI_RATE_LIMIT_UNTIL.get_or_init(|| Mutex::new(None))
}

/// Waits outside any download slot or mutex if a prior Bhoonidhi response
/// reported an exhausted rate-limit window. Returns `true` when cancelled.
pub(crate) fn wait_for_bhoonidhi_rate_limit_window(
    url: &Url,
    cancelled: &dyn Fn() -> bool,
) -> bool {
    if !is_bhoonidhi_download_host(url) {
        return false;
    }
    loop {
        if cancelled() {
            return true;
        }
        let remaining = {
            let mut guard = rate_limit_deadline()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match *guard {
                Some(deadline) => {
                    let now = Instant::now();
                    if deadline > now {
                        Some(deadline.saturating_duration_since(now))
                    } else {
                        *guard = None;
                        None
                    }
                }
                None => None,
            }
        };
        match remaining {
            Some(delay) => {
                if crate::transient_retry::wait_unless_stopped(delay, cancelled) {
                    return true;
                }
            }
            None => return false,
        }
    }
}

/// Records an exhausted or 429 rate-limit window from a Bhoonidhi response so
/// subsequent probes/transfers pause proactively instead of burning quota.
pub(crate) fn record_bhoonidhi_rate_limit_headers(
    url: &Url,
    status: u16,
    headers: &reqwest::header::HeaderMap,
) {
    if !is_bhoonidhi_download_host(url) {
        return;
    }
    let Some(wait) = parse_rate_limit_wait_from_headers(status, headers) else {
        return;
    };
    let candidate = Instant::now() + wait;
    let mut guard = rate_limit_deadline()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = Some(guard.map_or(candidate, |prev| prev.max(candidate)));
}

/// Header evidence produced by one exact-range probe.
///
/// The ETag remains the server's opaque strong validator.  In particular, a
/// multipart suffix such as `-402` is retained as part of the ETag and is
/// never converted to an MD5 checksum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BhoonidhiDownloadProbeEvidence {
    expected_size_bytes: u64,
    etag: String,
    last_modified: Option<String>,
}

impl BhoonidhiDownloadProbeEvidence {
    fn new(
        expected_size_bytes: u64,
        etag: String,
        last_modified: Option<String>,
    ) -> Result<Self, BhoonidhiDownloadProbeError> {
        if expected_size_bytes == 0 || !valid_strong_etag(&etag) {
            return Err(BhoonidhiDownloadProbeError::EtagInvalid);
        }
        if last_modified
            .as_deref()
            .is_some_and(|value| !valid_last_modified(value))
        {
            return Err(BhoonidhiDownloadProbeError::LastModifiedInvalid);
        }
        Ok(Self {
            expected_size_bytes,
            etag,
            last_modified,
        })
    }

    pub fn expected_size_bytes(&self) -> u64 {
        self.expected_size_bytes
    }

    pub fn etag(&self) -> &str {
        &self.etag
    }

    pub fn last_modified(&self) -> Option<&str> {
        self.last_modified.as_deref()
    }

    pub fn remote_validator(&self) -> super::RemoteValidator {
        super::RemoteValidator {
            etag: Some(self.etag.clone()),
            last_modified: self.last_modified.clone(),
            content_length: Some(self.expected_size_bytes),
        }
    }
}

/// Network-owner seam for the one-byte probe.
///
/// Implementations send a `GET` with [`BHOONIDHI_DOWNLOAD_PROBE_RANGE`], use
/// the existing provider network policy and timeout owner, disable redirects,
/// bound all response bytes by [`BHOONIDHI_DOWNLOAD_PROBE_MAX_RESPONSE_BYTES`],
/// and pass the response through [`parse_download_probe_response`].  The
/// resolved asset carries the runtime Bearer header; the trait never receives
/// a token as a durable value or diagnostic string.
pub trait BhoonidhiDownloadProbe: Send + Sync {
    fn probe(
        &self,
        asset: &ResolvedAsset,
        context: &ResolveContext<'_>,
    ) -> Result<BhoonidhiDownloadProbeEvidence, BhoonidhiDownloadProbeError>;
}

/// Native blocking probe for the exact Bhoonidhi `/download` origin.
///
/// A fresh client is built for each probe so the context's route and timeout
/// bounds are applied to the request. The client uses rustls, no redirects,
/// no retries, no cookies, and the fixed 15-second connect/45-second total
/// ceilings. The one-byte response is consumed under the 64 KiB bound before
/// the strict header parser is called.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReqwestBhoonidhiDownloadProbe;

impl ReqwestBhoonidhiDownloadProbe {
    pub const fn new() -> Self {
        Self
    }
}

impl BhoonidhiDownloadProbe for ReqwestBhoonidhiDownloadProbe {
    fn probe(
        &self,
        asset: &ResolvedAsset,
        context: &ResolveContext<'_>,
    ) -> Result<BhoonidhiDownloadProbeEvidence, BhoonidhiDownloadProbeError> {
        context
            .check_cancelled()
            .map_err(|_| BhoonidhiDownloadProbeError::Cancelled)?;
        let expected_url = asset.effective_url();
        if asset.method() != ResolvedMethod::Get
            || !exact_runtime_download_url(expected_url)
            || !asset
                .host_policy()
                .is_some_and(|policy| policy.allows_authenticated_request_url(expected_url))
        {
            return Err(BhoonidhiDownloadProbeError::UnexpectedUrl);
        }

        if wait_for_bhoonidhi_rate_limit_window(expected_url, &|| {
            context.check_cancelled().is_err()
        }) {
            return Err(BhoonidhiDownloadProbeError::Cancelled);
        }
        let _permit = try_acquire_bhoonidhi_download_slot()?;
        context
            .check_cancelled()
            .map_err(|_| BhoonidhiDownloadProbeError::Cancelled)?;
        let client = build_probe_client(context)?;
        let request = asset
            .apply_request_access(client.get(expected_url.clone()))
            .header(RANGE, BHOONIDHI_DOWNLOAD_PROBE_RANGE)
            .header(ACCEPT_ENCODING, "identity");
        let sent = Instant::now();
        let response = request.send().map_err(classify_probe_transport_error)?;
        let headers_ms = elapsed_ms(sent);
        context
            .check_cancelled()
            .map_err(|_| BhoonidhiDownloadProbeError::Cancelled)?;

        let response_url = response.url().clone();
        let status = response.status().as_u16();
        record_bhoonidhi_rate_limit_headers(&response_url, status, response.headers());
        let rejected = matches!(status, 401 | 403);
        let rejection_headers = rejected.then(|| response.headers().clone());
        let content_range = header_string(response.headers().get(CONTENT_RANGE));
        let content_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| (value == "1").then_some(1));
        let etag = header_string(response.headers().get(ETAG));
        let last_modified = header_string(response.headers().get(LAST_MODIFIED));
        let (body_len, body_prefix) = read_bounded_probe_body(response, context, rejected)?;
        if let Some(headers) = rejection_headers {
            let total_ms = elapsed_ms(sent);
            if let Some(facts) = rejection_facts(
                status,
                &headers,
                &body_prefix,
                body_len,
                headers_ms,
                total_ms.saturating_sub(headers_ms),
                SystemTime::now(),
            ) {
                record_probe_rejection(facts);
            }
        }

        parse_download_probe_response(
            expected_url,
            &response_url,
            status,
            content_range.as_deref(),
            content_length,
            etag.as_deref(),
            last_modified.as_deref(),
            body_len,
        )
    }
}

fn build_probe_client(context: &ResolveContext<'_>) -> Result<Client, BhoonidhiDownloadProbeError> {
    let timeout_policy = context.timeout_policy();
    let connect_timeout = timeout_policy
        .connect
        .min(BHOONIDHI_DOWNLOAD_PROBE_CONNECT_TIMEOUT);
    let total_timeout = timeout_policy
        .operation
        .min(BHOONIDHI_DOWNLOAD_PROBE_TOTAL_TIMEOUT);
    if connect_timeout.is_zero() || total_timeout.is_zero() || connect_timeout > total_timeout {
        return Err(BhoonidhiDownloadProbeError::Configuration);
    }

    let mut builder = Client::builder()
        .user_agent("EOStage/2.2 Bhoonidhi S-RSLC download probe")
        .connect_timeout(connect_timeout)
        .timeout(total_timeout)
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .cookie_store(false)
        .referer(false)
        .http1_only();
    builder =
        insar_provider::network_proxy::blocking_builder_with_proxy(builder, context.proxy_policy());
    builder.build().map_err(|error| {
        BhoonidhiDownloadProbeError::from_external_error(
            error.is_timeout(),
            "building Bhoonidhi download probe client",
            &error,
        )
    })
}

fn header_string(value: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    value
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Reads the bounded probe body; with `keep_prefix` its first 4 KiB are kept
/// for the sanitized rejection facts.
fn read_bounded_probe_body(
    mut response: Response,
    context: &ResolveContext<'_>,
    keep_prefix: bool,
) -> Result<(usize, Vec<u8>), BhoonidhiDownloadProbeError> {
    const KEPT_PREFIX_BYTES: usize = 4096;
    let mut body_len = 0_usize;
    let mut prefix = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        context
            .check_cancelled()
            .map_err(|_| BhoonidhiDownloadProbeError::Cancelled)?;
        let count = response
            .read(&mut buffer)
            .map_err(classify_probe_response_read_error)?;
        if count == 0 {
            break;
        }
        body_len = body_len
            .checked_add(count)
            .ok_or(BhoonidhiDownloadProbeError::ResponseTooLarge)?;
        if body_len > BHOONIDHI_DOWNLOAD_PROBE_MAX_RESPONSE_BYTES {
            return Err(BhoonidhiDownloadProbeError::ResponseTooLarge);
        }
        if keep_prefix && prefix.len() < KEPT_PREFIX_BYTES {
            let room = KEPT_PREFIX_BYTES - prefix.len();
            prefix.extend_from_slice(&buffer[..count.min(room)]);
        }
    }
    Ok((body_len, prefix))
}

fn classify_probe_transport_error(error: reqwest::Error) -> BhoonidhiDownloadProbeError {
    BhoonidhiDownloadProbeError::from_external_error(
        error.is_timeout(),
        "sending Bhoonidhi download probe request",
        &error,
    )
}

fn classify_probe_response_read_error(error: std::io::Error) -> BhoonidhiDownloadProbeError {
    let mut timeout = false;
    let mut source: Option<&(dyn StdError + 'static)> = Some(&error);
    while let Some(cause) = source {
        timeout = cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout);
        if let Some(io_error) = cause.downcast_ref::<std::io::Error>() {
            timeout = timeout
                || io_error.kind() == std::io::ErrorKind::TimedOut
                || io_error
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<reqwest::Error>())
                    .is_some_and(reqwest::Error::is_timeout);
        }
        if timeout {
            break;
        }
        source = cause.source();
    }
    if timeout {
        BhoonidhiDownloadProbeError::from_external_error(
            true,
            "reading Bhoonidhi download probe response",
            &error,
        )
    } else {
        BhoonidhiDownloadProbeError::response_read(&error)
    }
}

/// Strictly parses the bounded probe response without reading or exposing a
/// response body. `body_len` is measured by the network owner while enforcing
/// the 64 KiB response bound.
#[allow(clippy::too_many_arguments)] // Explicit HTTP facts keep this parser transport-independent.
pub fn parse_download_probe_response(
    expected_url: &Url,
    response_url: &Url,
    status: u16,
    content_range: Option<&str>,
    content_length: Option<u64>,
    etag: Option<&str>,
    last_modified: Option<&str>,
    body_len: usize,
) -> Result<BhoonidhiDownloadProbeEvidence, BhoonidhiDownloadProbeError> {
    if body_len > BHOONIDHI_DOWNLOAD_PROBE_MAX_RESPONSE_BYTES {
        return Err(BhoonidhiDownloadProbeError::ResponseTooLarge);
    }
    if !exact_runtime_download_url(expected_url) || response_url != expected_url {
        return Err(BhoonidhiDownloadProbeError::UnexpectedUrl);
    }
    if status != 206 {
        return Err(BhoonidhiDownloadProbeError::Status(status));
    }
    if body_len != 1 {
        return Err(BhoonidhiDownloadProbeError::BodyLength);
    }
    let total_size = content_range
        .and_then(parse_probe_content_range)
        .ok_or(BhoonidhiDownloadProbeError::ContentRange)?;
    if content_length != Some(1) {
        return Err(BhoonidhiDownloadProbeError::ContentLength);
    }
    let etag = etag.ok_or(BhoonidhiDownloadProbeError::EtagMissing)?;
    if etag.trim_start().starts_with("W/") {
        return Err(BhoonidhiDownloadProbeError::EtagWeak);
    }
    if !valid_strong_etag(etag) {
        return Err(BhoonidhiDownloadProbeError::EtagInvalid);
    }
    let last_modified = last_modified.map(str::to_owned);
    BhoonidhiDownloadProbeEvidence::new(total_size, etag.to_owned(), last_modified)
}

/// Parses the only accepted probe range: `bytes 0-0/<positive-total>`.
pub fn parse_probe_content_range(value: &str) -> Option<u64> {
    let total = value.strip_prefix("bytes 0-0/")?;
    if total.is_empty()
        || (total.len() > 1 && total.starts_with('0'))
        || !total.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let total = total.parse::<u64>().ok()?;
    (total > 0).then_some(total)
}

fn valid_strong_etag(value: &str) -> bool {
    if value.len() < 3
        || value.len() > BHOONIDHI_DOWNLOAD_PROBE_MAX_RESPONSE_BYTES
        || value.trim() != value
        || !value.starts_with('"')
        || !value.ends_with('"')
    {
        return false;
    }
    let opaque = &value[1..value.len() - 1];
    !opaque.is_empty()
        && opaque
            .bytes()
            .all(|byte| byte == b'!' || (0x23..=0x7e).contains(&byte) || byte >= 0x80)
}

fn valid_last_modified(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

/// Validates the closed access descriptor used by the Bhoonidhi complete
/// download route.  The durable descriptor never contains item query values;
/// those are derived from the frozen identity at execution time.
pub fn validate_bhoonidhi_download_descriptor(
    descriptor: &AssetAccessDescriptor,
) -> Result<(), ResolveError> {
    if descriptor.resolver_id != BHOONIDHI_DOWNLOAD_RESOLVER_ID
        || descriptor.scheme != AssetAccessScheme::OauthBearerHttps
        || !descriptor.requires_credentials
        || descriptor.credential_profile.as_deref() != Some(BHOONIDHI_CREDENTIAL_PROFILE)
        || !descriptor
            .stable_href
            .as_deref()
            .is_some_and(exact_stable_href)
    {
        return Err(ResolveError::descriptor_invalid());
    }
    Ok(())
}

/// Backoff before automatic probe retry `retry` (1-based) after a transient
/// failure with the given HTTP status, if any.
pub(crate) type BhoonidhiProbeRetryBackoff = dyn Fn(u32, Option<u16>) -> Duration + Send + Sync;

/// Exact resolver for the authenticated `NISAR_SSAR_RSLC` complete product.
pub struct BhoonidhiAssetResolver {
    token_provider: Arc<dyn BhoonidhiAccessTokenProvider>,
    probe: Arc<dyn BhoonidhiDownloadProbe>,
    probe_retry_backoff: Arc<BhoonidhiProbeRetryBackoff>,
}

impl BhoonidhiAssetResolver {
    pub fn new(
        token_provider: Arc<dyn BhoonidhiAccessTokenProvider>,
        probe: Arc<dyn BhoonidhiDownloadProbe>,
    ) -> Self {
        Self {
            token_provider,
            probe,
            probe_retry_backoff: Arc::new(crate::transient_retry::transient_retry_backoff),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_probe_retry_backoff_for_test(
        mut self,
        backoff: Arc<BhoonidhiProbeRetryBackoff>,
    ) -> Self {
        self.probe_retry_backoff = backoff;
        self
    }

    /// The first download probe, retried like a science Range when it fails
    /// transiently (downloader enhancement D1d; L4 2026-10-18: one send
    /// timeout failed the item as DL005): at most three attempts, with the
    /// shared bounded backoff before each retry. A stop observed during the
    /// wait (cancel, or a pause where the caller's context includes it) ends
    /// the resolution as cancelled. Returns the attempts made with the error.
    fn probe_with_transient_retry(
        &self,
        resolved: &ResolvedAsset,
        context: &ResolveContext<'_>,
    ) -> Result<BhoonidhiDownloadProbeEvidence, (BhoonidhiDownloadProbeError, u32)> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.probe.probe(resolved, context) {
                Ok(evidence) => return Ok(evidence),
                Err(error)
                    if attempt < crate::transient_retry::TRANSIENT_MAX_ATTEMPTS
                        && probe_failure_is_transient(&error) =>
                {
                    let delay = (self.probe_retry_backoff)(attempt, probe_http_status(&error));
                    if crate::transient_retry::wait_unless_stopped(delay, &|| {
                        context.check_cancelled().is_err()
                    }) {
                        return Err((BhoonidhiDownloadProbeError::Cancelled, attempt));
                    }
                }
                Err(error) => return Err((error, attempt)),
            }
        }
    }
}

impl fmt::Debug for BhoonidhiAssetResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BhoonidhiAssetResolver")
            .field("resolver_id", &BHOONIDHI_DOWNLOAD_RESOLVER_ID)
            .field("token_provider", &"runtime-only")
            .field("probe", &"injected-network-owner")
            .finish()
    }
}

impl AssetResolver for BhoonidhiAssetResolver {
    fn resolver_id(&self) -> &'static str {
        BHOONIDHI_DOWNLOAD_RESOLVER_ID
    }

    fn supported_schemes(&self) -> &'static [AssetAccessScheme] {
        &[AssetAccessScheme::OauthBearerHttps]
    }

    fn can_resolve(&self, descriptor: &AssetAccessDescriptor) -> bool {
        validate_bhoonidhi_download_descriptor(descriptor).is_ok()
    }

    fn validate_frozen(&self, frozen: &FrozenAssetAccess) -> Result<(), ResolveError> {
        if !self.can_resolve(&frozen.descriptor)
            || frozen.role != AssetRole::Primary
            || !safe_optional_filename(frozen.file_name.as_deref())
            || frozen.expected_size_bytes.is_some_and(|size| size == 0)
            || !valid_frozen_checksum(
                frozen.checksum_algorithm.as_deref(),
                frozen.checksum_value.as_deref(),
            )
        {
            return Err(ResolveError::descriptor_invalid());
        }
        derive_identity(&frozen.reference).map(|_| ())
    }

    fn resolve(
        &self,
        reference: &StableAssetReference,
        descriptor: &AssetAccessDescriptor,
        context: &ResolveContext<'_>,
    ) -> Result<ResolvedAsset, ResolveError> {
        let _item_id = derive_identity(reference)?;
        if !self.can_resolve(descriptor) {
            return Err(ResolveError::descriptor_invalid());
        }
        if descriptor.stable_href.as_deref() != Some(BHOONIDHI_DOWNLOAD_URL) {
            return Err(ResolveError::access_not_durable());
        }
        context.check_cancelled()?;
        let token = self
            .token_provider
            .access_token_with_generation()
            .map_err(map_catalog_auth_error)?;
        if !valid_runtime_token(token.as_str()) {
            return Err(ResolveError::auth_failed());
        }
        context.check_cancelled()?;

        let download_url = derive_bhoonidhi_download_url(reference)?;
        let mut resolved = ResolvedAsset::new(download_url.as_str(), ResolvedMethod::Get)?
            .with_host_policy(ResolvedHostPolicy::BhoonidhiDownloadV1)
            .with_asset_role(AssetRole::Primary)
            .with_auth_generation(token.generation());
        super::CredentialMaterial::bearer(token.as_str().to_owned())
            .map_err(|_| ResolveError::auth_failed())?
            .apply_bearer_to(&mut resolved)?;
        context.check_cancelled()?;
        let evidence = match self.probe_with_transient_retry(&resolved, context) {
            Ok(evidence) => evidence,
            Err((BhoonidhiDownloadProbeError::Status(401), _)) => {
                let refresh = self
                    .token_provider
                    .refresh_after_rejection(token.generation())
                    .map_err(map_catalog_auth_error)?;
                if matches!(refresh, BhoonidhiRefreshResult::Unsupported) {
                    return Err(ResolveError::auth_failed());
                }
                context.check_cancelled()?;
                let replacement = self
                    .token_provider
                    .access_token_with_generation()
                    .map_err(map_catalog_auth_error)?;
                if !valid_runtime_token(replacement.as_str()) {
                    return Err(ResolveError::auth_failed());
                }
                super::CredentialMaterial::bearer(replacement.as_str().to_owned())
                    .map_err(|_| ResolveError::auth_failed())?
                    .apply_bearer_to(&mut resolved)?;
                resolved = resolved.with_auth_generation(replacement.generation());
                match self.probe.probe(&resolved, context) {
                    Ok(evidence) => evidence,
                    Err(BhoonidhiDownloadProbeError::Status(401)) => {
                        self.token_provider
                            .mark_generation_rejected(replacement.generation())
                            .map_err(map_catalog_auth_error)?;
                        return Err(ResolveError::auth_failed().with_diagnostic(
                            "BHOONIDHI_DOWNLOAD_PROBE_STATUS_401 stage=post_refresh_probe",
                        ));
                    }
                    Err(error) => {
                        // The refresh itself succeeded; a transient probe failure
                        // keeps a static staged diagnostic and stays retryable.
                        let staged = post_refresh_probe_diagnostic(&error);
                        let mapped = map_probe_error(error);
                        return Err(match staged {
                            Some(diagnostic) => mapped.with_diagnostic(diagnostic),
                            None => mapped,
                        });
                    }
                }
            }
            Err((error, attempts)) => return Err(map_probe_error_after_attempts(error, attempts)),
        };
        context.check_cancelled()?;
        Ok(resolved
            .with_expected_size(Some(evidence.expected_size_bytes()))
            .with_remote_validator(evidence.remote_validator()))
    }

    fn plan_shadow(
        &self,
        _frozen: &FrozenAssetAccess,
        _context: &ShadowResolveContext<'_>,
    ) -> Result<ShadowAccessIntent, ResolveError> {
        Err(ResolveError::not_configured())
    }
}

/// Derives the exact runtime URL from a validated frozen identity.
pub fn derive_bhoonidhi_download_url(
    reference: &StableAssetReference,
) -> Result<Url, ResolveError> {
    let item_id = derive_identity(reference)?;
    let mut url =
        Url::parse(BHOONIDHI_DOWNLOAD_URL).map_err(|_| ResolveError::descriptor_invalid())?;
    // Bhoonidhi's documented download contract uses the short query names
    // `id` and `collection`; the durable descriptor remains queryless and
    // these values are derived only from the validated frozen identity.
    url.query_pairs_mut()
        .append_pair("id", &item_id)
        .append_pair("collection", BHOONIDHI_COLLECTION_ID);
    Ok(url)
}

fn derive_identity(reference: &StableAssetReference) -> Result<String, ResolveError> {
    reference
        .validate()
        .map_err(|_| ResolveError::reference_invalid())?;
    if reference.provider_id != BHOONIDHI_PROVIDER_ID
        || reference.collection_id != BHOONIDHI_COLLECTION_ID
        || reference.asset_key != BHOONIDHI_DOWNLOAD_ASSET_KEY
        || !safe_item_id(&reference.provider_product_id)
    {
        return Err(ResolveError::reference_invalid());
    }
    let expected_product_key = provider_product_key(
        BHOONIDHI_PROVIDER_ID,
        BHOONIDHI_COLLECTION_ID,
        &reference.provider_product_id,
    )
    .map_err(|_| ResolveError::reference_invalid())?;
    let expected_asset_key = stable_asset_key(&expected_product_key, BHOONIDHI_DOWNLOAD_ASSET_KEY)
        .map_err(|_| ResolveError::reference_invalid())?;
    if reference.provider_product_key != expected_product_key
        || reference.stable_asset_key != expected_asset_key
        || reference
            .canonical_product_key
            .as_deref()
            .is_some_and(|value| value != expected_product_key.as_str())
    {
        return Err(ResolveError::reference_invalid());
    }
    Ok(reference.provider_product_id.clone())
}

fn safe_item_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.trim() == value
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn exact_stable_href(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    exact_download_origin_url(&url) && value == BHOONIDHI_DOWNLOAD_URL
}

fn exact_download_origin_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("bhoonidhi-api.nrsc.gov.in")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == BHOONIDHI_DOWNLOAD_PATH
        && url.query().is_none()
        && url.fragment().is_none()
}

fn exact_runtime_download_url(url: &Url) -> bool {
    if url.scheme() != "https"
        || url.host_str() != Some("bhoonidhi-api.nrsc.gov.in")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != BHOONIDHI_DOWNLOAD_PATH
        || url.fragment().is_some()
    {
        return false;
    }
    let pairs = url.query_pairs().collect::<Vec<_>>();
    pairs.len() == 2
        && pairs[0].0 == "id"
        && safe_item_id(&pairs[0].1)
        && pairs[1].0 == "collection"
        && pairs[1].1 == BHOONIDHI_COLLECTION_ID
}

fn safe_optional_filename(value: Option<&str>) -> bool {
    value.is_none_or(|value| {
        !value.is_empty()
            && value.len() <= 512
            && value.trim() == value
            && !value.chars().any(char::is_control)
            && !value.contains(['/', '\\', ':'])
            && value != "."
            && value != ".."
    })
}

fn valid_frozen_checksum(algorithm: Option<&str>, value: Option<&str>) -> bool {
    match (algorithm, value) {
        (None, None) => true,
        (Some("SHA256"), Some(value)) => {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }
        // The multipart ETag is an opaque validator.  It must never be
        // admitted as a provider MD5 checksum, including values ending in
        // `-402`.
        _ => false,
    }
}

fn valid_runtime_token(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= 16 * 1024
        && !value.chars().any(char::is_control)
}

fn map_catalog_auth_error(error: CatalogAuthError) -> ResolveError {
    let mapped = match error {
        CatalogAuthError::Missing => ResolveError::credentials_missing(),
        CatalogAuthError::Timeout => ResolveError::timeout(),
        CatalogAuthError::NotActivated
        | CatalogAuthError::Unavailable
        | CatalogAuthError::InvalidToken
        | CatalogAuthError::CredentialInvalid
        | CatalogAuthError::Rejected
        | CatalogAuthError::MaxSessions
        | CatalogAuthError::RateLimited
        | CatalogAuthError::Transport
        | CatalogAuthError::ResponseInvalid
        | CatalogAuthError::RefreshUnavailable
        | CatalogAuthError::RefreshExhausted
        | CatalogAuthError::SessionChanged
        | CatalogAuthError::SessionUnavailable
        | CatalogAuthError::Synchronization
        | CatalogAuthError::Configuration => ResolveError::auth_failed(),
    };
    mapped.with_diagnostic(error.code())
}

fn map_probe_error(error: BhoonidhiDownloadProbeError) -> ResolveError {
    let diagnostic = match &error {
        BhoonidhiDownloadProbeError::Configuration => {
            "BHOONIDHI_DOWNLOAD_PROBE_CONFIGURATION".to_owned()
        }
        BhoonidhiDownloadProbeError::Status(403) => {
            BHOONIDHI_PROBE_FORBIDDEN_DIAGNOSTIC_ZH.to_owned()
        }
        BhoonidhiDownloadProbeError::Transport { .. }
        | BhoonidhiDownloadProbeError::Timeout { .. }
        | BhoonidhiDownloadProbeError::ResponseRead { .. }
        | BhoonidhiDownloadProbeError::Status(_) => error.to_string(),
        BhoonidhiDownloadProbeError::Cancelled => "BHOONIDHI_DOWNLOAD_PROBE_CANCELLED".to_owned(),
        BhoonidhiDownloadProbeError::ResponseTooLarge => {
            "BHOONIDHI_DOWNLOAD_PROBE_RESPONSE_TOO_LARGE".to_owned()
        }
        BhoonidhiDownloadProbeError::UnexpectedUrl => {
            "BHOONIDHI_DOWNLOAD_PROBE_UNEXPECTED_URL".to_owned()
        }
        BhoonidhiDownloadProbeError::ContentRange => {
            "BHOONIDHI_DOWNLOAD_PROBE_CONTENT_RANGE_INVALID".to_owned()
        }
        BhoonidhiDownloadProbeError::ContentLength => {
            "BHOONIDHI_DOWNLOAD_PROBE_CONTENT_LENGTH_INVALID".to_owned()
        }
        BhoonidhiDownloadProbeError::BodyLength => {
            "BHOONIDHI_DOWNLOAD_PROBE_BODY_LENGTH_INVALID".to_owned()
        }
        BhoonidhiDownloadProbeError::EtagMissing => {
            "BHOONIDHI_DOWNLOAD_PROBE_ETAG_MISSING".to_owned()
        }
        BhoonidhiDownloadProbeError::EtagWeak => "BHOONIDHI_DOWNLOAD_PROBE_ETAG_WEAK".to_owned(),
        BhoonidhiDownloadProbeError::EtagInvalid => {
            "BHOONIDHI_DOWNLOAD_PROBE_ETAG_INVALID".to_owned()
        }
        BhoonidhiDownloadProbeError::LastModifiedInvalid => {
            "BHOONIDHI_DOWNLOAD_PROBE_LAST_MODIFIED_INVALID".to_owned()
        }
    };
    let mapped = match &error {
        BhoonidhiDownloadProbeError::Status(401) => ResolveError::auth_failed(),
        BhoonidhiDownloadProbeError::Status(403) => ResolveError::failed(),
        BhoonidhiDownloadProbeError::Status(408 | 504)
        | BhoonidhiDownloadProbeError::Timeout { .. } => ResolveError::timeout(),
        // Design 2026-10-17 §2: a temporarily unavailable endpoint or a lost
        // connection/body is transient, never an asset identity conflict.
        BhoonidhiDownloadProbeError::Status(412 | 429 | 500..=503 | 505..=599)
        | BhoonidhiDownloadProbeError::Transport { .. }
        | BhoonidhiDownloadProbeError::ResponseRead { .. } => ResolveError::unavailable(),
        BhoonidhiDownloadProbeError::Cancelled => ResolveError::cancelled(),
        _ => ResolveError::failed(),
    };
    mapped.with_diagnostic(diagnostic)
}

/// A failure a later attempt may not repeat: no usable response (a timeout, a
/// lost connection or body) or a temporarily unavailable endpoint (408, 412,
/// 429, 5xx), as classified by `map_probe_error`.
fn probe_failure_is_transient(error: &BhoonidhiDownloadProbeError) -> bool {
    matches!(
        error,
        BhoonidhiDownloadProbeError::Timeout { .. }
            | BhoonidhiDownloadProbeError::Transport { .. }
            | BhoonidhiDownloadProbeError::ResponseRead { .. }
            | BhoonidhiDownloadProbeError::Status(408 | 412 | 429 | 500..=599)
    )
}

fn probe_http_status(error: &BhoonidhiDownloadProbeError) -> Option<u16> {
    match error {
        BhoonidhiDownloadProbeError::Status(status) => Some(*status),
        _ => None,
    }
}

/// A transient failure that ended the automatic retries names its attempts in
/// the diagnostic tail, like a failed science Range (`attempts=N`). Other
/// failures keep their diagnostics unchanged.
fn map_probe_error_after_attempts(
    error: BhoonidhiDownloadProbeError,
    attempts: u32,
) -> ResolveError {
    let transient = probe_failure_is_transient(&error);
    // L5 recheck 2026-10-19: a certificate that cannot be verified is named as
    // such, not as a lost connection.
    let tls_certificate = error
        .source_chain()
        .is_some_and(crate::http_transfer::error_chain_names_tls_certificate);
    // Downloader D1d: no usable response (a timeout, a lost connection or body)
    // is a network cause the task card names instead of a login check.
    let network = !tls_certificate
        && matches!(
            error,
            BhoonidhiDownloadProbeError::Timeout { .. }
                | BhoonidhiDownloadProbeError::Transport { .. }
                | BhoonidhiDownloadProbeError::ResponseRead { .. }
        );
    let mapped = map_probe_error(error);
    if !transient {
        return mapped;
    }
    let diagnostic = mapped
        .diagnostic()
        .unwrap_or("BHOONIDHI_DOWNLOAD_PROBE_FAILED")
        .to_owned();
    if tls_certificate {
        return mapped.with_diagnostic(format!(
            "{}，断点已保留。{diagnostic} [stage=download_probe transport_cause=tls_certificate attempts={attempts} source=tls_certificate]",
            crate::http_transfer::TLS_CERTIFICATE_FAILURE_ZH
        ));
    }
    mapped.with_diagnostic(format!(
        "{diagnostic} [stage=download_probe attempts={attempts}{}]",
        if network { " source=network" } else { "" }
    ))
}

/// Static, allowlisted diagnostics for a transient download probe failure
/// right after a successful token refresh. Never provider response text.
pub(crate) const BHOONIDHI_POST_REFRESH_PROBE_TRANSIENT_DIAGNOSTICS: [&str; 10] = [
    "BHOONIDHI_DOWNLOAD_PROBE_STATUS_408 stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_STATUS_412 stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_STATUS_429 stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_STATUS_503 stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_STATUS_504 stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_STATUS_5XX stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_TRANSPORT stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_TIMEOUT stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_RESPONSE_READ stage=post_refresh_probe",
    "BHOONIDHI_DOWNLOAD_PROBE_STATUS_401 stage=post_refresh_probe",
];

fn post_refresh_probe_diagnostic(error: &BhoonidhiDownloadProbeError) -> Option<&'static str> {
    let index = match error {
        BhoonidhiDownloadProbeError::Status(408) => 0,
        BhoonidhiDownloadProbeError::Status(412) => 1,
        BhoonidhiDownloadProbeError::Status(429) => 2,
        BhoonidhiDownloadProbeError::Status(503) => 3,
        BhoonidhiDownloadProbeError::Status(504) => 4,
        BhoonidhiDownloadProbeError::Status(500..=599) => 5,
        BhoonidhiDownloadProbeError::Transport { .. } => 6,
        BhoonidhiDownloadProbeError::Timeout { .. } => 7,
        BhoonidhiDownloadProbeError::ResponseRead { .. } => 8,
        _ => return None,
    };
    Some(BHOONIDHI_POST_REFRESH_PROBE_TRANSIENT_DIAGNOSTICS[index])
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use insar_core::{
        provider_product_key, stable_asset_key, AssetAccessDescriptor, AssetAccessScheme,
        AssetResolverParameters, AssetRole, StableAssetReference,
    };
    use zeroize::Zeroizing;

    use super::*;
    use crate::asset_resolver::{
        CredentialMaterial, CredentialProvider, CredentialProviderError, SystemClock,
    };

    #[test]
    fn catalog_auth_mapping_preserves_safe_provider_error_code() {
        let transport = map_catalog_auth_error(CatalogAuthError::Transport);
        assert_eq!(transport.kind(), crate::ResolveErrorKind::AuthFailed);
        assert_eq!(
            transport.diagnostic(),
            Some("BHOONIDHI_AUTH_TRANSPORT_FAILED")
        );

        let timeout = map_catalog_auth_error(CatalogAuthError::Timeout);
        assert_eq!(timeout.kind(), crate::ResolveErrorKind::Timeout);
        assert_eq!(timeout.diagnostic(), Some("BHOONIDHI_AUTH_TIMEOUT"));
    }

    #[test]
    fn download_probe_http_status_mapping_keeps_actual_status_and_classification() {
        // Design 2026-10-17 §2: 5xx/429/412 are transient (Unavailable), no
        // longer the generic Failed kind that refresh turned into Integrity.
        for code in [412, 429, 500, 502, 503] {
            let status = map_probe_error(BhoonidhiDownloadProbeError::Status(code));
            assert_eq!(
                status.kind(),
                crate::ResolveErrorKind::Unavailable,
                "{code}"
            );
            let expected = format!("Bhoonidhi probe returned HTTP status {code}");
            assert_eq!(status.diagnostic(), Some(expected.as_str()));
        }
        let not_found = map_probe_error(BhoonidhiDownloadProbeError::Status(404));
        assert_eq!(not_found.kind(), crate::ResolveErrorKind::Failed);

        let forbidden = map_probe_error(BhoonidhiDownloadProbeError::Status(403));
        assert_eq!(forbidden.kind(), crate::ResolveErrorKind::Failed);
        assert_eq!(
            forbidden.diagnostic(),
            Some(BHOONIDHI_PROBE_FORBIDDEN_DIAGNOSTIC_ZH)
        );
        assert!(!probe_failure_is_transient(
            &BhoonidhiDownloadProbeError::Status(403)
        ));

        let unauthorized = map_probe_error(BhoonidhiDownloadProbeError::Status(401));
        assert_eq!(unauthorized.kind(), crate::ResolveErrorKind::AuthFailed);
        assert_eq!(
            unauthorized.diagnostic(),
            Some("Bhoonidhi probe returned HTTP status 401")
        );

        for status in [408, 504] {
            let mapped = map_probe_error(BhoonidhiDownloadProbeError::Status(status));
            assert_eq!(mapped.kind(), crate::ResolveErrorKind::Timeout);
            let expected = format!("Bhoonidhi probe returned HTTP status {status}");
            assert_eq!(mapped.diagnostic(), Some(expected.as_str()));
            assert!(!mapped.diagnostic().unwrap().contains("probe timed out"));
        }
    }

    #[test]
    fn post_refresh_probe_transient_failures_keep_a_static_staged_diagnostic() {
        for (error, expected) in [
            (
                BhoonidhiDownloadProbeError::Status(503),
                "BHOONIDHI_DOWNLOAD_PROBE_STATUS_503 stage=post_refresh_probe",
            ),
            (
                BhoonidhiDownloadProbeError::Status(412),
                "BHOONIDHI_DOWNLOAD_PROBE_STATUS_412 stage=post_refresh_probe",
            ),
            (
                BhoonidhiDownloadProbeError::Status(507),
                "BHOONIDHI_DOWNLOAD_PROBE_STATUS_5XX stage=post_refresh_probe",
            ),
        ] {
            assert_eq!(post_refresh_probe_diagnostic(&error), Some(expected));
            assert!(BHOONIDHI_POST_REFRESH_PROBE_TRANSIENT_DIAGNOSTICS.contains(&expected));
        }
        for error in [
            BhoonidhiDownloadProbeError::Status(404),
            BhoonidhiDownloadProbeError::EtagMissing,
            BhoonidhiDownloadProbeError::Cancelled,
        ] {
            assert_eq!(post_refresh_probe_diagnostic(&error), None, "{error:?}");
        }
    }

    #[test]
    fn download_probe_connection_failure_preserves_sanitized_reqwest_classification() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let url = format!(
            "http://fixture-user:fixture-password@{address}/download?id=fixture&token=fixture-secret"
        );
        let error = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_millis(250))
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap()
            .get(url)
            .bearer_auth("fixture-bearer-secret")
            .send()
            .unwrap_err();

        let upstream_timed_out = error.is_timeout();
        let probe_error = classify_probe_transport_error(error);
        assert_eq!(
            matches!(&probe_error, BhoonidhiDownloadProbeError::Timeout { .. }),
            upstream_timed_out,
            "the typed probe classification must preserve reqwest's actual result",
        );
        assert_eq!(
            probe_error.operation(),
            Some("sending Bhoonidhi download probe request")
        );
        assert!(!probe_error.source_chain().unwrap().is_empty());
        let debug = format!("{probe_error:?}");
        assert!(!debug.contains("fixture-password"));
        assert!(!debug.contains("fixture-secret"));
        assert!(!debug.contains("fixture-bearer-secret"));
        let mapped = map_probe_error(probe_error);
        // A refused connection (Linux answers the closed port with RST) is a
        // transient transport failure, Unavailable per map_probe_error; only
        // Windows, where the same connect usually times out, reached the
        // timeout arm before.
        assert_eq!(
            mapped.kind(),
            if upstream_timed_out {
                crate::ResolveErrorKind::Timeout
            } else {
                crate::ResolveErrorKind::Unavailable
            }
        );
        let diagnostic = mapped.diagnostic().unwrap();
        assert!(diagnostic.contains("error sending request"));
        assert!(diagnostic.contains("caused by:"));
        assert!(diagnostic.contains("?REDACTED"));
        assert!(diagnostic.contains("secret material redacted"));
        assert!(!diagnostic.contains("fixture-password"));
        assert!(!diagnostic.contains("fixture-secret"));
        assert!(!diagnostic.contains("fixture-bearer-secret"));
        assert!(!diagnostic.contains("BHOONIDHI_DOWNLOAD_PROBE_TRANSPORT_FAILED"));
    }

    #[test]
    fn download_probe_send_timeout_preserves_sanitized_reqwest_chain() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(250));
        });
        let error = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_millis(100))
            .timeout(Duration::from_millis(75))
            .build()
            .unwrap()
            .get(format!("http://{address}/download?id=fixture"))
            .send()
            .unwrap_err();
        server.join().unwrap();

        let probe_error = classify_probe_transport_error(error);
        let BhoonidhiDownloadProbeError::Timeout { .. } = &probe_error else {
            panic!("expected a request timeout, got {probe_error:?}");
        };
        assert_eq!(
            probe_error.operation(),
            Some("sending Bhoonidhi download probe request")
        );
        assert!(!probe_error.source_chain().unwrap().is_empty());
        let mapped = map_probe_error(probe_error);
        assert_eq!(mapped.kind(), crate::ResolveErrorKind::Timeout);
        let diagnostic = mapped.diagnostic().unwrap();
        assert!(diagnostic.to_ascii_lowercase().contains("timed out"));
        assert!(!diagnostic.contains("BHOONIDHI_DOWNLOAD_PROBE_TIMEOUT"));
    }

    #[test]
    fn download_probe_body_timeout_preserves_sanitized_io_chain() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut stream = BufReader::new(stream);
            loop {
                let mut line = String::new();
                stream.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let stream = stream.get_mut();
            stream
                .write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 2\r\nConnection: close\r\n\r\nx")
                .unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(250));
        });
        let response = Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap()
            .get(format!("http://{address}/download?id=fixture"))
            .send()
            .unwrap();
        let probe_error =
            read_bounded_probe_body(response, &context(&NoCredentials), false).unwrap_err();
        server.join().unwrap();

        let BhoonidhiDownloadProbeError::Timeout { .. } = &probe_error else {
            panic!("expected a response-body timeout, got {probe_error:?}");
        };
        assert_eq!(
            probe_error.operation(),
            Some("reading Bhoonidhi download probe response")
        );
        assert!(!probe_error.source_chain().unwrap().is_empty());
        let mapped = map_probe_error(probe_error);
        assert_eq!(mapped.kind(), crate::ResolveErrorKind::Timeout);
        let diagnostic = mapped.diagnostic().unwrap();
        assert!(diagnostic.to_ascii_lowercase().contains("timed out"));
        assert!(!diagnostic.contains("BHOONIDHI_DOWNLOAD_PROBE_TIMEOUT"));
    }

    #[test]
    fn download_probe_truncated_body_preserves_sanitized_io_chain() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut stream = BufReader::new(stream);
            loop {
                let mut line = String::new();
                stream.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let stream = stream.get_mut();
            stream
                .write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 2\r\nConnection: close\r\n\r\nx")
                .unwrap();
            stream.flush().unwrap();
        });
        let response = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap()
            .get(format!("http://{address}/download?id=fixture"))
            .send()
            .unwrap();
        let probe_error =
            read_bounded_probe_body(response, &context(&NoCredentials), false).unwrap_err();
        server.join().unwrap();

        let BhoonidhiDownloadProbeError::ResponseRead { .. } = &probe_error else {
            panic!("expected a response-body read failure, got {probe_error:?}");
        };
        assert_eq!(
            probe_error.operation(),
            Some("reading Bhoonidhi download probe response")
        );
        assert!(!probe_error.source_chain().unwrap().is_empty());
        let mapped = map_probe_error(probe_error);
        // Design 2026-10-17 §2: an interrupted body is transient (Unavailable),
        // no longer the generic Failed kind; the sanitized chain is unchanged.
        assert_eq!(mapped.kind(), crate::ResolveErrorKind::Unavailable);
        let diagnostic = mapped.diagnostic().unwrap();
        assert!(diagnostic.contains("caused by:"));
        assert!(!diagnostic.contains("BHOONIDHI_DOWNLOAD_PROBE_RESPONSE_READ_FAILED"));
    }

    const ITEM_ID: &str = "NISAR_S1_PR_RSLC_029_063_D_079_2500_CRNA_A_20260829T135915_20260829T135945_P00500_N_P_I_001";
    const FIXTURE_TOKEN: &str = "fixture-bhoonidhi-access";

    struct TokenProvider;

    impl BhoonidhiAccessTokenProvider for TokenProvider {
        fn access_token(&self) -> Result<Zeroizing<String>, CatalogAuthError> {
            Ok(Zeroizing::new(FIXTURE_TOKEN.to_owned()))
        }
    }

    struct Probe {
        calls: AtomicUsize,
    }

    impl BhoonidhiDownloadProbe for Probe {
        fn probe(
            &self,
            asset: &ResolvedAsset,
            _context: &ResolveContext<'_>,
        ) -> Result<BhoonidhiDownloadProbeEvidence, BhoonidhiDownloadProbeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            parse_download_probe_response(
                &derive_probe_url(asset.effective_url()),
                asset.effective_url(),
                206,
                Some("bytes 0-0/21054588114"),
                Some(1),
                Some("\"308dbc892d0ce417013aa66f28a61cb0-402\""),
                Some("2026/8/30 10:30:01 +00:00"),
                1,
            )
        }
    }

    fn derive_probe_url(url: &Url) -> Url {
        url.clone()
    }

    struct NoCredentials;

    impl CredentialProvider for NoCredentials {
        fn resolve(&self, _profile: &str) -> Result<CredentialMaterial, CredentialProviderError> {
            Err(CredentialProviderError::Missing)
        }
    }

    fn reference() -> StableAssetReference {
        let product_key =
            provider_product_key(BHOONIDHI_PROVIDER_ID, BHOONIDHI_COLLECTION_ID, ITEM_ID).unwrap();
        StableAssetReference {
            provider_id: BHOONIDHI_PROVIDER_ID.to_owned(),
            collection_id: BHOONIDHI_COLLECTION_ID.to_owned(),
            provider_product_id: ITEM_ID.to_owned(),
            provider_product_key: product_key.clone(),
            canonical_product_key: Some(product_key.clone()),
            asset_key: BHOONIDHI_DOWNLOAD_ASSET_KEY.to_owned(),
            stable_asset_key: stable_asset_key(&product_key, BHOONIDHI_DOWNLOAD_ASSET_KEY).unwrap(),
        }
    }

    fn descriptor() -> AssetAccessDescriptor {
        AssetAccessDescriptor {
            resolver_id: BHOONIDHI_DOWNLOAD_RESOLVER_ID.to_owned(),
            scheme: AssetAccessScheme::OauthBearerHttps,
            stable_href: Some(BHOONIDHI_DOWNLOAD_URL.to_owned()),
            requires_credentials: true,
            credential_profile: Some(BHOONIDHI_CREDENTIAL_PROFILE.to_owned()),
            resolver_parameters: AssetResolverParameters {},
        }
    }

    fn never_cancelled() -> bool {
        false
    }

    fn context<'a>(credentials: &'a dyn CredentialProvider) -> ResolveContext<'a> {
        static CLOCK: SystemClock = SystemClock;
        ResolveContext::new(credentials, &never_cancelled, &CLOCK)
    }

    /// Downloader enhancement D1d: scripted probe failures, then the valid
    /// fixture response.
    struct ScriptedProbe {
        failures: Mutex<std::collections::VecDeque<BhoonidhiDownloadProbeError>>,
        calls: AtomicUsize,
    }

    impl ScriptedProbe {
        fn new(failures: Vec<BhoonidhiDownloadProbeError>) -> Self {
            Self {
                failures: Mutex::new(failures.into()),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl BhoonidhiDownloadProbe for ScriptedProbe {
        fn probe(
            &self,
            asset: &ResolvedAsset,
            context: &ResolveContext<'_>,
        ) -> Result<BhoonidhiDownloadProbeEvidence, BhoonidhiDownloadProbeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.failures.lock().unwrap().pop_front() {
                return Err(error);
            }
            Probe {
                calls: AtomicUsize::new(0),
            }
            .probe(asset, context)
        }
    }

    fn send_timeout() -> BhoonidhiDownloadProbeError {
        BhoonidhiDownloadProbeError::Timeout {
            operation: "sending Bhoonidhi download probe request",
            message: "error sending request for url (https://bhoonidhi-api.nrsc.gov.in/download)"
                .to_owned(),
            source_chain: vec!["operation timed out".to_owned()],
            secret_material_redacted: false,
        }
    }

    type RecordedBackoff = Arc<Mutex<Vec<(u32, Option<u16>)>>>;

    fn recording_backoff(delay: Duration) -> (RecordedBackoff, Arc<BhoonidhiProbeRetryBackoff>) {
        let recorded = RecordedBackoff::default();
        let sink = Arc::clone(&recorded);
        let backoff: Arc<BhoonidhiProbeRetryBackoff> = Arc::new(move |retry, status| {
            sink.lock().unwrap().push((retry, status));
            delay
        });
        (recorded, backoff)
    }

    #[test]
    fn transient_download_probe_failures_are_retried_after_the_shared_backoff() {
        let probe = Arc::new(ScriptedProbe::new(vec![
            send_timeout(),
            BhoonidhiDownloadProbeError::Status(503),
        ]));
        let (recorded, backoff) = recording_backoff(Duration::ZERO);
        let resolver = BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone())
            .with_probe_retry_backoff_for_test(backoff);
        let credentials = NoCredentials;

        let resolved = resolver
            .resolve(&reference(), &descriptor(), &context(&credentials))
            .unwrap();
        assert_eq!(resolved.expected_size(), Some(21_054_588_114));
        assert_eq!(probe.calls.load(Ordering::SeqCst), 3);
        assert_eq!(*recorded.lock().unwrap(), vec![(1, None), (2, Some(503))]);
    }

    #[test]
    fn exhausted_probe_retries_name_their_attempts_and_stay_transient() {
        let credentials = NoCredentials;
        // Downloader D1d: a failure without a usable response (here a timeout)
        // declares its network cause (source=network); an HTTP status does not.
        for (failure, kind, status, tail) in [
            (
                send_timeout(),
                super::super::ResolveErrorKind::Timeout,
                None,
                " [stage=download_probe attempts=3 source=network]",
            ),
            (
                BhoonidhiDownloadProbeError::Status(429),
                super::super::ResolveErrorKind::Unavailable,
                Some(429),
                " [stage=download_probe attempts=3]",
            ),
        ] {
            let probe = Arc::new(ScriptedProbe::new(vec![failure.clone(); 3]));
            let (recorded, backoff) = recording_backoff(Duration::ZERO);
            let resolver = BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone())
                .with_probe_retry_backoff_for_test(backoff);

            let error = resolver
                .resolve(&reference(), &descriptor(), &context(&credentials))
                .unwrap_err();
            assert_eq!(error.kind(), kind, "{failure:?}");
            let diagnostic = error.diagnostic().unwrap();
            assert!(
                diagnostic.starts_with(&failure.to_string()) && diagnostic.ends_with(tail),
                "{diagnostic}"
            );
            assert_eq!(probe.calls.load(Ordering::SeqCst), 3);
            assert_eq!(*recorded.lock().unwrap(), vec![(1, status), (2, status)]);
        }
    }

    /// L5 recheck 2026-10-19: an unverifiable server certificate is named as
    /// such, not as a lost connection.
    #[test]
    fn a_probe_tls_certificate_failure_names_its_cause() {
        let certificate = BhoonidhiDownloadProbeError::Transport {
            operation: "sending Bhoonidhi download probe request",
            message: "error sending request for url (https://bhoonidhi-api.nrsc.gov.in/download)"
                .to_owned(),
            source_chain: vec![
                "client error (Connect)".to_owned(),
                "invalid peer certificate: UnknownIssuer".to_owned(),
            ],
            secret_material_redacted: false,
        };
        let probe = Arc::new(ScriptedProbe::new(vec![certificate; 3]));
        let (_, backoff) = recording_backoff(Duration::ZERO);
        let resolver = BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone())
            .with_probe_retry_backoff_for_test(backoff);
        let credentials = NoCredentials;

        let error = resolver
            .resolve(&reference(), &descriptor(), &context(&credentials))
            .unwrap_err();
        let diagnostic = error.diagnostic().unwrap();
        assert!(
            diagnostic
                .starts_with("无法验证服务器证书（本机证书库无法读取或网络被拦截），断点已保留。")
                && diagnostic.contains("invalid peer certificate: UnknownIssuer")
                && diagnostic.ends_with(
                    " [stage=download_probe transport_cause=tls_certificate attempts=3 source=tls_certificate]"
                ),
            "{diagnostic}"
        );
    }

    #[test]
    fn a_non_transient_probe_failure_is_neither_retried_nor_relabelled() {
        let probe = Arc::new(ScriptedProbe::new(vec![
            BhoonidhiDownloadProbeError::EtagWeak,
        ]));
        let (recorded, backoff) = recording_backoff(Duration::ZERO);
        let resolver = BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone())
            .with_probe_retry_backoff_for_test(backoff);
        let credentials = NoCredentials;

        let error = resolver
            .resolve(&reference(), &descriptor(), &context(&credentials))
            .unwrap_err();
        assert_eq!(
            error.diagnostic(),
            Some("BHOONIDHI_DOWNLOAD_PROBE_ETAG_WEAK")
        );
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert!(recorded.lock().unwrap().is_empty());
    }

    #[test]
    fn a_stop_during_the_probe_retry_wait_ends_the_resolution_as_cancelled() {
        let probe = Arc::new(ScriptedProbe::new(vec![send_timeout()]));
        let (_, backoff) = recording_backoff(Duration::from_secs(30));
        let resolver = BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone())
            .with_probe_retry_backoff_for_test(backoff);
        let credentials = NoCredentials;
        let stopped = std::sync::atomic::AtomicBool::new(false);
        let cancellation = || stopped.load(Ordering::SeqCst);
        static CLOCK: SystemClock = SystemClock;
        let started = std::time::Instant::now();

        let error = thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(100));
                stopped.store(true, Ordering::SeqCst);
            });
            resolver
                .resolve(
                    &reference(),
                    &descriptor(),
                    &ResolveContext::new(&credentials, &cancellation, &CLOCK),
                )
                .unwrap_err()
        });
        assert_eq!(error.kind(), super::super::ResolveErrorKind::Cancelled);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn auth_recovery_second_probe_rejection_has_safe_stage() {
        struct RefreshingProvider {
            refreshes: AtomicUsize,
            rejected: AtomicUsize,
        }
        impl BhoonidhiAccessTokenProvider for RefreshingProvider {
            fn access_token(&self) -> Result<Zeroizing<String>, CatalogAuthError> {
                Ok(Zeroizing::new(FIXTURE_TOKEN.to_owned()))
            }
            fn access_token_with_generation(
                &self,
            ) -> Result<insar_provider::bhoonidhi::BhoonidhiAccessTokenLease, CatalogAuthError>
            {
                insar_provider::bhoonidhi::BhoonidhiAccessTokenLease::new(
                    self.access_token()?,
                    self.refreshes.load(Ordering::SeqCst) as u64,
                )
            }
            fn refresh_after_rejection(
                &self,
                generation: u64,
            ) -> Result<BhoonidhiRefreshResult, CatalogAuthError> {
                assert_eq!(generation, 0);
                self.refreshes.fetch_add(1, Ordering::SeqCst);
                Ok(BhoonidhiRefreshResult::Refreshed)
            }
            fn mark_generation_rejected(&self, generation: u64) -> Result<(), CatalogAuthError> {
                assert_eq!(generation, 1);
                self.rejected.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        struct RejectingProbe {
            calls: AtomicUsize,
            reject_again: bool,
        }
        impl BhoonidhiDownloadProbe for RejectingProbe {
            fn probe(
                &self,
                asset: &ResolvedAsset,
                context: &ResolveContext<'_>,
            ) -> Result<BhoonidhiDownloadProbeEvidence, BhoonidhiDownloadProbeError> {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(asset.auth_generation(), Some(call as u64));
                assert_eq!(
                    asset.effective_url(),
                    &derive_bhoonidhi_download_url(&reference()).unwrap()
                );
                if call == 0 || self.reject_again {
                    Err(BhoonidhiDownloadProbeError::Status(401))
                } else {
                    Probe {
                        calls: AtomicUsize::new(0),
                    }
                    .probe(asset, context)
                }
            }
        }
        for reject_again in [false, true] {
            let probe = Arc::new(RejectingProbe {
                calls: AtomicUsize::new(0),
                reject_again,
            });
            let provider = Arc::new(RefreshingProvider {
                refreshes: AtomicUsize::new(0),
                rejected: AtomicUsize::new(0),
            });
            let resolver = BhoonidhiAssetResolver::new(provider.clone(), probe.clone());
            let result = resolver.resolve(&reference(), &descriptor(), &context(&NoCredentials));
            if reject_again {
                assert_eq!(
                    result.unwrap_err().diagnostic(),
                    Some("BHOONIDHI_DOWNLOAD_PROBE_STATUS_401 stage=post_refresh_probe")
                );
            } else {
                assert_eq!(result.unwrap().auth_generation(), Some(1));
            }
            assert_eq!(probe.calls.load(Ordering::SeqCst), 2);
            assert_eq!(provider.refreshes.load(Ordering::SeqCst), 1);
            assert_eq!(
                provider.rejected.load(Ordering::SeqCst),
                usize::from(reject_again)
            );
        }
    }

    #[test]
    fn runtime_url_contains_only_collection_and_item_identity() {
        let url = derive_bhoonidhi_download_url(&reference()).unwrap();
        assert_eq!(
            url.as_str(),
            format!("{BHOONIDHI_DOWNLOAD_URL}?id={ITEM_ID}&collection={BHOONIDHI_COLLECTION_ID}")
        );
        assert!(!url.as_str().contains("token"));
        assert!(!url.as_str().contains(FIXTURE_TOKEN));
    }

    #[test]
    fn resolver_probes_and_preserves_opaque_multipart_etag() {
        let probe = Arc::new(Probe {
            calls: AtomicUsize::new(0),
        });
        let resolver = BhoonidhiAssetResolver::new(Arc::new(TokenProvider), probe.clone());
        let resolved = resolver
            .resolve(&reference(), &descriptor(), &context(&NoCredentials))
            .unwrap();
        assert_eq!(resolved.expected_size(), Some(21_054_588_114));
        assert_eq!(
            resolved.remote_validator().etag.as_deref(),
            Some("\"308dbc892d0ce417013aa66f28a61cb0-402\"")
        );
        assert_eq!(
            resolved.remote_validator().content_length,
            Some(21_054_588_114)
        );
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        let debug = format!("{resolved:?}");
        assert!(!debug.contains(FIXTURE_TOKEN));
        assert!(!debug.contains("collection_id="));
    }

    #[test]
    fn malformed_probe_ranges_and_weak_or_md5_etags_fail_closed() {
        let url = derive_bhoonidhi_download_url(&reference()).unwrap();
        assert_eq!(parse_probe_content_range("bytes 0-1/2"), None);
        assert_eq!(parse_probe_content_range("bytes 0-0/*"), None);
        assert_eq!(parse_probe_content_range("bytes 0-0/0"), None);
        assert_eq!(
            parse_download_probe_response(
                &url,
                &url,
                206,
                Some("bytes 0-0/2"),
                Some(1),
                Some("W/\"weak\""),
                None,
                1,
            )
            .unwrap_err(),
            BhoonidhiDownloadProbeError::EtagWeak
        );
        assert!(!valid_frozen_checksum(
            Some("MD5"),
            Some("0".repeat(32).as_str())
        ));
    }

    #[test]
    fn frozen_identity_and_descriptor_are_closed() {
        let resolver = BhoonidhiAssetResolver::new(
            Arc::new(TokenProvider),
            Arc::new(Probe {
                calls: AtomicUsize::new(0),
            }),
        );
        let mut frozen = FrozenAssetAccess {
            reference: reference(),
            descriptor: descriptor(),
            role: AssetRole::Primary,
            file_name: None,
            expected_size_bytes: None,
            checksum_algorithm: None,
            checksum_value: None,
        };
        assert!(resolver.validate_frozen(&frozen).is_ok());
        frozen.reference.asset_key = "metadata".to_owned();
        assert_eq!(
            resolver.validate_frozen(&frozen).unwrap_err().kind(),
            super::super::ResolveErrorKind::ReferenceInvalid
        );
    }

    #[test]
    fn download_slots_fail_with_provider_concurrency_status_and_release() {
        let first = try_acquire_bhoonidhi_download_slot().unwrap();
        let second = try_acquire_bhoonidhi_download_slot().unwrap();
        let third = try_acquire_bhoonidhi_download_slot().unwrap();
        assert_eq!(
            try_acquire_bhoonidhi_download_slot().unwrap_err(),
            BhoonidhiDownloadProbeError::Status(412)
        );
        drop(third);
        let replacement = try_acquire_bhoonidhi_download_slot().unwrap();
        drop(replacement);
        drop(second);
        drop(first);
    }

    #[test]
    fn parser_rejects_redirects_and_wrong_probe_body_size() {
        let expected = derive_bhoonidhi_download_url(&reference()).unwrap();
        let redirected = Url::parse("https://bhoonidhi.nrsc.gov.in/download").unwrap();
        assert_eq!(
            parse_download_probe_response(
                &expected,
                &redirected,
                206,
                Some("bytes 0-0/2"),
                Some(1),
                Some("\"opaque-402\""),
                None,
                1,
            )
            .unwrap_err(),
            BhoonidhiDownloadProbeError::UnexpectedUrl
        );
        assert_eq!(
            parse_download_probe_response(
                &expected,
                &expected,
                206,
                Some("bytes 0-0/2"),
                Some(1),
                Some("\"opaque-402\""),
                None,
                2,
            )
            .unwrap_err(),
            BhoonidhiDownloadProbeError::BodyLength
        );
    }

    #[test]
    fn rate_limit_pacer_parses_apisix_headers_and_scopes_to_bhoonidhi_host() {
        use reqwest::header::{HeaderMap, HeaderValue};

        assert!(is_bhoonidhi_download_host(
            &Url::parse("https://bhoonidhi-api.nrsc.gov.in/download").unwrap()
        ));
        assert!(!is_bhoonidhi_download_host(
            &Url::parse("http://127.0.0.1:8080/download").unwrap()
        ));

        let mut h429 = HeaderMap::new();
        h429.insert("x-ratelimit-limit", HeaderValue::from_static("2"));
        h429.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        h429.insert("x-ratelimit-reset", HeaderValue::from_static("35"));
        assert_eq!(
            parse_rate_limit_wait_from_headers(429, &h429),
            Some(Duration::from_secs(36))
        );

        let mut h206_zero = HeaderMap::new();
        h206_zero.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        h206_zero.insert("x-ratelimit-reset", HeaderValue::from_static("42"));
        assert_eq!(
            parse_rate_limit_wait_from_headers(206, &h206_zero),
            Some(Duration::from_secs(43))
        );

        let mut h206_one = HeaderMap::new();
        h206_one.insert("x-ratelimit-remaining", HeaderValue::from_static("1"));
        h206_one.insert("x-ratelimit-reset", HeaderValue::from_static("42"));
        assert_eq!(parse_rate_limit_wait_from_headers(206, &h206_one), None);

        let mut h_clamped = HeaderMap::new();
        h_clamped.insert("x-ratelimit-reset", HeaderValue::from_static("999"));
        assert_eq!(
            parse_rate_limit_wait_from_headers(429, &h_clamped),
            Some(Duration::from_secs(121))
        );
    }
}
