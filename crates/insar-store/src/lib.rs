//! SQLite-backed persistence for frozen search batches and task snapshots.

mod administrative_boundary_cache;
mod agent_plans;
mod agent_runs;
mod asf_cache;
mod batch_candidates;
mod batch_dataset;
mod batch_page;
mod batch_progress;
mod cache_maintenance;
mod core_restore;
mod download;
mod error;
mod experiment_projects;
mod gacos_submission;
mod history;
mod loading_admission;
mod local_reuse;
mod nisar_output_wait;
mod nisar_subset_plan;
mod nisar_workspace_rebinding;
mod orbit_recheck;
mod repository;
mod runtime;
mod sar_cache;
mod schema;
mod snapshot_codec;
mod source_gate;
mod source_index;
mod source_receipts;
mod standalone_aoi;
mod task_failure_summary;
mod task_pause_reason;
mod task_telemetry;
mod view;

pub use administrative_boundary_cache::{
    StoredAdministrativeBoundary, ADMIN_BOUNDARY_CACHE_DEFAULT_TTL_SECONDS,
    ADMIN_BOUNDARY_CACHE_MAX_BYTES, ADMIN_BOUNDARY_CACHE_MAX_ENTRIES,
};
pub use agent_plans::{AgentApprovalRow, AgentPlanRow};
pub use agent_runs::{AgentRunFlags, AgentRunRow};
pub use asf_cache::{
    ASF_SEARCH_CACHE_DEFAULT_TTL_SECONDS, ASF_SEARCH_CACHE_MAX_BYTES, ASF_SEARCH_CACHE_MAX_ENTRIES,
};
pub use batch_candidates::{StoreBatchCandidatePage, StoreBatchSelection};
pub use batch_progress::{partial_search_stats, StoreBatchProgress, PROGRESSIVE_PUBLICATION_ROWS};
pub use cache_maintenance::{
    StoreCacheClearOutcome, StoreDisposableCacheStats, SEARCH_CACHE_DEFAULT_TTL_SECONDS,
    SEARCH_CACHE_MAX_BYTES, SEARCH_CACHE_MAX_ENTRIES,
};
pub use core_restore::{
    LegacyTaskCompatibilityStatus, LegacyTaskRestoreReasonCode, LegacyTaskRestoreWarning,
    StoreCoreRestore, StoreDemConversionSource, StoreDemConversionSourceRequest,
    StoreDemSourceIdentity, StoreReadiness, StoreTaskActiveProgressSummary, StoreTaskFailureCode,
    StoreTaskHistoryCursor, StoreTaskHistoryItem, StoreTaskHistoryPage,
    StoreTaskHistoryPageRequest, StoreTaskLocalScanSummary, StoreTaskOperationSummary,
    StoreTaskSummary, StoreWorkbenchPage, StoreWorkbenchPageItem, StoreWorkbenchPageRequest,
    StoreWorkbenchQuerySummary, StoreWorkbenchSummary, StoreWorkbenchSummaryReasonCode,
    DEFAULT_TASK_HISTORY_PAGE_SIZE, DEFAULT_WORKBENCH_PAGE_SIZE, MAX_TASK_HISTORY_PAGE_SIZE,
    MAX_WORKBENCH_PAGE_SIZE,
};
pub use error::{StoreError, TaskSnapshotRowEnvelopeIssue, V3SnapshotValidationIssue};
pub use experiment_projects::{
    ExperimentProjectItemRecord, ExperimentProjectRecord, ExperimentProjectStore,
    ExperimentProjectUpsertItem, DEFAULT_EXPERIMENT_PROJECT_ID, DEFAULT_EXPERIMENT_PROJECT_TITLE,
    MAX_EXPERIMENT_PROJECT_ITEMS,
};
pub use gacos_submission::{
    GacosSubmissionAttempt, GacosSubmissionAttemptIdentity, GacosSubmissionOutcome,
    GacosSubmissionOutcomeRequest, GacosSubmissionReserveRequest, GacosSubmissionState,
    GacosSubmissionSubmittingRequest, GACOS_MAX_ATTEMPT_HISTORY,
};
pub use history::{canonical_utc_identity_timestamp, CompletedDownloadDestination, StoredHistory};
pub use insar_core::{StoreBatchPage, StoreBatchPageRequest};
pub use loading_admission::PROGRESSIVE_ADMISSION_DELIVERY_KIND;
pub use local_reuse::{
    StoreFileIdentity, StoreLandedArtifact, StoreLandingEvidence, StoreLocalReuseBatch,
    StoreLocalReuseBatchItem, StoreLocalReuseDisposition, StoreLocalScanState,
    StoreTaskItemLocalReuse, StoreTaskLocalScan, STORE_LOCAL_REUSE_BATCH_LIMIT,
};
pub use nisar_output_wait::{StoreNisarOutputWaitEventV2Request, StoreNisarOutputWaitReasonV2};
pub use nisar_subset_plan::{
    embed_nisar_subset_plan_v2_envelope, embed_nisar_subset_prepared_intent_v2_envelope,
    parse_nisar_subset_plan_v2_envelope, parse_nisar_subset_prepared_intent_v2_envelope,
    NisarSubsetExecutionPlanEnvelopeV2, NisarSubsetPreparedIntentEnvelopeV2,
    NISAR_SUBSET_PLAN_V2_ENVELOPE_CONTRACT, NISAR_SUBSET_PLAN_V2_ENVELOPE_METADATA_KEY,
    NISAR_SUBSET_PLAN_V2_ENVELOPE_SCHEMA_VERSION,
    NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_CONTRACT,
    NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_METADATA_KEY,
    NISAR_SUBSET_PREPARED_INTENT_V2_ENVELOPE_SCHEMA_VERSION,
};
pub use nisar_workspace_rebinding::{
    StoreNisarWorkspaceRebindingEventV2Request, NISAR_WORKSPACE_REBINDING_EVENT_TYPE,
};
pub use orbit_recheck::{
    StoreCompanionPreferences, StoreOrbitRecheckRequest, StoreOrbitRecheckRow,
    StoreOrbitRecheckState,
};
pub use repository::nisar_budget_upgrade::{
    NisarBudgetUpgradeLimitsV2, StoreNisarBudgetUpgradeContextV2,
    StoreNisarBudgetUpgradeDownloadProgressV2, StoreNisarBudgetUpgradeDownloadResultV2,
    StoreNisarBudgetUpgradeEvidenceV2, StoreNisarBudgetUpgradePreReconciliationV2,
    StoredNisarBudgetUpgradeProofV2, StoredNisarBudgetUpgradeRefusalV2,
};
pub use repository::{
    StoreAttachNisarSubsetExecutionPlanV2Request,
    StoreCompareAndSwapNisarSubsetExecutionAnchorV2Request,
    StoreCreateHistoricalExplicitTaskRequest, StoreCreateNisarArchivedRecoveryV2Request,
    StoreFreezeNisarSubsetSourceSizeV2Request, StoreSearchBatchStagingRequest,
    StoredNisarArchivedRecoveryV2, StoredNisarSubsetExecutionAnchorV2, StoredTask, StoredWorkbench,
    DEFAULT_WORKSPACE_ID, NISAR_SUBSET_SCIENCE_CHECKPOINT_ANCHOR_V2_KIND,
    NISAR_SUBSET_WRITER_RECOVERY_ANCHOR_V2_KIND,
};
pub use runtime::{
    ReacquireEventStatus, SchedulerCancelledFinish, SchedulerPendingCancelledFinish,
    StoreAppendExplicitTaskItemsRequest, StoreTaskEventPage, StoreTaskEventPageItem,
    StoreTaskEventPageRequest, StoredTaskEvent, DEFAULT_TASK_EVENT_PAGE_SIZE,
    MAX_TASK_EVENT_PAGE_SIZE,
};
pub use sar_cache::StoreSarCacheDecision;
pub use schema::{StoreHealth, TaskStore, STORE_SCHEMA_VERSION};
pub use snapshot_codec::{
    decode_any_snapshot, decode_v2_only_snapshot, encode_task_snapshot,
    validate_task_snapshot_row_envelope, CompactTaskSnapshotV2, StoredTaskSnapshot,
    TaskSnapshotRowEnvelope,
};
pub use source_index::{
    is_replayable_optical_delivery_kind, StoreBatchRouteHeader, StoreRetryFallbackRequest,
    StoreSourceView, RETRY_FALLBACK_BATCH_PREFIX,
};
pub use source_receipts::{
    BurstAppendReceiptStatus, BurstSuccessorReceiptStatus, HistoricalSuccessorReceiptStatus,
    NisarAppendReceiptStatus, NisarArchivedRecoveryReceiptStatus, NisarSuccessorReceiptStatus,
    SourceAppendReceiptStatus, SourceMutationCommitStatus, SourceMutationQuery,
    SourceMutationStatus, SourceMutationTargetStatus, StoreBurstAppendIdentity,
    StoreBurstSuccessorIdentity, StoreNisarAppendIdentity, StoreNisarSuccessorIdentity,
};
pub use standalone_aoi::{StoreStandaloneAoi, StoreStandaloneAoiInput};
pub use task_failure_summary::StoreTaskFailureSummary;
pub use task_telemetry::{
    StoreTaskOutputDirectory, StoreTaskOutputDirectoryRequest, StoreTaskTransferTelemetry,
    StoreTaskTransferTelemetryUpdate,
};
