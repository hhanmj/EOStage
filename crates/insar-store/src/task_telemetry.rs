use std::path::PathBuf;

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::history::canonical_archive_timestamp;
use crate::repository::sqlite_integer;
use crate::runtime::reject_closed_task;
use crate::{StoreError, TaskStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreTaskTransferTelemetryUpdate {
    pub task_id: String,
    pub expected_revision: u64,
    pub transfer_active_millis: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreTaskTransferTelemetry {
    pub task_id: String,
    pub revision: u64,
    pub transfer_active_millis: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoreTaskOutputDirectoryRequest {
    pub task_id: String,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreTaskOutputDirectory {
    pub task_id: String,
    pub revision: u64,
    pub output_root: PathBuf,
}

impl TaskStore {
    /// Persisted baseline for a newly created scheduler/worker lifetime.
    pub fn task_transfer_checkpoint_baseline(
        &self,
        task_id: &str,
    ) -> Result<(u64, u64), StoreError> {
        let row: Option<(i64, i64)> = self
            .connection
            .query_row(
                "SELECT transfer_active_millis, COALESCE(network_bytes_received, 0)
             FROM task_transfer_telemetry WHERE task_id = ?1",
                [task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (millis, bytes) = row.unwrap_or((0, 0));
        Ok((
            u64::try_from(millis).map_err(|_| StoreError::invalid("negative transfer time"))?,
            u64::try_from(bytes).map_err(|_| StoreError::invalid("negative network bytes"))?,
        ))
    }

    pub fn save_task_transfer_telemetry(
        &mut self,
        update: &StoreTaskTransferTelemetryUpdate,
    ) -> Result<StoreTaskTransferTelemetry, StoreError> {
        self.save_task_transfer_telemetry_with_network(update, None)
    }

    /// One Store-owned cumulative counter, checkpointed with the existing
    /// transfer clock. Older workers leave network measurement unavailable.
    pub fn save_task_transfer_telemetry_with_network(
        &mut self,
        update: &StoreTaskTransferTelemetryUpdate,
        network_bytes_received: Option<u64>,
    ) -> Result<StoreTaskTransferTelemetry, StoreError> {
        if update.task_id.is_empty()
            || update.task_id.chars().count() > 128
            || update.task_id.trim() != update.task_id
            || update.task_id.chars().any(char::is_control)
        {
            return Err(StoreError::invalid("task telemetry task ID is invalid"));
        }
        let (canonical_updated_at, _) = canonical_archive_timestamp(&update.updated_at)
            .filter(|(canonical, _)| canonical == &update.updated_at)
            .ok_or_else(|| StoreError::invalid("task telemetry timestamp is not canonical"))?;
        let expected_revision = sqlite_integer(update.expected_revision, "task revision")?;
        let transfer_active_millis = sqlite_integer(
            update.transfer_active_millis,
            "task transfer active milliseconds",
        )?;
        let network_bytes_received = network_bytes_received
            .map(|bytes| sqlite_integer(bytes, "network bytes received"))
            .transpose()?;

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        reject_closed_task(&transaction, &update.task_id)?;
        // Transfer telemetry is checkpointed against the same download-engine
        // snapshot that owns the sampled bytes. Task lifecycle revisions move
        // independently on pause/resume and are not the checkpoint authority.
        let revision: i64 = transaction
            .query_row(
                "SELECT revision FROM download_engine_state WHERE task_id = ?1",
                [&update.task_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if revision != expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        let previous: Option<i64> = transaction
            .query_row(
                "SELECT transfer_active_millis FROM task_transfer_telemetry WHERE task_id = ?1",
                [&update.task_id],
                |row| row.get(0),
            )
            .optional()?;
        if previous.is_some_and(|value| transfer_active_millis < value) {
            return Err(StoreError::invalid(
                "task transfer active milliseconds cannot move backwards",
            ));
        }
        let previous_network: Option<i64> = transaction
            .query_row(
                "SELECT network_bytes_received FROM task_transfer_telemetry WHERE task_id = ?1",
                [&update.task_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if previous_network
            .zip(network_bytes_received)
            .is_some_and(|(prior, next)| next < prior)
        {
            return Err(StoreError::invalid(
                "network byte counter cannot move backwards",
            ));
        }
        transaction.execute(
            "INSERT INTO task_transfer_telemetry(task_id, transfer_active_millis, updated_at, network_bytes_received)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(task_id) DO UPDATE SET
                transfer_active_millis = excluded.transfer_active_millis,
                network_bytes_received = COALESCE(excluded.network_bytes_received, task_transfer_telemetry.network_bytes_received),
                updated_at = excluded.updated_at",
            params![update.task_id, transfer_active_millis, canonical_updated_at, network_bytes_received],
        )?;
        transaction.commit()?;
        Ok(StoreTaskTransferTelemetry {
            task_id: update.task_id.clone(),
            revision: update.expected_revision,
            transfer_active_millis: update.transfer_active_millis,
            updated_at: update.updated_at.clone(),
        })
    }

    pub fn resolve_task_output_directory(
        &self,
        request: &StoreTaskOutputDirectoryRequest,
    ) -> Result<StoreTaskOutputDirectory, StoreError> {
        if request.task_id.is_empty()
            || request.task_id.chars().count() > 128
            || request.task_id.trim() != request.task_id
            || request.task_id.chars().any(char::is_control)
        {
            return Err(StoreError::invalid(
                "task output directory task ID is invalid",
            ));
        }
        if !crate::core_restore::task_is_actionable(&self.connection, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        let row = self
            .connection
            .query_row(
                "SELECT r.revision, COALESCE(summary.output_root, snapshot.output_root)
                 FROM task_runtime r
                 JOIN task_snapshots snapshot ON snapshot.task_id = r.task_id
                 LEFT JOIN task_history history ON history.task_id = r.task_id
                 LEFT JOIN task_history_summary summary ON summary.task_id = r.task_id
                 LEFT JOIN task_tombstones tombstone ON tombstone.task_id = r.task_id
                 WHERE r.task_id = ?1 AND tombstone.task_id IS NULL
                   AND (history.task_id IS NULL OR summary.task_id IS NOT NULL)",
                [&request.task_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let revision = u64::try_from(row.0)
            .map_err(|_| StoreError::invalid("negative task output directory revision"))?;
        if revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if row.1.is_empty()
            || row.1.chars().count() > 4096
            || row.1.trim() != row.1
            || row.1.chars().any(char::is_control)
        {
            return Err(StoreError::invalid("task output directory is invalid"));
        }
        Ok(StoreTaskOutputDirectory {
            task_id: request.task_id.clone(),
            revision,
            output_root: PathBuf::from(row.1),
        })
    }
}
