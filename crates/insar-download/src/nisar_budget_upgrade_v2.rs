use super::*;
use insar_store::{
    NisarBudgetUpgradeLimitsV2, StoreNisarBudgetUpgradeEvidenceV2,
    StoreNisarBudgetUpgradePreReconciliationV2, StoredNisarBudgetUpgradeProofV2,
    StoredNisarSubsetExecutionAnchorV2,
};

const RECEIPT_CONTRACT: &str = "nisar_science_budget_upgrade_v1";
const LEGACY_FETCH_MAX_REQUESTS: u64 = 22;
const UPGRADE_REQUESTS_PER_REMAINING_RANGE: u64 = 44;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct NisarScienceBudgetUpgradeReceiptV2 {
    contract: String,
    proof_sha256: String,
    limits: NisarBudgetUpgradeLimitsV2,
}

/// Constructed from the original frozen plan, or a fully checked immutable proof.
/// A receipt alone never creates execution authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NisarValidatedScienceBudgetV2 {
    limits: NisarBudgetUpgradeLimitsV2,
    receipt: Option<NisarScienceBudgetUpgradeReceiptV2>,
}

impl NisarValidatedScienceBudgetV2 {
    pub(super) fn original(plan: &NisarSubsetPhysicalExecutionPlanV2) -> Self {
        Self {
            limits: original_limits(plan.budget_projection()),
            receipt: None,
        }
    }

    /// Narrow validation capability for a receipt-less legacy V1 predecessor:
    /// the frozen V1 request ceilings plus at most one legacy fetch minus one.
    /// It validates and reconciles already-charged evidence only; reservations
    /// never use it, so it cannot spend a request the V1 plan did not allow.
    fn legacy_upgrade_predecessor(
        plan: &NisarSubsetPhysicalExecutionPlanV2,
    ) -> Result<Self, NisarSubsetScienceTransferErrorV2> {
        if !plan.budget_projection().uses_physical_execution_union_v1() {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        let mut budget = Self::original(plan);
        for limit in [
            &mut budget.limits.max_request_count,
            &mut budget.limits.max_science_request_count,
        ] {
            *limit = limit
                .checked_add(LEGACY_FETCH_MAX_REQUESTS - 1)
                .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
        }
        Ok(budget)
    }

    pub(super) fn max_request_count(&self) -> u64 {
        self.limits.max_request_count
    }
    pub(super) fn max_science_request_count(&self) -> u64 {
        self.limits.max_science_request_count
    }
    pub(super) fn max_response_body_bytes(&self) -> u64 {
        self.limits.max_response_body_bytes
    }
    pub(super) fn matches_receipt(&self, checkpoint: &NisarSubsetScienceCheckpointV2) -> bool {
        checkpoint.budget_upgrade_receipt == self.receipt
    }
}

/// Which validation budget the shared recovery reconciliation uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScienceRecoveryBudgetModeV2 {
    /// Executable authority: the original plan budget or a proof-backed receipt.
    Effective,
    /// Migration entry only: reconcile a proof-less, receipt-less legacy V1
    /// predecessor before its one-time upgrade.
    LegacyUpgradePredecessor,
}

/// Where a one-time budget upgrade stopped; used for the local diagnostic only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NisarBudgetMigrationPhaseV2 {
    PlanningValidation,
    LegacyReconciliation,
    PredecessorValidation,
    BudgetDerivation,
    StoreAdmission,
    ReceiptCommit,
    ProofReconciliation,
    EffectiveBudgetValidation,
}

impl NisarBudgetMigrationPhaseV2 {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::PlanningValidation => "planning_validation",
            Self::LegacyReconciliation => "legacy_reconciliation",
            Self::PredecessorValidation => "predecessor_validation",
            Self::BudgetDerivation => "budget_derivation",
            Self::StoreAdmission => "store_admission",
            Self::ReceiptCommit => "receipt_commit",
            Self::ProofReconciliation => "proof_reconciliation",
            Self::EffectiveBudgetValidation => "effective_budget_validation",
        }
    }

    pub(crate) const fn description_zh(self) -> &'static str {
        match self {
            Self::PlanningValidation => "核对冻结的规划记录与已下载的元数据",
            Self::LegacyReconciliation => "整理上次中断时遗留的传输记录",
            Self::PredecessorValidation => "核对已下载片段、断点记录与冻结计划",
            Self::BudgetDerivation => "按已记账的请求数和字节数计算可继承的预算",
            Self::StoreAdmission => "写入一次性升级凭据前核对任务状态",
            Self::ReceiptCommit => "提交升级后的断点记录",
            Self::ProofReconciliation => "核对已保存的预算升级凭据",
            Self::EffectiveBudgetValidation => "继续下载前复核生效预算",
        }
    }
}

/// A failed migration-entry reconciliation. `refusal` is set only when the
/// one-time upgrade applies to this item (a proof exists or the predecessor is
/// an incomplete receipt-less V1 checkpoint) and the failure is a local
/// validation result; transient Store/filesystem failures and lifecycle stops
/// keep their ordinary codes. `reconciled` reports whether the shared recovery
/// may already have settled (committed) an interrupted legacy transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NisarBudgetMigrationFailureV2 {
    pub(crate) error: NisarSubsetScienceTransferErrorV2,
    pub(crate) refusal: Option<NisarBudgetMigrationPhaseV2>,
    pub(crate) reconciled: bool,
}

trait MigrationResultExt<T> {
    fn ordinary(self) -> Result<T, NisarBudgetMigrationFailureV2>;
    fn in_phase(
        self,
        phase: NisarBudgetMigrationPhaseV2,
    ) -> Result<T, NisarBudgetMigrationFailureV2>;
}

impl<T> MigrationResultExt<T> for Result<T, NisarSubsetScienceTransferErrorV2> {
    fn ordinary(self) -> Result<T, NisarBudgetMigrationFailureV2> {
        self.map_err(|error| NisarBudgetMigrationFailureV2 {
            error,
            refusal: None,
            reconciled: false,
        })
    }

    fn in_phase(
        self,
        phase: NisarBudgetMigrationPhaseV2,
    ) -> Result<T, NisarBudgetMigrationFailureV2> {
        self.map_err(|error| NisarBudgetMigrationFailureV2 {
            error,
            refusal: refusal_for(error, phase),
            reconciled: false,
        })
    }
}

/// Only a local validation result is a terminal refusal. A transient local
/// Store or filesystem failure (`Persistence`, or a full disk or I/O error as
/// `LocalStorageFailure`, review 2026-10-17 round 3, P2-2) and a lifecycle
/// stop keep the ordinary failure path, so a later continue simply retries the
/// migration.
fn refusal_for(
    error: NisarSubsetScienceTransferErrorV2,
    phase: NisarBudgetMigrationPhaseV2,
) -> Option<NisarBudgetMigrationPhaseV2> {
    match error {
        NisarSubsetScienceTransferErrorV2::Persistence
        | NisarSubsetScienceTransferErrorV2::LocalStorageFailure { .. }
        | NisarSubsetScienceTransferErrorV2::Paused
        | NisarSubsetScienceTransferErrorV2::Cancelled => None,
        _ => Some(phase),
    }
}

fn refused<T>(phase: NisarBudgetMigrationPhaseV2) -> Result<T, NisarBudgetMigrationFailureV2> {
    Err(NisarBudgetMigrationFailureV2 {
        error: NisarSubsetScienceTransferErrorV2::RecoveryInvalid,
        refusal: Some(phase),
        reconciled: false,
    })
}

fn original_limits(budget: &NisarSubsetCombinedBudgetProjectionV2) -> NisarBudgetUpgradeLimitsV2 {
    NisarBudgetUpgradeLimitsV2 {
        max_request_count: budget.max_request_count(),
        max_science_request_count: budget.max_science_request_count(),
        max_response_body_bytes: budget.max_response_body_bytes(),
    }
}

#[allow(clippy::too_many_arguments)]
fn inherited_limits(
    old: &NisarBudgetUpgradeLimitsV2,
    planning_requests: u64,
    planning_bytes: u64,
    reserved_requests: u64,
    reserved_bytes: u64,
    remaining_count: u64,
    remaining_bytes: u64,
) -> Result<NisarBudgetUpgradeLimitsV2, NisarSubsetScienceTransferErrorV2> {
    let invalid = NisarSubsetScienceTransferErrorV2::RecoveryInvalid;
    if old.max_request_count.checked_sub(planning_requests) != Some(old.max_science_request_count)
        || old
            .max_science_request_count
            .checked_add(LEGACY_FETCH_MAX_REQUESTS - 1)
            .is_none_or(|limit| reserved_requests > limit)
        || planning_bytes
            .checked_add(reserved_bytes)
            .is_none_or(|total| total > old.max_response_body_bytes)
        || remaining_count == 0
        || remaining_bytes == 0
    {
        return Err(invalid);
    }
    let max_science_request_count = old.max_science_request_count.max(
        reserved_requests
            .checked_add(
                remaining_count
                    .checked_mul(UPGRADE_REQUESTS_PER_REMAINING_RANGE)
                    .ok_or(invalid)?,
            )
            .ok_or(invalid)?,
    );
    let max_request_count = planning_requests
        .checked_add(max_science_request_count)
        .ok_or(invalid)?;
    let max_response_body_bytes = old.max_response_body_bytes.max(
        planning_bytes
            .checked_add(reserved_bytes)
            .and_then(|total| {
                remaining_bytes
                    .checked_mul(2)
                    .and_then(|remaining| total.checked_add(remaining))
            })
            .ok_or(invalid)?,
    );
    Ok(NisarBudgetUpgradeLimitsV2 {
        max_request_count,
        max_science_request_count,
        max_response_body_bytes,
    })
}

fn remaining_ranges(
    checkpoint: &NisarSubsetScienceCheckpointV2,
) -> Vec<NisarSubsetPhysicalRangeV2> {
    checkpoint
        .planned_ranges
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            !checkpoint
                .completed_ranges
                .iter()
                .any(|done| usize::from(done.index) == *index)
        })
        .map(|(_, range)| range.clone())
        .collect()
}

fn derive_upgrade_limits(
    checkpoint: &NisarSubsetScienceCheckpointV2,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
) -> Result<NisarBudgetUpgradeLimitsV2, NisarSubsetScienceTransferErrorV2> {
    let remaining = remaining_ranges(checkpoint);
    let bytes = remaining.iter().try_fold(0_u64, |total, range| {
        total
            .checked_add(
                range
                    .len()
                    .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?,
            )
            .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
    })?;
    inherited_limits(
        &original_limits(plan.budget_projection()),
        checkpoint.committed_planning_request_count,
        checkpoint.committed_planning_response_body_bytes,
        checkpoint.reserved_science_request_count,
        checkpoint.reserved_science_response_body_bytes,
        remaining.len() as u64,
        bytes,
    )
}

/// Structural validation of a receipt-less legacy V1 upgrade predecessor under
/// the narrow predecessor budget. It reads no part bytes.
fn validate_upgrade_source_structure(
    checkpoint: &NisarSubsetScienceCheckpointV2,
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    path: &str,
    identity: &str,
    binding: &str,
    part_identity: &str,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    if !plan.budget_projection().uses_physical_execution_union_v1()
        || checkpoint.budget_upgrade_receipt.is_some()
        || checkpoint.pending_attempt.is_some()
        || checkpoint.state == NisarSubsetScienceCheckpointStateV2::Complete
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    checkpoint.validate_with_budget(
        work,
        plan,
        path,
        identity,
        &lineage_key_sha256(work, plan)?,
        binding,
        part_identity,
        &NisarValidatedScienceBudgetV2::legacy_upgrade_predecessor(plan)?,
    )
}

/// Structure plus every completed Range's allocation evidence and digest. Used
/// once, before a proof is derived; a proof's later checks rely on the caller
/// hashing a checkpoint whose completed Ranges include all of the predecessor's.
#[allow(clippy::too_many_arguments)]
fn validate_upgrade_source(
    checkpoint: &NisarSubsetScienceCheckpointV2,
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    path: &str,
    identity: &str,
    binding: &str,
    part_identity: &str,
    part: &mut File,
) -> Result<(), NisarSubsetScienceTransferErrorV2> {
    validate_upgrade_source_structure(
        checkpoint,
        work,
        plan,
        path,
        identity,
        binding,
        part_identity,
    )?;
    verify_completed_material(part, checkpoint)
}

fn target_checkpoint(
    proof: &StoredNisarBudgetUpgradeProofV2,
    old: &NisarSubsetScienceCheckpointV2,
) -> Result<NisarSubsetScienceCheckpointV2, NisarSubsetScienceTransferErrorV2> {
    let mut target = old.clone();
    target.budget_upgrade_receipt = Some(NisarScienceBudgetUpgradeReceiptV2 {
        contract: RECEIPT_CONTRACT.to_owned(),
        proof_sha256: proof.proof_sha256().to_owned(),
        limits: proof.evidence().new_limits.clone(),
    });
    target.anchor_generation = old
        .anchor_generation
        .checked_add(1)
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)?;
    target.previous_checkpoint_sha256 = Some(old.checkpoint_sha256.clone());
    target.seal()?;
    // Roundtrip typed canonical bytes before giving the Store a target document.
    parse_checkpoint_canonical_json(&checkpoint_canonical_json(&target)?)
}

/// Verifies an immutable proof against the retained planning evidence, the
/// frozen plan, its predecessor anchor and the derived limits. It hashes no part
/// bytes: every caller then verifies the material of a checkpoint whose
/// completed Ranges are required to include all of the proof's predecessor
/// Ranges (see `validated_budget`), so the same bytes are hashed once.
#[allow(clippy::too_many_arguments)]
fn validate_proof_source(
    proof: &StoredNisarBudgetUpgradeProofV2,
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    workspace: &Hdf5RangeWorkspaceV1,
    path: &str,
    identity: &str,
    binding: &str,
    part_identity: &str,
) -> Result<NisarSubsetScienceCheckpointV2, NisarSubsetScienceTransferErrorV2> {
    let evidence = proof.evidence();
    if proof.context().task_id != work.task.task_id
        || proof.context().item_id != work.item.item_id
        || proof.context().item != work.item
        || workspace
            .load_planning_checkpoint_v2()
            .map_err(map_workspace_error)?
            .as_deref()
            != Some(evidence.planning_checkpoint_canonical_json.as_str())
        || evidence.old_limits != original_limits(plan.budget_projection())
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    let old = parse_checkpoint_canonical_json(&evidence.science_checkpoint_canonical_json)?;
    let old_anchor = NisarSubsetScienceCheckpointAnchorRecordV2::from_canonical_json(
        &proof.context().science_anchor.canonical_json,
    )?;
    if old_anchor.pending_transition.is_some()
        || old_anchor.committed_checkpoint_sha256.as_deref() != Some(&old.checkpoint_sha256)
        || old_anchor.generation != old.anchor_generation
        || old_anchor.lineage_key_sha256 != old.lineage_key_sha256
        || old_anchor.execution_binding_sha256 != binding
        || old.execution_binding_sha256 != binding
        || old_anchor.record_sha256 != proof.context().science_anchor.record_sha256
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    if let Some(pre) = &evidence.pre_reconciliation {
        let reconciled_from = NisarSubsetScienceCheckpointAnchorRecordV2::from_canonical_json(
            &pre.science_anchor_canonical_json,
        )?;
        if reconciled_from.lineage_key_sha256 != old.lineage_key_sha256
            || reconciled_from.execution_binding_sha256 != binding
            || reconciled_from.record_sha256 != pre.science_anchor_record_sha256
        {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
    }
    validate_upgrade_source_structure(&old, work, plan, path, identity, binding, part_identity)?;
    if evidence.new_limits != derive_upgrade_limits(&old, plan)?
        || evidence.completed_ranges_sha256 != digest_serializable(&old.completed_ranges)?
        || evidence.remaining_ranges_sha256 != digest_serializable(&remaining_ranges(&old))?
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    Ok(old)
}

/// The effective budget of `checkpoint`. With a proof, this checks the proof and
/// that every predecessor Range survives unchanged in `checkpoint`; it hashes no
/// part bytes, so every caller must verify `checkpoint`'s completed material
/// (which therefore covers the predecessor's) before relying on the budget.
#[allow(clippy::too_many_arguments)]
pub(super) fn validated_budget(
    checkpoint: &NisarSubsetScienceCheckpointV2,
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    workspace: &Hdf5RangeWorkspaceV1,
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    path: &str,
    identity: &str,
    binding: &str,
    part_identity: &str,
) -> Result<NisarValidatedScienceBudgetV2, NisarSubsetScienceTransferErrorV2> {
    let Some(proof) = anchor.load_budget_upgrade_proof()? else {
        if checkpoint.budget_upgrade_receipt.is_some() {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
        return Ok(NisarValidatedScienceBudgetV2::original(plan));
    };
    let old = validate_proof_source(
        &proof,
        work,
        plan,
        workspace,
        path,
        identity,
        binding,
        part_identity,
    )?;
    let target = target_checkpoint(&proof, &old)?;
    if checkpoint.budget_upgrade_receipt != target.budget_upgrade_receipt
        || checkpoint.anchor_generation < target.anchor_generation
        || (checkpoint.anchor_generation == target.anchor_generation && checkpoint != &target)
        || old
            .completed_ranges
            .iter()
            .any(|range| !checkpoint.completed_ranges.contains(range))
    {
        return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
    }
    // Every charged/observed/committed field survives inheritance and later retries.
    for (before, after) in [
        (
            old.reserved_science_request_count,
            checkpoint.reserved_science_request_count,
        ),
        (
            old.reserved_science_response_body_bytes,
            checkpoint.reserved_science_response_body_bytes,
        ),
        (
            old.issued_science_request_count,
            checkpoint.issued_science_request_count,
        ),
        (
            old.observed_science_response_body_bytes,
            checkpoint.observed_science_response_body_bytes,
        ),
        (
            old.failed_science_request_count,
            checkpoint.failed_science_request_count,
        ),
        (
            old.unissued_science_reservation_count,
            checkpoint.unissued_science_reservation_count,
        ),
        (
            old.indeterminate_science_request_count,
            checkpoint.indeterminate_science_request_count,
        ),
        (
            old.indeterminate_science_response_body_bytes,
            checkpoint.indeterminate_science_response_body_bytes,
        ),
        (
            old.committed_science_request_count,
            checkpoint.committed_science_request_count,
        ),
        (
            old.committed_science_response_body_bytes,
            checkpoint.committed_science_response_body_bytes,
        ),
        (old.resumed_bytes, checkpoint.resumed_bytes),
    ] {
        if after < before {
            return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
        }
    }
    Ok(NisarValidatedScienceBudgetV2 {
        limits: proof.evidence().new_limits.clone(),
        receipt: target.budget_upgrade_receipt,
    })
}

/// Validation budget for the shared recovery reconciliation. The caller
/// verifies `checkpoint`'s completed material (see `validated_budget`).
#[allow(clippy::too_many_arguments)]
pub(super) fn recovery_budget(
    mode: ScienceRecoveryBudgetModeV2,
    checkpoint: &NisarSubsetScienceCheckpointV2,
    work: &DownloadWork,
    plan: &NisarSubsetPhysicalExecutionPlanV2,
    workspace: &Hdf5RangeWorkspaceV1,
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    path: &str,
    identity: &str,
    binding: &str,
    part_identity: &str,
) -> Result<NisarValidatedScienceBudgetV2, NisarSubsetScienceTransferErrorV2> {
    match mode {
        ScienceRecoveryBudgetModeV2::Effective => validated_budget(
            checkpoint,
            work,
            plan,
            workspace,
            anchor,
            path,
            identity,
            binding,
            part_identity,
        ),
        ScienceRecoveryBudgetModeV2::LegacyUpgradePredecessor => {
            if checkpoint.budget_upgrade_receipt.is_some()
                || anchor.load_budget_upgrade_proof()?.is_some()
            {
                return Err(NisarSubsetScienceTransferErrorV2::RecoveryInvalid);
            }
            NisarValidatedScienceBudgetV2::legacy_upgrade_predecessor(plan)
        }
    }
}

fn stored_anchor_matches(
    stored: &StoredNisarSubsetExecutionAnchorV2,
    work: &DownloadWork,
    record: &NisarSubsetScienceCheckpointAnchorRecordV2,
) -> Result<bool, NisarSubsetScienceTransferErrorV2> {
    Ok(stored.task_id == work.task.task_id
        && stored.item_id == work.item.item_id
        && stored.anchor_kind == insar_store::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
        && stored.lineage_key_sha256 == record.lineage_key_sha256
        && stored.record_sha256 == record.record_sha256
        && stored.canonical_json == record.canonical_json()?)
}

fn load_local_checkpoint(
    workspace: &Hdf5RangeWorkspaceV1,
) -> Result<Option<(String, NisarSubsetScienceCheckpointV2)>, NisarSubsetScienceTransferErrorV2> {
    workspace
        .load_science_checkpoint_v2()
        .map_err(map_workspace_error)?
        .map(|canonical| {
            let checkpoint = parse_checkpoint_canonical_json(&canonical)?;
            Ok((canonical, checkpoint))
        })
        .transpose()
}

/// The production entry calls this before resolving an asset or publishing any
/// progress. All predecessor material is read under the existing workspace lock.
///
/// Only an item whose frozen plan uses the legacy `physical_execution_union_v1`
/// policy is touched here. A non-quiescent legacy predecessor (an ordinary pending
/// anchor transition, a pending genesis or a pending attempt) is first reconciled
/// by the same recovery code the executor uses, validated with the narrow legacy
/// predecessor budget; quiescence is then rechecked before the one-time upgrade.
pub(crate) fn prepare_nisar_budget_upgrade_v2(
    work: &DownloadWork,
    workspace: &Hdf5RangeWorkspaceV1,
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    stored_anchor: Option<&StoredNisarSubsetExecutionAnchorV2>,
    control: &WorkerControl,
) -> Result<(), NisarBudgetMigrationFailureV2> {
    #[cfg(test)]
    let _hash_site = completed_material_hash_ledger::enter("migration_entry");
    let mut reconciliation_started = false;
    prepare_nisar_budget_upgrade_inner(
        work,
        workspace,
        anchor,
        stored_anchor,
        control,
        &mut reconciliation_started,
    )
    .map_err(|mut failure| {
        failure.reconciled = reconciliation_started;
        failure
    })
}

fn prepare_nisar_budget_upgrade_inner(
    work: &DownloadWork,
    workspace: &Hdf5RangeWorkspaceV1,
    anchor: &dyn NisarSubsetScienceCheckpointAnchorV2,
    stored_anchor: Option<&StoredNisarSubsetExecutionAnchorV2>,
    control: &WorkerControl,
    reconciliation_started: &mut bool,
) -> Result<(), NisarBudgetMigrationFailureV2> {
    use NisarBudgetMigrationPhaseV2 as Phase;
    archived_import_active(control).ordinary()?;
    let _lock = workspace
        .acquire_lock()
        .map_err(map_workspace_error)
        .ordinary()?;
    // Routing only, never authority: the retained planning checkpoint must declare
    // the legacy V1 policy before this entry does anything. V2 and acceptance-policy
    // plans (and items without planning yet) keep their unchanged executor
    // recovery; everything is fully verified below before an upgrade.
    let declares_legacy_policy = workspace
        .load_planning_checkpoint_v2()
        .map_err(map_workspace_error)
        .ordinary()?
        .as_deref()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .is_some_and(|value| {
            value["physical_plan"]["combined_budget"]["budget_policy"]
                == "physical_execution_union_v1"
        });
    if !declares_legacy_policy {
        // Rule 5 of the 2026-10-17 decision: these items only check that no proof
        // exists. Any read failure keeps its ordinary code; it is not a refusal.
        return match anchor.load_budget_upgrade_proof().ordinary()? {
            Some(_) => refused(Phase::ProofReconciliation),
            None => Ok(()),
        };
    }
    let Some(stored) = stored_anchor else {
        // No science anchor yet: the ordinary executor owns genesis under the
        // original V1 limits. A proof or local checkpoint without its anchor is
        // invalid and refused before any I/O.
        if anchor
            .load_budget_upgrade_proof()
            .in_phase(Phase::ProofReconciliation)?
            .is_some()
        {
            return refused(Phase::ProofReconciliation);
        }
        if load_local_checkpoint(workspace)
            .in_phase(Phase::PredecessorValidation)?
            .is_some()
        {
            return refused(Phase::PredecessorValidation);
        }
        return Ok(());
    };
    let mut record =
        NisarSubsetScienceCheckpointAnchorRecordV2::from_canonical_json(&stored.canonical_json)
            .in_phase(Phase::PredecessorValidation)?;
    if !stored_anchor_matches(stored, work, &record).in_phase(Phase::PredecessorValidation)? {
        return refused(Phase::PredecessorValidation);
    }
    let (path, identity) = workspace
        .bound_workspace_identity_v2()
        .map_err(map_workspace_error)
        .in_phase(Phase::PlanningValidation)?;
    // Reuse the existing read-only local inventory/source/plan/material verifier;
    // it never binds HTTP, creates a prefix materializer or changes old evidence.
    let planning = crate::nisar_planning_v2::export_archived_planning_v2(work, workspace, control)
        .map_err(|error| map_planning_error(error, control))
        .in_phase(Phase::PlanningValidation)?;
    let plan = planning.physical_plan();
    if !plan.budget_projection().uses_physical_execution_union_v1() {
        return refused(Phase::PlanningValidation);
    }
    let lineage = lineage_key_sha256(work, plan).in_phase(Phase::PredecessorValidation)?;
    if record.lineage_key_sha256 != lineage
        || anchor
            .load(&lineage)
            .in_phase(Phase::PredecessorValidation)?
            .as_ref()
            != Some(&record)
    {
        return refused(Phase::PredecessorValidation);
    }
    let binding = record.execution_binding_sha256.clone();
    let proof = anchor
        .load_budget_upgrade_proof()
        .in_phase(Phase::ProofReconciliation)?;
    let mut local = load_local_checkpoint(workspace).in_phase(Phase::PredecessorValidation)?;
    if proof.is_none() {
        let pending_target = record
            .pending_transition
            .as_ref()
            .map(|pending| parse_checkpoint_canonical_json(&pending.checkpoint_canonical_json))
            .transpose()
            .in_phase(Phase::PredecessorValidation)?;
        let newest = pending_target
            .as_ref()
            .or(local.as_ref().map(|(_, checkpoint)| checkpoint));
        if newest.is_some_and(|checkpoint| checkpoint.budget_upgrade_receipt.is_some()) {
            // A receipt without its immutable Store proof never grants authority.
            return refused(Phase::ProofReconciliation);
        }
        if newest.is_some_and(|checkpoint| {
            checkpoint.state == NisarSubsetScienceCheckpointStateV2::Complete
        }) {
            // Complete legacy science needs no upgrade; the executor validates it.
            return Ok(());
        }
    }
    let (mut part, part_identity, marker) = workspace
        .reopen_science_part_for_writer_v2(plan.source_size())
        .map_err(map_workspace_error)
        .in_phase(Phase::PredecessorValidation)?;
    // Set when the proof is created in this call: the predecessor's completed
    // material is then hashed below under this same workspace lock.
    let mut material_verified_under_lock = false;
    let proof = match proof {
        Some(proof) => proof,
        None => {
            let quiescent = record.pending_transition.is_none()
                && local
                    .as_ref()
                    .is_some_and(|(_, checkpoint)| checkpoint.pending_attempt.is_none());
            // The exact raw state reconciliation is about to settle is retained
            // in the immutable proof; it is never discarded or guessed.
            let pre_reconciliation =
                (!quiescent).then(|| StoreNisarBudgetUpgradePreReconciliationV2 {
                    science_anchor_record_sha256: stored.record_sha256.clone(),
                    science_anchor_canonical_json: stored.canonical_json.clone(),
                    local_checkpoint_canonical_json: local
                        .as_ref()
                        .map(|(canonical, _)| canonical.clone()),
                });
            if !quiescent {
                // Same reconciliation as the executor: commit a valid pending
                // transition (or genesis) and charge a pending attempt as
                // indeterminate/failed. Nothing is discarded or refunded.
                *reconciliation_started = true;
                let (mut reconciled, budget) = recover_or_initialize_checkpoint(
                    anchor,
                    &lineage,
                    &binding,
                    workspace,
                    local.take().map(|(_, checkpoint)| checkpoint),
                    work,
                    plan,
                    &path,
                    &identity,
                    &part_identity,
                    &mut part,
                    ScienceRecoveryBudgetModeV2::LegacyUpgradePredecessor,
                    // The one-time migration keeps its refusal semantics:
                    // its digests run to the end (downloader enhancement D2).
                    &digest_runs_to_the_end,
                )
                .in_phase(Phase::LegacyReconciliation)?;
                reconciled
                    .validate_with_budget(
                        work,
                        plan,
                        &path,
                        &identity,
                        &lineage,
                        &binding,
                        &part_identity,
                        &budget,
                    )
                    .in_phase(Phase::LegacyReconciliation)?;
                finalize_recovered_pending_attempt_durably(
                    anchor,
                    &lineage,
                    workspace,
                    &mut reconciled,
                )
                .in_phase(Phase::LegacyReconciliation)?;
                record = anchor
                    .load(&lineage)
                    .in_phase(Phase::LegacyReconciliation)?
                    .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
                    .in_phase(Phase::LegacyReconciliation)?;
                local = load_local_checkpoint(workspace).in_phase(Phase::LegacyReconciliation)?;
                if local.as_ref().map(|(_, checkpoint)| checkpoint) != Some(&reconciled) {
                    return refused(Phase::LegacyReconciliation);
                }
                if reconciled.state == NisarSubsetScienceCheckpointStateV2::Complete {
                    return Ok(());
                }
            }
            let Some((canonical, predecessor)) = local.as_ref() else {
                return refused(Phase::PredecessorValidation);
            };
            if record.pending_transition.is_some()
                || record.committed_checkpoint_sha256.as_deref()
                    != Some(predecessor.checkpoint_sha256.as_str())
                || record.generation != predecessor.anchor_generation
            {
                return refused(Phase::PredecessorValidation);
            }
            validate_upgrade_source(
                predecessor,
                work,
                plan,
                &path,
                &identity,
                &binding,
                &part_identity,
                &mut part,
            )
            .in_phase(Phase::PredecessorValidation)?;
            let new_limits =
                derive_upgrade_limits(predecessor, plan).in_phase(Phase::BudgetDerivation)?;
            let context = anchor
                .load_budget_upgrade_context()
                .in_phase(Phase::StoreAdmission)?;
            if context.task_id != work.task.task_id
                || context.item_id != work.item.item_id
                || context.item != work.item
                || !stored_anchor_matches(&context.science_anchor, work, &record)
                    .in_phase(Phase::StoreAdmission)?
            {
                return refused(Phase::StoreAdmission);
            }
            let evidence = StoreNisarBudgetUpgradeEvidenceV2 {
                planning_checkpoint_canonical_json: workspace
                    .load_planning_checkpoint_v2()
                    .map_err(map_workspace_error)
                    .in_phase(Phase::PredecessorValidation)?
                    .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
                    .in_phase(Phase::PredecessorValidation)?,
                science_checkpoint_canonical_json: canonical.clone(),
                old_limits: original_limits(plan.budget_projection()),
                new_limits,
                completed_ranges_sha256: digest_serializable(&predecessor.completed_ranges)
                    .in_phase(Phase::BudgetDerivation)?,
                remaining_ranges_sha256: digest_serializable(&remaining_ranges(predecessor))
                    .in_phase(Phase::BudgetDerivation)?,
                pre_reconciliation,
            };
            let proof = StoredNisarBudgetUpgradeProofV2::new(
                context,
                crate::scheduler::timestamp_now(),
                evidence,
            )
            .map_err(|_| NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
            .in_phase(Phase::StoreAdmission)?;
            let target = target_checkpoint(&proof, predecessor).in_phase(Phase::ReceiptCommit)?;
            let pending = record
                .with_pending(&target)
                .in_phase(Phase::ReceiptCommit)?;
            if workspace
                .science_part_mutation_marker_for_writer_v2(&part)
                .map_err(map_workspace_error)
                .in_phase(Phase::PredecessorValidation)?
                != marker
            {
                return refused(Phase::PredecessorValidation);
            }
            archived_import_active(control).ordinary()?;
            #[cfg(test)]
            control.run_budget_migration_hook_for_test(
                crate::scheduler::NisarBudgetMigrationStageForTest::BeforeStorePrepare,
            );
            if !anchor
                .prepare_budget_upgrade(&proof, &pending)
                .in_phase(Phase::StoreAdmission)?
            {
                return refused(Phase::StoreAdmission);
            }
            #[cfg(test)]
            control.run_budget_migration_hook_for_test(
                crate::scheduler::NisarBudgetMigrationStageForTest::AfterStorePrepare,
            );
            material_verified_under_lock = true;
            proof
        }
    };
    let Some((_, mut current_local)) = local else {
        return refused(Phase::ProofReconciliation);
    };
    let old = validate_proof_source(
        &proof,
        work,
        plan,
        workspace,
        &path,
        &identity,
        &binding,
        &part_identity,
    )
    .in_phase(Phase::ProofReconciliation)?;
    let target = target_checkpoint(&proof, &old).in_phase(Phase::ProofReconciliation)?;
    let predecessor = NisarSubsetScienceCheckpointAnchorRecordV2::from_canonical_json(
        &proof.context().science_anchor.canonical_json,
    )
    .in_phase(Phase::ProofReconciliation)?;
    let pending = predecessor
        .with_pending(&target)
        .in_phase(Phase::ProofReconciliation)?;
    let current = anchor
        .load(&lineage)
        .in_phase(Phase::ProofReconciliation)?
        .ok_or(NisarSubsetScienceTransferErrorV2::RecoveryInvalid)
        .in_phase(Phase::ProofReconciliation)?;
    let committed_in_this_call = material_verified_under_lock && current == pending;
    if current == pending {
        if current_local != old && current_local != target {
            return refused(Phase::ReceiptCommit);
        }
        if current_local != target {
            persist_checkpoint(workspace, &target).in_phase(Phase::ReceiptCommit)?;
        }
        #[cfg(test)]
        control.run_budget_migration_hook_for_test(
            crate::scheduler::NisarBudgetMigrationStageForTest::AfterLocalTarget,
        );
        let committed = pending.commit_pending().in_phase(Phase::ReceiptCommit)?;
        if !anchor
            .compare_and_swap(&lineage, Some(&pending), &committed)
            .in_phase(Phase::ReceiptCommit)?
        {
            return refused(Phase::ReceiptCommit);
        }
        #[cfg(test)]
        control.run_budget_migration_hook_for_test(
            crate::scheduler::NisarBudgetMigrationStageForTest::AfterCommit,
        );
        current_local = target;
    } else if current == predecessor {
        // Proof without its atomic prepare transition is not successful migration.
        return refused(Phase::ProofReconciliation);
    }
    let (checkpoint, budget) = if committed_in_this_call {
        // First upgrade: the target carries exactly the predecessor's completed
        // Ranges, whose bytes were hashed above under this same workspace lock;
        // the part mutation marker is rechecked below. Verify the effective
        // budget and the committed anchor without hashing the same bytes again.
        let budget = validated_budget(
            &current_local,
            work,
            plan,
            workspace,
            anchor,
            &path,
            &identity,
            &binding,
            &part_identity,
        )
        .in_phase(Phase::EffectiveBudgetValidation)?;
        (current_local, budget)
    } else {
        // Includes later ordinary science pending transitions. This verifies the
        // effective budget, local bytes and committed Store anchor before guard
        // clear (the one completed-material pass of this continuation).
        recover_or_initialize_checkpoint(
            anchor,
            &lineage,
            &binding,
            workspace,
            Some(current_local),
            work,
            plan,
            &path,
            &identity,
            &part_identity,
            &mut part,
            ScienceRecoveryBudgetModeV2::Effective,
            &digest_runs_to_the_end,
        )
        .in_phase(Phase::EffectiveBudgetValidation)?
    };
    checkpoint
        .validate_with_budget(
            work,
            plan,
            &path,
            &identity,
            &lineage,
            &binding,
            &part_identity,
            &budget,
        )
        .in_phase(Phase::EffectiveBudgetValidation)?;
    let persisted = load_local_checkpoint(workspace).in_phase(Phase::EffectiveBudgetValidation)?;
    if persisted.as_ref().map(|(_, persisted)| persisted) != Some(&checkpoint)
        || workspace
            .science_part_mutation_marker_for_writer_v2(&part)
            .map_err(map_workspace_error)
            .in_phase(Phase::EffectiveBudgetValidation)?
            != marker
    {
        return refused(Phase::EffectiveBudgetValidation);
    }
    reconcile_checkpoint_anchor(anchor, &lineage, &checkpoint)
        .in_phase(Phase::EffectiveBudgetValidation)?;
    archived_import_active(control).ordinary()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_inheritance_owner_935_of_934_retains_charges_and_only_budgets_missing_ranges() {
        let old = NisarBudgetUpgradeLimitsV2 {
            max_request_count: 965,
            max_science_request_count: 934,
            max_response_body_bytes: 1_186_766_086,
        };
        let next = inherited_limits(&old, 31, 195_035_138, 935, 493_865_474, 2, 2_000_000).unwrap();
        assert_eq!(
            next,
            NisarBudgetUpgradeLimitsV2 {
                max_request_count: 1054,
                max_science_request_count: 1023,
                max_response_body_bytes: old.max_response_body_bytes
            }
        );
        // A retained near-exhausted body charge is not refunded or reset.
        let next = inherited_limits(&old, 31, 195_035_138, 934, 990_000_000, 2, 2_000_000).unwrap();
        assert_eq!(next.max_response_body_bytes, 1_189_035_138);
        assert_eq!(old.max_science_request_count, 934);
    }

    /// Review P2: only a local validation result is a terminal refusal. Transient
    /// Store/filesystem failures (for example a busy or failing context load or
    /// admission transaction) and lifecycle stops keep the ordinary path.
    #[test]
    fn budget_migration_classifies_transient_store_failures_and_stops_as_ordinary() {
        use NisarBudgetMigrationPhaseV2 as Phase;
        use NisarSubsetScienceTransferErrorV2 as Error;
        for phase in [Phase::StoreAdmission, Phase::ProofReconciliation] {
            for error in [Error::Persistence, Error::Paused, Error::Cancelled] {
                let failure = Err::<(), _>(error).in_phase(phase).unwrap_err();
                assert_eq!(failure.refusal, None, "{error:?} in {phase:?}");
                assert_eq!(failure.error, error);
            }
            // Review 2026-10-17 round 2, P1-2: a Store refusal that waiting does
            // not change keeps its Store code and is a terminal refusal.
            for error in [
                Error::RecoveryInvalid,
                Error::AuthorityMismatch,
                Error::CombinedBudgetExceeded,
                Error::StoreRecordRejected {
                    store_code: "V3_STORE_NISAR_BUDGET_UPGRADE_PROOF_VERSION_UNSUPPORTED",
                },
                Error::StoreRecordRejected {
                    store_code: "V3_STORE_NOT_FOUND",
                },
            ] {
                assert_eq!(
                    Err::<(), _>(error).in_phase(phase).unwrap_err().refusal,
                    Some(phase),
                    "{error:?} in {phase:?}"
                );
            }
        }
    }

    /// Review 2026-10-17 round 3, P2-2: a full disk or an I/O error of the local
    /// Store at the migration entry is transient, never a terminal refusal.
    #[test]
    fn budget_migration_keeps_a_local_storage_failure_on_the_ordinary_path() {
        let error = NisarSubsetScienceTransferErrorV2::LocalStorageFailure {
            store_code: "V3_STORE_DATABASE_ERROR",
            cause: "disk_full",
        };
        for phase in [
            NisarBudgetMigrationPhaseV2::StoreAdmission,
            NisarBudgetMigrationPhaseV2::ProofReconciliation,
            NisarBudgetMigrationPhaseV2::PredecessorValidation,
        ] {
            let failure = Err::<(), _>(error).in_phase(phase).unwrap_err();
            assert_eq!(failure.refusal, None, "{phase:?}");
            assert_eq!(failure.error, error);
        }
    }

    #[test]
    fn budget_inheritance_rejects_excessive_legacy_overrun_body_overrun_and_overflow() {
        let old = NisarBudgetUpgradeLimitsV2 {
            max_request_count: 965,
            max_science_request_count: 934,
            max_response_body_bytes: 1_186_766_086,
        };
        assert!(inherited_limits(&old, 31, 195_035_138, 956, 1, 2, 2).is_err());
        assert!(inherited_limits(&old, 31, 195_035_138, 935, 991_730_949, 2, 2).is_err());
        assert!(inherited_limits(&old, 31, 195_035_138, 935, 1, u64::MAX, 2).is_err());
        assert!(inherited_limits(&old, 31, 195_035_138, 935, 1, 2, u64::MAX).is_err());
        assert!(inherited_limits(&old, 31, 195_035_138, 935, 1, 0, 0).is_err());
    }
}
