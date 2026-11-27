//! Native, Store-owned execution of a manually materialized GACOS result.
//!
//! The request task and this result task are deliberately separate.  The
//! request task remains a paused, user-mediated form submission while this
//! worker handles only already materialized local files.  The scheduler and
//! TaskStore remain the sole owners of lifecycle, progress, retry, and history
//! state; the filesystem below is only an atomic artifact staging boundary.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chrono::{NaiveDate, SecondsFormat, Utc};
use insar_core::{
    DownloadFailure, DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadOutcome,
    FrozenTaskSnapshot, TaskOperationDescriptor,
};
use insar_download::{
    DownloadWork, DownloadWorker, NisarExecutionPlanFreezeAuthority, TaskTransferSample,
    WorkerControl,
};
use insar_provider::gacos_artifact::{
    admit_gacos_artifact_evidence, GacosArtifactError, GacosArtifactEvidence,
    GacosArtifactEvidenceRequest, GacosRasterInspectionFacts, GACOS_MAX_ARTIFACT_BYTES,
    GACOS_MAX_FILES_PER_ADMISSION, GACOS_MAX_RSC_HEADER_BYTES, GACOS_TIFF_PREFIX_BYTES,
};
use insar_provider::gacos_delivery::{GacosBoundingBox, GacosUtcTime};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::Map;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::DemOperationPort;

const GACOS_IMPORT_WORKER: &str = "gacos_import_result";
const GACOS_IMPORT_MANIFEST_KEY: &str = "gacos_import_manifest";
const GACOS_IMPORT_SCHEMA_VERSION: u32 = 1;
const GACOS_IMPORT_MAX_PATH_CHARS: usize = 32_767;
const GACOS_IMPORT_COPY_CHUNK_BYTES: usize = 1024 * 1024;
const GACOS_IMPORT_PAUSE_POLL: Duration = Duration::from_millis(25);
const GACOS_ROTATION_EPSILON: f64 = 1.0e-12;

/// Immutable source identity frozen by the ApplicationService before the
/// result task is created.  The path is an input reference only; no lifecycle
/// state is inferred from it and no separate import ledger is written.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct FrozenGacosImport {
    pub(super) schema_version: u32,
    pub(super) source_request_task_id: String,
    pub(super) source_request_item_id: String,
    pub(super) bbox: [f64; 4],
    pub(super) dates: Vec<String>,
    pub(super) hour: u8,
    pub(super) minute: u8,
    pub(super) files: Vec<FrozenGacosFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct FrozenGacosFile {
    pub(super) path: PathBuf,
    pub(super) name: String,
    pub(super) size_bytes: u64,
    pub(super) sha256: String,
}

const GACOS_IMPORT_RECEIPT_NAME: &str = "gacos-import-receipt.json";
const GACOS_IMPORT_RECEIPT_MAX_BYTES: u64 = 128 * 1024;

/// Immutable publication receipt.  It contains the task and frozen-manifest
/// identity plus exact artifact hashes, but never source paths or credentials.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GacosImportReceipt {
    schema_version: u32,
    task_id: String,
    item_id: String,
    manifest_sha256: String,
    source_request_task_id: String,
    source_request_item_id: String,
    requested_utc: GacosImportUtc,
    observed_utc: Option<GacosImportUtc>,
    files: Vec<GacosReceiptFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GacosImportUtc {
    hour: u8,
    minute: u8,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct GacosReceiptFile {
    name: String,
    size_bytes: u64,
    sha256: String,
}

impl GacosImportReceipt {
    fn for_task(
        task_id: &str,
        item_id: &str,
        manifest: &FrozenGacosImport,
        manifest_sha256: &str,
    ) -> Self {
        Self {
            schema_version: GACOS_IMPORT_SCHEMA_VERSION,
            task_id: task_id.to_owned(),
            item_id: item_id.to_owned(),
            manifest_sha256: manifest_sha256.to_owned(),
            source_request_task_id: manifest.source_request_task_id.clone(),
            source_request_item_id: manifest.source_request_item_id.clone(),
            requested_utc: GacosImportUtc {
                hour: manifest.hour,
                minute: manifest.minute,
            },
            observed_utc: None,
            files: manifest
                .files
                .iter()
                .map(|file| GacosReceiptFile {
                    name: file.name.clone(),
                    size_bytes: file.size_bytes,
                    sha256: file.sha256.clone(),
                })
                .collect(),
        }
    }
}

/// ApplicationService wraps the normal provider worker with this worker after
/// it has constructed the same Scheduler and Store lease.  Non-GACOS-import
/// operations retain their original worker and authority behavior.
pub(super) struct NativeGacosImportWorker {
    pub(super) base: Arc<dyn DownloadWorker>,
    pub(super) port: Option<Arc<dyn DemOperationPort>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImportFailure {
    InvalidManifest,
    SourceUnavailable,
    SourceChanged,
    DestinationConflict,
    StagingConflict,
    Filesystem,
    ArtifactInvalid,
    RasterInspectionUnavailable,
    RasterInspectionInvalid,
    Busy,
    Promotion,
}

impl ImportFailure {
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::InvalidManifest => "NATIVE_GACOS_MANIFEST_INVALID",
            Self::SourceUnavailable => "NATIVE_GACOS_SOURCE_UNAVAILABLE",
            Self::SourceChanged => "NATIVE_GACOS_SOURCE_CHANGED",
            Self::DestinationConflict => "NATIVE_GACOS_DESTINATION_CONFLICT",
            Self::StagingConflict => "NATIVE_GACOS_STAGING_CONFLICT",
            Self::Filesystem => "NATIVE_GACOS_FILESYSTEM",
            Self::ArtifactInvalid => "NATIVE_GACOS_ARTIFACT_INVALID",
            Self::RasterInspectionUnavailable => "NATIVE_GACOS_RASTER_INSPECTION_UNAVAILABLE",
            Self::RasterInspectionInvalid => "NATIVE_GACOS_RASTER_INSPECTION_INVALID",
            Self::Busy => "NATIVE_GACOS_BUSY",
            Self::Promotion => "NATIVE_GACOS_PROMOTION_FAILED",
        }
    }

    pub(super) const fn reason_zh(self) -> &'static str {
        match self {
            Self::InvalidManifest => "GACOS 导入任务清单无效。",
            Self::SourceUnavailable => "GACOS 本地结果文件不可读。",
            Self::SourceChanged => "GACOS 本地结果文件已改变，未发布。",
            Self::DestinationConflict => "GACOS 输出目录已有不匹配文件，未覆盖。",
            Self::StagingConflict => "GACOS 暂存目录含有不匹配文件，已保留现场。",
            Self::Filesystem => "GACOS 导入目录不可写或暂存状态不可用。",
            Self::ArtifactInvalid => "GACOS 结果未通过科学格式与空间范围校验。",
            Self::RasterInspectionUnavailable => "GACOS GeoTIFF 的 native GIS 检查不可用。",
            Self::RasterInspectionInvalid => "GACOS GeoTIFF 未通过 native GIS 检查。",
            Self::Busy => "GACOS 导入正在由另一个进程执行。",
            Self::Promotion => "GACOS 结果无法安全发布，已保留暂存文件。",
        }
    }

    pub(super) const fn retryable(self) -> bool {
        matches!(
            self,
            Self::SourceUnavailable
                | Self::SourceChanged
                | Self::Filesystem
                | Self::RasterInspectionUnavailable
                | Self::Busy
                | Self::Promotion
        )
    }
}

impl FrozenGacosImport {
    /// Validate the immutable manifest without opening any input file.
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.schema_version != GACOS_IMPORT_SCHEMA_VERSION {
            return Err("GACOS import schema version is unsupported".to_owned());
        }
        validate_scalar(&self.source_request_task_id, "source request task ID")?;
        validate_scalar(&self.source_request_item_id, "source request item ID")?;

        let bbox = GacosBoundingBox::new(self.bbox[0], self.bbox[1], self.bbox[2], self.bbox[3]);
        bbox.validate()
            .map_err(|_| "GACOS import bounding box is invalid".to_owned())?;
        if self.dates.is_empty() || self.dates.len() > 20 {
            return Err("GACOS import date count is outside the 1..20 bound".to_owned());
        }
        let mut dates = BTreeSet::new();
        for value in &self.dates {
            let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map_err(|_| "GACOS import date is invalid".to_owned())?;
            if parsed.format("%Y-%m-%d").to_string() != *value || !dates.insert(parsed) {
                return Err("GACOS import dates must be unique canonical UTC dates".to_owned());
            }
        }
        if self.hour > 23 || self.minute > 59 {
            return Err("GACOS import UTC time is invalid".to_owned());
        }
        if self.files.is_empty() || self.files.len() > GACOS_MAX_FILES_PER_ADMISSION {
            return Err("GACOS import file count is outside the bounded range".to_owned());
        }

        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for file in &self.files {
            validate_file_name(&file.name)?;
            let path_text = file.path.to_string_lossy();
            if !file.path.is_absolute() || path_text.chars().count() > GACOS_IMPORT_MAX_PATH_CHARS {
                return Err("GACOS import source path is invalid".to_owned());
            }
            if file.path.file_name().and_then(|value| value.to_str()) != Some(file.name.as_str()) {
                return Err("GACOS import source name does not match its path".to_owned());
            }
            if !names.insert(file.name.as_str()) {
                return Err("GACOS import file names must be unique".to_owned());
            }
            if !paths.insert(path_text.to_string()) {
                return Err("GACOS import source paths must be unique".to_owned());
            }
            if file.size_bytes == 0 || file.size_bytes > GACOS_MAX_ARTIFACT_BYTES {
                return Err("GACOS import source size is outside the product bound".to_owned());
            }
            if file.name.ends_with(".ztd.rsc")
                && file.size_bytes > GACOS_MAX_RSC_HEADER_BYTES as u64
            {
                return Err("GACOS RSC header exceeds the bounded header limit".to_owned());
            }
            if !is_lower_sha256(&file.sha256) {
                return Err("GACOS import source hash is not lowercase SHA-256".to_owned());
            }
        }
        Ok(())
    }

    fn dates(&self) -> Result<Vec<NaiveDate>, ImportFailure> {
        self.dates
            .iter()
            .map(|date| {
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .map_err(|_| ImportFailure::InvalidManifest)
            })
            .collect()
    }

    fn bbox(&self) -> GacosBoundingBox {
        GacosBoundingBox::new(self.bbox[0], self.bbox[1], self.bbox[2], self.bbox[3])
    }

    fn utc(&self) -> Result<GacosUtcTime, ImportFailure> {
        GacosUtcTime::new(self.hour, self.minute).map_err(|_| ImportFailure::InvalidManifest)
    }

    pub(super) fn total_bytes(&self) -> Result<u64, ImportFailure> {
        self.files.iter().try_fold(0_u64, |total, file| {
            total
                .checked_add(file.size_bytes)
                .ok_or(ImportFailure::InvalidManifest)
        })
    }
}

impl DownloadWorker for NativeGacosImportWorker {
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
        if is_gacos_import_operation(work.task.operation.as_ref()) {
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
        if !is_gacos_import_operation(work.task.operation.as_ref()) {
            return self.base.execute(work, control, progress);
        }

        let task_id = work.task.task_id.clone();
        let item_id = work.item.item_id.clone();
        let destination = match planned_destination(&work.task) {
            Ok(destination) => destination,
            Err(failure) => {
                return failed_result(task_id, item_id, failure, PathBuf::new(), 0);
            }
        };
        if control.is_cancelled() {
            return cancelled_result(task_id, item_id, destination, 0);
        }

        let result = self.execute_import(&work, &control, progress, destination.clone());
        match result {
            Ok(ImportResult { outcome, bytes }) => {
                if outcome == DownloadOutcome::Completed {
                    emit_progress(
                        progress,
                        &work.task.task_id,
                        &work.item.item_id,
                        DownloadItemStatus::Completed,
                        bytes,
                        Some(bytes),
                    );
                }
                DownloadItemResult {
                    task_id,
                    item_id,
                    outcome,
                    destination,
                    bytes_written: bytes,
                    attempt_count: 1,
                    failure: None,
                    finished_at: now(),
                }
            }
            Err(ImportExecutionError::Cancelled { bytes }) => {
                cancelled_result(task_id, item_id, destination, bytes)
            }
            Err(ImportExecutionError::Failed { failure, bytes }) => DownloadItemResult {
                task_id,
                item_id,
                outcome: DownloadOutcome::Failed,
                destination,
                bytes_written: bytes,
                attempt_count: 1,
                failure: Some(DownloadFailure {
                    code: failure.code().to_owned(),
                    reason_zh: failure.reason_zh().to_owned(),
                    retryable: failure.retryable(),
                }),
                finished_at: now(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ImportResult {
    pub(super) outcome: DownloadOutcome,
    pub(super) bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImportExecutionError {
    Cancelled { bytes: u64 },
    Failed { failure: ImportFailure, bytes: u64 },
}

fn is_gacos_import_operation(operation: Option<&TaskOperationDescriptor>) -> bool {
    operation.is_some_and(|operation| {
        operation.kind == "gacos"
            && operation.phase == "execute"
            && operation.approved_worker_operation == GACOS_IMPORT_WORKER
    })
}

fn planned_destination(task: &FrozenTaskSnapshot) -> Result<PathBuf, ImportFailure> {
    if !task.output_root.is_absolute() || task.output_root.as_os_str().is_empty() {
        return Err(ImportFailure::InvalidManifest);
    }
    let key = task_key(&task.task_id);
    Ok(task.output_root.join(key))
}

pub(super) fn task_key(task_id: &str) -> String {
    format!("{:x}", Sha256::digest(task_id.as_bytes()))
}

pub(super) fn staging_path(output_root: &Path, key: &str) -> PathBuf {
    output_root.join(format!(".gacos-{key}.partial"))
}

pub(super) fn lock_path(output_root: &Path, key: &str) -> PathBuf {
    output_root.join(format!(".gacos-{key}.lock"))
}

impl NativeGacosImportWorker {
    fn execute_import(
        &self,
        work: &DownloadWork,
        control: &WorkerControl,
        progress: &mut dyn FnMut(DownloadItemProgress),
        destination: PathBuf,
    ) -> Result<ImportResult, ImportExecutionError> {
        execute_import_with_port(self, work, control, progress, destination)
    }
}

fn execute_import_with_port(
    worker: &NativeGacosImportWorker,
    work: &DownloadWork,
    control: &WorkerControl,
    progress: &mut dyn FnMut(DownloadItemProgress),
    destination: PathBuf,
) -> Result<ImportResult, ImportExecutionError> {
    // Keep all manifest and filesystem logic in the single implementation;
    // this small bridge supplies the worker-owned native GIS port.
    execute_import_inner(worker, work, control, progress, destination)
}

fn execute_import_inner(
    worker: &NativeGacosImportWorker,
    work: &DownloadWork,
    control: &WorkerControl,
    progress: &mut dyn FnMut(DownloadItemProgress),
    destination: PathBuf,
) -> Result<ImportResult, ImportExecutionError> {
    let operation = work
        .task
        .operation
        .as_ref()
        .ok_or(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        })?;
    insar_core::validate_gacos_import_operation(operation).map_err(|_| {
        ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        }
    })?;
    if work.task.initial_selected_item_ids.len() != 1
        || work.task.initial_selected_item_ids.first() != Some(&work.item.item_id)
    {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        });
    }
    let manifest_value = work
        .item
        .payload
        .get(GACOS_IMPORT_MANIFEST_KEY)
        .cloned()
        .ok_or(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        })?;
    let manifest: FrozenGacosImport =
        serde_json::from_value(manifest_value.clone()).map_err(|_| {
            ImportExecutionError::Failed {
                failure: ImportFailure::InvalidManifest,
                bytes: 0,
            }
        })?;
    manifest
        .validate()
        .map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        })?;
    let canonical_value =
        serde_json::to_value(&manifest).map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        })?;
    if canonical_value != manifest_value {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        });
    }
    let expected_manifest_sha = operation
        .payload
        .get("manifest_sha256")
        .and_then(Value::as_str)
        .filter(|value| is_lower_sha256(value))
        .ok_or(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        })?;
    if digest_value(&manifest_value) != expected_manifest_sha {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        });
    }
    for (key, expected) in [
        (
            "source_request_task_id",
            manifest.source_request_task_id.as_str(),
        ),
        (
            "source_request_item_id",
            manifest.source_request_item_id.as_str(),
        ),
    ] {
        if operation.payload.get(key).and_then(Value::as_str) != Some(expected) {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::InvalidManifest,
                bytes: 0,
            });
        }
    }
    if manifest.source_request_task_id == work.task.task_id {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: 0,
        });
    }
    let receipt = GacosImportReceipt::for_task(
        &work.task.task_id,
        &work.item.item_id,
        &manifest,
        expected_manifest_sha,
    );

    let total = manifest
        .total_bytes()
        .map_err(|failure| ImportExecutionError::Failed { failure, bytes: 0 })?;
    emit_progress(
        progress,
        &work.task.task_id,
        &work.item.item_id,
        DownloadItemStatus::Active,
        0,
        Some(total),
    );
    if control.is_cancelled() {
        return Err(ImportExecutionError::Cancelled { bytes: 0 });
    }
    ensure_output_root(&work.task.output_root)
        .map_err(|failure| ImportExecutionError::Failed { failure, bytes: 0 })?;
    let key = task_key(&work.task.task_id);
    let _lock = acquire_import_lock(&work.task.output_root, &key)
        .map_err(|failure| ImportExecutionError::Failed { failure, bytes: 0 })?;
    if path_exists(&destination)
        .map_err(|failure| ImportExecutionError::Failed { failure, bytes: 0 })?
    {
        let bytes = validate_complete_directory(
            &destination,
            &manifest,
            &receipt,
            worker.port.as_deref(),
            control,
            0,
        )?;
        return Ok(ImportResult {
            outcome: DownloadOutcome::Completed,
            bytes,
        });
    }
    let staging = staging_path(&work.task.output_root, &key);
    prepare_staging_directory(&staging, &manifest, &receipt)
        .map_err(|failure| ImportExecutionError::Failed { failure, bytes: 0 })?;
    let bytes = copy_manifest_files(
        &staging,
        &manifest,
        &receipt,
        control,
        progress,
        &work.task.task_id,
        &work.item.item_id,
        total,
    )?;
    if control.is_cancelled() {
        return Err(ImportExecutionError::Cancelled { bytes });
    }
    publish_materialized_gacos_staging(
        &work.task.task_id,
        &work.item.item_id,
        &manifest,
        expected_manifest_sha,
        &staging,
        &destination,
        worker.port.as_deref(),
        control,
        bytes,
    )
}

/// Finish one already-materialized result in the task-owned staging directory.
///
/// Both the local-file importer and the remote result worker use this same
/// boundary. The caller must have materialized only the files described by
/// `manifest` under `staging`; this function rechecks that directory, creates
/// the receipt binding the outer manifest hash to the actual file hashes,
/// performs native artifact admission, and atomically publishes the directory.
#[allow(clippy::too_many_arguments)] // Frozen identity, paths, and lifecycle controls stay explicit.
pub(super) fn publish_materialized_gacos_staging(
    task_id: &str,
    item_id: &str,
    manifest: &FrozenGacosImport,
    expected_manifest_sha: &str,
    staging: &Path,
    destination: &Path,
    port: Option<&dyn DemOperationPort>,
    control: &WorkerControl,
    bytes_before: u64,
) -> Result<ImportResult, ImportExecutionError> {
    manifest
        .validate()
        .map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: bytes_before,
        })?;
    if !is_lower_sha256(expected_manifest_sha) {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: bytes_before,
        });
    }
    if task_id.trim().is_empty() || item_id.trim().is_empty() {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::InvalidManifest,
            bytes: bytes_before,
        });
    }
    let total = manifest
        .total_bytes()
        .map_err(|failure| ImportExecutionError::Failed {
            failure,
            bytes: bytes_before,
        })?;
    let receipt = GacosImportReceipt::for_task(task_id, item_id, manifest, expected_manifest_sha);
    prepare_staging_directory(staging, manifest, &receipt).map_err(|failure| {
        ImportExecutionError::Failed {
            failure,
            bytes: bytes_before,
        }
    })?;
    validate_complete_directory(staging, manifest, &receipt, port, control, bytes_before)?;
    if control.is_cancelled() {
        return Err(ImportExecutionError::Cancelled {
            bytes: bytes_before,
        });
    }
    sync_staging_files(staging, manifest, &receipt).map_err(|failure| {
        ImportExecutionError::Failed {
            failure,
            bytes: bytes_before,
        }
    })?;
    revalidate_staged_hashes(staging, manifest, control, bytes_before)?;
    publish_staging(
        staging,
        destination,
        manifest,
        &receipt,
        port,
        control,
        bytes_before.max(total),
    )?;
    Ok(ImportResult {
        outcome: DownloadOutcome::Completed,
        bytes: total,
    })
}

fn validate_scalar(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().count() > 128 || value.chars().any(char::is_control) {
        Err(format!("{label} is invalid"))
    } else {
        Ok(())
    }
}

fn validate_file_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(':')
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.chars().any(char::is_control)
    {
        return Err("GACOS import file name is invalid".to_owned());
    }
    if !(name.ends_with(".ztd") || name.ends_with(".ztd.rsc") || name.ends_with(".tif")) {
        return Err("GACOS import file type is unsupported".to_owned());
    }
    Ok(())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest_value(value: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
    )
}

pub(super) fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(super) fn emit_progress(
    progress: &mut dyn FnMut(DownloadItemProgress),
    task_id: &str,
    item_id: &str,
    status: DownloadItemStatus,
    bytes_completed: u64,
    total_bytes: Option<u64>,
) {
    progress(DownloadItemProgress {
        task_id: task_id.to_owned(),
        item_id: item_id.to_owned(),
        status,
        bytes_completed,
        total_bytes,
        bytes_per_second: 0.0,
        attempt: 1,
        updated_at: now(),
    });
}

pub(super) fn cancelled_result(
    task_id: String,
    item_id: String,
    destination: PathBuf,
    bytes: u64,
) -> DownloadItemResult {
    DownloadItemResult {
        task_id,
        item_id,
        outcome: DownloadOutcome::Cancelled,
        destination,
        bytes_written: bytes,
        attempt_count: 0,
        failure: None,
        finished_at: now(),
    }
}

pub(super) fn failed_result(
    task_id: String,
    item_id: String,
    failure: ImportFailure,
    destination: PathBuf,
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
            code: failure.code().to_owned(),
            reason_zh: failure.reason_zh().to_owned(),
            retryable: failure.retryable(),
        }),
        finished_at: now(),
    }
}

pub(super) fn ensure_output_root(output_root: &Path) -> Result<(), ImportFailure> {
    fs::create_dir_all(output_root).map_err(|_| ImportFailure::Filesystem)?;
    let metadata = fs::symlink_metadata(output_root).map_err(|_| ImportFailure::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ImportFailure::Filesystem);
    }
    Ok(())
}

pub(super) fn acquire_import_lock(output_root: &Path, key: &str) -> Result<File, ImportFailure> {
    let path = lock_path(output_root, key);
    match fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() != 0 =>
        {
            return Err(ImportFailure::Filesystem);
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(ImportFailure::Filesystem),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|_| ImportFailure::Filesystem)?;
    file.try_lock().map_err(|_| ImportFailure::Busy)?;
    Ok(file)
}

fn path_exists(path: &Path) -> Result<bool, ImportFailure> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ImportFailure::Filesystem),
    }
}

fn prepare_staging_directory(
    staging: &Path,
    manifest: &FrozenGacosImport,
    receipt: &GacosImportReceipt,
) -> Result<(), ImportFailure> {
    match fs::symlink_metadata(staging) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(ImportFailure::StagingConflict)
        }
        Ok(_) => {
            validate_staging_entries(staging, manifest)?;
            ensure_staging_receipt(staging, receipt)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(staging).map_err(|_| ImportFailure::Filesystem)?;
            ensure_staging_receipt(staging, receipt)
        }
        Err(_) => Err(ImportFailure::Filesystem),
    }
}

fn validate_staging_entries(
    staging: &Path,
    manifest: &FrozenGacosImport,
) -> Result<(), ImportFailure> {
    let mut names = manifest
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect::<BTreeSet<_>>();
    names.insert(GACOS_IMPORT_RECEIPT_NAME);
    for entry in fs::read_dir(staging).map_err(|_| ImportFailure::Filesystem)? {
        let entry = entry.map_err(|_| ImportFailure::Filesystem)?;
        let entry_name = entry.file_name();
        let name = entry_name
            .to_str()
            .ok_or(ImportFailure::StagingConflict)?
            .to_owned();
        if names.contains(name.as_str()) {
            continue;
        }
        if let Some(base) = name.strip_suffix(".part") {
            if names.contains(base) {
                continue;
            }
        }
        return Err(ImportFailure::StagingConflict);
    }
    Ok(())
}

fn ensure_staging_receipt(
    staging: &Path,
    expected: &GacosImportReceipt,
) -> Result<(), ImportFailure> {
    let path = staging.join(GACOS_IMPORT_RECEIPT_NAME);
    let expected_bytes = serde_json::to_vec(expected).map_err(|_| ImportFailure::Filesystem)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(ImportFailure::StagingConflict);
            }
            let actual_bytes = read_bounded(&path, metadata.len(), GACOS_IMPORT_RECEIPT_MAX_BYTES)
                .map_err(|_| ImportFailure::StagingConflict)?;
            let actual: GacosImportReceipt = serde_json::from_slice(&actual_bytes)
                .map_err(|_| ImportFailure::StagingConflict)?;
            if actual != *expected
                || serde_json::to_vec(&actual).map_err(|_| ImportFailure::StagingConflict)?
                    != actual_bytes
            {
                return Err(ImportFailure::StagingConflict);
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return ensure_staging_receipt(staging, expected);
                }
                Err(_) => return Err(ImportFailure::Filesystem),
            };
            file.write_all(&expected_bytes)
                .map_err(|_| ImportFailure::Filesystem)?;
            file.sync_all().map_err(|_| ImportFailure::Filesystem)
        }
        Err(_) => Err(ImportFailure::Filesystem),
    }
}

fn read_receipt(directory: &Path, expected: &GacosImportReceipt) -> Result<(), ImportFailure> {
    let path = directory.join(GACOS_IMPORT_RECEIPT_NAME);
    let metadata = fs::symlink_metadata(&path).map_err(|_| ImportFailure::DestinationConflict)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ImportFailure::DestinationConflict);
    }
    let bytes = read_bounded(&path, metadata.len(), GACOS_IMPORT_RECEIPT_MAX_BYTES)
        .map_err(|_| ImportFailure::DestinationConflict)?;
    let actual: GacosImportReceipt =
        serde_json::from_slice(&bytes).map_err(|_| ImportFailure::DestinationConflict)?;
    if actual != *expected
        || serde_json::to_vec(&actual).map_err(|_| ImportFailure::DestinationConflict)? != bytes
    {
        return Err(ImportFailure::DestinationConflict);
    }
    Ok(())
}

// Carry the existing scheduler control/progress scope through the bounded copy.
#[allow(clippy::too_many_arguments)]
fn copy_manifest_files(
    staging: &Path,
    manifest: &FrozenGacosImport,
    receipt: &GacosImportReceipt,
    control: &WorkerControl,
    progress: &mut dyn FnMut(DownloadItemProgress),
    task_id: &str,
    item_id: &str,
    total: u64,
) -> Result<u64, ImportExecutionError> {
    let mut committed = 0_u64;
    for file in &manifest.files {
        wait_until_resumed(control, committed)?;
        let destination = staging.join(&file.name);
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(ImportExecutionError::Failed {
                    failure: ImportFailure::StagingConflict,
                    bytes: committed,
                });
            }
            Ok(metadata) => {
                if metadata.len() != file.size_bytes
                    || hash_path(&destination, Some(file.size_bytes), control, committed)?
                        != file.sha256
                {
                    return Err(ImportExecutionError::Failed {
                        failure: ImportFailure::StagingConflict,
                        bytes: committed,
                    });
                }
                if verify_source_hash(file, control, committed)? != file.sha256 {
                    return Err(ImportExecutionError::Failed {
                        failure: ImportFailure::SourceChanged,
                        bytes: committed,
                    });
                }
                recover_stale_partial_for_completed_file(
                    staging,
                    file,
                    &part_path(&destination),
                    receipt,
                    control,
                    committed,
                )?;
                committed =
                    committed
                        .checked_add(file.size_bytes)
                        .ok_or(ImportExecutionError::Failed {
                            failure: ImportFailure::InvalidManifest,
                            bytes: committed,
                        })?;
                emit_progress(
                    progress,
                    task_id,
                    item_id,
                    DownloadItemStatus::Active,
                    committed,
                    Some(total),
                );
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(ImportExecutionError::Failed {
                    failure: ImportFailure::Filesystem,
                    bytes: committed,
                });
            }
        }

        let partial = part_path(&destination);
        let offset = prepare_partial(file, &partial, control, committed)?;
        let source_hash = verify_source_hash(file, control, committed)?;
        if source_hash != file.sha256 {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::SourceChanged,
                bytes: committed,
            });
        }
        if offset > 0 {
            committed = committed
                .checked_add(offset)
                .ok_or(ImportExecutionError::Failed {
                    failure: ImportFailure::InvalidManifest,
                    bytes: committed,
                })?;
            emit_progress(
                progress,
                task_id,
                item_id,
                DownloadItemStatus::Active,
                committed,
                Some(total),
            );
        }
        let new_bytes = copy_one(
            file, &partial, offset, control, progress, task_id, item_id, committed, total,
        )?;
        committed = committed
            .checked_add(new_bytes)
            .ok_or(ImportExecutionError::Failed {
                failure: ImportFailure::InvalidManifest,
                bytes: committed,
            })?;
        let actual = hash_path(&partial, Some(file.size_bytes), control, committed)?;
        if actual != file.sha256 {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::SourceChanged,
                bytes: committed,
            });
        }
        promote_partial(&partial, &destination, file, control, committed)?;
        emit_progress(
            progress,
            task_id,
            item_id,
            DownloadItemStatus::Active,
            committed,
            Some(total),
        );
    }
    Ok(committed)
}

fn recover_stale_partial_for_completed_file(
    staging: &Path,
    file: &FrozenGacosFile,
    partial: &Path,
    receipt: &GacosImportReceipt,
    control: &WorkerControl,
    bytes: u64,
) -> Result<(), ImportExecutionError> {
    // The caller has already verified the destination and frozen source hash.
    // Recheck the immutable receipt immediately before deleting a redundant
    // partial so a stale or tampered staging directory cannot be cleaned up
    // under a different import identity.
    wait_until_resumed(control, bytes)?;
    read_receipt(staging, receipt).map_err(|_| ImportExecutionError::Failed {
        failure: ImportFailure::StagingConflict,
        bytes,
    })?;
    let metadata = match fs::symlink_metadata(partial) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::StagingConflict,
                bytes,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::StagingConflict,
            bytes,
        });
    }
    let length = metadata.len();
    if length > file.size_bytes || !partial_prefix_matches(file, partial, length, control, bytes)? {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::StagingConflict,
            bytes,
        });
    }
    wait_until_resumed(control, bytes)?;
    let latest = fs::symlink_metadata(partial).map_err(|_| ImportExecutionError::Failed {
        failure: ImportFailure::StagingConflict,
        bytes,
    })?;
    if latest.file_type().is_symlink() || !latest.is_file() || latest.len() != length {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::StagingConflict,
            bytes,
        });
    }
    fs::remove_file(partial).map_err(|_| ImportExecutionError::Failed {
        failure: ImportFailure::Filesystem,
        bytes,
    })
}

fn part_path(destination: &Path) -> PathBuf {
    let mut value = destination.as_os_str().to_os_string();
    value.push(".part");
    PathBuf::from(value)
}

fn wait_until_resumed(control: &WorkerControl, bytes: u64) -> Result<(), ImportExecutionError> {
    while control.is_paused() {
        if control.is_cancelled() {
            return Err(ImportExecutionError::Cancelled { bytes });
        }
        thread::sleep(GACOS_IMPORT_PAUSE_POLL);
    }
    if control.is_cancelled() {
        Err(ImportExecutionError::Cancelled { bytes })
    } else {
        Ok(())
    }
}

fn open_source(file: &FrozenGacosFile) -> Result<File, ImportFailure> {
    let link_metadata =
        fs::symlink_metadata(&file.path).map_err(|_| ImportFailure::SourceUnavailable)?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(ImportFailure::SourceUnavailable);
    }
    let canonical = file
        .path
        .canonicalize()
        .map_err(|_| ImportFailure::SourceUnavailable)?;
    if canonical != file.path {
        return Err(ImportFailure::SourceUnavailable);
    }
    let handle = File::open(&file.path).map_err(|_| ImportFailure::SourceUnavailable)?;
    let metadata = handle
        .metadata()
        .map_err(|_| ImportFailure::SourceUnavailable)?;
    if !metadata.is_file() || metadata.len() != file.size_bytes {
        return Err(ImportFailure::SourceChanged);
    }
    Ok(handle)
}

fn open_staged(path: &Path, expected_size: Option<u64>) -> Result<File, ImportFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ImportFailure::StagingConflict)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ImportFailure::StagingConflict);
    }
    if expected_size.is_some_and(|expected| metadata.len() != expected) {
        return Err(ImportFailure::StagingConflict);
    }
    File::open(path).map_err(|_| ImportFailure::StagingConflict)
}

fn hash_path(
    path: &Path,
    expected_size: Option<u64>,
    control: &WorkerControl,
    bytes: u64,
) -> Result<String, ImportExecutionError> {
    let mut file = open_staged(path, expected_size)
        .map_err(|failure| ImportExecutionError::Failed { failure, bytes })?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; GACOS_IMPORT_COPY_CHUNK_BYTES];
    loop {
        wait_until_resumed(control, bytes)?;
        let read = file
            .read(&mut buffer)
            .map_err(|_| ImportExecutionError::Failed {
                failure: ImportFailure::StagingConflict,
                bytes,
            })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn hash_source(
    file: &FrozenGacosFile,
    control: &WorkerControl,
    bytes: u64,
) -> Result<String, ImportExecutionError> {
    let mut source =
        open_source(file).map_err(|failure| ImportExecutionError::Failed { failure, bytes })?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; GACOS_IMPORT_COPY_CHUNK_BYTES];
    loop {
        wait_until_resumed(control, bytes)?;
        let read = source
            .read(&mut buffer)
            .map_err(|_| ImportExecutionError::Failed {
                failure: ImportFailure::SourceUnavailable,
                bytes,
            })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn verify_source_hash(
    file: &FrozenGacosFile,
    control: &WorkerControl,
    bytes: u64,
) -> Result<String, ImportExecutionError> {
    hash_source(file, control, bytes)
}

fn prepare_partial(
    file: &FrozenGacosFile,
    partial: &Path,
    control: &WorkerControl,
    bytes: u64,
) -> Result<u64, ImportExecutionError> {
    let metadata = match fs::symlink_metadata(partial) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let _new_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(partial)
                .map_err(|_| ImportExecutionError::Failed {
                    failure: ImportFailure::StagingConflict,
                    bytes,
                })?;
            return Ok(0);
        }
        Err(_) => {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::Filesystem,
                bytes,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::StagingConflict,
            bytes,
        });
    }
    let length = metadata.len();
    if length > file.size_bytes || !partial_prefix_matches(file, partial, length, control, bytes)? {
        truncate_partial(partial)
            .map_err(|failure| ImportExecutionError::Failed { failure, bytes })?;
        return Ok(0);
    }
    Ok(length)
}

fn partial_prefix_matches(
    file: &FrozenGacosFile,
    partial: &Path,
    length: u64,
    control: &WorkerControl,
    bytes: u64,
) -> Result<bool, ImportExecutionError> {
    if length == 0 {
        return Ok(true);
    }
    let mut source =
        open_source(file).map_err(|failure| ImportExecutionError::Failed { failure, bytes })?;
    let mut partial = open_staged(partial, Some(length))
        .map_err(|failure| ImportExecutionError::Failed { failure, bytes })?;
    let mut remaining = length;
    let mut source_buffer = vec![0_u8; GACOS_IMPORT_COPY_CHUNK_BYTES];
    let mut partial_buffer = vec![0_u8; GACOS_IMPORT_COPY_CHUNK_BYTES];
    while remaining > 0 {
        wait_until_resumed(control, bytes)?;
        let amount = remaining.min(GACOS_IMPORT_COPY_CHUNK_BYTES as u64) as usize;
        source
            .read_exact(&mut source_buffer[..amount])
            .map_err(|_| ImportExecutionError::Failed {
                failure: ImportFailure::SourceUnavailable,
                bytes,
            })?;
        partial
            .read_exact(&mut partial_buffer[..amount])
            .map_err(|_| ImportExecutionError::Failed {
                failure: ImportFailure::StagingConflict,
                bytes,
            })?;
        if source_buffer[..amount] != partial_buffer[..amount] {
            return Ok(false);
        }
        remaining -= amount as u64;
    }
    Ok(true)
}

fn truncate_partial(partial: &Path) -> Result<(), ImportFailure> {
    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(partial)
        .map_err(|_| ImportFailure::StagingConflict)?;
    file.sync_all().map_err(|_| ImportFailure::Filesystem)
}

// Carry the existing scheduler control/progress scope through the bounded copy.
#[allow(clippy::too_many_arguments)]
fn copy_one(
    source_file: &FrozenGacosFile,
    partial: &Path,
    offset: u64,
    control: &WorkerControl,
    progress: &mut dyn FnMut(DownloadItemProgress),
    task_id: &str,
    item_id: &str,
    committed_before: u64,
    total: u64,
) -> Result<u64, ImportExecutionError> {
    let mut source = open_source(source_file).map_err(|failure| ImportExecutionError::Failed {
        failure,
        bytes: committed_before,
    })?;
    source
        .seek(SeekFrom::Start(offset))
        .map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::SourceUnavailable,
            bytes: committed_before,
        })?;
    let mut output = OpenOptions::new()
        .read(true)
        .write(true)
        .open(partial)
        .map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::StagingConflict,
            bytes: committed_before,
        })?;
    output
        .seek(SeekFrom::Start(offset))
        .map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::StagingConflict,
            bytes: committed_before,
        })?;
    let mut copied = offset;
    let mut buffer = vec![0_u8; GACOS_IMPORT_COPY_CHUNK_BYTES];
    while copied < source_file.size_bytes {
        wait_until_resumed(control, committed_before + copied - offset)?;
        let wanted = (source_file.size_bytes - copied).min(buffer.len() as u64) as usize;
        let read =
            source
                .read(&mut buffer[..wanted])
                .map_err(|_| ImportExecutionError::Failed {
                    failure: ImportFailure::SourceUnavailable,
                    bytes: committed_before + copied - offset,
                })?;
        if read == 0 {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::SourceChanged,
                bytes: committed_before + copied - offset,
            });
        }
        output
            .write_all(&buffer[..read])
            .map_err(|_| ImportExecutionError::Failed {
                failure: ImportFailure::Filesystem,
                bytes: committed_before + copied - offset,
            })?;
        copied += read as u64;
        emit_progress(
            progress,
            task_id,
            item_id,
            DownloadItemStatus::Active,
            committed_before + copied - offset,
            Some(total),
        );
    }
    if source
        .metadata()
        .map_err(|_| ImportFailure::SourceUnavailable)
        .map_err(|failure| ImportExecutionError::Failed {
            failure,
            bytes: committed_before + copied - offset,
        })?
        .len()
        != source_file.size_bytes
    {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::SourceChanged,
            bytes: committed_before + copied - offset,
        });
    }
    output
        .sync_all()
        .map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::Filesystem,
            bytes: committed_before + copied - offset,
        })?;
    Ok(copied - offset)
}

fn promote_partial(
    partial: &Path,
    destination: &Path,
    file: &FrozenGacosFile,
    control: &WorkerControl,
    bytes: u64,
) -> Result<(), ImportExecutionError> {
    wait_until_resumed(control, bytes)?;
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(ImportExecutionError::Failed {
                failure: ImportFailure::StagingConflict,
                bytes,
            })
        }
        Ok(metadata) => {
            if metadata.len() != file.size_bytes
                || hash_path(destination, Some(file.size_bytes), control, bytes)? != file.sha256
            {
                return Err(ImportExecutionError::Failed {
                    failure: ImportFailure::StagingConflict,
                    bytes,
                });
            }
            fs::remove_file(partial).map_err(|_| ImportExecutionError::Failed {
                failure: ImportFailure::Filesystem,
                bytes,
            })?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::rename(partial, destination)
            .map_err(|_| ImportExecutionError::Failed {
                failure: ImportFailure::Promotion,
                bytes,
            }),
        Err(_) => Err(ImportExecutionError::Failed {
            failure: ImportFailure::Filesystem,
            bytes,
        }),
    }
}

fn validate_complete_directory(
    directory: &Path,
    manifest: &FrozenGacosImport,
    receipt: &GacosImportReceipt,
    port: Option<&dyn DemOperationPort>,
    control: &WorkerControl,
    bytes_before: u64,
) -> Result<u64, ImportExecutionError> {
    wait_until_resumed(control, bytes_before)?;
    let metadata = fs::symlink_metadata(directory).map_err(|_| ImportExecutionError::Failed {
        failure: ImportFailure::DestinationConflict,
        bytes: bytes_before,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ImportExecutionError::Failed {
            failure: ImportFailure::DestinationConflict,
            bytes: bytes_before,
        });
    }
    let mut expected_names = manifest
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect::<BTreeSet<_>>();
    expected_names.insert(GACOS_IMPORT_RECEIPT_NAME);
    for entry in fs::read_dir(directory).map_err(|_| ImportExecutionError::Failed {
        failure: ImportFailure::Filesystem,
        bytes: bytes_before,
    })? {
        let entry = entry.map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::Filesystem,
            bytes: bytes_before,
        })?;
        let entry_name = entry.file_name();
        let name = entry_name.to_str().ok_or(ImportExecutionError::Failed {
            failure: ImportFailure::DestinationConflict,
            bytes: bytes_before,
        })?;
        if !expected_names.contains(name) {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::DestinationConflict,
                bytes: bytes_before,
            });
        }
    }
    read_receipt(directory, receipt).map_err(|failure| ImportExecutionError::Failed {
        failure,
        bytes: bytes_before,
    })?;
    let mut bytes = 0_u64;
    for file in &manifest.files {
        let path = directory.join(&file.name);
        let metadata = fs::symlink_metadata(&path).map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::DestinationConflict,
            bytes: bytes_before.max(bytes),
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::DestinationConflict,
                bytes: bytes_before.max(bytes),
            });
        }
        if metadata.len() != file.size_bytes {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::DestinationConflict,
                bytes: bytes_before.max(bytes),
            });
        }
        let completed = bytes_before.max(bytes);
        let hash = hash_path(&path, Some(file.size_bytes), control, completed)?;
        if hash != file.sha256 {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::DestinationConflict,
                bytes: completed,
            });
        }
        bytes = bytes
            .checked_add(file.size_bytes)
            .ok_or(ImportExecutionError::Failed {
                failure: ImportFailure::InvalidManifest,
                bytes: bytes_before,
            })?;
    }
    wait_until_resumed(control, bytes_before.max(bytes))?;
    admit_directory_evidence(directory, manifest, port).map_err(|failure| {
        ImportExecutionError::Failed {
            failure,
            bytes: bytes_before.max(bytes),
        }
    })?;
    wait_until_resumed(control, bytes_before.max(bytes))?;
    Ok(bytes)
}

struct EvidenceStorage {
    name: String,
    size_bytes: u64,
    header_bytes: Option<Vec<u8>>,
    tiff_prefix: Option<Vec<u8>>,
    raster_inspection: Option<GacosRasterInspectionFacts>,
}

fn admit_directory_evidence(
    directory: &Path,
    manifest: &FrozenGacosImport,
    port: Option<&dyn DemOperationPort>,
) -> Result<(), ImportFailure> {
    let mut storage = Vec::with_capacity(manifest.files.len());
    for file in &manifest.files {
        let path = directory.join(&file.name);
        let header_bytes = if file.name.ends_with(".ztd.rsc") {
            Some(read_bounded(
                &path,
                file.size_bytes,
                GACOS_MAX_RSC_HEADER_BYTES as u64,
            )?)
        } else {
            None
        };
        let (tiff_prefix, raster_inspection) = if file.name.ends_with(".tif") {
            let prefix = read_prefix(&path, file.size_bytes, GACOS_TIFF_PREFIX_BYTES as u64)?;
            let port = port.ok_or(ImportFailure::RasterInspectionUnavailable)?;
            let inspection = port
                .call(
                    "dem_inspect",
                    serde_json::json!({
                        "path": path.to_string_lossy().to_string(),
                    }),
                )
                .map_err(|_| ImportFailure::RasterInspectionUnavailable)?;
            (Some(prefix), Some(parse_raster_inspection(&inspection)?))
        } else {
            (None, None)
        };
        storage.push(EvidenceStorage {
            name: file.name.clone(),
            size_bytes: file.size_bytes,
            header_bytes,
            tiff_prefix,
            raster_inspection,
        });
    }
    let evidence = storage
        .iter()
        .map(|file| GacosArtifactEvidence {
            name: &file.name,
            size_bytes: file.size_bytes,
            header_bytes: file.header_bytes.as_deref(),
            tiff_prefix: file.tiff_prefix.as_deref(),
            raster_inspection: file.raster_inspection.as_ref(),
        })
        .collect::<Vec<_>>();
    let dates = manifest
        .dates()
        .map_err(|_| ImportFailure::InvalidManifest)?;
    let utc = manifest.utc()?;
    let request = GacosArtifactEvidenceRequest::new(&dates, manifest.bbox(), utc, &evidence);
    admit_gacos_artifact_evidence(&request)
        .map(|_| ())
        .map_err(map_artifact_error)
}

fn read_bounded(path: &Path, expected_size: u64, bound: u64) -> Result<Vec<u8>, ImportFailure> {
    if expected_size > bound || expected_size > usize::MAX as u64 {
        return Err(ImportFailure::ArtifactInvalid);
    }
    let file = open_staged(path, Some(expected_size))?;
    let mut bytes = Vec::with_capacity(expected_size as usize);
    let mut limited = file.take(bound.saturating_add(1));
    limited
        .read_to_end(&mut bytes)
        .map_err(|_| ImportFailure::ArtifactInvalid)?;
    if bytes.len() as u64 != expected_size || bytes.len() as u64 > bound {
        return Err(ImportFailure::ArtifactInvalid);
    }
    Ok(bytes)
}

fn read_prefix(
    path: &Path,
    expected_size: u64,
    prefix_size: u64,
) -> Result<Vec<u8>, ImportFailure> {
    if expected_size < prefix_size {
        return Err(ImportFailure::ArtifactInvalid);
    }
    let mut file = open_staged(path, Some(expected_size))?;
    let mut prefix = vec![0_u8; prefix_size as usize];
    file.read_exact(&mut prefix)
        .map_err(|_| ImportFailure::ArtifactInvalid)?;
    Ok(prefix)
}

fn parse_raster_inspection(value: &Value) -> Result<GacosRasterInspectionFacts, ImportFailure> {
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(ImportFailure::RasterInspectionInvalid);
    }
    let metadata = value
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or(ImportFailure::RasterInspectionInvalid)?;
    let driver = metadata
        .get("driver")
        .and_then(Value::as_str)
        .ok_or(ImportFailure::RasterInspectionInvalid)?
        .to_owned();
    let width = metadata
        .get("width")
        .and_then(Value::as_u64)
        .ok_or(ImportFailure::RasterInspectionInvalid)?;
    let height = metadata
        .get("height")
        .and_then(Value::as_u64)
        .ok_or(ImportFailure::RasterInspectionInvalid)?;
    let band_count = metadata
        .get("count")
        .and_then(Value::as_u64)
        .ok_or(ImportFailure::RasterInspectionInvalid)?;
    let dtypes = match metadata.get("dtype") {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(ImportFailure::RasterInspectionInvalid)
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::String(value)) => vec![value.clone()],
        _ => return Err(ImportFailure::RasterInspectionInvalid),
    };
    let crs = metadata
        .get("crs")
        .and_then(Value::as_str)
        .ok_or(ImportFailure::RasterInspectionInvalid)?
        .to_owned();
    let bounds = parse_array::<4>(metadata.get("bounds"))?;
    let transform = parse_array::<6>(metadata.get("geotransform"))?;
    if transform[2].abs() > GACOS_ROTATION_EPSILON
        || transform[4].abs() > GACOS_ROTATION_EPSILON
        || !transform.iter().all(|value| value.is_finite())
    {
        return Err(ImportFailure::RasterInspectionInvalid);
    }
    Ok(GacosRasterInspectionFacts::new(
        driver,
        width,
        height,
        band_count,
        dtypes,
        crs,
        bounds,
        transform[1],
        transform[5],
    ))
}

fn parse_array<const N: usize>(value: Option<&Value>) -> Result<[f64; N], ImportFailure> {
    let values = value
        .and_then(Value::as_array)
        .ok_or(ImportFailure::RasterInspectionInvalid)?;
    if values.len() != N {
        return Err(ImportFailure::RasterInspectionInvalid);
    }
    let mut result = [0.0; N];
    for (index, value) in values.iter().enumerate() {
        result[index] = value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or(ImportFailure::RasterInspectionInvalid)?;
    }
    Ok(result)
}

fn map_artifact_error(error: GacosArtifactError) -> ImportFailure {
    match error {
        GacosArtifactError::MissingRasterInspection
        | GacosArtifactError::MissingRasterPrefix
        | GacosArtifactError::RasterPrefixIncomplete
        | GacosArtifactError::RasterPrefixTooLarge
        | GacosArtifactError::InvalidRasterBytes
        | GacosArtifactError::InvalidRasterDriver
        | GacosArtifactError::InvalidRasterDimensions
        | GacosArtifactError::InvalidRasterDataType
        | GacosArtifactError::InvalidRasterGeoreference
        | GacosArtifactError::InvalidRasterCrs => ImportFailure::RasterInspectionInvalid,
        _ => ImportFailure::ArtifactInvalid,
    }
}

fn revalidate_staged_hashes(
    staging: &Path,
    manifest: &FrozenGacosImport,
    control: &WorkerControl,
    bytes: u64,
) -> Result<(), ImportExecutionError> {
    wait_until_resumed(control, bytes)?;
    for file in &manifest.files {
        let path = staging.join(&file.name);
        let metadata = fs::symlink_metadata(&path).map_err(|_| ImportExecutionError::Failed {
            failure: ImportFailure::StagingConflict,
            bytes,
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::StagingConflict,
                bytes,
            });
        }
        if metadata.len() != file.size_bytes
            || hash_path(&path, Some(file.size_bytes), control, bytes)? != file.sha256
        {
            return Err(ImportExecutionError::Failed {
                failure: ImportFailure::StagingConflict,
                bytes,
            });
        }
    }
    wait_until_resumed(control, bytes)?;
    Ok(())
}

fn sync_staging_files(
    staging: &Path,
    manifest: &FrozenGacosImport,
    receipt: &GacosImportReceipt,
) -> Result<(), ImportFailure> {
    for file in &manifest.files {
        let path = staging.join(&file.name);
        let metadata = fs::symlink_metadata(&path).map_err(|_| ImportFailure::StagingConflict)?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != file.size_bytes
        {
            return Err(ImportFailure::StagingConflict);
        }
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|_| ImportFailure::Filesystem)?;
        handle.sync_all().map_err(|_| ImportFailure::Filesystem)?;
    }
    let receipt_path = staging.join(GACOS_IMPORT_RECEIPT_NAME);
    let receipt_metadata =
        fs::symlink_metadata(&receipt_path).map_err(|_| ImportFailure::StagingConflict)?;
    if receipt_metadata.file_type().is_symlink() || !receipt_metadata.is_file() {
        return Err(ImportFailure::StagingConflict);
    }
    read_receipt(staging, receipt).map_err(|_| ImportFailure::StagingConflict)?;
    let handle = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&receipt_path)
        .map_err(|_| ImportFailure::StagingConflict)?;
    handle.sync_all().map_err(|_| ImportFailure::Filesystem)?;
    Ok(())
}

fn publish_staging(
    staging: &Path,
    destination: &Path,
    manifest: &FrozenGacosImport,
    receipt: &GacosImportReceipt,
    port: Option<&dyn DemOperationPort>,
    control: &WorkerControl,
    bytes: u64,
) -> Result<bool, ImportExecutionError> {
    wait_until_resumed(control, bytes)?;
    if path_exists(destination)
        .map_err(|failure| ImportExecutionError::Failed { failure, bytes })?
    {
        validate_complete_directory(destination, manifest, receipt, port, control, bytes)?;
        return Ok(false);
    }
    wait_until_resumed(control, bytes)?;
    match fs::rename(staging, destination) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            validate_complete_directory(destination, manifest, receipt, port, control, bytes)?;
            Ok(false)
        }
        Err(_) => Err(ImportExecutionError::Failed {
            failure: ImportFailure::Promotion,
            bytes,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insar_core::{DownloadLayout, SnapshotItem};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct PanicWorker;

    impl DownloadWorker for PanicWorker {
        fn execute(
            &self,
            _work: DownloadWork,
            _control: WorkerControl,
            _progress: &mut dyn FnMut(DownloadItemProgress),
        ) -> DownloadItemResult {
            panic!("base worker should not execute the GACOS import operation")
        }
    }

    struct RasterPort {
        calls: Arc<AtomicUsize>,
        rotated: bool,
    }

    impl DemOperationPort for RasterPort {
        fn call(&self, operation: &str, _payload: Value) -> Result<Value, String> {
            assert_eq!(operation, "dem_inspect");
            self.calls.fetch_add(1, Ordering::SeqCst);
            let transform = if self.rotated {
                serde_json::json!([110.0, 0.001, 0.01, 30.002, 0.0, -0.001])
            } else {
                serde_json::json!([110.0, 0.001, 0.0, 30.002, 0.0, -0.001])
            };
            Ok(serde_json::json!({
                "ok": true,
                "metadata": {
                    "driver": "GTiff",
                    "width": 2,
                    "height": 2,
                    "count": 1,
                    "dtype": ["float32"],
                    "crs": "EPSG:4326",
                    "bounds": [110.0, 30.0, 110.002, 30.002],
                    "geotransform": transform,
                }
            }))
        }
    }

    fn unique_root(label: &str) -> PathBuf {
        let nonce = format!(
            "{}-{}-{}",
            label,
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        std::env::temp_dir().join(nonce)
    }

    fn binary_header() -> &'static [u8] {
        b"WIDTH 2\nFILE_LENGTH 2\nX_FIRST 110.0\nY_FIRST 30.002\nX_STEP 0.001\nY_STEP -0.001\n"
    }

    fn binary_manifest(root: &Path) -> FrozenGacosImport {
        let source = root.join("20240312.ztd");
        let header = root.join("20240312.ztd.rsc");
        fs::write(&source, [0_u8; 16]).expect("write binary source");
        fs::write(&header, binary_header()).expect("write header source");
        FrozenGacosImport {
            schema_version: GACOS_IMPORT_SCHEMA_VERSION,
            source_request_task_id: "request-task".to_owned(),
            source_request_item_id: "request-item".to_owned(),
            bbox: [110.0, 30.001, 110.001, 30.002],
            dates: vec!["2024-03-12".to_owned()],
            hour: 10,
            minute: 26,
            files: vec![
                FrozenGacosFile {
                    path: source.canonicalize().expect("canonical fixture source"),
                    name: "20240312.ztd".to_owned(),
                    size_bytes: 16,
                    sha256: format!("{:x}", Sha256::digest([0_u8; 16])),
                },
                FrozenGacosFile {
                    path: header.canonicalize().expect("canonical fixture source"),
                    name: "20240312.ztd.rsc".to_owned(),
                    size_bytes: binary_header().len() as u64,
                    sha256: format!("{:x}", Sha256::digest(binary_header())),
                },
            ],
        }
    }

    fn work_for(manifest: &FrozenGacosImport, output_root: &Path) -> DownloadWork {
        let value = serde_json::to_value(manifest).expect("manifest value");
        let manifest_sha = digest_value(&value);
        let operation = TaskOperationDescriptor {
            operation_id: "import-task:import".to_owned(),
            kind: "gacos".to_owned(),
            phase: "execute".to_owned(),
            region_id: "request-batch".to_owned(),
            approved_worker_operation: GACOS_IMPORT_WORKER.to_owned(),
            payload: serde_json::json!({
                "source_request_task_id": manifest.source_request_task_id,
                "source_request_item_id": manifest.source_request_item_id,
                "manifest_sha256": manifest_sha,
            })
            .as_object()
            .expect("operation payload")
            .clone(),
        };
        let item_id = "import-item".to_owned();
        let item = SnapshotItem {
            item_id: item_id.clone(),
            provider_id: "gacos.local-result".to_owned(),
            display_name: "GACOS result import".to_owned(),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: serde_json::json!({GACOS_IMPORT_MANIFEST_KEY: value})
                .as_object()
                .expect("item payload")
                .clone(),
            asset_access: None,
            execution_policy: None,
        };
        DownloadWork {
            task: FrozenTaskSnapshot {
                schema_version: 1,
                task_id: "import-task".to_owned(),
                batch_id: "import-batch".to_owned(),
                provider_id: "gacos.local-result".to_owned(),
                created_at: "2026-09-06T00:00:00Z".to_owned(),
                aoi_name: "test".to_owned(),
                aoi_geojson: None,
                query: Map::new(),
                candidates: vec![item.clone()],
                initial_selected_item_ids: vec![item_id],
                output_root: output_root.to_owned(),
                layout: DownloadLayout::Direct,
                operation: Some(operation),
                orbit_association: None,
            },
            item,
        }
    }

    #[test]
    fn binary_import_publishes_atomically_and_reuses_exact_output() {
        let root = unique_root("gacos-import-binary");
        let source_root = root.join("source");
        let output_root = root.join("output");
        fs::create_dir_all(&source_root).expect("source root");
        let manifest = binary_manifest(&source_root);
        let work = work_for(&manifest, &output_root);
        let worker = NativeGacosImportWorker {
            base: Arc::new(PanicWorker),
            port: None,
        };
        let mut progress = Vec::new();
        let first = worker.execute(work.clone(), WorkerControl::default(), &mut |value| {
            progress.push(value)
        });
        assert_eq!(first.outcome, DownloadOutcome::Completed, "{first:?}");
        assert_eq!(first.bytes_written, manifest.total_bytes().unwrap());
        assert!(progress.iter().any(|value| {
            value.status == DownloadItemStatus::Completed
                && value.bytes_completed == value.total_bytes.unwrap_or_default()
        }));
        assert!(first.destination.join("20240312.ztd").is_file());
        assert!(first.destination.join("20240312.ztd.rsc").is_file());

        let second = worker.execute(work, WorkerControl::default(), &mut |_| {});
        assert_eq!(second.outcome, DownloadOutcome::Completed);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn matching_partial_prefix_is_resumed_and_published() {
        let root = unique_root("gacos-import-partial");
        let source_root = root.join("source");
        let output_root = root.join("output");
        fs::create_dir_all(&source_root).expect("source root");
        let manifest = binary_manifest(&source_root);
        let work = work_for(&manifest, &output_root);
        let worker = NativeGacosImportWorker {
            base: Arc::new(PanicWorker),
            port: None,
        };
        fs::create_dir_all(&output_root).expect("output root");
        let key = task_key(&work.task.task_id);
        let staging = staging_path(&output_root, &key);
        fs::create_dir_all(&staging).expect("staging root");
        let manifest_value = serde_json::to_value(&manifest).expect("manifest value");
        let receipt = GacosImportReceipt::for_task(
            &work.task.task_id,
            &work.item.item_id,
            &manifest,
            &digest_value(&manifest_value),
        );
        ensure_staging_receipt(&staging, &receipt).expect("staging receipt");
        let source_bytes = fs::read(&manifest.files[0].path).expect("source bytes");
        fs::write(
            staging.join(format!("{}.part", manifest.files[0].name)),
            &source_bytes[..5],
        )
        .expect("partial prefix");

        let mut progress = Vec::new();
        let result = worker.execute(work, WorkerControl::default(), &mut |value| {
            progress.push(value)
        });
        assert_eq!(result.outcome, DownloadOutcome::Completed, "{result:?}");
        assert_eq!(result.bytes_written, manifest.total_bytes().unwrap());
        assert!(progress.iter().any(|value| value.bytes_completed == 5));
        assert!(result.destination.join(GACOS_IMPORT_RECEIPT_NAME).is_file());
        assert!(!staging.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn complete_staged_file_rechecks_frozen_source_before_reuse() {
        let root = unique_root("gacos-import-staged-source");
        let source_root = root.join("source");
        let output_root = root.join("output");
        fs::create_dir_all(&source_root).expect("source root");
        let manifest = binary_manifest(&source_root);
        let work = work_for(&manifest, &output_root);
        let worker = NativeGacosImportWorker {
            base: Arc::new(PanicWorker),
            port: None,
        };
        fs::create_dir_all(&output_root).expect("output root");
        let key = task_key(&work.task.task_id);
        let staging = staging_path(&output_root, &key);
        fs::create_dir_all(&staging).expect("staging root");
        let manifest_value = serde_json::to_value(&manifest).expect("manifest value");
        let receipt = GacosImportReceipt::for_task(
            &work.task.task_id,
            &work.item.item_id,
            &manifest,
            &digest_value(&manifest_value),
        );
        ensure_staging_receipt(&staging, &receipt).expect("staging receipt");
        let source_path = &manifest.files[0].path;
        let original = fs::read(source_path).expect("original source");
        fs::write(staging.join(&manifest.files[0].name), &original).expect("staged file");
        fs::write(source_path, [9_u8; 16]).expect("changed source");

        let failed = worker.execute(work.clone(), WorkerControl::default(), &mut |_| {});
        assert_eq!(failed.outcome, DownloadOutcome::Failed, "{failed:?}");
        assert_eq!(
            failed.failure.as_ref().unwrap().code,
            "NATIVE_GACOS_SOURCE_CHANGED"
        );
        assert!(!failed.destination.exists());
        assert_eq!(
            fs::read(staging.join(&manifest.files[0].name)).expect("staged bytes"),
            original
        );

        fs::write(source_path, original).expect("restore source");
        let completed = worker.execute(work, WorkerControl::default(), &mut |_| {});
        assert_eq!(
            completed.outcome,
            DownloadOutcome::Completed,
            "{completed:?}"
        );
        assert!(!staging.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn completed_staged_file_removes_owned_stale_partial_before_publish() {
        let root = unique_root("gacos-import-stale-partial");
        let source_root = root.join("source");
        let output_root = root.join("output");
        fs::create_dir_all(&source_root).expect("source root");
        let manifest = binary_manifest(&source_root);
        let work = work_for(&manifest, &output_root);
        let worker = NativeGacosImportWorker {
            base: Arc::new(PanicWorker),
            port: None,
        };
        fs::create_dir_all(&output_root).expect("output root");
        let key = task_key(&work.task.task_id);
        let staging = staging_path(&output_root, &key);
        fs::create_dir_all(&staging).expect("staging root");
        let manifest_value = serde_json::to_value(&manifest).expect("manifest value");
        let receipt = GacosImportReceipt::for_task(
            &work.task.task_id,
            &work.item.item_id,
            &manifest,
            &digest_value(&manifest_value),
        );
        ensure_staging_receipt(&staging, &receipt).expect("staging receipt");
        let source_path = &manifest.files[0].path;
        let source_bytes = fs::read(source_path).expect("source bytes");
        fs::write(staging.join(&manifest.files[0].name), &source_bytes).expect("staged file");
        fs::write(
            staging.join(format!("{}.part", manifest.files[0].name)),
            &source_bytes[..5],
        )
        .expect("stale partial");

        let completed = worker.execute(work, WorkerControl::default(), &mut |_| {});
        assert_eq!(
            completed.outcome,
            DownloadOutcome::Completed,
            "{completed:?}"
        );
        assert!(!staging.exists());
        assert_eq!(
            fs::read(completed.destination.join(&manifest.files[0].name)).expect("published bytes"),
            source_bytes
        );
        assert!(!completed
            .destination
            .join(format!("{}.part", manifest.files[0].name))
            .exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn source_change_fails_without_publish_then_retry_revalidates_and_completes() {
        let root = unique_root("gacos-import-retry");
        let source_root = root.join("source");
        let output_root = root.join("output");
        fs::create_dir_all(&source_root).expect("source root");
        let manifest = binary_manifest(&source_root);
        let work = work_for(&manifest, &output_root);
        let worker = NativeGacosImportWorker {
            base: Arc::new(PanicWorker),
            port: None,
        };
        let original = fs::read(&manifest.files[0].path).expect("original source");
        fs::write(&manifest.files[0].path, [9_u8; 16]).expect("changed source");
        let failed = worker.execute(work.clone(), WorkerControl::default(), &mut |_| {});
        assert_eq!(failed.outcome, DownloadOutcome::Failed);
        assert_eq!(
            failed.failure.as_ref().unwrap().code,
            "NATIVE_GACOS_SOURCE_CHANGED"
        );
        let key = task_key(&work.task.task_id);
        let staging = staging_path(&output_root, &key);
        assert!(staging.join(GACOS_IMPORT_RECEIPT_NAME).is_file());
        assert!(!failed.destination.exists());

        fs::write(&manifest.files[0].path, original).expect("restore source");
        let completed = worker.execute(work, WorkerControl::default(), &mut |_| {});
        assert_eq!(
            completed.outcome,
            DownloadOutcome::Completed,
            "{completed:?}"
        );
        assert_eq!(completed.bytes_written, manifest.total_bytes().unwrap());
        assert!(completed.destination.join("20240312.ztd").is_file());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn foreign_final_file_is_never_overwritten() {
        let root = unique_root("gacos-import-conflict");
        let source_root = root.join("source");
        let output_root = root.join("output");
        fs::create_dir_all(&source_root).expect("source root");
        let manifest = binary_manifest(&source_root);
        let work = work_for(&manifest, &output_root);
        let worker = NativeGacosImportWorker {
            base: Arc::new(PanicWorker),
            port: None,
        };
        let first = worker.execute(work.clone(), WorkerControl::default(), &mut |_| {});
        assert_eq!(first.outcome, DownloadOutcome::Completed, "{first:?}");
        fs::write(first.destination.join("20240312.ztd"), [9_u8; 16]).expect("foreign bytes");
        let second = worker.execute(work, WorkerControl::default(), &mut |_| {});
        assert_eq!(second.outcome, DownloadOutcome::Failed);
        assert_eq!(
            second.failure.as_ref().unwrap().code,
            "NATIVE_GACOS_DESTINATION_CONFLICT"
        );
        assert_eq!(
            fs::read(first.destination.join("20240312.ztd")).unwrap(),
            [9_u8; 16]
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rotated_native_raster_transform_is_rejected() {
        let value = serde_json::json!({
            "metadata": {
                "driver": "GTiff",
                "width": 2,
                "height": 2,
                "count": 1,
                "dtype": ["float32"],
                "crs": "EPSG:4326",
                "bounds": [110.0, 30.0, 110.002, 30.002],
                "geotransform": [110.0, 0.001, 0.001, 30.002, 0.0, -0.001]
            }
        });
        assert_eq!(
            parse_raster_inspection(&value),
            Err(ImportFailure::RasterInspectionInvalid)
        );
    }

    #[test]
    fn geotiff_import_requires_native_port_and_preserves_requested_utc_metadata_shape() {
        let root = unique_root("gacos-import-tiff");
        let source_root = root.join("source");
        let output_root = root.join("output");
        fs::create_dir_all(&source_root).expect("source root");
        let source = source_root.join("20240312.tif");
        let bytes = b"II*\0tiff-fixture";
        fs::write(&source, bytes).expect("write tiff source");
        let manifest = FrozenGacosImport {
            schema_version: GACOS_IMPORT_SCHEMA_VERSION,
            source_request_task_id: "request-task".to_owned(),
            source_request_item_id: "request-item".to_owned(),
            bbox: [110.0, 30.0, 110.002, 30.002],
            dates: vec!["2024-03-12".to_owned()],
            hour: 10,
            minute: 26,
            files: vec![FrozenGacosFile {
                path: source.canonicalize().expect("canonical fixture source"),
                name: "20240312.tif".to_owned(),
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            }],
        };
        let work = work_for(&manifest, &output_root);
        let calls = Arc::new(AtomicUsize::new(0));
        let worker = NativeGacosImportWorker {
            base: Arc::new(PanicWorker),
            port: Some(Arc::new(RasterPort {
                calls: Arc::clone(&calls),
                rotated: false,
            })),
        };
        let result = worker.execute(work, WorkerControl::default(), &mut |_| {});
        assert_eq!(result.outcome, DownloadOutcome::Completed, "{result:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
