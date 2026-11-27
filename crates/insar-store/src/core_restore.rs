use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use insar_core::{
    AppendTaskFromSourceBatchRequest, AppendTaskFromWorkbenchRequest,
    CreateTaskFromHistoricalSourceBatchRequest, CreateTaskFromWorkbenchRequest, DownloadFailure,
    DownloadItemProgress, DownloadItemResult, DownloadItemStatus, DownloadLayout, DownloadOutcome,
    FrozenSearchSourceAttempt, FrozenSearchSourceAttemptOutcome, FrozenSearchSourceFallbackReason,
    FrozenSearchStats, RegionAoiState, SearchWorkbenchState, SnapshotItem, StoreCreateTaskRequest,
    StoreTaskPage, StoreTaskPageItem, StoreTaskPageRequest, StoreTaskSourceBatchPage,
    StoreTaskSourceBatchPageItem, StoreTaskSourceBatchPageRequest, TaskLifecycleStatus,
    TaskRuntimeSnapshot, WorkbenchSelectionCommand, WorkbenchSelectionCommandKind,
    WorkbenchSelectionSummary, WorkspaceContextState,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::repository::{
    create_task_in_transaction, hydrate_task_snapshot_for_runtime, load_batch,
    load_compact_task_connection, load_explicit_task_source_item_ids, load_task_connection,
    load_task_membership_additions_batched, sqlite_integer, validate_runtime_item_graph,
    validate_runtime_lifecycle_partition,
};
use crate::runtime::{
    append_task_membership_in_transaction, reject_closed_task, reject_tombstoned_task,
    scheduler_generation_uncertain_failure, SCHEDULER_GENERATION_UNCERTAIN,
};
use crate::{
    decode_any_snapshot, validate_task_snapshot_row_envelope, CompactTaskSnapshotV2, StoreError,
    StoredTask, StoredTaskSnapshot, TaskSnapshotRowEnvelope, TaskStore, STORE_SCHEMA_VERSION,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreReadiness {
    pub schema_version: i64,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LegacyTaskCompatibilityStatus {
    AdaptedReadOnly,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LegacyTaskRestoreReasonCode {
    Sentinel1SlcDisplayFieldsAdapted,
    MalformedSnapshot,
    UnsupportedPersistenceVersion,
    TaskEnvelopeMismatch,
    MissingOrInvalidRuntime,
    RuntimeEnvelopeMismatch,
    AmbiguousLegacyEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LegacyTaskRestoreWarning {
    pub task_id: String,
    pub provider_id: String,
    pub created_at: String,
    pub status: Option<TaskLifecycleStatus>,
    pub reason_code: LegacyTaskRestoreReasonCode,
    pub compatibility_status: LegacyTaskCompatibilityStatus,
}

/// Downloader D3-2: where a task's local pre-scan stands and what it
/// reused (counts only, no paths).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskLocalScanSummary {
    /// "pending", "scanning", "done" or "stopped"; "none" when the task was
    /// never pre-scanned and its workers' gates reused files.
    pub state: String,
    pub scanned_items: u64,
    pub total_items: u64,
    /// Items finished by reusing an existing file.
    pub reused_count: u64,
    /// Of those, reused on size and a readable format only.
    pub unverified_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreTaskSummary {
    pub task_id: String,
    pub batch_id: String,
    pub provider_id: String,
    #[serde(default)]
    pub dataset_id: Option<String>,
    /// Store-owned scheduler concurrency. Historical rows may not retain the
    /// runtime authority, so the projection is explicitly nullable.
    #[serde(default)]
    pub concurrency: Option<u64>,
    /// Stable product task classification used by the download cards.
    #[serde(default)]
    pub task_type: Option<String>,
    pub created_at: String,
    pub status: TaskLifecycleStatus,
    pub revision: u64,
    pub updated_at: String,
    pub selected_count: usize,
    /// Original batch delivery request count. This is creation admission
    /// telemetry, not a transfer lifecycle count.
    #[serde(default)]
    pub delivery_requested_count: Option<usize>,
    /// Items rejected before the transfer task was created. These items never
    /// enter the scheduler and must not be merged into failed_count.
    #[serde(default)]
    pub delivery_rejected_count: Option<usize>,
    pub checked_count: usize,
    pub active_count: usize,
    pub paused_count: usize,
    pub queued_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub cancelled_count: usize,
    pub terminal_bytes: u64,
    pub active_bytes: u64,
    pub other_bytes: u64,
    pub transferred_bytes: u64,
    pub total_bytes: Option<u64>,
    /// Immutable sum of every selected item's trusted frozen source size.
    /// This is display-only reference data, not progress or execution authority.
    #[serde(default)]
    pub source_total_bytes: Option<u64>,
    pub bytes_per_second: f64,
    pub transfer_active_millis: u64,
    /// Actual HTTP source bytes, independent from output/artifact telemetry.
    /// Absent on tasks created before native network measurement was available.
    #[serde(default)]
    pub network_bytes_received: Option<u64>,
    /// Bytes in committed science ranges that are safe to reuse after restart.
    /// This is checkpoint authority, not HTTP traffic or output-file progress.
    #[serde(default)]
    pub reusable_verified_bytes: Option<u64>,
    /// Verified bytes actually reused by the current committed checkpoint.
    #[serde(default)]
    pub reused_verified_bytes: Option<u64>,
    pub active_item: Option<StoreTaskActiveItemSummary>,
    /// Active items by what their current progress row reports. Review
    /// 2026-10-17 round 2, P2-2: a NISAR subset worker publishes a zero-rate
    /// row without a total exactly when its science transfer finished, so the
    /// task card reads its output phase from here instead of from a persisted
    /// total.
    #[serde(default)]
    pub active_progress: StoreTaskActiveProgressSummary,
    /// Distinct stable failure codes of the task's failed items (review
    /// 2026-10-17, L4): codes only, never provider text.
    #[serde(default)]
    pub failure_codes: Vec<StoreTaskFailureCode>,
    /// Downloader D3-2: the local pre-scan and the reuse it decided; None
    /// when the task was never scanned and reused nothing.
    #[serde(default)]
    pub local_scan: Option<StoreTaskLocalScanSummary>,
    pub output_root: PathBuf,
    pub layout: DownloadLayout,
    pub operation: Option<StoreTaskOperationSummary>,
    /// Orbit scheduled-fetch S4 (plan B): the owning image task this
    /// companion task was derived from, projected from the frozen operation
    /// payload's `source_task_id`; `None` for ordinary tasks and old
    /// payloads.
    #[serde(default)]
    pub source_task_id: Option<String>,
    /// Orbit scheduled-fetch S4: the auxiliary row kind of this task (e.g.
    /// `AUX_POEORB` for a companion precise-orbit task); `None` for
    /// ordinary tasks and old payloads. Both the queue task and the
    /// archived history item expose it, so an archived companion task
    /// keeps its grouping identity in history.
    #[serde(default)]
    pub auxiliary_kind: Option<String>,
    pub actionable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskActiveItemSummary {
    pub item_id: String,
    pub bytes_completed: u64,
    pub total_bytes: Option<u64>,
}

/// Active items grouped by their current progress row: none yet in this run,
/// a positive transfer rate, a zero rate (no transfer active), or a zero-rate
/// row whose status is `queued`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskActiveProgressSummary {
    pub without_progress: u64,
    pub transferring: u64,
    pub idle: u64,
    /// Downloader enhancement D1 (2026-10-18): the item waits for a local
    /// resource and transfers nothing; a NISAR subset item waits for the
    /// output writer (another writer of its artifact or an output slot). Not
    /// counted in `idle`.
    #[serde(default)]
    pub waiting: u64,
}

/// One distinct failure code of a task's failed items. `local` is set when
/// every such failure's diagnostic declares a local-only cause (its tail
/// carries `source=local`). Invalid or missing codes are reported as
/// `UNKNOWN_FAILURE_CODE`; more than eight codes end with `MORE_FAILURE_CODES`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskFailureCode {
    pub code: String,
    pub local: bool,
    /// Every such failure's diagnostic declares that the server certificate
    /// could not be verified (its tail carries `source=tls_certificate`; L5
    /// recheck 2026-10-19). Omitted when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tls_certificate: bool,
    /// Every such failure's diagnostic declares a network transport cause (its
    /// tail carries `source=network`; downloader enhancement D1d). Omitted when
    /// false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub network: bool,
}

/// What every failure with one code declares about its cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TaskFailureCodeFacts {
    local: bool,
    tls_certificate: bool,
    network: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct StoreTaskTelemetrySummary {
    terminal_bytes: u64,
    active_bytes: u64,
    other_bytes: u64,
    transferred_bytes: u64,
    total_bytes: Option<u64>,
    bytes_per_second: f64,
    transfer_active_millis: u64,
    network_bytes_received: Option<u64>,
    active_item: Option<StoreTaskActiveItemSummary>,
    active_progress: StoreTaskActiveProgressSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StoreNisarSubsetRecoveryTelemetrySummary {
    reusable_verified_bytes: u64,
    reused_verified_bytes: u64,
}

const NISAR_SUBSET_RECOVERY_TELEMETRY_CONTRACT_V1: &str =
    "nisar_subset_committed_recovery_telemetry_v1";
const NISAR_SUBSET_RECOVERY_TELEMETRY_DIGEST_CONTRACT_V1: &str =
    "nisar_subset_science_checkpoint_anchor_record_v2_recovery_telemetry_v1";
const NISAR_SUBSET_SCIENCE_CHECKPOINT_MAX_CANONICAL_BYTES: usize = 16 * 1024 * 1024;
const NISAR_SUBSET_SCIENCE_ANCHOR_MAX_CANONICAL_BYTES: usize = 40 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreNisarSubsetSciencePendingAnchorTransitionV2 {
    predecessor_checkpoint_sha256: Option<String>,
    predecessor_generation: Option<u64>,
    checkpoint_generation: u64,
    checkpoint_sha256: String,
    checkpoint_canonical_json: String,
    transition_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreNisarSubsetCommittedRecoveryTelemetryV1 {
    contract: String,
    checkpoint_sha256: String,
    reusable_verified_bytes: u64,
    reused_verified_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreNisarSubsetScienceCheckpointAnchorRecordV2 {
    lineage_key_sha256: String,
    execution_binding_sha256: String,
    generation: u64,
    committed_checkpoint_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    committed_recovery_telemetry: Option<StoreNisarSubsetCommittedRecoveryTelemetryV1>,
    pending_transition: Option<StoreNisarSubsetSciencePendingAnchorTransitionV2>,
    record_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskOperationSummary {
    pub operation_id: String,
    pub kind: String,
    pub phase: String,
    pub region_id: String,
    pub approved_worker_operation: String,
    #[serde(default)]
    pub source_provider: Option<String>,
    #[serde(default)]
    pub input_path: Option<String>,
    #[serde(default)]
    pub source_vertical_datum: Option<String>,
    #[serde(default)]
    pub convert: Option<bool>,
}

/// Exact product-owned identity required to resolve a formal DEM preparation
/// as the source of a conversion handoff. Operation kind, phase, and worker
/// are deliberately server-owned constants rather than caller-controlled
/// selectors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreDemConversionSourceRequest {
    pub region_id: String,
    pub dataset: String,
    pub provider: String,
    pub resource_id: String,
    pub capability_id: String,
}

/// Fixed scalar handoff evidence. No task snapshot, runtime JSON, or history
/// page crosses this lookup boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreDemConversionSource {
    pub task_id: String,
    pub revision: u64,
    pub region_id: String,
    pub dataset: String,
    pub provider: String,
    pub resource_id: String,
    pub capability_id: String,
    pub input_path: PathBuf,
    pub output_root: PathBuf,
}

/// Store-owned selected DEM catalogue identity. This deliberately contains
/// only canonical bounded scalars; credentials and provider request state do
/// not belong in the restore contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreDemSourceIdentity {
    pub dataset: String,
    pub provider: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreWorkbenchQuerySummary {
    pub provider: Option<String>,
    pub dataset: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    /// Fixed search target restored without transporting the full query.
    pub target_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StoreWorkbenchSummaryReasonCode {
    CandidatePayloadDeferred,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreWorkbenchSummary {
    pub batch_id: String,
    pub provider_id: String,
    pub created_at: String,
    pub aoi_name: String,
    pub query: StoreWorkbenchQuerySummary,
    /// Optional fixed-size facts from the frozen provider response. Older
    /// batches keep this absent rather than inventing a provider total.
    pub search_stats: Option<StoreWorkbenchSearchStats>,
    pub candidate_count: usize,
    pub checked_count: usize,
    pub current_page_item_count: usize,
    pub page_index: u64,
    pub page_size: u64,
    pub revision: u64,
    pub deferred: bool,
    pub reason_code: Option<StoreWorkbenchSummaryReasonCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreWorkbenchSearchStats {
    pub matched_count: Option<u64>,
    pub provider_returned_count: u64,
    pub frozen_candidate_count: u64,
    pub truncated: bool,
    pub source_attempts: Vec<StoreWorkbenchSourceAttempt>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreWorkbenchSourceAttempt {
    pub provider_id: String,
    pub native_collection_id: String,
    pub outcome: String,
    pub fallback_reason: Option<String>,
}

fn project_workbench_source_attempt(
    attempt: &FrozenSearchSourceAttempt,
) -> StoreWorkbenchSourceAttempt {
    StoreWorkbenchSourceAttempt {
        provider_id: attempt.provider_id.clone(),
        native_collection_id: attempt.native_collection_id.clone(),
        outcome: match attempt.outcome {
            FrozenSearchSourceAttemptOutcome::Fallback => "fallback",
            FrozenSearchSourceAttemptOutcome::Succeeded => "succeeded",
        }
        .to_owned(),
        fallback_reason: attempt.fallback_reason.as_ref().map(|reason| {
            match reason {
                FrozenSearchSourceFallbackReason::TransientNetwork => "transient_network",
                FrozenSearchSourceFallbackReason::RateLimited => "rate_limited",
                FrozenSearchSourceFallbackReason::ServerError => "server_error",
            }
            .to_owned()
        }),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreCoreRestore {
    pub readiness: StoreReadiness,
    pub workspace_context: Option<WorkspaceContextState>,
    pub region_aoi: Option<RegionAoiState>,
    pub region_aois: Vec<RegionAoiState>,
    pub dem_source_identity: Option<StoreDemSourceIdentity>,
    pub tasks: Vec<StoreTaskSummary>,
    pub legacy_task_warnings: Vec<LegacyTaskRestoreWarning>,
    pub workbench: Option<StoreWorkbenchSummary>,
}

pub const DEFAULT_WORKBENCH_PAGE_SIZE: u64 = 250;
pub const MAX_WORKBENCH_PAGE_SIZE: u64 = 250;
/// History is deliberately a small, separately paged read model. It must not
/// turn the status hydrate into a terminal-task restore.
pub const DEFAULT_TASK_HISTORY_PAGE_SIZE: u64 = 40;
pub const MAX_TASK_HISTORY_PAGE_SIZE: u64 = 100;
const MIN_HISTORY_CURSOR_EPOCH_MS: i64 = -62_167_219_200_000;
const MAX_HISTORY_CURSOR_EPOCH_MS: i64 = 253_402_300_799_999;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskHistoryPageRequest {
    pub page_size: u64,
    /// Exclusive stable keyset position. Both members are required together.
    pub after_archived_at_epoch_ms: Option<i64>,
    pub after_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreTaskHistoryPage {
    pub page_size: u64,
    pub has_more: bool,
    pub next_cursor: Option<StoreTaskHistoryCursor>,
    /// Fixed summaries only. Task items remain behind `load_task_page`.
    pub items: Vec<StoreTaskHistoryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskHistoryCursor {
    pub archived_at_epoch_ms: i64,
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreTaskHistoryItem {
    pub archived_at: String,
    #[serde(flatten)]
    pub task: StoreTaskSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreWorkbenchPageRequest {
    pub batch_id: String,
    pub expected_revision: u64,
    pub page_index: u64,
    pub page_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreWorkbenchPageItem {
    pub ordinal: u64,
    pub item: SnapshotItem,
    pub checked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreWorkbenchPage {
    pub batch_id: String,
    pub revision: u64,
    pub page_index: u64,
    pub page_size: u64,
    pub total_count: usize,
    pub checked_count: usize,
    pub items: Vec<StoreWorkbenchPageItem>,
}

#[derive(Debug)]
struct TaskRow {
    task_id: String,
    batch_id: String,
    provider_id: String,
    created_at: String,
    output_root: PathBuf,
    layout: Option<DownloadLayout>,
    snapshot_json: String,
    runtime_status: Option<String>,
    runtime_revision: Option<i64>,
    runtime_updated_at: Option<String>,
    runtime_json: Option<String>,
    membership_additions: Option<Vec<SnapshotItem>>,
}

enum TaskProjection {
    Compatible(Box<StoreTaskSummary>),
    Degraded(LegacyTaskRestoreWarning),
}

pub(crate) fn task_page_item_status(
    runtime: &TaskRuntimeSnapshot,
    item_id: &str,
    _progress: Option<&DownloadItemProgress>,
    _result: Option<&DownloadItemResult>,
) -> DownloadItemStatus {
    if runtime
        .completed_item_ids
        .iter()
        .any(|value| value == item_id)
    {
        return DownloadItemStatus::Completed;
    }
    if runtime.failed_item_ids.iter().any(|value| value == item_id) {
        return DownloadItemStatus::Failed;
    }
    if runtime
        .skipped_item_ids
        .iter()
        .any(|value| value == item_id)
    {
        return DownloadItemStatus::Skipped;
    }
    if runtime
        .cancelled_item_ids
        .iter()
        .any(|value| value == item_id)
    {
        return DownloadItemStatus::Cancelled;
    }
    if runtime.paused_item_ids.iter().any(|value| value == item_id) {
        DownloadItemStatus::Paused
    } else if runtime.active_item_ids.iter().any(|value| value == item_id) {
        if runtime.task_paused {
            DownloadItemStatus::Paused
        } else {
            DownloadItemStatus::Active
        }
    } else {
        DownloadItemStatus::Queued
    }
}

fn public_download_failure(result: Option<&DownloadItemResult>) -> Option<DownloadFailure> {
    result
        .and_then(|value| value.failure.clone())
        .filter(|failure| {
            !matches!(
                failure.code.as_str(),
                "NISAR_SUBSET_V2_PAUSED" | "NISAR_SUBSET_V2_CANCELLED"
            )
        })
}

fn is_nisar_subset_v2_item(item: &SnapshotItem) -> bool {
    item.metadata.get("delivery_kind").and_then(Value::as_str) == Some("remote_derived_subset_v2")
}

/// Review 2026-10-17 round 3, P2-3: the failure recorded on an item's latest
/// `mark_failed` event, when that event carries one. Only a NISAR
/// budget-migration rejection records a failure on its event; when that
/// rejection could not retain its refusal evidence, the prior download result
/// row stays in place (for example an earlier provider failure), so this event
/// is the item's current failure. A later ordinary failure records none on its
/// event, and the result row applies again. An unreadable record also falls
/// back to the row. Same rule as `TASK_FAILURE_CODES_SQL`.
fn latest_failure_event_failure(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
) -> Result<Option<DownloadFailure>, StoreError> {
    let error_json = connection
        .query_row(
            "SELECT error_json FROM task_events \
             WHERE task_id = ?1 AND item_id = ?2 AND event_type = 'mark_failed' \
             ORDER BY sequence DESC LIMIT 1",
            params![task_id, item_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    Ok(error_json.and_then(|raw| {
        let value: Value = serde_json::from_str(&raw).ok()?;
        if value.get("code").and_then(Value::as_str) == Some(SCHEDULER_GENERATION_UNCERTAIN) {
            return Some(scheduler_generation_uncertain_failure());
        }
        serde_json::from_value::<DownloadFailure>(value)
            .ok()
            .filter(|failure| failure.validate().is_ok())
    }))
}

#[cfg(test)]
mod public_download_failure_tests {
    use super::*;

    fn result(code: &str) -> DownloadItemResult {
        DownloadItemResult {
            task_id: "task".to_owned(),
            item_id: "item".to_owned(),
            outcome: DownloadOutcome::Failed,
            destination: PathBuf::from("output"),
            bytes_written: 0,
            attempt_count: 1,
            failure: Some(DownloadFailure {
                code: code.to_owned(),
                reason_zh: "diagnostic".to_owned(),
                retryable: false,
            }),
            finished_at: "2026-10-16T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn hides_only_legacy_synthetic_stop_failures() {
        for code in ["NISAR_SUBSET_V2_PAUSED", "NISAR_SUBSET_V2_CANCELLED"] {
            let result = result(code);
            assert!(public_download_failure(Some(&result)).is_none());
        }
        let real = result("NISAR_SUBSET_V2_TRANSPORT_FAILED");
        assert_eq!(
            public_download_failure(Some(&real))
                .expect("real provider or transport failure must remain visible")
                .code,
            "NISAR_SUBSET_V2_TRANSPORT_FAILED"
        );
    }
}

pub(crate) fn aggregate_task_item_status(statuses: &[DownloadItemStatus]) -> DownloadItemStatus {
    for status in [
        DownloadItemStatus::Failed,
        DownloadItemStatus::Active,
        DownloadItemStatus::Paused,
        DownloadItemStatus::Queued,
        DownloadItemStatus::Interrupted,
        DownloadItemStatus::Cancelled,
        DownloadItemStatus::Skipped,
        DownloadItemStatus::Completed,
    ] {
        if statuses.iter().any(|candidate| candidate == &status) {
            return status;
        }
    }
    DownloadItemStatus::Queued
}

impl TaskStore {
    pub fn core_restore(&self, include_terminal: bool) -> Result<StoreCoreRestore, StoreError> {
        let readiness = StoreReadiness {
            schema_version: self
                .connection
                .pragma_query_value(None, "user_version", |row| row.get(0))?,
            ready: true,
        };
        if readiness.schema_version != STORE_SCHEMA_VERSION {
            return Err(StoreError::invalid("Store schema is not ready"));
        }

        let workspace_context = self.load_workspace_context()?;
        let (region_aoi, region_aois) = match workspace_context.as_ref() {
            Some(context) => (
                context
                    .region_id
                    .as_deref()
                    .map(|region_id| {
                        self.load_region_aoi_for_workspace(&context.workspace_id, region_id)
                    })
                    .transpose()?
                    .flatten(),
                self.load_region_aois(&context.workspace_id)?,
            ),
            None => (None, Vec::new()),
        };

        let mut tasks = Vec::new();
        let mut legacy_task_warnings = Vec::new();
        for row in load_task_rows(&self.connection, include_terminal)? {
            match project_task(row) {
                TaskProjection::Compatible(summary) => tasks.push(*summary),
                TaskProjection::Degraded(warning) => legacy_task_warnings.push(warning),
            }
        }
        let task_ids = tasks
            .iter()
            .map(|task| task.task_id.clone())
            .collect::<Vec<_>>();
        let telemetry_by_task = load_task_telemetries(&self.connection, &task_ids)?;
        let nisar_subset_task_ids = tasks
            .iter()
            .filter(|task| task.task_type.as_deref() == Some("nisar_subset"))
            .map(|task| task.task_id.clone())
            .collect::<Vec<_>>();
        let mut failure_codes_by_task =
            load_task_failure_codes(&self.connection, &task_ids, &nisar_subset_task_ids)?;
        let recovery_by_task =
            load_nisar_subset_recovery_telemetries(&self.connection, &nisar_subset_task_ids)?;
        let mut local_scan_by_task =
            crate::local_reuse::load_task_local_scan_summaries(&self.connection, &task_ids)?;
        for summary in &mut tasks {
            if let Some(telemetry) = telemetry_by_task.get(&summary.task_id) {
                summary.terminal_bytes = telemetry.terminal_bytes;
                summary.active_bytes = telemetry.active_bytes;
                summary.other_bytes = telemetry.other_bytes;
                summary.transferred_bytes = telemetry.transferred_bytes;
                summary.total_bytes = telemetry.total_bytes;
                summary.bytes_per_second = telemetry.bytes_per_second;
                summary.transfer_active_millis = telemetry.transfer_active_millis;
                summary.network_bytes_received = telemetry.network_bytes_received;
                summary.active_item = telemetry.active_item.clone();
                summary.active_progress = telemetry.active_progress;
            }
            if let Some(failure_codes) = failure_codes_by_task.remove(&summary.task_id) {
                summary.failure_codes = failure_codes;
            }
            if let Some(recovery) = recovery_by_task.get(&summary.task_id) {
                summary.reusable_verified_bytes = Some(recovery.reusable_verified_bytes);
                summary.reused_verified_bytes = Some(recovery.reused_verified_bytes);
            }
            summary.local_scan = local_scan_by_task.remove(&summary.task_id);
        }

        Ok(StoreCoreRestore {
            readiness,
            workspace_context,
            region_aoi,
            region_aois,
            dem_source_identity: load_dem_source_identity(&self.connection)?,
            tasks,
            legacy_task_warnings,
            workbench: load_workbench_summary(&self.connection)?,
        })
    }

    /// Persist one canonical selection under the Store's single mutation
    /// lease. A rejected selection cannot replace the last confirmed row.
    pub fn save_dem_source_identity(
        &mut self,
        identity: &StoreDemSourceIdentity,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_dem_source_identity_in_transaction(&transaction, identity)?;
        transaction.commit()?;
        Ok(())
    }

    /// Resolve the newest completed formal DEM preparation across live tasks
    /// and validated materialized archive summaries. This is one bounded
    /// scalar UNION with a deterministic LIMIT 1; task JSON never leaves the
    /// Store and archived rows rely on the same materialization authority as
    /// CoreRestore/task-page actionability.
    pub fn resolve_dem_conversion_source(
        &self,
        request: &StoreDemConversionSourceRequest,
    ) -> Result<Option<StoreDemConversionSource>, StoreError> {
        if [
            &request.region_id,
            &request.dataset,
            &request.provider,
            &request.resource_id,
            &request.capability_id,
        ]
        .into_iter()
        .any(|value| !valid_history_task_id(value))
        {
            return Err(StoreError::invalid(
                "DEM conversion source identity is invalid",
            ));
        }
        let row = self
            .connection
            .query_row(
                r#"WITH candidates(
                    task_id, revision, observed_epoch_ms, region_id, dataset, provider,
                    resource_id, capability_id, input_path, output_root
                ) AS (
                    SELECT s.task_id, r.revision,
                           CAST(ROUND((julianday(r.updated_at) - 2440587.5) * 86400000.0) AS INTEGER),
                           json_extract(s.snapshot_json, '$.operation.region_id'),
                           json_extract(s.snapshot_json, '$.operation.payload.dataset'),
                           json_extract(s.snapshot_json, '$.operation.payload.provider'),
                           json_extract(s.snapshot_json, '$.operation.payload.resource_id'),
                           json_extract(s.snapshot_json, '$.operation.payload.capability_id'),
                           json_extract(s.snapshot_json, '$.operation.payload.input_path'),
                           s.output_root
                    FROM task_snapshots s
                    JOIN task_runtime r ON r.task_id = s.task_id
                    LEFT JOIN task_history h ON h.task_id = s.task_id
                    LEFT JOIN task_tombstones d ON d.task_id = s.task_id
                    WHERE h.task_id IS NULL AND d.task_id IS NULL
                      AND length(s.task_id) BETWEEN 1 AND 128
                      AND r.status = 'completed' AND r.revision >= 0
                      AND julianday(r.updated_at) IS NOT NULL
                      AND json_valid(s.snapshot_json) AND json_type(s.snapshot_json, '$') = 'object'
                      AND json_extract(s.snapshot_json, '$.task_id') = s.task_id
                      AND json_extract(s.snapshot_json, '$.batch_id') = s.batch_id
                      AND json_extract(s.snapshot_json, '$.provider_id') = s.provider_id
                      AND json_extract(s.snapshot_json, '$.created_at') = s.created_at
                      AND json_extract(s.snapshot_json, '$.output_root') = s.output_root
                      AND json_extract(s.snapshot_json, '$.layout') = s.layout
                      AND json_type(s.snapshot_json, '$.selected_products') = 'array'
                      AND json_array_length(s.snapshot_json, '$.selected_products') > 0
                      AND json_type(s.snapshot_json, '$.operation') = 'object'
                      AND (
                        (
                          json_extract(s.snapshot_json, '$.operation.kind') = 'dem'
                          AND json_extract(s.snapshot_json, '$.operation.phase') = 'execute'
                          AND json_extract(s.snapshot_json, '$.operation.approved_worker_operation')
                                = 'dem_prepare_sarscape'
                        )
                        OR (
                          json_extract(s.snapshot_json, '$.operation.kind') = 'dem'
                          AND json_extract(s.snapshot_json, '$.operation.phase') = 'download'
                          AND json_extract(s.snapshot_json, '$.operation.approved_worker_operation')
                                = 'dem_download'
                        )
                      )
                      AND json_extract(s.snapshot_json, '$.operation.region_id') = ?1
                      AND (
                        json_extract(s.snapshot_json, '$.operation.payload.region_id') = ?1
                        OR (
                          json_extract(s.snapshot_json, '$.operation.phase') = 'download'
                          AND json_extract(s.snapshot_json, '$.operation.approved_worker_operation') = 'dem_download'
                          AND json_type(s.snapshot_json, '$.operation.payload.region_id') IS NULL
                        )
                      )
                      AND json_extract(s.snapshot_json, '$.operation.payload.dataset') = ?2
                      AND json_extract(s.snapshot_json, '$.operation.payload.provider') = ?3
                      AND json_extract(s.snapshot_json, '$.operation.payload.resource_id') = ?4
                      AND json_extract(s.snapshot_json, '$.operation.payload.capability_id') = ?5
                      AND json_type(s.snapshot_json, '$.operation.payload.input_path') = 'text'
                      AND length(json_extract(s.snapshot_json, '$.operation.payload.input_path')) BETWEEN 1 AND 4096
                      AND trim(json_extract(s.snapshot_json, '$.operation.payload.input_path'))
                            = json_extract(s.snapshot_json, '$.operation.payload.input_path')
                      AND instr(json_extract(s.snapshot_json, '$.operation.payload.input_path'), char(0)) = 0
                      AND json_extract(s.snapshot_json, '$.operation.payload.input_path') NOT GLOB
                            ('*[' || char(1) || '-' || char(31) || char(127) || '-' || char(159) || ']*')
                      AND json_extract(s.snapshot_json, '$.operation.payload.path')
                            = json_extract(s.snapshot_json, '$.operation.payload.input_path')
                      AND json_extract(s.snapshot_json, '$.operation.payload.output_root') = s.output_root
                      AND json_extract(s.snapshot_json, '$.operation.payload.output_dir') = s.output_root
                      AND length(s.output_root) BETWEEN 1 AND 4096 AND trim(s.output_root) = s.output_root
                      AND instr(s.output_root, char(0)) = 0
                      AND s.output_root NOT GLOB
                            ('*[' || char(1) || '-' || char(31) || char(127) || '-' || char(159) || ']*')
                      AND json_valid(r.state_json) AND json_type(r.state_json, '$') = 'object'
                      AND json_extract(r.state_json, '$.task_id') = r.task_id
                      AND json_extract(r.state_json, '$.status') = r.status
                      AND json_type(r.state_json, '$.revision') = 'integer'
                      AND json_extract(r.state_json, '$.revision') = r.revision
                      AND json_extract(r.state_json, '$.updated_at') = r.updated_at
                      AND json_type(r.state_json, '$.selected_item_ids') = 'array'
                      AND json_array_length(r.state_json, '$.selected_item_ids') > 0
                      AND json_type(r.state_json, '$.checked_item_ids') = 'array'
                      AND json_type(r.state_json, '$.active_item_ids') = 'array'
                      AND json_type(r.state_json, '$.paused_item_ids') = 'array'
                      AND json_type(r.state_json, '$.queued_item_ids') = 'array'
                      AND json_type(r.state_json, '$.completed_item_ids') = 'array'
                      AND json_type(r.state_json, '$.failed_item_ids') = 'array'
                      AND json_type(r.state_json, '$.skipped_item_ids') = 'array'
                      AND json_type(r.state_json, '$.cancelled_item_ids') = 'array'
                      AND json_type(r.state_json, '$.page_index') = 'integer'
                      AND json_extract(r.state_json, '$.page_index') >= 0
                      AND json_type(r.state_json, '$.page_size') = 'integer'
                      AND json_extract(r.state_json, '$.page_size') >= 1
                      AND json_type(r.state_json, '$.concurrency') = 'integer'
                      AND json_extract(r.state_json, '$.concurrency') >= 1
                      AND json_array_length(r.state_json, '$.active_item_ids')
                            <= json_extract(r.state_json, '$.concurrency')
                      AND (
                        json_array_length(r.state_json, '$.active_item_ids')
                        + json_array_length(r.state_json, '$.paused_item_ids')
                        + json_array_length(r.state_json, '$.queued_item_ids')
                        + json_array_length(r.state_json, '$.completed_item_ids')
                        + json_array_length(r.state_json, '$.failed_item_ids')
                        + json_array_length(r.state_json, '$.skipped_item_ids')
                        + json_array_length(r.state_json, '$.cancelled_item_ids')
                      ) = json_array_length(r.state_json, '$.selected_item_ids')
                      AND NOT EXISTS (
                        SELECT 1 FROM json_each(r.state_json, '$.selected_item_ids') selected
                        WHERE selected.type <> 'text'
                           OR (SELECT COUNT(*) FROM json_each(r.state_json, '$.selected_item_ids') duplicate
                               WHERE duplicate.value = selected.value) <> 1
                           OR (
                             (SELECT COUNT(*) FROM json_each(r.state_json, '$.active_item_ids') state WHERE state.value = selected.value)
                             + (SELECT COUNT(*) FROM json_each(r.state_json, '$.paused_item_ids') state WHERE state.value = selected.value)
                             + (SELECT COUNT(*) FROM json_each(r.state_json, '$.queued_item_ids') state WHERE state.value = selected.value)
                             + (SELECT COUNT(*) FROM json_each(r.state_json, '$.completed_item_ids') state WHERE state.value = selected.value)
                             + (SELECT COUNT(*) FROM json_each(r.state_json, '$.failed_item_ids') state WHERE state.value = selected.value)
                             + (SELECT COUNT(*) FROM json_each(r.state_json, '$.skipped_item_ids') state WHERE state.value = selected.value)
                             + (SELECT COUNT(*) FROM json_each(r.state_json, '$.cancelled_item_ids') state WHERE state.value = selected.value)
                           ) <> 1
                      )
                      AND NOT EXISTS (
                        SELECT 1 FROM json_each(r.state_json, '$.checked_item_ids') checked
                        WHERE checked.type <> 'text'
                           OR NOT EXISTS (
                             SELECT 1 FROM json_each(r.state_json, '$.selected_item_ids') selected
                             WHERE selected.value = checked.value
                           )
                           OR (SELECT COUNT(*) FROM json_each(r.state_json, '$.checked_item_ids') duplicate
                               WHERE duplicate.value = checked.value) <> 1
                      )
                    UNION ALL
                    SELECT summary.task_id, summary.revision, summary.archived_at_epoch_ms,
                           authority.region_id, authority.dataset, authority.provider,
                           authority.resource_id, authority.capability_id,
                           authority.input_path, authority.output_root
                    FROM task_history_summary summary
                    JOIN task_history h ON h.task_id = summary.task_id
                    JOIN task_history_dem_source_authority authority
                      ON authority.task_id = summary.task_id
                    LEFT JOIN task_tombstones d ON d.task_id = summary.task_id
                    WHERE d.task_id IS NULL
                      AND summary.status = 'completed'
                      AND (
                        (
                          summary.operation_kind = 'dem'
                          AND summary.operation_phase = 'execute'
                          AND summary.approved_worker_operation = 'dem_prepare_sarscape'
                        )
                        OR (
                          summary.operation_kind = 'dem'
                          AND summary.operation_phase = 'download'
                          AND summary.approved_worker_operation = 'dem_download'
                        )
                      )
                      AND summary.operation_region_id = ?1
                      AND authority.region_id = ?1
                      AND authority.dataset = ?2
                      AND authority.provider = ?3
                      AND authority.resource_id = ?4
                      AND authority.capability_id = ?5
                      AND authority.output_root = summary.output_root
                      AND length(authority.input_path) BETWEEN 1 AND 4096
                      AND trim(authority.input_path) = authority.input_path
                      AND instr(authority.input_path, char(0)) = 0
                      AND authority.input_path NOT GLOB
                            ('*[' || char(1) || '-' || char(31) || char(127) || '-' || char(159) || ']*')
                      AND length(authority.output_root) BETWEEN 1 AND 4096
                      AND trim(authority.output_root) = authority.output_root
                      AND instr(authority.output_root, char(0)) = 0
                      AND authority.output_root NOT GLOB
                            ('*[' || char(1) || '-' || char(31) || char(127) || '-' || char(159) || ']*')
                )
                SELECT task_id, revision, region_id, dataset, provider, resource_id,
                       capability_id, input_path, output_root
                FROM candidates
                ORDER BY observed_epoch_ms DESC, task_id ASC LIMIT 1"#,
                params![
                    request.region_id,
                    request.dataset,
                    request.provider,
                    request.resource_id,
                    request.capability_id,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(
                task_id,
                revision,
                region_id,
                dataset,
                provider,
                resource_id,
                capability_id,
                input_path,
                output_root,
            )| {
                let revision = u64::try_from(revision)
                    .map_err(|_| StoreError::invalid("DEM source revision is invalid"))?;
                if [
                    &task_id,
                    &region_id,
                    &dataset,
                    &provider,
                    &resource_id,
                    &capability_id,
                ]
                .into_iter()
                .any(|value| !valid_history_task_id(value))
                    || !valid_bounded_scalar(&input_path, 4096)
                    || !valid_bounded_scalar(&output_root, 4096)
                {
                    return Err(StoreError::invalid("DEM source authority is invalid"));
                }
                Ok(StoreDemConversionSource {
                    task_id,
                    revision,
                    region_id,
                    dataset,
                    provider,
                    resource_id,
                    capability_id,
                    input_path: PathBuf::from(input_path),
                    output_root: PathBuf::from(output_root),
                })
            },
        )
        .transpose()
    }

    /// Read one bounded page from the archive authority. The history query uses
    /// a LIMIT+1 probe; telemetry is loaded only for the returned task IDs.
    pub fn load_task_history_page(
        &self,
        request: &StoreTaskHistoryPageRequest,
    ) -> Result<StoreTaskHistoryPage, StoreError> {
        if request.page_size == 0 || request.page_size > MAX_TASK_HISTORY_PAGE_SIZE {
            return Err(StoreError::invalid("task history page request is invalid"));
        }
        if request.after_archived_at_epoch_ms.is_some() != request.after_task_id.is_some()
            || request.after_archived_at_epoch_ms.is_some_and(|value| {
                !(MIN_HISTORY_CURSOR_EPOCH_MS..=MAX_HISTORY_CURSOR_EPOCH_MS).contains(&value)
            })
            || request
                .after_task_id
                .as_deref()
                .is_some_and(|value| !valid_history_task_id(value))
        {
            return Err(StoreError::invalid("task history cursor is invalid"));
        }
        let limit = request
            .page_size
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("task history page limit overflow"))?;
        let limit_sql = i64::try_from(limit)
            .map_err(|_| StoreError::invalid("task history page limit exceeds SQLite range"))?;
        let mut statement = self.connection.prepare(
            "SELECT h.archived_at, h.archived_at_epoch_ms, h.task_id, h.batch_id, h.provider_id,
                    h.created_at, h.output_root, h.layout, h.status, h.revision, h.updated_at,
                    h.selected_count, h.checked_count, h.active_count, h.paused_count, h.queued_count,
                    h.completed_count, h.failed_count, h.skipped_count, h.cancelled_count,
                    h.operation_id, h.operation_kind, h.operation_phase, h.operation_region_id,
                    h.approved_worker_operation, h.transfer_active_millis, h.dataset_id,
                    CASE
                        WHEN json_type(r.state_json, '$.concurrency') = 'integer'
                             AND json_extract(r.state_json, '$.concurrency') >= 1
                        THEN json_extract(r.state_json, '$.concurrency')
                        ELSE NULL
                    END AS concurrency,
                    CASE
                        WHEN lower(h.dataset_id) IN ('sar.nisar.l.rslc.provisional.v1', 'sar.nisar.s.ssar.rslc.v1') THEN
                            CASE
                                WHEN EXISTS(
                                    SELECT 1
                                    FROM json_each(s.snapshot_json, '$.selected_products') AS selected
                                    WHERE json_extract(selected.value, '$.metadata.delivery_kind') = 'remote_derived_subset_v2'
                                ) THEN 'nisar_subset'
                                ELSE 'nisar_complete'
                            END
                        ELSE NULL
                    END AS task_type,
                    CASE
                        WHEN h.approved_worker_operation = 'orbit_download'
                        THEN 'AUX_POEORB'
                        ELSE NULL
                    END AS auxiliary_kind,
                    json_extract(s.snapshot_json, '$.operation.payload.source_task_id') AS source_task_id
             FROM task_history_summary h
             LEFT JOIN task_snapshots s ON s.task_id = h.task_id
             LEFT JOIN task_runtime r ON r.task_id = h.task_id
             WHERE (?1 IS NULL OR h.archived_at_epoch_ms < ?1
                    OR (h.archived_at_epoch_ms = ?1 AND h.task_id > ?2))
             ORDER BY h.archived_at_epoch_ms DESC, h.task_id ASC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                request.after_archived_at_epoch_ms,
                request.after_task_id,
                limit_sql
            ],
            history_summary_from_sql,
        )?;
        let mut items = Vec::new();
        let mut keys = Vec::new();
        for row in rows {
            let (cursor, archived_at, summary) = row?;
            items.push(StoreTaskHistoryItem {
                archived_at,
                task: summary,
            });
            keys.push(cursor);
        }
        let has_more = items.len() > request.page_size as usize;
        items.truncate(request.page_size as usize);
        keys.truncate(request.page_size as usize);
        let task_ids: Vec<String> = items.iter().map(|item| item.task.task_id.clone()).collect();
        let telemetry_by_task = load_task_telemetries(&self.connection, &task_ids)?;
        for item in &mut items {
            if let Some(telemetry) = telemetry_by_task.get(&item.task.task_id) {
                item.task.terminal_bytes = telemetry.terminal_bytes;
                item.task.active_bytes = telemetry.active_bytes;
                item.task.other_bytes = telemetry.other_bytes;
                item.task.transferred_bytes = telemetry.transferred_bytes;
                item.task.total_bytes = telemetry.total_bytes;
            }
        }
        Ok(StoreTaskHistoryPage {
            page_size: request.page_size,
            has_more,
            next_cursor: has_more.then(|| keys.last().cloned()).flatten(),
            items,
        })
    }

    pub fn load_workbench_page(
        &self,
        request: &StoreWorkbenchPageRequest,
    ) -> Result<StoreWorkbenchPage, StoreError> {
        if request.batch_id.trim().is_empty()
            || request.page_size == 0
            || request.page_size > MAX_WORKBENCH_PAGE_SIZE
        {
            return Err(StoreError::invalid("workbench page request is invalid"));
        }
        let offset = request
            .page_index
            .checked_mul(request.page_size)
            .ok_or_else(|| StoreError::invalid("workbench page offset overflow"))?;
        let end = offset
            .checked_add(request.page_size)
            .ok_or_else(|| StoreError::invalid("workbench page end overflow"))?;
        if i64::try_from(end).is_err() {
            return Err(StoreError::invalid(
                "workbench page end exceeds SQLite range",
            ));
        }

        let transaction = self.connection.unchecked_transaction()?;
        let active: Option<(String, i64)> = transaction
            .query_row(
                "SELECT batch_id, revision FROM workbench_state WHERE slot = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((active_batch_id, active_revision_sql)) = active else {
            return Err(StoreError::NotFound);
        };
        if active_batch_id != request.batch_id {
            return Err(StoreError::NotFound);
        }
        let active_revision = u64::try_from(active_revision_sql)
            .map_err(|_| StoreError::invalid("workbench revision is invalid"))?;
        if active_revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }

        #[allow(clippy::type_complexity)]
        let metadata: (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
            String,
        ) = transaction.query_row(
            "SELECT b.batch_id, b.provider_id, b.created_at, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_extract(b.snapshot_json, '$.batch_id') END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_extract(b.snapshot_json, '$.provider_id') END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_extract(b.snapshot_json, '$.created_at') END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_array_length(b.snapshot_json, '$.candidates') END, \
                    w.state_json \
             FROM workbench_state w JOIN search_batches b ON b.batch_id = w.batch_id \
             WHERE w.slot = 1 AND w.batch_id = ?1 AND w.revision = ?2",
            params![request.batch_id, active_revision_sql],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )?;
        let (
            row_batch_id,
            row_provider_id,
            row_created_at,
            json_batch_id,
            json_provider_id,
            json_created_at,
            candidate_count_sql,
            state_json,
        ) = metadata;
        if json_batch_id.as_deref() != Some(row_batch_id.as_str())
            || json_provider_id.as_deref() != Some(row_provider_id.as_str())
            || json_created_at.as_deref() != Some(row_created_at.as_str())
        {
            return Err(StoreError::invalid("workbench batch envelope differs"));
        }
        let candidate_count_sql = candidate_count_sql
            .ok_or_else(|| StoreError::invalid("workbench candidate array is invalid"))?;
        let state: SearchWorkbenchState =
            serde_json::from_str(&state_json).map_err(StoreError::Corrupt)?;
        state.validate().map_err(StoreError::invalid)?;
        if state.batch_id != row_batch_id || state.revision != active_revision {
            return Err(StoreError::invalid("workbench state envelope differs"));
        }
        let checked_ids: HashSet<&str> =
            state.checked_item_ids.iter().map(String::as_str).collect();

        // #22 B slice 2: read the requested page from the schema-39 candidate
        // rows, falling back to the blob's `json_each` for batches frozen
        // before v39. The kind check keeps the same fail-closed rule as the
        // other bounded candidate readers: a partially written row set is
        // corruption, not a reason to silently serve the blob. Both paths
        // share the original frozen order and the same page boundary
        // semantics (the exact tail page is empty; beyond it is NotFound).
        let kind = crate::batch_candidates::candidate_rows_kind_connection(
            &transaction,
            &request.batch_id,
        )?
        .ok_or(StoreError::NotFound)?;
        let candidate_page = crate::batch_candidates::load_batch_candidate_page_connection(
            &transaction,
            &request.batch_id,
            request.page_index,
            request.page_size,
            kind,
        )?;
        if usize::try_from(candidate_page.total_count)
            .map_err(|_| StoreError::invalid("workbench candidate count is invalid"))?
            != usize::try_from(candidate_count_sql)
                .map_err(|_| StoreError::invalid("workbench candidate count is invalid"))?
        {
            return Err(StoreError::invalid(
                "workbench candidate rows disagree with the batch header",
            ));
        }
        let total_count = usize::try_from(candidate_page.total_count)
            .map_err(|_| StoreError::invalid("workbench candidate count is invalid"))?;
        let mut items = Vec::new();
        for (ordinal, item) in candidate_page
            .items
            .into_iter()
            .enumerate()
            .map(|(index, item)| (offset + index as u64, item))
        {
            if item.provider_id != row_provider_id {
                return Err(StoreError::invalid(
                    "workbench item provider differs from batch",
                ));
            }
            items.push(StoreWorkbenchPageItem {
                ordinal,
                checked: checked_ids.contains(item.item_id.as_str()),
                item,
            });
        }
        if items.len() > request.page_size as usize {
            return Err(StoreError::invalid("workbench page exceeds requested size"));
        }

        Ok(StoreWorkbenchPage {
            batch_id: row_batch_id,
            revision: active_revision,
            page_index: request.page_index,
            page_size: request.page_size,
            total_count,
            checked_count: state.checked_item_ids.len(),
            items,
        })
    }

    /// Load one bounded page of the selected items in a persisted task.
    ///
    /// The compact task snapshot is hydrated inside Store, but only the
    /// requested page and the corresponding progress/result fields cross the
    /// ApplicationService boundary. The renderer never receives the complete
    /// task candidate array. Runtime revision is observed and returned rather
    /// than used as a stale-read gate: a download may legitimately advance
    /// while the user opens this read-only detail view.
    pub fn load_task_page(
        &self,
        request: &StoreTaskPageRequest,
    ) -> Result<StoreTaskPage, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        let task = load_compact_task_connection(&self.connection, &request.task_id)?
            .ok_or(StoreError::NotFound)?;
        let source_total_bytes = frozen_selected_source_total_bytes(
            &task.snapshot.candidates,
            &task.runtime.selected_item_ids,
        );
        // This page needs the persisted network scalar, not the aggregate
        // progress totals (which may overflow independently of source-size facts).
        let network_bytes_received = self
            .connection
            .query_row(
                "SELECT network_bytes_received FROM task_transfer_telemetry WHERE task_id = ?1",
                [&request.task_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .map(|value| {
                u64::try_from(value)
                    .map_err(|_| StoreError::invalid("negative network bytes received"))
            })
            .transpose()?;
        let download = self.load_download_snapshot(&request.task_id)?;
        let last_known_totals = load_last_known_totals(&self.connection, &request.task_id)?;
        // Orbit scheduled-fetch S4: the persisted recheck rows of exactly this
        // task, one bounded query; page items carry the row as a projection
        // (UI-REG-003: the UI never computes or counts down next_check_at).
        let orbit_rechecks_by_id: HashMap<String, insar_core::OrbitRecheckInfo> = self
            .load_orbit_rechecks_for_task(&request.task_id)?
            .into_iter()
            .map(|row| (row.item_id.clone(), row.recheck_info()))
            .collect();
        let progress_by_id: HashMap<&str, &DownloadItemProgress> = download
            .progress
            .iter()
            .map(|value| (value.item_id.as_str(), value))
            .collect();
        let results_by_id: HashMap<&str, &DownloadItemResult> = download
            .results
            .iter()
            .map(|value| (value.item_id.as_str(), value))
            .collect();
        let candidates_by_id: HashMap<&str, &SnapshotItem> = task
            .snapshot
            .candidates
            .iter()
            .map(|value| (value.item_id.as_str(), value))
            .collect();
        let total_count = u64::try_from(task.runtime.selected_item_ids.len())
            .map_err(|_| StoreError::invalid("task item count exceeds platform range"))?;
        let offset = request
            .page_index
            .checked_mul(request.page_size)
            .ok_or_else(|| StoreError::invalid("task page offset overflow"))?;
        let start = usize::try_from(offset)
            .map_err(|_| StoreError::invalid("task page offset exceeds platform range"))?;
        let end = start.saturating_add(request.page_size as usize);
        let mut items = Vec::new();
        for (ordinal, item_id) in task
            .runtime
            .selected_item_ids
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            let item = candidates_by_id
                .get(item_id.as_str())
                .ok_or_else(|| StoreError::invalid("task page item is not in the frozen task"))?;
            let progress = progress_by_id.get(item_id.as_str()).copied();
            let result = results_by_id.get(item_id.as_str()).copied();
            let status = task_page_item_status(&task.runtime, item_id, progress, result);
            let failure = if status == DownloadItemStatus::Failed {
                latest_failure_event_failure(&self.connection, &request.task_id, item_id)?
                    .filter(|failure| {
                        is_nisar_subset_v2_item(item)
                            || failure.code == SCHEDULER_GENERATION_UNCERTAIN
                    })
                    .map(|failure| {
                        if failure.code == SCHEDULER_GENERATION_UNCERTAIN {
                            scheduler_generation_uncertain_failure()
                        } else {
                            failure
                        }
                    })
                    .or_else(|| public_download_failure(result))
            } else {
                public_download_failure(result)
            };
            let page_item = StoreTaskPageItem {
                ordinal: u64::try_from(ordinal)
                    .map_err(|_| StoreError::invalid("task item ordinal exceeds platform range"))?,
                item: (*item).clone(),
                checked: task
                    .runtime
                    .checked_item_ids
                    .iter()
                    .any(|checked| checked == item_id),
                status: status.clone(),
                bytes_completed: if matches!(
                    status,
                    DownloadItemStatus::Completed
                        | DownloadItemStatus::Failed
                        | DownloadItemStatus::Skipped
                        | DownloadItemStatus::Cancelled
                ) {
                    result
                        .map(|value| value.bytes_written)
                        .or_else(|| progress.map(|value| value.bytes_completed))
                        .unwrap_or(0)
                } else {
                    progress.map(|value| value.bytes_completed).unwrap_or(0)
                },
                total_bytes: progress
                    .and_then(|value| value.total_bytes)
                    .or_else(|| last_known_totals.get(item_id).copied().flatten())
                    .or_else(|| {
                        result.and_then(|value| {
                            matches!(
                                value.outcome,
                                DownloadOutcome::Completed | DownloadOutcome::Skipped
                            )
                            .then_some(value.bytes_written)
                        })
                    })
                    .or_else(|| frozen_item_total_bytes(item)),
                source_total_bytes: frozen_item_total_bytes(item),
                local_reuse: self
                    .task_item_local_reuse(&request.task_id, item_id)?
                    .as_ref()
                    .map(crate::local_reuse::local_reuse_projection),
                bytes_per_second: if status != DownloadItemStatus::Active {
                    0.0
                } else {
                    progress.map(|value| value.bytes_per_second).unwrap_or(0.0)
                },
                attempt: result
                    .map(|value| value.attempt_count)
                    .or_else(|| progress.map(|value| value.attempt))
                    .unwrap_or(0),
                destination: result.map(|value| value.destination.clone()),
                failure,
                finished_at: result.map(|value| value.finished_at.clone()),
                orbit_recheck: orbit_rechecks_by_id.get(item_id.as_str()).cloned(),
                retryable: matches!(
                    status,
                    DownloadItemStatus::Failed
                        | DownloadItemStatus::Cancelled
                        | DownloadItemStatus::Interrupted
                ) || result
                    .and_then(|value| value.failure.as_ref())
                    .is_some_and(|value| value.retryable),
            };
            page_item.validate().map_err(StoreError::invalid)?;
            items.push(page_item);
        }

        Ok(StoreTaskPage {
            task_id: task.runtime.task_id,
            revision: task.runtime.revision,
            page_index: request.page_index,
            page_size: request.page_size,
            total_count,
            source_total_bytes,
            network_bytes_received,
            items,
        })
    }

    /// Load one bounded page from the immutable SearchBatch that created a task.
    ///
    /// This read never consults or activates the global Workbench. Task revision
    /// is observed and returned from the same read transaction rather than used
    /// as a caller-supplied gate, so live progress cannot stale-fail browsing.
    pub fn load_task_source_batch_page(
        &self,
        request: &StoreTaskSourceBatchPageRequest,
    ) -> Result<StoreTaskSourceBatchPage, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        let transaction = self.connection.unchecked_transaction()?;
        let task = load_compact_task_connection(&transaction, &request.task_id)?
            .ok_or(StoreError::NotFound)?;
        let landsat_file_batch = task
            .snapshot
            .query
            .get("delivery_kind")
            .and_then(Value::as_str)
            == Some("landsat_c2_l2_asset_batch");
        let matched_landsat_archive = task
            .snapshot
            .query
            .get("delivery_kind")
            .and_then(Value::as_str)
            == Some("landsat_c2_l2_matched_full_bundle");
        let landsat_archive_batch = task
            .snapshot
            .query
            .get("delivery_kind")
            .and_then(Value::as_str)
            == Some("landsat_c2_l2_bundle_batch");
        let sentinel2_band_batch = task
            .snapshot
            .query
            .get("delivery_kind")
            .and_then(Value::as_str)
            == Some("sentinel2_l2a_analysis_band_batch");
        let sentinel2_l1c_band_batch = task
            .snapshot
            .query
            .get("delivery_kind")
            .and_then(Value::as_str)
            == Some("sentinel2_l1c_band_batch");
        let hls_band_batch = task
            .snapshot
            .query
            .get("delivery_kind")
            .and_then(Value::as_str)
            == Some("hls_multiband_batch");
        let progressive_source = match crate::loading_admission::verified_progressive_source(
            &transaction,
            &task.snapshot,
        ) {
            Ok(source) => source,
            // Before seal the existing task-local page remains available. Writes
            // against the original source always wait for the seal.
            Err(StoreError::BatchStillLoading { .. }) => None,
            Err(error) => return Err(error),
        };
        let source_batch_id = if let Some(source) = progressive_source.as_deref() {
            source
        } else if landsat_file_batch
            || matched_landsat_archive
            || landsat_archive_batch
            || sentinel2_band_batch
            || sentinel2_l1c_band_batch
            || hls_band_batch
        {
            task.snapshot
                .query
                .get("derived_from_batch_id")
                .and_then(Value::as_str)
                .filter(|id| *id != task.snapshot.batch_id)
                .ok_or_else(|| StoreError::invalid("file batch lacks its immutable scene source"))?
        } else {
            &task.snapshot.batch_id
        };
        let (source_batch, progressive_total) = if progressive_source.is_some() {
            let kind = crate::batch_candidates::candidate_rows_kind_connection(
                &transaction,
                source_batch_id,
            )?
            .ok_or(StoreError::NotFound)?;
            let page = crate::batch_candidates::load_batch_candidate_page_connection(
                &transaction,
                source_batch_id,
                request.page_index,
                request.page_size,
                kind,
            )?;
            // The verified derived header carries the same provider and AOI.
            // Only the requested original page crosses the decode boundary.
            let mut batch =
                load_batch(&transaction, &task.snapshot.batch_id)?.ok_or(StoreError::NotFound)?;
            batch.batch_id = source_batch_id.to_owned();
            batch.candidates = page.items;
            (batch, Some(page.total_count))
        } else {
            (
                load_batch(&transaction, source_batch_id)?
                    .ok_or_else(|| StoreError::invalid("task source batch does not exist"))?,
                None,
            )
        };
        if matched_landsat_archive
            && (task.snapshot.provider_id != "usgs.eros.m2m"
                || source_batch.provider_id != "microsoft.planetary-computer")
        {
            return Err(StoreError::invalid(
                "official Landsat archive source binding is invalid",
            ));
        }
        if landsat_file_batch && source_batch.provider_id != task.snapshot.provider_id {
            return Err(StoreError::invalid(
                "file batch source provider does not match",
            ));
        }
        if landsat_archive_batch
            && (task.snapshot.provider_id != "usgs.eros.m2m"
                || !matches!(
                    source_batch.provider_id.as_str(),
                    "usgs.eros.m2m" | "microsoft.planetary-computer"
                )
                || source_batch
                    .query
                    .get("logical_dataset")
                    .and_then(Value::as_str)
                    != Some("optical.landsat.c2.l2"))
        {
            return Err(StoreError::invalid(
                "official Landsat batch source binding is invalid",
            ));
        }
        if sentinel2_band_batch
            && (!matches!(
                task.snapshot.provider_id.as_str(),
                "microsoft.planetary-computer" | "element84.earth-search"
            ) || !matches!(
                source_batch.provider_id.as_str(),
                "microsoft.planetary-computer"
                    | "copernicus.dataspace.odata"
                    | "element84.earth-search"
            ) || source_batch
                .query
                .get("logical_dataset")
                .and_then(Value::as_str)
                != Some("optical.sentinel-2.l2a"))
        {
            return Err(StoreError::invalid(
                "Sentinel-2 band batch source binding is invalid",
            ));
        }
        if sentinel2_l1c_band_batch
            && (task.snapshot.provider_id != "copernicus.dataspace.odata"
                || source_batch.provider_id != "copernicus.dataspace.odata"
                || source_batch
                    .query
                    .get("logical_dataset")
                    .and_then(Value::as_str)
                    != Some("optical.sentinel-2.l1c"))
        {
            return Err(StoreError::invalid(
                "Sentinel-2 L1C band batch source binding is invalid",
            ));
        }
        if hls_band_batch
            && (task.snapshot.provider_id != "microsoft.planetary-computer"
                || source_batch.provider_id != "microsoft.planetary-computer"
                || !matches!(
                    source_batch
                        .query
                        .get("logical_dataset")
                        .and_then(Value::as_str),
                    Some("optical.hls.l30.v2" | "optical.hls.s30.v2")
                ))
        {
            return Err(StoreError::invalid(
                "HLS band batch source binding is invalid",
            ));
        }
        let explicit_source_item_ids =
            load_explicit_task_source_item_ids(&transaction, &request.task_id)?;
        let download =
            crate::download::load_snapshot_connection(&transaction, &request.task_id, 0)?;
        let last_known_totals = load_last_known_totals(&transaction, &request.task_id)?;
        let progress_by_id: HashMap<&str, &DownloadItemProgress> = download
            .progress
            .iter()
            .map(|value| (value.item_id.as_str(), value))
            .collect();
        let results_by_id: HashMap<&str, &DownloadItemResult> = download
            .results
            .iter()
            .map(|value| (value.item_id.as_str(), value))
            .collect();
        let task_items_by_source: HashMap<&str, Vec<&str>> = if explicit_source_item_ids.is_some()
            || landsat_file_batch
            || matched_landsat_archive
            || landsat_archive_batch
            || sentinel2_band_batch
            || sentinel2_l1c_band_batch
            || hls_band_batch
        {
            let mut by_source: HashMap<&str, Vec<&str>> = HashMap::new();
            for item in &task.snapshot.candidates {
                let source_item_id = item
                    .metadata
                    .get("source_item_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StoreError::invalid("explicit task item lacks its source item ID")
                    })?;
                if (landsat_file_batch
                    || sentinel2_band_batch
                    || sentinel2_l1c_band_batch
                    || hls_band_batch)
                    && !source_batch
                        .candidates
                        .iter()
                        .any(|candidate| candidate.item_id == source_item_id)
                {
                    return Err(StoreError::invalid(
                        "file task item refers outside its immutable scene source",
                    ));
                }
                by_source
                    .entry(source_item_id)
                    .or_default()
                    .push(item.item_id.as_str());
            }
            by_source
        } else {
            task.runtime
                .selected_item_ids
                .iter()
                .map(|item_id| (item_id.as_str(), vec![item_id.as_str()]))
                .collect()
        };
        if progressive_source.is_some() {
            let source_ids = task_items_by_source
                .keys()
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>();
            for chunk in source_ids.chunks(crate::MAX_WORKBENCH_PAGE_SIZE as usize) {
                let items = crate::batch_candidates::load_batch_candidate_items_connection(
                    &transaction,
                    source_batch_id,
                    chunk,
                )?
                .ok_or(StoreError::NotFound)?;
                if items.len() != chunk.len() {
                    return Err(StoreError::invalid(
                        "task item refers outside its immutable source batch",
                    ));
                }
            }
        } else {
            let source_batch_item_ids = source_batch
                .candidates
                .iter()
                .map(|item| item.item_id.as_str())
                .collect::<HashSet<_>>();
            if task_items_by_source
                .keys()
                .any(|id| !source_batch_item_ids.contains(id))
            {
                return Err(StoreError::invalid(
                    "task item refers outside its immutable source batch",
                ));
            }
        }
        let checked_task_items: HashSet<&str> = task
            .runtime
            .checked_item_ids
            .iter()
            .map(String::as_str)
            .collect();
        let total_count = match progressive_total {
            Some(total) => total,
            None => u64::try_from(source_batch.candidates.len())
                .map_err(|_| StoreError::invalid("source batch count exceeds platform range"))?,
        };
        let offset = request
            .page_index
            .checked_mul(request.page_size)
            .ok_or_else(|| StoreError::invalid("source batch page offset overflow"))?;
        let start = usize::try_from(offset)
            .map_err(|_| StoreError::invalid("source batch page offset exceeds platform range"))?;
        let mut items = Vec::new();
        for (ordinal, item) in source_batch
            .candidates
            .iter()
            .enumerate()
            .skip(if progressive_total.is_some() {
                0
            } else {
                start
            })
            .take(request.page_size as usize)
        {
            let ordinal = if progressive_total.is_some() {
                start + ordinal
            } else {
                ordinal
            };
            let belongs_to_task = task_items_by_source.contains_key(item.item_id.as_str());
            let task_item_ids = task_items_by_source
                .get(item.item_id.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut statuses = Vec::new();
            let mut bytes_completed = 0_u64;
            let mut total_bytes = Some(0_u64);
            let mut bytes_per_second = 0.0_f64;
            let mut failure = None;
            for task_item_id in task_item_ids {
                let progress = progress_by_id.get(task_item_id).copied();
                let result = results_by_id.get(task_item_id).copied();
                let status = task_page_item_status(&task.runtime, task_item_id, progress, result);
                let terminal = matches!(
                    status,
                    DownloadItemStatus::Completed
                        | DownloadItemStatus::Failed
                        | DownloadItemStatus::Skipped
                        | DownloadItemStatus::Cancelled
                );
                let item_bytes_completed = if terminal {
                    result
                        .map(|value| value.bytes_written)
                        .or_else(|| progress.map(|value| value.bytes_completed))
                        .unwrap_or(0)
                } else {
                    progress.map(|value| value.bytes_completed).unwrap_or(0)
                };
                bytes_completed = bytes_completed
                    .checked_add(item_bytes_completed)
                    .ok_or_else(|| StoreError::invalid("source batch aggregate bytes overflow"))?;
                let item_total = progress
                    .and_then(|value| value.total_bytes)
                    .or_else(|| last_known_totals.get(*task_item_id).copied().flatten())
                    .or_else(|| result.map(|value| value.bytes_written));
                total_bytes = match (total_bytes, item_total) {
                    (Some(total), Some(value)) => {
                        Some(total.checked_add(value).ok_or_else(|| {
                            StoreError::invalid("source batch aggregate total bytes overflow")
                        })?)
                    }
                    _ => None,
                };
                if !terminal {
                    bytes_per_second += progress.map(|value| value.bytes_per_second).unwrap_or(0.0);
                }
                if failure.is_none() {
                    failure = public_download_failure(result);
                }
                statuses.push(status);
            }
            let task_status = belongs_to_task.then(|| aggregate_task_item_status(&statuses));
            let page_item = StoreTaskSourceBatchPageItem {
                ordinal: u64::try_from(ordinal).map_err(|_| {
                    StoreError::invalid("source batch item ordinal exceeds platform range")
                })?,
                item: item.clone(),
                belongs_to_task,
                task_status,
                checked: task_item_ids
                    .iter()
                    .any(|item_id| checked_task_items.contains(item_id)),
                bytes_completed,
                total_bytes,
                bytes_per_second,
                failure,
            };
            page_item.validate().map_err(StoreError::invalid)?;
            items.push(page_item);
        }
        let response = StoreTaskSourceBatchPage {
            task_id: task.runtime.task_id,
            source_batch_id: source_batch.batch_id,
            task_revision: task.runtime.revision,
            task_status: task.runtime.status,
            page_index: request.page_index,
            page_size: request.page_size,
            total_count,
            items,
        };
        response.validate().map_err(StoreError::invalid)?;
        transaction.commit()?;
        Ok(response)
    }

    pub fn apply_workbench_selection(
        &mut self,
        command: &WorkbenchSelectionCommand,
    ) -> Result<WorkbenchSelectionSummary, StoreError> {
        command.validate().map_err(StoreError::invalid)?;
        if command.updated_at.trim().is_empty() {
            return Err(StoreError::invalid(
                "workbench selection timestamp is required",
            ));
        }
        let delta = match command.kind {
            WorkbenchSelectionCommandKind::CheckItems
            | WorkbenchSelectionCommandKind::UncheckItems => {
                if command.item_ids.is_empty() {
                    return Err(StoreError::invalid("item deltas require exact item IDs"));
                }
                if command.item_ids.len() > MAX_WORKBENCH_PAGE_SIZE as usize {
                    return Err(StoreError::invalid("item delta exceeds the 250-item cap"));
                }
                let unique: HashSet<&str> = command.item_ids.iter().map(String::as_str).collect();
                if unique.len() != command.item_ids.len() {
                    return Err(StoreError::invalid("item delta IDs must be unique"));
                }
                command.item_ids.clone()
            }
            WorkbenchSelectionCommandKind::SelectAll
            | WorkbenchSelectionCommandKind::ClearAll
            | WorkbenchSelectionCommandKind::SetPage => {
                if !command.item_ids.is_empty() {
                    return Err(StoreError::invalid(
                        "batch and page commands carry no item IDs",
                    ));
                }
                Vec::new()
            }
        };

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (state, revision, page_index, page_size) = load_active_workbench_state(
            &transaction,
            &command.batch_id,
            command.expected_revision,
        )?;
        let candidate_ids = match command.kind {
            WorkbenchSelectionCommandKind::SelectAll => {
                Some(load_candidate_ids(&transaction, &command.batch_id)?)
            }
            _ => None,
        };

        let mut checked: Vec<String> = state.checked_item_ids;
        match command.kind {
            WorkbenchSelectionCommandKind::CheckItems => {
                validate_candidate_delta(&transaction, &command.batch_id, &delta)?;
                for item_id in &delta {
                    if !checked.iter().any(|existing| existing == item_id) {
                        checked.push(item_id.clone());
                    }
                }
            }
            WorkbenchSelectionCommandKind::UncheckItems => {
                validate_candidate_delta(&transaction, &command.batch_id, &delta)?;
                let remove: HashSet<&str> = delta.iter().map(String::as_str).collect();
                checked.retain(|item_id| !remove.contains(item_id.as_str()));
            }
            WorkbenchSelectionCommandKind::SelectAll => {
                checked = candidate_ids.expect("SelectAll loads candidate IDs");
            }
            WorkbenchSelectionCommandKind::ClearAll => {
                checked.clear();
            }
            WorkbenchSelectionCommandKind::SetPage => {}
        }

        let new_revision = revision
            .checked_add(1)
            .ok_or_else(|| StoreError::invalid("workbench revision overflow"))?;
        let (next_page_index, next_page_size) = match command.kind {
            WorkbenchSelectionCommandKind::SetPage => (command.page_index, command.page_size),
            _ => (page_index, page_size),
        };
        let next_state = SearchWorkbenchState {
            batch_id: command.batch_id.clone(),
            checked_item_ids: checked,
            page_index: next_page_index,
            page_size: next_page_size,
            revision: new_revision,
        };
        next_state.validate().map_err(StoreError::invalid)?;
        let state_json = serde_json::to_string(&next_state).map_err(StoreError::Corrupt)?;
        let new_revision_sql = sqlite_integer(new_revision, "workbench revision")?;
        let changed = transaction.execute(
            "UPDATE workbench_state SET revision = ?1, updated_at = ?2, state_json = ?3 \
             WHERE slot = 1 AND batch_id = ?4 AND revision = ?5",
            params![
                new_revision_sql,
                command.updated_at,
                state_json,
                command.batch_id,
                sqlite_integer(command.expected_revision, "expected workbench revision")?,
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::RevisionConflict);
        }
        transaction.commit()?;
        Ok(WorkbenchSelectionSummary {
            batch_id: command.batch_id.clone(),
            revision: new_revision,
            checked_count: next_state.checked_item_ids.len() as u64,
            page_index: next_page_index,
            page_size: next_page_size,
        })
    }

    pub fn create_task_from_workbench(
        &mut self,
        request: &CreateTaskFromWorkbenchRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.output_root.as_os_str().is_empty() {
            return Err(StoreError::invalid("task output is required"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (state, _revision, page_index, page_size) = load_active_workbench_state(
            &transaction,
            &request.batch_id,
            request.expected_workbench_revision,
        )?;
        let selected_item_ids = request
            .exact_item_id
            .as_ref()
            .map(|item_id| vec![item_id.clone()])
            .unwrap_or(state.checked_item_ids);
        if selected_item_ids.is_empty() {
            return Err(StoreError::invalid(
                "task creation requires a non-empty workbench selection",
            ));
        }
        let task_request = StoreCreateTaskRequest {
            task_id: request.task_id.clone(),
            batch_id: request.batch_id.clone(),
            selected_item_ids: selected_item_ids.clone(),
            checked_item_ids: selected_item_ids,
            page_index,
            page_size,
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };
        let stored = create_task_in_transaction(&transaction, &task_request, None, None)?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn append_task_from_workbench(
        &mut self,
        request: &AppendTaskFromWorkbenchRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("append timestamp is required"));
        }
        let request_json = serde_json::to_string(request).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT task_id, payload_json, event_type FROM task_events WHERE event_id = ?1",
                [&request.event_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((task_id, payload_json, event_type)) = existing {
            if task_id != request.task_id
                || payload_json != request_json
                || event_type != "append_items"
            {
                return Err(StoreError::AlreadyExists);
            }
            reject_tombstoned_task(&transaction, &request.task_id)?;
            let task = load_task_connection(&transaction, &request.task_id)?
                .ok_or(StoreError::NotFound)?;
            transaction.commit()?;
            return Ok(task);
        }
        let (state, ..) = load_active_workbench_state(
            &transaction,
            &request.batch_id,
            request.expected_workbench_revision,
        )?;
        reject_closed_task(&transaction, &request.task_id)?;
        if !task_is_actionable(&transaction, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        if load_explicit_task_source_item_ids(&transaction, &request.task_id)?.is_some() {
            return Err(StoreError::invalid(
                "explicit task membership must be expanded into frozen task items",
            ));
        }
        let task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        let progressive_source =
            crate::loading_admission::verified_progressive_source(&transaction, &task.snapshot)?;
        if progressive_source
            .as_deref()
            .unwrap_or(&task.snapshot.batch_id)
            != request.batch_id
        {
            return Err(StoreError::invalid(
                "task batch differs from the active workbench batch",
            ));
        }
        let selected: HashSet<&str> = task
            .runtime
            .selected_item_ids
            .iter()
            .map(String::as_str)
            .collect();
        let to_append: Vec<String> = state
            .checked_item_ids
            .into_iter()
            .filter(|item_id| !selected.contains(item_id.as_str()))
            .collect();
        if to_append.is_empty() {
            return Err(StoreError::invalid(
                "workbench selection adds no new items to this task",
            ));
        }
        if let Some(source_id) = progressive_source {
            let expected_revision = task.runtime.revision;
            let task = append_progressive_source_items(
                &transaction,
                task,
                &source_id,
                expected_revision,
                &to_append,
                &request.event_id,
                &request.occurred_at,
                &request_json,
            )?;
            transaction.commit()?;
            return Ok(task);
        }
        // The unified search freezes a burst as its parent product, while the
        // verified burst worker downloads per-asset items; appends through
        // this generic seam cannot carry new frozen burst assets. The typed
        // burst asset task seam owns burst appends.
        let burst_parent = |item_id: &str| {
            task.snapshot.candidates.iter().any(|item| {
                item.item_id == item_id
                    && item.provider_id == "asf.catalog"
                    && item
                        .metadata
                        .get("dataset")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SLC-BURST"))
            })
        };
        if to_append.iter().any(|item_id| burst_parent(item_id)) {
            return Err(StoreError::invalid(
                "burst products are appended through the burst asset task seam",
            ));
        }
        let expected_revision = task.runtime.revision;
        let task = append_task_membership_in_transaction(
            &transaction,
            task,
            expected_revision,
            &to_append,
            &request.event_id,
            &request.occurred_at,
            &request_json,
        )?;
        transaction.commit()?;
        Ok(task)
    }

    pub fn append_task_from_source_batch(
        &mut self,
        request: &AppendTaskFromSourceBatchRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.occurred_at.trim().is_empty() {
            return Err(StoreError::invalid("append timestamp is required"));
        }
        let request_json = serde_json::to_string(request).map_err(StoreError::Corrupt)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        match crate::source_receipts::source_append_status(&transaction, request)? {
            crate::SourceAppendReceiptStatus::Committed
            | crate::SourceAppendReceiptStatus::LegacyCommitted => {
                reject_tombstoned_task(&transaction, &request.task_id)?;
                let task = load_task_connection(&transaction, &request.task_id)?
                    .ok_or(StoreError::NotFound)?;
                transaction.commit()?;
                return Ok(task);
            }
            crate::SourceAppendReceiptStatus::Conflict => return Err(StoreError::AlreadyExists),
            crate::SourceAppendReceiptStatus::Unknown => {}
        }
        let existing = transaction
            .query_row(
                "SELECT task_id, payload_json, event_type FROM task_events WHERE event_id = ?1",
                [&request.event_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((task_id, payload_json, event_type)) = existing {
            if task_id != request.task_id
                || payload_json != request_json
                || event_type != "append_items"
            {
                return Err(StoreError::AlreadyExists);
            }
            reject_tombstoned_task(&transaction, &request.task_id)?;
            let task = load_task_connection(&transaction, &request.task_id)?
                .ok_or(StoreError::NotFound)?;
            transaction.commit()?;
            return Ok(task);
        }
        reject_closed_task(&transaction, &request.task_id)?;
        if !task_is_actionable(&transaction, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        if load_explicit_task_source_item_ids(&transaction, &request.task_id)?.is_some() {
            return Err(StoreError::invalid(
                "explicit task membership must be expanded into frozen task items",
            ));
        }
        let task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        if let Some(source_id) =
            crate::loading_admission::verified_progressive_source(&transaction, &task.snapshot)?
        {
            let task = append_progressive_source_items(
                &transaction,
                task,
                &source_id,
                request.expected_task_revision,
                &request.item_ids,
                &request.event_id,
                &request.occurred_at,
                &request_json,
            )?;
            crate::source_receipts::insert_source_append_receipt(
                &transaction,
                request,
                task.runtime.revision,
            )?;
            transaction.commit()?;
            return Ok(task);
        }
        // Burst items are appended as frozen per-asset items by the typed burst
        // asset task seam; the generic source-batch append cannot freeze new
        // burst assets, so a burst parent in the request fails closed instead
        // of freezing a parent the worker cannot download (DL004).
        let burst_parent = |item_id: &str| {
            task.snapshot.candidates.iter().any(|item| {
                item.item_id == item_id
                    && item.provider_id == "asf.catalog"
                    && item
                        .metadata
                        .get("dataset")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SLC-BURST"))
            })
        };
        if request.item_ids.iter().any(|item_id| burst_parent(item_id)) {
            return Err(StoreError::invalid(
                "burst products are appended through the burst asset task seam",
            ));
        }
        let task = append_task_membership_in_transaction(
            &transaction,
            task,
            request.expected_task_revision,
            &request.item_ids,
            &request.event_id,
            &request.occurred_at,
            &request_json,
        )?;
        crate::source_receipts::insert_source_append_receipt(
            &transaction,
            request,
            task.runtime.revision,
        )?;
        transaction.commit()?;
        Ok(task)
    }

    pub fn create_task_from_historical_source_batch(
        &mut self,
        request: &CreateTaskFromHistoricalSourceBatchRequest,
    ) -> Result<StoredTask, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.output_root.as_os_str().is_empty() {
            return Err(StoreError::invalid("task output is required"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        match crate::source_receipts::historical_successor_status(&transaction, request)? {
            crate::HistoricalSuccessorReceiptStatus::Committed => {
                reject_tombstoned_task(&transaction, &request.new_task_id)?;
                let stored = load_task_connection(&transaction, &request.new_task_id)?
                    .ok_or(StoreError::NotFound)?;
                transaction.commit()?;
                return Ok(stored);
            }
            crate::HistoricalSuccessorReceiptStatus::Conflict => {
                return Err(StoreError::AlreadyExists)
            }
            crate::HistoricalSuccessorReceiptStatus::Unknown => {}
        }
        let existed: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_snapshots WHERE task_id = ?1)",
            [&request.new_task_id],
            |row| row.get(0),
        )?;
        if existed {
            return Err(StoreError::AlreadyExists);
        }
        reject_tombstoned_task(&transaction, &request.new_task_id)?;
        let tombstoned: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            [&request.source_task_id],
            |row| row.get(0),
        )?;
        if tombstoned || !task_is_actionable(&transaction, &request.source_task_id)? {
            return Err(StoreError::NotFound);
        }
        let source = load_task_connection(&transaction, &request.source_task_id)?
            .ok_or(StoreError::NotFound)?;
        if load_explicit_task_source_item_ids(&transaction, &request.source_task_id)?.is_some() {
            return Err(StoreError::invalid(
                "explicit task successors must be expanded into frozen task items",
            ));
        }
        if source.runtime.revision != request.expected_source_task_revision {
            return Err(StoreError::RevisionConflict);
        }
        let source_batch_id = match crate::loading_admission::verified_progressive_source(
            &transaction,
            &source.snapshot,
        )? {
            Some(original_id) => {
                if request.item_ids.len() > crate::MAX_WORKBENCH_PAGE_SIZE as usize
                    || request.item_ids.iter().collect::<HashSet<_>>().len()
                        != request.item_ids.len()
                {
                    return Err(StoreError::invalid(
                        "historical selection must contain unique bounded IDs",
                    ));
                }
                let mut candidates =
                    crate::batch_candidates::load_batch_candidate_items_connection(
                        &transaction,
                        &original_id,
                        &request.item_ids,
                    )?
                    .ok_or(StoreError::NotFound)?;
                if candidates.len() != request.item_ids.len() {
                    return Err(StoreError::invalid(
                        "historical selection differs from its source batch",
                    ));
                }
                let requested =
                    serde_json::to_string(&request.item_ids).map_err(StoreError::Corrupt)?;
                let conflict: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM batch_task_items i JOIN task_runtime r ON r.task_id = i.task_id \
                     LEFT JOIN task_tombstones d ON d.task_id = i.task_id WHERE i.source_batch_id = ?1 \
                     AND i.source_item_id IN (SELECT value FROM json_each(?2)) AND i.task_id != ?3 \
                     AND d.task_id IS NULL AND r.status IN ('queued','running','paused','interrupted'))",
                    params![original_id, requested, request.new_task_id], |row| row.get(0),
                )?;
                if conflict {
                    return Err(StoreError::PartialActiveTaskOverlap);
                }
                for item in &mut candidates {
                    if item.metadata.contains_key("source_item_id") {
                        return Err(StoreError::invalid(
                            "historical source uses the reserved backlink",
                        ));
                    }
                    if item.provider_id == "asf.catalog"
                        && item
                            .metadata
                            .get("dataset")
                            .and_then(Value::as_str)
                            .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SLC-BURST"))
                    {
                        return Err(StoreError::invalid(
                            "burst successors require the explicit asset path",
                        ));
                    }
                    item.metadata.insert(
                        "source_item_id".to_owned(),
                        Value::String(item.item_id.clone()),
                    );
                }
                // A compact derived batch preserves the existing snapshot contract
                // without hydrating the entire original search batch.
                let identity = serde_json::to_vec(request).map_err(StoreError::Corrupt)?;
                let digest = Sha256::digest(identity);
                let batch_id = format!("progressive-successor-{digest:x}");
                let mut batch = load_batch(&transaction, &source.snapshot.batch_id)?
                    .ok_or(StoreError::NotFound)?;
                batch.batch_id = batch_id.clone();
                batch.created_at = request.created_at.clone();
                batch.candidates = candidates;
                crate::repository::save_search_batch_in_transaction(
                    &transaction,
                    &insar_core::StoreSearchBatchRequest {
                        batch,
                        workbench: SearchWorkbenchState {
                            batch_id: batch_id.clone(),
                            checked_item_ids: Vec::new(),
                            page_index: 0,
                            page_size: source.runtime.page_size,
                            revision: 0,
                        },
                        activate_workbench: false,
                    },
                )?;
                batch_id
            }
            None => {
                let requested =
                    serde_json::to_string(&request.item_ids).map_err(StoreError::Corrupt)?;
                let conflict: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM batch_task_items i JOIN task_runtime r ON r.task_id = i.task_id \
                     LEFT JOIN task_tombstones d ON d.task_id = i.task_id WHERE i.source_batch_id = ?1 \
                     AND i.source_item_id IN (SELECT value FROM json_each(?2)) AND i.task_id != ?3 \
                     AND d.task_id IS NULL AND r.status IN ('queued','running','paused','interrupted'))",
                    params![source.snapshot.batch_id, requested, request.new_task_id], |row| row.get(0),
                )?;
                if conflict {
                    return Err(StoreError::PartialActiveTaskOverlap);
                }
                source.snapshot.batch_id.clone()
            }
        };
        let task_request = StoreCreateTaskRequest {
            task_id: request.new_task_id.clone(),
            batch_id: source_batch_id,
            selected_item_ids: request.item_ids.clone(),
            checked_item_ids: request.item_ids.clone(),
            page_index: 0,
            page_size: source.runtime.page_size,
            output_root: request.output_root.clone(),
            layout: request.layout.clone(),
            concurrency: request.concurrency,
            created_at: request.created_at.clone(),
        };
        if request
            .reacquire_item_ids
            .iter()
            .any(|item_id| !request.item_ids.contains(item_id))
        {
            return Err(StoreError::invalid(
                "items to download again must be items of the successor task",
            ));
        }
        let stored = create_task_in_transaction(&transaction, &task_request, None, None)?;
        crate::local_reuse::record_reacquire_requests(
            &transaction,
            &request.new_task_id,
            &request.reacquire_item_ids,
            &request.created_at,
        )?;
        crate::source_receipts::insert_historical_successor_receipt(
            &transaction,
            request,
            stored.runtime.revision,
        )?;
        transaction.commit()?;
        Ok(stored)
    }
}

/// Append sealed original products to a progressive admission task. The
/// original is never modified; new frozen task membership and reverse links
/// are written by the existing append transaction.
#[allow(clippy::too_many_arguments)]
fn append_progressive_source_items(
    transaction: &rusqlite::Transaction<'_>,
    mut task: StoredTask,
    source_id: &str,
    expected_revision: u64,
    item_ids: &[String],
    event_id: &str,
    occurred_at: &str,
    payload_json: &str,
) -> Result<StoredTask, StoreError> {
    if item_ids.is_empty()
        || item_ids.len() > crate::MAX_WORKBENCH_PAGE_SIZE as usize
        || item_ids.iter().collect::<HashSet<_>>().len() != item_ids.len()
    {
        return Err(StoreError::invalid(
            "progressive append requires unique bounded source IDs",
        ));
    }
    if task.runtime.revision != expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    let existing = task
        .runtime
        .selected_item_ids
        .iter()
        .collect::<HashSet<_>>();
    if item_ids.iter().any(|id| existing.contains(id)) {
        return Err(StoreError::invalid(
            "progressive source item is already selected",
        ));
    }
    let items = crate::batch_candidates::load_batch_candidate_items_connection(
        transaction,
        source_id,
        item_ids,
    )?
    .ok_or(StoreError::NotFound)?;
    if items.len() != item_ids.len() {
        return Err(StoreError::invalid(
            "progressive append source rows are missing",
        ));
    }
    // A conflicting active task must not silently duplicate the products.
    // This conservative check includes all indexed source relations.
    let requested = serde_json::to_string(item_ids).map_err(StoreError::Corrupt)?;
    let conflict: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM batch_task_items i \
         JOIN task_runtime r ON r.task_id = i.task_id \
         LEFT JOIN task_tombstones d ON d.task_id = i.task_id \
         WHERE i.source_batch_id = ?1 AND i.source_item_id IN (SELECT value FROM json_each(?2)) \
         AND i.task_id != ?3 AND d.task_id IS NULL \
         AND r.status IN ('queued','running','paused','interrupted'))",
        params![source_id, requested, task.runtime.task_id],
        |row| row.get(0),
    )?;
    if conflict {
        return Err(StoreError::invalid(
            "progressive source products already belong to an active task",
        ));
    }
    for mut item in items {
        if item.metadata.contains_key("source_item_id") {
            return Err(StoreError::invalid(
                "progressive source row uses the reserved backlink",
            ));
        }
        if item.provider_id == "asf.catalog"
            && item
                .metadata
                .get("dataset")
                .and_then(Value::as_str)
                .is_some_and(|dataset| dataset.eq_ignore_ascii_case("SLC-BURST"))
        {
            return Err(StoreError::invalid(
                "burst products require the explicit asset append path",
            ));
        }
        item.metadata.insert(
            "source_item_id".to_owned(),
            Value::String(item.item_id.clone()),
        );
        task.snapshot.candidates.push(item);
    }
    task.snapshot.validate().map_err(StoreError::invalid)?;
    append_task_membership_in_transaction(
        transaction,
        task,
        expected_revision,
        item_ids,
        event_id,
        occurred_at,
        payload_json,
    )
}

fn valid_history_task_id(value: &str) -> bool {
    valid_bounded_scalar(value, 128)
}

fn valid_bounded_scalar(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn valid_canonical_identity_code(value: &str) -> bool {
    valid_bounded_scalar(value, 128)
        && value.chars().all(|character| {
            character.is_ascii_uppercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-' | '.')
        })
}

pub(crate) fn validate_dem_source_identity(
    identity: &StoreDemSourceIdentity,
) -> Result<(), StoreError> {
    if !valid_canonical_identity_code(&identity.dataset)
        || !valid_canonical_identity_code(&identity.provider)
        || crate::canonical_utc_identity_timestamp(&identity.updated_at)
            .is_none_or(|canonical| canonical != identity.updated_at)
    {
        return Err(StoreError::invalid("DEM source identity is invalid"));
    }
    Ok(())
}

pub(crate) fn save_dem_source_identity_in_transaction(
    connection: &Connection,
    identity: &StoreDemSourceIdentity,
) -> Result<(), StoreError> {
    if !valid_canonical_identity_code(&identity.dataset)
        || !valid_canonical_identity_code(&identity.provider)
    {
        return Err(StoreError::invalid("DEM source identity is invalid"));
    }
    let updated_at = crate::canonical_utc_identity_timestamp(&identity.updated_at)
        .ok_or_else(|| StoreError::invalid("DEM source identity is invalid"))?;
    connection.execute(
        "INSERT INTO dem_source_identity(slot,dataset,provider,updated_at) VALUES (1,?1,?2,?3) \
         ON CONFLICT(slot) DO UPDATE SET dataset=excluded.dataset,provider=excluded.provider,updated_at=excluded.updated_at",
        params![identity.dataset, identity.provider, updated_at],
    )?;
    Ok(())
}

fn load_dem_source_identity(
    connection: &Connection,
) -> Result<Option<StoreDemSourceIdentity>, StoreError> {
    let identity = connection
        .query_row(
            "SELECT dataset,provider,updated_at FROM dem_source_identity WHERE slot=1",
            [],
            |row| {
                Ok(StoreDemSourceIdentity {
                    dataset: row.get(0)?,
                    provider: row.get(1)?,
                    updated_at: row.get(2)?,
                })
            },
        )
        .optional()?;
    if let Some(identity) = identity.as_ref() {
        validate_dem_source_identity(identity)?;
    }
    Ok(identity)
}

pub(crate) fn load_active_workbench_state(
    connection: &Connection,
    batch_id: &str,
    expected_revision: u64,
) -> Result<(SearchWorkbenchState, u64, u64, u64), StoreError> {
    let row: Option<(String, i64, String)> = connection
        .query_row(
            "SELECT batch_id, revision, state_json FROM workbench_state WHERE slot = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    // #22 B slice 5: a loading batch is never the active Workbench (it is
    // activated only by its seal), so stored selection and select-all over it
    // answer the typed still-loading error rather than an unknown batch.
    let Some((active_batch_id, revision_sql, state_json)) = row else {
        return Err(crate::batch_progress::missing_batch_error(
            connection, batch_id,
        ));
    };
    if active_batch_id != batch_id {
        return Err(crate::batch_progress::missing_batch_error(
            connection, batch_id,
        ));
    }
    let revision = u64::try_from(revision_sql)
        .map_err(|_| StoreError::invalid("workbench revision is invalid"))?;
    if revision != expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    let state: SearchWorkbenchState =
        serde_json::from_str(&state_json).map_err(StoreError::Corrupt)?;
    state.validate().map_err(StoreError::invalid)?;
    if state.batch_id != batch_id || state.revision != revision {
        return Err(StoreError::invalid("workbench state envelope differs"));
    }
    let (page_index, page_size) = (state.page_index, state.page_size);
    Ok((state, revision, page_index, page_size))
}

fn load_candidate_ids(connection: &Connection, batch_id: &str) -> Result<Vec<String>, StoreError> {
    crate::batch_page::load_candidate_ids_connection(connection, batch_id)
}

fn validate_candidate_delta(
    connection: &Connection,
    batch_id: &str,
    item_ids: &[String],
) -> Result<(), StoreError> {
    crate::batch_page::validate_candidate_delta_connection(connection, batch_id, item_ids)
}

pub(crate) fn task_is_actionable(
    connection: &Connection,
    task_id: &str,
) -> Result<bool, StoreError> {
    let row = load_one_task_row(connection, task_id)?;
    Ok(row.is_some_and(|row| matches!(project_task(row), TaskProjection::Compatible(_))))
}

pub(crate) fn actionable_task_summary(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<StoreTaskSummary>, StoreError> {
    // History materialization is a one-time boundary, so use the full frozen
    // SearchBatch authority here. This validates products appended after the
    // compact snapshot's initial selected-product subset without putting the
    // full batch on the bounded CoreRestore/status path.
    match load_task_connection(connection, task_id) {
        Ok(Some(_)) => {}
        Ok(None) | Err(_) => return Ok(None),
    }
    let row = load_one_task_row(connection, task_id)?;
    Ok(row.and_then(|row| match project_task(row) {
        TaskProjection::Compatible(summary) => Some(*summary),
        TaskProjection::Degraded(_) => None,
    }))
}

fn load_task_rows(
    connection: &Connection,
    include_terminal: bool,
) -> Result<Vec<TaskRow>, StoreError> {
    let sql = if include_terminal {
        "SELECT s.task_id, s.batch_id, s.provider_id, s.created_at, s.output_root, s.layout, \
                s.snapshot_json, r.status, r.revision, r.updated_at, r.state_json \
         FROM task_snapshots s \
         LEFT JOIN task_runtime r ON r.task_id = s.task_id \
         LEFT JOIN task_tombstones d ON d.task_id = s.task_id \
         WHERE d.task_id IS NULL ORDER BY s.created_at, s.task_id"
    } else {
        "SELECT s.task_id, s.batch_id, s.provider_id, s.created_at, s.output_root, s.layout, \
                s.snapshot_json, r.status, r.revision, r.updated_at, r.state_json \
         FROM task_snapshots s \
         LEFT JOIN task_runtime r ON r.task_id = s.task_id \
         LEFT JOIN task_history h ON h.task_id = s.task_id \
         LEFT JOIN task_tombstones d ON d.task_id = s.task_id \
         WHERE h.task_id IS NULL AND d.task_id IS NULL ORDER BY s.created_at, s.task_id"
    };
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], task_row_from_sql)?;
    let mut rows = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)?;
    let task_ids = rows
        .iter()
        .map(|row| row.task_id.clone())
        .collect::<Vec<_>>();
    let mut additions = load_task_membership_additions_batched(connection, &task_ids)?;
    for row in &mut rows {
        row.membership_additions = additions
            .remove(&row.task_id)
            .unwrap_or_else(|| Some(Vec::new()));
    }
    Ok(rows)
}

fn load_one_task_row(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<TaskRow>, StoreError> {
    let mut row = connection
        .query_row(
            "SELECT s.task_id, s.batch_id, s.provider_id, s.created_at, s.output_root, s.layout, \
                    s.snapshot_json, r.status, r.revision, r.updated_at, r.state_json \
             FROM task_snapshots s LEFT JOIN task_runtime r ON r.task_id = s.task_id \
             WHERE s.task_id = ?1",
            [task_id],
            task_row_from_sql,
        )
        .optional()
        .map_err(StoreError::from)?;
    if let Some(row) = row.as_mut() {
        let mut additions =
            load_task_membership_additions_batched(connection, std::slice::from_ref(&row.task_id))?;
        row.membership_additions = additions
            .remove(&row.task_id)
            .unwrap_or_else(|| Some(Vec::new()));
    }
    Ok(row)
}

fn task_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    let layout = parse_download_layout(&row.get::<_, String>(5)?);
    Ok(TaskRow {
        task_id: row.get(0)?,
        batch_id: row.get(1)?,
        provider_id: row.get(2)?,
        created_at: row.get(3)?,
        output_root: PathBuf::from(row.get::<_, String>(4)?),
        layout,
        snapshot_json: row.get(6)?,
        runtime_status: row.get(7)?,
        runtime_revision: row.get(8)?,
        runtime_updated_at: row.get(9)?,
        runtime_json: row.get(10)?,
        membership_additions: Some(Vec::new()),
    })
}

/// Scalar-only terminal history projection. The query intentionally never
/// returns snapshot_json or runtime state_json: archive-time/backfill authority
/// validation has already materialized this fixed-size row.
fn history_summary_from_sql(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(StoreTaskHistoryCursor, String, StoreTaskSummary)> {
    let archived_at: String = row.get(0)?;
    let archived_at_epoch_ms: i64 = row.get(1)?;
    let task_id: String = row.get(2)?;
    let status_name: String = row.get(8)?;
    let revision =
        u64::try_from(row.get::<_, i64>(9)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let status = parse_lifecycle_status(&status_name).ok_or(rusqlite::Error::InvalidQuery)?;
    let count = |index| -> rusqlite::Result<usize> {
        usize::try_from(row.get::<_, i64>(index)?).map_err(|_| rusqlite::Error::InvalidQuery)
    };
    let operation_id: Option<String> = row.get(20)?;
    let kind: Option<String> = row.get(21)?;
    let phase: Option<String> = row.get(22)?;
    let region_id: Option<String> = row.get(23)?;
    let approved_worker_operation: Option<String> = row.get(24)?;
    // Orbit scheduled-fetch S4: the auxiliary kind derives from the frozen
    // worker identity; the source task id is projected from the surviving
    // task snapshot's operation payload (null for ordinary archived tasks
    // and rows whose snapshot is gone).
    let auxiliary_kind: Option<String> = row.get(29)?;
    let source_task_id: Option<String> = row.get(30)?;
    let source_task_id = source_task_id.filter(|value| {
        !value.trim().is_empty()
            && value.chars().count() <= 128
            && !value.chars().any(char::is_control)
    });
    let transfer_active_millis =
        u64::try_from(row.get::<_, i64>(25)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let operation = match (
        operation_id,
        kind,
        phase,
        region_id,
        approved_worker_operation,
    ) {
        (None, None, None, None, None) => None,
        (
            Some(operation_id),
            Some(kind),
            Some(phase),
            Some(region_id),
            Some(approved_worker_operation),
        ) => Some(StoreTaskOperationSummary {
            operation_id,
            kind,
            phase,
            region_id,
            approved_worker_operation,
            source_provider: None,
            input_path: None,
            source_vertical_datum: None,
            convert: None,
        }),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok((
        StoreTaskHistoryCursor {
            archived_at_epoch_ms,
            task_id: task_id.clone(),
        },
        archived_at,
        StoreTaskSummary {
            task_id,
            batch_id: row.get(3)?,
            provider_id: row.get(4)?,
            dataset_id: row.get(26)?,
            concurrency: row
                .get::<_, Option<i64>>(27)?
                .and_then(|value| u64::try_from(value).ok()),
            task_type: row.get(28)?,
            created_at: row.get(5)?,
            status,
            revision,
            updated_at: row.get(10)?,
            selected_count: count(11)?,
            delivery_requested_count: None,
            delivery_rejected_count: None,
            checked_count: count(12)?,
            active_count: count(13)?,
            paused_count: count(14)?,
            queued_count: count(15)?,
            completed_count: count(16)?,
            failed_count: count(17)?,
            skipped_count: count(18)?,
            cancelled_count: count(19)?,
            terminal_bytes: 0,
            active_bytes: 0,
            other_bytes: 0,
            transferred_bytes: 0,
            total_bytes: None,
            source_total_bytes: None,
            bytes_per_second: 0.0,
            transfer_active_millis,
            active_item: None,
            active_progress: StoreTaskActiveProgressSummary::default(),
            failure_codes: Vec::new(),
            local_scan: None,
            network_bytes_received: None,
            reusable_verified_bytes: None,
            reused_verified_bytes: None,
            output_root: PathBuf::from(row.get::<_, String>(6)?),
            layout: parse_download_layout(&row.get::<_, String>(7)?)
                .ok_or(rusqlite::Error::InvalidQuery)?,
            operation,
            source_task_id,
            auxiliary_kind,
            actionable: true,
        },
    ))
}

fn parse_download_layout(value: &str) -> Option<DownloadLayout> {
    match value {
        "direct" => Some(DownloadLayout::Direct),
        "sentinel1_slc" => Some(DownloadLayout::Sentinel1Slc),
        "sentinel_orbit_aux_poeorb" => Some(DownloadLayout::SentinelOrbitAuxPoeorb),
        "sar_dataset_processing" => Some(DownloadLayout::SarDatasetProcessing),
        _ => None,
    }
}

fn project_task(row: TaskRow) -> TaskProjection {
    let stored_status = row
        .runtime_status
        .as_deref()
        .and_then(parse_lifecycle_status);
    match decode_any_snapshot(&row.snapshot_json) {
        Ok(StoredTaskSnapshot::CompactV2(snapshot)) => {
            project_compact_task(row, snapshot, stored_status)
        }
        Ok(StoredTaskSnapshot::LegacyV2(snapshot)) => {
            project_legacy_v2_task(row, snapshot, stored_status)
        }
        Ok(StoredTaskSnapshot::V3(_)) => unavailable(
            &row,
            stored_status,
            LegacyTaskRestoreReasonCode::UnsupportedPersistenceVersion,
        ),
        Err(error) => {
            let reason = if error.code() == "V3_STORE_UNSUPPORTED_SNAPSHOT_VERSION" {
                LegacyTaskRestoreReasonCode::UnsupportedPersistenceVersion
            } else {
                LegacyTaskRestoreReasonCode::MalformedSnapshot
            };
            unavailable(&row, stored_status, reason)
        }
    }
}

fn bounded_task_dataset_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 128 || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.to_owned())
}

fn task_operation_scalar(
    operation: Option<&insar_core::TaskOperationDescriptor>,
    key: &str,
) -> Option<String> {
    operation
        .and_then(|operation| operation.payload.get(key))
        .and_then(Value::as_str)
        .and_then(bounded_task_dataset_id)
}

fn task_operation_text(
    operation: Option<&insar_core::TaskOperationDescriptor>,
    key: &str,
    max_chars: usize,
) -> Option<String> {
    let value = operation
        .and_then(|operation| operation.payload.get(key))
        .and_then(Value::as_str)?
        .trim();
    (!value.is_empty()
        && value.chars().count() <= max_chars
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}

fn task_operation_bool(
    operation: Option<&insar_core::TaskOperationDescriptor>,
    key: &str,
) -> Option<bool> {
    operation
        .and_then(|operation| operation.payload.get(key))
        .and_then(Value::as_bool)
}

fn task_dataset_id_with_operation(
    query: &serde_json::Map<String, Value>,
    products: &[SnapshotItem],
    operation: Option<&insar_core::TaskOperationDescriptor>,
) -> Option<String> {
    task_dataset_id(query, products).or_else(|| task_operation_scalar(operation, "dataset"))
}

fn task_dataset_id(
    query: &serde_json::Map<String, Value>,
    products: &[SnapshotItem],
) -> Option<String> {
    let one_unique = |values: Vec<&str>| {
        let mut unique = HashMap::<String, String>::new();
        for value in values {
            let value = bounded_task_dataset_id(value)?;
            unique.entry(value.to_uppercase()).or_insert(value);
        }
        (unique.len() == 1)
            .then(|| unique.into_values().next())
            .flatten()
    };
    let mut query_values = Vec::new();
    for key in ["dataset", "logical_dataset"] {
        if let Some(value) = query.get(key).and_then(Value::as_str) {
            query_values.push(value);
        }
    }
    if let Some(datasets) = query.get("datasets").and_then(Value::as_array) {
        query_values.extend(datasets.iter().filter_map(Value::as_str));
    }
    if !query_values.is_empty() {
        return one_unique(query_values);
    }
    let mut product_values = Vec::new();
    for product in products {
        for key in ["dataset", "logical_dataset"] {
            if let Some(value) = product.metadata.get(key).and_then(Value::as_str) {
                product_values.push(value);
            }
        }
    }
    one_unique(product_values)
}

pub(crate) fn task_dataset_id_from_snapshot_json(snapshot_json: &str) -> Option<String> {
    match decode_any_snapshot(snapshot_json).ok()? {
        StoredTaskSnapshot::CompactV2(snapshot) => task_dataset_id_with_operation(
            &snapshot.query,
            &snapshot.selected_products,
            snapshot.operation.as_ref(),
        ),
        StoredTaskSnapshot::LegacyV2(snapshot) => task_dataset_id_with_operation(
            &snapshot.query,
            &snapshot.candidates,
            snapshot.operation.as_ref(),
        ),
        StoredTaskSnapshot::V3(_) => None,
    }
}

fn task_type_for(dataset_id: Option<&str>, products: &[SnapshotItem]) -> Option<String> {
    if dataset_id.is_some_and(|value| value.eq_ignore_ascii_case("SENTINEL-1")) {
        for item in products {
            if let Some(pt) = item.metadata.get("product_type").and_then(Value::as_str) {
                let upper = pt.trim().to_ascii_uppercase();
                if upper.contains("SLC") {
                    return Some("sentinel1_slc".to_owned());
                }
                if upper.contains("GRD") {
                    return Some("sentinel1_grd".to_owned());
                }
                if upper.contains("RAW") {
                    return Some("sentinel1_raw".to_owned());
                }
                if upper.contains("OCN") {
                    return Some("sentinel1_ocn".to_owned());
                }
            }
            let id_upper = item.item_id.to_ascii_uppercase();
            if id_upper.contains("_SLC_") || id_upper.ends_with("-SLC") {
                return Some("sentinel1_slc".to_owned());
            }
            if id_upper.contains("_GRD") || id_upper.ends_with("-GRD") {
                return Some("sentinel1_grd".to_owned());
            }
        }
        return None;
    }
    if !dataset_id.is_some_and(|value| {
        value.eq_ignore_ascii_case("sar.nisar.l.rslc.provisional.v1")
            || value.eq_ignore_ascii_case("sar.nisar.s.ssar.rslc.v1")
    }) {
        return None;
    }
    let subset = products.iter().any(|item| {
        item.metadata.get("delivery_kind").and_then(Value::as_str)
            == Some("remote_derived_subset_v2")
    });
    Some(if subset {
        "nisar_subset".to_owned()
    } else {
        "nisar_complete".to_owned()
    })
}

fn delivery_admission_counts(
    query: &serde_json::Map<String, Value>,
    selected_count: usize,
) -> (Option<usize>, Option<usize>) {
    let kind = query.get("delivery_kind").and_then(Value::as_str);
    if !matches!(
        kind,
        Some(
            "sentinel2_cdse_safe_archive_batch"
                | "landsat_c2_l2_asset_batch"
                | "landsat_c2_l2_bundle_batch"
                | "sentinel2_l2a_analysis_band_batch"
                | "sentinel2_l1c_band_batch"
                | "hls_multiband_batch"
        )
    ) {
        return (None, None);
    }
    let Some(requested) = query
        .get("delivery_requested_count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    else {
        return (None, None);
    };
    let Some(admitted) = query
        .get("delivery_admitted_count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    else {
        return (None, None);
    };
    let Some(rejected) = query
        .get("delivery_rejected_count")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    else {
        return (None, None);
    };
    // SAFE has one transfer per scene; optical asset batches have many transfers.
    let transfer_count_matches = if matches!(
        kind,
        Some(
            "landsat_c2_l2_asset_batch"
                | "sentinel2_l2a_analysis_band_batch"
                | "sentinel2_l1c_band_batch"
                | "hls_multiband_batch"
        )
    ) {
        let max_band_count = match kind {
            Some("hls_multiband_batch") => Some(14usize),
            Some("sentinel2_l1c_band_batch") => Some(13usize),
            Some("sentinel2_l2a_analysis_band_batch") => Some(12usize),
            _ => None,
        };
        admitted > 0
            && admitted <= selected_count
            && query.get("delivery_asset_count").and_then(Value::as_u64)
                == Some(selected_count as u64)
            && max_band_count.is_none_or(|max_len| {
                query
                    .get("delivery_band_keys")
                    .and_then(Value::as_array)
                    .is_some_and(|keys| {
                        !keys.is_empty()
                            && keys.len() <= max_len
                            && admitted.checked_mul(keys.len()) == Some(selected_count)
                    })
            })
    } else {
        admitted == selected_count
    };
    if requested == 0
        || !transfer_count_matches
        || admitted.checked_add(rejected) != Some(requested)
    {
        return (None, None);
    }
    (Some(requested), Some(rejected))
}

fn project_compact_task(
    row: TaskRow,
    snapshot: CompactTaskSnapshotV2,
    stored_status: Option<TaskLifecycleStatus>,
) -> TaskProjection {
    let stored_snapshot = StoredTaskSnapshot::CompactV2(snapshot.clone());
    let runtime = match validate_task_and_runtime(&row, &stored_snapshot) {
        Ok(runtime) => runtime,
        Err(reason) => return unavailable(&row, stored_status, reason),
    };
    if snapshot.selected_products.is_empty() {
        return unavailable(
            &row,
            stored_status,
            LegacyTaskRestoreReasonCode::AmbiguousLegacyEvidence,
        );
    }
    if has_missing_legacy_display_fields(&snapshot) {
        if exact_sentinel1_slc_legacy_predicate(&row, &snapshot) {
            return TaskProjection::Degraded(LegacyTaskRestoreWarning {
                task_id: row.task_id,
                provider_id: row.provider_id,
                created_at: row.created_at,
                status: Some(runtime.status),
                reason_code: LegacyTaskRestoreReasonCode::Sentinel1SlcDisplayFieldsAdapted,
                compatibility_status: LegacyTaskCompatibilityStatus::AdaptedReadOnly,
            });
        }
        return unavailable(
            &row,
            stored_status,
            LegacyTaskRestoreReasonCode::AmbiguousLegacyEvidence,
        );
    }

    let dataset_id = task_dataset_id_with_operation(
        &snapshot.query,
        &snapshot.selected_products,
        snapshot.operation.as_ref(),
    );
    let task_type = task_type_for(dataset_id.as_deref(), &snapshot.selected_products);
    let (delivery_requested_count, delivery_rejected_count) =
        delivery_admission_counts(&snapshot.query, runtime.selected_item_ids.len());
    let source_total_bytes = row
        .membership_additions
        .as_deref()
        .and_then(|additions| {
            hydrate_task_snapshot_for_runtime(stored_snapshot, additions, &runtime).ok()
        })
        .and_then(|hydrated| {
            frozen_selected_source_total_bytes(&hydrated.candidates, &runtime.selected_item_ids)
        });
    compatible_summary(
        row,
        runtime,
        CompatibleTaskSummaryFields {
            dataset_id,
            task_type,
            delivery_requested_count,
            delivery_rejected_count,
            source_total_bytes,
            output_root: snapshot.output_root,
            layout: snapshot.layout,
            operation: snapshot.operation,
        },
    )
}

fn project_legacy_v2_task(
    row: TaskRow,
    snapshot: insar_core::FrozenTaskSnapshot,
    stored_status: Option<TaskLifecycleStatus>,
) -> TaskProjection {
    let stored_snapshot = StoredTaskSnapshot::LegacyV2(snapshot.clone());
    let runtime = match validate_task_and_runtime(&row, &stored_snapshot) {
        Ok(runtime) => runtime,
        Err(reason) => return unavailable(&row, stored_status, reason),
    };
    let candidate_ids: HashSet<&str> = snapshot
        .candidates
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    if snapshot.candidates.is_empty()
        || candidate_ids.len() != snapshot.candidates.len()
        || runtime
            .selected_item_ids
            .iter()
            .any(|item_id| !candidate_ids.contains(item_id.as_str()))
        || runtime
            .checked_item_ids
            .iter()
            .any(|item_id| !candidate_ids.contains(item_id.as_str()))
    {
        return unavailable(
            &row,
            stored_status,
            LegacyTaskRestoreReasonCode::AmbiguousLegacyEvidence,
        );
    }
    let dataset_id = task_dataset_id_with_operation(
        &snapshot.query,
        &snapshot.candidates,
        snapshot.operation.as_ref(),
    );
    let task_type = task_type_for(dataset_id.as_deref(), &snapshot.candidates);
    let (delivery_requested_count, delivery_rejected_count) =
        delivery_admission_counts(&snapshot.query, runtime.selected_item_ids.len());
    let source_total_bytes = row
        .membership_additions
        .as_deref()
        .and_then(|additions| {
            hydrate_task_snapshot_for_runtime(stored_snapshot, additions, &runtime).ok()
        })
        .and_then(|hydrated| {
            frozen_selected_source_total_bytes(&hydrated.candidates, &runtime.selected_item_ids)
        });
    compatible_summary(
        row,
        runtime,
        CompatibleTaskSummaryFields {
            dataset_id,
            task_type,
            delivery_requested_count,
            delivery_rejected_count,
            source_total_bytes,
            output_root: snapshot.output_root,
            layout: snapshot.layout,
            operation: snapshot.operation,
        },
    )
}

fn validate_task_and_runtime(
    row: &TaskRow,
    snapshot: &StoredTaskSnapshot,
) -> Result<TaskRuntimeSnapshot, LegacyTaskRestoreReasonCode> {
    let Some(layout) = row.layout.clone() else {
        return Err(LegacyTaskRestoreReasonCode::TaskEnvelopeMismatch);
    };
    if validate_task_snapshot_row_envelope(
        TaskSnapshotRowEnvelope {
            task_id: &row.task_id,
            batch_id: &row.batch_id,
            provider_id: &row.provider_id,
            created_at: &row.created_at,
            output_root: &row.output_root,
            layout: &layout,
        },
        snapshot,
    )
    .is_err()
    {
        return Err(LegacyTaskRestoreReasonCode::TaskEnvelopeMismatch);
    }

    let Some(runtime_json) = row.runtime_json.as_deref() else {
        return Err(LegacyTaskRestoreReasonCode::MissingOrInvalidRuntime);
    };
    let runtime = match serde_json::from_str::<TaskRuntimeSnapshot>(runtime_json) {
        Ok(runtime) if runtime.validate().is_ok() => runtime,
        _ => return Err(LegacyTaskRestoreReasonCode::MissingOrInvalidRuntime),
    };
    let runtime_envelope_matches = runtime.task_id == row.task_id
        && row.runtime_status.as_deref() == Some(lifecycle_status_name(&runtime.status))
        && row
            .runtime_revision
            .and_then(|value| u64::try_from(value).ok())
            == Some(runtime.revision)
        && row.runtime_updated_at.as_deref() == Some(runtime.updated_at.as_str());
    if !runtime_envelope_matches {
        return Err(LegacyTaskRestoreReasonCode::RuntimeEnvelopeMismatch);
    }
    let graph_validation = match snapshot {
        StoredTaskSnapshot::LegacyV2(snapshot) => {
            validate_runtime_item_graph(&snapshot.candidates, &runtime)
        }
        // Compact persistence deliberately stores only initial selected
        // products. Appended products are validated against SearchBatch by the
        // actionability/materialization boundary above.
        StoredTaskSnapshot::CompactV2(_) => validate_runtime_lifecycle_partition(&runtime),
        StoredTaskSnapshot::V3(_) => {
            return Err(LegacyTaskRestoreReasonCode::AmbiguousLegacyEvidence)
        }
    };
    if graph_validation.is_err() {
        return Err(LegacyTaskRestoreReasonCode::AmbiguousLegacyEvidence);
    }
    Ok(runtime)
}

struct CompatibleTaskSummaryFields {
    dataset_id: Option<String>,
    task_type: Option<String>,
    delivery_requested_count: Option<usize>,
    delivery_rejected_count: Option<usize>,
    source_total_bytes: Option<u64>,
    output_root: PathBuf,
    layout: DownloadLayout,
    operation: Option<insar_core::TaskOperationDescriptor>,
}

fn compatible_summary(
    row: TaskRow,
    runtime: TaskRuntimeSnapshot,
    fields: CompatibleTaskSummaryFields,
) -> TaskProjection {
    let CompatibleTaskSummaryFields {
        dataset_id,
        task_type,
        delivery_requested_count,
        delivery_rejected_count,
        source_total_bytes,
        output_root,
        layout,
        operation,
    } = fields;
    let (active_count, paused_count) = if runtime.task_paused {
        (
            0,
            runtime.paused_item_ids.len() + runtime.active_item_ids.len(),
        )
    } else {
        (runtime.active_item_ids.len(), runtime.paused_item_ids.len())
    };
    let source_provider = task_operation_scalar(operation.as_ref(), "provider");
    let input_path = task_operation_text(operation.as_ref(), "input_path", 4_096);
    let source_vertical_datum =
        task_operation_text(operation.as_ref(), "source_vertical_datum", 128);
    let convert = task_operation_bool(operation.as_ref(), "convert");
    // Orbit scheduled-fetch S4 (plan B): the companion-orbit operation
    // payload already persists `source_task_id`; it is only projected here.
    // The auxiliary kind is derived from the frozen worker identity, never
    // from a caller-controlled field.
    let source_task_id = task_operation_text(operation.as_ref(), "source_task_id", 128);
    let auxiliary_kind = operation
        .as_ref()
        .filter(|operation| operation.approved_worker_operation == "orbit_download")
        .map(|_| "AUX_POEORB".to_owned());
    TaskProjection::Compatible(Box::new(StoreTaskSummary {
        task_id: row.task_id,
        batch_id: row.batch_id,
        provider_id: row.provider_id,
        dataset_id,
        concurrency: Some(runtime.concurrency),
        task_type,
        created_at: row.created_at,
        status: runtime.status,
        revision: runtime.revision,
        updated_at: runtime.updated_at,
        selected_count: runtime.selected_item_ids.len(),
        delivery_requested_count,
        delivery_rejected_count,
        checked_count: runtime.checked_item_ids.len(),
        active_count,
        paused_count,
        queued_count: runtime.queued_item_ids.len(),
        completed_count: runtime.completed_item_ids.len(),
        failed_count: runtime.failed_item_ids.len(),
        skipped_count: runtime.skipped_item_ids.len(),
        cancelled_count: runtime.cancelled_item_ids.len(),
        terminal_bytes: 0,
        active_bytes: 0,
        other_bytes: 0,
        transferred_bytes: 0,
        total_bytes: None,
        source_total_bytes,
        bytes_per_second: 0.0,
        transfer_active_millis: 0,
        active_item: None,
        active_progress: StoreTaskActiveProgressSummary::default(),
        failure_codes: Vec::new(),
        local_scan: None,
        network_bytes_received: None,
        reusable_verified_bytes: None,
        reused_verified_bytes: None,
        output_root,
        layout,
        operation: operation.map(|operation| StoreTaskOperationSummary {
            operation_id: operation.operation_id,
            kind: operation.kind,
            phase: operation.phase,
            region_id: operation.region_id,
            approved_worker_operation: operation.approved_worker_operation,
            source_provider,
            input_path,
            source_vertical_datum,
            convert,
        }),
        source_task_id,
        auxiliary_kind,
        actionable: true,
    }))
}

/// Frozen catalogue bytes are descriptive progress, never execution authority.
/// Measured remote totals take priority. ASF stores these bytes in its payload;
/// do not coerce string, fractional or negative values into download totals.
fn frozen_item_total_bytes(item: &SnapshotItem) -> Option<u64> {
    item.asset_access
        .as_ref()
        .and_then(|access| access.expected_size_bytes)
        .or_else(|| {
            item.payload
                .get("file_size_remote")
                .and_then(Value::as_i64)
                .and_then(|value| u64::try_from(value).ok())
        })
}

fn frozen_selected_source_total_bytes(
    items: &[SnapshotItem],
    selected_item_ids: &[String],
) -> Option<u64> {
    let mut by_id = HashMap::with_capacity(items.len());
    for item in items {
        if by_id.insert(item.item_id.as_str(), item).is_some() {
            return None;
        }
    }
    selected_item_ids.iter().try_fold(0_u64, |total, item_id| {
        let item = by_id.get(item_id.as_str())?;
        total.checked_add(frozen_item_total_bytes(item)?)
    })
}

#[cfg(test)]
mod frozen_source_total_tests {
    use super::*;
    use serde_json::{json, Map};

    fn item(item_id: &str, size: u64) -> SnapshotItem {
        SnapshotItem {
            item_id: item_id.to_owned(),
            provider_id: "fixture".to_owned(),
            display_name: item_id.to_owned(),
            acquisition_datetime: None,
            footprint_geojson: None,
            metadata: Map::new(),
            payload: Map::from_iter([("file_size_remote".to_owned(), json!(size))]),
            asset_access: None,
            execution_policy: None,
        }
    }

    #[test]
    fn selected_source_total_checked_addition_fails_closed_on_u64_overflow() {
        let items = [
            item("A", i64::MAX as u64),
            item("B", i64::MAX as u64),
            item("C", 2),
        ];
        let selected = vec!["A".to_owned(), "B".to_owned(), "C".to_owned()];
        assert_eq!(frozen_selected_source_total_bytes(&items, &selected), None);
    }
}

// Query-local projections of authoritative rows, not another cache or Store.
// Parse each task's frozen products and lifecycle sets once per query instead
// of rescanning them for every selected item; retain status and size precedence.
const TASK_TELEMETRY_SQL: &str = r#"WITH included AS (SELECT value AS task_id FROM json_each(?1)),
          task_inputs AS MATERIALIZED (
            SELECT t.task_id, t.snapshot_json, r.state_json
            FROM task_snapshots t
            JOIN included i ON i.task_id=t.task_id
            JOIN task_runtime r ON r.task_id=t.task_id
            WHERE json_valid(t.snapshot_json) AND json_valid(r.state_json)
          ), frozen AS MATERIALIZED (
            SELECT t.task_id,
              COALESCE(json_extract(candidate.value, '$.item_id'),
                       json_extract(candidate.value, '$.item.item_id')) AS item_id,
              COALESCE(
                json_extract(candidate.value, '$.asset_access.expected_size_bytes'),
                json_extract(candidate.value, '$.item.asset_access.expected_size_bytes'),
                CASE WHEN json_type(candidate.value, '$.payload.file_size_remote') = 'integer'
                  AND typeof(json_extract(candidate.value, '$.payload.file_size_remote')) = 'integer'
                  AND json_extract(candidate.value, '$.payload.file_size_remote') >= 0
                  THEN json_extract(candidate.value, '$.payload.file_size_remote') END,
                CASE WHEN json_type(candidate.value, '$.item.payload.file_size_remote') = 'integer'
                  AND typeof(json_extract(candidate.value, '$.item.payload.file_size_remote')) = 'integer'
                  AND json_extract(candidate.value, '$.item.payload.file_size_remote') >= 0
                  THEN json_extract(candidate.value, '$.item.payload.file_size_remote') END
              ) AS frozen_total
            FROM task_inputs t
            JOIN json_each(
              CASE
                WHEN json_type(t.snapshot_json, '$.selected_products') = 'array'
                  THEN json_extract(t.snapshot_json, '$.selected_products')
                WHEN json_type(t.snapshot_json, '$.candidates') = 'array'
                  THEN json_extract(t.snapshot_json, '$.candidates')
                WHEN json_type(t.snapshot_json, '$.source_items') = 'array'
                  THEN json_extract(t.snapshot_json, '$.source_items')
                ELSE json('[]')
              END
            ) candidate
          ), state_paths(path, priority) AS (
            VALUES ('$.completed_item_ids', 1), ('$.failed_item_ids', 2),
                   ('$.skipped_item_ids', 3), ('$.cancelled_item_ids', 4),
                   ('$.paused_item_ids', 5), ('$.active_item_ids', 6)
          ), item_states AS MATERIALIZED (
            SELECT r.task_id, member.value AS item_id,
              MIN(CASE WHEN p.priority=6 AND json_extract(r.state_json, '$.task_paused')=1
                       THEN 0 ELSE p.priority END) AS priority
            FROM task_inputs r CROSS JOIN state_paths p
            JOIN json_each(r.state_json, p.path) member
            GROUP BY r.task_id, member.value
          ), selected AS (
            SELECT r.task_id, selected.value AS item_id, candidate.frozen_total,
              CASE state.priority
                WHEN 0 THEN 'paused' WHEN 1 THEN 'completed' WHEN 2 THEN 'failed'
                WHEN 3 THEN 'skipped' WHEN 4 THEN 'cancelled' WHEN 5 THEN 'paused'
                WHEN 6 THEN 'active' ELSE 'queued' END AS runtime_status
            FROM task_inputs r
            JOIN json_each(r.state_json, '$.selected_item_ids') selected
            LEFT JOIN item_states state ON state.task_id=r.task_id AND state.item_id=selected.value
            LEFT JOIN frozen candidate ON candidate.task_id=r.task_id AND candidate.item_id=selected.value
          ), per_item AS (
            SELECT s.task_id, s.item_id, s.runtime_status, p.bytes_completed, p.bytes_per_second,
              p.status AS progress_status, r.bytes_written,
              CASE WHEN s.runtime_status IN ('completed','failed','skipped','cancelled')
                THEN COALESCE(r.last_known_total_bytes, p.last_known_total_bytes,
                  CASE WHEN r.outcome IN ('completed','skipped') THEN r.bytes_written END, s.frozen_total)
                ELSE COALESCE(p.last_known_total_bytes, s.frozen_total) END AS effective_total
            FROM selected s
            LEFT JOIN download_progress p ON p.task_id=s.task_id AND p.item_id=s.item_id
            LEFT JOIN download_results r ON r.task_id=s.task_id AND r.item_id=s.item_id
          ), ranked_active AS (
            SELECT task_id, item_id, COALESCE(bytes_completed, 0) AS bytes_completed, effective_total,
              ROW_NUMBER() OVER (PARTITION BY task_id ORDER BY item_id) AS rank
            FROM per_item WHERE runtime_status='active'
          )
          SELECT p.task_id,
            COALESCE(SUM(CASE WHEN p.runtime_status IN ('completed','failed','skipped','cancelled') THEN COALESCE(p.bytes_written, p.bytes_completed, 0) ELSE 0 END),0),
            COALESCE(SUM(CASE WHEN p.runtime_status='active' THEN COALESCE(p.bytes_completed,0) ELSE 0 END),0),
            COALESCE(SUM(CASE WHEN p.runtime_status NOT IN ('completed','failed','skipped','cancelled','active') THEN COALESCE(p.bytes_completed,0) ELSE 0 END),0),
            CASE WHEN SUM(CASE WHEN p.effective_total IS NULL THEN 1 ELSE 0 END)=0 THEN SUM(p.effective_total) END,
            COALESCE(SUM(CASE WHEN p.runtime_status='active' THEN COALESCE(p.bytes_per_second,0) ELSE 0 END),0),
            a.item_id, a.bytes_completed, a.effective_total,
            COALESCE(t.transfer_active_millis, 0), t.network_bytes_received,
            COALESCE(SUM(CASE WHEN p.runtime_status='active' AND p.bytes_completed IS NULL THEN 1 ELSE 0 END),0),
            COALESCE(SUM(CASE WHEN p.runtime_status='active' AND p.bytes_completed IS NOT NULL
              AND COALESCE(p.bytes_per_second,0) > 0 THEN 1 ELSE 0 END),0),
            COALESCE(SUM(CASE WHEN p.runtime_status='active' AND p.bytes_completed IS NOT NULL
              AND NOT COALESCE(p.bytes_per_second,0) > 0
              AND COALESCE(p.progress_status,'') <> 'queued' THEN 1 ELSE 0 END),0),
            COALESCE(SUM(CASE WHEN p.runtime_status='active' AND p.bytes_completed IS NOT NULL
              AND NOT COALESCE(p.bytes_per_second,0) > 0
              AND p.progress_status = 'queued' THEN 1 ELSE 0 END),0)
          FROM per_item p
          LEFT JOIN ranked_active a ON a.task_id=p.task_id AND a.rank=1
          LEFT JOIN task_transfer_telemetry t ON t.task_id=p.task_id
          GROUP BY p.task_id, t.transfer_active_millis, t.network_bytes_received"#;

fn load_task_telemetries(
    connection: &Connection,
    task_ids: &[String],
) -> Result<HashMap<String, StoreTaskTelemetrySummary>, StoreError> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let task_ids_json = serde_json::to_string(task_ids).map_err(StoreError::Corrupt)?;
    let mut statement = connection.prepare(TASK_TELEMETRY_SQL)?;
    let rows = statement.query_map([task_ids_json], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, Option<i64>>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, Option<i64>>(10)?,
            [
                row.get::<_, i64>(11)?,
                row.get::<_, i64>(12)?,
                row.get::<_, i64>(13)?,
                row.get::<_, i64>(14)?,
            ],
        ))
    })?;
    let mut telemetry = HashMap::new();
    for row in rows {
        let (
            task_id,
            terminal,
            active,
            other,
            total,
            rate,
            active_id,
            active_completed,
            active_total,
            transfer_active_millis,
            network_bytes_received,
            [without_progress, transferring, idle, waiting],
        ) = row?;
        let as_u64 =
            |value: i64, label: &str| u64::try_from(value).map_err(|_| StoreError::invalid(label));
        let terminal_bytes = as_u64(terminal, "negative terminal bytes")?;
        let active_bytes = as_u64(active, "negative active bytes")?;
        let other_bytes = as_u64(other, "negative other bytes")?;
        let total_bytes = total
            .map(|value| as_u64(value, "negative total bytes"))
            .transpose()?;
        let active_item = match (active_id, active_completed, active_total) {
            (Some(item_id), Some(bytes_completed), active_total) => {
                Some(StoreTaskActiveItemSummary {
                    item_id,
                    bytes_completed: as_u64(bytes_completed, "negative active item bytes")?,
                    total_bytes: active_total
                        .map(|value| as_u64(value, "negative active item total"))
                        .transpose()?,
                })
            }
            (None, None, _) => None,
            _ => {
                return Err(StoreError::invalid(
                    "representative active telemetry is incomplete",
                ))
            }
        };
        telemetry.insert(
            task_id,
            StoreTaskTelemetrySummary {
                network_bytes_received: network_bytes_received
                    .map(|value| as_u64(value, "negative network bytes received"))
                    .transpose()?,
                terminal_bytes,
                active_bytes,
                other_bytes,
                transferred_bytes: terminal_bytes
                    .checked_add(active_bytes)
                    .and_then(|value| value.checked_add(other_bytes))
                    .ok_or_else(|| StoreError::invalid("transferred byte total overflow"))?,
                total_bytes,
                bytes_per_second: rate,
                transfer_active_millis: as_u64(
                    transfer_active_millis,
                    "negative task transfer active milliseconds",
                )?,
                active_item,
                active_progress: StoreTaskActiveProgressSummary {
                    without_progress: as_u64(without_progress, "negative active item count")?,
                    transferring: as_u64(transferring, "negative active item count")?,
                    idle: as_u64(idle, "negative active item count")?,
                    waiting: as_u64(waiting, "negative active item count")?,
                },
            },
        );
    }
    Ok(telemetry)
}

/// Review 2026-10-17, L4: the distinct stable failure codes of each task's
/// failed items, for the task card's retry hint. Codes only (validated
/// `[A-Z0-9_]{1,128}`); a missing or invalid code is `UNKNOWN_FAILURE_CODE`;
/// at most eight entries, the eighth being `MORE_FAILURE_CODES` when more
/// exist. `local` holds when every such failure's diagnostic tail declares
/// `source=local`. JSON is inspected only inside `json_valid` branches.
///
/// Review 2026-10-17 round 3, P2-3: for the NISAR subset tasks in `?2` an
/// item's current failure is the failure recorded on its latest `mark_failed`
/// event when that event carries one (see `latest_failure_event_failure`):
/// an unretained budget-migration rejection leaves the prior result row in
/// place, and only its event names the local refusal. Other tasks never record
/// a failure on that event, so their events are not read.
const TASK_FAILURE_CODES_SQL: &str = r#"
    WITH included(task_id) AS (SELECT value FROM json_each(?1)),
         event_scoped(task_id) AS (SELECT value FROM json_each(?2)),
         failed AS (
           SELECT r.task_id, member.value AS item_id
           FROM included i
           JOIN task_runtime r ON r.task_id = i.task_id
           JOIN json_each(r.state_json, '$.failed_item_ids') member
           WHERE member.type = 'text'
         ),
         latest_failed_event AS (
           SELECT e.task_id, e.item_id, MAX(e.sequence) AS sequence
           FROM included s
           JOIN task_events e ON e.task_id = s.task_id
           WHERE e.event_type = 'mark_failed' AND e.item_id IS NOT NULL
           GROUP BY e.task_id, e.item_id
         ),
         current_failure AS (
           SELECT f.task_id,
                  COALESCE(
                    CASE WHEN json_valid(failure_event.error_json) THEN
                      CASE WHEN json_extract(failure_event.error_json, '$.code') = 'SCHEDULER_GENERATION_UNCERTAIN'
                        THEN json_object('code', 'SCHEDULER_GENERATION_UNCERTAIN',
                                         'reason_zh', '无法确认取消/重试代际，需重新检查任务记录',
                                         'retryable', json('false'))
                        WHEN f.task_id IN (SELECT task_id FROM event_scoped)
                             AND json_type(failure_event.error_json, '$.code') = 'text'
                        THEN failure_event.error_json END
                    END,
                    CASE WHEN json_valid(d.result_json) THEN
                      CASE WHEN json_type(d.result_json, '$.failure') = 'object'
                        THEN json_extract(d.result_json, '$.failure') END
                    END
                  ) AS failure_json
           FROM failed f
           LEFT JOIN download_results d ON d.task_id = f.task_id AND d.item_id = f.item_id
           LEFT JOIN latest_failed_event latest
             ON latest.task_id = f.task_id AND latest.item_id = f.item_id
           LEFT JOIN task_events failure_event
             ON failure_event.task_id = latest.task_id
            AND failure_event.sequence = latest.sequence
         )
    SELECT task_id,
           CASE WHEN json_type(failure_json, '$.code') = 'text'
             THEN json_extract(failure_json, '$.code') END,
           CASE WHEN json_type(failure_json, '$.reason_zh') = 'text'
             THEN instr(json_extract(failure_json, '$.reason_zh'), 'source=local') > 0
             ELSE 0 END,
           CASE WHEN json_type(failure_json, '$.reason_zh') = 'text'
             THEN instr(json_extract(failure_json, '$.reason_zh'), 'source=tls_certificate') > 0
             ELSE 0 END,
           CASE WHEN json_type(failure_json, '$.reason_zh') = 'text'
             THEN instr(json_extract(failure_json, '$.reason_zh'), 'source=network') > 0
             ELSE 0 END
    FROM current_failure
"#;

const MAX_TASK_FAILURE_CODES: usize = 8;
const UNKNOWN_TASK_FAILURE_CODE: &str = "UNKNOWN_FAILURE_CODE";
const MORE_TASK_FAILURE_CODES: &str = "MORE_FAILURE_CODES";

fn valid_task_failure_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 128
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn load_task_failure_codes(
    connection: &Connection,
    task_ids: &[String],
    event_scoped_task_ids: &[String],
) -> Result<HashMap<String, Vec<StoreTaskFailureCode>>, StoreError> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let task_ids_json = serde_json::to_string(task_ids).map_err(StoreError::Corrupt)?;
    let event_scoped_json =
        serde_json::to_string(event_scoped_task_ids).map_err(StoreError::Corrupt)?;
    let mut statement = connection.prepare(TASK_FAILURE_CODES_SQL)?;
    let rows = statement.query_map(params![task_ids_json, event_scoped_json], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, bool>(2)?,
            row.get::<_, bool>(3)?,
            row.get::<_, bool>(4)?,
        ))
    })?;
    let mut codes_by_task: HashMap<
        String,
        std::collections::BTreeMap<String, TaskFailureCodeFacts>,
    > = HashMap::new();
    for row in rows {
        let (task_id, code, local, tls_certificate, network) = row?;
        let (code, facts) = match code.filter(|code| valid_task_failure_code(code)) {
            Some(code) => (
                code,
                TaskFailureCodeFacts {
                    local,
                    tls_certificate,
                    network,
                },
            ),
            None => (
                UNKNOWN_TASK_FAILURE_CODE.to_owned(),
                TaskFailureCodeFacts {
                    local: false,
                    tls_certificate: false,
                    network: false,
                },
            ),
        };
        codes_by_task
            .entry(task_id)
            .or_default()
            .entry(code)
            .and_modify(|all| {
                all.local &= facts.local;
                all.tls_certificate &= facts.tls_certificate;
                all.network &= facts.network;
            })
            .or_insert(facts);
    }
    Ok(codes_by_task
        .into_iter()
        .map(|(task_id, codes)| (task_id, bounded_task_failure_codes(codes)))
        .collect())
}

/// Sorted codes, at most eight entries; the eighth is `MORE_FAILURE_CODES`
/// when more distinct codes exist, so a consumer never mistakes a cut list for
/// a complete one.
fn bounded_task_failure_codes(
    codes: std::collections::BTreeMap<String, TaskFailureCodeFacts>,
) -> Vec<StoreTaskFailureCode> {
    let overflow = codes.len() > MAX_TASK_FAILURE_CODES;
    let kept = if overflow {
        MAX_TASK_FAILURE_CODES - 1
    } else {
        MAX_TASK_FAILURE_CODES
    };
    let mut failure_codes = codes
        .into_iter()
        .take(kept)
        .map(|(code, facts)| StoreTaskFailureCode {
            code,
            local: facts.local,
            tls_certificate: facts.tls_certificate,
            network: facts.network,
        })
        .collect::<Vec<_>>();
    if overflow {
        failure_codes.push(StoreTaskFailureCode {
            code: MORE_TASK_FAILURE_CODES.to_owned(),
            local: false,
            tls_certificate: false,
            network: false,
        });
    }
    failure_codes
}

#[cfg(test)]
mod failure_code_tests {
    use super::*;

    #[test]
    fn task_failure_codes_are_validated_and_bounded_with_an_overflow_marker() {
        assert!(valid_task_failure_code(
            "NISAR_SUBSET_V2_BUDGET_UPGRADE_REFUSED"
        ));
        let too_long = "A".repeat(129);
        for invalid in ["", "old-provider-error", "lower_case", too_long.as_str()] {
            assert!(!valid_task_failure_code(invalid), "{invalid}");
        }
        let few = (0..3)
            .map(|index| {
                (
                    format!("CODE_{index}"),
                    TaskFailureCodeFacts {
                        local: index == 1,
                        tls_certificate: index == 2,
                        network: index == 0,
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            bounded_task_failure_codes(few),
            vec![
                StoreTaskFailureCode {
                    code: "CODE_0".into(),
                    local: false,
                    tls_certificate: false,
                    network: true
                },
                StoreTaskFailureCode {
                    code: "CODE_1".into(),
                    local: true,
                    tls_certificate: false,
                    network: false
                },
                StoreTaskFailureCode {
                    code: "CODE_2".into(),
                    local: false,
                    tls_certificate: true,
                    network: false
                },
            ]
        );
        let many = (0..9)
            .map(|index| {
                (
                    format!("CODE_{index}"),
                    TaskFailureCodeFacts {
                        local: true,
                        tls_certificate: false,
                        network: false,
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let bounded = bounded_task_failure_codes(many);
        assert_eq!(bounded.len(), MAX_TASK_FAILURE_CODES);
        assert_eq!(bounded[6].code, "CODE_6");
        assert_eq!(
            bounded[7],
            StoreTaskFailureCode {
                code: MORE_TASK_FAILURE_CODES.into(),
                local: false,
                tls_certificate: false,
                network: false
            }
        );
        // The flag is left out of the serialized form when false.
        let serialized = serde_json::to_value(&bounded[0]).unwrap();
        assert_eq!(
            serialized,
            serde_json::json!({"code": "CODE_0", "local": true})
        );
        let tls = StoreTaskFailureCode {
            code: "CODE_X".into(),
            local: false,
            tls_certificate: true,
            network: false,
        };
        assert_eq!(
            serde_json::to_value(&tls).unwrap(),
            serde_json::json!({"code": "CODE_X", "local": false, "tls_certificate": true})
        );
        let network = StoreTaskFailureCode {
            code: "CODE_Y".into(),
            local: false,
            tls_certificate: false,
            network: true,
        };
        assert_eq!(
            serde_json::to_value(&network).unwrap(),
            serde_json::json!({"code": "CODE_Y", "local": false, "network": true})
        );
    }
}

const NISAR_SUBSET_RECOVERY_TELEMETRY_SQL: &str = r#"
    WITH included(task_id) AS (SELECT value FROM json_each(?1)),
         selected AS (
           SELECT r.task_id, member.value AS item_id
           FROM included i
           JOIN task_runtime r ON r.task_id = i.task_id
           JOIN json_each(r.state_json, '$.selected_item_ids') member
           WHERE member.type = 'text'
         )
    SELECT s.task_id, s.item_id, a.lineage_key_sha256,
           a.record_sha256, a.canonical_json
    FROM selected s
    LEFT JOIN nisar_subset_execution_anchors_v2 a
      ON a.task_id = s.task_id
     AND a.item_id = s.item_id
     AND a.anchor_kind = ?2
    ORDER BY s.task_id, s.item_id
"#;

fn load_nisar_subset_recovery_telemetries(
    connection: &Connection,
    task_ids: &[String],
) -> Result<HashMap<String, StoreNisarSubsetRecoveryTelemetrySummary>, StoreError> {
    if task_ids.is_empty() {
        return Ok(HashMap::new());
    }
    #[derive(Default)]
    struct Aggregate {
        seen: usize,
        complete: bool,
        reusable_verified_bytes: u64,
        reused_verified_bytes: u64,
    }

    let mut aggregates = task_ids
        .iter()
        .map(|task_id| {
            (
                task_id.clone(),
                Aggregate {
                    complete: true,
                    ..Aggregate::default()
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let task_ids_json = serde_json::to_string(task_ids).map_err(StoreError::Corrupt)?;
    let mut statement = connection.prepare(NISAR_SUBSET_RECOVERY_TELEMETRY_SQL)?;
    let rows = statement.query_map(
        params![
            task_ids_json,
            crate::repository::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    )?;
    for row in rows {
        let (task_id, _item_id, lineage_key_sha256, record_sha256, canonical_json) = row?;
        let aggregate = aggregates
            .get_mut(&task_id)
            .ok_or(StoreError::NisarSubsetExecutionAnchorInvalid)?;
        aggregate.seen = aggregate
            .seen
            .checked_add(1)
            .ok_or(StoreError::NisarSubsetExecutionAnchorInvalid)?;
        let telemetry = match (lineage_key_sha256, record_sha256, canonical_json) {
            (None, None, None) => {
                aggregate.complete = false;
                continue;
            }
            (Some(lineage), Some(record), Some(canonical)) => {
                parse_nisar_subset_recovery_telemetry(&lineage, &record, &canonical)?
            }
            _ => return Err(StoreError::NisarSubsetExecutionAnchorInvalid),
        };
        let Some(telemetry) = telemetry else {
            aggregate.complete = false;
            continue;
        };
        aggregate.reusable_verified_bytes = aggregate
            .reusable_verified_bytes
            .checked_add(telemetry.reusable_verified_bytes)
            .ok_or(StoreError::NisarSubsetExecutionAnchorInvalid)?;
        aggregate.reused_verified_bytes = aggregate
            .reused_verified_bytes
            .checked_add(telemetry.reused_verified_bytes)
            .ok_or(StoreError::NisarSubsetExecutionAnchorInvalid)?;
    }

    Ok(aggregates
        .into_iter()
        .filter_map(|(task_id, aggregate)| {
            (aggregate.complete && aggregate.seen > 0).then_some((
                task_id,
                StoreNisarSubsetRecoveryTelemetrySummary {
                    reusable_verified_bytes: aggregate.reusable_verified_bytes,
                    reused_verified_bytes: aggregate.reused_verified_bytes,
                },
            ))
        })
        .collect())
}

pub(crate) fn validate_nisar_science_anchor_canonical_v2(
    lineage_key_sha256: &str,
    record_sha256: &str,
    canonical_json: &str,
) -> Result<(), StoreError> {
    parse_nisar_subset_recovery_telemetry(lineage_key_sha256, record_sha256, canonical_json)
        .map(|_| ())
}

fn parse_nisar_subset_recovery_telemetry(
    row_lineage_key_sha256: &str,
    row_record_sha256: &str,
    canonical_json: &str,
) -> Result<Option<StoreNisarSubsetRecoveryTelemetrySummary>, StoreError> {
    if canonical_json.is_empty()
        || canonical_json.len() > NISAR_SUBSET_SCIENCE_ANCHOR_MAX_CANONICAL_BYTES
    {
        return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
    }
    let record: StoreNisarSubsetScienceCheckpointAnchorRecordV2 =
        serde_json::from_str(canonical_json)
            .map_err(|_| StoreError::NisarSubsetExecutionAnchorInvalid)?;
    if serde_json::to_string(&record).map_err(|_| StoreError::NisarSubsetExecutionAnchorInvalid)?
        != canonical_json
        || record.lineage_key_sha256 != row_lineage_key_sha256
        || record.record_sha256 != row_record_sha256
    {
        return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
    }
    validate_nisar_subset_science_anchor_record(&record)?;
    // Valid records written before recovery telemetry was introduced remain
    // readable and deliberately report unknown. They still pass the same
    // strict schema, canonical serialization, row binding, legacy digest, and
    // checkpoint/pending-transition invariants before reaching this branch.
    let Some(telemetry) = record.committed_recovery_telemetry else {
        return Ok(None);
    };
    Ok(Some(StoreNisarSubsetRecoveryTelemetrySummary {
        reusable_verified_bytes: telemetry.reusable_verified_bytes,
        reused_verified_bytes: telemetry.reused_verified_bytes,
    }))
}

fn validate_nisar_subset_science_anchor_record(
    record: &StoreNisarSubsetScienceCheckpointAnchorRecordV2,
) -> Result<(), StoreError> {
    let expected_record_sha256 = match &record.committed_recovery_telemetry {
        Some(telemetry) => nisar_subset_digest(&(
            NISAR_SUBSET_RECOVERY_TELEMETRY_DIGEST_CONTRACT_V1,
            &record.lineage_key_sha256,
            &record.execution_binding_sha256,
            record.generation,
            &record.committed_checkpoint_sha256,
            telemetry,
            &record.pending_transition,
        ))?,
        None => nisar_subset_digest(&(
            &record.lineage_key_sha256,
            &record.execution_binding_sha256,
            record.generation,
            &record.committed_checkpoint_sha256,
            &record.pending_transition,
        ))?,
    };
    if !valid_lowercase_sha256(&record.lineage_key_sha256)
        || !valid_lowercase_sha256(&record.execution_binding_sha256)
        || record
            .committed_checkpoint_sha256
            .as_deref()
            .is_some_and(|digest| !valid_lowercase_sha256(digest))
        || (record.pending_transition.is_none() && record.committed_checkpoint_sha256.is_none())
        || !valid_lowercase_sha256(&record.record_sha256)
        || expected_record_sha256 != record.record_sha256
    {
        return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
    }
    if let Some(telemetry) = &record.committed_recovery_telemetry {
        let committed = record
            .committed_checkpoint_sha256
            .as_deref()
            .ok_or(StoreError::NisarSubsetExecutionAnchorInvalid)?;
        if telemetry.contract != NISAR_SUBSET_RECOVERY_TELEMETRY_CONTRACT_V1
            || !valid_lowercase_sha256(&telemetry.checkpoint_sha256)
            || telemetry.checkpoint_sha256 != committed
            || telemetry.reused_verified_bytes > telemetry.reusable_verified_bytes
        {
            return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
        }
    }
    if let Some(pending) = &record.pending_transition {
        validate_nisar_subset_pending_transition(pending)?;
        let predecessor_matches = match record.committed_checkpoint_sha256.as_deref() {
            Some(committed) => {
                pending.predecessor_checkpoint_sha256.as_deref() == Some(committed)
                    && pending.predecessor_generation == Some(record.generation)
                    && record.generation.checked_add(1) == Some(pending.checkpoint_generation)
            }
            None => {
                record.generation == 0
                    && pending.predecessor_checkpoint_sha256.is_none()
                    && pending.predecessor_generation.is_none()
                    && pending.checkpoint_generation == 0
            }
        };
        if !predecessor_matches {
            return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
        }
    }
    Ok(())
}

fn validate_nisar_subset_pending_transition(
    pending: &StoreNisarSubsetSciencePendingAnchorTransitionV2,
) -> Result<(), StoreError> {
    if pending
        .predecessor_checkpoint_sha256
        .as_deref()
        .is_some_and(|digest| !valid_lowercase_sha256(digest))
        || pending.predecessor_checkpoint_sha256.is_some()
            != pending.predecessor_generation.is_some()
        || !valid_lowercase_sha256(&pending.checkpoint_sha256)
        || pending.checkpoint_canonical_json.is_empty()
        || pending.checkpoint_canonical_json.len()
            > NISAR_SUBSET_SCIENCE_CHECKPOINT_MAX_CANONICAL_BYTES
        || !valid_lowercase_sha256(&pending.transition_sha256)
        || nisar_subset_digest(&(
            &pending.predecessor_checkpoint_sha256,
            pending.predecessor_generation,
            pending.checkpoint_generation,
            &pending.checkpoint_sha256,
            &pending.checkpoint_canonical_json,
        ))? != pending.transition_sha256
    {
        return Err(StoreError::NisarSubsetExecutionAnchorInvalid);
    }
    Ok(())
}

fn nisar_subset_digest(value: &impl Serialize) -> Result<String, StoreError> {
    let bytes =
        serde_json::to_vec(value).map_err(|_| StoreError::NisarSubsetExecutionAnchorInvalid)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn valid_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn load_last_known_totals(
    connection: &Connection,
    task_id: &str,
) -> Result<HashMap<String, Option<u64>>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT item_id, last_known_total_bytes FROM download_results WHERE task_id = ?1",
    )?;
    let rows = statement.query_map([task_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
    })?;
    let mut totals = HashMap::new();
    for row in rows {
        let (item_id, total) = row?;
        let total = total
            .map(|value| {
                u64::try_from(value)
                    .map_err(|_| StoreError::invalid("negative last known total bytes"))
            })
            .transpose()?;
        if totals.insert(item_id, total).is_some() {
            return Err(StoreError::invalid("duplicate terminal download result"));
        }
    }
    Ok(totals)
}

fn unavailable(
    row: &TaskRow,
    status: Option<TaskLifecycleStatus>,
    reason_code: LegacyTaskRestoreReasonCode,
) -> TaskProjection {
    TaskProjection::Degraded(LegacyTaskRestoreWarning {
        task_id: row.task_id.clone(),
        provider_id: row.provider_id.clone(),
        created_at: row.created_at.clone(),
        status,
        reason_code,
        compatibility_status: LegacyTaskCompatibilityStatus::Unavailable,
    })
}

fn validate_runtime_ids(snapshot: &CompactTaskSnapshotV2, runtime: &TaskRuntimeSnapshot) -> bool {
    let item_ids: HashSet<&str> = snapshot
        .selected_products
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    item_ids.len() == snapshot.selected_products.len()
        && snapshot
            .initial_selected_item_ids
            .iter()
            .all(|item_id| item_ids.contains(item_id.as_str()))
        && runtime
            .selected_item_ids
            .iter()
            .all(|item_id| item_ids.contains(item_id.as_str()))
        && runtime
            .checked_item_ids
            .iter()
            .all(|item_id| item_ids.contains(item_id.as_str()))
}

fn has_missing_legacy_display_fields(snapshot: &CompactTaskSnapshotV2) -> bool {
    snapshot.selected_products.iter().any(|item| {
        ["dataset", "product_family", "provider"]
            .into_iter()
            .any(|field| item.metadata.get(field).is_some_and(Value::is_null))
    })
}

fn exact_sentinel1_slc_legacy_predicate(row: &TaskRow, snapshot: &CompactTaskSnapshotV2) -> bool {
    row.provider_id == "asf.sentinel1"
        && row.runtime_json.as_deref().is_some_and(|runtime_json| {
            serde_json::from_str::<TaskRuntimeSnapshot>(runtime_json)
                .is_ok_and(|runtime| validate_runtime_ids(snapshot, &runtime))
        })
        && snapshot.query.get("dataset").and_then(Value::as_str) == Some("SENTINEL-1")
        && snapshot.query.get("processing_levels")
            == Some(&Value::Array(vec![Value::String("SLC".to_owned())]))
        && !snapshot.selected_products.is_empty()
        && snapshot.selected_products.iter().all(|item| {
            item.provider_id == "asf.sentinel1"
                && item.metadata.iter().all(|(field, value)| {
                    !value.is_null()
                        || matches!(field.as_str(), "dataset" | "product_family" | "provider")
                })
                && item
                    .metadata
                    .get("processing_level")
                    .and_then(Value::as_str)
                    == Some("SLC")
                && item.metadata.get("product_type").and_then(Value::as_str) == Some("SLC")
                && optional_exact_or_null(&item.metadata, "dataset", "SENTINEL-1")
                && optional_exact_or_null(&item.metadata, "product_family", "SLC")
                && optional_exact_or_null(&item.metadata, "provider", "ASF")
        })
}

fn optional_exact_or_null(
    metadata: &serde_json::Map<String, Value>,
    field: &str,
    exact: &str,
) -> bool {
    metadata
        .get(field)
        .is_none_or(|value| value.is_null() || value.as_str() == Some(exact))
}

fn parse_lifecycle_status(value: &str) -> Option<TaskLifecycleStatus> {
    match value {
        "queued" => Some(TaskLifecycleStatus::Queued),
        "running" => Some(TaskLifecycleStatus::Running),
        "paused" => Some(TaskLifecycleStatus::Paused),
        "completed" => Some(TaskLifecycleStatus::Completed),
        "failed" => Some(TaskLifecycleStatus::Failed),
        "cancelled" => Some(TaskLifecycleStatus::Cancelled),
        "interrupted" => Some(TaskLifecycleStatus::Interrupted),
        _ => None,
    }
}

pub(crate) fn lifecycle_status_name(status: &TaskLifecycleStatus) -> &'static str {
    match status {
        TaskLifecycleStatus::Queued => "queued",
        TaskLifecycleStatus::Running => "running",
        TaskLifecycleStatus::Paused => "paused",
        TaskLifecycleStatus::Completed => "completed",
        TaskLifecycleStatus::Failed => "failed",
        TaskLifecycleStatus::Cancelled => "cancelled",
        TaskLifecycleStatus::Interrupted => "interrupted",
    }
}

fn load_workbench_summary(
    connection: &Connection,
) -> Result<Option<StoreWorkbenchSummary>, StoreError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        i64,
        i64,
        i64,
        Option<String>,
        Option<i64>,
        String,
    )> = connection
        .query_row(
            "SELECT b.batch_id, b.provider_id, b.created_at, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_extract(b.snapshot_json, '$.aoi_name') END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_extract(b.snapshot_json, '$.query.provider') END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN COALESCE(json_extract(b.snapshot_json, '$.query.dataset'), json_extract(b.snapshot_json, '$.query.logical_dataset')) END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_extract(b.snapshot_json, '$.query.start') END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_extract(b.snapshot_json, '$.query.end') END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN COALESCE(json_extract(b.snapshot_json, '$.query.max_results'), json_extract(b.snapshot_json, '$.query.page_size')) END, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_array_length(b.snapshot_json, '$.candidates') ELSE -1 END, \
                    CASE WHEN json_valid(w.state_json) THEN json_array_length(w.state_json, '$.checked_item_ids') ELSE -1 END, \
                    w.revision, \
                    CASE WHEN json_valid(b.snapshot_json) THEN json_extract(b.snapshot_json, '$.search_stats') END, \
                    c.matched_count, \
                    w.state_json \
             FROM workbench_state w JOIN search_batches b ON b.batch_id = w.batch_id \
             LEFT JOIN sar_search_batch_count c ON c.batch_id = b.batch_id \
             WHERE w.slot = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?,
                    row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?,
                    row.get(12)?, row.get(13)?, row.get(14)?,
                ))
            },
        )
        .optional()?;
    let Some((
        batch_id,
        provider_id,
        created_at,
        aoi_name,
        provider,
        dataset,
        start,
        end,
        target_count,
        candidate_count,
        checked_count,
        revision,
        search_stats_json,
        background_matched_count,
        state_json,
    )) = row
    else {
        return Ok(None);
    };
    let state: SearchWorkbenchState =
        serde_json::from_str(&state_json).map_err(StoreError::Corrupt)?;
    state.validate().map_err(StoreError::invalid)?;
    if state.batch_id != batch_id
        || u64::try_from(revision).ok() != Some(state.revision)
        || candidate_count < 0
        || checked_count < 0
        || usize::try_from(checked_count).ok() != Some(state.checked_item_ids.len())
    {
        return Err(StoreError::invalid("workbench summary envelope differs"));
    }
    let candidate_count = usize::try_from(candidate_count)
        .map_err(|_| StoreError::invalid("workbench candidate count is invalid"))?;
    let target_count = match target_count {
        Some(value) if (1..=10_000).contains(&value) => Some(
            u64::try_from(value)
                .map_err(|_| StoreError::invalid("workbench search target is invalid"))?,
        ),
        Some(_) => return Err(StoreError::invalid("workbench search target is invalid")),
        None => None,
    };
    let search_stats = match search_stats_json {
        Some(value) => {
            let mut stats: FrozenSearchStats =
                serde_json::from_str(&value).map_err(StoreError::Corrupt)?;
            if let Some(matched_count) = background_matched_count {
                let matched_count = u64::try_from(matched_count)
                    .map_err(|_| StoreError::invalid("SAR matched count is invalid"))?;
                stats.matched_count = Some(matched_count);
                stats.truncated = matched_count > stats.frozen_candidate_count
                    || stats.provider_returned_count > stats.frozen_candidate_count;
            }
            stats.validate().map_err(StoreError::invalid)?;
            if stats.frozen_candidate_count
                != u64::try_from(candidate_count)
                    .map_err(|_| StoreError::invalid("workbench candidate count is invalid"))?
            {
                return Err(StoreError::invalid(
                    "workbench frozen candidate count differs from Store candidates",
                ));
            }
            Some(StoreWorkbenchSearchStats {
                matched_count: stats.matched_count,
                provider_returned_count: stats.provider_returned_count,
                frozen_candidate_count: stats.frozen_candidate_count,
                truncated: stats.truncated,
                source_attempts: stats
                    .source_attempts
                    .iter()
                    .map(project_workbench_source_attempt)
                    .collect(),
                source: stats.source,
            })
        }
        None if background_matched_count.is_none() => None,
        None => return Err(StoreError::invalid("SAR count update lacks frozen stats")),
    };
    let current_page_item_count = match (
        state
            .page_index
            .checked_mul(state.page_size)
            .and_then(|offset| usize::try_from(offset).ok()),
        usize::try_from(state.page_size),
    ) {
        (Some(offset), Ok(page_size)) => candidate_count.saturating_sub(offset).min(page_size),
        _ => 0,
    };
    Ok(Some(StoreWorkbenchSummary {
        batch_id,
        provider_id,
        created_at,
        aoi_name,
        query: StoreWorkbenchQuerySummary {
            provider,
            dataset,
            start,
            end,
            target_count,
        },
        search_stats,
        candidate_count,
        checked_count: usize::try_from(checked_count)
            .map_err(|_| StoreError::invalid("workbench checked count is invalid"))?,
        current_page_item_count,
        page_index: state.page_index,
        page_size: state.page_size,
        revision: state.revision,
        deferred: true,
        reason_code: Some(StoreWorkbenchSummaryReasonCode::CandidatePayloadDeferred),
    }))
}

#[cfg(test)]
mod telemetry_query_tests {
    use super::*;
    use rusqlite::StatementStatus;
    use serde_json::json;

    fn insert_recovery_query_task(store: &TaskStore, task_id: &str, item_ids: &[&str]) {
        let batch_id = format!("{task_id}-batch");
        let runtime = json!({"selected_item_ids": item_ids}).to_string();
        store
            .connection
            .execute(
                "INSERT INTO search_batches VALUES (?1, 'asf.nasa.earthdata', '2026-10-15T00:00:00Z', '{}')",
                [&batch_id],
            )
            .unwrap();
        store.connection.execute(
            "INSERT INTO task_snapshots VALUES (?1, ?2, 'asf.nasa.earthdata', '2026-10-15T00:00:00Z', 'test-output', 'direct', '{}')",
            params![task_id, batch_id],
        ).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO task_runtime VALUES (?1, 'running', 0, '2026-10-15T00:00:00Z', ?2)",
                params![task_id, runtime],
            )
            .unwrap();
    }

    fn recovery_anchor(
        reusable_verified_bytes: u64,
        reused_verified_bytes: u64,
    ) -> (String, String, String) {
        let lineage_key_sha256 = "a".repeat(64);
        let execution_binding_sha256 = "b".repeat(64);
        let committed_checkpoint_sha256 = Some("c".repeat(64));
        let telemetry = StoreNisarSubsetCommittedRecoveryTelemetryV1 {
            contract: NISAR_SUBSET_RECOVERY_TELEMETRY_CONTRACT_V1.to_owned(),
            checkpoint_sha256: "c".repeat(64),
            reusable_verified_bytes,
            reused_verified_bytes,
        };
        let pending_transition: Option<StoreNisarSubsetSciencePendingAnchorTransitionV2> = None;
        let record_sha256 = nisar_subset_digest(&(
            NISAR_SUBSET_RECOVERY_TELEMETRY_DIGEST_CONTRACT_V1,
            &lineage_key_sha256,
            &execution_binding_sha256,
            7_u64,
            &committed_checkpoint_sha256,
            &telemetry,
            &pending_transition,
        ))
        .unwrap();
        let record = StoreNisarSubsetScienceCheckpointAnchorRecordV2 {
            lineage_key_sha256: lineage_key_sha256.clone(),
            execution_binding_sha256,
            generation: 7,
            committed_checkpoint_sha256,
            committed_recovery_telemetry: Some(telemetry),
            pending_transition,
            record_sha256: record_sha256.clone(),
        };
        (
            lineage_key_sha256,
            record_sha256,
            serde_json::to_string(&record).unwrap(),
        )
    }

    fn legacy_recovery_anchor() -> (String, String, String) {
        let lineage_key_sha256 = "d".repeat(64);
        let execution_binding_sha256 = "e".repeat(64);
        let committed_checkpoint_sha256 = Some("f".repeat(64));
        let pending_transition: Option<StoreNisarSubsetSciencePendingAnchorTransitionV2> = None;
        let record_sha256 = nisar_subset_digest(&(
            &lineage_key_sha256,
            &execution_binding_sha256,
            7_u64,
            &committed_checkpoint_sha256,
            &pending_transition,
        ))
        .unwrap();
        let record = StoreNisarSubsetScienceCheckpointAnchorRecordV2 {
            lineage_key_sha256: lineage_key_sha256.clone(),
            execution_binding_sha256,
            generation: 7,
            committed_checkpoint_sha256,
            committed_recovery_telemetry: None,
            pending_transition,
            record_sha256: record_sha256.clone(),
        };
        (
            lineage_key_sha256,
            record_sha256,
            serde_json::to_string(&record).unwrap(),
        )
    }

    fn insert_recovery_anchor(
        store: &TaskStore,
        task_id: &str,
        item_id: &str,
        anchor: &(String, String, String),
    ) {
        store
            .connection
            .execute(
                "INSERT INTO nisar_subset_execution_anchors_v2(\
                 task_id,item_id,anchor_kind,lineage_key_sha256,record_sha256,canonical_json\
             ) VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    task_id,
                    item_id,
                    crate::repository::NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
                    anchor.0,
                    anchor.1,
                    anchor.2,
                ],
            )
            .unwrap();
    }

    // Run the production SQL against the real migrated schema. VM work, unlike
    // wall-clock timing, is not distorted by build load or debug/release mode.
    fn telemetry_vm_steps(item_count: usize) -> i32 {
        let store = TaskStore::open(":memory:").expect("in-memory Store");
        let ids = (0..item_count)
            .map(|index| format!("ITEM-{index:04}"))
            .collect::<Vec<_>>();
        let products = ids
            .iter()
            .map(|id| {
                json!({
                    "item_id": id,
                    "payload": {"file_size_remote": 1_000_000, "padding": "x".repeat(512)}
                })
            })
            .collect::<Vec<_>>();
        let snapshot = json!({"selected_products": products}).to_string();
        let runtime = json!({
            "selected_item_ids": ids,
            "active_item_ids": [], "paused_item_ids": ids,
            "queued_item_ids": [], "completed_item_ids": [],
            "failed_item_ids": [], "skipped_item_ids": [],
            "cancelled_item_ids": [], "task_paused": true
        })
        .to_string();
        // Only query inputs are needed here; public CoreRestore validation is
        // exercised by ua0_core_restore and task_telemetry integration tests.
        store.connection.execute(
            "INSERT INTO search_batches VALUES ('batch', 'asf.sentinel1', '2026-09-25T00:00:00Z', '{}')",
            [],
        ).unwrap();
        store.connection.execute(
            "INSERT INTO task_snapshots VALUES ('task', 'batch', 'asf.sentinel1', '2026-09-25T00:00:00Z', 'test-output', 'direct', ?1)",
            [&snapshot],
        ).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO task_runtime VALUES ('task', 'paused', 0, '2026-09-25T00:00:00Z', ?1)",
                [&runtime],
            )
            .unwrap();
        let mut statement = store.connection.prepare(TASK_TELEMETRY_SQL).unwrap();
        let total: Option<i64> = statement
            .query_row(["[\"task\"]"], |row| row.get(4))
            .unwrap();
        assert_eq!(total, Some(item_count as i64 * 1_000_000));
        statement.get_status(StatementStatus::VmStep)
    }

    #[test]
    fn telemetry_query_vm_work_scales_below_quadratic() {
        let small = telemetry_vm_steps(100);
        let large = telemetry_vm_steps(200);
        eprintln!("telemetry VM instructions: n=100 {small}, n=200 {large}");
        assert!(small > 0);
        assert!(
            large < small * 3,
            "doubling task items must not quadruple telemetry work: {small} -> {large}"
        );
    }

    #[test]
    fn nisar_recovery_query_keeps_missing_legacy_and_mixed_tasks_unknown() {
        let store = TaskStore::open(":memory:").unwrap();
        insert_recovery_query_task(&store, "missing", &["item-a"]);
        insert_recovery_query_task(&store, "legacy", &["item-a"]);
        insert_recovery_query_task(&store, "mixed", &["item-a", "item-b"]);
        let legacy = legacy_recovery_anchor();
        insert_recovery_anchor(&store, "legacy", "item-a", &legacy);
        insert_recovery_anchor(&store, "mixed", "item-a", &recovery_anchor(20, 8));
        insert_recovery_anchor(&store, "mixed", "item-b", &legacy);

        let telemetry = load_nisar_subset_recovery_telemetries(
            &store.connection,
            &[
                "missing".to_owned(),
                "legacy".to_owned(),
                "mixed".to_owned(),
            ],
        )
        .unwrap();
        assert!(telemetry.is_empty());
    }

    #[test]
    fn nisar_recovery_query_sums_every_selected_item_and_rejects_overflow() {
        let store = TaskStore::open(":memory:").unwrap();
        insert_recovery_query_task(&store, "aggregate", &["item-a", "item-b"]);
        insert_recovery_anchor(&store, "aggregate", "item-a", &recovery_anchor(20, 8));
        insert_recovery_anchor(&store, "aggregate", "item-b", &recovery_anchor(30, 9));
        let telemetry =
            load_nisar_subset_recovery_telemetries(&store.connection, &["aggregate".to_owned()])
                .unwrap();
        assert_eq!(
            telemetry.get("aggregate"),
            Some(&StoreNisarSubsetRecoveryTelemetrySummary {
                reusable_verified_bytes: 50,
                reused_verified_bytes: 17,
            })
        );

        insert_recovery_query_task(&store, "overflow", &["item-a", "item-b"]);
        insert_recovery_anchor(&store, "overflow", "item-a", &recovery_anchor(u64::MAX, 0));
        insert_recovery_anchor(&store, "overflow", "item-b", &recovery_anchor(1, 0));
        assert!(matches!(
            load_nisar_subset_recovery_telemetries(&store.connection, &["overflow".to_owned()]),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));
    }

    #[test]
    fn nisar_recovery_projection_rejects_tampered_extended_records() {
        let (lineage, record_sha256, canonical) = recovery_anchor(20, 8);
        let mut wrong_digest: Value = serde_json::from_str(&canonical).unwrap();
        wrong_digest["committed_recovery_telemetry"]["reusable_verified_bytes"] = json!(21);
        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(
                &lineage,
                &record_sha256,
                &wrong_digest.to_string()
            ),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));

        let mut wrong_contract: StoreNisarSubsetScienceCheckpointAnchorRecordV2 =
            serde_json::from_str(&canonical).unwrap();
        wrong_contract
            .committed_recovery_telemetry
            .as_mut()
            .unwrap()
            .contract = "wrong".to_owned();
        wrong_contract.record_sha256 = nisar_subset_digest(&(
            NISAR_SUBSET_RECOVERY_TELEMETRY_DIGEST_CONTRACT_V1,
            &wrong_contract.lineage_key_sha256,
            &wrong_contract.execution_binding_sha256,
            wrong_contract.generation,
            &wrong_contract.committed_checkpoint_sha256,
            wrong_contract
                .committed_recovery_telemetry
                .as_ref()
                .unwrap(),
            &wrong_contract.pending_transition,
        ))
        .unwrap();
        let wrong_contract_sha256 = wrong_contract.record_sha256.clone();
        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(
                &lineage,
                &wrong_contract_sha256,
                &serde_json::to_string(&wrong_contract).unwrap()
            ),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));

        let mut wrong_binding: StoreNisarSubsetScienceCheckpointAnchorRecordV2 =
            serde_json::from_str(&canonical).unwrap();
        wrong_binding.execution_binding_sha256 = "B".repeat(64);
        wrong_binding.record_sha256 = nisar_subset_digest(&(
            NISAR_SUBSET_RECOVERY_TELEMETRY_DIGEST_CONTRACT_V1,
            &wrong_binding.lineage_key_sha256,
            &wrong_binding.execution_binding_sha256,
            wrong_binding.generation,
            &wrong_binding.committed_checkpoint_sha256,
            wrong_binding.committed_recovery_telemetry.as_ref().unwrap(),
            &wrong_binding.pending_transition,
        ))
        .unwrap();
        let wrong_binding_sha256 = wrong_binding.record_sha256.clone();
        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(
                &lineage,
                &wrong_binding_sha256,
                &serde_json::to_string(&wrong_binding).unwrap()
            ),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));

        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(&"f".repeat(64), &record_sha256, &canonical),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));

        let (lineage, record_sha256, impossible_reuse) = recovery_anchor(20, 21);
        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(&lineage, &record_sha256, &impossible_reuse),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));
    }

    #[test]
    fn nisar_recovery_projection_validates_legacy_records_before_reporting_unknown() {
        let (legacy_lineage, legacy_record_sha256, legacy_canonical) = legacy_recovery_anchor();
        assert_eq!(
            parse_nisar_subset_recovery_telemetry(
                &legacy_lineage,
                &legacy_record_sha256,
                &legacy_canonical,
            )
            .unwrap(),
            None
        );

        let (extended_lineage, extended_record_sha256, extended_canonical) = recovery_anchor(20, 8);
        let mut stripped_extended: Value = serde_json::from_str(&extended_canonical).unwrap();
        stripped_extended
            .as_object_mut()
            .unwrap()
            .remove("committed_recovery_telemetry");
        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(
                &extended_lineage,
                &extended_record_sha256,
                &stripped_extended.to_string(),
            ),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));

        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(
                &legacy_lineage,
                &"0".repeat(64),
                &legacy_canonical,
            ),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));
        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(
                &"0".repeat(64),
                &legacy_record_sha256,
                &legacy_canonical,
            ),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));

        let mut unknown_field: Value = serde_json::from_str(&legacy_canonical).unwrap();
        unknown_field["unknown"] = json!(true);
        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(
                &legacy_lineage,
                &legacy_record_sha256,
                &unknown_field.to_string(),
            ),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));
        assert!(matches!(
            parse_nisar_subset_recovery_telemetry(
                &legacy_lineage,
                &legacy_record_sha256,
                &format!(" {legacy_canonical}"),
            ),
            Err(StoreError::NisarSubsetExecutionAnchorInvalid)
        ));
    }
}
