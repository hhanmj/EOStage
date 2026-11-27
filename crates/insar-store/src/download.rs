use insar_core::{
    DownloadEngineSnapshot, DownloadItemProgress, DownloadItemResult, DownloadItemStatus,
    DownloadOutcome,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::repository::{load_compact_task_connection, sqlite_integer};
use crate::{StoreError, TaskStore};

impl TaskStore {
    pub fn load_download_snapshot(
        &self,
        task_id: &str,
    ) -> Result<DownloadEngineSnapshot, StoreError> {
        let snapshot = load_snapshot(&self.connection, task_id)?;
        if !crate::core_restore::task_is_actionable(&self.connection, task_id)? {
            return Err(StoreError::NotFound);
        }
        Ok(snapshot)
    }

    /// Lightweight scalar revision lookup for `checkpoint_task_transfer_activity`.
    ///
    /// Returns `Ok(None)` when the task does not exist or has already been
    /// archived (`task_history`) or deleted (`task_tombstones`), preventing both
    /// multi-megabyte JSON hydration under the Store mutex and immutable-task
    /// `V3_STORE_INVALID_SNAPSHOT` retry loops after archival.
    pub fn scheduler_transfer_checkpoint_revision(
        &self,
        task_id: &str,
    ) -> Result<Option<u64>, StoreError> {
        let open_actionable: bool = self.connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM task_snapshots s
                JOIN task_runtime r ON r.task_id = s.task_id
                LEFT JOIN task_history h ON h.task_id = s.task_id
                LEFT JOIN task_tombstones t ON t.task_id = s.task_id
                WHERE s.task_id = ?1 AND h.task_id IS NULL AND t.task_id IS NULL
            )",
            [task_id],
            |row| row.get(0),
        )?;
        if !open_actionable {
            return Ok(None);
        }
        current_download_revision(&self.connection, task_id).map(Some)
    }

    pub fn save_download_progress(
        &mut self,
        progress: &DownloadItemProgress,
        expected_revision: u64,
    ) -> Result<DownloadEngineSnapshot, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let next_revision =
            persist_download_progress(&transaction, progress, expected_revision, true)?;
        let snapshot = load_snapshot_connection(&transaction, &progress.task_id, next_revision)?;
        transaction.commit()?;
        Ok(snapshot)
    }

    /// Scheduler-only progress hot path.
    ///
    /// The coordinator serializes progress/result messages for one Store writer,
    /// so it can read the current download revision inside the same IMMEDIATE
    /// transaction and return only the next scalar revision. This preserves the
    /// existing download-engine revision authority and row validation while
    /// avoiding the full pre-read/post-write snapshot materialization that the
    /// Scheduler never consumes.
    pub fn save_scheduler_download_progress(
        &mut self,
        progress: &DownloadItemProgress,
    ) -> Result<u64, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_scheduler_item(&transaction, &progress.task_id, &progress.item_id)?;
        let expected_revision = current_download_revision(&transaction, &progress.task_id)?;
        let next_revision =
            persist_download_progress(&transaction, progress, expected_revision, false)?;
        transaction.commit()?;
        Ok(next_revision)
    }

    pub fn save_download_result(
        &mut self,
        result: &DownloadItemResult,
        expected_revision: u64,
    ) -> Result<DownloadEngineSnapshot, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let next_revision = persist_download_result(&transaction, result, expected_revision, true)?;
        let snapshot = load_snapshot_connection(&transaction, &result.task_id, next_revision)?;
        transaction.commit()?;
        Ok(snapshot)
    }

    /// Scheduler-only terminal-result hot path sharing the canonical
    /// download-engine revision authority without materializing a snapshot the
    /// coordinator never consumes.
    pub fn save_scheduler_download_result(
        &mut self,
        result: &DownloadItemResult,
    ) -> Result<u64, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_scheduler_item(&transaction, &result.task_id, &result.item_id)?;
        let expected_revision = current_download_revision(&transaction, &result.task_id)?;
        let next_revision =
            persist_download_result(&transaction, result, expected_revision, false)?;
        transaction.commit()?;
        Ok(next_revision)
    }
}

pub(crate) fn persist_download_result(
    connection: &Connection,
    result: &DownloadItemResult,
    expected_revision: u64,
    full_item_validation: bool,
) -> Result<u64, StoreError> {
    result.validate().map_err(StoreError::invalid)?;
    if result.finished_at.trim().is_empty() || result.destination.as_os_str().is_empty() {
        return Err(StoreError::invalid(
            "download result timestamp and destination are required",
        ));
    }
    if full_item_validation {
        validate_item(connection, &result.task_id, &result.item_id)?;
    }
    let next_revision = advance_revision(
        connection,
        &result.task_id,
        expected_revision,
        &result.finished_at,
    )?;
    let previous_progress_total_sql: Option<i64> = connection
        .query_row(
            "SELECT last_known_total_bytes FROM download_progress WHERE task_id = ?1 AND item_id = ?2",
            params![result.task_id, result.item_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let previous_result_total_sql: Option<i64> = connection
        .query_row(
            "SELECT last_known_total_bytes FROM download_results WHERE task_id = ?1 AND item_id = ?2",
            params![result.task_id, result.item_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let previous_result_total = previous_result_total_sql
        .map(|value| {
            u64::try_from(value).map_err(|_| StoreError::invalid("negative last known total bytes"))
        })
        .transpose()?;
    let last_known_total_bytes = previous_progress_total_sql
        .map(|value| {
            u64::try_from(value).map_err(|_| StoreError::invalid("negative last known total bytes"))
        })
        .transpose()?
        .or(previous_result_total)
        .or_else(|| {
            matches!(
                result.outcome,
                DownloadOutcome::Completed | DownloadOutcome::Skipped
            )
            .then_some(result.bytes_written)
        });
    let last_known_total_sql = last_known_total_bytes
        .map(|value| sqlite_integer(value, "last known total bytes"))
        .transpose()?;
    let result_json = serde_json::to_string(result).map_err(StoreError::Corrupt)?;
    let outcome = outcome_name(&result.outcome);
    connection.execute(
        "INSERT INTO download_results(task_id, item_id, outcome, finished_at, result_json, last_known_total_bytes, bytes_written) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(task_id, item_id) DO UPDATE SET \
         outcome = excluded.outcome, finished_at = excluded.finished_at, \
             result_json = excluded.result_json, \
             last_known_total_bytes = excluded.last_known_total_bytes, bytes_written = excluded.bytes_written",
        params![
            result.task_id,
            result.item_id,
            outcome,
            result.finished_at,
            result_json,
            last_known_total_sql,
            sqlite_integer(result.bytes_written, "result bytes written")?,
        ],
    )?;
    connection.execute(
        "DELETE FROM download_progress WHERE task_id = ?1 AND item_id = ?2",
        params![result.task_id, result.item_id],
    )?;
    Ok(next_revision)
}

fn persist_download_progress(
    connection: &Connection,
    progress: &DownloadItemProgress,
    expected_revision: u64,
    full_item_validation: bool,
) -> Result<u64, StoreError> {
    progress.validate().map_err(StoreError::invalid)?;
    if progress.updated_at.trim().is_empty() {
        return Err(StoreError::invalid(
            "download progress timestamp is required",
        ));
    }
    if full_item_validation {
        validate_item(connection, &progress.task_id, &progress.item_id)?;
    }
    let next_revision = advance_revision(
        connection,
        &progress.task_id,
        expected_revision,
        &progress.updated_at,
    )?;
    let prior_result_total_sql: Option<i64> = connection
        .query_row(
            "SELECT last_known_total_bytes FROM download_results WHERE task_id = ?1 AND item_id = ?2",
            params![progress.task_id, progress.item_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let prior_result_total = prior_result_total_sql
        .map(|value| {
            u64::try_from(value).map_err(|_| StoreError::invalid("negative last known total bytes"))
        })
        .transpose()?;
    let prior_progress_total_sql: Option<i64> = connection
        .query_row(
            "SELECT last_known_total_bytes FROM download_progress WHERE task_id = ?1 AND item_id = ?2",
            params![progress.task_id, progress.item_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let prior_progress_total = prior_progress_total_sql
        .map(|value| {
            u64::try_from(value).map_err(|_| StoreError::invalid("negative last known total bytes"))
        })
        .transpose()?;
    let mut persisted_progress = progress.clone();
    persisted_progress.total_bytes = progress
        .total_bytes
        .or(prior_progress_total)
        .or(prior_result_total);
    let progress_json = serde_json::to_string(&persisted_progress).map_err(StoreError::Corrupt)?;
    let status = status_name(&persisted_progress.status);
    let bytes_completed = sqlite_integer(
        persisted_progress.bytes_completed,
        "download bytes completed",
    )?;
    let total_bytes = persisted_progress
        .total_bytes
        .map(|value| sqlite_integer(value, "download total bytes"))
        .transpose()?;
    connection.execute(
        "INSERT INTO download_progress(task_id, item_id, status, updated_at, progress_json, bytes_completed, total_bytes, bytes_per_second, last_known_total_bytes) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?7) \
         ON CONFLICT(task_id, item_id) DO UPDATE SET \
         status = excluded.status, updated_at = excluded.updated_at, \
             progress_json = excluded.progress_json, bytes_completed = excluded.bytes_completed, \
             total_bytes = excluded.total_bytes, bytes_per_second = excluded.bytes_per_second, \
             last_known_total_bytes = excluded.last_known_total_bytes",
        params![
            progress.task_id,
            progress.item_id,
            status,
            persisted_progress.updated_at,
            progress_json,
            bytes_completed,
            total_bytes,
            persisted_progress.bytes_per_second,
        ],
    )?;
    connection.execute(
        "DELETE FROM download_results WHERE task_id = ?1 AND item_id = ?2",
        params![progress.task_id, progress.item_id],
    )?;
    Ok(next_revision)
}

pub(crate) fn current_download_revision(
    connection: &Connection,
    task_id: &str,
) -> Result<u64, StoreError> {
    connection
        .query_row(
            "SELECT revision FROM download_engine_state WHERE task_id = ?1",
            [task_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map(|value| {
            u64::try_from(value).map_err(|_| StoreError::invalid("negative download revision"))
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn validate_item(connection: &Connection, task_id: &str, item_id: &str) -> Result<(), StoreError> {
    if !crate::core_restore::task_is_actionable(connection, task_id)? {
        return Err(StoreError::NotFound);
    }
    let task = load_compact_task_connection(connection, task_id)?.ok_or(StoreError::NotFound)?;
    if task
        .runtime
        .selected_item_ids
        .iter()
        .any(|selected| selected == item_id)
    {
        Ok(())
    } else {
        Err(StoreError::invalid(
            "download item must be selected in the frozen task",
        ))
    }
}

/// Scheduler workers are spawned only from a fully decoded, actionable task.
/// Frozen task identity is immutable while the worker runs; the hot path
/// therefore revalidates only the mutable runtime authority and selected-item
/// membership. Public Store callers retain the full snapshot validation above.
fn validate_scheduler_item(
    connection: &Connection,
    task_id: &str,
    item_id: &str,
) -> Result<(), StoreError> {
    let runtime_json = connection
        .query_row(
            "SELECT r.state_json
             FROM task_runtime r
             JOIN task_snapshots s ON s.task_id = r.task_id
             WHERE r.task_id = ?1",
            [task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(StoreError::NotFound)?;
    let runtime: insar_core::TaskRuntimeSnapshot =
        serde_json::from_str(&runtime_json).map_err(StoreError::Corrupt)?;
    if runtime.task_id != task_id {
        return Err(StoreError::invalid("runtime task ID differs"));
    }
    runtime.validate().map_err(StoreError::invalid)?;
    if runtime
        .selected_item_ids
        .iter()
        .any(|selected| selected == item_id)
    {
        Ok(())
    } else {
        Err(StoreError::invalid(
            "download item must be selected in the frozen task",
        ))
    }
}

fn advance_revision(
    connection: &Connection,
    task_id: &str,
    expected_revision: u64,
    updated_at: &str,
) -> Result<u64, StoreError> {
    let current = connection
        .query_row(
            "SELECT revision FROM download_engine_state WHERE task_id = ?1",
            [task_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map(|value| {
            u64::try_from(value).map_err(|_| StoreError::invalid("negative download revision"))
        })
        .transpose()?
        .unwrap_or(0);
    if current != expected_revision {
        return Err(StoreError::RevisionConflict);
    }
    let next = current
        .checked_add(1)
        .ok_or_else(|| StoreError::invalid("download revision overflow"))?;
    let next_sql = sqlite_integer(next, "download revision")?;
    connection.execute(
        "INSERT INTO download_engine_state(task_id, revision, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(task_id) DO UPDATE SET revision = excluded.revision, updated_at = excluded.updated_at",
        params![task_id, next_sql, updated_at],
    )?;
    Ok(next)
}

fn load_snapshot(
    connection: &Connection,
    task_id: &str,
) -> Result<DownloadEngineSnapshot, StoreError> {
    if load_compact_task_connection(connection, task_id)?.is_none() {
        return Err(StoreError::NotFound);
    }
    let revision = connection
        .query_row(
            "SELECT revision FROM download_engine_state WHERE task_id = ?1",
            [task_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .map(|value| {
            u64::try_from(value).map_err(|_| StoreError::invalid("negative download revision"))
        })
        .transpose()?
        .unwrap_or(0);
    load_snapshot_connection(connection, task_id, revision)
}

pub(crate) fn load_snapshot_connection(
    connection: &Connection,
    task_id: &str,
    revision: u64,
) -> Result<DownloadEngineSnapshot, StoreError> {
    let mut progress_statement = connection.prepare(
        "SELECT progress_json FROM download_progress WHERE task_id = ?1 ORDER BY item_id",
    )?;
    let progress = progress_statement
        .query_map([task_id], |row| row.get::<_, String>(0))?
        .map(|row| {
            let value = row?;
            serde_json::from_str(&value).map_err(StoreError::Corrupt)
        })
        .collect::<Result<Vec<DownloadItemProgress>, StoreError>>()?;
    let mut result_statement = connection
        .prepare("SELECT result_json FROM download_results WHERE task_id = ?1 ORDER BY item_id")?;
    let results = result_statement
        .query_map([task_id], |row| row.get::<_, String>(0))?
        .map(|row| {
            let value = row?;
            serde_json::from_str(&value).map_err(StoreError::Corrupt)
        })
        .collect::<Result<Vec<DownloadItemResult>, StoreError>>()?;
    let snapshot = DownloadEngineSnapshot {
        task_id: task_id.to_string(),
        revision,
        progress,
        results,
    };
    snapshot.validate().map_err(StoreError::invalid)?;
    if snapshot
        .progress
        .iter()
        .any(|value| value.task_id != task_id)
        || snapshot
            .results
            .iter()
            .any(|value| value.task_id != task_id)
    {
        return Err(StoreError::invalid("download snapshot task IDs differ"));
    }
    Ok(snapshot)
}

fn status_name(value: &DownloadItemStatus) -> &'static str {
    match value {
        DownloadItemStatus::Queued => "queued",
        DownloadItemStatus::Active => "active",
        DownloadItemStatus::Paused => "paused",
        DownloadItemStatus::Completed => "completed",
        DownloadItemStatus::Failed => "failed",
        DownloadItemStatus::Skipped => "skipped",
        DownloadItemStatus::Cancelled => "cancelled",
        DownloadItemStatus::Interrupted => "interrupted",
    }
}

fn outcome_name(value: &DownloadOutcome) -> &'static str {
    match value {
        DownloadOutcome::Completed => "completed",
        DownloadOutcome::Skipped => "skipped",
        DownloadOutcome::Failed => "failed",
        DownloadOutcome::Cancelled => "cancelled",
        DownloadOutcome::Interrupted => "interrupted",
    }
}
