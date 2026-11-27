//! Store-authoritative V2 science Range transfer and recovery.

use std::cell::Cell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use insar_core::{
    NisarSubsetChecksumAlgorithm, NisarSubsetSourceAssetIdentityV2,
    NisarSubsetSourceProductIdentityV2, NisarSubsetSourceValidatorV1, NisarSubsetValidatorKind,
};
use insar_provider::bhoonidhi::{BHOONIDHI_COLLECTION_ID, BHOONIDHI_PROVIDER_ID};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::asset_resolver::{
    ResolvedHostPolicy, BHOONIDHI_DOWNLOAD_ORIGIN, BHOONIDHI_DOWNLOAD_PATH,
};
use crate::hdf5_range_transfer::{
    response_strong_etag, science_allocated_range_evidence_v2, validate_content_range,
    validate_science_allocated_range_evidence_v2, Hdf5RangeTransferError, Hdf5RangeWorkspaceV1,
    Hdf5WorkspaceLockV1, ImmutableSciencePartForWriterV2, NisarScienceAllocatedRangeEvidenceV2,
    NisarScienceAllocationEvidenceErrorV2,
};
use crate::http_transfer::ExactRangeSendOutcome;
use crate::nisar_planning_v2::{
    revalidate_science_authority_v2, revalidate_science_range_binding_v2,
    NisarSubsetBootstrapStatisticsV2, NisarSubsetCombinedBudgetProjectionV2,
    NisarSubsetMetadataBootstrapV2, NisarSubsetPhysicalExecutionPlanV2, NisarSubsetPhysicalRangeV2,
    NisarSubsetPlanningCompleteV2, NisarSubsetPlanningErrorV2,
};
use crate::{DownloadWork, HttpTransfer, ResolvedAsset, ResolvedMethod, WorkerControl};

const SCIENCE_CHECKPOINT_CONTRACT_V2: &str = "NisarSubsetScienceCheckpointV2";
const SCIENCE_CHECKPOINT_SCHEMA_VERSION_V2: u32 = 1;
const SCIENCE_READ_BUFFER_BYTES: usize = 64 * 1024;
// One initial send and at most one authenticated retry, each with bounded redirects.
const MAX_SCIENCE_HTTP_REQUESTS_PER_ATTEMPT: u64 =
    2 * crate::http_transfer::MAX_REQUESTS_PER_TRUSTED_REDIRECT_SEND;
const SCIENCE_CHECKPOINT_MAX_CANONICAL_BYTES: usize = 16 * 1024 * 1024;
const SCIENCE_ANCHOR_MAX_CANONICAL_BYTES: usize = 40 * 1024 * 1024;
const NISAR_SUBSET_COMMITTED_RECOVERY_TELEMETRY_CONTRACT_V1: &str =
    "nisar_subset_committed_recovery_telemetry_v1";
const SCIENCE_ANCHOR_RECOVERY_TELEMETRY_DIGEST_CONTRACT_V1: &str =
    "nisar_subset_science_checkpoint_anchor_record_v2_recovery_telemetry_v1";

#[path = "nisar_budget_upgrade_v2.rs"]
mod budget_upgrade;
#[cfg(test)]
pub(crate) use budget_upgrade::NisarBudgetMigrationPhaseV2;
pub(crate) use budget_upgrade::{prepare_nisar_budget_upgrade_v2, NisarBudgetMigrationFailureV2};
use budget_upgrade::{
    NisarScienceBudgetUpgradeReceiptV2, NisarValidatedScienceBudgetV2, ScienceRecoveryBudgetModeV2,
};

pub(crate) struct NisarArchivedRecoveryPreparedV2 {
    source_part: File,
    source_workspace: Hdf5RangeWorkspaceV1,
    source_mutation_marker: String,
    source_checkpoint: NisarSubsetScienceCheckpointV2,
    planning: crate::nisar_planning_v2::NisarArchivedPlanningExportV2,
    target_task_id: String,
    target_item_id: String,
    target_path: String,
    target_identity: String,
    _target_lock: Hdf5WorkspaceLockV1,
}

fn archived_import_active(
    control: &WorkerControl,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    if control.is_cancelled() {
        return Err(NisarSubsetScienceTransferErrorV2::Cancelled);
    }
    if control.is_paused() {
        return Err(NisarSubsetScienceTransferErrorV2::Paused);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_archived_nisar_recovery_v2(
    source_work: &DownloadWork,
    new_work: &DownloadWork,
    source_anchor_json: &str,
    expected_source_anchor_sha256: &str,
    source_workspace: &Hdf5RangeWorkspaceV1,
    dest_workspace: &Hdf5RangeWorkspaceV1,
    control: &WorkerControl,
) -> Result<NisarArchivedRecoveryPreparedV2, NisarSubsetScienceTransferErrorV2> {
    // Other platforms need an immutable read view that creates no historical files.
    if !cfg!(windows) {
        return Err(NisarSubsetScienceTransferErrorV2::AllocationEvidenceUnsupported);
    }
    archived_import_active(control)?;
    let record =
        NisarSubsetScienceCheckpointAnchorRecordV2::from_canonical_json(source_anchor_json)?;
    if record.record_sha256 != expected_source_anchor_sha256
        || record.pending_transition.is_some()
        || source_work.task.task_id == new_work.task.task_id
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let (source_path, source_identity) = source_workspace
        .bound_workspace_identity_v2()
        .map_err(map_workspace_error)?;
    let (target_path, target_identity) = dest_workspace
        .bound_workspace_identity_v2()
        .map_err(map_workspace_error)?;
    if source_path == target_path || source_identity == target_identity {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let text = source_workspace
        .load_science_checkpoint_v2()
        .map_err(map_workspace_error)?
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    let checkpoint = parse_checkpoint_canonical_json(&text)?;
    if checkpoint.pending_attempt.is_some()
        || checkpoint.state == NisarSubsetScienceCheckpointStateV2::Complete
        || record.committed_checkpoint_sha256.as_deref() != Some(&checkpoint.checkpoint_sha256)
        || record.generation != checkpoint.anchor_generation
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let (mut source_part, source_part_identity, source_mutation_marker) = source_workspace
        .reopen_science_part_for_writer_v2(checkpoint.source_size)
        .map_err(map_workspace_error)?;
    let planning = crate::nisar_planning_v2::export_archived_planning_v2(
        source_work,
        source_workspace,
        control,
    )
    .map_err(|error| map_planning_error(error, control))?;
    let lineage = lineage_key_sha256(source_work, planning.physical_plan())?;
    if record.lineage_key_sha256 != lineage {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    validate_recovery_checkpoint(
        &checkpoint,
        source_work,
        planning.physical_plan(),
        &source_path,
        &source_identity,
        &lineage,
        &record.execution_binding_sha256,
        &source_part_identity,
        &mut source_part,
    )?;
    let lock = dest_workspace.acquire_lock().map_err(map_workspace_error)?;
    // The marker is an immutable local import identity, not an alternative authority.
    let marker = serde_json::to_string(&(
        "NisarArchivedImportV2",
        &source_work.task.task_id,
        &source_work.item.item_id,
        expected_source_anchor_sha256,
        planning.checkpoint_sha256(),
        &new_work.task.task_id,
        &new_work.item.item_id,
        &target_path,
        &target_identity,
    ))
    .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    match dest_workspace
        .load_archived_import_marker_v2()
        .map_err(map_workspace_error)?
    {
        Some(existing) if existing != marker => {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
        }
        Some(_) => {}
        None => {
            if std::fs::symlink_metadata(dest_workspace.part_path()).is_ok()
                || dest_workspace
                    .load_planning_checkpoint_v2()
                    .map_err(map_workspace_error)?
                    .is_some()
                || dest_workspace
                    .load_planning_prefix_checkpoint_v2()
                    .map_err(map_workspace_error)?
                    .is_some()
                || dest_workspace
                    .load_science_checkpoint_v2()
                    .map_err(map_workspace_error)?
                    .is_some()
            {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
            dest_workspace
                .persist_archived_import_marker_v2(&marker, &lock)
                .map_err(map_workspace_error)?;
        }
    }
    let planning_exists = dest_workspace
        .load_planning_checkpoint_v2()
        .map_err(map_workspace_error)?
        .is_some();
    if !planning_exists {
        if dest_workspace
            .load_science_checkpoint_v2()
            .map_err(map_workspace_error)?
            .is_some()
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let mut part = dest_workspace
            .open_archived_import_part_v2(planning.source_size(), &lock)
            .map_err(map_workspace_error)?;
        let mut ranges = planning.material_ranges();
        ranges.extend(
            checkpoint
                .completed_ranges
                .iter()
                .map(|range| (range.start, range.end_exclusive)),
        );
        ranges.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for (start, end) in ranges {
            if start >= end || end > planning.source_size() {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
            if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        let mut buffer = [0_u8; SCIENCE_READ_BUFFER_BYTES];
        for (start, end) in merged {
            source_part
                .seek(SeekFrom::Start(start))
                .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
            part.seek(SeekFrom::Start(start))
                .map_err(|_| NisarSubsetScienceTransferErrorV2::Persistence)?;
            let mut remaining = end - start;
            while remaining > 0 {
                archived_import_active(control)?;
                let count = remaining.min(buffer.len() as u64) as usize;
                source_part
                    .read_exact(&mut buffer[..count])
                    .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
                part.write_all(&buffer[..count])
                    .map_err(|_| NisarSubsetScienceTransferErrorV2::Persistence)?;
                remaining -= count as u64;
            }
        }
        part.sync_all()
            .map_err(|_| NisarSubsetScienceTransferErrorV2::Persistence)?;
        drop(part);
    }
    crate::nisar_planning_v2::rebind_archived_planning_v2(
        &planning,
        new_work,
        dest_workspace,
        control,
        &lock,
    )
    .map_err(|error| map_planning_error(error, control))?;
    if source_mutation_marker
        != source_workspace
            .science_part_mutation_marker_for_writer_v2(&source_part)
            .map_err(map_workspace_error)?
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    Ok(NisarArchivedRecoveryPreparedV2 {
        source_part,
        source_workspace: source_workspace.clone(),
        source_mutation_marker,
        source_checkpoint: checkpoint,
        planning,
        target_task_id: new_work.task.task_id.clone(),
        target_item_id: new_work.item.item_id.clone(),
        target_path,
        target_identity,
        _target_lock: lock,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_archived_nisar_recovery_v2(
    prepared: NisarArchivedRecoveryPreparedV2,
    new_work: &DownloadWork,
    completed: &NisarSubsetPlanningCompleteV2,
    remote: &NisarSubsetScienceRemoteSourceV2,
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    dest_workspace: &Hdf5RangeWorkspaceV1,
    control: &WorkerControl,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    archived_import_active(control)?;
    let (path, identity) = dest_workspace
        .bound_workspace_identity_v2()
        .map_err(map_workspace_error)?;
    if new_work.task.task_id != prepared.target_task_id
        || new_work.item.item_id != prepared.target_item_id
        || path != prepared.target_path
        || identity != prepared.target_identity
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let current =
        crate::nisar_planning_v2::export_archived_planning_v2(new_work, dest_workspace, control)
            .map_err(|error| map_planning_error(error, control))?;
    let plan = current.physical_plan();
    if plan != prepared.planning.physical_plan() || plan != completed.physical_execution_plan() {
        return Err(NisarSubsetScienceTransferErrorV2::AuthorityMismatch);
    }
    remote.validate(new_work, completed, plan)?;
    let lineage = lineage_key_sha256(new_work, plan)?;
    let binding = execution_binding_sha256(&lineage, plan, remote, &path, &identity)?;
    let (mut part, part_identity) = dest_workspace
        .open_science_part_v2(plan.source_size())
        .map_err(map_workspace_error)?;
    let mut imported = prepared.source_checkpoint.clone();
    imported.task_id = new_work.task.task_id.clone();
    imported.item_id = new_work.item.item_id.clone();
    imported.workspace_canonical_path = path.clone();
    imported.workspace_identity = identity.clone();
    imported.part_identity_sha256 = part_identity.clone();
    imported.lineage_key_sha256 = lineage.clone();
    imported.execution_binding_sha256 = binding.clone();
    imported.anchor_generation = 0;
    imported.previous_checkpoint_sha256 = None;
    imported.state = NisarSubsetScienceCheckpointStateV2::Interrupted;
    for range in &mut imported.completed_ranges {
        range.allocated_range_evidence =
            science_allocated_range_evidence_v2(&mut part, range.start, range.end_exclusive)
                .map_err(map_allocation_error)?;
    }
    imported.checkpoint_sha256 = imported.recompute_sha256()?;
    validate_recovery_checkpoint(
        &imported,
        new_work,
        plan,
        &path,
        &identity,
        &lineage,
        &binding,
        &part_identity,
        &mut part,
    )?;
    if prepared.source_mutation_marker
        != prepared
            .source_workspace
            .science_part_mutation_marker_for_writer_v2(&prepared.source_part)
            .map_err(map_workspace_error)?
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let local = dest_workspace
        .load_science_checkpoint_v2()
        .map_err(map_workspace_error)?
        .map(|text| parse_checkpoint_canonical_json(&text))
        .transpose()?;
    if let Some(record) = anchor.load(&lineage)? {
        // Reuse durable CAS recovery; never replace later target progress with the export.
        record.validate()?;
        let candidate = match &record.pending_transition {
            Some(pending) => parse_checkpoint_canonical_json(&pending.checkpoint_canonical_json)?,
            None => local
                .clone()
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?,
        };
        inherited_science_is_retained(&imported, &candidate)?;
        recover_or_initialize_checkpoint(
            anchor,
            &lineage,
            &binding,
            dest_workspace,
            local,
            new_work,
            plan,
            &path,
            &identity,
            &part_identity,
            &mut part,
            ScienceRecoveryBudgetModeV2::Effective,
            &digest_runs_to_the_end,
        )?;
    } else {
        if local.is_some() {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let genesis = NisarSubsetScienceCheckpointAnchorRecordV2::prepared_genesis(
            &lineage, &binding, &imported,
        )?;
        if !anchor.compare_and_swap(&lineage, None, &genesis)? {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        persist_checkpoint(dest_workspace, &imported)?;
        let committed = genesis.commit_pending()?;
        if !anchor.compare_and_swap(&lineage, Some(&genesis), &committed)? {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
    }
    Ok(())
}

fn inherited_science_is_retained(
    source: &NisarSubsetScienceCheckpointV2,
    current: &NisarSubsetScienceCheckpointV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    let counters = |c: &NisarSubsetScienceCheckpointV2| {
        [
            c.reserved_science_request_count,
            c.reserved_science_response_body_bytes,
            c.issued_science_request_count,
            c.observed_science_response_body_bytes,
            c.failed_science_request_count,
            c.unissued_science_reservation_count,
            c.indeterminate_science_request_count,
            c.indeterminate_science_response_body_bytes,
            c.committed_science_request_count,
            c.committed_science_response_body_bytes,
        ]
    };
    if counters(source)
        .iter()
        .zip(counters(current))
        .any(|(old, new)| new < *old)
        || source.completed_ranges.iter().any(|old| {
            !current.completed_ranges.iter().any(|new| {
                new.index == old.index
                    && new.start == old.start
                    && new.end_exclusive == old.end_exclusive
                    && new.response_sha256 == old.response_sha256
                    && new.request_count == old.request_count
            })
        })
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum NisarSubsetScienceTransferErrorV2 {
    #[error("NISAR V2 server ignored the exact science Range request")]
    RangeUnsupported,
    #[error("NISAR V2 strong source validator changed")]
    StrongValidatorChanged,
    #[error("NISAR V2 science response is invalid")]
    ResponseInvalid,
    #[error("NISAR V2 science Content-Range or response length is invalid")]
    RangeResponseInvalid,
    #[error("NISAR V2 combined planning and science budget was exceeded")]
    CombinedBudgetExceeded,
    #[error("NISAR V2 science transfer is incomplete")]
    ScienceTransferIncomplete,
    #[error("NISAR V2 science recovery evidence is invalid")]
    RecoveryInvalid,
    #[error("exact science allocation evidence is unsupported")]
    AllocationEvidenceUnsupported,
    #[error("NISAR V2 science authority does not match the Store-frozen plan")]
    AuthorityMismatch,
    #[error("NISAR V2 science transfer is paused")]
    Paused,
    #[error("NISAR V2 science transfer is cancelled")]
    Cancelled,
    #[error("NISAR V2 science Range transport failed")]
    Transport,
    #[error("NISAR V2 science Range authentication refresh failed")]
    AuthRefreshFailed,
    /// A refresh that failed for a transient reason (timeout, transport, a
    /// temporarily unavailable post-refresh probe); the generation may still
    /// be refreshed, so the Range may be retried. Same stable code.
    #[error("NISAR V2 science Range authentication refresh failed temporarily")]
    AuthRefreshTransient,
    #[error("NISAR V2 science checkpoint could not be persisted")]
    Persistence,
    /// The local task Store failed for a storage reason that freeing disk
    /// space or repairing the disk clears (review 2026-10-17 round 3, P2-2): a
    /// full disk, an I/O error, a database file it cannot open or a
    /// file-system error. Transient like `Persistence`, with the same stable
    /// code but its own local diagnostic (`local_storage_failure_reason_zh`).
    #[error("NISAR V2 local task store storage failed ({store_code}, {cause})")]
    LocalStorageFailure {
        store_code: &'static str,
        cause: &'static str,
    },
    /// The task Store refused the operation for a reason that waiting does not
    /// change: an unsupported proof version, a corrupt or invalid record, a
    /// missing task or item, or another database refusal. Only a revision
    /// conflict, a busy database and an unavailable coordinator are transient
    /// (`Persistence`). The stable code is the Store's own code.
    #[error("NISAR V2 task store refused the execution record ({store_code})")]
    StoreRecordRejected { store_code: &'static str },
}

impl NisarSubsetScienceTransferErrorV2 {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::RangeUnsupported => "NISAR_SUBSET_V2_RANGE_UNSUPPORTED",
            Self::StrongValidatorChanged => "NISAR_SUBSET_V2_VALIDATOR_CHANGED",
            Self::ResponseInvalid | Self::RangeResponseInvalid => {
                "NISAR_SUBSET_V2_RANGE_RESPONSE_INVALID"
            }
            Self::CombinedBudgetExceeded => "NISAR_SUBSET_V2_COMBINED_BUDGET_EXCEEDED",
            Self::ScienceTransferIncomplete | Self::Paused | Self::Cancelled | Self::Transport => {
                "NISAR_SUBSET_V2_SCIENCE_INCOMPLETE"
            }
            Self::AuthRefreshFailed | Self::AuthRefreshTransient => {
                "NISAR_SUBSET_V2_AUTH_REFRESH_FAILED"
            }
            Self::RecoveryInvalid => "NISAR_SUBSET_V2_RECOVERY_INVALID",
            Self::AllocationEvidenceUnsupported => {
                "NISAR_SUBSET_V2_ALLOCATION_EVIDENCE_UNSUPPORTED"
            }
            Self::AuthorityMismatch => "NISAR_SUBSET_V2_AUTHORITY_MISMATCH",
            Self::Persistence | Self::LocalStorageFailure { .. } => {
                "NISAR_SUBSET_V2_CHECKPOINT_PERSISTENCE"
            }
            Self::StoreRecordRejected { store_code } => store_code,
        }
    }
}

/// Review 2026-10-17 round 3, P2-2: the local diagnostic of a
/// `LocalStorageFailure`: the local cause, what `context_zh` means at the
/// caller's stage, that downloaded material is kept and how to continue, and a
/// tail naming the stage, the Store code and the cause. `None` for every other
/// error.
pub(crate) fn local_storage_failure_reason_zh(
    error: NisarSubsetScienceTransferErrorV2,
    stage: &str,
    context_zh: &str,
) -> Option<String> {
    let NisarSubsetScienceTransferErrorV2::LocalStorageFailure { store_code, cause } = error else {
        return None;
    };
    Some(format!(
        "本地磁盘空间不足或文件读写失败，{context_zh}已下载的片段与断点记录保留在恢复工作区。\
         释放磁盘空间或检查磁盘后可继续。\
         [stage={stage} source=local store_error={store_code} cause={cause}]"
    ))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetScienceRemoteSourceV2 {
    effective_url: String,
    source_product_identity: NisarSubsetSourceProductIdentityV2,
    source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    source_validator: NisarSubsetSourceValidatorV1,
    source_size: u64,
    frozen_authority_sha256: String,
    identity_sha256: String,
}

impl NisarSubsetScienceRemoteSourceV2 {
    pub(crate) fn from_frozen_authority(
        effective_url: &Url,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        plan: &NisarSubsetPhysicalExecutionPlanV2,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        if completed.physical_execution_plan() != plan
            || !matches!(effective_url.scheme(), "http" | "https")
            || effective_url.has_authority() && effective_url.host_str().is_none()
            || !effective_url.username().is_empty()
            || effective_url.password().is_some()
            || effective_url.fragment().is_some()
        {
            return Err(NisarSubsetScienceTransferErrorV2::AuthorityMismatch);
        }
        let frozen_authority_sha256 = frozen_authority_sha256(work, plan)?;
        let mut source = Self {
            effective_url: effective_url.as_str().to_owned(),
            source_product_identity: plan.source_product_identity().clone(),
            source_asset_identity: plan.source_asset_identity().clone(),
            source_validator: plan.source_validator().clone(),
            source_size: plan.source_size(),
            frozen_authority_sha256,
            identity_sha256: String::new(),
        };
        source.identity_sha256 = source.recompute_identity_sha256()?;
        Ok(source)
    }

    fn recompute_identity_sha256(&self) -> Result<String, NisarSubsetScienceTransferErrorV2> {
        digest_serializable(&(
            &self.effective_url,
            &self.source_product_identity,
            &self.source_asset_identity,
            &self.source_validator,
            self.source_size,
            &self.frozen_authority_sha256,
        ))
    }

    fn validate(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        plan: &NisarSubsetPhysicalExecutionPlanV2,
    ) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        if completed.physical_execution_plan() != plan
            || self.source_product_identity != *plan.source_product_identity()
            || self.source_asset_identity != *plan.source_asset_identity()
            || self.source_validator != *plan.source_validator()
            || self.source_size != plan.source_size()
            || self.frozen_authority_sha256 != frozen_authority_sha256(work, plan)?
            || !valid_sha256(&self.identity_sha256)
            || self.recompute_identity_sha256()? != self.identity_sha256
        {
            return Err(NisarSubsetScienceTransferErrorV2::AuthorityMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NisarSubsetSciencePendingAnchorTransitionV2 {
    predecessor_checkpoint_sha256: Option<String>,
    predecessor_generation: Option<u64>,
    checkpoint_generation: u64,
    checkpoint_sha256: String,
    checkpoint_canonical_json: String,
    transition_sha256: String,
}

impl NisarSubsetSciencePendingAnchorTransitionV2 {
    fn new(
        predecessor_checkpoint_sha256: Option<&str>,
        predecessor_generation: Option<u64>,
        checkpoint: &NisarSubsetScienceCheckpointV2,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        let checkpoint_canonical_json = checkpoint_canonical_json(checkpoint)?;
        let mut transition = Self {
            predecessor_checkpoint_sha256: predecessor_checkpoint_sha256.map(str::to_owned),
            predecessor_generation,
            checkpoint_generation: checkpoint.anchor_generation,
            checkpoint_sha256: checkpoint.checkpoint_sha256.clone(),
            checkpoint_canonical_json,
            transition_sha256: String::new(),
        };
        transition.seal()?;
        Ok(transition)
    }

    fn seal(&mut self) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        self.transition_sha256 = digest_serializable(&(
            &self.predecessor_checkpoint_sha256,
            self.predecessor_generation,
            self.checkpoint_generation,
            &self.checkpoint_sha256,
            &self.checkpoint_canonical_json,
        ))?;
        Ok(())
    }

    fn validate(&self) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        if self
            .predecessor_checkpoint_sha256
            .as_deref()
            .is_some_and(|digest| !valid_sha256(digest))
            || self.predecessor_checkpoint_sha256.is_some() != self.predecessor_generation.is_some()
            || !valid_sha256(&self.checkpoint_sha256)
            || self.checkpoint_canonical_json.is_empty()
            || self.checkpoint_canonical_json.len() > SCIENCE_CHECKPOINT_MAX_CANONICAL_BYTES
            || !valid_sha256(&self.transition_sha256)
            || digest_serializable(&(
                &self.predecessor_checkpoint_sha256,
                self.predecessor_generation,
                self.checkpoint_generation,
                &self.checkpoint_sha256,
                &self.checkpoint_canonical_json,
            ))? != self.transition_sha256
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NisarSubsetCommittedRecoveryTelemetryV1 {
    contract: String,
    checkpoint_sha256: String,
    reusable_verified_bytes: u64,
    reused_verified_bytes: u64,
}

impl NisarSubsetCommittedRecoveryTelemetryV1 {
    fn from_checkpoint(
        checkpoint: &NisarSubsetScienceCheckpointV2,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        let telemetry = Self {
            contract: NISAR_SUBSET_COMMITTED_RECOVERY_TELEMETRY_CONTRACT_V1.to_owned(),
            checkpoint_sha256: checkpoint.checkpoint_sha256.clone(),
            reusable_verified_bytes: checkpoint.committed_science_response_body_bytes,
            reused_verified_bytes: checkpoint.resumed_bytes,
        };
        telemetry.validate()?;
        Ok(telemetry)
    }

    fn validate(&self) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        if self.contract != NISAR_SUBSET_COMMITTED_RECOVERY_TELEMETRY_CONTRACT_V1
            || !valid_sha256(&self.checkpoint_sha256)
            || self.reused_verified_bytes > self.reusable_verified_bytes
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NisarSubsetScienceCheckpointAnchorRecordV2 {
    lineage_key_sha256: String,
    execution_binding_sha256: String,
    generation: u64,
    committed_checkpoint_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    committed_recovery_telemetry: Option<NisarSubsetCommittedRecoveryTelemetryV1>,
    pending_transition: Option<NisarSubsetSciencePendingAnchorTransitionV2>,
    record_sha256: String,
}

impl NisarSubsetScienceCheckpointAnchorRecordV2 {
    fn prepared_genesis(
        lineage_key_sha256: &str,
        execution_binding_sha256: &str,
        checkpoint: &NisarSubsetScienceCheckpointV2,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        if checkpoint.anchor_generation != 0 || checkpoint.previous_checkpoint_sha256.is_some() {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let mut record = Self {
            lineage_key_sha256: lineage_key_sha256.to_owned(),
            execution_binding_sha256: execution_binding_sha256.to_owned(),
            generation: 0,
            committed_checkpoint_sha256: None,
            committed_recovery_telemetry: None,
            pending_transition: Some(NisarSubsetSciencePendingAnchorTransitionV2::new(
                None, None, checkpoint,
            )?),
            record_sha256: String::new(),
        };
        record.seal()?;
        record.validate()?;
        Ok(record)
    }

    fn with_pending(
        &self,
        checkpoint: &NisarSubsetScienceCheckpointV2,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        self.validate()?;
        let committed = self
            .committed_checkpoint_sha256
            .as_deref()
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        let next_generation = self
            .generation
            .checked_add(1)
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        if self.pending_transition.is_some()
            || checkpoint.anchor_generation != next_generation
            || checkpoint.previous_checkpoint_sha256.as_deref() != Some(committed)
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let mut record = self.clone();
        record.pending_transition = Some(NisarSubsetSciencePendingAnchorTransitionV2::new(
            Some(committed),
            Some(self.generation),
            checkpoint,
        )?);
        record.seal()?;
        record.validate()?;
        Ok(record)
    }

    fn commit_pending(&self) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        self.validate()?;
        let pending = self
            .pending_transition
            .as_ref()
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        let checkpoint = parse_checkpoint_canonical_json(&pending.checkpoint_canonical_json)?;
        if checkpoint.checkpoint_sha256 != pending.checkpoint_sha256
            || checkpoint.anchor_generation != pending.checkpoint_generation
            || checkpoint.previous_checkpoint_sha256 != pending.predecessor_checkpoint_sha256
            || checkpoint.lineage_key_sha256 != self.lineage_key_sha256
            || checkpoint.execution_binding_sha256 != self.execution_binding_sha256
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let mut record = Self {
            lineage_key_sha256: self.lineage_key_sha256.clone(),
            execution_binding_sha256: self.execution_binding_sha256.clone(),
            generation: pending.checkpoint_generation,
            committed_checkpoint_sha256: Some(pending.checkpoint_sha256.clone()),
            committed_recovery_telemetry: Some(
                NisarSubsetCommittedRecoveryTelemetryV1::from_checkpoint(&checkpoint)?,
            ),
            pending_transition: None,
            record_sha256: String::new(),
        };
        record.seal()?;
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn from_canonical_json(
        canonical: &str,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        if canonical.is_empty() || canonical.len() > SCIENCE_ANCHOR_MAX_CANONICAL_BYTES {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let record: Self = serde_json::from_str(canonical)
            .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        record.validate()?;
        if record.canonical_json()? != canonical {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        Ok(record)
    }

    pub(crate) fn canonical_json(&self) -> Result<String, NisarSubsetScienceTransferErrorV2> {
        self.validate()?;
        let canonical = serde_json::to_string(self)
            .map_err(|_| NisarSubsetScienceTransferErrorV2::Persistence)?;
        if canonical.len() > SCIENCE_ANCHOR_MAX_CANONICAL_BYTES {
            return Err(NisarSubsetScienceTransferErrorV2::Persistence);
        }
        Ok(canonical)
    }

    pub(crate) fn record_sha256(&self) -> &str {
        &self.record_sha256
    }

    pub(crate) fn lineage_key_sha256(&self) -> &str {
        &self.lineage_key_sha256
    }

    pub(crate) fn execution_binding_sha256(&self) -> &str {
        &self.execution_binding_sha256
    }

    pub(crate) const fn has_pending_checkpoint(&self) -> bool {
        self.pending_transition.is_some()
    }

    pub(crate) fn pending_checkpoint_canonical_json(&self) -> Option<&str> {
        self.pending_transition
            .as_ref()
            .map(|pending| pending.checkpoint_canonical_json.as_str())
    }

    #[cfg(test)]
    pub(crate) fn corrupt_pending_payload_for_test(&mut self) {
        self.pending_transition
            .as_mut()
            .expect("pending transition")
            .checkpoint_canonical_json
            .push(' ');
    }

    #[cfg(test)]
    pub(crate) fn coherently_mismatch_pending_predecessor_for_test(&mut self) {
        let replacement = sha256(b"coherently mismatched pending predecessor");
        self.committed_checkpoint_sha256 = Some(replacement.clone());
        let pending = self
            .pending_transition
            .as_mut()
            .expect("pending transition");
        pending.predecessor_checkpoint_sha256 = Some(replacement);
        pending.seal().unwrap();
        self.seal().unwrap();
    }

    #[cfg(test)]
    pub(crate) fn coherently_set_pending_generation_max_for_test(&mut self) {
        let pending = self
            .pending_transition
            .as_mut()
            .expect("pending transition");
        let mut target =
            parse_checkpoint_canonical_json(&pending.checkpoint_canonical_json).unwrap();
        target.anchor_generation = u64::MAX;
        target.seal().unwrap();
        pending.predecessor_generation = Some(u64::MAX);
        pending.checkpoint_generation = u64::MAX;
        pending.checkpoint_sha256 = target.checkpoint_sha256.clone();
        pending.checkpoint_canonical_json = checkpoint_canonical_json(&target).unwrap();
        pending.seal().unwrap();
        self.generation = u64::MAX;
        self.seal().unwrap();
    }

    fn seal(&mut self) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        self.record_sha256 = match &self.committed_recovery_telemetry {
            Some(telemetry) => digest_serializable(&(
                SCIENCE_ANCHOR_RECOVERY_TELEMETRY_DIGEST_CONTRACT_V1,
                &self.lineage_key_sha256,
                &self.execution_binding_sha256,
                self.generation,
                &self.committed_checkpoint_sha256,
                telemetry,
                &self.pending_transition,
            ))?,
            None => digest_serializable(&(
                &self.lineage_key_sha256,
                &self.execution_binding_sha256,
                self.generation,
                &self.committed_checkpoint_sha256,
                &self.pending_transition,
            ))?,
        };
        Ok(())
    }

    fn validate(&self) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        let expected_record_sha256 = match &self.committed_recovery_telemetry {
            Some(telemetry) => digest_serializable(&(
                SCIENCE_ANCHOR_RECOVERY_TELEMETRY_DIGEST_CONTRACT_V1,
                &self.lineage_key_sha256,
                &self.execution_binding_sha256,
                self.generation,
                &self.committed_checkpoint_sha256,
                telemetry,
                &self.pending_transition,
            ))?,
            None => digest_serializable(&(
                &self.lineage_key_sha256,
                &self.execution_binding_sha256,
                self.generation,
                &self.committed_checkpoint_sha256,
                &self.pending_transition,
            ))?,
        };
        if !valid_sha256(&self.lineage_key_sha256)
            || !valid_sha256(&self.execution_binding_sha256)
            || self
                .committed_checkpoint_sha256
                .as_deref()
                .is_some_and(|digest| !valid_sha256(digest))
            || (!self.has_pending_checkpoint() && self.committed_checkpoint_sha256.is_none())
            || !valid_sha256(&self.record_sha256)
            || expected_record_sha256 != self.record_sha256
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        match (
            &self.committed_recovery_telemetry,
            self.committed_checkpoint_sha256.as_deref(),
        ) {
            (Some(telemetry), Some(committed)) => {
                telemetry.validate()?;
                if telemetry.checkpoint_sha256 != committed {
                    return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
                }
            }
            (Some(_), None) => {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
            (None, _) => {}
        }
        if let Some(pending) = &self.pending_transition {
            pending.validate()?;
            let predecessor_matches = match self.committed_checkpoint_sha256.as_deref() {
                Some(committed) => {
                    pending.predecessor_checkpoint_sha256.as_deref() == Some(committed)
                        && pending.predecessor_generation == Some(self.generation)
                        && self.generation.checked_add(1) == Some(pending.checkpoint_generation)
                }
                None => {
                    self.generation == 0
                        && pending.predecessor_checkpoint_sha256.is_none()
                        && pending.predecessor_generation.is_none()
                        && pending.checkpoint_generation == 0
                }
            };
            if !predecessor_matches {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
        }
        Ok(())
    }
}

pub(crate) mod science_checkpoint_anchor_sealed {
    pub(crate) trait Sealed {}
}

pub(crate) trait NisarSubsetScienceCheckpointAnchorV2:
    science_checkpoint_anchor_sealed::Sealed + Send + Sync
{
    fn load_budget_upgrade_proof(
        &self,
    ) -> Result<
        Option<insar_store::StoredNisarBudgetUpgradeProofV2>,
        NisarSubsetScienceTransferErrorV2,
    > {
        Ok(None)
    }

    fn load_budget_upgrade_context(
        &self,
    ) -> Result<insar_store::StoreNisarBudgetUpgradeContextV2, NisarSubsetScienceTransferErrorV2>
    {
        Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
    }

    fn prepare_budget_upgrade(
        &self,
        _proof: &insar_store::StoredNisarBudgetUpgradeProofV2,
        _replacement: &NisarSubsetScienceCheckpointAnchorRecordV2,
    ) -> Result<bool, NisarSubsetScienceTransferErrorV2> {
        Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
    }

    fn load(
        &self,
        lineage_key_sha256: &str,
    ) -> Result<Option<NisarSubsetScienceCheckpointAnchorRecordV2>, NisarSubsetScienceTransferErrorV2>;

    fn compare_and_swap(
        &self,
        lineage_key_sha256: &str,
        expected: Option<&NisarSubsetScienceCheckpointAnchorRecordV2>,
        replacement: &NisarSubsetScienceCheckpointAnchorRecordV2,
    ) -> Result<bool, NisarSubsetScienceTransferErrorV2>;
}

pub(crate) mod science_authority_sealed {
    pub(crate) trait Sealed {}
}

pub(crate) trait NisarSubsetScienceAuthorityV2: science_authority_sealed::Sealed {
    fn revalidate(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        control: &WorkerControl,
    ) -> Result<
        (
            NisarSubsetPhysicalExecutionPlanV2,
            NisarSubsetBootstrapStatisticsV2,
        ),
        NisarSubsetPlanningErrorV2,
    >;

    fn revalidate_before_range(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        expected_plan: &NisarSubsetPhysicalExecutionPlanV2,
        control: &WorkerControl,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        let (current, _) = self.revalidate(work, completed, control)?;
        if &current != expected_plan {
            return Err(NisarSubsetPlanningErrorV2::PhysicalPlanInvalid);
        }
        Ok(())
    }

    fn workspace(&self) -> &Hdf5RangeWorkspaceV1;
    fn remote_source(&self) -> &NisarSubsetScienceRemoteSourceV2;
    fn checkpoint_anchor(&self) -> &dyn NisarSubsetScienceCheckpointAnchorV2;
}

pub(crate) struct NisarSubsetScienceAuthorityBindingV2<'a> {
    planning_authority: &'a dyn NisarSubsetMetadataBootstrapV2,
    workspace: &'a Hdf5RangeWorkspaceV1,
    remote_source: NisarSubsetScienceRemoteSourceV2,
    checkpoint_anchor: &'a dyn NisarSubsetScienceCheckpointAnchorV2,
}

impl<'a> NisarSubsetScienceAuthorityBindingV2<'a> {
    pub(crate) fn new(
        planning_authority: &'a dyn NisarSubsetMetadataBootstrapV2,
        workspace: &'a Hdf5RangeWorkspaceV1,
        remote_source: NisarSubsetScienceRemoteSourceV2,
        checkpoint_anchor: &'a dyn NisarSubsetScienceCheckpointAnchorV2,
    ) -> Self {
        Self {
            planning_authority,
            workspace,
            remote_source,
            checkpoint_anchor,
        }
    }
}

impl science_authority_sealed::Sealed for NisarSubsetScienceAuthorityBindingV2<'_> {}

impl NisarSubsetScienceAuthorityV2 for NisarSubsetScienceAuthorityBindingV2<'_> {
    fn revalidate(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        control: &WorkerControl,
    ) -> Result<
        (
            NisarSubsetPhysicalExecutionPlanV2,
            NisarSubsetBootstrapStatisticsV2,
        ),
        NisarSubsetPlanningErrorV2,
    > {
        revalidate_science_authority_v2(work, completed, self.planning_authority, control)
    }

    fn revalidate_before_range(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        expected_plan: &NisarSubsetPhysicalExecutionPlanV2,
        control: &WorkerControl,
    ) -> Result<(), NisarSubsetPlanningErrorV2> {
        revalidate_science_range_binding_v2(work, completed, expected_plan, control)
    }

    fn workspace(&self) -> &Hdf5RangeWorkspaceV1 {
        self.workspace
    }

    fn remote_source(&self) -> &NisarSubsetScienceRemoteSourceV2 {
        &self.remote_source
    }

    fn checkpoint_anchor(&self) -> &dyn NisarSubsetScienceCheckpointAnchorV2 {
        self.checkpoint_anchor
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum NisarSubsetScienceCheckpointStateV2 {
    Planned,
    InProgress,
    Interrupted,
    Complete,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CompletedScienceRangeV2 {
    index: u16,
    start: u64,
    end_exclusive: u64,
    request_count: u64,
    response_sha256: String,
    allocated_range_evidence: NisarScienceAllocatedRangeEvidenceV2,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum NisarSubsetSciencePendingAttemptStateV2 {
    Reserved,
    ResponseRecorded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct NisarSubsetSciencePendingAttemptV2 {
    range_index: u16,
    start: u64,
    end_exclusive: u64,
    reserved_response_body_bytes: u64,
    // Absence preserves the canonical identity and one-slot reservation of legacy records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reserved_request_count: Option<u64>,
    state: NisarSubsetSciencePendingAttemptStateV2,
    request_issued: bool,
    request_count: u64,
    observed_response_body_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct NisarSubsetScienceCheckpointV2 {
    checkpoint_contract: String,
    schema_version: u32,
    workspace_canonical_path: String,
    workspace_identity: String,
    lineage_key_sha256: String,
    execution_binding_sha256: String,
    anchor_generation: u64,
    previous_checkpoint_sha256: Option<String>,
    part_identity_sha256: String,
    task_id: String,
    item_id: String,
    prepared_intent_sha256: String,
    final_plan_sha256: String,
    physical_plan_sha256: String,
    inventory_sha256: String,
    inventory_evidence_sha256: String,
    source_graph_sha256: String,
    source_product_identity: NisarSubsetSourceProductIdentityV2,
    source_asset_identity: NisarSubsetSourceAssetIdentityV2,
    source_validator: NisarSubsetSourceValidatorV1,
    source_validator_sha256: String,
    source_size: u64,
    planned_ranges: Vec<NisarSubsetPhysicalRangeV2>,
    completed_ranges: Vec<CompletedScienceRangeV2>,
    pending_attempt: Option<NisarSubsetSciencePendingAttemptV2>,
    committed_planning_request_count: u64,
    committed_planning_response_body_bytes: u64,
    reserved_science_request_count: u64,
    reserved_science_response_body_bytes: u64,
    issued_science_request_count: u64,
    observed_science_response_body_bytes: u64,
    failed_science_request_count: u64,
    unissued_science_reservation_count: u64,
    indeterminate_science_request_count: u64,
    indeterminate_science_response_body_bytes: u64,
    committed_science_request_count: u64,
    committed_science_response_body_bytes: u64,
    resumed_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget_upgrade_receipt: Option<NisarScienceBudgetUpgradeReceiptV2>,
    state: NisarSubsetScienceCheckpointStateV2,
    checkpoint_sha256: String,
}

impl NisarSubsetScienceCheckpointV2 {
    fn new(
        work: &DownloadWork,
        plan: &NisarSubsetPhysicalExecutionPlanV2,
        workspace_canonical_path: String,
        workspace_identity: String,
        lineage_key_sha256: String,
        execution_binding_sha256: String,
        part_identity_sha256: String,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        let budget = plan.budget_projection();
        let planned_ranges = plan
            .execution_transfer_ranges()
            .map_err(|error| match error {
                NisarSubsetPlanningErrorV2::CombinedBudgetExceeded => {
                    NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded
                }
                _ => NisarSubsetScienceTransferErrorV2::AuthorityMismatch,
            })?;
        let mut checkpoint = Self {
            checkpoint_contract: SCIENCE_CHECKPOINT_CONTRACT_V2.to_owned(),
            schema_version: SCIENCE_CHECKPOINT_SCHEMA_VERSION_V2,
            workspace_canonical_path,
            workspace_identity,
            lineage_key_sha256,
            execution_binding_sha256,
            anchor_generation: 0,
            previous_checkpoint_sha256: None,
            part_identity_sha256,
            task_id: work.task.task_id.clone(),
            item_id: work.item.item_id.clone(),
            prepared_intent_sha256: plan.prepared_intent_sha256().to_owned(),
            final_plan_sha256: plan.final_plan_sha256().to_owned(),
            physical_plan_sha256: plan.physical_plan_sha256().to_owned(),
            inventory_sha256: plan.inventory_sha256().to_owned(),
            inventory_evidence_sha256: plan.inventory_evidence_sha256().to_owned(),
            source_graph_sha256: plan.source_graph_sha256().to_owned(),
            source_product_identity: plan.source_product_identity().clone(),
            source_asset_identity: plan.source_asset_identity().clone(),
            source_validator: plan.source_validator().clone(),
            source_validator_sha256: plan.source_validator_sha256().to_owned(),
            source_size: plan.source_size(),
            planned_ranges,
            completed_ranges: Vec::new(),
            pending_attempt: None,
            committed_planning_request_count: budget.committed_planning_request_count(),
            committed_planning_response_body_bytes: budget.committed_planning_response_body_bytes(),
            reserved_science_request_count: 0,
            reserved_science_response_body_bytes: 0,
            issued_science_request_count: 0,
            observed_science_response_body_bytes: 0,
            failed_science_request_count: 0,
            unissued_science_reservation_count: 0,
            indeterminate_science_request_count: 0,
            indeterminate_science_response_body_bytes: 0,
            committed_science_request_count: 0,
            committed_science_response_body_bytes: 0,
            resumed_bytes: 0,
            budget_upgrade_receipt: None,
            state: NisarSubsetScienceCheckpointStateV2::Planned,
            checkpoint_sha256: String::new(),
        };
        checkpoint.seal()?;
        Ok(checkpoint)
    }

    fn seal(&mut self) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        self.checkpoint_sha256 = self.recompute_sha256()?;
        Ok(())
    }

    fn recompute_sha256(&self) -> Result<String, NisarSubsetScienceTransferErrorV2> {
        #[derive(Serialize)]
        struct Identity<'a> {
            checkpoint_contract: &'a str,
            schema_version: u32,
            workspace_canonical_path: &'a str,
            workspace_identity: &'a str,
            lineage_key_sha256: &'a str,
            execution_binding_sha256: &'a str,
            anchor_generation: u64,
            previous_checkpoint_sha256: &'a Option<String>,
            part_identity_sha256: &'a str,
            task_id: &'a str,
            item_id: &'a str,
            prepared_intent_sha256: &'a str,
            final_plan_sha256: &'a str,
            physical_plan_sha256: &'a str,
            inventory_sha256: &'a str,
            inventory_evidence_sha256: &'a str,
            source_graph_sha256: &'a str,
            source_product_identity: &'a NisarSubsetSourceProductIdentityV2,
            source_asset_identity: &'a NisarSubsetSourceAssetIdentityV2,
            source_validator: &'a NisarSubsetSourceValidatorV1,
            source_validator_sha256: &'a str,
            source_size: u64,
            planned_ranges: &'a [NisarSubsetPhysicalRangeV2],
            completed_ranges: &'a [CompletedScienceRangeV2],
            pending_attempt: &'a Option<NisarSubsetSciencePendingAttemptV2>,
            committed_planning_request_count: u64,
            committed_planning_response_body_bytes: u64,
            reserved_science_request_count: u64,
            reserved_science_response_body_bytes: u64,
            issued_science_request_count: u64,
            observed_science_response_body_bytes: u64,
            failed_science_request_count: u64,
            unissued_science_reservation_count: u64,
            indeterminate_science_request_count: u64,
            indeterminate_science_response_body_bytes: u64,
            committed_science_request_count: u64,
            committed_science_response_body_bytes: u64,
            resumed_bytes: u64,
            #[serde(skip_serializing_if = "Option::is_none")]
            budget_upgrade_receipt: &'a Option<NisarScienceBudgetUpgradeReceiptV2>,
            state: NisarSubsetScienceCheckpointStateV2,
        }
        digest_serializable(&Identity {
            checkpoint_contract: &self.checkpoint_contract,
            schema_version: self.schema_version,
            workspace_canonical_path: &self.workspace_canonical_path,
            workspace_identity: &self.workspace_identity,
            lineage_key_sha256: &self.lineage_key_sha256,
            execution_binding_sha256: &self.execution_binding_sha256,
            anchor_generation: self.anchor_generation,
            previous_checkpoint_sha256: &self.previous_checkpoint_sha256,
            part_identity_sha256: &self.part_identity_sha256,
            task_id: &self.task_id,
            item_id: &self.item_id,
            prepared_intent_sha256: &self.prepared_intent_sha256,
            final_plan_sha256: &self.final_plan_sha256,
            physical_plan_sha256: &self.physical_plan_sha256,
            inventory_sha256: &self.inventory_sha256,
            inventory_evidence_sha256: &self.inventory_evidence_sha256,
            source_graph_sha256: &self.source_graph_sha256,
            source_product_identity: &self.source_product_identity,
            source_asset_identity: &self.source_asset_identity,
            source_validator: &self.source_validator,
            source_validator_sha256: &self.source_validator_sha256,
            source_size: self.source_size,
            planned_ranges: &self.planned_ranges,
            completed_ranges: &self.completed_ranges,
            pending_attempt: &self.pending_attempt,
            committed_planning_request_count: self.committed_planning_request_count,
            committed_planning_response_body_bytes: self.committed_planning_response_body_bytes,
            reserved_science_request_count: self.reserved_science_request_count,
            reserved_science_response_body_bytes: self.reserved_science_response_body_bytes,
            issued_science_request_count: self.issued_science_request_count,
            observed_science_response_body_bytes: self.observed_science_response_body_bytes,
            failed_science_request_count: self.failed_science_request_count,
            unissued_science_reservation_count: self.unissued_science_reservation_count,
            indeterminate_science_request_count: self.indeterminate_science_request_count,
            indeterminate_science_response_body_bytes: self
                .indeterminate_science_response_body_bytes,
            committed_science_request_count: self.committed_science_request_count,
            committed_science_response_body_bytes: self.committed_science_response_body_bytes,
            resumed_bytes: self.resumed_bytes,
            budget_upgrade_receipt: &self.budget_upgrade_receipt,
            state: self.state,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn validate(
        &self,
        work: &DownloadWork,
        plan: &NisarSubsetPhysicalExecutionPlanV2,
        workspace_canonical_path: &str,
        workspace_identity: &str,
        lineage_key_sha256: &str,
        execution_binding_sha256: &str,
        part_identity_sha256: &str,
    ) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        self.validate_with_budget(
            work,
            plan,
            workspace_canonical_path,
            workspace_identity,
            lineage_key_sha256,
            execution_binding_sha256,
            part_identity_sha256,
            &NisarValidatedScienceBudgetV2::original(plan),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_with_budget(
        &self,
        work: &DownloadWork,
        plan: &NisarSubsetPhysicalExecutionPlanV2,
        workspace_canonical_path: &str,
        workspace_identity: &str,
        lineage_key_sha256: &str,
        execution_binding_sha256: &str,
        part_identity_sha256: &str,
        effective_budget: &NisarValidatedScienceBudgetV2,
    ) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        plan.validate()
            .map_err(|_| NisarSubsetScienceTransferErrorV2::AuthorityMismatch)?;
        let budget = plan.budget_projection();
        let planned_ranges = plan
            .execution_transfer_ranges()
            .map_err(|_| NisarSubsetScienceTransferErrorV2::AuthorityMismatch)?;
        if !effective_budget.matches_receipt(self)
            || self.checkpoint_contract != SCIENCE_CHECKPOINT_CONTRACT_V2
            || self.schema_version != SCIENCE_CHECKPOINT_SCHEMA_VERSION_V2
            || self.workspace_canonical_path != workspace_canonical_path
            || self.workspace_identity != workspace_identity
            || self.lineage_key_sha256 != lineage_key_sha256
            || self.execution_binding_sha256 != execution_binding_sha256
            || (self.anchor_generation == 0) != self.previous_checkpoint_sha256.is_none()
            || self
                .previous_checkpoint_sha256
                .as_deref()
                .is_some_and(|digest| !valid_sha256(digest))
            || self.part_identity_sha256 != part_identity_sha256
            || self.task_id != work.task.task_id
            || self.item_id != work.item.item_id
            || self.prepared_intent_sha256 != plan.prepared_intent_sha256()
            || self.final_plan_sha256 != plan.final_plan_sha256()
            || self.physical_plan_sha256 != plan.physical_plan_sha256()
            || self.inventory_sha256 != plan.inventory_sha256()
            || self.inventory_evidence_sha256 != plan.inventory_evidence_sha256()
            || self.source_graph_sha256 != plan.source_graph_sha256()
            || self.source_product_identity != *plan.source_product_identity()
            || self.source_asset_identity != *plan.source_asset_identity()
            || self.source_validator != *plan.source_validator()
            || self.source_validator_sha256 != plan.source_validator_sha256()
            || self.source_size != plan.source_size()
            || self.planned_ranges != planned_ranges
            || self.committed_planning_request_count != budget.committed_planning_request_count()
            || self.committed_planning_response_body_bytes
                != budget.committed_planning_response_body_bytes()
            || !valid_sha256(&self.lineage_key_sha256)
            || !valid_sha256(&self.execution_binding_sha256)
            || !valid_sha256(&self.part_identity_sha256)
            || !valid_sha256(&self.checkpoint_sha256)
            || self.recompute_sha256()? != self.checkpoint_sha256
            || self.completed_ranges.len() > self.planned_ranges.len()
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let mut completed_bytes = 0_u64;
        let mut completed_request_count = 0_u64;
        let mut previous_index = None;
        for completed in &self.completed_ranges {
            let index = usize::from(completed.index);
            let planned = self
                .planned_ranges
                .get(index)
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
            if previous_index.is_some_and(|previous| previous >= completed.index)
                || completed.start != planned.start()
                || completed.end_exclusive != planned.end_exclusive()
                || completed.request_count == 0
                || !valid_sha256(&completed.response_sha256)
                || !completed
                    .allocated_range_evidence
                    .covers_exactly(completed.start, completed.end_exclusive)
            {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
            previous_index = Some(completed.index);
            completed_request_count = completed_request_count
                .checked_add(completed.request_count)
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
            completed_bytes = completed_bytes
                .checked_add(
                    planned
                        .len()
                        .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?,
                )
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        }
        let pending_reserved_count = self
            .pending_attempt
            .as_ref()
            .filter(|pending| pending.state == NisarSubsetSciencePendingAttemptStateV2::Reserved)
            .map_or(0, |pending| pending.reserved_request_count.unwrap_or(1));
        let pending_response_count = self
            .pending_attempt
            .as_ref()
            .filter(|pending| {
                pending.state == NisarSubsetSciencePendingAttemptStateV2::ResponseRecorded
            })
            .map_or(0, |pending| pending.request_count);
        if let Some(pending) = &self.pending_attempt {
            let planned = self
                .planned_ranges
                .get(usize::from(pending.range_index))
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
            if pending.start != planned.start()
                || pending.end_exclusive != planned.end_exclusive()
                || pending.reserved_request_count.is_some_and(|reserved| {
                    reserved == 0
                        || reserved > MAX_SCIENCE_HTTP_REQUESTS_PER_ATTEMPT
                        || pending.request_count > reserved
                })
                || pending.reserved_response_body_bytes
                    != planned
                        .len()
                        .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?
                || pending.observed_response_body_bytes > pending.reserved_response_body_bytes
                || (pending.state == NisarSubsetSciencePendingAttemptStateV2::Reserved
                    && (pending.request_issued
                        || pending.request_count != 0
                        || pending.observed_response_body_bytes != 0))
                || (pending.state == NisarSubsetSciencePendingAttemptStateV2::ResponseRecorded
                    && (!pending.request_issued || pending.request_count == 0))
            {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
        }
        let all_complete = self.completed_ranges.len() == self.planned_ranges.len()
            && self.pending_attempt.is_none();
        if self.committed_science_request_count != completed_request_count
            || self.committed_science_response_body_bytes != completed_bytes
            || self.resumed_bytes > completed_bytes
            || matches!(self.state, NisarSubsetScienceCheckpointStateV2::Complete) != all_complete
            || self.reserved_science_request_count
                != self
                    .issued_science_request_count
                    .checked_add(self.unissued_science_reservation_count)
                    .and_then(|total| total.checked_add(self.indeterminate_science_request_count))
                    .and_then(|total| total.checked_add(pending_reserved_count))
                    .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?
            || self.issued_science_request_count
                != self
                    .committed_science_request_count
                    .checked_add(self.failed_science_request_count)
                    .and_then(|total| total.checked_add(pending_response_count))
                    .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?
            || self.observed_science_response_body_bytes > self.reserved_science_response_body_bytes
            || self.indeterminate_science_response_body_bytes
                > self.reserved_science_response_body_bytes
            || (self.indeterminate_science_request_count == 0)
                != (self.indeterminate_science_response_body_bytes == 0)
            || self.committed_science_response_body_bytes
                > self.observed_science_response_body_bytes
            || self.reserved_science_request_count > effective_budget.max_science_request_count()
            || self
                .committed_planning_request_count
                .checked_add(self.reserved_science_request_count)
                .is_none_or(|total| total > effective_budget.max_request_count())
            || self
                .committed_planning_response_body_bytes
                .checked_add(self.reserved_science_response_body_bytes)
                .is_none_or(|total| total > effective_budget.max_response_body_bytes())
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NisarSubsetScienceAccountingV2 {
    planning_request_count: u64,
    planning_response_body_bytes: u64,
    reserved_science_request_count: u64,
    reserved_science_response_body_bytes: u64,
    science_request_count: u64,
    science_response_body_bytes: u64,
    committed_science_request_count: u64,
    committed_science_response_body_bytes: u64,
    failed_science_request_count: u64,
    indeterminate_science_request_count: u64,
    indeterminate_science_response_body_bytes: u64,
    resumed_bytes: u64,
}

impl NisarSubsetScienceAccountingV2 {
    pub(crate) const fn planning_request_count(&self) -> u64 {
        self.planning_request_count
    }
    pub(crate) const fn planning_response_body_bytes(&self) -> u64 {
        self.planning_response_body_bytes
    }
    pub(crate) const fn reserved_science_request_count(&self) -> u64 {
        self.reserved_science_request_count
    }
    pub(crate) const fn reserved_science_response_body_bytes(&self) -> u64 {
        self.reserved_science_response_body_bytes
    }
    pub(crate) const fn charged_science_response_body_bytes(&self) -> u64 {
        self.reserved_science_response_body_bytes
    }
    pub(crate) const fn science_request_count(&self) -> u64 {
        self.science_request_count
    }
    pub(crate) const fn science_response_body_bytes(&self) -> u64 {
        self.science_response_body_bytes
    }
    pub(crate) const fn observed_science_response_body_bytes(&self) -> u64 {
        self.science_response_body_bytes
    }
    pub(crate) const fn committed_science_request_count(&self) -> u64 {
        self.committed_science_request_count
    }
    pub(crate) const fn committed_science_response_body_bytes(&self) -> u64 {
        self.committed_science_response_body_bytes
    }
    pub(crate) const fn failed_science_request_count(&self) -> u64 {
        self.failed_science_request_count
    }
    pub(crate) const fn indeterminate_science_request_count(&self) -> u64 {
        self.indeterminate_science_request_count
    }
    pub(crate) const fn indeterminate_science_response_body_bytes(&self) -> u64 {
        self.indeterminate_science_response_body_bytes
    }
    pub(crate) const fn resumed_bytes(&self) -> u64 {
        self.resumed_bytes
    }
    pub(crate) fn combined_request_count(&self) -> u64 {
        self.planning_request_count
            .saturating_add(self.reserved_science_request_count)
    }
    pub(crate) fn combined_response_body_bytes(&self) -> u64 {
        self.planning_response_body_bytes
            .saturating_add(self.reserved_science_response_body_bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarSubsetScienceTransferResultV2 {
    complete: bool,
    effective_budget: NisarValidatedScienceBudgetV2,
    accounting: NisarSubsetScienceAccountingV2,
    physical_plan_sha256: String,
    completed_ranges: Vec<(u64, u64)>,
    completed_range_request_counts: Vec<u64>,
}

impl NisarSubsetScienceTransferResultV2 {
    pub(crate) fn max_request_count(&self) -> u64 {
        self.effective_budget.max_request_count()
    }
    pub(crate) fn max_response_body_bytes(&self) -> u64 {
        self.effective_budget.max_response_body_bytes()
    }
    pub(crate) fn max_science_request_count(&self) -> u64 {
        self.effective_budget.max_science_request_count()
    }
    #[cfg(test)]
    pub(crate) fn mark_incomplete_for_test(&mut self) {
        self.complete = false;
    }

    pub(crate) const fn complete(&self) -> bool {
        self.complete
    }
    pub(crate) const fn accounting(&self) -> NisarSubsetScienceAccountingV2 {
        self.accounting
    }
    pub(crate) const fn science_request_count(&self) -> u64 {
        self.accounting.science_request_count()
    }
    pub(crate) const fn science_response_body_bytes(&self) -> u64 {
        self.accounting.science_response_body_bytes()
    }
    pub(crate) const fn resumed_bytes(&self) -> u64 {
        self.accounting.resumed_bytes()
    }
    pub(crate) fn physical_plan_sha256(&self) -> &str {
        &self.physical_plan_sha256
    }

    pub(crate) fn completed_ranges(&self) -> &[(u64, u64)] {
        &self.completed_ranges
    }

    pub(crate) fn completed_range_request_counts(&self) -> &[u64] {
        &self.completed_range_request_counts
    }
}

/// How much a writer revalidation of the pinned science part reads
/// (downloader enhancement D2, 2026-10-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriterMaterialCheckV2 {
    /// Identities and markers of the fixed handles and the workspace child and
    /// the sealed checkpoint; no science byte is read.
    Identity,
    /// `Identity` plus one digest of every completed Range, right before the
    /// writer publishes (or completes a recovered artifact).
    FinalDigest,
}

/// Stops a completed-material digest between two reads (downloader
/// enhancement D2): `None` continues; `Some(error)` returns that error.
pub(crate) type MaterialDigestStopV2<'a> =
    &'a dyn Fn() -> Option<NisarSubsetScienceTransferErrorV2>;

/// A digest that always runs to its end: the one-time budget migration and
/// archived-recovery entries, which have their own refusal semantics, and the
/// digest that completes the commit of a freshly written Range.
pub(crate) fn digest_runs_to_the_end() -> Option<NisarSubsetScienceTransferErrorV2> {
    None
}

/// The worker's cancellation or pause stops a digest before its next read.
pub(crate) fn worker_digest_stop(
    control: &WorkerControl,
) -> impl Fn() -> Option<NisarSubsetScienceTransferErrorV2> + '_ {
    move || {
        if control.is_cancelled() {
            Some(NisarSubsetScienceTransferErrorV2::Cancelled)
        } else if control.is_paused() {
            Some(NisarSubsetScienceTransferErrorV2::Paused)
        } else {
            None
        }
    }
}

/// A read-only, externally anchored proof that the exact Task 2 science
/// checkpoint and guarded sparse part are complete. The workspace lock is kept
/// alive so no checkpoint or part mutation can race a Writer pass.
pub(crate) struct RevalidatedCompleteScienceForWriterV2 {
    workspace: Hdf5RangeWorkspaceV1,
    work: DownloadWork,
    plan: NisarSubsetPhysicalExecutionPlanV2,
    result: NisarSubsetScienceTransferResultV2,
    checkpoint: NisarSubsetScienceCheckpointV2,
    checkpoint_sha256: String,
    completed_range_evidence_sha256: String,
    workspace_canonical_path: String,
    workspace_identity: String,
    lineage_key_sha256: String,
    execution_binding_sha256: String,
    part_identity_sha256: String,
    pinned_part: ImmutableSciencePartForWriterV2,
    part_path: std::path::PathBuf,
    _workspace_lock: Hdf5WorkspaceLockV1,
}

impl RevalidatedCompleteScienceForWriterV2 {
    pub(crate) fn plan(&self) -> &NisarSubsetPhysicalExecutionPlanV2 {
        &self.plan
    }

    pub(crate) fn result(&self) -> &NisarSubsetScienceTransferResultV2 {
        &self.result
    }

    pub(crate) fn checkpoint_sha256(&self) -> &str {
        &self.checkpoint_sha256
    }

    pub(crate) fn completed_range_evidence_sha256(&self) -> &str {
        &self.completed_range_evidence_sha256
    }

    pub(crate) fn part_path(&self) -> &std::path::Path {
        &self.part_path
    }

    /// Returns the HDF5-openable immutable view retained for this capability.
    pub(crate) fn pinned_hdf5_path(&self) -> std::path::PathBuf {
        self.pinned_part.view_path().to_path_buf()
    }

    #[cfg(test)]
    pub(crate) fn writer_snapshot_path_v2_for_test(&self) -> std::path::PathBuf {
        self.workspace.writer_snapshot_path_v2_for_test()
    }

    /// Downloader enhancement D2 (2026-10-18): compares, without reading any
    /// science byte, the workspace identity, the sealed checkpoint, the file
    /// identity (volume serial and file id) and modification marker (size,
    /// times and attributes) of the workspace's current part and of the
    /// retained source handle with the values pinned at the writer entry, and
    /// the size of the immutable view. The completed material is digested at
    /// the writer entry and once more by `revalidate_part_material_digest`
    /// right before the writer publishes; in between the bytes cannot change
    /// unnoticed: Windows pins the part denying write and delete sharing, and
    /// Linux reads the pinned part through its descriptor while the source
    /// marker includes its change time.
    pub(crate) fn verify_part_identity(&self) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        #[cfg(test)]
        let _hash_site = completed_material_hash_ledger::enter("writer_part_material");
        self.verify_part_identity_and_reopen().map(|_| ())
    }

    fn verify_part_identity_and_reopen(
        &self,
    ) -> Result<(NisarSubsetScienceCheckpointV2, File), NisarSubsetScienceTransferErrorV2> {
        let (workspace_canonical_path, workspace_identity) = self
            .workspace
            .bound_workspace_identity_v2()
            .map_err(map_workspace_error)?;
        if workspace_canonical_path != self.workspace_canonical_path
            || workspace_identity != self.workspace_identity
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let checkpoint = self
            .workspace
            .load_science_checkpoint_v2()
            .map_err(map_workspace_error)?
            .ok_or(NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete)
            .and_then(|canonical| parse_checkpoint_canonical_json(&canonical))?;
        if checkpoint != self.checkpoint {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let (current_part, current_identity_sha256, current_marker) = self
            .workspace
            .reopen_science_part_for_writer_v2(self.plan.source_size())
            .map_err(map_workspace_error)?;
        if current_identity_sha256 != self.part_identity_sha256
            || current_marker != self.pinned_part.source_mutation_marker_sha256()
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        self.verify_pinned_handles()?;
        Ok((checkpoint, current_part))
    }

    /// The fixed open handles still describe the pinned part: the retained
    /// source has its pinned modification marker and the immutable view the
    /// planned size.
    fn verify_pinned_handles(&self) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        if self
            .workspace
            .science_part_mutation_marker_for_writer_v2(self.pinned_part.source())
            .map_err(map_workspace_error)?
            != self.pinned_part.source_mutation_marker_sha256()
            || self
                .pinned_part
                .view()
                .metadata()
                .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?
                .len()
                != self.plan.source_size()
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        Ok(())
    }

    /// The final digest before the writer publishes or completes a recovered
    /// artifact: the identity checks, one digest of every completed Range of
    /// the immutable view (the bytes the writer read) and the markers again
    /// after it. `stop` ends the digest at a pause or cancellation.
    pub(crate) fn revalidate_part_material_digest(
        &self,
        stop: MaterialDigestStopV2<'_>,
    ) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        #[cfg(test)]
        let _hash_site = completed_material_hash_ledger::enter("writer_final_digest");
        let (checkpoint, current_part) = self.verify_part_identity_and_reopen()?;
        let view = self
            .pinned_part
            .view()
            .try_clone()
            .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        #[cfg(test)]
        let view = final_digest_view_for_test::substitute(view);
        let mut view = view;
        validate_recovery_checkpoint_with_budget(
            &checkpoint,
            &self.work,
            &self.plan,
            &self.workspace_canonical_path,
            &self.workspace_identity,
            &self.lineage_key_sha256,
            &self.execution_binding_sha256,
            &self.part_identity_sha256,
            &mut view,
            &self.result.effective_budget,
            stop,
        )?;
        if self
            .workspace
            .science_part_mutation_marker_for_writer_v2(&current_part)
            .map_err(map_workspace_error)?
            != self.pinned_part.source_mutation_marker_sha256()
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        self.verify_pinned_handles()
    }

    /// Revalidates the Store authority and the part: by identity only, or with
    /// the final digest right before the writer publishes (downloader
    /// enhancement D2). A pause or cancellation stops the digest.
    pub(crate) fn revalidate_authority(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        reported: &NisarSubsetScienceTransferResultV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        check: WriterMaterialCheckV2,
    ) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        let (plan, _) = authority
            .revalidate(work, completed, control)
            .map_err(|error| map_planning_error(error, control))?;
        authority.remote_source().validate(work, completed, &plan)?;
        if plan != self.plan || !result_matches_checkpoint_authority(reported, &self.result) {
            return Err(NisarSubsetScienceTransferErrorV2::AuthorityMismatch);
        }
        match check {
            WriterMaterialCheckV2::Identity => self.verify_part_identity()?,
            WriterMaterialCheckV2::FinalDigest => {
                self.revalidate_part_material_digest(&worker_digest_stop(control))?
            }
        }
        reconcile_checkpoint_anchor(
            authority.checkpoint_anchor(),
            &self.lineage_key_sha256,
            &self.checkpoint,
        )?;
        Ok(())
    }
}

pub(crate) fn revalidate_complete_science_for_writer_v2(
    work: &DownloadWork,
    completed: &NisarSubsetPlanningCompleteV2,
    reported: &NisarSubsetScienceTransferResultV2,
    authority: &dyn NisarSubsetScienceAuthorityV2,
    control: &WorkerControl,
) -> Result<RevalidatedCompleteScienceForWriterV2, NisarSubsetScienceTransferErrorV2> {
    #[cfg(test)]
    let _hash_site = completed_material_hash_ledger::enter("writer_science_revalidation");
    let (plan, _) = authority
        .revalidate(work, completed, control)
        .map_err(|error| map_planning_error(error, control))?;
    authority.remote_source().validate(work, completed, &plan)?;
    let workspace = authority.workspace();
    let workspace_lock = workspace.acquire_lock().map_err(map_workspace_error)?;
    let (workspace_canonical_path, workspace_identity) = workspace
        .bound_workspace_identity_v2()
        .map_err(map_workspace_error)?;
    let pinned_part = workspace
        .pin_science_part_for_writer_v2(plan.source_size())
        .map_err(map_workspace_error)?;
    let part_identity_sha256 = pinned_part.source_identity_sha256().to_owned();
    let mut part = pinned_part
        .view()
        .try_clone()
        .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    let lineage_key_sha256 = lineage_key_sha256(work, &plan)?;
    let execution_binding_sha256 = execution_binding_sha256(
        &lineage_key_sha256,
        &plan,
        authority.remote_source(),
        &workspace_canonical_path,
        &workspace_identity,
    )?;
    let checkpoint = workspace
        .load_science_checkpoint_v2()
        .map_err(map_workspace_error)?
        .ok_or(NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete)
        .and_then(|canonical| parse_checkpoint_canonical_json(&canonical))?;
    let budget = budget_upgrade::validated_budget(
        &checkpoint,
        work,
        &plan,
        workspace,
        authority.checkpoint_anchor(),
        &workspace_canonical_path,
        &workspace_identity,
        &execution_binding_sha256,
        &part_identity_sha256,
    )?;
    validate_recovery_checkpoint_with_budget(
        &checkpoint,
        work,
        &plan,
        &workspace_canonical_path,
        &workspace_identity,
        &lineage_key_sha256,
        &execution_binding_sha256,
        &part_identity_sha256,
        &mut part,
        &budget,
        &worker_digest_stop(control),
    )?;
    reconcile_checkpoint_anchor(
        authority.checkpoint_anchor(),
        &lineage_key_sha256,
        &checkpoint,
    )?;
    let result = result_from_checkpoint(&checkpoint, &budget);
    // Owner decision 2026-10-17 (option B, replacing decision A): an
    // indeterminate charge (a Range attempt a killed process left Reserved,
    // reclassified on recovery and never refunded) is an accounting fact, not
    // missing science authority. It is accepted only here, after every
    // completed Range passed its allocation evidence and response digest over
    // the pinned part (validate_recovery_checkpoint_with_budget) and the ledger
    // passed validate_with_budget; every other condition below still refuses.
    // The writer records the charge bounds in the artifact manifest.
    if checkpoint.state != NisarSubsetScienceCheckpointStateV2::Complete
        || checkpoint.pending_attempt.is_some()
        || checkpoint.completed_ranges.len() != checkpoint.planned_ranges.len()
        || !reported.complete()
        || !result_matches_checkpoint_authority(reported, &result)
        || result.physical_plan_sha256() != plan.physical_plan_sha256()
    {
        return Err(NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete);
    }
    let completed_range_evidence_sha256 = digest_serializable(&checkpoint.completed_ranges)?;
    Ok(RevalidatedCompleteScienceForWriterV2 {
        workspace: workspace.clone(),
        work: work.clone(),
        plan,
        result,
        checkpoint_sha256: checkpoint.checkpoint_sha256.clone(),
        completed_range_evidence_sha256,
        workspace_canonical_path,
        workspace_identity,
        lineage_key_sha256,
        execution_binding_sha256,
        part_identity_sha256,
        pinned_part,
        part_path: workspace.part_path(),
        checkpoint,
        _workspace_lock: workspace_lock,
    })
}

struct NisarSubsetScienceFetchSuccessV2 {
    body: Vec<u8>,
    request_count: u64,
    observed_response_body_bytes: u64,
}

struct NisarSubsetScienceFetchFailureV2 {
    error: NisarSubsetScienceTransferErrorV2,
    request_count: u64,
    observed_response_body_bytes: u64,
    /// The HTTP status of the response that ended the attempt, when one exists.
    http_status: Option<u16>,
    /// The transport cause when no usable response was received.
    transport_cause: Option<&'static str>,
}

/// Bounded automatic retry of one science Range after a transient failure
/// (design 2026-10-17 §3): the first attempt plus two automatic retries per
/// Range, and at most eight automatic retries per execution.
pub(crate) const SCIENCE_RANGE_MAX_ATTEMPTS: u32 = crate::transient_retry::TRANSIENT_MAX_ATTEMPTS;
const SCIENCE_EXECUTION_MAX_AUTOMATIC_RETRIES: u32 = 8;

/// Backoff before automatic retry `retry` (1-based): 5 s, then 20 s; an HTTP
/// 429 waits at least 30 s.
/// The shared policy of `crate::transient_retry` (also used by the Bhoonidhi
/// download probe since downloader enhancement D1d).
pub(crate) fn science_retry_backoff(retry: u32, http_status: Option<u16>) -> std::time::Duration {
    crate::transient_retry::transient_retry_backoff(retry, http_status)
}

pub(crate) type ScienceRetryBackoff = dyn Fn(u32, Option<u16>) -> std::time::Duration + Send + Sync;

impl NisarSubsetScienceFetchFailureV2 {
    /// Transient per design §2: no usable response (connection, timeout, 5xx,
    /// 429), a 412, a short or interrupted body, or a transient auth refresh.
    /// HTTP 200 and other non-206 statuses, validator/size/URL mismatches,
    /// budget exhaustion and local recovery errors stay terminal.
    fn is_transient(&self) -> bool {
        match self.error {
            NisarSubsetScienceTransferErrorV2::Transport
            | NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete
            | NisarSubsetScienceTransferErrorV2::AuthRefreshTransient => true,
            NisarSubsetScienceTransferErrorV2::ResponseInvalid => self.http_status == Some(412),
            _ => false,
        }
    }
}

/// What the science executor reports to its caller while it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarScienceTransferProgressV2 {
    /// Response-body bytes of the current attempt arrived.
    Bytes(u64),
    /// Every planned Range is committed, so no science transfer follows in
    /// this execution; final verification and the output come next. Review
    /// 2026-10-17 round 2, P2-2: reported before that final verification, so
    /// the task card shows the output phase at once.
    TransferComplete,
}

/// Clock of the interruptible retry backoff. Review 2026-10-17 round 2, P2-4:
/// injectable, so tests measure the real waits without sleeping.
pub(crate) trait ScienceRetryClock {
    fn now(&self) -> std::time::Instant;
    fn sleep(&self, duration: std::time::Duration);
}

struct SystemScienceRetryClock;

impl ScienceRetryClock for SystemScienceRetryClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn sleep(&self, duration: std::time::Duration) {
        std::thread::sleep(duration);
    }
}

/// Interruptible retry backoff: returns true as soon as a pause or cancel is
/// observed (polled every 20 ms), false when the delay elapsed.
pub(crate) fn wait_for_science_retry(delay: std::time::Duration, control: &WorkerControl) -> bool {
    wait_for_science_retry_with_clock(delay, control, &SystemScienceRetryClock)
}

pub(crate) fn wait_for_science_retry_with_clock(
    delay: std::time::Duration,
    control: &WorkerControl,
    clock: &dyn ScienceRetryClock,
) -> bool {
    let deadline = clock.now() + delay;
    loop {
        if control.is_cancelled() || control.is_paused() {
            return true;
        }
        let now = clock.now();
        if now >= deadline {
            return false;
        }
        clock.sleep((deadline - now).min(std::time::Duration::from_millis(20)));
    }
}

type ScienceRetryWait = dyn Fn(std::time::Duration, &WorkerControl) -> bool + Send + Sync;

/// The real cause of the last failed science attempt of one execution, kept
/// for the user-visible diagnostic. Counts are real attempts, never estimates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NisarScienceFailureDiagnosticV2 {
    /// Zero-based index of the Range whose attempt failed.
    pub(crate) range_index: usize,
    pub(crate) range_count: usize,
    /// Attempts of that Range in this execution, including the failed one.
    pub(crate) attempts: u32,
    /// Automatic retries already used by this execution.
    pub(crate) automatic_retries: u32,
    pub(crate) http_status: Option<u16>,
    pub(crate) transport_cause: Option<&'static str>,
    /// The failure class allows an automatic retry.
    pub(crate) transient: bool,
    /// The attempt failed in its authentication refresh step, so the cause is
    /// the refresh diagnostic rather than an HTTP status or transport cause.
    pub(crate) auth_refresh_failure: bool,
    /// A retry was due but its request budget could not be reserved, so
    /// nothing more was sent; the cause fields still describe the last attempt.
    pub(crate) budget_stopped: bool,
}

#[cfg(feature = "nisar-subset-live-acceptance")]
type ScienceBodyInterruptionHook =
    std::sync::Arc<dyn Fn(u64, &WorkerControl) -> bool + Send + Sync + 'static>;

type ScienceAssetRefresh<'a> = dyn FnMut(
        &ResolvedAsset,
        &WorkerControl,
    ) -> Result<ResolvedAsset, NisarSubsetScienceTransferErrorV2>
    + 'a;

struct NisarSubsetExactRangeFetchContextV2<'a> {
    plan: &'a NisarSubsetPhysicalExecutionPlanV2,
    remote_source: &'a NisarSubsetScienceRemoteSourceV2,
    range: &'a NisarSubsetPhysicalRangeV2,
    control: &'a WorkerControl,
    reserved_request_count: u64,
}

pub(crate) struct NisarSubsetScienceRangeExecutorV2 {
    http: std::sync::Arc<HttpTransfer>,
    asset: std::sync::Arc<ResolvedAsset>,
    body_activity: Option<crate::transfer_activity::TaskBodyActivity>,
    #[cfg(test)]
    after_commit_hook: Option<std::sync::Arc<dyn Fn(usize) + Send + Sync>>,
    #[cfg(test)]
    before_final_revalidation_hook: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    after_body_read_hook: Option<std::sync::Arc<dyn Fn(u64) + Send + Sync>>,
    #[cfg(test)]
    body_read_limit: Option<usize>,
    #[cfg(test)]
    completed_material_hash_bytes: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    #[cfg(feature = "nisar-subset-live-acceptance")]
    after_body_read_interruption_hook: Option<ScienceBodyInterruptionHook>,
    retry_backoff: std::sync::Arc<ScienceRetryBackoff>,
    retry_wait: std::sync::Arc<ScienceRetryWait>,
    parallel_range_workers: usize,
    /// The failure that ended the last execution, recorded together with the
    /// exact error it describes immediately before that error is returned.
    last_failure: std::sync::Mutex<
        Option<(
            NisarSubsetScienceTransferErrorV2,
            NisarScienceFailureDiagnosticV2,
        )>,
    >,
}

impl NisarSubsetScienceRangeExecutorV2 {
    pub(crate) fn new(
        http: impl Into<std::sync::Arc<HttpTransfer>>,
        asset: impl Into<std::sync::Arc<ResolvedAsset>>,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        let http = http.into();
        let asset = asset.into();
        if asset.method() != ResolvedMethod::Get {
            return Err(NisarSubsetScienceTransferErrorV2::RangeUnsupported);
        }
        Ok(Self {
            http,
            asset,
            body_activity: None,
            #[cfg(test)]
            after_commit_hook: None,
            #[cfg(test)]
            before_final_revalidation_hook: None,
            #[cfg(test)]
            after_body_read_hook: None,
            #[cfg(test)]
            body_read_limit: None,
            #[cfg(test)]
            completed_material_hash_bytes: None,
            #[cfg(feature = "nisar-subset-live-acceptance")]
            after_body_read_interruption_hook: None,
            retry_backoff: std::sync::Arc::new(science_retry_backoff),
            retry_wait: std::sync::Arc::new(wait_for_science_retry),
            parallel_range_workers: if cfg!(test) { 1 } else { 4 },
            last_failure: std::sync::Mutex::new(None),
        })
    }

    /// Sets the maximum bounded worker threads (`1..=4`) used to fetch independent Range V2 slices concurrently.
    #[allow(dead_code)]
    pub(crate) fn with_parallel_range_workers(mut self, workers: usize) -> Self {
        self.parallel_range_workers = workers.clamp(1, 4);
        self
    }

    /// Replaces the interruptible backoff wait in tests, so the requested
    /// delays are observed without sleeping (review 2026-10-17 round 2, P2-4).
    #[cfg(test)]
    pub(crate) fn with_retry_wait_for_test(
        mut self,
        wait: std::sync::Arc<ScienceRetryWait>,
    ) -> Self {
        self.retry_wait = wait;
        self
    }

    /// Replaces the 5 s / 20 s / 30 s retry backoff in tests.
    #[cfg(test)]
    pub(crate) fn with_retry_backoff_for_test(
        mut self,
        backoff: std::sync::Arc<ScienceRetryBackoff>,
    ) -> Self {
        self.retry_backoff = backoff;
        self
    }

    fn clear_failure(&self) {
        if let Ok(mut slot) = self.last_failure.lock() {
            *slot = None;
        }
    }

    /// Records the diagnostic of the failure that ends this execution and
    /// returns that error. Review 2026-10-17 (round 2, P1-1): only this call
    /// records a diagnostic, so a Range that recovered after a transient
    /// failure never leaves one behind for a later, unrelated error.
    fn end_with_failure(
        &self,
        error: NisarSubsetScienceTransferErrorV2,
        diagnostic: NisarScienceFailureDiagnosticV2,
    ) -> NisarSubsetScienceTransferErrorV2 {
        if let Ok(mut slot) = self.last_failure.lock() {
            *slot = Some((error, diagnostic));
        }
        error
    }

    /// The real cause of the failed attempt that ended the last execution.
    #[cfg(test)]
    pub(crate) fn last_failure_diagnostic(&self) -> Option<NisarScienceFailureDiagnosticV2> {
        self.last_failure
            .lock()
            .map(|slot| slot.as_ref().map(|(_, diagnostic)| diagnostic.clone()))
            .unwrap_or(None)
    }

    /// The diagnostic of the last execution only when it describes exactly
    /// `error`, the error that execution returned (same Range, same class).
    pub(crate) fn failure_diagnostic_for(
        &self,
        error: NisarSubsetScienceTransferErrorV2,
    ) -> Option<NisarScienceFailureDiagnosticV2> {
        self.last_failure
            .lock()
            .map(|slot| {
                slot.as_ref()
                    .filter(|(recorded, _)| *recorded == error)
                    .map(|(_, diagnostic)| diagnostic.clone())
            })
            .unwrap_or(None)
    }

    pub(crate) fn with_body_activity(
        mut self,
        activity: crate::transfer_activity::TaskBodyActivity,
    ) -> Self {
        self.body_activity = Some(activity);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_commit_hook_for_test(
        mut self,
        hook: std::sync::Arc<dyn Fn(usize) + Send + Sync>,
    ) -> Self {
        self.after_commit_hook = Some(hook);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_before_final_revalidation_hook_for_test(
        mut self,
        hook: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        self.before_final_revalidation_hook = Some(hook);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_after_body_read_hook_for_test(
        mut self,
        read_limit: usize,
        hook: std::sync::Arc<dyn Fn(u64) + Send + Sync>,
    ) -> Self {
        assert!(read_limit > 0 && read_limit <= SCIENCE_READ_BUFFER_BYTES);
        self.body_read_limit = Some(read_limit);
        self.after_body_read_hook = Some(hook);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_completed_material_hash_counter_for_test(
        mut self,
        counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        self.completed_material_hash_bytes = Some(counter);
        self
    }

    fn verify_completed_material_for_execution(
        &self,
        part: &mut File,
        checkpoint: &NisarSubsetScienceCheckpointV2,
        stop: MaterialDigestStopV2<'_>,
    ) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        #[cfg(test)]
        {
            verify_completed_material_with_observer(
                part,
                checkpoint,
                |bytes| {
                    if let Some(counter) = &self.completed_material_hash_bytes {
                        counter.fetch_add(bytes, std::sync::atomic::Ordering::SeqCst);
                    }
                },
                stop,
            )
        }
        #[cfg(not(test))]
        {
            verify_completed_material_with_observer(part, checkpoint, |_| {}, stop)
        }
    }

    #[cfg(feature = "nisar-subset-live-acceptance")]
    pub(crate) fn with_after_body_read_interruption_hook_for_acceptance(
        mut self,
        hook: ScienceBodyInterruptionHook,
    ) -> Self {
        self.after_body_read_interruption_hook = Some(hook);
        self
    }

    pub(crate) fn execute(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
    ) -> Result<NisarSubsetScienceTransferResultV2, NisarSubsetScienceTransferErrorV2> {
        self.execute_with_progress(work, completed, authority, control, &mut |_| {})
    }

    pub(crate) fn execute_with_progress(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(NisarScienceTransferProgressV2),
    ) -> Result<NisarSubsetScienceTransferResultV2, NisarSubsetScienceTransferErrorV2> {
        self.execute_internal(work, completed, authority, control, transfer_progress, None)
    }

    pub(crate) fn execute_with_progress_and_auth_refresh(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(NisarScienceTransferProgressV2),
        auth_refresh: &mut ScienceAssetRefresh<'_>,
    ) -> Result<NisarSubsetScienceTransferResultV2, NisarSubsetScienceTransferErrorV2> {
        self.execute_internal(
            work,
            completed,
            authority,
            control,
            transfer_progress,
            Some(auth_refresh),
        )
    }

    fn execute_internal(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(NisarScienceTransferProgressV2),
        auth_refresh: Option<&mut ScienceAssetRefresh<'_>>,
    ) -> Result<NisarSubsetScienceTransferResultV2, NisarSubsetScienceTransferErrorV2> {
        self.clear_failure();
        let result = self.execute_attempts(
            work,
            completed,
            authority,
            control,
            transfer_progress,
            auth_refresh,
        );
        if result.is_ok() {
            self.clear_failure();
        }
        result
    }

    fn execute_attempts(
        &self,
        work: &DownloadWork,
        completed: &NisarSubsetPlanningCompleteV2,
        authority: &dyn NisarSubsetScienceAuthorityV2,
        control: &WorkerControl,
        transfer_progress: &mut dyn FnMut(NisarScienceTransferProgressV2),
        mut auth_refresh: Option<&mut ScienceAssetRefresh<'_>>,
    ) -> Result<NisarSubsetScienceTransferResultV2, NisarSubsetScienceTransferErrorV2> {
        #[cfg(test)]
        let _hash_site = completed_material_hash_ledger::enter("executor");
        // Downloader enhancement D2: a pause or cancellation ends the
        // continuation digests of the completed material at once.
        let stop = worker_digest_stop(control);
        let (plan, _) = authority
            .revalidate(work, completed, control)
            .map_err(|error| map_planning_error(error, control))?;
        authority.remote_source().validate(work, completed, &plan)?;
        let mut current_asset = std::sync::Arc::clone(&self.asset);
        validate_asset_binding(&current_asset, &plan, authority.remote_source())?;
        let workspace = authority.workspace();
        let _workspace_lock = workspace.acquire_lock().map_err(map_workspace_error)?;
        let (workspace_canonical_path, workspace_identity) = workspace
            .bound_workspace_identity_v2()
            .map_err(map_workspace_error)?;
        let (mut part, part_identity_sha256) = workspace
            .open_science_part_v2(plan.source_size())
            .map_err(map_workspace_error)?;
        let lineage_key_sha256 = lineage_key_sha256(work, &plan)?;
        let execution_binding_sha256 = execution_binding_sha256(
            &lineage_key_sha256,
            &plan,
            authority.remote_source(),
            &workspace_canonical_path,
            &workspace_identity,
        )?;
        let anchor = authority.checkpoint_anchor();
        let local_checkpoint = workspace
            .load_science_checkpoint_v2()
            .map_err(map_workspace_error)?
            .map(|text| parse_checkpoint_canonical_json(&text))
            .transpose()?;
        let (mut checkpoint, budget) = recover_or_initialize_checkpoint(
            anchor,
            &lineage_key_sha256,
            &execution_binding_sha256,
            workspace,
            local_checkpoint,
            work,
            &plan,
            &workspace_canonical_path,
            &workspace_identity,
            &part_identity_sha256,
            &mut part,
            ScienceRecoveryBudgetModeV2::Effective,
            &stop,
        )?;
        checkpoint.validate_with_budget(
            work,
            &plan,
            &workspace_canonical_path,
            &workspace_identity,
            &lineage_key_sha256,
            &execution_binding_sha256,
            &part_identity_sha256,
            &budget,
        )?;
        self.verify_completed_material_for_execution(&mut part, &checkpoint, &stop)?;
        finalize_recovered_pending_attempt_durably(
            anchor,
            &lineage_key_sha256,
            workspace,
            &mut checkpoint,
        )?;
        let resumed_bytes = checkpoint.committed_science_response_body_bytes;
        if checkpoint.state == NisarSubsetScienceCheckpointStateV2::Complete {
            transfer_progress(NisarScienceTransferProgressV2::TransferComplete);
            let mut recovered = result_from_checkpoint(&checkpoint, &budget);
            recovered.accounting.resumed_bytes = resumed_bytes;
            return Ok(recovered);
        }
        if checkpoint.resumed_bytes != resumed_bytes {
            persist_anchored_transition(
                anchor,
                &lineage_key_sha256,
                workspace,
                &mut checkpoint,
                |checkpoint| {
                    checkpoint.resumed_bytes = resumed_bytes;
                    Ok(())
                },
            )?;
        }

        let range_count = checkpoint.planned_ranges.len();
        let mut automatic_retries = 0_u32;
        #[cfg(test)]
        let has_after_commit_hook = self.after_commit_hook.is_some();
        #[cfg(not(test))]
        let has_after_commit_hook = false;
        let effective_parallel_workers = if has_after_commit_hook {
            1
        } else {
            self.parallel_range_workers.clamp(1, 4)
        };
        let mut prefetched_slices: std::collections::HashMap<
            usize,
            NisarSubsetScienceFetchSuccessV2,
        > = std::collections::HashMap::new();

        for index in 0..checkpoint.planned_ranges.len() {
            if checkpoint
                .completed_ranges
                .iter()
                .any(|completed| usize::from(completed.index) == index)
            {
                continue;
            }
            if effective_parallel_workers > 1 && !prefetched_slices.contains_key(&index) {
                stop_if_requested(
                    control,
                    anchor,
                    &lineage_key_sha256,
                    workspace,
                    &mut checkpoint,
                )?;
                let mut batch_indices = Vec::new();
                let mut batch_bytes_needed = checkpoint
                    .reserved_science_response_body_bytes
                    .saturating_add(checkpoint.committed_planning_response_body_bytes);
                let remaining_req_slots = budget
                    .max_science_request_count()
                    .min(
                        budget
                            .max_request_count()
                            .saturating_sub(checkpoint.committed_planning_request_count),
                    )
                    .saturating_sub(checkpoint.reserved_science_request_count);
                for candidate_idx in index..checkpoint.planned_ranges.len() {
                    if batch_indices.len() >= effective_parallel_workers {
                        break;
                    }
                    if checkpoint
                        .completed_ranges
                        .iter()
                        .any(|c| usize::from(c.index) == candidate_idx)
                        || prefetched_slices.contains_key(&candidate_idx)
                    {
                        continue;
                    }
                    if let Some(planned_range) = checkpoint.planned_ranges.get(candidate_idx) {
                        if let Ok(len) = planned_range.len() {
                            let next_reqs = (batch_indices.len() as u64 + 1)
                                .saturating_mul(MAX_SCIENCE_HTTP_REQUESTS_PER_ATTEMPT);
                            let next_bytes = batch_bytes_needed.saturating_add(len);
                            if next_reqs <= remaining_req_slots
                                && next_bytes <= budget.max_response_body_bytes()
                            {
                                batch_bytes_needed = next_bytes;
                                batch_indices.push((candidate_idx, planned_range.clone()));
                            } else {
                                break;
                            }
                        }
                    }
                }
                if batch_indices.len() > 1 {
                    let batch_precheck_ok = authority
                        .revalidate_before_range(work, completed, &plan, control)
                        .is_ok()
                        && authority
                            .remote_source()
                            .validate(work, completed, &plan)
                            .is_ok()
                        && validate_asset_binding(&current_asset, &plan, authority.remote_source())
                            .is_ok();
                    if batch_precheck_ok {
                        let asset_snapshot = std::sync::Arc::clone(&current_asset);
                        let plan_ref = &plan;
                        let remote_source_ref = authority.remote_source();
                        let batch_results: Vec<(
                            usize,
                            Result<
                                NisarSubsetScienceFetchSuccessV2,
                                NisarSubsetScienceFetchFailureV2,
                            >,
                        )> = {
                            let progress_bytes = std::sync::atomic::AtomicU64::new(0);
                            let done_workers = std::sync::atomic::AtomicUsize::new(0);
                            let total_workers = batch_indices.len();
                            let out = std::thread::scope(|scope| {
                                let handles: Vec<_> = batch_indices
                                    .iter()
                                    .map(|(slice_idx, slice_range)| {
                                        let slice_idx = *slice_idx;
                                        let asset_ref = &asset_snapshot;
                                        let progress_ref = &progress_bytes;
                                        let done_ref = &done_workers;
                                        scope.spawn(move || {
                                            let res = self.fetch_exact_range_core(
                                                asset_ref,
                                                None,
                                                NisarSubsetExactRangeFetchContextV2 {
                                                    plan: plan_ref,
                                                    remote_source: remote_source_ref,
                                                    range: slice_range,
                                                    control,
                                                    reserved_request_count:
                                                        MAX_SCIENCE_HTTP_REQUESTS_PER_ATTEMPT,
                                                },
                                                &mut |evt| {
                                                    if let NisarScienceTransferProgressV2::Bytes(
                                                        delta,
                                                    ) = evt
                                                    {
                                                        progress_ref.fetch_add(
                                                            delta,
                                                            std::sync::atomic::Ordering::Relaxed,
                                                        );
                                                    }
                                                },
                                            );
                                            done_ref
                                                .fetch_add(1, std::sync::atomic::Ordering::Release);
                                            (slice_idx, res)
                                        })
                                    })
                                    .collect();
                                let mut emitted = 0_u64;
                                while done_workers.load(std::sync::atomic::Ordering::Acquire)
                                    < total_workers
                                {
                                    let current =
                                        progress_bytes.load(std::sync::atomic::Ordering::Relaxed);
                                    if current > emitted {
                                        transfer_progress(NisarScienceTransferProgressV2::Bytes(
                                            current - emitted,
                                        ));
                                        emitted = current;
                                    }
                                    std::thread::sleep(std::time::Duration::from_millis(15));
                                }
                                let final_bytes =
                                    progress_bytes.load(std::sync::atomic::Ordering::Relaxed);
                                if final_bytes > emitted {
                                    transfer_progress(NisarScienceTransferProgressV2::Bytes(
                                        final_bytes - emitted,
                                    ));
                                }
                                handles
                                    .into_iter()
                                    .map(|h| h.join().expect("NISAR Range V2 worker panicked"))
                                    .collect()
                            });
                            out
                        };
                        if control.is_cancelled() || control.is_paused() {
                            stop_if_requested(
                                control,
                                anchor,
                                &lineage_key_sha256,
                                workspace,
                                &mut checkpoint,
                            )?;
                        }
                        for (slice_idx, res) in batch_results {
                            if let Ok(success) = res {
                                prefetched_slices.insert(slice_idx, success);
                            }
                        }
                    }
                }
            }

            // Design 2026-10-17 §3: a transient failure is settled durably first,
            // then this Range is retried with every per-attempt check (Store
            // revalidation, binding, part identity, a new durable reservation).
            let mut attempts = 0_u32;
            let mut last_transient: Option<NisarScienceFailureDiagnosticV2> = None;
            let (success, planned, expected) = loop {
                attempts += 1;
                stop_if_requested(
                    control,
                    anchor,
                    &lineage_key_sha256,
                    workspace,
                    &mut checkpoint,
                )?;
                drop(part);
                match authority.revalidate_before_range(work, completed, &plan, control) {
                    Ok(()) => {}
                    Err(error)
                        if error == NisarSubsetPlanningErrorV2::ExecutionStopped
                            || control.is_cancelled()
                            || control.is_paused() =>
                    {
                        // Store can reject execution before the scheduler propagates
                        // PauseTask to WorkerControl. Persist that authoritative stop
                        // even while the process-local flag is still false.
                        persist_anchored_transition(
                            anchor,
                            &lineage_key_sha256,
                            workspace,
                            &mut checkpoint,
                            |checkpoint| {
                                checkpoint.state = NisarSubsetScienceCheckpointStateV2::Interrupted;
                                Ok(())
                            },
                        )?;
                        return Err(if control.is_cancelled() || control.is_paused() {
                            stop_error(control)
                        } else {
                            map_planning_error(error, control)
                        });
                    }
                    Err(error) => return Err(map_planning_error(error, control)),
                }
                authority.remote_source().validate(work, completed, &plan)?;
                validate_asset_binding(&current_asset, &plan, authority.remote_source())?;
                let (reopened_part, reopened_identity) = workspace
                    .open_science_part_v2(plan.source_size())
                    .map_err(map_workspace_error)?;
                if reopened_identity != part_identity_sha256 {
                    return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
                }
                part = reopened_part;
                let planned = checkpoint
                    .planned_ranges
                    .get(index)
                    .cloned()
                    .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
                let expected = planned
                    .len()
                    .map_err(|_| NisarSubsetScienceTransferErrorV2::AuthorityMismatch)?;
                let reserved_request_count = match reserve_budget(&budget, &checkpoint, expected) {
                    Ok(count) => count,
                    Err(error) => {
                        // A due retry that cannot be reserved stops before sending;
                        // the diagnostic keeps the last real cause.
                        return Err(match last_transient.take() {
                            Some(mut last) => {
                                last.budget_stopped = true;
                                self.end_with_failure(error, last)
                            }
                            None => error,
                        });
                    }
                };
                persist_anchored_transition(
                    anchor,
                    &lineage_key_sha256,
                    workspace,
                    &mut checkpoint,
                    |checkpoint| {
                        begin_science_attempt(checkpoint, index, &planned, reserved_request_count)
                    },
                )?;
                if let Some(prefetched) = prefetched_slices.remove(&index) {
                    break (prefetched, planned, expected);
                }
                match self.fetch_exact_range(
                    &mut current_asset,
                    NisarSubsetExactRangeFetchContextV2 {
                        plan: &plan,
                        remote_source: authority.remote_source(),
                        range: &planned,
                        control,
                        reserved_request_count,
                    },
                    transfer_progress,
                    auth_refresh.as_deref_mut(),
                ) {
                    Ok(success) => break (success, planned, expected),
                    Err(failure) => {
                        let error = failure.error;
                        persist_anchored_transition(
                            anchor,
                            &lineage_key_sha256,
                            workspace,
                            &mut checkpoint,
                            |checkpoint| record_failed_science_attempt(checkpoint, &failure),
                        )?;
                        if matches!(
                            error,
                            NisarSubsetScienceTransferErrorV2::Paused
                                | NisarSubsetScienceTransferErrorV2::Cancelled
                        ) {
                            return Err(error);
                        }
                        let diagnostic = NisarScienceFailureDiagnosticV2 {
                            range_index: index,
                            range_count,
                            attempts,
                            automatic_retries,
                            http_status: failure.http_status,
                            transport_cause: failure.transport_cause,
                            transient: failure.is_transient(),
                            auth_refresh_failure: matches!(
                                error,
                                NisarSubsetScienceTransferErrorV2::AuthRefreshFailed
                                    | NisarSubsetScienceTransferErrorV2::AuthRefreshTransient
                            ),
                            budget_stopped: false,
                        };
                        if !diagnostic.transient
                            || attempts >= SCIENCE_RANGE_MAX_ATTEMPTS
                            || automatic_retries >= SCIENCE_EXECUTION_MAX_AUTOMATIC_RETRIES
                        {
                            return Err(self.end_with_failure(error, diagnostic));
                        }
                        automatic_retries += 1;
                        // A pause or cancel during the backoff takes effect at once;
                        // the failed attempt is already settled as Interrupted.
                        if (self.retry_wait)(
                            (self.retry_backoff)(attempts, failure.http_status),
                            control,
                        ) {
                            return Err(stop_error(control));
                        }
                        last_transient = Some(diagnostic);
                    }
                }
            };
            persist_anchored_transition(
                anchor,
                &lineage_key_sha256,
                workspace,
                &mut checkpoint,
                |checkpoint| record_successful_science_response(checkpoint, &success),
            )?;
            let body = success.body;
            if body.len() as u64 != expected {
                return Err(NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete);
            }
            let response_sha256 = sha256(&body);
            part.seek(SeekFrom::Start(planned.start()))
                .and_then(|_| part.write_all(&body))
                .and_then(|_| part.sync_data())
                .map_err(|_| NisarSubsetScienceTransferErrorV2::Persistence)?;
            let allocated_range_evidence = science_allocated_range_evidence_v2(
                &mut part,
                planned.start(),
                planned.end_exclusive(),
            )
            .map_err(map_allocation_error)?;
            if !allocated_range_evidence.covers_exactly(planned.start(), planned.end_exclusive()) {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
            if digest_file_range(
                &mut part,
                planned.start(),
                planned.end_exclusive(),
                &digest_runs_to_the_end,
            )? != response_sha256
            {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
            persist_anchored_transition(
                anchor,
                &lineage_key_sha256,
                workspace,
                &mut checkpoint,
                |checkpoint| {
                    let pending = checkpoint
                        .pending_attempt
                        .take()
                        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
                    if pending.state != NisarSubsetSciencePendingAttemptStateV2::ResponseRecorded
                        || usize::from(pending.range_index) != index
                        || !pending.request_issued
                        || pending.observed_response_body_bytes != expected
                    {
                        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
                    }
                    checkpoint.completed_ranges.push(CompletedScienceRangeV2 {
                        index: u16::try_from(index)
                            .map_err(|_| NisarSubsetScienceTransferErrorV2::AuthorityMismatch)?,
                        start: planned.start(),
                        end_exclusive: planned.end_exclusive(),
                        request_count: success.request_count,
                        response_sha256,
                        allocated_range_evidence,
                    });
                    checkpoint.committed_science_request_count = checkpoint
                        .committed_science_request_count
                        .checked_add(success.request_count)
                        .ok_or(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)?;
                    checkpoint.committed_science_response_body_bytes = checkpoint
                        .committed_science_response_body_bytes
                        .checked_add(expected)
                        .ok_or(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)?;
                    checkpoint.state =
                        if checkpoint.completed_ranges.len() == checkpoint.planned_ranges.len() {
                            NisarSubsetScienceCheckpointStateV2::Complete
                        } else {
                            NisarSubsetScienceCheckpointStateV2::InProgress
                        };
                    Ok(())
                },
            )?;
            checkpoint.validate_with_budget(
                work,
                &plan,
                &workspace_canonical_path,
                &workspace_identity,
                &lineage_key_sha256,
                &execution_binding_sha256,
                &part_identity_sha256,
                &budget,
            )?;
            #[cfg(test)]
            if let Some(hook) = &self.after_commit_hook {
                hook(index);
            }
        }
        if checkpoint.state != NisarSubsetScienceCheckpointStateV2::Complete {
            return Err(NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete);
        }
        transfer_progress(NisarScienceTransferProgressV2::TransferComplete);
        drop(part);
        #[cfg(test)]
        if let Some(hook) = &self.before_final_revalidation_hook {
            hook();
        }
        let (current, _) = authority
            .revalidate(work, completed, control)
            .map_err(|error| map_planning_error(error, control))?;
        if current != plan {
            return Err(NisarSubsetScienceTransferErrorV2::AuthorityMismatch);
        }
        authority
            .remote_source()
            .validate(work, completed, &current)?;
        validate_asset_binding(&current_asset, &current, authority.remote_source())?;
        let persisted_checkpoint = workspace
            .load_science_checkpoint_v2()
            .map_err(map_workspace_error)?
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
            .and_then(|canonical| parse_checkpoint_canonical_json(&canonical))?;
        if persisted_checkpoint != checkpoint {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        reconcile_checkpoint_anchor(anchor, &lineage_key_sha256, &checkpoint)?;
        let (mut part, reopened_identity) = workspace
            .open_science_part_v2(plan.source_size())
            .map_err(map_workspace_error)?;
        if reopened_identity != part_identity_sha256 {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        self.verify_completed_material_for_execution(&mut part, &checkpoint, &stop)?;
        Ok(result_from_checkpoint(&checkpoint, &budget))
    }

    fn fetch_exact_range(
        &self,
        current_asset: &mut std::sync::Arc<ResolvedAsset>,
        context: NisarSubsetExactRangeFetchContextV2<'_>,
        transfer_progress: &mut dyn FnMut(NisarScienceTransferProgressV2),
        auth_refresh: Option<&mut ScienceAssetRefresh<'_>>,
    ) -> Result<NisarSubsetScienceFetchSuccessV2, NisarSubsetScienceFetchFailureV2> {
        let initial_asset = std::sync::Arc::clone(current_asset);
        self.fetch_exact_range_core(
            &initial_asset,
            auth_refresh.map(|refresh| (current_asset, refresh)),
            context,
            transfer_progress,
        )
    }

    fn fetch_exact_range_core(
        &self,
        initial_asset: &std::sync::Arc<ResolvedAsset>,
        auth_refresh: Option<(
            &mut std::sync::Arc<ResolvedAsset>,
            &mut ScienceAssetRefresh<'_>,
        )>,
        context: NisarSubsetExactRangeFetchContextV2<'_>,
        transfer_progress: &mut dyn FnMut(NisarScienceTransferProgressV2),
    ) -> Result<NisarSubsetScienceFetchSuccessV2, NisarSubsetScienceFetchFailureV2> {
        let NisarSubsetExactRangeFetchContextV2 {
            plan,
            remote_source,
            range,
            control,
            reserved_request_count,
        } = context;
        let start = range.start();
        let end_exclusive = range.end_exclusive();
        let source_size = plan.source_size();
        let strong_etag = plan.source_validator().value.as_str();
        let stopped = || control.is_cancelled() || control.is_paused();
        let failure_with =
            |error, request_count, observed_response_body_bytes, http_status, transport_cause| {
                Err(NisarSubsetScienceFetchFailureV2 {
                    error,
                    request_count,
                    observed_response_body_bytes,
                    http_status,
                    transport_cause,
                })
            };
        let failure = |error, request_count, observed_response_body_bytes| {
            failure_with(
                error,
                request_count,
                observed_response_body_bytes,
                None,
                None,
            )
        };
        let request_slots = Cell::new(reserved_request_count);
        let reserve_request = || {
            let Some(remaining) = request_slots.get().checked_sub(1) else {
                return false;
            };
            request_slots.set(remaining);
            true
        };
        let outcome = self.http.send_resolved_exact_range_once_with_request_gate(
            initial_asset.as_ref(),
            start,
            end_exclusive,
            strong_etag,
            &stopped,
            &reserve_request,
        );
        let (mut response, mut request_count) = match outcome {
            ExactRangeSendOutcome::Response {
                response,
                request_count,
            } => (response, request_count),
            ExactRangeSendOutcome::Cancelled { request_count } => {
                return failure(stop_error(control), request_count, 0);
            }
            ExactRangeSendOutcome::RetryableExhausted {
                request_count,
                diagnostic,
            } => {
                return failure_with(
                    NisarSubsetScienceTransferErrorV2::Transport,
                    request_count,
                    0,
                    diagnostic.http_status,
                    Some(diagnostic.cause.as_str()),
                );
            }
            ExactRangeSendOutcome::ProtocolRejected { request_count }
            | ExactRangeSendOutcome::TrustedRedirectRejected { request_count, .. } => {
                return failure(
                    NisarSubsetScienceTransferErrorV2::ResponseInvalid,
                    request_count,
                    0,
                );
            }
            #[cfg(feature = "nisar-subset-production")]
            ExactRangeSendOutcome::BudgetExceeded { request_count } => {
                return failure(
                    NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded,
                    request_count,
                    0,
                );
            }
        };
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            let Some((current_asset, auth_refresh)) = auth_refresh else {
                return failure_with(
                    NisarSubsetScienceTransferErrorV2::ResponseInvalid,
                    request_count,
                    0,
                    Some(response.status().as_u16()),
                    None,
                );
            };
            if request_slots.get() == 0 {
                return failure(
                    NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded,
                    request_count,
                    0,
                );
            }
            drop(response);
            let refreshed = match auth_refresh(current_asset.as_ref(), control) {
                Ok(asset) => asset,
                Err(error) => return failure(error, request_count, 0),
            };
            if let Err(error) = validate_asset_binding(&refreshed, plan, remote_source) {
                return failure(error, request_count, 0);
            }
            *current_asset = std::sync::Arc::new(refreshed);
            let retry = self.http.send_resolved_exact_range_once_with_request_gate(
                current_asset.as_ref(),
                start,
                end_exclusive,
                strong_etag,
                &stopped,
                &reserve_request,
            );
            let (retry_response, retry_count) = match retry {
                ExactRangeSendOutcome::Response {
                    response,
                    request_count,
                } => (response, request_count),
                ExactRangeSendOutcome::Cancelled {
                    request_count: retry_count,
                } => {
                    return failure(
                        stop_error(control),
                        request_count.saturating_add(retry_count),
                        0,
                    );
                }
                ExactRangeSendOutcome::RetryableExhausted {
                    request_count: retry_count,
                    diagnostic,
                } => {
                    return failure_with(
                        NisarSubsetScienceTransferErrorV2::Transport,
                        request_count.saturating_add(retry_count),
                        0,
                        diagnostic.http_status,
                        Some(diagnostic.cause.as_str()),
                    );
                }
                ExactRangeSendOutcome::ProtocolRejected {
                    request_count: retry_count,
                }
                | ExactRangeSendOutcome::TrustedRedirectRejected {
                    request_count: retry_count,
                    ..
                } => {
                    return failure(
                        NisarSubsetScienceTransferErrorV2::ResponseInvalid,
                        request_count.saturating_add(retry_count),
                        0,
                    );
                }
                #[cfg(feature = "nisar-subset-production")]
                ExactRangeSendOutcome::BudgetExceeded {
                    request_count: retry_count,
                } => {
                    return failure(
                        NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded,
                        request_count.saturating_add(retry_count),
                        0,
                    );
                }
            };
            request_count = request_count.saturating_add(retry_count);
            response = retry_response;
        }
        let status = Some(response.status().as_u16());
        if response.status() == StatusCode::OK {
            return failure_with(
                NisarSubsetScienceTransferErrorV2::RangeUnsupported,
                request_count,
                0,
                status,
                None,
            );
        }
        if response.status() != StatusCode::PARTIAL_CONTENT {
            return failure_with(
                NisarSubsetScienceTransferErrorV2::ResponseInvalid,
                request_count,
                0,
                status,
                None,
            );
        }
        let response_etag = match response_strong_etag(&response) {
            Ok(etag) => etag,
            Err(_) => {
                return failure(
                    NisarSubsetScienceTransferErrorV2::ResponseInvalid,
                    request_count,
                    0,
                );
            }
        };
        if response_etag != strong_etag {
            return failure(
                NisarSubsetScienceTransferErrorV2::StrongValidatorChanged,
                request_count,
                0,
            );
        }
        if validate_content_range(&response, start, end_exclusive, source_size).is_err() {
            return failure(
                NisarSubsetScienceTransferErrorV2::RangeResponseInvalid,
                request_count,
                0,
            );
        }
        let expected = end_exclusive - start;
        let capacity = match usize::try_from(expected) {
            Ok(capacity) => capacity,
            Err(_) => {
                return failure(
                    NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded,
                    request_count,
                    0,
                );
            }
        };
        let mut body = Vec::with_capacity(capacity);
        let mut body_activity = self.body_activity.as_ref().map(|activity| activity.start());
        let mut buffer = [0_u8; SCIENCE_READ_BUFFER_BYTES];
        while (body.len() as u64) < expected {
            if stopped() {
                return failure(stop_error(control), request_count, body.len() as u64);
            }
            let remaining = expected - body.len() as u64;
            let read_limit = usize::try_from(remaining.min(buffer.len() as u64))
                .expect("bounded science buffer length fits usize");
            #[cfg(test)]
            let read_limit = read_limit.min(self.body_read_limit.unwrap_or(read_limit));
            match response.read(&mut buffer[..read_limit]) {
                Ok(0) => break,
                Ok(count) => {
                    if let Some(activity) = &mut body_activity {
                        activity.observe(count);
                    }
                    body.extend_from_slice(&buffer[..count]);
                    transfer_progress(NisarScienceTransferProgressV2::Bytes(count as u64));
                    #[cfg(test)]
                    if let Some(hook) = &self.after_body_read_hook {
                        hook(body.len() as u64);
                    }
                    #[cfg(feature = "nisar-subset-live-acceptance")]
                    if let Some(hook) = &self.after_body_read_interruption_hook {
                        if hook(body.len() as u64, control) {
                            return failure(
                                NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete,
                                request_count,
                                body.len() as u64,
                            );
                        }
                    }
                }
                Err(_) => {
                    return failure(
                        NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete,
                        request_count,
                        body.len() as u64,
                    );
                }
            }
        }
        if body.len() as u64 != expected {
            return failure(
                NisarSubsetScienceTransferErrorV2::ScienceTransferIncomplete,
                request_count,
                body.len() as u64,
            );
        }
        Ok(NisarSubsetScienceFetchSuccessV2 {
            observed_response_body_bytes: body.len() as u64,
            request_count,
            body,
        })
    }
}

fn lineage_key_sha256(
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
) -> Result<String, NisarSubsetScienceTransferErrorV2> {
    digest_serializable(&(
        &work.task.task_id,
        &work.item.item_id,
        plan.prepared_intent_sha256(),
        plan.final_plan_sha256(),
        plan.physical_plan_sha256(),
    ))
}

fn frozen_authority_sha256(
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
) -> Result<String, NisarSubsetScienceTransferErrorV2> {
    digest_serializable(&(
        lineage_key_sha256(work, plan)?,
        plan.inventory_sha256(),
        plan.inventory_evidence_sha256(),
        plan.source_graph_sha256(),
        plan.source_product_identity(),
        plan.source_asset_identity(),
        plan.source_validator(),
        plan.source_size(),
    ))
}

fn execution_binding_sha256(
    lineage_key_sha256: &str,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    remote_source: &NisarSubsetScienceRemoteSourceV2,
    workspace_canonical_path: &str,
    workspace_identity: &str,
) -> Result<String, NisarSubsetScienceTransferErrorV2> {
    digest_serializable(&(
        lineage_key_sha256,
        plan.inventory_sha256(),
        plan.inventory_evidence_sha256(),
        plan.source_graph_sha256(),
        &remote_source.identity_sha256,
        workspace_canonical_path,
        workspace_identity,
    ))
}

fn validate_asset_binding(
    asset: &ResolvedAsset,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    remote_source: &NisarSubsetScienceRemoteSourceV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    let validator = plan.source_validator();
    let actual_validator = asset.remote_validator();
    let identity_matches = resolved_asset_identity_matches(asset, plan);
    let checksum_matches = match (
        plan.source_asset_identity().checksum.as_ref(),
        asset.expected_checksum(),
    ) {
        (None, None) => true,
        (Some(expected), Some(actual)) => {
            let algorithm = match expected.algorithm {
                NisarSubsetChecksumAlgorithm::Md5 => "md5",
                NisarSubsetChecksumAlgorithm::Sha1 => "sha1",
                NisarSubsetChecksumAlgorithm::Sha256 => "sha256",
            };
            actual.algorithm.eq_ignore_ascii_case(algorithm)
                && actual.value.eq_ignore_ascii_case(&expected.value)
        }
        _ => false,
    };
    if asset.method() != ResolvedMethod::Get
        || asset.effective_url().as_str() != remote_source.effective_url
        || !identity_matches
        || !checksum_matches
    {
        return Err(NisarSubsetScienceTransferErrorV2::AuthorityMismatch);
    }
    if validator.kind != NisarSubsetValidatorKind::StrongEtag
        || validator.value.starts_with("W/")
        || asset.expected_size() != Some(plan.source_size())
        || actual_validator.content_length != Some(plan.source_size())
    {
        return Err(NisarSubsetScienceTransferErrorV2::StrongValidatorChanged);
    }
    if actual_validator.etag.as_deref() != Some(validator.value.as_str()) {
        return Err(NisarSubsetScienceTransferErrorV2::StrongValidatorChanged);
    }
    Ok(())
}

pub(crate) fn resolved_asset_identity_matches(
    asset: &ResolvedAsset,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
) -> bool {
    if asset.host_policy() == Some(ResolvedHostPolicy::BhoonidhiDownloadV1) {
        let url = asset.effective_url();
        if plan.source_product_identity().provider_id != BHOONIDHI_PROVIDER_ID
            || plan.source_product_identity().collection_id != BHOONIDHI_COLLECTION_ID
            || url.scheme() != "https"
            || url.origin().ascii_serialization() != BHOONIDHI_DOWNLOAD_ORIGIN
            || url.path() != BHOONIDHI_DOWNLOAD_PATH
            || url.fragment().is_some()
        {
            return false;
        }
        let pairs = url.query_pairs().collect::<Vec<_>>();
        return pairs.len() == 2
            && pairs[0].0 == "id"
            && pairs[0].1 == plan.source_product_identity().provider_product_id
            && pairs[1].0 == "collection"
            && pairs[1].1 == plan.source_product_identity().collection_id;
    }

    asset
        .effective_url()
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        == Some(plan.source_asset_identity().file_name.as_str())
}

fn reserve_budget(
    budget: &NisarValidatedScienceBudgetV2,
    checkpoint: &NisarSubsetScienceCheckpointV2,
    expected_response_body_bytes: u64,
) -> Result<u64, NisarSubsetScienceTransferErrorV2> {
    validate_budget_reservation_values(
        budget.max_request_count(),
        budget.max_response_body_bytes(),
        budget.max_science_request_count(),
        checkpoint.committed_planning_request_count,
        checkpoint.committed_planning_response_body_bytes,
        checkpoint.reserved_science_request_count,
        checkpoint.reserved_science_response_body_bytes,
        expected_response_body_bytes,
    )?;
    let remaining = budget
        .max_science_request_count()
        .min(
            budget
                .max_request_count()
                .checked_sub(checkpoint.committed_planning_request_count)
                .ok_or(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)?,
        )
        .checked_sub(checkpoint.reserved_science_request_count)
        .ok_or(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)?;
    let allowance = remaining.min(MAX_SCIENCE_HTTP_REQUESTS_PER_ATTEMPT);
    if allowance == 0 {
        return Err(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded);
    }
    Ok(allowance)
}

fn begin_science_attempt(
    checkpoint: &mut NisarSubsetScienceCheckpointV2,
    range_index: usize,
    planned: &NisarSubsetPhysicalRangeV2,
    reserved_request_count: u64,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    if checkpoint.pending_attempt.is_some()
        || reserved_request_count == 0
        || reserved_request_count > MAX_SCIENCE_HTTP_REQUESTS_PER_ATTEMPT
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let reserved_response_body_bytes = planned
        .len()
        .map_err(|_| NisarSubsetScienceTransferErrorV2::AuthorityMismatch)?;
    checkpoint.reserved_science_request_count = checkpoint
        .reserved_science_request_count
        .checked_add(reserved_request_count)
        .ok_or(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)?;
    checkpoint.reserved_science_response_body_bytes = checkpoint
        .reserved_science_response_body_bytes
        .checked_add(reserved_response_body_bytes)
        .ok_or(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)?;
    checkpoint.pending_attempt = Some(NisarSubsetSciencePendingAttemptV2 {
        range_index: u16::try_from(range_index)
            .map_err(|_| NisarSubsetScienceTransferErrorV2::AuthorityMismatch)?,
        start: planned.start(),
        end_exclusive: planned.end_exclusive(),
        reserved_response_body_bytes,
        reserved_request_count: Some(reserved_request_count),
        state: NisarSubsetSciencePendingAttemptStateV2::Reserved,
        request_issued: false,
        request_count: 0,
        observed_response_body_bytes: 0,
    });
    checkpoint.state = NisarSubsetScienceCheckpointStateV2::InProgress;
    Ok(())
}

fn record_failed_science_attempt(
    checkpoint: &mut NisarSubsetScienceCheckpointV2,
    failure: &NisarSubsetScienceFetchFailureV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    let pending = checkpoint
        .pending_attempt
        .take()
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    if pending.state != NisarSubsetSciencePendingAttemptStateV2::Reserved
        || failure.observed_response_body_bytes > pending.reserved_response_body_bytes
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    checkpoint.reserved_science_request_count = settled_request_reservation(
        checkpoint.reserved_science_request_count,
        &pending,
        failure.request_count,
    )?;
    if failure.request_count > 0 {
        checkpoint.issued_science_request_count = checkpoint
            .issued_science_request_count
            .checked_add(failure.request_count)
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        checkpoint.failed_science_request_count = checkpoint
            .failed_science_request_count
            .checked_add(failure.request_count)
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    } else {
        checkpoint.unissued_science_reservation_count = checkpoint
            .unissued_science_reservation_count
            .checked_add(1)
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    }
    checkpoint.observed_science_response_body_bytes = checkpoint
        .observed_science_response_body_bytes
        .checked_add(failure.observed_response_body_bytes)
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    checkpoint.state = NisarSubsetScienceCheckpointStateV2::Interrupted;
    Ok(())
}

fn record_successful_science_response(
    checkpoint: &mut NisarSubsetScienceCheckpointV2,
    success: &NisarSubsetScienceFetchSuccessV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    let pending = checkpoint
        .pending_attempt
        .as_mut()
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    if pending.state != NisarSubsetSciencePendingAttemptStateV2::Reserved
        || success.request_count == 0
        || success.observed_response_body_bytes != pending.reserved_response_body_bytes
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    checkpoint.reserved_science_request_count = settled_request_reservation(
        checkpoint.reserved_science_request_count,
        pending,
        success.request_count,
    )?;
    checkpoint.issued_science_request_count = checkpoint
        .issued_science_request_count
        .checked_add(success.request_count)
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    checkpoint.observed_science_response_body_bytes = checkpoint
        .observed_science_response_body_bytes
        .checked_add(success.observed_response_body_bytes)
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    pending.state = NisarSubsetSciencePendingAttemptStateV2::ResponseRecorded;
    pending.request_issued = true;
    pending.request_count = success.request_count;
    pending.observed_response_body_bytes = success.observed_response_body_bytes;
    Ok(())
}

fn settled_request_reservation(
    reserved_total: u64,
    pending: &NisarSubsetSciencePendingAttemptV2,
    actual_requests: u64,
) -> Result<u64, NisarSubsetScienceTransferErrorV2> {
    let allowance = pending
        .reserved_request_count
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    if allowance == 0 || allowance > MAX_SCIENCE_HTTP_REQUESTS_PER_ATTEMPT {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let unused = allowance
        .checked_sub(actual_requests.max(1))
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    reserved_total
        .checked_sub(unused)
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
}

fn finalize_recovered_pending_attempt(
    checkpoint: &mut NisarSubsetScienceCheckpointV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    let Some(pending) = checkpoint.pending_attempt.take() else {
        return Ok(());
    };
    match pending.state {
        NisarSubsetSciencePendingAttemptStateV2::Reserved => {
            checkpoint.indeterminate_science_request_count = checkpoint
                .indeterminate_science_request_count
                .checked_add(pending.reserved_request_count.unwrap_or(1))
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
            checkpoint.indeterminate_science_response_body_bytes = checkpoint
                .indeterminate_science_response_body_bytes
                .checked_add(pending.reserved_response_body_bytes)
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        }
        NisarSubsetSciencePendingAttemptStateV2::ResponseRecorded => {
            checkpoint.failed_science_request_count = checkpoint
                .failed_science_request_count
                .checked_add(pending.request_count)
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        }
    }
    checkpoint.state = NisarSubsetScienceCheckpointStateV2::Interrupted;
    Ok(())
}

/// The one recovery rule for an attempt left pending by a crash: charge it as
/// indeterminate (or failed when its response was recorded) through an anchored
/// transition. Counters are reclassified, never refunded.
fn finalize_recovered_pending_attempt_durably(
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    lineage_key_sha256: &str,
    workspace: &Hdf5RangeWorkspaceV1,
    checkpoint: &mut NisarSubsetScienceCheckpointV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    if checkpoint.pending_attempt.is_none() {
        return Ok(());
    }
    persist_anchored_transition(
        anchor,
        lineage_key_sha256,
        workspace,
        checkpoint,
        finalize_recovered_pending_attempt,
    )
}

// Keep frozen limits and charged counters explicit for checked-arithmetic review.
#[allow(clippy::too_many_arguments)]
fn validate_budget_reservation_values(
    max_request_count: u64,
    max_response_body_bytes: u64,
    max_science_request_count: u64,
    committed_planning_request_count: u64,
    committed_planning_response_body_bytes: u64,
    committed_science_request_count: u64,
    committed_science_response_body_bytes: u64,
    expected_response_body_bytes: u64,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    if expected_response_body_bytes == 0
        || committed_science_request_count
            .checked_add(1)
            .is_none_or(|count| count > max_science_request_count)
        || committed_planning_request_count
            .checked_add(committed_science_request_count)
            .and_then(|total| total.checked_add(1))
            .is_none_or(|total| total > max_request_count)
        || committed_planning_response_body_bytes
            .checked_add(committed_science_response_body_bytes)
            .and_then(|total| total.checked_add(expected_response_body_bytes))
            .is_none_or(|total| total > max_response_body_bytes)
    {
        return Err(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded);
    }
    Ok(())
}

fn verify_completed_material(
    part: &mut File,
    checkpoint: &NisarSubsetScienceCheckpointV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    verify_completed_material_with_observer(part, checkpoint, |_| {}, &digest_runs_to_the_end)
}

fn verify_completed_material_with_observer(
    part: &mut File,
    checkpoint: &NisarSubsetScienceCheckpointV2,
    mut observe_hashed_bytes: impl FnMut(u64),
    stop: MaterialDigestStopV2<'_>,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    for completed in &checkpoint.completed_ranges {
        validate_science_allocated_range_evidence_v2(
            part,
            completed.start,
            completed.end_exclusive,
            &completed.allocated_range_evidence,
        )
        .map_err(map_allocation_error)?;
        let hashed_bytes = completed
            .end_exclusive
            .checked_sub(completed.start)
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        observe_hashed_bytes(hashed_bytes);
        #[cfg(test)]
        completed_material_hash_ledger::record(&checkpoint.workspace_canonical_path, hashed_bytes);
        let digest = digest_file_range(part, completed.start, completed.end_exclusive, stop)?;
        if digest != completed.response_sha256 {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
    }
    Ok(())
}

/// SHA-256 of `file[start..end_exclusive]`. `stop` is asked before every
/// read (downloader enhancement D2), so a pause ends a long digest at once.
fn digest_file_range(
    file: &mut File,
    start: u64,
    end_exclusive: u64,
    stop: MaterialDigestStopV2<'_>,
) -> Result<String, NisarSubsetScienceTransferErrorV2> {
    if start >= end_exclusive {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    let mut digest = Sha256::new();
    let mut remaining = end_exclusive - start;
    let mut buffer = [0_u8; SCIENCE_READ_BUFFER_BYTES];
    while remaining > 0 {
        #[cfg(test)]
        material_digest_hook_for_test::before_read();
        if let Some(error) = stop() {
            #[cfg(test)]
            material_digest_hook_for_test::after_stop();
            return Err(error);
        }
        let count = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        file.read_exact(&mut buffer[..count])
            .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn checkpoint_canonical_json(
    checkpoint: &NisarSubsetScienceCheckpointV2,
) -> Result<String, NisarSubsetScienceTransferErrorV2> {
    let canonical = serde_json::to_string(checkpoint)
        .map_err(|_| NisarSubsetScienceTransferErrorV2::Persistence)?;
    if canonical.is_empty() || canonical.len() > SCIENCE_CHECKPOINT_MAX_CANONICAL_BYTES {
        return Err(NisarSubsetScienceTransferErrorV2::Persistence);
    }
    Ok(canonical)
}

fn parse_checkpoint_canonical_json(
    canonical: &str,
) -> Result<NisarSubsetScienceCheckpointV2, NisarSubsetScienceTransferErrorV2> {
    if canonical.is_empty() || canonical.len() > SCIENCE_CHECKPOINT_MAX_CANONICAL_BYTES {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let checkpoint: NisarSubsetScienceCheckpointV2 = serde_json::from_str(canonical)
        .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    if checkpoint_canonical_json(&checkpoint)
        .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?
        != canonical
        || checkpoint.recompute_sha256()? != checkpoint.checkpoint_sha256
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    Ok(checkpoint)
}

fn persist_checkpoint(
    workspace: &Hdf5RangeWorkspaceV1,
    checkpoint: &NisarSubsetScienceCheckpointV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    workspace
        .persist_science_checkpoint_v2(&checkpoint_canonical_json(checkpoint)?)
        .map_err(|_| NisarSubsetScienceTransferErrorV2::Persistence)
}

#[allow(clippy::too_many_arguments)]
fn validate_recovery_checkpoint(
    checkpoint: &NisarSubsetScienceCheckpointV2,
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    workspace_canonical_path: &str,
    workspace_identity: &str,
    lineage_key_sha256: &str,
    execution_binding_sha256: &str,
    part_identity_sha256: &str,
    part: &mut File,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    checkpoint.validate(
        work,
        plan,
        workspace_canonical_path,
        workspace_identity,
        lineage_key_sha256,
        execution_binding_sha256,
        part_identity_sha256,
    )?;
    verify_completed_material(part, checkpoint)
}

#[allow(clippy::too_many_arguments)]
fn validate_recovery_checkpoint_with_budget(
    checkpoint: &NisarSubsetScienceCheckpointV2,
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    workspace_canonical_path: &str,
    workspace_identity: &str,
    lineage_key_sha256: &str,
    execution_binding_sha256: &str,
    part_identity_sha256: &str,
    part: &mut File,
    budget: &NisarValidatedScienceBudgetV2,
    stop: MaterialDigestStopV2<'_>,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    checkpoint.validate_with_budget(
        work,
        plan,
        workspace_canonical_path,
        workspace_identity,
        lineage_key_sha256,
        execution_binding_sha256,
        part_identity_sha256,
        budget,
    )?;
    verify_completed_material_with_observer(part, checkpoint, |_| {}, stop)
}

#[allow(clippy::too_many_arguments)]
fn recover_or_initialize_checkpoint(
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    lineage_key_sha256: &str,
    execution_binding_sha256: &str,
    workspace: &Hdf5RangeWorkspaceV1,
    local_checkpoint: Option<NisarSubsetScienceCheckpointV2>,
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    workspace_canonical_path: &str,
    workspace_identity: &str,
    part_identity_sha256: &str,
    part: &mut File,
    budget_mode: ScienceRecoveryBudgetModeV2,
    stop: MaterialDigestStopV2<'_>,
) -> Result<
    (
        NisarSubsetScienceCheckpointV2,
        NisarValidatedScienceBudgetV2,
    ),
    NisarSubsetScienceTransferErrorV2,
> {
    let Some(record) = anchor.load(lineage_key_sha256)? else {
        // Only the effective executor may create a genesis; migration-entry
        // reconciliation always starts from an existing Store anchor.
        if budget_mode != ScienceRecoveryBudgetModeV2::Effective
            || local_checkpoint.is_some()
            || anchor.load_budget_upgrade_proof()?.is_some()
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let checkpoint = NisarSubsetScienceCheckpointV2::new(
            work,
            plan,
            workspace_canonical_path.to_owned(),
            workspace_identity.to_owned(),
            lineage_key_sha256.to_owned(),
            execution_binding_sha256.to_owned(),
            part_identity_sha256.to_owned(),
        )?;
        validate_recovery_checkpoint(
            &checkpoint,
            work,
            plan,
            workspace_canonical_path,
            workspace_identity,
            lineage_key_sha256,
            execution_binding_sha256,
            part_identity_sha256,
            part,
        )?;
        let prepared = NisarSubsetScienceCheckpointAnchorRecordV2::prepared_genesis(
            lineage_key_sha256,
            execution_binding_sha256,
            &checkpoint,
        )?;
        if !anchor.compare_and_swap(lineage_key_sha256, None, &prepared)? {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        persist_checkpoint(workspace, &checkpoint)?;
        let committed = prepared.commit_pending()?;
        if !anchor.compare_and_swap(lineage_key_sha256, Some(&prepared), &committed)? {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        return Ok((checkpoint, NisarValidatedScienceBudgetV2::original(plan)));
    };

    record.validate()?;
    if record.lineage_key_sha256 != lineage_key_sha256
        || record.execution_binding_sha256 != execution_binding_sha256
    {
        return Err(NisarSubsetScienceTransferErrorV2::AuthorityMismatch);
    }

    if let Some(pending) = &record.pending_transition {
        let target = parse_checkpoint_canonical_json(&pending.checkpoint_canonical_json)?;
        let budget = budget_upgrade::recovery_budget(
            budget_mode,
            &target,
            work,
            plan,
            workspace,
            anchor,
            workspace_canonical_path,
            workspace_identity,
            execution_binding_sha256,
            part_identity_sha256,
        )?;
        let target_predecessor_generation = target.anchor_generation.checked_sub(1);
        if target.checkpoint_sha256 != pending.checkpoint_sha256
            || target.anchor_generation != pending.checkpoint_generation
            || target.previous_checkpoint_sha256 != pending.predecessor_checkpoint_sha256
            || target_predecessor_generation != pending.predecessor_generation
            || target.lineage_key_sha256 != lineage_key_sha256
            || target.execution_binding_sha256 != execution_binding_sha256
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        validate_recovery_checkpoint_with_budget(
            &target,
            work,
            plan,
            workspace_canonical_path,
            workspace_identity,
            lineage_key_sha256,
            execution_binding_sha256,
            part_identity_sha256,
            part,
            &budget,
            stop,
        )?;
        if let Some(local) = &local_checkpoint {
            validate_recovery_checkpoint_with_budget(
                local,
                work,
                plan,
                workspace_canonical_path,
                workspace_identity,
                lineage_key_sha256,
                execution_binding_sha256,
                part_identity_sha256,
                part,
                &budget,
                stop,
            )?;
            let local_is_target = local.checkpoint_sha256 == pending.checkpoint_sha256;
            let local_is_predecessor = pending
                .predecessor_checkpoint_sha256
                .as_deref()
                .is_some_and(|digest| local.checkpoint_sha256 == digest);
            if !local_is_target && !local_is_predecessor {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
        }
        if local_checkpoint
            .as_ref()
            .is_none_or(|local| local.checkpoint_sha256 != target.checkpoint_sha256)
        {
            persist_checkpoint(workspace, &target)?;
        }
        let committed = record.commit_pending()?;
        if !anchor.compare_and_swap(lineage_key_sha256, Some(&record), &committed)? {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        return Ok((target, budget));
    }

    let local = local_checkpoint.ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    let budget = budget_upgrade::recovery_budget(
        budget_mode,
        &local,
        work,
        plan,
        workspace,
        anchor,
        workspace_canonical_path,
        workspace_identity,
        execution_binding_sha256,
        part_identity_sha256,
    )?;
    validate_recovery_checkpoint_with_budget(
        &local,
        work,
        plan,
        workspace_canonical_path,
        workspace_identity,
        lineage_key_sha256,
        execution_binding_sha256,
        part_identity_sha256,
        part,
        &budget,
        stop,
    )?;
    if record.committed_checkpoint_sha256.as_deref() != Some(&local.checkpoint_sha256)
        || record.generation != local.anchor_generation
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    Ok((local, budget))
}

fn reconcile_checkpoint_anchor(
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    lineage_key_sha256: &str,
    checkpoint: &NisarSubsetScienceCheckpointV2,
) -> Result<NisarSubsetScienceCheckpointAnchorRecordV2, NisarSubsetScienceTransferErrorV2> {
    if checkpoint.recompute_sha256()? != checkpoint.checkpoint_sha256 {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let record = anchor
        .load(lineage_key_sha256)?
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    record.validate()?;
    if record.lineage_key_sha256 != lineage_key_sha256
        || record.execution_binding_sha256 != checkpoint.execution_binding_sha256
        || record.pending_transition.is_some()
        || record.committed_checkpoint_sha256.as_deref()
            != Some(checkpoint.checkpoint_sha256.as_str())
        || record.generation != checkpoint.anchor_generation
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    Ok(record)
}

fn persist_anchored_transition(
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    lineage_key_sha256: &str,
    workspace: &Hdf5RangeWorkspaceV1,
    checkpoint: &mut NisarSubsetScienceCheckpointV2,
    update: impl FnOnce(
        &mut NisarSubsetScienceCheckpointV2,
    ) -> Result<(), NisarSubsetScienceTransferErrorV2>,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    let current = reconcile_checkpoint_anchor(anchor, lineage_key_sha256, checkpoint)?;
    let previous_checkpoint_sha256 = checkpoint.checkpoint_sha256.clone();
    update(checkpoint)?;
    checkpoint.anchor_generation = current
        .generation
        .checked_add(1)
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    checkpoint.previous_checkpoint_sha256 = Some(previous_checkpoint_sha256);
    checkpoint.seal()?;
    let pending = current.with_pending(checkpoint)?;
    if !anchor.compare_and_swap(lineage_key_sha256, Some(&current), &pending)? {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    persist_checkpoint(workspace, checkpoint)?;
    let committed = pending.commit_pending()?;
    if !anchor.compare_and_swap(lineage_key_sha256, Some(&pending), &committed)? {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    Ok(())
}

fn stop_if_requested(
    control: &WorkerControl,
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    lineage_key_sha256: &str,
    workspace: &Hdf5RangeWorkspaceV1,
    checkpoint: &mut NisarSubsetScienceCheckpointV2,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    if !control.is_cancelled() && !control.is_paused() {
        return Ok(());
    }
    persist_anchored_transition(
        anchor,
        lineage_key_sha256,
        workspace,
        checkpoint,
        |checkpoint| {
            checkpoint.state = NisarSubsetScienceCheckpointStateV2::Interrupted;
            Ok(())
        },
    )?;
    Err(stop_error(control))
}

fn stop_error(control: &WorkerControl) -> NisarSubsetScienceTransferErrorV2 {
    if control.is_cancelled() {
        NisarSubsetScienceTransferErrorV2::Cancelled
    } else {
        NisarSubsetScienceTransferErrorV2::Paused
    }
}

fn result_from_checkpoint(
    checkpoint: &NisarSubsetScienceCheckpointV2,
    budget: &NisarValidatedScienceBudgetV2,
) -> NisarSubsetScienceTransferResultV2 {
    NisarSubsetScienceTransferResultV2 {
        complete: checkpoint.state == NisarSubsetScienceCheckpointStateV2::Complete,
        effective_budget: budget.clone(),
        accounting: NisarSubsetScienceAccountingV2 {
            planning_request_count: checkpoint.committed_planning_request_count,
            planning_response_body_bytes: checkpoint.committed_planning_response_body_bytes,
            reserved_science_request_count: checkpoint.reserved_science_request_count,
            reserved_science_response_body_bytes: checkpoint.reserved_science_response_body_bytes,
            science_request_count: checkpoint.issued_science_request_count,
            science_response_body_bytes: checkpoint.observed_science_response_body_bytes,
            committed_science_request_count: checkpoint.committed_science_request_count,
            committed_science_response_body_bytes: checkpoint.committed_science_response_body_bytes,
            failed_science_request_count: checkpoint.failed_science_request_count,
            indeterminate_science_request_count: checkpoint.indeterminate_science_request_count,
            indeterminate_science_response_body_bytes: checkpoint
                .indeterminate_science_response_body_bytes,
            resumed_bytes: checkpoint.resumed_bytes,
        },
        physical_plan_sha256: checkpoint.physical_plan_sha256.clone(),
        completed_ranges: checkpoint
            .completed_ranges
            .iter()
            .map(|range| (range.start, range.end_exclusive))
            .collect(),
        completed_range_request_counts: checkpoint
            .completed_ranges
            .iter()
            .map(|range| range.request_count)
            .collect(),
    }
}

fn result_matches_checkpoint_authority(
    reported: &NisarSubsetScienceTransferResultV2,
    expected: &NisarSubsetScienceTransferResultV2,
) -> bool {
    let reported_resumed = reported.accounting.resumed_bytes;
    if reported_resumed < expected.accounting.resumed_bytes
        || reported_resumed > expected.accounting.committed_science_response_body_bytes
    {
        return false;
    }
    let mut normalized = reported.clone();
    normalized.accounting.resumed_bytes = expected.accounting.resumed_bytes;
    normalized == *expected
}

fn map_planning_error(
    error: NisarSubsetPlanningErrorV2,
    control: &WorkerControl,
) -> NisarSubsetScienceTransferErrorV2 {
    match error {
        NisarSubsetPlanningErrorV2::ExecutionStopped => stop_error(control),
        NisarSubsetPlanningErrorV2::ValidatorChanged
        | NisarSubsetPlanningErrorV2::SourceValidatorChanged => {
            NisarSubsetScienceTransferErrorV2::StrongValidatorChanged
        }
        NisarSubsetPlanningErrorV2::CombinedBudgetExceeded
        | NisarSubsetPlanningErrorV2::BootstrapBudgetExceeded => {
            NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded
        }
        NisarSubsetPlanningErrorV2::PlanningWorkspaceMissing
        | NisarSubsetPlanningErrorV2::PlanningWorkspaceIdentityMismatch
        | NisarSubsetPlanningErrorV2::RecoveryInvalid => {
            NisarSubsetScienceTransferErrorV2::RecoveryInvalid
        }
        _ => NisarSubsetScienceTransferErrorV2::AuthorityMismatch,
    }
}

fn map_allocation_error(
    error: NisarScienceAllocationEvidenceErrorV2,
) -> NisarSubsetScienceTransferErrorV2 {
    match error {
        NisarScienceAllocationEvidenceErrorV2::AllocationEvidenceUnsupported => {
            NisarSubsetScienceTransferErrorV2::AllocationEvidenceUnsupported
        }
        NisarScienceAllocationEvidenceErrorV2::RecoveryInvalid => {
            NisarSubsetScienceTransferErrorV2::RecoveryInvalid
        }
    }
}

fn map_workspace_error(error: Hdf5RangeTransferError) -> NisarSubsetScienceTransferErrorV2 {
    match error {
        Hdf5RangeTransferError::Persistence => NisarSubsetScienceTransferErrorV2::Persistence,
        Hdf5RangeTransferError::WorkspaceBusy => NisarSubsetScienceTransferErrorV2::RecoveryInvalid,
        _ => NisarSubsetScienceTransferErrorV2::RecoveryInvalid,
    }
}

/// Coherent legacy (pre-upgrade) science states that older software could have
/// left durable, used to exercise the migration entry's reconciliation.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum LegacyScienceChangeV2ForTest {
    /// Charge failed requests the legacy V1 accounting already spent.
    ChargeFailedRequests(u64),
    /// Charge observed and reserved response bytes of failed attempts.
    ChargeFailedBodyBytes(u64),
    /// A crash during the next unfinished Range left this reservation pending;
    /// `None` is the legacy one-slot reservation encoding.
    ReservedAttempt(Option<u64>),
    /// A crash after the next unfinished Range's complete response was recorded
    /// with this many requests, before the Range commit.
    ResponseRecordedAttempt(u64),
    /// The first completed Range's recorded response digest no longer matches
    /// the retained part bytes (same counters, corrupted evidence).
    CorruptFirstCompletedDigest,
    /// The checkpoint carries an upgrade receipt (as after a committed upgrade).
    AttachUpgradeReceipt,
}

/// Advance the durable local checkpoint and its anchor record by one coherent
/// transition. With `leave_pending` the transition stays prepared in the anchor
/// while the local file keeps its predecessor, exactly as a crash between the
/// pending CAS and the local write would leave it.
#[cfg(test)]
pub(crate) fn advance_legacy_science_state_v2_for_test(
    workspace: &Hdf5RangeWorkspaceV1,
    record: &NisarSubsetScienceCheckpointAnchorRecordV2,
    changes: &[LegacyScienceChangeV2ForTest],
    leave_pending: bool,
) -> NisarSubsetScienceCheckpointAnchorRecordV2 {
    let current = parse_checkpoint_canonical_json(
        &workspace
            .load_science_checkpoint_v2()
            .unwrap()
            .expect("science checkpoint"),
    )
    .unwrap();
    let mut next = current.clone();
    let next_unfinished = |checkpoint: &NisarSubsetScienceCheckpointV2| {
        (0..checkpoint.planned_ranges.len())
            .find(|index| {
                !checkpoint
                    .completed_ranges
                    .iter()
                    .any(|completed| usize::from(completed.index) == *index)
            })
            .expect("an unfinished planned Range")
    };
    for change in changes {
        match *change {
            LegacyScienceChangeV2ForTest::ChargeFailedRequests(count) => {
                next.reserved_science_request_count += count;
                next.issued_science_request_count += count;
                next.failed_science_request_count += count;
                next.state = NisarSubsetScienceCheckpointStateV2::Interrupted;
            }
            LegacyScienceChangeV2ForTest::ChargeFailedBodyBytes(bytes) => {
                next.reserved_science_response_body_bytes += bytes;
                next.observed_science_response_body_bytes += bytes;
            }
            LegacyScienceChangeV2ForTest::ReservedAttempt(reserved_request_count) => {
                let index = next_unfinished(&next);
                let planned = next.planned_ranges[index].clone();
                let bytes = planned.len().unwrap();
                next.reserved_science_request_count += reserved_request_count.unwrap_or(1);
                next.reserved_science_response_body_bytes += bytes;
                next.pending_attempt = Some(NisarSubsetSciencePendingAttemptV2 {
                    range_index: u16::try_from(index).unwrap(),
                    start: planned.start(),
                    end_exclusive: planned.end_exclusive(),
                    reserved_response_body_bytes: bytes,
                    reserved_request_count,
                    state: NisarSubsetSciencePendingAttemptStateV2::Reserved,
                    request_issued: false,
                    request_count: 0,
                    observed_response_body_bytes: 0,
                });
                next.state = NisarSubsetScienceCheckpointStateV2::InProgress;
            }
            LegacyScienceChangeV2ForTest::ResponseRecordedAttempt(request_count) => {
                let index = next_unfinished(&next);
                let planned = next.planned_ranges[index].clone();
                let bytes = planned.len().unwrap();
                next.reserved_science_request_count += request_count;
                next.issued_science_request_count += request_count;
                next.reserved_science_response_body_bytes += bytes;
                next.observed_science_response_body_bytes += bytes;
                next.pending_attempt = Some(NisarSubsetSciencePendingAttemptV2 {
                    range_index: u16::try_from(index).unwrap(),
                    start: planned.start(),
                    end_exclusive: planned.end_exclusive(),
                    reserved_response_body_bytes: bytes,
                    reserved_request_count: Some(request_count),
                    state: NisarSubsetSciencePendingAttemptStateV2::ResponseRecorded,
                    request_issued: true,
                    request_count,
                    observed_response_body_bytes: bytes,
                });
                next.state = NisarSubsetScienceCheckpointStateV2::InProgress;
            }
            LegacyScienceChangeV2ForTest::CorruptFirstCompletedDigest => {
                next.completed_ranges[0].response_sha256 = "0".repeat(64);
            }
            LegacyScienceChangeV2ForTest::AttachUpgradeReceipt => {
                next.budget_upgrade_receipt = Some(
                    serde_json::from_value(serde_json::json!({
                        "contract": "nisar_science_budget_upgrade_v1",
                        "proof_sha256": "9".repeat(64),
                        "limits": {
                            "max_request_count": u64::MAX / 4,
                            "max_science_request_count": u64::MAX / 8,
                            "max_response_body_bytes": u64::MAX / 4,
                        },
                    }))
                    .unwrap(),
                );
            }
        }
    }
    next.anchor_generation = current.anchor_generation + 1;
    next.previous_checkpoint_sha256 = Some(current.checkpoint_sha256.clone());
    next.seal().unwrap();
    let pending = record.with_pending(&next).unwrap();
    if leave_pending {
        return pending;
    }
    persist_checkpoint(workspace, &next).unwrap();
    pending.commit_pending().unwrap()
}

#[cfg(test)]
pub(crate) enum ScienceCheckpointMutationV2ForTest {
    FirstCompletedDigest(String),
    RemoveFirstPlannedOwner,
    DecrementCommittedScienceBytes,
}

#[cfg(test)]
pub(crate) fn mutate_science_checkpoint_v2_for_test(
    workspace: &Hdf5RangeWorkspaceV1,
    mutation: ScienceCheckpointMutationV2ForTest,
) {
    let text = workspace
        .load_science_checkpoint_v2()
        .unwrap()
        .expect("science checkpoint");
    let mut checkpoint: NisarSubsetScienceCheckpointV2 = serde_json::from_str(&text).unwrap();
    match mutation {
        ScienceCheckpointMutationV2ForTest::FirstCompletedDigest(digest) => {
            checkpoint.completed_ranges[0].response_sha256 = digest;
        }
        ScienceCheckpointMutationV2ForTest::RemoveFirstPlannedOwner => {
            checkpoint.planned_ranges[0].remove_logical_owner_for_test();
        }
        ScienceCheckpointMutationV2ForTest::DecrementCommittedScienceBytes => {
            checkpoint.committed_science_response_body_bytes = checkpoint
                .committed_science_response_body_bytes
                .saturating_sub(1);
        }
    }
    checkpoint.seal().unwrap();
    persist_checkpoint(workspace, &checkpoint).unwrap();
}

/// Test-only hook run before every read of a completed-material digest on
/// this thread (downloader enhancement D2: a test pauses a digest midway).
#[cfg(test)]
pub(crate) mod material_digest_hook_for_test {
    use std::cell::RefCell;

    type Hook = Box<dyn Fn()>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
        static AFTER_STOP: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    pub(crate) struct Installed(());

    pub(crate) fn install(hook: impl Fn() + 'static) -> Installed {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        Installed(())
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            HOOK.with(|slot| *slot.borrow_mut() = None);
            AFTER_STOP.with(|slot| *slot.borrow_mut() = None);
        }
    }

    /// Also runs `hook` right after a digest observed a stop, before the
    /// stop error is returned: a scheduler that resumes the item at once
    /// (downloader enhancement D1b, P2-4).
    pub(crate) fn install_with_after_stop(
        hook: impl Fn() + 'static,
        after_stop: impl Fn() + 'static,
    ) -> Installed {
        AFTER_STOP.with(|slot| *slot.borrow_mut() = Some(Box::new(after_stop)));
        install(hook)
    }

    pub(super) fn before_read() {
        HOOK.with(|slot| {
            if let Some(hook) = slot.borrow().as_ref() {
                hook();
            }
        });
    }

    pub(super) fn after_stop() {
        AFTER_STOP.with(|slot| {
            if let Some(hook) = slot.borrow().as_ref() {
                hook();
            }
        });
    }
}

/// Test-only substitute for the immutable view the writer's final digest
/// reads on this thread (downloader enhancement D2): bytes that differ from the
/// checkpoint while every identity fact stays the same.
#[cfg(test)]
pub(crate) mod final_digest_view_for_test {
    use std::cell::RefCell;
    use std::fs::File;
    use std::path::PathBuf;

    thread_local! {
        static VIEW: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    pub(crate) struct Installed(());

    pub(crate) fn install(path: PathBuf) -> Installed {
        VIEW.with(|slot| *slot.borrow_mut() = Some(path));
        Installed(())
    }

    impl Drop for Installed {
        fn drop(&mut self) {
            VIEW.with(|slot| *slot.borrow_mut() = None);
        }
    }

    pub(super) fn substitute(view: File) -> File {
        VIEW.with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|path| File::open(path).ok())
        })
        .unwrap_or(view)
    }
}

/// Test-only ledger of completed-material hash work, keyed by workspace and by
/// the labelled caller (a thread-local site guard), so route tests can count
/// exactly how many full passes each continuation path performs.
#[cfg(test)]
pub(crate) mod completed_material_hash_ledger {
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::sync::Mutex;

    type Ledger = HashMap<(String, &'static str), u64>;

    static LEDGER: Mutex<Option<Ledger>> = Mutex::new(None);

    thread_local! {
        static SITE: Cell<&'static str> = const { Cell::new("unlabelled") };
    }

    pub(crate) struct SiteGuard(&'static str);

    pub(crate) fn enter(site: &'static str) -> SiteGuard {
        SiteGuard(SITE.with(|current| current.replace(site)))
    }

    impl Drop for SiteGuard {
        fn drop(&mut self) {
            SITE.with(|current| current.set(self.0));
        }
    }

    pub(crate) fn record(workspace: &str, bytes: u64) {
        let site = SITE.with(Cell::get);
        let mut ledger = LEDGER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *ledger
            .get_or_insert_with(HashMap::new)
            .entry((workspace.to_owned(), site))
            .or_insert(0) += bytes;
    }

    /// Completed-material bytes hashed for `workspace` by `site` so far.
    pub(crate) fn hashed_bytes(workspace: &str, site: &str) -> u64 {
        let ledger = LEDGER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ledger
            .as_ref()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|((path, label), _)| path == workspace && *label == site)
                    .map(|(_, bytes)| *bytes)
                    .sum()
            })
            .unwrap_or(0)
    }
}

fn digest_serializable(
    value: &impl Serialize,
) -> Result<String, NisarSubsetScienceTransferErrorV2> {
    serde_json::to_vec(value)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
}

fn sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn science_pending_legacy_canonical_and_digest_are_unchanged() {
        for raw in [
            r#"{"range_index":0,"start":0,"end_exclusive":32,"reserved_response_body_bytes":32,"state":"Reserved","request_issued":false,"request_count":0,"observed_response_body_bytes":0}"#,
            r#"{"range_index":0,"start":0,"end_exclusive":32,"reserved_response_body_bytes":32,"state":"ResponseRecorded","request_issued":true,"request_count":5,"observed_response_body_bytes":32}"#,
        ] {
            let pending: NisarSubsetSciencePendingAttemptV2 = serde_json::from_str(raw).unwrap();
            assert_eq!(pending.reserved_request_count, None);
            assert_eq!(serde_json::to_string(&pending).unwrap(), raw);
            assert_eq!(
                digest_serializable(&pending).unwrap(),
                sha256(raw.as_bytes())
            );
        }
    }

    /// Shared verbatim with the strict offline mirror in
    /// crates/insar-application-service/tests/support/nisar_recovery_receipt.rs.
    const GOLDEN_SCIENCE_CHECKPOINT_IDENTITY_V2: &str = concat!(
        r#"{"checkpoint_contract":"NisarSubsetScienceCheckpointV2","schema_version":1,"#,
        r#""workspace_canonical_path":"recovery-workspace","#,
        r#""workspace_identity":"workspace-identity","#,
        r#""lineage_key_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","#,
        r#""execution_binding_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","#,
        r#""anchor_generation":7,"#,
        r#""previous_checkpoint_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","#,
        r#""part_identity_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","#,
        r#""task_id":"task","item_id":"item","#,
        r#""prepared_intent_sha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","#,
        r#""final_plan_sha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff","#,
        r#""physical_plan_sha256":"1111111111111111111111111111111111111111111111111111111111111111","#,
        r#""inventory_sha256":"2222222222222222222222222222222222222222222222222222222222222222","#,
        r#""inventory_evidence_sha256":"3333333333333333333333333333333333333333333333333333333333333333","#,
        r#""source_graph_sha256":"4444444444444444444444444444444444444444444444444444444444444444","#,
        r#""source_product_identity":{"provider_id":"asf","collection_id":"C1-ASF","#,
        r#""collection_profile_id":"nisar-l1-rslc","provider_product_id":"G1-ASF","#,
        r#""provider_product_key":"PROVIDER::asf::ITEM::G1-ASF","#,
        r#""product_specification_version":"1.0","crid":"A","quality_status":"PROVISIONAL"},"#,
        r#""source_asset_identity":{"asset_key":"science-rslc-hdf5","#,
        r#""stable_asset_key":"stable-asset","file_name":"source.h5","#,
        r#""checksum":{"algorithm":"sha256","#,
        r#""value":"5555555555555555555555555555555555555555555555555555555555555555"}},"#,
        r#""source_validator":{"kind":"strong_etag","value":"\"etag\"","content_length":4096},"#,
        r#""source_validator_sha256":"6666666666666666666666666666666666666666666666666666666666666666","#,
        r#""source_size":4096,"planned_ranges":[{"start":0,"end_exclusive":32,"#,
        r#""logical_owners":[{"dataset_path":"/science/LSAR/RSLC/swaths/frequencyA/HH","#,
        r#""polarization":"HH","component_id":null,"window_sha256":null,"#,
        r#""logical_chunk":[0,0]}]},{"start":64,"end_exclusive":96,"logical_owners":[]}],"#,
        r#""completed_ranges":[{"index":0,"start":0,"end_exclusive":32,"request_count":2,"#,
        r#""response_sha256":"7777777777777777777777777777777777777777777777777777777777777777","#,
        r#""allocated_range_evidence":{"intervals":[[0,32]]}}],"pending_attempt":null,"#,
        r#""committed_planning_request_count":31,"#,
        r#""committed_planning_response_body_bytes":195035138,"#,
        r#""reserved_science_request_count":935,"#,
        r#""reserved_science_response_body_bytes":493865474,"issued_science_request_count":935,"#,
        r#""observed_science_response_body_bytes":493865474,"failed_science_request_count":933,"#,
        r#""unissued_science_reservation_count":0,"indeterminate_science_request_count":0,"#,
        r#""indeterminate_science_response_body_bytes":0,"committed_science_request_count":2,"#,
        r#""committed_science_response_body_bytes":32,"resumed_bytes":0,"state":"Interrupted"}"#,
    );
    const GOLDEN_BUDGET_UPGRADE_RECEIPT_V2: &str = concat!(
        r#"{"contract":"nisar_science_budget_upgrade_v1","#,
        r#""proof_sha256":"9999999999999999999999999999999999999999999999999999999999999999","#,
        r#""limits":{"max_request_count":1054,"max_science_request_count":1023,"#,
        r#""max_response_body_bytes":1186766086}}"#,
    );

    /// A checkpoint digest covers every field except `checkpoint_sha256`, in
    /// declaration order, so the sealed canonical text is the identity text
    /// followed by the digest member.
    fn sealed_golden_checkpoint(identity: &str) -> String {
        format!(
            "{},\"checkpoint_sha256\":\"{}\"}}",
            &identity[..identity.len() - 1],
            sha256(identity.as_bytes())
        )
    }

    fn golden_identity_with_receipt() -> String {
        GOLDEN_SCIENCE_CHECKPOINT_IDENTITY_V2.replacen(
            ",\"state\":",
            &format!(",\"budget_upgrade_receipt\":{GOLDEN_BUDGET_UPGRADE_RECEIPT_V2},\"state\":"),
            1,
        )
    }

    #[test]
    fn science_checkpoint_budget_receipt_canonical_and_digest_match_the_offline_mirror() {
        let receipt: NisarScienceBudgetUpgradeReceiptV2 =
            serde_json::from_str(GOLDEN_BUDGET_UPGRADE_RECEIPT_V2).unwrap();
        assert_eq!(
            serde_json::to_string(&receipt).unwrap(),
            GOLDEN_BUDGET_UPGRADE_RECEIPT_V2
        );
        let upgraded_identity = golden_identity_with_receipt();
        for (identity, has_receipt) in [
            (GOLDEN_SCIENCE_CHECKPOINT_IDENTITY_V2.to_owned(), false),
            (upgraded_identity.clone(), true),
        ] {
            let canonical = sealed_golden_checkpoint(&identity);
            // Strict parse rechecks byte-exact canonical form and the digest.
            let checkpoint = parse_checkpoint_canonical_json(&canonical).unwrap();
            assert_eq!(checkpoint_canonical_json(&checkpoint).unwrap(), canonical);
            assert_eq!(
                checkpoint.recompute_sha256().unwrap(),
                sha256(identity.as_bytes())
            );
            assert_eq!(checkpoint.budget_upgrade_receipt.is_some(), has_receipt);
        }
        // A legacy checkpoint keeps its pre-receipt canonical text and digest.
        assert!(!GOLDEN_SCIENCE_CHECKPOINT_IDENTITY_V2.contains("budget_upgrade_receipt"));
        assert_ne!(
            sealed_golden_checkpoint(GOLDEN_SCIENCE_CHECKPOINT_IDENTITY_V2),
            sealed_golden_checkpoint(&upgraded_identity)
        );
        for tampered in [
            upgraded_identity.replacen("\"limits\":{", "\"limits\":{\"extra\":1,", 1),
            upgraded_identity.replacen("\"contract\":", "\"extra\":1,\"contract\":", 1),
        ] {
            assert_eq!(
                parse_checkpoint_canonical_json(&sealed_golden_checkpoint(&tampered)),
                Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
            );
        }
    }

    #[test]
    fn science_request_settlement_releases_only_unused_slots_and_charges_zero_hop() {
        let mut pending = NisarSubsetSciencePendingAttemptV2 {
            range_index: 0,
            start: 0,
            end_exclusive: 32,
            reserved_response_body_bytes: 32,
            reserved_request_count: Some(22),
            state: NisarSubsetSciencePendingAttemptStateV2::Reserved,
            request_issued: false,
            request_count: 0,
            observed_response_body_bytes: 0,
        };
        assert_eq!(settled_request_reservation(100, &pending, 3), Ok(81));
        assert_eq!(settled_request_reservation(100, &pending, 0), Ok(79));
        assert_eq!(settled_request_reservation(100, &pending, 22), Ok(100));
        assert_eq!(
            settled_request_reservation(100, &pending, 23),
            Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
        );
        assert_eq!(
            settled_request_reservation(1, &pending, 0),
            Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
        );
        for allowance in [None, Some(0), Some(23)] {
            pending.reserved_request_count = allowance;
            assert_eq!(
                settled_request_reservation(100, &pending, 0),
                Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
            );
        }
    }

    struct BudgetReservation {
        max_request_count: u64,
        max_response_body_bytes: u64,
        max_science_request_count: u64,
        planning_requests: u64,
        planning_bytes: u64,
        science_requests: u64,
        science_bytes: u64,
        expected_bytes: u64,
    }

    fn reserve_then_observe_io(
        reservation: BudgetReservation,
        io_count: &mut u64,
    ) -> Result<(), NisarSubsetScienceTransferErrorV2> {
        validate_budget_reservation_values(
            reservation.max_request_count,
            reservation.max_response_body_bytes,
            reservation.max_science_request_count,
            reservation.planning_requests,
            reservation.planning_bytes,
            reservation.science_requests,
            reservation.science_bytes,
            reservation.expected_bytes,
        )?;
        *io_count += 1;
        Ok(())
    }

    #[test]
    fn science_budget_exact_request_and_byte_limits_pass_before_io() {
        let mut io_count = 0;
        assert_eq!(
            reserve_then_observe_io(
                BudgetReservation {
                    max_request_count: 256,
                    max_response_body_bytes: 1_000,
                    max_science_request_count: 256,
                    planning_requests: 255,
                    planning_bytes: 999,
                    science_requests: 0,
                    science_bytes: 0,
                    expected_bytes: 1,
                },
                &mut io_count,
            ),
            Ok(())
        );
        assert_eq!(io_count, 1);
    }

    #[test]
    fn science_budget_rejects_request_257_before_io() {
        let mut io_count = 0;
        assert_eq!(
            reserve_then_observe_io(
                BudgetReservation {
                    max_request_count: 256,
                    max_response_body_bytes: 1_000,
                    max_science_request_count: 256,
                    planning_requests: 256,
                    planning_bytes: 1,
                    science_requests: 0,
                    science_bytes: 0,
                    expected_bytes: 1,
                },
                &mut io_count,
            ),
            Err(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)
        );
        assert_eq!(io_count, 0);
    }

    #[test]
    fn science_budget_rejects_257th_science_request_even_with_larger_total_budget() {
        let mut io_count = 0;
        assert_eq!(
            reserve_then_observe_io(
                BudgetReservation {
                    max_request_count: 768,
                    max_response_body_bytes: 10_000,
                    max_science_request_count: 256,
                    planning_requests: 300,
                    planning_bytes: 1,
                    science_requests: crate::NISAR_LIVE_MAX_REQUESTS,
                    science_bytes: 256,
                    expected_bytes: 1,
                },
                &mut io_count,
            ),
            Err(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)
        );
        assert_eq!(io_count, 0);
    }

    #[test]
    fn science_budget_rejects_ratio_and_512_mib_overshoot_before_io() {
        for hard_limit in [1_000_u64, 512 * 1024 * 1024] {
            let mut io_count = 0;
            assert_eq!(
                reserve_then_observe_io(
                    BudgetReservation {
                        max_request_count: 256,
                        max_response_body_bytes: hard_limit,
                        max_science_request_count: 256,
                        planning_requests: 1,
                        planning_bytes: hard_limit,
                        science_requests: 0,
                        science_bytes: 0,
                        expected_bytes: 1,
                    },
                    &mut io_count,
                ),
                Err(NisarSubsetScienceTransferErrorV2::CombinedBudgetExceeded)
            );
            assert_eq!(io_count, 0);
        }
    }

    #[test]
    fn science_budget_uses_frozen_policy_limit_above_legacy_256() {
        let mut io_count = 0;
        assert_eq!(
            reserve_then_observe_io(
                BudgetReservation {
                    max_request_count: 1_186,
                    max_response_body_bytes: 1_000_000,
                    max_science_request_count: 986,
                    planning_requests: 200,
                    planning_bytes: 1,
                    science_requests: 493,
                    science_bytes: 493,
                    expected_bytes: 1,
                },
                &mut io_count,
            ),
            Ok(())
        );
        assert_eq!(io_count, 1);
    }

    #[test]
    fn nisar_subset_committed_recovery_telemetry_preserves_legacy_anchor_canonical_json() {
        let mut legacy = NisarSubsetScienceCheckpointAnchorRecordV2 {
            lineage_key_sha256: "a".repeat(64),
            execution_binding_sha256: "b".repeat(64),
            generation: 7,
            committed_checkpoint_sha256: Some("c".repeat(64)),
            committed_recovery_telemetry: None,
            pending_transition: None,
            record_sha256: String::new(),
        };
        legacy.seal().unwrap();
        let canonical = legacy.canonical_json().unwrap();

        assert!(!canonical.contains("committed_recovery_telemetry"));
        let parsed =
            NisarSubsetScienceCheckpointAnchorRecordV2::from_canonical_json(&canonical).unwrap();
        assert_eq!(parsed, legacy);
        assert_eq!(parsed.canonical_json().unwrap(), canonical);
    }

    #[test]
    fn nisar_subset_committed_recovery_telemetry_rejects_tampered_projection() {
        let committed = "c".repeat(64);
        let mut record = NisarSubsetScienceCheckpointAnchorRecordV2 {
            lineage_key_sha256: "a".repeat(64),
            execution_binding_sha256: "b".repeat(64),
            generation: 7,
            committed_checkpoint_sha256: Some(committed.clone()),
            committed_recovery_telemetry: Some(NisarSubsetCommittedRecoveryTelemetryV1 {
                contract: NISAR_SUBSET_COMMITTED_RECOVERY_TELEMETRY_CONTRACT_V1.to_owned(),
                checkpoint_sha256: committed,
                reusable_verified_bytes: 13,
                reused_verified_bytes: 5,
            }),
            pending_transition: None,
            record_sha256: String::new(),
        };
        record.seal().unwrap();
        assert!(record.validate().is_ok());

        let mut checkpoint_mismatch = record.clone();
        checkpoint_mismatch
            .committed_recovery_telemetry
            .as_mut()
            .unwrap()
            .checkpoint_sha256 = "d".repeat(64);
        checkpoint_mismatch.seal().unwrap();
        assert_eq!(
            checkpoint_mismatch.validate(),
            Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
        );

        let mut impossible_reuse = record.clone();
        impossible_reuse
            .committed_recovery_telemetry
            .as_mut()
            .unwrap()
            .reused_verified_bytes = 14;
        impossible_reuse.seal().unwrap();
        assert_eq!(
            impossible_reuse.validate(),
            Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
        );

        let mut projection_without_committed_checkpoint = record.clone();
        projection_without_committed_checkpoint.committed_checkpoint_sha256 = None;
        projection_without_committed_checkpoint.seal().unwrap();
        assert_eq!(
            projection_without_committed_checkpoint.validate(),
            Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
        );

        let mut tampered_without_reseal = record;
        tampered_without_reseal
            .committed_recovery_telemetry
            .as_mut()
            .unwrap()
            .reusable_verified_bytes += 1;
        let canonical = serde_json::to_string(&tampered_without_reseal).unwrap();
        assert_eq!(
            NisarSubsetScienceCheckpointAnchorRecordV2::from_canonical_json(&canonical),
            Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
        );
    }
}
