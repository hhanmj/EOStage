//! Native execution of a manually supplied GACOS result link.
//!
//! The result link is an explicit user input. Its complete URL remains in a
//! process-owned cache (or the reviewed desktop credential profile) and is
//! never copied into a task snapshot, operation payload, history event, or
//! diagnostic. The task itself freezes only the parent request identity, a
//! digest of the validated link, and a safe result filename.

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, RANGE};
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroizing;

use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadOutcome,
    FrozenTaskSnapshot, ProxyPolicy, TaskOperationDescriptor,
};
use insar_download::{
    part_path, AuthRefreshPolicyId, DownloadWork, DownloadWorker, HttpTransfer,
    NisarExecutionPlanFreezeAuthority, RemoteValidator, ResolvedAsset, ResolvedMethod,
    ResolvedTransferResult, TaskTransferSample, TransferOptions, TransferRequest, TransferUpdate,
    WorkerControl,
};
use insar_provider::gacos_artifact::{GACOS_MAX_ARTIFACT_BYTES, GACOS_MAX_FILES_PER_ADMISSION};
use insar_provider::gacos_delivery::{
    validate_manual_result_url, GacosBoundingBox, GacosManualResultLink, GACOS_MAX_RESPONSE_BYTES,
    GACOS_MAX_RESULT_URL_BYTES,
};

use super::gacos_import_worker::{
    acquire_import_lock, cancelled_result, emit_progress, ensure_output_root, now,
    publish_materialized_gacos_staging, staging_path, task_key, FrozenGacosFile, FrozenGacosImport,
    ImportExecutionError, ImportFailure, ImportResult,
};
use super::DemOperationPort;

const GACOS_RESULT_WORKER: &str = "gacos_fetch_result";
const GACOS_RESULT_MANIFEST_KEY: &str = "gacos_result_manifest";
const GACOS_RESULT_PROVIDER: &str = "gacos.manual-result";
const GACOS_RESULT_SCHEMA_VERSION: u32 = 1;
const GACOS_RESULT_MAX_ID_BYTES: usize = 128;
const GACOS_RESULT_MAX_FILE_NAME_BYTES: usize = 128;
const GACOS_FETCH_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const GACOS_FETCH_TOTAL_TIMEOUT: Duration = Duration::from_secs(45);
const GACOS_TRANSFER_TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const GACOS_FETCH_CHUNK_BYTES: usize = 64 * 1024;
const GACOS_FETCH_RECEIPT_SCHEMA_VERSION: u32 = 1;
const GACOS_FETCH_RECEIPT_MAX_BYTES: u64 = 64 * 1024;
const GACOS_PARTIAL_RECEIPT_SCHEMA_VERSION: u32 = 1;
const GACOS_PARTIAL_RECEIPT_MAX_BYTES: u64 = 64 * 1024;

/// Typed failures emitted by the result-link cache and remote worker.
///
/// Variants intentionally carry no URL, query, response body, username, or
/// credential material. HTTP status values are safe bounded protocol facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GacosResultError {
    InvalidLink,
    UnreviewedLinkHost,
    LinkHashMismatch,
    LinkConflict,
    LinkRequired,
    CredentialUnavailable,
    InvalidManifest,
    InvalidOperation,
    InvalidProvider,
    PreflightTransport,
    PreflightRedirect,
    PreflightStatus(u16),
    PreflightResponseTooLarge,
    PreflightRangeInvalid,
    PreflightSizeMissing,
    PreflightSizeTooLarge,
    PreflightValidatorMissing,
    PreflightBodyInvalid,
    TransferUnavailable,
    TransferFailed,
    AuthorizationRejected(u16),
    TransferTooLarge,
    TransferIdentityChanged,
    TransferEvidenceMissing,
    ArchiveMaterialization,
    Archive(super::gacos_result_archive::GacosResultArchiveError),
    ArchiveIdentityMismatch,
    ArchiveTooManyFiles,
    ImportFailed,
    RasterInspectionUnavailable,
    RasterInspectionInvalid,
    Cancelled,
}

impl GacosResultError {
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::InvalidLink => "GACOS_RESULT_LINK_INVALID",
            Self::UnreviewedLinkHost => "GACOS_RESULT_LINK_HOST_UNREVIEWED",
            Self::LinkHashMismatch => "GACOS_RESULT_LINK_HASH_MISMATCH",
            Self::LinkConflict => "GACOS_RESULT_LINK_CONFLICT",
            Self::LinkRequired => "GACOS_RESULT_LINK_REQUIRED",
            Self::CredentialUnavailable => "GACOS_RESULT_LINK_CREDENTIAL_UNAVAILABLE",
            Self::InvalidManifest => "GACOS_RESULT_MANIFEST_INVALID",
            Self::InvalidOperation => "GACOS_RESULT_OPERATION_INVALID",
            Self::InvalidProvider => "GACOS_RESULT_PROVIDER_INVALID",
            Self::PreflightTransport => "GACOS_RESULT_PREFLIGHT_TRANSPORT",
            Self::PreflightRedirect => "GACOS_RESULT_REDIRECT_REJECTED",
            Self::PreflightStatus(_) => "GACOS_RESULT_PREFLIGHT_STATUS",
            Self::PreflightResponseTooLarge => "GACOS_RESULT_PREFLIGHT_RESPONSE_TOO_LARGE",
            Self::PreflightRangeInvalid => "GACOS_RESULT_PREFLIGHT_RANGE_INVALID",
            Self::PreflightSizeMissing => "GACOS_RESULT_SIZE_MISSING",
            Self::PreflightSizeTooLarge => "GACOS_RESULT_SIZE_TOO_LARGE",
            Self::PreflightValidatorMissing => "GACOS_RESULT_STRONG_VALIDATOR_REQUIRED",
            Self::PreflightBodyInvalid => "GACOS_RESULT_PREFLIGHT_BODY_INVALID",
            Self::TransferUnavailable => "GACOS_RESULT_TRANSFER_UNAVAILABLE",
            Self::TransferFailed => "GACOS_RESULT_TRANSFER_FAILED",
            Self::AuthorizationRejected(_) => "GACOS_RESULT_AUTH_REJECTED",
            Self::TransferTooLarge => "GACOS_RESULT_SIZE_TOO_LARGE",
            Self::TransferIdentityChanged => "GACOS_RESULT_REMOTE_CHANGED",
            Self::TransferEvidenceMissing => "GACOS_RESULT_TRANSFER_EVIDENCE_MISSING",
            Self::ArchiveMaterialization => "GACOS_RESULT_ARCHIVE_UNSUPPORTED",
            Self::Archive(error) => error.code(),
            Self::ArchiveIdentityMismatch => "GACOS_RESULT_ARCHIVE_IDENTITY_MISMATCH",
            Self::ArchiveTooManyFiles => "GACOS_RESULT_ARCHIVE_TOO_MANY_FILES",
            Self::ImportFailed => "GACOS_RESULT_IMPORT_FAILED",
            Self::RasterInspectionUnavailable => "GACOS_RESULT_RASTER_INSPECTION_UNAVAILABLE",
            Self::RasterInspectionInvalid => "GACOS_RESULT_RASTER_INSPECTION_INVALID",
            Self::Cancelled => "GACOS_RESULT_CANCELLED",
        }
    }

    pub(super) const fn reason_zh(self) -> &'static str {
        match self {
            Self::InvalidLink => "GACOS 结果链接无效。",
            Self::UnreviewedLinkHost => "GACOS 结果链接主机未通过审核。",
            Self::LinkHashMismatch => "GACOS 结果链接与冻结身份不一致。",
            Self::LinkConflict => "GACOS 结果链接身份发生冲突，未覆盖已有值。",
            Self::LinkRequired => "需要重新粘贴 GACOS 结果链接后才能继续。",
            Self::CredentialUnavailable => "GACOS 结果链接的桌面凭据不可用。",
            Self::InvalidManifest => "GACOS 结果任务清单无效。",
            Self::InvalidOperation => "GACOS 结果任务操作声明无效。",
            Self::InvalidProvider => "GACOS 结果任务来源不受支持。",
            Self::PreflightTransport => "GACOS 结果链接预检连接失败。",
            Self::PreflightRedirect => "GACOS 结果链接预检拒绝重定向。",
            Self::PreflightStatus(_) => "GACOS 结果链接预检返回了不可接受的状态。",
            Self::PreflightResponseTooLarge => "GACOS 结果链接预检响应超过大小限制。",
            Self::PreflightRangeInvalid => "GACOS 结果链接未返回严格的单字节范围证据。",
            Self::PreflightSizeMissing => "GACOS 结果缺少可靠的总大小证据。",
            Self::PreflightSizeTooLarge => "GACOS 结果超过受控大小上限。",
            Self::PreflightValidatorMissing => "GACOS 结果缺少强 ETag 证据。",
            Self::PreflightBodyInvalid => "GACOS 结果预检响应体无效。",
            Self::TransferUnavailable => "GACOS 结果下载器不可用。",
            Self::TransferFailed => "GACOS 结果下载失败，未发布产物。",
            Self::AuthorizationRejected(_) => "GACOS 结果链接拒绝了当前请求。",
            Self::TransferTooLarge => "GACOS 结果超过受控大小上限。",
            Self::TransferIdentityChanged => "GACOS 结果在传输期间发生改变，未发布产物。",
            Self::TransferEvidenceMissing => "GACOS 结果缺少可恢复传输证据。",
            Self::ArchiveMaterialization | Self::Archive(_) => {
                "GACOS 结果包装格式或内容无法安全展开。"
            }
            Self::ArchiveIdentityMismatch => "GACOS 已有归档与当前结果身份不一致，未覆盖。",
            Self::ArchiveTooManyFiles => "GACOS 结果文件数量超过受控上限。",
            Self::ImportFailed => "GACOS 结果未通过完整性或科学验证。",
            Self::RasterInspectionUnavailable => "GACOS GeoTIFF 的 native GIS 检查不可用。",
            Self::RasterInspectionInvalid => "GACOS GeoTIFF 未通过 native GIS 检查。",
            Self::Cancelled => "GACOS 结果任务已取消。",
        }
    }

    pub(super) const fn retryable(self) -> bool {
        match self {
            Self::PreflightStatus(code) => matches!(code, 408 | 429 | 500..=599),
            Self::Archive(error) => error.retryable(),
            Self::PreflightTransport
            | Self::TransferUnavailable
            | Self::TransferFailed
            | Self::TransferEvidenceMissing => true,
            _ => false,
        }
    }
}

#[derive(Default)]
struct LinkCacheState {
    links: HashMap<String, Zeroizing<String>>,
    desktop_credentials: bool,
}

/// Process-owned result-link cache. The URL is never serializable or logged.
pub(super) struct GacosResultLinkCache {
    state: Mutex<LinkCacheState>,
    transport: Arc<dyn GacosResultTransport>,
}

impl Default for GacosResultLinkCache {
    fn default() -> Self {
        Self::new(false, Arc::new(HttpTransferGacosResultTransport::new()))
    }
}

impl std::fmt::Debug for GacosResultLinkCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self
            .state
            .lock()
            .map(|state| state.links.len())
            .unwrap_or(0);
        formatter
            .debug_struct("GacosResultLinkCache")
            .field("cached_links", &count)
            .field("desktop_credentials", &"<redacted>")
            .finish()
    }
}

impl GacosResultLinkCache {
    fn new(desktop_credentials: bool, transport: Arc<dyn GacosResultTransport>) -> Self {
        Self {
            state: Mutex::new(LinkCacheState {
                links: HashMap::new(),
                desktop_credentials,
            }),
            transport,
        }
    }

    /// Production constructor. The URL is persisted only in the dedicated
    /// reviewed desktop credential profile, never in TaskStore.
    pub(super) fn with_desktop_credentials() -> Self {
        Self::new(true, Arc::new(HttpTransferGacosResultTransport::new()))
    }

    /// Later result fetches use the saved ordinary network route.
    pub(super) fn set_network_route(&self, route: ProxyPolicy) {
        self.transport.set_network_route(route);
    }

    /// Offline fixture seam. It does not enable desktop credential storage.
    #[cfg(test)]
    pub(super) fn with_transport(transport: Arc<dyn GacosResultTransport>) -> Self {
        Self::new(false, transport)
    }

    pub(super) fn insert(&self, raw: &str) -> Result<(String, String), GacosResultError> {
        let url = validate_link(raw)?;
        let link_sha256 = sha256_text(raw);
        let result_file_name = safe_result_file_name(&url);
        self.insert_validated(raw, &link_sha256)?;
        Ok((link_sha256, result_file_name))
    }

    pub(super) fn supply(&self, expected_sha256: &str, raw: &str) -> Result<(), GacosResultError> {
        if !is_lower_sha256(expected_sha256) {
            return Err(GacosResultError::LinkHashMismatch);
        }
        validate_link(raw)?;
        let actual = sha256_text(raw);
        if actual != expected_sha256 {
            return Err(GacosResultError::LinkHashMismatch);
        }
        self.insert_validated(raw, expected_sha256)
    }

    fn insert_validated(&self, raw: &str, link_sha256: &str) -> Result<(), GacosResultError> {
        let desktop_credentials = self
            .state
            .lock()
            .map_err(|_| GacosResultError::CredentialUnavailable)?
            .desktop_credentials;
        if desktop_credentials {
            let existing = insar_download::with_desktop_gacos_result_link(link_sha256, |value| {
                Zeroizing::new(value.to_owned())
            })
            .map_err(|_| GacosResultError::CredentialUnavailable)?;
            if let Some(existing) = existing {
                if existing.as_str() != raw {
                    return Err(GacosResultError::LinkConflict);
                }
            } else {
                insar_download::store_desktop_gacos_result_link(link_sha256, raw)
                    .map_err(|_| GacosResultError::CredentialUnavailable)?;
            }
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| GacosResultError::CredentialUnavailable)?;
        if let Some(existing) = state.links.get(link_sha256) {
            if existing.as_str() != raw {
                return Err(GacosResultError::LinkConflict);
            }
            return Ok(());
        }
        state
            .links
            .insert(link_sha256.to_owned(), Zeroizing::new(raw.to_owned()));
        Ok(())
    }

    pub(super) fn resolve(
        &self,
        expected_sha256: &str,
    ) -> Result<Zeroizing<String>, GacosResultError> {
        if !is_lower_sha256(expected_sha256) {
            return Err(GacosResultError::LinkHashMismatch);
        }
        if let Some(value) = self
            .state
            .lock()
            .map_err(|_| GacosResultError::CredentialUnavailable)?
            .links
            .get(expected_sha256)
            .cloned()
        {
            return Ok(value);
        }
        let desktop_credentials = self
            .state
            .lock()
            .map_err(|_| GacosResultError::CredentialUnavailable)?
            .desktop_credentials;
        if !desktop_credentials {
            return Err(GacosResultError::LinkRequired);
        }
        let stored = insar_download::with_desktop_gacos_result_link(expected_sha256, |value| {
            Zeroizing::new(value.to_owned())
        })
        .map_err(|_| GacosResultError::CredentialUnavailable)?;
        let Some(stored) = stored else {
            return Err(GacosResultError::LinkRequired);
        };
        validate_link(stored.as_str())?;
        if sha256_text(stored.as_str()) != expected_sha256 {
            return Err(GacosResultError::LinkHashMismatch);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| GacosResultError::CredentialUnavailable)?;
        state
            .links
            .insert(expected_sha256.to_owned(), stored.clone());
        Ok(stored)
    }

    fn transport(&self) -> Arc<dyn GacosResultTransport> {
        Arc::clone(&self.transport)
    }
}

/// Immutable task-side identity. It contains no raw URL or mutable
/// submission/download state.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct FrozenGacosResult {
    pub(super) schema_version: u32,
    pub(super) source_request_task_id: String,
    pub(super) source_request_item_id: String,
    pub(super) bbox: [f64; 4],
    pub(super) dates: Vec<String>,
    pub(super) hour: u8,
    pub(super) minute: u8,
    pub(super) result_link_sha256: String,
    pub(super) result_file_name: String,
}

impl FrozenGacosResult {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.schema_version != GACOS_RESULT_SCHEMA_VERSION {
            return Err("unsupported GACOS result schema".to_owned());
        }
        validate_scalar(&self.source_request_task_id, "source request task ID")?;
        validate_scalar(&self.source_request_item_id, "source request item ID")?;
        GacosBoundingBox::new(self.bbox[0], self.bbox[1], self.bbox[2], self.bbox[3])
            .validate()
            .map_err(|_| "invalid GACOS result bounding box".to_owned())?;
        if self.dates.is_empty() || self.dates.len() > 20 {
            return Err("GACOS result date count is outside the 1..20 bound".to_owned());
        }
        let mut dates = BTreeSet::new();
        for value in &self.dates {
            let date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|_| "invalid GACOS result date".to_owned())?;
            if date.format("%Y-%m-%d").to_string() != *value || !dates.insert(date) {
                return Err("GACOS result dates must be unique canonical dates".to_owned());
            }
        }
        if self.hour > 23 || self.minute > 59 {
            return Err("invalid GACOS result UTC".to_owned());
        }
        if !is_lower_sha256(&self.result_link_sha256) {
            return Err("invalid GACOS result link digest".to_owned());
        }
        validate_result_file_name(&self.result_file_name)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GacosFetchReceipt {
    schema_version: u32,
    link_sha256: String,
    size_bytes: u64,
    etag: String,
    archive_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GacosPartialReceipt {
    schema_version: u32,
    link_sha256: String,
    size_bytes: u64,
    etag: String,
    prefix_len: u64,
    prefix_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct GacosResultFetchEvidence {
    pub(super) size_bytes: u64,
    pub(super) etag: String,
    pub(super) bytes_written: u64,
    pub(super) reused_existing: bool,
}

/// Private transport boundary used by offline AppService fixtures. The
/// production cache installs [`HttpTransferGacosResultTransport`].
pub(super) trait GacosResultTransport: Send + Sync {
    #[allow(clippy::too_many_arguments)] // Keep credentials separate from persisted transfer identity.
    fn fetch(
        &self,
        task_id: &str,
        item_id: &str,
        link_sha256: &str,
        raw_url: &str,
        destination: &Path,
        control: &WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> Result<GacosResultFetchEvidence, GacosResultError>;

    /// The saved network route for later fetches. Fixtures ignore it.
    fn set_network_route(&self, _route: ProxyPolicy) {}
}

/// Fetches on the saved ordinary route; until the runtime sets it, the
/// system proxy as before.
struct HttpTransferGacosResultTransport {
    route: Mutex<ProxyPolicy>,
}

impl HttpTransferGacosResultTransport {
    fn new() -> Self {
        Self {
            route: Mutex::new(ProxyPolicy::System),
        }
    }

    fn route(&self) -> ProxyPolicy {
        *self
            .route
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl GacosResultTransport for HttpTransferGacosResultTransport {
    fn set_network_route(&self, route: ProxyPolicy) {
        *self
            .route
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = route;
    }

    fn fetch(
        &self,
        task_id: &str,
        item_id: &str,
        link_sha256: &str,
        raw_url: &str,
        destination: &Path,
        control: &WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> Result<GacosResultFetchEvidence, GacosResultError> {
        if path_is_file(destination)? {
            let receipt = read_fetch_receipt(&fetch_receipt_path(destination));
            let reusable = match receipt {
                Ok(receipt)
                    if receipt.link_sha256 == link_sha256
                        && file_size(destination)? == receipt.size_bytes
                        && hash_file_exact(destination, receipt.size_bytes, control)?
                            == receipt.archive_sha256 =>
                {
                    Some(receipt)
                }
                Ok(_) | Err(_) => None,
            };
            if let Some(receipt) = reusable {
                return Ok(GacosResultFetchEvidence {
                    size_bytes: receipt.size_bytes,
                    etag: receipt.etag,
                    bytes_written: receipt.size_bytes,
                    reused_existing: true,
                });
            }
            // A final archive without a matching immutable receipt is not safe
            // to reuse. Preserve it under a deterministic, task-owned name so
            // a fresh bounded fetch can replace the canonical destination.
            quarantine_existing_archive(destination, control)?;
        }
        let route = self.route();
        let identity = preflight_result_url(raw_url, control, route)?;

        let asset = ResolvedAsset::new(raw_url, ResolvedMethod::Get)
            .map_err(|_| GacosResultError::InvalidLink)?
            .with_expected_size(Some(identity.size_bytes))
            .with_remote_validator(RemoteValidator {
                etag: Some(identity.etag.clone()),
                last_modified: None,
                content_length: Some(identity.size_bytes),
            });
        let request = TransferRequest::new(
            task_id,
            item_id,
            raw_url,
            destination.to_owned(),
            Some(identity.size_bytes),
        )
        .map_err(|_| GacosResultError::TransferUnavailable)?;
        let transfer = HttpTransfer::new_with_proxy_policy(
            TransferOptions {
                max_attempts: 1,
                backoff: Duration::ZERO,
                timeout: GACOS_TRANSFER_TOTAL_TIMEOUT,
                chunk_size: GACOS_FETCH_CHUNK_BYTES,
                ..TransferOptions::default()
            },
            route,
        )
        .map_err(|_| GacosResultError::TransferUnavailable)?;
        let mut session = transfer.resolved_session();
        let over_limit = Arc::new(AtomicBool::new(false));
        let mut emit = |update: TransferUpdate| match update {
            TransferUpdate::Progress(value) => {
                if value.bytes_completed > GACOS_MAX_ARTIFACT_BYTES {
                    over_limit.store(true, Ordering::SeqCst);
                }
                progress(DownloadItemProgress {
                    task_id: task_id.to_owned(),
                    item_id: item_id.to_owned(),
                    status: if control.is_paused() {
                        DownloadItemStatus::Paused
                    } else {
                        DownloadItemStatus::Active
                    },
                    bytes_completed: value.bytes_completed,
                    total_bytes: value.total_bytes,
                    bytes_per_second: 0.0,
                    attempt: u64::from(value.attempt),
                    updated_at: chrono::Utc::now()
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                });
            }
            TransferUpdate::BodyStarted { .. }
            | TransferUpdate::BodyStopped { .. }
            | TransferUpdate::StallReconnect { .. } => {}
        };
        let cancelled = || control.is_cancelled() || over_limit.load(Ordering::SeqCst);
        let paused = || control.is_paused();
        let checkpoint_validator = RemoteValidator {
            etag: Some(identity.etag.clone()),
            last_modified: None,
            content_length: Some(identity.size_bytes),
        };
        if let Some(checkpoint) = read_partial_receipt(&partial_receipt_path(destination)) {
            if checkpoint.link_sha256 == link_sha256
                && checkpoint.size_bytes == identity.size_bytes
                && checkpoint.etag == identity.etag
            {
                if let Err(err) = session.resume_verified_part(
                    &request,
                    &asset,
                    &checkpoint_validator,
                    checkpoint.prefix_len,
                    &checkpoint.prefix_sha256,
                    &cancelled,
                ) {
                    if control.is_cancelled() {
                        return Err(GacosResultError::Cancelled);
                    }
                    eprintln!(
                        "[gacos_result_worker] resume_verified_part fell back to full transfer: code={}",
                        err.code()
                    );
                }
            }
        }
        let result = transfer.download_resolved(
            &request,
            &asset,
            // Signed URL semantics provide the existing no-redirect branch;
            // no bearer refresh is permitted for a pasted GACOS link.
            AuthRefreshPolicyId::UsgsM2mSignedUrlV1,
            &mut session,
            &cancelled,
            &paused,
            &mut emit,
        );
        if over_limit.load(Ordering::SeqCst) {
            return Err(GacosResultError::TransferTooLarge);
        }
        let observed = session.prior_response_validator().cloned();
        match result {
            ResolvedTransferResult::AuthRejected(rejection) => {
                return Err(GacosResultError::AuthorizationRejected(
                    rejection.status_code,
                ));
            }
            ResolvedTransferResult::Finished(result) => match result.outcome {
                DownloadOutcome::Completed | DownloadOutcome::Skipped => {
                    let evidence = observed
                        .as_ref()
                        .ok_or(GacosResultError::TransferEvidenceMissing)?;
                    validate_transfer_identity(evidence, &identity)?;
                }
                DownloadOutcome::Interrupted => {
                    if let Some(evidence) = observed.as_ref() {
                        if validate_transfer_identity(evidence, &identity).is_ok() {
                            write_partial_checkpoint(
                                destination,
                                link_sha256,
                                &identity,
                                evidence,
                                control,
                            );
                        } else {
                            return Err(GacosResultError::TransferIdentityChanged);
                        }
                    }
                    if control.is_cancelled() {
                        return Err(GacosResultError::Cancelled);
                    }
                    return Err(GacosResultError::TransferFailed);
                }
                DownloadOutcome::Cancelled => {
                    if control.is_cancelled() {
                        return Err(GacosResultError::Cancelled);
                    }
                    return Err(GacosResultError::TransferFailed);
                }
                DownloadOutcome::Failed => {
                    let error = classify_transfer_failure(result.failure);
                    if error != GacosResultError::TransferIdentityChanged {
                        if let Some(evidence) = observed.as_ref() {
                            if validate_transfer_identity(evidence, &identity).is_ok() {
                                write_partial_checkpoint(
                                    destination,
                                    link_sha256,
                                    &identity,
                                    evidence,
                                    control,
                                );
                            }
                        }
                    }
                    return Err(error);
                }
            },
        }
        if file_size(destination)? != identity.size_bytes {
            return Err(GacosResultError::TransferIdentityChanged);
        }
        let archive_sha256 = hash_file_exact(destination, identity.size_bytes, control)?;
        write_fetch_receipt(
            &fetch_receipt_path(destination),
            &GacosFetchReceipt {
                schema_version: GACOS_FETCH_RECEIPT_SCHEMA_VERSION,
                link_sha256: link_sha256.to_owned(),
                size_bytes: identity.size_bytes,
                etag: identity.etag.clone(),
                archive_sha256: archive_sha256.clone(),
            },
        )?;
        Ok(GacosResultFetchEvidence {
            size_bytes: identity.size_bytes,
            etag: identity.etag,
            bytes_written: identity.size_bytes,
            reused_existing: false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RemoteIdentity {
    size_bytes: u64,
    etag: String,
}

fn preflight_result_url(
    raw_url: &str,
    control: &WorkerControl,
    route: ProxyPolicy,
) -> Result<RemoteIdentity, GacosResultError> {
    if control.is_cancelled() {
        return Err(GacosResultError::Cancelled);
    }
    let builder = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(GACOS_FETCH_CONNECT_TIMEOUT)
        .timeout(GACOS_FETCH_TOTAL_TIMEOUT);
    let client = insar_provider::network_proxy::blocking_builder_with_proxy(builder, route)
        .build()
        .map_err(|_| GacosResultError::PreflightTransport)?;
    let response = client
        .get(raw_url)
        .header(RANGE, "bytes=0-0")
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .map_err(|_| GacosResultError::PreflightTransport)?;
    if response.status().is_redirection() {
        return Err(GacosResultError::PreflightRedirect);
    }
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(GacosResultError::PreflightStatus(
            response.status().as_u16(),
        ));
    }
    let total = parse_exact_probe_content_range(response.headers().get(CONTENT_RANGE))?;
    let content_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(GacosResultError::PreflightSizeMissing)?;
    if content_length != 1 {
        return Err(GacosResultError::PreflightRangeInvalid);
    }
    if total == 0 {
        return Err(GacosResultError::PreflightSizeMissing);
    }
    if total > GACOS_MAX_ARTIFACT_BYTES {
        return Err(GacosResultError::PreflightSizeTooLarge);
    }
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| is_strong_etag(value))
        .map(str::to_owned)
        .ok_or(GacosResultError::PreflightValidatorMissing)?;
    let mut body = Vec::with_capacity(1);
    response
        .take((GACOS_MAX_RESPONSE_BYTES as u64).saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|_| GacosResultError::PreflightTransport)?;
    if body.len() > GACOS_MAX_RESPONSE_BYTES {
        return Err(GacosResultError::PreflightResponseTooLarge);
    }
    if body.len() != 1 {
        return Err(GacosResultError::PreflightBodyInvalid);
    }
    Ok(RemoteIdentity {
        size_bytes: total,
        etag,
    })
}

fn validate_transfer_identity(
    actual: &RemoteValidator,
    expected: &RemoteIdentity,
) -> Result<(), GacosResultError> {
    if actual.content_length != Some(expected.size_bytes)
        || actual.etag.as_deref() != Some(expected.etag.as_str())
    {
        return Err(GacosResultError::TransferIdentityChanged);
    }
    Ok(())
}

fn classify_transfer_failure(failure: Option<DownloadFailure>) -> GacosResultError {
    match failure.as_ref().map(|value| value.code.as_str()) {
        Some("DL002") => GacosResultError::TransferIdentityChanged,
        Some("DL003") => GacosResultError::TransferUnavailable,
        _ => GacosResultError::TransferFailed,
    }
}

fn parse_exact_probe_content_range(
    value: Option<&reqwest::header::HeaderValue>,
) -> Result<u64, GacosResultError> {
    let value = value
        .and_then(|value| value.to_str().ok())
        .ok_or(GacosResultError::PreflightRangeInvalid)?;
    let value = value.trim();
    let value = value
        .strip_prefix("bytes ")
        .ok_or(GacosResultError::PreflightRangeInvalid)?;
    let (range, total) = value
        .split_once('/')
        .ok_or(GacosResultError::PreflightRangeInvalid)?;
    if range != "0-0" || total.is_empty() || !total.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GacosResultError::PreflightRangeInvalid);
    }
    total
        .parse::<u64>()
        .map_err(|_| GacosResultError::PreflightRangeInvalid)
}

fn is_strong_etag(value: &str) -> bool {
    let value = value.trim();
    value.len() >= 2
        && value.len() <= 1024
        && !value
            .get(..2)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("w/"))
        && value.starts_with('"')
        && value.ends_with('"')
        && value.as_bytes()[1..value.len() - 1]
            .iter()
            .all(|byte| *byte >= 0x21 && *byte != b'"' && *byte != 0x7f)
}

fn validate_link(raw: &str) -> Result<Url, GacosResultError> {
    if raw.is_empty()
        || raw.len() > GACOS_MAX_RESULT_URL_BYTES
        || raw.trim() != raw
        || raw.chars().any(char::is_control)
    {
        return Err(GacosResultError::InvalidLink);
    }
    let link: GacosManualResultLink = validate_manual_result_url(raw).map_err(|error| {
        if matches!(
            error,
            insar_provider::gacos_delivery::GacosDeliveryError::UnreviewedResultHost
        ) {
            GacosResultError::UnreviewedLinkHost
        } else {
            GacosResultError::InvalidLink
        }
    })?;
    let url = link.url().clone();
    if let Some(query) = url.query() {
        if query.len() > GACOS_MAX_RESULT_URL_BYTES {
            return Err(GacosResultError::InvalidLink);
        }
        let mut keys = BTreeSet::new();
        for part in query.split('&') {
            let key = part.split_once('=').map_or(part, |(key, _)| key);
            if key.is_empty() || !keys.insert(key) {
                return Err(GacosResultError::InvalidLink);
            }
        }
    }
    Ok(url)
}

fn safe_result_file_name(url: &Url) -> String {
    let lower_path = url.path().to_ascii_lowercase();
    if lower_path.ends_with(".tar.gz") {
        return "gacos-result.tar.gz".to_owned();
    }
    if lower_path.ends_with(".tar.bz2") {
        return "gacos-result.tar.bz2".to_owned();
    }
    if lower_path.ends_with(".tar") {
        return "gacos-result.tar".to_owned();
    }
    if lower_path.ends_with(".zip") {
        return "gacos-result.zip".to_owned();
    }
    let segment = url
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .unwrap_or_default();
    if is_date_tiff_name(segment) {
        return segment.to_owned();
    }
    "gacos-result.archive".to_owned()
}

fn is_date_tiff_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    (bytes.len() == 12 || bytes.len() == 16)
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && value
            .get(8..)
            .is_some_and(|suffix| suffix == ".tif" || suffix == ".ztd.tif")
}

fn validate_result_file_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > GACOS_RESULT_MAX_FILE_NAME_BYTES
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value.contains("..")
        || value.chars().any(char::is_control)
    {
        return Err("invalid GACOS result filename".to_owned());
    }
    if value == "gacos-result.archive"
        || value == "gacos-result.zip"
        || value == "gacos-result.tar"
        || value == "gacos-result.tar.gz"
        || value == "gacos-result.tar.bz2"
        || is_date_tiff_name(value)
    {
        Ok(())
    } else {
        Err("unsupported GACOS result filename".to_owned())
    }
}

fn validate_scalar(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > GACOS_RESULT_MAX_ID_BYTES
        || value.chars().any(char::is_control)
    {
        Err(format!("{label} is invalid"))
    } else {
        Ok(())
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn path_is_file(path: &Path) -> Result<bool, GacosResultError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_file() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(GacosResultError::TransferUnavailable),
    }
}

fn file_size(path: &Path) -> Result<u64, GacosResultError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| GacosResultError::TransferUnavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(GacosResultError::TransferUnavailable);
    }
    Ok(metadata.len())
}

fn partial_receipt_path(archive: &Path) -> PathBuf {
    archive.with_extension("resume.json")
}

fn fetch_receipt_path(archive: &Path) -> PathBuf {
    archive.with_extension("fetch.json")
}

fn read_fetch_receipt(path: &Path) -> Result<GacosFetchReceipt, GacosResultError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| GacosResultError::TransferEvidenceMissing)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > GACOS_FETCH_RECEIPT_MAX_BYTES
    {
        return Err(GacosResultError::TransferEvidenceMissing);
    }
    let file = File::open(path).map_err(|_| GacosResultError::TransferEvidenceMissing)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(GACOS_FETCH_RECEIPT_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| GacosResultError::TransferEvidenceMissing)?;
    if bytes.len() as u64 > GACOS_FETCH_RECEIPT_MAX_BYTES {
        return Err(GacosResultError::TransferEvidenceMissing);
    }
    let receipt: GacosFetchReceipt =
        serde_json::from_slice(&bytes).map_err(|_| GacosResultError::TransferEvidenceMissing)?;
    if serde_json::to_vec(&receipt).map_err(|_| GacosResultError::TransferEvidenceMissing)? != bytes
        || receipt.schema_version != GACOS_FETCH_RECEIPT_SCHEMA_VERSION
        || !is_lower_sha256(&receipt.link_sha256)
        || receipt.size_bytes == 0
        || receipt.size_bytes > GACOS_MAX_ARTIFACT_BYTES
        || !is_strong_etag(&receipt.etag)
        || !is_lower_sha256(&receipt.archive_sha256)
    {
        return Err(GacosResultError::TransferEvidenceMissing);
    }
    Ok(receipt)
}

fn write_fetch_receipt(path: &Path, receipt: &GacosFetchReceipt) -> Result<(), GacosResultError> {
    let bytes = serde_json::to_vec(receipt).map_err(|_| GacosResultError::TransferUnavailable)?;
    if bytes.len() as u64 > GACOS_FETCH_RECEIPT_MAX_BYTES {
        return Err(GacosResultError::TransferUnavailable);
    }
    if let Ok(existing) = read_fetch_receipt(path) {
        if existing == *receipt {
            return Ok(());
        }
        return Err(GacosResultError::ArchiveIdentityMismatch);
    }
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = read_fetch_receipt(path)?;
            if existing == *receipt {
                return Ok(());
            }
            return Err(GacosResultError::ArchiveIdentityMismatch);
        }
        Err(_) => return Err(GacosResultError::TransferUnavailable),
    };
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| GacosResultError::TransferUnavailable)
}

fn quarantine_existing_archive(
    archive: &Path,
    control: &WorkerControl,
) -> Result<PathBuf, GacosResultError> {
    if control.is_cancelled() {
        return Err(GacosResultError::Cancelled);
    }
    let metadata =
        fs::symlink_metadata(archive).map_err(|_| GacosResultError::TransferUnavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(GacosResultError::ArchiveIdentityMismatch);
    }
    let digest = quarantine_digest(archive, metadata.len(), control)?;
    let file_name = archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(GacosResultError::ArchiveIdentityMismatch)?;
    let quarantined = archive.with_file_name(format!("{file_name}.unverified-{digest}"));
    if fs::symlink_metadata(&quarantined).is_ok() {
        return Err(GacosResultError::ArchiveIdentityMismatch);
    }
    fs::rename(archive, &quarantined).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            GacosResultError::ArchiveIdentityMismatch
        } else {
            GacosResultError::TransferUnavailable
        }
    })?;

    let receipt = fetch_receipt_path(archive);
    match fs::symlink_metadata(&receipt) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(metadata) => {
            if !metadata.is_file() && !metadata.file_type().is_symlink() {
                return Err(GacosResultError::ArchiveIdentityMismatch);
            }
            let quarantined_receipt = fetch_receipt_path(&quarantined);
            if fs::symlink_metadata(&quarantined_receipt).is_ok() {
                return Err(GacosResultError::ArchiveIdentityMismatch);
            }
            fs::rename(receipt, quarantined_receipt).map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    GacosResultError::ArchiveIdentityMismatch
                } else {
                    GacosResultError::TransferUnavailable
                }
            })?;
        }
        Err(_) => return Err(GacosResultError::TransferUnavailable),
    }
    Ok(quarantined)
}

fn quarantine_digest(
    archive: &Path,
    size_bytes: u64,
    control: &WorkerControl,
) -> Result<String, GacosResultError> {
    if size_bytes <= GACOS_MAX_ARTIFACT_BYTES {
        return hash_file_exact(archive, size_bytes, control);
    }
    let metadata =
        fs::symlink_metadata(archive).map_err(|_| GacosResultError::TransferUnavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != size_bytes {
        return Err(GacosResultError::ArchiveIdentityMismatch);
    }
    let mut file = File::open(archive).map_err(|_| GacosResultError::TransferUnavailable)?;
    let mut digest = Sha256::new();
    digest.update(b"gacos-unverified-prefix-v1");
    digest.update([0]);
    digest.update(size_bytes.to_le_bytes());
    let mut prefix = vec![0_u8; GACOS_FETCH_CHUNK_BYTES];
    let read = file
        .read(&mut prefix)
        .map_err(|_| GacosResultError::TransferUnavailable)?;
    if control.is_cancelled() {
        return Err(GacosResultError::Cancelled);
    }
    digest.update(&prefix[..read]);
    let final_size = fs::symlink_metadata(archive)
        .map_err(|_| GacosResultError::TransferUnavailable)?
        .len();
    if final_size != size_bytes {
        return Err(GacosResultError::ArchiveIdentityMismatch);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Scheduler worker for the second, user-mediated GACOS result task.
///
/// The wrapper delegates every other operation to the normal provider worker.
/// A result task keeps the same scheduler, Store lease, pause/cancel control,
/// and publication boundary as the local importer; only the source acquisition
/// is different.
pub(super) struct NativeGacosResultWorker {
    pub(super) base: Arc<dyn DownloadWorker>,
    pub(super) port: Option<Arc<dyn DemOperationPort>>,
    pub(super) links: Arc<GacosResultLinkCache>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResultExecutionError {
    Cancelled { bytes: u64 },
    Failed { error: GacosResultError, bytes: u64 },
}

impl DownloadWorker for NativeGacosResultWorker {
    fn capability_runtime_observations(&self) -> Vec<(String, insar_core::RuntimeReadinessState)> {
        self.base.capability_runtime_observations()
    }

    fn task_transfer_sample(&self, task_id: &str) -> Option<TaskTransferSample> {
        self.base.task_transfer_sample(task_id)
    }

    fn execute_with_authority(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        plan_freeze: &NisarExecutionPlanFreezeAuthority,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if is_gacos_result_operation(work.task.operation.as_ref()) {
            self.execute(work, control, progress)
        } else {
            self.base
                .execute_with_authority(work, control, plan_freeze, progress)
        }
    }

    fn execute(
        &self,
        work: DownloadWork,
        control: WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
    ) -> DownloadItemResult {
        if !is_gacos_result_operation(work.task.operation.as_ref()) {
            return self.base.execute(work, control, progress);
        }

        let task_id = work.task.task_id.clone();
        let item_id = work.item.item_id.clone();
        let destination = match planned_result_destination(&work.task) {
            Ok(value) => value,
            Err(error) => return result_failed(task_id, item_id, PathBuf::new(), error, 0),
        };
        if control.is_cancelled() {
            return cancelled_result(task_id, item_id, destination, 0);
        }
        match self.execute_result(&work, &control, progress, destination.clone()) {
            Ok(result) => {
                let bytes = result.bytes;
                emit_progress(
                    progress,
                    &work.task.task_id,
                    &work.item.item_id,
                    DownloadItemStatus::Completed,
                    bytes,
                    Some(bytes),
                );
                DownloadItemResult {
                    task_id,
                    item_id,
                    outcome: result.outcome,
                    destination,
                    bytes_written: bytes,
                    attempt_count: 1,
                    failure: None,
                    finished_at: now(),
                }
            }
            Err(ResultExecutionError::Cancelled { bytes }) => {
                cancelled_result(task_id, item_id, destination, bytes)
            }
            Err(ResultExecutionError::Failed { error, bytes }) => {
                result_failed(task_id, item_id, destination, error, bytes)
            }
        }
    }
}

impl NativeGacosResultWorker {
    fn execute_result(
        &self,
        work: &DownloadWork,
        control: &WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
        destination: PathBuf,
    ) -> Result<ImportResult, ResultExecutionError> {
        let (manifest, manifest_sha256) = parse_result_manifest(work)
            .map_err(|error| ResultExecutionError::Failed { error, bytes: 0 })?;
        ensure_output_root(&work.task.output_root).map_err(|failure| {
            ResultExecutionError::Failed {
                error: map_import_failure(failure),
                bytes: 0,
            }
        })?;
        let key = task_key(&work.task.task_id);
        let _lock = acquire_import_lock(&work.task.output_root, &key).map_err(|failure| {
            ResultExecutionError::Failed {
                error: map_import_failure(failure),
                bytes: 0,
            }
        })?;
        if control.is_cancelled() {
            return Err(ResultExecutionError::Cancelled { bytes: 0 });
        }

        let archive = work
            .task
            .output_root
            .join(format!(".gacos-result-{key}.archive"));
        let raw_url = self
            .links
            .resolve(&manifest.result_link_sha256)
            .map_err(|error| ResultExecutionError::Failed { error, bytes: 0 })?;
        let fetch = self.links.transport().fetch(
            &work.task.task_id,
            &work.item.item_id,
            &manifest.result_link_sha256,
            raw_url.as_str(),
            &archive,
            control,
            &mut |value| progress(value),
        );
        let fetch = match fetch {
            Ok(value) => value,
            Err(GacosResultError::Cancelled) => {
                return Err(ResultExecutionError::Cancelled {
                    bytes: partial_file_size(&archive),
                })
            }
            Err(error) => {
                return Err(ResultExecutionError::Failed {
                    error,
                    bytes: partial_file_size(&archive),
                })
            }
        };
        drop(raw_url);
        if fetch.size_bytes == 0 || fetch.size_bytes > GACOS_MAX_ARTIFACT_BYTES {
            return Err(ResultExecutionError::Failed {
                error: GacosResultError::TransferEvidenceMissing,
                bytes: fetch.bytes_written,
            });
        }
        if control.is_cancelled() {
            return Err(ResultExecutionError::Cancelled {
                bytes: fetch.bytes_written,
            });
        }

        let staging = staging_path(&work.task.output_root, &key);
        let materialized =
            materialize_result_products(&archive, &manifest.result_file_name, &staging, control)
                .map_err(|error| match error {
                    GacosResultError::Cancelled => ResultExecutionError::Cancelled {
                        bytes: fetch.bytes_written,
                    },
                    error => ResultExecutionError::Failed {
                        error,
                        bytes: fetch.bytes_written,
                    },
                })?;
        if materialized.is_empty() || materialized.len() > GACOS_MAX_FILES_PER_ADMISSION {
            return Err(ResultExecutionError::Failed {
                error: GacosResultError::ArchiveTooManyFiles,
                bytes: fetch.bytes_written,
            });
        }
        let import_manifest = FrozenGacosImport {
            schema_version: GACOS_RESULT_SCHEMA_VERSION,
            source_request_task_id: manifest.source_request_task_id.clone(),
            source_request_item_id: manifest.source_request_item_id.clone(),
            bbox: manifest.bbox,
            dates: manifest.dates.clone(),
            hour: manifest.hour,
            minute: manifest.minute,
            files: materialized,
        };
        let published = publish_materialized_gacos_staging(
            &work.task.task_id,
            &work.item.item_id,
            &import_manifest,
            &manifest_sha256,
            &staging,
            &destination,
            self.port.as_deref(),
            control,
            fetch.bytes_written,
        )
        .map_err(|error| match error {
            ImportExecutionError::Cancelled { bytes } => ResultExecutionError::Cancelled { bytes },
            ImportExecutionError::Failed { failure, bytes } => ResultExecutionError::Failed {
                error: map_import_failure(failure),
                bytes,
            },
        })?;
        Ok(ImportResult {
            outcome: published.outcome,
            bytes: fetch.bytes_written.max(published.bytes),
        })
    }
}

fn is_gacos_result_operation(operation: Option<&TaskOperationDescriptor>) -> bool {
    operation.is_some_and(|operation| {
        operation.kind == "gacos"
            && operation.phase == "execute"
            && operation.approved_worker_operation == GACOS_RESULT_WORKER
    })
}

fn parse_result_manifest(
    work: &DownloadWork,
) -> Result<(FrozenGacosResult, String), GacosResultError> {
    let operation = work
        .task
        .operation
        .as_ref()
        .ok_or(GacosResultError::InvalidOperation)?;
    insar_core::validate_gacos_result_operation(operation)
        .map_err(|_| GacosResultError::InvalidOperation)?;
    if work.task.provider_id != GACOS_RESULT_PROVIDER
        || work.item.provider_id != GACOS_RESULT_PROVIDER
        || work.task.initial_selected_item_ids.len() != 1
        || work.task.initial_selected_item_ids.first() != Some(&work.item.item_id)
    {
        return Err(GacosResultError::InvalidProvider);
    }
    let manifest_value = work
        .item
        .payload
        .get(GACOS_RESULT_MANIFEST_KEY)
        .cloned()
        .ok_or(GacosResultError::InvalidManifest)?;
    let manifest: FrozenGacosResult = serde_json::from_value(manifest_value.clone())
        .map_err(|_| GacosResultError::InvalidManifest)?;
    manifest
        .validate()
        .map_err(|_| GacosResultError::InvalidManifest)?;
    let canonical =
        serde_json::to_value(&manifest).map_err(|_| GacosResultError::InvalidManifest)?;
    if canonical != manifest_value {
        return Err(GacosResultError::InvalidManifest);
    }
    let expected = operation
        .payload
        .get("manifest_sha256")
        .and_then(Value::as_str)
        .filter(|value| is_lower_sha256(value))
        .ok_or(GacosResultError::InvalidManifest)?;
    if sha256_value(&manifest_value) != expected {
        return Err(GacosResultError::InvalidManifest);
    }
    for (key, expected_value) in [
        (
            "source_request_task_id",
            manifest.source_request_task_id.as_str(),
        ),
        (
            "source_request_item_id",
            manifest.source_request_item_id.as_str(),
        ),
    ] {
        if operation.payload.get(key).and_then(Value::as_str) != Some(expected_value) {
            return Err(GacosResultError::InvalidManifest);
        }
    }
    if manifest.source_request_task_id == work.task.task_id
        || manifest.source_request_item_id == work.item.item_id
    {
        return Err(GacosResultError::InvalidManifest);
    }
    Ok((manifest, expected.to_owned()))
}

fn planned_result_destination(task: &FrozenTaskSnapshot) -> Result<PathBuf, GacosResultError> {
    if !task.output_root.is_absolute() || task.output_root.as_os_str().is_empty() {
        return Err(GacosResultError::InvalidManifest);
    }
    Ok(task.output_root.join(task_key(&task.task_id)))
}

fn materialize_result_products(
    archive: &Path,
    original_name: &str,
    staging: &Path,
    control: &WorkerControl,
) -> Result<Vec<FrozenGacosFile>, GacosResultError> {
    // Archive extraction owns its temporary files in a sibling directory;
    // importer staging contains only admitted products and its immutable receipt.
    if let Ok(metadata) = fs::symlink_metadata(staging) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(GacosResultError::ArchiveIdentityMismatch);
        }
    }
    let materialized = staging.with_extension("materialized");
    let files = super::gacos_result_archive::materialize_result(
        archive,
        original_name,
        &materialized,
        control,
    )
    .map_err(|error| {
        if control.is_cancelled() {
            GacosResultError::Cancelled
        } else {
            GacosResultError::Archive(error)
        }
    })?;
    let adopted = adopt_materialized_products(&files, &materialized, staging, control)?;
    let _ = fs::remove_dir(&materialized);
    Ok(adopted)
}

fn adopt_materialized_products(
    files: &[FrozenGacosFile],
    materialized: &Path,
    staging: &Path,
    control: &WorkerControl,
) -> Result<Vec<FrozenGacosFile>, GacosResultError> {
    let materialized_root = materialized
        .canonicalize()
        .map_err(|_| GacosResultError::ArchiveMaterialization)?;
    let staging_root = if staging.exists() {
        staging
            .canonicalize()
            .map_err(|_| GacosResultError::ArchiveMaterialization)?
    } else {
        fs::create_dir_all(staging).map_err(|_| GacosResultError::ArchiveMaterialization)?;
        staging
            .canonicalize()
            .map_err(|_| GacosResultError::ArchiveMaterialization)?
    };
    let mut adopted = Vec::with_capacity(files.len());
    for file in files {
        if control.is_cancelled() {
            return Err(GacosResultError::Cancelled);
        }
        if file
            .path
            .parent()
            .and_then(|path| path.canonicalize().ok())
            .as_deref()
            != Some(materialized_root.as_path())
            || file.path.file_name().and_then(|name| name.to_str()) != Some(file.name.as_str())
        {
            return Err(GacosResultError::ArchiveIdentityMismatch);
        }
        let source = materialized_root.join(&file.name);
        let destination = staging_root.join(&file.name);
        let source_metadata =
            fs::symlink_metadata(&source).map_err(|_| GacosResultError::ArchiveMaterialization)?;
        if source_metadata.file_type().is_symlink()
            || !source_metadata.is_file()
            || source_metadata.len() != file.size_bytes
        {
            return Err(GacosResultError::ArchiveIdentityMismatch);
        }
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || !metadata.is_file()
                    || metadata.len() != file.size_bytes
                    || hash_file_exact(&destination, file.size_bytes, control)? != file.sha256
                {
                    return Err(GacosResultError::ArchiveIdentityMismatch);
                }
                fs::remove_file(&source).map_err(|_| GacosResultError::ArchiveMaterialization)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::rename(&source, &destination)
                    .map_err(|_| GacosResultError::ArchiveMaterialization)?;
            }
            Err(_) => return Err(GacosResultError::ArchiveMaterialization),
        }
        adopted.push(FrozenGacosFile {
            path: destination,
            name: file.name.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256.clone(),
        });
    }
    Ok(adopted)
}

fn hash_file_exact(
    path: &Path,
    expected_size: u64,
    control: &WorkerControl,
) -> Result<String, GacosResultError> {
    if expected_size == 0 || expected_size > GACOS_MAX_ARTIFACT_BYTES {
        return Err(GacosResultError::ArchiveIdentityMismatch);
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| GacosResultError::ArchiveMaterialization)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != expected_size {
        return Err(GacosResultError::ArchiveIdentityMismatch);
    }
    let mut file = File::open(path).map_err(|_| GacosResultError::ArchiveMaterialization)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; GACOS_FETCH_CHUNK_BYTES];
    let mut read_total = 0_u64;
    loop {
        if control.is_cancelled() {
            return Err(GacosResultError::Cancelled);
        }
        let read = file
            .read(&mut buffer)
            .map_err(|_| GacosResultError::ArchiveMaterialization)?;
        if read == 0 {
            break;
        }
        read_total = read_total
            .checked_add(read as u64)
            .ok_or(GacosResultError::ArchiveIdentityMismatch)?;
        if read_total > expected_size {
            return Err(GacosResultError::ArchiveIdentityMismatch);
        }
        digest.update(&buffer[..read]);
    }
    if read_total != expected_size {
        return Err(GacosResultError::ArchiveIdentityMismatch);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn map_import_failure(failure: ImportFailure) -> GacosResultError {
    match failure {
        ImportFailure::InvalidManifest => GacosResultError::InvalidManifest,
        ImportFailure::RasterInspectionUnavailable => GacosResultError::RasterInspectionUnavailable,
        ImportFailure::RasterInspectionInvalid => GacosResultError::RasterInspectionInvalid,
        ImportFailure::Busy | ImportFailure::Filesystem | ImportFailure::SourceUnavailable => {
            GacosResultError::TransferUnavailable
        }
        _ => GacosResultError::ImportFailed,
    }
}

fn sha256_value(value: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
    )
}

fn partial_file_size(archive: &Path) -> u64 {
    let part = part_path(archive);
    fs::symlink_metadata(part)
        .ok()
        .filter(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
        .map(|metadata| metadata.len().min(GACOS_MAX_ARTIFACT_BYTES))
        .unwrap_or_default()
}

fn result_failed(
    task_id: String,
    item_id: String,
    destination: PathBuf,
    error: GacosResultError,
    bytes: u64,
) -> DownloadItemResult {
    DownloadItemResult {
        task_id,
        item_id,
        outcome: DownloadOutcome::Failed,
        destination,
        bytes_written: bytes,
        attempt_count: 1,
        failure: Some(DownloadFailure {
            code: error.code().to_owned(),
            reason_zh: error.reason_zh().to_owned(),
            retryable: error.retryable(),
        }),
        finished_at: now(),
    }
}

fn read_partial_receipt(path: &Path) -> Option<GacosPartialReceipt> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > GACOS_PARTIAL_RECEIPT_MAX_BYTES
        || metadata.len() > usize::MAX as u64
    {
        return None;
    }
    let file = File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(GACOS_PARTIAL_RECEIPT_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > GACOS_PARTIAL_RECEIPT_MAX_BYTES {
        return None;
    }
    let receipt: GacosPartialReceipt = serde_json::from_slice(&bytes).ok()?;
    if serde_json::to_vec(&receipt).ok()? != bytes
        || receipt.schema_version != GACOS_PARTIAL_RECEIPT_SCHEMA_VERSION
        || !is_lower_sha256(&receipt.link_sha256)
        || receipt.size_bytes == 0
        || receipt.size_bytes > GACOS_MAX_ARTIFACT_BYTES
        || receipt.prefix_len == 0
        || receipt.prefix_len >= receipt.size_bytes
        || !is_strong_etag(&receipt.etag)
        || !is_lower_sha256(&receipt.prefix_sha256)
    {
        return None;
    }
    Some(receipt)
}

fn write_partial_checkpoint(
    archive: &Path,
    link_sha256: &str,
    identity: &RemoteIdentity,
    observed: &RemoteValidator,
    control: &WorkerControl,
) {
    if validate_transfer_identity(observed, identity).is_err() {
        return;
    }
    let prefix_len = partial_file_size(archive);
    if prefix_len == 0 || prefix_len >= identity.size_bytes {
        return;
    }
    let partial = part_path(archive);
    let Ok(prefix_sha256) = hash_file_prefix(&partial, prefix_len, control) else {
        return;
    };
    let receipt = GacosPartialReceipt {
        schema_version: GACOS_PARTIAL_RECEIPT_SCHEMA_VERSION,
        link_sha256: link_sha256.to_owned(),
        size_bytes: identity.size_bytes,
        etag: identity.etag.clone(),
        prefix_len,
        prefix_sha256,
    };
    let _ = write_partial_receipt(&partial_receipt_path(archive), &receipt);
}

fn hash_file_prefix(
    path: &Path,
    expected_size: u64,
    control: &WorkerControl,
) -> Result<String, GacosResultError> {
    if expected_size == 0 || expected_size > GACOS_MAX_ARTIFACT_BYTES {
        return Err(GacosResultError::TransferEvidenceMissing);
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| GacosResultError::TransferEvidenceMissing)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != expected_size {
        return Err(GacosResultError::TransferEvidenceMissing);
    }
    let mut file = File::open(path).map_err(|_| GacosResultError::TransferEvidenceMissing)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; GACOS_FETCH_CHUNK_BYTES];
    let mut read_total = 0_u64;
    while read_total < expected_size {
        if control.is_cancelled() {
            return Err(GacosResultError::Cancelled);
        }
        let wanted = (expected_size - read_total).min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..wanted])
            .map_err(|_| GacosResultError::TransferEvidenceMissing)?;
        if read == 0 {
            return Err(GacosResultError::TransferEvidenceMissing);
        }
        read_total = read_total
            .checked_add(read as u64)
            .ok_or(GacosResultError::TransferEvidenceMissing)?;
        digest.update(&buffer[..read]);
    }
    if fs::symlink_metadata(path)
        .map_err(|_| GacosResultError::TransferEvidenceMissing)?
        .len()
        != expected_size
    {
        return Err(GacosResultError::TransferEvidenceMissing);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn write_partial_receipt(
    path: &Path,
    receipt: &GacosPartialReceipt,
) -> Result<(), GacosResultError> {
    let bytes = serde_json::to_vec(receipt).map_err(|_| GacosResultError::TransferUnavailable)?;
    if bytes.len() as u64 > GACOS_PARTIAL_RECEIPT_MAX_BYTES {
        return Err(GacosResultError::TransferUnavailable);
    }
    if let Some(existing) = read_partial_receipt(path) {
        if existing == *receipt {
            return Ok(());
        }
        if existing.link_sha256 != receipt.link_sha256
            || existing.size_bytes != receipt.size_bytes
            || existing.etag != receipt.etag
        {
            return Ok(());
        }
    } else if fs::symlink_metadata(path).is_ok() {
        // Preserve an invalid or foreign checkpoint. The next execution will
        // conservatively restart instead of trusting a partially read sidecar.
        return Ok(());
    }

    let temporary = path.with_extension("resume.json.tmp");
    let write_outcome = {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| GacosResultError::TransferUnavailable)?;
        file.write_all(&bytes).and_then(|_| file.sync_all())
    };
    if write_outcome.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(GacosResultError::TransferUnavailable);
    }
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let same_identity = read_partial_receipt(path).is_some_and(|existing| {
                existing.link_sha256 == receipt.link_sha256
                    && existing.size_bytes == receipt.size_bytes
                    && existing.etag == receipt.etag
            });
            if same_identity {
                let _ = fs::remove_file(path);
                if fs::rename(&temporary, path).is_err() {
                    let _ = fs::remove_file(&temporary);
                    return Err(GacosResultError::TransferUnavailable);
                }
                Ok(())
            } else {
                let _ = fs::remove_file(&temporary);
                Ok(())
            }
        }
        Err(_) => {
            let _ = fs::remove_file(&temporary);
            Err(GacosResultError::TransferUnavailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    struct FixtureTransport;

    impl GacosResultTransport for FixtureTransport {
        fn fetch(
            &self,
            _task_id: &str,
            _item_id: &str,
            _link_sha256: &str,
            _raw_url: &str,
            _destination: &Path,
            _control: &WorkerControl,
            _progress: &mut dyn FnMut(DownloadItemProgress),
        ) -> Result<GacosResultFetchEvidence, GacosResultError> {
            Err(GacosResultError::TransferUnavailable)
        }
    }

    #[test]
    fn fixture_cache_keeps_url_runtime_only_and_uses_transport_seam() {
        let raw = "https://www.gacos.net/results/private.tar.gz?download_token=opaque";
        let cache = GacosResultLinkCache::with_transport(Arc::new(FixtureTransport));
        let (sha, file_name) = cache.insert(raw).expect("validated fixture link");
        assert_eq!(file_name, "gacos-result.tar.gz");
        assert_eq!(sha, sha256_text(raw));
        assert_eq!(cache.resolve(&sha).expect("cached link").as_str(), raw);
        let debug = format!("{cache:?}");
        assert!(!debug.contains("opaque"));
        assert!(!debug.contains("download_token"));
        assert!(!debug.contains("www.gacos.net"));
    }

    #[test]
    fn cache_rejects_unreviewed_hosts_duplicate_queries_and_hash_mismatch() {
        let cache = GacosResultLinkCache::with_transport(Arc::new(FixtureTransport));
        assert_eq!(
            cache.insert("https://example.invalid/result.zip"),
            Err(GacosResultError::UnreviewedLinkHost)
        );
        assert_eq!(
            cache.insert("https://www.gacos.net/result.zip?a=1&a=2"),
            Err(GacosResultError::InvalidLink)
        );
        assert_eq!(
            cache.supply(&"a".repeat(64), "https://www.gacos.net/result.zip"),
            Err(GacosResultError::LinkHashMismatch)
        );
    }

    #[test]
    fn safe_result_name_never_copies_archive_basename_or_query() {
        let url = validate_link(
            "https://gacos.net/opaque/path/with-a-secret-name.tar.bz2?download_token=opaque",
        )
        .expect("validated URL");
        assert_eq!(safe_result_file_name(&url), "gacos-result.tar.bz2");
        let url = validate_link("https://gacos.net/a/private-name?token=opaque").expect("URL");
        assert_eq!(safe_result_file_name(&url), "gacos-result.archive");
    }

    fn valid_manifest() -> FrozenGacosResult {
        FrozenGacosResult {
            schema_version: GACOS_RESULT_SCHEMA_VERSION,
            source_request_task_id: "request-task".to_owned(),
            source_request_item_id: "request-item".to_owned(),
            bbox: [70.0, 10.0, 70.1, 10.1],
            dates: vec!["2024-03-12".to_owned()],
            hour: 10,
            minute: 26,
            result_link_sha256: "a".repeat(64),
            result_file_name: "gacos-result.archive".to_owned(),
        }
    }

    #[test]
    fn frozen_result_rejects_duplicate_dates_and_path_traversal_name() {
        let mut duplicate_dates = valid_manifest();
        duplicate_dates.dates.push("2024-03-12".to_owned());
        assert!(duplicate_dates.validate().is_err());

        let mut traversal = valid_manifest();
        traversal.result_file_name = "../result.zip".to_owned();
        assert!(traversal.validate().is_err());
    }

    #[test]
    fn probe_requires_exact_single_byte_range_and_strong_etag() {
        let valid_range = HeaderValue::from_static("bytes 0-0/42");
        assert_eq!(
            parse_exact_probe_content_range(Some(&valid_range)).expect("range"),
            42
        );
        for value in ["bytes 0-1/42", "bytes 1-1/42", "bytes 0-0/*", "bytes 0-0/0"] {
            let header = HeaderValue::from_str(value).expect("header");
            assert_eq!(
                parse_exact_probe_content_range(Some(&header)),
                if value == "bytes 0-0/0" {
                    Ok(0)
                } else {
                    Err(GacosResultError::PreflightRangeInvalid)
                }
            );
        }
        assert!(is_strong_etag("\"opaque\""));
        assert!(!is_strong_etag("W/\"opaque\""));
        assert!(!is_strong_etag("opaque"));
        assert!(!is_strong_etag("\"opaque value\""));
        assert!(!is_strong_etag("\"opaque\\\"value\""));
    }

    #[test]
    fn quarantine_missing_receipt_preserves_archive_for_fresh_fetch() {
        let root = std::env::temp_dir().join(format!(
            "insar-gacos-quarantine-missing-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary test directory");
        let archive = root.join(".gacos-result-task.archive");
        let original = b"unverified archive bytes";
        fs::write(&archive, original).expect("archive");

        let quarantined =
            quarantine_existing_archive(&archive, &WorkerControl::default()).expect("quarantine");
        assert!(!archive.exists());
        assert_eq!(
            fs::read(&quarantined).expect("quarantined archive"),
            original
        );
        assert!(quarantined
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                value
                    .strip_prefix(".gacos-result-task.archive.unverified-")
                    .is_some_and(|digest| digest.len() == 64 && is_lower_sha256(digest))
            }));
        assert!(!fetch_receipt_path(&archive).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn quarantine_hash_mismatch_moves_receipt_and_never_overwrites() {
        let root = std::env::temp_dir().join(format!(
            "insar-gacos-quarantine-mismatch-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temporary test directory");
        let archive = root.join(".gacos-result-task.archive");
        let original = b"same length but different digest";
        fs::write(&archive, original).expect("archive");
        let receipt = GacosFetchReceipt {
            schema_version: GACOS_FETCH_RECEIPT_SCHEMA_VERSION,
            link_sha256: "a".repeat(64),
            size_bytes: original.len() as u64,
            etag: "\"fixture-v1\"".to_owned(),
            archive_sha256: "b".repeat(64),
        };
        fs::write(
            fetch_receipt_path(&archive),
            serde_json::to_vec(&receipt).expect("receipt JSON"),
        )
        .expect("receipt");
        assert_ne!(
            hash_file_exact(&archive, original.len() as u64, &WorkerControl::default())
                .expect("archive digest"),
            receipt.archive_sha256
        );

        let quarantined =
            quarantine_existing_archive(&archive, &WorkerControl::default()).expect("quarantine");
        assert_eq!(
            fs::read(&quarantined).expect("quarantined archive"),
            original
        );
        assert_eq!(
            fs::read(fetch_receipt_path(&quarantined)).expect("quarantined receipt"),
            serde_json::to_vec(&receipt).expect("receipt JSON")
        );

        let collision = quarantined;
        fs::write(&archive, original).expect("replacement archive");
        assert_eq!(
            quarantine_existing_archive(&archive, &WorkerControl::default()),
            Err(GacosResultError::ArchiveIdentityMismatch)
        );
        assert_eq!(fs::read(&collision).expect("original quarantine"), original);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_partial_receipt_closes_handle_before_rename_and_cleans_empty_materialized_dir() {
        use std::io::Write as _;
        let root = std::env::temp_dir().join(format!(
            "insar-gacos-receipt-cleanup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let receipt_path = root.join(".gacos-result.archive.resume.json");
        let receipt = GacosPartialReceipt {
            schema_version: GACOS_PARTIAL_RECEIPT_SCHEMA_VERSION,
            link_sha256: "a".repeat(64),
            size_bytes: 128,
            etag: "\"v1\"".to_owned(),
            prefix_len: 64,
            prefix_sha256: "b".repeat(64),
        };
        write_partial_receipt(&receipt_path, &receipt).expect("initial partial receipt write");
        assert!(receipt_path.exists());
        assert!(!receipt_path.with_extension("resume.json.tmp").exists());

        // Overwrite an existing receipt with same identity cleanly (no handle lock or leftover .tmp).
        let updated = GacosPartialReceipt {
            prefix_len: 96,
            prefix_sha256: "c".repeat(64),
            ..receipt
        };
        write_partial_receipt(&receipt_path, &updated).expect("replacement receipt write");
        assert!(!receipt_path.with_extension("resume.json.tmp").exists());
        assert_eq!(read_partial_receipt(&receipt_path), Some(updated));

        // Materialize valid GACOS product zip and confirm sibling `.materialized` dir is removed.
        let archive = root.join("sample.zip");
        {
            let file = File::create(&archive).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("20240101.ztd.tif", options).unwrap();
            writer.write_all(b"II*\0native-test-tiff").unwrap();
            writer.finish().unwrap();
        }
        let staging = root.join("staging");
        let adopted = materialize_result_products(
            &archive,
            "sample.zip",
            &staging,
            &WorkerControl::default(),
        )
        .expect("materialize zip");
        assert_eq!(adopted.len(), 1);
        assert!(
            !staging.with_extension("materialized").exists(),
            "empty .materialized directory must be removed after product adoption"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn result_fetches_use_the_route_the_runtime_sets() {
        let transport = Arc::new(HttpTransferGacosResultTransport::new());
        assert_eq!(transport.route(), ProxyPolicy::System);
        let links = GacosResultLinkCache::with_transport(transport.clone());
        links.set_network_route(ProxyPolicy::Direct);
        assert_eq!(transport.route(), ProxyPolicy::Direct);
    }
}
