use std::collections::VecDeque;
use std::fmt;
use std::fs;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use insar_core::{
    sanitize_external_error_message, AssetRole, DownloadFailure, DownloadOutcome, ProxyPolicy,
};
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, CONTENT_RANGE, ETAG, IF_RANGE, LAST_MODIFIED, LOCATION, RANGE,
    RETRY_AFTER, WWW_AUTHENTICATE,
};
use reqwest::redirect::Policy;
use reqwest::{StatusCode, Url};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::asset_resolver::{
    asf_extractor_signed_object_diagnostic, try_acquire_bhoonidhi_download_slot,
    BhoonidhiDownloadProbeError,
};
use crate::checksum::{ChecksumAccumulator, Sha256Accumulator};
use crate::disk_guard::{default_free_space_probe, FreeSpaceProbe};
use crate::file_core::{
    check_ssd_staging_space, finalize_part_with_sha256, is_slow_target_volume,
    open_checked_download_read, open_part_preallocated_for_segmented, part_completed_bytes,
    part_url_sha256, promote_staged_part_to_destination, read_exact_at, save_part_meta_checkpoint,
    staging_target_path, write_all_at, PartMetaCheckpoint, SegmentProgressEntry,
};
use crate::hdf5_range_transfer::parse_canonical_http_date;
use crate::segment_planner::{
    accept_ranges_supports_bytes, parse_content_range, parse_entity_tag,
    partition_half_open_ranges, plan_segments, validate_segment_response, EntityTag,
    FrozenChecksumAlgorithm, FrozenChecksumEvidence, LastModified, MaximumSegmentPolicy,
    RangeCapability, RemoteLength, SegmentLevel, SegmentPlan, SegmentPlanContext,
    SegmentPlanningDecision, SegmentPlanningInput, SegmentResponseDecision,
    SegmentResponseEvidence, SegmentResponseStatus, StableObjectIdentity,
};
use crate::transfer_engine_prefs::{active_transfer_engine_prefs, normalize_per_file_segments};
use crate::transfer_tuning::{
    global_bandwidth_limiter, global_host_concurrency_tracker, AcquireOutcome, ActiveStreamGuard,
    BandwidthLimiter, DynamicSegmentPool, HostConcurrencyTracker, RelativeSpeedConfig,
    RelativeSpeedTracker, DEFAULT_MIN_SPLIT_CHUNK_BYTES, DEFAULT_RESUME_OVERLAP_BYTES,
};
use crate::{
    finalize_part, inspect_destination, inspect_resolved_destination, open_part_for_transfer,
    part_path, transfer_admission::NormalizedOrigin, verify_file_checksum, ArtifactValidationError,
    ArtifactValidationPlan, AuthRefreshPolicyId, ChecksumAlgorithm, DownloadFileError,
    ExpectedChecksum, FilePreparation, RefreshDecision, RemoteValidator, ResolvedAsset,
    ResolvedMethod, ResolvedPartContext,
};

#[derive(Clone)]
pub struct TransferRequest {
    pub task_id: String,
    pub item_id: String,
    url: Url,
    pub destination: PathBuf,
    pub expected_size: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct BurstAuthEndpoints {
    token_url: Url,
    authorize_url: Url,
    cookie_scope_url: Url,
}

impl BurstAuthEndpoints {
    pub fn new(
        token_url: impl AsRef<str>,
        authorize_url: impl AsRef<str>,
        cookie_scope_url: impl AsRef<str>,
    ) -> Result<Self, TransferRequestError> {
        let parse = |value: &str| {
            let url = Url::parse(value).map_err(|_| TransferRequestError::InvalidUrl)?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(TransferRequestError::InvalidUrl);
            }
            Ok(url)
        };
        Ok(Self {
            token_url: parse(token_url.as_ref())?,
            authorize_url: parse(authorize_url.as_ref())?,
            cookie_scope_url: parse(cookie_scope_url.as_ref())?,
        })
    }

    pub fn official() -> Self {
        Self::new(
            EARTHDATA_TOKEN_ENDPOINT,
            EARTHDATA_SESSION_ENDPOINT,
            EARTHDATA_SESSION_COOKIE_SCOPE,
        )
        .expect("official Burst authentication endpoints are valid")
    }
}

#[derive(Debug, Error)]
pub enum BurstSessionError {
    #[error("Sentinel-1 Burst requires an Earthdata account session cookie")]
    CookieRequired,
    #[error("Earthdata rejected the Burst account session")]
    CredentialRejected,
    #[error("Earthdata Burst session could not be established")]
    Remote,
}

impl BurstSessionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::CookieRequired => "ASF_BURST_COOKIE_REQUIRED",
            Self::CredentialRejected => "DL004",
            Self::Remote => "DL005",
        }
    }

    pub fn reason_zh(&self) -> &'static str {
        match self {
            Self::CookieRequired => {
                "Sentinel-1 Burst 下载需要账号密码或 .netrc 建立 asf-urs Cookie；仅 Token 不可用。"
            }
            Self::CredentialRejected => "Earthdata 拒绝了账号会话，请检查账号密码或 .netrc。",
            Self::Remote => "暂时无法建立 Earthdata Burst 下载会话，请稍后重试。",
        }
    }
}

pub enum TransferAuth {
    None,
    Bearer(SecretString),
    Basic {
        username: SecretString,
        password: SecretString,
    },
}

/// Secret-free result of an explicit, non-mutating Earthdata credential check.
/// Basic credentials are verified against EDL's read-only token-list endpoint.
/// Bearer tokens remain ReachableUnverified because the documented renew
/// endpoint invalidates the submitted token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarthdataAuthProbeOutcome {
    Valid,
    Invalid,
    ReachableUnverified,
    Unavailable,
}

pub struct SecretString(Zeroizing<String>);

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl SecretString {
    fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

/// Read the bearer value out of a `TransferAuth::Bearer` credential for
/// authenticated metadata search of restricted collections. Crate-private
/// secret discipline stays intact: only this crate's reviewed call sites may
/// use it, and the value must never be persisted or logged.
pub(crate) fn expose_transfer_auth_bearer(token: &SecretString) -> String {
    token.expose().trim().to_owned()
}

impl TransferAuth {
    /// Apply this credential to one reqwest blocking request builder. The
    /// secret values never cross into a log, a Store, or a URL; callers must
    /// not persist the returned builder. This exists for the native DEM tile
    /// path, which reuses the shared Earthdata credential on its own client.
    pub fn apply_to_blocking_request(
        &self,
        builder: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match self {
            Self::None => builder,
            Self::Bearer(token) => builder.bearer_auth(token.expose()),
            Self::Basic { username, password } => {
                builder.basic_auth(username.expose(), Some(password.expose()))
            }
        }
    }
}

impl fmt::Debug for TransferAuth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::None => "none",
            Self::Bearer(_) => "bearer",
            Self::Basic { .. } => "basic",
        };
        formatter
            .debug_struct("TransferAuth")
            .field("kind", &kind)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl TransferAuth {
    pub fn bearer(token: impl Into<String>) -> Result<Self, TransferRequestError> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(TransferRequestError::InvalidIdentity);
        }
        Ok(Self::Bearer(SecretString::new(token)))
    }

    pub fn basic(
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, TransferRequestError> {
        let username = username.into();
        let password = password.into();
        if username.trim().is_empty() || password.is_empty() {
            return Err(TransferRequestError::InvalidIdentity);
        }
        Ok(Self::Basic {
            username: SecretString::new(username),
            password: SecretString::new(password),
        })
    }

    pub(crate) fn shadow_auth_kind(&self) -> crate::ShadowAuthKind {
        match self {
            Self::None => crate::ShadowAuthKind::None,
            Self::Bearer(_) => crate::ShadowAuthKind::EarthdataBearer,
            Self::Basic { .. } => crate::ShadowAuthKind::EarthdataBasic,
        }
    }
}

impl fmt::Debug for TransferRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferRequest")
            .field("task_id", &self.task_id)
            .field("item_id", &self.item_id)
            .field("url", &"[REDACTED]")
            .field("destination", &self.destination)
            .field("expected_size", &self.expected_size)
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum TransferRequestError {
    #[error("下载请求标识无效")]
    InvalidIdentity,
    #[error("下载地址无效")]
    InvalidUrl,
    #[error("下载目标路径无效")]
    InvalidDestination,
    #[error("下载器配置无效")]
    InvalidOptions,
    #[error("无法初始化 HTTP 下载器")]
    ClientBuild,
}

impl TransferRequest {
    pub fn new(
        task_id: impl Into<String>,
        item_id: impl Into<String>,
        url: &str,
        destination: PathBuf,
        expected_size: Option<u64>,
    ) -> Result<Self, TransferRequestError> {
        let task_id = task_id.into();
        let item_id = item_id.into();
        if task_id.trim().is_empty() || item_id.trim().is_empty() {
            return Err(TransferRequestError::InvalidIdentity);
        }
        if destination.as_os_str().is_empty() {
            return Err(TransferRequestError::InvalidDestination);
        }
        let url = Url::parse(url).map_err(|_| TransferRequestError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(TransferRequestError::InvalidUrl);
        }
        Ok(Self {
            task_id,
            item_id,
            url,
            destination,
            expected_size,
        })
    }

    pub(crate) fn admission_origin(&self) -> Option<NormalizedOrigin> {
        NormalizedOrigin::from_url(&self.url)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TransferOptions {
    pub max_attempts: u32,
    pub backoff: Duration,
    pub timeout: Duration,
    pub chunk_size: usize,
    pub max_accepted_polls: u32,
    pub accepted_poll_timeout: Duration,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: Duration::from_secs(2),
            timeout: Duration::from_secs(60),
            chunk_size: 1024 * 1024,
            max_accepted_polls: 60,
            accepted_poll_timeout: Duration::from_secs(30 * 60),
        }
    }
}

pub const MAX_REDIRECT_HOPS: u8 = 10;
#[cfg(feature = "nisar-subset-production")]
pub(crate) const MAX_REQUESTS_PER_TRUSTED_REDIRECT_SEND: u64 = MAX_REDIRECT_HOPS as u64 + 1;

const EARTHDATA_TOKEN_ENDPOINT: &str =
    "https://urs.earthdata.nasa.gov/api/users/find_or_create_token";
const EARTHDATA_REACHABILITY_ENDPOINT: &str = "https://urs.earthdata.nasa.gov/";
const EARTHDATA_SESSION_ENDPOINT: &str = "https://urs.earthdata.nasa.gov/oauth/authorize?splash=false&client_id=BO_n7nTIlMljdvU6kRRB3g&response_type=code&redirect_uri=https://auth.asf.alaska.edu/login";
const EARTHDATA_SESSION_COOKIE_SCOPE: &str = "https://auth.asf.alaska.edu/";
const MAX_EARTHDATA_TOKEN_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug)]
struct ResolvedEarthdataSessionEndpoints {
    token_url: Url,
    authorize_url: Url,
    cookie_scope_url: Url,
}

impl ResolvedEarthdataSessionEndpoints {
    fn new(
        token_url: &str,
        authorize_url: &str,
        cookie_scope_url: &str,
    ) -> Result<Self, TransferRequestError> {
        let parse = |value: &str| {
            let url = Url::parse(value).map_err(|_| TransferRequestError::InvalidUrl)?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(TransferRequestError::InvalidUrl);
            }
            Ok(url)
        };
        Ok(Self {
            token_url: parse(token_url)?,
            authorize_url: parse(authorize_url)?,
            cookie_scope_url: parse(cookie_scope_url)?,
        })
    }

    fn official() -> Self {
        Self::new(
            EARTHDATA_TOKEN_ENDPOINT,
            EARTHDATA_SESSION_ENDPOINT,
            EARTHDATA_SESSION_COOKIE_SCOPE,
        )
        .expect("fixed Earthdata session endpoints are valid")
    }
}

#[derive(Debug)]
pub struct TransferAttemptLedger {
    max_attempts: u32,
    attempts_used: u32,
}

impl TransferAttemptLedger {
    pub fn new(max_attempts: u32) -> Result<Self, TransferRequestError> {
        if max_attempts == 0 {
            return Err(TransferRequestError::InvalidOptions);
        }
        Ok(Self {
            max_attempts,
            attempts_used: 0,
        })
    }

    pub fn try_claim(&mut self) -> Option<u32> {
        if self.attempts_used >= self.max_attempts {
            return None;
        }
        self.attempts_used += 1;
        Some(self.attempts_used)
    }

    pub const fn attempts_used(&self) -> u32 {
        self.attempts_used
    }

    pub const fn remaining(&self) -> u32 {
        self.max_attempts - self.attempts_used
    }

    /// An attempt that only set a refused partial aside made no progress; the
    /// transfer from zero keeps an attempt of its own. Bounded: a refusal needs
    /// a partial, and the transfer that writes a new one uses a normal attempt.
    fn allow_one_more(&mut self) {
        self.max_attempts = self.max_attempts.saturating_add(1);
    }
}

#[derive(Debug)]
pub struct RedirectBudget {
    hops_remaining: u8,
}

impl Default for RedirectBudget {
    fn default() -> Self {
        Self {
            hops_remaining: MAX_REDIRECT_HOPS,
        }
    }
}

impl RedirectBudget {
    pub fn try_claim(&mut self) -> bool {
        if self.hops_remaining == 0 {
            return false;
        }
        self.hops_remaining -= 1;
        true
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum AcceptedPollBudgetError {
    #[error("accepted-response poll limit exhausted")]
    PollLimit,
    #[error("accepted-response polling deadline exhausted")]
    Deadline,
}

#[derive(Debug)]
pub struct AcceptedPollBudget {
    polls_remaining: u32,
    deadline: Instant,
}

impl AcceptedPollBudget {
    pub fn new(
        max_polls: u32,
        timeout: Duration,
        started_at: Instant,
    ) -> Result<Self, TransferRequestError> {
        if max_polls == 0 || timeout.is_zero() {
            return Err(TransferRequestError::InvalidOptions);
        }
        let deadline = started_at
            .checked_add(timeout)
            .ok_or(TransferRequestError::InvalidOptions)?;
        Ok(Self {
            polls_remaining: max_polls,
            deadline,
        })
    }

    pub fn try_claim(&mut self, now: Instant) -> Result<(), AcceptedPollBudgetError> {
        if now >= self.deadline {
            return Err(AcceptedPollBudgetError::Deadline);
        }
        if self.polls_remaining == 0 {
            return Err(AcceptedPollBudgetError::PollLimit);
        }
        self.polls_remaining -= 1;
        Ok(())
    }

    fn check_deadline(&self, now: Instant) -> Result<(), AcceptedPollBudgetError> {
        if now >= self.deadline {
            Err(AcceptedPollBudgetError::Deadline)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransferProgress {
    pub bytes_completed: u64,
    pub total_bytes: Option<u64>,
    pub attempt: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransferUpdate {
    BodyStarted { attempt: u32, bytes_completed: u64 },
    Progress(TransferProgress),
    BodyStopped { attempt: u32 },
    StallReconnect { attempt: u32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransferResult {
    pub outcome: DownloadOutcome,
    pub bytes_written: u64,
    pub attempt_count: u32,
    pub failure: Option<DownloadFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseHostClass {
    Approved,
    Unapproved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthRejection {
    pub status_code: u16,
    pub response_host_class: ResponseHostClass,
    pub refresh_candidate: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedTransferResult {
    Finished(TransferResult),
    AuthRejected(AuthRejection),
}

use crate::resolved_part_checkpoint::{checkpoint_diagnostic, PartCheckpoint};

#[derive(Debug)]
pub struct ResolvedTransferSession {
    attempt_ledger: TransferAttemptLedger,
    prior_response_validator: Option<RemoteValidator>,
    part_predates_execution: Option<bool>,
    force_restart: bool,
    required_resume_offset: Option<u64>,
    identity: Option<ResolvedSessionIdentity>,
    attestation_identity: Arc<()>,
    recovery_identity: Option<String>,
    recovered_checkpoint: Option<PartCheckpoint>,
    checkpoint_checked: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct ResolvedSessionIdentity {
    task_id: String,
    item_id: String,
    destination: PathBuf,
    expected_size: Option<u64>,
    expected_checksum: Option<(String, String)>,
}

impl ResolvedTransferSession {
    pub fn new(max_attempts: u32) -> Result<Self, TransferRequestError> {
        Ok(Self {
            attempt_ledger: TransferAttemptLedger::new(max_attempts)?,
            prior_response_validator: None,
            part_predates_execution: None,
            force_restart: false,
            required_resume_offset: None,
            identity: None,
            attestation_identity: Arc::new(()),
            recovery_identity: None,
            recovered_checkpoint: None,
            checkpoint_checked: false,
        })
    }

    pub(crate) fn set_recovery_identity(&mut self, identity: String) {
        self.recovery_identity = Some(identity);
    }

    pub const fn attempts_used(&self) -> u32 {
        self.attempt_ledger.attempts_used()
    }

    pub const fn attempts_remaining(&self) -> u32 {
        self.attempt_ledger.remaining()
    }

    pub(crate) fn attestation_identity(&self) -> Arc<()> {
        Arc::clone(&self.attestation_identity)
    }

    pub(crate) fn matches_attestation_identity(&self, identity: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.attestation_identity, identity)
    }

    pub fn prior_response_validator(&self) -> Option<&RemoteValidator> {
        self.prior_response_validator.as_ref()
    }

    /// Test-only observation of the durable-evidence binding installed by the
    /// resolver runtime (B13 restart resume). Production never reads it here.
    #[doc(hidden)]
    pub fn recovery_identity(&self) -> Option<&str> {
        self.recovery_identity.as_deref()
    }

    /// Admit an externally checkpointed prefix while holding the destination lease.
    /// Both the old response and current resolved asset must bind the same
    /// validator — a strong ETag, or the Last-Modified plus exact total that
    /// `If-Range` itself accepts — and the exact on-disk prefix is hashed
    /// before admission. Ordinary recovered parts retain their conservative
    /// restart policy.
    #[allow(clippy::too_many_arguments)]
    pub fn resume_verified_part(
        &mut self,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        validator: &RemoteValidator,
        prefix_len: u64,
        prefix_sha256: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<(), DownloadFileError> {
        let invalid = || DownloadFileError::IntegrityMismatch;
        // The asset's expected size wins over the request's: the caller builds
        // the verification asset from the real response (remote total), while
        // the request may carry a frozen advisory size (USGS D806) that can
        // legitimately differ from the real object. For frozen-size routes the
        // two are equal, so this only fixes the remote-authoritative case.
        let expected = asset
            .expected_size()
            .or(request.expected_size)
            .ok_or_else(invalid)?;
        // A strong ETag binds the prefix; without one, a canonical
        // Last-Modified together with the exact total is the only other pair
        // `If-Range` may carry (RFC 9110 §13.1.5).
        let etag_valid = validator
            .etag
            .as_deref()
            .is_some_and(crate::resolved_part_checkpoint::strong_etag);
        let last_modified_valid = validator
            .last_modified
            .as_deref()
            .is_some_and(crate::resolved_part_checkpoint::canonical_http_date);
        let validator_valid =
            (etag_valid || last_modified_valid) && validator.content_length == Some(expected);
        // The current response must carry the same binding validator value.
        // Unrelated optional headers (for example a Last-Modified alongside a
        // matching ETag) must not turn an unchanged object into a rejection.
        let response = asset.remote_validator();
        let binding_matches = if etag_valid {
            response.etag.as_deref() == validator.etag.as_deref()
        } else {
            response.last_modified.as_deref() == validator.last_modified.as_deref()
        };
        // The request's size must match the asset's only when the asset does
        // not hold the remote-response size authority; an advisory request
        // size (USGS D806) may legitimately differ from the real object.
        let size_conflict = !asset.remote_response_size_is_authoritative()
            && request
                .expected_size
                .is_some_and(|size| asset.expected_size().is_some_and(|other| size != other));
        if self.identity.is_some()
            || self.attempts_used() != 0
            || request.url != *asset.effective_url()
            || size_conflict
            || prefix_len == 0
            || prefix_len >= expected
            || !validator_valid
            || response.content_length != Some(expected)
            || !binding_matches
            || prefix_sha256.len() != 64
            || !prefix_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        let path = part_path(&request.destination);
        let metadata = fs::symlink_metadata(&path).map_err(DownloadFileError::Filesystem)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != prefix_len
        {
            return Err(invalid());
        }
        let mut file = crate::file_core::open_checked_download_read(&path)?;
        let mut hasher = Sha256Accumulator::default();
        let mut bytes = 0u64;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            if cancelled() {
                return Err(DownloadFileError::RecoveryRequired);
            }
            let count = file
                .read(&mut buffer)
                .map_err(DownloadFileError::Filesystem)?;
            if count == 0 {
                break;
            }
            bytes = bytes.checked_add(count as u64).ok_or_else(invalid)?;
            if bytes > prefix_len {
                return Err(invalid());
            }
            hasher.update(&buffer[..count]);
        }
        if bytes != prefix_len
            || hasher.finalize_hex() != prefix_sha256
            || file
                .metadata()
                .map_err(DownloadFileError::Filesystem)?
                .len()
                != prefix_len
            || !self.bind_request(request, asset)
        {
            return Err(invalid());
        }
        self.prior_response_validator = Some(validator.clone());
        self.part_predates_execution = Some(false);
        self.force_restart = false;
        self.required_resume_offset = Some(prefix_len);
        Ok(())
    }

    pub fn apply_refresh_decision(&mut self, decision: RefreshDecision) {
        // A refreshed SAS URL alone cannot invalidate an identity-bound prefix.
        // Its actual response still must pass the strict ETag/206 gate before append.
        if self.recovered_checkpoint.is_some() {
            return;
        }
        match decision {
            RefreshDecision::RestartFromBeginning => {
                self.force_restart = true;
                self.prior_response_validator = None;
                self.required_resume_offset = None;
            }
            RefreshDecision::ResumeFrom { offset } => {
                self.force_restart = false;
                self.required_resume_offset = Some(offset);
            }
        }
    }

    fn bind_request(&mut self, request: &TransferRequest, asset: &ResolvedAsset) -> bool {
        let identity = ResolvedSessionIdentity {
            task_id: request.task_id.clone(),
            item_id: request.item_id.clone(),
            destination: request.destination.clone(),
            expected_size: request.expected_size.or(asset.expected_size()),
            expected_checksum: asset.expected_checksum().map(|checksum| {
                (
                    checksum.algorithm.to_ascii_lowercase(),
                    checksum.value.to_ascii_lowercase(),
                )
            }),
        };
        match self.identity.as_ref() {
            Some(bound) => bound == &identity,
            None => {
                self.identity = Some(identity);
                true
            }
        }
    }

    fn observe_initial_part(&mut self, destination: &Path) {
        self.part_predates_execution
            .get_or_insert_with(|| part_path(destination).is_file());
    }

    fn part_context(&self) -> ResolvedPartContext {
        if self.part_predates_execution == Some(true) {
            ResolvedPartContext::RecoveredAfterRestart
        } else {
            ResolvedPartContext::CurrentExecution
        }
    }

    fn mark_part_opened(&mut self) {
        self.recovered_checkpoint = None;
        self.part_predates_execution = Some(false);
        self.force_restart = false;
        self.required_resume_offset = None;
    }

    fn forget_set_aside_part(&mut self) {
        self.prior_response_validator = None;
        self.part_predates_execution = Some(false);
        self.force_restart = false;
        self.required_resume_offset = None;
        self.attempt_ledger.allow_one_more();
    }

    fn record_response_validator(
        &mut self,
        validator: RemoteValidator,
        diagnostic: IntegrityDiagnostic,
    ) -> Result<(), AttemptError> {
        if let Some(previous) = self.prior_response_validator.as_mut() {
            ensure_response_validator_compatible(previous, &validator, diagnostic)?;
            if previous.etag.is_none() {
                previous.etag = validator.etag;
            }
            if previous.last_modified.is_none() {
                previous.last_modified = validator.last_modified;
            }
            if previous.content_length.is_none() {
                previous.content_length = validator.content_length;
            }
        } else {
            self.prior_response_validator = Some(validator);
        }
        Ok(())
    }
}

fn classify_auth_rejection(
    policy: crate::AuthRefreshPolicyId,
    status_code: u16,
    response_host_class: ResponseHostClass,
    auth_kind: crate::asset_resolver::ResolvedAuthKind,
    www_authenticate: Option<&[u8]>,
) -> AuthRejection {
    let approved = response_host_class == ResponseHostClass::Approved;
    let refresh_candidate = approved
        && match (policy, status_code) {
            (crate::AuthRefreshPolicyId::EarthdataStandardV1, 401) => true,
            (crate::AuthRefreshPolicyId::EarthdataStandardV1, 403) => {
                auth_kind == crate::asset_resolver::ResolvedAuthKind::Bearer
                    && www_authenticate.is_some_and(is_bounded_bearer_invalid_token)
            }
            (crate::AuthRefreshPolicyId::CdseOAuthBearerV1, 401)
            | (crate::AuthRefreshPolicyId::BhoonidhiOAuthBearerV1, 401)
            | (crate::AuthRefreshPolicyId::EarthdataBurstSessionV1, 401) => {
                auth_kind == crate::asset_resolver::ResolvedAuthKind::Bearer
            }
            (crate::AuthRefreshPolicyId::CdseOAuthBearerV1, 403) => {
                auth_kind == crate::asset_resolver::ResolvedAuthKind::Bearer
                    && www_authenticate.is_some_and(is_bounded_bearer_invalid_token)
            }
            (crate::AuthRefreshPolicyId::UsgsM2mSignedUrlV1, 401 | 403) => {
                auth_kind == crate::asset_resolver::ResolvedAuthKind::None
            }
            (crate::AuthRefreshPolicyId::PlanetaryComputerSasV1, 401 | 403) => {
                auth_kind == crate::asset_resolver::ResolvedAuthKind::None
            }
            _ => false,
        };
    AuthRejection {
        status_code,
        response_host_class,
        refresh_candidate,
    }
}

fn is_bounded_bearer_invalid_token(value: &[u8]) -> bool {
    if value.len() > 1024 || !value.is_ascii() {
        return false;
    }
    let Ok(value) = std::str::from_utf8(value) else {
        return false;
    };
    let Some((scheme, parameters)) = value.trim().split_once(char::is_whitespace) else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return false;
    }
    parameters.split(',').any(|parameter| {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            return false;
        };
        name.trim().eq_ignore_ascii_case("error")
            && value
                .trim()
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                == Some("invalid_token")
    })
}

fn resolved_redirect_allowed(
    policy: AuthRefreshPolicyId,
    candidate_allowed_by_host_policy: bool,
) -> bool {
    // CDSE `$value`, Bhoonidhi and Planetary Computer signed access use
    // exact frozen locators and reject redirects before a second request is sent.
    // USGS Landsat broker URLs are different: the current service can redirect
    // within the exact landsatlook.usgs.gov HTTPS origin. That redirect is allowed
    // only when the resolver-frozen host policy independently approves the target.
    !matches!(
        policy,
        AuthRefreshPolicyId::CdseOAuthBearerV1
            | AuthRefreshPolicyId::BhoonidhiOAuthBearerV1
            | AuthRefreshPolicyId::PlanetaryComputerSasV1
    ) && candidate_allowed_by_host_policy
}

fn apply_resolved_asset_access(
    asset: &ResolvedAsset,
    initial_url: &Url,
    current_url: &Url,
    runtime_bearer: Option<&SecretString>,
    builder: RequestBuilder,
    uncredentialed_client: &Client,
) -> RequestBuilder {
    let carries_asset_access = current_url == initial_url
        || asset.host_policy().is_some_and(|policy| {
            policy.allows_authenticated_request_url(current_url)
                && auth_allowed_redirect(initial_url, current_url)
        });
    if !carries_asset_access {
        return uncredentialed_client.get(current_url.clone());
    }
    match runtime_bearer {
        Some(token) => asset.apply_request_access_with_bearer(builder, token.expose()),
        None => asset.apply_request_access(builder),
    }
}

const PART_BUF_WRITER_CAPACITY: usize = 8 * 1024 * 1024;
const DEFAULT_SEGMENT_THRESHOLD_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_STALL_MIN_BPS: u64 = 32 * 1024;
const DEFAULT_STALL_WINDOW: Duration = Duration::from_secs(15);
const DEFAULT_MAX_STALL_RECONNECTS: u32 = 12;

#[derive(Clone, Debug)]
struct SsdStagingOverride {
    enabled: bool,
    auto_slow_disk: bool,
    staging_dir: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct TransferEngineRuntimeConfig {
    per_file_segments_override: Option<u8>,
    segment_threshold_bytes: u64,
    min_split_chunk_bytes: u64,
    segmented_resume_overlap_bytes: u64,
    single_stream_resume_overlap_bytes: u64,
    slow_shed_enabled: bool,
    relative_speed_config: RelativeSpeedConfig,
    stall_reconnect_override: Option<bool>,
    stall_min_bps: u64,
    stall_window: Duration,
    max_stall_reconnects: u32,
    ssd_staging_override: Option<SsdStagingOverride>,
    slow_target_override: Option<bool>,
}

impl Default for TransferEngineRuntimeConfig {
    fn default() -> Self {
        Self {
            per_file_segments_override: None,
            segment_threshold_bytes: DEFAULT_SEGMENT_THRESHOLD_BYTES,
            min_split_chunk_bytes: DEFAULT_MIN_SPLIT_CHUNK_BYTES,
            segmented_resume_overlap_bytes: DEFAULT_RESUME_OVERLAP_BYTES,
            // Single-stream default stays 0 so existing exact-offset Range probes preserve prior behavior;
            // segmented resumes verify 64-byte continuity by default and callers/tests may opt in to
            // single-stream overlap checks explicitly via `with_single_stream_resume_overlap_bytes`.
            single_stream_resume_overlap_bytes: 0,
            slow_shed_enabled: true,
            relative_speed_config: RelativeSpeedConfig::default(),
            stall_reconnect_override: None,
            stall_min_bps: DEFAULT_STALL_MIN_BPS,
            stall_window: DEFAULT_STALL_WINDOW,
            max_stall_reconnects: DEFAULT_MAX_STALL_RECONNECTS,
            ssd_staging_override: None,
            slow_target_override: None,
        }
    }
}

pub struct HttpTransfer {
    client: Client,
    range_client: OnceLock<Client>,
    uncredentialed_client: OnceLock<Client>,
    cookie_jar: Arc<Jar>,
    resolved_earthdata_session_endpoints: ResolvedEarthdataSessionEndpoints,
    /// One Earthdata session (bearer plus the ASF cookie in `cookie_jar`) shared
    /// by every transfer of this client. Establishment is single-flight through
    /// `establishing`; the lock is never held across a network request, so a
    /// cancelled transfer stops waiting. The generation lets a rejection retire
    /// only the session it was sent with.
    earthdata_session: std::sync::Mutex<EarthdataSessionCache>,
    earthdata_session_ready: std::sync::Condvar,
    options: TransferOptions,
    #[cfg(test)]
    checkpoint_interval_bytes: Option<u64>,
    proxy_policy: ProxyPolicy,
    /// Downloader D3: the SHA-256 each streaming landing computed, keyed by
    /// destination, until the worker takes it for the landing ledger.
    landed_sha256: std::sync::Mutex<std::collections::HashMap<PathBuf, String>>,
    engine_config: TransferEngineRuntimeConfig,
    staging_free_space_probe: Arc<dyn FreeSpaceProbe>,
    bandwidth_limiter_override: Option<Arc<BandwidthLimiter>>,
    host_concurrency_override: Option<Arc<HostConcurrencyTracker>>,
    resume_prefix_hash_skipped_count: AtomicU64,
    stall_reconnect_count: AtomicU64,
    segmented_transfers_count: AtomicU64,
    ssd_staging_promotions_count: AtomicU64,
    dynamic_segment_splits_count: AtomicU64,
    slow_connection_shed_count: AtomicU64,
    resume_overlap_verified_count: AtomicU64,
}

#[derive(Default)]
struct EarthdataSessionCache {
    token: Option<SecretString>,
    generation: u64,
    establishing: bool,
    /// Bumped when every session is retired (credential change); a session
    /// requested under an older epoch is discarded.
    epoch: u64,
}

/// Bounded wait slice so a waiting transfer notices its cancellation.
const EARTHDATA_SESSION_WAIT_SLICE: Duration = Duration::from_millis(50);

/// Clears `establishing` and wakes waiters however the establishment ends.
struct EarthdataEstablishment<'a>(&'a HttpTransfer);

impl Drop for EarthdataEstablishment<'_> {
    fn drop(&mut self) {
        self.0.lock_earthdata_session().establishing = false;
        self.0.earthdata_session_ready.notify_all();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExactRangeRedirectEvidence {
    pub(crate) response_status: Option<u16>,
    pub(crate) redirect_occurred: bool,
    pub(crate) redirect_hop_index: Option<u8>,
    pub(crate) source_host_class: String,
    pub(crate) destination_host_class: String,
    pub(crate) source_path_class: String,
    pub(crate) destination_path_class: String,
    pub(crate) same_origin: bool,
    pub(crate) scheme_transition: String,
    pub(crate) policy_result: String,
    pub(crate) rejection_reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExactRangeTransportCause {
    RequestTimeout,
    RequestConnect,
    /// The server certificate could not be verified (see
    /// `request_error_is_tls_certificate`); no longer reported as a connect
    /// failure (L5 recheck 2026-10-19).
    TlsCertificate,
    RequestBody,
    RequestOther,
    RetryableHttpStatus,
}

impl ExactRangeTransportCause {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::RequestTimeout => "request_timeout",
            Self::RequestConnect => "request_connect",
            Self::TlsCertificate => "tls_certificate",
            Self::RequestBody => "request_body",
            Self::RequestOther => "request_other",
            Self::RetryableHttpStatus => "retryable_http_status",
        }
    }
}

/// The user-facing cause of a TLS certificate verification failure.
pub(crate) const TLS_CERTIFICATE_FAILURE_ZH: &str =
    "无法验证服务器证书（本机证书库无法读取或网络被拦截）";

/// How rustls describes a certificate it could not verify
/// (`Error::InvalidCertificate`, for example "invalid peer certificate:
/// UnknownIssuer").
const TLS_CERTIFICATE_ERROR_PREFIX: &str = "invalid peer certificate";

/// Whether a failed request could not verify the server certificate: rustls
/// reported an invalid peer certificate somewhere in the error's source chain.
/// L5 recheck 2026-10-19: the program ran as a low-integrity process (its
/// directory carried an explicit Low integrity label), which cannot open the
/// current user's certificate store; no native root certificate was loaded and
/// every NISAR Range request failed with UnknownIssuer while being reported as
/// request_connect.
pub(crate) fn request_error_is_tls_certificate(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = current {
        if error.to_string().starts_with(TLS_CERTIFICATE_ERROR_PREFIX) {
            return true;
        }
        current = error.source();
    }
    false
}

/// The same test on an already sanitized cause chain.
pub(crate) fn error_chain_names_tls_certificate(chain: &[String]) -> bool {
    chain
        .iter()
        .any(|cause| cause.starts_with(TLS_CERTIFICATE_ERROR_PREFIX))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExactRangeTransportDiagnostic {
    pub(crate) cause: ExactRangeTransportCause,
    pub(crate) http_status: Option<u16>,
}

impl ExactRangeTransportDiagnostic {
    fn from_request_error(error: &reqwest::Error) -> Self {
        let cause = if error.is_timeout() {
            ExactRangeTransportCause::RequestTimeout
        } else if request_error_is_tls_certificate(error) {
            ExactRangeTransportCause::TlsCertificate
        } else if error.is_connect() {
            ExactRangeTransportCause::RequestConnect
        } else if error.is_body() {
            ExactRangeTransportCause::RequestBody
        } else {
            ExactRangeTransportCause::RequestOther
        };
        Self {
            cause,
            http_status: None,
        }
    }

    const fn retryable_http_status(status: u16) -> Self {
        Self {
            cause: ExactRangeTransportCause::RetryableHttpStatus,
            http_status: Some(status),
        }
    }

    const fn defensive_fallback() -> Self {
        Self {
            cause: ExactRangeTransportCause::RequestOther,
            http_status: None,
        }
    }
}

pub(crate) enum ExactRangeSendOutcome {
    Response {
        response: Response,
        request_count: u64,
    },
    Cancelled {
        request_count: u64,
    },
    RetryableExhausted {
        request_count: u64,
        diagnostic: ExactRangeTransportDiagnostic,
    },
    ProtocolRejected {
        request_count: u64,
    },
    TrustedRedirectRejected {
        request_count: u64,
        evidence: ExactRangeRedirectEvidence,
    },
    #[cfg(feature = "nisar-subset-production")]
    BudgetExceeded {
        request_count: u64,
    },
}

fn classify_earthdata_basic_probe_status(status: StatusCode) -> EarthdataAuthProbeOutcome {
    if status.is_success() {
        EarthdataAuthProbeOutcome::Valid
    } else if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        EarthdataAuthProbeOutcome::Invalid
    } else if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        EarthdataAuthProbeOutcome::Unavailable
    } else {
        EarthdataAuthProbeOutcome::ReachableUnverified
    }
}

fn decode_base64url_bytes(segment: &str) -> Option<Vec<u8>> {
    let val = |b: u8| -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'-' | b'+' => Some(62),
            b'_' | b'/' => Some(63),
            _ => None,
        }
    };
    let clean = segment.trim_end_matches('=');
    let mut out = Vec::with_capacity((clean.len() * 3) / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for &b in clean.as_bytes() {
        let v = val(b)?;
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1u32 << bits) - 1;
        }
    }
    Some(out)
}

fn decode_earthdata_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let _sig = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let bytes = decode_base64url_bytes(payload_b64)?;
    serde_json::from_slice(&bytes).ok()
}

impl HttpTransfer {
    pub fn new(options: TransferOptions) -> Result<Self, TransferRequestError> {
        Self::new_with_proxy_policy(options, ProxyPolicy::System)
    }

    /// Construct the existing transfer state machine with an explicit route
    /// policy. The policy governs only system-proxy use; it does not change
    /// redirect, host, TLS, or authorization behavior.
    pub fn new_with_proxy_policy(
        options: TransferOptions,
        proxy_policy: ProxyPolicy,
    ) -> Result<Self, TransferRequestError> {
        Self::build(options, proxy_policy)
    }

    /// Construct the existing transfer state machine with system proxies disabled.
    ///
    /// This crate-private constructor is reserved for an explicitly opted-in
    /// acceptance harness; normal download construction retains its current
    /// system-proxy behavior.
    #[cfg(test)]
    pub(crate) fn new_without_system_proxy(
        options: TransferOptions,
    ) -> Result<Self, TransferRequestError> {
        Self::new_with_proxy_policy(options, ProxyPolicy::Direct)
    }

    pub const fn proxy_policy(&self) -> ProxyPolicy {
        self.proxy_policy
    }

    /// Contact Earthdata Login without creating, renewing, or revoking a token.
    /// No response body or credential material crosses this boundary.
    pub fn probe_earthdata_auth(&self, auth: &TransferAuth) -> EarthdataAuthProbeOutcome {
        let endpoint_origin_path = |path: &str| -> Url {
            let mut url = self
                .resolved_earthdata_session_endpoints
                .authorize_url
                .clone();
            url.set_path(path);
            url.set_query(None);
            url.set_fragment(None);
            url
        };
        match auth {
            TransferAuth::Basic { username, password } => {
                let url = endpoint_origin_path("/api/users/tokens");
                let response = self
                    .client
                    .get(url)
                    .basic_auth(username.expose(), Some(password.expose()))
                    .send();
                match response {
                    Ok(response) => classify_earthdata_basic_probe_status(response.status()),
                    Err(_) => EarthdataAuthProbeOutcome::Unavailable,
                }
            }
            TransferAuth::Bearer(token) => {
                let raw_token = token.expose().trim();
                if raw_token.is_empty() {
                    return EarthdataAuthProbeOutcome::Invalid;
                }
                let mut uid_candidate: Option<String> = None;
                if let Some(claims) = decode_earthdata_jwt_payload(raw_token) {
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    if let Some(exp) = claims.get("exp").and_then(|v| {
                        v.as_u64()
                            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
                    }) {
                        if exp <= now_secs {
                            return EarthdataAuthProbeOutcome::Invalid;
                        }
                    }
                    if let Some(uid) = claims.get("uid").and_then(serde_json::Value::as_str) {
                        let uid = uid.trim();
                        if !uid.is_empty()
                            && uid.len() <= 64
                            && uid.bytes().all(|b| {
                                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
                            })
                        {
                            uid_candidate = Some(uid.to_owned());
                        }
                    }
                }
                if let Some(uid) = uid_candidate {
                    let profile_url = endpoint_origin_path(&format!("/api/users/{uid}"));
                    match self.client.get(profile_url).bearer_auth(raw_token).send() {
                        Ok(resp) if resp.status().is_success() => {
                            return EarthdataAuthProbeOutcome::Valid;
                        }
                        Ok(resp)
                            if matches!(
                                resp.status(),
                                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                            ) =>
                        {
                            return EarthdataAuthProbeOutcome::Invalid;
                        }
                        Ok(resp)
                            if resp.status().is_server_error()
                                || resp.status() == StatusCode::TOO_MANY_REQUESTS =>
                        {
                            return EarthdataAuthProbeOutcome::Unavailable;
                        }
                        Ok(_) => {}
                        Err(_) => return EarthdataAuthProbeOutcome::Unavailable,
                    }
                }
                let auth_url = self
                    .resolved_earthdata_session_endpoints
                    .authorize_url
                    .clone();
                let response = self
                    .client
                    .get(auth_url.clone())
                    .bearer_auth(raw_token)
                    .send();
                match response {
                    Ok(resp) => {
                        let status = resp.status();
                        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                            return EarthdataAuthProbeOutcome::Invalid;
                        }
                        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                            return EarthdataAuthProbeOutcome::Unavailable;
                        }
                        if status.is_redirection() {
                            if let Some(loc) =
                                resp.headers().get(LOCATION).and_then(|v| v.to_str().ok())
                            {
                                if let Ok(next) = auth_url.join(loc) {
                                    let expected_host = self
                                        .resolved_earthdata_session_endpoints
                                        .cookie_scope_url
                                        .host_str();
                                    let has_code = next.query_pairs().any(|(k, _)| k == "code");
                                    if has_code || expected_host == next.host_str() {
                                        return EarthdataAuthProbeOutcome::Valid;
                                    }
                                    if next.path().starts_with("/login")
                                        || next.path().starts_with("/oauth/authorize")
                                        || next.query_pairs().any(|(k, _)| k == "error")
                                    {
                                        return EarthdataAuthProbeOutcome::Invalid;
                                    }
                                }
                            }
                            return EarthdataAuthProbeOutcome::ReachableUnverified;
                        }
                        if status.is_success() {
                            let is_html = resp
                                .headers()
                                .get(reqwest::header::CONTENT_TYPE)
                                .and_then(|v| v.to_str().ok())
                                .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("text/html"));
                            return if is_html {
                                EarthdataAuthProbeOutcome::ReachableUnverified
                            } else {
                                EarthdataAuthProbeOutcome::Valid
                            };
                        }
                        EarthdataAuthProbeOutcome::ReachableUnverified
                    }
                    Err(_) => EarthdataAuthProbeOutcome::Unavailable,
                }
            }
            TransferAuth::None => {
                let response = self
                    .uncredentialed_client()
                    .get(EARTHDATA_REACHABILITY_ENDPOINT)
                    .send();
                match response {
                    Ok(response)
                        if !response.status().is_server_error()
                            && response.status() != StatusCode::TOO_MANY_REQUESTS =>
                    {
                        EarthdataAuthProbeOutcome::ReachableUnverified
                    }
                    Ok(_) | Err(_) => EarthdataAuthProbeOutcome::Unavailable,
                }
            }
        }
    }

    fn build(
        options: TransferOptions,
        proxy_policy: ProxyPolicy,
    ) -> Result<Self, TransferRequestError> {
        if options.max_attempts == 0
            || options.chunk_size == 0
            || options.timeout.is_zero()
            || options.max_accepted_polls == 0
            || options.accepted_poll_timeout.is_zero()
            || Instant::now()
                .checked_add(options.accepted_poll_timeout)
                .is_none()
        {
            return Err(TransferRequestError::InvalidOptions);
        }
        let cookie_jar = Arc::new(Jar::default());
        let client = Client::builder()
            .redirect(Policy::none())
            .cookie_provider(cookie_jar.clone())
            .timeout(options.timeout)
            .connect_timeout(options.timeout.min(Duration::from_secs(30)));
        let client =
            insar_provider::network_proxy::blocking_builder_with_proxy(client, proxy_policy)
                .build()
                .map_err(|_| TransferRequestError::ClientBuild)?;
        let engine_config = TransferEngineRuntimeConfig::default();
        Ok(Self {
            client,
            range_client: OnceLock::new(),
            uncredentialed_client: OnceLock::new(),
            cookie_jar,
            resolved_earthdata_session_endpoints: ResolvedEarthdataSessionEndpoints::official(),
            earthdata_session: std::sync::Mutex::new(EarthdataSessionCache::default()),
            earthdata_session_ready: std::sync::Condvar::new(),
            options,
            #[cfg(test)]
            checkpoint_interval_bytes: None,
            proxy_policy,
            landed_sha256: std::sync::Mutex::new(std::collections::HashMap::new()),
            engine_config,
            staging_free_space_probe: default_free_space_probe(),
            bandwidth_limiter_override: None,
            host_concurrency_override: None,
            resume_prefix_hash_skipped_count: AtomicU64::new(0),
            stall_reconnect_count: AtomicU64::new(0),
            segmented_transfers_count: AtomicU64::new(0),
            ssd_staging_promotions_count: AtomicU64::new(0),
            dynamic_segment_splits_count: AtomicU64::new(0),
            slow_connection_shed_count: AtomicU64::new(0),
            resume_overlap_verified_count: AtomicU64::new(0),
        })
    }

    fn range_client(&self) -> &Client {
        self.range_client.get_or_init(|| {
            let range_client = Client::builder()
                .redirect(Policy::none())
                // Earthdata/ASF cumulus objects resolve through an EDL OAuth 302
                // chain that issues an `asf-urs` session cookie; the Range client
                // must share the same cookie jar so the redirect chain converges.
                .cookie_provider(self.cookie_jar.clone())
                // reqwest blocking timeouts apply independently to connect, send,
                // and each body read. Continuous progress therefore resets this
                // idle-I/O bound; it is not a total deadline for the Range body.
                // Earthdata/ASF EDL OAuth redirects and Bhoonidhi proxy hops can
                // spend several seconds waiting on response headers, so allow up
                // to 30s per phase/read while still bounded by `self.options.timeout`.
                .timeout(self.options.timeout.min(Duration::from_secs(30)))
                .connect_timeout(self.options.timeout.min(Duration::from_secs(30)));
            insar_provider::network_proxy::blocking_builder_with_proxy(
                range_client,
                self.proxy_policy,
            )
            .build()
            .unwrap_or_else(|_| self.client.clone())
        })
    }

    fn uncredentialed_client(&self) -> &Client {
        self.uncredentialed_client.get_or_init(|| {
            let uncredentialed_client = Client::builder()
                .redirect(Policy::none())
                .timeout(self.options.timeout)
                .connect_timeout(self.options.timeout.min(Duration::from_secs(30)));
            insar_provider::network_proxy::blocking_builder_with_proxy(
                uncredentialed_client,
                self.proxy_policy,
            )
            .build()
            .unwrap_or_else(|_| self.client.clone())
        })
    }

    /// Overrides the per-file HTTP Range connection count (`1`, `2`, `4`, or `8`).
    pub fn with_per_file_segments(mut self, segments: u8) -> Self {
        self.engine_config.per_file_segments_override = Some(normalize_per_file_segments(segments));
        self
    }

    /// Overrides the minimum known file size threshold at which multi-segment Range kicks in.
    pub fn with_segment_threshold_bytes(mut self, threshold_bytes: u64) -> Self {
        self.engine_config.segment_threshold_bytes = threshold_bytes.max(1);
        self
    }

    /// Configures the sliding-window low-speed stall watchdog (`min_bps` over `window`).
    pub fn with_stall_watchdog(
        mut self,
        enabled: bool,
        min_bps: u64,
        window: Duration,
        max_reconnects: u32,
    ) -> Self {
        self.engine_config.stall_reconnect_override = Some(enabled);
        self.engine_config.stall_min_bps = min_bps.max(1);
        self.engine_config.stall_window = window.max(Duration::from_millis(25));
        self.engine_config.max_stall_reconnects = max_reconnects;
        self
    }

    /// Overrides SSD staging preferences for this [`HttpTransfer`] instance.
    pub fn with_ssd_staging(
        mut self,
        enabled: bool,
        auto_slow_disk: bool,
        staging_dir: Option<PathBuf>,
    ) -> Self {
        self.engine_config.ssd_staging_override = Some(SsdStagingOverride {
            enabled,
            auto_slow_disk,
            staging_dir,
        });
        self
    }

    /// Overrides destination volume slowness detection for deterministic testing.
    pub fn with_slow_target_volume_override(mut self, slow: Option<bool>) -> Self {
        self.engine_config.slow_target_override = slow;
        self
    }

    /// Overrides the free-space probe used when validating `ssd_staging_dir` headroom.
    pub fn with_staging_free_space_probe(mut self, probe: Arc<dyn FreeSpaceProbe>) -> Self {
        self.staging_free_space_probe = probe;
        self
    }

    /// Overrides the minimum per-side chunk size used when dynamically bisecting slow tail segments.
    pub fn with_min_split_chunk_bytes(mut self, min_split_chunk_bytes: u64) -> Self {
        self.engine_config.min_split_chunk_bytes = min_split_chunk_bytes.max(1);
        self
    }

    /// Overrides the byte overlap re-read on resumed segmented HTTP Range requests (`0` disables).
    pub fn with_segmented_resume_overlap_bytes(mut self, overlap_bytes: u64) -> Self {
        self.engine_config.segmented_resume_overlap_bytes = overlap_bytes.min(4096);
        self
    }

    /// Overrides the byte overlap re-read on single-stream HTTP Range resumes (`0` default).
    pub fn with_single_stream_resume_overlap_bytes(mut self, overlap_bytes: u64) -> Self {
        self.engine_config.single_stream_resume_overlap_bytes = overlap_bytes.min(4096);
        self
    }

    /// Configures relative slow-connection shedding across concurrent segment workers.
    pub fn with_slow_connection_shedding(
        mut self,
        enabled: bool,
        config: RelativeSpeedConfig,
    ) -> Self {
        self.engine_config.slow_shed_enabled = enabled;
        self.engine_config.relative_speed_config = config;
        self
    }

    /// Overrides the shared bandwidth limiter used by this [`HttpTransfer`] instance.
    pub fn with_bandwidth_limiter(mut self, limiter: Arc<BandwidthLimiter>) -> Self {
        self.bandwidth_limiter_override = Some(limiter);
        self
    }

    /// Overrides the per-host learned concurrency tracker used by this [`HttpTransfer`] instance.
    pub fn with_host_concurrency_tracker(mut self, tracker: Arc<HostConcurrencyTracker>) -> Self {
        self.host_concurrency_override = Some(tracker);
        self
    }

    /// Number of single-stream `.part` resumes that safely skipped prefix SHA-256 re-hashing.
    pub fn resume_prefix_hash_skipped_count(&self) -> u64 {
        self.resume_prefix_hash_skipped_count
            .load(Ordering::Relaxed)
    }

    /// Number of low-speed stall reconnects performed across this transfer client.
    pub fn stall_reconnect_count(&self) -> u64 {
        self.stall_reconnect_count.load(Ordering::Relaxed)
    }

    /// Number of transfers completed via multi-connection HTTP Range segmentation.
    pub fn segmented_transfers_count(&self) -> u64 {
        self.segmented_transfers_count.load(Ordering::Relaxed)
    }

    /// Number of verified `.part` files sequentially promoted out of SSD staging.
    pub fn ssd_staging_promotions_count(&self) -> u64 {
        self.ssd_staging_promotions_count.load(Ordering::Relaxed)
    }

    /// Number of dynamic work-stealing segment bisections performed across segmented transfers.
    pub fn dynamic_segment_splits_count(&self) -> u64 {
        self.dynamic_segment_splits_count.load(Ordering::Relaxed)
    }

    /// Number of relatively slow segment connections shed and re-acquired cleanly.
    pub fn slow_connection_shed_count(&self) -> u64 {
        self.slow_connection_shed_count.load(Ordering::Relaxed)
    }

    /// Number of resume responses verified via overlapping local prefix byte comparison.
    pub fn resume_overlap_verified_count(&self) -> u64 {
        self.resume_overlap_verified_count.load(Ordering::Relaxed)
    }

    fn effective_bandwidth_limiter(&self) -> &Arc<BandwidthLimiter> {
        self.bandwidth_limiter_override
            .as_ref()
            .unwrap_or_else(|| global_bandwidth_limiter())
    }

    fn effective_host_concurrency(&self) -> &Arc<HostConcurrencyTracker> {
        self.host_concurrency_override
            .as_ref()
            .unwrap_or_else(|| global_host_concurrency_tracker())
    }

    fn bhoonidhi_aware_retry_after(
        url: &Url,
        status: StatusCode,
        headers: &reqwest::header::HeaderMap,
    ) -> Option<Duration> {
        let explicit = headers
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(crate::transient_retry::parse_retry_after_header);
        if explicit.is_some() {
            return explicit;
        }
        if status == StatusCode::TOO_MANY_REQUESTS
            && crate::asset_resolver::bhoonidhi::is_bhoonidhi_download_host(url)
        {
            return crate::asset_resolver::bhoonidhi::parse_rate_limit_wait_from_headers(
                status.as_u16(),
                headers,
            );
        }
        None
    }

    fn resolve_segment_policy(
        auth_refresh_policy: Option<AuthRefreshPolicyId>,
        url: &Url,
    ) -> (&'static str, MaximumSegmentPolicy) {
        let is_bhoonidhi = auth_refresh_policy == Some(AuthRefreshPolicyId::BhoonidhiOAuthBearerV1)
            || url
                .host_str()
                .is_some_and(|host| host.to_ascii_lowercase().contains("bhoonidhi"));
        if is_bhoonidhi {
            (
                "isro_bhoonidhi",
                MaximumSegmentPolicy::AtMost(SegmentLevel::One),
            )
        } else {
            ("", MaximumSegmentPolicy::Unbounded)
        }
    }

    fn effective_per_file_segments(&self) -> u8 {
        self.effective_per_file_segments_for_provider("")
    }

    fn effective_per_file_segments_for_provider(&self, provider_id: &str) -> u8 {
        normalize_per_file_segments(
            self.engine_config
                .per_file_segments_override
                .unwrap_or_else(|| {
                    active_transfer_engine_prefs().effective_segments_for_provider(provider_id)
                }),
        )
    }

    fn effective_stall_enabled(&self) -> bool {
        self.engine_config
            .stall_reconnect_override
            .unwrap_or_else(|| active_transfer_engine_prefs().stall_reconnect_enabled)
    }

    fn active_streaming_client(&self) -> &Client {
        &self.client
    }

    fn resolve_ssd_staging_dir(&self, destination: &Path) -> Option<PathBuf> {
        let (enabled, auto_slow, dir) = match &self.engine_config.ssd_staging_override {
            Some(override_cfg) => (
                override_cfg.enabled,
                override_cfg.auto_slow_disk,
                override_cfg.staging_dir.clone(),
            ),
            None => {
                let prefs = active_transfer_engine_prefs();
                (
                    prefs.ssd_staging_enabled,
                    prefs.ssd_staging_auto_slow_disk,
                    prefs
                        .staging_dir_path()
                        .or_else(crate::transfer_engine_prefs::active_default_ssd_staging_dir),
                )
            }
        };
        let staging_dir = dir?;
        let slow_disk = self
            .engine_config
            .slow_target_override
            .unwrap_or_else(|| is_slow_target_volume(destination));
        (enabled || (auto_slow && slow_disk)).then_some(staging_dir)
    }

    /// Downloader D3: takes the SHA-256 the last streaming landing at
    /// `destination` computed while writing it, if any.
    pub(crate) fn take_landed_sha256(&self, destination: &Path) -> Option<String> {
        self.landed_sha256.lock().ok()?.remove(destination)
    }

    fn lock_earthdata_session(&self) -> std::sync::MutexGuard<'_, EarthdataSessionCache> {
        self.earthdata_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Return the shared Earthdata session, establishing it first if none is
    /// held. Concurrent callers wait for one establishment instead of each
    /// requesting a token; a waiter leaves as soon as it is cancelled.
    fn ensure_earthdata_session(
        &self,
        authorize: &dyn Fn(RequestBuilder) -> RequestBuilder,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<(SecretString, u64), DownloadFailure> {
        let mut cache = self.lock_earthdata_session();
        loop {
            if let Some(token) = &cache.token {
                return Ok((
                    SecretString::new(token.expose().to_owned()),
                    cache.generation,
                ));
            }
            if cancelled() {
                return Err(earthdata_session_cancelled_failure());
            }
            if !cache.establishing {
                break;
            }
            cache = self
                .earthdata_session_ready
                .wait_timeout(cache, EARTHDATA_SESSION_WAIT_SLICE)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        cache.establishing = true;
        let epoch = cache.epoch;
        drop(cache);
        let establishment = EarthdataEstablishment(self);
        let token = self.establish_earthdata_session(authorize, cancelled)?;
        let mut cache = self.lock_earthdata_session();
        if cache.epoch != epoch {
            // The saved credential changed while this request was in flight.
            drop(cache);
            return Err(earthdata_session_retired_failure());
        }
        cache.generation = cache.generation.wrapping_add(1);
        let shared = SecretString::new(token.expose().to_owned());
        cache.token = Some(token);
        let generation = cache.generation;
        drop(cache);
        drop(establishment);
        Ok((shared, generation))
    }

    /// Retire the session a rejected request carried; a newer session that
    /// another transfer already established is kept.
    fn invalidate_earthdata_session(&self, generation: u64) {
        let mut cache = self.lock_earthdata_session();
        if cache.generation == generation {
            cache.token = None;
        }
    }

    /// Retire every Earthdata session of this client, e.g. after the saved
    /// credential changed; the next transfer establishes a new one.
    pub fn invalidate_all_earthdata_sessions(&self) {
        let mut cache = self.lock_earthdata_session();
        cache.token = None;
        cache.epoch = cache.epoch.wrapping_add(1);
    }

    /// Whether a shared Earthdata session is currently held. Reads no
    /// credential and makes no request.
    pub fn has_earthdata_session(&self) -> bool {
        self.lock_earthdata_session().token.is_some()
    }

    /// The shared Earthdata session token, when one is currently held. Reads
    /// no credential and makes no request; the returned value is the opaque
    /// bearer token authenticated metadata search of restricted collections
    /// needs. Callers must never persist or log it.
    pub fn earthdata_session_token(&self) -> Option<String> {
        self.lock_earthdata_session()
            .token
            .as_ref()
            .map(|token| token.expose().to_owned())
    }

    /// Establish the shared Earthdata session from a saved Basic credential
    /// before any task is admitted, so the first transfer reuses it.
    pub fn ensure_earthdata_session_with_credential(
        &self,
        auth: &TransferAuth,
    ) -> Result<(), DownloadFailure> {
        let TransferAuth::Basic { username, password } = auth else {
            return Err(earthdata_session_credential_failure());
        };
        let authorize = |builder: RequestBuilder| {
            builder.basic_auth(username.expose(), Some(password.expose()))
        };
        self.ensure_earthdata_session(&authorize, &|| false)
            .map(|_| ())
    }

    fn establish_earthdata_session(
        &self,
        authorize: &dyn Fn(RequestBuilder) -> RequestBuilder,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<SecretString, DownloadFailure> {
        let endpoints = &self.resolved_earthdata_session_endpoints;
        if cancelled() {
            return Err(earthdata_session_cancelled_failure());
        }

        let token_response = authorize(self.client.post(endpoints.token_url.clone()))
            .send()
            .map_err(|err| earthdata_session_transport_failure_for_reqwest(&err))?;
        let token_status = token_response.status();
        if matches!(
            token_status,
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(earthdata_session_credential_failure());
        }
        if !token_status.is_success() {
            return Err(earthdata_session_http_failure(token_status));
        }
        let mut token_body = Vec::new();
        token_response
            .take(MAX_EARTHDATA_TOKEN_RESPONSE_BYTES as u64)
            .read_to_end(&mut token_body)
            .map_err(|err| earthdata_session_transport_failure_for_io(&err))?;
        let token = serde_json::from_slice::<serde_json::Value>(&token_body)
            .ok()
            .and_then(|value| {
                value
                    .get("access_token")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .map(|value| value.trim().to_owned())
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= MAX_EARTHDATA_TOKEN_RESPONSE_BYTES
                    && !value.chars().any(char::is_control)
            })
            .ok_or_else(earthdata_session_invalid_failure)?;

        if cancelled() {
            return Err(DownloadFailure {
                code: "DL001".to_string(),
                reason_zh: "Earthdata 会话建立在传输开始前被取消。".to_string(),
                retryable: true,
            });
        }

        let initial = endpoints.authorize_url.clone();
        let mut current = initial.clone();
        let mut redirect_budget = RedirectBudget::default();
        loop {
            if cancelled() {
                return Err(DownloadFailure {
                    code: "DL001".to_string(),
                    reason_zh: "Earthdata 会话建立在传输开始前被取消。".to_string(),
                    retryable: true,
                });
            }
            let mut builder = self.client.get(current.clone());
            if same_origin(&initial, &current) {
                builder = authorize(builder);
            }
            let response = builder
                .send()
                .map_err(|err| earthdata_session_transport_failure_for_reqwest(&err))?;
            let status = response.status();
            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                return Err(earthdata_session_credential_failure());
            }
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(earthdata_session_redirect_failure)?;
                if !redirect_budget.try_claim() {
                    return Err(earthdata_session_redirect_failure());
                }
                let next = current
                    .join(location)
                    .map_err(|_| earthdata_session_redirect_failure())?;
                if next == current
                    || !redirect_scheme_allowed(&initial, &next)
                    || !auth_allowed_redirect(&initial, &next)
                {
                    return Err(earthdata_session_redirect_failure());
                }
                current = next;
                continue;
            }
            if !status.is_success() {
                return Err(earthdata_session_http_failure(status));
            }
            break;
        }

        let has_asf_cookie = self
            .cookie_jar
            .cookies(&endpoints.cookie_scope_url)
            .and_then(|value| value.to_str().ok().map(str::to_owned))
            .is_some_and(|value| {
                value.split(';').any(|cookie| {
                    cookie
                        .trim()
                        .split_once('=')
                        .is_some_and(|(name, content)| {
                            name == "asf-urs" && !content.trim().is_empty()
                        })
                })
            });
        if !has_asf_cookie {
            return Err(earthdata_session_credential_failure());
        }
        Ok(SecretString::new(token))
    }

    pub(crate) fn send_resolved_exact_range(
        &self,
        asset: &ResolvedAsset,
        start: u64,
        end_exclusive: u64,
        if_range: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ExactRangeSendOutcome {
        self.send_resolved_exact_range_inner(
            asset,
            start,
            end_exclusive,
            Some(if_range),
            cancelled,
            self.options.max_attempts,
            true,
            None,
            false,
        )
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn send_resolved_exact_range_once_trusted_redirect(
        &self,
        asset: &ResolvedAsset,
        start: u64,
        end_exclusive: u64,
        if_range: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ExactRangeSendOutcome {
        self.send_resolved_exact_range_inner(
            asset,
            start,
            end_exclusive,
            Some(if_range),
            cancelled,
            1,
            true,
            None,
            true,
        )
    }

    #[cfg(feature = "nisar-subset-production")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn send_resolved_exact_range_once_with_request_gate(
        &self,
        asset: &ResolvedAsset,
        start: u64,
        end_exclusive: u64,
        if_range: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
        reserve_request: &dyn Fn() -> bool,
    ) -> ExactRangeSendOutcome {
        self.send_resolved_exact_range_inner(
            asset,
            start,
            end_exclusive,
            Some(if_range),
            cancelled,
            1,
            true,
            Some(reserve_request),
            true,
        )
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn send_resolved_preflight_range_once_trusted_redirect(
        &self,
        asset: &ResolvedAsset,
        start: u64,
        end_exclusive: u64,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ExactRangeSendOutcome {
        self.send_resolved_exact_range_inner(
            asset,
            start,
            end_exclusive,
            None,
            cancelled,
            1,
            true,
            None,
            true,
        )
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn send_resolved_preflight_range(
        &self,
        asset: &ResolvedAsset,
        start: u64,
        end_exclusive: u64,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> ExactRangeSendOutcome {
        self.send_resolved_exact_range_inner(
            asset,
            start,
            end_exclusive,
            None,
            cancelled,
            self.options.max_attempts,
            true,
            None,
            false,
        )
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn send_resolved_exact_range_with_nisar_budget(
        &self,
        asset: &ResolvedAsset,
        start: u64,
        end_exclusive: u64,
        if_range: &str,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &crate::hdf5_range_transfer::NisarLiveNetworkBudgetV1,
    ) -> ExactRangeSendOutcome {
        self.send_resolved_exact_range_inner(
            asset,
            start,
            end_exclusive,
            Some(if_range),
            cancelled,
            self.options.max_attempts,
            true,
            Some(&|| budget.reserve_request().is_ok()),
            false,
        )
    }

    #[cfg(feature = "nisar-subset-production")]
    pub(crate) fn send_resolved_preflight_range_with_nisar_budget(
        &self,
        asset: &ResolvedAsset,
        start: u64,
        end_exclusive: u64,
        cancelled: &(dyn Fn() -> bool + Sync),
        budget: &crate::hdf5_range_transfer::NisarLiveNetworkBudgetV1,
    ) -> ExactRangeSendOutcome {
        self.send_resolved_exact_range_inner(
            asset,
            start,
            end_exclusive,
            None,
            cancelled,
            self.options.max_attempts,
            true,
            Some(&|| budget.reserve_request().is_ok()),
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn send_resolved_exact_range_inner(
        &self,
        asset: &ResolvedAsset,
        start: u64,
        end_exclusive: u64,
        if_range: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
        maximum_attempts: u32,
        allow_redirects: bool,
        #[cfg(feature = "nisar-subset-production")] request_budget: Option<&dyn Fn() -> bool>,
        #[cfg(not(feature = "nisar-subset-production"))] _request_budget: Option<()>,
        capture_trusted_redirect_evidence: bool,
    ) -> ExactRangeSendOutcome {
        if asset.method() != ResolvedMethod::Get
            || start >= end_exclusive
            || if_range.is_some_and(str::is_empty)
            || !range_url_allowed(asset, asset.effective_url())
        {
            return ExactRangeSendOutcome::ProtocolRejected { request_count: 0 };
        }
        let initial = asset.effective_url().clone();
        let mut request_count = 0_u64;
        for attempt in 1..=maximum_attempts {
            if cancelled() {
                return ExactRangeSendOutcome::Cancelled { request_count };
            }
            let mut current = initial.clone();
            let mut redirect_budget = RedirectBudget::default();
            let mut redirect_hop_index = 0_u8;
            let response = loop {
                if cancelled() {
                    return ExactRangeSendOutcome::Cancelled { request_count };
                }
                if crate::asset_resolver::bhoonidhi::wait_for_bhoonidhi_rate_limit_window(
                    &current, cancelled,
                ) {
                    return ExactRangeSendOutcome::Cancelled { request_count };
                }
                let carries_asset_access = same_origin(&initial, &current)
                    || (asset
                        .host_policy()
                        .is_some_and(|policy| policy.allows_authenticated_request_url(&current))
                        && auth_allowed_redirect(&initial, &current));
                let request_client = if carries_asset_access {
                    self.range_client()
                } else {
                    self.uncredentialed_client()
                };
                let mut builder = request_client
                    .get(current.clone())
                    .header(RANGE, format!("bytes={start}-{}", end_exclusive - 1))
                    .header(ACCEPT_ENCODING, "identity");
                if let Some(if_range) = if_range {
                    builder = builder.header(IF_RANGE, if_range);
                }
                if carries_asset_access {
                    builder = asset.apply_request_access(builder);
                }
                let Some(next_request_count) = request_count.checked_add(1) else {
                    return ExactRangeSendOutcome::ProtocolRejected { request_count };
                };
                #[cfg(feature = "nisar-subset-production")]
                if let Some(budget) = request_budget {
                    if !budget() {
                        return ExactRangeSendOutcome::BudgetExceeded { request_count };
                    }
                }
                request_count = next_request_count;
                let response = match builder.send() {
                    Ok(response) => {
                        crate::asset_resolver::bhoonidhi::record_bhoonidhi_rate_limit_headers(
                            response.url(),
                            response.status().as_u16(),
                            response.headers(),
                        );
                        response
                    }
                    Err(_) if attempt < maximum_attempts => break None,
                    Err(error) => {
                        return ExactRangeSendOutcome::RetryableExhausted {
                            request_count,
                            diagnostic: ExactRangeTransportDiagnostic::from_request_error(&error),
                        };
                    }
                };
                if !response.status().is_redirection() {
                    break Some(response);
                }
                let response_status = response.status();
                if !allow_redirects {
                    let evidence = trusted_redirect_evidence(
                        &initial,
                        Some(&current),
                        Some(response_status),
                        Some(redirect_hop_index.saturating_add(1)),
                        "rejected",
                        "redirects_disabled",
                    );
                    return if capture_trusted_redirect_evidence {
                        ExactRangeSendOutcome::TrustedRedirectRejected {
                            request_count,
                            evidence,
                        }
                    } else {
                        ExactRangeSendOutcome::ProtocolRejected { request_count }
                    };
                }
                let Some(location) = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                else {
                    let evidence = trusted_redirect_evidence(
                        &initial,
                        Some(&current),
                        Some(response_status),
                        Some(redirect_hop_index.saturating_add(1)),
                        "rejected",
                        "missing_or_invalid_location",
                    );
                    return if capture_trusted_redirect_evidence {
                        ExactRangeSendOutcome::TrustedRedirectRejected {
                            request_count,
                            evidence,
                        }
                    } else {
                        ExactRangeSendOutcome::ProtocolRejected { request_count }
                    };
                };
                if !redirect_budget.try_claim() {
                    let evidence = trusted_redirect_evidence(
                        &initial,
                        Some(&current),
                        Some(response_status),
                        Some(redirect_hop_index.saturating_add(1)),
                        "rejected",
                        "hop_budget_exhausted",
                    );
                    return if capture_trusted_redirect_evidence {
                        ExactRangeSendOutcome::TrustedRedirectRejected {
                            request_count,
                            evidence,
                        }
                    } else {
                        ExactRangeSendOutcome::ProtocolRejected { request_count }
                    };
                }
                redirect_hop_index = redirect_hop_index.saturating_add(1);
                let Ok(candidate) = current.join(location) else {
                    let evidence = trusted_redirect_evidence(
                        &initial,
                        None,
                        Some(response_status),
                        Some(redirect_hop_index),
                        "rejected",
                        "malformed_location",
                    );
                    return if capture_trusted_redirect_evidence {
                        ExactRangeSendOutcome::TrustedRedirectRejected {
                            request_count,
                            evidence,
                        }
                    } else {
                        ExactRangeSendOutcome::ProtocolRejected { request_count }
                    };
                };
                let policy_allowed = asset.host_policy().map_or_else(
                    || {
                        crate::redirect_policy::default_redirect_target_allowed(
                            &initial, &candidate,
                        )
                    },
                    |policy| policy.allows_redirect(&initial, &candidate),
                );
                let scheme_allowed = redirect_scheme_allowed(&initial, &candidate);
                let range_allowed = range_url_allowed(asset, &candidate);
                let rejection_reason = if candidate == current {
                    "redirect_loop"
                } else if !scheme_allowed {
                    "scheme_transition_rejected"
                } else if !range_allowed || !policy_allowed {
                    "destination_policy_rejected"
                } else {
                    "redirect_accepted"
                };
                if candidate == current || !range_allowed || !scheme_allowed || !policy_allowed {
                    let evidence = trusted_redirect_evidence(
                        &initial,
                        Some(&candidate),
                        Some(response_status),
                        Some(redirect_hop_index),
                        "rejected",
                        rejection_reason,
                    );
                    return if capture_trusted_redirect_evidence {
                        ExactRangeSendOutcome::TrustedRedirectRejected {
                            request_count,
                            evidence,
                        }
                    } else {
                        ExactRangeSendOutcome::ProtocolRejected { request_count }
                    };
                }
                current = candidate;
            };
            if let Some(response) = response {
                let retryable = response.status() == StatusCode::TOO_MANY_REQUESTS
                    || response.status().is_server_error();
                if !retryable {
                    return ExactRangeSendOutcome::Response {
                        response,
                        request_count,
                    };
                }
                if attempt == maximum_attempts {
                    return ExactRangeSendOutcome::RetryableExhausted {
                        request_count,
                        diagnostic: ExactRangeTransportDiagnostic::retryable_http_status(
                            response.status().as_u16(),
                        ),
                    };
                }
            }
            if wait_backoff(self.options.backoff, cancelled) {
                return ExactRangeSendOutcome::Cancelled { request_count };
            }
        }
        ExactRangeSendOutcome::RetryableExhausted {
            request_count,
            diagnostic: ExactRangeTransportDiagnostic::defensive_fallback(),
        }
    }

    fn send_with_controls<F, R>(
        &self,
        initial_url: &Url,
        resume_from: u64,
        if_range: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
        apply_access: F,
        redirect_allowed: R,
    ) -> Result<Option<Response>, AttemptError>
    where
        F: FnMut(RequestBuilder, &Url) -> RequestBuilder,
        R: FnMut(&Url, &Url) -> bool,
    {
        let range_header = (resume_from > 0).then(|| format!("bytes={resume_from}-"));
        self.send_with_range_header(
            self.active_streaming_client(),
            initial_url,
            range_header.as_deref(),
            if_range,
            cancelled,
            apply_access,
            redirect_allowed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn send_with_range_header<F, R>(
        &self,
        client: &Client,
        initial_url: &Url,
        range_header: Option<&str>,
        if_range: Option<&str>,
        cancelled: &(dyn Fn() -> bool + Sync),
        mut apply_access: F,
        mut redirect_allowed: R,
    ) -> Result<Option<Response>, AttemptError>
    where
        F: FnMut(RequestBuilder, &Url) -> RequestBuilder,
        R: FnMut(&Url, &Url) -> bool,
    {
        let mut current_url = initial_url.clone();
        let mut redirect_budget = RedirectBudget::default();
        let mut left_initial_url = false;
        let mut returned_to_initial_url = false;
        let mut accepted_poll_budget = AcceptedPollBudget::new(
            self.options.max_accepted_polls,
            self.options.accepted_poll_timeout,
            Instant::now(),
        )
        .map_err(|_| AttemptError::bounded())?;
        loop {
            if cancelled() {
                return Ok(None);
            }
            if crate::asset_resolver::bhoonidhi::wait_for_bhoonidhi_rate_limit_window(
                &current_url,
                cancelled,
            ) {
                return Ok(None);
            }
            let mut builder = apply_access(client.get(current_url.clone()), &current_url);
            if let Some(range) = range_header {
                builder = builder
                    .header(RANGE, range)
                    .header(ACCEPT_ENCODING, "identity");
                if let Some(if_range) = if_range {
                    builder = builder.header(IF_RANGE, if_range);
                }
            }
            let response = builder.send().map_err(AttemptError::transport)?;
            crate::asset_resolver::bhoonidhi::record_bhoonidhi_rate_limit_headers(
                response.url(),
                response.status().as_u16(),
                response.headers(),
            );
            if response.status() == StatusCode::ACCEPTED {
                accepted_poll_budget
                    .try_claim(Instant::now())
                    .map_err(|_| AttemptError::bounded())?;
                let delay = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .map(|seconds| Duration::from_secs(seconds.min(300)))
                    .unwrap_or(self.options.backoff);
                drop(response);
                if wait_backoff(delay, cancelled) {
                    return Ok(None);
                }
                accepted_poll_budget
                    .check_deadline(Instant::now())
                    .map_err(|_| AttemptError::bounded())?;
                continue;
            }
            if !response.status().is_redirection() {
                return Ok(Some(response));
            }
            let Some(location) = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
            else {
                return Ok(Some(response));
            };
            if !redirect_budget.try_claim() {
                return Err(AttemptError::bounded());
            }
            let candidate = current_url
                .join(location)
                .map_err(|_| AttemptError::redirect_rejected(initial_url, None))?;
            let returns_to_initial = candidate == *initial_url;
            let one_oauth_return = returns_to_initial
                && current_url != *initial_url
                && left_initial_url
                && !returned_to_initial_url;
            if candidate == current_url
                || !redirect_scheme_allowed(initial_url, &candidate)
                || !redirect_allowed(initial_url, &candidate)
                || (returns_to_initial && !one_oauth_return)
            {
                return Err(AttemptError::redirect_rejected(
                    initial_url,
                    Some(&candidate),
                ));
            }
            if returns_to_initial {
                returned_to_initial_url = true;
            } else {
                left_initial_url = true;
            }
            current_url = candidate;
        }
    }

    pub fn resolved_session(&self) -> ResolvedTransferSession {
        ResolvedTransferSession::new(self.options.max_attempts)
            .expect("validated transfer options always create a resolved session")
    }

    #[cfg(test)]
    pub(crate) fn set_resolved_earthdata_session_endpoints_for_test(
        &mut self,
        token_url: &str,
        authorize_url: &str,
        cookie_scope_url: &str,
    ) -> Result<(), TransferRequestError> {
        self.resolved_earthdata_session_endpoints =
            ResolvedEarthdataSessionEndpoints::new(token_url, authorize_url, cookie_scope_url)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn download_resolved(
        &self,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        auth_refresh_policy: AuthRefreshPolicyId,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        let result = self.download_resolved_inner(
            request,
            asset,
            auth_refresh_policy,
            None,
            session,
            cancelled,
            paused,
            progress,
        );
        self.clear_completed_checkpoint(request, session, result)
    }

    fn clear_completed_checkpoint(
        &self,
        request: &TransferRequest,
        session: &ResolvedTransferSession,
        result: ResolvedTransferResult,
    ) -> ResolvedTransferResult {
        if matches!(&result, ResolvedTransferResult::Finished(value)
            if matches!(value.outcome, DownloadOutcome::Completed | DownloadOutcome::Skipped))
        {
            if let Some(binding) = &session.recovery_identity {
                if PartCheckpoint::clear(&request.destination, binding).is_err() {
                    checkpoint_diagnostic(
                        "checkpoint_clear_failed",
                        "\"reason\":\"completion_cleanup_failed\"",
                    );
                }
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    pub fn download_resolved_validated(
        &self,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        auth_refresh_policy: AuthRefreshPolicyId,
        validation_plan: &ArtifactValidationPlan,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        let result = self.download_resolved_inner(
            request,
            asset,
            auth_refresh_policy,
            Some(validation_plan),
            session,
            cancelled,
            paused,
            progress,
        );
        self.clear_completed_checkpoint(request, session, result)
    }

    #[allow(clippy::too_many_arguments)]
    fn download_resolved_inner(
        &self,
        request: &TransferRequest,
        asset: &ResolvedAsset,
        auth_refresh_policy: AuthRefreshPolicyId,
        validation_plan: Option<&ArtifactValidationPlan>,
        session: &mut ResolvedTransferSession,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> ResolvedTransferResult {
        let validation = CompletedArtifactValidationInvocation {
            registered: validation_plan.map(|plan| ArtifactValidationInvocation { plan, asset }),
            burst: BurstArtifactValidationKind::from_policy(auth_refresh_policy, asset),
        };
        if asset.method() != ResolvedMethod::Get
            || request.url != *asset.effective_url()
            || matches!(
                (request.expected_size, asset.expected_size()),
                (Some(left), Some(right)) if left != right
            )
        {
            return ResolvedTransferResult::Finished(resolved_failure(
                &request.destination,
                session.attempts_used(),
                "DL005",
                "解析后的下载资产与冻结任务不一致，未开始传输。",
                false,
            ));
        }

        let checksum = match asset.expected_checksum() {
            Some(checksum) => match ExpectedChecksum::new(&checksum.algorithm, &checksum.value) {
                Ok(checksum) => Some(checksum),
                Err(_) => {
                    let diagnostic = IntegrityDiagnostic {
                        subtype: IntegritySubtype::ResolvedChecksumInvalid,
                        response_status: IntegrityResponseStatusClass::Other,
                        resume: IntegrityResumeState::Fresh,
                        remote_total: IntegrityRemoteTotalPresence::Unknown,
                        size_relation: IntegritySizeRelation::Unknown,
                    };
                    let reason =
                        integrity_reason("解析后的下载校验信息无效，未生成最终文件。", diagnostic);
                    return ResolvedTransferResult::Finished(resolved_failure(
                        &request.destination,
                        session.attempts_used(),
                        "DL002",
                        &reason,
                        false,
                    ));
                }
            },
            None => None,
        };
        let expected_size = if resolved_size_is_remote_authoritative(
            auth_refresh_policy,
            asset,
            validation.burst,
        ) {
            None
        } else {
            request.expected_size.or(asset.expected_size())
        };

        // Fast skip when final destination is already complete and valid before staging setup.
        if let Ok(FilePreparation::SkipComplete { bytes }) = inspect_resolved_destination(
            &request.destination,
            expected_size,
            checksum.as_ref(),
            session.part_context(),
        ) {
            if let Err(error) = validate_completed_artifact(validation, &request.destination) {
                return ResolvedTransferResult::Finished(artifact_validation_failure(
                    bytes,
                    session.attempts_used(),
                    &error,
                ));
            }
            return ResolvedTransferResult::Finished(success(
                DownloadOutcome::Skipped,
                bytes,
                session.attempts_used(),
            ));
        }

        let effective_request_owned: Option<TransferRequest> =
            if let Some(staging_dir) = self.resolve_ssd_staging_dir(&request.destination) {
                let staged_dest = staging_target_path(&staging_dir, &request.destination);
                let required_bytes = request
                    .expected_size
                    .or(asset.expected_size())
                    .unwrap_or(0)
                    .saturating_sub(part_allocated_size(&staged_dest));
                if let Err(error) = check_ssd_staging_space(
                    &*self.staging_free_space_probe,
                    &staging_dir,
                    required_bytes,
                ) {
                    return ResolvedTransferResult::Finished(file_failure(
                        error,
                        session.attempts_used(),
                    ));
                }
                let mut cloned = request.clone();
                cloned.destination = staged_dest;
                Some(cloned)
            } else {
                None
            };
        let effective_request = effective_request_owned.as_ref().unwrap_or(request);

        // Promotion may have completed immediately before a crash, leaving a
        // checkpoint but no part. Let the normal final-file validation run
        // before clearing that bound record; never treat the record as proof.
        let promoted_without_part = matches!(
            fs::symlink_metadata(part_path(&effective_request.destination)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ) && fs::symlink_metadata(&request.destination).is_ok();
        if !promoted_without_part
            && !session.checkpoint_checked
            && session.identity.is_none()
            && asset.expected_checksum().is_none()
        {
            session.checkpoint_checked = true;
            if let Some(binding) = &session.recovery_identity {
                // For a remote-authoritative size (USGS D806 advisory
                // filesize), the saved total is validated against the real
                // response instead: the exact 206 Content-Range gate in
                // matches_response_range. The frozen advisory must not be
                // compared here (live B13 2026-10-31: advisory 957,283,399 vs
                // real 957,306,880 rejected a valid checkpoint).
                let expected_for_checkpoint = if resolved_size_is_remote_authoritative(
                    auth_refresh_policy,
                    asset,
                    validation.burst,
                ) {
                    None
                } else {
                    request.expected_size.or(asset.expected_size())
                };
                match PartCheckpoint::load(
                    &effective_request.destination,
                    binding,
                    expected_for_checkpoint,
                    cancelled,
                ) {
                    Ok(Some(saved)) => {
                        session.prior_response_validator = Some(saved.validator());
                        session.required_resume_offset = Some(saved.bytes);
                        session.recovered_checkpoint = Some(saved);
                    }
                    Ok(None) => {}
                    Err(_unprovable) => {
                        // An unprovable checkpoint is not authority to fail
                        // the USGS archive task: both the `.part` and its
                        // record are quarantined under a conflict name (never
                        // deleted, D3 owner rule 2026-10-18) and the transfer
                        // restarts cleanly from zero within the normal attempt
                        // budget. The MPC route keeps its verified fail-closed
                        // contract (issue #20); only an unquarantinable prefix
                        // fails closed here, because a fresh start would
                        // truncate surviving evidence.
                        if auth_refresh_policy != AuthRefreshPolicyId::UsgsM2mSignedUrlV1
                            || !checkpoint_quarantine_unprovable(&effective_request.destination)
                        {
                            return ResolvedTransferResult::Finished(file_failure(
                                DownloadFileError::RecoveryRequired,
                                session.attempts_used(),
                            ));
                        }
                    }
                }
            }
        }
        if !session.bind_request(request, asset) {
            let diagnostic = IntegrityDiagnostic {
                subtype: IntegritySubtype::ResolvedIdentityMismatch,
                response_status: IntegrityResponseStatusClass::Other,
                resume: IntegrityResumeState::Fresh,
                remote_total: IntegrityRemoteTotalPresence::Unknown,
                size_relation: IntegritySizeRelation::Unknown,
            };
            let reason = integrity_reason(
                "认证刷新后的任务或完整性身份发生变化，未继续传输。",
                diagnostic,
            );
            return ResolvedTransferResult::Finished(resolved_failure(
                &request.destination,
                session.attempts_used(),
                "DL002",
                &reason,
                false,
            ));
        }
        session.observe_initial_part(&effective_request.destination);
        let preparation = match inspect_resolved_for_session(
            session,
            &effective_request.destination,
            expected_size,
            checksum.as_ref(),
        ) {
            Ok(preparation) => preparation,
            Err(error) => {
                return ResolvedTransferResult::Finished(file_failure(
                    error,
                    session.attempts_used(),
                ));
            }
        };
        match preparation {
            FilePreparation::SkipComplete { bytes } => {
                if let Err(error) =
                    validate_completed_artifact(validation, &effective_request.destination)
                {
                    return ResolvedTransferResult::Finished(artifact_validation_failure(
                        bytes,
                        session.attempts_used(),
                        &error,
                    ));
                }
                return ResolvedTransferResult::Finished(success(
                    DownloadOutcome::Skipped,
                    bytes,
                    session.attempts_used(),
                ));
            }
            FilePreparation::FinalizePart { bytes } => {
                if let Err(error) = validate_completed_artifact(
                    validation,
                    &part_path(&effective_request.destination),
                ) {
                    if error.clears_part() {
                        remove_part(&effective_request.destination);
                    }
                    return ResolvedTransferResult::Finished(artifact_validation_failure(
                        bytes,
                        session.attempts_used(),
                        &error,
                    ));
                }
                let fin_result = if effective_request.destination != request.destination {
                    verify_and_hash_part(
                        &part_path(&effective_request.destination),
                        checksum.as_ref(),
                        expected_size,
                        IntegrityResponseStatusClass::Other,
                        IntegrityResumeState::Resume,
                        expected_size,
                        cancelled,
                    )
                    .and_then(|sha| {
                        self.finalize_and_promote(
                            &effective_request.destination,
                            &request.destination,
                            expected_size,
                            validation,
                            sha,
                            cancelled,
                        )
                    })
                    .map_err(|e| e.failure)
                } else {
                    finalize_part(&request.destination, expected_size)
                        .map(|_| ())
                        .map_err(|e| DownloadFailure {
                            code: e.code().to_string(),
                            reason_zh: e.reason_zh().to_string(),
                            retryable: false,
                        })
                };
                return ResolvedTransferResult::Finished(match fin_result {
                    Ok(()) => success(DownloadOutcome::Completed, bytes, session.attempts_used()),
                    Err(failure) => TransferResult {
                        outcome: DownloadOutcome::Failed,
                        bytes_written: 0,
                        attempt_count: session.attempts_used(),
                        failure: Some(failure),
                    },
                });
            }
            FilePreparation::Resume { .. } | FilePreparation::Restart => {}
        }
        if cancelled() {
            return ResolvedTransferResult::Finished(interrupted(
                &effective_request.destination,
                session.attempts_used(),
            ));
        }

        // The provider allows at most three simultaneous downloads per user/IP.
        // Acquire the process-wide permit only after local completion checks and
        // keep it alive through the complete response-body transfer.
        let _bhoonidhi_download_permit =
            if auth_refresh_policy == AuthRefreshPolicyId::BhoonidhiOAuthBearerV1 {
                match try_acquire_bhoonidhi_download_slot() {
                    Ok(permit) => Some(permit),
                    Err(BhoonidhiDownloadProbeError::Status(412)) => {
                        return ResolvedTransferResult::Finished(resolved_failure(
                            &request.destination,
                            session.attempts_used(),
                            "BHOONIDHI_CONCURRENCY_LIMIT",
                            "Bhoonidhi 当前同时下载数已达上限，请稍后重试。",
                            false,
                        ));
                    }
                    Err(_) => {
                        return ResolvedTransferResult::Finished(resolved_failure(
                            &request.destination,
                            session.attempts_used(),
                            "BHOONIDHI_TRANSFER_ADMISSION_FAILED",
                            "Bhoonidhi 下载并发闸门不可用，未开始传输。",
                            false,
                        ));
                    }
                }
            } else {
                None
            };

        let runtime_bearer = if asset.requires_earthdata_session() {
            let authorize = |builder: RequestBuilder| asset.apply_request_access(builder);
            match self.ensure_earthdata_session(&authorize, cancelled) {
                Ok(session) => Some(session),
                Err(failure) => {
                    let failure = if auth_refresh_policy
                        == AuthRefreshPolicyId::EarthdataBurstSessionV1
                        && failure.code == "DL004"
                    {
                        earthdata_session_cookie_required_failure()
                    } else {
                        failure
                    };
                    return ResolvedTransferResult::Finished(resolved_failure(
                        &request.destination,
                        session.attempts_used(),
                        &failure.code,
                        &failure.reason_zh,
                        failure.retryable,
                    ));
                }
            }
        } else {
            None
        };

        let mut last_failure = transient_failure();
        while let Some(attempt) = session.attempt_ledger.try_claim() {
            if cancelled() {
                return ResolvedTransferResult::Finished(interrupted(
                    &effective_request.destination,
                    attempt - 1,
                ));
            }
            let next_retry_after = match self.attempt_resolved(
                effective_request,
                &request.destination,
                asset,
                runtime_bearer.as_ref().map(|(token, _)| token),
                auth_refresh_policy,
                checksum.as_ref(),
                validation,
                session,
                attempt,
                cancelled,
                paused,
                progress,
            ) {
                Ok(result) => {
                    if let (ResolvedTransferResult::AuthRejected(_), Some((_, generation))) =
                        (&result, &runtime_bearer)
                    {
                        // The caller's single refresh then establishes one new
                        // session for every transfer instead of reusing this one.
                        self.invalidate_earthdata_session(*generation);
                    }
                    return self.clear_completed_checkpoint(effective_request, session, result);
                }
                Err(error) => {
                    last_failure = error.failure;
                    if !apply_part_action(&effective_request.destination, error.part) {
                        return ResolvedTransferResult::Finished(part_set_aside_failure(
                            &effective_request.destination,
                            session.attempts_used(),
                        ));
                    }
                    if error.part == PartAction::SetAside {
                        session.forget_set_aside_part();
                    }
                    if !error.retryable || session.attempts_remaining() == 0 {
                        return ResolvedTransferResult::Finished(TransferResult {
                            outcome: DownloadOutcome::Failed,
                            bytes_written: part_size(&effective_request.destination),
                            attempt_count: session.attempts_used(),
                            failure: Some(last_failure),
                        });
                    }
                    error.retry_after_hint
                }
            };
            let base_delay = self.options.backoff.saturating_mul(attempt);
            let delay = next_retry_after.map_or(base_delay, |hint| base_delay.max(hint));
            if wait_backoff(delay, cancelled) {
                return ResolvedTransferResult::Finished(interrupted(
                    &effective_request.destination,
                    session.attempts_used(),
                ));
            }
        }
        ResolvedTransferResult::Finished(TransferResult {
            outcome: DownloadOutcome::Failed,
            bytes_written: part_size(&effective_request.destination),
            attempt_count: session.attempts_used(),
            failure: Some(last_failure),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn attempt_resolved(
        &self,
        request: &TransferRequest,
        final_destination: &Path,
        asset: &ResolvedAsset,
        runtime_bearer: Option<&SecretString>,
        auth_refresh_policy: AuthRefreshPolicyId,
        checksum: Option<&ExpectedChecksum>,
        validation: CompletedArtifactValidationInvocation<'_>,
        session: &mut ResolvedTransferSession,
        attempt: u32,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> Result<ResolvedTransferResult, AttemptError> {
        let expected_size = if resolved_size_is_remote_authoritative(
            auth_refresh_policy,
            asset,
            validation.burst,
        ) {
            None
        } else {
            request.expected_size.or(asset.expected_size())
        };
        let initial_url = asset.effective_url().clone();
        let classify_rejection = |resp: &Response| {
            let response_host_class = if asset
                .host_policy()
                .is_some_and(|policy| policy.allows_response_url(resp.url()))
            {
                ResponseHostClass::Approved
            } else {
                ResponseHostClass::Unapproved
            };
            let www_authenticate = resp
                .headers()
                .get(WWW_AUTHENTICATE)
                .map(|value| value.as_bytes());
            ResolvedTransferResult::AuthRejected(classify_auth_rejection(
                auth_refresh_policy,
                resp.status().as_u16(),
                response_host_class,
                runtime_bearer.map_or_else(
                    || asset.auth_kind(),
                    |_| crate::asset_resolver::ResolvedAuthKind::Bearer,
                ),
                www_authenticate,
            ))
        };

        let mut stall_reconnects: u32 = 0;
        let mut last_stall_bytes: u64 = 0;
        let mut zero_progress_stalls: u32 = 0;
        let mut is_stall_retry = false;

        loop {
            let preparation = inspect_resolved_for_session(
                session,
                &request.destination,
                expected_size,
                checksum,
            )
            .map_err(AttemptError::file)?;
            let mut resume_from = match preparation {
                FilePreparation::SkipComplete { bytes } => {
                    validate_completed_artifact(validation, &request.destination)
                        .map_err(AttemptError::artifact_validation)?;
                    return Ok(ResolvedTransferResult::Finished(success(
                        DownloadOutcome::Skipped,
                        bytes,
                        attempt - 1,
                    )));
                }
                FilePreparation::FinalizePart { bytes } => {
                    validate_completed_artifact(validation, &part_path(&request.destination))
                        .map_err(AttemptError::artifact_validation)?;
                    if request.destination != final_destination {
                        let sha = verify_and_hash_part(
                            &part_path(&request.destination),
                            checksum,
                            expected_size,
                            IntegrityResponseStatusClass::Other,
                            IntegrityResumeState::Resume,
                            expected_size,
                            cancelled,
                        )?;
                        self.finalize_and_promote(
                            &request.destination,
                            final_destination,
                            expected_size,
                            validation,
                            sha,
                            cancelled,
                        )?;
                    } else {
                        finalize_part(final_destination, expected_size)
                            .map_err(AttemptError::file)?;
                    }
                    return Ok(ResolvedTransferResult::Finished(success(
                        DownloadOutcome::Completed,
                        bytes,
                        attempt - 1,
                    )));
                }
                FilePreparation::Resume { offset } => offset,
                FilePreparation::Restart => 0,
            };

            let allow_resume_existing_meta =
                matches!(preparation, FilePreparation::Resume { .. }) && !session.force_restart;
            let mut prefetched_response: Option<Response> = None;
            if !is_stall_retry
                && session.recovered_checkpoint.is_none()
                && self.should_attempt_segmented(
                    &request.destination,
                    &request.url,
                    expected_size,
                    resume_from,
                    allow_resume_existing_meta,
                )
            {
                let expected_remote_validator = asset.remote_validator().clone();
                match self.try_segmented_download(
                    request,
                    final_destination,
                    expected_size,
                    checksum,
                    validation,
                    attempt,
                    allow_resume_existing_meta,
                    Some(auth_refresh_policy),
                    Some(&expected_remote_validator),
                    Some(session),
                    cancelled,
                    paused,
                    progress,
                    &|builder, current_url| {
                        apply_resolved_asset_access(
                            asset,
                            &initial_url,
                            current_url,
                            runtime_bearer,
                            builder,
                            self.uncredentialed_client(),
                        )
                    },
                    &|_, candidate| {
                        resolved_redirect_allowed(
                            auth_refresh_policy,
                            asset.host_policy().map_or_else(
                                || {
                                    crate::redirect_policy::default_redirect_target_allowed(
                                        &initial_url,
                                        candidate,
                                    )
                                },
                                |policy| policy.allows_redirect(&initial_url, candidate),
                            ),
                        )
                    },
                )? {
                    SegmentedAttemptOutcome::Completed(result) => {
                        return Ok(ResolvedTransferResult::Finished(result));
                    }
                    SegmentedAttemptOutcome::AuthRejected(resp) => {
                        return Ok(classify_rejection(&resp));
                    }
                    SegmentedAttemptOutcome::FallbackWithFullResponse(resp) => {
                        resume_from = 0;
                        prefetched_response = Some(resp);
                    }
                    SegmentedAttemptOutcome::NotApplicable
                    | SegmentedAttemptOutcome::FallbackSingleStream => {}
                }
            }

            if prefetched_response.is_none()
                && resume_from > 0
                && PartMetaCheckpoint::load(&request.destination)
                    .is_some_and(|meta| meta.segmented.is_some())
            {
                if checksum.is_some() && session.recovered_checkpoint.is_none() {
                    return Err(AttemptError::resume_refused(
                        IntegrityDiagnostic::for_response(
                            IntegritySubtype::ResponsePreparation,
                            StatusCode::OK,
                            resume_from,
                            Some(expected_size),
                            IntegritySizeRelation::Unknown,
                        ),
                    ));
                }
                if session.required_resume_offset.is_some() {
                    return Err(AttemptError::file(DownloadFileError::IntegrityMismatch));
                }
                PartMetaCheckpoint::clear(&request.destination);
                resume_from = 0;
            }

            let (response, single_overlap) = if let Some(resp) = prefetched_response {
                (resp, 0_u64)
            } else {
                let single_overlap = if resume_from > 0 && session.recovered_checkpoint.is_none() {
                    resume_from.min(self.engine_config.single_stream_resume_overlap_bytes)
                } else {
                    0
                };
                let request_offset = resume_from.saturating_sub(single_overlap);
                let if_range = session.prior_response_validator().and_then(if_range_value);
                if resume_from > 0 && resume_diagnostic_enabled() {
                    // Validator kind only; the raw ETag/date value is never printed.
                    eprintln!(
                        "INSAR_USGS_M2M_RESUME_DIAGNOSTIC {{\"event\":\"range_probe\",\"offset\":{request_offset},\"if_range\":\"{}\"}}",
                        if if_range.is_some() { "present" } else { "none" }
                    );
                }
                let response = self.send_with_controls(
                    &initial_url,
                    request_offset,
                    if_range,
                    cancelled,
                    |builder, current_url| {
                        apply_resolved_asset_access(
                            asset,
                            &initial_url,
                            current_url,
                            runtime_bearer,
                            builder,
                            self.uncredentialed_client(),
                        )
                    },
                    |_, candidate| {
                        resolved_redirect_allowed(
                            auth_refresh_policy,
                            asset.host_policy().map_or_else(
                                || {
                                    crate::redirect_policy::default_redirect_target_allowed(
                                        &initial_url,
                                        candidate,
                                    )
                                },
                                |policy| policy.allows_redirect(&initial_url, candidate),
                            ),
                        )
                    },
                )?;
                let Some(response) = response else {
                    return Ok(ResolvedTransferResult::Finished(interrupted(
                        &request.destination,
                        attempt,
                    )));
                };
                (response, single_overlap)
            };

            let status = response.status();
            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                return Ok(classify_rejection(&response));
            }
            if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                if status == StatusCode::TOO_MANY_REQUESTS {
                    self.effective_host_concurrency()
                        .record_rate_limit(&request.url, 1);
                }
                let retry_after =
                    Self::bhoonidhi_aware_retry_after(&request.url, status, response.headers());
                return Err(AttemptError::transient_for_status(status, retry_after));
            }
            // A refused resume (416, or a full 200 that ignored Range) cannot
            // authorize deleting the prefix. With a service checksum and no
            // independently verified checkpoint, the prefix is kept under a
            // conflict name and a new transfer starts from zero, which the checksum
            // verifies. CDSE builds Sentinel-2 zips on the fly and stops honouring
            // Range about a minute after the original transfer.
            let refused_resume_restarts =
                resume_from > 0 && checksum.is_some() && session.recovered_checkpoint.is_none();
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                let diagnostic = IntegrityDiagnostic::for_response(
                    IntegritySubtype::RangeNotSatisfiable,
                    status,
                    resume_from,
                    None,
                    IntegritySizeRelation::Unknown,
                );
                return Err(if refused_resume_restarts {
                    AttemptError::resume_refused(diagnostic)
                } else {
                    AttemptError::integrity(false, diagnostic)
                });
            }
            if status.is_client_error() || status.is_redirection() {
                return Err(AttemptError::http_status(status));
            }
            if refused_resume_restarts && status == StatusCode::OK {
                return Err(AttemptError::resume_refused(
                    IntegrityDiagnostic::for_response(
                        IntegritySubtype::ResponsePreparation,
                        status,
                        resume_from,
                        None,
                        IntegritySizeRelation::Unknown,
                    ),
                ));
            }

            if !is_stall_retry {
                if let Some(saved) = &session.recovered_checkpoint {
                    let range = response
                        .headers()
                        .get(CONTENT_RANGE)
                        .and_then(|value| value.to_str().ok());
                    if status != StatusCode::PARTIAL_CONTENT
                        || !saved.matches_response_range(range, response.content_length())
                    {
                        // A malformed response is not authority to discard an independently
                        // verified local prefix. Reject before generic restart/cleanup logic.
                        return Err(AttemptError::integrity(
                            false,
                            IntegrityDiagnostic::for_response(
                                IntegritySubtype::ResponsePreparation,
                                status,
                                resume_from,
                                None,
                                IntegritySizeRelation::Unknown,
                            ),
                        ));
                    }
                }
            }
            let (preparation, remote_total) =
                prepare_response_file(resume_from, single_overlap, &response)?;
            if matches!((expected_size, remote_total), (Some(left), Some(right)) if left != right) {
                return Err(AttemptError::integrity(
                    false,
                    IntegrityDiagnostic::for_response(
                        IntegritySubtype::ExpectedRemoteTotalMismatch,
                        status,
                        resume_from,
                        Some(remote_total),
                        IntegritySizeRelation::Mismatch,
                    ),
                ));
            }
            let response_validator = response_validator(&response, remote_total);
            let validator_diagnostic = IntegrityDiagnostic::for_response(
                IntegritySubtype::ValidatorMismatch,
                status,
                resume_from,
                Some(remote_total),
                IntegritySizeRelation::between(expected_size, remote_total),
            );
            ensure_response_validator_compatible(
                asset.remote_validator(),
                &response_validator,
                validator_diagnostic,
            )?;
            if !is_stall_retry {
                if let Some(saved) = &session.recovered_checkpoint {
                    if status != StatusCode::PARTIAL_CONTENT {
                        return Err(AttemptError::integrity(false, validator_diagnostic));
                    }
                    // The probe asset is only verification input, built from this real
                    // response. It is never dispatched and carries no credential.
                    let mut observed =
                        ResolvedAsset::new(request.url.as_str(), ResolvedMethod::Get)
                            .map_err(|_| AttemptError::fatal())?
                            .with_expected_size(expected_size.or(remote_total))
                            .with_remote_validator(response_validator.clone());
                    if asset.remote_response_size_is_authoritative() {
                        // Carry the real asset's size authority onto the probe asset:
                        // the request may hold a frozen advisory size (USGS D806)
                        // that legitimately differs from the real object.
                        observed = observed.with_remote_response_size_authority();
                    }
                    let mut verification =
                        ResolvedTransferSession::new(1).map_err(|_| AttemptError::fatal())?;
                    verification
                        .resume_verified_part(
                            request,
                            &observed,
                            &saved.validator(),
                            saved.bytes,
                            &saved.prefix_sha256,
                            cancelled,
                        )
                        .map_err(AttemptError::file)?;
                }
            }
            let checkpoint = session
                .recovery_identity
                .clone()
                .map(|binding| (binding, response_validator.clone()));
            session.record_response_validator(response_validator, validator_diagnostic)?;
            let expected = expected_size.or(remote_total);
            match self.stream_response_to_part(
                request,
                final_destination,
                attempt,
                preparation,
                single_overlap,
                expected,
                remote_total,
                IntegrityResumeState::from_offset(resume_from),
                response,
                checksum,
                validation,
                false,
                cancelled,
                paused,
                progress,
                checkpoint
                    .as_ref()
                    .map(|(binding, validator)| (binding.as_str(), validator)),
                &mut || session.mark_part_opened(),
            ) {
                Ok(result) => {
                    if result.outcome == DownloadOutcome::Completed {
                        self.effective_host_concurrency()
                            .record_clean_completion(&request.url);
                    }
                    return Ok(ResolvedTransferResult::Finished(result));
                }
                Err(err) if err.stall_resume_offset.is_some() => {
                    let offset = err.stall_resume_offset.unwrap_or(0);
                    if offset > last_stall_bytes || !is_stall_retry {
                        zero_progress_stalls = 0;
                        last_stall_bytes = offset;
                        stall_reconnects += 1;
                    } else {
                        zero_progress_stalls += 1;
                    }
                    if stall_reconnects > self.engine_config.max_stall_reconnects
                        || zero_progress_stalls > 2
                    {
                        return Err(AttemptError::stall_exhausted(
                            stall_reconnects,
                            zero_progress_stalls,
                        ));
                    }
                    self.stall_reconnect_count.fetch_add(1, Ordering::Relaxed);
                    progress(TransferUpdate::StallReconnect { attempt });
                    is_stall_retry = true;
                    if wait_backoff(Duration::from_millis(25), cancelled) {
                        return Ok(ResolvedTransferResult::Finished(interrupted(
                            &request.destination,
                            attempt,
                        )));
                    }
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub fn establish_burst_session(
        &self,
        auth: &TransferAuth,
        endpoints: &BurstAuthEndpoints,
    ) -> Result<(), BurstSessionError> {
        let TransferAuth::Basic { username, password } = auth else {
            return Err(BurstSessionError::CookieRequired);
        };
        let token_response = self
            .client
            .post(endpoints.token_url.clone())
            .basic_auth(username.expose(), Some(password.expose()))
            .send()
            .map_err(|_| BurstSessionError::Remote)?;
        if matches!(
            token_response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(BurstSessionError::CredentialRejected);
        }
        if !token_response.status().is_success() {
            return Err(BurstSessionError::Remote);
        }
        drop(token_response);

        let initial = endpoints.authorize_url.clone();
        let mut current = initial.clone();
        let mut redirect_budget = RedirectBudget::default();
        loop {
            let mut builder = self.client.get(current.clone());
            if current == initial || auth_allowed_redirect(&initial, &current) {
                builder = builder.basic_auth(username.expose(), Some(password.expose()));
            }
            let response = builder.send().map_err(|_| BurstSessionError::Remote)?;
            if matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ) {
                return Err(BurstSessionError::CredentialRejected);
            }
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(BurstSessionError::Remote)?;
                if !redirect_budget.try_claim() {
                    return Err(BurstSessionError::Remote);
                }
                let next = current
                    .join(location)
                    .map_err(|_| BurstSessionError::Remote)?;
                if !auth_allowed_redirect(&initial, &next) {
                    return Err(BurstSessionError::Remote);
                }
                current = next;
                continue;
            }
            if !response.status().is_success() {
                return Err(BurstSessionError::Remote);
            }
            break;
        }

        let has_cookie = self
            .cookie_jar
            .cookies(&endpoints.cookie_scope_url)
            .and_then(|value| value.to_str().ok().map(str::to_owned))
            .is_some_and(|value| {
                value.split(';').any(|cookie| {
                    cookie
                        .trim()
                        .split_once('=')
                        .is_some_and(|(name, content)| name == "asf-urs" && !content.is_empty())
                })
            });
        if !has_cookie {
            return Err(BurstSessionError::CookieRequired);
        }
        Ok(())
    }

    pub fn download(
        &self,
        request: &TransferRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        self.download_authenticated(request, &TransferAuth::None, cancelled, paused, progress)
    }

    pub fn download_authenticated(
        &self,
        request: &TransferRequest,
        auth: &TransferAuth,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> TransferResult {
        let preparation = match inspect_destination(&request.destination, request.expected_size) {
            Ok(preparation) => preparation,
            Err(error) => return file_failure(error, 0),
        };
        match preparation {
            FilePreparation::SkipComplete { bytes } => {
                return success(DownloadOutcome::Skipped, bytes, 0);
            }
            FilePreparation::FinalizePart { bytes } => {
                return match finalize_part(&request.destination, request.expected_size) {
                    Ok(_) => success(DownloadOutcome::Completed, bytes, 0),
                    Err(error) => file_failure(error, 0),
                };
            }
            FilePreparation::Resume { .. } | FilePreparation::Restart => {}
        }
        if cancelled() {
            return interrupted(&request.destination, 0);
        }

        let effective_request_owned: Option<TransferRequest> =
            if let Some(staging_dir) = self.resolve_ssd_staging_dir(&request.destination) {
                let staged_dest = staging_target_path(&staging_dir, &request.destination);
                let required_bytes = request
                    .expected_size
                    .unwrap_or(0)
                    .saturating_sub(part_allocated_size(&staged_dest));
                if let Err(error) = check_ssd_staging_space(
                    &*self.staging_free_space_probe,
                    &staging_dir,
                    required_bytes,
                ) {
                    return file_failure(error, 0);
                }
                let mut cloned = request.clone();
                cloned.destination = staged_dest;
                Some(cloned)
            } else {
                None
            };
        let effective_request = effective_request_owned.as_ref().unwrap_or(request);

        let mut last_failure = transient_failure();
        for attempt in 1..=self.options.max_attempts {
            if cancelled() {
                return interrupted(&effective_request.destination, attempt - 1);
            }
            let next_retry_after = match self.attempt(
                effective_request,
                &request.destination,
                auth,
                attempt,
                cancelled,
                paused,
                progress,
            ) {
                Ok(result) => return result,
                Err(error) => {
                    last_failure = error.failure;
                    if !apply_part_action(&effective_request.destination, error.part) {
                        return part_set_aside_failure(&effective_request.destination, attempt);
                    }
                    if !error.retryable || attempt == self.options.max_attempts {
                        return TransferResult {
                            outcome: DownloadOutcome::Failed,
                            bytes_written: part_size(&effective_request.destination),
                            attempt_count: attempt,
                            failure: Some(last_failure),
                        };
                    }
                    error.retry_after_hint
                }
            };
            let base_delay = self.options.backoff.saturating_mul(attempt);
            let delay = next_retry_after.map_or(base_delay, |hint| base_delay.max(hint));
            if wait_backoff(delay, cancelled) {
                return interrupted(&effective_request.destination, attempt);
            }
        }
        TransferResult {
            outcome: DownloadOutcome::Failed,
            bytes_written: part_size(&effective_request.destination),
            attempt_count: self.options.max_attempts,
            failure: Some(last_failure),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn attempt(
        &self,
        request: &TransferRequest,
        final_destination: &Path,
        auth: &TransferAuth,
        attempt: u32,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> Result<TransferResult, AttemptError> {
        let initial_url = request.url.clone();
        let mut stall_reconnects: u32 = 0;
        let mut last_stall_bytes: u64 = 0;
        let mut zero_progress_stalls: u32 = 0;
        let mut is_stall_retry = false;

        loop {
            let preparation = inspect_destination(&request.destination, request.expected_size)
                .map_err(AttemptError::file)?;
            let mut resume_from = match preparation {
                FilePreparation::SkipComplete { bytes } => {
                    return Ok(success(DownloadOutcome::Skipped, bytes, attempt - 1));
                }
                FilePreparation::FinalizePart { bytes } => {
                    if request.destination != final_destination {
                        let sha = verify_and_hash_part(
                            &part_path(&request.destination),
                            None,
                            request.expected_size,
                            IntegrityResponseStatusClass::Other,
                            IntegrityResumeState::Resume,
                            request.expected_size,
                            cancelled,
                        )?;
                        self.finalize_and_promote(
                            &request.destination,
                            final_destination,
                            request.expected_size,
                            CompletedArtifactValidationInvocation::none(),
                            sha,
                            cancelled,
                        )?;
                    } else {
                        finalize_part(final_destination, request.expected_size)
                            .map_err(AttemptError::file)?;
                    }
                    return Ok(success(DownloadOutcome::Completed, bytes, attempt - 1));
                }
                FilePreparation::Resume { offset } => offset,
                FilePreparation::Restart => 0,
            };

            let allow_resume_existing_meta = matches!(preparation, FilePreparation::Resume { .. });
            let mut prefetched_response: Option<Response> = None;
            if !is_stall_retry
                && self.should_attempt_segmented(
                    &request.destination,
                    &request.url,
                    request.expected_size,
                    resume_from,
                    allow_resume_existing_meta,
                )
            {
                match self.try_segmented_download(
                    request,
                    final_destination,
                    request.expected_size,
                    None,
                    CompletedArtifactValidationInvocation::none(),
                    attempt,
                    allow_resume_existing_meta,
                    None,
                    None,
                    None,
                    cancelled,
                    paused,
                    progress,
                    &|builder, current_url| {
                        if *current_url == initial_url
                            || auth_allowed_redirect(&initial_url, current_url)
                        {
                            match auth {
                                TransferAuth::None => builder,
                                TransferAuth::Bearer(token) => builder.bearer_auth(token.expose()),
                                TransferAuth::Basic { username, password } => {
                                    builder.basic_auth(username.expose(), Some(password.expose()))
                                }
                            }
                        } else {
                            builder
                        }
                    },
                    &crate::redirect_policy::default_redirect_target_allowed,
                )? {
                    SegmentedAttemptOutcome::Completed(result) => return Ok(result),
                    SegmentedAttemptOutcome::AuthRejected(_) => {
                        return Err(AttemptError::credential());
                    }
                    SegmentedAttemptOutcome::FallbackWithFullResponse(resp) => {
                        resume_from = 0;
                        prefetched_response = Some(resp);
                    }
                    SegmentedAttemptOutcome::NotApplicable
                    | SegmentedAttemptOutcome::FallbackSingleStream => {}
                }
            }

            if prefetched_response.is_none()
                && resume_from > 0
                && PartMetaCheckpoint::load(&request.destination)
                    .is_some_and(|meta| meta.segmented.is_some())
            {
                PartMetaCheckpoint::clear(&request.destination);
                resume_from = 0;
            }

            let (response, single_overlap) = if let Some(resp) = prefetched_response {
                (resp, 0_u64)
            } else {
                let single_overlap = if resume_from > 0 {
                    resume_from.min(self.engine_config.single_stream_resume_overlap_bytes)
                } else {
                    0
                };
                let request_offset = resume_from.saturating_sub(single_overlap);
                let response = self.send_with_controls(
                    &initial_url,
                    request_offset,
                    None,
                    cancelled,
                    |builder, current_url| {
                        if *current_url == initial_url
                            || auth_allowed_redirect(&initial_url, current_url)
                        {
                            match auth {
                                TransferAuth::None => builder,
                                TransferAuth::Bearer(token) => builder.bearer_auth(token.expose()),
                                TransferAuth::Basic { username, password } => {
                                    builder.basic_auth(username.expose(), Some(password.expose()))
                                }
                            }
                        } else {
                            builder
                        }
                    },
                    crate::redirect_policy::default_redirect_target_allowed,
                )?;
                let Some(response) = response else {
                    return Ok(interrupted(&request.destination, attempt));
                };
                (response, single_overlap)
            };

            match self.consume_response(
                request,
                final_destination,
                attempt,
                resume_from,
                single_overlap,
                response,
                cancelled,
                paused,
                progress,
            ) {
                Ok(result) => return Ok(result),
                Err(err) if err.stall_resume_offset.is_some() => {
                    let offset = err.stall_resume_offset.unwrap_or(0);
                    if offset > last_stall_bytes || !is_stall_retry {
                        zero_progress_stalls = 0;
                        last_stall_bytes = offset;
                        stall_reconnects += 1;
                    } else {
                        zero_progress_stalls += 1;
                    }
                    if stall_reconnects > self.engine_config.max_stall_reconnects
                        || zero_progress_stalls > 2
                    {
                        return Err(AttemptError::stall_exhausted(
                            stall_reconnects,
                            zero_progress_stalls,
                        ));
                    }
                    self.stall_reconnect_count.fetch_add(1, Ordering::Relaxed);
                    progress(TransferUpdate::StallReconnect { attempt });
                    is_stall_retry = true;
                    if wait_backoff(Duration::from_millis(25), cancelled) {
                        return Ok(interrupted(&request.destination, attempt));
                    }
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn verify_stream_overlap(
        &self,
        reader: &mut WatchdogStreamReader,
        expected_overlap: &[u8],
        response_status: StatusCode,
        resume_from: u64,
        remote_total: Option<u64>,
        watchdog: &mut StallWatchdog,
        limiter: &Arc<BandwidthLimiter>,
        stream_guard: &mut Option<ActiveStreamGuard>,
        worker_abort: Option<&AtomicBool>,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        on_pause: &mut dyn FnMut() -> Result<(), AttemptError>,
    ) -> Result<bool, AttemptError> {
        if expected_overlap.is_empty() {
            return Ok(true);
        }
        let mut remote_buf = vec![0_u8; expected_overlap.len()];
        let mut filled = 0_usize;
        while filled < expected_overlap.len() {
            if cancelled() || worker_abort.is_some_and(|a| a.load(Ordering::Acquire)) {
                return Ok(false);
            }
            if paused() {
                *stream_guard = None;
                on_pause()?;
                while paused() {
                    if cancelled() || worker_abort.is_some_and(|a| a.load(Ordering::Acquire)) {
                        return Ok(false);
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                watchdog.reset();
                *stream_guard = Some(limiter.begin_stream());
            }
            let need = expected_overlap.len() - filled;
            let acquire_started = (limiter.limit_bps() > 0).then(Instant::now);
            let grant = match limiter.acquire(need, cancelled, paused, worker_abort) {
                AcquireOutcome::Granted(n) => n,
                AcquireOutcome::Paused => continue,
                AcquireOutcome::Cancelled => return Ok(false),
            };
            if let Some(started) = acquire_started {
                watchdog.extend_for_pacing(started.elapsed());
            }
            match reader.read(&mut remote_buf[filled..filled + grant]) {
                Ok(0) => {
                    limiter.refund(grant);
                    return Err(AttemptError::integrity(
                        false,
                        IntegrityDiagnostic::for_response(
                            IntegritySubtype::BodyFinalSizeMismatch,
                            response_status,
                            resume_from,
                            Some(remote_total),
                            IntegritySizeRelation::Unknown,
                        ),
                    ));
                }
                Ok(n) => {
                    limiter.refund(grant.saturating_sub(n));
                    filled += n;
                    watchdog.record(n as u64, Instant::now());
                    if watchdog.is_stalled(
                        Instant::now(),
                        limiter.effective_stall_min_bps(self.engine_config.stall_min_bps),
                    ) {
                        return Err(AttemptError::stalled(resume_from));
                    }
                }
                Err(ref err) if watchdog.enabled && is_io_timeout(err) => {
                    limiter.refund(grant);
                    if watchdog.is_stalled(
                        Instant::now(),
                        limiter.effective_stall_min_bps(self.engine_config.stall_min_bps),
                    ) {
                        return Err(AttemptError::stalled(resume_from));
                    }
                    continue;
                }
                Err(err) => {
                    limiter.refund(grant);
                    return Err(AttemptError::stream_read(&err));
                }
            }
        }
        if remote_buf.as_slice() != expected_overlap {
            return Err(AttemptError::integrity(
                false,
                IntegrityDiagnostic::for_response(
                    IntegritySubtype::ValidatorMismatch,
                    response_status,
                    resume_from,
                    Some(remote_total),
                    IntegritySizeRelation::Unknown,
                ),
            ));
        }
        self.resume_overlap_verified_count
            .fetch_add(1, Ordering::Relaxed);
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn stream_response_to_part(
        &self,
        request: &TransferRequest,
        final_destination: &Path,
        attempt: u32,
        preparation: FilePreparation,
        overlap_bytes: u64,
        expected: Option<u64>,
        remote_total: Option<u64>,
        resume_state: IntegrityResumeState,
        response: Response,
        checksum: Option<&ExpectedChecksum>,
        validation: CompletedArtifactValidationInvocation<'_>,
        overflow_clears_part: bool,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
        checkpoint: Option<(&str, &RemoteValidator)>,
        on_open: &mut dyn FnMut(),
    ) -> Result<TransferResult, AttemptError> {
        let response_status = IntegrityResponseStatusClass::from_status(response.status());
        let integrity_diagnostic = |subtype, size_relation| IntegrityDiagnostic {
            subtype,
            response_status,
            resume: resume_state,
            remote_total: IntegrityRemoteTotalPresence::from_known(remote_total),
            size_relation,
        };
        let response_etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let response_last_modified = response
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let url_digest = part_url_sha256(request.url.as_str());

        let mut written = match preparation {
            FilePreparation::Resume { offset } => offset,
            FilePreparation::Restart => 0,
            _ => return Err(AttemptError::fatal()),
        };
        let file = open_part_for_transfer(&request.destination, preparation)
            .map_err(AttemptError::file)?;
        on_open();

        let can_skip_prefix_hash = if written > 0 && checkpoint.is_none() {
            match (
                file.metadata(),
                PartMetaCheckpoint::load(&request.destination),
            ) {
                (Ok(metadata), Some(meta)) => meta.matches_single_stream_resume(
                    &metadata,
                    written,
                    &url_digest,
                    response_etag.as_deref(),
                ),
                _ => false,
            }
        } else {
            false
        };

        let mut hashed_from_zero = !can_skip_prefix_hash;
        if can_skip_prefix_hash {
            self.resume_prefix_hash_skipped_count
                .fetch_add(1, Ordering::Relaxed);
        }

        let mut checksum_accumulator = if hashed_from_zero {
            checksum.map(|expected| ChecksumAccumulator::new(expected.algorithm()))
        } else {
            None
        };
        let mut promotion_sha256 = Sha256Accumulator::default();
        let mut writer = BufWriter::with_capacity(PART_BUF_WRITER_CAPACITY, file);
        let mut buffer = vec![0_u8; self.options.chunk_size];
        if written > 0 && hashed_from_zero {
            let mut prefix = writer
                .get_ref()
                .try_clone()
                .map_err(AttemptError::file_io)?;
            prefix
                .seek(SeekFrom::Start(0))
                .map_err(AttemptError::file_io)?;
            let mut remaining = written;
            while remaining > 0 {
                if cancelled() {
                    writer.flush().map_err(AttemptError::file_io)?;
                    return Ok(interrupted(&request.destination, attempt));
                }
                while paused() {
                    if cancelled() {
                        writer.flush().map_err(AttemptError::file_io)?;
                        return Ok(interrupted(&request.destination, attempt));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                let limit = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| {
                    AttemptError::integrity(
                        false,
                        integrity_diagnostic(
                            IntegritySubtype::ResponsePreparation,
                            IntegritySizeRelation::Unknown,
                        ),
                    )
                })?;
                let count = prefix
                    .read(&mut buffer[..limit])
                    .map_err(AttemptError::file_io)?;
                if count == 0 {
                    return Err(AttemptError::integrity(
                        false,
                        integrity_diagnostic(
                            IntegritySubtype::ResponsePreparation,
                            IntegritySizeRelation::Unknown,
                        ),
                    ));
                }
                if let Some(checksum) = checksum_accumulator.as_mut() {
                    checksum.update(&buffer[..count]);
                }
                promotion_sha256.update(&buffer[..count]);
                remaining -= count as u64;
            }
            hashed_from_zero = true;
        }
        let mut body_active = true;
        let mut checkpoint_bytes = written;
        let mut checkpoint_time = Instant::now();
        let mut last_flush_at = Instant::now();
        let checkpoint_interval_bytes = 32 * 1024 * 1024;
        #[cfg(test)]
        let checkpoint_interval_bytes = self
            .checkpoint_interval_bytes
            .unwrap_or(checkpoint_interval_bytes);
        let http_status_raw = response.status();
        let limiter = self.effective_bandwidth_limiter();
        let expected_overlap = if overlap_bytes > 0 && written >= overlap_bytes {
            let overlap_len = usize::try_from(overlap_bytes).map_err(|_| {
                AttemptError::integrity(
                    false,
                    integrity_diagnostic(
                        IntegritySubtype::ResponsePreparation,
                        IntegritySizeRelation::Unknown,
                    ),
                )
            })?;
            let mut buf = vec![0_u8; overlap_len];
            read_exact_at(writer.get_ref(), &mut buf, written - overlap_bytes)
                .map_err(AttemptError::file_io)?;
            buf
        } else {
            Vec::new()
        };
        let mut watchdog = StallWatchdog::new(
            self.effective_stall_enabled(),
            self.engine_config.stall_min_bps,
            self.engine_config.stall_window,
        );
        let mut reader = WatchdogStreamReader::new(
            response,
            watchdog.enabled,
            buffer.len(),
            self.engine_config.stall_window,
        );
        progress(TransferUpdate::BodyStarted {
            attempt,
            bytes_completed: written,
        });
        let body_result = catch_unwind(AssertUnwindSafe(
            || -> Result<Option<TransferResult>, AttemptError> {
                let mut stream_guard = Some(limiter.begin_stream());
                if !expected_overlap.is_empty() {
                    let mut on_pause = || -> Result<(), AttemptError> {
                        writer.flush().map_err(AttemptError::file_io)?;
                        writer
                            .get_ref()
                            .sync_data()
                            .map_err(AttemptError::file_io)?;
                        let _ = save_part_meta_checkpoint(
                            &request.destination,
                            writer.get_ref(),
                            &url_digest,
                            response_etag.clone(),
                            response_last_modified.clone(),
                            None,
                        );
                        if body_active {
                            progress(TransferUpdate::BodyStopped { attempt });
                            body_active = false;
                        }
                        Ok(())
                    };
                    let verified = self.verify_stream_overlap(
                        &mut reader,
                        &expected_overlap,
                        http_status_raw,
                        written,
                        remote_total,
                        &mut watchdog,
                        limiter,
                        &mut stream_guard,
                        None,
                        cancelled,
                        paused,
                        &mut on_pause,
                    )?;
                    if !body_active {
                        progress(TransferUpdate::BodyStarted {
                            attempt,
                            bytes_completed: written,
                        });
                        body_active = true;
                    }
                    if !verified {
                        writer.flush().map_err(AttemptError::file_io)?;
                        writer
                            .get_ref()
                            .sync_data()
                            .map_err(AttemptError::file_io)?;
                        let _ = save_part_meta_checkpoint(
                            &request.destination,
                            writer.get_ref(),
                            &url_digest,
                            response_etag.clone(),
                            response_last_modified.clone(),
                            None,
                        );
                        return Ok(Some(interrupted(&request.destination, attempt)));
                    }
                }
                loop {
                    if cancelled() {
                        writer.flush().map_err(AttemptError::file_io)?;
                        writer
                            .get_ref()
                            .sync_data()
                            .map_err(AttemptError::file_io)?;
                        let _ = save_part_meta_checkpoint(
                            &request.destination,
                            writer.get_ref(),
                            &url_digest,
                            response_etag.clone(),
                            response_last_modified.clone(),
                            None,
                        );
                        return Ok(Some(interrupted(&request.destination, attempt)));
                    }
                    if paused() {
                        drop(stream_guard.take());
                        writer.flush().map_err(AttemptError::file_io)?;
                        writer
                            .get_ref()
                            .sync_data()
                            .map_err(AttemptError::file_io)?;
                        let _ = save_part_meta_checkpoint(
                            &request.destination,
                            writer.get_ref(),
                            &url_digest,
                            response_etag.clone(),
                            response_last_modified.clone(),
                            None,
                        );
                        if let Some((binding, validator)) = checkpoint {
                            if written > checkpoint_bytes {
                                let saved = PartCheckpoint::save(
                                    &request.destination,
                                    binding,
                                    validator,
                                    written,
                                    promotion_sha256.snapshot_hex(),
                                );
                                if saved.is_err() {
                                    checkpoint_diagnostic(
                                        "checkpoint_save_failed",
                                        "\"reason\":\"periodic_save_failed\"",
                                    );
                                }
                                checkpoint_bytes = written;
                                checkpoint_time = Instant::now();
                            }
                        }
                        if body_active {
                            progress(TransferUpdate::BodyStopped { attempt });
                            body_active = false;
                        }
                        while paused() {
                            if cancelled() {
                                writer.flush().map_err(AttemptError::file_io)?;
                                return Ok(Some(interrupted(&request.destination, attempt)));
                            }
                            thread::sleep(Duration::from_millis(50));
                        }
                        watchdog.reset();
                        stream_guard = Some(limiter.begin_stream());
                        progress(TransferUpdate::BodyStarted {
                            attempt,
                            bytes_completed: written,
                        });
                        body_active = true;
                    }
                    let acquire_started = (limiter.limit_bps() > 0).then(Instant::now);
                    let grant = match limiter.acquire(buffer.len(), cancelled, paused, None) {
                        AcquireOutcome::Granted(n) => n,
                        AcquireOutcome::Paused => continue,
                        AcquireOutcome::Cancelled => {
                            writer.flush().map_err(AttemptError::file_io)?;
                            writer
                                .get_ref()
                                .sync_data()
                                .map_err(AttemptError::file_io)?;
                            let _ = save_part_meta_checkpoint(
                                &request.destination,
                                writer.get_ref(),
                                &url_digest,
                                response_etag.clone(),
                                response_last_modified.clone(),
                                None,
                            );
                            return Ok(Some(interrupted(&request.destination, attempt)));
                        }
                    };
                    if let Some(started) = acquire_started {
                        watchdog.extend_for_pacing(started.elapsed());
                    }
                    let count = match reader.read(&mut buffer[..grant]) {
                        Ok(n) => {
                            limiter.refund(grant.saturating_sub(n));
                            n
                        }
                        Err(ref err) if watchdog.enabled && is_io_timeout(err) => {
                            limiter.refund(grant);
                            let _ = writer.flush();
                            last_flush_at = Instant::now();
                            if cancelled() {
                                writer.flush().map_err(AttemptError::file_io)?;
                                writer
                                    .get_ref()
                                    .sync_data()
                                    .map_err(AttemptError::file_io)?;
                                return Ok(Some(interrupted(&request.destination, attempt)));
                            }
                            if watchdog.is_stalled(
                                Instant::now(),
                                limiter.effective_stall_min_bps(self.engine_config.stall_min_bps),
                            ) {
                                writer.flush().map_err(AttemptError::file_io)?;
                                writer
                                    .get_ref()
                                    .sync_data()
                                    .map_err(AttemptError::file_io)?;
                                let _ = save_part_meta_checkpoint(
                                    &request.destination,
                                    writer.get_ref(),
                                    &url_digest,
                                    response_etag.clone(),
                                    response_last_modified.clone(),
                                    None,
                                );
                                if let Some((binding, validator)) = checkpoint {
                                    if hashed_from_zero {
                                        let _ = PartCheckpoint::save(
                                            &request.destination,
                                            binding,
                                            validator,
                                            written,
                                            promotion_sha256.snapshot_hex(),
                                        );
                                    }
                                }
                                return Err(AttemptError::stalled(written));
                            }
                            continue;
                        }
                        Err(err) => {
                            limiter.refund(grant);
                            if written > 0 {
                                let _ = writer.flush();
                                let _ = writer.get_ref().sync_data();
                                let _ = save_part_meta_checkpoint(
                                    &request.destination,
                                    writer.get_ref(),
                                    &url_digest,
                                    response_etag.clone(),
                                    response_last_modified.clone(),
                                    None,
                                );
                            }
                            return Err(AttemptError::stream_read(&err));
                        }
                    };
                    if count == 0 {
                        break;
                    }
                    writer
                        .write_all(&buffer[..count])
                        .map_err(AttemptError::file_io)?;
                    if last_flush_at.elapsed() >= Duration::from_millis(200) {
                        let _ = writer.flush();
                        last_flush_at = Instant::now();
                    }
                    if hashed_from_zero {
                        if let Some(checksum) = checksum_accumulator.as_mut() {
                            checksum.update(&buffer[..count]);
                        }
                        promotion_sha256.update(&buffer[..count]);
                    }
                    written = written.checked_add(count as u64).ok_or_else(|| {
                        AttemptError::integrity(
                            overflow_clears_part,
                            integrity_diagnostic(
                                IntegritySubtype::BodyFinalSizeMismatch,
                                IntegritySizeRelation::Mismatch,
                            ),
                        )
                    })?;
                    watchdog.record(count as u64, Instant::now());
                    if watchdog.is_stalled(
                        Instant::now(),
                        limiter.effective_stall_min_bps(self.engine_config.stall_min_bps),
                    ) {
                        writer.flush().map_err(AttemptError::file_io)?;
                        writer
                            .get_ref()
                            .sync_data()
                            .map_err(AttemptError::file_io)?;
                        let _ = save_part_meta_checkpoint(
                            &request.destination,
                            writer.get_ref(),
                            &url_digest,
                            response_etag.clone(),
                            response_last_modified.clone(),
                            None,
                        );
                        if let Some((binding, validator)) = checkpoint {
                            if hashed_from_zero {
                                let _ = PartCheckpoint::save(
                                    &request.destination,
                                    binding,
                                    validator,
                                    written,
                                    promotion_sha256.snapshot_hex(),
                                );
                            }
                        }
                        return Err(AttemptError::stalled(written));
                    }
                    if written.saturating_sub(checkpoint_bytes) >= checkpoint_interval_bytes
                        || checkpoint_time.elapsed() >= Duration::from_secs(30)
                    {
                        writer.flush().map_err(AttemptError::file_io)?;
                        writer
                            .get_ref()
                            .sync_data()
                            .map_err(AttemptError::file_io)?;
                        let _ = save_part_meta_checkpoint(
                            &request.destination,
                            writer.get_ref(),
                            &url_digest,
                            response_etag.clone(),
                            response_last_modified.clone(),
                            None,
                        );
                        if let Some((binding, validator)) = checkpoint {
                            let saved = PartCheckpoint::save(
                                &request.destination,
                                binding,
                                validator,
                                written,
                                promotion_sha256.snapshot_hex(),
                            );
                            if saved.is_err() {
                                checkpoint_diagnostic(
                                    "checkpoint_save_failed",
                                    "\"reason\":\"periodic_save_failed\"",
                                );
                            }
                        }
                        checkpoint_bytes = written;
                        checkpoint_time = Instant::now();
                    }
                    progress(TransferUpdate::Progress(TransferProgress {
                        bytes_completed: written,
                        total_bytes: expected,
                        attempt,
                    }));
                }
                drop(stream_guard);
                Ok(None)
            },
        ));
        if body_active {
            progress(TransferUpdate::BodyStopped { attempt });
        }
        let body_result = match body_result {
            Ok(result) => result,
            Err(payload) => resume_unwind(payload),
        };
        if let Some(result) = body_result? {
            if result.outcome == DownloadOutcome::Interrupted {
                writer.flush().map_err(AttemptError::file_io)?;
                writer
                    .get_ref()
                    .sync_data()
                    .map_err(AttemptError::file_io)?;
                let _ = save_part_meta_checkpoint(
                    &request.destination,
                    writer.get_ref(),
                    &url_digest,
                    response_etag,
                    response_last_modified,
                    None,
                );
                if let Some((binding, validator)) = checkpoint {
                    if hashed_from_zero {
                        PartCheckpoint::save(
                            &request.destination,
                            binding,
                            validator,
                            written,
                            promotion_sha256.finalize_hex(),
                        )
                        .map_err(AttemptError::file)?;
                    }
                }
            }
            return Ok(result);
        }
        writer.flush().map_err(AttemptError::file_io)?;
        writer
            .get_ref()
            .sync_data()
            .map_err(AttemptError::file_io)?;
        if expected.is_some_and(|value| written > 0 && written < value) {
            let _ = save_part_meta_checkpoint(
                &request.destination,
                writer.get_ref(),
                &url_digest,
                response_etag,
                response_last_modified,
                None,
            );
        }
        drop(writer);
        if expected.is_some_and(|value| value != written) {
            return Err(AttemptError::integrity(
                false,
                integrity_diagnostic(
                    IntegritySubtype::BodyFinalSizeMismatch,
                    IntegritySizeRelation::Mismatch,
                ),
            ));
        }
        let promotion_sha256 = if hashed_from_zero {
            if checksum
                .zip(checksum_accumulator)
                .is_some_and(|(expected, actual)| actual.finalize_hex() != expected.value())
            {
                return Err(AttemptError::integrity(
                    false,
                    integrity_diagnostic(
                        IntegritySubtype::ChecksumMismatch,
                        IntegritySizeRelation::between(expected, Some(written)),
                    ),
                ));
            }
            promotion_sha256.finalize_hex()
        } else {
            verify_and_hash_part(
                &part_path(&request.destination),
                checksum,
                expected,
                response_status,
                resume_state,
                remote_total,
                cancelled,
            )?
        };
        self.finalize_and_promote(
            &request.destination,
            final_destination,
            expected,
            validation,
            promotion_sha256,
            cancelled,
        )?;
        Ok(success(DownloadOutcome::Completed, written, attempt))
    }

    fn finalize_and_promote(
        &self,
        effective_destination: &Path,
        final_destination: &Path,
        expected: Option<u64>,
        validation: CompletedArtifactValidationInvocation<'_>,
        promotion_sha256: String,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<(), AttemptError> {
        validate_completed_artifact(validation, &part_path(effective_destination))
            .map_err(AttemptError::artifact_validation)?;
        if effective_destination != final_destination {
            promote_staged_part_to_destination(
                effective_destination,
                final_destination,
                expected,
                Some(&promotion_sha256),
                cancelled,
            )
            .map_err(AttemptError::file)?;
            self.ssd_staging_promotions_count
                .fetch_add(1, Ordering::Relaxed);
        } else {
            finalize_part_with_sha256(final_destination, expected, &promotion_sha256)
                .map_err(AttemptError::file)?;
        }
        if let Ok(mut landed) = self.landed_sha256.lock() {
            // Bounded: entries nobody takes (callers without a ledger) cannot pile up.
            if landed.len() >= 1024 {
                landed.clear();
            }
            landed.insert(final_destination.to_path_buf(), promotion_sha256);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn consume_response(
        &self,
        request: &TransferRequest,
        final_destination: &Path,
        attempt: u32,
        resume_from: u64,
        overlap_bytes: u64,
        response: Response,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
    ) -> Result<TransferResult, AttemptError> {
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(AttemptError::credential());
        }
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            if status == StatusCode::TOO_MANY_REQUESTS {
                self.effective_host_concurrency()
                    .record_rate_limit(&request.url, 1);
            }
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(crate::transient_retry::parse_retry_after_header);
            return Err(AttemptError::transient_for_status(status, retry_after));
        }
        if status == StatusCode::RANGE_NOT_SATISFIABLE {
            return Err(AttemptError::integrity(
                false,
                IntegrityDiagnostic::for_response(
                    IntegritySubtype::RangeNotSatisfiable,
                    status,
                    resume_from,
                    None,
                    IntegritySizeRelation::Unknown,
                ),
            ));
        }
        if status.is_client_error() || status.is_redirection() {
            return Err(AttemptError::http_status(status));
        }

        let (preparation, remote_total) =
            prepare_response_file(resume_from, overlap_bytes, &response)?;
        if matches!((request.expected_size, remote_total), (Some(left), Some(right)) if left != right)
        {
            return Err(AttemptError::integrity(
                false,
                IntegrityDiagnostic::for_response(
                    IntegritySubtype::ExpectedRemoteTotalMismatch,
                    status,
                    resume_from,
                    Some(remote_total),
                    IntegritySizeRelation::Mismatch,
                ),
            ));
        }
        let expected = request.expected_size.or(remote_total);
        let result = self.stream_response_to_part(
            request,
            final_destination,
            attempt,
            preparation,
            overlap_bytes,
            expected,
            remote_total,
            IntegrityResumeState::from_offset(resume_from),
            response,
            None,
            CompletedArtifactValidationInvocation::none(),
            true,
            cancelled,
            paused,
            progress,
            None,
            &mut || {},
        )?;
        if result.outcome == DownloadOutcome::Completed {
            self.effective_host_concurrency()
                .record_clean_completion(&request.url);
        }
        Ok(result)
    }

    fn should_attempt_segmented(
        &self,
        destination: &Path,
        url: &Url,
        expected_size: Option<u64>,
        resume_from: u64,
        allow_resume_existing_meta: bool,
    ) -> bool {
        let Some(len) = expected_size else {
            return false;
        };
        if allow_resume_existing_meta {
            let url_digest = part_url_sha256(url.as_str());
            return PartMetaCheckpoint::load(destination).is_some_and(|meta| {
                meta.url_sha256 == url_digest
                    && meta
                        .segmented
                        .as_ref()
                        .is_some_and(|seg| seg.total_length == len && seg.is_valid_partition())
            });
        }
        if resume_from > 0 {
            return false;
        }
        let learned_cap = self.effective_host_concurrency().effective_cap(url);
        let effective_segments = learned_cap.map_or_else(
            || self.effective_per_file_segments(),
            |cap| self.effective_per_file_segments().min(cap),
        );
        effective_segments > 1 && len >= self.engine_config.segment_threshold_bytes
    }

    #[allow(clippy::too_many_arguments)]
    fn try_segmented_download(
        &self,
        request: &TransferRequest,
        final_destination: &Path,
        expected_size: Option<u64>,
        checksum: Option<&ExpectedChecksum>,
        validation: CompletedArtifactValidationInvocation<'_>,
        attempt: u32,
        allow_resume_existing_meta: bool,
        auth_refresh_policy: Option<AuthRefreshPolicyId>,
        expected_remote_validator: Option<&RemoteValidator>,
        mut session: Option<&mut ResolvedTransferSession>,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        progress: &mut dyn FnMut(TransferUpdate),
        apply_access: &(dyn Fn(RequestBuilder, &Url) -> RequestBuilder + Sync),
        redirect_allowed: &(dyn Fn(&Url, &Url) -> bool + Sync),
    ) -> Result<SegmentedAttemptOutcome, AttemptError> {
        let Some(total_length) = expected_size else {
            return Ok(SegmentedAttemptOutcome::NotApplicable);
        };
        let prior_validator: Option<RemoteValidator> = session
            .as_deref()
            .and_then(|s| s.prior_response_validator())
            .cloned();
        let (provider_hint, maximum_segment_policy) =
            Self::resolve_segment_policy(auth_refresh_policy, &request.url);
        let base_segments = self.effective_per_file_segments_for_provider(provider_hint);
        let learned_cap = self
            .effective_host_concurrency()
            .effective_cap(&request.url);
        let effective_segments = learned_cap
            .map_or(base_segments, |cap| base_segments.min(cap))
            .max(1);
        let requested_level =
            maximum_segment_policy.clamp_level(SegmentLevel::from_count(effective_segments));
        let url_digest = part_url_sha256(request.url.as_str());
        let existing_meta = if allow_resume_existing_meta {
            PartMetaCheckpoint::load(&request.destination).filter(|meta| {
                meta.url_sha256 == url_digest
                    && meta.segmented.as_ref().is_some_and(|seg| {
                        seg.total_length == total_length && seg.is_valid_partition()
                    })
            })
        } else {
            None
        };
        if existing_meta.is_none()
            && (requested_level == SegmentLevel::One
                || total_length < self.engine_config.segment_threshold_bytes)
        {
            return Ok(SegmentedAttemptOutcome::NotApplicable);
        }
        let refused_resume_restarts = checksum.is_some();
        let frozen_checksum = checksum.and_then(|c| {
            let algorithm = match c.algorithm() {
                ChecksumAlgorithm::Md5 => FrozenChecksumAlgorithm::Md5,
                ChecksumAlgorithm::Sha1 => FrozenChecksumAlgorithm::Sha1,
                ChecksumAlgorithm::Sha256 => FrozenChecksumAlgorithm::Sha256,
                ChecksumAlgorithm::Sha512 => return None,
            };
            Some(FrozenChecksumEvidence {
                algorithm,
                digest: c.value().to_owned(),
                expected_size: Some(total_length),
            })
        });
        let minimum_segment_bytes = (self.engine_config.segment_threshold_bytes / 8).max(1);

        let (
            plan,
            initial_segments,
            mut probe_response_for_zero,
            response_etag,
            response_last_modified,
        ) = if let Some(meta) = existing_meta {
            let meta_validator = RemoteValidator {
                etag: meta
                    .etag
                    .as_deref()
                    .and_then(strong_etag)
                    .map(str::to_string),
                last_modified: meta
                    .last_modified
                    .as_deref()
                    .and_then(parse_canonical_http_date),
                content_length: Some(total_length),
            };
            let meta_validator_diagnostic = IntegrityDiagnostic::for_response(
                IntegritySubtype::ValidatorMismatch,
                StatusCode::PARTIAL_CONTENT,
                meta.total_segmented_written(),
                Some(Some(total_length)),
                IntegritySizeRelation::Equal,
            );
            if let Some(expected_validator) = expected_remote_validator {
                ensure_response_validator_compatible(
                    expected_validator,
                    &meta_validator,
                    meta_validator_diagnostic,
                )?;
            }
            if let Some(prior) = prior_validator.as_ref() {
                ensure_response_validator_compatible(
                    prior,
                    &meta_validator,
                    meta_validator_diagnostic,
                )?;
            }
            if meta.is_segmented_complete() {
                if let Some(s) = session.as_deref_mut() {
                    s.record_response_validator(meta_validator, meta_validator_diagnostic)?;
                }
                let sha = verify_and_hash_part(
                    &part_path(&request.destination),
                    checksum,
                    Some(total_length),
                    IntegrityResponseStatusClass::Partial206,
                    IntegrityResumeState::Resume,
                    Some(total_length),
                    cancelled,
                )?;
                self.finalize_and_promote(
                    &request.destination,
                    final_destination,
                    Some(total_length),
                    validation,
                    sha,
                    cancelled,
                )?;
                self.segmented_transfers_count
                    .fetch_add(1, Ordering::Relaxed);
                self.effective_host_concurrency()
                    .record_clean_completion(&request.url);
                return Ok(SegmentedAttemptOutcome::Completed(success(
                    DownloadOutcome::Completed,
                    total_length,
                    attempt,
                )));
            }
            let etag = parse_entity_tag(meta.etag.as_deref());
            let last_modified = meta.last_modified.as_deref().map(LastModified::new);
            let prior_identity = match (&etag, &last_modified) {
                (EntityTag::Strong(value), _) => StableObjectIdentity::StableStrongEtag {
                    value: value.clone(),
                    length: total_length,
                },
                (_, Some(lm)) => StableObjectIdentity::ConditionalLastModified {
                    value: lm.clone(),
                    length: total_length,
                },
                _ => return Ok(SegmentedAttemptOutcome::FallbackSingleStream),
            };
            let (planning_level, resume_policy) = if requested_level == SegmentLevel::One {
                (
                    SegmentLevel::Two,
                    MaximumSegmentPolicy::AtMost(SegmentLevel::Two),
                )
            } else {
                (requested_level, maximum_segment_policy)
            };
            let decision = plan_segments(&SegmentPlanningInput {
                remote_length: RemoteLength::Known(total_length),
                range_capability: RangeCapability::Proven,
                etag: etag.clone(),
                last_modified: last_modified.clone(),
                frozen_checksum: frozen_checksum.clone(),
                context: SegmentPlanContext::ProcessRestart { prior_identity },
                requested_level: planning_level,
                minimum_segment_bytes: 1,
                maximum_segment_policy: resume_policy,
            });
            let SegmentPlanningDecision::Segmented(plan) = decision else {
                return Ok(SegmentedAttemptOutcome::FallbackSingleStream);
            };
            let segs = meta.segmented.map(|s| s.segments).unwrap_or_else(|| {
                plan.ranges()
                    .iter()
                    .map(|r| SegmentProgressEntry {
                        start: r.start,
                        end: r.end,
                        written: 0,
                    })
                    .collect()
            });
            (plan, segs, None, meta.etag, meta.last_modified)
        } else {
            let provisional = partition_half_open_ranges(total_length, requested_level)
                .map_err(|_| AttemptError::fatal())?;
            let first_range = provisional[0];
            let range_header = format!("bytes={}-{}", first_range.start, first_range.end - 1);
            let if_range = prior_validator.as_ref().and_then(if_range_value);
            let Some(probe_resp) = self.send_with_range_header(
                self.active_streaming_client(),
                &request.url,
                Some(&range_header),
                if_range,
                cancelled,
                |b, u| apply_access(b, u),
                |a, b| redirect_allowed(a, b),
            )?
            else {
                return Ok(SegmentedAttemptOutcome::Completed(interrupted(
                    &request.destination,
                    attempt,
                )));
            };
            let status = probe_resp.status();
            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                return Ok(SegmentedAttemptOutcome::AuthRejected(probe_resp));
            }
            if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                if status == StatusCode::TOO_MANY_REQUESTS {
                    self.effective_host_concurrency()
                        .record_rate_limit(&request.url, requested_level.count() as u8);
                }
                let retry_after =
                    Self::bhoonidhi_aware_retry_after(&request.url, status, probe_resp.headers());
                return Err(AttemptError::transient_for_status(status, retry_after));
            }
            if status == StatusCode::OK {
                return Ok(SegmentedAttemptOutcome::FallbackWithFullResponse(
                    probe_resp,
                ));
            }
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                // Dynamic archive generators (such as CDSE `$zip`) reject speculative
                // multi-segment `Range: bytes=0-...` probes with 416 even at byte 0.
                // Only fall back to single-stream GET on a clean restart when no prior
                // partial resume or response validator is active.
                let has_prior_partial = allow_resume_existing_meta || prior_validator.is_some();
                if !has_prior_partial {
                    return Ok(SegmentedAttemptOutcome::FallbackSingleStream);
                }
                let diagnostic = IntegrityDiagnostic::for_response(
                    IntegritySubtype::RangeNotSatisfiable,
                    status,
                    0,
                    Some(Some(total_length)),
                    IntegritySizeRelation::Unknown,
                );
                return Err(if refused_resume_restarts {
                    AttemptError::resume_refused(diagnostic)
                } else {
                    AttemptError::integrity(false, diagnostic)
                });
            }
            if status.is_client_error() || status.is_redirection() {
                return Err(AttemptError::http_status(status));
            }
            if status != StatusCode::PARTIAL_CONTENT {
                return Ok(SegmentedAttemptOutcome::FallbackSingleStream);
            }

            let etag_raw = probe_resp
                .headers()
                .get(ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let lm_raw = probe_resp
                .headers()
                .get(LAST_MODIFIED)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let accept_ranges_raw = probe_resp
                .headers()
                .get(ACCEPT_RANGES)
                .and_then(|v| v.to_str().ok());
            let parsed_content_range = probe_resp
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range);
            let remote_total = parsed_content_range
                .and_then(|cr| cr.total)
                .or(Some(total_length));
            if remote_total != Some(total_length) {
                return Err(AttemptError::integrity(
                    false,
                    IntegrityDiagnostic::for_response(
                        IntegritySubtype::ExpectedRemoteTotalMismatch,
                        status,
                        0,
                        Some(remote_total),
                        IntegritySizeRelation::Mismatch,
                    ),
                ));
            }

            let etag = parse_entity_tag(etag_raw.as_deref());
            let last_modified = lm_raw.as_deref().map(LastModified::new);
            let _ = accept_ranges_supports_bytes(accept_ranges_raw);
            let decision = plan_segments(&SegmentPlanningInput {
                remote_length: RemoteLength::Known(total_length),
                range_capability: RangeCapability::Proven,
                etag: etag.clone(),
                last_modified: last_modified.clone(),
                frozen_checksum,
                context: SegmentPlanContext::Fresh,
                requested_level,
                minimum_segment_bytes,
                maximum_segment_policy,
            });
            let SegmentPlanningDecision::Segmented(plan) = decision else {
                return Ok(SegmentedAttemptOutcome::FallbackSingleStream);
            };

            let evidence = SegmentResponseEvidence {
                status: SegmentResponseStatus::PartialContent,
                content_range: parsed_content_range,
                content_length: probe_resp.content_length(),
                etag,
                last_modified,
            };
            match validate_segment_response(&plan, &plan.ranges()[0], &evidence) {
                SegmentResponseDecision::AcceptSegment => {}
                SegmentResponseDecision::RetrySameSegment => {
                    return Err(AttemptError::segment_range_retry());
                }
                SegmentResponseDecision::RestartNativeSingleStream(_) => {
                    return Ok(SegmentedAttemptOutcome::FallbackSingleStream);
                }
                SegmentResponseDecision::FailRemoteIdentityConflict(_) => {
                    return Err(AttemptError::integrity(
                        false,
                        IntegrityDiagnostic::for_response(
                            IntegritySubtype::ValidatorMismatch,
                            status,
                            0,
                            Some(Some(total_length)),
                            IntegritySizeRelation::Equal,
                        ),
                    ));
                }
                SegmentResponseDecision::FailMalformedEvidence(_) => {
                    return Err(AttemptError::integrity(
                        false,
                        IntegrityDiagnostic::for_response(
                            IntegritySubtype::ResponsePreparation,
                            status,
                            0,
                            Some(Some(total_length)),
                            IntegritySizeRelation::Equal,
                        ),
                    ));
                }
                SegmentResponseDecision::FailTerminalResponse(_) => {
                    return Err(AttemptError::fatal());
                }
            }
            let probe_validator = response_validator(&probe_resp, Some(total_length));
            let validator_diagnostic = IntegrityDiagnostic::for_response(
                IntegritySubtype::ValidatorMismatch,
                status,
                0,
                Some(Some(total_length)),
                IntegritySizeRelation::Equal,
            );
            if let Some(expected_validator) = expected_remote_validator {
                ensure_response_validator_compatible(
                    expected_validator,
                    &probe_validator,
                    validator_diagnostic,
                )?;
            }
            if let Some(s) = session.as_deref_mut() {
                s.record_response_validator(probe_validator, validator_diagnostic)?;
            }
            let segs: Vec<SegmentProgressEntry> = plan
                .ranges()
                .iter()
                .map(|r| SegmentProgressEntry {
                    start: r.start,
                    end: r.end,
                    written: 0,
                })
                .collect();
            (plan, segs, Some(probe_resp), etag_raw, lm_raw)
        };

        let is_resume = probe_response_for_zero.is_none();
        let initial_bytes_completed: u64 = initial_segments
            .iter()
            .map(|s| s.written.min(s.end.saturating_sub(s.start)))
            .sum();
        let Some(pool) = DynamicSegmentPool::new(
            total_length,
            initial_segments,
            self.engine_config.min_split_chunk_bytes,
            probe_response_for_zero.is_some(),
        ) else {
            return Ok(SegmentedAttemptOutcome::FallbackSingleStream);
        };
        let pool = Arc::new(pool);

        let part_file =
            open_part_preallocated_for_segmented(&request.destination, total_length, !is_resume)
                .map_err(AttemptError::file)?;
        if let Some(s) = session.as_deref_mut() {
            s.mark_part_opened();
        }

        let meta_save_lock = Arc::new(Mutex::new(()));
        {
            let _save_guard = meta_save_lock.lock();
            let _ = save_part_meta_checkpoint(
                &request.destination,
                &part_file,
                &url_digest,
                response_etag.clone(),
                response_last_modified.clone(),
                Some(pool.snapshot_meta()),
            );
        }

        let worker_count_usize = usize::try_from(requested_level.count())
            .unwrap_or(1)
            .clamp(1, 8);
        let worker_count = worker_count_usize as u64;
        let limiter = self.effective_bandwidth_limiter();
        let speed_tracker = Arc::new(RelativeSpeedTracker::new(
            worker_count_usize,
            self.engine_config.relative_speed_config.clone(),
            limiter.limit_generation(),
        ));
        let total_completed = Arc::new(AtomicU64::new(initial_bytes_completed));
        let segment_stall_reconnects = Arc::new(AtomicU64::new(0));
        let shared_file = Arc::new(part_file);
        let worker_abort = Arc::new(AtomicBool::new(false));
        let rate_limit_hit = Arc::new(AtomicBool::new(false));
        let worker_error: Arc<Mutex<Option<SegmentedWorkerOutcome>>> = Arc::new(Mutex::new(None));
        let done_workers = Arc::new(AtomicU64::new(0));

        let if_range_val = response_etag
            .as_deref()
            .filter(|v| !v.starts_with("W/") && !v.starts_with("w/"))
            .or(response_last_modified.as_deref())
            .map(str::to_owned);
        let worker_prior_validator: Option<RemoteValidator> = session
            .as_deref()
            .and_then(|s| s.prior_response_validator())
            .cloned();
        let observed_live_validator: Arc<Mutex<Option<(RemoteValidator, IntegrityDiagnostic)>>> =
            Arc::new(Mutex::new(None));

        progress(TransferUpdate::BodyStarted {
            attempt,
            bytes_completed: initial_bytes_completed,
        });

        thread::scope(|scope| {
            let mut probe_zero = probe_response_for_zero.take();
            for worker_id in 0..worker_count_usize {
                let initial_resp = if worker_id == 0 {
                    probe_zero.take()
                } else {
                    None
                };
                let pool = Arc::clone(&pool);
                let meta_save_lock = Arc::clone(&meta_save_lock);
                let speed_tracker = Arc::clone(&speed_tracker);
                let total_completed = total_completed.clone();
                let segment_stall_reconnects = segment_stall_reconnects.clone();
                let shared_file = shared_file.clone();
                let worker_abort = worker_abort.clone();
                let rate_limit_hit = rate_limit_hit.clone();
                let worker_error = worker_error.clone();
                let done_workers = done_workers.clone();
                let observed_live_validator = Arc::clone(&observed_live_validator);
                let plan_ref = &plan;
                let dest_ref = &request.destination;
                let url_ref = &request.url;
                let etag_ref = response_etag.as_deref();
                let lm_ref = response_last_modified.as_deref();
                let url_digest_ref = &url_digest;
                let if_range_ref = if_range_val.as_deref();
                let prior_validator_ref = worker_prior_validator.as_ref();

                scope.spawn(move || {
                    let outcome = self.run_segment_worker(
                        worker_id,
                        worker_count_usize,
                        is_resume,
                        refused_resume_restarts,
                        initial_resp,
                        plan_ref,
                        dest_ref,
                        url_ref,
                        etag_ref,
                        lm_ref,
                        expected_remote_validator,
                        prior_validator_ref,
                        &observed_live_validator,
                        url_digest_ref,
                        if_range_ref,
                        &shared_file,
                        &pool,
                        &meta_save_lock,
                        &speed_tracker,
                        &total_completed,
                        &segment_stall_reconnects,
                        &worker_abort,
                        &rate_limit_hit,
                        cancelled,
                        paused,
                        apply_access,
                        redirect_allowed,
                    );
                    if let Err(err) = outcome {
                        worker_abort.store(true, Ordering::Release);
                        if let Ok(mut slot) = worker_error.lock() {
                            if slot.is_none() {
                                *slot = Some(err);
                            }
                        }
                    }
                    done_workers.fetch_add(1, Ordering::Release);
                });
            }

            let mut last_reported = initial_bytes_completed;
            let mut last_reported_stalls: u64 = 0;
            while done_workers.load(Ordering::Acquire) < worker_count {
                let current = total_completed.load(Ordering::Relaxed);
                if current > last_reported {
                    last_reported = current;
                    progress(TransferUpdate::Progress(TransferProgress {
                        bytes_completed: current,
                        total_bytes: Some(total_length),
                        attempt,
                    }));
                }
                let stalls = segment_stall_reconnects.load(Ordering::Relaxed);
                while last_reported_stalls < stalls {
                    last_reported_stalls += 1;
                    progress(TransferUpdate::StallReconnect { attempt });
                }
                thread::sleep(Duration::from_millis(20));
            }
            let final_cur = total_completed.load(Ordering::Relaxed);
            if final_cur > last_reported {
                progress(TransferUpdate::Progress(TransferProgress {
                    bytes_completed: final_cur,
                    total_bytes: Some(total_length),
                    attempt,
                }));
            }
            let final_stalls = segment_stall_reconnects.load(Ordering::Relaxed);
            while last_reported_stalls < final_stalls {
                last_reported_stalls += 1;
                progress(TransferUpdate::StallReconnect { attempt });
            }
        });

        progress(TransferUpdate::BodyStopped { attempt });

        self.dynamic_segment_splits_count
            .fetch_add(pool.split_count(), Ordering::Relaxed);

        if !cancelled() {
            shared_file.sync_data().map_err(AttemptError::file_io)?;
        }
        {
            let _save_guard = meta_save_lock.lock();
            let _ = save_part_meta_checkpoint(
                &request.destination,
                &shared_file,
                &url_digest,
                response_etag.clone(),
                response_last_modified.clone(),
                Some(pool.snapshot_meta()),
            );
        }
        // Release the `.part` file handle immediately after checkpointing so
        // cancel/pause paths on Windows never leave a multi-GB handle open while
        // reporting results or winding down errors.
        drop(shared_file);

        if rate_limit_hit.load(Ordering::Acquire) {
            self.effective_host_concurrency()
                .record_rate_limit(&request.url, requested_level.count() as u8);
        }

        if let Some((live_val, live_diag)) = observed_live_validator
            .lock()
            .ok()
            .and_then(|mut guard| guard.take())
        {
            if let Some(s) = session {
                s.record_response_validator(live_val, live_diag)?;
            }
        }

        if let Ok(mut slot) = worker_error.lock() {
            if let Some(err) = slot.take() {
                return match err {
                    SegmentedWorkerOutcome::Attempt(err) => Err(err),
                    SegmentedWorkerOutcome::FallbackSingleStream => {
                        Ok(SegmentedAttemptOutcome::FallbackSingleStream)
                    }
                    SegmentedWorkerOutcome::AuthRejected(resp) => {
                        Ok(SegmentedAttemptOutcome::AuthRejected(*resp))
                    }
                };
            }
        }

        if cancelled() {
            return Ok(SegmentedAttemptOutcome::Completed(interrupted(
                &request.destination,
                attempt,
            )));
        }

        if !pool.is_all_complete() {
            return Err(AttemptError::integrity(
                false,
                IntegrityDiagnostic::for_response(
                    IntegritySubtype::BodyFinalSizeMismatch,
                    StatusCode::PARTIAL_CONTENT,
                    total_completed.load(Ordering::Relaxed),
                    Some(Some(total_length)),
                    IntegritySizeRelation::Mismatch,
                ),
            ));
        }

        let promotion_sha256 = verify_and_hash_part(
            &part_path(&request.destination),
            checksum,
            Some(total_length),
            IntegrityResponseStatusClass::Partial206,
            if is_resume {
                IntegrityResumeState::Resume
            } else {
                IntegrityResumeState::Fresh
            },
            Some(total_length),
            cancelled,
        )?;
        self.finalize_and_promote(
            &request.destination,
            final_destination,
            Some(total_length),
            validation,
            promotion_sha256,
            cancelled,
        )?;
        self.segmented_transfers_count
            .fetch_add(1, Ordering::Relaxed);
        self.effective_host_concurrency()
            .record_clean_completion(&request.url);
        Ok(SegmentedAttemptOutcome::Completed(success(
            DownloadOutcome::Completed,
            total_length,
            attempt,
        )))
    }

    #[allow(clippy::too_many_arguments)]
    fn run_segment_worker(
        &self,
        worker_id: usize,
        worker_count: usize,
        is_resume: bool,
        refused_resume_restarts: bool,
        mut pending_response: Option<Response>,
        plan: &SegmentPlan,
        destination: &Path,
        initial_url: &Url,
        expected_etag: Option<&str>,
        expected_lm: Option<&str>,
        expected_remote_validator: Option<&RemoteValidator>,
        prior_validator: Option<&RemoteValidator>,
        observed_live_validator: &Arc<Mutex<Option<(RemoteValidator, IntegrityDiagnostic)>>>,
        url_digest: &str,
        if_range: Option<&str>,
        shared_file: &Arc<fs::File>,
        pool: &Arc<DynamicSegmentPool>,
        meta_save_lock: &Arc<Mutex<()>>,
        speed_tracker: &Arc<RelativeSpeedTracker>,
        total_completed: &Arc<AtomicU64>,
        segment_stall_reconnects: &Arc<AtomicU64>,
        worker_abort: &Arc<AtomicBool>,
        rate_limit_hit: &Arc<AtomicBool>,
        cancelled: &(dyn Fn() -> bool + Sync),
        paused: &(dyn Fn() -> bool + Sync),
        apply_access: &(dyn Fn(RequestBuilder, &Url) -> RequestBuilder + Sync),
        redirect_allowed: &(dyn Fn(&Url, &Url) -> bool + Sync),
    ) -> Result<(), SegmentedWorkerOutcome> {
        let mut stall_reconnects: u32 = 0;
        let mut zero_progress_stalls: u32 = 0;
        let mut buf = vec![0_u8; self.options.chunk_size.max(64 * 1024)];
        let mut last_meta_save = Instant::now();
        let limiter = self.effective_bandwidth_limiter();
        let save_snapshot_once = || {
            let _save_guard = meta_save_lock.lock();
            let _ = save_part_meta_checkpoint(
                destination,
                shared_file,
                url_digest,
                expected_etag.map(str::to_owned),
                expected_lm.map(str::to_owned),
                Some(pool.snapshot_meta()),
            );
        };

        'worker: loop {
            if cancelled() || worker_abort.load(Ordering::Acquire) {
                return Ok(());
            }
            if paused() {
                let _ = shared_file.sync_data();
                save_snapshot_once();
                while paused() {
                    if cancelled() || worker_abort.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }

            let Some(acquired) = pool.acquire_or_split(worker_id) else {
                break 'worker;
            };
            let seg_id = acquired.seg_id;
            let mut partial_eof_retries: u32 = 0;

            'segment: loop {
                if cancelled() || worker_abort.load(Ordering::Acquire) {
                    pool.release_segment(seg_id, worker_id, false);
                    return Ok(());
                }
                let Some((offset, req_end, written_now, seg_shed_count)) =
                    pool.prepare_segment_request(seg_id)
                else {
                    pool.release_segment(seg_id, worker_id, false);
                    break 'segment;
                };

                let open_probe_resp = if worker_id == 0 && seg_id == 0 && written_now == 0 {
                    pending_response.take()
                } else {
                    pending_response = None;
                    None
                };
                let overlap = if open_probe_resp.is_some() {
                    0
                } else {
                    written_now.min(self.engine_config.segmented_resume_overlap_bytes)
                };
                let req_start = offset - overlap;

                let response = if let Some(resp) = open_probe_resp {
                    resp
                } else {
                    let range_header = format!("bytes={req_start}-{}", req_end - 1);
                    let Some(resp) = self
                        .send_with_range_header(
                            self.active_streaming_client(),
                            initial_url,
                            Some(&range_header),
                            if_range,
                            cancelled,
                            |b, u| apply_access(b, u),
                            |a, b| redirect_allowed(a, b),
                        )
                        .map_err(|err| {
                            pool.release_segment(seg_id, worker_id, false);
                            SegmentedWorkerOutcome::Attempt(err)
                        })?
                    else {
                        pool.release_segment(seg_id, worker_id, false);
                        return Ok(());
                    };
                    let status = resp.status();
                    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                        pool.release_segment(seg_id, worker_id, false);
                        return Err(SegmentedWorkerOutcome::AuthRejected(Box::new(resp)));
                    }
                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        if status == StatusCode::TOO_MANY_REQUESTS
                            || (status == StatusCode::SERVICE_UNAVAILABLE && worker_count > 1)
                        {
                            rate_limit_hit.store(true, Ordering::Release);
                        }
                        let retry_after =
                            Self::bhoonidhi_aware_retry_after(initial_url, status, resp.headers());
                        pool.release_segment(seg_id, worker_id, false);
                        return Err(SegmentedWorkerOutcome::Attempt(
                            AttemptError::transient_for_status(status, retry_after),
                        ));
                    }
                    if status == StatusCode::OK {
                        pool.release_segment(seg_id, worker_id, false);
                        if is_resume || written_now > 0 {
                            let diagnostic = IntegrityDiagnostic::for_response(
                                IntegritySubtype::ValidatorMismatch,
                                status,
                                offset,
                                Some(Some(plan.total_length())),
                                IntegritySizeRelation::Unknown,
                            );
                            return Err(SegmentedWorkerOutcome::Attempt(
                                if refused_resume_restarts {
                                    AttemptError::resume_refused(diagnostic)
                                } else {
                                    AttemptError::integrity(false, diagnostic)
                                },
                            ));
                        }
                        return Err(SegmentedWorkerOutcome::FallbackSingleStream);
                    }
                    if status == StatusCode::RANGE_NOT_SATISFIABLE {
                        pool.release_segment(seg_id, worker_id, false);
                        let diagnostic = IntegrityDiagnostic::for_response(
                            IntegritySubtype::RangeNotSatisfiable,
                            status,
                            offset,
                            Some(Some(plan.total_length())),
                            IntegritySizeRelation::Unknown,
                        );
                        return Err(SegmentedWorkerOutcome::Attempt(
                            if refused_resume_restarts && (is_resume || written_now > 0) {
                                AttemptError::resume_refused(diagnostic)
                            } else {
                                AttemptError::integrity(false, diagnostic)
                            },
                        ));
                    }
                    if status != StatusCode::PARTIAL_CONTENT {
                        pool.release_segment(seg_id, worker_id, false);
                        return Err(SegmentedWorkerOutcome::Attempt(AttemptError::http_status(
                            status,
                        )));
                    }
                    let parsed_cr = resp
                        .headers()
                        .get(CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        .and_then(parse_content_range);
                    let Some(cr) = parsed_cr else {
                        pool.release_segment(seg_id, worker_id, false);
                        return Err(SegmentedWorkerOutcome::Attempt(AttemptError::integrity(
                            false,
                            IntegrityDiagnostic::for_response(
                                IntegritySubtype::ResponsePreparation,
                                status,
                                offset,
                                Some(Some(plan.total_length())),
                                IntegritySizeRelation::Unknown,
                            ),
                        )));
                    };
                    if cr.start != req_start
                        || cr.end_inclusive != req_end - 1
                        || cr.total != Some(plan.total_length())
                    {
                        pool.release_segment(seg_id, worker_id, false);
                        return Err(SegmentedWorkerOutcome::Attempt(AttemptError::integrity(
                            false,
                            IntegrityDiagnostic::for_response(
                                IntegritySubtype::ValidatorMismatch,
                                status,
                                offset,
                                Some(cr.total),
                                IntegritySizeRelation::Mismatch,
                            ),
                        )));
                    }
                    let cur_etag = resp.headers().get(ETAG).and_then(|v| v.to_str().ok());
                    if expected_etag.is_some() && cur_etag != expected_etag {
                        pool.release_segment(seg_id, worker_id, false);
                        return Err(SegmentedWorkerOutcome::Attempt(AttemptError::integrity(
                            false,
                            IntegrityDiagnostic::for_response(
                                IntegritySubtype::ValidatorMismatch,
                                status,
                                offset,
                                Some(cr.total),
                                IntegritySizeRelation::Equal,
                            ),
                        )));
                    }
                    let cur_lm = resp
                        .headers()
                        .get(LAST_MODIFIED)
                        .and_then(|v| v.to_str().ok());
                    if expected_etag.is_none() && expected_lm.is_some() && cur_lm != expected_lm {
                        pool.release_segment(seg_id, worker_id, false);
                        return Err(SegmentedWorkerOutcome::Attempt(AttemptError::integrity(
                            false,
                            IntegrityDiagnostic::for_response(
                                IntegritySubtype::ValidatorMismatch,
                                status,
                                offset,
                                Some(cr.total),
                                IntegritySizeRelation::Equal,
                            ),
                        )));
                    }
                    let worker_validator = response_validator(&resp, Some(plan.total_length()));
                    let worker_diag = IntegrityDiagnostic::for_response(
                        IntegritySubtype::ValidatorMismatch,
                        status,
                        offset,
                        Some(cr.total),
                        IntegritySizeRelation::Equal,
                    );
                    if let Some(expected_val) = expected_remote_validator {
                        if let Err(err) = ensure_response_validator_compatible(
                            expected_val,
                            &worker_validator,
                            worker_diag,
                        ) {
                            pool.release_segment(seg_id, worker_id, false);
                            return Err(SegmentedWorkerOutcome::Attempt(err));
                        }
                    }
                    if let Some(prior_val) = prior_validator {
                        if let Err(err) = ensure_response_validator_compatible(
                            prior_val,
                            &worker_validator,
                            worker_diag,
                        ) {
                            pool.release_segment(seg_id, worker_id, false);
                            return Err(SegmentedWorkerOutcome::Attempt(err));
                        }
                    }
                    if let Ok(mut live_guard) = observed_live_validator.lock() {
                        if let Some((prev_live, _)) = live_guard.as_mut() {
                            if let Err(err) = ensure_response_validator_compatible(
                                prev_live,
                                &worker_validator,
                                worker_diag,
                            ) {
                                pool.release_segment(seg_id, worker_id, false);
                                return Err(SegmentedWorkerOutcome::Attempt(err));
                            }
                            if prev_live.etag.is_none() {
                                prev_live.etag = worker_validator.etag;
                            }
                            if prev_live.last_modified.is_none() {
                                prev_live.last_modified = worker_validator.last_modified;
                            }
                            if prev_live.content_length.is_none() {
                                prev_live.content_length = worker_validator.content_length;
                            }
                        } else {
                            *live_guard = Some((worker_validator, worker_diag));
                        }
                    }
                    resp
                };

                let response_status = response.status();
                let expected_overlap = if overlap > 0 {
                    let overlap_len = usize::try_from(overlap).map_err(|_| {
                        pool.release_segment(seg_id, worker_id, false);
                        SegmentedWorkerOutcome::Attempt(AttemptError::integrity(
                            false,
                            IntegrityDiagnostic::for_response(
                                IntegritySubtype::ResponsePreparation,
                                response_status,
                                offset,
                                Some(Some(plan.total_length())),
                                IntegritySizeRelation::Unknown,
                            ),
                        ))
                    })?;
                    let mut expected_buf = vec![0_u8; overlap_len];
                    read_exact_at(shared_file, &mut expected_buf, req_start).map_err(|err| {
                        pool.release_segment(seg_id, worker_id, false);
                        SegmentedWorkerOutcome::Attempt(AttemptError::file_io(err))
                    })?;
                    expected_buf
                } else {
                    Vec::new()
                };

                let mut conn_started = Instant::now();
                let mut watchdog = StallWatchdog::new(
                    self.effective_stall_enabled(),
                    self.engine_config.stall_min_bps,
                    self.engine_config.stall_window,
                );
                let mut reader = WatchdogStreamReader::new(
                    response,
                    watchdog.enabled,
                    buf.len(),
                    self.engine_config.stall_window,
                );
                let is_direct_reader = reader.is_direct();
                let mut stream_guard = Some(limiter.begin_stream());

                if !expected_overlap.is_empty() {
                    let mut on_pause = || -> Result<(), AttemptError> {
                        save_snapshot_once();
                        let _ = shared_file.sync_data();
                        Ok(())
                    };
                    match self.verify_stream_overlap(
                        &mut reader,
                        &expected_overlap,
                        response_status,
                        offset,
                        Some(plan.total_length()),
                        &mut watchdog,
                        limiter,
                        &mut stream_guard,
                        Some(worker_abort),
                        cancelled,
                        paused,
                        &mut on_pause,
                    ) {
                        Ok(true) => {}
                        Ok(false) => {
                            drop(stream_guard);
                            pool.release_segment(seg_id, worker_id, false);
                            return Ok(());
                        }
                        Err(err) if err.stall_resume_offset.is_some() => {
                            drop(stream_guard);
                            drop(reader);
                            save_snapshot_once();
                            let _ = shared_file.sync_data();
                            zero_progress_stalls += 1;
                            if stall_reconnects > self.engine_config.max_stall_reconnects
                                || zero_progress_stalls > 2
                            {
                                pool.release_segment(seg_id, worker_id, false);
                                return Err(SegmentedWorkerOutcome::Attempt(
                                    AttemptError::stall_exhausted(
                                        stall_reconnects,
                                        zero_progress_stalls,
                                    ),
                                ));
                            }
                            self.stall_reconnect_count.fetch_add(1, Ordering::Relaxed);
                            segment_stall_reconnects.fetch_add(1, Ordering::Relaxed);
                            thread::sleep(Duration::from_millis(25));
                            continue 'segment;
                        }
                        Err(err) => {
                            drop(stream_guard);
                            pool.release_segment(seg_id, worker_id, false);
                            return Err(SegmentedWorkerOutcome::Attempt(err));
                        }
                    }
                    conn_started = Instant::now();
                }

                let mut stalled_this_stream = false;
                let mut shed_this_stream = false;
                let mut disk_write_elapsed = Duration::ZERO;
                let mut bytes_this_conn: u64 = 0;
                let mut bytes_written_this_stream: u64 = 0;

                loop {
                    if cancelled() || worker_abort.load(Ordering::Acquire) {
                        drop(stream_guard);
                        pool.release_segment(seg_id, worker_id, false);
                        return Ok(());
                    }
                    if paused() {
                        drop(stream_guard.take());
                        speed_tracker.clear_worker(worker_id);
                        save_snapshot_once();
                        let _ = shared_file.sync_data();
                        while paused() {
                            if cancelled() || worker_abort.load(Ordering::Acquire) {
                                pool.release_segment(seg_id, worker_id, false);
                                return Ok(());
                            }
                            thread::sleep(Duration::from_millis(50));
                        }
                        watchdog.reset();
                        conn_started = Instant::now();
                        disk_write_elapsed = Duration::ZERO;
                        bytes_this_conn = 0;
                        stream_guard = Some(limiter.begin_stream());
                    }

                    let live_remaining = pool.live_remaining(seg_id);
                    if live_remaining == 0 {
                        break;
                    }
                    let max_read =
                        usize::try_from(live_remaining.min(buf.len() as u64)).unwrap_or(buf.len());
                    let acquire_started = (limiter.limit_bps() > 0).then(Instant::now);
                    let grant =
                        match limiter.acquire(max_read, cancelled, paused, Some(worker_abort)) {
                            AcquireOutcome::Granted(n) => n,
                            AcquireOutcome::Paused => continue,
                            AcquireOutcome::Cancelled => {
                                drop(stream_guard);
                                pool.release_segment(seg_id, worker_id, false);
                                return Ok(());
                            }
                        };
                    if let Some(started) = acquire_started {
                        watchdog.extend_for_pacing(started.elapsed());
                    }
                    let count = match reader.read(&mut buf[..grant]) {
                        Ok(0) => {
                            limiter.refund(grant);
                            break;
                        }
                        Ok(n) => {
                            limiter.refund(grant.saturating_sub(n));
                            n
                        }
                        Err(ref err) if watchdog.enabled && is_io_timeout(err) => {
                            limiter.refund(grant);
                            if watchdog.is_stalled(
                                Instant::now(),
                                limiter.effective_stall_min_bps(self.engine_config.stall_min_bps),
                            ) {
                                stalled_this_stream = true;
                                break;
                            }
                            continue;
                        }
                        Err(err) => {
                            limiter.refund(grant);
                            drop(stream_guard);
                            speed_tracker.clear_worker(worker_id);
                            pool.release_segment(seg_id, worker_id, false);
                            return Err(SegmentedWorkerOutcome::Attempt(
                                AttemptError::stream_read(&err),
                            ));
                        }
                    };
                    let claim = pool.claim_write(seg_id, count as u64);
                    if claim.allowed > 0 {
                        let allowed_usize = claim.allowed as usize;
                        let write_start = Instant::now();
                        if let Err(err) =
                            write_all_at(shared_file, &buf[..allowed_usize], claim.write_offset)
                        {
                            pool.rollback_claimed(seg_id, claim.allowed);
                            drop(stream_guard);
                            speed_tracker.clear_worker(worker_id);
                            pool.release_segment(seg_id, worker_id, false);
                            return Err(SegmentedWorkerOutcome::Attempt(AttemptError::file_io(
                                err,
                            )));
                        }
                        if is_direct_reader {
                            disk_write_elapsed += write_start.elapsed();
                        }
                        pool.commit_written(seg_id, claim.allowed);
                        total_completed.fetch_add(claim.allowed, Ordering::Relaxed);
                        bytes_this_conn = bytes_this_conn.saturating_add(claim.allowed);
                        bytes_written_this_stream =
                            bytes_written_this_stream.saturating_add(claim.allowed);

                        if last_meta_save.elapsed() >= Duration::from_secs(5) {
                            save_snapshot_once();
                            last_meta_save = Instant::now();
                        }
                    }

                    watchdog.record(claim.allowed, Instant::now());
                    if claim.hit_shrunk_end {
                        break;
                    }
                    if watchdog.is_stalled(
                        Instant::now(),
                        limiter.effective_stall_min_bps(self.engine_config.stall_min_bps),
                    ) {
                        stalled_this_stream = true;
                        break;
                    }

                    if self.engine_config.slow_shed_enabled && worker_count > 1 {
                        let active_elapsed =
                            conn_started.elapsed().saturating_sub(if is_direct_reader {
                                disk_write_elapsed
                            } else {
                                Duration::ZERO
                            });
                        if speed_tracker.report_and_check_shed(
                            worker_id,
                            bytes_this_conn,
                            active_elapsed,
                            pool.live_remaining(seg_id),
                            seg_shed_count,
                            limiter.limit_bps(),
                            limiter.limit_generation(),
                            worker_count,
                        ) {
                            shed_this_stream = true;
                            break;
                        }
                    }
                }

                let final_active_elapsed =
                    conn_started.elapsed().saturating_sub(if is_direct_reader {
                        disk_write_elapsed
                    } else {
                        Duration::ZERO
                    });
                drop(stream_guard);
                drop(reader);

                if shed_this_stream {
                    speed_tracker.clear_worker(worker_id);
                    pool.release_segment(seg_id, worker_id, true);
                    self.slow_connection_shed_count
                        .fetch_add(1, Ordering::Relaxed);
                    save_snapshot_once();
                    thread::sleep(Duration::from_millis(25));
                    break 'segment;
                }

                if stalled_this_stream {
                    speed_tracker.clear_worker(worker_id);
                    save_snapshot_once();
                    let _ = shared_file.sync_data();
                    if bytes_written_this_stream > 0 {
                        zero_progress_stalls = 0;
                        stall_reconnects += 1;
                    } else {
                        zero_progress_stalls += 1;
                    }
                    if stall_reconnects > self.engine_config.max_stall_reconnects
                        || zero_progress_stalls > 2
                    {
                        pool.release_segment(seg_id, worker_id, false);
                        return Err(SegmentedWorkerOutcome::Attempt(
                            AttemptError::stall_exhausted(stall_reconnects, zero_progress_stalls),
                        ));
                    }
                    self.stall_reconnect_count.fetch_add(1, Ordering::Relaxed);
                    segment_stall_reconnects.fetch_add(1, Ordering::Relaxed);
                    thread::sleep(Duration::from_millis(25));
                    continue 'segment;
                }

                if pool.prepare_segment_request(seg_id).is_none() {
                    speed_tracker.record_completed_stream(
                        worker_id,
                        bytes_this_conn,
                        final_active_elapsed,
                        limiter.limit_bps(),
                        limiter.limit_generation(),
                    );
                    pool.release_segment(seg_id, worker_id, false);
                    break 'segment;
                }

                speed_tracker.clear_worker(worker_id);
                if bytes_written_this_stream == 0 || partial_eof_retries >= 8 {
                    pool.release_segment(seg_id, worker_id, false);
                    return Err(SegmentedWorkerOutcome::Attempt(AttemptError::integrity(
                        false,
                        IntegrityDiagnostic::for_response(
                            IntegritySubtype::BodyFinalSizeMismatch,
                            StatusCode::PARTIAL_CONTENT,
                            offset,
                            Some(Some(plan.total_length())),
                            IntegritySizeRelation::Mismatch,
                        ),
                    )));
                }
                partial_eof_retries += 1;
                save_snapshot_once();
            }
        }
        Ok(())
    }
}

enum SegmentedAttemptOutcome {
    NotApplicable,
    Completed(TransferResult),
    FallbackWithFullResponse(Response),
    FallbackSingleStream,
    AuthRejected(Response),
}

enum SegmentedWorkerOutcome {
    Attempt(AttemptError),
    FallbackSingleStream,
    AuthRejected(Box<Response>),
}

enum WatchdogStreamReader {
    Direct(Response),
    Channel {
        rx: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
        pending: Vec<u8>,
        pending_offset: usize,
        poll_interval: Duration,
    },
}

impl WatchdogStreamReader {
    fn new(response: Response, watchdog_enabled: bool, buf_cap: usize, window: Duration) -> Self {
        if !watchdog_enabled {
            return Self::Direct(response);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<std::io::Result<Vec<u8>>>(2);
        let read_cap = buf_cap.clamp(4 * 1024, 1024 * 1024);
        thread::spawn(move || {
            let mut resp = response;
            let mut local = vec![0_u8; read_cap];
            loop {
                match resp.read(&mut local) {
                    Ok(0) => {
                        let _ = tx.send(Ok(Vec::new()));
                        break;
                    }
                    Ok(n) => {
                        if tx.send(Ok(local[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err));
                        break;
                    }
                }
            }
        });
        let poll_interval =
            (window / 3).clamp(Duration::from_millis(15), Duration::from_millis(250));
        Self::Channel {
            rx,
            pending: Vec::new(),
            pending_offset: 0,
            poll_interval,
        }
    }

    fn is_direct(&self) -> bool {
        matches!(self, Self::Direct(_))
    }
}

impl Read for WatchdogStreamReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Direct(resp) => resp.read(out),
            Self::Channel {
                rx,
                pending,
                pending_offset,
                poll_interval,
            } => {
                if *pending_offset < pending.len() {
                    let avail = &pending[*pending_offset..];
                    let take = avail.len().min(out.len());
                    out[..take].copy_from_slice(&avail[..take]);
                    *pending_offset += take;
                    if *pending_offset >= pending.len() {
                        pending.clear();
                        *pending_offset = 0;
                    }
                    return Ok(take);
                }
                match rx.recv_timeout(*poll_interval) {
                    Ok(Ok(chunk)) => {
                        if chunk.is_empty() {
                            return Ok(0);
                        }
                        let take = chunk.len().min(out.len());
                        out[..take].copy_from_slice(&chunk[..take]);
                        if take < chunk.len() {
                            *pending = chunk;
                            *pending_offset = take;
                        }
                        Ok(take)
                    }
                    Ok(Err(err)) => Err(err),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "watchdog poll interval elapsed",
                    )),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Ok(0),
                }
            }
        }
    }
}

fn is_io_timeout(err: &std::io::Error) -> bool {
    if matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        return true;
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = err
        .get_ref()
        .map(|inner| inner as &(dyn std::error::Error + 'static));
    while let Some(inner) = source {
        if let Some(req_err) = inner.downcast_ref::<reqwest::Error>() {
            if req_err.is_timeout() {
                return true;
            }
        }
        source = inner.source();
    }
    false
}

struct StallWatchdog {
    enabled: bool,
    window: Duration,
    started_at: Instant,
    samples: VecDeque<(Instant, u64)>,
    window_bytes: u64,
}

impl StallWatchdog {
    fn new(enabled: bool, _min_bps: u64, window: Duration) -> Self {
        Self {
            enabled,
            window,
            started_at: Instant::now(),
            samples: VecDeque::new(),
            window_bytes: 0,
        }
    }

    fn reset(&mut self) {
        self.started_at = Instant::now();
        self.samples.clear();
        self.window_bytes = 0;
    }

    fn extend_for_pacing(&mut self, elapsed: Duration) {
        if !self.enabled || elapsed.is_zero() {
            return;
        }
        let now = Instant::now();
        self.started_at = self.started_at.checked_add(elapsed).unwrap_or(now).min(now);
        for (ts, _) in &mut self.samples {
            *ts = ts.checked_add(elapsed).unwrap_or(now).min(now);
        }
    }

    fn prune(&mut self, now: Instant) {
        let Some(cutoff) = now.checked_sub(self.window) else {
            return;
        };
        while let Some(&(ts, bytes)) = self.samples.front() {
            if ts < cutoff {
                self.samples.pop_front();
                self.window_bytes = self.window_bytes.saturating_sub(bytes);
            } else {
                break;
            }
        }
    }

    fn record(&mut self, bytes: u64, now: Instant) {
        if bytes == 0 {
            return;
        }
        self.prune(now);
        if let Some((last_ts, last_bytes)) = self.samples.back_mut() {
            if now.saturating_duration_since(*last_ts) < Duration::from_millis(25) {
                *last_bytes = last_bytes.saturating_add(bytes);
                self.window_bytes = self.window_bytes.saturating_add(bytes);
                return;
            }
        }
        self.samples.push_back((now, bytes));
        self.window_bytes = self.window_bytes.saturating_add(bytes);
    }

    fn is_stalled(&mut self, now: Instant, effective_min_bps: u64) -> bool {
        if !self.enabled {
            return false;
        }
        self.prune(now);
        if now.saturating_duration_since(self.started_at) < self.window {
            return false;
        }
        let window_ms = self.window.as_millis().max(1);
        let required = (u128::from(effective_min_bps) * window_ms) / 1000;
        let required = required.max(u128::from(u64::from(effective_min_bps > 0)));
        u128::from(self.window_bytes) < required
    }
}

fn verify_and_hash_part(
    part: &Path,
    checksum: Option<&ExpectedChecksum>,
    expected_size: Option<u64>,
    response_status: IntegrityResponseStatusClass,
    resume_state: IntegrityResumeState,
    remote_total: Option<u64>,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<String, AttemptError> {
    let mut file = open_checked_download_read(part).map_err(AttemptError::file)?;
    let actual_size = file.metadata().map_err(AttemptError::file_io)?.len();
    if expected_size.is_some_and(|expected| expected != actual_size) {
        return Err(AttemptError::integrity(
            false,
            IntegrityDiagnostic {
                subtype: IntegritySubtype::BodyFinalSizeMismatch,
                response_status,
                resume: resume_state,
                remote_total: IntegrityRemoteTotalPresence::from_known(remote_total),
                size_relation: IntegritySizeRelation::Mismatch,
            },
        ));
    }
    let mut sha256_acc = Sha256Accumulator::default();
    let mut extra_acc = checksum
        .filter(|expected| expected.algorithm() != ChecksumAlgorithm::Sha256)
        .map(|expected| ChecksumAccumulator::new(expected.algorithm()));
    let mut buf = vec![0_u8; 1024 * 1024];
    loop {
        if cancelled() {
            return Err(AttemptError::file_io(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "checksum verification cancelled",
            )));
        }
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(AttemptError::file_io(err)),
        };
        sha256_acc.update(&buf[..n]);
        if let Some(acc) = extra_acc.as_mut() {
            acc.update(&buf[..n]);
        }
    }
    let sha256_hex = sha256_acc.finalize_hex();
    if let Some(expected) = checksum {
        let actual_hex = match extra_acc {
            Some(acc) => acc.finalize_hex(),
            None => sha256_hex.clone(),
        };
        if actual_hex != expected.value() {
            return Err(AttemptError::integrity(
                false,
                IntegrityDiagnostic {
                    subtype: IntegritySubtype::ChecksumMismatch,
                    response_status,
                    resume: resume_state,
                    remote_total: IntegrityRemoteTotalPresence::from_known(remote_total),
                    size_relation: IntegritySizeRelation::between(expected_size, Some(actual_size)),
                },
            ));
        }
    }
    let _ = verify_file_checksum;
    Ok(sha256_hex)
}

fn auth_allowed_redirect(initial: &Url, next: &Url) -> bool {
    if same_origin(initial, next) {
        return true;
    }
    initial.scheme() == "https"
        && next.scheme() == "https"
        && initial.host_str().is_some_and(trusted_earthdata_host)
        && next.host_str().is_some_and(trusted_earthdata_host)
}

fn redirect_scheme_allowed(initial: &Url, next: &Url) -> bool {
    matches!(next.scheme(), "https" | "http")
        && (initial.scheme() != "https" || next.scheme() == "https")
}

fn trusted_earthdata_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "urs.earthdata.nasa.gov"
        || host.ends_with(".earthdata.nasa.gov")
        || host == "asf.alaska.edu"
        || host.ends_with(".asf.alaska.edu")
        || host == "earthdatacloud.nasa.gov"
        || host.ends_with(".earthdatacloud.nasa.gov")
}

#[derive(Clone, Copy)]
struct ArtifactValidationInvocation<'a> {
    plan: &'a ArtifactValidationPlan,
    asset: &'a ResolvedAsset,
}

#[derive(Clone, Copy)]
struct CompletedArtifactValidationInvocation<'a> {
    registered: Option<ArtifactValidationInvocation<'a>>,
    burst: Option<BurstArtifactValidationKind>,
}

impl CompletedArtifactValidationInvocation<'_> {
    const fn none() -> Self {
        Self {
            registered: None,
            burst: None,
        }
    }
}

#[derive(Clone, Copy)]
enum BurstArtifactValidationKind {
    Tiff,
    Xml,
    UnsupportedRole,
}

impl BurstArtifactValidationKind {
    fn from_policy(policy: AuthRefreshPolicyId, asset: &ResolvedAsset) -> Option<Self> {
        if policy != AuthRefreshPolicyId::EarthdataBurstSessionV1 {
            return None;
        }
        Some(match asset.asset_role() {
            Some(AssetRole::Primary) => Self::Tiff,
            Some(AssetRole::Metadata) => Self::Xml,
            _ => Self::UnsupportedRole,
        })
    }
}

fn resolved_size_is_remote_authoritative(
    policy: AuthRefreshPolicyId,
    asset: &ResolvedAsset,
    validation: Option<BurstArtifactValidationKind>,
) -> bool {
    asset.remote_response_size_is_authoritative()
        || burst_catalog_size_is_advisory(policy, asset, validation)
}

fn burst_catalog_size_is_advisory(
    policy: AuthRefreshPolicyId,
    asset: &ResolvedAsset,
    validation: Option<BurstArtifactValidationKind>,
) -> bool {
    if policy != AuthRefreshPolicyId::EarthdataBurstSessionV1
        || !matches!(
            validation,
            Some(BurstArtifactValidationKind::Tiff | BurstArtifactValidationKind::Xml)
        )
    {
        return false;
    }
    asset.is_earthdata_burst_catalog_size_advisory_path()
}

#[derive(Debug)]
enum CompletedArtifactValidationError {
    Registered(ArtifactValidationError),
    Burst,
}

impl CompletedArtifactValidationError {
    const fn code(&self) -> &'static str {
        match self {
            Self::Registered(error) => error.code(),
            Self::Burst => "ASF_BURST_ARTIFACT_INVALID",
        }
    }

    const fn reason_zh(&self) -> &'static str {
        match self {
            Self::Registered(error) => error.reason_zh(),
            Self::Burst => "ASF Sentinel-1 Burst 资产未通过冻结角色对应的 TIFF/XML 校验。",
        }
    }

    const fn clears_part(&self) -> bool {
        matches!(self, Self::Burst)
    }

    /// Whether a later attempt may succeed: only a local read failure of a
    /// registered validator (the candidate is kept either way).
    const fn retryable(&self) -> bool {
        match self {
            Self::Registered(error) => error.retryable(),
            Self::Burst => false,
        }
    }
}

fn validate_completed_artifact(
    invocation: CompletedArtifactValidationInvocation<'_>,
    candidate_path: &Path,
) -> Result<(), CompletedArtifactValidationError> {
    if let Some(registered) = invocation.registered {
        registered
            .plan
            .validate(candidate_path, registered.asset)
            .map_err(CompletedArtifactValidationError::Registered)?;
    }
    match invocation.burst {
        Some(BurstArtifactValidationKind::Tiff) => validate_burst_tiff(candidate_path),
        Some(BurstArtifactValidationKind::Xml) => validate_burst_xml(candidate_path),
        Some(BurstArtifactValidationKind::UnsupportedRole) => {
            Err(CompletedArtifactValidationError::Burst)
        }
        None => Ok(()),
    }
}

fn validate_burst_tiff(candidate_path: &Path) -> Result<(), CompletedArtifactValidationError> {
    let mut file =
        fs::File::open(candidate_path).map_err(|_| CompletedArtifactValidationError::Burst)?;
    let length = file
        .metadata()
        .map_err(|_| CompletedArtifactValidationError::Burst)?
        .len();
    if length < 14 {
        return Err(CompletedArtifactValidationError::Burst);
    }

    let mut header = [0_u8; 8];
    file.read_exact(&mut header)
        .map_err(|_| CompletedArtifactValidationError::Burst)?;
    let little_endian = match &header[..4] {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return Err(CompletedArtifactValidationError::Burst),
    };
    let read_u16 = |bytes: [u8; 2]| {
        if little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    };
    let read_u32 = |bytes: [u8; 4]| {
        if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    };
    let first_ifd = u64::from(read_u32(header[4..8].try_into().unwrap()));
    if first_ifd < 8 || first_ifd.checked_add(2).is_none_or(|end| end > length) {
        return Err(CompletedArtifactValidationError::Burst);
    }
    file.seek(SeekFrom::Start(first_ifd))
        .map_err(|_| CompletedArtifactValidationError::Burst)?;
    let mut count = [0_u8; 2];
    file.read_exact(&mut count)
        .map_err(|_| CompletedArtifactValidationError::Burst)?;
    let entry_count = u64::from(read_u16(count));
    let directory_end = first_ifd
        .checked_add(2)
        .and_then(|value| {
            entry_count
                .checked_mul(12)
                .and_then(|size| value.checked_add(size))
        })
        .and_then(|value| value.checked_add(4))
        .ok_or(CompletedArtifactValidationError::Burst)?;
    if directory_end > length {
        return Err(CompletedArtifactValidationError::Burst);
    }
    Ok(())
}

const MAX_BURST_XML_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BURST_XML_DEPTH: usize = 256;
const MAX_BURST_XML_EVENTS: usize = 1_000_000;

fn validate_burst_xml(candidate_path: &Path) -> Result<(), CompletedArtifactValidationError> {
    let file =
        fs::File::open(candidate_path).map_err(|_| CompletedArtifactValidationError::Burst)?;
    if file
        .metadata()
        .map_err(|_| CompletedArtifactValidationError::Burst)?
        .len()
        > MAX_BURST_XML_BYTES
    {
        return Err(CompletedArtifactValidationError::Burst);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BURST_XML_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CompletedArtifactValidationError::Burst)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BURST_XML_BYTES {
        return Err(CompletedArtifactValidationError::Burst);
    }
    let bytes = bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(bytes.as_slice());
    std::str::from_utf8(bytes).map_err(|_| CompletedArtifactValidationError::Burst)?;
    let mut reader = Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut event_count = 0_usize;
    let mut root_seen = false;
    let mut root_closed = false;
    let mut declaration_seen = false;

    loop {
        event_count = event_count
            .checked_add(1)
            .filter(|count| *count <= MAX_BURST_XML_EVENTS)
            .ok_or(CompletedArtifactValidationError::Burst)?;
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|_| CompletedArtifactValidationError::Burst)?;
        match event {
            Event::Start(event) => {
                validate_xml_attributes(&event)?;
                if depth == 0 {
                    if root_seen || root_closed || rejected_burst_xml_root(event.name().as_ref()) {
                        return Err(CompletedArtifactValidationError::Burst);
                    }
                    root_seen = true;
                }
                depth = depth
                    .checked_add(1)
                    .filter(|value| *value <= MAX_BURST_XML_DEPTH)
                    .ok_or(CompletedArtifactValidationError::Burst)?;
            }
            Event::Empty(event) => {
                validate_xml_attributes(&event)?;
                if depth == 0 {
                    if root_seen || root_closed || rejected_burst_xml_root(event.name().as_ref()) {
                        return Err(CompletedArtifactValidationError::Burst);
                    }
                    root_seen = true;
                    root_closed = true;
                }
            }
            Event::End(_) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or(CompletedArtifactValidationError::Burst)?;
                if depth == 0 {
                    root_closed = true;
                }
            }
            Event::Text(text) => {
                if depth == 0 && !text.iter().all(|byte| byte.is_ascii_whitespace()) {
                    return Err(CompletedArtifactValidationError::Burst);
                }
            }
            Event::CData(_) if depth == 0 => {
                return Err(CompletedArtifactValidationError::Burst);
            }
            Event::Decl(declaration) => {
                if declaration_seen || root_seen || depth != 0 {
                    return Err(CompletedArtifactValidationError::Burst);
                }
                declaration
                    .xml_version()
                    .map_err(|_| CompletedArtifactValidationError::Burst)?;
                if declaration.encoding().is_some_and(|encoding| {
                    encoding
                        .map(|value| {
                            !value.eq_ignore_ascii_case(b"utf-8")
                                && !value.eq_ignore_ascii_case(b"utf8")
                        })
                        .unwrap_or(true)
                }) {
                    return Err(CompletedArtifactValidationError::Burst);
                }
                declaration_seen = true;
            }
            Event::GeneralRef(reference) => {
                if depth == 0 || !safe_xml_reference(reference.as_ref()) {
                    return Err(CompletedArtifactValidationError::Burst);
                }
            }
            Event::PI(_) | Event::DocType(_) => {
                return Err(CompletedArtifactValidationError::Burst);
            }
            Event::Comment(_) | Event::CData(_) => {}
            Event::Eof => {
                return if root_seen && root_closed && depth == 0 {
                    Ok(())
                } else {
                    Err(CompletedArtifactValidationError::Burst)
                };
            }
        }
        buffer.clear();
    }
}

fn validate_xml_attributes(
    event: &quick_xml::events::BytesStart<'_>,
) -> Result<(), CompletedArtifactValidationError> {
    for attribute in event.attributes() {
        attribute.map_err(|_| CompletedArtifactValidationError::Burst)?;
    }
    Ok(())
}

fn rejected_burst_xml_root(name: &[u8]) -> bool {
    let local_name = name.rsplit(|byte| *byte == b':').next().unwrap_or(name);
    [
        b"html".as_slice(),
        b"login".as_slice(),
        b"error".as_slice(),
        b"errors".as_slice(),
        b"exception".as_slice(),
        b"accessdenied".as_slice(),
    ]
    .iter()
    .any(|rejected| local_name.eq_ignore_ascii_case(rejected))
}

fn safe_xml_reference(reference: &[u8]) -> bool {
    matches!(reference, b"amp" | b"lt" | b"gt" | b"apos" | b"quot")
        || reference
            .strip_prefix(b"#")
            .is_some_and(|digits| !digits.is_empty() && digits.iter().all(u8::is_ascii_digit))
        || reference
            .strip_prefix(b"#x")
            .is_some_and(|digits| !digits.is_empty() && digits.iter().all(u8::is_ascii_hexdigit))
}

struct AttemptError {
    failure: DownloadFailure,
    retryable: bool,
    part: PartAction,
    stall_resume_offset: Option<u64>,
    retry_after_hint: Option<Duration>,
}

/// What happens to the `.part` file after a failed attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PartAction {
    Keep,
    Remove,
    /// Renamed to `<name>.part.eostage-conflict-<UTC>`, never deleted.
    SetAside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntegritySubtype {
    RangeNotSatisfiable,
    ResponsePreparation,
    ExpectedRemoteTotalMismatch,
    ValidatorMismatch,
    BodyFinalSizeMismatch,
    ChecksumMismatch,
    ResolvedIdentityMismatch,
    ResolvedChecksumInvalid,
}

impl IntegritySubtype {
    const fn as_str(self) -> &'static str {
        match self {
            Self::RangeNotSatisfiable => "range_not_satisfiable",
            Self::ResponsePreparation => "response_preparation",
            Self::ExpectedRemoteTotalMismatch => "expected_remote_total_mismatch",
            Self::ValidatorMismatch => "validator_mismatch",
            Self::BodyFinalSizeMismatch => "body_final_size_mismatch",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::ResolvedIdentityMismatch => "resolved_identity_mismatch",
            Self::ResolvedChecksumInvalid => "resolved_checksum_invalid",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntegrityResponseStatusClass {
    Success2xx,
    Partial206,
    Range416,
    Other,
}

impl IntegrityResponseStatusClass {
    fn from_status(status: StatusCode) -> Self {
        if status == StatusCode::PARTIAL_CONTENT {
            Self::Partial206
        } else if status == StatusCode::RANGE_NOT_SATISFIABLE {
            Self::Range416
        } else if status.is_success() {
            Self::Success2xx
        } else {
            Self::Other
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Success2xx => "2xx",
            Self::Partial206 => "206",
            Self::Range416 => "416",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntegrityResumeState {
    Fresh,
    Resume,
}

impl IntegrityResumeState {
    const fn from_offset(offset: u64) -> Self {
        if offset == 0 {
            Self::Fresh
        } else {
            Self::Resume
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Resume => "resume",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntegrityRemoteTotalPresence {
    Present,
    Absent,
    Unknown,
}

impl IntegrityRemoteTotalPresence {
    const fn from_known(value: Option<u64>) -> Self {
        if value.is_some() {
            Self::Present
        } else {
            Self::Absent
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntegritySizeRelation {
    Equal,
    Mismatch,
    Unknown,
}

impl IntegritySizeRelation {
    const fn between(expected: Option<u64>, actual: Option<u64>) -> Self {
        match (expected, actual) {
            (Some(left), Some(right)) if left == right => Self::Equal,
            (Some(_), Some(_)) => Self::Mismatch,
            _ => Self::Unknown,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Equal => "equal",
            Self::Mismatch => "mismatch",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IntegrityDiagnostic {
    subtype: IntegritySubtype,
    response_status: IntegrityResponseStatusClass,
    resume: IntegrityResumeState,
    remote_total: IntegrityRemoteTotalPresence,
    size_relation: IntegritySizeRelation,
}

impl IntegrityDiagnostic {
    fn for_response(
        subtype: IntegritySubtype,
        status: StatusCode,
        resume_from: u64,
        remote_total: Option<Option<u64>>,
        size_relation: IntegritySizeRelation,
    ) -> Self {
        Self {
            subtype,
            response_status: IntegrityResponseStatusClass::from_status(status),
            resume: IntegrityResumeState::from_offset(resume_from),
            remote_total: remote_total.map_or(
                IntegrityRemoteTotalPresence::Unknown,
                IntegrityRemoteTotalPresence::from_known,
            ),
            size_relation,
        }
    }
}

fn integrity_reason(base: &str, diagnostic: IntegrityDiagnostic) -> String {
    format!(
        "{} INTEGRITY_DIAGNOSTIC subtype={} response_status={} resume={} remote_total={} size_relation={}",
        base,
        diagnostic.subtype.as_str(),
        diagnostic.response_status.as_str(),
        diagnostic.resume.as_str(),
        diagnostic.remote_total.as_str(),
        diagnostic.size_relation.as_str(),
    )
}

impl AttemptError {
    fn from_parts(failure: DownloadFailure, retryable: bool, part: PartAction) -> Self {
        Self {
            failure,
            retryable,
            part,
            stall_resume_offset: None,
            retry_after_hint: None,
        }
    }

    fn stalled(resume_offset: u64) -> Self {
        Self {
            failure: DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "下载连接低速停滞，正在从断点重连。".to_string(),
                retryable: true,
            },
            retryable: true,
            part: PartAction::Keep,
            stall_resume_offset: Some(resume_offset),
            retry_after_hint: None,
        }
    }

    fn credential() -> Self {
        Self::from_parts(
            DownloadFailure {
                code: "DL004".to_string(),
                reason_zh: "Earthdata 凭据被拒绝，请检查 Token、账号密码或 .netrc。".to_string(),
                retryable: false,
            },
            false,
            PartAction::Keep,
        )
    }

    fn stall_exhausted(stall_reconnects: u32, zero_progress_stalls: u32) -> Self {
        let reason_zh = if zero_progress_stalls > 2 {
            format!(
                "下载速率持续停滞且连续 {zero_progress_stalls} 次重连无数据进展，已保留断点，请检查网络或代理设置后重试。 network_cause=timeout [source=network]"
            )
        } else {
            format!(
                "下载连接多次低速停滞（已重连 {stall_reconnects} 次），已保留断点，请检查网络稳定性后重试。 network_cause=timeout [source=network]"
            )
        };
        Self::from_parts(
            DownloadFailure {
                code: "DL005".to_string(),
                reason_zh,
                retryable: true,
            },
            true,
            PartAction::Keep,
        )
    }

    fn segment_range_retry() -> Self {
        Self::from_parts(
            DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "远端分段 Range 响应不完整或偏移不一致，已保留断点，请稍后重试。 network_cause=stream_interrupted [source=network]".to_string(),
                retryable: true,
            },
            true,
            PartAction::Keep,
        )
    }

    fn stream_read(error: &std::io::Error) -> Self {
        let raw_text = error.to_string();
        let net_cause = crate::transient_retry::classify_network_error_text(&raw_text);
        let cause_zh = match error.kind() {
            std::io::ErrorKind::ConnectionReset => {
                "下载连接被远端服务器重置（ConnectionReset），已保留断点，请稍后重试。"
            }
            std::io::ErrorKind::UnexpectedEof => {
                "下载数据流意外提前结束（UnexpectedEof），已保留断点，请稍后重试。"
            }
            std::io::ErrorKind::BrokenPipe => {
                "下载连接通道意外断开（BrokenPipe），已保留断点，请稍后重试。"
            }
            std::io::ErrorKind::ConnectionAborted => {
                "下载连接被中止（ConnectionAborted），已保留断点，请检查网络或代理后重试。"
            }
            std::io::ErrorKind::TimedOut => {
                "读取远端数据流超时（TimedOut），已保留断点，请检查网络稳定性后重试。"
            }
            _ => match net_cause {
                "tls" => "安全传输（TLS）通道在读取数据时中断，已保留断点，请检查代理或证书设置。",
                "proxy" => "代理转发下载数据流中断，已保留断点，请检查代理配置后重试。",
                "timeout" => "读取远端数据流超时，已保留断点，请检查网络稳定性后重试。",
                "dns" => "域名解析失败（DNS 异常），已保留断点，请检查网络或 DNS 设置。",
                _ => "读取远端响应数据流中断，已保留断点，请检查网络后重试。",
            },
        };
        let effective_cause = match error.kind() {
            std::io::ErrorKind::TimedOut => "timeout",
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionAborted => "stream_interrupted",
            _ if net_cause != "unknown" => net_cause,
            _ => "stream_interrupted",
        };
        let kind_label = format!("{:?}", error.kind());
        let reason_zh = if effective_cause == "tls" {
            format!(
                "{cause_zh} stream_io_kind={kind_label} [transport_cause=tls_certificate source=tls_certificate]"
            )
        } else {
            format!(
                "{cause_zh} stream_io_kind={kind_label} network_cause={effective_cause} [source=network]"
            )
        };
        Self::from_parts(
            DownloadFailure {
                code: "DL005".to_string(),
                reason_zh,
                retryable: true,
            },
            true,
            PartAction::Keep,
        )
    }

    fn transient_for_status(status: StatusCode, retry_after: Option<Duration>) -> Self {
        let mut failure = transient_failure();
        let code_u16 = status.as_u16();
        failure.reason_zh = match status {
            StatusCode::TOO_MANY_REQUESTS => match retry_after {
                Some(d) => format!(
                    "远程服务器触发限流（HTTP 429，Retry-After={}s），已保留断点，请稍后重试或降低并发数。 [source=network]",
                    d.as_secs()
                ),
                None => {
                    "远程服务器触发限流（HTTP 429），已保留断点，请稍后重试或降低并发数。 [source=network]"
                        .to_owned()
                }
            },
            StatusCode::BAD_GATEWAY => {
                "上游网关返回异常（HTTP 502 Bad Gateway），已保留断点，请稍后重试。 [source=network]"
                    .to_owned()
            }
            StatusCode::SERVICE_UNAVAILABLE => {
                "远程数据服务暂时不可用（HTTP 503 Service Unavailable），已保留断点，请稍后重试。 [source=network]"
                    .to_owned()
            }
            StatusCode::GATEWAY_TIMEOUT => {
                "远程网关响应超时（HTTP 504 Gateway Timeout），已保留断点，请稍后重试。 [source=network]"
                    .to_owned()
            }
            _ => format!(
                "远程服务器返回临时异常（HTTP {code_u16}），已保留断点，请稍后重试。 [source=network]"
            ),
        };
        Self {
            failure,
            retryable: true,
            part: PartAction::Keep,
            stall_resume_offset: None,
            retry_after_hint: retry_after,
        }
    }

    fn transport(error: reqwest::Error) -> Self {
        let net_cause = crate::transient_retry::classify_reqwest_network_cause(&error);
        let cause = if request_error_is_tls_certificate(&error) {
            TransportFailureCause::TlsCertificate
        } else if error.is_timeout() || error.is_connect() || error.is_request() || error.is_body()
        {
            TransportFailureCause::Network
        } else {
            TransportFailureCause::Other
        };
        let raw_without_url = error.without_url().to_string();
        let (message, redacted) = sanitize_download_diagnostic_text(&raw_without_url);
        let reason = transport_failure_reason_detailed(&message, redacted, cause, Some(net_cause));
        Self::from_parts(
            DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: reason,
                retryable: true,
            },
            true,
            PartAction::Keep,
        )
    }

    fn redirect_rejected(initial: &Url, candidate: Option<&Url>) -> Self {
        let target = candidate
            .and_then(Url::host_str)
            .filter(|host| !host.is_empty() && host.len() <= 253)
            .unwrap_or("unavailable");
        let scheme = candidate.map(Url::scheme).unwrap_or("unavailable");
        let port = match candidate.and_then(Url::port_or_known_default) {
            Some(443) => "default_https",
            Some(_) => "non_default",
            None => "unknown",
        };
        let userinfo = format!(
            "{} same_url={}",
            candidate.is_some_and(|url| !url.username().is_empty() || url.password().is_some()),
            candidate.is_some_and(|url| url == initial),
        );
        let extractor_diagnostic = candidate
            .and_then(asf_extractor_signed_object_diagnostic)
            .map(|diagnostic| {
                let query_keys = if diagnostic.query_keys.is_empty() {
                    "none".to_owned()
                } else {
                    diagnostic.query_keys.join(",")
                };
                format!(
                    " ASF_EXTRACTOR_DIAGNOSTIC rejection={} path_segment_count={} path_terminal={} path_encoded={} path_trailing_slash={} query_keys={}",
                    diagnostic.rejection.as_str(),
                    diagnostic.path_segment_count.as_str(),
                    diagnostic.path_terminal.as_str(),
                    diagnostic.path_has_encoded_separator_or_dot,
                    diagnostic.path_trailing_slash,
                    query_keys
                )
            })
            .unwrap_or_default();
        Self::from_parts(
            DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: format!(
                    "重定向目标主机 {} scheme={} port={} userinfo={} 被冻结的远程主机策略拒绝。{}",
                    target, scheme, port, userinfo, extractor_diagnostic
                ),
                retryable: false,
            },
            false,
            PartAction::Keep,
        )
    }

    fn http_status(status: StatusCode) -> Self {
        let code_u16 = status.as_u16();
        let canonical = status.canonical_reason().unwrap_or("unknown");
        let prefix_zh = match status {
            StatusCode::NOT_FOUND => {
                "远端文件不存在或下载地址已失效（HTTP 404 Not Found），请重新检索后重试。"
                    .to_owned()
            }
            StatusCode::GONE => {
                "远端下载链接已过期失效（HTTP 410 Gone），请重新检索后重试。".to_owned()
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                "远端服务器拒绝当前断点范围（HTTP 416 Range Not Satisfiable），请重试。".to_owned()
            }
            _ => format!("远端服务器返回异常状态（HTTP {code_u16} {canonical}），请稍后重试。"),
        };
        Self::from_parts(
            DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: format!(
                    "{prefix_zh} HTTP response status={code_u16} reason={canonical}"
                ),
                retryable: false,
            },
            false,
            PartAction::Keep,
        )
    }

    fn integrity(clear_part: bool, diagnostic: IntegrityDiagnostic) -> Self {
        Self::from_parts(
            DownloadFailure {
                code: "DL002".to_string(),
                reason_zh: integrity_reason(
                    "下载文件不完整或服务器拒绝断点续传，将按规则重试。",
                    diagnostic,
                ),
                retryable: true,
            },
            true,
            if clear_part {
                PartAction::Remove
            } else {
                PartAction::Keep
            },
        )
    }

    fn resume_refused(diagnostic: IntegrityDiagnostic) -> Self {
        Self::from_parts(
            DownloadFailure {
                code: "DL002".to_string(),
                reason_zh: integrity_reason(
                    "服务器不再接受断点续传，未完成文件已改名另存，将从头重新下载。",
                    diagnostic,
                ),
                retryable: true,
            },
            true,
            PartAction::SetAside,
        )
    }

    fn fatal() -> Self {
        Self::from_parts(
            DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "下载请求被远端拒绝，请稍后重新检索或重试。".to_string(),
                retryable: false,
            },
            false,
            PartAction::Keep,
        )
    }

    fn bounded() -> Self {
        Self::from_parts(
            DownloadFailure {
                code: "DL005".to_string(),
                reason_zh: "远程服务等待或重定向次数超过安全上限，请稍后重试。".to_string(),
                retryable: false,
            },
            false,
            PartAction::Keep,
        )
    }

    fn artifact_validation(error: CompletedArtifactValidationError) -> Self {
        let part = if error.clears_part() {
            PartAction::Remove
        } else {
            PartAction::Keep
        };
        Self::from_parts(
            DownloadFailure {
                code: error.code().to_string(),
                reason_zh: error.reason_zh().to_string(),
                retryable: error.retryable(),
            },
            false,
            part,
        )
    }

    fn file(error: DownloadFileError) -> Self {
        Self::from_parts(
            DownloadFailure {
                code: error.code().to_string(),
                reason_zh: error.reason_zh().to_string(),
                retryable: false,
            },
            false,
            PartAction::Keep,
        )
    }

    fn file_io(error: std::io::Error) -> Self {
        Self::file(DownloadFileError::Filesystem(error))
    }
}

/// What a transport failure tells the task card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransportFailureCause {
    /// The server certificate could not be verified (L5 recheck 2026-10-19).
    TlsCertificate,
    /// No usable response: a timeout, a connection or a body failure
    /// (downloader enhancement D1d).
    Network,
    Other,
}

/// The reason of a transport failure; a certificate verification failure
/// names its cause first (L5 recheck 2026-10-19) and a network failure
/// declares its cause in the tail (D1d). The `.part` file is kept either way.
#[cfg(test)]
fn transport_failure_reason(message: &str, redacted: bool, cause: TransportFailureCause) -> String {
    transport_failure_reason_detailed(message, redacted, cause, None)
}

fn sanitize_download_diagnostic_text(raw: &str) -> (String, bool) {
    let (sanitized, mut redacted) = sanitize_external_error_message(raw);
    let mut cleaned = Vec::new();
    for token in sanitized.split_whitespace() {
        let trimmed = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | ',' | ';'
            )
        });
        let lower = trimmed.to_ascii_lowercase();
        let is_url = lower.starts_with("http://") || lower.starts_with("https://");
        let is_local_path = trimmed.starts_with("/Users/")
            || trimmed.starts_with("/home/")
            || trimmed.starts_with("/var/")
            || trimmed.starts_with("/tmp/")
            || trimmed.starts_with("/private/")
            || (trimmed.len() >= 3
                && trimmed.as_bytes()[0].is_ascii_alphabetic()
                && trimmed.as_bytes()[1] == b':'
                && (trimmed.as_bytes()[2] == b'\\' || trimmed.as_bytes()[2] == b'/'));
        if is_url {
            redacted = true;
            cleaned.push("<url>".to_owned());
        } else if is_local_path {
            redacted = true;
            cleaned.push("<path>".to_owned());
        } else {
            cleaned.push(token.to_owned());
        }
    }
    (cleaned.join(" "), redacted)
}

fn network_cause_summary_zh(network_cause: &str) -> Option<&'static str> {
    match network_cause {
        "timeout" => Some("连接或读取远端服务器超时，已保留断点，请检查网络或代理设置后重试。"),
        "dns" => {
            Some("域名解析失败（DNS 无法解析远端服务器地址），已保留断点，请检查网络或 DNS 设置。")
        }
        "proxy" => Some("代理服务器连接失败，已保留断点，请检查系统或应用代理设置。"),
        "tls" => {
            Some("安全连接（TLS 握手或证书校验）失败，已保留断点，请检查网络环境或代理证书设置。")
        }
        "stream_interrupted" => Some("下载数据流被远端重置或意外中断，已保留断点，请稍后重试。"),
        "connect" => {
            Some("无法建立到远端服务器的网络连接，已保留断点，请检查网络连通性或代理设置。")
        }
        _ => None,
    }
}

fn transport_failure_reason_detailed(
    message: &str,
    redacted: bool,
    cause: TransportFailureCause,
    network_cause: Option<&'static str>,
) -> String {
    let mut reason = if cause == TransportFailureCause::TlsCertificate {
        format!("{TLS_CERTIFICATE_FAILURE_ZH}，断点已保留。HTTP transport error: {message}")
    } else if let Some(prefix_zh) = network_cause.and_then(network_cause_summary_zh) {
        format!("{prefix_zh} HTTP transport error: {message}")
    } else {
        format!("HTTP transport error: {message}")
    };
    if redacted {
        reason.push_str(" secret_material_redacted=true");
    }
    if let Some(nc) = network_cause.filter(|c| *c != "unknown") {
        if cause == TransportFailureCause::Network {
            reason.push_str(&format!(" network_cause={nc}"));
        }
    }
    match cause {
        TransportFailureCause::TlsCertificate => {
            reason.push_str(" [transport_cause=tls_certificate source=tls_certificate]");
        }
        TransportFailureCause::Network => reason.push_str(" [source=network]"),
        TransportFailureCause::Other => {}
    }
    reason
}

fn success(outcome: DownloadOutcome, bytes: u64, attempts: u32) -> TransferResult {
    TransferResult {
        outcome,
        bytes_written: bytes,
        attempt_count: attempts,
        failure: None,
    }
}

fn artifact_validation_failure(
    bytes: u64,
    attempts: u32,
    error: &CompletedArtifactValidationError,
) -> TransferResult {
    TransferResult {
        outcome: DownloadOutcome::Failed,
        bytes_written: bytes,
        attempt_count: attempts,
        failure: Some(DownloadFailure {
            code: error.code().to_string(),
            reason_zh: error.reason_zh().to_string(),
            retryable: error.retryable(),
        }),
    }
}

fn interrupted(destination: &Path, attempts: u32) -> TransferResult {
    TransferResult {
        outcome: DownloadOutcome::Interrupted,
        bytes_written: part_size(destination),
        attempt_count: attempts,
        failure: Some(DownloadFailure {
            code: "DL001".to_string(),
            reason_zh: "下载已中断，已保留 .part 文件，可继续断点续传。".to_string(),
            retryable: true,
        }),
    }
}

fn file_failure(error: DownloadFileError, attempts: u32) -> TransferResult {
    TransferResult {
        outcome: DownloadOutcome::Failed,
        bytes_written: 0,
        attempt_count: attempts,
        failure: Some(DownloadFailure {
            code: error.code().to_string(),
            reason_zh: error.reason_zh().to_string(),
            retryable: false,
        }),
    }
}

fn inspect_resolved_for_session(
    session: &ResolvedTransferSession,
    destination: &Path,
    expected_size: Option<u64>,
    checksum: Option<&ExpectedChecksum>,
) -> Result<FilePreparation, DownloadFileError> {
    let preparation = if let Some(saved) = &session.recovered_checkpoint {
        // This selects the request offset only; no file is opened for writing
        // until the response has independently verified this saved prefix.
        FilePreparation::Resume {
            offset: saved.bytes,
        }
    } else if session.force_restart && !destination.is_file() {
        FilePreparation::Restart
    } else {
        inspect_resolved_destination(destination, expected_size, checksum, session.part_context())?
    };
    if let Some(required) = session.required_resume_offset {
        let actual = match preparation {
            FilePreparation::Resume { offset } => Some(offset),
            FilePreparation::FinalizePart { bytes } => Some(bytes),
            FilePreparation::SkipComplete { bytes } => Some(bytes),
            FilePreparation::Restart => None,
        };
        if actual != Some(required) {
            return Err(DownloadFileError::IntegrityMismatch);
        }
    }
    if resume_diagnostic_enabled() {
        // Fixed decision/reason codes and numeric offsets only; no file names.
        let decision = match &preparation {
            FilePreparation::SkipComplete { .. } => "skip_complete",
            FilePreparation::FinalizePart { .. } => "finalize_part",
            FilePreparation::Resume { .. } => {
                if session.recovered_checkpoint.is_some() {
                    "resume_checkpoint"
                } else {
                    "resume_prefix"
                }
            }
            FilePreparation::Restart => "restart",
        };
        let reason = if session.recovered_checkpoint.is_some() {
            "checkpoint"
        } else if session.force_restart {
            "force_restart"
        } else {
            match session.part_context() {
                crate::file_core::ResolvedPartContext::CurrentExecution => "current_execution",
                crate::file_core::ResolvedPartContext::RecoveredAfterRestart => {
                    "recovered_after_restart"
                }
            }
        };
        let offset = match &preparation {
            FilePreparation::Resume { offset } => Some(*offset),
            _ => None,
        };
        let offset_text = offset
            .map(|value| format!(",\"offset\":{value}"))
            .unwrap_or_default();
        eprintln!(
            "INSAR_USGS_M2M_RESUME_DIAGNOSTIC {{\"event\":\"inspect_destination\",\"decision\":\"{decision}\",\"reason\":\"{reason}\"{offset_text}}}"
        );
    }
    Ok(preparation)
}

fn resolved_failure(
    destination: &Path,
    attempts: u32,
    code: &str,
    reason_zh: &str,
    retryable: bool,
) -> TransferResult {
    TransferResult {
        outcome: DownloadOutcome::Failed,
        bytes_written: part_size(destination),
        attempt_count: attempts,
        failure: Some(DownloadFailure {
            code: code.to_string(),
            reason_zh: reason_zh.to_string(),
            retryable,
        }),
    }
}

fn earthdata_session_transport_failure_for_reqwest(error: &reqwest::Error) -> DownloadFailure {
    let net_cause = crate::transient_retry::classify_reqwest_network_cause(error);
    let prefix = if request_error_is_tls_certificate(error) {
        "Earthdata 会话认证 TLS 证书校验失败，请检查代理或系统证书设置。"
    } else {
        match net_cause {
            "timeout" => "Earthdata 会话认证请求超时，请检查网络或代理后重试。",
            "dns" => "Earthdata 认证服务域名解析失败（DNS 异常），请检查网络设置。",
            "proxy" => "通过代理连接 Earthdata 认证服务失败，请检查代理设置。",
            "tls" => "Earthdata 认证服务 TLS 握手失败，请检查网络环境或代理证书。",
            "stream_interrupted" => "Earthdata 会话认证响应流中断，请稍后重试。",
            "connect" => "无法连接到 Earthdata 认证服务，请检查网络连通性或代理设置。",
            _ => "Earthdata 会话请求未获得可用响应，请检查网络后重试。",
        }
    };
    let tail = if request_error_is_tls_certificate(error) {
        " [transport_cause=tls_certificate source=tls_certificate]"
    } else {
        " [source=network]"
    };
    DownloadFailure {
        code: "DL005".to_string(),
        reason_zh: format!("{prefix}{tail}"),
        retryable: true,
    }
}

fn earthdata_session_transport_failure_for_io(error: &std::io::Error) -> DownloadFailure {
    let kind_zh = match error.kind() {
        std::io::ErrorKind::TimedOut => "读取超时",
        std::io::ErrorKind::ConnectionReset => "连接被远端重置",
        std::io::ErrorKind::UnexpectedEof => "响应提前结束",
        std::io::ErrorKind::BrokenPipe => "连接通道断开",
        _ => "响应流中断",
    };
    DownloadFailure {
        code: "DL005".to_string(),
        reason_zh: format!(
            "读取 Earthdata 会话认证响应失败（{kind_zh}），请检查网络后重试。 [source=network]"
        ),
        retryable: true,
    }
}

fn earthdata_session_cancelled_failure() -> DownloadFailure {
    DownloadFailure {
        code: "DL001".to_string(),
        reason_zh: "Earthdata 会话建立在传输开始前被取消。".to_string(),
        retryable: true,
    }
}

fn earthdata_session_retired_failure() -> DownloadFailure {
    DownloadFailure {
        code: "DL005".to_string(),
        reason_zh: "Earthdata 登录信息已更换，会话需要重新建立。".to_string(),
        retryable: true,
    }
}

fn earthdata_session_credential_failure() -> DownloadFailure {
    DownloadFailure {
        code: "DL004".to_string(),
        reason_zh: "Earthdata 会话凭据被远程服务拒绝。".to_string(),
        retryable: false,
    }
}

fn earthdata_session_cookie_required_failure() -> DownloadFailure {
    DownloadFailure {
        code: BurstSessionError::CookieRequired.code().to_string(),
        reason_zh: BurstSessionError::CookieRequired.reason_zh().to_string(),
        retryable: false,
    }
}

fn earthdata_session_invalid_failure() -> DownloadFailure {
    DownloadFailure {
        code: "DL005".to_string(),
        reason_zh: "Earthdata 会话响应缺少安全的访问令牌。".to_string(),
        retryable: false,
    }
}

fn earthdata_session_http_failure(status: StatusCode) -> DownloadFailure {
    DownloadFailure {
        code: "DL005".to_string(),
        reason_zh: format!("Earthdata 会话 HTTP 状态码 {} 被拒绝。", status.as_u16()),
        retryable: false,
    }
}

fn earthdata_session_redirect_failure() -> DownloadFailure {
    DownloadFailure {
        code: "DL005".to_string(),
        reason_zh: "Earthdata 会话重定向未通过冻结的主机策略。".to_string(),
        retryable: false,
    }
}

fn transient_failure() -> DownloadFailure {
    DownloadFailure {
        code: "DL005".to_string(),
        reason_zh: "下载连接暂时失败，已按重试策略处理。".to_string(),
        retryable: true,
    }
}

fn prepare_response_file(
    resume_from: u64,
    overlap_bytes: u64,
    response: &Response,
) -> Result<(FilePreparation, Option<u64>), AttemptError> {
    let status = response.status();
    let preparation_error = || {
        // A remote response cannot authorize deletion of an existing prefix.
        AttemptError::integrity(
            false,
            IntegrityDiagnostic::for_response(
                IntegritySubtype::ResponsePreparation,
                status,
                resume_from,
                None,
                IntegritySizeRelation::Unknown,
            ),
        )
    };
    if resume_from > 0 {
        let expected_start = resume_from
            .checked_sub(overlap_bytes)
            .ok_or_else(preparation_error)?;
        if status == StatusCode::PARTIAL_CONTENT {
            let (_start, total) = content_range(response)
                .filter(|(start, _)| *start == expected_start)
                .ok_or_else(preparation_error)?;
            Ok((
                FilePreparation::Resume {
                    offset: resume_from,
                },
                total,
            ))
        } else {
            // Ignored Range / failed If-Range needs an explicit new transfer,
            // not truncation of the partial accepted by this execution.
            Err(preparation_error())
        }
    } else if status == StatusCode::OK {
        Ok((FilePreparation::Restart, response.content_length()))
    } else {
        Err(preparation_error())
    }
}

fn content_range(response: &Response) -> Option<(u64, Option<u64>)> {
    let value = response.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    let range_and_total = value.strip_prefix("bytes ")?;
    let (range, total) = range_and_total.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.parse().ok()?;
    let end: u64 = end.parse().ok()?;
    let length = end.checked_sub(start)?.checked_add(1)?;
    if response
        .content_length()
        .is_some_and(|value| value != length)
    {
        return None;
    }
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    if total.is_some_and(|total| end >= total) {
        return None;
    }
    Some((start, total))
}

fn response_validator(response: &Response, remote_total: Option<u64>) -> RemoteValidator {
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .and_then(strong_etag)
        .map(str::to_string);
    let last_modified = response
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_canonical_http_date);
    RemoteValidator {
        etag,
        last_modified,
        content_length: remote_total.or_else(|| response.content_length()),
    }
}

fn if_range_value(validator: &RemoteValidator) -> Option<&str> {
    validator.etag.as_deref().and_then(strong_etag).or_else(|| {
        validator
            .content_length
            .and(validator.last_modified.as_deref())
    })
}

/// Opt-in resume diagnosis under the shared `INSAR_DIAGNOSTIC_USGS_M2M` gate
/// (same values as the resolve diagnostic). Emits only decisions, offsets and
/// validator kinds — never a URL, path, token or credential.
fn resume_diagnostic_enabled() -> bool {
    std::env::var("INSAR_DIAGNOSTIC_USGS_M2M")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
}

/// Quarantine an unprovable checkpoint's evidence and restart from zero.
///
/// The `.part` file and the `<name>.part.resume.json` record are renamed
/// under conflict names (never deleted or truncated, downloader D3 owner
/// rule 2026-10-18), so a task whose saved prefix cannot be proven safe
/// restarts cleanly instead of failing, and no surviving recovery artifact
/// is destroyed. The record is quarantined as well because it can no longer
/// be paired with its prefix; both stay inspectable under their stamped
/// names. Returns false only when the `.part` could not be quarantined: the
/// caller then fails closed, because a fresh start would truncate the
/// surviving unproven prefix.
fn checkpoint_quarantine_unprovable(destination: &Path) -> bool {
    let stamp = crate::local_reuse::conflict_stamp(&crate::scheduler::timestamp_now());
    if resume_diagnostic_enabled() {
        eprintln!("INSAR_USGS_M2M_RESUME_DIAGNOSTIC {{\"event\":\"checkpoint_quarantined\"}}");
    }
    let quarantined = crate::local_reuse::quarantine_conflict(&part_path(destination), &stamp)
        .map(|_| true)
        .unwrap_or_else(|error| {
            if resume_diagnostic_enabled() {
                eprintln!(
                    "INSAR_USGS_M2M_RESUME_DIAGNOSTIC {{\"event\":\"checkpoint_quarantine_failed\"}}"
                );
            }
            let _ = error;
            false
        });
    if !quarantined {
        return false;
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if let Some(name) = destination.file_name().and_then(|n| n.to_str()) {
        let record = parent.join(format!("{name}.part.resume.json"));
        let _ = crate::local_reuse::quarantine_conflict(&record, &stamp);
    }
    true
}

fn ensure_response_validator_compatible(
    previous: &RemoteValidator,
    current: &RemoteValidator,
    diagnostic: IntegrityDiagnostic,
) -> Result<(), AttemptError> {
    ensure_response_value_compatible(
        previous.etag.as_deref().and_then(strong_etag),
        current.etag.as_deref().and_then(strong_etag),
        diagnostic,
    )?;
    ensure_response_value_compatible(
        previous.last_modified.as_deref(),
        current.last_modified.as_deref(),
        diagnostic,
    )?;
    ensure_response_value_compatible(previous.content_length, current.content_length, diagnostic)
}

fn strong_etag(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.len() <= 2
        || value.len() > 1024
        || value
            .get(..2)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("w/"))
        || !value.starts_with('"')
        || !value.ends_with('"')
        || value[1..value.len() - 1].contains('"')
        || value.chars().any(char::is_control)
    {
        None
    } else {
        Some(value)
    }
}

fn ensure_response_value_compatible<T: PartialEq>(
    previous: Option<T>,
    current: Option<T>,
    diagnostic: IntegrityDiagnostic,
) -> Result<(), AttemptError> {
    if matches!((previous, current), (Some(left), Some(right)) if left != right) {
        Err(AttemptError::integrity(false, diagnostic))
    } else {
        Ok(())
    }
}

fn remove_part(destination: &Path) {
    let _ = fs::remove_file(part_path(destination));
    PartMetaCheckpoint::clear(destination);
}

/// Returns false only when a part that must be set aside could not be renamed.
fn apply_part_action(destination: &Path, action: PartAction) -> bool {
    match action {
        PartAction::Keep => true,
        PartAction::Remove => {
            remove_part(destination);
            true
        }
        PartAction::SetAside => {
            let ok = crate::local_reuse::quarantine_conflict(
                &part_path(destination),
                &crate::local_reuse::conflict_stamp(&crate::scheduler::timestamp_now()),
            )
            .is_ok();
            if ok {
                PartMetaCheckpoint::clear(destination);
            }
            ok
        }
    }
}

fn part_set_aside_failure(destination: &Path, attempts: u32) -> TransferResult {
    resolved_failure(
        destination,
        attempts,
        "DL002",
        "服务器不再接受断点续传，且未完成文件无法改名另存（可能被占用或无权限），未改动。",
        true,
    )
}

fn part_allocated_size(destination: &Path) -> u64 {
    fs::metadata(part_path(destination))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn part_size(destination: &Path) -> u64 {
    part_completed_bytes(destination)
}

fn wait_backoff(duration: Duration, cancelled: &(dyn Fn() -> bool + Sync)) -> bool {
    let mut remaining = duration;
    while !remaining.is_zero() {
        if cancelled() {
            return true;
        }
        let interval = remaining.min(Duration::from_millis(50));
        thread::sleep(interval);
        remaining = remaining.saturating_sub(interval);
    }
    cancelled()
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn loopback_url(value: &Url) -> bool {
    matches!(value.scheme(), "http" | "https")
        && value.username().is_empty()
        && value.password().is_none()
        && value.query().is_none()
        && value.fragment().is_none()
        && value
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback())
}

fn range_url_allowed(asset: &ResolvedAsset, value: &Url) -> bool {
    #[cfg(any(test, feature = "offline-nisar-subset-test-support"))]
    if loopback_url(value) {
        return true;
    }
    #[cfg(feature = "nisar-subset-production")]
    {
        asset
            .host_policy()
            .is_some_and(|policy| policy.allows_response_url(value))
    }
    #[cfg(not(feature = "nisar-subset-production"))]
    {
        let _ = asset;
        false
    }
}

fn redirect_host_class(value: &Url) -> &'static str {
    let Some(host) = value.host_str() else {
        return "missing";
    };
    if host.parse::<std::net::IpAddr>().is_ok() {
        if loopback_url(value) {
            return "loopback";
        }
        return "ip-literal";
    }
    let host = host.to_ascii_lowercase();
    if host == "urs.earthdata.nasa.gov"
        || host.ends_with(".earthdata.nasa.gov")
        || host == "earthdatacloud.nasa.gov"
        || host.ends_with(".earthdatacloud.nasa.gov")
    {
        "earthdata"
    } else if host == "asf.alaska.edu" || host.ends_with(".asf.alaska.edu") {
        "asf"
    } else if host.contains("cloudfront.net") {
        "cloudfront"
    } else {
        "other"
    }
}

fn redirect_path_class(value: &Url) -> &'static str {
    let path = value.path();
    if path.is_empty() || path == "/" {
        "root"
    } else if path.contains("oauth") || path.contains("authorize") || path.contains("login") {
        "auth"
    } else if path.ends_with(".tif") || path.ends_with(".tiff") {
        "signed-raster"
    } else {
        "resource"
    }
}

fn scheme_transition(source: &Url, destination: &Url) -> String {
    format!("{}-to-{}", source.scheme(), destination.scheme())
}

fn trusted_redirect_evidence(
    source: &Url,
    destination: Option<&Url>,
    response_status: Option<StatusCode>,
    redirect_hop_index: Option<u8>,
    policy_result: &str,
    rejection_reason: &str,
) -> ExactRangeRedirectEvidence {
    let destination = destination.unwrap_or(source);
    ExactRangeRedirectEvidence {
        response_status: response_status.map(|status| status.as_u16()),
        redirect_occurred: true,
        redirect_hop_index,
        source_host_class: redirect_host_class(source).to_owned(),
        destination_host_class: redirect_host_class(destination).to_owned(),
        source_path_class: redirect_path_class(source).to_owned(),
        destination_path_class: redirect_path_class(destination).to_owned(),
        same_origin: same_origin(source, destination),
        scheme_transition: scheme_transition(source, destination),
        policy_result: policy_result.to_owned(),
        rejection_reason: rejection_reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    use std::thread;
    use std::time::Instant;

    use super::{
        apply_resolved_asset_access, auth_allowed_redirect, classify_auth_rejection,
        redirect_scheme_allowed, resolved_redirect_allowed, same_origin, strong_etag,
        trusted_earthdata_host, BurstAuthEndpoints, Duration, ExactRangeSendOutcome,
        ExactRangeTransportCause, HttpTransfer, ResolvedEarthdataSessionEndpoints,
        ResolvedTransferResult, ResponseHostClass, TransferOptions, TransferRequest, Url,
    };
    use crate::{
        asset_resolver::{ResolvedAuthKind, ResolvedHostPolicy},
        part_path, AdmittedTransferBackend, AuthRefreshPolicyId, CredentialMaterial, ResolvedAsset,
        ResolvedMethod, TransferAdmissionController, TransferBackend,
    };
    use insar_core::{AssetRole, DownloadOutcome};
    use reqwest::{
        cookie::CookieStore,
        header::{AUTHORIZATION, COOKIE},
    };

    #[test]
    fn official_earthdata_sessions_use_the_current_asf_auth_callback_and_cookie_scope() {
        let expected_callback = "https://auth.asf.alaska.edu/login";
        let expected_cookie_scope = "https://auth.asf.alaska.edu/";
        let burst = BurstAuthEndpoints::official();
        let resolved = ResolvedEarthdataSessionEndpoints::official();

        for (authorize_url, cookie_scope_url) in [
            (&burst.authorize_url, &burst.cookie_scope_url),
            (&resolved.authorize_url, &resolved.cookie_scope_url),
        ] {
            assert_eq!(
                authorize_url
                    .query_pairs()
                    .find_map(|(key, value)| (key == "redirect_uri").then(|| value.into_owned()))
                    .as_deref(),
                Some(expected_callback)
            );
            assert_eq!(cookie_scope_url.as_str(), expected_cookie_scope);
            assert_ne!(cookie_scope_url.host_str(), Some("cumulus.asf.alaska.edu"));
        }
        let callback = Url::parse(expected_callback).unwrap();
        assert!(trusted_earthdata_host("auth.asf.alaska.edu"));
        assert!(auth_allowed_redirect(&burst.authorize_url, &callback));
        assert!(auth_allowed_redirect(&resolved.authorize_url, &callback));
    }

    #[test]
    fn extractor_signed_object_second_request_has_no_authorization_or_cookie() {
        let source = Url::parse("https://sentinel1-burst.asf.alaska.edu/0.tiff").unwrap();
        let signed_object = Url::parse("https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1_301495_IW3_20141003T054235_VV_D5C8-BURST/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=ASIAFIXTURE%2F20260826%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Security-Token=FIXTURE&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
        let mut asset = ResolvedAsset::new(source.as_str(), ResolvedMethod::Get)
            .unwrap()
            .with_host_policy(ResolvedHostPolicy::EarthdataAsfV1);
        CredentialMaterial::basic("fixture-user", "fixture-password")
            .unwrap()
            .apply_to(&mut asset)
            .unwrap();
        let transfer = HttpTransfer::new(TransferOptions::default()).unwrap();
        transfer
            .cookie_jar
            .add_cookie_str("asf-urs=fixture", &source);
        assert!(transfer.cookie_jar.cookies(&source).is_some());
        assert!(transfer.cookie_jar.cookies(&signed_object).is_none());

        let initial = apply_resolved_asset_access(
            &asset,
            &source,
            &source,
            None,
            transfer.client.get(source.clone()),
            transfer.uncredentialed_client(),
        )
        .build()
        .unwrap();
        assert!(initial.headers().contains_key(AUTHORIZATION));
        let redirected = apply_resolved_asset_access(
            &asset,
            &source,
            &signed_object,
            None,
            transfer.client.get(signed_object.clone()),
            transfer.uncredentialed_client(),
        )
        .build()
        .unwrap();
        assert!(!redirected.headers().contains_key(AUTHORIZATION));
        assert!(!redirected.headers().contains_key(COOKIE));
    }

    #[test]
    fn extractor_redirect_rejection_projects_only_fixed_safe_diagnostics() {
        let source = Url::parse("https://sentinel1-burst.asf.alaska.edu/0.tiff").unwrap();
        let rejected = Url::parse("https://extractor-main-extractorstagingbucket-efrxkhdd3zlp.s3.amazonaws.com/S1_SECRET_NATIVE_ID/0.tiff?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=ASIASECRET%2F20260826%2Fus-west-2%2Fs3%2Faws4_request&X-Amz-Date=20260826T120000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host&X-Amz-Security-Token=SECRET_TOKEN&X-Amz-Signature=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef&x-id=GetObject").unwrap();

        let failure = super::AttemptError::redirect_rejected(&source, Some(&rejected)).failure;

        assert!(failure
            .reason_zh
            .contains("ASF_EXTRACTOR_DIAGNOSTIC rejection=query_keys"));
        assert!(failure
            .reason_zh
            .contains("path_segment_count=2 path_terminal=exact_0_tiff"));
        assert!(failure
            .reason_zh
            .contains("path_encoded=false path_trailing_slash=false"));
        assert!(failure.reason_zh.contains(
            "query_keys=X-Amz-Algorithm,X-Amz-Credential,X-Amz-Date,X-Amz-Expires,X-Amz-Security-Token,X-Amz-Signature,X-Amz-SignedHeaders,x-id"
        ));
        for forbidden in [
            "S1_SECRET_NATIVE_ID",
            "ASIASECRET",
            "SECRET_TOKEN",
            "GetObject",
            "0123456789abcdef",
        ] {
            assert!(!failure.reason_zh.contains(forbidden));
        }
    }

    #[test]
    fn a_partial_that_cannot_be_set_aside_stops_the_transfer_as_retryable() {
        let destination = std::env::temp_dir()
            .join("sceneport-missing-folder-for-set-aside")
            .join("absent.SAFE.zip");
        assert!(!super::apply_part_action(
            &destination,
            super::PartAction::SetAside
        ));
        assert!(super::apply_part_action(
            &destination,
            super::PartAction::Keep
        ));
        let result = super::part_set_aside_failure(&destination, 2);
        assert_eq!(result.outcome, DownloadOutcome::Failed);
        assert_eq!(result.attempt_count, 2);
        let failure = result.failure.expect("failure");
        assert_eq!(failure.code, "DL002");
        assert!(failure.retryable);
    }

    #[test]
    fn integrity_rejections_project_only_fixed_safe_diagnostics() {
        let cases = [
            (
                super::IntegritySubtype::RangeNotSatisfiable,
                super::IntegrityResponseStatusClass::Range416,
                super::IntegrityResumeState::Resume,
                super::IntegrityRemoteTotalPresence::Unknown,
                super::IntegritySizeRelation::Unknown,
                "range_not_satisfiable",
            ),
            (
                super::IntegritySubtype::ResponsePreparation,
                super::IntegrityResponseStatusClass::Other,
                super::IntegrityResumeState::Fresh,
                super::IntegrityRemoteTotalPresence::Unknown,
                super::IntegritySizeRelation::Unknown,
                "response_preparation",
            ),
            (
                super::IntegritySubtype::ExpectedRemoteTotalMismatch,
                super::IntegrityResponseStatusClass::Partial206,
                super::IntegrityResumeState::Resume,
                super::IntegrityRemoteTotalPresence::Present,
                super::IntegritySizeRelation::Mismatch,
                "expected_remote_total_mismatch",
            ),
            (
                super::IntegritySubtype::ValidatorMismatch,
                super::IntegrityResponseStatusClass::Success2xx,
                super::IntegrityResumeState::Fresh,
                super::IntegrityRemoteTotalPresence::Present,
                super::IntegritySizeRelation::Equal,
                "validator_mismatch",
            ),
            (
                super::IntegritySubtype::BodyFinalSizeMismatch,
                super::IntegrityResponseStatusClass::Success2xx,
                super::IntegrityResumeState::Fresh,
                super::IntegrityRemoteTotalPresence::Absent,
                super::IntegritySizeRelation::Mismatch,
                "body_final_size_mismatch",
            ),
            (
                super::IntegritySubtype::ChecksumMismatch,
                super::IntegrityResponseStatusClass::Success2xx,
                super::IntegrityResumeState::Fresh,
                super::IntegrityRemoteTotalPresence::Absent,
                super::IntegritySizeRelation::Unknown,
                "checksum_mismatch",
            ),
            (
                super::IntegritySubtype::ResolvedIdentityMismatch,
                super::IntegrityResponseStatusClass::Other,
                super::IntegrityResumeState::Fresh,
                super::IntegrityRemoteTotalPresence::Unknown,
                super::IntegritySizeRelation::Unknown,
                "resolved_identity_mismatch",
            ),
            (
                super::IntegritySubtype::ResolvedChecksumInvalid,
                super::IntegrityResponseStatusClass::Other,
                super::IntegrityResumeState::Fresh,
                super::IntegrityRemoteTotalPresence::Unknown,
                super::IntegritySizeRelation::Unknown,
                "resolved_checksum_invalid",
            ),
        ];
        for (subtype, response_status, resume, remote_total, size_relation, label) in cases {
            let failure = super::AttemptError::integrity(
                false,
                super::IntegrityDiagnostic {
                    subtype,
                    response_status,
                    resume,
                    remote_total,
                    size_relation,
                },
            )
            .failure;
            assert_eq!(failure.code, "DL002");
            assert!(failure.retryable);
            assert!(failure.reason_zh.contains(&format!(
                "INTEGRITY_DIAGNOSTIC subtype={label} response_status={} resume={} remote_total={} size_relation={}",
                response_status.as_str(),
                resume.as_str(),
                remote_total.as_str(),
                size_relation.as_str(),
            )));
            for forbidden in [
                "Content-Range",
                "ETag",
                "https://",
                "171218848",
                "fixture-secret",
            ] {
                assert!(!failure.reason_zh.contains(forbidden));
            }
        }
    }

    struct BurstFixtureEvidence {
        result: super::TransferResult,
        promoted: bool,
        part_exists: bool,
    }

    fn run_burst_content_fixture(
        label: &str,
        role: AssetRole,
        body: &'static [u8],
        expected_size: u64,
        auth_refresh_policy: AuthRefreshPolicyId,
    ) -> BurstFixtureEvidence {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for index in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut stream = reader.into_inner();
                match index {
                    0 => {
                        let token = br#"{"access_token":"fixture-token"}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            token.len()
                        )
                        .unwrap();
                        stream.write_all(token).unwrap();
                    }
                    1 => write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    _ => {
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    }
                }
                stream.flush().unwrap();
            }
        });

        let destination = std::env::temp_dir().join(format!(
            "insar-burst-validation-{label}-{}.artifact",
            std::process::id()
        ));
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        let burst_url = format!("{base}/burst");
        let mut asset = ResolvedAsset::new(&burst_url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(expected_size))
            .with_asset_role(role)
            .with_host_policy(ResolvedHostPolicy::EarthdataBurstLoopbackFixture);
        CredentialMaterial::basic("fixture-user", "fixture-password")
            .unwrap()
            .apply_to(&mut asset)
            .unwrap();
        let request = TransferRequest::new(
            "burst-validation-task",
            format!("burst-validation-{label}"),
            &burst_url,
            destination.clone(),
            Some(expected_size),
        )
        .unwrap();
        let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        transfer.resolved_earthdata_session_endpoints = ResolvedEarthdataSessionEndpoints::new(
            &format!("{base}/token"),
            &format!("{base}/authorize"),
            &format!("{base}/"),
        )
        .unwrap();
        let mut session = transfer.resolved_session();
        let result = transfer.download_resolved(
            &request,
            &asset,
            auth_refresh_policy,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        server.join().unwrap();
        let ResolvedTransferResult::Finished(result) = result else {
            panic!("Burst fixture unexpectedly requested an auth refresh")
        };
        let evidence = BurstFixtureEvidence {
            result,
            promoted: destination.exists(),
            part_exists: part_path(&destination).exists(),
        };
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        evidence
    }

    #[test]
    fn authorization_cross_origin_allowlist_is_https_and_provider_bounded() {
        let asf = Url::parse("https://datapool.asf.alaska.edu/SLC/a.zip").unwrap();
        let urs = Url::parse("https://urs.earthdata.nasa.gov/oauth").unwrap();
        let s3 = Url::parse("https://example-bucket.s3.amazonaws.com/a.zip").unwrap();
        let insecure = Url::parse("http://urs.earthdata.nasa.gov/oauth").unwrap();
        assert!(auth_allowed_redirect(&asf, &urs));
        assert!(!same_origin(&asf, &urs));
        assert!(same_origin(
            &urs,
            &Url::parse("https://urs.earthdata.nasa.gov/another-step").unwrap()
        ));
        assert!(!auth_allowed_redirect(&asf, &s3));
        assert!(!auth_allowed_redirect(&asf, &insecure));
    }

    #[test]
    fn redirect_scheme_never_downgrades_an_https_download() {
        let secure = Url::parse("https://sentinel1-burst.asf.alaska.edu/a.tiff").unwrap();
        let secure_target = Url::parse("https://cumulus.asf.alaska.edu/a.tiff").unwrap();
        let insecure_target = Url::parse("http://sentinel1-burst.asf.alaska.edu/a.tiff").unwrap();
        let local = Url::parse("http://127.0.0.1/source").unwrap();
        let local_target = Url::parse("http://127.0.0.1/target").unwrap();
        assert!(redirect_scheme_allowed(&secure, &secure_target));
        assert!(!redirect_scheme_allowed(&secure, &insecure_target));
        assert!(redirect_scheme_allowed(&local, &local_target));
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn trusted_redirect_same_origin_preserves_exact_range_and_counts_hops() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for index in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut stream = reader.into_inner();
                if index == 0 {
                    write!(
                        stream,
                        "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                } else {
                    let etag = r#""fixture""#;
                    write!(
                        stream,
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/1\r\nETag: {etag}\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx"
                    )
                    .unwrap();
                }
                stream.flush().unwrap();
            }
        });
        let mut asset = ResolvedAsset::new(&format!("{base}/start"), ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(1))
            .with_host_policy(ResolvedHostPolicy::LoopbackFixture);
        asset = asset.with_remote_validator(crate::asset_resolver::RemoteValidator {
            etag: Some(r#""fixture""#.to_owned()),
            last_modified: None,
            content_length: Some(1),
        });
        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        let outcome =
            transfer.send_resolved_preflight_range_once_trusted_redirect(&asset, 0, 1, &|| false);
        let ExactRangeSendOutcome::Response {
            mut response,
            request_count,
        } = outcome
        else {
            panic!("trusted same-origin redirect did not return a response");
        };
        assert_eq!(request_count, 2);
        assert_eq!(response.status().as_u16(), 206);
        assert_eq!(
            response
                .headers()
                .get("content-range")
                .and_then(|value| value.to_str().ok()),
            Some("bytes 0-0/1")
        );
        assert_eq!(
            response
                .headers()
                .get("etag")
                .and_then(|value| value.to_str().ok()),
            Some(r#""fixture""#)
        );
        let mut body = Vec::new();
        response.read_to_end(&mut body).unwrap();
        assert_eq!(body, b"x");
        server.join().unwrap();
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn exact_range_retryable_exhaustion_retains_closed_cause_and_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let closed_address = listener.local_addr().unwrap();
        drop(listener);
        let closed_asset = ResolvedAsset::new(
            &format!("http://{closed_address}/closed"),
            ResolvedMethod::Get,
        )
        .unwrap()
        .with_expected_size(Some(1))
        .with_host_policy(ResolvedHostPolicy::LoopbackFixture);
        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        match transfer.send_resolved_exact_range_once_trusted_redirect(
            &closed_asset,
            0,
            1,
            r#""fixture""#,
            &|| false,
        ) {
            ExactRangeSendOutcome::RetryableExhausted {
                request_count,
                diagnostic,
            } => {
                assert_eq!(request_count, 1);
                assert!(matches!(
                    diagnostic.cause,
                    ExactRangeTransportCause::RequestConnect
                        | ExactRangeTransportCause::RequestTimeout
                ));
                assert_eq!(diagnostic.http_status, None);
            }
            other => panic!("unexpected closed-port outcome: {}", outcome_kind(&other)),
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
        });
        let status_asset = ResolvedAsset::new(&format!("{base}/status"), ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(1))
            .with_host_policy(ResolvedHostPolicy::LoopbackFixture);
        match transfer.send_resolved_exact_range_once_trusted_redirect(
            &status_asset,
            0,
            1,
            r#""fixture""#,
            &|| false,
        ) {
            ExactRangeSendOutcome::RetryableExhausted {
                request_count,
                diagnostic,
            } => {
                assert_eq!(request_count, 1);
                assert_eq!(
                    diagnostic.cause,
                    ExactRangeTransportCause::RetryableHttpStatus
                );
                assert_eq!(diagnostic.http_status, Some(503));
            }
            other => panic!("unexpected 503 outcome: {}", outcome_kind(&other)),
        }
        server.join().unwrap();
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn trusted_redirect_rejection_captures_status_hop_and_safe_policy_evidence() {
        let target_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        target_listener.set_nonblocking(true).unwrap();
        let target = target_listener.local_addr().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: http://{target}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
        });
        let asset = ResolvedAsset::new(&format!("{base}/start"), ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(1))
            .with_host_policy(ResolvedHostPolicy::EarthdataAsfV1);
        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        let outcome =
            transfer.send_resolved_preflight_range_once_trusted_redirect(&asset, 0, 1, &|| false);
        match outcome {
            ExactRangeSendOutcome::TrustedRedirectRejected {
                request_count,
                evidence,
            } => {
                assert_eq!(request_count, 1);
                assert_eq!(evidence.response_status, Some(302));
                assert!(evidence.redirect_occurred);
                assert_eq!(evidence.redirect_hop_index, Some(1));
                assert_eq!(evidence.source_host_class, "loopback");
                assert_eq!(evidence.destination_host_class, "loopback");
                assert!(!evidence.same_origin);
                assert_eq!(evidence.scheme_transition, "http-to-http");
                assert_eq!(evidence.policy_result, "rejected");
                assert_eq!(evidence.rejection_reason, "destination_policy_rejected");
            }
            other => panic!(
                "unexpected trusted redirect outcome: {:?}",
                outcome_kind(&other)
            ),
        }
        server.join().unwrap();
        match target_listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("rejected redirect target received a request"),
            Err(error) => panic!("unexpected target listener error: {error}"),
        }
    }

    #[cfg(feature = "nisar-subset-production")]
    fn outcome_kind(outcome: &ExactRangeSendOutcome) -> &'static str {
        match outcome {
            ExactRangeSendOutcome::Response { .. } => "response",
            ExactRangeSendOutcome::Cancelled { .. } => "cancelled",
            ExactRangeSendOutcome::RetryableExhausted { .. } => "retryable_exhausted",
            ExactRangeSendOutcome::ProtocolRejected { .. } => "protocol_rejected",
            ExactRangeSendOutcome::TrustedRedirectRejected { .. } => "trusted_redirect_rejected",
            ExactRangeSendOutcome::BudgetExceeded { .. } => "budget_exceeded",
        }
    }

    #[test]
    fn legacy_transfer_allows_one_oauth_return_to_the_initial_archive_url() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for index in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut stream = reader.into_inner();
                match index {
                    0 => write!(
                        stream,
                        "HTTP/1.1 302 Found\r\nLocation: /oauth\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    1 => write!(
                        stream,
                        "HTTP/1.1 302 Found\r\nLocation: /archive\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    _ => write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                    )
                    .unwrap(),
                }
                stream.flush().unwrap();
            }
        });
        let archive_url = format!("{base}/archive");
        let destination = std::env::temp_dir().join(format!(
            "insar-earthdata-oauth-return-{}.h5",
            std::process::id()
        ));
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        let request = TransferRequest::new(
            "oauth-return-task",
            "oauth-return-item",
            &archive_url,
            destination.clone(),
            Some(2),
        )
        .unwrap();
        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();

        let result = transfer.download(&request, &|| false, &|| false, &mut |_| {});

        assert_eq!(result.outcome, DownloadOutcome::Completed);
        server.join().unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"ok");
        fs::remove_file(destination).unwrap();
    }

    #[test]
    fn earthdata_auth_rejection_classification_is_host_and_auth_typed() {
        let policy = AuthRefreshPolicyId::EarthdataStandardV1;
        assert!(
            classify_auth_rejection(
                policy,
                401,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Bearer,
                None,
            )
            .refresh_candidate
        );
        assert!(
            classify_auth_rejection(
                policy,
                401,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Basic,
                None,
            )
            .refresh_candidate,
            "Earthdata's existing 401 classification must remain unchanged"
        );
        assert!(
            !classify_auth_rejection(
                policy,
                403,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Basic,
                None,
            )
            .refresh_candidate
        );
        assert!(
            !classify_auth_rejection(
                policy,
                401,
                ResponseHostClass::Unapproved,
                ResolvedAuthKind::Bearer,
                None,
            )
            .refresh_candidate
        );
        assert!(
            classify_auth_rejection(
                policy,
                403,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Bearer,
                Some(br#"Bearer error="invalid_token""#),
            )
            .refresh_candidate
        );
    }

    #[test]
    fn www_authenticate_refresh_semantics_are_bounded_ascii_and_exact() {
        let policy = AuthRefreshPolicyId::EarthdataStandardV1;
        for value in [
            vec![b'a'; 1025],
            b"Bearer error=\"not_invalid_token\"".to_vec(),
            b"Basic error=\"invalid_token\"".to_vec(),
            vec![0xff, b'a'],
        ] {
            assert!(
                !classify_auth_rejection(
                    policy,
                    403,
                    ResponseHostClass::Approved,
                    ResolvedAuthKind::Bearer,
                    Some(&value),
                )
                .refresh_candidate
            );
        }
    }

    #[test]
    fn cdse_auth_rejection_requires_approved_bearer_and_exact_invalid_token_evidence() {
        let policy = AuthRefreshPolicyId::CdseOAuthBearerV1;
        assert!(
            classify_auth_rejection(
                policy,
                401,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Bearer,
                None,
            )
            .refresh_candidate
        );
        for (status, host, auth, challenge) in [
            (
                401,
                ResponseHostClass::Approved,
                ResolvedAuthKind::None,
                None,
            ),
            (
                401,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Basic,
                None,
            ),
            (
                401,
                ResponseHostClass::Unapproved,
                ResolvedAuthKind::Bearer,
                None,
            ),
            (
                403,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Bearer,
                None,
            ),
            (
                403,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Basic,
                Some(&b"Bearer error=\"invalid_token\""[..]),
            ),
            (
                403,
                ResponseHostClass::Unapproved,
                ResolvedAuthKind::Bearer,
                Some(&b"Bearer error=\"invalid_token\""[..]),
            ),
            (
                404,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Bearer,
                Some(&b"Bearer error=\"invalid_token\""[..]),
            ),
            (
                429,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Bearer,
                Some(&b"Bearer error=\"invalid_token\""[..]),
            ),
            (
                500,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Bearer,
                Some(&b"Bearer error=\"invalid_token\""[..]),
            ),
        ] {
            assert!(
                !classify_auth_rejection(policy, status, host, auth, challenge).refresh_candidate,
                "unexpected CDSE refresh candidate for status {status}"
            );
        }
        assert!(
            classify_auth_rejection(
                policy,
                403,
                ResponseHostClass::Approved,
                ResolvedAuthKind::Bearer,
                Some(br#"Bearer error="invalid_token""#),
            )
            .refresh_candidate
        );
        for challenge in [
            b"Basic error=\"invalid_token\"".as_slice(),
            b"Bearer error=invalid_token".as_slice(),
            b"Bearer error=\"other\"".as_slice(),
            &[b'x'; 1025][..],
        ] {
            assert!(
                !classify_auth_rejection(
                    policy,
                    403,
                    ResponseHostClass::Approved,
                    ResolvedAuthKind::Bearer,
                    Some(challenge),
                )
                .refresh_candidate
            );
        }
    }

    #[test]
    fn resolved_transfer_redirects_require_policy_approval() {
        let initial = Url::parse("https://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value").unwrap();
        for candidate in [
            "https://catalogue.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value",
            "https://download.dataspace.copernicus.eu:444/other",
            "http://download.dataspace.copernicus.eu/odata/v1/Products(00000000-0000-4000-8000-000000000001)/$value",
            "https://user@download.dataspace.copernicus.eu/other",
            "https://download.dataspace.copernicus.eu/unrelated",
            "http://127.0.0.1:43123/fixture",
        ] {
            let candidate = Url::parse(candidate).unwrap();
            assert!(!resolved_redirect_allowed(
                AuthRefreshPolicyId::CdseOAuthBearerV1,
                true
            ));
            assert!(resolved_redirect_allowed(
                AuthRefreshPolicyId::EarthdataStandardV1,
                true
            ));
            assert!(!resolved_redirect_allowed(
                AuthRefreshPolicyId::EarthdataStandardV1,
                false
            ));
            assert_ne!(candidate, initial);
        }
    }

    #[test]
    fn usgs_signed_url_rejection_reresolves_once_only_for_the_approved_host_class() {
        let policy = AuthRefreshPolicyId::UsgsM2mSignedUrlV1;
        for status in [401, 403] {
            assert!(
                classify_auth_rejection(
                    policy,
                    status,
                    ResponseHostClass::Approved,
                    ResolvedAuthKind::None,
                    None,
                )
                .refresh_candidate
            );
        }
        for (status, host, auth) in [
            (401, ResponseHostClass::Unapproved, ResolvedAuthKind::None),
            (403, ResponseHostClass::Approved, ResolvedAuthKind::Bearer),
            (404, ResponseHostClass::Approved, ResolvedAuthKind::None),
        ] {
            assert!(!classify_auth_rejection(policy, status, host, auth, None).refresh_candidate);
        }
        assert!(resolved_redirect_allowed(policy, true));
        assert!(!resolved_redirect_allowed(policy, false));
    }

    #[test]
    fn resolved_earthdata_session_and_archive_share_one_admission_permit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let base = format!("http://{address}");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let server = thread::spawn(move || {
            for index in 0..4 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    request.push_str(&line);
                }
                captured.lock().unwrap().push(request);
                let mut stream = reader.into_inner();
                match index {
                    0 => {
                        let body = br#"{"access_token":"fixture-token"}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    }
                    1 => write!(
                        stream,
                        "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    2 => write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    _ => write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                    )
                    .unwrap(),
                }
                stream.flush().unwrap();
            }
        });

        let destination = std::env::temp_dir().join(format!(
            "insar-earthdata-session-admission-{}.zip",
            std::process::id()
        ));
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        let archive_url = format!("{base}/archive");
        let mut asset = ResolvedAsset::new(&archive_url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(2))
            .with_host_policy(ResolvedHostPolicy::EarthdataAsfV1);
        CredentialMaterial::basic("fixture-user", "fixture-password")
            .unwrap()
            .apply_to(&mut asset)
            .unwrap();
        let request = TransferRequest::new(
            "earthdata-session-task",
            "earthdata-session-item",
            &archive_url,
            destination.clone(),
            Some(2),
        )
        .unwrap();
        let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        transfer.resolved_earthdata_session_endpoints = ResolvedEarthdataSessionEndpoints::new(
            &format!("{base}/token"),
            &format!("{base}/authorize"),
            &format!("{base}/"),
        )
        .unwrap();
        let controller = TransferAdmissionController::compatibility();
        let admitted = AdmittedTransferBackend::new(&transfer, Arc::clone(&controller));
        let mut session = admitted.resolved_session();

        let result = admitted.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );

        assert!(matches!(
            result,
            ResolvedTransferResult::Finished(ref result)
                if result.outcome == DownloadOutcome::Completed
        ));
        server.join().unwrap();
        let requests = Arc::try_unwrap(requests).unwrap().into_inner().unwrap();
        assert_eq!(requests.len(), 4);
        let normalized = requests
            .iter()
            .map(|request| request.to_ascii_lowercase())
            .collect::<Vec<_>>();
        assert!(normalized[0].starts_with("post /token "));
        assert!(normalized[1].starts_with("get /authorize "));
        assert!(normalized[2].starts_with("get /login "));
        assert!(normalized[3].starts_with("get /archive "));
        assert!(normalized[..3]
            .iter()
            .all(|request| request.contains("authorization: basic ")));
        assert!(normalized[3].contains("authorization: bearer fixture-token"));
        assert!(normalized[3].contains("cookie: asf-urs=fixture-session"));
        assert!(normalized
            .iter()
            .all(|request| !request.contains("fixture-password")));
        let admission = controller.snapshot();
        assert_eq!(admission.grants, 1);
        assert_eq!(admission.releases, 1);
        assert_eq!(admission.active_global, 0);
        assert_eq!(fs::read(&destination).unwrap(), b"ok");
        fs::remove_file(destination).unwrap();
    }

    #[test]
    fn completed_earthdata_destination_skips_without_a_session_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let destination = std::env::temp_dir().join(format!(
            "insar-earthdata-session-skip-{}.zip",
            std::process::id()
        ));
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        fs::write(&destination, b"ok").unwrap();
        let archive_url = format!("{base}/archive");
        let mut asset = ResolvedAsset::new(&archive_url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(2))
            .with_host_policy(ResolvedHostPolicy::EarthdataAsfV1);
        CredentialMaterial::basic("fixture-user", "fixture-password")
            .unwrap()
            .apply_to(&mut asset)
            .unwrap();
        let request = TransferRequest::new(
            "earthdata-session-skip-task",
            "earthdata-session-skip-item",
            &archive_url,
            destination.clone(),
            Some(2),
        )
        .unwrap();
        let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        transfer.resolved_earthdata_session_endpoints = ResolvedEarthdataSessionEndpoints::new(
            &format!("{base}/token"),
            &format!("{base}/authorize"),
            &format!("{base}/"),
        )
        .unwrap();
        let controller = TransferAdmissionController::compatibility();
        let admitted = AdmittedTransferBackend::new(&transfer, Arc::clone(&controller));
        let mut session = admitted.resolved_session();

        let result = admitted.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );

        assert!(matches!(
            result,
            ResolvedTransferResult::Finished(ref result)
                if result.outcome == DownloadOutcome::Skipped
        ));
        assert!(matches!(
            listener.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        let admission = controller.snapshot();
        assert_eq!(admission.grants, 1);
        assert_eq!(admission.releases, 1);
        fs::remove_file(destination).unwrap();
    }

    #[test]
    fn alos2_earthdata_policy_rejects_redirect_before_a_second_request() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_url = format!("http://{}/target", target.local_addr().unwrap());
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let source_url =
            Url::parse(&format!("http://{}/source", source.local_addr().unwrap())).unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = source.accept().unwrap();
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
        });
        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();

        let destination = std::env::temp_dir().join(format!(
            "insar-alos2-rejected-redirect-{}.zip",
            std::process::id()
        ));
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        let asset = ResolvedAsset::new(source_url.as_str(), ResolvedMethod::Get)
            .unwrap()
            .with_host_policy(ResolvedHostPolicy::EarthdataAsfV1);
        let request = TransferRequest::new(
            "alos2-redirect-task",
            "alos2-redirect-item",
            source_url.as_str(),
            destination.clone(),
            Some(1),
        )
        .unwrap();
        let mut session = transfer.resolved_session();
        let result = transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataStandardV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );
        assert!(matches!(
            result,
            ResolvedTransferResult::Finished(ref result)
                if result.outcome == DownloadOutcome::Failed
        ));
        server.join().unwrap();
        assert!(matches!(
            target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        assert!(!destination.exists());
        assert!(!part_path(&destination).exists());
    }

    #[test]
    fn burst_session_polls_and_trusted_redirect_share_attempt_and_admission() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let tiff = b"II*\0\x08\0\0\0\0\0\0\0\0\0";
        let server = thread::spawn(move || {
            for index in 0..6 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    request.push_str(&line);
                }
                captured.lock().unwrap().push(request);
                let mut stream = reader.into_inner();
                match index {
                    0 => {
                        let body = br#"{"access_token":"fixture-token"}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    }
                    1 => write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    2 | 3 => write!(
                        stream,
                        "HTTP/1.1 202 Accepted\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    4 => write!(
                        stream,
                        "HTTP/1.1 307 Temporary Redirect\r\nLocation: /content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    _ => {
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: image/tiff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            tiff.len()
                        )
                        .unwrap();
                        stream.write_all(tiff).unwrap();
                    }
                }
                stream.flush().unwrap();
            }
        });

        let destination = std::env::temp_dir().join(format!(
            "insar-burst-session-loopback-{}.tiff",
            std::process::id()
        ));
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        let burst_url = format!("{base}/burst");
        let mut asset = ResolvedAsset::new(&burst_url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(tiff.len() as u64))
            .with_asset_role(AssetRole::Primary)
            .with_host_policy(ResolvedHostPolicy::EarthdataBurstLoopbackFixture);
        CredentialMaterial::basic("fixture-user", "fixture-password")
            .unwrap()
            .apply_to(&mut asset)
            .unwrap();
        let request = TransferRequest::new(
            "burst-session-task",
            "burst-session-item",
            &burst_url,
            destination.clone(),
            Some(tiff.len() as u64),
        )
        .unwrap();
        let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            max_accepted_polls: 2,
            accepted_poll_timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        transfer.resolved_earthdata_session_endpoints = ResolvedEarthdataSessionEndpoints::new(
            &format!("{base}/token"),
            &format!("{base}/authorize"),
            &format!("{base}/"),
        )
        .unwrap();
        let controller = TransferAdmissionController::compatibility();
        let admitted = AdmittedTransferBackend::new(&transfer, Arc::clone(&controller));
        let mut session = admitted.resolved_session();

        let result = admitted.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataBurstSessionV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );

        assert!(matches!(
            result,
            ResolvedTransferResult::Finished(ref result)
                if result.outcome == DownloadOutcome::Completed && result.attempt_count == 1
        ));
        server.join().unwrap();
        let requests = Arc::try_unwrap(requests).unwrap().into_inner().unwrap();
        assert_eq!(requests.len(), 6);
        let normalized = requests
            .iter()
            .map(|request| request.to_ascii_lowercase())
            .collect::<Vec<_>>();
        assert!(normalized[0].starts_with("post /token "));
        assert!(normalized[1].starts_with("get /authorize "));
        assert!(normalized[2].starts_with("get /burst "));
        assert!(normalized[3].starts_with("get /burst "));
        assert!(normalized[4].starts_with("get /burst "));
        assert!(normalized[5].starts_with("get /content "));
        assert!(normalized[..2]
            .iter()
            .all(|request| request.contains("authorization: basic ")));
        assert!(normalized[2..]
            .iter()
            .all(|request| request.contains("cookie: asf-urs=fixture-session")));
        assert!(normalized[2..]
            .iter()
            .all(|request| request.contains("authorization: bearer fixture-token")));
        assert_eq!(session.attempts_used(), 1);
        let admission = controller.snapshot();
        assert_eq!(admission.grants, 1);
        assert_eq!(admission.releases, 1);
        assert_eq!(fs::read(&destination).unwrap(), tiff);
        fs::remove_file(destination).unwrap();
    }

    #[test]
    fn burst_untrusted_redirect_is_rejected_before_second_request() {
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_url = format!("http://{}/stolen", target.local_addr().unwrap());
        let source = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", source.local_addr().unwrap());
        let server = thread::spawn(move || {
            for index in 0..3 {
                let (stream, _) = source.accept().unwrap();
                let mut reader = BufReader::new(stream);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut stream = reader.into_inner();
                match index {
                    0 => {
                        let body = br#"{"access_token":"fixture-token"}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    }
                    1 => write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    _ => write!(
                        stream,
                        "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                }
                stream.flush().unwrap();
            }
        });

        let destination = std::env::temp_dir().join(format!(
            "insar-burst-untrusted-redirect-{}.tiff",
            std::process::id()
        ));
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        let burst_url = format!("{base}/burst");
        let mut asset = ResolvedAsset::new(&burst_url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(10))
            .with_asset_role(AssetRole::Primary)
            .with_host_policy(ResolvedHostPolicy::EarthdataBurstLoopbackFixture);
        CredentialMaterial::basic("fixture-user", "fixture-password")
            .unwrap()
            .apply_to(&mut asset)
            .unwrap();
        let request = TransferRequest::new(
            "burst-untrusted-task",
            "burst-untrusted-item",
            &burst_url,
            destination.clone(),
            Some(10),
        )
        .unwrap();
        let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            ..TransferOptions::default()
        })
        .unwrap();
        transfer.resolved_earthdata_session_endpoints = ResolvedEarthdataSessionEndpoints::new(
            &format!("{base}/token"),
            &format!("{base}/authorize"),
            &format!("{base}/"),
        )
        .unwrap();
        let mut session = transfer.resolved_session();

        let result = transfer.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataBurstSessionV1,
            &mut session,
            &|| false,
            &|| false,
            &mut |_| {},
        );

        assert!(matches!(
            result,
            ResolvedTransferResult::Finished(ref result)
                if result.outcome == DownloadOutcome::Failed && result.attempt_count == 1
        ));
        server.join().unwrap();
        assert!(matches!(
            target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        assert!(!destination.exists());
        assert!(!part_path(&destination).exists());
    }

    #[test]
    fn burst_accepted_poll_cancellation_is_bounded_and_creates_no_part() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_after_accepted = Arc::clone(&cancelled);
        let server = thread::spawn(move || {
            for index in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut stream = reader.into_inner();
                match index {
                    0 => {
                        let body = br#"{"access_token":"fixture-token"}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    }
                    1 => write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nSet-Cookie: asf-urs=fixture-session; Path=/; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                    _ => write!(
                        stream,
                        "HTTP/1.1 202 Accepted\r\nRetry-After: 30\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap(),
                }
                stream.flush().unwrap();
                if index == 2 {
                    cancel_after_accepted.store(true, Ordering::SeqCst);
                }
            }
        });

        let destination = std::env::temp_dir().join(format!(
            "insar-burst-accepted-cancel-{}.tiff",
            std::process::id()
        ));
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(part_path(&destination));
        let burst_url = format!("{base}/burst");
        let mut asset = ResolvedAsset::new(&burst_url, ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(14))
            .with_asset_role(AssetRole::Primary)
            .with_host_policy(ResolvedHostPolicy::EarthdataBurstLoopbackFixture);
        CredentialMaterial::basic("fixture-user", "fixture-password")
            .unwrap()
            .apply_to(&mut asset)
            .unwrap();
        let request = TransferRequest::new(
            "burst-accepted-cancel-task",
            "burst-accepted-cancel-item",
            &burst_url,
            destination.clone(),
            Some(14),
        )
        .unwrap();
        let mut transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_secs(2),
            max_accepted_polls: 2,
            accepted_poll_timeout: Duration::from_secs(60),
            ..TransferOptions::default()
        })
        .unwrap();
        transfer.resolved_earthdata_session_endpoints = ResolvedEarthdataSessionEndpoints::new(
            &format!("{base}/token"),
            &format!("{base}/authorize"),
            &format!("{base}/"),
        )
        .unwrap();
        let controller = TransferAdmissionController::compatibility();
        let admitted = AdmittedTransferBackend::new(&transfer, Arc::clone(&controller));
        let mut session = admitted.resolved_session();
        let started = Instant::now();

        let result = admitted.download_resolved(
            &request,
            &asset,
            AuthRefreshPolicyId::EarthdataBurstSessionV1,
            &mut session,
            &|| cancelled.load(Ordering::SeqCst),
            &|| false,
            &mut |_| {},
        );

        server.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(matches!(
            result,
            ResolvedTransferResult::Finished(ref result)
                if result.outcome == DownloadOutcome::Interrupted && result.attempt_count == 1
        ));
        assert_eq!(session.attempts_used(), 1);
        let admission = controller.snapshot();
        assert_eq!(admission.grants, 1);
        assert_eq!(admission.releases, 1);
        assert!(!destination.exists());
        assert!(!part_path(&destination).exists());
    }

    #[test]
    fn burst_tiff_validation_rejects_size_and_structure_without_promotion() {
        for (label, body, expected_size) in [
            ("tiff-non", b"not a tiff".as_slice(), 10),
            ("tiff-truncated", b"II*\0\x08\0\0\0".as_slice(), 8),
        ] {
            let evidence = run_burst_content_fixture(
                label,
                AssetRole::Primary,
                body,
                expected_size,
                AuthRefreshPolicyId::EarthdataBurstSessionV1,
            );
            assert_eq!(evidence.result.outcome, DownloadOutcome::Failed, "{label}");
            assert!(!evidence.promoted, "invalid TIFF was promoted for {label}");
            assert!(
                !evidence.part_exists,
                "invalid TIFF part survived for {label}"
            );
        }
    }

    #[test]
    fn burst_xml_validation_rejects_unsafe_or_invalid_bodies_without_promotion() {
        for (label, body) in [
            ("xml-html", b"<html><form>login</form></html>".as_slice()),
            ("xml-login", b"<login>Earthdata</login>".as_slice()),
            (
                "xml-error",
                b"<Error><Code>AccessDenied</Code></Error>".as_slice(),
            ),
            ("xml-malformed", b"<burst><id></burst>".as_slice()),
            ("xml-invalid-utf8", b"<burst>\xff</burst>".as_slice()),
            (
                "xml-unsafe",
                br#"<!DOCTYPE burst [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><burst>&xxe;</burst>"#
                    .as_slice(),
            ),
        ] {
            let evidence = run_burst_content_fixture(
                label,
                AssetRole::Metadata,
                body,
                body.len() as u64,
                AuthRefreshPolicyId::EarthdataBurstSessionV1,
            );
            assert_eq!(evidence.result.outcome, DownloadOutcome::Failed, "{label}");
            assert!(!evidence.promoted, "invalid XML was promoted for {label}");
            assert!(!evidence.part_exists, "invalid XML part survived for {label}");
        }
    }

    #[test]
    fn burst_xml_validation_accepts_well_formed_safe_metadata() {
        const XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?><burst:product xmlns:burst="urn:asf:burst"><burst:id>S1</burst:id><burst:label>A &amp; B</burst:label></burst:product>"#;
        let evidence = run_burst_content_fixture(
            "xml-valid",
            AssetRole::Metadata,
            XML,
            XML.len() as u64,
            AuthRefreshPolicyId::EarthdataBurstSessionV1,
        );
        assert_eq!(evidence.result.outcome, DownloadOutcome::Completed);
        assert!(evidence.promoted);
        assert!(!evidence.part_exists);
    }

    #[test]
    fn remote_response_size_authority_is_explicit_and_defaults_to_strict() {
        let strict = ResolvedAsset::new("https://example.com/object", ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(5));
        assert!(!super::resolved_size_is_remote_authoritative(
            AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
            &strict,
            None,
        ));

        let remote = strict.with_remote_response_size_authority();
        assert!(super::resolved_size_is_remote_authoritative(
            AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
            &remote,
            None,
        ));
    }

    #[test]
    fn burst_catalog_size_is_advisory_but_standard_expected_size_remains_authoritative() {
        const VALID_TIFF: &[u8] = b"II*\0\x08\0\0\0\0\0\0\0\0\0";
        let burst = run_burst_content_fixture(
            "tiff-advisory-size",
            AssetRole::Primary,
            VALID_TIFF,
            VALID_TIFF.len() as u64 + 1,
            AuthRefreshPolicyId::EarthdataBurstSessionV1,
        );
        assert_eq!(
            burst.result.outcome,
            DownloadOutcome::Completed,
            "{:?}",
            burst.result.failure
        );
        assert!(burst.promoted);
        assert!(!burst.part_exists);

        let standard = run_burst_content_fixture(
            "tiff-standard-size",
            AssetRole::Primary,
            VALID_TIFF,
            VALID_TIFF.len() as u64 + 1,
            AuthRefreshPolicyId::EarthdataStandardV1,
        );
        assert_eq!(standard.result.outcome, DownloadOutcome::Failed);
        let failure = standard.result.failure.unwrap();
        assert_eq!(failure.code, "DL002");
        assert!(failure
            .reason_zh
            .contains("subtype=expected_remote_total_mismatch"));
        assert!(!standard.promoted);
        assert!(!standard.part_exists);
    }

    #[test]
    fn burst_no_checksum_part_resumes_only_within_the_same_execution() {
        let destination = std::env::temp_dir().join(format!(
            "insar-burst-part-scope-{}.tiff",
            std::process::id()
        ));
        let part = part_path(&destination);
        let _ = fs::remove_file(&destination);
        let _ = fs::remove_file(&part);

        let mut same_execution = super::ResolvedTransferSession::new(1).unwrap();
        same_execution.observe_initial_part(&destination);
        fs::write(&part, b"II*\0").unwrap();
        assert_eq!(
            super::inspect_resolved_for_session(&same_execution, &destination, None, None).unwrap(),
            crate::FilePreparation::Resume { offset: 4 }
        );

        let mut restarted_execution = super::ResolvedTransferSession::new(1).unwrap();
        restarted_execution.observe_initial_part(&destination);
        assert_eq!(
            super::inspect_resolved_for_session(&restarted_execution, &destination, None, None)
                .unwrap(),
            crate::FilePreparation::Restart
        );
        let _ = fs::remove_file(part);
    }

    #[test]
    fn only_quoted_non_weak_entity_tags_are_strong_validators() {
        assert_eq!(strong_etag("\"stable\""), Some("\"stable\""));
        for invalid in ["W/\"weak\"", "not-quoted", "\"unterminated", "\"\""] {
            assert_eq!(strong_etag(invalid), None);
        }
    }
    /// L5 recheck 2026-10-19: rustls reports an unverifiable server certificate
    /// as "invalid peer certificate: ..." inside the I/O error of the connect
    /// failure; only that cause is a TLS certificate failure.
    #[test]
    fn a_tls_certificate_failure_is_recognised_in_the_error_chain_only() {
        #[derive(Debug)]
        struct Rustls(&'static str);
        impl std::fmt::Display for Rustls {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.0)
            }
        }
        impl std::error::Error for Rustls {}
        #[derive(Debug)]
        struct Connect(std::io::Error);
        impl std::fmt::Display for Connect {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("client error (Connect)")
            }
        }
        impl std::error::Error for Connect {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let tls = Connect(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            Rustls("invalid peer certificate: UnknownIssuer"),
        ));
        assert!(super::request_error_is_tls_certificate(&tls));
        let alert = Connect(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            Rustls("received fatal alert: HandshakeFailure"),
        ));
        assert!(!super::request_error_is_tls_certificate(&alert));
        assert!(super::error_chain_names_tls_certificate(&[
            "client error (Connect)".to_owned(),
            "invalid peer certificate: UnknownIssuer".to_owned(),
        ]));
        assert!(!super::error_chain_names_tls_certificate(&[
            "operation timed out".to_owned()
        ]));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let refused = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/"))
            .send()
            .expect_err("closed loopback port must fail");
        assert!(refused.is_connect());
        assert!(!super::request_error_is_tls_certificate(&refused));
    }

    #[test]
    fn a_tls_certificate_transport_failure_names_its_cause_first() {
        use super::TransportFailureCause;
        assert_eq!(
            super::transport_failure_reason(
                "error sending request",
                false,
                TransportFailureCause::TlsCertificate
            ),
            "无法验证服务器证书（本机证书库无法读取或网络被拦截），断点已保留。HTTP transport error: error sending request [transport_cause=tls_certificate source=tls_certificate]"
        );
        // Downloader D1d: no usable response (timeout, connection, body) is a
        // network cause the task card can name.
        assert_eq!(
            super::transport_failure_reason(
                "error sending request",
                true,
                TransportFailureCause::Network
            ),
            "HTTP transport error: error sending request secret_material_redacted=true [source=network]"
        );
        assert_eq!(
            super::transport_failure_reason(
                "error decoding response body",
                false,
                TransportFailureCause::Other
            ),
            "HTTP transport error: error decoding response body"
        );
    }

    #[test]
    fn transport_failure_preserves_reqwest_detail_without_signed_query_secrets() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let error = client
            .get(format!("http://{address}/archive?token=fixture-secret"))
            .send()
            .expect_err("closed loopback port must fail");
        let failure = super::AttemptError::transport(error).failure;

        assert_eq!(failure.code, "DL005");
        assert!(failure.retryable);
        assert!(
            failure.reason_zh.contains("无法建立到远端服务器的网络连接"),
            "expected concrete Chinese network cause, got: {}",
            failure.reason_zh
        );
        assert!(
            failure.reason_zh.contains("HTTP transport error:"),
            "expected sanitized transport detail, got: {}",
            failure.reason_zh
        );
        assert!(!failure.reason_zh.contains("fixture-secret"));
        assert!(!failure.reason_zh.contains("token=fixture-secret"));
        assert!(!failure.reason_zh.contains("http://127.0.0.1"));
        // Downloader D1d: a refused connection is a network cause.
        assert!(
            failure.reason_zh.ends_with(" [source=network]"),
            "{}",
            failure.reason_zh
        );
    }

    #[test]
    fn dl005_preserves_real_sanitized_causes_for_http_status_stream_io_and_stall() {
        let http503 = super::AttemptError::transient_for_status(
            reqwest::StatusCode::SERVICE_UNAVAILABLE,
            None,
        )
        .failure;
        assert_eq!(http503.code, "DL005");
        assert!(http503.reason_zh.contains("HTTP 503"));
        assert!(!http503.reason_zh.contains("下载连接暂时失败"));

        let http429 = super::AttemptError::transient_for_status(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            Some(Duration::from_secs(45)),
        )
        .failure;
        assert_eq!(http429.code, "DL005");
        assert!(http429.reason_zh.contains("HTTP 429"));
        assert!(http429.reason_zh.contains("45s"));

        let reset_io = std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset while reading https://secret.host/file?token=abc from /Users/alice/private.h5",
        );
        let reset_failure = super::AttemptError::stream_read(&reset_io).failure;
        assert_eq!(reset_failure.code, "DL005");
        assert!(reset_failure.reason_zh.contains("ConnectionReset"));
        assert!(reset_failure.reason_zh.contains("重置"));
        assert!(!reset_failure.reason_zh.contains("https://secret.host"));
        assert!(!reset_failure.reason_zh.contains("token=abc"));
        assert!(!reset_failure.reason_zh.contains("/Users/alice"));

        let stall_failure = super::AttemptError::stall_exhausted(4, 3).failure;
        assert_eq!(stall_failure.code, "DL005");
        assert!(stall_failure.reason_zh.contains("停滞"));
        assert!(!stall_failure.reason_zh.contains("下载连接暂时失败"));

        let (scrubbed, redacted) = super::sanitize_download_diagnostic_text(
            "failed fetching https://example.com/a.h5?access_token=xyz saving /Users/bob/data.h5",
        );
        assert!(redacted);
        assert!(!scrubbed.contains("https://example.com"));
        assert!(!scrubbed.contains("access_token=xyz"));
        assert!(!scrubbed.contains("/Users/bob"));
    }

    fn encode_test_base64url(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        let mut i = 0;
        while i < bytes.len() {
            let b0 = bytes[i];
            let b1 = bytes.get(i + 1).copied().unwrap_or(0);
            let b2 = bytes.get(i + 2).copied().unwrap_or(0);
            out.push(TABLE[usize::from(b0 >> 2)] as char);
            out.push(TABLE[usize::from(((b0 & 0x03) << 4) | (b1 >> 4))] as char);
            if i + 1 < bytes.len() {
                out.push(TABLE[usize::from(((b1 & 0x0f) << 2) | (b2 >> 6))] as char);
            }
            if i + 2 < bytes.len() {
                out.push(TABLE[usize::from(b2 & 0x3f)] as char);
            }
            i += 3;
        }
        out
    }

    fn make_test_jwt(uid: &str, exp: u64) -> String {
        let header = encode_test_base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let payload_json = format!(r#"{{"type":"User","uid":"{uid}","exp":{exp}}}"#);
        let payload = encode_test_base64url(payload_json.as_bytes());
        format!("{header}.{payload}.sig")
    }

    #[test]
    fn earthdata_bearer_probe_validates_jwt_profile_rejection_and_expiry() {
        use super::{EarthdataAuthProbeOutcome, ProxyPolicy, TransferAuth};
        use crate::http_transfer::SecretString;

        let future_exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
        let past_exp = 1_500_000_000_u64;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            for idx in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                }
                let resp = match idx {
                    0 => {
                        assert!(
                            request_line.starts_with("GET /api/users/fixture.user "),
                            "{request_line}"
                        );
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\n\r\n{\"uid\":\"ok\"}\r\n"
                    }
                    1 => {
                        assert!(
                            request_line.starts_with("GET /api/users/fixture.user "),
                            "{request_line}"
                        );
                        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n"
                    }
                    _ => "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
                };
                stream.write_all(resp.as_bytes()).unwrap();
            }
        });

        let mut transfer =
            HttpTransfer::new_with_proxy_policy(TransferOptions::default(), ProxyPolicy::Direct)
                .unwrap();
        transfer
            .set_resolved_earthdata_session_endpoints_for_test(
                &format!("http://{addr}/api/users/find_or_create_token"),
                &format!("http://{addr}/oauth/authorize?redirect_uri=http://{addr}/cb"),
                &format!("http://{addr}/"),
            )
            .unwrap();

        let valid_jwt = make_test_jwt("fixture.user", future_exp);
        assert_eq!(
            transfer
                .probe_earthdata_auth(&TransferAuth::Bearer(SecretString::new(valid_jwt.clone()))),
            EarthdataAuthProbeOutcome::Valid
        );
        assert_eq!(
            transfer
                .probe_earthdata_auth(&TransferAuth::Bearer(SecretString::new(valid_jwt.clone()))),
            EarthdataAuthProbeOutcome::Invalid
        );
        assert_eq!(
            transfer.probe_earthdata_auth(&TransferAuth::Bearer(SecretString::new(valid_jwt))),
            EarthdataAuthProbeOutcome::Unavailable
        );
        // Expired JWT returns Invalid immediately without hitting network
        let expired_jwt = make_test_jwt("fixture.user", past_exp);
        assert_eq!(
            transfer.probe_earthdata_auth(&TransferAuth::Bearer(SecretString::new(expired_jwt))),
            EarthdataAuthProbeOutcome::Invalid
        );
        handle.join().unwrap();
    }

    #[cfg(feature = "nisar-subset-production")]
    #[test]
    fn range_client_allows_steady_slow_stream_beyond_timeout_and_rejects_idle_stall() {
        use std::io::Read;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for idx in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                if idx == 0 {
                    write!(
                        stream,
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-15/16\r\nContent-Length: 16\r\nETag: \"steady\"\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                    stream.flush().unwrap();
                    for chunk in [b"aaaa", b"bbbb", b"cccc", b"dddd"] {
                        thread::sleep(Duration::from_millis(90));
                        stream.write_all(chunk).unwrap();
                        stream.flush().unwrap();
                    }
                } else {
                    write!(
                        stream,
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-15/16\r\nContent-Length: 16\r\nETag: \"stall\"\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                    stream.flush().unwrap();
                    stream.write_all(b"aaaa").unwrap();
                    stream.flush().unwrap();
                    thread::sleep(Duration::from_millis(450));
                    let _ = stream.write_all(b"bbbbbbbbbbbb");
                }
            }
        });

        let transfer = HttpTransfer::new_without_system_proxy(TransferOptions {
            max_attempts: 1,
            backoff: Duration::ZERO,
            timeout: Duration::from_millis(250),
            ..TransferOptions::default()
        })
        .unwrap();
        let asset = ResolvedAsset::new(&format!("{base}/range"), ResolvedMethod::Get)
            .unwrap()
            .with_expected_size(Some(16))
            .with_host_policy(ResolvedHostPolicy::LoopbackFixture);

        match transfer.send_resolved_exact_range_once_trusted_redirect(
            &asset,
            0,
            16,
            r#""steady""#,
            &|| false,
        ) {
            ExactRangeSendOutcome::Response { mut response, .. } => {
                let mut body = Vec::new();
                response.read_to_end(&mut body).unwrap();
                assert_eq!(body, b"aaaabbbbccccdddd");
            }
            other => panic!(
                "expected steady stream response, got {}",
                outcome_kind(&other)
            ),
        }

        match transfer.send_resolved_exact_range_once_trusted_redirect(
            &asset,
            0,
            16,
            r#""stall""#,
            &|| false,
        ) {
            ExactRangeSendOutcome::Response { mut response, .. } => {
                let mut body = Vec::new();
                let err = response.read_to_end(&mut body).unwrap_err();
                let timed_out = matches!(
                    err.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) || err.get_ref().is_some_and(|inner| {
                    let message = inner.to_string().to_ascii_lowercase();
                    message.contains("timed out")
                        || inner.downcast_ref::<reqwest::Error>().is_some_and(|req| {
                            req.is_timeout() || format!("{req:?}").contains("TimedOut")
                        })
                });
                assert!(
                    timed_out,
                    "expected idle read timeout error in chain, got {err:?}"
                );
            }
            other => panic!(
                "expected response before stall, got {}",
                outcome_kind(&other)
            ),
        }
        server.join().unwrap();
    }

    #[test]
    fn resolve_segment_policy_forces_single_stream_for_bhoonidhi_and_honors_apisix_reset() {
        use reqwest::header::{HeaderMap, HeaderValue};

        let bhoonidhi_url = Url::parse("https://bhoonidhi-api.nrsc.gov.in/download").unwrap();
        let earthdata_url = Url::parse("https://datapool.asf.alaska.edu/file.h5").unwrap();

        assert_eq!(
            HttpTransfer::resolve_segment_policy(
                Some(AuthRefreshPolicyId::BhoonidhiOAuthBearerV1),
                &earthdata_url,
            ),
            (
                "isro_bhoonidhi",
                crate::segment_planner::MaximumSegmentPolicy::AtMost(
                    crate::segment_planner::SegmentLevel::One,
                ),
            )
        );
        assert_eq!(
            HttpTransfer::resolve_segment_policy(None, &earthdata_url),
            ("", crate::segment_planner::MaximumSegmentPolicy::Unbounded)
        );

        let mut apisix_headers = HeaderMap::new();
        apisix_headers.insert("x-ratelimit-reset", HeaderValue::from_static("35"));
        assert_eq!(
            super::HttpTransfer::bhoonidhi_aware_retry_after(
                &bhoonidhi_url,
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                &apisix_headers,
            ),
            Some(Duration::from_secs(36))
        );
        assert_eq!(
            super::HttpTransfer::bhoonidhi_aware_retry_after(
                &earthdata_url,
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                &apisix_headers,
            ),
            None
        );
    }
}

#[cfg(test)]
#[path = "http_transfer_recovered_tests.rs"]
mod recovered_part_tests;

#[cfg(test)]
#[path = "http_transfer_earthdata_session_tests.rs"]
mod earthdata_session_tests;
