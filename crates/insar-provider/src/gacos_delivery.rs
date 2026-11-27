//! Bounded, offline-testable construction of the legacy GACOS delivery request.
//!
//! The current repository has one concrete GACOS wire contract: a form POST to
//! [`GACOS_SUBMIT_ENDPOINT`].  The service sends a result link by email and does
//! not expose a verified machine-readable result protocol.  The request builder and
//! classifier are I/O-free; the optional transport performs one caller-authorized form
//! POST only.  The module never reads a mailbox, follows a result URL, or persists an
//! email/result link.

use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;
use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use reqwest::blocking::{Client, Response};
use reqwest::header::CONTENT_TYPE;
use thiserror::Error;
use url::{form_urlencoded, Url};
use zeroize::Zeroizing;

/// The current GACOS form action confirmed by a bounded read-only page GET.
pub const GACOS_SUBMIT_ENDPOINT: &str = "http://www.gacos.net/M/action_page.php";
/// The current GACOS web root linked by the COMET service page.
pub const GACOS_WEB_ROOT: &str = "http://www.gacos.net/";
/// GACOS accepts at most this many dates in one form submission.
pub const GACOS_MAX_DATES_PER_REQUEST: usize = 20;
/// GACOS accepts at most a ten-degree span in either coordinate direction.
pub const GACOS_MAX_SPAN_DEGREES: f64 = 10.0;
/// Response bodies are bounded before any text classification is attempted.
pub const GACOS_MAX_RESPONSE_BYTES: usize = 64 * 1024;
/// A manually pasted result link is held only as a bounded runtime value.
pub const GACOS_MAX_RESULT_URL_BYTES: usize = 4096;
/// Default connect timeout for one native form POST.
pub const GACOS_DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Default overall timeout for one native form POST.
pub const GACOS_DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// Maximum caller-configurable overall timeout for one native form POST.
pub const GACOS_MAX_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);
const GACOS_MAX_EMAIL_BYTES: usize = 320;
const GACOS_MAX_DATE_YEAR: i32 = 9999;
const GACOS_FORM_TYPE_GEOTIFF: &str = "2";
const GACOS_FORM_TYPE_BINARY: &str = "1";
const GACOS_FORM_SEQUENCE: &str = "OSM Map";
/// GACOS currently has no verified automatic result-delivery contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GacosDeliveryMode {
    /// The application may require manual confirmation; any emailed result link
    /// belongs to the separate result download/import workflow.
    ManualActionRequired,
}

/// Return the delivery mode supported by the verified contract.
pub const fn delivery_mode() -> GacosDeliveryMode {
    GacosDeliveryMode::ManualActionRequired
}

/// Typed validation failures for request construction and bounded response parsing.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GacosDeliveryError {
    #[error("GACOS request has no dates")]
    MissingDates,
    #[error("GACOS request contains a duplicate date")]
    DuplicateDate,
    #[error("GACOS request exceeds the 20-date batch limit")]
    TooManyDates,
    #[error("GACOS date cannot be represented as YYYYMMDD")]
    InvalidDate,
    #[error("GACOS UTC time is invalid")]
    InvalidUtcTime,
    #[error("GACOS bounding box is invalid")]
    InvalidBoundingBox,
    #[error("GACOS delivery email is invalid")]
    InvalidEmail,
    #[error("GACOS submission endpoint is not the reviewed endpoint")]
    InvalidSubmissionEndpoint,
    #[error("GACOS result URL is invalid for the manual boundary")]
    InvalidResultUrl,
    #[error("GACOS result URL host is not a reviewed GACOS host")]
    UnreviewedResultHost,
    #[error("GACOS response body exceeds the 64 KiB bound")]
    ResponseTooLarge,
    #[error("GACOS response status is invalid")]
    InvalidResponseStatus,
    #[error("GACOS transport configuration is invalid")]
    InvalidTransportConfiguration,
    #[error("GACOS POST outcome is uncertain after it was sent")]
    SubmissionUncertain,
    #[error("GACOS POST was cancelled before it was sent")]
    CancelledBeforeSend,
}

/// A finite, ordered GACOS request bounding box in decimal degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GacosBoundingBox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl GacosBoundingBox {
    pub const fn new(west: f64, south: f64, east: f64, north: f64) -> Self {
        Self {
            west,
            south,
            east,
            north,
        }
    }

    pub fn validate(&self) -> Result<(), GacosDeliveryError> {
        if !self.west.is_finite()
            || !self.south.is_finite()
            || !self.east.is_finite()
            || !self.north.is_finite()
            || self.west < -180.0
            || self.west > 180.0
            || self.east < -180.0
            || self.east > 180.0
            || self.south < -90.0
            || self.south > 90.0
            || self.north < -90.0
            || self.north > 90.0
            || self.west >= self.east
            || self.south >= self.north
            || self.east - self.west > GACOS_MAX_SPAN_DEGREES
            || self.north - self.south > GACOS_MAX_SPAN_DEGREES
        {
            return Err(GacosDeliveryError::InvalidBoundingBox);
        }
        Ok(())
    }

    pub const fn width(&self) -> f64 {
        self.east - self.west
    }

    pub const fn height(&self) -> f64 {
        self.north - self.south
    }
}

/// Explicit UTC time used by the GACOS form's separate H/M fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GacosUtcTime {
    hour: u8,
    minute: u8,
}

impl GacosUtcTime {
    pub const fn new(hour: u8, minute: u8) -> Result<Self, GacosDeliveryError> {
        if hour > 23 || minute > 59 {
            return Err(GacosDeliveryError::InvalidUtcTime);
        }
        Ok(Self { hour, minute })
    }

    pub const fn hour(self) -> u8 {
        self.hour
    }

    pub const fn minute(self) -> u8 {
        self.minute
    }

    pub fn hhmm(self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

/// Output format accepted by the legacy GACOS form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GacosOutputFormat {
    /// `type=2`, the GeoTIFF product.
    #[default]
    GeoTiff,
    /// `type=1`, the little-endian binary grid.
    Binary,
}

impl GacosOutputFormat {
    pub const fn form_value(self) -> &'static str {
        match self {
            Self::GeoTiff => GACOS_FORM_TYPE_GEOTIFF,
            Self::Binary => GACOS_FORM_TYPE_BINARY,
        }
    }
}

/// A frozen request independent of runtime credentials.
#[derive(Clone, Debug, PartialEq)]
pub struct GacosRequestInput {
    pub bbox: GacosBoundingBox,
    pub dates: Vec<NaiveDate>,
    pub utc_time: GacosUtcTime,
    pub output_format: GacosOutputFormat,
}

impl GacosRequestInput {
    pub fn new(
        bbox: GacosBoundingBox,
        dates: Vec<NaiveDate>,
        utc_time: GacosUtcTime,
        output_format: GacosOutputFormat,
    ) -> Self {
        Self {
            bbox,
            dates,
            utc_time,
            output_format,
        }
    }

    pub fn validate(&self) -> Result<(), GacosDeliveryError> {
        self.bbox.validate()?;
        if self.dates.is_empty() {
            return Err(GacosDeliveryError::MissingDates);
        }
        if self.dates.len() > GACOS_MAX_DATES_PER_REQUEST {
            return Err(GacosDeliveryError::TooManyDates);
        }
        let mut seen = BTreeSet::new();
        for date in &self.dates {
            if !representable_date(*date) {
                return Err(GacosDeliveryError::InvalidDate);
            }
            if !seen.insert(*date) {
                return Err(GacosDeliveryError::DuplicateDate);
            }
        }
        // The constructor is normally the first validation boundary, but keep
        // this check here so a value assembled by deserialization cannot bypass it.
        if self.utc_time.hour > 23 || self.utc_time.minute > 59 {
            return Err(GacosDeliveryError::InvalidUtcTime);
        }
        Ok(())
    }

    /// Return validated dates in deterministic ascending order for the form body.
    pub fn normalized_dates(&self) -> Result<Vec<NaiveDate>, GacosDeliveryError> {
        self.validate()?;
        let mut dates = self.dates.clone();
        dates.sort_unstable();
        Ok(dates)
    }
}

fn representable_date(date: NaiveDate) -> bool {
    let year = date.year();
    (1..=GACOS_MAX_DATE_YEAR).contains(&year)
}

fn validate_email(email: &str) -> Result<(), GacosDeliveryError> {
    if email.is_empty()
        || email.len() > GACOS_MAX_EMAIL_BYTES
        || email.trim() != email
        || email
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(GacosDeliveryError::InvalidEmail);
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err(GacosDeliveryError::InvalidEmail);
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
    {
        return Err(GacosDeliveryError::InvalidEmail);
    }
    Ok(())
}

fn coordinate_text(value: f64) -> String {
    // Avoid emitting `-0`, which is a distinct string but the same coordinate.
    let value = if value == 0.0 { 0.0 } else { value };
    value.to_string()
}

/// A runtime-only form POST. The encoded body is zeroized on drop and its
/// `Debug` implementation never includes the email or form body.
pub struct GacosFormPost {
    endpoint: Url,
    body: Zeroizing<Vec<u8>>,
}

impl GacosFormPost {
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub fn content_type(&self) -> &'static str {
        "application/x-www-form-urlencoded"
    }

    /// Borrow the encoded body for one immediate transport operation.
    pub fn body(&self) -> &[u8] {
        self.body.as_slice()
    }
}

impl fmt::Debug for GacosFormPost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GacosFormPost")
            .field("endpoint", &self.endpoint)
            .field("body", &"<redacted>")
            .finish()
    }
}

/// Validate the exact legacy form endpoint. A caller cannot silently substitute
/// a redirector, a different host, credentials, or an alternate path.
pub fn validate_submission_endpoint(endpoint: &str) -> Result<Url, GacosDeliveryError> {
    if endpoint != GACOS_SUBMIT_ENDPOINT {
        return Err(GacosDeliveryError::InvalidSubmissionEndpoint);
    }
    Url::parse(endpoint).map_err(|_| GacosDeliveryError::InvalidSubmissionEndpoint)
}

/// Construct the observed legacy GACOS form body without performing I/O.
pub fn build_form_post(
    input: &GacosRequestInput,
    delivery_email: &str,
) -> Result<GacosFormPost, GacosDeliveryError> {
    build_form_post_at(GACOS_SUBMIT_ENDPOINT, input, delivery_email)
}

/// Testable construction entry point. Production callers should use
/// [`build_form_post`], which fixes the reviewed endpoint.
pub fn build_form_post_at(
    endpoint: &str,
    input: &GacosRequestInput,
    delivery_email: &str,
) -> Result<GacosFormPost, GacosDeliveryError> {
    let endpoint = validate_submission_endpoint(endpoint)?;
    input.validate()?;
    validate_email(delivery_email)?;
    let dates = input.normalized_dates()?;
    let date_list = dates
        .iter()
        .map(|date| date.format("%Y%m%d").to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let mut form = form_urlencoded::Serializer::new(String::new());
    form.append_pair("N", &coordinate_text(input.bbox.north));
    form.append_pair("S", &coordinate_text(input.bbox.south));
    form.append_pair("W", &coordinate_text(input.bbox.west));
    form.append_pair("E", &coordinate_text(input.bbox.east));
    form.append_pair("H", &input.utc_time.hour.to_string());
    form.append_pair("M", &input.utc_time.minute.to_string());
    form.append_pair("date", &date_list);
    form.append_pair("type", input.output_format.form_value());
    form.append_pair("email", delivery_email);
    form.append_pair("seq", GACOS_FORM_SEQUENCE);

    Ok(GacosFormPost {
        endpoint,
        body: Zeroizing::new(form.finish().into_bytes()),
    })
}

/// Bounds and timeouts for one native form POST.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GacosTransportTimeouts {
    connect_timeout: Duration,
    total_timeout: Duration,
}

impl GacosTransportTimeouts {
    pub fn new(
        connect_timeout: Duration,
        total_timeout: Duration,
    ) -> Result<Self, GacosDeliveryError> {
        if connect_timeout.is_zero()
            || total_timeout.is_zero()
            || total_timeout > GACOS_MAX_TOTAL_TIMEOUT
            || connect_timeout > total_timeout
        {
            return Err(GacosDeliveryError::InvalidTransportConfiguration);
        }
        Ok(Self {
            connect_timeout,
            total_timeout,
        })
    }

    pub const fn connect_timeout(self) -> Duration {
        self.connect_timeout
    }

    pub const fn total_timeout(self) -> Duration {
        self.total_timeout
    }
}

impl Default for GacosTransportTimeouts {
    fn default() -> Self {
        Self {
            connect_timeout: GACOS_DEFAULT_CONNECT_TIMEOUT,
            total_timeout: GACOS_DEFAULT_TOTAL_TIMEOUT,
        }
    }
}

/// A bounded response returned by an injectable form executor.
pub struct GacosHttpResponse {
    status: u16,
    body: Zeroizing<Vec<u8>>,
}

impl GacosHttpResponse {
    pub fn new(status: u16, body: Vec<u8>) -> Result<Self, GacosDeliveryError> {
        if !(100..=599).contains(&status) {
            return Err(GacosDeliveryError::InvalidResponseStatus);
        }
        if body.len() > GACOS_MAX_RESPONSE_BYTES {
            return Err(GacosDeliveryError::ResponseTooLarge);
        }
        Ok(Self {
            status,
            body: Zeroizing::new(body),
        })
    }

    fn from_zeroized(status: u16, body: Zeroizing<Vec<u8>>) -> Result<Self, GacosDeliveryError> {
        if !(100..=599).contains(&status) {
            return Err(GacosDeliveryError::InvalidResponseStatus);
        }
        Ok(Self { status, body })
    }

    pub const fn status(&self) -> u16 {
        self.status
    }

    pub fn body(&self) -> &[u8] {
        self.body.as_slice()
    }
}

impl fmt::Debug for GacosHttpResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GacosHttpResponse")
            .field("status", &self.status)
            .field("body", &"<redacted>")
            .finish()
    }
}

/// Injectable one-shot form executor. Implementations must never retry the POST.
pub trait GacosFormPostExecutor: Send + Sync {
    fn execute_form_post(
        &self,
        post: &GacosFormPost,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<GacosHttpResponse, GacosDeliveryError>;
}

/// Runs response classification after one injected form execution.
pub struct GacosDeliveryAdapter<E> {
    executor: E,
}

impl<E: GacosFormPostExecutor> GacosDeliveryAdapter<E> {
    pub const fn new(executor: E) -> Self {
        Self { executor }
    }

    pub fn submit(
        &self,
        post: &GacosFormPost,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<GacosSubmissionReceipt, GacosDeliveryError> {
        if cancelled() {
            return Err(GacosDeliveryError::CancelledBeforeSend);
        }
        if post.endpoint().as_str() != GACOS_SUBMIT_ENDPOINT {
            return Err(GacosDeliveryError::InvalidSubmissionEndpoint);
        }
        let response = self.executor.execute_form_post(post, cancelled)?;
        classify_submission_response(response.status(), response.body())
    }

    pub const fn executor(&self) -> &E {
        &self.executor
    }
}

/// Native blocking reqwest transport for exactly one GACOS form POST.
///
/// The client has an explicit connect/overall timeout, disables redirects, and
/// disables reqwest retries. A transport or response-read failure after the POST
/// has been handed to reqwest returns [`GacosDeliveryError::SubmissionUncertain`].
pub struct GacosReqwestTransport {
    client: Client,
    timeouts: GacosTransportTimeouts,
}

impl fmt::Debug for GacosReqwestTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GacosReqwestTransport")
            .field("timeouts", &self.timeouts)
            .field("redirects", &"disabled")
            .field("retries", &"disabled")
            .finish()
    }
}

impl GacosReqwestTransport {
    pub fn new(timeouts: GacosTransportTimeouts) -> Result<Self, GacosDeliveryError> {
        Self::with_route(timeouts, insar_core::ProxyPolicy::System)
    }

    pub fn with_route(
        timeouts: GacosTransportTimeouts,
        route: insar_core::ProxyPolicy,
    ) -> Result<Self, GacosDeliveryError> {
        let builder = Client::builder()
            .connect_timeout(timeouts.connect_timeout())
            .timeout(timeouts.total_timeout())
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never());
        let client = crate::network_proxy::blocking_builder_with_proxy(builder, route)
            .build()
            .map_err(|_| GacosDeliveryError::InvalidTransportConfiguration)?;
        Ok(Self { client, timeouts })
    }

    pub fn production() -> Result<Self, GacosDeliveryError> {
        Self::new(GacosTransportTimeouts::default())
    }

    pub fn production_with_route(
        route: insar_core::ProxyPolicy,
    ) -> Result<Self, GacosDeliveryError> {
        Self::with_route(GacosTransportTimeouts::default(), route)
    }

    pub const fn timeouts(&self) -> GacosTransportTimeouts {
        self.timeouts
    }
}

fn read_bounded_response(
    mut response: Response,
    cancelled: &dyn Fn() -> bool,
) -> Result<Zeroizing<Vec<u8>>, GacosDeliveryError> {
    if response
        .content_length()
        .is_some_and(|length| length > GACOS_MAX_RESPONSE_BYTES as u64)
    {
        return Err(GacosDeliveryError::SubmissionUncertain);
    }
    let mut body = Zeroizing::new(Vec::new());
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        if cancelled() {
            return Err(GacosDeliveryError::SubmissionUncertain);
        }
        let count = response
            .read(&mut chunk)
            .map_err(|_| GacosDeliveryError::SubmissionUncertain)?;
        if count == 0 {
            break;
        }
        let Some(next_len) = body.len().checked_add(count) else {
            return Err(GacosDeliveryError::SubmissionUncertain);
        };
        if next_len > GACOS_MAX_RESPONSE_BYTES {
            return Err(GacosDeliveryError::SubmissionUncertain);
        }
        body.extend_from_slice(&chunk[..count]);
    }
    if cancelled() {
        return Err(GacosDeliveryError::SubmissionUncertain);
    }
    Ok(body)
}

impl GacosFormPostExecutor for GacosReqwestTransport {
    fn execute_form_post(
        &self,
        post: &GacosFormPost,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<GacosHttpResponse, GacosDeliveryError> {
        if cancelled() {
            return Err(GacosDeliveryError::CancelledBeforeSend);
        }
        let response = self
            .client
            .post(post.endpoint().clone())
            .header(CONTENT_TYPE, post.content_type())
            .body(post.body().to_vec())
            .send()
            .map_err(|_| GacosDeliveryError::SubmissionUncertain)?;
        let status = response.status().as_u16();
        let body = read_bounded_response(response, cancelled)?;
        GacosHttpResponse::from_zeroized(status, body)
    }
}

/// A user-supplied result link. It remains manual-only and is not serializable.
pub struct GacosManualResultLink {
    url: Url,
}

impl GacosManualResultLink {
    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn as_str(&self) -> &str {
        self.url.as_str()
    }

    pub const fn requires_manual_action(&self) -> bool {
        true
    }
}

impl fmt::Debug for GacosManualResultLink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GacosManualResultLink")
            .field("url", &"<redacted>")
            .field("manual_action_required", &true)
            .finish()
    }
}

/// Validate only the syntax of a result URL supplied by the user. The caller
/// must keep it runtime-only and must not use this helper as a fetch/redirect
/// authorization decision.
pub fn validate_manual_result_url(raw: &str) -> Result<GacosManualResultLink, GacosDeliveryError> {
    if raw.is_empty()
        || raw.len() > GACOS_MAX_RESULT_URL_BYTES
        || raw.trim() != raw
        || raw.chars().any(char::is_control)
    {
        return Err(GacosDeliveryError::InvalidResultUrl);
    }
    let url = Url::parse(raw).map_err(|_| GacosDeliveryError::InvalidResultUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port().is_some()
        || url.path().is_empty()
        || url.path() == "/"
    {
        return Err(GacosDeliveryError::InvalidResultUrl);
    }
    let Some(host) = url.host_str() else {
        return Err(GacosDeliveryError::InvalidResultUrl);
    };
    if !host.eq_ignore_ascii_case("www.gacos.net") && !host.eq_ignore_ascii_case("gacos.net") {
        return Err(GacosDeliveryError::UnreviewedResultHost);
    }
    Ok(GacosManualResultLink { url })
}

/// The safe outcome of classifying one legacy form response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GacosSubmissionOutcome {
    /// The response explicitly acknowledged the application, not a result download.
    Submitted,
    /// A 2xx response had no documented explicit acceptance evidence. The
    /// caller must verify the submission manually before treating it as sent.
    ManualVerificationRequired,
    /// The provider or the response body rejected the request.
    Rejected,
    /// The POST outcome cannot be established; automatic retry is forbidden.
    Uncertain,
}

/// Why a response received its typed outcome. No response text is retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GacosSubmissionReason {
    ExplicitSuccessEvidence,
    NoExplicitSuccessEvidence,
    ProviderRejected,
    RedirectNotFollowed,
    AcceptanceUncertain,
}

/// Bounded response metadata safe for task/history projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GacosSubmissionReceipt {
    http_status: u16,
    response_bytes: usize,
    outcome: GacosSubmissionOutcome,
    reason: GacosSubmissionReason,
}

impl GacosSubmissionReceipt {
    pub const fn http_status(self) -> u16 {
        self.http_status
    }

    pub const fn response_bytes(self) -> usize {
        self.response_bytes
    }

    pub const fn outcome(self) -> GacosSubmissionOutcome {
        self.outcome
    }

    pub const fn reason(self) -> GacosSubmissionReason {
        self.reason
    }

    pub const fn manual_action_required(self) -> bool {
        matches!(
            self.outcome,
            GacosSubmissionOutcome::ManualVerificationRequired
        )
    }

    pub const fn automatic_retry_allowed(self) -> bool {
        // The legacy form has no idempotency key; retrying after a response or
        // transport ambiguity could submit a duplicate request.
        false
    }
}

/// Classify one already-received response without retaining or returning its body.
pub fn classify_submission_response(
    http_status: u16,
    body: &[u8],
) -> Result<GacosSubmissionReceipt, GacosDeliveryError> {
    if body.len() > GACOS_MAX_RESPONSE_BYTES {
        return Err(GacosDeliveryError::ResponseTooLarge);
    }
    if !(200..=599).contains(&http_status) {
        return Err(GacosDeliveryError::InvalidResponseStatus);
    }

    let (outcome, reason) = if http_status == 408 || http_status == 429 || http_status >= 500 {
        (
            GacosSubmissionOutcome::Uncertain,
            GacosSubmissionReason::AcceptanceUncertain,
        )
    } else if http_status >= 400 {
        (
            GacosSubmissionOutcome::Rejected,
            GacosSubmissionReason::ProviderRejected,
        )
    } else if http_status >= 300 {
        // A POST can take effect before redirecting to a confirmation page.
        // Do not follow it and do not claim that the provider rejected it.
        (
            GacosSubmissionOutcome::Uncertain,
            GacosSubmissionReason::RedirectNotFollowed,
        )
    } else {
        classify_application_page(body)
    };

    Ok(GacosSubmissionReceipt {
        http_status,
        response_bytes: body.len(),
        outcome,
        reason,
    })
}

/// Recognize the explicit response examples already covered by the legacy
/// Python client tests. This is a small offline-tested response profile, not
/// evidence that the live service currently emits that protocol. Unreviewed
/// markup (scripts, comments, forms, hidden elements) or extra page text stays
/// unconfirmed rather than inheriting the old client's no-error-means-success rule.
fn classify_application_page(body: &[u8]) -> (GacosSubmissionOutcome, GacosSubmissionReason) {
    let unknown = (
        GacosSubmissionOutcome::ManualVerificationRequired,
        GacosSubmissionReason::NoExplicitSuccessEvidence,
    );
    let Ok(text) = std::str::from_utf8(body) else {
        return unknown;
    };
    let mut visible = Zeroizing::new(text.to_ascii_lowercase());
    for wrapper in [
        "<html>", "</html>", "<body>", "</body>", "<p>", "</p>", "<div>", "</div>", "<span>",
        "</span>", "<br>", "<br/>", "<br />",
    ] {
        *visible = visible.replace(wrapper, " ");
    }
    // Remaining markup cannot be interpreted as a visible provider receipt.
    if visible.contains(['<', '>']) {
        return unknown;
    }
    let normalized = Zeroizing::new(visible.split_whitespace().collect::<Vec<_>>().join(" "));
    if normalized.starts_with("error:") || normalized.starts_with("failed:") {
        return (
            GacosSubmissionOutcome::Rejected,
            GacosSubmissionReason::ProviderRejected,
        );
    }
    if normalized.trim_end_matches(['.', '!']) == "request received" {
        return (
            GacosSubmissionOutcome::Submitted,
            GacosSubmissionReason::ExplicitSuccessEvidence,
        );
    }
    unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    fn valid_input() -> GacosRequestInput {
        GacosRequestInput::new(
            GacosBoundingBox::new(10.25, 20.5, 12.75, 22.0),
            vec![
                NaiveDate::from_ymd_opt(2026, 9, 5).expect("fixture date"),
                NaiveDate::from_ymd_opt(2026, 9, 3).expect("fixture date"),
            ],
            GacosUtcTime::new(4, 7).expect("fixture time"),
            GacosOutputFormat::GeoTiff,
        )
    }

    #[test]
    fn form_fixture_is_deterministic_and_redacts_email() {
        let post = build_form_post(&valid_input(), "operator@example.test").expect("valid form");
        assert_eq!(post.endpoint().as_str(), GACOS_SUBMIT_ENDPOINT);
        assert_eq!(post.content_type(), "application/x-www-form-urlencoded");
        let pairs = form_urlencoded::parse(post.body()).collect::<Vec<_>>();
        assert_eq!(pairs[0], ("N".into(), "22".into()));
        assert_eq!(pairs[1], ("S".into(), "20.5".into()));
        assert_eq!(pairs[2], ("W".into(), "10.25".into()));
        assert_eq!(pairs[3], ("E".into(), "12.75".into()));
        assert_eq!(pairs[4], ("H".into(), "4".into()));
        assert_eq!(pairs[5], ("M".into(), "7".into()));
        assert_eq!(pairs[6], ("date".into(), "20260903\n20260905".into()));
        assert_eq!(pairs[7], ("type".into(), "2".into()));
        assert_eq!(pairs[8], ("email".into(), "operator@example.test".into()));
        assert_eq!(pairs[9], ("seq".into(), "OSM Map".into()));
        let debug = format!("{post:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("operator@example.test"));
    }

    #[test]
    fn invalid_request_inputs_are_rejected() {
        let mut too_many = valid_input();
        too_many.dates = (0..=GACOS_MAX_DATES_PER_REQUEST)
            .map(|offset| {
                NaiveDate::from_ymd_opt(2026, 1, 1).expect("fixture date")
                    + ChronoDuration::days(offset as i64)
            })
            .collect();
        assert_eq!(too_many.validate(), Err(GacosDeliveryError::TooManyDates));

        let mut duplicate = valid_input();
        duplicate.dates.push(duplicate.dates[0]);
        assert_eq!(duplicate.validate(), Err(GacosDeliveryError::DuplicateDate));

        let invalid_bbox = GacosBoundingBox::new(0.0, 0.0, 10.1, 1.0);
        assert_eq!(
            invalid_bbox.validate(),
            Err(GacosDeliveryError::InvalidBoundingBox)
        );
        assert_eq!(
            GacosUtcTime::new(24, 0),
            Err(GacosDeliveryError::InvalidUtcTime)
        );
        assert!(matches!(
            build_form_post(&valid_input(), "operator @example.test"),
            Err(GacosDeliveryError::InvalidEmail)
        ));
    }

    #[test]
    fn endpoint_and_result_url_validation_are_strict_at_the_boundary() {
        assert!(validate_submission_endpoint(GACOS_SUBMIT_ENDPOINT).is_ok());
        assert_eq!(
            validate_submission_endpoint("https://evil.example/M/action_page.php"),
            Err(GacosDeliveryError::InvalidSubmissionEndpoint)
        );
        let link = validate_manual_result_url("https://www.gacos.net/pub/result.zip?token=runtime")
            .expect("valid manual link");
        assert!(link.requires_manual_action());
        assert!(!format!("{link:?}").contains("token=runtime"));
        assert!(matches!(
            validate_manual_result_url("https://download.example/result.zip"),
            Err(GacosDeliveryError::UnreviewedResultHost)
        ));
        assert!(validate_manual_result_url("ftp://www.gacos.net/pub/result.zip").is_err());
        assert!(validate_manual_result_url("javascript:alert(1)").is_err());
        assert!(validate_manual_result_url("https://www.gacos.net/").is_err());
    }

    struct FixtureExecutor {
        result: Result<(u16, Vec<u8>), GacosDeliveryError>,
    }

    impl GacosFormPostExecutor for FixtureExecutor {
        fn execute_form_post(
            &self,
            _post: &GacosFormPost,
            _cancelled: &dyn Fn() -> bool,
        ) -> Result<GacosHttpResponse, GacosDeliveryError> {
            match &self.result {
                Ok((status, body)) => GacosHttpResponse::new(*status, body.clone()),
                Err(error) => Err(error.clone()),
            }
        }
    }

    #[test]
    fn injectable_adapter_preserves_manual_verification_and_uncertain_errors() {
        let post = build_form_post(&valid_input(), "operator@example.test").expect("valid form");
        let adapter = GacosDeliveryAdapter::new(FixtureExecutor {
            result: Ok((200, b"<html>request queued</html>".to_vec())),
        });
        let receipt = adapter.submit(&post, &|| false).expect("fixture response");
        assert_eq!(
            receipt.outcome(),
            GacosSubmissionOutcome::ManualVerificationRequired
        );

        let uncertain_adapter = GacosDeliveryAdapter::new(FixtureExecutor {
            result: Err(GacosDeliveryError::SubmissionUncertain),
        });
        assert!(matches!(
            uncertain_adapter.submit(&post, &|| false),
            Err(GacosDeliveryError::SubmissionUncertain)
        ));
        assert!(matches!(
            adapter.submit(&post, &|| true),
            Err(GacosDeliveryError::CancelledBeforeSend)
        ));
    }

    #[test]
    fn response_fixture_enforces_status_and_body_bounds() {
        assert!(matches!(
            GacosHttpResponse::new(0, Vec::new()),
            Err(GacosDeliveryError::InvalidResponseStatus)
        ));
        assert!(matches!(
            GacosHttpResponse::new(200, vec![b'x'; GACOS_MAX_RESPONSE_BYTES + 1]),
            Err(GacosDeliveryError::ResponseTooLarge)
        ));
        assert_eq!(
            GacosTransportTimeouts::new(Duration::ZERO, Duration::from_secs(1)),
            Err(GacosDeliveryError::InvalidTransportConfiguration)
        );
        assert_eq!(
            GacosTransportTimeouts::new(Duration::from_secs(2), Duration::from_secs(1)),
            Err(GacosDeliveryError::InvalidTransportConfiguration)
        );
    }

    #[test]
    fn legacy_explicit_application_response_is_submitted_without_a_result_link() {
        for page in [
            "<html>Request received</html>",
            "<html><body><p> Request\n received. </p></body></html>",
        ] {
            let receipt = classify_submission_response(200, page.as_bytes()).unwrap();
            assert_eq!(receipt.outcome(), GacosSubmissionOutcome::Submitted);
            assert_eq!(
                receipt.reason(),
                GacosSubmissionReason::ExplicitSuccessEvidence
            );
            assert!(!receipt.manual_action_required());
            assert!(!receipt.automatic_retry_allowed());
        }
        let rejection =
            classify_submission_response(200, b"Error: maximum 20 dates exceeded").unwrap();
        assert_eq!(rejection.outcome(), GacosSubmissionOutcome::Rejected);
        let mixed =
            classify_submission_response(200, b"Error: request received but processing failed")
                .unwrap();
        assert_eq!(mixed.outcome(), GacosSubmissionOutcome::Rejected);
    }

    #[test]
    fn unrecognized_pages_and_hidden_confirmation_are_never_submitted() {
        for page in [
            "",
            "OK",
            "Request not received",
            "Please log in",
            "Captcha required",
            "<script>Request received</script>",
            "<!-- Request received -->",
            "<p hidden>Request received</p>",
            "<form>Request received</form>",
            "<html>Login required. Request received</html>",
            "<script>const error = false</script>",
        ] {
            let receipt = classify_submission_response(200, page.as_bytes()).unwrap();
            assert_eq!(
                receipt.outcome(),
                GacosSubmissionOutcome::ManualVerificationRequired
            );
            assert!(!receipt.automatic_retry_allowed());
        }
        for status in [302, 408, 429, 500] {
            let receipt = classify_submission_response(status, b"Request received").unwrap();
            assert_eq!(receipt.outcome(), GacosSubmissionOutcome::Uncertain);
            assert!(!receipt.automatic_retry_allowed());
        }
    }

    #[test]
    fn responses_are_bounded_and_never_promote_manual_delivery_to_download() {
        let unverified = classify_submission_response(200, b"<html>request queued</html>")
            .expect("unverified fixture");
        assert_eq!(
            unverified.outcome(),
            GacosSubmissionOutcome::ManualVerificationRequired
        );
        assert_eq!(
            unverified.reason(),
            GacosSubmissionReason::NoExplicitSuccessEvidence
        );
        assert!(unverified.manual_action_required());
        assert!(!unverified.automatic_retry_allowed());

        let rejected = classify_submission_response(200, b"<html>invalid date</html>")
            .expect("rejected fixture");
        assert_eq!(
            rejected.outcome(),
            GacosSubmissionOutcome::ManualVerificationRequired
        );
        assert_eq!(
            rejected.reason(),
            GacosSubmissionReason::NoExplicitSuccessEvidence
        );
        let redirected = classify_submission_response(302, b"redirect").unwrap();
        assert_eq!(redirected.outcome(), GacosSubmissionOutcome::Uncertain);
        assert_eq!(
            redirected.reason(),
            GacosSubmissionReason::RedirectNotFollowed
        );
        assert_eq!(
            classify_submission_response(403, b"denied")
                .unwrap()
                .outcome(),
            GacosSubmissionOutcome::Rejected
        );

        let uncertain = classify_submission_response(503, b"temporary service response")
            .expect("uncertain fixture");
        assert_eq!(uncertain.outcome(), GacosSubmissionOutcome::Uncertain);
        assert!(!uncertain.automatic_retry_allowed());

        let oversized = vec![b'x'; GACOS_MAX_RESPONSE_BYTES + 1];
        assert_eq!(
            classify_submission_response(200, &oversized),
            Err(GacosDeliveryError::ResponseTooLarge)
        );
    }
}
