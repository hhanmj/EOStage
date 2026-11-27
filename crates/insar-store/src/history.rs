use insar_core::{
    DownloadLayout, StoreArchiveTaskRequest, StoreDeleteHistoryRequest, StoreRestoreRequest,
    StoreRestoreResponse, TaskLifecycleStatus,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::json;

use crate::core_restore::{
    actionable_task_summary, lifecycle_status_name, task_dataset_id_from_snapshot_json,
    StoreTaskSummary,
};
use crate::repository::{load_compact_task_connection, load_task_connection};
use crate::{StoreError, StoredTask, TaskStore};

#[derive(Debug, Clone, PartialEq)]
pub struct StoredHistory {
    pub archived_at: String,
    pub task: StoredTask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedDownloadDestination {
    pub task_id: String,
    pub task_title: Option<String>,
    pub item_id: String,
    pub destination: std::path::PathBuf,
}

struct PreparedHistorySummary {
    archived_at: String,
    archived_at_epoch_ms: i64,
    output_root: String,
    counts: [i64; 9],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedDemSourceAuthority {
    region_id: String,
    dataset: String,
    provider: String,
    resource_id: String,
    capability_id: String,
    input_path: String,
    output_root: String,
}

fn bounded_scalar(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.chars().count() <= max_chars
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn transfer_active_millis(connection: &Connection, task_id: &str) -> Result<u64, StoreError> {
    let value = connection
        .query_row(
            "SELECT transfer_active_millis FROM task_transfer_telemetry WHERE task_id = ?1",
            [task_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0);
    u64::try_from(value)
        .map_err(|_| StoreError::invalid("task transfer active milliseconds are negative"))
}

pub(crate) fn canonical_archive_timestamp(value: &str) -> Option<(String, i64)> {
    if value.len() < 20 || value.len() > 30 || !value.is_ascii() || !value.ends_with('Z') {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let number = |start: usize, end: usize| value.get(start..end)?.parse::<i64>().ok();
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if day < 1 || day > max_day {
        return None;
    }
    let fractional = value.get(19..value.len() - 1)?;
    let milliseconds = if fractional.is_empty() {
        0
    } else {
        let digits = fractional.strip_prefix('.')?;
        if digits.is_empty()
            || digits.len() > 9
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let mut millis = digits.chars().take(3).collect::<String>();
        while millis.len() < 3 {
            millis.push('0');
        }
        millis.parse::<i64>().ok()?
    };

    // Howard Hinnant's civil-date conversion, with Euclidean division for year 0000.
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let epoch_days = era * 146_097 + day_of_era - 719_468;
    let epoch_ms = epoch_days
        .checked_mul(86_400_000)?
        .checked_add(hour.checked_mul(3_600_000)?)?
        .checked_add(minute.checked_mul(60_000)?)?
        .checked_add(second.checked_mul(1_000)?)?
        .checked_add(milliseconds)?;
    let canonical = if milliseconds == 0 {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    } else {
        let fraction = format!("{milliseconds:03}")
            .trim_end_matches('0')
            .to_owned();
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{fraction}Z")
    };
    Some((canonical, epoch_ms))
}

/// Canonical UTC timestamp for the selected DEM identity contract. Inputs may
/// use whole seconds or one to three fractional digits; persisted/output values
/// remove trailing fractional zeroes. Other timezone and precision forms fail
/// closed rather than inheriting the broader archive timestamp contract.
pub fn canonical_utc_identity_timestamp(value: &str) -> Option<String> {
    if value.len() < 20 || value.len() > 24 {
        return None;
    }
    let fractional = value.get(19..value.len().checked_sub(1)?)?;
    if !fractional.is_empty() {
        let digits = fractional.strip_prefix('.')?;
        if digits.is_empty()
            || digits.len() > 3
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
    }
    canonical_archive_timestamp(value).map(|(canonical, _)| canonical)
}

fn prepare_history_summary(
    summary: &StoreTaskSummary,
    archived_at: &str,
) -> Result<PreparedHistorySummary, StoreError> {
    let (archived_at, archived_at_epoch_ms) = canonical_archive_timestamp(archived_at)
        .ok_or_else(|| StoreError::invalid("archive timestamp is invalid"))?;
    if !bounded_scalar(&summary.task_id, 128)
        || !bounded_scalar(&summary.batch_id, 128)
        || !bounded_scalar(&summary.provider_id, 128)
        || summary
            .dataset_id
            .as_deref()
            .is_some_and(|value| !bounded_scalar(value, 128))
        || canonical_archive_timestamp(&summary.created_at).is_none()
        || canonical_archive_timestamp(&summary.updated_at).is_none()
        || !is_terminal(&summary.status)
        || !summary.actionable
    {
        return Err(StoreError::invalid(
            "task history summary scalars are invalid",
        ));
    }
    let output_root = summary
        .output_root
        .to_str()
        .filter(|value| bounded_scalar(value, 4096))
        .ok_or_else(|| StoreError::invalid("task history output root is invalid"))?
        .to_owned();
    if let Some(operation) = &summary.operation {
        for value in [
            &operation.operation_id,
            &operation.kind,
            &operation.phase,
            &operation.region_id,
            &operation.approved_worker_operation,
        ] {
            if !bounded_scalar(value, 128) {
                return Err(StoreError::invalid(
                    "task history operation summary is invalid",
                ));
            }
        }
    }
    let as_i64 = |value: usize| {
        i64::try_from(value).map_err(|_| StoreError::invalid("task history count overflow"))
    };
    let counts = [
        as_i64(summary.selected_count)?,
        as_i64(summary.checked_count)?,
        as_i64(summary.active_count)?,
        as_i64(summary.paused_count)?,
        as_i64(summary.queued_count)?,
        as_i64(summary.completed_count)?,
        as_i64(summary.failed_count)?,
        as_i64(summary.skipped_count)?,
        as_i64(summary.cancelled_count)?,
    ];
    if counts[0] < 1 || counts[1] > counts[0] || counts[2..].iter().sum::<i64>() != counts[0] {
        return Err(StoreError::invalid(
            "task history lifecycle counts are invalid",
        ));
    }
    Ok(PreparedHistorySummary {
        archived_at,
        archived_at_epoch_ms,
        output_root,
        counts,
    })
}

fn prepare_dem_source_authority(
    task: &StoredTask,
) -> Result<Option<PreparedDemSourceAuthority>, StoreError> {
    let Some(operation) = task.snapshot.operation.as_ref() else {
        return Ok(None);
    };
    let accepted_operation = (operation.kind == "dem"
        && operation.phase == "execute"
        && operation.approved_worker_operation == "dem_prepare_sarscape")
        || (operation.kind == "dem"
            && operation.phase == "download"
            && operation.approved_worker_operation == "dem_download");
    if !accepted_operation {
        return Ok(None);
    }
    // Legacy download operations can have no scientific source envelope. They
    // remain archivable as history, but cannot become a DEM conversion source.
    if operation.approved_worker_operation == "dem_download" && operation.payload.is_empty() {
        return Ok(None);
    }
    let scalar = |key: &str, max_chars: usize| {
        operation
            .payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| bounded_scalar(value, max_chars))
            .map(ToOwned::to_owned)
            .ok_or_else(|| StoreError::invalid("DEM source authority payload is invalid"))
    };
    // Earlier native AOI downloads froze the region only in the operation
    // descriptor. It remains authoritative; a present payload alias must match.
    let region_id = if operation.phase == "download"
        && operation.approved_worker_operation == "dem_download"
        && !operation.payload.contains_key("region_id")
    {
        if !bounded_scalar(&operation.region_id, 128) {
            return Err(StoreError::invalid("DEM source region identity is invalid"));
        }
        operation.region_id.clone()
    } else {
        scalar("region_id", 128)?
    };
    let dataset = scalar("dataset", 128)?;
    let provider = scalar("provider", 128)?;
    let resource_id = scalar("resource_id", 128)?;
    let capability_id = scalar("capability_id", 128)?;
    let input_path = scalar("input_path", 4096)?;
    let path_alias = scalar("path", 4096)?;
    let output_root = scalar("output_root", 4096)?;
    let output_dir = scalar("output_dir", 4096)?;
    let snapshot_output_root = task
        .snapshot
        .output_root
        .to_str()
        .filter(|value| bounded_scalar(value, 4096))
        .ok_or_else(|| StoreError::invalid("DEM source output root is invalid"))?;
    if region_id != operation.region_id
        || input_path != path_alias
        || output_root != output_dir
        || output_root != snapshot_output_root
    {
        return Err(StoreError::invalid(
            "DEM source authority differs from its persisted task",
        ));
    }
    Ok(Some(PreparedDemSourceAuthority {
        region_id,
        dataset,
        provider,
        resource_id,
        capability_id,
        input_path,
        output_root,
    }))
}

fn insert_history_summary(
    connection: &Connection,
    summary: &StoreTaskSummary,
    prepared: &PreparedHistorySummary,
) -> Result<(), StoreError> {
    let operation = summary.operation.as_ref();
    let has_transfer_active_millis: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('task_history_summary') WHERE name='transfer_active_millis')",
        [],
        |row| row.get(0),
    )?;
    let has_dataset_id: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('task_history_summary') WHERE name='dataset_id')",
        [],
        |row| row.get(0),
    )?;
    if has_dataset_id {
        connection.execute(
            "INSERT INTO task_history_summary(
                task_id, archived_at, archived_at_epoch_ms, batch_id, provider_id, created_at,
                output_root, layout, status, revision, updated_at, selected_count, checked_count,
                active_count, paused_count, queued_count, completed_count, failed_count,
                skipped_count, cancelled_count, operation_id, operation_kind, operation_phase,
                operation_region_id, approved_worker_operation, transfer_active_millis, dataset_id
             ) VALUES (
                ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
                ?19,?20,?21,?22,?23,?24,?25,?26,?27
             )",
            params![
                summary.task_id,
                prepared.archived_at,
                prepared.archived_at_epoch_ms,
                summary.batch_id,
                summary.provider_id,
                summary.created_at,
                prepared.output_root,
                layout_name(&summary.layout),
                lifecycle_status_name(&summary.status),
                i64::try_from(summary.revision)
                    .map_err(|_| StoreError::invalid("task history revision overflow"))?,
                summary.updated_at,
                prepared.counts[0],
                prepared.counts[1],
                prepared.counts[2],
                prepared.counts[3],
                prepared.counts[4],
                prepared.counts[5],
                prepared.counts[6],
                prepared.counts[7],
                prepared.counts[8],
                operation.map(|value| value.operation_id.as_str()),
                operation.map(|value| value.kind.as_str()),
                operation.map(|value| value.phase.as_str()),
                operation.map(|value| value.region_id.as_str()),
                operation.map(|value| value.approved_worker_operation.as_str()),
                i64::try_from(summary.transfer_active_millis).map_err(|_| {
                    StoreError::invalid("task history transfer active milliseconds overflow")
                })?,
                summary.dataset_id.as_deref(),
            ],
        )?;
        return Ok(());
    }
    if !has_transfer_active_millis {
        connection.execute(
            "INSERT INTO task_history_summary(
                task_id, archived_at, archived_at_epoch_ms, batch_id, provider_id, created_at,
                output_root, layout, status, revision, updated_at, selected_count, checked_count,
                active_count, paused_count, queued_count, completed_count, failed_count,
                skipped_count, cancelled_count, operation_id, operation_kind, operation_phase,
                operation_region_id, approved_worker_operation
             ) VALUES (
                ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
                ?19,?20,?21,?22,?23,?24,?25
             )",
            params![
                summary.task_id,
                prepared.archived_at,
                prepared.archived_at_epoch_ms,
                summary.batch_id,
                summary.provider_id,
                summary.created_at,
                prepared.output_root,
                layout_name(&summary.layout),
                lifecycle_status_name(&summary.status),
                i64::try_from(summary.revision)
                    .map_err(|_| StoreError::invalid("task history revision overflow"))?,
                summary.updated_at,
                prepared.counts[0],
                prepared.counts[1],
                prepared.counts[2],
                prepared.counts[3],
                prepared.counts[4],
                prepared.counts[5],
                prepared.counts[6],
                prepared.counts[7],
                prepared.counts[8],
                operation.map(|value| value.operation_id.as_str()),
                operation.map(|value| value.kind.as_str()),
                operation.map(|value| value.phase.as_str()),
                operation.map(|value| value.region_id.as_str()),
                operation.map(|value| value.approved_worker_operation.as_str()),
            ],
        )?;
        return Ok(());
    }
    connection.execute(
        "INSERT INTO task_history_summary(
            task_id, archived_at, archived_at_epoch_ms, batch_id, provider_id, created_at,
            output_root, layout, status, revision, updated_at, selected_count, checked_count,
            active_count, paused_count, queued_count, completed_count, failed_count,
            skipped_count, cancelled_count, operation_id, operation_kind, operation_phase,
            operation_region_id, approved_worker_operation, transfer_active_millis
         ) VALUES (
            ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,
            ?19,?20,?21,?22,?23,?24,?25,?26
         )",
        params![
            summary.task_id,
            prepared.archived_at,
            prepared.archived_at_epoch_ms,
            summary.batch_id,
            summary.provider_id,
            summary.created_at,
            prepared.output_root,
            layout_name(&summary.layout),
            lifecycle_status_name(&summary.status),
            i64::try_from(summary.revision)
                .map_err(|_| StoreError::invalid("task history revision overflow"))?,
            summary.updated_at,
            prepared.counts[0],
            prepared.counts[1],
            prepared.counts[2],
            prepared.counts[3],
            prepared.counts[4],
            prepared.counts[5],
            prepared.counts[6],
            prepared.counts[7],
            prepared.counts[8],
            operation.map(|value| value.operation_id.as_str()),
            operation.map(|value| value.kind.as_str()),
            operation.map(|value| value.phase.as_str()),
            operation.map(|value| value.region_id.as_str()),
            operation.map(|value| value.approved_worker_operation.as_str()),
            i64::try_from(summary.transfer_active_millis).map_err(|_| {
                StoreError::invalid("task history transfer active milliseconds overflow")
            })?,
        ],
    )?;
    Ok(())
}

fn insert_dem_source_authority(
    connection: &Connection,
    task_id: &str,
    authority: &PreparedDemSourceAuthority,
) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO task_history_dem_source_authority(
            task_id, region_id, dataset, provider, resource_id, capability_id,
            input_path, output_root
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            task_id,
            authority.region_id,
            authority.dataset,
            authority.provider,
            authority.resource_id,
            authority.capability_id,
            authority.input_path,
            authority.output_root,
        ],
    )?;
    Ok(())
}

pub(crate) fn backfill_history_summaries(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let history_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='task_history')",
        [],
        |row| row.get(0),
    )?;
    if !history_exists {
        return Ok(());
    }
    let histories = {
        let mut statement = transaction.prepare(
            "SELECT h.task_id, h.archived_at FROM task_history h
             LEFT JOIN task_tombstones d ON d.task_id = h.task_id
             WHERE d.task_id IS NULL ORDER BY h.task_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (task_id, archived_at) in histories {
        let Some(summary) = actionable_task_summary(transaction, &task_id)? else {
            continue;
        };
        let Ok(prepared) = prepare_history_summary(&summary, &archived_at) else {
            continue;
        };
        insert_history_summary(transaction, &summary, &prepared)?;
    }
    Ok(())
}

pub(crate) fn backfill_history_dataset_ids(
    transaction: &Transaction<'_>,
) -> Result<(), StoreError> {
    for table in ["task_history_summary", "task_snapshots"] {
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(());
        }
    }
    let snapshot_json_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('task_snapshots') WHERE name='snapshot_json')",
        [],
        |row| row.get(0),
    )?;
    if !snapshot_json_exists {
        return Ok(());
    }
    let rows = {
        let mut statement = transaction.prepare(
            "SELECT summary.task_id, snapshots.snapshot_json
             FROM task_history_summary summary
             JOIN task_snapshots snapshots ON snapshots.task_id = summary.task_id
             ORDER BY summary.task_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (task_id, snapshot_json) in rows {
        let dataset_id = task_dataset_id_from_snapshot_json(&snapshot_json);
        transaction.execute(
            "UPDATE task_history_summary SET dataset_id = ?2 WHERE task_id = ?1",
            params![task_id, dataset_id],
        )?;
    }
    Ok(())
}

pub(crate) fn backfill_dem_source_authorities(
    transaction: &Transaction<'_>,
) -> Result<(), StoreError> {
    for table in [
        "task_history_summary",
        "task_history",
        "task_snapshots",
        "task_runtime",
    ] {
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(());
        }
    }
    let task_ids = {
        let mut statement = transaction.prepare(
            "SELECT summary.task_id FROM task_history_summary summary
             JOIN task_history history ON history.task_id = summary.task_id
             LEFT JOIN task_tombstones tombstone ON tombstone.task_id = summary.task_id
             WHERE tombstone.task_id IS NULL
             ORDER BY summary.task_id",
        )?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for task_id in task_ids {
        let Some(task) = load_task_connection(transaction, &task_id)? else {
            continue;
        };
        let Ok(Some(authority)) = prepare_dem_source_authority(&task) else {
            continue;
        };
        insert_dem_source_authority(transaction, &task_id, &authority)?;
    }
    Ok(())
}

impl TaskStore {
    pub fn archive_task(
        &mut self,
        request: &StoreArchiveTaskRequest,
    ) -> Result<StoredHistory, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        let (canonical_archived_at, _) = canonical_archive_timestamp(&request.archived_at)
            .ok_or_else(|| StoreError::invalid("archive timestamp is invalid"))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::gacos_submission::ensure_no_active_gacos_submission(&transaction, &request.task_id)?;
        let tombstoned: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            [&request.task_id],
            |row| row.get(0),
        )?;
        if tombstoned {
            return Err(StoreError::AlreadyExists);
        }
        let existing = transaction
            .query_row(
                "SELECT archived_at FROM task_history WHERE task_id = ?1",
                [&request.task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let mut summary =
            actionable_task_summary(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        summary.transfer_active_millis = transfer_active_millis(&transaction, &request.task_id)?;
        let task =
            load_task_connection(&transaction, &request.task_id)?.ok_or(StoreError::NotFound)?;
        if let Some(archived_at) = existing {
            if canonical_archive_timestamp(&archived_at).map(|value| value.0)
                != Some(canonical_archived_at.clone())
            {
                return Err(StoreError::AlreadyExists);
            }
            transaction.commit()?;
            return Ok(StoredHistory {
                archived_at: canonical_archived_at,
                task,
            });
        }
        if task.runtime.revision != request.expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        if !is_terminal(&summary.status) {
            return Err(StoreError::invalid("only terminal tasks can be archived"));
        }
        let layout = layout_name(&task.snapshot.layout);
        let record_json = "{}";
        transaction.execute(
            "INSERT INTO task_history(task_id, archived_at, layout, record_json) \
             VALUES (?1, ?2, ?3, ?4)",
            params![request.task_id, canonical_archived_at, layout, record_json],
        )?;
        let prepared = prepare_history_summary(&summary, &canonical_archived_at)?;
        insert_history_summary(&transaction, &summary, &prepared)?;
        if let Some(authority) = prepare_dem_source_authority(&task)? {
            insert_dem_source_authority(&transaction, &request.task_id, &authority)?;
        }
        transaction.commit()?;
        Ok(StoredHistory {
            archived_at: canonical_archived_at,
            task,
        })
    }

    pub fn delete_history(
        &mut self,
        request: &StoreDeleteHistoryRequest,
    ) -> Result<(), StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        if request.deleted_at.trim().is_empty() {
            return Err(StoreError::invalid(
                "history deletion timestamp is required",
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::gacos_submission::ensure_no_active_gacos_submission(&transaction, &request.task_id)?;
        let existing_tombstone = transaction
            .query_row(
                "SELECT deleted_at FROM task_tombstones WHERE task_id = ?1",
                [&request.task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(deleted_at) = existing_tombstone {
            if deleted_at == request.deleted_at {
                transaction.commit()?;
                return Ok(());
            }
            return Err(StoreError::AlreadyExists);
        }
        if !crate::core_restore::task_is_actionable(&transaction, &request.task_id)? {
            return Err(StoreError::NotFound);
        }
        let layout = transaction
            .query_row(
                "SELECT layout FROM task_history WHERE task_id = ?1",
                [&request.task_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let identity_json = serde_json::to_string(&json!({
            "task_id": request.task_id,
            "layout": layout,
        }))
        .map_err(StoreError::Corrupt)?;
        transaction.execute(
            "INSERT INTO task_tombstones(task_id, deleted_at, identity_json) VALUES (?1, ?2, ?3)",
            params![request.task_id, request.deleted_at, identity_json],
        )?;
        transaction.execute(
            "DELETE FROM task_history_summary WHERE task_id = ?1",
            [&request.task_id],
        )?;
        transaction.execute(
            "DELETE FROM task_history WHERE task_id = ?1",
            [&request.task_id],
        )?;
        // Downloader D3: the task's local reuse decisions go with its history;
        // ledger facts, user files and renamed conflict files stay.
        crate::local_reuse::forget_task_local_reuse(&transaction, &request.task_id)?;
        let has_pins_table: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'task_pins')",
            [],
            |row| row.get(0),
        )?;
        if has_pins_table {
            transaction.execute(
                "DELETE FROM task_pins WHERE task_id = ?1",
                [&request.task_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_history(&self) -> Result<Vec<StoredHistory>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT h.task_id, h.archived_at FROM task_history h \
             LEFT JOIN task_tombstones d ON d.task_id = h.task_id \
             WHERE d.task_id IS NULL ORDER BY h.archived_at DESC, h.task_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (task_id, archived_at) = row?;
            if !crate::core_restore::task_is_actionable(&self.connection, &task_id)? {
                return Ok(None);
            }
            let task =
                load_task_connection(&self.connection, &task_id)?.ok_or(StoreError::NotFound)?;
            Ok(Some(StoredHistory { archived_at, task }))
        })
        .filter_map(|row| row.transpose())
        .collect()
    }

    pub fn restore(
        &self,
        request: &StoreRestoreRequest,
    ) -> Result<StoreRestoreResponse, StoreError> {
        request.validate().map_err(StoreError::invalid)?;
        let workbench = self.load_workbench()?;
        let sql = if request.include_terminal {
            "SELECT s.task_id FROM task_snapshots s \
             LEFT JOIN task_tombstones d ON d.task_id = s.task_id \
             WHERE d.task_id IS NULL ORDER BY s.created_at, s.task_id"
        } else {
            "SELECT s.task_id FROM task_snapshots s \
             LEFT JOIN task_history h ON h.task_id = s.task_id \
             LEFT JOIN task_tombstones d ON d.task_id = s.task_id \
             WHERE h.task_id IS NULL AND d.task_id IS NULL ORDER BY s.created_at, s.task_id"
        };
        let mut statement = self.connection.prepare(sql)?;
        let task_ids: Vec<String> = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        let stored: Vec<StoredTask> = task_ids
            .iter()
            .map(|task_id| {
                let task = if request.include_terminal {
                    load_task_connection(&self.connection, task_id)?
                } else {
                    load_compact_task_connection(&self.connection, task_id)?
                };
                task.ok_or(StoreError::NotFound)
            })
            .collect::<Result<_, _>>()?;
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
        let response = StoreRestoreResponse {
            workbench_batch: workbench.as_ref().map(|value| value.batch.clone()),
            workbench: workbench.map(|value| value.state),
            tasks: stored.iter().map(|value| value.snapshot.clone()).collect(),
            runtimes: stored.into_iter().map(|value| value.runtime).collect(),
            region_aoi,
            region_aois,
            workspace_context,
        };
        response.validate().map_err(StoreError::invalid)?;
        Ok(response)
    }

    pub fn set_task_pinned(
        &mut self,
        task_id: &str,
        pinned: bool,
        pinned_at: &str,
    ) -> Result<Vec<String>, StoreError> {
        if task_id.trim().is_empty() {
            return Err(StoreError::invalid("task_id is required"));
        }
        if pinned && pinned_at.trim().is_empty() {
            return Err(StoreError::invalid("pinned timestamp is required"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS task_pins (
                 task_id TEXT PRIMARY KEY NOT NULL,
                 pinned_at TEXT NOT NULL
             ) STRICT;",
        )?;
        if pinned {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM task_snapshots s
                     LEFT JOIN task_tombstones d ON d.task_id = s.task_id
                     WHERE s.task_id = ?1 AND d.task_id IS NULL
                 )",
                [task_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(StoreError::NotFound);
            }
            transaction.execute(
                "INSERT INTO task_pins(task_id, pinned_at) VALUES (?1, ?2)
                 ON CONFLICT(task_id) DO UPDATE SET pinned_at = excluded.pinned_at",
                params![task_id, pinned_at],
            )?;
        } else {
            transaction.execute("DELETE FROM task_pins WHERE task_id = ?1", [task_id])?;
        }
        let ids = list_pinned_ids_tx(&transaction)?;
        transaction.commit()?;
        Ok(ids)
    }

    pub fn list_pinned_task_ids(&self) -> Result<Vec<String>, StoreError> {
        list_pinned_ids_tx(&self.connection)
    }

    pub fn is_task_archived(&self, task_id: &str) -> Result<bool, StoreError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_history WHERE task_id = ?1)",
            [task_id],
            |row| row.get::<_, bool>(0),
        )?)
    }

    pub fn is_task_tombstoned(&self, task_id: &str) -> Result<bool, StoreError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_tombstones WHERE task_id = ?1)",
            [task_id],
            |row| row.get::<_, bool>(0),
        )?)
    }

    pub fn load_completed_download_destinations(
        &self,
    ) -> Result<Vec<CompletedDownloadDestination>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT r.task_id, r.item_id, r.result_json, s.provider_id, s.snapshot_json
             FROM download_results r
             INNER JOIN task_snapshots s ON s.task_id = r.task_id
             LEFT JOIN task_tombstones d ON d.task_id = r.task_id
             WHERE d.task_id IS NULL AND r.outcome IN ('completed', 'skipped')
             ORDER BY r.finished_at DESC, r.task_id, r.item_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (task_id, item_id, result_json, provider_id, snapshot_json) = row?;
            let Ok(result) = serde_json::from_str::<insar_core::DownloadItemResult>(&result_json)
            else {
                continue;
            };
            let task_title = task_label_from_snapshot(&snapshot_json, &provider_id, &task_id);
            out.push(CompletedDownloadDestination {
                task_id,
                task_title,
                item_id,
                destination: result.destination,
            });
        }
        Ok(out)
    }
}

fn list_pinned_ids_tx(connection: &Connection) -> Result<Vec<String>, StoreError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'task_pins')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "SELECT p.task_id FROM task_pins p
         LEFT JOIN task_tombstones d ON d.task_id = p.task_id
         WHERE d.task_id IS NULL
         ORDER BY p.pinned_at DESC, p.task_id ASC",
    )?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn task_label_from_snapshot(
    snapshot_json: &str,
    provider_id: &str,
    task_id: &str,
) -> Option<String> {
    let decoded = crate::snapshot_codec::decode_any_snapshot(snapshot_json).ok()?;
    let (aoi_name, dataset) = match &decoded {
        crate::StoredTaskSnapshot::CompactV2(snap) => (
            snap.aoi_name.as_str(),
            task_dataset_id_from_snapshot_json(snapshot_json),
        ),
        crate::StoredTaskSnapshot::LegacyV2(snap) => (
            snap.aoi_name.as_str(),
            task_dataset_id_from_snapshot_json(snapshot_json),
        ),
        crate::StoredTaskSnapshot::V3(snap) => (
            snap.aoi_name.as_str(),
            task_dataset_id_from_snapshot_json(snapshot_json),
        ),
    };
    let aoi = aoi_name.trim();
    let ds = dataset.as_deref().unwrap_or(provider_id).trim();
    if !aoi.is_empty() && !ds.is_empty() {
        Some(format!("{aoi} · {ds}"))
    } else if !aoi.is_empty() {
        Some(aoi.to_owned())
    } else if !ds.is_empty() {
        Some(ds.to_owned())
    } else {
        Some(task_id.to_owned())
    }
}

fn is_terminal(status: &TaskLifecycleStatus) -> bool {
    matches!(
        status,
        TaskLifecycleStatus::Completed
            | TaskLifecycleStatus::Failed
            | TaskLifecycleStatus::Cancelled
            | TaskLifecycleStatus::Interrupted
    )
}

fn layout_name(layout: &DownloadLayout) -> &'static str {
    match layout {
        DownloadLayout::Direct => "direct",
        DownloadLayout::Sentinel1Slc => "sentinel1_slc",
        DownloadLayout::SentinelOrbitAuxPoeorb => "sentinel_orbit_aux_poeorb",
        DownloadLayout::SarDatasetProcessing => "sar_dataset_processing",
    }
}
