//! The worker gate (downloader D3, D3-1d). Inside the destination lease and
//! before credentials, Provider resolution and network admission, it decides
//! an existing planned file, renames a conflict aside and records the facts
//! in the Store through the Scheduler coordinator.

use std::path::Path;

use insar_core::{AssetRole, DownloadFailure, DownloadItemResult, DownloadOutcome};
use insar_store::{
    StoreFileIdentity, StoreLandedArtifact, StoreLandingEvidence, StoreLocalReuseDisposition,
    StoreTaskItemLocalReuse,
};

use super::{
    conflict_stamp, decide_local_reuse, quarantine_conflict, LedgerEvidence, LocalReuseDecision,
    LocalReuseRequest, ProbeFormat, ReuseGrade,
};
use crate::checksum::ChecksumAlgorithm;
use crate::file_identity::{file_status, FileStatus, IdentityStrength};
use crate::scheduler::{timestamp_now, LocalReuseAuthorityError, LocalReuseRecord};
use crate::{
    AuthRefreshPolicyId, DownloadWork, EligibleResolverAsset, ExpectedChecksum,
    NisarExecutionPlanFreezeAuthority, WorkerControl,
};

/// The frozen size a reuse decision may treat as authoritative. The burst
/// extractor's TIFF and XML catalogue sizes are the raw burst lengths, not
/// the delivered files; the transfer treats them as advisory as well.
pub(crate) fn authoritative_reuse_size(eligible: &EligibleResolverAsset) -> Option<u64> {
    let advisory = eligible.policy.auth_refresh_policy_id
        == AuthRefreshPolicyId::EarthdataBurstSessionV1
        && matches!(
            eligible.frozen_access.role,
            AssetRole::Primary | AssetRole::Metadata
        );
    if advisory {
        None
    } else {
        eligible.frozen_access.expected_size_bytes
    }
}

/// What the gate leaves for the transfer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GatePassage {
    /// The user asked to download the item again.
    pub(crate) reacquire: bool,
    /// An existing file was renamed aside as a conflict.
    pub(crate) renamed: bool,
    /// The file is reusable by its evidence, but the asset's policy-bound
    /// validator must accept it first: the validated route decides.
    pub(crate) deferred_reuse: Option<ReuseGrade>,
}

/// One planned destination and what is known about its file.
pub(crate) struct GateRequest<'a> {
    pub(crate) work: &'a DownloadWork,
    pub(crate) destination: &'a Path,
    pub(crate) file_name: &'a str,
    /// The size the Provider froze or reported; None when unknown.
    pub(crate) authoritative_size: Option<u64>,
    pub(crate) checksum: Option<&'a ExpectedChecksum>,
    /// The asset has an artifact validation plan (a policy-bound structure
    /// validator, e.g. a complete NISAR HDF5): no reuse without it.
    pub(crate) reuse_needs_validation: bool,
    /// The Scheduler-owned Store authority; None without a Scheduler.
    pub(crate) store: Option<&'a NisarExecutionPlanFreezeAuthority>,
    pub(crate) control: &'a WorkerControl,
}

const LOCAL_TAIL: &str = " [stage=local_reuse source=local]";

fn local_failure(
    request: &GateRequest<'_>,
    code: &str,
    reason: &str,
    retryable: bool,
) -> DownloadItemResult {
    DownloadItemResult {
        task_id: request.work.task.task_id.clone(),
        item_id: request.work.item.item_id.clone(),
        outcome: DownloadOutcome::Failed,
        destination: request.destination.to_path_buf(),
        bytes_written: 0,
        attempt_count: 0,
        failure: Some(DownloadFailure {
            code: code.to_owned(),
            reason_zh: format!("{reason}（本地检查，未联网）{LOCAL_TAIL}"),
            retryable,
        }),
        finished_at: timestamp_now(),
    }
}

fn stopped(request: &GateRequest<'_>) -> DownloadItemResult {
    DownloadItemResult {
        task_id: request.work.task.task_id.clone(),
        item_id: request.work.item.item_id.clone(),
        outcome: if request.control.is_cancelled() {
            DownloadOutcome::Cancelled
        } else {
            DownloadOutcome::Interrupted
        },
        destination: request.destination.to_path_buf(),
        bytes_written: 0,
        attempt_count: 0,
        failure: None,
        finished_at: timestamp_now(),
    }
}

/// The SAFE product name a Sentinel-1 ZIP must hold its manifest under.
fn zip_product(file_name: &str) -> Option<&str> {
    let split = file_name.len().checked_sub(4)?;
    (file_name.is_char_boundary(split) && file_name[split..].eq_ignore_ascii_case(".zip"))
        .then(|| &file_name[..split])
}

fn ledger_evidence(row: &StoreLandedArtifact) -> LedgerEvidence {
    LedgerEvidence {
        platform: row.identity.platform.clone(),
        volume_id: row.identity.volume_id,
        file_id: row.identity.file_id,
        size: row.size_bytes,
        modified: row.identity.modified,
        changed: row.identity.changed,
        stable: row.identity.stable,
        sha256: row.sha256.clone(),
        service_checksum: row
            .service_checksum
            .as_ref()
            .and_then(|(algorithm, value)| {
                ChecksumAlgorithm::parse(algorithm)
                    .ok()
                    .map(|algorithm| (algorithm, value.clone()))
            }),
    }
}

fn landed_artifact(
    request: &GateRequest<'_>,
    status: &FileStatus,
    sha256: Option<String>,
    evidence: StoreLandingEvidence,
    verified_at: &str,
) -> StoreLandedArtifact {
    StoreLandedArtifact {
        path_key: request.destination.to_string_lossy().into_owned(),
        provider_id: request.work.task.provider_id.clone(),
        product_id: request.work.item.item_id.clone(),
        size_bytes: status.size,
        sha256,
        service_checksum: request.checksum.map(|checksum| {
            (
                checksum.algorithm().name().to_owned(),
                checksum.value().to_owned(),
            )
        }),
        identity: StoreFileIdentity {
            platform: status.platform.to_owned(),
            volume_id: status.volume_id,
            file_id: status.file_id,
            modified: status.modified,
            changed: status.changed,
            stable: status.strength == IdentityStrength::Stable,
        },
        evidence,
        source_task_id: Some(request.work.task.task_id.clone()),
        source_item_id: Some(request.work.item.item_id.clone()),
        verified_at: verified_at.to_owned(),
    }
}

fn decision_row(
    request: &GateRequest<'_>,
    disposition: StoreLocalReuseDisposition,
    size_bytes: Option<u64>,
    decided_at: &str,
) -> StoreTaskItemLocalReuse {
    StoreTaskItemLocalReuse {
        task_id: request.work.task.task_id.clone(),
        item_id: request.work.item.item_id.clone(),
        disposition,
        size_bytes,
        decided_at: decided_at.to_owned(),
    }
}

fn record(request: &GateRequest<'_>, record: LocalReuseRecord) {
    // The facts are about files already decided on disk; a Store that cannot
    // take them costs only the annotation or a later ledger reuse.
    if let Some(store) = request.store {
        let _ = store.record_local_reuse(record);
    }
}

/// Decides the planned destination before any credential, resolution or
/// network admission. Err carries the item's finished result: skipped for a
/// reuse, stopped, or a local failure (source=local).
pub(crate) fn pass_local_reuse_gate(
    request: &GateRequest<'_>,
) -> Result<GatePassage, Box<DownloadItemResult>> {
    let task_id = &request.work.task.task_id;
    let item_id = &request.work.item.item_id;
    let path_key = request.destination.to_string_lossy().into_owned();
    let (ledger_row, prior) = match request
        .store
        .map(|store| store.load_local_reuse(&path_key, task_id, item_id, request.control))
    {
        Some(Ok(loaded)) => loaded,
        Some(Err(LocalReuseAuthorityError::ExecutionStopped)) => {
            return Err(Box::new(stopped(request)))
        }
        // Without the Store the decision loses only the ledger's evidence.
        Some(Err(_)) | None => (None, None),
    };
    let reacquire = prior
        .as_ref()
        .is_some_and(|prior| prior.disposition == StoreLocalReuseDisposition::ReacquireRequested);
    let ledger = ledger_row
        .filter(|row| {
            row.provider_id == request.work.task.provider_id && row.product_id == *item_id
        })
        .map(|row| ledger_evidence(&row));
    let stop = || request.control.is_cancelled() || request.control.is_paused();
    let decision = decide_local_reuse(&LocalReuseRequest {
        destination: request.destination,
        authoritative_size: request.authoritative_size,
        checksum: request.checksum,
        format: ProbeFormat::for_file_name(request.file_name),
        product: zip_product(request.file_name),
        ledger: ledger.as_ref(),
        allow_digest: true,
        stop: &stop,
    });
    let decided_at = timestamp_now();
    let reason = match decision {
        LocalReuseDecision::Missing => {
            return Ok(GatePassage {
                reacquire,
                ..GatePassage::default()
            })
        }
        // The validated route reuses a complete file only after its
        // policy-bound validator accepts it, and fails without retry when the
        // service's own bytes are structurally invalid: the gate must not
        // finish such an item on size, format or checksum alone.
        LocalReuseDecision::Reuse(reuse) if !reacquire && request.reuse_needs_validation => {
            return Ok(GatePassage {
                reacquire,
                deferred_reuse: Some(reuse.grade),
                ..GatePassage::default()
            })
        }
        LocalReuseDecision::Reuse(reuse) if !reacquire => {
            let landed = match reuse.grade {
                ReuseGrade::ServiceChecksum => Some(StoreLandingEvidence::ServiceChecksum),
                ReuseGrade::LedgerRehash => Some(StoreLandingEvidence::LedgerRehash),
                ReuseGrade::LedgerIdentity | ReuseGrade::SizeAndFormat => None,
            }
            .map(|evidence| {
                landed_artifact(
                    request,
                    &reuse.status,
                    reuse.sha256.clone(),
                    evidence,
                    &decided_at,
                )
            });
            record(
                request,
                LocalReuseRecord {
                    landed,
                    decision: Some(decision_row(
                        request,
                        StoreLocalReuseDisposition::Reused {
                            grade: reuse.grade.as_str().to_owned(),
                        },
                        Some(reuse.status.size),
                        &decided_at,
                    )),
                    ..LocalReuseRecord::default()
                },
            );
            return Err(Box::new(DownloadItemResult {
                task_id: task_id.clone(),
                item_id: item_id.clone(),
                outcome: DownloadOutcome::Skipped,
                destination: request.destination.to_path_buf(),
                bytes_written: reuse.status.size,
                attempt_count: 0,
                failure: None,
                finished_at: decided_at,
            }));
        }
        LocalReuseDecision::Reuse(_) => "reacquire",
        LocalReuseDecision::Conflict(reason) => reason.as_str(),
        LocalReuseDecision::Unsafe => {
            return Err(Box::new(local_failure(
                request,
                "LOCAL_REUSE_TARGET_UNSAFE",
                "目标位置已是链接、目录或多重硬链接文件，未改动；请处理后重试",
                false,
            )))
        }
        LocalReuseDecision::Stopped => return Err(Box::new(stopped(request))),
        // The gate always allows a digest; nothing to decide without one.
        LocalReuseDecision::NeedsDigest => {
            return Ok(GatePassage {
                reacquire,
                ..GatePassage::default()
            })
        }
        LocalReuseDecision::ReadFailed => {
            return Err(Box::new(local_failure(
                request,
                "LOCAL_REUSE_VERIFY_IO",
                "读取已有文件失败，未改动",
                true,
            )))
        }
        LocalReuseDecision::OverBudget => {
            return Err(Box::new(local_failure(
                request,
                "LOCAL_REUSE_PROBE_TIMEOUT",
                "已有文件的格式检查超出预算，未改动",
                true,
            )))
        }
    };
    // Owner rule 2026-10-20: a conflicting file is neither deleted nor
    // overwritten; it keeps its content under a conflict name.
    let renamed = match quarantine_conflict(request.destination, &conflict_stamp(&decided_at)) {
        Ok(renamed) => renamed,
        Err(_) => {
            return Err(Box::new(local_failure(
                request,
                "LOCAL_REUSE_QUARANTINE_FAILED",
                "已有文件无法改名另存（可能被占用或无权限），未改动",
                true,
            )))
        }
    };
    let conflict_file_name = renamed
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    record(
        request,
        LocalReuseRecord {
            invalidate: Some((path_key, "quarantined".to_owned())),
            decision: Some(decision_row(
                request,
                StoreLocalReuseDisposition::RenamedConflict {
                    reason: reason.to_owned(),
                    conflict_file_name,
                },
                None,
                &decided_at,
            )),
            ..LocalReuseRecord::default()
        },
    );
    Ok(GatePassage {
        reacquire,
        renamed: true,
        ..GatePassage::default()
    })
}

/// Records a reuse the validated route confirmed (the item finished skipped
/// after its policy-bound validator accepted the existing file).
pub(crate) fn record_deferred_reuse(
    request: &GateRequest<'_>,
    passage: GatePassage,
    size_bytes: u64,
) {
    let Some(grade) = passage.deferred_reuse else {
        return;
    };
    if request.store.is_none() {
        return;
    }
    let decided_at = timestamp_now();
    let landed = match grade {
        ReuseGrade::ServiceChecksum => Some(StoreLandingEvidence::ServiceChecksum),
        ReuseGrade::LedgerRehash => Some(StoreLandingEvidence::LedgerRehash),
        ReuseGrade::LedgerIdentity | ReuseGrade::SizeAndFormat => None,
    }
    .and_then(|evidence| {
        let file = std::fs::File::open(crate::long_path::native_path(request.destination)).ok()?;
        let status = file_status(&file).ok()?;
        Some(landed_artifact(
            request,
            &status,
            None,
            evidence,
            &decided_at,
        ))
    });
    record(
        request,
        LocalReuseRecord {
            landed,
            decision: Some(decision_row(
                request,
                StoreLocalReuseDisposition::Reused {
                    grade: grade.as_str().to_owned(),
                },
                Some(size_bytes),
                &decided_at,
            )),
            ..LocalReuseRecord::default()
        },
    );
}

/// Downloader D3-2: the pre-scan's local decision for one queued item. The
/// scan renames, records and transfers nothing; only a reuse it can finish
/// is reported, everything else is left to the worker's gate.
#[derive(Debug)]
pub(crate) enum ScanReuse {
    /// The worker decides (missing, conflict, unsafe, reacquire, validated
    /// asset, unreadable...).
    Release,
    /// A digest is needed and the request does not allow one.
    NeedsDigest,
    Reuse(Box<insar_store::StoreLocalReuseBatchItem>),
    Stopped,
}

pub(crate) fn scan_local_reuse(request: &GateRequest<'_>, allow_digest: bool) -> ScanReuse {
    let task_id = &request.work.task.task_id;
    let item_id = &request.work.item.item_id;
    let path_key = request.destination.to_string_lossy().into_owned();
    let (ledger_row, prior) = match request
        .store
        .map(|store| store.load_local_reuse(&path_key, task_id, item_id, request.control))
    {
        Some(Ok(loaded)) => loaded,
        Some(Err(LocalReuseAuthorityError::ExecutionStopped)) => return ScanReuse::Stopped,
        Some(Err(_)) | None => (None, None),
    };
    if prior
        .is_some_and(|prior| prior.disposition == StoreLocalReuseDisposition::ReacquireRequested)
    {
        return ScanReuse::Release;
    }
    let ledger = ledger_row
        .filter(|row| {
            row.provider_id == request.work.task.provider_id && row.product_id == *item_id
        })
        .map(|row| ledger_evidence(&row));
    let stop = || request.control.is_cancelled() || request.control.is_paused();
    match decide_local_reuse(&LocalReuseRequest {
        destination: request.destination,
        authoritative_size: request.authoritative_size,
        checksum: request.checksum,
        format: ProbeFormat::for_file_name(request.file_name),
        product: zip_product(request.file_name),
        ledger: ledger.as_ref(),
        allow_digest,
        stop: &stop,
    }) {
        // Owner rule: an asset with a policy-bound validator is reused only
        // through that validator, which only the worker's route runs.
        LocalReuseDecision::Reuse(_) if request.reuse_needs_validation => ScanReuse::Release,
        LocalReuseDecision::Reuse(reuse) => {
            let verified_at = timestamp_now();
            let landed = match reuse.grade {
                ReuseGrade::ServiceChecksum => Some(StoreLandingEvidence::ServiceChecksum),
                ReuseGrade::LedgerRehash => Some(StoreLandingEvidence::LedgerRehash),
                ReuseGrade::LedgerIdentity | ReuseGrade::SizeAndFormat => None,
            }
            .map(|evidence| {
                landed_artifact(
                    request,
                    &reuse.status,
                    reuse.sha256.clone(),
                    evidence,
                    &verified_at,
                )
            });
            ScanReuse::Reuse(Box::new(insar_store::StoreLocalReuseBatchItem {
                item_id: item_id.clone(),
                destination: request.destination.to_path_buf(),
                size_bytes: reuse.status.size,
                grade: reuse.grade.as_str().to_owned(),
                landed,
            }))
        }
        LocalReuseDecision::NeedsDigest => ScanReuse::NeedsDigest,
        LocalReuseDecision::Stopped => ScanReuse::Stopped,
        LocalReuseDecision::Missing
        | LocalReuseDecision::Conflict(_)
        | LocalReuseDecision::Unsafe
        | LocalReuseDecision::ReadFailed
        | LocalReuseDecision::OverBudget => ScanReuse::Release,
    }
}

/// Records the verified landing of the planned file in the Store ledger:
/// its identity, size, SHA-256 when the transfer computed it, and the service
/// checksum the transfer verified. A reacquisition without a renamed file
/// leaves no annotation behind.
pub(crate) fn record_local_landing(
    request: &GateRequest<'_>,
    passage: GatePassage,
    sha256: Option<String>,
) {
    if request.store.is_none() {
        return;
    }
    let Ok(file) = std::fs::File::open(crate::long_path::native_path(request.destination)) else {
        return;
    };
    let Ok(status) = file_status(&file) else {
        return;
    };
    drop(file);
    let landed = landed_artifact(
        request,
        &status,
        sha256,
        StoreLandingEvidence::Landing,
        &timestamp_now(),
    );
    let clear_decision = (passage.reacquire && !passage.renamed).then(|| {
        (
            request.work.task.task_id.clone(),
            request.work.item.item_id.clone(),
        )
    });
    record(
        request,
        LocalReuseRecord {
            landed: Some(landed),
            clear_decision,
            ..LocalReuseRecord::default()
        },
    );
}
