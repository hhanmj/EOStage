use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V3SnapshotValidationIssue {
    Structural,
    Graph,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskSnapshotRowEnvelopeIssue {
    TaskId,
    BatchId,
    ProviderId,
    CreatedAt,
    OutputRoot,
    Layout,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite 任务存储操作失败")]
    Database(#[source] rusqlite::Error),
    #[error("SQLite 任务存储目录创建失败")]
    Filesystem(#[source] std::io::Error),
    #[error("SQLite 任务存储不存在")]
    Missing,
    #[error("SQLite 任务存储为空文件")]
    Empty,
    #[error("任务存储版本 {found} 高于当前支持版本 {supported}")]
    NewerSchema { found: i64, supported: i64 },
    #[error("任务快照合同无效")]
    InvalidSnapshot { reason: String },
    #[error("V3 任务快照合同无效")]
    InvalidV3Snapshot { issue: V3SnapshotValidationIssue },
    #[error("任务快照行信封与内容不一致")]
    SnapshotRowEnvelopeMismatch { issue: TaskSnapshotRowEnvelopeIssue },
    #[error("V2 读取器拒绝 V3 任务快照")]
    V3SnapshotRejectedByV2Reader,
    #[error("任务快照持久化版本不受支持")]
    UnsupportedSnapshotVersion,
    #[error("任务存储记录已存在")]
    AlreadyExists,
    #[error("任务存储记录不存在")]
    NotFound,
    /// #22 B slice 3(c): a page of a loading batch that starts at or beyond
    /// the rows that have arrived. Retry once more rows have arrived.
    #[error("检索结果仍在加载，该页尚未到达（已到 {loaded_count} 条）")]
    PageNotYetLoaded { batch_id: String, loaded_count: u64 },
    /// #22 B slice 3(c): the batch is readable page by page but not yet
    /// sealed; selection and task admission wait for it to finish or be
    /// cancelled.
    #[error("检索结果仍在加载，完成或取消后才能全选")]
    BatchStillLoading { batch_id: String },
    /// #22 B slice 5: a loading admission named a row that has not arrived
    /// yet. Nothing is frozen; retry once the row has arrived.
    #[error("检索结果仍在加载，所选的行尚未到达")]
    RowsNotYetLoaded { batch_id: String },
    /// A request overlaps a live task without being fully covered by it.
    #[error("部分所选场景或资产已有进行中的任务，请刷新后重新选择")]
    PartialActiveTaskOverlap,
    #[error("任务存储状态修订冲突")]
    RevisionConflict,
    #[error("GACOS 请求需要确认提交或提供匹配的结果，不能通过普通恢复或重试执行")]
    GacosManualActionRequired,
    #[error("GACOS 提交结果尚在返回，请等待记录落盘后再归档或删除")]
    GacosSubmissionInFlight,
    #[error("NISAR V2 prepared intent 无效")]
    NisarSubsetPreparedIntentInvalid,
    #[error("NISAR V2 prepared intent 身份绑定不一致")]
    NisarSubsetPreparedIntentIdentityMismatch,
    #[error("NISAR V2 prepared intent 缺失")]
    NisarSubsetPreparedIntentMissing,
    #[error("NISAR V2 prepared intent 已冻结")]
    NisarSubsetPreparedIntentAlreadyFrozen,
    #[error("NISAR V2 execution plan 与 prepared intent 不一致")]
    NisarSubsetExecutionPlanIntentMismatch,
    #[error("NISAR V2 execution plan 只能为当前 active item 冻结")]
    NisarSubsetExecutionPlanAttachStateInvalid,
    #[error("NISAR V2 execution plan 已冻结且不可替换")]
    NisarSubsetExecutionPlanAlreadyFrozenConflict,
    #[error("NISAR V2 execution plan 无效")]
    NisarSubsetExecutionPlanInvalid,
    #[error("NISAR V2 resolver source size 无效")]
    NisarSubsetSourceSizeInvalid,
    #[error("NISAR V2 resolver source size 身份绑定不一致")]
    NisarSubsetSourceSizeIdentityMismatch,
    #[error("NISAR V2 resolver source size 只能为当前 active item 冻结")]
    NisarSubsetSourceSizeStateInvalid,
    #[error("NISAR V2 resolver source size 已冻结且不可替换")]
    NisarSubsetSourceSizeAlreadyFrozenConflict,
    #[error("NISAR V2 execution anchor 无效")]
    NisarSubsetExecutionAnchorInvalid,
    #[error("NISAR V2 execution anchor lineage 冲突")]
    NisarSubsetExecutionAnchorLineageConflict,
    #[error("归档 NISAR 裁剪任务不满足已验证断点继承条件")]
    NisarArchivedRecoverySourceIneligible,
    #[error("NISAR 预算升级凭据版本 {found} 不受当前程序支持")]
    NisarBudgetUpgradeProofVersionUnsupported { found: u64 },
    #[error("该任务的数据源或类型不支持重检兜底")]
    RetryFallbackUnsupported,
    #[error("只有失败的任务可以重检兜底")]
    RetryFallbackTaskNotFailed,
    #[error("确认的重检批次不是本次为该任务冻结的批次")]
    RetryFallbackConfirmationMismatch,
    #[error("只有失败或已取消的产品可以重检重建")]
    RetryFallbackItemNotRetryable,
    #[error("任务存储记录损坏")]
    Corrupt(#[source] serde_json::Error),
}

impl StoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Database(_) => "V3_STORE_DATABASE_ERROR",
            Self::Filesystem(_) => "V3_STORE_FILESYSTEM_ERROR",
            Self::Missing => "V3_STORE_MISSING",
            Self::Empty => "V3_STORE_EMPTY",
            Self::NewerSchema { .. } => "V3_STORE_SCHEMA_NEWER",
            Self::InvalidSnapshot { .. }
            | Self::InvalidV3Snapshot { .. }
            | Self::SnapshotRowEnvelopeMismatch { .. } => "V3_STORE_INVALID_SNAPSHOT",
            Self::V3SnapshotRejectedByV2Reader => "V3_STORE_V2_READER_REJECTED_V3",
            Self::UnsupportedSnapshotVersion => "V3_STORE_UNSUPPORTED_SNAPSHOT_VERSION",
            Self::AlreadyExists => "V3_STORE_ALREADY_EXISTS",
            Self::NotFound => "V3_STORE_NOT_FOUND",
            Self::PageNotYetLoaded { .. } => "V3_STORE_BATCH_PAGE_NOT_YET_LOADED",
            Self::BatchStillLoading { .. } => "V3_STORE_BATCH_STILL_LOADING",
            Self::RowsNotYetLoaded { .. } => "V3_STORE_BATCH_ROWS_NOT_YET_LOADED",
            Self::PartialActiveTaskOverlap => "V3_STORE_PARTIAL_ACTIVE_TASK_OVERLAP",
            Self::RevisionConflict => "V3_STORE_REVISION_CONFLICT",
            Self::GacosManualActionRequired => "GACOS_MANUAL_ACTION_REQUIRED",
            Self::GacosSubmissionInFlight => "GACOS_SUBMISSION_IN_FLIGHT",
            Self::NisarSubsetPreparedIntentInvalid => {
                "V3_STORE_NISAR_SUBSET_PREPARED_INTENT_INVALID"
            }
            Self::NisarSubsetPreparedIntentIdentityMismatch => {
                "V3_STORE_NISAR_SUBSET_PREPARED_INTENT_IDENTITY_MISMATCH"
            }
            Self::NisarSubsetPreparedIntentMissing => {
                "V3_STORE_NISAR_SUBSET_PREPARED_INTENT_MISSING"
            }
            Self::NisarSubsetPreparedIntentAlreadyFrozen => {
                "V3_STORE_NISAR_SUBSET_PREPARED_INTENT_ALREADY_FROZEN"
            }
            Self::NisarSubsetExecutionPlanIntentMismatch => {
                "V3_STORE_NISAR_SUBSET_EXECUTION_PLAN_INTENT_MISMATCH"
            }
            Self::NisarSubsetExecutionPlanAttachStateInvalid => {
                "V3_STORE_NISAR_SUBSET_EXECUTION_PLAN_ATTACH_STATE_INVALID"
            }
            Self::NisarSubsetExecutionPlanAlreadyFrozenConflict => {
                "V3_STORE_NISAR_SUBSET_EXECUTION_PLAN_ALREADY_FROZEN_CONFLICT"
            }
            Self::NisarSubsetExecutionPlanInvalid => "V3_STORE_NISAR_SUBSET_EXECUTION_PLAN_INVALID",
            Self::NisarSubsetSourceSizeInvalid => "V3_STORE_NISAR_SUBSET_SOURCE_SIZE_INVALID",
            Self::NisarSubsetSourceSizeIdentityMismatch => {
                "V3_STORE_NISAR_SUBSET_SOURCE_SIZE_IDENTITY_MISMATCH"
            }
            Self::NisarSubsetSourceSizeStateInvalid => {
                "V3_STORE_NISAR_SUBSET_SOURCE_SIZE_STATE_INVALID"
            }
            Self::NisarSubsetSourceSizeAlreadyFrozenConflict => {
                "V3_STORE_NISAR_SUBSET_SOURCE_SIZE_ALREADY_FROZEN_CONFLICT"
            }
            Self::NisarSubsetExecutionAnchorInvalid => {
                "V3_STORE_NISAR_SUBSET_EXECUTION_ANCHOR_INVALID"
            }
            Self::NisarSubsetExecutionAnchorLineageConflict => {
                "V3_STORE_NISAR_SUBSET_EXECUTION_ANCHOR_LINEAGE_CONFLICT"
            }
            Self::NisarArchivedRecoverySourceIneligible => {
                "NISAR_ARCHIVED_RECOVERY_SOURCE_INELIGIBLE"
            }
            Self::NisarBudgetUpgradeProofVersionUnsupported { .. } => {
                "V3_STORE_NISAR_BUDGET_UPGRADE_PROOF_VERSION_UNSUPPORTED"
            }
            Self::RetryFallbackUnsupported => "RETRY_FALLBACK_UNSUPPORTED",
            Self::RetryFallbackTaskNotFailed => "RETRY_FALLBACK_TASK_NOT_FAILED",
            Self::RetryFallbackConfirmationMismatch => "RETRY_FALLBACK_CONFIRMATION_MISMATCH",
            Self::RetryFallbackItemNotRetryable => "RETRY_FALLBACK_ITEM_NOT_RETRYABLE",
            Self::Corrupt(_) => "V3_STORE_CORRUPT_ROW",
        }
    }

    /// Another connection holds the database lock (SQLITE_BUSY or
    /// SQLITE_LOCKED), so the same operation may succeed when repeated later.
    pub fn is_database_busy(&self) -> bool {
        matches!(
            self,
            Self::Database(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    }

    /// The database or its directory failed for a local storage reason that
    /// freeing disk space or repairing the disk clears (review 2026-10-17
    /// round 3, P2-2): a full disk (SQLITE_FULL), an I/O error (SQLITE_IOERR
    /// and its extended codes), a database file SQLite cannot open
    /// (SQLITE_CANTOPEN) or a file-system error. Returns a stable cause name;
    /// `None` for every other error, including corruption and schema or
    /// version refusals.
    pub fn local_storage_failure(&self) -> Option<&'static str> {
        match self {
            Self::Database(rusqlite::Error::SqliteFailure(failure, _)) => match failure.code {
                rusqlite::ErrorCode::DiskFull => Some("disk_full"),
                rusqlite::ErrorCode::SystemIoFailure => Some("io_error"),
                rusqlite::ErrorCode::CannotOpen => Some("cannot_open"),
                _ => None,
            },
            Self::Filesystem(_) => Some("filesystem"),
            _ => None,
        }
    }

    pub fn v3_snapshot_validation_issue(&self) -> Option<V3SnapshotValidationIssue> {
        match self {
            Self::InvalidV3Snapshot { issue } => Some(*issue),
            _ => None,
        }
    }

    pub fn task_snapshot_row_envelope_issue(&self) -> Option<TaskSnapshotRowEnvelopeIssue> {
        match self {
            Self::SnapshotRowEnvelopeMismatch { issue } => Some(*issue),
            _ => None,
        }
    }

    pub(crate) fn invalid(reason: impl Into<String>) -> Self {
        Self::InvalidSnapshot {
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_v3_snapshot(issue: V3SnapshotValidationIssue) -> Self {
        Self::InvalidV3Snapshot { issue }
    }

    pub(crate) fn snapshot_row_envelope_mismatch(issue: TaskSnapshotRowEnvelopeIssue) -> Self {
        Self::SnapshotRowEnvelopeMismatch { issue }
    }

    pub(crate) fn v3_snapshot_rejected_by_v2_reader() -> Self {
        Self::V3SnapshotRejectedByV2Reader
    }

    pub(crate) fn unsupported_snapshot_version() -> Self {
        Self::UnsupportedSnapshotVersion
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite(code: std::ffi::c_int) -> StoreError {
        StoreError::Database(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            None,
        ))
    }

    /// Review 2026-10-17 round 3, P2-2: only a full disk, an I/O error, a file
    /// SQLite cannot open and a file-system error are local storage failures.
    #[test]
    fn local_storage_failures_are_disk_full_io_cannot_open_and_filesystem_only() {
        for (error, cause) in [
            (sqlite(rusqlite::ffi::SQLITE_FULL), "disk_full"),
            (sqlite(rusqlite::ffi::SQLITE_IOERR), "io_error"),
            (sqlite(rusqlite::ffi::SQLITE_IOERR_WRITE), "io_error"),
            (sqlite(rusqlite::ffi::SQLITE_IOERR_FSYNC), "io_error"),
            (sqlite(rusqlite::ffi::SQLITE_CANTOPEN), "cannot_open"),
            (
                StoreError::Filesystem(std::io::Error::other("injected")),
                "filesystem",
            ),
        ] {
            assert_eq!(error.local_storage_failure(), Some(cause), "{error:?}");
            assert!(!error.is_database_busy(), "{error:?}");
        }
        for error in [
            sqlite(rusqlite::ffi::SQLITE_CORRUPT),
            sqlite(rusqlite::ffi::SQLITE_NOTADB),
            sqlite(rusqlite::ffi::SQLITE_CONSTRAINT),
            sqlite(rusqlite::ffi::SQLITE_READONLY),
            sqlite(rusqlite::ffi::SQLITE_BUSY),
            StoreError::NewerSchema {
                found: 31,
                supported: 30,
            },
            StoreError::UnsupportedSnapshotVersion,
            StoreError::RevisionConflict,
            StoreError::NotFound,
        ] {
            assert_eq!(error.local_storage_failure(), None, "{error:?}");
        }
    }
}
